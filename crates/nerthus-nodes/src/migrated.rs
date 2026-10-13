//! `Migrated`: node bodies from the older node set kept as library
//! functions.

use nerthus_engine::{Map, Resolution};

use crate::aspect;
use crate::combiner;
use crate::fast_blur;
use crate::fractal_terraces;
use crate::gradients;
use crate::maths;
use crate::random::XorShift64Star;
use crate::rock_core::{self, LargeRockSettings, RockClusters, RockRelief};
use crate::simulations;
use crate::slope_blur;
use crate::surfacer;
use crate::transformer::{self, AnchorPosition, BlendMode, ResizeMethod, TransformQuality};
use crate::warps::{self, FractalWarpSettings};

/// Terrain width and height in metres, which warps with a z coefficient use.
pub type TerrainSize = (f32, f32);

/// `Migrated.IslandProcess`: a coastline from the input (or a disc
/// `size` across), thresholded, scaled to 0..0.25 and warped twice.
pub fn island(map: Option<&Map>, size_pixels: usize, seed: i32, size: f32, chaos: f32, terrain: TerrainSize) -> Map {
    let half = size_pixels as f32 / 2.0;
    let mut shape = match map {
        Some(map) => map.clone(),
        None => gradients::radial(size_pixels, half, half, half * size, 1.0),
    };
    shape = aspect::threshold(&shape, 0.01);
    shape = maths::fit_range(&shape, 0.0, 0.25);
    let coarse = FractalWarpSettings {
        size: 0.3 + chaos * (1.2 - 0.3),
        iterations: 3,
        seed: seed + 1,
        ..FractalWarpSettings::default()
    };
    shape = warps::fractal_warp(&shape, &coarse, None, terrain.0, terrain.1);
    let fine = FractalWarpSettings {
        size: 0.14,
        iterations: 7,
        seed: seed + 2,
        ..FractalWarpSettings::default()
    };
    shape = warps::fractal_warp(&shape, &fine, None, terrain.0, terrain.1);
    shape = aspect::threshold(&shape, 0.01);
    maths::fit_range(&shape, 0.0, 0.25)
}

/// Settings of [`pockmarks`].
pub struct PockmarkSettings {
    pub size: [f32; 2],
    pub scale: f32,
    pub height: f32,
    pub density: f32,
    pub multiplier: f32,
    pub invert: bool,
    pub seed: i32,
}

/// `Migrated.PockmarksProces`: stamps warped discs into the map as pits
/// (or bumps when inverted).
pub fn pockmarks(map: &Map, size_pixels: usize, settings: &PockmarkSettings, terrain: TerrainSize) -> Map {
    let count = (settings.density * (6000.0 * settings.multiplier)) as i32;
    let center = size_pixels as f32 * 0.5;
    let disc = gradients::radial(size_pixels, center, center, size_pixels as f32 * 0.2, 1.0);
    let mut random = XorShift64Star::new(settings.seed);
    let mut stamps = Vec::with_capacity(4);
    for index in 0..4 {
        let warp = FractalWarpSettings {
            size: random.next_float_range(settings.scale * 0.25, settings.scale),
            seed: settings.seed + index,
            ..FractalWarpSettings::default()
        };
        let warped = warps::fractal_warp(&disc, &warp, None, terrain.0, terrain.1);
        stamps.push(warped.multiply_scalar(settings.height * 0.1));
    }

    let mut marks = Map::new(nerthus_engine::Resolution::square(size_pixels), 1);
    let normalizer = 1.0 / (maths::exp(10.000001) - 1.0);
    let mode = if settings.invert { BlendMode::Add } else { BlendMode::Subtract };
    for _ in 0..count {
        let stamp = &stamps[random.next_range(0, stamps.len() as i32) as usize];
        let mut depth = random.next_float_range(0.5, 1.0);
        let mut scale = random.next_float_range(settings.size[0], settings.size[1] * 0.88);
        scale = (maths::exp(scale * 1.0 * 10.0 + 1e-6) - 1.0) * normalizer;
        depth *= scale;
        let position = (random.next_float(), random.next_float());
        let rotation = 360.0 * random.next_float_range(0.0, 360.0);
        transformer::multi_transform(stamp, &mut marks, position, (scale, scale), rotation, depth, mode, TransformQuality::Medium, None, true, AnchorPosition::Center);
    }
    map.add(&marks)
}

/// `Migrated.BulbousProcess`: a slope blur along the map's own blurred
/// slopes that rounds features into bulbs.
pub fn bulbous(map: &Map, scale: f32, jitter: f32) -> Map {
    let guide = fast_blur::blur_relative(map, 2.0);
    let settings = slope_blur::BlurSettings {
        intensity: 0.02 + scale * (0.4 - 0.02),
        quality: 0,
        direction: jitter * 45.0,
        bias: 1.0 - jitter * 0.2,
        normalized: false,
        window: slope_blur::Window::Bell,
        ssaa: 0,
        spectral_quality: 4,
        iterations: 1,
    };
    let blurred = slope_blur::blur(map.data(), guide.data(), map.width(), &settings);
    Map::from_data(map.resolution(), blurred)
}

/// `Migrated.CraggyProcess`: large rocks embedded and slope-warped into
/// the map, then iteratively terraced and stretched back to its range.
pub fn craggy(map: &Map, seed: i32, size: f32, depth: f32, shape: f32, terrain: (f32, f32)) -> Map {
    let (minimum, maximum) = maths::min_max(map.data());
    let rocks = LargeRockSettings {
        size: 2.0 - size,
        fracture: 0.5,
        roughness: 0.7,
        breakage: 0.55,
        surface: 0.34,
        clusters: RockClusters::Off,
        relief: RockRelief::Off,
        jitter: 0.5,
        seed,
    };
    let rocks = rock_core::large(map.width(), &rocks, terrain);
    let embedded = combiner::embed(map, &rocks, 1.0, 0.0);
    let warp = slope_blur::WarpSettings {
        intensity: 0.12,
        iterations: 2,
        direction: -90.0 + shape * (30.0 - -90.0),
        normalized: false,
        quality: 1,
        ssaa: 0,
        spectral_quality: 12,
    };
    let warped = slope_blur::warp(embedded.data(), rocks.data(), None, map.width(), &warp);
    let warped = Map::from_data(map.resolution(), warped);
    let crags = maths::lift_above_zero(combiner::embed(&warped, &rocks, 0.5 + depth, 0.0));
    let shape_bias = -0.5 + shape * (0.5 - -0.5);
    let mut terraced = fractal_terraces::iterracions(&crags, 9, 0.5, shape_bias, 0.025, 0.25, 0, 12, seed + 55, None, terrain).0;
    let (terraced_minimum, terraced_maximum) = maths::min_max(terraced.data());
    maths::remap(&mut terraced, terraced_maximum, terraced_minimum, maximum, minimum);
    terraced
}

/// `RelativeQuality`: the working resolution of a process.
pub fn relative_quality_size(name: &str, size: usize) -> usize {
    let limit = match name {
        "Quarter" => 512,
        "Third" => 1024,
        "Half" => 2048,
        _ => size,
    };
    limit.min(size)
}

/// `GroundStyle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroundStyle {
    Harsh,
    Rocky,
    Rough,
}

impl GroundStyle {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Rocky" => Self::Rocky,
            "Rough" => Self::Rough,
            _ => Self::Harsh,
        }
    }
}

/// `Migrated.GroundTextureProcess`: rocky curvature screened faintly over
/// the map. Harsh widens the falloff with `coverage`; the other styles
/// keep it minimal.
pub fn ground_texture(map: &Map, style: GroundStyle, strength: f32, coverage: f32, density: f32, terrain_height: f32) -> Map {
    let falloff = if style == GroundStyle::Harsh {
        (0.05 + coverage * 0.95) * 0.1
    } else {
        0.0001
    };
    let rocky = surfacer::rocky(map, falloff, density, terrain_height);
    combiner::screen(map, &rocky, strength * 0.003)
}

/// `Migrated.RoughenProcess`: flow lines (computed at a reduced
/// resolution) carved into the map.
pub fn roughen(map: &Map, width: f32, depth: f32, oriented: bool, quality: &str) -> Map {
    let mut strength = depth * 0.5;
    strength *= 0.35;
    let working_size = relative_quality_size(quality, map.width());
    let reduced = quality != "Full";
    let working = if reduced {
        transformer::resize_with(map, Resolution::square(working_size), ResizeMethod::HighQuality)
    } else {
        map.clone()
    };
    let mut flow = maths::autolevel(simulations::create_flow_map(&working, 3, 0.00075, oriented));
    if reduced {
        flow = transformer::resize_with(&flow, map.resolution(), ResizeMethod::HighQuality);
    }
    maths::strengthen(&mut flow, width);
    combiner::subtract(map, &flow, strength * 0.05)
}
