//! LibRaw's `unpack` for DNG files: the raw buffer the decoder fills
//! (`raw_image` for CFA and single-channel data, `color4_image` or
//! `color3_image` for linear RGB), the DNG decoders (`decoders/dng.cpp`,
//! `fp_dng.cpp`), `crop_masked_pixels` and the black level fold.
//! Magick.Native builds LibRaw with `NO_JPEG`, so lossy DNG files never
//! get here (`identify` rejects them).

use std::io::Read;

use anyhow::{bail, Context, Result};

use super::identify::{fc, Decoder, Raw, CBLACK_SIZE, NOT_TILED};
use super::ljpeg::{ljpeg_idct, ljpeg_row, ljpeg_start, BitReader};
use super::stream::{Stream, BIG_ENDIAN};

/// The unpacked raw data.
pub(super) enum RawData {
    /// `raw_image`: one sample a pixel, `raw_width` a row.
    Bayer(Vec<u16>),
    /// `color4_image`.
    Color4(Vec<[u16; 4]>),
    /// `color3_image`.
    Color3(Vec<[u16; 3]>),
}

/// Where `adobe_copy_pixel` writes.
enum Target<'a> {
    Raw(&'a mut [u16]),
    Image(&'a mut [[u16; 4]]),
}

struct Unpacker<'a, 'b> {
    stream: Stream<'b>,
    bits: BitReader,
    state: &'a mut Raw,
}

pub(super) fn unpack(data: &[u8], state: &mut Raw) -> Result<RawData> {
    let raw_width = state.raw_width as usize;
    let raw_height = state.raw_height as usize;
    let width = state.width as usize;
    let height = state.height as usize;
    let alloc_width = raw_width.max(width + state.left_margin as usize);
    let alloc_height = raw_height.max(height + state.top_margin as usize);
    if alloc_width > 65535 || alloc_height > 65535 {
        bail!("the DNG raw image is damaged");
    }
    let mut stream = Stream::new(data);
    stream.order = state.order;
    stream.seek(state.data_offset.max(0) as u64);
    let mut unpacker = Unpacker { stream, bits: BitReader::default(), state };
    let single_plane = unpacker.state.filters != 0 || unpacker.state.colors == 1;

    let mut raw_data = match unpacker.state.decoder {
        Decoder::Lossless | Decoder::Packed if single_plane => {
            // LibRaw mallocs this; a valid file's decoder writes it all.
            let mut raw_image = vec![0u16; alloc_width * (alloc_height + 8)];
            let mut target = Target::Raw(&mut raw_image);
            unpacker.load_adobe(&mut target)?;
            RawData::Bayer(raw_image)
        }
        Decoder::Lossless | Decoder::Packed => {
            let mut image = vec![[0u16; 4]; width.max(raw_width) * (height.max(raw_height) + 8)];
            let mut target = Target::Image(&mut image);
            unpacker.load_adobe(&mut target)?;
            RawData::Color4(image)
        }
        Decoder::Deflate | Decoder::UncompressedFloat => unpacker.load_floats()?,
    };

    let state = unpacker.state;
    if let RawData::Bayer(raw_image) = &raw_data {
        crop_masked_pixels(state, raw_image);
    }
    // Move the smallest channel black into the common black.
    let mut common = state.cblack[3];
    for channel in 0..3 {
        if common > state.cblack[channel] {
            common = state.cblack[channel];
        }
    }
    for channel in 0..4 {
        state.cblack[channel] -= common;
    }
    state.black = state.black.wrapping_add(common);
    if let RawData::Bayer(raw_image) = &mut raw_data {
        raw_image.truncate(state.raw_width as usize * state.raw_height as usize);
    }
    Ok(raw_data)
}

impl Unpacker<'_, '_> {
    /// `adobe_copy_pixel`: one pixel's samples through `curve`.
    fn copy_pixel(&self, target: &mut Target, row: u64, column: u64, samples: &[u16], position: &mut usize) {
        let state = &self.state;
        let raw_width = state.raw_width as u64;
        let count = state.tiff_samples as usize;
        let sample = |index: usize| -> u16 { state.curve[samples.get(index).copied().unwrap_or(0) as usize] };
        if row < state.raw_height as u64 && column < raw_width {
            let index = (row * raw_width + column) as usize;
            match target {
                Target::Raw(raw_image) => raw_image[index] = sample(*position),
                Target::Image(image) => {
                    for channel in 0..count {
                        image[index][channel] = sample(*position + channel);
                    }
                }
            }
        }
        *position += count;
    }

    fn load_adobe(&mut self, target: &mut Target) -> Result<()> {
        match self.state.decoder {
            Decoder::Lossless => self.lossless_dng_load_raw(target),
            _ if self.state.tile_length < NOT_TILED => self.packed_tiled_dng_load_raw(target),
            _ => self.packed_dng_load_raw(target),
        }
        Ok(())
    }

    /// `lossless_dng_load_raw`: lossless JPEG tiles (or strips).
    fn lossless_dng_load_raw(&mut self, target: &mut Target) {
        let raw_width = self.state.raw_width as u64;
        let raw_height = self.state.raw_height as u64;
        let tile_width = self.state.tile_width as u64;
        let tile_length = self.state.tile_length as u64;
        let dng_version = self.state.dng_version;
        let single_plane = self.state.filters != 0 || self.state.colors == 1;
        let mut trow = 0u64;
        let mut tcol = 0u64;
        while trow < raw_height {
            let save = self.stream.tell();
            if tile_length < NOT_TILED as u64 {
                let offset = self.stream.get4();
                self.stream.seek(offset as u64);
            }
            let Some(mut head) = ljpeg_start(&mut self.stream, &mut self.bits, dng_version, false) else {
                break;
            };
            let mut jwide = head.wide as u64;
            if single_plane {
                jwide *= head.clrs as u64;
            }
            match head.algo {
                0xc1 => {
                    head.vpred[0] = 16384;
                    self.bits.restart();
                    let mut jrow = 0u64;
                    while jrow + 7 < head.high as u64 {
                        let mut jcol = 0u64;
                        while jcol + 7 < head.wide as u64 {
                            ljpeg_idct(&mut self.stream, &mut self.bits, &mut head, dng_version);
                            let block = head.idct;
                            let mut position = 0usize;
                            let row = trow + jcol / tile_width + jrow * 2;
                            let column = tcol + jcol % tile_width;
                            for i in (0..16).step_by(2) {
                                for j in 0..8 {
                                    self.copy_pixel(target, row + i, column + j, &block, &mut position);
                                }
                            }
                            jcol += 8;
                        }
                        jrow += 8;
                    }
                }
                0xc3 => {
                    let mut row = 0u64;
                    let mut column = 0u64;
                    let clrs = head.clrs as u64;
                    let whole_rows = self.state.tiff_samples == 1 && clrs > 1 && clrs * jwide == raw_width;
                    let count = if whole_rows { jwide * clrs } else { jwide };
                    for jrow in 0..head.high {
                        let start = ljpeg_row(&mut self.stream, &mut self.bits, &mut head, jrow, dng_version);
                        let samples = &head.rows[start..];
                        let mut position = 0usize;
                        for _ in 0..count {
                            self.copy_pixel(target, trow + row, tcol + column, samples, &mut position);
                            column += 1;
                            if column >= tile_width || column >= raw_width {
                                row += 1;
                                column = 0;
                            }
                        }
                    }
                }
                _ => {}
            }
            self.stream.seek(save + 4);
            tcol += tile_width;
            if tcol >= raw_width {
                trow += tile_length;
                tcol = 0;
            }
        }
    }

    /// One row of `count` uncompressed samples.
    fn read_samples(&mut self, samples: &mut [u16]) {
        if self.state.tiff_bps == 16 {
            self.stream.read_shorts(samples);
        } else {
            self.bits.restart();
            let bps = self.state.tiff_bps as i32;
            for sample in samples.iter_mut() {
                *sample = self.bits.getbits(&mut self.stream, bps) as u16;
            }
        }
    }

    /// `packed_dng_load_raw`: uncompressed strips.
    fn packed_dng_load_raw(&mut self, target: &mut Target) {
        let raw_width = self.state.raw_width as usize;
        let samples = self.state.tiff_samples as usize;
        let mut pixel = vec![0u16; raw_width * samples];
        for row in 0..self.state.raw_height as u64 {
            self.read_samples(&mut pixel);
            let mut position = 0usize;
            for column in 0..raw_width as u64 {
                self.copy_pixel(target, row, column, &pixel, &mut position);
            }
        }
    }

    /// `packed_tiled_dng_load_raw`: uncompressed tiles.
    fn packed_tiled_dng_load_raw(&mut self, target: &mut Target) {
        let raw_width = self.state.raw_width as u64;
        let raw_height = self.state.raw_height as u64;
        let tile_width = self.state.tile_width as u64;
        let tile_length = self.state.tile_length as u64;
        let samples = self.state.tiff_samples as usize;
        let tiles = 1 + raw_width / tile_width;
        let mut pixel = vec![0u16; (tile_width * tiles) as usize * samples];
        let row_samples = tile_width as usize * samples;
        let mut trow = 0u64;
        let mut tcol = 0u64;
        while trow < raw_height {
            let save = self.stream.tell();
            if tile_length < NOT_TILED as u64 {
                let offset = self.stream.get4();
                self.stream.seek(offset as u64);
            }
            let mut row = 0u64;
            while row < tile_length && row + trow < raw_height {
                self.read_samples(&mut pixel[..row_samples]);
                let mut position = 0usize;
                for column in 0..tile_width {
                    self.copy_pixel(target, trow + row, tcol + column, &pixel, &mut position);
                }
                row += 1;
            }
            self.stream.seek(save + 4);
            tcol += tile_width;
            if tcol >= raw_width {
                trow += tile_length;
                tcol = 0;
            }
        }
    }

    /// `deflate_dng_load_raw` and `uncompressed_fp_dng_load_raw`, then
    /// `convertFloatToInt`.
    fn load_floats(&mut self) -> Result<RawData> {
        let raw = self.state.raw_ifd;
        let ifd = self.state.ifds.get(raw).cloned().context("the floating-point DNG raw image is damaged")?;
        let samples = ifd.samples as usize;
        if !matches!(samples, 1 | 3 | 4) || self.state.tiff_samples as usize != samples || (self.state.filters != 0 && samples > 1) {
            bail!("the floating-point DNG raw image is damaged");
        }
        if ifd.sample_format != 3 {
            bail!("only floating-point deflated DNG files are supported");
        }
        let raw_width = self.state.raw_width as usize;
        let raw_height = self.state.raw_height as usize;
        let tiles = TileLayout::new(&ifd, self.state, &mut self.stream)?;
        let mut floats = vec![0.0f32; tiles.count * tiles.width * tiles.height * samples];
        let mut maximum = 0.0f32;
        let data = self.stream.data();

        if self.state.decoder == Decoder::Deflate {
            let factor = match ifd.predictor {
                34894 => 2,
                34895 => 4,
                _ => 1,
            };
            let bytes_per_sample = (ifd.bps >> 3) as usize;
            let tile_pixels = tiles.width * tiles.height;
            let pixel_size = 4 * samples;
            let tile_bytes = tile_pixels * pixel_size;
            let tile_row_bytes = tiles.width * pixel_size;
            let mut buffer = vec![0u8; tile_bytes + tile_row_bytes];
            let mut tile = 0usize;
            for y in (0..raw_height).step_by(tiles.height) {
                for x in (0..raw_width).step_by(tiles.width) {
                    let start = (tiles.offsets[tile].max(0) as usize).min(data.len());
                    let end = (start + tiles.byte_counts[tile].max(0) as usize).min(data.len());
                    tile += 1;
                    let mut inflater = flate2::read::ZlibDecoder::new(&data[start..end]);
                    let mut inflated = Vec::new();
                    inflater.read_to_end(&mut inflated).context("cannot inflate a floating-point DNG tile")?;
                    let length = inflated.len().min(tile_bytes);
                    buffer[tile_row_bytes..tile_row_bytes + length].copy_from_slice(&inflated[..length]);
                    let rows = tiles.height.min(raw_height - y);
                    let columns = tiles.width.min(raw_width - x);
                    for row in 0..rows {
                        let destination = row * tiles.width * bytes_per_sample * samples;
                        let source = destination + tile_row_bytes;
                        decode_fp_delta(&mut buffer, source, destination, tiles.width / factor, samples * factor, bytes_per_sample);
                        let row_maximum = expand_floats(&mut buffer[destination..], tiles.width * samples, bytes_per_sample);
                        maximum = maximum.max(row_maximum);
                        let output = ((y + row) * raw_width + x) * samples;
                        for index in 0..columns * samples {
                            let bytes = &buffer[destination + index * 4..destination + index * 4 + 4];
                            floats[output + index] = f32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                        }
                    }
                }
            }
        } else {
            let bytes_per_sample = ((ifd.bps + 7) >> 3) as usize;
            if !(1..=4).contains(&bytes_per_sample) {
                bail!("the floating-point DNG raw image is damaged");
            }
            let byte_swapped = self.state.order == BIG_ENDIAN;
            let mut row_buffer = vec![0u8; tiles.width * 4 * samples];
            let mut tile = 0usize;
            for y in (0..raw_height).step_by(tiles.height) {
                let mut x = 0usize;
                while x < raw_width && tile < tiles.count {
                    self.stream.seek(tiles.offsets[tile].max(0) as u64);
                    let rows = tiles.height.min(raw_height - y);
                    let columns = tiles.width.min(raw_width - x);
                    let full_row_bytes = tiles.width * bytes_per_sample * samples;
                    for row in 0..rows {
                        row_buffer.fill(0);
                        self.stream.read(&mut row_buffer[..full_row_bytes]);
                        let row_bytes = &mut row_buffer[..full_row_bytes];
                        if bytes_per_sample == 2 && byte_swapped {
                            for pair in row_bytes.as_chunks_mut::<2>().0 {
                                pair.swap(0, 1);
                            }
                        } else if bytes_per_sample == 3 && self.state.order != BIG_ENDIAN {
                            for triple in row_bytes.as_chunks_mut::<3>().0 {
                                triple.swap(0, 2);
                            }
                        }
                        if bytes_per_sample == 4 && byte_swapped {
                            for quad in row_bytes.as_chunks_mut::<4>().0 {
                                quad.reverse();
                            }
                        }
                        let row_maximum = expand_floats(&mut row_buffer, tiles.width * samples, bytes_per_sample);
                        maximum = maximum.max(row_maximum);
                        let output = ((y + row) * raw_width + x) * samples;
                        for index in 0..columns * samples {
                            let bytes = &row_buffer[index * 4..index * 4 + 4];
                            floats[output + index] = f32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                        }
                    }
                    x += tiles.width;
                    tile += 1;
                }
            }
        }

        // convertFloatToInt(4096, 32767, 16383).
        let state = &mut self.state;
        let mut top = (state.maximum.max(1)) as f32;
        top = top.max(maximum);
        top = top.max(1.0);
        let mut multiplier = 1.0f32;
        if !(4096.0..=32767.0).contains(&top) {
            multiplier = 16383.0 / top;
            state.maximum = 16383;
            state.black = (state.black as f32 * multiplier) as u32;
            for index in 0..CBLACK_SIZE {
                if index != 4 && index != 5 {
                    state.cblack[index] = (state.cblack[index] as f32 * multiplier) as u32;
                }
            }
        }
        let count = raw_height * raw_width * samples;
        let integers: Vec<u16> = floats[..count].iter().map(|&value| ((if value > 0.0 { value } else { 0.0 }) * multiplier) as i32 as u16).collect();
        Ok(match samples {
            1 => RawData::Bayer(integers),
            3 => RawData::Color3(integers.as_chunks::<3>().0.to_vec()),
            _ => RawData::Color4(integers.as_chunks::<4>().0.to_vec()),
        })
    }
}

/// `tile_stripe_data_t`: the tiles or strips of a floating-point image.
struct TileLayout {
    width: usize,
    height: usize,
    count: usize,
    offsets: Vec<i64>,
    byte_counts: Vec<i64>,
}

impl TileLayout {
    fn new(ifd: &super::identify::Ifd, state: &Raw, stream: &mut Stream) -> Result<TileLayout> {
        let raw_width = state.raw_width;
        let raw_height = state.raw_height;
        let tiled = state.tile_width <= raw_width && state.tile_length <= raw_height;
        let rows_per_strip = ifd.rows_per_strip.max(0) as u32;
        let striped = rows_per_strip > 0 && rows_per_strip < raw_height && !ifd.strip_byte_counts.is_empty();
        let width = if tiled { state.tile_width } else { raw_width } as usize;
        let height = if tiled {
            state.tile_length
        } else if striped {
            rows_per_strip
        } else {
            raw_height
        } as usize;
        let across = if tiled { (raw_width as usize).div_ceil(width) } else { 1 };
        let down = if tiled {
            (raw_height as usize).div_ceil(height)
        } else if striped {
            (raw_height as usize).div_ceil(rows_per_strip as usize)
        } else {
            1
        };
        let count = across * down;
        if !(1..=1_000_000).contains(&count) {
            bail!("the floating-point DNG raw image is damaged");
        }
        let mut offsets = vec![0i64; count];
        let mut byte_counts = vec![0i64; count];
        if tiled {
            for offset in offsets.iter_mut() {
                *offset = stream.get4() as i64;
            }
        } else if striped {
            for (offset, &strip) in offsets.iter_mut().zip(&ifd.strip_offsets) {
                *offset = strip;
            }
        } else {
            offsets[0] = ifd.offset;
        }
        if count == 1 || (!tiled && !striped) {
            byte_counts[0] = ifd.bytes;
        } else if tiled {
            stream.seek(ifd.bytes.max(0) as u64);
            for byte_count in byte_counts.iter_mut() {
                *byte_count = stream.get4() as i64;
            }
        } else {
            for (byte_count, &strip) in byte_counts.iter_mut().zip(&ifd.strip_byte_counts) {
                *byte_count = strip;
            }
        }
        Ok(TileLayout { width, height, count, offsets, byte_counts })
    }
}

/// `DecodeDeltaBytes`: undoes the byte-wise horizontal differencing.
fn decode_delta_bytes(bytes: &mut [u8], columns: usize, channels: usize) {
    for column in 1..columns {
        for channel in 0..channels {
            let index = column * channels + channel;
            bytes[index] = bytes[index].wrapping_add(bytes[index - channels]);
        }
    }
}

/// `DecodeFPDelta`: the floating-point predictor (34894/34895) from the
/// row at `source` into the row at `destination` (on little-endian hosts).
fn decode_fp_delta(buffer: &mut [u8], source: usize, destination: usize, columns: usize, channels: usize, bytes_per_sample: usize) {
    let row_increment = columns * channels;
    decode_delta_bytes(&mut buffer[source..source + row_increment * bytes_per_sample], columns * bytes_per_sample, channels);
    let input = buffer[source..source + row_increment * bytes_per_sample].to_vec();
    let output = &mut buffer[destination..destination + row_increment * bytes_per_sample];
    for column in 0..row_increment {
        match bytes_per_sample {
            2 => {
                output[column * 2] = input[row_increment + column];
                output[column * 2 + 1] = input[column];
            }
            3 => {
                output[column * 3] = input[column];
                output[column * 3 + 1] = input[row_increment + column];
                output[column * 3 + 2] = input[row_increment * 2 + column];
            }
            _ => {
                output[column * 4] = input[row_increment * 3 + column];
                output[column * 4 + 1] = input[row_increment * 2 + column];
                output[column * 4 + 2] = input[row_increment + column];
                output[column * 4 + 3] = input[column];
            }
        }
    }
}

/// `__DNG_HalfToFloat`.
fn half_to_float(half: u16) -> u32 {
    let sign = ((half >> 15) & 1) as u32;
    let mut exponent = ((half >> 10) & 0x1f) as i32;
    let mut mantissa = (half & 0x3ff) as u32;
    if exponent == 0 {
        if mantissa == 0 {
            return sign << 31;
        }
        while mantissa & 0x400 == 0 {
            mantissa <<= 1;
            exponent -= 1;
        }
        exponent += 1;
        mantissa &= !0x400;
    } else if exponent == 31 {
        if mantissa == 0 {
            return (sign << 31) | ((0x1e + 127 - 15) << 23) | (0x3ff << 13);
        }
        return 0;
    }
    exponent += 127 - 15;
    mantissa <<= 13;
    (sign << 31) | ((exponent as u32) << 23) | mantissa
}

/// `__DNG_FP24ToFloat`.
fn fp24_to_float(bytes: &[u8]) -> u32 {
    let sign = ((bytes[0] >> 7) & 1) as u32;
    let mut exponent = (bytes[0] & 0x7f) as i32;
    let mut mantissa = (bytes[1] as u32) << 8 | bytes[2] as u32;
    if exponent == 0 {
        if mantissa == 0 {
            return sign << 31;
        }
        while mantissa & 0x10000 == 0 {
            mantissa <<= 1;
            exponent -= 1;
        }
        exponent += 1;
        mantissa &= !0x10000;
    } else if exponent == 127 {
        if mantissa == 0 {
            return (sign << 31) | ((0x7e + 128 - 64) << 23) | (0xffff << 7);
        }
        return 0;
    }
    exponent += 128 - 64;
    mantissa <<= 7;
    (sign << 31) | ((exponent as u32) << 23) | mantissa
}

/// `expandFloats`: 16- and 24-bit floats to 32-bit in place (from the
/// end); returns the largest value.
fn expand_floats(bytes: &mut [u8], count: usize, bytes_per_sample: usize) -> f32 {
    let mut maximum = 0.0f32;
    match bytes_per_sample {
        2 => {
            for index in (0..count).rev() {
                let half = u16::from_ne_bytes([bytes[index * 2], bytes[index * 2 + 1]]);
                let value = half_to_float(half);
                bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_ne_bytes());
                let float = f32::from_bits(value);
                if float > maximum {
                    maximum = float;
                }
            }
        }
        3 => {
            for index in (0..count).rev() {
                let value = fp24_to_float(&bytes[index * 3..index * 3 + 3]);
                bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_ne_bytes());
                let float = f32::from_bits(value);
                if float > maximum {
                    maximum = float;
                }
            }
        }
        _ => {
            for index in 0..count {
                let float = f32::from_ne_bytes([bytes[index * 4], bytes[index * 4 + 1], bytes[index * 4 + 2], bytes[index * 4 + 3]]);
                if float > maximum {
                    maximum = float;
                }
            }
        }
    }
    maximum
}

/// `crop_masked_pixels`: black levels from MaskedAreas.
fn crop_masked_pixels(state: &mut Raw, raw_image: &[u16]) {
    if state.mask[0][3] <= 0 {
        return;
    }
    let raw_width = state.raw_width as i32;
    let raw_height = state.raw_height as i32;
    let mut sums = [0u32; 8];
    let mut zero = 0u32;
    for area in state.mask {
        for row in area[0].max(0)..area[2].min(raw_height) {
            for column in area[1].max(0)..area[3].min(raw_width) {
                let color = fc(state.filters, row as u32, column as u32) as usize;
                let value = raw_image[(row * raw_width + column) as usize] as u32;
                sums[color] = sums[color].wrapping_add(value);
                sums[4 + color] += 1;
                if value == 0 {
                    zero += 1;
                }
            }
        }
    }
    if zero < sums[4] && sums[5] != 0 && sums[6] != 0 && sums[7] != 0 {
        for color in 0..4 {
            state.cblack[color] = sums[color] / sums[4 + color].max(1);
        }
        state.black = 0;
        state.cblack[4] = 0;
        state.cblack[5] = 0;
        state.cblack[6] = 0;
    }
}
