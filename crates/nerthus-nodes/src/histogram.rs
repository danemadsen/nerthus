//! Histogram equalization from the engine.
//!
//! Per-thread float histograms summed in whatever order the threads finish
//! vary in the last bits between runs; this port accumulates row by row.

use nerthus_engine::Map;
use rayon::prelude::*;

use crate::maths;
use crate::spline::RegularSpline;

/// The default bin count for equalization.
pub const EQUALIZE_BINS: usize = 16384;

/// The default bin count for histogram matching.
pub const MATCH_BINS: usize = 65536;

/// A smoothed, normalized cumulative histogram of a map.
pub struct Histogram {
    pub bins: Vec<f32>,
    pub cumulative: Vec<f32>,
}

/// Passes of the [0.28, 0.44, 0.28] smoothing for `bins` bins.
fn smoothing_passes(bins: usize) -> usize {
    let passes = ((bins as f64).log2() / 2.0) as i64 - 3;
    passes.max(0) as usize
}

/// Two passes of the 3-tap smoothing per iteration, endpoints fixed.
fn smooth(values: &mut [f32], passes: usize) {
    let length = values.len();
    let mut scratch = vec![0.0f32; length];
    scratch[0] = values[0];
    scratch[length - 1] = values[length - 1];
    for _ in 0..passes {
        for index in 1..length - 1 {
            scratch[index] = (values[index - 1] + values[index + 1]) * 0.28 + 0.44 * values[index];
        }
        for index in 1..length - 1 {
            values[index] = (scratch[index - 1] + scratch[index + 1]) * 0.28 + 0.44 * scratch[index];
        }
    }
}

impl Histogram {
    /// Builds the histogram of a map's first channel.
    pub fn of(map: &Map, bin_count: usize) -> Self {
        let data = map.data();
        let width = map.width().max(1);
        let last_bin = bin_count - 1;

        let row_histograms: Vec<Vec<f32>> = data
            .par_chunks(width)
            .map(|row| {
                let mut local = vec![0.0f32; bin_count];
                for &value in row {
                    let position = value * last_bin as f32;
                    let lower = (position as i32).min(bin_count as i32 - 2);
                    let fraction = position - lower as f32;
                    local[lower as usize] += 1.0 - fraction;
                    local[lower as usize + 1] += fraction;
                }
                local
            })
            .collect();

        let mut bins = vec![0.0f32; bin_count];
        for local in &row_histograms {
            for (total, value) in bins.iter_mut().zip(local) {
                *total += value;
            }
        }
        let area = data.len() as f32;
        for value in &mut bins {
            *value /= area;
        }

        smooth(&mut bins, smoothing_passes(bin_count));

        let mut cumulative = vec![0.0f32; bin_count];
        cumulative[0] = bins[0];
        for index in 1..bin_count {
            cumulative[index] = cumulative[index - 1] + bins[index];
        }
        let first = cumulative[0];
        let last = cumulative[bin_count - 1];
        for value in &mut cumulative {
            *value = (*value - first) / (last - first);
        }

        Self { bins, cumulative }
    }

    /// Maps every value through the cumulative histogram.
    pub fn equalize(&self, map: &Map) -> Map {
        let spline = RegularSpline::constrained(&self.cumulative);
        let scale = (self.cumulative.len() - 1) as f32;
        map.map_values(|value| spline.interpolate(value * scale))
    }
}

/// Autolevel, then histogram equalization.
pub fn equalize(map: Map) -> Map {
    let leveled = maths::autolevel(map);
    let histogram = Histogram::of(&leveled, EQUALIZE_BINS);
    histogram.equalize(&leveled)
}

/// The index in ascending `values` nearest `target`.
fn nearest_index(values: &[f32], target: f32) -> usize {
    let mut low: i64 = 0;
    let mut high: i64 = values.len() as i64 - 1;
    while low <= high {
        let middle = low + (high - low) / 2;
        let value = values[middle as usize];
        if value < target {
            low = middle + 1;
            continue;
        }
        if value > target {
            high = middle - 1;
            continue;
        }
        return middle as usize;
    }
    if high < 0 {
        return 0;
    }
    if low >= values.len() as i64 {
        return values.len() - 1;
    }
    let above = (values[low as usize] - target).abs();
    let below = (values[high as usize] - target).abs();
    if above < below {
        return low as usize;
    }
    high as usize
}

/// Maps `source` so its histogram follows `target`'s:
/// every source bin goes to the target bin with the nearest cumulative
/// share, smoothed, then read back through a spline.
fn match_to(source: &Map, source_histogram: &Histogram, target_histogram: &Histogram) -> Map {
    let target_cumulative = &target_histogram.cumulative;
    let mut mapping: Vec<f32> = source_histogram
        .cumulative
        .par_iter()
        .map(|&share| nearest_index(target_cumulative, share) as f32)
        .collect();
    let passes = smoothing_passes(mapping.len());
    smooth(&mut mapping, passes);
    let spline = RegularSpline::constrained(&mapping);
    let scale = (mapping.len() - 1) as f32;
    let target_scale = (target_cumulative.len() - 1) as f32;
    source.map_values(|value| spline.interpolate(value * scale) / target_scale)
}

/// Autolevels `source`, then reshapes its histogram to match
/// `reference`'s and returns it in `reference`'s range.
pub fn match_histogram(source: &Map, reference: &Map) -> Map {
    let leveled = maths::autolevel(source.clone());
    let source_histogram = Histogram::of(&leveled, MATCH_BINS);

    let (reference_min, reference_max) = maths::min_max(reference.data());
    let mut normalized_reference = reference.clone();
    maths::remap(&mut normalized_reference, reference_max, reference_min, 1.0, 0.0);
    let reference_histogram = Histogram::of(&normalized_reference, MATCH_BINS);

    let mut matched = match_to(&leveled, &source_histogram, &reference_histogram);
    maths::remap(&mut matched, 1.0, 0.0, reference_max, reference_min);
    matched
}
