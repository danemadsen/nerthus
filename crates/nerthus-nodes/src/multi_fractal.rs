//! The multifractal noise (`RawNoise.MultiFractal3`): Perlin octaves
//! whose amplitudes are modulated by a secondary fractal (or by
//! themselves), each octave on its own rotated axes, optionally domain
//! warped.

use nerthus_engine::{Map, Resolution};
use nerthus_noise::fast_noise::{FastNoise, NoiseType, PerturbType};
use rayon::prelude::*;

use crate::maths::{self, dotnet_max};

/// FastNoiseSIMD's default seed (`new FNSIMD()`).
const DEFAULT_SEED: i32 = 1337;
// FastNoise's truncated literals, not the exact constants.
#[allow(clippy::approx_constant)]
const LOG2_E: f64 = 1.44269504088896;
const DEGREES_TO_RADIANS: f64 = 0.0174532925199433;
#[allow(clippy::approx_constant)]
const QUARTER_TURN: f64 = 1.5707963267949;

/// `MultifractalNoiseType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultifractalType {
    Fbm,
    Billowy,
    Ridged,
}

impl MultifractalType {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Billowy" => Self::Billowy,
            "Ridged" => Self::Ridged,
            _ => Self::Fbm,
        }
    }
}

/// `VariationType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VariationType {
    SecondaryFractal,
    SelfModulation,
}

impl VariationType {
    pub fn from_name(name: &str) -> Self {
        if name == "SelfModulation" { Self::SelfModulation } else { Self::SecondaryFractal }
    }
}

/// Parameters of [`multi_fractal`] (`Noises.MultiFractal`'s defaults).
#[derive(Clone, Copy, Debug)]
pub struct MultiFractalSettings {
    pub noise_type: MultifractalType,
    /// 0 picks the octave count from the size.
    pub octaves: i32,
    pub size: f32,
    pub feature_size: f32,
    pub fractal_gain: f32,
    pub edge_smoothing: f32,
    pub seed: i32,
    pub variation_type: VariationType,
    pub variation: f32,
    pub smoothness: f32,
    pub variation_contrast: f32,
    pub damping: f32,
    pub bias: f32,
    /// `Perturb`: 0 none, 1 simple, 2 complex.
    pub perturb: i32,
    pub relative_warp_size: f32,
    pub warp_strength: f32,
    pub warp_complexity: i32,
    pub warp_roughness: f32,
    pub warp_attenuation: f32,
    pub warp_iterations: i32,
    pub anisotropy: f32,
    pub relative_warp_anisotropy: f32,
    pub rotation: f32,
    pub offset: [f32; 2],
    pub top_left: [f32; 2],
    pub bottom_right: [f32; 2],
}

impl Default for MultiFractalSettings {
    fn default() -> Self {
        Self {
            noise_type: MultifractalType::Fbm,
            octaves: 10,
            size: 0.5,
            feature_size: 0.5,
            fractal_gain: 0.5,
            edge_smoothing: 0.1,
            seed: 0,
            variation_type: VariationType::SecondaryFractal,
            variation: 2.0,
            smoothness: 0.0,
            variation_contrast: 0.3,
            damping: 0.25,
            bias: 0.5,
            perturb: 0,
            relative_warp_size: 1.0,
            warp_strength: 0.5,
            warp_complexity: 3,
            warp_roughness: 0.4,
            warp_attenuation: 0.5,
            warp_iterations: 3,
            anisotropy: 0.0,
            relative_warp_anisotropy: 1.0,
            rotation: 0.0,
            offset: [0.0, 0.0],
            top_left: [0.0, 0.0],
            bottom_right: [1.0, 1.0],
        }
    }
}

/// `softAbs`: |x| with a rounded bottom.
fn soft_abs(x: f32, smoothing: f32) -> f32 {
    let mut x = x.abs();
    if x < smoothing {
        x /= smoothing;
        x *= x;
        x *= smoothing;
    } else {
        x = smoothing + (x - smoothing) * 2.0;
    }
    x /= 2.0 - smoothing;
    x + smoothing * 0.2
}

fn shaped(noise_type: MultifractalType, value: f32, smoothing: f32) -> f32 {
    match noise_type {
        MultifractalType::Billowy => 1.5 * soft_abs(value, smoothing) - 0.3,
        MultifractalType::Ridged => 0.3 - 1.5 * soft_abs(value, smoothing),
        MultifractalType::Fbm => value,
    }
}

/// A 2D axis pair (`Vector2` columns of a linear map).
#[derive(Clone, Copy)]
struct Axes {
    x: [f32; 2],
    y: [f32; 2],
}

impl Axes {
    fn apply(&self, point: [f32; 2]) -> [f32; 2] {
        [self.x[0] * point[0] + self.y[0] * point[1], self.x[1] * point[0] + self.y[1] * point[1]]
    }
}

/// The viewport and global axes shared by the sampling positions.
struct Sampling {
    resolution_inverse: [f32; 2],
    viewport_offset: [f32; 2],
    viewport_size: [f32; 2],
    offset: [f32; 2],
    size: f32,
}

impl Sampling {
    /// `SamplingPosition` / `WarpSamplingPosition`.
    fn position(&self, x: f32, y: f32, global: &Axes, octave: &Axes) -> [f32; 3] {
        let mut point = [x * self.resolution_inverse[0], y * self.resolution_inverse[1]];
        point = [
            self.viewport_offset[0] + point[0] * self.viewport_size[0],
            self.viewport_offset[1] + point[1] * self.viewport_size[1],
        ];
        point = [point[0] + self.offset[0] * self.size, point[1] + self.offset[1] * self.size];
        point = [point[0] - 0.5, point[1] - 0.5];
        point = global.apply(point);
        point = octave.apply(point);
        [point[0], point[1], 0.0]
    }
}

/// The two anisotropy-stretched, rotated global axes.
fn global_axes(stretch: f32, cosine: f32, sine: f32) -> Axes {
    let ratio = [1.0 / stretch, stretch];
    let scale = [2.1639533 * (ratio[0] * 0.5).tanh(), 2.1639533 * (ratio[1] * 0.5).tanh()];
    Axes {
        x: [cosine * scale[0], sine * scale[1]],
        y: [sine * scale[0], (0.0 - cosine) * scale[1]],
    }
}

/// `RawNoise.MultiFractal3`.
pub fn multi_fractal(resolution: Resolution, settings: &MultiFractalSettings) -> Map {
    let width = resolution.x;
    let height = resolution.y;
    let size = dotnet_max(settings.size, 1e-7);
    let relative_warp_size = dotnet_max(settings.relative_warp_size, 1e-7);
    let feature_size = settings.feature_size * size;
    let edge_smoothing = settings.edge_smoothing * 0.5;
    let warp_gain = 1.0 - settings.warp_attenuation * 0.5;
    let sampling = Sampling {
        resolution_inverse: [1.0 / width as f32, 1.0 / height as f32],
        viewport_offset: settings.top_left,
        viewport_size: [settings.bottom_right[0] - settings.top_left[0], settings.bottom_right[1] - settings.top_left[1]],
        offset: settings.offset,
        size,
    };
    let stretch = maths::exp(settings.anisotropy * 4.0);
    let warp_stretch = maths::exp(settings.anisotropy * settings.relative_warp_anisotropy * 4.0);
    let angle = settings.rotation / 180.0 * std::f32::consts::PI;
    let cosine = maths::cos(angle);
    let sine = maths::sin(angle);
    let global = global_axes(stretch, cosine, sine);
    let global_warp = global_axes(warp_stretch, cosine, sine);

    let mut octaves = settings.octaves;
    if octaves == 0 {
        octaves = (((size * width as f32) as f64).ln() * LOG2_E) as i32 + 1;
    }
    let octave_span = (dotnet_max(size / feature_size, 1.0) as f64).ln() * LOG2_E;
    let octave_span = octave_span as f32;
    let feature_octave = octave_span as i32;
    let feature_coefficient = 1.0 - (octave_span - feature_octave as f32);
    let feature_gain = ((1.0 - (octave_span - feature_octave as f32)) as f64 * (settings.fractal_gain as f64).ln()).exp() as f32;

    let warp_size = relative_warp_size * size;
    let mut warp_strength = settings.warp_strength * (warp_size * width as f32);
    let warp_iterations = settings.warp_iterations as f32;
    if warp_iterations > 1.0 {
        warp_strength /= warp_iterations.sqrt();
    }
    warp_strength *= 2.0;

    let warping = settings.perturb > 0;
    let warp_octaves = if settings.perturb == 2 { settings.warp_complexity } else { settings.perturb };
    let identity = Axes {
        x: [1.0, 0.0],
        y: [0.0, 1.0],
    };
    let mut warp_vectors = vec![[0.0f32; 2]; width * height];
    if warping {
        warp_vectors.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
            let make = || {
                let mut noise = FastNoise::new(DEFAULT_SEED);
                noise.set_noise_type(NoiseType::PerlinFractal);
                noise.set_perturb_type(PerturbType::None);
                noise.set_frequency(1.0 / warp_size);
                noise.set_fractal_gain(settings.warp_roughness);
                noise.set_fractal_octaves(warp_octaves);
                noise
            };
            let mut noise_x = make();
            let mut noise_y = make();
            let mut seed = settings.seed + 512;
            let positions: Vec<[f32; 3]> = (0..width)
                .map(|x| sampling.position(x as f32, y as f32, &global_warp, &identity))
                .collect();
            let mut iteration = 0;
            while (iteration as f32) < warp_iterations {
                noise_x.set_seed(seed);
                seed += 1;
                noise_y.set_seed(seed);
                seed += 1;
                let values_x = noise_x.fill_vector_set(&positions, [0.0; 3]);
                let values_y = noise_y.fill_vector_set(&positions, [0.0; 3]);
                for (x, vector) in row.iter_mut().enumerate() {
                    vector[0] += values_x[x] * warp_strength;
                    vector[1] += values_y[x] * warp_strength;
                }
                iteration += 1;
            }
        });
    }

    let axes: Vec<Axes> = (0..=octaves)
        .map(|octave| {
            let angle = (octave as f32 * 64.54) as f64 * DEGREES_TO_RADIANS;
            let turned = (octave as f32 * 64.54) as f64 * DEGREES_TO_RADIANS + QUARTER_TURN;
            Axes {
                x: [angle.cos() as f32, angle.sin() as f32],
                y: [turned.cos() as f32, turned.sin() as f32],
            }
        })
        .collect();
    let contrast_coefficient = if settings.variation_contrast != 0.0 {
        1.0 / ((settings.variation_contrast as f64).exp() - 1.0) as f32
    } else {
        1.0
    };

    let mut values = vec![0.0f32; width * height];
    values.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
        let mut amplitude = 1.0f32;
        let mut total = 0.0f32;
        let mut frequency = 1.0 / size;
        let mut warp_scale = 1.0f32;
        let mut variation_seed = settings.seed + 257;
        let mut primary = FastNoise::new(DEFAULT_SEED);
        primary.set_noise_type(NoiseType::Perlin);
        primary.set_perturb_type(PerturbType::None);
        let mut secondary = FastNoise::new(DEFAULT_SEED);
        secondary.set_noise_type(NoiseType::Perlin);
        secondary.set_perturb_type(PerturbType::None);
        let mut modulation = vec![1.0f32; width];
        let mut positions: Vec<[f32; 3]> = (0..width)
            .map(|x| {
                let warp = warp_vectors[y * width + x];
                sampling.position(x as f32 + warp[0], y as f32 + warp[1], &global, &axes[0])
            })
            .collect();
        // Each octave takes the next seed.
        for (octave, seed) in (0..octaves).zip(settings.seed..) {
            let octave_axes = axes[octave as usize + 1];
            let octave_frequency = if settings.noise_type == MultifractalType::Fbm { frequency } else { 0.5 * frequency };
            primary.set_frequency(octave_frequency);
            primary.set_seed(seed);
            let noise = primary.fill_vector_set(&positions, [0.0; 3]);
            let variation_noise = if settings.variation_type == VariationType::SecondaryFractal {
                secondary.set_frequency(octave_frequency);
                secondary.set_seed(variation_seed);
                variation_seed += 1;
                secondary.fill_vector_set(&positions, [0.0; 3])
            } else {
                noise.clone()
            };
            warp_scale *= warp_gain;
            for x in 0..width {
                let mut variation = shaped(settings.noise_type, variation_noise[x], edge_smoothing);
                variation = variation * 0.5 + 0.5;
                variation = if !(settings.variation > 0.0) {
                    (((-0.3f32 * variation) as f64).exp() - 1.0) as f32 * -3.858296
                } else {
                    (((0.3f32 * variation) as f64).exp() - 1.0) as f32 * 2.858296
                };
                variation = variation * 2.0 - 1.0;
                let mut weight = modulation[x];
                weight *= maths::exp((variation - settings.smoothness) * settings.variation);
                if settings.variation_contrast != 0.0 && octave >= feature_octave {
                    let contrasted = (((weight * settings.variation_contrast) as f64).exp() - 1.0) as f32 * contrast_coefficient;
                    weight = if octave != feature_octave {
                        contrasted
                    } else {
                        (1.0 - feature_coefficient) * weight + feature_coefficient * contrasted
                    };
                }
                let saturated = 1.0 - 1.0 / (1.0 + weight);
                weight = 10.0 * (1.0 - 1.0 / (1.0 + 1.0 / 9.0 * weight));
                weight = settings.damping * saturated + (1.0 - settings.damping) * weight;
                modulation[x] = weight;
                weight = 1.0 - 1.0 / (1.0 + weight);
                let value = shaped(settings.noise_type, noise[x], edge_smoothing);
                row[x] += (value * weight + settings.bias * (weight - 0.5)) * amplitude;
                let warp = warp_vectors[y * width + x];
                let warped_x = x as f32 + warp[0] * warp_scale / sampling.viewport_size[0];
                let warped_y = y as f32 + warp[1] * warp_scale / sampling.viewport_size[1];
                positions[x] = sampling.position(warped_x, warped_y, &global, &octave_axes);
            }
            total += amplitude;
            frequency += frequency;
            if octave >= feature_octave {
                amplitude = if octave != feature_octave { amplitude * settings.fractal_gain } else { amplitude * feature_gain };
            }
        }
        let normalizer = 1.0 / total;
        for value in row.iter_mut() {
            *value = *value * normalizer + 0.5;
        }
    });
    Map::from_data(resolution, values)
}
