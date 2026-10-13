//! The ray-cast distance (`Morph.DistanceRT` and `MorphologyRT`).
//!
//! Rays march across the map in 16.16 fixed point, carrying a height that
//! drops by `dz` per step, and leave the highest value seen along the way.
//! The rays of a direction may be cast in parallel, adding into shared
//! cells atomically; interior cells receive exactly two contributions, so
//! casting them in order gives the same sums.

use nerthus_engine::Map;
use rayon::prelude::*;

use crate::dotnet_vector;
use crate::maths;
use crate::random::XorShift64Star;

/// `MorphologyRT.DistanceRTMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistanceRtMode {
    Asterisk,
    Pyramid,
}

impl DistanceRtMode {
    pub fn from_name(name: &str) -> Self {
        if name == "Pyramid" { Self::Pyramid } else { Self::Asterisk }
    }
}

/// `Fixed`: 16.16 fixed point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Fixed(i32);

const FIXED_SHIFT: u32 = 16;
const FIXED_SCALE: f32 = (1 << FIXED_SHIFT) as f32;

impl Fixed {
    const ONE: Fixed = Fixed(1 << FIXED_SHIFT);
    const ZERO: Fixed = Fixed(0);

    fn from_int(value: i32) -> Self {
        Fixed(value << FIXED_SHIFT)
    }

    fn from_float(value: f32) -> Self {
        Fixed((value * FIXED_SCALE) as i32)
    }

    fn int(self) -> i32 {
        self.0 >> FIXED_SHIFT
    }

    fn float(self) -> f32 {
        self.0 as f32 / FIXED_SCALE
    }

    fn add(self, other: Fixed) -> Fixed {
        Fixed(self.0.wrapping_add(other.0))
    }

    fn sub(self, other: Fixed) -> Fixed {
        Fixed(self.0.wrapping_sub(other.0))
    }

    fn negate(self) -> Fixed {
        Fixed(self.0.wrapping_neg())
    }
}

/// `NextAfterF`: the next float from `from` towards `to`.
fn next_after(from: f32, to: f32) -> f32 {
    if from.is_nan() || to.is_nan() {
        return from + to;
    }
    if from == to {
        return to;
    }
    if from == 0.0 {
        let smallest = f32::from_bits(1);
        return if to > 0.0 { smallest } else { 0.0 - smallest };
    }
    let bits = from.to_bits() as i32;
    let stepped = if (from > 0.0) == (to > from) { bits + 1 } else { bits - 1 };
    f32::from_bits(stepped as u32)
}

/// The state of one `_DistanceRT` build.
struct RayTracer {
    resolution: usize,
    fixed_resolution: Fixed,
    z_coefficient: f32,
    max_coordinate: f32,
    heightmap: Vec<f32>,
    result: Vec<f32>,
    accumulated: Vec<f32>,
}

impl RayTracer {
    /// `_DistanceRT.InterpolateBilinear`: clamped bilinear sample.
    fn sample(&self, x: f32, y: f32) -> f32 {
        let mut x = x - 0.5;
        let mut y = y - 0.5;
        if x < 0.0 {
            x = 0.0;
        } else if x > self.max_coordinate {
            x = self.max_coordinate;
        }
        if y < 0.0 {
            y = 0.0;
        } else if y > self.max_coordinate {
            y = self.max_coordinate;
        }
        let size = self.resolution;
        let top = y as usize;
        let bottom = top + 1;
        let left = x as usize;
        let right = left + 1;
        let fraction_x = x - left as f32;
        let fraction_y = y - top as f32;
        let map = &self.heightmap;
        let upper = map[size * top + left] * (1.0 - fraction_x) + map[size * top + right] * fraction_x;
        let lower = map[size * bottom + left] * (1.0 - fraction_x) + map[size * bottom + right] * fraction_x;
        upper * (1.0 - fraction_y) + lower * fraction_y
    }

    fn add(&mut self, index: i64, value: f32) {
        self.accumulated[index as usize] += value;
    }

    /// `DistanceRay`: one ray from (`x`, `y`) stepping (`dx`, `dy`).
    fn ray(&mut self, mut x: Fixed, mut y: Fixed, dx: Fixed, dy: Fixed, dz: f32) {
        let size = self.resolution as i64;
        let x_major = dx.float().abs() > dy.float().abs();
        let start = self.sample(x.sub(dx).float(), y.sub(dy).float()) + dz;
        let mut carried = start;
        for _ in 0..self.resolution {
            if y < Fixed::ZERO {
                y = y.add(self.fixed_resolution);
                carried = self.sample(x.sub(dx).float(), y.sub(dy).float()) + dz;
            } else if y >= self.fixed_resolution {
                y = y.sub(self.fixed_resolution);
                carried = self.sample(x.sub(dx).float(), y.sub(dy).float()) + dz;
            }
            if x < Fixed::ZERO {
                x = x.add(self.fixed_resolution);
                carried = self.sample(x.sub(dx).float(), y.sub(dy).float()) + dz;
            } else if x >= self.fixed_resolution {
                x = x.sub(self.fixed_resolution);
                carried = self.sample(x.sub(dx).float(), y.sub(dy).float()) + dz;
            }
            let here = self.sample(x.float(), y.float());
            if carried < here {
                carried = here;
            }
            let column = x.int() as i64;
            let row = y.int() as i64;
            if x_major {
                let mut fraction = y.float() - row as f32 - 0.5;
                if fraction > 0.0 {
                    self.add(size * row + column, carried * (1.0 - fraction));
                    if row < size - 2 {
                        self.add(size * (row + 1) + column, carried * fraction);
                    }
                } else {
                    fraction += 1.0;
                    self.add(size * row + column, carried * fraction);
                    if row > 1 {
                        self.add(size * (row - 1) + column, carried * (1.0 - fraction));
                    }
                }
                if row == 0 || row == size - 1 {
                    self.accumulated[(size * row + column) as usize] = carried;
                }
            } else {
                let mut fraction = x.float() - column as f32 - 0.5;
                if fraction > 0.0 {
                    self.add(size * row + column, carried * (1.0 - fraction));
                    if column < size - 2 {
                        self.add(size * row + column + 1, carried * fraction);
                    }
                } else {
                    fraction += 1.0;
                    self.add(size * row + column, carried * fraction);
                    if column > 1 {
                        self.add(size * row + column - 1, carried * (1.0 - fraction));
                    }
                }
                if column == 0 || column == size - 1 {
                    self.accumulated[(size * row + column) as usize] = carried;
                }
            }
            x = x.add(dx);
            y = y.add(dy);
            carried += dz;
        }
    }

    /// `DistanceRayCast`: every ray of one direction, then the running
    /// maximum.
    fn cast(&mut self, direction: [f32; 3], mode: DistanceRtMode) {
        let half = Fixed::from_float(0.5);
        let mut step = [0.0 - direction[0], 0.0 - direction[1], 0.0 - direction[2]];
        step[2] *= self.z_coefficient;
        let absolute = [direction[0].abs(), direction[1].abs(), direction[2].abs()];
        let rows = self.resolution / 16 * 16;
        if absolute[0] > absolute[1] {
            step = [step[0] / absolute[0], step[1] / absolute[0], step[2] / absolute[0]];
            let step_y = Fixed::from_float(step[1]);
            for index in 0..rows {
                let row_start = Fixed::from_int(index as i32).add(half);
                if step[0] > 0.0 {
                    self.ray(half, row_start, Fixed::ONE, step_y, step[2]);
                } else {
                    let start = self.fixed_resolution.sub(Fixed::ONE).add(half);
                    self.ray(start, row_start, Fixed::ONE.negate(), step_y, step[2]);
                }
            }
        } else {
            step = [step[0] / absolute[1], step[1] / absolute[1], step[2] / absolute[1]];
            let step_x = Fixed::from_float(step[0]);
            for index in 0..rows {
                let column_start = Fixed::from_int(index as i32).add(half);
                if step[1] > 0.0 {
                    self.ray(column_start, half, step_x, Fixed::ONE, step[2]);
                } else {
                    let start = self.fixed_resolution.sub(Fixed::ONE).add(half);
                    self.ray(column_start, start, step_x, Fixed::ONE.negate(), step[2]);
                }
            }
        }
        let accumulated = std::mem::take(&mut self.accumulated);
        self.result.par_iter_mut().zip(accumulated.par_iter()).for_each(|(result, &value)| {
            *result = maths::dotnet_max(value, *result);
        });
        self.accumulated = vec![0.0; accumulated.len()];
        if mode == DistanceRtMode::Pyramid {
            self.heightmap.copy_from_slice(&self.result);
        }
    }
}

/// `MorphologyRT.DistanceRT`.
fn distance_rays(heightmap: &[f32], resolution: usize, directions: &[[f32; 3]], mode: DistanceRtMode, terrain: (f32, f32)) -> Vec<f32> {
    let mut tracer = RayTracer {
        resolution,
        fixed_resolution: Fixed::from_int(resolution as i32),
        z_coefficient: terrain.0 / (terrain.1 * resolution as f32),
        max_coordinate: next_after((resolution - 1) as f32, 0.0),
        heightmap: heightmap.to_vec(),
        result: vec![0.0; heightmap.len()],
        accumulated: vec![0.0; heightmap.len()],
    };
    for &direction in directions {
        let reversed = [0.0 - direction[0], 0.0 - direction[1], 0.0 - direction[2]];
        let normalized = dotnet_vector::normalize(reversed);
        tracer.cast(normalized, mode);
    }
    tracer.result
}

/// Settings of [`distance_rt`] (`Morph.DistanceRT`).
#[derive(Clone, Copy, Debug)]
pub struct DistanceRtSettings {
    pub mode: DistanceRtMode,
    pub directions: i32,
    pub skew: f32,
    pub angle: f32,
    pub angular_jitter: f32,
    pub falloff: f32,
    pub falloff_jitter: f32,
    pub invert_input: bool,
    pub invert_output: bool,
    pub seed: i32,
}

/// `Morph.DistanceRT`: distance by casting `directions` families of rays.
pub fn distance_rt(map: &Map, settings: &DistanceRtSettings, terrain: (f32, f32)) -> Map {
    let resolution = map.width();
    let data: Vec<f32> = if settings.invert_input {
        map.to_interleaved()
    } else {
        map.data().par_iter().map(|&value| 1.0 - value).collect()
    };

    let mut angle = settings.angle / 180.0 * std::f32::consts::PI;
    let spread = std::f32::consts::PI * 2.0 / settings.directions as f32;
    let mut random = XorShift64Star::new(settings.seed);
    let (reference_sine, reference_cosine) = maths::sin_cos(angle);
    let mut directions = Vec::with_capacity(settings.directions.max(0) as usize);
    for _ in 0..settings.directions {
        let jittered = angle + (2.0 * random.next_float() - 1.0) * spread * settings.angular_jitter;
        let falloff = settings.falloff + (random.next_float() - 0.5) * settings.falloff_jitter;
        let (sine, cosine) = maths::sin_cos(jittered);
        let alignment = reference_cosine * cosine + reference_sine * sine;
        let mut weight = maths::exp(settings.skew * alignment);
        weight *= weight;
        let x = cosine * falloff * weight;
        let y = sine * falloff * weight;
        directions.push([x, y, -0.5]);
        angle += spread;
    }

    let mut distances = distance_rays(&data, resolution, &directions, settings.mode, terrain);
    if !settings.invert_output {
        distances.par_iter_mut().for_each(|value| *value = 1.0 - *value);
    }
    Map::from_data(maths::square_resolution(distances.len()), distances)
}
