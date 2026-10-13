//! Render nodes: Shade, AO and LightX (built on the terrain renderer,
//! [`crate::shade`]), Halftone and Cartography.

use nerthus_engine::{BuildContext, Map, NodeBuilder, NodeData, NodeError, RenderIntent};
use serde_json::Value;

use crate::aspect;
use crate::cartographer;
use crate::colour;
use crate::colour_transport::{self, ColourTransportSettings};
use crate::combiner::{self, BlendMode};
use crate::masking;
use super::colorize::satellite_map_gradient;
use nerthus_image::octree::{self, Dither};
use nerthus_image::quanta;

use crate::lighting::{self, SunlightSettings};
use crate::magick;
use crate::maths::{self, GrayscaleConversion};
use crate::shade::{self, AmbientOcclusionModel, LightRig};

/// The icosphere subdivisions and pyramid levels of the hybrid occlusion
/// for an `AOQuality` below Ultra (Ultra raytraces instead).
fn hybrid_quality(quality: &str) -> (i32, i32) {
    match quality {
        "High" => (4, 2),
        "Medium" => (3, 3),
        _ => (3, 4),
    }
}

/// Shade: lights a colour map over a heightfield, as a table-top render,
/// a hill shade, an occlusion shade or soft daylight.
pub struct Shade;

impl NodeBuilder for Shade {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let colour = context.require_input("In")?;
        let heightfield = match context.input("Height") {
            Some(map) => map,
            None => context.underlay_heightfield().ok_or_else(|| NodeError::Validation("Required port [Height] not connected.".to_string()))?,
        };
        context.render_as(RenderIntent::Color);
        context.render_secondary_as(RenderIntent::Mask);

        let terrain = (context.terrain_width(), context.terrain_height());
        let azimuth = context.f32("SunAzimuth");
        let elevation = context.f32("SunElevation");
        let sunlight = context.f32("Sunlight");
        let shade_bias = context.f32("ShadeBias");
        let high_quality = context.bool("HighQuality");
        let style = context.enum_name("RenderStyle").to_string();

        let shaded = match style.as_str() {
            "HillShade" | "Occlusion" => {
                let occlusion_style = style == "Occlusion";
                let rig = LightRig {
                    shadow_quality: 5,
                    hybrid_ao_octaves: 2,
                    hybrid_ao_quality: 4,
                    sun_angular_diameter: 1.0,
                    direct_lighting: true,
                    exposure: sunlight,
                    shadows: false,
                    ambient_lighting: true,
                    ambient_occlusion: occlusion_style,
                    ..LightRig::default()
                };
                let sky = rig.sky(0.0, 90.0);
                let maps = rig.shade(&sky, &heightfield, Some(colour.as_ref()), terrain);
                let mut factor = if occlusion_style { maps.occlusion } else { maps.diffuse };
                if shade_bias != 0.0 {
                    maths::strengthen(&mut factor, shade_bias);
                }
                colour.multiply(&factor)
            }
            "SoftLight" => {
                let settings = SunlightSettings {
                    day_begin: 180,
                    day_end: 210,
                    latitude: elevation,
                    north: azimuth,
                    air_density: LightRig::default().air_density,
                };
                let mut light = lighting::sunlight_integral(&heightfield, &settings, terrain);
                if shade_bias != 0.0 {
                    maths::strengthen(&mut light, shade_bias);
                }
                colour.multiply(&light)
            }
            _ => {
                let ao_model = if high_quality { AmbientOcclusionModel::Raytraced } else { AmbientOcclusionModel::Hybrid };
                let rig = LightRig {
                    ao_model,
                    shadow_quality: 5,
                    hybrid_ao_octaves: 2,
                    hybrid_ao_quality: 4,
                    sun_angular_diameter: 20.0 * context.f32("SoftShadows"),
                    direct_lighting: true,
                    shadows: true,
                    exposure: sunlight,
                    ambient_lighting: true,
                    ambient_occlusion: true,
                    ..LightRig::default()
                };
                let sky = rig.sky(azimuth, elevation);
                let maps = rig.shade(&sky, &heightfield, Some(colour.as_ref()), terrain);
                if context.bool("ReflectedLight") {
                    let reflected = maths::fit_range(&maps.diffuse, 0.4, 1.0);
                    combiner::mix(BlendMode::HardLight, 0.2, &maps.color, &reflected)
                } else {
                    maps.color
                }
            }
        };

        context.set_port_intent("Out", RenderIntent::Raw);
        context.commit(shaded)
    }
}

/// AO: how open to the sky each point of the terrain is.
pub struct Ao;

impl NodeBuilder for Ao {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Mask);
        let input = context.require_input("In")?;
        let quality = context.enum_name("Quality").to_string();
        let mut rig = LightRig {
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
        if quality == "Ultra" {
            rig.ao_model = AmbientOcclusionModel::Raytraced;
        } else {
            let (hybrid_quality, octaves) = hybrid_quality(&quality);
            rig.hybrid_ao_quality = hybrid_quality;
            rig.hybrid_ao_octaves = octaves;
        }

        let terrain = (context.terrain_width(), context.terrain_height());
        let sky = rig.sky(33.0, 90.0);
        let maps = rig.shade(&sky, &input, None, terrain);
        let mut occlusion = maps.occlusion;
        maths::strengthen(&mut occlusion, 0.0 - context.f32("Enhance"));
        context.commit(occlusion)
    }
}

/// LightX: the full renderer, lit by a sun placed by angle or by place and
/// time, with its diffuse, shadow and occlusion maps as extra outputs.
pub struct LightX;

impl NodeBuilder for LightX {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Color);
        context.render_secondary_as(RenderIntent::Mask);
        let input = context.require_input("In")?;
        let heightfield = maths::grayscale(&input, GrayscaleConversion::RedChannel);
        let texture = context.input("Texture");

        let sun_color = context.color("SunColor");
        let ambient_color = context.color("AmbientColor");
        let mut rig = LightRig {
            automatic_colors: context.bool("AutomaticColors"),
            shadow_quality: 5,
            hybrid_ao_octaves: 2,
            hybrid_ao_quality: 4,
            sun_angular_diameter: context.f32("AngularDiameter"),
            direct_lighting: context.bool("DirectLight"),
            shadows: context.bool("Shadows"),
            ambient_lighting: context.bool("AmbientLight"),
            ambient_occlusion: context.bool("AmbientOcclusion"),
            sun_intensity: context.f32("SunIntensity"),
            sun_color: [sun_color[0], sun_color[1], sun_color[2]],
            ambient_intensity: context.f32("AmbientIntensity"),
            ambient_color: [ambient_color[0], ambient_color[1], ambient_color[2]],
            exposure: context.f32("Exposure"),
            air_density: context.f32("AirDensity") * 0.01,
            aerosols: context.f32("Haze") * 0.05,
            ozone: context.f32("Ozone") * 0.0025,
            default_color: context.f32("GroundBrightness"),
            ..LightRig::default()
        };
        let quality = context.enum_name("Quality").to_string();
        if quality == "Ultra" {
            rig.ao_model = AmbientOcclusionModel::Raytraced;
        } else {
            let (hybrid_quality, octaves) = hybrid_quality(&quality);
            rig.ao_model = AmbientOcclusionModel::Hybrid;
            rig.hybrid_ao_quality = hybrid_quality;
            rig.hybrid_ao_octaves = octaves;
        }

        let (azimuth, elevation) = if context.enum_name("DirectionSelection") == "Geographic" {
            let latitude = context.i32("Latitude") as f32;
            let (azimuth, elevation) = shade::sun_position(latitude, context.i32("DayOfYear"), context.f32("SolarHour"));
            (context.f32("North") + azimuth, elevation)
        } else {
            (context.f32("SunAzimuth"), context.f32("SunElevation"))
        };

        let terrain = (context.terrain_width(), context.terrain_height());
        let sky = rig.sky(azimuth, elevation);
        let maps = rig.shade(&sky, &heightfield, texture.as_deref(), terrain);
        context.set_port_intent("Out", RenderIntent::Raw);
        context.commit(maps.color)?;
        context.commit_port("Shadow", maps.shadow, RenderIntent::Default);
        context.commit_port("Diffuse", maps.diffuse, RenderIntent::Default);
        context.commit_port("AO", maps.occlusion, RenderIntent::Default);
        Ok(())
    }
}

/// The threshold maps of Halftone's patterns, by pattern index (`maps`);
/// "diag" and "hlines" are extra maps beyond ImageMagick's own.
const HALFTONE_MAPS: [&str; 18] = [
    "FloydSteinberg",
    "Riemersma",
    "o2x2",
    "o3x3",
    "o4x4",
    "o8x8",
    "h6x6a",
    "h8x8a",
    "h6x6o",
    "h8x8o",
    "c5x5b",
    "c5x5w",
    "c6x6b",
    "c6x6w",
    "c7x7b",
    "c7x7w",
    "diag",
    "hlines",
];

/// The pattern names in `HalftonePattern` order.
const HALFTONE_PATTERNS: [&str; 18] = [
    "FloydSteinberg",
    "Riemersma",
    "O2X2",
    "O3X3",
    "O4X4",
    "O8X8",
    "H6X6A",
    "H8X8A",
    "H6X6O",
    "H8X8O",
    "C5X5B",
    "C5X5W",
    "C6X6B",
    "C6X6W",
    "C7X7B",
    "C7X7W",
    "DIAG",
    "HLINE",
];

/// The colour cache granularity of ImageMagick's dithering in Windows
/// builds of Magick.Native (`CacheShift`; Apple builds use 3).
const DITHER_CACHE_SHIFT: u32 = 2;

/// Halftone: the terrain as a dithered print (`Migrated.HalftoneProcess`),
/// either error diffusion through ImageMagick's quantizer or an ordered
/// dither pattern, optionally posterized first.
pub struct Halftone;

impl NodeBuilder for Halftone {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Mask);
        let input = context.require_input("In")?;
        let depth = if context.bool("Posterize") { context.i32("Depth") } else { -1 };
        let pattern_name = context.enum_name("Pattern");
        let pattern = HALFTONE_PATTERNS.iter().position(|name| *name == pattern_name).unwrap_or(0);

        let mut image = magick::to_image(&input);
        quanta::auto_level(&mut image);
        let gray = if pattern < 2 {
            // `Quantize` in LinearGray with `Colors` and `TreeDepth` both
            // `(uint)depth`, then the map bridge's `ColorSpace = Gray`.
            let dither = if pattern == 0 { Dither::FloydSteinberg } else { Dither::Riemersma };
            if image.channels == 1 {
                quanta::grayscale_colormap_values(&mut image);
            }
            let mut linear = quanta::to_linear_gray(&image);
            let colors = depth as u32 as usize;
            octree::quantize_gray(&mut linear.data, linear.width, linear.height, colors, colors, dither, DITHER_CACHE_SHIFT);
            quanta::linear_to_gray(&linear)
        } else {
            if depth > 0 {
                quanta::posterize(&mut image, depth as usize);
            }
            quanta::ordered_dither(&mut image, HALFTONE_MAPS[pattern]).map_err(NodeError::Failed)?;
            quanta::grayscale(&image)
        };
        context.commit(magick::to_map(&gray))
    }
}

/// The gradient a colour port's source node holds (`GetGradientData`):
/// a SatelliteMap's palette data, a Synth's samples or a CLUTer's gradient.
fn gradient_data(source: Option<&NodeData>) -> Option<Vec<[f32; 4]>> {
    let source = source?;
    let value = |name: &str| source.values.get(name).unwrap_or(&Value::Null);
    let float4_list = |name: &str| -> Option<Vec<[f32; 4]>> {
        let items = value(name).as_array()?;
        let component = |item: &Value, key: &str| item.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let list = items.iter().map(|item| [component(item, "X"), component(item, "Y"), component(item, "Z"), component(item, "W")]);
        Some(list.collect())
    };
    match source.type_name.as_str() {
        "SatelliteMap" => {
            let library = value("Library").as_str().unwrap_or("Rock");
            let item = value("LibraryItem").as_i64().unwrap_or(0);
            let reverse = value("Reverse").as_bool().unwrap_or(false);
            let range_value = value("Range");
            let member = |key: &str| range_value.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32;
            satellite_map_gradient(library, item, reverse, [member("X"), member("Y")])
        }
        "Synth" => float4_list("Data"),
        "CLUTer" => float4_list("GradientData"),
        _ => None,
    }
}

/// The colour transport Cartography's Erode runs over the land (and, more
/// gently, the water) colours.
fn erode_colours(heights: &Map, colours: &Map, strength: f32) -> Map {
    let settings = ColourTransportSettings {
        transport_distance: 0.5,
        sediment_density: strength,
        blend: strength,
        flow_volume: 0.0,
        colour_hold: 0.5,
        laminar_flow: false,
        seed: 256,
    };
    colour_transport::simulate(heights, colours, None, None, &settings)
}

/// Cartography: a coloured map — the heights through a land palette (and
/// a water palette over the Water input), with contour lines, the coast
/// and a grid drawn over it.
pub struct Cartography;

impl NodeBuilder for Cartography {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let water_connected = context.is_connected("Water");
        let input = context.require_input("In")?;
        let mut heights = maths::autolevel((*input).clone());
        context.render_secondary_as(RenderIntent::Mask);
        let resolution = heights.resolution();
        let zeros = || Map::new(resolution, 1);

        let accuracy = context.f32("Accuracy");
        let thickness = context.i32("ContourThickness");
        let [land_low, land_high] = context.float2("LandContourIntenstity");
        let land = cartographer::contours(&heights, context.i32("LandContours"), 5, accuracy, land_high, land_low, thickness);
        let mut land_lines = land.lines;
        let mut terraces = land.terraces;
        let mut subterraces = land.subterraces;
        let mut water_lines = zeros();
        let mut boundary = zeros();
        let mut water_heights = None;
        let mut water_mask = None;
        let mut land_mask = None;
        if water_connected {
            let water_input = context.require_input("Water")?;
            let water = maths::autolevel((*water_input).clone());
            let [water_low, water_high] = context.float2("WaterContourIntenstity");
            water_lines = cartographer::contours(&water, context.i32("WaterContours"), 5, accuracy, water_high, water_low, thickness).lines;
            let mask = aspect::threshold(&water, 0.0001 + context.f32("Trim") * 0.05);
            boundary = cartographer::outline(&mask, land_high, context.i32("BoundaryThickness"));
            let mut land_only = mask.clone();
            maths::invert(&mut land_only);
            land_lines = land_lines.multiply(&land_only);
            terraces = terraces.multiply(&land_only);
            subterraces = subterraces.multiply(&land_only);
            water_heights = Some(water);
            water_mask = Some(mask);
            land_mask = Some(land_only);
        }

        let grid_style = context.enum_name("GridStyle");
        let no_grid = grid_style == "None" || (grid_style == "OverWater" && !water_connected);
        let grid = if no_grid {
            zeros()
        } else {
            let [grid_low, grid_high] = context.float2("GridIntenstity");
            let mut grid = cartographer::grid(resolution, context.i32("GridCount"), grid_high, grid_low, context.i32("GridThickness"));
            match (grid_style, &water_mask, &land_mask) {
                ("OverWater", Some(water_mask), _) => grid = grid.multiply(water_mask),
                ("OverLand", _, Some(land_mask)) => grid = grid.multiply(land_mask),
                _ => {}
            }
            grid
        };
        let lines = land_lines.add(&water_lines).add(&boundary).add(&grid);
        let mut decorations = maths::fit_range(&lines, 0.0, 1.0);
        maths::invert(&mut decorations);
        context.commit_port("Decorations", decorations.clone(), RenderIntent::Mask);
        context.commit_port("Grid", grid, RenderIntent::Mask);
        context.commit_port("LandContours", land_lines, RenderIntent::Mask);
        context.commit_port("WaterContours", water_lines, RenderIntent::Mask);

        let smooth = context.enum_name("GradientStyle") == "Smooth";
        let mut gradient_source = match context.enum_name("GradientStyle") {
            "LargeContours" => terraces,
            "AllContours" => subterraces,
            _ => heights.clone(),
        };
        if !context.bool("EqualizeGradient") {
            let (input_min, input_max) = maths::min_max(input.data());
            let (source_min, source_max) = maths::min_max(gradient_source.data());
            maths::remap(&mut gradient_source, source_max, source_min, input_max, input_min);
            if smooth {
                heights = gradient_source.clone();
            }
        }
        let land_bias = context.f32("LandBias");
        if land_bias != 0.0 {
            maths::strengthen(&mut gradient_source, land_bias);
        }
        let erode = context.bool("Erode");
        let mut land_colours = match gradient_data(context.source_node("LandColor")) {
            Some(stops) => colour::clut(&gradient_source, &stops),
            None => {
                let mut palette = cartographer::land_palette(context.enum_value("LandPalette") as usize);
                if context.bool("Reverse") {
                    palette.reverse();
                }
                colour::clut_colours(&gradient_source, &palette)
            }
        };
        if erode {
            land_colours = erode_colours(&heights, &land_colours, 1.0);
        }

        let mut coloured = land_colours;
        if let (Some(mut water), Some(water_mask)) = (water_heights, water_mask) {
            let water_bias = context.f32("WaterBias");
            if water_bias != 0.0 {
                maths::strengthen(&mut water, water_bias);
            }
            let mut water_colours = match gradient_data(context.source_node("WaterColor")) {
                Some(stops) => colour::clut(&water, &stops),
                None => {
                    let palette = cartographer::water_palette(context.enum_value("WaterPalette") as usize);
                    colour::clut_colours(&water, &palette)
                }
            };
            if erode {
                water_colours = erode_colours(&heights, &water_colours, 0.5);
            }
            coloured = masking::mask(&coloured, &water_colours, &water_mask);
        }
        let coloured = combiner::multiply(&coloured, &decorations, 1.0);
        context.render_as(RenderIntent::Color);
        context.commit(coloured)
    }
}
