//! DNG (`coders/dng.c`): Magick.Native reads DNG files with LibRaw 0.22.0
//! (built without the DNG SDK, RawSpeed or OpenMP): `libraw_open_file`,
//! `libraw_unpack`, then with IM's settings (`user_flip` 0, 16-bit output,
//! the camera's white balance) `libraw_dcraw_process` and
//! `libraw_dcraw_make_mem_image`. This ports what DNG files go through:
//!
//! - [identify]: the TIFF/DNG tags, the raw IFD, its colour matrix, white
//!   balance and black and white levels;
//! - [unpack]: lossless JPEG, uncompressed and floating-point tiles or
//!   strips;
//! - [process]: black subtraction, white balance, demosaicing (AHD;
//!   [demosaic]: X-Trans, VNG), camera to sRGB, and the BT.709 gamma curve
//!   with LibRaw's automatic brightness.
//!
//! IM copies each 16-bit sample with `ScaleShortToQuantum`; fewer than
//! three colours make a GRAY image. Nerthus cannot read lossy
//! (JPEG-compressed) DNG files, as Magick.Native builds LibRaw with
//! `NO_JPEG`. Fuji SuperCCD layouts are not ported.

mod demosaic;
mod identify;
mod ljpeg;
mod process;
mod stream;
mod unpack;

use std::path::Path;

use anyhow::{Context, Result};

use crate::pixel_cache::{CacheColorspace, DEFAULT_GAMMA, PixelCache, PixelChannel};

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    let output = decode(&bytes).with_context(|| format!("cannot read {}", path.display()))?;
    let (channels, colorspace) = if output.colors < 3 {
        (vec![PixelChannel::Gray], CacheColorspace::Gray)
    } else {
        (vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue], CacheColorspace::SRgb)
    };
    let data = output.samples.iter().map(|&sample| sample as f32).collect();
    Ok(PixelCache::new(output.width, output.height, channels, data, colorspace, DEFAULT_GAMMA))
}

/// The 16-bit image LibRaw makes of a DNG file.
fn decode(bytes: &[u8]) -> Result<process::Output> {
    let mut state = identify::identify(bytes)?;
    let raw = unpack::unpack(bytes, &mut state)?;
    process::process(&mut state, raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: u32 = 40;
    const HEIGHT: u32 = 32;

    /// A 40×32 16-bit RGGB DNG: one uncompressed strip, black and white
    /// levels, ColorMatrix1 and AsShotNeutral.
    fn synthetic_dng() -> Vec<u8> {
        let matrix: [i32; 9] = [7234, -1413, -600, -3631, 11150, 2850, -382, 1335, 6437];
        let neutral: [u32; 3] = [6063, 10000, 4619];
        let mut matrix_bytes = Vec::new();
        for numerator in matrix {
            matrix_bytes.extend(numerator.to_le_bytes());
            matrix_bytes.extend(10000i32.to_le_bytes());
        }
        let mut neutral_bytes = Vec::new();
        for numerator in neutral {
            neutral_bytes.extend(numerator.to_le_bytes());
            neutral_bytes.extend(10000u32.to_le_bytes());
        }

        const ENTRIES: u32 = 17;
        let matrix_offset = 8 + 2 + ENTRIES * 12 + 4;
        let neutral_offset = matrix_offset + matrix_bytes.len() as u32;
        let pixels_offset = neutral_offset + neutral_bytes.len() as u32;
        // (tag, type, count, value or offset), in tag order.
        let entries: [(u16, u16, u32, u32); ENTRIES as usize] = [
            (254, 4, 1, 0),
            (256, 4, 1, WIDTH),
            (257, 4, 1, HEIGHT),
            (258, 3, 1, 16),
            (259, 3, 1, 1),
            (262, 3, 1, 32803),
            (273, 4, 1, pixels_offset),
            (277, 3, 1, 1),
            (278, 4, 1, HEIGHT),
            (279, 4, 1, WIDTH * HEIGHT * 2),
            (33421, 3, 2, 2 | 2 << 16),
            (33422, 1, 4, u32::from_le_bytes([0, 1, 1, 2])),
            (50706, 1, 4, u32::from_le_bytes([1, 4, 0, 0])),
            (50714, 3, 1, 64),
            (50717, 3, 1, 4095),
            (50721, 10, 9, matrix_offset),
            (50728, 5, 3, neutral_offset),
        ];

        let mut bytes = b"II*\0".to_vec();
        bytes.extend(8u32.to_le_bytes());
        bytes.extend((ENTRIES as u16).to_le_bytes());
        for (tag, kind, count, value) in entries {
            bytes.extend(tag.to_le_bytes());
            bytes.extend(kind.to_le_bytes());
            bytes.extend(count.to_le_bytes());
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(matrix_bytes);
        bytes.extend(neutral_bytes);
        assert_eq!(bytes.len() as u32, pixels_offset);
        for row in 0..HEIGHT {
            for column in 0..WIDTH {
                let value = 64 + (column * 97 + row * 61 + (column * row) % 13 * 50) % 4000;
                bytes.extend((value as u16).to_le_bytes());
            }
        }
        bytes
    }

    /// FNV-1a over the samples' little-endian bytes.
    fn fingerprint(samples: &[u16]) -> u64 {
        let mut hash = 0xcbf29ce484222325u64;
        for sample in samples {
            for byte in sample.to_le_bytes() {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(0x100000001b3);
            }
        }
        hash
    }

    /// The image Magick.Native's LibRaw makes of the synthetic DNG
    /// (tools/re/libraw-probe).
    #[test]
    fn decodes_like_libraw() {
        let output = decode(&synthetic_dng()).unwrap();
        assert_eq!((output.width, output.height, output.colors), (WIDTH as usize, HEIGHT as usize, 3));
        assert_eq!(fingerprint(&output.samples), EXPECTED_FINGERPRINT);
    }

    const EXPECTED_FINGERPRINT: u64 = 0x25bc1584828db967;

    /// Writes the synthetic DNG for the probe: `DNG_OUT=… cargo test
    /// -p nerthus-io write_synthetic_dng -- --ignored`.
    #[test]
    #[ignore]
    fn write_synthetic_dng() {
        let path = std::env::var("DNG_OUT").unwrap();
        std::fs::write(path, synthetic_dng()).unwrap();
    }
}
