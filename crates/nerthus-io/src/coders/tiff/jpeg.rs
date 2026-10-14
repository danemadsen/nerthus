//! libtiff's JPEG codec (`tif_jpeg.c`) on the libjpeg-turbo port: the
//! JPEGTables stream read once, each strip or tile a datastream of its own
//! whose size, component count, precision and sampling must agree with the
//! directory. Colour conversion is suppressed — samples come out as stored
//! — except through the RGBA interface, which asks for YCbCr as RGB.
//! Subsampled YCbCr is otherwise read raw and packed into TIFF's clumps,
//! one scanline's worth further on per clump row (so rows overlap, as in
//! libtiff).

use anyhow::{anyhow, bail, Result};

use super::super::jpeg::{Strip, StripSamples, Tables, TiffOutput};
use super::directory::*;

/// The codec state (`JPEGState`): the tables libjpeg keeps.
pub(super) struct TiffJpeg {
    tables: Tables,
    h_sampling: u32,
    v_sampling: u32,
}

impl TiffJpeg {
    /// `JPEGSetupDecode`.
    pub fn setup(tiff: &Tiff) -> Result<Self> {
        let mut tables = Tables::default();
        if let Some(stream) = &tiff.dir.jpeg_tables {
            let tables_only = tables.read(stream).map_err(|error| anyhow!("{error}"))?;
            if !tables_only {
                bail!("Bogus JPEGTables field");
            }
        }
        let (h_sampling, v_sampling) = if tiff.dir.photometric == Some(PHOTOMETRIC_YCBCR) {
            (tiff.dir.ycbcr_subsampling[0] as u32, tiff.dir.ycbcr_subsampling[1] as u32)
        } else {
            (1, 1)
        };
        Ok(Self { tables, h_sampling, v_sampling })
    }

    /// `JPEGPreDecode` then `JPEGDecode` or `JPEGDecodeRaw` for the
    /// `output.len()` bytes asked of strip or tile data `raw`, which starts
    /// at image row `row`.
    pub fn decode(&mut self, tiff: &Tiff, raw: &[u8], output: &mut [u8], plane: u16, is_tile: bool, row: u32) -> Result<()> {
        let dir = &tiff.dir;
        let strip = Strip::open(raw, &self.tables).map_err(|error| anyhow!("{error}"))?;
        let info = strip.info();
        let (mut segment_width, mut segment_height, bytes_per_line) = if is_tile {
            (dir.tile_width, dir.tile_length, tiff.tile_row_size()?)
        } else {
            let height = (dir.length - row).min(dir.rows_per_strip);
            (dir.width, height, tiff.scanline_size()?)
        };
        let contig = dir.planar_config == PLANARCONFIG_CONTIG;
        if !contig && plane > 0 {
            if self.h_sampling == 0 || self.v_sampling == 0 {
                bail!("JPEG horizontal or vertical sampling is zero");
            }
            segment_width = how_many(segment_width, self.h_sampling);
            segment_height = how_many(segment_height, self.v_sampling);
        }
        let jpeg_width = info.width as u32;
        let jpeg_height = info.height as u32;
        let truncated_last = jpeg_width == segment_width && jpeg_height > segment_height && row + segment_height == dir.length && !is_tile;
        if !truncated_last && (jpeg_width > segment_width || jpeg_height > segment_height) {
            bail!("JPEG strip/tile size exceeds expected dimensions, expected {segment_width}x{segment_height}, got {jpeg_width}x{jpeg_height}");
        }
        let expected_components = if contig { dir.samples_per_pixel as usize } else { 1 };
        if info.sampling.len() != expected_components {
            bail!("Improper JPEG component count");
        }
        if info.precision != dir.bits_per_sample as i32 {
            bail!("Improper JPEG data precision");
        }
        if contig {
            let (h, v, _) = info.sampling[0];
            if h as u32 != self.h_sampling || v as u32 != self.v_sampling {
                bail!("Improper JPEG sampling factors {h},{v}\nApparently should be {},{}.", self.h_sampling, self.v_sampling);
            }
            if info.sampling[1..].iter().any(|&(h, v, _)| h != 1 || v != 1) {
                bail!("Improper JPEG sampling factors");
            }
        } else {
            let (h, v, _) = info.sampling[0];
            if h != 1 || v != 1 {
                bail!("Improper JPEG sampling factors");
            }
        }
        let ycbcr = dir.photometric == Some(PHOTOMETRIC_YCBCR);
        let mode = if contig && ycbcr && tiff.jpeg_color_mode_rgb {
            TiffOutput::YCbCrToRgb
        } else if contig && (self.h_sampling != 1 || self.v_sampling != 1) {
            TiffOutput::Raw
        } else {
            TiffOutput::Unconverted
        };
        if bytes_per_line == 0 {
            output.fill(0);
            bail!("JPEGDecode: zero bytes per line");
        }
        let bytes_per_line = bytes_per_line as usize;
        if mode == TiffOutput::Raw {
            return self.decode_raw(tiff, strip, &info, output, bytes_per_line, is_tile, row);
        }
        // `JPEGDecode`: whole scanlines straight into the buffer.
        let rows = (output.len() / bytes_per_line).min(info.height);
        let finish = rows == info.height;
        let samples = strip.decode(mode, finish, &mut self.tables).map_err(|error| {
            anyhow!("{error}")
        })?;
        let StripSamples::Interleaved { components, samples } = samples else {
            unreachable!("interleaved output asked for");
        };
        let line = info.width * components;
        let length = output.len();
        for (index, chunk) in samples.chunks_exact(line.max(1)).take(rows).enumerate() {
            let start = index * bytes_per_line;
            let destination = &mut output[start..(start + line).min(length)];
            for (byte, &sample) in destination.iter_mut().zip(chunk.iter()) {
                *byte = sample as u8;
            }
        }
        Ok(())
    }

    /// `JPEGDecodeRaw`: each clump row's components packed into clumps of
    /// Y samples then Cb and Cr, the buffer moving on a scanline per clump
    /// row.
    #[allow(clippy::too_many_arguments)]
    fn decode_raw(&mut self, tiff: &Tiff, strip: Strip, info: &super::super::jpeg::StripInfo, output: &mut [u8], bytes_per_line: usize, is_tile: bool, row: u32) -> Result<()> {
        let dir = &tiff.dir;
        let mut rows = info.height as i64;
        if !is_tile && rows > (dir.length - row) as i64 {
            rows = (dir.length - row) as i64;
        }
        // Every iMCU row the loop reaches is read; reading them all lets
        // the decompressor finish.
        let clump_rows = (rows.max(0) as usize).div_ceil(self.v_sampling as usize);
        let max_v = info.sampling.iter().map(|&(_, v, _)| v).max().unwrap_or(1);
        let imcu_rows_read = clump_rows.div_ceil(8);
        let total_imcu_rows = info.height.div_ceil(8 * max_v);
        let finish = imcu_rows_read >= total_imcu_rows;
        let samples = strip.decode(TiffOutput::Raw, finish, &mut self.tables).map_err(|error| anyhow!("{error}"))?;
        let StripSamples::Raw { planes } = samples else {
            unreachable!("raw output asked for");
        };
        if rows <= 0 {
            return Ok(());
        }
        let clumps_per_line = info.sampling.get(1).map_or(0, |&(_, _, width)| width);
        let samples_per_clump: usize = info.sampling.iter().map(|&(h, v, _)| h * v).sum();
        let mut remaining = output.len() as i64;
        let mut base = 0usize;
        for clump_row in 0..clump_rows {
            if remaining < bytes_per_line as i64 {
                bail!("application buffer not large enough for all data.");
            }
            let mut clump_offset = 0usize;
            for (component, &(h, v, _)) in info.sampling.iter().enumerate() {
                let (samples, stride) = &planes[component];
                for y in 0..v {
                    let source_row = clump_row * v + y;
                    let needed = clump_offset + samples_per_clump * (clumps_per_line.max(1) - 1) + h;
                    if remaining < needed as i64 {
                        bail!("application buffer not large enough for all data, possible subsampling issue");
                    }
                    let line = &samples[source_row * stride..];
                    let mut input = 0usize;
                    for clump in 0..clumps_per_line {
                        let start = base + clump_offset + clump * samples_per_clump;
                        for x in 0..h {
                            if let Some(byte) = output.get_mut(start + x) {
                                *byte = line.get(input).copied().unwrap_or(0) as u8;
                            }
                            input += 1;
                        }
                    }
                    clump_offset += h;
                }
            }
            base += bytes_per_line;
            remaining -= bytes_per_line as i64;
        }
        Ok(())
    }
}

/// `JPEGFixupTagsSubsampling`: YCbCr without a YCbCrSubsampling tag takes
/// its sampling from the first strip's frame header.
pub(super) fn fixup_subsampling(tiff: &mut Tiff) {
    let dir = &tiff.dir;
    let applies = dir.photometric == Some(PHOTOMETRIC_YCBCR) && dir.planar_config == PLANARCONFIG_CONTIG && dir.samples_per_pixel == 3 && !dir.ycbcr_subsampling_set;
    if !applies {
        return;
    }
    let offset = dir.strip_offsets.first().copied().unwrap_or(0);
    if offset == 0 {
        return;
    }
    let count = dir.strip_byte_counts.first().copied().unwrap_or(0);
    let start = (offset as usize).min(tiff.data.len());
    let end = (offset.saturating_add(count) as usize).min(tiff.data.len());
    let bytes = &tiff.data[start..end];
    let samples = dir.samples_per_pixel as usize;
    if let Some([horizontal, vertical]) = scan_sampling(bytes, samples) {
        tiff.dir.ycbcr_subsampling = [horizontal as u16, vertical as u16];
    }
}

/// `JPEGFixupTagsSubsamplingSec`: the first SOF's sampling factors when
/// the chroma components are unsampled and the luma's are 1, 2 or 4.
fn scan_sampling(bytes: &[u8], samples: usize) -> Option<[u8; 2]> {
    let mut position = 0usize;
    let next = |position: &mut usize| -> Option<u8> {
        let byte = *bytes.get(*position)?;
        *position += 1;
        Some(byte)
    };
    loop {
        loop {
            if next(&mut position)? == 255 {
                break;
            }
        }
        let mut marker;
        loop {
            marker = next(&mut position)?;
            if marker != 255 {
                break;
            }
        }
        match marker {
            0xD8 => {}
            0xFE | 0xE0..=0xEF | 0xDB | 0xDA | 0xC4 | 0xDD => {
                let high = next(&mut position)? as u16;
                let low = next(&mut position)? as u16;
                let length = (high << 8) | low;
                if length < 2 {
                    return None;
                }
                position += (length - 2) as usize;
            }
            0xC0 | 0xC1 | 0xC2 | 0xC9 | 0xCA => {
                let high = next(&mut position)? as u16;
                let low = next(&mut position)? as u16;
                let length = (high << 8) | low;
                if length as usize != 8 + samples * 3 {
                    return None;
                }
                position += 7;
                let sampling = next(&mut position)?;
                let (horizontal, vertical) = (sampling >> 4, sampling & 15);
                position += 1;
                for _ in 1..samples {
                    position += 1;
                    if next(&mut position)? != 0x11 {
                        return None;
                    }
                    position += 1;
                }
                let valid = |value: u8| matches!(value, 1 | 2 | 4);
                if !valid(horizontal) || !valid(vertical) {
                    return None;
                }
                return Some([horizontal, vertical]);
            }
            _ => return None,
        }
    }
}
