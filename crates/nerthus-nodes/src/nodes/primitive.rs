//! Primitive shapes: Island, Shape, Draw, Cellular3D and Pattern.

use nerthus_engine::{BuildContext, Map, NodeBuilder, NodeError, RenderIntent, Resolution};

use crate::cellular3d::{self, Cellular3dSettings};
use crate::fast_blur;
use crate::maths::{self, dotnet_min};
use crate::migrated;
use crate::morphology::{self, Kernel};
use crate::patterns;
use crate::vector_mask;

/// A warped island coastline (`Migrated.IslandProcess`).
pub struct Island;

impl NodeBuilder for Island {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.input("In");
        let terrain = (context.terrain_width(), context.terrain_height());
        let island = migrated::island(input.as_deref(), context.resolution.x, context.i32("Seed"), context.f32("Size"), context.f32("Chaos"), terrain);
        context.commit(island)
    }
}

/// `Shape.GenerateOval`: an elliptical ring `thickness` of its size wide.
fn oval(size: usize, size_x: f32, size_y: f32, thickness: f32, center_x: f32, center_y: f32) -> Map {
    let half = size as f32 / 2.0;
    let radius_x = size_x * half;
    let radius_y = size_y * half;
    let ring_x = thickness * radius_x / 2.0;
    let ring_y = thickness * radius_y / 2.0;
    let inner_x = radius_x - ring_x;
    let inner_y = radius_y - ring_y;
    let outer_x = radius_x + ring_x;
    let outer_y = radius_y + ring_y;
    let middle_x = center_x * size as f32;
    let middle_y = center_y * size as f32;
    let mut values = vec![0.0f32; size * size];
    for row in 0..size {
        let dy = row as f32 + 0.5 - middle_y;
        for column in 0..size {
            let dx = column as f32 + 0.5 - middle_x;
            let inner = dx * dx / (inner_x * inner_x) + dy * dy / (inner_y * inner_y);
            let outer = dx * dx / (outer_x * outer_x) + dy * dy / (outer_y * outer_y);
            values[row * size + column] = if inner >= 1.0 && outer <= 1.0 { 1.0 } else { 0.0 };
        }
    }
    Map::from_data(Resolution::square(size), values)
}

/// `Shape.GenerateRectangle`: a rectangular frame.
fn rectangle(size: usize, size_x: f32, size_y: f32, thickness: f32, center_x: f32, center_y: f32) -> Map {
    let half = size as f32 / 2.0;
    let half_width = size_x * half / 2.0;
    let half_height = size_y * half / 2.0;
    let frame = thickness * dotnet_min(half_width, half_height);
    let middle_x = center_x * size as f32;
    let middle_y = center_y * size as f32;
    let inner = (middle_x - half_width + frame, middle_x + half_width - frame, middle_y - half_height + frame, middle_y + half_height - frame);
    let outer = (middle_x - half_width, middle_x + half_width, middle_y - half_height, middle_y + half_height);
    let inside = |bounds: (f32, f32, f32, f32), x: f32, y: f32| x >= bounds.0 && x <= bounds.1 && y >= bounds.2 && y <= bounds.3;
    Map::generate(Resolution::square(size), |x, y| {
        let column = x + 0.5;
        let row = y + 0.5;
        if inside(outer, column, row) && !inside(inner, column, row) { 1.0 } else { 0.0 }
    })
}

/// Shape: an oval or rectangular frame.
pub struct Shape;

impl NodeBuilder for Shape {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let size = context.resolution.x;
        let uniform = context.bool("Uniform");
        let scale = context.f32("Scale");
        let size_x = if uniform { scale } else { context.f32("ScaleX") };
        let size_y = if uniform { scale } else { context.f32("ScaleY") };
        let thickness = context.f32("Thickness");
        let x = context.f32("X");
        let y = context.f32("Y");
        let shape = match context.enum_name("Geometry") {
            "Rectangle" => rectangle(size, size_x, size_y, thickness, x, y),
            _ => oval(size, size_x, size_y, thickness, x, y),
        };
        context.commit(shape.multiply_scalar(context.f32("Height")))
    }
}

/// Draw: brush strokes painted in the editor (`StrokeData`), softened and
/// scaled to `Height`.
pub struct Draw;

impl NodeBuilder for Draw {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let stroke_data = context.string("StrokeData");
        if stroke_data.is_empty() {
            return context.commit(Map::new(context.resolution, 1));
        }
        let strokes = if stroke_data.trim().is_empty() {
            Vec::new()
        } else {
            vector_mask::parse_stroke_data(stroke_data).map_err(|_| NodeError::Failed("Stroke deserialization failed.".to_string()))?
        };
        let painted = vector_mask::render(&strokes, context.resolution.x);
        let mut map = Map::from_data(context.resolution, painted);
        let soften = context.f32("Soften");
        if soften > 0.0 {
            map = fast_blur::blur_relative(&map, soften);
        }
        let height = context.f32("Height");
        if height < 1.0 {
            map = maths::fit_range(&map, 0.0, height);
        }
        context.commit(map)
    }
}

/// Cellular3D's `NormalizeInput`: maps [0.5, 1] onto [0, 1].
fn normalize_input(value: f32) -> f32 {
    (value - 0.5) * 2.0
}

/// Cellular3D: the terrain filled with 3D Voronoi cells cut off at its
/// surface, with each pixel's cell hash as a second output.
pub struct Cellular3d;

impl NodeBuilder for Cellular3d {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.input("In");
        let gap_input = context.input("Gap");
        let (heights, gap_map) = if context.bool("NormalizeInput") {
            let heights = match input {
                Some(map) => map.map_values(normalize_input),
                None => Map::filled(context.resolution, 0.0),
            };
            let gap_map = gap_input.map(|map| map.map_values(normalize_input));
            (heights, gap_map)
        } else {
            let heights = match input {
                Some(map) => (*map).clone(),
                None => Map::filled(context.resolution, 0.5),
            };
            let gap_map = gap_input.map(|map| (*map).clone());
            (heights, gap_map)
        };

        let settings = Cellular3dSettings {
            cells: 1.0 / context.f32("Size"),
            gap: context.f32("Gap"),
            scale: [context.f32("ScaleX"), context.f32("ScaleY"), context.f32("ScaleZ")],
            jitter: [context.f32("JitterX"), context.f32("JitterY"), context.f32("JitterZ")],
            seed: context.i32("Seed") as u32,
            terrain_width: context.terrain_width(),
            terrain_height: context.terrain_height(),
        };
        let (surface, hashes) = cellular3d::render(&heights, gap_map.as_ref(), &settings);
        context.commit(surface)?;
        context.commit_port("Hashmap", hashes, RenderIntent::Mask);
        Ok(())
    }
}

/// Pattern: lines, engraved lines, a dot grid or staggered dots
/// (`Patterns`), thickened by the pixel size and masked by the input.
pub struct Pattern;

impl NodeBuilder for Pattern {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let size = context.resolution.x;
        let line_size = context.f32("Size");
        let steps = context.i32("Steps");
        let direction = context.i32("Direction");
        let spacing = context.f32("Spacing");
        let dot_size = context.f32("DotSize");
        let mut map = match context.enum_name("Type") {
            "Engraving" => patterns::engrave(size, line_size, line_size, steps, direction),
            "Grid" => patterns::dots(size, spacing, dot_size, false).ok_or_else(divided_by_zero)?,
            "Dots" => patterns::dots(size, spacing, dot_size, true).ok_or_else(divided_by_zero)?,
            _ => patterns::lines(size, steps, direction),
        };
        let pixel_size = context.i32("PixelSize");
        if pixel_size > 1 {
            let kernel_size = 1.0 / size as f32 * pixel_size as f32;
            map = morphology::dilate(&map, Kernel::Cross, kernel_size, 1, 0.0);
        }
        context.render_as(RenderIntent::Mask);
        if let Some(input) = context.input("In") {
            map = map.multiply(&input);
        }
        context.commit(map)
    }
}

fn divided_by_zero() -> NodeError {
    NodeError::Failed("Attempted to divide by zero.".to_string())
}
