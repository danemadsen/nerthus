//! Gradient primitives: LinearGradient, RadialGradient, Cone, Hemisphere
//! and Constant.

use nerthus_engine::{BuildContext, Map, NodeBuilder, NodeError, RenderIntent, Resolution};

use crate::gradients::{self, EdgeBehaviour};

/// Every gradient primitive multiplies its result by its `In` map when one
/// is connected.
fn multiply_by_input(context: &BuildContext, map: Map) -> Map {
    match context.input("In") {
        Some(input) => map.multiply(&input),
        None => map,
    }
}

pub struct LinearGradient;

impl NodeBuilder for LinearGradient {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let size = context.resolution.x;
        let half = size as f32 / 2.0;
        let length = context.f32("Scale") * size as f32;
        let direction = context.i32("Direction") as f32;
        let edge = EdgeBehaviour::from_name(context.enum_name("Edge"));
        let gradient = gradients::linear(size, half, half, length, direction, edge);
        let map = multiply_by_input(context, gradient);
        context.commit(map)
    }
}

pub struct RadialGradient;

impl NodeBuilder for RadialGradient {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let size = context.resolution.x as f32;
        let center_x = size * context.f32("X");
        let center_y = size * context.f32("Y");
        let radius = size * (context.f32("Scale") * 0.5);
        let gradient = gradients::radial(context.resolution.x, center_x, center_y, radius, context.f32("Height"));
        let map = multiply_by_input(context, gradient);
        context.commit(map)
    }
}

pub struct Cone;

impl NodeBuilder for Cone {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let size = context.resolution.x as f32;
        let center_x = size * context.f32("X");
        let center_y = size * context.f32("Y");
        let radius = size * (context.f32("Scale") * 0.5);
        let gradient = gradients::cone(context.resolution.x, center_x, center_y, radius, context.f32("Height"));
        let map = multiply_by_input(context, gradient);
        context.commit(map)
    }
}

/// `Migrated.HemisphereProcess`: the radius leaves a 2-pixel margin.
pub struct Hemisphere;

impl NodeBuilder for Hemisphere {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let size = context.resolution.x;
        let resolution = Resolution::square(size);
        let center_x = size as f32 * context.f32("X");
        let center_y = size as f32 * context.f32("Y");
        let radius = (size as i64 - 4) as f32 * (context.f32("Scale") * 0.5);
        let height = context.f32("Height");
        let dome = if context.bool("Flatten") {
            gradients::hemisphere_flattened(resolution, center_x, center_y, radius, height)
        } else {
            gradients::hemisphere(resolution, center_x, center_y, radius, height)
        };
        let map = multiply_by_input(context, dome);
        context.commit(map)
    }
}

/// A flat height, or a flat colour when `Output` is `Color`.
pub struct Constant;

impl NodeBuilder for Constant {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let as_color = context.enum_name("Output") == "Color";
        let intent = if as_color { RenderIntent::Color } else { RenderIntent::Heightfield };
        context.render_as(intent);
        let mut map = if as_color {
            let color = context.color("Color");
            let mut colored = Map::new(context.resolution, 3);
            for (channel, value) in color.iter().take(3).enumerate() {
                colored.channel_mut(channel).fill(*value);
            }
            colored
        } else {
            let mut flat = Map::new(context.resolution, 1);
            flat.fill(context.f32("Height"));
            flat
        };

        if let Some(input) = context.input("In") {
            map = if as_color {
                let grey = Map::stack(&[&input, &input, &input]);
                map.multiply(&grey)
            } else {
                map.multiply(&input)
            };
        }
        context.commit(map)
    }
}
