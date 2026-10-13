//! Convolution and blur operations matching ImageMagick's kernel generators
//! (`morphology.c` / `effect.c`).

use rayon::prelude::*;

use crate::image::{Image, QUANTUM_SCALE};

pub const MAGICK_EPSILON: f64 = 1.0e-12;
const MAGICK_SQ2PI: f64 = 2.506_628_274_631_000_241_612_355_239_340_104_162_693_023_681_640_62;
const MAGICK_2PI: f64 = std::f64::consts::TAU;
const KERNEL_RANK: usize = 3;

/// A convolution kernel with its origin.
#[derive(Clone, Debug, PartialEq)]
pub struct Kernel {
    pub width: usize,
    pub height: usize,
    pub x: usize,
    pub y: usize,
    /// Row-major values; NaN marks "ignore this element" like IM.
    pub values: Vec<f64>,
}

impl Kernel {
    pub fn new(width: usize, height: usize, values: Vec<f64>) -> Self {
        Self {
            width,
            height,
            x: (width - 1) / 2,
            y: (height - 1) / 2,
            values,
        }
    }

    /// Rotates a 1D horizontal kernel into a vertical one (IM "+90").
    pub fn rotated_90(&self) -> Kernel {
        let mut values = vec![0.0; self.values.len()];
        for row in 0..self.height {
            for column in 0..self.width {
                let target_row = column;
                let target_column = self.height - 1 - row;
                values[target_row * self.height + target_column] = self.values[row * self.width + column];
            }
        }
        Kernel {
            width: self.height,
            height: self.width,
            x: self.height - 1 - self.y,
            y: self.x,
            values,
        }
    }

    /// IM `ScaleKernelInfo(kernel, 1.0, CorrelateNormalizeValue)` for non-negative kernels.
    pub fn normalize(&mut self) {
        let sum: f64 = self.values.iter().filter(|value| !value.is_nan()).sum();
        if sum.abs() > MAGICK_EPSILON {
            for value in self.values.iter_mut() {
                *value /= sum;
            }
        }
    }
}

/// IM `GetOptimalKernelWidth1D`.
pub fn optimal_kernel_width_1d(radius: f64, sigma: f64) -> usize {
    if radius > MAGICK_EPSILON {
        return (2.0 * radius.ceil() + 1.0) as usize;
    }
    let gamma = sigma.abs();
    if gamma <= MAGICK_EPSILON {
        return 3;
    }
    let alpha = 1.0 / (2.0 * gamma * gamma);
    let beta = 1.0 / (MAGICK_SQ2PI * gamma);
    let mut width = 5usize;
    loop {
        let half = (width as isize - 1) / 2;
        let mut normalize = 0.0;
        for i in -half..=half {
            normalize += (-((i * i) as f64) * alpha).exp() * beta;
        }
        let value = (-((half * half) as f64) * alpha).exp() * beta / normalize;
        if value < QUANTUM_SCALE || value < MAGICK_EPSILON {
            break;
        }
        width += 2;
    }
    width - 2
}

/// IM `GetOptimalKernelWidth2D`.
pub fn optimal_kernel_width_2d(radius: f64, sigma: f64) -> usize {
    if radius > MAGICK_EPSILON {
        return (2.0 * radius.ceil() + 1.0) as usize;
    }
    let gamma = sigma.abs();
    if gamma <= MAGICK_EPSILON {
        return 3;
    }
    let alpha = 1.0 / (2.0 * gamma * gamma);
    let beta = 1.0 / (MAGICK_2PI * gamma * gamma);
    let mut width = 5usize;
    loop {
        let half = (width as isize - 1) / 2;
        let mut normalize = 0.0;
        for v in -half..=half {
            for u in -half..=half {
                normalize += (-((u * u + v * v) as f64) * alpha).exp() * beta;
            }
        }
        let value = (-((half * half) as f64) * alpha).exp() * beta / normalize;
        if value < QUANTUM_SCALE || value < MAGICK_EPSILON {
            break;
        }
        width += 2;
    }
    width - 2
}

/// IM "blur:radius x sigma" 1D kernel (3x over-sampled then compressed).
pub fn blur_kernel(radius: f64, sigma: f64) -> Kernel {
    let sigma = sigma.abs();
    let width = if radius >= 1.0 { (radius as usize) * 2 + 1 } else { optimal_kernel_width_1d(radius, sigma) };
    let mut values = vec![0.0f64; width];
    if sigma > MAGICK_EPSILON {
        let half = ((width * KERNEL_RANK - 1) / 2) as isize;
        let ranked_sigma = sigma * KERNEL_RANK as f64;
        let alpha = 1.0 / (2.0 * ranked_sigma * ranked_sigma);
        let beta = 1.0 / (MAGICK_SQ2PI * ranked_sigma);
        for u in -half..=half {
            let index = ((u + half) as usize) / KERNEL_RANK;
            values[index] += (-((u * u) as f64) * alpha).exp() * beta;
        }
    } else {
        values[(width - 1) / 2] = 1.0;
    }
    let mut kernel = Kernel::new(width, 1, values);
    kernel.normalize();
    kernel
}

/// IM "gaussian:radius x sigma" 2D kernel.
pub fn gaussian_kernel(radius: f64, sigma: f64) -> Kernel {
    let sigma = sigma.abs();
    let width = if radius >= 1.0 { (radius as usize) * 2 + 1 } else { optimal_kernel_width_2d(radius, sigma) };
    let half = (width as isize - 1) / 2;
    let mut values = vec![0.0f64; width * width];
    if sigma > MAGICK_EPSILON {
        let a = 1.0 / (2.0 * sigma * sigma);
        let b = 1.0 / (MAGICK_2PI * sigma * sigma);
        let mut index = 0;
        for v in -half..=half {
            for u in -half..=half {
                values[index] = (-((u * u + v * v) as f64) * a).exp() * b;
                index += 1;
            }
        }
    } else {
        values[(width * width) / 2] = 1.0;
    }
    let mut kernel = Kernel::new(width, width, values);
    kernel.normalize();
    kernel
}

/// IM `ConvolveImage`: true convolution (kernel reversed), virtual pixels at the edges.
pub fn convolve(image: &Image, kernel: &Kernel) -> Image {
    let channels = image.channels;
    let width = image.width;
    let mut result = image.new_like(width, image.height);
    let last = kernel.values.len() - 1;
    result
        .data
        .par_chunks_mut(width * channels)
        .enumerate()
        .for_each(|(y, out_row)| {
            for x in 0..width {
                for channel in 0..channels {
                    let mut sum = 0.0f64;
                    let mut index = 0usize;
                    for v in 0..kernel.height {
                        let source_y = y as isize + v as isize - kernel.y as isize;
                        for u in 0..kernel.width {
                            let weight = kernel.values[last - index];
                            index += 1;
                            if weight.is_nan() {
                                continue;
                            }
                            let source_x = x as isize + u as isize - kernel.x as isize;
                            sum += weight * image.virtual_get(source_x, source_y, channel) as f64;
                        }
                    }
                    out_row[x * channels + channel] = sum as f32;
                }
            }
        });
    result
}

/// Convolution with a 1D horizontal kernel (fast path, identical maths).
fn convolve_horizontal(image: &Image, weights: &[f64], origin: usize) -> Image {
    let channels = image.channels;
    let width = image.width;
    let mut result = image.new_like(width, image.height);
    let len = weights.len();
    result
        .data
        .par_chunks_mut(width * channels)
        .enumerate()
        .for_each(|(y, out_row)| {
            for x in 0..width {
                for channel in 0..channels {
                    let mut sum = 0.0f64;
                    for (u, _) in weights.iter().enumerate() {
                        let weight = weights[len - 1 - u];
                        let source_x = x as isize + u as isize - origin as isize;
                        sum += weight * image.virtual_get(source_x, y as isize, channel) as f64;
                    }
                    out_row[x * channels + channel] = sum as f32;
                }
            }
        });
    result
}

fn convolve_vertical(image: &Image, weights: &[f64], origin: usize) -> Image {
    let channels = image.channels;
    let width = image.width;
    let mut result = image.new_like(width, image.height);
    let len = weights.len();
    result
        .data
        .par_chunks_mut(width * channels)
        .enumerate()
        .for_each(|(y, out_row)| {
            for x in 0..width {
                for channel in 0..channels {
                    let mut sum = 0.0f64;
                    for (v, _) in weights.iter().enumerate() {
                        let weight = weights[len - 1 - v];
                        let source_y = y as isize + v as isize - origin as isize;
                        sum += weight * image.virtual_get(x as isize, source_y, channel) as f64;
                    }
                    out_row[x * channels + channel] = sum as f32;
                }
            }
        });
    result
}

/// IM `BlurImage(radius, sigma)`: separable gaussian built from the "blur" kernel.
pub fn blur(image: &Image, radius: f64, sigma: f64) -> Image {
    let kernel = blur_kernel(radius, sigma);
    let horizontal = convolve_horizontal(image, &kernel.values, kernel.x);
    convolve_vertical(&horizontal, &kernel.values, kernel.x)
}

/// IM `GaussianBlurImage(radius, sigma)`; evaluated separably (same result as the 2D kernel).
pub fn gaussian_blur(image: &Image, radius: f64, sigma: f64) -> Image {
    let kernel = gaussian_kernel(radius, sigma);
    let width = kernel.width;
    let center_row = &kernel.values[kernel.y * width..(kernel.y + 1) * width];
    let row_sum: f64 = center_row.iter().sum();
    // The normalised 2D gaussian factors into normalised 1D gaussians.
    let weights: Vec<f64> = center_row.iter().map(|value| value / row_sum).collect();
    let horizontal = convolve_horizontal(image, &weights, kernel.x);
    convolve_vertical(&horizontal, &weights, kernel.x)
}

/// IM `EdgeImage(radius)`.
pub fn edge(image: &Image, radius: f64) -> Image {
    let width = optimal_kernel_width_1d(radius, 0.5);
    let count = width * width;
    let mut values = vec![-1.0f64; count];
    values[count / 2] = count as f64 - 1.0;
    let kernel = Kernel::new(width, width, values);
    convolve(image, &kernel)
}

/// IM `EmbossImage(radius, sigma)`: anti-diagonal gaussian kernel, then equalize.
pub fn emboss(image: &Image, radius: f64, sigma: f64) -> Image {
    let sigma = if sigma.abs() < MAGICK_EPSILON { MAGICK_EPSILON } else { sigma };
    let width = optimal_kernel_width_1d(radius, sigma);
    let half = (width as isize - 1) / 2;
    let mut values = vec![0.0f64; width * width];
    let mut index = 0;
    let mut k = half;
    for v in -half..=half {
        for u in -half..=half {
            let base = if u < 0 || v < 0 { -8.0 } else { 8.0 };
            let distance = (u * u + v * v) as f64;
            let gaussian = (-distance / (2.0 * sigma * sigma)).exp() / (2.0 * std::f64::consts::PI * sigma * sigma);
            values[index] = if u == k { base * gaussian } else { 0.0 };
            index += 1;
        }
        k -= 1;
    }
    let normalize: f64 = values.iter().sum();
    let gamma = if normalize.abs() >= MAGICK_EPSILON {
        1.0 / normalize
    } else {
        normalize.signum() / MAGICK_EPSILON
    };
    for value in values.iter_mut() {
        *value *= gamma;
    }
    let kernel = Kernel::new(width, width, values);
    let mut result = convolve(image, &kernel);
    crate::levels::equalize(&mut result);
    result
}

/// IM `SharpenImage(radius, sigma)`.
pub fn sharpen(image: &Image, radius: f64, sigma: f64) -> Image {
    let width = optimal_kernel_width_2d(radius, sigma);
    let half = (width as isize - 1) / 2;
    let mut values = vec![0.0f64; width * width];
    let mut normalize = 0.0;
    let mut index = 0;
    for v in -half..=half {
        for u in -half..=half {
            let value = -(-((u * u + v * v) as f64) / (2.0 * sigma * sigma)).exp() / (MAGICK_2PI * sigma * sigma);
            values[index] = value;
            normalize += value;
            index += 1;
        }
    }
    let center = (width * width) / 2;
    values[center] = -2.0 * normalize;
    let mut kernel = Kernel::new(width, width, values);
    kernel.normalize();
    convolve(image, &kernel)
}

/// IM `UnsharpMaskImage(radius, sigma, gain, threshold)`.
pub fn unsharp_mask(image: &Image, radius: f64, sigma: f64, gain: f64, threshold: f64) -> Image {
    let blurred = gaussian_blur(image, radius, sigma);
    let mut result = image.clone();
    let quantum_threshold = threshold;
    result.data.par_iter_mut().zip(blurred.data.par_iter()).for_each(|(value, blur_value)| {
        let difference = *value as f64 - *blur_value as f64;
        if difference.abs() >= quantum_threshold {
            *value = (*value as f64 + difference * gain) as f32;
        }
    });
    result
}

/// IM `MotionBlurImage(radius, sigma, angle)`: a one-sided gaussian along `angle`.
pub fn motion_blur(image: &Image, radius: f64, sigma: f64, angle: f64) -> Image {
    let width = optimal_kernel_width_1d(radius, sigma);
    let magick_sigma = if sigma.abs() < MAGICK_EPSILON { MAGICK_EPSILON } else { sigma };
    let mut weights = vec![0.0f64; width];
    let mut normalize = 0.0;
    for (index, weight) in weights.iter_mut().enumerate() {
        let i = index as f64;
        *weight = (-(i * i) / (2.0 * magick_sigma * magick_sigma)).exp() / (MAGICK_SQ2PI * magick_sigma);
        normalize += *weight;
    }
    for weight in weights.iter_mut() {
        *weight /= normalize;
    }
    let radians = std::f64::consts::PI * angle / 180.0;
    let point_x = width as f64 * radians.sin();
    let point_y = width as f64 * radians.cos();
    let length = point_x.hypot(point_y);
    let offsets: Vec<(isize, isize)> = (0..width)
        .map(|index| {
            let i = index as f64;
            let offset_x = ((i * point_y) / length - 0.5).ceil() as isize;
            let offset_y = ((i * point_x) / length - 0.5).ceil() as isize;
            (offset_x, offset_y)
        })
        .collect();
    let channels = image.channels;
    let image_width = image.width;
    let mut result = image.new_like(image_width, image.height);
    result.data.par_chunks_mut(image_width * channels).enumerate().for_each(|(y, out_row)| {
        for x in 0..image_width {
            for channel in 0..channels {
                let mut sum = 0.0f64;
                for (weight, offset) in weights.iter().zip(offsets.iter()) {
                    sum += weight * image.virtual_get(x as isize + offset.0, y as isize + offset.1, channel) as f64;
                }
                out_row[x * channels + channel] = sum as f32;
            }
        }
    });
    result
}

/// IM `RotationalBlurImage(angle)`: averages nearest samples along arcs about
/// the image centre, thinning the samples near the centre.
pub fn rotational_blur(image: &Image, angle: f64) -> Image {
    let center_x = (image.width as f64 - 1.0) / 2.0;
    let center_y = (image.height as f64 - 1.0) / 2.0;
    let blur_radius = center_x.hypot(center_y);
    let radians = std::f64::consts::PI * angle / 180.0;
    let count = (4.0 * radians * blur_radius.sqrt() + 2.0).abs() as usize;
    let theta = radians / (count as f64 - 1.0);
    let offset = theta * (count as f64 - 1.0) / 2.0;
    let mut cos_table = Vec::with_capacity(count);
    let mut sin_table = Vec::with_capacity(count);
    for index in 0..count {
        let step_angle = theta * index as f64 - offset;
        cos_table.push(step_angle.cos());
        sin_table.push(step_angle.sin());
    }
    let channels = image.channels;
    let width = image.width;
    let mut result = image.clone();
    result.data.par_chunks_mut(width * channels).enumerate().for_each(|(y, out_row)| {
        let dy = y as f64 - center_y;
        for x in 0..width {
            let dx = x as f64 - center_x;
            let radius = dx.hypot(dy);
            let step = if radius == 0.0 {
                1
            } else {
                let ratio = (blur_radius / radius) as usize;
                if ratio == 0 {
                    1
                } else if ratio >= count {
                    count - 1
                } else {
                    ratio
                }
            };
            for channel in 0..channels {
                let mut sum = 0.0f64;
                let mut samples = 0.0f64;
                let mut index = 0;
                while index < count {
                    let sample_x = (center_x + dx * cos_table[index] - dy * sin_table[index] + 0.5) as isize;
                    let sample_y = (center_y + dx * sin_table[index] + dy * cos_table[index] + 0.5) as isize;
                    sum += image.virtual_get(sample_x, sample_y, channel) as f64;
                    samples += 1.0;
                    index += step.max(1);
                }
                out_row[x * channels + channel] = (crate::image::perceptible_reciprocal(samples) * sum) as f32;
            }
        }
    });
    result
}

/// Per-size kernels of IM's adaptive blur/sharpen, indexed by the even
/// reduction `j` (kernel `j` is `(width - j)` square).
fn adaptive_kernels(width: usize, sigma: f64, sharpen: bool) -> Vec<Vec<f64>> {
    let magick_sigma = if sigma.abs() < MAGICK_EPSILON { MAGICK_EPSILON } else { sigma };
    let mut kernels = vec![Vec::new(); width];
    let mut reduction = 0;
    while reduction < width {
        let side = width - reduction;
        let half = (side as isize - 1) / 2;
        let mut values = Vec::with_capacity(side * side);
        let mut normalize = 0.0f64;
        for v in -half..=half {
            for u in -half..=half {
                let distance = (u as f64 * u as f64) + (v * v) as f64;
                let gaussian = (-distance / (2.0 * magick_sigma * magick_sigma)).exp() / (2.0 * std::f64::consts::PI * magick_sigma * magick_sigma);
                let value = if sharpen { -gaussian } else { gaussian };
                values.push(value);
                normalize += value;
            }
        }
        let center = (values.len() - 1) / 2;
        if sharpen {
            values[center] = -2.0 * normalize;
        } else {
            values[center] += 1.0 - normalize;
        }
        if sigma < MAGICK_EPSILON {
            values[center] = 1.0;
        }
        kernels[reduction] = values;
        reduction += 2;
    }
    kernels
}

/// IM `AdaptiveBlurImage` / `AdaptiveSharpenImage`: the kernel size at each
/// pixel shrinks with the (levelled, blurred) edge strength.
fn adaptive_filter(image: &Image, radius: f64, sigma: f64, sharpen: bool) -> Image {
    if sigma.abs() < MAGICK_EPSILON {
        return image.clone();
    }
    let mut edges = edge(image, radius);
    crate::levels::auto_level(&mut edges);
    let mut edges = blur(&edges, radius, sigma);
    crate::levels::auto_level(&mut edges);
    let width = optimal_kernel_width_2d(radius, sigma);
    let kernels = adaptive_kernels(width, sigma, sharpen);

    let channels = image.channels;
    let color_channels = if channels == 2 || channels == 4 { channels - 1 } else { channels };
    let image_width = image.width;
    let mut result = image.clone();
    result.data.par_chunks_mut(image_width * channels).enumerate().for_each(|(y, out_row)| {
        for x in 0..image_width {
            let intensity = edges.intensity(x, y);
            let mut reduction = (width as f64 * (1.0 - intensity) - 0.5).ceil() as isize;
            reduction = reduction.clamp(0, width as isize);
            if reduction & 1 != 0 {
                reduction -= 1;
            }
            let side = width - reduction as usize;
            let offset = (side / 2) as isize;
            let kernel = &kernels[reduction as usize];
            for channel in 0..color_channels {
                let mut pixel = 0.0f64;
                let mut gamma = 0.0f64;
                let mut index = 0;
                for v in 0..side as isize {
                    for u in 0..side as isize {
                        let weight = kernel[index];
                        index += 1;
                        pixel += weight * image.virtual_get(x as isize - offset + u, y as isize - offset + v, channel) as f64;
                        gamma += weight;
                    }
                }
                let gamma = crate::image::perceptible_reciprocal(gamma);
                out_row[x * channels + channel] = (gamma * pixel) as f32;
            }
        }
    });
    result
}

/// IM `AdaptiveBlurImage(radius, sigma)`.
pub fn adaptive_blur(image: &Image, radius: f64, sigma: f64) -> Image {
    adaptive_filter(image, radius, sigma, false)
}

/// IM `AdaptiveSharpenImage(radius, sigma)`.
pub fn adaptive_sharpen(image: &Image, radius: f64, sigma: f64) -> Image {
    adaptive_filter(image, radius, sigma, true)
}
