//! ImageMagick 7.1.2 operations on images that hold Q16 HDRI quanta (floats
//! in 0..65535), the way the nodes feed Magick.NET. Working on the quanta
//! themselves keeps every intermediate value the one IM computes; the rest
//! of this crate stores normalised values for the older ports.

use std::collections::HashMap;

use rayon::prelude::*;

use crate::image::Image;

/// `QuantumRange`.
pub const QUANTUM_RANGE: f32 = 65535.0;
/// `QuantumScale`.
const QUANTUM_SCALE: f64 = 1.0 / 65535.0;
/// `MaxMap` (Q16).
const MAX_MAP: usize = 65535;
/// `MagickEpsilon`.
const MAGICK_EPSILON: f64 = 1.0e-12;

/// `MagickSafeReciprocal`.
fn safe_reciprocal(value: f64) -> f64 {
    if value > -MAGICK_EPSILON && value < MAGICK_EPSILON {
        return 1.0 / MAGICK_EPSILON;
    }
    1.0 / value
}

/// `pow` as a library call at run time.
fn pow(base: f64, exponent: f64) -> f64 {
    std::hint::black_box(base).powf(exponent)
}

/// IM's `gamma_pow`: negative values pass through.
fn gamma_pow(value: f64, gamma: f64) -> f64 {
    if value < 0.0 {
        return value;
    }
    pow(value, gamma)
}

/// `ScaleQuantumToMap` (Q16 HDRI): the 16-bit map index of a quantum,
/// rounded in single precision.
pub fn quantum_to_map(quantum: f32) -> usize {
    if quantum >= MAX_MAP as f32 {
        return MAX_MAP;
    }
    if quantum.is_nan() || quantum <= 0.0 {
        return 0;
    }
    (quantum + 0.5) as usize
}

/// `ScaleMapToQuantum` (Q16 HDRI).
fn map_to_quantum(value: f64) -> f32 {
    if value <= 0.0 {
        return 0.0;
    }
    if value >= MAX_MAP as f64 {
        return QUANTUM_RANGE;
    }
    value as f32
}

/// `ClampPixel`.
fn clamp_pixel(value: f32) -> f32 {
    if value < 0.0 {
        return 0.0;
    }
    if value >= QUANTUM_RANGE {
        return QUANTUM_RANGE;
    }
    value
}

/// The channels IM updates by default: every channel but alpha (the last
/// of a two- or four-channel image).
fn colour_channels(image: &Image) -> usize {
    match image.channels {
        2 | 4 => image.channels - 1,
        channels => channels,
    }
}

/// `GammaImage`: every colour channel through IM's 16-bit gamma map.
pub fn gamma(image: &mut Image, gamma: f64) {
    if gamma == 1.0 {
        return;
    }
    let mut map = vec![0.0f32; MAX_MAP + 1];
    if gamma != 0.0 {
        let exponent = safe_reciprocal(gamma);
        for (index, entry) in map.iter_mut().enumerate() {
            let fraction = index as f64 / MAX_MAP as f64;
            *entry = map_to_quantum(MAX_MAP as f64 * pow(fraction, exponent));
        }
    }
    let channels = image.channels;
    let updated = colour_channels(image);
    image.data.par_chunks_mut(channels).for_each(|pixel| {
        for value in &mut pixel[..updated] {
            *value = map[quantum_to_map(*value)];
        }
    });
}

/// `LevelImage` on one channel (a single-channel mask), with the
/// `ClampImage` it ends with.
fn level_channel(image: &mut Image, channel: usize, black: f64, white: f64, gamma: f64) {
    let scale = safe_reciprocal(white - black);
    let exponent = safe_reciprocal(gamma);
    let channels = image.channels;
    image.data.par_chunks_mut(channels).for_each(|pixel| {
        let leveled = QUANTUM_RANGE as f64 * gamma_pow(scale * (pixel[channel] as f64 - black), exponent);
        pixel[channel] = clamp_pixel(leveled as f32);
    });
}

/// `GetImageMean` of one channel. IM sums in `long double`, which is a
/// plain double in an MSVC build of Magick.Native.
fn channel_mean(image: &Image, channel: usize) -> f64 {
    let mut sum = 0.0f64;
    let mut area = 0.0f64;
    for pixel in image.data.chunks(image.channels) {
        area += 1.0;
        sum += pixel[channel] as f64;
    }
    if area > 0.0 {
        return sum / area;
    }
    0.0
}

/// `AutoGammaImage` with the Composite channel mask Magick.NET passes:
/// each colour channel gets the gamma that moves its own mean to 0.5.
pub fn auto_gamma(image: &mut Image) {
    let log_mean = 0.5f64.ln();
    for channel in 0..colour_channels(image) {
        let mean = channel_mean(image, channel);
        let gamma = (mean * QUANTUM_SCALE).ln() / log_mean;
        level_channel(image, channel, 0.0, QUANTUM_RANGE as f64, gamma);
    }
}

/// `LiquidRescaleImage` (Magick.NET `LiquidRescale(width, height)`).
pub fn liquid_rescale(image: &Image, width: usize, height: usize) -> Image {
    crate::liquid_rescale::carve(image, width, height, 1.0)
}

/// `MaxColormapSize`: the most objects `ConnectedComponentsImage` labels.
const MAX_OBJECTS: usize = 65536;

/// `ConnectedComponentsImage` failed: more than [`MAX_OBJECTS`] objects
/// (IM's "too many objects").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooManyObjects;

/// One labelled object: its bounding box (inclusive), area and colour.
#[derive(Clone)]
struct ComponentObject {
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
    area: f64,
    color: [f64; 3],
}

/// IM 7.1.2's `IsFuzzyEquivalencePixelInfo` for opaque pixels and a fuzz
/// of zero: the squared red, green and blue differences, summed one by one,
/// must stay within `MagickEpsilon`.
fn same_color(a: [f64; 3], b: [f64; 3]) -> bool {
    let threshold = MAGICK_EPSILON * (1.0 + MAGICK_EPSILON);
    let mut distance = 0.0f64;
    for channel in 0..3 {
        let difference = a[channel] - b[channel];
        distance += difference * difference;
        if distance > threshold {
            return false;
        }
    }
    true
}

/// The red, green and blue quanta IM reads for a pixel (a grey pixel reads
/// its grey three times).
fn pixel_rgb(image: &Image, index: usize) -> [f64; 3] {
    let channels = image.channels;
    let pixel = &image.data[index * channels..index * channels + channels];
    if channels < 3 {
        let gray = pixel[0] as f64;
        return [gray, gray, gray];
    }
    [pixel[0] as f64, pixel[1] as f64, pixel[2] as f64]
}

/// Labels the objects of equal colour, joined along `directions`, in
/// reading order (IM's union with its partial path compression), and
/// gathers each object's bounding box, area and mean colour.
fn label_components(image: &Image, directions: &[(isize, isize)]) -> Result<(Vec<usize>, Vec<ComponentObject>), TooManyObjects> {
    let width = image.width;
    let height = image.height;
    let size = width * height;
    let mut equivalences: Vec<usize> = (0..size).collect();
    for &(dy, dx) in directions {
        for y in 0..height {
            for x in 0..width {
                let neighbour_x = x as isize + dx;
                let neighbour_y = y as isize + dy;
                let outside_x = neighbour_x < 0 || neighbour_x >= width as isize;
                let outside_y = neighbour_y < 0 || neighbour_y >= height as isize;
                if outside_x || outside_y {
                    continue;
                }
                let offset = y * width + x;
                let neighbour = neighbour_y as usize * width + neighbour_x as usize;
                if !same_color(pixel_rgb(image, offset), pixel_rgb(image, neighbour)) {
                    continue;
                }
                let mut own_root = offset;
                while equivalences[own_root] != own_root {
                    own_root = equivalences[own_root];
                }
                let mut neighbour_root = neighbour;
                while equivalences[neighbour_root] != neighbour_root {
                    neighbour_root = equivalences[neighbour_root];
                }
                let root = if own_root < neighbour_root {
                    equivalences[neighbour_root] = own_root;
                    own_root
                } else {
                    equivalences[own_root] = neighbour_root;
                    neighbour_root
                };
                equivalences[offset] = root;
                equivalences[neighbour] = root;
            }
        }
    }

    let empty = ComponentObject {
        left: width,
        top: height,
        right: 0,
        bottom: 0,
        area: 0.0,
        color: [0.0; 3],
    };
    let mut objects: Vec<ComponentObject> = Vec::new();
    let mut labels = vec![0usize; size];
    for y in 0..height {
        for x in 0..width {
            let offset = y * width + x;
            let mut id = equivalences[offset];
            if id != offset {
                id = equivalences[id];
            } else {
                id = objects.len();
                if id >= MAX_OBJECTS {
                    return Err(TooManyObjects);
                }
                objects.push(empty.clone());
            }
            equivalences[offset] = id;
            labels[offset] = id;
            let rgb = pixel_rgb(image, offset);
            let object = &mut objects[id];
            object.left = object.left.min(x);
            object.right = object.right.max(x);
            object.top = object.top.min(y);
            object.bottom = object.bottom.max(y);
            for (sum, value) in object.color.iter_mut().zip(rgb) {
                *sum += QUANTUM_SCALE * value;
            }
            object.area += 1.0;
        }
    }
    for object in &mut objects {
        let divisor = QUANTUM_SCALE * object.area;
        for channel in 0..3 {
            object.color[channel] /= divisor;
        }
    }
    Ok((labels, objects))
}

/// Merges object `merged` into the neighbour it touches most, counting
/// contacts along `directions` from inside its bounding box with tiled
/// edges (ties go to the lowest id, and no contact at all to object 0).
fn merge_object(labels: &mut [usize], width: usize, height: usize, object: &ComponentObject, merged: usize, directions: &[(isize, isize)]) {
    let mut census: HashMap<usize, u64> = HashMap::new();
    for y in object.top..=object.bottom {
        for x in object.left..=object.right {
            if labels[y * width + x] != merged {
                continue;
            }
            for &(dy, dx) in directions {
                let neighbour_x = (x as isize + dx).rem_euclid(width as isize) as usize;
                let neighbour_y = (y as isize + dy).rem_euclid(height as isize) as usize;
                let neighbour = labels[neighbour_y * width + neighbour_x];
                if neighbour != merged {
                    *census.entry(neighbour).or_insert(0) += 1;
                }
            }
        }
    }
    let mut target = 0usize;
    let mut most = 0u64;
    for (&id, &count) in &census {
        if count > most || (count == most && id < target) {
            target = id;
            most = count;
        }
    }
    for y in object.top..=object.bottom {
        for x in object.left..=object.right {
            let label = &mut labels[y * width + x];
            if *label == merged {
                *label = target;
            }
        }
    }
}

/// `ConnectedComponentsImage` as Magick.NET's `ConnectedComponents` runs
/// it: pixels of equal colour joined 4- or 8-wise into objects;
/// objects smaller than `area_threshold` (but not the largest) merged into
/// the neighbour they touch most; then every pixel painted its object's
/// mean colour, or its object id without `mean_color`.
pub fn connected_components(image: &Image, connectivity: usize, area_threshold: Option<f64>, mean_color: bool) -> Result<Image, TooManyObjects> {
    const CONNECT4: [(isize, isize); 2] = [(-1, 0), (0, -1)];
    const CONNECT8: [(isize, isize); 4] = [(-1, -1), (-1, 0), (-1, 1), (0, -1)];
    let directions: &[(isize, isize)] = if connectivity > 4 { &CONNECT8 } else { &CONNECT4 };
    let width = image.width;
    let height = image.height;
    let (mut labels, objects) = label_components(image, directions)?;

    let mut background = 0usize;
    for (id, object) in objects.iter().enumerate() {
        if object.area > objects[background].area {
            background = id;
        }
    }
    if let Some(minimum) = area_threshold {
        for (id, object) in objects.iter().enumerate() {
            let merge = object.area < minimum && id != background;
            if merge {
                merge_object(&mut labels, width, height, object, id, directions);
            }
        }
    }

    let channels = image.channels;
    let mut result = image.clone();
    for (index, &label) in labels.iter().enumerate() {
        let pixel = &mut result.data[index * channels..index * channels + channels];
        if !mean_color {
            pixel.iter_mut().take(3).for_each(|value| *value = label as f32);
            continue;
        }
        let color = objects[label].color;
        if channels < 3 {
            pixel[0] = color[2] as f32;
        } else {
            for channel in 0..3 {
                pixel[channel] = color[channel] as f32;
            }
        }
    }
    Ok(result)
}

/// `GrayscaleImage` with Magick.NET's default (Rec. 709 luma) for an sRGB
/// or GRAY image: one grey channel.
pub fn grayscale(image: &Image) -> Image {
    if image.channels < 3 {
        let gray: Vec<f32> = image.data.iter().step_by(image.channels).copied().collect();
        let mut result = Image::from_data(image.width, image.height, 1, gray);
        result.colorspace = crate::Colorspace::Gray;
        return result;
    }
    let channels = image.channels;
    let gray: Vec<f32> = image
        .data
        .par_chunks(channels)
        .map(|pixel| {
            let red = pixel[0] as f64;
            let green = pixel[1] as f64;
            let blue = pixel[2] as f64;
            (0.212656 * red + 0.715158 * green + 0.072186 * blue) as f32
        })
        .collect();
    let mut result = Image::from_data(image.width, image.height, 1, gray);
    result.colorspace = crate::Colorspace::Gray;
    result
}

/// The alpha channel left by `Alpha(Set)` then `Transparent(target)` with
/// a colour fuzz of `fuzz` quanta: transparent (0) where a pixel matches
/// the opaque `target` colour within the fuzz, opaque (`QuantumRange`)
/// elsewhere. Follows IM 7.1.2's `IsFuzzyEquivalencePixelInfo` for an
/// opaque pixel against an opaque colour without fuzz of its own.
pub fn transparent_alpha(image: &Image, target: [f32; 3], fuzz: f64) -> Image {
    let squared_fuzz = 3.0 * (fuzz * fuzz + 0.0 * 0.0);
    let base = if squared_fuzz > 0.0 { squared_fuzz } else { MAGICK_EPSILON };
    let threshold = base * (1.0 + MAGICK_EPSILON);
    let scale = QUANTUM_SCALE * QUANTUM_RANGE as f64;
    let target = [target[0] as f64, target[1] as f64, target[2] as f64];
    let alpha: Vec<f32> = (0..image.width * image.height)
        .into_par_iter()
        .map(|index| {
            let rgb = pixel_rgb(image, index);
            let mut distance = 0.0f64;
            for channel in 0..3 {
                let difference = rgb[channel] - target[channel];
                distance += difference * difference * scale;
                if distance > threshold {
                    return QUANTUM_RANGE;
                }
            }
            0.0
        })
        .collect();
    let mut result = Image::from_data(image.width, image.height, 1, alpha);
    result.colorspace = crate::Colorspace::Gray;
    result
}

/// `NegateImage` on every colour channel.
pub fn negate(image: &mut Image) {
    let channels = image.channels;
    let updated = colour_channels(image);
    image.data.par_chunks_mut(channels).for_each(|pixel| {
        for value in &mut pixel[..updated] {
            *value = QUANTUM_RANGE - *value;
        }
    });
}

/// `0.212656·red + 0.715158·green + 0.072186·blue`, IM's Rec. 709 luma.
fn luma(red: f64, green: f64, blue: f64) -> f64 {
    0.212656 * red + 0.715158 * green + 0.072186 * blue
}

/// IM 7.1.2's `DecodePixelGamma` on a quantum (the sRGB curve to linear).
fn decode_pixel_gamma(pixel: f64) -> f64 {
    if pixel <= 0.0404482362771076 * QUANTUM_RANGE as f64 {
        return pixel / 12.92;
    }
    QUANTUM_RANGE as f64 * crate::colorspace::decode_gamma((QUANTUM_SCALE * pixel + 0.055) / 1.055)
}

/// IM 7.1.2's `EncodePixelGamma` on a quantum (linear to the sRGB curve).
pub fn encode_pixel_gamma(pixel: f64) -> f64 {
    if pixel <= 0.0031306684425005883 * QUANTUM_RANGE as f64 {
        return 12.92 * pixel;
    }
    QUANTUM_RANGE as f64 * (1.055 * crate::colorspace::encode_gamma(QUANTUM_SCALE * pixel) - 0.055)
}

/// `TransformImageColorspace` to LinearGRAY, through sRGB: grey images are
/// spread to red, green and blue and taken back by luma first; every
/// channel is then decoded and the luma of the decoded values kept.
pub fn to_linear_gray(image: &Image) -> Image {
    let rgb: Vec<[f64; 3]> = (0..image.width * image.height)
        .map(|index| {
            let pixel = pixel_rgb(image, index);
            if image.channels < 3 {
                let gray = luma(pixel[0], pixel[1], pixel[2]) as f32 as f64;
                return [gray, gray, gray];
            }
            pixel
        })
        .collect();
    let linear: Vec<f32> = rgb
        .par_iter()
        .map(|pixel| {
            let red = decode_pixel_gamma(pixel[0]);
            let green = decode_pixel_gamma(pixel[1]);
            let blue = decode_pixel_gamma(pixel[2]);
            luma(red, green, blue) as f32
        })
        .collect();
    let mut result = Image::from_data(image.width, image.height, 1, linear);
    result.colorspace = crate::Colorspace::LinearGray;
    result
}

/// `TransformImageColorspace` from LinearGRAY to GRAY, through sRGB: the
/// grey is encoded on red, green and blue, then taken back by luma.
pub fn linear_to_gray(image: &Image) -> Image {
    let gray: Vec<f32> = image
        .data
        .par_iter()
        .map(|&quantum| {
            let encoded = encode_pixel_gamma(quantum as f64);
            let srgb = luma(encoded, encoded, encoded) as f32 as f64;
            luma(srgb, srgb, srgb) as f32
        })
        .collect();
    let mut result = Image::from_data(image.width, image.height, 1, gray);
    result.colorspace = crate::Colorspace::Gray;
    result
}

/// `AutoLevelImage` with the default (all channels) mask: the colour
/// channels stretched together from the image's range to the quantum
/// range, then clamped.
pub fn auto_level(image: &mut Image) {
    let channels = image.channels;
    let updated = colour_channels(image);
    let mut minimum = f64::MAX;
    let mut maximum = -f64::MAX;
    for pixel in image.data.chunks(channels) {
        for &value in &pixel[..updated] {
            let value = value as f64;
            if value < minimum {
                minimum = value;
            }
            if value > maximum {
                maximum = value;
            }
        }
    }
    if (minimum - maximum).abs() < MAGICK_EPSILON {
        return;
    }
    let scale = safe_reciprocal(maximum - minimum);
    let exponent = safe_reciprocal(1.0);
    image.data.par_chunks_mut(channels).for_each(|pixel| {
        for value in &mut pixel[..updated] {
            let leveled = QUANTUM_RANGE as f64 * gamma_pow(scale * (*value as f64 - minimum), exponent);
            *value = clamp_pixel(leveled as f32);
        }
    });
}

/// IM's `MagickRound`: halves round up.
fn magick_round(value: f64) -> f64 {
    if (value - value.floor()) < (value.ceil() - value) {
        return value.floor();
    }
    value.ceil()
}

/// `PosterizeImage` without dithering: each colour channel snapped to
/// `levels` evenly spaced values.
pub fn posterize(image: &mut Image, levels: usize) {
    let channels = image.channels;
    let updated = colour_channels(image);
    let steps = levels as f64 - 1.0;
    let divisor = if steps > 1.0 { steps } else { 1.0 };
    image.data.par_chunks_mut(channels).for_each(|pixel| {
        for value in &mut pixel[..updated] {
            let rounded = magick_round(QUANTUM_SCALE * *value as f64 * steps);
            *value = (QUANTUM_RANGE as f64 * rounded / divisor) as f32;
        }
    });
}

/// `OrderedDitherImage(image, map)` with IM's threshold maps and the extra
/// ones of `threshold_maps` (by name or alias). Unknown maps are an error,
/// as in IM.
pub fn ordered_dither(image: &mut Image, threshold_map: &str) -> Result<(), String> {
    let builtin = crate::threshold_maps::THRESHOLD_MAPS.iter();
    let maps = builtin.chain(crate::threshold_maps::EXTRA_THRESHOLD_MAPS.iter());
    let map = maps.into_iter().find(|map| map.name == threshold_map || map.alias == Some(threshold_map));
    let Some(map) = map else {
        return Err(format!("ordered-dither : '{threshold_map}'"));
    };
    let levels = 2.0f64 - 1.0;
    let divisor = map.divisor;
    let width = image.width;
    let channels = image.channels;
    let updated = colour_channels(image);
    image.data.par_chunks_mut(width * channels).enumerate().for_each(|(y, row)| {
        for x in 0..width {
            let cell = map.levels[(x % map.width) + map.width * (y % map.height)];
            for value in &mut row[x * channels..x * channels + updated] {
                let mut threshold = (QUANTUM_SCALE * *value as f64 * (levels * (divisor - 1) as f64 + 1.0)) as i64;
                let level = threshold / (divisor - 1);
                threshold -= level * (divisor - 1);
                let bumped = level + i64::from(threshold >= cell);
                *value = (bumped as f64 * QUANTUM_RANGE as f64 / levels) as f32;
            }
        }
    });
    Ok(())
}

/// The values a grey image keeps through `QuantizeImage`'s
/// `SetGrayscaleImage` once its colormap is applied: pixels whose quanta
/// round to the same 16-bit level all take the value of the first such
/// pixel in reading order.
pub fn grayscale_colormap_values(image: &mut Image) {
    let mut first_seen: Vec<Option<f32>> = vec![None; MAX_MAP + 1];
    for value in image.data.iter_mut().step_by(image.channels) {
        let level = quantum_to_map(*value);
        *value = *first_seen[level].get_or_insert(*value);
    }
}
