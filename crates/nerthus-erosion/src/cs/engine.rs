//! The snow driver: `ErosionWrapperS.ErodeTerrain` (version 1) and
//! `Erosion1`'s virtual methods in the CS engine — `erode`, the per-level
//! loop, the pyramid and the row jobs they run.
//!
//! Snow falls (and melts) only while the coarsest level runs; every level
//! then lets it slip down slopes steeper than the slip-off angle. Rows run
//! in the engine's job order, which matters because slipping snow is
//! scattered into neighbouring cells.

use super::resample::{downsample, upsample_height, upsample_snow};
use crate::ce::kernel::{kernel_sum, FloatMap};

/// The arguments of `ErosionWrapperS.SetVars` that `Erosion1` reads.
#[derive(Clone, Debug)]
pub struct SnowSettings {
    /// Terrain width in world units.
    pub size: f64,
    /// Feature scale in world units (callers pass the model scale times 4).
    pub scale: f64,
    pub snowfall_duration: f64,
    pub snowfall_intensity: f64,
    pub post_snowfall_duration: f64,
    pub melting: f64,
    /// Height in world units below which no snow falls.
    pub snow_line: f64,
    /// Copies the snow into the slip scratch map before each level runs
    /// (`enable_thermal`; every caller sets it).
    pub enable_thermal: bool,
    /// Slip-off angle in degrees (`talus_angle`).
    pub talus_angle: f64,
    /// Snow mass that still clings to steep slopes (`flow_volume`).
    pub flow_volume: f64,
    /// Vertical range the [0, 1] heights are scaled to while simulating.
    pub height_range: f64,
    /// 1: the snow output is the soft mask; 2: binary coverage; 3: depth
    /// plus soft and hard masks.
    pub snow_output: i32,
}

/// The wrapper's outputs.
#[derive(Clone, Debug)]
pub struct SnowOutput {
    /// Terrain plus snow, rescaled to [0, 1] heights.
    pub height: Vec<f32>,
    /// Snow depth (or the mask `snow_output` selects).
    pub snow: Vec<f32>,
    /// Soft coverage mask (`snow_output` 3 only, otherwise zeros).
    pub soft: Vec<f32>,
    /// Snow deeper than one world unit (`snow_output` 3 only).
    pub hard: Vec<f32>,
}

/// SSE `MAXSS a, b`: `a` when it is greater, otherwise `b` (also for NaN).
#[inline]
fn maxss(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

/// SSE `MINSS a, b`: `a` when it is smaller, otherwise `b`.
#[inline]
fn minss(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

/// `CVTTSS2SI`: truncation, with `int.MinValue` for NaN or out of range.
#[inline]
fn truncate(value: f32) -> i32 {
    if value.is_nan() || value >= 2_147_483_648.0 || value < -2_147_483_648.0 {
        return i32::MIN;
    }
    value as i32
}

/// Neighbour offsets and slip weights of the slip pass.
const NEIGHBOUR_X: [i32; 8] = [-1, 0, 1, -1, 1, -1, 0, 1];
const NEIGHBOUR_Y: [i32; 8] = [-1, -1, -1, 0, 0, 1, 1, 1];
/// `0x3f34fdf4`: 0.707, not 1/sqrt(2).
const DIAGONAL: f32 = f32::from_bits(0x3f34_fdf4);
const NEIGHBOUR_WEIGHT: [f32; 8] = [DIAGONAL, 1.0, DIAGONAL, 1.0, 1.0, DIAGONAL, 1.0, DIAGONAL];

/// One pyramid level (`LandscapeLayer`, 152 bytes in the CS engine).
struct Layer {
    /// [0] terrain plus snow, scaled by the height range.
    height: FloatMap,
    /// [1] slip scratch: the snow after this step's slipping.
    work: FloatMap,
    /// [3] soft snow mask.
    soft: FloatMap,
    /// [4] snow depth.
    snow: FloatMap,
    /// [10] detail removed when the next coarser level was built.
    residual: Option<FloatMap>,
    /// [11] snowfall area mask.
    snowfall: Option<FloatMap>,
    /// [12] melt area mask.
    melt: Option<FloatMap>,
    /// [13] hard snow mask.
    hard: Option<FloatMap>,
    resolution: i32,
    /// 0 for the full resolution layer.
    level: i32,
    /// Cell size in world units.
    cell: f32,
}

impl Layer {
    fn new(level: i32, resolution: i32) -> Self {
        Self {
            height: FloatMap::new(resolution, 0.0),
            work: FloatMap::new(resolution, 0.0),
            soft: FloatMap::new(resolution, 0.0),
            snow: FloatMap::new(resolution, 0.0),
            residual: None,
            snowfall: None,
            melt: None,
            hard: None,
            resolution,
            level,
            cell: 0.0,
        }
    }
}

fn downsampled(fine: &FloatMap, resolution: i32, reciprocal: f32) -> FloatMap {
    let mut coarse = FloatMap::new(resolution, 0.0);
    for y in 0..resolution {
        for x in 0..resolution {
            let index = coarse.index(x, y);
            coarse.data[index] = downsample(fine, x, y, reciprocal);
        }
    }
    coarse
}

/// Engine state (`Erosion1`).
struct Engine {
    iteration: i32,
    /// The settling passes after the main loop.
    finishing: bool,
    /// The next level run is the first (coarsest) one.
    first_run: bool,
    /// World units of snow simulated per iteration and cell.
    speed: f32,
    /// Time the finer levels' passes account for.
    time_offset: f32,
    estimated_iterations: i32,
    /// Iterations with snowfall.
    snowfall_iterations: i32,
    snowfall_step: f32,
    melt_step: f32,
    use_snowfall_area: bool,
    use_melt_area: bool,
    size: f64,
    resolution: i32,
    /// Total duration in iterations.
    duration: i32,
    /// Snowfall duration in iterations.
    snowfall_duration: i32,
    snowfall_intensity: f64,
    melting: f64,
    snow_line: f64,
    /// Log2 of the feature scale in cells.
    scale_levels: f64,
    thermal: bool,
    /// Tan(slip-off angle).
    slip_slope: f64,
    scale: f64,
    adhesion: f64,
    height_range: f64,
    snow_output: i32,
    /// `1 / kernel sum`, as the downsamplers use it.
    reciprocal_kernel_sum: f32,
    /// Debugging aid: the slip pass before which `captured` is filled.
    capture_at: Option<usize>,
    slip_passes: usize,
    captured: Option<SnowSnapshot>,
}

/// Layer maps (in world units) captured before a given slip pass, for
/// comparing against the native engine step by step.
#[derive(Clone, Debug)]
pub struct SnowSnapshot {
    pub height: Vec<f32>,
    pub work: Vec<f32>,
    pub snow: Vec<f32>,
}

impl Engine {
    /// Job A: clears the slip scratch map.
    fn clear_work(layer: &mut Layer) {
        layer.work.data.fill(0.0);
    }

    /// Job C: starts the slip scratch from the snow.
    fn copy_snow(layer: &mut Layer) {
        let snow = layer.snow.data.clone();
        layer.work.data.copy_from_slice(&snow);
    }

    /// Job B for one cell: moves snow resting on a slope
    /// steeper than the (adhesion-scaled) slip-off slope to lower
    /// neighbours, accumulating the result in the scratch map.
    fn slip_cell(&self, layer: &mut Layer, x: i32, y: i32) {
        let size = layer.resolution;
        let snow = layer.snow.at(x, y);
        let adhesion = 0.5 / self.adhesion;
        let mut hold = 1.0 - adhesion * snow as f64;
        hold = if hold > 0.0 { hold } else { 0.0 };
        let slip_height = (layer.cell as f64 * self.slip_slope) as f32;
        let mut threshold = (hold * slip_height as f64) as f32;
        if self.iteration == -1 {
            threshold = 0.0;
        }
        if !(snow > 0.0) {
            return;
        }

        let heights = &layer.height;
        let width = heights.size;
        let clamped = |value: i32| -> i32 {
            let low = if value < 0 { 0 } else { value };
            if low < width { low } else { width - 1 }
        };
        let left = clamped(x - 1);
        let right = clamped(x + 1);
        let column = clamped(x);
        let above = clamped(y - 1);
        let below = clamped(y + 1);
        let row = clamped(y);
        let h = |cx: i32, cy: i32| heights.data[(cy * width + cx) as usize];
        let top_left = h(left, above);
        let top = h(column, above);
        let top_right = h(right, above);
        let middle_left = h(left, row);
        let middle_right = h(right, row);
        let bottom_left = h(left, below);
        let bottom = h(column, below);
        let bottom_right = h(right, below);
        let centre = heights.data[(x + width * y) as usize];

        let right_sum = (top_right + middle_right) + (bottom_right + middle_right);
        let left_sum = (top_left + middle_left) + (bottom_left + middle_left);
        let gradient_x = right_sum - left_sum;
        let bottom_sum = (bottom + bottom_right) + (bottom_left + bottom);
        let top_sum = (top_right + top) + (top_left + top);
        let gradient_y = bottom_sum - top_sum;
        let slope = ((gradient_y * gradient_y + gradient_x * gradient_x) * 0.015625).sqrt();

        let mut drops = [0.0f32; 8];
        let mut total = 0.0f32;
        let mut excess = 0.0f32;
        for neighbour in 0..8 {
            let nx = NEIGHBOUR_X[neighbour] + x;
            let ny = NEIGHBOUR_Y[neighbour] + y;
            let inside = nx >= 0 && nx < size && ny >= 0 && ny < size;
            let steep = self.finishing || slope > threshold;
            if !inside || !steep {
                drops[neighbour] = -1.0;
                continue;
            }
            let drop = centre - h(clamped(nx), clamped(ny));
            drops[neighbour] = drop;
            if drop > 0.0 {
                total += drop;
                let over = drop - threshold * NEIGHBOUR_WEIGHT[neighbour];
                excess += maxss(over, 0.0);
            }
        }

        let factor = if self.finishing {
            0.125f32
        } else {
            excess = total;
            0.16f32
        };
        let moved = minss(factor * excess, snow);
        if !(moved > 0.0) {
            return;
        }
        let work = &mut layer.work;
        let work_width = work.size;
        for neighbour in 0..8 {
            let drop = drops[neighbour];
            if drop > 0.0 {
                let index = ((y + NEIGHBOUR_Y[neighbour]) * work_width + NEIGHBOUR_X[neighbour] + x) as usize;
                work.data[index] += (drop * moved) / total;
            }
        }
        let index = (work_width * y + x) as usize;
        work.data[index] -= moved;
    }

    /// Job B over every row, in the thread pool's job order.
    fn slip(&mut self, layer: &mut Layer) {
        if self.capture_at == Some(self.slip_passes) && self.captured.is_none() {
            self.captured = Some(SnowSnapshot {
                height: layer.height.data.clone(),
                work: layer.work.data.clone(),
                snow: layer.snow.data.clone(),
            });
        }
        self.slip_passes += 1;
        let size = layer.resolution;
        // Below 4 pixels the native engine divides by zero (and crashes).
        let quarter = (size / 4).max(1);
        for job in 0..size {
            let row = job / quarter + (job * 4) % size;
            if row >= size {
                continue;
            }
            for x in 0..size {
                self.slip_cell(layer, x, row);
            }
        }
    }

    /// Job D (`Erosion1` vtable slot 1): adds snowfall and
    /// melts (first level only), then commits the slipped snow.
    fn accumulate(&self, layer: &mut Layer) {
        let size = layer.resolution;
        for y in 0..size {
            for x in 0..size {
                let index = layer.height.index(x, y);
                let mut snowfall = self.snowfall_step;
                let mut melt = self.melt_step;
                let mut snow = layer.work.data[index];
                let height = layer.height.data[index];
                if height as f64 >= self.snow_line {
                    if self.use_snowfall_area {
                        let area = layer.snowfall.as_ref().expect("snowfall area present");
                        snowfall *= area.data[index];
                    }
                } else {
                    snowfall = 0.0;
                }
                if self.use_melt_area {
                    let area = layer.melt.as_ref().expect("melt area present");
                    melt *= area.data[index];
                }
                if self.first_run {
                    if self.iteration < self.snowfall_iterations && self.iteration != -1 {
                        snow += snowfall;
                    }
                    snow -= melt;
                    snow = maxss(snow, 0.0);
                }
                layer.height.data[index] = (height - layer.snow.data[index]) + snow;
                layer.snow.data[index] = snow;
                layer.work.data[index] = snow;
            }
        }
    }

    fn step(&mut self, layer: &mut Layer) {
        self.slip(layer);
        self.accumulate(layer);
    }

    /// The iterations run at one pyramid level.
    fn run_level(&mut self, layer: &mut Layer) {
        Self::clear_work(layer);
        let melt_base = (self.melting * 0.25) as f32;
        let snowfall_duration = self.snowfall_duration;
        let iterations;
        let mut melt_numerator = melt_base;
        if self.first_run {
            let scale = self.scale * 0.25;
            let duration = self.duration;
            let duration_f32 = duration as f32;
            let time = duration as f64 * scale;
            let span = (duration_f32 * layer.cell) as f64;
            let speed = (time / span) as f32;
            self.speed = if speed >= 0.0 { speed } else { 0.0 };
            let remaining_time = (duration - snowfall_duration) as f64 * scale - self.time_offset as f64;
            let remaining = (remaining_time / layer.cell as f64) as f32;
            let remaining = if remaining >= 0.0 { remaining } else { 0.0 };
            self.snowfall_iterations = truncate(snowfall_duration as f32 * self.speed);
            iterations = truncate(self.snowfall_iterations as f32 + remaining);
            let simulated = truncate(duration_f32 * self.speed);
            melt_numerator = (simulated as f32 / iterations as f32) * melt_base;
        } else {
            iterations = self.estimated_iterations;
        }
        let snowfall_share = (self.snowfall_iterations as f32 * 0.5) / (snowfall_duration as f32 * self.speed);
        self.snowfall_step = ((snowfall_share as f64 * self.snowfall_intensity) / self.speed as f64) as f32;
        self.melt_step = melt_numerator / self.speed;
        if self.thermal {
            Self::copy_snow(layer);
        }

        self.finishing = false;
        self.iteration = 0;
        while self.iteration < iterations {
            self.step(layer);
            self.iteration += 1;
        }
        self.finishing = true;
        for _ in 0..6 {
            self.step(layer);
        }
        if layer.level == 0 {
            self.iteration = -1;
            self.step(layer);
        }
        self.first_run = false;
    }

    /// Simulates a half-resolution copy of `fine`
    /// (recursively coarser while the feature scale allows), then rebuilds
    /// `fine` from it.
    fn run_coarse_levels(&mut self, fine: &mut Layer) {
        self.time_offset += ((self.estimated_iterations as f32 + 0.5) * fine.cell) * 0.25;
        let resolution = fine.resolution >> 1;
        let mut coarse = Layer::new(fine.level + 1, resolution);
        coarse.cell = fine.cell + fine.cell;
        let reciprocal = self.reciprocal_kernel_sum;
        if let Some(area) = &fine.snowfall {
            coarse.snowfall = Some(downsampled(area, resolution, reciprocal));
        }
        if let Some(area) = &fine.melt {
            coarse.melt = Some(downsampled(area, resolution, reciprocal));
        }
        coarse.height = downsampled(&fine.height, resolution, reciprocal);
        Self::store_residual(fine, &coarse);
        if (coarse.level as f64) < self.scale_levels - 1.0 && 0xb < coarse.resolution {
            self.run_coarse_levels(&mut coarse);
        }
        self.run_level(&mut coarse);
        Self::restore(fine, &coarse);
    }

    /// The detail the coarse level cannot represent.
    fn store_residual(fine: &mut Layer, coarse: &Layer) {
        let size = fine.resolution;
        let mut residual = FloatMap::new(size, 0.0);
        for y in 0..size {
            for x in 0..size {
                let smooth = upsample_height(&coarse.height, x as u32, y as u32);
                let index = residual.index(x, y);
                residual.data[index] = fine.height.data[index] - smooth;
            }
        }
        fine.residual = Some(residual);
    }

    /// Rebuilds `fine` from the simulated coarse level,
    /// re-applying the stored detail under the snow. (The soft mask and
    /// map [6] are still zero on coarse levels, so their upsampled values
    /// are too.)
    fn restore(fine: &mut Layer, coarse: &Layer) {
        let size = fine.resolution;
        let residual = fine.residual.take().expect("residual stored before the coarse level ran");
        for y in 0..size as u32 {
            for x in 0..size as u32 {
                let index = fine.height.index(x as i32, y as i32);
                let height = upsample_height(&coarse.height, x, y);
                let mut soft = 0.0f32;
                let mut snow = upsample_snow(&coarse.snow, x, y);
                let loose = snow + soft;
                let mut detail = residual.data[index];
                if loose > 0.0 {
                    if loose - detail >= 0.0 {
                        let mut keep = 1.0 - loose / coarse.cell;
                        if !(keep > 0.0) {
                            keep = 0.0;
                        }
                        let moved = (keep - 1.0) * detail;
                        detail *= keep;
                        snow += moved;
                    } else {
                        soft = 0.0;
                        detail -= loose;
                    }
                }
                fine.height.data[index] = height + detail;
                fine.soft.data[index] = maxss(soft, 0.0);
                fine.snow.data[index] = maxss(snow, 0.0);
            }
        }
    }

    /// `Erosion1` vtable slot 0 for one cell: soft snow
    /// coverage from the snow deeper than one unit around it (weights
    /// 1-2-1 / 2-4-2 / 1-2-1), and the hard mask.
    fn snow_masks_cell(&self, layer: &mut Layer, x: i32, y: i32) {
        let snow = &layer.snow;
        let width = snow.size;
        let clamped = |value: i32| -> i32 {
            let low = if value < 0 { 0 } else { value };
            if low < width { low } else { width - 1 }
        };
        let s = |cx: i32, cy: i32| snow.data[(cy * width + cx) as usize];
        let left = clamped(x - 1);
        let column = clamped(x);
        let right = clamped(x + 1);
        let above = clamped(y - 1);
        let row = clamped(y);
        let below = clamped(y + 1);

        let mut sum = 0.0f32;
        let mut count = 0.0f32;
        let corner = s(left, above);
        if corner > 1.0 {
            sum = corner;
            count = 1.0;
            sum -= 1.0;
        }
        let edge = s(column, above);
        if edge > 1.0 {
            sum += edge;
            count += 2.0;
            sum -= 1.0;
            sum += edge;
            sum -= 1.0;
        }
        let corner = s(right, above);
        if corner > 1.0 {
            sum = (corner + sum) - 1.0;
            count += 1.0;
        }
        let edge = s(left, row);
        if edge > 1.0 {
            count += 2.0;
            sum = (((edge + sum) - 1.0) + edge) - 1.0;
        }
        let edge = s(right, row);
        if edge > 1.0 {
            sum += edge;
            count += 2.0;
            sum -= 1.0;
            sum += edge;
            sum -= 1.0;
        }
        let corner = s(left, below);
        if corner > 1.0 {
            sum = (corner + sum) - 1.0;
            count += 1.0;
        }
        let edge = s(column, below);
        if edge > 1.0 {
            sum += edge;
            count += 2.0;
            sum -= 1.0;
            sum += edge;
            sum -= 1.0;
        }
        let corner = s(right, below);
        if corner > 1.0 {
            sum = (corner + sum) - 1.0;
            count += 1.0;
        }
        let centre = snow.data[(x + width * y) as usize];
        let mut depth = centre - 1.0;
        if centre > 1.0 {
            count += 4.0;
            sum += depth * 4.0;
        }
        depth = maxss(depth, 0.0);
        let coverage = minss(maxss(count - 4.0, 0.0) * 0.108_333_334, 1.0);
        let mut mask = if depth > 0.0 { minss(((depth * 1.3) / sum) * count, 1.0) } else { 0.0 };
        mask *= coverage;
        if mask > 0.0 {
            mask = mask * 0.8 + 0.2;
        }
        let soft_index = layer.soft.index(x, y);
        layer.soft.data[soft_index] = mask;
        if self.snow_output == 3 {
            let hard = layer.hard.as_mut().expect("hard mask allocated for snow output 3");
            let hard_index = hard.index(x, y);
            hard.data[hard_index] = if centre > 1.0 { 1.0 } else { 0.0 };
        }
    }

    /// The last row job of `erode`: drops the first unit
    /// of snow and rescales the outputs.
    fn finish(&self, layer: &mut Layer) {
        let size = layer.resolution;
        for y in 0..size {
            for x in 0..size {
                let index = layer.height.index(x, y);
                let snow = layer.snow.data[index];
                let kept = if snow >= 1.0 { snow + -1.0 } else { 0.0 };
                layer.height.data[index] += kept - snow;
                let value = match self.snow_output {
                    1 => layer.soft.data[index],
                    2 => {
                        if kept > 0.0 {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    _ => (kept as f64 / self.height_range) as f32,
                };
                layer.snow.data[index] = value;
                layer.height.data[index] = (layer.height.data[index] as f64 / self.height_range) as f32;
            }
        }
    }

    /// `Erosion1::erode`, after the wrapper copied the
    /// bedrock into `bedrock`.
    fn erode(&mut self, layer: &mut Layer, bedrock: &[f32]) {
        self.finishing = false;
        self.speed = 1.0;
        self.first_run = true;
        self.time_offset = 0.0;
        let after_snowfall = (self.duration - self.snowfall_duration) as f32 * 0.25;
        let share = self.duration as f32 * 0.125;
        let estimate = if after_snowfall <= share { share } else { after_snowfall };
        let estimate = truncate(estimate);
        self.estimated_iterations = if 1 < estimate { estimate } else { 1 };
        layer.cell = (self.size / self.resolution as f64) as f32;
        for (height, value) in layer.height.data.iter_mut().zip(bedrock) {
            *height = (*value as f64 * self.height_range) as f32;
        }
        if 1.0 < self.scale_levels {
            self.run_coarse_levels(layer);
        }
        self.run_level(layer);
        if self.snow_output == 1 || self.snow_output == 3 {
            for y in 0..layer.resolution {
                for x in 0..layer.resolution {
                    self.snow_masks_cell(layer, x, y);
                }
            }
        }
        self.finish(layer);
    }
}

/// `ErosionWrapperS.ErodeTerrain(heightmap, snowfall_area, melt_area,
/// fixed: true)`. `heightmap` holds [0, 1] heights; the areas optionally
/// scale snowfall and melting per cell.
pub fn simulate(settings: &SnowSettings, heightmap: &[f32], snowfall_area: Option<&[f32]>, melt_area: Option<&[f32]>) -> SnowOutput {
    run(settings, heightmap, snowfall_area, melt_area, None).0
}

/// Runs the simulation and also returns the layer state right before slip
/// pass `pass` (counting from 0, across all pyramid levels).
pub fn simulate_with_snapshot(settings: &SnowSettings, heightmap: &[f32], pass: usize) -> (SnowOutput, Option<SnowSnapshot>) {
    run(settings, heightmap, None, None, Some(pass))
}

fn run(settings: &SnowSettings, heightmap: &[f32], snowfall_area: Option<&[f32]>, melt_area: Option<&[f32]>, capture_at: Option<usize>) -> (SnowOutput, Option<SnowSnapshot>) {
    let resolution = (heightmap.len() as f64).sqrt() as i32;
    let snowfall_duration = (settings.snowfall_duration * 1.1 * 256.0) as i32;
    let post_snowfall_duration = (settings.post_snowfall_duration * 1.1 * 256.0) as i32;
    let scale_ratio = settings.scale * 0.5 / (settings.size / resolution as f64);
    let scale_levels = scale_ratio.ln() * std::f64::consts::LOG2_E;
    let talus_angle = if settings.talus_angle < 89.999 { settings.talus_angle } else { 89.999 };
    let mut engine = Engine {
        iteration: 0,
        finishing: false,
        first_run: true,
        speed: 1.0,
        time_offset: 0.0,
        estimated_iterations: 1,
        snowfall_iterations: 0,
        snowfall_step: 0.0,
        melt_step: 0.0,
        use_snowfall_area: snowfall_area.is_some(),
        use_melt_area: melt_area.is_some(),
        size: settings.size,
        resolution,
        duration: post_snowfall_duration + snowfall_duration,
        snowfall_duration,
        snowfall_intensity: settings.snowfall_intensity,
        melting: settings.melting,
        snow_line: settings.snow_line,
        scale_levels,
        thermal: settings.enable_thermal,
        slip_slope: (talus_angle * 0.0174532925199433).tan(),
        scale: settings.scale,
        adhesion: settings.flow_volume,
        height_range: settings.height_range,
        snow_output: settings.snow_output,
        reciprocal_kernel_sum: 1.0 / kernel_sum(),
        capture_at,
        slip_passes: 0,
        captured: None,
    };

    let mut layer = Layer::new(0, resolution);
    layer.snowfall = snowfall_area.map(|values| FloatMap { size: resolution, data: values.to_vec() });
    layer.melt = melt_area.map(|values| FloatMap { size: resolution, data: values.to_vec() });
    if settings.snow_output == 3 {
        layer.hard = Some(FloatMap::new(resolution, 0.0));
    }
    engine.erode(&mut layer, heightmap);

    let count = (resolution * resolution) as usize;
    let (soft, hard) = match layer.hard {
        Some(hard) => (layer.soft.data, hard.data),
        None => (vec![0.0; count], vec![0.0; count]),
    };
    let output = SnowOutput {
        height: layer.height.data,
        snow: layer.snow.data,
        soft,
        hard,
    };
    (output, engine.captured)
}
