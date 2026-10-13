//! The native hydraulic erosion (`ParallelErosion_Erode`), the engine of
//! ErosionPro and of every node built on `Erosions.PE`.
//!
//! The heightmap becomes a Laplacian pyramid. Droplets erode the coarsest
//! level first; each finer level then takes back the eroded coarse surface
//! with its own detail on top (sediment fills that detail in) and runs
//! droplets of its own. A droplet walks up to eleven cells per round and
//! leaves its height changes in integer accumulators that the last droplet
//! through a cell applies. The droplets may run on many threads; the
//! integer sums make their order irrelevant, so walking them in sequence
//! here gives the same result.

use nerthus_engine::Map;

use crate::random::{self, Mt19937};
use crate::{gpu, maths, sycl};
use crate::x86::{cvttss2si, maxss, minss};

/// Size of the table of jitter directions; droplets step through it.
const DIRECTION_COUNT: usize = 32;
const DIRECTION_MASK: u32 = DIRECTION_COUNT as u32 - 1;
/// Steps a droplet takes per round, and the cells its path can hold.
const STEPS_PER_ROUND: usize = 11;
const PATH_CAPACITY: usize = STEPS_PER_ROUND + 1;
/// Height changes accumulate in 8.24 fixed point.
const FIXED_SCALE: f32 = 16777216.0;
const FIXED_UNIT: f32 = 5.9604645e-8;
/// A droplet's water level before its first step, and the value above
/// which a level counts as set.
const LEVEL_UNSET: f32 = -999_999.9;
const LEVEL_SET_ABOVE: f32 = -999999.0;
/// The position hash: xorshift32 over `seed × salt + droplet`, scaled by an
/// odd multiplier into `[0, 1)`.
const HASH_FALLBACK: u32 = 0xA235_1342;
const HASH_MULTIPLIER: u32 = 0x2545_F381;
const POSITION_SALT: u32 = 0x1B227;
const PRECIPITATION_SALT: u32 = 0x1B253;
const UNIT_SCALE: f32 = 2.3283064e-10;
/// ParallelErosion zeroes the parameter that would scale suspended-load
/// removal by flow, but still multiplies by it.
const SUSPENDED_REMOVAL: f32 = 0.0;
const FLOW_SCALE: f32 = 0.001;
/// The shaping mean weights diagonal neighbours by about 1/√2 and
/// normalises by 1 / (4 + 4 × 0.7071).
const DIAGONAL_WEIGHT: f32 = f32::from_bits(0x3F35_0481);
const MEAN_NORMALISATION: f32 = f32::from_bits(0x3E15_F641);
/// How far below a cell (in cells, times √2 on diagonals) a neighbour must
/// lie for the cell's wetness to reach it: 1 / (2√3).
const SIDE_DROP: f32 = f32::from_bits(0x3EB7_290C);

/// `ParallelErosion.Parameters` without the resolution and terrain size,
/// which come from the map and the build.
#[derive(Clone, Copy, Debug)]
pub struct ErosionSettings {
    pub duration: f32,
    pub downcutting: f32,
    pub downcutting_type: f32,
    pub erosion_scale: f32,
    pub suspended_load_amount: f32,
    pub suspended_load_angle: f32,
    pub bed_load_amount: f32,
    pub bed_load_angle: f32,
    pub coarse_sediments_amount: f32,
    pub coarse_sediments_angle: f32,
    pub shape: f32,
    pub shape_sharpness: f32,
    pub shape_detail_scale: f32,
    pub seed: i32,
    pub deposition_boost: f32,
    pub extra_deposition_boost: f32,
    pub no_erosion: bool,
}

impl Default for ErosionSettings {
    /// The defaults of `Erosions.PE`.
    fn default() -> Self {
        Self {
            duration: 10.0,
            downcutting: 0.25,
            downcutting_type: 0.0,
            erosion_scale: 1000.0,
            suspended_load_amount: 1.0,
            suspended_load_angle: 24.0,
            bed_load_amount: 0.0,
            bed_load_angle: 15.0,
            coarse_sediments_amount: 0.0,
            coarse_sediments_angle: 10.0,
            shape: 0.2,
            shape_sharpness: 0.6,
            shape_detail_scale: 0.8,
            seed: 0,
            deposition_boost: 0.0,
            extra_deposition_boost: 0.0,
            no_erosion: false,
        }
    }
}

/// What [`erode`] returns.
pub struct ErosionMaps {
    pub height: Map,
    pub flow: Map,
    pub deposition: Map,
    pub wear: Map,
}

/// The precipitation mask of a level and the wetness the shaping pass
/// spreads with it.
struct Precipitation {
    mask: Vec<f32>,
    wetness: Vec<f32>,
    spread: Vec<f32>,
}

/// One level of the pyramid. Every layer is `width × height`, row-major.
struct Level {
    width: usize,
    height: usize,
    heights: Vec<f32>,
    flow: Vec<f32>,
    shaping: Vec<f32>,
    deposition: Vec<f32>,
    wear: Vec<f32>,
    /// This level's height minus the next coarser level (empty on the
    /// coarsest level).
    detail: Vec<f32>,
    /// Fixed-point height changes, their weights and the droplet visits
    /// still to come, per cell.
    sums: Vec<i32>,
    weights: Vec<i32>,
    visits: Vec<i32>,
    precipitation: Option<Precipitation>,
}

impl Level {
    fn new(width: usize, height: usize, heights: Vec<f32>, precipitation: Option<Vec<f32>>) -> Self {
        let size = width * height;
        let precipitation = precipitation.map(|mask| Precipitation {
            mask,
            wetness: vec![0.0; size],
            spread: vec![0.0; size],
        });
        Self {
            width,
            height,
            heights,
            flow: vec![0.0; size],
            shaping: vec![0.0; size],
            deposition: vec![0.0; size],
            wear: vec![0.0; size],
            detail: Vec::new(),
            sums: vec![0; size],
            weights: vec![0; size],
            visits: vec![0; size],
            precipitation,
        }
    }

    fn cell(&self, x: i32, y: i32) -> usize {
        (y * self.width as i32 + x) as usize
    }

    /// The next coarser level. Its heights (and mask) are
    /// this level's filtered down to half size; this level keeps what the
    /// coarser one cannot represent as its detail.
    fn coarsen(&mut self) -> Level {
        let width = self.width >> 1;
        let height = self.height >> 1;
        let heights = downsample(&self.heights, self.width, self.height);
        let mask = self.precipitation.as_ref().map(|precipitation| {
            let halved = downsample(&precipitation.mask, self.width, self.height);
            sharpen(&halved, width, height)
        });
        let coarser = Level::new(width, height, heights, mask);

        let mut detail = vec![0.0f32; self.width * self.height];
        for y in 0..self.height as i32 {
            for x in 0..self.width as i32 {
                let index = self.cell(x, y);
                let coarse = upsample(&coarser.heights, width, height, x, y);
                detail[index] = self.heights[index] - coarse;
            }
        }
        self.detail = detail;
        coarser
    }

    /// Takes back the eroded surface of
    /// the coarser level. Deposits first fill this level's detail; where
    /// the detail stands above them it is added again, scaled by how worn
    /// the coarse surface is.
    fn refine(&mut self, coarser: &Level, cell_size: f32) {
        for y in 0..self.height as i32 {
            for x in 0..self.width as i32 {
                let index = self.cell(x, y);
                let mut height = upsample(&coarser.heights, coarser.width, coarser.height, x, y);
                let detail = self.detail[index];
                let mut deposit = upsample(&coarser.deposition, coarser.width, coarser.height, x, y);
                let wear = upsample(&coarser.wear, coarser.width, coarser.height, x, y);
                if !(deposit > 0.0) {
                    height += detail;
                } else if !(0.0 >= deposit - detail) {
                    if detail > 0.0 {
                        let fill = minss((cell_size + cell_size) / deposit - 0.25, 1.0);
                        let filled = fill * detail;
                        height += filled;
                        deposit -= filled;
                    }
                } else {
                    let exposure = maxss(0.1, minss(1.0, cell_size / wear));
                    height += exposure * (detail - deposit);
                    deposit = 0.0;
                }
                self.deposition[index] = deposit;
                self.wear[index] = maxss(0.0, wear - detail);
                self.heights[index] = height;

                if let (Some(fine), Some(coarse)) = (self.precipitation.as_mut(), coarser.precipitation.as_ref()) {
                    fine.wetness[index] = upsample(&coarse.wetness, coarser.width, coarser.height, x, y);
                    fine.spread[index] = upsample(&coarse.spread, coarser.width, coarser.height, x, y);
                }
            }
        }
    }
}

/// The `[1 3 3 1]² / 64` filter around coarse cell
/// (`x`, `y`), clamped to the edges.
fn downsample_cell(values: &[f32], width: usize, height: usize, x: i32, y: i32) -> f32 {
    let last_x = width as i32 - 1;
    let last_y = height as i32 - 1;
    let columns = [(2 * x - 1).max(0), 2 * x, (2 * x + 1).min(last_x), (2 * x + 2).min(last_x)];
    let rows = [(2 * y - 1).max(0), 2 * y, (2 * y + 1).min(last_y), (2 * y + 2).min(last_y)];
    let at = |row: usize, column: usize| values[(rows[row] * width as i32 + columns[column]) as usize];

    let mut sum = at(0, 1) * 3.0 + at(0, 0);
    sum += at(0, 2) * 3.0;
    sum += at(0, 3);
    sum += at(1, 0) * 3.0;
    sum += at(1, 1) * 9.0;
    sum += at(1, 2) * 9.0;
    sum += at(1, 3) * 3.0;
    sum += at(2, 0) * 3.0;
    sum += at(2, 1) * 9.0;
    sum += at(2, 2) * 9.0;
    sum += at(2, 3) * 3.0;
    sum += at(3, 0);
    sum += at(3, 1) * 3.0;
    sum += at(3, 2) * 3.0;
    sum += at(3, 3);
    sum * 0.015625
}

/// A layer filtered down to half size.
fn downsample(values: &[f32], width: usize, height: usize) -> Vec<f32> {
    let half_width = width >> 1;
    let half_height = height >> 1;
    let mut result = vec![0.0f32; half_width * half_height];
    for y in 0..half_height {
        for x in 0..half_width {
            result[y * half_width + x] = downsample_cell(values, width, height, x as i32, y as i32);
        }
    }
    result
}

/// A half-size layer sampled at fine cell (`x`, `y`),
/// weighting the coarse cell under it 9, its nearer neighbours 3 and the
/// diagonal 1.
fn upsample(values: &[f32], width: usize, height: usize, x: i32, y: i32) -> f32 {
    let center_x = x >> 1;
    let center_y = y >> 1;
    let neighbour_x = (center_x + (x & 1) * 2 - 1).max(0).min(width as i32 - 1);
    let neighbour_y = (center_y + (y & 1) * 2 - 1).max(0).min(height as i32 - 1);
    let at = |column: i32, row: i32| values[(row * width as i32 + column) as usize];

    let near = at(center_x, center_y);
    let across = at(neighbour_x, center_y);
    let down = at(center_x, neighbour_y);
    let diagonal = at(neighbour_x, neighbour_y);
    (across * 3.0 + near * 9.0 + down * 3.0 + diagonal) * 0.0625
}

/// The 3×3 filter that sharpens a downsampled mask back
/// (corners 1/16, edges −3/8, centre 9/4).
fn sharpen(values: &[f32], width: usize, height: usize) -> Vec<f32> {
    let mut result = vec![0.0f32; width * height];
    for y in 0..height as i32 {
        let above = (y - 1).max(0);
        let below = (y + 1).min(height as i32 - 1);
        for x in 0..width as i32 {
            let left = (x - 1).max(0);
            let right = (x + 1).min(width as i32 - 1);
            let at = |column: i32, row: i32| values[(row * width as i32 + column) as usize];
            let corners = at(right, above) + at(left, above) + at(left, below) + at(right, below);
            let edges = at(x, above) + at(right, y) + at(left, y) + at(x, below);
            result[(y * width as i32 + x) as usize] = (corners * 0.0625 - edges * 0.375) + 2.25 * at(x, y);
        }
    }
    result
}

/// How strongly a setting applies on the pyramid level
/// `level` (counted from the coarsest), given the pyramid's `scale` in
/// octaves. The fractional octave fades in the coarsest level when
/// `fade_first`.
fn level_falloff(level: i32, scale: f32, offset: f32, falloff: f32, base: f32, fade_first: bool) -> f32 {
    let mut scale = scale;
    if !(scale >= 1.0) {
        scale = maths::exp(scale - 1.0);
    }
    let mut fraction = scale - scale.floor();
    if fraction == 0.0 {
        fraction = 1.0;
    }
    let depth = maxss(0.0, (level as f32 - (1.0 - fraction)) - offset);
    let decay = maths::exp((-depth) * falloff);
    let exponent = (decay / falloff + depth) - 1.0 / falloff;
    let log_base = maths::ln(base);
    let weight = if level > 0 || !fade_first { 1.0 } else { fraction };
    maths::exp(exponent * log_base) * weight
}

/// 32 unit vectors around the circle, shuffled by the
/// seed (`std::mt19937` + MSVC's `std::shuffle`).
pub(crate) fn jitter_directions(seed: i32) -> Vec<[f32; 2]> {
    let step = 6.2831855f32 / DIRECTION_COUNT as f32;
    let mut directions: Vec<[f32; 2]> = (0..DIRECTION_COUNT)
        .map(|index| {
            let angle = index as f32 * step;
            let (sine, cosine) = maths::sin_cos(angle);
            [cosine, sine]
        })
        .collect();
    let mut generator = Mt19937::new(seed as u32);
    random::msvc_shuffle(&mut directions, &mut generator);
    directions
}

fn xorshift(mut value: u32) -> u32 {
    value ^= value << 13;
    value ^= value >> 17;
    value ^= value << 5;
    value
}

fn hash_start(seed: i32, salt: u32, droplet: u32) -> u32 {
    let mixed = (seed as u32).wrapping_mul(salt).wrapping_add(droplet);
    if mixed == 0 { HASH_FALLBACK } else { mixed }
}

fn unit_float(state: u32) -> f32 {
    state.wrapping_mul(HASH_MULTIPLIER) as f32 * UNIT_SCALE
}

/// A droplet (512 bytes natively). Its path holds the cells of the current
/// round: where it stood, the surface height there and how far it had to
/// climb.
#[derive(Clone)]
struct Droplet {
    position: [f32; 2],
    path: [[f32; 2]; PATH_CAPACITY],
    heights: [f32; PATH_CAPACITY],
    rises: [f32; PATH_CAPACITY],
    path_length: usize,
    total_steps: i32,
    uphill_steps: i32,
    stuck: bool,
    finished: bool,
    /// The surface the droplet's water and load stand at, and its value
    /// one cell back.
    level: f32,
    previous_level: f32,
    sediment: f32,
    water: f32,
    direction_index: u32,
}

impl Droplet {
    fn new(position: [f32; 2], direction_index: u32) -> Self {
        Self {
            position,
            path: [[0.0; 2]; PATH_CAPACITY],
            heights: [0.0; PATH_CAPACITY],
            rises: [0.0; PATH_CAPACITY],
            path_length: 0,
            total_steps: 0,
            uphill_steps: 0,
            stuck: false,
            finished: false,
            level: LEVEL_UNSET,
            previous_level: 0.0,
            sediment: 0.0,
            water: 0.0,
            direction_index,
        }
    }
}

/// The surface under a point: its bilinear height and gradient.
struct Surface {
    height: f32,
    gradient_x: f32,
    gradient_y: f32,
    slope: f32,
}

/// Bilinear height and gradient at (`x`, `y`), which must lie at least
/// half a cell inside the map.
fn surface_at(level: &Level, x: f32, y: f32) -> Surface {
    let cell_x = (x - 0.5) as i32;
    let cell_y = (y - 0.5) as i32;
    let fraction_x = (x - cell_x as f32) - 0.5;
    let fraction_y = (y - cell_y as f32) - 0.5;
    let top_left = level.heights[level.cell(cell_x, cell_y)];
    let top_right = level.heights[level.cell(cell_x + 1, cell_y)];
    let bottom_left = level.heights[level.cell(cell_x, cell_y + 1)];
    let bottom_right = level.heights[level.cell(cell_x + 1, cell_y + 1)];

    let top = (1.0 - fraction_x) * top_left + fraction_x * top_right;
    let bottom = (1.0 - fraction_x) * bottom_left + fraction_x * bottom_right;
    let height = top * (1.0 - fraction_y) + bottom * fraction_y;
    let gradient_x = (top_right - top_left) * (1.0 - fraction_y) + (bottom_right - bottom_left) * fraction_y;
    let gradient_y = (bottom_left - top_left) * (1.0 - fraction_x) + (bottom_right - top_right) * fraction_x;
    let slope = (gradient_y * gradient_y + gradient_x * gradient_x).sqrt();
    Surface {
        height,
        gradient_x,
        gradient_y,
        slope,
    }
}

/// The run-wide state of `ParallelErosion_Erode`.
struct Simulation {
    settings: ErosionSettings,
    terrain_width: f32,
    terrain_height: f32,
    directions: Vec<[f32; 2]>,
    suspended_tangent: f32,
    bed_tangent: f32,
    coarse_tangent: f32,
    has_precipitation: bool,
    /// The pyramid's size in octaves, and the octave below which shaping
    /// detail fades.
    octaves: f32,
    detail_octave: f32,
    finished_levels: i32,
    downcutting: f32,
    shape: f32,
    relaxation: f32,
}

/// What a level works out before its rounds: how many
/// rounds run, the level's downcutting, shaping and relaxation, and the
/// droplets each round spawns.
pub(crate) struct LevelPlan {
    pub(crate) rounds: i32,
    pub(crate) downcutting: f32,
    pub(crate) shape: f32,
    pub(crate) relaxation: f32,
    pub(crate) droplets: u32,
}

/// The plan for the level `index` levels finer than the coarsest.
pub(crate) fn plan_level(index: i32, octaves: f32, detail_octave: f32, settings: &ErosionSettings, width: usize, height: usize) -> LevelPlan {
    let rounds_scale = level_falloff(index, octaves, 2.5, 0.75, 0.5, true);
    let rounds = cvttss2si((rounds_scale * settings.duration).ceil());
    let downcutting = level_falloff(index, octaves, 1.0, 1.2, 0.6, false) * settings.downcutting;
    let shape = level_falloff(index, octaves, detail_octave, 10.0, 0.4, false) * settings.shape;
    let relaxation = maxss(1.0, level_falloff(index, octaves, 0.0, 10.0, 2.0, false));
    let droplets = ((width * height) as i32 >> 8).max(0) as u32;
    LevelPlan { rounds, downcutting, shape, relaxation, droplets }
}

/// The boost deposits get on the level `index` levels finer than the
/// coarsest.
pub(crate) fn deposition_boost(index: i32, has_precipitation: bool, settings: &ErosionSettings) -> f32 {
    let rate = if has_precipitation { 0.3 } else { 0.5 };
    let decay = maths::exp(rate * (-index) as f32);
    (1.0 - decay) * settings.deposition_boost * 1.4 + 1.0
}

impl Simulation {
    /// Every round of droplets on one level.
    fn erode_level(&mut self, level: &mut Level) {
        let index = self.finished_levels;
        let plan = plan_level(index, self.octaves, self.detail_octave, &self.settings, level.width, level.height);
        let rounds = plan.rounds;
        self.downcutting = plan.downcutting;
        self.shape = plan.shape;
        self.relaxation = plan.relaxation;

        let count = plan.droplets;
        let mut first_droplet = 0u32;
        for _ in 0..rounds {
            let mut droplets = self.spawn(level, first_droplet, count);
            let mut active: Vec<usize> = (0..droplets.len()).collect();
            while !active.is_empty() {
                let mut moving = Vec::with_capacity(active.len());
                for &index in &active {
                    let droplet = &mut droplets[index];
                    self.advance(droplet, level);
                    if droplet.stuck {
                        droplet.finished = true;
                    }
                    if !droplet.finished {
                        moving.push(index);
                    }
                }
                for &index in &active {
                    self.settle(&mut droplets[index], level);
                }
                active = moving;
            }
            if self.settings.shape > 0.0 {
                if level.precipitation.is_some() {
                    self.reshape_wet(level);
                } else {
                    self.reshape(level);
                }
            }
            first_droplet = first_droplet.wrapping_add(count);
        }
    }

    /// One round's droplets at hashed
    /// positions; with a precipitation mask, each falls with the mask's
    /// probability.
    fn spawn(&self, level: &Level, first_droplet: u32, count: u32) -> Vec<Droplet> {
        let direction_index = first_droplet & DIRECTION_MASK;
        let mut droplets = Vec::with_capacity(count as usize);
        for offset in 0..count {
            let id = first_droplet.wrapping_add(offset);
            let first = xorshift(hash_start(self.settings.seed, POSITION_SALT, id));
            let second = xorshift(first);
            let x = unit_float(first) * level.width as f32;
            let y = unit_float(second) * level.height as f32;
            if let Some(precipitation) = &level.precipitation {
                let chance = unit_float(xorshift(hash_start(self.settings.seed, PRECIPITATION_SALT, id)));
                let cell = level.cell(x as i32, y as i32);
                if !(chance < precipitation.mask[cell]) {
                    continue;
                }
            }
            droplets.push(Droplet::new([x, y], direction_index));
        }
        droplets
    }

    /// Up to eleven steps downhill, with inertia and a
    /// little jitter, counting every cell visited.
    fn advance(&self, droplet: &mut Droplet, level: &mut Level) {
        let [mut x, mut y] = droplet.position;
        droplet.path_length = 0;
        let limit_x = level.width as f32 - 1.5;
        let limit_y = level.height as f32 - 1.5;
        let inside = |x: f32, y: f32| cvttss2si(x - 0.5) >= 0 && cvttss2si(y - 0.5) >= 0 && x < limit_x && y < limit_y;
        if !inside(x, y) {
            droplet.finished = true;
            return;
        }
        let start = surface_at(level, x, y);
        if !(start.slope > 0.0) {
            droplet.finished = true;
            return;
        }
        let mut direction = [(-start.gradient_x) / start.slope, (-start.gradient_y) / start.slope];
        let mut height = start.height;
        let water_level = if LEVEL_SET_ABOVE < droplet.level { droplet.level } else { height };
        droplet.previous_level = if LEVEL_SET_ABOVE < water_level { water_level } else { height };
        droplet.level = water_level;

        let mut climbing = 0.0f32;
        let mut stop = false;
        let mut left_map = false;
        let mut step_start = [x, y];
        let mut step = 0usize;
        loop {
            droplet.path[step] = step_start;
            droplet.heights[step] = height;
            droplet.rises[step] = 0.0;
            let visited = level.cell(x as i32, y as i32);
            level.flow[visited] += 1.0;
            level.visits[visited] += 1;

            x += direction[0];
            y += direction[1];
            if !inside(x, y) {
                droplet.finished = true;
                left_map = true;
                stop = true;
            } else {
                droplet.finished = false;
                let surface = surface_at(level, x, y);
                droplet.direction_index = (droplet.direction_index + 1) & DIRECTION_MASK;
                if surface.slope > 0.0 {
                    let inertia = if climbing != 0.0 { 30.0 } else { 1.0 };
                    let jitter = self.directions[droplet.direction_index as usize];
                    let pull_x = (-surface.gradient_x) / surface.slope;
                    let pull_y = (-surface.gradient_y) / surface.slope;
                    let next_x = (0.5 * direction[0] * inertia + 0.25 * jitter[0]) + pull_x;
                    let next_y = (0.5 * direction[1] * inertia + 0.25 * jitter[1]) + pull_y;
                    let length = (next_y * next_y + next_x * next_x).sqrt();
                    let inverse = 1.0 / length;
                    direction = [inverse * next_x, inverse * next_y];
                } else {
                    direction = [0.0, 0.0];
                    stop = true;
                }

                if surface.height >= height {
                    droplet.uphill_steps += 1;
                    climbing = 1.0;
                    droplet.rises[step] = surface.height - height;
                    droplet.heights[step] = surface.height;
                    if droplet.uphill_steps >= 5 || droplet.total_steps + step as i32 <= 2 {
                        stop = true;
                        droplet.stuck = true;
                    }
                } else {
                    climbing = 0.0;
                    droplet.rises[step] = 0.0;
                }
                height = surface.height;
            }

            step += 1;
            if step == STEPS_PER_ROUND || stop {
                break;
            }
            step_start = [x, y];
        }

        if climbing != 0.0 {
            droplet.stuck = true;
        }
        let last = if droplet.stuck || left_map {
            droplet.position = step_start;
            step - 1
        } else {
            droplet.path[step] = [x, y];
            droplet.heights[step] = height;
            droplet.rises[step] = 0.0;
            droplet.position = [x, y];
            let visited = level.cell(x as i32, y as i32);
            level.visits[visited] += 1;
            level.flow[visited] += 1.0;
            step
        };
        droplet.path_length = last + 1;
        droplet.total_steps += last as i32 + 1;
    }

    /// Erosion and deposition along the round's path.
    /// Each cell's change goes into the fixed-point sums; the last droplet
    /// through the cell applies their weighted mean.
    fn settle(&self, droplet: &mut Droplet, level: &mut Level) {
        let cell_size = (self.terrain_width / self.terrain_height) / level.width as f32;
        let length = droplet.path_length;
        let boost = deposition_boost(self.finished_levels, self.has_precipitation, &self.settings);
        if length == 0 {
            return;
        }
        let floor_bias = droplet.heights[length - 1] * 0.05;
        let last = length - 1;

        for index in 0..length {
            let [x, y] = droplet.path[index];
            let cell = level.cell(x as i32, y as i32);
            let before = droplet.heights[index.saturating_sub(1)];
            let after = droplet.heights[(index + 1).min(last)];
            let current = droplet.heights[index];
            let far_before = droplet.heights[index.saturating_sub(2)];
            let far_after = droplet.heights[(index + 2).min(last)];

            let slope = ((before - after) * 0.5) / cell_size;
            let downhill = maxss(0.0, slope);
            let carried = if self.suspended_tangent > downhill * self.settings.suspended_load_amount {
                1.0
            } else {
                0.5 / (downhill * 4.0 + 1.0)
            };
            let mean3 = ((current + before) + after) / 3.0;
            let mean5 = ((((before + far_before) + current) + after) + far_after) / 5.0;
            let rise3 = mean3 - current;
            let rise5 = mean5 - current;
            let fill = maxss(0.0, maxss(rise5, rise3));
            let mut amount = fill * carried;
            let smoothed = if 0.0 > rise3 && 0.0 > rise5 { mean5 } else { mean3 };

            let floor = far_after * 0.95 + floor_bias;
            let sediment = droplet.sediment;
            let lowest = minss(current, smoothed);
            let downcutting_type = self.settings.downcutting_type;
            let relative_cut = (lowest - floor) * self.downcutting * (1.0 - downcutting_type);
            let constant_cut = downcutting_type * self.settings.downcutting * cell_size * 0.15;
            let cut = relative_cut + constant_cut;
            if sediment > cut + fill {
                let coarse = self.coarse_tangent;
                if slope > coarse * 0.8 && coarse * 1.25 > slope {
                    let dropped = sediment * 0.5 * self.settings.coarse_sediments_amount * 0.5;
                    amount += minss(dropped, cut);
                }
                if self.bed_tangent > slope {
                    let steepness = if downhill < self.bed_tangent * 0.1 { 100.0 } else { 1.0 };
                    let dropped = sediment * 0.3 * self.settings.bed_load_amount * 0.5 * steepness;
                    amount += minss(dropped, cut);
                }
            }

            let water = droplet.water;
            let target = current - (current - (lowest - cut));
            let water_level = droplet.level;
            let deposit = minss(minss((water_level - target) * 0.9, amount), sediment);
            let removal_limit = water * 0.02 * SUSPENDED_REMOVAL * level.flow[cell];
            let removal = minss((droplet.previous_level - (deposit + target)) * 0.95, removal_limit);
            droplet.previous_level = water_level;
            let kept = (1.0 - self.settings.extra_deposition_boost) * removal;
            let total = removal + deposit;
            droplet.water = water - kept;
            let raised = maxss(0.0, total * boost);
            droplet.level = raised + target;
            let gain = droplet.level - current;
            let used = if 0.0 > gain { gain } else { gain * 1.05 };
            let returned = (raised - total) * self.settings.extra_deposition_boost;
            droplet.sediment = maxss(0.0, returned + (sediment - used));

            let weight = if index == 0 || index == last || droplet.stuck { 1 } else { 2 };
            let flowing = if droplet.stuck { 0.0 } else { 1.0 };
            let settled = if droplet.total_steps > 3 { 1.0 } else { 0.0 };
            let mut change = ((weight * weight) as f32 * 0.5) * flowing * settled * gain;
            let rise = droplet.rises[index];
            if rise > 0.0 {
                change = maxss(rise, change);
            }

            level.sums[cell] = level.sums[cell].wrapping_add(cvttss2si(change * FIXED_SCALE));
            level.weights[cell] += weight;
            let remaining = level.visits[cell];
            level.visits[cell] -= 1;
            if remaining == 1 {
                let mut delta = (level.sums[cell] as f32 * FIXED_UNIT) / level.weights[cell] as f32;
                if self.settings.no_erosion {
                    delta = maxss(0.0, delta);
                }
                level.heights[cell] += delta;
                let deposited = if delta > 0.0 { delta } else { delta * 0.95 };
                level.deposition[cell] = maxss(deposited + level.deposition[cell], 0.0);
                let worn = minss(0.0, delta);
                level.wear[cell] += -worn;
                level.sums[cell] = 0;
                level.weights[cell] = 0;
            }
        }
    }

    /// The shaping change at one cell: slopes
    /// steeper than a cell relax towards their lower neighbours, the rest
    /// towards the local mean.
    fn shaping_at(&self, cells: &Neighbourhood, cell_size: f32) -> f32 {
        let diagonals = (((cells.top_right + cells.top_left) + cells.bottom_left) + cells.bottom_right) * DIAGONAL_WEIGHT;
        let sides = ((cells.left + cells.top) + cells.right) + cells.bottom;
        let mean = (diagonals + sides) * MEAN_NORMALISATION;
        let center = cells.center;
        let above_mean = center - mean;

        let mut total_weight = 0.0f32;
        let mut pull = 0.0f32;
        if center > cells.top_left {
            let difference = center - cells.top_left;
            let weight = difference / cell_size;
            total_weight = weight + 0.0;
            pull = 0.0 - weight * difference;
        }
        let neighbours = [cells.top, cells.top_right, cells.left, cells.right, cells.bottom_left, cells.bottom, cells.bottom_right];
        for neighbour in neighbours {
            if center > neighbour {
                let difference = center - neighbour;
                let weight = difference / cell_size;
                total_weight += weight;
                pull -= weight * difference;
            }
        }
        if total_weight > 0.0 {
            pull /= total_weight;
        }

        let sharpness = self.settings.shape_sharpness;
        let sharp_share = sharpness * sharpness * 0.9;
        let lower_row = ((cells.bottom + cells.bottom_left) + cells.bottom) + cells.bottom_right;
        let upper_row = ((cells.top + cells.top_left) + cells.top) + cells.top_right;
        let gradient_y = (lower_row - upper_row) * 0.125;
        let right_column = ((cells.right + cells.top_right) + cells.right) + cells.bottom_right;
        let left_column = ((cells.left + cells.top_left) + cells.left) + cells.bottom_left;
        let gradient_x = (right_column - left_column) * 0.125;
        let gradient = (gradient_x * gradient_x + gradient_y * gradient_y).sqrt();
        let steepness = (gradient * (1.0 - sharp_share) - sharp_share * pull) * sharpness;
        let ridge = maxss(0.0, steepness - cell_size * 0.3) * 0.7;
        let cliff = maxss(0.0, steepness - cell_size) * 30.0;
        let relax = (1.0 - sharpness) * 0.02 * self.relaxation * above_mean;
        // The native code multiplies by -1 (a NaN keeps its sign).
        #[allow(clippy::neg_multiply)]
        let pushed = (ridge + cliff + relax) * -1.0;
        maxss(pull, pushed)
    }

    /// One shaping pass over the level,
    /// counted as wear.
    fn reshape(&self, level: &mut Level) {
        let cell_size = (self.terrain_width / self.terrain_height) / level.width as f32;
        for y in 0..level.height as i32 {
            for x in 0..level.width as i32 {
                let index = level.cell(x, y);
                let cells = Neighbourhood::around(&level.heights, level.width, level.height, x, y);
                let shaped = self.shaping_at(&cells, cell_size) * self.shape;
                level.shaping[index] = shaped;
                level.wear[index] += shaped;
            }
        }
        apply_shaping(level);
    }

    /// The shaping
    /// pass under a precipitation mask. Shaping only happens where water
    /// has run: the wetness each cell has seen spreads to every neighbour
    /// that lies a little below it.
    fn reshape_wet(&self, level: &mut Level) {
        let cell_size = (self.terrain_width / self.terrain_height) / level.width as f32;
        let width = level.width;
        let height = level.height;
        let mut precipitation = level.precipitation.take().expect("the level has a precipitation mask");
        for (wetness, &flow) in precipitation.wetness.iter_mut().zip(&level.flow) {
            let wet = minss(1.0, 0.25 * flow);
            if wet > *wetness {
                *wetness = wet;
            }
        }

        let side_drop = cell_size * SIDE_DROP;
        let diagonal_drop = side_drop * std::f32::consts::SQRT_2;
        for y in 0..height as i32 {
            for x in 0..width as i32 {
                let index = level.cell(x, y);
                let cells = Neighbourhood::around(&level.heights, width, height, x, y);
                let wetness = Neighbourhood::around(&precipitation.wetness, width, height, x, y);
                let side_limit = cells.center - side_drop;
                let diagonal_limit = cells.center - diagonal_drop;
                let reach = |neighbour: f32, limit: f32, wet: f32| {
                    let below = if neighbour < limit { 1.0 } else { 0.0 };
                    below * wet
                };
                // The neighbours pair in this order before taking the maximum.
                let bottom_pair = maxss(
                    reach(cells.bottom_right, diagonal_limit, wetness.bottom_right),
                    reach(cells.bottom, side_limit, wetness.bottom),
                );
                let crossed_pair = maxss(
                    reach(cells.bottom_left, diagonal_limit, wetness.bottom_left),
                    reach(cells.right, side_limit, wetness.right),
                );
                let side_pair = maxss(
                    reach(cells.left, side_limit, wetness.left),
                    reach(cells.top_right, diagonal_limit, wetness.top_right),
                );
                let top_pair = maxss(
                    reach(cells.top, side_limit, wetness.top),
                    reach(cells.top_left, diagonal_limit, wetness.top_left),
                );
                let lower = maxss(bottom_pair, crossed_pair);
                let upper = maxss(maxss(side_pair, top_pair), wetness.center);
                let spread = maxss(lower, upper);
                precipitation.spread[index] = spread;

                let shaped = self.shaping_at(&cells, cell_size) * self.shape * spread;
                level.shaping[index] = shaped;
                level.wear[index] += shaped;
            }
        }
        apply_shaping(level);
        std::mem::swap(&mut precipitation.wetness, &mut precipitation.spread);
        level.precipitation = Some(precipitation);
    }
}

/// A cell and its eight neighbours, clamped at the edges.
struct Neighbourhood {
    top_left: f32,
    top: f32,
    top_right: f32,
    left: f32,
    center: f32,
    right: f32,
    bottom_left: f32,
    bottom: f32,
    bottom_right: f32,
}

impl Neighbourhood {
    fn around(values: &[f32], width: usize, height: usize, x: i32, y: i32) -> Self {
        let above = (y - 1).max(0);
        let below = (y + 1).min(height as i32 - 1);
        let left = (x - 1).max(0);
        let right = (x + 1).min(width as i32 - 1);
        let at = |column: i32, row: i32| values[(row * width as i32 + column) as usize];
        Self {
            top_left: at(left, above),
            top: at(x, above),
            top_right: at(right, above),
            left: at(left, y),
            center: at(x, y),
            right: at(right, y),
            bottom_left: at(left, below),
            bottom: at(x, below),
            bottom_right: at(right, below),
        }
    }
}

/// Adds the shaping changes to the heights and clears them.
fn apply_shaping(level: &mut Level) {
    for (height, shaping) in level.heights.iter_mut().zip(level.shaping.iter_mut()) {
        *height += *shaping;
        *shaping = 0.0;
    }
}

/// What `ParallelErosion_Erode` works out before it erodes: the settings
/// as adjusted, the discharge angles' tangents, the pyramid's size in
/// octaves and levels, and the jitter directions.
pub(crate) struct Prepared {
    pub(crate) settings: ErosionSettings,
    pub(crate) suspended_tangent: f32,
    pub(crate) bed_tangent: f32,
    pub(crate) coarse_tangent: f32,
    pub(crate) octaves: f32,
    pub(crate) detail_octave: f32,
    pub(crate) level_count: i32,
    pub(crate) directions: Vec<[f32; 2]>,
}

pub(crate) fn prepare(settings: &ErosionSettings, width: usize, terrain: (f32, f32)) -> Prepared {
    let (terrain_width, _) = terrain;
    let mut settings = *settings;

    let mut scale_ratio = settings.erosion_scale / terrain_width;
    if scale_ratio > 0.2 {
        let excess = (scale_ratio - 0.2) * 6.0 + 1.0;
        scale_ratio = (1.0 - 1.0 / excess) * 0.16666667 + 0.2;
    }
    settings.erosion_scale = scale_ratio * terrain_width;
    if settings.no_erosion {
        settings.downcutting = 0.0;
    }

    let to_radians = |angle: f32| (angle / 180.0) * std::f32::consts::PI;
    let bed_tangent = maths::tan(to_radians(settings.bed_load_angle));
    let coarse_tangent = maths::tan(to_radians(settings.coarse_sediments_angle));
    let suspended_tangent = maths::tan(to_radians(settings.suspended_load_angle));
    let cell_metres = terrain_width / width as f32;
    let octaves = maths::log2((settings.erosion_scale * 0.5) / cell_metres);
    let detail_octave = if settings.shape_detail_scale == 0.0 {
        100.0
    } else {
        maths::log2(1.0 / settings.shape_detail_scale)
    };
    let level_count = cvttss2si(octaves.ceil());
    Prepared {
        settings,
        suspended_tangent,
        bed_tangent,
        coarse_tangent,
        octaves,
        detail_octave,
        level_count,
        directions: jitter_directions(settings.seed),
    }
}

/// `ParallelErosion_Erode`: erodes `heightmap` (optionally only where
/// `precipitation` falls) on a terrain of `terrain` (width, height) metres.
/// It runs as SYCL kernels on a discrete GPU for builds wider than 2560
/// pixels (`Erosions.PE`); otherwise as native CPU code.
pub fn erode(heightmap: &Map, precipitation: Option<&Map>, settings: &ErosionSettings, terrain: (f32, f32)) -> ErosionMaps {
    let cpu = || erode_cpu(heightmap, precipitation, settings, terrain);
    let on_gpu = sycl::discrete_gpu_active() && heightmap.width() > 2560;
    if !on_gpu {
        return cpu();
    }
    let gpu = |device: &nerthus_compute::GpuDevice| gpu::parallel_erosion::erode(device, heightmap, precipitation, settings, terrain);
    sycl::run("ParallelErosionSimulation", heightmap.width(), sycl::DISCRETE, gpu, cpu)
}

/// [`erode`] on the CPU (the native `ParallelErosion_Erode`).
pub fn erode_cpu(heightmap: &Map, precipitation: Option<&Map>, settings: &ErosionSettings, terrain: (f32, f32)) -> ErosionMaps {
    let resolution = heightmap.resolution();
    let (terrain_width, terrain_height) = terrain;
    let prepared = prepare(settings, resolution.x, terrain);
    let settings = prepared.settings;
    let level_count = prepared.level_count;

    let mask = precipitation.map(|map| map.data().to_vec());
    let finest = Level::new(resolution.x, resolution.y, heightmap.data().to_vec(), mask);
    let mut levels = vec![finest];
    for _ in 1..level_count {
        let finer = levels.last_mut().expect("the pyramid has a finest level");
        let coarser = finer.coarsen();
        levels.push(coarser);
    }

    let mut simulation = Simulation {
        settings,
        terrain_width,
        terrain_height,
        directions: prepared.directions,
        suspended_tangent: prepared.suspended_tangent,
        bed_tangent: prepared.bed_tangent,
        coarse_tangent: prepared.coarse_tangent,
        has_precipitation: precipitation.is_some(),
        octaves: prepared.octaves,
        detail_octave: prepared.detail_octave,
        finished_levels: 0,
        downcutting: 0.0,
        shape: 0.0,
        relaxation: 1.0,
    };
    while levels.len() > 1 {
        let mut coarser = levels.pop().expect("more than one level is left");
        simulation.erode_level(&mut coarser);
        let cell_size = (terrain_width / coarser.width as f32) / terrain_height;
        let finer = levels.last_mut().expect("a finer level is left");
        finer.refine(&coarser, cell_size);
        simulation.finished_levels += 1;
    }
    let mut finest = levels.pop().expect("the finest level is left");
    simulation.erode_level(&mut finest);

    let flow: Vec<f32> = finest.flow.iter().map(|&visits| minss(1.0, FLOW_SCALE * visits)).collect();
    ErosionMaps {
        height: Map::from_data(resolution, finest.heights),
        flow: Map::from_data(resolution, flow),
        deposition: Map::from_data(resolution, finest.deposition),
        wear: Map::from_data(resolution, finest.wear),
    }
}
