//! `HydrologicNative::rivers`: headwaters
//! traced downhill over a resolution pyramid, carving channels and
//! building the water depth, surface and flow-direction maps.

use nerthus_noise::FastNoise;
use rayon::prelude::*;

use super::drainage::{drainage_tree, raise_pit, smooth};
use crate::ce::kernel::{downsample, kernel_sum, upsample, FloatMap};

/// Kernel weights the river passes share.
const CORNER: f32 = 21.0;
const EDGE: f32 = 31.0;
const CENTRE: f32 = 48.0;
/// Visits a cell needs before it counts as river water.
const WATER_THRESHOLD: f32 = 20.0;
/// The grey of a flat direction (`0x3f008081`, 128/255).
const NEUTRAL: f32 = f32::from_bits(0x3f00_8081);
/// Height kept between the water surface and the bed.
const SURFACE_GAP: f32 = f32::from_bits(0x3727_c5ac);
/// Trace momentum scale.
const MOMENTUM: f32 = f32::from_bits(0x3851_b717);
const SMOOTH_CORNER: f32 = f32::from_bits(0x3d9a_042a);
const SMOOTH_EDGE: f32 = f32::from_bits(0x3dfd_eb30);
const SMOOTH_CENTRE: f32 = f32::from_bits(0x3e50_214d);
const RAND_MAX: f32 = 32767.0;

/// The Rivers node's parameters (`RiversParameters`).
#[derive(Clone, Copy, Debug)]
pub struct RiverSettings {
    pub seed: i32,
    pub width: f32,
    pub depth: f32,
    pub headwaters: i32,
    pub water: f32,
    pub render_surface: bool,
    pub downcutting: f32,
}

/// What `MakeRivers` returns.
#[derive(Clone, Debug)]
pub struct RiverOutput {
    pub heights: Vec<f32>,
    pub depth: Vec<f32>,
    pub surface: Vec<f32>,
    /// Three floats per pixel.
    pub direction: Vec<f32>,
}

/// MSVC's `rand()` (per-thread `holdrand`).
struct CrtRandom {
    state: u32,
}

impl CrtRandom {
    fn new(seed: u32) -> CrtRandom {
        CrtRandom { state: seed }
    }

    fn next(&mut self) -> i32 {
        self.state = self.state.wrapping_mul(214_013).wrapping_add(2_531_011);
        ((self.state >> 16) & 0x7fff) as i32
    }
}

/// `max(|x|, |y|)` the way the passes compute it.
fn largest_component(x: f32, y: f32) -> f32 {
    let horizontal = x.abs();
    let vertical = y.abs();
    if vertical <= horizontal { horizontal } else { vertical }
}

/// `x <= y ? x : y`.
fn lower(current: f32, candidate: f32) -> f32 {
    if current <= candidate { current } else { candidate }
}

/// One pyramid level of the river engine.
struct RiverLevel {
    size: usize,
    level: i32,
    /// Whether this is the coarsest level.
    coarsest: bool,
    /// [0] how often traces visited each cell.
    water: FloatMap,
    /// [2] carving of the (perturbed) terrain.
    correction: Vec<f32>,
    /// [3] smoothing scratch.
    scratch: Vec<f32>,
    /// [4] carving of the original terrain.
    original_correction: Vec<f32>,
    /// [5] flow vectors from the flood.
    vectors: Vec<[f32; 2]>,
    /// [10] drainage tree.
    parents: Vec<(i32, i32)>,
    /// [12] terrain detail lost to the coarser level.
    residual: Vec<f32>,
    /// [13] terrain (noise-perturbed at the start).
    height: FloatMap,
    /// [17] the original terrain.
    original: FloatMap,
    /// [18] original-terrain detail lost to the coarser level.
    original_residual: Vec<f32>,
}

impl RiverLevel {
    fn new(level: i32, size: usize, height: FloatMap, original: FloatMap) -> RiverLevel {
        let count = size * size;
        RiverLevel {
            size,
            level,
            coarsest: false,
            water: FloatMap::new(size as i32, 0.0),
            correction: vec![0.0; count],
            scratch: vec![0.0; count],
            original_correction: vec![0.0; count],
            vectors: vec![[0.0, 0.0]; count],
            parents: Vec::new(),
            residual: Vec::new(),
            height,
            original,
            original_residual: Vec::new(),
        }
    }

    fn index(&self, x: i32, y: i32) -> usize {
        (y * self.size as i32 + x) as usize
    }

    /// The half-resolution level with the rivers maps, recording what both
    /// terrains lose.
    fn coarser(&mut self) -> RiverLevel {
        let fine_size = self.size;
        let size = fine_size >> 1;
        let sum = kernel_sum();
        let shrink = |map: &FloatMap| {
            let mut coarse = FloatMap::new(size as i32, 0.0);
            coarse.data.par_chunks_mut(size).enumerate().for_each(|(y, row)| {
                for (x, value) in row.iter_mut().enumerate() {
                    *value = downsample(map, x as i32, y as i32, sum);
                }
            });
            coarse
        };
        let height = shrink(&self.height);
        let original = shrink(&self.original);
        let lost = |fine: &FloatMap, coarse: &FloatMap| {
            let mut residual = vec![0.0f32; fine_size * fine_size];
            residual.par_chunks_mut(fine_size).enumerate().for_each(|(y, row)| {
                for (x, value) in row.iter_mut().enumerate() {
                    let smooth = upsample(coarse, x as u32, y as u32);
                    *value = fine.data[y * fine_size + x] - smooth;
                }
            });
            residual
        };
        self.residual = lost(&self.height, &height);
        self.original_residual = lost(&self.original, &original);
        RiverLevel::new(self.level + 1, size, height, original)
    }

    /// The level restored with the rivers maps: `fine` takes this level's
    /// terrains upsampled plus their lost detail; the original terrain's
    /// detail fades where much water passed.
    fn restore(&self, fine: &mut RiverLevel) {
        let size = fine.size;
        let residual = std::mem::take(&mut fine.residual);
        let original_residual = std::mem::take(&mut fine.original_residual);
        fine.height.data.par_chunks_mut(size).zip(fine.original.data.par_chunks_mut(size)).enumerate().for_each(|(y, (height_row, original_row))| {
            for x in 0..size {
                let index = y * size + x;
                let smooth = upsample(&self.height, x as u32, y as u32);
                height_row[x] = smooth + residual[index];
                let water = upsample(&self.water, x as u32, y as u32);
                let mut fade = ((water * -0.25) as f64).exp() as f32;
                let detail = original_residual[index];
                if 0.0 < detail {
                    fade = 1.0;
                }
                let smooth_original = upsample(&self.original, x as u32, y as u32);
                original_row[x] = smooth_original + detail * fade;
            }
        });
    }
}

/// A point of a coarse trace kept for downcutting.
#[derive(Clone, Copy)]
struct PathPoint {
    x: i32,
    y: i32,
    level: f32,
    original_level: f32,
}

struct Rivers {
    settings: RiverSettings,
    /// The downcutting setting after `(exp(4 d) - 1) / (exp(4) - 1)`.
    downcutting: f32,
    /// Water times 4000.
    water: f32,
    jitter: [[f32; 2]; 32],
    sources: Vec<[f32; 2]>,
    path: Vec<PathPoint>,
    /// Levels flooded so far (starting at -1).
    floods: i32,
    /// Flow directions accumulated at full resolution.
    directions: Vec<[f32; 2]>,
}

fn expf(value: f32) -> f32 {
    (value as f64).exp() as f32
}

impl Rivers {
    /// The parameter set-up and the seeded headwaters at the start of
    /// `rivers`.
    fn new(settings: RiverSettings, size: usize, density: Option<&[f32]>) -> Rivers {
        let mut downcutting = settings.downcutting;
        if 0.0 < downcutting {
            let scale = 4.0f32;
            downcutting = (expf(downcutting * scale) - 1.0) / (expf(scale) - 1.0);
        }
        let mut random = CrtRandom::new(settings.seed as u32);
        let mut jitter = [[0.0f32; 2]; 32];
        for vector in jitter.iter_mut() {
            let first = (random.next() & 0xff) as f32 / 255.0;
            let y = (first + first) - 1.0;
            let second = (random.next() & 0xff) as f32 / 255.0;
            let x = (second + second) - 1.0;
            let mut value = [x, y];
            if 0.0 < x * x + y * y {
                let largest = largest_component(x, y);
                value = [x / largest, y / largest];
            }
            *vector = value;
        }

        let mut random = CrtRandom::new(settings.seed as u32);
        let mut sources = Vec::new();
        let resolution = size as f32;
        for _ in 0..settings.headwaters.max(0) {
            let first = random.next();
            let second = random.next();
            let third = random.next();
            let x = first as f32 / RAND_MAX;
            let y = second as f32 / RAND_MAX;
            match density {
                None => sources.push([x, y]),
                Some(density) => {
                    let column = (resolution * x) as i32;
                    let row = (resolution * y) as i32;
                    let index = (column + row * size as i32) as usize;
                    let chance = density.get(index).copied().unwrap_or(0.0) * f32::from_bits(0x3f80_20c5);
                    if (third as f32) / RAND_MAX < chance {
                        sources.push([x, y]);
                    }
                }
            }
        }
        Rivers {
            settings,
            downcutting,
            water: settings.water * 4000.0,
            jitter,
            sources,
            path: Vec::new(),
            floods: -1,
            directions: Vec::new(),
        }
    }

    /// The drainage tree and normalised
    /// flow vectors of a level.
    fn flood(&mut self, level: &mut RiverLevel) {
        self.floods += 1;
        let size = level.size;
        level.parents = drainage_tree(&level.height.data, size, Some(&mut level.vectors));
        level.vectors.par_iter_mut().for_each(|vector| {
            let [mut x, mut y] = *vector;
            if x != 0.0 || y != 0.0 {
                let length = (x * x + y * y).sqrt();
                let inverse = 1.0 / length;
                x *= inverse;
                y *= inverse;
            }
            if x != 0.0 || y != 0.0 {
                let largest = largest_component(x, y);
                x /= largest;
                y /= largest;
            }
            *vector = [x, y];
        });
    }

    /// The pit raise at one cell of both terrains.
    fn raise(&self, level: &mut RiverLevel, x: i32, y: i32) {
        let n = level.size as i32;
        raise_pit(&mut level.height.data, n, x, y);
        raise_pit(&mut level.original.data, n, x, y);
    }

    /// A coarse trace follows the drainage tree from
    /// (`x`, `y`), carving to the level it entered at.
    fn trace_coarse(&mut self, level: &mut RiverLevel, mut x: i32, mut y: i32) {
        self.path.clear();
        let n = level.size as i32;
        let step = -0.0f32 / n as f32;
        let mut level_height = level.height.data[level.index(x, y)];
        let mut level_original = level.original.data[level.index(x, y)];
        loop {
            if 0.0 < self.downcutting {
                let point = PathPoint {
                    x,
                    y,
                    level: level_height,
                    original_level: level_original,
                };
                self.path.push(point);
            }
            level_height += step;
            level_original += step;
            if x == 0 || y == 0 || n - 1 <= x || n - 1 <= y {
                break;
            }
            if 0xff < n {
                self.raise(level, x, y);
            }
            let (px, py) = level.parents[level.index(x, y)];
            if (px - x).abs() > 1 || (py - y).abs() > 1 {
                break;
            }
            let index = level.index(px, py);
            let height = level.height.data[index];
            let carved = lower(level_height, height);
            if carved - height < level.correction[index] {
                level.correction[index] = carved - height;
            }
            level.water.data[index] += 1.0;
            level_height = height + level.correction[index];
            x = px;
            y = py;
            let original = level.original.data[index];
            let carved = lower(level_original, original);
            if carved - original < level.original_correction[index] {
                level.original_correction[index] = carved - original;
            }
            level_original = original + level.original_correction[index];
        }
        if 0.0 < self.downcutting && !level.coarsest {
            self.downcut(level);
        }
    }

    /// Re-grades the recorded path towards the steepest
    /// fall found two, six and eleven points ahead.
    fn downcut(&self, level: &mut RiverLevel) {
        let path = &self.path;
        let length = path.len() as i32;
        if length < 2 {
            return;
        }
        let last = length - 1;
        let mut level_height = path[0].level;
        let mut level_original = path[0].original_level;
        let mut slope = 0.0f32;
        let mut original_slope = 0.0f32;
        let mut target = 0i32;
        let mut original_target = 0i32;
        let mut near = 2i32;
        let mut middle = 6i32;
        let mut far = 11i32;
        for index in 0..=length - 2 {
            near = near.min(last);
            middle = middle.min(last);
            far = far.min(last);
            let point = path[index as usize];
            let at = |position: i32| path[position as usize];

            if point.level < level_height {
                slope = 0.0;
                level_height = point.level;
                target = index;
            }
            let fallback = middle - 1;
            let near_slope = (at(near).level - level_height) / (near - index) as f32;
            let far_span = ((far - index) - 1) as f32;
            let middle_span = (middle - index) as f32 - 0.5;
            let far_slope = (at(far).level - level_height) / far_span;
            let middle_slope = ((at(middle).level + at(middle - 1).level) * 0.5 - level_height) / middle_span;
            if near_slope < slope {
                slope = near_slope;
                target = near;
            }
            let mut chosen = fallback;
            let mut best = middle_slope;
            if slope <= middle_slope {
                chosen = target;
                best = slope;
            }
            target = far - 1;
            slope = far_slope;
            if best <= far_slope {
                target = chosen;
                slope = best;
            }
            let cell = level.index(point.x, point.y);
            let current = level.correction[cell];
            level.correction[cell] = ((level_height - level.height.data[cell]) - current) * self.downcutting + current;
            if index < target - 1 {
                level_height += slope;
            } else {
                slope = 0.0;
            }

            let mut base = level_original;
            if point.original_level < level_original {
                original_slope = 0.0;
                base = point.original_level;
            }
            let near_original = (at(near).original_level - base) / (near - index) as f32;
            let mut chosen_original = if level_original <= point.original_level { original_target } else { index };
            let far_original = (at(far).original_level - base) / far_span;
            let middle_original = ((at(middle - 1).original_level + at(middle).original_level) * 0.5 - base) / middle_span;
            if near_original < original_slope {
                original_slope = near_original;
                chosen_original = near;
            }
            let mut fallback_original = fallback;
            let mut best_original = middle_original;
            if original_slope <= middle_original {
                fallback_original = chosen_original;
                best_original = original_slope;
            }
            original_slope = if far_original < best_original { far_original } else { best_original };
            original_target = if best_original <= far_original { fallback_original } else { far - 1 };
            let current = level.original_correction[cell];
            level.original_correction[cell] = ((base - level.original.data[cell]) - current) * self.downcutting + current;
            if index < original_target - 1 {
                base += original_slope;
            } else {
                original_slope = 0.0;
            }
            level_original = base;

            near += 1;
            middle += 1;
            far += 1;
        }
    }

    /// Bends the flow vector at (`x`, `y`) towards the
    /// visit-weighted average around it (and towards the drainage tree a
    /// few steps on).
    fn widen(&self, level: &mut RiverLevel, x: i32, y: i32, amount: f32) {
        let n = level.size as i32;
        let left = (x - 1).max(0);
        let right = (x + 1).min(n - 1);
        let top = (y - 1).max(0);
        let bottom = (y + 1).min(n - 1);
        let water = |cx: i32, cy: i32| level.water.data[(cy * n + cx) as usize];
        let vector = |cx: i32, cy: i32| level.vectors[(cy * n + cx) as usize];
        let w_top_left = water(left, top) * CORNER;
        let w_top = water(x, top) * EDGE;
        let w_top_right = water(right, top) * CORNER;
        let w_left = water(left, y) * EDGE;
        let w_centre = water(x, y) * CENTRE;
        let w_right = water(right, y) * EDGE;
        let w_bottom_left = water(left, bottom) * CORNER;
        let w_bottom = water(x, bottom) * EDGE;
        let w_bottom_right = water(right, bottom) * CORNER;
        let centre = vector(x, y);

        let mut pull = [0.0f32, 0.0f32];
        let mut pull_weight = 0.0f32;
        let interior = x != 0 && y != 0 && x != n - 1 && y != n - 1;
        if interior {
            let reach = (n * 3) / 512;
            let mut fx = x as f32;
            let mut fy = y as f32;
            let start_x = x as f32;
            let start_y = y as f32;
            let mut steps = 0;
            if 0 < reach {
                loop {
                    let (px, py) = level.parents[((fy as i32) * n + fx as i32) as usize];
                    fx = px as f32;
                    fy = py as f32;
                    if fx as i32 == 0 || fy as i32 == 0 || fx as i32 == n - 1 || fy as i32 == n - 1 {
                        break;
                    }
                    steps += 1;
                    if reach <= steps {
                        break;
                    }
                }
            }
            let dx = fx - start_x;
            let dy = fy - start_y;
            let largest = largest_component(dx, dy);
            pull = [dx / largest, dy / largest];
            pull_weight = 10.0;
        }

        let weight = w_top_left + 0.0 + w_top + w_top_right + w_left + w_centre + w_right + w_bottom_left + w_bottom + w_bottom_right + pull_weight;
        let sum_x = pull[0] * pull_weight
            + (w_top_left * vector(left, top)[0] + 0.0 + w_top * vector(x, top)[0] + w_top_right * vector(right, top)[0] + w_left * vector(left, y)[0] + centre[0] * w_centre + w_right * vector(right, y)[0] + w_bottom_left * vector(left, bottom)[0] + w_bottom * vector(x, bottom)[0] + w_bottom_right * vector(right, bottom)[0]);
        let sum_y = pull[1] * pull_weight
            + (w_top_left * vector(left, top)[1] + 0.0 + w_top * vector(x, top)[1] + w_top_right * vector(right, top)[1] + w_left * vector(left, y)[1] + centre[1] * w_centre + w_right * vector(right, y)[1] + w_bottom_left * vector(left, bottom)[1] + w_bottom * vector(x, bottom)[1] + w_bottom_right * vector(right, bottom)[1]);
        if 0.0 < weight && (sum_x != 0.0 || sum_y != 0.0) {
            let mean_x = sum_x / weight;
            let mean_y = sum_y / weight;
            let largest = largest_component(mean_x, mean_y);
            let blended_x = (mean_x / largest) * amount + centre[0] * (1.0 - amount);
            let blended_y = (mean_y / largest) * amount + (1.0 - amount) * centre[1];
            let largest = largest_component(blended_x, blended_y);
            let index = level.index(x, y);
            level.vectors[index] = [blended_x / largest, blended_y / largest];
        }
    }

    /// The visit-weighted original-terrain level around a
    /// cell.
    fn original_level(level: &RiverLevel, x: i32, y: i32) -> f32 {
        let n = level.size as i32;
        let left = (x - 1).max(0);
        let right = (x + 1).min(n - 1);
        let top = (y - 1).max(0);
        let bottom = (y + 1).min(n - 1);
        let water = |cx: i32, cy: i32| level.water.data[(cy * n + cx) as usize];
        let carved = |cx: i32, cy: i32| {
            let index = (cy * n + cx) as usize;
            level.original_correction[index] + level.original.data[index]
        };
        let w_top_left = water(left, top) * CORNER;
        let w_top = water(x, top) * EDGE;
        let w_top_right = water(right, top) * CORNER;
        let w_left = water(left, y) * EDGE;
        let w_centre = water(x, y) * 24.0;
        let w_right = water(right, y) * EDGE;
        let w_bottom_left = water(left, bottom) * CORNER;
        let w_bottom = water(x, bottom) * EDGE;
        let w_bottom_right = water(right, bottom) * CORNER;
        let centre = carved(x, y);
        let weight = w_top_left + 0.0 + w_top + w_top_right + w_left + w_centre + w_right + w_bottom_left + w_bottom + w_bottom_right;
        if !(0.0 < weight) {
            return centre;
        }
        let sum = carved(left, top) * w_top_left + 0.0 + carved(x, top) * w_top + carved(right, top) * w_top_right + carved(left, y) * w_left + centre * w_centre + carved(right, y) * w_right + carved(left, bottom) * w_bottom_left + carved(x, bottom) * w_bottom;
        (carved(right, bottom) * w_bottom_right + sum) / weight
    }

    /// A full-resolution trace in continuous space from
    /// (`x`, `y`), steered by the flow vectors, the water already traced,
    /// its own momentum and the seeded jitter.
    fn trace_fine(&mut self, level: &mut RiverLevel, mut x: f32, mut y: f32, mut counter: u32) {
        let n = level.size as i32;
        let width = self.settings.width;
        let strength = ((n as f32 + n as f32) * MOMENTUM) * width;
        let mut level_height = level.height.data[level.index(x as i32, y as i32)];
        let mut level_original = level.original.data[level.index(x as i32, y as i32)];
        let mut previous = [0.0f32, 0.0f32];
        let mut last_depth = 0.0f32;
        let mut steps = 0i32;
        if x as i32 == 0 {
            return;
        }
        while y as i32 != 0 {
            let ix = x as i32;
            let iy = y as i32;
            if n - 1 <= ix || n - 1 <= iy {
                return;
            }
            self.raise(level, ix, iy);
            counter = (counter + 1) & 0x1f;
            let fy = y - 0.5;
            let fx = x - 0.5;
            let row = fy as i32;
            let column = fx as i32;
            let water_here = level.water.data[level.index(ix, iy)] - WATER_THRESHOLD;
            let depth_here = if 0.0 <= water_here { water_here } else { 0.0 };

            let clamp = |value: i32| if value < n - 1 { value } else { n - 1 };
            let column0 = clamp(if -1 < column { column } else { 0 });
            let column1 = clamp(if -1 < column { column + 1 } else { 1 });
            let row0 = clamp(if -1 < row { row } else { 0 });
            let row1 = clamp(if -1 < row { row + 1 } else { 1 });
            let tx = fx - column0 as f32;
            let ty = fy - row0 as f32;
            let v00 = level.vectors[level.index(column0, row0)];
            let v10 = level.vectors[level.index(column1, row0)];
            let v01 = level.vectors[level.index(column0, row1)];
            let v11 = level.vectors[level.index(column1, row1)];
            let mut flow_x = ((1.0 - tx) * v00[0] + tx * v10[0]) * (1.0 - ty) + (tx * v11[0] + (1.0 - tx) * v01[0]) * ty;
            let mut flow_y = (tx * v10[1] + (1.0 - tx) * v00[1]) * (1.0 - ty) + (tx * v11[1] + (1.0 - tx) * v01[1]) * ty;
            if flow_x == 0.0 && flow_y == 0.0 {
                return;
            }
            let largest = largest_component(flow_x, flow_y);
            flow_x /= largest;
            flow_y /= largest;

            let depth_at = |cx: i32, cy: i32| {
                let value = level.water.data[(cy * n + cx) as usize] - WATER_THRESHOLD;
                (if 0.0 <= value { value } else { 0.0 }) + 0.0
            };
            let d00 = depth_at(column, row);
            let d10 = depth_at(column + 1, row);
            let d01 = depth_at(column, row + 1);
            let d11 = depth_at(column + 1, row + 1);
            let frac_y = fy - row as f32;
            let frac_x = fx - column as f32;
            let mut slope_x = (d10 - d00) * (1.0 - frac_y) + (d11 - d01) * frac_y;
            let mut slope_y = (d01 - d00) * (1.0 - frac_x) + (d11 - d10) * frac_x;
            if slope_x != 0.0 || slope_y != 0.0 {
                let largest = largest_component(slope_x, slope_y);
                slope_x /= largest;
                slope_y /= largest;
            }
            let deepest = if depth_here <= last_depth { last_depth } else { depth_here };
            let mut move_x = flow_x;
            let mut move_y = flow_y;
            if 0.0 <= previous[1] * flow_y + previous[0] * flow_x {
                let jitter = self.jitter[counter as usize];
                move_x = previous[0] * deepest * strength + flow_x + width * 0.2 * jitter[0] + (-slope_x) * 0.04 * deepest * width * strength;
                move_y = previous[1] * deepest * strength + flow_y + width * 0.2 * jitter[1] + (-slope_y) * 0.04 * deepest * width * strength;
            }
            let mut step_x = flow_x;
            let mut step_y = flow_y;
            if move_x != 0.0 || move_y != 0.0 {
                let largest = largest_component(move_x, move_y);
                step_x = move_x / largest;
                step_y = move_y / largest;
            }
            x += step_x;
            y += step_y;
            let here = level.index(ix, iy);
            self.directions[here][0] += move_x;
            self.directions[here][1] += move_y;
            let nx = x as i32;
            if (nx as f32) < 0.0 || n - 1 < nx {
                return;
            }
            let ny = y as i32;
            if (ny as f32) < 0.0 || n - 1 < ny {
                return;
            }
            let next = level.index(nx, ny);
            if level.level == 0 {
                self.directions[next][0] += move_x;
                self.directions[next][1] += move_y;
            }
            let height = level.height.data[next];
            let carved = lower(level_height, height);
            if carved - height < level.correction[next] {
                level.correction[next] = carved - height;
            }
            level_height = height + level.correction[next];
            let original = level.original.data[next];
            let carved = lower(level_original, original);
            if carved - original < level.original_correction[next] {
                level.original_correction[next] = carved - original;
            }
            let mut smoothed = original;
            if 1 < steps {
                smoothed = Self::original_level(level, nx, ny);
            }
            if smoothed - original < level.original_correction[next] {
                level.original_correction[next] = smoothed - original;
            }
            level_original = smoothed;

            level.water.data[next] += 1.0;
            let visits = level.water.data[next];
            if visits <= 1.0 || (500.0 - width * 420.0) < visits || counter == 0 {
                let spread = visits * 0.04 * width;
                let amount = if spread <= 1.0 { spread } else { 1.0 };
                self.widen(level, nx, ny, amount);
            }
            if (n as f32 + n as f32) < steps as f32 {
                return;
            }
            previous = [step_x, step_y];
            steps += 1;
            last_depth = depth_here;
            if nx == 0 {
                return;
            }
        }
    }

    /// Traces every headwater (cycling until the water
    /// amount is spent) and smooths the carving.
    fn trace_level(&mut self, level: &mut RiverLevel) {
        let n = level.size as i32;
        if self.sources.is_empty() {
            self.smooth_corrections(level);
            return;
        }
        if level.level < 1 {
            let requested = self.settings.headwaters;
            let accepted = self.sources.len() as f32;
            let ratio = ((n / 512) as f32 * self.water) / requested as f32;
            let per_source = if 1.0 <= ratio { ratio } else { 1.0 };
            let total = (per_source * accepted) as i32 as f32;
            let mut traced = 0.0f32;
            let mut counter = 0u32;
            'outer: loop {
                for source_index in 0..self.sources.len() {
                    let source = self.sources[source_index];
                    let span = n as f32 - 0.1;
                    let x = span * source[0];
                    let y = span * source[1];
                    self.trace_fine(level, x, y, counter);
                    counter = counter.wrapping_add(1);
                    traced += 1.0;
                    if total <= traced {
                        break 'outer;
                    }
                }
                if !(traced < total) {
                    break;
                }
            }
        } else {
            let count = (self.sources.len() as f32 * (self.water / self.settings.headwaters as f32)) as i32;
            let mut traced = 0;
            'coarse: loop {
                for source_index in 0..self.sources.len() {
                    let source = self.sources[source_index];
                    let resolution = n as f32;
                    let x = ((resolution * source[0]) as i32).min(n - 1);
                    let y = ((resolution * source[1]) as i32).min(n - 1);
                    self.trace_coarse(level, x, y);
                    traced += 1;
                    if count <= traced {
                        break 'coarse;
                    }
                }
                if count <= traced {
                    break;
                }
            }
            let fall = -0.005f32 / n as f32;
            let water = &level.water.data;
            level.correction.par_iter_mut().zip(water.par_iter()).for_each(|(correction, &visits)| {
                *correction += (visits.sqrt() * fall) * 4.0;
            });
        }
        self.smooth_corrections(level);
    }

    /// The smoothing schedule at the end of the headwater tracing.
    fn smooth_corrections(&self, level: &mut RiverLevel) {
        let size = level.size;
        if level.coarsest {
            smooth(&level.correction, &mut level.scratch, size);
            level.correction.copy_from_slice(&level.scratch);
            smooth(&level.original_correction, &mut level.scratch, size);
            level.original_correction.copy_from_slice(&level.scratch);
            return;
        }
        smooth(&level.correction, &mut level.scratch, size);
        smooth(&level.scratch, &mut level.correction, size);
        if 2 < self.floods {
            smooth(&level.correction, &mut level.scratch, size);
            smooth(&level.scratch, &mut level.correction, size);
        }
        if 1 < self.floods {
            terrain_smooth(&level.correction, &level.height.data, &mut level.scratch, &level.water.data, size);
            terrain_smooth(&level.scratch, &level.height.data, &mut level.correction, &level.water.data, size);
            if 2 < self.floods {
                terrain_smooth(&level.correction, &level.height.data, &mut level.scratch, &level.water.data, size);
                terrain_smooth(&level.scratch, &level.height.data, &mut level.correction, &level.water.data, size);
            }
        }
        smooth(&level.original_correction, &mut level.scratch, size);
        smooth(&level.scratch, &mut level.original_correction, size);
        if 2 < self.floods {
            smooth(&level.original_correction, &mut level.scratch, size);
            smooth(&level.scratch, &mut level.original_correction, size);
        }
        if 1 < self.floods {
            terrain_smooth(&level.original_correction, &level.original.data, &mut level.scratch, &level.water.data, size);
            terrain_smooth(&level.scratch, &level.original.data, &mut level.original_correction, &level.water.data, size);
            if 2 < self.floods {
                terrain_smooth(&level.original_correction, &level.original.data, &mut level.scratch, &level.water.data, size);
                terrain_smooth(&level.scratch, &level.original.data, &mut level.original_correction, &level.water.data, size);
            }
        }
    }
}

/// Smooths positive corrections away
/// from rivers, measured on the terrain they apply to.
fn terrain_smooth(source: &[f32], terrain: &[f32], target: &mut [f32], water: &[f32], size: usize) {
    let n = size as i32;
    target.par_chunks_mut(size).enumerate().for_each(|(y, row)| {
        let y = y as i32;
        let top = (y - 1).max(0);
        let bottom = (y + 1).min(n - 1);
        for x in 0..n {
            let index = (y * n + x) as usize;
            let value = source[index];
            if value <= 0.0 || water[index] != 0.0 {
                row[x as usize] = value;
                continue;
            }
            let left = (x - 1).max(0);
            let right = (x + 1).min(n - 1);
            let both = |cx: i32, cy: i32| {
                let cell = (cy * n + cx) as usize;
                terrain[cell] + source[cell]
            };
            let base = terrain[index];
            let total = both(left, top) * SMOOTH_CORNER + 0.0 + both(x, top) * SMOOTH_EDGE + both(right, top) * SMOOTH_CORNER + both(left, y) * SMOOTH_EDGE + (value + base) * SMOOTH_CENTRE + both(right, y) * SMOOTH_EDGE + both(left, bottom) * SMOOTH_CORNER + both(x, bottom) * SMOOTH_EDGE + both(right, bottom) * SMOOTH_CORNER;
            row[x as usize] = total - base;
        }
    });
}

/// The 3x3 taps of the river blurs, in the engine's order, with the weight
/// each uses (`None` for the centre).
fn blur_taps(x: i32, y: i32, n: i32) -> [(i32, i32, Option<f32>); 9] {
    let left = (x - 1).max(0);
    let right = (x + 1).min(n - 1);
    let top = (y - 1).max(0);
    let bottom = (y + 1).min(n - 1);
    [
        (left, top, Some(CORNER)),
        (x, top, Some(EDGE)),
        (right, top, Some(CORNER)),
        (left, y, Some(EDGE)),
        (x, y, None),
        (right, y, Some(EDGE)),
        (left, bottom, Some(CORNER)),
        (x, bottom, Some(EDGE)),
        (right, bottom, Some(CORNER)),
    ]
}

/// A 3x3 blur of the direction vectors over cells with
/// water (`all` cells are written; empty neighbourhoods keep their vector).
fn blur_directions(vectors: &[[f32; 2]], mask: &[f32], size: usize, centre_factor: f32, outside: f32) -> Vec<[f32; 2]> {
    let n = size as i32;
    let mut result = vec![[0.0f32; 2]; size * size];
    result.par_chunks_mut(size).enumerate().for_each(|(y, row)| {
        let y = y as i32;
        for x in 0..n {
            let mut sum = [0.0f32; 2];
            let mut weight = 0.0f32;
            for (cx, cy, tap) in blur_taps(x, y, n) {
                let cell = (cy * n + cx) as usize;
                if !(0.0 < mask[cell]) {
                    continue;
                }
                let tap_weight = tap.unwrap_or(centre_factor * CENTRE);
                sum[0] += vectors[cell][0] * tap_weight;
                sum[1] += vectors[cell][1] * tap_weight;
                weight += tap_weight;
            }
            let index = (y * n + x) as usize;
            row[x as usize] = if weight <= 0.0 {
                vectors[index]
            } else {
                let scale = if 0.0 < mask[index] { 1.0 } else { outside };
                [scale * (sum[0] / weight), scale * (sum[1] / weight)]
            };
        }
    });
    result
}

/// A 3x3 blur of `values` over cells where `mask` > 0
/// (every cell written; empty neighbourhoods become 0 or keep their value).
fn blur_masked(values: &[f32], mask: &[f32], size: usize, zero_outside: bool) -> Vec<f32> {
    let n = size as i32;
    let mut result = vec![0.0f32; size * size];
    result.par_chunks_mut(size).enumerate().for_each(|(y, row)| {
        let y = y as i32;
        for x in 0..n {
            let mut sum = 0.0f32;
            let mut weight = 0.0f32;
            for (cx, cy, tap) in blur_taps(x, y, n) {
                let cell = (cy * n + cx) as usize;
                if !(0.0 < mask[cell]) {
                    continue;
                }
                let tap_weight = tap.unwrap_or(CENTRE);
                sum += values[cell] * tap_weight;
                weight += tap_weight;
            }
            let index = (y * n + x) as usize;
            row[x as usize] = if weight <= 0.0 {
                if zero_outside { 0.0 } else { values[index] }
            } else {
                sum / weight
            };
        }
    });
    result
}

/// Smooths the water surface, never
/// above the bed beneath.
fn smooth_surface(surface: &[f32], mask: &[f32], heights: &[f32], size: usize) -> Vec<f32> {
    let n = size as i32;
    let mut result = vec![0.0f32; size * size];
    result.par_chunks_mut(size).enumerate().for_each(|(y, row)| {
        let y = y as i32;
        let top = (y - 1).max(0);
        let bottom = (y + 1).min(n - 1);
        for x in 0..n {
            let left = (x - 1).max(0);
            let right = (x + 1).min(n - 1);
            let index = (y * n + x) as usize;
            let at = |cx: i32, cy: i32| surface[(cy * n + cx) as usize];
            if mask[index] == 0.0 {
                let centre = at(x, y);
                let limit = heights[index] - SURFACE_GAP;
                let capped = |value: f32| if centre <= value { centre } else { value };
                let mean = (capped(at(left, top)) * CORNER + 0.0 + capped(at(x, top)) * EDGE + capped(at(right, top)) * CORNER + capped(at(left, y)) * EDGE + centre * 12.0 + capped(at(right, y)) * EDGE + capped(at(left, bottom)) * CORNER + capped(at(x, bottom)) * EDGE + capped(at(right, bottom)) * CORNER) / 220.0;
                let value = if mean <= limit { mean } else { limit };
                row[x as usize] = if centre <= value { centre } else { value };
                continue;
            }
            let mut sum = 0.0f32;
            let mut weight = 0.0f32;
            for (cx, cy, tap) in blur_taps(x, y, n) {
                if !(0.0 < mask[(cy * n + cx) as usize]) {
                    continue;
                }
                let tap_weight = tap.unwrap_or(12.0);
                weight += tap_weight;
                sum += at(cx, cy) * tap_weight;
            }
            row[x as usize] = if weight <= 0.0 {
                at(x, y)
            } else {
                let limit = heights[index] - SURFACE_GAP;
                let mean = sum / weight;
                if limit <= mean { limit } else { mean }
            };
        }
    });
    result
}

/// Relaxes the terrain beside rivers towards a blur that
/// stays on the right side of the water surface.
fn relax_banks(heights: &[f32], water: &[f32], surface: &[f32], size: usize, centre_factor: f32) -> Vec<f32> {
    let n = size as i32;
    let mut result = vec![0.0f32; size * size];
    result.par_chunks_mut(size).enumerate().for_each(|(y, row)| {
        let y = y as i32;
        let top = (y - 1).max(0);
        let bottom = (y + 1).min(n - 1);
        for x in 0..n {
            let left = (x - 1).max(0);
            let right = (x + 1).min(n - 1);
            let w = |cx: i32, cy: i32| water[(cy * n + cx) as usize];
            let h = |cx: i32, cy: i32| heights[(cy * n + cx) as usize];
            let visits = w(left, top) + 0.0 + w(x, top) + w(right, top) + w(left, y) + w(x, y) + w(right, y) + w(left, bottom) + w(x, bottom) + w(right, bottom);
            let share = (if 25.0 < visits { 25.0 } else { visits }) * 0.04;
            let centre = h(x, y);
            let centre_weight = centre_factor * CENTRE;
            let mut blurred = (h(left, top) * CORNER + 0.0 + h(x, top) * EDGE + h(right, top) * CORNER + h(left, y) * EDGE + centre * centre_weight + h(right, y) * EDGE + h(left, bottom) * CORNER + h(x, bottom) * EDGE + h(right, bottom) * CORNER) / (centre_weight + 104.0 + EDGE + CORNER + EDGE + CORNER);
            let index = (y * n + x) as usize;
            let level = surface[index];
            let keep = if w(x, y) <= 0.0 { level < blurred } else { blurred < level };
            if !keep && blurred != level {
                blurred = centre;
            }
            row[x as usize] = (1.0 - share) * centre + blurred * share;
        }
    });
    result
}

/// `log2(res) - 3` as `rivers` computes it (`log(res) / logf(2) - 3`).
fn river_levels(size: usize) -> i32 {
    let ratio = (size as f64).ln() / (2.0f32.ln() as f64);
    (ratio - 3.0) as i32
}

/// `HydrologicNative::rivers`.
pub fn rivers(settings: &RiverSettings, heights: &[f32], size: usize, density: Option<&[f32]>) -> RiverOutput {
    let mut engine = Rivers::new(*settings, size, density);
    let n = size as i32;

    // The terrain gets a little fractal noise (scaled by downcutting) so
    // flat ground still drains; the original is kept alongside.
    let amplitude = settings.downcutting * settings.downcutting * 0.025 + 0.005;
    let mut noise = FastNoise::new(1337);
    noise.set_frequency(64.0 / n as f32);
    let octaves = ((n as f32 * 0.015625) as f64).log2() as f32 as i32;
    noise.set_fractal_octaves(octaves);
    noise.set_seed(settings.seed);
    let mut perturbed = vec![0.0f32; size * size];
    perturbed.par_chunks_mut(size).enumerate().for_each(|(y, row)| {
        let values = noise.get_noise_set(0, y as i32, 0, size, 1, 1, 1.0);
        for (x, value) in row.iter_mut().enumerate() {
            *value = values[x] * amplitude + heights[y * size + x];
        }
    });
    let height = FloatMap {
        size: n,
        data: perturbed,
    };
    let original = FloatMap {
        size: n,
        data: heights.to_vec(),
    };
    let mut stack = vec![RiverLevel::new(0, size, height, original)];
    let levels = river_levels(size);
    if levels < 2 {
        stack[0].coarsest = true;
    } else {
        for _ in 0..levels - 1 {
            let finer = stack.last_mut().expect("the stack starts with the input level");
            let coarser = finer.coarser();
            stack.push(coarser);
        }
        if let Some(coarsest) = stack.last_mut() {
            coarsest.coarsest = true;
        }
        while stack.len() > 1 {
            let mut level = stack.pop().expect("more than one level is left");
            engine.flood(&mut level);
            engine.trace_level(&mut level);
            apply_corrections(&mut level);
            let finer = stack.last_mut().expect("a finer level is left");
            level.restore(finer);
        }
    }

    let mut level = stack.pop().expect("the input level is left");
    engine.flood(&mut level);
    engine.directions = vec![[0.0, 0.0]; size * size];
    engine.trace_level(&mut level);
    finish(engine, level, settings)
}

/// Both terrains take their carving.
fn apply_corrections(level: &mut RiverLevel) {
    let correction = &level.correction;
    level.height.data.par_iter_mut().zip(correction.par_iter()).for_each(|(height, &change)| {
        *height += change;
    });
    let correction = &level.original_correction;
    level.original.data.par_iter_mut().zip(correction.par_iter()).for_each(|(height, &change)| {
        *height += change;
    });
}

/// The end of `rivers`: directions, water surface, depth and the final
/// terrain.
fn finish(mut engine: Rivers, mut level: RiverLevel, settings: &RiverSettings) -> RiverOutput {
    let size = level.size;
    for _ in 0..8 {
        engine.directions = blur_directions(&engine.directions, &level.water.data, size, 0.25, 1.0);
    }

    let mut direction = vec![0.0f32; size * size * 3];
    for index in 0..size * size {
        level.height.data[index] += level.correction[index];
        let [vx, vy] = engine.directions[index];
        let (red, green) = if vx == 0.0 && vy == 0.0 {
            (NEUTRAL, NEUTRAL)
        } else {
            let largest = largest_component(vx, vy);
            ((vx / largest) * 0.5 + 0.5, (vy / largest) * 0.5 + 0.5)
        };
        direction[index * 3] = red;
        direction[index * 3 + 1] = green;
        direction[index * 3 + 2] = NEUTRAL;
        if 0.0 < level.original_correction[index] {
            level.original_correction[index] = 0.0;
        }
        level.original.data[index] += level.original_correction[index];
    }
    // The original terrain (with its carving) becomes the result.
    std::mem::swap(&mut level.height, &mut level.original);
    std::mem::swap(&mut level.correction, &mut level.original_correction);

    // The water surface: the two masked blurs of the bed.
    let mut surface = level.height.data.clone();
    level.correction = level.height.data.clone();
    level.correction = blur_masked(&level.correction, &level.water.data, size, true);
    let self_mask = level.correction.clone();
    level.correction = blur_masked(&level.correction, &self_mask, size, false);
    for (height, &bed) in surface.iter_mut().zip(&level.correction) {
        *height = if bed <= 0.0 { *height - SURFACE_GAP } else { bed };
    }
    for _ in 0..5 {
        surface = smooth_surface(&surface, &level.correction, &level.height.data, size);
    }

    let mut depth = vec![0.0f32; size * size];
    for index in 0..size * size {
        let carved = (expf(level.water.data[index] * -0.005) - 1.0) * ((-settings.depth) * 0.005);
        depth[index] = carved;
        if 0.0 < carved {
            level.height.data[index] = surface[index] - carved;
        }
    }
    for _ in 0..4 {
        level.height.data = relax_banks(&level.height.data, &level.water.data, &surface, size, 1.0);
    }

    let mut heights = vec![0.0f32; size * size];
    for index in 0..size * size {
        let mut value = level.height.data[index];
        if 0.0 < level.water.data[index] && settings.render_surface {
            value = surface[index];
        }
        heights[index] = if value < 0.0 { 0.0 } else { value };
    }
    RiverOutput {
        heights,
        depth,
        surface,
        direction,
    }
}
