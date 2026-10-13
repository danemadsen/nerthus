//! Project thumbnails (`Shell.SaveThumbnail`, the viewport's
//! `ScreenshotTool.SaveThumbnail`): on each save, the 3D view rendered
//! again, its middle cut to 16:9, scaled to at most 720 wide and saved as a
//! JPEG (quality 65) where Options › Paths › File Thumbnails says: the data
//! folder's `Thumbnails` under the MD5 of the project's path (Central, the
//! default), or beside the project (Next to File). The Start dialog shows
//! them beside its recent files.

use std::path::{Path, PathBuf};

use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use nerthus_render::ViewportCamera;

use crate::editor::Editor;

/// `Shell.GetScreenshotFilename`: `Thumbnails/<md5 of the path>.jpg` for
/// Central storage, else `<project>.jpg` beside the project.
pub fn thumbnail_path(project: &Path, storage: &str) -> Option<PathBuf> {
    if storage != "Central" {
        return Some(project.with_extension("jpg"));
    }
    let folder = super::data_subfolder("Thumbnails")?;
    let text = project.to_string_lossy();
    Some(folder.join(format!("{}.jpg", md5_hex(text.as_bytes()))))
}

/// A thumbnail being rendered.
#[derive(Component)]
pub(super) struct ThumbnailCamera {
    target: Handle<Image>,
    path: PathBuf,
    frames: u32,
}

/// Frames the camera draws before its picture is taken (a new camera and
/// image take a frame or two to start drawing).
const CAPTURE_FRAMES: u32 = 3;
/// `SaveThumbnail`: at most 720 wide, 16:9, JPEG quality 65.
const MAX_WIDTH: u32 = 720;
const ASPECT: f32 = 0.5625;
const QUALITY: u8 = 65;

/// A save asked for a thumbnail: unless thumbnails are off, the old one
/// beside the project goes (for both kinds of storage) and a
/// camera like the viewport's renders the new one.
pub(super) fn start_capture(mut commands: Commands, mut editor: ResMut<Editor>, mut images: ResMut<Assets<Image>>, viewport: Single<(&Camera, &Projection, &Transform), With<ViewportCamera>>) {
    let Some(project) = editor.files.thumbnail_request.take() else {
        return;
    };
    let storage = editor.user.options.thumbnail_storage().to_string();
    // None writes nothing.
    if storage == "None" {
        return;
    }
    let _ = std::fs::remove_file(project.with_extension("jpg"));
    if !editor.rendered() {
        return;
    }
    let Some(path) = thumbnail_path(&project, &storage) else {
        return;
    };
    let (camera, projection, transform) = *viewport;
    let Some(size) = camera.viewport.as_ref().map(|viewport| viewport.physical_size) else {
        return;
    };
    let target = Image::new_target_texture(size.x.max(1), size.y.max(1), TextureFormat::Rgba8UnormSrgb, None);
    let target = images.add(target);
    let capture = ThumbnailCamera { target: target.clone(), path, frames: 0 };
    commands.spawn((capture, Camera3d::default(), RenderTarget::Image(target.into()), projection.clone(), Tonemapping::None, DebandDither::Disabled, *transform));
}

/// Takes each thumbnail camera's picture once it has drawn, then lets the
/// camera go.
pub(super) fn finish_capture(mut commands: Commands, mut cameras: Query<(Entity, &mut ThumbnailCamera)>) {
    for (entity, mut capture) in &mut cameras {
        capture.frames += 1;
        if capture.frames != CAPTURE_FRAMES {
            continue;
        }
        let path = capture.path.clone();
        let mut screenshot = commands.spawn(Screenshot::image(capture.target.clone()));
        screenshot.observe(move |captured: On<ScreenshotCaptured>, mut commands: Commands| {
            if let Err(error) = save_thumbnail(&captured.image, &path) {
                log::warn!("cannot save the thumbnail {}: {error}", path.display());
            }
            commands.entity(entity).despawn();
        });
    }
}

/// `ScreenshotTool.SaveThumbnail`'s cut and scale: the widest 16:9 part
/// of the middle, at the view's width (or height) scaled to at most 720.
fn save_thumbnail(image: &Image, path: &Path) -> Result<(), String> {
    let picture = image.clone().try_into_dynamic().map_err(|error| error.to_string())?;
    let rgb = picture.to_rgb8();
    let (width, height) = (rgb.width(), rgb.height());
    let (crop_width, crop_height) = thumbnail_crop(width, height);
    let left = (width - crop_width) / 2;
    let top = (height - crop_height) / 2;
    let cropped = image::imageops::crop_imm(&rgb, left, top, crop_width, crop_height).to_image();
    let out_width = width.min(MAX_WIDTH);
    let out_height = round_to_even((out_width as f32 * ASPECT) as f64).max(1);
    let scaled = image::imageops::resize(&cropped, out_width.max(1), out_height, image::imageops::FilterType::Triangle);
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    let file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    let mut writer = std::io::BufWriter::new(file);
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, QUALITY);
    encoder.encode_image(&scaled).map_err(|error| error.to_string())
}

/// The part of a `width` × `height` view a thumbnail shows: its width at
/// 16:9, or its height at 16:9 when the view is too short.
fn thumbnail_crop(width: u32, height: u32) -> (u32, u32) {
    let crop_height = round_to_even((width as f32 * ASPECT) as f64);
    if crop_height <= height {
        return (width, crop_height.max(1));
    }
    let crop_width = round_to_even((height as f32 * 1.777_777_8) as f64).min(width);
    (crop_width.max(1), height.max(1))
}

/// `Mathf.RoundToInt`: halves go to the even neighbour.
fn round_to_even(value: f64) -> u32 {
    value.round_ties_even().max(0.0) as u32
}

/// MD5 (RFC 1321) as lowercase hex.
pub fn md5_hex(bytes: &[u8]) -> String {
    const SHIFTS: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let constants: Vec<u32> = (0..64).map(|index| ((index as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32).collect();
    let mut message = bytes.to_vec();
    let bit_length = (bytes.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_length.to_le_bytes());
    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    for chunk in message.chunks(64) {
        let words: Vec<u32> = chunk.chunks(4).map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]])).collect();
        let [mut a, mut b, mut c, mut d] = state;
        for index in 0..64 {
            let (mixed, word) = match index / 16 {
                0 => ((b & c) | (!b & d), index),
                1 => ((d & b) | (!d & c), (5 * index + 1) % 16),
                2 => (b ^ c ^ d, (3 * index + 5) % 16),
                _ => (c ^ (b | !d), (7 * index) % 16),
            };
            let rotated = a.wrapping_add(mixed).wrapping_add(constants[index]).wrapping_add(words[word]).rotate_left(SHIFTS[index]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(rotated);
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
    }
    state.iter().flat_map(|word| word.to_le_bytes()).map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_matches_known_digests() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"The quick brown fox jumps over the lazy dog"), "9e107d9d372bb6826bd81d3542a419d6");
        let long = vec![b'a'; 1000];
        assert_eq!(md5_hex(&long), "cabe45dcc9ae5b66ba86600cca6b8ba8");
    }

    #[test]
    fn thumbnails_cut_the_middle_at_16_by_9() {
        assert_eq!(thumbnail_crop(1600, 1000), (1600, 900));
        assert_eq!(thumbnail_crop(1600, 500), (889, 500));
        let central = thumbnail_path(Path::new("/projects/a.terrain"), "Local").unwrap();
        assert_eq!(central, PathBuf::from("/projects/a.jpg"));
    }
}
