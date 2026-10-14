//! Huffman entropy decoding: the derived tables, the bit reader, and the
//! sequential, progressive and lossless MCU decoders.
//!
//! When the compressed data ends early the reader feeds zero bits (one
//! warning per segment); from the next MCU on, nothing more is decoded
//! until a restart marker, so the rest of the segment comes out flat.

use super::source::{read_restart_marker, MarkerState, Source};
use super::tables::NATURAL_ORDER;
use super::{HuffTable, JpegError, ScanInfo};

/// Bits of lookahead in the fast lookup table.
const LOOKAHEAD: i32 = 8;
/// The fewest bits a load of the bit buffer leaves (a 64-bit buffer).
const MIN_GET_BITS: i32 = 64 - 7;

/// `d_derived_tbl`: a Huffman table ready for decoding.
#[derive(Clone)]
pub(super) struct DerivedTable {
    /// Largest code of each length (-1 if none); `maxcode[17]` is a
    /// sentinel that ends any search.
    maxcode: [i64; 18],
    /// `huffval` index of each length's first symbol, less its code.
    valoffset: [i64; 18],
    /// The next 8 bits → code length (high byte) and symbol (low byte), or
    /// length 9 when the code is longer.
    lookup: [i32; 1 << LOOKAHEAD],
    huffval: [u8; 256],
}

/// `jpeg_make_d_derived_tbl`, with its checks of the table.
pub(super) fn derive_table(table: Option<&HuffTable>, is_dc: bool, lossless: bool) -> Result<DerivedTable, JpegError> {
    let table = table.ok_or_else(|| JpegError::new("Huffman table not defined"))?;
    let mut huffsize = [0u8; 257];
    let mut huffcode = [0u32; 257];
    let mut p = 0usize;
    for length in 1..=16 {
        let count = table.bits[length] as usize;
        if p + count > 256 {
            return Err(JpegError::new("Bogus Huffman table definition"));
        }
        for _ in 0..count {
            huffsize[p] = length as u8;
            p += 1;
        }
    }
    huffsize[p] = 0;
    let symbol_count = p;

    let mut code: u32 = 0;
    let mut size = huffsize[0] as u32;
    p = 0;
    while huffsize[p] != 0 {
        while huffsize[p] as u32 == size {
            huffcode[p] = code;
            p += 1;
            code += 1;
        }
        if code as i64 >= 1i64 << size {
            return Err(JpegError::new("Bogus Huffman table definition"));
        }
        code <<= 1;
        size += 1;
    }

    let mut derived = DerivedTable { maxcode: [0; 18], valoffset: [0; 18], lookup: [0; 1 << LOOKAHEAD], huffval: table.huffval };
    p = 0;
    for length in 1..=16 {
        if table.bits[length] != 0 {
            derived.valoffset[length] = p as i64 - huffcode[p] as i64;
            p += table.bits[length] as usize;
            derived.maxcode[length] = huffcode[p - 1] as i64;
        } else {
            derived.maxcode[length] = -1;
        }
    }
    derived.valoffset[17] = 0;
    derived.maxcode[17] = 0xFFFFF;

    for entry in derived.lookup.iter_mut() {
        *entry = (LOOKAHEAD + 1) << LOOKAHEAD;
    }
    p = 0;
    for length in 1..=LOOKAHEAD as usize {
        for _ in 0..table.bits[length] {
            let first = (huffcode[p] as usize) << (LOOKAHEAD as usize - length);
            let count = 1usize << (LOOKAHEAD as usize - length);
            let entry = ((length as i32) << LOOKAHEAD) | table.huffval[p] as i32;
            derived.lookup[first..first + count].fill(entry);
            p += 1;
        }
    }

    if is_dc {
        let limit = if lossless { 16 } else { 15 };
        if table.huffval[..symbol_count].iter().any(|&symbol| symbol > limit) {
            return Err(JpegError::new("Bogus Huffman table definition"));
        }
    }
    Ok(derived)
}

/// The bit buffer between MCUs (`bitread_perm_state`).
#[derive(Clone, Copy, Default)]
pub(super) struct BitBuffer {
    get_buffer: u64,
    bits_left: i32,
}

/// What the bit reader works with.
pub(super) struct BitReader<'s, 'a> {
    pub source: &'s mut Source<'a>,
    pub markers: &'s mut MarkerState,
    pub insufficient_data: &'s mut bool,
    get_buffer: u64,
    bits_left: i32,
}

impl<'s, 'a> BitReader<'s, 'a> {
    pub fn new(source: &'s mut Source<'a>, markers: &'s mut MarkerState, insufficient_data: &'s mut bool, state: BitBuffer) -> Self {
        Self { source, markers, insufficient_data, get_buffer: state.get_buffer, bits_left: state.bits_left }
    }

    pub fn state(&self) -> BitBuffer {
        BitBuffer { get_buffer: self.get_buffer, bits_left: self.bits_left }
    }

    /// `jpeg_fill_bit_buffer`: loads bytes until at least `MIN_GET_BITS`
    /// bits are held, undoing byte stuffing; at a marker it stops, and when
    /// fewer than `bits` are left it pads with zero bits.
    fn fill(&mut self, bits: i32) -> Result<(), JpegError> {
        let mut hit_marker = self.markers.unread_marker != 0;
        if !hit_marker {
            while self.bits_left < MIN_GET_BITS {
                let mut c = self.source.read_byte()? as u64;
                if c == 0xFF {
                    loop {
                        c = self.source.read_byte()? as u64;
                        if c != 0xFF {
                            break;
                        }
                    }
                    if c == 0 {
                        c = 0xFF;
                    } else {
                        self.markers.unread_marker = c as u8;
                        hit_marker = true;
                        break;
                    }
                }
                self.get_buffer = (self.get_buffer << 8) | c;
                self.bits_left += 8;
            }
        }
        if hit_marker && bits > self.bits_left {
            if !*self.insufficient_data {
                self.source.warn();
                *self.insufficient_data = true;
            }
            self.get_buffer <<= MIN_GET_BITS - self.bits_left;
            self.bits_left = MIN_GET_BITS;
        }
        Ok(())
    }

    fn check(&mut self, bits: i32) -> Result<(), JpegError> {
        if self.bits_left < bits {
            self.fill(bits)?;
        }
        Ok(())
    }

    fn get_bits(&mut self, bits: i32) -> i32 {
        self.bits_left -= bits;
        ((self.get_buffer >> self.bits_left) as i32) & ((1 << bits) - 1)
    }

    fn peek_bits(&self, bits: i32) -> i32 {
        ((self.get_buffer >> (self.bits_left - bits)) as i32) & ((1 << bits) - 1)
    }

    pub fn read_bits(&mut self, bits: i32) -> Result<i32, JpegError> {
        self.check(bits)?;
        Ok(self.get_bits(bits))
    }

    /// `HUFF_DECODE`: the next symbol.
    pub fn decode(&mut self, table: &DerivedTable) -> Result<i32, JpegError> {
        let mut min_bits = 1;
        if self.bits_left < LOOKAHEAD {
            self.fill(0)?;
            if self.bits_left < LOOKAHEAD {
                return self.decode_slow(table, min_bits);
            }
        }
        let look = self.peek_bits(LOOKAHEAD) as usize;
        let length = table.lookup[look] >> LOOKAHEAD;
        if length <= LOOKAHEAD {
            self.bits_left -= length;
            return Ok(table.lookup[look] & ((1 << LOOKAHEAD) - 1));
        }
        min_bits = length;
        self.decode_slow(table, min_bits)
    }

    /// `jpeg_huff_decode`: a code at least `min_bits` long, bit by bit;
    /// garbage that runs past 16 bits gives symbol 0 and a warning.
    fn decode_slow(&mut self, table: &DerivedTable, min_bits: i32) -> Result<i32, JpegError> {
        let mut length = min_bits;
        self.check(length)?;
        let mut code = self.get_bits(length) as i64;
        while code > table.maxcode[length as usize] {
            code <<= 1;
            self.check(1)?;
            code |= self.get_bits(1) as i64;
            length += 1;
        }
        if length > 16 {
            self.source.warn();
            return Ok(0);
        }
        Ok(table.huffval[(code + table.valoffset[length as usize]) as usize] as i32)
    }
}

/// `HUFF_EXTEND`: the signed value of `bits` received bits.
fn extend(value: i32, bits: i32) -> i32 {
    if value < (1 << (bits - 1)) {
        value.wrapping_add(((-1i32 as u32) << bits).wrapping_add(1) as i32)
    } else {
        value
    }
}

/// Throws away the buffered bits (counting whole bytes as skipped) and
/// reads the restart marker (`process_restart`'s common part).
fn read_restart(source: &mut Source, markers: &mut MarkerState, bits: &mut BitBuffer, insufficient_data: &mut bool) -> Result<(), JpegError> {
    markers.discarded_bytes += (bits.bits_left / 8) as usize;
    bits.bits_left = 0;
    read_restart_marker(source, markers)?;
    if markers.unread_marker == 0 {
        *insufficient_data = false;
    }
    Ok(())
}

/// Sequential Huffman decoding (`jdhuff.c`).
pub(super) struct Sequential {
    dc_tables: Vec<DerivedTable>,
    ac_tables: Vec<DerivedTable>,
    /// Each MCU block's tables (indexes into the lists above).
    block_dc: Vec<usize>,
    block_ac: Vec<usize>,
    bits: BitBuffer,
    last_dc_val: [i32; 4],
    restarts_to_go: u32,
    pub insufficient_data: bool,
}

impl Sequential {
    /// `start_pass_huff_decoder`.
    pub fn start(scan: &ScanInfo, dc: &[Option<HuffTable>; 4], ac: &[Option<HuffTable>; 4], source: &mut Source) -> Result<Self, JpegError> {
        if scan.ss != 0 || scan.se != 63 || scan.ah != 0 || scan.al != 0 {
            source.warn();
        }
        let mut dc_tables = Vec::new();
        let mut ac_tables = Vec::new();
        let mut comp_dc = Vec::new();
        let mut comp_ac = Vec::new();
        for ci in 0..scan.comps_in_scan {
            dc_tables.push(derive_table(dc[scan.dc_tbl_no[ci]].as_ref(), true, false)?);
            comp_dc.push(dc_tables.len() - 1);
            ac_tables.push(derive_table(ac[scan.ac_tbl_no[ci]].as_ref(), false, false)?);
            comp_ac.push(ac_tables.len() - 1);
        }
        let block_dc = scan.mcu_membership.iter().map(|&ci| comp_dc[ci]).collect();
        let block_ac = scan.mcu_membership.iter().map(|&ci| comp_ac[ci]).collect();
        Ok(Self { dc_tables, ac_tables, block_dc, block_ac, bits: BitBuffer::default(), last_dc_val: [0; 4], restarts_to_go: scan.restart_interval, insufficient_data: false })
    }

    /// `decode_mcu`.
    pub fn decode_mcu(&mut self, scan: &ScanInfo, source: &mut Source, markers: &mut MarkerState, blocks: &mut [[i16; 64]]) -> Result<(), JpegError> {
        if scan.restart_interval != 0 && self.restarts_to_go == 0 {
            read_restart(source, markers, &mut self.bits, &mut self.insufficient_data)?;
            self.last_dc_val = [0; 4];
            self.restarts_to_go = scan.restart_interval;
        }
        if !self.insufficient_data {
            let mut reader = BitReader::new(source, markers, &mut self.insufficient_data, self.bits);
            for (block_index, block) in blocks.iter_mut().enumerate().take(scan.blocks_in_mcu) {
                let dc_table = &self.dc_tables[self.block_dc[block_index]];
                let ac_table = &self.ac_tables[self.block_ac[block_index]];
                let mut s = reader.decode(dc_table)?;
                if s != 0 {
                    let r = reader.read_bits(s)?;
                    s = extend(r, s);
                }
                let ci = scan.mcu_membership[block_index];
                s = s.wrapping_add(self.last_dc_val[ci]);
                self.last_dc_val[ci] = s;
                block[0] = s as i16;
                let mut k = 1usize;
                while k < 64 {
                    let symbol = reader.decode(ac_table)?;
                    let mut r = symbol >> 4;
                    let mut s = symbol & 15;
                    if s != 0 {
                        k += r as usize;
                        r = reader.read_bits(s)?;
                        s = extend(r, s);
                        block[NATURAL_ORDER[k]] = s as i16;
                    } else {
                        if r != 15 {
                            break;
                        }
                        k += 15;
                    }
                    k += 1;
                }
            }
            self.bits = reader.state();
        }
        if scan.restart_interval != 0 {
            self.restarts_to_go = self.restarts_to_go.wrapping_sub(1);
        }
        Ok(())
    }
}

/// Which progressive pass a scan makes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ProgressivePass {
    DcFirst,
    AcFirst,
    DcRefine,
    AcRefine,
}

/// Progressive Huffman decoding (`jdphuff.c`).
pub(super) struct Progressive {
    pass: ProgressivePass,
    /// Each scan component's DC table (DC first passes).
    dc_tables: Vec<Option<DerivedTable>>,
    ac_table: Option<DerivedTable>,
    bits: BitBuffer,
    last_dc_val: [i32; 4],
    eob_run: u32,
    restarts_to_go: u32,
    pub insufficient_data: bool,
}

impl Progressive {
    /// `start_pass_phuff_decoder`: checks the scan, updates the progression
    /// status (`coef_bits`, with the previous scan's kept beside it).
    pub fn start(scan: &ScanInfo, dc: &[Option<HuffTable>; 4], ac: &[Option<HuffTable>; 4], coef_bits: &mut [[i32; 64]], component_count: usize, input_scan_number: i32, source: &mut Source) -> Result<Self, JpegError> {
        let is_dc_band = scan.ss == 0;
        let mut bad = false;
        if is_dc_band {
            if scan.se != 0 {
                bad = true;
            }
        } else {
            if scan.ss > scan.se || scan.se >= 64 {
                bad = true;
            }
            if scan.comps_in_scan != 1 {
                bad = true;
            }
        }
        if scan.ah != 0 && scan.al != scan.ah - 1 {
            bad = true;
        }
        if scan.al > 13 {
            bad = true;
        }
        if bad {
            return Err(JpegError::new(&format!("Invalid progressive parameters Ss={} Se={} Ah={} Al={}", scan.ss, scan.se, scan.ah, scan.al)));
        }
        update_coef_bits(scan, coef_bits, component_count, input_scan_number, !is_dc_band, source);
        let pass = match (scan.ah == 0, is_dc_band) {
            (true, true) => ProgressivePass::DcFirst,
            (true, false) => ProgressivePass::AcFirst,
            (false, true) => ProgressivePass::DcRefine,
            (false, false) => ProgressivePass::AcRefine,
        };
        let mut dc_tables = Vec::new();
        let mut ac_table = None;
        for ci in 0..scan.comps_in_scan {
            if is_dc_band {
                if scan.ah == 0 {
                    dc_tables.push(Some(derive_table(dc[scan.dc_tbl_no[ci]].as_ref(), true, false)?));
                } else {
                    dc_tables.push(None);
                }
            } else {
                ac_table = Some(derive_table(ac[scan.ac_tbl_no[ci]].as_ref(), false, false)?);
                dc_tables.push(None);
            }
        }
        Ok(Self { pass, dc_tables, ac_table, bits: BitBuffer::default(), last_dc_val: [0; 4], eob_run: 0, restarts_to_go: scan.restart_interval, insufficient_data: false })
    }

    pub fn decode_mcu(&mut self, scan: &ScanInfo, source: &mut Source, markers: &mut MarkerState, blocks: &mut [[i16; 64]]) -> Result<(), JpegError> {
        if scan.restart_interval != 0 && self.restarts_to_go == 0 {
            read_restart(source, markers, &mut self.bits, &mut self.insufficient_data)?;
            self.last_dc_val = [0; 4];
            self.eob_run = 0;
            self.restarts_to_go = scan.restart_interval;
        }
        match self.pass {
            ProgressivePass::DcFirst => self.dc_first(scan, source, markers, blocks)?,
            ProgressivePass::AcFirst => self.ac_first(scan, source, markers, blocks)?,
            ProgressivePass::DcRefine => self.dc_refine(scan, source, markers, blocks)?,
            ProgressivePass::AcRefine => self.ac_refine(scan, source, markers, blocks)?,
        }
        if scan.restart_interval != 0 {
            self.restarts_to_go = self.restarts_to_go.wrapping_sub(1);
        }
        Ok(())
    }

    fn dc_first(&mut self, scan: &ScanInfo, source: &mut Source, markers: &mut MarkerState, blocks: &mut [[i16; 64]]) -> Result<(), JpegError> {
        if self.insufficient_data {
            return Ok(());
        }
        let mut reader = BitReader::new(source, markers, &mut self.insufficient_data, self.bits);
        for (block_index, block) in blocks.iter_mut().enumerate().take(scan.blocks_in_mcu) {
            let ci = scan.mcu_membership[block_index];
            let table = self.dc_tables[ci].as_ref().expect("a DC first scan has DC tables");
            let mut s = reader.decode(table)?;
            if s != 0 {
                let r = reader.read_bits(s)?;
                s = extend(r, s);
            }
            let last = self.last_dc_val[ci];
            if (last >= 0 && s > i32::MAX - last) || (last < 0 && s < i32::MIN - last) {
                return Err(JpegError::new("DCT coefficient out of range"));
            }
            s += last;
            self.last_dc_val[ci] = s;
            block[0] = ((s as u32) << scan.al) as i16;
        }
        self.bits = reader.state();
        Ok(())
    }

    fn ac_first(&mut self, scan: &ScanInfo, source: &mut Source, markers: &mut MarkerState, blocks: &mut [[i16; 64]]) -> Result<(), JpegError> {
        if self.insufficient_data {
            return Ok(());
        }
        if self.eob_run > 0 {
            self.eob_run -= 1;
            return Ok(());
        }
        let mut reader = BitReader::new(source, markers, &mut self.insufficient_data, self.bits);
        let table = self.ac_table.as_ref().expect("an AC scan has its table");
        let block = &mut blocks[0];
        let mut eob_run = 0u32;
        let mut k = scan.ss as usize;
        while k <= scan.se as usize {
            let symbol = reader.decode(table)?;
            let mut r = symbol >> 4;
            let mut s = symbol & 15;
            if s != 0 {
                k += r as usize;
                r = reader.read_bits(s)?;
                s = extend(r, s);
                block[NATURAL_ORDER[k]] = ((s as u32) << scan.al) as i16;
            } else if r == 15 {
                k += 15;
            } else {
                eob_run = 1 << r;
                if r != 0 {
                    r = reader.read_bits(r)?;
                    eob_run += r as u32;
                }
                eob_run -= 1;
                break;
            }
            k += 1;
        }
        self.bits = reader.state();
        self.eob_run = eob_run;
        Ok(())
    }

    fn dc_refine(&mut self, scan: &ScanInfo, source: &mut Source, markers: &mut MarkerState, blocks: &mut [[i16; 64]]) -> Result<(), JpegError> {
        let p1 = 1i32 << scan.al;
        let mut reader = BitReader::new(source, markers, &mut self.insufficient_data, self.bits);
        for block in blocks.iter_mut().take(scan.blocks_in_mcu) {
            if reader.read_bits(1)? != 0 {
                block[0] = (block[0] as i32 | p1) as i16;
            }
        }
        self.bits = reader.state();
        Ok(())
    }

    fn ac_refine(&mut self, scan: &ScanInfo, source: &mut Source, markers: &mut MarkerState, blocks: &mut [[i16; 64]]) -> Result<(), JpegError> {
        if self.insufficient_data {
            return Ok(());
        }
        let p1 = 1i32 << scan.al;
        let m1 = ((-1i32 as u32) << scan.al) as i32;
        let mut reader = BitReader::new(source, markers, &mut self.insufficient_data, self.bits);
        let table = self.ac_table.as_ref().expect("an AC scan has its table");
        let block = &mut blocks[0];
        let mut eob_run = self.eob_run;
        let mut k = scan.ss as usize;
        let se = scan.se as usize;
        if eob_run == 0 {
            while k <= se {
                let symbol = reader.decode(table)?;
                let mut r = symbol >> 4;
                let mut s = symbol & 15;
                if s != 0 {
                    if s != 1 {
                        reader.source.warn();
                    }
                    s = if reader.read_bits(1)? != 0 { p1 } else { m1 };
                } else if r != 15 {
                    eob_run = 1 << r;
                    if r != 0 {
                        r = reader.read_bits(r)?;
                        eob_run += r as u32;
                    }
                    break;
                }
                loop {
                    let position = NATURAL_ORDER[k];
                    if block[position] != 0 {
                        if reader.read_bits(1)? != 0 && (block[position] as i32 & p1) == 0 {
                            let delta = if block[position] >= 0 { p1 } else { m1 };
                            block[position] = block[position].wrapping_add(delta as i16);
                        }
                    } else {
                        r -= 1;
                        if r < 0 {
                            break;
                        }
                    }
                    k += 1;
                    if k > se {
                        break;
                    }
                }
                if s != 0 {
                    block[NATURAL_ORDER[k]] = s as i16;
                }
                k += 1;
            }
        }
        if eob_run > 0 {
            while k <= se {
                let position = NATURAL_ORDER[k];
                if block[position] != 0 && reader.read_bits(1)? != 0 && (block[position] as i32 & p1) == 0 {
                    let delta = if block[position] >= 0 { p1 } else { m1 };
                    block[position] = block[position].wrapping_add(delta as i16);
                }
                k += 1;
            }
            eob_run -= 1;
        }
        self.bits = reader.state();
        self.eob_run = eob_run;
        Ok(())
    }
}

/// The progression status update shared by the Huffman and arithmetic
/// progressive decoders: the previous scan's status kept, warnings for an
/// AC scan without DC or a pass out of order, then the new status.
pub(super) fn update_coef_bits(scan: &ScanInfo, coef_bits: &mut [[i32; 64]], component_count: usize, input_scan_number: i32, is_ac_scan: bool, source: &mut Source) {
    for ci in 0..scan.comps_in_scan {
        let index = scan.component_index[ci];
        if is_ac_scan && coef_bits[index][0] < 0 {
            source.warn();
        }
        let first = scan.ss.min(1) as usize;
        let last = scan.se.max(9) as usize;
        let current_row = coef_bits[index];
        let previous_row = &mut coef_bits[index + component_count];
        for (previous, &current) in previous_row[first..=last].iter_mut().zip(current_row[first..=last].iter()) {
            *previous = if input_scan_number > 1 { current } else { 0 };
        }
        for bits in &mut coef_bits[index][scan.ss as usize..=scan.se as usize] {
            let expected = if *bits < 0 { 0 } else { *bits };
            if scan.ah != expected {
                source.warn();
            }
            *bits = scan.al;
        }
    }
}

/// Lossless Huffman decoding (`jdlhuff.c`): sample differences.
pub(super) struct Lossless {
    tables: Vec<DerivedTable>,
    block_table: Vec<usize>,
    bits: BitBuffer,
    pub insufficient_data: bool,
}

impl Lossless {
    pub fn start(scan: &ScanInfo, dc: &[Option<HuffTable>; 4]) -> Result<Self, JpegError> {
        let mut tables = Vec::new();
        let mut comp_table = Vec::new();
        for ci in 0..scan.comps_in_scan {
            let number = scan.dc_tbl_no[ci];
            tables.push(derive_table(dc[number].as_ref(), true, true)?);
            comp_table.push(tables.len() - 1);
        }
        let block_table = scan.mcu_membership.iter().map(|&ci| comp_table[ci]).collect();
        Ok(Self { tables, block_table, bits: BitBuffer::default(), insufficient_data: false })
    }

    /// `process_restart`.
    pub fn restart(&mut self, source: &mut Source, markers: &mut MarkerState) -> Result<(), JpegError> {
        read_restart(source, markers, &mut self.bits, &mut self.insufficient_data)
    }

    /// `decode_mcus`: the differences of `count` MCUs, each MCU's samples
    /// in order. Returns false when it found the data had run out (the
    /// differences are then zero and the predictors start over).
    pub fn decode_mcus(&mut self, scan: &ScanInfo, source: &mut Source, markers: &mut MarkerState, count: usize, differences: &mut Vec<i32>) -> Result<bool, JpegError> {
        differences.clear();
        if self.insufficient_data {
            differences.resize(count * scan.blocks_in_mcu, 0);
            return Ok(false);
        }
        let mut reader = BitReader::new(source, markers, &mut self.insufficient_data, self.bits);
        for _ in 0..count {
            for sample in 0..scan.blocks_in_mcu {
                let table = &self.tables[self.block_table[sample]];
                let mut s = reader.decode(table)?;
                if s != 0 {
                    if s == 16 {
                        s = 32768;
                    } else {
                        let r = reader.read_bits(s)?;
                        s = extend(r, s);
                    }
                }
                differences.push(s);
            }
            self.bits = reader.state();
        }
        Ok(true)
    }
}
