//! ImageMagick-compatible resizing (`resize.c`).

use rayon::prelude::*;

use crate::image::{perceptible_reciprocal, Image, Interpolate};

const MAGICK_EPSILON: f64 = 1.0e-12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Point,
    Box,
    Triangle,
    Hermite,
    Mitchell,
    Catrom,
    Cubic,
    Lanczos,
    Gaussian,
}

impl Filter {
    /// Default support (`filters[].support` in `AcquireResizeFilter`).
    fn support(self) -> f64 {
        match self {
            Filter::Point => 0.0,
            Filter::Box => 0.5,
            Filter::Triangle | Filter::Hermite => 1.0,
            Filter::Gaussian | Filter::Mitchell | Filter::Catrom | Filter::Cubic => 2.0,
            Filter::Lanczos => 3.0,
        }
    }

    /// `GetResizeFilterWeight` for the filter/window pair IM maps this to.
    fn weight(self, x: f64) -> f64 {
        let x = x.abs();
        match self {
            // Box() ignores its argument; the support limits the taps.
            Filter::Point | Filter::Box => 1.0,
            Filter::Triangle => {
                if x < 1.0 { 1.0 - x } else { 0.0 }
            }
            Filter::Hermite => CubicCoefficients::new(0.0, 0.0).weight(x),
            Filter::Gaussian => (-GAUSSIAN_SCALE * x * x).exp(),
            Filter::Cubic => CubicCoefficients::new(1.0, 0.0).weight(x),
            Filter::Catrom => CubicCoefficients::new(0.0, 0.5).weight(x),
            Filter::Mitchell => CubicCoefficients::new(1.0 / 3.0, 1.0 / 3.0).weight(x),
            // Sinc-windowed sinc; the window is scaled by 1 / support.
            Filter::Lanczos => sinc_fast(x * (1.0 / 3.0)) * sinc_fast(x),
        }
    }
}

/// `1 / (2 sigma^2)` for IM's default resize Gaussian (sigma 0.5).
const GAUSSIAN_SCALE: f64 = 2.0;

/// Cubic BC-spline coefficients exactly as `AcquireResizeFilter` derives them.
struct CubicCoefficients([f64; 7]);

impl CubicCoefficients {
    fn new(b: f64, c: f64) -> Self {
        let two_b = b + b;
        Self([
            1.0 - (1.0 / 3.0) * b,
            -3.0 + two_b + c,
            2.0 - 1.5 * b - c,
            (4.0 / 3.0) * b + 4.0 * c,
            -8.0 * c - two_b,
            b + 5.0 * c,
            (-1.0 / 6.0) * b - c,
        ])
    }

    /// `CubicBC()`.
    fn weight(&self, x: f64) -> f64 {
        let k = &self.0;
        if x < 1.0 {
            return k[0] + x * (x * (k[1] + x * k[2]));
        }
        if x < 2.0 {
            return k[3] + x * (k[4] + x * (k[5] + x * k[6]));
        }
        0.0
    }
}

/// IM's `SincFast()` (Q16 polynomial approximation of sin(pi x) / (pi x)).
fn sinc_fast(x: f64) -> f64 {
    if x > 4.0 {
        let alpha = std::f64::consts::PI * x;
        return alpha.sin() / alpha;
    }
    let xx = x * x;
    const C0: f64 = 0.173611107357320220183368594093166520811e-2;
    const C1: f64 = -3.842_409_211_149_466e-4;
    const C2: f64 = 0.394201182359318128221229891724947048771e-4;
    const C3: f64 = -2.509_633_016_091_172e-6;
    const C4: f64 = 0.111902032818095784414237782071368805120e-6;
    const C5: f64 = -3.728_951_014_087_796e-9;
    const C6: f64 = 0.957694196677572570319816780188718518330e-10;
    const C7: f64 = -1.872_085_777_765_907e-12;
    const C8: f64 = 0.253524321426864752676094495396308636823e-13;
    const C9: f64 = -1.770_848_050_107_011e-16;
    let p = C0 + xx * (C1 + xx * (C2 + xx * (C3 + xx * (C4 + xx * (C5 + xx * (C6 + xx * (C7 + xx * (C8 + xx * C9))))))));
    (xx - 1.0) * (xx - 4.0) * (xx - 9.0) * (xx - 16.0) * p
}

#[derive(Clone)]
struct Contribution {
    start: usize,
    weights: Vec<f64>,
}

/// Normalised filter taps for every target pixel (`HorizontalFilter` /
/// `VerticalFilter` set-up).
fn contributions(source_len: usize, target_len: usize, filter: Filter) -> Vec<Contribution> {
    let factor = target_len as f64 / source_len as f64;
    let mut scale = (1.0 / factor + MAGICK_EPSILON).max(1.0);
    let mut support = scale * filter.support();
    if support < 0.5 {
        support = 0.5;
        scale = 1.0;
    }
    let scale = perceptible_reciprocal(scale);
    let mut result = Vec::with_capacity(target_len);
    for index in 0..target_len {
        let bisect = (index as f64 + 0.5) / factor + MAGICK_EPSILON;
        let start = (bisect - support + 0.5).max(0.0) as usize;
        let stop = (bisect + support + 0.5).min(source_len as f64) as usize;
        let mut weights = Vec::with_capacity(stop.saturating_sub(start));
        let mut density = 0.0f64;
        for pixel in start..stop {
            let weight = filter.weight(scale * (pixel as f64 - bisect + 0.5));
            weights.push(weight);
            density += weight;
        }
        if density != 0.0 && density != 1.0 {
            let reciprocal = perceptible_reciprocal(density);
            for weight in weights.iter_mut() {
                *weight *= reciprocal;
            }
        }
        result.push(Contribution { start, weights });
    }
    result
}

/// `MagickImage.Resize(width, height)`: the image's own filter when one is
/// set (`FilterType`), otherwise IM's default choice: a copy for identity,
/// Mitchell when enlarging, Lanczos otherwise.
pub fn resize(image: &Image, width: usize, height: usize) -> Image {
    if let Some(filter) = image.filter {
        return resize_with_filter(image, width, height, filter);
    }
    let x_factor = width as f64 / image.width as f64;
    let y_factor = height as f64 / image.height as f64;
    if x_factor == 1.0 && y_factor == 1.0 {
        return image.clone();
    }
    let has_alpha = image.channels == 2 || image.channels == 4;
    let filter = if has_alpha || x_factor * y_factor > 1.0 { Filter::Mitchell } else { Filter::Lanczos };
    resize_with_filter(image, width, height, filter)
}

/// The size IM's `ParseMetaGeometry` gives a plain `"WxH"` geometry for a
/// `width` x `height` image: scaled to fit inside the box, keeping the aspect
/// ratio (each side at least one pixel).
pub fn fit_size(width: usize, height: usize, box_width: usize, box_height: usize) -> (usize, usize) {
    if box_width == width && box_height == height {
        return (width, height);
    }
    let mut scale = 1.0;
    if width != 0 && height != 0 {
        scale = box_width as f64 / width as f64;
        let height_scale = box_height as f64 / height as f64;
        if scale > height_scale {
            scale = height_scale;
        }
    }
    let fitted_width = ((scale * width as f64 + 0.5).floor() as usize).max(1);
    let fitted_height = ((scale * height as f64 + 0.5).floor() as usize).max(1);
    (fitted_width, fitted_height)
}

/// Magick.NET `Resize(width, height)`: an aspect-preserving resize into the
/// `width` x `height` box.
pub fn resize_to_fit(image: &Image, box_width: usize, box_height: usize) -> Image {
    let (width, height) = fit_size(image.width, image.height, box_width, box_height);
    resize(image, width, height)
}

/// `ResizeImage(image, width, height, filter)`: the pass along the axis with
/// the larger factor runs first, exactly like IM.
pub fn resize_with_filter(image: &Image, width: usize, height: usize, filter: Filter) -> Image {
    let x_factor = width as f64 / image.width as f64;
    let y_factor = height as f64 / image.height as f64;
    let mut result = if x_factor > y_factor {
        let horizontal = resize_horizontal(image, width, filter);
        resize_vertical(&horizontal, height, filter)
    } else {
        let vertical = resize_vertical(image, height, filter);
        resize_horizontal(&vertical, width, filter)
    };
    result.virtual_pixel = image.virtual_pixel;
    result
}

fn resize_horizontal(image: &Image, width: usize, filter: Filter) -> Image {
    if width == image.width {
        return image.clone();
    }
    let channels = image.channels;
    let alpha_channel = alpha_channel_of(channels);
    let weights = contributions(image.width, width, filter);
    let mut result = image.new_like(width, image.height);
    result.data.par_chunks_mut(width * channels).enumerate().for_each(|(y, out_row)| {
        let source_row = &image.data[y * image.width * channels..(y + 1) * image.width * channels];
        for (x, contribution) in weights.iter().enumerate() {
            let taps = contribution.weights.iter().enumerate();
            let pixel = |offset: usize, channel: usize| source_row[(contribution.start + offset) * channels + channel] as f64;
            for channel in 0..channels {
                let value = match alpha_channel {
                    Some(alpha) if channel != alpha => {
                        // Colour channels are weighted by alpha (`BlendPixelTrait`).
                        let mut sum = 0.0f64;
                        let mut gamma = 0.0f64;
                        for (offset, weight) in taps.clone() {
                            let weighted_alpha = weight * pixel(offset, alpha);
                            sum += weighted_alpha * pixel(offset, channel);
                            gamma += weighted_alpha;
                        }
                        perceptible_reciprocal(gamma) * sum
                    }
                    _ => {
                        let mut sum = 0.0f64;
                        for (offset, weight) in taps.clone() {
                            sum += weight * pixel(offset, channel);
                        }
                        sum
                    }
                };
                out_row[x * channels + channel] = value as f32;
            }
        }
    });
    result
}

fn resize_vertical(image: &Image, height: usize, filter: Filter) -> Image {
    if height == image.height {
        return image.clone();
    }
    let channels = image.channels;
    let alpha_channel = alpha_channel_of(channels);
    let width = image.width;
    let row_len = width * channels;
    let weights = contributions(image.height, height, filter);
    let mut result = image.new_like(width, height);
    result.data.par_chunks_mut(row_len).enumerate().for_each(|(y, out_row)| {
        let contribution = &weights[y];
        let mut sums = vec![0.0f64; row_len];
        let mut gammas = vec![0.0f64; if alpha_channel.is_some() { width } else { 0 }];
        for (offset, weight) in contribution.weights.iter().enumerate() {
            let source_y = contribution.start + offset;
            let source_row = &image.data[source_y * row_len..(source_y + 1) * row_len];
            for x in 0..width {
                let pixel = &source_row[x * channels..(x + 1) * channels];
                let weighted_alpha = alpha_channel.map(|alpha| weight * pixel[alpha] as f64);
                for channel in 0..channels {
                    let tap = match weighted_alpha {
                        Some(alpha_weight) if Some(channel) != alpha_channel => alpha_weight,
                        _ => *weight,
                    };
                    sums[x * channels + channel] += tap * pixel[channel] as f64;
                }
                if let Some(alpha_weight) = weighted_alpha {
                    gammas[x] += alpha_weight;
                }
            }
        }
        for x in 0..width {
            // Colour is divided by the alpha weights, when there is alpha.
            let reciprocal = gammas.get(x).map(|gamma| perceptible_reciprocal(*gamma));
            for channel in 0..channels {
                let index = x * channels + channel;
                let value = match (alpha_channel, reciprocal) {
                    (Some(alpha), Some(reciprocal)) if channel != alpha => reciprocal * sums[index],
                    _ => sums[index],
                };
                out_row[index] = value as f32;
            }
        }
    });
    result
}

/// Index of the alpha channel for 2- and 4-channel images.
fn alpha_channel_of(channels: usize) -> Option<usize> {
    if channels == 2 || channels == 4 { Some(channels - 1) } else { None }
}

/// `MagickImage.AdaptiveResize` = interpolative resize with mesh interpolation.
pub fn adaptive_resize(image: &Image, width: usize, height: usize) -> Image {
    interpolative_resize(image, width, height, Interpolate::Mesh)
}

/// IM `InterpolativeResizeImage`.
pub fn interpolative_resize(image: &Image, width: usize, height: usize, method: Interpolate) -> Image {
    if width == image.width && height == image.height {
        return image.clone();
    }
    let channels = image.channels;
    let scale_x = image.width as f64 / width as f64;
    let scale_y = image.height as f64 / height as f64;
    let mut result = image.new_like(width, height);
    result
        .data
        .par_chunks_mut(width * channels)
        .enumerate()
        .for_each(|(y, out_row)| {
            let offset_y = (y as f64 + 0.5) * scale_y - 0.5;
            for x in 0..width {
                let offset_x = (x as f64 + 0.5) * scale_x - 0.5;
                for channel in 0..channels {
                    out_row[x * channels + channel] = image.interpolate(offset_x, offset_y, channel, method);
                }
            }
        });
    result
}

/// Nearest-neighbour resampling (IM `Sample`).
pub fn sample(image: &Image, width: usize, height: usize) -> Image {
    let channels = image.channels;
    let mut result = image.new_like(width, height);
    result
        .data
        .par_chunks_mut(width * channels)
        .enumerate()
        .for_each(|(y, out_row)| {
            let source_y = ((y as f64 + 0.5) * image.height as f64 / height as f64) as usize;
            let source_y = source_y.min(image.height - 1);
            for x in 0..width {
                let source_x = ((x as f64 + 0.5) * image.width as f64 / width as f64) as usize;
                let source_x = source_x.min(image.width - 1);
                for channel in 0..channels {
                    out_row[x * channels + channel] = image.get(source_x, source_y, channel);
                }
            }
        });
    result
}

/// IM `Minify`: half size with a box-like filter.
pub fn minify(image: &Image) -> Image {
    let width = (image.width / 2).max(1);
    let height = (image.height / 2).max(1);
    resize_with_filter(image, width, height, Filter::Box)
}

/// Convenience for square greyscale buffers.
pub fn resize_gray(data: &[f32], size: usize) -> Vec<f32> {
    let image = Image::from_gray(data);
    let resized = resize(&image, size, size);
    resized.data
}

/// Square RGB buffer resize.
pub fn resize_rgb(data: &[f32], size: usize) -> Vec<f32> {
    let image = Image::from_rgb(data);
    let resized = resize(&image, size, size);
    resized.data
}
