//! TIFF as ImageMagick 7.1.2 reads it (`coders/tiff.c`) through libtiff
//! 4.7.1 as the Windows Magick.NET 14.12 builds it: the first directory only;
//! samples imported row by row from strips or tiles (`ImportQuantumPixels`)
//! or, for 8-bit-or-less images without RowsPerStrip, fax and LogLuv
//! files, through libtiff's 8-bit RGBA interface; palettes as PseudoClass;
//! grey, CMYK, CIELab (its a and b shifted) and uncompressed YCbCr kept in
//! their colorspaces; extra samples as alpha or meta channels.

mod directory;
mod fax;
mod jpeg;
mod luv;
mod lzw;
mod next;
mod ojpeg;
mod quantum;
mod read;
mod rgba;
mod thunder;
mod webp;

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::pixel_cache::{CacheColorspace, ImageType, PixelCache, PixelChannel, DEFAULT_GAMMA};
use directory::*;
use quantum::{Picture, QuantumInfo, QuantumType};
use read::Decoder;

/// `MagickMaxBufferExtent`: ImageMagick memory-maps larger files.
const MAGICK_MAX_BUFFER_EXTENT: usize = 524_288;
/// `MaxPixelChannels`.
const MAX_PIXEL_CHANNELS: u16 = 64;
/// `MaxPixelChannels - MetaPixelChannels`.
const MAX_META_CHANNELS: usize = 64 - 10;

/// How ImageMagick reads the pixels (`TIFFMethodType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Method {
    Ycck,
    Strip,
    Tile,
    Generic,
}

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

/// `GetJPEGMethod`: the strip method for 8-bit non-CMYK JPEG, the RGBA
/// interface for other depths, and for 8-bit CMYK the YCCK method when the
/// first strip's Adobe marker says the data is YCCK.
fn jpeg_method(tiff: &Tiff, photometric: u16) -> Method {
    let dir = &tiff.dir;
    if photometric != PHOTOMETRIC_SEPARATED {
        return if dir.bits_per_sample != 8 { Method::Generic } else { Method::Strip };
    }
    if dir.bits_per_sample != 8 || dir.samples_per_pixel != 4 {
        return Method::Strip;
    }
    let Some(&offset) = dir.strip_offsets.first() else {
        return Method::Strip;
    };
    const SIZE: usize = 2048;
    let start = offset as usize;
    if offset > tiff.data.len() as u64 || tiff.data.len() - start < SIZE {
        return Method::Strip;
    }
    let mut buffer = [0u8; SIZE + 32];
    buffer[..SIZE].copy_from_slice(&tiff.data[start..start + SIZE]);
    let mut i = 0usize;
    while i < SIZE {
        while i < SIZE {
            let byte = buffer[i];
            i += 1;
            if byte == 255 {
                break;
            }
        }
        while i < SIZE {
            i += 1;
            if buffer[i] != 255 {
                break;
            }
        }
        let marker = buffer[i];
        i += 1;
        if marker == 216 {
            i += 1;
            continue;
        }
        let length = ((buffer[i] as usize) << 8) | buffer[i + 1] as usize;
        if i + length >= SIZE {
            break;
        }
        if buffer[i - 1] == 238 {
            if length != 14 {
                break;
            }
            if buffer[i + 13] == 2 {
                return Method::Ycck;
            }
            break;
        }
        i += length;
        i += 1;
    }
    Method::Strip
}

/// `ClampYCC`.
fn clamp_ycc(value: f64) -> u8 {
    let value = 255.0 - value;
    if value < 0.0 {
        return 0;
    }
    if value > 255.0 {
        return 255;
    }
    value as u8
}

/// `ScaleCharToQuantum`.
fn char_quantum(value: u8) -> f32 {
    (257.0 * value as f64) as f32
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mapped = bytes.len() > MAGICK_MAX_BUFFER_EXTENT;
    let mut tiff = Tiff::open(bytes, mapped)?;
    let dir = tiff.dir.clone();
    let Some(photometric) = dir.photometric else {
        bail!("ImproperImageHeader");
    };
    let bits = dir.bits_per_sample;
    let samples = dir.samples_per_pixel;
    let sample_format = dir.sample_format;
    let compression = dir.compression;
    if !(sample_format == SAMPLEFORMAT_IEEEFP && bits == 64) && (bits == 0 || bits > 32) {
        bail!("UnsupportedBitsPerPixel");
    }
    if samples > MAX_PIXEL_CHANNELS {
        bail!("MaximumChannelsExceeded");
    }
    let columns = dir.width as usize;
    let rows = dir.length as usize;
    let jpeg_compression = compression == COMPRESSION_JPEG || compression == COMPRESSION_OJPEG;
    let colorspace = match photometric {
        PHOTOMETRIC_MINISBLACK | PHOTOMETRIC_MINISWHITE => CacheColorspace::Gray,
        PHOTOMETRIC_SEPARATED => CacheColorspace::Cmyk,
        PHOTOMETRIC_CIELAB => CacheColorspace::Lab,
        PHOTOMETRIC_YCBCR if !jpeg_compression => CacheColorspace::YCbCr,
        _ => CacheColorspace::SRgb,
    };
    if compression != COMPRESSION_NONE && !codec_configured(compression) {
        bail!("CompressNotSupported");
    }

    // A palette image is PseudoClass, its colormap from the file.
    let pseudo = photometric == PHOTOMETRIC_PALETTE && bits <= 16;
    let mut colormap = Vec::new();
    if pseudo {
        let colors = 1usize << bits;
        let step = (65535.0f32 / (colors - 1).max(1) as f32) as f64;
        colormap = (0..colors).map(|index| [index as f64 * step; 3]).collect();
        if let Some([red, green, blue]) = &dir.colormap {
            let wide = (0..colors).any(|index| red[index] >= 256 || green[index] >= 256 || blue[index] >= 256);
            let range = if wide { 65535.0 } else { 255.0 };
            for (index, entry) in colormap.iter_mut().enumerate() {
                let scale = |value: u16| ((65535.0 * value as f64) / range) as f32 as f64;
                *entry = [scale(red[index]), scale(green[index]), scale(blue[index])];
            }
        }
    }

    // Extra samples: the first alpha one is alpha, the rest meta channels.
    let mut alpha = false;
    let mut associated = false;
    let mut meta = 0usize;
    let extra = dir.extra_samples.clone();
    if extra.is_empty() {
        if samples == 4 && photometric == PHOTOMETRIC_RGB {
            alpha = true;
        }
    } else {
        for &info in &extra {
            if info == EXTRASAMPLE_ASSOCALPHA {
                alpha = true;
                associated = true;
                break;
            }
            if info == EXTRASAMPLE_UNASSALPHA {
                alpha = true;
                break;
            }
        }
        meta = extra.len() - usize::from(alpha);
        if meta >= MAX_META_CHANNELS {
            bail!("SetPixelMetaChannelsFailure");
        }
    }

    let mut method = Method::Generic;
    let mut rows_per_strip = rows;
    if dir.rows_per_strip_set {
        method = Method::Strip;
        rows_per_strip = (dir.rows_per_strip as usize).min(rows);
    } else if bits > 8 {
        method = Method::Strip;
    }
    if dir.tiled {
        method = Method::Tile;
    }
    if photometric == PHOTOMETRIC_LOGLUV || compression == COMPRESSION_CCITTFAX3 {
        method = Method::Generic;
    }
    if jpeg_compression {
        method = jpeg_method(&tiff, photometric);
    }

    let mut info = QuantumInfo::new(bits as u32, sample_format == SAMPLEFORMAT_IEEEFP);
    info.min_is_white = photometric == PHOTOMETRIC_MINISWHITE;
    info.associated_alpha = associated;
    let scanline_size = tiff.scanline_size()?;
    let depth_power = 2f64.powf(((bits as f64).ln() / 2f64.ln()).ceil());
    let by_depth = ((columns * samples as usize) as f64 * depth_power) as u64;
    let number_pixels = by_depth.max((columns * rows_per_strip) as u64);
    if scanline_size as f64 > 1.5 * number_pixels as f64 {
        bail!("CorruptImage");
    }

    let pseudo_kept = pseudo && method != Method::Generic;
    let gray = colorspace == CacheColorspace::Gray;
    let cmyk = colorspace == CacheColorspace::Cmyk;
    let mut picture = Picture::new(columns, rows, gray, cmyk, alpha, pseudo_kept, meta);
    picture.colormap = colormap;

    // The quantum type is picked while a palette image is still
    // PseudoClass (the generic method makes it DirectClass later).
    let mut quantum_type = if pseudo { QuantumType::Index } else { QuantumType::Gray };
    if meta != 0 {
        quantum_type = QuantumType::Multispectral;
    } else if dir.planar_config != PLANARCONFIG_SEPARATE {
        let mut pad = samples as isize;
        if samples > 2 {
            if cmyk {
                pad -= 4;
                quantum_type = QuantumType::Cmyk;
                if alpha {
                    pad -= 1;
                    quantum_type = QuantumType::Cmyka;
                }
            } else {
                pad -= 3;
                quantum_type = QuantumType::Rgb;
                if alpha {
                    pad -= 1;
                    quantum_type = QuantumType::Rgba;
                }
            }
        } else {
            pad -= 1;
            if alpha {
                if samples == 1 {
                    quantum_type = QuantumType::Alpha;
                } else {
                    pad -= 1;
                    quantum_type = if pseudo { QuantumType::IndexAlpha } else { QuantumType::GrayAlpha };
                }
            }
        }
        if pad < 0 {
            bail!("CorruptImage");
        }
        info.pad = pad as usize * ((bits as usize + 7) >> 3);
    }

    let mut decoder = Decoder::default();
    match method {
        Method::Ycck => read_ycck(&tiff, &mut decoder, &mut picture)?,
        Method::Strip => read_strips(&tiff, &mut decoder, &mut picture, &mut info, quantum_type, rows_per_strip)?,
        Method::Tile => read_tiles(&tiff, &mut decoder, &mut picture, &mut info, quantum_type)?,
        Method::Generic => read_generic(&mut tiff, &mut decoder, &mut picture)?,
    }
    if photometric == PHOTOMETRIC_CIELAB {
        decode_lab(&mut picture);
    }

    let mut channels = if gray { vec![PixelChannel::Gray] } else { vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue] };
    if cmyk {
        channels.push(PixelChannel::Black);
    }
    if alpha {
        channels.push(PixelChannel::Alpha);
    }
    if pseudo_kept {
        channels.push(PixelChannel::Index);
    }
    channels.extend(std::iter::repeat_n(PixelChannel::Meta, meta));

    // `SetQuantumImageType`, then `DecodeLabImage` writes pixels (which
    // leaves the type undefined) and the grey photometrics set their own.
    let mut image_type = match quantum_type {
        QuantumType::Index | QuantumType::IndexAlpha => ImageType::Palette,
        QuantumType::Gray | QuantumType::GrayAlpha if bits == 1 => ImageType::Bilevel,
        QuantumType::Gray | QuantumType::GrayAlpha => ImageType::Grayscale,
        QuantumType::Cmyk | QuantumType::Cmyka | QuantumType::Multispectral => ImageType::ColorSeparation,
        _ => ImageType::TrueColor,
    };
    if photometric == PHOTOMETRIC_CIELAB {
        image_type = ImageType::Undefined;
    }
    if matches!(photometric, PHOTOMETRIC_LOGL | PHOTOMETRIC_MINISBLACK | PHOTOMETRIC_MINISWHITE) {
        image_type = if bits == 1 { ImageType::Bilevel } else { ImageType::Grayscale };
    }
    let mut cache = PixelCache::new(columns, rows, channels, picture.data, colorspace, DEFAULT_GAMMA);
    cache.image_type = image_type;
    if pseudo_kept {
        cache.colormap = picture.colormap.iter().map(|entry| [entry[0], entry[1], entry[2], 65535.0]).collect();
    }
    Ok(cache)
}

/// The quantum type of sample `index` (the strip and tile loops' switch),
/// pointing multispectral planes at their meta channel.
fn sample_type(base: QuantumType, index: usize, separate: bool, cmyk: bool, info: &mut QuantumInfo, meta: usize) -> QuantumType {
    match index {
        0 if separate => QuantumType::Red,
        0 => base,
        1 => QuantumType::Green,
        2 => QuantumType::Blue,
        3 if cmyk => QuantumType::Black,
        3 => QuantumType::Alpha,
        4 if cmyk => QuantumType::Alpha,
        _ => {
            if base == QuantumType::Multispectral {
                // `SetQuantumMetaChannel` ignores a channel out of range.
                let channel = if cmyk { index as isize - 5 } else { index as isize - 4 };
                if channel >= -1 && channel < meta as isize {
                    info.meta_channel = (channel + 1) as usize;
                }
            }
            base
        }
    }
}

/// `ReadStripMethod`.
fn read_strips(tiff: &Tiff, decoder: &mut Decoder, picture: &mut Picture, info: &mut QuantumInfo, base: QuantumType, rows_per_strip: usize) -> Result<()> {
    let dir = &tiff.dir;
    let columns = picture.width;
    let rows = picture.height;
    let strip_size = tiff.strip_size()? as usize;
    let stride = tiff.vstrip_size(1)? as usize;
    let length = info.extent(picture, base);
    let extent = strip_size.max(rows_per_strip * stride.max(length));
    let mut strip_pixels = vec![0u8; extent];
    let separate = dir.planar_config == PLANARCONFIG_SEPARATE;
    let samples = dir.samples_per_pixel as usize;
    let mut strip_id = 0u32;
    for index in 0..samples {
        let kind = sample_type(base, index, separate, picture.cmyk, info, picture.meta);
        let mut rows_remaining = 0usize;
        let mut position = 0usize;
        let mut failed = false;
        for y in 0..rows {
            if rows_remaining == 0 {
                let read = tiff.read_encoded_strip(decoder, strip_id, &mut strip_pixels, Some(strip_size as u64))?;
                if !read.is_read() {
                    failed = true;
                    break;
                }
                rows_remaining = rows_per_strip;
                position = 0;
                strip_id += 1;
            }
            let row = strip_pixels.get(position..).unwrap_or(&[]);
            quantum::import(picture, info, kind, y * columns, columns, row)?;
            position += stride;
            rows_remaining -= 1;
        }
        if failed || (samples > 1 && !separate) {
            break;
        }
    }
    info.meta_channel = 0;
    Ok(())
}

/// `ReadTileMethod`.
fn read_tiles(tiff: &Tiff, decoder: &mut Decoder, picture: &mut Picture, info: &mut QuantumInfo, base: QuantumType) -> Result<()> {
    let dir = &tiff.dir;
    let columns = picture.width;
    let rows = picture.height;
    let tile_width = dir.tile_width as usize;
    let tile_length = dir.tile_length as usize;
    let tile_size = tiff.tile_size()? as usize;
    let stride = tiff.tile_row_size()? as usize;
    let length = info.extent(picture, base);
    let extent = tile_size.max(tile_length * stride.max(length));
    let mut tile_pixels = vec![0u8; extent];
    let separate = dir.planar_config == PLANARCONFIG_SEPARATE;
    let samples = dir.samples_per_pixel as usize;
    if tile_width == 0 || tile_length == 0 {
        bail!("ImageIsNotTiled");
    }
    for index in 0..samples {
        let kind = sample_type(base, index, separate, picture.cmyk, info, picture.meta);
        let mut failed = false;
        'rows: for y in (0..rows).step_by(tile_length) {
            let rows_remaining = if y + tile_length < rows { tile_length } else { rows - y };
            for x in (0..columns).step_by(tile_width) {
                let columns_remaining = if x + tile_width < columns { tile_width } else { columns - x };
                let read = tiff.read_tile(decoder, x as u32, y as u32, index as u16, &mut tile_pixels)?;
                if !read.is_read() {
                    failed = true;
                    break 'rows;
                }
                let mut position = 0usize;
                for row in 0..rows_remaining {
                    let line = tile_pixels.get(position..).unwrap_or(&[]);
                    quantum::import(picture, info, kind, (y + row) * columns + x, columns_remaining, line)?;
                    position += stride;
                }
            }
        }
        if failed || (samples > 1 && !separate) {
            break;
        }
    }
    info.meta_channel = 0;
    Ok(())
}

/// `ReadYCCKMethod`: JPEG-compressed CMYK whose samples are YCCK,
/// converted with ImageMagick's own coefficients (inverted).
fn read_ycck(tiff: &Tiff, decoder: &mut Decoder, picture: &mut Picture) -> Result<()> {
    let columns = picture.width;
    for y in 0..picture.height {
        let Some(line) = tiff.read_scanline(decoder, y as u32, 0)? else {
            bail!("UnableToReadImageData");
        };
        for x in 0..columns {
            let p = |index: usize| line.get(4 * x + index).copied().unwrap_or(0) as f64;
            let cyan = clamp_ycc(p(0) + (1.402 * p(2)) - 179.456);
            let magenta = clamp_ycc(p(0) - (0.34414 * p(1)) - (0.71414 * p(2)) + 135.45984);
            let yellow = clamp_ycc(p(0) + (1.772 * p(1)) - 226.816);
            let black = p(3) as u8;
            let base = (y * columns + x) * picture.stride;
            picture.data[base + picture.red] = char_quantum(cyan);
            picture.data[base + picture.green] = char_quantum(magenta);
            picture.data[base + picture.blue] = char_quantum(yellow);
            if let Some(offset) = picture.black {
                picture.data[base + offset] = char_quantum(black);
            }
        }
    }
    Ok(())
}

/// `ReadGenericMethod`: libtiff's RGBA raster (bottom-up), each 8-bit
/// sample scaled to a quantum.
fn read_generic(tiff: &mut Tiff, decoder: &mut Decoder, picture: &mut Picture) -> Result<()> {
    let columns = picture.width;
    let rows = picture.height;
    let mut raster = vec![0u32; columns * rows];
    rgba::read_rgba_image(tiff, decoder, columns as u32, rows as u32, &mut raster)?;
    for y in 0..rows {
        let source_row = rows - 1 - y;
        for x in 0..columns {
            let abgr = raster[source_row * columns + x];
            let base = (y * columns + x) * picture.stride;
            let red = char_quantum((abgr & 0xFF) as u8);
            let green = char_quantum(((abgr >> 8) & 0xFF) as u8);
            let blue = char_quantum(((abgr >> 16) & 0xFF) as u8);
            picture.data[base + picture.red] = red;
            picture.data[base + picture.green] = green;
            picture.data[base + picture.blue] = blue;
            if let Some(offset) = picture.alpha_offset {
                picture.data[base + offset] = char_quantum((abgr >> 24) as u8);
            }
        }
    }
    Ok(())
}

/// `DecodeLabImage`: a and b moved from signed to ImageMagick's offset
/// encoding.
fn decode_lab(picture: &mut Picture) {
    const QUANTUM_SCALE: f64 = 1.0 / 65535.0;
    let (green, blue) = (picture.green, picture.blue);
    for pixel in picture.data.chunks_exact_mut(picture.stride) {
        let mut a = QUANTUM_SCALE * pixel[green] as f64 + 0.5;
        if a > 1.0 {
            a -= 1.0;
        }
        let mut b = QUANTUM_SCALE * pixel[blue] as f64 + 0.5;
        if b > 1.0 {
            b -= 1.0;
        }
        pixel[green] = (65535.0 * a) as f32;
        pixel[blue] = (65535.0 * b) as f32;
    }
}
