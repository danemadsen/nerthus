//! The variable-radius blurs over a summed-area table: every pixel
//! averages a box, or a fan of boxes that
//! approximates a disk, sized by a radius map. Sums are kept in double
//! precision with 0.5 taken off every cell.

use std::f64::consts::PI;

/// A summed-area table of a square plane.
struct SummedArea {
    size: usize,
    table: Vec<f64>,
}

impl SummedArea {
    fn new(values: &[f64], size: usize) -> Self {
        let first = Self::running_sums(values, size);
        let table = Self::running_sums(&first, size);
        Self { size, table }
    }

    /// Running sums along each row, less 0.5 per cell, written
    /// transposed.
    fn running_sums(source: &[f64], size: usize) -> Vec<f64> {
        let mut result = vec![0.0f64; size * size];
        for row in 0..size {
            let mut sum = 0.0f64;
            for column in 0..size {
                sum += source[size * row + column] - 0.5;
                result[column * size + row] = sum;
            }
        }
        result
    }

    /// The table sampled bilinearly half a cell up and left of
    /// (`x`, `y`).
    fn sample(&self, x: f64, y: f64) -> f64 {
        let x = x - 0.5;
        let y = y - 0.5;
        let column = x as i32;
        let row = y as i32;
        let fraction_x = x - column as f64;
        let fraction_y = y - row as f64;
        let size = self.size as i32;
        let at = |column: i32, row: i32| self.table[(row * size + column) as usize];
        let upper = at(column, row) * (1.0 - fraction_x) + at(column + 1, row) * fraction_x;
        let lower = at(column, row + 1) * (1.0 - fraction_x) + at(column + 1, row + 1) * fraction_x;
        upper * (1.0 - fraction_y) + lower * fraction_y
    }

    /// The mean over the box between two corners.
    fn box_sum(&self, left: f64, top: f64, right: f64, bottom: f64) -> f64 {
        self.sample(right, bottom) + self.sample(left, top) - self.sample(left, bottom) - self.sample(right, top)
    }
}

/// Averages each pixel of `source` over `directions`
/// boxes whose half-diagonals fan around a quarter circle, `radii ×
/// radius_scale` pixels long. A zero radius keeps the pixel.
pub fn disk_blur(source: &[f32], radii: &[f32], size: usize, radius_scale: f32, directions: i32) -> Vec<f32> {
    if directions < 1 {
        return source.to_vec();
    }
    let count = directions as usize;
    let mut cosines = vec![0.0f64; count];
    let mut sines = vec![0.0f64; count];
    for index in 0..count {
        let share = index as f64 / count as f64;
        let mut curve = share - 0.5;
        curve *= 2.0;
        // `curve *= curve * curve`, as the blur has it.
        #[allow(clippy::misrefactored_assign_op)]
        {
            curve *= curve * curve;
        }
        curve = (curve * 0.5 + 0.5) * 0.4 + 0.6 * share;
        let angle = (1.0 / (count as f64 * 2.0) + curve) * PI * 0.5;
        cosines[index] = angle.cos();
        sines[index] = angle.sin();
    }

    let values: Vec<f64> = source.iter().map(|&value| value as f64).collect();
    let table = SummedArea::new(&values, size);
    let limit = (size as f32 - 1.0) as f64;
    let mut result = vec![0.0f32; size * size];
    for y in 0..size {
        for x in 0..size {
            let index = y * size + x;
            let mut radius = (radii[index] * radius_scale) as f64;
            if radius == 0.0 {
                result[index] = source[index];
                continue;
            }
            radius += 0.5;
            let mut total = 0.0f64;
            let mut area = 0.0f64;
            for direction in 0..count {
                let reach_x = cosines[direction] * radius;
                let reach_y = sines[direction] * radius;
                let left = (x as f64 - reach_x).max(0.0);
                let right = (x as f64 + reach_x).min(limit);
                let top = (y as f64 - reach_y).max(0.0);
                let bottom = (y as f64 + reach_y).min(limit);
                total += table.box_sum(left, top, right, bottom);
                area += (right - left) * (bottom - top);
            }
            total /= area;
            result[index] = total as f32 + 0.5;
        }
    }
    result
}
