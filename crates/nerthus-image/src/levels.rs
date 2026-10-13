//! Tone operations with ImageMagick semantics (`enhance.c`, `statistic.c`).
//!
//! Values are normalised (1.0 = QuantumRange). Where IM routes through its
//! 16-bit map (histogram based operations) we do the same so results match.

use rayon::prelude::*;

use crate::convolve::MAGICK_EPSILON;
use crate::colorspace::Colorspace;
use crate::image::{perceptible_reciprocal, Image, QUANTUM_RANGE, QUANTUM_SCALE};

pub(crate) const MAX_MAP: usize = 65535;

#[inline]
fn clamp_quantum(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

/// The single-precision quantum IM holds for a normalised value (images are
/// filled with `value * Quantum.Max` in single precision).
#[inline]
pub(crate) fn quantum_of(value: f32) -> f64 {
    (value * QUANTUM_RANGE as f32) as f64
}

/// `ScaleQuantumToMap(ClampToQuantum(quantum))`: the quantum is rounded to
/// IM's float `Quantum` before it is binned.
#[inline]
pub(crate) fn quantum_map_index(quantum: f64) -> usize {
    let quantum = quantum as f32 as f64;
    if quantum >= MAX_MAP as f64 {
        return MAX_MAP;
    }
    if quantum.is_nan() || quantum <= 0.0 {
        return 0;
    }
    (quantum + 0.5) as usize
}

/// `ScaleQuantumToMap(ClampToQuantum(value))` for a normalised value.
#[inline]
pub(crate) fn map_index(value: f64) -> usize {
    quantum_map_index(value * QUANTUM_RANGE)
}

/// Per-pixel `GetPixelIntensity` in quantum units, used by histogram
/// operations: the grey value for one-channel images, Rec.709 luma otherwise
/// (grey + alpha images read the grey channel as red, green and blue).
fn quantum_intensities(image: &Image) -> Vec<f64> {
    let channels = image.channels;
    let intensity = move |pixel: &[f32]| {
        let red = quantum_of(pixel[0]);
        if channels == 1 {
            return red;
        }
        let (green, blue) = if channels < 3 { (red, red) } else { (quantum_of(pixel[1]), quantum_of(pixel[2])) };
        0.212656 * red + 0.715158 * green + 0.072186 * blue
    };
    image.data.par_chunks(channels).map(intensity).collect()
}

/// IM `ScaleMapToQuantum` (Q16 HDRI) in quantum units: clamped to the map
/// range, then stored as a float `Quantum`.
#[inline]
fn scale_map_to_quantum(value: f64) -> f64 {
    if value <= 0.0 {
        return 0.0;
    }
    if value >= MAX_MAP as f64 {
        return QUANTUM_RANGE;
    }
    value as f32 as f64
}

/// IM `AutoLevelImage` = `MinMaxStretchImage(0, 0, 1)`; with the default
/// channel mask (Magick.NET passes `Channels.All`) every channel shares one
/// range.
pub fn auto_level(image: &mut Image) {
    let (min, max) = image.min_max();
    let black = min as f64 * QUANTUM_RANGE;
    let white = max as f64 * QUANTUM_RANGE;
    if (black - white).abs() < MAGICK_EPSILON {
        return;
    }
    level_quantum(image, None, black, white, 1.0);
}

/// IM `LevelImage(black, white, gamma)` with normalised black/white points.
/// IM 7.1.0 ends the operation with `ClampImage`, so results are clamped.
pub fn level(image: &mut Image, black: f64, white: f64, gamma: f64) {
    level_quantum(image, None, black * QUANTUM_RANGE, white * QUANTUM_RANGE, gamma);
}

/// IM `gamma_pow`.
#[inline]
fn gamma_pow(value: f64, gamma: f64) -> f64 {
    if value < 0.0 {
        return value;
    }
    value.powf(gamma)
}

/// `LevelImage` in quantum units, on every channel or only on `only_channel`
/// (a single-channel mask).
fn level_quantum(image: &mut Image, only_channel: Option<usize>, black: f64, white: f64, gamma: f64) {
    let scale = perceptible_reciprocal(white - black);
    let inverse_gamma = perceptible_reciprocal(gamma);
    let channels = image.channels;
    image.data.par_chunks_mut(channels).for_each(|pixel| {
        for (channel, value) in pixel.iter_mut().enumerate() {
            if only_channel.is_some_and(|selected| selected != channel) {
                continue;
            }
            let quantum = *value as f64 * QUANTUM_RANGE;
            let leveled = QUANTUM_RANGE * gamma_pow(scale * (quantum - black), inverse_gamma);
            *value = (clamp_pixel(leveled) / QUANTUM_RANGE) as f32;
        }
    });
}

/// IM `ClampPixel` on a quantum value (NaN passes through).
#[inline]
fn clamp_pixel(value: f64) -> f64 {
    if value < 0.0 {
        return 0.0;
    }
    if value >= QUANTUM_RANGE {
        return QUANTUM_RANGE;
    }
    value
}

/// IM `ClampImage` (HDRI builds): every channel to [0, QuantumRange].
pub fn clamp_image(image: &mut Image) {
    image.data.par_iter_mut().for_each(|value| {
        let clamped = clamp_pixel(*value as f64 * QUANTUM_RANGE);
        *value = (clamped / QUANTUM_RANGE) as f32;
    });
}

/// IM `NormalizeImage` = contrast stretch with 2% black / 1% white clipping.
pub fn normalize(image: &mut Image) {
    let pixels = image.pixel_count() as f64;
    contrast_stretch(image, 0.02 * pixels, 0.99 * pixels);
}

/// IM `ContrastStretchImage(black_point, white_point)` with pixel-count points.
pub fn contrast_stretch(image: &mut Image, black_point: f64, white_point: f64) {
    let intensities = quantum_intensities(image);
    let mut histogram = vec![0.0f64; MAX_MAP + 1];
    for &intensity in &intensities {
        histogram[quantum_map_index(intensity)] += 1.0;
    }
    let total = image.pixel_count() as f64;

    let mut black = 0usize;
    let mut accumulated = 0.0;
    while black <= MAX_MAP {
        accumulated += histogram[black];
        if accumulated > black_point {
            break;
        }
        black += 1;
    }
    let mut white = MAX_MAP;
    accumulated = 0.0;
    while white != 0 {
        accumulated += histogram[white];
        if accumulated > total - white_point {
            break;
        }
        white -= 1;
    }

    let gamma = perceptible_reciprocal(white as f64 - black as f64);
    let mut stretch = vec![0.0f64; MAX_MAP + 1];
    for (index, entry) in stretch.iter_mut().enumerate() {
        if index < black {
            *entry = 0.0;
        } else if index > white {
            *entry = 1.0;
        } else if black != white {
            *entry = (MAX_MAP as f64 * gamma * (index as f64 - black as f64)) / QUANTUM_RANGE;
        }
    }
    image.data.par_iter_mut().for_each(|value| {
        let index = map_index(*value as f64);
        *value = clamp_quantum(stretch[index]) as f32;
    });
}

/// IM `EqualizeImage` with the default channel mask, whose `SyncChannels`
/// flag makes every channel's histogram count the pixel intensity; all
/// channels then share one map.
pub fn equalize(image: &mut Image) {
    let intensities = quantum_intensities(image);
    let mut histogram = vec![0.0f64; MAX_MAP + 1];
    for &intensity in &intensities {
        histogram[quantum_map_index(intensity)] += 1.0;
    }
    let mut map = vec![0.0f64; MAX_MAP + 1];
    let mut accumulated = 0.0;
    for (index, count) in histogram.iter().enumerate() {
        accumulated += count;
        map[index] = accumulated;
    }
    let black = map[0];
    let white = map[MAX_MAP];
    if black == white {
        return;
    }
    let mut equalize_map = vec![0.0f64; MAX_MAP + 1];
    for (entry, cumulative) in equalize_map.iter_mut().zip(&map) {
        let mapped = (MAX_MAP as f64 * (cumulative - black)) / (white - black);
        *entry = scale_map_to_quantum(mapped);
    }
    image.data.par_iter_mut().for_each(|value| {
        let index = map_index(*value as f64);
        *value = (equalize_map[index] / QUANTUM_RANGE) as f32;
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvaluateOperator {
    Add,
    AddModulus,
    Subtract,
    Multiply,
    Divide,
    Pow,
    Log,
    InverseLog,
    Max,
    Min,
    Mean,
    Set,
    Abs,
    Exponential,
    Cosine,
    Sine,
    LeftShift,
    RightShift,
    RootMeanSquare,
    Threshold,
    ThresholdBlack,
    ThresholdWhite,
}

/// IM `EvaluateImage(op, value)`. HDRI `ClampToQuantum` does not clamp.
pub fn evaluate(image: &mut Image, operator: EvaluateOperator, value: f64) {
    image.data.par_iter_mut().for_each(|pixel| {
        let quantum = *pixel as f64 * QUANTUM_RANGE;
        let result = apply_evaluate(quantum, operator, value);
        *pixel = (result / QUANTUM_RANGE) as f32;
    });
}

/// IM `ApplyEvaluateOperator` on a quantum-scale pixel.
fn apply_evaluate(pixel: f64, operator: EvaluateOperator, value: f64) -> f64 {
    let scale = 1.0 / QUANTUM_RANGE;
    match operator {
        EvaluateOperator::Abs => (pixel + value).abs(),
        EvaluateOperator::Add => pixel + value,
        EvaluateOperator::AddModulus => {
            let sum = pixel + value;
            sum - (QUANTUM_RANGE + 1.0) * (sum / (QUANTUM_RANGE + 1.0)).floor()
        }
        EvaluateOperator::Cosine => QUANTUM_RANGE * (0.5 * (2.0 * std::f64::consts::PI * scale * pixel * value).cos() + 0.5),
        EvaluateOperator::Divide => pixel / if value == 0.0 { 1.0 } else { value },
        EvaluateOperator::Exponential => QUANTUM_RANGE * (value * scale * pixel).exp(),
        EvaluateOperator::InverseLog => (QUANTUM_RANGE * (value + 1.0).powf(scale * pixel) - 1.0) * perceptible_reciprocal(value),
        EvaluateOperator::LeftShift => {
            let mut result = pixel;
            let mut index = 0.0;
            while index < value.trunc() {
                result *= 2.0;
                index += 1.0;
            }
            result
        }
        EvaluateOperator::Log => {
            if scale * pixel >= MAGICK_EPSILON {
                return QUANTUM_RANGE * (scale * value * pixel + 1.0).ln() / (value + 1.0).ln();
            }
            0.0
        }
        EvaluateOperator::Max => pixel.max(value),
        EvaluateOperator::Mean => (pixel + value) / 2.0,
        EvaluateOperator::Min => pixel.min(value),
        EvaluateOperator::Multiply => value * pixel,
        EvaluateOperator::Pow => {
            if pixel < 0.0 {
                return -(QUANTUM_RANGE * (-(scale * pixel)).powf(value));
            }
            QUANTUM_RANGE * (scale * pixel).powf(value)
        }
        EvaluateOperator::RightShift => {
            let mut result = pixel;
            let mut index = 0.0;
            while index < value.trunc() {
                result /= 2.0;
                index += 1.0;
            }
            result
        }
        EvaluateOperator::RootMeanSquare => pixel * pixel + value,
        EvaluateOperator::Set => value,
        EvaluateOperator::Sine => QUANTUM_RANGE * (0.5 * (2.0 * std::f64::consts::PI * scale * pixel * value).sin() + 0.5),
        EvaluateOperator::Subtract => pixel - value,
        EvaluateOperator::Threshold => {
            if pixel <= value { 0.0 } else { QUANTUM_RANGE }
        }
        EvaluateOperator::ThresholdBlack => {
            if pixel <= value { 0.0 } else { pixel }
        }
        EvaluateOperator::ThresholdWhite => {
            if pixel > value { QUANTUM_RANGE } else { pixel }
        }
    }
}

/// IM `ConvertRGBToHSB` on quantum values: hue and saturation in [0, 1],
/// brightness normalised.
fn convert_rgb_to_hsb(red: f64, green: f64, blue: f64) -> (f64, f64, f64) {
    let mut min = if red < green { red } else { green };
    if blue < min {
        min = blue;
    }
    let mut max = if red > green { red } else { green };
    if blue > max {
        max = blue;
    }
    if max.abs() < MAGICK_EPSILON {
        return (0.0, 0.0, 0.0);
    }
    let delta = max - min;
    let saturation = delta / max;
    let brightness = QUANTUM_SCALE * max;
    if delta.abs() < MAGICK_EPSILON {
        return (0.0, saturation, brightness);
    }
    let mut hue = if (red - max).abs() < MAGICK_EPSILON {
        (green - blue) / delta
    } else if (green - max).abs() < MAGICK_EPSILON {
        2.0 + (blue - red) / delta
    } else {
        4.0 + (red - green) / delta
    };
    hue /= 6.0;
    if hue < 0.0 {
        hue += 1.0;
    }
    (hue, saturation, brightness)
}

/// IM `ConvertHSBToRGB`, returning quantum values.
fn convert_hsb_to_rgb(hue: f64, saturation: f64, brightness: f64) -> (f64, f64, f64) {
    if saturation.abs() < MAGICK_EPSILON {
        let gray = QUANTUM_RANGE * brightness;
        return (gray, gray, gray);
    }
    let h = 6.0 * (hue - hue.floor());
    let f = h - h.floor();
    let p = brightness * (1.0 - saturation);
    let q = brightness * (1.0 - saturation * f);
    let t = brightness * (1.0 - (saturation * (1.0 - f)));
    let (red, green, blue) = match h as i32 {
        1 => (q, brightness, p),
        2 => (p, brightness, t),
        3 => (p, q, brightness),
        4 => (t, p, brightness),
        5 => (brightness, p, q),
        _ => (brightness, t, p),
    };
    (QUANTUM_RANGE * red, QUANTUM_RANGE * green, QUANTUM_RANGE * blue)
}

/// Applies `function` to the red, green and blue quantum values of every
/// pixel. Grey images read their single channel as all three and keep the
/// blue result, which IM writes last.
fn map_rgb(image: &mut Image, function: impl Fn(f64, f64, f64) -> (f64, f64, f64) + Sync) {
    let channels = image.channels;
    if channels < 3 {
        image.data.par_chunks_mut(channels).for_each(|pixel| {
            let gray = pixel[0] as f64 * QUANTUM_RANGE;
            let (_, _, blue) = function(gray, gray, gray);
            pixel[0] = (blue / QUANTUM_RANGE) as f32;
        });
        return;
    }
    image.data.par_chunks_mut(channels).for_each(|pixel| {
        let red = pixel[0] as f64 * QUANTUM_RANGE;
        let green = pixel[1] as f64 * QUANTUM_RANGE;
        let blue = pixel[2] as f64 * QUANTUM_RANGE;
        let (red, green, blue) = function(red, green, blue);
        pixel[0] = (red / QUANTUM_RANGE) as f32;
        pixel[1] = (green / QUANTUM_RANGE) as f32;
        pixel[2] = (blue / QUANTUM_RANGE) as f32;
    });
}

/// IM `ContrastImage(sharpen)`: moves the HSB brightness along a sine curve.
pub fn contrast(image: &mut Image, sharpen: bool) {
    let sign = if sharpen { 1.0 } else { -1.0 };
    map_rgb(image, |red, green, blue| {
        let (hue, saturation, mut brightness) = convert_rgb_to_hsb(red, green, blue);
        brightness += 0.5 * sign * (0.5 * ((std::f64::consts::PI * (brightness - 0.5)).sin() + 1.0) - brightness);
        brightness = brightness.clamp(0.0, 1.0);
        convert_hsb_to_rgb(hue, saturation, brightness)
    });
}

/// IM `SigmoidalContrastImage(sharpen, contrast, midpoint)` with a normalised midpoint.
pub fn sigmoidal_contrast(image: &mut Image, sharpen: bool, contrast: f64, midpoint: f64) {
    if contrast < MAGICK_EPSILON {
        return;
    }
    let sigmoidal = |x: f64| (0.5 * contrast * (x - midpoint)).tanh();
    let low = sigmoidal(0.0);
    let high = sigmoidal(1.0);
    image.data.par_iter_mut().for_each(|value| {
        let x = *value as f64;
        let result = if sharpen {
            (sigmoidal(x) - low) / (high - low)
        } else {
            // Inverse scaled sigmoidal.
            let argument = (high - low) * x + low;
            let clamped = argument.clamp(-1.0 + MAGICK_EPSILON, 1.0 - MAGICK_EPSILON);
            midpoint + (2.0 / contrast) * clamped.atanh()
        };
        *value = result as f32;
    });
}

/// IM `BrightnessContrastImage(brightness%, contrast%)`: a first-order
/// `FunctionImage` polynomial on every channel (Magick.NET passes the
/// Composite channel mask, which includes alpha).
pub fn brightness_contrast(image: &mut Image, brightness: f64, contrast: f64) {
    let mut slope = (std::f64::consts::PI * (contrast / 100.0 + 1.0) / 4.0).tan();
    if slope < 0.0 {
        slope = 0.0;
    }
    let intercept = brightness / 100.0 + ((100.0 - brightness) / 200.0) * (1.0 - slope);
    let coefficients = [slope, intercept];
    image.data.par_iter_mut().for_each(|value| {
        let pixel = *value as f64 * QUANTUM_RANGE;
        let mut result = 0.0f64;
        for coefficient in coefficients {
            result = result * QUANTUM_SCALE * pixel + coefficient;
        }
        result *= QUANTUM_RANGE;
        *value = (result / QUANTUM_RANGE) as f32;
    });
}

/// IM `ScaleMapToQuantum` (Q16 HDRI) on a map value, normalised.
fn map_to_quantum(value: f64) -> f64 {
    if value <= 0.0 {
        return 0.0;
    }
    if value >= MAX_MAP as f64 {
        return 1.0;
    }
    value / QUANTUM_RANGE
}

/// IM `GammaImage(gamma)`. IM 7.1.0 goes through its 16-bit gamma map even
/// in HDRI builds, so values are clamped to [0, 1] and quantised.
pub fn gamma(image: &mut Image, gamma: f64) {
    if gamma == 1.0 {
        return;
    }
    let mut map = vec![0.0f64; MAX_MAP + 1];
    if gamma != 0.0 {
        let exponent = perceptible_reciprocal(gamma);
        for (index, entry) in map.iter_mut().enumerate() {
            let mapped = MAX_MAP as f64 * (index as f64 / MAX_MAP as f64).powf(exponent);
            *entry = map_to_quantum(mapped);
        }
    }
    image.data.par_iter_mut().for_each(|value| {
        *value = map[map_index(*value as f64)] as f32;
    });
}

/// IM `AutoGammaImage` with the Composite channel mask Magick.NET passes:
/// each channel gets the gamma that moves its own mean to 0.5. The mask bit
/// IM derives from an alpha channel's offset selects no channel, so alpha is
/// left alone.
pub fn auto_gamma(image: &mut Image) {
    let log_mean = 0.5f64.ln();
    let channels = image.channels;
    let alpha_channel = if channels == 2 || channels == 4 { Some(channels - 1) } else { None };
    for channel in 0..channels {
        if Some(channel) == alpha_channel {
            continue;
        }
        // `GetImageMean`: a sequential sum scaled by the reciprocal area.
        let mut sum = 0.0f64;
        let mut area = 0.0f64;
        for pixel in image.data.chunks(channels) {
            sum += pixel[channel] as f64 * QUANTUM_RANGE;
            area += 1.0;
        }
        let mean = sum * perceptible_reciprocal(area);
        let gamma = (mean * QUANTUM_SCALE).ln() / log_mean;
        level_quantum(image, Some(channel), 0.0, QUANTUM_RANGE, gamma);
    }
}

/// IM `AdaptiveThresholdImage(width, height, bias)`: compares each pixel
/// with the mean of a `width` x `height` window (offset like IM) plus `bias`
/// (normalised).
pub fn adaptive_threshold(image: &Image, width: usize, height: usize, bias: f64) -> Image {
    if width == 0 || height == 0 {
        return image.clone();
    }
    let channels = image.channels;
    let columns = image.width;
    let count = (width * height) as f64;
    let left = (width / 2) as isize;
    let top = (height / 2) as isize;
    let mut result = image.clone();
    result.data.par_chunks_mut(columns * channels).enumerate().for_each(|(y, out_row)| {
        for channel in 0..channels {
            let window_sum = |x: isize| -> f64 {
                let mut sum = 0.0;
                for v in 0..height as isize {
                    for u in 0..width as isize {
                        sum += image.virtual_get(x - left + u, y as isize - top + v, channel) as f64;
                    }
                }
                sum
            };
            let column_sum = |x: isize| -> f64 {
                let mut sum = 0.0;
                for v in 0..height as isize {
                    sum += image.virtual_get(x, y as isize - top + v, channel) as f64;
                }
                sum
            };
            // Running window like IM: drop the left column, add the right one.
            let mut sum = window_sum(0) - column_sum(-left + width as isize - 1);
            let mut previous_left = 0.0f64;
            for x in 0..columns {
                sum -= previous_left;
                let window_left = x as isize - left;
                previous_left = column_sum(window_left);
                sum += column_sum(window_left + width as isize - 1);
                let mean = sum / count + bias;
                let value = image.get(x, y, channel) as f64;
                out_row[x * channels + channel] = if value <= mean { 0.0 } else { 1.0 };
            }
        }
    });
    result
}

/// IM `BilevelImage` / `Threshold(percentage)`.
pub fn threshold(image: &mut Image, threshold: f64) {
    image.data.par_iter_mut().for_each(|value| {
        *value = if (*value as f64) <= threshold { 0.0 } else { 1.0 };
    });
}

/// IM `BlackThresholdImage(image, "N%")` as Magick.NET calls it: grey
/// images are relabelled sRGB first, and with the Composite channel mask
/// every channel is compared on its own; values below the threshold become 0
/// (alpha uses a 100% threshold).
pub fn black_threshold(image: &mut Image, percent: f64) {
    if image.colorspace.is_gray() {
        image.set_colorspace(Colorspace::SRgb);
    }
    let threshold = percent * (QUANTUM_RANGE / 100.0);
    let alpha_threshold = 100.0 * (QUANTUM_RANGE / 100.0);
    let channels = image.channels;
    let alpha_channel = if channels == 2 || channels == 4 { Some(channels - 1) } else { None };
    image.data.par_chunks_mut(channels).for_each(|pixel| {
        for (channel, value) in pixel.iter_mut().enumerate() {
            let limit = if Some(channel) == alpha_channel { alpha_threshold } else { threshold };
            if (*value as f64 * QUANTUM_RANGE) < limit {
                *value = 0.0;
            }
        }
    });
}

/// IM `ConvertRGBToHSL` on quantum values (all results in [0, 1]).
fn convert_rgb_to_hsl(red: f64, green: f64, blue: f64) -> (f64, f64, f64) {
    let red = QUANTUM_SCALE * red;
    let green = QUANTUM_SCALE * green;
    let blue = QUANTUM_SCALE * blue;
    let max = red.max(green.max(blue));
    let min = red.min(green.min(blue));
    let c = max - min;
    let lightness = (max + min) / 2.0;
    if c <= 0.0 {
        return (0.0, 0.0, lightness);
    }
    let mut hue = if (max - red).abs() < MAGICK_EPSILON {
        let mut hue = (green - blue) / c;
        if green < blue {
            hue += 6.0;
        }
        hue
    } else if (max - green).abs() < MAGICK_EPSILON {
        2.0 + (blue - red) / c
    } else {
        4.0 + (red - green) / c
    };
    hue *= 60.0 / 360.0;
    let saturation = if lightness <= 0.5 {
        c * perceptible_reciprocal(2.0 * lightness)
    } else {
        c * perceptible_reciprocal(2.0 - 2.0 * lightness)
    };
    (hue, saturation, lightness)
}

/// IM `ConvertHSLToRGB`, returning quantum values.
fn convert_hsl_to_rgb(hue: f64, saturation: f64, lightness: f64) -> (f64, f64, f64) {
    let mut h = hue * 360.0;
    let c = if lightness <= 0.5 { 2.0 * lightness * saturation } else { (2.0 - 2.0 * lightness) * saturation };
    let min = lightness - 0.5 * c;
    h -= 360.0 * (h / 360.0).floor();
    h /= 60.0;
    let x = c * (1.0 - (h - 2.0 * (h / 2.0).floor() - 1.0).abs());
    let (red, green, blue) = match h.floor() as i32 {
        0 => (min + c, min + x, min),
        1 => (min + x, min + c, min),
        2 => (min, min + c, min + x),
        3 => (min, min + x, min + c),
        4 => (min + x, min, min + c),
        5 => (min + c, min, min + x),
        _ => return (0.0, 0.0, 0.0),
    };
    (QUANTUM_RANGE * red, QUANTUM_RANGE * green, QUANTUM_RANGE * blue)
}

/// IM `ModulateImage("brightness,saturation,hue")` with its default HSL
/// model (`ModulateHSL`). Grey images are sRGB-compatible, so IM keeps them
/// grey.
pub fn modulate(image: &mut Image, brightness: f64, saturation: f64, hue: f64) {
    map_rgb(image, |red, green, blue| {
        let (mut h, mut s, mut l) = convert_rgb_to_hsl(red, green, blue);
        h += ((hue - 100.0) % 200.0) / 200.0;
        s *= 0.01 * saturation;
        l *= 0.01 * brightness;
        convert_hsl_to_rgb(h, s, l)
    });
}

/// IM `EnhanceImage`: weighted 5x5 average of similar neighbours.
pub fn enhance(image: &Image) -> Image {
    const WEIGHTS: [[f64; 5]; 5] = [
        [5.0, 8.0, 10.0, 8.0, 5.0],
        [8.0, 20.0, 40.0, 20.0, 8.0],
        [10.0, 40.0, 80.0, 40.0, 10.0],
        [8.0, 20.0, 40.0, 20.0, 8.0],
        [5.0, 8.0, 10.0, 8.0, 5.0],
    ];
    let channels = image.channels;
    let width = image.width;
    let mut result = image.new_like(width, image.height);
    let rgb = |x: isize, y: isize| -> [f64; 3] {
        if channels == 1 {
            let gray = image.virtual_get(x, y, 0) as f64;
            return [gray, gray, gray];
        }
        [image.virtual_get(x, y, 0) as f64, image.virtual_get(x, y, 1) as f64, image.virtual_get(x, y, 2) as f64]
    };
    result
        .data
        .par_chunks_mut(width * channels)
        .enumerate()
        .for_each(|(y, out_row)| {
            for x in 0..width {
                let center = rgb(x as isize, y as isize);
                let mut aggregate = [0.0f64; 3];
                let mut total = 0.0f64;
                for (j, weight_row) in WEIGHTS.iter().enumerate() {
                    for (i, weight) in weight_row.iter().enumerate() {
                        let sample = rgb(x as isize + i as isize - 2, y as isize + j as isize - 2);
                        let mean_r = (sample[0] + center[0]) / 2.0;
                        let distance_r = sample[0] - center[0];
                        let mut distance_squared = (4.0 + mean_r) * distance_r * distance_r;
                        let mean_g = (sample[1] + center[1]) / 2.0;
                        let distance_g = sample[1] - center[1];
                        distance_squared += (7.0 - mean_g) * distance_g * distance_g;
                        let mean_b = (sample[2] + center[2]) / 2.0;
                        let distance_b = sample[2] - center[2];
                        distance_squared += (5.0 - mean_b) * distance_b * distance_b;
                        if distance_squared < 0.069 {
                            aggregate[0] += weight * sample[0];
                            aggregate[1] += weight * sample[1];
                            aggregate[2] += weight * sample[2];
                            total += weight;
                        }
                    }
                }
                for channel in 0..channels {
                    // IM averages in quantum units with a half-step rounding bias.
                    let value = if total > MAGICK_EPSILON {
                        (aggregate[channel.min(2)] * QUANTUM_RANGE + total / 2.0) / total / QUANTUM_RANGE
                    } else {
                        center[channel.min(2)]
                    };
                    out_row[x * channels + channel] = value as f32;
                }
            }
        });
    result
}
