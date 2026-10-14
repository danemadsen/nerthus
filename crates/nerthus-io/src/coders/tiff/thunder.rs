//! libtiff's ThunderScan 4-bit codec (`tif_thunder.c`): bytes of a 2-bit
//! code and 6 bits of data — runs of the last pixel, two- or three-bit
//! deltas from it, or a raw pixel.

use anyhow::{bail, Result};

use super::directory::Tiff;

const THUNDER_RUN: u8 = 0x00;
const THUNDER_2BITDELTAS: u8 = 0x40;
const THUNDER_3BITDELTAS: u8 = 0x80;
const DELTA2_SKIP: usize = 2;
const DELTA3_SKIP: usize = 4;
const TWO_BIT_DELTAS: [i32; 4] = [0, 1, 0, -1];
const THREE_BIT_DELTAS: [i32; 8] = [0, 1, 2, 3, 0, -3, -2, -1];

/// One row's pixels (`ThunderDecode`).
fn decode_row(raw: &[u8], input: &mut usize, row: &mut [u8], max_pixels: usize) -> Result<()> {
    let mut last: u32 = 0;
    let mut pixels = 0usize;
    let mut out = 0usize;
    let set_pixel = |row: &mut [u8], value: u32, pixels: &mut usize, out: &mut usize, last: &mut u32| {
        *last = value & 0xf;
        if *pixels < max_pixels {
            if *pixels & 1 == 1 {
                row[*out] |= *last as u8;
                *out += 1;
            } else {
                row[*out] = (*last << 4) as u8;
            }
            *pixels += 1;
        }
    };
    while *input < raw.len() && pixels < max_pixels {
        let n = raw[*input];
        *input += 1;
        match n & 0xc0 {
            THUNDER_RUN => {
                let mut count = (n & 0x3f) as isize;
                if count == 0 {
                    continue;
                }
                if pixels & 1 == 1 {
                    row[out] |= last as u8;
                    last = row[out] as u32;
                    out += 1;
                    pixels += 1;
                    count -= 1;
                } else {
                    last |= last << 4;
                }
                pixels += count.max(0) as usize;
                if pixels > max_pixels {
                    continue;
                }
                while count > 0 {
                    row[out] = last as u8;
                    out += 1;
                    count -= 2;
                }
                if count == -1 {
                    out -= 1;
                    row[out] &= 0xf0;
                }
                last &= 0xf;
            }
            THUNDER_2BITDELTAS => {
                for delta in [((n >> 4) & 3) as usize, ((n >> 2) & 3) as usize, (n & 3) as usize] {
                    if delta != DELTA2_SKIP {
                        let value = (last as i32 + TWO_BIT_DELTAS[delta]) as u32;
                        set_pixel(row, value, &mut pixels, &mut out, &mut last);
                    }
                }
            }
            THUNDER_3BITDELTAS => {
                for delta in [((n >> 3) & 7) as usize, (n & 7) as usize] {
                    if delta != DELTA3_SKIP {
                        let value = (last as i32 + THREE_BIT_DELTAS[delta]) as u32;
                        set_pixel(row, value, &mut pixels, &mut out, &mut last);
                    }
                }
            }
            _ => set_pixel(row, n as u32, &mut pixels, &mut out, &mut last),
        }
    }
    if pixels != max_pixels {
        let end = max_pixels.div_ceil(2);
        if out < end {
            row[out..end].fill(0);
        }
        let what = if pixels < max_pixels { "Not enough" } else { "Too much" };
        bail!("{what} data at scanline ({pixels} != {max_pixels})");
    }
    Ok(())
}

/// `ThunderDecodeRow` (strips only: the codec has no tile decoder).
pub(super) fn decode(tiff: &Tiff, raw: &[u8], output: &mut [u8], is_tile: bool) -> Result<()> {
    if is_tile {
        bail!("Compression scheme ThunderScan tile decoding is not implemented");
    }
    let scanline = tiff.scanline_size()? as usize;
    if !output.len().is_multiple_of(scanline) {
        bail!("Fractional scanlines cannot be read");
    }
    let width = tiff.dir.width as usize;
    let mut input = 0usize;
    for row in output.chunks_mut(scanline) {
        decode_row(raw, &mut input, row, width)?;
    }
    Ok(())
}
