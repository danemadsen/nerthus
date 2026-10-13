//! The line, engraving and dot patterns (`Patterns`).

use nerthus_engine::{Map, Resolution};

use crate::aspect::{self, CurvatureType};
use crate::fast_blur;
use crate::gradients::{self, EdgeBehaviour as GradientEdge};
use crate::profiles;
use crate::transformer::{self, AnchorPosition, BlendMode, EdgeBehaviour};
use crate::warps;
use crate::x86;

/// The side of the square the line patterns are drawn on before cropping:
/// half as wide again, so rotation leaves no gaps.
fn drawing_span(size: usize) -> usize {
    x86::cvttss2si(size as f32 * 1.5) as usize
}

/// `Patterns.Lines`: parallel ridges, the vertical curvature of a terraced
/// ramp running at `direction` degrees.
pub fn lines(size: usize, steps: i32, direction: i32) -> Map {
    let span = drawing_span(size);
    let centre = span as f32 / 2.0;
    let ramp = gradients::linear(span, centre, centre, span as f32, direction as f32, GradientEdge::Mirror);
    let half_span = x86::cvttss2si(span as f32 / 2.0);
    let count = half_span.min(steps).max(0) as usize;
    let terraced = profiles::terrace(&ramp, count, 1.0, 1.0, 1.0, 0, false);
    let ridges = aspect::curvature(&terraced, 0.0, 1.0, 1.0, CurvatureType::Vertical);
    transformer::crop(&ridges, span / 4, span / 4, Resolution::square(size))
}

/// `Patterns.Engrave`: lines bent across their run by a blurred ramp.
pub fn engrave(size: usize, scale: f32, softness: f32, steps: i32, direction: i32) -> Map {
    let span = drawing_span(size);
    let lines = lines(span, steps, direction);
    let centre = span as f32 / 2.0;
    let across = (direction + 90) as f32;
    let ramp = gradients::linear(span, centre, centre, span as f32 * scale, across, GradientEdge::Mirror);
    let bend = fast_blur::blur_relative(&ramp, softness * 20.0);
    let warped = warps::direction_warp_map(&lines, &bend, softness * 0.5, across, EdgeBehaviour::Mirror);
    transformer::crop(&warped, span / 4, span / 4, Resolution::square(size))
}

/// `Patterns.Dots`: a grid of round dots `spacing` tenths of the width
/// apart, every other row shifted half a cell when `jittered`. A spacing
/// under one pixel divides by zero.
pub fn dots(size: usize, spacing: f32, dot_size: f32, jittered: bool) -> Option<Map> {
    let cell = x86::cvttss2si(size as f32 * (spacing * 0.1));
    if cell <= 0 {
        return None;
    }
    let half = cell as f32 / 2.0;
    let disc = gradients::radial(cell as usize, half, half, half, 1.0);
    let dot = aspect::threshold(&disc, 1.0 - dot_size);
    let mut map = Map::new(Resolution::square(size), 1);
    let per_side = (((size as i32 / cell) as f32 / 2.0).ceil()) as i32;
    let mut shift = 0.0f32;
    let mut shifted = false;
    let mut positions = Vec::with_capacity((per_side * per_side).max(0) as usize);
    for row in 0..per_side {
        for column in 0..per_side {
            let x = (column as f32 + 0.5 + shift) / per_side as f32;
            let y = (row as f32 + 0.5) / per_side as f32;
            positions.push((x, y));
        }
        if jittered {
            shifted = !shifted;
            shift = if shifted { 0.5 } else { 0.0 };
        }
    }
    for position in positions {
        transformer::insert(&dot, &mut map, position, BlendMode::Blend, None, AnchorPosition::TopLeft);
    }
    Some(map)
}
