//! The domain warps (`Warps`).

use std::f32::consts::PI;

use nerthus_engine::{Map, Resolution};
use nerthus_noise::fast_noise::{CellularReturnType, FastNoise, FractalType, NoiseType};
use rayon::prelude::*;

use crate::maths::{self, dotnet_max};
use crate::transformer::EdgeBehaviour;

/// `WarpNoiseType`: the noise that displaces the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarpNoiseType {
    PerlinFbm,
    VoronoiR,
    VoronoiP,
    VoronoiA,
    VoronoiS,
    VoronoiM,
    VoronoiD,
}

impl WarpNoiseType {
    pub fn from_name(name: &str) -> Self {
        match name {
            "VoronoiR" => Self::VoronoiR,
            "VoronoiP" => Self::VoronoiP,
            "VoronoiA" => Self::VoronoiA,
            "VoronoiS" => Self::VoronoiS,
            "VoronoiM" => Self::VoronoiM,
            "VoronoiD" => Self::VoronoiD,
            _ => Self::PerlinFbm,
        }
    }

    fn cellular_return(self) -> Option<CellularReturnType> {
        match self {
            Self::PerlinFbm => None,
            Self::VoronoiR => Some(CellularReturnType::Distance),
            Self::VoronoiP => Some(CellularReturnType::Distance2),
            Self::VoronoiA => Some(CellularReturnType::Distance2Add),
            Self::VoronoiS => Some(CellularReturnType::Distance2Sub),
            Self::VoronoiM => Some(CellularReturnType::Distance2Mul),
            Self::VoronoiD => Some(CellularReturnType::Distance2Div),
        }
    }
}

/// `IterativeWarpMode`: how repeated warps accumulate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IterativeWarpMode {
    /// Warps the map itself again each time.
    Real,
    /// Warps a field of sample positions, then samples the map once.
    Virtual,
    /// Adds each warp to the sample positions, then samples once.
    Integral,
}

/// Settings of [`fractal_warp`] (`Warps.FractalWarp`'s parameters).
#[derive(Clone, Debug)]
pub struct FractalWarpSettings {
    pub size: f32,
    pub strength: f32,
    pub persist_strength: bool,
    pub z_coefficient: f32,
    pub noise_type: WarpNoiseType,
    pub perturbation: f32,
    pub octaves: i32,
    pub roughness: f32,
    pub normalized: bool,
    pub iterations: i32,
    pub mode: IterativeWarpMode,
    pub edges: EdgeBehaviour,
    pub seed: i32,
    pub modulation: f32,
    pub direction: f32,
    pub jitter: f32,
}

impl Default for FractalWarpSettings {
    fn default() -> Self {
        Self {
            size: 0.5,
            strength: 0.5,
            persist_strength: true,
            z_coefficient: 0.0,
            noise_type: WarpNoiseType::PerlinFbm,
            perturbation: 0.5,
            octaves: 12,
            roughness: 0.5,
            normalized: false,
            iterations: 1,
            mode: IterativeWarpMode::Virtual,
            edges: EdgeBehaviour::Edge,
            seed: 0,
            modulation: 0.0,
            direction: 45.0,
            jitter: 0.45,
        }
    }
}

/// The source cell of a warped sample, mirrored or clamped at the borders
/// (`Warps.InterpolateBilinear`, which has no half-pixel shift).
fn wrap_index(index: i32, resolution: i32, edges: EdgeBehaviour) -> usize {
    if edges == EdgeBehaviour::Edge {
        return index.max(0).min(resolution - 1) as usize;
    }
    let mut wrapped = index.abs();
    wrapped %= resolution * 2;
    if wrapped >= resolution {
        wrapped = resolution * 2 - 1 - wrapped;
    }
    wrapped as usize
}

/// Bilinear corner indices and fractions of a sample position.
struct Bilinear {
    left: usize,
    right: usize,
    top: usize,
    bottom: usize,
    fraction_x: f32,
    fraction_y: f32,
}

impl Bilinear {
    fn at(resolution: usize, x: f32, y: f32, edges: EdgeBehaviour) -> Self {
        let left = x as i32;
        let top = y as i32;
        let size = resolution as i32;
        Self {
            fraction_x: x - left as f32,
            fraction_y: y - top as f32,
            left: wrap_index(left, size, edges),
            right: wrap_index(left + 1, size, edges),
            top: wrap_index(top, size, edges),
            bottom: wrap_index(top + 1, size, edges),
        }
    }

    fn sample(&self, data: &[f32], resolution: usize) -> f32 {
        let top_left = data[resolution * self.top + self.left];
        let top_right = data[resolution * self.top + self.right];
        let bottom_left = data[resolution * self.bottom + self.left];
        let bottom_right = data[resolution * self.bottom + self.right];
        let upper = top_left * (1.0 - self.fraction_x) + top_right * self.fraction_x;
        let lower = bottom_left * (1.0 - self.fraction_x) + bottom_right * self.fraction_x;
        upper * (1.0 - self.fraction_y) + lower * self.fraction_y
    }

    fn sample_vector(&self, field: &[[f32; 2]], resolution: usize) -> [f32; 2] {
        let mut result = [0.0f32; 2];
        for (axis, value) in result.iter_mut().enumerate() {
            let top_left = field[resolution * self.top + self.left][axis];
            let top_right = field[resolution * self.top + self.right][axis];
            let bottom_left = field[resolution * self.bottom + self.left][axis];
            let bottom_right = field[resolution * self.bottom + self.right][axis];
            let upper = top_left * (1.0 - self.fraction_x) + top_right * self.fraction_x;
            let lower = bottom_left * (1.0 - self.fraction_x) + bottom_right * self.fraction_x;
            *value = upper * (1.0 - self.fraction_y) + lower * self.fraction_y;
        }
        result
    }
}

fn sample_bilinear(data: &[f32], resolution: usize, x: f32, y: f32, edges: EdgeBehaviour) -> f32 {
    Bilinear::at(resolution, x, y, edges).sample(data, resolution)
}

/// The two noise generators that displace x and y.
struct WarpNoises {
    x: FastNoise,
    y: FastNoise,
}

impl WarpNoises {
    fn new(settings: &FractalWarpSettings, frequency: f32) -> Self {
        let mut x = FastNoise::new(1337);
        let mut y = FastNoise::new(1337);
        x.set_fractal_type(FractalType::Billow);
        x.set_noise_type(NoiseType::Cellular);
        x.set_cellular_return_type(CellularReturnType::Distance2Sub);
        x.set_fractal_octaves(settings.octaves);
        x.set_frequency(frequency);
        x.set_fractal_gain(settings.roughness);
        y.set_noise_type(NoiseType::Cellular);
        y.set_cellular_return_type(CellularReturnType::Distance2Sub);
        y.set_fractal_octaves(settings.octaves);
        y.set_fractal_gain(settings.roughness);
        y.set_frequency(frequency);
        match settings.noise_type.cellular_return() {
            None => {
                x.set_fractal_type(FractalType::Fbm);
                x.set_noise_type(NoiseType::PerlinFractal);
                y.set_fractal_type(FractalType::Fbm);
                y.set_noise_type(NoiseType::PerlinFractal);
            }
            Some(return_type) => {
                x.set_noise_type(NoiseType::Cellular);
                x.set_cellular_return_type(return_type);
                y.set_noise_type(NoiseType::Cellular);
                y.set_cellular_return_type(return_type);
                x.set_cellular_jitter(settings.jitter);
            }
        }
        Self { x, y }
    }

    /// The fine Perlin pass that ends a warp with perturbation.
    fn switch_to_perturbation(&mut self, frequency: f32) {
        for noise in [&mut self.x, &mut self.y] {
            noise.set_fractal_type(FractalType::Fbm);
            noise.set_noise_type(NoiseType::PerlinFractal);
            noise.set_frequency(frequency * 4.0);
        }
    }

    fn reseed(&mut self, seed: &mut i32) {
        self.x.set_seed(*seed);
        *seed += 1;
        self.y.set_seed(*seed);
        *seed += 1;
    }

    fn sample(&self, positions: &[[f32; 3]]) -> (Vec<f32>, Vec<f32>) {
        let origin = [0.0, 0.0, 0.0];
        let x = self.x.fill_vector_set(positions, origin);
        let y = self.y.fill_vector_set(positions, origin);
        (x, y)
    }
}

/// Per-iteration state of the warp loop.
struct WarpPass {
    sign: f32,
    move_x: f32,
    move_y: f32,
    strength: f32,
    normalized: bool,
}

/// One sample's displacement: the noise part and the modulation part, kept
/// apart because they are added to positions in different orders.
struct Displacement {
    noise_x: f32,
    noise_y: f32,
    modulated_x: f32,
    modulated_y: f32,
}

impl Displacement {
    /// `(column + noise) + modulation`, as the Real and Virtual modes add.
    fn applied_to(&self, column: f32, row: f32) -> (f32, f32) {
        let x = column + self.noise_x + self.modulated_x;
        let y = row + self.noise_y + self.modulated_y;
        (x, y)
    }

    /// `noise + modulation`, as the Integral mode accumulates.
    fn combined(&self) -> (f32, f32) {
        (self.noise_x + self.modulated_x, self.noise_y + self.modulated_y)
    }
}

impl WarpPass {
    /// The displacement of one sample.
    fn offset(&self, noise_x: f32, noise_y: f32, modulator: Option<&[f32]>, index: usize, modulation: f32, resolution: usize) -> Displacement {
        let mut x = noise_x * self.sign;
        let mut y = noise_y * self.sign;
        if self.normalized {
            let length_squared = x * x + y * y + 0.1;
            let factor = 0.5 / length_squared.sqrt();
            x *= factor;
            y *= factor;
        }
        let mut modulated = 0.0f32;
        if let Some(modulator) = modulator
            && modulation > 0.0 {
                modulated = (modulator[index] - 0.5) * modulation * resolution as f32 * 1.0;
            }
        Displacement {
            noise_x: x * self.strength,
            noise_y: y * self.strength,
            modulated_x: self.move_x * modulated,
            modulated_y: self.move_y * modulated,
        }
    }
}

/// `Warps.FractalWarp`: displaces the map's first channel by fractal noise
/// `size` of the map across, `strength` deep, `iterations` times.
pub fn fractal_warp(map: &Map, settings: &FractalWarpSettings, modulator: Option<&Map>, terrain_width: f32, terrain_height: f32) -> Map {
    let resolution = map.width();
    let mut data = map.data().to_vec();
    let size = dotnet_max(0.0001, settings.size);
    let modulator_data = modulator.map(|map| map.data().to_vec());
    let modulator = modulator_data.as_deref();
    let mut heightmap = vec![0.0f32; resolution * resolution];
    let wavelength = size * resolution as f32;
    let frequency = 1.0 / wavelength;
    let mut noises = WarpNoises::new(settings, frequency);

    let persist = if settings.persist_strength { size } else { 1.0 };
    let mut strength = settings.strength * resolution as f32 * persist;
    let z_coefficient = settings.z_coefficient * resolution as f32 / terrain_width * terrain_height;
    let perturb = settings.noise_type != WarpNoiseType::PerlinFbm && settings.perturbation != 0.0;
    let mut iterations = settings.iterations;
    let turn = 360.0 / iterations as f32;
    if perturb && settings.mode != IterativeWarpMode::Integral {
        iterations += 1;
    }
    let mut direction = settings.direction;
    let mut seed = settings.seed;
    let mut normalized = settings.normalized;
    let perturbation_strength = settings.perturbation * resolution as f32 * size * 0.25;

    let next_pass = |iteration: i32, noises: &mut WarpNoises, direction: &mut f32, strength: &mut f32, normalized: &mut bool, seed: &mut i32| {
        let radians = *direction * (PI / 180.0);
        let (sine, cosine) = maths::sin_cos(radians);
        *direction += turn;
        let mut sign = if iteration & 1 == 0 { 1.0 } else { -1.0 };
        if perturb && iteration == iterations - 1 {
            noises.switch_to_perturbation(frequency);
            *normalized = false;
            *strength = perturbation_strength;
            sign = 1.0;
        }
        noises.reseed(seed);
        WarpPass {
            sign,
            move_x: 0.0 - cosine,
            move_y: sine,
            strength: *strength,
            normalized: *normalized,
        }
    };

    match settings.mode {
        IterativeWarpMode::Real => {
            for iteration in 0..iterations {
                let pass = next_pass(iteration, &mut noises, &mut direction, &mut strength, &mut normalized, &mut seed);
                let source = &data;
                heightmap.par_chunks_mut(resolution).enumerate().for_each(|(row, values)| {
                    let positions: Vec<[f32; 3]> = (0..resolution)
                        .map(|column| [column as f32, row as f32, source[row * resolution + column] * z_coefficient])
                        .collect();
                    let (noise_x, noise_y) = noises.sample(&positions);
                    for (column, value) in values.iter_mut().enumerate() {
                        let index = row * resolution + column;
                        let displacement = pass.offset(noise_x[column], noise_y[column], modulator, index, settings.modulation, resolution);
                        let (x, y) = displacement.applied_to(column as f32, row as f32);
                        *value = sample_bilinear(source, resolution, x, y, settings.edges);
                    }
                });
                data.copy_from_slice(&heightmap);
            }
        }
        IterativeWarpMode::Virtual => {
            let mut field: Vec<[f32; 2]> = (0..resolution * resolution).map(|index| [(index % resolution) as f32, (index / resolution) as f32]).collect();
            let mut next_field = vec![[0.0f32; 2]; resolution * resolution];
            for iteration in 0..iterations {
                let pass = next_pass(iteration, &mut noises, &mut direction, &mut strength, &mut normalized, &mut seed);
                let source = &data;
                let current = &field;
                next_field.par_chunks_mut(resolution).enumerate().for_each(|(row, values)| {
                    let positions: Vec<[f32; 3]> = (0..resolution)
                        .map(|column| {
                            let z = if z_coefficient > 0.0 {
                                let [field_x, field_y] = current[row * resolution + column];
                                sample_bilinear(source, resolution, field_x, field_y, settings.edges)
                            } else {
                                0.0
                            };
                            [column as f32, row as f32, z]
                        })
                        .collect();
                    let (noise_x, noise_y) = noises.sample(&positions);
                    for (column, value) in values.iter_mut().enumerate() {
                        let index = row * resolution + column;
                        let displacement = pass.offset(noise_x[column], noise_y[column], modulator, index, settings.modulation, resolution);
                        let (x, y) = displacement.applied_to(column as f32, row as f32);
                        *value = Bilinear::at(resolution, x, y, settings.edges).sample_vector(current, resolution);
                    }
                });
                std::mem::swap(&mut field, &mut next_field);
            }
            let source = &data;
            heightmap.par_iter_mut().enumerate().for_each(|(index, value)| {
                let [x, y] = field[index];
                *value = sample_bilinear(source, resolution, x, y, settings.edges);
            });
        }
        IterativeWarpMode::Integral => {
            let mut vectors: Vec<[f32; 3]> = (0..resolution * resolution)
                .map(|index| [(index % resolution) as f32, (index / resolution) as f32, data[index] * z_coefficient])
                .collect();
            for iteration in 0..iterations {
                let radians = direction * (PI / 180.0);
                let (sine, cosine) = maths::sin_cos(radians);
                direction += turn;
                let sign = if iteration & 1 == 0 { 1.0 } else { -1.0 };
                noises.reseed(&mut seed);
                let pass = WarpPass {
                    sign,
                    move_x: 0.0 - cosine,
                    move_y: sine,
                    strength,
                    normalized,
                };
                let source = &data;
                vectors.par_chunks_mut(resolution).enumerate().for_each(|(row, row_vectors)| {
                    let (noise_x, noise_y) = noises.sample(row_vectors);
                    for (column, vector) in row_vectors.iter_mut().enumerate() {
                        let index = row * resolution + column;
                        let displacement = pass.offset(noise_x[column], noise_y[column], modulator, index, settings.modulation, resolution);
                        let (offset_x, offset_y) = displacement.combined();
                        vector[0] += offset_x;
                        vector[1] += offset_y;
                        if z_coefficient > 0.0 {
                            vector[2] = sample_bilinear(source, resolution, vector[0], vector[1], settings.edges) * z_coefficient;
                        }
                    }
                });
            }
            let source = &data;
            heightmap.par_iter_mut().enumerate().for_each(|(index, value)| {
                let [x, y, _] = vectors[index];
                *value = sample_bilinear(source, resolution, x, y, settings.edges);
            });
            if perturb {
                std::mem::swap(&mut heightmap, &mut data);
                noises.switch_to_perturbation(frequency);
                noises.reseed(&mut seed);
                let source = &data;
                heightmap.par_chunks_mut(resolution).enumerate().for_each(|(row, values)| {
                    let positions: Vec<[f32; 3]> = (0..resolution)
                        .map(|column| [column as f32, row as f32, source[row * resolution + column] * z_coefficient])
                        .collect();
                    let (noise_x, noise_y) = noises.sample(&positions);
                    for (column, value) in values.iter_mut().enumerate() {
                        let x = column as f32 + noise_x[column] * perturbation_strength;
                        let y = row as f32 + noise_y[column] * perturbation_strength;
                        *value = sample_bilinear(source, resolution, x, y, settings.edges);
                    }
                });
            }
        }
    }
    Map::from_data(Resolution { x: map.width(), y: map.height() }, heightmap)
}

/// `Warps.DirectionWarpF`: shifts each pixel along `direction` (degrees)
/// by `(custom − 0.5) × strength` pixels.
pub fn direction_warp(data: &[f32], custom: &[f32], resolution: usize, strength: f32, direction: f32, edges: EdgeBehaviour) -> Vec<f32> {
    let radians = direction * (PI / 180.0);
    let (sine, cosine) = maths::sin_cos(radians);
    let move_x = (0.0 - cosine) * strength;
    let move_y = sine * strength;
    let mut result = vec![0.0f32; resolution * resolution];
    result.par_chunks_mut(resolution).enumerate().for_each(|(row, values)| {
        for (column, value) in values.iter_mut().enumerate() {
            let amount = custom[row * resolution + column] - 0.5;
            let x = column as f32 + amount * move_x;
            let y = row as f32 + amount * move_y;
            *value = sample_bilinear(data, resolution, x, y, edges);
        }
    });
    result
}

/// `Warps.DirectionWarp` on maps: `strength` is a fraction of the width.
pub fn direction_warp_map(data: &Map, custom: &Map, strength: f32, direction: f32, edges: EdgeBehaviour) -> Map {
    let resolution = data.width();
    let pixels = strength * resolution as f32;
    let warped = direction_warp(data.data(), custom.data(), resolution, pixels, direction, edges);
    Map::from_data(data.resolution(), warped)
}
