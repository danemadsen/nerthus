//! Arithmetic entropy decoding (`jdarith.c`): the QM decoder with its
//! statistics bins, for sequential and progressive scans. A marker inside
//! the data is legal here: from then on zeros are decoded. Bad data stops
//! the scan (until the next restart) after a warning.

use super::huffman::update_coef_bits;
use super::source::{read_restart_marker, MarkerState, Source};
use super::tables::{ARITAB, NATURAL_ORDER};
use super::{JpegError, ScanInfo};

const DC_STAT_BINS: usize = 64;
const AC_STAT_BINS: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    Sequential,
    DcFirst,
    AcFirst,
    DcRefine,
    AcRefine,
}

pub(super) struct Arithmetic {
    pass: Pass,
    c: i64,
    a: i64,
    /// Bit shift counter; -1 after bad data (nothing more is decoded).
    ct: i32,
    last_dc_val: [i32; 4],
    dc_context: [usize; 4],
    restarts_to_go: u32,
    dc_stats: Vec<[u8; DC_STAT_BINS]>,
    ac_stats: Vec<[u8; AC_STAT_BINS]>,
    /// The fixed-probability bin (state 113).
    fixed_bin: u8,
}

impl Arithmetic {
    /// `start_pass`: checks a progressive scan and updates the progression
    /// status, then resets the statistics this scan uses.
    #[allow(clippy::too_many_arguments)]
    pub fn start(scan: &ScanInfo, progressive: bool, coef_bits: &mut [[i32; 64]], component_count: usize, input_scan_number: i32, previous: Option<Arithmetic>, source: &mut Source) -> Result<Self, JpegError> {
        let pass = if progressive {
            let mut bad = false;
            if scan.ss == 0 {
                if scan.se != 0 {
                    bad = true;
                }
            } else if scan.se < scan.ss || scan.se > 63 || scan.comps_in_scan != 1 {
                bad = true;
            }
            if scan.ah != 0 && scan.ah - 1 != scan.al {
                bad = true;
            }
            if scan.al > 13 {
                bad = true;
            }
            if bad {
                return Err(JpegError::new(&format!("Invalid progressive parameters Ss={} Se={} Ah={} Al={}", scan.ss, scan.se, scan.ah, scan.al)));
            }
            update_coef_bits(scan, coef_bits, component_count, input_scan_number, scan.ss != 0, source);
            match (scan.ah == 0, scan.ss == 0) {
                (true, true) => Pass::DcFirst,
                (true, false) => Pass::AcFirst,
                (false, true) => Pass::DcRefine,
                (false, false) => Pass::AcRefine,
            }
        } else {
            if scan.ss != 0 || scan.se != 63 || scan.ah != 0 || scan.al != 0 {
                source.warn();
            }
            Pass::Sequential
        };
        // The statistics areas live as long as the image; a scan resets the
        // ones it uses.
        let (mut dc_stats, mut ac_stats) = match previous {
            Some(previous) => (previous.dc_stats, previous.ac_stats),
            None => (vec![[0u8; DC_STAT_BINS]; 16], vec![[0u8; AC_STAT_BINS]; 16]),
        };
        let mut decoder = Self { pass, c: 0, a: 0, ct: -16, last_dc_val: [0; 4], dc_context: [0; 4], restarts_to_go: scan.restart_interval, dc_stats: Vec::new(), ac_stats: Vec::new(), fixed_bin: 113 };
        for ci in 0..scan.comps_in_scan {
            if !progressive || (scan.ss == 0 && scan.ah == 0) {
                let table = scan.dc_tbl_no[ci];
                if table >= 16 {
                    return Err(JpegError::new("Arithmetic table not defined"));
                }
                dc_stats[table] = [0; DC_STAT_BINS];
                decoder.last_dc_val[ci] = 0;
                decoder.dc_context[ci] = 0;
            }
            if !progressive || scan.ss != 0 {
                let table = scan.ac_tbl_no[ci];
                if table >= 16 {
                    return Err(JpegError::new("Arithmetic table not defined"));
                }
                ac_stats[table] = [0; AC_STAT_BINS];
            }
        }
        decoder.dc_stats = dc_stats;
        decoder.ac_stats = ac_stats;
        Ok(decoder)
    }

    /// `arith_decode`: one binary decision with the bin `state` (updated).
    fn decode(&mut self, source: &mut Source, markers: &mut MarkerState, state: &mut u8) -> Result<i32, JpegError> {
        while self.a < 0x8000 {
            self.ct -= 1;
            if self.ct < 0 {
                let data: i64 = if markers.unread_marker != 0 {
                    0
                } else {
                    let mut data = source.read_byte()? as i64;
                    if data == 0xFF {
                        loop {
                            data = source.read_byte()? as i64;
                            if data != 0xFF {
                                break;
                            }
                        }
                        if data == 0 {
                            data = 0xFF;
                        } else {
                            markers.unread_marker = data as u8;
                            data = 0;
                        }
                    }
                    data
                };
                self.c = (self.c << 8) | data;
                self.ct += 8;
                if self.ct < 0 {
                    self.ct += 1;
                    if self.ct == 0 {
                        self.a = 0x8000;
                    }
                }
            }
            self.a <<= 1;
        }
        let sv = *state as i64;
        let mut qe = ARITAB[(sv & 0x7F) as usize];
        let nl = (qe & 0xFF) as u8;
        qe >>= 8;
        let nm = (qe & 0xFF) as u8;
        qe >>= 8;
        let mut sv = sv;
        let mut temp = self.a - qe;
        self.a = temp;
        temp <<= self.ct;
        if self.c >= temp {
            self.c -= temp;
            if self.a < qe {
                self.a = qe;
                *state = (sv as u8 & 0x80) ^ nm;
            } else {
                self.a = qe;
                *state = (sv as u8 & 0x80) ^ nl;
                sv ^= 0x80;
            }
        } else if self.a < 0x8000 {
            if self.a < qe {
                *state = (sv as u8 & 0x80) ^ nl;
                sv ^= 0x80;
            } else {
                *state = (sv as u8 & 0x80) ^ nm;
            }
        }
        Ok((sv >> 7) as i32)
    }

    /// `arith_decode` on a statistics bin of a DC table.
    fn decode_dc(&mut self, source: &mut Source, markers: &mut MarkerState, table: usize, bin: usize) -> Result<i32, JpegError> {
        let mut state = self.dc_stats[table][bin];
        let bit = self.decode(source, markers, &mut state)?;
        self.dc_stats[table][bin] = state;
        Ok(bit)
    }

    fn decode_ac(&mut self, source: &mut Source, markers: &mut MarkerState, table: usize, bin: usize) -> Result<i32, JpegError> {
        let mut state = self.ac_stats[table][bin];
        let bit = self.decode(source, markers, &mut state)?;
        self.ac_stats[table][bin] = state;
        Ok(bit)
    }

    fn decode_fixed(&mut self, source: &mut Source, markers: &mut MarkerState) -> Result<i32, JpegError> {
        let mut state = self.fixed_bin;
        let bit = self.decode(source, markers, &mut state)?;
        self.fixed_bin = state;
        Ok(bit)
    }

    /// `process_restart`.
    fn restart(&mut self, scan: &ScanInfo, progressive: bool, source: &mut Source, markers: &mut MarkerState) -> Result<(), JpegError> {
        read_restart_marker(source, markers)?;
        for ci in 0..scan.comps_in_scan {
            if !progressive || (scan.ss == 0 && scan.ah == 0) {
                self.dc_stats[scan.dc_tbl_no[ci]] = [0; DC_STAT_BINS];
                self.last_dc_val[ci] = 0;
                self.dc_context[ci] = 0;
            }
            if !progressive || scan.ss != 0 {
                self.ac_stats[scan.ac_tbl_no[ci]] = [0; AC_STAT_BINS];
            }
        }
        self.c = 0;
        self.a = 0;
        self.ct = -16;
        self.restarts_to_go = scan.restart_interval;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn decode_mcu(&mut self, scan: &ScanInfo, progressive: bool, arith_dc_l: &[u8; 16], arith_dc_u: &[u8; 16], arith_ac_k: &[u8; 16], source: &mut Source, markers: &mut MarkerState, blocks: &mut [[i16; 64]]) -> Result<(), JpegError> {
        if scan.restart_interval != 0 {
            if self.restarts_to_go == 0 {
                self.restart(scan, progressive, source, markers)?;
            }
            self.restarts_to_go = self.restarts_to_go.wrapping_sub(1);
        }
        match self.pass {
            Pass::Sequential => self.sequential(scan, arith_dc_l, arith_dc_u, arith_ac_k, source, markers, blocks),
            Pass::DcFirst => self.dc_first(scan, arith_dc_l, arith_dc_u, source, markers, blocks),
            Pass::AcFirst => self.ac_first(scan, arith_ac_k, source, markers, blocks),
            Pass::DcRefine => self.dc_refine(scan, source, markers, blocks),
            Pass::AcRefine => self.ac_refine(scan, source, markers, blocks),
        }
    }

    /// Figures F.19 – F.24: a DC difference, updating the conditioning
    /// category. `None` after a magnitude overflow.
    fn dc_difference(&mut self, ci: usize, table: usize, arith_dc_l: &[u8; 16], arith_dc_u: &[u8; 16], source: &mut Source, markers: &mut MarkerState) -> Result<Option<i32>, JpegError> {
        let mut bin = self.dc_context[ci];
        if self.decode_dc(source, markers, table, bin)? == 0 {
            self.dc_context[ci] = 0;
            return Ok(Some(0));
        }
        let sign = self.decode_dc(source, markers, table, bin + 1)?;
        bin += 2 + sign as usize;
        let mut m = self.decode_dc(source, markers, table, bin)?;
        if m != 0 {
            bin = 20;
            while self.decode_dc(source, markers, table, bin)? != 0 {
                m <<= 1;
                if m == 0x8000 {
                    source.warn();
                    self.ct = -1;
                    return Ok(None);
                }
                bin += 1;
            }
        }
        if m < ((1i64 << arith_dc_l[table]) >> 1) as i32 {
            self.dc_context[ci] = 0;
        } else if m > ((1i64 << arith_dc_u[table]) >> 1) as i32 {
            self.dc_context[ci] = 12 + sign as usize * 4;
        } else {
            self.dc_context[ci] = 4 + sign as usize * 4;
        }
        let mut v = m;
        bin += 14;
        loop {
            m >>= 1;
            if m == 0 {
                break;
            }
            if self.decode_dc(source, markers, table, bin)? != 0 {
                v |= m;
            }
        }
        v += 1;
        if sign != 0 {
            v = -v;
        }
        Ok(Some(v))
    }

    /// Figures F.20 – F.24: one AC coefficient at `k`. `Err` stops on bad
    /// data; `Ok(None)` is an end of block; otherwise the new `k` and value.
    fn ac_coefficient(&mut self, table: usize, mut k: usize, last: usize, arith_ac_k: &[u8; 16], source: &mut Source, markers: &mut MarkerState) -> Result<AcResult, JpegError> {
        let mut bin = 3 * (k - 1);
        if self.decode_ac(source, markers, table, bin)? != 0 {
            return Ok(AcResult::EndOfBlock);
        }
        while self.decode_ac(source, markers, table, bin + 1)? == 0 {
            bin += 3;
            k += 1;
            if k > last {
                source.warn();
                self.ct = -1;
                return Ok(AcResult::Stopped);
            }
        }
        let sign = self.decode_fixed(source, markers)?;
        bin += 2;
        let mut m = self.decode_ac(source, markers, table, bin)?;
        if m != 0 && self.decode_ac(source, markers, table, bin)? != 0 {
            m <<= 1;
            bin = if k <= arith_ac_k[table] as usize { 189 } else { 217 };
            while self.decode_ac(source, markers, table, bin)? != 0 {
                m <<= 1;
                if m == 0x8000 {
                    source.warn();
                    self.ct = -1;
                    return Ok(AcResult::Stopped);
                }
                bin += 1;
            }
        }
        let mut v = m;
        bin += 14;
        loop {
            m >>= 1;
            if m == 0 {
                break;
            }
            if self.decode_ac(source, markers, table, bin)? != 0 {
                v |= m;
            }
        }
        v += 1;
        if sign != 0 {
            v = -v;
        }
        Ok(AcResult::Coefficient(k, v))
    }

    #[allow(clippy::too_many_arguments)]
    fn sequential(&mut self, scan: &ScanInfo, arith_dc_l: &[u8; 16], arith_dc_u: &[u8; 16], arith_ac_k: &[u8; 16], source: &mut Source, markers: &mut MarkerState, blocks: &mut [[i16; 64]]) -> Result<(), JpegError> {
        if self.ct == -1 {
            return Ok(());
        }
        for (block_index, block) in blocks.iter_mut().enumerate().take(scan.blocks_in_mcu) {
            let ci = scan.mcu_membership[block_index];
            let Some(difference) = self.dc_difference(ci, scan.dc_tbl_no[ci], arith_dc_l, arith_dc_u, source, markers)? else {
                return Ok(());
            };
            if difference != 0 {
                self.last_dc_val[ci] = (self.last_dc_val[ci] + difference) & 0xffff;
            }
            block[0] = self.last_dc_val[ci] as i16;
            let table = scan.ac_tbl_no[ci];
            let mut k = 1;
            while k <= 63 {
                match self.ac_coefficient(table, k, 63, arith_ac_k, source, markers)? {
                    AcResult::EndOfBlock => break,
                    AcResult::Stopped => return Ok(()),
                    AcResult::Coefficient(position, value) => {
                        block[NATURAL_ORDER[position]] = value as i16;
                        k = position;
                    }
                }
                k += 1;
            }
        }
        Ok(())
    }

    fn dc_first(&mut self, scan: &ScanInfo, arith_dc_l: &[u8; 16], arith_dc_u: &[u8; 16], source: &mut Source, markers: &mut MarkerState, blocks: &mut [[i16; 64]]) -> Result<(), JpegError> {
        if self.ct == -1 {
            return Ok(());
        }
        for (block_index, block) in blocks.iter_mut().enumerate().take(scan.blocks_in_mcu) {
            let ci = scan.mcu_membership[block_index];
            let Some(difference) = self.dc_difference(ci, scan.dc_tbl_no[ci], arith_dc_l, arith_dc_u, source, markers)? else {
                return Ok(());
            };
            if difference != 0 {
                self.last_dc_val[ci] = (self.last_dc_val[ci] + difference) & 0xffff;
            }
            block[0] = ((self.last_dc_val[ci] as u32) << scan.al) as i16;
        }
        Ok(())
    }

    fn ac_first(&mut self, scan: &ScanInfo, arith_ac_k: &[u8; 16], source: &mut Source, markers: &mut MarkerState, blocks: &mut [[i16; 64]]) -> Result<(), JpegError> {
        if self.ct == -1 {
            return Ok(());
        }
        let block = &mut blocks[0];
        let table = scan.ac_tbl_no[0];
        let last = scan.se as usize;
        let mut k = scan.ss as usize;
        while k <= last {
            match self.ac_coefficient(table, k, last, arith_ac_k, source, markers)? {
                AcResult::EndOfBlock => break,
                AcResult::Stopped => return Ok(()),
                AcResult::Coefficient(position, value) => {
                    block[NATURAL_ORDER[position]] = ((value as u32) << scan.al) as i16;
                    k = position;
                }
            }
            k += 1;
        }
        Ok(())
    }

    fn dc_refine(&mut self, scan: &ScanInfo, source: &mut Source, markers: &mut MarkerState, blocks: &mut [[i16; 64]]) -> Result<(), JpegError> {
        let p1 = 1i32 << scan.al;
        for block in blocks.iter_mut().take(scan.blocks_in_mcu) {
            if self.decode_fixed(source, markers)? != 0 {
                block[0] = (block[0] as i32 | p1) as i16;
            }
        }
        Ok(())
    }

    fn ac_refine(&mut self, scan: &ScanInfo, source: &mut Source, markers: &mut MarkerState, blocks: &mut [[i16; 64]]) -> Result<(), JpegError> {
        if self.ct == -1 {
            return Ok(());
        }
        let block = &mut blocks[0];
        let table = scan.ac_tbl_no[0];
        let p1 = 1i32 << scan.al;
        let m1 = ((-1i32 as u32) << scan.al) as i32;
        let last = scan.se as usize;
        // The previous stage's end of block.
        let mut kex = last;
        while kex > 0 {
            if block[NATURAL_ORDER[kex]] != 0 {
                break;
            }
            kex -= 1;
        }
        let mut k = scan.ss as usize;
        while k <= last {
            let mut bin = 3 * (k - 1);
            if k > kex && self.decode_ac(source, markers, table, bin)? != 0 {
                break;
            }
            loop {
                let position = NATURAL_ORDER[k];
                if block[position] != 0 {
                    if self.decode_ac(source, markers, table, bin + 2)? != 0 {
                        let delta = if block[position] < 0 { m1 } else { p1 };
                        block[position] = block[position].wrapping_add(delta as i16);
                    }
                    break;
                }
                if self.decode_ac(source, markers, table, bin + 1)? != 0 {
                    block[position] = if self.decode_fixed(source, markers)? != 0 { m1 as i16 } else { p1 as i16 };
                    break;
                }
                bin += 3;
                k += 1;
                if k > last {
                    source.warn();
                    self.ct = -1;
                    return Ok(());
                }
            }
            k += 1;
        }
        Ok(())
    }
}

enum AcResult {
    EndOfBlock,
    Stopped,
    Coefficient(usize, i32),
}
