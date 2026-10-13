//! The Simulate toolbox's Vegetation family: Trees and Shrubs, grown where
//! fresh water gathers inside Cellular3D patches, kept off steep, high and
//! snow-swept ground.

use std::sync::Arc;

use nerthus_engine::{BuildContext, Map, NodeBuilder, NodeError, RenderIntent};

use crate::aspect;
use crate::cellular3d::{self, Cellular3dSettings};
use crate::colour;
use crate::combiner;
use crate::erosions::{self, SnowSettings};
use crate::fast_blur;
use crate::flow::{self, FlowSettings};
use crate::histogram;
use crate::maths;
use crate::morphology::{self, Kernel};
use crate::morphology2::{self, ApertureKernel, ApertureSettings};
use crate::profiles;
use crate::random_noises;
use crate::transformer::EdgeBehaviour;
use crate::warps::{self, FractalWarpSettings, IterativeWarpMode, WarpNoiseType};

/// `low + t × (high − low)`.
fn lerp(t: f32, low: f32, high: f32) -> f32 {
    low + t * (high - low)
}

/// A third.
fn third(value: f32) -> f32 {
    value * (1.0 / 3.0)
}

/// `Combiner.Max(map, Warps.FractalWarp(map, size, chaos, …))`: the map
/// smeared by a fractal warp and kept where that is higher.
fn chaos_warp(map: &Map, size: f32, chaos: f32, roughness: f32, seed: i32, terrain: (f32, f32)) -> Map {
    let settings = FractalWarpSettings {
        size,
        strength: chaos,
        persist_strength: true,
        z_coefficient: 0.0,
        noise_type: WarpNoiseType::PerlinFbm,
        perturbation: 0.5,
        octaves: 12,
        roughness,
        normalized: false,
        iterations: 1,
        mode: IterativeWarpMode::Virtual,
        edges: EdgeBehaviour::Edge,
        seed,
        ..FractalWarpSettings::default()
    };
    let warped = warps::fractal_warp(map, &settings, None, terrain.0, terrain.1);
    combiner::max(map, &warped, 1.0)
}

/// The dead zones: where `Simulations.Snow` would leave snow (its soft
/// mask), standing in for ground too exposed for vegetation.
fn dead_zones(heights: &Map, settings: &SnowSettings, terrain: (f32, f32)) -> Map {
    let [_, _, _, soft_mask] = erosions::snow(heights, settings, None, None, terrain);
    soft_mask
}

/// `BuildGrowthCells`: Cellular3D cells (`cells` across, gap 1) over the
/// fresh-water map, their gaps widened where water and inhibition are low.
fn growth_cells(fresh_water: &Map, inhibitors: &Map, cells: f32, seed: i32, terrain: (f32, f32)) -> Map {
    let combined = inhibitors.add(fresh_water);
    let mut gap_map = maths::autolevel(combined);
    maths::invert(&mut gap_map);
    let settings = Cellular3dSettings {
        cells,
        gap: 1.0,
        scale: [1.0, 1.0, 1.0],
        jitter: [1.0, 1.0, 1.0],
        seed: seed as u32,
        terrain_width: terrain.0,
        terrain_height: terrain.1,
    };
    let (surface, _) = cellular3d::render(fresh_water, Some(&gap_map), &settings);
    surface
}

/// The custom inhibition input (`Inhibition`), levelled.
fn custom_inhibition(context: &BuildContext) -> Option<Map> {
    let custom = context.input("Inhibition")?;
    Some(maths::autolevel((*custom).clone()))
}

/// The trees to avoid (`AvoidTrees`): every tree node's `Trees` output, or
/// the previous one's when `In` comes straight from Trees or Shrubs, as a
/// mask. `None` when there are none to avoid.
fn avoided_trees(context: &BuildContext) -> Result<Option<Map>, NodeError> {
    let trees = match context.enum_name("AvoidTrees") {
        "All" => Some(context.accumulation("Trees", "Trees", None, 0)),
        "Previous" => {
            let source = context.source_type("In").unwrap_or_default();
            let tree_node = matches!(source, "Trees" | "Shrubs") || source.ends_with(".Trees") || source.ends_with(".Shrubs");
            let previous = if tree_node { context.source_output("In", "Trees") } else { None };
            match previous {
                Some(map) => Some((*map).clone()),
                None => return Err(NodeError::Failed("Object reference not set to an instance of an object.".to_string())),
            }
        }
        _ => return Ok(None),
    };
    Ok(trees.map(|map| aspect::threshold(&map, 0.000_499_999_97)))
}

/// What Trees and Shrubs read from their parameters for the inhibitor maps.
struct InhibitorSettings {
    seed: i32,
    slope: [f32; 2],
    slope_falloff: f32,
    altitude: [f32; 2],
    altitude_falloff: f32,
    dead_flow: f32,
    model: f32,
    chaos: f32,
    edges: f32,
    bias: f32,
}

fn inhibitor_settings(context: &BuildContext) -> InhibitorSettings {
    InhibitorSettings {
        seed: context.i32("Seed"),
        slope: context.float2("Slope"),
        slope_falloff: context.f32("SlopeFalloff"),
        altitude: context.float2("Altitude"),
        altitude_falloff: context.f32("AltitudeFalloff"),
        dead_flow: context.f32("DeadFlow"),
        model: maths::dotnet_max(1.0, context.f32("ConsolidateFlows") * 20.0),
        chaos: context.f32("Chaos"),
        edges: context.f32("BlurEdges"),
        bias: context.f32("Bias"),
    }
}

/// Trees: tree instances (or an area mask) grown along fresh water within
/// Cellular3D patches.
pub struct Trees;

impl Trees {
    /// `Trees.BuildInhibitors`: the dead zones and where slope, altitude
    /// and peaks allow trees.
    fn inhibitors(heights: &Map, settings: &InhibitorSettings, peaks: f32, snow_line: f32, terrain: (f32, f32)) -> (Map, Map) {
        let slope = aspect::slope(heights, settings.slope[0], settings.slope[1], settings.slope_falloff, terrain.0, terrain.1);
        let altitude = aspect::height_normalized(heights, settings.altitude[0], settings.altitude[1], settings.altitude_falloff);
        let mut inhibitors = altitude.multiply(&slope);
        if peaks > 0.0 {
            let bands = profiles::graphic_eq(heights, [0.0, 2.0, 2.0, 2.0, 0.0, 0.0, 0.0]);
            let levelled = maths::autolevel(bands);
            let summits = aspect::threshold(&levelled, 1.0 - peaks);
            inhibitors = inhibitors.subtract(&summits);
            maths::clamp_if_out_of_range(&mut inhibitors);
        }
        if settings.chaos > 0.0 {
            inhibitors = chaos_warp(&inhibitors, 0.05, settings.chaos, 0.5, 257 + settings.seed, terrain);
        }
        if settings.edges > 0.0 {
            let blurred = fast_blur::blur_relative(&inhibitors, settings.edges * 0.05);
            inhibitors = maths::clip_map(&blurred, 0.015, 1.0);
            maths::strengthen(&mut inhibitors, 0.0 - settings.edges);
        }

        let snow = SnowSettings {
            duration: settings.dead_flow,
            intensity: 0.35,
            settle_thaw: 0.9,
            melt: 0.05,
            snow_line,
            real_scale: false,
            scale: 5000.0,
            verticality: settings.bias * 40000.0,
            slip_off: 20.0,
            adhered_mass: 12.0,
            model_scale: settings.model,
        };
        (dead_zones(heights, &snow, terrain), inhibitors)
    }

    /// `Trees.BuildFreshwaterMap`: where water gathers (or, avoiding water,
    /// where it does not), outside the dead zones.
    fn fresh_water(context: &BuildContext, heights: &Map, factor: f32, spread: f32, equalize: bool, dead_zones: &Map) -> Map {
        let seed = context.i32("Seed");
        let chaos = context.f32("Chaos");
        let edges = context.f32("BlurEdges");
        let terrain = (context.terrain_width(), context.terrain_height());
        let settings = FlowSettings {
            flow_length: (1.0 - context.f32("Health")) * factor,
            flow_volume: spread,
            seed,
        };
        let flow = flow::simulate(heights, None, &settings);
        let mut fresh = if equalize { histogram::equalize(flow) } else { maths::autolevel(flow) };
        if context.enum_name("WaterAffinity") == "Avoid" {
            maths::invert(&mut fresh);
        }
        if chaos > 0.0 {
            fresh = chaos_warp(&fresh, 0.2, chaos, 0.65, 257 + seed, terrain);
        }
        if edges > 0.0 {
            let blurred = fast_blur::blur_relative(&fresh, edges * 0.1);
            fresh = maths::clip_map(&blurred, 0.015, 1.0);
        }
        let fresh = maths::autolevel(fresh).add_scalar(context.f32("Random"));
        let smoothed = fast_blur::blur_relative(&fresh, context.f32("Spread") * 0.5);
        let gathered = maths::autolevel(combiner::min(&fresh, &smoothed, 1.0));
        let mut fresh = gathered.subtract(dead_zones);
        maths::clamp_if_out_of_range(&mut fresh);
        fresh
    }

    fn growth_cells(context: &BuildContext, fresh_water: &Map, inhibitors: &Map) -> Map {
        let terrain = (context.terrain_width(), context.terrain_height());
        let cells = 1.0 / lerp(context.f32("Patches"), 0.005, 0.02);
        growth_cells(fresh_water, inhibitors, cells, context.i32("Seed"), terrain)
    }

    /// Takes the custom inhibition and the trees to avoid off `trees`.
    fn remove_inhibited(context: &BuildContext, trees: Map, custom: Option<Map>, trim: bool) -> Result<Map, NodeError> {
        let mut trees = trees;
        let mut touched = false;
        if let Some(mut custom) = custom {
            let trim_level = context.f32("Trim");
            if trim && trim_level > 0.0 {
                custom = maths::clip_map(&custom, trim_level, 1.0);
            }
            let influence = context.f32("CustomInfluence");
            if influence > 0.0 {
                custom = morphology::dilate(&custom, Kernel::Cross, 0.2 * influence, 1, 0.0);
            }
            trees = trees.subtract(&custom);
            touched = true;
        }
        if let Some(avoided) = avoided_trees(context)? {
            trees = trees.subtract(&avoided);
            touched = true;
        }
        if touched {
            maths::clamp_if_out_of_range(&mut trees);
        }
        Ok(trees)
    }

    /// `BuildInstances`: scattered tree points sized by `TreeSize`.
    fn instances(context: &mut BuildContext, input: &Map) -> Result<(), NodeError> {
        let terrain = (context.terrain_width(), context.terrain_height());
        let seed = context.i32("Seed");
        let settings = inhibitor_settings(context);
        let (dead, inhibitors) = Trees::inhibitors(input, &settings, context.f32("Peaks"), context.f32("Snowline"), terrain);
        let spread = context.f32("Spread") * 6.0;
        let fresh = Trees::fresh_water(context, input, 2.0, spread, true, &dead);
        let cells = Trees::growth_cells(context, &fresh, &inhibitors);

        let area = (context.resolution.x * context.resolution.y) as i32 / 2;
        let tree_count = context.i32("TreeCount");
        let size = third(context.f32("TreeSize"));
        let relative = context.bool("IsRelative");
        let healthy = ((tree_count * 1000) as f32 * context.f32("Health")) as i32;
        let guide = inhibitors.add(&dead);
        let planted = random_noises::fixed_white_mapped(&guide, area.min(healthy), size, relative, seed);
        let strays = ((tree_count * 200) as f32 * context.f32("Random")) as i32;
        let random_trees = random_noises::fixed_white(cells.resolution(), area.min(strays), size, relative, seed + 1);

        let mut trees = planted.multiply(&cells).multiply(&fresh);
        trees = maths::clip_map(&trees, size * context.f32("TrimUnder"), 1.0);
        trees = trees.add(&random_trees);
        if context.bool("Strict") {
            let allowed = aspect::threshold(&inhibitors, 0.5);
            trees = trees.multiply(&allowed);
        }
        let custom = custom_inhibition(context);
        trees = Trees::remove_inhibited(context, trees, custom, true)?;

        let shape = context.enum_name("Shape").to_string();
        if shape != "Point" {
            let width = trees.width() as f32;
            let settings = ApertureSettings {
                size: width * (1.0 / width),
                kernel: ApertureKernel::Disk,
                erosion: false,
                vertices: 4,
                antialiased: false,
                direction: 0.0,
                corner_angle: 90.0,
            };
            trees = morphology2::aperture(&trees, &settings);
        }
        if shape == "BigOnly" {
            let big = aspect::threshold(&trees, 0.0001);
            trees = big.multiply_scalar(size);
        }

        context.render_secondary_as(RenderIntent::Mask);
        context.render_as(RenderIntent::Heightfield);
        let planted_terrain = input.add(&trees);
        context.commit(planted_terrain)?;
        context.commit_port("DeadZones", dead, RenderIntent::Default);
        context.commit_port("FreshWater", fresh, RenderIntent::Default);
        context.commit_port("Trees", trees, RenderIntent::Default);
        Ok(())
    }

    /// `BuildAreaMask`: where forest grows, as a mask.
    fn area_mask(context: &mut BuildContext, input: &Map) -> Result<(), NodeError> {
        let terrain = (context.terrain_width(), context.terrain_height());
        let spread = context.f32("Spread") + 0.45;
        let settings = inhibitor_settings(context);
        let (dead, mut inhibitors) = Trees::inhibitors(input, &settings, context.f32("Peaks"), context.f32("Snowline"), terrain);
        maths::strengthen(&mut inhibitors, -1.0 + spread);
        let fresh = Trees::fresh_water(context, input, 2.0, spread * 6.0, false, &dead);
        let cells = Trees::growth_cells(context, &fresh, &inhibitors);

        let ones = Map::filled(context.resolution, 1.0);
        let area = ones.multiply(&cells).multiply(&fresh);
        let custom = custom_inhibition(context);
        let area = Trees::remove_inhibited(context, area, custom, false)?;

        context.render_as(RenderIntent::Mask);
        context.render_secondary_as(RenderIntent::Mask);
        let forested = input.add(&area);
        context.commit(forested)?;
        context.commit_port("DeadZones", dead, RenderIntent::Default);
        context.commit_port("FreshWater", fresh, RenderIntent::Default);
        context.commit_port("Trees", area, RenderIntent::Default);
        Ok(())
    }
}

impl NodeBuilder for Trees {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        if context.enum_name("Output") == "AreaMask" {
            Trees::area_mask(context, &input)
        } else {
            Trees::instances(context, &input)
        }
    }

    /// `Trees.GetCustomColorMap`: the trees, widened by a pixel, in green
    /// over grey ground, with all the vegetation built faintly around them;
    /// with Visualize Data, dead zones, trees and fresh water as red, green
    /// and blue.
    fn custom_colour_map(&self, context: &BuildContext) -> Option<Map> {
        let trees = context.own_output("Trees")?;
        let planted = aspect::threshold(&trees, 0.0001);
        let width = planted.width() as f32;
        let settings = ApertureSettings {
            size: width * (1.0 / width),
            kernel: ApertureKernel::Disk,
            erosion: false,
            vertices: 4,
            antialiased: false,
            direction: 0.0,
            corner_angle: 90.0,
        };
        let crowns = morphology2::aperture(&planted, &settings);
        let vegetation = crowns.add(&built_vegetation(context));
        if !context.bool("VisualizeData") {
            return Some(colour::colorize(&vegetation, &FOREST_COLOURS));
        }
        let dead_zones = context.own_output("DeadZones")?;
        let fresh_water = context.own_output("FreshWater").or_else(|| context.input("Inhibition"));
        let fresh_water = fresh_water.unwrap_or_else(|| Arc::new(Map::new(context.resolution, 1)));
        let channels = vec![dead_zones.data().to_vec(), vegetation.data().to_vec(), fresh_water.data().to_vec()];
        Some(Map::from_channels(dead_zones.resolution(), channels))
    }
}

/// Trees' and Shrubs' viewport colours: grey ground to forest green.
const FOREST_COLOURS: [[f32; 3]; 4] = [[0.6, 0.6, 0.6], FOREST_GREEN, FOREST_GREEN, FOREST_GREEN];
const FOREST_GREEN: [f32; 3] = [33.0 / 255.0, 92.0 / 255.0, 29.0 / 255.0];

/// Where any built tree or shrub node (this one included) planted, at a
/// quarter strength (`GetThreshold(GetAccumulation(Trees), 0.0001f) / 4f`).
fn built_vegetation(context: &BuildContext) -> Map {
    let planted = context.accumulation("Trees", "Trees", None, 0);
    let present = aspect::threshold(&planted, 0.0001);
    present.divide_scalar(4.0)
}

/// Shrubs: low vegetation scattered like Trees, with denser patches.
pub struct Shrubs;

impl Shrubs {
    /// `Shrubs.BuildInhibitors`: the dead zones and where slope and
    /// altitude allow shrubs (old-style slope, always softened).
    fn inhibitors(heights: &Map, settings: &InhibitorSettings, simulation_bias: f32, terrain: (f32, f32)) -> (Map, Map) {
        let mut biased = heights.clone();
        if simulation_bias != 0.0 {
            maths::strengthen(&mut biased, simulation_bias);
        }
        let slope = aspect::slope_normalized_old(&biased, settings.slope[0] / 90.0, settings.slope[1] / 90.0, settings.slope_falloff / 90.0);
        let altitude = aspect::height_normalized(&biased, settings.altitude[0], settings.altitude[1], settings.altitude_falloff);
        let mut inhibitors = altitude.multiply(&slope);
        if settings.chaos > 0.0 {
            inhibitors = chaos_warp(&inhibitors, 0.05, settings.chaos, 0.5, 257 + settings.seed, terrain);
        }
        let blurred = fast_blur::blur_relative(&inhibitors, 0.025);
        let mut inhibitors = maths::clip_map(&blurred, 0.015, 1.0);
        maths::strengthen(&mut inhibitors, -0.5);

        let snow = SnowSettings {
            duration: settings.dead_flow,
            intensity: 0.35,
            settle_thaw: 0.9,
            melt: 0.05,
            snow_line: 0.0,
            real_scale: false,
            scale: 5000.0,
            verticality: settings.bias * 40000.0,
            slip_off: 20.0,
            adhered_mass: 12.0,
            model_scale: settings.model,
        };
        (dead_zones(&biased, &snow, terrain), inhibitors)
    }

    /// `Shrubs.BuildFreshwaterMap`.
    fn fresh_water(context: &BuildContext, heights: &Map, dead_zones: &Map) -> Map {
        let seed = context.i32("Seed");
        let chaos = context.f32("Chaos");
        let terrain = (context.terrain_width(), context.terrain_height());
        let settings = FlowSettings {
            flow_length: (1.0 - context.f32("Health")) * 2.0,
            flow_volume: context.f32("Spread") * 6.0,
            seed,
        };
        let flow = flow::simulate(heights, None, &settings);
        let mut fresh = histogram::equalize(flow);
        if context.enum_name("WaterAffinity") == "Avoid" {
            maths::invert(&mut fresh);
        }
        if chaos > 0.0 {
            fresh = chaos_warp(&fresh, 0.5, chaos, 0.65, 257 + seed, terrain);
        }
        let blurred = fast_blur::blur_relative(&fresh, 0.3);
        let clipped = maths::clip_map(&blurred, 0.015, 1.0);
        let fresh = maths::autolevel(clipped).add_scalar(context.f32("Random"));
        let mut fresh = fresh.subtract(dead_zones);
        maths::clamp_if_out_of_range(&mut fresh);
        fresh
    }
}

impl NodeBuilder for Shrubs {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let custom = custom_inhibition(context);
        let terrain = (context.terrain_width(), context.terrain_height());
        let seed = context.i32("Seed");
        let simulation_bias = context.f32("SimulationBias");
        let settings = inhibitor_settings(context);
        let (dead, inhibitors) = Shrubs::inhibitors(&input, &settings, simulation_bias, terrain);

        let mut heights = (*input).clone();
        if simulation_bias != 0.0 {
            maths::strengthen(&mut heights, simulation_bias);
        }
        let fresh = Shrubs::fresh_water(context, &heights, &dead);
        let cells = growth_cells(&fresh, &inhibitors, 200.0, seed, terrain);

        let area = (context.resolution.x * context.resolution.y) as i32 / 2;
        let shrub_count = context.i32("ShrubCount") * 1000;
        let scatter = random_noises::fixed_white(cells.resolution(), area.min(shrub_count), 0.001, context.bool("IsRelative"), seed);
        let mut shrubs = scatter.multiply(&cells).multiply(&fresh);

        let mut touched = false;
        if let Some(mut custom) = custom {
            let trim = context.f32("Trim");
            if trim > 0.0 {
                custom = maths::clip_map(&custom, trim, 1.0);
            }
            let influence = context.f32("CustomInfluence");
            if influence > 0.0 {
                custom = morphology::dilate(&custom, Kernel::Cross, 0.2 * influence, 1, 0.0);
            }
            shrubs = shrubs.subtract(&custom);
            touched = true;
        }
        if let Some(avoided) = avoided_trees(context)? {
            shrubs = shrubs.subtract(&avoided);
            touched = true;
        }
        if touched {
            maths::clamp_if_out_of_range(&mut shrubs);
        }
        if context.enum_name("Shape") != "Point" {
            let width = shrubs.width() as f32;
            let settings = ApertureSettings {
                size: width * (1.0 / width),
                kernel: ApertureKernel::Disk,
                erosion: false,
                vertices: 4,
                antialiased: false,
                direction: 0.0,
                corner_angle: 90.0,
            };
            shrubs = morphology2::aperture(&shrubs, &settings);
        }

        context.render_secondary_as(RenderIntent::Mask);
        let planted = input.add(&shrubs);
        context.commit(planted)?;
        context.commit_port("DeadZones", dead, RenderIntent::Default);
        context.commit_port("FreshWater", fresh, RenderIntent::Default);
        context.commit_port("Trees", shrubs, RenderIntent::Default);
        Ok(())
    }

    /// `Shrubs.GetCustomColorMap`: the shrubs (widened by a pixel above
    /// 1024 pixels, antialiased) in green over grey ground, with all the
    /// vegetation built faintly around them.
    fn custom_colour_map(&self, context: &BuildContext) -> Option<Map> {
        let shrubs = context.own_output("Trees")?;
        let mut planted = aspect::threshold(&shrubs, 0.0001);
        if planted.width() > 1024 {
            let width = planted.width() as f32;
            let settings = ApertureSettings {
                size: width * (1.0 / width),
                kernel: ApertureKernel::Disk,
                erosion: false,
                vertices: 4,
                antialiased: true,
                direction: 0.0,
                corner_angle: 90.0,
            };
            planted = morphology2::aperture(&planted, &settings);
        }
        let vegetation = planted.add(&built_vegetation(context));
        Some(colour::colorize(&vegetation, &FOREST_COLOURS))
    }
}
