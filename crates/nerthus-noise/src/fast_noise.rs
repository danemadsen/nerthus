//! Scalar port of FastNoiseSIMD (Jordan Peck, MIT licence).
//!
//! This follows the `FastNoiseSIMD_CLib` build of the library. Every constant
//! and operation order here was checked against that binary so that a given
//! seed produces the same field as it does.

use rayon::prelude::*;

const X_PRIME: i32 = 1619;
const Y_PRIME: i32 = 31337;
const Z_PRIME: i32 = 6971;
const HASH_MULTIPLIER: i32 = 60493;
const HASH_TO_FLOAT: f32 = 1.0 / 2_147_483_648.0;
const BIT10_MASK: i32 = 0x3ff;
const CELL_HALF_RANGE: f32 = 511.5;
const F3: f32 = 1.0 / 3.0;
const G3: f32 = 1.0 / 6.0;
const G33: f32 = -0.5;
const CUBIC_BOUNDING: f32 = 1.0 / (1.5 * 1.5 * 1.5);
const CELLULAR_INDEX_MAX: usize = 3;
const FAR_DISTANCE: f32 = 999_999.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseType {
    Value,
    ValueFractal,
    Perlin,
    PerlinFractal,
    Simplex,
    SimplexFractal,
    WhiteNoise,
    Cellular,
    Cubic,
    CubicFractal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FractalType {
    Fbm,
    Billow,
    RigidMulti,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PerturbType {
    None,
    Gradient,
    GradientFractal,
    Normalise,
    GradientNormalise,
    GradientFractalNormalise,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellularDistanceFunction {
    Euclidean,
    Manhattan,
    Natural,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellularReturnType {
    CellValue,
    Distance,
    Distance2,
    Distance2Add,
    Distance2Sub,
    Distance2Mul,
    Distance2Div,
    NoiseLookup,
    Distance2Cave,
}

impl NoiseType {
    pub fn from_index(index: i32) -> Self {
        match index {
            0 => Self::Value,
            1 => Self::ValueFractal,
            2 => Self::Perlin,
            3 => Self::PerlinFractal,
            4 => Self::Simplex,
            6 => Self::WhiteNoise,
            7 => Self::Cellular,
            8 => Self::Cubic,
            9 => Self::CubicFractal,
            _ => Self::SimplexFractal,
        }
    }
}

impl FractalType {
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Billow,
            2 => Self::RigidMulti,
            _ => Self::Fbm,
        }
    }
}

impl PerturbType {
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Gradient,
            2 => Self::GradientFractal,
            3 => Self::Normalise,
            4 => Self::GradientNormalise,
            5 => Self::GradientFractalNormalise,
            _ => Self::None,
        }
    }
}

impl CellularDistanceFunction {
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Manhattan,
            2 => Self::Natural,
            _ => Self::Euclidean,
        }
    }
}

impl CellularReturnType {
    pub fn from_index(index: i32) -> Self {
        match index {
            0 => Self::CellValue,
            2 => Self::Distance2,
            3 => Self::Distance2Add,
            4 => Self::Distance2Sub,
            5 => Self::Distance2Mul,
            6 => Self::Distance2Div,
            7 => Self::NoiseLookup,
            8 => Self::Distance2Cave,
            _ => Self::Distance,
        }
    }
}

/// Settings captured for the cellular "noise lookup" return type.
#[derive(Clone, Copy, Debug)]
struct NoiseLookupSettings {
    noise_type: NoiseType,
    frequency: f32,
    fractal_type: FractalType,
    octaves: i32,
    lacunarity: f32,
    gain: f32,
    fractal_bounding: f32,
}

/// Perturb parameters resolved once per fill (mirrors `INIT_PERTURB_VALUES`).
#[derive(Clone, Copy, Debug)]
struct PerturbValues {
    amp: f32,
    frequency: f32,
    lacunarity: f32,
    gain: f32,
    normalise_length: f32,
}

/// Fractal parameters resolved once per fill.
#[derive(Clone, Copy, Debug)]
struct FractalValues {
    fractal_type: FractalType,
    octaves: i32,
    lacunarity: f32,
    gain: f32,
    bounding: f32,
}

#[derive(Clone, Debug)]
pub struct FastNoise {
    seed: i32,
    frequency: f32,
    noise_type: NoiseType,
    x_scale: f32,
    y_scale: f32,
    z_scale: f32,

    octaves: i32,
    lacunarity: f32,
    gain: f32,
    fractal_type: FractalType,
    fractal_bounding: f32,

    cellular_distance_function: CellularDistanceFunction,
    cellular_return_type: CellularReturnType,
    cellular_noise_lookup_type: NoiseType,
    cellular_noise_lookup_frequency: f32,
    cellular_distance_index0: usize,
    cellular_distance_index1: usize,
    cellular_jitter: f32,

    perturb_type: PerturbType,
    perturb_amp: f32,
    perturb_frequency: f32,
    perturb_octaves: i32,
    perturb_lacunarity: f32,
    perturb_gain: f32,
    perturb_fractal_bounding: f32,
    perturb_normalise_length: f32,
}

impl Default for FastNoise {
    fn default() -> Self {
        Self::new(1337)
    }
}

pub fn calculate_fractal_bounding(octaves: i32, gain: f32) -> f32 {
    let mut amp = gain;
    let mut amp_fractal = 1.0f32;
    let mut index = 1;
    while index < octaves {
        amp_fractal += amp;
        amp *= gain;
        index += 1;
    }
    1.0 / amp_fractal
}

impl FastNoise {
    pub fn new(seed: i32) -> Self {
        Self {
            seed,
            frequency: 0.01,
            noise_type: NoiseType::SimplexFractal,
            x_scale: 1.0,
            y_scale: 1.0,
            z_scale: 1.0,
            octaves: 3,
            lacunarity: 2.0,
            gain: 0.5,
            fractal_type: FractalType::Fbm,
            fractal_bounding: calculate_fractal_bounding(3, 0.5),
            cellular_distance_function: CellularDistanceFunction::Euclidean,
            cellular_return_type: CellularReturnType::Distance,
            cellular_noise_lookup_type: NoiseType::Simplex,
            cellular_noise_lookup_frequency: 0.2,
            cellular_distance_index0: 0,
            cellular_distance_index1: 1,
            cellular_jitter: 0.45,
            perturb_type: PerturbType::None,
            perturb_amp: 1.0 / CELL_HALF_RANGE,
            perturb_frequency: 0.5,
            perturb_octaves: 3,
            perturb_lacunarity: 2.0,
            perturb_gain: 0.5,
            perturb_fractal_bounding: calculate_fractal_bounding(3, 0.5),
            perturb_normalise_length: 1.0,
        }
    }

    pub fn seed(&self) -> i32 {
        self.seed
    }

    pub fn set_seed(&mut self, seed: i32) {
        self.seed = seed;
    }

    pub fn set_frequency(&mut self, frequency: f32) {
        self.frequency = frequency;
    }

    pub fn set_noise_type(&mut self, noise_type: NoiseType) {
        self.noise_type = noise_type;
    }

    pub fn set_axis_scales(&mut self, x_scale: f32, y_scale: f32, z_scale: f32) {
        self.x_scale = x_scale;
        self.y_scale = y_scale;
        self.z_scale = z_scale;
    }

    pub fn set_fractal_octaves(&mut self, octaves: i32) {
        self.octaves = octaves;
        self.fractal_bounding = calculate_fractal_bounding(self.octaves, self.gain);
    }

    pub fn set_fractal_lacunarity(&mut self, lacunarity: f32) {
        self.lacunarity = lacunarity;
    }

    pub fn set_fractal_gain(&mut self, gain: f32) {
        self.gain = gain;
        self.fractal_bounding = calculate_fractal_bounding(self.octaves, self.gain);
    }

    pub fn set_fractal_type(&mut self, fractal_type: FractalType) {
        self.fractal_type = fractal_type;
    }

    pub fn set_cellular_distance_function(&mut self, function: CellularDistanceFunction) {
        self.cellular_distance_function = function;
    }

    pub fn set_cellular_return_type(&mut self, return_type: CellularReturnType) {
        self.cellular_return_type = return_type;
    }

    pub fn set_cellular_noise_lookup_type(&mut self, noise_type: NoiseType) {
        self.cellular_noise_lookup_type = noise_type;
    }

    pub fn set_cellular_noise_lookup_frequency(&mut self, frequency: f32) {
        self.cellular_noise_lookup_frequency = frequency;
    }

    pub fn set_cellular_distance2_indices(&mut self, index0: i32, index1: i32) {
        let low = index0.min(index1).clamp(0, CELLULAR_INDEX_MAX as i32);
        let high = index0.max(index1).clamp(0, CELLULAR_INDEX_MAX as i32);
        self.cellular_distance_index0 = low as usize;
        self.cellular_distance_index1 = high as usize;
    }

    pub fn set_cellular_jitter(&mut self, jitter: f32) {
        self.cellular_jitter = jitter;
    }

    pub fn set_perturb_type(&mut self, perturb_type: PerturbType) {
        self.perturb_type = perturb_type;
    }

    /// Maximum distance a position can be perturbed (stored pre-divided like the original).
    pub fn set_perturb_amp(&mut self, amp: f32) {
        self.perturb_amp = amp * (1.0 / CELL_HALF_RANGE);
    }

    /// Writes the stored (already divided) perturb amplitude directly, as
    /// code that pokes the field instead of calling `SetPerturbAmp` does.
    pub fn set_perturb_amp_raw(&mut self, stored: f32) {
        self.perturb_amp = stored;
    }

    pub fn set_perturb_frequency(&mut self, frequency: f32) {
        self.perturb_frequency = frequency;
    }

    pub fn set_perturb_fractal_octaves(&mut self, octaves: i32) {
        self.perturb_octaves = octaves;
        self.perturb_fractal_bounding = calculate_fractal_bounding(self.perturb_octaves, self.perturb_gain);
    }

    pub fn set_perturb_fractal_lacunarity(&mut self, lacunarity: f32) {
        self.perturb_lacunarity = lacunarity;
    }

    pub fn set_perturb_fractal_gain(&mut self, gain: f32) {
        self.perturb_gain = gain;
        self.perturb_fractal_bounding = calculate_fractal_bounding(self.perturb_octaves, self.perturb_gain);
    }

    pub fn set_perturb_normalise_length(&mut self, length: f32) {
        self.perturb_normalise_length = length;
    }

    fn perturb_values(&self) -> PerturbValues {
        let normalise_length = self.perturb_normalise_length * self.frequency;
        // GradientFractal deliberately uses the main fractal bounding (matches the original).
        let fractal_amp = self.perturb_amp * self.fractal_bounding;
        let amp = match self.perturb_type {
            PerturbType::GradientFractal | PerturbType::GradientFractalNormalise => fractal_amp,
            _ => self.perturb_amp,
        };
        PerturbValues {
            amp,
            frequency: self.perturb_frequency,
            lacunarity: self.perturb_lacunarity,
            gain: self.perturb_gain,
            normalise_length,
        }
    }

    fn fractal_values(&self) -> FractalValues {
        FractalValues {
            fractal_type: self.fractal_type,
            octaves: self.octaves,
            lacunarity: self.lacunarity,
            gain: self.gain,
            bounding: self.fractal_bounding,
        }
    }

    fn apply_perturb(&self, values: &PerturbValues, x: &mut f32, y: &mut f32, z: &mut f32) {
        let seed_base = self.seed.wrapping_sub(1);
        match self.perturb_type {
            PerturbType::None => {}
            PerturbType::Gradient => {
                gradient_perturb_single(seed_base, values.amp, values.frequency, x, y, z);
            }
            PerturbType::GradientFractal => {
                self.gradient_perturb_fractal(values, x, y, z);
            }
            PerturbType::Normalise => {
                normalise_position(values.normalise_length, x, y, z);
            }
            PerturbType::GradientNormalise => {
                gradient_perturb_single(seed_base, values.amp, values.frequency, x, y, z);
                normalise_position(values.normalise_length, x, y, z);
            }
            PerturbType::GradientFractalNormalise => {
                self.gradient_perturb_fractal(values, x, y, z);
                normalise_position(values.normalise_length, x, y, z);
            }
        }
    }

    fn gradient_perturb_fractal(&self, values: &PerturbValues, x: &mut f32, y: &mut f32, z: &mut f32) {
        let mut seed = self.seed.wrapping_sub(1);
        let mut frequency = values.frequency;
        let mut amp = values.amp;
        gradient_perturb_single(seed, amp, frequency, x, y, z);
        let mut octave = 1;
        while octave < self.perturb_octaves {
            frequency *= values.lacunarity;
            seed = seed.wrapping_sub(1);
            amp *= values.gain;
            gradient_perturb_single(seed, amp, frequency, x, y, z);
            octave += 1;
        }
    }

    /// Evaluates the configured noise at an already-scaled position (after perturbation).
    fn evaluate(&self, fractal: &FractalValues, lookup: &NoiseLookupSettings, x: f32, y: f32, z: f32) -> f32 {
        let seed = self.seed;
        match self.noise_type {
            NoiseType::Value => value_single(seed, x, y, z),
            NoiseType::ValueFractal => fractal_single(fractal, seed, x, y, z, value_single),
            NoiseType::Perlin => perlin_single(seed, x, y, z),
            NoiseType::PerlinFractal => fractal_single(fractal, seed, x, y, z, perlin_single),
            NoiseType::Simplex => simplex_single(seed, x, y, z),
            NoiseType::SimplexFractal => fractal_single(fractal, seed, x, y, z, simplex_single),
            NoiseType::Cubic => cubic_single(seed, x, y, z),
            NoiseType::CubicFractal => fractal_single(fractal, seed, x, y, z, cubic_single),
            NoiseType::Cellular => self.cellular_single(lookup, seed, x, y, z),
            NoiseType::WhiteNoise => white_noise_single(seed, x, y, z),
        }
    }

    fn cellular_single(&self, lookup: &NoiseLookupSettings, seed: i32, x: f32, y: f32, z: f32) -> f32 {
        let jitter = self.cellular_jitter;
        let function = self.cellular_distance_function;
        let index0 = self.cellular_distance_index0;
        let index1 = self.cellular_distance_index1;
        match self.cellular_return_type {
            CellularReturnType::CellValue => cellular_value_single(function, seed, x, y, z, jitter),
            CellularReturnType::Distance => cellular_distance_single(function, seed, x, y, z, jitter),
            CellularReturnType::Distance2
            | CellularReturnType::Distance2Add
            | CellularReturnType::Distance2Sub
            | CellularReturnType::Distance2Mul
            | CellularReturnType::Distance2Div => {
                let distances = cellular_distances_single(function, seed, x, y, z, jitter, index1);
                combine_distance2(self.cellular_return_type, distances[index0], distances[index1])
            }
            CellularReturnType::Distance2Cave => {
                let first_distances = cellular_distances_single(function, seed, x, y, z, jitter, index1);
                let first = first_distances[index0] / first_distances[index1];
                let shifted_seed = seed.wrapping_add(1);
                let second_distances = cellular_distances_single(function, shifted_seed, x + 0.5, y + 0.5, z + 0.5, jitter, index1);
                let second = second_distances[index0] / second_distances[index1];
                first.min(second)
            }
            CellularReturnType::NoiseLookup => cellular_lookup_single(function, seed, x, y, z, jitter, lookup),
        }
    }

    fn lookup_settings(&self) -> NoiseLookupSettings {
        NoiseLookupSettings {
            noise_type: self.cellular_noise_lookup_type,
            frequency: self.cellular_noise_lookup_frequency,
            fractal_type: self.fractal_type,
            octaves: self.octaves,
            lacunarity: self.lacunarity,
            gain: self.gain,
            fractal_bounding: self.fractal_bounding,
        }
    }

    /// Fills `noise_set` exactly like `FastNoiseSIMD::FillNoiseSet`.
    ///
    /// Layout is x-major: `index = (ix * y_size + iy) * z_size + iz`.
    #[allow(clippy::too_many_arguments)]
    pub fn fill_noise_set(
        &self,
        noise_set: &mut [f32],
        x_start: i32,
        y_start: i32,
        z_start: i32,
        x_size: usize,
        y_size: usize,
        z_size: usize,
        scale_modifier: f32,
    ) {
        assert!(noise_set.len() >= x_size * y_size * z_size);
        let row_len = z_size;
        let rows = &mut noise_set[..x_size * y_size * z_size];
        let context = FillContext::new(self, scale_modifier);

        rows.par_chunks_mut(row_len).enumerate().for_each(|(row_index, row)| {
            let ix = (row_index / y_size) as i32;
            let iy = (row_index % y_size) as i32;
            let x = x_start + ix;
            let y = y_start + iy;
            for (iz, out) in row.iter_mut().enumerate() {
                let z = z_start + iz as i32;
                *out = context.sample(self, x, y, z);
            }
        });
    }

    /// Convenience wrapper returning a freshly allocated set.
    #[allow(clippy::too_many_arguments)]
    pub fn get_noise_set(
        &self,
        x_start: i32,
        y_start: i32,
        z_start: i32,
        x_size: usize,
        y_size: usize,
        z_size: usize,
        scale_modifier: f32,
    ) -> Vec<f32> {
        let mut values = vec![0.0f32; x_size * y_size * z_size];
        self.fill_noise_set(&mut values, x_start, y_start, z_start, x_size, y_size, z_size, scale_modifier);
        values
    }

    /// Samples one lattice position the same way `fill_noise_set` would.
    pub fn sample(&self, x: i32, y: i32, z: i32, scale_modifier: f32) -> f32 {
        let context = FillContext::new(self, scale_modifier);
        context.sample(self, x, y, z)
    }

    /// Samples a continuous (already frequency-scaled) position. Used by vector sets.
    pub fn sample_position(&self, x: f32, y: f32, z: f32) -> f32 {
        let context = FillContext::new(self, 1.0);
        let mut px = x;
        let mut py = y;
        let mut pz = z;
        self.apply_perturb(&context.perturb, &mut px, &mut py, &mut pz);
        self.evaluate(&context.fractal, &context.lookup, px, py, pz)
    }

    /// `FillNoiseSet(noiseSet, vectorSet, xOffset, yOffset, zOffset)`: evaluates the
    /// noise at arbitrary positions. Positions are scaled by frequency and axis
    /// scales (no scale modifier), offsets likewise, then perturbed.
    pub fn fill_vector_set(&self, positions: &[[f32; 3]], offset: [f32; 3]) -> Vec<f32> {
        let x_frequency = self.frequency * self.x_scale;
        let y_frequency = self.frequency * self.y_scale;
        let z_frequency = self.frequency * self.z_scale;
        let x_offset = offset[0] * x_frequency;
        let y_offset = offset[1] * y_frequency;
        let z_offset = offset[2] * z_frequency;
        let perturb = self.perturb_values();
        let fractal = self.fractal_values();
        let lookup = self.lookup_settings();
        positions
            .iter()
            .map(|position| {
                let mut xf = position[0] * x_frequency + x_offset;
                let mut yf = position[1] * y_frequency + y_offset;
                let mut zf = position[2] * z_frequency + z_offset;
                self.apply_perturb(&perturb, &mut xf, &mut yf, &mut zf);
                self.evaluate(&fractal, &lookup, xf, yf, zf)
            })
            .collect()
    }

    /// 2D maps are sampled as the x = 0 slice of the 3D set: rows on y, columns on z.
    /// This fills a `rows x columns` row-major map that way.
    pub fn fill_slice(&self, rows: usize, columns: usize, row_offset: i32, column_offset: i32, scale_modifier: f32) -> Vec<f32> {
        let mut values = vec![0.0f32; rows * columns];
        let context = FillContext::new(self, scale_modifier);
        values.par_chunks_mut(columns).enumerate().for_each(|(row, out_row)| {
            let y = row as i32 + row_offset;
            for (column, out) in out_row.iter_mut().enumerate() {
                let z = column as i32 + column_offset;
                *out = context.sample(self, 0, y, z);
            }
        });
        values
    }
}

/// Per-fill resolved state shared by all samples.
struct FillContext {
    x_frequency: f32,
    y_frequency: f32,
    z_frequency: f32,
    perturb: PerturbValues,
    fractal: FractalValues,
    lookup: NoiseLookupSettings,
}

impl FillContext {
    fn new(noise: &FastNoise, scale_modifier: f32) -> Self {
        let scale = scale_modifier * noise.frequency;
        Self {
            x_frequency: scale * noise.x_scale,
            y_frequency: scale * noise.y_scale,
            z_frequency: scale * noise.z_scale,
            perturb: noise.perturb_values(),
            fractal: noise.fractal_values(),
            lookup: noise.lookup_settings(),
        }
    }

    #[inline]
    fn sample(&self, noise: &FastNoise, x: i32, y: i32, z: i32) -> f32 {
        if noise.noise_type == NoiseType::WhiteNoise {
            // White noise ignores frequency and hashes the integer lattice directly.
            let xi = x.wrapping_mul(X_PRIME);
            let yi = y.wrapping_mul(Y_PRIME);
            let zi = z.wrapping_mul(Z_PRIME);
            return val_coord(noise.seed, xi, yi, zi);
        }
        let mut xf = x as f32 * self.x_frequency;
        let mut yf = y as f32 * self.y_frequency;
        let mut zf = z as f32 * self.z_frequency;
        noise.apply_perturb(&self.perturb, &mut xf, &mut yf, &mut zf);
        noise.evaluate(&self.fractal, &self.lookup, xf, yf, zf)
    }
}

// ---------------------------------------------------------------------------
// Primitive helpers
// ---------------------------------------------------------------------------

#[inline]
fn round_to_int(value: f32) -> i32 {
    // _mm_cvtps_epi32 with the default rounding mode (nearest, ties to even).
    value.round_ties_even() as i32
}

#[inline]
fn interp_quintic(t: f32) -> f32 {
    let inner = (t * 6.0 - 15.0) * t + 10.0;
    inner * t * t * t
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
fn cubic_lerp(a: f32, b: f32, c: f32, d: f32, t: f32) -> f32 {
    let p = (d - c) - (a - b);
    let linear = t * (c - a) + b;
    let quadratic = t * (t * ((a - b) - p)) + linear;
    t * (t * (t * p)) + quadratic
}

#[inline]
fn hash(seed: i32, x: i32, y: i32, z: i32) -> i32 {
    let mut value = seed ^ x;
    value ^= y;
    value ^= z;
    value = value.wrapping_mul(value).wrapping_mul(HASH_MULTIPLIER).wrapping_mul(value);
    (value >> 13) ^ value
}

#[inline]
fn hash_hb(seed: i32, x: i32, y: i32, z: i32) -> i32 {
    let mut value = seed ^ x;
    value ^= y;
    value ^= z;
    value.wrapping_mul(value).wrapping_mul(HASH_MULTIPLIER).wrapping_mul(value)
}

#[inline]
fn val_coord(seed: i32, x: i32, y: i32, z: i32) -> f32 {
    hash_hb(seed, x, y, z) as f32 * HASH_TO_FLOAT
}

#[inline]
fn grad_coord(seed: i32, xi: i32, yi: i32, zi: i32, x: f32, y: f32, z: f32) -> f32 {
    let value = hash(seed, xi, yi, zi);
    let selector = value & 13;
    let u = if selector < 8 { x } else { y };
    let v = if selector < 2 {
        y
    } else if selector == 12 {
        x
    } else {
        z
    };
    let u = if value & 1 != 0 { -u } else { u };
    let v = if value & 2 != 0 { -v } else { v };
    u + v
}

// ---------------------------------------------------------------------------
// Noise kernels
// ---------------------------------------------------------------------------

fn value_single(seed: i32, x: f32, y: f32, z: f32) -> f32 {
    let xs = x.floor();
    let ys = y.floor();
    let zs = z.floor();

    let x0 = (xs as i32).wrapping_mul(X_PRIME);
    let y0 = (ys as i32).wrapping_mul(Y_PRIME);
    let z0 = (zs as i32).wrapping_mul(Z_PRIME);
    let x1 = x0.wrapping_add(X_PRIME);
    let y1 = y0.wrapping_add(Y_PRIME);
    let z1 = z0.wrapping_add(Z_PRIME);

    let tx = interp_quintic(x - xs);
    let ty = interp_quintic(y - ys);
    let tz = interp_quintic(z - zs);

    let a = lerp(val_coord(seed, x0, y0, z0), val_coord(seed, x1, y0, z0), tx);
    let b = lerp(val_coord(seed, x0, y1, z0), val_coord(seed, x1, y1, z0), tx);
    let c = lerp(val_coord(seed, x0, y0, z1), val_coord(seed, x1, y0, z1), tx);
    let d = lerp(val_coord(seed, x0, y1, z1), val_coord(seed, x1, y1, z1), tx);

    let near = lerp(a, b, ty);
    let far = lerp(c, d, ty);
    lerp(near, far, tz)
}

pub(crate) fn perlin_single(seed: i32, x: f32, y: f32, z: f32) -> f32 {
    let xs = x.floor();
    let ys = y.floor();
    let zs = z.floor();

    let x0 = (xs as i32).wrapping_mul(X_PRIME);
    let y0 = (ys as i32).wrapping_mul(Y_PRIME);
    let z0 = (zs as i32).wrapping_mul(Z_PRIME);
    let x1 = x0.wrapping_add(X_PRIME);
    let y1 = y0.wrapping_add(Y_PRIME);
    let z1 = z0.wrapping_add(Z_PRIME);

    let xf0 = x - xs;
    let yf0 = y - ys;
    let zf0 = z - zs;
    let xf1 = xf0 - 1.0;
    let yf1 = yf0 - 1.0;
    let zf1 = zf0 - 1.0;

    let tx = interp_quintic(xf0);
    let ty = interp_quintic(yf0);
    let tz = interp_quintic(zf0);

    let a = lerp(grad_coord(seed, x0, y0, z0, xf0, yf0, zf0), grad_coord(seed, x1, y0, z0, xf1, yf0, zf0), tx);
    let b = lerp(grad_coord(seed, x0, y1, z0, xf0, yf1, zf0), grad_coord(seed, x1, y1, z0, xf1, yf1, zf0), tx);
    let c = lerp(grad_coord(seed, x0, y0, z1, xf0, yf0, zf1), grad_coord(seed, x1, y0, z1, xf1, yf0, zf1), tx);
    let d = lerp(grad_coord(seed, x0, y1, z1, xf0, yf1, zf1), grad_coord(seed, x1, y1, z1, xf1, yf1, zf1), tx);

    let near = lerp(a, b, ty);
    let far = lerp(c, d, ty);
    lerp(near, far, tz)
}

fn simplex_single(seed: i32, x: f32, y: f32, z: f32) -> f32 {
    let f = F3 * ((x + y) + z);
    let x0f = (x + f).floor();
    let y0f = (y + f).floor();
    let z0f = (z + f).floor();

    let i = (x0f as i32).wrapping_mul(X_PRIME);
    let j = (y0f as i32).wrapping_mul(Y_PRIME);
    let k = (z0f as i32).wrapping_mul(Z_PRIME);

    let g = G3 * ((x0f + y0f) + z0f);
    let x0 = x - (x0f - g);
    let y0 = y - (y0f - g);
    let z0 = z - (z0f - g);

    let x0_ge_y0 = x0 >= y0;
    let y0_ge_z0 = y0 >= z0;
    let x0_ge_z0 = x0 >= z0;

    let i1 = x0_ge_y0 && x0_ge_z0;
    let j1 = !x0_ge_y0 && y0_ge_z0;
    let k1 = !x0_ge_z0 && !y0_ge_z0;

    let i2 = x0_ge_y0 || x0_ge_z0;
    let j2 = !x0_ge_y0 || y0_ge_z0;
    let k2 = !(x0_ge_z0 && y0_ge_z0);

    let x1 = (x0 - if i1 { 1.0 } else { 0.0 }) + G3;
    let y1 = (y0 - if j1 { 1.0 } else { 0.0 }) + G3;
    let z1 = (z0 - if k1 { 1.0 } else { 0.0 }) + G3;
    let x2 = (x0 - if i2 { 1.0 } else { 0.0 }) + F3;
    let y2 = (y0 - if j2 { 1.0 } else { 0.0 }) + F3;
    let z2 = (z0 - if k2 { 1.0 } else { 0.0 }) + F3;
    let x3 = x0 + G33;
    let y3 = y0 + G33;
    let z3 = z0 + G33;

    let t0 = 0.6 - x0 * x0 - y0 * y0 - z0 * z0;
    let t1 = 0.6 - x1 * x1 - y1 * y1 - z1 * z1;
    let t2 = 0.6 - x2 * x2 - y2 * y2 - z2 * z2;
    let t3 = 0.6 - x3 * x3 - y3 * y3 - z3 * z3;

    let mut sum = 0.0f32;
    if t3 >= 0.0 {
        let t = t3 * t3;
        let gradient = grad_coord(seed, i.wrapping_add(X_PRIME), j.wrapping_add(Y_PRIME), k.wrapping_add(Z_PRIME), x3, y3, z3);
        sum = t * t * gradient;
    }
    if t2 >= 0.0 {
        let t = t2 * t2;
        let ii = if i2 { i.wrapping_add(X_PRIME) } else { i };
        let jj = if j2 { j.wrapping_add(Y_PRIME) } else { j };
        let kk = if k2 { k.wrapping_add(Z_PRIME) } else { k };
        sum += t * t * grad_coord(seed, ii, jj, kk, x2, y2, z2);
    }
    if t1 >= 0.0 {
        let t = t1 * t1;
        let ii = if i1 { i.wrapping_add(X_PRIME) } else { i };
        let jj = if j1 { j.wrapping_add(Y_PRIME) } else { j };
        let kk = if k1 { k.wrapping_add(Z_PRIME) } else { k };
        sum += t * t * grad_coord(seed, ii, jj, kk, x1, y1, z1);
    }
    if t0 >= 0.0 {
        let t = t0 * t0;
        sum += t * t * grad_coord(seed, i, j, k, x0, y0, z0);
    }
    32.0 * sum
}

fn cubic_single(seed: i32, x: f32, y: f32, z: f32) -> f32 {
    let xf1 = x.floor();
    let yf1 = y.floor();
    let zf1 = z.floor();

    let x1 = (xf1 as i32).wrapping_mul(X_PRIME);
    let y1 = (yf1 as i32).wrapping_mul(Y_PRIME);
    let z1 = (zf1 as i32).wrapping_mul(Z_PRIME);

    let xs_list = [x1.wrapping_sub(X_PRIME), x1, x1.wrapping_add(X_PRIME), x1.wrapping_add(X_PRIME.wrapping_mul(2))];
    let ys_list = [y1.wrapping_sub(Y_PRIME), y1, y1.wrapping_add(Y_PRIME), y1.wrapping_add(Y_PRIME.wrapping_mul(2))];
    let zs_list = [z1.wrapping_sub(Z_PRIME), z1, z1.wrapping_add(Z_PRIME), z1.wrapping_add(Z_PRIME.wrapping_mul(2))];

    let tx = x - xf1;
    let ty = y - yf1;
    let tz = z - zf1;

    let mut planes = [0.0f32; 4];
    for (plane_index, &zc) in zs_list.iter().enumerate() {
        let mut lines = [0.0f32; 4];
        for (line_index, &yc) in ys_list.iter().enumerate() {
            let a = val_coord(seed, xs_list[0], yc, zc);
            let b = val_coord(seed, xs_list[1], yc, zc);
            let c = val_coord(seed, xs_list[2], yc, zc);
            let d = val_coord(seed, xs_list[3], yc, zc);
            lines[line_index] = cubic_lerp(a, b, c, d, tx);
        }
        planes[plane_index] = cubic_lerp(lines[0], lines[1], lines[2], lines[3], ty);
    }
    let value = cubic_lerp(planes[0], planes[1], planes[2], planes[3], tz);
    value * CUBIC_BOUNDING
}

fn white_noise_single(seed: i32, x: f32, y: f32, z: f32) -> f32 {
    let xb = x.to_bits() as i32;
    let yb = y.to_bits() as i32;
    let zb = z.to_bits() as i32;
    let xi = (xb ^ (xb >> 16)).wrapping_mul(X_PRIME);
    let yi = (yb ^ (yb >> 16)).wrapping_mul(Y_PRIME);
    let zi = (zb ^ (zb >> 16)).wrapping_mul(Z_PRIME);
    val_coord(seed, xi, yi, zi)
}

fn fractal_single(
    fractal: &FractalValues,
    seed: i32,
    x: f32,
    y: f32,
    z: f32,
    kernel: fn(i32, f32, f32, f32) -> f32,
) -> f32 {
    match fractal.fractal_type {
        FractalType::Fbm => fbm_single(fractal, seed, x, y, z, kernel),
        FractalType::Billow => billow_single(fractal, seed, x, y, z, kernel),
        FractalType::RigidMulti => rigid_multi_single(fractal, seed, x, y, z, kernel),
    }
}

fn fbm_single(fractal: &FractalValues, seed: i32, x: f32, y: f32, z: f32, kernel: fn(i32, f32, f32, f32) -> f32) -> f32 {
    let mut seed_f = seed;
    let mut xf = x;
    let mut yf = y;
    let mut zf = z;
    let mut result = kernel(seed_f, xf, yf, zf);
    let mut amp = 1.0f32;
    let mut octave = 1;
    while octave < fractal.octaves {
        xf *= fractal.lacunarity;
        yf *= fractal.lacunarity;
        zf *= fractal.lacunarity;
        seed_f = seed_f.wrapping_add(1);
        amp *= fractal.gain;
        result += kernel(seed_f, xf, yf, zf) * amp;
        octave += 1;
    }
    result * fractal.bounding
}

fn billow_single(fractal: &FractalValues, seed: i32, x: f32, y: f32, z: f32, kernel: fn(i32, f32, f32, f32) -> f32) -> f32 {
    let mut seed_f = seed;
    let mut xf = x;
    let mut yf = y;
    let mut zf = z;
    let mut result = kernel(seed_f, xf, yf, zf).abs() * 2.0 - 1.0;
    let mut amp = 1.0f32;
    let mut octave = 1;
    while octave < fractal.octaves {
        xf *= fractal.lacunarity;
        yf *= fractal.lacunarity;
        zf *= fractal.lacunarity;
        seed_f = seed_f.wrapping_add(1);
        amp *= fractal.gain;
        let octave_value = kernel(seed_f, xf, yf, zf).abs() * 2.0 - 1.0;
        result += octave_value * amp;
        octave += 1;
    }
    result * fractal.bounding
}

fn rigid_multi_single(fractal: &FractalValues, seed: i32, x: f32, y: f32, z: f32, kernel: fn(i32, f32, f32, f32) -> f32) -> f32 {
    let mut seed_f = seed;
    let mut xf = x;
    let mut yf = y;
    let mut zf = z;
    let mut result = 1.0 - kernel(seed_f, xf, yf, zf).abs();
    let mut amp = 1.0f32;
    let mut octave = 1;
    while octave < fractal.octaves {
        xf *= fractal.lacunarity;
        yf *= fractal.lacunarity;
        zf *= fractal.lacunarity;
        seed_f = seed_f.wrapping_add(1);
        amp *= fractal.gain;
        let ridge = 1.0 - kernel(seed_f, xf, yf, zf).abs();
        result -= amp * ridge;
        octave += 1;
    }
    result
}

// ---------------------------------------------------------------------------
// Cellular
// ---------------------------------------------------------------------------

/// Distance used by the CellValue and NoiseLookup returns (Natural = E + M).
#[inline]
fn cellular_distance(function: CellularDistanceFunction, x: f32, y: f32, z: f32) -> f32 {
    match function {
        CellularDistanceFunction::Euclidean => x * x + (y * y + z * z),
        CellularDistanceFunction::Manhattan => x.abs() + y.abs() + z.abs(),
        CellularDistanceFunction::Natural => {
            let euclidean = x * x + (y * y + z * z);
            let manhattan = x.abs() + y.abs() + z.abs();
            euclidean + manhattan
        }
    }
}

/// Distance used by the Distance/Distance2/Cave returns. FastNoiseSIMD
/// redefines Natural as E * M before instantiating these.
#[inline]
fn cellular_distance_product(function: CellularDistanceFunction, x: f32, y: f32, z: f32) -> f32 {
    match function {
        CellularDistanceFunction::Natural => {
            let euclidean = x * x + (y * y + z * z);
            let manhattan = x.abs() + y.abs() + z.abs();
            euclidean * manhattan
        }
        _ => cellular_distance(function, x, y, z),
    }
}

/// Jittered offset of the feature point inside one cell.
#[inline]
fn cell_offset(value: i32, jitter: f32) -> (f32, f32, f32) {
    let xd = (value & BIT10_MASK) as f32 - CELL_HALF_RANGE;
    let yd = ((value >> 10) & BIT10_MASK) as f32 - CELL_HALF_RANGE;
    let zd = ((value >> 20) & BIT10_MASK) as f32 - CELL_HALF_RANGE;
    let length_sq = xd * xd + (yd * yd + zd * zd);
    let inv_mag = jitter * (1.0 / length_sq.sqrt());
    (xd * inv_mag, yd * inv_mag, zd * inv_mag)
}

/// Visits the 27 candidate cells around (x, y, z) in the original iteration order.
#[inline]
fn for_each_cell(seed: i32, x: f32, y: f32, z: f32, jitter: f32, mut visit: impl FnMut(f32, f32, f32, i32, (f32, f32, f32), (f32, f32, f32))) {
    let xc_start = round_to_int(x) - 1;
    let yc_start = round_to_int(y) - 1;
    let zc_start = round_to_int(z) - 1;

    let mut xcf = xc_start as f32 - x;
    let ycf_base = yc_start as f32 - y;
    let zcf_base = zc_start as f32 - z;

    let mut xc = xc_start.wrapping_mul(X_PRIME);
    let yc_base = yc_start.wrapping_mul(Y_PRIME);
    let zc_base = zc_start.wrapping_mul(Z_PRIME);

    let mut cell_x = xc_start as f32;
    for _ in 0..3 {
        let mut ycf = ycf_base;
        let mut yc = yc_base;
        let mut cell_y = yc_start as f32;
        for _ in 0..3 {
            let mut zcf = zcf_base;
            let mut zc = zc_base;
            let mut cell_z = zc_start as f32;
            for _ in 0..3 {
                let value = hash_hb(seed, xc, yc, zc);
                let offset = cell_offset(value, jitter);
                let delta = (offset.0 + xcf, offset.1 + ycf, offset.2 + zcf);
                visit(cell_x, cell_y, cell_z, value, offset, delta);
                zcf += 1.0;
                zc = zc.wrapping_add(Z_PRIME);
                cell_z += 1.0;
            }
            ycf += 1.0;
            yc = yc.wrapping_add(Y_PRIME);
            cell_y += 1.0;
        }
        xcf += 1.0;
        xc = xc.wrapping_add(X_PRIME);
        cell_x += 1.0;
    }
}

fn cellular_value_single(function: CellularDistanceFunction, seed: i32, x: f32, y: f32, z: f32, jitter: f32) -> f32 {
    let mut distance = FAR_DISTANCE;
    let mut cell_value = 0.0f32;
    for_each_cell(seed, x, y, z, jitter, |_, _, _, value, _, delta| {
        let new_distance = cellular_distance(function, delta.0, delta.1, delta.2);
        let new_value = value as f32 * HASH_TO_FLOAT;
        if new_distance < distance {
            cell_value = new_value;
        }
        distance = new_distance.min(distance);
    });
    cell_value
}

fn cellular_distance_single(function: CellularDistanceFunction, seed: i32, x: f32, y: f32, z: f32, jitter: f32) -> f32 {
    let mut distance = FAR_DISTANCE;
    for_each_cell(seed, x, y, z, jitter, |_, _, _, _, _, delta| {
        let new_distance = cellular_distance_product(function, delta.0, delta.1, delta.2);
        distance = distance.min(new_distance);
    });
    distance
}

fn cellular_distances_single(
    function: CellularDistanceFunction,
    seed: i32,
    x: f32,
    y: f32,
    z: f32,
    jitter: f32,
    index1: usize,
) -> [f32; CELLULAR_INDEX_MAX + 1] {
    let mut distances = [FAR_DISTANCE; CELLULAR_INDEX_MAX + 1];
    for_each_cell(seed, x, y, z, jitter, |_, _, _, _, _, delta| {
        let new_distance = cellular_distance_product(function, delta.0, delta.1, delta.2);
        let mut index = index1;
        while index > 0 {
            let keep_lower = distances[index].min(new_distance);
            distances[index] = keep_lower.max(distances[index - 1]);
            index -= 1;
        }
        distances[0] = distances[0].min(new_distance);
    });
    distances
}

fn combine_distance2(return_type: CellularReturnType, distance: f32, distance2: f32) -> f32 {
    match return_type {
        CellularReturnType::Distance2 => distance2,
        CellularReturnType::Distance2Add => distance + distance2,
        CellularReturnType::Distance2Sub => distance2 - distance,
        CellularReturnType::Distance2Mul => distance * distance2,
        CellularReturnType::Distance2Div => distance / distance2,
        _ => distance2,
    }
}

fn cellular_lookup_single(
    function: CellularDistanceFunction,
    seed: i32,
    x: f32,
    y: f32,
    z: f32,
    jitter: f32,
    lookup: &NoiseLookupSettings,
) -> f32 {
    let mut distance = FAR_DISTANCE;
    let mut cell = (0.0f32, 0.0f32, 0.0f32);
    for_each_cell(seed, x, y, z, jitter, |cell_x, cell_y, cell_z, _, offset, delta| {
        let new_distance = cellular_distance(function, delta.0, delta.1, delta.2);
        if new_distance < distance {
            cell = (offset.0 + cell_x, offset.1 + cell_y, offset.2 + cell_z);
        }
        distance = new_distance.min(distance);
    });

    let xf = cell.0 * lookup.frequency;
    let yf = cell.1 * lookup.frequency;
    let zf = cell.2 * lookup.frequency;
    let fractal = FractalValues {
        fractal_type: lookup.fractal_type,
        octaves: lookup.octaves,
        lacunarity: lookup.lacunarity,
        gain: lookup.gain,
        bounding: lookup.fractal_bounding,
    };
    match lookup.noise_type {
        NoiseType::Value => value_single(seed, xf, yf, zf),
        NoiseType::ValueFractal => fractal_single(&fractal, seed, xf, yf, zf, value_single),
        NoiseType::Perlin => perlin_single(seed, xf, yf, zf),
        NoiseType::PerlinFractal => fractal_single(&fractal, seed, xf, yf, zf, perlin_single),
        NoiseType::Simplex => simplex_single(seed, xf, yf, zf),
        NoiseType::SimplexFractal => fractal_single(&fractal, seed, xf, yf, zf, simplex_single),
        NoiseType::Cubic => cubic_single(seed, xf, yf, zf),
        NoiseType::CubicFractal => fractal_single(&fractal, seed, xf, yf, zf, cubic_single),
        NoiseType::WhiteNoise | NoiseType::Cellular => 0.0,
    }
}

// ---------------------------------------------------------------------------
// Perturbation
// ---------------------------------------------------------------------------

fn gradient_perturb_single(seed: i32, amp: f32, frequency: f32, x: &mut f32, y: &mut f32, z: &mut f32) {
    let xf = *x * frequency;
    let yf = *y * frequency;
    let zf = *z * frequency;

    let xs = xf.floor();
    let ys = yf.floor();
    let zs = zf.floor();

    let x0 = (xs as i32).wrapping_mul(X_PRIME);
    let y0 = (ys as i32).wrapping_mul(Y_PRIME);
    let z0 = (zs as i32).wrapping_mul(Z_PRIME);
    let x1 = x0.wrapping_add(X_PRIME);
    let y1 = y0.wrapping_add(Y_PRIME);
    let z1 = z0.wrapping_add(Z_PRIME);

    let tx = interp_quintic(xf - xs);
    let ty = interp_quintic(yf - ys);
    let tz = interp_quintic(zf - zs);

    let corners = [
        hash_hb(seed, x0, y0, z0),
        hash_hb(seed, x1, y0, z0),
        hash_hb(seed, x0, y1, z0),
        hash_hb(seed, x1, y1, z0),
        hash_hb(seed, x0, y0, z1),
        hash_hb(seed, x1, y0, z1),
        hash_hb(seed, x0, y1, z1),
        hash_hb(seed, x1, y1, z1),
    ];

    let mut offsets = [0.0f32; 3];
    for (axis, offset) in offsets.iter_mut().enumerate() {
        let shift = axis as u32 * 10;
        let mut values = [0.0f32; 8];
        for (value, corner) in values.iter_mut().zip(corners.iter()) {
            let field = (corner >> shift) & BIT10_MASK;
            *value = field as f32 - CELL_HALF_RANGE;
        }
        let a = lerp(values[0], values[1], tx);
        let b = lerp(values[2], values[3], tx);
        let c = lerp(values[4], values[5], tx);
        let d = lerp(values[6], values[7], tx);
        let near = lerp(a, b, ty);
        let far = lerp(c, d, ty);
        *offset = lerp(near, far, tz);
    }

    *x += offsets[0] * amp;
    *y += offsets[1] * amp;
    *z += offsets[2] * amp;
}

fn normalise_position(length: f32, x: &mut f32, y: &mut f32, z: &mut f32) {
    let length_sq = *x * *x + (*y * *y + *z * *z);
    let inv_mag = length * (1.0 / length_sq.sqrt());
    *x *= inv_mag;
    *y *= inv_mag;
    *z *= inv_mag;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractal_bounding_matches_reference() {
        let bounding = calculate_fractal_bounding(3, 0.5);
        assert!((bounding - 1.0 / 1.75).abs() < 1e-7);
    }

    #[test]
    fn perlin_is_zero_on_lattice() {
        let value = perlin_single(1337, 3.0, 4.0, 5.0);
        assert_eq!(value, 0.0);
    }

    #[test]
    fn fill_matches_single_sample() {
        let mut noise = FastNoise::new(42);
        noise.set_noise_type(NoiseType::PerlinFractal);
        noise.set_perturb_type(PerturbType::GradientFractal);
        let set = noise.get_noise_set(0, 3, 0, 1, 1, 16, 1.0);
        let single = noise.sample(0, 3, 7, 1.0);
        assert_eq!(set[7], single);
    }
}
