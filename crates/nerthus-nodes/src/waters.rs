//! `Waters`: the Sea and (advanced) Lake nodes on top of the CW water
//! engine (`nerthus_erosion::cw`).

use nerthus_engine::{Map, Resolution};
use nerthus_erosion::cw;
use nerthus_noise::fast_noise::{FastNoise, NoiseType};
use rayon::prelude::*;

use crate::fast_blur;
use crate::maths::{self, dotnet_max, dotnet_min};
use crate::morphology::{self, DistanceOptions};

/// `FNSIMD`'s default seed.
const FNSIMD_DEFAULT_SEED: i32 = 1337;

/// `SeaArrangement`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeaArrangement {
    Global,
    /// Water flooded in from the map edge (`Surrounding`).
    Surrounding,
}

/// Settings of [`sea`] (`Waters.Sea`).
#[derive(Clone, Copy, Debug)]
pub struct SeaSettings {
    pub arrangement: SeaArrangement,
    pub level: f32,
    pub coastal_erosion: bool,
    pub shore_size: f32,
    pub shore_height: f32,
    pub variation: f32,
    pub uniform_variations: bool,
    pub extra_cliff_details: bool,
    pub render_surface: bool,
}

/// What [`sea`] returns.
pub struct SeaMaps {
    pub height: Map,
    pub water: Map,
    pub depth: Map,
    pub shore: Map,
    pub surface: Map,
}

/// `fade`: the smootherstep polynomial.
fn fade(x: f32) -> f32 {
    x * x * x * (10.0 + x * (6.0 * x - 15.0))
}

/// `fadeInf`: [`fade`] clamped to [0, 1]; NaN counts as 1.
fn fade_clamped(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if !(x < 1.0) {
        return 1.0;
    }
    fade(x)
}

/// `smoothMax`: a maximum with a cubic blend over `k`.
fn smooth_max(a: f32, b: f32, k: f32) -> f32 {
    let overlap = dotnet_max(k - (a - b).abs(), 0.0) / k;
    dotnet_max(a, b) + overlap * overlap * overlap * k * (1.0 / 6.0)
}

/// The arguments of `MakeShoreface`.
struct Shoreface<'a> {
    size: usize,
    coastal_erosion: bool,
    shore_size: f32,
    shore_height: f32,
    level: f32,
    variation: f32,
    uniform_variations: bool,
    heights: &'a [f32],
    smooth: &'a [f32],
    sea: &'a [f32],
    render_surface: bool,
    extra_cliff_details: bool,
}

/// `MakeShoreface`: beaches and cliffs along the coast. Returns the shaped
/// terrain, the shore mask and the water depth.
fn make_shoreface(input: &Shoreface) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let size = input.size;
    let level = input.level;
    let shore_size = input.shore_size;
    let coastal_erosion = input.coastal_erosion && input.shore_height * shore_size != 0.0;
    let variation = input.variation.sqrt();
    let shore_height = input.shore_height * 0.5;

    let land_options = DistanceOptions {
        falloff: size as f32 * shore_size * 0.5,
        threshold: 0.5,
        invert_input: false,
        invert_output: false,
        clamp: true,
    };
    let sea_options = DistanceOptions {
        falloff: size as f32 * shore_size,
        invert_input: true,
        clamp: false,
        ..land_options
    };
    let land_distance = morphology::distance_transform(input.sea, size, land_options);
    let sea_distance = morphology::distance_transform(input.sea, size, sea_options);
    let radius = size as f32 * 0.025 * dotnet_min(shore_size, shore_height);
    let land_distance = fast_blur::managed_box_blur(&land_distance, size, radius, 3);
    let sea_distance = fast_blur::managed_box_blur(&sea_distance, size, radius, 3);
    let analysis = fast_blur::managed_box_blur(input.heights, size, radius * 8.0, 3);

    let cliff_scale = size as f32 * 0.23 * shore_height;
    let detailed = coastal_erosion && input.extra_cliff_details;
    let cliff_noise = if detailed {
        let mut noise = FastNoise::new(FNSIMD_DEFAULT_SEED);
        noise.set_frequency(1.0 / cliff_scale);
        noise.set_noise_type(NoiseType::PerlinFractal);
        // log2(e), truncated as the engine has it.
        #[allow(clippy::approx_constant)]
        let octaves = (maths::ln(cliff_scale) as f64 * 1.44269504088896 + 1.0) as i32;
        noise.set_fractal_octaves(octaves);
        noise.set_fractal_gain(0.4);
        Some(noise)
    } else {
        None
    };

    let mut result = vec![0.0f32; size * size];
    let mut shore = vec![0.0f32; size * size];
    let mut depth = vec![0.0f32; size * size];
    let rows = result.par_chunks_mut(size).zip(shore.par_chunks_mut(size)).zip(depth.par_chunks_mut(size));
    rows.enumerate().for_each(|(y, ((result_row, shore_row), depth_row))| {
        let noise_row = cliff_noise.as_ref().map(|noise| noise.get_noise_set(0, y as i32, 0, size, 1, 1, 1.0));
        for x in 0..size {
            let index = y * size + x;
            // `landDistance` and `seaDistance` are clamped in place, but
            // only read back at the same pixel.
            let land = dotnet_min(1.0, land_distance[index] * 1.05);
            let sea = dotnet_min(1.0, sea_distance[index] * 1.05);
            let mut above = input.heights[index] - level;
            let mut surface = input.smooth[index] - level;
            let mut exposed = 0.0f32;
            let steep;
            let flooded;
            if coastal_erosion {
                surface /= shore_height;
                above /= shore_height;
                let mut slope;
                let mut reach;
                let mut ridge;
                if surface < 0.0 {
                    slope = dotnet_min(0.0 - surface, 1.0);
                    reach = sea;
                    slope *= 2.0 - slope;
                    slope *= slope;
                    ridge = slope;
                } else {
                    slope = dotnet_min(surface / 0.5, 1.0);
                    ridge = dotnet_min(above / 0.5, 1.0);
                    let cliff = dotnet_max(dotnet_min((analysis[index] - level) / shore_height / 0.5, 1.0), 0.0);
                    let cliff = dotnet_min(cliff, slope);
                    reach = land;
                    let mut wear = if input.uniform_variations {
                        fade_clamped(reach / (cliff * variation) - 0.25)
                    } else {
                        fade_clamped(reach / (slope * variation) - 0.2)
                    };
                    if input.extra_cliff_details {
                        let noise_row = noise_row.as_ref().expect("cliff noise exists when cliffs are detailed");
                        wear += 0.0 - noise_row[x].abs() * 0.2;
                    }
                    let sharpness = 3.0f32;
                    let mut softness = dotnet_min(reach, 1.0);
                    softness = fade_clamped(softness * 1.5 - 0.3) * 0.5 + 1e-6;
                    let mut lower = dotnet_min(1.0, smooth_max(0.0, slope * sharpness - wear * (sharpness - 1.0), softness));
                    let mut upper = dotnet_min(1.0, smooth_max(0.0, ridge * sharpness - wear * (sharpness - 1.0), softness));
                    let mut lower_fourth = lower * lower;
                    lower_fourth *= lower_fourth;
                    let mut upper_fourth = upper * upper;
                    upper_fourth *= upper_fourth;
                    lower = (lower - lower_fourth * 0.25) * 1.3333334;
                    upper = (upper - upper_fourth * 0.25) * 1.3333334;
                    lower = lower * 0.75 + 0.25 * lower * lower;
                    upper = upper * 0.75 + 0.25 * upper * upper;
                    slope = lower;
                    ridge = upper;
                    exposed = 1.0 - dotnet_min(lower / softness, 1.0);
                }
                steep = ridge;
                flooded = (surface > 0.0 && reach >= 1.0) || (input.sea[index] == 0.0 && surface < 0.0);
                if flooded {
                    reach = 1.0;
                }
                let shallow = surface * 0.3;
                let blend;
                if surface < 0.0 {
                    reach = dotnet_min(reach, 1.0);
                    reach *= reach;
                    slope = slope * (1.0 - reach) + reach;
                    surface = (0.3 * ((0.0 - surface - 0.5) * (0.0 - surface - 0.5)) - 0.075) * (1.0 - slope) + surface * slope;
                    exposed = fade_clamped((1.0 - slope) * 1.5);
                    blend = exposed;
                } else {
                    reach = dotnet_min(reach, 1.0);
                    reach = fade_clamped(reach * 1.5 - 0.5);
                    blend = exposed * (1.0 - reach) + reach;
                    exposed *= 1.0 - dotnet_min(reach, 1.0);
                    slope = slope * (1.0 - reach) + reach;
                    above = above * (1.0 - reach) + reach;
                    surface = shallow * (1.0 - slope) + surface * slope;
                    above = above * 0.3 * (1.0 - steep) + above * steep;
                }
                shore_row[x] = if flooded { 0.0 } else { exposed };
                if flooded {
                    slope = 1.0;
                }
                if surface > 0.0 {
                    let rise = above - surface;
                    surface += rise * (1.0 - blend);
                    if surface < shallow {
                        // The shore value is raised here too, after it was
                        // stored; only the height changes.
                        surface = shallow;
                    }
                    surface *= shore_height;
                    surface += level;
                    surface = (1.0 - slope) * surface + slope * input.heights[index];
                } else {
                    surface = (1.0 - slope) * (surface * shore_height + level) + slope * input.heights[index];
                }
            } else {
                surface = above + level;
                flooded = false;
            }
            depth_row[x] = if surface < level && !flooded { level - surface } else { 0.0 };
            if input.render_surface && !flooded {
                surface = dotnet_max(surface, level);
            }
            result_row[x] = surface;
        }
    });
    (result, shore, depth)
}

/// `Waters.Sea`: water below `level` (everywhere, or flooded in from the
/// map edge through the CW engine's edge fill), with beaches and cliffs shaped
/// along the coast.
pub fn sea(heightmap: &Map, edge_mask: Option<&Map>, settings: &SeaSettings) -> SeaMaps {
    let resolution = heightmap.resolution();
    let size = resolution.x;
    let heights = heightmap.data().to_vec();
    let level = settings.level;
    let eroding = settings.coastal_erosion && settings.shore_height * settings.shore_size != 0.0;
    let radius = size as f32 * 0.025 * dotnet_min(settings.shore_size, settings.shore_height);
    let smooth = if eroding { fast_blur::managed_box_blur(&heights, size, radius, 3) } else { heights.clone() };

    let (depth, surface) = match settings.arrangement {
        SeaArrangement::Global => {
            let depth: Vec<f32> = smooth.iter().map(|&height| dotnet_max(0.0, level - height)).collect();
            (depth, vec![level; size * size])
        }
        SeaArrangement::Surrounding => {
            let mask = edge_mask.map(|map| map.data());
            let filled = cw::edge_fill(&smooth, size, level, mask);
            (filled.depth, filled.surface)
        }
    };
    let water: Vec<f32> = depth.iter().map(|&value| if value > 0.0 { 1.0 } else { 0.0 }).collect();
    let shoreface = Shoreface {
        size,
        coastal_erosion: eroding,
        shore_size: settings.shore_size,
        shore_height: settings.shore_height,
        level,
        variation: settings.variation,
        uniform_variations: settings.uniform_variations,
        heights: &heights,
        smooth: &smooth,
        sea: &water,
        render_surface: settings.render_surface,
        extra_cliff_details: settings.extra_cliff_details,
    };
    let (height, shore, depth) = make_shoreface(&shoreface);
    SeaMaps {
        height: Map::from_data(resolution, height),
        water: Map::from_data(resolution, water),
        depth: Map::from_data(resolution, depth),
        shore: Map::from_data(resolution, shore),
        surface: Map::from_data(resolution, surface),
    }
}

/// Settings of [`lakes`] (`Waters.Lakes`).
#[derive(Clone, Copy, Debug)]
pub struct LakesSettings {
    pub precipitation: f32,
    pub small_lakes: f32,
    pub flood_control: bool,
    pub water_floor: f32,
    pub shore_size: f32,
    pub altitude_bias: f32,
    pub size_bias: f32,
}

/// What [`lakes`] returns.
pub struct LakeMaps {
    pub height: Map,
    pub depth: Map,
    pub shore: Map,
    pub surface: Map,
    pub water: Map,
}

/// `Waters.Lakes`: the CW engine's rain-filled lakes (the Lake node's Advanced
/// type).
pub fn lakes(heightmap: &Map, precipitation_map: Option<&Map>, settings: &LakesSettings, terrain: (f32, f32)) -> LakeMaps {
    let resolution = heightmap.resolution();
    let water_floor = if settings.flood_control { settings.water_floor as f64 } else { 0.0 };
    let engine = cw::LakeSettings {
        resolution: resolution.x,
        precipitation: settings.precipitation as f64,
        water_floor,
        lake_size_bias: settings.small_lakes as f64,
        shore_size: settings.shore_size as f64,
        altitude_bias: settings.altitude_bias as f64,
        size_bias: settings.size_bias as f64,
        size: terrain.0 as f64,
        height_range: terrain.1 as f64,
    };
    let precipitation = precipitation_map.map(|map| map.data());
    let output = cw::erode_lakes(&engine, heightmap.data(), precipitation);
    let water: Vec<f32> = output.depth.iter().map(|&depth| if depth > 0.0 { 1.0 } else { 0.0 }).collect();
    let plane = |values: Vec<f32>| Map::from_data(Resolution::square(resolution.x), values);
    LakeMaps {
        height: plane(output.results),
        depth: plane(output.depth),
        shore: plane(output.shore),
        surface: plane(output.surface),
        water: plane(water),
    }
}
