//! Gizmos in the 3D view (Options › Gizmos › 3D + 2D Viewport): a position
//! handle (`RuntimeTransformHandle`) on the node's gizmo, with arrows
//! along X and Z and the square between them for
//! gizmos that move over the ground (`GizmoXZ`), and an arrow up and the
//! other two squares as well for those with a height (`GizmoXYZ`).
//! Dragging an arrow moves along it, a square across its plane, and the
//! camera keeps still meanwhile (`StartGizmoMovements`). The handle is
//! drawn over the terrain and keeps its size on screen.

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::camera::ViewportInput;
use crate::ViewportCamera;

/// The layer the handle is drawn on, by its own camera over the view.
const HANDLE_LAYER: usize = 1;
/// The handle's arrows against the camera's distance, and the parts'
/// proportions (of an arrow's length).
const SIZE_PER_DISTANCE: f32 = 0.12;
const SHAFT_RADIUS: f32 = 0.018;
const HEAD_RADIUS: f32 = 0.06;
const HEAD_LENGTH: f32 = 0.22;
const SQUARE_NEAR: f32 = 0.25;
const SQUARE_FAR: f32 = 0.5;
/// How near a ray must pass an arrow to take it.
const PICK_RADIUS: f32 = 0.07;
const X_COLOUR: Color = Color::srgb(0.86, 0.22, 0.2);
const Y_COLOUR: Color = Color::srgb(0.38, 0.78, 0.22);
const Z_COLOUR: Color = Color::srgb(0.22, 0.45, 0.9);
const HOT_COLOUR: Color = Color::srgb(1.0, 0.86, 0.0);

/// Which way the handle moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandleAxes {
    /// Over the ground: X, Z and their square.
    Ground,
    /// Up too.
    All,
}

/// One of the handle's parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Component)]
pub(crate) enum HandlePart {
    X,
    Y,
    Z,
    /// The squares, by the axis they face.
    PlaneY,
    PlaneZ,
    PlaneX,
}

impl HandlePart {
    const ALL: [HandlePart; 6] = [HandlePart::X, HandlePart::Y, HandlePart::Z, HandlePart::PlaneY, HandlePart::PlaneZ, HandlePart::PlaneX];

    /// North (the map's top) is −Z here, Unity's +Z.
    fn axis(self) -> Vec3 {
        match self {
            HandlePart::X | HandlePart::PlaneX => Vec3::X,
            HandlePart::Y | HandlePart::PlaneY => Vec3::Y,
            HandlePart::Z | HandlePart::PlaneZ => Vec3::NEG_Z,
        }
    }

    fn is_plane(self) -> bool {
        matches!(self, HandlePart::PlaneX | HandlePart::PlaneY | HandlePart::PlaneZ)
    }

    fn shown_for(self, axes: HandleAxes) -> bool {
        axes == HandleAxes::All || matches!(self, HandlePart::X | HandlePart::Z | HandlePart::PlaneY)
    }

    fn colour(self) -> Color {
        match self {
            HandlePart::X | HandlePart::PlaneX => X_COLOUR,
            HandlePart::Y | HandlePart::PlaneY => Y_COLOUR,
            HandlePart::Z | HandlePart::PlaneZ => Z_COLOUR,
        }
    }

    /// A square's two edges' directions.
    fn plane_axes(self) -> (Vec3, Vec3) {
        match self {
            HandlePart::PlaneY => (Vec3::X, Vec3::NEG_Z),
            HandlePart::PlaneZ => (Vec3::X, Vec3::Y),
            _ => (Vec3::Y, Vec3::NEG_Z),
        }
    }
}

/// A drag under way.
#[derive(Clone, Copy, Debug)]
struct HandleDrag {
    part: HandlePart,
    start_position: Vec3,
    /// Where the pointer first met the arrow's line or the square's plane.
    start_point: Vec3,
}

/// The handle, as the editor wants it and as it was dragged.
#[derive(Resource, Default)]
pub struct PositionHandle {
    /// Where it stands and how it moves; none hides it.
    pub shown: Option<(Vec3, HandleAxes)>,
    /// The box it keeps within.
    pub minimum: Vec3,
    pub maximum: Vec3,
    /// While dragged, where it has been moved to; and whether a drag
    /// started or ended this frame.
    pub moved: Option<Vec3>,
    pub started: bool,
    pub finished: bool,
    drag: Option<HandleDrag>,
    hovered: Option<HandlePart>,
}

impl PositionHandle {
    /// Whether the handle holds the pointer (the camera leaves it be).
    pub fn holds_pointer(&self) -> bool {
        self.drag.is_some() || self.hovered.is_some()
    }
}

/// The handle's root and its camera.
#[derive(Component)]
pub(crate) struct HandleRoot;

#[derive(Component)]
pub(crate) struct HandleCamera;

pub(crate) fn spawn_handle(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) {
    let layer = RenderLayers::layer(HANDLE_LAYER);
    let camera = Camera { order: 1, clear_color: ClearColorConfig::None, ..default() };
    commands.spawn((HandleCamera, Camera3d::default(), camera, Tonemapping::None, DebandDither::Disabled, layer.clone()));

    let root = commands.spawn((HandleRoot, Transform::default(), Visibility::Hidden, layer.clone())).id();
    let shaft = meshes.add(Cylinder::new(SHAFT_RADIUS, 1.0 - HEAD_LENGTH));
    let head = meshes.add(Cone { radius: HEAD_RADIUS, height: HEAD_LENGTH });
    let square = meshes.add(Rectangle::new(SQUARE_FAR - SQUARE_NEAR, SQUARE_FAR - SQUARE_NEAR));
    for part in HandlePart::ALL {
        let base_colour = if part.is_plane() { part.colour().with_alpha(0.45) } else { part.colour() };
        let material = StandardMaterial {
            base_color: base_colour,
            unlit: true,
            fog_enabled: false,
            cull_mode: None,
            alpha_mode: if part.is_plane() { AlphaMode::Blend } else { AlphaMode::Opaque },
            ..default()
        };
        let material = materials.add(material);
        let part_entity = commands.spawn((part, Transform::default(), Visibility::Inherited, layer.clone())).id();
        commands.entity(root).add_child(part_entity);
        if part.is_plane() {
            let (first, second) = part.plane_axes();
            let middle = (SQUARE_NEAR + SQUARE_FAR) / 2.0;
            let rotation = Quat::from_mat3(&Mat3::from_cols(first, second, first.cross(second)));
            let transform = Transform::from_translation((first + second) * middle).with_rotation(rotation);
            let piece = commands.spawn((Mesh3d(square.clone()), MeshMaterial3d(material), transform, NotShadowCaster, layer.clone())).id();
            commands.entity(part_entity).add_child(piece);
            continue;
        }
        // Cylinders and cones stand along Y; each arrow is turned onto its
        // axis.
        let rotation = Quat::from_rotation_arc(Vec3::Y, part.axis());
        let shaft_length = 1.0 - HEAD_LENGTH;
        let shaft_transform = Transform::from_translation(part.axis() * shaft_length / 2.0).with_rotation(rotation);
        let head_transform = Transform::from_translation(part.axis() * (shaft_length + HEAD_LENGTH / 2.0)).with_rotation(rotation);
        let shaft_piece = commands.spawn((Mesh3d(shaft.clone()), MeshMaterial3d(material.clone()), shaft_transform, NotShadowCaster, layer.clone())).id();
        let head_piece = commands.spawn((Mesh3d(head.clone()), MeshMaterial3d(material), head_transform, NotShadowCaster, layer.clone())).id();
        commands.entity(part_entity).add_child(shaft_piece);
        commands.entity(part_entity).add_child(head_piece);
    }
}

/// Where along `axis` (through `origin`) the ray passes nearest, and how
/// near; none when they run side by side.
fn nearest_on_line(ray: Ray3d, origin: Vec3, axis: Vec3) -> Option<(f32, f32, f32)> {
    let direction = *ray.direction;
    let between = ray.origin - origin;
    let along = axis.dot(direction);
    let denominator = 1.0 - along * along;
    if denominator.abs() < 1e-6 {
        return None;
    }
    let axis_part = axis.dot(between);
    let ray_part = direction.dot(between);
    // Parameters on the axis (s) and the ray (t) of the nearest points.
    let s = (axis_part - along * ray_part) / denominator;
    let t = (along * axis_part - ray_part) / denominator;
    let distance = ((origin + axis * s) - (ray.origin + direction * t)).length();
    Some((s, t, distance))
}

/// Where the ray meets the plane through `origin` facing `normal`.
fn plane_hit(ray: Ray3d, origin: Vec3, normal: Vec3) -> Option<(Vec3, f32)> {
    let facing = normal.dot(*ray.direction);
    if facing.abs() < 1e-6 {
        return None;
    }
    let t = normal.dot(origin - ray.origin) / facing;
    (t > 0.0).then(|| (ray.origin + *ray.direction * t, t))
}

/// The part of the handle (at `position`, arrows `size` long) the ray
/// meets first.
fn pick(ray: Ray3d, position: Vec3, size: f32, axes: HandleAxes) -> Option<HandlePart> {
    let mut best: Option<(f32, HandlePart)> = None;
    for part in HandlePart::ALL.into_iter().filter(|part| part.shown_for(axes)) {
        let hit = if part.is_plane() {
            let (first, second) = part.plane_axes();
            plane_hit(ray, position, part.axis()).and_then(|(point, t)| {
                let offset = (point - position) / size;
                let a = offset.dot(first);
                let b = offset.dot(second);
                let inside = (SQUARE_NEAR..=SQUARE_FAR).contains(&a) && (SQUARE_NEAR..=SQUARE_FAR).contains(&b);
                inside.then_some(t)
            })
        } else {
            nearest_on_line(ray, position, part.axis()).and_then(|(s, t, distance)| {
                let on_arrow = (0.0..=size).contains(&s) && distance <= PICK_RADIUS * size && t > 0.0;
                on_arrow.then_some(t)
            })
        };
        if let Some(t) = hit
            && best.is_none_or(|(nearest, _)| t < nearest)
        {
            best = Some((t, part));
        }
    }
    best.map(|(_, part)| part)
}

/// Where the pointer meets the dragged part's line or plane.
fn drag_point(ray: Ray3d, drag: &HandleDrag) -> Option<Vec3> {
    if drag.part.is_plane() {
        return plane_hit(ray, drag.start_position, drag.part.axis()).map(|(point, _)| point);
    }
    nearest_on_line(ray, drag.start_position, drag.part.axis()).map(|(s, _, _)| drag.start_position + drag.part.axis() * s)
}

/// The view's camera as the handle reads it.
type ViewQuery<'a> = (&'a Camera, &'a GlobalTransform, Ref<'a, Projection>, &'a RenderTarget);
type ViewFilter = (With<ViewportCamera>, Without<HandleCamera>);
/// The handle's own camera, made to match the view's.
type HandleCameraQuery<'a> = (&'a mut Camera, &'a mut Transform, &'a mut Projection, &'a mut RenderTarget);
type HandleCameraFilter = (With<HandleCamera>, Without<ViewportCamera>, Without<HandleRoot>);
type RootFilter = (With<HandleRoot>, Without<HandleCamera>);

/// Places the handle and its camera, picks and drags its parts.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_handle(
    mut handle: ResMut<PositionHandle>,
    input: Res<ViewportInput>,
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<(Entity, &Window, Has<PrimaryWindow>)>,
    viewport: Single<ViewQuery, ViewFilter>,
    handle_camera: Single<HandleCameraQuery, HandleCameraFilter>,
    mut root: Single<(&mut Transform, &mut Visibility), RootFilter>,
    mut parts: Query<(&HandlePart, &mut Visibility, &Children), Without<HandleRoot>>,
    pieces: Query<&MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    handle.started = false;
    handle.finished = false;
    handle.moved = None;
    let (camera, camera_transform, projection, target) = viewport.into_inner();
    let (mut handle_view, mut handle_transform, mut handle_projection, mut handle_target) = handle_camera.into_inner();
    // The handle's camera sees what the view's camera sees, in the same
    // window.
    handle_view.viewport = camera.viewport.clone();
    handle_view.is_active = handle.shown.is_some();
    *handle_transform = camera_transform.compute_transform();
    // `RenderTarget` cannot be compared; its description can.
    if format!("{:?}", *handle_target) != format!("{target:?}") {
        *handle_target = target.clone();
    }
    // Only when it changes: a changed projection has Bevy work the camera's
    // window out again.
    if projection.is_changed() {
        *handle_projection = Projection::clone(&projection);
    }

    let Some((position, axes)) = handle.shown else {
        *root.1 = Visibility::Hidden;
        handle.drag = None;
        handle.hovered = None;
        return;
    };
    let shown_position = handle.drag.map_or(position, |drag| drag.start_position);
    let distance = camera_transform.translation().distance(shown_position).max(0.001);
    let size = distance * SIZE_PER_DISTANCE;

    // The pointer, in the window the view is in.
    let window = match target {
        RenderTarget::Window(bevy::window::WindowRef::Entity(entity)) => windows.iter().find(|(candidate, ..)| candidate == entity),
        _ => windows.iter().find(|(.., primary)| *primary),
    };
    let cursor = window.and_then(|(_, window, _)| window.cursor_position());
    let ray = cursor.and_then(|cursor| camera.viewport_to_world(camera_transform, cursor).ok());

    if let Some(drag) = handle.drag {
        let pressed = buttons.pressed(MouseButton::Left);
        if let Some(point) = ray.and_then(|ray| drag_point(ray, &drag)) {
            let moved = (drag.start_position + (point - drag.start_point)).clamp(handle.minimum, handle.maximum);
            handle.moved = Some(moved);
        }
        if !pressed {
            handle.drag = None;
            handle.finished = true;
        }
    } else {
        let over = if input.enabled { ray.and_then(|ray| pick(ray, position, size, axes)) } else { None };
        handle.hovered = over;
        if let (Some(part), Some(ray)) = (over, ray)
            && buttons.just_pressed(MouseButton::Left)
        {
            let start = HandleDrag { part, start_position: position, start_point: position };
            if let Some(point) = drag_point(ray, &start) {
                handle.drag = Some(HandleDrag { start_point: point, ..start });
                handle.started = true;
            }
        }
    }

    let placed = handle.moved.unwrap_or(position);
    *root.0 = Transform::from_translation(placed).with_scale(Vec3::splat(size));
    *root.1 = Visibility::Visible;
    let hot = handle.drag.map(|drag| drag.part).or(handle.hovered);
    for (part, mut visibility, children) in &mut parts {
        *visibility = if part.shown_for(axes) { Visibility::Inherited } else { Visibility::Hidden };
        let colour = if hot == Some(*part) { HOT_COLOUR } else { part.colour() };
        let colour = if part.is_plane() { colour.with_alpha(0.45) } else { colour };
        for child in children.iter() {
            if let Ok(material) = pieces.get(child)
                && let Some(mut material) = materials.get_mut(&material.0)
                && material.base_color != colour
            {
                material.base_color = colour;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ray_down(x: f32, z: f32) -> Ray3d {
        Ray3d { origin: Vec3::new(x, 5.0, z), direction: Dir3::NEG_Y }
    }

    #[test]
    fn rays_take_the_arrow_or_square_they_meet() {
        let position = Vec3::ZERO;
        // Straight down onto the X arrow's middle.
        assert_eq!(pick(ray_down(0.5, 0.0), position, 1.0, HandleAxes::Ground), Some(HandlePart::X));
        // North is −Z.
        assert_eq!(pick(ray_down(0.0, -0.5), position, 1.0, HandleAxes::Ground), Some(HandlePart::Z));
        // The ground square sits between them.
        assert_eq!(pick(ray_down(0.4, -0.4), position, 1.0, HandleAxes::Ground), Some(HandlePart::PlaneY));
        assert_eq!(pick(ray_down(-0.5, 0.5), position, 1.0, HandleAxes::Ground), None);
    }

    #[test]
    fn arrows_drag_along_their_line() {
        let drag = HandleDrag { part: HandlePart::X, start_position: Vec3::ZERO, start_point: Vec3::new(0.5, 0.0, 0.0) };
        let point = drag_point(ray_down(0.8, 0.3), &drag).unwrap();
        assert!((point - Vec3::new(0.8, 0.0, 0.0)).length() < 1e-5);
    }
}
