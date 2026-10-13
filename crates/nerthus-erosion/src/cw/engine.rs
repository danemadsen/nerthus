//! The CW water engine: `Erosion::erode_terrain` (Lake) and
//! `Erosion::edge_fill` (Sea), transcribed from the disassembly.

use rayon::prelude::*;

use super::blur::box_blur;
use super::fill::SpanFill;

/// The wrapper's model size and feature scale (`SetVars`).
const MODEL_SIZE: f64 = 1024.0;
const MODEL_SCALE: f64 = 32.0;
/// The share of the height drop that flows per step.
const FLOW_RATE: f32 = 0.16;
/// The flow stops once no row moves more than this.
const CONVERGED: f64 = 0.01;
/// Float constants the engine widens to double.
const ONE_TWENTIETH: f64 = 0.050_000_000_745_058_06;
const ONE_TENTH: f64 = 0.100_000_001_490_116_12;
const SIZE_BIAS_RADIUS: f64 = 0.020_999_999_716_877_937;
const ALTITUDE_SCALE: f64 = 24.0;
/// The CW engine's literal 3.14 (not π).
#[allow(clippy::approx_constant)]
const SIZE_BIAS_GAIN: f32 = 3.14;

/// The 8 neighbours in the engine's table order: row offsets, then column
/// offsets.
const NEIGHBOUR_ROWS: [i32; 8] = [-1, -1, -1, 0, 0, 1, 1, 1];
const NEIGHBOUR_COLUMNS: [i32; 8] = [-1, 0, 1, -1, 1, -1, 0, 1];

/// `ErosionWrapperW.SetVars` arguments.
#[derive(Clone, Copy, Debug)]
pub struct LakeSettings {
    pub resolution: usize,
    pub precipitation: f64,
    pub water_floor: f64,
    pub lake_size_bias: f64,
    pub shore_size: f64,
    pub altitude_bias: f64,
    pub size_bias: f64,
    /// Terrain width in metres.
    pub size: f64,
    /// Terrain height in metres.
    pub height_range: f64,
}

/// The wrapper's outputs.
#[derive(Clone, Debug)]
pub struct WaterOutput {
    /// Terrain with the water filled in.
    pub results: Vec<f32>,
    pub depth: Vec<f32>,
    pub shore: Vec<f32>,
    pub surface: Vec<f32>,
}

/// One level of the resolution pyramid (`LandscapeLayer`).
struct Layer {
    size: usize,
    level: i32,
    /// Cell size.
    cell: f32,
    /// [0] terrain plus water.
    height: Vec<f32>,
    /// [1] water being moved / the surface output.
    water: Vec<f32>,
    /// [3] shore.
    shore: Vec<f32>,
    /// [4] water depth.
    depth: Vec<f32>,
    /// [9] detail lost by downsampling (fine levels of the pyramid).
    residual: Vec<f32>,
    /// [10] precipitation area.
    precipitation: Option<Vec<f32>>,
    /// [15] largest drop that moved water in each row.
    row_drop: Vec<f32>,
}

impl Layer {
    fn new(level: i32, size: usize) -> Layer {
        let count = size * size;
        Layer {
            size,
            level,
            cell: 1.0,
            height: vec![0.0; count],
            water: vec![0.0; count],
            shore: vec![0.0; count],
            depth: vec![0.0; count],
            residual: Vec::new(),
            precipitation: None,
            row_drop: vec![0.0; size],
        }
    }
}

/// The scalars of `Erosion` the water passes read.
struct Water {
    use_precipitation: bool,
    /// Precipitation after the small-lake adjustment.
    precipitation: f64,
    /// Precipitation per cell of the coarsest level.
    precipitation_per_cell: f64,
    shore_size: f64,
    size_bias: f64,
    altitude_bias: f64,
    lake_size_bias: f64,
    levels: f64,
    levels_adjusted: f64,
    water_floor: f64,
    /// Depth above which neighbours count towards the flow rate.
    threshold: f32,
    /// The next simulated level is the first (it gets the rain).
    first: bool,
}

fn clamp_index(value: i32, size: i32) -> i32 {
    let low = if value < 0 { 0 } else { value };
    if low > size - 1 { size - 1 } else { low }
}

/// Rows in the interleaved order the flow jobs use
/// (`job / (n / 4) + (4 * job) % n`).
fn job_row(job: usize, size: usize) -> usize {
    let quarter = size / 4;
    job / quarter + (4 * job) % size
}

impl Water {
    /// Rain on every cell, topped up to the water floor.
    fn rain(&self, layer: &mut Layer) {
        let amount = self.precipitation as f32;
        for index in 0..layer.height.len() {
            let height = layer.height[index];
            let mut floor = self.water_floor - height as f64;
            if 0.0 > floor {
                floor = 0.0;
            }
            let mut fall = (amount as f64 + floor) as f32;
            if self.use_precipitation
                && let Some(area) = &layer.precipitation {
                    fall *= area[index];
                }
            layer.height[index] = height + fall;
            layer.depth[index] += fall;
        }
    }

    /// Each wet cell passes `FLOW_RATE` of its height drop
    /// to lower neighbours, split by drop. Jobs run rows in the order
    /// `job / (n / 4) + (4 * job) % n`, so each quarter of the jobs covers
    /// rows four apart; their writes (the row and its two neighbours) never
    /// overlap, so a quarter runs in parallel and every sum accumulates in
    /// the engine's order.
    fn flow(&self, layer: &mut Layer) {
        let size = layer.size;
        let phase_length = if size.is_multiple_of(4) { size / 4 } else { 1 };
        let jobs: Vec<usize> = (0..size).collect();
        let mut water = std::mem::take(&mut layer.water);
        let shared = SharedWater {
            pointer: water.as_mut_ptr(),
            length: water.len(),
        };
        let layer_view = &*layer;
        let mut row_drop = vec![0.0f32; size];
        for phase in jobs.chunks(phase_length.max(1)) {
            let drops: Vec<(usize, f32)> = phase
                .par_iter()
                .filter_map(|&job| {
                    let row = job_row(job, size);
                    if row >= size {
                        return None;
                    }
                    let mut apply = |transfer: Transfer| match transfer {
                        // SAFETY: a job only writes its own row and the rows
                        // beside it, and jobs in one phase are four rows apart.
                        Transfer::Gain(index, amount) => unsafe { shared.gain(index, amount) },
                        Transfer::Loss(index, amount) => unsafe { shared.loss(index, amount) },
                    };
                    let drop = self.flow_row(layer_view, row, &mut apply);
                    Some((row, drop))
                })
                .collect();
            for (row, drop) in drops {
                row_drop[row] = drop;
            }
        }
        layer.water = water;
        layer.row_drop = row_drop;
    }

    /// One flow job, reporting each write in order.
    fn flow_row(&self, layer: &Layer, row: usize, apply: &mut impl FnMut(Transfer)) -> f32 {
        let size = layer.size as i32;
        let y = row as i32;
        let mut largest = 0.0f32;
        for x in 0..size {
            let index = (y * size + x) as usize;
            let depth = layer.depth[index];
            if !(depth > 0.0) {
                continue;
            }
            let height = layer.height[index];
            let mut total = 0.0f32;
            let mut drops = [0.0f32; 8];
            for neighbour in 0..8 {
                let nx = x + NEIGHBOUR_COLUMNS[neighbour];
                let ny = y + NEIGHBOUR_ROWS[neighbour];
                if nx < 0 || nx >= size || ny < 0 || ny >= size {
                    drops[neighbour] = -1.0;
                    continue;
                }
                let neighbour_index = (ny * size + nx) as usize;
                let drop = height - layer.height[neighbour_index];
                drops[neighbour] = drop;
                if drop > 0.0 {
                    total += drop;
                    let both_deep = depth > self.threshold && layer.depth[neighbour_index] > self.threshold;
                    if both_deep {
                        largest = if drop > largest { drop } else { largest };
                    }
                }
            }
            let scaled = total * FLOW_RATE;
            let outflow = if scaled < depth { scaled } else { depth };
            if !(outflow > 0.0) {
                continue;
            }
            for neighbour in 0..8 {
                let drop = drops[neighbour];
                if drop > 0.0 {
                    let nx = x + NEIGHBOUR_COLUMNS[neighbour];
                    let ny = y + NEIGHBOUR_ROWS[neighbour];
                    let share = (drop * outflow) / total;
                    apply(Transfer::Gain((ny * size + nx) as usize, share));
                }
            }
            apply(Transfer::Loss(index, outflow));
        }
        largest
    }

    /// The moved water replaces the old depth.
    fn settle(layer: &mut Layer) {
        let water = &layer.water;
        layer.height.par_iter_mut().zip(layer.depth.par_iter_mut()).zip(water.par_iter()).for_each(|((height, depth), &moved)| {
            *height = (*height - *depth) + moved;
            *depth = moved;
        });
    }

    /// Drops puddles of one or two cells (unless small
    /// lakes are kept or the rain area says otherwise).
    fn remove_puddles(&self, layer: &mut Layer) {
        let size = layer.size as i32;
        for y in 0..size {
            for x in 0..size {
                let index = (y * size + x) as usize;
                let depth = layer.depth[index];
                if depth <= 0.0 {
                    layer.water[index] = depth;
                    continue;
                }
                let mut wet = 0;
                for column in (x - 1).max(0)..=(x + 1).min(size - 1) {
                    for neighbour_row in (y - 1).max(0)..=(y + 1).min(size - 1) {
                        if layer.depth[(neighbour_row * size + column) as usize] > 0.0 {
                            wet += 1;
                        }
                    }
                }
                let remove = if self.use_precipitation {
                    let area = layer.precipitation.as_ref().map_or(0.0, |area| area[index]);
                    wet < 3 && (self.lake_size_bias < 1.0 || (area as f64) < self.lake_size_bias)
                } else {
                    !(wet > 2 || 1.0 <= self.lake_size_bias)
                };
                layer.water[index] = if remove { 0.0 } else { depth };
            }
        }
    }

    /// Levels every lake at its lowest water surface.
    fn flatten_lakes(layer: &mut Layer) {
        let size = layer.size;
        let mut fill = SpanFill::new(size);
        let depth = layer.depth.clone();
        let height = layer.height.clone();
        let wet = |x: i32, y: i32| depth[y as usize * size + x as usize] > 0.0;
        for y in 0..size as i32 {
            for x in 0..size as i32 {
                let index = y as usize * size + x as usize;
                if !(depth[index] > 0.0) || fill.is_visited(x, y) {
                    continue;
                }
                let mut lowest = f32::MAX;
                let mut cells: Vec<usize> = Vec::new();
                let mut collect = |cx: i32, cy: i32| {
                    let cell = cy as usize * size + cx as usize;
                    let surface = height[cell];
                    lowest = if surface <= lowest { surface } else { lowest };
                    cells.push(cell);
                };
                fill.fill(x, y, &wet, &mut collect);
                for cell in cells {
                    let mut level = (lowest - height[cell]) + depth[cell];
                    if 0.0 > level {
                        level = 0.0;
                    }
                    layer.water[cell] = level;
                }
            }
        }
        Self::settle(layer);
    }

    /// The water simulation of one level.
    fn simulate(&mut self, layer: &mut Layer) {
        layer.water.fill(0.0);
        let iterations = if self.first {
            self.precipitation_per_cell = self.precipitation / layer.cell as f64;
            self.rain(layer);
            2000
        } else {
            100
        };
        layer.water.copy_from_slice(&layer.depth);
        let mut iteration = 0;
        loop {
            self.flow(layer);
            Self::settle(layer);
            if iteration % 4 == 0 {
                let mut largest = 0.0f32;
                for &drop in &layer.row_drop {
                    largest = if drop > largest { drop } else { largest };
                }
                let moved = (largest / layer.cell) as f64;
                if CONVERGED > moved {
                    break;
                }
            }
            iteration += 1;
            if iteration >= iterations {
                break;
            }
        }
        if layer.level == 0 {
            self.remove_puddles(layer);
            Self::settle(layer);
            Self::flatten_lakes(layer);
        }
        self.first = false;
    }

    /// The coarse value at (`x`, `y`),
    /// the 3x3 kernel around fine (2x, 2y) divided by its sum.
    fn downsample(fine: &[f32], fine_size: usize, x: i32, y: i32) -> f32 {
        let size = fine_size as i32;
        let column = |offset: i32| clamp_index(2 * x + offset, size);
        let row = |offset: i32| clamp_index(2 * y + offset, size) * size;
        let at = |dx: i32, dy: i32| fine[(row(dy) + column(dx)) as usize];
        let total = 0.25 * at(-1, -1) + 0.5 * at(0, -1) + 0.25 * at(1, -1) + 0.5 * at(-1, 0) + 1.0 * at(0, 0) + 0.5 * at(1, 0) + 0.25 * at(-1, 1) + 0.5 * at(0, 1) + 0.25 * at(1, 1);
        total / 4.0
    }

    /// The coarse heights at fine (`x`, `y`).
    fn upsample_height(coarse: &[f32], coarse_size: usize, x: usize, y: usize) -> f32 {
        let size = coarse_size as i32;
        let half_x = (x >> 1) as i32;
        let half_y = (y >> 1) as i32;
        let at = |cx: i32, cy: i32| coarse[(clamp_index(cy, size) * size + clamp_index(cx, size)) as usize];
        match (x & 1, y & 1) {
            (0, 0) => at(half_x, half_y) + 0.0,
            (0, _) => ((at(half_x, half_y) + 0.0) + at(half_x, half_y + 1)) * 0.5,
            (_, 0) => ((at(half_x, half_y) + 0.0) + at(half_x + 1, half_y)) * 0.5,
            _ => ((((at(half_x, half_y) + 0.0) + at(half_x + 1, half_y)) + at(half_x, half_y + 1)) + at(half_x + 1, half_y + 1)) * 0.25,
        }
    }

    /// The coarse depth at fine (`x`, `y`), kernel weighted.
    fn upsample_depth(coarse: &[f32], coarse_size: usize, x: usize, y: usize) -> f32 {
        let size = coarse_size as i32;
        let half_x = (x >> 1) as i32;
        let half_y = (y >> 1) as i32;
        let at = |cx: i32, cy: i32| coarse[(clamp_index(cy, size) * size + clamp_index(cx, size)) as usize];
        match (x & 1, y & 1) {
            (0, 0) => 1.0 * at(half_x, half_y) + 0.0,
            (0, _) => ((0.5 * at(half_x, half_y) + 0.0) + 0.5 * at(half_x, half_y + 1)) / (0.5 * 2.0),
            (_, 0) => ((0.5 * at(half_x, half_y) + 0.0) + 0.5 * at(half_x + 1, half_y)) / (0.5 * 2.0),
            _ => ((((0.25 * at(half_x, half_y) + 0.0) + 0.25 * at(half_x + 1, half_y)) + 0.25 * at(half_x, half_y + 1)) + 0.25 * at(half_x + 1, half_y + 1)) / (0.25 * 4.0),
        }
    }

    /// Simulates a half-resolution copy first, then brings
    /// its water back to `fine` with the fine detail restored.
    fn pyramid(&mut self, fine: &mut Layer) {
        let coarse_size = fine.size >> 1;
        let mut coarse = Layer::new(fine.level + 1, coarse_size);
        coarse.cell = fine.cell + fine.cell;
        if let Some(area) = &fine.precipitation {
            let downsampled = downsample_map(area, fine.size, coarse_size);
            coarse.precipitation = Some(downsampled);
        }
        coarse.height = downsample_map(&fine.height, fine.size, coarse_size);
        let fine_size = fine.size;
        let mut residual = vec![0.0; fine_size * fine_size];
        residual.par_chunks_mut(fine_size).enumerate().for_each(|(y, row)| {
            for (x, value) in row.iter_mut().enumerate() {
                let upsampled = Self::upsample_height(&coarse.height, coarse_size, x, y);
                *value = fine.height[y * fine_size + x] - upsampled;
            }
        });
        fine.residual = residual;
        let deeper = (coarse.level as f64) < self.levels - 1.0 && coarse.size > 11;
        if deeper {
            self.pyramid(&mut coarse);
        }
        self.simulate(&mut coarse);
        self.restore(fine, &coarse);
        fine.residual = Vec::new();
    }

    /// Upsampled water and heights plus the rain the coarse
    /// level stood for, faded in by level.
    fn restore(&self, fine: &mut Layer, coarse: &Layer) {
        let fade = ((coarse.level as f64 - self.levels_adjusted) + 1.0) as f32;
        let weight = if 1.0 > fade {
            if fade > 0.0 { fade } else { 0.0 }
        } else {
            1.0
        };
        let rain = (self.precipitation_per_cell * ONE_TENTH * coarse.cell as f64 * weight as f64) as f32;
        let size = fine.size;
        for y in 0..size {
            for x in 0..size {
                let index = y * size + x;
                let mut height = Self::upsample_height(&coarse.height, coarse.size, x, y);
                let mut depth = Self::upsample_depth(&coarse.depth, coarse.size, x, y);
                let mut residual = fine.residual[index];
                if depth > 0.0 {
                    if residual > depth {
                        residual -= depth;
                        depth = 0.0;
                        height += residual;
                    } else if 0.0 > residual {
                        height += residual;
                    } else {
                        depth -= residual;
                    }
                } else {
                    height += residual;
                }
                let mut fall = rain;
                if self.use_precipitation
                    && let Some(area) = &fine.precipitation {
                        fall *= area[index];
                    }
                let wet = fall + depth;
                fine.height[index] = fall + height;
                fine.shore[index] = 0.0;
                fine.depth[index] = if wet > 0.0 { wet } else { 0.0 };
            }
        }
    }

    /// How far each cell sits inside a shore band.
    fn shore(&self, layer: &mut Layer) {
        let size = layer.size;
        let altitude = ((self.altitude_bias * ALTITUDE_SCALE) / layer.cell as f64) as f32;
        let mut strength: Vec<f32> = layer.depth.iter().map(|&depth| if depth <= 0.0 { 0.0 } else { 1.0 }).collect();
        if self.size_bias > 0.0 {
            let radius = ((size as f64) * self.size_bias * SIZE_BIAS_RADIUS) as f32;
            let blurred = box_blur(&strength, size, radius);
            for (value, &spread) in strength.iter_mut().zip(blurred.iter()) {
                let scaled = spread * SIZE_BIAS_GAIN;
                let capped = if scaled < 1.0 { scaled } else { 1.0 };
                *value *= capped;
            }
        }
        let shore_cells = self.shore_size as i32;
        let band = shore_cells as f32;
        let height = &layer.height;

        // Per row, the signed column offset to the shore
        // source each cell keeps.
        let rows: Vec<Vec<i32>> = (0..size)
            .into_par_iter()
            .map(|y| shore_sources(&height[y * size..(y + 1) * size], &strength[y * size..(y + 1) * size], band, altitude))
            .collect();

        // The best source within the band for each cell.
        let radius = (shore_cells as f64).ceil() as i32;
        let radius_squared = radius * radius;
        let n = size as i32;
        let columns: Vec<Vec<f32>> = (0..size)
            .into_par_iter()
            .map(|x| {
                let x = x as i32;
                let mut column = Vec::with_capacity(size);
                for y in 0..n {
                    let base = height[(y * n + x) as usize];
                    let start = if y - radius >= 0 { y - radius } else { 0 };
                    let end = (y + radius).min(n - 1);
                    let mut best = 0.0f32;
                    for k in start..end {
                        let dx = rows[k as usize][x as usize];
                        let dy = k - y;
                        let distance_squared = dx * dx + dy * dy;
                        if distance_squared > radius_squared {
                            continue;
                        }
                        let source = (k * n + x + dx) as usize;
                        let distance = (distance_squared as f64).sqrt() as f32;
                        let climb = ((base - height[source]) * altitude).abs();
                        let value = (band * strength[source] - distance) - climb;
                        best = if value > best { value } else { best };
                    }
                    column.push(best / band);
                }
                column
            })
            .collect();
        for (x, column) in columns.iter().enumerate() {
            for (y, value) in column.iter().enumerate() {
                layer.shore[y * size + x] = *value;
            }
        }
    }
}

/// One row of the shore offsets: a forward and a backward sweep score the
/// distance (and climb) back to the last water cell; each cell keeps the
/// column offset of the better one.
fn shore_sources(heights: &[f32], strength: &[f32], band: f32, altitude: f32) -> Vec<i32> {
    let size = heights.len();
    let mut forward_value = vec![0.0f32; size];
    let mut forward_offset = vec![0i32; size];
    let mut last_height = heights[0];
    let mut peak = 0.0f32;
    let mut offset = 0i32;
    for x in 0..size {
        let height = heights[x];
        let mut rise = height - last_height;
        if 0.0 > rise {
            rise = 0.0;
        }
        let mut value = (offset as f32 + peak) - rise * altitude;
        let source = band * strength[x];
        if !(source < value) && source > 0.0 {
            offset = 0;
            peak = source;
            value = source;
            last_height = height;
        }
        forward_value[x] = value;
        forward_offset[x] = offset;
        offset -= 1;
    }
    let mut selected = vec![0i32; size];
    let mut last_height = heights[size - 1];
    let mut peak = 0.0f32;
    let mut offset = 0i32;
    for x in (0..size).rev() {
        let height = heights[x];
        let source = band * strength[x];
        let climb = (height - last_height).abs() * altitude;
        let mut value = (peak - offset as f32) - climb;
        if !(source < value) && source > 0.0 {
            last_height = height;
            offset = 0;
            peak = source;
            value = source;
        }
        selected[x] = if value > forward_value[x] { offset } else { forward_offset[x] };
        offset += 1;
    }
    selected
}

/// One write of a flow job.
enum Transfer {
    /// A neighbour's share: `share + water`.
    Gain(usize, f32),
    /// The cell's own outflow: `water - outflow`.
    Loss(usize, f32),
}

/// The water map shared by the flow jobs of one phase.
#[derive(Clone, Copy)]
struct SharedWater {
    pointer: *mut f32,
    length: usize,
}

// SAFETY: jobs sharing it write disjoint rows (see `Water::flow`).
unsafe impl Send for SharedWater {}
unsafe impl Sync for SharedWater {}

impl SharedWater {
    unsafe fn gain(&self, index: usize, amount: f32) {
        assert!(index < self.length);
        let cell = unsafe { &mut *self.pointer.add(index) };
        *cell += amount;
    }

    unsafe fn loss(&self, index: usize, amount: f32) {
        assert!(index < self.length);
        let cell = unsafe { &mut *self.pointer.add(index) };
        *cell -= amount;
    }
}

fn downsample_map(fine: &[f32], fine_size: usize, coarse_size: usize) -> Vec<f32> {
    let mut coarse = vec![0.0; coarse_size * coarse_size];
    coarse.par_chunks_mut(coarse_size).enumerate().for_each(|(y, row)| {
        for (x, value) in row.iter_mut().enumerate() {
            *value = Water::downsample(fine, fine_size, x as i32, y as i32);
        }
    });
    coarse
}

/// `ErosionWrapperW.SetVars` + `ErodeTerrain`: fills the terrain's basins
/// with rain water and measures the shores. `heightmap` is in 0..1.
pub fn erode_lakes(settings: &LakeSettings, heightmap: &[f32], precipitation_area: Option<&[f32]>) -> WaterOutput {
    let size = settings.resolution;
    let scale_factor = MODEL_SIZE / settings.size;
    let precipitation_amount = scale_factor * settings.precipitation;
    let height_range = scale_factor * settings.height_range;
    let levels = (MODEL_SCALE * 0.5 / (MODEL_SIZE / size as f64)).log2();
    let bias_exponent = (settings.lake_size_bias * 6.0) as f32;
    let bias_power = 0.5f64.powf(bias_exponent as f64) as f32;
    let adjusted = precipitation_amount - (1.0 - bias_power as f64) * 2.0 * (precipitation_amount * ONE_TENTH);
    let mut water = Water {
        use_precipitation: precipitation_area.is_some(),
        precipitation: adjusted,
        precipitation_per_cell: 0.0,
        shore_size: settings.shore_size * 0.1 * size as f64,
        size_bias: settings.size_bias,
        altitude_bias: settings.altitude_bias,
        lake_size_bias: settings.lake_size_bias,
        levels,
        levels_adjusted: levels - settings.lake_size_bias * 6.0,
        water_floor: height_range * settings.water_floor,
        threshold: (adjusted * ONE_TWENTIETH) as f32,
        first: true,
    };

    let mut layer = Layer::new(0, size);
    layer.cell = (MODEL_SIZE / size as f64) as f32;
    layer.precipitation = precipitation_area.map(|area| area.to_vec());
    for (height, &bedrock) in layer.height.iter_mut().zip(heightmap.iter()) {
        *height = (bedrock as f64 * height_range) as f32;
    }
    if levels > 1.0 {
        water.pyramid(&mut layer);
    }
    water.simulate(&mut layer);
    water.shore(&mut layer);
    let inverse = (1.0 / height_range) as f32;
    for value in layer.height.iter_mut() {
        *value *= inverse;
    }
    for value in layer.depth.iter_mut() {
        *value *= inverse;
    }
    let surface = water_surface(&layer.height, &layer.depth, size);
    WaterOutput {
        results: layer.height,
        depth: layer.depth,
        shore: layer.shore,
        surface,
    }
}

/// The water surface over wet cells, and the highest
/// neighbouring water surface over dry cells beside water (0 elsewhere).
fn water_surface(height: &[f32], depth: &[f32], size: usize) -> Vec<f32> {
    let n = size as i32;
    let mut surface = vec![0.0; size * size];
    surface.par_chunks_mut(size).enumerate().for_each(|(y, row)| {
        let y = y as i32;
        for x in 0..n {
            let index = (y * n + x) as usize;
            if depth[index] > 0.0 {
                row[x as usize] = height[index];
                continue;
            }
            let mut best = 0.0f32;
            for neighbour_row in (y - 1).max(0)..=(y + 1).min(n - 1) {
                for column in (x - 1).max(0)..=(x + 1).min(n - 1) {
                    let neighbour = (neighbour_row * n + column) as usize;
                    if depth[neighbour] > 0.0 && !(best > height[neighbour]) {
                        best = height[neighbour];
                    }
                }
            }
            row[x as usize] = best;
        }
    });
    surface
}

/// `ErosionWrapperW.EdgeFill`: floods everything at or below `level` that
/// connects to the map edge (only from edge cells where `mask` > 0.5, when
/// given). Heights and level share the same units.
pub fn edge_fill(heightmap: &[f32], size: usize, level: f32, mask: Option<&[f32]>) -> WaterOutput {
    let n = size as i32;
    let height = heightmap.to_vec();
    let mut water = vec![0.0f32; size * size];
    let mut fill = SpanFill::new(size);
    let below = |x: i32, y: i32| height[(y * n + x) as usize] <= level;
    let seeds = edge_seeds(size);
    for (x, y) in seeds {
        let index = (y * n + x) as usize;
        let allowed = mask.is_none_or(|mask| 0.5 < mask[index]);
        if !(height[index] <= level) || water[index] != 0.0 || !allowed {
            continue;
        }
        let mut flood = |cx: i32, cy: i32| {
            let cell = (cy * n + cx) as usize;
            let mut depth = level - height[cell];
            if depth <= 0.0 {
                depth = 0.0;
            }
            water[cell] = depth;
        };
        fill.fill(x, y, &below, &mut flood);
    }
    let mut results = height.clone();
    let mut depth = vec![0.0f32; size * size];
    for index in 0..size * size {
        results[index] = (results[index] - depth[index]) + water[index];
        depth[index] = water[index];
    }
    let surface = water_surface(&results, &depth, size);
    WaterOutput {
        results,
        depth,
        shore: vec![0.0; size * size],
        surface,
    }
}

/// Edge cells in the order `edge_fill` tries them: top row, bottom row,
/// then the left and right columns between them.
fn edge_seeds(size: usize) -> Vec<(i32, i32)> {
    let n = size as i32;
    let mut seeds = Vec::with_capacity(size * 4);
    for x in 0..n {
        seeds.push((x, 0));
    }
    for x in 0..n {
        seeds.push((x, n - 1));
    }
    for y in 1..n - 1 {
        seeds.push((0, y));
    }
    for y in 1..n - 1 {
        seeds.push((n - 1, y));
    }
    seeds
}
