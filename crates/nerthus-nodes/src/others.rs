//! `Others`: small image effects
//! (oil paint, median and mean blurs, pixelate, hard denoise, swirl).

use std::cmp::Ordering;

use nerthus_engine::Map;
use rayon::prelude::*;

use crate::maths;

/// .NET's float ordering for `Array.Sort`: NaN first, then by value
/// (signed zeros compare equal).
fn dotnet_float_order(a: &f32, b: &f32) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
    }
}

/// `float.Equals`, which treats NaN as equal to itself.
fn dotnet_float_equals(a: f32, b: f32) -> bool {
    a == b || (a.is_nan() && b.is_nan())
}

/// `Others.ApplyOilPaintEffect`: every pixel takes the most common value in
/// its `brush_size` neighbourhood (the first one seen on ties).
pub fn oil_paint(heightfield: &Map, brush_size: i32) -> Map {
    let size = heightfield.width() as i32;
    let data = heightfield.data();
    let mut result = vec![0.0f32; data.len()];
    result.par_chunks_mut(size as usize).enumerate().for_each(|(row, values)| {
        let y = row as i32;
        let mut counts: Vec<(f32, u32)> = Vec::new();
        for (column, value) in values.iter_mut().enumerate() {
            let x = column as i32;
            counts.clear();
            for offset_y in -brush_size..=brush_size {
                for offset_x in -brush_size..=brush_size {
                    let sample_x = x + offset_x;
                    let sample_y = y + offset_y;
                    let inside = sample_x >= 0 && sample_x < size && sample_y >= 0 && sample_y < size;
                    if !inside {
                        continue;
                    }
                    let key = data[(sample_x + sample_y * size) as usize];
                    match counts.iter_mut().find(|(candidate, _)| dotnet_float_equals(*candidate, key)) {
                        Some((_, count)) => *count += 1,
                        None => counts.push((key, 1)),
                    }
                }
            }
            let mut best_count = 0.0f32;
            let mut best_value = 0.0f32;
            for &(key, count) in &counts {
                if count as f32 > best_count {
                    best_count = count as f32;
                    best_value = key;
                }
            }
            *value = best_value;
        }
    });
    Map::from_data(heightfield.resolution(), result)
}

/// Half sizes of the median and mean windows, scaled from a 512 map.
fn window_half_sizes(size: i32, size2: i32, width: i32) -> (i32, i32) {
    let scale = width / 512;
    (size / 2 * scale, size2 / 2 * scale)
}

/// `Others.SmoothedMedianBlur`: the average of the two values either side of
/// the median of each window. The window buffer is reused along a row and
/// sorted whole, so near the borders stale values from the previous pixel
/// take part. Below 512 pixels the window is a single pixel, which has no
/// neighbours to average and fails (`None`).
pub fn smoothed_median_blur(heightfield: &Map, size: i32, size2: i32, width: i32) -> Option<Map> {
    let (half, half2) = window_half_sizes(size, size2, width);
    if (2 * half + 1) * (2 * half2 + 1) < 3 {
        return None;
    }
    let data = heightfield.data();
    let source_width = heightfield.width() as i32;
    let window_length = ((2 * half + 1) * (2 * half2 + 1)) as usize;
    let mut result = vec![0.0f32; (width * width) as usize];
    result.par_chunks_mut(width as usize).enumerate().for_each(|(row, values)| {
        let y = row as i32;
        let mut window = vec![0.0f32; window_length];
        for (column, value) in values.iter_mut().enumerate() {
            let x = column as i32;
            let mut filled = 0;
            for offset_y in -half..=half {
                for offset_x in -half2..=half2 {
                    let sample_x = x + offset_x;
                    let sample_y = y + offset_y;
                    let inside = sample_x >= 0 && sample_x < width && sample_y >= 0 && sample_y < width;
                    if inside {
                        window[filled] = data[(sample_x + sample_y * source_width) as usize];
                        filled += 1;
                    }
                }
            }
            window.sort_unstable_by(dotnet_float_order);
            let middle = window_length / 2;
            *value = (window[middle - 1] + window[middle + 1]) * 0.5;
        }
    });
    let resolution = nerthus_engine::Resolution::square(width as usize);
    Some(Map::from_data(resolution, result))
}

/// `Others.MedianBlur` (a mean, despite the name): the window sum over the
/// full window size, so borders darken.
pub fn mean_blur(heightfield: &Map, size: i32, size2: i32, width: i32) -> Map {
    let (half, half2) = window_half_sizes(size, size2, width);
    let data = heightfield.data();
    let source_width = heightfield.width() as i32;
    let divisor = ((half * 2 + 1) * (half2 * 2 + 1)) as f32;
    let mut result = vec![0.0f32; (width * width) as usize];
    result.par_chunks_mut(width as usize).enumerate().for_each(|(row, values)| {
        let y = row as i32;
        for (column, value) in values.iter_mut().enumerate() {
            let x = column as i32;
            let mut sum = 0.0f32;
            for offset_y in -half..=half {
                for offset_x in -half2..=half2 {
                    let sample_x = x + offset_x;
                    let sample_y = y + offset_y;
                    let inside = sample_x >= 0 && sample_x < width && sample_y >= 0 && sample_y < width;
                    if inside {
                        sum += data[(sample_x + sample_y * source_width) as usize];
                    }
                }
            }
            *value = sum / divisor;
        }
    });
    Map::from_data(nerthus_engine::Resolution::square(width as usize), result)
}

/// `Others.PixelateHeightfield`: square cells `pixel_size_ratio` of the map
/// wide, each the cell average (`anti_alias`) or its centre value.
pub fn pixelate(original: &Map, pixel_size_ratio: f32, anti_alias: bool) -> Map {
    let size = original.width();
    let data = original.data();
    let cell = ((pixel_size_ratio * size as f32) as i32).max(1) as usize;
    let mut result = vec![0.0f32; size * size];
    for row in (0..size).step_by(cell) {
        for column in (0..size).step_by(cell) {
            let row_end = (row + cell).min(size);
            let column_end = (column + cell).min(size);
            let value = if anti_alias {
                let mut sum = 0.0f32;
                let mut count = 0;
                for sample_row in row..row_end {
                    for sample_column in column..column_end {
                        sum += data[sample_row * size + sample_column];
                        count += 1;
                    }
                }
                sum / count as f32
            } else {
                let centre_row = (row + cell / 2).min(size - 1);
                let centre_column = (column + cell / 2).min(size - 1);
                data[centre_row * size + centre_column]
            };
            for target_row in row..row_end {
                for target_column in column..column_end {
                    result[target_row * size + target_column] = value;
                }
            }
        }
    }
    Map::from_data(original.resolution(), result)
}

/// `Others.HardDenoise`: replaces isolated spikes with the mean of their four
/// neighbours, in place. Rows run in parallel over the same buffer would
/// see their neighbours' replacements or not by chance; this runs them in
/// order, which is what each worker does within its own rows.
pub fn hard_denoise(map: &Map) -> Map {
    let mut result = map.clone();
    let stride = map.width();
    let data = result.data_mut();
    for y in 1..stride.saturating_sub(1) {
        for x in 1..stride - 1 {
            let left = data[y * stride + x - 1];
            let right = data[y * stride + x + 1];
            let above = data[(y - 1) * stride + x];
            let below = data[(y + 1) * stride + x];
            let centre = data[y * stride + x];
            let mean = (left + right + above + below) * 0.25;
            let spike = centre > mean * 2.0 && left <= mean;
            if spike || right <= mean || above <= mean || below <= mean {
                data[y * stride + x] = mean;
            }
        }
    }
    result
}

/// `Others.MirrorCoordinate`.
fn mirror_coordinate(coordinate: f32, dimension: usize) -> f32 {
    let limit = dimension as f32;
    let mut mirrored = coordinate;
    if mirrored < 0.0 {
        mirrored = 0.0 - mirrored;
    } else if mirrored >= limit {
        mirrored = (2 * dimension) as f32 - mirrored - 1.0;
    }
    let upper = maths::dotnet_min((dimension - 1) as f32, mirrored);
    maths::dotnet_max(0.0, upper)
}

/// `Others.InterpolateHeight`: bilinear sample with mirrored edges.
fn interpolate_height(heightfield: &[f32], dimension: usize, x: f32, y: f32) -> f32 {
    let x = mirror_coordinate(x, dimension);
    let y = mirror_coordinate(y, dimension);
    let left = x as usize;
    let top = y as usize;
    let right = (left + 1).min(dimension - 1);
    let bottom = (top + 1).min(dimension - 1);
    let fraction_x = x - left as f32;
    let fraction_y = y - top as f32;
    let top_left = heightfield[top * dimension + left];
    let top_right = heightfield[top * dimension + right];
    let bottom_left = heightfield[bottom * dimension + left];
    let bottom_right = heightfield[bottom * dimension + right];
    let upper = top_left + (top_right - top_left) * fraction_x;
    let lower = bottom_left + (bottom_right - bottom_left) * fraction_x;
    upper + (lower - upper) * fraction_y
}

/// `Others.SwirlHeightfield`: rotates each pixel around `center` (0..1) by
/// an angle growing with its distance.
pub fn swirl(original: &Map, swirl_amount: f32, center: [f32; 2]) -> Map {
    let size = original.width();
    let data = original.data();
    let center_x = center[0] * size as f32;
    let center_y = center[1] * size as f32;
    let mut result = vec![0.0f32; size * size];
    result.par_chunks_mut(size).enumerate().for_each(|(row, values)| {
        for (column, value) in values.iter_mut().enumerate() {
            let dx = column as f32 - center_x;
            let dy = row as f32 - center_y;
            let distance = (dx * dx + dy * dy).sqrt();
            let angle = swirl_amount * distance;
            let (sine, cosine) = maths::sin_cos(angle);
            let source_x = cosine * dx - sine * dy + center_x;
            let source_y = sine * dx + cosine * dy + center_y;
            *value = interpolate_height(data, size, source_x, source_y);
        }
    });
    Map::from_data(original.resolution(), result)
}
