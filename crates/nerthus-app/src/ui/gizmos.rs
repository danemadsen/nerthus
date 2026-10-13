//! Gizmos (the property area's Toggle Gizmo, Z): handles over the docked
//! 2D view for nodes with a `[Gizmo]` (`Gizmo2DLayer` with a
//! `Gizmo2DControl` for `XY`, `TS` and `Y1`; `GizmoSRTLayer` with a
//! `GizmoSRT` for `XYZ` and `SRT`). The centre moves the node's position,
//! the ring scales it (dragged sideways), and an SRT gizmo's triangle sets
//! its height and its dot its rotation (dragged up and down). Each node
//! reads its pose and takes the gizmo's back as its own `Gizmo2DPosition`,
//! `GizmoRotateScale` and `Gizmo2DHit` do.

use egui::{Color32, CursorIcon, Id, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};
use nerthus_engine::json::f32_value;
use nerthus_engine::{Catalogue, Map, NodeData};
use nerthus_render::{HandleAxes, PositionHandle, TerrainPreview};
use serde_json::Value;

use crate::editor::Editor;
use crate::theme;
use crate::ui::commands::Command;

/// `Color`'s default: purple (188, 0, 255).
const GIZMO_COLOUR: Color32 = Color32::from_rgb(188, 0, 255);
/// `ScaleStep`, `HeightStepPerPixel`, `RotateStepPerPixel` and
/// `RotateSnapDegrees`.
const SCALE_STEP: f32 = 0.01;
const HEIGHT_STEP: f32 = 0.005;
const ROTATE_STEP: f32 = 0.5;
const ROTATE_SNAP: f32 = 15.0;
/// The scale ring (`ScaleHandle`): 83 across, 4 thick.
const RING_OUTER: f32 = 41.0;
const RING_INNER: f32 = 37.0;

/// `GizmoType`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GizmoKind {
    Xy,
    Xyz,
    Ts,
    Srt,
    Y1,
}

impl GizmoKind {
    /// The node type's `[Gizmo]`.
    pub fn of(type_name: &str) -> Option<GizmoKind> {
        let spec = Catalogue::get().node(type_name)?;
        let name = spec.gizmo.as_ref()?.first()?.as_str()?;
        let kind = match name {
            "XY" => GizmoKind::Xy,
            "XYZ" => GizmoKind::Xyz,
            "TS" => GizmoKind::Ts,
            "SRT" => GizmoKind::Srt,
            "Y1" => GizmoKind::Y1,
            _ => return None,
        };
        Some(kind)
    }

    /// `GizmoSRT` rather than `Gizmo2DControl`.
    fn is_srt(self) -> bool {
        matches!(self, GizmoKind::Xyz | GizmoKind::Srt)
    }

    /// The scale ring shows (`TS` layers and `GizmoSRT`, whose `CanScale`
    /// stays on).
    fn can_scale(self) -> bool {
        matches!(self, GizmoKind::Ts | GizmoKind::Xyz | GizmoKind::Srt)
    }

    fn can_rotate(self) -> bool {
        self == GizmoKind::Srt
    }
}

/// Where a gizmo stands: X and Y across the map (0–1, down from the top),
/// Z its height (0–2), its rotation in degrees and its scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GizmoPose {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub rotate: f32,
    pub scale: f32,
}

fn number(node: &NodeData, name: &str) -> f32 {
    node.values.get(name).and_then(Value::as_f64).unwrap_or(0.0) as f32
}

/// `Gizmo2DPosition` and `GizmoRotateScale`, coerced as the gizmo's own
/// properties are (X and Y to 0–1, Z to 0–2, scale at least 0.01). The
/// scale comes from the node for every kind, so a `TS` gizmo's first drag
/// keeps the node's scale.
pub fn pose_of(node: &NodeData) -> GizmoPose {
    let value = |name: &str| number(node, name);
    // −1…1 to 0…1.
    let unit = |value: f32| (value + 1.0) * 0.5;
    let (x, y, z, rotate, scale) = match node.type_name.as_str() {
        "Cone" | "Hemisphere" | "RadialGradient" => (value("X"), value("Y"), value("Height"), 0.0, value("Scale")),
        "Crater" | "Mountain" => (value("X"), value("Y"), 0.0, 0.0, value("Scale")),
        "Lake" => (unit(value("X")), unit(value("Y")), 0.0, 0.0, value("WaterLevel")),
        "Perlin" | "Voronoi" => (value("X"), 1.0 - value("Y"), 0.0, 0.0, 0.0),
        "Swirl" | "Pond" => (value("X"), value("Y"), 0.0, 0.0, 0.0),
        "Snow" => (0.5, 0.5, value("SnowLine"), 0.0, 0.0),
        "Transform" => (unit(value("OffsetX")), unit(value("OffsetY")), value("OffsetZ"), value("Rotate"), value("Scale")),
        "Transform3D" => (value("OffsetX") + 0.5, 1.0 - (value("OffsetY") + 0.5), 0.0, 0.0, 0.0),
        _ => (0.0, 0.0, 0.0, 0.0, 0.0),
    };
    GizmoPose {
        x: x.clamp(0.0, 1.0),
        y: y.clamp(0.0, 1.0),
        z: z.clamp(0.0, 2.0),
        rotate: wrap_degrees(rotate),
        scale: scale.max(0.01),
    }
}

/// `Coercions.CoerceAngle`.
fn wrap_degrees(degrees: f32) -> f32 {
    let wrapped = degrees % 360.0;
    if wrapped < 0.0 { wrapped + 360.0 } else { wrapped }
}

/// `Gizmo2DHit`: the node's values for a pose (an `XY` layer passes Z = 1,
/// rotation 0). Snow takes the height of its input under the gizmo. Pond
/// and Transform3D only answer the 3D gizmo (`GizmoHit`, where north is
/// up); here they take the same point. TriplanarDisplacement answers
/// neither.
pub fn hit_values(node: &NodeData, pose: &GizmoPose, kind: GizmoKind, snow_input: Option<&Map>) -> Vec<(String, Value)> {
    let float = |value: f32| f32_value(value);
    let clamp01 = |value: f32| value.clamp(0.0, 1.0);
    // 0…1 to −1…1.
    let signed = |value: f32| value * 2.0 - 1.0;
    let z = if kind.is_srt() { pose.z } else { 1.0 };
    let rotate = if kind.is_srt() { pose.rotate } else { 0.0 };
    let set = |pairs: &[(&str, f32)]| pairs.iter().map(|(name, value)| (name.to_string(), float(*value))).collect::<Vec<_>>();
    match node.type_name.as_str() {
        "Cone" | "Hemisphere" | "RadialGradient" => set(&[("X", pose.x), ("Y", pose.y), ("Height", clamp01(z)), ("Scale", pose.scale)]),
        "Crater" | "Mountain" => set(&[("X", pose.x), ("Y", pose.y), ("Scale", pose.scale)]),
        "Lake" => set(&[("X", signed(pose.x)), ("Y", signed(pose.y)), ("WaterLevel", clamp01(pose.scale))]),
        "Perlin" | "Voronoi" => set(&[("X", pose.x), ("Y", 1.0 - pose.y)]),
        "Swirl" | "Pond" => set(&[("X", pose.x), ("Y", pose.y)]),
        "Transform" => set(&[("OffsetX", signed(pose.x)), ("OffsetY", signed(pose.y)), ("OffsetZ", z), ("Scale", pose.scale), ("Rotate", rotate)]),
        "Transform3D" => set(&[("OffsetX", pose.x - 0.5), ("OffsetY", (1.0 - pose.y) - 0.5)]),
        "Snow" => {
            // `heightCheck[(int)(x · width), (int)(y · height)]`.
            let Some(input) = snow_input else {
                return Vec::new();
            };
            let column = ((pose.x * input.width() as f32) as usize).min(input.width() - 1);
            let row = ((pose.y * input.height() as f32) as usize).min(input.height() - 1);
            let line = input.data()[row * input.width() + column];
            set(&[("SnowLine", line)])
        }
        _ => Vec::new(),
    }
}

/// Which part of the gizmo is held.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Handle {
    Move,
    Scale,
    Height,
    Rotate,
}

/// A drag under way: the handle, where it started and how.
#[derive(Clone, Debug)]
struct GizmoDrag {
    handle: Handle,
    start_pointer: Pos2,
    start_pose: GizmoPose,
    /// `_mouseOffsetFromCenter`, in map fractions.
    offset: Vec2,
    /// `_scaleDir`: the ring's left half scales the other way.
    scale_direction: f32,
    /// `_rawRotateDegrees`.
    raw_rotate: f32,
}

/// The gizmo of the node it was turned on for.
#[derive(Clone, Debug)]
pub struct GizmoState {
    node: i32,
    kind: GizmoKind,
    pose: GizmoPose,
    drag: Option<GizmoDrag>,
}

/// What a gizmo drag did this frame: the values to set, and whether the
/// drag began (the edit's undo point) or ended.
#[derive(Clone, Debug, PartialEq)]
pub struct GizmoEdit {
    pub node: i32,
    pub values: Vec<(String, Value)>,
    pub started: bool,
    pub finished: bool,
}

impl GizmoState {
    pub fn new(node: i32, kind: GizmoKind) -> Self {
        let pose = GizmoPose { x: 0.0, y: 0.0, z: 0.0, rotate: 0.0, scale: 1.0 };
        Self { node, kind, pose, drag: None }
    }

    pub fn node(&self) -> i32 {
        self.node
    }

    pub fn kind(&self) -> GizmoKind {
        self.kind
    }

    /// Whether one of its handles is held.
    pub fn held(&self) -> bool {
        self.drag.is_some()
    }
}

/// Draws the gizmo over the map (`image`, the map's square as shown; the
/// gizmo grows and shrinks with the view's `zoom`) and runs its drags.
pub fn gizmo_layer(ui: &mut Ui, image: Rect, clip: Rect, zoom: f32, node: &NodeData, state: &mut GizmoState, snow_input: Option<&Map>) -> Option<GizmoEdit> {
    if state.drag.is_none() {
        // `UpdatePositionInViewportLayers`: the pose follows the node.
        state.pose = pose_of(node);
    }
    let kind = state.kind;
    let unit = zoom.max(0.2);
    let centre = image.min + Vec2::new(state.pose.x, state.pose.y) * image.width();
    let painter = ui.painter_at(clip);

    // The parts' hit areas, topmost last.
    let move_area = Rect::from_center_size(centre, Vec2::splat(32.0 * unit));
    let ring_area = Rect::from_center_size(centre, Vec2::splat(2.0 * RING_OUTER * unit));
    let rotate_centre = centre + Vec2::new(RING_OUTER * unit + 14.0 * unit, 0.0);
    let rotate_area = Rect::from_center_size(rotate_centre, Vec2::splat(12.0 * unit));
    let height_centre = centre - Vec2::new(0.0, RING_OUTER * unit + 21.0 * unit - 9.0 * unit);
    let height_area = Rect::from_center_size(height_centre, Vec2::splat(18.0 * unit));
    let id = Id::new(("gizmo", state.node));
    let ring = kind.can_scale().then(|| ui.interact(ring_area, id.with("scale"), Sense::drag()));
    let ring_hovered = ring.as_ref().is_some_and(|response| {
        let distance = response.hover_pos().map(|pointer| pointer.distance(centre));
        distance.is_some_and(|distance| distance >= RING_INNER * unit - 2.0 && distance <= RING_OUTER * unit + 2.0)
    });
    let movement = ui.interact(move_area, id.with("move"), Sense::drag());
    let height = kind.is_srt().then(|| ui.interact(height_area, id.with("height"), Sense::drag()));
    let rotate = kind.can_rotate().then(|| ui.interact(rotate_area, id.with("rotate"), Sense::drag()));

    // Painting: shadow, rings, the colour, the cross, the scale ring, the
    // height triangle and the rotation dot.
    let extent = if kind.can_scale() { RING_OUTER + 0.5 } else { 24.0 } * unit;
    painter.add(Shape::circle_filled(centre, extent, Color32::from_black_alpha(0x50)));
    painter.add(Shape::circle_filled(centre, extent * 0.6, Color32::from_black_alpha(0x40)));
    painter.circle_stroke(centre, 17.0 * unit, Stroke::new(1.0, Color32::BLACK));
    painter.circle_stroke(centre, 16.0 * unit - 1.0, Stroke::new(2.0, Color32::WHITE));
    painter.circle(centre, 14.0 * unit, GIZMO_COLOUR.gamma_multiply(0.65), Stroke::new(1.5, GIZMO_COLOUR.gamma_multiply(0.65)));
    let tick = 18.0 * unit / 24.0;
    for (from, to) in [((19.0, 12.0), (15.95, 12.0)), ((5.0, 12.0), (8.05, 12.0)), ((12.0, 19.0), (12.0, 15.95)), ((12.0, 5.0), (12.0, 8.05))] {
        let start = centre + Vec2::new((from.0 - 12.0) * tick, (from.1 - 12.0) * tick);
        let end = centre + Vec2::new((to.0 - 12.0) * tick, (to.1 - 12.0) * tick);
        painter.line_segment([start, end], Stroke::new(1.0 * unit.min(1.5), Color32::BLACK));
    }
    if kind.can_scale() {
        let middle = (RING_OUTER + RING_INNER) / 2.0 * unit;
        let thickness = (RING_OUTER - RING_INNER) * unit;
        painter.circle_stroke(centre, middle, Stroke::new(thickness + 2.0, Color32::BLACK));
        painter.circle_stroke(centre, middle, Stroke::new(thickness, Color32::WHITE));
    }
    if kind.is_srt() {
        let hovered = height.as_ref().is_some_and(|response| response.hovered() || response.dragged());
        let fill = if hovered { theme::ACCENT_PRIMARY } else { GIZMO_COLOUR };
        let scale = 18.0 * unit / 24.0;
        let corner = |x: f32, y: f32| height_centre + Vec2::new((x - 12.0) * scale, (y - 12.0) * scale);
        let triangle = vec![corner(12.0, 4.0), corner(20.0, 18.0), corner(4.0, 18.0)];
        painter.add(Shape::convex_polygon(triangle, fill, Stroke::new(2.0 * scale, Color32::BLACK)));
    }
    if kind.can_rotate() {
        let hovered = rotate.as_ref().is_some_and(|response| response.hovered() || response.dragged());
        let fill = if hovered { theme::ACCENT_PRIMARY } else { GIZMO_COLOUR };
        painter.circle(rotate_centre, 6.0 * unit, fill, Stroke::new(1.0, Color32::BLACK));
    }

    // Cursors (`SizeAll`, `SizeWE`, `SizeNS`).
    if movement.hovered() || movement.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::Move);
    } else if height.as_ref().is_some_and(|response| response.hovered()) || rotate.as_ref().is_some_and(|response| response.hovered()) {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
    } else if ring_hovered {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }

    // `Handle_MouseLeftButtonDown`.
    let pointer = ui.input(|input| input.pointer.interact_pos());
    let pressed = |response: &Option<egui::Response>| response.as_ref().is_some_and(|response| response.drag_started_by(egui::PointerButton::Primary));
    let mut started = false;
    if state.drag.is_none()
        && let Some(pointer) = pointer
    {
        let handle = if pressed(&rotate) {
            Some(Handle::Rotate)
        } else if pressed(&height) {
            Some(Handle::Height)
        } else if movement.drag_started_by(egui::PointerButton::Primary) {
            Some(Handle::Move)
        } else if pressed(&ring) && ring_hovered {
            Some(Handle::Scale)
        } else {
            None
        };
        if let Some(handle) = handle {
            let offset = (pointer - centre) / image.width();
            let scale_direction = if pointer.x < centre.x { -1.0 } else { 1.0 };
            state.drag = Some(GizmoDrag { handle, start_pointer: pointer, start_pose: state.pose, offset, scale_direction, raw_rotate: state.pose.rotate });
            started = true;
        }
    }
    let drag = state.drag.clone()?;
    let still_down = ui.input(|input| input.pointer.primary_down());
    let modifiers = ui.input(|input| input.modifiers);
    if let Some(pointer) = pointer {
        let screen = pointer - drag.start_pointer;
        match drag.handle {
            Handle::Move => {
                // `Handle_MouseMove`: Shift halves the movement.
                let start_centre = Pos2::new(drag.start_pose.x, drag.start_pose.y);
                let mut target = (pointer - image.min) / image.width() - drag.offset;
                if modifiers.shift {
                    target = start_centre.to_vec2() + (target - start_centre.to_vec2()) * 0.5;
                }
                state.pose.x = target.x.clamp(0.0, 1.0);
                state.pose.y = target.y.clamp(0.0, 1.0);
            }
            Handle::Scale => {
                // Sideways, 0.01 a point (a quarter with Alt), whole
                // twentieths with Shift.
                let step = if modifiers.alt { SCALE_STEP * 0.25 } else { SCALE_STEP };
                let mut scale = (drag.start_pose.scale + screen.x * drag.scale_direction * step).max(0.01);
                if modifiers.shift {
                    scale = ((scale / 0.05).round() * 0.05).max(0.01);
                }
                state.pose.scale = scale;
            }
            Handle::Height => {
                // Up raises, 0.005 a point (a quarter with Shift), whole
                // twentieths with Alt.
                let step = if modifiers.shift { HEIGHT_STEP * 0.25 } else { HEIGHT_STEP };
                let mut z = drag.start_pose.z - screen.y * step;
                if modifiers.alt {
                    z = (z / 0.05).round() * 0.05;
                }
                state.pose.z = z.clamp(0.0, 2.0);
            }
            Handle::Rotate => {
                // Up turns, half a degree a point (a quarter with Shift),
                // in 15° steps with Ctrl.
                let step = if modifiers.shift { ROTATE_STEP * 0.25 } else { ROTATE_STEP };
                let raw = wrap_degrees(drag.start_pose.rotate - screen.y * step);
                let shown = if modifiers.ctrl || modifiers.command { (raw / ROTATE_SNAP).round() * ROTATE_SNAP } else { raw };
                state.pose.rotate = wrap_degrees(shown);
                if let Some(active) = state.drag.as_mut() {
                    active.raw_rotate = raw;
                }
            }
        }
    }
    let finished = !still_down;
    if finished {
        state.drag = None;
    }
    let values = hit_values(node, &state.pose, kind, snow_input);
    let edit = GizmoEdit { node: state.node, values, started, finished };
    Some(edit)
}

/// The 3D gizmos (`ToggleGizmo` with `GizmoType` 3D + 2D): `XY` and
/// `TS` gizmos get the handle that moves over the ground (`GizmoXZ`, set
/// on the terrain under the node's position), `XYZ` and `SRT` the one that
/// moves up too (`GizmoXYZ`, at the node's height); `Y1` gets none. A drag
/// sets the node's position (and height) as the 2D gizmo's would
/// (`GizmoHit`), its rotation and scale kept.
pub fn place_3d_handle(editor: &Editor, handle: &mut PositionHandle, preview: &TerrainPreview, commands: &mut Vec<Command>) {
    let node = editor.active_node().and_then(|node| editor.document.node(node));
    let kind = node.and_then(|node| GizmoKind::of(&node.type_name));
    let wanted = editor.gizmo_active && editor.user.options.gizmos_in_3d();
    let (Some(node), Some(kind)) = (node, kind.filter(|kind| *kind != GizmoKind::Y1 && wanted)) else {
        handle.shown = None;
        return;
    };
    let pose = pose_of(node);
    let height_scale = preview.height_scale().max(0.0001);
    let axes = if kind.is_srt() { HandleAxes::All } else { HandleAxes::Ground };
    let height = if kind.is_srt() {
        pose.z * height_scale
    } else {
        ground_height(preview, pose.x, pose.y) * height_scale
    };
    let position = bevy::math::Vec3::new(pose.x - 0.5, height, pose.y - 0.5);
    handle.shown = Some((position, axes));
    handle.minimum = bevy::math::Vec3::new(-0.5, 0.0, -0.5);
    handle.maximum = bevy::math::Vec3::new(0.5, 2.0 * height_scale, 0.5);
    let Some(moved) = handle.moved else {
        if handle.finished {
            let edit = GizmoEdit { node: node.id, values: Vec::new(), started: false, finished: true };
            commands.push(Command::GizmoEdit(edit));
        }
        return;
    };
    let mut moved_pose = pose;
    moved_pose.x = (moved.x + 0.5).clamp(0.0, 1.0);
    moved_pose.y = (moved.z + 0.5).clamp(0.0, 1.0);
    if kind.is_srt() {
        moved_pose.z = (moved.y / height_scale).clamp(0.0, 2.0);
    }
    let values = hit_values(node, &moved_pose, kind, None);
    let edit = GizmoEdit { node: node.id, values, started: handle.started, finished: handle.finished };
    commands.push(Command::GizmoEdit(edit));
}

/// The terrain's height (0–1) under a point of the map.
fn ground_height(preview: &TerrainPreview, x: f32, y: f32) -> f32 {
    let Some(heights) = preview.heights.as_ref() else {
        return 0.0;
    };
    let column = (x * (heights.width().max(1) - 1) as f32).round() as usize;
    let row = (y * (heights.height().max(1) - 1) as f32).round() as usize;
    heights.get(column.min(heights.width() - 1), row.min(heights.height() - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node_of(type_name: &str) -> NodeData {
        let spec = Catalogue::get().node(type_name).expect("type exists");
        NodeData::new(spec, 1, 7)
    }

    #[test]
    fn poses_and_hits_round_trip() {
        for type_name in ["Cone", "Crater", "Mountain", "Perlin", "Swirl", "Transform", "Voronoi", "Lake", "Pond", "Transform3D"] {
            let node = node_of(type_name);
            let kind = GizmoKind::of(type_name).expect("has a gizmo");
            let pose = pose_of(&node);
            let values = hit_values(&node, &pose, kind, None);
            for (name, value) in values {
                let before = node.values.get(&name).and_then(Value::as_f64).unwrap_or(0.0);
                let after = value.as_f64().unwrap_or(0.0);
                assert!((before - after).abs() < 1e-6, "{type_name}.{name}: {before} → {after}");
            }
        }
    }

    #[test]
    fn perlin_flips_y_and_lake_spans_minus_one_to_one() {
        let perlin = node_of("Perlin");
        let pose = GizmoPose { x: 0.25, y: 0.1, z: 0.0, rotate: 0.0, scale: 1.0 };
        let values = hit_values(&perlin, &pose, GizmoKind::Xy, None);
        assert_eq!(values[1].1.as_f64(), Some(0.9));
        let lake = node_of("Lake");
        let values = hit_values(&lake, &pose, GizmoKind::Ts, None);
        assert_eq!(values[0].1.as_f64(), Some(-0.5));
    }
}
