//! JPEG as ImageMagick reads it through libjpeg-turbo 3.1: a port of the
//! library's decoder — markers, sequential, progressive, arithmetic and
//! lossless entropy decoding, block smoothing of incomplete progressive
//! images, the accurate integer IDCT, fancy upsampling and the colour
//! conversions — with the reader's own handling on top (`coders/jpeg.c`):
//! grey images as a palette of grey, CMYK inverted, warnings ignored and
//! errors fatal until the image has been read.

mod arith;
mod huffman;
mod output;
mod source;
mod tables;

use std::path::Path;

use anyhow::{Context, Result};

use crate::pixel_cache::{CacheColorspace, DEFAULT_GAMMA, PixelCache, PixelChannel};
use source::*;

/// A fatal decoding error (libjpeg's `ERREXIT`).
#[derive(Clone, Debug)]
pub struct JpegError(String);

impl JpegError {
    fn new(message: &str) -> Self {
        Self(message.to_string())
    }
}

impl std::fmt::Display for JpegError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for JpegError {}

/// A DHT table as stored (`JHUFF_TBL`).
#[derive(Clone)]
pub(super) struct HuffTable {
    pub bits: [u8; 17],
    pub huffval: [u8; 256],
}

impl HuffTable {
    fn standard(bits: &[u8; 17], values: &[u8]) -> Self {
        let mut huffval = [0u8; 256];
        huffval[..values.len()].copy_from_slice(values);
        Self { bits: *bits, huffval }
    }
}

/// Colour spaces of the data and of the output (`J_COLOR_SPACE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSpace {
    Unknown,
    Grayscale,
    Rgb,
    YCbCr,
    Cmyk,
    Ycck,
}

/// A frame component.
#[derive(Clone)]
pub(super) struct Component {
    pub id: i32,
    pub h_samp: usize,
    pub v_samp: usize,
    pub quant_tbl_no: usize,
    pub dc_tbl_no: usize,
    pub ac_tbl_no: usize,
    pub width_in_blocks: usize,
    pub height_in_blocks: usize,
    pub downsampled_width: usize,
    pub downsampled_height: usize,
    /// The quantization table latched by the first scan holding it.
    pub quant_table: Option<[u16; 64]>,
    pub mcu_width: usize,
    pub mcu_height: usize,
    pub last_col_width: usize,
    pub last_row_height: usize,
}

/// What a scan's MCU decoders need (`per_scan_setup` and SOS).
#[derive(Clone, Default)]
pub(super) struct ScanInfo {
    pub comps_in_scan: usize,
    /// The frame component of each scan component.
    pub component_index: Vec<usize>,
    pub dc_tbl_no: Vec<usize>,
    pub ac_tbl_no: Vec<usize>,
    pub blocks_in_mcu: usize,
    /// The scan component of each block of an MCU.
    pub mcu_membership: Vec<usize>,
    pub mcus_per_row: usize,
    pub ss: i32,
    pub se: i32,
    pub ah: i32,
    pub al: i32,
    pub restart_interval: u32,
}

/// The entropy decoder of the current scan.
enum Entropy {
    Sequential(huffman::Sequential),
    Progressive(Box<huffman::Progressive>),
    Arithmetic(arith::Arithmetic),
    Lossless(huffman::Lossless),
}

impl Entropy {
    fn insufficient_data(&self) -> bool {
        match self {
            Entropy::Sequential(decoder) => decoder.insufficient_data,
            Entropy::Progressive(decoder) => decoder.insufficient_data,
            Entropy::Lossless(decoder) => decoder.insufficient_data,
            Entropy::Arithmetic(_) => false,
        }
    }
}

/// The decoded image as the reader takes it: `output_components`
/// interleaved samples per pixel.
pub struct Decoded {
    pub width: usize,
    pub height: usize,
    pub precision: i32,
    pub out_color_space: ColorSpace,
    pub components: usize,
    pub samples: Vec<u16>,
    /// The `8bim` profile's data begins with `G3FAX`: the reader labels the
    /// image Lab.
    pub fax: bool,
}

/// The decompressor (`jpeg_decompress_struct` and its modules).
struct Decoder<'a> {
    source: Source<'a>,
    markers: MarkerState,
    quant_tables: [Option<[u16; 64]>; 4],
    dc_tables: [Option<HuffTable>; 4],
    ac_tables: [Option<HuffTable>; 4],
    arith_dc_l: [u8; 16],
    arith_dc_u: [u8; 16],
    arith_ac_k: [u8; 16],
    restart_interval: u32,
    saw_jfif: bool,
    saw_adobe: bool,
    adobe_transform: u8,
    progressive: bool,
    lossless: bool,
    arith: bool,
    precision: i32,
    width: usize,
    height: usize,
    components: Vec<Component>,
    max_h: usize,
    max_v: usize,
    total_imcu_rows: usize,
    has_multiple_scans: bool,
    jpeg_color_space: ColorSpace,
    out_color_space: ColorSpace,
    scan: ScanInfo,
    input_scan_number: i32,
    /// Progression status: per component, then the previous scan's.
    coef_bits: Vec<[i32; 64]>,
    last_good_imcu_row: usize,
    /// Coefficients of every block, per component, padded to whole MCUs.
    coefficients: Vec<Vec<[i16; 64]>>,
    /// Blocks per padded row, per component.
    block_stride: Vec<usize>,
    /// Lossless: the samples of each component (rows of `width_in_blocks`).
    planes: Vec<Vec<u16>>,
    /// The 8BIM (Photoshop) profile gathered from APP13 markers.
    profile_8bim: Vec<u8>,
    profile_8bim_set: bool,
    entropy: Option<Entropy>,
    /// Driven by libtiff: libjpeg's own marker handling, errors to the end.
    libtiff: bool,
}

/// Restart and input state of the lossless difference controller.
struct LosslessState {
    /// Previous row of undifferenced samples per component (the last row
    /// of the row group before).
    undiff: Vec<Vec<Vec<i32>>>,
    /// Whether each component's next row starts the prediction over.
    first_row: Vec<bool>,
    restart_rows_to_go: u32,
}

fn round_up(value: usize, multiple: usize) -> usize {
    value.div_ceil(multiple) * multiple
}

impl<'a> Decoder<'a> {
    fn new(file: &'a [u8]) -> Self {
        Self {
            source: Source::new(file),
            markers: MarkerState::new(),
            quant_tables: [None, None, None, None],
            dc_tables: [None, None, None, None],
            ac_tables: [None, None, None, None],
            arith_dc_l: [0; 16],
            arith_dc_u: [1; 16],
            arith_ac_k: [5; 16],
            restart_interval: 0,
            saw_jfif: false,
            saw_adobe: false,
            adobe_transform: 0,
            progressive: false,
            lossless: false,
            arith: false,
            precision: 0,
            width: 0,
            height: 0,
            components: Vec::new(),
            max_h: 1,
            max_v: 1,
            total_imcu_rows: 0,
            has_multiple_scans: false,
            jpeg_color_space: ColorSpace::Unknown,
            out_color_space: ColorSpace::Unknown,
            scan: ScanInfo::default(),
            input_scan_number: 0,
            coef_bits: Vec::new(),
            last_good_imcu_row: 0,
            coefficients: Vec::new(),
            block_stride: Vec::new(),
            planes: Vec::new(),
            profile_8bim: Vec::new(),
            profile_8bim_set: false,
            entropy: None,
            libtiff: false,
        }
    }

    /// A decompressor as libtiff drives it: its source, and the tables
    /// earlier datastreams left.
    fn new_libtiff(data: &'a [u8], tables: &Tables) -> Self {
        let mut decoder = Self::new(data);
        decoder.source = Source::new_libtiff(data);
        decoder.libtiff = true;
        decoder.quant_tables = tables.quant;
        decoder.dc_tables = tables.dc.clone();
        decoder.ac_tables = tables.ac.clone();
        decoder
    }

    /// `read_markers`: processes markers up to the next SOS or EOI; returns
    /// whether it was an SOS.
    fn read_markers(&mut self) -> Result<bool, JpegError> {
        loop {
            if self.markers.unread_marker == 0 {
                if !self.markers.saw_soi {
                    first_marker(&mut self.source, &mut self.markers)?;
                } else {
                    next_marker(&mut self.source, &mut self.markers)?;
                }
            }
            let marker = self.markers.unread_marker;
            match marker {
                M_SOI => self.get_soi()?,
                M_SOF0 | M_SOF1 => self.get_sof(false, false, false)?,
                M_SOF2 => self.get_sof(true, false, false)?,
                M_SOF3 => self.get_sof(false, true, false)?,
                M_SOF9 => self.get_sof(false, false, true)?,
                M_SOF10 => self.get_sof(true, false, true)?,
                M_SOF11 => self.get_sof(false, true, true)?,
                M_SOF5 | M_SOF6 | M_SOF7 | M_JPG | M_SOF13 | M_SOF14 | M_SOF15 => {
                    return Err(JpegError::new(&format!("Unsupported JPEG process: SOF type 0x{marker:02x}")));
                }
                M_SOS => {
                    self.get_sos()?;
                    self.markers.unread_marker = 0;
                    return Ok(true);
                }
                M_EOI => {
                    self.markers.unread_marker = 0;
                    return Ok(false);
                }
                M_DAC => self.get_dac()?,
                M_DHT => self.get_dht()?,
                M_DQT => self.get_dqt()?,
                M_DRI => self.get_dri()?,
                M_APP0 | M_APP14 => self.get_interesting_appn(marker)?,
                0xE1..=M_APP15 | M_COM if self.libtiff => self.skip_variable()?,
                0xE2 => self.read_icc_profile()?,
                M_APP13 => self.read_iptc_profile()?,
                0xE1..=M_APP15 => self.read_app_profile()?,
                M_COM => self.read_comment()?,
                M_RST0..=M_RST7 | M_TEM => {}
                M_DNL => self.skip_variable()?,
                _ => return Err(JpegError::new(&format!("Unsupported marker type 0x{marker:02x}"))),
            }
            self.markers.unread_marker = 0;
        }
    }

    fn get_soi(&mut self) -> Result<(), JpegError> {
        if self.markers.saw_soi {
            return Err(JpegError::new("Invalid JPEG file structure: two SOI markers"));
        }
        self.arith_dc_l = [0; 16];
        self.arith_dc_u = [1; 16];
        self.arith_ac_k = [5; 16];
        self.restart_interval = 0;
        self.jpeg_color_space = ColorSpace::Unknown;
        self.saw_jfif = false;
        self.saw_adobe = false;
        self.adobe_transform = 0;
        self.markers.saw_soi = true;
        Ok(())
    }

    fn get_sof(&mut self, progressive: bool, lossless: bool, arith: bool) -> Result<(), JpegError> {
        if self.markers.saw_sof {
            return Err(JpegError::new("Invalid JPEG file structure: two SOF markers"));
        }
        self.progressive = progressive;
        self.lossless = lossless;
        self.arith = arith;
        let mut length = self.source.read_u16()? as i64;
        self.precision = self.source.read_byte()? as i32;
        self.height = self.source.read_u16()? as usize;
        self.width = self.source.read_u16()? as usize;
        let count = self.source.read_byte()? as usize;
        length -= 8;
        if self.height == 0 || self.width == 0 || count == 0 {
            return Err(JpegError::new("Empty JPEG image (DNL not supported)"));
        }
        if length != (count * 3) as i64 {
            return Err(JpegError::new("Bogus marker length"));
        }
        self.components.clear();
        for _ in 0..count {
            let id = self.source.read_byte()? as i32;
            let sampling = self.source.read_byte()?;
            let quant_tbl_no = self.source.read_byte()? as usize;
            self.components.push(Component {
                id,
                h_samp: ((sampling >> 4) & 15) as usize,
                v_samp: (sampling & 15) as usize,
                quant_tbl_no,
                dc_tbl_no: 0,
                ac_tbl_no: 0,
                width_in_blocks: 0,
                height_in_blocks: 0,
                downsampled_width: 0,
                downsampled_height: 0,
                quant_table: None,
                mcu_width: 0,
                mcu_height: 0,
                last_col_width: 0,
                last_row_height: 0,
            });
        }
        self.markers.saw_sof = true;
        Ok(())
    }

    fn get_sos(&mut self) -> Result<(), JpegError> {
        if !self.markers.saw_sof {
            return Err(JpegError::new("Invalid JPEG file structure: SOS before SOF"));
        }
        let length = self.source.read_u16()? as i64;
        let count = self.source.read_byte()? as usize;
        if length != (count * 2 + 6) as i64 || !(1..=4).contains(&count) {
            return Err(JpegError::new("Bogus marker length"));
        }
        let mut scan = ScanInfo { comps_in_scan: count, restart_interval: self.restart_interval, ..ScanInfo::default() };
        let mut taken = [false; 4];
        for _ in 0..count {
            let id = self.source.read_byte()? as i32;
            let tables = self.source.read_byte()?;
            let limit = self.components.len().min(4);
            let found = (0..limit).find(|&index| self.components[index].id == id && !taken[index]);
            let Some(index) = found else {
                return Err(JpegError::new(&format!("Invalid component ID {id} in SOS")));
            };
            taken[index] = true;
            self.components[index].dc_tbl_no = ((tables >> 4) & 15) as usize;
            self.components[index].ac_tbl_no = (tables & 15) as usize;
            if scan.component_index.contains(&index) {
                return Err(JpegError::new(&format!("Invalid component ID {id} in SOS")));
            }
            scan.component_index.push(index);
        }
        scan.ss = self.source.read_byte()? as i32;
        scan.se = self.source.read_byte()? as i32;
        let approximation = self.source.read_byte()?;
        scan.ah = ((approximation >> 4) & 15) as i32;
        scan.al = (approximation & 15) as i32;
        for &index in &scan.component_index {
            scan.dc_tbl_no.push(self.components[index].dc_tbl_no);
            scan.ac_tbl_no.push(self.components[index].ac_tbl_no);
        }
        self.scan = scan;
        self.markers.next_restart_num = 0;
        self.input_scan_number += 1;
        Ok(())
    }

    fn get_dac(&mut self) -> Result<(), JpegError> {
        let mut length = self.source.read_u16()? as i64 - 2;
        while length > 0 {
            let index = self.source.read_byte()? as usize;
            let value = self.source.read_byte()?;
            length -= 2;
            if index >= 32 {
                return Err(JpegError::new(&format!("Bogus DAC index {index}")));
            }
            if index >= 16 {
                self.arith_ac_k[index - 16] = value;
            } else {
                self.arith_dc_l[index] = value & 0x0F;
                self.arith_dc_u[index] = value >> 4;
                if self.arith_dc_l[index] > self.arith_dc_u[index] {
                    return Err(JpegError::new(&format!("Bogus DAC value 0x{value:x}")));
                }
            }
        }
        if length != 0 {
            return Err(JpegError::new("Bogus marker length"));
        }
        Ok(())
    }

    fn get_dht(&mut self) -> Result<(), JpegError> {
        let mut length = self.source.read_u16()? as i64 - 2;
        while length > 16 {
            let mut index = self.source.read_byte()? as i32;
            let mut bits = [0u8; 17];
            let mut count = 0usize;
            for entry in bits.iter_mut().skip(1) {
                *entry = self.source.read_byte()?;
                count += *entry as usize;
            }
            length -= 17;
            if count > 256 || count as i64 > length {
                return Err(JpegError::new("Bogus Huffman table definition"));
            }
            let mut huffval = [0u8; 256];
            for value in huffval.iter_mut().take(count) {
                *value = self.source.read_byte()?;
            }
            length -= count as i64;
            let is_ac = index & 0x10 != 0;
            if is_ac {
                index -= 0x10;
            }
            if !(0..4).contains(&index) {
                return Err(JpegError::new(&format!("Bogus DHT index {index}")));
            }
            let table = Some(HuffTable { bits, huffval });
            if is_ac {
                self.ac_tables[index as usize] = table;
            } else {
                self.dc_tables[index as usize] = table;
            }
        }
        if length != 0 {
            return Err(JpegError::new("Bogus marker length"));
        }
        Ok(())
    }

    fn get_dqt(&mut self) -> Result<(), JpegError> {
        let mut length = self.source.read_u16()? as i64 - 2;
        while length > 0 {
            let byte = self.source.read_byte()?;
            let precision = byte >> 4;
            let number = (byte & 0x0F) as usize;
            if number >= 4 {
                return Err(JpegError::new(&format!("Bogus DQT index {number}")));
            }
            let mut table = self.quant_tables[number].unwrap_or([0; 64]);
            for i in 0..64 {
                let value = if precision != 0 { self.source.read_u16()? } else { self.source.read_byte()? as u32 };
                table[tables::NATURAL_ORDER[i]] = value as u16;
            }
            self.quant_tables[number] = Some(table);
            length -= 65;
            if precision != 0 {
                length -= 64;
            }
        }
        if length != 0 {
            return Err(JpegError::new("Bogus marker length"));
        }
        Ok(())
    }

    fn get_dri(&mut self) -> Result<(), JpegError> {
        let length = self.source.read_u16()?;
        if length != 4 {
            return Err(JpegError::new("Bogus marker length"));
        }
        self.restart_interval = self.source.read_u16()?;
        Ok(())
    }

    /// `get_interesting_appn`: APP0 (JFIF) and APP14 (Adobe) as the library
    /// itself reads them.
    fn get_interesting_appn(&mut self, marker: u8) -> Result<(), JpegError> {
        let mut length = self.source.read_u16()? as i64 - 2;
        let count = length.clamp(0, 14) as usize;
        let mut data = [0u8; 14];
        for value in data.iter_mut().take(count) {
            *value = self.source.read_byte()?;
        }
        length -= count as i64;
        if marker == M_APP0 {
            if count >= 14 && &data[..5] == b"JFIF\0" {
                self.saw_jfif = true;
                if data[5] != 1 {
                    self.source.warn();
                }
            }
        } else if count >= 12 && &data[..5] == b"Adobe" {
            self.saw_adobe = true;
            self.adobe_transform = data[11];
        }
        self.source.skip(length)
    }

    fn skip_variable(&mut self) -> Result<(), JpegError> {
        let length = self.source.read_u16()? as i64 - 2;
        self.source.skip(length)
    }

    /// The reader's `GetProfileLength`: zero when the data ends.
    fn profile_length(&mut self) -> usize {
        let Some(high) = self.source.get_character() else {
            return 0;
        };
        let Some(low) = self.source.get_character() else {
            return 0;
        };
        256 * high as usize + low as usize
    }

    /// The reader's `ReadProfilePayload`: `length` bytes, or an error when
    /// the data ends first.
    fn profile_payload(&mut self, length: usize) -> Result<Vec<u8>, JpegError> {
        let mut payload = Vec::with_capacity(length.min(1 << 20));
        for _ in 0..length {
            let Some(byte) = self.source.get_character() else {
                return Err(JpegError::new("InsufficientImageDataInFile"));
            };
            payload.push(byte);
        }
        Ok(payload)
    }

    /// Skips up to `length` bytes, stopping where the data ends.
    fn skip_characters(&mut self, length: usize) {
        for _ in 0..length {
            if self.source.get_character().is_none() {
                break;
            }
        }
    }

    /// `GetCharacter` as the profile signatures read it: the end of the data
    /// reads as -1.
    fn signature_character(&mut self) -> u8 {
        self.source.get_character().unwrap_or(0xFF)
    }

    /// `ReadComment`.
    fn read_comment(&mut self) -> Result<(), JpegError> {
        let length = self.profile_length();
        if length <= 2 {
            return Ok(());
        }
        self.profile_payload(length - 2)?;
        Ok(())
    }

    /// `ReadAPPProfiles` (APP1, APP3 – APP12, APP15).
    fn read_app_profile(&mut self) -> Result<(), JpegError> {
        let length = self.profile_length();
        if length <= 2 {
            return Ok(());
        }
        self.profile_payload(length - 2)?;
        Ok(())
    }

    /// `ReadICCProfile` (APP2).
    fn read_icc_profile(&mut self) -> Result<(), JpegError> {
        let mut length = self.profile_length();
        if length <= 2 {
            return Ok(());
        }
        length -= 2;
        if length <= 14 {
            self.skip_characters(length);
            return Ok(());
        }
        let mut magick = [0u8; 12];
        for value in magick.iter_mut() {
            *value = self.signature_character();
        }
        if !c_string_equals_ignore_case(&magick, b"ICC_PROFILE") {
            if c_string_equals_ignore_case(&magick, b"MPF") {
                self.profile_payload(length - 12)?;
                return Ok(());
            }
            self.skip_characters(length - 12);
            return Ok(());
        }
        self.source.get_character();
        self.source.get_character();
        self.profile_payload(length - 14)?;
        Ok(())
    }

    /// `ReadIPTCProfile` (APP13): a Photoshop resource block joins the 8BIM
    /// profile. Short markers leave their tail for the marker scan to skip.
    fn read_iptc_profile(&mut self) -> Result<(), JpegError> {
        let mut length = self.profile_length();
        if length <= 2 {
            return Ok(());
        }
        length -= 2;
        if length <= 14 {
            self.skip_characters(length);
            return Ok(());
        }
        let mut magick = [0u8; 11];
        for value in magick.iter_mut().take(10) {
            *value = self.signature_character();
        }
        length -= 10;
        if length <= 10 {
            return Ok(());
        }
        if !c_string_equals_ignore_case(&magick, b"Photoshop ") {
            self.skip_characters(length);
            return Ok(());
        }
        if length <= 15 {
            return Ok(());
        }
        for _ in 0..4 {
            if self.source.get_character().is_none() {
                break;
            }
        }
        let payload = self.profile_payload(length - 4)?;
        self.profile_8bim.extend_from_slice(&payload);
        self.profile_8bim_set = true;
        Ok(())
    }

    /// `initial_setup`: checks the frame and sizes the components.
    fn initial_setup(&mut self) -> Result<(), JpegError> {
        if self.height > 65500 || self.width > 65500 {
            return Err(JpegError::new("Maximum supported image dimension is 65500 pixels"));
        }
        if self.lossless {
            if !(2..=16).contains(&self.precision) {
                return Err(JpegError::new(&format!("Unsupported JPEG data precision {}", self.precision)));
            }
        } else if self.precision != 8 && self.precision != 12 {
            return Err(JpegError::new(&format!("Unsupported JPEG data precision {}", self.precision)));
        }
        if self.components.len() > 10 {
            return Err(JpegError::new("Too many color components"));
        }
        self.max_h = 1;
        self.max_v = 1;
        for component in &self.components {
            if component.h_samp == 0 || component.h_samp > 4 || component.v_samp == 0 || component.v_samp > 4 {
                return Err(JpegError::new("Bogus sampling factors"));
            }
            self.max_h = self.max_h.max(component.h_samp);
            self.max_v = self.max_v.max(component.v_samp);
        }
        let unit = if self.lossless { 1 } else { 8 };
        for component in &mut self.components {
            component.width_in_blocks = (self.width * component.h_samp).div_ceil(self.max_h * unit);
            component.height_in_blocks = (self.height * component.v_samp).div_ceil(self.max_v * unit);
            component.downsampled_width = (self.width * component.h_samp).div_ceil(self.max_h);
            component.downsampled_height = (self.height * component.v_samp).div_ceil(self.max_v);
            component.quant_table = None;
        }
        self.total_imcu_rows = self.height.div_ceil(self.max_v * unit);
        self.has_multiple_scans = self.scan.comps_in_scan < self.components.len() || self.progressive;
        Ok(())
    }

    /// `default_decompress_parms`: the colour space guessed from the
    /// markers, and the output space.
    fn default_color_spaces(&mut self) {
        match self.components.len() {
            1 => {
                self.jpeg_color_space = ColorSpace::Grayscale;
                self.out_color_space = ColorSpace::Grayscale;
            }
            3 => {
                self.jpeg_color_space = if self.saw_jfif {
                    ColorSpace::YCbCr
                } else if self.saw_adobe {
                    match self.adobe_transform {
                        0 => ColorSpace::Rgb,
                        1 => ColorSpace::YCbCr,
                        _ => {
                            self.source.warn();
                            ColorSpace::YCbCr
                        }
                    }
                } else {
                    let ids = (self.components[0].id, self.components[1].id, self.components[2].id);
                    if ids == (1, 2, 3) {
                        if self.lossless {
                            ColorSpace::Rgb
                        } else {
                            ColorSpace::YCbCr
                        }
                    } else if ids == (82, 71, 66) || self.lossless {
                        ColorSpace::Rgb
                    } else {
                        ColorSpace::YCbCr
                    }
                };
                self.out_color_space = ColorSpace::Rgb;
            }
            4 => {
                self.jpeg_color_space = if self.saw_adobe {
                    match self.adobe_transform {
                        0 => ColorSpace::Cmyk,
                        2 => ColorSpace::Ycck,
                        _ => {
                            self.source.warn();
                            ColorSpace::Ycck
                        }
                    }
                } else {
                    ColorSpace::Cmyk
                };
                self.out_color_space = ColorSpace::Cmyk;
            }
            _ => {
                self.jpeg_color_space = ColorSpace::Unknown;
                self.out_color_space = ColorSpace::Unknown;
            }
        }
    }

    /// `jinit_color_deconverter`'s checks; the output components.
    fn check_color_conversion(&self) -> Result<usize, JpegError> {
        let count = self.components.len();
        let consistent = match self.jpeg_color_space {
            ColorSpace::Grayscale => count == 1,
            ColorSpace::Rgb | ColorSpace::YCbCr => count == 3,
            ColorSpace::Cmyk | ColorSpace::Ycck => count == 4,
            ColorSpace::Unknown => count >= 1,
        };
        if !consistent {
            return Err(JpegError::new("Bogus input colorspace"));
        }
        let unsupported = || Err(JpegError::new("Unsupported color conversion request"));
        match self.out_color_space {
            ColorSpace::Grayscale => {
                if self.lossless && self.jpeg_color_space != ColorSpace::Grayscale {
                    return unsupported();
                }
                if !matches!(self.jpeg_color_space, ColorSpace::Grayscale | ColorSpace::YCbCr | ColorSpace::Rgb) {
                    return unsupported();
                }
                Ok(1)
            }
            ColorSpace::Rgb => {
                if self.lossless && self.jpeg_color_space != ColorSpace::Rgb {
                    return unsupported();
                }
                if !matches!(self.jpeg_color_space, ColorSpace::YCbCr | ColorSpace::Grayscale | ColorSpace::Rgb) {
                    return unsupported();
                }
                Ok(3)
            }
            ColorSpace::Cmyk => {
                if self.lossless && self.jpeg_color_space != ColorSpace::Cmyk {
                    return unsupported();
                }
                if !matches!(self.jpeg_color_space, ColorSpace::Ycck | ColorSpace::Cmyk) {
                    return unsupported();
                }
                Ok(4)
            }
            _ => {
                if self.out_color_space == self.jpeg_color_space {
                    Ok(count)
                } else {
                    unsupported()
                }
            }
        }
    }

    /// `jinit_upsampler`'s check that every component's sampling is an
    /// integral fraction of the largest.
    fn check_sampling(&self) -> Result<(), JpegError> {
        for component in &self.components {
            if !self.max_h.is_multiple_of(component.h_samp) || !self.max_v.is_multiple_of(component.v_samp) {
                return Err(JpegError::new("Fractional sampling not implemented yet"));
            }
        }
        Ok(())
    }

    /// `per_scan_setup` and `latch_quant_tables`, then the entropy
    /// decoder's start (`start_input_pass`).
    fn start_input_pass(&mut self) -> Result<(), JpegError> {
        let unit = if self.lossless { 1 } else { 8 };
        let mut scan = std::mem::take(&mut self.scan);
        scan.mcu_membership.clear();
        if scan.comps_in_scan == 1 {
            let component = &mut self.components[scan.component_index[0]];
            scan.mcus_per_row = component.width_in_blocks;
            component.mcu_width = 1;
            component.mcu_height = 1;
            component.last_col_width = 1;
            let rows = component.height_in_blocks % component.v_samp;
            component.last_row_height = if rows == 0 { component.v_samp } else { rows };
            scan.blocks_in_mcu = 1;
            scan.mcu_membership.push(0);
        } else {
            scan.mcus_per_row = self.width.div_ceil(self.max_h * unit);
            scan.blocks_in_mcu = 0;
            for ci in 0..scan.comps_in_scan {
                let component = &mut self.components[scan.component_index[ci]];
                component.mcu_width = component.h_samp;
                component.mcu_height = component.v_samp;
                let columns = component.width_in_blocks % component.mcu_width;
                component.last_col_width = if columns == 0 { component.mcu_width } else { columns };
                let rows = component.height_in_blocks % component.mcu_height;
                component.last_row_height = if rows == 0 { component.mcu_height } else { rows };
                let blocks = component.mcu_width * component.mcu_height;
                if scan.blocks_in_mcu + blocks > 10 {
                    self.scan = scan;
                    return Err(JpegError::new("Sampling factors too large for interleaved scan"));
                }
                for _ in 0..blocks {
                    scan.mcu_membership.push(ci);
                }
                scan.blocks_in_mcu += blocks;
            }
        }
        if !self.lossless {
            for &index in &scan.component_index {
                if self.components[index].quant_table.is_some() {
                    continue;
                }
                let number = self.components[index].quant_tbl_no;
                let table = self.quant_tables.get(number).and_then(|table| *table);
                let Some(table) = table else {
                    self.scan = scan;
                    return Err(JpegError::new(&format!("Quantization table 0x{number:02x} was not defined")));
                };
                self.components[index].quant_table = Some(table);
            }
        }
        let count = self.components.len();
        let entropy = if self.lossless {
            Entropy::Lossless(huffman::Lossless::start(&scan, &self.dc_tables)?)
        } else if self.arith {
            let previous = match self.entropy.take() {
                Some(Entropy::Arithmetic(previous)) => Some(previous),
                _ => None,
            };
            Entropy::Arithmetic(arith::Arithmetic::start(&scan, self.progressive, &mut self.coef_bits, count, self.input_scan_number, previous, &mut self.source)?)
        } else if self.progressive {
            let progressive = huffman::Progressive::start(&scan, &self.dc_tables, &self.ac_tables, &mut self.coef_bits, count, self.input_scan_number, &mut self.source)?;
            Entropy::Progressive(Box::new(progressive))
        } else {
            Entropy::Sequential(huffman::Sequential::start(&scan, &self.dc_tables, &self.ac_tables, &mut self.source)?)
        };
        self.entropy = Some(entropy);
        self.scan = scan;
        Ok(())
    }

    /// The MCU rows of an iMCU row of this scan (`start_iMCU_row`).
    fn mcu_rows_in_imcu_row(&self, imcu_row: usize) -> usize {
        if self.scan.comps_in_scan > 1 {
            return 1;
        }
        let component = &self.components[self.scan.component_index[0]];
        if imcu_row < self.total_imcu_rows - 1 {
            component.v_samp
        } else {
            component.last_row_height
        }
    }

    /// `consume_data`: decodes the whole scan into the coefficient arrays.
    fn decode_scan(&mut self) -> Result<(), JpegError> {
        let mut entropy = self.entropy.take().expect("a scan has its entropy decoder");
        let mut mcu = [[0i16; 64]; 10];
        let mut positions = [(0usize, 0usize); 10];
        for imcu_row in 0..self.total_imcu_rows {
            let rows = self.mcu_rows_in_imcu_row(imcu_row);
            for yoffset in 0..rows {
                for column in 0..self.scan.mcus_per_row {
                    // The MCU's blocks, as component and block index.
                    let mut block_count = 0;
                    for ci in 0..self.scan.comps_in_scan {
                        let index = self.scan.component_index[ci];
                        let component = &self.components[index];
                        let start_col = column * component.mcu_width;
                        for yindex in 0..component.mcu_height {
                            let row = imcu_row * component.v_samp + yindex + yoffset;
                            for xindex in 0..component.mcu_width {
                                let block = row * self.block_stride[index] + start_col + xindex;
                                positions[block_count] = (index, block);
                                mcu[block_count] = self.coefficients[index][block];
                                block_count += 1;
                            }
                        }
                    }
                    if !entropy.insufficient_data() {
                        self.last_good_imcu_row = imcu_row;
                    }
                    let blocks = &mut mcu[..block_count];
                    match &mut entropy {
                        Entropy::Sequential(decoder) => decoder.decode_mcu(&self.scan, &mut self.source, &mut self.markers, blocks)?,
                        Entropy::Progressive(decoder) => decoder.decode_mcu(&self.scan, &mut self.source, &mut self.markers, blocks)?,
                        Entropy::Arithmetic(decoder) => decoder.decode_mcu(&self.scan, self.progressive, &self.arith_dc_l, &self.arith_dc_u, &self.arith_ac_k, &mut self.source, &mut self.markers, blocks)?,
                        Entropy::Lossless(_) => unreachable!("lossless scans decode rows"),
                    }
                    for (slot, &(index, block)) in positions.iter().enumerate().take(block_count) {
                        self.coefficients[index][block] = mcu[slot];
                    }
                }
            }
        }
        self.entropy = Some(entropy);
        Ok(())
    }

    /// Allocates the coefficient arrays (or, lossless, the sample planes).
    fn allocate(&mut self) {
        for component in &self.components {
            let stride = round_up(component.width_in_blocks, component.h_samp);
            let rows = round_up(component.height_in_blocks, component.v_samp);
            self.block_stride.push(stride);
            if self.lossless {
                self.planes.push(vec![0u16; stride * rows]);
            } else {
                self.coefficients.push(vec![[0i16; 64]; stride * rows]);
            }
        }
        if self.progressive {
            let mut bits = vec![[-1i32; 64]; self.components.len()];
            bits.extend(vec![[0i32; 64]; self.components.len()]);
            self.coef_bits = bits;
        }
    }

    /// The lossless difference controller (`jddiffct.c`) for one scan:
    /// differences row by row, undifferenced by the scan's predictor and
    /// scaled by the point transform into the sample planes.
    fn decode_lossless_scan(&mut self, state: &mut LosslessState) -> Result<(), JpegError> {
        let Some(Entropy::Lossless(mut entropy)) = self.entropy.take() else {
            unreachable!("a lossless scan has its decoder");
        };
        let scan = self.scan.clone();
        if scan.ss < 1 || scan.ss > 7 || scan.se != 0 || scan.ah != 0 || scan.al < 0 || scan.al >= self.precision {
            return Err(JpegError::new(&format!("Invalid progressive parameters Ss={} Se={} Ah={} Al={}", scan.ss, scan.se, scan.ah, scan.al)));
        }
        if !scan.restart_interval.is_multiple_of(scan.mcus_per_row as u32) {
            return Err(JpegError::new(&format!("Invalid restart interval {}, must be an integer multiple of the number of MCUs in an MCU row ({})", scan.restart_interval, scan.mcus_per_row)));
        }
        // The predictors start over at each scan.
        for first in state.first_row.iter_mut() {
            *first = true;
        }
        state.restart_rows_to_go = scan.restart_interval / scan.mcus_per_row as u32;
        let mut differences = Vec::new();
        let sample_mask: i32 = match self.precision {
            2..=8 => 0xFF,
            _ => 0xFFFF,
        };
        for imcu_row in 0..self.total_imcu_rows {
            let rows = self.mcu_rows_in_imcu_row(imcu_row);
            // Each scan component's difference rows for this iMCU row.
            let mut diff_rows: Vec<Vec<Vec<i32>>> = scan
                .component_index
                .iter()
                .map(|&index| {
                    let component = &self.components[index];
                    vec![vec![0i32; self.block_stride[index]]; component.v_samp]
                })
                .collect();
            for yoffset in 0..rows {
                if scan.restart_interval != 0 && state.restart_rows_to_go == 0 {
                    entropy.restart(&mut self.source, &mut self.markers)?;
                    for first in state.first_row.iter_mut() {
                        *first = true;
                    }
                    state.restart_rows_to_go = scan.restart_interval / scan.mcus_per_row as u32;
                }
                let decoded = entropy.decode_mcus(&scan, &mut self.source, &mut self.markers, scan.mcus_per_row, &mut differences)?;
                if !decoded {
                    for first in state.first_row.iter_mut() {
                        *first = true;
                    }
                }
                let mut cursor = 0;
                for column in 0..scan.mcus_per_row {
                    for (rows, &index) in diff_rows.iter_mut().zip(scan.component_index.iter()).take(scan.comps_in_scan) {
                        let component = &self.components[index];
                        for row in &mut rows[yoffset..yoffset + component.mcu_height] {
                            let start = column * component.mcu_width;
                            for value in &mut row[start..start + component.mcu_width] {
                                *value = differences[cursor];
                                cursor += 1;
                            }
                        }
                    }
                }
                if scan.restart_interval != 0 {
                    state.restart_rows_to_go -= 1;
                }
            }
            for (ci, &index) in scan.component_index.iter().enumerate() {
                let component = &self.components[index];
                let row_count = if imcu_row == self.total_imcu_rows - 1 { component.last_row_height } else { component.v_samp };
                let width = component.width_in_blocks;
                for (row, differences) in diff_rows[ci].iter().enumerate().take(row_count) {
                    let above = &state.undiff[index][(row + component.v_samp - 1) % component.v_samp];
                    let undiff = undifference(differences, above, width, scan.ss, self.precision, scan.al, state.first_row[index]);
                    state.first_row[index] = false;
                    let plane_row = imcu_row * component.v_samp + row;
                    let stride = self.block_stride[index];
                    for (x, &value) in undiff.iter().enumerate().take(width) {
                        let scaled = (value << scan.al) & sample_mask;
                        self.planes[index][plane_row * stride + x] = scaled as u16;
                    }
                    state.undiff[index][row] = undiff;
                }
            }
        }
        self.entropy = Some(Entropy::Lossless(entropy));
        Ok(())
    }
}

/// One row undifferenced (`jpeg_undifference*`): the first row of a scan
/// or after a restart predicts from its left neighbour (the first sample
/// from 2^(P−Pt−1)); later rows use the scan's predictor, their first
/// sample predicted from the sample above. Sums are taken modulo 2^16.
fn undifference(differences: &[i32], previous: &[i32], width: usize, predictor: i32, precision: i32, point_transform: i32, first_row: bool) -> Vec<i32> {
    let mut output = vec![0i32; width];
    if width == 0 {
        return output;
    }
    if first_row {
        let mut ra = (differences[0] + (1 << (precision - point_transform - 1))) & 0xFFFF;
        output[0] = ra;
        for x in 1..width {
            ra = (differences[x] + ra) & 0xFFFF;
            output[x] = ra;
        }
        return output;
    }
    if predictor == 1 {
        let mut ra = (differences[0] + previous[0]) & 0xFFFF;
        output[0] = ra;
        for x in 1..width {
            ra = (differences[x] + ra) & 0xFFFF;
            output[x] = ra;
        }
        return output;
    }
    let mut rb = previous[0];
    let mut ra = (differences[0] + rb) & 0xFFFF;
    output[0] = ra;
    for x in 1..width {
        let rc = rb;
        rb = previous[x];
        let prediction = match predictor {
            2 => rb,
            3 => rc,
            4 => ra + rb - rc,
            5 => ra + ((rb - rc) >> 1),
            6 => rb + ((ra - rc) >> 1),
            _ => (ra + rb) >> 1,
        };
        ra = (differences[x] + prediction) & 0xFFFF;
        output[x] = ra;
    }
    output
}

/// C `LocaleCompare` equality of a NUL-terminated signature with `text`,
/// ignoring case.
fn c_string_equals_ignore_case(signature: &[u8], text: &[u8]) -> bool {
    let end = signature.iter().position(|&byte| byte == 0).unwrap_or(signature.len());
    signature[..end].eq_ignore_ascii_case(text)
}

/// The standard tables for any of tables 0 and 1 a sequential Huffman
/// stream left undefined (`std_huff_tables`).
fn install_standard_tables(decoder: &mut Decoder) {
    let fill = |slot: &mut Option<HuffTable>, bits: &[u8; 17], values: &[u8]| {
        if slot.is_none() {
            *slot = Some(HuffTable::standard(bits, values));
        }
    };
    fill(&mut decoder.dc_tables[0], &tables::STD_DC_LUMINANCE_BITS, &tables::STD_DC_LUMINANCE_VALUES);
    fill(&mut decoder.ac_tables[0], &tables::STD_AC_LUMINANCE_BITS, &tables::STD_AC_LUMINANCE_VALUES);
    fill(&mut decoder.dc_tables[1], &tables::STD_DC_CHROMINANCE_BITS, &tables::STD_DC_CHROMINANCE_VALUES);
    fill(&mut decoder.ac_tables[1], &tables::STD_AC_CHROMINANCE_BITS, &tables::STD_AC_CHROMINANCE_VALUES);
}

/// Scans a file may have before the reader gives up ("too many scans").
const MAX_SCANS: i32 = 1024;

/// Decodes a JPEG as the reader drives libjpeg: the header, all the input
/// (for a multi-scan image) or the one scan, the output, then the markers
/// up to EOI, where errors no longer matter (but a profile cut short
/// still does).
pub fn decode(file: &[u8]) -> Result<Decoded, JpegError> {
    let mut decoder = Decoder::new(file);
    // jpeg_read_header: markers up to the first SOS.
    if !decoder.read_markers()? {
        if decoder.markers.saw_sof {
            return Err(JpegError::new("Invalid JPEG file structure: missing SOS marker"));
        }
        return Err(JpegError::new("JPEG datastream contains no image"));
    }
    decoder.initial_setup()?;
    decoder.default_color_spaces();
    let fax = decoder.profile_8bim_set && decoder.profile_8bim.len() >= 5 && &decoder.profile_8bim[..5] == b"G3FAX";
    if fax {
        decoder.out_color_space = ColorSpace::YCbCr;
    }
    // jpeg_start_decompress: the modules' checks, then the first scan.
    let output_components = decoder.check_color_conversion()?;
    decoder.check_sampling()?;
    if decoder.lossless && decoder.arith {
        return Err(JpegError::new("Sorry, arithmetic coding is not supported"));
    }
    if !decoder.lossless && !decoder.arith && !decoder.progressive {
        install_standard_tables(&mut decoder);
    }
    decoder.allocate();
    decoder.start_input_pass()?;
    let mut lossless_state = LosslessState {
        undiff: decoder.components.iter().map(|component| vec![vec![0i32; round_up(component.width_in_blocks, component.h_samp)]; component.v_samp]).collect(),
        first_row: vec![true; decoder.components.len()],
        restart_rows_to_go: 0,
    };
    loop {
        if decoder.lossless {
            decoder.decode_lossless_scan(&mut lossless_state)?;
        } else {
            decoder.decode_scan()?;
        }
        if !decoder.has_multiple_scans {
            break;
        }
        if !decoder.read_markers()? {
            break;
        }
        if decoder.input_scan_number >= MAX_SCANS {
            return Err(JpegError::new("too many scans"));
        }
        decoder.start_input_pass()?;
    }
    if output_components != 1 && output_components != 3 && output_components != 4 {
        return Err(JpegError::new("ImageTypeNotSupported"));
    }
    let samples = output::produce(&decoder);
    // jpeg_finish_decompress: a one-scan image's remaining markers are read
    // up to EOI; their errors only warn.
    if !decoder.has_multiple_scans {
        // A second SOS where none was expected is an error, but at this
        // point only a warning; so are other marker errors.
        if let Err(error) = decoder.read_markers()
            && error.0 == "InsufficientImageDataInFile"
        {
            return Err(error);
        }
    }
    Ok(Decoded { width: decoder.width, height: decoder.height, precision: decoder.precision, out_color_space: decoder.out_color_space, components: output_components, samples, fax })
}

/// IM `ScaleAnyToQuantum` in an HDRI build.
fn scale_any(value: u16, range: u32) -> f32 {
    if value as u32 > range {
        return 65535.0;
    }
    (65535.0 * (value as f64 * (1.0 / range as f64))) as f32
}

/// `JPEGGetQuantum`.
fn quantum(value: u16, precision: i32) -> f32 {
    match precision {
        8 => 257.0 * value as f32,
        16 => value as f32,
        _ => scale_any(value, (1u32 << precision) - 1),
    }
}

/// `ReadJPEGImage`: what the pixel cache holds after reading a JPEG.
pub fn read(path: &Path) -> Result<PixelCache> {
    let file = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    if file.len() < 107 {
        anyhow::bail!("InsufficientImageDataInFile");
    }
    let decoded = decode(&file).map_err(|error| anyhow::anyhow!("{error}"))?;
    Ok(to_pixel_cache(&decoded))
}

pub(crate) fn to_pixel_cache(decoded: &Decoded) -> PixelCache {
    let width = decoded.width;
    let height = decoded.height;
    let precision = decoded.precision;
    let pixels = width * height;
    match decoded.components {
        1 => {
            // A colormap of every grey the precision has; the index is the
            // sample.
            // `QuantumRange` is a float, so the ramp's step is a float
            // quotient.
            let colors = 1u64 << precision;
            let step = (65535.0f32 / (colors - 1).max(1) as f32) as f64;
            let mut data = Vec::with_capacity(pixels * 2);
            for &sample in &decoded.samples[..pixels] {
                let index = if (sample as u64) < colors { sample as u64 } else { 0 };
                data.push((index as f64 * step) as f32);
                data.push(index as f32);
            }
            let colorspace = if decoded.fax { CacheColorspace::Lab } else { CacheColorspace::Gray };
            let channels = vec![PixelChannel::Gray, PixelChannel::Index];
            let mut cache = PixelCache::new(width, height, channels, data, colorspace, DEFAULT_GAMMA);
            cache.colormap = (0..colors).map(|index| [index as f64 * step, index as f64 * step, index as f64 * step, 65535.0]).collect();
            cache
        }
        4 => {
            let mut data = Vec::with_capacity(pixels * 4);
            for &sample in &decoded.samples[..pixels * 4] {
                data.push((65535.0 - quantum(sample, precision) as f64) as f32);
            }
            let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Black];
            PixelCache::new(width, height, channels, data, CacheColorspace::Cmyk, DEFAULT_GAMMA)
        }
        _ => {
            let data = decoded.samples[..pixels * 3].iter().map(|&sample| quantum(sample, precision)).collect();
            let colorspace = if decoded.fax { CacheColorspace::Lab } else { CacheColorspace::SRgb };
            let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
            PixelCache::new(width, height, channels, data, colorspace, DEFAULT_GAMMA)
        }
    }
}

/// The tables libjpeg keeps from one datastream to the next (they survive
/// `jpeg_abort`): what a TIFF's JPEGTables and earlier strips define.
#[derive(Clone, Default)]
pub(crate) struct Tables {
    quant: [Option<[u16; 64]>; 4],
    dc: [Option<HuffTable>; 4],
    ac: [Option<HuffTable>; 4],
}

impl Tables {
    fn take_from(&mut self, decoder: &Decoder) {
        self.quant = decoder.quant_tables;
        self.dc = decoder.dc_tables.clone();
        self.ac = decoder.ac_tables.clone();
    }

    /// `jpeg_read_header(cinfo, FALSE)` on an abbreviated table-
    /// specification datastream: true when it held tables only.
    pub fn read(&mut self, stream: &[u8]) -> Result<bool, JpegError> {
        let mut decoder = Decoder::new_libtiff(stream, self);
        let image = decoder.read_markers()?;
        if !image && decoder.markers.saw_sof {
            return Err(JpegError::new("Invalid JPEG file structure: missing SOS marker"));
        }
        self.take_from(&decoder);
        Ok(!image)
    }
}

/// How libtiff has libjpeg deliver a strip or tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TiffOutput {
    /// Colour conversion suppressed (`JCS_UNKNOWN` in and out).
    Unconverted,
    /// YCbCr to RGB with fancy upsampling (`JPEGCOLORMODE_RGB`).
    YCbCrToRgb,
    /// The downsampled components (`raw_data_out`).
    Raw,
}

/// What `jpeg_read_header` learnt of a strip, for libtiff's checks.
pub(crate) struct StripInfo {
    pub width: usize,
    pub height: usize,
    pub precision: i32,
    /// Each component's sampling factors and downsampled width.
    pub sampling: Vec<(usize, usize, usize)>,
}

/// A strip's samples: interleaved rows, or one plane per component
/// (rows padded to whole blocks).
pub(crate) enum StripSamples {
    Interleaved { components: usize, samples: Vec<u16> },
    Raw { planes: Vec<(Vec<u16>, usize)> },
}

/// One JPEG datastream of a TIFF strip or tile.
pub(crate) struct Strip<'a> {
    decoder: Decoder<'a>,
}

impl<'a> Strip<'a> {
    /// `jpeg_read_header(cinfo, TRUE)` on the strip's data.
    pub fn open(data: &'a [u8], tables: &Tables) -> Result<Self, JpegError> {
        let mut decoder = Decoder::new_libtiff(data, tables);
        if !decoder.read_markers()? {
            if decoder.markers.saw_sof {
                return Err(JpegError::new("Invalid JPEG file structure: missing SOS marker"));
            }
            return Err(JpegError::new("JPEG datastream contains no image"));
        }
        decoder.initial_setup()?;
        decoder.default_color_spaces();
        Ok(Self { decoder })
    }

    pub fn info(&self) -> StripInfo {
        let decoder = &self.decoder;
        let sampling = decoder.components.iter().map(|component| (component.h_samp, component.v_samp, component.downsampled_width)).collect();
        StripInfo { width: decoder.width, height: decoder.height, precision: decoder.precision, sampling }
    }

    /// `jpeg_start_decompress`, the output, and — when `finish` (every
    /// output row was read) — `jpeg_finish_decompress`, whose errors are
    /// libtiff's errors. The tables are kept for the next strip.
    pub fn decode(mut self, output: TiffOutput, finish: bool, tables: &mut Tables) -> Result<StripSamples, JpegError> {
        let decoder = &mut self.decoder;
        match output {
            TiffOutput::YCbCrToRgb => {
                decoder.jpeg_color_space = ColorSpace::YCbCr;
                decoder.out_color_space = ColorSpace::Rgb;
            }
            _ => {
                decoder.jpeg_color_space = ColorSpace::Unknown;
                decoder.out_color_space = ColorSpace::Unknown;
            }
        }
        let output_components = if output == TiffOutput::Raw {
            decoder.components.len()
        } else {
            let count = decoder.check_color_conversion()?;
            decoder.check_sampling()?;
            count
        };
        if decoder.lossless && decoder.arith {
            return Err(JpegError::new("Sorry, arithmetic coding is not supported"));
        }
        if !decoder.lossless && !decoder.arith && !decoder.progressive {
            install_standard_tables(decoder);
        }
        decoder.allocate();
        decoder.start_input_pass()?;
        let mut lossless_state = LosslessState {
            undiff: decoder.components.iter().map(|component| vec![vec![0i32; round_up(component.width_in_blocks, component.h_samp)]; component.v_samp]).collect(),
            first_row: vec![true; decoder.components.len()],
            restart_rows_to_go: 0,
        };
        loop {
            if decoder.lossless {
                decoder.decode_lossless_scan(&mut lossless_state)?;
            } else {
                decoder.decode_scan()?;
            }
            if !decoder.has_multiple_scans {
                break;
            }
            if !decoder.read_markers()? {
                break;
            }
            if decoder.input_scan_number >= MAX_SCANS {
                return Err(JpegError::new("too many scans"));
            }
            decoder.start_input_pass()?;
        }
        let samples = if output == TiffOutput::Raw {
            StripSamples::Raw { planes: output::raw_planes(decoder) }
        } else {
            StripSamples::Interleaved { components: output_components, samples: output::produce(decoder) }
        };
        if finish && !decoder.has_multiple_scans && decoder.read_markers()? {
            return Err(JpegError::new("Didn't expect more than one scan"));
        }
        tables.take_from(decoder);
        Ok(samples)
    }
}
