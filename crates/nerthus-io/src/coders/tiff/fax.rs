//! libtiff's CCITT decoders (`tif_fax3.c`, `tif_fax3.h`): Group 3 one- and
//! two-dimensional, Group 4, and Modified Huffman RLE (byte- or
//! word-aligned rows), driven by mkg3states' tables. Rows decode to runs
//! that fill the scanline (white clears bits, black sets them).
//!
//! Bad codes, uncompressed extensions and run overflows are libtiff
//! errors; a premature end of data is only a warning, but makes the strip
//! read fail quietly (Group 4 keeps what it decoded).

use std::sync::OnceLock;

use anyhow::{bail, Result};

use super::directory::*;

const S_PASS: u8 = 1;
const S_HORIZ: u8 = 2;
const S_V0: u8 = 3;
const S_VR: u8 = 4;
const S_VL: u8 = 5;
const S_EXT: u8 = 6;
const S_TERM_W: u8 = 7;
const S_TERM_B: u8 = 8;
const S_MAKE_UP_W: u8 = 9;
const S_MAKE_UP_B: u8 = 10;
const S_MAKE_UP: u8 = 11;
const S_EOL: u8 = 12;

pub(super) const FAXMODE_NORTC: u32 = 0x0001;
pub(super) const FAXMODE_NOEOL: u32 = 0x0002;
pub(super) const FAXMODE_BYTEALIGN: u32 = 0x0004;
pub(super) const FAXMODE_WORDALIGN: u32 = 0x0008;

const GROUP3OPT_2DENCODING: u32 = 0x1;

/// A state table entry (`TIFFFaxTabEnt`).
#[derive(Clone, Copy, Default)]
struct Entry {
    state: u8,
    width: u8,
    param: u32,
}

struct Tables {
    main: Vec<Entry>,
    white: Vec<Entry>,
    black: Vec<Entry>,
}

/// mkg3states' code prototypes: (code, (pixel count << 4) + width).
const PASS: &[(u16, u16)] = &[(0x0008, 4)];
const HORIZ: &[(u16, u16)] = &[(0x0004, 3)];
const V0: &[(u16, u16)] = &[(0x0001, 1)];
const VR: &[(u16, u16)] = &[(0x0006, (1 << 4) + 3), (0x0030, (2 << 4) + 6), (0x0060, (3 << 4) + 7)];
const VL: &[(u16, u16)] = &[(0x0002, (1 << 4) + 3), (0x0010, (2 << 4) + 6), (0x0020, (3 << 4) + 7)];
const EXT: &[(u16, u16)] = &[(0x0040, 7)];
const EOLV: &[(u16, u16)] = &[(0x0000, 7)];
const MAKE_UP_W: &[(u16, u16)] = &[
    (0x001b, 1029), (0x0009, 2053), (0x003a, 3078), (0x0076, 4103), (0x006c, 5128), (0x00ec, 6152), (0x0026, 7176), (0x00a6, 8200), (0x0016, 9224), (0x00e6, 10248), (0x0066, 11273), (0x0166, 12297),
    (0x0096, 13321), (0x0196, 14345), (0x0056, 15369), (0x0156, 16393), (0x00d6, 17417), (0x01d6, 18441), (0x0036, 19465), (0x0136, 20489), (0x00b6, 21513), (0x01b6, 22537), (0x0032, 23561), (0x0132, 24585),
    (0x00b2, 25609), (0x0006, 26630), (0x01b2, 27657),
];
const MAKE_UP_B: &[(u16, u16)] = &[
    (0x03c0, 1034), (0x0130, 2060), (0x0930, 3084), (0x0da0, 4108), (0x0cc0, 5132), (0x02c0, 6156), (0x0ac0, 7180), (0x06c0, 8205), (0x16c0, 9229), (0x0a40, 10253), (0x1a40, 11277), (0x0640, 12301),
    (0x1640, 13325), (0x09c0, 14349), (0x19c0, 15373), (0x05c0, 16397), (0x15c0, 17421), (0x0dc0, 18445), (0x1dc0, 19469), (0x0940, 20493), (0x1940, 21517), (0x0540, 22541), (0x1540, 23565), (0x0b40, 24589),
    (0x1b40, 25613), (0x04c0, 26637), (0x14c0, 27661),
];
const MAKE_UP: &[(u16, u16)] = &[
    (0x0080, 28683), (0x0180, 29707), (0x0580, 30731), (0x0480, 31756), (0x0c80, 32780), (0x0280, 33804), (0x0a80, 34828), (0x0680, 35852), (0x0e80, 36876), (0x0380, 37900), (0x0b80, 38924), (0x0780, 39948),
    (0x0f80, 40972),
];
const TERM_W: &[(u16, u16)] = &[
    (0x00ac, 8), (0x0038, 22), (0x000e, 36), (0x0001, 52), (0x000d, 68), (0x0003, 84), (0x0007, 100), (0x000f, 116), (0x0019, 133), (0x0005, 149), (0x001c, 165), (0x0002, 181), (0x0004, 198),
    (0x0030, 214), (0x000b, 230), (0x002b, 246), (0x0015, 262), (0x0035, 278), (0x0072, 295), (0x0018, 311), (0x0008, 327), (0x0074, 343), (0x0060, 359), (0x0010, 375), (0x000a, 391), (0x006a, 407),
    (0x0064, 423), (0x0012, 439), (0x000c, 455), (0x0040, 472), (0x00c0, 488), (0x0058, 504), (0x00d8, 520), (0x0048, 536), (0x00c8, 552), (0x0028, 568), (0x00a8, 584), (0x0068, 600), (0x00e8, 616),
    (0x0014, 632), (0x0094, 648), (0x0054, 664), (0x00d4, 680), (0x0034, 696), (0x00b4, 712), (0x0020, 728), (0x00a0, 744), (0x0050, 760), (0x00d0, 776), (0x004a, 792), (0x00ca, 808), (0x002a, 824),
    (0x00aa, 840), (0x0024, 856), (0x00a4, 872), (0x001a, 888), (0x009a, 904), (0x005a, 920), (0x00da, 936), (0x0052, 952), (0x00d2, 968), (0x004c, 984), (0x00cc, 1000), (0x002c, 1016),
];
const TERM_B: &[(u16, u16)] = &[
    (0x03b0, 10), (0x0002, 19), (0x0003, 34), (0x0001, 50), (0x0006, 67), (0x000c, 84), (0x0004, 100), (0x0018, 117), (0x0028, 134), (0x0008, 150), (0x0010, 167), (0x0050, 183), (0x0070, 199),
    (0x0020, 216), (0x00e0, 232), (0x0030, 249), (0x03a0, 266), (0x0060, 282), (0x0040, 298), (0x0730, 315), (0x00b0, 331), (0x01b0, 347), (0x0760, 363), (0x00a0, 379), (0x0740, 395), (0x00c0, 411),
    (0x0530, 428), (0x0d30, 444), (0x0330, 460), (0x0b30, 476), (0x0160, 492), (0x0960, 508), (0x0560, 524), (0x0d60, 540), (0x04b0, 556), (0x0cb0, 572), (0x02b0, 588), (0x0ab0, 604), (0x06b0, 620),
    (0x0eb0, 636), (0x0360, 652), (0x0b60, 668), (0x05b0, 684), (0x0db0, 700), (0x02a0, 716), (0x0aa0, 732), (0x06a0, 748), (0x0ea0, 764), (0x0260, 780), (0x0a60, 796), (0x04a0, 812), (0x0ca0, 828),
    (0x0240, 844), (0x0ec0, 860), (0x01c0, 876), (0x0e40, 892), (0x0140, 908), (0x01a0, 924), (0x09a0, 940), (0x0d40, 956), (0x0340, 972), (0x05a0, 988), (0x0660, 1004), (0x0e60, 1020),
];
const EOLH: &[(u16, u16)] = &[(0x0000, 11)];

/// mkg3states' `FillTable`.
fn fill_table(table: &mut [Entry], size: u32, prototypes: &[(u16, u16)], state: u8) {
    let limit = 1usize << size;
    for &(code, value) in prototypes {
        let width = (value & 15) as u8;
        let param = (value >> 4) as u32;
        let step = 1usize << width;
        let mut index = code as usize;
        while index < limit {
            table[index] = Entry { state, width, param };
            index += step;
        }
    }
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut main = vec![Entry::default(); 128];
        let mut white = vec![Entry::default(); 4096];
        let mut black = vec![Entry::default(); 8192];
        fill_table(&mut main, 7, PASS, S_PASS);
        fill_table(&mut main, 7, HORIZ, S_HORIZ);
        fill_table(&mut main, 7, V0, S_V0);
        fill_table(&mut main, 7, VR, S_VR);
        fill_table(&mut main, 7, VL, S_VL);
        fill_table(&mut main, 7, EXT, S_EXT);
        fill_table(&mut main, 7, EOLV, S_EOL);
        fill_table(&mut white, 12, MAKE_UP_W, S_MAKE_UP_W);
        fill_table(&mut white, 12, MAKE_UP, S_MAKE_UP);
        fill_table(&mut white, 12, TERM_W, S_TERM_W);
        fill_table(&mut white, 12, EOLH, S_EOL);
        fill_table(&mut black, 13, MAKE_UP_B, S_MAKE_UP_B);
        fill_table(&mut black, 13, MAKE_UP, S_MAKE_UP);
        fill_table(&mut black, 13, TERM_B, S_TERM_B);
        fill_table(&mut black, 13, EOLH, S_EOL);
        Tables { main, white, black }
    })
}

/// How a row's decoding ended.
enum Expand {
    Done,
    /// The data ran out (`eoflab`).
    Eof,
}

/// The decoder's registers (`DECLARE_STATE`).
struct Bits<'a> {
    data: &'a [u8],
    position: usize,
    reverse: bool,
    acc: u32,
    avail: i32,
}

impl Bits<'_> {
    fn end_of_data(&self) -> bool {
        self.position >= self.data.len()
    }

    fn load(&mut self) {
        let byte = self.data[self.position];
        self.position += 1;
        let byte = if self.reverse { byte.reverse_bits() } else { byte };
        self.acc |= (byte as u32) << self.avail;
        self.avail += 8;
    }

    /// `NeedBits8`: false when the data ended with no bits left.
    fn need8(&mut self, n: i32) -> bool {
        if self.avail < n {
            if self.end_of_data() {
                if self.avail == 0 {
                    return false;
                }
                self.avail = n;
            } else {
                self.load();
            }
        }
        true
    }

    /// `NeedBits16`.
    fn need16(&mut self, n: i32) -> bool {
        if self.avail < n {
            if self.end_of_data() {
                if self.avail == 0 {
                    return false;
                }
                self.avail = n;
            } else {
                self.load();
                if self.avail < n {
                    if self.end_of_data() {
                        self.avail = n;
                    } else {
                        self.load();
                    }
                }
            }
        }
        true
    }

    fn get(&self, n: i32) -> u32 {
        self.acc & ((1u32 << n) - 1)
    }

    fn clear(&mut self, n: i32) {
        self.avail -= n;
        self.acc = if n >= 32 { 0 } else { self.acc >> n };
    }

    /// `LOOKUP8`/`LOOKUP16`.
    fn lookup(&mut self, width: i32, table: &[Entry], sixteen: bool) -> Option<Entry> {
        let available = if sixteen { self.need16(width) } else { self.need8(width) };
        if !available {
            return None;
        }
        let entry = table[self.get(width) as usize];
        self.clear(entry.width as i32);
        Some(entry)
    }
}

/// One row's runs being built (`pa`, `a0`, `RunLength`, `lastx`).
struct Row<'a> {
    runs: &'a mut [u32],
    start: usize,
    end: usize,
    pa: usize,
    a0: i32,
    run_length: i32,
    last_x: i32,
}

impl Row<'_> {
    /// `SETVALUE`.
    fn set_value(&mut self, value: i32) -> Result<()> {
        if self.pa >= self.end {
            bail!("Buffer overflow at line of strip");
        }
        self.runs[self.pa] = self.run_length.wrapping_add(value) as u32;
        self.pa += 1;
        self.a0 = self.a0.wrapping_add(value);
        self.run_length = 0;
        Ok(())
    }

    /// `CLEANUP_RUNS`: the row brought to exactly `lastx` pixels.
    fn cleanup(&mut self) -> Result<()> {
        if self.run_length != 0 {
            self.set_value(0)?;
        }
        if self.a0 != self.last_x {
            while self.a0 > self.last_x && self.pa > self.start {
                self.pa -= 1;
                self.a0 = (self.a0 as u32).wrapping_sub(self.runs[self.pa]) as i32;
            }
            if self.a0 < self.last_x {
                if self.a0 < 0 {
                    self.a0 = 0;
                }
                if (self.pa - self.start) & 1 == 1 {
                    self.set_value(0)?;
                }
                self.set_value(self.last_x - self.a0)?;
            } else if self.a0 > self.last_x {
                self.set_value(self.last_x)?;
                self.set_value(0)?;
            }
        }
        Ok(())
    }
}

/// `EXPAND1D`: alternating white and black runs to the row's end or an
/// EOL.
fn expand_1d(bits: &mut Bits, row: &mut Row, eol_count: &mut i32) -> Result<Expand> {
    let tables = tables();
    loop {
        loop {
            let Some(entry) = bits.lookup(12, &tables.white, true) else {
                row.cleanup()?;
                return Ok(Expand::Eof);
            };
            match entry.state {
                S_EOL => {
                    *eol_count = 1;
                    row.cleanup()?;
                    return Ok(Expand::Done);
                }
                S_TERM_W => {
                    row.set_value(entry.param as i32)?;
                    break;
                }
                S_MAKE_UP_W | S_MAKE_UP => {
                    row.a0 = row.a0.wrapping_add(entry.param as i32);
                    row.run_length = row.run_length.wrapping_add(entry.param as i32);
                }
                _ => bail!("Bad code word at line of strip (WhiteTable)"),
            }
        }
        if row.a0 >= row.last_x {
            row.cleanup()?;
            return Ok(Expand::Done);
        }
        loop {
            let Some(entry) = bits.lookup(13, &tables.black, true) else {
                row.cleanup()?;
                return Ok(Expand::Eof);
            };
            match entry.state {
                S_EOL => {
                    *eol_count = 1;
                    row.cleanup()?;
                    return Ok(Expand::Done);
                }
                S_TERM_B => {
                    row.set_value(entry.param as i32)?;
                    break;
                }
                S_MAKE_UP_B | S_MAKE_UP => {
                    row.a0 = row.a0.wrapping_add(entry.param as i32);
                    row.run_length = row.run_length.wrapping_add(entry.param as i32);
                }
                _ => bail!("Bad code word at line of strip (BlackTable)"),
            }
        }
        if row.a0 >= row.last_x {
            row.cleanup()?;
            return Ok(Expand::Done);
        }
        if row.pa >= row.start + 2 && row.runs[row.pa - 1] == 0 && row.runs[row.pa - 2] == 0 {
            row.pa -= 2;
        }
    }
}

/// A horizontal mode's two runs, in `first` then the other colour.
fn horizontal_runs(bits: &mut Bits, row: &mut Row, black_first: bool) -> Result<Option<()>> {
    let tables = tables();
    for black in [black_first, !black_first] {
        loop {
            let (width, table) = if black { (13, &tables.black) } else { (12, &tables.white) };
            let Some(entry) = bits.lookup(width, table, true) else {
                return Ok(None);
            };
            match entry.state {
                S_TERM_B if black => {
                    row.set_value(entry.param as i32)?;
                    break;
                }
                S_TERM_W if !black => {
                    row.set_value(entry.param as i32)?;
                    break;
                }
                S_MAKE_UP_B if black => {
                    row.a0 = row.a0.wrapping_add(entry.param as i32);
                    row.run_length = row.run_length.wrapping_add(entry.param as i32);
                }
                S_MAKE_UP_W if !black => {
                    row.a0 = row.a0.wrapping_add(entry.param as i32);
                    row.run_length = row.run_length.wrapping_add(entry.param as i32);
                }
                S_MAKE_UP => {
                    row.a0 = row.a0.wrapping_add(entry.param as i32);
                    row.run_length = row.run_length.wrapping_add(entry.param as i32);
                }
                _ => bail!("Bad code word at line of strip ({})", if black { "BlackTable" } else { "WhiteTable" }),
            }
        }
    }
    Ok(Some(()))
}

/// The reference line's next change position (`b1`) and run (`pb`).
struct Reference {
    pb: usize,
    end: usize,
    b1: i32,
}

/// `CHECK_b1`.
fn check_b1(row: &Row, reference: &mut Reference, references: &[u32]) -> Result<()> {
    if row.pa != row.start {
        while reference.b1 <= row.a0 && reference.b1 < row.last_x {
            if reference.pb + 1 >= reference.end {
                bail!("Buffer overflow at line of strip");
            }
            let step = references[reference.pb].wrapping_add(references[reference.pb + 1]);
            reference.b1 = (reference.b1 as u32).wrapping_add(step) as i32;
            reference.pb += 2;
        }
    }
    Ok(())
}

/// `EXPAND2D`: modes against the reference line to the row's end.
fn expand_2d(bits: &mut Bits, row: &mut Row, reference: &mut Reference, references: &[u32], eol_count: &mut i32) -> Result<Expand> {
    let tables = tables();
    while row.a0 < row.last_x {
        if row.pa >= row.end {
            bail!("Buffer overflow at line of strip");
        }
        let Some(entry) = bits.lookup(7, &tables.main, false) else {
            row.cleanup()?;
            return Ok(Expand::Eof);
        };
        match entry.state {
            S_PASS => {
                check_b1(row, reference, references)?;
                if reference.pb + 1 >= reference.end {
                    bail!("Buffer overflow at line of strip");
                }
                reference.b1 = (reference.b1 as u32).wrapping_add(references[reference.pb]) as i32;
                reference.pb += 1;
                row.run_length = row.run_length.wrapping_add(reference.b1.wrapping_sub(row.a0));
                row.a0 = reference.b1;
                reference.b1 = (reference.b1 as u32).wrapping_add(references[reference.pb]) as i32;
                reference.pb += 1;
            }
            S_HORIZ => {
                let black_first = (row.pa - row.start) & 1 == 1;
                if horizontal_runs(bits, row, black_first)?.is_none() {
                    row.cleanup()?;
                    return Ok(Expand::Eof);
                }
                check_b1(row, reference, references)?;
            }
            S_V0 | S_VR => {
                check_b1(row, reference, references)?;
                let offset = if entry.state == S_VR { entry.param as i32 } else { 0 };
                row.set_value(reference.b1.wrapping_sub(row.a0).wrapping_add(offset))?;
                if reference.pb >= reference.end {
                    bail!("Buffer overflow at line of strip");
                }
                reference.b1 = (reference.b1 as u32).wrapping_add(references[reference.pb]) as i32;
                reference.pb += 1;
            }
            S_VL => {
                check_b1(row, reference, references)?;
                if reference.b1 < row.a0.wrapping_add(entry.param as i32) {
                    bail!("Bad code word at line of strip (VL)");
                }
                row.set_value(reference.b1.wrapping_sub(row.a0).wrapping_sub(entry.param as i32))?;
                reference.pb -= 1;
                reference.b1 = (reference.b1 as u32).wrapping_sub(references[reference.pb]) as i32;
            }
            S_EXT => bail!("Uncompressed data (not supported) at line of strip"),
            S_EOL => {
                row.runs[row.pa] = row.last_x.wrapping_sub(row.a0) as u32;
                row.pa += 1;
                if !bits.need8(4) {
                    row.cleanup()?;
                    return Ok(Expand::Eof);
                }
                if bits.get(4) != 0 {
                    bail!("Bad code word at line of strip (EOL)");
                }
                bits.clear(4);
                *eol_count = 1;
                row.cleanup()?;
                return Ok(Expand::Done);
            }
            _ => bail!("Bad code word at line of strip (MainTable)"),
        }
    }
    if row.run_length != 0 {
        if row.run_length.wrapping_add(row.a0) < row.last_x {
            if !bits.need8(1) {
                row.cleanup()?;
                return Ok(Expand::Eof);
            }
            if bits.get(1) == 0 {
                bail!("Bad code word at line of strip (MainTable)");
            }
            bits.clear(1);
        }
        row.set_value(0)?;
    }
    row.cleanup()?;
    Ok(Expand::Done)
}

const FILL_MASKS: [u8; 9] = [0x00, 0x80, 0xc0, 0xe0, 0xf0, 0xf8, 0xfc, 0xfe, 0xff];

/// `_TIFFFax3fillruns`: white runs clear bits, black runs set them.
fn fill_runs(buffer: &mut [u8], runs: &mut [u32], start: usize, mut end: usize, last_x: u32) {
    if (end - start) & 1 == 1 && end < runs.len() {
        runs[end] = 0;
        end += 1;
    }
    let mut x: u32 = 0;
    let mut index = start;
    while index < end {
        for black in [false, true] {
            let slot = index + usize::from(black);
            let mut run = runs[slot];
            if x.wrapping_add(run) > last_x || run > last_x {
                run = last_x.wrapping_sub(x);
                runs[slot] = run;
            }
            if run == 0 {
                continue;
            }
            let mut cp = (x >> 3) as usize;
            let bx = x & 7;
            if run > 8 - bx {
                if bx != 0 {
                    if black {
                        buffer[cp] |= 0xff >> bx;
                    } else {
                        buffer[cp] &= 0xff << (8 - bx);
                    }
                    cp += 1;
                    run -= 8 - bx;
                }
                let bytes = (run >> 3) as usize;
                if bytes != 0 {
                    buffer[cp..cp + bytes].fill(if black { 0xff } else { 0x00 });
                    cp += bytes;
                    run &= 7;
                }
                if run != 0 {
                    if black {
                        buffer[cp] = ((buffer[cp] as u32 | (0xff00u32 >> run)) & 0xff) as u8;
                    } else {
                        buffer[cp] &= 0xff >> run;
                    }
                }
            } else if black {
                buffer[cp] |= FILL_MASKS[run as usize] >> bx;
            } else {
                buffer[cp] &= !(FILL_MASKS[run as usize] >> bx);
            }
            x = x.wrapping_add(runs[slot]);
        }
        index += 2;
    }
}

/// `Fax3SetupState`'s checks: the row's bytes and pixels.
pub(super) fn setup(tiff: &Tiff) -> Result<()> {
    let dir = &tiff.dir;
    if dir.bits_per_sample != 1 {
        bail!("Bits/sample must be 1 for Group 3/4 encoding/decoding");
    }
    if dir.samples_per_pixel != 1 && dir.planar_config != PLANARCONFIG_SEPARATE {
        bail!("Samples/pixel shall be 1 for Group 3/4 encoding/decoding, or PlanarConfiguration must be set to Separate.");
    }
    let (row_bytes, row_pixels) = row_geometry(tiff)?;
    if (row_bytes as i64) < (row_pixels as i64 + 7) / 8 {
        bail!("Inconsistent number of bytes per row : rowbytes={row_bytes} rowpixels={row_pixels}");
    }
    Ok(())
}

fn row_geometry(tiff: &Tiff) -> Result<(u64, u32)> {
    if tiff.dir.tiled {
        Ok((tiff.tile_row_size()?, tiff.dir.tile_width))
    } else {
        Ok((tiff.scanline_size()?, tiff.dir.width))
    }
}

/// The codec's starting mode (`TIFFInitCCITTFax3` and friends).
pub(super) fn initial_mode(compression: u16) -> u32 {
    match compression {
        COMPRESSION_CCITTRLE => FAXMODE_NORTC | FAXMODE_NOEOL | FAXMODE_BYTEALIGN,
        COMPRESSION_CCITTRLEW => FAXMODE_NORTC | FAXMODE_NOEOL | FAXMODE_WORDALIGN,
        _ => FAXMODE_NORTC,
    }
}

/// Decodes a strip or tile into `output`; false when it failed without a
/// libtiff error (the data ran out). `mode` keeps the codec's mode
/// between strips (Group 3 may give up on EOLs for good).
pub(super) fn decode(tiff: &Tiff, raw: &[u8], output: &mut [u8], mode: &mut u32, raw_offset: u64) -> Result<bool> {
    let dir = &tiff.dir;
    let (row_bytes, row_pixels) = row_geometry(tiff)?;
    let row_bytes = row_bytes as usize;
    if !output.len().is_multiple_of(row_bytes) {
        bail!("Fractional scanlines cannot be read");
    }
    let compression = dir.compression;
    let two_d = compression == COMPRESSION_CCITTFAX3 && dir.group3_options & GROUP3OPT_2DENCODING != 0;
    let needs_reference = two_d || compression == COMPRESSION_CCITTFAX4;
    let mut runs_per_line = (row_pixels as usize + 1).div_ceil(32) * 32;
    if needs_reference {
        runs_per_line *= 2;
    }
    let mut runs = vec![0u32; runs_per_line * 2];
    // `Fax3PreDecode`: the reference line all white.
    let (mut current, mut referenced) = (0usize, runs_per_line);
    if needs_reference {
        runs[referenced] = row_pixels;
        runs[referenced + 1] = 0;
    }
    let reverse = dir.fill_order != FILLORDER_LSB2MSB;
    let mut bits = Bits { data: raw, position: 0, reverse, acc: 0, avail: 0 };
    let mut eol_count = 0i32;
    let last_x = row_pixels as i32;
    let rows = output.len() / row_bytes;
    let mut line = 0usize;
    let mut out = 0usize;

    if compression == COMPRESSION_CCITTFAX4 {
        while line < rows {
            let (row_slice, reference_slice) = split_runs(&mut runs, current, referenced, runs_per_line);
            let mut row = Row { runs: row_slice, start: 0, end: runs_per_line, pa: 0, a0: 0, run_length: 0, last_x };
            let mut reference = Reference { pb: 1, end: runs_per_line, b1: reference_slice[0] as i32 };
            let outcome = expand_2d(&mut bits, &mut row, &mut reference, reference_slice, &mut eol_count)?;
            let pa = row.pa;
            if matches!(outcome, Expand::Eof) || eol_count != 0 {
                // `EOFG4`: the end-of-block code, then the partial row.
                if bits.need16(13) {
                    bits.clear(13);
                }
                if ((last_x + 7) >> 3) as usize > output.len() - out {
                    bail!("Buffer overrun detected");
                }
                fill_runs(&mut output[out..out + row_bytes], row_slice_mut(&mut runs, current, runs_per_line), 0, pa, row_pixels);
                return Ok(line != 0);
            }
            if ((last_x + 7) >> 3) as usize > output.len() - out {
                bail!("Buffer overrun detected");
            }
            fill_runs(&mut output[out..out + row_bytes], row_slice_mut(&mut runs, current, runs_per_line), 0, pa, row_pixels);
            // The imaginary change for the reference.
            let slice = row_slice_mut(&mut runs, current, runs_per_line);
            let mut row = Row { runs: slice, start: 0, end: runs_per_line, pa, a0: 0, run_length: 0, last_x };
            row.set_value(0)?;
            std::mem::swap(&mut current, &mut referenced);
            out += row_bytes;
            line += 1;
        }
        return Ok(true);
    }

    let rle = matches!(compression, COMPRESSION_CCITTRLE | COMPRESSION_CCITTRLEW);
    while line < rows {
        if !rle && *mode & FAXMODE_NOEOL == 0 {
            // `SYNC_EOL`.
            let mut found = true;
            if eol_count == 0 {
                loop {
                    if !bits.need16(11) {
                        let (row_slice, _) = split_runs(&mut runs, current, referenced, runs_per_line);
                        let mut row = Row { runs: row_slice, start: 0, end: runs_per_line, pa: 0, a0: 0, run_length: 0, last_x };
                        row.cleanup()?;
                        let pa = row.pa;
                        fill_runs(&mut output[out..out + row_bytes], row_slice_mut(&mut runs, current, runs_per_line), 0, pa, row_pixels);
                        return Ok(false);
                    }
                    if bits.get(11) == 0 {
                        break;
                    }
                    bits.clear(1);
                }
            }
            loop {
                if !bits.need8(8) {
                    found = false;
                    break;
                }
                if bits.get(8) != 0 {
                    break;
                }
                bits.clear(8);
            }
            if !found {
                // No EOL after all: start the data over without them.
                *mode |= FAXMODE_NOEOL;
                bits = Bits { data: raw, position: 0, reverse, acc: 0, avail: 0 };
                eol_count = 0;
                continue;
            }
            while bits.get(1) == 0 {
                bits.clear(1);
            }
            bits.clear(1);
            eol_count = 0;
        }
        let one_d = if two_d {
            if !bits.need8(1) {
                let (row_slice, _) = split_runs(&mut runs, current, referenced, runs_per_line);
                let mut row = Row { runs: row_slice, start: 0, end: runs_per_line, pa: 0, a0: 0, run_length: 0, last_x };
                row.cleanup()?;
                let pa = row.pa;
                fill_runs(&mut output[out..out + row_bytes], row_slice_mut(&mut runs, current, runs_per_line), 0, pa, row_pixels);
                return Ok(false);
            }
            let tag = bits.get(1);
            bits.clear(1);
            tag != 0
        } else {
            true
        };
        let (row_slice, reference_slice) = split_runs(&mut runs, current, referenced, runs_per_line);
        let mut row = Row { runs: row_slice, start: 0, end: runs_per_line, pa: 0, a0: 0, run_length: 0, last_x };
        let outcome = if one_d {
            expand_1d(&mut bits, &mut row, &mut eol_count)?
        } else {
            let mut reference = Reference { pb: 1, end: runs_per_line, b1: reference_slice[0] as i32 };
            expand_2d(&mut bits, &mut row, &mut reference, reference_slice, &mut eol_count)?
        };
        let pa = row.pa;
        fill_runs(&mut output[out..out + row_bytes], row_slice_mut(&mut runs, current, runs_per_line), 0, pa, row_pixels);
        if matches!(outcome, Expand::Eof) {
            return Ok(false);
        }
        if two_d {
            if pa < runs_per_line {
                let slice = row_slice_mut(&mut runs, current, runs_per_line);
                let mut row = Row { runs: slice, start: 0, end: runs_per_line, pa, a0: 0, run_length: 0, last_x };
                row.set_value(0)?;
            }
            std::mem::swap(&mut current, &mut referenced);
        }
        if rle {
            if *mode & FAXMODE_BYTEALIGN != 0 {
                let n = bits.avail - (bits.avail & !7);
                bits.clear(n);
            } else if *mode & FAXMODE_WORDALIGN != 0 {
                let n = bits.avail - (bits.avail & !15);
                bits.clear(n);
                let address = if tiff.mapped { raw_offset as usize + bits.position } else { bits.position };
                if bits.avail == 0 && address % 2 == 1 {
                    bits.position += 1;
                }
            }
        }
        out += row_bytes;
        line += 1;
    }
    Ok(true)
}

/// The current line's runs and the reference line's, as two slices of
/// the shared array.
fn split_runs(runs: &mut [u32], current: usize, referenced: usize, per_line: usize) -> (&mut [u32], &[u32]) {
    if current < referenced {
        let (first, second) = runs.split_at_mut(referenced);
        (&mut first[current..current + per_line], &second[..per_line])
    } else {
        let (first, second) = runs.split_at_mut(current);
        let length = per_line.min(second.len());
        (&mut second[..length], &first[referenced..referenced + per_line])
    }
}

fn row_slice_mut(runs: &mut [u32], current: usize, per_line: usize) -> &mut [u32] {
    let end = (current + per_line + 1).min(runs.len());
    &mut runs[current..end]
}
