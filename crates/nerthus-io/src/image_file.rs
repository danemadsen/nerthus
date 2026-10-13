//! Image files read the way Magick.NET (IM 7.1.0-6, Q16 HDRI) reads them:
//! every decoder records the colorspace and gamma IM would assign, and the
//! colorspace transforms reproduce IM's pixel maths. This matters because
//! loaded images are converted to linear grey, which sRGB-decodes any file
//! IM considers non-linear.

use std::io::{BufRead, BufReader, Cursor, Read, Seek};
use std::path::Path;

use anyhow::{bail, Context, Result};

/// IM's default image gamma (`1.000f / 2.200f`).
pub const DEFAULT_GAMMA: f64 = (1.0f32 / 2.2f32) as f64;

/// The colorspaces file loading can encounter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colorspace {
    SRgb,
    LinearRgb,
    Gray,
    LinearGray,
}

/// A decoded image with normalised samples (1.0 = QuantumRange).
#[derive(Clone, Debug)]
pub struct DecodedImage {
    pub width: usize,
    pub height: usize,
    /// Colour channels per pixel: 1 for grey colorspaces, 3 otherwise.
    pub channels: usize,
    pub data: Vec<f32>,
    pub alpha: Option<Vec<f32>>,
    pub colorspace: Colorspace,
    pub gamma: f64,
}

/// IM `DecodeGamma`: x^2.4 via a Chebyshev approximation.
fn decode_gamma(x: f64) -> f64 {
    const COEFFICIENTS: [f64; 9] = [
        1.791_748_858_804_327_8,
        0.820_456_143_719_768_6,
        0.027_694_100_686_325_412,
        -0.000_942_443_351_817_621_3,
        0.000_064_355_540_911_469_71,
        -5.722_440_463_606_075e-6,
        5.876_766_943_731_118e-7,
        -6.613_992_005_358_972e-8,
        7.932_324_269_622_745e-9,
    ];
    const POWERS_OF_TWO: [f64; 5] = [1.0, 2.639_015_821_545_788_4, 6.964_404_506_368_992, 1.837_917_367_995_255_8e1, 4.850_293_012_833_273e1];
    let (mantissa, exponent) = frexp(x);
    let p = chebyshev(mantissa, &COEFFICIENTS);
    let (quotient, remainder) = floor_div(exponent - 1, 5);
    x * ldexp(POWERS_OF_TWO[remainder as usize] * p, 7 * quotient)
}

/// IM `EncodeGamma`: x^(1/2.4) via a Chebyshev approximation.
fn encode_gamma(x: f64) -> f64 {
    const COEFFICIENTS: [f64; 9] = [
        1.175_820_023_299_690_3,
        0.166_657_630_948_890_6,
        -0.008_315_489_493_904_213,
        0.000_751_879_767_804_202_8,
        -0.000_083_240_178_519_391_8,
        0.000_010_229_209_410_070_008,
        -1.340_046_640_986_024_6e-06,
        1.833_342_224_163_537_5e-7,
        -2.587_859_676_134_886e-8,
    ];
    const POWERS_OF_TWO: [f64; 12] = [
        1.0,
        1.334_839_854_170_034_4,
        1.781_797_436_280_678_5,
        2.378_414_230_005_442,
        3.174_802_103_936_399,
        4.237_852_377_437_181,
        5.656_854_249_492_381,
        7.550_994_501_453_548,
        1.007_936_839_915_898_6e1,
        1.345_434_264_405_943_4e1,
        1.795_939_277_294_997e1,
        2.397_291_323_002_690_8e1,
    ];
    let (mantissa, exponent) = frexp(x);
    let p = chebyshev(mantissa, &COEFFICIENTS);
    let (quotient, remainder) = floor_div(exponent - 1, 12);
    ldexp(POWERS_OF_TWO[remainder as usize] * p, 5 * quotient)
}

fn chebyshev(mantissa: f64, coefficients: &[f64; 9]) -> f64 {
    let mut terms = [0.0f64; 9];
    terms[0] = 1.0;
    terms[1] = 4.0 * mantissa - 3.0;
    for index in 2..9 {
        terms[index] = 2.0 * terms[1] * terms[index - 1] - terms[index - 2];
    }
    let mut sum = 0.0;
    for (coefficient, term) in coefficients.iter().zip(terms.iter()) {
        sum += coefficient * term;
    }
    sum
}

/// C `div` adjusted to a non-negative remainder, as IM does.
fn floor_div(value: i32, divisor: i32) -> (i32, i32) {
    let mut quotient = value / divisor;
    let mut remainder = value % divisor;
    if remainder < 0 {
        quotient -= 1;
        remainder += divisor;
    }
    (quotient, remainder)
}

/// C `frexp`: mantissa in [0.5, 1) and exponent.
fn frexp(value: f64) -> (f64, i32) {
    if value == 0.0 || !value.is_finite() {
        return (value, 0);
    }
    let bits = value.to_bits();
    let exponent_bits = ((bits >> 52) & 0x7ff) as i32;
    if exponent_bits == 0 {
        // Subnormal: normalise first.
        let (mantissa, exponent) = frexp(value * f64::powi(2.0, 64));
        return (mantissa, exponent - 64);
    }
    let exponent = exponent_bits - 1022;
    let mantissa_bits = (bits & !(0x7ffu64 << 52)) | (1022u64 << 52);
    (f64::from_bits(mantissa_bits), exponent)
}

fn ldexp(value: f64, exponent: i32) -> f64 {
    value * 2.0f64.powi(exponent)
}

/// IM `DecodePixelGamma` on a normalised value.
pub fn decode_pixel_gamma(value: f64) -> f64 {
    if value <= 0.040_448_236_277_107_6 {
        return value / 12.92f32 as f64;
    }
    decode_gamma((value + 0.055) / 1.055)
}

/// IM `EncodePixelGamma` on a normalised value.
pub fn encode_pixel_gamma(value: f64) -> f64 {
    if value <= 0.003_130_668_442_500_588_3 {
        return 12.92f32 as f64 * value;
    }
    1.055 * encode_gamma(value) - 0.055
}

const LUMA_RED: f64 = 0.212656;
const LUMA_GREEN: f64 = 0.715158;
const LUMA_BLUE: f64 = 0.072186;

impl DecodedImage {
    fn pixel_count(&self) -> usize {
        self.width * self.height
    }

    /// IM `SetImageColorspace`: relabels and resets gamma.
    fn set_colorspace(&mut self, colorspace: Colorspace) {
        if self.colorspace == colorspace {
            return;
        }
        self.colorspace = colorspace;
        self.gamma = match colorspace {
            Colorspace::LinearGray | Colorspace::LinearRgb => 1.0,
            Colorspace::Gray | Colorspace::SRgb => DEFAULT_GAMMA,
        };
    }

    /// `TransformsRGBImage`: any colorspace to sRGB.
    fn transform_to_srgb(&mut self) {
        match self.colorspace {
            Colorspace::SRgb => {}
            Colorspace::Gray => {
                self.data = self.data.iter().flat_map(|&gray| [gray, gray, gray]).collect();
                self.channels = 3;
            }
            Colorspace::LinearGray => {
                self.data = self
                    .data
                    .iter()
                    .flat_map(|&gray| {
                        let encoded = encode_pixel_gamma(gray as f64) as f32;
                        [encoded, encoded, encoded]
                    })
                    .collect();
                self.channels = 3;
            }
            Colorspace::LinearRgb => {
                for value in self.data.iter_mut() {
                    *value = encode_pixel_gamma(*value as f64) as f32;
                }
            }
        }
        self.set_colorspace(Colorspace::SRgb);
    }

    /// `sRGBTransformImage`: sRGB to another colorspace.
    fn transform_from_srgb(&mut self, target: Colorspace) {
        match target {
            Colorspace::SRgb => {}
            Colorspace::Gray | Colorspace::LinearGray => {
                let linear = target == Colorspace::LinearGray;
                let mut gray = Vec::with_capacity(self.pixel_count());
                for pixel in self.data.chunks(3) {
                    let (red, green, blue) = if linear {
                        (decode_pixel_gamma(pixel[0] as f64), decode_pixel_gamma(pixel[1] as f64), decode_pixel_gamma(pixel[2] as f64))
                    } else {
                        (pixel[0] as f64, pixel[1] as f64, pixel[2] as f64)
                    };
                    gray.push((LUMA_RED * red + LUMA_GREEN * green + LUMA_BLUE * blue) as f32);
                }
                self.data = gray;
                self.channels = 1;
            }
            Colorspace::LinearRgb => {
                for value in self.data.iter_mut() {
                    *value = decode_pixel_gamma(*value as f64) as f32;
                }
            }
        }
        self.set_colorspace(target);
    }

    /// IM `TransformImageColorspace` (Magick.NET `ColorSpace` setter).
    pub fn transform_colorspace(&mut self, target: Colorspace) {
        if self.colorspace == target {
            return;
        }
        if target == Colorspace::SRgb {
            self.transform_to_srgb();
            return;
        }
        if self.colorspace != Colorspace::SRgb {
            self.transform_to_srgb();
        }
        self.transform_from_srgb(target);
    }

    /// IM `SetImageType(GrayscaleType)` (Magick.NET `ColorType = Grayscale`).
    pub fn set_grayscale_type(&mut self) {
        self.transform_colorspace(Colorspace::Gray);
        self.alpha = None;
    }

    /// IM `GammaImage(gamma)` in HDRI builds.
    pub fn gamma_correct(&mut self, gamma: f64) {
        if gamma == 1.0 {
            return;
        }
        let exponent = if gamma.abs() >= 1e-12 { 1.0 / gamma } else { gamma.signum() / 1e-12 };
        for value in self.data.iter_mut() {
            let sample = *value as f64;
            let corrected = if sample < 0.0 { -(-sample).powf(exponent) } else { sample.powf(exponent) };
            *value = corrected as f32;
        }
        if self.gamma != 0.0 {
            self.gamma *= gamma;
        }
    }

    /// `Alpha(AlphaOption.Off)`.
    pub fn alpha_off(&mut self) {
        self.alpha = None;
    }

    /// `Alpha(AlphaOption.Remove)`: flattens onto IM's white background.
    pub fn alpha_remove(&mut self) {
        let Some(alpha) = self.alpha.take() else {
            return;
        };
        let channels = self.channels;
        for (pixel, &opacity) in self.data.chunks_mut(channels).zip(alpha.iter()) {
            for value in pixel.iter_mut() {
                *value = *value * opacity + 1.0 * (1.0 - opacity);
            }
        }
    }

    /// Interleaved colour + alpha image for resampling.
    fn to_image(&self) -> nerthus_image::Image {
        match &self.alpha {
            Some(alpha) => {
                let channels = self.channels + 1;
                let mut data = Vec::with_capacity(self.pixel_count() * channels);
                for (pixel, &opacity) in self.data.chunks(self.channels).zip(alpha.iter()) {
                    data.extend_from_slice(pixel);
                    data.push(opacity);
                }
                nerthus_image::Image::from_data(self.width, self.height, channels, data)
            }
            None => nerthus_image::Image::from_data(self.width, self.height, self.channels, self.data.clone()),
        }
    }

    /// The image as a `nerthus_image::Image` with the same colourspace and
    /// any alpha interleaved as the last channel, for further Magick
    /// operations.
    pub fn into_image(self) -> nerthus_image::Image {
        let mut image = self.to_image();
        image.colorspace = match self.colorspace {
            Colorspace::SRgb => nerthus_image::Colorspace::SRgb,
            Colorspace::LinearRgb => nerthus_image::Colorspace::LinearRgb,
            Colorspace::Gray => nerthus_image::Colorspace::Gray,
            Colorspace::LinearGray => nerthus_image::Colorspace::LinearGray,
        };
        image
    }

    fn take_image(&mut self, image: nerthus_image::Image) {
        self.width = image.width;
        self.height = image.height;
        if self.alpha.is_some() {
            let channels = image.channels;
            let mut colors = Vec::with_capacity(image.width * image.height * self.channels);
            let mut alpha = Vec::with_capacity(image.width * image.height);
            for pixel in image.data.chunks(channels) {
                colors.extend_from_slice(&pixel[..channels - 1]);
                alpha.push(pixel[channels - 1]);
            }
            self.data = colors;
            self.alpha = Some(alpha);
        } else {
            self.data = image.data;
        }
    }

    /// `MagickImage.Resize(width, height)` with IM's default filter choice.
    pub fn resize(&mut self, width: usize, height: usize) {
        let image = self.to_image();
        let resized = nerthus_image::resize::resize(&image, width, height);
        self.take_image(resized);
    }

    /// `MagickImage.AdaptiveResize(width, height)`.
    pub fn adaptive_resize(&mut self, width: usize, height: usize) {
        let image = self.to_image();
        let resized = nerthus_image::resize::adaptive_resize(&image, width, height);
        self.take_image(resized);
    }

    /// IM `AutoLevelImage`: one range over all colour channels.
    pub fn auto_level(&mut self) {
        let mut min = f64::MAX;
        let mut max = f64::MIN;
        for &value in &self.data {
            min = min.min(value as f64);
            max = max.max(value as f64);
        }
        if (min - max).abs() < 1.0e-12 {
            return;
        }
        let scale = 1.0 / (max - min);
        for value in self.data.iter_mut() {
            *value = ((*value as f64 - min) * scale) as f32;
        }
    }
}

/// Decodes any image file the loaders hand to Magick.NET.
pub fn decode_image(path: &Path) -> Result<DecodedImage> {
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
    match extension.as_str() {
        "png" => decode_png(path),
        "tif" | "tiff" => decode_tiff(path),
        "exr" => decode_exr(path),
        "pfm" => decode_pfm(path),
        "hdr" => decode_with_image_crate(path, Colorspace::LinearRgb),
        "psd" | "svg" => bail!("{} files are not supported yet", extension.to_uppercase()),
        _ => decode_with_image_crate(path, Colorspace::SRgb),
    }
}

/// Decodes an in-memory image (a node's resource), picking the reader by
/// the data's signature as IM does.
pub fn decode_image_bytes(bytes: &[u8]) -> Result<DecodedImage> {
    let is_png = bytes.starts_with(&[0x89, b'P', b'N', b'G']);
    let is_tiff = bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*");
    let is_pfm = bytes.starts_with(b"PF") || bytes.starts_with(b"Pf");
    if is_png {
        return decode_png_from(Cursor::new(bytes));
    }
    if is_tiff {
        return decode_tiff_from(Cursor::new(bytes));
    }
    if is_pfm {
        return parse_pfm(bytes);
    }
    let decoded = image::load_from_memory(bytes).context("unrecognised image data")?;
    Ok(from_dynamic_image(decoded, Colorspace::SRgb))
}

fn decode_png(path: &Path) -> Result<DecodedImage> {
    let file = std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode_png_from(BufReader::new(file))
}

/// IM's PNG reader (`coders/png.c`): gray types become GRAY or LinearGRAY by
/// gAMA, colour types sRGB or RGB.
fn decode_png_from<R: BufRead + Seek>(reader: R) -> Result<DecodedImage> {
    let mut decoder = png::Decoder::new(reader);
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info()?;
    let info = reader.info();
    let mut gamma = DEFAULT_GAMMA;
    if info.srgb.is_some() {
        gamma = 0.45455;
    }
    if let Some(file_gamma) = info.gamma() {
        gamma = file_gamma.into_value() as f64;
    }
    let mut buffer = vec![0u8; reader.output_buffer_size().context("PNG too large")?];
    let frame = reader.next_frame(&mut buffer)?;
    let width = frame.width as usize;
    let height = frame.height as usize;
    let samples_per_pixel = frame.color_type.samples();
    let sixteen_bit = frame.bit_depth == png::BitDepth::Sixteen;
    let mut samples = Vec::with_capacity(width * height * samples_per_pixel);
    if sixteen_bit {
        let (pairs, _) = buffer[..frame.buffer_size()].as_chunks::<2>();
        for pair in pairs {
            samples.push(u16::from_be_bytes(*pair) as f32 / 65535.0);
        }
    } else {
        for &byte in &buffer[..frame.buffer_size()] {
            samples.push(byte as f32 / 255.0);
        }
    }
    let (color_channels, has_alpha) = match frame.color_type {
        png::ColorType::Grayscale => (1, false),
        png::ColorType::GrayscaleAlpha => (1, true),
        png::ColorType::Rgb | png::ColorType::Indexed => (3, false),
        png::ColorType::Rgba => (3, true),
    };
    let (data, alpha) = split_alpha(&samples, color_channels, has_alpha);
    let colorspace = match (color_channels == 1, gamma > 0.75) {
        (true, true) => Colorspace::LinearGray,
        (true, false) => Colorspace::Gray,
        (false, true) => Colorspace::LinearRgb,
        (false, false) => Colorspace::SRgb,
    };
    Ok(DecodedImage {
        width,
        height,
        channels: color_channels,
        data,
        alpha,
        colorspace,
        gamma,
    })
}

fn split_alpha(samples: &[f32], color_channels: usize, has_alpha: bool) -> (Vec<f32>, Option<Vec<f32>>) {
    if !has_alpha {
        return (samples.to_vec(), None);
    }
    let stride = color_channels + 1;
    let mut colors = Vec::with_capacity(samples.len() / stride * color_channels);
    let mut alpha = Vec::with_capacity(samples.len() / stride);
    for pixel in samples.chunks_exact(stride) {
        colors.extend_from_slice(&pixel[..color_channels]);
        alpha.push(pixel[color_channels]);
    }
    (colors, Some(alpha))
}

fn decode_tiff(path: &Path) -> Result<DecodedImage> {
    let file = std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode_tiff_from(BufReader::new(file))
}

/// IM's TIFF reader: min-is-black/white become GRAY, everything else sRGB.
fn decode_tiff_from<R: Read + Seek>(reader: R) -> Result<DecodedImage> {
    use tiff::decoder::{Decoder, DecodingResult};
    use tiff::tags::{PhotometricInterpretation, Tag};
    let mut decoder = Decoder::new(reader)?;
    let (width, height) = decoder.dimensions()?;
    let photometric = decoder.get_tag_unsigned::<u16>(Tag::PhotometricInterpretation).ok();
    let min_is_white = photometric == Some(PhotometricInterpretation::WhiteIsZero.to_u16());
    let gray = photometric == Some(PhotometricInterpretation::BlackIsZero.to_u16()) || min_is_white;
    let samples_per_pixel = decoder.get_tag_unsigned::<u16>(Tag::SamplesPerPixel).unwrap_or(1) as usize;
    let image = decoder.read_image()?;
    let mut samples: Vec<f32> = match image {
        DecodingResult::U8(values) => values.iter().map(|&value| value as f32 / 255.0).collect(),
        DecodingResult::U16(values) => values.iter().map(|&value| value as f32 / 65535.0).collect(),
        DecodingResult::U32(values) => values.iter().map(|&value| (value as f64 / u32::MAX as f64) as f32).collect(),
        DecodingResult::U64(values) => values.iter().map(|&value| (value as f64 / u64::MAX as f64) as f32).collect(),
        DecodingResult::I8(values) => values.iter().map(|&value| (value as f32 + 128.0) / 255.0).collect(),
        DecodingResult::I16(values) => values.iter().map(|&value| (value as f32 + 32768.0) / 65535.0).collect(),
        DecodingResult::I32(values) => values.iter().map(|&value| ((value as f64 + 2147483648.0) / u32::MAX as f64) as f32).collect(),
        DecodingResult::I64(values) => values.iter().map(|&value| ((value as f64 + 9.223_372_036_854_776e18) / u64::MAX as f64) as f32).collect(),
        DecodingResult::F32(values) => values,
        DecodingResult::F64(values) => values.iter().map(|&value| value as f32).collect(),
        _ => bail!("unsupported TIFF sample format"),
    };
    if min_is_white {
        for value in samples.iter_mut() {
            *value = 1.0 - *value;
        }
    }
    let width = width as usize;
    let height = height as usize;
    let (color_channels, has_alpha) = match (gray, samples_per_pixel) {
        (true, 1) => (1, false),
        (true, _) => (1, true),
        (false, 3) => (3, false),
        (false, 4) => (3, true),
        (false, 1) => (1, false),
        (false, _) => (3, samples_per_pixel > 3),
    };
    let stride = samples_per_pixel.max(1);
    let mut packed = Vec::with_capacity(width * height * (color_channels + has_alpha as usize));
    for pixel in samples.chunks(stride) {
        packed.extend_from_slice(&pixel[..(color_channels + has_alpha as usize).min(pixel.len())]);
    }
    let (data, alpha) = split_alpha(&packed, color_channels, has_alpha);
    let colorspace = if color_channels == 1 { Colorspace::Gray } else { Colorspace::SRgb };
    Ok(DecodedImage {
        width,
        height,
        channels: color_channels,
        data,
        alpha,
        colorspace,
        gamma: DEFAULT_GAMMA,
    })
}

/// IM's EXR reader: linear RGB with gamma 1.
fn decode_exr(path: &Path) -> Result<DecodedImage> {
    use exr::prelude::*;
    let image = read_first_flat_layer_from_file(path)?;
    let layer = image.layer_data;
    let width = layer.size.0;
    let height = layer.size.1;
    let find = |names: &[&str]| -> Option<Vec<f32>> {
        layer
            .channel_data
            .list
            .iter()
            .find(|channel| names.contains(&channel.name.to_string().as_str()))
            .map(|channel| channel.sample_data.values_as_f32().collect())
    };
    let first: Vec<f32> = layer.channel_data.list.first().context("EXR has no channels")?.sample_data.values_as_f32().collect();
    let red = find(&["R", "r", "Y", "y"]).unwrap_or_else(|| first.clone());
    let green = find(&["G", "g"]).unwrap_or_else(|| red.clone());
    let blue = find(&["B", "b"]).unwrap_or_else(|| red.clone());
    let alpha = find(&["A", "a"]);
    let mut data = Vec::with_capacity(width * height * 3);
    for index in 0..width * height {
        data.extend_from_slice(&[red[index], green[index], blue[index]]);
    }
    Ok(DecodedImage {
        width,
        height,
        channels: 3,
        data,
        alpha,
        colorspace: Colorspace::LinearRgb,
        gamma: 1.0,
    })
}

fn decode_pfm(path: &Path) -> Result<DecodedImage> {
    let bytes = std::fs::read(path)?;
    parse_pfm(&bytes)
}

/// IM's PNM reader for PFM ("PF" colour, "Pf" grey); rows are stored bottom-up.
fn parse_pfm(bytes: &[u8]) -> Result<DecodedImage> {
    let mut fields = Vec::new();
    let mut cursor = 0;
    while fields.len() < 4 && cursor < bytes.len() {
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let start = cursor;
        while cursor < bytes.len() && !bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        fields.push(String::from_utf8_lossy(&bytes[start..cursor]).to_string());
    }
    cursor += 1;
    if fields.len() < 4 {
        bail!("invalid PFM header");
    }
    let channels = if fields[0] == "PF" { 3 } else { 1 };
    let width: usize = fields[1].parse()?;
    let height: usize = fields[2].parse()?;
    let scale: f32 = fields[3].parse()?;
    let little_endian = scale < 0.0;
    let mut data = vec![0.0f32; width * height * channels];
    for row in 0..height {
        let target_row = height - 1 - row;
        for column in 0..width {
            for channel in 0..channels {
                let offset = cursor + ((row * width + column) * channels + channel) * 4;
                if offset + 4 > bytes.len() {
                    bail!("truncated PFM");
                }
                let raw = [bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]];
                let value = if little_endian { f32::from_le_bytes(raw) } else { f32::from_be_bytes(raw) };
                data[(target_row * width + column) * channels + channel] = value;
            }
        }
    }
    Ok(DecodedImage {
        width,
        height,
        channels,
        data,
        alpha: None,
        colorspace: if channels == 1 { Colorspace::Gray } else { Colorspace::SRgb },
        gamma: DEFAULT_GAMMA,
    })
}

/// JPEG, BMP, TGA and HDR through the `image` crate.
fn decode_with_image_crate(path: &Path, colour_space: Colorspace) -> Result<DecodedImage> {
    let decoded = image::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    Ok(from_dynamic_image(decoded, colour_space))
}

fn from_dynamic_image(decoded: image::DynamicImage, colour_space: Colorspace) -> DecodedImage {
    let width = decoded.width() as usize;
    let height = decoded.height() as usize;
    let gray_source = matches!(decoded.color(), image::ColorType::L8 | image::ColorType::L16 | image::ColorType::La8 | image::ColorType::La16);
    let has_alpha = decoded.color().has_alpha();
    let (channels, data, alpha) = if gray_source {
        let luma = decoded.to_luma_alpha32f();
        let samples: Vec<f32> = luma.into_raw();
        let (data, alpha) = split_alpha(&samples, 1, true);
        (1, data, if has_alpha { alpha } else { None })
    } else {
        let rgba = decoded.to_rgba32f();
        let samples: Vec<f32> = rgba.into_raw();
        let (data, alpha) = split_alpha(&samples, 3, true);
        (3, data, if has_alpha { alpha } else { None })
    };
    let colorspace = match (channels, colour_space) {
        (1, Colorspace::LinearRgb) => Colorspace::LinearGray,
        (1, _) => Colorspace::Gray,
        (_, space) => space,
    };
    let gamma = if matches!(colorspace, Colorspace::LinearGray | Colorspace::LinearRgb) { 1.0 } else { DEFAULT_GAMMA };
    DecodedImage {
        width,
        height,
        channels,
        data,
        alpha,
        colorspace,
        gamma,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gamma_curves_match_srgb() {
        for step in 0..=100 {
            let value = step as f64 / 100.0;
            let exact_decode = if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) };
            assert!((decode_pixel_gamma(value) - exact_decode).abs() < 1e-6, "decode {value}");
            let roundtrip = encode_pixel_gamma(decode_pixel_gamma(value));
            assert!((roundtrip - value).abs() < 1e-6, "roundtrip {value}");
        }
    }

    fn write_gray_png(path: &Path, gamma: Option<f32>) {
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), 2, 1);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Sixteen);
        if let Some(gamma) = gamma {
            encoder.set_source_gamma(png::ScaledFloat::new(gamma));
        }
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[0x80, 0x00, 0xff, 0xff]).unwrap();
    }

    #[test]
    fn png_gamma_selects_gray_colorspace() {
        let directory = std::env::temp_dir().join(format!("nerthus-png-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let plain = directory.join("plain.png");
        let linear = directory.join("linear.png");
        write_gray_png(&plain, None);
        write_gray_png(&linear, Some(1.0));

        let mut image = decode_image(&plain).unwrap();
        assert_eq!(image.colorspace, Colorspace::Gray);
        image.set_grayscale_type();
        image.transform_colorspace(Colorspace::LinearGray);
        let half = 0x8000 as f64 / 65535.0;
        assert!((image.data[0] as f64 - decode_pixel_gamma(half)).abs() < 1e-6);

        let mut image = decode_image(&linear).unwrap();
        assert_eq!(image.colorspace, Colorspace::LinearGray);
        image.set_grayscale_type();
        image.transform_colorspace(Colorspace::LinearGray);
        assert!((image.data[0] as f64 - half).abs() < 1e-5);
        std::fs::remove_dir_all(&directory).ok();
    }
}
