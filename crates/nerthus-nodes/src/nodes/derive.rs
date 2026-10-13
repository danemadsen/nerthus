//! Data maps derived from the heightfield: Height, Threshold, Angle,
//! Slope, Curvature, Normals, RockMap, Occlusion, FlowMapClassic, Soil
//! and Accumulator.

use nerthus_engine::{BuildContext, Map, NodeBuilder, NodeError, RenderIntent};

use crate::aspect::{self, CurvatureType, Handedness, UpAxis};
use crate::colour_library;
use crate::filter_core::{self, FilterMode};
use crate::flow::{self, FlowSettings};
use crate::histogram;
use crate::landscapes::{self, Abc, MountainBulk, MountainRangeSettings, MountainRangeStyle, RidgeSettings};
use crate::maths;
use crate::profiles::{self, FoldWaveform};
use crate::simulations;
use crate::spline::Polyline;
use crate::texturize::{self, TextureBaseSettings};

/// `Node.RenderAsMask()`.
fn render_as_mask(context: &mut BuildContext) {
    context.render_as(RenderIntent::Mask);
}

pub struct Height;

impl NodeBuilder for Height {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        render_as_mask(context);
        let input = context.require_input("In")?;
        let [min, max] = context.float2("Range");
        let map = aspect::height_mask(&input, min, max, context.f32("Falloff"));
        context.commit(map)
    }
}

pub struct Threshold;

impl NodeBuilder for Threshold {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let map = aspect::threshold(&input, context.f32("Level"));
        context.commit(map)
    }
}

pub struct Angle;

impl NodeBuilder for Angle {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        render_as_mask(context);
        let input = context.require_input("In")?;
        let [min, max] = context.float2("Range");
        let map = aspect::angle(&input, context.f32("Azimuth"), min, max, context.f32("Falloff"));
        context.commit(map)
    }
}

pub struct Slope;

impl NodeBuilder for Slope {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        render_as_mask(context);
        let input = context.require_input("In")?;
        let [min, max] = context.float2("Range");
        let falloff = context.f32("Falloff");
        let width = context.terrain_width();
        let height = context.terrain_height();
        let map = match context.enum_name("Type") {
            "Normalized" => aspect::slope_normalized(&input, min / 90.0, max / 90.0, falloff / 90.0),
            _ => {
                let micro_accent = context.f32("MicroAccent");
                if micro_accent > 0.0 {
                    aspect::slope_accented(&input, micro_accent, min, max, falloff, width, height)
                } else {
                    aspect::slope(&input, min, max, falloff, width, height)
                }
            }
        };
        context.commit(map)
    }
}

pub struct Curvature;

impl NodeBuilder for Curvature {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        render_as_mask(context);
        let input = context.require_input("In")?;
        let [min, max] = context.float2("Range");
        let kind = match context.enum_name("Type") {
            "Vertical" => CurvatureType::Vertical,
            "Average" => CurvatureType::Average,
            _ => CurvatureType::Horizontal,
        };
        let map = aspect::curvature(&input, min, max, context.f32("Falloff"), kind);
        context.commit(map)
    }
}

pub struct Normals;

impl NodeBuilder for Normals {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.set_port_intent("Out", RenderIntent::Color);
        let input = context.require_input("In")?;
        let handedness = if context.enum_name("Handedness") == "Right" { Handedness::Right } else { Handedness::Left };
        let up_axis = if context.enum_name("UpAxis") == "YUp" { UpAxis::YUp } else { UpAxis::ZUp };
        let width = context.f32("Width");
        let height = context.f32("Height");
        let terrain_width = context.terrain_width();
        let terrain_height = context.terrain_height();
        let map = match context.enum_name("Type") {
            "Detail" => {
                // A high pass keeps the features smaller than `DetailSize`,
                // and the smaller they are, the more they are raised.
                let detail_size = context.f32("DetailSize");
                let frequency = 0.999 * (1.0 - detail_size);
                let detail = filter_core::filter_process(&input, frequency, 1.0, 0.5, FilterMode::HighPass, false);
                let boost = 3.0 / (detail_size * 99.0 + 1.0) + 0.97;
                let boosted = detail.multiply_scalar(boost);
                aspect::normal_map(&boosted, width, height, handedness, up_axis, terrain_width, terrain_height)
            }
            _ => aspect::normal_map(&input, width, height, handedness, up_axis, terrain_width, terrain_height),
        };
        context.commit(map)
    }
}

pub struct RockMap;

impl NodeBuilder for RockMap {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        render_as_mask(context);
        let input = context.require_input("In")?;
        let map = aspect::rock_map(&input, context.f32("Coverage"), context.f32("Density"), context.terrain_height());
        context.commit(map)
    }
}

pub struct Occlusion;

impl NodeBuilder for Occlusion {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        render_as_mask(context);
        let input = context.require_input("In")?;
        let invert = context.node.version() > 0;
        let map = aspect::occlusion(&input, context.f32("Strength"), context.i32("Octaves"), invert);
        context.commit(map)
    }
}

/// FlowMapClassic: the classic pipe-model flow map at up to four scales.
pub struct FlowMapClassic;

impl NodeBuilder for FlowMapClassic {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Mask);
        let input = context.require_input("In")?;
        let multiplier = if context.bool("Simulate2X") { 2 } else { 1 };
        let rain = context.f32("Rainfall") * 0.0005;
        let enhance = context.f32("Enhance");
        let mut flow = Map::new(context.resolution, 1);
        let scales = [("Primary", 200), ("Secondary", 80), ("Tertiary", 20), ("Quaternary", 6)];
        for (switch, iterations) in scales {
            if !context.bool(switch) {
                continue;
            }
            let mut speed = maths::autolevel(simulations::create_flow_map(&input, iterations * multiplier, rain, false));
            maths::strengthen(&mut speed, enhance);
            flow = flow.add(&speed);
        }
        let flow = maths::autolevel(flow);
        context.commit(flow)
    }
}

/// Soil: where blurred copies rise above the map (`Texturize.Soil2`).
pub struct Soil;

impl NodeBuilder for Soil {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Mask);
        let input = context.require_input("In")?;
        let mut soil = texturize::soil2(&input, context.f32("Amount"));
        maths::strengthen(&mut soil, context.f32("Bias"));
        context.commit(soil)
    }
}

/// Accumulator: the maximum of the snow, water, debris or tree outputs of
/// every built node (`Texturize.GetAccumulation`).
pub struct Accumulator;

impl NodeBuilder for Accumulator {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Mask);
        let (kind, port, rivers_port) = match context.enum_name("Type") {
            "SnowDepth" => ("Snow", "Depth", None),
            "Rivers" => ("River", "Depth", None),
            "WaterMask" => ("Water", "Water", Some("Rivers")),
            "WaterDepth" => ("Water", "Depth", None),
            "ShoreMask" => ("Shore", "Shore", None),
            "DebrisMask" => ("Debris", "Debris", None),
            "DebrisColor" => ("Debris", "ColorIndex", None),
            "Trees" => ("Trees", "Trees", None),
            "TreeDeadZones" => ("Trees", "DeadZones", None),
            "TreeFreshwater" => ("Trees", "FreshWater", None),
            _ => ("Snow", "Snow", None),
        };
        let group = if context.bool("RestrictToGroup") { context.node_group() } else { 0 };
        let accumulation = context.accumulation(kind, port, rivers_port, group);
        context.commit(accumulation)
    }
}

/// Flow (`FlowMap`): where water runs over the terrain, from the particle
/// flow simulation.
pub struct FlowMap;

impl NodeBuilder for FlowMap {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let precipitation = context.input("Precipitation");
        let settings = FlowSettings {
            flow_length: context.f32("FlowLength"),
            flow_volume: context.f32("FlowVolume"),
            seed: context.i32("Seed"),
        };
        let flow = flow::simulate(&input, precipitation.as_deref(), &settings);
        context.set_port_intent("Out", RenderIntent::Mask);
        context.commit(flow)
    }
}

/// The red channel of `Rock` palette 117 as a curve over 0..1, which
/// TextureBase's new accentuation maps soil through.
fn rock_accent_curve() -> Polyline {
    let palette = colour_library::palette("Rock", 117).expect("the colour library has Rock palette 117");
    let count = palette.len() as f32;
    let points: Vec<[f32; 2]> = palette
        .iter()
        .enumerate()
        .map(|(index, rgb)| [index as f32 / count, rgb[0] as i32 as f32 / 255.0])
        .collect();
    Polyline::from_unsorted(&points)
}

/// TextureBase: the base texture mask (`Texturize.TextureBase`), with
/// optional peaks, accentuation and enhancement.
pub struct TextureBase;

impl NodeBuilder for TextureBase {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Mask);
        let input = context.require_input("In")?;
        let guide = context.input("Guide");
        let settings = TextureBaseSettings {
            slope: context.f32("Slope"),
            scale: context.f32("Scale"),
            soil: context.f32("Soil"),
            chaos: context.f32("Chaos"),
            patches: context.f32("Patches"),
            seed: context.i32("Seed"),
            old_slopes: context.node.version() < 3,
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let base = texturize::texture_base(&input, guide.as_deref(), &settings, terrain);
        let mut texture = maths::autolevel(base);

        let peaks = context.f32("Peaks");
        if peaks > 0.0 {
            let equalized = profiles::graphic_eq(&input, [0.0, 2.0, 2.0, 2.0, 2.0, 1.0, 1.0]);
            let peak_map = maths::autolevel(equalized).multiply_scalar(peaks);
            texture = texture.add(&peak_map);
            maths::clamp_if_out_of_range(&mut texture);
        }

        match context.enum_name("Accentuation") {
            "Old" => {
                let curvature = aspect::curvature(&input, 0.0, 1.0, 0.1, CurvatureType::Average);
                let accent = curvature.multiply_scalar(0.5);
                texture = texture.add(&accent);
            }
            "New" => {
                let curve = rock_accent_curve();
                let mut soil = texturize::soil2(&input, 0.000_999_999_9);
                maths::strengthen(&mut soil, 0.5);
                let soil = histogram::equalize(soil);
                let accent = soil.map_values(|height| curve.interpolate(height));
                let accent = accent.multiply_scalar(0.33);
                texture = texture.add(&accent);
                maths::clamp_if_out_of_range(&mut texture);
            }
            _ => {}
        }

        match context.enum_name("Enhance") {
            "Autolevel" => texture = maths::autolevel(texture),
            "Equalize" => texture = histogram::equalize(texture),
            _ => {}
        }
        if context.bool("Reverse") {
            maths::invert(&mut texture);
        }
        context.commit(texture)
    }
}

/// `low + t × (high − low)`.
fn lerp(t: f32, low: f32, high: f32) -> f32 {
    low + t * (high - low)
}

/// Texturizer: TextureBase folded through one of twelve quick patterns.
pub struct Texturizer;

impl NodeBuilder for Texturizer {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Mask);
        let input = context.require_input("In")?;
        let seed = context.i32("Seed");
        let settings = TextureBaseSettings {
            slope: context.f32("Slope"),
            scale: context.f32("Flows"),
            soil: context.f32("Soil"),
            chaos: 0.5,
            patches: 1.0,
            seed,
            old_slopes: context.node.version() < 2,
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let base = texturize::texture_base(&input, None, &settings, terrain);

        let factor = context.f32("Factor");
        let size = context.resolution.x;
        let scale = lerp(factor, 0.4, 0.75);
        let (pattern, folds) = match context.enum_name("Style") {
            "A" => (landscapes::slump(size, scale, Abc::A, Abc::A, seed + 5, terrain), lerp(factor, 0.02, 0.8)),
            "B" => (landscapes::slump(size, scale, Abc::A, Abc::A, seed + 25, terrain), lerp(factor, 0.05, 1.8)),
            "C" => (landscapes::slump(size, scale, Abc::A, Abc::A, seed + 2, terrain), lerp(factor, 0.5, 3.0)),
            "D" | "E" => {
                let ridge_seed = if context.enum_name("Style") == "D" { seed + 2 } else { seed + 6 };
                let ridge_settings = RidgeSettings {
                    scale,
                    height: 0.6,
                    definition: 0.4,
                    seed: ridge_seed,
                    scale_x: 1.0,
                    scale_y: 1.0,
                };
                let high = if context.enum_name("Style") == "D" { 3.0 } else { 2.0 };
                (landscapes::ridge(size, &ridge_settings, terrain), lerp(factor, 0.02, high))
            }
            "F" => {
                let range_settings = MountainRangeSettings {
                    scale,
                    height: 0.5,
                    style: MountainRangeStyle::Eroded,
                    bulk: MountainBulk::Medium,
                    seed: seed + 6,
                };
                (landscapes::mountain_range(context.resolution, &range_settings, terrain), lerp(factor, 0.02, 0.75))
            }
            "G" => (landscapes::surface_m2(size, lerp(factor, 0.6, 0.75), seed + 6, terrain), lerp(factor, 0.02, 0.75)),
            "H" => (landscapes::surface_m2(size, lerp(factor, 0.26, 0.45), seed + 6, terrain), lerp(factor, 0.02, 0.75)),
            "I" => (landscapes::surface_m4(size, lerp(factor, 0.16, 0.45), seed + 6, terrain), lerp(factor, 0.02, 0.75)),
            "J" => (landscapes::surface_m3(size, lerp(factor, 0.46, 0.95), seed + 6, terrain), lerp(factor, 0.02, 0.75)),
            "K" => (landscapes::surface_m5(size, lerp(factor, 0.26, 0.95), seed + 6, terrain), lerp(factor, 0.02, 0.75)),
            "L" => (landscapes::surface_m6(size, lerp(factor, 0.26, 0.85), seed + 6, terrain), lerp(factor, 0.02, 0.75)),
            _ => return Err(NodeError::Failed("Specified argument was out of the range of valid values.".to_string())),
        };
        let mut texture = profiles::fold(&base, Some(&pattern), FoldWaveform::Sine, folds, false);

        let secondary = context.f32("Secondary");
        if secondary > 0.0 {
            texture = profiles::fold(&texture, Some(&texture), FoldWaveform::Sine, secondary, false);
        }
        context.commit(texture)
    }
}
