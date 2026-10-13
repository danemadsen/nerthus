//! File loading: raw float and 16-bit heightfields, and images through
//! Magick.NET.
//!
//! An image is read straight from ImageMagick's pixel cache, so its
//! quirks reach the map: an 8-bit grey PNG (a palette image to IM) or a
//! grey image with alpha gives a map interleaving its grey with its index
//! or alpha, taken from the top half of the image; colour images give
//! their red channel unless kept as colour.

use std::path::Path;

use nerthus_engine::{Map, Resolution};
use nerthus_image::{quanta, Colorspace, Image};
use nerthus_io::pixel_cache::{read_pixel_cache, CacheColorspace, PixelCache, PixelChannel};

/// `1f / Quantum.Max`: a map is divided by a scalar as a multiplication
/// by its reciprocal.
const QUANTUM_SCALE: f32 = 1.0 / 65535.0;
/// IM's `QuantumRange` and `QuantumScale`.
const QUANTUM_RANGE: f64 = 65535.0;
const QUANTUM_SCALE_F64: f64 = 1.0 / 65535.0;

/// Whether a channel is one of the image's colours (grey, red, green,
/// blue or black).
fn is_colour(channel: PixelChannel) -> bool {
    !matches!(channel, PixelChannel::Index | PixelChannel::Alpha | PixelChannel::Meta)
}

/// The grey or red, green and blue channels of the cache as an `Image` for
/// IM's colour operations.
fn colour_image(cache: &PixelCache) -> Image {
    let kept: Vec<usize> = (0..cache.channels.len()).filter(|&index| is_colour(cache.channels[index])).collect();
    let stride = cache.channels.len();
    let mut data = Vec::with_capacity(cache.width * cache.height * kept.len());
    for pixel in cache.data.chunks_exact(stride) {
        for &index in &kept {
            data.push(pixel[index]);
        }
    }
    let mut image = Image::from_data(cache.width, cache.height, kept.len(), data);
    // CMYK and Lab caches are taken to sRGB before they get here.
    image.colorspace = match cache.colorspace {
        CacheColorspace::SRgb | CacheColorspace::Cmyk | CacheColorspace::Lab => Colorspace::SRgb,
        CacheColorspace::Rgb => Colorspace::LinearRgb,
        CacheColorspace::Gray => Colorspace::Gray,
        CacheColorspace::LinearGray => Colorspace::LinearGray,
    };
    image
}

/// The channels a colorspace transform keeps as they are: alpha and meta
/// channels, in order, with their samples.
fn extra_channels(cache: &PixelCache) -> Vec<(PixelChannel, Vec<f32>)> {
    let stride = cache.channels.len();
    let mut extras = Vec::new();
    for (position, channel) in cache.channels.iter().enumerate() {
        if !matches!(channel, PixelChannel::Alpha | PixelChannel::Meta) {
            continue;
        }
        let samples: Vec<f32> = cache.data.chunks_exact(stride).map(|pixel| pixel[position]).collect();
        extras.push((*channel, samples));
    }
    extras
}

/// A cache of new colours (`colours`, `colour_channels` per pixel) and the
/// extra channels after them.
fn with_extras(colours: &[f32], colour_channels: Vec<PixelChannel>, extras: Vec<(PixelChannel, Vec<f32>)>, model: &PixelCache, colorspace: CacheColorspace, gamma: f64) -> PixelCache {
    let colour_count = colour_channels.len();
    let mut channels = colour_channels;
    channels.extend(extras.iter().map(|(channel, _)| *channel));
    let mut data = Vec::with_capacity(model.width * model.height * channels.len());
    for (index, pixel) in colours.chunks_exact(colour_count).enumerate() {
        data.extend_from_slice(pixel);
        for (_, samples) in &extras {
            data.push(samples[index]);
        }
    }
    PixelCache { width: model.width, height: model.height, channels, data, colorspace, gamma }
}

/// The cache after a transform to grey: `DirectClass` (no index), grey
/// plus the extra channels.
fn from_gray_image(image: Image, extras: Vec<(PixelChannel, Vec<f32>)>, model: &PixelCache, colorspace: CacheColorspace, gamma: f64) -> PixelCache {
    with_extras(&image.data, vec![PixelChannel::Gray], extras, model, colorspace, gamma)
}

/// `TransformsRGBImage` from CMYK: `ConvertCMYKToRGB` on each pixel, then
/// the black channel goes.
fn cmyk_to_srgb(cache: &PixelCache) -> PixelCache {
    let stride = cache.channels.len();
    let position = |wanted: PixelChannel| cache.channels.iter().position(|channel| *channel == wanted).unwrap_or(0);
    let (cyan, magenta, yellow, black) = (position(PixelChannel::Red), position(PixelChannel::Green), position(PixelChannel::Blue), position(PixelChannel::Black));
    let mut colours = Vec::with_capacity(cache.width * cache.height * 3);
    for pixel in cache.data.chunks_exact(stride) {
        let black = pixel[black] as f64;
        for ink in [pixel[cyan], pixel[magenta], pixel[yellow]] {
            let value = QUANTUM_RANGE - (QUANTUM_SCALE_F64 * ink as f64 * (QUANTUM_RANGE - black) + black);
            colours.push(value as f32);
        }
    }
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    with_extras(&colours, channels, extra_channels(cache), cache, CacheColorspace::SRgb, cache.gamma)
}

/// `TransformsRGBImage` from Lab (`ConvertLabToRGB` under D65): Lab to XYZ
/// to sRGB.
fn lab_to_srgb(cache: &PixelCache) -> PixelCache {
    const CIE_EPSILON: f64 = 216.0 / 24389.0;
    const CIE_K: f64 = 24389.0 / 27.0;
    const D65: [f64; 3] = [0.95047, 1.00000, 1.08883];
    let stride = cache.channels.len();
    let mut colours = Vec::with_capacity(cache.width * cache.height * 3);
    for pixel in cache.data.chunks_exact(stride) {
        let lightness = 100.0 * (QUANTUM_SCALE_F64 * pixel[0] as f64);
        let a = 255.0 * ((QUANTUM_SCALE_F64 * pixel[1] as f64) - 0.5);
        let b = 255.0 * ((QUANTUM_SCALE_F64 * pixel[2] as f64) - 0.5);
        // `ConvertLabToXYZ`.
        let mut y = (lightness + 16.0) / 116.0;
        let mut x = y + a / 500.0;
        let mut z = y - b / 200.0;
        x = if (x * x * x) > CIE_EPSILON { x * x * x } else { (116.0 * x - 16.0) / CIE_K };
        y = if lightness > (CIE_K * CIE_EPSILON) { y * y * y } else { lightness / CIE_K };
        z = if (z * z * z) > CIE_EPSILON { z * z * z } else { (116.0 * z - 16.0) / CIE_K };
        let (x, y, z) = (D65[0] * x, D65[1] * y, D65[2] * z);
        // `ConvertXYZToRGB`.
        let mut red = (3.240969941904521 * x) + (-1.537383177570093 * y) + (-0.498610760293 * z);
        let mut green = (-0.96924363628087 * x) + (1.87596750150772 * y) + (0.041555057407175 * z);
        let mut blue = (0.055630079696993 * x) + (-0.20397695888897 * y) + (1.056971514242878 * z);
        let smallest = if red < (if green < blue { green } else { blue }) { red } else if green < blue { green } else { blue };
        if smallest < 0.0 {
            red -= smallest;
            green -= smallest;
            blue -= smallest;
        }
        for value in [red, green, blue] {
            colours.push(quanta::encode_pixel_gamma(QUANTUM_RANGE * value) as f32);
        }
    }
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    with_extras(&colours, channels, extra_channels(cache), cache, CacheColorspace::SRgb, cache.gamma)
}

/// `TransformsRGBImage` from linear RGB: each colour encoded with the
/// sRGB curve (`EncodePixelGamma`).
fn rgb_to_srgb(cache: &PixelCache) -> PixelCache {
    let mut encoded = cache.clone();
    let stride = cache.channels.len();
    let colour_positions: Vec<usize> = (0..stride).filter(|&index| is_colour(cache.channels[index])).collect();
    for pixel in encoded.data.chunks_exact_mut(stride) {
        for &position in &colour_positions {
            pixel[position] = quanta::encode_pixel_gamma(pixel[position] as f64) as f32;
        }
    }
    encoded.colorspace = CacheColorspace::SRgb;
    encoded
}

/// `TransformsRGBImage`: linear RGB, CMYK and Lab caches as sRGB, others
/// as they are.
fn to_srgb(cache: &PixelCache) -> PixelCache {
    match cache.colorspace {
        CacheColorspace::Rgb => rgb_to_srgb(cache),
        CacheColorspace::Cmyk => cmyk_to_srgb(cache),
        CacheColorspace::Lab => lab_to_srgb(cache),
        _ => cache.clone(),
    }
}

/// `TransformImageColorspace` to LinearGRAY (through sRGB, IM 7.1.2).
fn to_linear_gray(cache: &PixelCache) -> PixelCache {
    let cache = to_srgb(cache);
    let extras = extra_channels(&cache);
    let image = colour_image(&cache);
    let linear = quanta::to_linear_gray(&image);
    from_gray_image(linear, extras, &cache, CacheColorspace::LinearGray, 1.0)
}

/// `TransformImageColorspace` from LinearGRAY to GRAY.
fn linear_to_gray(cache: &PixelCache) -> PixelCache {
    let extras = extra_channels(cache);
    let image = colour_image(cache);
    let gray = quanta::linear_to_gray(&image);
    from_gray_image(gray, extras, cache, CacheColorspace::Gray, cache.gamma)
}

/// `GammaImage` (Magick.NET `GammaCorrect`) on the colour channels; the
/// index, alpha and meta channels keep their values.
fn gamma_correct(cache: &mut PixelCache, gamma: f64) {
    let colour_positions: Vec<usize> = (0..cache.channels.len()).filter(|&index| is_colour(cache.channels[index])).collect();
    let stride = cache.channels.len();
    let mut colours = Vec::with_capacity(cache.width * cache.height * colour_positions.len());
    for pixel in cache.data.chunks_exact(stride) {
        for &position in &colour_positions {
            colours.push(pixel[position]);
        }
    }
    // Every colour sample on its own (a CMYK image's black included; an
    // image of four channels would take the last for alpha).
    let samples = colours.len();
    let mut image = Image::from_data(samples, 1, 1, colours);
    quanta::gamma(&mut image, gamma);
    for (pixel, corrected) in cache.data.chunks_exact_mut(stride).zip(image.data.chunks_exact(colour_positions.len())) {
        for (&position, &value) in colour_positions.iter().zip(corrected.iter()) {
            pixel[position] = value;
        }
    }
    if cache.gamma != 0.0 {
        cache.gamma *= gamma;
    }
}

/// The cache with its alpha channel gone (other channels kept).
fn without_alpha(cache: PixelCache) -> PixelCache {
    let Some(position) = cache.channels.iter().position(|channel| *channel == PixelChannel::Alpha) else {
        return cache;
    };
    let stride = cache.channels.len();
    let mut data = Vec::with_capacity(cache.width * cache.height * (stride - 1));
    for pixel in cache.data.chunks_exact(stride) {
        for (index, &value) in pixel.iter().enumerate() {
            if index != position {
                data.push(value);
            }
        }
    }
    let mut channels = cache.channels;
    channels.remove(position);
    PixelCache { channels, data, ..cache }
}

/// The cache as a map. A cache of fewer than three
/// channels is made GRAY and read as one channel — its first
/// width × height values, whatever the cache interleaves (after a real
/// transform to GRAY, alpha is gone from it but meta channels are not);
/// otherwise alpha is turned off and every remaining channel kept. Quanta
/// are scaled to 0..1.
fn cache_to_map(mut cache: PixelCache) -> Map {
    let resolution = Resolution { x: cache.width, y: cache.height };
    let area = cache.width * cache.height;
    let mut map = if cache.channels.len() < 3 {
        // A real transform to GRAY leaves the grey channel alone in what is
        // read; otherwise the cache is read as it was, extra channels
        // interleaved.
        if cache.colorspace == CacheColorspace::LinearGray {
            cache = linear_to_gray(&cache);
            cache = without_alpha(cache);
        }
        let mut values = cache.data;
        values.resize(area.max(values.len()), 0.0);
        values.truncate(area);
        Map::from_data(resolution, values)
    } else {
        cache = without_alpha(cache);
        Map::from_interleaved(resolution, &cache.data, cache.channels.len())
    };
    map.apply(|quantum| quantum * QUANTUM_SCALE);
    map
}

/// A square map from raw samples (the side is the square root
/// of the count) or, for colour, from interleaved RGB.
fn raw_map(values: Vec<f32>, is_rgb: bool) -> Map {
    if is_rgb {
        let side = (values.len() as f64 / 3.0).sqrt() as usize;
        let resolution = Resolution::square(side);
        let mut padded = values;
        padded.resize(side * side * 3, 0.0);
        return Map::from_interleaved(resolution, &padded, 3);
    }
    let side = (values.len() as f32 as f64).sqrt() as usize;
    let mut padded = values;
    padded.resize(side * side, 0.0);
    Map::from_data(Resolution::square(side), padded)
}

/// `MagickImage.ChannelCount`: how many channels ImageMagick reads the
/// image with (an alpha or a palette's index channel counts too), or
/// nothing when it cannot be read.
pub fn image_channel_count(path: &Path) -> Option<usize> {
    let cache = read_pixel_cache(path).ok()?;
    Some(cache.channels.len())
}

/// A map from a file — `.r32` floats, `.raw` 16-bit
/// unsigned samples, anything else an image.
pub fn load_map_file(path: &Path, is_rgb: bool, enforce_linear_gamma: bool) -> Result<Map, String> {
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
    if extension == "r32" {
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        let values = bytes.as_chunks::<4>().0.iter().map(|quad| f32::from_le_bytes([quad[0], quad[1], quad[2], quad[3]])).collect();
        return Ok(raw_map(values, is_rgb));
    }
    if extension == "raw" {
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        let values = bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes([pair[0], pair[1]]) as f32 / 65535.0).collect();
        return Ok(raw_map(values, is_rgb));
    }

    let mut cache = read_pixel_cache(path).map_err(|error| error.to_string())?;
    if enforce_linear_gamma {
        let gamma = cache.gamma;
        if gamma < 0.5 {
            gamma_correct(&mut cache, 2.2);
        } else if gamma > 2.0 {
            gamma_correct(&mut cache, 0.454545454545455);
        }
        if cache.colorspace != CacheColorspace::LinearGray {
            cache = to_linear_gray(&cache);
        }
    }
    let map = cache_to_map(cache);
    if !is_rgb && map.channel_count() > 1 {
        let first = map.channel(0).to_vec();
        return Ok(Map::from_data(map.resolution(), first));
    }
    if is_rgb && map.channel_count() == 1 {
        let gray = map.channel(0).to_vec();
        return Ok(Map::from_channels(map.resolution(), vec![gray.clone(), gray.clone(), gray]));
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Loads every image named in `$NERTHUS_MAP_FILES` (a list split by
    /// newlines) with each combination of IsRGB and EnforceLinearGamma and
    /// writes the map as the oracle's `load-map` does, into
    /// `<file>.<rgb><linear>.ours/` (`map.raw`, or `map.cN.raw` per
    /// channel, and `map.json`), for comparing with the oracle's maps.
    #[test]
    #[ignore]
    fn dump_loaded_maps() {
        let list = std::env::var("NERTHUS_MAP_FILES").expect("set NERTHUS_MAP_FILES");
        for file in list.lines().filter(|line| !line.trim().is_empty()) {
            let path = Path::new(file.trim());
            for (is_rgb, linear) in [(false, false), (true, false), (false, true), (true, true)] {
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                let folder = path.with_file_name(format!("{name}.{}{}.ours", u8::from(is_rgb), u8::from(linear)));
                std::fs::create_dir_all(&folder).unwrap();
                let report = match load_map_file(path, is_rgb, linear) {
                    Ok(map) => {
                        let channels = map.channel_count();
                        for channel in 0..channels {
                            let file_name = if channels == 1 { "map.raw".to_string() } else { format!("map.c{channel}.raw") };
                            let bytes: Vec<u8> = map.channel(channel).iter().flat_map(|value| value.to_le_bytes()).collect();
                            std::fs::write(folder.join(file_name), bytes).unwrap();
                        }
                        format!("{{\"width\":{},\"height\":{},\"channels\":{channels}}}", map.width(), map.height())
                    }
                    Err(error) => format!("{{\"error\":{:?}}}", error),
                };
                std::fs::write(folder.join("map.json"), report).unwrap();
            }
        }
    }
}
