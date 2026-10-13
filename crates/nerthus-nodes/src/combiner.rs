//! The blend operations (`Combiner`).

use nerthus_engine::Map;

use crate::aspect;
use crate::fast_blur;
use crate::filter_core::{self, FilterMode};
use crate::masking;
use crate::maths;
use crate::mathv;
use crate::profiles;

/// `BlendMode` as the Combine node lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendMode {
    Blend,
    Add,
    Screen,
    Subtract,
    Difference,
    Multiply,
    Divide,
    Divide2,
    Max,
    Min,
    Hypotenuse,
    Overlay,
    Power,
    Exclusion,
    Dodge,
    Burn,
    SoftLight,
    HardLight,
    PinLight,
    GrainMerge,
    GrainExtract,
    Reflect,
    Glow,
    Phoenix,
}

impl BlendMode {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Add" => Self::Add,
            "Screen" => Self::Screen,
            "Subtract" => Self::Subtract,
            "Difference" => Self::Difference,
            "Multiply" => Self::Multiply,
            "Divide" => Self::Divide,
            "Divide2" => Self::Divide2,
            "Max" => Self::Max,
            "Min" => Self::Min,
            "Hypotenuse" => Self::Hypotenuse,
            "Overlay" => Self::Overlay,
            "Power" => Self::Power,
            "Exclusion" => Self::Exclusion,
            "Dodge" => Self::Dodge,
            "Burn" => Self::Burn,
            "SoftLight" => Self::SoftLight,
            "HardLight" => Self::HardLight,
            "PinLight" => Self::PinLight,
            "GrainMerge" => Self::GrainMerge,
            "GrainExtract" => Self::GrainExtract,
            "Reflect" => Self::Reflect,
            "Glow" => Self::Glow,
            "Phoenix" => Self::Phoenix,
            _ => Self::Blend,
        }
    }
}

/// `Combiner.Mix`: blends `b` into `a` with `mode`, `ratio` of the way.
/// Single-channel maps apply to every channel of a colour map.
pub fn mix(mode: BlendMode, ratio: f32, a: &Map, b: &Map) -> Map {
    if mode == BlendMode::Divide2 {
        return divide2(ratio, a, b);
    }
    a.zip_values(b, |a, b| blend(mode, ratio, a, b))
}

/// One value of [`mix`] for every mode but `Divide2`.
fn blend(mode: BlendMode, ratio: f32, a: f32, b: f32) -> f32 {
    match mode {
        BlendMode::Blend => mathv::mix(a, b, ratio),
        BlendMode::Add => mathv::mix(a, a + b, ratio),
        BlendMode::Screen => mathv::mix(a, 1.0 - (1.0 - a) * (1.0 - b), ratio),
        BlendMode::Subtract => mathv::mix(a, a - b, ratio),
        BlendMode::Multiply => mathv::mix(a, a * b, ratio),
        BlendMode::Divide => {
            let divisor = if b == 0.0 { 1e-35 } else { b };
            mathv::mix(a, a / divisor, ratio)
        }
        BlendMode::Max => mathv::mix(a, mathv::max(a, b), ratio),
        BlendMode::Min => mathv::mix(a, mathv::min(a, b), ratio),
        BlendMode::Difference => (a - b * ratio).abs(),
        BlendMode::Hypotenuse => mathv::mix(a, (a * a + b * b).sqrt(), ratio),
        BlendMode::Overlay => {
            let overlaid = if a < 0.5 { 2.0 * a * b } else { 1.0 - 2.0 * (1.0 - a) * (1.0 - b) };
            mathv::mix(a, overlaid, ratio)
        }
        BlendMode::Power => mathv::mix(a, mathv::pow(a, b), ratio),
        BlendMode::Exclusion => mathv::mix(a, a + b - 2.0 * a * b, ratio),
        BlendMode::Dodge => {
            let dodged = if b < 1.0 { a / (1.0 - b) } else { 1.0 };
            mathv::mix(a, dodged, ratio)
        }
        BlendMode::Burn => {
            let burned = if b > 0.0 { 1.0 - (1.0 - a) / b } else { 0.0 };
            mathv::mix(a, burned, ratio)
        }
        BlendMode::SoftLight => {
            let lit = if b > 0.0 { (1.0 - 2.0 * b) * a * a + 2.0 * b * a } else { 0.0 };
            mathv::mix(a, lit, ratio)
        }
        BlendMode::HardLight => {
            let lit = if b <= 0.5 { 2.0 * a * b } else { 1.0 - 2.0 * (1.0 - a) * (1.0 - b) };
            mathv::mix(a, lit, ratio)
        }
        BlendMode::PinLight => {
            let pinned = if b < 0.5 { mathv::min(a, 2.0 * b) } else { mathv::max(a, 2.0 * b - 1.0) };
            mathv::mix(a, pinned, ratio)
        }
        BlendMode::GrainMerge => mathv::mix(a, a + b - 0.5, ratio),
        BlendMode::GrainExtract => mathv::mix(a, a - b + 0.5, ratio),
        BlendMode::Reflect => {
            let reflected = if b < 1.0 { a * a / (1.0 - b) } else { 1.0 };
            mathv::mix(a, reflected, ratio)
        }
        BlendMode::Glow => {
            let glowing = if a < 1.0 { b * b / (1.0 - a) } else { 1.0 };
            mathv::mix(b, glowing, ratio)
        }
        BlendMode::Phoenix => mathv::mix(a, mathv::min(a, b) - mathv::max(a, b) + 1.0, ratio),
        BlendMode::Divide2 => unreachable!("Divide2 works on whole maps"),
    }
}

/// `Divide2`: `|a · (1 − ratio) + a / (b · ratio)|`, built from whole-map
/// operations.
fn divide2(ratio: f32, a: &Map, b: &Map) -> Map {
    let kept = a.multiply_scalar(1.0 - ratio);
    let scaled_divisor = b.multiply_scalar(ratio);
    let divided = a.divide(&scaled_divisor);
    let mut result = kept.add(&divided);
    result.apply(f32::abs);
    result
}

pub fn blend_maps(a: &Map, b: &Map, ratio: f32) -> Map {
    mix(BlendMode::Blend, ratio, a, b)
}

pub fn add(a: &Map, b: &Map, ratio: f32) -> Map {
    mix(BlendMode::Add, ratio, a, b)
}

pub fn screen(a: &Map, b: &Map, ratio: f32) -> Map {
    mix(BlendMode::Screen, ratio, a, b)
}

pub fn subtract(a: &Map, b: &Map, ratio: f32) -> Map {
    mix(BlendMode::Subtract, ratio, a, b)
}

pub fn multiply(a: &Map, b: &Map, ratio: f32) -> Map {
    mix(BlendMode::Multiply, ratio, a, b)
}

pub fn divide(a: &Map, b: &Map, ratio: f32) -> Map {
    mix(BlendMode::Divide, ratio, a, b)
}

pub fn difference(a: &Map, b: &Map, ratio: f32) -> Map {
    mix(BlendMode::Difference, ratio, a, b)
}

pub fn max(a: &Map, b: &Map, ratio: f32) -> Map {
    mix(BlendMode::Max, ratio, a, b)
}

pub fn min(a: &Map, b: &Map, ratio: f32) -> Map {
    mix(BlendMode::Min, ratio, a, b)
}

/// `Combiner.Embed`: presses the inverted shape of `b` into `a`
/// (`ratio` deep, deeper as `extend` grows past 0.1), then lifts the
/// result back to zero if it went below.
pub fn embed(a: &Map, b: &Map, ratio: f32, extend: f32) -> Map {
    let boost = extend - 0.1;
    let mut stamp = b.clone();
    maths::invert(&mut stamp);
    let (minimum, _) = maths::min_max(stamp.data());
    stamp = maths::clip_map(&stamp, minimum, 1.0);
    if boost > 0.0 {
        stamp = stamp.multiply_scalar(boost + 1.0);
    }
    let pressed = stamp.multiply_scalar(ratio);
    let embedded = a.subtract(&pressed);
    maths::lift_above_zero(embedded)
}

/// `Combiner.Insert`: flattens the finer detail of `terrain` under the
/// blurred footprint of `input` (where it rises above `threshold`), then
/// adds `input` on top.
pub fn insert(terrain: &Map, input: &Map, ratio: f32, threshold: f32, flatten: f32, boundary: f32) -> Map {
    let footprint = aspect::threshold(input, threshold);
    let footprint = fast_blur::blur_relative(&footprint, boundary * 3.0);
    let level = 1.0 - flatten;
    let flattened = profiles::graphic_eq(terrain, [1.0, 1.0, level, level, level, level, level]);
    let masked = masking::mask(terrain, &flattened, &footprint);
    let lifted = input.multiply_scalar(ratio);
    masked.add(&lifted)
}

/// `Combiner.Transpose`: adds the broad shapes of `reference` (low-shelf
/// filtered and dropped to zero) to `ground`, then stretches the sum back
/// over `ground`'s original range.
pub fn transpose(ground: &Map, reference: &Map, amount: f32) -> Map {
    let (ground_min, ground_max) = maths::min_max(ground.data());
    let filtered = filter_core::filter_process(reference, 0.75, 0.0, 0.0, FilterMode::LowShelf, true);
    let lowered = maths::drop_to_zero(&filtered);
    let mut combined = add(ground, &lowered, amount);
    let (combined_min, combined_max) = maths::min_max(combined.data());
    maths::remap(&mut combined, combined_max, combined_min, ground_max, ground_min);
    combined
}
