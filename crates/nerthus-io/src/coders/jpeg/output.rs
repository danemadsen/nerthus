//! From coefficients to output samples: the accurate integer IDCT (block
//! smoothing first, for a progressive image missing some refinement),
//! upsampling to full size, and colour conversion.
//!
//! Integer arithmetic follows the library's 32-bit `long`, wrapping where
//! corrupt data overflows it.

use super::{ColorSpace, Decoder};

/// Fixed-point constants of the IDCT (`CONST_BITS` 13).
const FIX_0_298631336: i32 = 2446;
const FIX_0_390180644: i32 = 3196;
const FIX_0_541196100: i32 = 4433;
const FIX_0_765366865: i32 = 6270;
const FIX_0_899976223: i32 = 7373;
const FIX_1_175875602: i32 = 9633;
const FIX_1_501321110: i32 = 12299;
const FIX_1_847759065: i32 = 15137;
const FIX_1_961570560: i32 = 16069;
const FIX_2_053119869: i32 = 16819;
const FIX_2_562915447: i32 = 20995;
const FIX_3_072711026: i32 = 25172;
const CONST_BITS: u32 = 13;

/// A component's samples: `stride` per row.
pub(super) struct Plane {
    pub samples: Vec<u16>,
    pub stride: usize,
}

/// The post-IDCT range limiting: the masked index into the library's
/// table, which clamps to the sample range near it and wraps further out.
struct RangeLimit {
    table: Vec<u16>,
    mask: i32,
}

impl RangeLimit {
    /// `IDCT_range_limit` of `prepare_range_limit_table`.
    fn new(maximum: i32) -> Self {
        let size = maximum + 1;
        let center = size / 2;
        let mut table = vec![0u16; (4 * size) as usize];
        for (index, entry) in table.iter_mut().enumerate() {
            let index = index as i32;
            *entry = if index < size - center {
                (index + center) as u16
            } else if index < 2 * size {
                maximum as u16
            } else if index < 4 * size - center {
                0
            } else {
                (index - (4 * size - center)) as u16
            };
        }
        Self { table, mask: 4 * maximum + 3 }
    }

    fn get(&self, value: i32) -> u16 {
        self.table[(value & self.mask) as usize]
    }
}

fn descale(value: i32, bits: u32) -> i32 {
    value.wrapping_add(1 << (bits - 1)) >> bits
}

/// `jpeg_idct_islow`: one block (`quant` the dequantizing multipliers) into
/// `output` at `stride`.
fn idct_islow(block: &[i16; 64], quant: &[i16; 64], pass1_bits: u32, range: &RangeLimit, output: &mut [u16], stride: usize) {
    let mut workspace = [0i32; 64];
    let dequantize = |index: usize| (block[index] as i32).wrapping_mul(quant[index] as i32);
    for column in 0..8 {
        let ac_zero = (1..8).all(|row| block[row * 8 + column] == 0);
        if ac_zero {
            let dc = dequantize(column).wrapping_shl(pass1_bits);
            for row in 0..8 {
                workspace[row * 8 + column] = dc;
            }
            continue;
        }
        let mut z2 = dequantize(2 * 8 + column);
        let mut z3 = dequantize(6 * 8 + column);
        let mut z1 = z2.wrapping_add(z3).wrapping_mul(FIX_0_541196100);
        let mut tmp2 = z1.wrapping_add(z3.wrapping_mul(-FIX_1_847759065));
        let mut tmp3 = z1.wrapping_add(z2.wrapping_mul(FIX_0_765366865));
        z2 = dequantize(column);
        z3 = dequantize(4 * 8 + column);
        let mut tmp0 = z2.wrapping_add(z3).wrapping_shl(CONST_BITS);
        let mut tmp1 = z2.wrapping_sub(z3).wrapping_shl(CONST_BITS);
        let tmp10 = tmp0.wrapping_add(tmp3);
        let tmp13 = tmp0.wrapping_sub(tmp3);
        let tmp11 = tmp1.wrapping_add(tmp2);
        let tmp12 = tmp1.wrapping_sub(tmp2);

        tmp0 = dequantize(7 * 8 + column);
        tmp1 = dequantize(5 * 8 + column);
        tmp2 = dequantize(3 * 8 + column);
        tmp3 = dequantize(8 + column);
        z1 = tmp0.wrapping_add(tmp3);
        z2 = tmp1.wrapping_add(tmp2);
        z3 = tmp0.wrapping_add(tmp2);
        let mut z4 = tmp1.wrapping_add(tmp3);
        let z5 = z3.wrapping_add(z4).wrapping_mul(FIX_1_175875602);
        tmp0 = tmp0.wrapping_mul(FIX_0_298631336);
        tmp1 = tmp1.wrapping_mul(FIX_2_053119869);
        tmp2 = tmp2.wrapping_mul(FIX_3_072711026);
        tmp3 = tmp3.wrapping_mul(FIX_1_501321110);
        z1 = z1.wrapping_mul(-FIX_0_899976223);
        z2 = z2.wrapping_mul(-FIX_2_562915447);
        z3 = z3.wrapping_mul(-FIX_1_961570560);
        z4 = z4.wrapping_mul(-FIX_0_390180644);
        z3 = z3.wrapping_add(z5);
        z4 = z4.wrapping_add(z5);
        tmp0 = tmp0.wrapping_add(z1.wrapping_add(z3));
        tmp1 = tmp1.wrapping_add(z2.wrapping_add(z4));
        tmp2 = tmp2.wrapping_add(z2.wrapping_add(z3));
        tmp3 = tmp3.wrapping_add(z1.wrapping_add(z4));

        let bits = CONST_BITS - pass1_bits;
        workspace[column] = descale(tmp10.wrapping_add(tmp3), bits);
        workspace[7 * 8 + column] = descale(tmp10.wrapping_sub(tmp3), bits);
        workspace[8 + column] = descale(tmp11.wrapping_add(tmp2), bits);
        workspace[6 * 8 + column] = descale(tmp11.wrapping_sub(tmp2), bits);
        workspace[2 * 8 + column] = descale(tmp12.wrapping_add(tmp1), bits);
        workspace[5 * 8 + column] = descale(tmp12.wrapping_sub(tmp1), bits);
        workspace[3 * 8 + column] = descale(tmp13.wrapping_add(tmp0), bits);
        workspace[4 * 8 + column] = descale(tmp13.wrapping_sub(tmp0), bits);
    }

    let final_bits = CONST_BITS + pass1_bits + 3;
    for row in 0..8 {
        let w = &workspace[row * 8..row * 8 + 8];
        let out = &mut output[row * stride..row * stride + 8];
        if w[1..8].iter().all(|&value| value == 0) {
            let dc = range.get(descale(w[0], pass1_bits + 3));
            out.fill(dc);
            continue;
        }
        let mut z2 = w[2];
        let mut z3 = w[6];
        let mut z1 = z2.wrapping_add(z3).wrapping_mul(FIX_0_541196100);
        let mut tmp2 = z1.wrapping_add(z3.wrapping_mul(-FIX_1_847759065));
        let mut tmp3 = z1.wrapping_add(z2.wrapping_mul(FIX_0_765366865));
        let mut tmp0 = w[0].wrapping_add(w[4]).wrapping_shl(CONST_BITS);
        let mut tmp1 = w[0].wrapping_sub(w[4]).wrapping_shl(CONST_BITS);
        let tmp10 = tmp0.wrapping_add(tmp3);
        let tmp13 = tmp0.wrapping_sub(tmp3);
        let tmp11 = tmp1.wrapping_add(tmp2);
        let tmp12 = tmp1.wrapping_sub(tmp2);

        tmp0 = w[7];
        tmp1 = w[5];
        tmp2 = w[3];
        tmp3 = w[1];
        z1 = tmp0.wrapping_add(tmp3);
        z2 = tmp1.wrapping_add(tmp2);
        z3 = tmp0.wrapping_add(tmp2);
        let mut z4 = tmp1.wrapping_add(tmp3);
        let z5 = z3.wrapping_add(z4).wrapping_mul(FIX_1_175875602);
        tmp0 = tmp0.wrapping_mul(FIX_0_298631336);
        tmp1 = tmp1.wrapping_mul(FIX_2_053119869);
        tmp2 = tmp2.wrapping_mul(FIX_3_072711026);
        tmp3 = tmp3.wrapping_mul(FIX_1_501321110);
        z1 = z1.wrapping_mul(-FIX_0_899976223);
        z2 = z2.wrapping_mul(-FIX_2_562915447);
        z3 = z3.wrapping_mul(-FIX_1_961570560);
        z4 = z4.wrapping_mul(-FIX_0_390180644);
        z3 = z3.wrapping_add(z5);
        z4 = z4.wrapping_add(z5);
        tmp0 = tmp0.wrapping_add(z1.wrapping_add(z3));
        tmp1 = tmp1.wrapping_add(z2.wrapping_add(z4));
        tmp2 = tmp2.wrapping_add(z2.wrapping_add(z3));
        tmp3 = tmp3.wrapping_add(z1.wrapping_add(z4));

        out[0] = range.get(descale(tmp10.wrapping_add(tmp3), final_bits));
        out[7] = range.get(descale(tmp10.wrapping_sub(tmp3), final_bits));
        out[1] = range.get(descale(tmp11.wrapping_add(tmp2), final_bits));
        out[6] = range.get(descale(tmp11.wrapping_sub(tmp2), final_bits));
        out[2] = range.get(descale(tmp12.wrapping_add(tmp1), final_bits));
        out[5] = range.get(descale(tmp12.wrapping_sub(tmp1), final_bits));
        out[3] = range.get(descale(tmp13.wrapping_add(tmp0), final_bits));
        out[4] = range.get(descale(tmp13.wrapping_sub(tmp0), final_bits));
    }
}

/// The 8-bit IDCT as the library runs it on x86-64 with AVX2
/// (`jsimd_idct_islow_avx2`): the same factorization, but dequantized
/// coefficients and some sums are 16-bit and wrap, each pass's results are
/// saturated to 16 bits, the DC-only shortcut needs the whole block's AC
/// terms to be zero, and outputs are clamped rather than range-limited.
/// On sound data this matches `idct_islow` exactly.
fn idct_islow_avx2(block: &[i16; 64], quant: &[i16; 64], output: &mut [u16], stride: usize) {
    let mut pass1 = [0i16; 64];
    let ac_zero = block[8..64].iter().all(|&value| value == 0);
    if ac_zero {
        for column in 0..8 {
            let dc = block[column].wrapping_mul(quant[column]).wrapping_shl(2);
            for row in 0..8 {
                pass1[row * 8 + column] = dc;
            }
        }
    } else {
        let mut dequantized = [0i16; 64];
        for (index, value) in dequantized.iter_mut().enumerate() {
            *value = block[index].wrapping_mul(quant[index]);
        }
        for column in 0..8 {
            let input: [i16; 8] = std::array::from_fn(|row| dequantized[row * 8 + column]);
            let results = idct_1d_avx2(&input, CONST_BITS - 2);
            for (row, &value) in results.iter().enumerate() {
                pass1[row * 8 + column] = saturate_i16(value);
            }
        }
    }
    for row in 0..8 {
        let input: [i16; 8] = std::array::from_fn(|column| pass1[row * 8 + column]);
        let results = idct_1d_avx2(&input, CONST_BITS + 2 + 3);
        for (column, &value) in results.iter().enumerate() {
            output[row * stride + column] = (saturate_i16(value).clamp(-128, 127) + 128) as u16;
        }
    }
}

fn saturate_i16(value: i32) -> i16 {
    value.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

/// One column or row of `jsimd_idct_islow_avx2`'s `DODCT`, descaled by
/// `shift` (not yet saturated). Products pair up as `vpmaddwd` makes them.
fn idct_1d_avx2(input: &[i16; 8], shift: u32) -> [i32; 8] {
    let value = |index: usize| input[index] as i32;
    let pair = |a: i32, weight_a: i32, b: i32, weight_b: i32| a.wrapping_mul(weight_a).wrapping_add(b.wrapping_mul(weight_b));
    let tmp3 = pair(value(2), FIX_0_541196100 + FIX_0_765366865, value(6), FIX_0_541196100);
    let tmp2 = pair(value(2), FIX_0_541196100, value(6), FIX_0_541196100 - FIX_1_847759065);
    let tmp0 = (input[0].wrapping_add(input[4]) as i32) << CONST_BITS;
    let tmp1 = (input[0].wrapping_sub(input[4]) as i32) << CONST_BITS;
    let tmp10 = tmp0.wrapping_add(tmp3);
    let tmp13 = tmp0.wrapping_sub(tmp3);
    let tmp11 = tmp1.wrapping_add(tmp2);
    let tmp12 = tmp1.wrapping_sub(tmp2);

    let z3 = input[7].wrapping_add(input[3]) as i32;
    let z4 = input[5].wrapping_add(input[1]) as i32;
    let z3 = pair(z3, FIX_1_175875602 - FIX_1_961570560, z4, FIX_1_175875602);
    let z4_product = pair(input[7].wrapping_add(input[3]) as i32, FIX_1_175875602, input[5].wrapping_add(input[1]) as i32, FIX_1_175875602 - FIX_0_390180644);
    let odd0 = pair(value(7), FIX_0_298631336 - FIX_0_899976223, value(1), -FIX_0_899976223).wrapping_add(z3);
    let odd1 = pair(value(5), FIX_2_053119869 - FIX_2_562915447, value(3), -FIX_2_562915447).wrapping_add(z4_product);
    let odd3 = pair(value(7), -FIX_0_899976223, value(1), FIX_1_501321110 - FIX_0_899976223).wrapping_add(z4_product);
    let odd2 = pair(value(5), -FIX_2_562915447, value(3), FIX_3_072711026 - FIX_2_562915447).wrapping_add(z3);

    let round = |sum: i32| sum.wrapping_add(1 << (shift - 1)) >> shift;
    [
        round(tmp10.wrapping_add(odd3)),
        round(tmp11.wrapping_add(odd2)),
        round(tmp12.wrapping_add(odd1)),
        round(tmp13.wrapping_add(odd0)),
        round(tmp13.wrapping_sub(odd0)),
        round(tmp12.wrapping_sub(odd1)),
        round(tmp11.wrapping_sub(odd2)),
        round(tmp10.wrapping_sub(odd3)),
    ]
}

/// The IDCT a component's samples go through: the AVX2 one for 8-bit
/// data, the C one (with 32-bit `long`) for 12-bit.
fn inverse_dct(block: &[i16; 64], quant: &[i16; 64], precision: i32, range: &RangeLimit, output: &mut [u16], stride: usize) {
    if precision == 8 {
        idct_islow_avx2(block, quant, output, stride);
    } else {
        idct_islow(block, quant, 1, range, output, stride);
    }
}

/// Coefficients of the progression `smoothing_ok` latched: per component,
/// the current status and the previous scan's.
struct SmoothingBits {
    current: Vec<[i32; 10]>,
    previous: Vec<[i32; 10]>,
}

/// `smoothing_ok`: smoothing applies to a progressive image whose DC
/// values are known and some of whose first AC coefficients are not
/// fully, when the quantizers it divides by are nonzero.
fn smoothing_bits(decoder: &Decoder) -> Option<SmoothingBits> {
    if !decoder.progressive || decoder.coef_bits.is_empty() {
        return None;
    }
    let count = decoder.components.len();
    let mut useful = false;
    let mut current = Vec::with_capacity(count);
    let mut previous = Vec::with_capacity(count);
    for (index, component) in decoder.components.iter().enumerate() {
        let table = component.quant_table?;
        let positions = [0, 1, 8, 16, 9, 2, 3, 10, 17, 24];
        if positions.iter().any(|&position| table[position] == 0) {
            return None;
        }
        let bits = &decoder.coef_bits[index];
        let prior = &decoder.coef_bits[index + count];
        if bits[0] < 0 {
            return None;
        }
        let mut latched = [0i32; 10];
        let mut latched_prior = [0i32; 10];
        latched[0] = bits[0];
        for coefficient in 1..10 {
            latched_prior[coefficient] = if decoder.input_scan_number > 1 { prior[coefficient] } else { -1 };
            latched[coefficient] = bits[coefficient];
            if bits[coefficient] != 0 {
                useful = true;
            }
        }
        current.push(latched);
        previous.push(latched_prior);
    }
    if !useful {
        return None;
    }
    Some(SmoothingBits { current, previous })
}

/// One estimated coefficient (`num` against quantizer `q`), no larger than
/// its remaining precision `al` allows.
fn estimate(num: i32, q: i32, al: i32) -> i16 {
    let mut prediction;
    if num >= 0 {
        prediction = (q.wrapping_shl(7).wrapping_add(num)) / q.wrapping_shl(8);
        if al > 0 && prediction >= (1 << al) {
            prediction = (1 << al) - 1;
        }
    } else {
        prediction = (q.wrapping_shl(7).wrapping_sub(num)) / q.wrapping_shl(8);
        if al > 0 && prediction >= (1 << al) {
            prediction = (1 << al) - 1;
        }
        prediction = -prediction;
    }
    prediction as i16
}

/// The component's samples from its coefficients, with block smoothing
/// when it applies (`decompress_smooth_data`, else `decompress_data`).
fn component_plane(decoder: &Decoder, index: usize, smoothing: Option<&SmoothingBits>, range: &RangeLimit) -> Plane {
    let component = &decoder.components[index];
    let quant_table = component.quant_table.unwrap_or([0; 64]);
    let mut quant = [0i16; 64];
    for (multiplier, &value) in quant.iter_mut().zip(quant_table.iter()) {
        *multiplier = value as i16;
    }
    let stride = component.width_in_blocks * 8;
    let rows = component.height_in_blocks * 8;
    let mut samples = vec![0u16; stride * rows];
    let blocks = &decoder.coefficients[index];
    let block_stride = decoder.block_stride[index];
    let Some(smoothing) = smoothing else {
        for block_row in 0..component.height_in_blocks {
            for block_column in 0..component.width_in_blocks {
                let block = &blocks[block_row * block_stride + block_column];
                let offset = block_row * 8 * stride + block_column * 8;
                inverse_dct(block, &quant, decoder.precision, range, &mut samples[offset..], stride);
            }
        }
        return Plane { samples, stride };
    };

    let v_samp = component.v_samp;
    let last_imcu_row = decoder.total_imcu_rows - 1;
    let q = |position: usize| quant_table[position] as i32;
    let (q00, q01, q10, q20, q11, q02) = (q(0), q(1), q(8), q(16), q(9), q(2));
    let (q03, q12, q21, q30) = (q(3), q(10), q(17), q(24));
    let dc = |row: usize, column: usize| blocks[row * block_stride + column][0] as i32;
    let last_block_column = component.width_in_blocks - 1;
    for imcu_row in 0..decoder.total_imcu_rows {
        let block_rows = if imcu_row < last_imcu_row {
            v_samp
        } else {
            let rows = component.height_in_blocks % v_samp;
            if rows == 0 {
                v_samp
            } else {
                rows
            }
        };
        let bits = if imcu_row > decoder.last_good_imcu_row { &smoothing.previous[index] } else { &smoothing.current[index] };
        let change_dc = (1..10).all(|coefficient| bits[coefficient] == -1);
        let image_block_rows = block_rows * decoder.total_imcu_rows;
        for block_row in 0..block_rows {
            let image_block_row = imcu_row * block_rows + block_row;
            let row = imcu_row * v_samp + block_row;
            let previous_row = if image_block_row > 0 { row - 1 } else { row };
            let previous_previous_row = if image_block_row > 1 { row - 2 } else { previous_row };
            let next_row = if image_block_row + 1 < image_block_rows { row + 1 } else { row };
            let next_next_row = if image_block_row + 2 < image_block_rows { row + 2 } else { next_row };
            let row_list = [previous_previous_row, previous_row, row, next_row, next_next_row];
            // The 5 × 5 neighbourhood of DC values, sliding along the row.
            let mut window = [[0i32; 5]; 5];
            for (line, &source_row) in row_list.iter().enumerate() {
                let value = dc(source_row, 0);
                window[line] = [value; 5];
            }
            for block_column in 0..component.width_in_blocks {
                let mut workspace = blocks[row * block_stride + block_column];
                if block_column == 0 && block_column < last_block_column {
                    for (line, &source_row) in row_list.iter().enumerate() {
                        let value = dc(source_row, 1);
                        window[line][3] = value;
                        window[line][4] = value;
                    }
                }
                if block_column + 1 < last_block_column {
                    for (line, &source_row) in row_list.iter().enumerate() {
                        window[line][4] = dc(source_row, block_column + 2);
                    }
                }
                let d = |number: usize| window[(number - 1) / 5][(number - 1) % 5];
                let (dc01, dc02, dc03, dc04, dc05) = (d(1), d(2), d(3), d(4), d(5));
                let (dc06, dc07, dc08, dc09, dc10) = (d(6), d(7), d(8), d(9), d(10));
                let (dc11, dc12, dc13, dc14, dc15) = (d(11), d(12), d(13), d(14), d(15));
                let (dc16, dc17, dc18, dc19, dc20) = (d(16), d(17), d(18), d(19), d(20));
                let (dc21, dc22, dc23, dc24, dc25) = (d(21), d(22), d(23), d(24), d(25));
                let times = |weight: i32, value: i32| weight.wrapping_mul(value);
                if bits[1] != 0 && workspace[1] == 0 {
                    let sum = if change_dc {
                        -dc01 - dc02 + dc04 + dc05 - times(3, dc06) + times(13, dc07) - times(13, dc09) + times(3, dc10) - times(3, dc11) + times(38, dc12) - times(38, dc14) + times(3, dc15) - times(3, dc16) + times(13, dc17) - times(13, dc19) + times(3, dc20) - dc21 - dc22 + dc24 + dc25
                    } else {
                        times(-7, dc11) + times(50, dc12) - times(50, dc14) + times(7, dc15)
                    };
                    workspace[1] = estimate(q00.wrapping_mul(sum), q01, bits[1]);
                }
                if bits[2] != 0 && workspace[8] == 0 {
                    let sum = if change_dc {
                        -dc01 - times(3, dc02) - times(3, dc03) - times(3, dc04) - dc05 - dc06 + times(13, dc07) + times(38, dc08) + times(13, dc09) - dc10 + dc16 - times(13, dc17) - times(38, dc18) - times(13, dc19) + dc20 + dc21 + times(3, dc22) + times(3, dc23) + times(3, dc24) + dc25
                    } else {
                        times(-7, dc03) + times(50, dc08) - times(50, dc18) + times(7, dc23)
                    };
                    workspace[8] = estimate(q00.wrapping_mul(sum), q10, bits[2]);
                }
                if bits[3] != 0 && workspace[16] == 0 {
                    let sum = if change_dc {
                        dc03 + times(2, dc07) + times(7, dc08) + times(2, dc09) - times(5, dc12) - times(14, dc13) - times(5, dc14) + times(2, dc17) + times(7, dc18) + times(2, dc19) + dc23
                    } else {
                        -dc03 + times(13, dc08) - times(24, dc13) + times(13, dc18) - dc23
                    };
                    workspace[16] = estimate(q00.wrapping_mul(sum), q20, bits[3]);
                }
                if bits[4] != 0 && workspace[9] == 0 {
                    let sum = if change_dc {
                        -dc01 + dc05 + times(9, dc07) - times(9, dc09) - times(9, dc17) + times(9, dc19) + dc21 - dc25
                    } else {
                        dc10 + dc16 - times(10, dc17) + times(10, dc19) - dc02 - dc20 + dc22 - dc24 + dc04 - dc06 + times(10, dc07) - times(10, dc09)
                    };
                    workspace[9] = estimate(q00.wrapping_mul(sum), q11, bits[4]);
                }
                if bits[5] != 0 && workspace[2] == 0 {
                    let sum = if change_dc {
                        times(2, dc07) - times(5, dc08) + times(2, dc09) + dc11 + times(7, dc12) - times(14, dc13) + times(7, dc14) + dc15 + times(2, dc17) - times(5, dc18) + times(2, dc19)
                    } else {
                        -dc11 + times(13, dc12) - times(24, dc13) + times(13, dc14) - dc15
                    };
                    workspace[2] = estimate(q00.wrapping_mul(sum), q02, bits[5]);
                }
                if change_dc {
                    if bits[6] != 0 && workspace[3] == 0 {
                        let sum = dc07 - dc09 + times(2, dc12) - times(2, dc14) + dc17 - dc19;
                        workspace[3] = estimate(q00.wrapping_mul(sum), q03, bits[6]);
                    }
                    if bits[7] != 0 && workspace[10] == 0 {
                        let sum = dc07 - times(3, dc08) + dc09 - dc17 + times(3, dc18) - dc19;
                        workspace[10] = estimate(q00.wrapping_mul(sum), q12, bits[7]);
                    }
                    if bits[8] != 0 && workspace[17] == 0 {
                        let sum = dc07 - dc09 - times(3, dc12) + times(3, dc14) + dc17 - dc19;
                        workspace[17] = estimate(q00.wrapping_mul(sum), q21, bits[8]);
                    }
                    if bits[9] != 0 && workspace[24] == 0 {
                        let sum = dc07 + times(2, dc08) + dc09 - dc17 - times(2, dc18) - dc19;
                        workspace[24] = estimate(q00.wrapping_mul(sum), q30, bits[9]);
                    }
                    let sum = times(-2, dc01) - times(6, dc02) - times(8, dc03) - times(6, dc04) - times(2, dc05) - times(6, dc06) + times(6, dc07) + times(42, dc08) + times(6, dc09) - times(6, dc10) - times(8, dc11) + times(42, dc12) + times(152, dc13) + times(42, dc14) - times(8, dc15) - times(6, dc16) + times(6, dc17) + times(42, dc18) + times(6, dc19) - times(6, dc20) - times(2, dc21) - times(6, dc22) - times(8, dc23) - times(6, dc24) - times(2, dc25);
                    workspace[0] = estimate(q00.wrapping_mul(sum), q00, 0);
                }
                let offset = row * 8 * stride + block_column * 8;
                inverse_dct(&workspace, &quant, decoder.precision, range, &mut samples[offset..], stride);
                for line in window.iter_mut() {
                    line.copy_within(1..5, 0);
                }
            }
        }
    }
    Plane { samples, stride }
}

/// The upsampling method of a component (`jinit_upsampler`).
#[derive(Clone, Copy)]
enum Method {
    FullSize,
    H2V1,
    H2V1Fancy,
    H1V2Fancy,
    H2V2,
    H2V2Fancy,
    Integral(usize, usize),
}

fn method(h_samp: usize, v_samp: usize, max_h: usize, max_v: usize, do_fancy: bool, downsampled_width: usize) -> Method {
    if h_samp == max_h && v_samp == max_v {
        Method::FullSize
    } else if h_samp * 2 == max_h && v_samp == max_v {
        if do_fancy && downsampled_width > 2 {
            Method::H2V1Fancy
        } else {
            Method::H2V1
        }
    } else if h_samp == max_h && v_samp * 2 == max_v && do_fancy {
        Method::H1V2Fancy
    } else if h_samp * 2 == max_h && v_samp * 2 == max_v {
        if do_fancy && downsampled_width > 2 {
            Method::H2V2Fancy
        } else {
            Method::H2V2
        }
    } else {
        Method::Integral(max_h / h_samp, max_v / v_samp)
    }
}

/// A component brought to full size, `width` × `height`.
fn upsample(plane: &Plane, method: Method, downsampled_width: usize, downsampled_height: usize, width: usize, height: usize) -> Vec<i32> {
    let mut output = vec![0i32; width * height];
    // Rows above the first and below the last real one repeat them.
    let row_at = |row: isize| -> &[u16] {
        let clamped = row.clamp(0, downsampled_height as isize - 1) as usize;
        &plane.samples[clamped * plane.stride..clamped * plane.stride + plane.stride]
    };
    let dw = downsampled_width;
    match method {
        Method::FullSize => {
            for y in 0..height {
                let input = row_at(y as isize);
                for x in 0..width {
                    output[y * width + x] = input[x] as i32;
                }
            }
        }
        Method::H2V1 | Method::H2V2 | Method::Integral(..) => {
            let (h_expand, v_expand) = match method {
                Method::H2V1 => (2, 1),
                Method::H2V2 => (2, 2),
                Method::Integral(h, v) => (h, v),
                _ => unreachable!(),
            };
            for y in 0..height {
                let input = &plane.samples[(y / v_expand) * plane.stride..];
                for x in 0..width {
                    output[y * width + x] = input[x / h_expand] as i32;
                }
            }
        }
        Method::H2V1Fancy => {
            let mut row = vec![0i32; 2 * dw];
            for y in 0..height {
                let input = row_at(y as isize);
                h2v1_fancy_row(input, dw, &mut row);
                output[y * width..y * width + width].copy_from_slice(&row[..width]);
            }
        }
        Method::H1V2Fancy => {
            for y in 0..height {
                let input_row = (y / 2) as isize;
                let nearest = row_at(input_row);
                let (further, bias) = if y % 2 == 0 { (row_at(input_row - 1), 1) } else { (row_at(input_row + 1), 2) };
                for x in 0..width.min(dw) {
                    let sum = nearest[x] as i32 * 3 + further[x] as i32;
                    output[y * width + x] = (sum + bias) >> 2;
                }
            }
        }
        Method::H2V2Fancy => {
            let mut row = vec![0i32; 2 * dw];
            for y in 0..height {
                let input_row = (y / 2) as isize;
                let nearest = row_at(input_row);
                let further = if y % 2 == 0 { row_at(input_row - 1) } else { row_at(input_row + 1) };
                h2v2_fancy_row(nearest, further, dw, &mut row);
                output[y * width..y * width + width].copy_from_slice(&row[..width]);
            }
        }
    }
    output
}

/// `h2v1_fancy_upsample` on one row of `dw` samples.
fn h2v1_fancy_row(input: &[u16], dw: usize, output: &mut [i32]) {
    let value = |x: usize| input[x] as i32;
    output[0] = value(0);
    output[1] = (value(0) * 3 + value(1) + 2) >> 2;
    for x in 1..dw - 1 {
        let current = value(x) * 3;
        output[2 * x] = (current + value(x - 1) + 1) >> 2;
        output[2 * x + 1] = (current + value(x + 1) + 2) >> 2;
    }
    let last = dw - 1;
    output[2 * last] = (value(last) * 3 + value(last - 1) + 1) >> 2;
    output[2 * last + 1] = value(last);
}

/// `h2v2_fancy_upsample` on one output row: the nearest input row and the
/// next nearest (above for the upper output row, below for the lower).
fn h2v2_fancy_row(nearest: &[u16], further: &[u16], dw: usize, output: &mut [i32]) {
    let column_sum = |x: usize| nearest[x] as i32 * 3 + further[x] as i32;
    let mut this = column_sum(0);
    let mut next = column_sum(1);
    output[0] = (this * 4 + 8) >> 4;
    output[1] = (this * 3 + next + 7) >> 4;
    let mut last = this;
    this = next;
    for x in 1..dw - 1 {
        next = column_sum(x + 1);
        output[2 * x] = (this * 3 + last + 8) >> 4;
        output[2 * x + 1] = (this * 3 + next + 7) >> 4;
        last = this;
        this = next;
    }
    let end = dw - 1;
    output[2 * end] = (this * 3 + last + 8) >> 4;
    output[2 * end + 1] = (this * 4 + 7) >> 4;
}

/// `build_ycc_rgb_table`'s tables, by sample value.
struct YccTables {
    cr_r: Vec<i32>,
    cb_b: Vec<i32>,
    cr_g: Vec<i32>,
    cb_g: Vec<i32>,
}

impl YccTables {
    fn new(maximum: i32) -> Self {
        let center = (maximum + 1) / 2;
        let mut tables = YccTables { cr_r: Vec::new(), cb_b: Vec::new(), cr_g: Vec::new(), cb_g: Vec::new() };
        for i in 0..=maximum {
            let x = i - center;
            tables.cr_r.push((91881i32.wrapping_mul(x).wrapping_add(32768)) >> 16);
            tables.cb_b.push((116130i32.wrapping_mul(x).wrapping_add(32768)) >> 16);
            tables.cr_g.push((-46802i32).wrapping_mul(x));
            tables.cb_g.push((-22554i32).wrapping_mul(x).wrapping_add(32768));
        }
        tables
    }

    /// Red, green and blue of one pixel, before range limiting.
    fn convert(&self, y: i32, cb: i32, cr: i32) -> [i32; 3] {
        let (cb, cr) = (cb as usize, cr as usize);
        [y + self.cr_r[cr], y + (self.cb_g[cb].wrapping_add(self.cr_g[cr]) >> 16), y + self.cb_b[cb]]
    }
}

/// The output samples, interleaved: components brought to full size, then
/// converted to the output colour space.
pub(super) fn produce(decoder: &Decoder) -> Vec<u16> {
    let width = decoder.width;
    let height = decoder.height;
    let maximum: i32 = if decoder.precision <= 8 { 255 } else if decoder.precision <= 12 { 4095 } else { 65535 };
    let planes: Vec<Plane> = if decoder.lossless {
        decoder.planes.iter().zip(decoder.block_stride.iter()).map(|(samples, &stride)| Plane { samples: samples.clone(), stride }).collect()
    } else {
        let range = RangeLimit::new(maximum);
        let smoothing = smoothing_bits(decoder);
        (0..decoder.components.len()).map(|index| component_plane(decoder, index, smoothing.as_ref(), &range)).collect()
    };
    let do_fancy = !decoder.lossless;
    let full: Vec<Vec<i32>> = decoder
        .components
        .iter()
        .zip(planes.iter())
        .map(|(component, plane)| {
            let method = method(component.h_samp, component.v_samp, decoder.max_h, decoder.max_v, do_fancy, component.downsampled_width);
            upsample(plane, method, component.downsampled_width, component.downsampled_height, width, height)
        })
        .collect();
    let pixels = width * height;
    let clamp = |value: i32| value.clamp(0, maximum) as u16;
    match (decoder.jpeg_color_space, decoder.out_color_space) {
        (ColorSpace::YCbCr, ColorSpace::Rgb) => {
            let tables = YccTables::new(maximum);
            let mut samples = Vec::with_capacity(pixels * 3);
            for ((&luma, &blue), &red) in full[0].iter().zip(&full[1]).zip(&full[2]).take(pixels) {
                let rgb = tables.convert(luma, blue, red);
                samples.extend(rgb.iter().map(|&value| clamp(value)));
            }
            samples
        }
        (ColorSpace::Ycck, ColorSpace::Cmyk) => {
            let tables = YccTables::new(maximum);
            let mut samples = Vec::with_capacity(pixels * 4);
            for (((&luma, &blue), &red), &black) in full[0].iter().zip(&full[1]).zip(&full[2]).zip(&full[3]).take(pixels) {
                let rgb = tables.convert(luma, blue, red);
                samples.extend(rgb.iter().map(|&value| clamp(maximum - value)));
                samples.push(black as u16);
            }
            samples
        }
        _ => {
            let components = full.len();
            let mut samples = Vec::with_capacity(pixels * components);
            for pixel in 0..pixels {
                for plane in &full {
                    samples.push(plane[pixel] as u16);
                }
            }
            samples
        }
    }
}

/// Each component's samples as `jpeg_read_raw_data` gives them: no
/// upsampling, rows padded to whole blocks (plane and row stride).
pub(super) fn raw_planes(decoder: &Decoder) -> Vec<(Vec<u16>, usize)> {
    let maximum: i32 = if decoder.precision <= 8 { 255 } else if decoder.precision <= 12 { 4095 } else { 65535 };
    if decoder.lossless {
        return decoder.planes.iter().zip(decoder.block_stride.iter()).map(|(samples, &stride)| (samples.clone(), stride)).collect();
    }
    let range = RangeLimit::new(maximum);
    let smoothing = smoothing_bits(decoder);
    (0..decoder.components.len())
        .map(|index| {
            let plane = component_plane(decoder, index, smoothing.as_ref(), &range);
            (plane.samples, plane.stride)
        })
        .collect()
}
