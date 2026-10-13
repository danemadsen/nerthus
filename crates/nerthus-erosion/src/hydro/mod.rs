//! Port of Hydrologic.dll (`HydrologicNative`), the drainage engine behind
//! the HydroFix (`HydrologicalCorrection`) and Rivers nodes, transcribed
//! from the disassembly. The native passes are OpenMP loops; this port
//! follows the single-threaded order (the native output varies slightly
//! with the thread count where those loops update cells in place).

mod drainage;
mod rivers;

use rayon::prelude::*;

use crate::ce::kernel::{downsample, kernel_sum, upsample, FloatMap};
use drainage::Drainage;
pub use rivers::{rivers, RiverOutput, RiverSettings};

/// One level of the resolution pyramid (`HydrologicNative` layers).
pub(crate) struct Level {
    pub size: usize,
    /// Whether this is the coarsest level, which smooths its correction once.
    pub coarsest: bool,
    /// [13] terrain.
    pub height: FloatMap,
    /// [2] correction applied after the level (negative carving).
    pub correction: Vec<f32>,
    /// [3] smoothing scratch.
    pub scratch: Vec<f32>,
    /// [12] detail the next coarser level lost.
    pub residual: Vec<f32>,
}

impl Level {
    fn new(size: usize, height: FloatMap) -> Level {
        let count = size * size;
        Level {
            size,
            coarsest: false,
            height,
            correction: vec![0.0; count],
            scratch: vec![0.0; count],
            residual: Vec::new(),
        }
    }

    /// The half-resolution copy, recording the detail it
    /// loses in `self.residual`.
    fn coarser(&mut self) -> Level {
        let fine_size = self.size;
        let size = fine_size >> 1;
        let sum = kernel_sum();
        let mut height = FloatMap::new(size as i32, 0.0);
        height.data.par_chunks_mut(size).enumerate().for_each(|(y, row)| {
            for (x, value) in row.iter_mut().enumerate() {
                *value = downsample(&self.height, x as i32, y as i32, sum);
            }
        });
        let mut residual = vec![0.0f32; fine_size * fine_size];
        residual.par_chunks_mut(fine_size).enumerate().for_each(|(y, row)| {
            for (x, value) in row.iter_mut().enumerate() {
                let smooth = upsample(&height, x as u32, y as u32);
                *value = self.height.data[y * fine_size + x] - smooth;
            }
        });
        self.residual = residual;
        Level::new(size, height)
    }

    /// `fine` becomes this level's (corrected) terrain
    /// upsampled plus the detail it lost.
    fn restore(&self, fine: &mut Level) {
        let size = fine.size;
        let residual = std::mem::take(&mut fine.residual);
        fine.height.data.par_chunks_mut(size).enumerate().for_each(|(y, row)| {
            for (x, value) in row.iter_mut().enumerate() {
                let smooth = upsample(&self.height, x as u32, y as u32);
                *value = smooth + residual[y * size + x];
            }
        });
    }
}

/// `log2(res) - 1` as the engine computes it (`log(res) / logf(2) - 3 + 2`),
/// which truncates to one less for powers of two.
fn pyramid_levels(size: usize) -> i32 {
    let ratio = (size as f64).ln() / (2.0f32.ln() as f64);
    ((ratio - 3.0) + 2.0) as i32
}

/// `HydrologicNative::hydrological_correction`: makes every pit drain by
/// carving channels along the priority-flood drainage tree. `downcutting`
/// is the HydroFix parameter (-1 for version 1 nodes).
pub fn hydrological_correction(heights: &[f32], size: usize, downcutting: f32) -> Vec<f32> {
    let levels = pyramid_levels(size);
    let map = FloatMap {
        size: size as i32,
        data: heights.to_vec(),
    };
    let mut stack = vec![Level::new(size, map)];
    for _ in 1..levels.max(1) {
        let finer = stack.last_mut().expect("the stack starts with the input level");
        let coarser = finer.coarser();
        stack.push(coarser);
    }
    if let Some(coarsest) = stack.last_mut() {
        coarsest.coarsest = true;
    }

    let mut drainage = Drainage::new(downcutting);
    while stack.len() > 1 {
        let mut level = stack.pop().expect("more than one level is left");
        drainage.correct(&mut level);
        apply_correction(&mut level);
        let finer = stack.last_mut().expect("a finer level is left");
        level.restore(finer);
    }
    let mut level = stack.pop().expect("the input level is left");
    drainage.correct(&mut level);
    apply_correction(&mut level);
    level.height.data.iter().map(|&value| if value < 0.0 { 0.0 } else { value }).collect()
}

/// Adds the carved correction.
fn apply_correction(level: &mut Level) {
    let correction = &level.correction;
    level.height.data.par_iter_mut().zip(correction.par_iter()).for_each(|(height, &change)| {
        *height += change;
    });
}
