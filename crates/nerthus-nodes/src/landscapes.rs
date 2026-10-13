//! The landscape generators (`Landscapes`) and the surfaces built from
//! them (`Surfaces`).

use nerthus_engine::{Map, Resolution};
use nerthus_noise::fast_noise::{CellularDistanceFunction, CellularReturnType};

use crate::aspect;
use crate::combiner::{self, BlendMode};
use crate::fast_blur;
use crate::fractal_terraces::{self, FractalTerraceSettings};
use crate::gradients;
use crate::masking;
use crate::morph::{self, DistanceRtMode, DistanceRtSettings};
use crate::multi_fractal::{self, MultiFractalSettings, MultifractalType};
use crate::maths;
use crate::noise::{self, GaborSettings, NoisesPerlin, Viewport, VoronoiSettings};
use crate::parallel_erosion::{self, ErosionSettings};
use crate::profiles;
use crate::random::XorShift64Star;
use crate::rock_core::{self, LargeRockSettings, RockClusters, RockRelief};
use crate::slope_blur;
use crate::transformer::EdgeBehaviour;
use crate::warps::{self, FractalWarpSettings, WarpNoiseType};

/// Terrain width and height in metres.
pub type TerrainSize = (f32, f32);

/// The three-way style switch (`ABC`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Abc {
    A,
    B,
    C,
}

/// `Landscapes.Slump`: Perlin relief cut by a Voronoi-warped copy of
/// itself.
pub fn slump(size: usize, scale: f32, noise_style: Abc, style: Abc, seed: i32, terrain: TerrainSize) -> Map {
    let octaves = match noise_style {
        Abc::A => 2,
        Abc::B => 4,
        Abc::C => 6,
    };
    let noise_type = match style {
        Abc::A => WarpNoiseType::VoronoiR,
        Abc::B => WarpNoiseType::VoronoiA,
        Abc::C => WarpNoiseType::VoronoiM,
    };
    let base_parameters = NoisesPerlin {
        scale: 0.7 + scale * (0.99 - 0.7),
        octaves,
        gain: 0.5,
        seed,
        warp_type: 2,
        frequency: 0.05,
        amplitude: 0.5,
        warp_octaves: 12,
        ..NoisesPerlin::default()
    };
    let base = noise::noises_perlin(size, &base_parameters);
    let detail_parameters = NoisesPerlin {
        scale: 0.5,
        octaves: 10,
        gain: 0.9,
        seed: seed + 2,
        ..NoisesPerlin::default()
    };
    let detail = noise::noises_perlin(size, &detail_parameters);
    let combined = base.add(&detail.multiply_scalar(0.005));
    let warp = FractalWarpSettings {
        size: 0.5,
        strength: 1.0,
        noise_type,
        edges: EdgeBehaviour::Mirror,
        seed: seed + 1,
        ..FractalWarpSettings::default()
    };
    let warped = warps::fractal_warp(&combined, &warp, None, terrain.0, terrain.1);
    let lowest = combiner::min(&combined, &warped, 1.0);
    maths::clip_map(&lowest, 0.25, 1.0)
}

/// Settings of [`ridge`].
#[derive(Clone, Copy, Debug)]
pub struct RidgeSettings {
    pub scale: f32,
    pub height: f32,
    pub definition: f32,
    pub seed: i32,
    pub scale_x: f32,
    pub scale_y: f32,
}

/// `Landscapes.Ridge`: Voronoi ridges pushed around by terraced Perlin
/// noise and cut by a Voronoi-warped copy.
pub fn ridge(size: usize, settings: &RidgeSettings, terrain: TerrainSize) -> Map {
    let seed = settings.seed;
    let voronoi_settings = VoronoiSettings {
        scale: 1.0 - settings.scale * 1.05,
        function: CellularDistanceFunction::Euclidean,
        form: CellularReturnType::Distance2,
        warp: noise::perturb_for_warp(2),
        warp_frequency: 0.1,
        warp_amplitude: 0.3,
        warp_octaves: 10,
        seed,
        scale_x: settings.scale_x,
        scale_y: settings.scale_y,
        jitter: 0.45,
    };
    let cells = noise::voronoi(size, &voronoi_settings, Viewport::with_offset([0.0, 0.0]));
    let perlin_parameters = NoisesPerlin {
        scale: 0.75,
        octaves: 12,
        gain: 0.5,
        seed: seed + 1,
        warp_type: 0,
        frequency: 0.0,
        amplitude: 0.0,
        warp_octaves: 0,
        ..NoisesPerlin::default()
    };
    let perlin = noise::noises_perlin(size, &perlin_parameters);
    let mut terraced = profiles::terrace(&perlin, 67, 0.6, 0.2, 0.8, seed + 2, false);
    let terrace_warp = FractalWarpSettings {
        size: 1.0 - settings.definition * 0.99,
        strength: 1.0,
        seed: seed + 3,
        ..FractalWarpSettings::default()
    };
    let warped_terraces = warps::fractal_warp(&terraced, &terrace_warp, None, terrain.0, terrain.1);
    terraced = combiner::max(&terraced, &warped_terraces, 1.0);
    let strength = settings.definition * 1.25 * size as f32;
    let pushed = warps::direction_warp(cells.data(), terraced.data(), size, strength, 45.0, EdgeBehaviour::Mirror);
    let pushed = Map::from_data(Resolution::square(size), pushed);
    let cut_warp = FractalWarpSettings {
        size: settings.scale / 2.0,
        strength: 0.65,
        noise_type: WarpNoiseType::VoronoiP,
        perturbation: 0.4,
        edges: EdgeBehaviour::Mirror,
        seed: seed + 4,
        ..FractalWarpSettings::default()
    };
    let cut = warps::fractal_warp(&pushed, &cut_warp, None, terrain.0, terrain.1);
    let lowest = combiner::min(&pushed, &cut, 1.0);
    let fitted = maths::fit_range(&lowest, 0.0, settings.scale);
    let dropped = maths::drop_to_zero(&fitted);
    dropped.multiply_scalar(settings.height)
}

/// `Landscapes.Rugged`: slumps and ridges combined by `style` (only the
/// last of `complexity` rounds is kept).
pub fn rugged(size: usize, style: Abc, scale: f32, complexity: i32, seed: i32, terrain: TerrainSize) -> Option<Map> {
    let mut random = XorShift64Star::new(seed);
    let mut next_seed = seed;
    let mut result = None;
    for _ in 0..complexity {
        let first_scale = scale * random.next_float_range(0.7, 1.0);
        let first = slump(size, first_scale, Abc::A, Abc::A, next_seed, terrain);
        next_seed += 1;
        let second_scale = scale * random.next_float_range(0.7, 1.0);
        let second = slump(size, second_scale, Abc::A, Abc::A, next_seed, terrain);
        next_seed += 1;
        let slumped = combiner::max(&first, &second, 1.0);
        let ridge_settings = RidgeSettings {
            scale: (scale + 0.25) * random.next_float_range(0.7, 1.0),
            height: 0.6,
            definition: 0.4,
            seed: next_seed,
            scale_x: 1.0,
            scale_y: 1.0,
        };
        let ridges = ridge(size, &ridge_settings, terrain);
        next_seed += 1;
        let mode = match style {
            Abc::A => BlendMode::Screen,
            Abc::C => BlendMode::Difference,
            Abc::B => BlendMode::Blend,
        };
        result = Some(combiner::mix(mode, 0.5, &slumped, &ridges));
    }
    result
}

/// `Surfaces.M2`: four slumps at different scales, differenced.
pub fn surface_m2(size: usize, scale: f32, seed: i32, terrain: TerrainSize) -> Map {
    let first = slump(size, scale, Abc::A, Abc::A, seed, terrain);
    let second = slump(size, scale * 1.2, Abc::A, Abc::A, seed + 1, terrain);
    let third = slump(size, scale * 0.8, Abc::A, Abc::A, seed + 2, terrain);
    let fourth = slump(size, scale * 1.6, Abc::A, Abc::A, seed + 3, terrain);
    let difference = first.subtract(&second);
    let average = third.multiply_scalar(0.5).add(&fourth.multiply_scalar(0.5));
    difference.add(&average)
}

/// `low + t × (high − low)`.
fn lerp(t: f32, low: f32, high: f32) -> f32 {
    low + t * (high - low)
}

/// `Profiles.GraphicEQ(map, 0)`: the map without its lowest band.
fn without_lowest_band(map: &Map) -> Map {
    profiles::graphic_eq(map, [0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0])
}

/// `Surfaces.M3`: band-limited slumps embedded under terraced ridges,
/// with large rocks on top.
pub fn surface_m3(size: usize, scale: f32, seed: i32, terrain: TerrainSize) -> Map {
    let scale = lerp(scale, 0.4, 0.5);
    let first = slump(size, 2.0 * scale, Abc::A, Abc::A, seed, terrain);
    let second = slump(size, 2.0 * scale * 1.2, Abc::A, Abc::A, seed + 1, terrain);
    let third = slump(size, 2.0 * scale * 0.8, Abc::A, Abc::A, seed + 2, terrain);
    let fourth = slump(size, 2.0 * scale * 1.6, Abc::A, Abc::A, seed + 3, terrain);
    let relief = without_lowest_band(&first.subtract(&second).add(&third.subtract(&fourth)));

    let ridges = RidgeSettings {
        scale: scale + 0.25,
        height: 0.6,
        definition: 0.4,
        seed: seed + 6,
        scale_x: 1.0,
        scale_y: 1.0,
    };
    let mut ridges = ridge(size, &ridges, terrain);
    let first_terraces = FractalTerraceSettings {
        seed: seed + 1,
        protect_range: true,
        ..FractalTerraceSettings::default()
    };
    let terraced = fractal_terraces::fractal_terrace(&ridges, None, &first_terraces, terrain).0;
    ridges = combiner::min(&ridges, &terraced, 1.0);
    let second_terraces = FractalTerraceSettings {
        seed: seed + 2,
        direction: 45,
        ..first_terraces
    };
    let terraced = fractal_terraces::fractal_terrace(&ridges, None, &second_terraces, terrain).0;
    ridges = combiner::min(&ridges, &terraced, 1.0);

    let raised = relief.add_scalar(0.6);
    let embedded = combiner::embed(&raised, &maths::drop_to_zero(&ridges), 0.65, 0.0);
    let rocks = LargeRockSettings {
        size: 0.5,
        fracture: 0.5,
        roughness: 0.7,
        breakage: 0.25,
        surface: 0.5,
        clusters: RockClusters::Large,
        relief: RockRelief::Off,
        jitter: 0.5,
        seed: seed + 21,
    };
    let boulders = rock_core::large(size, &rocks, terrain).multiply_scalar(0.3);
    maths::drop_to_zero(&embedded).add(&boulders)
}

/// `Surfaces.M4`: differenced slumps without their lowest band, pushed
/// twice by ridge fields, given ridges and eroded.
pub fn surface_m4(size: usize, scale: f32, seed: i32, terrain: TerrainSize) -> Map {
    let scale = lerp(scale, 0.4, 0.65);
    let mut random = XorShift64Star::new(seed);
    let first = slump(size, scale * 1.3, Abc::A, Abc::A, seed, terrain);
    let second = slump(size, scale * 1.2, Abc::A, Abc::A, seed + 1, terrain);
    let third = slump(size, scale * 2.0, Abc::A, Abc::A, seed + 2, terrain);
    let relief = first.subtract(&second).add(&third);
    let relief = maths::drop_to_zero(&without_lowest_band(&relief));

    let push_ridges = RidgeSettings {
        scale: scale * 1.8,
        height: 0.6,
        definition: 0.4,
        seed: seed + 5,
        scale_x: 1.0,
        scale_y: 1.0,
    };
    let ridges = ridge(size, &push_ridges, terrain);
    let pushed = warps::direction_warp_map(&relief, &ridges, 1.0, random.next_range(0, 360) as f32, EdgeBehaviour::Mirror);
    let relief = combiner::min(&relief, &pushed, 1.0);
    let ridges = ridge(size, &push_ridges, terrain);
    let pushed = warps::direction_warp_map(&relief, &ridges, 1.0, random.next_range(0, 360) as f32, EdgeBehaviour::Mirror);
    let lowest = combiner::min(&relief, &pushed, 1.0);
    let top_ridges = RidgeSettings {
        scale: scale * 1.6,
        seed: seed + 6,
        ..push_ridges
    };
    let relief = lowest.add(&ridge(size, &top_ridges, terrain).multiply_scalar(0.5));

    let erosion = ErosionSettings {
        duration: 37.661713,
        bed_load_amount: 0.63589674,
        coarse_sediments_amount: 0.5446682,
        shape_sharpness: 0.09055278,
        shape_detail_scale: 0.03784295,
        seed: 8989,
        ..ErosionSettings::default()
    };
    let worn = eroded(&relief, erosion, terrain);
    combiner::min(&relief, &worn, 1.0)
}

/// `Surfaces.M5`: [`surface_m2`]-like slumps without their lowest band,
/// iteratively terraced.
pub fn surface_m5(size: usize, scale: f32, seed: i32, terrain: TerrainSize) -> Map {
    let scale = lerp(scale, 0.45, 0.65);
    let first = slump(size, 2.0 * scale, Abc::A, Abc::A, seed, terrain);
    let second = slump(size, 2.0 * scale * 1.2, Abc::A, Abc::A, seed + 1, terrain);
    let third = slump(size, 2.0 * scale * 0.8, Abc::A, Abc::A, seed + 2, terrain);
    let fourth = slump(size, 2.0 * scale * 1.6, Abc::A, Abc::A, seed + 3, terrain);
    let difference = first.subtract(&second);
    let average = third.multiply_scalar(0.5).add(&fourth.multiply_scalar(0.5));
    let relief = maths::drop_to_zero(&without_lowest_band(&difference.add(&average)));
    let direction = XorShift64Star::new(seed).next_range(0, 360);
    fractal_terraces::iterracions(&relief, 6, 0.9, 0.43, 0.033, 0.5, direction, 2, seed + 8, None, terrain).0
}

/// `Surfaces.M6`: blended slumps cut by a slope warp along ridges.
pub fn surface_m6(size: usize, scale: f32, seed: i32, terrain: TerrainSize) -> Map {
    let scale = lerp(scale, 0.05, 0.3);
    let half = scale * 0.5;
    let first = slump(size, half + 0.38, Abc::A, Abc::A, seed, terrain);
    let second = slump(size, half + 0.46, Abc::A, Abc::A, seed + 1, terrain);
    let third = slump(size, half + 0.66, Abc::A, Abc::A, seed + 2, terrain);
    let fourth = slump(size, half + 0.6, Abc::A, Abc::A, seed + 3, terrain);
    let upper = combiner::blend_maps(&third, &fourth, 0.5);
    let lower = combiner::blend_maps(&first, &second, 0.5);
    let blended = combiner::blend_maps(&lower, &upper, 1.0).multiply_scalar(2.0);
    let surface = maths::fit_range(&maths::autolevel(blended), 0.0, 0.4);

    let guide_settings = RidgeSettings {
        scale,
        height: 0.6,
        definition: 0.4,
        seed: seed + 16,
        scale_x: 1.0,
        scale_y: 1.0,
    };
    let guide = ridge(size, &guide_settings, terrain);
    let warp = slope_blur::WarpSettings {
        intensity: 0.15,
        iterations: 1,
        direction: 0.0,
        normalized: false,
        quality: 1,
        ssaa: 0,
        spectral_quality: 12,
    };
    let warped = slope_blur::warp(surface.data(), guide.data(), None, size, &warp);
    let warped = Map::from_data(surface.resolution(), warped);
    combiner::min(&surface, &warped, 1.0)
}

/// `MountainStyle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountainStyle {
    Basic,
    Eroded,
    Old,
    Alpine,
    Strata,
}

impl MountainStyle {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Basic" => Self::Basic,
            "Old" => Self::Old,
            "Alpine" => Self::Alpine,
            "Strata" => Self::Strata,
            _ => Self::Eroded,
        }
    }
}

/// `MountainBulk`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountainBulk {
    Low,
    Medium,
    High,
}

impl MountainBulk {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Low" => Self::Low,
            "High" => Self::High,
            _ => Self::Medium,
        }
    }
}

/// `Erosions.PE` without a precipitation mask; returns the eroded height.
fn eroded(map: &Map, settings: ErosionSettings, terrain: TerrainSize) -> Map {
    parallel_erosion::erode(map, None, &settings, terrain).height
}

/// Settings of [`mountain`].
#[derive(Clone, Copy, Debug)]
pub struct MountainSettings {
    pub scale: f32,
    pub height: f32,
    pub reduce_detail: bool,
    pub style: MountainStyle,
    pub bulk: MountainBulk,
    pub seed: i32,
    pub x: f32,
    pub y: f32,
}

/// `Landscapes.Mountain`: three ridge fields under a radial falloff,
/// roughened by a warp and eroded per style.
pub fn mountain(resolution: Resolution, settings: &MountainSettings, terrain: TerrainSize) -> Map {
    let seed = settings.seed;
    let size = resolution.x;
    let mut random = XorShift64Star::new(seed);
    let first = RidgeSettings {
        scale: random.next_float_range(0.5, 0.75),
        height: 0.6,
        definition: 0.4,
        seed,
        scale_x: 1.0,
        scale_y: 1.0,
    };
    let first_ridges = ridge(size, &first, terrain);
    let second_scale = random.next_float_range(0.65, 0.95);
    let second = RidgeSettings {
        scale: second_scale,
        definition: random.next_float_range(0.5, 1.0),
        seed: seed + 1,
        ..first
    };
    let second_ridges = ridge(size, &second, terrain);
    let third_scale = random.next_float_range(0.35, 0.7);
    let third = RidgeSettings {
        scale: third_scale,
        definition: random.next_float_range(0.1, 0.6),
        seed: seed + 2,
        ..first
    };
    let third_ridges = ridge(size, &third, terrain);
    let mut peaks = first_ridges.add(&second_ridges).add(&third_ridges);

    let center_x = resolution.x as f32 * settings.x;
    let center_y = resolution.y as f32 * settings.y;
    let falloff = gradients::radial(size, center_x, center_y, resolution.x as f32 * settings.scale, 1.0);
    peaks = peaks.multiply(&falloff);
    if settings.style != MountainStyle::Alpine {
        let warp = FractalWarpSettings {
            seed: seed + 36,
            ..FractalWarpSettings::default()
        };
        let warped = warps::fractal_warp(&peaks, &warp, None, terrain.0, terrain.1);
        peaks = combiner::min(&peaks, &warped, 1.0);
    }
    peaks = peaks.multiply_scalar(settings.height);

    let reduce = settings.reduce_detail;
    let defaults = ErosionSettings::default();
    let mut mountain = match settings.style {
        MountainStyle::Eroded => {
            let erosion = ErosionSettings {
                duration: 10.0,
                downcutting: 0.15,
                erosion_scale: 1400.0,
                shape: 0.8,
                shape_sharpness: if reduce { 0.3 } else { 0.6 },
                shape_detail_scale: if reduce { 0.05 } else { 0.4 },
                seed,
                ..defaults
            };
            eroded(&peaks, erosion, terrain)
        }
        MountainStyle::Old => {
            let erosion = ErosionSettings {
                duration: 40.0,
                downcutting: 0.1,
                erosion_scale: 1600.0,
                coarse_sediments_amount: 0.65,
                shape: 0.5,
                shape_sharpness: 0.5,
                shape_detail_scale: if reduce { 0.05 } else { 0.25 },
                seed,
                ..defaults
            };
            eroded(&peaks, erosion, terrain)
        }
        MountainStyle::Alpine | MountainStyle::Strata => {
            let erosion = ErosionSettings {
                duration: 60.0,
                downcutting: 0.5,
                erosion_scale: 1200.0,
                bed_load_amount: 0.33,
                shape: 0.8,
                shape_sharpness: 0.9,
                shape_detail_scale: 0.0,
                seed,
                ..defaults
            };
            let mut doubled = eroded(&peaks, erosion, terrain).multiply_scalar(2.0);
            if settings.style == MountainStyle::Strata {
                let terraced = fractal_terraces::complex_terraces(&doubled, 0.5, 0.4, 0.08, 0.65, 0, 12, seed, terrain).0;
                let terraced = maths::fit_range(&terraced, 0.0, 1.0);
                let tops = aspect::height_normalized(&doubled, 0.65, 1.0, 0.33);
                doubled = masking::mask(&doubled, &terraced, &tops);
            }
            doubled
        }
        MountainStyle::Basic => peaks,
    };
    match settings.bulk {
        MountainBulk::Low => maths::strengthen(&mut mountain, -0.25),
        MountainBulk::High => maths::strengthen(&mut mountain, 0.2),
        MountainBulk::Medium => {}
    }
    mountain
}

/// `CanyonStyle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanyonStyle {
    Classic,
    Eroded,
    Eroded2,
    Strata,
    Both,
}

impl CanyonStyle {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Classic" => Self::Classic,
            "Eroded2" => Self::Eroded2,
            "Strata" => Self::Strata,
            "Both" => Self::Both,
            _ => Self::Eroded,
        }
    }
}

/// Settings of [`canyon`].
#[derive(Clone, Copy, Debug)]
pub struct CanyonSettings {
    pub style: CanyonStyle,
    pub scale: f32,
    pub slot: f32,
    pub valley: f32,
    pub surrounding: f32,
    pub depth: f32,
    pub perturbation: f32,
    pub detail_perturbation: f32,
    pub voronoi: bool,
    pub seed: i32,
}

/// One tilted `Profiles.FractalTerrace` pass of the strata canyon.
fn canyon_strata(map: &Map, spacing: f32, seed: i32, direction: i32, terrain: TerrainSize) -> Map {
    let settings = FractalTerraceSettings {
        spacing,
        seed,
        tilt: true,
        tilt_amount: 0.05,
        direction,
        ..FractalTerraceSettings::default()
    };
    fractal_terraces::fractal_terrace(map, None, &settings, terrain).0
}

/// `Landscapes.Canyon`: a mirrored ramp folded into a valley, warped
/// twice and cut into the ground, then eroded and stratified per style.
/// Returns the canyon and its depth mask.
pub fn canyon(resolution: Resolution, settings: &CanyonSettings, terrain: TerrainSize) -> (Map, Map) {
    let size = resolution.x;
    let seed = settings.seed;
    let slot = -1.0 + settings.slot * (1.0 - -1.0);
    let valley = -1.0 + settings.valley * (1.0 - -1.0);
    let surrounding = -1.0 + settings.surrounding * (1.0 - -1.0);
    let center = size as f32 / 2.0;
    let ramp = gradients::linear(size, center, center, size as f32, 0.0, gradients::EdgeBehaviour::Mirror);
    let mut inverted = ramp.clone();
    maths::invert(&mut inverted);
    let mut fold = combiner::min(&ramp, &inverted, 1.0);
    maths::strengthen(&mut fold, slot);

    let structure = FractalWarpSettings {
        size: if settings.voronoi { 0.4 * settings.scale } else { 0.8 * settings.perturbation },
        strength: 0.35,
        noise_type: if settings.voronoi { WarpNoiseType::VoronoiR } else { WarpNoiseType::PerlinFbm },
        perturbation: settings.perturbation,
        iterations: 6,
        seed: seed + 1,
        ..FractalWarpSettings::default()
    };
    let mut shaped = warps::fractal_warp(&fold, &structure, None, terrain.0, terrain.1);
    maths::strengthen(&mut shaped, valley);
    let detail = FractalWarpSettings {
        size: 0.6 * settings.scale,
        strength: 0.7,
        noise_type: WarpNoiseType::VoronoiR,
        perturbation: settings.detail_perturbation,
        iterations: 2,
        seed: seed + 2,
        ..FractalWarpSettings::default()
    };
    let mut detailed = warps::fractal_warp(&shaped, &detail, None, terrain.0, terrain.1);
    maths::strengthen(&mut detailed, surrounding);
    let ground = Map::filled(resolution, 1.0);
    let mut canyon = combiner::subtract(&ground, &detailed, settings.depth);

    let erosion = ErosionSettings {
        duration: 50.0,
        downcutting: 0.4,
        bed_load_amount: 0.6,
        coarse_sediments_amount: 0.4,
        shape: 0.5,
        shape_sharpness: 0.5,
        shape_detail_scale: 0.5,
        seed,
        ..ErosionSettings::default()
    };
    if matches!(settings.style, CanyonStyle::Eroded | CanyonStyle::Both) {
        let worn = eroded(&canyon, erosion, terrain);
        canyon = combiner::min(&canyon, &worn, 1.0);
    }
    if settings.style == CanyonStyle::Eroded2 {
        let boosted = ErosionSettings {
            duration: 20.0,
            erosion_scale: 700.0,
            seed: seed + 5,
            deposition_boost: 1.0,
            extra_deposition_boost: 1.0,
            ..erosion
        };
        let worn = eroded(&canyon, boosted, terrain);
        canyon = combiner::min(&canyon, &worn, 1.0);
    }
    if matches!(settings.style, CanyonStyle::Strata | CanyonStyle::Both) {
        let unstratified = canyon.clone();
        let first = canyon_strata(&canyon, 0.05, seed + 1, 0, terrain);
        let first_lowest = combiner::min(&canyon, &first, 1.0);
        let second = canyon_strata(&first_lowest, 0.1, seed + 2, 24, terrain);
        let second_lowest = combiner::min(&first, &second, 1.0);
        let third = canyon_strata(&second_lowest, 0.01, seed + 3, 36, terrain);
        canyon = combiner::min(&second, &third, 1.0);
        let final_erosion = ErosionSettings {
            duration: 10.0,
            ..erosion
        };
        let worn = eroded(&canyon, final_erosion, terrain);
        canyon = combiner::min(&canyon, &worn, 1.0);
        let lowlands = aspect::height_normalized(&canyon, 0.0, 0.65, 0.1);
        canyon = masking::mask(&unstratified, &canyon, &lowlands);
    }
    let mut depth = maths::autolevel(canyon.clone());
    maths::invert(&mut depth);
    let canyon = maths::drop_to_zero(&canyon);
    (canyon, depth)
}

/// `Landscapes.Uplift`: a stretched ridge field lifted into complex
/// terraces.
pub fn uplift(resolution: Resolution, scale: f32, height: f32, octaves: i32, direction: i32, seed: i32, terrain: TerrainSize) -> Map {
    let ridges = RidgeSettings {
        scale: 0.11 - scale * 0.1,
        height: height + 1.2,
        definition: 0.3,
        seed,
        scale_x: 0.0001,
        scale_y: 1.0,
    };
    let field = ridge(resolution.x, &ridges, terrain);
    let lifted = maths::fit_range(&field, 0.1, 1.0);
    let terraced = fractal_terraces::complex_terraces(&lifted, 0.5, 0.75, height, 0.3, direction + 178, octaves, seed + 10, terrain).0;
    maths::lift_above_zero(terraced)
}

/// `Landscapes.Plates`: Perlin and billowy multifractal relief cut into
/// plates by a single ray-cast distance pass, then warped.
#[allow(clippy::too_many_arguments)]
pub fn plates(size: usize, scale: f32, range: f32, falloff: f32, warp: f32, angle: f32, seed: i32, terrain: TerrainSize) -> Map {
    let perlin = NoisesPerlin {
        scale,
        octaves: 10,
        gain: 0.35,
        seed,
        ..NoisesPerlin::default()
    };
    let base = noise::noises_perlin(size, &perlin);
    let billows = MultiFractalSettings {
        noise_type: MultifractalType::Billowy,
        octaves: 10,
        size: scale,
        feature_size: scale * 0.6,
        fractal_gain: 0.3,
        edge_smoothing: 0.1,
        seed,
        variation: 2.0,
        smoothness: 0.33,
        ..MultiFractalSettings::default()
    };
    let detail = multi_fractal::multi_fractal(Resolution::square(size), &billows);
    let relief = base.add(&detail.multiply_scalar(0.4));
    let distance = DistanceRtSettings {
        mode: DistanceRtMode::Asterisk,
        directions: 1,
        skew: lerp(range, 0.7, 1.0),
        angle,
        angular_jitter: 0.0,
        falloff,
        falloff_jitter: 0.0,
        invert_input: false,
        invert_output: false,
        seed,
    };
    let cut = morph::distance_rt(&relief, &distance, terrain);
    let settings = FractalWarpSettings {
        size: 0.9 - lerp(warp, 0.02, 0.2),
        strength: lerp(warp, 0.02, 0.1),
        iterations: 3,
        seed,
        ..FractalWarpSettings::default()
    };
    warps::fractal_warp(&cut, &settings, None, terrain.0, terrain.1)
}

/// `MountainRangeStyle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountainRangeStyle {
    Basic,
    Eroded,
    Stratified,
    Alpine,
}

impl MountainRangeStyle {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Basic" => Self::Basic,
            "Stratified" => Self::Stratified,
            "Alpine" => Self::Alpine,
            _ => Self::Eroded,
        }
    }
}

/// Settings of [`mountain_range`].
#[derive(Clone, Copy, Debug)]
pub struct MountainRangeSettings {
    pub scale: f32,
    pub height: f32,
    pub style: MountainRangeStyle,
    pub bulk: MountainBulk,
    pub seed: i32,
}

/// The two fractal terraces of a stratified mountain range (`Profiles.
/// FractalTerrace` with tilt), cut into each other.
fn range_strata(eroded_range: &Map, seed: i32, terrain: TerrainSize) -> Map {
    let fine = FractalTerraceSettings {
        spacing: 0.01,
        seed: seed + 1,
        tilt: true,
        tilt_amount: 0.025,
        ..FractalTerraceSettings::default()
    };
    let fine_terraces = fractal_terraces::fractal_terrace(eroded_range, None, &fine, terrain).0;
    let cut = combiner::min(eroded_range, &fine_terraces, 1.0);
    let coarse = FractalTerraceSettings {
        spacing: 0.02,
        seed: seed + 2,
        tilt: true,
        tilt_amount: 0.05,
        direction: 45,
        ..FractalTerraceSettings::default()
    };
    let coarse_terraces = fractal_terraces::fractal_terrace(&cut, None, &coarse, terrain).0;
    combiner::min(&fine_terraces, &coarse_terraces, 1.0)
}

/// `Landscapes.MountainRange`: three ridge layers (the middle one thermally
/// shaped) summed, warped and eroded in the chosen style.
pub fn mountain_range(resolution: Resolution, settings: &MountainRangeSettings, terrain: TerrainSize) -> Map {
    let seed = settings.seed;
    let size = resolution.x;
    let mut random = XorShift64Star::new(seed);
    let first = RidgeSettings {
        scale: random.next_float_range(0.25, settings.scale),
        height: 0.6,
        definition: 0.4,
        seed,
        scale_x: 1.0,
        scale_y: 1.0,
    };
    let first_ridges = ridge(size, &first, terrain);
    let second_scale = random.next_float_range(0.45, 0.2 + settings.scale);
    let second = RidgeSettings {
        scale: second_scale,
        definition: random.next_float_range(0.3, 1.0),
        seed: seed + 1,
        ..first
    };
    let second_ridges = profiles::thermal_shaper(&ridge(size, &second, terrain), 0.5, -0.05);
    let third_scale = random.next_float_range(0.15, 0.3 + settings.scale);
    let third = RidgeSettings {
        scale: third_scale,
        definition: random.next_float_range(0.1, 0.4),
        seed: seed + 2,
        ..first
    };
    let third_ridges = ridge(size, &third, terrain);
    let mut range = first_ridges.add(&second_ridges).add(&third_ridges);
    if settings.style != MountainRangeStyle::Alpine {
        let warp = FractalWarpSettings {
            seed: seed + 36,
            ..FractalWarpSettings::default()
        };
        let warped = warps::fractal_warp(&range, &warp, None, terrain.0, terrain.1);
        range = combiner::min(&range, &warped, 1.0);
    }

    let defaults = ErosionSettings::default();
    let mut shaped = match settings.style {
        MountainRangeStyle::Basic => range,
        MountainRangeStyle::Eroded => {
            let erosion = ErosionSettings {
                duration: 20.0,
                downcutting: 0.35,
                shape: 0.6,
                shape_sharpness: 0.6,
                shape_detail_scale: 0.4,
                seed,
                ..defaults
            };
            eroded(&range, erosion, terrain)
        }
        MountainRangeStyle::Stratified => {
            let erosion = ErosionSettings {
                duration: 20.0,
                downcutting: 0.35,
                bed_load_amount: 0.33,
                coarse_sediments_amount: 0.33,
                shape: 0.8,
                shape_sharpness: 0.6,
                shape_detail_scale: 0.4,
                seed,
                ..defaults
            };
            let eroded_range = eroded(&range, erosion, terrain);
            let strata = range_strata(&eroded_range, seed, terrain);
            let second_erosion = ErosionSettings {
                duration: 20.0,
                downcutting: 0.5,
                erosion_scale: 900.0,
                bed_load_amount: 0.33,
                shape: 0.6,
                shape_sharpness: 0.6,
                shape_detail_scale: 0.3,
                seed: seed + 5,
                ..defaults
            };
            let second_pass = eroded(&range, second_erosion, terrain);
            let layered = combiner::min(&strata, &second_pass, 1.0);
            let lowlands = aspect::height_normalized(&layered, 0.0, 0.3, 0.3);
            masking::mask(&eroded_range, &layered, &lowlands)
        }
        MountainRangeStyle::Alpine => {
            let erosion = ErosionSettings {
                duration: 30.0,
                downcutting: 0.85,
                erosion_scale: 900.0,
                bed_load_amount: 0.33,
                shape: 0.8,
                shape_sharpness: 0.9,
                shape_detail_scale: 0.0,
                seed: seed + 2,
                ..defaults
            };
            eroded(&range, erosion, terrain)
        }
    };
    let bulk = match settings.bulk {
        MountainBulk::Low => -0.25,
        MountainBulk::Medium => -0.14999999,
        MountainBulk::High => -0.049999982,
    };
    maths::strengthen(&mut shaped, bulk);
    shaped.multiply_scalar(settings.height)
}

/// Settings of [`dune_sea`]; the three levels are indices into the
/// generator's tables.
#[derive(Clone, Copy, Debug)]
pub struct DuneSeaSettings {
    /// `ABCD` dune type.
    pub dune_type: usize,
    pub scale: f32,
    pub height: f32,
    /// Degrees.
    pub direction: i32,
    pub softness: i32,
    pub sharpness: f32,
    pub seed: i32,
    /// `DuneChaosLevel`.
    pub chaos: usize,
    /// `DuneUndulationLevel`.
    pub undulation: usize,
}

/// `Landscapes.DuneSea`: Gabor dunes, warped, softened and thermally
/// sharpened.
pub fn dune_sea(resolution: Resolution, settings: &DuneSeaSettings, aniso: Option<&Map>, terrain: TerrainSize) -> Map {
    const ENTROPY: [f32; 4] = [0.12, 0.36, 0.64, 0.96];
    const ANISOTROPY: [f32; 4] = [0.96, 0.64, 0.36, 0.16];
    const WARP_SIZE: [f32; 4] = [0.01, 0.15, 0.3, 0.05];
    // Radians are passed where `RawNoise.Gabor` expects degrees.
    let direction = settings.direction as f32 / 360.0 * 2.0 * std::f32::consts::PI;
    let gabor = GaborSettings {
        size: lerp(settings.scale, 0.1, 0.5) * 0.25,
        entropy: ENTROPY[settings.chaos],
        anisotropy: ANISOTROPY[settings.undulation],
        direction,
        seed: settings.seed,
    };
    let mut dunes = noise::gabor(resolution.x, &gabor, aniso.map(|map| map.data()));
    let height = settings.height;
    dunes.apply(|value| value * height * 0.4);
    let warp = FractalWarpSettings {
        size: WARP_SIZE[settings.dune_type],
        strength: 0.5,
        roughness: 0.0,
        edges: EdgeBehaviour::Mirror,
        seed: settings.seed + 2,
        ..FractalWarpSettings::default()
    };
    let warped = warps::fractal_warp(&dunes, &warp, None, terrain.0, terrain.1);
    let softened = fast_blur::blur_relative(&warped, settings.softness as f32 * 0.25);
    profiles::thermal_shaper(&softened, 1.0, -0.2 - settings.sharpness * 0.8)
}

/// `Landscapes.Sand`: fine Gabor ripples, thermally shaped, scaled and
/// softened. `aniso` (blurred) steers the ripples.
#[allow(clippy::too_many_arguments)]
pub fn sand(resolution: Resolution, scale: f32, chaos: f32, height: f32, softness: f32, direction: i32, seed: i32, aniso: Option<&Map>) -> Map {
    let steering = aniso.map(|map| fast_blur::blur_relative(map, 0.025));
    let gabor = GaborSettings {
        size: scale * 0.009,
        entropy: chaos * 0.45,
        anisotropy: 0.9582376,
        direction: direction as f32,
        seed,
    };
    let ripples = noise::gabor(resolution.x, &gabor, steering.as_ref().map(|map| map.data()));
    let shaped = profiles::thermal_shaper(&ripples, 1.0, -1.0 / 32.0);
    let scaled = shaped.multiply_scalar(height * 0.5);
    fast_blur::blur_relative(&scaled, 0.0025 * softness)
}
