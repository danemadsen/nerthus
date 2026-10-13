//! The slope-following blur and warp (`SlopeBlur`): samples are traced along
//! the gradient of a guide map, switching to coarser levels of detail as
//! the trace grows longer.

use nerthus_engine::Map;
use rayon::prelude::*;

use crate::maths;

/// `System.Numerics.Vector2`, with its component-wise operations.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Vector2 {
    x: f32,
    y: f32,
}

impl Vector2 {
    const ZERO: Vector2 = Vector2 { x: 0.0, y: 0.0 };

    fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    fn length(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    fn scaled(self, factor: f32) -> Self {
        Self::new(self.x * factor, self.y * factor)
    }

    fn divided(self, divisor: f32) -> Self {
        Self::new(self.x / divisor, self.y / divisor)
    }

    fn plus(self, other: Vector2) -> Self {
        Self::new(self.x + other.x, self.y + other.y)
    }

    fn minus(self, other: Vector2) -> Self {
        Self::new(self.x - other.x, self.y - other.y)
    }

    /// `Vector2.Max(v, Zero)` then `Vector2.Min(v, resolution − 0.5)`.
    fn clamped(self, resolution: usize) -> Self {
        let limit = resolution as f32 - 0.5;
        let floor = |value: f32| if value > 0.0 { value } else { 0.0 };
        let ceiling = |value: f32| if value < limit { value } else { limit };
        Self::new(ceiling(floor(self.x)), ceiling(floor(self.y)))
    }

    fn outside(self, resolution: usize) -> bool {
        let size = resolution as f32;
        self.x < 0.0 || self.x >= size || self.y < 0.0 || self.y >= size
    }
}

/// `SlopeBlur.Window`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Window {
    Bell,
    Box,
}

impl Window {
    pub fn from_name(name: &str) -> Self {
        if name == "Box" { Self::Box } else { Self::Bell }
    }
}

/// One level of detail: the data and guide at a resolution, and the
/// guide's rotated gradient.
struct Lod {
    resolution: usize,
    coefficient: f32,
    step: f32,
    normalized: bool,
    data: Vec<f32>,
    guide: Vec<f32>,
    gradient: Vec<Vector2>,
}

/// `LOD.ReadCheckBoudaries`: a sample clamped to the plane.
fn read_clamped(values: &[f32], resolution: usize, x: i32, y: i32) -> f32 {
    let last = resolution as i32 - 1;
    let x = if x < 0 { 0 } else if x > last { last } else { x };
    let y = if y < 0 { 0 } else if y > last { last } else { y };
    values[(y * resolution as i32 + x) as usize]
}

/// `LOD.Downsample`: the `[1 3 3 1]²/64` filter to half size.
fn downsample(source: &[f32], resolution: usize) -> Vec<f32> {
    let source_resolution = resolution << 1;
    const WEIGHTS: [f32; 4] = [1.0, 3.0, 3.0, 1.0];
    let mut result = vec![0.0f32; resolution * resolution];
    result.par_chunks_mut(resolution).enumerate().for_each(|(row, values)| {
        let y = (row << 1) as i32;
        for (column, value) in values.iter_mut().enumerate() {
            let x = (column << 1) as i32;
            let mut sum = 0.0f32;
            for (row_offset, &row_weight) in WEIGHTS.iter().enumerate() {
                for (column_offset, &column_weight) in WEIGHTS.iter().enumerate() {
                    let sample = read_clamped(source, source_resolution, x - 1 + column_offset as i32, y - 1 + row_offset as i32);
                    let weight = row_weight * column_weight;
                    if row_offset == 0 && column_offset == 0 {
                        sum = weight * sample;
                    } else {
                        sum += weight * sample;
                    }
                }
            }
            *value = sum / 64.0;
        }
    });
    result
}

/// Clamped neighbour indices as the blur computes them: the upper bound is
/// only checked when the lower one did not need clamping.
fn neighbour_range(index: usize, resolution: usize) -> (usize, usize) {
    let mut before = index as i32 - 1;
    let mut after = index as i32 + 1;
    if before < 0 {
        before = 0;
    } else if after > resolution as i32 - 1 {
        after = resolution as i32 - 1;
    }
    (before as usize, after as usize)
}

/// `LOD.CalculateGradient`: the Sobel gradient of `guide`, normalised or
/// scaled by the resolution, rotated by `direction` radians.
fn sobel_gradient(guide: &[f32], resolution: usize, direction: f32, normalized: bool) -> Vec<Vector2> {
    let sine = maths::sin(direction);
    let cosine = maths::cos(direction);
    let mut gradient = vec![Vector2::ZERO; resolution * resolution];
    gradient.par_chunks_mut(resolution).enumerate().for_each(|(y, row)| {
        let (above, below) = neighbour_range(y, resolution);
        for (x, cell) in row.iter_mut().enumerate() {
            let (left, right) = neighbour_range(x, resolution);
            let at = |column: usize, row: usize| guide[row * resolution + column];
            let top_left = at(left, above);
            let top = at(x, above);
            let top_right = at(right, above);
            let middle_left = at(left, y);
            let middle_right = at(right, y);
            let bottom_left = at(left, below);
            let bottom = at(x, below);
            let bottom_right = at(right, below);
            let mut vector = Vector2::new(
                top_right + middle_right + middle_right + bottom_right - top_left - middle_left - middle_left - bottom_left,
                bottom_left + bottom + bottom + bottom_right - top_left - top - top - top_right,
            );
            if normalized {
                let length = vector.length();
                if length > 0.0 {
                    vector = vector.divided(length);
                }
            } else {
                vector = vector.scaled(resolution as f32);
            }
            *cell = Vector2::new(cosine * vector.x - sine * vector.y, sine * vector.x + cosine * vector.y);
        }
    });
    gradient
}

impl Lod {
    fn root(resolution: usize, data: &[f32], guide: &[f32], direction: f32, normalized: bool) -> Self {
        Self {
            resolution,
            coefficient: 1.0,
            step: 1.0,
            normalized,
            data: data.to_vec(),
            guide: guide.to_vec(),
            gradient: sobel_gradient(guide, resolution, direction, normalized),
        }
    }

    fn coarser(&self, direction: f32) -> Self {
        let resolution = self.resolution >> 1;
        let data = downsample(&self.data, resolution);
        let guide = downsample(&self.guide, resolution);
        let gradient = sobel_gradient(&guide, resolution, direction, self.normalized);
        Self {
            resolution,
            coefficient: self.coefficient * 0.5,
            step: self.step * 2.0,
            normalized: self.normalized,
            data,
            guide,
            gradient,
        }
    }

    /// Bilinear corners and fractions at (`x`, `y`) in root pixels.
    fn corners(&self, x: f32, y: f32) -> (usize, usize, usize, usize, f32, f32) {
        let x = x * self.coefficient - 0.5;
        let y = y * self.coefficient - 0.5;
        let left = (x as i32).max(0);
        let top = (y as i32).max(0);
        let right = (left + 1).min(self.resolution as i32 - 1);
        let bottom = (top + 1).min(self.resolution as i32 - 1);
        let fraction_x = x - left as f32;
        let fraction_y = y - top as f32;
        (left as usize, top as usize, right as usize, bottom as usize, fraction_x, fraction_y)
    }

    /// `LOD.Data`: bilinear data.
    fn data_at(&self, x: f32, y: f32) -> f32 {
        let (left, top, right, bottom, fraction_x, fraction_y) = self.corners(x, y);
        let size = self.resolution;
        let data = &self.data;
        let upper = data[size * top + left] * (1.0 - fraction_x) + data[size * top + right] * fraction_x;
        let lower = data[size * bottom + left] * (1.0 - fraction_x) + data[size * bottom + right] * fraction_x;
        upper * (1.0 - fraction_y) + lower * fraction_y
    }

    /// `LOD.Gradient`: the gradient of the cell under (`x`, `y`).
    fn gradient_at(&self, x: f32, y: f32) -> Vector2 {
        let x = x * self.coefficient;
        let y = y * self.coefficient;
        self.gradient[y as i32 as usize * self.resolution + x as i32 as usize]
    }

    /// `LOD.Gradient2`: bilinear gradient, normalised when the LOD is.
    fn gradient_smooth(&self, x: f32, y: f32) -> Vector2 {
        let (left, top, right, bottom, fraction_x, fraction_y) = self.corners(x, y);
        let size = self.resolution;
        let gradient = &self.gradient;
        let upper = gradient[size * top + left].scaled(1.0 - fraction_x).plus(gradient[size * top + right].scaled(fraction_x));
        let lower = gradient[size * bottom + left].scaled(1.0 - fraction_x).plus(gradient[size * bottom + right].scaled(fraction_x));
        let mut result = upper.scaled(1.0 - fraction_y).plus(lower.scaled(fraction_y));
        if self.normalized {
            let length = result.length();
            if length != 0.0 {
                result = result.divided(length);
            }
        }
        result
    }
}

/// The root LOD and every coarser one down to 900 pixels or fewer.
fn lod_chain(resolution: usize, data: &[f32], guide: &[f32], direction: f32, normalized: bool) -> Vec<Lod> {
    let mut chain = vec![Lod::root(resolution, data, guide, direction, normalized)];
    while chain.last().expect("the chain has a root").resolution > 900 {
        let coarser = chain.last().expect("the chain has a root").coarser(direction);
        chain.push(coarser);
    }
    chain
}

/// `SlopeBlur.Starts`: sub-pixel sample offsets for `quality` 0, 1 or 2.
fn starts(quality: i32) -> Vec<Vector2> {
    let rotated_grid = vec![
        Vector2::new(0.625, 0.125),
        Vector2::new(0.125, 0.375),
        Vector2::new(0.875, 0.625),
        Vector2::new(0.375, 0.875),
    ];
    match quality {
        0 => vec![Vector2::new(0.5, 0.5)],
        1 => rotated_grid,
        _ => {
            let count = rotated_grid.len();
            let mut quadrupled = vec![Vector2::ZERO; count * 4];
            for (index, start) in rotated_grid.iter().enumerate() {
                quadrupled[index] = start.scaled(0.5);
                quadrupled[index + count] = Vector2::new(start.x * 0.5 + 0.5, start.y * 0.5);
                quadrupled[index + count * 2] = Vector2::new(start.x * 0.5, start.y * 0.5 + 0.5);
                quadrupled[index + count * 3] = Vector2::new(start.x * 0.5 + 0.5, start.y * 0.5 + 0.5);
            }
            quadrupled
        }
    }
}

/// The bell window: `w = (1 − t)²`, then `w(2 − w)`.
fn bell(t: f32) -> f32 {
    let mut weight = 1.0 - t;
    weight *= weight;
    weight *= 2.0 - weight;
    weight
}

/// Settings of [`blur`].
#[derive(Clone, Copy, Debug)]
pub struct BlurSettings {
    pub intensity: f32,
    pub quality: i32,
    /// Degrees.
    pub direction: f32,
    pub bias: f32,
    pub normalized: bool,
    pub window: Window,
    pub ssaa: i32,
    pub spectral_quality: i32,
    pub iterations: i32,
}

/// The weighted sum and weight a blur trace collects.
#[derive(Default)]
struct Accumulator {
    sum: f32,
    weight: f32,
}

/// `SlopeBlur.Blur`: averages each pixel along the guide's slope, `bias`
/// of the way downhill and the rest uphill.
pub fn blur(data: &[f32], guide: &[f32], resolution: usize, settings: &BlurSettings) -> Vec<f32> {
    let mut current = data.to_vec();
    for _ in 0..settings.iterations {
        current = blur_once(&current, guide, resolution, settings);
    }
    current
}

fn blur_once(data: &[f32], guide: &[f32], resolution: usize, settings: &BlurSettings) -> Vec<f32> {
    let mut intensity = settings.intensity;
    if settings.window == Window::Bell {
        intensity *= 2.0;
    }
    let direction = settings.direction / 180.0 * std::f32::consts::PI;
    let length;
    if settings.normalized {
        length = resolution as f32 * 0.05 * intensity;
        intensity = 1.0;
    } else {
        let octave = maths::pow(2.0, settings.quality as f32);
        length = resolution as f32 / 6.0 * octave * intensity;
        intensity = 0.08 / octave;
    }
    let chain = lod_chain(resolution, data, guide, direction, settings.normalized);
    let starts = starts(settings.ssaa);
    let share = 1.0 / starts.len() as f32;
    let bias = settings.bias * 0.5 + 0.5;
    let tracer = BlurTracer {
        chain: &chain,
        resolution,
        intensity,
        window: settings.window,
        ssaa: settings.ssaa,
        spectral_quality: settings.spectral_quality,
    };

    let mut result = vec![0.0f32; resolution * resolution];
    result.par_chunks_mut(resolution).enumerate().for_each(|(y, row)| {
        for (x, value) in row.iter_mut().enumerate() {
            let mut total = 0.0f32;
            for start in &starts {
                let origin = Vector2::new(x as f32 + start.x, y as f32 + start.y);
                let mut accumulator = Accumulator::default();
                let mut last_sample = 0.0f32;
                tracer.trace(origin, length * bias, bias, true, &mut accumulator, &mut last_sample);
                tracer.trace(origin, length * (1.0 - bias), 1.0 - bias, false, &mut accumulator, &mut last_sample);
                if accumulator.weight != 0.0 {
                    let mean = accumulator.sum / accumulator.weight;
                    total += mean * share;
                } else {
                    total += data[y * resolution + x] * share;
                }
            }
            *value = total;
        }
    });
    result
}

/// One `SlopeBlur.Blur` trace setup.
struct BlurTracer<'a> {
    chain: &'a [Lod],
    resolution: usize,
    intensity: f32,
    window: Window,
    ssaa: i32,
    spectral_quality: i32,
}

impl BlurTracer<'_> {
    /// Walks `distance` along the slope (downhill when `forward`),
    /// collecting LOD data weighted by the window; `first_weight` scales
    /// the first sample.
    fn trace(&self, origin: Vector2, distance: f32, first_weight: f32, forward: bool, accumulator: &mut Accumulator, last_sample: &mut f32) {
        let mut level = 0usize;
        let mut lod = &self.chain[0];
        let mut steps_on_level = 0;
        let mut position = origin;
        let mut previous = Vector2::ZERO;
        let mut left_map = false;
        let mut k = 0i32;
        while (k as f32) < distance {
            let mut weight = if self.window == Window::Bell { bell(k as f32 / distance) } else { 1.0 };
            if k == 0 {
                weight *= first_weight;
            }
            let step = lod.step;
            if previous != position {
                *last_sample = lod.data_at(position.x, position.y);
            }
            accumulator.sum += *last_sample * step * weight;
            accumulator.weight += step * weight;
            previous = position;
            let gradient = if self.ssaa != 0 {
                lod.gradient_smooth(position.x, position.y)
            } else {
                lod.gradient_at(position.x, position.y)
            };
            let move_by = gradient.scaled(self.intensity).scaled(step);
            position = if forward { position.plus(move_by) } else { position.minus(move_by) };
            if position.outside(self.resolution) {
                left_map = true;
                break;
            }
            steps_on_level += 1;
            if steps_on_level > self.spectral_quality {
                steps_on_level = 0;
                if level + 1 < self.chain.len() {
                    level += 1;
                    lod = &self.chain[level];
                }
            }
            k += lod.step as i32;
        }

        if left_map || !(distance > 0.0) {
            return;
        }
        let remainder = (distance - (k - lod.step as i32) as f32) / lod.step;
        let step = lod.step;
        let gradient = lod.gradient_smooth(previous.x, previous.y);
        let position = previous.plus(gradient.scaled(step).scaled(self.intensity).scaled(remainder));
        let position = position.clamped(self.resolution);
        let weight = match self.window {
            Window::Box => 1.0,
            Window::Bell if distance == 0.0 => 0.0,
            Window::Bell if forward => bell(0.5 * remainder / distance),
            Window::Bell => {
                let mut weight = remainder / distance;
                weight *= weight;
                weight *= 2.0 - weight;
                weight
            }
        };
        accumulator.sum += remainder * lod.data_at(position.x, position.y) * weight;
        accumulator.weight += remainder * weight;
    }
}

/// Settings of [`warp`].
#[derive(Clone, Copy, Debug)]
pub struct WarpSettings {
    pub intensity: f32,
    pub iterations: i32,
    /// Degrees.
    pub direction: f32,
    pub normalized: bool,
    pub quality: i32,
    pub ssaa: i32,
    pub spectral_quality: i32,
}

/// `SlopeBlur.Warp`: moves each pixel along the guide's slope (scaled
/// and signed by `custom` when given) and samples the data there.
pub fn warp(data: &[f32], guide: &[f32], custom: Option<&[f32]>, resolution: usize, settings: &WarpSettings) -> Vec<f32> {
    let mut current = data.to_vec();
    for _ in 0..settings.iterations {
        current = warp_once(&current, guide, custom, resolution, settings);
    }
    current
}

fn warp_once(data: &[f32], guide: &[f32], custom: Option<&[f32]>, resolution: usize, settings: &WarpSettings) -> Vec<f32> {
    let direction = settings.direction / 180.0 * std::f32::consts::PI;
    let mut intensity = settings.intensity;
    let length;
    if settings.normalized {
        length = resolution as f32 * 0.04 * intensity;
        intensity = 1.0;
    } else {
        let octave = maths::pow(2.0, settings.quality as f32);
        length = resolution as f32 / 4.0 * octave * intensity;
        intensity = 0.04 / octave;
    }
    let chain = lod_chain(resolution, data, guide, direction, settings.normalized);
    let starts = starts(settings.ssaa);
    let share = 1.0 / starts.len() as f32;
    let spectral_quality = settings.spectral_quality;

    let mut result = vec![0.0f32; resolution * resolution];
    result.par_chunks_mut(resolution).enumerate().for_each(|(y, row)| {
        for (x, value) in row.iter_mut().enumerate() {
            let mut total = 0.0f32;
            for start in &starts {
                let mut level = 0usize;
                let mut lod = &chain[0];
                let mut steps_on_level = 0;
                let mut position = Vector2::new(x as f32 + start.x, y as f32 + start.y);
                let mut previous = position;
                let mut left_map = false;
                let mut distance = length;
                let mut sign = 1.0f32;
                if let Some(custom) = custom {
                    let amount = custom[y * resolution + x] * 2.0 - 1.0;
                    if amount < 0.0 {
                        sign = -1.0;
                    }
                    distance *= amount * sign;
                }
                let mut k = 0i32;
                while (k as f32) < distance {
                    let step = lod.step;
                    let gradient = lod.gradient_smooth(position.x, position.y);
                    position = position.plus(gradient.scaled(step).scaled(intensity).scaled(sign));
                    if position.outside(resolution) {
                        position = position.clamped(resolution);
                        left_map = true;
                        break;
                    }
                    steps_on_level += 1;
                    if steps_on_level > spectral_quality {
                        steps_on_level = 0;
                        if level + 1 < chain.len() {
                            level += 1;
                            lod = &chain[level];
                        }
                    }
                    previous = position;
                    k += lod.step as i32;
                }
                if !left_map {
                    let step = lod.step;
                    let remainder = (distance - (k - lod.step as i32) as f32) / lod.step;
                    let gradient = lod.gradient_smooth(previous.x, previous.y);
                    let moved = previous.plus(gradient.scaled(step).scaled(intensity).scaled(remainder).scaled(sign));
                    position = moved.clamped(resolution);
                }
                total += chain[0].data_at(position.x, position.y) * share;
            }
            *value = total;
        }
    });
    result
}

/// `SlopeBlurCore.Blur`: [`blur`] of every channel of `map` along `guide`.
#[allow(clippy::too_many_arguments)]
pub fn blur_map(map: &Map, guide: &Map, intensity: f32, iterations: i32, direction: f32, asymmetry: f32, normalized: bool, window: Window, quality: i32, antialiasing: i32) -> Map {
    let resolution = map.width();
    let settings = BlurSettings {
        intensity,
        quality,
        direction,
        bias: asymmetry,
        normalized,
        window,
        ssaa: antialiasing,
        spectral_quality: (quality + 1) * 6,
        iterations,
    };
    let channels: Vec<Vec<f32>> = (0..map.channel_count())
        .map(|channel| blur(map.channel(channel), guide.channel(0), resolution, &settings))
        .collect();
    Map::from_channels(map.resolution(), channels)
}

/// `SlopeBlurCore.Warp`: [`warp`] of every channel of `map` along `guide`.
#[allow(clippy::too_many_arguments)]
pub fn warp_map(map: &Map, guide: &Map, intensity: f32, iterations: i32, direction: f32, normalized: bool, quality: i32, antialiasing: i32) -> Map {
    let resolution = map.width();
    let settings = WarpSettings {
        intensity,
        iterations,
        direction,
        normalized,
        quality,
        ssaa: antialiasing,
        spectral_quality: (quality + 1) * 4,
    };
    let channels: Vec<Vec<f32>> = (0..map.channel_count())
        .map(|channel| warp(map.channel(channel), guide.channel(0), None, resolution, &settings))
        .collect();
    Map::from_channels(map.resolution(), channels)
}
