//! The SYCL median filter (`filter_fast<1>` in the SYCL kernels, exported
//! as `MedianFilter`), as the Median node runs it: a smoothed
//! median over a disc of radius r + ½ with clamped borders, `iterations`
//! times. The kernels (radius 2, 3 and 4) select the middle three order
//! statistics with insertion
//! min/max networks, so the result is the same as sorting the disc.

use nerthus_engine::Map;
use rayon::prelude::*;

use crate::{gpu, sycl};

/// `MedianFilter.MedianQuality`: the window radius.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MedianQuality {
    Low,
    Medium,
    High,
}

impl MedianQuality {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Medium" => Self::Medium,
            "High" => Self::High,
            _ => Self::Low,
        }
    }

    pub(crate) fn radius(self) -> i32 {
        match self {
            Self::Low => 2,
            Self::Medium => 3,
            Self::High => 4,
        }
    }
}

/// The disc a kernel of `radius` reads: every offset within r + ½.
fn disc_offsets(radius: i32) -> Vec<(i32, i32)> {
    let limit = radius * (radius + 1);
    let mut offsets = Vec::new();
    for offset_y in -radius..=radius {
        for offset_x in -radius..=radius {
            if offset_x * offset_x + offset_y * offset_y <= limit {
                offsets.push((offset_y, offset_x));
            }
        }
    }
    offsets
}

/// One smoothed-median pass: the disc's median weighted ½ and its two
/// neighbours in sorted order ¼ each.
fn smoothed_median_pass(source: &[f32], width: usize, height: usize, radius: i32) -> Vec<f32> {
    let last_x = width as i32 - 1;
    let last_y = height as i32 - 1;
    let offsets = disc_offsets(radius);
    let middle = offsets.len() / 2;
    let mut result = vec![0.0f32; width * height];
    result.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
        let mut window = Vec::with_capacity(offsets.len());
        for (x, out) in row.iter_mut().enumerate() {
            window.clear();
            for &(offset_y, offset_x) in &offsets {
                let sample_y = (y as i32 + offset_y).max(0).min(last_y) as usize;
                let sample_x = (x as i32 + offset_x).max(0).min(last_x) as usize;
                window.push(source[sample_y * width + sample_x]);
            }
            let (lower, centre, upper) = window.select_nth_unstable_by(middle, |a, b| a.total_cmp(b));
            let centre = *centre;
            let below = lower.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let above = upper.iter().copied().fold(f32::INFINITY, f32::min);
            *out = (below + above).mul_add(0.25, centre * 0.5);
        }
    });
    result
}

/// The passes `MedianFilter.ResolutionIndependent` runs: `strength` scaled
/// to the resolution (per 512 pixels), over the window radius.
pub(crate) fn iterations(width: usize, strength: i32, quality: MedianQuality) -> i32 {
    let scaled = strength * width as i32 / 512;
    (scaled * 2 / quality.radius()).max(1)
}

/// `MedianFilter.ResolutionIndependent`: `strength` scaled to the
/// resolution decides how many passes run. Runs on the GPU when one is
/// active (see [`sycl`]).
pub fn resolution_independent(map: &Map, strength: i32, quality: MedianQuality) -> Map {
    let gpu = |device: &nerthus_compute::GpuDevice| gpu::median::resolution_independent(device, map, strength, quality);
    let cpu = || resolution_independent_cpu(map, strength, quality);
    sycl::run("MedianFilter", map.width(), sycl::PREFERRED, gpu, cpu)
}

/// [`resolution_independent`] on the CPU.
pub fn resolution_independent_cpu(map: &Map, strength: i32, quality: MedianQuality) -> Map {
    let width = map.width();
    let height = map.height();
    let radius = quality.radius();
    let iterations = iterations(width, strength, quality);
    let mut data = map.channel(0).to_vec();
    for _ in 0..iterations {
        data = smoothed_median_pass(&data, width, height, radius);
    }
    Map::from_data(map.resolution(), data)
}
