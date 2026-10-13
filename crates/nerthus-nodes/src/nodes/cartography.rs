//! Cartography nodes: Contours and Grid.

use nerthus_engine::{BuildContext, NodeBuilder, NodeError, RenderIntent};

use crate::cartographer;

pub struct Contours;

impl NodeBuilder for Contours {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Mask);
        context.set_port_intent("CLUTRef", RenderIntent::Mask);
        let input = context.require_input("In")?;
        let maps = cartographer::contours(
            &input,
            context.i32("Count"),
            context.i32("Subdivision"),
            context.f32("Accuracy"),
            context.f32("LineIntensity"),
            context.f32("SubIntensity"),
            context.i32("LineThickness"),
        );
        context.commit(maps.lines)?;
        context.commit_port("CLUTRef", maps.terraces, RenderIntent::Default);
        Ok(())
    }
}

pub struct Grid;

impl NodeBuilder for Grid {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Mask);
        let map = cartographer::grid(
            context.resolution,
            context.i32("Subdivision"),
            context.f32("LineIntensity"),
            context.f32("SubIntensity"),
            context.i32("LineThickness"),
        );
        context.commit(map)
    }
}
