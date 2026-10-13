//! Port of the TS thermal shaper behind the ThermalShaper (`TS`)
//! node: slopes relax towards their lowest neighbours over a resolution
//! pyramid, transcribed from the disassembly.

use rayon::prelude::*;

use crate::ce::kernel::{downsample, kernel_sum, upsample, FloatMap};

/// The wrapper's terrain size and feature scale (`SetVars(res, 5000, 38, ...)`).
const SIZE: f64 = 5000.0;
const SCALE: f64 = 38.0;
/// Base iteration count before the pyramid adjustment.
const BASE_ITERATIONS: i32 = 64;
/// Weight of the diagonal neighbours.
const DIAGONAL: f32 = f32::from_bits(0x3f35_0481);
/// One over the neighbour weight total.
const NEIGHBOUR_NORM: f32 = f32::from_bits(0x3e15_f641);
/// Residual detail is scaled by this over the resolution.
const DETAIL_SCALE: f32 = 256.0;

/// One level of the pyramid.
struct Level {
    level: i32,
    height: FloatMap,
    /// Detail the coarser level lost (fine levels only).
    residual: Vec<f32>,
}

struct Shaper {
    levels: f32,
    iterations: i32,
    intensity: f32,
    first: bool,
}

/// `logf` / `powf` as the CRT computes them: in double, rounded once.
fn log_f32(value: f32) -> f32 {
    (value as f64).ln() as f32
}

fn pow_f32(base: f32, exponent: f32) -> f32 {
    (base as f64).powf(exponent as f64) as f32
}

/// MINSS: the first operand when it is smaller, otherwise the second.
fn min_ss(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

/// MAXSS: the first operand when it is larger, otherwise the second.
fn max_ss(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

fn clamp_index(value: i32, size: i32) -> i32 {
    let low = if value < 0 { 0 } else { value };
    if low > size - 1 { size - 1 } else { low }
}

impl Shaper {
    /// How far one cell moves this step.
    fn change(&self, map: &FloatMap, x: i32, y: i32) -> f32 {
        let size = map.size;
        let column = |offset: i32| clamp_index(x + offset, size);
        let row = |offset: i32| clamp_index(y + offset, size) * size;
        let at = |dx: i32, dy: i32| map.data[(row(dy) + column(dx)) as usize];
        let top_left = at(-1, -1);
        let top = at(0, -1);
        let top_right = at(1, -1);
        let left = at(-1, 0);
        let right = at(1, 0);
        let bottom_left = at(-1, 1);
        let bottom = at(0, 1);
        let bottom_right = at(1, 1);
        let centre = map.data[(y * size + x) as usize];

        let diagonals = ((top_right + top_left) + bottom_left) + bottom_right;
        let sides = ((left + top) + right) + bottom;
        let mean = (diagonals * DIAGONAL + sides) * NEIGHBOUR_NORM;
        let detail = centre - mean;

        let east = ((right + top_right) + right) + bottom_right;
        let west = ((left + top_left) + left) + bottom_left;
        let south = ((bottom + bottom_left) + bottom) + bottom_right;
        let north = ((top + top_left) + top) + top_right;

        let upper_diagonal = min_ss(top_left, top_right);
        let lower_diagonal = min_ss(bottom_left, bottom_right);
        let lowest_diagonal = if lower_diagonal > upper_diagonal { upper_diagonal } else { lower_diagonal };
        let vertical = min_ss(top, left);
        let horizontal = min_ss(right, bottom);
        let diagonal_drop = (lowest_diagonal - centre) * DIAGONAL;
        let lowest_side = if horizontal > vertical { vertical } else { horizontal };
        let side_drop = lowest_side - centre;
        let drop = if side_drop > diagonal_drop { diagonal_drop } else { side_drop };

        let gradient_x = (east - west) * 0.125;
        let gradient_y = (south - north) * 0.125;
        let slope = (gradient_y * gradient_y + gradient_x * gradient_x).sqrt();

        let depth = -drop;
        let pressure = -(slope + detail);
        let mut push = min_ss(0.0, pressure);
        if depth > 0.0 {
            push = -log_f32(1.0 - push / depth) * depth;
        }
        let moved = (pow_f32(slope, 0.4) * push) * self.intensity;
        max_ss(drop, moved)
    }

    /// The shaping passes of one level.
    fn shape(&mut self, level: &mut Level) {
        let mut fraction = 1.0f32;
        let remaining = self.levels - level.level as f32;
        if remaining < 1.0 {
            fraction = pow_f32(2.0, remaining) - 1.0;
        }
        let passes = (self.iterations as f32 * fraction) as i32;
        let size = level.height.size as usize;
        for _ in 0..passes.max(0) {
            let map = &level.height;
            let mut changes = vec![0.0f32; size * size];
            changes.par_chunks_mut(size).enumerate().for_each(|(y, row)| {
                for (x, change) in row.iter_mut().enumerate() {
                    *change = self.change(map, x as i32, y as i32);
                }
            });
            // Apply and clamp to -1..1.
            level.height.data.par_iter_mut().zip(changes.par_iter()).for_each(|(height, &change)| {
                *height = (change + *height).clamp(-1.0, 1.0);
            });
        }
        if !self.first {
            self.iterations = ((self.iterations - 1) >> 1) + 1;
            self.intensity *= 0.5;
        }
        self.first = false;
    }

    /// Shapes a half-resolution copy first and brings the
    /// result back with (scaled) fine detail.
    fn pyramid(&mut self, fine: &mut Level) {
        let fine_size = fine.height.size;
        let coarse_size = fine_size >> 1;
        let sum = kernel_sum();
        let mut coarse_height = FloatMap::new(coarse_size, 0.0);
        coarse_height.data.par_chunks_mut(coarse_size as usize).enumerate().for_each(|(y, row)| {
            for (x, value) in row.iter_mut().enumerate() {
                *value = downsample(&fine.height, x as i32, y as i32, sum);
            }
        });
        let mut coarse = Level {
            level: fine.level + 1,
            height: coarse_height,
            residual: Vec::new(),
        };
        let detail_scale = DETAIL_SCALE / fine_size as f32;
        let mut residual = vec![0.0f32; (fine_size * fine_size) as usize];
        residual.par_chunks_mut(fine_size as usize).enumerate().for_each(|(y, row)| {
            for (x, value) in row.iter_mut().enumerate() {
                let smooth = upsample(&coarse.height, x as u32, y as u32);
                *value = (fine.height.data[y * fine_size as usize + x] - smooth) * detail_scale;
            }
        });
        fine.residual = residual;
        let deeper = (coarse.level as f64) < self.levels as f64 - 1.0 && coarse_size > 11;
        if deeper {
            self.pyramid(&mut coarse);
        }
        self.shape(&mut coarse);
        let residual = std::mem::take(&mut fine.residual);
        fine.height.data.par_chunks_mut(fine_size as usize).enumerate().for_each(|(y, row)| {
            for (x, value) in row.iter_mut().enumerate() {
                let smooth = upsample(&coarse.height, x as u32, y as u32);
                *value = smooth + residual[y * fine_size as usize + x];
            }
        });
    }
}

/// `ErosionWrapperTS.SetVars(res, 5000, 38, 0, shape)` + `ErodeTerrain`.
pub fn thermal_shape(heights: &[f32], size: usize, shape: f64) -> Vec<f32> {
    let sign = if shape > 0.0 { -1.0f64 } else { 1.0 };
    let levels = ((SCALE * 0.5 / (SIZE / size as f64)).ln() * std::f64::consts::LOG2_E) as f32;
    let base = BASE_ITERATIONS as f32;
    let fraction = pow_f32(2.0, levels - (levels as i32) as f32);
    let halving = (0.5f64.powf((levels as i32) as f64) - 1.0) * BASE_ITERATIONS as f64;
    let denominator = (((fraction - 1.0) * base) as f64 - (halving + halving)) as i32 as f32;
    let iterations = (((base * 1.5) / denominator) * base) as i32;
    let mut shaper = Shaper {
        levels,
        iterations,
        intensity: ((shape * 4.0) as f32).abs(),
        first: true,
    };

    let data: Vec<f32> = heights.iter().map(|&value| (value as f64 * sign) as f32).collect();
    let mut level = Level {
        level: 0,
        height: FloatMap { size: size as i32, data },
        residual: Vec::new(),
    };
    if 1.0 < levels {
        shaper.pyramid(&mut level);
    }
    shaper.shape(&mut level);
    level.height.data.iter().map(|&value| (value as f64 * sign) as f32).collect()
}
