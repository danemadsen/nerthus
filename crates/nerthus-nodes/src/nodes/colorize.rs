//! The Colorize toolbox's palette nodes: SatelliteMap colours a
//! heightfield through a palette from the colour library.

use nerthus_engine::{BuildContext, Map, NodeBuilder, NodeError, RenderIntent};
use serde_json::Value;

use crate::colour;
use crate::colour_library::{self, Palette};
use crate::colour_transport::{self, ColourTransportSettings};
use crate::flow::{self, FlowSettings};
use crate::profiles;
use crate::texturize::{self, TextureBaseSettings};
use crate::combiner;
use crate::histogram;
use crate::maths::{self, GrayscaleConversion};
use crate::random::XorShift64Star;
use crate::shade::{AmbientOcclusionModel, LightRig};
use crate::random_noises;
use crate::spline::Polyline;
use crate::transformer::EdgeBehaviour;
use crate::warps::{self, FractalWarpSettings, IterativeWarpMode, WarpNoiseType};

/// The red, green and blue curves of the palette entries in `range`, each
/// over the entry's position in the palette (as `lineR`, `lineG` and
/// `lineB` are rebuilt).
pub(crate) struct PaletteCurves {
    curves: [Polyline; 3],
}

impl PaletteCurves {
    pub(crate) fn new(palette: &Palette, range: [f32; 2]) -> Result<PaletteCurves, NodeError> {
        let count = palette.len() as f32;
        let start = (count * range[0]) as i32;
        let end = (count * range[1]) as i32;
        let mut channels: [Vec<[f32; 2]>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for index in start.max(0)..end.min(palette.len() as i32) {
            let position = index as f32 / count;
            let rgb = palette[index as usize];
            for (channel, points) in channels.iter_mut().enumerate() {
                points.push([position, rgb[channel] as i32 as f32 / 255.0]);
            }
        }
        if channels[0].len() < 2 {
            return Err(NodeError::Failed("Index was outside the bounds of the array.".to_string()));
        }
        let [red, green, blue] = channels;
        let curves = [Polyline::from_unsorted(&red), Polyline::from_unsorted(&green), Polyline::from_unsorted(&blue)];
        Ok(PaletteCurves { curves })
    }

    /// `heights` mapped through one channel's curve.
    pub(crate) fn channel(&self, heights: &Map, channel: usize) -> Map {
        let curve = &self.curves[channel];
        heights.map_values(|height| curve.interpolate(height))
    }

    /// The colour map of `heights` (`new Map(lineR(h), lineG(h), lineB(h))`).
    pub(crate) fn colour(&self, heights: &Map) -> Map {
        let channels: Vec<Vec<f32>> = self
            .curves
            .iter()
            .map(|curve| {
                let channel = heights.map_values(|height| curve.interpolate(height));
                channel.into_data()
            })
            .collect();
        Map::from_channels(heights.resolution(), channels)
    }
}

/// The palette a SatelliteMap-style node points at (`Library`,
/// `LibraryItem`), clamping the item to the library (and saving that).
/// SatelliteMap picks a random palette for an unset (-1) item.
pub(crate) fn chosen_palette(context: &mut BuildContext, randomize_unset: bool) -> Result<Palette, NodeError> {
    let library_name = context.enum_name("Library").to_string();
    let Some(library) = colour_library::category(&library_name) else {
        return Err(NodeError::Failed(format!("The given key '{library_name}' was not present in the dictionary.")));
    };
    let mut item = context.i32("LibraryItem");
    if item == -1 && randomize_unset {
        // `UpdateUI` picks a random Rock palette on first use.
        let seed = web_time::SystemTime::now()
            .duration_since(web_time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as i32)
            .unwrap_or(1);
        let rock_count = colour_library::category("Rock").map(|rock| rock.len()).unwrap_or(1) as i32;
        item = XorShift64Star::new(seed).next_range(0, rock_count);
        context.update_member("LibraryItem", Value::from(item));
    }
    let clamped = item.min(library.len() as i32 - 1);
    if clamped != item {
        context.update_member("LibraryItem", Value::from(clamped));
    }
    let Some(palette) = library.get(clamped.max(0) as usize) else {
        return Err(NodeError::Failed("Index was outside the bounds of the array.".to_string()));
    };
    Ok(*palette)
}

/// `Generic5` roughness: how many warps break up the palette bands.
fn rough_passes(name: &str) -> i32 {
    match name {
        "Low" => 1,
        "Med" => 4,
        "High" => 8,
        "Ultra" => 16,
        _ => 0,
    }
}

/// SatelliteMap: the heightfield coloured through a palette from the
/// colour library, optionally roughened and shifted in hue, saturation and
/// lightness.
pub struct SatelliteMap;

impl NodeBuilder for SatelliteMap {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Color);
        let mut palette = chosen_palette(context, true)?;
        if context.bool("Reverse") {
            palette.reverse();
        }
        let curves = PaletteCurves::new(&palette, context.float2("Range"))?;

        let input = context.require_input("In")?;
        let mut heights = maths::grayscale(&input, GrayscaleConversion::RedChannel);
        match context.enum_name("Enhance") {
            "Autolevel" => heights = maths::autolevel(heights),
            "Equalize" => heights = histogram::equalize(heights),
            _ => {}
        }
        let bias = context.f32("Bias");
        if bias != 0.0 {
            maths::strengthen(&mut heights, bias);
        }

        let passes = rough_passes(context.enum_name("Rough"));
        if passes > 0 {
            let mut random = XorShift64Star::new(context.node_id());
            let terrain = (context.terrain_width(), context.terrain_height());
            for _ in 0..passes {
                let settings = FractalWarpSettings {
                    size: 0.05,
                    strength: 0.45 * (passes as f32 / 2.0),
                    persist_strength: true,
                    z_coefficient: 0.0,
                    noise_type: WarpNoiseType::PerlinFbm,
                    perturbation: 0.5,
                    octaves: 12,
                    roughness: 0.85,
                    normalized: false,
                    iterations: 1,
                    mode: IterativeWarpMode::Virtual,
                    edges: EdgeBehaviour::Edge,
                    seed: random.next_range(0, 65535),
                    ..FractalWarpSettings::default()
                };
                heights = warps::fractal_warp(&heights, &settings, None, terrain.0, terrain.1);
            }
        }

        let mut coloured = curves.colour(&heights);
        let hue = context.f32("Hue");
        let saturation = context.f32("Saturation");
        let lightness = context.f32("Lightness");
        if hue != 0.0 || saturation != 0.0 || lightness != 0.0 {
            coloured = colour::hsl(&coloured, hue, saturation, lightness);
        }
        context.commit(coloured)
    }
}

/// HueErosion: a colour map carried downhill by flowing water over its
/// heightfield (the colour transport).
pub struct HueErosion;

impl NodeBuilder for HueErosion {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let colour = context.require_input("In")?;
        let heights = match context.input("Height") {
            Some(map) => map,
            None => context
                .underlay_heightfield()
                .ok_or_else(|| NodeError::Validation("Required port [Height] not connected.".to_string()))?,
        };
        let mut heights = (*heights).clone();
        let seed = context.i32("Seed");
        let diffusion = context.f32("Diffusion");
        if diffusion > 0.0 {
            let area = (context.resolution.x * context.resolution.y) as i32 as f32;
            let count = (area / 8.0 * diffusion) as i32;
            let jitter = random_noises::fixed(context.resolution, count, true, 0.0, 0.01, seed);
            heights = heights.add(&jitter);
        }
        let precipitation = context.input("Precipitation");
        let settings = ColourTransportSettings {
            transport_distance: context.f32("TransportDistance"),
            sediment_density: context.f32("SedimentDensity"),
            blend: context.f32("Blend"),
            flow_volume: context.f32("FlowVolume"),
            colour_hold: context.f32("ColorHold"),
            laminar_flow: context.bool("LaminarFlow") && diffusion == 0.0,
            seed,
        };
        let eroded = colour_transport::simulate(&heights, &colour, precipitation.as_deref(), None, &settings);
        context.render_as(RenderIntent::Color);
        context.render_secondary_as(RenderIntent::Mask);
        context.commit(eroded)
    }
}

/// `low + t × (high − low)`.
fn lerp(t: f32, low: f32, high: f32) -> f32 {
    low + t * (high - low)
}

/// SuperColor: the heightfield coloured through a library palette along a
/// texture (peaks, flow, TextureBase or a custom map), optionally eroded
/// by colour transport.
pub struct SuperColor;

impl SuperColor {
    /// `GetTextureMap`: the map the palette is laid along.
    fn texture_map(context: &BuildContext, heights: &Map) -> Map {
        let seed = context.i32("Seed");
        let strength = context.f32("Strength");
        let terrain = (context.terrain_width(), context.terrain_height());
        let mut texture = match context.enum_name("Texture") {
            "None" => match context.input("Texture") {
                Some(map) => (*map).clone(),
                None => maths::autolevel(heights.clone()),
            },
            "Texture" => {
                let settings = TextureBaseSettings {
                    slope: 0.25,
                    scale: strength,
                    soil: 0.6,
                    chaos: 0.1,
                    patches: 0.4,
                    seed,
                    old_slopes: false,
                };
                maths::autolevel(texturize::texture_base(heights, None, &settings, terrain))
            }
            "Flow" => {
                let settings = FlowSettings {
                    flow_length: strength,
                    flow_volume: 0.1,
                    seed,
                };
                flow::simulate(heights, None, &settings)
            }
            _ => {
                let bands = profiles::graphic_eq(heights, [0.0, 2.0, 2.0, 2.0, 0.0, 0.0, 0.0]);
                let mut peaks = maths::autolevel(bands);
                maths::strengthen(&mut peaks, lerp(strength, -0.6, 0.6));
                let soil = texturize::soil2(heights, lerp(strength, 0.0, 0.7));
                let screened = combiner::screen(&peaks, &soil, 1.0);
                histogram::equalize(screened)
            }
        };
        let warp = context.f32("Warp");
        if warp > 0.0 {
            let settings = FractalWarpSettings {
                size: 0.5,
                strength: warp,
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
                seed,
                ..FractalWarpSettings::default()
            };
            texture = warps::fractal_warp(&texture, &settings, None, terrain.0, terrain.1);
        }
        texture
    }

    /// One `ColorTransport.Simulation` pass of the Erosion process.
    fn erode(heights: &Map, colour: &Map, distance: f32, flow_volume: f32, hold: f32, seed: i32) -> Map {
        let settings = ColourTransportSettings {
            transport_distance: distance,
            sediment_density: 1.0,
            blend: 0.5,
            flow_volume,
            colour_hold: hold,
            laminar_flow: false,
            seed,
        };
        colour_transport::simulate(heights, colour, None, None, &settings)
    }
}

impl NodeBuilder for SuperColor {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Color);
        let input = context.require_input("In")?;
        let mut texture = SuperColor::texture_map(context, &input);

        let palette = chosen_palette(context, false)?;
        let curves = PaletteCurves::new(&palette, context.float2("Range"))?;
        if context.bool("Reverse") {
            maths::invert(&mut texture);
        }
        let bias = context.f32("Bias");
        if bias != 0.0 {
            maths::strengthen(&mut texture, bias);
        }
        let mut coloured = curves.colour(&texture);

        if context.enum_name("Process") == "Erosion" {
            let seed = context.i32("Seed");
            let amount = context.enum_name("Amount").to_string();
            coloured = SuperColor::erode(&input, &coloured, 0.5, 0.0, 0.5, seed);
            if amount != "Low" {
                coloured = SuperColor::erode(&input, &coloured, 0.25, 0.5, 0.02, seed + 1);
                if amount == "High" || amount == "Ultra" {
                    let ultra = amount == "Ultra";
                    let distance = if ultra { 0.65 } else { 0.25 };
                    let flow_volume = if ultra { 1.0 } else { 0.5 };
                    coloured = SuperColor::erode(&input, &coloured, distance, flow_volume, 0.1, seed + 2);
                }
            }
        }
        context.render_secondary_as(RenderIntent::Mask);
        context.commit(coloured)
    }
}

/// Weathering: grime laid into a colour map along the soil of its
/// heightfield (Rock palette 117), optionally washed out by colour
/// transport and darkened in crevices.
pub struct Weathering;

impl NodeBuilder for Weathering {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Color);
        let colour = context.require_input("In")?;
        let heights = match context.input("Height") {
            Some(map) => map,
            None => context
                .underlay_heightfield()
                .ok_or_else(|| NodeError::Validation("Required port [Height] not connected.".to_string()))?,
        };
        let mut heights = (*heights).clone();
        if context.bool("Inverse") {
            maths::invert(&mut heights);
        }

        let Some(rock) = colour_library::palette("Rock", 117) else {
            return Err(NodeError::Failed("Index was outside the bounds of the array.".to_string()));
        };
        let curves = PaletteCurves::new(rock, [0.0, 1.0])?;
        let soil = texturize::soil2(&heights, lerp(context.f32("Scale"), 0.000_001, 0.6));
        let mut soil = histogram::equalize(soil);
        maths::strengthen(&mut soil, 1.0 - context.f32("Creep"));
        let mut grime = maths::autolevel(curves.channel(&soil, 0));

        if context.bool("WashedOut") {
            let settings = ColourTransportSettings {
                transport_distance: 0.5,
                sediment_density: 0.65,
                blend: 0.65,
                flow_volume: 0.0,
                colour_hold: 0.5,
                laminar_flow: false,
                seed: context.node_id(),
            };
            grime = colour_transport::simulate(&heights, &grime, None, None, &settings);
        }

        let amount = context.f32("Amount");
        let dirt = context.f32("Dirt");
        let mut weathered = if dirt != 0.0 {
            let rig = LightRig {
                shadow_quality: 5,
                hybrid_ao_octaves: 2,
                hybrid_ao_quality: 4,
                sun_angular_diameter: 0.2,
                direct_lighting: false,
                shadows: false,
                ambient_lighting: true,
                ambient_occlusion: true,
                ao_model: AmbientOcclusionModel::Hybrid,
                ..LightRig::default()
            };
            let terrain = (context.terrain_width(), context.terrain_height());
            let sky = rig.sky(33.0, 90.0);
            let maps = rig.shade(&sky, &heights, None, terrain);
            let mut crevices = maps.occlusion;
            let contrast = if context.bool("Darker") { 0.0 - dirt } else { 1.0 - dirt };
            maths::strengthen(&mut crevices, contrast);
            let darkened = colour.multiply(&crevices);
            combiner::screen(&darkened, &grime, amount)
        } else {
            combiner::screen(&colour, &grime, amount)
        };

        let hue = context.f32("Hue");
        let saturation = context.f32("Saturation");
        let lightness = context.f32("Lightness");
        if hue != 0.0 || saturation != 0.0 || lightness != 0.0 {
            weathered = colour::hsl(&weathered, hue, saturation, lightness);
        }
        context.commit(weathered)
    }
}

/// The `Data` a SatelliteMap node builds (its palette's colours from the
/// range, each at its position in the full palette), as Cartography reads
/// it.
pub(crate) fn satellite_map_gradient(library: &str, item: i64, reverse: bool, range: [f32; 2]) -> Option<Vec<[f32; 4]>> {
    let palettes = colour_library::category(library)?;
    let item = item.min(palettes.len() as i64 - 1).max(0) as usize;
    let mut palette: Vec<[u8; 3]> = palettes.get(item)?.to_vec();
    if reverse {
        palette.reverse();
    }
    let count = palette.len() as f32;
    let start = (count * range[0]) as i32;
    let end = (count * range[1]) as i32;
    let mut data = Vec::new();
    for index in start.max(0)..end.min(palette.len() as i32) {
        let [red, green, blue] = palette[index as usize];
        let channel = |byte: u8| byte as i32 as f32 / 255.0;
        data.push([channel(red), channel(green), channel(blue), index as f32 / count]);
    }
    Some(data)
}
