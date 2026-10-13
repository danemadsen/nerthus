//! The node modifiers (`Modifiers`): steps applied, in `Order`, to a
//! node's result before it reaches `Out`. Missing values take the defaults.

use nerthus_engine::{BuildContext, Map, ModifierBuilder, ModifierData, NodeError, Registry};
use serde_json::Value;

use crate::aspect;
use crate::combiner;
use crate::fast_blur;
use crate::histogram;
use crate::masking;
use crate::maths;
use crate::transformer::EdgeBehaviour;
use crate::warps::{self, FractalWarpSettings, IterativeWarpMode, WarpNoiseType};

pub fn register(registry: &mut Registry) {
    registry.add_modifier("Autolevel", Autolevel);
    registry.add_modifier("Blur", Blur);
    registry.add_modifier("Clamp", Clamp);
    registry.add_modifier("Clip", Clip);
    registry.add_modifier("Difference", Difference);
    registry.add_modifier("Drop", Drop);
    registry.add_modifier("Equalize", Equalize);
    registry.add_modifier("Height", Height);
    registry.add_modifier("Influence", Influence);
    registry.add_modifier("Invert", Invert);
    registry.add_modifier("MatchHeight", MatchHeight);
    registry.add_modifier("Max", Max);
    registry.add_modifier("Min", Min);
    registry.add_modifier("Multiplier", Multiplier);
    registry.add_modifier("Shaper", Shaper);
    registry.add_modifier("Slope", Slope);
    registry.add_modifier("Strong", Strong);
    registry.add_modifier("Threshold", Threshold);
    registry.add_modifier("Warp", Warp);
}

fn f32_value(modifier: &ModifierData, name: &str, default: f32) -> f32 {
    modifier.values.get(name).and_then(Value::as_f64).map_or(default, |value| value as f32)
}

fn float2_value(modifier: &ModifierData, name: &str, default: [f32; 2]) -> [f32; 2] {
    let Some(value) = modifier.values.get(name) else {
        return default;
    };
    let member = |key: &str, fallback: f32| value.get(key).and_then(Value::as_f64).map_or(fallback, |member| member as f32);
    [member("X", default[0]), member("Y", default[1])]
}

/// The parent's input (`Parent.In.GetMap()`), else a zero map.
fn input_or_zeros(context: &BuildContext) -> Map {
    match context.input("In") {
        Some(input) => (*input).clone(),
        None => Map::new(context.resolution, 1),
    }
}

/// The heights a Height or Slope modifier measures: the parent's input,
/// or for a colour input the nearest heightfield up the chain.
fn measured_heights(context: &BuildContext) -> Map {
    let input = input_or_zeros(context);
    if input.channel_count() <= 1 {
        return input;
    }
    match context.underlay_heightfield() {
        Some(heights) => (*heights).clone(),
        None => Map::new(context.resolution, 1),
    }
}

/// The fractal warp the Height modifier's distortion and the Warp
/// modifier use.
fn modifier_warp(size: f32, strength: f32, edges: EdgeBehaviour, seed: i32) -> FractalWarpSettings {
    FractalWarpSettings {
        size,
        strength,
        persist_strength: true,
        z_coefficient: 0.0,
        noise_type: WarpNoiseType::PerlinFbm,
        perturbation: 0.5,
        octaves: 12,
        roughness: 0.5,
        normalized: false,
        iterations: 1,
        mode: IterativeWarpMode::Virtual,
        edges,
        seed,
        ..FractalWarpSettings::default()
    }
}

/// Autolevel: stretches the result to 0..1.
pub struct Autolevel;

impl ModifierBuilder for Autolevel {
    fn process(&self, _modifier: &ModifierData, map: Map, _context: &BuildContext) -> Result<Map, NodeError> {
        Ok(maths::autolevel(map))
    }
}

/// Blur: the result blurred by `Factor` fortieths of its width.
pub struct Blur;

impl ModifierBuilder for Blur {
    fn process(&self, modifier: &ModifierData, map: Map, _context: &BuildContext) -> Result<Map, NodeError> {
        let factor = f32_value(modifier, "Factor", 0.1);
        Ok(fast_blur::blur_relative(&map, factor))
    }
}

/// Clamp: squeezes 0..1 into `Value`.
pub struct Clamp;

impl ModifierBuilder for Clamp {
    fn process(&self, modifier: &ModifierData, map: Map, _context: &BuildContext) -> Result<Map, NodeError> {
        let [low, high] = float2_value(modifier, "Value", [0.0, 1.0]);
        Ok(maths::fit_range(&map, low, high))
    }
}

/// Clip: the clip to `Value`.
pub struct Clip;

impl ModifierBuilder for Clip {
    fn process(&self, modifier: &ModifierData, map: Map, _context: &BuildContext) -> Result<Map, NodeError> {
        let [low, high] = float2_value(modifier, "Value", [0.0, 1.0]);
        Ok(maths::clip_map(&map, low, high))
    }
}

/// Difference: the difference between the parent's input and its result.
pub struct Difference;

impl ModifierBuilder for Difference {
    fn process(&self, _modifier: &ModifierData, map: Map, context: &BuildContext) -> Result<Map, NodeError> {
        let input = context.input("In").ok_or_else(|| NodeError::Failed("Object reference not set to an instance of an object.".to_string()))?;
        Ok(combiner::difference(&input, &map, 1.0))
    }
}

/// Drop: lowers the result so its lowest point (capped at 1) is 0.
pub struct Drop;

impl ModifierBuilder for Drop {
    fn process(&self, _modifier: &ModifierData, map: Map, _context: &BuildContext) -> Result<Map, NodeError> {
        Ok(maths::drop_to_zero(&map))
    }
}

/// Equalize: histogram equalization.
pub struct Equalize;

impl ModifierBuilder for Equalize {
    fn process(&self, _modifier: &ModifierData, map: Map, _context: &BuildContext) -> Result<Map, NodeError> {
        Ok(histogram::equalize(map))
    }
}

/// Height: applies the result only within a height band of the parent's
/// input (softened by `Falloff`, optionally distorted), elsewhere keeping
/// the input; on a mask node it masks the result.
pub struct Height;

impl ModifierBuilder for Height {
    fn process(&self, modifier: &ModifierData, map: Map, context: &BuildContext) -> Result<Map, NodeError> {
        let [low, high] = float2_value(modifier, "Range", [0.45, 0.65]);
        let falloff = f32_value(modifier, "Falloff", 0.15);
        let distort = f32_value(modifier, "Distort", 0.0);
        let unchanged = if context.is_heightfield_now() { input_or_zeros(context) } else { Map::new(context.resolution, 1) };
        let heights = measured_heights(context);
        let mut band = aspect::height_normalized(&heights, low, high, falloff);
        if distort > 0.0 {
            let settings = modifier_warp(0.5, distort, EdgeBehaviour::Edge, context.node_id());
            band = warps::fractal_warp(&band, &settings, None, context.terrain_width(), context.terrain_height());
        }
        if context.renders_as_mask() {
            return Ok(map.multiply(&band));
        }
        Ok(masking::mask(&unchanged, &map, &band))
    }
}

/// Influence: blends the result with the parent's input by `Factor` (on a
/// mask node, scales it).
pub struct Influence;

impl ModifierBuilder for Influence {
    fn process(&self, modifier: &ModifierData, map: Map, context: &BuildContext) -> Result<Map, NodeError> {
        let factor = f32_value(modifier, "Factor", 1.0);
        if factor == 1.0 {
            return Ok(map);
        }
        let input = input_or_zeros(context);
        let scaled = map.multiply_scalar(factor);
        if context.renders_as_mask() {
            return Ok(scaled);
        }
        let kept = input.multiply_scalar(1.0 - factor);
        Ok(scaled.add(&kept))
    }
}

/// Invert: `1 - x`.
pub struct Invert;

impl ModifierBuilder for Invert {
    fn process(&self, _modifier: &ModifierData, mut map: Map, _context: &BuildContext) -> Result<Map, NodeError> {
        maths::invert(&mut map);
        Ok(map)
    }
}

/// MatchHeight: remaps the result onto the parent input's height range.
pub struct MatchHeight;

impl ModifierBuilder for MatchHeight {
    fn process(&self, _modifier: &ModifierData, mut map: Map, context: &BuildContext) -> Result<Map, NodeError> {
        let input = context.input("In").ok_or_else(|| NodeError::Failed("Object reference not set to an instance of an object.".to_string()))?;
        let (input_min, input_max) = maths::min_max(input.data());
        let (result_min, result_max) = maths::min_max(map.data());
        maths::remap(&mut map, result_max, result_min, input_max, input_min);
        Ok(map)
    }
}

/// Max: the higher of the parent's input and its result.
pub struct Max;

impl ModifierBuilder for Max {
    fn process(&self, _modifier: &ModifierData, map: Map, context: &BuildContext) -> Result<Map, NodeError> {
        match context.input("In") {
            Some(input) if context.is_connected("In") => Ok(combiner::max(&input, &map, 1.0)),
            _ => Ok(map),
        }
    }
}

/// Min: the lower of the parent's input and its result.
pub struct Min;

impl ModifierBuilder for Min {
    fn process(&self, _modifier: &ModifierData, map: Map, context: &BuildContext) -> Result<Map, NodeError> {
        match context.input("In") {
            Some(input) if context.is_connected("In") => Ok(combiner::min(&input, &map, 1.0)),
            _ => Ok(map),
        }
    }
}

/// Height Remap (`Multiplier`): squeezes 0..1 into `Value`, then clamps.
pub struct Multiplier;

impl ModifierBuilder for Multiplier {
    fn process(&self, modifier: &ModifierData, map: Map, _context: &BuildContext) -> Result<Map, NodeError> {
        let [low, high] = float2_value(modifier, "Value", [0.0, 1.0]);
        if low == 0.0 && high == 0.0 {
            return Ok(map);
        }
        let mut remapped = maths::fit_range(&map, low, high);
        maths::clamp(&mut remapped);
        Ok(remapped)
    }
}

/// Shaper: strengthens or weakens the result's profile.
pub struct Shaper;

impl ModifierBuilder for Shaper {
    fn process(&self, modifier: &ModifierData, mut map: Map, _context: &BuildContext) -> Result<Map, NodeError> {
        let shape = f32_value(modifier, "Shape", 0.0);
        maths::strengthen(&mut map, shape);
        Ok(map)
    }
}

/// Slope: applies the result only within a slope band of the parent's
/// input, elsewhere keeping the input; on a mask node it masks the result.
pub struct Slope;

impl ModifierBuilder for Slope {
    fn process(&self, modifier: &ModifierData, map: Map, context: &BuildContext) -> Result<Map, NodeError> {
        let [low, high] = float2_value(modifier, "Range", [0.0, 30.0]);
        let falloff = f32_value(modifier, "Falloff", 15.0);
        let input = input_or_zeros(context);
        let heights = measured_heights(context);
        let band = aspect::slope(&heights, low, high, falloff, context.terrain_width(), context.terrain_height());
        if context.renders_as_mask() {
            return Ok(map.multiply(&band));
        }
        Ok(masking::mask(&input, &map, &band))
    }
}

/// Strong: autolevels, then strengthens fully.
pub struct Strong;

impl ModifierBuilder for Strong {
    fn process(&self, _modifier: &ModifierData, map: Map, _context: &BuildContext) -> Result<Map, NodeError> {
        let mut leveled = maths::autolevel(map);
        maths::strengthen(&mut leveled, 1.0);
        Ok(leveled)
    }
}

/// Threshold: 1 above `Level`, else 0.
pub struct Threshold;

impl ModifierBuilder for Threshold {
    fn process(&self, modifier: &ModifierData, map: Map, _context: &BuildContext) -> Result<Map, NodeError> {
        let level = f32_value(modifier, "Level", 0.5);
        Ok(aspect::threshold(&map, level))
    }
}

/// Warp: each channel fractal-warped, seeded by the parent node.
pub struct Warp;

impl ModifierBuilder for Warp {
    fn process(&self, modifier: &ModifierData, map: Map, context: &BuildContext) -> Result<Map, NodeError> {
        let size = f32_value(modifier, "Size", 0.5);
        let strength = f32_value(modifier, "Strength", 0.5);
        let settings = modifier_warp(size, strength, EdgeBehaviour::Mirror, context.node_id());
        let resolution = map.resolution();
        let channels: Vec<Vec<f32>> = (0..map.channel_count())
            .map(|channel| {
                let plane = Map::from_data(resolution, map.channel(channel).to_vec());
                let warped = warps::fractal_warp(&plane, &settings, None, context.terrain_width(), context.terrain_height());
                warped.into_data()
            })
            .collect();
        Ok(Map::from_channels(resolution, channels))
    }
}
