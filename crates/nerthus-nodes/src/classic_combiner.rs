//! `ClassicCombiner`: the classic plane blend.

use rayon::prelude::*;

use crate::maths::{self, dotnet_max, dotnet_min};

/// `ClassicCombiner.Combine`: blends plane `b` into `a` by `ratio` with one
/// of eleven `mode`s, then clamps to [0, 1] when asked.
pub fn combine(a: &[f32], b: &[f32], ratio: f32, mode: i32, clamp: bool) -> Vec<f32> {
    let right = ratio;
    let left = 1.0 - ratio;
    let blend = |x: f32, y: f32| -> f32 {
        match mode {
            1 => (x + y * ratio).abs(),
            2 => (x * left + (1.0 - (1.0 - y) * (1.0 - x)) * ratio).abs(),
            3 => x - y * ratio,
            4 => (x * left + x * y * ratio).abs(),
            5 => (x * left + x / y * ratio).abs(),
            6 => (x * left + dotnet_max(x, y) * ratio).abs(),
            7 => (x * left + dotnet_min(x, y) * ratio).abs(),
            8 => x * left + (x + y).sqrt() * ratio,
            9 => x * left + maths::pow(x, y) * (x - y) * ratio,
            10 => (y - x * ratio).abs(),
            _ => x * left + y * right,
        }
    };
    a.par_iter()
        .zip(b.par_iter())
        .map(|(&x, &y)| {
            let value = blend(x, y);
            if clamp { dotnet_max(dotnet_min(value, 1.0), 0.0) } else { value }
        })
        .collect()
}
