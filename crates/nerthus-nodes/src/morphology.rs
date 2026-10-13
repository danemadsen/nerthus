//! The morphology (`Morphology`): flat
//! dilations by segment envelopes and Felzenszwalb–Huttenlocher distance
//! transforms. Every pass works on rows and writes its result transposed,
//! so two passes cover both axes.

use nerthus_engine::Map;
use rayon::prelude::*;

use crate::maths::{self, dotnet_max, dotnet_min};

/// `KernalType`: the dilation footprint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kernel {
    Cross,
    Rectangle,
    Line,
    LineVertical,
}

impl Kernel {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Rectangle" => Self::Rectangle,
            "Line" => Self::Line,
            "LineVertical" => Self::LineVertical,
            _ => Self::Cross,
        }
    }
}

/// Options of [`rectangular_pass`].
#[derive(Clone, Copy, Default)]
struct PassOptions {
    transpose_output: bool,
    max_merge: bool,
    transpose_input: bool,
}

/// `RectangularPass`: a running maximum over ±`kernel` along each row of
/// `source`, built from the row's upper envelope of flat segments. Writes
/// into `target` (merging with what is there when `max_merge`) and returns
/// the transposed input when `transpose_input`.
fn rectangular_pass(source: &[f32], target: &mut [f32], resolution: usize, kernel: f32, options: PassOptions) -> Option<Vec<f32>> {
    let rows: Vec<(Vec<f32>, Option<Vec<f32>>)> = source
        .par_chunks(resolution)
        .map(|row| {
            let mut centers = vec![0usize; resolution];
            let mut ranges = vec![0.0f32; resolution + 1];
            let mut segment = 0usize;
            centers[0] = 0;
            ranges[0] = 0.0 - kernel;
            ranges[1] = kernel;
            for index in 1..resolution {
                let mut start = index as f32 - kernel;
                if row[index] <= row[centers[segment]] {
                    start = ranges[segment + 1];
                }
                while start < ranges[segment] {
                    segment -= 1;
                    start = index as f32 - kernel;
                    if row[index] <= row[centers[segment]] {
                        start = ranges[segment + 1];
                    }
                }
                segment += 1;
                centers[segment] = index;
                ranges[segment] = start;
                ranges[segment + 1] = index as f32 + kernel;
            }
            segment = 0;
            let mut values = vec![0.0f32; resolution];
            for (position, value) in values.iter_mut().enumerate() {
                while ranges[segment + 1] < position as f32 {
                    segment += 1;
                }
                *value = row[centers[segment]];
            }
            let transposed = options.transpose_input.then(|| row.to_vec());
            (values, transposed)
        })
        .collect();

    let mut transposed_input = options.transpose_input.then(|| vec![0.0f32; resolution * resolution]);
    for (row, (values, original)) in rows.into_iter().enumerate() {
        for (position, value) in values.into_iter().enumerate() {
            let index = if options.transpose_output { position * resolution + row } else { row * resolution + position };
            let merged = if options.max_merge { dotnet_max(target[index], value) } else { value };
            target[index] = merged;
        }
        if let (Some(transposed), Some(original)) = (transposed_input.as_mut(), original) {
            for (position, value) in original.into_iter().enumerate() {
                transposed[position * resolution + row] = value;
            }
        }
    }
    transposed_input
}

/// `DilateLine`: horizontal dilation only.
fn dilate_line(data: &[f32], resolution: usize, kernel_size: f32) -> Vec<f32> {
    let mut result = vec![0.0f32; resolution * resolution];
    rectangular_pass(data, &mut result, resolution, kernel_size * 0.5, PassOptions::default());
    result
}

/// `DilateRectangle`: rows by `height`, then the result's rows by `width`,
/// merged by maximum with the input.
fn dilate_rectangle(data: &[f32], resolution: usize, width: f32, height: f32) -> Vec<f32> {
    let half_width = width * 0.5;
    let half_height = height * 0.5;
    let source = data.to_vec();
    let mut first = vec![0.0f32; resolution * resolution];
    let transpose = PassOptions {
        transpose_output: true,
        ..PassOptions::default()
    };
    rectangular_pass(&source, &mut first, resolution, half_height, transpose);
    // The buffers swap here: the second pass merges into the input.
    let mut result = source;
    let merge = PassOptions {
        transpose_output: true,
        max_merge: true,
        ..PassOptions::default()
    };
    rectangular_pass(&first, &mut result, resolution, half_width, merge);
    result
}

/// `DilateCross`: the union of a horizontal and a vertical dilation.
pub fn dilate_cross(data: &[f32], resolution: usize, kernel_size: f32) -> Vec<f32> {
    let half = kernel_size * 0.5;
    let mut result = vec![0.0f32; resolution * resolution];
    let first = PassOptions {
        transpose_input: true,
        ..PassOptions::default()
    };
    let transposed = rectangular_pass(data, &mut result, resolution, half, first).expect("the first pass transposes its input");
    let second = PassOptions {
        transpose_output: true,
        max_merge: true,
        ..PassOptions::default()
    };
    rectangular_pass(&transposed, &mut result, resolution, half, second);
    result
}

/// `Morphology.Dilate`: `iterations` flat dilations by `kernel`, `size` of
/// the map across (`shape` stretches the rectangle).
pub fn dilate(map: &Map, kernel: Kernel, size: f32, iterations: i32, shape: f32) -> Map {
    let resolution = map.width();
    let pixels = resolution as f32 * size;
    let mut tall = shape * 0.98 + 1.01;
    let mut wide = 2.0 - tall;
    wide *= pixels;
    tall *= pixels;
    let mut data = map.data().to_vec();
    for _ in 0..iterations {
        data = match kernel {
            Kernel::Cross => dilate_cross(&data, resolution, pixels),
            Kernel::Rectangle => dilate_rectangle(&data, resolution, wide, tall),
            Kernel::Line => dilate_line(&data, resolution, pixels),
            Kernel::LineVertical => dilate_rectangle(&data, resolution, pixels, 1.0),
        };
    }
    Map::from_data(maths::square_resolution(data.len()), data)
}

/// `DTFirstPass`: distances to the nearest seed along each row (seeds are
/// 0, everything else `resolution`), squared and written transposed.
fn distance_rows(seeds: &mut [f32], resolution: usize) -> Vec<f32> {
    let limit = resolution as f32;
    let squared_rows: Vec<Vec<f32>> = seeds
        .par_chunks_mut(resolution)
        .map(|row| {
            let mut nearest = limit;
            for value in row.iter_mut() {
                if *value <= nearest {
                    nearest = *value;
                }
                *value = nearest;
                nearest += 1.0;
            }
            let mut squared = vec![0.0f32; resolution];
            nearest = limit;
            for index in (0..resolution).rev() {
                let value = row[index];
                if value <= nearest {
                    nearest = value;
                }
                squared[index] = nearest * nearest;
                nearest += 1.0;
            }
            squared
        })
        .collect();
    let mut transposed = vec![0.0f32; resolution * resolution];
    for (row, squared) in squared_rows.iter().enumerate() {
        for (index, &value) in squared.iter().enumerate() {
            transposed[index * resolution + row] = value;
        }
    }
    transposed
}

/// `ParabolicPass`: the lower envelope of parabolas rooted at each value of
/// a row, sampled at every position and written transposed.
fn parabolic_pass(source: &[f32], resolution: usize) -> Vec<f32> {
    let rows: Vec<Vec<f32>> = source
        .par_chunks(resolution)
        .map(|row| {
            let mut centers = vec![0usize; resolution];
            let mut ranges = vec![0.0f32; resolution + 1];
            let mut segment = 0usize;
            centers[0] = 0;
            ranges[0] = f32::MIN;
            ranges[1] = f32::MAX;
            let intersection = |index: usize, center: usize| {
                let here = row[index] + (index * index) as f32;
                let there = row[center] + (center * center) as f32;
                (here - there) / (2 * index as i64 - 2 * center as i64) as f32
            };
            for index in 1..resolution {
                let mut start = intersection(index, centers[segment]);
                while start <= ranges[segment] {
                    segment -= 1;
                    start = intersection(index, centers[segment]);
                }
                segment += 1;
                centers[segment] = index;
                ranges[segment] = start;
                ranges[segment + 1] = f32::MAX;
            }
            segment = 0;
            let mut values = vec![0.0f32; resolution];
            for (position, value) in values.iter_mut().enumerate() {
                while ranges[segment + 1] < position as f32 {
                    segment += 1;
                }
                let center = centers[segment];
                let offset = position as i64 - center as i64;
                *value = (offset * offset) as f32 + row[center];
            }
            values
        })
        .collect();
    let mut transposed = vec![0.0f32; resolution * resolution];
    for (row, values) in rows.iter().enumerate() {
        for (index, &value) in values.iter().enumerate() {
            transposed[index * resolution + row] = value;
        }
    }
    transposed
}

/// Options of [`distance_transform`].
#[derive(Clone, Copy, Debug)]
pub struct DistanceOptions {
    pub falloff: f32,
    pub threshold: f32,
    pub invert_input: bool,
    pub invert_output: bool,
    pub clamp: bool,
}

/// `Morphology.DistanceTransform`: the Euclidean distance (in pixels,
/// divided by `falloff`) from every pixel to the nearest pixel above
/// `threshold`.
pub fn distance_transform(data: &[f32], resolution: usize, options: DistanceOptions) -> Vec<f32> {
    let falloff = if options.falloff <= 0.001 { 0.001 } else { options.falloff };
    let limit = resolution as f32;
    let mut seeds: Vec<f32> = data
        .par_iter()
        .map(|&value| {
            let tested = if options.invert_input { 1.0 - value } else { value };
            if tested > options.threshold { 0.0 } else { limit }
        })
        .collect();
    let squared = distance_rows(&mut seeds, resolution);
    let distances = parabolic_pass(&squared, resolution);
    distances
        .par_iter()
        .map(|&value| {
            let distance = value.sqrt();
            let mut scaled = dotnet_max(distance / falloff, 0.0);
            if options.clamp {
                scaled = dotnet_min(scaled, 1.0);
            }
            if options.invert_output { 1.0 - scaled } else { scaled }
        })
        .collect()
}

/// `Morphology.DistanceTransform3VariableRadius`: distance where each pixel
/// starts at `(1 − value) × resolution` instead of a binary seed.
pub fn distance_transform_variable_radius(data: &[f32], resolution: usize) -> Vec<f32> {
    let limit = resolution as f32;
    let squared: Vec<f32> = data
        .par_iter()
        .map(|&value| {
            let start = limit - limit * value;
            start * start
        })
        .collect();
    let first = parabolic_pass(&squared, resolution);
    let second = parabolic_pass(&first, resolution);
    second.par_iter().map(|value| value.sqrt()).collect()
}
