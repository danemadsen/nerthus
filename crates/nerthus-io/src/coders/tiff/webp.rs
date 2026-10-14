//! libtiff's WebP codec (`tif_webp.c`): one WebP image per strip or tile,
//! its size the segment's, decoded by libwebp's incremental decoder to RGB
//! or RGBA by the sample count (fancy upsampling for lossy data).
//!
//! The macOS build of Magick.NET lacks this codec, so it was checked
//! against libwebp's own decoder (`dwebp`) instead.

use std::io::Cursor;

use anyhow::{bail, Result};
use image_webp::{UpsamplingMethod, WebPDecoder};

use super::directory::*;

/// `TWebPSetupDecode`'s checks.
pub(super) fn setup(tiff: &Tiff) -> Result<()> {
    let dir = &tiff.dir;
    if dir.samples_per_pixel != 3 && dir.samples_per_pixel != 4 {
        bail!("WEBP driver doesn't support {} bands. Must be 3 (RGB) or 4 (RGBA) bands.", dir.samples_per_pixel);
    }
    if dir.bits_per_sample != 8 && dir.sample_format != SAMPLEFORMAT_UINT {
        bail!("WEBP driver requires 8 bit unsigned data");
    }
    Ok(())
}

/// `TWebPPreDecode` and `TWebPDecode` for one strip or tile starting at
/// image row `row`.
pub(super) fn decode(tiff: &Tiff, raw: &[u8], output: &mut [u8], is_tile: bool, row: u32) -> Result<()> {
    let dir = &tiff.dir;
    let (width, height) = if is_tile { (dir.tile_width, dir.tile_length) } else { (dir.width, (dir.length - row).min(dir.rows_per_strip)) };
    if width > 16383 || height > 16383 {
        bail!("WEBP maximum image dimensions are 16383 x 16383.");
    }
    let mut decoder = match WebPDecoder::new(Cursor::new(raw)) {
        Ok(decoder) => decoder,
        Err(_) => {
            output.fill(0);
            bail!("WebPGetInfo() failed");
        }
    };
    let (webp_width, webp_height) = decoder.dimensions();
    if webp_width != width || webp_height != height {
        output.fill(0);
        bail!("WebP blob dimension is {webp_width}x{webp_height}. Expected {width}x{height}");
    }
    let samples = dir.samples_per_pixel as usize;
    let bands = if decoder.has_alpha() { 4 } else { 3 };
    if bands != samples && !(bands == 3 && samples == 4) {
        output.fill(0);
        bail!("WebP blob band count is {bands}. Expected {samples}");
    }
    let stride = width as usize * samples;
    if !output.len().is_multiple_of(stride.max(1)) {
        output.fill(0);
        bail!("Fractional scanlines cannot be read");
    }
    decoder.set_lossy_upsampling(UpsamplingMethod::Bilinear);
    let Some(size) = decoder.output_buffer_size() else {
        output.fill(0);
        bail!("Unable to decode WebP data.");
    };
    let mut decoded = vec![0u8; size];
    if decoder.read_image(&mut decoded).is_err() {
        output.fill(0);
        bail!("Unable to decode WebP data.");
    }
    // The decoder gives RGBA only for images with alpha; RGBA output of
    // one without is opaque.
    let pixels = width as usize * height as usize;
    let mut converted = Vec::with_capacity(pixels * samples);
    for pixel in decoded.chunks_exact(bands) {
        converted.extend_from_slice(&pixel[..3]);
        if samples == 4 {
            converted.push(if bands == 4 { pixel[3] } else { 255 });
        }
    }
    if output.len() > converted.len() {
        output.fill(0);
        bail!("Unable to decode WebP data: less lines than expected.");
    }
    let length = output.len();
    output.copy_from_slice(&converted[..length]);
    Ok(())
}
