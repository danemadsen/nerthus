//! The map transformer (`Transformer`):
//! resizing, rotation, cropping and pasting one map into another.

use std::f32::consts::PI;

use nerthus_engine::{Map, Resolution};
use rayon::prelude::*;

use crate::maths::{self, dotnet_max, dotnet_min};

/// `EdgeBehaviour`: how bilinear samples past the border are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeBehaviour {
    Edge,
    Mirror,
}

impl EdgeBehaviour {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Mirror" => Self::Mirror,
            _ => Self::Edge,
        }
    }
}

/// `Transformer.ResizeMethod`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeMethod {
    LinearFast,
    HighQuality,
    OverSmooth,
}

/// `Transformer.BlendMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendMode {
    None,
    Blend,
    Add,
    Subtract,
    Difference,
    Multiply,
    Screen,
    Max,
    Min,
}

impl BlendMode {
    /// The member named `name` (`BombBlendMode` shares these names).
    pub fn from_name(name: &str) -> Self {
        match name {
            "None" => Self::None,
            "Add" => Self::Add,
            "Subtract" => Self::Subtract,
            "Difference" => Self::Difference,
            "Multiply" => Self::Multiply,
            "Screen" => Self::Screen,
            "Max" => Self::Max,
            "Min" => Self::Min,
            _ => Self::Blend,
        }
    }

    /// The combining function of every mode but `None` and `Blend`.
    fn function(self) -> Option<fn(f32, f32) -> f32> {
        let function: fn(f32, f32) -> f32 = match self {
            Self::Add => |a, b| a + b,
            Self::Subtract => |a, b| a - b,
            Self::Difference => |a, b| (a - b).abs(),
            Self::Multiply => |a, b| a * b,
            Self::Screen => |a, b| 1.0 - (1.0 - a) * (1.0 - b),
            Self::Max => |a, b| if a > b { a } else { b },
            Self::Min => |a, b| if a < b { a } else { b },
            Self::None | Self::Blend => return None,
        };
        Some(function)
    }
}

/// `Transformer.TransformQuality`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransformQuality {
    Draft,
    Medium,
    High,
}

/// `Transformer.AnchorPosition`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorPosition {
    TopLeft,
    Center,
}

/// Samples of the resampling kernels per unit of their support.
const KERNEL_STEPS: usize = 64;

// --- sampling -------------------------------------------------------------------

/// A borrowed channel of a map.
#[derive(Clone, Copy)]
struct Plane<'a> {
    data: &'a [f32],
    width: usize,
    height: usize,
}

impl<'a> Plane<'a> {
    fn of(map: &'a Map, channel: usize) -> Self {
        Plane {
            data: map.channel(channel),
            width: map.width(),
            height: map.height(),
        }
    }

    fn get(&self, x: usize, y: usize) -> f32 {
        self.data[y * self.width + x]
    }

    /// `Channel.InterpolateBilinear(position)`: pixel centres at +0.5, the
    /// cell index truncated toward zero, then clamped (so the fraction may
    /// extrapolate at the top and left borders).
    fn bilinear(&self, position_x: f32, position_y: f32) -> f32 {
        let x = position_x - 0.5;
        let y = position_y - 0.5;
        let mut left = x as i32;
        let mut top = y as i32;
        let fraction_x = x - left as f32;
        let fraction_y = y - top as f32;
        left = left.max(0).min(self.width as i32 - 1);
        top = top.max(0).min(self.height as i32 - 1);
        let right = (left + 1).min(self.width as i32 - 1) as usize;
        let bottom = (top + 1).min(self.height as i32 - 1) as usize;
        let left = left as usize;
        let top = top as usize;
        let upper = self.get(left, top) * (1.0 - fraction_x) + self.get(right, top) * fraction_x;
        let lower = self.get(left, bottom) * (1.0 - fraction_x) + self.get(right, bottom) * fraction_x;
        upper * (1.0 - fraction_y) + lower * fraction_y
    }

    /// `Channel.InterpolateBilinear(position, edgeBehaviour)`.
    fn bilinear_with_edges(&self, position_x: f32, position_y: f32, edges: EdgeBehaviour) -> f32 {
        let x = position_x - 0.5;
        let y = position_y - 0.5;
        let mut left = x as i32;
        let mut top = y as i32;
        let mut right = left + 1;
        let mut bottom = top + 1;
        let fraction_x = x - left as f32;
        let fraction_y = y - top as f32;
        let width = self.width as i32;
        let height = self.height as i32;
        if edges == EdgeBehaviour::Edge {
            left = left.max(0).min(width - 1);
            top = top.max(0).min(height - 1);
            right = right.max(0).min(width - 1);
            bottom = bottom.max(0).min(height - 1);
        } else {
            left = mirror_index(left, width);
            right = mirror_index(right, width);
            top = mirror_index(top, height);
            bottom = mirror_index(bottom, height);
        }
        let get = |x: i32, y: i32| self.get(x as usize, y as usize);
        let upper = get(left, top) * (1.0 - fraction_x) + get(right, top) * fraction_x;
        let lower = get(left, bottom) * (1.0 - fraction_x) + get(right, bottom) * fraction_x;
        upper * (1.0 - fraction_y) + lower * fraction_y
    }
}

fn mirror_index(index: i32, length: i32) -> i32 {
    if index < 0 {
        return -index;
    }
    if index < length { index } else { length - 1 + length - index }
}

/// Builds a map of `resolution` channel by channel.
fn map_channels(source: &Map, resolution: Resolution, mut build: impl FnMut(Plane, usize) -> Vec<f32>) -> Map {
    let mut channels = Vec::with_capacity(source.channel_count());
    for channel in 0..source.channel_count() {
        let plane = Plane::of(source, channel);
        channels.push(build(plane, channel));
    }
    Map::from_channels(resolution, channels)
}

// --- resizing ---------------------------------------------------------------------

/// `Transformer.Resize(map, resolution)`: the high-quality method.
pub fn resize(map: &Map, resolution: Resolution) -> Map {
    resize_with(map, resolution, ResizeMethod::HighQuality)
}

/// `Transformer.Resize(map, resolution, method)`.
pub fn resize_with(map: &Map, resolution: Resolution, method: ResizeMethod) -> Map {
    match method {
        ResizeMethod::LinearFast => resize_linear(map, resolution),
        ResizeMethod::HighQuality => resize_high_quality(map, resolution, true),
        ResizeMethod::OverSmooth => resize_oversmooth(map, resolution),
    }
}

/// Bilinear sampling at `i · old / new`.
fn resize_linear(map: &Map, resolution: Resolution) -> Map {
    let ratio_x = map.width() as f32 / resolution.x as f32;
    let ratio_y = map.height() as f32 / resolution.y as f32;
    map_channels(map, resolution, |plane, _| {
        let mut output = vec![0.0f32; resolution.area()];
        output.par_chunks_mut(resolution.x).enumerate().for_each(|(row, values)| {
            let y = row as f32 * ratio_y;
            for (column, value) in values.iter_mut().enumerate() {
                *value = plane.bilinear(column as f32 * ratio_x, y);
            }
        });
        output
    })
}

/// The Mitchell–Netravali cubic with parameters `b` and `c`.
fn mitchell(x: f32, b: f32, c: f32) -> f32 {
    if x < 1.0 {
        let cubic = (12.0 - 9.0 * b - 6.0 * c) * x * x * x;
        let quadratic = (-18.0 + 12.0 * b + 6.0 * c) * x * x;
        let constant = 6.0 - 2.0 * b;
        return (1.0 / 6.0) * (cubic + quadratic + constant);
    }
    let cubic = (0.0 - b - 6.0 * c) * x * x * x;
    let quadratic = (6.0 * b + 30.0 * c) * x * x;
    let linear = (-12.0 * b - 48.0 * c) * x;
    let constant = 8.0 * b + 24.0 * c;
    (1.0 / 6.0) * (cubic + quadratic + linear + constant)
}

/// A Mitchell kernel over `0..2`, sampled at `KERNEL_STEPS + 1`
/// points (one spare zero at the end).
fn mitchell_table(b: f32, c: f32) -> Vec<f32> {
    let mut table = vec![0.0f32; KERNEL_STEPS + 2];
    for (index, value) in table.iter_mut().take(KERNEL_STEPS + 1).enumerate() {
        let x = 2.0 * index as f32 / KERNEL_STEPS as f32;
        *value = mitchell(x, b, c);
    }
    table
}

/// One sample of a kernel resample along a row, the kernel
/// stretched by `1 / scale` (scale ≤ 1 when shrinking).
fn kernel_sample(plane: Plane, center: f32, row: usize, scale: f32, table: &[f32], radius: f32) -> f32 {
    let support = 1.0 / scale * radius;
    let normalized_scale = scale / radius;
    let first = ((center - support + 1.0) as i32).max(0);
    let last = ((center + support) as i32).min(plane.width as i32 - 1);
    let mut weight_sum = 0.0f32;
    let mut sum = 0.0f32;
    for index in first..=last {
        let distance = dotnet_min((center - index as f32).abs() * normalized_scale, 1.0);
        let position = distance * KERNEL_STEPS as f32;
        let whole = position as usize;
        let fraction = position - whole as f32;
        let weight = table[whole] * (1.0 - fraction) + table[whole + 1] * fraction;
        weight_sum += weight;
        sum += plane.get(index as usize, row) * weight;
    }
    sum / weight_sum
}

/// Resamples every row to `length` values and writes them
/// transposed (the result is `height` wide and `length` tall).
fn kernel_resample_transposed(map: &Map, length: usize, table: &[f32], radius: f32) -> Map {
    let scale = length as f32 / map.width() as f32;
    let step = 1.0 / scale;
    let clamped_scale = if scale > 1.0 { 1.0 } else { scale };
    let height = map.height();
    let resolution = Resolution { x: height, y: length };
    map_channels(map, resolution, |plane, _| {
        let mut output = vec![0.0f32; resolution.area()];
        output.par_chunks_mut(height).enumerate().for_each(|(position, values)| {
            let center = position as f32 * step;
            for (row, value) in values.iter_mut().enumerate() {
                *value = kernel_sample(plane, center, row, clamped_scale, table, radius);
            }
        });
        output
    })
}

/// Two cubic B-spline resamples (blurrier than the default).
fn resize_oversmooth(map: &Map, resolution: Resolution) -> Map {
    let table = mitchell_table(1.0, 0.0);
    let across = kernel_resample_transposed(map, resolution.x, &table, 2.0);
    kernel_resample_transposed(&across, resolution.y, &table, 2.0)
}

/// The integer base-2 logarithm.
fn integer_log2(mut value: usize) -> i32 {
    let mut log = 0;
    loop {
        value >>= 1;
        if value == 0 {
            return log;
        }
        log += 1;
    }
}

/// How many doublings (positive) or halvings (negative) lead from
/// `from` towards `to`; doublings overshoot a non-power-of-two ratio.
fn doubling_steps(from: usize, to: usize) -> i32 {
    if to == from {
        return 0;
    }
    if to > from {
        let steps = integer_log2(to / from);
        let overshoot = if from << steps != to { 1 } else { 0 };
        return steps + overshoot;
    }
    -integer_log2(from / to)
}

/// Halves both axes with 2×2 averages.
fn halve_both(map: &Map) -> Map {
    let resolution = Resolution { x: map.width() >> 1, y: map.height() >> 1 };
    map_channels(map, resolution, |plane, _| {
        let mut output = vec![0.0f32; resolution.area()];
        output.par_chunks_mut(resolution.x).enumerate().for_each(|(row, values)| {
            let source_row = row << 1;
            for (column, value) in values.iter_mut().enumerate() {
                let source_column = column << 1;
                let sum = plane.get(source_column, source_row)
                    + plane.get(source_column + 1, source_row)
                    + plane.get(source_column, source_row + 1)
                    + plane.get(source_column + 1, source_row + 1);
                *value = 0.25 * sum;
            }
        });
        output
    })
}

/// Halves the width with pair averages.
fn halve_x(map: &Map) -> Map {
    let resolution = Resolution { x: map.width() >> 1, y: map.height() };
    map_channels(map, resolution, |plane, _| {
        let mut output = vec![0.0f32; resolution.area()];
        output.par_chunks_mut(resolution.x).enumerate().for_each(|(row, values)| {
            for (column, value) in values.iter_mut().enumerate() {
                let source_column = column << 1;
                *value = 0.5 * (plane.get(source_column, row) + plane.get(source_column + 1, row));
            }
        });
        output
    })
}

/// Halves the height with pair averages.
fn halve_y(map: &Map) -> Map {
    let resolution = Resolution { x: map.width(), y: map.height() >> 1 };
    map_channels(map, resolution, |plane, _| {
        let mut output = vec![0.0f32; resolution.area()];
        output.par_chunks_mut(resolution.x).enumerate().for_each(|(row, values)| {
            let source_row = row << 1;
            for (column, value) in values.iter_mut().enumerate() {
                *value = 0.5 * (plane.get(column, source_row) + plane.get(column, source_row + 1));
            }
        });
        output
    })
}

/// The slope at `b` from its neighbours, each side limited to
/// three times the other.
fn limited_slope(a: f32, b: f32, c: f32) -> f32 {
    let mut before = b - a;
    let mut after = c - b;
    let before_sign = if before <= 0.0 { -1.0 } else { 1.0 };
    let after_sign = if after <= 0.0 { -1.0 } else { 1.0 };
    let mut before_size = before * before_sign;
    let mut after_size = after * after_sign;
    if before_size > after_size {
        before_size = dotnet_min(after_size * 3.0, before_size);
        before = before_size * before_sign;
    } else {
        after_size = dotnet_min(before_size * 3.0, after_size);
        after = after_size * after_sign;
    }
    0.5 * (before + after)
}

/// The doubling of one row shared by [`double_x_transposed`] and
/// [`double_x`]: each value is
/// followed by a slope-limited midpoint kept between its neighbours.
fn double_row(row: &[f32], third: f32, mut write: impl FnMut(usize, f32)) {
    let width = row.len();
    let first = row[0];
    let mut current = row[0];
    let mut next = row[1.min(width - 1)];
    let mut after = row[2.min(width - 1)];
    let mut slope = limited_slope(first, current, next);
    for index in 0..width {
        let mut previous_slope = slope;
        slope = limited_slope(current, next, after);
        if previous_slope == 0.0 {
            slope *= 1.5;
        }
        if slope == 0.0 {
            previous_slope *= 1.5;
        }
        let mut middle = (current + previous_slope * third + next - slope * third) * 0.5;
        middle = dotnet_min(middle, dotnet_max(current, next));
        middle = dotnet_max(middle, dotnet_min(current, next));
        write(index << 1, current);
        write((index << 1) + 1, middle);
        let following = (index + 3).min(width - 1);
        current = next;
        next = after;
        after = row[following];
    }
}

/// Doubles the width and writes transposed (`height` wide,
/// `2·width` tall).
fn double_x_transposed(map: &Map) -> Map {
    let width = map.width();
    let height = map.height();
    let resolution = Resolution { x: height, y: width * 2 };
    map_channels(map, resolution, |plane, _| {
        let rows: Vec<Vec<f32>> = plane
            .data
            .par_chunks(width)
            .map(|row| {
                let mut doubled = vec![0.0f32; width * 2];
                double_row(row, 1.0 / 3.0, |index, value| doubled[index] = value);
                doubled
            })
            .collect();
        let mut output = vec![0.0f32; resolution.area()];
        for (row, doubled) in rows.iter().enumerate() {
            for (index, &value) in doubled.iter().enumerate() {
                output[index * height + row] = value;
            }
        }
        output
    })
}

/// Doubles the width.
fn double_x(map: &Map) -> Map {
    let width = map.width();
    let resolution = Resolution { x: width * 2, y: map.height() };
    map_channels(map, resolution, |plane, _| {
        let mut output = vec![0.0f32; resolution.area()];
        output.par_chunks_mut(width * 2).zip(plane.data.par_chunks(width)).for_each(|(doubled, row)| {
            double_row(row, 0.3333, |index, value| doubled[index] = value);
        });
        output
    })
}

/// Transposes.
fn transpose(map: &Map) -> Map {
    let width = map.width();
    let height = map.height();
    let resolution = Resolution { x: height, y: width };
    map_channels(map, resolution, |plane, _| {
        let mut output = vec![0.0f32; resolution.area()];
        output.par_chunks_mut(height).enumerate().for_each(|(column, values)| {
            for (row, value) in values.iter_mut().enumerate() {
                *value = plane.get(column, row);
            }
        });
        output
    })
}

/// The 3×3 neighbourhood rows and columns [`extrema`] and
/// [`restore_extrema_smooth`] read, with
/// their `else if` clamping (only one side can need it).
fn neighbour_indices(index: usize, length: usize) -> (usize, usize) {
    let mut before = index as i32 - 1;
    let mut after = index as i32 + 1;
    if before < 0 {
        before = 0;
    } else if after > length as i32 - 1 {
        after = length as i32 - 1;
    }
    (before as usize, after as usize)
}

/// Each value that is a strict local extremum of its 3×3
/// neighbourhood, NaN elsewhere (first channel only: callers read only it).
fn extrema(map: &Map) -> Plane2 {
    let plane = Plane::of(map, 0);
    let width = plane.width;
    let height = plane.height;
    let mut output = vec![0.0f32; width * height];
    output.par_chunks_mut(width).enumerate().for_each(|(row, values)| {
        let (above, below) = neighbour_indices(row, height);
        for (column, value) in values.iter_mut().enumerate() {
            let (left, right) = neighbour_indices(column, width);
            let neighbours = [
                plane.get(left, below),
                plane.get(column, below),
                plane.get(right, below),
                plane.get(left, row),
                plane.get(right, row),
                plane.get(left, above),
                plane.get(column, above),
                plane.get(right, above),
            ];
            let [below_left, below_middle, below_right, middle_left, middle_right, above_left, above_middle, above_right] = neighbours;
            let lowest = dotnet_min(
                dotnet_min(dotnet_min(below_left, below_middle), dotnet_min(below_right, middle_left)),
                dotnet_min(dotnet_min(middle_right, above_left), dotnet_min(above_middle, above_right)),
            );
            let highest = dotnet_max(
                dotnet_max(dotnet_max(below_left, below_middle), dotnet_max(below_right, middle_left)),
                dotnet_max(dotnet_max(middle_right, above_left), dotnet_max(above_middle, above_right)),
            );
            let centre = plane.get(column, row);
            *value = if centre > highest || centre < lowest { centre } else { f32::NAN };
        }
    });
    Plane2 { data: output, width, height }
}

/// An owned single plane.
struct Plane2 {
    data: Vec<f32>,
    width: usize,
    height: usize,
}

impl Plane2 {
    fn get(&self, x: usize, y: usize) -> f32 {
        self.data[y * self.width + x]
    }
}

/// The blend weight of a restored extremum by position within its block
/// of `factor` doubled pixels (`switch`).
fn extremum_weight(position: usize, factor: usize) -> f32 {
    match position.min(factor - position) {
        0 => 1.0,
        1 => 0.6,
        _ => 0.0,
    }
}

/// Smooths a doubled map with a 3×3 binomial filter, pulling the
/// pixels nearest each original extremum back to it.
fn restore_extrema_smooth(map: &Map, extrema: &Plane2) -> Map {
    let width = map.width();
    let height = map.height();
    let factor_x = width / extrema.width;
    let factor_y = height / extrema.height;
    map_channels(map, map.resolution(), |plane, _| {
        let mut output = vec![0.0f32; width * height];
        output.par_chunks_mut(width).enumerate().for_each(|(row, values)| {
            let block_row = row & (factor_y - 1);
            let row_weight = extremum_weight(block_row, factor_y);
            let extremum_row = ((row + (factor_y >> 1)) / factor_y).min(extrema.height - 1);
            let (above, below) = neighbour_indices(row, height);
            for (column, value) in values.iter_mut().enumerate() {
                let (left, right) = neighbour_indices(column, width);
                let above_left = plane.get(left, above);
                let above_middle = plane.get(column, above);
                let above_right = plane.get(right, above);
                let middle_left = plane.get(left, row);
                let middle_right = plane.get(right, row);
                let below_left = plane.get(left, below);
                let below_middle = plane.get(column, below);
                let below_right = plane.get(right, below);
                let centre = plane.get(column, row);

                let extremum_column = ((column + (factor_x >> 1)) / factor_x).min(extrema.width - 1);
                let mut extremum = extrema.get(extremum_column, extremum_row);
                let weight;
                if extremum.is_nan() {
                    weight = 0.0;
                    extremum = 0.0;
                } else {
                    let block_column = column & (factor_x - 1);
                    weight = extremum_weight(block_column, factor_x) * row_weight;
                }
                let corners = below_left + below_right + above_left + above_right;
                let edges = below_middle + middle_left + middle_right + above_middle;
                let mut smoothed = 0.0625 * corners + 0.125 * edges + 0.25 * centre;
                smoothed = weight * extremum + (1.0 - weight) * smoothed;
                *value = smoothed;
            }
        });
        output
    })
}

/// Never transposing: after the second doubling of an axis,
/// lightly smooths each row and snaps the pixels at original extrema back to
/// them.
fn restore_extrema_rows(map: Map, extrema: &Plane2) -> Map {
    let factor = map.width() / extrema.width;
    if factor <= 2 {
        return map;
    }
    let width = map.width();
    let height = map.height();
    map_channels(&map, map.resolution(), |plane, _| {
        let mut output = vec![0.0f32; width * height];
        output.par_chunks_mut(width).enumerate().for_each(|(row, values)| {
            let mut held = 0.0f32;
            let mut age = -10i32;
            let mut current = plane.get(0, row);
            let mut next = plane.get(1.min(width - 1), row);
            let mut after = plane.get(2.min(width - 1), row);
            let mut smoothed = 0.75 * current + 0.25 * next;
            let mut smoothed_next = 0.5 * next + 0.25 * (current + after);
            let extremum_row = row * extrema.height / height;
            let first_column = ((factor >> 1) / factor).min(extrema.width - 1);
            let first = extrema.get(first_column, extremum_row);
            if !first.is_nan() {
                held = first;
                age = 0;
            }
            for (column, value) in values.iter_mut().enumerate() {
                let extremum_column = ((column + (factor >> 1)) / factor).min(extrema.width - 1);
                let extremum = extrema.get(extremum_column, extremum_row);
                if !extremum.is_nan() {
                    held = extremum;
                    age = 1;
                }
                *value = if age < -1 {
                    smoothed
                } else if age != 0 {
                    (current + smoothed) * 0.5
                } else {
                    held
                };
                current = next;
                next = after;
                after = plane.get((column + 3).min(width - 1), row);
                smoothed = smoothed_next;
                smoothed_next = 0.5 * next + 0.25 * (current + after);
                age -= 1;
            }
        });
        output
    })
}

/// Doubles both axes, averaging the two axis orders; after the
/// first doubling, re-smooths with the original extrema.
fn double_both(map: &Map, restore: bool, extrema: &Plane2) -> Map {
    let rows_first = double_x_transposed(&double_x_transposed(map));
    let transposed = transpose(map);
    let columns_first = transpose(&double_x_transposed(&double_x_transposed(&transposed)));
    let averaged = rows_first.zip_values(&columns_first, |a, b| (a + b) * 0.5);
    if restore {
        return restore_extrema_smooth(&averaged, extrema);
    }
    averaged
}

/// Doubles or halves the width `steps` times.
fn resize_axis_x(map: Map, steps: &mut i32) -> Map {
    let mut current = map;
    if *steps > 0 {
        let original_extrema = extrema(&current);
        while *steps > 0 {
            let doubled = double_x(&current);
            current = restore_extrema_rows(doubled, &original_extrema);
            *steps -= 1;
        }
        return current;
    }
    while *steps < 0 {
        current = halve_x(&current);
        *steps += 1;
    }
    current
}

/// Doubles or halves the height `steps` times.
fn resize_axis_y(map: Map, steps: &mut i32) -> Map {
    if *steps > 0 {
        let mut current = transpose(&map);
        let original_extrema = extrema(&current);
        while *steps > 0 {
            let doubled = double_x(&current);
            current = restore_extrema_rows(doubled, &original_extrema);
            *steps -= 1;
        }
        return transpose(&current);
    }
    let mut current = map;
    while *steps < 0 {
        current = halve_y(&current);
        *steps += 1;
    }
    current
}

/// Resizes by repeated doubling or halving, then (when `exact`)
/// bilinearly to the exact size.
fn resize_high_quality(map: &Map, target: Resolution, exact: bool) -> Map {
    if target == map.resolution() {
        return map.clone();
    }
    let mut steps_x = doubling_steps(map.width(), target.x);
    let mut steps_y = doubling_steps(map.height(), target.y);
    let mut current = map.clone();
    if steps_x > 0 && steps_y > 0 {
        let original_extrema = extrema(map);
        let both = steps_x.min(steps_y);
        for step in 0..both {
            current = double_both(&current, step > 0, &original_extrema);
            steps_x -= 1;
            steps_y -= 1;
        }
        current = resize_axis_y(current, &mut steps_y);
        current = resize_axis_x(current, &mut steps_x);
    } else if steps_x < 0 && steps_y < 0 {
        let both = (-steps_x).min(-steps_y);
        for _ in 0..both {
            current = halve_both(&current);
            steps_x += 1;
            steps_y += 1;
        }
        current = resize_axis_x(current, &mut steps_x);
        current = resize_axis_y(current, &mut steps_y);
    } else if steps_x < steps_y {
        current = resize_axis_x(current, &mut steps_x);
        current = resize_axis_y(current, &mut steps_y);
    } else {
        current = resize_axis_y(current, &mut steps_y);
        current = resize_axis_x(current, &mut steps_x);
    }
    if current.resolution() == target || !exact {
        return current;
    }
    resize_linear(&current, target)
}

// --- rotation and cropping -----------------------------------------------------------

/// `Transformer.Rotate`: rotates about the centre by `angle` degrees,
/// optionally growing the canvas to fit.
pub fn rotate(map: &Map, angle: f32, enlarge_canvas: bool, edges: EdgeBehaviour, clockwise: bool) -> Map {
    let source_center_x = map.width() as f32 * 0.5;
    let source_center_y = map.height() as f32 * 0.5;
    let signed_angle = if clockwise { -angle } else { angle };
    let radians = signed_angle * (PI / 180.0);
    let cosine = (radians as f64).cos() as f32;
    let sine = (radians as f64).sin() as f32;

    let mut resolution = map.resolution();
    let mut target_center_x = source_center_x;
    let mut target_center_y = source_center_y;
    if enlarge_canvas {
        let corners = [(0.0f32, 0.0f32), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)];
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;
        for (corner_x, corner_y) in corners {
            let x = corner_x * map.width() as f32 - source_center_x;
            let y = corner_y * map.height() as f32 - source_center_y;
            let rotated_x = x * cosine - y * sine + source_center_x;
            let rotated_y = x * sine + y * cosine + source_center_y;
            min_x = min_x.min(rotated_x as i32);
            min_y = min_y.min(rotated_y as i32);
            max_x = max_x.max(rotated_x.ceil() as i32);
            max_y = max_y.max(rotated_y.ceil() as i32);
        }
        resolution = Resolution {
            x: (max_x - min_x + 1) as usize,
            y: (max_y - min_y + 1) as usize,
        };
        target_center_x = resolution.x as f32 * 0.5;
        target_center_y = resolution.y as f32 * 0.5;
    }

    map_channels(map, resolution, |plane, _| {
        let mut output = vec![0.0f32; resolution.area()];
        output.par_chunks_mut(resolution.x).enumerate().for_each(|(row, values)| {
            for (column, value) in values.iter_mut().enumerate() {
                let x = column as f32 - target_center_x;
                let y = row as f32 - target_center_y;
                let source_x = x * cosine - y * sine + source_center_x;
                let source_y = x * sine + y * cosine + source_center_y;
                *value = plane.bilinear_with_edges(source_x, source_y, edges);
            }
        });
        output
    })
}

/// The `resolution` window of `map` starting at (`left`, `top`).
pub fn crop(map: &Map, left: usize, top: usize, resolution: Resolution) -> Map {
    map_channels(map, resolution, |plane, _| {
        let mut output = vec![0.0f32; resolution.area()];
        output.par_chunks_mut(resolution.x).enumerate().for_each(|(row, values)| {
            for (column, value) in values.iter_mut().enumerate() {
                *value = plane.get(column + left, row + top);
            }
        });
        output
    })
}

// --- pasting ---------------------------------------------------------------------------

/// The destination rectangle and source offset `Insert` works with.
struct InsertWindow {
    first_row: usize,
    last_row: usize,
    first_column: usize,
    last_column: usize,
    offset_x: i32,
    offset_y: i32,
}

fn insert_window(source: &Map, destination: &Map, position: (f32, f32), anchor: AnchorPosition) -> InsertWindow {
    let mut start_x = position.0;
    let mut start_y = position.1;
    if anchor == AnchorPosition::Center {
        start_x -= source.width() as f32 * 0.5;
        start_y -= source.height() as f32 * 0.5;
    }
    let end_x = start_x + source.width() as f32;
    let end_y = start_y + source.height() as f32;
    let clamp_x = |value: i32| value.max(0).min(destination.width() as i32 - 1);
    let clamp_y = |value: i32| value.max(0).min(destination.height() as i32 - 1);
    let first_row = clamp_y(start_y as i32);
    let last_row = clamp_y(end_y as i32 - 1);
    let first_column = clamp_x(start_x as i32);
    let last_column = clamp_x(end_x as i32 - 1);
    InsertWindow {
        first_row: first_row as usize,
        last_row: last_row as usize,
        first_column: first_column as usize,
        last_column: last_column as usize,
        offset_x: -first_column - (start_x as i32).min(0),
        offset_y: -first_row - (start_y as i32).min(0),
    }
}

/// `Transformer.Insert`: pastes `source` into `destination` at `position`
/// (0..1 of the destination), replacing (`Blend`, through `mask` when
/// given) or combining with `mode`.
pub fn insert(source: &Map, destination: &mut Map, position: (f32, f32), mode: BlendMode, mask: Option<&Map>, anchor: AnchorPosition) {
    let mask = if mode == BlendMode::None { None } else { mask };
    let function = mode.function();
    let scaled = (position.0 * destination.width() as f32, position.1 * destination.height() as f32);
    let channel_count = source.channel_count().max(destination.channel_count());
    let window = insert_window(source, destination, scaled, anchor);
    let mask_plane = mask.map(|mask| Plane::of(mask, 0));
    let destination_width = destination.width();
    for channel in 0..channel_count {
        let source_plane = Plane::of(source, channel.min(source.channel_count() - 1));
        let target_channel = channel.min(destination.channel_count() - 1);
        let target = destination.channel_mut(target_channel);
        for row in window.first_row..=window.last_row {
            let source_row = (row as i32 + window.offset_y) as usize;
            for column in window.first_column..=window.last_column {
                let source_column = (column as i32 + window.offset_x) as usize;
                let index = row * destination_width + column;
                let pasted = source_plane.get(source_column, source_row);
                let combined = match function {
                    Some(function) => function(target[index], pasted),
                    None => pasted,
                };
                target[index] = match mask_plane {
                    Some(mask) => {
                        let weight = mask.get(source_column, source_row);
                        target[index] * (1.0 - weight) + weight * combined
                    }
                    None => combined,
                };
            }
        }
    }
}

/// `Transformer.MultiTransform`: pastes `source` scaled by `scale`, rotated
/// by `rotation` degrees and multiplied by `multiplier` into `destination`
/// around `position` (0..1 of the destination).
#[allow(clippy::too_many_arguments)]
pub fn multi_transform(
    source: &Map,
    destination: &mut Map,
    position: (f32, f32),
    scale: (f32, f32),
    rotation: f32,
    multiplier: f32,
    mode: BlendMode,
    quality: TransformQuality,
    mask: Option<&Map>,
    clockwise: bool,
    anchor: AnchorPosition,
) {
    let mask = if mode == BlendMode::None { None } else { mask };
    let function = mode.function();

    let mut offset = (position.0 - 0.5, position.1 - 0.5);
    let scaled_size = (source.width() as f32 * scale.0, source.height() as f32 * scale.1);
    let mut placement = (position.0 * destination.width() as f32, position.1 * destination.height() as f32);
    if anchor == AnchorPosition::TopLeft {
        placement.0 += scaled_size.0 * 0.5;
        placement.1 += scaled_size.1 * 0.5;
    }
    let angle = if clockwise { -rotation } else { rotation };
    let mut sampling_scale = (source.width() as f32 / scaled_size.0, source.height() as f32 / scaled_size.1);
    let radians = angle * (PI / 180.0);
    let (sine, cosine) = maths::sin_cos(radians);
    let (inverse_sine, inverse_cosine) = maths::sin_cos(0.0 - radians);
    offset = (offset.0 * cosine - offset.1 * sine, offset.0 * sine + offset.1 * cosine);

    let mut source_map = source.clone();
    let mut mask_map = mask.cloned();
    let mut source_shift = (0.0f32, 0.0f32);
    if quality == TransformQuality::High {
        if scaled_size.0 > destination.width() as f32 && scaled_size.1 > destination.height() as f32 {
            let (diagonal_sine, diagonal_cosine) = maths::sin_cos(radians + PI / 4.0);
            let spread = 1.42 * dotnet_max(diagonal_cosine.abs(), diagonal_sine.abs()) * maths::sqrt(2.0);
            offset = (offset.0 / spread, offset.1 / spread);
            let reduction = |sampling: f32| 0.5 / maths::pow(2.0, maths::log2(1.0 / sampling / spread).floor());
            let reduction_x = reduction(sampling_scale.0);
            let reduction_y = reduction(sampling_scale.1);
            let window = (source.width() as f32 * reduction_x * 2.0, source.height() as f32 * reduction_y * 2.0);
            source_shift = (-offset.0 * source.width() as f32, -offset.1 * source.height() as f32);
            let shift_factor = if window.0 as i32 >= source.width() as i32 { (0.0, 0.0) } else { (reduction_x * 2.0, reduction_y * 2.0) };
            source_shift = (source_shift.0 * shift_factor.0, source_shift.1 * shift_factor.1);
            let mut center = (source.width() as f32 * 0.5 + source_shift.0, source.height() as f32 * 0.5 + source_shift.1);
            center.0 = dotnet_max(center.0, window.0 / 2.0);
            center.0 = dotnet_min(center.0, source.width() as f32 - window.0 / 2.0);
            center.1 = dotnet_max(center.1, window.1 / 2.0);
            center.1 = dotnet_min(center.1, source.height() as f32 - window.1 / 2.0);
            let left = 0.max((center.0 - window.0 / 2.0) as i32) as usize;
            let top = 0.max((center.1 - window.1 / 2.0) as i32) as usize;
            let right = (source.width() as i32).min((center.0 + window.0 / 2.0).ceil() as i32) as usize;
            let bottom = (source.height() as i32).min((center.1 + window.1 / 2.0).ceil() as i32) as usize;
            let window_resolution = Resolution { x: right - left, y: bottom - top };
            source_map = crop(&source_map, left, top, window_resolution);
            mask_map = mask_map.map(|mask| crop(&mask, left, top, window_resolution));
            let reduced = Resolution {
                x: (source_map.width() as f32 / reduction_x) as usize,
                y: (source_map.height() as f32 / reduction_y) as usize,
            };
            source_map = resize_high_quality(&source_map, reduced, true);
            // The mask is sized from the already resized source.
            let mask_target = Resolution {
                x: (source_map.width() as f32 / reduction_x) as usize,
                y: (source_map.height() as f32 / reduction_y) as usize,
            };
            mask_map = mask_map.map(|mask| resize_high_quality(&mask, mask_target, true));
            sampling_scale = (sampling_scale.0 / reduction_x, sampling_scale.1 / reduction_y);
            source_shift = (source_shift.0 / reduction_x, source_shift.1 / reduction_y);
        } else {
            let target = Resolution { x: scaled_size.0 as usize, y: scaled_size.1 as usize };
            source_map = resize_high_quality(&source_map, target, false);
            mask_map = mask_map.map(|mask| resize_high_quality(&mask, target, false));
            sampling_scale = (source_map.width() as f32 / scaled_size.0, source_map.height() as f32 / scaled_size.1);
        }
    }

    let source_center = (
        source_map.width() as f32 * 0.5 - source_shift.0,
        source_map.height() as f32 * 0.5 - source_shift.1,
    );
    let corners = [(0.0f32, 0.0f32), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)];
    let mut first_column = i32::MAX;
    let mut first_row = i32::MAX;
    let mut last_column = i32::MIN;
    let mut last_row = i32::MIN;
    for (corner_x, corner_y) in corners {
        let mut x = corner_x * source_map.width() as f32;
        let mut y = corner_y * source_map.height() as f32;
        x -= source_center.0;
        y -= source_center.1;
        x /= sampling_scale.0;
        y /= sampling_scale.1;
        let rotated_x = x * inverse_cosine - y * inverse_sine + placement.0;
        let rotated_y = x * inverse_sine + y * inverse_cosine + placement.1;
        first_column = first_column.min(rotated_x as i32);
        first_row = first_row.min(rotated_y as i32);
        last_column = last_column.max(rotated_x.ceil() as i32);
        last_row = last_row.max(rotated_y.ceil() as i32);
    }
    let clamp_x = |value: i32| value.max(0).min(destination.width() as i32 - 1) as usize;
    let clamp_y = |value: i32| value.max(0).min(destination.height() as i32 - 1) as usize;
    let first_column = clamp_x(first_column);
    let last_column = clamp_x(last_column);
    let first_row = clamp_y(first_row);
    let last_row = clamp_y(last_row);

    let destination_width = destination.width();
    let source_count = source_map.channel_count();
    let mask_plane = mask_map.as_ref().map(|mask| Plane::of(mask, 0));
    for channel in 0..destination.channel_count() {
        let source_plane = Plane::of(&source_map, channel.min(source_count - 1));
        let target = destination.channel_mut(channel);
        target
            .par_chunks_mut(destination_width)
            .enumerate()
            .skip(first_row)
            .take(last_row + 1 - first_row)
            .for_each(|(row, values)| {
                for (offset, value) in values[first_column..=last_column].iter_mut().enumerate() {
                    let column = first_column + offset;
                    let x = column as f32 - placement.0;
                    let y = row as f32 - placement.1;
                    let mut sample_x = x * cosine - y * sine;
                    let mut sample_y = x * sine + y * cosine;
                    sample_x *= sampling_scale.0;
                    sample_y *= sampling_scale.1;
                    sample_x += source_center.0;
                    sample_y += source_center.1;
                    let outside = sample_x < 0.0
                        || sample_x >= source_plane.width as f32
                        || sample_y < 0.0
                        || sample_y >= source_plane.height as f32;
                    if outside {
                        continue;
                    }
                    let (sample, weight) = if quality == TransformQuality::Draft {
                        let nearest_x = (sample_x as i32).min(source_plane.width as i32 - 1).max(0) as usize;
                        let nearest_y = (sample_y as i32).min(source_plane.height as i32 - 1).max(0) as usize;
                        let weight = mask_plane.map(|mask| mask.get(nearest_x, nearest_y));
                        (source_plane.get(nearest_x, nearest_y), weight)
                    } else {
                        let weight = mask_plane.map(|mask| mask.bilinear(sample_x, sample_y));
                        (source_plane.bilinear(sample_x, sample_y), weight)
                    };
                    let current = *value;
                    *value = match (function, weight) {
                        (Some(function), Some(weight)) => current * (1.0 - weight) + weight * function(current, sample * multiplier),
                        (Some(function), None) => function(current, sample * multiplier),
                        // `Blend` with a mask multiplies in this order: (m · s) · k.
                        (None, Some(weight)) => current * (1.0 - weight) + weight * sample * multiplier,
                        (None, None) => sample * multiplier,
                    };
                }
            });
    }
}
