//! Preview › Record Turntable Animation (`Comms.StartRecording`): the
//! terrain turned a further 360/frames degrees for each frame (round the
//! vertical, as Unity turns `SURFACE`), each frame rendered at the size
//! asked for (the viewport window takes that size) and saved as
//! `<base>_0000.png`, `<base>_0001.png` and on. The grid and the bounds
//! can be left out (Hide Grid and Compass).

use std::path::PathBuf;

use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::pbr::DistanceFog;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};

use crate::screenshot::save_png;
use crate::terrain::{TerrainBackface, TerrainSurface};
use crate::ViewportCamera;

/// Frames a frame's camera draws before its picture is taken.
const CAPTURE_FRAMES: u32 = 3;

/// An animation to record.
#[derive(Clone, Debug, PartialEq)]
pub struct TurntableRequest {
    /// The frames' path without `_0000.png`.
    pub base: PathBuf,
    pub frames: u32,
    pub size: UVec2,
    pub hide_helpers: bool,
}

/// How a recording ended.
#[derive(Clone, Debug, PartialEq)]
pub enum TurntableOutcome {
    /// So many frames saved.
    Finished(u32),
    Failed(String),
}

/// The recording asked for, the one running and how the last one ended.
#[derive(Resource, Default)]
pub struct Turntable {
    pub request: Option<TurntableRequest>,
    pub outcome: Option<TurntableOutcome>,
    recording: Option<Recording>,
}

struct Recording {
    request: TurntableRequest,
    /// The frame being rendered, and whether its camera is out.
    frame: u32,
    rendering: bool,
    /// The frame just saved, or why it was not.
    saved: Option<Result<u32, String>>,
}

impl Turntable {
    pub fn recording(&self) -> bool {
        self.recording.is_some()
    }

    /// Whether the grid and bounds are left out of the pictures now.
    pub fn hides_helpers(&self) -> bool {
        self.recording.as_ref().is_some_and(|recording| recording.request.hide_helpers)
    }

    /// Frames saved so far, and how many there will be.
    pub fn progress(&self) -> Option<(u32, u32)> {
        self.recording.as_ref().map(|recording| (recording.frame, recording.request.frames))
    }
}

/// The camera one frame is rendered with.
#[derive(Component)]
pub(crate) struct TurntableCamera {
    target: Handle<Image>,
    frame: u32,
    drawn: u32,
}

/// The terrain's surface and underside, which turn.
type SurfaceFilter = (Or<(With<TerrainSurface>, With<TerrainBackface>)>, Without<ViewportCamera>);

/// Runs the recording: turns the terrain and puts out a camera for each
/// frame, takes its picture once it has drawn, and puts the terrain back
/// at the end.
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_turntable(
    mut commands: Commands,
    mut turntable: ResMut<Turntable>,
    mut images: ResMut<Assets<Image>>,
    viewport: Single<(&Projection, &Transform, Option<&DistanceFog>), With<ViewportCamera>>,
    mut surfaces: Query<&mut Transform, SurfaceFilter>,
    mut cameras: Query<(Entity, &mut TurntableCamera)>,
) {
    if turntable.recording.is_none()
        && let Some(request) = turntable.request.take()
    {
        turntable.outcome = None;
        turntable.recording = Some(Recording { request, frame: 0, rendering: false, saved: None });
    }
    let Some(recording) = turntable.recording.as_mut() else {
        return;
    };

    match recording.saved.take() {
        Some(Ok(_)) => {
            recording.frame += 1;
            recording.rendering = false;
        }
        Some(Err(problem)) => {
            for mut transform in &mut surfaces {
                transform.rotation = Quat::IDENTITY;
            }
            turntable.recording = None;
            turntable.outcome = Some(TurntableOutcome::Failed(problem));
            return;
        }
        None => {}
    }
    if recording.frame >= recording.request.frames {
        for mut transform in &mut surfaces {
            transform.rotation = Quat::IDENTITY;
        }
        let frames = recording.request.frames;
        turntable.recording = None;
        turntable.outcome = Some(TurntableOutcome::Finished(frames));
        return;
    }

    if !recording.rendering {
        // `SURFACE.transform.eulerAngles = (0, frame * 360 / frames, 0)`:
        // clockwise seen from above.
        let degrees = recording.frame as f32 * 360.0 / recording.request.frames as f32;
        let rotation = Quat::from_rotation_y(-degrees.to_radians());
        for mut transform in &mut surfaces {
            transform.rotation = rotation;
        }
        let size = recording.request.size;
        let target = images.add(Image::new_target_texture(size.x.max(1), size.y.max(1), TextureFormat::Rgba8UnormSrgb, None));
        let (projection, transform, fog) = *viewport;
        let camera = TurntableCamera { target: target.clone(), frame: recording.frame, drawn: 0 };
        let mut spawned = commands.spawn((camera, Camera3d::default(), RenderTarget::Image(target.into()), projection.clone(), Tonemapping::None, DebandDither::Disabled, *transform));
        if let Some(fog) = fog {
            spawned.insert(fog.clone());
        }
        recording.rendering = true;
    }

    let base = recording.request.base.clone();
    for (entity, mut camera) in &mut cameras {
        camera.drawn += 1;
        if camera.drawn != CAPTURE_FRAMES {
            continue;
        }
        let frame = camera.frame;
        let path = PathBuf::from(format!("{}_{frame:04}.png", base.display()));
        let mut screenshot = commands.spawn(Screenshot::image(camera.target.clone()));
        screenshot.observe(move |captured: On<ScreenshotCaptured>, mut turntable: ResMut<Turntable>, mut commands: Commands| {
            let saved = save_png(&captured.image, &path, None);
            if let Some(recording) = turntable.recording.as_mut() {
                recording.saved = Some(saved.map(|()| frame));
            }
            commands.entity(entity).despawn();
        });
    }
}
