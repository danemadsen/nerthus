//! Level and range adjustments: Adjust, Autolevel, Clamp, Clip, Equalize,
//! Extend, SoftClip, Invert, Transpose, Match and Gamma.

use nerthus_engine::{BuildContext, Map, NodeBuilder, NodeError, RenderIntent};
use nerthus_image::quanta;

use crate::combiner;
use crate::histogram;
use crate::magick;
use crate::maths;

/// Every adjustment in one node, applied in a fixed order.
pub struct Adjust;

impl NodeBuilder for Adjust {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let mut map = (*input).clone();
        // The outputs present the data the way the input does.
        context.set_all_port_intents(map.info.render_intent);

        if context.bool("Invert") {
            maths::invert(&mut map);
        }
        if context.bool("Autolevel") {
            map = maths::autolevel(map);
        }
        if context.bool("Equalize") {
            map = histogram::equalize(map);
        }
        let [clamp_low, clamp_high] = context.float2("Clamp");
        if clamp_low > 0.0 || clamp_high < 1.0 {
            map = maths::fit_range(&map, clamp_low, clamp_high);
        }
        let [clip_low, clip_high] = context.float2("Clip");
        if clip_low > 0.0 || clip_high < 1.0 {
            map = maths::clip_map(&map, clip_low, clip_high);
        }
        let shaper = context.f32("Shaper");
        if shaper != 0.0 {
            maths::exponential_shape(map.data_mut(), shaper * -10.0);
            let shaped = map.data().to_vec();
            map = Map::from_data(maths::square_resolution(shaped.len()), shaped);
        }
        if context.bool("Strong") {
            maths::strengthen(&mut map, 0.5);
        }
        let multiply = context.f32("Multiply");
        if (multiply - 1.0).abs() > 0.0001 {
            map = map.multiply_scalar(multiply);
        }
        if context.bool("Drop") {
            map = maths::drop_to_zero(&map);
        }
        context.commit(map)
    }
}

pub struct Autolevel;

impl NodeBuilder for Autolevel {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let map = maths::autolevel((*input).clone());
        context.commit(map)
    }
}

/// Squeezes the map into a range, or stretches its actual range onto one.
pub struct Clamp;

impl NodeBuilder for Clamp {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let [low, high] = context.float2("Value");
        let drop = context.bool("Drop");
        let resolution = input.resolution();

        if context.enum_name("Mode") == "Normalized" {
            let mut data = input.data().to_vec();
            let (minimum, maximum) = maths::min_max(&data);
            maths::remap_plane(&mut data, maximum, minimum, high, low);
            if drop {
                data = maths::drop_plane(&data);
            }
            return context.commit(Map::from_data(resolution, data));
        }

        let mut map = maths::fit_range(&input, low, high);
        if drop {
            let dropped = maths::drop_plane(map.data());
            map = Map::from_data(maths::square_resolution(dropped.len()), dropped);
        }
        context.commit(map)
    }
}

pub struct Clip;

impl NodeBuilder for Clip {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let map = if context.bool("AutoClip") {
            let (minimum, maximum) = maths::min_max(input.data());
            maths::clip_map(&input, minimum, maximum)
        } else {
            maths::clip_map(&input, context.f32("Min"), context.f32("Max"))
        };
        context.commit(map)
    }
}

pub struct Equalize;

impl NodeBuilder for Equalize {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let equalized = histogram::equalize((*input).clone());
        let map = equalized.multiply_scalar(context.f32("Factor"));
        context.commit(map)
    }
}

/// Pushes values past the range ends to 0 and 1.
pub struct Extend;

impl NodeBuilder for Extend {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let [low, high] = context.float2("Value");
        let map = maths::extend(&input, low, high);
        context.commit(map)
    }
}

pub struct SoftClip;

impl NodeBuilder for SoftClip {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let above = context.enum_name("ClipMode") == "AboveThreshold";
        let threshold = context.f32("Threshold");
        let softness = context.f32("Softness");
        let clipping = context.f32("Clipping");
        let clipped = maths::soft_clip(input.data(), above, threshold, softness, clipping, None);
        let map = Map::from_data(maths::square_resolution(clipped.len()), clipped);
        context.commit(map)
    }
}

pub struct Invert;

impl NodeBuilder for Invert {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let mut map = (*input).clone();
        maths::invert(&mut map);
        context.commit(map)
    }
}

/// Transpose: lays the shapes of `Reference` onto the input (Transpose),
/// presses them in (Embed) or sets them on a flattened bed (Insert).
pub struct Transpose;

impl NodeBuilder for Transpose {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let ground = context.require_input("In")?;
        let reference = context.require_input("Reference")?;
        let amount = context.f32("Amount");
        let map = match context.enum_name("Mode") {
            "Embed" => combiner::embed(&ground, &reference, amount, context.f32("Extend")),
            "Insert" => {
                let threshold = context.f32("Threshold");
                let flatten = context.f32("Flatten");
                let boundary = context.f32("Boundary");
                combiner::insert(&ground, &reference, amount, threshold, flatten, boundary)
            }
            _ => combiner::transpose(&ground, &reference, amount),
        };
        context.commit(map)
    }
}

/// Match: gives the input the height range (Height) or the histogram
/// (Histogram) of `Reference`.
pub struct Match;

impl NodeBuilder for Match {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let map = context.require_input("In")?;
        let reference = context.require_input("Reference")?;
        let matched = if context.enum_name("Mode") == "Histogram" {
            histogram::match_histogram(&map, &reference)
        } else {
            let (reference_min, reference_max) = maths::min_max(reference.data());
            let (map_min, map_max) = maths::min_max(map.data());
            let mut matched = (*map).clone();
            maths::remap(&mut matched, map_max, map_min, reference_max, reference_min);
            matched
        };
        context.commit(matched)
    }
}

/// Gamma: ImageMagick's gamma correction, or the automatic gamma that
/// moves each channel's mean to 0.5.
pub struct Gamma;

impl NodeBuilder for Gamma {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let mut image = magick::to_image(&input);
        let intent = if input.channel_count() > 1 { RenderIntent::Color } else { RenderIntent::Heightfield };
        context.render_as(intent);
        if context.bool("Automatic") {
            quanta::auto_gamma(&mut image);
        } else {
            quanta::gamma(&mut image, context.f32("GammaValue") as f64);
        }
        context.commit(magick::to_map(&image))
    }
}
