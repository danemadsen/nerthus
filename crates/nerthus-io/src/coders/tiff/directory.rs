//! A TIFF header and its first directory as libtiff 4.7.1 reads them
//! (`TIFFClientOpen`, `TIFFReadDirectory`): the tags in two passes with
//! libtiff's type conversions and range checks, its defaults, and its
//! fix-ups — strip byte counts estimated when missing or bogus, a single
//! uncompressed strip chopped into ~8 KiB strips (which sets RowsPerStrip),
//! samples beyond the photometric's colours made extra samples.
//!
//! libtiff errors fail the read (ImageMagick turns them into exceptions
//! that Magick.NET throws); its warnings are dropped.

use anyhow::{anyhow, bail, Result};

pub(super) const COMPRESSION_NONE: u16 = 1;
pub(super) const COMPRESSION_CCITTRLE: u16 = 2;
pub(super) const COMPRESSION_CCITTFAX3: u16 = 3;
pub(super) const COMPRESSION_CCITTFAX4: u16 = 4;
pub(super) const COMPRESSION_LZW: u16 = 5;
pub(super) const COMPRESSION_OJPEG: u16 = 6;
pub(super) const COMPRESSION_JPEG: u16 = 7;
pub(super) const COMPRESSION_ADOBE_DEFLATE: u16 = 8;
pub(super) const COMPRESSION_NEXT: u16 = 32766;
pub(super) const COMPRESSION_CCITTRLEW: u16 = 32771;
pub(super) const COMPRESSION_PACKBITS: u16 = 32773;
pub(super) const COMPRESSION_THUNDERSCAN: u16 = 32809;
pub(super) const COMPRESSION_DEFLATE: u16 = 32946;
pub(super) const COMPRESSION_SGILOG: u16 = 34676;
pub(super) const COMPRESSION_SGILOG24: u16 = 34677;
pub(super) const COMPRESSION_LZMA: u16 = 34925;
pub(super) const COMPRESSION_WEBP: u16 = 50001;

pub(super) const PHOTOMETRIC_MINISWHITE: u16 = 0;
pub(super) const PHOTOMETRIC_MINISBLACK: u16 = 1;
pub(super) const PHOTOMETRIC_RGB: u16 = 2;
pub(super) const PHOTOMETRIC_PALETTE: u16 = 3;
pub(super) const PHOTOMETRIC_MASK: u16 = 4;
pub(super) const PHOTOMETRIC_SEPARATED: u16 = 5;
pub(super) const PHOTOMETRIC_YCBCR: u16 = 6;
pub(super) const PHOTOMETRIC_CIELAB: u16 = 8;
pub(super) const PHOTOMETRIC_ICCLAB: u16 = 9;
pub(super) const PHOTOMETRIC_ITULAB: u16 = 10;
pub(super) const PHOTOMETRIC_LOGL: u16 = 32844;
pub(super) const PHOTOMETRIC_LOGLUV: u16 = 32845;

pub(super) const PLANARCONFIG_CONTIG: u16 = 1;
pub(super) const PLANARCONFIG_SEPARATE: u16 = 2;

pub(super) const SAMPLEFORMAT_UINT: u16 = 1;
pub(super) const SAMPLEFORMAT_INT: u16 = 2;
pub(super) const SAMPLEFORMAT_IEEEFP: u16 = 3;

pub(super) const EXTRASAMPLE_UNSPECIFIED: u16 = 0;
pub(super) const EXTRASAMPLE_ASSOCALPHA: u16 = 1;
pub(super) const EXTRASAMPLE_UNASSALPHA: u16 = 2;

pub(super) const FILLORDER_MSB2LSB: u16 = 1;
pub(super) const FILLORDER_LSB2MSB: u16 = 2;

pub(super) const ORIENTATION_TOPLEFT: u16 = 1;

pub(super) const INKSET_CMYK: u16 = 1;

const TAG_OSUBFILETYPE: u16 = 255;
const TAG_IMAGEWIDTH: u16 = 256;
const TAG_IMAGELENGTH: u16 = 257;
const TAG_BITSPERSAMPLE: u16 = 258;
const TAG_COMPRESSION: u16 = 259;
const TAG_PHOTOMETRIC: u16 = 262;
const TAG_FILLORDER: u16 = 266;
const TAG_STRIPOFFSETS: u16 = 273;
const TAG_ORIENTATION: u16 = 274;
const TAG_SAMPLESPERPIXEL: u16 = 277;
const TAG_ROWSPERSTRIP: u16 = 278;
const TAG_STRIPBYTECOUNTS: u16 = 279;
const TAG_MINSAMPLEVALUE: u16 = 280;
const TAG_MAXSAMPLEVALUE: u16 = 281;
const TAG_XRESOLUTION: u16 = 282;
const TAG_YRESOLUTION: u16 = 283;
const TAG_PLANARCONFIG: u16 = 284;
const TAG_GROUP3OPTIONS: u16 = 292;
const TAG_GROUP4OPTIONS: u16 = 293;
const TAG_RESOLUTIONUNIT: u16 = 296;
const TAG_TRANSFERFUNCTION: u16 = 301;
const TAG_PREDICTOR: u16 = 317;
const TAG_WHITEPOINT: u16 = 318;
const TAG_COLORMAP: u16 = 320;
const TAG_TILEWIDTH: u16 = 322;
const TAG_TILELENGTH: u16 = 323;
const TAG_TILEOFFSETS: u16 = 324;
const TAG_TILEBYTECOUNTS: u16 = 325;
const TAG_BADFAXLINES: u16 = 326;
const TAG_CLEANFAXDATA: u16 = 327;
const TAG_CONSECUTIVEBADFAXLINES: u16 = 328;
const TAG_INKSET: u16 = 332;
const TAG_EXTRASAMPLES: u16 = 338;
const TAG_SAMPLEFORMAT: u16 = 339;
const TAG_SMINSAMPLEVALUE: u16 = 340;
const TAG_SMAXSAMPLEVALUE: u16 = 341;
const TAG_JPEGTABLES: u16 = 347;
const TAG_JPEGPROC: u16 = 512;
const TAG_JPEGIFOFFSET: u16 = 513;
const TAG_JPEGIFBYTECOUNT: u16 = 514;
const TAG_JPEGRESTARTINTERVAL: u16 = 515;
const TAG_JPEGQTABLES: u16 = 519;
const TAG_JPEGDCTABLES: u16 = 520;
const TAG_JPEGACTABLES: u16 = 521;
const TAG_YCBCRCOEFFICIENTS: u16 = 529;
const TAG_YCBCRSUBSAMPLING: u16 = 530;
const TAG_YCBCRPOSITIONING: u16 = 531;
const TAG_REFERENCEBLACKWHITE: u16 = 532;
const TAG_MATTEING: u16 = 32995;
const TAG_DATATYPE: u16 = 32996;
const TAG_IMAGEDEPTH: u16 = 32997;
const TAG_TILEDEPTH: u16 = 32998;
const TAG_LERC_PARAMETERS: u16 = 50674;

const TYPE_BYTE: u16 = 1;
const TYPE_ASCII: u16 = 2;
const TYPE_SHORT: u16 = 3;
const TYPE_LONG: u16 = 4;
const TYPE_RATIONAL: u16 = 5;
const TYPE_SBYTE: u16 = 6;
const TYPE_UNDEFINED: u16 = 7;
const TYPE_SSHORT: u16 = 8;
const TYPE_SLONG: u16 = 9;
const TYPE_SRATIONAL: u16 = 10;
const TYPE_FLOAT: u16 = 11;
const TYPE_DOUBLE: u16 = 12;
const TYPE_IFD: u16 = 13;
const TYPE_LONG8: u16 = 16;
const TYPE_SLONG8: u16 = 17;
const TYPE_IFD8: u16 = 18;

/// `STRIP_SIZE_DEFAULT`: the strip size strip chopping aims for.
const STRIP_SIZE_DEFAULT: u64 = 8192;

/// `TIFFDataWidth`.
fn data_width(field_type: u16) -> u64 {
    match field_type {
        TYPE_BYTE | TYPE_ASCII | TYPE_SBYTE | TYPE_UNDEFINED => 1,
        TYPE_SHORT | TYPE_SSHORT => 2,
        TYPE_LONG | TYPE_SLONG | TYPE_FLOAT | TYPE_IFD => 4,
        TYPE_RATIONAL | TYPE_SRATIONAL | TYPE_DOUBLE | TYPE_LONG8 | TYPE_SLONG8 | TYPE_IFD8 => 8,
        _ => 0,
    }
}

/// Whether a compression scheme has a codec in the Windows Magick.Native
/// (`TIFFIsCODECConfigured`): its libtiff lacks JBIG, PixarLog, ZSTD and
/// LERC.
pub(super) fn codec_configured(compression: u16) -> bool {
    matches!(
        compression,
        COMPRESSION_NONE
            | COMPRESSION_LZW
            | COMPRESSION_PACKBITS
            | COMPRESSION_THUNDERSCAN
            | COMPRESSION_NEXT
            | COMPRESSION_JPEG
            | COMPRESSION_OJPEG
            | COMPRESSION_CCITTRLE
            | COMPRESSION_CCITTRLEW
            | COMPRESSION_CCITTFAX3
            | COMPRESSION_CCITTFAX4
            | COMPRESSION_ADOBE_DEFLATE
            | COMPRESSION_DEFLATE
            | COMPRESSION_SGILOG
            | COMPRESSION_SGILOG24
            | COMPRESSION_LZMA
            | COMPRESSION_WEBP
    )
}

/// `TIFFReadDirEntryErr`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryError {
    Count,
    Type,
    Io,
    Range,
    PerSampleDiffers,
    Size,
}

/// One directory entry (`TIFFDirEntry`); its value field stays in the
/// file, at `value_position`.
#[derive(Clone, Copy)]
struct Entry {
    tag: u16,
    field_type: u16,
    count: u64,
    value_position: usize,
    ignore: bool,
}

/// The OJPEG codec's tags.
#[derive(Clone, Debug, Default)]
pub(super) struct OldJpegTags {
    pub proc: Option<u16>,
    pub interchange_offset: Option<u64>,
    pub interchange_length: Option<u64>,
    pub restart_interval: Option<u16>,
    pub q_tables: Vec<u64>,
    pub dc_tables: Vec<u64>,
    pub ac_tables: Vec<u64>,
}

/// `TIFFDirectory`: what the reader needs of the first directory.
#[derive(Clone, Debug)]
pub(super) struct Directory {
    pub width: u32,
    pub length: u32,
    pub image_depth: u32,
    pub bits_per_sample: u16,
    pub samples_per_pixel: u16,
    pub sample_format: u16,
    pub compression: u16,
    pub photometric: Option<u16>,
    pub fill_order: u16,
    pub planar_config: u16,
    pub orientation: u16,
    pub rows_per_strip: u32,
    pub rows_per_strip_set: bool,
    pub tile_width: u32,
    pub tile_length: u32,
    pub tile_depth: u32,
    pub tiled: bool,
    pub strip_count: u32,
    pub strips_per_image: u32,
    pub strip_offsets: Vec<u64>,
    pub strip_byte_counts: Vec<u64>,
    pub extra_samples: Vec<u16>,
    pub colormap: Option<[Vec<u16>; 3]>,
    pub max_sample_value: u16,
    pub ycbcr_subsampling: [u16; 2],
    pub ycbcr_subsampling_set: bool,
    pub ycbcr_positioning: u16,
    pub ycbcr_coefficients: Option<[f32; 3]>,
    pub reference_black_white: Option<[f32; 6]>,
    pub white_point: Option<[f32; 2]>,
    pub ink_set: u16,
    pub predictor: u16,
    pub jpeg_tables: Option<Vec<u8>>,
    pub group3_options: u32,
    pub group4_options: u32,
    pub old_jpeg: OldJpegTags,
}

impl Directory {
    /// `TIFFDefaultDirectory`.
    fn new() -> Self {
        Self {
            width: 0,
            length: 0,
            image_depth: 1,
            bits_per_sample: 1,
            samples_per_pixel: 1,
            sample_format: SAMPLEFORMAT_UINT,
            compression: COMPRESSION_NONE,
            photometric: None,
            fill_order: FILLORDER_MSB2LSB,
            planar_config: PLANARCONFIG_CONTIG,
            orientation: ORIENTATION_TOPLEFT,
            rows_per_strip: u32::MAX,
            rows_per_strip_set: false,
            tile_width: 0,
            tile_length: 0,
            tile_depth: 1,
            tiled: false,
            strip_count: 0,
            strips_per_image: 0,
            strip_offsets: Vec::new(),
            strip_byte_counts: Vec::new(),
            extra_samples: Vec::new(),
            colormap: None,
            max_sample_value: 1,
            ycbcr_subsampling: [2, 2],
            ycbcr_subsampling_set: false,
            ycbcr_positioning: 1,
            ycbcr_coefficients: None,
            reference_black_white: None,
            white_point: None,
            ink_set: INKSET_CMYK,
            predictor: 1,
            jpeg_tables: None,
            group3_options: 0,
            group4_options: 0,
            old_jpeg: OldJpegTags::default(),
        }
    }
}

/// An open TIFF (`TIFF`): the file, its byte order and kind, and the
/// first directory.
pub(super) struct Tiff<'a> {
    pub data: &'a [u8],
    /// The file is big-endian (`TIFF_SWAB` on a little-endian host).
    pub swab: bool,
    pub bigtiff: bool,
    /// ImageMagick memory-maps files above 512 KiB (`TIFF_MAPPED`).
    pub mapped: bool,
    pub dir: Directory,
    /// JPEG decoding converts YCbCr to RGB (`JPEGCOLORMODE_RGB`, set by
    /// the RGBA interface), which also makes strips "upsampled".
    pub jpeg_color_mode_rgb: bool,
    /// SGILog decodes to 8-bit samples (`SGILOGDATAFMT_8BIT`, set by the
    /// RGBA interface).
    pub sgilog_8bit: bool,
}

fn missing_required(name: &str) -> anyhow::Error {
    anyhow!("TIFF directory is missing required \"{name}\" field")
}

fn entry_error(error: EntryError, name: &str) -> anyhow::Error {
    let what = match error {
        EntryError::Count => "Incorrect count for",
        EntryError::Type => "Incompatible type for",
        EntryError::Io => "IO error during reading of",
        EntryError::Range => "Incorrect value for",
        EntryError::PerSampleDiffers => "Cannot handle different values per sample for",
        EntryError::Size => "Sanity check on size of",
    };
    anyhow!("{what} \"{name}\"")
}

/// `TIFFhowmany_32`.
pub(super) fn how_many(x: u32, y: u32) -> u32 {
    if y == 0 {
        return 0;
    }
    (x as u64).div_ceil(y as u64) as u32
}

/// `TIFFhowmany8_64`.
pub(super) fn bytes_for_bits(bits: u64) -> u64 {
    bits.div_ceil(8)
}

impl<'a> Tiff<'a> {
    pub fn u16_at(&self, bytes: &[u8]) -> u16 {
        let pair = [bytes[0], bytes[1]];
        if self.swab {
            u16::from_be_bytes(pair)
        } else {
            u16::from_le_bytes(pair)
        }
    }

    pub fn u32_at(&self, bytes: &[u8]) -> u32 {
        let quad = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if self.swab {
            u32::from_be_bytes(quad)
        } else {
            u32::from_le_bytes(quad)
        }
    }

    pub fn u64_at(&self, bytes: &[u8]) -> u64 {
        let mut eight = [0u8; 8];
        eight.copy_from_slice(&bytes[..8]);
        if self.swab {
            u64::from_be_bytes(eight)
        } else {
            u64::from_le_bytes(eight)
        }
    }

    /// `TIFFClientOpen` for reading: the header, then the first directory.
    pub fn open(data: &'a [u8], mapped: bool) -> Result<Self> {
        if data.len() < 8 {
            bail!("Cannot read TIFF header");
        }
        let swab = match &data[..2] {
            b"II" => false,
            b"MM" => true,
            _ => bail!("Not a TIFF or MDI file, bad magic number"),
        };
        let mut tiff = Tiff { data, swab, bigtiff: false, mapped, dir: Directory::new(), jpeg_color_mode_rgb: false, sgilog_8bit: false };
        let version = tiff.u16_at(&data[2..4]);
        let first_directory = match version {
            42 => tiff.u32_at(&data[4..8]) as u64,
            43 => {
                if data.len() < 16 {
                    bail!("Cannot read TIFF header");
                }
                tiff.bigtiff = true;
                if tiff.u16_at(&data[4..6]) != 8 {
                    bail!("Not a TIFF file, bad BigTIFF offsetsize");
                }
                if tiff.u16_at(&data[6..8]) != 0 {
                    bail!("Not a TIFF file, bad BigTIFF unused");
                }
                tiff.u64_at(&data[8..16])
            }
            _ => bail!("Not a TIFF file, bad version number"),
        };
        if first_directory == 0 {
            bail!("UnableToReadImageData");
        }
        tiff.read_directory(first_directory)?;
        Ok(tiff)
    }

    /// `TIFFFetchDirectory`: the entries of the directory at `offset`.
    fn fetch_directory(&self, offset: u64) -> Result<Vec<Entry>> {
        let data = self.data;
        let count_size = if self.bigtiff { 8 } else { 2 };
        let entry_size: u64 = if self.bigtiff { 20 } else { 12 };
        let count_end = offset.checked_add(count_size).filter(|&end| end <= data.len() as u64);
        let Some(count_end) = count_end else {
            bail!("Can not read TIFF directory count");
        };
        let count_bytes = &data[offset as usize..count_end as usize];
        let count = if self.bigtiff { self.u64_at(count_bytes) } else { self.u16_at(count_bytes) as u64 };
        if count > 4096 {
            bail!("Sanity check on directory count failed, this is probably not a valid IFD offset");
        }
        if count == 0 {
            bail!("Sanity check on directory count failed, zero tag directories not supported");
        }
        let entries_end = count_end + count * entry_size;
        if entries_end > data.len() as u64 {
            bail!("Can not read TIFF directory");
        }
        let mut entries = Vec::with_capacity(count as usize);
        for index in 0..count {
            let start = (count_end + index * entry_size) as usize;
            let raw = &data[start..start + entry_size as usize];
            let tag = self.u16_at(&raw[0..2]);
            let field_type = self.u16_at(&raw[2..4]);
            let (entry_count, value_position) = if self.bigtiff { (self.u64_at(&raw[4..12]), start + 12) } else { (self.u32_at(&raw[4..8]) as u64, start + 8) };
            entries.push(Entry { tag, field_type, count: entry_count, value_position, ignore: false });
        }
        Ok(entries)
    }

    /// The raw bytes of an entry's first `count` values: inline when the
    /// entry fits its value field, otherwise at its offset
    /// (`TIFFReadDirEntryArrayWithLimit`).
    fn entry_bytes(&self, entry: &Entry, count: u64) -> Result<&'a [u8], EntryError> {
        let width = data_width(entry.field_type);
        if count == 0 || width == 0 {
            return Ok(&[]);
        }
        if 2_147_483_647 / width < count {
            return Err(EntryError::Size);
        }
        let size = count * width;
        let inline = if self.bigtiff { 8 } else { 4 };
        let original = entry.count.min(10) * width;
        if original <= inline && size <= inline {
            let start = entry.value_position;
            return Ok(&self.data[start..start + size as usize]);
        }
        let field = &self.data[entry.value_position..];
        let offset = if self.bigtiff { self.u64_at(field) } else { self.u32_at(field) as u64 };
        let end = offset.checked_add(size).ok_or(EntryError::Io)?;
        if end > self.data.len() as u64 {
            return Err(EntryError::Io);
        }
        Ok(&self.data[offset as usize..end as usize])
    }

    /// An entry's first `count` values as integers, for any integer type.
    fn entry_integers(&self, entry: &Entry, count: u64) -> Result<Vec<i128>, EntryError> {
        let bytes = self.entry_bytes(entry, count)?;
        let width = data_width(entry.field_type) as usize;
        let mut values = Vec::with_capacity(count as usize);
        for chunk in bytes.chunks_exact(width.max(1)) {
            let value = match entry.field_type {
                TYPE_BYTE | TYPE_UNDEFINED | TYPE_ASCII => chunk[0] as i128,
                TYPE_SBYTE => chunk[0] as i8 as i128,
                TYPE_SHORT => self.u16_at(chunk) as i128,
                TYPE_SSHORT => self.u16_at(chunk) as i16 as i128,
                TYPE_LONG | TYPE_IFD => self.u32_at(chunk) as i128,
                TYPE_SLONG => self.u32_at(chunk) as i32 as i128,
                TYPE_LONG8 | TYPE_IFD8 => self.u64_at(chunk) as i128,
                TYPE_SLONG8 => self.u64_at(chunk) as i64 as i128,
                _ => return Err(EntryError::Type),
            };
            values.push(value);
        }
        Ok(values)
    }

    /// `TIFFReadDirEntryShort`/`Long`/`Long8`: one integer value within
    /// `0..=max`.
    fn entry_unsigned(&self, entry: &Entry, max: u64) -> Result<u64, EntryError> {
        if entry.count != 1 {
            return Err(EntryError::Count);
        }
        let allowed = matches!(entry.field_type, TYPE_BYTE | TYPE_SBYTE | TYPE_SHORT | TYPE_SSHORT | TYPE_LONG | TYPE_SLONG | TYPE_LONG8 | TYPE_SLONG8);
        let allowed = allowed || (max == u64::MAX && matches!(entry.field_type, TYPE_IFD | TYPE_IFD8));
        if !allowed {
            return Err(EntryError::Type);
        }
        let values = self.entry_integers(entry, 1)?;
        let value = values[0];
        if value < 0 || value > max as i128 {
            return Err(EntryError::Range);
        }
        Ok(value as u64)
    }

    /// `TIFFReadDirEntryShortArray` (and the like): every value of an
    /// integer entry within `0..=max`, at most `limit` of them.
    fn entry_unsigned_array(&self, entry: &Entry, max: u64, limit: u64) -> Result<Vec<u64>, EntryError> {
        let allowed = matches!(entry.field_type, TYPE_BYTE | TYPE_SBYTE | TYPE_SHORT | TYPE_SSHORT | TYPE_LONG | TYPE_SLONG | TYPE_LONG8 | TYPE_SLONG8);
        let allowed = allowed || (max == u64::MAX && matches!(entry.field_type, TYPE_IFD | TYPE_IFD8));
        if !allowed {
            return Err(EntryError::Type);
        }
        let count = entry.count.min(limit);
        let values = self.entry_integers(entry, count)?;
        let mut unsigned = Vec::with_capacity(values.len());
        for value in values {
            if value < 0 || value > max as i128 {
                return Err(EntryError::Range);
            }
            unsigned.push(value as u64);
        }
        Ok(unsigned)
    }

    /// `TIFFReadDirEntryPersampleShort`: the first value, which every
    /// sample must share.
    fn entry_per_sample_short(&self, entry: &Entry) -> Result<u16, EntryError> {
        let samples = self.dir.samples_per_pixel as u64;
        if entry.count < samples {
            return Err(EntryError::Count);
        }
        let values = self.entry_unsigned_array(entry, u16::MAX as u64, u64::MAX)?;
        let first = values.first().copied().unwrap_or(0);
        for &value in values.iter().take(samples as usize).skip(1) {
            if value != first {
                return Err(EntryError::PerSampleDiffers);
            }
        }
        Ok(first as u16)
    }

    /// `TIFFReadDirEntryShort`, falling back to the per-sample form when
    /// the entry has several values.
    fn entry_short_or_per_sample(&self, entry: &Entry) -> Result<u16, EntryError> {
        match self.entry_unsigned(entry, u16::MAX as u64) {
            Err(EntryError::Count) => self.entry_per_sample_short(entry),
            other => other.map(|value| value as u16),
        }
    }

    /// `TIFFReadDirEntryFloatArray`: an entry's values as floats.
    fn entry_floats(&self, entry: &Entry) -> Result<Vec<f32>, EntryError> {
        let width = data_width(entry.field_type) as usize;
        match entry.field_type {
            TYPE_RATIONAL | TYPE_SRATIONAL => {
                let bytes = self.entry_bytes(entry, entry.count)?;
                let mut values = Vec::with_capacity(entry.count as usize);
                for chunk in bytes.as_chunks::<8>().0 {
                    let numerator = self.u32_at(&chunk[0..4]);
                    let denominator = self.u32_at(&chunk[4..8]);
                    let value = if denominator == 0 {
                        0.0
                    } else if entry.field_type == TYPE_RATIONAL {
                        numerator as f32 / denominator as f32
                    } else {
                        numerator as i32 as f32 / denominator as f32
                    };
                    values.push(value);
                }
                Ok(values)
            }
            TYPE_FLOAT => {
                let bytes = self.entry_bytes(entry, entry.count)?;
                Ok(bytes.chunks_exact(width).map(|chunk| f32::from_bits(self.u32_at(chunk))).collect())
            }
            TYPE_DOUBLE => {
                let bytes = self.entry_bytes(entry, entry.count)?;
                Ok(bytes.chunks_exact(width).map(|chunk| f64::from_bits(self.u64_at(chunk)) as f32).collect())
            }
            _ => {
                let values = self.entry_integers(entry, entry.count)?;
                Ok(values.into_iter().map(|value| value as f32).collect())
            }
        }
    }

    /// `TIFFReadDirEntryFloat`/`Double` on one value: a resolution.
    fn entry_double(&self, entry: &Entry) -> Result<f64, EntryError> {
        if entry.count != 1 {
            return Err(EntryError::Count);
        }
        let bytes = self.entry_bytes(entry, 1)?;
        match entry.field_type {
            TYPE_RATIONAL | TYPE_SRATIONAL => {
                let numerator = self.u32_at(&bytes[0..4]);
                let denominator = self.u32_at(&bytes[4..8]);
                let signed = entry.field_type == TYPE_SRATIONAL;
                let numerator_value = if signed { numerator as i32 as f64 } else { numerator as f64 };
                if numerator_value == 0.0 || denominator == 0 {
                    return Ok(0.0);
                }
                Ok(numerator_value / denominator as f64)
            }
            TYPE_FLOAT => Ok(f32::from_bits(self.u32_at(bytes)) as f64),
            TYPE_DOUBLE => Ok(f64::from_bits(self.u64_at(bytes))),
            _ => {
                let values = self.entry_integers(entry, 1)?;
                Ok(values[0] as f64)
            }
        }
    }

    /// `_TIFFCheckFieldIsValidForCodec`: codec tags count only for their
    /// own (configured) codec.
    fn tag_valid_for_codec(&self, tag: u16) -> bool {
        let codec_tag = matches!(
            tag,
            TAG_PREDICTOR
                | TAG_JPEGTABLES
                | TAG_JPEGIFOFFSET
                | TAG_JPEGIFBYTECOUNT
                | TAG_JPEGQTABLES
                | TAG_JPEGDCTABLES
                | TAG_JPEGACTABLES
                | TAG_JPEGPROC
                | TAG_JPEGRESTARTINTERVAL
                | TAG_BADFAXLINES
                | TAG_CLEANFAXDATA
                | TAG_CONSECUTIVEBADFAXLINES
                | TAG_GROUP3OPTIONS
                | TAG_GROUP4OPTIONS
                | TAG_LERC_PARAMETERS
        );
        if !codec_tag {
            return true;
        }
        let compression = self.dir.compression;
        if !codec_configured(compression) {
            return false;
        }
        match compression {
            COMPRESSION_LZW | COMPRESSION_ADOBE_DEFLATE | COMPRESSION_DEFLATE | COMPRESSION_LZMA => tag == TAG_PREDICTOR,
            COMPRESSION_JPEG => tag == TAG_JPEGTABLES,
            COMPRESSION_OJPEG => matches!(tag, TAG_JPEGIFOFFSET | TAG_JPEGIFBYTECOUNT | TAG_JPEGQTABLES | TAG_JPEGDCTABLES | TAG_JPEGACTABLES | TAG_JPEGPROC | TAG_JPEGRESTARTINTERVAL),
            COMPRESSION_CCITTRLE | COMPRESSION_CCITTRLEW | COMPRESSION_CCITTFAX3 | COMPRESSION_CCITTFAX4 => match tag {
                TAG_BADFAXLINES | TAG_CLEANFAXDATA | TAG_CONSECUTIVEBADFAXLINES => true,
                TAG_GROUP3OPTIONS => compression == COMPRESSION_CCITTFAX3,
                TAG_GROUP4OPTIONS => compression == COMPRESSION_CCITTFAX4,
                _ => false,
            },
            _ => false,
        }
    }

    /// `TIFFReadDirectory` on the directory at `offset`.
    fn read_directory(&mut self, offset: u64) -> Result<()> {
        let mut entries = self.fetch_directory(offset)?;
        // Later duplicates of a tag are ignored.
        for first in 0..entries.len() {
            for later in first + 1..entries.len() {
                if entries[first].tag == entries[later].tag {
                    entries[later].ignore = true;
                }
            }
        }
        self.dir = Directory::new();

        if let Some(entry) = entries.iter_mut().find(|entry| entry.tag == TAG_SAMPLESPERPIXEL && !entry.ignore) {
            let value = self.entry_unsigned(entry, u16::MAX as u64).map_err(|error| entry_error(error, "SamplesPerPixel"))?;
            if value == 0 {
                bail!("Bad value 0 for \"SamplesPerPixel\" tag");
            }
            self.dir.samples_per_pixel = value as u16;
            entry.ignore = true;
        }
        if let Some(entry) = entries.iter_mut().find(|entry| entry.tag == TAG_COMPRESSION && !entry.ignore) {
            let value = self.entry_short_or_per_sample(entry).map_err(|error| entry_error(error, "Compression"))?;
            self.dir.compression = value;
            entry.ignore = true;
        }

        // The first pass: the tags that size everything else.
        let mut offsets_set = false;
        let mut counts_set = false;
        let mut dimensions_set = false;
        let mut tile_dimensions_set = false;
        for slot in entries.iter_mut() {
            if slot.ignore {
                continue;
            }
            let entry = *slot;
            match entry.tag {
                TAG_STRIPOFFSETS | TAG_TILEOFFSETS => offsets_set = true,
                TAG_STRIPBYTECOUNTS | TAG_TILEBYTECOUNTS => counts_set = true,
                TAG_IMAGEWIDTH | TAG_IMAGELENGTH | TAG_IMAGEDEPTH | TAG_TILEWIDTH | TAG_TILELENGTH | TAG_TILEDEPTH | TAG_PLANARCONFIG | TAG_ROWSPERSTRIP | TAG_EXTRASAMPLES => {
                    self.first_pass_tag(&entry)?;
                    match entry.tag {
                        TAG_IMAGEWIDTH | TAG_IMAGELENGTH => dimensions_set = true,
                        TAG_TILEWIDTH | TAG_TILELENGTH => tile_dimensions_set = true,
                        _ => {}
                    }
                    slot.ignore = true;
                }
                tag => {
                    if !self.tag_valid_for_codec(tag) {
                        slot.ignore = true;
                    }
                }
            }
        }
        if self.dir.compression == COMPRESSION_OJPEG && self.dir.planar_config == PLANARCONFIG_SEPARATE {
            let offsets_one = entries.iter().find(|entry| entry.tag == TAG_STRIPOFFSETS).is_some_and(|entry| entry.count == 1);
            let counts_one = entries.iter().find(|entry| entry.tag == TAG_STRIPBYTECOUNTS).is_some_and(|entry| entry.count == 1);
            if offsets_one && counts_one {
                self.dir.planar_config = PLANARCONFIG_CONTIG;
            }
        }
        if !dimensions_set {
            return Err(missing_required("ImageLength"));
        }

        // The second pass.
        let mut bits_per_sample_read = false;
        let mut bits_per_sample_set = false;
        let mut max_sample_value_set = false;
        let mut offsets_entry = None;
        let mut counts_entry = None;
        for entry in entries.iter().filter(|entry| !entry.ignore) {
            match entry.tag {
                TAG_MINSAMPLEVALUE | TAG_MAXSAMPLEVALUE | TAG_BITSPERSAMPLE | TAG_DATATYPE | TAG_SAMPLEFORMAT => {
                    let name = match entry.tag {
                        TAG_MINSAMPLEVALUE => "MinSampleValue",
                        TAG_MAXSAMPLEVALUE => "MaxSampleValue",
                        TAG_BITSPERSAMPLE => "BitsPerSample",
                        TAG_DATATYPE => "DataType",
                        _ => "SampleFormat",
                    };
                    let value = self.entry_short_or_per_sample(entry).map_err(|error| entry_error(error, name))?;
                    match entry.tag {
                        TAG_BITSPERSAMPLE => {
                            self.dir.bits_per_sample = value;
                            bits_per_sample_read = true;
                            bits_per_sample_set = true;
                        }
                        TAG_MAXSAMPLEVALUE => {
                            self.dir.max_sample_value = value;
                            max_sample_value_set = true;
                        }
                        TAG_MINSAMPLEVALUE => {}
                        TAG_DATATYPE => {
                            self.dir.sample_format = match value {
                                0 => 4,
                                1 => SAMPLEFORMAT_INT,
                                2 => SAMPLEFORMAT_UINT,
                                3 => SAMPLEFORMAT_IEEEFP,
                                _ => bail!("Bad value {value} for \"DataType\" tag"),
                            };
                        }
                        _ => {
                            if !(SAMPLEFORMAT_UINT..=6).contains(&value) {
                                bail!("Bad value {value} for \"SampleFormat\" tag");
                            }
                            self.dir.sample_format = value;
                        }
                    }
                }
                TAG_SMINSAMPLEVALUE | TAG_SMAXSAMPLEVALUE => {
                    if entry.count != self.dir.samples_per_pixel as u64 {
                        return Err(entry_error(EntryError::Count, "SMinSampleValue"));
                    }
                    self.entry_floats(entry).map_err(|error| entry_error(error, "SMinSampleValue"))?;
                }
                TAG_STRIPOFFSETS | TAG_TILEOFFSETS => offsets_entry = Some(*entry),
                TAG_STRIPBYTECOUNTS | TAG_TILEBYTECOUNTS => counts_entry = Some(*entry),
                TAG_COLORMAP | TAG_TRANSFERFUNCTION => {
                    if !bits_per_sample_read || self.dir.bits_per_sample > 24 {
                        continue;
                    }
                    let per_sample = 1u64 << self.dir.bits_per_sample;
                    let transfer_single = entry.tag == TAG_TRANSFERFUNCTION && entry.count == per_sample;
                    let required = if transfer_single { per_sample } else { 3 * per_sample };
                    if entry.count != required {
                        continue;
                    }
                    let Ok(values) = self.entry_unsigned_array(entry, u16::MAX as u64, u64::MAX) else {
                        continue;
                    };
                    if entry.tag == TAG_COLORMAP {
                        let count = per_sample as usize;
                        let red = values[..count].iter().map(|&value| value as u16).collect();
                        let green = values[count..2 * count].iter().map(|&value| value as u16).collect();
                        let blue = values[2 * count..].iter().map(|&value| value as u16).collect();
                        self.dir.colormap = Some([red, green, blue]);
                    }
                }
                TAG_OSUBFILETYPE => {}
                _ => self.normal_tag(entry)?,
            }
        }

        let compression = self.dir.compression;
        if compression == COMPRESSION_OJPEG {
            match self.dir.photometric {
                None => self.dir.photometric = Some(PHOTOMETRIC_YCBCR),
                Some(PHOTOMETRIC_RGB) => self.dir.photometric = Some(PHOTOMETRIC_YCBCR),
                _ => {}
            }
            if !bits_per_sample_set {
                self.dir.bits_per_sample = 8;
            }
            let samples_set = entries.iter().any(|entry| entry.tag == TAG_SAMPLESPERPIXEL);
            if !samples_set {
                match self.dir.photometric {
                    Some(PHOTOMETRIC_RGB) | Some(PHOTOMETRIC_YCBCR) => self.dir.samples_per_pixel = 3,
                    Some(PHOTOMETRIC_MINISWHITE) | Some(PHOTOMETRIC_MINISBLACK) => self.dir.samples_per_pixel = 1,
                    _ => {}
                }
            }
        }

        // Strips or tiles.
        if !tile_dimensions_set {
            self.dir.strip_count = self.number_of_strips();
            self.dir.tile_width = self.dir.width;
            self.dir.tile_length = self.dir.rows_per_strip;
            self.dir.tile_depth = self.dir.image_depth;
            self.dir.tiled = false;
        } else {
            self.dir.strip_count = self.number_of_tiles();
            self.dir.tiled = true;
        }
        if self.dir.strip_count == 0 {
            bail!("Cannot handle zero number of {}", if self.dir.tiled { "tiles" } else { "strips" });
        }
        self.dir.strips_per_image = self.dir.strip_count;
        if self.dir.planar_config == PLANARCONFIG_SEPARATE {
            self.dir.strips_per_image /= self.dir.samples_per_pixel as u32;
        }
        if !offsets_set {
            let tolerated = compression == COMPRESSION_OJPEG && !self.dir.tiled && self.dir.strip_count == 1;
            if !tolerated {
                return Err(missing_required(if self.dir.tiled { "TileOffsets" } else { "StripOffsets" }));
            }
        }
        let strip_count = self.dir.strip_count as u64;
        if let Some(entry) = offsets_entry {
            self.dir.strip_offsets = self.fetch_strip_thing(&entry, strip_count, "StripOffsets")?;
        }
        if let Some(entry) = counts_entry {
            self.dir.strip_byte_counts = self.fetch_strip_thing(&entry, strip_count, "StripByteCounts")?;
        }
        if self.dir.strip_offsets.len() < strip_count as usize {
            self.dir.strip_offsets.resize(strip_count as usize, 0);
        }

        // Samples beyond the photometric's colours are extra samples.
        // A missing photometric reads as 0, min-is-white.
        let colour_channels = max_colour_channels(self.dir.photometric.unwrap_or(PHOTOMETRIC_MINISWHITE));
        let samples = self.dir.samples_per_pixel;
        if colour_channels != 0 && samples as i32 - self.dir.extra_samples.len() as i32 > colour_channels {
            let count = (samples as i32 - colour_channels) as usize;
            self.dir.extra_samples.resize(count, EXTRASAMPLE_UNSPECIFIED);
        }

        // A palette image needs its colormap.
        if self.dir.photometric == Some(PHOTOMETRIC_PALETTE) && self.dir.colormap.is_none() {
            if self.dir.bits_per_sample >= 8 && samples == 3 {
                self.dir.photometric = Some(PHOTOMETRIC_RGB);
            } else if self.dir.bits_per_sample >= 8 {
                self.dir.photometric = Some(PHOTOMETRIC_MINISBLACK);
            } else {
                return Err(missing_required("Colormap"));
            }
        }

        if compression != COMPRESSION_OJPEG {
            if !counts_set {
                let contig_many = self.dir.planar_config == PLANARCONFIG_CONTIG && self.dir.strip_count > 1;
                let separate_wrong = self.dir.planar_config == PLANARCONFIG_SEPARATE && self.dir.strip_count != samples as u32;
                if contig_many || separate_wrong {
                    return Err(missing_required("StripByteCounts"));
                }
                self.estimate_strip_byte_counts(&entries)?;
            } else if self.dir.strip_count == 1 && !self.dir.tiled && self.byte_count_looks_bad() {
                self.estimate_strip_byte_counts(&entries)?;
            } else if self.dir.planar_config == PLANARCONFIG_CONTIG && self.dir.strip_count > 2 && compression == COMPRESSION_NONE {
                let first = self.dir.strip_byte_counts.first().copied().unwrap_or(0);
                let second = self.dir.strip_byte_counts.get(1).copied().unwrap_or(0);
                if first != second && first != 0 && second != 0 {
                    self.estimate_strip_byte_counts(&entries)?;
                }
            }
        }
        if self.dir.strip_byte_counts.len() < strip_count as usize {
            self.dir.strip_byte_counts.resize(strip_count as usize, 0);
        }
        if !max_sample_value_set {
            self.dir.max_sample_value = if self.dir.bits_per_sample >= 16 { 0xFFFF } else { ((1u32 << self.dir.bits_per_sample) - 1) as u16 };
        }

        self.fixup_tags()?;

        if self.dir.planar_config == PLANARCONFIG_CONTIG && self.dir.strip_count == 1 && compression == COMPRESSION_NONE && !self.dir.tiled {
            self.chop_up_single_uncompressed_strip();
        }

        if self.scanline_size()? == 0 {
            bail!("Cannot handle zero scanline size");
        }
        if self.dir.tiled {
            if self.tile_size()? == 0 {
                bail!("Cannot handle zero tile size");
            }
        } else if self.strip_size()? == 0 {
            bail!("Cannot handle zero strip size");
        }
        Ok(())
    }

    /// The first pass's tags (`TIFFFetchNormalTag` without recovery and
    /// the checks of `_TIFFVSetField`).
    fn first_pass_tag(&mut self, entry: &Entry) -> Result<()> {
        match entry.tag {
            TAG_IMAGEWIDTH | TAG_IMAGELENGTH | TAG_IMAGEDEPTH | TAG_TILEWIDTH | TAG_TILELENGTH | TAG_TILEDEPTH | TAG_ROWSPERSTRIP => {
                let name = match entry.tag {
                    TAG_IMAGEWIDTH => "ImageWidth",
                    TAG_IMAGELENGTH => "ImageLength",
                    TAG_IMAGEDEPTH => "ImageDepth",
                    TAG_TILEWIDTH => "TileWidth",
                    TAG_TILELENGTH => "TileLength",
                    TAG_TILEDEPTH => "TileDepth",
                    _ => "RowsPerStrip",
                };
                let value = self.entry_unsigned(entry, u32::MAX as u64).map_err(|error| entry_error(error, name))? as u32;
                match entry.tag {
                    TAG_IMAGEWIDTH => self.dir.width = value,
                    TAG_IMAGELENGTH => self.dir.length = value,
                    TAG_IMAGEDEPTH => self.dir.image_depth = value,
                    TAG_TILEWIDTH => self.dir.tile_width = value,
                    TAG_TILELENGTH => self.dir.tile_length = value,
                    TAG_TILEDEPTH => {
                        if value == 0 {
                            bail!("Bad value 0 for \"TileDepth\" tag");
                        }
                        self.dir.tile_depth = value;
                    }
                    _ => {
                        if value == 0 {
                            bail!("Bad value 0 for \"RowsPerStrip\" tag");
                        }
                        self.dir.rows_per_strip = value;
                        self.dir.rows_per_strip_set = true;
                    }
                }
            }
            TAG_PLANARCONFIG => {
                let value = self.entry_unsigned(entry, u16::MAX as u64).map_err(|error| entry_error(error, "PlanarConfiguration"))? as u16;
                if value != PLANARCONFIG_CONTIG && value != PLANARCONFIG_SEPARATE {
                    bail!("Bad value {value} for \"PlanarConfiguration\" tag");
                }
                self.dir.planar_config = value;
            }
            _ => {
                // ExtraSamples (`setExtraSamples`).
                let values = self.entry_unsigned_array(entry, u16::MAX as u64, u64::MAX).map_err(|error| entry_error(error, "ExtraSamples"))?;
                if values.len() > self.dir.samples_per_pixel as usize {
                    bail!("Bad value {} for \"ExtraSamples\" tag", values.len());
                }
                let mut samples = Vec::with_capacity(values.len());
                for value in values {
                    let value = match value {
                        0..=2 => value as u16,
                        999 => EXTRASAMPLE_UNASSALPHA,
                        _ => bail!("Bad value for \"ExtraSamples\" tag"),
                    };
                    samples.push(value);
                }
                self.dir.extra_samples = samples;
            }
        }
        Ok(())
    }

    /// `TIFFFetchNormalTag` with recovery: a bad entry is skipped, but a
    /// value `_TIFFVSetField` refuses is an error.
    fn normal_tag(&mut self, entry: &Entry) -> Result<()> {
        let short = |tiff: &Self| tiff.entry_unsigned(entry, u16::MAX as u64).map(|value| value as u16);
        match entry.tag {
            TAG_PHOTOMETRIC => {
                if let Ok(value) = short(self) {
                    self.dir.photometric = Some(value);
                }
            }
            TAG_FILLORDER => {
                if let Ok(value) = short(self) {
                    if value != FILLORDER_LSB2MSB && value != FILLORDER_MSB2LSB {
                        bail!("Bad value {value} for \"FillOrder\" tag");
                    }
                    self.dir.fill_order = value;
                }
            }
            TAG_ORIENTATION => {
                if let Ok(value) = short(self) {
                    if !(1..=8).contains(&value) {
                        bail!("Bad value {value} for \"Orientation\" tag");
                    }
                    self.dir.orientation = value;
                }
            }
            TAG_RESOLUTIONUNIT => {
                if let Ok(value) = short(self)
                    && !(1..=3).contains(&value)
                {
                    bail!("Bad value {value} for \"ResolutionUnit\" tag");
                }
            }
            TAG_XRESOLUTION | TAG_YRESOLUTION => {
                if let Ok(value) = self.entry_double(entry) {
                    let as_float = value as f32 as f64;
                    if as_float.is_nan() || as_float < 0.0 {
                        bail!("Bad value {value} for resolution tag");
                    }
                }
            }
            TAG_INKSET => {
                if let Ok(value) = short(self) {
                    self.dir.ink_set = value;
                }
            }
            TAG_PREDICTOR => {
                if let Ok(value) = short(self) {
                    self.dir.predictor = value;
                }
            }
            TAG_YCBCRSUBSAMPLING => {
                if entry.count == 2
                    && let Ok(values) = self.entry_unsigned_array(entry, u16::MAX as u64, u64::MAX)
                {
                    self.dir.ycbcr_subsampling = [values[0] as u16, values[1] as u16];
                    self.dir.ycbcr_subsampling_set = true;
                }
            }
            TAG_YCBCRPOSITIONING => {
                if let Ok(value) = short(self) {
                    self.dir.ycbcr_positioning = value;
                }
            }
            TAG_YCBCRCOEFFICIENTS => {
                if entry.count == 3
                    && let Ok(values) = self.entry_floats(entry)
                {
                    self.dir.ycbcr_coefficients = Some([values[0], values[1], values[2]]);
                }
            }
            TAG_REFERENCEBLACKWHITE => {
                if entry.count == 6
                    && let Ok(values) = self.entry_floats(entry)
                {
                    let mut array = [0f32; 6];
                    array.copy_from_slice(&values[..6]);
                    self.dir.reference_black_white = Some(array);
                }
            }
            TAG_WHITEPOINT => {
                if entry.count == 2
                    && let Ok(values) = self.entry_floats(entry)
                {
                    self.dir.white_point = Some([values[0], values[1]]);
                }
            }
            TAG_JPEGTABLES => {
                let allowed = matches!(entry.field_type, TYPE_ASCII | TYPE_UNDEFINED | TYPE_BYTE | TYPE_SBYTE);
                if allowed
                    && entry.count != 0
                    && let Ok(bytes) = self.entry_bytes(entry, entry.count)
                {
                    self.dir.jpeg_tables = Some(bytes.to_vec());
                }
            }
            TAG_GROUP3OPTIONS | TAG_GROUP4OPTIONS => {
                if let Ok(value) = self.entry_unsigned(entry, u32::MAX as u64) {
                    if entry.tag == TAG_GROUP3OPTIONS {
                        self.dir.group3_options = value as u32;
                    } else {
                        self.dir.group4_options = value as u32;
                    }
                }
            }
            TAG_JPEGPROC => {
                if let Ok(value) = short(self) {
                    self.dir.old_jpeg.proc = Some(value);
                }
            }
            TAG_JPEGRESTARTINTERVAL => {
                if let Ok(value) = short(self) {
                    self.dir.old_jpeg.restart_interval = Some(value);
                }
            }
            TAG_JPEGIFOFFSET | TAG_JPEGIFBYTECOUNT => {
                if let Ok(value) = self.entry_unsigned(entry, u64::MAX) {
                    if entry.tag == TAG_JPEGIFOFFSET {
                        self.dir.old_jpeg.interchange_offset = Some(value);
                    } else {
                        self.dir.old_jpeg.interchange_length = Some(value);
                    }
                }
            }
            TAG_JPEGQTABLES | TAG_JPEGDCTABLES | TAG_JPEGACTABLES => {
                if let Ok(values) = self.entry_unsigned_array(entry, u64::MAX, u64::MAX) {
                    match entry.tag {
                        TAG_JPEGQTABLES => self.dir.old_jpeg.q_tables = values,
                        TAG_JPEGDCTABLES => self.dir.old_jpeg.dc_tables = values,
                        _ => self.dir.old_jpeg.ac_tables = values,
                    }
                }
            }
            TAG_MATTEING => {
                if let Ok(value) = short(self) {
                    self.dir.extra_samples = if value != 0 { vec![EXTRASAMPLE_ASSOCALPHA] } else { Vec::new() };
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// `TIFFFetchStripThing`: an offsets or byte counts array, cut or
    /// padded with zeros to the strip count.
    fn fetch_strip_thing(&self, entry: &Entry, strip_count: u64, name: &str) -> Result<Vec<u64>> {
        let mut values = self.entry_unsigned_array(entry, u64::MAX, strip_count).map_err(|error| entry_error(error, name))?;
        if (values.len() as u64) < strip_count {
            if strip_count > 1_000_000 {
                return Err(entry_error(EntryError::Count, name));
            }
            values.resize(strip_count as usize, 0);
        }
        Ok(values)
    }

    /// `TIFFNumberOfStrips`.
    fn number_of_strips(&self) -> u32 {
        let dir = &self.dir;
        let strips = if dir.rows_per_strip == u32::MAX { 1 } else { how_many(dir.length, dir.rows_per_strip) };
        if dir.planar_config == PLANARCONFIG_SEPARATE {
            return strips.saturating_mul(dir.samples_per_pixel as u32);
        }
        strips
    }

    /// `TIFFNumberOfTiles`.
    fn number_of_tiles(&self) -> u32 {
        let dir = &self.dir;
        let across = if dir.tile_width == u32::MAX { dir.width } else { dir.tile_width };
        let down = if dir.tile_length == u32::MAX { dir.length } else { dir.tile_length };
        let deep = if dir.tile_depth == u32::MAX { dir.image_depth } else { dir.tile_depth };
        if across == 0 || down == 0 || deep == 0 {
            return 0;
        }
        let per_plane = (how_many(dir.width, across) as u64) * (how_many(dir.length, down) as u64) * (how_many(dir.image_depth, deep) as u64);
        let tiles = if dir.planar_config == PLANARCONFIG_SEPARATE { per_plane * dir.samples_per_pixel as u64 } else { per_plane };
        if tiles > u32::MAX as u64 {
            return 0;
        }
        tiles as u32
    }

    /// `ByteCountLooksBad` for a single strip.
    fn byte_count_looks_bad(&self) -> bool {
        let byte_count = self.dir.strip_byte_counts.first().copied().unwrap_or(0);
        let offset = self.dir.strip_offsets.first().copied().unwrap_or(0);
        if offset == 0 {
            return false;
        }
        if byte_count == 0 {
            return true;
        }
        if self.dir.compression != COMPRESSION_NONE {
            return false;
        }
        let file_size = self.data.len() as u64;
        if offset <= file_size && byte_count > file_size - offset {
            return true;
        }
        let scanline = self.scanline_size().unwrap_or(0);
        let length = self.dir.length as u64;
        if length > 0 && scanline > u64::MAX / length {
            return true;
        }
        byte_count < scanline * length
    }

    /// `EstimateStripByteCounts`.
    fn estimate_strip_byte_counts(&mut self, entries: &[Entry]) -> Result<()> {
        let strip_count = self.dir.strip_count as usize;
        let mut counts = vec![0u64; strip_count];
        if self.dir.compression != COMPRESSION_NONE {
            let header = if self.bigtiff { 16 } else { 8 };
            let mut space: u64 = if self.bigtiff { header + 8 + entries.len() as u64 * 20 + 8 } else { header + 2 + entries.len() as u64 * 12 + 4 };
            for entry in entries {
                let width = data_width(entry.field_type);
                if width == 0 {
                    bail!("Cannot determine size of unknown tag type {}", entry.field_type);
                }
                let size = width.saturating_mul(entry.count);
                let inline = if self.bigtiff { 8 } else { 4 };
                if size > inline {
                    space = space.saturating_add(size);
                }
            }
            let file_size = self.data.len() as u64;
            space = if file_size < space { file_size } else { file_size - space };
            if self.dir.planar_config == PLANARCONFIG_SEPARATE {
                space /= self.dir.samples_per_pixel as u64;
            }
            counts.fill(space);
            let last = strip_count - 1;
            let last_offset = self.dir.strip_offsets[last];
            if last_offset.saturating_add(counts[last]) > file_size {
                counts[last] = file_size.saturating_sub(last_offset);
            }
        } else if self.dir.tiled {
            let tile = self.tile_size()?;
            counts.fill(tile);
        } else {
            let row_bytes = self.scanline_size()?;
            let rows = self.dir.length / self.dir.strips_per_image.max(1);
            counts.fill(row_bytes * rows as u64);
        }
        self.dir.strip_byte_counts = counts;
        if !self.dir.rows_per_strip_set {
            self.dir.rows_per_strip = self.dir.length;
        }
        Ok(())
    }

    /// `ChopUpSingleUncompressedStrip`: one big uncompressed strip read as
    /// strips of about 8 KiB, which sets RowsPerStrip.
    fn chop_up_single_uncompressed_strip(&mut self) {
        let row_block = if self.dir.photometric == Some(PHOTOMETRIC_YCBCR) && !self.upsampled() { self.dir.ycbcr_subsampling[1] as u32 } else { 1 };
        let Ok(row_block_bytes) = self.vtile_size(row_block) else {
            return;
        };
        let (strip_bytes, rows_per_strip) = if row_block_bytes > STRIP_SIZE_DEFAULT {
            (row_block_bytes, row_block)
        } else if let Some(blocks) = STRIP_SIZE_DEFAULT.checked_div(row_block_bytes) {
            let blocks = blocks as u32;
            (blocks as u64 * row_block_bytes, blocks.wrapping_mul(row_block))
        } else {
            return;
        };
        if rows_per_strip >= self.dir.rows_per_strip || rows_per_strip == 0 {
            return;
        }
        let strip_count = how_many(self.dir.length, rows_per_strip);
        if strip_count == 0 {
            return;
        }
        let first_offset = self.dir.strip_offsets[0];
        let last_offset = self.dir.strip_offsets[self.dir.strip_count as usize - 1];
        let last_count = self.dir.strip_byte_counts[self.dir.strip_count as usize - 1];
        if last_offset > u64::MAX - last_count || last_offset + last_count < first_offset {
            return;
        }
        let mut remaining = last_offset + last_count - first_offset;
        let mut offset = first_offset;
        let mut offsets = Vec::with_capacity(strip_count as usize);
        let mut counts = Vec::with_capacity(strip_count as usize);
        let mut size = strip_bytes;
        for _ in 0..strip_count {
            if size > remaining {
                size = remaining;
            }
            counts.push(size);
            offsets.push(if size != 0 { offset } else { 0 });
            offset += size;
            remaining -= size;
        }
        self.dir.strip_count = strip_count;
        self.dir.strips_per_image = strip_count;
        self.dir.rows_per_strip = rows_per_strip;
        self.dir.rows_per_strip_set = true;
        if !self.dir.tiled {
            self.dir.tile_length = rows_per_strip;
            self.dir.tile_width = self.dir.width;
        }
        self.dir.strip_offsets = offsets;
        self.dir.strip_byte_counts = counts;
    }

    /// The codecs' `tif_fixuptags`: the JPEG codec takes a missing
    /// YCbCr subsampling from the JPEG stream; unconfigured codecs fail.
    fn fixup_tags(&mut self) -> Result<()> {
        let compression = self.dir.compression;
        if compression != COMPRESSION_NONE && !codec_configured(compression) && known_codec(compression) {
            bail!("{} compression support is not configured", codec_name(compression));
        }
        if compression == COMPRESSION_JPEG {
            super::jpeg::fixup_subsampling(self);
        }
        Ok(())
    }

    /// Whether decoded YCbCr data is upsampled (`isUpSampled`).
    pub fn upsampled(&self) -> bool {
        self.dir.compression == COMPRESSION_JPEG && self.dir.planar_config == PLANARCONFIG_CONTIG && self.dir.photometric == Some(PHOTOMETRIC_YCBCR) && self.jpeg_color_mode_rgb
    }

    /// Packed YCbCr: one Cb and Cr for each subsampling block of Y.
    fn packed_ycbcr(&self) -> bool {
        self.dir.planar_config == PLANARCONFIG_CONTIG && self.dir.photometric == Some(PHOTOMETRIC_YCBCR) && !self.upsampled()
    }

    fn checked_subsampling(&self) -> Result<[u32; 2]> {
        let [horizontal, vertical] = self.dir.ycbcr_subsampling;
        let valid = |value: u16| matches!(value, 1 | 2 | 4);
        if !valid(horizontal) || !valid(vertical) {
            bail!("Invalid YCbCr subsampling ({horizontal}x{vertical})");
        }
        Ok([horizontal as u32, vertical as u32])
    }

    /// `TIFFScanlineSize64`.
    pub fn scanline_size(&self) -> Result<u64> {
        let dir = &self.dir;
        let size = if dir.planar_config == PLANARCONFIG_CONTIG {
            if dir.photometric == Some(PHOTOMETRIC_YCBCR) && dir.samples_per_pixel == 3 && !self.upsampled() {
                let [horizontal, vertical] = self.checked_subsampling()?;
                let block_samples = (horizontal * vertical + 2) as u64;
                let blocks = how_many(dir.width, horizontal) as u64;
                let row_size = (blocks * block_samples * dir.bits_per_sample as u64).div_ceil(8);
                row_size / vertical as u64
            } else {
                let samples = dir.width as u64 * dir.samples_per_pixel as u64;
                (samples * dir.bits_per_sample as u64).div_ceil(8)
            }
        } else {
            (dir.width as u64 * dir.bits_per_sample as u64).div_ceil(8)
        };
        if size == 0 {
            bail!("Computed scanline size is zero");
        }
        Ok(size)
    }

    /// `TIFFVStripSize64`.
    pub fn vstrip_size(&self, rows: u32) -> Result<u64> {
        let dir = &self.dir;
        let rows = if rows == u32::MAX { dir.length } else { rows };
        if self.packed_ycbcr() {
            if dir.samples_per_pixel != 3 {
                bail!("Invalid td_samplesperpixel value");
            }
            let [horizontal, vertical] = self.checked_subsampling()?;
            let block_samples = (horizontal * vertical + 2) as u64;
            let blocks_across = how_many(dir.width, horizontal) as u64;
            let blocks_down = how_many(rows, vertical) as u64;
            let row_size = bytes_for_bits(blocks_across * block_samples * dir.bits_per_sample as u64);
            return Ok(row_size * blocks_down);
        }
        Ok(rows as u64 * self.scanline_size()?)
    }

    /// `TIFFStripSize64`.
    pub fn strip_size(&self) -> Result<u64> {
        let rows = self.dir.rows_per_strip.min(self.dir.length);
        self.vstrip_size(rows)
    }

    /// `TIFFTileRowSize64`.
    pub fn tile_row_size(&self) -> Result<u64> {
        let dir = &self.dir;
        if dir.tile_length == 0 {
            bail!("Tile length is zero");
        }
        if dir.tile_width == 0 {
            bail!("Tile width is zero");
        }
        let mut row_bits = dir.bits_per_sample as u64 * dir.tile_width as u64;
        if dir.planar_config == PLANARCONFIG_CONTIG {
            row_bits *= dir.samples_per_pixel as u64;
        }
        let size = bytes_for_bits(row_bits);
        if size == 0 {
            bail!("Computed tile row size is zero");
        }
        Ok(size)
    }

    /// `TIFFVTileSize64`.
    pub fn vtile_size(&self, rows: u32) -> Result<u64> {
        let dir = &self.dir;
        if dir.tile_length == 0 || dir.tile_width == 0 || dir.tile_depth == 0 {
            return Ok(0);
        }
        if self.packed_ycbcr() && dir.samples_per_pixel == 3 {
            let [horizontal, vertical] = self.checked_subsampling()?;
            let block_samples = (horizontal * vertical + 2) as u64;
            let blocks_across = how_many(dir.tile_width, horizontal) as u64;
            let blocks_down = how_many(rows, vertical) as u64;
            let row_size = bytes_for_bits(blocks_across * block_samples * dir.bits_per_sample as u64);
            return Ok(row_size * blocks_down);
        }
        Ok(rows as u64 * self.tile_row_size()?)
    }

    /// `TIFFTileSize64`.
    pub fn tile_size(&self) -> Result<u64> {
        self.vtile_size(self.dir.tile_length)
    }
}

/// `_TIFFGetMaxColorChannels`.
fn max_colour_channels(photometric: u16) -> i32 {
    match photometric {
        PHOTOMETRIC_PALETTE | PHOTOMETRIC_MINISWHITE | PHOTOMETRIC_MINISBLACK => 1,
        PHOTOMETRIC_YCBCR | PHOTOMETRIC_RGB | PHOTOMETRIC_CIELAB | PHOTOMETRIC_LOGLUV | PHOTOMETRIC_ITULAB | PHOTOMETRIC_ICCLAB => 3,
        PHOTOMETRIC_SEPARATED | PHOTOMETRIC_MASK => 4,
        _ => 0,
    }
}

/// Schemes in libtiff's codec table (`_TIFFBuiltinCODECS`).
fn known_codec(compression: u16) -> bool {
    codec_configured(compression) || matches!(compression, 34661 | 32909 | 50000 | 34887)
}

fn codec_name(compression: u16) -> &'static str {
    match compression {
        34661 => "ISO JBIG",
        32909 => "PixarLog",
        50000 => "ZSTD",
        34887 => "LERC",
        _ => "unknown",
    }
}
