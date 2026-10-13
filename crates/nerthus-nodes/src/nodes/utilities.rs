//! Utility and colour nodes: RGBSplit, RGBMerge, Splat, Tint, HSL, CLUTer,
//! Synth, WaterColor and DataExtractor.

use nerthus_engine::automation::{VarType, VariableValue};
use nerthus_engine::{BuildContext, Map, NodeBuilder, NodeError, RenderIntent};
use serde_json::{Value, json};

use crate::aspect;
use crate::colour;
use crate::combiner;
use crate::fast_blur;
use crate::histogram;
use crate::lighting;
use crate::magick;
use crate::masking;
use crate::maths::{self, GrayscaleConversion};
use crate::noise::{self, WaveShineSettings};
use crate::profiles;
use crate::spline::Polyline;
use crate::transformer::EdgeBehaviour;
use crate::warps::{self, FractalWarpSettings, IterativeWarpMode, WarpNoiseType};
use crate::x86;

/// Splits a colour map into its channels plus a grayscale version.
pub struct RgbSplit;

impl NodeBuilder for RgbSplit {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Mask);
        for port in ["R", "G", "B"] {
            context.set_port_intent(port, RenderIntent::Mask);
        }
        let Some(input) = context.input("In") else {
            return Ok(());
        };
        if input.channel_count() < 3 {
            return Ok(());
        }
        let conversion = GrayscaleConversion::from_name(context.enum_name("Method"));
        let mut gray = maths::grayscale(&input, conversion);
        let autolevel = context.bool("AutolevelChannels");
        if autolevel {
            gray = maths::autolevel(gray);
        }
        for (channel, port) in ["R", "G", "B"].into_iter().enumerate() {
            let mut plane = Map::from_data(maths::square_resolution(input.channel(channel).len()), input.channel(channel).to_vec());
            if autolevel {
                plane = maths::autolevel(plane);
            }
            context.commit_port(port, plane, RenderIntent::Default);
        }
        context.commit(gray)
    }
}

/// Tint: colours the map along a two-colour gradient
/// (`Migrated.TintProcess`).
pub struct Tint;

impl NodeBuilder for Tint {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Color);
        let input = context.require_input("In")?;
        let mut map = maths::grayscale(&input, GrayscaleConversion::RedChannel);
        match context.enum_name("Enhance") {
            "Autolevel" => map = maths::autolevel(map),
            "Equalize" => map = histogram::equalize(map),
            _ => {}
        }
        if context.bool("Reverse") {
            maths::invert(&mut map);
        }
        let start = context.color("Start");
        let end = context.color("End");
        let mut tinted = colour::colorize(&map, &[[start[0], start[1], start[2]], [end[0], end[1], end[2]]]);
        let hue = context.f32("Hue");
        let saturation = context.f32("Saturation");
        let lightness = context.f32("Lightness");
        if hue != 0.0 || saturation != 0.0 || lightness != 0.0 {
            tinted = colour::hsl(&tinted, hue, saturation, lightness);
        }
        context.commit(tinted)
    }
}

/// CLUTer: colours the map through a three-channel gradient
/// (`Migrated.ClutterProcess`).
pub struct Cluter;

impl NodeBuilder for Cluter {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let Some(gradient) = context.float4_list("GradientData") else {
            return Ok(());
        };
        context.render_as(RenderIntent::Color);
        let input = context.require_input("In")?;
        let reverse = context.bool("Reverse");
        let channels: Vec<Polyline> = (0..3)
            .map(|channel| {
                let points: Vec<[f32; 2]> = gradient
                    .iter()
                    .map(|stop| {
                        let position = if reverse { 1.0 - stop[3] } else { stop[3] };
                        [position, stop[channel]]
                    })
                    .collect();
                Polyline::from_unsorted(&points)
            })
            .collect();
        let mut heights = maths::grayscale(&input, GrayscaleConversion::RedChannel);
        if context.bool("AutolevelInput") {
            heights = maths::autolevel(heights);
        }
        let bias = context.f32("Bias");
        if bias != 0.0 {
            maths::strengthen(&mut heights, bias);
        }
        let planes: Vec<Vec<f32>> = channels
            .iter()
            .map(|polyline| heights.data().iter().map(|&height| polyline.interpolate(height)).collect())
            .collect();
        let mut coloured = Map::from_channels(heights.resolution(), planes);
        let hue = context.f32("Hue");
        let saturation = context.f32("Saturation");
        let lightness = context.f32("Lightness");
        if hue != 0.0 || saturation != 0.0 || lightness != 0.0 {
            coloured = colour::hsl(&coloured, hue, saturation, lightness);
        }
        context.commit(coloured)
    }
}

/// `ProcessInput`: Autolevel or Equalize, by name.
fn enhance(map: Map, process: &str) -> Map {
    match process {
        "Autolevel" => maths::autolevel(map),
        "Equalize" => histogram::equalize(map),
        _ => map,
    }
}

/// HSL: shifts the hue and scales the saturation and lightness of a colour
/// map.
pub struct Hsl;

impl NodeBuilder for Hsl {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Color);
        let input = context.require_input("In")?;
        if input.channel_count() < 3 {
            return Err(NodeError::Failed("HSL needs a colour input.".to_string()));
        }
        let hue = context.f32("Hue");
        let saturation = context.f32("Saturation");
        let lightness = context.f32("Lightness");
        let map = colour::hsl(&input, hue, saturation, lightness);
        context.commit(map)
    }
}

/// RGBMerge: three maps as the channels of a colour map, each optionally
/// autolevelled or equalized first.
pub struct RgbMerge;

impl NodeBuilder for RgbMerge {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Color);
        let red = context.require_input("In")?;
        let green = context.require_input("In2")?;
        let blue = context.require_input("In3")?;
        let red = enhance((*red).clone(), context.enum_name("EnhanceR"));
        let green = enhance((*green).clone(), context.enum_name("EnhanceG"));
        let blue = enhance((*blue).clone(), context.enum_name("EnhanceB"));
        let merged = Map::stack(&[&red, &green, &blue]);
        context.commit(merged)
    }
}

/// Splat: three or four maps stacked as splat-map channels. With
/// Autolevel each is levelled and weighted. ClampProduct normalizes a map
/// that is never filled and then commits nothing, so it leaves no output.
pub struct Splat;

impl NodeBuilder for Splat {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Color);
        let mut red = (*context.require_input("In")?).clone();
        let mut green = (*context.require_input("In2")?).clone();
        let mut blue = (*context.require_input("In3")?).clone();
        let mut alpha = context.input("In4").map(|map| (*map).clone());
        if context.bool("Autolevel") {
            red = maths::autolevel(red).multiply_scalar(context.f32("R"));
            green = maths::autolevel(green).multiply_scalar(context.f32("G"));
            blue = maths::autolevel(blue).multiply_scalar(context.f32("B"));
            let alpha_weight = context.f32("A");
            alpha = alpha.map(|map| maths::autolevel(map).multiply_scalar(alpha_weight));
        }
        if context.bool("ClampProduct") {
            return Ok(());
        }
        let merged = match &alpha {
            Some(alpha) => Map::stack(&[&red, &green, &blue, alpha]),
            None => Map::stack(&[&red, &green, &blue]),
        };
        context.commit(merged)
    }
}

/// Synth: colours a height map through the gradient sampled from an image
/// (`Data`, filled by the Synthesize Colors command).
pub struct Synth;

/// `SynthSamples`: how many colours Synthesize Colors takes.
pub fn synth_sample_count(samples: &str) -> Option<usize> {
    let count = match samples {
        "_8" => 8,
        "_16" => 16,
        "_24" => 24,
        "_36" => 36,
        "_64" => 64,
        "_128" => 128,
        "_256" => 256,
        "_1024" => 1024,
        "_2048" => 2048,
        _ => return None,
    };
    Some(count)
}

/// `Synth.ExtractSamples` (the Synthesize Colors command): the input
/// turned by `offset` of a full turn (mirrored at the edges), then
/// `Synth.GetSamples`: `samples` colours left to right along the middle
/// row, each moved down by up to half the height times `fuzzy`, as
/// (red, green, blue, position).
pub fn synth_samples(input: &Map, samples: usize, fuzzy: f32, offset: f32, seed: i32) -> Vec<[f32; 4]> {
    let rotated;
    let map = if offset > 0.0 {
        rotated = crate::transformer::rotate(input, 360.0 * offset, false, EdgeBehaviour::Mirror, true);
        &rotated
    } else {
        input
    };
    let width = map.width();
    let height = map.height();
    let channels = map.channel_count();
    let red = map.channel(0);
    let green = map.channel(1.min(channels - 1));
    let blue = map.channel(2.min(channels - 1));
    let mut random = crate::random::XorShift64Star::new(seed);
    let mut colours = Vec::with_capacity(samples);
    for index in 0..samples {
        let position = index as f32 / samples as f32;
        let x = (position * width as f32) as i32 as usize;
        let lift = random.next_float() * 0.5 * fuzzy;
        let y = ((0.5 + lift) * height as f32) as i32 as usize;
        let cell = y * width + x;
        colours.push([red[cell], green[cell], blue[cell], position]);
    }
    colours
}

impl NodeBuilder for Synth {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Color);
        let height = match context.input("Height") {
            Some(map) => map,
            None => context.underlay_heightfield().ok_or_else(|| NodeError::Validation("Required port [Height] not connected.".to_string()))?,
        };
        let Some(samples) = context.float4_list("Data") else {
            let message = "Connect a Bitmap or Color Input and use the Synthesize Colors command to syntehsize colors.";
            return Err(NodeError::Failed(message.to_string()));
        };
        // Each colour line runs over every channel of the height map
        // (`Map.Generate`), and the results are stacked.
        let mut channels = Vec::new();
        for component in 0..3 {
            let points: Vec<[f32; 2]> = samples.iter().map(|sample| [sample[3], sample[component]]).collect();
            let line = Polyline::from_unsorted(&points);
            for channel in 0..height.channel_count() {
                let plane: Vec<f32> = height.channel(channel).iter().map(|&value| line.interpolate(value)).collect();
                channels.push(plane);
            }
        }
        let coloured = Map::from_channels(height.resolution(), channels);
        context.commit(coloured)
    }
}

/// The (shallow, deep) colours of a `WaterColors` palette, or `None` for
/// Custom.
fn water_palette(name: &str) -> Option<([f32; 3], [f32; 3])> {
    // `new Color("#rrggbb")`.
    let hex = |red: u8, green: u8, blue: u8| [red as f32 / 255.0, green as f32 / 255.0, blue as f32 / 255.0];
    let palette = match name {
        "Generic" => ([0.271, 0.749, 1.0], [0.171, 0.349, 0.6]),
        "Jungle" => ([0.0561, 0.1095, 0.0723], [0.184, 0.258, 0.097]),
        "Oasis" => (hex(0x8b, 0xa5, 0x7b), hex(0x44, 0x84, 0x6e)),
        "Dirty" => ([0.0222, 0.0356, 0.008], [0.0931, 0.1413, 0.0823]),
        "Murky" => ([0.1095, 0.227, 0.1714], [0.0176, 0.0307, 0.0242]),
        "OceanGreen" => ([120.0 / 255.0, 195.0 / 255.0, 214.0 / 255.0], [22.0 / 255.0, 65.0 / 255.0, 74.0 / 255.0]),
        "OceanBlue" => (hex(0x22, 0x4f, 0x7b), hex(0x15, 0x19, 0x22)),
        "Muddy" => (hex(0x77, 0x6f, 0x5e), hex(0x30, 0x52, 0x52)),
        _ => return None,
    };
    Some(palette)
}

/// A `Color` member as projects save it.
fn colour_value(rgb: [f32; 3]) -> Value {
    json!({ "R": rgb[0], "G": rgb[1], "B": rgb[2] })
}

/// WaterColor: colours standing water and rivers from the accumulated
/// water depth (or a guide node's `Depth`), shallow to deep, and lays it
/// over the input where there is water.
pub struct WaterColor;

impl NodeBuilder for WaterColor {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let (shallow, deep) = match water_palette(context.enum_name("Palette")) {
            Some((shallow, deep)) => {
                context.update_member("Shallow", colour_value(shallow));
                context.update_member("Deep", colour_value(deep));
                (shallow, deep)
            }
            None => {
                let shallow = context.color("Shallow");
                let deep = context.color("Deep");
                ([shallow[0], shallow[1], shallow[2]], [deep[0], deep[1], deep[2]])
            }
        };
        context.render_as(RenderIntent::Color);
        context.render_secondary_as(RenderIntent::Mask);
        let stops = if context.bool("Reverse") { [deep, shallow] } else { [shallow, deep] };
        let resolution = context.resolution;

        let mut water = if context.enum_name("Source") == "Port" {
            if !context.is_connected("Guide") {
                return Ok(());
            }
            let Some(depth) = context.source_output("Guide", "Depth") else {
                return Err(NodeError::Failed("The Guide node has no Depth output.".to_string()));
            };
            let depth = maths::autolevel((*depth).clone());
            context.commit_port("Rivers", Map::new(resolution, 1), RenderIntent::Default);
            context.commit_port("Stillwater", Map::new(resolution, 1), RenderIntent::Default);
            depth
        } else {
            let group = if context.bool("RestrictToGroup") { context.node_group() } else { 0 };
            let mut stillwater = maths::autolevel(context.accumulation("Water", "Depth", None, group));
            maths::strengthen(&mut stillwater, context.f32("Stillwater"));
            context.commit_port("Stillwater", stillwater.clone(), RenderIntent::Default);
            let rivers_bias = context.f32("Rivers");
            if rivers_bias != 0.0 {
                let mut rivers = maths::autolevel(context.accumulation("River", "Depth", None, group));
                maths::strengthen(&mut rivers, rivers_bias);
                context.commit_port("Rivers", rivers.clone(), RenderIntent::Default);
                combiner::max(&stillwater, &rivers, 1.0)
            } else {
                context.commit_port("Rivers", Map::new(resolution, 1), RenderIntent::Default);
                stillwater
            }
        };

        let mut biased = water.clone();
        maths::strengthen(&mut biased, context.f32("MaskBias"));
        let mask = aspect::threshold(&biased, 0.0001);
        context.commit_port("M\u{0251}sk", mask.clone(), RenderIntent::Default);

        let terrain = (context.terrain_width(), context.terrain_height());
        if context.bool("Refraction") {
            let blurred = fast_blur::blur_relative(&water, 0.2);
            let refraction = FractalWarpSettings {
                size: 0.04,
                strength: 0.3,
                iterations: 4,
                seed: context.node_id(),
                ..FractalWarpSettings::default()
            };
            water = warps::fractal_warp(&blurred, &refraction, None, terrain.0, terrain.1);
        }
        maths::strengthen(&mut water, context.f32("ColorBias"));
        let mut coloured = colour::colorize(&water, &stops);
        let hue = context.f32("Hue");
        let saturation = context.f32("Saturation");
        let lightness = context.f32("Lightness");
        if hue != 0.0 || saturation != 0.0 || lightness != 0.0 {
            coloured = colour::hsl(&coloured, hue, saturation, lightness);
        }
        if context.bool("Waves") {
            let waves = WaveShineSettings {
                size: 0.025,
                wind_patch_intensity: 0.1,
                ..WaveShineSettings::default()
            };
            let shine = noise::wave_shine(water.width(), &waves, terrain);
            coloured = coloured.add(&shine);
        }
        context.commit_port("Color", coloured.clone(), RenderIntent::Default);

        let input = context.require_input("In")?;
        let result = masking::mask(&input, &coloured, &mask);
        context.commit(result)
    }
}

/// Sunlight: the share of daylight each point receives over part of the
/// year, with terrain shadows (the sunlight integral).
pub struct Sunlight;

impl NodeBuilder for Sunlight {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Mask);
        let input = context.require_input("In")?;
        let [start, end] = context.float2("Year");
        let settings = lighting::SunlightSettings {
            day_begin: x86::cvttss2si(start * 360.0),
            day_end: x86::cvttss2si(end * 360.0),
            latitude: context.i32("Latitude") as f32,
            north: context.f32("North"),
            air_density: 0.01,
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let light = lighting::sunlight_integral(&input, &settings, terrain);
        context.commit(light)
    }
}

/// Heal: ImageMagick's wavelet denoise, `Passes` times, removing detail
/// below the `Reconstruction` threshold.
pub struct Heal;

impl NodeBuilder for Heal {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let mut image = magick::to_image(&input);
        // Magick.NET's percentage of the quantum range, in single precision.
        let percentage = (context.f32("Reconstruction") * 0.5) as f64;
        let threshold = (65535.0 * (percentage / 100.0)) as f32;
        for _ in 0..context.i32("Passes") {
            image = nerthus_image::filters::wavelet_denoise(&image, threshold as f64, 0.0);
        }
        context.commit(magick::to_map(&image))
    }
}

/// BlobRemover: merges small flat patches into the patch they border most
/// (ImageMagick's connected components with mean colours), optionally after
/// terracing the input into `Quantization` levels so patches exist at all.
pub struct BlobRemover;

impl NodeBuilder for BlobRemover {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let source = if context.bool("EnforceQuantization") {
            let levels = context.i32("Quantization").max(0) as usize;
            profiles::terrace(&input, levels, 1.0, 1.0, 1.0, 5, false)
        } else {
            (*input).clone()
        };
        let image = magick::to_image(&source);
        let area_threshold = (context.resolution.x as f32 * context.f32("Range")) as f64;
        let connectivity = context.i32("Connectivity").max(0) as usize;
        let Ok(components) = nerthus_image::quanta::connected_components(&image, connectivity, Some(area_threshold), true) else {
            return Err(NodeError::Failed("Reduce complexity or choose Enforce Quantization.".to_string()));
        };
        let gray = nerthus_image::quanta::grayscale(&components);
        context.commit(magick::to_map(&gray))
    }
}

/// ColorThreshold: a mask of the pixels within `Falloff` of the `Start`
/// colour (ImageMagick's fuzzy `Transparent`, read back from the alpha).
pub struct ColorThreshold;

impl NodeBuilder for ColorThreshold {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Mask);
        let input = context.require_input("In")?;
        let image = magick::to_image(&input);
        let start = context.color("Start");
        let target = [start[0] * 65535.0, start[1] * 65535.0, start[2] * 65535.0];
        // `ColorFuzz = new Percentage(Falloff * 100f)`.
        let percentage = (context.f32("Falloff") * 100.0) as f64;
        let fuzz = 65535.0 * (percentage / 100.0);
        let mut alpha = nerthus_image::quanta::transparent_alpha(&image, target, fuzz);
        nerthus_image::quanta::negate(&mut alpha);
        context.commit(magick::to_map(&alpha))
    }
}

/// DataExtractor: data from the node feeding it — the mask its Height and
/// Slope modifiers apply, how it changed its own input (or that change's
/// outline), or one of a Mixer's preserved layer masks.
pub struct DataExtractor;

impl NodeBuilder for DataExtractor {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Mask);
        let Some(source) = context.source_node("In").cloned() else {
            return Ok(());
        };
        match context.enum_name("Data") {
            "Difference" | "Boundary" => {
                let boundary = context.enum_name("Data") == "Boundary";
                let (Some(before), Some(after)) = (context.upstream_input("In"), context.input("In")) else {
                    return Ok(());
                };
                let mut change = maths::lift_above_zero(after.subtract(&before));
                if boundary {
                    change = aspect::threshold(&change, context.f32("Level"));
                }
                context.commit(change)
            }
            "MixerMask" => {
                if source.type_name != "Mixer" {
                    return Ok(());
                }
                if !source.values.get("PreserveMasks").and_then(Value::as_bool).unwrap_or(false) {
                    return Err(NodeError::Failed("Incoming Mixer node must enable Preserve Masks.".to_string()));
                }
                let layer = context.i32("Layer");
                let Some(mask) = context.source_output("In", &format!("MaskOut{layer}")) else {
                    return Err(NodeError::Failed(format!("Layer {layer} does not exist in Mixer node, or is malfunctioning.")));
                };
                context.commit((*mask).clone())
            }
            _ => {
                let resolution = context.resolution;
                let heights = match context.upstream_input("In") {
                    Some(map) => (*map).clone(),
                    None => Map::new(resolution, 1),
                };
                let mut mask = Map::new(resolution, 1);
                let terrain = (context.terrain_width(), context.terrain_height());
                let member = |values: &serde_json::Map<String, Value>, name: &str, default: f32| {
                    values.get(name).and_then(Value::as_f64).map_or(default, |value| value as f32)
                };
                let pair = |values: &serde_json::Map<String, Value>, name: &str, default: [f32; 2]| {
                    let value = values.get(name);
                    let part = |key: &str, fallback: f32| value.and_then(|value| value.get(key)).and_then(Value::as_f64).map_or(fallback, |part| part as f32);
                    [part("X", default[0]), part("Y", default[1])]
                };
                if let Some(height) = source.modifiers.iter().find(|modifier| modifier.type_name == "Height") {
                    let values: serde_json::Map<String, Value> = height.values.clone().into_iter().collect();
                    let [low, high] = pair(&values, "Range", [0.45, 0.65]);
                    let falloff = member(&values, "Falloff", 0.15);
                    let distort = member(&values, "Distort", 0.0);
                    let mut band = aspect::height_normalized(&heights, low, high, falloff);
                    if distort > 0.0 {
                        let settings = FractalWarpSettings {
                            size: 0.5,
                            strength: distort,
                            persist_strength: true,
                            z_coefficient: 0.0,
                            noise_type: WarpNoiseType::PerlinFbm,
                            perturbation: 0.5,
                            octaves: 12,
                            roughness: 0.5,
                            normalized: false,
                            iterations: 1,
                            mode: IterativeWarpMode::Virtual,
                            edges: EdgeBehaviour::Edge,
                            seed: source.id,
                            ..FractalWarpSettings::default()
                        };
                        band = warps::fractal_warp(&band, &settings, None, terrain.0, terrain.1);
                    }
                    mask = mask.add(&band);
                }
                if let Some(slope) = source.modifiers.iter().find(|modifier| modifier.type_name == "Slope") {
                    let values: serde_json::Map<String, Value> = slope.values.clone().into_iter().collect();
                    let [low, high] = pair(&values, "Range", [0.0, 30.0]);
                    let falloff = member(&values, "Falloff", 15.0);
                    let band = aspect::slope(&heights, low, high, falloff, terrain.0, terrain.1);
                    mask = mask.add(&band);
                }
                context.commit(mask)
            }
        }
    }
}

/// Var: in `Modify` mode, sets a project variable to its expression of the
/// variable's value (`x`); outputs a blank map either way. The variable
/// must hold the boxed type its kind recasts to, and the result is
/// always a float — so an `Int` variable can be modified only once.
pub struct Var;

impl NodeBuilder for Var {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let blank = Map::filled(context.resolution, 0.0);
        if context.enum_name("EditMode") == "Modify" {
            let name = context.string("VariableName").to_string();
            let Some(variable) = context.automation().variables.get(&name) else {
                return Err(NodeError::Failed(format!("The given key '{name}' was not present in the dictionary.")));
            };
            let var_type = variable.var_type;
            let current = context.variable_value(&name).unwrap_or(VariableValue::Null);
            let cast_failure = || NodeError::Failed("Specified cast is not valid.".to_string());
            let input = match var_type {
                VarType::Float => match current {
                    VariableValue::Float(value) => Some(value),
                    _ => return Err(cast_failure()),
                },
                VarType::Int => match current {
                    VariableValue::Int(value) => Some(value as f32),
                    _ => return Err(cast_failure()),
                },
                _ => None,
            };
            if let Some(input) = input {
                let expression = context.string("Expression").to_string();
                let expression_context = context.expression_context();
                let result = context.automation().evaluate(&expression, input, &expression_context);
                context.set_variable(&name, VariableValue::Float(result));
            }
        }
        context.commit(blank)
    }
}

/// Math (MathX): an ExprTk expression of the coordinates `x`, `y`, the
/// inputs `a`, `b`, `c` and the project's variables, per pixel
/// (the native `MathX_Compute`).
pub struct MathX;

impl NodeBuilder for MathX {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let a = context.input("In");
        let b = context.input("b");
        let c = context.input("c");
        let mut variables = Vec::new();
        let names: Vec<(String, VarType)> = context.automation().variables.values().map(|variable| (variable.name.clone(), variable.var_type)).collect();
        for (name, var_type) in names {
            let value = context.variable_value(&name).unwrap_or(VariableValue::Null);
            variables.push((name, var_type, value));
        }
        let size = context.resolution.x;
        let variables_text = crate::mathx::variables_string(size, &variables);
        let expression = context.string("Expression").to_string();
        let normalized = context.bool("NormalizedCoordinates");
        let inputs = [a.as_deref().map(Map::data), b.as_deref().map(Map::data), c.as_deref().map(Map::data)];
        let result = crate::mathx::compute(&expression, &variables_text, inputs, size, size, normalized);
        match result {
            Ok(data) => context.commit(Map::from_data(context.resolution, data)),
            Err(error) => Err(NodeError::Failed(crate::mathx::node_message(&error))),
        }
    }
}
