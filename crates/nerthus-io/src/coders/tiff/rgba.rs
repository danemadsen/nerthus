//! libtiff's RGBA interface (`tif_getimage.c`, `TIFFReadRGBAImage` with a
//! bottom-left raster) and its colour conversions (`tif_color.c`): every
//! pixel packed as 8-bit ABGR — grey and palette through lookup tables,
//! 16-bit samples rounded to 8, unassociated alpha premultiplied, CMYK
//! crudely inverted, YCbCr and CIELab converted to RGB. The raster's
//! pointer arithmetic is kept as signed indices, so flips and clipped
//! tiles land where libtiff puts them.

use anyhow::{bail, Result};

use super::directory::*;
use super::read::{Decoder, Outcome};

const FLIP_VERTICALLY: u32 = 1;
const FLIP_HORIZONTALLY: u32 = 2;
const ORIENTATION_BOTLEFT: u16 = 4;

/// `PACK`: opaque RGB.
fn pack(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16) | 0xFF00_0000
}

/// `PACK4`.
fn pack4(r: u32, g: u32, b: u32, a: u32) -> u32 {
    r | (g << 8) | (b << 16) | (a << 24)
}

/// `TIFFYCbCrToRGB`: the lookup tables of `TIFFYCbCrToRGBInit`.
struct YCbCrTables {
    cr_r: [i32; 256],
    cb_b: [i32; 256],
    cr_g: [i32; 256],
    cb_g: [i32; 256],
    y: [i32; 256],
}

/// `FIX`.
fn fix(value: f32) -> i32 {
    ((value * 65536.0) as f64 + 0.5) as i32
}

/// `CLAMP` on floats (NaN goes to the minimum).
fn clamp_float(value: f32, min: f32, max: f32) -> f32 {
    if value.is_nan() || value < min {
        return min;
    }
    if value > max {
        return max;
    }
    value
}

/// `CLAMPw`.
fn clamp_w(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        return min;
    }
    if value > max {
        return max;
    }
    value
}

/// `Code2V`.
fn code_to_value(code: i32, black: f32, white: f32, range: f32) -> f32 {
    let span = if white - black != 0.0 { white - black } else { 1.0 };
    ((code - black as i32) as f32 * range) / span
}

impl YCbCrTables {
    /// `TIFFYCbCrToRGBInit`.
    fn new(luma: [f32; 3], reference: [f32; 6]) -> Self {
        let f1 = 2.0 - 2.0 * luma[0];
        let d1 = fix(clamp_float(f1, 0.0, 2.0));
        let f2 = luma[0] * f1 / luma[1];
        let d2 = -fix(clamp_float(f2, 0.0, 2.0));
        let f3 = 2.0 - 2.0 * luma[2];
        let d3 = fix(clamp_float(f3, 0.0, 2.0));
        let f4 = luma[2] * f3 / luma[1];
        let d4 = -fix(clamp_float(f4, 0.0, 2.0));
        let mut tables = Self { cr_r: [0; 256], cb_b: [0; 256], cr_g: [0; 256], cb_g: [0; 256], y: [0; 256] };
        for index in 0..256 {
            let x = index as i32 - 128;
            let cr_value = code_to_value(x, reference[4] - 128.0, reference[5] - 128.0, 127.0);
            let cr = clamp_w(cr_value, -128.0 * 32.0, 128.0 * 32.0) as i32;
            let cb_value = code_to_value(x, reference[2] - 128.0, reference[3] - 128.0, 127.0);
            let cb = clamp_w(cb_value, -128.0 * 32.0, 128.0 * 32.0) as i32;
            tables.cr_r[index] = d1.wrapping_mul(cr).wrapping_add(1 << 15) >> 16;
            tables.cb_b[index] = d3.wrapping_mul(cb).wrapping_add(1 << 15) >> 16;
            tables.cr_g[index] = d2.wrapping_mul(cr);
            tables.cb_g[index] = d4.wrapping_mul(cb).wrapping_add(1 << 15);
            let y_value = code_to_value(x + 128, reference[0], reference[1], 255.0);
            tables.y[index] = clamp_w(y_value, -128.0 * 32.0, 128.0 * 32.0) as i32;
        }
        tables
    }

    /// `TIFFYCbCrtoRGB`.
    fn to_rgb(&self, y: u32, cb: i32, cr: i32) -> (u32, u32, u32) {
        let y = y.min(255) as usize;
        let cb = cb.clamp(0, 255) as usize;
        let cr = cr.clamp(0, 255) as usize;
        let r = (self.y[y] + self.cr_r[cr]).clamp(0, 255);
        let g = (self.y[y] + ((self.cb_g[cb].wrapping_add(self.cr_g[cr])) >> 16)).clamp(0, 255);
        let b = (self.y[y] + self.cb_b[cb]).clamp(0, 255);
        (r as u32, g as u32, b as u32)
    }
}

/// `TIFFCIELabToRGB` for the sRGB display libtiff uses.
struct CieLab {
    x0: f32,
    y0: f32,
    z0: f32,
    step: f32,
    to_value: Vec<f32>,
}

/// `CIELABTORGB_TABLE_RANGE`.
const CIELAB_RANGE: usize = 1500;
const DISPLAY_MATRIX: [f32; 9] = [3.2410, -1.5374, -0.4986, -0.9692, 1.8760, 0.0416, 0.0556, -0.2040, 1.0570];
const DISPLAY_LUMINANCE: f32 = 100.0;
const DISPLAY_BLACK: f32 = 1.0;
const DISPLAY_WHITE_VALUE: u32 = 255;
const DISPLAY_GAMMA: f32 = 2.4;

impl CieLab {
    /// `TIFFCIELabToRGBInit` (every gun alike).
    fn new(reference_white: [f32; 3]) -> Self {
        let gamma = 1.0 / DISPLAY_GAMMA as f64;
        let step = (DISPLAY_LUMINANCE - DISPLAY_BLACK) / CIELAB_RANGE as f32;
        let mut to_value = Vec::with_capacity(CIELAB_RANGE + 1);
        for index in 0..=CIELAB_RANGE {
            let ratio = index as f64 / CIELAB_RANGE as f64;
            to_value.push(DISPLAY_WHITE_VALUE as f32 * ratio.powf(gamma) as f32);
        }
        Self { x0: reference_white[0], y0: reference_white[1], z0: reference_white[2], step, to_value }
    }

    /// `TIFFCIELab16ToXYZ`.
    fn lab16_to_xyz(&self, l: u32, a: i32, b: i32) -> (f32, f32, f32) {
        let lightness = l as f32 * 100.0 / 65535.0;
        let (y, cby) = if lightness < 8.856 {
            let y = (lightness * self.y0) / 903.292;
            (y, 7.787 * (y / self.y0) + 16.0 / 116.0)
        } else {
            let cby = (lightness + 16.0) / 116.0;
            (self.y0 * cby * cby * cby, cby)
        };
        let tmp = a as f32 / 256.0 / 500.0 + cby;
        let x = if tmp < 0.2069 { self.x0 * (tmp - 0.13793) / 7.787 } else { self.x0 * tmp * tmp * tmp };
        let tmp = cby - b as f32 / 256.0 / 200.0;
        let z = if tmp < 0.2069 { self.z0 * (tmp - 0.13793) / 7.787 } else { self.z0 * tmp * tmp * tmp };
        (x, y, z)
    }

    /// `TIFFXYZToRGB`.
    fn xyz_to_rgb(&self, x: f32, y: f32, z: f32) -> (u32, u32, u32) {
        let m = DISPLAY_MATRIX;
        let luminances = [m[0] * x + m[1] * y + m[2] * z, m[3] * x + m[4] * y + m[5] * z, m[6] * x + m[7] * y + m[8] * z];
        let mut values = [0u32; 3];
        for (value, luminance) in values.iter_mut().zip(luminances) {
            // `TIFFmax` then `TIFFmin` (NaN comes out as black).
            let floored = if luminance > DISPLAY_BLACK { luminance } else { DISPLAY_BLACK };
            let clipped = if floored < DISPLAY_LUMINANCE { floored } else { DISPLAY_LUMINANCE };
            let index = ((clipped - DISPLAY_BLACK) / self.step) as usize;
            let index = index.min(CIELAB_RANGE);
            let entry = self.to_value[index];
            let rounded = if entry > 0.0 { (entry as f64 + 0.5) as u32 } else { (entry as f64 - 0.5) as u32 };
            *value = rounded.min(DISPLAY_WHITE_VALUE);
        }
        (values[0], values[1], values[2])
    }
}

/// The put routine `PickContigCase`/`PickSeparateCase` chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Put {
    Cmap8,
    Cmap4,
    Cmap2,
    Cmap1,
    Grey,
    AlphaGrey,
    Bw16,
    Bw1,
    Bw2,
    Bw4,
    Rgb8,
    RgbaAssociated8,
    RgbaUnassociated8,
    Rgb16,
    RgbaAssociated16,
    RgbaUnassociated16,
    Cmyk8,
    Cmyk8Map,
    Lab8,
    Lab16,
    YCbCr44,
    YCbCr42,
    YCbCr41,
    YCbCr22,
    YCbCr21,
    YCbCr12,
    YCbCr11,
    SeparateRgb8,
    SeparateRgbaAssociated8,
    SeparateCmyk8,
    SeparateRgbaUnassociated8,
    SeparateRgb16,
    SeparateRgbaAssociated16,
    SeparateRgbaUnassociated16,
    SeparateYCbCr11,
}

/// `TIFFRGBAImage`.
struct RgbaImage {
    bits: u16,
    samples: u16,
    alpha: u16,
    photometric: u16,
    width: u32,
    height: u32,
    orientation: u16,
    put: Put,
    map: Option<Vec<u8>>,
    bw_map: Vec<Vec<u32>>,
    palette_map: Vec<Vec<u32>>,
    ycbcr: Option<YCbCrTables>,
    cielab: Option<CieLab>,
    ua_to_aa: Vec<u8>,
    sixteen_to_eight: Vec<u8>,
}

/// `TIFFRGBAImageOK` and `TIFFRGBAImageBegin`'s checks; the message is a
/// libtiff error.
fn check(tiff: &Tiff) -> Result<()> {
    let dir = &tiff.dir;
    if !matches!(dir.bits_per_sample, 1 | 2 | 4 | 8 | 16) {
        bail!("Sorry, can not handle images with {}-bit samples", dir.bits_per_sample);
    }
    if dir.sample_format == SAMPLEFORMAT_IEEEFP {
        bail!("Sorry, can not handle images with IEEE floating-point samples");
    }
    let colour_channels = dir.samples_per_pixel as i32 - dir.extra_samples.len() as i32;
    let photometric = match dir.photometric {
        Some(value) => value,
        None => match colour_channels {
            1 => PHOTOMETRIC_MINISBLACK,
            3 => PHOTOMETRIC_RGB,
            _ => bail!("Missing needed PhotometricInterpretation tag"),
        },
    };
    match photometric {
        PHOTOMETRIC_MINISWHITE | PHOTOMETRIC_MINISBLACK | PHOTOMETRIC_PALETTE => {
            if dir.planar_config == PLANARCONFIG_CONTIG && dir.samples_per_pixel != 1 && dir.bits_per_sample < 8 {
                bail!("Sorry, can not handle contiguous data with PhotometricInterpretation={photometric}, and Samples/pixel={} and Bits/Sample={}", dir.samples_per_pixel, dir.bits_per_sample);
            }
        }
        PHOTOMETRIC_YCBCR => {}
        PHOTOMETRIC_RGB => {
            if colour_channels < 3 {
                bail!("Sorry, can not handle RGB image with Color channels={colour_channels}");
            }
        }
        PHOTOMETRIC_SEPARATED => {
            if dir.ink_set != INKSET_CMYK {
                bail!("Sorry, can not handle separated image with InkSet={}", dir.ink_set);
            }
            if dir.samples_per_pixel < 4 {
                bail!("Sorry, can not handle separated image with Samples/pixel={}", dir.samples_per_pixel);
            }
        }
        PHOTOMETRIC_LOGL => {
            if dir.compression != COMPRESSION_SGILOG {
                bail!("Sorry, LogL data must have Compression={COMPRESSION_SGILOG}");
            }
        }
        PHOTOMETRIC_LOGLUV => {
            if dir.compression != COMPRESSION_SGILOG && dir.compression != COMPRESSION_SGILOG24 {
                bail!("Sorry, LogLuv data must have Compression={COMPRESSION_SGILOG} or {COMPRESSION_SGILOG24}");
            }
            if dir.planar_config != PLANARCONFIG_CONTIG {
                bail!("Sorry, can not handle LogLuv images with Planarconfiguration={}", dir.planar_config);
            }
            if dir.samples_per_pixel != 3 || colour_channels != 3 {
                bail!("Sorry, can not handle image with Samples/pixel={}, colorchannels={colour_channels}", dir.samples_per_pixel);
            }
        }
        PHOTOMETRIC_CIELAB => {
            if dir.samples_per_pixel != 3 || colour_channels != 3 || (dir.bits_per_sample != 8 && dir.bits_per_sample != 16) {
                bail!("Sorry, can not handle image with Samples/pixel={}, colorchannels={colour_channels} and Bits/sample={}", dir.samples_per_pixel, dir.bits_per_sample);
            }
        }
        other => bail!("Sorry, can not handle image with PhotometricInterpretation={other}"),
    }
    Ok(())
}

/// `setupMap`: samples to 0–255 (inverted for min-is-white), and for grey
/// the `makebwmap` tables.
fn setup_map(image: &mut RgbaImage) {
    let bits = image.bits;
    let range: i32 = if bits == 16 { 255 } else { (1i32 << bits) - 1 };
    let mut map = vec![0u8; range as usize + 1];
    for x in 0..=range {
        map[x as usize] = if image.photometric == PHOTOMETRIC_MINISWHITE { (((range - x) * 255) / range) as u8 } else { ((x * 255) / range) as u8 };
    }
    if bits <= 16 && (image.photometric == PHOTOMETRIC_MINISBLACK || image.photometric == PHOTOMETRIC_MINISWHITE) {
        image.bw_map = (0..256u32)
            .map(|index| {
                let grey = |value: u32| {
                    let c = map[value as usize] as u32;
                    pack(c, c, c)
                };
                match bits {
                    1 => (0..8).map(|bit| grey((index >> (7 - bit)) & 1)).collect(),
                    2 => (0..4).map(|pair| grey((index >> (6 - 2 * pair)) & 3)).collect(),
                    4 => vec![grey(index >> 4), grey(index & 0xF)],
                    _ => vec![grey(index)],
                }
            })
            .collect();
        image.map = None;
    } else {
        image.map = Some(map);
    }
}

/// `buildMap`.
fn build_map(image: &mut RgbaImage, colormap: Option<&[Vec<u16>; 3]>) {
    match image.photometric {
        PHOTOMETRIC_RGB | PHOTOMETRIC_YCBCR | PHOTOMETRIC_SEPARATED if image.bits == 8 => {}
        PHOTOMETRIC_RGB | PHOTOMETRIC_YCBCR | PHOTOMETRIC_SEPARATED | PHOTOMETRIC_MINISBLACK | PHOTOMETRIC_MINISWHITE => setup_map(image),
        PHOTOMETRIC_PALETTE => {
            let Some([red, green, blue]) = colormap else {
                return;
            };
            let count = 1usize << image.bits;
            let wide = (0..count).any(|index| red[index] >= 256 || green[index] >= 256 || blue[index] >= 256);
            let convert = |value: u16| if wide { value >> 8 } else { value };
            let red: Vec<u16> = red[..count].iter().map(|&value| convert(value)).collect();
            let green: Vec<u16> = green[..count].iter().map(|&value| convert(value)).collect();
            let blue: Vec<u16> = blue[..count].iter().map(|&value| convert(value)).collect();
            if image.bits <= 8 {
                let entry = |c: usize| pack((red[c] & 0xFF) as u32, (green[c] & 0xFF) as u32, (blue[c] & 0xFF) as u32);
                image.palette_map = (0..256usize)
                    .map(|index| match image.bits {
                        1 => (0..8).map(|bit| entry((index >> (7 - bit)) & 1)).collect(),
                        2 => (0..4).map(|pair| entry((index >> (6 - 2 * pair)) & 3)).collect(),
                        4 => vec![entry(index >> 4), entry(index & 0xF)],
                        _ => vec![entry(index)],
                    })
                    .collect();
            }
        }
        _ => {}
    }
}

/// `BuildMapUaToAa`.
fn build_ua_to_aa() -> Vec<u8> {
    let mut table = Vec::with_capacity(65536);
    for alpha in 0..256u32 {
        for value in 0..256u32 {
            table.push(((value * alpha + 127) / 255) as u8);
        }
    }
    table
}

/// `BuildMapBitdepth16To8`.
fn build_sixteen_to_eight() -> Vec<u8> {
    (0..65536u32).map(|value| ((value + 128) / 257) as u8).collect()
}

/// `initYCbCrConversion`.
fn init_ycbcr(tiff: &Tiff) -> Result<YCbCrTables> {
    let dir = &tiff.dir;
    let luma = dir.ycbcr_coefficients.unwrap_or([0.299, 0.587, 0.114]);
    let reference = dir.reference_black_white.unwrap_or_else(|| default_reference_black_white(dir));
    if luma[0].is_nan() || luma[1].is_nan() || luma[1] == 0.0 || luma[2].is_nan() {
        bail!("Invalid values for YCbCrCoefficients tag");
    }
    let in_range = |value: f32| value > (-0x7FFF_FFFF + 128) as f32 && value < 0x7FFF_FFFF as f32;
    if !reference.iter().all(|&value| in_range(value)) {
        bail!("Invalid values for ReferenceBlackWhite tag");
    }
    Ok(YCbCrTables::new(luma, reference))
}

/// `TIFFDefaultRefBlackWhite`.
fn default_reference_black_white(dir: &Directory) -> [f32; 6] {
    if dir.photometric == Some(PHOTOMETRIC_YCBCR) {
        return [0.0, 255.0, 128.0, 255.0, 128.0, 255.0];
    }
    let white = ((1i64 << dir.bits_per_sample) - 1) as f32;
    [0.0, white, 0.0, white, 0.0, white]
}

/// `initCIELabConversion`: the white point (D50 when missing) as the
/// reference white.
fn init_cielab(tiff: &Tiff) -> Result<CieLab> {
    let white_point = tiff.dir.white_point.unwrap_or_else(|| {
        let sum = 96.4250f32 + 100.0f32 + 82.4680f32;
        [96.4250f32 / sum, 100.0f32 / sum]
    });
    if white_point[1] == 0.0 {
        bail!("Invalid value for WhitePoint tag.");
    }
    let x = white_point[0] / white_point[1] * 100.0;
    let z = (1.0 - white_point[0] - white_point[1]) / white_point[1] * 100.0;
    Ok(CieLab::new([x, 100.0, z]))
}

/// `TIFFRGBAImageBegin`.
fn begin(tiff: &mut Tiff) -> Result<RgbaImage> {
    check(tiff)?;
    let dir = tiff.dir.clone();
    let mut alpha = 0u16;
    let mut extra = dir.extra_samples.len() as u16;
    if extra >= 1 {
        match dir.extra_samples[0] {
            EXTRASAMPLE_UNSPECIFIED => {
                if dir.samples_per_pixel > 3 {
                    alpha = EXTRASAMPLE_ASSOCALPHA;
                }
            }
            EXTRASAMPLE_ASSOCALPHA | EXTRASAMPLE_UNASSALPHA => alpha = dir.extra_samples[0],
            _ => {}
        }
    }
    let photometric_tag = dir.photometric;
    let mut photometric = photometric_tag.unwrap_or(PHOTOMETRIC_MINISWHITE);
    if extra == 0 && dir.samples_per_pixel == 4 && photometric == PHOTOMETRIC_RGB {
        alpha = EXTRASAMPLE_ASSOCALPHA;
        extra = 1;
    }
    let colour_channels = dir.samples_per_pixel as i32 - extra as i32;
    if photometric_tag.is_none() {
        photometric = match colour_channels {
            1 => {
                let ccitt = matches!(dir.compression, COMPRESSION_CCITTFAX3 | COMPRESSION_CCITTFAX4 | COMPRESSION_CCITTRLE | COMPRESSION_CCITTRLEW);
                if ccitt {
                    PHOTOMETRIC_MINISWHITE
                } else {
                    PHOTOMETRIC_MINISBLACK
                }
            }
            3 => PHOTOMETRIC_RGB,
            _ => bail!("Missing needed PhotometricInterpretation tag"),
        };
    }
    let mut bits = dir.bits_per_sample;
    match photometric {
        PHOTOMETRIC_PALETTE => {
            if dir.colormap.is_none() {
                bail!("Missing required \"Colormap\" tag");
            }
            if dir.planar_config == PLANARCONFIG_CONTIG && dir.samples_per_pixel != 1 && bits < 8 {
                bail!("Sorry, can not handle contiguous data with PhotometricInterpretation={photometric}");
            }
        }
        PHOTOMETRIC_MINISWHITE | PHOTOMETRIC_MINISBLACK => {
            if dir.planar_config == PLANARCONFIG_CONTIG && dir.samples_per_pixel != 1 && bits < 8 {
                bail!("Sorry, can not handle contiguous data with PhotometricInterpretation={photometric}");
            }
        }
        PHOTOMETRIC_YCBCR => {
            if dir.planar_config == PLANARCONFIG_CONTIG && dir.compression == COMPRESSION_JPEG {
                tiff.jpeg_color_mode_rgb = true;
                photometric = PHOTOMETRIC_RGB;
            }
        }
        PHOTOMETRIC_RGB => {
            if colour_channels < 3 {
                bail!("Sorry, can not handle RGB image with Color channels={colour_channels}");
            }
        }
        PHOTOMETRIC_SEPARATED => {
            if dir.ink_set != INKSET_CMYK || dir.samples_per_pixel < 4 {
                bail!("Sorry, can not handle separated image");
            }
        }
        PHOTOMETRIC_LOGL => {
            if dir.compression != COMPRESSION_SGILOG {
                bail!("Sorry, LogL data must have Compression={COMPRESSION_SGILOG}");
            }
            // `SGILOGDATAFMT_8BIT` retags the samples as 8-bit.
            tiff.sgilog_8bit = true;
            tiff.dir.bits_per_sample = 8;
            tiff.dir.sample_format = SAMPLEFORMAT_UINT;
            photometric = PHOTOMETRIC_MINISBLACK;
            bits = 8;
        }
        PHOTOMETRIC_LOGLUV => {
            if dir.compression != COMPRESSION_SGILOG && dir.compression != COMPRESSION_SGILOG24 {
                bail!("Sorry, LogLuv data must have Compression={COMPRESSION_SGILOG} or {COMPRESSION_SGILOG24}");
            }
            if dir.planar_config != PLANARCONFIG_CONTIG {
                bail!("Sorry, can not handle LogLuv images with Planarconfiguration={}", dir.planar_config);
            }
            tiff.sgilog_8bit = true;
            tiff.dir.bits_per_sample = 8;
            tiff.dir.sample_format = SAMPLEFORMAT_UINT;
            photometric = PHOTOMETRIC_RGB;
            bits = 8;
        }
        PHOTOMETRIC_CIELAB => {}
        other => bail!("Sorry, can not handle image with PhotometricInterpretation={other}"),
    }
    let mut image = RgbaImage {
        bits,
        samples: dir.samples_per_pixel,
        alpha,
        photometric,
        width: dir.width,
        height: dir.length,
        orientation: dir.orientation,
        put: Put::Grey,
        map: None,
        bw_map: Vec::new(),
        palette_map: Vec::new(),
        ycbcr: None,
        cielab: None,
        ua_to_aa: Vec::new(),
        sixteen_to_eight: Vec::new(),
    };
    let contig = !(dir.planar_config == PLANARCONFIG_SEPARATE && dir.samples_per_pixel > 1);
    let put = if contig { pick_contig(tiff, &mut image)? } else { pick_separate(tiff, &mut image)? };
    let Some(put) = put else {
        bail!("Sorry, can not handle image");
    };
    image.put = put;
    Ok(image)
}

/// `PickContigCase`.
fn pick_contig(tiff: &Tiff, image: &mut RgbaImage) -> Result<Option<Put>> {
    let colormap = tiff.dir.colormap.as_ref();
    let put = match image.photometric {
        PHOTOMETRIC_RGB => match image.bits {
            8 if image.alpha == EXTRASAMPLE_ASSOCALPHA && image.samples >= 4 => Some(Put::RgbaAssociated8),
            8 if image.alpha == EXTRASAMPLE_UNASSALPHA && image.samples >= 4 => {
                image.ua_to_aa = build_ua_to_aa();
                Some(Put::RgbaUnassociated8)
            }
            8 if image.samples >= 3 => Some(Put::Rgb8),
            16 if image.alpha == EXTRASAMPLE_ASSOCALPHA && image.samples >= 4 => {
                image.sixteen_to_eight = build_sixteen_to_eight();
                Some(Put::RgbaAssociated16)
            }
            16 if image.alpha == EXTRASAMPLE_UNASSALPHA && image.samples >= 4 => {
                image.sixteen_to_eight = build_sixteen_to_eight();
                image.ua_to_aa = build_ua_to_aa();
                Some(Put::RgbaUnassociated16)
            }
            16 if image.samples >= 3 => {
                image.sixteen_to_eight = build_sixteen_to_eight();
                Some(Put::Rgb16)
            }
            _ => None,
        },
        PHOTOMETRIC_SEPARATED => {
            if image.samples >= 4 {
                build_map(image, colormap);
                if image.bits == 8 {
                    Some(if image.map.is_none() { Put::Cmyk8 } else { Put::Cmyk8Map })
                } else {
                    None
                }
            } else {
                None
            }
        }
        PHOTOMETRIC_PALETTE => {
            build_map(image, colormap);
            match image.bits {
                8 => Some(Put::Cmap8),
                4 => Some(Put::Cmap4),
                2 => Some(Put::Cmap2),
                1 => Some(Put::Cmap1),
                _ => None,
            }
        }
        PHOTOMETRIC_MINISWHITE | PHOTOMETRIC_MINISBLACK => {
            build_map(image, colormap);
            match image.bits {
                16 => Some(Put::Bw16),
                8 if image.alpha != 0 && image.samples == 2 => Some(Put::AlphaGrey),
                8 => Some(Put::Grey),
                4 => Some(Put::Bw4),
                2 => Some(Put::Bw2),
                1 => Some(Put::Bw1),
                _ => None,
            }
        }
        PHOTOMETRIC_YCBCR => {
            if image.bits == 8 && image.samples == 3 {
                image.ycbcr = Some(init_ycbcr(tiff)?);
                let [horizontal, vertical] = tiff.dir.ycbcr_subsampling;
                match (horizontal << 4) | vertical {
                    0x44 => Some(Put::YCbCr44),
                    0x42 => Some(Put::YCbCr42),
                    0x41 => Some(Put::YCbCr41),
                    0x22 => Some(Put::YCbCr22),
                    0x21 => Some(Put::YCbCr21),
                    0x12 => Some(Put::YCbCr12),
                    0x11 => Some(Put::YCbCr11),
                    _ => None,
                }
            } else {
                None
            }
        }
        PHOTOMETRIC_CIELAB => {
            if image.samples == 3 {
                build_map(image, colormap);
                if image.bits == 8 || image.bits == 16 {
                    image.cielab = Some(init_cielab(tiff)?);
                    Some(if image.bits == 8 { Put::Lab8 } else { Put::Lab16 })
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
    };
    Ok(put)
}

/// `PickSeparateCase`.
fn pick_separate(tiff: &Tiff, image: &mut RgbaImage) -> Result<Option<Put>> {
    let put = match image.photometric {
        PHOTOMETRIC_MINISWHITE | PHOTOMETRIC_MINISBLACK | PHOTOMETRIC_RGB => match image.bits {
            8 if image.alpha == EXTRASAMPLE_ASSOCALPHA => Some(Put::SeparateRgbaAssociated8),
            8 if image.alpha == EXTRASAMPLE_UNASSALPHA => {
                image.ua_to_aa = build_ua_to_aa();
                Some(Put::SeparateRgbaUnassociated8)
            }
            8 => Some(Put::SeparateRgb8),
            16 if image.alpha == EXTRASAMPLE_ASSOCALPHA => {
                image.sixteen_to_eight = build_sixteen_to_eight();
                Some(Put::SeparateRgbaAssociated16)
            }
            16 if image.alpha == EXTRASAMPLE_UNASSALPHA => {
                image.sixteen_to_eight = build_sixteen_to_eight();
                image.ua_to_aa = build_ua_to_aa();
                Some(Put::SeparateRgbaUnassociated16)
            }
            16 => {
                image.sixteen_to_eight = build_sixteen_to_eight();
                Some(Put::SeparateRgb16)
            }
            _ => None,
        },
        PHOTOMETRIC_SEPARATED => {
            if image.bits == 8 && image.samples == 4 {
                image.alpha = 1;
                Some(Put::SeparateCmyk8)
            } else {
                None
            }
        }
        PHOTOMETRIC_YCBCR => {
            if image.bits == 8 && image.samples == 3 {
                image.ycbcr = Some(init_ycbcr(tiff)?);
                let [horizontal, vertical] = tiff.dir.ycbcr_subsampling;
                if (horizontal << 4) | vertical == 0x11 {
                    Some(Put::SeparateYCbCr11)
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
    };
    Ok(put)
}

/// `setorientation` for a bottom-left raster.
fn flip_for(orientation: u16) -> u32 {
    match orientation {
        1 | 5 => FLIP_VERTICALLY,
        2 | 6 => FLIP_HORIZONTALLY | FLIP_VERTICALLY,
        3 | 7 => FLIP_HORIZONTALLY,
        _ => 0,
    }
}

/// The raster and the moving write position (`cp`).
struct Raster<'a> {
    pixels: &'a mut [u32],
}

impl Raster<'_> {
    fn set(&mut self, index: i64, value: u32) {
        if index >= 0
            && let Some(pixel) = self.pixels.get_mut(index as usize)
        {
            *pixel = value;
        }
    }
}

/// A sample buffer read at a moving position (`pp`); bytes outside it read
/// as zero.
#[derive(Clone, Copy)]
struct Source<'a> {
    bytes: &'a [u8],
}

impl Source<'_> {
    fn byte(&self, index: i64) -> u8 {
        if index < 0 {
            return 0;
        }
        self.bytes.get(index as usize).copied().unwrap_or(0)
    }

    fn word(&self, index: i64) -> u16 {
        u16::from_le_bytes([self.byte(index), self.byte(index + 1)])
    }
}

impl RgbaImage {
    /// The contiguous put routines, `w` × `h` pixels from `pp` to `cp`.
    #[allow(clippy::too_many_arguments)]
    fn put_contig(&self, raster: &mut Raster, mut cp: i64, w: u32, h: u32, mut fromskew: i32, toskew: i32, source: Source, mut pp: i64) {
        let samples = self.samples as i64;
        let w64 = w as i64;
        let toskew = toskew as i64;
        match self.put {
            Put::Cmap8 | Put::Grey => {
                let table = if self.put == Put::Cmap8 { &self.palette_map } else { &self.bw_map };
                for _ in 0..h {
                    for _ in 0..w {
                        raster.set(cp, table[source.byte(pp) as usize][0]);
                        cp += 1;
                        pp += samples;
                    }
                    cp += toskew;
                    pp += fromskew as i64;
                }
            }
            Put::AlphaGrey => {
                for _ in 0..h {
                    for _ in 0..w {
                        let alpha = (source.byte(pp + 1) as u32) << 24 | !0xFF00_0000u32;
                        raster.set(cp, self.bw_map[source.byte(pp) as usize][0] & alpha);
                        cp += 1;
                        pp += samples;
                    }
                    cp += toskew;
                    pp += fromskew as i64;
                }
            }
            Put::Bw16 => {
                for _ in 0..h {
                    for _ in 0..w {
                        let value = source.word(pp);
                        raster.set(cp, self.bw_map[(value >> 8) as usize][0]);
                        cp += 1;
                        pp += 2 * samples;
                    }
                    cp += toskew;
                    pp += fromskew as i64;
                }
            }
            Put::Cmap4 | Put::Cmap2 | Put::Cmap1 | Put::Bw4 | Put::Bw2 | Put::Bw1 => {
                let per_byte: u32 = match self.put {
                    Put::Cmap4 | Put::Bw4 => 2,
                    Put::Cmap2 | Put::Bw2 => 4,
                    _ => 8,
                };
                let table = if matches!(self.put, Put::Cmap4 | Put::Cmap2 | Put::Cmap1) { &self.palette_map } else { &self.bw_map };
                fromskew /= per_byte as i32;
                for _ in 0..h {
                    let mut left = w;
                    while left > 0 {
                        let entry = &table[source.byte(pp) as usize];
                        pp += 1;
                        let count = left.min(per_byte);
                        for value in entry.iter().take(count as usize) {
                            raster.set(cp, *value);
                            cp += 1;
                        }
                        left -= count;
                    }
                    cp += toskew;
                    pp += fromskew as i64;
                }
            }
            Put::Rgb8 | Put::RgbaAssociated8 | Put::Cmyk8 | Put::Cmyk8Map => {
                fromskew *= self.samples as i32;
                for _ in 0..h {
                    for _ in 0..w {
                        let p = |offset: i64| source.byte(pp + offset) as u32;
                        let value = match self.put {
                            Put::Rgb8 => pack(p(0), p(1), p(2)),
                            Put::RgbaAssociated8 => pack4(p(0), p(1), p(2), p(3)),
                            _ => {
                                let k = 255 - p(3);
                                let r = (k * (255 - p(0))) / 255;
                                let g = (k * (255 - p(1))) / 255;
                                let b = (k * (255 - p(2))) / 255;
                                match &self.map {
                                    Some(map) if self.put == Put::Cmyk8Map => pack(map[r as usize] as u32, map[g as usize] as u32, map[b as usize] as u32),
                                    _ => pack(r & 0xFFFF, g & 0xFFFF, b & 0xFFFF),
                                }
                            }
                        };
                        raster.set(cp, value);
                        cp += 1;
                        pp += samples;
                    }
                    cp += toskew;
                    pp += fromskew as i64;
                }
            }
            Put::RgbaUnassociated8 => {
                fromskew *= self.samples as i32;
                for _ in 0..h {
                    for _ in 0..w {
                        let a = source.byte(pp + 3) as usize;
                        let m = |value: u8| self.ua_to_aa[(a << 8) + value as usize] as u32;
                        raster.set(cp, pack4(m(source.byte(pp)), m(source.byte(pp + 1)), m(source.byte(pp + 2)), a as u32));
                        cp += 1;
                        pp += samples;
                    }
                    cp += toskew;
                    pp += fromskew as i64;
                }
            }
            Put::Rgb16 | Put::RgbaAssociated16 | Put::RgbaUnassociated16 => {
                fromskew *= self.samples as i32;
                // `wp` walks words; skews are in words too.
                let mut wp = pp;
                for _ in 0..h {
                    for _ in 0..w {
                        let word = |offset: i64| self.sixteen_to_eight[source.word(wp + 2 * offset) as usize] as u32;
                        let value = match self.put {
                            Put::Rgb16 => pack(word(0), word(1), word(2)),
                            Put::RgbaAssociated16 => pack4(word(0), word(1), word(2), word(3)),
                            _ => {
                                let a = word(3) as usize;
                                let m = |value: u32| self.ua_to_aa[(a << 8) + value as usize] as u32;
                                pack4(m(word(0)), m(word(1)), m(word(2)), a as u32)
                            }
                        };
                        raster.set(cp, value);
                        cp += 1;
                        wp += 2 * samples;
                    }
                    cp += toskew;
                    wp += 2 * fromskew as i64;
                }
            }
            Put::Lab8 | Put::Lab16 => {
                let cielab = self.cielab.as_ref().expect("CIELab tables");
                fromskew *= 3;
                for _ in 0..h {
                    for _ in 0..w {
                        let (x, y, z) = if self.put == Put::Lab8 {
                            let l = source.byte(pp) as u32;
                            let a = source.byte(pp + 1) as i8 as i32;
                            let b = source.byte(pp + 2) as i8 as i32;
                            pp += 3;
                            cielab.lab16_to_xyz(l * 257, a * 256, b * 256)
                        } else {
                            let l = source.word(pp) as u32;
                            let a = source.word(pp + 2) as i16 as i32;
                            let b = source.word(pp + 4) as i16 as i32;
                            pp += 6;
                            cielab.lab16_to_xyz(l, a, b)
                        };
                        let (r, g, b) = cielab.xyz_to_rgb(x, y, z);
                        raster.set(cp, pack(r, g, b));
                        cp += 1;
                    }
                    cp += toskew;
                    pp += if self.put == Put::Lab8 { fromskew as i64 } else { 2 * fromskew as i64 };
                }
            }
            Put::YCbCr44 | Put::YCbCr42 | Put::YCbCr41 | Put::YCbCr22 | Put::YCbCr21 | Put::YCbCr12 | Put::YCbCr11 => {
                self.put_ycbcr(raster, cp, w64, h as i64, fromskew as i64, toskew, source, pp);
            }
            _ => {}
        }
    }

    /// The packed YCbCr put routines.
    #[allow(clippy::too_many_arguments)]
    fn put_ycbcr(&self, raster: &mut Raster, mut cp: i64, w: i64, mut h: i64, fromskew: i64, toskew: i64, source: Source, mut pp: i64) {
        let tables = self.ycbcr.as_ref().expect("YCbCr tables");
        let convert = |raster: &mut Raster, index: i64, y: u8, cb: i32, cr: i32| {
            let (r, g, b) = tables.to_rgb(y as u32, cb, cr);
            raster.set(index, pack(r, g, b));
        };
        let p = |pp: i64, offset: i64| source.byte(pp + offset);
        match self.put {
            Put::YCbCr44 => {
                let mut cp1 = cp + w + toskew;
                let mut cp2 = cp1 + w + toskew;
                let mut cp3 = cp2 + w + toskew;
                let incr = 3 * w + 4 * toskew;
                let fromskew = (fromskew / 4) * (4 * 2 + 2);
                let rows = [0i64, 1, 2, 3];
                if (h & 3) == 0 && (w & 3) == 0 {
                    while h >= 4 {
                        for _ in 0..(w >> 2) {
                            let cb = p(pp, 16) as i32;
                            let cr = p(pp, 17) as i32;
                            for row in rows {
                                let target = [cp, cp1, cp2, cp3][row as usize];
                                for column in 0..4 {
                                    convert(raster, target + column, p(pp, row * 4 + column), cb, cr);
                                }
                            }
                            cp += 4;
                            cp1 += 4;
                            cp2 += 4;
                            cp3 += 4;
                            pp += 18;
                        }
                        cp += incr;
                        cp1 += incr;
                        cp2 += incr;
                        cp3 += incr;
                        pp += fromskew;
                        h -= 4;
                    }
                } else {
                    while h > 0 {
                        let mut x = w;
                        while x > 0 {
                            let cb = p(pp, 16) as i32;
                            let cr = p(pp, 17) as i32;
                            let columns = x.min(4);
                            let rows_here = h.min(4);
                            for column in (0..columns).rev() {
                                for row in (0..rows_here).rev() {
                                    let target = [cp, cp1, cp2, cp3][row as usize];
                                    convert(raster, target + column, p(pp, row * 4 + column), cb, cr);
                                }
                            }
                            if x < 4 {
                                cp += x;
                                cp1 += x;
                                cp2 += x;
                                cp3 += x;
                                x = 0;
                            } else {
                                cp += 4;
                                cp1 += 4;
                                cp2 += 4;
                                cp3 += 4;
                                x -= 4;
                            }
                            pp += 18;
                        }
                        if h <= 4 {
                            break;
                        }
                        h -= 4;
                        cp += incr;
                        cp1 += incr;
                        cp2 += incr;
                        cp3 += incr;
                        pp += fromskew;
                    }
                }
            }
            Put::YCbCr42 => {
                let mut cp1 = cp + w + toskew;
                let incr = 2 * toskew + w;
                let fromskew = (fromskew / 4) * (4 * 2 + 2);
                if (w & 3) == 0 && (h & 1) == 0 {
                    while h >= 2 {
                        for _ in 0..(w >> 2) {
                            let cb = p(pp, 8) as i32;
                            let cr = p(pp, 9) as i32;
                            for column in 0..4 {
                                convert(raster, cp + column, p(pp, column), cb, cr);
                                convert(raster, cp1 + column, p(pp, 4 + column), cb, cr);
                            }
                            cp += 4;
                            cp1 += 4;
                            pp += 10;
                        }
                        cp += incr;
                        cp1 += incr;
                        pp += fromskew;
                        h -= 2;
                    }
                } else {
                    while h > 0 {
                        let mut x = w;
                        while x > 0 {
                            let cb = p(pp, 8) as i32;
                            let cr = p(pp, 9) as i32;
                            let columns = x.min(4);
                            for column in (0..columns).rev() {
                                if h >= 2 {
                                    convert(raster, cp1 + column, p(pp, 4 + column), cb, cr);
                                }
                                convert(raster, cp + column, p(pp, column), cb, cr);
                            }
                            if x < 4 {
                                cp += x;
                                cp1 += x;
                                x = 0;
                            } else {
                                cp += 4;
                                cp1 += 4;
                                x -= 4;
                            }
                            pp += 10;
                        }
                        if h <= 2 {
                            break;
                        }
                        h -= 2;
                        cp += incr;
                        cp1 += incr;
                        pp += fromskew;
                    }
                }
            }
            Put::YCbCr41 => {
                let fromskew = (fromskew / 4) * (4 + 2);
                loop {
                    for _ in 0..(w >> 2) {
                        let cb = p(pp, 4) as i32;
                        let cr = p(pp, 5) as i32;
                        for column in 0..4 {
                            convert(raster, cp + column, p(pp, column), cb, cr);
                        }
                        cp += 4;
                        pp += 6;
                    }
                    if (w & 3) != 0 {
                        let cb = p(pp, 4) as i32;
                        let cr = p(pp, 5) as i32;
                        for column in (0..(w & 3)).rev() {
                            convert(raster, cp + column, p(pp, column), cb, cr);
                        }
                        cp += w & 3;
                        pp += 6;
                    }
                    cp += toskew;
                    pp += fromskew;
                    h -= 1;
                    if h == 0 {
                        break;
                    }
                }
            }
            Put::YCbCr22 => {
                let incr = 2 * toskew + w;
                let fromskew = (fromskew / 2) * (2 * 2 + 2);
                let mut cp2 = cp + w + toskew;
                while h >= 2 {
                    let mut x = w;
                    while x >= 2 {
                        let cb = p(pp, 4) as i32;
                        let cr = p(pp, 5) as i32;
                        convert(raster, cp, p(pp, 0), cb, cr);
                        convert(raster, cp + 1, p(pp, 1), cb, cr);
                        convert(raster, cp2, p(pp, 2), cb, cr);
                        convert(raster, cp2 + 1, p(pp, 3), cb, cr);
                        cp += 2;
                        cp2 += 2;
                        pp += 6;
                        x -= 2;
                    }
                    if x == 1 {
                        let cb = p(pp, 4) as i32;
                        let cr = p(pp, 5) as i32;
                        convert(raster, cp, p(pp, 0), cb, cr);
                        convert(raster, cp2, p(pp, 2), cb, cr);
                        cp += 1;
                        cp2 += 1;
                        pp += 6;
                    }
                    cp += incr;
                    cp2 += incr;
                    pp += fromskew;
                    h -= 2;
                }
                if h == 1 {
                    let mut x = w;
                    while x >= 2 {
                        let cb = p(pp, 4) as i32;
                        let cr = p(pp, 5) as i32;
                        convert(raster, cp, p(pp, 0), cb, cr);
                        convert(raster, cp + 1, p(pp, 1), cb, cr);
                        cp += 2;
                        pp += 6;
                        x -= 2;
                    }
                    if x == 1 {
                        let cb = p(pp, 4) as i32;
                        let cr = p(pp, 5) as i32;
                        convert(raster, cp, p(pp, 0), cb, cr);
                    }
                }
            }
            Put::YCbCr21 => {
                let fromskew = (fromskew / 2) * (2 + 2);
                loop {
                    for _ in 0..(w >> 1) {
                        let cb = p(pp, 2) as i32;
                        let cr = p(pp, 3) as i32;
                        convert(raster, cp, p(pp, 0), cb, cr);
                        convert(raster, cp + 1, p(pp, 1), cb, cr);
                        cp += 2;
                        pp += 4;
                    }
                    if (w & 1) != 0 {
                        let cb = p(pp, 2) as i32;
                        let cr = p(pp, 3) as i32;
                        convert(raster, cp, p(pp, 0), cb, cr);
                        cp += 1;
                        pp += 4;
                    }
                    cp += toskew;
                    pp += fromskew;
                    h -= 1;
                    if h == 0 {
                        break;
                    }
                }
            }
            Put::YCbCr12 => {
                let incr = 2 * toskew + w;
                let fromskew = fromskew * (2 + 2);
                let mut cp2 = cp + w + toskew;
                while h >= 2 {
                    for _ in 0..w.max(1) {
                        let cb = p(pp, 2) as i32;
                        let cr = p(pp, 3) as i32;
                        convert(raster, cp, p(pp, 0), cb, cr);
                        convert(raster, cp2, p(pp, 1), cb, cr);
                        cp += 1;
                        cp2 += 1;
                        pp += 4;
                    }
                    cp += incr;
                    cp2 += incr;
                    pp += fromskew;
                    h -= 2;
                }
                if h == 1 {
                    for _ in 0..w.max(1) {
                        let cb = p(pp, 2) as i32;
                        let cr = p(pp, 3) as i32;
                        convert(raster, cp, p(pp, 0), cb, cr);
                        cp += 1;
                        pp += 4;
                    }
                }
            }
            _ => {
                let fromskew = fromskew * 3;
                loop {
                    for _ in 0..w.max(1) {
                        let cb = p(pp, 1) as i32;
                        let cr = p(pp, 2) as i32;
                        convert(raster, cp, p(pp, 0), cb, cr);
                        cp += 1;
                        pp += 3;
                    }
                    cp += toskew;
                    pp += fromskew;
                    h -= 1;
                    if h <= 0 {
                        break;
                    }
                }
            }
        }
    }

    /// The separate put routines: red, green, blue and alpha planes.
    #[allow(clippy::too_many_arguments)]
    fn put_separate(&self, raster: &mut Raster, mut cp: i64, w: u32, h: u32, fromskew: i32, toskew: i32, planes: [Source; 4], mut positions: [i64; 4]) {
        let toskew = toskew as i64;
        let wide = matches!(self.put, Put::SeparateRgb16 | Put::SeparateRgbaAssociated16 | Put::SeparateRgbaUnassociated16);
        let step: i64 = if wide { 2 } else { 1 };
        for _ in 0..h {
            for _ in 0..w {
                let sample = |plane: usize| -> u32 {
                    if wide {
                        self.sixteen_to_eight[planes[plane].word(positions[plane]) as usize] as u32
                    } else {
                        planes[plane].byte(positions[plane]) as u32
                    }
                };
                let value = match self.put {
                    Put::SeparateRgb8 | Put::SeparateRgb16 => pack(sample(0), sample(1), sample(2)),
                    Put::SeparateRgbaAssociated8 | Put::SeparateRgbaAssociated16 => pack4(sample(0), sample(1), sample(2), sample(3)),
                    Put::SeparateCmyk8 => {
                        let k = 255 - sample(3);
                        let r = (k * (255 - sample(0))) / 255;
                        let g = (k * (255 - sample(1))) / 255;
                        let b = (k * (255 - sample(2))) / 255;
                        pack4(r, g, b, 255)
                    }
                    Put::SeparateRgbaUnassociated8 | Put::SeparateRgbaUnassociated16 => {
                        let a = sample(3) as usize;
                        let m = |value: u32| self.ua_to_aa[(a << 8) + value as usize] as u32;
                        pack4(m(sample(0)), m(sample(1)), m(sample(2)), a as u32)
                    }
                    _ => {
                        let tables = self.ycbcr.as_ref().expect("YCbCr tables");
                        let (r, g, b) = tables.to_rgb(sample(0), sample(1) as i32, sample(2) as i32);
                        pack(r, g, b)
                    }
                };
                raster.set(cp, value);
                cp += 1;
                let uses_alpha = !matches!(self.put, Put::SeparateRgb8 | Put::SeparateRgb16 | Put::SeparateYCbCr11);
                let advanced = if uses_alpha { 4 } else { 3 };
                for position in positions.iter_mut().take(advanced) {
                    *position += step;
                }
            }
            let uses_alpha = !matches!(self.put, Put::SeparateRgb8 | Put::SeparateRgb16 | Put::SeparateYCbCr11);
            let skewed = if uses_alpha { 4 } else { 3 };
            for position in positions.iter_mut().take(skewed) {
                *position += step * fromskew as i64;
            }
            cp += toskew;
        }
    }
}

/// `TIFFReadRGBAImage(tif, width, height, raster, 0)`: the image into a
/// bottom-left raster of `width` × `height`.
pub(super) fn read_rgba_image(tiff: &mut Tiff, decoder: &mut Decoder, width: u32, height: u32, raster: &mut [u32]) -> Result<()> {
    let image = begin(tiff)?;
    let tiff = &*tiff;
    let mut raster = Raster { pixels: raster };
    let separate = tiff.dir.planar_config == PLANARCONFIG_SEPARATE && tiff.dir.samples_per_pixel > 1;
    // `TIFFRGBAImageGet` keeps the height within the image.
    let h = height.min(image.height);
    let offset = (height - h) as i64 * width as i64;
    match (tiff.dir.tiled, separate) {
        (false, false) => strips_contig(tiff, decoder, &image, &mut raster, offset, width, h),
        (false, true) => strips_separate(tiff, decoder, &image, &mut raster, offset, width, h),
        (true, false) => tiles_contig(tiff, decoder, &image, &mut raster, offset, width, h),
        (true, true) => tiles_separate(tiff, decoder, &image, &mut raster, offset, width, h),
    }
}

/// `setorientation` with ImageMagick's bottom-left request.
fn orientation_flip(image: &RgbaImage) -> u32 {
    let _ = ORIENTATION_BOTLEFT;
    flip_for(image.orientation)
}

fn flip_horizontally(raster: &mut Raster, offset: i64, width: u32, h: u32, wmin: u32) {
    for line in 0..h as i64 {
        let start = offset + line * width as i64;
        let mut left = start;
        let mut right = start + wmin as i64 - 1;
        while left < right {
            let (Ok(l), Ok(r)) = (usize::try_from(left), usize::try_from(right)) else {
                break;
            };
            if r < raster.pixels.len() {
                raster.pixels.swap(l, r);
            }
            left += 1;
            right -= 1;
        }
    }
}

/// `gtStripContig`.
fn strips_contig(tiff: &Tiff, decoder: &mut Decoder, image: &RgbaImage, raster: &mut Raster, offset: i64, w: u32, h: u32) -> Result<()> {
    let wmin = w.min(image.width);
    let [_, vertical] = tiff.dir.ycbcr_subsampling;
    if vertical == 0 {
        bail!("Invalid vertical YCbCr subsampling");
    }
    let max_strip_size = tiff.strip_size()?;
    let flip = orientation_flip(image);
    let (mut y, toskew) = if flip & FLIP_VERTICALLY != 0 { (h as i64 - 1, -((w + wmin) as i32)) } else { (0, (w - wmin) as i32) };
    let rows_per_strip = tiff.dir.rows_per_strip;
    if rows_per_strip == 0 {
        bail!("rowsperstrip is zero");
    }
    let scanline = tiff.scanline_size()?;
    let fromskew = if w < image.width { (image.width - w) as i32 } else { 0 };
    let mut buffer: Option<Vec<u8>> = None;
    let mut row = 0u32;
    while row < h {
        let rows_to_read = rows_per_strip - row % rows_per_strip;
        let nrow = if row + rows_to_read > h { h - row } else { rows_to_read };
        let mut nrow_sub = nrow;
        if nrow_sub % vertical as u32 != 0 {
            nrow_sub += vertical as u32 - nrow_sub % vertical as u32;
        }
        let temp = (row % rows_per_strip + nrow_sub) as u64;
        let strip = row / rows_per_strip;
        let direct = buffer.is_some();
        let mut stored = buffer.take().unwrap_or_else(|| vec![0u8; max_strip_size as usize]);
        let read = tiff.read_encoded_strip_with(decoder, strip, &mut stored, Some(temp * scanline), direct)?;
        // The allocating read has no buffer until the raw data is in.
        if read == Outcome::FillFailed && !direct {
            break;
        }
        buffer = Some(stored);
        let bytes = buffer.as_deref().unwrap_or(&[]);
        let position = ((row % rows_per_strip) as u64 * scanline) as i64;
        let cp = offset + y * w as i64;
        image.put_contig(raster, cp, wmin, nrow, fromskew, toskew, Source { bytes }, position);
        y += if flip & FLIP_VERTICALLY != 0 { -(nrow as i64) } else { nrow as i64 };
        row += nrow;
    }
    if flip & FLIP_HORIZONTALLY != 0 {
        flip_horizontally(raster, offset, w, h, wmin);
    }
    Ok(())
}

/// `gtStripSeparate`.
fn strips_separate(tiff: &Tiff, decoder: &mut Decoder, image: &RgbaImage, raster: &mut Raster, offset: i64, w: u32, h: u32) -> Result<()> {
    let wmin = w.min(image.width);
    let flip = orientation_flip(image);
    let (mut y, toskew) = if flip & FLIP_VERTICALLY != 0 { (h as i64 - 1, -((w + wmin) as i32)) } else { (0, (w - wmin) as i32) };
    let colour_channels: u16 = match image.photometric {
        PHOTOMETRIC_MINISWHITE | PHOTOMETRIC_MINISBLACK | PHOTOMETRIC_PALETTE => 1,
        _ => 3,
    };
    let rows_per_strip = tiff.dir.rows_per_strip;
    if rows_per_strip == 0 {
        bail!("rowsperstrip is zero");
    }
    let scanline = tiff.scanline_size()?;
    let fromskew = if w < image.width { (image.width - w) as i32 } else { 0 };
    let alpha = image.alpha != 0;
    let mut planes: [Option<Vec<u8>>; 4] = [None, None, None, None];
    let mut row = 0u32;
    while row < h {
        let rows_to_read = rows_per_strip - row % rows_per_strip;
        let nrow = if row + rows_to_read > h { h - row } else { rows_to_read };
        let temp = (row % rows_per_strip + nrow) as u64;
        let strips_per_plane = tiff.dir.strips_per_image;
        let strip_for = |plane: u16| row / rows_per_strip + plane as u32 * strips_per_plane;
        let strip_size = tiff.strip_size()? as usize;
        let mut read_plane = |plane: u16, index: usize, planes: &mut [Option<Vec<u8>>; 4]| -> Result<bool> {
            let direct = planes[0].is_some();
            let mut stored = planes[index].take().unwrap_or_else(|| vec![0u8; strip_size]);
            let read = tiff.read_encoded_strip_with(decoder, strip_for(plane), &mut stored, Some(temp * scanline), direct)?;
            if read == Outcome::FillFailed && !direct {
                return Ok(false);
            }
            planes[index] = Some(stored);
            Ok(true)
        };
        if !read_plane(0, 0, &mut planes)? {
            break;
        }
        if colour_channels > 1 {
            read_plane(1, 1, &mut planes)?;
            read_plane(2, 2, &mut planes)?;
        }
        if alpha {
            read_plane(colour_channels, 3, &mut planes)?;
        }
        let empty: &[u8] = &[];
        let first = planes[0].as_deref().unwrap_or(empty);
        let source = |index: usize| -> Source {
            if colour_channels == 1 && index < 3 {
                return Source { bytes: first };
            }
            Source { bytes: planes[index].as_deref().unwrap_or(empty) }
        };
        let position = ((row % rows_per_strip) as u64 * scanline) as i64;
        let cp = offset + y * w as i64;
        let sources = [source(0), source(1), source(2), source(3)];
        image.put_separate(raster, cp, wmin, nrow, fromskew, toskew, sources, [position; 4]);
        y += if flip & FLIP_VERTICALLY != 0 { -(nrow as i64) } else { nrow as i64 };
        row += nrow;
    }
    if flip & FLIP_HORIZONTALLY != 0 {
        flip_horizontally(raster, offset, w, h, wmin);
    }
    Ok(())
}

/// `gtTileContig`.
fn tiles_contig(tiff: &Tiff, decoder: &mut Decoder, image: &RgbaImage, raster: &mut Raster, offset: i64, w: u32, h: u32) -> Result<()> {
    let wmin = w.min(image.width);
    let tile_width = tiff.dir.tile_width;
    let tile_length = tiff.dir.tile_length;
    let flip = orientation_flip(image);
    let (mut y, toskew) = if flip & FLIP_VERTICALLY != 0 { (h as i64 - 1, -((tile_width + w) as i32)) } else { (0, -((tile_width as i64 - w as i64) as i32)) };
    if tile_width == 0 || tile_length == 0 {
        bail!("tile width or height is zero");
    }
    let row_size = tiff.tile_row_size()?;
    let tile_size = tiff.tile_size()? as usize;
    let mut buffer: Option<Vec<u8>> = None;
    let mut row = 0u32;
    while row < h {
        let rows_to_read = tile_length - row % tile_length;
        let nrow = if row + rows_to_read > h { h - row } else { rows_to_read };
        let mut fromskew = 0i32;
        let mut this_width = tile_width;
        let mut this_toskew = toskew;
        let mut to_column = 0u32;
        let mut column = 0u32;
        while to_column < wmin {
            let had_buffer = buffer.is_some();
            let mut stored = buffer.take().unwrap_or_else(|| vec![0u8; tile_size]);
            let read = tiff.read_tile(decoder, column, row, 0, &mut stored)?;
            if read == Outcome::FillFailed && !had_buffer {
                break;
            }
            buffer = Some(stored);
            let position = ((row % tile_length) as u64 * row_size) as i64 + fromskew as i64 * image.samples as i64;
            if to_column + this_width > wmin {
                fromskew = (tile_width - (wmin - to_column)) as i32;
                this_width = tile_width - fromskew as u32;
                this_toskew = toskew + fromskew;
            }
            let cp = offset + y * w as i64 + to_column as i64;
            let bytes = buffer.as_deref().unwrap_or(&[]);
            image.put_contig(raster, cp, this_width, nrow, fromskew, this_toskew, Source { bytes }, position);
            to_column += this_width;
            column += this_width;
            fromskew = 0;
            this_width = tile_width;
            this_toskew = toskew;
        }
        y += if flip & FLIP_VERTICALLY != 0 { -(nrow as i64) } else { nrow as i64 };
        row += nrow;
    }
    if flip & FLIP_HORIZONTALLY != 0 {
        flip_horizontally(raster, offset, w, h, wmin);
    }
    Ok(())
}

/// `gtTileSeparate`.
fn tiles_separate(tiff: &Tiff, decoder: &mut Decoder, image: &RgbaImage, raster: &mut Raster, offset: i64, w: u32, h: u32) -> Result<()> {
    let wmin = w.min(image.width);
    let tile_width = tiff.dir.tile_width;
    let tile_length = tiff.dir.tile_length;
    let flip = orientation_flip(image);
    let (mut y, toskew) = if flip & FLIP_VERTICALLY != 0 { (h as i64 - 1, -((tile_width + w) as i32)) } else { (0, -((tile_width as i64 - w as i64) as i32)) };
    let colour_channels: u16 = match image.photometric {
        PHOTOMETRIC_MINISWHITE | PHOTOMETRIC_MINISBLACK | PHOTOMETRIC_PALETTE => 1,
        _ => 3,
    };
    if tile_width == 0 || tile_length == 0 {
        bail!("tile width or height is zero");
    }
    let row_size = tiff.tile_row_size()?;
    let tile_size = tiff.tile_size()? as usize;
    let alpha = image.alpha != 0;
    let mut planes: [Option<Vec<u8>>; 4] = [None, None, None, None];
    let mut row = 0u32;
    while row < h {
        let rows_to_read = tile_length - row % tile_length;
        let nrow = if row + rows_to_read > h { h - row } else { rows_to_read };
        let mut fromskew = 0i32;
        let mut this_width = tile_width;
        let mut this_toskew = toskew;
        let mut to_column = 0u32;
        let mut column = 0u32;
        while to_column < wmin {
            let mut read_plane = |plane: u16, index: usize, planes: &mut [Option<Vec<u8>>; 4]| -> Result<bool> {
                let had_buffer = planes[0].is_some();
                let mut stored = planes[index].take().unwrap_or_else(|| vec![0u8; tile_size]);
                let read = tiff.read_tile(decoder, column, row, plane, &mut stored)?;
                if read == Outcome::FillFailed && !had_buffer {
                    return Ok(false);
                }
                planes[index] = Some(stored);
                Ok(true)
            };
            if !read_plane(0, 0, &mut planes)? {
                break;
            }
            if colour_channels > 1 {
                read_plane(1, 1, &mut planes)?;
                read_plane(2, 2, &mut planes)?;
            }
            if alpha {
                read_plane(colour_channels, 3, &mut planes)?;
            }
            let position = ((row % tile_length) as u64 * row_size) as i64 + fromskew as i64;
            if to_column + this_width > wmin {
                fromskew = (tile_width - (wmin - to_column)) as i32;
                this_width = tile_width - fromskew as u32;
                this_toskew = toskew + fromskew;
            }
            let empty: &[u8] = &[];
            let first = planes[0].as_deref().unwrap_or(empty);
            let source = |index: usize| -> Source {
                if colour_channels == 1 && index < 3 {
                    return Source { bytes: first };
                }
                Source { bytes: planes[index].as_deref().unwrap_or(empty) }
            };
            let cp = offset + y * w as i64 + to_column as i64;
            let sources = [source(0), source(1), source(2), source(3)];
            image.put_separate(raster, cp, this_width, nrow, fromskew, this_toskew, sources, [position; 4]);
            to_column += this_width;
            column += this_width;
            fromskew = 0;
            this_width = tile_width;
            this_toskew = toskew;
        }
        y += if flip & FLIP_VERTICALLY != 0 { -(nrow as i64) } else { nrow as i64 };
        row += nrow;
    }
    if flip & FLIP_HORIZONTALLY != 0 {
        flip_horizontally(raster, offset, w, h, wmin);
    }
    Ok(())
}
