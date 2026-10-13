//! Noise reduction and statistical filters (`effect.c`, `paint.c`, `fx.c`).

use rayon::prelude::*;

use crate::convolve::blur;
use crate::image::{Image, QUANTUM_RANGE};

const NOISE_LEVELS: [f32; 8] = [0.8002, 0.2735, 0.1202, 0.0585, 0.0291, 0.0152, 0.0080, 0.0044];

/// IM "hat" transform used by the wavelet denoiser.
fn hat_transform(pixels: &[f32], base: usize, stride: usize, extent: usize, scale: usize, kernel: &mut [f32]) {
    let at = |index: isize| pixels[(base as isize + index * stride as isize) as usize];
    let scale_i = scale as isize;
    let mut i = 0usize;
    let mut p = 0isize;
    let mut q = scale_i;
    let mut r = scale_i;
    while i < scale {
        kernel[i] = 0.25 * (at(p) + at(p) + at(q) + at(r));
        p += 1;
        q -= 1;
        r += 1;
        i += 1;
    }
    while i < extent - scale {
        kernel[i] = 0.25 * (2.0 * at(p) + at(p - scale_i) + at(p + scale_i));
        p += 1;
        i += 1;
    }
    let mut q = p - scale_i;
    let mut r = extent as isize - 2;
    while i < extent {
        kernel[i] = 0.25 * (at(p) + at(p) + at(q) + at(r));
        p += 1;
        q += 1;
        r -= 1;
        i += 1;
    }
}

/// IM `WaveletDenoiseImage(threshold, softness)`; `threshold` is normalised
/// (fraction of QuantumRange).
pub fn wavelet_denoise(image: &Image, threshold: f64, softness: f64) -> Image {
    let width = image.width;
    let height = image.height;
    let pixel_count = width * height;
    let channels = image.channels;
    let mut result = image.clone();
    // IM only denoises the red, green and blue channels (never alpha).
    let color_channels = if channels == 2 || channels == 4 { channels - 1 } else { channels };
    // Five levels, fewer for images smaller than 32 pixels (IM 7.1.2).
    let mut number_levels = 5usize;
    while number_levels > 0 && (1usize << (number_levels - 1)) >= width.min(height) {
        number_levels -= 1;
    }
    for channel in 0..color_channels {
        let mut pixels = vec![0.0f32; 3 * pixel_count];
        for (index, pixel) in pixels.iter_mut().take(pixel_count).enumerate() {
            *pixel = image.data[index * channels + channel];
        }
        let mut high_pass = 0usize;
        let mut low_pass = 0usize;
        for (level, noise_level) in NOISE_LEVELS.iter().enumerate().take(number_levels) {
            low_pass = pixel_count * ((level & 1) + 1);
            let scale = 1usize << level;
            // Rows.
            let row_results: Vec<Vec<f32>> = (0..height)
                .into_par_iter()
                .map(|y| {
                    let mut kernel = vec![0.0f32; width];
                    hat_transform(&pixels, high_pass + y * width, 1, width, scale, &mut kernel);
                    kernel
                })
                .collect();
            for (y, row) in row_results.into_iter().enumerate() {
                pixels[low_pass + y * width..low_pass + (y + 1) * width].copy_from_slice(&row);
            }
            // Columns.
            let column_results: Vec<Vec<f32>> = (0..width)
                .into_par_iter()
                .map(|x| {
                    let mut kernel = vec![0.0f32; height];
                    hat_transform(&pixels, low_pass + x, width, height, scale, &mut kernel);
                    kernel
                })
                .collect();
            for (x, column) in column_results.into_iter().enumerate() {
                for (y, value) in column.into_iter().enumerate() {
                    pixels[low_pass + y * width + x] = value;
                }
            }
            // IM shrinks the coefficients in single precision.
            let magnitude = threshold * *noise_level as f64;
            let shrink = (magnitude - softness * magnitude) as f32;
            let attenuation = softness as f32;
            for index in 0..pixel_count {
                let mut detail = pixels[high_pass + index] - pixels[low_pass + index];
                if (detail as f64) < -magnitude {
                    detail += shrink;
                } else if (detail as f64) > magnitude {
                    detail -= shrink;
                } else {
                    detail *= attenuation;
                }
                pixels[high_pass + index] = detail;
                if high_pass != 0 {
                    pixels[index] += detail;
                }
            }
            high_pass = low_pass;
        }
        for index in 0..pixel_count {
            result.data[index * channels + channel] = pixels[index] + pixels[low_pass + index];
        }
    }
    result
}

/// IM `DespeckleImage` (Crimmins speckle removal).
pub fn despeckle(image: &Image) -> Image {
    const X: [isize; 4] = [0, 1, 1, -1];
    const Y: [isize; 4] = [1, 0, 1, 1];
    let step = 257.0f32 / 65535.0;
    let width = image.width;
    let height = image.height;
    let padded_width = width + 2;
    let length = padded_width * (height + 2);
    let channels = image.channels;
    let mut result = image.clone();
    for channel in 0..channels {
        let mut pixels = vec![0.0f32; length];
        let mut buffer = vec![0.0f32; length];
        let mut j = padded_width;
        for y in 0..height {
            j += 1;
            for x in 0..width {
                pixels[j] = image.get(x, y, channel);
                j += 1;
            }
            j += 1;
        }
        for k in 0..4 {
            hull(X[k], Y[k], width, height, 1, &mut pixels, &mut buffer, step);
            hull(-X[k], -Y[k], width, height, 1, &mut pixels, &mut buffer, step);
            hull(-X[k], -Y[k], width, height, -1, &mut pixels, &mut buffer, step);
            hull(X[k], Y[k], width, height, -1, &mut pixels, &mut buffer, step);
        }
        let mut j = padded_width;
        for y in 0..height {
            j += 1;
            for x in 0..width {
                result.set(x, y, channel, pixels[j]);
                j += 1;
            }
            j += 1;
        }
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn hull(x_offset: isize, y_offset: isize, columns: usize, rows: usize, polarity: i32, f: &mut [f32], g: &mut [f32], step: f32) {
    let stride = columns as isize + 2;
    let base = stride;
    let shift = y_offset * stride + x_offset;
    for y in 0..rows as isize {
        let row_start = (2 * y + 1) + y * columns as isize;
        for i in row_start..row_start + columns as isize {
            let p_index = (base + i) as usize;
            let r_index = (base + i + shift) as usize;
            let mut v = f[p_index];
            if polarity > 0 {
                if f[r_index] >= v + 2.0 * step {
                    v += step;
                }
            } else if f[r_index] <= v - 2.0 * step {
                v -= step;
            }
            g[p_index] = v;
        }
    }
    for y in 0..rows as isize {
        let row_start = (2 * y + 1) + y * columns as isize;
        for i in row_start..row_start + columns as isize {
            let q_index = (base + i) as usize;
            let r_index = (base + i + shift) as usize;
            let s_index = (base + i - shift) as usize;
            let mut v = g[q_index];
            if polarity > 0 {
                if g[s_index] >= v + 2.0 * step && g[r_index] > v {
                    v += step;
                }
            } else if g[s_index] <= v - 2.0 * step && g[r_index] < v {
                v -= step;
            }
            f[q_index] = v;
        }
    }
}

/// IM `ScaleQuantumToShort` (Q16 HDRI): the 16-bit key IM's pixel lists
/// sort by.
#[inline]
fn short_key(value: f32) -> u16 {
    let quantum = value * QUANTUM_RANGE as f32;
    if quantum.is_nan() || quantum <= 0.0 {
        return 0;
    }
    if quantum >= 65535.0 {
        return 65535;
    }
    (quantum as f64 + 0.5) as u16
}

/// Counts of 16-bit keys in a sliding window, with a coarse level of 256
/// buckets so ranks are found in at most 512 steps.
struct KeyHistogram {
    coarse: Vec<u32>,
    fine: Vec<u32>,
}

impl KeyHistogram {
    fn new() -> Self {
        Self {
            coarse: vec![0; 256],
            fine: vec![0; 65536],
        }
    }

    fn clear(&mut self) {
        self.coarse.fill(0);
        self.fine.fill(0);
    }

    fn add(&mut self, key: u16) {
        self.coarse[(key >> 8) as usize] += 1;
        self.fine[key as usize] += 1;
    }

    fn remove(&mut self, key: u16) {
        self.coarse[(key >> 8) as usize] -= 1;
        self.fine[key as usize] -= 1;
    }

    /// The key at sorted position `rank` (0-based).
    fn select(&self, rank: usize) -> u16 {
        let mut remaining = rank as u32;
        let mut bucket = 0;
        while self.coarse[bucket] <= remaining {
            remaining -= self.coarse[bucket];
            bucket += 1;
        }
        let mut key = bucket << 8;
        while self.fine[key] <= remaining {
            remaining -= self.fine[key];
            key += 1;
        }
        key as u16
    }
}

/// IM `StatisticImage(Median, width, height)`; Magick.NET's
/// `MedianFilter(radius)` passes the radius as both sizes. The window is
/// `width` x `height` with its origin `width / 2`, `height / 2` before the
/// pixel, and the median is taken over values keyed to 16 bits as IM's
/// pixel lists do, so results land on whole quantum steps.
pub fn median_filter(image: &Image, width: usize, height: usize) -> Image {
    let window_width = width.max(1);
    let window_height = height.max(1);
    let left = (window_width / 2) as isize;
    let top = (window_height / 2) as isize;
    let rank = window_width * window_height / 2;
    let channels = image.channels;
    let columns = image.width;
    let mut result = image.new_like(columns, image.height);
    result.data.par_chunks_mut(columns * channels).enumerate().for_each_init(KeyHistogram::new, |histogram, (y, out_row)| {
        let top_row = y as isize - top;
        let column_keys = |x: isize, channel: usize, histogram: &mut KeyHistogram, adding: bool| {
            for v in 0..window_height as isize {
                let key = short_key(image.virtual_get(x, top_row + v, channel));
                if adding {
                    histogram.add(key);
                } else {
                    histogram.remove(key);
                }
            }
        };
        for channel in 0..channels {
            histogram.clear();
            for u in 0..window_width as isize {
                column_keys(u - left, channel, histogram, true);
            }
            for x in 0..columns {
                if x > 0 {
                    let leaving = x as isize - 1 - left;
                    let entering = leaving + window_width as isize;
                    column_keys(leaving, channel, histogram, false);
                    column_keys(entering, channel, histogram, true);
                }
                let median = histogram.select(rank);
                out_row[x * channels + channel] = (median as f64 / QUANTUM_RANGE) as f32;
            }
        }
    });
    result
}

/// IM `KuwaharaImage(radius, sigma)`.
pub fn kuwahara(image: &Image, radius: f64, sigma: f64) -> Image {
    let quadrant = radius as isize + 1;
    let gaussian = blur(image, radius, sigma);
    let channels = image.channels;
    let width = image.width;
    let luma_at = |x: isize, y: isize| -> f64 {
        let red = gaussian.virtual_get(x, y, 0) as f64;
        if channels >= 3 {
            let green = gaussian.virtual_get(x, y, 1) as f64;
            let blue = gaussian.virtual_get(x, y, 2) as f64;
            return 0.212656f32 as f64 * red + 0.715158f32 as f64 * green + 0.072186f32 as f64 * blue;
        }
        (0.212656f32 as f64 + 0.715158f32 as f64 + 0.072186f32 as f64) * red
    };
    let mut result = image.new_like(width, image.height);
    result.data.par_chunks_mut(width * channels).enumerate().for_each(|(y, out_row)| {
        for x in 0..width {
            let mut min_variance = f64::MAX;
            let mut target = (0isize, 0isize);
            for corner in 0..4 {
                let (start_x, start_y) = match corner {
                    0 => (x as isize - (quadrant - 1), y as isize - (quadrant - 1)),
                    1 => (x as isize, y as isize - (quadrant - 1)),
                    2 => (x as isize - (quadrant - 1), y as isize),
                    _ => (x as isize, y as isize),
                };
                let count = (quadrant * quadrant) as f64;
                let mut means = [0.0f64; 4];
                for v in 0..quadrant {
                    for u in 0..quadrant {
                        for (channel, mean) in means.iter_mut().enumerate().take(channels.min(4)) {
                            *mean += gaussian.virtual_get(start_x + u, start_y + v, channel) as f64;
                        }
                    }
                }
                for mean in means.iter_mut() {
                    *mean /= count;
                }
                let mean_luma = if channels >= 3 {
                    0.212656f32 as f64 * means[0] + 0.715158f32 as f64 * means[1] + 0.072186f32 as f64 * means[2]
                } else {
                    (0.212656f32 as f64 + 0.715158f32 as f64 + 0.072186f32 as f64) * means[0]
                };
                let mut variance = 0.0f64;
                for v in 0..quadrant {
                    for u in 0..quadrant {
                        let luma = luma_at(start_x + u, start_y + v);
                        variance += (luma - mean_luma) * (luma - mean_luma);
                    }
                }
                if variance < min_variance {
                    min_variance = variance;
                    target = (start_x, start_y);
                }
            }
            // IM samples the gaussian at the quadrant centre (bilinear).
            let sample_x = target.0 as f64 + quadrant as f64 / 2.0;
            let sample_y = target.1 as f64 + quadrant as f64 / 2.0;
            for channel in 0..channels {
                out_row[x * channels + channel] = gaussian.interpolate(sample_x, sample_y, channel, crate::image::Interpolate::Bilinear);
            }
        }
    });
    result
}

/// IM `OilPaintImage(radius, sigma)`: most frequent intensity in the neighbourhood.
pub fn oil_paint(image: &Image, radius: f64, _sigma: f64) -> Image {
    const BINS: usize = 256;
    let width_k = (2.0 * radius.ceil() + 1.0) as isize;
    let half = width_k / 2;
    let channels = image.channels;
    let width = image.width;
    let mut result = image.new_like(width, image.height);
    result
        .data
        .par_chunks_mut(width * channels)
        .enumerate()
        .for_each(|(y, out_row)| {
            let mut histogram = [0u32; BINS];
            for x in 0..width {
                histogram.fill(0);
                let mut best_count = 0u32;
                let mut best = (x as isize, y as isize);
                for v in -half..=half {
                    for u in -half..=half {
                        let sx = x as isize + u;
                        let sy = y as isize + v;
                        let intensity = image.virtual_get(sx, sy, 0).clamp(0.0, 1.0);
                        let bin = (intensity * (BINS - 1) as f32) as usize;
                        histogram[bin] += 1;
                        if histogram[bin] > best_count {
                            best_count = histogram[bin];
                            best = (sx, sy);
                        }
                    }
                }
                for channel in 0..channels {
                    out_row[x * channels + channel] = image.virtual_get(best.0, best.1, channel);
                }
            }
        });
    result
}

/// IM `GetPixelLuma` (Rec.709 weights as single-precision constants).
fn pixel_luma(image: &Image, x: isize, y: isize) -> f64 {
    let red = image.virtual_get(x, y, 0) as f64;
    let (green, blue) = if image.channels >= 3 {
        (image.virtual_get(x, y, 1) as f64, image.virtual_get(x, y, 2) as f64)
    } else {
        (red, red)
    };
    0.212656f32 as f64 * red + 0.715158f32 as f64 * green + 0.072186f32 as f64 * blue
}

/// IM's lopsided triangle sum used by `LocalContrastImage`: weights 1..w
/// before the centre, then w+1 down to 3 (2w - 1 taps in total).
fn triangle_sum(samples: &[f32], start: usize, width: usize) -> f32 {
    let mut weight = 1.0f32;
    let mut sum = 0.0f32;
    let mut index = start;
    for _ in 0..width {
        sum += weight * samples[index];
        index += 1;
        weight += 1.0;
    }
    for _ in (width + 1)..(2 * width) {
        sum += weight * samples[index];
        index += 1;
        weight -= 1.0;
    }
    sum
}

/// IM `LocalContrastImage(radius, strength%)` (7.1.0 CPU path).
pub fn local_contrast(image: &Image, radius: f64, strength: f64) -> Image {
    let columns = image.width;
    let rows = image.height;
    let scan_line = columns.max(rows);
    let width = (scan_line as f64 * 0.002f32 as f64 * radius.abs()) as usize;
    let padded_columns = columns + 2 * width;
    let total_weight = ((width + 1) * (width + 1)) as f32;

    // Vertical pass into a horizontally padded buffer.
    let column_sums: Vec<Vec<f32>> = (0..columns)
        .into_par_iter()
        .map(|x| {
            let lumas: Vec<f32> = (0..rows + 2 * width).map(|offset| pixel_luma(image, x as isize, offset as isize - width as isize) as f32).collect();
            (0..rows).map(|y| triangle_sum(&lumas, y, width) / total_weight).collect()
        })
        .collect();
    let mut blurred_rows = vec![0.0f32; rows * padded_columns];
    for (x, sums) in column_sums.iter().enumerate() {
        for (y, &value) in sums.iter().enumerate() {
            let base = y * padded_columns + x + width;
            blurred_rows[base] = value;
            // Mirror into the padding.
            if x <= width && x != 0 {
                blurred_rows[base - x * 2] = value;
            }
            if x as isize > columns as isize - width as isize - 2 && x != columns - 1 {
                blurred_rows[base + (columns - x - 1) * 2] = value;
            }
        }
    }

    let factor = (strength / 100.0) as f32;
    let channels = image.channels;
    let color_channels = if channels >= 3 { 3 } else { 1 };
    let mut result = image.clone();
    result.data.par_chunks_mut(columns * channels).enumerate().for_each(|(y, out_row)| {
        let padded = &blurred_rows[y * padded_columns..(y + 1) * padded_columns];
        for x in 0..columns {
            let sum = triangle_sum(padded, x, width);
            let source = pixel_luma(image, x as isize, y as isize) as f32;
            let mut multiplier = (source - sum / total_weight) * factor;
            multiplier = (source + multiplier) / source;
            for channel in 0..color_channels {
                let index = x * channels + channel;
                out_row[index] = (out_row[index] as f64 * multiplier as f64) as f32;
            }
        }
    });
    result
}

/// IM `StatisticImage(Nonpeak, width, height)`: the window's median, unless
/// it is the smallest distinct value present (then the next distinct value
/// is used) or the largest (then the previous one). Values are keyed to 16
/// bits as in IM's pixel lists.
pub fn nonpeak_filter(image: &Image, width: usize, height: usize) -> Image {
    let window_width = width.max(1);
    let window_height = height.max(1);
    let left = (window_width / 2) as isize;
    let top = (window_height / 2) as isize;
    let rank = window_width * window_height / 2;
    let channels = image.channels;
    let columns = image.width;
    let mut result = image.new_like(columns, image.height);
    result.data.par_chunks_mut(columns * channels).enumerate().for_each(|(y, out_row)| {
        let mut keys = Vec::with_capacity(window_width * window_height);
        for x in 0..columns {
            for channel in 0..channels {
                keys.clear();
                for v in 0..window_height as isize {
                    for u in 0..window_width as isize {
                        let value = image.virtual_get(x as isize - left + u, y as isize - top + v, channel);
                        keys.push(short_key(value));
                    }
                }
                keys.sort_unstable();
                let median = keys[rank];
                let has_smaller = keys[0] < median;
                let has_larger = keys[keys.len() - 1] > median;
                let chosen = if !has_smaller && has_larger {
                    keys[keys.partition_point(|&key| key <= median)]
                } else if has_smaller && !has_larger {
                    keys[keys.partition_point(|&key| key < median) - 1]
                } else {
                    median
                };
                out_row[x * channels + channel] = (chosen as f64 / QUANTUM_RANGE) as f32;
            }
        }
    });
    result
}

/// Magick.NET `ReduceNoise()`, which is `ReduceNoise(3)`: IM
/// `StatisticImage(Nonpeak, 3, 3)`.
pub fn reduce_noise(image: &Image) -> Image {
    nonpeak_filter(image, 3, 3)
}
