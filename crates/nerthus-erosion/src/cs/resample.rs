//! The snow pyramid's resamplers (`LandscapeLayer` methods in the CS engine).
//!
//! Same 5x5 kernel as the CE engine, but this build multiplies by the reciprocal
//! of the kernel sum when downsampling and accumulates the upsampled taps in
//! a different order per map, so each sum is written exactly as compiled.

use crate::ce::kernel::{FloatMap, KERNEL};

/// Clamped index into a `size`-wide map.
#[inline]
fn clamp(value: i32, size: i32) -> i32 {
    let mut clamped = if value < 0 { 0 } else { value };
    if clamped > size - 1 {
        clamped = size - 1;
    }
    clamped
}

/// Coarse pixel (`x`, `y`) from the fine map: the kernel centred on fine
/// pixel (2x, 2y), times `1 / kernel_sum` (for the heights, and the
/// snowfall and melt masks).
pub fn downsample(fine: &FloatMap, x: i32, y: i32, reciprocal: f32) -> f32 {
    let size = fine.size;
    let k = &KERNEL;
    let at = |dx: i32, dy: i32| fine.at(clamp(2 * x + dx, size), clamp(2 * y + dy, size));
    let total = k[0] * at(-2, -2)
        + k[5] * at(-1, -2)
        + k[10] * at(0, -2)
        + k[15] * at(1, -2)
        + k[20] * at(2, -2)
        + k[1] * at(-2, -1)
        + k[6] * at(-1, -1)
        + k[11] * at(0, -1)
        + k[16] * at(1, -1)
        + k[21] * at(2, -1)
        + k[2] * at(-2, 0)
        + k[7] * at(-1, 0)
        + k[12] * at(0, 0)
        + k[17] * at(1, 0)
        + k[22] * at(2, 0)
        + k[3] * at(-2, 1)
        + k[8] * at(-1, 1)
        + k[13] * at(0, 1)
        + k[18] * at(1, 1)
        + k[23] * at(2, 1)
        + k[4] * at(-2, 2)
        + k[9] * at(-1, 2)
        + k[14] * at(0, 2)
        + k[19] * at(1, 2)
        + k[24] * at(2, 2);
    total * reciprocal
}

/// Coarse pixel reader for the upsamplers: offsets are relative to
/// (`x >> 1`, `y >> 1`) and clamped to the coarse map.
struct CoarseTaps<'a> {
    map: &'a FloatMap,
    x: i32,
    y: i32,
}

impl CoarseTaps<'_> {
    fn new(map: &FloatMap, x: u32, y: u32) -> CoarseTaps<'_> {
        CoarseTaps {
            map,
            x: (x as i32) >> 1,
            y: (y as i32) >> 1,
        }
    }

    #[inline]
    fn at(&self, dx: i32, dy: i32) -> f32 {
        let size = self.map.size;
        self.map.at(clamp(self.x + dx, size), clamp(self.y + dy, size))
    }
}

/// Divisors shared by both upsamplers, one per (x, y) parity.
fn even_even_weight() -> f32 {
    let k = &KERNEL;
    (k[10] + k[0]) * 4.0 + k[12]
}

fn even_odd_weight() -> f32 {
    let k = &KERNEL;
    k[11] * 2.0 + k[1] * 4.0
}

fn odd_even_weight() -> f32 {
    let k = &KERNEL;
    k[7] * 2.0 + k[5] * 4.0
}

fn odd_odd_weight() -> f32 {
    KERNEL[6] * 4.0
}

/// The height at fine pixel (`x`, `y`) of the parent layer.
pub fn upsample_height(coarse: &FloatMap, x: u32, y: u32) -> f32 {
    let k = &KERNEL;
    let taps = CoarseTaps::new(coarse, x, y);
    let at = |dx: i32, dy: i32| taps.at(dx, dy);
    match (x & 1, y & 1) {
        (0, 0) => {
            let total = k[0] * at(-1, -1)
                + k[10] * at(0, -1)
                + k[20] * at(1, -1)
                + k[2] * at(-1, 0)
                + k[12] * at(0, 0)
                + k[22] * at(1, 0)
                + k[4] * at(-1, 1)
                + k[14] * at(0, 1)
                + k[24] * at(1, 1);
            total / even_even_weight()
        }
        (0, _) => {
            let total = k[1] * at(-1, 0) + k[11] * at(0, 0) + k[21] * at(1, 0) + k[3] * at(-1, 1) + k[13] * at(0, 1) + k[23] * at(1, 1);
            total / even_odd_weight()
        }
        (_, 0) => {
            let total = k[5] * at(0, -1) + k[15] * at(1, -1) + k[7] * at(0, 0) + k[17] * at(1, 0) + k[9] * at(0, 1) + k[19] * at(1, 1);
            total / odd_even_weight()
        }
        _ => {
            let total = k[6] * at(0, 0) + k[16] * at(1, 0) + k[8] * at(0, 1) + k[18] * at(1, 1);
            total / odd_odd_weight()
        }
    }
}

/// The snow depth at fine pixel (`x`, `y`) of the parent
/// layer. Same taps as the heights, but the compiler summed them as small
/// trees (transcribed from the disassembly).
pub fn upsample_snow(coarse: &FloatMap, x: u32, y: u32) -> f32 {
    let k = &KERNEL;
    let taps = CoarseTaps::new(coarse, x, y);
    let at = |dx: i32, dy: i32| taps.at(dx, dy);
    match (x & 1, y & 1) {
        (0, 0) => {
            let top = (k[0] * at(-1, -1) + k[10] * at(0, -1)) + k[20] * at(1, -1);
            let middle = k[12] * at(0, 0) + k[2] * at(-1, 0);
            let rest = (k[24] * at(1, 1) + k[14] * at(0, 1)) + (k[4] * at(-1, 1) + k[22] * at(1, 0));
            ((top + middle) + rest) / even_even_weight()
        }
        (0, _) => {
            let below = (k[3] * at(-1, 1) + k[13] * at(0, 1)) + k[23] * at(1, 1);
            let level = (k[1] * at(-1, 0) + k[11] * at(0, 0)) + k[21] * at(1, 0);
            (below + level) / even_odd_weight()
        }
        (_, 0) => {
            let right = (k[9] * at(0, 1) + k[17] * at(1, 0)) + k[19] * at(1, 1);
            let rest = (k[5] * at(0, -1) + k[15] * at(1, -1)) + k[7] * at(0, 0);
            (right + rest) / odd_even_weight()
        }
        _ => {
            let below = k[18] * at(1, 1) + k[8] * at(0, 1);
            let level = k[6] * at(0, 0) + k[16] * at(1, 0);
            (below + level) / odd_odd_weight()
        }
    }
}
