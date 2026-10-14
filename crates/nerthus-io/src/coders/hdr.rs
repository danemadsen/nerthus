//! Radiance RGBE (`.hdr`) as ImageMagick reads it (`coders/hdr.c`): the
//! header's keywords up to the `-Y height +X width` line, then rows that
//! are run-length encoded per channel (or flat RGBE once a row lacks the
//! encoding's marker). Each pixel is `QuantumRange · m · 2^(e−136)`,
//! unclamped, so values above 1 survive. The image is linear RGB, or XYZ
//! for `32-bit_rle_xyze`, with gamma 1.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel};

fn is_graph(c: i32) -> bool {
    (0x21..=0x7e).contains(&c)
}

fn is_alnum(c: i32) -> bool {
    (0..=0x7f).contains(&c) && (c as u8).is_ascii_alphanumeric()
}

fn is_space(c: i32) -> bool {
    matches!(c, 0x20 | 0x09..=0x0d)
}

/// C `sscanf(value, "%d +X %d")`: the height and width.
fn scan_size(value: &str) -> Option<(i64, i64)> {
    let mut rest = value;
    let (height, after) = scan_int(rest)?;
    rest = after.trim_start();
    rest = rest.strip_prefix('+')?;
    rest = rest.strip_prefix('X')?;
    let (width, _) = scan_int(rest)?;
    Some((height, width))
}

/// C `%d`: optional white space, sign and digits; what follows.
fn scan_int(text: &str) -> Option<(i64, &str)> {
    let trimmed = text.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let bytes = trimmed.as_bytes();
    let mut end = 0;
    if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
        end += 1;
    }
    let digits_start = end;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end == digits_start {
        return None;
    }
    let value = trimmed[..end].parse::<i64>().ok()?.clamp(i32::MIN as i64, i32::MAX as i64);
    Some((value, &trimmed[end..]))
}

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut blob = Blob::new(&bytes);
    let next = |blob: &mut Blob| blob.byte().map_or(-1, |byte| byte as i32);
    let mut columns: usize = 0;
    let mut rows: usize = 0;
    let mut format = String::new();
    let mut c = next(&mut blob);
    if c == -1 {
        bail!("UnableToReadImageData");
    }
    while is_graph(c) && columns == 0 && rows == 0 {
        if c == b'#' as i32 {
            loop {
                c = next(&mut blob);
                if c == -1 || c == b'\n' as i32 {
                    break;
                }
            }
            c = next(&mut blob);
        } else if !is_alnum(c) {
            c = next(&mut blob);
        } else {
            let mut keyword = String::new();
            loop {
                if keyword.len() < 4095 {
                    keyword.push(c as u8 as char);
                }
                c = next(&mut blob);
                if !(is_alnum(c) || c == b'_' as i32) {
                    break;
                }
            }
            let mut value_expected = false;
            while is_space(c) || c == b'=' as i32 {
                if c == b'=' as i32 {
                    value_expected = true;
                }
                c = next(&mut blob);
            }
            if keyword.eq_ignore_ascii_case("Y") {
                value_expected = true;
            }
            if !value_expected {
                continue;
            }
            let mut value = Vec::new();
            while c != b'\n' as i32 && c != 0 && c != -1 {
                if value.len() < 4095 {
                    value.push(c as u8);
                }
                c = next(&mut blob);
            }
            let value = String::from_utf8_lossy(&value).to_string();
            if keyword.eq_ignore_ascii_case("format") {
                format = value;
            } else if keyword == "Y"
                && let Some((height, width)) = scan_size(&value)
            {
                columns = width as u64 as usize;
                rows = height as u64 as usize;
            }
        }
        if columns == 0 && rows == 0 {
            while is_space(c) {
                c = next(&mut blob);
            }
        }
    }
    if columns == 0 || rows == 0 || columns > i32::MAX as usize || rows > i32::MAX as usize {
        bail!("NegativeOrZeroImageSize");
    }
    let colorspace = if format.eq_ignore_ascii_case("32-bit_rle_rgbe") {
        CacheColorspace::Rgb
    } else if format.eq_ignore_ascii_case("32-bit_rle_xyze") {
        CacheColorspace::Xyz
    } else {
        bail!("ImproperImageHeader");
    };
    let mut run_length = (8..=0x7ffff).contains(&columns);
    let mut pixels = vec![0u8; 4 * columns];
    let mut data = vec![0f32; columns * rows * 3];
    for y in 0..rows {
        if !run_length {
            let row = blob.read(4 * columns);
            if row.len() != 4 * columns {
                break;
            }
            pixels.copy_from_slice(row);
        } else {
            let start = blob.read(4);
            if start.len() != 4 {
                break;
            }
            let start = [start[0], start[1], start[2], start[3]];
            if ((start[2] as usize) << 8 | start[3] as usize) != columns {
                pixels[..4].copy_from_slice(&start);
                let rest = blob.read(4 * (columns - 1));
                pixels[4..4 + rest.len()].copy_from_slice(rest);
                run_length = false;
            } else {
                // The pair shares the row start's buffer, so a short read
                // keeps its second byte.
                let mut pair = [start[0], start[1]];
                let mut p = 0usize;
                for channel in 0..4 {
                    let end = (channel + 1) * columns;
                    while p < end {
                        let read = blob.read(2);
                        if read.is_empty() {
                            break;
                        }
                        pair[..read.len()].copy_from_slice(read);
                        if pair[0] > 128 {
                            let count = pair[0] as usize - 128;
                            if count == 0 || count > end - p {
                                break;
                            }
                            pixels[p..p + count].fill(pair[1]);
                            p += count;
                        } else {
                            let count = pair[0] as usize;
                            if count == 0 || count > end - p {
                                break;
                            }
                            pixels[p] = pair[1];
                            p += 1;
                            if count > 1 {
                                let literal = blob.read(count - 1);
                                if literal.is_empty() {
                                    break;
                                }
                                pixels[p..p + literal.len()].copy_from_slice(literal);
                                p += literal.len();
                            }
                        }
                    }
                }
            }
        }
        for x in 0..columns {
            let pixel = if run_length { [pixels[x], pixels[x + columns], pixels[x + 2 * columns], pixels[x + 3 * columns]] } else { [pixels[4 * x], pixels[4 * x + 1], pixels[4 * x + 2], pixels[4 * x + 3]] };
            let offset = (y * columns + x) * 3;
            if pixel[3] != 0 {
                let scale = 2.0f64.powf(pixel[3] as f64 - (128.0 + 8.0));
                for channel in 0..3 {
                    data[offset + channel] = (65535.0 * scale * pixel[channel] as f64) as f32;
                }
            }
        }
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    Ok(PixelCache::new(columns, rows, channels, data, colorspace, 1.0))
}
