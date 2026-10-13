//! The rock generators (`RockCore`).

use nerthus_engine::{Map, Resolution};
use nerthus_noise::fast_noise::{CellularDistanceFunction, CellularReturnType};

use crate::combiner;
use crate::craterfield;
use crate::erosions;
use crate::fast_blur;
use crate::fractal_terraces;
use crate::gradients;
use crate::masking;
use crate::maths::{self, dotnet_max};
use crate::noise::{self, CrackSettings, CrackStyle, NoisesPerlin, PerlinType, Viewport, VoronoiSettings};
use crate::profiles;
use crate::random::XorShift64Star;
use crate::slope_blur;
use crate::transformer::{self, AnchorPosition, BlendMode, EdgeBehaviour, ResizeMethod, TransformQuality};
use crate::warps::{self, FractalWarpSettings, IterativeWarpMode, WarpNoiseType};

/// `RockCore.Shatter`: stamps warped, edge-faded discs into the map as
/// dents, `shatters` × 2 of them.
pub fn shatter(map: &Map, shatters: i32, size: f32, coverage: f32, seed: i32, terrain: (f32, f32)) -> Map {
    let width = map.width();
    let center = width as f32 * 0.5;
    let disc = gradients::radial(width, center, center, width as f32 * coverage, 1.0);
    let mut random = XorShift64Star::new(seed);

    let mut stamps = Vec::new();
    let mut index = 0;
    while (index as f32) < shatters as f32 / 2.0 {
        let settings = FractalWarpSettings {
            size: random.next_float_range(size * 0.25, size),
            iterations: 8,
            seed: seed + index + 2,
            ..FractalWarpSettings::default()
        };
        let warped = warps::fractal_warp(&disc, &settings, None, terrain.0, terrain.1);
        let shallow = warped.multiply_scalar(0.05);
        stamps.push(masking::mask_by_square_edge(&shallow, 0.7));
        index += 1;
    }

    let mut dents = Map::new(map.resolution(), 1);
    let normalizer = 1.0 / (maths::exp(10.000001) - 1.0);
    for _ in 0..shatters * 2 {
        let stamp = &stamps[random.next_range(0, stamps.len() as i32) as usize];
        let mut depth = random.next_float_range(0.5, 1.0);
        let mut scale = random.next_float();
        scale = (maths::exp(scale * 0.2 * 10.0 + 1e-6) - 1.0) * normalizer;
        scale = 0.7 + 0.19999999 * scale;
        depth *= scale;
        let position = (random.next_float(), random.next_float());
        let rotation = 360.0 * random.next_float_range(0.0, 360.0);
        transformer::multi_transform(
            stamp,
            &mut dents,
            position,
            (scale, scale),
            rotation,
            depth,
            BlendMode::Subtract,
            TransformQuality::Medium,
            None,
            true,
            AnchorPosition::Center,
        );
    }
    map.add(&dents)
}

/// `RockClusters`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RockClusters {
    Off,
    Large,
    Small,
    Shattered,
}

impl RockClusters {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Large" => Self::Large,
            "Small" => Self::Small,
            "Shattered" => Self::Shattered,
            _ => Self::Off,
        }
    }
}

/// `RockRelief`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RockRelief {
    Off,
    Sharp,
    Soft,
    Stratified,
}

impl RockRelief {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Sharp" => Self::Sharp,
            "Soft" => Self::Soft,
            "Stratified" => Self::Stratified,
            _ => Self::Off,
        }
    }

    fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Sharp,
            2 => Self::Soft,
            3 => Self::Stratified,
            _ => Self::Off,
        }
    }
}

/// Settings of [`large`] (`RockCore.Large`).
#[derive(Clone, Copy, Debug)]
pub struct LargeRockSettings {
    pub size: f32,
    pub fracture: f32,
    pub roughness: f32,
    pub breakage: f32,
    pub surface: f32,
    pub clusters: RockClusters,
    pub relief: RockRelief,
    pub jitter: f32,
    pub seed: i32,
}

/// `low + t × (high − low)`.
fn lerp(t: f32, low: f32, high: f32) -> f32 {
    low + t * (high - low)
}

fn perlin(size: usize, scale: f32, kind: PerlinType, seed: i32) -> Map {
    let parameters = NoisesPerlin {
        scale,
        octaves: 10,
        gain: 0.5,
        kind,
        seed,
        ..NoisesPerlin::default()
    };
    noise::noises_perlin(size, &parameters)
}

#[allow(clippy::too_many_arguments)]
fn crack_field(size: usize, scale: f32, seed: i32, stretch: f32, warp_size: f32, warp_strength: f32, depth: f32, terrain: (f32, f32)) -> Map {
    let settings = CrackSettings {
        octaves: 1,
        scale,
        seed,
        scale_x: stretch,
        scale_y: stretch,
        jitter: 0.5,
        warp_size,
        warp_strength,
        style: CrackStyle::Normal,
        depth,
    };
    noise::cracks(Resolution::square(size), &settings, terrain)
}

/// `RockCore.Large`: blurred Perlin cells broken by warps and recurved
/// into boulders, combined with flattened Voronoi faces, optionally
/// cracked into clusters and given relief.
pub fn large(size: usize, settings: &LargeRockSettings, terrain: (f32, f32)) -> Map {
    let mut seed = settings.seed;
    let jitter = settings.jitter;
    let mut fracture = lerp(settings.fracture, 0.01, 0.75);
    let mut breakage = settings.breakage;
    match settings.clusters {
        RockClusters::Small => {
            breakage = dotnet_max(breakage, 0.7);
            fracture = dotnet_max(fracture, 0.45);
        }
        RockClusters::Shattered => {
            breakage = dotnet_max(breakage, 0.9);
            fracture = dotnet_max(fracture, 0.5);
        }
        _ => {}
    }
    let first = perlin(size, fracture, PerlinType::Fbm, seed);
    seed += 1;
    let second = perlin(size, fracture, PerlinType::Fbm, seed);
    seed += 1;
    let third = perlin(size, fracture, PerlinType::Billowy, seed);
    seed += 1;
    let cells = combiner::max(&combiner::max(&first, &second, 1.0), &third, 1.0);

    let radius = dotnet_max(1.0, lerp(1.0 - breakage, 0.01, 4.0) * 0.005 * size as f32);
    let blur_iterations = (20.0 * (1.0 - settings.size)) as i32;
    let blurred = fast_blur::managed_box_blur(cells.data(), size, radius, blur_iterations);
    let blurred = Map::from_data(cells.resolution(), blurred);
    let cell_warp = FractalWarpSettings {
        size: 0.07,
        strength: 0.05,
        normalized: true,
        iterations: 32,
        mode: IterativeWarpMode::Integral,
        edges: EdgeBehaviour::Mirror,
        seed,
        jitter,
        ..FractalWarpSettings::default()
    };
    let warped = warps::fractal_warp(&blurred, &cell_warp, None, terrain.0, terrain.1);
    seed += 1;
    let upper = combiner::max(&cells, &warped, 1.0);
    let lower = combiner::min(&cells, &warped, 1.0);
    let difference = combiner::difference(&upper, &lower, 1.0);
    let recurved = profiles::recurve(&difference, 1.0, 3, true, true, 0.1);
    let fracture_warp = FractalWarpSettings {
        size: 0.08,
        strength: 0.25,
        noise_type: WarpNoiseType::VoronoiA,
        normalized: true,
        iterations: 2,
        mode: IterativeWarpMode::Real,
        edges: EdgeBehaviour::Mirror,
        seed,
        jitter,
        ..FractalWarpSettings::default()
    };
    let fractured = warps::fractal_warp(&recurved, &fracture_warp, None, terrain.0, terrain.1);

    let faces = VoronoiSettings {
        scale: 1.0 - settings.size,
        function: CellularDistanceFunction::Euclidean,
        form: CellularReturnType::Distance2Sub,
        warp: noise::perturb_for_warp(2),
        warp_frequency: 0.3,
        warp_amplitude: 0.1,
        warp_octaves: 10,
        seed,
        scale_x: 1.0,
        scale_y: 1.0,
        jitter,
    };
    let mut flat = noise::voronoi(size, &faces, Viewport::with_offset([0.0, 0.0]));
    seed += 1;
    flat = maths::fit_range(&flat, 0.0, 0.16);
    maths::strengthen(&mut flat, 0.45);
    flat = maths::fit_range(&flat, 0.0, 0.13);
    let face_warp = FractalWarpSettings {
        size: 0.5,
        strength: settings.roughness * 1.25,
        edges: EdgeBehaviour::Mirror,
        seed,
        jitter,
        ..FractalWarpSettings::default()
    };
    flat = warps::fractal_warp(&flat, &face_warp, None, terrain.0, terrain.1);
    let mut rock = combiner::min(&fractured, &flat, 1.0);

    match settings.clusters {
        RockClusters::Large | RockClusters::Small => {
            let mut cracks = crack_field(size, 0.25, seed + 99, 2.0, 0.35, 0.5, 1.0, terrain);
            if settings.clusters == RockClusters::Small {
                let fine = crack_field(size, 0.55, seed + 98, 1.5, 0.25, 0.65, 0.65, terrain);
                cracks = combiner::max(&cracks, &fine, 1.0);
            }
            let first_warp = FractalWarpSettings {
                size: 0.15,
                strength: 0.45,
                edges: EdgeBehaviour::Mirror,
                seed: seed + 100,
                ..FractalWarpSettings::default()
            };
            cracks = warps::fractal_warp(&cracks, &first_warp, None, terrain.0, terrain.1);
            let second_warp = FractalWarpSettings {
                size: 0.3,
                strength: 0.65,
                edges: EdgeBehaviour::Mirror,
                seed: seed + 101,
                ..FractalWarpSettings::default()
            };
            cracks = warps::fractal_warp(&cracks, &second_warp, None, terrain.0, terrain.1);
            rock = combiner::subtract(&rock, &cracks, 0.075);
            let (minimum, maximum) = maths::min_max(rock.data());
            if maximum > 1.0 || minimum < 0.0 {
                maths::clamp(&mut rock);
            }
        }
        RockClusters::Shattered => rock = maths::clip_map(&rock, 0.01, 1.0),
        RockClusters::Off => {}
    }

    match settings.relief {
        RockRelief::Sharp => {
            let guide_warp = FractalWarpSettings {
                size: 0.75,
                roughness: 0.45,
                seed,
                ..FractalWarpSettings::default()
            };
            let guide = warps::fractal_warp(&rock, &guide_warp, None, terrain.0, terrain.1);
            let warp = slope_blur::WarpSettings {
                intensity: 0.03,
                iterations: 1,
                direction: 0.0,
                normalized: false,
                quality: 0,
                ssaa: 0,
                spectral_quality: 4,
            };
            let sharpened = slope_blur::warp(rock.data(), guide.data(), None, size, &warp);
            let sharpened = Map::from_data(rock.resolution(), sharpened);
            rock = combiner::min(&rock, &sharpened, 1.0);
        }
        RockRelief::Soft => {
            let blur = slope_blur::BlurSettings {
                intensity: 0.05,
                quality: 0,
                direction: 0.0,
                bias: 1.0,
                normalized: false,
                window: slope_blur::Window::Bell,
                ssaa: 0,
                spectral_quality: 4,
                iterations: 1,
            };
            let softened = slope_blur::blur(rock.data(), rock.data(), size, &blur);
            rock = Map::from_data(rock.resolution(), softened);
        }
        RockRelief::Stratified => {
            let guide = perlin(size, 0.25, PerlinType::Fbm, seed + 50);
            let mask = maths::autolevel(perlin(size, 0.05, PerlinType::Fbm, seed + 52));
            let warp = slope_blur::WarpSettings {
                intensity: 0.085,
                iterations: 1,
                direction: 0.0,
                normalized: false,
                quality: 1,
                ssaa: 0,
                spectral_quality: 8,
            };
            let strata = slope_blur::warp(rock.data(), guide.data(), None, size, &warp);
            let strata = Map::from_data(rock.resolution(), strata);
            rock = masking::mask(&rock, &strata, &mask);
        }
        RockRelief::Off => {}
    }
    rock
}

/// `RockCore.MakeNoiseStamps`: `variety` small-cluster rocks with random
/// relief, faded at the edges.
pub fn make_noise_stamps(size: usize, variety: i32, seed: i32, terrain: (f32, f32)) -> Vec<Map> {
    let mut random = XorShift64Star::new(seed);
    let mut stamps = Vec::new();
    for index in 0..variety {
        let settings = LargeRockSettings {
            size: 0.5,
            fracture: 0.9,
            roughness: 0.7,
            breakage: 0.25,
            surface: 0.5,
            clusters: RockClusters::Small,
            relief: RockRelief::from_index(random.next_range(0, 4)),
            jitter: 0.5,
            seed: seed + index,
        };
        let rock = large(size, &settings, terrain);
        let faded = masking::mask_by_square_edge(&rock, 0.9);
        stamps.push(transformer::resize_with(&faded, Resolution::square(size), ResizeMethod::HighQuality));
    }
    stamps
}

/// `RockCore.Noise`: `octaves` random rock stamps scattered by maximum.
pub fn noise(size: usize, variety: i32, octaves: i32, scale_range: (f32, f32), seed: i32, terrain: (f32, f32)) -> Map {
    let stamps = make_noise_stamps(size, variety, seed, terrain);
    let mut map = Map::new(Resolution::square(size), 1);
    let mut random = XorShift64Star::new(seed);
    let (low, high) = scale_range;
    for _ in 0..octaves {
        let stamp = &stamps[random.next_range(0, stamps.len() as i32) as usize];
        let position = (random.next_float(), random.next_float());
        let scale = random.next_float_range(low, high);
        let rotation = 360.0 * random.next_float();
        transformer::multi_transform(
            stamp,
            &mut map,
            position,
            (scale, scale),
            rotation,
            low + (high - low),
            BlendMode::Max,
            TransformQuality::High,
            None,
            true,
            AnchorPosition::Center,
        );
    }
    map
}

/// Settings of [`sandstone`] (`RockCore.Sandstone`).
#[derive(Clone, Copy, Debug)]
pub struct SandstoneSettings {
    pub passes: i32,
    pub iterations: i32,
    pub spacing: f32,
    pub intensity: f32,
    pub convexity: f32,
    pub tilt: f32,
    pub direction: i32,
    pub chaos: f32,
    pub seed: i32,
    pub chipped: bool,
    pub chipping: f32,
}

impl Default for SandstoneSettings {
    fn default() -> Self {
        Self {
            passes: 1,
            iterations: 10,
            spacing: 0.1,
            intensity: 0.5,
            convexity: 0.0,
            tilt: 0.5,
            direction: 0,
            chaos: 0.15,
            seed: 0,
            chipped: true,
            chipping: 0.45,
        }
    }
}

/// `RockCore.Sandstone`: chaotic iterative terraces stretched back to the
/// input's range, optionally chipped by small crater fields. Returns the
/// rock and its layers.
pub fn sandstone(map: &Map, settings: &SandstoneSettings, terrain: (f32, f32)) -> (Map, Map) {
    let original = map.clone();
    let mut rock = map.clone();
    let mut layers = Map::filled(map.resolution(), 1.0);
    for pass in 0..settings.passes {
        let (minimum, maximum) = maths::min_max(rock.data());
        let seed = settings.seed + pass;
        let (mut terraced, mut pass_layers) = fractal_terraces::iterracions(
            &rock,
            settings.iterations,
            lerp(settings.intensity, 0.0, 0.5),
            settings.convexity,
            lerp(settings.spacing, 0.0, 0.2),
            settings.tilt,
            settings.direction,
            12,
            seed,
            Some(settings.chaos),
            terrain,
        );
        let (terraced_minimum, terraced_maximum) = maths::min_max(terraced.data());
        maths::remap(&mut terraced, terraced_maximum, terraced_minimum, maximum, minimum);
        if settings.chipped {
            let chips = craterfield::craterfield(&terraced, settings.chipping, 0.2, 0.0003, 0.0, 0.0, seed);
            terraced = combiner::min(&terraced, &chips, 1.0);
            let layer_chips = craterfield::craterfield(&pass_layers, settings.chipping, 0.2, 0.0003, 0.0, 0.0, seed);
            pass_layers = combiner::min(&pass_layers, &layer_chips, 1.0);
        }
        rock = terraced;
        layers = combiner::min(&pass_layers, &layers, 1.0);
    }
    let change = combiner::difference(&rock, &original, 1.0);
    layers = layers.subtract(&change);
    (rock, layers)
}

/// Settings of [`outcrops`] (`RockCore.Outcrops`), with the node's
/// defaults.
#[derive(Clone, Copy, Debug)]
pub struct OutcropSettings {
    pub variations: i32,
    pub strata: f32,
    pub density: f32,
    pub shape: f32,
    pub chipped: bool,
    pub seed: i32,
    /// Size range of the scattered outcrops.
    pub size: [f32; 2],
    /// Height range.
    pub height: [f32; 2],
    /// Rotation range in turns.
    pub rotation: [f32; 2],
}

impl Default for OutcropSettings {
    fn default() -> Self {
        Self {
            variations: 3,
            strata: 0.1,
            density: 0.2,
            shape: 0.0,
            chipped: true,
            seed: 0,
            size: [0.4, 0.8],
            height: [0.45, 0.8],
            rotation: [0.0, 0.6],
        }
    }
}

/// `RockCore.Outcrops`: a few terraced, warped domes scattered over the
/// terrain (largest wins), crumbled and optionally chipped by craters.
pub fn outcrops(input: &Map, settings: &OutcropSettings, terrain: (f32, f32)) -> Map {
    let resolution = input.resolution();
    let size = resolution.x as f32;
    let dome = gradients::hemisphere_flattened(resolution, size * 0.5, size * 0.5, size * 0.3, 0.25);
    let mut random = XorShift64Star::new(settings.seed);
    let mut stamps = Vec::new();
    for variation in 0..settings.variations {
        let iterations = random.next_range(1, 4);
        let warp = FractalWarpSettings {
            size: 0.2,
            strength: 0.5,
            iterations,
            seed: settings.seed + variation,
            ..FractalWarpSettings::default()
        };
        let warped = warps::fractal_warp(&dome, &warp, None, terrain.0, terrain.1);
        let spacing = random.next_float_range(settings.strata, settings.strata * 2.0);
        let terraced = fractal_terraces::complex_terraces(&warped, 0.5, settings.shape, spacing, 0.8, 0, 12, settings.seed + variation + 2, terrain).0;
        let clipped = maths::clip_map(&terraced, 0.02, 1.0);
        stamps.push(masking::mask_by_square_edge(&clipped, 0.7));
    }

    let mut field = Map::new(resolution, 1);
    let normalizer = 1.0 / (maths::exp(10.000001) - 1.0);
    let count = (settings.density * 300.0) as i32;
    for _ in 0..count {
        let stamp = &stamps[random.next_range(0, stamps.len() as i32) as usize];
        let mut height = random.next_float_range(settings.height[0], settings.height[1]);
        let mut scale = random.next_float_range(settings.size[0], settings.size[1]);
        scale = (maths::exp(scale * 10.0 + 1e-6) - 1.0) * normalizer;
        scale = settings.size[0] + (settings.size[1] - settings.size[0]) * scale;
        height *= scale;
        let position = (random.next_float(), random.next_float());
        let rotation = 360.0 * random.next_float_range(settings.rotation[0], settings.rotation[1]);
        transformer::multi_transform(stamp, &mut field, position, (scale, scale), rotation, height, BlendMode::Max, TransformQuality::Medium, None, true, AnchorPosition::Center);
    }

    let raised = input.add(&field);
    let crumble = erosions::CrumbleSettings {
        downcutting: 0.1,
        ..erosions::CrumbleSettings::default()
    };
    let [mut result, _] = erosions::crumble(&raised, None, &crumble, terrain);
    if settings.chipped {
        let craters = craterfield::craterfield(&result, 0.1, 0.4, 0.0005, 0.0, 0.0, settings.seed);
        result = combiner::min(&result, &craters, 1.0);
        let craters = craterfield::craterfield(&result, 0.15, 0.2, 0.0005, 0.0, 0.0, settings.seed + 5);
        result = combiner::min(&result, &craters, 1.0);
    }
    result
}
