//! BMP as ImageMagick reads it (`coders/bmp.c`): OS/2 and Windows headers
//! of every size, palettes of up to 256 colours (a `PseudoClass` image:
//! red, green, blue and the index), RLE4 and RLE8, 16- and 32-bit
//! bitfields (alpha only from a bitfield mask, or for a 32-bit BI_RGB file
//! with the plain 40-byte header whose fourth bytes are not all zero),
//! 24-bit, and 64-bit (alpha dropped). A header file size that is not the
//! file's size, a palette index past the colour count, or a run-length
//! stream that falls short is an error.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use crate::pixel_cache::{CacheColorspace, DEFAULT_GAMMA, PixelCache, PixelChannel, char_quantum};

const BI_RGB: u32 = 0;
const BI_RLE8: u32 = 1;
const BI_RLE4: u32 = 2;
const BI_BITFIELDS: u32 = 3;
const BI_JPEG: u32 = 4;
const BI_PNG: u32 = 5;
const BI_ALPHABITFIELDS: u32 = 6;

/// The header fields the reader keeps (`BMPInfo`).
#[derive(Default)]
struct Info {
    file_size: u32,
    offset_bits: u32,
    size: u32,
    width: i64,
    height: i64,
    planes: u16,
    bits_per_pixel: u32,
    compression: u32,
    number_colors: u32,
    red_mask: u32,
    green_mask: u32,
    blue_mask: u32,
    alpha_mask: u32,
    colorspace: i32,
    profile_data: i64,
    profile_size: i64,
}

/// `DecodeImage`: RLE4/RLE8 into one index byte per pixel, rows bottom-up;
/// false when the stream ends before every pixel was set.
fn decode_rle(blob: &mut Blob, compression: u32, pixels: &mut [u8], columns: usize, rows: usize) -> bool {
    pixels.fill(0);
    let end = pixels.len() as i64;
    let mut p: i64 = 0;
    let mut x: i64 = 0;
    let mut y: i64 = 0;
    while y < rows as i64 {
        if p < 0 || p >= end {
            break;
        }
        let Some(count) = blob.byte() else {
            break;
        };
        if count > 0 {
            let count = (count as i64).min(end - p);
            let Some(byte) = blob.byte() else {
                break;
            };
            for i in 0..count {
                pixels[p as usize] = if compression == BI_RLE8 {
                    byte
                } else if i & 1 != 0 {
                    byte & 0x0f
                } else {
                    (byte >> 4) & 0x0f
                };
                p += 1;
            }
            x += count;
            continue;
        }
        let Some(code) = blob.byte() else {
            break;
        };
        if code == 0x01 {
            break;
        }
        match code {
            0x00 => {
                x = 0;
                y += 1;
                p = y * columns as i64;
            }
            0x02 => {
                let Some(dx) = blob.byte() else {
                    return false;
                };
                x += dx as i64;
                let Some(dy) = blob.byte() else {
                    return false;
                };
                y += dy as i64;
                p = y * columns as i64 + x;
            }
            _ => {
                let count = (code as i64).min(end - p);
                if count < 0 {
                    continue;
                }
                let mut byte = 0u8;
                let mut stopped = false;
                for i in 0..count {
                    if compression == BI_RLE8 {
                        match blob.byte() {
                            Some(value) => byte = value,
                            None => {
                                stopped = true;
                                break;
                            }
                        }
                        pixels[p as usize] = byte;
                    } else {
                        if i & 1 == 0 {
                            match blob.byte() {
                                Some(value) => byte = value,
                                None => {
                                    stopped = true;
                                    break;
                                }
                            }
                        }
                        pixels[p as usize] = if i & 1 != 0 { byte & 0x0f } else { (byte >> 4) & 0x0f };
                    }
                    p += 1;
                }
                let _ = stopped;
                x += count;
                let pad = if compression == BI_RLE8 { count & 1 != 0 } else { (count & 3) == 1 || (count & 3) == 2 };
                if pad && blob.byte().is_none() {
                    break;
                }
            }
        }
    }
    blob.byte();
    blob.byte();
    p >= end
}

/// The shift that brings a mask's top bit to bit 31, and its width there.
fn mask_layout(mask: u32) -> (u32, u32) {
    let mut shift = 0u32;
    if mask != 0 {
        while (mask.wrapping_shl(shift) & 0x8000_0000) == 0 {
            shift += 1;
            if shift >= 32 {
                break;
            }
        }
    }
    let mut sample = shift;
    while (mask.wrapping_shl(sample) & 0x8000_0000) != 0 {
        sample += 1;
        if sample >= 32 {
            break;
        }
    }
    (shift, sample - shift)
}

/// `ScaleShortToQuantum`.
fn short_quantum(value: u32) -> f32 {
    (value & 0xFFFF) as f32
}

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    read_bytes(&bytes)
}

pub fn read_bytes(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let blob_size = blob.size();
    let mut magick = blob.read(2).to_vec();
    if magick.len() != 2 {
        bail!("ImproperImageHeader");
    }
    let mut info = Info::default();
    let start_position = blob.tell() as i64 - 2;
    let mut count_ok = true;
    while magick.eq_ignore_ascii_case(b"BA") {
        info.file_size = blob.lsb_u32();
        blob.lsb_u32();
        info.offset_bits = blob.lsb_u32();
        magick = blob.read(2).to_vec();
        if magick.len() != 2 {
            count_ok = false;
            break;
        }
    }
    if !count_ok || !(magick.eq_ignore_ascii_case(b"BM") || magick.eq_ignore_ascii_case(b"CI")) {
        bail!("ImproperImageHeader");
    }
    info.file_size = blob.lsb_u32();
    blob.lsb_u32();
    info.offset_bits = blob.lsb_u32();
    info.size = blob.lsb_u32();
    if magick.eq_ignore_ascii_case(b"CI") && info.size != 12 && info.size != 40 && info.size != 64 {
        bail!("ImproperImageHeader");
    }
    if info.size > 124 {
        bail!("ImproperImageHeader");
    }
    if info.file_size != 0 && info.file_size as u64 > blob_size {
        bail!("ImproperImageHeader");
    }
    if info.offset_bits < info.size {
        bail!("ImproperImageHeader");
    }
    info.offset_bits = info.offset_bits.max(14 + info.size);
    let mut gamma = DEFAULT_GAMMA;
    if info.size == 12 {
        info.width = blob.lsb_u16() as i16 as i64;
        info.height = blob.lsb_u16() as i16 as i64;
        info.planes = blob.lsb_u16();
        info.bits_per_pixel = blob.lsb_u16() as u32;
        info.compression = BI_RGB;
    } else {
        if !matches!(info.size, 40 | 52 | 56 | 64 | 78 | 108 | 124) && info.size < 64 {
            bail!("ImproperImageHeader");
        }
        info.width = blob.lsb_u32() as i32 as i64;
        info.height = blob.lsb_u32() as i32 as i64;
        info.planes = blob.lsb_u16();
        info.bits_per_pixel = blob.lsb_u16() as u32;
        info.compression = blob.lsb_u32();
        if info.size > 16 {
            blob.lsb_u32();
            blob.lsb_u32();
            blob.lsb_u32();
            info.number_colors = blob.lsb_u32();
            blob.lsb_u32();
        }
        if info.height < 0 && info.compression != BI_RGB && info.compression != BI_BITFIELDS {
            bail!("CompressNotSupported");
        }
        if info.size > 40 || info.compression == BI_BITFIELDS || info.compression == BI_ALPHABITFIELDS {
            info.red_mask = blob.lsb_u32();
            info.green_mask = blob.lsb_u32();
            info.blue_mask = blob.lsb_u32();
            if info.compression == BI_ALPHABITFIELDS {
                info.alpha_mask = blob.lsb_u32();
            }
            if (info.size == 40 || info.compression == BI_ALPHABITFIELDS) && info.bits_per_pixel != 16 && info.bits_per_pixel != 32 {
                bail!("UnsupportedBitsPerPixel");
            }
        }
        if info.size > 40 {
            info.alpha_mask = blob.lsb_u32();
            info.colorspace = blob.lsb_u32() as i32;
            for _ in 0..9 {
                blob.lsb_u32();
            }
            let scale_x = blob.lsb_u32() as f64 / 65536.0;
            let scale_y = blob.lsb_u32() as f64 / 65536.0;
            let scale_z = blob.lsb_u32() as f64 / 65536.0;
            if info.colorspace == 0 {
                gamma = (scale_x + scale_y + scale_z) / 3.0;
            }
        }
        if info.size > 108 {
            blob.lsb_u32();
            info.profile_data = blob.lsb_u32() as i64;
            info.profile_size = blob.lsb_u32() as i64;
            blob.lsb_u32();
        }
    }
    // A header file size that is not the file's is reported as an error.
    let size_mismatch = info.file_size as u64 != blob_size;
    if info.width <= 0 || info.height == 0 {
        bail!("NegativeOrZeroImageSize");
    }
    if info.compression == BI_JPEG || info.compression == BI_PNG {
        let rest = blob.read((blob_size - blob.tell()) as usize);
        let embedded = if info.compression == BI_JPEG {
            if rest.len() < 107 {
                bail!("InsufficientImageDataInFile");
            }
            let decoded = super::jpeg::decode(rest).map_err(|error| anyhow::anyhow!("{error}"))?;
            super::jpeg::to_pixel_cache(&decoded)
        } else {
            crate::pixel_cache::read_png_bytes(rest)?
        };
        if size_mismatch {
            bail!("LengthAndFilesizeDoNotMatch");
        }
        return Ok(embedded);
    }
    if info.planes != 1 {
        bail!("StaticPlanesValueNotEqualToOne");
    }
    if !matches!(info.bits_per_pixel, 1 | 4 | 8 | 16 | 24 | 32 | 64) {
        bail!("UnsupportedBitsPerPixel");
    }
    if info.bits_per_pixel < 16 && info.number_colors > (1u32 << info.bits_per_pixel) {
        bail!("UnrecognizedNumberOfColors");
    }
    if info.number_colors as u64 > blob_size {
        bail!("InsufficientImageDataInFile");
    }
    if (info.compression == BI_RLE8 && info.bits_per_pixel != 8) || (info.compression == BI_RLE4 && info.bits_per_pixel != 4) || (info.compression == BI_BITFIELDS && info.bits_per_pixel < 16) {
        bail!("UnsupportedBitsPerPixel");
    }
    if !matches!(info.compression, BI_RGB | BI_RLE8 | BI_RLE4 | BI_BITFIELDS | BI_ALPHABITFIELDS) {
        bail!("UnrecognizedImageCompression");
    }
    let columns = info.width.unsigned_abs() as usize;
    let rows = info.height.unsigned_abs() as usize;
    let mut has_alpha = info.alpha_mask != 0 && (info.compression == BI_BITFIELDS || info.compression == BI_ALPHABITFIELDS);
    let pseudo = info.bits_per_pixel < 16;
    let mut colormap: Vec<[f32; 3]> = Vec::new();
    if pseudo {
        let mut colors = info.number_colors as usize;
        if colors == 0 {
            colors = 1 << info.bits_per_pixel;
        }
        // `AcquireImageColormap` starts every entry as a grey ramp.
        let step = 65535.0f32 / (colors - 1).max(1) as f32;
        colormap = (0..colors).map(|i| [(i as f64 * step as f64) as f32; 3]).collect();
        let packet = if info.size == 12 || info.size == 64 { 3 } else { 4 };
        if blob.seek(start_position + 14 + info.size as i64).is_none() {
            bail!("ImproperImageHeader");
        }
        let entries = blob.read(packet * colors);
        if entries.len() != packet * colors {
            bail!("InsufficientImageDataInFile");
        }
        for (index, entry) in entries.chunks_exact(packet).enumerate() {
            colormap[index] = [char_quantum(entry[2]), char_quantum(entry[1]), char_quantum(entry[0])];
        }
    }
    if blob.seek(start_position + info.offset_bits as i64).is_none() {
        bail!("ImproperImageHeader");
    }
    // RLE4 decodes to a byte per pixel.
    if info.compression == BI_RLE4 {
        info.bits_per_pixel <<= 1;
    }
    let extent = columns as u64 * info.bits_per_pixel as u64;
    let mut bytes_per_line = 4 * extent.div_ceil(32) as usize;
    let length = bytes_per_line * rows;
    if (length / 256) as u64 > blob_size {
        bail!("InsufficientImageDataInFile");
    }
    let buffer_line = bytes_per_line.max(columns + 1);
    let mut pixels = vec![0u8; buffer_line * rows];
    if matches!(info.compression, BI_RGB | BI_BITFIELDS | BI_ALPHABITFIELDS) {
        let data = blob.read(length);
        if data.len() != length {
            bail!("InsufficientImageDataInFile");
        }
        pixels[..length].copy_from_slice(data);
    } else if !decode_rle(&mut blob, info.compression, &mut pixels[..columns * rows], columns, rows) {
        bail!("UnableToRunlengthDecodeImage");
    }
    if info.compression == BI_RGB {
        if !has_alpha && info.size == 40 && info.bits_per_pixel == 32 {
            let line = 4 * columns;
            has_alpha = (0..rows).any(|row| pixels[row * line..row * line + line].as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 0));
        }
        info.alpha_mask = if has_alpha { 0xff00_0000 } else { 0 };
        info.red_mask = 0x00ff_0000;
        info.green_mask = 0x0000_ff00;
        info.blue_mask = 0x0000_00ff;
        if info.bits_per_pixel == 16 {
            info.red_mask = 0x7c00;
            info.green_mask = 0x03e0;
            info.blue_mask = 0x001f;
        }
    }
    let (red_shift, red_bits) = mask_layout(info.red_mask);
    let (green_shift, green_bits) = mask_layout(info.green_mask);
    let (blue_shift, blue_bits) = mask_layout(info.blue_mask);
    let (alpha_shift, alpha_bits) = mask_layout(info.alpha_mask);

    // Rows as the image holds them: file rows run bottom-up, or top-down
    // when the height is negative.
    let image_row = |file_row: usize| if info.height < 0 { file_row } else { rows - 1 - file_row };
    let mut channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    if pseudo {
        channels.push(PixelChannel::Index);
    } else if has_alpha {
        channels.push(PixelChannel::Alpha);
    }
    let stride = channels.len();
    let mut data = vec![0f32; columns * rows * stride];
    let mut invalid_index = false;
    match info.bits_per_pixel {
        1 | 4 | 8 => {
            if info.compression == BI_RLE8 || info.compression == BI_RLE4 {
                bytes_per_line = columns;
            }
            let colors = colormap.len();
            for file_row in 0..rows {
                let line = &pixels[file_row * bytes_per_line..];
                let y = image_row(file_row);
                for x in 0..columns {
                    let raw = match info.bits_per_pixel {
                        1 => ((line[x / 8] >> (7 - (x % 8))) & 1) as usize,
                        4 => (if x % 2 == 0 { line[x / 2] >> 4 } else { line[x / 2] & 0x0f }) as usize,
                        _ => line[x] as usize,
                    };
                    // A 1-bit index is kept as it is, its colour falling back
                    // to the first entry; deeper ones must be in the palette.
                    let valid = raw < colors;
                    if !valid && info.bits_per_pixel != 1 {
                        invalid_index = true;
                    }
                    let kept = if valid || info.bits_per_pixel == 1 { raw } else { 0 };
                    let entry = colormap[if valid { raw } else { 0 }];
                    let offset = (y * columns + x) * stride;
                    data[offset..offset + 3].copy_from_slice(&entry);
                    data[offset + 3] = kept as f32;
                }
            }
        }
        16 => {
            if info.compression != BI_RGB && info.compression != BI_BITFIELDS {
                bail!("UnrecognizedImageCompression");
            }
            bytes_per_line = 2 * (columns + columns % 2);
            for file_row in 0..rows {
                let line = &pixels[file_row * bytes_per_line..];
                let y = image_row(file_row);
                for x in 0..columns {
                    let pixel = line[2 * x] as u32 | (line[2 * x + 1] as u32) << 8;
                    let mut red = ((pixel & info.red_mask).wrapping_shl(red_shift)) >> 16;
                    if red_bits == 5 {
                        red |= (red & 0xff00) >> 5;
                    }
                    if red_bits <= 8 {
                        red |= (red & 0xff00) >> 8;
                    }
                    let mut green = ((pixel & info.green_mask).wrapping_shl(green_shift)) >> 16;
                    if green_bits == 5 {
                        green |= (green & 0xff00) >> 5;
                    }
                    if green_bits == 6 {
                        green |= (green & 0xff00) >> 6;
                    }
                    if green_bits <= 8 {
                        green |= (green & 0xff00) >> 8;
                    }
                    let mut blue = ((pixel & info.blue_mask).wrapping_shl(blue_shift)) >> 16;
                    if blue_bits == 5 {
                        blue |= (blue & 0xff00) >> 5;
                    }
                    if blue_bits <= 8 {
                        blue |= (blue & 0xff00) >> 8;
                    }
                    let offset = (y * columns + x) * stride;
                    data[offset] = short_quantum(red);
                    data[offset + 1] = short_quantum(green);
                    data[offset + 2] = short_quantum(blue);
                    if has_alpha {
                        let mut alpha = ((pixel & info.alpha_mask).wrapping_shl(alpha_shift)) >> 16;
                        if alpha_bits <= 8 {
                            alpha |= (alpha & 0xff00) >> 8;
                        }
                        data[offset + 3] = short_quantum(alpha);
                    }
                }
            }
        }
        24 => {
            bytes_per_line = 4 * (columns * 24).div_ceil(32);
            for file_row in 0..rows {
                let line = &pixels[file_row * bytes_per_line..];
                let y = image_row(file_row);
                for x in 0..columns {
                    let offset = (y * columns + x) * stride;
                    data[offset] = char_quantum(line[3 * x + 2]);
                    data[offset + 1] = char_quantum(line[3 * x + 1]);
                    data[offset + 2] = char_quantum(line[3 * x]);
                    if has_alpha {
                        data[offset + 3] = 65535.0;
                    }
                }
            }
        }
        32 => {
            if info.compression != BI_RGB && info.compression != BI_BITFIELDS {
                bail!("UnrecognizedImageCompression");
            }
            bytes_per_line = 4 * columns;
            for file_row in 0..rows {
                let line = &pixels[file_row * bytes_per_line..];
                let y = image_row(file_row);
                for x in 0..columns {
                    let pixel = u32::from_le_bytes([line[4 * x], line[4 * x + 1], line[4 * x + 2], line[4 * x + 3]]);
                    let mut red = ((pixel & info.red_mask).wrapping_shl(red_shift)) >> 16;
                    if red_bits == 8 {
                        red |= red >> 8;
                    }
                    let mut green = ((pixel & info.green_mask).wrapping_shl(green_shift)) >> 16;
                    if green_bits == 8 {
                        green |= green >> 8;
                    }
                    let mut blue = ((pixel & info.blue_mask).wrapping_shl(blue_shift)) >> 16;
                    if blue_bits == 8 {
                        blue |= blue >> 8;
                    }
                    let offset = (y * columns + x) * stride;
                    data[offset] = short_quantum(red);
                    data[offset + 1] = short_quantum(green);
                    data[offset + 2] = short_quantum(blue);
                    if has_alpha {
                        let mut alpha = ((pixel & info.alpha_mask).wrapping_shl(alpha_shift)) >> 16;
                        if alpha_bits == 8 {
                            alpha |= alpha >> 8;
                        }
                        data[offset + 3] = short_quantum(alpha);
                    }
                }
            }
        }
        _ => {
            bytes_per_line = 4 * (columns * 64).div_ceil(32);
            for file_row in 0..rows {
                let line = &pixels[file_row * bytes_per_line..];
                let y = image_row(file_row);
                for x in 0..columns {
                    let sample = |index: usize| u16::from_le_bytes([line[8 * x + 2 * index], line[8 * x + 2 * index + 1]]) as f32;
                    let offset = (y * columns + x) * stride;
                    data[offset] = sample(2);
                    data[offset + 1] = sample(1);
                    data[offset + 2] = sample(0);
                    if has_alpha {
                        data[offset + 3] = sample(3);
                    }
                }
            }
        }
    }
    if invalid_index {
        bail!("InvalidColormapIndex");
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    if info.colorspace == 0x4D42_4544 && info.profile_data > 0 && info.profile_size > 0 {
        let offset = start_position + 14 + info.profile_data;
        if offset < blob.tell() as i64 || blob.seek(offset).is_none() || blob_size < (offset + info.profile_size) as u64 {
            bail!("ImproperImageHeader");
        }
    }
    if size_mismatch {
        bail!("LengthAndFilesizeDoNotMatch");
    }
    let mut cache = PixelCache::new(columns, rows, channels, data, CacheColorspace::SRgb, gamma);
    cache.colormap = colormap.iter().map(|entry| [entry[0] as f64, entry[1] as f64, entry[2] as f64, 65535.0]).collect();
    Ok(cache)
}
