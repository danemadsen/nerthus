//! ImageMagick colourspace bookkeeping (`colorspace.c`).
//!
//! The nodes toggle images between grey, linear grey and sRGB constantly. Most
//! of those conversions are gamma round trips, but some (for example a
//! relabelled grey image converted back to linear grey) really change the
//! pixels, so images carry their IM colourspace and convert like IM does.

use rayon::prelude::*;

use crate::image::Image;

/// The colourspaces the nodes' images move between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colorspace {
    SRgb,
    LinearRgb,
    Gray,
    LinearGray,
}

impl Colorspace {
    pub fn is_gray(self) -> bool {
        matches!(self, Colorspace::Gray | Colorspace::LinearGray)
    }

    /// The default colourspace for an image with `channels` channels: grey
    /// buffers are linear grey, colour ones are sRGB.
    pub fn for_channels(channels: usize) -> Self {
        if channels <= 2 {
            Colorspace::LinearGray
        } else {
            Colorspace::SRgb
        }
    }
}

const LUMA_RED: f64 = 0.212656;
const LUMA_GREEN: f64 = 0.715158;
const LUMA_BLUE: f64 = 0.072186;

/// IM `DecodeGamma`: x^2.4 via a Chebyshev approximation.
pub fn decode_gamma(x: f64) -> f64 {
    const COEFFICIENTS: [f64; 9] = [
        1.791_748_858_804_327_8,
        0.820_456_143_719_768_6,
        0.027_694_100_686_325_412,
        -0.000_942_443_351_817_621_3,
        0.000_064_355_540_911_469_71,
        -5.722_440_463_606_075e-6,
        5.876_766_943_731_118e-7,
        -6.613_992_005_358_972e-8,
        7.932_324_269_622_745e-9,
    ];
    const POWERS_OF_TWO: [f64; 5] = [1.0, 2.639_015_821_545_788_4, 6.964_404_506_368_992, 1.837_917_367_995_255_8e1, 4.850_293_012_833_273e1];
    let (mantissa, exponent) = frexp(x);
    let p = chebyshev(mantissa, &COEFFICIENTS);
    let (quotient, remainder) = floor_div(exponent - 1, 5);
    x * ldexp(POWERS_OF_TWO[remainder as usize] * p, 7 * quotient)
}

/// IM `EncodeGamma`: x^(1/2.4) via a Chebyshev approximation.
pub fn encode_gamma(x: f64) -> f64 {
    const COEFFICIENTS: [f64; 9] = [
        1.175_820_023_299_690_3,
        0.166_657_630_948_890_6,
        -0.008_315_489_493_904_213,
        0.000_751_879_767_804_202_8,
        -0.000_083_240_178_519_391_8,
        0.000_010_229_209_410_070_008,
        -1.340_046_640_986_024_6e-06,
        1.833_342_224_163_537_5e-7,
        -2.587_859_676_134_886e-8,
    ];
    const POWERS_OF_TWO: [f64; 12] = [
        1.0,
        1.334_839_854_170_034_4,
        1.781_797_436_280_678_5,
        2.378_414_230_005_442,
        3.174_802_103_936_399,
        4.237_852_377_437_181,
        5.656_854_249_492_381,
        7.550_994_501_453_548,
        1.007_936_839_915_898_6e1,
        1.345_434_264_405_943_4e1,
        1.795_939_277_294_997e1,
        2.397_291_323_002_690_8e1,
    ];
    let (mantissa, exponent) = frexp(x);
    let p = chebyshev(mantissa, &COEFFICIENTS);
    let (quotient, remainder) = floor_div(exponent - 1, 12);
    ldexp(POWERS_OF_TWO[remainder as usize] * p, 5 * quotient)
}

fn chebyshev(mantissa: f64, coefficients: &[f64; 9]) -> f64 {
    let mut terms = [0.0f64; 9];
    terms[0] = 1.0;
    terms[1] = 4.0 * mantissa - 3.0;
    for index in 2..9 {
        terms[index] = 2.0 * terms[1] * terms[index - 1] - terms[index - 2];
    }
    let mut sum = 0.0;
    for (coefficient, term) in coefficients.iter().zip(terms.iter()) {
        sum += coefficient * term;
    }
    sum
}

/// C `div` adjusted to a non-negative remainder, as IM does.
fn floor_div(value: i32, divisor: i32) -> (i32, i32) {
    let mut quotient = value / divisor;
    let mut remainder = value % divisor;
    if remainder < 0 {
        quotient -= 1;
        remainder += divisor;
    }
    (quotient, remainder)
}

/// C `frexp`: mantissa in [0.5, 1) and exponent.
fn frexp(value: f64) -> (f64, i32) {
    if value == 0.0 || !value.is_finite() {
        return (value, 0);
    }
    let bits = value.to_bits();
    let exponent_bits = ((bits >> 52) & 0x7ff) as i32;
    if exponent_bits == 0 {
        // Subnormal: normalise first.
        let (mantissa, exponent) = frexp(value * f64::powi(2.0, 64));
        return (mantissa, exponent - 64);
    }
    let exponent = exponent_bits - 1022;
    let mantissa_bits = (bits & !(0x7ffu64 << 52)) | (1022u64 << 52);
    (f64::from_bits(mantissa_bits), exponent)
}

fn ldexp(value: f64, exponent: i32) -> f64 {
    value * 2.0f64.powi(exponent)
}

/// IM `DecodePixelGamma` on a normalised value.
pub fn decode_pixel_gamma(value: f64) -> f64 {
    if value <= 0.040_448_236_277_107_6 {
        return value / 12.92f32 as f64;
    }
    decode_gamma((value + 0.055) / 1.055)
}

/// IM `EncodePixelGamma` on a normalised value.
pub fn encode_pixel_gamma(value: f64) -> f64 {
    if value <= 0.003_130_668_442_500_588_3 {
        return 12.92f32 as f64 * value;
    }
    1.055 * encode_gamma(value) - 0.055
}

fn luma(red: f64, green: f64, blue: f64) -> f64 {
    LUMA_RED * red + LUMA_GREEN * green + LUMA_BLUE * blue
}

impl Image {
    /// Number of colour (non-alpha) channels.
    fn color_channels(&self) -> usize {
        if self.channels == 2 || self.channels == 4 {
            self.channels - 1
        } else {
            self.channels
        }
    }

    fn has_alpha_channel(&self) -> bool {
        self.channels == 2 || self.channels == 4
    }

    /// IM `SetImageColorspace`: relabels without converting. Grey to colour
    /// replicates the grey channel; colour to grey keeps the red channel.
    pub fn set_colorspace(&mut self, colorspace: Colorspace) {
        if self.colorspace == colorspace {
            return;
        }
        let was_gray = self.colorspace.is_gray();
        self.colorspace = colorspace;
        if was_gray == colorspace.is_gray() {
            return;
        }
        let alpha = self.has_alpha_channel();
        let old_channels = self.channels;
        let colors = if colorspace.is_gray() { 1 } else { 3 };
        let new_channels = colors + usize::from(alpha);
        let mut data = Vec::with_capacity(self.pixel_count() * new_channels);
        for pixel in self.data.chunks(old_channels) {
            for _ in 0..colors {
                data.push(pixel[0]);
            }
            if alpha {
                data.push(pixel[old_channels - 1]);
            }
        }
        self.data = data;
        self.channels = new_channels;
    }

    /// `TransformsRGBImage`: the current colourspace to sRGB.
    fn transform_to_srgb(&mut self) {
        match self.colorspace {
            Colorspace::SRgb => {}
            Colorspace::Gray | Colorspace::LinearGray => {
                let linear = self.colorspace == Colorspace::LinearGray;
                self.set_colorspace(Colorspace::SRgb);
                let channels = self.channels;
                self.data.par_chunks_mut(channels).for_each(|pixel| {
                    let (red, green, blue) = (pixel[0] as f64, pixel[1] as f64, pixel[2] as f64);
                    let gray = if linear {
                        luma(encode_pixel_gamma(red), encode_pixel_gamma(green), encode_pixel_gamma(blue))
                    } else {
                        luma(red, green, blue)
                    };
                    pixel[0] = gray as f32;
                    pixel[1] = gray as f32;
                    pixel[2] = gray as f32;
                });
            }
            Colorspace::LinearRgb => {
                let colors = self.color_channels();
                let channels = self.channels;
                self.data.par_chunks_mut(channels).for_each(|pixel| {
                    for value in pixel.iter_mut().take(colors) {
                        *value = encode_pixel_gamma(*value as f64) as f32;
                    }
                });
                self.colorspace = Colorspace::SRgb;
            }
        }
    }

    /// `sRGBTransformImage`: sRGB to `target`.
    fn transform_from_srgb(&mut self, target: Colorspace) {
        match target {
            Colorspace::SRgb => {}
            Colorspace::Gray | Colorspace::LinearGray => {
                let linear = target == Colorspace::LinearGray;
                let channels = self.channels;
                self.data.par_chunks_mut(channels).for_each(|pixel| {
                    let (red, green, blue) = (pixel[0] as f64, pixel[1] as f64, pixel[2] as f64);
                    let gray = if linear {
                        luma(decode_pixel_gamma(red), decode_pixel_gamma(green), decode_pixel_gamma(blue))
                    } else {
                        luma(red, green, blue)
                    };
                    pixel[0] = gray as f32;
                });
                self.set_colorspace(target);
            }
            Colorspace::LinearRgb => {
                let colors = self.color_channels();
                let channels = self.channels;
                self.data.par_chunks_mut(channels).for_each(|pixel| {
                    for value in pixel.iter_mut().take(colors) {
                        *value = decode_pixel_gamma(*value as f64) as f32;
                    }
                });
                self.colorspace = Colorspace::LinearRgb;
            }
        }
    }

    /// IM `TransformImageColorspace` (Magick.NET's `ColorSpace` setter).
    pub fn transform_colorspace(&mut self, target: Colorspace) {
        if self.colorspace == target {
            return;
        }
        if target == Colorspace::SRgb {
            self.transform_to_srgb();
            return;
        }
        if self.colorspace != Colorspace::SRgb {
            self.transform_to_srgb();
        }
        self.transform_from_srgb(target);
    }

    /// IM `SetImageType(GrayscaleType)` (Magick.NET `ColorType = Grayscale`):
    /// converts to GRAY and drops alpha.
    pub fn set_grayscale_type(&mut self) {
        self.transform_colorspace(Colorspace::Gray);
        self.alpha_off();
    }

    /// Magick.NET `Alpha(AlphaOption.Off)`: deactivates (drops) alpha.
    pub fn alpha_off(&mut self) {
        if !self.has_alpha_channel() {
            return;
        }
        let channels = self.channels;
        let colors = channels - 1;
        let mut data = Vec::with_capacity(self.pixel_count() * colors);
        for pixel in self.data.chunks(channels) {
            data.extend_from_slice(&pixel[..colors]);
        }
        self.data = data;
        self.channels = colors;
    }

    /// The habitual `Alpha(Off/Remove); ColorType = Grayscale;
    /// ColorSpace = LinearGray` sequence for opaque images.
    pub fn to_linear_gray(&mut self) {
        self.alpha_off();
        self.set_grayscale_type();
        self.transform_colorspace(Colorspace::LinearGray);
    }
}
