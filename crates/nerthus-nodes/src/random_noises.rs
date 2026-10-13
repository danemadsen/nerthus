//! The per-pixel random maps (`RandomNoises`).

use nerthus_engine::{Map, Resolution};
use crate::maths;
use crate::noise::{self, NoisesPerlin};
use crate::random::XorShift64Star;

/// `RandomNoises.Random`: uniform values in `min..max`, row by row.
pub fn uniform(resolution: Resolution, min: f32, max: f32, seed: i32) -> Map {
    let mut random = XorShift64Star::new(seed);
    let data = (0..resolution.area()).map(|_| random.next_float_range(min, max)).collect();
    Map::from_data(resolution, data)
}

/// `RandomNoises.Gaussian` (`GaussianNoiseGenerator`): Box–Muller normal
/// values.
pub fn gaussian(resolution: Resolution, seed: i32, mean: f32, deviation: f32) -> Map {
    let mut random = XorShift64Star::new(seed);
    let data = (0..resolution.area())
        .map(|_| {
            let radius_sample = 1.0 - random.next_float();
            let angle_sample = 1.0 - random.next_float();
            let radius = (-2.0 * maths::ln(radius_sample)).sqrt();
            let angle = std::f32::consts::PI * 2.0 * angle_sample;
            let normal = radius * maths::sin(angle);
            mean + deviation * normal
        })
        .collect();
    Map::from_data(resolution, data)
}

/// `RandomNoises.Fixed`: `count + 1` random pixels set to random values in
/// `min..max` on black (`relative` scales the count with the resolution).
pub fn fixed(resolution: Resolution, count: i32, relative: bool, min: f32, max: f32, seed: i32) -> Map {
    let mut map = Map::new(resolution, 1);
    let mut random = XorShift64Star::new(seed);
    let width = resolution.x as i32;
    let mut total = count;
    if relative {
        total *= (width as f32 / 512.0) as i32;
    }
    for _ in 0..=total {
        let x = random.next_range(0, width);
        let y = random.next_range(0, width);
        let value = random.next_float_range(min, max);
        map.set(x as usize, y as usize, value);
    }
    map
}

/// `RandomNoises.FixedWhite`: `count` random pixels set to `value`.
pub fn fixed_white(resolution: Resolution, count: i32, value: f32, relative: bool, seed: i32) -> Map {
    let mut map = Map::new(resolution, 1);
    let mut random = XorShift64Star::new(seed);
    let width = resolution.x as i32;
    let mut total = count;
    if relative {
        total *= (width as f32 / 512.0) as i32;
    }
    let span = (width - 1) as f32;
    for _ in 0..total {
        let x = (random.next_float() * span + 1.0) as usize;
        let y = (random.next_float() * span + 1.0) as usize;
        map.set(x, y, value);
    }
    map
}

/// `RandomNoises.FixedWhiteMapped`: like [`fixed_white`], but only where
/// `guide` is above 0.01 (it keeps drawing until `count` pixels land).
pub fn fixed_white_mapped(guide: &Map, count: i32, value: f32, relative: bool, seed: i32) -> Map {
    let mut map = Map::new(guide.resolution(), 1);
    let mut random = XorShift64Star::new(seed);
    let width = guide.width() as i32;
    let mut total = count;
    if relative {
        total *= (width as f32 / 512.0) as i32;
    }
    let span = (width - 1) as f32;
    let mut placed = 0;
    while placed < total {
        let x = (random.next_float() * span + 1.0) as usize;
        let y = (random.next_float() * span + 1.0) as usize;
        if guide.get(x, y) > 0.01 {
            map.set(x, y, value);
            placed += 1;
        }
    }
    map
}

/// `RandomNoises.Perlin`: fine, fixed-seed Perlin noise leveled to
/// `0..height` (the seed argument is ignored).
pub fn perlin(resolution: Resolution, height: f32) -> Map {
    let parameters = NoisesPerlin {
        scale: 0.0,
        octaves: 14,
        gain: 1.0,
        seed: 14456,
        warp_type: 0,
        ..NoisesPerlin::default()
    };
    let noise = noise::noises_perlin(resolution.x, &parameters);
    let leveled = maths::autolevel(noise);
    leveled.multiply_scalar(height)
}
