//! PNG as Magick.NET writes it: the PNG coder of
//! ImageMagick 7.1.2-19 over libpng 1.6. PNG16 and PNG64 both reach the
//! plain coder (the 64-bit case matches none of the format switch), which
//! picks the colour type and bit depth from the pixels:
//!
//! - 16-bit samples drop to 8 bits when no red, green or blue value loses
//!   anything (`LosslessReduceDepthOK`; alpha is not checked);
//! - grey pixels are written grey, even from a colour map;
//! - an image of 256 colours or fewer (counting the black background)
//!   becomes a palette at 8 bits, or stays grey with 1, 2 or 4 bits when
//!   its grey levels allow;
//! - libpng filters 8- and 16-bit non-palette rows adaptively.
//!
//! PNG8 (`MagickFormat.Png8`) quantizes first: `SetImageType` to a
//! palette type runs ImageMagick's octree quantizer (256 colours,
//! Riemersma dither). The coder then takes the samples to 8 bits and
//! writes every image as an 8-bit palette of its colours in the order
//! they first appear, without the background colour; alpha goes binary
//! when there is more transparency than one transparent colour.
//!
//! The chunks follow the coder (cHRM for colour images, bKGD, tIME and
//! the `date:` texts before the image data). The compressed stream comes
//! from flate2 rather than zlib, so its bytes differ while the image data
//! inside is the same.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use chrono::{DateTime, Datelike, Timelike, Utc};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use nerthus_engine::Map;

use crate::colorspace::Colorspace;
use crate::magick::{is_8_bit_exact, quantum_to_char, quantum_to_short, MagickImage, QUANTUM_RANGE};

const COLOR_TYPE_GRAY: u8 = 0;
const COLOR_TYPE_RGB: u8 = 2;
const COLOR_TYPE_PALETTE: u8 = 3;
const COLOR_TYPE_GRAY_ALPHA: u8 = 4;
const COLOR_TYPE_RGB_ALPHA: u8 = 6;

/// libpng's IDAT size as ImageMagick sets it (`png_set_compression_buffer_size`).
const IDAT_SIZE: usize = 32768;

/// ImageMagick's default PNG quality 75: zlib level 7.
const COMPRESSION_LEVEL: u32 = 7;

/// The most colours of each kind `BUILD_PALETTE` keeps.
const CENSUS_LIMIT: usize = 259;

/// The image background: Magick.NET's `MagickImage(color, …)` sets it to
/// the image's colour, black here.
const BACKGROUND: Color = Color {
    red: 0.0,
    green: 0.0,
    blue: 0.0,
    alpha: QUANTUM_RANGE as f64,
};

/// `PixelInfo` colours of the palette pass, in quanta.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Color {
    red: f64,
    green: f64,
    blue: f64,
    alpha: f64,
}

impl Color {
    fn from_pixel(image: &MagickImage, pixel: usize) -> Self {
        let [red, green, blue] = image.rgb(pixel);
        Self {
            red: red as f64,
            green: green as f64,
            blue: blue as f64,
            alpha: image.alpha(pixel) as f64,
        }
    }

    fn rgb_equals(&self, other: &Color) -> bool {
        self.red == other.red && self.green == other.green && self.blue == other.blue
    }

    /// The colour's PLTE entry: each sample `ScaleQuantumToChar`.
    fn palette_entry(&self) -> [u8; 3] {
        [quantum_to_char(self.red as f32), quantum_to_char(self.green as f32), quantum_to_char(self.blue as f32)]
    }

    /// `IsColorEqual`: red, green and blue within `MagickEpsilon`.
    fn rgb_close(&self, other: &Color) -> bool {
        let close = |a: f64, b: f64| (a - b).abs() < MAGICK_EPSILON;
        close(self.red, other.red) && close(self.green, other.green) && close(self.blue, other.blue)
    }

    /// The colour's samples as hash keys that agree with `==` (the
    /// samples are never NaN; -0 is keyed as 0).
    fn rgb_key(&self) -> [u64; 3] {
        [sample_key(self.red), sample_key(self.green), sample_key(self.blue)]
    }

    /// A key that every colour `rgb_close` to this one shares: samples
    /// from `TINY_SAMPLE` up exactly (floats there lie more than
    /// `MagickEpsilon` apart, so close means identical), smaller ones only
    /// as small.
    fn census_key(&self) -> [Option<u64>; 3] {
        let key = |value: f64| (value >= TINY_SAMPLE).then(|| sample_key(value));
        [key(self.red), key(self.green), key(self.blue)]
    }
}

fn sample_key(value: f64) -> u64 {
    (value + 0.0).to_bits()
}

/// `MagickEpsilon`.
const MAGICK_EPSILON: f64 = 1.0e-12;

/// 2^-15: below it, distinct float samples can lie within `MagickEpsilon`
/// of each other.
const TINY_SAMPLE: f64 = 1.0 / 32768.0;

/// One of `BUILD_PALETTE`'s colour lists: the colours in the order they
/// first appear, matched as the coder matches them (`IsColorEqual`, the
/// first entry that is close enough). Entries are bucketed by
/// `census_key`, outside which nothing is close.
#[derive(Default)]
struct ColorList {
    entries: Vec<Color>,
    buckets: HashMap<[Option<u64>; 3], Vec<usize>>,
}

impl ColorList {
    /// Adds `color` unless an entry matches it (with the same alpha too
    /// when `same_alpha`) or the list is full.
    fn add(&mut self, color: Color, same_alpha: bool) {
        let bucket = self.buckets.entry(color.census_key()).or_default();
        let matches = |index: &usize| {
            let entry = &self.entries[*index];
            entry.rgb_close(&color) && (!same_alpha || entry.alpha == color.alpha)
        };
        if bucket.iter().any(matches) || self.entries.len() >= CENSUS_LIMIT {
            return;
        }
        bucket.push(self.entries.len());
        self.entries.push(color);
    }
}

/// `BUILD_PALETTE`'s colour census: the distinct colours of the opaque,
/// fully transparent and partly transparent pixels in the order they
/// first appear, at most 259 of each (partly transparent ones told apart
/// by alpha too).
struct Census {
    opaque: Vec<Color>,
    transparent: Vec<Color>,
    semitransparent: Vec<Color>,
    /// The first transparent pixel's red, green, blue and grey as
    /// `unsigned short`s (`ping_trans_color`).
    transparent_color: [u16; 4],
}

impl Census {
    fn take(image: &MagickImage) -> Self {
        let has_alpha = image.has_alpha();
        let mut opaque = ColorList::default();
        let mut transparent = ColorList::default();
        let mut semitransparent = ColorList::default();
        let mut transparent_color = [0; 4];
        for pixel in 0..image.pixel_count() {
            let color = Color::from_pixel(image, pixel);
            if !has_alpha || color.alpha == QUANTUM_RANGE as f64 {
                let entry = Color {
                    alpha: QUANTUM_RANGE as f64,
                    ..color
                };
                opaque.add(entry, false);
            } else if color.alpha == 0.0 {
                if transparent.entries.is_empty() {
                    let [red, green, blue] = image.rgb(pixel);
                    transparent_color = [red as u16, green as u16, blue as u16, red as u16];
                }
                transparent.add(color, false);
            } else {
                semitransparent.add(color, true);
            }
        }
        Self {
            opaque: opaque.entries,
            transparent: transparent.entries,
            semitransparent: semitransparent.entries,
            transparent_color,
        }
    }
}

/// What the coder decided: the PNG's header fields and how rows are made.
#[derive(Clone, Debug)]
struct Plan {
    color_type: u8,
    bit_depth: u8,
    /// Each pixel's colormap entry, for palette rows.
    indexes: Vec<usize>,
    palette: Option<Vec<[u8; 3]>>,
    transparency: Option<Vec<u8>>,
    background: Option<Vec<u8>>,
    /// Rows hold black and white only, written through 8 bits (`> 127`).
    monochrome: bool,
    /// Samples are exported at this depth (8 or 16) before libpng packs
    /// sub-byte depths.
    sample_depth: u8,
}

/// PNG 16-bit and 64-bit: a colour map in the build's
/// `ColorSpace` for PNG 16-bit (`color_space`), as it is for 64-bit (none).
pub fn write_png(path: &Path, map: &Map, color_space: Option<Colorspace>) -> io::Result<()> {
    let mut image = MagickImage::from_map(map);
    if let Some(colorspace) = color_space
        && map.channel_count() > 1
    {
        image.transform_colorspace(colorspace);
    }
    let bytes = png_bytes(image, Utc::now());
    fs::write(path, bytes)
}

/// PNG 8-bit. `cache_shift` is the `CacheShift` of the ImageMagick whose
/// dithering is matched (2 for Windows builds); `color_space` the
/// build's `ColorSpace`.
pub fn write_png8(path: &Path, map: &Map, cache_shift: u32, color_space: &str) -> io::Result<()> {
    let image = MagickImage::from_map(map);
    let bytes = png8_bytes(image, cache_shift, color_space, Utc::now());
    fs::write(path, bytes)
}

/// The PNG file for a Magick.NET image, stamped with `time`.
/// `WriteOnePNGImage` writes an image in an sRGB-compatible colourspace as
/// it is and takes any other back to sRGB first.
pub fn png_bytes(image: MagickImage, time: DateTime<Utc>) -> Vec<u8> {
    let mut image = image;
    if !image.colorspace.is_srgb_compatible() {
        image.transform_colorspace(Colorspace::SRgb);
    }
    let reduced = lossless_reduce_ok(&image);
    let depth: u8 = if reduced { 8 } else { 16 };
    if depth == 8 {
        image.set_depth_8();
    }
    let plan = plan(&image, depth);
    encode(&image, &plan, time)
}

/// The PNG 8-bit file for a Magick.NET image, stamped with `time`.
pub fn png8_bytes(image: MagickImage, cache_shift: u32, color_space: &str, time: DateTime<Utc>) -> Vec<u8> {
    let mut image = image;
    // The PNG writer gives colour maps the build's colour space;
    // `SetImageType` takes one that is not sRGB-compatible back to sRGB
    // before quantizing.
    if image.channels > 1 {
        image.transform_colorspace(Colorspace::from_name(color_space));
    }
    image.set_palette_type(cache_shift);
    // `BUILD_PALETTE` starts by setting the image's depth (8) unless
    // `GetImageDepth` finds exactly that.
    if image.depth() != 8 {
        image.set_depth_8();
    }
    let plan = palette_plan(&mut image);
    encode(&image, &plan, time)
}

/// The file: signature, header, the chunks the plan asks for, the time
/// stamps and the compressed rows.
fn encode(image: &MagickImage, plan: &Plan, time: DateTime<Utc>) -> Vec<u8> {
    let rows = rows(image, plan);
    let filtered = filter_rows(&rows, plan);

    let mut file = Vec::new();
    file.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&(image.width as u32).to_be_bytes());
    header.extend_from_slice(&(image.height as u32).to_be_bytes());
    header.extend_from_slice(&[plan.bit_depth, plan.color_type, 0, 0, 0]);
    write_chunk(&mut file, b"IHDR", &header);
    if image.colorspace.has_chromaticity() {
        write_chunk(&mut file, b"cHRM", &srgb_chromaticity());
    }
    if let Some(palette) = &plan.palette {
        let bytes: Vec<u8> = palette.iter().flatten().copied().collect();
        write_chunk(&mut file, b"PLTE", &bytes);
    }
    if let Some(transparency) = &plan.transparency {
        write_chunk(&mut file, b"tRNS", transparency);
    }
    if let Some(background) = &plan.background {
        write_chunk(&mut file, b"bKGD", background);
    }
    write_chunk(&mut file, b"tIME", &time_chunk(time));
    let stamp = time.format("%Y-%m-%dT%H:%M:%S+00:00").to_string();
    for key in ["date:create", "date:modify", "date:timestamp"] {
        let mut text = key.as_bytes().to_vec();
        text.push(0);
        text.extend_from_slice(stamp.as_bytes());
        write_chunk(&mut file, b"tEXt", &text);
    }

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(COMPRESSION_LEVEL));
    encoder.write_all(&filtered).expect("compressing into memory");
    let compressed = encoder.finish().expect("compressing into memory");
    for part in compressed.chunks(IDAT_SIZE) {
        write_chunk(&mut file, b"IDAT", part);
    }
    write_chunk(&mut file, b"IEND", &[]);
    file
}

fn write_chunk(file: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    file.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = file.len();
    file.extend_from_slice(kind);
    file.extend_from_slice(data);
    let checksum = crc32fast::hash(&file[start..]);
    file.extend_from_slice(&checksum.to_be_bytes());
}

/// ImageMagick's sRGB chromaticity in cHRM units (100000ths): the white
/// point, then red, green and blue.
fn srgb_chromaticity() -> Vec<u8> {
    let values: [u32; 8] = [31270, 32900, 64000, 33000, 30000, 60000, 15000, 6000];
    values.iter().flat_map(|value| value.to_be_bytes()).collect()
}

fn time_chunk(time: DateTime<Utc>) -> Vec<u8> {
    let mut chunk = (time.year() as u16).to_be_bytes().to_vec();
    chunk.push(time.month() as u8);
    chunk.push(time.day() as u8);
    chunk.push(time.hour() as u8);
    chunk.push(time.minute() as u8);
    chunk.push(time.second() as u8);
    chunk
}

/// `LosslessReduceDepthOK`: every red, green and blue value (and the
/// black background) is a whole 8-bit level.
fn lossless_reduce_ok(image: &MagickImage) -> bool {
    (0..image.pixel_count()).all(|pixel| {
        let [red, green, blue] = image.rgb(pixel);
        is_8_bit_exact(red) && is_8_bit_exact(green) && is_8_bit_exact(blue)
    })
}

/// The palette pass (`BUILD_PALETTE`, once, as for any PNG but PNG8) and
/// the colour type and bit depth selection after it.
fn plan(image: &MagickImage, depth: u8) -> Plan {
    let has_alpha = image.has_alpha();
    let background = BACKGROUND;

    let census = Census::take(image);
    let mut opaque = census.opaque;
    let transparent = census.transparent;
    let semitransparent = census.semitransparent;
    let transparent_color = census.transparent_color;

    // The background joins the palette when there is room.
    let counted = opaque.len() + transparent.len() + semitransparent.len();
    if opaque.len() < CENSUS_LIMIT && !opaque.iter().any(|known| known.rgb_equals(&background)) && counted < 256 {
        opaque.push(background);
    }
    let image_colors = opaque.len() + transparent.len() + semitransparent.len();

    let mut have_color = false;
    let mut have_non_bw = false;
    if image_colors > 256 {
        scan_colors(image, &mut have_color, &mut have_non_bw);
    }

    let mut colormap: Option<Vec<Color>> = None;
    let mut indexes = Vec::new();
    if image_colors < 257 {
        let mut entries = Vec::with_capacity(image_colors);
        entries.extend_from_slice(&transparent);
        entries.extend_from_slice(&semitransparent);
        entries.extend_from_slice(&opaque);
        for entry in &entries {
            if !have_color && (entry.red != entry.green || entry.red != entry.blue) {
                have_color = true;
                have_non_bw = true;
                break;
            }
            if !have_non_bw && entry.red != 0.0 && entry.red != QUANTUM_RANGE as f64 {
                have_non_bw = true;
            }
        }
        indexes = palette_indexes(image, &entries);
        colormap = Some(entries);
    }

    // Cheap transparency: one fully transparent colour no opaque pixel
    // shares.
    let mut cheap_transparency = false;
    if transparent.len() == 1 {
        cheap_transparency = semitransparent.is_empty();
        if cheap_transparency {
            match &colormap {
                Some(entries) => {
                    let first = entries[0];
                    cheap_transparency = !entries.iter().skip(1).any(|entry| entry.rgb_equals(&first));
                }
                None => {
                    cheap_transparency = !(0..image.pixel_count()).any(|pixel| {
                        let [red, green, blue] = image.rgb(pixel);
                        image.alpha(pixel) != 0.0 && [red as u16, green as u16, blue as u16] == transparent_color[..3]
                    });
                }
            }
        }
    }

    let is_palette = colormap.is_some();
    let save_depth = depth;
    let mut bit_depth = depth;
    let matte = has_alpha;
    let mut image_matte = has_alpha;
    let mut color_type = match (have_color, image_matte) {
        (false, false) => COLOR_TYPE_GRAY,
        (false, true) => COLOR_TYPE_GRAY_ALPHA,
        (true, false) => COLOR_TYPE_RGB,
        (true, true) => COLOR_TYPE_RGB_ALPHA,
    };
    if color_type == COLOR_TYPE_GRAY && !has_alpha && !have_non_bw {
        bit_depth = 1;
    }
    let image_depth = bit_depth;

    let mut have_transparency = false;
    let mut transparency_gray: u16 = 0;
    let mut transparency_rgb = [0u16; 3];
    if matte {
        if let Some(entries) = &colormap {
            color_type = if have_color { COLOR_TYPE_RGB_ALPHA } else { COLOR_TYPE_GRAY_ALPHA };
            if transparent.len() + semitransparent.len() == 0 {
                image_matte = false;
                color_type &= 0x03;
            } else {
                let mask = depth_mask(bit_depth);
                let first = entries[0];
                transparency_rgb = [quantum_to_short(first.red as f32) & mask, quantum_to_short(first.green as f32) & mask, quantum_to_short(first.blue as f32) & mask];
                transparency_gray = quantum_to_short(intensity(&first) as f32) & mask;
                have_transparency = cheap_transparency;
                if have_transparency {
                    color_type &= 0x03;
                    if image_depth == 8 {
                        transparency_rgb = transparency_rgb.map(|sample| sample & 0xff);
                        transparency_gray &= 0xff;
                    }
                }
            }
        } else if image_depth == 8 {
            transparency_rgb = transparency_rgb.map(|sample| sample & 0xff);
            transparency_gray &= 0xff;
        }
    }
    let matte = image_matte;
    if have_transparency {
        image_matte = false;
    }

    let mut palette = None;
    let mut transparency = None;
    let mut background_chunk = None;
    let mut palette_rows = false;
    if is_palette && !have_color && (!image_matte || image_depth >= 8) {
        let colors = colormap.as_ref().map_or(0, Vec::len);
        if image_matte {
            color_type = COLOR_TYPE_GRAY_ALPHA;
        } else {
            color_type = COLOR_TYPE_GRAY;
            if save_depth == 16 && image_depth == 8 {
                transparency_gray = transparency_gray.wrapping_mul(0x0101);
            }
        }
        if image_depth > 8 {
            bit_depth = 16;
        } else {
            bit_depth = 8;
            if color_type == COLOR_TYPE_GRAY && colors < 17 {
                bit_depth = reducible_gray_depth(colormap.as_deref().unwrap_or(&[]));
            }
        }
    } else if let Some(entries) = colormap.as_ref().filter(|_| image_depth <= 8) {
        color_type = COLOR_TYPE_PALETTE;
        let colors: Vec<[u8; 3]> = entries.iter().map(Color::palette_entry).collect();
        bit_depth = 1;
        while (1usize << bit_depth) < colors.len() {
            bit_depth <<= 1;
        }
        if matte {
            let count = transparent.len() + semitransparent.len();
            if count > 0 {
                let alphas: Vec<u8> = entries.iter().take(count).map(|entry| quantum_to_char(entry.alpha as f32)).collect();
                transparency = Some(alphas);
            }
        }
        palette = Some(colors);
        palette_rows = true;
        // bKGD names the background's palette entry.
        let position = entries.iter().position(|entry| entry.rgb_equals(&background));
        background_chunk = position.map(|index| vec![index as u8]);
    }

    if !palette_rows {
        // bKGD: the black background (zero at any depth).
        background_chunk = Some(match color_type {
            COLOR_TYPE_GRAY | COLOR_TYPE_GRAY_ALPHA => vec![0u8; 2],
            _ => vec![0u8; 6],
        });
        if have_transparency && color_type < 4 {
            let values = if color_type == COLOR_TYPE_GRAY {
                let gray = if bit_depth < 8 {
                    let maximum = ((1u32 << bit_depth) - 1) as f64;
                    ((maximum / 255.0) * transparency_gray as f64 + 0.5) as u16
                } else {
                    transparency_gray
                };
                gray.to_be_bytes().to_vec()
            } else {
                transparency_rgb.iter().flat_map(|sample| sample.to_be_bytes()).collect()
            };
            transparency = Some(values);
        }
    }

    // The "palette, bilevel or opaque monochrome" rows.
    let monochrome = is_palette && !image_matte && !have_non_bw;
    let sample_depth = if bit_depth > 8 { 16 } else { 8 };
    Plan {
        color_type,
        bit_depth,
        indexes,
        palette,
        transparency,
        background: background_chunk,
        monochrome,
        sample_depth,
    }
}

fn depth_mask(bit_depth: u8) -> u16 {
    match bit_depth {
        8 => 0x00ff,
        4 => 0x000f,
        2 => 0x0003,
        1 => 0x0001,
        _ => 0xffff,
    }
}

/// `GetPixelInfoIntensity` (Rec. 709 luma of the colour).
fn intensity(color: &Color) -> f64 {
    0.212656 * color.red + 0.715158 * color.green + 0.072186 * color.blue
}

/// The colour scan for images of more than 256 colours: any non-grey
/// pixel, any pixel neither black nor white.
fn scan_colors(image: &MagickImage, have_color: &mut bool, have_non_bw: &mut bool) {
    for y in 0..image.height {
        let row = y * image.width..(y + 1) * image.width;
        for pixel in row.clone() {
            let [red, green, blue] = image.rgb(pixel);
            if red != green || red != blue {
                *have_color = true;
                *have_non_bw = true;
                break;
            }
        }
        if *have_color {
            break;
        }
        if !*have_non_bw {
            for pixel in row {
                let [red, _, _] = image.rgb(pixel);
                if red != 0.0 && red != QUANTUM_RANGE {
                    *have_non_bw = true;
                    break;
                }
            }
        }
    }
}

/// Each pixel's first matching colormap entry (alpha compared too when the
/// image has it), 0 for none.
fn palette_indexes(image: &MagickImage, entries: &[Color]) -> Vec<usize> {
    let has_alpha = image.has_alpha();
    let key = |color: &Color| {
        let [red, green, blue] = color.rgb_key();
        let alpha = if has_alpha { sample_key(color.alpha) } else { 0 };
        [red, green, blue, alpha]
    };
    let mut first_entries = HashMap::new();
    for (index, entry) in entries.iter().enumerate() {
        first_entries.entry(key(entry)).or_insert(index);
    }
    (0..image.pixel_count())
        .map(|pixel| {
            let color = Color::from_pixel(image, pixel);
            first_entries.get(&key(&color)).copied().unwrap_or(0)
        })
        .collect()
}

/// The PNG8 coder's palette passes and chunks: the census (without the
/// background colour), taken again after alpha goes binary when the
/// image has partly transparent colours or more than one transparent
/// one; an 8-bit palette of the transparent colours, then the opaque
/// ones; tRNS for the transparent entries; bKGD naming the first black
/// entry, if any. The quantizer leaves at most 256 colours, so the coder's
/// 4-4-4, 3-3-3 and 3-3-2 fallbacks for more never run.
fn palette_plan(image: &mut MagickImage) -> Plan {
    let mut census = Census::take(image);
    if !census.semitransparent.is_empty() || census.transparent.len() > 1 {
        make_alpha_binary(image);
        census = Census::take(image);
    }
    let transparent_count = census.transparent.len() + census.semitransparent.len();
    let mut entries = census.transparent;
    entries.extend(census.semitransparent);
    entries.extend(census.opaque);

    let palette = entries.iter().map(Color::palette_entry).collect();
    let mut transparency = None;
    if image.has_alpha() && transparent_count > 0 {
        let alphas = entries[..transparent_count].iter().map(|entry| quantum_to_char(entry.alpha as f32)).collect();
        transparency = Some(alphas);
    }
    let background_index = entries.iter().position(|entry| entry.rgb_equals(&BACKGROUND));
    let background = background_index.map(|index| vec![index as u8]);
    let indexes = palette_indexes(image, &entries);
    Plan {
        color_type: COLOR_TYPE_PALETTE,
        bit_depth: 8,
        indexes,
        palette: Some(palette),
        transparency,
        background,
        monochrome: false,
        sample_depth: 8,
    }
}

/// PNG8's binary alpha: pixels less than half opaque become the (black)
/// background, fully transparent; the rest fully opaque.
fn make_alpha_binary(image: &mut MagickImage) {
    if !image.has_alpha() {
        return;
    }
    let half = QUANTUM_RANGE / 2.0;
    for pixel in image.quanta.chunks_mut(4) {
        if pixel[3] < half {
            pixel[0] = BACKGROUND.red as f32;
            pixel[1] = BACKGROUND.green as f32;
            pixel[2] = BACKGROUND.blue as f32;
            pixel[3] = 0.0;
        } else {
            pixel[3] = QUANTUM_RANGE;
        }
    }
}

/// The smallest grey depth (1, 2 or 4, else 8) whose levels repeat in
/// every 8-bit grey of the colormap.
fn reducible_gray_depth(entries: &[Color]) -> u8 {
    let mut depth_4 = true;
    let mut depth_2 = true;
    let mut depth_1 = true;
    for entry in entries {
        let level = quantum_to_char(entry.red as f32);
        if (level & 0x0f) != ((level & 0xf0) >> 4) {
            depth_4 = false;
            depth_2 = false;
            depth_1 = false;
        } else if (level & 0x03) != ((level & 0x0c) >> 2) {
            depth_2 = false;
            depth_1 = false;
        } else if (level & 0x01) != ((level & 0x02) >> 1) {
            depth_1 = false;
        }
    }
    if depth_1 {
        1
    } else if depth_2 {
        2
    } else if depth_4 {
        4
    } else {
        8
    }
}

/// Unfiltered rows, packed to the PNG bit depth.
fn rows(image: &MagickImage, plan: &Plan) -> Vec<Vec<u8>> {
    let mut rows = Vec::with_capacity(image.height);
    for y in 0..image.height {
        let mut samples: Vec<u16> = Vec::new();
        for x in 0..image.width {
            let pixel = y * image.width + x;
            let [red, green, blue] = image.rgb(pixel);
            let alpha = image.alpha(pixel);
            match plan.color_type {
                COLOR_TYPE_PALETTE => samples.push(plan.indexes[pixel] as u16),
                COLOR_TYPE_GRAY => {
                    let level = sample(red, plan.sample_depth);
                    let level = if plan.monochrome { if level > 127 { 255 } else { 0 } } else { level };
                    samples.push(level);
                }
                COLOR_TYPE_GRAY_ALPHA => {
                    samples.push(sample(red, plan.sample_depth));
                    samples.push(sample(alpha, plan.sample_depth));
                }
                COLOR_TYPE_RGB => {
                    samples.push(sample(red, plan.sample_depth));
                    samples.push(sample(green, plan.sample_depth));
                    samples.push(sample(blue, plan.sample_depth));
                }
                _ => {
                    samples.push(sample(red, plan.sample_depth));
                    samples.push(sample(green, plan.sample_depth));
                    samples.push(sample(blue, plan.sample_depth));
                    samples.push(sample(alpha, plan.sample_depth));
                }
            }
        }
        rows.push(pack(&samples, plan.bit_depth));
    }
    rows
}

fn sample(quantum: f32, depth: u8) -> u16 {
    if depth == 16 {
        return quantum_to_short(quantum);
    }
    quantum_to_char(quantum) as u16
}

/// libpng's `png_set_packing`: samples to the bit depth, most significant
/// first (low bits of each 8-bit value below 8 bits; any non-zero value
/// is 1 at depth 1).
fn pack(samples: &[u16], bit_depth: u8) -> Vec<u8> {
    match bit_depth {
        16 => samples.iter().flat_map(|sample| sample.to_be_bytes()).collect(),
        8 => samples.iter().map(|sample| *sample as u8).collect(),
        _ => {
            let per_byte = 8 / bit_depth as usize;
            let mask = (1u16 << bit_depth) - 1;
            let mut bytes = vec![0u8; samples.len().div_ceil(per_byte)];
            for (index, sample) in samples.iter().enumerate() {
                let value = if bit_depth == 1 { u16::from(*sample != 0) } else { *sample & mask };
                let shift = 8 - bit_depth as usize * (index % per_byte + 1);
                bytes[index / per_byte] |= (value << shift) as u8;
            }
            bytes
        }
    }
}

/// libpng's row filtering: adaptive (`png_write_find_filter`, minimum sum
/// of absolute differences, earliest filter on ties) for 8- and 16-bit
/// non-palette rows, none otherwise. Returns the filtered stream.
fn filter_rows(rows: &[Vec<u8>], plan: &Plan) -> Vec<u8> {
    let adaptive = plan.color_type != COLOR_TYPE_PALETTE && plan.bit_depth >= 8;
    let channels = match plan.color_type {
        COLOR_TYPE_GRAY | COLOR_TYPE_PALETTE => 1,
        COLOR_TYPE_GRAY_ALPHA => 2,
        COLOR_TYPE_RGB => 3,
        _ => 4,
    };
    let pixel_bits = channels * plan.bit_depth as usize;
    let bpp = pixel_bits.div_ceil(8);

    let mut stream = Vec::with_capacity(rows.iter().map(|row| row.len() + 1).sum());
    let mut previous = vec![0u8; rows.first().map_or(0, Vec::len)];
    for row in rows {
        if !adaptive {
            stream.push(0);
            stream.extend_from_slice(row);
            previous.clone_from(row);
            continue;
        }
        let (filter, filtered) = best_filter(row, &previous, bpp);
        stream.push(filter);
        stream.extend_from_slice(&filtered);
        previous.clone_from(row);
    }
    stream
}

fn weight(value: u8) -> usize {
    if value < 128 { value as usize } else { 256 - value as usize }
}

fn best_filter(row: &[u8], previous: &[u8], bpp: usize) -> (u8, Vec<u8>) {
    let mut best_filter = 0u8;
    let mut best_row = row.to_vec();
    let mut minimum: usize = row.iter().map(|&value| weight(value)).sum();
    for filter in 1..=4u8 {
        let candidate = apply_filter(filter, row, previous, bpp);
        let sum: usize = candidate.iter().map(|&value| weight(value)).sum();
        if sum < minimum {
            minimum = sum;
            best_filter = filter;
            best_row = candidate;
        }
    }
    (best_filter, best_row)
}

fn apply_filter(filter: u8, row: &[u8], previous: &[u8], bpp: usize) -> Vec<u8> {
    let mut output = vec![0u8; row.len()];
    for index in 0..row.len() {
        let left = if index >= bpp { row[index - bpp] as i32 } else { 0 };
        let up = previous[index] as i32;
        let up_left = if index >= bpp { previous[index - bpp] as i32 } else { 0 };
        let predictor = match filter {
            1 => left,
            2 => up,
            3 => (left + up) / 2,
            _ => paeth(left, up, up_left),
        };
        output[index] = (row[index] as i32 - predictor) as u8;
    }
    output
}

fn paeth(left: i32, up: i32, up_left: i32) -> i32 {
    let p = up - up_left;
    let pc_raw = left - up_left;
    let pa = p.abs();
    let pb = pc_raw.abs();
    let pc = (p + pc_raw).abs();
    if pa <= pb && pa <= pc {
        left
    } else if pb <= pc {
        up
    } else {
        up_left
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerthus_engine::Resolution;

    fn planned(values: Vec<f32>, width: usize) -> Plan {
        let height = values.len() / width;
        let map = Map::from_data(Resolution::new(width, height), values);
        let mut image = MagickImage::from_map(&map);
        let depth = if lossless_reduce_ok(&image) { 8 } else { 16 };
        if depth == 8 {
            image.set_depth_8();
        }
        plan(&image, depth)
    }

    #[test]
    fn continuous_greys_are_16_bit() {
        let values: Vec<f32> = (0..1024).map(|index| index as f32 / 1023.0 * 0.999).collect();
        let plan = planned(values, 32);
        assert_eq!((plan.color_type, plan.bit_depth), (COLOR_TYPE_GRAY, 16));
    }

    #[test]
    fn black_and_white_is_one_bit() {
        let values = vec![0.0, 1.0, 1.0, 0.0];
        let plan = planned(values, 2);
        assert_eq!((plan.color_type, plan.bit_depth), (COLOR_TYPE_GRAY, 1));
        assert!(plan.monochrome);
    }

    #[test]
    fn packing_takes_low_bits() {
        assert_eq!(pack(&[255, 0, 255, 255, 0, 0, 0, 255], 1), vec![0b1011_0001]);
        assert_eq!(pack(&[0x55, 0xaa], 2), vec![0b0110_0000]);
    }

    #[test]
    fn colours_within_magick_epsilon_count_once() {
        // 1e-20 is 6.5e-16 quanta: black to the census, so the image is
        // black and white.
        let values = vec![0.0, 1e-20, 1.0, 0.0];
        let plan = planned(values, 2);
        assert_eq!((plan.color_type, plan.bit_depth), (COLOR_TYPE_GRAY, 1));
    }

    /// The PNG8 coder's plan for planar map values.
    fn planned_png8(channels: Vec<Vec<f32>>, width: usize) -> Plan {
        let height = channels[0].len() / width;
        let map = Map::from_channels(Resolution::new(width, height), channels);
        let mut image = MagickImage::from_map(&map);
        image.set_palette_type(2);
        if image.depth() != 8 {
            image.set_depth_8();
        }
        palette_plan(&mut image)
    }

    #[test]
    fn png8_palettes_colours_in_order_seen() {
        let gray = vec![0.5, 1.0, 0.5, 0.0, 1.0, 0.0];
        let plan = planned_png8(vec![gray], 3);
        assert_eq!((plan.color_type, plan.bit_depth), (COLOR_TYPE_PALETTE, 8));
        assert_eq!(plan.palette, Some(vec![[128, 128, 128], [255, 255, 255], [0, 0, 0]]));
        assert_eq!(plan.indexes, vec![0, 1, 0, 2, 1, 2]);
        assert_eq!(plan.transparency, None);
        // bKGD names the black entry.
        assert_eq!(plan.background, Some(vec![2]));
    }

    #[test]
    fn png8_makes_alpha_binary() {
        let red = vec![1.0, 0.25, 0.5, 0.75];
        let green = vec![0.0, 0.25, 0.5, 0.75];
        let blue = vec![0.0, 0.25, 0.5, 0.75];
        let alpha = vec![1.0, 0.25, 0.6, 0.0];
        let plan = planned_png8(vec![red, green, blue, alpha], 2);
        // The transparent black entry first, then the opaque colours.
        let palette = plan.palette.expect("a palette");
        assert_eq!(palette[0], [0, 0, 0]);
        assert_eq!(palette.len(), 3);
        assert_eq!(plan.transparency, Some(vec![0]));
        assert_eq!(plan.indexes, vec![1, 0, 2, 0]);
    }
}
