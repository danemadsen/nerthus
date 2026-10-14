//! Map maths from the engine: its map helpers and the exponential shaper.

use nerthus_engine::{Map, Resolution};
use rayon::prelude::*;

/// `MathF.Pow` as .NET evaluates it: a call into the C library. Rust would
/// otherwise fold constant exponents (`x.powf(2.0)` becomes `x * x`), which
/// libm does not always round the same way.
pub fn pow(base: f32, exponent: f32) -> f32 {
    base.powf(std::hint::black_box(exponent))
}

/// `MathF.Log` as a library call at run time (never folded at compile
/// time, which could round differently).
pub fn ln(x: f32) -> f32 {
    std::hint::black_box(x).ln()
}

/// `MathF.Log2` as a library call at run time.
pub fn log2(x: f32) -> f32 {
    std::hint::black_box(x).log2()
}

/// `MathF.Sqrt` (correctly rounded, so folding cannot change it).
pub fn sqrt(x: f32) -> f32 {
    x.sqrt()
}

/// `MathF.Sin` as a library call at run time.
pub fn sin(x: f32) -> f32 {
    std::hint::black_box(x).sin()
}

/// `MathF.Cos` as a library call at run time.
pub fn cos(x: f32) -> f32 {
    std::hint::black_box(x).cos()
}

/// `tanf` as a library call at run time.
pub fn tan(x: f32) -> f32 {
    std::hint::black_box(x).tan()
}

/// `MathF.Asin` (`asinf`) as a library call at run time.
pub fn asin(x: f32) -> f32 {
    std::hint::black_box(x).asin()
}

/// `MathF.Acos` (`acosf`) as a library call at run time.
pub fn acos(x: f32) -> f32 {
    std::hint::black_box(x).acos()
}

/// `MathF.Tanh` as a library call at run time.
pub fn tanh(x: f32) -> f32 {
    std::hint::black_box(x).tanh()
}

/// `MathF.Exp` as a library call at run time.
pub fn exp(x: f32) -> f32 {
    std::hint::black_box(x).exp()
}

/// `(MathF.Sin(x), MathF.Cos(x))` as two separate library calls. When Rust
/// code takes both of one angle, LLVM merges them into a single `sincos`
/// call that does not round exactly like `sinf` and `cosf`.
pub fn sin_cos(angle: f32) -> (f32, f32) {
    let sine = std::hint::black_box(angle).sin();
    let cosine = angle.cos();
    (sine, cosine)
}

/// `MathF.Max` (IEEE 754-2019 `maximum`): NaN propagates and +0 is greater
/// than -0.
pub fn dotnet_max(a: f32, b: f32) -> f32 {
    if a != b {
        if !a.is_nan() {
            return if b < a { a } else { b };
        }
        return a;
    }
    if b.is_sign_negative() { a } else { b }
}

/// `MathF.Min` (IEEE 754-2019 `minimum`): NaN propagates and -0 is smaller
/// than +0.
pub fn dotnet_min(a: f32, b: f32) -> f32 {
    if a != b {
        if !a.is_nan() {
            return if a < b { a } else { b };
        }
        return a;
    }
    if a.is_sign_negative() { a } else { b }
}

/// `GrayscaleConversion`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrayscaleConversion {
    Average,
    Bt601,
    Bt709,
    RedChannel,
}

impl GrayscaleConversion {
    pub fn from_name(name: &str) -> Self {
        match name {
            "BT601" => Self::Bt601,
            "BT709" => Self::Bt709,
            "RedChannel" => Self::RedChannel,
            _ => Self::Average,
        }
    }
}

/// One channel from a colour map (single-channel maps are
/// copied).
pub fn grayscale(map: &Map, conversion: GrayscaleConversion) -> Map {
    let resolution = map.resolution();
    if map.channel_count() == 1 {
        return Map::from_data(resolution, map.data().to_vec());
    }
    let red = map.channel(0);
    let green = map.channel(1);
    let blue = map.channel(2);
    let data: Vec<f32> = (0..red.len())
        .map(|index| {
            let (r, g, b) = (red[index], green[index], blue[index]);
            match conversion {
                GrayscaleConversion::Average => (r + g + b) / 3.0,
                GrayscaleConversion::Bt709 => r * 0.2126 + g * 0.7152 + b * 0.0722,
                GrayscaleConversion::Bt601 => r * 0.299 + g * 0.587 + b * 0.114,
                GrayscaleConversion::RedChannel => r,
            }
        })
        .collect();
    Map::from_data(resolution, data)
}

/// Mirrors a map left to right.
pub fn flip_x(map: &Map) -> Map {
    let width = map.width();
    let mut channels = Vec::with_capacity(map.channel_count());
    for channel in 0..map.channel_count() {
        let source = map.channel(channel);
        let mut flipped = vec![0.0f32; source.len()];
        for (target_row, source_row) in flipped.chunks_mut(width).zip(source.chunks(width)) {
            for (index, &value) in source_row.iter().enumerate() {
                target_row[width - 1 - index] = value;
            }
        }
        channels.push(flipped);
    }
    Map::from_channels(map.resolution(), channels)
}

/// Mirrors a square map top to bottom.
pub fn flip_y(map: &Map) -> Map {
    let width = map.width();
    let last = width - 1;
    let mut channels = Vec::with_capacity(map.channel_count());
    for channel in 0..map.channel_count() {
        let source = map.channel(channel);
        let mut flipped = vec![0.0f32; source.len()];
        for (row, source_row) in source.chunks(width).enumerate() {
            let target = (last - row) * width;
            flipped[target..target + width].copy_from_slice(source_row);
        }
        channels.push(flipped);
    }
    Map::from_channels(map.resolution(), channels)
}

/// Rotates a square map by half a turn.
pub fn flip_xy(map: &Map) -> Map {
    let width = map.width();
    let last = width - 1;
    let mut channels = Vec::with_capacity(map.channel_count());
    for channel in 0..map.channel_count() {
        let source = map.channel(channel);
        let mut flipped = vec![0.0f32; source.len()];
        for (row, source_row) in source.chunks(width).enumerate() {
            let target = (last - row) * width;
            for (index, &value) in source_row.iter().enumerate() {
                flipped[target + last - index] = value;
            }
        }
        channels.push(flipped);
    }
    Map::from_channels(map.resolution(), channels)
}

/// Smallest and largest value of a plane (which reports them as
/// max, min).
pub fn min_max(data: &[f32]) -> (f32, f32) {
    let mut minimum = f32::MAX;
    let mut maximum = f32::MIN;
    for &value in data {
        minimum = minimum.min(value);
        maximum = maximum.max(value);
    }
    (minimum, maximum)
}

/// The lowest and highest value of a square plane `width` wide as the
/// vectorised scan finds them: each row is read eight floats at a time
/// from its start, so when the width is not a multiple of eight the last
/// row's final read runs past the plane into the buffer's padding (zeros
/// in a fresh buffer), which counts too.
pub fn square_plane_min_max(data: &[f32], width: usize) -> (f32, f32) {
    let area = (width * width).min(data.len());
    let (mut minimum, mut maximum) = min_max(&data[..area]);
    if !width.is_multiple_of(8) {
        minimum = minimum.min(0.0);
        maximum = maximum.max(0.0);
    }
    (minimum, maximum)
}

/// `1 - x` on every channel.
pub fn invert(map: &mut Map) {
    map.apply(|value| 1.0 - value);
}

/// Linear remap of `from_max..from_min` onto `to_max..to_min`, in place:
/// `x * scale + offset` with `scale = (to_max - to_min) /
/// (from_max - from_min)` and `offset = to_min - from_min * scale`.
pub fn remap(map: &mut Map, from_max: f32, from_min: f32, to_max: f32, to_min: f32) {
    let scale = (to_max - to_min) / (from_max - from_min);
    let offset = to_min - from_min * scale;
    map.apply(|value| value * scale + offset);
}

/// Stretches each colour channel (or the single channel) to 0..1. Colour
/// maps come back with three channels.
pub fn autolevel(map: Map) -> Map {
    if map.channel_count() > 1 {
        let resolution = map.resolution();
        let mut leveled = Vec::with_capacity(3);
        for channel in 0..3 {
            let plane = map.rgb_channel(channel).to_vec();
            let single = Map::from_data(resolution, plane);
            leveled.push(autolevel(single).into_data());
        }
        return Map::from_channels(resolution, leveled);
    }
    let mut map = map;
    let (minimum, maximum) = min_max(map.data());
    remap(&mut map, maximum, minimum, 1.0, 0.0);
    map
}

/// `low + x * (high - low)`: squeezes 0..1 into `low..high`.
pub fn fit_range(map: &Map, low: f32, high: f32) -> Map {
    let range = high - low;
    map.map_values(|value| low + value * range)
}

/// The clip: shifts down by `low`, flattens everything above
/// `high` to `high - low`, then clamps to 0..1.
pub fn clip(data: &[f32], low: f32, high: f32) -> Vec<f32> {
    data.par_iter()
        .map(|&original| {
            let mut value = original - low;
            if original > high {
                value = high - low;
            }
            value.clamp(0.0, 1.0)
        })
        .collect()
}

/// [`clip`] on a single-channel map.
pub fn clip_map(map: &Map, low: f32, high: f32) -> Map {
    let clipped = clip(map.data(), low, high);
    Map::from_data(square_resolution(clipped.len()), clipped)
}

/// Values above `high` become 1 and below `low` become 0.
pub fn extend(map: &Map, low: f32, high: f32) -> Map {
    let mut result = map.clone();
    let source = map.data();
    result.data_mut().par_iter_mut().zip(source.par_iter()).for_each(|(value, &original)| {
        if original > high {
            *value = 1.0;
        }
        if original < low {
            *value = 0.0;
        }
    });
    result
}

/// Lowers the map so its smallest value (capped at 1) becomes 0.
pub fn drop_to_zero(map: &Map) -> Map {
    let mut minimum = 1.0f32;
    for &value in map.data() {
        minimum = value.min(minimum);
    }
    clip_map(map, minimum, 1.0)
}

/// [`drop_to_zero`], but only when the map dips below zero.
pub fn lift_above_zero(map: Map) -> Map {
    let (minimum, _) = min_max(map.data());
    if !(minimum < 0.0) {
        return map;
    }
    drop_to_zero(&map)
}

/// Clamps every channel to [0, 1] in place.
pub fn clamp(map: &mut Map) {
    map.apply(|value| {
        let raised = crate::mathv::max(value, 0.0);
        crate::mathv::min(raised, 1.0)
    });
}

/// Clamps to [0, 1] only when the first channel leaves it.
pub fn clamp_if_out_of_range(map: &mut Map) {
    let (minimum, maximum) = min_max(map.data());
    if maximum > 1.0 || minimum < 0.0 {
        clamp(map);
    }
}

/// [`drop_to_zero`] on a plane, using its true minimum.
pub fn drop_plane(data: &[f32]) -> Vec<f32> {
    let (minimum, _) = min_max(data);
    clip(data, minimum, 1.0)
}

/// In-place remap of a plane, row by row.
pub fn remap_plane(data: &mut [f32], from_max: f32, from_min: f32, to_max: f32, to_min: f32) {
    let scale = (to_max - to_min) / (from_max - from_min);
    let offset = to_min - from_min * scale;
    data.par_iter_mut().for_each(|value| *value = *value * scale + offset);
}

/// One value of the soft clip: values past `threshold` bend
/// smoothly instead of being cut off.
pub fn soft_clip_value(mut value: f32, above: bool, mut threshold: f32, softness: f32, clipping: f32) -> f32 {
    let knee = 1.0 - clipping;
    if above {
        value = 0.0 - value;
        threshold = 0.0 - threshold;
        if value < threshold {
            value = bend(value, threshold, softness, knee);
        }
        return 0.0 - value;
    }
    if value < threshold {
        value = bend(value, threshold, softness, knee);
    }
    value
}

fn bend(value: f32, threshold: f32, softness: f32, knee: f32) -> f32 {
    let mut amount = (threshold - value) / (softness * 2.0);
    if amount > 1.0 - knee {
        return threshold - softness * (1.0 - knee * knee + 2.0 * knee * (amount - 1.0 + knee));
    }
    amount = 1.0 - amount;
    amount *= amount;
    threshold + softness * (amount - 1.0)
}

/// Soft clip of a square plane, optionally against a per-pixel
/// threshold offset.
pub fn soft_clip(data: &[f32], above: bool, threshold: f32, softness: f32, clipping: f32, threshold_map: Option<&[f32]>) -> Vec<f32> {
    data.par_iter()
        .enumerate()
        .map(|(index, &value)| {
            let local_threshold = match threshold_map {
                Some(offsets) => offsets[index] + threshold,
                None => threshold,
            };
            soft_clip_value(value, above, local_threshold, softness, clipping)
        })
        .collect()
}

/// Exponential reshaping between the plane's own minimum and maximum;
/// `factor` bends the curve (negative raises).
pub fn exponential_shape(data: &mut [f32], factor: f32) {
    let (minimum, maximum) = min_max(data);
    let normalizer = 1.0 / ((factor as f64).exp() as f32 - 1.0);
    let range = maximum - minimum;
    let range_inverse = 1.0 / range;
    if ((maximum - minimum).abs() as f64) < 1e-5 {
        return;
    }
    data.par_iter_mut().for_each(|value| {
        let mut shaped = *value - minimum;
        shaped *= range_inverse;
        shaped = (((shaped * factor) as f64).exp() as f32 - 1.0) * normalizer;
        shaped *= range;
        *value = shaped + minimum;
    });
}

/// The "Strong" contrast, an exponential shape on every channel.
pub fn strengthen(map: &mut Map, amount: f32) {
    if amount as f64 == 0.0 {
        return;
    }
    let factor = amount * -10.0;
    for channel in 0..map.channel_count() {
        exponential_shape(map.channel_mut(channel), factor);
    }
}

/// The square resolution of a plane.
pub fn square_resolution(length: usize) -> Resolution {
    let side = (length as f64).sqrt() as usize;
    Resolution::square(side)
}
