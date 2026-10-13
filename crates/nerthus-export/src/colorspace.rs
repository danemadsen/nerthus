//! ImageMagick 7.1.2's colourspace conversions, as Magick.NET's
//! `MagickImage.ColorSpace` setter runs them (`TransformImageColorspace`):
//! from sRGB to another colourspace (`sRGBTransformImage`) and back
//! (`TransformsRGBImage`). Q16 HDRI: the quanta are `f32` and the
//! arithmetic `double`, and nothing is clamped (`ClampToQuantum` only
//! converts). The conversions of single values are `colorspace-private.h`'s
//! and `colorspace.c`'s, in the same operation order.

use nerthus_image::colorspace::{decode_gamma, encode_gamma};

use crate::magick::{char_to_quantum, quantum_to_map, MagickImage};
use crate::ycc_map::YCC_MAP;

pub const QUANTUM_RANGE: f64 = 65535.0;
pub const QUANTUM_SCALE: f64 = 1.0 / 65535.0;
const MAX_MAP: f64 = 65535.0;
const MAGICK_EPSILON: f64 = 1.0e-12;
const MAGICK_PI: f64 = std::f64::consts::PI;
const MAXIMUM_LOGARITHMIC_COLORSPACE: f64 = 1024.0;
/// `illuminant_tristimulus[D65Illuminant]`, the default illuminant.
const ILLUMINANT: [f64; 3] = [0.95047, 1.0, 1.08883];
const CIE_EPSILON: f64 = 216.0 / 24389.0;
const CIE_K: f64 = 24389.0 / 27.0;
/// The `white-luminance` Jzazbz uses when an image has none.
const WHITE_LUMINANCE: f64 = 10000.0;

/// `ColorspaceType` (Magick.NET's `ColorSpace`, the build's `Colorspace`),
/// in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colorspace {
    Undefined,
    Cmy,
    Cmyk,
    Gray,
    Hcl,
    Hclp,
    Hsb,
    Hsi,
    Hsl,
    Hsv,
    Hwb,
    Lab,
    Lch,
    LchAb,
    LchUv,
    Log,
    Lms,
    Luv,
    Ohta,
    Rec601YCbCr,
    Rec709YCbCr,
    Rgb,
    ScRgb,
    SRgb,
    Transparent,
    XyY,
    Xyz,
    YCbCr,
    Ycc,
    YDbDr,
    Yiq,
    YPbPr,
    Yuv,
    LinearGray,
    Jzazbz,
    DisplayP3,
    Adobe98,
    ProPhoto,
    Oklab,
    Oklch,
}

/// The members and the names projects save them by.
const NAMES: [(Colorspace, &str); 40] = [
    (Colorspace::Undefined, "Undefined"),
    (Colorspace::Cmy, "CMY"),
    (Colorspace::Cmyk, "CMYK"),
    (Colorspace::Gray, "Gray"),
    (Colorspace::Hcl, "HCL"),
    (Colorspace::Hclp, "HCLp"),
    (Colorspace::Hsb, "HSB"),
    (Colorspace::Hsi, "HSI"),
    (Colorspace::Hsl, "HSL"),
    (Colorspace::Hsv, "HSV"),
    (Colorspace::Hwb, "HWB"),
    (Colorspace::Lab, "Lab"),
    (Colorspace::Lch, "LCH"),
    (Colorspace::LchAb, "LCHab"),
    (Colorspace::LchUv, "LCHuv"),
    (Colorspace::Log, "Log"),
    (Colorspace::Lms, "LMS"),
    (Colorspace::Luv, "Luv"),
    (Colorspace::Ohta, "OHTA"),
    (Colorspace::Rec601YCbCr, "Rec601YCbCr"),
    (Colorspace::Rec709YCbCr, "Rec709YCbCr"),
    (Colorspace::Rgb, "RGB"),
    (Colorspace::ScRgb, "scRGB"),
    (Colorspace::SRgb, "sRGB"),
    (Colorspace::Transparent, "Transparent"),
    (Colorspace::XyY, "XyY"),
    (Colorspace::Xyz, "XYZ"),
    (Colorspace::YCbCr, "YCbCr"),
    (Colorspace::Ycc, "YCC"),
    (Colorspace::YDbDr, "YDbDr"),
    (Colorspace::Yiq, "YIQ"),
    (Colorspace::YPbPr, "YPbPr"),
    (Colorspace::Yuv, "YUV"),
    (Colorspace::LinearGray, "LinearGray"),
    (Colorspace::Jzazbz, "Jzazbz"),
    (Colorspace::DisplayP3, "DisplayP3"),
    (Colorspace::Adobe98, "Adobe98"),
    (Colorspace::ProPhoto, "ProPhoto"),
    (Colorspace::Oklab, "Oklab"),
    (Colorspace::Oklch, "Oklch"),
];

impl Colorspace {
    /// A `Colorspace` member by its saved name (Undefined when it is none
    /// of them).
    pub fn from_name(name: &str) -> Self {
        let found = NAMES.iter().find(|(_, known)| *known == name);
        found.map(|(colorspace, _)| *colorspace).unwrap_or(Colorspace::Undefined)
    }

    pub fn name(self) -> &'static str {
        let found = NAMES.iter().find(|(colorspace, _)| *colorspace == self);
        found.map(|(_, name)| *name).unwrap_or("Undefined")
    }

    /// `IsGrayColorspace`.
    pub fn is_gray(self) -> bool {
        matches!(self, Colorspace::Gray | Colorspace::LinearGray)
    }

    /// `IsRGBColorspace`: linear RGB.
    pub fn is_rgb(self) -> bool {
        matches!(self, Colorspace::Rgb | Colorspace::ScRgb | Colorspace::LinearGray)
    }

    /// `IssRGBColorspace`.
    pub fn is_srgb(self) -> bool {
        matches!(self, Colorspace::SRgb | Colorspace::Transparent)
    }

    /// `IssRGBCompatibleColorspace`: what coders write as it is.
    pub fn is_srgb_compatible(self) -> bool {
        matches!(
            self,
            Colorspace::SRgb
                | Colorspace::Rgb
                | Colorspace::Adobe98
                | Colorspace::ProPhoto
                | Colorspace::DisplayP3
                | Colorspace::ScRgb
                | Colorspace::Transparent
                | Colorspace::Gray
                | Colorspace::LinearGray
        )
    }

    /// `IsYCbCrCompatibleColorspace`.
    pub fn is_ycbcr_compatible(self) -> bool {
        matches!(self, Colorspace::YCbCr | Colorspace::Rec709YCbCr | Colorspace::Rec601YCbCr)
    }

    /// Whether `SetImageColorspace` gives an image in this colourspace the
    /// sRGB primaries (which coders write as cHRM, or TIFF's white point
    /// and primaries): all but grey, linear RGB, XYZ and xyY.
    pub fn has_chromaticity(self) -> bool {
        let linear = self.is_rgb() || matches!(self, Colorspace::Xyz | Colorspace::XyY);
        !(self.is_gray() || linear)
    }
}

/// `MagickSafeReciprocal`.
fn safe_reciprocal(x: f64) -> f64 {
    if x > -MAGICK_EPSILON && x < MAGICK_EPSILON {
        return 1.0 / MAGICK_EPSILON;
    }
    1.0 / x
}

/// `DegreesToRadians`.
fn degrees_to_radians(degrees: f64) -> f64 {
    MAGICK_PI * degrees / 180.0
}

/// `DecodePixelGamma` (7.1.2) on a quantum.
pub fn decode_pixel_gamma(pixel: f64) -> f64 {
    if pixel <= 0.040_448_236_277_107_6 * QUANTUM_RANGE {
        return pixel / 12.92;
    }
    let normal = (QUANTUM_SCALE * pixel + 0.055) / 1.055;
    QUANTUM_RANGE * decode_gamma(normal)
}

/// `EncodePixelGamma` (7.1.2) on a quantum.
pub fn encode_pixel_gamma(pixel: f64) -> f64 {
    if pixel <= 0.003_130_668_442_500_588_3 * QUANTUM_RANGE {
        return 12.92 * pixel;
    }
    let encoded = encode_gamma(QUANTUM_SCALE * pixel);
    QUANTUM_RANGE * (1.055 * encoded - 0.055)
}

/// `ScaleMapToQuantum` (Q16 HDRI).
pub fn map_to_quantum(value: f64) -> f32 {
    if value <= 0.0 {
        return 0.0;
    }
    if value >= MAX_MAP {
        return QUANTUM_RANGE as f32;
    }
    value as f32
}

// --- sRGB to another colourspace (`ConvertRGBTo…`, quanta in) ----------------

fn rgb_to_xyz(red: f64, green: f64, blue: f64) -> [f64; 3] {
    let r = QUANTUM_SCALE * decode_pixel_gamma(red);
    let g = QUANTUM_SCALE * decode_pixel_gamma(green);
    let b = QUANTUM_SCALE * decode_pixel_gamma(blue);
    let x = (0.412_395_588_967_414_24 * r) + (0.357_583_430_763_714_8 * g) + (0.180_492_647_381_701_57 * b);
    let y = (0.212_586_230_785_595_55 * r) + (0.715_170_303_703_410_8 * g) + (0.072_200_498_643_336_23 * b);
    let z = (0.019_297_215_491_746_945 * r) + (0.119_183_864_580_848_54 * g) + (0.950_497_125_131_579_8 * b);
    [x, y, z]
}

fn rgb_to_cmy(red: f64, green: f64, blue: f64) -> [f64; 3] {
    let cyan = QUANTUM_SCALE * (QUANTUM_RANGE - red);
    let magenta = QUANTUM_SCALE * (QUANTUM_RANGE - green);
    let yellow = QUANTUM_SCALE * (QUANTUM_RANGE - blue);
    [cyan, magenta, yellow]
}

/// `ConvertRGBToHCL` and `ConvertRGBToHCLp` (the same forwards).
fn rgb_to_hcl(red: f64, green: f64, blue: f64) -> [f64; 3] {
    let max = magick_max(red, magick_max(green, blue));
    let c = max - magick_min(red, magick_min(green, blue));
    let mut h = 0.0;
    if c.abs() < MAGICK_EPSILON {
        h = 0.0;
    } else if (red - max).abs() < MAGICK_EPSILON {
        h = c_fmod((green - blue) / c + 6.0, 6.0);
    } else if (green - max).abs() < MAGICK_EPSILON {
        h = ((blue - red) / c) + 2.0;
    } else if (blue - max).abs() < MAGICK_EPSILON {
        h = ((red - green) / c) + 4.0;
    }
    let hue = h / 6.0;
    let chroma = QUANTUM_SCALE * c;
    let luma = QUANTUM_SCALE * (0.298839 * red + 0.586811 * green + 0.11435 * blue);
    [hue, chroma, luma]
}

/// C `fmod`.
fn c_fmod(x: f64, y: f64) -> f64 {
    x % y
}

/// `MagickMax` and `MagickMin` as IM writes them (`a > b ? a : b`).
fn magick_max(a: f64, b: f64) -> f64 {
    if a > b { a } else { b }
}

fn magick_min(a: f64, b: f64) -> f64 {
    if a < b { a } else { b }
}

fn rgb_to_hsb(red: f64, green: f64, blue: f64) -> [f64; 3] {
    let mut hue = 0.0;
    let mut min = if red < green { red } else { green };
    if blue < min {
        min = blue;
    }
    let mut max = if red > green { red } else { green };
    if blue > max {
        max = blue;
    }
    if max.abs() < MAGICK_EPSILON {
        return [0.0, 0.0, 0.0];
    }
    let delta = max - min;
    let saturation = delta / max;
    let brightness = QUANTUM_SCALE * max;
    if delta.abs() < MAGICK_EPSILON {
        return [hue, saturation, brightness];
    }
    if (red - max).abs() < MAGICK_EPSILON {
        hue = (green - blue) / delta;
    } else if (green - max).abs() < MAGICK_EPSILON {
        hue = 2.0 + (blue - red) / delta;
    } else {
        hue = 4.0 + (red - green) / delta;
    }
    hue /= 6.0;
    if hue < 0.0 {
        hue += 1.0;
    }
    [hue, saturation, brightness]
}

fn rgb_to_hsi(red: f64, green: f64, blue: f64) -> [f64; 3] {
    let intensity = (QUANTUM_SCALE * red + QUANTUM_SCALE * green + QUANTUM_SCALE * blue) / 3.0;
    if intensity <= 0.0 {
        return [0.0, 0.0, intensity];
    }
    let smallest = magick_min(QUANTUM_SCALE * red, magick_min(QUANTUM_SCALE * green, QUANTUM_SCALE * blue));
    let saturation = 1.0 - smallest / intensity;
    let alpha = 0.5 * (2.0 * QUANTUM_SCALE * red - QUANTUM_SCALE * green - QUANTUM_SCALE * blue);
    let beta = 0.866_025_403_784_438_5 * (QUANTUM_SCALE * green - QUANTUM_SCALE * blue);
    let mut hue = beta.atan2(alpha) * (180.0 / MAGICK_PI) / 360.0;
    if hue < 0.0 {
        hue += 1.0;
    }
    [hue, saturation, intensity]
}

fn rgb_to_hsl(red: f64, green: f64, blue: f64) -> [f64; 3] {
    let max = magick_max(QUANTUM_SCALE * red, magick_max(QUANTUM_SCALE * green, QUANTUM_SCALE * blue));
    let min = magick_min(QUANTUM_SCALE * red, magick_min(QUANTUM_SCALE * green, QUANTUM_SCALE * blue));
    let c = max - min;
    let lightness = (max + min) / 2.0;
    if c <= 0.0 {
        return [0.0, 0.0, lightness];
    }
    let mut hue;
    if (max - QUANTUM_SCALE * red).abs() < MAGICK_EPSILON {
        hue = (QUANTUM_SCALE * green - QUANTUM_SCALE * blue) / c;
        if (QUANTUM_SCALE * green) < (QUANTUM_SCALE * blue) {
            hue += 6.0;
        }
    } else if (max - QUANTUM_SCALE * green).abs() < MAGICK_EPSILON {
        hue = 2.0 + (QUANTUM_SCALE * blue - QUANTUM_SCALE * red) / c;
    } else {
        hue = 4.0 + (QUANTUM_SCALE * red - QUANTUM_SCALE * green) / c;
    }
    hue *= 60.0 / 360.0;
    let saturation = if lightness <= 0.5 { c * safe_reciprocal(2.0 * lightness) } else { c * safe_reciprocal(2.0 - 2.0 * lightness) };
    [hue, saturation, lightness]
}

fn rgb_to_hsv(red: f64, green: f64, blue: f64) -> [f64; 3] {
    let max = magick_max(QUANTUM_SCALE * red, magick_max(QUANTUM_SCALE * green, QUANTUM_SCALE * blue));
    let min = magick_min(QUANTUM_SCALE * red, magick_min(QUANTUM_SCALE * green, QUANTUM_SCALE * blue));
    let c = max - min;
    let value = max;
    if c <= 0.0 {
        return [0.0, 0.0, value];
    }
    let mut hue;
    if (max - QUANTUM_SCALE * red).abs() < MAGICK_EPSILON {
        hue = (QUANTUM_SCALE * green - QUANTUM_SCALE * blue) / c;
        if (QUANTUM_SCALE * green) < (QUANTUM_SCALE * blue) {
            hue += 6.0;
        }
    } else if (max - QUANTUM_SCALE * green).abs() < MAGICK_EPSILON {
        hue = 2.0 + (QUANTUM_SCALE * blue - QUANTUM_SCALE * red) / c;
    } else {
        hue = 4.0 + (QUANTUM_SCALE * red - QUANTUM_SCALE * green) / c;
    }
    hue *= 60.0 / 360.0;
    let saturation = c * safe_reciprocal(max);
    [hue, saturation, value]
}

fn rgb_to_hwb(red: f64, green: f64, blue: f64) -> [f64; 3] {
    let w = magick_min(red, magick_min(green, blue));
    let v = magick_max(red, magick_max(green, blue));
    let blackness = 1.0 - QUANTUM_SCALE * v;
    let whiteness = QUANTUM_SCALE * w;
    if (v - w).abs() < MAGICK_EPSILON {
        return [-1.0, whiteness, blackness];
    }
    let red_is_least = (red - w).abs() < MAGICK_EPSILON;
    let green_is_least = (green - w).abs() < MAGICK_EPSILON;
    let f = if red_is_least { green - blue } else if green_is_least { blue - red } else { red - green };
    let p = if red_is_least { 3.0 } else if green_is_least { 5.0 } else { 1.0 };
    let hue = (p - f / (v - 1.0 * w)) / 6.0;
    [hue, whiteness, blackness]
}

fn xyz_to_lab(x: f64, y: f64, z: f64) -> [f64; 3] {
    let cube_root = |value: f64, white: f64| -> f64 {
        if (value / white) > CIE_EPSILON {
            (value / white).powf(1.0 / 3.0)
        } else {
            (CIE_K * value / white + 16.0) / 116.0
        }
    };
    let fx = cube_root(x, ILLUMINANT[0]);
    let fy = cube_root(y, ILLUMINANT[1]);
    let fz = cube_root(z, ILLUMINANT[2]);
    let l = ((116.0 * fy) - 16.0) / 100.0;
    let a = (500.0 * (fx - fy)) / 255.0 + 0.5;
    let b = (200.0 * (fy - fz)) / 255.0 + 0.5;
    [l, a, b]
}

fn xyz_to_lch_ab(x: f64, y: f64, z: f64) -> [f64; 3] {
    let [luma, a, b] = xyz_to_lab(x, y, z);
    let chroma = (a - 0.5).hypot(b - 0.5) / 1.0 + 0.5;
    let mut hue = 180.0 * (b - 0.5).atan2(a - 0.5) / MAGICK_PI / 360.0;
    if hue < 0.0 {
        hue += 1.0;
    }
    [luma, chroma, hue]
}

/// `4·Xn/(Xn + 15·Yn + 3·Zn)` and `9·Yn/(…)` for the illuminant.
fn white_uv() -> (f64, f64) {
    let denominator = ILLUMINANT[0] + 15.0 * ILLUMINANT[1] + 3.0 * ILLUMINANT[2];
    (4.0 * ILLUMINANT[0] / denominator, 9.0 * ILLUMINANT[1] / denominator)
}

fn xyz_to_luv(x: f64, y: f64, z: f64) -> [f64; 3] {
    let mut l = if (y / ILLUMINANT[1]) > CIE_EPSILON { 116.0 * (y / ILLUMINANT[1]).powf(1.0 / 3.0) - 16.0 } else { CIE_K * (y / ILLUMINANT[1]) };
    let alpha = safe_reciprocal(x + 15.0 * y + 3.0 * z);
    let (white_u, white_v) = white_uv();
    let mut u = 13.0 * l * ((4.0 * alpha * x) - white_u);
    let mut v = 13.0 * l * ((9.0 * alpha * y) - white_v);
    l /= 100.0;
    u = (u + 134.0) / 354.0;
    v = (v + 140.0) / 262.0;
    [l, u, v]
}

fn xyz_to_lch_uv(x: f64, y: f64, z: f64) -> [f64; 3] {
    let [luma, u, v] = xyz_to_luv(x, y, z);
    let chroma = (354.0 * u - 134.0).hypot(262.0 * v - 140.0) / 255.0 + 0.5;
    let mut hue = 180.0 * (262.0 * v - 140.0).atan2(354.0 * u - 134.0) / MAGICK_PI / 360.0;
    if hue < 0.0 {
        hue += 1.0;
    }
    [luma, chroma, hue]
}

fn xyz_to_lms(x: f64, y: f64, z: f64) -> [f64; 3] {
    let l = 0.7328 * x + 0.4296 * y - 0.1624 * z;
    let m = -0.7036 * x + 1.6975 * y + 0.0061 * z;
    let s = 0.003 * x + 0.0136 * y + 0.9834 * z;
    [l, m, s]
}

/// The XYZ to companded RGB matrices of Adobe '98, Display P3 and
/// ProPhoto, each channel then gamma-encoded as sRGB's.
fn xyz_to_companded(x: f64, y: f64, z: f64, matrix: &[[f64; 3]; 3]) -> [f64; 3] {
    let mut companded = [0.0; 3];
    for (channel, row) in companded.iter_mut().zip(matrix.iter()) {
        let linear = row[0] * x + row[1] * y + row[2] * z;
        *channel = QUANTUM_SCALE * encode_pixel_gamma(QUANTUM_RANGE * linear);
    }
    companded
}

const XYZ_TO_ADOBE98: [[f64; 3]; 3] = [
    [2.041_587_903_810_746_5, -0.565_006_974_278_859_6, -0.344_731_350_778_329_56],
    [-0.969_243_636_280_879_5, 1.875_967_501_507_720_2, 0.041_555_057_407_175_57],
    [0.013_444_280_632_031_142, -0.118_362_392_231_018_38, 1.015_174_994_391_205_4],
];

const XYZ_TO_DISPLAY_P3: [[f64; 3]; 3] = [
    [2.493_496_911_941_425, -0.931_383_617_919_123_9, -0.402_710_784_450_716_84],
    [-0.829_488_969_561_574_7, 1.762_664_060_318_346_3, 0.023_624_685_841_943_577],
    [0.035_845_830_243_784_47, -0.076_172_389_268_041_82, 0.956_884_524_007_687_2],
];

const XYZ_TO_PROPHOTO: [[f64; 3]; 3] = [
    [1.345_798_973_102_828_1, -0.255_580_100_079_975_34, -0.051_106_285_067_534_01],
    [-0.544_622_493_902_834_7, 1.508_232_741_313_278_1, 0.020_536_032_391_479_73],
    [0.0, 0.0, 1.211_967_545_638_945_4],
];

fn xyz_to_jzazbz(x: f64, y: f64, z: f64, white_luminance: f64) -> [f64; 3] {
    const B: f64 = 1.15;
    const G: f64 = 0.66;
    const C1: f64 = 3424.0 / 4096.0;
    const C2: f64 = 2413.0 / 128.0;
    const C3: f64 = 2392.0 / 128.0;
    const N: f64 = 2610.0 / 16384.0;
    const P: f64 = 1.7 * 2523.0 / 32.0;
    const D: f64 = -0.56;
    const D0: f64 = 1.629_549_953_282_156_6e-11;
    let white = safe_reciprocal(white_luminance);
    let xp = z + B * (x - z);
    let yp = x + G * (y - x);
    let mut l = 0.014_648 * z;
    let mut m = 0.053_100_8 * z;
    let mut s = 0.668_479_9 * z;
    l += 0.414_789_72 * xp;
    m += -0.20151 * xp;
    s += -0.016_600_8 * xp;
    l += 0.579_999 * yp;
    m += 1.120_649 * yp;
    s += 0.2648 * yp;
    let gl = (l * white).powf(N);
    let gm = (m * white).powf(N);
    let gs = (s * white).powf(N);
    let nl = C1 + C2 * gl;
    let nm = C1 + C2 * gm;
    let ns = C1 + C2 * gs;
    let dl = 1.0 + C3 * gl;
    let dm = 1.0 + C3 * gm;
    let ds = 1.0 + C3 * gs;
    let lp = (nl / dl).powf(P);
    let mp = (nm / dm).powf(P);
    let sp = (ns / ds).powf(P);
    let iz = (lp + mp) * 0.5;
    let jdi = D * iz;
    let j = (jdi + iz) / (jdi + 1.0) - D0;
    let mut a = 0.5 + 3.524 * lp;
    let mut b = 0.5 + 0.199_076 * lp;
    a += -4.066_708 * mp;
    b += 1.096_799 * mp;
    a += 0.542_708 * sp;
    b += -1.295_875 * sp;
    let jz = if j.is_nan() { 0.0 } else { j };
    let az = if a.is_nan() { 0.5 } else { a };
    let bz = if b.is_nan() { 0.5 } else { b };
    [jz, az, bz]
}

const OKLAB_LR: f64 = 0.412_221_470_8;
const OKLAB_LG: f64 = 0.536_332_536_3;
const OKLAB_LB: f64 = 0.051_445_992_9;
const OKLAB_MR: f64 = 0.211_903_498_2;
const OKLAB_MG: f64 = 0.680_699_545_1;
const OKLAB_MB: f64 = 0.107_396_956_6;
const OKLAB_SR: f64 = 0.088_302_461_9;
const OKLAB_SG: f64 = 0.281_718_837_6;
const OKLAB_SB: f64 = 0.629_978_700_5;
const OKLAB_LL: f64 = 0.210_454_255_3;
const OKLAB_LM: f64 = 0.793_617_785;
const OKLAB_LS: f64 = -0.004_072_046_8;
const OKLAB_AL: f64 = 1.977_998_495_1;
const OKLAB_AM: f64 = -2.428_592_205;
const OKLAB_AS: f64 = 0.450_593_709_9;
const OKLAB_BL: f64 = 0.025_904_037_1;
const OKLAB_BM: f64 = 0.782_771_766_2;
const OKLAB_BS: f64 = -0.808_675_766;

fn rgb_to_oklab(red: f64, green: f64, blue: f64) -> [f64; 3] {
    let r = decode_pixel_gamma(red);
    let g = decode_pixel_gamma(green);
    let b = decode_pixel_gamma(blue);
    let l_r = OKLAB_LR * QUANTUM_SCALE;
    let l_g = OKLAB_LG * QUANTUM_SCALE;
    let l_b = OKLAB_LB * QUANTUM_SCALE;
    let m_r = OKLAB_MR * QUANTUM_SCALE;
    let m_g = OKLAB_MG * QUANTUM_SCALE;
    let m_b = OKLAB_MB * QUANTUM_SCALE;
    let s_r = OKLAB_SR * QUANTUM_SCALE;
    let s_g = OKLAB_SG * QUANTUM_SCALE;
    let s_b = OKLAB_SB * QUANTUM_SCALE;
    let l = (l_r * r + l_g * g + l_b * b).cbrt();
    let m = (m_r * r + m_g * g + m_b * b).cbrt();
    let s = (s_r * r + s_g * g + s_b * b).cbrt();
    let lightness = OKLAB_LL * l + OKLAB_LM * m + OKLAB_LS * s;
    let a = OKLAB_AL * l + OKLAB_AM * m + OKLAB_AS * s + 0.5;
    let b = OKLAB_BL * l + OKLAB_BM * m + OKLAB_BS * s + 0.5;
    [lightness, a, b]
}

fn rgb_to_oklch(red: f64, green: f64, blue: f64) -> [f64; 3] {
    let l = OKLAB_LR * QUANTUM_SCALE * decode_pixel_gamma(red) + OKLAB_LG * QUANTUM_SCALE * decode_pixel_gamma(green) + OKLAB_LB * QUANTUM_SCALE * decode_pixel_gamma(blue);
    let m = OKLAB_MR * QUANTUM_SCALE * decode_pixel_gamma(red) + OKLAB_MG * QUANTUM_SCALE * decode_pixel_gamma(green) + OKLAB_MB * QUANTUM_SCALE * decode_pixel_gamma(blue);
    let s = OKLAB_SR * QUANTUM_SCALE * decode_pixel_gamma(red) + OKLAB_SG * QUANTUM_SCALE * decode_pixel_gamma(green) + OKLAB_SB * QUANTUM_SCALE * decode_pixel_gamma(blue);
    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();
    let lightness = OKLAB_LL * l_ + OKLAB_LM * m_ + OKLAB_LS * s_;
    let a = OKLAB_AL * l_ + OKLAB_AM * m_ + OKLAB_AS * s_;
    let b = OKLAB_BL * l_ + OKLAB_BM * m_ + OKLAB_BS * s_;
    let chroma = (a * a + b * b).sqrt();
    let mut hue = b.atan2(a);
    if hue < 0.0 {
        hue += 2.0 * MAGICK_PI;
    }
    [lightness, chroma, hue]
}

fn rgb_to_ydbdr(red: f64, green: f64, blue: f64) -> [f64; 3] {
    let y = QUANTUM_SCALE * (0.298839 * red + 0.586811 * green + 0.11435 * blue);
    let db = QUANTUM_SCALE * (-0.45 * red - 0.883 * green + 1.333 * blue) + 0.5;
    let dr = QUANTUM_SCALE * (-1.333 * red + 1.116 * green + 0.217 * blue) + 0.5;
    [y, db, dr]
}

fn rgb_to_yiq(red: f64, green: f64, blue: f64) -> [f64; 3] {
    let y = QUANTUM_SCALE * (0.298839 * red + 0.586811 * green + 0.11435 * blue);
    let i = QUANTUM_SCALE * (0.595716 * red - 0.274453 * green - 0.321263 * blue) + 0.5;
    let q = QUANTUM_SCALE * (0.211456 * red - 0.522591 * green + 0.311135 * blue) + 0.5;
    [y, i, q]
}

/// `ConvertRGBToYPbPr` (and `ConvertRGBToYCbCr`, the same).
fn rgb_to_ypbpr(red: f64, green: f64, blue: f64) -> [f64; 3] {
    let y = QUANTUM_SCALE * (0.298839 * red + 0.586811 * green + 0.11435 * blue);
    let pb = QUANTUM_SCALE * ((-0.168_736_7) * red - 0.331264 * green + 0.5 * blue) + 0.5;
    let pr = QUANTUM_SCALE * (0.5 * red - 0.418688 * green - 0.081312 * blue) + 0.5;
    [y, pb, pr]
}

fn rgb_to_yuv(red: f64, green: f64, blue: f64) -> [f64; 3] {
    let y = QUANTUM_SCALE * (0.298839 * red + 0.586811 * green + 0.11435 * blue);
    let u = QUANTUM_SCALE * ((-0.147) * red - 0.289 * green + 0.436 * blue) + 0.5;
    let v = QUANTUM_SCALE * (0.615 * red - 0.515 * green - 0.1 * blue) + 0.5;
    [y, u, v]
}

/// `ConvertRGBToGeneric`: an sRGB pixel's quanta to the colourspace's
/// values, each nominally 0–1.
pub fn rgb_to_generic(colorspace: Colorspace, red: f64, green: f64, blue: f64) -> [f64; 3] {
    match colorspace {
        Colorspace::Adobe98 => {
            let [x, y, z] = rgb_to_xyz(red, green, blue);
            xyz_to_companded(x, y, z, &XYZ_TO_ADOBE98)
        }
        Colorspace::Cmy => rgb_to_cmy(red, green, blue),
        Colorspace::DisplayP3 => {
            let [x, y, z] = rgb_to_xyz(red, green, blue);
            xyz_to_companded(x, y, z, &XYZ_TO_DISPLAY_P3)
        }
        Colorspace::Hcl | Colorspace::Hclp => rgb_to_hcl(red, green, blue),
        Colorspace::Hsb => rgb_to_hsb(red, green, blue),
        Colorspace::Hsi => rgb_to_hsi(red, green, blue),
        Colorspace::Hsl => rgb_to_hsl(red, green, blue),
        Colorspace::Hsv => rgb_to_hsv(red, green, blue),
        Colorspace::Hwb => rgb_to_hwb(red, green, blue),
        Colorspace::Jzazbz => {
            // `ConvertRGBToJzazbz` passes blue and green swapped.
            let [x, y, z] = rgb_to_xyz(red, blue, green);
            xyz_to_jzazbz(x, y, z, WHITE_LUMINANCE)
        }
        Colorspace::Lab => {
            let [x, y, z] = rgb_to_xyz(red, green, blue);
            xyz_to_lab(x, y, z)
        }
        Colorspace::Lch | Colorspace::LchAb => {
            let [x, y, z] = rgb_to_xyz(red, green, blue);
            xyz_to_lch_ab(x, y, z)
        }
        Colorspace::LchUv => {
            let [x, y, z] = rgb_to_xyz(red, green, blue);
            xyz_to_lch_uv(x, y, z)
        }
        Colorspace::Lms => {
            let [x, y, z] = rgb_to_xyz(red, green, blue);
            xyz_to_lms(x, y, z)
        }
        Colorspace::Luv => {
            let [x, y, z] = rgb_to_xyz(red, green, blue);
            xyz_to_luv(x, y, z)
        }
        Colorspace::Oklab => rgb_to_oklab(red, green, blue),
        Colorspace::Oklch => rgb_to_oklch(red, green, blue),
        Colorspace::ProPhoto => {
            let [x, y, z] = rgb_to_xyz(red, green, blue);
            xyz_to_companded(x, y, z, &XYZ_TO_PROPHOTO)
        }
        Colorspace::XyY => {
            let [x, y, z] = rgb_to_xyz(red, green, blue);
            let gamma = safe_reciprocal(x + y + z);
            [gamma * x, gamma * y, y]
        }
        Colorspace::Xyz => rgb_to_xyz(red, green, blue),
        Colorspace::YCbCr | Colorspace::YPbPr => rgb_to_ypbpr(red, green, blue),
        Colorspace::YDbDr => rgb_to_ydbdr(red, green, blue),
        Colorspace::Yiq => rgb_to_yiq(red, green, blue),
        Colorspace::Yuv => rgb_to_yuv(red, green, blue),
        _ => [QUANTUM_SCALE * red, QUANTUM_SCALE * green, QUANTUM_SCALE * blue],
    }
}

// --- back to sRGB (`Convert…ToRGB`, quanta out) --------------------------------

fn xyz_to_rgb(x: f64, y: f64, z: f64) -> [f64; 3] {
    let mut r = (3.240_969_941_904_521 * x) + (-1.537_383_177_570_093 * y) + (-0.498_610_760_293 * z);
    let mut g = (-0.969_243_636_280_87 * x) + (1.875_967_501_507_72 * y) + (0.041_555_057_407_175 * z);
    let mut b = (0.055_630_079_696_993 * x) + (-0.203_976_958_888_97 * y) + (1.056_971_514_242_878 * z);
    let min = magick_min(r, magick_min(g, b));
    if min < 0.0 {
        r -= min;
        g -= min;
        b -= min;
    }
    [encode_pixel_gamma(QUANTUM_RANGE * r), encode_pixel_gamma(QUANTUM_RANGE * g), encode_pixel_gamma(QUANTUM_RANGE * b)]
}

/// Adobe '98, Display P3 and ProPhoto (companded) to XYZ.
fn companded_to_xyz(red: f64, green: f64, blue: f64, matrix: &[[f64; 3]; 3]) -> [f64; 3] {
    let r = QUANTUM_SCALE * decode_pixel_gamma(QUANTUM_RANGE * red);
    let g = QUANTUM_SCALE * decode_pixel_gamma(QUANTUM_RANGE * green);
    let b = QUANTUM_SCALE * decode_pixel_gamma(QUANTUM_RANGE * blue);
    let mut xyz = [0.0; 3];
    for (channel, row) in xyz.iter_mut().zip(matrix.iter()) {
        *channel = row[0] * r + row[1] * g + row[2] * b;
    }
    xyz
}

const ADOBE98_TO_XYZ: [[f64; 3]; 3] = [
    [0.576_669_042_910_130_5, 0.185_558_237_906_546_3, 0.188_228_646_234_994_7],
    [0.297_344_975_250_536_05, 0.627_363_566_255_466_1, 0.075_291_458_493_997_88],
    [0.027_031_361_386_412_34, 0.070_688_852_535_827_23, 0.991_337_536_837_638_8],
];

const DISPLAY_P3_TO_XYZ: [[f64; 3]; 3] = [
    [0.486_570_948_648_216_2, 0.265_667_693_169_093_06, 0.198_217_285_234_362_5],
    [0.228_974_564_069_748_8, 0.691_738_521_836_506_4, 0.079_286_914_093_745],
    [0.0, 0.045_113_381_858_902_64, 1.043_944_368_900_976],
];

/// `ConvertProPhotoToXYZ` (its first X, overwritten in IM, is left out).
const PROPHOTO_TO_XYZ: [[f64; 3]; 3] = [
    [0.797_760_489_672_302_7, 0.135_185_837_175_740_31, 0.031_349_349_581_524_8],
    [0.288_071_128_229_293_4, 0.711_843_217_810_101_4, 8.565_396_060_525_902e-05],
    [0.0, 0.0, 0.825_104_602_510_460_1],
];

/// `ConvertHCLToRGB`'s and `ConvertHCLpToRGB`'s shared start: the hue's
/// sector, red, green and blue before the luma is added.
fn hue_chroma_rgb(hue: f64, chroma: f64) -> [f64; 3] {
    let h = 6.0 * hue;
    let c = chroma;
    let x = c * (1.0 - (c_fmod(h, 2.0) - 1.0).abs());
    if (0.0..1.0).contains(&h) {
        return [c, x, 0.0];
    }
    if (1.0..2.0).contains(&h) {
        return [x, c, 0.0];
    }
    if (2.0..3.0).contains(&h) {
        return [0.0, c, x];
    }
    if (3.0..4.0).contains(&h) {
        return [0.0, x, c];
    }
    if (4.0..5.0).contains(&h) {
        return [x, 0.0, c];
    }
    if (5.0..6.0).contains(&h) {
        return [c, 0.0, x];
    }
    [0.0, 0.0, 0.0]
}

fn hcl_to_rgb(hue: f64, chroma: f64, luma: f64) -> [f64; 3] {
    let [r, g, b] = hue_chroma_rgb(hue, chroma);
    let m = luma - (0.298839 * r + 0.586811 * g + 0.11435 * b);
    [QUANTUM_RANGE * (r + m), QUANTUM_RANGE * (g + m), QUANTUM_RANGE * (b + m)]
}

fn hclp_to_rgb(hue: f64, chroma: f64, luma: f64) -> [f64; 3] {
    let [r, g, b] = hue_chroma_rgb(hue, chroma);
    let c = chroma;
    let mut m = luma - (0.298839 * r + 0.586811 * g + 0.11435 * b);
    let mut z = 1.0;
    if m < 0.0 {
        z = luma / (luma - m);
        m = 0.0;
    } else if m + c > 1.0 {
        z = (1.0 - luma) / (m + c - luma);
        m = 1.0 - z * c;
    }
    [QUANTUM_RANGE * (z * r + m), QUANTUM_RANGE * (z * g + m), QUANTUM_RANGE * (z * b + m)]
}

fn hsb_to_rgb(hue: f64, saturation: f64, brightness: f64) -> [f64; 3] {
    if saturation.abs() < MAGICK_EPSILON {
        let gray = QUANTUM_RANGE * brightness;
        return [gray, gray, gray];
    }
    let h = 6.0 * (hue - hue.floor());
    let f = h - h.floor();
    let p = brightness * (1.0 - saturation);
    let q = brightness * (1.0 - saturation * f);
    let t = brightness * (1.0 - (saturation * (1.0 - f)));
    let (r, g, b) = match h as i32 {
        1 => (q, brightness, p),
        2 => (p, brightness, t),
        3 => (p, q, brightness),
        4 => (t, p, brightness),
        5 => (brightness, p, q),
        _ => (brightness, t, p),
    };
    [QUANTUM_RANGE * r, QUANTUM_RANGE * g, QUANTUM_RANGE * b]
}

fn hsi_to_rgb(hue: f64, saturation: f64, intensity: f64) -> [f64; 3] {
    let mut h = 360.0 * hue;
    h -= 360.0 * (h / 360.0).floor();
    let degrees = MAGICK_PI / 180.0;
    let (r, g, b);
    if h < 120.0 {
        b = intensity * (1.0 - saturation);
        r = intensity * (1.0 + saturation * (h * degrees).cos() / ((60.0 - h) * degrees).cos());
        g = 3.0 * intensity - r - b;
    } else if h < 240.0 {
        h -= 120.0;
        r = intensity * (1.0 - saturation);
        g = intensity * (1.0 + saturation * (h * degrees).cos() / ((60.0 - h) * degrees).cos());
        b = 3.0 * intensity - r - g;
    } else {
        h -= 240.0;
        g = intensity * (1.0 - saturation);
        b = intensity * (1.0 + saturation * (h * degrees).cos() / ((60.0 - h) * degrees).cos());
        r = 3.0 * intensity - g - b;
    }
    [QUANTUM_RANGE * r, QUANTUM_RANGE * g, QUANTUM_RANGE * b]
}

/// `ConvertHSLToRGB` and `ConvertHSVToRGB` after their chroma and
/// minimum: the hue's sector picks the order.
fn sector_rgb(hue: f64, c: f64, min: f64) -> [f64; 3] {
    let mut h = hue * 360.0;
    h -= 360.0 * (h / 360.0).floor();
    h /= 60.0;
    let x = c * (1.0 - (h - 2.0 * (h / 2.0).floor() - 1.0).abs());
    let (r, g, b) = match h.floor() as i32 {
        1 => (min + x, min + c, min),
        2 => (min, min + c, min + x),
        3 => (min, min + x, min + c),
        4 => (min + x, min, min + c),
        5 => (min + c, min, min + x),
        _ => (min + c, min + x, min),
    };
    [QUANTUM_RANGE * r, QUANTUM_RANGE * g, QUANTUM_RANGE * b]
}

fn hsl_to_rgb(hue: f64, saturation: f64, lightness: f64) -> [f64; 3] {
    let c = if lightness <= 0.5 { 2.0 * lightness * saturation } else { (2.0 - 2.0 * lightness) * saturation };
    let min = lightness - 0.5 * c;
    sector_rgb(hue, c, min)
}

fn hsv_to_rgb(hue: f64, saturation: f64, value: f64) -> [f64; 3] {
    let c = value * saturation;
    let min = value - c;
    sector_rgb(hue, c, min)
}

fn hwb_to_rgb(hue: f64, whiteness: f64, blackness: f64) -> [f64; 3] {
    let v = 1.0 - blackness;
    if (hue - (-1.0)).abs() < MAGICK_EPSILON {
        return [QUANTUM_RANGE * v, QUANTUM_RANGE * v, QUANTUM_RANGE * v];
    }
    let i = (6.0 * hue).floor() as i64;
    let mut f = 6.0 * hue - i as f64;
    if (i & 0x01) != 0 {
        f = 1.0 - f;
    }
    let n = whiteness + f * (v - whiteness);
    let (r, g, b) = match i {
        1 => (n, v, whiteness),
        2 => (whiteness, v, n),
        3 => (whiteness, n, v),
        4 => (n, whiteness, v),
        5 => (v, whiteness, n),
        _ => (v, n, whiteness),
    };
    [QUANTUM_RANGE * r, QUANTUM_RANGE * g, QUANTUM_RANGE * b]
}

fn lab_to_xyz(l: f64, a: f64, b: f64) -> [f64; 3] {
    let mut y = (l + 16.0) / 116.0;
    let mut x = y + a / 500.0;
    let mut z = y - b / 200.0;
    if (x * x * x) > CIE_EPSILON {
        x = x * x * x;
    } else {
        x = (116.0 * x - 16.0) / CIE_K;
    }
    if l > (CIE_K * CIE_EPSILON) {
        y = y * y * y;
    } else {
        y = l / CIE_K;
    }
    if (z * z * z) > CIE_EPSILON {
        z = z * z * z;
    } else {
        z = (116.0 * z - 16.0) / CIE_K;
    }
    [ILLUMINANT[0] * x, ILLUMINANT[1] * y, ILLUMINANT[2] * z]
}

fn luv_to_xyz(l: f64, u: f64, v: f64) -> [f64; 3] {
    let y = if l > (CIE_K * CIE_EPSILON) { ((l + 16.0) / 116.0).powf(3.0) } else { l / CIE_K };
    let (white_u, white_v) = white_uv();
    let gamma = safe_reciprocal((((52.0 * l * safe_reciprocal(u + 13.0 * l * white_u)) - 1.0) / 3.0) - (-1.0 / 3.0));
    let x = gamma * ((y * ((39.0 * l * safe_reciprocal(v + 13.0 * l * white_v)) - 5.0)) + 5.0 * y);
    let z = (x * (((52.0 * l * safe_reciprocal(u + 13.0 * l * white_u)) - 1.0) / 3.0)) - 5.0 * y;
    [x, y, z]
}

fn lms_to_xyz(l: f64, m: f64, s: f64) -> [f64; 3] {
    let x = 1.096_123_820_835_514 * l - 0.278_869_000_218_287 * m + 0.182_745_179_382_773 * s;
    let y = 0.454_369_041_975_359 * l + 0.473_533_154_307_412 * m + 0.072_097_803_717_229 * s;
    let z = (-0.009_627_608_738_429) * l - 0.005_698_031_216_113 * m + 1.015_325_639_954_543 * s;
    [x, y, z]
}

fn jzazbz_to_xyz(jz: f64, az: f64, bz: f64, white_luminance: f64) -> [f64; 3] {
    const B: f64 = 1.15;
    const G: f64 = 0.66;
    const C1: f64 = 3424.0 / 4096.0;
    const C2: f64 = 2413.0 / 128.0;
    const MINUS_C3: f64 = -2392.0 / 128.0;
    const N: f64 = 2610.0 / 16384.0;
    const P: f64 = 1.7 * 2523.0 / 32.0;
    const D: f64 = -0.56;
    const D0: f64 = 1.629_549_953_282_156_6e-11;
    let g = jz + D0;
    let azz = az - 0.5;
    let bzz = bz - 0.5;
    let c = 0.138_605_043_271_539 * azz + 0.058_047_316_156_118_9 * bzz;
    let mut sp = g / (1.0 + D * (1.0 - g));
    let lp = sp + c;
    let mp = sp - c;
    sp += -0.096_019_242_026_318_9 * azz;
    sp += -0.811_891_896_056_039 * bzz;
    let jpr = 1.0 / P;
    let gl = lp.powf(jpr);
    let gm = mp.powf(jpr);
    let gs = sp.powf(jpr);
    let jnr = 1.0 / N;
    let nl = gl - C1;
    let nm = gm - C1;
    let ns = gs - C1;
    let dl = C2 + MINUS_C3 * gl;
    let dm = C2 + MINUS_C3 * gm;
    let ds = C2 + MINUS_C3 * gs;
    let mut l = (nl / dl).powf(jnr);
    let mut m = (nm / dm).powf(jnr);
    let mut s = (ns / ds).powf(jnr);
    l *= white_luminance;
    m *= white_luminance;
    s *= white_luminance;
    let mut zp = -0.090_982_810_982_847_6 * l;
    let mut xp = 1.924_226_435_787_61 * l;
    let mut yp = 0.350_316_762_094_999 * l;
    zp += -0.312_728_290_523_074 * m;
    xp += -1.004_792_312_595_37 * m;
    yp += 0.726_481_193_931_655 * m;
    zp += 1.522_766_561_305_26 * s;
    xp += 0.037_651_404_030_618 * s;
    yp += -0.065_384_422_948_085 * s;
    zp = if zp.is_nan() { 0.0 } else { zp };
    xp = zp + (xp - zp) / B;
    xp = if xp.is_nan() { 0.0 } else { xp };
    yp = xp + (yp - xp) / G;
    yp = if yp.is_nan() { 0.0 } else { yp };
    [xp, yp, zp]
}

const OKLAB_LA: f64 = 0.396_337_777_4;
const OKLAB_LB_: f64 = 0.215_803_757_3;
const OKLAB_MA: f64 = -0.105_561_345_8;
const OKLAB_MB_: f64 = -0.063_854_172_8;
const OKLAB_SA: f64 = -0.089_484_177_5;
const OKLAB_SB_: f64 = -1.291_485_548;
const OKLAB_RL: f64 = 4.076_741_662_1;
const OKLAB_RM: f64 = -3.307_711_591_3;
const OKLAB_RS: f64 = 0.230_969_929_2;
const OKLAB_GL: f64 = -1.268_438_004_6;
const OKLAB_GM: f64 = 2.609_757_401_1;
const OKLAB_GS: f64 = -0.341_319_396_5;
const OKLAB_BL_: f64 = -0.004_196_086_3;
const OKLAB_BM_: f64 = -0.703_418_614_7;
const OKLAB_BS_: f64 = 1.707_614_701;

fn oklab_to_rgb(lightness: f64, a: f64, b: f64) -> [f64; 3] {
    let aa = a - 0.5;
    let bb = b - 0.5;
    let l = lightness + OKLAB_LA * aa + OKLAB_LB_ * bb;
    let m = lightness + OKLAB_MA * aa + OKLAB_MB_ * bb;
    let s = lightness + OKLAB_SA * aa + OKLAB_SB_ * bb;
    // IM's `l*=l*l`: the square first.
    let l = l * (l * l);
    let m = m * (m * m);
    let s = s * (s * s);
    let r_l = OKLAB_RL * QUANTUM_RANGE;
    let r_m = OKLAB_RM * QUANTUM_RANGE;
    let r_s = OKLAB_RS * QUANTUM_RANGE;
    let g_l = OKLAB_GL * QUANTUM_RANGE;
    let g_m = OKLAB_GM * QUANTUM_RANGE;
    let g_s = OKLAB_GS * QUANTUM_RANGE;
    let b_l = OKLAB_BL_ * QUANTUM_RANGE;
    let b_m = OKLAB_BM_ * QUANTUM_RANGE;
    let b_s = OKLAB_BS_ * QUANTUM_RANGE;
    let red = r_l * l + r_m * m + r_s * s;
    let green = g_l * l + g_m * m + g_s * s;
    let blue = b_l * l + b_m * m + b_s * s;
    [encode_pixel_gamma(red), encode_pixel_gamma(green), encode_pixel_gamma(blue)]
}

fn oklch_to_rgb(lightness: f64, chroma: f64, hue: f64) -> [f64; 3] {
    let a = chroma * hue.cos();
    let b = chroma * hue.sin();
    let l_ = lightness + OKLAB_LA * a + OKLAB_LB_ * b;
    let m_ = lightness + OKLAB_MA * a + OKLAB_MB_ * b;
    let s_ = lightness + OKLAB_SA * a + OKLAB_SB_ * b;
    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;
    let red = encode_pixel_gamma(QUANTUM_RANGE * (OKLAB_RL * l + OKLAB_RM * m + OKLAB_RS * s));
    let green = encode_pixel_gamma(QUANTUM_RANGE * (OKLAB_GL * l + OKLAB_GM * m + OKLAB_GS * s));
    let blue = encode_pixel_gamma(QUANTUM_RANGE * (OKLAB_BL_ * l + OKLAB_BM_ * m + OKLAB_BS_ * s));
    [red, green, blue]
}

fn ypbpr_to_rgb(y: f64, pb: f64, pr: f64) -> [f64; 3] {
    let red = QUANTUM_RANGE * (0.999_999_999_999_146_8 * y - 1.218_894_188_714_587_5e-06 * (pb - 0.5) + 1.401_999_588_656_144 * (pr - 0.5));
    let green = QUANTUM_RANGE * (0.999_999_759_105_025_1 * y - 0.344_135_678_165_043_04 * (pb - 0.5) - 0.714_136_493_316_467_9 * (pr - 0.5));
    let blue = QUANTUM_RANGE * (1.000_001_240_400_046_2 * y + 1.772_000_066_072_304 * (pb - 0.5) + 2.145_338_417_459_327_3e-06 * (pr - 0.5));
    [red, green, blue]
}

fn ydbdr_to_rgb(y: f64, db: f64, dr: f64) -> [f64; 3] {
    let red = QUANTUM_RANGE * (y + 9.230_371_614_765_7e-05 * (db - 0.5) - 0.525_912_630_661_865_3 * (dr - 0.5));
    let green = QUANTUM_RANGE * (y - 0.129_132_898_890_509_27 * (db - 0.5) + 0.267_899_328_207_598_76 * (dr - 0.5));
    let blue = QUANTUM_RANGE * (y + 0.664_679_059_978_954_8 * (db - 0.5) - 7.920_254_353_310_8e-05 * (dr - 0.5));
    [red, green, blue]
}

fn yiq_to_rgb(y: f64, i: f64, q: f64) -> [f64; 3] {
    let red = QUANTUM_RANGE * (y + 0.956_295_719_758_948_2 * (i - 0.5) + 0.621_024_416_465_261_1 * (q - 0.5));
    let green = QUANTUM_RANGE * (y - 0.272_122_099_318_510_45 * (i - 0.5) - 0.647_380_596_825_695 * (q - 0.5));
    let blue = QUANTUM_RANGE * (y - 1.106_989_016_736_490_2 * (i - 0.5) + 1.704_614_998_364_648_1 * (q - 0.5));
    [red, green, blue]
}

fn yuv_to_rgb(y: f64, u: f64, v: f64) -> [f64; 3] {
    let red = QUANTUM_RANGE * (y - 3.945_707_070_708_279e-05 * (u - 0.5) + 1.139_827_967_171_717 * (v - 0.5));
    let green = QUANTUM_RANGE * (y - 0.394_610_164_141_414_14 * (u - 0.5) - 0.580_500_315_656_565_7 * (v - 0.5));
    let blue = QUANTUM_RANGE * (y + 2.031_999_684_343_434_3 * (u - 0.5) - 4.813_762_626_262_513e-04 * (v - 0.5));
    [red, green, blue]
}

/// `ConvertGenericToRGB`: a colourspace's values (nominally 0–1) to sRGB
/// quanta.
pub fn generic_to_rgb(colorspace: Colorspace, x: f64, y: f64, z: f64) -> [f64; 3] {
    match colorspace {
        Colorspace::Adobe98 => {
            let [cx, cy, cz] = companded_to_xyz(x, y, z, &ADOBE98_TO_XYZ);
            xyz_to_rgb(cx, cy, cz)
        }
        Colorspace::Cmy => [QUANTUM_RANGE * (1.0 - x), QUANTUM_RANGE * (1.0 - y), QUANTUM_RANGE * (1.0 - z)],
        Colorspace::DisplayP3 => {
            let [cx, cy, cz] = companded_to_xyz(x, y, z, &DISPLAY_P3_TO_XYZ);
            xyz_to_rgb(cx, cy, cz)
        }
        Colorspace::Hcl => hcl_to_rgb(x, y, z),
        Colorspace::Hclp => hclp_to_rgb(x, y, z),
        Colorspace::Hsb => hsb_to_rgb(x, y, z),
        Colorspace::Hsi => hsi_to_rgb(x, y, z),
        Colorspace::Hsl => hsl_to_rgb(x, y, z),
        Colorspace::Hsv => hsv_to_rgb(x, y, z),
        Colorspace::Hwb => hwb_to_rgb(x, y, z),
        Colorspace::Jzazbz => {
            let [cx, cy, cz] = jzazbz_to_xyz(x, y, z, WHITE_LUMINANCE);
            // `ConvertJzazbzToRGB` takes blue and green back swapped.
            let [red, blue, green] = xyz_to_rgb(cx, cy, cz);
            [red, green, blue]
        }
        Colorspace::Lab => {
            let [cx, cy, cz] = lab_to_xyz(100.0 * x, 255.0 * (y - 0.5), 255.0 * (z - 0.5));
            xyz_to_rgb(cx, cy, cz)
        }
        Colorspace::Lch | Colorspace::LchAb => {
            let luma = 100.0 * x;
            let chroma = 255.0 * (y - 0.5);
            let hue = 360.0 * z;
            let [cx, cy, cz] = lab_to_xyz(luma, chroma * degrees_to_radians(hue).cos(), chroma * degrees_to_radians(hue).sin());
            xyz_to_rgb(cx, cy, cz)
        }
        Colorspace::LchUv => {
            let luma = 100.0 * x;
            let chroma = 255.0 * (y - 0.5);
            let hue = 360.0 * z;
            let [cx, cy, cz] = luv_to_xyz(luma, chroma * degrees_to_radians(hue).cos(), chroma * degrees_to_radians(hue).sin());
            xyz_to_rgb(cx, cy, cz)
        }
        Colorspace::Lms => {
            let [cx, cy, cz] = lms_to_xyz(x, y, z);
            xyz_to_rgb(cx, cy, cz)
        }
        Colorspace::Luv => {
            let [cx, cy, cz] = luv_to_xyz(100.0 * x, 354.0 * y - 134.0, 262.0 * z - 140.0);
            xyz_to_rgb(cx, cy, cz)
        }
        Colorspace::Oklab => oklab_to_rgb(x, y, z),
        Colorspace::Oklch => oklch_to_rgb(x, y, z),
        Colorspace::ProPhoto => {
            let [cx, cy, cz] = companded_to_xyz(x, y, z, &PROPHOTO_TO_XYZ);
            xyz_to_rgb(cx, cy, cz)
        }
        Colorspace::XyY => {
            let gamma = safe_reciprocal(y);
            let cx = gamma * z * x;
            let cz = gamma * z * (1.0 - x - y);
            xyz_to_rgb(cx, z, cz)
        }
        Colorspace::Xyz => xyz_to_rgb(x, y, z),
        Colorspace::YCbCr | Colorspace::YPbPr => ypbpr_to_rgb(x, y, z),
        Colorspace::YDbDr => ydbdr_to_rgb(x, y, z),
        Colorspace::Yiq => yiq_to_rgb(x, y, z),
        Colorspace::Yuv => yuv_to_rgb(x, y, z),
        _ => [QUANTUM_RANGE * x, QUANTUM_RANGE * y, QUANTUM_RANGE * z],
    }
}

/// `ConvertRGBToCMYK` on a pixel of an image already labelled CMYK (so
/// without gamma decoding): cyan, magenta, yellow and black quanta.
pub fn rgb_to_cmyk(red: f64, green: f64, blue: f64) -> [f64; 4] {
    let r = QUANTUM_SCALE * red;
    let g = QUANTUM_SCALE * green;
    let b = QUANTUM_SCALE * blue;
    if r.abs() < MAGICK_EPSILON && g.abs() < MAGICK_EPSILON && b.abs() < MAGICK_EPSILON {
        // IM sets only black: cyan, magenta and yellow keep the pixel's
        // red, green and blue.
        return [red, green, blue, QUANTUM_RANGE];
    }
    let mut cyan = 1.0 - r;
    let mut magenta = 1.0 - g;
    let mut yellow = 1.0 - b;
    let mut black = cyan;
    if magenta < black {
        black = magenta;
    }
    if yellow < black {
        black = yellow;
    }
    cyan = safe_reciprocal(1.0 - black) * (cyan - black);
    magenta = safe_reciprocal(1.0 - black) * (magenta - black);
    yellow = safe_reciprocal(1.0 - black) * (yellow - black);
    [QUANTUM_RANGE * cyan, QUANTUM_RANGE * magenta, QUANTUM_RANGE * yellow, QUANTUM_RANGE * black]
}

/// `ConvertCMYKToRGB`.
pub fn cmyk_to_rgb(cyan: f64, magenta: f64, yellow: f64, black: f64) -> [f64; 3] {
    let red = QUANTUM_RANGE - (QUANTUM_SCALE * cyan * (QUANTUM_RANGE - black) + black);
    let green = QUANTUM_RANGE - (QUANTUM_SCALE * magenta * (QUANTUM_RANGE - black) + black);
    let blue = QUANTUM_RANGE - (QUANTUM_SCALE * yellow * (QUANTUM_RANGE - black) + black);
    [red, green, blue]
}

// --- table conversions ------------------------------------------------------------

/// `TransformPacket` tables: what each map level of red, green and blue
/// adds to the three results, plus `primary_info`.
pub struct TransformTables {
    x: Vec<[f64; 3]>,
    y: Vec<[f64; 3]>,
    z: Vec<[f64; 3]>,
    primary: [f64; 3],
}

impl TransformTables {
    fn filled(build: impl Fn(f64) -> [[f64; 3]; 3]) -> Self {
        let levels = MAX_MAP as usize + 1;
        let mut x = Vec::with_capacity(levels);
        let mut y = Vec::with_capacity(levels);
        let mut z = Vec::with_capacity(levels);
        for level in 0..levels {
            let [x_entry, y_entry, z_entry] = build(level as f64);
            x.push(x_entry);
            y.push(y_entry);
            z.push(z_entry);
        }
        Self { x, y, z, primary: [0.0; 3] }
    }

    /// The three sums for a pixel's map levels (`x_map[red].x+y_map[green].x+z_map[blue].x`, …).
    pub fn sums(&self, red: usize, green: usize, blue: usize) -> [f64; 3] {
        let mut sums = [0.0; 3];
        for (index, sum) in sums.iter_mut().enumerate() {
            *sum = (self.x[red][index] + self.y[green][index] + self.z[blue][index]) + self.primary[index];
        }
        sums
    }

    /// `sRGBTransformImage`'s tables for OHTA, the YCbCr of Rec. 601 and
    /// 709 and Photo YCC (identity for anything else).
    pub fn from_srgb(colorspace: Colorspace) -> Self {
        // `(MaxMap+1)/2`.
        let half = 32768.0;
        match colorspace {
            Colorspace::Ohta => {
                let mut tables = Self::filled(|i| [[0.33333 * i, 0.5 * i, -0.25 * i], [0.33334 * i, 0.0 * i, 0.5 * i], [0.33333 * i, -0.5 * i, -0.25 * i]]);
                tables.primary = [0.0, half, half];
                tables
            }
            Colorspace::Rec601YCbCr => {
                let mut tables = Self::filled(|i| [[0.298839 * i, -0.168_736_7 * i, 0.5 * i], [0.586811 * i, -0.331264 * i, -0.418688 * i], [0.11435 * i, 0.5 * i, -0.081312 * i]]);
                tables.primary = [0.0, half, half];
                tables
            }
            Colorspace::Rec709YCbCr => {
                let mut tables = Self::filled(|i| [[0.212656 * i, -0.114572 * i, 0.5 * i], [0.715158 * i, -0.385428 * i, -0.454153 * i], [0.072186 * i, 0.5 * i, -0.045847 * i]]);
                tables.primary = [0.0, half, half];
                tables
            }
            Colorspace::Ycc => {
                let linear_end = (0.018 * MAX_MAP) as usize;
                let mut tables = Self::filled(|i| {
                    if (i as usize) <= linear_end {
                        return [[0.005382 * i, (-0.003296) * i, 0.00941 * i], [0.010566 * i, (-0.006471) * i, (-0.00788) * i], [0.002052 * i, 0.009768 * i, (-0.00153) * i]];
                    }
                    let v = 1.099 * i - 0.099;
                    [[0.298839 * v, (-0.298839) * v, 0.701 * v], [0.586811 * v, (-0.586811) * v, (-0.586811) * v], [0.11435 * v, 0.886 * v, (-0.11435) * v]]
                });
                tables.primary = [0.0, quantum_to_map(char_to_quantum(156)) as f64, quantum_to_map(char_to_quantum(137)) as f64];
                tables
            }
            _ => Self::filled(|i| [[i, 0.0, 0.0], [0.0, i, 0.0], [0.0, 0.0, i]]),
        }
    }

    /// `TransformsRGBImage`'s tables.
    pub fn to_srgb(colorspace: Colorspace) -> Self {
        match colorspace {
            Colorspace::Ohta => Self::filled(|i| {
                let centred = 2.0 * i - MAX_MAP;
                [[1.0 * i, 1.0 * i, 1.0 * i], [0.5 * 1.0 * centred, 0.5 * 0.0 * centred, -0.5 * 1.0 * centred], [-0.5 * 0.66668 * centred, 0.5 * 1.33333 * centred, -0.5 * 0.66668 * centred]]
            }),
            Colorspace::Rec601YCbCr => Self::filled(|i| {
                let centred = 2.0 * i - MAX_MAP;
                [
                    [0.999_999_999_999_146_8 * i, 0.999_999_759_105_025_1 * i, 1.000_001_240_400_046_2 * i],
                    [0.5 * (-1.218_894_188_714_587_5e-06) * centred, 0.5 * (-0.344_135_678_165_043_04) * centred, 0.5 * 1.772_000_066_072_304 * centred],
                    [0.5 * 1.401_999_588_656_144 * centred, 0.5 * (-0.714_136_493_316_467_9) * centred, 0.5 * 2.145_338_417_459_327_3e-06 * centred],
                ]
            }),
            Colorspace::Rec709YCbCr => Self::filled(|i| {
                let centred = 2.0 * i - MAX_MAP;
                [[1.0 * i, 1.0 * i, 1.0 * i], [0.5 * 0.0 * centred, 0.5 * (-0.187324) * centred, 0.5 * 1.8556 * centred], [0.5 * 1.5748 * centred, 0.5 * (-0.468124) * centred, 0.5 * 0.0 * centred]]
            }),
            Colorspace::Ycc => {
                let c1_zero = quantum_to_map(char_to_quantum(156)) as f64;
                let c2_zero = quantum_to_map(char_to_quantum(137)) as f64;
                Self::filled(|i| {
                    let c1 = 1.0 * i - c1_zero;
                    let c2 = 1.0 * i - c2_zero;
                    [[1.3584 * i, 1.3584 * i, 1.3584 * i], [0.0, -0.430_272_6 * c1, 2.2179 * c1], [1.8215 * c2, -0.927_143_5 * c2, 0.0]]
                })
            }
            _ => Self::filled(|i| [[i, 0.0, 0.0], [0.0, i, 0.0], [0.0, 0.0, i]]),
        }
    }
}

/// `RoundToYCC`.
fn round_to_ycc(value: f64) -> usize {
    if value <= 0.0 {
        return 0;
    }
    if value >= 1388.0 {
        return 1388;
    }
    (value + 0.5) as usize
}

/// `TransformsRGBImage`'s Photo YCC tone curve on a table sum.
pub fn ycc_curve(sum: f64) -> f64 {
    let level = round_to_ycc(MAXIMUM_LOGARITHMIC_COLORSPACE * sum / MAX_MAP);
    QUANTUM_RANGE * YCC_MAP[level] as f64
}

// --- Log -------------------------------------------------------------------------

const DISPLAY_GAMMA: f64 = 1.0 / 1.7;
const FILM_GAMMA: f64 = 0.6;
const REFERENCE_BLACK: f64 = 95.0;
const REFERENCE_WHITE: f64 = 685.0;

/// `sRGBTransformImage`'s `logmap` (Cineon log) for the default film
/// settings.
pub fn log_map_from_srgb() -> Vec<f32> {
    let density = DISPLAY_GAMMA;
    let gamma = DISPLAY_GAMMA;
    let black = 10.0f64.powf((REFERENCE_BLACK - REFERENCE_WHITE) * (gamma / density) * 0.002 * safe_reciprocal(FILM_GAMMA));
    let mut map = Vec::with_capacity(MAX_MAP as usize + 1);
    for i in 0..=MAX_MAP as usize {
        let level = 1.0 * i as f64 / MAX_MAP;
        let logarithm = (black + level * (1.0 - black)).log10();
        let value = MAX_MAP * (REFERENCE_WHITE + logarithm / ((gamma / density) * 0.002 * safe_reciprocal(FILM_GAMMA))) / MAXIMUM_LOGARITHMIC_COLORSPACE;
        map.push(map_to_quantum(value));
    }
    map
}

/// `TransformsRGBImage`'s `logmap`.
pub fn log_map_to_srgb() -> Vec<f32> {
    let density = DISPLAY_GAMMA;
    let gamma = DISPLAY_GAMMA;
    let reference_black = REFERENCE_BLACK;
    let reference_white = REFERENCE_WHITE;
    let black = 10.0f64.powf((reference_black - reference_white) * (gamma / density) * 0.002 * safe_reciprocal(FILM_GAMMA));
    let mut map = vec![0.0f32; MAX_MAP as usize + 1];
    let mut i = 0usize;
    let black_end = (reference_black * MAX_MAP / MAXIMUM_LOGARITHMIC_COLORSPACE) as i64;
    while (i as i64) <= black_end {
        map[i] = 0.0;
        i += 1;
    }
    let white_end = (reference_white * MAX_MAP / MAXIMUM_LOGARITHMIC_COLORSPACE) as i64;
    while (i as i64) < white_end {
        let exponent = (MAXIMUM_LOGARITHMIC_COLORSPACE * i as f64 / MAX_MAP - reference_white) * (gamma / density) * 0.002 * safe_reciprocal(FILM_GAMMA);
        map[i] = (QUANTUM_RANGE / (1.0 - black) * (10.0f64.powf(exponent) - black)) as f32;
        i += 1;
    }
    while i <= MAX_MAP as usize {
        map[i] = QUANTUM_RANGE as f32;
        i += 1;
    }
    map
}

// --- the image transform --------------------------------------------------------

/// The colourspaces `sRGBTransformImage` and `TransformsRGBImage` convert
/// pixel by pixel (`ConvertRGBToGeneric`, `ConvertGenericToRGB`).
fn is_generic(colorspace: Colorspace) -> bool {
    matches!(
        colorspace,
        Colorspace::Cmy
            | Colorspace::Adobe98
            | Colorspace::DisplayP3
            | Colorspace::Hcl
            | Colorspace::Hclp
            | Colorspace::Hsb
            | Colorspace::Hsi
            | Colorspace::Hsl
            | Colorspace::Hsv
            | Colorspace::Hwb
            | Colorspace::Jzazbz
            | Colorspace::Lab
            | Colorspace::Lch
            | Colorspace::LchAb
            | Colorspace::LchUv
            | Colorspace::Lms
            | Colorspace::Luv
            | Colorspace::Oklab
            | Colorspace::Oklch
            | Colorspace::ProPhoto
            | Colorspace::XyY
            | Colorspace::Xyz
            | Colorspace::YCbCr
            | Colorspace::YDbDr
            | Colorspace::Yiq
            | Colorspace::YPbPr
            | Colorspace::Yuv
    )
}

impl MagickImage {
    /// `TransformImageColorspace` (Magick.NET's `ColorSpace` setter).
    pub fn transform_colorspace(&mut self, target: Colorspace) {
        if self.colorspace == target {
            return;
        }
        // Undefined only relabels the image.
        if target == Colorspace::Undefined {
            self.colorspace = Colorspace::Undefined;
            return;
        }
        if target.is_srgb() {
            self.transform_to_srgb();
            return;
        }
        if !self.colorspace.is_srgb() {
            self.transform_to_srgb();
        }
        self.transform_from_srgb(target);
    }

    /// The image's quanta with its colour channels replaced by `colors`
    /// per pixel, alpha kept.
    fn rebuild(&mut self, colors: usize, colorspace: Colorspace, convert: impl Fn(&[f32]) -> [f32; 4]) {
        let alpha = self.has_alpha();
        let channels = colors + usize::from(alpha);
        let mut quanta = Vec::with_capacity(self.pixel_count() * channels);
        for pixel in self.quanta.chunks(self.channels) {
            let converted = convert(pixel);
            quanta.extend_from_slice(&converted[..colors]);
            if alpha {
                quanta.push(pixel[pixel.len() - 1]);
            }
        }
        self.quanta = quanta;
        self.channels = channels;
        self.colorspace = colorspace;
    }

    /// `sRGBTransformImage`: the sRGB image to `target`.
    fn transform_from_srgb(&mut self, target: Colorspace) {
        match target {
            Colorspace::Cmyk => self.rebuild(4, target, |pixel| {
                let [cyan, magenta, yellow, black] = rgb_to_cmyk(pixel[0] as f64, pixel[1] as f64, pixel[2] as f64);
                [cyan as f32, magenta as f32, yellow as f32, black as f32]
            }),
            Colorspace::LinearGray => self.rebuild(1, target, |pixel| {
                let gray = 0.212656 * decode_pixel_gamma(pixel[0] as f64) + 0.715158 * decode_pixel_gamma(pixel[1] as f64) + 0.072186 * decode_pixel_gamma(pixel[2] as f64);
                [gray as f32, 0.0, 0.0, 0.0]
            }),
            Colorspace::Gray => self.rebuild(1, target, |pixel| {
                let gray = 0.212656 * pixel[0] as f64 + 0.715158 * pixel[1] as f64 + 0.072186 * pixel[2] as f64;
                [gray as f32, 0.0, 0.0, 0.0]
            }),
            Colorspace::Log => {
                let log_map = log_map_from_srgb();
                self.rebuild(3, target, |pixel| {
                    let mut converted = [0.0f32; 4];
                    for (channel, value) in converted.iter_mut().zip(pixel.iter()).take(3) {
                        let linear = decode_pixel_gamma(*value as f64);
                        *channel = log_map[quantum_to_map(linear as f32)];
                    }
                    converted
                });
            }
            Colorspace::Rgb | Colorspace::ScRgb => self.rebuild(3, target, |pixel| {
                let red = decode_pixel_gamma(pixel[0] as f64);
                let green = decode_pixel_gamma(pixel[1] as f64);
                let blue = decode_pixel_gamma(pixel[2] as f64);
                [red as f32, green as f32, blue as f32, 0.0]
            }),
            _ if is_generic(target) => self.rebuild(3, target, |pixel| {
                let [x, y, z] = rgb_to_generic(target, pixel[0] as f64, pixel[1] as f64, pixel[2] as f64);
                [(QUANTUM_RANGE * x) as f32, (QUANTUM_RANGE * y) as f32, (QUANTUM_RANGE * z) as f32, 0.0]
            }),
            _ => {
                let tables = TransformTables::from_srgb(target);
                self.rebuild(3, target, |pixel| {
                    let sums = tables.sums(quantum_to_map(pixel[0]), quantum_to_map(pixel[1]), quantum_to_map(pixel[2]));
                    [map_to_quantum(sums[0]), map_to_quantum(sums[1]), map_to_quantum(sums[2]), 0.0]
                });
            }
        }
    }

    /// `TransformsRGBImage`: the image back to sRGB.
    pub(crate) fn transform_to_srgb(&mut self) {
        let source = self.colorspace;
        match source {
            Colorspace::Cmyk => self.rebuild(3, Colorspace::SRgb, |pixel| {
                let [red, green, blue] = cmyk_to_rgb(pixel[0] as f64, pixel[1] as f64, pixel[2] as f64, pixel[3] as f64);
                [red as f32, green as f32, blue as f32, 0.0]
            }),
            Colorspace::Gray | Colorspace::LinearGray => {
                // Writers never take grey back (it is sRGB-compatible); the
                // grey value stands for red, green and blue.
                let linear = source == Colorspace::LinearGray;
                self.rebuild(3, Colorspace::SRgb, |pixel| {
                    let value = pixel[0] as f64;
                    let channel = if linear { encode_pixel_gamma(value) } else { value };
                    let gray = 0.212656 * channel + 0.715158 * channel + 0.072186 * channel;
                    [gray as f32, gray as f32, gray as f32, 0.0]
                });
            }
            Colorspace::Log => {
                let log_map = log_map_to_srgb();
                self.rebuild(3, Colorspace::SRgb, |pixel| {
                    let mut converted = [0.0f32; 4];
                    for (channel, value) in converted.iter_mut().zip(pixel.iter()).take(3) {
                        let level = log_map[quantum_to_map(*value)] as f64;
                        *channel = encode_pixel_gamma(level) as f32;
                    }
                    converted
                });
            }
            Colorspace::Rgb | Colorspace::ScRgb => self.rebuild(3, Colorspace::SRgb, |pixel| {
                let red = encode_pixel_gamma(pixel[0] as f64);
                let green = encode_pixel_gamma(pixel[1] as f64);
                let blue = encode_pixel_gamma(pixel[2] as f64);
                [red as f32, green as f32, blue as f32, 0.0]
            }),
            _ if is_generic(source) => self.rebuild(3, Colorspace::SRgb, |pixel| {
                let [red, green, blue] = generic_to_rgb(source, QUANTUM_SCALE * pixel[0] as f64, QUANTUM_SCALE * pixel[1] as f64, QUANTUM_SCALE * pixel[2] as f64);
                [red as f32, green as f32, blue as f32, 0.0]
            }),
            _ => {
                let tables = TransformTables::to_srgb(source);
                let ycc = source == Colorspace::Ycc;
                self.rebuild(3, Colorspace::SRgb, |pixel| {
                    let sums = tables.sums(quantum_to_map(pixel[0]), quantum_to_map(pixel[1]), quantum_to_map(pixel[2]));
                    let mut converted = [0.0f32; 4];
                    for (channel, sum) in converted.iter_mut().zip(sums) {
                        let value = if ycc { ycc_curve(sum) } else { map_to_quantum(sum) as f64 };
                        *channel = value as f32;
                    }
                    converted
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_saved_and_imagemagick_names() {
        assert_eq!(Colorspace::from_name("sRGB"), Colorspace::SRgb);
        assert_eq!(Colorspace::from_name("Rec709YCbCr"), Colorspace::Rec709YCbCr);
        assert_eq!(Colorspace::from_name("nonsense"), Colorspace::Undefined);
        for (colorspace, name) in NAMES {
            assert_eq!(colorspace.name(), name);
        }
    }

    #[test]
    fn hsl_round_trips_a_colour() {
        let [hue, saturation, lightness] = rgb_to_hsl(40000.0, 20000.0, 10000.0);
        let [red, green, blue] = hsl_to_rgb(hue, saturation, lightness);
        assert!((red - 40000.0).abs() < 1e-6 && (green - 20000.0).abs() < 1e-6 && (blue - 10000.0).abs() < 1e-6);
    }

    #[test]
    fn cmyk_takes_out_the_black() {
        let [cyan, magenta, yellow, black] = rgb_to_cmyk(QUANTUM_RANGE, QUANTUM_RANGE * 0.5, 0.0);
        assert_eq!((cyan, black), (0.0, 0.0));
        assert!((magenta - QUANTUM_RANGE * 0.5).abs() < 1e-9 && (yellow - QUANTUM_RANGE).abs() < 1e-9);
        let [red, green, blue] = cmyk_to_rgb(cyan, magenta, yellow, black);
        assert!((red - QUANTUM_RANGE).abs() < 1e-9 && (green - QUANTUM_RANGE * 0.5).abs() < 1e-9 && blue.abs() < 1e-9);
    }
}
