//! Terrain rendering for the Nerthus viewport: the previewed heightfield
//! as a mesh draped with the colours the viewport renderer baked (shown
//! unlit), the ground grid and the terrain's bounds, seen through a
//! strategy camera.

mod camera;
mod gizmo3d;
mod helpers;
mod screenshot;
mod sky;
mod terrain;
mod turntable;

use bevy::camera::{CameraUpdateSystems, RenderTarget};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;

pub use camera::{CameraRequest, CameraValues, NavigationSettings, StrategyCamera, ViewportInput};
pub use gizmo3d::{HandleAxes, PositionHandle};
pub use screenshot::{CapturedPixels, ScreenshotRequest, Screenshots};
pub use terrain::{TerrainColours, TerrainPreview, TerrainSurface, ViewportSettings};
pub use turntable::{Turntable, TurntableOutcome, TurntableRequest};

use terrain::{rebuild_terrain, spawn_terrain, BuiltTerrain, TerrainMaterial};

/// `Options.ViewportBackground`'s default (5/255 grey).
const NO_SKY: Color = Color::srgb(0.01960784, 0.01960784, 0.01960784);
/// The Unity camera's vertical field of view.
const FIELD_OF_VIEW_DEGREES: f32 = 30.0;
/// Close enough to stay out of the way when zoomed right in (the terrain
/// spans one unit).
const NEAR_PLANE: f32 = 0.001;

pub struct TerrainRenderPlugin;

impl Plugin for TerrainRenderPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "terrain.wesl");
        app.add_plugins(MaterialPlugin::<TerrainMaterial>::default());
        app.insert_resource(ClearColor(NO_SKY));
        app.init_resource::<TerrainPreview>();
        app.init_resource::<ViewportSettings>();
        app.init_resource::<ViewportInput>();
        app.init_resource::<NavigationSettings>();
        app.init_resource::<CameraRequest>();
        app.init_resource::<Screenshots>();
        app.init_resource::<BuiltTerrain>();
        app.init_resource::<sky::SkyWorker>();
        app.init_resource::<sky::SkyState>();
        app.init_resource::<Turntable>();
        app.init_resource::<PositionHandle>();
        app.add_systems(Startup, setup_scene);
        app.add_systems(Update, (rebuild_terrain, helpers::update_helpers, update_background, camera::strategy_camera));
        app.add_systems(Update, sky::update_sky.after(camera::strategy_camera));
        app.add_systems(Update, gizmo3d::update_handle.before(camera::strategy_camera));
        app.add_systems(Update, turntable::record_turntable.after(sky::update_sky));
        app.add_systems(PostUpdate, (screenshot::finish_captures, screenshot::take_screenshots).chain());
        app.add_systems(PostUpdate, refresh_moved_cameras.before(CameraUpdateSystems));
    }
}

/// The marker for the 3D viewport camera.
#[derive(Component)]
pub struct ViewportCamera;

fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut terrain_materials: ResMut<Assets<TerrainMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let strategy = StrategyCamera::default();
    let transform = strategy.transform();
    let perspective = PerspectiveProjection {
        fov: FIELD_OF_VIEW_DEGREES.to_radians(),
        near: NEAR_PLANE,
        ..default()
    };
    // The colours are final: no tone mapping or dithering on top.
    let camera = (ViewportCamera, Camera3d::default(), Projection::Perspective(perspective), Tonemapping::None, DebandDither::Disabled, strategy, transform);
    commands.spawn(camera);
    spawn_terrain(&mut commands, &mut meshes, &mut terrain_materials, &mut images);
    helpers::spawn_helpers(&mut commands, &mut meshes, &mut materials, &mut images);
    sky::spawn_sky(&mut commands, &mut meshes, &mut materials);
    gizmo3d::spawn_handle(&mut commands, &mut meshes, &mut materials);
}

/// Bevy works a camera's window out again (its size, which sizes the
/// textures the window's cameras share) when the camera's projection or
/// viewport changes, but not when the camera moves to another window, as
/// the 3D view's do when the floating viewport's window opens and closes.
/// Marking the projection changed has it do so the frame the camera moves.
fn refresh_moved_cameras(mut cameras: Query<&mut Projection, Changed<RenderTarget>>) {
    for mut projection in &mut cameras {
        projection.set_changed();
    }
}

/// Behind the terrain: the viewport background (`SetSkyColor`), which the
/// sky's dome covers while Show Skybox is on.
fn update_background(settings: Res<ViewportSettings>, mut clear: ResMut<ClearColor>) {
    if !settings.is_changed() {
        return;
    }
    let [red, green, blue] = settings.background;
    clear.0 = Color::srgb(red, green, blue);
}
