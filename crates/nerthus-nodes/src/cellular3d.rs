//! Cellular3D's renderer (`Cellular3D::render` in the SYCL kernels, run
//! as four SYCL kernels): the space under a heightfield is filled with the
//! cells of a jittered 3D grid, and each pixel shows the top of the highest
//! Voronoi cell above it, carved from the cell's neighbours by planes
//! halfway between their points (pushed back by the gap).

use nerthus_engine::{Map, Resolution};
use rayon::prelude::*;

use crate::{gpu, sycl, x86};

/// Cells across the terrain are capped at this.
const MAX_CELLS: f32 = 512.0;

/// The most cells the grid may hold; denser grids get larger cells.
const CELL_BUDGET: f32 = 100_000_000.0;

/// The grid reaches this many cells past the terrain on every side.
const BORDER: i32 = 2;

/// Cells along an axis beyond the last one a pixel falls in: the border on
/// both sides and the last cell itself.
const EXTRA_CELLS: i32 = 2 * BORDER + 1;

/// Grid columns with no cell under the surface report this top layer.
const EMPTY_COLUMN: i32 = -3;

const FNV_OFFSET: u32 = 0x811C_9DC5;
const FNV_PRIME: u32 = 0x0100_0193;

/// A cell whose hash is zero seeds its generator with this instead
/// (xorshift never leaves zero).
const ZERO_HASH_SEED: u32 = 0xA235_1342;

const XORSHIFT_MULTIPLIER: u32 = 0x2545_F381;

/// 2⁻³², turning a 32-bit random number into [0, 1].
const UNIT_SCALE: f32 = 2.328_306_4e-10;

/// The parameters of `Cellular3D_Render`.
pub struct Cellular3dSettings {
    /// Cells across the terrain (`1 / Size`).
    pub cells: f32,
    /// How far each cell's faces are pushed in, in cells.
    pub gap: f32,
    pub scale: [f32; 3],
    pub jitter: [f32; 3],
    pub seed: u32,
    pub terrain_width: f32,
    pub terrain_height: f32,
}

/// One cell of the grid (the first kernel's outputs).
#[derive(Clone, Copy, Default)]
struct Cell {
    /// The jittered point, in cells.
    point: [f32; 3],
    /// `1 − gap` for cells whose point lies under the surface, else zero.
    weight: f32,
    /// The cell's random number in [0, 1], its value in the hash map.
    hash: f32,
}

/// The grid of cells covering the terrain, from `BORDER` cells before it to
/// `BORDER` cells past the last pixel on each axis.
pub(crate) struct Grid {
    pub(crate) columns: usize,
    pub(crate) rows: usize,
    pub(crate) layers: usize,
    /// The highest layer the per-pixel kernel starts from (`zMax`).
    pub(crate) top_layer: i32,
    /// Cells per pixel along x and y.
    pub(crate) cells_per_pixel: [f32; 2],
    /// Pixels per cell along x and y.
    pub(crate) pixels_per_cell: [f32; 2],
    /// Height (0 to 1) per cell along z.
    pub(crate) height_per_cell: f32,
}

impl Grid {
    pub(crate) fn new(resolution: Resolution, settings: &Cellular3dSettings) -> Grid {
        let [scale_x, scale_y, scale_z] = settings.scale;

        // Coarsen grids over budget, by the same factor on every axis.
        let mut cells = x86::minss(settings.cells, MAX_CELLS);
        let budget_columns = scale_x.mul_add(cells, EXTRA_CELLS as f32);
        let budget_rows = scale_y.mul_add(cells, EXTRA_CELLS as f32);
        let budget_layers = ((cells * settings.terrain_height) * scale_z) / settings.terrain_width + EXTRA_CELLS as f32;
        let total = budget_layers * (budget_rows * budget_columns);
        if total > CELL_BUDGET {
            let shrink = CELL_BUDGET / total;
            cells *= cbrt(shrink);
        }

        let cells = x86::minss(cells, MAX_CELLS);
        let layers_per_height = ((cells * settings.terrain_height) * scale_z) / settings.terrain_width;
        let cells_per_pixel_y = (cells * scale_y) / resolution.y as f32;
        let cells_per_pixel_x = (cells * scale_x) / resolution.x as f32;

        let last_x = (resolution.x as i32 - 1) as f32;
        let last_y = (resolution.y as i32 - 1) as f32;
        let last_column = x86::cvttss2si(cells_per_pixel_x * last_x);
        let last_row = x86::cvttss2si(cells_per_pixel_y * last_y);
        let last_layer = x86::cvttss2si(layers_per_height);

        Grid {
            columns: (last_column + EXTRA_CELLS) as usize,
            rows: (last_row + EXTRA_CELLS) as usize,
            layers: (last_layer + EXTRA_CELLS) as usize,
            top_layer: last_layer + BORDER,
            cells_per_pixel: [cells_per_pixel_x, cells_per_pixel_y],
            pixels_per_cell: [1.0 / cells_per_pixel_x, 1.0 / cells_per_pixel_y],
            height_per_cell: 1.0 / layers_per_height,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.columns * self.rows * self.layers
    }

    /// The index of the cell at grid coordinates `x, y, z`, which start at
    /// `-BORDER`.
    fn index(&self, x: i32, y: i32, z: i32) -> usize {
        let layer = (z + BORDER) as usize;
        let row = (y + BORDER) as usize;
        let column = (x + BORDER) as usize;
        (layer * self.rows + row) * self.columns + column
    }
}

/// The UCRT's `cbrtf`, taken to round the double cube root.
fn cbrt(value: f32) -> f32 {
    (value as f64).cbrt() as f32
}

fn xorshift(state: u32) -> u32 {
    let mut next = state ^ (state << 13);
    next ^= next >> 17;
    next ^ (next << 5)
}

/// The 32-bit random number a generator state gives, as a float.
fn random_bits(state: u32) -> f32 {
    let bits = state.wrapping_mul(XORSHIFT_MULTIPLIER).wrapping_sub(1);
    bits as f32
}

/// The first kernel: the cell at grid index `index`, its point jittered
/// from the cell's centre by a generator seeded from its coordinates.
fn make_cell(index: usize, grid: &Grid, heights: &[f32], gap_map: Option<&[f32]>, resolution: Resolution, settings: &Cellular3dSettings) -> Cell {
    let index = index as i32;
    let columns = grid.columns as i32;
    let rows = grid.rows as i32;
    let x = index % columns - BORDER;
    let y = (index / columns) % rows - BORDER;
    let z = index / (columns * rows) - BORDER;

    let mut hash = (x as u32 ^ FNV_OFFSET).wrapping_mul(FNV_PRIME);
    hash = (hash ^ y as u32).wrapping_mul(FNV_PRIME);
    hash = (hash ^ z as u32).wrapping_mul(FNV_PRIME);
    hash = hash.wrapping_add(settings.seed);
    let mut state = if hash != 0 { hash } else { ZERO_HASH_SEED };

    let mut point = [0.0f32; 3];
    let coordinates = [x, y, z];
    for axis in 0..3 {
        state = xorshift(state);
        let offset = random_bits(state).mul_add(UNIT_SCALE, -0.5);
        let centre = coordinates[axis] as f32 + 0.5;
        point[axis] = offset.mul_add(settings.jitter[axis], centre);
    }

    // The pixel under the point, clamped to the map.
    let pixel_x = x86::cvttss2si(point[0] * grid.pixels_per_cell[0]);
    let pixel_y = x86::cvttss2si(point[1] * grid.pixels_per_cell[1]);
    let pixel_x = pixel_x.max(0).min(resolution.x as i32 - 1);
    let pixel_y = pixel_y.max(0).min(resolution.y as i32 - 1);
    let pixel = pixel_y as usize * resolution.x + pixel_x as usize;
    let surface = heights[pixel];

    let mut gap = settings.gap;
    if let Some(gap_map) = gap_map {
        gap *= gap_map[pixel];
    }
    let point_height = point[2] * grid.height_per_cell;

    state = xorshift(state);
    let cell_hash = random_bits(state) * UNIT_SCALE;
    let weight = if point_height < surface { 1.0 - gap } else { 0.0 };
    Cell { point, weight, hash: cell_hash }
}

/// The second and third kernels: for each grid column, the highest layer
/// holding a cell under the surface (`EMPTY_COLUMN` if none), then the
/// highest of each column's 3×3 neighbourhood.
fn column_tops(grid: &Grid, cells: &[Cell]) -> Vec<i32> {
    let column_count = grid.columns * grid.rows;
    let mut tops = vec![EMPTY_COLUMN; column_count];
    for (column, top) in tops.iter_mut().enumerate() {
        for layer in (0..grid.layers).rev() {
            let weight = cells[layer * column_count + column].weight;
            if weight != 0.0 && !weight.is_nan() {
                *top = layer as i32 - BORDER;
                break;
            }
        }
    }

    let mut dilated = vec![i32::MIN; column_count];
    for row in 0..grid.rows as i32 {
        for column in 0..grid.columns as i32 {
            let mut highest = i32::MIN;
            for neighbour_row in row - 1..=row + 1 {
                if neighbour_row < 0 || neighbour_row >= grid.rows as i32 {
                    continue;
                }
                for neighbour_column in column - 1..=column + 1 {
                    if neighbour_column < 0 || neighbour_column >= grid.columns as i32 {
                        continue;
                    }
                    let neighbour = neighbour_row as usize * grid.columns + neighbour_column as usize;
                    highest = highest.max(tops[neighbour]);
                }
            }
            dilated[row as usize * grid.columns + column as usize] = highest;
        }
    }
    dilated
}

/// Where the face between `cell` and its neighbour crosses the pixel's
/// vertical line, as a height, and the z component of the face's normal
/// (positive when the neighbour lies above, so the face is a ceiling).
fn face_height(cell: &Cell, neighbour: &Cell, gap: f32, position: [f32; 2], height_per_cell: f32) -> (f32, f32) {
    let [cell_x, cell_y, cell_z] = cell.point;
    let [neighbour_x, neighbour_y, neighbour_z] = neighbour.point;
    let delta_x = neighbour_x - cell_x;
    let delta_y = neighbour_y - cell_y;
    let delta_z = neighbour_z - cell_z;

    // The normal, by an approximate reciprocal square root and one
    // Newton-Raphson step.
    let length_squared = delta_z.mul_add(delta_z, delta_y.mul_add(delta_y, delta_x * delta_x));
    let estimate = x86::rsqrt(length_squared);
    let correction = (length_squared * estimate).mul_add(estimate, -3.0);
    let inverse_length = (-0.5 * estimate) * correction;
    let normal_x = delta_x * inverse_length;
    let normal_y = delta_y * inverse_length;
    let normal_z = delta_z * inverse_length;

    // The plane through the midpoint, pushed back along the normal by the
    // gap: `point.z − (n.x·(p.x − point.x) + n.y·(p.y − point.y)) / n.z`.
    let mut across_x = (cell_x + neighbour_x).mul_add(-0.5, position[0]);
    across_x = normal_x.mul_add(gap, across_x);
    across_x *= normal_x;
    let mut across = (cell_y + neighbour_y).mul_add(-0.5, position[1]);
    across = normal_y.mul_add(gap, across);
    across = across.mul_add(normal_y, across_x);
    let mut drop = across / normal_z;
    drop = gap.mul_add(normal_z, drop);
    let height = (cell_z + neighbour_z).mul_add(0.5, -drop);
    (height * height_per_cell, normal_z)
}

/// The top of `cell` over the pixel: the lowest ceiling above it, or `None`
/// once a floor rises over a ceiling (the pixel misses the cell).
fn cell_top(grid: &Grid, cells: &[Cell], cell_x: i32, cell_y: i32, cell_z: i32, position: [f32; 2], pixel_cell_y: i32) -> Option<f32> {
    let cell = &cells[grid.index(cell_x, cell_y, cell_z)];
    let gap = 1.0 - cell.weight;
    let mut floor = 0.0f32;
    let mut ceiling = f32::MAX;
    for neighbour_z in cell_z - 1..=cell_z + 1 {
        for neighbour_y in cell_y - 1..=cell_y + 1 {
            for neighbour_x in cell_x - 1..=cell_x + 1 {
                let same_cell = neighbour_x == cell_x && neighbour_y == cell_y && neighbour_z == cell_z;
                if same_cell {
                    continue;
                }
                // Only face neighbours and edge neighbours count; the y
                // offset is measured from the pixel's cell, not this one.
                let offset_x = neighbour_x - cell_x;
                let offset_y = neighbour_y - pixel_cell_y;
                let offset_z = neighbour_z - cell_z;
                let distance_squared = offset_x * offset_x + offset_y * offset_y + offset_z * offset_z;
                if distance_squared as u32 > 3 {
                    continue;
                }

                let neighbour = &cells[grid.index(neighbour_x, neighbour_y, neighbour_z)];
                let (height, normal_z) = face_height(cell, neighbour, gap, position, grid.height_per_cell);
                if !(normal_z > 0.0) {
                    floor = x86::maxss(floor, height);
                }
                if 0.0 < normal_z {
                    ceiling = x86::minss(ceiling, height);
                }
                if floor > ceiling {
                    return None;
                }
            }
        }
    }
    Some(ceiling)
}

/// The fourth kernel: the height and hash of the highest cell over pixel
/// `x, y`, searched downwards from the highest occupied layer nearby and
/// stopping one layer after the first that covers the pixel.
fn render_pixel(x: usize, y: usize, grid: &Grid, cells: &[Cell], tops: &[i32]) -> (f32, f32) {
    let position_x = x as f32 * grid.cells_per_pixel[0];
    let position_y = y as f32 * grid.cells_per_pixel[1];
    let position = [position_x, position_y];
    let pixel_cell_x = x86::cvttss2si(position_x);
    let pixel_cell_y = x86::cvttss2si(position_y);

    let row = (pixel_cell_y.max(-BORDER) + BORDER).min(grid.rows as i32 - 1);
    let column = (pixel_cell_x.max(-BORDER) + BORDER).min(grid.columns as i32 - 1);
    let top = tops[row as usize * grid.columns + column as usize];
    let lowest_layer = -BORDER;
    let mut layer = top.min(grid.top_layer - 1);

    let mut best = 0.0f32;
    let mut best_hash = 0.0f32;
    if layer <= lowest_layer {
        return (best, best_hash);
    }

    let mut covered_before = false;
    loop {
        for cell_y in pixel_cell_y - 1..=pixel_cell_y + 1 {
            for cell_x in pixel_cell_x - 1..=pixel_cell_x + 1 {
                let cell = &cells[grid.index(cell_x, cell_y, layer)];
                if cell.weight == 0.0 || cell.weight.is_nan() {
                    continue;
                }
                let Some(cell_top) = cell_top(grid, cells, cell_x, cell_y, layer, position, pixel_cell_y) else {
                    continue;
                };
                if cell_top > best {
                    best = cell_top;
                    best_hash = cell.hash;
                }
            }
        }

        let covered = best > 0.0;
        if covered_before || layer - 1 <= lowest_layer {
            break;
        }
        covered_before = covered;
        layer -= 1;
    }
    (best, best_hash)
}

/// Renders the cells under `heights`, giving the cell surface and each
/// pixel's cell hash. `gap_map` scales the gap per pixel. Runs on the GPU
/// when one is active (see [`sycl`]).
pub fn render(heights: &Map, gap_map: Option<&Map>, settings: &Cellular3dSettings) -> (Map, Map) {
    let gpu = |device: &nerthus_compute::GpuDevice| gpu::cellular3d::render(device, heights, gap_map, settings);
    let cpu = || render_cpu(heights, gap_map, settings);
    sycl::run("Cellular3D_Render", heights.width(), sycl::CELLULAR, gpu, cpu)
}

/// [`render`] on the CPU.
pub fn render_cpu(heights: &Map, gap_map: Option<&Map>, settings: &Cellular3dSettings) -> (Map, Map) {
    let resolution = heights.resolution();
    let grid = Grid::new(resolution, settings);
    let height_data = heights.channel(0);
    let gap_data = gap_map.map(|map| map.channel(0));

    let cell_indices = (0..grid.len()).into_par_iter();
    let cells: Vec<Cell> = cell_indices.map(|index| make_cell(index, &grid, height_data, gap_data, resolution, settings)).collect();
    let tops = column_tops(&grid, &cells);

    let mut surface = vec![0.0f32; resolution.x * resolution.y];
    let mut hashes = vec![0.0f32; resolution.x * resolution.y];
    let surface_rows = surface.par_chunks_mut(resolution.x);
    let hash_rows = hashes.par_chunks_mut(resolution.x);
    surface_rows.zip(hash_rows).enumerate().for_each(|(y, (surface_row, hash_row))| {
        for x in 0..resolution.x {
            let (height, hash) = render_pixel(x, y, &grid, &cells, &tops);
            surface_row[x] = height;
            hash_row[x] = hash;
        }
    });

    let surface_map = Map::from_data(resolution, surface);
    let hash_map = Map::from_data(resolution, hashes);
    (surface_map, hash_map)
}
