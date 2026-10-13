//! TIFF: 32-bit float through LibTiff.NET, 8- and 16-bit through
//! Magick.NET, whose ImageMagick uses libtiff 4. Both are uncompressed,
//! little-endian, contiguous samples; the files are laid out as those
//! libraries lay them out.

use std::fs;
use std::io;
use std::path::Path;

use nerthus_engine::Map;

use crate::colorspace::{Colorspace, QUANTUM_RANGE, QUANTUM_SCALE};
use crate::magick::{quantum_to_char, quantum_to_short, MagickImage};

const SHORT: u16 = 3;
const LONG: u16 = 4;
const RATIONAL: u16 = 5;
const ASCII: u16 = 2;

/// LibTiff.NET's rational for 96 (X and Y resolution).
const RESOLUTION_96: [u32; 2] = [1_610_612_736, 16_777_216];

/// libtiff's rationals for ImageMagick's sRGB chromaticity: red, green
/// and blue primaries, then the white point.
const PRIMARY_CHROMATICITIES: [u32; 12] = [5_368_709, 8_388_608, 11_072_963, 33_554_432, 5_033_165, 16_777_216, 5_033_165, 8_388_608, 5_033_165, 33_554_432, 16_106_127, 268_435_456];
const WHITE_POINT: [u32; 4] = [10_492_471, 33_554_432, 689_963, 2_097_152];

/// ImageMagick's `TIFFStripSizeDefault`.
const STRIP_SIZE_DEFAULT: usize = 1_048_576;

/// The sample depth of ImageMagick's 8- and 16-bit TIFFs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleDepth {
    Eight,
    Sixteen,
}

/// One directory entry: its value bytes, inline when they fit in four.
struct Entry {
    tag: u16,
    field_type: u16,
    count: u32,
    value: Vec<u8>,
}

impl Entry {
    fn shorts(tag: u16, values: &[u16]) -> Self {
        let mut value = Vec::with_capacity(values.len() * 2);
        for item in values {
            value.extend_from_slice(&item.to_le_bytes());
        }
        Self { tag, field_type: SHORT, count: values.len() as u32, value }
    }

    fn longs(tag: u16, values: &[u32]) -> Self {
        let mut value = Vec::with_capacity(values.len() * 4);
        for item in values {
            value.extend_from_slice(&item.to_le_bytes());
        }
        Self { tag, field_type: LONG, count: values.len() as u32, value }
    }

    /// SHORT when the value fits, LONG otherwise (as both libraries write
    /// sizes and strip rows).
    fn short_or_long(tag: u16, value: u32) -> Self {
        if value <= u16::MAX as u32 {
            return Self::shorts(tag, &[value as u16]);
        }
        Self::longs(tag, &[value])
    }

    fn rationals(tag: u16, values: &[u32]) -> Self {
        let mut entry = Self::longs(tag, values);
        entry.field_type = RATIONAL;
        entry.count = (values.len() / 2) as u32;
        entry
    }

    fn ascii(tag: u16, text: &str) -> Self {
        let mut value = text.as_bytes().to_vec();
        value.push(0);
        Self { tag, field_type: ASCII, count: value.len() as u32, value }
    }

    fn is_inline(&self) -> bool {
        self.value.len() <= 4
    }
}

/// Appends a directory (entries sorted by tag, then a zero next-IFD
/// offset) at `file.len()` and the out-of-line values after it, each at
/// an even offset, in `data_order` (the order the library writes them).
fn append_directory(file: &mut Vec<u8>, entries: &mut [Entry], data_order: &[u16]) {
    entries.sort_by_key(|entry| entry.tag);
    let directory_offset = file.len();
    let directory_size = 2 + entries.len() * 12 + 4;
    let mut data_offset = directory_offset + directory_size;
    let mut offsets = vec![0usize; entries.len()];
    for tag in data_order {
        let Some(index) = entries.iter().position(|entry| entry.tag == *tag) else {
            continue;
        };
        if entries[index].is_inline() {
            continue;
        }
        data_offset += data_offset & 1;
        offsets[index] = data_offset;
        data_offset += entries[index].value.len();
    }

    file.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (index, entry) in entries.iter().enumerate() {
        file.extend_from_slice(&entry.tag.to_le_bytes());
        file.extend_from_slice(&entry.field_type.to_le_bytes());
        file.extend_from_slice(&entry.count.to_le_bytes());
        if entry.is_inline() {
            let mut inline = [0u8; 4];
            inline[..entry.value.len()].copy_from_slice(&entry.value);
            file.extend_from_slice(&inline);
        } else {
            file.extend_from_slice(&(offsets[index] as u32).to_le_bytes());
        }
    }
    file.extend_from_slice(&0u32.to_le_bytes());

    for tag in data_order {
        let Some(index) = entries.iter().position(|entry| entry.tag == *tag) else {
            continue;
        };
        if entries[index].is_inline() {
            continue;
        }
        if file.len() & 1 == 1 {
            file.push(0);
        }
        file.extend_from_slice(&entries[index].value);
    }
}

fn set_directory_offset(file: &mut [u8], offset: usize) {
    file[4..8].copy_from_slice(&(offset as u32).to_le_bytes());
}

/// 32-bit float samples, interleaved, in one strip, as
/// LibTiff.NET writes them (the strip at offset 16, MINISBLACK even for
/// colour, X and Y resolution 96 per centimetre).
pub fn write_float_tiff(path: &Path, map: &Map, artist: &str) -> io::Result<()> {
    let bytes = float_tiff_bytes(map, artist);
    fs::write(path, bytes)
}

pub fn float_tiff_bytes(map: &Map, artist: &str) -> Vec<u8> {
    let width = map.width() as u32;
    let height = map.height() as u32;
    let channels = map.channel_count() as u16;
    let samples = map.to_interleaved();

    let mut file = Vec::with_capacity(16 + samples.len() * 4 + 512);
    file.extend_from_slice(b"II*\0");
    file.extend_from_slice(&[0u8; 12]);
    for value in &samples {
        file.extend_from_slice(&value.to_le_bytes());
    }
    let strip_size = (samples.len() * 4) as u32;

    let bits_per_sample = vec![32u16; channels as usize];
    let sample_format = vec![3u16; channels as usize];
    let mut entries = vec![
        Entry::short_or_long(256, width),
        Entry::short_or_long(257, height),
        Entry::shorts(258, &bits_per_sample),
        Entry::shorts(259, &[1]),
        Entry::shorts(262, &[1]),
        Entry::shorts(266, &[1]),
        Entry::longs(273, &[16]),
        Entry::shorts(274, &[1]),
        Entry::shorts(277, &[channels]),
        Entry::short_or_long(278, height),
        Entry::longs(279, &[strip_size]),
        Entry::rationals(282, &RESOLUTION_96),
        Entry::rationals(283, &RESOLUTION_96),
        Entry::shorts(284, &[1]),
        Entry::shorts(296, &[3]),
        Entry::ascii(315, artist),
        Entry::shorts(339, &sample_format),
    ];
    let directory_offset = file.len();
    let data_order = [258, 282, 283, 315, 339];
    append_directory(&mut file, &mut entries, &data_order);
    set_directory_offset(&mut file, directory_offset);
    file
}

/// 8-bit (after `SetBitDepth(8)`) and 16-bit TIFF: a colour map in the
/// build's `ColorSpace` (`color_space`), then ImageMagick's TIFF coder.
pub fn write_magick_tiff(path: &Path, map: &Map, depth: SampleDepth, color_space: Colorspace) -> io::Result<()> {
    let mut image = MagickImage::from_map(map);
    if depth == SampleDepth::Eight {
        image.set_depth_8();
    }
    if map.channel_count() > 1 {
        image.transform_colorspace(color_space);
    }
    let bytes = magick_tiff_bytes(image, depth);
    fs::write(path, bytes)
}

/// ImageMagick's TIFF coder, which writes every colourspace's values as
/// they are: CMYK as SEPARATED (four samples, CMYK inks), Lab as CIELAB
/// (a and b shifted to TIFF's signed form, `EncodeLabImage`), the YCbCr
/// spaces as YCbCr (8-bit, no subsampling), grey images (and images in an
/// sRGB-compatible colourspace whose pixels are all grey) as MINISBLACK,
/// the rest as RGB. Alpha is an unassociated extra sample; images whose
/// colourspace has chromaticity carry the sRGB primaries.
pub fn magick_tiff_bytes(mut image: MagickImage, depth: SampleDepth) -> Vec<u8> {
    let colorspace = image.colorspace;
    if colorspace == Colorspace::Lab {
        encode_lab(&mut image);
    }
    let gray = colorspace != Colorspace::Cmyk && image.is_gray();
    let alpha = image.has_alpha();
    let photometric: u16 = if colorspace == Colorspace::Cmyk {
        5
    } else if gray {
        1
    } else if colorspace == Colorspace::Lab {
        8
    } else if colorspace.is_ycbcr_compatible() {
        6
    } else {
        2
    };
    let color_samples = match photometric {
        1 => 1,
        5 => 4,
        _ => 3,
    };
    let samples_per_pixel = color_samples + usize::from(alpha);
    let depth = if photometric == 6 { SampleDepth::Eight } else { depth };
    let bits = match depth {
        SampleDepth::Eight => 8u16,
        SampleDepth::Sixteen => 16u16,
    };
    let bytes_per_sample = (bits / 8) as usize;
    let scanline = image.width * samples_per_pixel * bytes_per_sample;

    // `GetTIFFInfo`: strips of about a megabyte, in whole multiples of 16
    // rows, no more than the image.
    let mut rows_per_strip = STRIP_SIZE_DEFAULT / scanline.max(1);
    rows_per_strip = 16 * ((rows_per_strip.max(16) + 1) / 16);
    rows_per_strip = rows_per_strip.min(image.height).max(1);

    let mut file = Vec::with_capacity(8 + scanline * image.height + 512);
    file.extend_from_slice(b"II*\0");
    file.extend_from_slice(&[0u8; 4]);
    let mut strip_offsets = Vec::new();
    let mut strip_byte_counts = Vec::new();
    let mut row = 0;
    while row < image.height {
        let rows = rows_per_strip.min(image.height - row);
        strip_offsets.push(file.len() as u32);
        strip_byte_counts.push((rows * scanline) as u32);
        for y in row..row + rows {
            append_row(&mut file, &image, y, color_samples, depth);
        }
        row += rows;
    }
    if file.len() & 1 == 1 {
        file.push(0);
    }

    let bits_per_sample = vec![bits; samples_per_pixel];
    let mut entries = vec![
        Entry::short_or_long(256, image.width as u32),
        Entry::short_or_long(257, image.height as u32),
        Entry::shorts(258, &bits_per_sample),
        Entry::shorts(259, &[1]),
        Entry::shorts(262, &[photometric]),
        Entry::shorts(266, &[1]),
        Entry::longs(273, &strip_offsets),
        Entry::shorts(274, &[1]),
        Entry::shorts(277, &[samples_per_pixel as u16]),
        Entry::short_or_long(278, rows_per_strip as u32),
        Entry::longs(279, &strip_byte_counts),
        Entry::shorts(284, &[1]),
        Entry::shorts(297, &[0, 1]),
    ];
    if alpha {
        entries.push(Entry::shorts(338, &[2]));
    }
    if photometric == 5 {
        entries.push(Entry::shorts(332, &[1]));
    }
    if photometric == 6 {
        entries.push(Entry::shorts(530, &[1, 1]));
    }
    if colorspace.has_chromaticity() {
        entries.push(Entry::rationals(319, &PRIMARY_CHROMATICITIES));
        entries.push(Entry::rationals(318, &WHITE_POINT));
    }
    let directory_offset = file.len();
    // libtiff writes the standard fields' values in its own order, then
    // custom fields in the order they were set.
    let data_order = [258, 279, 273, 338, 319, 318];
    append_directory(&mut file, &mut entries, &data_order);
    set_directory_offset(&mut file, directory_offset);
    file
}

/// One scanline as `ExportQuantumPixels` makes it: grey (or grey and
/// alpha) for MINISBLACK, the first three channels for three samples,
/// all four of CMYK, then alpha.
fn append_row(file: &mut Vec<u8>, image: &MagickImage, y: usize, color_samples: usize, depth: SampleDepth) {
    for x in 0..image.width {
        let pixel = y * image.width + x;
        if color_samples == 1 {
            let [gray, _, _] = image.rgb(pixel);
            append_sample(file, gray, depth);
        } else {
            let start = pixel * image.channels;
            for sample in &image.quanta[start..start + color_samples] {
                append_sample(file, *sample, depth);
            }
        }
        if image.has_alpha() {
            append_sample(file, image.alpha(pixel), depth);
        }
    }
}

/// `EncodeLabImage`: a and b (stored around half range) as TIFF's signed
/// values, wrapped into the quantum range.
fn encode_lab(image: &mut MagickImage) {
    let channels = image.channels;
    for pixel in image.quanta.chunks_mut(channels) {
        for sample in &mut pixel[1..3] {
            let mut shifted = QUANTUM_SCALE * *sample as f64 - 0.5;
            if shifted < 0.0 {
                shifted += 1.0;
            }
            *sample = (QUANTUM_RANGE * shifted) as f32;
        }
    }
}

fn append_sample(file: &mut Vec<u8>, quantum: f32, depth: SampleDepth) {
    match depth {
        SampleDepth::Eight => file.push(quantum_to_char(quantum)),
        SampleDepth::Sixteen => file.extend_from_slice(&quantum_to_short(quantum).to_le_bytes()),
    }
}
