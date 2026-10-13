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

/// The colour (and alpha) channels of the cache as an `Image` for IM's
/// colour operations, the index channel left out.
fn colour_image(cache: &PixelCache) -> Image {
    let kept: Vec<usize> = (0..cache.channels.len()).filter(|&index| cache.channels[index] != PixelChannel::Index).collect();
    let stride = cache.channels.len();
    let mut data = Vec::with_capacity(cache.width * cache.height * kept.len());
    for pixel in cache.data.chunks_exact(stride) {
        for &index in &kept {
            data.push(pixel[index]);
        }
    }
    let mut image = Image::from_data(cache.width, cache.height, kept.len(), data);
    image.colorspace = match cache.colorspace {
        CacheColorspace::SRgb => Colorspace::SRgb,
        CacheColorspace::Rgb => Colorspace::LinearRgb,
        CacheColorspace::Gray => Colorspace::Gray,
        CacheColorspace::LinearGray => Colorspace::LinearGray,
    };
    image
}

/// The cache after a colorspace transform: `DirectClass` (no index), grey
/// plus any alpha.
fn from_gray_image(image: Image, alpha: Option<Vec<f32>>, colorspace: CacheColorspace, gamma: f64) -> PixelCache {
    let mut channels = vec![PixelChannel::Gray];
    let data = match alpha {
        Some(alpha) => {
            channels.push(PixelChannel::Alpha);
            image.data.iter().zip(alpha.iter()).flat_map(|(&gray, &opacity)| [gray, opacity]).collect()
        }
        None => image.data,
    };
    PixelCache { width: image.width, height: image.height, channels, data, colorspace, gamma }
}

/// The cache's alpha samples, if it has alpha.
fn alpha_of(cache: &PixelCache) -> Option<Vec<f32>> {
    let position = cache.channels.iter().position(|channel| *channel == PixelChannel::Alpha)?;
    let stride = cache.channels.len();
    Some(cache.data.chunks_exact(stride).map(|pixel| pixel[position]).collect())
}

/// `TransformImageColorspace` to LinearGRAY (through sRGB, IM 7.1.2).
fn to_linear_gray(cache: &PixelCache) -> PixelCache {
    let alpha = alpha_of(cache);
    let mut image = colour_image(cache);
    if alpha.is_some() {
        image = without_alpha(&image);
    }
    let linear = quanta::to_linear_gray(&image);
    from_gray_image(linear, alpha, CacheColorspace::LinearGray, 1.0)
}

/// `TransformImageColorspace` from LinearGRAY to GRAY.
fn linear_to_gray(cache: &PixelCache) -> PixelCache {
    let alpha = alpha_of(cache);
    let image = without_alpha(&colour_image(cache));
    let gray = quanta::linear_to_gray(&image);
    from_gray_image(gray, alpha, CacheColorspace::Gray, cache.gamma)
}

/// The colour channels alone.
fn without_alpha(image: &Image) -> Image {
    let colours = match image.channels {
        2 | 4 => image.channels - 1,
        channels => channels,
    };
    if colours == image.channels {
        return image.clone();
    }
    let mut data = Vec::with_capacity(image.width * image.height * colours);
    for pixel in image.data.chunks_exact(image.channels) {
        data.extend_from_slice(&pixel[..colours]);
    }
    let mut result = Image::from_data(image.width, image.height, colours, data);
    result.colorspace = image.colorspace;
    result
}

/// `GammaImage` (Magick.NET `GammaCorrect`) on the colour channels; the
/// index and alpha channels keep their values.
fn gamma_correct(cache: &mut PixelCache, gamma: f64) {
    let colour_positions: Vec<usize> = (0..cache.channels.len())
        .filter(|&index| !matches!(cache.channels[index], PixelChannel::Index | PixelChannel::Alpha))
        .collect();
    let stride = cache.channels.len();
    let mut colours = Vec::with_capacity(cache.width * cache.height * colour_positions.len());
    for pixel in cache.data.chunks_exact(stride) {
        for &position in &colour_positions {
            colours.push(pixel[position]);
        }
    }
    let mut image = Image::from_data(cache.width, cache.height, colour_positions.len(), colours);
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

/// The cache as a map. A cache of fewer than three
/// channels is made GRAY and read as one channel — its first
/// width × height values, whatever the cache interleaves; otherwise alpha
/// is turned off and every remaining channel kept. Quanta are scaled to
/// 0..1.
fn cache_to_map(mut cache: PixelCache) -> Map {
    let resolution = Resolution { x: cache.width, y: cache.height };
    let area = cache.width * cache.height;
    let mut map = if cache.channels.len() < 3 {
        if cache.colorspace == CacheColorspace::LinearGray {
            cache = linear_to_gray(&cache);
        }
        let mut values = cache.data;
        values.resize(area.max(values.len()), 0.0);
        values.truncate(area);
        Map::from_data(resolution, values)
    } else {
        if let Some(position) = cache.channels.iter().position(|channel| *channel == PixelChannel::Alpha) {
            let stride = cache.channels.len();
            let mut kept = Vec::with_capacity(area * (stride - 1));
            for pixel in cache.data.chunks_exact(stride) {
                for (index, &value) in pixel.iter().enumerate() {
                    if index != position {
                        kept.push(value);
                    }
                }
            }
            cache.channels.remove(position);
            cache.data = kept;
        }
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
