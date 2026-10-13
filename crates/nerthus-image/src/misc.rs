//! Remaining ImageMagick operations used by the nodes: random noise,
//! spread, sparse colour, colour lookup tables, dithering, trimming and
//! simple drawing.

use rayon::prelude::*;

use crate::colorspace::Colorspace;
use crate::image::{perceptible_reciprocal, pixel_intensity, Image, Interpolate, QUANTUM_RANGE, QUANTUM_SCALE};
use crate::levels::{map_index, MAX_MAP};
use crate::threshold_maps::THRESHOLD_MAPS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseType {
    Uniform,
    Gaussian,
    Multiplicative,
    Impulse,
    Laplacian,
    Poisson,
    Random,
}

/// IM7 `GenerateDifferentialNoise` (gem.c), evaluated in quantum units.
fn differential_noise(random: &mut MagickRandom, pixel: f64, noise: NoiseType, attenuate: f64) -> f64 {
    const EPSILON: f64 = 1.0e-12;
    const PI: f64 = std::f64::consts::PI;
    let quantum = QUANTUM_RANGE;
    let value = pixel * quantum;
    let sigma_uniform = attenuate * 0.015625;
    let sigma_gaussian = attenuate * 0.015625;
    let sigma_impulse = attenuate * 0.1;
    let sigma_laplacian = attenuate * 0.0390625;
    let sigma_multiplicative = attenuate * 0.5;
    let sigma_poisson = attenuate * 12.5;
    let sigma_random = attenuate;
    let tau_gaussian = attenuate * 0.078125;
    let mut alpha = random.next();
    let result = match noise {
        NoiseType::Uniform => value + quantum * sigma_uniform * (alpha - 0.5),
        NoiseType::Gaussian => {
            if alpha.abs() < EPSILON {
                alpha = 1.0;
            }
            let beta = random.next();
            let gamma = (-2.0 * alpha.ln()).sqrt();
            let sigma = gamma * (2.0 * PI * beta).cos();
            let tau = gamma * (2.0 * PI * beta).sin();
            value + value.sqrt() * sigma_gaussian * sigma + quantum * tau_gaussian * tau
        }
        NoiseType::Impulse => {
            if alpha < sigma_impulse / 2.0 {
                0.0
            } else if alpha >= 1.0 - sigma_impulse / 2.0 {
                quantum
            } else {
                value
            }
        }
        NoiseType::Laplacian => {
            if alpha <= 0.5 {
                if alpha <= EPSILON {
                    value - quantum
                } else {
                    value + quantum * sigma_laplacian * (2.0 * alpha).ln() + 0.5
                }
            } else {
                let beta = 1.0 - alpha;
                if beta <= 0.5 * EPSILON {
                    value + quantum
                } else {
                    value - quantum * sigma_laplacian * (2.0 * beta).ln() + 0.5
                }
            }
        }
        NoiseType::Multiplicative => {
            let mut sigma = 1.0;
            if alpha > EPSILON {
                sigma = (-2.0 * alpha.ln()).sqrt();
            }
            let beta = random.next();
            value + value * sigma_multiplicative * sigma * (2.0 * PI * beta).cos() / 2.0
        }
        NoiseType::Poisson => {
            let poisson = (-sigma_poisson * QUANTUM_SCALE * value).exp();
            let mut count = 0.0;
            while alpha > poisson {
                alpha *= random.next();
                count += 1.0;
            }
            quantum * count * perceptible_reciprocal(sigma_poisson)
        }
        NoiseType::Random => quantum * sigma_random * alpha,
    };
    result / quantum
}

/// IM `AddNoiseImage(noise, attenuate)` with a seeded generator. A seeded IM
/// runs it on a single thread, so one random sequence walks the pixels row by
/// row and, within a pixel, channel by channel.
pub fn add_noise(image: &Image, noise: NoiseType, attenuate: f64, random: &mut MagickRandom) -> Image {
    let mut result = image.clone();
    for value in result.data.iter_mut() {
        *value = differential_noise(random, *value as f64, noise, attenuate) as f32;
    }
    result
}

/// ImageMagick's pseudo random generator (`random.c`, xoshiro256+) seeded
/// the way `MagickNET.SetRandomSeed(key)` does: SHA-256 of the 32-bit key.
#[derive(Clone, Debug)]
pub struct MagickRandom {
    seed: [u64; 4],
}

impl MagickRandom {
    /// Seeded like Windows builds of IM, where the `unsigned long` secret key
    /// hashed by `AcquireRandomInfo` is four bytes.
    pub fn with_secret_key(key: u32) -> Self {
        Self::from_key_bytes(&key.to_le_bytes())
    }

    /// Seeded like IM on LP64 platforms (eight-byte `unsigned long`), which is
    /// how the `magick` tool on Linux and macOS behaves.
    pub fn with_secret_key_lp64(key: u64) -> Self {
        Self::from_key_bytes(&key.to_le_bytes())
    }

    fn from_key_bytes(key: &[u8]) -> Self {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(key);
        let mut seed = [0u64; 4];
        for (index, chunk) in digest.as_chunks::<8>().0.iter().enumerate() {
            seed[index] = u64::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7]]);
        }
        Self { seed }
    }

    /// `GetPseudoRandomValue`: uniform in [0, 1].
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f64 {
        let normalize = 1.0 / (u64::MAX >> 11) as f64;
        let alpha = self.seed[1] << 17;
        let value = self.seed[0].wrapping_add(self.seed[3]);
        self.seed[2] ^= self.seed[0];
        self.seed[3] ^= self.seed[1];
        self.seed[1] ^= self.seed[2];
        self.seed[0] ^= self.seed[3];
        self.seed[2] ^= alpha;
        self.seed[3] = self.seed[3].rotate_left(45);
        (value >> 11) as f64 * normalize
    }
}

/// IM `SpreadImage(method, radius)` with a seeded generator, which IM runs on
/// a single thread (one random sequence, row by row).
pub fn spread(image: &Image, method: Interpolate, radius: f64, random: &mut MagickRandom) -> Image {
    let channels = image.channels;
    let width = image.width;
    let spread_width = crate::convolve::optimal_kernel_width_1d(radius, 0.5) as f64;
    let mut result = image.clone();
    for y in 0..image.height {
        for x in 0..width {
            let point_x = random.next();
            let point_y = random.next();
            let sample_x = x as f64 + spread_width * (point_x - 0.5);
            let sample_y = y as f64 + spread_width * (point_y - 0.5);
            for channel in 0..channels {
                let value = image.interpolate(sample_x, sample_y, channel, method);
                result.data[(y * width + x) * channels + channel] = value;
            }
        }
    }
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SparseColorMethod {
    Voronoi,
    Manhattan,
    Shepards,
    Inverse,
}

/// A control point: position plus colour (normalised channels).
#[derive(Clone, Debug)]
pub struct SparsePoint {
    pub x: f64,
    pub y: f64,
    pub color: Vec<f32>,
}

/// IM `SparseColorImage` (7.1.0). Pixels are sampled at integer coordinates,
/// colours are normalised and the result is clamped like IM's `ClampPixel`.
pub fn sparse_color(width: usize, height: usize, channels: usize, method: SparseColorMethod, points: &[SparsePoint]) -> Image {
    let mut result = Image::new(width, height, channels);
    if points.is_empty() {
        return result;
    }
    // Shepards weights by squared distance, Inverse by plain distance.
    let power = if method == SparseColorMethod::Inverse { 0.5 } else { 1.0 };
    result.data.par_chunks_mut(width * channels).enumerate().for_each(|(row, pixels)| {
        let j = row as f64;
        let mut sums = vec![0.0f64; channels];
        for column in 0..width {
            let i = column as f64;
            let pixel = &mut pixels[column * channels..(column + 1) * channels];
            match method {
                SparseColorMethod::Voronoi | SparseColorMethod::Manhattan => {
                    let mut minimum = f64::MAX;
                    let mut nearest = 0;
                    for (index, point) in points.iter().enumerate() {
                        let dx = i - point.x;
                        let dy = j - point.y;
                        let distance = if method == SparseColorMethod::Voronoi { dx * dx + dy * dy } else { dx.abs() + dy.abs() };
                        if distance < minimum {
                            minimum = distance;
                            nearest = index;
                        }
                    }
                    for (channel, value) in pixel.iter_mut().enumerate() {
                        let color = points[nearest].color.get(channel).copied().unwrap_or(0.0) as f64;
                        *value = color.clamp(0.0, 1.0) as f32;
                    }
                }
                SparseColorMethod::Shepards | SparseColorMethod::Inverse => {
                    sums.iter_mut().for_each(|sum| *sum = 0.0);
                    let mut denominator = 0.0f64;
                    for point in points {
                        let dx = i - point.x;
                        let dy = j - point.y;
                        let mut weight = (dx * dx + dy * dy).powf(power);
                        weight = if weight < 1.0 { 1.0 } else { 1.0 / weight };
                        for (channel, sum) in sums.iter_mut().enumerate() {
                            *sum += point.color.get(channel).copied().unwrap_or(0.0) as f64 * weight;
                        }
                        denominator += weight;
                    }
                    for (channel, value) in pixel.iter_mut().enumerate() {
                        *value = (sums[channel] / denominator).clamp(0.0, 1.0) as f32;
                    }
                }
            }
        }
    });
    result
}

/// IM `ClutImage(image, clut, method)`: every channel value is replaced by
/// the same channel of the colour table sampled along its diagonal. A grey
/// image is relabelled sRGB first when the table is in colour.
pub fn clut(image: &mut Image, table: &Image, method: Interpolate) {
    if image.colorspace.is_gray() && !table.colorspace.is_gray() {
        image.set_colorspace(Colorspace::SRgb);
    }
    let adjust = if table.interpolate == Interpolate::Integer { 0.0 } else { 1.0 };
    let table_colors = if table.channels == 2 || table.channels == 4 { table.channels - 1 } else { table.channels };
    let table_alpha = if table.channels == 2 || table.channels == 4 { Some(table.channels - 1) } else { None };
    let map: Vec<[f32; 4]> = (0..=MAX_MAP)
        .into_par_iter()
        .map(|index| {
            let x = index as f64 * (table.width as f64 - adjust) / MAX_MAP as f64;
            let y = index as f64 * (table.height as f64 - adjust) / MAX_MAP as f64;
            let mut entry = [0.0f32; 4];
            for (channel, value) in entry.iter_mut().take(3).enumerate() {
                let source = channel.min(table_colors - 1);
                *value = table.interpolate(x, y, source, method);
            }
            entry[3] = match table_alpha {
                Some(alpha) => table.interpolate(x, y, alpha, method),
                None => 1.0,
            };
            entry
        })
        .collect();
    let channels = image.channels;
    let colors = if channels == 2 || channels == 4 { channels - 1 } else { channels };
    let has_alpha = channels == 2 || channels == 4;
    image.data.par_chunks_mut(channels).for_each(|pixel| {
        for channel in 0..colors {
            // Grey images look up their single channel as blue, IM's last write.
            let table_channel = if colors == 1 { 2 } else { channel };
            pixel[channel] = map[map_index(pixel[channel] as f64)][table_channel];
        }
        if has_alpha {
            let alpha = channels - 1;
            pixel[alpha] = map[map_index(pixel[alpha] as f64)][3];
        }
    });
}

/// IM `AppendImages(images, stack = false)` for images of equal height:
/// the images side by side, in the first image's colourspace.
pub fn append_horizontally(images: &[Image]) -> Image {
    let height = images.iter().map(|image| image.height).max().unwrap_or(0);
    let width = images.iter().map(|image| image.width).sum();
    let first = &images[0];
    let channels = first.channels;
    let mut result = first.new_like(width, height);
    let mut x_offset = 0;
    for image in images {
        for y in 0..image.height {
            for x in 0..image.width {
                for channel in 0..channels {
                    let source = channel.min(image.channels - 1);
                    result.set(x_offset + x, y, channel, image.get(x, y, source));
                }
            }
        }
        x_offset += image.width;
    }
    result
}

/// IM `OrderedDitherImage(image, "map[,levels]")` with the maps of
/// `thresholds.xml` (by name or alias). Unknown maps are an error, as in IM.
/// Like IM's HDRI build, results are not clamped: the brightest pixels can
/// reach twice the quantum range.
pub fn ordered_dither(image: &mut Image, threshold_map: &str) -> Result<(), String> {
    let trimmed = threshold_map.trim_start_matches(|character: char| character.is_whitespace() || character == ',');
    let name: String = trimmed.chars().take_while(|character| !character.is_whitespace() && *character != ',').collect();
    let map = THRESHOLD_MAPS.iter().find(|map| map.name == name || map.alias == Some(name.as_str()));
    let Some(map) = map else {
        return Err(format!("ordered-dither : '{threshold_map}'"));
    };
    let mut levels = 2.0f64;
    if let Some((_, rest)) = trimmed.split_once(',') {
        let token: String = rest.chars().take_while(|character| character.is_ascii_digit() || *character == '.').collect();
        if let Ok(value) = token.parse::<f64>() {
            levels = value;
        }
    }
    if levels.abs() >= 1.0 {
        levels -= 1.0;
    }
    if levels.abs() < 1.0e-12 {
        return Ok(());
    }
    let width = image.width;
    let channels = image.channels;
    let alpha = if channels == 2 || channels == 4 { Some(channels - 1) } else { None };
    let divisor = map.divisor;
    image.data.par_chunks_mut(width * channels).enumerate().for_each(|(y, row)| {
        for x in 0..width {
            let cell = map.levels[(x % map.width) + map.width * (y % map.height)];
            for channel in 0..channels {
                if Some(channel) == alpha {
                    continue;
                }
                let value = &mut row[x * channels + channel];
                let quantum = *value as f64 * QUANTUM_RANGE;
                let mut threshold = (QUANTUM_SCALE * quantum * (levels * (divisor - 1) as f64 + 1.0)) as i64;
                let level = threshold / (divisor - 1);
                threshold -= level * (divisor - 1);
                let bumped = level + i64::from(threshold >= cell);
                *value = (bumped as f64 * QUANTUM_RANGE / levels / QUANTUM_RANGE) as f32;
            }
        }
    });
    Ok(())
}

/// Grey quantisation to `colors` evenly spaced levels (IM `Quantize` on grey images).
pub fn quantize_gray(image: &mut Image, colors: usize) {
    let levels = colors.max(2) as f32 - 1.0;
    image.map_in_place(|value| (value.clamp(0.0, 1.0) * levels).round() / levels);
}

/// IM `Trim`: bounding box of pixels that differ from the corner colour.
pub fn trim_bounds(image: &Image, fuzz: f32) -> (usize, usize, usize, usize) {
    let reference: Vec<f32> = (0..image.channels).map(|channel| image.get(0, 0, channel)).collect();
    let mut min_x = image.width;
    let mut min_y = image.height;
    let mut max_x = 0;
    let mut max_y = 0;
    for y in 0..image.height {
        for x in 0..image.width {
            let differs = (0..image.channels).any(|channel| (image.get(x, y, channel) - reference[channel]).abs() > fuzz);
            if differs {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    if min_x > max_x || min_y > max_y {
        return (0, 0, image.width, image.height);
    }
    (min_x, min_y, max_x - min_x + 1, max_y - min_y + 1)
}

pub fn trim(image: &Image) -> Image {
    let (x, y, width, height) = trim_bounds(image, 0.0);
    image.crop(x as isize, y as isize, width, height)
}

/// Draws an anti-aliased 1px line with the given grey/colour value.
pub fn draw_line(image: &mut Image, x0: f64, y0: f64, x1: f64, y1: f64, color: &[f32], stroke_width: f64) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let length = (dx * dx + dy * dy).sqrt().max(1e-9);
    let half = stroke_width.max(1.0) / 2.0;
    let min_x = (x0.min(x1) - half - 1.0).floor().max(0.0) as usize;
    let max_x = ((x0.max(x1) + half + 1.0).ceil() as usize).min(image.width.saturating_sub(1));
    let min_y = (y0.min(y1) - half - 1.0).floor().max(0.0) as usize;
    let max_y = ((y0.max(y1) + half + 1.0).ceil() as usize).min(image.height.saturating_sub(1));
    let channels = image.channels;
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let px = x as f64 + 0.5;
            let py = y as f64 + 0.5;
            let t = (((px - x0) * dx + (py - y0) * dy) / (length * length)).clamp(0.0, 1.0);
            let cx = x0 + t * dx;
            let cy = y0 + t * dy;
            let distance = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
            let coverage = (half + 0.5 - distance).clamp(0.0, 1.0) as f32;
            if coverage <= 0.0 {
                continue;
            }
            for channel in 0..channels {
                let target = color.get(channel).copied().unwrap_or(color[0]);
                let current = image.get(x, y, channel);
                image.set(x, y, channel, current + (target - current) * coverage);
            }
        }
    }
}

/// Grey intensity of an RGB(A) image as a single-channel image.
pub fn intensity_image(image: &Image) -> Image {
    if image.channels == 1 {
        return image.clone();
    }
    let channels = image.channels;
    let data: Vec<f32> = image.data.par_chunks(channels).map(|pixel| pixel_intensity(pixel[0], pixel[1], pixel[2]) as f32).collect();
    Image::from_data(image.width, image.height, 1, data)
}
