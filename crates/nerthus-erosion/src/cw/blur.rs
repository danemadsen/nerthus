//! The CW engine's fixed-point box blur, one pass per row: a running sum in
//! Q24 with a fractional radius, written transposed so two passes blur both
//! axes. Every product is rounded
//! through a float and back, exactly as compiled.

use rayon::prelude::*;

const ONE: f32 = 16_777_216.0;
const INVERSE_ONE: f32 = 1.0 / 16_777_216.0;

/// `cvttss2si` to 64 bits: truncation, with NaN and out-of-range values
/// giving the "integer indefinite" value.
fn truncate(value: f32) -> i64 {
    let limit = 9_223_372_036_854_775_808.0_f32;
    if value.is_nan() || value >= limit || value < -limit {
        return i64::MIN;
    }
    value as i64
}

/// A sample in Q24.
fn fixed(value: f32) -> i64 {
    truncate(value * ONE)
}

/// A Q24 value rounded to float precision (`(float)x * 2^-24 * 2^24`).
fn requantize(value: i64) -> i64 {
    let scaled = (value as f32) * INVERSE_ONE;
    truncate(scaled * ONE)
}

/// `(a * b) >> 24`, rounded to float precision.
fn product(a: i64, b: i64) -> i64 {
    let shifted = a.wrapping_mul(b) >> 24;
    requantize(shifted)
}

struct FixedBlur {
    size: usize,
    radius: i32,
    fraction: f32,
    fraction_q: i64,
    complement_q: i64,
}

impl FixedBlur {
    fn new(size: usize, radius: f32) -> FixedBlur {
        let whole = radius as i32;
        let fraction = radius - whole as f32;
        let fraction_q = truncate(fraction * ONE);
        let complement = (16_777_216_i64 - fraction_q) as f32;
        let complement_q = truncate(complement * INVERSE_ONE * ONE);
        FixedBlur {
            size,
            radius: whole,
            fraction,
            fraction_q,
            complement_q,
        }
    }

    /// Blurs row `row` of `source`; result `i` is
    /// column `row`, row `i` of the output (returned in order).
    fn row(&self, source: &[f32], row: usize) -> Vec<f32> {
        let n = self.size as i32;
        let radius = self.radius;
        let base = (n * row as i32) as usize;
        let sample = |offset: i32| fixed(source[base + offset as usize]);

        let width = ((radius * 2) as f32 + self.fraction) + self.fraction;
        let norm = (1.0 / (width as f64 + 1.0)) as f32;
        let norm_q = truncate(norm * ONE);
        let first = sample(0);
        let lead = truncate((radius as f32 + 1.0) * ONE);
        let lead = requantize(lead + self.fraction_q);
        let mut sum = product(lead, first);

        let mut even = 0i64;
        let mut odd = 0i64;
        let radius_wide = radius as i64;
        if radius_wide >= 2 {
            let pairs = ((radius_wide - 2) as u64 >> 1) + 1;
            for pair in 0..pairs as i32 {
                even += sample(2 * pair);
                odd += sample(2 * pair + 1);
            }
            let done = 2 * pairs as i64;
            if done < radius_wide {
                sum += sample(done as i32);
            }
        } else if radius_wide == 1 {
            sum += sample(0);
        }

        let entering = sample(radius);
        let partial = product(self.fraction_q, entering);
        sum += partial + odd + even;

        let output = |sum: i64| (product(norm_q, sum) as f32) * INVERSE_ONE;
        let mut result = Vec::with_capacity(self.size);
        let mut next = radius + 1;
        let mut previous_enter = entering;

        for _ in 0..(radius + 1).max(0) {
            let carried = previous_enter.wrapping_mul(self.complement_q) >> 24;
            let enter = sample(next);
            next += 1;
            let partial = product(enter, self.fraction_q);
            let carried = requantize(carried);
            sum += (partial + carried) - first;
            result.push(output(sum));
            previous_enter = enter;
        }

        let mut previous_leave = first;
        let mut leave_index = 0;
        let middle_end = n - radius - 1;
        let mut position = radius + 1;
        while position < middle_end {
            let leave_partial = previous_leave.wrapping_mul(self.fraction_q) >> 24;
            let leave = sample(leave_index);
            leave_index += 1;
            let carried = self.complement_q.wrapping_mul(previous_enter) >> 24;
            let enter = sample(next);
            next += 1;
            let leaving = product(self.complement_q, leave);
            let entering = product(self.fraction_q, enter);
            let mut delta = entering - leaving;
            delta -= requantize(leave_partial);
            sum += requantize(carried);
            sum += delta;
            result.push(output(sum));
            previous_leave = leave;
            previous_enter = enter;
            position += 1;
        }

        let last = sample(n - 1);
        let mut position = n - radius - 1;
        while position < n {
            let edge = last.wrapping_mul(self.fraction_q) >> 24;
            let leave_partial = self.fraction_q.wrapping_mul(previous_leave) >> 24;
            let carried = self.complement_q.wrapping_mul(previous_enter) >> 24;
            previous_enter = last;
            let leave = sample(leave_index);
            leave_index += 1;
            let leaving = product(self.complement_q, leave);
            let mut delta = requantize(edge) - leaving;
            delta -= requantize(leave_partial);
            sum += requantize(carried);
            sum += delta;
            result.push(output(sum));
            previous_leave = leave;
            position += 1;
        }
        result
    }

    /// One pass over every row, written transposed.
    fn pass(&self, source: &[f32]) -> Vec<f32> {
        let n = self.size;
        let rows: Vec<Vec<f32>> = (0..n).into_par_iter().map(|row| self.row(source, row)).collect();
        let mut transposed = vec![0.0; n * n];
        for (row, values) in rows.iter().enumerate() {
            for (index, value) in values.iter().take(n).enumerate() {
                transposed[index * n + row] = *value;
            }
        }
        transposed
    }
}

/// Three horizontal + vertical box passes of `radius`
/// pixels. The copy stops one value short of the map, so the last pixel
/// starts out as the fresh allocation's zero.
pub fn box_blur(data: &[f32], size: usize, radius: f32) -> Vec<f32> {
    let blur = FixedBlur::new(size, radius);
    let mut current = data.to_vec();
    if let Some(last) = current.last_mut() {
        *last = 0.0;
    }
    for _ in 0..3 {
        let across = blur.pass(&current);
        current = blur.pass(&across);
    }
    current
}
