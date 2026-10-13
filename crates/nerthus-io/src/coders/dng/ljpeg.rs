//! LibRaw's bit reader and lossless JPEG decoder (`decoders_dcraw.cpp`):
//! `getbithuff`, `make_decoder_ref`, `ljpeg_start`, `ljpeg_diff`,
//! `ljpeg_row` and `ljpeg_idct`.

use super::stream::Stream;

/// `getbithuff`'s state (`tls->getbits`) and `zero_after_ff`.
#[derive(Default)]
pub(super) struct BitReader {
    bitbuf: u32,
    vbits: i32,
    reset: bool,
    pub zero_after_ff: bool,
    /// `data_error`: reads past the data (LibRaw only warns).
    pub errors: u32,
}

impl BitReader {
    /// `getbits(-1)`.
    pub fn restart(&mut self) {
        self.bitbuf = 0;
        self.vbits = 0;
        self.reset = false;
    }

    /// `getbithuff`: `nbits` bits, or with a decoder table the symbol of
    /// the code those bits start with.
    pub fn getbithuff(&mut self, stream: &mut Stream, nbits: i32, huff: Option<&[u16]>) -> u32 {
        if nbits > 25 {
            return 0;
        }
        if nbits < 0 {
            self.restart();
            return 0;
        }
        if nbits == 0 || self.vbits < 0 {
            return 0;
        }
        while !self.reset && self.vbits < nbits {
            let Some(byte) = stream.getc() else {
                break;
            };
            // A 0xff followed by anything but 0 is a marker: stop there.
            if self.zero_after_ff && byte == 0xff && stream.getc() != Some(0) {
                self.reset = true;
                break;
            }
            self.bitbuf = (self.bitbuf << 8).wrapping_add(byte as u32);
            self.vbits += 8;
        }
        let mut value = if self.vbits == 0 { 0 } else { (self.bitbuf << (32 - self.vbits) as u32) >> (32 - nbits) as u32 };
        match huff {
            Some(table) => {
                let entry = table[value as usize];
                self.vbits -= (entry >> 8) as i32;
                value = (entry & 0xff) as u32;
            }
            None => self.vbits -= nbits,
        }
        if self.vbits < 0 {
            self.errors += 1;
        }
        value
    }

    pub fn getbits(&mut self, stream: &mut Stream, nbits: i32) -> u32 {
        self.getbithuff(stream, nbits, None)
    }

    /// `gethuff`: a table's first entry is its longest code length.
    pub fn gethuff(&mut self, stream: &mut Stream, table: &[u16]) -> u32 {
        self.getbithuff(stream, table[0] as i32, Some(&table[1..]))
    }
}

/// `make_decoder_ref`: a lookup table from 16 code-length counts and the
/// symbols after them; `source` moves past what it used.
fn make_decoder(data: &[u8], source: &mut usize) -> Vec<u16> {
    let counts_start = *source;
    *source += 16;
    let count = |length: usize| -> usize { data.get(counts_start + length - 1).copied().unwrap_or(0) as usize };
    let mut max = 16;
    while max > 0 && count(max) == 0 {
        max -= 1;
    }
    let mut huff = vec![0u16; 1 + (1 << max)];
    huff[0] = max as u16;
    let mut position = 1usize;
    for length in 1..=max {
        for _ in 0..count(length) {
            let symbol = data.get(*source).copied().unwrap_or(0) as u16;
            for _ in 0..1usize << (max - length) {
                if position <= 1 << max {
                    huff[position] = (length as u16) << 8 | symbol;
                    position += 1;
                }
            }
            *source += 1;
        }
    }
    huff
}

/// `struct jhead`.
pub(super) struct JpegHead {
    pub algo: u32,
    pub bits: i32,
    pub high: i32,
    pub wide: i32,
    pub clrs: i32,
    pub sraw: i32,
    pub psv: i32,
    pub restart: i32,
    pub vpred: [i32; 6],
    pub quant: [u16; 64],
    pub idct: [u16; 64],
    tables: Vec<Vec<u16>>,
    /// `huff[]`: which table each index uses.
    huff: [Option<usize>; 20],
    /// Two rows of samples (`row`).
    pub rows: Vec<u16>,
    cosines: Vec<f32>,
}

impl JpegHead {
    fn table(&self, index: usize) -> Option<&[u16]> {
        self.huff[index].map(|table| self.tables[table].as_slice())
    }
}

/// `ljpeg_start`: reads the markers up to the start of scan. `None` when
/// they are not a usable lossless JPEG header.
pub(super) fn ljpeg_start(stream: &mut Stream, bits: &mut BitReader, dng_version: u32, info_only: bool) -> Option<JpegHead> {
    let mut head = JpegHead {
        algo: 0,
        bits: 0,
        high: 0,
        wide: 0,
        clrs: 0,
        sraw: 0,
        psv: 0,
        restart: i32::MAX,
        vpred: [0; 6],
        quant: [0; 64],
        idct: [0; 64],
        tables: Vec::new(),
        huff: [None; 20],
        rows: Vec::new(),
        cosines: Vec::new(),
    };
    let mut start = [0u8; 2];
    if stream.read(&mut start) != 2 || start[1] != 0xd8 {
        return None;
    }
    let mut data = vec![0u8; 0x10000];
    let mut markers = 0;
    loop {
        if stream.eof() {
            return None;
        }
        markers += 1;
        if markers > 1025 {
            return None;
        }
        let mut header = [0u8; 4];
        if stream.read(&mut header) != 4 {
            return None;
        }
        let tag = (header[0] as u32) << 8 | header[1] as u32;
        let length = (((header[2] as u32) << 8 | header[3] as u32).wrapping_sub(2) & 0xffff) as usize;
        if tag <= 0xff00 {
            return None;
        }
        if stream.read(&mut data[..length]) != length {
            return None;
        }
        match tag {
            0xffc0 | 0xffc1 | 0xffc3 => {
                if tag == 0xffc3 {
                    head.sraw = (((data[7] >> 4) as i32 * (data[7] & 15) as i32) - 1) & 3;
                }
                head.algo = tag & 0xff;
                head.bits = data[0] as i32;
                head.high = (data[1] as i32) << 8 | data[2] as i32;
                head.wide = (data[3] as i32) << 8 | data[4] as i32;
                head.clrs = data[5] as i32 + head.sraw;
                if length == 9 && dng_version == 0 {
                    stream.getc();
                }
            }
            0xffc4 => {
                if !info_only {
                    let mut position = 0usize;
                    while position < length {
                        let class = data[position];
                        position += 1;
                        if class & 0xec != 0 {
                            break;
                        }
                        let table = make_decoder(&data, &mut position);
                        head.tables.push(table);
                        head.huff[class as usize] = Some(head.tables.len() - 1);
                    }
                }
            }
            0xffda => {
                let components = data[0] as usize;
                head.psv = data[1 + components * 2] as i32;
                head.bits -= (data[3 + components * 2] & 15) as i32;
            }
            0xffdb => {
                for (index, value) in head.quant.iter_mut().enumerate() {
                    *value = (data[index * 2 + 1] as u16) << 8 | data[index * 2 + 2] as u16;
                }
            }
            0xffdd => head.restart = (data[0] as i32) << 8 | data[1] as i32,
            _ => {}
        }
        if tag == 0xffda {
            break;
        }
    }
    if head.bits > 16 || head.clrs > 6 || head.bits == 0 || head.high == 0 || head.wide == 0 || head.clrs == 0 {
        return None;
    }
    if info_only {
        return Some(head);
    }
    head.huff[0]?;
    for index in 0..19 {
        if head.huff[index + 1].is_none() {
            head.huff[index + 1] = head.huff[index];
        }
    }
    if head.sraw != 0 {
        for index in 0..4 {
            head.huff[2 + index] = head.huff[1];
        }
        for index in 0..head.sraw as usize {
            head.huff[1 + index] = head.huff[0];
        }
    }
    head.rows = vec![0u16; head.wide as usize * head.clrs as usize * 2];
    bits.zero_after_ff = true;
    Some(head)
}

/// `ljpeg_diff`: one difference, its length coded with `table`.
fn ljpeg_diff(stream: &mut Stream, bits: &mut BitReader, table: Option<&[u16]>, dng_version: u32) -> i32 {
    let Some(table) = table else {
        return 0;
    };
    let length = bits.gethuff(stream, table) as i32;
    if length == 16 && (dng_version == 0 || dng_version >= 0x1010000) {
        return -32768;
    }
    let mut diff = bits.getbits(stream, length) as i32;
    // x86 masks the shift counts, as the C shifts do there.
    if diff & 1i32.wrapping_shl((length - 1) as u32) == 0 {
        diff -= 1i32.wrapping_shl(length as u32) - 1;
    }
    diff
}

/// `ljpeg_row`: one row of samples (`wide × clrs`), predicted from the
/// row before; returns where the row starts in [JpegHead::rows].
pub(super) fn ljpeg_row(stream: &mut Stream, bits: &mut BitReader, head: &mut JpegHead, jrow: i32, dng_version: u32) -> usize {
    let width = (head.wide * head.clrs) as usize;
    let clrs = head.clrs as usize;
    if head.restart != 0 && (jrow.wrapping_mul(head.wide)) % head.restart == 0 {
        head.vpred = [1i32.wrapping_shl((head.bits - 1) as u32); 6];
        if jrow != 0 {
            stream.skip(-2);
            // Past the next restart marker.
            let mut mark = 0u16;
            while let Some(byte) = stream.getc() {
                mark = (mark << 8).wrapping_add(byte as u16);
                if mark >> 4 == 0xffd {
                    break;
                }
            }
        }
        bits.restart();
    }
    let current = width * (jrow & 1) as usize;
    let previous = width * ((jrow + 1) & 1) as usize;
    let mut spred = 0i32;
    for col in 0..head.wide as usize {
        for c in 0..clrs {
            let diff = ljpeg_diff(stream, bits, head.table(c), dng_version);
            let index = col * clrs + c;
            let mut pred;
            if head.sraw != 0 && c as i32 <= head.sraw && (col | c) != 0 {
                pred = spred;
            } else if col != 0 {
                pred = head.rows[current + index - clrs] as i32;
            } else {
                head.vpred[c] += diff;
                pred = head.vpred[c] - diff;
            }
            if jrow != 0 && col != 0 {
                let above = head.rows[previous + index] as i32;
                let above_left = head.rows[previous + index - clrs] as i32;
                pred = match head.psv {
                    1 => pred,
                    2 => above,
                    3 => above_left,
                    4 => pred + above - above_left,
                    5 => pred + ((above - above_left) >> 1),
                    6 => above + ((pred - above_left) >> 1),
                    7 => (pred + above) >> 1,
                    _ => 0,
                };
            }
            let value = pred.wrapping_add(diff) as u16;
            head.rows[current + index] = value;
            if (value as i32).checked_shr(head.bits as u32).unwrap_or(0) != 0 {
                bits.errors += 1;
            }
            if c as i32 <= head.sraw {
                spred = value as i32;
            }
        }
    }
    current
}

const ZIGZAG: [u8; 80] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63, 63, 63, 63, 63,
    63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63,
];

/// `ljpeg_idct`: one 8×8 DCT block of a lossy (SOF1) tile into `idct`.
pub(super) fn ljpeg_idct(stream: &mut Stream, bits: &mut BitReader, head: &mut JpegHead, dng_version: u32) {
    if head.cosines.is_empty() {
        head.cosines = (0..106).map(|index: i32| (((index & 31) as f64 * std::f64::consts::PI / 16.0).cos() / 2.0) as f32).collect();
    }
    let mut coefficients = [[0.0f32; 8]; 8];
    let diff = ljpeg_diff(stream, bits, head.table(0), dng_version);
    head.vpred[0] = head.vpred[0].wrapping_add(diff.wrapping_mul(head.quant[0] as i32));
    coefficients[0][0] = head.vpred[0] as f32;
    let mut index = 1usize;
    while index < 64 {
        let Some(table) = head.table(16) else {
            break;
        };
        let mut length = bits.gethuff(stream, table) as i32;
        let skip = length >> 4;
        index += skip as usize;
        length &= 15;
        if length == 0 && skip < 15 {
            break;
        }
        let mut coefficient = bits.getbits(stream, length) as i32;
        if coefficient & 1i32.wrapping_shl((length - 1) as u32) == 0 {
            coefficient -= 1i32.wrapping_shl(length as u32) - 1;
        }
        let position = ZIGZAG[index.min(79)] as usize;
        let quant = head.quant.get(index).copied().unwrap_or(0) as i32;
        coefficients[position / 8][position % 8] = coefficient.wrapping_mul(quant) as f32;
        index += 1;
    }
    let root_half = std::f64::consts::FRAC_1_SQRT_2 as f32;
    for value in coefficients[0].iter_mut() {
        *value *= root_half;
    }
    for row in coefficients.iter_mut() {
        row[0] *= root_half;
    }
    // Along rows, then along columns.
    let mut rows = [[0.0f32; 8]; 8];
    for (input_row, output_row) in coefficients.iter().zip(rows.iter_mut()) {
        for (j, value) in output_row.iter_mut().enumerate() {
            for (c, coefficient) in input_row.iter().enumerate() {
                *value += coefficient * head.cosines[(j * 2 + 1) * c];
            }
        }
    }
    let mut block = [[0.0f32; 8]; 8];
    for (i, output_row) in block.iter_mut().enumerate() {
        for (j, value) in output_row.iter_mut().enumerate() {
            for (c, input_row) in rows.iter().enumerate() {
                *value += input_row[j] * head.cosines[(i * 2 + 1) * c];
            }
        }
    }
    for (index, value) in head.idct.iter_mut().enumerate() {
        let sample = (block[index / 8][index % 8] as f64 + 0.5) as i32;
        *value = sample.clamp(0, 65535) as u16;
    }
}
