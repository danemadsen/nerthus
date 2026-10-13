//! The Structure family: Repeat, Seamless and Shatter.

use nerthus_engine::{BuildContext, Map, NodeBuilder, NodeError, RenderIntent};

use crate::mosaic;
use crate::rock_core;

/// The second input (`Color`) of the tiling nodes, when connected.
fn color_input(context: &BuildContext) -> Option<std::sync::Arc<Map>> {
    let name = context.input_names().get(1).map(|name| name.to_string())?;
    context.input(&name)
}

/// `GetCustomColorMap` of the tiling nodes: their colour texture, when
/// they made one.
fn texture_colours(context: &BuildContext) -> Option<Map> {
    let texture = context.own_output("Texture")?;
    Some((*texture).clone())
}

/// Tiles a shrunk copy of the input across the map.
pub struct Repeat;

impl NodeBuilder for Repeat {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let tiles = context.i32("Tiles");
        let height = if context.bool("CompensateHeight") { 1.0 / tiles as f32 } else { context.f32("Height") };
        let first_channel = Map::from_data(input.resolution(), input.data().to_vec());
        let repeated = mosaic::repeat(&first_channel, tiles, height);
        context.commit(repeated)?;
        if let Some(color) = color_input(context) {
            let texture = mosaic::repeat(&color, tiles, 1.0);
            context.set_port_intent("Texture", RenderIntent::Color);
            context.commit_port("Texture", texture, RenderIntent::Color);
        }
        Ok(())
    }

    fn custom_colour_map(&self, context: &BuildContext) -> Option<Map> {
        texture_colours(context)
    }
}

/// Makes the input (and its colour texture) tile seamlessly.
pub struct Seamless;

impl NodeBuilder for Seamless {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let color = color_input(context);
        let (height, texture) = mosaic::seamless(&input, color.as_deref(), context.f32("Edge"), context.f32("ShiftX"), context.f32("ShiftY"));
        context.commit(height)?;
        if let Some(texture) = texture {
            context.commit_port("Texture", texture, RenderIntent::Color);
        }
        Ok(())
    }

    fn custom_colour_map(&self, context: &BuildContext) -> Option<Map> {
        texture_colours(context)
    }
}

/// Breaks the surface with scattered dents (`RockCore.Shatter`).
pub struct Shatter;

impl NodeBuilder for Shatter {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let terrain = (context.terrain_width(), context.terrain_height());
        let shattered = rock_core::shatter(&input, context.i32("Shatters"), context.f32("Size"), context.f32("Coverage"), context.i32("Seed"), terrain);
        context.commit(shattered)
    }
}
