//! The strata terraces (`FractalTerraces` and the terrace helpers of
//! `Profiles`: `FractalTerrace`,
//! `ComplexTerraces`, `Iterracions`).

use nerthus_engine::Map;
use rayon::prelude::*;

use crate::combiner;
use crate::gradients::{self, EdgeBehaviour};
use crate::maths::{self, dotnet_max, dotnet_min};
use crate::random::XorShift64Star;
use crate::transformer;
use crate::warps::{self, FractalWarpSettings, IterativeWarpMode, WarpNoiseType};

/// `FractalTerraces.Context`: what every terrace evaluation shares.
struct Strata {
    octaves: i32,
    random: [u8; 512],
    spacing: f32,
    spacing_inverse: f32,
    thickness_randomness: f32,
    hardness_randomness: f32,
    intensity: f32,
    shape: f32,
    shape_spread: f32,
    shape_low: f32,
    shape_high: f32,
    micro_shape: f32,
    micro_shape_spread: f32,
    micro_shape_low: f32,
    micro_shape_high: f32,
    micro_octave: i32,
    character: f32,
    color_contrast: f32,
}

/// One end of the stratum that contains a height.
#[derive(Clone, Copy, Default)]
struct Boundary {
    height: f32,
    position: f32,
    hash: i32,
}

impl Strata {
    fn byte(&self, index: i32) -> f32 {
        self.random[index as usize] as i32 as f32
    }

    /// `terrace2`: the terraced height and the stratum colour of `height`.
    /// Each octave splits the stratum around the height at a random
    /// thickness and lets the upper part lean by a random hardness.
    fn terrace(&self, height: f32) -> (f32, f32) {
        let base_thickness = (1.0 - self.thickness_randomness) * 0.5;
        let scaled = height * self.spacing_inverse;
        let mut stratum = (height * self.spacing_inverse + 1000.0) as i32;
        stratum -= 999;
        let mut lower = Boundary::default();
        let mut upper = Boundary::default();
        let mut contrast;
        let mut color;
        loop {
            stratum -= 1;
            lower.hash = stratum & 0xFF;
            upper.hash = (stratum + 1) & 0xFF;
            lower.height = stratum as f32;
            upper.height = (stratum + 1) as f32;
            contrast = 1.0f32;
            color = lower.height;
            lower.hash += 11;
            lower.height += self.byte(lower.hash) / 255.0 * self.thickness_randomness + 1.0 * base_thickness;
            upper.hash += 11;
            upper.height += self.byte(upper.hash) / 255.0 * self.thickness_randomness + 1.0 * base_thickness;
            lower.position = lower.height;
            upper.position = upper.height;
            if !(scaled < lower.position) {
                break;
            }
        }

        for octave in 0..self.octaves {
            contrast *= self.color_contrast;
            let along = (scaled - lower.position) / (upper.position - lower.position);
            let split = self.byte(lower.hash) / 255.0 * self.thickness_randomness + base_thickness;
            let (spread, shape, low, high) = if octave < self.micro_octave {
                (self.shape_spread, self.shape, self.shape_low, self.shape_high)
            } else {
                (self.micro_shape_spread, self.micro_shape, self.micro_shape_low, self.micro_shape_high)
            };
            let mut lean = (-2.0 * (self.byte(upper.hash) / 255.0) + 1.0) * spread + shape;
            lean = if lean < -1.0 { -1.0 } else { lean };
            lean = if lean > 1.0 { 1.0 } else { lean };
            let side = if along < split { low } else { high };
            lean = lean * self.hardness_randomness + (1.0 - self.hardness_randomness) * side;
            let narrow = dotnet_min(split, 1.0 - split);
            lean = if !(lean < 0.0) {
                lean * ((1.0 - split) * self.character + (1.0 - self.character) * narrow)
            } else {
                lean * (split * self.character + (1.0 - self.character) * narrow)
            };
            lean *= self.intensity;

            let shade = self.byte(lower.hash) / 127.5 - 1.0;
            if along < split {
                color -= contrast * shade;
                upper.height = lower.height + (upper.height - lower.height) * (split + lean);
                upper.position = lower.position + (upper.position - lower.position) * split;
                upper.hash = lower.hash;
            } else {
                color += contrast * shade;
                lower.height += (upper.height - lower.height) * (split + lean);
                lower.position += (upper.position - lower.position) * split;
            }
            lower.hash = self.random[(lower.hash + 11) as usize] as i32;
            upper.hash = self.random[(upper.hash + 13) as usize] as i32;
        }

        let along = if !(upper.position > lower.position) {
            lower.position
        } else {
            (scaled - lower.position) / (upper.position - lower.position)
        };
        let terraced = self.spacing * (lower.height + (upper.height - lower.height) * along);
        (terraced, color)
    }
}

/// Parameters of [`process`] (`FractalTerraces.Process2`).
#[derive(Clone, Copy, Debug)]
pub struct StrataSettings {
    pub spacing: f32,
    pub octaves: i32,
    pub intensity: f32,
    pub seed: i32,
    pub shape: f32,
    /// Below -1, the micro octaves use `shape` too.
    pub micro_shape: f32,
    pub micro_octave: i32,
    pub character: f32,
    pub thickness_uniformity: f32,
    pub hardness_uniformity: f32,
    pub strata_details: f32,
}

/// `FractalTerraces.Process2`: terraces `heightmap` into strata and
/// returns them with the strata layer colours, stretched back to the
/// input's range.
pub fn process(heightmap: &Map, settings: &StrataSettings) -> (Map, Map) {
    let spacing = dotnet_max(settings.spacing, 0.001);
    let shape = settings.shape;
    let micro_shape = if settings.micro_shape < -1.0 { shape } else { settings.micro_shape };
    let mut random = XorShift64Star::new(settings.seed);
    let mut bytes = [0u8; 512];
    for byte in bytes.iter_mut() {
        *byte = (random.next_int() & 0xFF) as u8;
    }
    let strata = Strata {
        octaves: settings.octaves,
        random: bytes,
        spacing,
        spacing_inverse: 1.0 / spacing,
        thickness_randomness: 1.0 - settings.thickness_uniformity,
        hardness_randomness: 1.0 - settings.hardness_uniformity,
        intensity: settings.intensity,
        shape,
        shape_spread: 2.0 - shape.abs(),
        shape_low: 0.5 * dotnet_max(-1.0, shape * 2.0 - 1.0),
        shape_high: 0.5 * dotnet_min(1.0, shape * 2.0 + 1.0),
        micro_shape,
        micro_shape_spread: 2.0 - micro_shape.abs(),
        micro_shape_low: 0.5 * dotnet_max(-1.0, micro_shape * 2.0 - 1.0),
        micro_shape_high: 0.5 * dotnet_min(1.0, micro_shape * 2.0 + 1.0),
        micro_octave: settings.micro_octave,
        character: settings.character,
        color_contrast: 0.5 + 0.5 * settings.strata_details,
    };

    let pairs: Vec<(f32, f32)> = heightmap.data().par_iter().map(|&height| strata.terrace(height)).collect();
    let terraced_values: Vec<f32> = pairs.iter().map(|&(terraced, _)| terraced).collect();
    let layer_values: Vec<f32> = pairs.iter().map(|&(_, color)| color * spacing).collect();

    let resolution = heightmap.resolution();
    let mut terraced = Map::from_data(resolution, terraced_values);
    let (input_minimum, input_maximum) = maths::min_max(heightmap.data());
    let (minimum, maximum) = maths::min_max(terraced.data());
    maths::remap(&mut terraced, maximum, minimum, input_maximum, input_minimum);
    (terraced, Map::from_data(resolution, layer_values))
}

/// `ABCD` style of the tilt warp in [`fractal_terrace`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarpStyle {
    A,
    B,
    C,
    D,
}

impl WarpStyle {
    pub fn from_name(name: &str) -> Self {
        match name {
            "B" => Self::B,
            "C" => Self::C,
            "D" => Self::D,
            _ => Self::A,
        }
    }
}

/// Parameters of [`fractal_terrace`] (`Profiles.FractalTerrace`).
#[derive(Clone, Copy, Debug)]
pub struct FractalTerraceSettings {
    pub spacing: f32,
    pub octaves: i32,
    pub intensity: f32,
    pub shape: f32,
    pub seed: i32,
    pub shapes_separation: bool,
    pub macro_octaves: i32,
    pub micro_shape: f32,
    pub character: f32,
    pub thickness_uniformity: f32,
    pub hardness_uniformity: f32,
    pub strata_details: f32,
    pub protect_range: bool,
    pub tilt: bool,
    pub tilt_amount: f32,
    pub tilt_seed: i32,
    pub direction: i32,
    pub warp_amount: f32,
    pub warp_size: f32,
    pub warp_style: WarpStyle,
}

impl Default for FractalTerraceSettings {
    fn default() -> Self {
        Self {
            spacing: 0.1,
            octaves: 12,
            intensity: 0.5,
            shape: 0.0,
            seed: -1,
            shapes_separation: false,
            macro_octaves: 5,
            micro_shape: 0.0,
            character: 0.0,
            thickness_uniformity: 0.0,
            hardness_uniformity: 0.0,
            strata_details: 0.6,
            protect_range: false,
            tilt: false,
            tilt_amount: 0.5,
            tilt_seed: -1,
            direction: 0,
            warp_amount: 0.33,
            warp_size: 0.33,
            warp_style: WarpStyle::A,
        }
    }
}

/// One `Warps.FractalWarp` of the tilt ramp.
fn warp_tilt(map: &Map, size: f32, strength: f32, noise_type: WarpNoiseType, seed: i32, terrain: (f32, f32)) -> Map {
    let settings = FractalWarpSettings {
        size,
        strength,
        persist_strength: true,
        z_coefficient: 0.0,
        noise_type,
        perturbation: 0.5,
        octaves: 12,
        roughness: 0.5,
        normalized: false,
        iterations: 1,
        mode: IterativeWarpMode::Virtual,
        edges: transformer::EdgeBehaviour::Mirror,
        seed,
        ..FractalWarpSettings::default()
    };
    warps::fractal_warp(map, &settings, None, terrain.0, terrain.1)
}

/// `Profiles.FractalTerrace`: [`process`] on the map, optionally tilted
/// by a warped ramp first (and untilted after) and raised by a modulator.
/// Returns the terraces and their layer colours.
pub fn fractal_terrace(map: &Map, modulator: Option<&Map>, settings: &FractalTerraceSettings, terrain: (f32, f32)) -> (Map, Map) {
    let size = map.width();
    let mut source = map.clone();
    let mut tilt = None;
    if settings.tilt {
        let center = size as f32 / 2.0;
        let mut ramp = gradients::linear(size, center, center, size as f32, settings.direction as f32, EdgeBehaviour::Mirror);
        if settings.tilt_amount < 1.0 {
            ramp = maths::fit_range(&ramp, 0.0, settings.tilt_amount);
        }
        if settings.warp_amount > 0.0 {
            let amount = settings.warp_amount;
            let warp_size = settings.warp_size;
            let seed = settings.tilt_seed;
            ramp = match settings.warp_style {
                WarpStyle::A => warp_tilt(&ramp, warp_size, amount, WarpNoiseType::PerlinFbm, seed, terrain),
                WarpStyle::B => {
                    let first = warp_tilt(&ramp, warp_size, amount, WarpNoiseType::VoronoiR, seed, terrain);
                    warp_tilt(&first, warp_size / 2.0, amount / 2.0, WarpNoiseType::PerlinFbm, seed, terrain)
                }
                WarpStyle::C => {
                    let first = warp_tilt(&ramp, warp_size, amount, WarpNoiseType::VoronoiA, seed, terrain);
                    warp_tilt(&first, warp_size / 2.0, amount / 2.0, WarpNoiseType::PerlinFbm, seed, terrain)
                }
                WarpStyle::D => warp_tilt(&ramp, warp_size, amount, WarpNoiseType::VoronoiR, seed, terrain),
            };
        }
        source = combiner::add(&source, &ramp, 1.0);
        tilt = Some(ramp);
    }
    if let Some(modulator) = modulator {
        source = combiner::add(&source, modulator, 1.0);
    }

    let strata = StrataSettings {
        spacing: settings.spacing,
        octaves: settings.octaves,
        intensity: settings.intensity,
        seed: settings.seed,
        shape: settings.shape,
        micro_shape: if settings.shapes_separation { settings.micro_shape } else { -2.0 },
        micro_octave: settings.macro_octaves,
        character: settings.character,
        thickness_uniformity: settings.thickness_uniformity,
        hardness_uniformity: settings.hardness_uniformity,
        strata_details: settings.strata_details,
    };
    let (mut terraced, layers) = process(&source, &strata);
    if let Some(modulator) = modulator {
        terraced = combiner::subtract(&terraced, modulator, 1.0);
    }
    if let Some(ramp) = &tilt {
        terraced = combiner::subtract(&terraced, ramp, 1.0);
    }
    terraced = if settings.protect_range {
        maths::drop_to_zero(&terraced)
    } else {
        maths::fit_range(&terraced, 0.0, 1.0)
    };
    (terraced, layers)
}

/// `Profiles.ComplexTerraces`: three tilted fractal terrace passes at
/// slightly different spacings and directions, keeping the lowest.
#[allow(clippy::too_many_arguments)]
pub fn complex_terraces(map: &Map, intensity: f32, shape: f32, spacing: f32, tilt_amount: f32, direction: i32, octaves: i32, seed: i32, terrain: (f32, f32)) -> (Map, Map) {
    let mut random = XorShift64Star::new(seed);
    let mut heading = direction;
    let base = FractalTerraceSettings {
        spacing,
        octaves,
        intensity,
        shape,
        seed,
        tilt: true,
        ..FractalTerraceSettings::default()
    };

    let first_tilt = tilt_amount + random.next_float_range(0.3, 0.6);
    let first_seed = random.next_int();
    let first_settings = FractalTerraceSettings {
        tilt_amount: first_tilt,
        tilt_seed: first_seed,
        direction: heading,
        ..base
    };
    let (first, first_layers) = fractal_terrace(map, None, &first_settings, terrain);

    heading += random.next_range(5, 15);
    let second_spacing = spacing + spacing * random.next_float_range(0.01, 0.2);
    let second_tilt = tilt_amount + random.next_float_range(0.3, 0.6);
    let second_seed = random.next_int();
    let second_settings = FractalTerraceSettings {
        spacing: second_spacing,
        tilt_amount: second_tilt,
        tilt_seed: second_seed,
        direction: heading,
        ..base
    };
    let (second, second_layers) = fractal_terrace(&first, None, &second_settings, terrain);
    let lowest = combiner::min(&first, &second, 1.0);
    let lowest_layers = combiner::min(&first_layers, &second_layers, 1.0);

    let third_direction = heading + random.next_range(25, 45);
    let third_spacing = spacing + spacing * random.next_float_range(0.1, 0.4);
    let third_tilt = tilt_amount + random.next_float_range(0.3, 0.6);
    let third_seed = random.next_int();
    let third_settings = FractalTerraceSettings {
        spacing: third_spacing,
        tilt_amount: third_tilt,
        tilt_seed: third_seed,
        direction: third_direction,
        ..base
    };
    let (third, third_layers) = fractal_terrace(&lowest, None, &third_settings, terrain);
    let terraced = combiner::min(&lowest, &third, 1.0);
    let layers = combiner::min(&lowest_layers, &third_layers, 1.0);
    (terraced, layers)
}

/// `Profiles.Iterracions` (`chaos` = None) and `Profiles.Iterracions2`:
/// repeated tilted terrace passes at half spacing with jittered intensity,
/// shape and direction, keeping the lowest. The first form protects the
/// range and lifts the result above zero.
#[allow(clippy::too_many_arguments)]
pub fn iterracions(map: &Map, iterations: i32, intensity: f32, shape: f32, spacing: f32, tilt_amount: f32, direction: i32, octaves: i32, seed: i32, chaos: Option<f32>, terrain: (f32, f32)) -> (Map, Map) {
    let mut random = XorShift64Star::new(seed);
    let mut heading = direction;
    let mut terraced = map.clone();
    let mut layers = Map::filled(map.resolution(), 1.0);
    let chaos_range = chaos.map(|chaos| (100.0 * chaos) as i32);
    for _ in 0..iterations {
        let half = intensity * 0.5;
        let jittered_intensity = half + half * random.next_float();
        let shape_jitter = match chaos_range {
            Some(range) => random.next_float_range(-1.0 / range as f32, 1.0 / range as f32),
            None => random.next_float_range(-0.15, 0.15),
        };
        let tilt_seed = random.next_int();
        let settings = FractalTerraceSettings {
            spacing: spacing * 0.5,
            octaves,
            intensity: jittered_intensity,
            shape: shape + shape_jitter,
            seed,
            protect_range: chaos_range.is_none(),
            tilt: true,
            tilt_amount,
            tilt_seed,
            direction: heading,
            ..FractalTerraceSettings::default()
        };
        let (pass, pass_layers) = fractal_terrace(&terraced, None, &settings, terrain);
        heading += match chaos_range {
            Some(range) => random.next_range(-range, range),
            None => random.next_range(-15, 15),
        };
        terraced = combiner::min(&terraced, &pass, 1.0);
        layers = combiner::min(&layers, &pass_layers, 1.0);
    }
    if chaos_range.is_none() {
        terraced = maths::lift_above_zero(terraced);
    }
    (terraced, layers)
}
