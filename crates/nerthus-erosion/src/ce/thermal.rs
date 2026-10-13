//! Thermal erosion (thermal model 1, used by the Thermal node): the
//! talus/weathering pass and its helpers, transcribed from the CE engine's
//! compiled code.

use nerthus_noise::{CellularReturnType, FastNoise, NoiseType, PerturbType};

use super::engine::Erosion;
use super::kernel::FloatMap;
use super::layer::Layer;

/// Neighbour offsets and distance weights.
const NEIGHBOUR_DX: [i32; 8] = [-1, 0, 1, -1, 1, -1, 0, 1];
const NEIGHBOUR_DY: [i32; 8] = [-1, -1, -1, 0, 0, 1, 1, 1];
const NEIGHBOUR_WEIGHT: [f32; 8] = [0.707, 1.0, 0.707, 1.0, 1.0, 0.707, 1.0, 0.707];

#[inline]
fn min_ss(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

#[inline]
fn clamp_low(value: i32) -> i32 {
    if value < 0 { 0 } else { value }
}

#[inline]
fn clamp_high(value: i32, size: i32) -> i32 {
    if value > size - 1 { size - 1 } else { value }
}

/// Copies loose sediment into the thermal scratch map.
pub(super) fn copy_sediment(layer: &mut Layer) {
    let thermal = layer.thermal.as_mut().expect("thermal map exists with thermal erosion");
    thermal.data.copy_from_slice(&layer.sediment.data);
}

/// Applies the redistributed sediment.
pub(super) fn apply_sediment(layer: &mut Layer) {
    let thermal = layer.thermal.as_ref().expect("thermal map exists with thermal erosion");
    for index in 0..layer.height.data.len() {
        let moved = thermal.data[index];
        layer.height.data[index] = (layer.height.data[index] - layer.sediment.data[index]) + moved;
        layer.sediment.data[index] = moved;
    }
}

/// The 3x3 neighbourhood of a cell in one map, edge-clamped.
struct Neighbourhood {
    top_left: f32,
    top: f32,
    top_right: f32,
    left: f32,
    centre: f32,
    right: f32,
    bottom_left: f32,
    bottom: f32,
    bottom_right: f32,
}

impl Neighbourhood {
    fn read(map: &FloatMap, x: i32, row: i32) -> Self {
        let size = map.size;
        let above = clamp_high(clamp_low(row - 1), size);
        let middle = clamp_high(clamp_low(row), size);
        let below = clamp_high(clamp_low(row + 1), size);
        let left = clamp_high(clamp_low(x - 1), size);
        let centre_column = clamp_high(clamp_low(x), size);
        let right = clamp_high(clamp_low(x + 1), size);
        Self {
            top_left: map.at(left, above),
            top: map.at(centre_column, above),
            top_right: map.at(right, above),
            left: map.at(left, middle),
            centre: map.at(centre_column, middle),
            right: map.at(right, middle),
            bottom_left: map.at(left, below),
            bottom: map.at(centre_column, below),
            bottom_right: map.at(right, below),
        }
    }
}

/// The talus pass for job `job`: sheds loose sediment downhill to the
/// thermal scratch map and weathers rock into sediment. Jobs visit rows
/// interleaved by quarters (`job / (res / 4) + 4 * job % res`).
pub(super) fn thermal_pass(erosion: &Erosion, layer: &mut Layer, job: i32) {
    let resolution = layer.resolution;
    let add_to_scratch = if (erosion.iterations as f32) * erosion.thermal_scale > erosion.iteration as f32 { 1.0f32 } else { 0.0 };
    let anisotropy = erosion.thermal_smoothness;
    let mut rounding = ((anisotropy - 0.5) + (anisotropy - 0.5)) as f32;
    let quarter = resolution / 4;
    let row = job / quarter + (job * 4) % resolution;
    if !(rounding > 0.0) {
        rounding = 0.0;
    }
    let mut blend = (anisotropy + anisotropy) as f32;
    if !(1.0 > blend) {
        blend = 1.0;
    }
    for x in 0..resolution {
        let keep = if erosion.sediment_removal {
            let removal = layer.removal.as_ref().expect("removal map present");
            (1.0f64 - removal.at(x, row) as f64) as f32
        } else {
            1.0
        };
        let talus = if erosion.iteration == -1 { 0.0f32 } else { (layer.cell as f64 * erosion.talus_slope) as f32 };
        let sediment_index = layer.sediment.index(x, clamp_high(clamp_low(row), resolution));
        let sediment_here = layer.sediment.data[layer.sediment.index(x, row)];
        let h = Neighbourhood::read(&layer.height, x, row);
        let centre = layer.height.at(x, row);

        // Steepest drop towards the lowest neighbour (diagonals weighted).
        let lowest_top = min_ss(h.top_left, h.top_right);
        let lowest_bottom = min_ss(h.bottom_left, h.bottom_right);
        let lowest_diagonal = if lowest_bottom > lowest_top { lowest_top } else { lowest_bottom };
        let lowest_top_left = min_ss(h.top, h.left);
        let lowest_right_bottom = min_ss(h.right, h.bottom);
        // The CE engine weighs diagonals by 0.7071, not 1/√2.
        #[allow(clippy::approx_constant)]
        let mut steepest = (lowest_diagonal - centre) * 0.7071;
        let lowest_straight = if lowest_right_bottom > lowest_top_left { lowest_top_left } else { lowest_right_bottom };
        if !(lowest_straight - centre > steepest) {
            steepest = lowest_straight - centre;
        }
        if !(0.0 > steepest) {
            steepest = 0.0;
        }

        if sediment_here > 0.0 {
            let gradient_x = (((h.right + h.top_right) + h.right) + h.bottom_right) - (((h.left + h.top_left) + h.left) + h.bottom_left);
            let gradient_x = gradient_x * 0.125;
            let gradient_y = (((h.bottom + h.bottom_left) + h.bottom) + h.bottom_right) - (((h.top + h.top_left) + h.top) + h.top_right);
            let gradient_y = gradient_y * 0.125;
            let slope = (gradient_y * gradient_y + gradient_x * gradient_x).sqrt();
            let mut drops = [0.0f32; 8];
            let mut total = 0.0f32;
            let mut excess = 0.0f32;
            for (k, drop_slot) in drops.iter_mut().enumerate() {
                let nx = NEIGHBOUR_DX[k] + x;
                let ny = NEIGHBOUR_DY[k] + row;
                let inside = nx >= 0 && nx < resolution && ny >= 0 && ny < resolution;
                let active = erosion.last_substep || slope > talus || erosion.settling;
                if !inside || !active {
                    *drop_slot = -1.0;
                    continue;
                }
                let neighbour = layer.height.at(clamp_high(nx, resolution), clamp_high(ny, resolution));
                let drop = centre - neighbour;
                *drop_slot = drop;
                if drop > 0.0 {
                    total += drop;
                    let over = drop - talus * NEIGHBOUR_WEIGHT[k];
                    excess += if 0.0 > over { 0.0 } else { over };
                }
            }
            let mut rate = if erosion.last_substep { 0.1f32 } else { 0.16 };
            if !erosion.last_substep {
                excess = total;
            }
            let available = if erosion.settling && !(talus < slope) {
                let sediment = layer.sediment.data[sediment_index];
                let share = (sediment * 6.0) as f64 / erosion.thermal_size;
                let limited = if 1.0 < share { 1.0 } else { share };
                rate = (limited * rate as f64) as f32;
                sediment
            } else {
                sediment_here
            };
            let moved = min_ss(rate * excess, available);
            if moved > 0.0 {
                let thermal = layer.thermal.as_mut().expect("thermal map exists with thermal erosion");
                for (k, &drop) in drops.iter().enumerate() {
                    if !(drop > 0.0) {
                        continue;
                    }
                    let nx = NEIGHBOUR_DX[k] + x;
                    let ny = NEIGHBOUR_DY[k] + row;
                    let mut share = (drop * moved) / total;
                    if keep > 0.0 {
                        let trail = layer.trail.as_ref().map_or(0.0, |trail| trail.data[(ny * trail.size + nx) as usize] as i32 as f32);
                        share = share * keep - trail * keep;
                        if !(share > 0.0) {
                            share = 0.0;
                        }
                    }
                    let index = thermal.index(nx, ny);
                    thermal.data[index] += share;
                }
                let index = thermal.index(x, row);
                thermal.data[index] -= moved;
            }
        }

        let weathering_iterations = erosion.iterations as f32 * layer.thermal_factor;
        if erosion.finishing || !(weathering_iterations > erosion.iteration as f32) {
            continue;
        }
        // Rock surface (height minus loose sediment) around the cell.
        let s = Neighbourhood::read(&layer.sediment, x, row);
        let mut top_left = h.top_left - s.top_left;
        let mut top = h.top - s.top;
        let mut top_right = h.top_right - s.top_right;
        let mut left = h.left - s.left;
        let mut right = h.right - s.right;
        let mut bottom_left = h.bottom_left - s.bottom_left;
        let mut bottom = h.bottom - s.bottom;
        let mut bottom_right = h.bottom_right - s.bottom_right;
        let rock = centre - s.centre;
        let diagonals = ((top_right + top_left) + bottom_left) + bottom_right;
        let straights = ((left + top) + right) + bottom;
        #[allow(clippy::approx_constant)]
        let curvature = rock - (diagonals * 0.7071 + straights) * 0.1464472;
        // Mirror neighbours lying below the cell so ridges and gullies read
        // as slopes.
        let mirror = |value: f32| (rock + rock) - value;
        if bottom_right > top_left && rock > top_left {
            bottom_right = mirror(top_left);
        }
        if bottom > top && rock > top {
            bottom = mirror(top);
        }
        if bottom_left > top_right && rock > top_right {
            bottom_left = mirror(top_right);
        }
        if right > left && rock > left {
            right = mirror(left);
        }
        if left > right && rock > right {
            left = mirror(right);
        }
        if top_right > bottom_left && rock > bottom_left {
            top_right = mirror(bottom_left);
        }
        if top > bottom && rock > bottom {
            top = mirror(bottom);
        }
        if top_left > bottom_right && rock > bottom_right {
            top_left = mirror(bottom_right);
        }
        let cell = layer.cell;
        let rock_gradient_x = ((((right + top_right) + right) + bottom_right) - (((left + top_left) + left) + bottom_left)) * 0.125;
        let rock_gradient_y = ((((bottom + bottom_left) + bottom) + bottom_right) - (((top + top_left) + top) + top_right)) * 0.125;
        let rock_slope = (rock_gradient_y * rock_gradient_y + rock_gradient_x * rock_gradient_x).sqrt() / cell;
        let index = layer.sediment.index(x, row);
        let sediment = layer.sediment.data[index];
        let mut weathering = if rock_slope as f64 > erosion.talus_slope {
            let flat = 1.0 - rounding;
            let mut shape = flat;
            if curvature > 0.0 {
                shape = ((curvature / cell) * rounding * rounding * 20.0 + 1.0) * (rounding * 1.6 + flat);
            }
            shape * rock_slope * blend + (1.0 - blend) * flat
        } else {
            0.0
        };
        weathering *= cell;
        let mask = if erosion.mask_as_thermal {
            layer.mask.as_ref().expect("mask present for thermal strength").at(x, row)
        } else {
            1.0
        };
        let mut amount = (erosion.thermal_strength * 0.02199999988079071 * mask as f64 * layer.thermal_factor as f64 * weathering as f64) as f32;
        let span = (cell * 4.0) as f64;
        if span > erosion.thermal_size {
            amount *= (erosion.thermal_size / span) as f32;
        }
        let cover = 1.0 - (sediment * 0.15) / cell;
        amount *= if 0.0 > cover { 0.0 } else { cover };
        let limit = -steepest;
        if !(limit > amount) {
            amount = limit;
        }
        layer.sediment.data[index] = sediment + amount;
        if add_to_scratch > 0.0 {
            let thermal = layer.thermal.as_mut().expect("thermal map exists with thermal erosion");
            thermal.data[index] += amount * add_to_scratch;
        }
        layer.wear.data[index] += amount;
    }
}

/// Octave count the CE engine derives from a feature size in cells:
/// `2 - (int)(logf(cells) * -1.442695)`.
#[allow(clippy::approx_constant)]
fn detail_octaves(cells: f32) -> i32 {
    let logarithm = (cells as f64).ln() as f32;
    2 - (logarithm * -1.442695f32) as i32
}

/// The cellular "pebble" noise of the fine-detail pass: Distance2Sub
/// cells perturbed by gradient fractal noise, sampled on the terrain
/// surface (`x`, `row`, `height / cell`).
fn pebble_noise(cells: f32, positions: &[[f32; 3]]) -> Vec<f32> {
    let mut noise = FastNoise::new(1337);
    noise.set_frequency(1.0 / cells);
    noise.set_noise_type(NoiseType::Cellular);
    noise.set_cellular_return_type(CellularReturnType::Distance2Sub);
    noise.set_perturb_type(PerturbType::GradientFractal);
    noise.set_perturb_amp_raw(f32::from_bits(0x394d_000d));
    noise.set_perturb_frequency(4.0);
    noise.set_perturb_fractal_octaves(detail_octaves(cells * 0.25));
    noise.fill_vector_set(positions, [0.0, 0.0, 0.0])
}

/// "Fine Detail" for row `row`: roughens slopes with
/// fractal noise scaled by wear, and lays debris-sized pebble noise over
/// loose sediment.
pub(super) fn fine_detail(erosion: &Erosion, layer: &mut Layer, row: i32) {
    let resolution = layer.resolution;
    let size = resolution as usize;
    let cells = resolution as f32;
    let debris_cells = (cells * erosion.debris_size as f32) / erosion.size as f32;
    let thermal_cells = (cells * (erosion.thermal_size * 4.0) as f32) / erosion.size as f32;
    let cell = layer.cell;
    let wear_scale = cell * 0.5 * thermal_cells;
    let debris_height = cell * 0.15 * debris_cells;

    let mut positions = Vec::with_capacity(size);
    for x in 0..resolution {
        let height = layer.height.at(x, row);
        positions.push([x as f32, row as f32, height / cell]);
    }

    let mut roughness = FastNoise::new(1337);
    roughness.set_frequency(1.0 / thermal_cells);
    roughness.set_fractal_octaves(detail_octaves(thermal_cells));
    roughness.set_noise_type(NoiseType::PerlinFractal);
    let mut rough = vec![0.0f32; size];
    roughness.fill_noise_set(&mut rough, 0x2b, row, 0x1d, size, 1, 1, 1.0);

    let small = pebble_noise(debris_cells, &positions);
    let medium = pebble_noise(debris_cells + debris_cells, &positions);
    let large = pebble_noise(debris_cells * 4.0, &positions);

    let mut clumping = FastNoise::new(1337);
    clumping.set_frequency(1.0 / (debris_cells * 16.0));
    clumping.set_noise_type(NoiseType::Perlin);
    let mut clumps = vec![0.0f32; size];
    clumping.fill_noise_set(&mut clumps, 0, row, 0, size, 1, 1, 1.0);

    for x in 0..resolution {
        let column = x as usize;
        let mut spread = (clumps[column] as f64 * 0.5 + 0.5) as f32;
        spread = spread * spread;
        spread = spread * spread;
        let sparsity = 1.0 / (spread * 10.0 + 1.0) - 0.9;
        let mut pebbles = small[column];
        let mut medium_pebbles = (sparsity + medium[column]) + (sparsity + medium[column]) - 0.5;
        if !(medium_pebbles > 0.0) {
            medium_pebbles = 0.0;
        }
        if !(pebbles > medium_pebbles) {
            pebbles = medium_pebbles;
        }
        let mut large_pebbles = (sparsity + large[column]) * 4.0 - 1.5;
        if !(large_pebbles > 0.0) {
            large_pebbles = 0.0;
        }
        if !(pebbles > large_pebbles) {
            pebbles = large_pebbles;
        }
        let index = layer.height.index(x, row);
        let height = layer.height.data[index];
        let sediment = layer.sediment.data[index];
        let wear = layer.wear.data[index];
        let worn = min_ss(wear * 0.5 / wear_scale, 1.0);
        let rock = ((worn * wear_scale) * (rough[column] + 0.5)) + (height - sediment);
        let cover = min_ss(1.0, sediment / debris_height);
        let detailed = if sediment > 0.0 {
            (((1.0 - cover) * rock + cover * height) - debris_height * 0.75) + pebbles * debris_height
        } else {
            rock
        };
        layer.height.data[index] = if detailed > rock { detailed } else { rock };
    }
}
