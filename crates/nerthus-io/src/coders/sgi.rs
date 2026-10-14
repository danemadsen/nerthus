//! SGI (Iris `.rgb`) as ImageMagick reads it (`coders/sgi.c`): verbatim or
//! run-length rows, 8 or 16 bits a channel, stored bottom-up. Three or four
//! channels give RGB(A); one or two give a `PseudoClass` image of a grey
//! palette (256 entries, or 65535 for 16 bits) whose index is the first
//! channel — a second channel is dropped.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use crate::pixel_cache::{CacheColorspace, DEFAULT_GAMMA, PixelCache, PixelChannel, char_quantum};

/// `SGIDecode`: one run-length row into every fourth sample (or pair of
/// bytes) of `pixels` from `start`; false on a malformed row.
fn sgi_decode(bytes_per_pixel: usize, mut packets: isize, input: &[u8], mut pixels_left: isize, pixels: &mut [u8], start: usize) -> bool {
    let mut p = 0usize;
    let mut q = start;
    let byte = |index: usize| input.get(index).copied().unwrap_or(0);
    if bytes_per_pixel == 2 {
        while pixels_left > 0 {
            if packets == 0 {
                return false;
            }
            packets -= 1;
            let pixel = (byte(p) as usize) << 8 | byte(p + 1) as usize;
            p += 2;
            let mut count = (pixel & 0x7f) as isize;
            if count == 0 {
                break;
            }
            if count > pixels_left {
                return false;
            }
            pixels_left -= count;
            if pixel & 0x80 != 0 {
                while count != 0 {
                    if packets == 0 {
                        return false;
                    }
                    packets -= 1;
                    pixels[q] = byte(p);
                    pixels[q + 1] = byte(p + 1);
                    p += 2;
                    q += 8;
                    count -= 1;
                }
            } else {
                if packets == 0 {
                    return false;
                }
                packets -= 1;
                let value = (byte(p) as usize) << 8 | byte(p + 1) as usize;
                p += 2;
                while count != 0 {
                    pixels[q] = (value >> 8) as u8;
                    pixels[q + 1] = value as u8;
                    q += 8;
                    count -= 1;
                }
            }
        }
        return true;
    }
    while pixels_left > 0 {
        if packets == 0 {
            return false;
        }
        packets -= 1;
        let pixel = byte(p) as usize;
        p += 1;
        let mut count = (pixel & 0x7f) as isize;
        if count == 0 {
            break;
        }
        if count > pixels_left {
            return false;
        }
        pixels_left -= count;
        if pixel & 0x80 != 0 {
            while count != 0 {
                if packets == 0 {
                    return false;
                }
                packets -= 1;
                pixels[q] = byte(p);
                p += 1;
                q += 4;
                count -= 1;
            }
        } else {
            if packets == 0 {
                return false;
            }
            packets -= 1;
            let value = byte(p);
            p += 1;
            while count != 0 {
                pixels[q] = value;
                q += 4;
                count -= 1;
            }
        }
    }
    true
}

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut blob = Blob::new(&bytes);
    if blob.msb_u16() != 0x01DA {
        bail!("ImproperImageHeader");
    }
    let storage = blob.byte().unwrap_or(0xFF);
    if storage > 1 {
        bail!("ImproperImageHeader");
    }
    let bytes_per_pixel = blob.byte().unwrap_or(0xFF) as usize;
    if bytes_per_pixel == 0 || bytes_per_pixel > 2 {
        bail!("ImproperImageHeader");
    }
    let dimension = blob.msb_u16();
    if dimension == 0 || dimension > 3 {
        bail!("ImproperImageHeader");
    }
    let columns = blob.msb_u16() as usize;
    let rows = blob.msb_u16() as usize;
    let depth = blob.msb_u16() as usize;
    if depth == 0 || depth > 4 {
        bail!("ImproperImageHeader");
    }
    blob.msb_u32();
    blob.msb_u32();
    blob.msb_u32();
    if blob.read(80).len() != 80 {
        bail!("ImproperImageHeader");
    }
    if blob.msb_u32() != 0 {
        bail!("ImproperImageHeader");
    }
    if blob.read(404).len() != 404 {
        bail!("ImproperImageHeader");
    }
    let has_alpha = depth == 4;
    let pseudo = depth < 3;
    if (columns * rows / 255) as u64 > blob.size() {
        bail!("InsufficientImageDataInFile");
    }
    if columns == 0 || rows == 0 {
        bail!("NegativeOrZeroImageSize");
    }
    let stride = 4 * bytes_per_pixel;
    let mut pixels = vec![0u8; stride * columns * rows];
    if storage != 0x01 {
        'channels: for z in 0..depth {
            let mut p = bytes_per_pixel * z;
            for _ in 0..rows {
                let scanline = blob.read(bytes_per_pixel * columns);
                if scanline.len() != bytes_per_pixel * columns {
                    break 'channels;
                }
                for x in 0..columns {
                    if bytes_per_pixel == 2 {
                        pixels[p] = scanline[2 * x];
                        pixels[p + 1] = scanline[2 * x + 1];
                        p += 8;
                    } else {
                        pixels[p] = scanline[x];
                        p += 4;
                    }
                }
            }
        }
    } else {
        let count = rows * depth;
        let offsets: Vec<i64> = (0..count).map(|_| blob.msb_u32() as i32 as i64).collect();
        let mut runlength = Vec::with_capacity(count);
        for _ in 0..count {
            let length = blob.msb_u32() as usize;
            if length > 4 * columns + 10 {
                bail!("ImproperImageHeader");
            }
            runlength.push(length);
        }
        let mut data_order = 0;
        let mut offset: i64 = 0;
        'order: for y in 0..rows {
            for z in 0..depth {
                if offsets[y + z * rows] < offset {
                    data_order = 1;
                    break 'order;
                }
                offset = offsets[y + z * rows];
            }
        }
        let mut offset = blob.tell() as i64;
        let row_bytes = columns * stride;
        let read_row = |blob: &mut Blob, y: usize, z: usize, pixels: &mut Vec<u8>, offset: &mut i64| -> Result<bool> {
            let index = y + z * rows;
            if *offset != offsets[index] {
                *offset = blob.seek(offsets[index]).map_or(-1, |value| value as i64);
            }
            let packets = blob.read(runlength[index]).to_vec();
            if packets.len() != runlength[index] {
                return Ok(false);
            }
            *offset += runlength[index] as i64;
            let start = y * row_bytes + bytes_per_pixel * z;
            if !sgi_decode(bytes_per_pixel, (runlength[index] / bytes_per_pixel) as isize, &packets, columns as isize, pixels, start) {
                bail!("ImproperImageHeader");
            }
            Ok(true)
        };
        if data_order == 1 {
            'channels: for z in 0..depth {
                for y in 0..rows {
                    if !read_row(&mut blob, y, z, &mut pixels, &mut offset)? {
                        break 'channels;
                    }
                }
            }
        } else {
            let position = blob.tell() as i64;
            'rows: for y in 0..rows {
                for z in 0..depth {
                    if !read_row(&mut blob, y, z, &mut pixels, &mut offset)? {
                        break 'rows;
                    }
                }
            }
            blob.seek(position);
        }
    }
    let mut channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    if pseudo {
        channels.push(PixelChannel::Index);
    } else if has_alpha {
        channels.push(PixelChannel::Alpha);
    }
    let channel_count = channels.len();
    let mut data = vec![0f32; columns * rows * channel_count];
    let colors = if bytes_per_pixel > 1 { 65535usize } else { 256 };
    let step = (65535.0f32 / (colors - 1) as f32) as f64;
    for y in 0..rows {
        let row = &pixels[(rows - y - 1) * columns * stride..];
        for x in 0..columns {
            let p = &row[x * stride..];
            let sample = |index: usize| -> u32 {
                if bytes_per_pixel == 2 {
                    (p[2 * index] as u32) << 8 | p[2 * index + 1] as u32
                } else {
                    p[index] as u32
                }
            };
            let quantum = |index: usize| if bytes_per_pixel == 2 { sample(index) as f32 } else { char_quantum(sample(index) as u8) };
            let offset = (y * columns + x) * channel_count;
            if pseudo {
                // `SyncImage` takes an index past the palette's end as the
                // first entry (keeping the index).
                let index = sample(0) as usize;
                let entry = if index < colors { index } else { 0 };
                let value = (entry as f64 * step) as f32;
                data[offset] = value;
                data[offset + 1] = value;
                data[offset + 2] = value;
                data[offset + 3] = index as f32;
            } else {
                data[offset] = quantum(0);
                data[offset + 1] = quantum(1);
                data[offset + 2] = quantum(2);
                if has_alpha {
                    data[offset + 3] = quantum(3);
                }
            }
        }
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    let mut cache = PixelCache::new(columns, rows, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA);
    if pseudo {
        cache.colormap = (0..colors).map(|index| [index as f64 * step, index as f64 * step, index as f64 * step, 65535.0]).collect();
    }
    Ok(cache)
}
