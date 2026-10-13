//! The FastNoiseSIMD-based noises (`RawNoise.Perlin`, `RawNoise.Voronoi`).

use std::f32::consts::PI;

use nerthus_engine::{Map, Resolution};
use nerthus_noise::fast_noise::{CellularDistanceFunction, CellularReturnType, FastNoise, FractalType, NoiseType, PerturbType};
use rayon::prelude::*;

use crate::combiner;
use crate::dotnet_vector;
use crate::gradients::{self, EdgeBehaviour as GradientEdge};
use crate::maths::{self, dotnet_max};
use crate::profiles;
use crate::random::XorShift64Star;
use crate::transformer::EdgeBehaviour;
use crate::warps::{self, FractalWarpSettings};

/// Where a noise is sampled: an offset in noise wavelengths and the part of
/// the unit square the map shows (default vectors for the whole square).
#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub offset: [f32; 2],
    pub top_left: [f32; 2],
    pub bottom_right: [f32; 2],
}

impl Viewport {
    pub fn with_offset(offset: [f32; 2]) -> Self {
        Self {
            offset,
            top_left: [0.0, 0.0],
            bottom_right: [0.0, 0.0],
        }
    }
}

/// Samples `generator` at every pixel through vector sets, row by row, the
/// way RawNoise's noises do, and maps each sample with `finish`.
fn sample_rows(generator: &FastNoise, size: usize, wavelength: f32, viewport: Viewport, finish: impl Fn(f32) -> f32 + Sync) -> Map {
    let mut bottom_right = viewport.bottom_right;
    if viewport.top_left == [0.0, 0.0] && bottom_right == [0.0, 0.0] {
        bottom_right = [1.0, 1.0];
    }
    let size_x = bottom_right[0] - viewport.top_left[0];
    let size_y = bottom_right[1] - viewport.top_left[1];
    let resolution_inverse = 1.0 / size as f32;
    let shift_x = viewport.offset[0] * wavelength * resolution_inverse;
    let shift_y = viewport.offset[1] * wavelength * resolution_inverse;

    let mut data = vec![0.0f32; size * size];
    data.par_chunks_mut(size).enumerate().for_each(|(y, row)| {
        let positions: Vec<[f32; 3]> = (0..size)
            .map(|x| {
                let mut px = x as f32 * resolution_inverse;
                let mut py = y as f32 * resolution_inverse;
                px = viewport.top_left[0] + px * size_x;
                py = viewport.top_left[1] + py * size_y;
                px += shift_x;
                py += shift_y;
                px -= 0.5;
                py -= 0.5;
                px *= size as f32;
                py *= size as f32;
                [px, py, 0.0]
            })
            .collect();
        let samples = generator.fill_vector_set(&positions, [0.0, 0.0, 0.0]);
        for (value, sample) in row.iter_mut().zip(samples) {
            *value = finish(sample);
        }
    });
    Map::from_data(Resolution::square(size), data)
}

/// `NoiseWarpType` → FastNoiseSIMD perturbation.
pub fn perturb_for_warp(warp: i64) -> PerturbType {
    match warp {
        1 => PerturbType::Gradient,
        2 => PerturbType::GradientFractal,
        _ => PerturbType::None,
    }
}

/// Settings of `RawNoise.Perlin`.
#[derive(Clone, Copy, Debug)]
pub struct PerlinSettings {
    pub scale: f32,
    pub octaves: i32,
    pub gain: f32,
    pub fractal: FractalType,
    pub invert: bool,
    pub seed: i32,
    pub warp: PerturbType,
    pub warp_frequency: f32,
    pub warp_amplitude: f32,
    pub warp_octaves: i32,
    pub scale_x: f32,
    pub scale_y: f32,
}

/// `RawNoise.Perlin`: fractal Perlin noise in 0..1.
pub fn perlin(size: usize, settings: &PerlinSettings, viewport: Viewport) -> Map {
    let relative = 512.0 / size as f32;
    let scale = 1.0 - settings.scale;
    let frequency = scale * 0.01 * relative;
    let wavelength = 1.0 / frequency;

    let mut generator = FastNoise::new(settings.seed);
    generator.set_frequency(scale * 0.01 * relative);
    generator.set_axis_scales(settings.scale_x, settings.scale_y, 1.0);
    generator.set_fractal_octaves(settings.octaves);
    generator.set_noise_type(NoiseType::PerlinFractal);
    generator.set_fractal_gain(settings.gain);
    generator.set_fractal_type(settings.fractal);
    generator.set_perturb_type(settings.warp);
    generator.set_perturb_amp(settings.warp_amplitude * 10.0);
    generator.set_perturb_frequency(settings.warp_frequency);
    generator.set_perturb_fractal_octaves(settings.warp_octaves);

    let sign = if settings.invert { -1.0 } else { 1.0 };
    sample_rows(&generator, size, wavelength, viewport, |sample| 0.5 + sample * sign * 0.5)
}

/// `PerlinNoiseType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PerlinType {
    Fbm,
    Ridged,
    Billowy,
}

impl PerlinType {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Ridged" => Self::Ridged,
            "Billowy" => Self::Billowy,
            _ => Self::Fbm,
        }
    }
}

/// `Noises.Perlin`'s parameters, with its defaults.
#[derive(Clone, Copy, Debug)]
pub struct NoisesPerlin {
    pub scale: f32,
    pub octaves: i32,
    pub gain: f32,
    pub kind: PerlinType,
    pub seed: i32,
    /// `NoiseWarpType` as its integer value (0 None, 1 Simple, 2 Complex).
    pub warp_type: i64,
    pub frequency: f32,
    pub amplitude: f32,
    pub warp_octaves: i32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub x: f32,
    pub y: f32,
}

impl Default for NoisesPerlin {
    fn default() -> Self {
        Self {
            scale: 0.5,
            octaves: 10,
            gain: 0.5,
            kind: PerlinType::Fbm,
            seed: 0,
            warp_type: 2,
            frequency: 0.05,
            amplitude: 0.5,
            warp_octaves: 10,
            scale_x: 1.0,
            scale_y: 1.0,
            x: 0.0,
            y: 0.0,
        }
    }
}

/// `Noises.Perlin`: [`perlin`], where "Ridged" is inverted billow noise.
pub fn noises_perlin(size: usize, parameters: &NoisesPerlin) -> Map {
    let fractal = match parameters.kind {
        PerlinType::Fbm => FractalType::Fbm,
        PerlinType::Ridged | PerlinType::Billowy => FractalType::Billow,
    };
    let settings = PerlinSettings {
        scale: parameters.scale,
        octaves: parameters.octaves,
        gain: parameters.gain,
        fractal,
        invert: false,
        seed: parameters.seed,
        warp: perturb_for_warp(parameters.warp_type),
        warp_frequency: parameters.frequency,
        warp_amplitude: parameters.amplitude,
        warp_octaves: parameters.warp_octaves,
        scale_x: parameters.scale_x,
        scale_y: parameters.scale_y,
    };
    let viewport = Viewport::with_offset([parameters.x, parameters.y]);
    let mut map = perlin(size, &settings, viewport);
    if parameters.kind == PerlinType::Ridged {
        crate::maths::invert(&mut map);
    }
    map
}

/// Settings of `RawNoise.Voronoi`.
#[derive(Clone, Copy, Debug)]
pub struct VoronoiSettings {
    pub scale: f32,
    pub function: CellularDistanceFunction,
    pub form: CellularReturnType,
    pub warp: PerturbType,
    pub warp_frequency: f32,
    pub warp_amplitude: f32,
    pub warp_octaves: i32,
    pub seed: i32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub jitter: f32,
}

/// `VoronoiForm` member → FastNoiseSIMD cellular return type.
pub fn voronoi_form(name: &str) -> CellularReturnType {
    match name {
        "C" => CellularReturnType::CellValue,
        "N" => CellularReturnType::NoiseLookup,
        "R" => CellularReturnType::Distance,
        "A" => CellularReturnType::Distance2Add,
        "S" => CellularReturnType::Distance2Sub,
        "M" => CellularReturnType::Distance2Mul,
        "D" => CellularReturnType::Distance2Div,
        _ => CellularReturnType::Distance2,
    }
}

/// `RawNoise.Voronoi`: cellular noise in roughly 0..1.
pub fn voronoi(size: usize, settings: &VoronoiSettings, viewport: Viewport) -> Map {
    let relative = 512.0 / size as f32;
    let frequency = settings.scale * 0.01 * relative;
    let wavelength = 1.0 / frequency;

    let mut generator = FastNoise::new(settings.seed);
    generator.set_frequency(settings.scale * 0.01 * relative);
    generator.set_cellular_noise_lookup_frequency(settings.scale * (512.0 / size as f32));
    generator.set_axis_scales(settings.scale_x, settings.scale_y, 1.0);
    generator.set_cellular_noise_lookup_type(NoiseType::PerlinFractal);
    generator.set_noise_type(NoiseType::Cellular);
    generator.set_cellular_jitter(settings.jitter);
    generator.set_cellular_distance_function(settings.function);
    generator.set_cellular_return_type(settings.form);
    generator.set_perturb_type(settings.warp);
    generator.set_perturb_amp(settings.warp_amplitude * 10.0);
    generator.set_perturb_frequency(settings.warp_frequency);
    generator.set_perturb_fractal_octaves(settings.warp_octaves);

    sample_rows(&generator, size, wavelength, viewport, |sample| 0.5 + sample * 0.5)
}

/// RawNoise's cell hash (FNV-1 style over the cell coordinates).
fn cell_hash(a: i32, b: i32) -> u32 {
    let mixed = (-2_128_831_035i32 ^ a).wrapping_mul(16_777_619);
    (mixed ^ b).wrapping_mul(16_777_619) as u32
}

/// RawNoise's LCG step.
fn cell_random(value: u32) -> u32 {
    1_402_024_253u32.wrapping_mul(value).wrapping_add(586_950_981)
}

const UNIT_16: f32 = 1.0 / 65536.0;

/// Settings of `RawNoise.CellularX`.
#[derive(Clone, Copy, Debug)]
pub struct CellularSettings {
    pub size: f32,
    pub density: i32,
    pub jitter: f32,
    /// `DistanceMetric`: Euclidean-Manhattan, Euclidean-Chebyshev, Minkowski.
    pub distance_metric: i32,
    pub metric_morph: f32,
    pub axes_angular_jitter: f32,
    pub clustering: f32,
    pub anisotropy: f32,
    /// `DistanceFunction`: Hash, DistanceInv, Distance2Inv, DistanceDiff,
    /// DistanceToEdge.
    pub distance_function: i32,
    pub seed: i32,
}

/// The hash of the feature point nearest (`x`, `y`) (`CellularHash`).
fn nearest_feature_hash(x: f32, y: f32, density: i32, seed: i32) -> u32 {
    let cell_x = x as i32;
    let cell_y = y as i32;
    let mut nearest = f32::MAX;
    let mut result = 0u32;
    for row in cell_y - 1..=cell_y + 1 {
        for column in cell_x - 1..=cell_x + 1 {
            let mut value = cell_random(cell_hash(column, row).wrapping_add(seed as u32));
            for _ in 0..density {
                value = cell_random(value);
                let feature_x = column as f32 + (value & 0xFFFF) as f32 * UNIT_16;
                value = cell_random(value);
                let feature_y = row as f32 + (value & 0xFFFF) as f32 * UNIT_16;
                value = cell_random(value);
                let dx = feature_x - x;
                let dy = feature_y - y;
                let distance = dx * dx + dy * dy;
                if distance < nearest {
                    nearest = distance;
                    result = value;
                }
            }
        }
    }
    result
}

/// One sample of the engine's own cellular noise (`RawNoise.Cellular`).
fn cellular_sample(x: f32, y: f32, settings: &CellularSettings) -> f32 {
    let minkowski_power = (settings.metric_morph * 1.53).tan() + 1.0;
    let mut distance_metric = settings.distance_metric;
    if settings.metric_morph == 1.0 && distance_metric == 2 {
        distance_metric = 1;
    }
    let cell_x = x as i32;
    let cell_y = y as i32;
    let mut nearest = f32::MAX;
    let mut second = f32::MAX;
    let mut nearest_value = 0u32;
    let anisotropy = 1.0 + settings.anisotropy * 1.5;
    let reach = 1 + (anisotropy * 1.5) as i32;

    for row in cell_y - reach..=cell_y + reach {
        for column in cell_x - reach..=cell_x + reach {
            let mut value = cell_random(cell_hash(column, row).wrapping_add(settings.seed as u32));
            value = cell_random(value);
            let center_x = 0.5 + column as f32;
            value = cell_random(value);
            let center_y = 0.5 + row as f32;
            for _ in 0..settings.density {
                value = cell_random(value);
                let feature_x = center_x + ((value & 0xFFFF) as f32 * UNIT_16 - 0.5) * settings.jitter;
                value = cell_random(value);
                let feature_y = center_y + ((value & 0xFFFF) as f32 * UNIT_16 - 0.5) * settings.jitter;
                value = cell_random(value);

                let (mut dx, mut dy);
                if settings.axes_angular_jitter > 0.0 {
                    let angle_hash = if settings.clustering > 1.0 {
                        nearest_feature_hash(feature_x / settings.clustering, feature_y / settings.clustering, 1, settings.seed + 1)
                    } else {
                        value
                    };
                    value = cell_random(value);
                    let angle = (angle_hash & 0xFFFF) as f32 * 9.58738e-5 * settings.axes_angular_jitter;
                    let (sine, cosine) = crate::maths::sin_cos(angle);
                    let negative_sine = 0.0 - sine;
                    dx = (feature_x - x) * cosine + (feature_y - y) * sine;
                    dy = (feature_x - x) * negative_sine + (feature_y - y) * cosine;
                } else {
                    dx = feature_x - x;
                    dy = feature_y - y;
                }
                dy *= anisotropy;
                dx /= anisotropy;

                let distance = match distance_metric {
                    0 => {
                        let euclidean = (dx * dx + dy * dy).sqrt();
                        let manhattan = dx.abs() + dy.abs();
                        settings.metric_morph * manhattan + (1.0 - settings.metric_morph) * euclidean
                    }
                    1 => {
                        let euclidean = (dx * dx + dy * dy).sqrt();
                        let chebyshev = dx.abs().max(dy.abs());
                        settings.metric_morph * chebyshev + (1.0 - settings.metric_morph) * euclidean
                    }
                    2 => {
                        let sum = crate::maths::pow(dx.abs(), minkowski_power) + crate::maths::pow(dy.abs(), minkowski_power);
                        crate::maths::pow(sum, 1.0 / minkowski_power)
                    }
                    _ => 1.0,
                };
                if distance < nearest {
                    second = nearest;
                    nearest = distance;
                    nearest_value = value;
                } else if distance < second {
                    second = distance;
                }
            }
        }
    }

    match settings.distance_function {
        0 => (nearest_value & 0xFFFF) as f32 * (1.0 / 65535.0),
        1 => 1.0 - nearest * 0.5,
        2 => 1.0 - second * 0.5,
        3 => second - nearest,
        4 => 2.0 * nearest / (nearest + second),
        _ => 0.0,
    }
}

/// `RawNoise.CellularX`: the engine's own cellular noise over a square map.
pub fn cellular(size: usize, settings: &CellularSettings) -> Map {
    let feature_size = settings.size.max(1e-6);
    let scale = 1.0 / (feature_size * size as f32);
    let mut data = vec![0.0f32; size * size];
    data.par_chunks_mut(size).enumerate().for_each(|(y, row)| {
        for (x, value) in row.iter_mut().enumerate() {
            *value = cellular_sample(x as f32 * scale, y as f32 * scale, settings);
        }
    });
    Map::from_data(Resolution::square(size), data)
}

/// `RawNoise.poisson3`: how many Gabor impulses a cell holds.
fn impulse_count(hash: u32) -> usize {
    let value = cell_random(hash.wrapping_add(7)) & 0xFFF;
    match value {
        0..=671 => 1,
        672..=1672 => 2,
        1673..=2673 => 3,
        2674..=3424 => 4,
        3425..=3871 => 5,
        _ => 6,
    }
}

/// `RawNoise.gaborKernel`: a sine wave along (`dx`, `dy`) under a radial
/// window, adding the window to `weight_sum`.
fn gabor_kernel(x: f32, y: f32, wavelength: f32, dx: f32, dy: f32, weight_sum: &mut f32) -> f32 {
    let distance = (x * x + y * y).sqrt();
    let inside = 1.0 - distance;
    let mut window = inside * inside;
    window *= 2.0 - inside;
    let wave = maths::sin((x * dx + y * dy) / wavelength * 0.5 * (std::f32::consts::PI * 2.0));
    *weight_sum += window;
    wave * window
}

/// `RawNoise.gradient`: Sobel gradients of a square plane, tiny components
/// zeroed.
fn sobel_gradients(data: &[f32], resolution: usize) -> Vec<[f32; 2]> {
    let mut gradients = vec![[0.0f32; 2]; resolution * resolution];
    gradients.par_chunks_mut(resolution).enumerate().for_each(|(row, values)| {
        let mut above = row as i32 - 1;
        let mut below = row as i32 + 1;
        if above < 0 {
            above = 0;
        } else if below > resolution as i32 - 1 {
            below = resolution as i32 - 1;
        }
        let (above, below) = (above as usize, below as usize);
        for (column, value) in values.iter_mut().enumerate() {
            let mut left = column as i32 - 1;
            let mut right = column as i32 + 1;
            if left < 0 {
                left = 0;
            } else if right > resolution as i32 - 1 {
                right = resolution as i32 - 1;
            }
            let (left, right) = (left as usize, right as usize);
            let above_left = data[above * resolution + left];
            let above_middle = data[above * resolution + column];
            let above_right = data[above * resolution + right];
            let middle_left = data[row * resolution + left];
            let middle_right = data[row * resolution + right];
            let below_left = data[below * resolution + left];
            let below_middle = data[below * resolution + column];
            let below_right = data[below * resolution + right];
            let mut x = above_right + middle_right + middle_right + below_right - above_left - middle_left - middle_left - below_left;
            let mut y = above_left + above_middle + above_middle + above_right - below_left - below_middle - below_middle - below_right;
            if x.abs() < 1e-5 {
                x = 0.0;
            }
            if y.abs() < 1e-5 {
                y = 0.0;
            }
            *value = [x, y];
        }
    });
    gradients
}

/// Settings of [`gabor`] (`RawNoise.Gabor`).
#[derive(Clone, Copy, Debug)]
pub struct GaborSettings {
    pub size: f32,
    pub entropy: f32,
    pub anisotropy: f32,
    /// Degrees.
    pub direction: f32,
    pub seed: i32,
}

/// `RawNoise.Gabor` at one point (in cell units).
#[allow(clippy::too_many_arguments)]
fn gabor_sample(x: f32, y: f32, wavelength: f32, anisotropy: f32, direction: f32, seed: i32, field: Option<&[[f32; 2]]>, resolution: usize, raster: f32) -> f32 {
    let cell_x = x as i32;
    let cell_y = y as i32;
    let mut sum = 0.0f32;
    let mut weight_sum = 0.0f32;
    for row in cell_y - 1..=cell_y + 1 {
        for column in cell_x - 1..=cell_x + 1 {
            let hashed = (cell_hash(column, row) as i32).wrapping_add(seed) as u32;
            let count = impulse_count(hashed);
            let mut state = cell_random(hashed);
            for _ in 0..count {
                state = cell_random(state);
                let impulse_x = column as f32 + (state & 0xFFFF) as f32 * UNIT_16;
                state = cell_random(state);
                let impulse_y = row as f32 + (state & 0xFFFF) as f32 * UNIT_16;
                let mut unaligned = false;
                let (along_y, along_x) = match field {
                    None => (1.0, 0.0),
                    Some(field) => {
                        let sample_x = ((impulse_x * raster) as i32).min(resolution as i32 - 1).max(0) as usize;
                        let sample_y = ((impulse_y * raster) as i32).min(resolution as i32 - 1).max(0) as usize;
                        let [gradient_x, gradient_y] = field[sample_y * resolution + sample_x];
                        if gradient_x != 0.0 || gradient_y != 0.0 {
                            let length = (gradient_x * gradient_x + gradient_y * gradient_y).sqrt();
                            (gradient_y / length, gradient_x / length)
                        } else {
                            unaligned = true;
                            (1.0, 0.0)
                        }
                    }
                };
                state = cell_random(state);
                let mut angle = (state & 0xFFFF) as f32 * UNIT_16 * 2.0 - 1.0;
                if !unaligned {
                    angle *= std::f32::consts::PI * (1.0 - anisotropy);
                }
                angle += direction;
                let (sine, cosine) = maths::sin_cos(angle);
                let dx = cosine * along_y - sine * along_x;
                let dy = sine * along_y + cosine * along_x;
                let offset_x = impulse_x - x;
                let offset_y = impulse_y - y;
                if offset_x * offset_x + offset_y * offset_y < 1.0 {
                    sum += gabor_kernel(x - impulse_x, y - impulse_y, wavelength, dx, dy, &mut weight_sum);
                }
            }
        }
    }
    weight_sum = maths::dotnet_max(weight_sum, 1e-8);
    sum / weight_sum
}

/// `RawNoise.Gabor`: sparse Gabor noise in 0..1, its orientation following
/// the gradients of `modulation` (blurred) when given.
pub fn gabor(resolution: usize, settings: &GaborSettings, modulation: Option<&[f32]>) -> Map {
    let size = maths::dotnet_max(settings.size, 1e-6);
    let direction = settings.direction * (std::f32::consts::PI / 180.0);
    let entropy = maths::dotnet_max(settings.entropy, 0.0001);
    let coefficient = 1.0 / (size * resolution as f32) * entropy;
    let field = modulation.map(|modulation| {
        let blurred = crate::blur::box_blur(modulation, resolution, 0.4 * resolution as f32 * size, 3);
        sobel_gradients(&blurred, resolution)
    });
    let raster = 1.0 / coefficient;
    let mut data = vec![0.0f32; resolution * resolution];
    data.par_chunks_mut(resolution).enumerate().for_each(|(row, values)| {
        for (column, value) in values.iter_mut().enumerate() {
            let sample = gabor_sample(column as f32 * coefficient, row as f32 * coefficient, entropy, settings.anisotropy, direction, settings.seed, field.as_deref(), resolution, raster);
            let total = 0.0 + sample * 1.0;
            let weight = 0.0 + 1.0;
            *value = total / weight * 0.5 + 0.5;
        }
    });
    Map::from_data(Resolution::square(resolution), data)
}

/// `CrackStyle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrackStyle {
    Normal,
    Hard,
    Classic,
}

impl CrackStyle {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Normal" => Self::Normal,
            "Classic" => Self::Classic,
            _ => Self::Hard,
        }
    }
}

/// Settings of [`cracks`] (`Noises.Cracks`).
#[derive(Clone, Copy, Debug)]
pub struct CrackSettings {
    pub octaves: i32,
    pub scale: f32,
    pub seed: i32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub jitter: f32,
    pub warp_size: f32,
    pub warp_strength: f32,
    pub style: CrackStyle,
    pub depth: f32,
}

/// `Noises.Cracks`: the maximum of warped Voronoi edge maps, cut down to
/// the cracks and, in the Normal style, softened by a distance transform.
pub fn cracks(resolution: Resolution, settings: &CrackSettings, terrain: (f32, f32)) -> Map {
    let size = resolution.x;
    let mut map = Map::new(resolution, 1);
    for octave in 0..settings.octaves {
        let voronoi_settings = VoronoiSettings {
            scale: settings.scale,
            function: CellularDistanceFunction::Euclidean,
            form: CellularReturnType::Distance2Div,
            warp: PerturbType::None,
            warp_frequency: 0.0,
            warp_amplitude: 0.0,
            warp_octaves: 0,
            seed: settings.seed + octave * 2,
            scale_x: settings.scale_x,
            scale_y: settings.scale_y,
            jitter: settings.jitter,
        };
        let mut layer = voronoi(size, &voronoi_settings, Viewport::with_offset([0.0, 0.0]));
        if settings.warp_size > 0.0 && settings.warp_strength > 0.0 {
            let warp = crate::warps::FractalWarpSettings {
                size: settings.warp_size,
                strength: settings.warp_strength,
                noise_type: crate::warps::WarpNoiseType::VoronoiR,
                perturbation: 0.85,
                seed: settings.seed + octave,
                ..crate::warps::FractalWarpSettings::default()
            };
            layer = crate::warps::fractal_warp(&layer, &warp, None, terrain.0, terrain.1);
        }
        map = crate::combiner::max(&map, &layer, 1.0);
    }
    let depth = 0.0 + settings.depth * (2.0 - 0.0);
    let low = 1.0 - 0.3 * (depth / settings.octaves as f32);
    map = maths::clip_map(&map, low, 1.0);
    map = maths::autolevel(map);
    match settings.style {
        CrackStyle::Normal => {
            let lines = crate::aspect::threshold(&map, 0.5);
            let options = crate::morphology::DistanceOptions {
                falloff: size as f32 * 0.5,
                threshold: 0.5,
                invert_input: true,
                invert_output: false,
                clamp: true,
            };
            let distances = crate::morphology::distance_transform(lines.data(), size, options);
            maths::autolevel(Map::from_data(resolution, distances))
        }
        CrackStyle::Hard => crate::aspect::threshold(&map, 0.5),
        CrackStyle::Classic => map,
    }
}

/// Settings of [`drift_noise`] (`Noises.DriftNoise`).
#[derive(Clone, Copy, Debug)]
pub struct DriftSettings {
    pub passes: i32,
    pub chaos: f32,
    pub scale_large: f32,
    pub scale_small: f32,
    pub turbulence_large: f32,
    pub turbulence_small: f32,
    pub wind_direction_large: f32,
    pub wind_direction_small: f32,
    pub overlap_shapes: bool,
    pub seed: i32,
}

/// `Noises.DriftNoise`: wind-blown ramps warped into drifts and recurved
/// into dunes, small and (over several passes) large.
pub fn drift_noise(size: usize, settings: &DriftSettings, terrain: (f32, f32)) -> Map {
    let seed = settings.seed;
    let mut random = XorShift64Star::new(seed);
    let edges = if settings.overlap_shapes { GradientEdge::Repeat } else { GradientEdge::Mirror };
    let center = size as f32 / 2.0;
    let small_length = size as f32 * (settings.scale_small * 0.1);
    let mut small = gradients::linear(size, center, center, small_length, settings.wind_direction_small, edges);
    let small_warp = FractalWarpSettings {
        size: settings.scale_small,
        strength: settings.turbulence_small * 1.15,
        edges: EdgeBehaviour::Mirror,
        seed: seed + 1,
        ..FractalWarpSettings::default()
    };
    small = warps::fractal_warp(&small, &small_warp, None, terrain.0, terrain.1);
    small = maths::fit_range(&small, 0.0, 0.02);
    let duration = if settings.overlap_shapes { 2 } else { 10 };
    small = profiles::recurve(&small, 1.0, duration, false, false, 0.05);
    if settings.passes <= 1 {
        return small;
    }

    let mut large: Option<Map> = None;
    let large_length = size as f32 * (settings.scale_large * 0.25);
    for pass in 0..settings.passes {
        let turn = if pass == 0 { 1.0 } else { (0.5 + random.next_float()) * settings.chaos };
        let turbulence = if pass == 0 { 0.0 } else { (0.5 + random.next_float()) * settings.chaos };
        let ramp = gradients::linear(size, center, center, large_length, settings.wind_direction_large + turn, edges);
        let warp = FractalWarpSettings {
            size: settings.scale_large + 0.15,
            strength: turbulence + settings.turbulence_large * 2.0,
            roughness: 0.4,
            edges: EdgeBehaviour::Mirror,
            seed: seed + 2 + pass,
            ..FractalWarpSettings::default()
        };
        let warped = warps::fractal_warp(&ramp, &warp, None, terrain.0, terrain.1);
        let drift = maths::fit_range(&warped, 0.0, 0.02);
        large = Some(match large {
            None => drift,
            Some(previous) => combiner::min(&previous, &drift, 1.0),
        });
    }
    let large = large.expect("at least two passes ran");
    let large = profiles::recurve(&large, 1.0, 3, false, true, 0.1);
    combiner::min(&large, &small, 1.0)
}

/// `FNSIMD`'s default seed.
const FNSIMD_DEFAULT_SEED: i32 = 1337;

/// Settings of [`wave_shine`] (`Noises.WaveShine`), with its defaults.
#[derive(Clone, Copy, Debug)]
pub struct WaveShineSettings {
    pub frame: i32,
    pub octaves: i32,
    pub seed: i32,
    pub animation_speed: f32,
    pub size: f32,
    pub amplitude: f32,
    pub direction: f32,
    pub speed: f32,
    pub wind_patch_intensity: f32,
    pub wind_patch_animation_speed: f32,
    pub wind_patch_size: f32,
    pub contrast: f32,
    pub brightness: f32,
    pub sun_azimuth: f32,
    pub sun_elevation: f32,
}

impl Default for WaveShineSettings {
    fn default() -> Self {
        Self {
            frame: 0,
            octaves: 4,
            seed: 0,
            animation_speed: 1.0,
            size: 0.1,
            amplitude: 0.5,
            direction: 0.0,
            speed: 0.1,
            wind_patch_intensity: 0.0,
            wind_patch_animation_speed: 1.0,
            wind_patch_size: 30.0,
            contrast: 2.0,
            brightness: 1.0,
            sun_azimuth: 315.0,
            sun_elevation: 30.0,
        }
    }
}

/// `CalculateShine`: specular highlights of the sun on a water surface,
/// from Sobel normals and the half vector between the sun and straight up.
fn calculate_shine(data: &[f32], size: usize, brightness: f32, sun_azimuth: f32, sun_elevation: f32, terrain: (f32, f32)) -> Vec<f32> {
    let brightness = brightness * 5.6;
    let (azimuth_sine, azimuth_cosine) = maths::sin_cos(PI / 180.0 * sun_azimuth);
    let elevation_tangent = maths::tan(PI / 180.0 * sun_elevation);
    let sun = dotnet_vector::normalize([azimuth_sine, 0.0 - azimuth_cosine, elevation_tangent]);
    let up = [0.0, 0.0, 1.0];
    let normal_scale = [0.0 - terrain.1, terrain.1, terrain.0 / size as f32];
    let bisector = dotnet_vector::normalize(dotnet_vector::add(sun, up));
    let last = size - 1;

    let mut shine = vec![0.0f32; data.len()];
    shine.par_chunks_mut(size).enumerate().for_each(|(y, values)| {
        let mut above = y.wrapping_sub(1);
        let mut below = y + 1;
        if y == 0 {
            above = 0;
        } else if below > last {
            below = last;
        }
        for (x, value) in values.iter_mut().enumerate() {
            let mut left = x.wrapping_sub(1);
            let mut right = x + 1;
            if x == 0 {
                left = 0;
            } else if right > last {
                right = last;
            }
            let top_left = data[above * size + left];
            let top = data[above * size + x];
            let top_right = data[above * size + right];
            let middle_left = data[y * size + left];
            let middle_right = data[y * size + right];
            let bottom_left = data[below * size + left];
            let bottom = data[below * size + x];
            let bottom_right = data[below * size + right];
            let slope_x = top_right + middle_right + middle_right + bottom_right - top_left - middle_left - middle_left - bottom_left;
            let slope_y = top_left + top + top + top_right - bottom_left - bottom - bottom - bottom_right;
            let scaled = dotnet_vector::multiply([slope_x, slope_y, 8.0], normal_scale);
            let normal = dotnet_vector::normalize(scaled);
            let mut highlight = dotnet_max(dotnet_vector::dot(normal, bisector), 0.0);
            for _ in 0..6 {
                highlight *= highlight;
            }
            *value = highlight * brightness;
        }
    });
    shine
}

/// `Noises.WaveShine`: sun glints on rippling water. Octaves of Perlin
/// ripples (optionally modulated by a wind-patch noise) drift with the
/// frame number; the summed surface is shaded by [`calculate_shine`].
pub fn wave_shine(size: usize, settings: &WaveShineSettings, terrain: (f32, f32)) -> Map {
    let wavelength = 0.1 * settings.size * size as f32;
    let mut ripples = FastNoise::new(FNSIMD_DEFAULT_SEED);
    ripples.set_noise_type(NoiseType::Perlin);
    ripples.set_axis_scales(1.0, 1.0, 0.02 * settings.animation_speed * wavelength);
    let mut amplitude = 0.5 * settings.amplitude * wavelength / size as f32 * terrain.0 / terrain.1;
    let mut frequency = 1.0 / wavelength;
    let frame = settings.frame;
    let tile_scale = (size / 512) as f32;
    let (direction_sine, direction_cosine) = maths::sin_cos(settings.direction / 180.0 * PI);
    let drift_x = direction_sine * settings.speed;
    let drift_y = (0.0 - direction_cosine) * settings.speed;
    let mut offset_x = (0.0 - frame as f32) * (drift_x * tile_scale * wavelength * 0.2);
    let mut offset_y = (0.0 - frame as f32) * (drift_y * tile_scale * wavelength * 0.2);
    amplitude /= (maths::pow(0.5, settings.octaves as f32) - 1.0) / -0.5;

    let wind_effect = settings.wind_patch_intensity;
    let mut wind_patch = vec![0.0f32; size * size];
    if wind_effect > 0.0 {
        let mut wind = FastNoise::new(FNSIMD_DEFAULT_SEED);
        wind.set_fractal_type(FractalType::Fbm);
        wind.set_noise_type(NoiseType::PerlinFractal);
        wind.set_perturb_type(PerturbType::Gradient);
        wind.set_perturb_frequency(1.0);
        wind.set_perturb_amp(0.25);
        wind.set_frequency(frequency / settings.wind_patch_size);
        wind.set_seed(settings.seed);
        let wind_time_scale = 0.01 * settings.wind_patch_animation_speed * wavelength * settings.wind_patch_size;
        wind.set_axis_scales(1.0, 1.0, wind_time_scale);
        let contrast = 1.0 + 2.0 * settings.contrast;
        wind_patch.par_chunks_mut(size).enumerate().for_each(|(row, values)| {
            wind.fill_noise_set(values, 0, row as i32, frame, size, 1, 1, 1.0);
            for value in values.iter_mut() {
                let shaped = maths::tanh(*value * contrast);
                *value = 1.0 - wind_effect + wind_effect * (shaped * 0.5 + 0.5);
            }
        });
    }

    let rows: Vec<Vec<[f32; 3]>> = (0..size)
        .map(|y| (0..size).map(|x| [x as f32, y as f32, frame as f32]).collect())
        .collect();
    let mut surface = vec![0.0f32; size * size];
    for octave in 0..settings.octaves {
        ripples.set_frequency(frequency);
        ripples.set_seed(octave + settings.seed);
        let offset = [offset_x, offset_y, 0.0];
        surface.par_chunks_mut(size).enumerate().for_each(|(row, values)| {
            let noise = ripples.fill_vector_set(&rows[row], offset);
            for (column, value) in values.iter_mut().enumerate() {
                let mut wave = noise[column];
                if wind_effect > 0.0 {
                    wave *= wind_patch[row * size + column];
                }
                wave += 1.0;
                wave *= wave;
                wave *= 0.5;
                wave -= 0.5;
                wave *= amplitude;
                *value += wave;
            }
        });
        amplitude *= 0.5;
        frequency += frequency;
        offset_x *= 0.5;
        offset_y *= 0.5;
    }

    let shine = calculate_shine(&surface, size, settings.brightness, settings.sun_azimuth, settings.sun_elevation, terrain);
    Map::from_data(Resolution::square(size), shine)
}
