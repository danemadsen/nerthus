//! LibRaw's `open_datastream` for DNG files: `identify` with
//! `parse_tiff`/`parse_tiff_ifd` (the tags DNG files carry), `apply_tiff`
//! (the raw IFD) and `identify_process_dng_fields` (colour matrices, white
//! balance, black and white levels), then the fix-ups `open_datastream`
//! makes afterwards. Maker notes (EXIF and DNGPrivateData) are not parsed:
//! for DNG files the fields LibRaw takes from them are overridden by the
//! DNG tags, except a few camera-specific crops (Pentax, Leica, Olympus).

use anyhow::{bail, Result};

use super::stream::{Stream, BIG_ENDIAN, LITTLE_ENDIAN, TYPE_IFD8, TYPE_SIZES};

pub(super) const CBLACK_SIZE: usize = 4104;
const IFD_MAXCOUNT: usize = 10;
/// `INT_MAX`: an untiled image's tile size.
pub(super) const NOT_TILED: u32 = i32::MAX as u32;

// LIBRAW_DNGFM_*
const DNGFM_ILLUMINANT: u32 = 1 << 1;
const DNGFM_COLORMATRIX: u32 = 1 << 2;
const DNGFM_CALIBRATION: u32 = 1 << 3;
const DNGFM_ANALOGBALANCE: u32 = 1 << 4;
const DNGFM_BLACK: u32 = 1 << 5;
const DNGFM_WHITE: u32 = 1 << 6;
const DNGFM_OPCODE2: u32 = 1 << 7;
const DNGFM_LINTABLE: u32 = 1 << 8;
const DNGFM_ASSHOTNEUTRAL: u32 = 1 << 12;

// LIBRAW_WBI_*
const WBI_DAYLIGHT: u16 = 1;
const WBI_FLASH: u16 = 4;
const WBI_D55: u16 = 20;
const WBI_D65: u16 = 21;
const WBI_D75: u16 = 22;
const WBI_D50: u16 = 23;

/// `LibRaw_constants::xyz_rgb`.
pub(super) const XYZ_RGB: [[f64; 3]; 3] = [[0.4124564, 0.3575761, 0.1804375], [0.2126729, 0.7151522, 0.0721750], [0.0193339, 0.1191920, 0.9503041]];
/// `LibRaw_constants::d65_white`.
pub(super) const D65_WHITE: [f32; 3] = [0.95047, 1.0, 1.08883];

/// The `load_raw` decoders DNG files use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Decoder {
    /// `lossless_dng_load_raw`: lossless JPEG tiles (compression 7).
    Lossless,
    /// `packed_dng_load_raw`: uncompressed integers (compression 1).
    Packed,
    /// `deflate_dng_load_raw`: zip-compressed floats (compression 8).
    Deflate,
    /// `uncompressed_fp_dng_load_raw`: uncompressed floats.
    UncompressedFloat,
}

/// `tiff_ifd_t::dng_color[]`.
#[derive(Clone, Default)]
pub(super) struct DngColor {
    pub parsed: u32,
    pub illuminant: u16,
    pub colormatrix: [[f32; 3]; 4],
    pub calibration: [[f32; 4]; 4],
}

/// `tiff_ifd_t::dng_levels`.
#[derive(Clone)]
pub(super) struct DngLevels {
    pub parsed: u32,
    pub dng_cblack: Vec<u32>,
    pub dng_fcblack: Vec<f32>,
    pub dng_black: u32,
    pub dng_fblack: f32,
    pub dng_whitelevel: [u32; 4],
    pub analogbalance: [f32; 4],
    pub asshotneutral: [f32; 4],
}

impl Default for DngLevels {
    fn default() -> Self {
        DngLevels {
            parsed: 0,
            dng_cblack: vec![0; CBLACK_SIZE],
            dng_fcblack: vec![0.0; CBLACK_SIZE],
            dng_black: 0,
            dng_fblack: 0.0,
            dng_whitelevel: [0; 4],
            analogbalance: [1.0; 4],
            asshotneutral: [0.0; 4],
        }
    }
}

/// `tiff_ifd_t`: what one IFD says about its image.
#[derive(Clone)]
pub(super) struct Ifd {
    pub width: i32,
    pub height: i32,
    pub bps: i32,
    pub comp: i32,
    pub phint: i32,
    pub offset: i64,
    pub flip: i32,
    pub samples: i32,
    pub bytes: i64,
    pub tile_width: i32,
    pub tile_length: i32,
    pub sample_format: i32,
    pub predictor: i32,
    pub rows_per_strip: i32,
    pub strip_offsets: Vec<i64>,
    pub strip_byte_counts: Vec<i64>,
    pub vwidth: i32,
    pub vheight: i32,
    pub left_margin: i32,
    pub top_margin: i32,
    pub fuji_width: i32,
    pub filters: u32,
    pub newsubfiletype: i32,
    pub dng_color: [DngColor; 2],
    pub dng_levels: DngLevels,
    pub lineartable_offset: i64,
    pub lineartable_len: u32,
    pub opcode2_offset: i64,
}

impl Default for Ifd {
    fn default() -> Self {
        let mut dng_color: [DngColor; 2] = Default::default();
        dng_color[0].illuminant = 0xffff;
        dng_color[1].illuminant = 0xffff;
        Ifd {
            width: 0,
            height: 0,
            bps: 0,
            comp: 0,
            phint: 0,
            offset: 0,
            flip: 0,
            samples: 0,
            bytes: 0,
            tile_width: 0,
            tile_length: 0,
            sample_format: 0,
            predictor: 0,
            rows_per_strip: 0,
            strip_offsets: Vec::new(),
            strip_byte_counts: Vec::new(),
            vwidth: 0,
            vheight: 0,
            left_margin: 0,
            top_margin: 0,
            fuji_width: 0,
            filters: 0,
            newsubfiletype: 0,
            dng_color,
            dng_levels: DngLevels::default(),
            lineartable_offset: 0,
            lineartable_len: 0,
            opcode2_offset: 0,
        }
    }
}

/// LibRaw's state once a DNG file is opened: `imgdata.sizes`, `idata`,
/// `color` and the unpacker's fields.
pub(super) struct Raw {
    pub order: u16,
    pub make: String,
    pub model: String,
    pub dng_version: u32,
    pub raw_width: u32,
    pub raw_height: u32,
    pub width: u32,
    pub height: u32,
    pub top_margin: u32,
    pub left_margin: u32,
    pub pixel_aspect: f64,
    pub filters: u32,
    pub colors: usize,
    pub tiff_bps: u32,
    pub tiff_samples: u32,
    pub data_offset: i64,
    pub tile_width: u32,
    pub tile_length: u32,
    pub meta_offset: i64,
    pub decoder: Decoder,
    pub curve: Vec<u16>,
    pub black: u32,
    pub cblack: Vec<u32>,
    pub maximum: u32,
    pub pre_mul: [f32; 4],
    pub cam_mul: [f32; 4],
    pub rgb_cam: [[f32; 4]; 3],
    pub raw_color: bool,
    pub mask: [[i32; 4]; 8],
    /// `xtrans`: the 6×6 pattern of an X-Trans CFA (`filters` 9).
    pub xtrans: [[u8; 6]; 6],
    pub ifds: Vec<Ifd>,
    /// The raw IFD (`find_ifd_by_offset(data_offset)`).
    pub raw_ifd: usize,
}

/// `parse_tiff_ifd`'s state across IFDs: the globals tags write.
struct Parser<'a> {
    stream: Stream<'a>,
    ifds: Vec<Ifd>,
    make: String,
    model: String,
    dng_version: u32,
    filters: u32,
    colors: usize,
    cblack: Vec<u32>,
    black: u32,
    maximum: u32,
    pixel_aspect: f64,
    top_margin: u32,
    left_margin: u32,
    width: u32,
    height: u32,
    mask: [[i32; 4]; 8],
    meta_offset: i64,
    curve: Vec<u16>,
    cmatrix: [[f32; 4]; 3],
    pre_mul: [f32; 4],
    cam_mul: [f32; 4],
    tiff_bps: u32,
    /// The X-Trans pattern (CFAPattern of a 6×6 CFA).
    xtrans: [[u8; 6]; 6],
}

pub(super) fn identify(data: &[u8]) -> Result<Raw> {
    let mut parser = Parser {
        stream: Stream::new(data),
        ifds: Vec::new(),
        make: String::new(),
        model: String::new(),
        dng_version: 0,
        filters: u32::MAX,
        colors: 3,
        cblack: vec![0; CBLACK_SIZE],
        black: 0,
        maximum: 0,
        pixel_aspect: 1.0,
        top_margin: 0,
        left_margin: 0,
        width: 0,
        height: 0,
        mask: [[0; 4]; 8],
        meta_offset: 0,
        curve: (0..=u16::MAX).collect(),
        cmatrix: [[0.0; 4]; 3],
        pre_mul: [1.0, 1.0, 1.0, 0.0],
        cam_mul: [0.0, 1.0, 0.0, 0.0],
        tiff_bps: 0,
        xtrans: [[0; 6]; 6],
    };
    let order = parser.stream.get2();
    if order != LITTLE_ENDIAN && order != BIG_ENDIAN {
        bail!("not a DNG file");
    }
    parser.stream.order = order;
    if !parser.parse_tiff(0) {
        bail!("not a DNG file");
    }
    if parser.dng_version == 0 {
        bail!("not a DNG file (other raw formats are not supported)");
    }
    parser.identify_dng()
}

impl Parser<'_> {
    fn parse_tiff(&mut self, base: i64) -> bool {
        self.stream.seek(base as u64);
        let order = self.stream.get2();
        if order != LITTLE_ENDIAN && order != BIG_ENDIAN {
            return false;
        }
        self.stream.order = order;
        self.stream.get2();
        loop {
            let offset = self.stream.get4() as i32;
            if offset == 0 {
                break;
            }
            let position = offset as i64 + base;
            if position > self.stream.size() as i64 || position < 0 {
                break;
            }
            self.stream.seek(position as u64);
            if self.parse_tiff_ifd(base) {
                break;
            }
        }
        true
    }

    /// `tiff_get`: a tag's id, type and count, the stream at its value.
    fn tiff_get(&mut self, base: i64) -> (u32, u32, u32, u64) {
        let tag = self.stream.get2() as u32;
        let kind = self.stream.get2() as u32;
        let count = self.stream.get4();
        let save = self.stream.tell() + 4;
        let unit = TYPE_SIZES[if kind <= TYPE_IFD8 { kind as usize } else { 0 }];
        if count.wrapping_mul(unit) > 4 {
            let offset = self.stream.get4() as i64 + base;
            self.stream.seek(offset.max(0) as u64);
        }
        (tag, kind, count, save)
    }

    /// `parse_tiff_ifd`, for the tags DNG files use. Returns true to stop
    /// reading IFDs.
    fn parse_tiff_ifd(&mut self, base: i64) -> bool {
        if self.ifds.len() >= IFD_MAXCOUNT {
            return true;
        }
        let ifd = self.ifds.len();
        self.ifds.push(Ifd::default());
        let entries = self.stream.get2();
        if entries > 512 {
            return true;
        }
        let file_size = self.stream.size() as i64;

        let mut cc = [[0.0f64; 4]; 4];
        for (row, values) in cc.iter_mut().enumerate() {
            values[row] = 1.0;
        }
        let mut cm = [[0.0f64; 3]; 4];
        let mut ab = [1.0f64; 4];
        let mut asn = [0.0f64; 4];
        let mut xyz = [1.0f64; 3];
        let mut use_cm = false;
        let mut cfa_pattern = [0u8; 16];
        let mut pattern_length = 16usize;
        let mut plane_colors = [0u8, 1, 2, 3];

        for _ in 0..entries {
            let (tag, kind, count, save) = self.tiff_get(base);
            let value_position = self.stream.tell() as i64;
            if count > 8 && value_position + count as i64 > 2 * file_size {
                self.stream.seek(save);
                continue;
            }
            match tag {
                // NewSubfileType
                0x00fe => self.ifds[ifd].newsubfiletype = self.stream.getreal(kind) as i32,
                0x0100 => self.ifds[ifd].width = self.stream.getint(kind) as i32,
                0x0101 => self.ifds[ifd].height = self.stream.getint(kind) as i32,
                // BitsPerSample
                0x0102 => {
                    if self.ifds[ifd].samples == 0 {
                        self.ifds[ifd].samples = (count & 7) as i32;
                    }
                    self.ifds[ifd].bps = self.stream.getint(kind) as i32;
                    if (self.tiff_bps as i32) < self.ifds[ifd].bps {
                        self.tiff_bps = self.ifds[ifd].bps as u32;
                    }
                }
                0x0103 => self.ifds[ifd].comp = self.stream.getint(kind) as i32,
                0x0106 => self.ifds[ifd].phint = self.stream.get2() as i32,
                0x010f => self.make = self.stream.read_string(64),
                0x0110 => self.model = self.stream.read_string(64),
                // StripOffsets
                0x0111 => {
                    if count > 1 && count < 16384 {
                        let position = self.stream.tell();
                        let offsets: Vec<i64> = (0..count).map(|_| self.stream.get4() as i64 + base).collect();
                        self.ifds[ifd].strip_offsets = offsets;
                        self.stream.seek(position);
                    }
                    self.ifds[ifd].offset = self.stream.get4() as i64 + base;
                }
                // Orientation
                0x0112 => {
                    let flips = b"50132467";
                    self.ifds[ifd].flip = (flips[(self.stream.get2() & 7) as usize] - b'0') as i32;
                }
                0x0115 => self.ifds[ifd].samples = (self.stream.getint(kind) & 7) as i32,
                0x0116 => self.ifds[ifd].rows_per_strip = self.stream.getint(kind) as i32,
                // StripByteCounts
                0x0117 => {
                    if count > 1 && count < 16384 {
                        let position = self.stream.tell();
                        let counts: Vec<i64> = (0..count).map(|_| self.stream.get4() as i64).collect();
                        self.ifds[ifd].strip_byte_counts = counts;
                        self.stream.seek(position);
                    }
                    self.ifds[ifd].bytes = self.stream.get4() as i64;
                }
                0x013d => self.ifds[ifd].predictor = self.stream.getint(kind) as i32,
                0x0142 => self.ifds[ifd].tile_width = self.stream.getint(kind) as i32,
                0x0143 => self.ifds[ifd].tile_length = self.stream.getint(kind) as i32,
                // TileOffsets: the table's position, or the only offset.
                0x0144 => {
                    self.ifds[ifd].offset = if count > 1 { self.stream.tell() as i64 } else { self.stream.get4() as i64 };
                    if count == 1 {
                        self.ifds[ifd].tile_width = 0;
                        self.ifds[ifd].tile_length = 0;
                    }
                }
                0x0145 => self.ifds[ifd].bytes = if count > 1 { self.stream.tell() as i64 } else { self.stream.get4() as i64 },
                // SubIFDs
                0x014a => {
                    for _ in 0..count.min(1000) {
                        let position = self.stream.tell();
                        let offset = self.stream.get4() as i64 + base;
                        self.stream.seek(offset.max(0) as u64);
                        if self.parse_tiff_ifd(base) {
                            break;
                        }
                        self.stream.seek(position + 4);
                    }
                }
                0x0153 => self.ifds[ifd].sample_format = self.stream.getint(kind) as i32,
                // CFARepeatPatternDim: 6×6 is X-Trans.
                0x828d => {
                    if self.stream.get2() == 6 && self.stream.get2() == 6 {
                        self.ifds[ifd].filters = 9;
                        self.filters = 9;
                    }
                }
                // CFAPattern
                0x828e => {
                    if self.filters == 9 || count == 36 {
                        if self.filters != 9 {
                            self.ifds[ifd].filters = 9;
                            self.filters = 9;
                            self.colors = 3;
                        }
                        for row in self.xtrans.iter_mut() {
                            for colour in row.iter_mut() {
                                *colour = self.stream.getc().map_or(3, |byte| byte & 3);
                            }
                        }
                    } else if count > 0 {
                        pattern_length = (count as usize).min(16);
                        self.stream.read(&mut cfa_pattern[..pattern_length]);
                        let mut cfa = 0u32;
                        self.colors = 0;
                        for &color in cfa_pattern.iter().take(pattern_length) {
                            if self.colors >= 4 {
                                break;
                            }
                            if color > 31 {
                                continue;
                            }
                            if cfa & (1 << color) == 0 {
                                self.colors += 1;
                            }
                            cfa |= 1 << color;
                        }
                        if cfa == 0o70 {
                            plane_colors[..3].copy_from_slice(&[3, 4, 5]);
                        }
                        if cfa == 0o72 {
                            plane_colors = [5, 3, 4, 1];
                        }
                        self.guess_cfa(ifd, &cfa_pattern, pattern_length, &plane_colors);
                    }
                }
                // ExifIFD, GPS: nothing the image depends on.
                0x8769 | 0x8825 => {}
                // DNGVersion
                0xc612 => {
                    for _ in 0..4 {
                        let byte = self.stream.getc().map_or(u32::MAX, |byte| byte as u32);
                        self.dng_version = (self.dng_version << 8).wrapping_add(byte);
                    }
                    if self.make.is_empty() {
                        self.make = "DNG".to_string();
                    }
                }
                // CFAPlaneColor
                0xc616 => {
                    if self.filters != 9 {
                        let length = (count as usize).min(4);
                        self.colors = length;
                        self.stream.read(&mut plane_colors[..length]);
                        self.guess_cfa(ifd, &cfa_pattern, pattern_length, &plane_colors);
                    }
                }
                // CFALayout: 2 is Fuji's rotated layout.
                0xc617 => {
                    if self.stream.get2() == 2 {
                        self.ifds[ifd].fuji_width = 1;
                    }
                }
                // LinearizationTable
                0xc618 => {
                    self.ifds[ifd].dng_levels.parsed |= DNGFM_LINTABLE;
                    self.ifds[ifd].lineartable_offset = self.stream.tell() as i64;
                    self.ifds[ifd].lineartable_len = count;
                    self.linear_table(count);
                }
                // BlackLevelRepeatDim
                0xc619 => {
                    let levels = &mut self.ifds[ifd].dng_levels;
                    levels.parsed |= DNGFM_BLACK;
                    self.cblack[4] = self.stream.get2() as u32;
                    self.cblack[5] = self.stream.get2() as u32;
                    levels.dng_cblack[4] = self.cblack[4];
                    levels.dng_fcblack[4] = self.cblack[4] as f32;
                    levels.dng_cblack[5] = self.cblack[5];
                    levels.dng_fcblack[5] = self.cblack[5] as f32;
                    if self.cblack[4] * self.cblack[5] > (CBLACK_SIZE - 7) as u32 {
                        levels.dng_fcblack[4] = 1.0;
                        levels.dng_fcblack[5] = 1.0;
                        levels.dng_cblack[4] = 1;
                        levels.dng_cblack[5] = 1;
                        self.cblack[4] = 1;
                        self.cblack[5] = 1;
                    }
                }
                // BlackLevel
                0xc61a => self.black_level(ifd, kind, count),
                // BlackLevelDeltaH, BlackLevelDeltaV
                0xc61b | 0xc61c => {
                    let mut sum = 0.0f64;
                    for _ in 0..count.min(65536) {
                        sum += self.stream.getreal(kind);
                    }
                    if count > 0 {
                        let mean = sum / count as f64;
                        self.black = self.black.wrapping_add((mean + 0.5) as u32);
                        let levels = &mut self.ifds[ifd].dng_levels;
                        levels.dng_fblack += mean as f32;
                        levels.dng_black = levels.dng_black.wrapping_add((mean + 0.5) as u32);
                        levels.parsed |= DNGFM_BLACK;
                    }
                }
                // WhiteLevel
                0xc61d => {
                    self.ifds[ifd].dng_levels.parsed |= DNGFM_WHITE;
                    self.maximum = self.stream.getint(kind);
                    self.ifds[ifd].dng_levels.dng_whitelevel[0] = self.maximum;
                    if self.ifds[ifd].samples > 1 {
                        for index in 1..(count as usize).min(4) {
                            self.ifds[ifd].dng_levels.dng_whitelevel[index] = self.stream.getint(kind);
                        }
                    }
                }
                // DefaultScale
                0xc61e => {
                    let horizontal = self.stream.getrealf(kind);
                    let vertical = self.stream.getrealf(kind);
                    if horizontal > 0.00001 && vertical > 0.00001 {
                        let aspect = horizontal / vertical;
                        self.pixel_aspect = if aspect > 0.995 && aspect < 1.005 { 1.0 } else { aspect as f64 };
                    }
                }
                // CalibrationIlluminant1, CalibrationIlluminant2
                0xc65a | 0xc65b => {
                    let index = (tag - 0xc65a) as usize;
                    self.ifds[ifd].dng_color[index].illuminant = self.stream.get2();
                    self.ifds[ifd].dng_color[index].parsed |= DNGFM_ILLUMINANT;
                }
                // ColorMatrix1, ColorMatrix2
                0xc621 | 0xc622 => {
                    let channels = match count {
                        9 => 3,
                        12 => 4,
                        _ => 0,
                    };
                    let index = (tag - 0xc621) as usize;
                    if channels > 0 {
                        self.ifds[ifd].dng_color[index].parsed |= DNGFM_COLORMATRIX;
                    }
                    let stored = &mut self.ifds[ifd].dng_color[index].colormatrix;
                    for (row, stored_row) in cm.iter_mut().zip(stored.iter_mut()).take(channels) {
                        for (value, stored_value) in row.iter_mut().zip(stored_row.iter_mut()) {
                            *value = self.stream.getreal(kind);
                            *stored_value = *value as f32;
                        }
                    }
                    use_cm = true;
                }
                // CameraCalibration1, CameraCalibration2
                0xc623 | 0xc624 => {
                    let channels = match count {
                        9 => 3,
                        16 => 4,
                        _ => 0,
                    };
                    let index = (tag - 0xc623) as usize;
                    if channels > 0 {
                        self.ifds[ifd].dng_color[index].parsed |= DNGFM_CALIBRATION;
                    }
                    let stored = &mut self.ifds[ifd].dng_color[index].calibration;
                    for (row, stored_row) in cc.iter_mut().zip(stored.iter_mut()).take(channels) {
                        for (value, stored_value) in row.iter_mut().zip(stored_row.iter_mut()).take(channels) {
                            *value = self.stream.getreal(kind);
                            *stored_value = *value as f32;
                        }
                    }
                }
                // AnalogBalance
                0xc627 => {
                    if count >= 3 {
                        self.ifds[ifd].dng_levels.parsed |= DNGFM_ANALOGBALANCE;
                    }
                    let stored = &mut self.ifds[ifd].dng_levels.analogbalance;
                    for (value, stored_value) in ab.iter_mut().zip(stored.iter_mut()).take(count as usize) {
                        *value = self.stream.getreal(kind);
                        *stored_value = *value as f32;
                    }
                }
                // AsShotNeutral
                0xc628 => {
                    if count >= 3 {
                        self.ifds[ifd].dng_levels.parsed |= DNGFM_ASSHOTNEUTRAL;
                    }
                    let stored = &mut self.ifds[ifd].dng_levels.asshotneutral;
                    for (value, stored_value) in asn.iter_mut().zip(stored.iter_mut()).take(count as usize) {
                        *value = self.stream.getreal(kind);
                        *stored_value = *value as f32;
                    }
                }
                // AsShotWhiteXY
                0xc629 => {
                    xyz[0] = self.stream.getreal(kind);
                    xyz[1] = self.stream.getreal(kind);
                    xyz[2] = 1.0 - xyz[0] - xyz[1];
                    for (value, white) in xyz.iter_mut().zip(D65_WHITE) {
                        *value /= white as f64;
                    }
                }
                // ActiveArea
                0xc68d => {
                    let top = self.stream.getint(kind);
                    let left = self.stream.getint(kind);
                    let bottom = self.stream.getint(kind);
                    let right = self.stream.getint(kind);
                    self.top_margin = top & 0xffff;
                    self.left_margin = left & 0xffff;
                    self.height = bottom.wrapping_sub(self.top_margin) & 0xffff;
                    self.width = right.wrapping_sub(self.left_margin) & 0xffff;
                    let current = &mut self.ifds[ifd];
                    current.top_margin = self.top_margin as i32;
                    current.left_margin = self.left_margin as i32;
                    current.vheight = self.height as i32;
                    current.vwidth = self.width as i32;
                }
                // MaskedAreas
                0xc68e => {
                    for index in 0..(count as usize).min(32) {
                        self.mask[index / 4][index % 4] = self.stream.getint(kind) as i32;
                    }
                    self.black = 0;
                }
                // OpcodeList2
                0xc741 => {
                    self.ifds[ifd].dng_levels.parsed |= DNGFM_OPCODE2;
                    self.meta_offset = self.stream.tell() as i64;
                    self.ifds[ifd].opcode2_offset = self.meta_offset;
                }
                _ => {}
            }
            self.stream.seek(save);
        }

        // The colour data of this IFD.
        let colors = self.colors;
        for (row, balance) in cc.iter_mut().zip(ab).take(colors.min(4)) {
            for value in row.iter_mut().take(colors.min(4)) {
                *value *= balance;
            }
        }
        if use_cm {
            let mut cam_xyz = [[0.0f64; 3]; 4];
            for (cam_row, cc_row) in cam_xyz.iter_mut().zip(cc.iter()).take(colors.min(4)) {
                for (column, value) in cam_row.iter_mut().enumerate() {
                    for (calibration, cm_row) in cc_row.iter().zip(cm.iter()).take(colors.min(4)) {
                        *value += calibration * cm_row[column] * xyz[column];
                    }
                }
            }
            self.cmatrix = cam_xyz_coeff(&cam_xyz, colors, &mut self.pre_mul);
        }
        if asn[0] != 0.0 {
            self.cam_mul[3] = 0.0;
            for (multiplier, neutral) in self.cam_mul.iter_mut().zip(asn).take(colors.min(4)) {
                if neutral.abs() > 0.0001 {
                    *multiplier = (1.0 / neutral) as f32;
                }
            }
        }
        if !use_cm {
            for (color, (multiplier, cc_row)) in self.pre_mul.iter_mut().zip(cc.iter()).enumerate().take(colors.min(4)) {
                if cc_row[color].abs() > 0.0001 {
                    *multiplier /= cc_row[color] as f32;
                }
            }
        }
        false
    }

    /// The `guess_cfa_pc` tail of CFAPattern and CFAPlaneColor.
    fn guess_cfa(&mut self, ifd: usize, pattern: &[u8; 16], length: usize, plane_colors: &[u8; 4]) {
        let mut table = [0u8; 256];
        for (color, &plane) in plane_colors.iter().enumerate().take(self.colors.min(4)) {
            table[plane as usize] = color as u8;
        }
        let mut filters = self.filters;
        for index in (0..16).rev() {
            filters = (filters << 2) | table[pattern[index % length] as usize] as u32;
        }
        if filters == 0 {
            filters = filters.wrapping_sub(1);
        }
        self.filters = filters;
        self.ifds[ifd].filters = filters;
    }

    /// BlackLevel (50714).
    fn black_level(&mut self, ifd: usize, kind: u32, count: u32) {
        let samples = self.ifds[ifd].samples;
        if samples > 1 && samples == count as i32 {
            // Linear DNG, a black level per channel.
            let levels = &mut self.ifds[ifd].dng_levels;
            levels.parsed |= DNGFM_BLACK;
            for index in 0..(count as usize).min(4) {
                levels.dng_fcblack[index] = self.stream.getrealf(kind);
                let value = (levels.dng_fcblack[index] + 0.5) as u32;
                levels.dng_cblack[index] = value;
                self.cblack[index] = value;
            }
            levels.dng_cblack[CBLACK_SIZE - 1] = count;
            levels.dng_fblack = 0.0;
            levels.dng_black = 0;
            self.black = 0;
        } else if samples > 1 && samples as u32 * self.cblack[4] * self.cblack[5] == count {
            // Linear DNG with a repeat pattern.
            let levels = &mut self.ifds[ifd].dng_levels;
            levels.parsed |= DNGFM_BLACK;
            levels.dng_cblack[CBLACK_SIZE - 1] = count;
            self.cblack[CBLACK_SIZE - 1] = count;
            for index in 0..(count as usize).min(CBLACK_SIZE - 7) {
                levels.dng_fcblack[index + 6] = self.stream.getrealf(kind);
                let value = (levels.dng_fcblack[index + 6] + 0.5) as u32;
                levels.dng_cblack[index + 6] = value;
                self.cblack[index + 6] = value;
            }
        } else if self.cblack[4] * self.cblack[5] < 2 && count == 1 {
            let levels = &mut self.ifds[ifd].dng_levels;
            levels.parsed |= DNGFM_BLACK;
            levels.dng_fblack = self.stream.getrealf(kind);
            levels.dng_black = levels.dng_fblack as u32;
            self.black = levels.dng_black;
        } else if self.cblack[4] * self.cblack[5] <= count {
            let pattern = (self.cblack[4] * self.cblack[5]) as usize;
            for index in 0..pattern {
                let value = self.stream.getrealf(kind);
                self.ifds[ifd].dng_levels.dng_fcblack[6 + index] = value;
                self.cblack[6 + index] = value as u32;
            }
            self.black = 0;
            self.cblack[..4].fill(0);
            let levels = &mut self.ifds[ifd].dng_levels;
            levels.parsed |= DNGFM_BLACK;
            levels.dng_cblack[6..6 + pattern].copy_from_slice(&self.cblack[6..6 + pattern]);
            levels.dng_fblack = 0.0;
            levels.dng_black = 0;
            levels.dng_fcblack[..4].fill(0.0);
            levels.dng_cblack[..4].fill(0);
        }
    }

    /// `linear_table`: the LinearizationTable into `curve`.
    fn linear_table(&mut self, length: u32) {
        let length = length.min(0x10000) as usize;
        if length < 1 {
            return;
        }
        self.stream.read_shorts(&mut self.curve[..length]);
        for index in length..0x10000 {
            self.curve[index] = self.curve[index - 1];
        }
        self.maximum = self.curve[if length < 0x1000 { 0xfff } else { length - 1 }] as u32;
    }

    fn find_ifd_by_offset(&self, offset: i64) -> Option<usize> {
        self.ifds.iter().position(|ifd| ifd.offset == offset)
    }

    /// `IFDLEVELINDEX`: the raw IFD if it has the level field, else IFD 0
    /// if that has it.
    fn level_index(&self, raw: usize, bit: u32) -> Option<usize> {
        if self.ifds[raw].dng_levels.parsed & bit != 0 {
            Some(raw)
        } else if self.ifds[0].dng_levels.parsed & bit != 0 {
            Some(0)
        } else {
            None
        }
    }

    /// `IFDCOLORINDEX`.
    fn color_index(&self, raw: usize, subset: usize, bit: u32) -> Option<usize> {
        if self.ifds[raw].dng_color[subset].parsed & bit != 0 {
            Some(raw)
        } else if self.ifds[0].dng_color[subset].parsed & bit != 0 {
            Some(0)
        } else {
            None
        }
    }

    /// `apply_tiff` for a DNG file, then the DNG branch of `identify`,
    /// `identify_process_dng_fields` and `open_datastream`'s fix-ups.
    fn identify_dng(mut self) -> Result<Raw> {
        // apply_tiff: the main (NewSubFileType 0) CFA or linear raw IFD.
        let mut frames = Vec::new();
        for (index, ifd) in self.ifds.iter().enumerate() {
            if ifd.width < 1 || ifd.width > 65535 || ifd.height < 1 || ifd.height > 65535 {
                continue;
            }
            let subfile = ifd.newsubfiletype;
            let raw_photometric = ifd.phint == 32803 || ifd.phint == 34892;
            if (subfile == 16 || subfile == 0 || (subfile & 0xffff) == 1) && !raw_photometric {
                continue;
            }
            if subfile == 0 {
                frames.push(index);
            }
        }
        let Some(&raw) = frames.first() else {
            bail!("the DNG file has no raw image");
        };
        let selected = self.ifds[raw].clone();
        if selected.samples == 2 {
            bail!("Fuji SuperCCD DNG files are not supported");
        }
        let tiff_compress = selected.comp as u32;
        let tiff_sampleformat = selected.sample_format as u32;
        let mut state = Raw {
            order: self.stream.order,
            make: self.make.clone(),
            model: self.model.clone(),
            dng_version: self.dng_version,
            raw_width: selected.width as u32,
            raw_height: selected.height as u32,
            width: selected.vwidth as u32,
            height: selected.vheight as u32,
            top_margin: selected.top_margin as u32,
            left_margin: selected.left_margin as u32,
            pixel_aspect: self.pixel_aspect,
            filters: 0,
            colors: self.colors,
            tiff_bps: selected.bps as u32,
            tiff_samples: selected.samples as u32,
            data_offset: selected.offset,
            tile_width: selected.tile_width as u32,
            tile_length: selected.tile_length as u32,
            meta_offset: self.meta_offset,
            decoder: Decoder::Packed,
            curve: std::mem::take(&mut self.curve),
            black: self.black,
            cblack: std::mem::take(&mut self.cblack),
            maximum: self.maximum,
            pre_mul: self.pre_mul,
            cam_mul: self.cam_mul,
            rgb_cam: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
            raw_color: true,
            mask: self.mask,
            xtrans: self.xtrans,
            ifds: Vec::new(),
            raw_ifd: raw,
        };
        state.filters = if selected.phint == 34892 {
            0
        } else if raw > 0 && selected.phint == 32803 && self.ifds[0].phint == 32803 && selected.filters == 0 && self.ifds[0].filters != 0 {
            self.ifds[0].filters
        } else {
            selected.filters
        };
        if selected.dng_levels.dng_whitelevel[0] != 0 {
            state.maximum = selected.dng_levels.dng_whitelevel[0];
        } else if selected.sample_format <= 2 && state.tiff_bps > 0 && state.tiff_bps < 32 {
            state.maximum = (1 << state.tiff_bps) - 1;
        } else if selected.sample_format == 3 {
            state.maximum = 1;
        }
        if state.tile_width == 0 {
            state.tile_width = NOT_TILED;
        }
        if state.tile_length == 0 {
            state.tile_length = NOT_TILED;
        }
        // apply_tiff's decoder choice, which stands only for uncompressed
        // floats; compressions it does not know make the file unsupported.
        let bps = state.tiff_bps;
        let float_samples = tiff_sampleformat == 3 && bps > 8 && bps.is_multiple_of(8) && bps <= 32;
        let uncompressed_fp = matches!(tiff_compress, 0 | 1) && float_samples;
        let known_compression = matches!(tiff_compress, 0 | 1 | 6 | 7 | 8 | 9 | 99 | 262 | 32767 | 32769 | 32770 | 32773 | 32867 | 34713 | 34892 | 52546 | 65000 | 65535);
        if !known_compression {
            bail!("unsupported DNG compression {tiff_compress}");
        }
        if selected.fuji_width != 0 {
            bail!("Fuji rotated DNG files are not supported");
        }
        // identify: sizes, then the DNG branch.
        self.make = normalized_make(&state.make).to_string();
        self.model = normalized_model(&self.make, &state.model);
        if state.height == 0 {
            state.height = state.raw_height;
        }
        if state.width == 0 {
            state.width = state.raw_width;
        }
        self.finetune_pentax(&mut state, &selected);
        if state.filters == u32::MAX {
            state.filters = 0;
        }
        if state.filters == 0 {
            state.colors = state.tiff_samples as usize;
        }
        state.decoder = match tiff_compress {
            0 | 1 if uncompressed_fp => Decoder::UncompressedFloat,
            0 | 1 if tiff_sampleformat != 3 => Decoder::Packed,
            7 => Decoder::Lossless,
            8 if float_samples => Decoder::Deflate,
            8 => bail!("only floating-point deflated DNG files are supported"),
            9 => bail!("VC-5 compressed DNG files are not supported"),
            52546 => bail!("JPEG XL compressed DNG files are not supported"),
            // lossy_dng_load_raw: Magick.Native's LibRaw is built with
            // NO_JPEG, which makes identify reject these.
            34892 => bail!("lossy DNG files are not supported"),
            _ => bail!("unsupported DNG compression {tiff_compress}"),
        };
        if self.make == "Leica" {
            if self.model == "SL2" {
                state.height -= 3;
            }
            if self.model.len() >= 7 && self.model[..7].eq_ignore_ascii_case("Q2 MONO") {
                state.height -= 18;
            }
        }

        let mut cmatrix = self.cmatrix;
        let levels = self.process_dng_fields(&mut state, &mut cmatrix);

        if state.height < 22 || state.width < 22 || (state.tiff_bps > 16 && !matches!(state.decoder, Decoder::Deflate | Decoder::UncompressedFloat)) {
            bail!("the DNG raw image is damaged or unsupported");
        }
        if matches!(state.decoder, Decoder::Deflate | Decoder::UncompressedFloat) && (state.tiff_bps < 16 || state.tiff_bps > 32 || !state.tiff_bps.is_multiple_of(8)) {
            bail!("unsupported floating-point DNG sample size");
        }
        if state.tiff_samples > 4 || state.colors > 4 || state.colors < 1 {
            bail!("unsupported DNG samples per pixel");
        }

        // The camera white balance must be in a sane range.
        let colors = state.colors.min(4);
        let mut cam_mul_ok = state.cam_mul.iter().take(colors).all(|&value| value > 0.001);
        if cam_mul_ok {
            let mut minimum = state.cam_mul[0] as f64;
            for &value in state.cam_mul.iter().take(colors) {
                minimum = minimum.min(value as f64);
            }
            let mut low = state.cam_mul[0] as f64 / minimum;
            let mut high = low;
            for &value in state.cam_mul.iter().take(colors) {
                let normalised = value as f64 / minimum;
                low = low.min(normalised);
                high = high.max(normalised);
            }
            if low <= 0.01f32 as f64 || high > 100.0 {
                cam_mul_ok = false;
            }
        }
        if !cam_mul_ok {
            if state.cam_mul[0] > 0.0 {
                state.cam_mul[0] = 0.0;
            }
            state.cam_mul[3] = 0.0;
        }
        // use_camera_matrix: the DNG matrix becomes rgb_cam. (Without
        // one LibRaw falls back to its own table of camera matrices,
        // which is not ported: such DNG files stay in camera colours.)
        if cmatrix[0][0] > 0.125 {
            state.rgb_cam = cmatrix;
            state.raw_color = false;
        }

        if state.raw_height < state.height {
            state.raw_height = state.height;
        }
        if state.raw_width < state.width {
            state.raw_width = state.width;
        }
        if state.tiff_bps == 0 {
            state.tiff_bps = 12;
        }
        if state.maximum == 0 {
            // `(1 << tiff_bps) - 1`: x86 masks the shift, so 32-bit
            // floating-point data gets 0 here.
            state.maximum = 1i32.wrapping_shl(state.tiff_bps).wrapping_sub(1) as u32;
        }
        if state.maximum > 0xffff {
            state.maximum = 0xffff;
        }
        if state.tiff_samples > 6 || state.colors == 2 {
            bail!("unsupported DNG samples per pixel");
        }
        if state.raw_width < 22 || state.raw_width > 64000 || state.raw_height < 22 || state.raw_height > 64000 || state.pixel_aspect < 0.1 || state.pixel_aspect > 10.0 {
            bail!("the DNG raw image is damaged or unsupported");
        }
        if state.raw_width <= state.left_margin || state.raw_height <= state.top_margin {
            bail!("the DNG raw image is damaged or unsupported");
        }
        if state.tiff_samples < 1 || state.tiff_samples > 4 {
            bail!("unsupported DNG samples per pixel");
        }
        if state.filters > 999 && state.colors == 3 {
            let filters = state.filters;
            state.filters |= ((filters >> 2 & 0x22222222) | (filters << 2 & 0x88888888)) & filters << 1;
        }
        // fcol's fixed 16×16 Leaf pattern (filters 1) is not ported.
        if state.filters == 1 {
            bail!("DNG files with this colour filter layout are not supported");
        }

        // open_datastream: X-Trans margins of Fuji files start on a multiple
        // of 6, the pattern then taken from xtrans_abs (which only maker
        // notes set: without it the pattern is empty and demosaicing
        // fails); Bayer margins start on an even row and column.
        if self.make == "Fujifilm" && state.filters == 9 {
            let new_top = state.top_margin.next_multiple_of(6);
            let new_left = state.left_margin.next_multiple_of(6);
            if new_top != state.top_margin || new_left != state.left_margin {
                state.height -= new_top - state.top_margin;
                state.top_margin = new_top;
                state.width -= new_left - state.left_margin;
                state.left_margin = new_left;
                state.xtrans = [[0; 6]; 6];
            }
        } else if state.filters >= 1000 && (!state.top_margin.is_multiple_of(2) || !state.left_margin.is_multiple_of(2)) {
            let mut crop = [0u32; 2];
            if !state.top_margin.is_multiple_of(2) {
                state.top_margin += 1;
                state.height -= 1;
                crop[1] = 1;
            }
            if !state.left_margin.is_multiple_of(2) {
                state.left_margin += 1;
                state.width -= 1;
                crop[0] = 1;
            }
            let mut filters = 0u32;
            for index in 0..16 {
                filters |= fc(state.filters, (index >> 1) + crop[1], (index & 1) + crop[0]) << (index * 2);
            }
            state.filters = filters;
        }

        // Linear DNG white balance from the channels' white levels.
        if state.filters == 0 && state.colors > 1 && state.colors < 5 {
            // White minus black in unsigned arithmetic, as the C has it.
            let mut delta = [0.0f32; 4];
            for (index, value) in delta.iter_mut().enumerate().take(state.colors) {
                let black = levels.dng_black.wrapping_add(levels.dng_cblack[index]);
                *value = levels.dng_whitelevel[index].wrapping_sub(black) as f32;
            }
            let mut minimum = delta[0];
            let mut maximum = delta[0];
            for &value in delta.iter().take(state.colors).skip(1) {
                if minimum > value {
                    minimum = value;
                }
                if maximum < value {
                    maximum = value;
                }
            }
            if minimum > 1.0 && maximum < minimum * 20.0 {
                for (index, value) in delta.iter().enumerate().take(state.colors) {
                    state.cam_mul[index] /= value / maximum;
                    state.pre_mul[index] /= value / maximum;
                }
                state.maximum = (state.cblack[0] as f32 + maximum) as u32;
            }
        }

        // Camera-specific crops of DNG files.
        if self.make == "Panasonic" && self.model.eq_ignore_ascii_case("DMC-LX100") {
            state.width = 4288;
        }
        if self.make == "Leica" {
            if self.model.eq_ignore_ascii_case("SL2") {
                state.height -= 16;
            } else if self.model.eq_ignore_ascii_case("SL3") {
                match state.raw_width {
                    9536 => state.width -= 12,
                    7424 => state.width -= 14,
                    5312 => state.width -= 18,
                    _ => {}
                }
            }
        }
        if self.make == "Sony" {
            match state.raw_width {
                3984 => state.width = 3925,
                4288 => state.width = state.raw_width - 32,
                4928 if state.height < 3280 => state.width = state.raw_width - 8,
                5504 => state.width = state.raw_width - if state.height > 3664 { 8 } else { 32 },
                _ => {}
            }
        }

        state.ifds = std::mem::take(&mut self.ifds);
        Ok(state)
    }

    /// `identify_finetune_pentax`, for the sizes it fixes without the
    /// camera ids from Pentax maker notes.
    fn finetune_pentax(&self, state: &mut Raw, selected: &Ifd) {
        if selected.phint == 34892 {
            return;
        }
        if self.make == "Pentax" || self.make == "Samsung" {
            if state.height == 2624 && state.width == 3936 {
                state.height = 2616;
                state.width = 3896;
            }
            if state.height == 3136 && state.width == 4864 {
                state.height = 3124;
                state.width = 4688;
                state.filters = 0x16161616;
            }
        }
    }

    /// `identify_process_dng_fields`: the colour matrix for a daylight
    /// illuminant, white and black levels, white balance, OpcodeList2 and
    /// the linearisation table of the raw IFD (or IFD 0).
    fn process_dng_fields(&mut self, state: &mut Raw, cmatrix: &mut [[f32; 4]; 3]) -> DngLevels {
        let mut levels = DngLevels::default();
        let colors = state.colors;
        if let Some(raw) = self.find_ifd_by_offset(state.data_offset) {
            let illuminants = [self.color_index(raw, 0, DNGFM_ILLUMINANT), self.color_index(raw, 1, DNGFM_ILLUMINANT)];
            let matrices = [self.color_index(raw, 0, DNGFM_COLORMATRIX), self.color_index(raw, 1, DNGFM_COLORMATRIX)];
            let calibrations = [self.color_index(raw, 0, DNGFM_CALIBRATION), self.color_index(raw, 1, DNGFM_CALIBRATION)];
            let balance = self.level_index(raw, DNGFM_ANALOGBALANCE);
            let same_ifd = illuminants[0].is_some() && illuminants[0] == illuminants[1] && illuminants[0] == matrices[0] && illuminants[0] == matrices[1];
            if let (true, Some(source)) = (same_ifd, illuminants[0]) {
                let colour = &self.ifds[source].dng_color;
                if colour[0].illuminant > 0 && colour[1].illuminant > 0 {
                    let mut selected = colour.iter().position(|subset| subset.illuminant == WBI_D65);
                    if selected.is_none() {
                        selected = colour.iter().position(|subset| matches!(subset.illuminant, WBI_DAYLIGHT | WBI_D55 | WBI_D75 | WBI_D50 | WBI_FLASH));
                    }
                    if let Some(index) = selected {
                        let mut cc = [[0.0f64; 4]; 4];
                        for (row, values) in cc.iter_mut().enumerate() {
                            values[row] = 1.0;
                        }
                        let mut cm = [[0.0f64; 3]; 4];
                        for (row, stored_row) in cm.iter_mut().zip(colour[index].colormatrix.iter()).take(colors.min(4)) {
                            for (value, &stored) in row.iter_mut().zip(stored_row) {
                                *value = stored as f64;
                            }
                        }
                        if calibrations[index] == Some(source) {
                            for (row, stored_row) in cc.iter_mut().zip(colour[index].calibration.iter()).take(colors.min(4)) {
                                for (value, &stored) in row.iter_mut().zip(stored_row).take(colors.min(4)) {
                                    *value = stored as f64;
                                }
                            }
                        }
                        if balance == Some(source) {
                            let analog_balance = self.ifds[source].dng_levels.analogbalance;
                            for (row, gain) in cc.iter_mut().zip(analog_balance).take(colors.min(4)) {
                                for value in row.iter_mut().take(colors.min(4)) {
                                    *value *= gain as f64;
                                }
                            }
                        }
                        let mut cam_xyz = [[0.0f64; 3]; 4];
                        for (cam_row, cc_row) in cam_xyz.iter_mut().zip(cc.iter()).take(colors.min(4)) {
                            for (column, value) in cam_row.iter_mut().enumerate() {
                                for (calibration, cm_row) in cc_row.iter().zip(cm.iter()).take(colors.min(4)) {
                                    *value += calibration * cm_row[column];
                                }
                            }
                        }
                        *cmatrix = cam_xyz_coeff(&cam_xyz, colors, &mut state.pre_mul);
                    }
                }
            }

            if let Some(source) = self.level_index(raw, DNGFM_WHITE).filter(|&source| self.ifds[source].dng_levels.dng_whitelevel[0] != 0) {
                levels.dng_whitelevel = self.ifds[source].dng_levels.dng_whitelevel;
            } else {
                let ifd = &self.ifds[raw];
                if ifd.sample_format <= 2 && ifd.bps > 0 && ifd.bps < 32 {
                    levels.dng_whitelevel = [((1u32 << ifd.bps) - 1); 4];
                }
            }
            if let Some(source) = self.level_index(raw, DNGFM_ASSHOTNEUTRAL) {
                levels.asshotneutral = self.ifds[source].dng_levels.asshotneutral;
                if levels.asshotneutral[0] != 0.0 {
                    state.cam_mul[3] = 0.0;
                    for color in 0..colors.min(4) {
                        if levels.asshotneutral[color].abs() > 0.0001 {
                            state.cam_mul[color] = 1.0 / levels.asshotneutral[color];
                        }
                    }
                }
            }
            if let Some(source) = self.level_index(raw, DNGFM_BLACK) {
                let black_levels = &self.ifds[source].dng_levels;
                levels.dng_fblack = black_levels.dng_fblack;
                levels.dng_black = black_levels.dng_black;
                levels.dng_cblack.copy_from_slice(&black_levels.dng_cblack);
                levels.dng_fcblack.copy_from_slice(&black_levels.dng_fcblack);
            }
            if let Some(source) = self.level_index(raw, DNGFM_OPCODE2) {
                state.meta_offset = self.ifds[source].opcode2_offset;
            }
            if let Some(source) = self.level_index(raw, DNGFM_LINTABLE) {
                let offset = self.ifds[source].lineartable_offset;
                let length = self.ifds[source].lineartable_len;
                if offset >= 0 && length > 0 {
                    let position = self.stream.tell();
                    self.stream.seek(offset as u64);
                    self.curve = std::mem::take(&mut state.curve);
                    self.maximum = state.maximum;
                    self.linear_table(length);
                    state.curve = std::mem::take(&mut self.curve);
                    state.maximum = self.maximum;
                    self.stream.seek(position);
                }
            }
        }
        // The DNG black and white levels become LibRaw's.
        state.maximum = levels.dng_whitelevel[0];
        state.black = levels.dng_black;
        let samples = state.tiff_samples as usize;
        let pattern = (levels.dng_cblack[4] * levels.dng_cblack[5]) as usize;
        if samples > 2 && samples <= 4 && pattern * samples == levels.dng_cblack[CBLACK_SIZE - 1] as usize {
            // Per-channel blacks in a repeat pattern: their averages.
            let mut sums = [0i32; 4];
            let mut counts = [0i32; 4];
            let mut index = 6;
            for _ in 0..pattern {
                for channel in 0..samples.min(4) {
                    sums[channel] = sums[channel].wrapping_add(levels.dng_cblack[index] as i32);
                    counts[channel] += 1;
                    index += 1;
                }
            }
            for channel in 0..4 {
                if counts[channel] != 0 {
                    let average = sums[channel] / counts[channel];
                    levels.dng_cblack[channel] = levels.dng_cblack[channel].wrapping_add(average as u32);
                }
            }
            levels.dng_cblack[4] = 0;
            levels.dng_cblack[5] = 0;
        }
        state.cblack.copy_from_slice(&levels.dng_cblack);
        levels
    }
}

/// `simplify_make_model`'s maker for the makes DNG fix-ups test.
fn normalized_make(make: &str) -> &str {
    let lower = make.to_ascii_lowercase();
    for (needle, name) in [("leica", "Leica"), ("panasonic", "Panasonic"), ("sony", "Sony"), ("pentax", "Pentax"), ("ricoh", "Ricoh"), ("samsung", "Samsung"), ("fujifilm", "Fujifilm")] {
        if lower.contains(needle) {
            return name;
        }
    }
    make.trim_end()
}

/// The model without the maker's name in front of it.
fn normalized_model(make: &str, model: &str) -> String {
    let model = model.trim_end();
    let length = make.len();
    if model.len() > length && model[..length].eq_ignore_ascii_case(make) && model.as_bytes()[length] == b' ' {
        return model[length + 1..].to_string();
    }
    model.to_string()
}

/// `FC`: the colour of a Bayer pattern position.
pub(super) fn fc(filters: u32, row: u32, column: u32) -> u32 {
    filters >> ((((row << 1) & 14) | (column & 1)) << 1) & 3
}

/// `fcol`: the colour of an image position, X-Trans patterns included
/// (`row + 6` and `column + 6` keep a step left or up in the pattern).
pub(super) fn fcol(state: &Raw, row: i32, column: i32) -> usize {
    if state.filters == 9 {
        return state.xtrans[((row + 6) % 6) as usize][((column + 6) % 6) as usize] as usize;
    }
    fc(state.filters, row as u32, column as u32) as usize
}

/// `cam_xyz_coeff`: rgb_cam (or cmatrix) from a camera-from-XYZ matrix,
/// and `pre_mul` from its rows' sums.
pub(super) fn cam_xyz_coeff(cam_xyz: &[[f64; 3]; 4], colors: usize, pre_mul: &mut [f32; 4]) -> [[f32; 4]; 3] {
    let mut cam_rgb = [[0.0f64; 3]; 4];
    for (rgb_row, xyz_row) in cam_rgb.iter_mut().zip(cam_xyz).take(colors.min(4)) {
        for (column, value) in rgb_row.iter_mut().enumerate() {
            for (index, xyz_value) in xyz_row.iter().enumerate() {
                *value += xyz_value * XYZ_RGB[index][column];
            }
        }
    }
    // Normalise each row so cam_rgb × (1, 1, 1) is (1, 1, 1, 1).
    for (rgb_row, multiplier) in cam_rgb.iter_mut().zip(pre_mul.iter_mut()).take(colors.min(4)) {
        let mut sum = 0.0f64;
        for value in rgb_row.iter() {
            sum += value;
        }
        if sum > 0.00001 {
            for value in rgb_row.iter_mut() {
                *value /= sum;
            }
            *multiplier = (1.0 / sum) as f32;
        } else {
            *rgb_row = [0.0; 3];
            *multiplier = 1.0;
        }
    }
    let inverse = pseudoinverse(&cam_rgb, colors);
    let mut rgb_cam = [[0.0f32; 4]; 3];
    for (row, values) in rgb_cam.iter_mut().enumerate() {
        for (value, inverse_row) in values.iter_mut().zip(inverse.iter()).take(colors.min(4)) {
            *value = inverse_row[row] as f32;
        }
    }
    rgb_cam
}

/// `pseudoinverse`: (inᵀ·in)⁻¹·inᵀ by Gauss–Jordan elimination.
pub(super) fn pseudoinverse(input: &[[f64; 3]; 4], size: usize) -> [[f64; 3]; 4] {
    let rows = &input[..size.min(4)];
    let mut work = [[0.0f64; 6]; 3];
    for (row, values) in work.iter_mut().enumerate() {
        values[row + 3] = 1.0;
        for (column, value) in values.iter_mut().take(3).enumerate() {
            for input_row in rows {
                *value += input_row[row] * input_row[column];
            }
        }
    }
    for row in 0..3 {
        let pivot = work[row][row];
        if pivot.abs() > 0.00001f32 as f64 {
            for value in work[row].iter_mut() {
                *value /= pivot;
            }
        }
        let pivot_row = work[row];
        for (other, other_row) in work.iter_mut().enumerate() {
            if other == row {
                continue;
            }
            let factor = other_row[row];
            for (value, pivot_value) in other_row.iter_mut().zip(pivot_row) {
                *value -= pivot_value * factor;
            }
        }
    }
    let mut output = [[0.0f64; 3]; 4];
    for (output_row, input_row) in output.iter_mut().zip(rows) {
        for (column, value) in output_row.iter_mut().enumerate() {
            for (index, input_value) in input_row.iter().enumerate() {
                *value += work[column][index + 3] * input_value;
            }
        }
    }
    output
}
