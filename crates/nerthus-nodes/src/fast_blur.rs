//! The native blur (`FastBlur4_Process`, ISPC): the Q28 fixed-point
//! running box blur of [`crate::blur::fixed_box_blur`], run four times in
//! each direction. The managed code reaches it through [`blur_map`] (and
//! [`blur_relative`]), the blur most nodes use.

use nerthus_engine::Map;
use rayon::prelude::*;

use crate::x86;

const FIXED_SHIFT: u32 = 28;
const FIXED_ONE: i64 = 1 << FIXED_SHIFT;
const FIXED_SCALE: f64 = FIXED_ONE as f64;
const FIXED_UNIT: f64 = 1.0 / FIXED_SCALE;

/// Rows each native task blurs; rows past the last whole block are left
/// alone.
const ROWS_PER_TASK: usize = 16;

/// Settings of one blur pass, derived from the radius like the ISPC kernel
/// does.
struct PassSettings {
    whole: usize,
    fraction: i64,
    complement: i64,
    leading_weight: i64,
    norm: f64,
}

impl PassSettings {
    fn new(radius: f32) -> Self {
        let whole = radius as i32;
        let fraction = radius - whole as f32;
        let mut denominator = fraction + (whole * 2) as f32;
        denominator += fraction;
        denominator += 1.0;
        let norm = x86::rcp_refined(denominator) as f64;

        let fraction_fixed = (fraction as f64 * FIXED_SCALE) as i64;
        let whole_fixed = (whole as f32 as f64 * FIXED_SCALE) as i64;
        Self {
            whole: whole.max(0) as usize,
            fraction: fraction_fixed,
            complement: FIXED_ONE - fraction_fixed,
            leading_weight: whole_fixed + FIXED_ONE + fraction_fixed,
            norm,
        }
    }
}

fn to_fixed(value: f32) -> i64 {
    (value as f64 * FIXED_SCALE) as i64
}

fn fixed_multiply(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) >> FIXED_SHIFT
}

fn from_fixed(sum: i64, norm: f64) -> f32 {
    (norm * sum as f64 * FIXED_UNIT) as f32
}

/// Blurs one row of `length` values into `output` with the running sum.
fn blur_row(row: &[f32], output: &mut [f32], settings: &PassSettings) {
    let length = row.len();
    let whole = settings.whole;
    let fraction = settings.fraction;
    let complement = settings.complement;
    let norm = settings.norm;

    let mut leaving = to_fixed(row[0]);
    let mut sum = fixed_multiply(settings.leading_weight, leaving);
    for &value in &row[..whole] {
        sum += to_fixed(value);
    }
    let mut entering = to_fixed(row[whole]);
    sum += fixed_multiply(entering, fraction);

    let mut out = 0;
    let mut right = whole + 1;
    for _ in 0..whole + 1 {
        sum -= leaving;
        sum += fixed_multiply(entering, complement);
        entering = to_fixed(row[right]);
        right += 1;
        sum += fixed_multiply(entering, fraction);
        output[out] = from_fixed(sum, norm);
        out += 1;
    }

    let mut left = 0;
    for _ in whole + 1..length - whole - 1 {
        sum -= fixed_multiply(leaving, fraction);
        leaving = to_fixed(row[left]);
        left += 1;
        sum -= fixed_multiply(leaving, complement);
        sum += fixed_multiply(entering, complement);
        entering = to_fixed(row[right]);
        right += 1;
        sum += fixed_multiply(entering, fraction);
        output[out] = from_fixed(sum, norm);
        out += 1;
    }

    let last = to_fixed(row[length - 1]);
    for _ in length - whole - 1..length {
        sum -= fixed_multiply(leaving, fraction);
        leaving = to_fixed(row[left]);
        left += 1;
        sum -= fixed_multiply(leaving, complement);
        sum += fixed_multiply(entering, complement);
        entering = last;
        sum += fixed_multiply(entering, fraction);
        output[out] = from_fixed(sum, norm);
        out += 1;
    }
}

/// One pass: blurs each of `count` rows of `length` values in `source` and
/// writes the results transposed (row `r` becomes column `r` of `target`,
/// which has `length` rows of `count` values).
fn blur_pass(source: &[f32], target: &mut [f32], length: usize, count: usize, settings: &PassSettings) {
    let blurred_rows = (count / ROWS_PER_TASK) * ROWS_PER_TASK;
    let blurred: Vec<Vec<f32>> = source
        .par_chunks(length)
        .take(blurred_rows)
        .map(|row| {
            let mut output = vec![0.0f32; length];
            blur_row(row, &mut output, settings);
            output
        })
        .collect();
    target.par_chunks_mut(count).enumerate().for_each(|(position, column_values)| {
        for (row, values) in blurred.iter().enumerate() {
            column_values[row] = values[position];
        }
    });
}

/// `FastBlur4_Process`: blurs a `width` × `height` plane in place.
pub fn fast_blur4(data: &mut [f32], width: usize, height: usize, radius: f32) {
    let mut scaled = radius * 0.35;
    let width_limit = width as f32 * 0.5 - 1.0;
    if width_limit < scaled {
        scaled = width_limit;
    }
    let height_limit = height as f32 * 0.5 - 1.0;
    if height_limit < scaled {
        scaled = height_limit;
    }

    // Above 32 pixels the kernel takes another code path that works in
    // sixteenths of a pixel, flooring the radius to them.
    if scaled > 32.0 {
        let sixteenths = (scaled * 16.0) as i32;
        scaled = sixteenths as f32 / 16.0;
    }
    let settings = PassSettings::new(scaled);
    let mut buffer = vec![0.0f32; width * height];
    for _ in 0..4 {
        blur_pass(data, &mut buffer, width, height, &settings);
        blur_pass(&buffer, data, height, width, &settings);
    }
}

/// [`fast_blur4`] on every channel of a copy.
pub fn blur_map(map: &Map, radius: f32) -> Map {
    let width = map.width();
    let height = map.height();
    let mut blurred = map.clone();
    for channel in 0..blurred.channel_count() {
        fast_blur4(blurred.channel_mut(channel), width, height, radius);
    }
    blurred
}

/// [`blur_map`] with the radius given per 40th of the map width.
pub fn blur_relative(map: &Map, radius: f32) -> Map {
    let pixels = radius / 40.0 * map.width() as f32;
    blur_map(map, pixels)
}

/// The fixed-point number's conversion back to float: `(float)(sum / 2²⁸ × norm)`.
fn managed_from_fixed(sum: i64, norm: f64) -> f32 {
    (sum as f64 / FIXED_SCALE * norm) as f32
}

/// One row of [`managed_box_blur`]: a running
/// box sum in Q28 over `±(whole + fraction)` with clamped edges. The
/// results go into column `row` of the target.
fn managed_row(source: &[f32], size: usize, row: usize, whole: usize, fraction: i64, norm: f64) -> Vec<f32> {
    let complement = FIXED_ONE - fraction;
    let start = row * size;
    let mut first = to_fixed(source[start]);
    let lead = ((whole as i64 + 1) << FIXED_SHIFT) + fraction;
    let mut sum = fixed_multiply(lead, first);
    for index in 0..whole {
        sum += to_fixed(source[start + index]);
    }
    let mut last = to_fixed(source[start + whole]);
    sum += fixed_multiply(last, fraction);

    let mut column = Vec::with_capacity(size);
    let mut ahead = start + whole + 1;
    for _ in 0..whole + 1 {
        sum -= first;
        sum += fixed_multiply(last, complement);
        last = to_fixed(source[ahead]);
        ahead += 1;
        sum += fixed_multiply(last, fraction);
        column.push(managed_from_fixed(sum, norm));
    }
    let mut behind = start;
    for _ in whole + 1..size - whole - 1 {
        sum -= fixed_multiply(first, fraction);
        first = to_fixed(source[behind]);
        behind += 1;
        sum -= fixed_multiply(first, complement);
        sum += fixed_multiply(last, complement);
        last = to_fixed(source[ahead]);
        ahead += 1;
        sum += fixed_multiply(last, fraction);
        column.push(managed_from_fixed(sum, norm));
    }
    let edge = to_fixed(source[start + size - 1]);
    for _ in size - whole - 1..size {
        sum -= fixed_multiply(first, fraction);
        first = to_fixed(source[behind]);
        behind += 1;
        sum -= fixed_multiply(first, complement);
        sum += fixed_multiply(last, complement);
        last = edge;
        sum += fixed_multiply(last, fraction);
        column.push(managed_from_fixed(sum, norm));
    }
    column
}

/// The managed Q28 box blur of a square plane,
/// `iterations` rounds of two passes (each written transposed) with
/// radius `whole + fraction`.
pub fn managed_box_blur(data: &[f32], size: usize, radius: f32, iterations: i32) -> Vec<f32> {
    let whole = radius as i32 as usize;
    let fraction_value = radius - whole as f32;
    let fraction = to_fixed(fraction_value);
    let width = (whole + whole) as f32 + fraction_value + fraction_value;
    let norm = 1.0 / (width as f64 + 1.0);
    let mut current = data.to_vec();
    for _ in 0..iterations {
        for _ in 0..2 {
            let columns: Vec<Vec<f32>> = (0..size)
                .into_par_iter()
                .map(|row| managed_row(&current, size, row, whole, fraction, norm))
                .collect();
            let mut target = vec![0.0f32; size * size];
            for (row, column) in columns.into_iter().enumerate() {
                for (position, value) in column.into_iter().enumerate() {
                    target[position * size + row] = value;
                }
            }
            current = target;
        }
    }
    current
}
