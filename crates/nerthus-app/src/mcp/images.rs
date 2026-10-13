//! Pictures for the model: maps, renders and captures scaled down to look
//! at, sent as MCP image content (PNG or JPEG).

use base64::Engine;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::{ExtendedColorType, ImageEncoder, RgbImage};
use nerthus_engine::Map;
use nerthus_render::CapturedPixels;
use serde_json::{json, Value};

const JPEG_QUALITY: u8 = 90;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ImageFormat {
    Png,
    Jpeg,
}

impl ImageFormat {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "png" => Some(Self::Png),
            "jpeg" | "jpg" => Some(Self::Jpeg),
            _ => None,
        }
    }

    fn mime_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
        }
    }
}

/// A map's values as a picture: one channel in grey from its lowest value
/// (black) to its highest (white), with that range; three or more as
/// colours, clamped to 0 to 1. Rows run as the 2D view shows them.
pub(crate) fn map_picture(map: &Map) -> (RgbImage, Option<(f32, f32)>) {
    let width = map.width() as u32;
    let height = map.height() as u32;
    if map.channel_count() >= 3 {
        let (red, green, blue) = (map.channel(0), map.channel(1), map.channel(2));
        let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        let mut rgb = Vec::with_capacity(red.len() * 3);
        for index in 0..red.len() {
            rgb.extend([byte(red[index]), byte(green[index]), byte(blue[index])]);
        }
        let picture = RgbImage::from_raw(width, height, rgb).unwrap_or_default();
        return (picture, None);
    }
    let values = map.channel(0);
    let (low, high) = value_range(values);
    let span = high - low;
    let mut rgb = Vec::with_capacity(values.len() * 3);
    for value in values {
        let fraction = if span > 0.0 && value.is_finite() { (value - low) / span } else { 0.0 };
        let grey = (fraction.clamp(0.0, 1.0) * 255.0).round() as u8;
        rgb.extend([grey, grey, grey]);
    }
    let picture = RgbImage::from_raw(width, height, rgb).unwrap_or_default();
    (picture, Some((low, high)))
}

/// The lowest and highest finite values (0 and 0 when there are none).
pub(crate) fn value_range(values: &[f32]) -> (f32, f32) {
    let mut low = f32::INFINITY;
    let mut high = f32::NEG_INFINITY;
    for value in values.iter().filter(|value| value.is_finite()) {
        low = low.min(*value);
        high = high.max(*value);
    }
    if low > high { (0.0, 0.0) } else { (low, high) }
}

/// A capture's pixels without their alpha.
pub(crate) fn capture_picture(pixels: &CapturedPixels) -> RgbImage {
    let mut rgb = Vec::with_capacity(pixels.rgba.len() / 4 * 3);
    let (rgba_pixels, _) = pixels.rgba.as_chunks::<4>();
    for pixel in rgba_pixels {
        rgb.extend_from_slice(&pixel[..3]);
    }
    RgbImage::from_raw(pixels.width, pixels.height, rgb).unwrap_or_default()
}

/// Scaled down (never up) so its longer side is at most `size`.
pub(crate) fn fit(picture: RgbImage, size: u32) -> RgbImage {
    let longer = picture.width().max(picture.height());
    if longer <= size || longer == 0 {
        return picture;
    }
    let scale = size as f64 / longer as f64;
    let width = ((picture.width() as f64 * scale).round() as u32).max(1);
    let height = ((picture.height() as f64 * scale).round() as u32).max(1);
    image::imageops::resize(&picture, width, height, FilterType::Triangle)
}

/// An image content block.
pub(crate) fn content(picture: &RgbImage, format: ImageFormat) -> Result<Value, String> {
    let mut bytes = Vec::new();
    let (width, height) = picture.dimensions();
    let encoded = match format {
        ImageFormat::Png => PngEncoder::new(&mut bytes).write_image(picture.as_raw(), width, height, ExtendedColorType::Rgb8),
        ImageFormat::Jpeg => JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY).write_image(picture.as_raw(), width, height, ExtendedColorType::Rgb8),
    };
    encoded.map_err(|error| format!("The picture could not be encoded: {error}"))?;
    let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(json!({"type": "image", "data": data, "mimeType": format.mime_type()}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerthus_engine::Resolution;

    #[test]
    fn heights_are_stretched_over_their_range() {
        let map = Map::from_data(Resolution::square(2), vec![0.25, 0.5, 0.75, f32::NAN]);
        let (picture, range) = map_picture(&map);
        assert_eq!(range, Some((0.25, 0.75)));
        assert_eq!(picture.get_pixel(0, 0).0, [0, 0, 0]);
        assert_eq!(picture.get_pixel(0, 1).0, [255, 255, 255]);
    }

    #[test]
    fn pictures_shrink_to_fit() {
        let picture = RgbImage::new(1000, 500);
        let fitted = fit(picture, 400);
        assert_eq!(fitted.dimensions(), (400, 200));
        let small = fit(RgbImage::new(100, 50), 400);
        assert_eq!(small.dimensions(), (100, 50));
    }

    #[test]
    fn content_is_base64() {
        let picture = RgbImage::new(4, 4);
        let block = content(&picture, ImageFormat::Png).unwrap();
        assert_eq!(block["mimeType"], "image/png");
        let data = block["data"].as_str().unwrap();
        let bytes = base64::engine::general_purpose::STANDARD.decode(data).unwrap();
        assert_eq!(&bytes[1..4], b"PNG");
    }
}
