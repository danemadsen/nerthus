//! DWAA and DWAB, OpenEXR's lossy DCT compression, decoded as OpenEXRCore
//! 3.4.9 decodes them in its Windows build (`internal_dwa*.h`).
//!
//! A chunk starts with eleven little-endian counters, then (version 2) the
//! channel rules that pick each channel's scheme, then four sections: the
//! UNKNOWN channels deflated, the lossy channels' AC coefficients (Huffman
//! coded or deflated), their DC coefficients (deflated, with the ZIP
//! predictor) and the RLE channels (deflated run lengths). Lossy channels
//! are rebuilt 8 × 8 block by block: the AC run lengths undone, an inverse
//! DCT, R, G and B of one layer taken back from Y'CbCr, the floats rounded
//! to halves and mapped back to linear light through a table.
//!
//! MSVC builds can't run `xgetbv`, so OpenEXR finds no AVX there and takes
//! its SSE2 inverse DCT and colour transform, with scalar half conversions.
//! Those are what is ported here (the SSE2 DCT differs from the scalar one
//! in its constants and summation order). Huffman streams go through the
//! classic decoder, which gives what OpenEXR's fast decoder gives for any
//! valid stream.

use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use flate2::{Decompress, FlushDecompress, Status};

/// `EXR_PIXEL_UINT`, `EXR_PIXEL_HALF` and `EXR_PIXEL_FLOAT`.
pub const PIXEL_UINT: u8 = 0;
pub const PIXEL_HALF: u8 = 1;
pub const PIXEL_FLOAT: u8 = 2;

/// One channel of the chunk (`exr_coding_channel_info_t`): its samples in
/// this chunk.
pub struct Channel<'a> {
    pub name: &'a [u8],
    pub pixel_type: u8,
    pub width: usize,
    pub height: usize,
    pub x_samples: i32,
    pub y_samples: i32,
    /// `pLinear`: the channel is stored without the nonlinear transfer.
    pub p_linear: bool,
}

impl Channel<'_> {
    fn bytes_per_element(&self) -> usize {
        if self.pixel_type == PIXEL_HALF {
            2
        } else {
            4
        }
    }
}

/// The chunk's first line and its size in the file's pixel space.
pub struct ChunkArea {
    pub start_y: i32,
    pub width: i32,
    pub height: i32,
}

fn corrupt() -> anyhow::Error {
    anyhow!("corrupt DWA chunk")
}

/// Where each `DataSizesSingle` counter sits in the chunk header.
const VERSION: usize = 0;
const UNKNOWN_UNCOMPRESSED_SIZE: usize = 1;
const UNKNOWN_COMPRESSED_SIZE: usize = 2;
const AC_COMPRESSED_SIZE: usize = 3;
const DC_COMPRESSED_SIZE: usize = 4;
const RLE_COMPRESSED_SIZE: usize = 5;
const RLE_UNCOMPRESSED_SIZE: usize = 6;
const RLE_RAW_SIZE: usize = 7;
const AC_UNCOMPRESSED_COUNT: usize = 8;
const DC_UNCOMPRESSED_COUNT: usize = 9;
const AC_COMPRESSION: usize = 10;
const NUM_SIZES_SINGLE: usize = 11;

/// The AC compressions (`AcCompression`).
const STATIC_HUFFMAN: u64 = 0;
const DEFLATE: u64 = 1;

/// A channel's compression (`CompressorScheme`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scheme {
    Unknown,
    LossyDct,
    Rle,
}

/// One channel rule (`Classifier`): channels whose name ends in `suffix`
/// (after the last `.`) and of `pixel_type` get `scheme`, and a CSC index
/// places them as R, G or B of their layer.
struct Rule {
    suffix: Vec<u8>,
    scheme: Scheme,
    pixel_type: u8,
    csc_index: i32,
    case_insensitive: bool,
}

/// `sLegacyChannelRules`, for chunks before version 2: suffix, scheme and
/// CSC index, for half and float channels alike (alpha for unsigned ints
/// too), matched in any case.
const LEGACY_RULES: [(&str, Scheme, i32); 12] = [
    ("r", Scheme::LossyDct, 0),
    ("red", Scheme::LossyDct, 0),
    ("g", Scheme::LossyDct, 1),
    ("grn", Scheme::LossyDct, 1),
    ("green", Scheme::LossyDct, 1),
    ("b", Scheme::LossyDct, 2),
    ("blu", Scheme::LossyDct, 2),
    ("blue", Scheme::LossyDct, 2),
    ("y", Scheme::LossyDct, -1),
    ("by", Scheme::LossyDct, -1),
    ("ry", Scheme::LossyDct, -1),
    ("a", Scheme::Rle, -1),
];

fn legacy_rules() -> Vec<Rule> {
    let mut rules = Vec::new();
    for (suffix, scheme, csc_index) in LEGACY_RULES {
        let pixel_types: &[u8] = if scheme == Scheme::Rle { &[PIXEL_UINT, PIXEL_HALF, PIXEL_FLOAT] } else { &[PIXEL_HALF, PIXEL_FLOAT] };
        for &pixel_type in pixel_types {
            let rule = Rule { suffix: suffix.as_bytes().to_vec(), scheme, pixel_type, csc_index, case_insensitive: true };
            rules.push(rule);
        }
    }
    rules
}

/// `Classifier_read`: a NUL-terminated suffix of at most 128 bytes, a byte
/// of CSC index, scheme and case flag, and the pixel type.
fn read_rule(bytes: &[u8], position: &mut usize) -> Result<Rule> {
    let rest = &bytes[*position..];
    let size = rest.len();
    if size <= 3 {
        return Err(corrupt());
    }
    let mut length = 0;
    while length < 129 {
        if length > size - 3 {
            return Err(corrupt());
        }
        if rest[length] == 0 {
            break;
        }
        length += 1;
    }
    if length == 129 {
        return Err(corrupt());
    }
    let suffix = rest[..length].to_vec();
    let length = length + 1;
    if size < length + 2 {
        return Err(corrupt());
    }
    let value = rest[length];
    let pixel_type = rest[length + 1];
    *position += length + 2;

    let csc_index = (value >> 4) as i32 - 1;
    if !(-1..3).contains(&csc_index) {
        return Err(corrupt());
    }
    let scheme = match (value >> 2) & 3 {
        0 => Scheme::Unknown,
        1 => Scheme::LossyDct,
        2 => Scheme::Rle,
        _ => return Err(corrupt()),
    };
    if pixel_type >= 3 {
        return Err(corrupt());
    }
    let case_insensitive = value & 1 != 0;
    Ok(Rule { suffix, scheme, pixel_type, csc_index, case_insensitive })
}

/// `DwaCompressor_readChannelRules`: a little-endian size (counting
/// itself) and the rules.
fn read_channel_rules(input: &[u8], position: &mut usize) -> Result<Vec<Rule>> {
    let left = input.len() - *position;
    if left <= 2 {
        return Err(corrupt());
    }
    let rule_size = u16::from_le_bytes([input[*position], input[*position + 1]]) as usize;
    if rule_size < 2 || left < rule_size {
        return Err(corrupt());
    }
    let rule_bytes = &input[*position + 2..*position + rule_size];
    *position += rule_size;
    let mut rules = Vec::new();
    let mut rule_position = 0;
    while rule_position < rule_bytes.len() {
        let rule = read_rule(rule_bytes, &mut rule_position)?;
        rules.push(rule);
    }
    Ok(rules)
}

/// `Classifier_match`.
fn rule_matches(rule: &Rule, suffix: &[u8], pixel_type: u8) -> bool {
    if rule.pixel_type != pixel_type {
        return false;
    }
    if rule.case_insensitive {
        suffix.eq_ignore_ascii_case(&rule.suffix)
    } else {
        suffix == rule.suffix.as_slice()
    }
}

/// `DwaCompressor_classifyChannels`: each channel's scheme (the last
/// matching rule wins), and the layers whose R, G and B are all present
/// and sampled alike, coded together in Y'CbCr.
fn classify_channels(channels: &[Channel], rules: &[Rule]) -> (Vec<Scheme>, Vec<[usize; 3]>) {
    let mut schemes = vec![Scheme::Unknown; channels.len()];
    // The layers by name prefix (up to and including the last `.`), with
    // the channel holding each CSC index.
    let mut layers: Vec<(&[u8], [Option<usize>; 3])> = Vec::new();
    for (index, channel) in channels.iter().enumerate() {
        let suffix_start = channel.name.iter().rposition(|&byte| byte == b'.').map_or(0, |dot| dot + 1);
        let prefix = &channel.name[..suffix_start];
        let suffix = &channel.name[suffix_start..];
        let layer = match layers.iter().position(|(known, _)| *known == prefix) {
            Some(layer) => layer,
            None => {
                layers.push((prefix, [None; 3]));
                layers.len() - 1
            }
        };
        for rule in rules {
            if !rule_matches(rule, suffix, channel.pixel_type) {
                continue;
            }
            schemes[index] = rule.scheme;
            if rule.csc_index >= 0 {
                layers[layer].1[rule.csc_index as usize] = Some(index);
            }
        }
    }
    let mut csc_sets = Vec::new();
    for (_, members) in &layers {
        let [Some(red), Some(green), Some(blue)] = *members else {
            continue;
        };
        let (red_channel, green_channel, blue_channel) = (&channels[red], &channels[green], &channels[blue]);
        let same_x = red_channel.x_samples == green_channel.x_samples && red_channel.x_samples == blue_channel.x_samples;
        let same_y = red_channel.y_samples == green_channel.y_samples && red_channel.y_samples == blue_channel.y_samples;
        if same_x && same_y {
            csc_sets.push([red, green, blue]);
        }
    }
    (schemes, csc_sets)
}

/// `exr_compress_max_buffer_size`: the largest a deflated buffer of
/// `size` bytes may be (libdeflate's bound, padded), and room for the
/// Huffman coder's tables.
fn compress_max_buffer_size(size: u64) -> u64 {
    const HUF_ENCODING_SIZE: u64 = (1 << 16) + 1;
    const HUF_COMPRESS_SPARE: u64 = HUF_ENCODING_SIZE * 8 * 3 + HUF_ENCODING_SIZE * 4;
    let blocks = size.div_ceil(5000).max(1);
    let zlib_bound = 6 + 5 * blocks + size;
    let padded = zlib_bound + 9;
    let mut bound = padded.max(padded * 130 / 128);
    bound = bound.max(size + HUF_COMPRESS_SPARE);
    bound.max(8192)
}

/// `exr_uncompress_buffer`: a zlib stream that must fill exactly
/// `expected` bytes and use up its input.
fn inflate_exact(input: &[u8], expected: usize) -> Result<Vec<u8>> {
    let mut output = vec![0u8; expected];
    let mut inflater = Decompress::new(true);
    let status = inflater.decompress(input, &mut output, FlushDecompress::Finish);
    if !matches!(status, Ok(Status::StreamEnd)) {
        return Err(corrupt());
    }
    let whole_input = inflater.total_in() as usize == input.len();
    let whole_output = inflater.total_out() as usize == expected;
    if !whole_input || !whole_output {
        return Err(corrupt());
    }
    Ok(output)
}

/// `internal_zip_reconstruct_bytes`: the predictor undone, then the two
/// halves interleaved.
fn reconstruct_bytes(mut bytes: Vec<u8>) -> Vec<u8> {
    for index in 1..bytes.len() {
        let value = bytes[index - 1] as i32 + bytes[index] as i32 - 128;
        bytes[index] = value as u8;
    }
    let (first, second) = bytes.split_at(bytes.len().div_ceil(2));
    let mut interleaved = Vec::with_capacity(bytes.len());
    for index in 0..bytes.len() {
        let source = if index % 2 == 0 { first[index / 2] } else { second[index / 2] };
        interleaved.push(source);
    }
    interleaved
}

/// `internal_rle_decompress`: a negative count copies that many bytes, a
/// positive one repeats the next byte count + 1 times. Returns how many
/// bytes came out, 0 when the data runs over either buffer.
fn rle_decompress(output: &mut [u8], input: &[u8]) -> usize {
    let mut read = 0usize;
    let mut written = 0usize;
    while read < input.len() {
        let count = input[read] as i8;
        read += 1;
        if count < 0 {
            let length = (-(count as i32)) as usize;
            if read + length > input.len() || written + length > output.len() {
                return 0;
            }
            output[written..written + length].copy_from_slice(&input[read..read + length]);
            read += length;
            written += length;
        } else {
            if read + 1 > input.len() {
                return 0;
            }
            let length = count as usize + 1;
            if written + length > output.len() {
                return 0;
            }
            output[written..written + length].fill(input[read]);
            read += 1;
            written += length;
        }
    }
    written
}

/// `half_to_float`.
fn half_to_float(half: u16) -> f32 {
    let mut exponent_mantissa = ((half as u32) << 17) >> 4;
    let mut bits = ((half >> 15) as u32) << 31;
    if exponent_mantissa >= 0x0080_0000 {
        bits |= exponent_mantissa;
        if exponent_mantissa < 0x0f80_0000 {
            bits += 0x3800_0000;
        } else {
            bits |= 0x7f80_0000;
        }
    } else if exponent_mantissa != 0 {
        let shift = exponent_mantissa.leading_zeros() - 8;
        bits |= 0x3880_0000;
        exponent_mantissa <<= shift;
        bits |= exponent_mantissa;
        bits -= shift << 23;
    }
    f32::from_bits(bits)
}

/// `float_to_half`: rounded to nearest even, values below the smallest
/// half denormal's half flushed to zero.
fn float_to_half(value: f32) -> u16 {
    let bits = value.to_bits();
    let mut magnitude = bits & !0x8000_0000;
    let mut half = ((bits >> 16) & 0x8000) as u16;
    if magnitude >= 0x3880_0000 {
        if magnitude >= 0x7f80_0000 {
            half |= 0x7c00;
            if magnitude == 0x7f80_0000 {
                return half;
            }
            let mantissa = (magnitude & 0x7f_ffff) >> 13;
            return half | mantissa as u16 | u16::from(mantissa == 0);
        }
        if magnitude > 0x477f_efff {
            return half | 0x7c00;
        }
        magnitude -= 0x3800_0000;
        magnitude = (magnitude + 0x0000_0fff + ((magnitude >> 13) & 1)) >> 13;
        return half | magnitude as u16;
    }
    if magnitude < 0x3300_0001 {
        return half;
    }
    let exponent = magnitude >> 23;
    let shift = 0x7e - exponent;
    let mantissa = 0x80_0000 | (magnitude & 0x7f_ffff);
    let remainder = mantissa << (32 - shift);
    half |= (mantissa >> shift) as u16;
    if remainder > 0x8000_0000 || (remainder == 0x8000_0000 && (half & 1) != 0) {
        half += 1;
    }
    half
}

/// `dwa_convertToLinear`: the nonlinear encoding undone — a 2.2 power up
/// to 1, an exponential above. (`powf` to the nearest float; every C
/// library's `powf` gives the same halves here.)
fn to_linear(half: u16) -> u16 {
    if half == 0 || (half & 0x7c00) == 0x7c00 {
        return 0;
    }
    let value = half_to_float(half);
    let sign = if value < 0.0 { -1.0f32 } else { 1.0 };
    let magnitude = value.abs();
    let (base, exponent) = if magnitude <= 1.0 { (magnitude, 2.2f32) } else { (9.025_013f32, magnitude - 1.0) };
    let power = (base as f64).powf(exponent as f64) as f32;
    float_to_half(sign * power)
}

/// `exrcore_dwaToLinearTable`.
fn to_linear_table() -> &'static [u16] {
    static TABLE: OnceLock<Vec<u16>> = OnceLock::new();
    TABLE.get_or_init(|| (0..=u16::MAX).map(to_linear).collect())
}

/// The zig-zag order: where each coefficient of the block, row by row,
/// sits in the coded order.
const ZIG_ZAG: [usize; 64] = [
    0, 1, 5, 6, 14, 15, 27, 28, 2, 4, 7, 13, 16, 26, 29, 42, 3, 8, 12, 17, 25, 30, 41, 43, 9, 11, 18, 24, 31, 40, 44, 53, 10, 19, 23, 32, 39, 45, 52, 54, 20, 22, 33, 38, 46, 51, 55, 60, 21, 34, 37, 47, 50,
    56, 59, 61, 35, 36, 48, 49, 57, 58, 62, 63,
];

/// The inverse DCT's constants, ½·cos(kπ/16), as the SSE2 code writes them.
const A: f32 = 3.535536e-01;
const B: f32 = 4.903927e-01;
const C: f32 = 4.619398e-01;
const D: f32 = 4.157349e-01;
const E: f32 = 2.777855e-01;
const F: f32 = 1.913422e-01;
const G: f32 = 9.754573e-02;

/// The columns of the row pass's matrices: M1's for the even inputs, M2's
/// for the odd ones.
const ROW_EVEN: [[f32; 4]; 4] = [[A, A, A, A], [C, F, -F, -C], [A, -A, -A, A], [F, -C, C, -F]];
const ROW_ODD: [[f32; 4]; 4] = [[B, D, E, G], [D, -G, -B, -E], [E, -B, G, D], [G, -E, D, -B]];

/// `dctInverse8x8_sse2`: rows as matrix products summed in order (the
/// last `zeroed_rows` rows, all zero, left alone), then columns through
/// the shared sub-expressions.
fn dct_inverse_8x8(data: &mut [f32; 64], zeroed_rows: usize) {
    for row in 0..8 - zeroed_rows {
        let input: [f32; 8] = data[row * 8..row * 8 + 8].try_into().unwrap();
        let mut even = [0.0f32; 4];
        let mut odd = [0.0f32; 4];
        for lane in 0..4 {
            let mut even_sum = 0.0f32;
            let mut odd_sum = 0.0f32;
            for term in 0..4 {
                even_sum += input[2 * term] * ROW_EVEN[term][lane];
                odd_sum += input[2 * term + 1] * ROW_ODD[term][lane];
            }
            even[lane] = even_sum;
            odd[lane] = odd_sum;
        }
        for lane in 0..4 {
            data[row * 8 + lane] = even[lane] + odd[lane];
            data[row * 8 + 7 - lane] = even[lane] - odd[lane];
        }
    }
    for column in 0..8 {
        let input: [f32; 8] = std::array::from_fn(|row| data[row * 8 + column]);
        let alpha = [C * input[2], F * input[2], C * input[6], F * input[6]];
        let beta = [
            (input[1] * B + input[3] * D) + (input[5] * E + input[7] * G),
            (input[1] * D - input[3] * G) - (input[5] * B + input[7] * E),
            (input[1] * E - input[3] * B) + (input[5] * G + input[7] * D),
            (input[1] * G - input[3] * E) + (input[5] * D - input[7] * B),
        ];
        let theta = [A * (input[0] + input[4]), alpha[0] + alpha[3], alpha[1] - alpha[2], A * (input[0] - input[4])];
        let gamma = [theta[0] + theta[1], theta[3] + theta[2], theta[3] - theta[2], theta[0] - theta[1]];
        for index in 0..4 {
            data[index * 8 + column] = gamma[index] + beta[index];
            data[(7 - index) * 8 + column] = gamma[index] - beta[index];
        }
    }
}

/// `dctInverse8x8DcOnly`.
fn dct_inverse_dc_only(data: &mut [f32; 64]) {
    let value = data[0] * A * A;
    data.fill(value);
}

/// How many trailing rows of the block are zero, judged by the last
/// non-zero coefficient's place in zig-zag order.
fn zeroed_rows(last_non_zero: usize) -> usize {
    match last_non_zero {
        0..2 => 7,
        2 => 6,
        3..9 => 5,
        9 => 4,
        10..20 => 3,
        20 => 2,
        21..35 => 1,
        _ => 0,
    }
}

/// `csc709Inverse64` (SSE2): Y'CbCr back to R'G'B' for a block.
fn csc_709_inverse_block(red: &mut [f32; 64], green: &mut [f32; 64], blue: &mut [f32; 64]) {
    for index in 0..64 {
        let luma = red[index];
        let blue_difference = green[index];
        let red_difference = blue[index];
        red[index] = luma + red_difference * 1.5747;
        green[index] = (blue_difference * -0.1873 + luma) + red_difference * -0.4682;
        blue[index] = 1.8556 * blue_difference + luma;
    }
}

/// `csc709Inverse`: the same for one value, as the scalar code writes it.
fn csc_709_inverse(red: &mut f32, green: &mut f32, blue: &mut f32) {
    let (luma, blue_difference, red_difference) = (*red, *green, *blue);
    *red = luma + 1.5747 * red_difference;
    *green = luma - 0.1873 * blue_difference - 0.4682 * red_difference;
    *blue = luma + 1.8556 * blue_difference;
}

fn write_u16(output: &mut [u8], offset: usize, value: u16) -> Result<()> {
    let destination = output.get_mut(offset..offset + 2).ok_or_else(corrupt)?;
    destination.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

/// One lossy channel decoded into the output: its row starts and type.
struct LossyComponent<'a> {
    rows: &'a [usize],
    pixel_type: u8,
}

/// `LossyDctDecoder`: decodes one channel, or the three of a CSC set, from
/// the AC and DC values left.
struct LossyDctDecoder<'a> {
    ac: &'a [u16],
    dc: &'a [u16],
    to_linear: Option<&'a [u16]>,
    width: usize,
    height: usize,
    ac_count: usize,
    dc_count: usize,
}

impl LossyDctDecoder<'_> {
    /// `LossyDctDecoder_unRleAc`: the block's AC values, `0xffNN` being a
    /// run of NN zeros (0: to the end). Returns the zig-zag index of the
    /// last non-zero value.
    fn un_rle_ac(&mut self, position: &mut usize, block: &mut [u16; 64]) -> Result<usize> {
        let mut component = 1usize;
        let mut last_non_zero = 0;
        let mut count = 0;
        while component < 64 {
            let value = *self.ac.get(*position).ok_or_else(corrupt)?;
            if value & 0xff00 == 0xff00 {
                let run = (value & 0xff) as usize;
                component += if run == 0 { 64 } else { run };
            } else {
                last_non_zero = component;
                block[component] = value;
                component += 1;
            }
            count += 1;
            *position += 1;
        }
        self.ac_count += count;
        Ok(last_non_zero)
    }

    /// `LossyDctDecoder_execute`.
    fn execute(&mut self, output: &mut [u8], components: &[LossyComponent]) -> Result<()> {
        let component_count = components.len();
        let blocks_x = self.width.div_ceil(8);
        let blocks_y = self.height.div_ceil(8);
        let leftover_x = self.width as i64 - (blocks_x as i64 - 1) * 8;
        let leftover_y = self.height as i64 - (blocks_y as i64 - 1) * 8;
        let full_blocks_x = self.width / 8;
        if self.dc.len() < component_count * blocks_x * blocks_y {
            return Err(corrupt());
        }

        // A row of blocks of halves per component, and where each
        // component's DC values start (each plane packed together).
        let mut row_blocks = vec![vec![0u16; blocks_x * 64]; component_count];
        let mut dc_positions: Vec<usize> = (0..component_count).map(|component| component * blocks_x * blocks_y).collect();
        let mut ac_position = 0usize;
        let mut half_zig = [[0u16; 64]; 3];
        let mut dct = [[0.0f32; 64]; 3];

        for block_y in 0..blocks_y {
            let max_y = if block_y == blocks_y - 1 { leftover_y as usize } else { 8 };
            let mut max_x = 8;
            for block_x in 0..blocks_x {
                if block_x == blocks_x - 1 {
                    max_x = leftover_x as usize;
                }
                // A block whose components hold only DC values is
                // constant: only its first value is worked on.
                let mut constant = true;
                for component in 0..component_count {
                    half_zig[component] = [0; 64];
                    half_zig[component][0] = self.dc[dc_positions[component]];
                    dc_positions[component] += 1;
                    self.dc_count += 1;
                    let last_non_zero = self.un_rle_ac(&mut ac_position, &mut half_zig[component])?;
                    if last_non_zero == 0 {
                        dct[component][0] = half_to_float(half_zig[component][0]);
                        dct_inverse_dc_only(&mut dct[component]);
                    } else {
                        constant = false;
                        for (value, &source) in dct[component].iter_mut().zip(ZIG_ZAG.iter()) {
                            *value = half_to_float(half_zig[component][source]);
                        }
                        dct_inverse_8x8(&mut dct[component], zeroed_rows(last_non_zero));
                    }
                }
                if component_count == 3 {
                    let [red, green, blue] = &mut dct;
                    if constant {
                        csc_709_inverse(&mut red[0], &mut green[0], &mut blue[0]);
                    } else {
                        csc_709_inverse_block(red, green, blue);
                    }
                }
                for component in 0..component_count {
                    let block = &mut row_blocks[component][block_x * 64..block_x * 64 + 64];
                    if constant {
                        let value = float_to_half(dct[component][0]);
                        block.fill(value);
                    } else {
                        for (half, &value) in block.iter_mut().zip(dct[component].iter()) {
                            *half = float_to_half(value);
                        }
                    }
                }
            }

            // Unblock into the rows, taking the halves back to linear.
            for (component, target) in components.iter().enumerate() {
                for y in 8 * block_y..8 * block_y + max_y {
                    let row_start = *target.rows.get(y).ok_or_else(corrupt)?;
                    let blocks = &row_blocks[component];
                    for block_x in 0..blocks_x {
                        let count = if block_x < full_blocks_x { 8 } else { max_x };
                        let source = &blocks[block_x * 64 + (y & 7) * 8..][..count];
                        for (offset, &half) in source.iter().enumerate() {
                            let value = match self.to_linear {
                                Some(table) => table[half as usize],
                                None => half,
                            };
                            let x = block_x * 8 + offset;
                            write_u16(output, row_start + x * 2, value)?;
                        }
                    }
                }
            }
        }

        // Float channels: each half widened in place.
        for target in components {
            if target.pixel_type != PIXEL_FLOAT {
                continue;
            }
            for &row_start in target.rows.iter().take(self.height) {
                for x in (0..self.width).rev() {
                    let bytes = output.get(row_start + x * 2..row_start + x * 2 + 2).ok_or_else(corrupt)?;
                    let value = half_to_float(u16::from_le_bytes([bytes[0], bytes[1]]));
                    let destination = output.get_mut(row_start + x * 4..row_start + x * 4 + 4).ok_or_else(corrupt)?;
                    destination.copy_from_slice(&value.to_le_bytes());
                }
            }
        }
        Ok(())
    }
}

/// `DwaCompressor_uncompress`: the chunk `input` decoded into `output_size`
/// bytes laid out as the file's lines (for each line, each channel's
/// samples on it), little-endian.
pub fn decompress(input: &[u8], channels: &[Channel], area: &ChunkArea, output_size: usize) -> Result<Vec<u8>> {
    let header_size = NUM_SIZES_SINGLE * 8;
    if input.len() < header_size {
        return Err(corrupt());
    }
    let mut output = vec![0u8; output_size];
    let mut counters = [0u64; NUM_SIZES_SINGLE];
    for (index, counter) in counters.iter_mut().enumerate() {
        let bytes: [u8; 8] = input[index * 8..index * 8 + 8].try_into().unwrap();
        *counter = u64::from_le_bytes(bytes);
    }
    let version = counters[VERSION];
    let unknown_uncompressed = counters[UNKNOWN_UNCOMPRESSED_SIZE];
    let unknown_compressed = counters[UNKNOWN_COMPRESSED_SIZE];
    let ac_compressed = counters[AC_COMPRESSED_SIZE];
    let dc_compressed = counters[DC_COMPRESSED_SIZE];
    let rle_compressed = counters[RLE_COMPRESSED_SIZE];
    let rle_uncompressed = counters[RLE_UNCOMPRESSED_SIZE];
    let rle_raw = counters[RLE_RAW_SIZE];
    let total_ac = counters[AC_UNCOMPRESSED_COUNT];
    let total_dc = counters[DC_UNCOMPRESSED_COUNT];
    let ac_compression = counters[AC_COMPRESSION];

    let input_size = input.len() as u64;
    let mut compressed_size = unknown_compressed.wrapping_add(ac_compressed);
    compressed_size = compressed_size.wrapping_add(dc_compressed);
    compressed_size = compressed_size.wrapping_add(rle_compressed);
    let section_too_big = [unknown_compressed, ac_compressed, dc_compressed, rle_compressed].iter().any(|&size| input_size < size);
    if input_size < (header_size as u64).wrapping_add(compressed_size) || section_too_big {
        return Err(corrupt());
    }
    let negative = counters[UNKNOWN_UNCOMPRESSED_SIZE..=DC_UNCOMPRESSED_COUNT].iter().any(|&value| (value as i64) < 0);
    if negative {
        return Err(corrupt());
    }

    let mut data_start = header_size;
    let rules = if version < 2 { legacy_rules() } else { read_channel_rules(input, &mut data_start)? };

    // `DwaCompressor_initializeBuffers`: the buffers' sizes, which bound
    // what the counters may claim.
    let (schemes, csc_sets) = classify_channels(channels, &rules);
    let chunk_width = area.width as u64;
    let lines = area.height as u64;
    let block_rows = (area.height as f32 / 8.0).ceil() as u64;
    let block_columns = (area.width as f32 / 8.0).ceil() as u64;
    let max_ac_size = block_rows * block_columns * 63 * 2;
    let max_dc_size = block_rows * block_columns * 2;
    let pixel_count = lines * chunk_width;
    let mut lossy_count = 0u64;
    let mut rle_buffer_size = 0u64;
    let mut planar_unknown_size = 0u64;
    for (channel, scheme) in channels.iter().zip(&schemes) {
        let bytes = channel.bytes_per_element() as u64;
        match scheme {
            Scheme::LossyDct => lossy_count += 1,
            Scheme::Rle => rle_buffer_size += 2 * pixel_count * bytes,
            Scheme::Unknown => planar_unknown_size += pixel_count * bytes,
        }
    }
    let planar_rle_size = rle_buffer_size;
    if planar_unknown_size > 0 {
        planar_unknown_size = compress_max_buffer_size(planar_unknown_size);
    }
    let packed_ac_size = max_ac_size * lossy_count;
    let packed_dc_size = max_dc_size * lossy_count;

    // The sections, one after another.
    let unknown_start = data_start;
    let ac_start = unknown_start.checked_add(unknown_compressed as usize).ok_or_else(corrupt)?;
    let dc_start = ac_start.checked_add(ac_compressed as usize).ok_or_else(corrupt)?;
    let rle_start = dc_start.checked_add(dc_compressed as usize).ok_or_else(corrupt)?;
    let rle_end = rle_start.checked_add(rle_compressed as usize).ok_or_else(corrupt)?;
    if rle_end > input.len() {
        return Err(corrupt());
    }
    if version > 2 {
        return Err(anyhow!("unsupported DWA chunk version {version}"));
    }

    // `DwaCompressor_setupChannelData`: where each UNKNOWN or RLE channel's
    // planar data starts.
    let mut planar_offsets = vec![0usize; channels.len()];
    let mut next_unknown = 0usize;
    let mut next_rle = 0usize;
    for (index, channel) in channels.iter().enumerate() {
        let size = channel.width * channel.height * channel.bytes_per_element();
        match schemes[index] {
            Scheme::Unknown => {
                planar_offsets[index] = next_unknown;
                next_unknown += size;
            }
            Scheme::Rle => {
                planar_offsets[index] = next_rle;
                next_rle += size;
            }
            Scheme::LossyDct => {}
        }
    }

    let mut planar_unknown = vec![0u8; planar_unknown_size as usize];
    if unknown_compressed > 0 {
        if unknown_uncompressed > planar_unknown_size {
            return Err(corrupt());
        }
        let inflated = inflate_exact(&input[unknown_start..ac_start], unknown_uncompressed as usize)?;
        planar_unknown[..inflated.len()].copy_from_slice(&inflated);
    }

    let mut packed_ac = vec![0u16; (packed_ac_size / 2) as usize];
    if ac_compressed > 0 {
        if packed_ac_size == 0 || total_ac * 2 > packed_ac_size {
            return Err(corrupt());
        }
        let section = &input[ac_start..dc_start];
        match ac_compression {
            STATIC_HUFFMAN => {
                let values = huffman::decompress(section, total_ac as usize)?;
                packed_ac[..values.len()].copy_from_slice(&values);
            }
            DEFLATE => {
                let bytes = inflate_exact(section, total_ac as usize * 2)?;
                for (value, pair) in packed_ac.iter_mut().zip(bytes.as_chunks::<2>().0) {
                    *value = u16::from_le_bytes([pair[0], pair[1]]);
                }
            }
            _ => return Err(corrupt()),
        }
    }

    let mut packed_dc = vec![0u16; (packed_dc_size / 2) as usize];
    if dc_compressed > 0 {
        let byte_count = total_dc * 2;
        if byte_count > packed_dc_size {
            return Err(corrupt());
        }
        let inflated = inflate_exact(&input[dc_start..rle_start], byte_count as usize)?;
        let reconstructed = reconstruct_bytes(inflated);
        for (value, pair) in packed_dc.iter_mut().zip(reconstructed.as_chunks::<2>().0) {
            *value = u16::from_le_bytes([pair[0], pair[1]]);
        }
    } else if total_dc != 0 {
        return Err(corrupt());
    }

    let mut planar_rle = vec![0u8; planar_rle_size as usize];
    if rle_raw > 0 {
        if rle_uncompressed > rle_buffer_size || rle_raw > planar_rle_size {
            return Err(corrupt());
        }
        let runs = inflate_exact(&input[rle_start..rle_end], rle_uncompressed as usize)?;
        let raw = &mut planar_rle[..rle_raw as usize];
        if rle_decompress(raw, &runs) as u64 != rle_raw {
            return Err(corrupt());
        }
    }

    // Each channel's rows in the output: line by line, a channel's row on
    // the lines it samples.
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); channels.len()];
    let mut cursor = 0usize;
    for y in area.start_y..area.start_y + area.height {
        for (index, channel) in channels.iter().enumerate() {
            if channel.y_samples <= 0 {
                return Err(corrupt());
            }
            if y % channel.y_samples != 0 {
                continue;
            }
            rows[index].push(cursor);
            cursor += channel.width * channel.bytes_per_element();
        }
    }

    let table = to_linear_table();
    let mut ac_position = 0usize;
    let mut dc_position = 0usize;
    let mut ac_left = total_ac as usize;
    let mut dc_left = total_dc as usize;
    let mut processed = vec![false; channels.len()];

    for set in &csc_sets {
        if set.iter().any(|&channel| schemes[channel] != Scheme::LossyDct) {
            return Err(corrupt());
        }
        let red = &channels[set[0]];
        let mut decoder = LossyDctDecoder {
            ac: &packed_ac[ac_position..ac_position + ac_left],
            dc: &packed_dc[dc_position..dc_position + dc_left],
            to_linear: Some(table),
            width: red.width,
            height: red.height,
            ac_count: 0,
            dc_count: 0,
        };
        let components: Vec<LossyComponent> = set.iter().map(|&channel| LossyComponent { rows: &rows[channel], pixel_type: channels[channel].pixel_type }).collect();
        let result = decoder.execute(&mut output, &components);
        ac_position += decoder.ac_count;
        dc_position += decoder.dc_count;
        ac_left -= decoder.ac_count;
        dc_left -= decoder.dc_count;
        for &channel in set {
            processed[channel] = true;
        }
        result?;
    }

    for (index, channel) in channels.iter().enumerate() {
        if processed[index] {
            continue;
        }
        if channel.width == 0 || channel.height == 0 {
            continue;
        }
        let bytes = channel.bytes_per_element();
        match schemes[index] {
            Scheme::LossyDct => {
                let to_linear = if channel.p_linear { None } else { Some(table) };
                let mut decoder = LossyDctDecoder {
                    ac: &packed_ac[ac_position..ac_position + ac_left],
                    dc: &packed_dc[dc_position..dc_position + dc_left],
                    to_linear,
                    width: channel.width,
                    height: channel.height,
                    ac_count: 0,
                    dc_count: 0,
                };
                let components = [LossyComponent { rows: &rows[index], pixel_type: channel.pixel_type }];
                let result = decoder.execute(&mut output, &components);
                ac_position += decoder.ac_count;
                dc_position += decoder.dc_count;
                ac_left -= decoder.ac_count;
                dc_left -= decoder.dc_count;
                result?;
            }
            Scheme::Rle => {
                // The planes hold each byte of the samples in turn.
                let plane_size = channel.width * channel.height;
                let mut plane_positions: Vec<usize> = (0..bytes).map(|byte| planar_offsets[index] + byte * plane_size).collect();
                for &row_start in &rows[index] {
                    for x in 0..channel.width {
                        for (byte, position) in plane_positions.iter_mut().enumerate() {
                            let value = *planar_rle.get(*position).ok_or_else(corrupt)?;
                            let destination = output.get_mut(row_start + x * bytes + byte).ok_or_else(corrupt)?;
                            *destination = value;
                            *position += 1;
                        }
                    }
                }
            }
            Scheme::Unknown => {
                let scanline = channel.width * bytes;
                let mut source = planar_offsets[index];
                for &row_start in &rows[index] {
                    if source + scanline > planar_unknown.len() {
                        return Err(corrupt());
                    }
                    let destination = output.get_mut(row_start..row_start + scanline).ok_or_else(corrupt)?;
                    destination.copy_from_slice(&planar_unknown[source..source + scanline]);
                    source += scanline;
                }
            }
        }
    }
    Ok(output)
}

/// OpenEXR's Huffman coding of 16-bit values (`internal_huf.c`): a header
/// of the symbol range and bit count, the code lengths run-length packed,
/// then the codes, with one symbol standing for runs of the last value.
mod huffman {
    use anyhow::Result;

    use super::corrupt;

    const DECODING_BITS: i32 = 14;
    const ENCODING_SIZE: usize = (1 << 16) + 1;
    const DECODING_SIZE: usize = 1 << DECODING_BITS;
    const DECODING_MASK: u64 = (DECODING_SIZE - 1) as u64;
    const SHORT_ZEROCODE_RUN: u64 = 59;
    const LONG_ZEROCODE_RUN: u64 = 63;
    const SHORTEST_LONG_RUN: u64 = 2 + LONG_ZEROCODE_RUN - SHORT_ZEROCODE_RUN;

    /// A decoding table entry (`HufDec`): a short code's length and
    /// symbol, or the symbols of the long codes sharing this prefix.
    #[derive(Clone, Default)]
    struct Entry {
        length: i32,
        symbol: u32,
        long_codes: Vec<u32>,
    }

    fn read_u32(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
    }

    /// The bit reader's state: bits in `buffer`, `count` of them unread.
    struct Bits<'a> {
        bytes: &'a [u8],
        position: usize,
        buffer: u64,
        count: i32,
    }

    impl Bits<'_> {
        fn take_byte(&mut self) {
            let byte = self.bytes.get(self.position).copied().unwrap_or(0);
            self.buffer = (self.buffer << 8) | byte as u64;
            self.position += 1;
            self.count += 8;
        }

        /// `getBits`.
        fn take(&mut self, bits: i32) -> u64 {
            while self.count < bits {
                self.take_byte();
            }
            self.count -= bits;
            (self.buffer >> self.count) & ((1u64 << bits) - 1)
        }
    }

    /// `hufUnpackEncTable`: each symbol's code length, then the canonical
    /// codes. A table cut short is left as read so far, uncoded (its
    /// caller does not check), with the position unmoved.
    fn unpack_encoding_table(bytes: &[u8], position: &mut usize, left: &mut u64, first: u32, last: u32) -> Vec<u64> {
        let mut codes = vec![0u64; ENCODING_SIZE];
        let mut bits = Bits { bytes, position: *position, buffer: 0, count: 0 };
        let start = *position;
        let mut symbol = first as usize;
        while symbol <= last as usize {
            let read = (bits.position - start) as u64;
            if bits.count < 6 && read >= *left {
                return codes;
            }
            let length = bits.take(6);
            codes[symbol] = length;
            if length == LONG_ZEROCODE_RUN {
                let read = (bits.position - start) as u64;
                if bits.count < 8 && read >= *left {
                    return codes;
                }
                let run = bits.take(8) + SHORTEST_LONG_RUN;
                if symbol as u64 + run > last as u64 + 1 {
                    return codes;
                }
                for _ in 0..run {
                    codes[symbol] = 0;
                    symbol += 1;
                }
                symbol -= 1;
            } else if length >= SHORT_ZEROCODE_RUN {
                let run = length - SHORT_ZEROCODE_RUN + 2;
                if symbol as u64 + run > last as u64 + 1 {
                    return codes;
                }
                for _ in 0..run {
                    codes[symbol] = 0;
                    symbol += 1;
                }
                symbol -= 1;
            }
            symbol += 1;
        }
        *left -= (bits.position - start) as u64;
        *position = bits.position;
        canonical_code_table(&mut codes);
        codes
    }

    /// `hufCanonicalCodeTable`: codes assigned from the lengths, longer
    /// codes numerically lower, each length's codes in symbol order; a
    /// code is stored as `code << 6 | length`.
    fn canonical_code_table(codes: &mut [u64]) {
        let mut counts = [0u64; 59];
        for &length in codes.iter() {
            counts[length as usize] += 1;
        }
        let mut code = 0u64;
        for length in (1..=58).rev() {
            let next = (code + counts[length]) >> 1;
            counts[length] = code;
            code = next;
        }
        for entry in codes.iter_mut() {
            let length = *entry;
            if length > 0 {
                *entry = length | (counts[length as usize] << 6);
                counts[length as usize] += 1;
            }
        }
    }

    /// `hufBuildDecTable`.
    fn build_decoding_table(codes: &[u64], first: u32, last: u32) -> Result<Vec<Entry>> {
        let mut table = vec![Entry::default(); DECODING_SIZE];
        for symbol in first..=last {
            let code = codes[symbol as usize] >> 6;
            let length = (codes[symbol as usize] & 63) as i32;
            if code >> length != 0 {
                return Err(corrupt());
            }
            if length > DECODING_BITS {
                let entry = &mut table[(code >> (length - DECODING_BITS)) as usize];
                if entry.length != 0 {
                    return Err(corrupt());
                }
                entry.long_codes.push(symbol);
            } else if length != 0 {
                let base = (code << (DECODING_BITS - length)) as usize;
                for entry in &mut table[base..base + (1 << (DECODING_BITS - length))] {
                    if entry.length != 0 || !entry.long_codes.is_empty() {
                        return Err(corrupt());
                    }
                    entry.length = length;
                    entry.symbol = symbol;
                }
            }
        }
        Ok(table)
    }

    /// `getCode`: a symbol, or for the run-length symbol a count of copies
    /// of the last value.
    fn put_symbol(symbol: u32, run_symbol: u32, bits: &mut Bits, end: usize, output: &mut Vec<u16>, expected: usize) -> Result<()> {
        if symbol == run_symbol {
            if bits.count < 8 {
                if bits.position >= end {
                    return Err(corrupt());
                }
                bits.take_byte();
            }
            bits.count -= 8;
            let repeats = (bits.buffer >> bits.count) as u8 as usize;
            if output.len() + repeats > expected {
                return Err(corrupt());
            }
            let Some(&last) = output.last() else {
                return Err(corrupt());
            };
            output.extend(std::iter::repeat_n(last, repeats));
        } else if output.len() < expected {
            output.push(symbol as u16);
        } else {
            return Err(corrupt());
        }
        Ok(())
    }

    /// `hufDecode`: `bit_count` bits of codes into `expected` values.
    fn decode(codes: &[u64], table: &[Entry], input: &[u8], bit_count: u64, run_symbol: u32, expected: usize) -> Result<Vec<u16>> {
        let mut output = Vec::with_capacity(expected);
        let end = bit_count.div_ceil(8) as usize;
        let mut bits = Bits { bytes: input, position: 0, buffer: 0, count: 0 };
        while bits.position < end {
            bits.take_byte();
            while bits.count >= DECODING_BITS {
                let entry = &table[((bits.buffer >> (bits.count - DECODING_BITS)) & DECODING_MASK) as usize];
                if entry.length != 0 {
                    if entry.length > bits.count {
                        return Err(corrupt());
                    }
                    bits.count -= entry.length;
                    put_symbol(entry.symbol, run_symbol, &mut bits, end, &mut output, expected)?;
                    continue;
                }
                if entry.long_codes.is_empty() {
                    return Err(corrupt());
                }
                let mut found = false;
                for &symbol in &entry.long_codes {
                    let length = (codes[symbol as usize] & 63) as i32;
                    while bits.count < length && bits.position < end {
                        bits.take_byte();
                    }
                    if bits.count < length {
                        continue;
                    }
                    let code = codes[symbol as usize] >> 6;
                    let mask = (1u64 << length) - 1;
                    if code == (bits.buffer >> (bits.count - length)) & mask {
                        bits.count -= length;
                        put_symbol(symbol, run_symbol, &mut bits, end, &mut output, expected)?;
                        found = true;
                        break;
                    }
                }
                if !found {
                    return Err(corrupt());
                }
            }
        }

        // The last short codes, past the padding bits.
        let padding = (8u64.wrapping_sub(bit_count) & 7) as i32;
        bits.buffer >>= padding;
        bits.count -= padding;
        while bits.count > 0 {
            let entry = &table[((bits.buffer << (DECODING_BITS - bits.count)) & DECODING_MASK) as usize];
            if entry.length == 0 || entry.length > bits.count {
                return Err(corrupt());
            }
            bits.count -= entry.length;
            put_symbol(entry.symbol, run_symbol, &mut bits, end, &mut output, expected)?;
        }
        if output.len() != expected {
            return Err(corrupt());
        }
        Ok(output)
    }

    /// `internal_huf_decompress`: `expected` values from `compressed`.
    pub fn decompress(compressed: &[u8], expected: usize) -> Result<Vec<u16>> {
        if compressed.len() < 20 {
            if expected != 0 {
                return Err(corrupt());
            }
            return Ok(Vec::new());
        }
        let first = read_u32(compressed, 0);
        let last = read_u32(compressed, 4);
        let bit_count = read_u32(compressed, 12) as u64;
        if first as usize >= ENCODING_SIZE || last as usize >= ENCODING_SIZE {
            return Err(corrupt());
        }
        let byte_count = bit_count.div_ceil(8);
        if 20 + byte_count > compressed.len() as u64 {
            return Err(corrupt());
        }
        let mut position = 20;
        let mut left = compressed.len() as u64 - 20;
        let codes = unpack_encoding_table(compressed, &mut position, &mut left, first, last);
        if bit_count > 8 * left {
            return Err(corrupt());
        }
        let table = build_decoding_table(&codes, first, last)?;
        decode(&codes, &table, &compressed[position..], bit_count, last, expected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_round_trip() {
        for bits in 0..=u16::MAX {
            let value = half_to_float(bits);
            let reference = half::f16::from_bits(bits).to_f32();
            if reference.is_nan() {
                assert!(value.is_nan());
                continue;
            }
            assert_eq!(value.to_bits(), reference.to_bits(), "{bits:#x}");
            assert_eq!(float_to_half(value), bits, "{bits:#x}");
        }
    }

    #[test]
    fn floats_round_to_nearest_even_halves() {
        for bits in (0..u32::MAX).step_by(4099) {
            let value = f32::from_bits(bits);
            if value.is_nan() {
                continue;
            }
            let reference = half::f16::from_f32(value).to_bits();
            assert_eq!(float_to_half(value), reference, "{value:e}");
        }
    }

    #[test]
    fn a_dc_only_block_is_flat() {
        let mut block = [0.0f32; 64];
        block[0] = 8.0;
        dct_inverse_dc_only(&mut block);
        assert!(block.iter().all(|&value| value == 8.0 * A * A));
    }
}
