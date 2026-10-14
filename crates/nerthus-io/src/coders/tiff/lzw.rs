//! libtiff's LZW decoders (`tif_lzw.c`): `LZWDecode` for codes written
//! most significant bit first, 9 to 12 bits wide and widening one code
//! early, and `LZWDecodeCompat` for the old bit-reversed codes. A strip
//! must start with a clear code and supply all its output; codes past the
//! output's end are never read.

use anyhow::{bail, Result};

const CODE_CLEAR: u32 = 256;
const CODE_EOI: u32 = 257;
const CODE_FIRST: i64 = 258;
const BITS_MIN: u32 = 9;
const BITS_MAX: u32 = 12;
/// `CSIZE`: the table, with 1024 entries of slack for old files.
const TABLE_SIZE: i64 = 4095 + 1024;

/// One string of the table (`code_t`); `next` is the entry its prefix
/// ends with, or −1.
#[derive(Clone, Copy, Default)]
struct Code {
    next: i32,
    length: u16,
    first: u8,
    value: u8,
    repeated: bool,
}

fn max_code(bits: u32) -> u32 {
    (1 << bits) - 1
}

/// The table as `LZWSetupDecode` loads it: single bytes, the clear and
/// end codes zeroed.
fn new_table() -> Vec<Code> {
    let mut table = vec![Code::default(); TABLE_SIZE as usize];
    for (code, entry) in table.iter_mut().enumerate().take(256) {
        *entry = Code { next: -1, length: 1, first: code as u8, value: code as u8, repeated: true };
    }
    table
}

/// Codes read most significant bit first.
struct MsbBits<'a> {
    bytes: &'a [u8],
    position: u64,
}

impl MsbBits<'_> {
    /// The next `count` bits, or none when the data ends first.
    fn read(&mut self, count: u32) -> Option<u32> {
        let total = self.bytes.len() as u64 * 8;
        if self.position + count as u64 > total {
            return None;
        }
        let mut value = 0u32;
        for _ in 0..count {
            let byte = self.bytes[(self.position / 8) as usize];
            let bit = (byte >> (7 - self.position % 8)) & 1;
            value = (value << 1) | bit as u32;
            self.position += 1;
        }
        Some(value)
    }
}

/// Writes the first `count` bytes of the string ending with `code`.
fn write_prefix(table: &[Code], code: usize, output: &mut [u8], count: usize) {
    let mut entry = code;
    let mut length = table[entry].length as usize;
    while length > count {
        entry = table[entry].next as usize;
        length -= 1;
    }
    for index in (0..count).rev() {
        output[index] = table[entry].value;
        if table[entry].next < 0 {
            break;
        }
        entry = table[entry].next as usize;
    }
}

/// `LZWDecode` over a whole strip or tile.
pub(super) fn decode(raw: &[u8], output: &mut [u8]) -> Result<()> {
    let mut table = new_table();
    let mut bits = MsbBits { bytes: raw, position: 0 };
    let mut width = BITS_MIN;
    let mut mask = max_code(width);
    let mut free_entry: i64 = -1;
    let mut max_entry: i64 = mask as i64 - 1;
    let mut old_code: usize = 0;
    let mut out = 0usize;
    let total = output.len();
    if total == 0 {
        return Ok(());
    }
    loop {
        let Some(code) = bits.read(width) else {
            output[out..].fill(0);
            bail!("LZWDecode: Strip not terminated with EOI code");
        };
        let code = code & mask;
        if code == CODE_EOI {
            break;
        }
        if code == CODE_CLEAR {
            free_entry = CODE_FIRST;
            width = BITS_MIN;
            mask = max_code(width);
            max_entry = mask as i64 - 1;
            let mut next = CODE_CLEAR;
            while next == CODE_CLEAR {
                let Some(read) = bits.read(width) else {
                    output[out..].fill(0);
                    bail!("LZWDecode: Strip not terminated with EOI code");
                };
                next = read & mask;
            }
            if next == CODE_EOI {
                break;
            }
            if next > CODE_EOI {
                output[out..].fill(0);
                bail!("Using code not yet in table");
            }
            output[out] = next as u8;
            out += 1;
            old_code = next as usize;
            if out == total {
                return Ok(());
            }
            continue;
        }
        let code_index = code as i64;
        if code < 256 {
            if code_index > free_entry {
                output[out..].fill(0);
                bail!("Using code not yet in table");
            }
        } else if code_index > free_entry {
            output[out..].fill(0);
            bail!("Using code not yet in table");
        }
        // Add the old string plus this one's first byte.
        let old = table[old_code];
        let value = if code < 256 {
            code as u8
        } else if code_index >= free_entry {
            old.first
        } else {
            table[code as usize].first
        };
        let entry = &mut table[free_entry as usize];
        entry.value = value;
        entry.repeated = old.repeated && old.value == value;
        entry.next = old_code as i32;
        entry.first = old.first;
        entry.length = old.length + 1;
        free_entry += 1;
        if free_entry > max_entry {
            width = (width + 1).min(BITS_MAX);
            mask = max_code(width);
            max_entry = mask as i64 - 1;
            if free_entry >= TABLE_SIZE {
                // Only a clear or the end may follow.
                free_entry = -1;
            }
        }
        old_code = code as usize;
        let length = table[code as usize].length as usize;
        let left = total - out;
        if length > left {
            write_prefix(&table, code as usize, &mut output[out..], left);
            return Ok(());
        }
        write_prefix(&table, code as usize, &mut output[out..], length);
        out += length;
        if out == total {
            return Ok(());
        }
    }
    let short = total - out;
    output[out..].fill(0);
    bail!("Not enough data at scanline (short {short} bytes)")
}

/// `LZWDecodeCompat`: codes least significant bit first, the width
/// growing once the table passes its limit.
pub(super) fn decode_compat(raw: &[u8], output: &mut [u8]) -> Result<()> {
    let mut table = new_table();
    let mut input = 0usize;
    let mut bits_left = raw.len() as u64 * 8;
    let mut next_data: u64 = 0;
    let mut next_bits: u32 = 0;
    let mut width = BITS_MIN;
    let mut mask = max_code(width);
    let mut free_entry: i64 = -1;
    let mut max_entry: i64 = mask as i64 - 1;
    let mut old_code: usize = 0;
    let mut out = 0usize;
    let total = output.len();

    let next_code = |width: u32, mask: u32, input: &mut usize, next_data: &mut u64, next_bits: &mut u32, bits_left: &mut u64| -> u32 {
        if *bits_left < width as u64 {
            return CODE_EOI;
        }
        *next_data |= (raw[*input] as u64) << *next_bits;
        *input += 1;
        *next_bits += 8;
        if *next_bits < width {
            *next_data |= (raw[*input] as u64) << *next_bits;
            *input += 1;
            *next_bits += 8;
        }
        let code = (*next_data & mask as u64) as u32;
        *next_data >>= width;
        *next_bits -= width;
        *bits_left -= width as u64;
        code
    };

    while out < total {
        let mut code = next_code(width, mask, &mut input, &mut next_data, &mut next_bits, &mut bits_left);
        if code == CODE_EOI {
            break;
        }
        if code == CODE_CLEAR {
            while code == CODE_CLEAR {
                free_entry = CODE_FIRST;
                for entry in table[CODE_FIRST as usize..].iter_mut() {
                    *entry = Code::default();
                }
                width = BITS_MIN;
                mask = max_code(width);
                max_entry = mask as i64;
                code = next_code(width, mask, &mut input, &mut next_data, &mut next_bits, &mut bits_left);
            }
            if code == CODE_EOI {
                break;
            }
            if code > CODE_CLEAR {
                bail!("LZWDecode: Corrupted LZW table at scanline");
            }
            output[out] = code as u8;
            out += 1;
            old_code = code as usize;
            continue;
        }
        if !(0..TABLE_SIZE).contains(&free_entry) {
            bail!("Corrupted LZW table at scanline");
        }
        let old = table[old_code];
        let code_index = code as i64;
        let value = if code_index < free_entry { table[code as usize].first } else { old.first };
        let entry = &mut table[free_entry as usize];
        entry.next = old_code as i32;
        entry.first = old.first;
        entry.length = old.length + 1;
        entry.value = value;
        free_entry += 1;
        if free_entry > max_entry {
            width = (width + 1).min(BITS_MAX);
            mask = max_code(width);
            max_entry = mask as i64;
        }
        old_code = code as usize;
        if code >= 256 {
            let length = table[code as usize].length as usize;
            if length == 0 {
                bail!("Wrong length of decoded string: data probably corrupted at scanline");
            }
            let left = total - out;
            if length > left {
                write_prefix(&table, code as usize, &mut output[out..], left);
                return Ok(());
            }
            write_prefix(&table, code as usize, &mut output[out..], length);
            out += length;
        } else {
            output[out] = code as u8;
            out += 1;
        }
    }
    if out < total {
        bail!("Not enough data at scanline (short {} bytes)", total - out);
    }
    Ok(())
}
