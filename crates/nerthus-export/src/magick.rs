//! The image Magick.NET makes of a map and the sample
//! conversions of the ImageMagick it bundles (7.1.2-19, Q16 HDRI): the
//! 8- and 16-bit formats are written from this.

use nerthus_engine::Map;
use nerthus_image::octree::{self, Dither, Layout, QuantizeOptions, Quantizer};

use crate::colorspace::Colorspace;

/// `QuantumRange`: Q16 quanta run from 0 to 65535 (as `f32` in HDRI).
pub const QUANTUM_RANGE: f32 = 65535.0;

/// `MaxMap + 1`: the levels `ScaleQuantumToMap` gives.
const MAP_LEVELS: usize = 65536;

/// `MAGICKCORE_QUANTUM_DEPTH`.
const QUANTUM_DEPTH: usize = 16;

/// A map as a Magick.NET image: grey for one channel, RGBA for four, RGB
/// otherwise, every value `v * 65535f`.
#[derive(Clone, Debug)]
pub struct MagickImage {
    pub width: usize,
    pub height: usize,
    /// Samples per pixel: the colourspace's colour channels (1 for grey,
    /// 4 for CMYK, else 3), then alpha if the image has it.
    pub channels: usize,
    /// Quanta, interleaved.
    pub quanta: Vec<f32>,
    /// The image's ImageMagick colourspace.
    pub colorspace: Colorspace,
}

impl MagickImage {
    /// The image a map makes. `SetPixels` copies the map's interleaved samples
    /// in order, so a map whose channel count differs from the image's
    /// (two channels, five, …) only partly fills it; the rest stays black.
    pub fn from_map(map: &Map) -> Self {
        let channels = match map.channel_count() {
            1 => 1,
            4 => 4,
            _ => 3,
        };
        let width = map.width();
        let height = map.height();
        let mut quanta = vec![0.0f32; width * height * channels];
        let interleaved = map.to_interleaved();
        let count = interleaved.len().min(quanta.len());
        for index in 0..count {
            quanta[index] = interleaved[index] * QUANTUM_RANGE;
        }
        let colorspace = if channels == 1 { Colorspace::Gray } else { Colorspace::SRgb };
        Self { width, height, channels, quanta, colorspace }
    }

    /// The colour channels of the image's colourspace.
    pub fn color_channels(&self) -> usize {
        match self.colorspace {
            Colorspace::Gray | Colorspace::LinearGray => 1,
            Colorspace::Cmyk => 4,
            _ => 3,
        }
    }

    pub fn has_alpha(&self) -> bool {
        self.channels > self.color_channels()
    }

    /// Whether the image is in a grey colourspace (made from one channel,
    /// or taken to Gray or LinearGray).
    pub fn is_gray_colorspace(&self) -> bool {
        self.colorspace.is_gray()
    }

    pub fn pixel_count(&self) -> usize {
        self.width * self.height
    }

    /// The red, green and blue of a pixel (all the grey value for a grey
    /// image, whose colour channels share one offset).
    pub fn rgb(&self, pixel: usize) -> [f32; 3] {
        let start = pixel * self.channels;
        if self.is_gray_colorspace() {
            let gray = self.quanta[start];
            return [gray, gray, gray];
        }
        [self.quanta[start], self.quanta[start + 1], self.quanta[start + 2]]
    }

    /// The alpha of a pixel (`OpaqueAlpha` without an alpha channel).
    pub fn alpha(&self, pixel: usize) -> f32 {
        if self.has_alpha() {
            return self.quanta[pixel * self.channels + self.channels - 1];
        }
        QUANTUM_RANGE
    }

    /// `IdentifyImageCoderGray`: a grey image, or an image in an
    /// sRGB-compatible colourspace whose every pixel has red, green and
    /// blue within `MagickEpsilon` of each other.
    pub fn is_gray(&self) -> bool {
        if self.is_gray_colorspace() {
            return true;
        }
        if !self.colorspace.is_srgb_compatible() {
            return false;
        }
        (0..self.pixel_count()).all(|pixel| {
            let [red, green, blue] = self.rgb(pixel);
            let red_green = (red as f64 - green as f64).abs();
            let green_blue = (green as f64 - blue as f64).abs();
            red_green < MAGICK_EPSILON && green_blue < MAGICK_EPSILON
        })
    }

    /// `SetImageDepth(image, 8)`: every value requantized to 8 bits.
    pub fn set_depth_8(&mut self) {
        for quantum in &mut self.quanta {
            *quantum = requantize_8(*quantum);
        }
    }

    /// `GetImageDepth`: the fewest bits (16 at most) that hold the
    /// samples, found as one thread finds it: the depth only grows, and
    /// each sample is tried from the depth reached so far.
    pub fn depth(&self) -> usize {
        let mut depth = 1;
        for &quantum in &self.quanta {
            while depth < QUANTUM_DEPTH {
                let range = (1u64 << depth) - 1;
                let level = quantum_to_any(quantum, range);
                if quantum == any_to_quantum(level, range) {
                    break;
                }
                depth += 1;
            }
            if depth == QUANTUM_DEPTH {
                break;
            }
        }
        depth
    }

    /// `SetImageType(image, PaletteType)`, or `PaletteAlphaType` for an
    /// image with alpha, as the PNG8 coder calls it on the build's sRGB and
    /// grey images: `QuantizeImage` to at most 256 colours with the
    /// `image->dither` default (Riemersma), every pixel left holding its
    /// colormap colour. `cache_shift` is the dither cache's `CacheShift`
    /// (2 in Windows builds of ImageMagick).
    pub fn set_palette_type(&mut self, cache_shift: u32) {
        // An image in a colourspace that is not sRGB-compatible goes back
        // to sRGB first. `IdentifyImageGray`: the grey image a map makes
        // is typed Grayscale, others are checked.
        if !self.colorspace.is_srgb_compatible() {
            self.transform_colorspace(Colorspace::SRgb);
        }
        let gray = self.is_gray();
        let maximum_colors = 256;
        let tree_depth = octree::automatic_tree_depth(maximum_colors, self.has_alpha(), gray);
        let options = QuantizeOptions {
            maximum_colors,
            tree_depth,
            dither: Dither::Riemersma,
            cache_shift,
        };
        if self.has_alpha() {
            self.quantize_transparent(gray, &options);
            return;
        }
        if gray && self.colorspace != Colorspace::Gray {
            // `SetGrayscaleImage` takes a grey image to GRAY first (a
            // linear one by way of sRGB, as PNG8 writes show); the
            // colormap it builds is replaced before anything reads it.
            self.transform_colorspace(Colorspace::Gray);
        }
        let layout = if self.channels == 1 { Layout::Gray } else { Layout::Rgb };
        octree::quantize(&mut self.quanta, self.width, self.height, layout, &options);
    }

    /// `QuantizeImage` in the Transparent colourspace (`PaletteAlphaType`):
    /// taking an image there runs `TransformsRGBImage` (for an sRGB image
    /// its identity lookup tables, which round red, green and blue to whole
    /// quanta) before classification, and again before assignment. A grey
    /// image reaches classification through `SetGrayscaleImage`'s colormap
    /// instead, taken to GRAY first as in `set_palette_type`.
    fn quantize_transparent(&mut self, gray: bool, options: &QuantizeOptions) {
        if gray {
            if self.colorspace != Colorspace::Gray {
                self.transform_colorspace(Colorspace::Gray);
            }
            self.gray_through_colormap();
        } else {
            self.transform_to_srgb();
        }
        let quantizer = Quantizer::classify(&self.quanta, self.width, self.height, Layout::Rgba, options);
        self.round_to_map();
        quantizer.assign(&mut self.quanta);
    }

    /// What a GRAY image with alpha becomes on its way to the Transparent
    /// colourspace. `SetGrayscaleImage` gives every 16-bit grey level a
    /// colormap entry holding the first value seen at that level,
    /// numbering the entries in the order seen in their alpha (its sort
    /// key, never reset). The transform then syncs every pixel to its
    /// entry, alpha included, and goes back to sRGB (luma again). An
    /// OpenMP build of ImageMagick races for "first seen"; this takes the
    /// pixels in order.
    fn gray_through_colormap(&mut self) {
        let mut entries: Vec<Option<(f32, usize)>> = vec![None; MAP_LEVELS];
        let mut entry_count = 0;
        let mut rgba = Vec::with_capacity(self.pixel_count() * 4);
        for pixel in self.quanta.chunks(self.channels) {
            let gray = pixel[0];
            let level = quantum_to_map(gray);
            let (value, number) = match entries[level] {
                Some(entry) => entry,
                None => {
                    let entry = (gray, entry_count);
                    entries[level] = Some(entry);
                    entry_count += 1;
                    entry
                }
            };
            let srgb = luma(value, value, value);
            rgba.extend_from_slice(&[srgb, srgb, srgb, number as f32]);
        }
        self.quanta = rgba;
        self.channels = 4;
        self.colorspace = Colorspace::SRgb;
    }

    /// `TransformsRGBImage` on an sRGB (or undefined) colour image: its
    /// identity lookup tables round red, green and blue to whole quanta
    /// (`ScaleQuantumToMap`).
    pub fn round_to_map(&mut self) {
        let channels = self.channels;
        for pixel in self.quanta.chunks_mut(channels) {
            for sample in &mut pixel[..3] {
                *sample = quantum_to_map(*sample) as f32;
            }
        }
    }
}

/// The Rec. 709 luma ImageMagick's GRAY transforms use, as a quantum.
fn luma(red: f32, green: f32, blue: f32) -> f32 {
    (0.212656 * red as f64 + 0.715158 * green as f64 + 0.072186 * blue as f64) as f32
}

/// `ScaleQuantumToMap` (Q16 HDRI): rounded in single precision.
pub fn quantum_to_map(quantum: f32) -> usize {
    if quantum >= (MAP_LEVELS - 1) as f32 {
        return MAP_LEVELS - 1;
    }
    if quantum.is_nan() || quantum <= 0.0 {
        return 0;
    }
    (quantum + 0.5f32) as usize
}

/// `MagickEpsilon`.
pub const MAGICK_EPSILON: f64 = 1.0e-12;

/// `ScaleQuantumToShort` (Q16 HDRI): rounded in single precision.
pub fn quantum_to_short(quantum: f32) -> u16 {
    if quantum.is_nan() || quantum <= 0.0 {
        return 0;
    }
    if quantum >= 65535.0 {
        return 65535;
    }
    (quantum + 0.5f32) as u16
}

/// `ScaleQuantumToChar` (Q16 HDRI): divided and rounded in single
/// precision.
pub fn quantum_to_char(quantum: f32) -> u8 {
    if quantum.is_nan() || quantum <= 0.0 {
        return 0;
    }
    let scaled = quantum / 257.0f32;
    if scaled >= 255.0 {
        return 255;
    }
    (scaled + 0.5f32) as u8
}

/// `ScaleCharToQuantum`.
pub fn char_to_quantum(value: u8) -> f32 {
    (257.0f64 * value as f64) as f32
}

/// `ScaleAnyToQuantum(ScaleQuantumToAny(ClampPixel(q), 255), 255)`: what
/// `SetImageDepth(image, 8)` makes of a value.
pub fn requantize_8(quantum: f32) -> f32 {
    let clamped = if (quantum as f64) < 0.0 {
        0.0f32
    } else if quantum as f64 >= QUANTUM_RANGE as f64 {
        QUANTUM_RANGE
    } else {
        quantum
    };
    let level = quantum_to_any(clamped, 255);
    any_to_quantum(level, 255)
}

/// `ScaleQuantumToAny` (HDRI): a quantum as a level of `range`.
fn quantum_to_any(quantum: f32, range: u64) -> u64 {
    if quantum.is_nan() || quantum <= 0.0 {
        return 0;
    }
    let scaled = range as f64 * quantum as f64 / QUANTUM_RANGE as f64;
    if scaled >= u64::MAX as f64 {
        return u64::MAX;
    }
    (scaled + 0.5) as u64
}

/// `ScaleAnyToQuantum` (HDRI): a level of `range` as a quantum.
fn any_to_quantum(level: u64, range: u64) -> f32 {
    if level > range {
        return QUANTUM_RANGE;
    }
    let reciprocal = safe_reciprocal(range as f64);
    (QUANTUM_RANGE as f64 * (level as f64 * reciprocal)) as f32
}

/// `MagickSafeReciprocal`.
fn safe_reciprocal(value: f64) -> f64 {
    if value > -MAGICK_EPSILON && value < MAGICK_EPSILON {
        return 1.0 / MAGICK_EPSILON;
    }
    1.0 / value
}

/// Whether a value survives the trip to 8 bits and back
/// (`QuantumToCharToQuantumEqQuantum`).
pub fn is_8_bit_exact(quantum: f32) -> bool {
    let value = quantum_to_char(quantum);
    char_to_quantum(value) as f64 == quantum as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_bit_values_round_trip() {
        for value in 0..=255u8 {
            let quantum = char_to_quantum(value);
            assert_eq!(quantum_to_char(quantum), value);
            assert!(is_8_bit_exact(quantum));
            assert_eq!(requantize_8(quantum), quantum);
        }
        assert!(!is_8_bit_exact(257.5));
    }

    #[test]
    fn shorts_round_in_single_precision() {
        assert_eq!(quantum_to_short(0.49), 0);
        assert_eq!(quantum_to_short(1.5), 2);
        assert_eq!(quantum_to_short(65534.6), 65535);
        assert_eq!(quantum_to_short(f32::NAN), 0);
    }

    fn gray_image(quanta: Vec<f32>) -> MagickImage {
        MagickImage {
            width: quanta.len(),
            height: 1,
            channels: 1,
            quanta,
            colorspace: Colorspace::Gray,
        }
    }

    #[test]
    fn depth_is_the_fewest_bits_that_hold_the_samples() {
        assert_eq!(gray_image(vec![0.0, 65535.0]).depth(), 1);
        assert_eq!(gray_image(vec![0.0, 21845.0, 65535.0]).depth(), 2);
        assert_eq!(gray_image(vec![0.0, char_to_quantum(37), 65535.0]).depth(), 8);
        assert_eq!(gray_image(vec![0.0, 1234.5]).depth(), 16);
    }

    #[test]
    fn map_levels_round_in_single_precision() {
        assert_eq!(quantum_to_map(0.49), 0);
        assert_eq!(quantum_to_map(1.5), 2);
        assert_eq!(quantum_to_map(-3.0), 0);
        assert_eq!(quantum_to_map(70000.0), 65535);
        assert_eq!(quantum_to_map(f32::NAN), 0);
    }

    #[test]
    fn palette_type_leaves_at_most_256_colours() {
        let quanta: Vec<f32> = (0..64 * 64 * 3).map(|index| ((index * 2654435761usize) % 65536) as f32).collect();
        let mut image = MagickImage {
            width: 64,
            height: 64,
            channels: 3,
            quanta,
            colorspace: Colorspace::SRgb,
        };
        image.set_palette_type(2);
        let mut colors: Vec<[u32; 3]> = image.quanta.chunks(3).map(|pixel| [pixel[0].to_bits(), pixel[1].to_bits(), pixel[2].to_bits()]).collect();
        colors.sort_unstable();
        colors.dedup();
        assert!(colors.len() <= 256);
    }

    #[test]
    fn grey_colour_images_quantize_as_grey() {
        let levels: Vec<f32> = (0..256).map(|index| index as f32 * 255.9).collect();
        let mut gray = gray_image(levels.clone());
        let mut rgb = MagickImage {
            width: 256,
            height: 1,
            channels: 3,
            quanta: levels.iter().flat_map(|&level| [level, level, level]).collect(),
            colorspace: Colorspace::SRgb,
        };
        gray.set_palette_type(2);
        rgb.set_palette_type(2);
        assert_eq!(rgb.channels, 1);
        assert_eq!(rgb.quanta, gray.quanta);
    }
}
