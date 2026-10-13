//! Blur and frequency nodes: Blur, Median, Sharpen, Denoise, Filter, GraphicEQ,
//! Peaks, SlopeBlur and SlopeWarp.

use nerthus_engine::{BuildContext, Map, NodeBuilder, NodeError, RenderIntent};

use crate::aspect::{self, CurvatureType};
use crate::fast_blur;
use crate::filter_core::{self, FilterMode};
use crate::maths;
use crate::median::{self, MedianQuality};
use crate::others;
use crate::profiles;
use crate::sat_blur;
use crate::slope_blur;

pub struct Blur;

impl NodeBuilder for Blur {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let radius = context.f32("Radius") * input.width() as f32;
        if input.channel_count() == 1 {
            let blurred = fast_blur::blur_map(&input, radius);
            return context.commit(blurred);
        }

        let mut channels = Vec::with_capacity(input.channel_count());
        for channel in 0..input.channel_count() {
            let plane = input.channel(channel).to_vec();
            let single = Map::from_data(maths::square_resolution(plane.len()), plane);
            let blurred = fast_blur::blur_map(&single, radius);
            channels.push(blurred.into_data());
        }
        context.render_as(RenderIntent::Color);
        let combined = Map::from_channels(input.resolution(), channels);
        context.commit(combined)
    }
}

pub struct Sharpen;

impl NodeBuilder for Sharpen {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let amount = context.f32("Amount");
        if context.enum_name("Method") == "Edge" {
            let curvature = aspect::curvature(&input, 0.0, 1.0, 0.2, CurvatureType::Vertical);
            let detail = curvature.multiply_scalar(0.008 * amount);
            let sharpened = input.add(&detail);
            return context.commit(sharpened);
        }
        let gains = [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0 + amount];
        let sharpened = profiles::graphic_eq(&input, gains);
        context.commit(sharpened)
    }
}

pub struct Denoise;

impl NodeBuilder for Denoise {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let amount = context.f32("Amount");
        let passes = context.i32("Passes");
        let kind = context.enum_name("Type").to_string();
        let mut map = (*input).clone();
        for _ in 0..passes {
            map = match kind.as_str() {
                "TwoPass" => {
                    let gains = [1.0, 1.0, 1.0, 1.0, 1.0, 1.0 - amount * 0.5, 1.0 - amount];
                    profiles::graphic_eq(&map, gains)
                }
                "Pixels" => others::hard_denoise(&map),
                _ => {
                    let gains = [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0 - amount];
                    profiles::graphic_eq(&map, gains)
                }
            };
        }
        context.commit(map)
    }
}

pub struct Filter;

impl NodeBuilder for Filter {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let mode = FilterMode::from_name(context.enum_name("Mode"));
        let frequency = context.f32("Frequency");
        let gain = context.f32("Gain");
        let q = context.f32("Q");
        let keep_dc = context.bool("KeepDC");
        let filtered = filter_core::filter_process(&input, frequency, gain, q, mode, keep_dc);
        context.commit(filtered)
    }
}

pub struct GraphicEq;

impl NodeBuilder for GraphicEq {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let gains = ["Band1", "Band2", "Band3", "Band4", "Band5", "Band6", "Band7"].map(|name| context.f32(name));
        let equalized = profiles::graphic_eq(&input, gains);
        context.commit(equalized)
    }
}

pub struct Peaks;

impl NodeBuilder for Peaks {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Mask);
        let input = context.require_input("In")?;
        let precise = context.bool("Precise");
        let fine = if precise { 2.0 } else { 0.0 };
        let finest = if precise { 1.0 } else { 0.0 };
        let gains = [0.0, 2.0, 2.0, 2.0, fine, finest, finest];
        let bands = profiles::graphic_eq(&input, gains);
        let mut peaks = maths::autolevel(bands);
        maths::strengthen(&mut peaks, context.f32("Falloff"));
        context.commit(peaks)
    }
}

/// SlopeBlur: blurs along the slope of a guide map.
pub struct SlopeBlur;

impl NodeBuilder for SlopeBlur {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let guide = context.require_input("Guide")?;
        let map = slope_blur::blur_map(
            &input,
            &guide,
            context.f32("Intensity"),
            context.i32("Iterations"),
            context.f32("Direction"),
            context.f32("Asymmetry"),
            context.bool("Normalized"),
            slope_blur::Window::from_name(context.enum_name("Window")),
            context.enum_value("Quality") as i32,
            context.enum_value("Antialiasing") as i32,
        );
        if map.channel_count() > 1 {
            context.render_as(RenderIntent::Color);
        }
        context.commit(map)
    }
}

/// SlopeWarp: moves the map along the slope of a guide map.
pub struct SlopeWarp;

impl NodeBuilder for SlopeWarp {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let guide = context.require_input("Guide")?;
        let map = slope_blur::warp_map(
            &input,
            &guide,
            context.f32("Intensity"),
            context.i32("Iterations"),
            context.f32("Direction"),
            context.bool("Normalized"),
            context.enum_value("Quality") as i32,
            context.enum_value("Antialiasing") as i32,
        );
        if map.channel_count() > 1 {
            context.render_as(RenderIntent::Color);
        }
        context.commit(map)
    }
}

/// VariableBlur: a disk blur whose radius `Input2` sets per pixel.
pub struct VariableBlur;

impl NodeBuilder for VariableBlur {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let radii = context.require_input("Input2")?;
        let size = context.resolution.x;
        let radius_scale = context.f32("Radius") * size as f32;
        let blurred = sat_blur::disk_blur(input.data(), radii.data(), size, radius_scale, 24);
        let resolution = maths::square_resolution(blurred.len());
        context.commit(Map::from_data(resolution, blurred))
    }
}

/// `MedianStrength`: how many passes a unit of strength stands for.
fn median_strength(name: &str) -> i32 {
    match name {
        "X2" => 2,
        "X4" => 4,
        "X8" => 8,
        _ => 1,
    }
}

/// Median: a smoothed median filter, the classic CPU windows or (the
/// default, Advanced2) the resolution-independent SYCL filter.
pub struct Median;

impl NodeBuilder for Median {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let strength = median_strength(context.enum_name("Strength"));
        let iterations = context.i32("Iterations");
        let failed = || NodeError::Failed("Index was outside the bounds of the array.".to_string());
        let filtered = match context.enum_name("Mode") {
            "Classic" => {
                let mut map = (*input).clone();
                for _ in 0..iterations * strength {
                    let width = map.width() as i32;
                    map = others::smoothed_median_blur(&map, 2, 2, width).ok_or_else(failed)?;
                }
                map
            }
            "Advanced" => {
                let horizontal = context.i32("Horizontal");
                let vertical = context.i32("Vertical");
                let mut map = (*input).clone();
                for _ in 0..iterations {
                    let width = map.width() as i32;
                    map = others::smoothed_median_blur(&map, horizontal, vertical, width).ok_or_else(failed)?;
                }
                map
            }
            _ => {
                let quality = MedianQuality::from_name(context.enum_name("Quality"));
                median::resolution_independent(&input, strength * iterations, quality)
            }
        };
        context.commit(filtered)
    }
}
