//! Screenshots. The 3D view's (`ScreenshotTool.SaveScreenshot`) renders
//! the viewport camera's view again into an image `supersample` times the
//! view's size, without what the interface draws over it. The graph's
//! (`ScreenshotService.TakeGraphScreenshot`) is its part of the window.
//! Both are saved as PNG. The 3D view and the whole window can also be
//! captured into memory, for whoever asked to look at them.

use std::path::{Path, PathBuf};

use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::image::Image;
use bevy::math::URect;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};

use crate::ViewportCamera;

/// A screenshot to take.
#[derive(Clone, Debug, PartialEq)]
pub enum ScreenshotRequest {
    /// The 3D view at `supersample` times its size.
    Viewport { path: PathBuf, supersample: u32 },
    /// Part of the window, in physical pixels.
    WindowRegion { path: PathBuf, region: URect },
    /// The 3D view at its size, kept in memory under `id`.
    ViewportPixels { id: u64 },
    /// The whole window, kept in memory under `id`.
    WindowPixels { id: u64 },
}

/// A capture kept in memory: RGBA bytes, row after row from the top.
#[derive(Clone, Debug, PartialEq)]
pub struct CapturedPixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Screenshots asked for, and how each went (the file saved, or why not;
/// the pixels, by the id they were asked for under).
#[derive(Resource, Default)]
pub struct Screenshots {
    pub requests: Vec<ScreenshotRequest>,
    pub finished: Vec<Result<PathBuf, String>>,
    pub captured: Vec<(u64, Result<CapturedPixels, String>)>,
}

/// Where a capture goes.
#[derive(Clone, Debug)]
enum Destination {
    File(PathBuf),
    Memory(u64),
}

impl Screenshots {
    /// A capture's image saved or kept, as its destination says.
    fn deliver(&mut self, destination: &Destination, image: &Image, region: Option<URect>) {
        match destination {
            Destination::File(path) => {
                let saved = save_png(image, path, region);
                self.finished.push(saved.map(|()| path.clone()));
            }
            Destination::Memory(id) => {
                let pixels = pixels_of(image);
                self.captured.push((*id, pixels));
            }
        }
    }

    /// Why a capture could not be taken.
    fn fail(&mut self, destination: &Destination, problem: &str) {
        match destination {
            Destination::File(_) => self.finished.push(Err(problem.to_string())),
            Destination::Memory(id) => self.captured.push((*id, Err(problem.to_string()))),
        }
    }
}

/// The camera a viewport screenshot renders with, until it is taken: the
/// image it renders into, where it goes, and the frames it has had (a new
/// camera and image take a frame or two to start drawing).
#[derive(Component)]
pub(crate) struct CaptureCamera {
    target: Handle<Image>,
    destination: Destination,
    frames: u32,
}

/// Frames a capture camera draws before its picture is taken.
const CAPTURE_FRAMES: u32 = 3;

/// Starts the screenshots asked for.
pub(crate) fn take_screenshots(
    mut commands: Commands,
    mut screenshots: ResMut<Screenshots>,
    mut images: ResMut<Assets<Image>>,
    viewport: Single<(&Camera, &Projection, &Transform), With<ViewportCamera>>,
) {
    if screenshots.requests.is_empty() {
        return;
    }
    let (camera, projection, transform) = *viewport;
    let view_size = camera.viewport.as_ref().map(|viewport| viewport.physical_size);
    for request in std::mem::take(&mut screenshots.requests) {
        let (destination, supersample) = match request {
            ScreenshotRequest::Viewport { path, supersample } => (Destination::File(path), supersample),
            ScreenshotRequest::ViewportPixels { id } => (Destination::Memory(id), 1),
            ScreenshotRequest::WindowRegion { path, region } => {
                let destination = Destination::File(path);
                capture_window(&mut commands, destination, Some(region));
                continue;
            }
            ScreenshotRequest::WindowPixels { id } => {
                capture_window(&mut commands, Destination::Memory(id), None);
                continue;
            }
        };
        let Some(size) = view_size else {
            screenshots.fail(&destination, "the 3D view is not showing");
            continue;
        };
        let scaled = size * supersample.max(1);
        let target = Image::new_target_texture(scaled.x, scaled.y, TextureFormat::Rgba8UnormSrgb, None);
        let target = images.add(target);
        let capture = CaptureCamera {
            target: target.clone(),
            destination,
            frames: 0,
        };
        let capture_camera = (
            capture,
            Camera3d::default(),
            RenderTarget::Image(target.clone().into()),
            projection.clone(),
            Tonemapping::None,
            DebandDither::Disabled,
            *transform,
        );
        commands.spawn(capture_camera);
    }
}

/// The window as drawn, cut to `region` when given.
fn capture_window(commands: &mut Commands, destination: Destination, region: Option<URect>) {
    let mut screenshot = commands.spawn(Screenshot::primary_window());
    screenshot.observe(move |captured: On<ScreenshotCaptured>, mut screenshots: ResMut<Screenshots>| {
        screenshots.deliver(&destination, &captured.image, region);
    });
}

/// Takes each capture camera's picture once it has drawn a few frames,
/// then lets the camera go.
pub(crate) fn finish_captures(mut commands: Commands, mut cameras: Query<(Entity, &mut CaptureCamera)>) {
    for (entity, mut capture) in &mut cameras {
        capture.frames += 1;
        if capture.frames != CAPTURE_FRAMES {
            continue;
        }
        let destination = capture.destination.clone();
        let mut screenshot = commands.spawn(Screenshot::image(capture.target.clone()));
        screenshot.observe(move |captured: On<ScreenshotCaptured>, mut screenshots: ResMut<Screenshots>, mut commands: Commands| {
            screenshots.deliver(&destination, &captured.image, None);
            commands.entity(entity).despawn();
        });
    }
}

/// A captured image's pixels as RGBA bytes.
fn pixels_of(image: &Image) -> Result<CapturedPixels, String> {
    let picture = image.clone().try_into_dynamic().map_err(|error| error.to_string())?;
    let rgba = picture.to_rgba8();
    let pixels = CapturedPixels {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    };
    Ok(pixels)
}

/// Writes a captured image as PNG, cut to `region` when given.
pub(crate) fn save_png(image: &Image, path: &Path, region: Option<URect>) -> Result<(), String> {
    let picture = image.clone().try_into_dynamic().map_err(|error| error.to_string())?;
    let picture = match region {
        Some(region) => {
            let right = region.max.x.min(picture.width());
            let bottom = region.max.y.min(picture.height());
            let width = right.saturating_sub(region.min.x);
            let height = bottom.saturating_sub(region.min.y);
            picture.crop_imm(region.min.x, region.min.y, width, height)
        }
        None => picture,
    };
    let pixels = picture.to_rgba8();
    // The path ends in .png, which picks the format.
    pixels.save(path).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension};

    use super::*;

    /// The width and height a PNG's header gives.
    fn png_size(bytes: &[u8]) -> (u32, u32) {
        let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        (width, height)
    }

    #[test]
    fn regions_are_cut_from_the_window() {
        let extent = Extent3d {
            width: 8,
            height: 6,
            depth_or_array_layers: 1,
        };
        let pixels = vec![200; 8 * 6 * 4];
        let window = Image::new(extent, TextureDimension::D2, pixels, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::default());
        let folder = std::env::temp_dir().join(format!("nerthus-screenshot-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("graph.png");

        save_png(&window, &path, Some(URect::new(2, 1, 7, 4))).unwrap();
        let written = std::fs::read(&path).unwrap();
        assert_eq!(&written[1..4], b"PNG");
        assert_eq!(png_size(&written), (5, 3));

        // A region running off the window is cut at its edge.
        save_png(&window, &path, Some(URect::new(6, 4, 20, 20))).unwrap();
        let written = std::fs::read(&path).unwrap();
        assert_eq!(png_size(&written), (2, 2));
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
