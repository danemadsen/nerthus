//! libtiff's SGI LogLuv and LogL codecs (`tif_luv.c`): each row's bytes
//! run-length coded plane by plane (most significant first), or packed
//! 24-bit LogLuv; then converted as the reader asked — through the RGBA
//! interface to 8-bit RGB (XYZ through CCIR-709 primaries, a square-root
//! gamma) or 8-bit grey, otherwise to the format the tags suggest.

use anyhow::{bail, Result};

use super::directory::*;

const U_NEU: f64 = 0.210526316;
const V_NEU: f64 = 0.473684211;
const UVSCALE: f64 = 410.0;
const LN2: f64 = std::f64::consts::LN_2;

const UV_SQSIZ: f32 = 0.003500;
const UV_NDIVS: i32 = 16289;
const UV_VSTART: f32 = 0.016940;

/// `uv_row`: each row of the (u', v') grid's start, cells and cumulative
/// count.
#[rustfmt::skip]
const UV_ROWS: [(f32, i16, i16); 163] = [
    (0.247663, 4, 0), (0.243779, 6, 4), (0.241684, 7, 10), (0.237874, 9, 17),
    (0.235906, 10, 26), (0.232153, 12, 36), (0.228352, 14, 48), (0.226259, 15, 62),
    (0.222371, 17, 77), (0.220410, 18, 94), (0.214710, 21, 112), (0.212714, 22, 133),
    (0.210721, 23, 155), (0.204976, 26, 178), (0.202986, 27, 204), (0.199245, 29, 231),
    (0.195525, 31, 260), (0.193560, 32, 291), (0.189878, 34, 323), (0.186216, 36, 357),
    (0.186216, 36, 393), (0.182592, 38, 429), (0.179003, 40, 467), (0.175466, 42, 507),
    (0.172001, 44, 549), (0.172001, 44, 593), (0.168612, 46, 637), (0.168612, 46, 683),
    (0.163575, 49, 729), (0.158642, 52, 778), (0.158642, 52, 830), (0.158642, 52, 882),
    (0.153815, 55, 934), (0.153815, 55, 989), (0.149097, 58, 1044), (0.149097, 58, 1102),
    (0.142746, 62, 1160), (0.142746, 62, 1222), (0.142746, 62, 1284), (0.138270, 65, 1346),
    (0.138270, 65, 1411), (0.138270, 65, 1476), (0.132166, 69, 1541), (0.132166, 69, 1610),
    (0.126204, 73, 1679), (0.126204, 73, 1752), (0.126204, 73, 1825), (0.120381, 77, 1898),
    (0.120381, 77, 1975), (0.120381, 77, 2052), (0.120381, 77, 2129), (0.112962, 82, 2206),
    (0.112962, 82, 2288), (0.112962, 82, 2370), (0.107450, 86, 2452), (0.107450, 86, 2538),
    (0.107450, 86, 2624), (0.107450, 86, 2710), (0.100343, 91, 2796), (0.100343, 91, 2887),
    (0.100343, 91, 2978), (0.095126, 95, 3069), (0.095126, 95, 3164), (0.095126, 95, 3259),
    (0.095126, 95, 3354), (0.088276, 100, 3449), (0.088276, 100, 3549), (0.088276, 100, 3649),
    (0.088276, 100, 3749), (0.081523, 105, 3849), (0.081523, 105, 3954), (0.081523, 105, 4059),
    (0.081523, 105, 4164), (0.074861, 110, 4269), (0.074861, 110, 4379), (0.074861, 110, 4489),
    (0.074861, 110, 4599), (0.068290, 115, 4709), (0.068290, 115, 4824), (0.068290, 115, 4939),
    (0.068290, 115, 5054), (0.063573, 119, 5169), (0.063573, 119, 5288), (0.063573, 119, 5407),
    (0.063573, 119, 5526), (0.057219, 124, 5645), (0.057219, 124, 5769), (0.057219, 124, 5893),
    (0.057219, 124, 6017), (0.050985, 129, 6141), (0.050985, 129, 6270), (0.050985, 129, 6399),
    (0.050985, 129, 6528), (0.050985, 129, 6657), (0.044859, 134, 6786), (0.044859, 134, 6920),
    (0.044859, 134, 7054), (0.044859, 134, 7188), (0.040571, 138, 7322), (0.040571, 138, 7460),
    (0.040571, 138, 7598), (0.040571, 138, 7736), (0.036339, 142, 7874), (0.036339, 142, 8016),
    (0.036339, 142, 8158), (0.036339, 142, 8300), (0.032139, 146, 8442), (0.032139, 146, 8588),
    (0.032139, 146, 8734), (0.032139, 146, 8880), (0.027947, 150, 9026), (0.027947, 150, 9176),
    (0.027947, 150, 9326), (0.023739, 154, 9476), (0.023739, 154, 9630), (0.023739, 154, 9784),
    (0.023739, 154, 9938), (0.019504, 158, 10092), (0.019504, 158, 10250), (0.019504, 158, 10408),
    (0.016976, 161, 10566), (0.016976, 161, 10727), (0.016976, 161, 10888), (0.016976, 161, 11049),
    (0.012639, 165, 11210), (0.012639, 165, 11375), (0.012639, 165, 11540), (0.009991, 168, 11705),
    (0.009991, 168, 11873), (0.009991, 168, 12041), (0.009016, 170, 12209), (0.009016, 170, 12379),
    (0.009016, 170, 12549), (0.006217, 173, 12719), (0.006217, 173, 12892), (0.005097, 175, 13065),
    (0.005097, 175, 13240), (0.005097, 175, 13415), (0.003909, 177, 13590), (0.003909, 177, 13767),
    (0.002340, 177, 13944), (0.002389, 170, 14121), (0.001068, 164, 14291), (0.001653, 157, 14455),
    (0.000717, 150, 14612), (0.001614, 143, 14762), (0.000270, 136, 14905), (0.000484, 129, 15041),
    (0.001103, 123, 15170), (0.001242, 115, 15293), (0.001188, 109, 15408), (0.001011, 103, 15517),
    (0.000709, 97, 15620), (0.000301, 89, 15717), (0.002416, 82, 15806), (0.003251, 76, 15888),
    (0.003246, 69, 15964), (0.004141, 62, 16033), (0.005963, 55, 16095), (0.008839, 47, 16150),
    (0.010490, 40, 16197), (0.016994, 31, 16237), (0.023659, 21, 16268),
];

/// The user data formats (`SGILOGDATAFMT_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DataFormat {
    Float,
    Sixteen,
    Raw,
    Eight,
}

/// `LogL16GuessDataFmt`.
fn guess_logl(dir: &Directory) -> Option<DataFormat> {
    if dir.samples_per_pixel != 1 {
        return None;
    }
    match (dir.bits_per_sample, dir.sample_format) {
        (32, SAMPLEFORMAT_IEEEFP) => Some(DataFormat::Float),
        (16, 4 | SAMPLEFORMAT_INT | SAMPLEFORMAT_UINT) => Some(DataFormat::Sixteen),
        (8, 4 | SAMPLEFORMAT_UINT) => Some(DataFormat::Eight),
        _ => None,
    }
}

/// `LogLuvGuessDataFmt`.
fn guess_logluv(dir: &Directory) -> Option<DataFormat> {
    let guess = match (dir.bits_per_sample, dir.sample_format) {
        (32, SAMPLEFORMAT_IEEEFP) => Some(DataFormat::Float),
        (32, 4 | SAMPLEFORMAT_UINT | SAMPLEFORMAT_INT) => Some(DataFormat::Raw),
        (16, 4 | SAMPLEFORMAT_INT | SAMPLEFORMAT_UINT) => Some(DataFormat::Sixteen),
        (8, 4 | SAMPLEFORMAT_UINT) => Some(DataFormat::Eight),
        _ => None,
    };
    match dir.samples_per_pixel {
        1 if guess == Some(DataFormat::Raw) => guess,
        3 if guess != Some(DataFormat::Raw) => guess,
        _ => None,
    }
}

/// The data format and bytes per pixel (`LogLuvInitState`,
/// `LogL16InitState`).
fn format(tiff: &Tiff) -> Result<(DataFormat, usize)> {
    let dir = &tiff.dir;
    match dir.photometric {
        Some(PHOTOMETRIC_LOGLUV) => {
            if dir.planar_config != PLANARCONFIG_CONTIG {
                bail!("SGILog compression cannot handle non-contiguous data");
            }
            let format = if tiff.sgilog_8bit { Some(DataFormat::Eight) } else { guess_logluv(dir) };
            match format {
                Some(DataFormat::Float) => Ok((DataFormat::Float, 12)),
                Some(DataFormat::Sixteen) => Ok((DataFormat::Sixteen, 6)),
                Some(DataFormat::Raw) => Ok((DataFormat::Raw, 4)),
                Some(DataFormat::Eight) => Ok((DataFormat::Eight, 3)),
                None => bail!("No support for converting user data format to LogLuv"),
            }
        }
        Some(PHOTOMETRIC_LOGL) => {
            if dir.samples_per_pixel != 1 {
                bail!("Sorry, can not handle LogL image with Samples/pixel={}", dir.samples_per_pixel);
            }
            let format = if tiff.sgilog_8bit { Some(DataFormat::Eight) } else { guess_logl(dir) };
            match format {
                Some(DataFormat::Float) => Ok((DataFormat::Float, 4)),
                Some(DataFormat::Sixteen) => Ok((DataFormat::Sixteen, 2)),
                Some(DataFormat::Eight) => Ok((DataFormat::Eight, 1)),
                _ => bail!("No support for converting user data format to LogL"),
            }
        }
        other => bail!("Inappropriate photometric interpretation {} for SGILog compression; must be either LogLUV or LogL", other.unwrap_or(0)),
    }
}

/// `LogLuvSetupDecode`.
pub(super) fn setup(tiff: &Tiff) -> Result<()> {
    format(tiff).map(|_| ())
}

/// `LogL16toY`.
fn logl16_to_y(p16: i32) -> f64 {
    let le = p16 & 0x7fff;
    if le == 0 {
        return 0.0;
    }
    let y = (LN2 / 256.0 * (le as f64 + 0.5) - LN2 * 64.0).exp();
    if p16 & 0x8000 == 0 {
        y
    } else {
        -y
    }
}

/// `LogL10toY`.
fn logl10_to_y(p10: i32) -> f64 {
    if p10 == 0 {
        return 0.0;
    }
    (LN2 / 64.0 * (p10 as f64 + 0.5) - LN2 * 12.0).exp()
}

/// `uv_decode`: the centre of grid cell `code`.
fn uv_decode(code: i32) -> Option<(f64, f64)> {
    if !(0..UV_NDIVS).contains(&code) {
        return None;
    }
    let mut lower = 0usize;
    let mut upper = UV_ROWS.len();
    while upper - lower > 1 {
        let middle = (lower + upper) >> 1;
        let offset = code - UV_ROWS[middle].2 as i32;
        if offset > 0 {
            lower = middle;
        } else if offset < 0 {
            upper = middle;
        } else {
            lower = middle;
            break;
        }
    }
    let row = UV_ROWS[lower];
    let ui = code - row.2 as i32;
    let u = row.0 as f64 + (ui as f64 + 0.5) * UV_SQSIZ as f64;
    let v = UV_VSTART as f64 + (lower as f64 + 0.5) * UV_SQSIZ as f64;
    Some((u, v))
}

/// XYZ from luminance and (u', v').
fn uv_to_xyz(luminance: f64, u: f64, v: f64) -> [f32; 3] {
    let s = 1.0 / (6.0 * u - 16.0 * v + 12.0);
    let x = 9.0 * u * s;
    let y = 4.0 * v * s;
    [(x / y * luminance) as f32, luminance as f32, ((1.0 - x - y) / y * luminance) as f32]
}

/// `LogLuv24toXYZ`.
fn logluv24_to_xyz(p: u32) -> [f32; 3] {
    let luminance = logl10_to_y((p >> 14 & 0x3ff) as i32);
    if luminance <= 0.0 {
        return [0.0; 3];
    }
    let (u, v) = uv_decode((p & 0x3fff) as i32).unwrap_or((U_NEU, V_NEU));
    uv_to_xyz(luminance, u, v)
}

/// `LogLuv32toXYZ`.
fn logluv32_to_xyz(p: u32) -> [f32; 3] {
    let luminance = logl16_to_y((p as i32) >> 16);
    if luminance <= 0.0 {
        return [0.0; 3];
    }
    let u = 1.0 / UVSCALE * ((p >> 8 & 0xff) as f64 + 0.5);
    let v = 1.0 / UVSCALE * ((p & 0xff) as f64 + 0.5);
    uv_to_xyz(luminance, u, v)
}

/// `(int)(256 * sqrt(value))`, clamped as libtiff clamps.
fn gamma_byte(value: f64) -> u8 {
    if value <= 0.0 {
        0
    } else if value >= 1.0 {
        255
    } else {
        (256.0 * value.sqrt()) as i32 as u8
    }
}

/// `XYZtoRGB24`.
fn xyz_to_rgb24(xyz: [f32; 3]) -> [u8; 3] {
    let (x, y, z) = (xyz[0] as f64, xyz[1] as f64, xyz[2] as f64);
    let r = 2.690 * x + -1.276 * y + -0.414 * z;
    let g = -1.022 * x + 1.978 * y + 0.044 * z;
    let b = 0.061 * x + -0.224 * y + 1.163 * z;
    [gamma_byte(r), gamma_byte(g), gamma_byte(b)]
}

/// The run-length byte planes of one row (`LogL16Decode`,
/// `LogLuvDecode32`): `planes` bytes per value, most significant first.
fn decode_planes(raw: &[u8], position: &mut usize, count: usize, planes: u32) -> Result<Vec<u32>> {
    let mut values = vec![0u32; count];
    let mut left = raw.len() - *position;
    for plane in (0..planes).rev() {
        let shift = plane * 8;
        let mut index = 0usize;
        while index < count && left > 0 {
            let byte = raw[*position];
            if byte >= 128 {
                if left < 2 {
                    break;
                }
                let mut run = byte as i32 + (2 - 128);
                let value = (raw[*position + 1] as u32) << shift;
                *position += 2;
                left -= 2;
                while run > 0 && index < count {
                    values[index] |= value;
                    index += 1;
                    run -= 1;
                }
            } else {
                let mut run = byte as i32;
                *position += 1;
                loop {
                    left -= 1;
                    if left == 0 {
                        break;
                    }
                    let more = run > 0;
                    run -= 1;
                    if !more || index >= count {
                        break;
                    }
                    values[index] |= (raw[*position] as u32) << shift;
                    *position += 1;
                    index += 1;
                }
            }
        }
        if index != count {
            bail!("Not enough data at row (short {} pixels)", count - index);
        }
    }
    Ok(values)
}

/// Decodes a strip or tile row by row (`LogLuvDecodeStrip`,
/// `LogLuvDecodeTile`).
pub(super) fn decode(tiff: &Tiff, raw: &[u8], output: &mut [u8], is_tile: bool) -> Result<()> {
    let (format, pixel_size) = format(tiff)?;
    let row_length = if is_tile { tiff.tile_row_size()? } else { tiff.scanline_size()? } as usize;
    if row_length == 0 {
        bail!("SGILog: zero row length");
    }
    let logl = tiff.dir.photometric == Some(PHOTOMETRIC_LOGL);
    let packed24 = tiff.dir.compression == COMPRESSION_SGILOG24;
    let mut position = 0usize;
    for row in output.chunks_mut(row_length) {
        let count = row.len() / pixel_size;
        if logl {
            let values = decode_planes(raw, &mut position, count, 2)?;
            for (index, &value) in values.iter().enumerate() {
                let p16 = value as u16 as i16 as i32;
                match format {
                    DataFormat::Eight => row[index] = gamma_byte(logl16_to_y(p16)),
                    DataFormat::Float => {
                        let y = logl16_to_y(p16) as f32;
                        row[index * 4..index * 4 + 4].copy_from_slice(&y.to_le_bytes());
                    }
                    _ => row[index * 2..index * 2 + 2].copy_from_slice(&(value as u16).to_le_bytes()),
                }
            }
            continue;
        }
        let values = if packed24 {
            let mut values = Vec::with_capacity(count);
            while values.len() < count && raw.len() - position >= 3 {
                let value = (raw[position] as u32) << 16 | (raw[position + 1] as u32) << 8 | raw[position + 2] as u32;
                values.push(value);
                position += 3;
            }
            if values.len() != count {
                bail!("Not enough data at row (short {} pixels)", count - values.len());
            }
            values
        } else {
            decode_planes(raw, &mut position, count, 4)?
        };
        for (index, &value) in values.iter().enumerate() {
            let xyz = || if packed24 { logluv24_to_xyz(value) } else { logluv32_to_xyz(value) };
            match format {
                DataFormat::Eight => row[index * 3..index * 3 + 3].copy_from_slice(&xyz_to_rgb24(xyz())),
                DataFormat::Float => {
                    for (channel, component) in xyz().iter().enumerate() {
                        let start = index * 12 + channel * 4;
                        row[start..start + 4].copy_from_slice(&component.to_le_bytes());
                    }
                }
                DataFormat::Sixteen => {
                    let (luminance, u, v) = if packed24 {
                        let (u, v) = uv_decode((value & 0x3fff) as i32).unwrap_or((U_NEU, V_NEU));
                        (((value >> 12 & 0xffd) + 13314) as i16, u, v)
                    } else {
                        let u = 1.0 / UVSCALE * ((value >> 8 & 0xff) as f64 + 0.5);
                        let v = 1.0 / UVSCALE * ((value & 0xff) as f64 + 0.5);
                        ((value >> 16) as i16, u, v)
                    };
                    let triple = [luminance, (u * 32768.0) as i16, (v * 32768.0) as i16];
                    for (channel, component) in triple.iter().enumerate() {
                        let start = index * 6 + channel * 2;
                        row[start..start + 2].copy_from_slice(&component.to_le_bytes());
                    }
                }
                DataFormat::Raw => row[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes()),
            }
        }
    }
    Ok(())
}
