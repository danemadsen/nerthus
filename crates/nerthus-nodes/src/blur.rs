//! The fast fractional-radius box blur.
//!
//! Each pass blurs rows with a running sum and writes the result transposed;
//! two passes therefore blur both axes. `iterations` pairs of passes
//! approximate a gaussian.

use rayon::prelude::*;

/// Blurs the square `data` by `radius`, `iterations` pairs of passes.
pub fn box_blur(data: &[f32], size: usize, radius: f32, iterations: usize) -> Vec<f32> {
    let whole = radius as usize;
    let fraction = radius - whole as f32;
    let mut current = data.to_vec();
    let mut scratch = vec![0.0f32; size * size];
    if size < 2 * whole + 2 {
        // The running window would overrun the row; clamp the radius like a
        // full-row average would.
        let clamped = ((size.saturating_sub(2)) / 2) as f32;
        return box_blur(data, size, clamped.min(radius), iterations);
    }
    for _ in 0..iterations {
        box_pass(&current, &mut scratch, size, whole, fraction);
        std::mem::swap(&mut current, &mut scratch);
        box_pass(&current, &mut scratch, size, whole, fraction);
        std::mem::swap(&mut current, &mut scratch);
    }
    current
}

/// One pass: horizontal running blur, transposed output.
fn box_pass(source: &[f32], target: &mut [f32], size: usize, whole: usize, fraction: f32) {
    let complement = 1.0 - fraction;
    let norm = 1.0 / ((whole + whole) as f32 + fraction + fraction + 1.0);
    let columns: Vec<Vec<f32>> = (0..size)
        .into_par_iter()
        .map(|row| {
            let base = row * size;
            let mut column = vec![0.0f32; size];
            let mut leaving = source[base];
            let mut sum = (whole as f32 + 1.0 + fraction) * leaving;
            for j in 0..whole {
                sum += source[base + j];
            }
            let mut entering = source[base + whole];
            sum += entering * fraction;
            let mut out = 0usize;
            let mut right = base + whole + 1;
            for _ in 0..whole + 1 {
                sum -= leaving;
                sum += entering * complement;
                entering = source[right];
                right += 1;
                sum += entering * fraction;
                column[out] = sum * norm;
                out += 1;
            }
            let mut left = base;
            for _ in whole + 1..size - whole - 1 {
                sum -= leaving * fraction;
                leaving = source[left];
                left += 1;
                sum -= leaving * complement;
                sum += entering * complement;
                entering = source[right];
                right += 1;
                sum += entering * fraction;
                column[out] = sum * norm;
                out += 1;
            }
            let last = source[base + size - 1];
            for _ in size - whole - 1..size {
                sum -= leaving * fraction;
                leaving = source[left];
                left += 1;
                sum -= leaving * complement;
                sum += entering * complement;
                entering = last;
                sum += entering * fraction;
                column[out] = sum * norm;
                out += 1;
            }
            column
        })
        .collect();
    // Transposed write: row `r` of the source becomes column `r` of the target.
    target.par_chunks_mut(size).enumerate().for_each(|(out_row, values)| {
        for (row, column) in columns.iter().enumerate() {
            values[row] = column[out_row];
        }
    });
}

/// Q28 fixed-point number used by the fixed box blur.
#[derive(Clone, Copy)]
struct Fixed(i64);

const FIXED_SHIFT: u32 = 28;
const FIXED_ONE: f64 = (1i64 << FIXED_SHIFT) as f64;

impl Fixed {
    fn from_f32(value: f32) -> Self {
        Fixed((value as f64 * FIXED_ONE) as i64)
    }

    fn from_int(value: i64) -> Self {
        Fixed(value << FIXED_SHIFT)
    }

    fn add(self, other: Fixed) -> Fixed {
        Fixed(self.0 + other.0)
    }

    fn sub(self, other: Fixed) -> Fixed {
        Fixed(self.0 - other.0)
    }

    fn mul(self, other: Fixed) -> Fixed {
        Fixed((self.0 * other.0) >> FIXED_SHIFT)
    }

    fn to_f64(self) -> f64 {
        self.0 as f64 / FIXED_ONE
    }
}

/// The same running box blur as [`box_blur`] accumulated in 64-bit fixed
/// point.
pub fn fixed_box_blur(data: &[f32], size: usize, radius: f32, iterations: usize) -> Vec<f32> {
    let whole = radius as usize;
    let fraction = radius - whole as f32;
    let fraction_fixed = Fixed::from_f32(fraction);
    let complement_fixed = Fixed::from_int(1).sub(fraction_fixed);
    let mut current = data.to_vec();
    let mut scratch = vec![0.0f32; size * size];
    for _ in 0..iterations {
        for _ in 0..2 {
            fixed_box_pass(&current, &mut scratch, size, whole, fraction, fraction_fixed, complement_fixed);
            std::mem::swap(&mut current, &mut scratch);
        }
    }
    current
}

/// One running-sum pass with transposed output.
fn fixed_box_pass(source: &[f32], target: &mut [f32], size: usize, whole: usize, fraction: f32, fraction_fixed: Fixed, complement_fixed: Fixed) {
    let norm = 1.0 / (((whole + whole) as f32 + fraction + fraction) as f64 + 1.0);
    let columns: Vec<Vec<f32>> = (0..size)
        .into_par_iter()
        .map(|row| {
            let base = row * size;
            let mut column = vec![0.0f32; size];
            let mut leaving = Fixed::from_f32(source[base]);
            let leading_weight = Fixed::from_int(whole as i64 + 1).add(fraction_fixed);
            let mut sum = leading_weight.mul(leaving);
            for index in 0..whole {
                sum = sum.add(Fixed::from_f32(source[base + index]));
            }
            let mut entering = Fixed::from_f32(source[base + whole]);
            sum = sum.add(entering.mul(fraction_fixed));
            let mut out = 0usize;
            let mut right = base + whole + 1;
            for _ in 0..whole + 1 {
                sum = sum.sub(leaving);
                sum = sum.add(entering.mul(complement_fixed));
                entering = Fixed::from_f32(source[right]);
                right += 1;
                sum = sum.add(entering.mul(fraction_fixed));
                column[out] = (sum.to_f64() * norm) as f32;
                out += 1;
            }
            let mut left = base;
            for _ in whole + 1..size - whole - 1 {
                sum = sum.sub(leaving.mul(fraction_fixed));
                leaving = Fixed::from_f32(source[left]);
                left += 1;
                sum = sum.sub(leaving.mul(complement_fixed));
                sum = sum.add(entering.mul(complement_fixed));
                entering = Fixed::from_f32(source[right]);
                right += 1;
                sum = sum.add(entering.mul(fraction_fixed));
                column[out] = (sum.to_f64() * norm) as f32;
                out += 1;
            }
            let last = source[base + size - 1];
            for _ in size - whole - 1..size {
                sum = sum.sub(leaving.mul(fraction_fixed));
                leaving = Fixed::from_f32(source[left]);
                left += 1;
                sum = sum.sub(leaving.mul(complement_fixed));
                sum = sum.add(entering.mul(complement_fixed));
                entering = Fixed::from_f32(last);
                sum = sum.add(entering.mul(fraction_fixed));
                column[out] = (sum.to_f64() * norm) as f32;
                out += 1;
            }
            column
        })
        .collect();
    target.par_chunks_mut(size).enumerate().for_each(|(out_row, values)| {
        for (row, column) in columns.iter().enumerate() {
            values[row] = column[out_row];
        }
    });
}
