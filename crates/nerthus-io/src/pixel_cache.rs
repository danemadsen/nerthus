//! Image files decoded into what ImageMagick 7.1.2 (Q16 HDRI, through
//! Magick.NET) holds after reading them: the pixel cache's channels in
//! their order — palette images (`PseudoClass`) carry an index channel —
//! as quanta (floats in 0..65535), with the colorspace and gamma IM
//! assigns. The File node reads the cache raw, so these details reach its
//! maps.

use std::io::Read;
use std::path::Path;

use anyhow::{bail, Context, Result};

/// IM's default image gamma (`1.000/2.200`).
pub(crate) const DEFAULT_GAMMA: f64 = 1.0 / 2.2;

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
    /// A CMYK image's black.
    Black,
    /// A meta channel (a PSD's extra channels).
    Meta,
}

/// The colorspaces IM gives the files the File node reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheColorspace {
    SRgb,
    /// Linear RGB (`RGBColorspace`).
    Rgb,
    Gray,
    LinearGray,
    /// CMYK, as a PSD reads (ink amounts, not negated).
    Cmyk,
    /// CIE Lab, as a PSD reads.
    Lab,
    /// CIE XYZ, as a Radiance XYZE file reads.
    Xyz,
    /// YCbCr, as an uncompressed YCbCr TIFF reads.
    YCbCr,
}

/// `image->type` as a reader leaves it. Writing pixels resets it to
/// `UndefinedType`, so only readers that set it after their pixels give
/// anything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageType {
    Undefined,
    Bilevel,
    Grayscale,
    GrayscaleAlpha,
    Palette,
    PaletteAlpha,
    TrueColor,
    TrueColorAlpha,
    ColorSeparation,
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
    /// `image->type`.
    pub image_type: ImageType,
    /// A `PseudoClass` image's colormap (red, green, blue and alpha, as
    /// doubles); empty for `DirectClass`. Its pixels normally hold their
    /// entries' colours, but colorspace transforms rebuild them from it
    /// (`SyncImage`), so it matters where they don't.
    pub colormap: Vec<[f64; 4]>,
}

impl PixelCache {
    /// A `DirectClass` cache of undefined type.
    pub fn new(width: usize, height: usize, channels: Vec<PixelChannel>, data: Vec<f32>, colorspace: CacheColorspace, gamma: f64) -> Self {
        Self { width, height, channels, data, colorspace, gamma, image_type: ImageType::Undefined, colormap: Vec::new() }
    }

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

/// The readers, by the format ImageMagick picks for a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Png,
    Jpeg,
    Gif,
    Bmp,
    Tiff,
    Exr,
    Webp,
    Jp2,
    Dds,
    Miff,
    Svg,
    Dng,
    Psd,
    Emf,
    Sgi,
    Hdr,
    /// Anything else: the `image` crate.
    Other,
}

/// Camera raw extensions, which name their format outright (LibRaw's).
const RAW_EXTENSIONS: [&str; 31] = [
    "dng", "3fr", "arw", "cr2", "cr3", "crw", "dcr", "dcraw", "erf", "fff", "iiq", "k25", "kdc", "mdc", "mef", "mos", "mrw", "nef", "nrw", "orf", "pef", "raf", "raw", "rmf", "rw2", "rwl", "sr2", "srf",
    "srw", "sti", "x3f",
];

/// The signatures IM recognises files by (its magic list), for the formats
/// read here and the camera raws that share TIFF's; longer ones first.
const SIGNATURES: [(usize, &[u8], Format); 33] = [
    (0, b"MM\x00\x2a\x00\x00\x00\x12\x41\x50\x50\x4c\x45\x44\x4e\x47", Format::Dng),
    (0, b"FUJIFILMCCD-RAW ", Format::Dng),
    (0, b"\x00\x00\x00\x0c\x6a\x50\x20\x20\x0d\x0a\x87\x0a", Format::Jp2),
    (0, b"II\x2a\x00\x10\x00\x00\x00CR\x02", Format::Dng),
    (0, b"MM\x00\x2a\x00\x10\x00\x00RC\x02", Format::Dng),
    (0, b"II\x1a\x00\x00\x00HEAPCCDR", Format::Dng),
    (0, b"Id=ImageMagick", Format::Miff),
    (0, b"id=ImageMagick", Format::Miff),
    (0, b"#?RADIANCE", Format::Hdr),
    (0, b"\x89PNG\r\n\x1a\n", Format::Png),
    (0, b"IIRO\x08\x00\x00\x00", Format::Dng),
    (0, b"IIU\x00\x08\x00\x00\x00", Format::Dng),
    (0, b"IIU\x00\x18\x00\x00\x00", Format::Dng),
    (0, b"II\x2b\x00\x08\x00\x00\x00", Format::Tiff),
    (0, b"MM\x00\x2b\x00\x08\x00\x00", Format::Tiff),
    (0, b"#?RGBE", Format::Hdr),
    (0, b"\x76\x2f\x31\x01", Format::Exr),
    (0, b"\x00MRM", Format::Dng),
    (0, b"II\x2a\x00", Format::Tiff),
    (0, b"MM\x00\x2a", Format::Tiff),
    (0, b"\xff\x4f\xff\x51", Format::Jp2),
    (0, b"\x0d\x0a\x87\x0a", Format::Jp2),
    (0, b"8BPS", Format::Psd),
    (0, b"8BPB", Format::Psd),
    (0, b"GIF8", Format::Gif),
    (0, b"DDS ", Format::Dds),
    (0, b"\xff\xd8\xff", Format::Jpeg),
    (0, b"BA", Format::Bmp),
    (0, b"BM", Format::Bmp),
    (0, b"CI", Format::Bmp),
    (0, b"\x01\xda", Format::Sgi),
    (8, b"WEBP", Format::Webp),
    (40, b"\x20\x45\x4d\x46\x00\x00\x01\x00", Format::Emf),
];

/// The format by extension, when no signature decides.
fn format_from_extension(extension: &str) -> Format {
    match extension {
        "png" | "png8" | "png24" | "png32" | "png48" | "png64" | "png00" => Format::Png,
        "bmp" | "bmp2" | "bmp3" | "dib" => Format::Bmp,
        "gif" | "gif87" => Format::Gif,
        "jpg" | "jpeg" | "jpe" | "jps" | "pjpeg" | "mpo" => Format::Jpeg,
        "tif" | "tiff" | "ptif" | "tiff64" | "group4" => Format::Tiff,
        "exr" => Format::Exr,
        "webp" => Format::Webp,
        "jp2" | "j2k" | "j2c" | "jpc" | "jpm" | "jpt" => Format::Jp2,
        "dds" | "dxt1" | "dxt5" => Format::Dds,
        "miff" => Format::Miff,
        "svg" | "svgz" | "msvg" | "rsvg" => Format::Svg,
        "psd" | "psb" => Format::Psd,
        "emf" | "wmf" => Format::Emf,
        "sgi" => Format::Sgi,
        "hdr" => Format::Hdr,
        _ => Format::Other,
    }
}

/// `SetImageInfo`: a camera raw extension names its format; otherwise the
/// file's leading bytes decide, and the extension only when they match no
/// known signature.
fn detect_format(path: &Path, extension: &str) -> Result<Format> {
    if RAW_EXTENSIONS.contains(&extension) {
        return Ok(Format::Dng);
    }
    let mut head = [0u8; 64];
    let mut file = std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut count = 0;
    loop {
        let read = file.read(&mut head[count..])?;
        if read == 0 {
            break;
        }
        count += read;
        if count == head.len() {
            break;
        }
    }
    let head = &head[..count];
    for (offset, signature, format) in SIGNATURES {
        if head.len() >= offset + signature.len() && &head[offset..offset + signature.len()] == signature {
            return Ok(format);
        }
    }
    Ok(format_from_extension(extension))
}

/// Reads an image file the way Magick.NET does: the format picked as IM
/// picks it, then read as IM's coder reads it (the `image` crate stands in
/// for the formats without a port).
pub fn read_pixel_cache(path: &Path) -> Result<PixelCache> {
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
    match detect_format(path, &extension)? {
        Format::Png => read_png(path),
        Format::Jpeg => crate::coders::jpeg::read(path),
        Format::Gif => crate::coders::gif::read(path),
        Format::Bmp => crate::coders::bmp::read(path),
        Format::Tiff => crate::coders::tiff::read(path),
        Format::Exr => crate::coders::exr::read(path),
        Format::Webp => crate::coders::webp::read(path),
        Format::Jp2 => crate::coders::jp2::read(path),
        Format::Dds => crate::coders::dds::read(path),
        Format::Miff => crate::coders::miff::read(path),
        Format::Svg => crate::coders::svg::read(path),
        Format::Dng => crate::coders::dng::read(path),
        Format::Psd => crate::coders::psd::read(path),
        Format::Emf => crate::coders::emf::read(path),
        Format::Sgi => crate::coders::sgi::read(path),
        Format::Hdr => crate::coders::hdr::read(path),
        Format::Other => read_with_image_crate(path),
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
/// A tRNS chunk adds alpha.
fn read_png(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    read_png_bytes(&bytes)
}

/// The chunks of a PNG without its tRNS chunks, and the first tRNS's
/// data when it comes before the image data (libpng ignores later and
/// repeated ones), with whether a palette came before it.
fn split_transparency(bytes: &[u8]) -> (Vec<u8>, Option<(Vec<u8>, bool)>) {
    let mut kept = bytes[..bytes.len().min(8)].to_vec();
    let mut transparency = None;
    let mut seen_palette = false;
    let mut seen_data = false;
    let mut position = 8;
    while position + 12 <= bytes.len() {
        let length = u32::from_be_bytes([bytes[position], bytes[position + 1], bytes[position + 2], bytes[position + 3]]) as usize;
        let kind = &bytes[position + 4..position + 8];
        let end = position.saturating_add(12).saturating_add(length);
        if end > bytes.len() {
            break;
        }
        match kind {
            b"PLTE" => seen_palette = true,
            b"IDAT" => seen_data = true,
            _ => {}
        }
        if kind == b"tRNS" {
            if transparency.is_none() && !seen_data {
                let data = bytes[position + 8..position + 8 + length].to_vec();
                transparency = Some((data, seen_palette));
            }
        } else {
            kept.extend_from_slice(&bytes[position..end]);
        }
        position = end;
    }
    kept.extend_from_slice(&bytes[position.min(bytes.len())..]);
    (kept, transparency)
}

/// A PNG held in memory (one embedded in another file).
pub(crate) fn read_png_bytes(bytes: &[u8]) -> Result<PixelCache> {
    let (stripped, transparency_chunk) = split_transparency(bytes);
    let mut decoder = png::Decoder::new(std::io::Cursor::new(stripped));
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
    let palette: Vec<u8> = info.palette.as_ref().map(|palette| palette.to_vec()).unwrap_or_default();
    let width = info.width as usize;
    let height = info.height as usize;
    let bit_depth = info.bit_depth as u8;
    let color_type = info.color_type;
    let transparency = png_transparency(transparency_chunk, &info, palette.len() / 3);

    let mut buffer = vec![0u8; reader.output_buffer_size().context("PNG too large")?];
    let frame = reader.next_frame(&mut buffer)?;
    let line_size = frame.line_size;

    let gray = matches!(color_type, png::ColorType::Grayscale | png::ColorType::GrayscaleAlpha);
    let colorspace = if gray { gray_colorspace(gamma) } else { colour_colorspace(gamma) };
    let pseudo = color_type == png::ColorType::Indexed || (color_type == png::ColorType::Grayscale && bit_depth < 16);
    let file_alpha = matches!(color_type, png::ColorType::GrayscaleAlpha | png::ColorType::Rgba);
    let alpha = file_alpha || transparency != PngTransparency::None;
    let mut channels = if gray { vec![PixelChannel::Gray] } else { vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue] };
    if alpha {
        channels.push(PixelChannel::Alpha);
    }
    if pseudo {
        channels.push(PixelChannel::Index);
    }

    // PseudoClass images keep their colormap: the palette, or a ramp of
    // every grey the depth holds (`AcquireImageColormap`). A tRNS chunk
    // gives palette entries their alpha, or clears the grey that matches.
    let ramp_step = 65535.0 / ((1u32 << bit_depth.min(15)) - 1).max(1) as f64;
    let mut colormap: Vec<[f64; 4]> = Vec::new();
    if color_type == png::ColorType::Indexed {
        for entry in palette.as_chunks::<3>().0 {
            colormap.push([char_quantum(entry[0]) as f64, char_quantum(entry[1]) as f64, char_quantum(entry[2]) as f64, 65535.0]);
        }
        if let PngTransparency::Palette(alphas) = &transparency {
            for (entry, &value) in colormap.iter_mut().zip(alphas.iter()) {
                entry[3] = char_quantum(value) as f64;
            }
        }
    } else if pseudo {
        for index in 0..1usize << bit_depth {
            let level = index as f64 * ramp_step;
            colormap.push([level, level, level, 65535.0]);
        }
        if let PngTransparency::Colour([transparent_gray, _, _]) = transparency {
            for entry in colormap.iter_mut() {
                if scale_quantum_to_short(entry[0] as f32) == transparent_gray {
                    entry[3] = 0.0;
                }
            }
        }
    }

    let samples_per_pixel = color_type.samples();
    let mut data = Vec::with_capacity(width * height * channels.len());
    for y in 0..height {
        let row = &buffer[y * line_size..(y + 1) * line_size];
        let samples = unpack_row(row, bit_depth, width * samples_per_pixel);
        for pixel in samples.chunks_exact(samples_per_pixel) {
            if pseudo {
                // `ConstrainColormapIndex` refuses an index past the colormap.
                let index = pixel[0] as usize;
                let Some(entry) = colormap.get(index) else {
                    bail!("InvalidColormapIndex");
                };
                if gray {
                    data.push(entry[2] as f32);
                } else {
                    data.extend([entry[0] as f32, entry[1] as f32, entry[2] as f32]);
                }
                if alpha {
                    data.push(entry[3] as f32);
                }
                data.push(index as f32);
                continue;
            }
            let start = data.len();
            if bit_depth == 16 {
                data.extend(pixel.iter().map(|&value| value as f32));
            } else {
                data.extend(pixel.iter().map(|&value| char_quantum(value as u8)));
            }
            // A pixel of the tRNS colour is transparent, every other opaque.
            if let PngTransparency::Colour(colour) = transparency {
                let pixel_colour = if gray { [data[start]; 3] } else { [data[start], data[start + 1], data[start + 2]] };
                let matches = pixel_colour.iter().zip(colour.iter()).all(|(&value, &wanted)| scale_quantum_to_short(value) == wanted);
                data.push(if matches { 0.0 } else { 65535.0 });
            }
        }
    }

    // The type `ReadOnePNGImage` sets once the pixels are in.
    let image_type = match (color_type, alpha) {
        (png::ColorType::GrayscaleAlpha, _) => ImageType::GrayscaleAlpha,
        (png::ColorType::Indexed, true) => ImageType::PaletteAlpha,
        (png::ColorType::Indexed, false) => ImageType::Palette,
        (png::ColorType::Grayscale, false) => ImageType::Grayscale,
        (_, true) => ImageType::TrueColorAlpha,
        (_, false) => ImageType::TrueColor,
    };
    let mut cache = PixelCache::new(width, height, channels, data, colorspace, gamma);
    cache.image_type = image_type;
    cache.colormap = colormap;
    Ok(cache)
}

/// What a PNG's tRNS chunk gives, as IM takes it.
#[derive(Clone, Debug, PartialEq)]
enum PngTransparency {
    None,
    /// The first palette entries' alphas.
    Palette(Vec<u8>),
    /// The transparent grey or colour, scaled to 16 bits.
    Colour([u32; 3]),
}

/// The tRNS chunk as libpng accepts it (a grey's two bytes, a colour's
/// six, no more alphas than palette entries and after them, none with an
/// alpha channel) and IM keeps it: a grey or colour sample past the bit
/// depth's range drops it.
fn png_transparency(chunk: Option<(Vec<u8>, bool)>, info: &png::Info, palette_entries: usize) -> PngTransparency {
    let Some((bytes, after_palette)) = chunk else {
        return PngTransparency::None;
    };
    if info.color_type == png::ColorType::Indexed && !after_palette {
        return PngTransparency::None;
    }
    let depth = info.bit_depth as u32;
    let max_sample = (1u32 << depth) - 1;
    let scale_to_short = 65535 / max_sample;
    let sample = |offset: usize| u16::from_be_bytes([bytes[offset], bytes[offset + 1]]) as u32;
    match info.color_type {
        png::ColorType::Indexed if !bytes.is_empty() && bytes.len() <= palette_entries => PngTransparency::Palette(bytes.to_vec()),
        png::ColorType::Grayscale if bytes.len() == 2 => {
            let gray = sample(0);
            if gray > max_sample {
                return PngTransparency::None;
            }
            PngTransparency::Colour([gray * scale_to_short; 3])
        }
        png::ColorType::Rgb if bytes.len() == 6 => {
            let colour = [sample(0), sample(2), sample(4)];
            if colour.iter().any(|&value| value > max_sample) {
                return PngTransparency::None;
            }
            PngTransparency::Colour(colour.map(|value| value * scale_to_short))
        }
        _ => PngTransparency::None,
    }
}

/// `ScaleQuantumToShort` (Q16 HDRI).
fn scale_quantum_to_short(quantum: f32) -> u32 {
    if quantum <= 0.0 {
        return 0;
    }
    if quantum >= 65535.0 {
        return 65535;
    }
    (quantum + 0.5) as u32
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
    Ok(PixelCache::new(width, height, channels, data, colorspace, DEFAULT_GAMMA))
}
