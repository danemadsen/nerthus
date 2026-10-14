//! libtiff's NeXT 2-bit codec (`tif_next.c`): each row a literal row, a
//! literal span over a white row, or runs of (grey, count) bytes.

use anyhow::{bail, Result};

use super::directory::Tiff;

const LITERAL_ROW: u8 = 0x00;
const LITERAL_SPAN: u8 = 0x40;

/// `NeXTDecode`: rows start white; the data running out early leaves the
/// rest so.
pub(super) fn decode(tiff: &Tiff, raw: &[u8], output: &mut [u8], is_tile: bool) -> Result<()> {
    if tiff.dir.bits_per_sample != 2 {
        bail!("Unsupported BitsPerSample = {}", tiff.dir.bits_per_sample);
    }
    output.fill(0xff);
    // `tif_scanlinesize` is the image's, tiled or not.
    let scanline = tiff.scanline_size()? as usize;
    if !output.len().is_multiple_of(scanline) {
        bail!("Fractional scanlines cannot be read");
    }
    let width = if is_tile { tiff.dir.tile_width } else { tiff.dir.width } as usize;
    let mut input = 0usize;
    let mut left = raw.len();
    let mut row_start = 0usize;
    while left > 0 && row_start < output.len() {
        let row = &mut output[row_start..row_start + scanline];
        let mut n = raw[input] as usize;
        input += 1;
        left -= 1;
        match n as u8 {
            LITERAL_ROW => {
                if left < scanline {
                    bail!("Not enough data for scanline");
                }
                row.copy_from_slice(&raw[input..input + scanline]);
                input += scanline;
                left -= scanline;
            }
            LITERAL_SPAN => {
                if left < 4 {
                    bail!("Not enough data for scanline");
                }
                let offset = raw[input] as usize * 256 + raw[input + 1] as usize;
                let count = raw[input + 2] as usize * 256 + raw[input + 3] as usize;
                if left < 4 + count || offset + count > scanline {
                    bail!("Not enough data for scanline");
                }
                row[offset..offset + count].copy_from_slice(&raw[input + 4..input + 4 + count]);
                input += 4 + count;
                left -= 4 + count;
            }
            _ => {
                let mut pixels = 0usize;
                let mut out = 0usize;
                loop {
                    let grey = ((n >> 6) & 0x3) as u8;
                    let mut count = n & 0x3f;
                    while count > 0 && pixels < width && out < scanline {
                        count -= 1;
                        match pixels & 3 {
                            0 => row[out] = grey << 6,
                            1 => row[out] |= grey << 4,
                            2 => row[out] |= grey << 2,
                            _ => {
                                row[out] |= grey;
                                out += 1;
                            }
                        }
                        pixels += 1;
                    }
                    if pixels >= width {
                        break;
                    }
                    if out >= scanline {
                        bail!("Invalid data for scanline");
                    }
                    if left == 0 {
                        bail!("Not enough data for scanline");
                    }
                    n = raw[input] as usize;
                    input += 1;
                    left -= 1;
                }
            }
        }
        row_start += scanline;
    }
    Ok(())
}
