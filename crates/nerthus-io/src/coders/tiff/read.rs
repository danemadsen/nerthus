//! Strips and tiles as libtiff 4.7.1 decodes them (`tif_read.c`): the raw
//! bytes (bit-reversed for the other fill order), the codec, the
//! predictor's accumulation, then byte swapping into the host's (little-
//! endian) order. Decoded data is in the order ImageMagick imports.
//!
//! A libtiff error is `Err`; `Ok(None)` is a read that fails without one
//! (a tile past the end of a memory-mapped file), which ImageMagick only
//! notices by stopping.

use anyhow::{bail, Result};

use super::directory::*;

/// How a strip or tile read went (`TIFFReadEncodedStrip`'s -1 comes in
/// two kinds that ImageMagick's callers treat differently).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    /// The bytes decoded.
    Read(usize),
    /// The raw data could not be had (the buffer is cleared).
    FillFailed,
    /// The codec stopped short without an error (what it decoded stays).
    DecodeFailed,
}

impl Outcome {
    pub fn is_read(self) -> bool {
        matches!(self, Outcome::Read(_))
    }
}

/// The codec state kept between strips (`tif_data`, `TIFF_CODERSETUP`).
#[derive(Default)]
pub(super) struct Decoder {
    set_up: bool,
    fax_mode: Option<u32>,
    lzw_old_style: Option<bool>,
    jpeg: Option<super::jpeg::TiffJpeg>,
    old_jpeg: Option<super::ojpeg::OldJpeg>,
    /// The strip decoded for scanline reads (`TIFFReadScanline`).
    scanline_strip: Option<(u32, Vec<u8>)>,
}

/// `TIFFReverseBits`.
fn reverse_bits(bytes: &mut [u8]) {
    for byte in bytes.iter_mut() {
        *byte = byte.reverse_bits();
    }
}

/// `_TIFFSwab16BitData` and friends: samples into the host's order.
fn swab_samples(bytes: &mut [u8], bits: u16) {
    let size = match bits {
        16 => 2,
        24 => 3,
        32 => 4,
        64 | 128 => 8,
        _ => return,
    };
    let whole = bytes.len() / size * size;
    for sample in bytes[..whole].chunks_exact_mut(size) {
        sample.reverse();
    }
}

/// Whether the codec reverses bits itself (`TIFF_NOBITREV`).
fn codec_reverses_bits(compression: u16) -> bool {
    matches!(compression, COMPRESSION_CCITTRLE | COMPRESSION_CCITTRLEW | COMPRESSION_CCITTFAX3 | COMPRESSION_CCITTFAX4)
}

/// Whether the codec takes a predictor (`TIFFPredictorInit`).
fn codec_has_predictor(compression: u16) -> bool {
    matches!(compression, COMPRESSION_LZW | COMPRESSION_ADOBE_DEFLATE | COMPRESSION_DEFLATE | COMPRESSION_LZMA)
}

impl Tiff<'_> {
    /// The byte swapping left after decoding (`tif_postdecode`): by sample
    /// size for big-endian files, unless a codec or the predictor took it
    /// over.
    fn post_decode_bits(&self) -> Option<u16> {
        if !self.swab {
            return None;
        }
        let compression = self.dir.compression;
        if matches!(compression, COMPRESSION_JPEG | COMPRESSION_OJPEG | COMPRESSION_SGILOG | COMPRESSION_SGILOG24) {
            return None;
        }
        let bits = self.dir.bits_per_sample;
        if codec_has_predictor(compression) {
            match self.dir.predictor {
                2 if matches!(bits, 16 | 32 | 64) => return None,
                3 => return None,
                _ => {}
            }
        }
        if matches!(bits, 16 | 24 | 32 | 64 | 128) {
            Some(bits)
        } else {
            None
        }
    }

    /// `PredictorSetup`'s checks and row size, for codecs with a
    /// predictor.
    fn predictor_row_size(&self) -> Result<u64> {
        let dir = &self.dir;
        match dir.predictor {
            1 => return Ok(0),
            2 => {
                if !matches!(dir.bits_per_sample, 8 | 16 | 32 | 64) {
                    bail!("Horizontal differencing \"Predictor\" not supported with {}-bit samples", dir.bits_per_sample);
                }
            }
            3 => {
                if dir.sample_format != SAMPLEFORMAT_IEEEFP {
                    bail!("Floating point \"Predictor\" not supported with {} data format", dir.sample_format);
                }
                if !matches!(dir.bits_per_sample, 16 | 24 | 32 | 64) {
                    bail!("Floating point \"Predictor\" not supported with {}-bit samples", dir.bits_per_sample);
                }
            }
            other => bail!("\"Predictor\" value {other} not supported"),
        }
        let size = if dir.tiled { self.tile_row_size()? } else { self.scanline_size()? };
        if size == 0 {
            bail!("PredictorSetup: zero row size");
        }
        Ok(size)
    }

    /// `tif_setupdecode`, once (`TIFF_CODERSETUP`).
    fn setup_decode(&self, decoder: &mut Decoder) -> Result<()> {
        if decoder.set_up {
            return Ok(());
        }
        let compression = self.dir.compression;
        if codec_has_predictor(compression) {
            self.predictor_row_size()?;
        }
        match compression {
            COMPRESSION_JPEG => decoder.jpeg = Some(super::jpeg::TiffJpeg::setup(self)?),
            COMPRESSION_OJPEG => decoder.old_jpeg = Some(super::ojpeg::OldJpeg::setup(self)?),
            COMPRESSION_SGILOG | COMPRESSION_SGILOG24 => super::luv::setup(self)?,
            COMPRESSION_WEBP => super::webp::setup(self)?,
            COMPRESSION_CCITTRLE | COMPRESSION_CCITTRLEW | COMPRESSION_CCITTFAX3 | COMPRESSION_CCITTFAX4 => {
                super::fax::setup(self)?;
                decoder.fax_mode = Some(super::fax::initial_mode(self.dir.compression));
            }
            COMPRESSION_THUNDERSCAN if self.dir.bits_per_sample != 4 => {
                bail!("Wrong bitspersample value ({}), Thunder decoder only supports 4bits per sample.", self.dir.bits_per_sample);
            }
            _ => {}
        }
        decoder.set_up = true;
        Ok(())
    }

    /// The raw bytes of strip or tile `index` (`TIFFFillStrip`,
    /// `TIFFFillTile`), bit-reversed for the other fill order; `None`
    /// when a tile is past the end of a mapped file.
    fn fill(&self, index: u32, is_tile: bool) -> Result<Option<Vec<u8>>> {
        let dir = &self.dir;
        let byte_count = dir.strip_byte_counts[index as usize];
        let offset = dir.strip_offsets[index as usize];
        if byte_count == 0 || byte_count > i64::MAX as u64 {
            bail!("Invalid strip byte count {byte_count}, strip {index}");
        }
        if byte_count > 1024 * 1024 {
            let size = if is_tile { self.tile_size()? } else { self.strip_size()? };
            if size != 0 && (byte_count - 4096) / 10 > size {
                let limited = size * 10 + 4096;
                bail!("Too large strip byte count {byte_count}, strip {index}. Limiting to {limited}");
            }
        }
        let file_size = self.data.len() as u64;
        if byte_count > file_size || offset > file_size - byte_count {
            if self.mapped && is_tile {
                return Ok(None);
            }
            bail!("Read error on strip {index}; got {} bytes, expected {byte_count}", file_size.saturating_sub(offset));
        }
        let mut raw = self.data[offset as usize..(offset + byte_count) as usize].to_vec();
        if dir.fill_order != FILLORDER_MSB2LSB && !codec_reverses_bits(dir.compression) {
            reverse_bits(&mut raw);
        }
        Ok(Some(raw))
    }

    /// `tif_decodestrip`/`tif_decodetile` into `output`, then the
    /// predictor; false when the codec stopped short without an error.
    /// `row` is the image row the data starts at, `offset` where the raw
    /// data lies in the file.
    #[allow(clippy::too_many_arguments)]
    fn decode(&self, decoder: &mut Decoder, raw: &[u8], output: &mut [u8], plane: u16, is_tile: bool, row: u32, offset: u64) -> Result<bool> {
        let compression = self.dir.compression;
        let size = output.len();
        match compression {
            COMPRESSION_NONE => {
                if raw.len() < size {
                    bail!("Not enough data for scanline, expected a request for at most {} bytes, got a request for {size} bytes", raw.len());
                }
                output.copy_from_slice(&raw[..size]);
            }
            COMPRESSION_PACKBITS => packbits(raw, output)?,
            COMPRESSION_LZW => {
                // The first strip decides between the decoders for good.
                let looks_old = raw.len() >= 2 && raw[0] == 0 && raw[1] & 1 != 0;
                let old_style = *decoder.lzw_old_style.get_or_insert(looks_old);
                if old_style {
                    super::lzw::decode_compat(raw, output)?;
                } else {
                    super::lzw::decode(raw, output)?;
                }
            }
            COMPRESSION_ADOBE_DEFLATE | COMPRESSION_DEFLATE => inflate(raw, output)?,
            COMPRESSION_LZMA => {
                let decoded = super::super::xz::decompress(raw).map_err(|error| anyhow::anyhow!("Decoding error at scanline 0, {error}"))?;
                if decoded.len() < size {
                    bail!("Not enough data at scanline 0 (short {} bytes)", size - decoded.len());
                }
                output.copy_from_slice(&decoded[..size]);
            }
            COMPRESSION_JPEG => {
                let jpeg = decoder.jpeg.as_mut().expect("JPEG decoder set up");
                jpeg.decode(self, raw, output, plane, is_tile, row)?;
            }
            COMPRESSION_OJPEG => {
                let old_jpeg = decoder.old_jpeg.as_mut().expect("OJPEG decoder set up");
                old_jpeg.decode(self, raw, output, plane)?;
            }
            COMPRESSION_CCITTRLE | COMPRESSION_CCITTRLEW | COMPRESSION_CCITTFAX3 | COMPRESSION_CCITTFAX4 => {
                let mode = decoder.fax_mode.get_or_insert(super::fax::initial_mode(compression));
                if !super::fax::decode(self, raw, output, mode, offset)? {
                    return Ok(false);
                }
            }
            COMPRESSION_SGILOG | COMPRESSION_SGILOG24 => super::luv::decode(self, raw, output, is_tile)?,
            COMPRESSION_NEXT => super::next::decode(self, raw, output, is_tile)?,
            COMPRESSION_THUNDERSCAN => super::thunder::decode(self, raw, output, is_tile)?,
            COMPRESSION_WEBP => super::webp::decode(self, raw, output, is_tile, row)?,
            other => bail!("Compression scheme {other} strip decoding is not implemented"),
        }
        if codec_has_predictor(compression) && self.dir.predictor != 1 {
            let row_size = self.predictor_row_size()? as usize;
            if !output.len().is_multiple_of(row_size) {
                bail!("occ0%rowsize != 0");
            }
            for line in output.chunks_exact_mut(row_size) {
                self.accumulate(line)?;
            }
        }
        Ok(true)
    }

    /// The predictor's accumulation of one row (`horAcc8`/`16`/`32`/`64`,
    /// their byte-swapping forms, `fpAcc`).
    fn accumulate(&self, row: &mut [u8]) -> Result<()> {
        let dir = &self.dir;
        let stride = if dir.planar_config == PLANARCONFIG_CONTIG { dir.samples_per_pixel as usize } else { 1 };
        if dir.predictor == 3 {
            return floating_accumulate(row, (dir.bits_per_sample / 8) as usize, stride);
        }
        let size = (dir.bits_per_sample / 8) as usize;
        if !row.len().is_multiple_of(size * stride) {
            bail!("horAcc{}: (cc%stride)!=0", dir.bits_per_sample);
        }
        if self.swab && size > 1 {
            for sample in row.chunks_exact_mut(size) {
                sample.reverse();
            }
        }
        let count = row.len() / size;
        match size {
            1 => {
                for index in stride..count {
                    row[index] = row[index].wrapping_add(row[index - stride]);
                }
            }
            2 => {
                for index in stride..count {
                    let previous = u16::from_le_bytes([row[2 * (index - stride)], row[2 * (index - stride) + 1]]);
                    let current = u16::from_le_bytes([row[2 * index], row[2 * index + 1]]);
                    let sum = current.wrapping_add(previous).to_le_bytes();
                    row[2 * index..2 * index + 2].copy_from_slice(&sum);
                }
            }
            4 => {
                for index in stride..count {
                    let previous = u32::from_le_bytes(row[4 * (index - stride)..4 * (index - stride) + 4].try_into().unwrap());
                    let current = u32::from_le_bytes(row[4 * index..4 * index + 4].try_into().unwrap());
                    let sum = current.wrapping_add(previous).to_le_bytes();
                    row[4 * index..4 * index + 4].copy_from_slice(&sum);
                }
            }
            _ => {
                for index in stride..count {
                    let previous = u64::from_le_bytes(row[8 * (index - stride)..8 * (index - stride) + 8].try_into().unwrap());
                    let current = u64::from_le_bytes(row[8 * index..8 * index + 8].try_into().unwrap());
                    let sum = current.wrapping_add(previous).to_le_bytes();
                    row[8 * index..8 * index + 8].copy_from_slice(&sum);
                }
            }
        }
        Ok(())
    }

    /// `TIFFReadEncodedStrip`: strip `strip` decoded into `buffer`, at
    /// most `limit` bytes of it; the bytes written.
    pub fn read_encoded_strip(&self, decoder: &mut Decoder, strip: u32, buffer: &mut [u8], limit: Option<u64>) -> Result<Outcome> {
        self.read_encoded_strip_with(decoder, strip, buffer, limit, true)
    }

    /// `TIFFReadEncodedStrip`, or without `direct` its allocating variant
    /// (`_TIFFReadEncodedStripAndAllocBuffer`), which never reads an
    /// uncompressed strip straight from the file.
    pub fn read_encoded_strip_with(&self, decoder: &mut Decoder, strip: u32, buffer: &mut [u8], limit: Option<u64>, direct: bool) -> Result<Outcome> {
        let dir = &self.dir;
        if dir.tiled {
            bail!("Can not read scanlines from a tiled image");
        }
        if strip >= dir.strip_count {
            bail!("{strip}: Strip out of range, max {}", dir.strip_count);
        }
        let rows_per_strip = dir.rows_per_strip.min(dir.length);
        if rows_per_strip == 0 {
            bail!("rowsperstrip is zero");
        }
        let strips_per_plane = dir.length.div_ceil(rows_per_strip);
        let strip_in_plane = strip % strips_per_plane;
        let plane = (strip / strips_per_plane) as u16;
        let rows = (dir.length - strip_in_plane * rows_per_strip).min(rows_per_strip);
        let mut size = self.vstrip_size(rows)?;
        if size == 0 {
            return Ok(Outcome::FillFailed);
        }
        // Uncompressed strips of unmapped files are read straight from the
        // file, whatever their byte count says.
        if direct && dir.compression == COMPRESSION_NONE && limit.is_some_and(|limit| limit >= size) && !self.mapped {
            let offset = dir.strip_offsets[strip as usize];
            let end = offset.checked_add(size);
            let available = end.is_some_and(|end| end <= self.data.len() as u64);
            if !available {
                bail!("Read error at scanline; got {} bytes, expected {size}", (self.data.len() as u64).saturating_sub(offset));
            }
            let output = &mut buffer[..size as usize];
            output.copy_from_slice(&self.data[offset as usize..(offset + size) as usize]);
            if dir.fill_order != FILLORDER_MSB2LSB {
                reverse_bits(output);
            }
            if let Some(bits) = self.post_decode_bits() {
                swab_samples(output, bits);
            }
            return Ok(Outcome::Read(size as usize));
        }
        if let Some(limit) = limit
            && limit < size
        {
            size = limit;
        }
        let Some(raw) = self.fill(strip, false)? else {
            buffer[..size as usize].fill(0);
            return Ok(Outcome::FillFailed);
        };
        self.setup_decode(decoder)?;
        // `TIFFStartStrip`: the strip's first row.
        let row = (strip % dir.strips_per_image.max(1)) * dir.rows_per_strip;
        let offset = dir.strip_offsets[strip as usize];
        let output = &mut buffer[..size as usize];
        if !self.decode(decoder, &raw, output, plane, false, row, offset)? {
            return Ok(Outcome::DecodeFailed);
        }
        if let Some(bits) = self.post_decode_bits() {
            swab_samples(output, bits);
        }
        Ok(Outcome::Read(size as usize))
    }

    /// `TIFFComputeTile`.
    fn compute_tile(&self, x: u32, y: u32, plane: u16) -> u32 {
        let dir = &self.dir;
        let across = how_many(dir.width, dir.tile_width);
        let down = how_many(dir.length, dir.tile_length);
        let deep = how_many(dir.image_depth, dir.tile_depth);
        let tile = across.wrapping_mul(y / dir.tile_length).wrapping_add(x / dir.tile_width);
        if dir.planar_config == PLANARCONFIG_SEPARATE {
            return across.wrapping_mul(down).wrapping_mul(deep).wrapping_mul(plane as u32).wrapping_add(tile);
        }
        tile
    }

    /// `TIFFReadTile`: the tile holding pixel `(x, y)` of `plane`, decoded
    /// into `buffer`; the bytes written. A tile that fails silently leaves
    /// the buffer cleared.
    pub fn read_tile(&self, decoder: &mut Decoder, x: u32, y: u32, plane: u16, buffer: &mut [u8]) -> Result<Outcome> {
        let dir = &self.dir;
        if !dir.tiled {
            bail!("Can not read tiles from a striped image");
        }
        if x >= dir.width {
            bail!("{x}: Col out of range, max {}", dir.width - 1);
        }
        if y >= dir.length {
            bail!("{y}: Row out of range, max {}", dir.length - 1);
        }
        if dir.planar_config == PLANARCONFIG_SEPARATE && plane >= dir.samples_per_pixel {
            bail!("{plane}: Sample out of range, max {}", dir.samples_per_pixel - 1);
        }
        let tile = self.compute_tile(x, y, plane);
        if tile >= dir.strip_count {
            bail!("{tile}: Tile out of range, max {}", dir.strip_count);
        }
        let size = self.tile_size()? as usize;
        let Some(raw) = self.fill(tile, true)? else {
            buffer[..size].fill(0);
            return Ok(Outcome::FillFailed);
        };
        self.setup_decode(decoder)?;
        let tile_plane = (tile / dir.strips_per_image.max(1)) as u16;
        // `TIFFStartTile`'s row (with libtiff's mix-up of the counts).
        let across = how_many(dir.width, dir.tile_width).max(1);
        let row = (tile % across) * dir.tile_length;
        let offset = dir.strip_offsets[tile as usize];
        let output = &mut buffer[..size];
        if !self.decode(decoder, &raw, output, tile_plane, true, row, offset)? {
            return Ok(Outcome::DecodeFailed);
        }
        if let Some(bits) = self.post_decode_bits() {
            swab_samples(output, bits);
        }
        Ok(Outcome::Read(size))
    }

    /// `TIFFReadScanline`: row `row` of `plane`, from its strip decoded
    /// whole.
    pub fn read_scanline(&self, decoder: &mut Decoder, row: u32, plane: u16) -> Result<Option<Vec<u8>>> {
        let dir = &self.dir;
        if dir.tiled {
            bail!("Can not read scanlines from a tiled image");
        }
        if row >= dir.length {
            bail!("{row}: Row out of range, max {}", dir.length);
        }
        let rows_per_strip = dir.rows_per_strip.min(dir.length).max(1);
        let mut strip = row / rows_per_strip;
        if dir.planar_config == PLANARCONFIG_SEPARATE {
            if plane >= dir.samples_per_pixel {
                bail!("{plane}: Sample out of range, max {}", dir.samples_per_pixel);
            }
            strip += plane as u32 * dir.strips_per_image;
        }
        let cached = decoder.scanline_strip.as_ref().is_some_and(|(index, _)| *index == strip);
        if !cached {
            let mut bytes = vec![0u8; self.strip_size()? as usize];
            let Outcome::Read(size) = self.read_encoded_strip(decoder, strip, &mut bytes, None)? else {
                return Ok(None);
            };
            bytes.truncate(size);
            decoder.scanline_strip = Some((strip, bytes));
        }
        let scanline = self.scanline_size()? as usize;
        let start = (row % rows_per_strip) as usize * scanline;
        let bytes = &decoder.scanline_strip.as_ref().expect("strip cached").1;
        let mut line = vec![0u8; scanline];
        let available = bytes.len().saturating_sub(start).min(scanline);
        line[..available].copy_from_slice(&bytes[start..start + available]);
        Ok(Some(line))
    }
}

/// `fpAcc`: bytes summed across the row, then each value's bytes gathered
/// from their planes (most significant first) into host order.
fn floating_accumulate(row: &mut [u8], bytes_per_sample: usize, stride: usize) -> Result<()> {
    if !row.len().is_multiple_of(bytes_per_sample * stride) {
        bail!("fpAcc: cc%(bps*stride))!=0");
    }
    for index in stride..row.len() {
        row[index] = row[index].wrapping_add(row[index - stride]);
    }
    let planes = row.to_vec();
    let count = row.len() / bytes_per_sample;
    for value in 0..count {
        for byte in 0..bytes_per_sample {
            row[bytes_per_sample * value + byte] = planes[(bytes_per_sample - byte - 1) * count + value];
        }
    }
    Ok(())
}

/// `PackBitsDecode`.
fn packbits(raw: &[u8], output: &mut [u8]) -> Result<()> {
    let mut input = 0usize;
    let mut out = 0usize;
    let mut occ = output.len();
    while input < raw.len() && occ > 0 {
        let n = raw[input] as i8 as i64;
        input += 1;
        if n < 0 {
            if n == -128 {
                continue;
            }
            let mut count = (-n + 1) as usize;
            if occ < count {
                count = occ;
            }
            if input >= raw.len() {
                break;
            }
            occ -= count;
            let value = raw[input];
            input += 1;
            output[out..out + count].fill(value);
            out += count;
        } else {
            let mut count = n as usize + 1;
            if occ < count {
                count = occ;
            }
            if raw.len() - input < count {
                break;
            }
            output[out..out + count].copy_from_slice(&raw[input..input + count]);
            out += count;
            occ -= count;
            input += count;
        }
    }
    if occ > 0 {
        output[out..].fill(0);
        bail!("Not enough data for scanline");
    }
    Ok(())
}

/// `ZIPDecode`: zlib's inflate until the output is full.
fn inflate(raw: &[u8], output: &mut [u8]) -> Result<()> {
    let mut inflater = flate2::Decompress::new(true);
    loop {
        let consumed = inflater.total_in() as usize;
        let produced = inflater.total_out() as usize;
        if produced >= output.len() {
            return Ok(());
        }
        let status = inflater.decompress(&raw[consumed.min(raw.len())..], &mut output[produced..], flate2::FlushDecompress::Sync);
        match status {
            Ok(flate2::Status::StreamEnd) => break,
            Ok(flate2::Status::Ok) => {
                let stalled = inflater.total_in() as usize == consumed && inflater.total_out() as usize == produced;
                if stalled {
                    bail!("ZLib error: buffer error");
                }
            }
            Ok(flate2::Status::BufError) => bail!("ZLib error: buffer error"),
            Err(error) => bail!("Decoding error at scanline 0, {error}"),
        }
    }
    let produced = inflater.total_out() as usize;
    if produced < output.len() {
        output[produced..].fill(0);
        bail!("Not enough data at scanline 0 (short {} bytes)", output.len() - produced);
    }
    Ok(())
}
