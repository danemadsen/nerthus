//! Height profile operations from the `Profiles` library.

use nerthus_engine::Map;
use nerthus_erosion::ts;
use rayon::prelude::*;

use crate::blur;
use crate::classic_combiner;
use crate::fast_blur;
use crate::maths;
use crate::mathv;
use crate::random::XorShift64Star;

/// `Profiles.Terrace`: snaps heights towards `count` jittered steps.
///
/// * `uniformity` keeps the steps evenly spaced (1 = even).
/// * `steepness` sharpens each riser (0 = smoothstep).
/// * `intensity` blends the terraced heights with the input.
/// * `force_zero` adds a near-zero first step.
pub fn terrace(source: &Map, count: usize, uniformity: f32, steepness: f32, intensity: f32, seed: i32, force_zero: bool) -> Map {
    let count = count.max(2);
    let length = if force_zero { count + 1 } else { count };
    let mut steps = vec![0.0f32; length];
    let mut random = XorShift64Star::new(seed);
    let spacing = 1.0 / (count as f32 - 1.0);
    if force_zero {
        steps[1] = 0.001;
    }
    let first = if force_zero { 2 } else { 1 };
    for index in first..count {
        steps[index] = steps[index - 1] + spacing;
    }
    for _ in 0..10 {
        for index in 1..count - 1 {
            let span = steps[index + 1] - steps[index - 1];
            let jitter = random.next_float();
            let candidate = steps[index - 1] + span * jitter;
            steps[index] = steps[index] * uniformity + candidate * (1.0 - uniformity);
        }
    }

    let exponent = 1.0 - steepness;
    let terraced: Vec<f32> = source
        .data()
        .par_iter()
        .map(|&value| {
            if value >= 0.999 {
                return value;
            }
            let mut segment = 0usize;
            for _ in 0..count - 1 {
                if steps[segment + 1] > value {
                    break;
                }
                if segment + 1 == count {
                    break;
                }
                segment += 1;
            }
            let span = steps[segment + 1] - steps[segment];
            let t = (value - steps[segment]) / span;
            let mut shaped = 3.0 * t * t - 2.0 * t * t * t;
            shaped = (shaped - 0.5) * 2.0;
            shaped = shaped / 2.0 + 0.5;
            shaped = maths::pow(shaped, exponent);
            (steps[segment] * (1.0 - shaped) + steps[segment + 1] * shaped) * intensity + value * (1.0 - intensity)
        })
        .collect();
    Map::from_data(maths::square_resolution(terraced.len()), terraced)
}

/// `Profiles.Shaper`: an exponential reshape of the height profile,
/// optionally only of the local relief (`local_effect`) and keeping fine
/// detail out of the reshape (`fine_details`).
pub fn shaper(map: &Map, shape: f32, local_effect: bool, local_area: f32, fine_details: bool, detail_size: f32) -> Map {
    let factor = shape * -10.0;
    if factor == 0.0 {
        return map.clone();
    }
    let size = map.width();
    let mut values = map.data().to_vec();
    let mut detail = Vec::new();
    let mut original_range = (0.0, 0.0);
    if fine_details || local_effect {
        original_range = maths::min_max(&values);
    }

    if fine_details {
        let radius = size as f32 * (0.0005 + detail_size * 0.01);
        let smooth = blur::box_blur(&values, size, radius, 5);
        detail = subtract(&values, &smooth);
        values = smooth;
    }
    if local_effect {
        let radius = size as f32 * 0.08 * (local_area + 0.1);
        let base = blur::box_blur(&values, size, radius, 5);
        let mut relief = subtract(&values, &base);
        maths::exponential_shape(&mut relief, factor);
        values = add(&base, &relief);
    } else {
        maths::exponential_shape(&mut values, factor);
    }
    if fine_details {
        values = add(&values, &detail);
    }
    if fine_details || local_effect {
        let (minimum, maximum) = maths::min_max(&values);
        let (original_minimum, original_maximum) = original_range;
        maths::remap_plane(&mut values, maximum, minimum, original_maximum, original_minimum);
    }
    Map::from_data(maths::square_resolution(values.len()), values)
}

fn subtract(a: &[f32], b: &[f32]) -> Vec<f32> {
    a.par_iter().zip(b.par_iter()).map(|(&x, &y)| x - y).collect()
}

fn add(a: &[f32], b: &[f32]) -> Vec<f32> {
    a.par_iter().zip(b.par_iter()).map(|(&x, &y)| x + y).collect()
}

/// `Profiles.GraphicEQ`: splits the map's first channel into seven bands,
/// from the blur at a tenth of the width down to the finest detail, and
/// sums them back weighted by `gains`.
pub fn graphic_eq(map: &Map, gains: [f32; 7]) -> Map {
    let resolution = map.resolution();
    let first_channel = Map::from_data(resolution, map.data().to_vec());
    let source = first_channel.data();
    let ratio = maths::ln(52.0) / maths::ln(5.0);
    let mut divisor = 10.0f32;
    let mut result = vec![0.0f32; source.len()];
    let mut band_map = first_channel.clone();
    let mut previous = band_map.clone();
    for (band, &gain) in gains.iter().enumerate() {
        let radius = resolution.x as f32 / divisor;
        if band < 6 {
            band_map = fast_blur::blur_map(&first_channel, radius);
        }
        let current = band_map.data();
        let earlier = previous.data();
        result.par_iter_mut().enumerate().for_each(|(index, value)| {
            let band_value = if band >= 6 {
                source[index] - earlier[index]
            } else if band != 0 {
                current[index] - earlier[index]
            } else {
                current[index]
            };
            *value += band_value * gain;
        });
        previous = band_map.clone();
        divisor *= ratio;
    }
    Map::from_data(resolution, result)
}

/// `FoldWaveform`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldWaveform {
    Sine,
    Triangle,
    Sawtooth,
}

impl FoldWaveform {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Triangle" => Self::Triangle,
            "Sawtooth" => Self::Sawtooth,
            _ => Self::Sine,
        }
    }
}

fn fract(value: f32) -> f32 {
    value - value.floor()
}

/// One folded value. The mask is multiplied in at different places per
/// waveform, so each order is kept.
fn fold_value(waveform: FoldWaveform, height: f32, mask: Option<f32>, folds: f32, symmetric: bool) -> f32 {
    let base = if symmetric { height - 0.5 } else { height };
    let two_pi = std::f32::consts::PI * 2.0;
    match waveform {
        FoldWaveform::Sine => {
            let phase = match mask {
                Some(mask) => base * mask * folds * two_pi,
                None => base * folds * two_pi,
            };
            mathv::sin(phase) * 0.5 + 0.5
        }
        FoldWaveform::Triangle => {
            let position = match mask {
                Some(mask) => base * folds * mask + 0.25,
                None => base * folds + 0.25,
            };
            1.0 - (fract(position) * 2.0 - 1.0).abs()
        }
        FoldWaveform::Sawtooth => {
            let position = match mask {
                Some(mask) => base * folds * mask + 0.5,
                None => base * folds + 0.5,
            };
            fract(position)
        }
    }
}

/// `Profiles.Sine/Triangle/Sawtooth`: folds the heights `folds` times,
/// scaled per pixel by `mask` when given.
pub fn fold(map: &Map, mask: Option<&Map>, waveform: FoldWaveform, folds: f32, symmetric: bool) -> Map {
    match mask {
        Some(mask) => map.zip_values(mask, |height, mask| fold_value(waveform, height, Some(mask), folds, symmetric)),
        None => map.map_values(|height| fold_value(waveform, height, None, folds, symmetric)),
    }
}

/// `Profiles.RecurveInternal`: `duration` passes that pull each height
/// towards its downhill (or, with `inflate`, uphill) neighbour.
fn recurve_passes(heights: Map, duration: i32, power: f32, scale: f32, inflate: bool, factor: i32) -> Map {
    let size = heights.width();
    let mut current = heights.into_data();
    let mut next = vec![0.0f32; size * size];
    let flatness = scale * factor as f32;
    for _ in 0..duration {
        let source = &current;
        let get = |x: usize, y: usize| source[y * size + x];
        next.par_chunks_mut(size).enumerate().skip(1).take(size.saturating_sub(2)).for_each(|(row, values)| {
            for (column, value) in values.iter_mut().enumerate().take(size - 1).skip(1) {
                let below_difference = get(column - 1, row + 1) - get(column + 1, row + 1);
                let middle_difference = get(column - 1, row) - get(column + 1, row);
                let above_difference = get(column - 1, row - 1) - get(column + 1, row - 1);
                let left_difference = get(column - 1, row + 1) - get(column - 1, row - 1);
                let centre_difference = get(column, row + 1) - get(column, row - 1);
                let right_difference = get(column + 1, row + 1) - get(column + 1, row - 1);
                let gradient_x = (below_difference + 2.0 * middle_difference + above_difference) * 0.25;
                let gradient_y = (0.0 - (left_difference + 2.0 * centre_difference + right_difference)) * 0.25;
                let length = (gradient_x * gradient_x + gradient_y * gradient_y + flatness).sqrt();
                let direction_x = gradient_x / length;
                let direction_y = gradient_y / length;
                let weight_x = direction_x.abs();
                let weight_y = direction_y.abs();
                let keep_x = 1.0 - weight_x;
                let keep_y = 1.0 - weight_y;
                let (neighbour_x, neighbour_y) = if inflate {
                    let x = if direction_x > 0.0 { column - 1 } else { column + 1 };
                    let y = if direction_y > 0.0 { row - 1 } else { row + 1 };
                    (x, y)
                } else {
                    let x = if direction_x < 0.0 { column - 1 } else { column + 1 };
                    let y = if direction_y < 0.0 { row - 1 } else { row + 1 };
                    (x, y)
                };
                let here = get(column, row);
                let vertical = get(column, neighbour_y);
                let horizontal = get(neighbour_x, row);
                let diagonal = get(neighbour_x, neighbour_y);
                let along_row = keep_x * here + weight_x * horizontal;
                let along_next_row = keep_x * vertical + weight_x * diagonal;
                let target = keep_y * along_row + weight_y * along_next_row;
                let erosion = power * (here - target);
                *value = here - erosion;
            }
        });
        for index in 0..size {
            next[index] = next[size + index];
            next[(size - 1) * size + index] = next[(size - 2) * size + index];
            next[index * size] = next[index * size + 1];
            next[index * size + size - 1] = next[index * size + size - 2];
        }
        current = next.clone();
    }
    Map::from_data(maths::square_resolution(current.len()), current)
}

/// `Profiles.Recurve` with `preserve` (the node's only use): works at 1024
/// in 16-bit units, then resizes back.
pub fn recurve(map: &Map, power: f32, duration: i32, inflate: bool, preserve: bool, scale: f32) -> Map {
    let size = map.width();
    let mut duration = duration;
    let mut pass_factor = 600_000;
    let working = if preserve {
        crate::transformer::resize(map, nerthus_engine::Resolution::square(1024))
    } else {
        let resolution_scale = size as f32 / 1024.0;
        duration = 1.max((duration as f32 * resolution_scale) as i32);
        pass_factor = match size {
            512 => 4_800_000,
            1024 => 600_000,
            2048 => 75_000,
            4096 => 18_750,
            8192 => 4_687,
            _ => pass_factor,
        };
        map.clone()
    };
    let scaled = working.multiply_scalar(65535.0);
    let recurved = recurve_passes(scaled, duration, power, scale, inflate, pass_factor);
    let normalized = recurved.divide_scalar(65535.0);
    if preserve {
        crate::transformer::resize(&normalized, nerthus_engine::Resolution::square(size))
    } else {
        normalized
    }
}

/// `Profiles.ThermalShaper`: the TS engine's thermal shaping of the first
/// channel (`SetVars(res, 5000, 38, 0, shape)`), blended back over the map
/// by `influence`.
pub fn thermal_shaper(map: &Map, influence: f32, shape: f32) -> Map {
    let shaped = ts::thermal_shape(map.data(), map.width(), shape as f64);
    let combined = classic_combiner::combine(map.data(), &shaped, influence, 0, true);
    Map::from_data(maths::square_resolution(combined.len()), combined)
}
