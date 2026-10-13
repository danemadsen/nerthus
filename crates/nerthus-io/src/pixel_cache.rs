//! Image files decoded into what ImageMagick 7.1.2 (Q16 HDRI, through
//! Magick.NET) holds after reading them: the pixel cache's channels in
//! their order — palette images (`PseudoClass`) carry an index channel —
//! as quanta (floats in 0..65535), with the colorspace and gamma IM
//! assigns. The File node reads the cache raw, so these details reach its
//! maps.

use std::io::{BufReader, Read, Seek};
use std::path::Path;

use anyhow::{bail, Context, Result};

/// IM's default image gamma (`1.000f / 2.200f`).
pub(crate) const DEFAULT_GAMMA: f64 = (1.0f32 / 2.2f32) as f64;

/// One channel of the pixel cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelChannel {
    Gray,
    Red,
    Green,
    Blue,
    Alpha,
    /// The colormap index of a `PseudoClass` image.
    Index,
}

/// The colorspaces IM gives the files the File node reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheColorspace {
    SRgb,
    /// Linear RGB (`RGBColorspace`).
    Rgb,
    Gray,
    LinearGray,
}

/// An image as IM's pixel cache holds it.
#[derive(Clone, Debug)]
pub struct PixelCache {
    pub width: usize,
    pub height: usize,
    /// The cache's channels, in order.
    pub channels: Vec<PixelChannel>,
    /// Interleaved quanta, `channels.len()` per pixel.
    pub data: Vec<f32>,
    pub colorspace: CacheColorspace,
    /// `image->gamma`.
    pub gamma: f64,
}

impl PixelCache {
    pub fn has_channel(&self, channel: PixelChannel) -> bool {
        self.channels.contains(&channel)
    }

    pub fn is_pseudo_class(&self) -> bool {
        self.has_channel(PixelChannel::Index)
    }
}

/// `ScaleCharToQuantum`.
pub(crate) fn char_quantum(value: u8) -> f32 {
    257.0 * value as f32
}

/// Reads an image file the way Magick.NET does (PNG, TIFF, EXR,
/// WebP, JPEG 2000, DDS, MIFF, SVG and DNG as IM's coders read them, and
/// JPEG, BMP, TGA, GIF through the `image` crate).
pub fn read_pixel_cache(path: &Path) -> Result<PixelCache> {
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
    match extension.as_str() {
        "png" => read_png(path),
        "tif" | "tiff" => read_tiff(path),
        "exr" => read_exr(path),
        "webp" => crate::coders::webp::read(path),
        "jp2" | "j2k" | "j2c" | "jpc" => crate::coders::jp2::read(path),
        "dds" => crate::coders::dds::read(path),
        "miff" => crate::coders::miff::read(path),
        "svg" => crate::coders::svg::read(path),
        "dng" => crate::coders::dng::read(path),
        "psd" => bail!("{} files are not supported", extension.to_uppercase()),
        _ => read_with_image_crate(path),
    }
}

/// The samples of one PNG row, unpacked from their bit depth.
fn unpack_row(row: &[u8], bit_depth: u8, count: usize) -> Vec<u32> {
    match bit_depth {
        16 => {
            let (pairs, _) = row.as_chunks::<2>();
            pairs.iter().take(count).map(|pair| u16::from_be_bytes(*pair) as u32).collect()
        }
        8 => row.iter().take(count).map(|&byte| byte as u32).collect(),
        depth => {
            let per_byte = 8 / depth as usize;
            let mask = (1u32 << depth) - 1;
            let mut samples = Vec::with_capacity(count);
            for index in 0..count {
                let byte = row[index / per_byte] as u32;
                let shift = 8 - depth as usize * (index % per_byte + 1);
                samples.push((byte >> shift) & mask);
            }
            samples
        }
    }
}

/// IM's PNG reader (`coders/png.c`): palette images and grey below 16
/// bits are `PseudoClass` (their index kept as a channel); grey types are
/// GRAY or, with a gamma above 0.75, LinearGRAY; colour types sRGB or RGB.
fn read_png(path: &Path) -> Result<PixelCache> {
    let file = std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info()?;
    let info = reader.info().clone();
    let mut gamma = DEFAULT_GAMMA;
    if info.srgb.is_some() {
        gamma = 0.45455;
    }
    if let Some(file_gamma) = info.gamma() {
        gamma = file_gamma.into_value() as f64;
    }
    let palette: Option<Vec<u8>> = info.palette.as_ref().map(|palette| palette.to_vec());
    let width = info.width as usize;
    let height = info.height as usize;
    let bit_depth = info.bit_depth as u8;
    let color_type = info.color_type;

    let mut buffer = vec![0u8; reader.output_buffer_size().context("PNG too large")?];
    let frame = reader.next_frame(&mut buffer)?;
    let line_size = frame.line_size;

    let (channels, colorspace): (Vec<PixelChannel>, CacheColorspace) = match color_type {
        png::ColorType::Grayscale if bit_depth < 16 => (vec![PixelChannel::Gray, PixelChannel::Index], gray_colorspace(gamma)),
        png::ColorType::Grayscale => (vec![PixelChannel::Gray], gray_colorspace(gamma)),
        png::ColorType::GrayscaleAlpha => (vec![PixelChannel::Gray, PixelChannel::Alpha], gray_colorspace(gamma)),
        png::ColorType::Rgb => (vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue], colour_colorspace(gamma)),
        png::ColorType::Rgba => (vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Alpha], colour_colorspace(gamma)),
        png::ColorType::Indexed => (vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Index], colour_colorspace(gamma)),
    };
    let samples_per_pixel = color_type.samples();
    // `AcquireImageColormap`'s grey ramp for grey PseudoClass images.
    let ramp_step = 65535.0 / ((1u32 << bit_depth.min(15)) - 1).max(1) as f64;

    let mut data = Vec::with_capacity(width * height * channels.len());
    for y in 0..height {
        let row = &buffer[y * line_size..(y + 1) * line_size];
        let samples = unpack_row(row, bit_depth, width * samples_per_pixel);
        for pixel in samples.chunks_exact(samples_per_pixel) {
            match color_type {
                png::ColorType::Grayscale if bit_depth < 16 => {
                    data.push((pixel[0] as f64 * ramp_step) as f32);
                    data.push(pixel[0] as f32);
                }
                png::ColorType::Indexed => {
                    let index = pixel[0] as usize;
                    let palette = palette.as_deref().unwrap_or(&[]);
                    let entry = palette.get(index * 3..index * 3 + 3).unwrap_or(&[0, 0, 0]);
                    data.extend([char_quantum(entry[0]), char_quantum(entry[1]), char_quantum(entry[2]), index as f32]);
                }
                _ if bit_depth == 16 => data.extend(pixel.iter().map(|&value| value as f32)),
                _ => data.extend(pixel.iter().map(|&value| char_quantum(value as u8))),
            }
        }
    }
    Ok(PixelCache { width, height, channels, data, colorspace, gamma })
}

fn gray_colorspace(gamma: f64) -> CacheColorspace {
    if gamma > 0.75 {
        CacheColorspace::LinearGray
    } else {
        CacheColorspace::Gray
    }
}

fn colour_colorspace(gamma: f64) -> CacheColorspace {
    if gamma > 0.75 {
        CacheColorspace::Rgb
    } else {
        CacheColorspace::SRgb
    }
}

fn read_tiff(path: &Path) -> Result<PixelCache> {
    let file = std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    read_tiff_from(BufReader::new(file))
}

/// IM's TIFF reader: min-is-black/white grey, everything else sRGB;
/// integer samples scaled to quanta, floats multiplied by the quantum range.
fn read_tiff_from<R: Read + Seek>(reader: R) -> Result<PixelCache> {
    use tiff::decoder::{Decoder, DecodingResult};
    use tiff::tags::{PhotometricInterpretation, Tag};
    let mut decoder = Decoder::new(reader)?;
    let (width, height) = decoder.dimensions()?;
    let photometric = decoder.get_tag_unsigned::<u16>(Tag::PhotometricInterpretation).ok();
    let min_is_white = photometric == Some(PhotometricInterpretation::WhiteIsZero.to_u16());
    let gray = photometric == Some(PhotometricInterpretation::BlackIsZero.to_u16()) || min_is_white;
    let samples_per_pixel = decoder.get_tag_unsigned::<u16>(Tag::SamplesPerPixel).unwrap_or(1) as usize;
    let image = decoder.read_image()?;
    let mut quanta: Vec<f32> = match image {
        DecodingResult::U8(values) => values.iter().map(|&value| char_quantum(value)).collect(),
        DecodingResult::U16(values) => values.iter().map(|&value| value as f32).collect(),
        DecodingResult::U32(values) => values.iter().map(|&value| (value as f64 / 65537.0) as f32).collect(),
        DecodingResult::F32(values) => values.iter().map(|&value| value * 65535.0f32).collect(),
        DecodingResult::F64(values) => values.iter().map(|&value| (value * 65535.0) as f32).collect(),
        _ => bail!("unsupported TIFF sample format"),
    };
    if min_is_white {
        for value in quanta.iter_mut() {
            *value = 65535.0 - *value;
        }
    }
    let channels = match (gray, samples_per_pixel) {
        (true, 1) => vec![PixelChannel::Gray],
        (true, _) => vec![PixelChannel::Gray, PixelChannel::Alpha],
        (false, 1) => vec![PixelChannel::Gray],
        (false, 3) => vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue],
        (false, _) => vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Alpha],
    };
    let mut data = Vec::with_capacity(width as usize * height as usize * channels.len());
    for pixel in quanta.chunks(samples_per_pixel.max(1)) {
        data.extend(pixel.iter().take(channels.len()));
    }
    let colorspace = if channels[0] == PixelChannel::Gray { CacheColorspace::Gray } else { CacheColorspace::SRgb };
    Ok(PixelCache { width: width as usize, height: height as usize, channels, data, colorspace, gamma: DEFAULT_GAMMA })
}

/// IM's EXR reader: linear RGB(A), gamma 1, `QuantumRange·value` per sample.
fn read_exr(path: &Path) -> Result<PixelCache> {
    use exr::prelude::*;
    let image = read_first_flat_layer_from_file(path)?;
    let layer = image.layer_data;
    let width = layer.size.0;
    let height = layer.size.1;
    let find = |names: &[&str]| -> Option<Vec<f32>> {
        let channel = layer.channel_data.list.iter().find(|channel| names.contains(&channel.name.to_string().as_str()))?;
        Some(channel.sample_data.values_as_f32().collect())
    };
    let first: Vec<f32> = layer.channel_data.list.first().context("EXR has no channels")?.sample_data.values_as_f32().collect();
    let red = find(&["R", "r", "Y", "y"]).unwrap_or_else(|| first.clone());
    let green = find(&["G", "g"]).unwrap_or_else(|| red.clone());
    let blue = find(&["B", "b"]).unwrap_or_else(|| red.clone());
    let alpha = find(&["A", "a"]);
    let quantum = |value: f32| (65535.0 * value as f64) as f32;
    let mut channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    if alpha.is_some() {
        channels.push(PixelChannel::Alpha);
    }
    let mut data = Vec::with_capacity(width * height * channels.len());
    for index in 0..width * height {
        data.extend([quantum(red[index]), quantum(green[index]), quantum(blue[index])]);
        if let Some(alpha) = &alpha {
            data.push(quantum(alpha[index]));
        }
    }
    Ok(PixelCache { width, height, channels, data, colorspace: CacheColorspace::Rgb, gamma: 1.0 })
}

/// JPEG, BMP, TGA and GIF through the `image` crate: 8-bit samples, grey
/// JPEGs and GIFs as `PseudoClass` (IM reads them with a colormap).
fn read_with_image_crate(path: &Path) -> Result<PixelCache> {
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
    let decoded = image::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let width = decoded.width() as usize;
    let height = decoded.height() as usize;
    let is_gray = matches!(decoded.color(), image::ColorType::L8 | image::ColorType::L16);
    let pseudo_gray = is_gray && matches!(extension.as_str(), "jpg" | "jpeg" | "gif");
    let (channels, data) = if is_gray {
        let luma = decoded.to_luma8();
        let mut data = Vec::with_capacity(width * height * 2);
        for &value in luma.as_raw() {
            data.push(char_quantum(value));
            if pseudo_gray {
                data.push(value as f32);
            }
        }
        let channels = if pseudo_gray { vec![PixelChannel::Gray, PixelChannel::Index] } else { vec![PixelChannel::Gray] };
        (channels, data)
    } else if decoded.color().has_alpha() {
        let rgba = decoded.to_rgba8();
        let data = rgba.as_raw().iter().map(|&value| char_quantum(value)).collect();
        (vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Alpha], data)
    } else {
        let rgb = decoded.to_rgb8();
        let data = rgb.as_raw().iter().map(|&value| char_quantum(value)).collect();
        (vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue], data)
    };
    let colorspace = if is_gray { CacheColorspace::Gray } else { CacheColorspace::SRgb };
    Ok(PixelCache { width, height, channels, data, colorspace, gamma: DEFAULT_GAMMA })
}
