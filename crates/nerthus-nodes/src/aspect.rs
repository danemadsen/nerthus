//! Data maps derived from a heightfield: `AspectMaps` (height,
//! slope, angle, curvature, normals, rock and occlusion masks).

use nerthus_engine::{Map, Resolution};
use rayon::prelude::*;

use crate::blur;
use crate::maths;

const DEGREES_PER_RADIAN: f32 = 57.29578;

/// Clamped neighbour indices around (`x`, `y`): left, right, up, down.
fn neighbours(x: usize, y: usize, width: usize, height: usize) -> (usize, usize, usize, usize) {
    let left = if x == 0 { x } else { x - 1 };
    let right = if x == width - 1 { x } else { x + 1 };
    let up = if y == 0 { y } else { y - 1 };
    let down = if y == height - 1 { y } else { y + 1 };
    (left, right, up, down)
}

/// `GetHeight`: 1 between `min` and `max` (fractions of the map's highest
/// value), ramping to 0 over `falloff` on either side.
pub fn height_mask(map: &Map, min: f32, max: f32, falloff: f32) -> Map {
    let (_, highest) = maths::min_max(map.data());
    let upper = highest * max;
    let lower = highest * min;
    let falloff_amount = falloff * highest;
    let falloff_upper = upper + falloff_amount;
    let falloff_lower = lower - falloff_amount;
    map.map_values(|value| {
        let above = value > upper;
        let edge = if above { falloff_upper } else { falloff_lower };
        let bound = if above { upper } else { lower };
        let ramp = 1.0 - (value - bound) / (edge - bound);
        // `Vector.Min(Vector.Max(ramp, 0), 1)`: NaN becomes 0, which
        // `clamp` would keep.
        #[allow(clippy::manual_clamp)]
        let limited = ramp.max(0.0).min(1.0);
        limited
    })
}

/// `GetHeightNormalized`: 1 where the autolevelled height lies between
/// `min` and `max`, ramping linearly to 0 over `falloff` beyond them.
pub fn height_normalized(map: &Map, min: f32, max: f32, falloff: f32) -> Map {
    let leveled = maths::autolevel(map.clone());
    let falloff_max = max + falloff;
    let falloff_min = min - falloff;
    let min_factor = min - falloff_min;
    let max_factor = falloff_max - max;
    leveled.map_values(|value| {
        let mut result = 0.0;
        if value >= min && value <= max {
            result = 1.0;
        }
        if falloff > 0.0 {
            if value > max && value < falloff_max {
                result = 1.0 - (value - max) / max_factor;
            }
            if value < min && value > falloff_min {
                result = (value - falloff_min) / min_factor;
            }
        }
        result
    })
}

/// `GetThreshold`: 1 where a channel is above `threshold`.
pub fn threshold(map: &Map, level: f32) -> Map {
    let channels = (0..map.channel_count())
        .map(|channel| map.channel(channel).par_iter().map(|&value| if value > level { 1.0 } else { 0.0 }).collect())
        .collect();
    Map::from_channels(map.resolution(), channels)
}

/// The window `[min, max]` with linear ramps of width `falloff` outside it.
struct Window {
    min: f32,
    max: f32,
    falloff: f32,
    falloff_min: f32,
    falloff_max: f32,
    min_factor: f32,
    max_factor: f32,
}

impl Window {
    fn new(min: f32, max: f32, falloff: f32) -> Self {
        let falloff_max = max + falloff;
        let falloff_min = min - falloff;
        Self {
            min,
            max,
            falloff,
            falloff_min,
            falloff_max,
            min_factor: min - falloff_min,
            max_factor: falloff_max - max,
        }
    }
}

/// `GetAngle`: a mask of slopes facing `azimuth` degrees.
pub fn angle(heights: &Map, azimuth: f32, min: f32, max: f32, falloff: f32) -> Map {
    let window = Window::new(min / 2.0, max / 2.0, falloff);
    let width = heights.width();
    let data = heights.data();
    let unit = 1.0 / (width as f32 - 1.0);
    let sign = |value: f32| -> f32 {
        if value > 0.0 {
            1.0
        } else if value < 0.0 {
            -1.0
        } else {
            0.0
        }
    };
    let mut result = vec![0.0f32; data.len()];
    result.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
        for (x, output) in row.iter_mut().enumerate() {
            let (left, right, up, down) = neighbours(x, y, width, width);
            let west = data[left + y * width];
            let east = data[right + y * width];
            let north = data[x + up * width];
            let south = data[x + down * width];
            let dx = (east - west) / (2.0 * unit);
            let dy = (south - north) / (2.0 * unit);
            let length = (dx * dx + dy * dy).sqrt();
            let mut degrees = ((0.0 - dy) / length).acos() * DEGREES_PER_RADIAN;
            if degrees.is_infinite() || degrees.is_nan() {
                degrees = 0.0;
            }
            let mut facing = 180.0 * (1.0 + sign(dx)) - sign(dx) * degrees + azimuth;
            facing = (facing * (std::f32::consts::PI / 180.0)).sin();
            facing = facing * 0.5 + 0.5;
            *output = if facing >= window.min && facing <= window.max { 1.0 } else { 0.0 };
            if window.falloff > 0.0 {
                if facing > window.max && facing < window.falloff_max {
                    *output = 1.0 - (facing - window.max) / window.max_factor;
                }
                if facing < window.min && facing > window.falloff_min {
                    *output = (facing - window.falloff_min) / window.min_factor;
                }
            }
        }
    });
    Map::from_data(heights.resolution(), result)
}

/// The eight neighbours of a pixel with clamped edges, as the slope
/// kernels read them: `[nw, n, ne, w, e, sw, s, se]`.
fn ring(data: &[f32], size: usize, x: usize, y: usize) -> [f32; 8] {
    let mut left = x as i64 - 1;
    let mut right = x + 1;
    if left < 0 {
        left = 0;
    } else if right > size - 1 {
        right = size - 1;
    }
    let mut up = y as i64 - 1;
    let mut down = y + 1;
    if up < 0 {
        up = 0;
    } else if down > size - 1 {
        down = size - 1;
    }
    let left = left as usize;
    let up = up as usize;
    let at = |column: usize, row: usize| data[row * size + column];
    [at(left, up), at(x, up), at(right, up), at(left, y), at(right, y), at(left, down), at(x, down), at(right, down)]
}

/// Scharr-like gradient of the ring (`3·corners + 10·edges`).
fn ring_gradient(values: &[f32; 8]) -> (f32, f32) {
    let [north_west, north, north_east, west, east, south_west, south, south_east] = *values;
    let gx = 3.0 * (north_east + south_east - north_west - south_west) + 10.0 * (east - west);
    let gy = 3.0 * (north_west + north_east - south_west - south_east) + 10.0 * (north - south);
    (gx, gy)
}

fn slope_window(degrees: f32, min: f32, max: f32, falloff: f32) -> f32 {
    if degrees >= min && degrees <= max {
        return 1.0;
    }
    if !(falloff > 0.0) {
        return 0.0;
    }
    if !(degrees > max) {
        return 0.0f32.max(1.0 - (min - degrees) / falloff);
    }
    0.0f32.max(1.0 - (degrees - max) / falloff)
}

/// The slope scale for a terrain of `width` by `height` metres sampled at
/// `size` pixels.
fn slope_coefficient(size: usize, terrain_width: f32, terrain_height: f32) -> f32 {
    1.0 / 32.0 * terrain_height / (terrain_width / size as f32)
}

/// `GetSlope`: a mask of slopes between `min` and `max` degrees.
pub fn slope(data: &Map, min: f32, max: f32, falloff: f32, terrain_width: f32, terrain_height: f32) -> Map {
    slope_with_accent(data, 0.0, min, max, falloff, terrain_width, terrain_height, false)
}

/// `GetSlopeX`: [`slope`] with a local-relief term added ("micro accent").
pub fn slope_accented(data: &Map, micro_accent: f32, min: f32, max: f32, falloff: f32, terrain_width: f32, terrain_height: f32) -> Map {
    slope_with_accent(data, micro_accent, min, max, falloff, terrain_width, terrain_height, true)
}

#[allow(clippy::too_many_arguments)]
fn slope_with_accent(data: &Map, micro_accent: f32, min: f32, max: f32, falloff: f32, terrain_width: f32, terrain_height: f32, accented: bool) -> Map {
    let size = data.width();
    let values = data.data();
    let coefficient = slope_coefficient(size, terrain_width, terrain_height);
    let mut result = vec![0.0f32; values.len()];
    result.par_chunks_mut(size).enumerate().for_each(|(y, row)| {
        for (x, output) in row.iter_mut().enumerate() {
            let samples = ring(values, size, x, y);
            let (gx, gy) = ring_gradient(&samples);
            let mut steepness = (gx * gx + gy * gy).sqrt();
            steepness *= coefficient;
            if accented {
                let [north_west, north, north_east, west, east, south_west, south, south_east] = samples;
                let center = values[y * size + x];
                let mut relief = center - 0.14644662 * (west + north + south + east) - 0.10355339 * (north_west + north_east + south_west + south_east);
                relief *= 32.0 * coefficient;
                steepness += relief * micro_accent;
            }
            let degrees = steepness.atan() * DEGREES_PER_RADIAN;
            *output = slope_window(degrees, min, max, falloff);
        }
    });
    Map::from_data(data.resolution(), result)
}

/// `GetSlopeNormalized`: slope as `g / sqrt(1 + g²)` against a 0..1 window.
pub fn slope_normalized(heights: &Map, min: f32, max: f32, falloff: f32) -> Map {
    let width = heights.width();
    let height = heights.height();
    let window = Window::new(min, max, falloff);
    let scale_x = 0.3125 * width as f32;
    let scale_y = 0.3125 * height as f32;
    let data = heights.data();
    let mut result = vec![0.0f32; data.len()];
    for y in 0..width {
        for x in 0..width {
            let mut left = x as i64 - 1;
            let mut right = x + 1;
            if left < 0 {
                left = 0;
            } else if right > width - 1 {
                right = width - 1;
            }
            let mut up = y as i64 - 1;
            let mut down = y + 1;
            if up < 0 {
                up = 0;
            } else if down > height - 1 {
                down = height - 1;
            }
            let (left, up) = (left as usize, up as usize);
            let at = |column: usize, row: usize| data[row * width + column];
            let samples = [at(left, up), at(x, up), at(right, up), at(left, y), at(right, y), at(left, down), at(x, down), at(right, down)];
            let (gx, gy) = ring_gradient(&samples);
            let scaled_x = gx * scale_x;
            let scaled_y = gy * scale_y;
            let gradient = (scaled_x * scaled_x + scaled_y * scaled_y).sqrt();
            let value = gradient / (1.0 + gradient * gradient).sqrt();
            let index = y * width + x;
            result[index] = if value >= window.min && value <= window.max {
                1.0
            } else if falloff > 0.0 {
                if value > window.max && value < window.falloff_max {
                    1.0 - (value - window.max) / window.max_factor
                } else if value < window.min && value > window.falloff_min {
                    (value - window.falloff_min) / window.min_factor
                } else {
                    0.0
                }
            } else {
                0.0
            };
        }
    }
    Map::from_data(heights.resolution(), result)
}

/// `Handedness` of a normal map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handedness {
    Left,
    Right,
}

/// `UpAxis` of a normal map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpAxis {
    ZUp,
    YUp,
}

/// `GetNormalMap`: tangent-free normals packed into RGB.
pub fn normal_map(data: &Map, width: f32, height: f32, handedness: Handedness, up_axis: UpAxis, terrain_width: f32, terrain_height: f32) -> Map {
    let size = data.width();
    let values = data.data();
    let coefficient_x = terrain_height * width;
    let mut coefficient_y = terrain_height * width;
    let mut coefficient_z = height * 32.0 * terrain_width / size as f32;
    if up_axis == UpAxis::ZUp && handedness == Handedness::Right {
        coefficient_y = 0.0 - coefficient_y;
    } else if up_axis == UpAxis::YUp && handedness == Handedness::Right {
        coefficient_z = 0.0 - coefficient_z;
    }

    let mut channels = vec![vec![0.0f32; values.len()]; 3];
    let rows: Vec<Vec<[f32; 3]>> = (0..size)
        .into_par_iter()
        .map(|y| {
            (0..size)
                .map(|x| {
                    let left = x.saturating_sub(1);
                    let right = (x + 1).min(size - 1);
                    let up = y.saturating_sub(1);
                    let down = (y + 1).min(size - 1);
                    let at = |column: usize, row: usize| values[row * size + column];
                    let north_west = at(left, up);
                    let north = at(x, up);
                    let north_east = at(right, up);
                    let west = at(left, y);
                    let east = at(right, y);
                    let south_west = at(left, down);
                    let south = at(x, down);
                    let south_east = at(right, down);
                    let mut normal_x = 0.0 - (3.0 * (north_east + south_east - north_west - south_west) + 10.0 * (east - west));
                    let mut normal_y = 3.0 * (north_west + north_east - south_west - south_east) + 10.0 * (north - south);
                    let mut normal_z = 1.0f32;
                    normal_x *= coefficient_x;
                    normal_y *= coefficient_y;
                    normal_z *= coefficient_z;
                    let length = (normal_x * normal_x + normal_y * normal_y + normal_z * normal_z).sqrt();
                    let unit = [normal_x / length, normal_y / length, normal_z / length];
                    match (up_axis, handedness) {
                        (UpAxis::ZUp, _) => [unit[0] * 0.5 + 0.5, unit[1] * 0.5 + 0.5, unit[2] * 0.5 + 0.5],
                        (UpAxis::YUp, Handedness::Left) => [unit[0] * 0.5 + 0.5, unit[2] * 0.5 + 0.5, unit[1] * 0.5 + 0.5],
                        (UpAxis::YUp, Handedness::Right) => [unit[0] * 0.5 + 0.5, (0.0 - unit[2]) * 0.5 + 0.5, unit[1] * 0.5 + 0.5],
                    }
                })
                .collect()
        })
        .collect();
    for (y, row) in rows.iter().enumerate() {
        for (x, rgb) in row.iter().enumerate() {
            for channel in 0..3 {
                channels[channel][y * size + x] = rgb[channel];
            }
        }
    }
    Map::from_channels(data.resolution(), channels)
}

/// `CurvatureType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurvatureType {
    Horizontal,
    Vertical,
    Average,
}

fn limit_curvature(mut value: f32) -> f32 {
    if value.is_infinite() || value.is_nan() {
        value = 0.0;
    }
    value = value.clamp(-10000.0, 10000.0);
    value /= 10000.0;
    value * 0.5 + 0.5
}

/// Plan curvature, scaled into 0..1 around 0.5.
pub fn horizontal_curvature(dx: f32, dy: f32, dxx: f32, dyy: f32, dxy: f32) -> f32 {
    let mut value = -2.0 * (dy * dy * dxx + dx * dx * dyy - dx * dy * dxy);
    value /= dx * dx + dy * dy;
    limit_curvature(value)
}

/// Profile curvature, scaled into 0..1 around 0.5.
pub fn vertical_curvature(dx: f32, dy: f32, dxx: f32, dyy: f32, dxy: f32) -> f32 {
    let mut value = -2.0 * (dx * dx * dxx + dy * dy * dyy + dx * dy * dxy);
    value /= dx * dx + dy * dy;
    limit_curvature(value)
}

/// `GetCurvature`: the falloff ramps around a curvature window.
pub fn curvature(map: &Map, min: f32, max: f32, falloff: f32, kind: CurvatureType) -> Map {
    let window = Window::new(min, max / 2.0, falloff);
    let width = map.width();
    let height = map.height();
    let data = map.data();
    let unit_x = 1.0 / (width as f32 - 1.0);
    let unit_y = 1.0 / (height as f32 - 1.0);
    let mut result = vec![0.0f32; data.len()];
    result.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
        for (x, output) in row.iter_mut().enumerate() {
            let (left, right, up, down) = neighbours(x, y, width, height);
            let center = data[x + y * width];
            let west = data[left + y * width];
            let east = data[right + y * width];
            let north = data[x + up * width];
            let south = data[x + down * width];
            let north_west = data[left + up * width];
            let south_west = data[left + down * width];
            let north_east = data[right + up * width];
            let south_east = data[right + down * width];
            let dx = (east - west) / (2.0 * unit_x);
            let dy = (south - north) / (2.0 * unit_y);
            let dxx = (east - 2.0 * center + west) / (unit_x * unit_x);
            let dyy = (south - 2.0 * center + north) / (unit_y * unit_y);
            let dxy = (south_east - north_east - south_west + north_west) / (4.0 * unit_x * unit_y);
            let value = match kind {
                CurvatureType::Horizontal => horizontal_curvature(dx, dy, dxx, dyy, dxy),
                CurvatureType::Vertical => vertical_curvature(dx, dy, dxx, dyy, dxy),
                CurvatureType::Average => {
                    let horizontal = horizontal_curvature(dx, dy, dxx, dyy, dxy);
                    let vertical = vertical_curvature(dx, dy, dxx, dyy, dxy);
                    (horizontal + vertical) * 0.5
                }
            };
            if value >= window.min && value <= window.max {
                *output = 0.0;
            }
            if window.falloff > 0.0 {
                if value > window.max && value < window.falloff_max {
                    *output = (value - window.max) / window.max_factor;
                }
                if value < window.min && value > window.falloff_min {
                    *output = 1.0 - (value - window.falloff_min) / window.min_factor;
                }
            }
        }
    });
    Map::from_data(map.resolution(), result)
}

fn safe_div(numerator: f32, denominator: f32) -> f32 {
    if denominator.abs() < 1e-18 {
        return 0.0;
    }
    numerator / denominator
}

fn safe_sqrt(value: f32) -> f32 {
    if value <= 0.0 {
        return 0.0;
    }
    value.sqrt()
}

fn mean_curvature(zx: f32, zy: f32, zxx: f32, zyy: f32, zxy: f32) -> f32 {
    let zx_squared = zx * zx;
    let zy_squared = zy * zy;
    let sum = zx_squared + zy_squared;
    let numerator = (1.0 + zy_squared) * zxx - 2.0 * zxy * zx * zy + (1.0 + zx_squared) * zyy;
    let denominator = 2.0 * maths::pow(sum + 1.0, 1.5);
    safe_div(numerator, denominator)
}

fn gaussian_curvature(zx: f32, zy: f32, zxx: f32, zyy: f32, zxy: f32) -> f32 {
    let sum = zx * zx + zy * zy;
    let numerator = zxx * zyy - zxy * zxy;
    let denominator = maths::pow(sum + 1.0, 2.0);
    safe_div(numerator, denominator)
}

fn minimal_curvature(zx: f32, zy: f32, zxx: f32, zyy: f32, zxy: f32) -> f32 {
    let mean = mean_curvature(zx, zy, zxx, zyy, zxy);
    let gaussian = gaussian_curvature(zx, zy, zxx, zyy, zxy);
    mean - safe_sqrt(mean * mean - gaussian)
}

/// `AspectMaps.RockMap`: exposed rock where the minimal curvature is small.
pub fn rock_map(heights: &Map, coverage: f32, density: f32, terrain_height: f32) -> Map {
    let size = heights.width();
    let data = heights.data();
    let falloff = coverage * 0.1;
    let falloff_max = 0.025 + falloff;
    let falloff_min = 0.0 - falloff;
    let min_factor = 0.0 - falloff_min;
    let max_factor = falloff_max - 0.025;
    let mut result = vec![1.0f32; data.len()];
    let spacing = 5.0f32;
    let spacing_squared = spacing * spacing;
    result.par_chunks_mut(size).enumerate().for_each(|(y, row)| {
        if y == 0 || y >= size - 1 {
            return;
        }
        for (x, output) in row.iter_mut().enumerate().take(size - 1).skip(1) {
            let height_at = |column: usize, row: usize| data[row * size + column] * terrain_height;
            let north_west = height_at(x - 1, y + 1);
            let north = height_at(x, y + 1);
            let north_east = height_at(x + 1, y + 1);
            let west = height_at(x - 1, y);
            let center = height_at(x, y);
            let east = height_at(x + 1, y);
            let south_west = height_at(x - 1, y - 1);
            let south = height_at(x, y - 1);
            let south_east = height_at(x + 1, y - 1);
            let zx = (north_east + east + south_east - north_west - west - south_west) / (6.0 * spacing);
            let zy = (north_west + north + north_east - south_west - south - south_east) / (6.0 * spacing);
            let zxx = (north_west + north_east + west + east + south_west + south_east - 2.0 * (north + center + south)) / (3.0 * spacing_squared);
            let zyy = (north_west + north + north_east + south_west + south + south_east - 2.0 * (west + center + east)) / (3.0 * spacing_squared);
            let zxy = (north_east + south_west - north_west - south_east) / (4.0 * spacing_squared);
            let curvature = minimal_curvature(0.0 - zx, 0.0 - zy, 0.0 - zxx, 0.0 - zyy, 0.0 - zxy);
            if (0.0..=0.025).contains(&curvature) {
                *output = 1.0;
            }
            if !(falloff > 0.0) {
                continue;
            }
            if curvature > 0.025 {
                if curvature < falloff_max {
                    *output = 1.0 - (curvature - 0.025) / max_factor;
                }
            } else if curvature < 0.0 && curvature > falloff_min {
                *output = (curvature - falloff_min) / min_factor;
            }
        }
    });
    let mut map = Map::from_data(heights.resolution(), result);
    if density as f64 != 0.0 {
        maths::strengthen(&mut map, density);
    }
    map
}

/// `AspectMaps.Occlusion`: a blurred-minus-original cavity estimate,
/// leveled and (for current nodes) inverted.
pub fn occlusion(map: &Map, strength: f32, octaves: i32, invert: bool) -> Map {
    let size = map.width();
    let blurred = blur::fixed_box_blur(map.data(), size, strength, octaves.max(0) as usize);
    let blurred_map = Map::from_data(Resolution::square(size), blurred);
    let cavity = blurred_map.subtract(map);
    let mut leveled = maths::autolevel(cavity);
    if invert {
        maths::invert(&mut leveled);
    }
    leveled
}

/// `AspectMaps.GetSlopeNormalizedOld`: steepness from central differences
/// over a map one unit wide, as `s / sqrt(1 + s²)`, 1 inside `[min, max]`
/// with linear falloffs `falloff` wide.
pub fn slope_normalized_old(heights: &Map, min: f32, max: f32, falloff: f32) -> Map {
    let size = heights.width();
    let falloff_max = max + falloff;
    let falloff_min = min - falloff;
    let min_factor = min - falloff_min;
    let max_factor = falloff_max - max;
    let spacing = 1.0 / (size as f32 - 1.0);
    let data = heights.data();
    let mut result = vec![0.0f32; size * size];
    result.par_chunks_mut(size).enumerate().for_each(|(row, values)| {
        let below = if row == size - 1 { row } else { row + 1 };
        let above = if row == 0 { row } else { row - 1 };
        for (column, value) in values.iter_mut().enumerate() {
            let right = if column == size - 1 { column } else { column + 1 };
            let left = if column == 0 { column } else { column - 1 };
            let west = data[row * size + left];
            let east = data[row * size + right];
            let north = data[above * size + column];
            let south = data[below * size + column];
            let dx = (east - west) / (2.0 * spacing);
            let dy = (south - north) / (2.0 * spacing);
            let gradient = (dx * dx + dy * dy).sqrt();
            let steepness = gradient / (1.0 + gradient * gradient).sqrt();
            *value = if steepness >= min && steepness <= max {
                1.0
            } else if falloff > 0.0 && steepness > max && steepness < falloff_max {
                1.0 - (steepness - max) / max_factor
            } else if falloff > 0.0 && steepness < min && steepness > falloff_min {
                (steepness - falloff_min) / min_factor
            } else {
                0.0
            };
        }
    });
    Map::from_data(heights.resolution(), result)
}
