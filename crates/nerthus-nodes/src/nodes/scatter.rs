//! Scatter nodes: Pockmarks, Bomber, SBomber and Debris.

use nerthus_engine::{ApplicationMode, BuildContext, Map, NodeBuilder, NodeData, NodeError, RenderIntent};

use crate::colour;
use crate::debris::{self, DebrisSettings};
use crate::masking;
use crate::maths;
use crate::migrated::{self, PockmarkSettings};
use super::blend::add_numbered_port;
use crate::random::XorShift64Star;
use crate::scatters::{self, BombDistribution, BombSettings};
use crate::transformer::{self, AnchorPosition, BlendMode, TransformQuality};
use crate::x86;

/// Scattered pits (`Migrated.PockmarksProces`).
pub struct Pockmarks;

impl NodeBuilder for Pockmarks {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let settings = PockmarkSettings {
            size: context.float2("Size"),
            scale: context.f32("Scale"),
            height: context.f32("Height"),
            density: context.f32("Density"),
            multiplier: context.f32("Multiplier"),
            invert: context.bool("Invert"),
            seed: context.i32("Seed"),
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let marked = migrated::pockmarks(&input, context.resolution.x, &settings, terrain);
        context.commit(marked)
    }
}

fn transform_quality(name: &str) -> TransformQuality {
    match name {
        "Draft" => TransformQuality::Draft,
        "High" => TransformQuality::High,
        _ => TransformQuality::Medium,
    }
}

/// Every connected stamp input from `first` on, in port order.
fn stamp_inputs(context: &BuildContext, first: usize) -> Vec<Map> {
    let names: Vec<String> = context.input_names().iter().skip(first).map(|name| name.to_string()).collect();
    names.iter().filter_map(|name| context.input(name)).map(|map| (*map).clone()).collect()
}

/// Bomber: stamps scattered over the input.
pub struct Bomber;

impl NodeBuilder for Bomber {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let names: Vec<String> = context.input_names().iter().map(|name| name.to_string()).collect();
        if !context.is_connected("In") || !names.get(2).is_some_and(|name| context.is_connected(name)) {
            return Ok(());
        }
        let mut map = (*context.require_input("In")?).clone();
        let mut stamps = stamp_inputs(context, 2);
        let colour_count = stamps.iter().filter(|stamp| stamp.channel_count() > 1).count();
        if colour_count > 0 && colour_count < stamps.len() {
            return Err(NodeError::Failed("All inputs must be either grayscale or color. Mixing is not allowed.".to_string()));
        }
        if colour_count == stamps.len() {
            context.render_as(RenderIntent::Color);
            if map.channel_count() == 1 {
                map = Map::stack(&[&map, &map, &map]);
            }
        } else {
            context.render_as(RenderIntent::Heightfield);
        }
        let inhibit = context.input("Inhibition");
        let [size_low, size_high] = context.float2("Size");
        let [rotation_low, rotation_high] = context.float2("Rotation");
        let [height_low, height_high] = context.float2("Height");
        let settings = BombSettings {
            iterations: context.i32("Iterations"),
            multiplier: context.f32("Multiplier"),
            size: (size_low, size_high),
            size_distribution: context.f32("SizeDistribution"),
            rotation: (rotation_low, rotation_high),
            maintain_proportions: context.bool("MaintainProportions"),
            variable_height: context.bool("VariableHeight"),
            height: (height_low, height_high),
            fade_edges: context.bool("FadeEdges"),
            seed: context.i32("Seed"),
            jitter: context.f32("Jitter"),
            distribution: if context.enum_name("Distribution") == "Grid" { BombDistribution::Grid } else { BombDistribution::Random },
            mode: BlendMode::from_name(context.enum_name("BlendMode")),
            quality: transform_quality(context.enum_name("Quality")),
        };
        let bombed = scatters::bomb(&map, &mut stamps, inhibit.as_deref(), &settings);
        context.commit(bombed)
    }

    fn add_new_port(&self, node: &mut NodeData) -> Option<String> {
        add_numbered_port(node, "Input", 10)
    }
}

/// SBomber: stamps scattered on a seamless cell grid.
pub struct SBomber;

impl NodeBuilder for SBomber {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let names: Vec<String> = context.input_names().iter().map(|name| name.to_string()).collect();
        if !context.is_connected("In") || !names.get(1).is_some_and(|name| context.is_connected(name)) {
            return Ok(());
        }
        let size_distribution = context.f32("SizeDistribution");
        let normalizer = 1.0 / (maths::exp(size_distribution * 10.0 + 1e-6) - 1.0);
        let mut stamps = stamp_inputs(context, 1);
        let mode = BlendMode::from_name(context.enum_name("BlendMode"));
        let mut mask = None;
        if context.bool("FadeEdges") {
            if mode != BlendMode::Blend {
                for stamp in stamps.iter_mut() {
                    *stamp = masking::mask_by_square_edge(stamp, 0.7);
                }
            } else {
                mask = Some(masking::square_edge_mask(context.resolution, 0.9));
            }
        }
        let quality = transform_quality(context.enum_name("Quality"));
        let [size_low, size_high] = context.float2("Size");
        let [rotation_low, rotation_high] = context.float2("Rotation");
        let [height_low, height_high] = context.float2("Height");
        let variable_height = context.bool("VariableHeight");
        let maintain_proportions = context.bool("MaintainProportions");
        let offset = (context.f32("OffsetX"), context.f32("OffsetY"));
        let instances = scatters::scatter(context.i32("Density"), context.f32("GlobalScale"), offset, context.i32("Seed"));
        let mut scattered = Map::new(context.resolution, 1);
        for instance in &instances {
            let mut random = XorShift64Star::new(instance.hash);
            let stamp = &stamps[random.next_range(0, stamps.len() as i32) as usize];
            let mut lift = if variable_height {
                random.next_float_range(height_low, height_high)
            } else {
                random.next_float();
                1.0
            };
            let mut size = random.next_float();
            size = (maths::exp(size * size_distribution * 10.0 + 1e-6) - 1.0) * normalizer;
            size = size_low + (size_high - size_low) * size;
            if maintain_proportions {
                lift *= size * instance.scale.0;
            }
            let scale = (size * instance.scale.0, size * instance.scale.1);
            let rotation = 360.0 * random.next_float_range(rotation_low, rotation_high);
            transformer::multi_transform(stamp, &mut scattered, instance.position, scale, rotation, lift, mode, quality, mask.as_ref(), true, AnchorPosition::Center);
        }
        match context.input("In") {
            Some(input) => context.commit(input.add(&scattered)),
            None => context.commit(scattered),
        }
    }

    fn add_new_port(&self, node: &mut NodeData) -> Option<String> {
        add_numbered_port(node, "Input", 10)
    }
}

/// Debris: rocks dropped on the terrain (where the emitter marks), rolled
/// downhill and rendered where they stop (`DebrisCore`).
pub struct Debris;

impl NodeBuilder for Debris {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let emitter = context.input("Emitter");
        let removal = context.input("Removal");
        let amount = context.i32("DebrisAmount") as f32 * context.f32("AmountMultiplier");
        let scale = context.f32("Scale");
        let [min_size, max_size] = context.float2("Size");
        let terrain = (context.terrain_width(), context.terrain_height());
        let settings = DebrisSettings {
            amount: x86::cvttss2si(amount),
            seed: context.i32("Seed"),
            friction: context.f32("Friction"),
            restitution: context.f32("Restitution"),
            heightmap_size: terrain.0 / scale,
            scene_height: terrain.1 / scale,
            min_size,
            max_size,
            natural_sizes: context.enum_name("Distribution") == "Natural",
            render_still_rocks: context.bool("RenderStillRocks"),
            rock_height: context.f32("Height"),
            sharp: context.enum_name("Shape") == "Sharp",
            terrain,
        };
        context.render_secondary_as(RenderIntent::Mask);
        // Outside the editor, Export Point Cloud keeps the rocks' positions
        // for a PointCloud node downstream.
        let export_points = context.bool("ExportPointCloud") && context.application_mode() != ApplicationMode::Ui;
        let mut points = Vec::new();
        let point_cloud = if export_points { Some(&mut points) } else { None };
        let maps = debris::simulate(&input, emitter.as_deref(), removal.as_deref(), &settings, point_cloud);
        if export_points {
            context.remember(debris::DebrisPointCloud(points));
        }
        context.commit(maps.heights)?;
        context.commit_port("ColorIndex", maps.colour_index, RenderIntent::Mask);
        context.commit_port("Debris", maps.debris, RenderIntent::Mask);
        Ok(())
    }

    /// `Debris.GetCustomColorMap`: each rock a shade from dark to light
    /// grey by its colour index, over mid-grey ground.
    fn custom_colour_map(&self, context: &BuildContext) -> Option<Map> {
        let resolution = context.resolution;
        let ground_plane = vec![0.45; resolution.area()];
        let ground = Map::from_channels(resolution, vec![ground_plane.clone(), ground_plane.clone(), ground_plane]);
        let colour_index = context.own_output("ColorIndex")?;
        let rocks = colour::colorize(&colour_index, &[[0.2, 0.2, 0.2], [0.8, 0.8, 0.8]]);
        let debris = context.own_output("Debris")?;
        Some(masking::mask(&ground, &rocks, &debris))
    }
}
