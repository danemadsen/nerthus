//! JPEG: written as Magick.NET writes it, a colour map in
//! the build's `ColorSpace`, alpha dropped. ImageMagick then asks libjpeg
//! for quality 92 with no chroma subsampling (it keeps 1×1 sampling from
//! quality 90 up), optimized Huffman tables and the default integer DCT;
//! the `jpeg-encoder` crate is set up the same way. The samples are
//! ImageMagick's 8-bit values: CMYK images are written as YCCK, the YCbCr
//! colourspaces' values as they are, grey images (and sRGB-compatible
//! images whose pixels are all grey) as their luma, others as RGB after
//! going back to sRGB when they are not sRGB-compatible.

use std::io;
use std::path::Path;

use jpeg_encoder::{ColorType, Encoder, SamplingFactor};
use nerthus_engine::Map;

use crate::colorspace::Colorspace;
use crate::magick::{quantum_to_char, MagickImage};

/// ImageMagick's JPEG quality when none is set.
const QUALITY: u8 = 92;

pub fn write_jpeg(path: &Path, map: &Map, color_space: Colorspace) -> io::Result<()> {
    let mut image = MagickImage::from_map(map);
    if map.channel_count() > 1 {
        image.transform_colorspace(color_space);
    }
    let (samples, color_type) = jpeg_samples(&mut image);
    let mut encoder = Encoder::new_file(path, QUALITY).map_err(io::Error::other)?;
    encoder.set_sampling_factor(SamplingFactor::F_1_1);
    encoder.set_optimized_huffman_tables(true);
    encoder.encode(&samples, image.width as u16, image.height as u16, color_type).map_err(io::Error::other)
}

/// `WriteJPEGImage`'s choice of colour space and its scanlines.
fn jpeg_samples(image: &mut MagickImage) -> (Vec<u8>, ColorType) {
    let colorspace = image.colorspace;
    if colorspace == Colorspace::Cmyk {
        // IM hands libjpeg inverted CMYK, which libjpeg makes YCCK from;
        // `CmykAsYcck` makes the same from CMYK as it is.
        return (color_samples(image, 4), ColorType::CmykAsYcck);
    }
    if colorspace.is_ycbcr_compatible() {
        return (color_samples(image, 3), ColorType::Ycbcr);
    }
    if !colorspace.is_gray() && !colorspace.is_srgb_compatible() {
        image.transform_colorspace(Colorspace::SRgb);
    }
    if image.is_gray() {
        return (luma_samples(image), ColorType::Luma);
    }
    (color_samples(image, 3), ColorType::Rgb)
}

/// The first `count` channels of each pixel as 8-bit samples.
fn color_samples(image: &MagickImage, count: usize) -> Vec<u8> {
    let mut samples = Vec::with_capacity(image.pixel_count() * count);
    for pixel in image.quanta.chunks(image.channels) {
        for quantum in &pixel[..count] {
            samples.push(quantum_to_char(*quantum));
        }
    }
    samples
}

/// `GetPixelLuma` of each pixel (a grey image's red, green and blue are
/// its grey), as 8-bit samples.
fn luma_samples(image: &MagickImage) -> Vec<u8> {
    let mut samples = Vec::with_capacity(image.pixel_count());
    for pixel in 0..image.pixel_count() {
        let [red, green, blue] = image.rgb(pixel);
        let luma = 0.212656 * red as f64 + 0.715158 * green as f64 + 0.072186 * blue as f64;
        samples.push(quantum_to_char(luma as f32));
    }
    samples
}
