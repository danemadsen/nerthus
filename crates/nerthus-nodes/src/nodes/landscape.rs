//! Landscape generators: Slump, Ridge, Rugged, Mountain, MountainSide,
//! Canyon, Uplift and Volcano.

use nerthus_engine::{BuildContext, Map, NodeBuilder, NodeError, RenderIntent, Resolution};
use nerthus_noise::fast_noise::{CellularDistanceFunction, CellularReturnType};

use crate::aspect;
use crate::combiner::{self, BlendMode};
use crate::craterfield;
use crate::flow::{self, FlowSettings};
use crate::fractal_terraces;
use crate::gradients;
use crate::histogram;
use crate::landscapes::{self, Abc, CanyonSettings, CanyonStyle, DuneSeaSettings, MountainBulk, MountainRangeSettings, MountainRangeStyle, MountainSettings, MountainStyle, RidgeSettings};
use crate::masking;
use crate::maths;
use crate::migrated;
use crate::noise::{self, CrackSettings, CrackStyle, NoisesPerlin, Viewport, VoronoiSettings};
use crate::parallel_erosion::{self, ErosionSettings};
use crate::profiles::{self, FoldWaveform};
use crate::random::XorShift64Star;
use crate::rock_core::{self, LargeRockSettings, RockClusters, RockRelief, SandstoneSettings};
use crate::slope_blur;
use crate::transformer::{self, ResizeMethod};
use crate::warps::{self, FractalWarpSettings, WarpNoiseType};

/// Multiplies by `In` when it is connected.
fn mask_by_input(context: &BuildContext, map: Map) -> Map {
    match context.input("In") {
        Some(input) => map.multiply(&input),
        None => map,
    }
}

pub struct Slump;

impl NodeBuilder for Slump {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let size = context.resolution.x;
        let scale = context.f32("Scale");
        let seed = context.i32("Seed");
        let terrain = (context.terrain_width(), context.terrain_height());
        let map = match context.enum_name("Style") {
            "B" => landscapes::rugged(size, Abc::A, scale, 1, seed, terrain),
            "C" => landscapes::rugged(size, Abc::B, scale, 5, seed, terrain),
            "D" => landscapes::rugged(size, Abc::C, scale, 8, seed, terrain),
            _ => Some(landscapes::slump(size, scale, Abc::A, Abc::A, seed, terrain)),
        };
        let map = map.ok_or_else(|| NodeError::Failed("Slump produced no map.".to_string()))?;
        let map = mask_by_input(context, map);
        context.commit(map)
    }
}

pub struct Ridge;

impl NodeBuilder for Ridge {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let settings = RidgeSettings {
            scale: context.f32("Scale"),
            height: context.f32("Height"),
            definition: context.f32("Definition"),
            seed: context.i32("Seed"),
            scale_x: context.f32("ScaleX"),
            scale_y: context.f32("ScaleY"),
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let map = landscapes::ridge(context.resolution.x, &settings, terrain);
        let map = mask_by_input(context, map);
        context.commit(map)
    }
}

pub struct Rugged;

impl NodeBuilder for Rugged {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let size = context.resolution.x;
        let scale = context.f32("Scale");
        let seed = context.i32("Seed");
        let terrain = (context.terrain_width(), context.terrain_height());
        let surface = match context.enum_name("Style") {
            "A" => landscapes::surface_m2(size, scale, seed, terrain),
            "B" => landscapes::surface_m3(size, scale, seed, terrain),
            "C" => landscapes::surface_m4(size, scale, seed, terrain),
            "D" => landscapes::surface_m5(size, scale, seed, terrain),
            "E" => landscapes::surface_m6(size, scale, seed, terrain),
            _ => landscapes::surface_m2(size, scale, seed, terrain),
        };
        let surface = maths::lift_above_zero(surface);
        match context.input("In") {
            Some(input) => {
                let embedded = combiner::embed(&input, &surface, 1.0, 0.0);
                context.commit(embedded)
            }
            None => context.commit(surface),
        }
    }
}

pub struct Mountain;

impl NodeBuilder for Mountain {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let settings = MountainSettings {
            scale: context.f32("Scale"),
            height: context.f32("Height"),
            reduce_detail: context.bool("ReduceDetails"),
            style: MountainStyle::from_name(context.enum_name("Style")),
            bulk: MountainBulk::from_name(context.enum_name("Bulk")),
            seed: context.i32("Seed"),
            x: context.f32("X"),
            y: context.f32("Y"),
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let map = landscapes::mountain(context.resolution, &settings, terrain);
        let map = mask_by_input(context, map);
        context.commit(map)
    }
}

pub struct Canyon;

impl NodeBuilder for Canyon {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let settings = CanyonSettings {
            style: CanyonStyle::from_name(context.enum_name("Style")),
            scale: context.f32("Scale"),
            slot: context.f32("Slot"),
            valley: context.f32("Valley"),
            surrounding: context.f32("Surrounding"),
            depth: context.f32("Depth"),
            perturbation: context.f32("StructualWarp"),
            detail_perturbation: context.f32("DetailWarp"),
            voronoi: !context.bool("AlternateStyle"),
            seed: context.i32("Seed"),
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let (canyon, depth) = landscapes::canyon(context.resolution, &settings, terrain);
        let canyon = mask_by_input(context, canyon);
        context.commit(canyon)?;
        context.commit_port("Depth", depth, RenderIntent::Default);
        Ok(())
    }
}

pub struct Uplift;

impl NodeBuilder for Uplift {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let scale = context.f32("Scale");
        let height = context.f32("Height");
        let octaves = context.i32("Octaves");
        let direction = context.i32("Direction");
        let seed = context.i32("Seed");
        let passes = context.i32("Passes");
        let jitter = context.f32("Jitter");
        let terrain = (context.terrain_width(), context.terrain_height());
        let resolution = context.resolution;
        let mut map = landscapes::uplift(resolution, scale, height, octaves, direction, seed, terrain);
        if passes > 1 {
            let mut random = XorShift64Star::new(seed);
            let mut extra = Vec::new();
            for pass in 1..passes {
                let turn = (90.0 * ((random.next_float() - 0.5) * jitter)) as i32;
                extra.push(landscapes::uplift(resolution, scale, height, octaves, direction + turn, seed + pass, terrain));
            }
            for uplift in extra {
                map = combiner::max(&map, &uplift, 1.0);
            }
        }
        let map = mask_by_input(context, map);
        context.commit(map)
    }
}

/// `Erosions.PE` without a mask; the eroded height.
fn eroded(map: &Map, settings: ErosionSettings, context: &BuildContext) -> Map {
    let terrain = (context.terrain_width(), context.terrain_height());
    parallel_erosion::erode(map, None, &settings, terrain).height
}

pub struct MountainSide;

impl NodeBuilder for MountainSide {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let scale = context.f32("Scale");
        let detail = context.f32("Detail");
        let direction = context.i32("Direction");
        let seed = context.i32("Seed");
        let style = MountainStyle::from_name(context.enum_name("Style"));
        let terrain = (context.terrain_width(), context.terrain_height());
        let size = context.resolution.x;
        let defaults = ErosionSettings::default();

        if context.enum_name("Type") == "Peak" {
            let noise = NoisesPerlin {
                scale: 0.3 + scale * (0.7 - 0.3),
                octaves: 5,
                gain: 0.38,
                seed,
                ..NoisesPerlin::default()
            };
            let mut map = noise::noises_perlin(size, &noise);
            let peak = ErosionSettings {
                duration: 15.0,
                downcutting: 0.5,
                coarse_sediments_amount: 1.0,
                shape: 0.45,
                shape_sharpness: 0.75,
                shape_detail_scale: detail,
                seed,
                ..defaults
            };
            map = match style {
                MountainStyle::Basic => {
                    let warp = FractalWarpSettings {
                        seed,
                        ..FractalWarpSettings::default()
                    };
                    warps::fractal_warp(&map, &warp, None, terrain.0, terrain.1)
                }
                MountainStyle::Eroded => eroded(&map, peak, context),
                MountainStyle::Alpine => {
                    let alpine = ErosionSettings {
                        duration: 45.0,
                        bed_load_angle: 0.0,
                        coarse_sediments_amount: 0.0,
                        coarse_sediments_angle: 0.0,
                        shape_detail_scale: detail * 0.25,
                        ..peak
                    };
                    eroded(&map, alpine, context)
                }
                MountainStyle::Old => {
                    let first = ErosionSettings {
                        shape_sharpness: 0.35,
                        shape_detail_scale: detail * 0.5,
                        ..peak
                    };
                    let worn = eroded(&map, first, context);
                    let second = ErosionSettings {
                        duration: 35.0,
                        erosion_scale: 1200.0,
                        shape_sharpness: 0.35,
                        seed: seed + 1,
                        ..peak
                    };
                    eroded(&worn, second, context)
                }
                MountainStyle::Strata => {
                    let worn = eroded(&map, peak, context);
                    fractal_terraces::complex_terraces(&worn, 0.5, 0.0, 0.1, 0.5, direction, 12, seed, terrain).0
                }
            };
            let map = mask_by_input(context, map);
            return context.commit(map);
        }

        let noise_size = (6 + (96.0 * detail) as i32).max(1) as usize;
        let noise = NoisesPerlin {
            scale,
            octaves: 10,
            gain: 0.5,
            seed,
            ..NoisesPerlin::default()
        };
        let coarse = noise::noises_perlin(noise_size, &noise);
        let resized = transformer::resize_with(&coarse, context.resolution, ResizeMethod::LinearFast);
        let center = size as f32 / 2.0;
        let ramp = gradients::linear(size, center, center, size as f32, direction as f32, gradients::EdgeBehaviour::Mirror);
        let mut slope = combiner::blend_maps(&ramp, &resized, 0.35);
        slope = maths::autolevel(slope);
        let cells = FractalWarpSettings {
            noise_type: WarpNoiseType::VoronoiR,
            roughness: 0.43,
            seed: seed + 2,
            ..FractalWarpSettings::default()
        };
        slope = warps::fractal_warp(&slope, &cells, None, terrain.0, terrain.1);
        let rough = FractalWarpSettings {
            seed: seed + 1,
            ..FractalWarpSettings::default()
        };
        let roughened = warps::fractal_warp(&slope, &rough, None, terrain.0, terrain.1);
        slope = combiner::min(&slope, &roughened, 1.0);

        slope = match style {
            MountainStyle::Eroded => {
                let settings = ErosionSettings {
                    duration: 10.0,
                    downcutting: 0.1,
                    erosion_scale: 666.0 + 1400.0 * scale,
                    bed_load_amount: 0.6,
                    shape: 0.8,
                    shape_detail_scale: 0.4,
                    seed,
                    ..defaults
                };
                eroded(&slope, settings, context)
            }
            MountainStyle::Alpine => {
                let settings = ErosionSettings {
                    duration: 40.0,
                    downcutting: 0.5,
                    erosion_scale: 600.0 + 1200.0 * scale,
                    bed_load_amount: 0.6,
                    shape: 0.8,
                    shape_detail_scale: 0.0,
                    seed,
                    ..defaults
                };
                eroded(&slope, settings, context).multiply_scalar(1.5)
            }
            MountainStyle::Old => {
                let first = ErosionSettings {
                    duration: 15.0,
                    downcutting: 0.12,
                    erosion_scale: 500.0 + 1200.0 * scale,
                    coarse_sediments_amount: 1.0,
                    shape: 0.45,
                    shape_sharpness: 0.35,
                    shape_detail_scale: detail * 0.5,
                    seed,
                    ..defaults
                };
                let worn = eroded(&slope, first, context);
                let second = ErosionSettings {
                    duration: 35.0,
                    downcutting: 0.23,
                    erosion_scale: 1800.0 * scale,
                    shape_detail_scale: detail,
                    seed: seed + 1,
                    ..first
                };
                eroded(&worn, second, context)
            }
            MountainStyle::Strata => {
                let settings = ErosionSettings {
                    duration: 15.0,
                    downcutting: 0.5,
                    erosion_scale: 500.0 + 1000.0 * scale,
                    coarse_sediments_amount: 1.0,
                    shape: 0.45,
                    shape_sharpness: 0.75,
                    shape_detail_scale: detail,
                    seed,
                    ..defaults
                };
                let worn = eroded(&slope, settings, context);
                fractal_terraces::complex_terraces(&worn, 0.5, 0.0, 0.1, 0.5, direction, 12, seed, terrain).0
            }
            MountainStyle::Basic => slope,
        };
        let slope = mask_by_input(context, slope);
        context.commit(slope)
    }
}

pub struct Volcano;

impl NodeBuilder for Volcano {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let size = context.resolution.x;
        let scale = context.f32("Scale");
        let seed = context.i32("Seed");
        let center_x = size as f32 * context.f32("X");
        let center_y = size as f32 * context.f32("Y");
        let radius = size as f32 * (scale * 0.5);
        let cone = gradients::radial(size, center_x, center_y, radius, context.f32("Height"));
        let mouth = context.f32("Mouth");
        let folds = 0.2 + mouth * (0.6 - 0.2);
        let mut volcano = profiles::fold(&cone, None, FoldWaveform::Triangle, folds, false);
        maths::strengthen(&mut volcano, context.f32("Bulk"));
        volcano = maths::drop_to_zero(&volcano);
        if context.enum_name("Surface") == "Eroded" {
            let warp = FractalWarpSettings {
                size: 0.1,
                noise_type: WarpNoiseType::VoronoiR,
                seed,
                ..FractalWarpSettings::default()
            };
            let terrain = (context.terrain_width(), context.terrain_height());
            let warped = warps::fractal_warp(&volcano, &warp, None, terrain.0, terrain.1);
            volcano = combiner::min(&volcano, &warped, 1.0);
            let settings = ErosionSettings {
                duration: 10.0,
                bed_load_amount: 0.5,
                seed,
                ..ErosionSettings::default()
            };
            volcano = eroded(&volcano, settings, context);
        }
        let volcano = mask_by_input(context, volcano);
        context.commit(volcano)
    }
}

pub struct Pond;

impl NodeBuilder for Pond {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let size = context.resolution.x;
        let seed = context.i32("Seed");
        let warp_amount = context.f32("Warp");
        let terrain = (context.terrain_width(), context.terrain_height());
        context.render_secondary_as(RenderIntent::Mask);

        let center_x = size as f32 * context.f32("X");
        let center_y = size as f32 * context.f32("Y");
        let radius = (size as i32 - 4) as f32 * (context.f32("Scale") * 0.1);
        let depth = context.f32("Depth") * 0.25;
        let bowl = gradients::hemisphere_flattened(Resolution::square(size), center_x, center_y, radius, depth);
        let shore = FractalWarpSettings {
            size: 0.1,
            strength: warp_amount,
            noise_type: WarpNoiseType::VoronoiR,
            seed,
            ..FractalWarpSettings::default()
        };
        let bowl = warps::fractal_warp(&bowl, &shore, None, terrain.0, terrain.1);
        let water_level = Map::filled(context.resolution, 0.5);
        let erosion = ErosionSettings {
            duration: 150.0,
            downcutting: 0.26434818,
            erosion_scale: 25.0,
            suspended_load_amount: 0.0,
            bed_load_amount: 1.0,
            coarse_sediments_amount: 1.0,
            shape: 0.14546323,
            shape_sharpness: context.f32("Spread"),
            shape_detail_scale: 0.157864,
            seed,
            ..ErosionSettings::default()
        };
        let worn = eroded(&water_level.subtract(&bowl), erosion, context);
        let ripple = FractalWarpSettings {
            size: 0.07,
            strength: warp_amount,
            seed: seed + 2,
            ..FractalWarpSettings::default()
        };
        let mut pond = warps::fractal_warp(&worn, &ripple, None, terrain.0, terrain.1);
        maths::invert(&mut pond);
        let pond = maths::drop_to_zero(&pond);
        let carved = input.subtract(&pond);
        context.commit(carved)
    }
}

pub struct Rockscape;

impl NodeBuilder for Rockscape {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let settings = LargeRockSettings {
            size: context.f32("Size"),
            fracture: context.f32("Erosion"),
            roughness: context.f32("Roughness"),
            breakage: context.f32("Breakage"),
            surface: 0.0,
            clusters: RockClusters::from_name(context.enum_name("Clusters")),
            relief: RockRelief::from_name(context.enum_name("Relief")),
            jitter: context.f32("Jitter"),
            seed: context.i32("Seed"),
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let rocks = rock_core::large(context.resolution.x, &settings, terrain);
        match context.input("In") {
            Some(input) => {
                let embedded = combiner::embed(&input, &rocks, 1.0, 0.0);
                context.commit(embedded)
            }
            None => context.commit(rocks),
        }
    }
}

pub struct RockNoise;

impl NodeBuilder for RockNoise {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let size = context.resolution.x;
        let [low, high] = context.float2("Size");
        let seed = context.i32("Seed");
        let terrain = (context.terrain_width(), context.terrain_height());
        let mut map = rock_core::noise(size, context.i32("Variety"), context.i32("Octaves"), (low, high), seed, terrain);
        match context.enum_name("Style") {
            "B" => map = slope_blur::blur_map(&map, &map.clone(), 0.25, 1, 0.0, 1.0, false, slope_blur::Window::Bell, 0, 0),
            "C" => map = slope_blur::warp_map(&map, &map.clone(), 0.25, 1, 0.0, false, 0, 0),
            _ => {}
        }
        match context.input("In") {
            Some(input) => context.commit(input.add(&map)),
            None => context.commit(map),
        }
    }
}

pub struct Crater;

impl NodeBuilder for Crater {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        if context.enum_name("Style") != "New" {
            return crater_old(context);
        }
        let base = match context.input("In") {
            Some(input) => (*input).clone(),
            None => Map::filled(context.resolution, 0.5),
        };
        let formation = context.f32("Formation");
        let map = craterfield::crater(
            &base,
            context.f32("Scale"),
            context.f32("Height") * 2.0,
            0.25 + formation * 1.25,
            context.i32("Seed"),
            context.f32("X"),
            context.f32("Y"),
        );
        context.commit(map)
    }
}

pub struct CraterField;

impl NodeBuilder for CraterField {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let base = match context.input("In") {
            Some(input) => (*input).clone(),
            None => Map::filled(context.resolution, 0.5),
        };
        let map = craterfield::craterfield(
            &base,
            context.f32("Scale"),
            context.f32("Density"),
            context.f32("Depth"),
            context.f32("X"),
            context.f32("Y"),
            context.i32("Seed"),
        );
        context.commit(map)
    }
}

pub struct Sandstone;

impl NodeBuilder for Sandstone {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let settings = SandstoneSettings {
            passes: context.i32("Passes"),
            iterations: context.i32("Iterations"),
            spacing: context.f32("Spacing"),
            intensity: context.f32("Intensity"),
            convexity: context.f32("Convexity"),
            tilt: context.f32("Tilt"),
            direction: context.i32("Direction"),
            chaos: context.f32("Chaos"),
            seed: context.i32("Seed"),
            chipped: context.bool("Chipped"),
            chipping: context.f32("Chipping"),
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let (rock, layers) = rock_core::sandstone(&input, &settings, terrain);
        context.commit(rock)?;
        context.commit_port("Layers", layers, RenderIntent::Default);
        Ok(())
    }
}

pub struct Steps;

impl NodeBuilder for Steps {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let scale = context.f32("Scale");
        let density = context.f32("Density");
        let seed = context.i32("Seed");
        let count = 10 + (40.0 * density) as i32;
        let terraces = (128 + (486.0 * scale) as i32).max(0) as usize;
        let mut map = (*input).clone();
        for pass in 0..count {
            map = profiles::terrace(&map, terraces, 0.5 + density * 0.5, 0.45, 0.5, seed + pass, false);
        }
        let settings = SandstoneSettings {
            seed,
            chipped: false,
            ..SandstoneSettings::default()
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let (rock, _) = rock_core::sandstone(&map, &settings, terrain);
        context.commit(rock)
    }
}

pub struct Plates;

impl NodeBuilder for Plates {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let terrain = (context.terrain_width(), context.terrain_height());
        let map = landscapes::plates(
            context.resolution.x,
            context.f32("Scale"),
            context.f32("Range"),
            context.f32("Falloff"),
            context.f32("Warp"),
            context.f32("Angle"),
            context.i32("Seed"),
            terrain,
        );
        let map = mask_by_input(context, map);
        context.commit(map)
    }
}

/// Glacier: snow-covered ice drifts, cut by flow and breakage.
pub struct Glacier;

impl NodeBuilder for Glacier {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let size = context.resolution.x;
        let seed = context.i32("Seed");
        let scale = context.f32("Scale");
        let scale2 = context.f32("Scale2");
        let direction = context.i32("Direction");
        let terrain = (context.terrain_width(), context.terrain_height());

        let reference = match context.input("Reference") {
            Some(reference) => (*reference).clone(),
            None => context.accumulation("Snow", "Depth", None, 0),
        };
        let mut snow = maths::autolevel(reference);
        maths::strengthen(&mut snow, 1.0);
        snow = snow.multiply_scalar(1.0 + context.f32("Height"));
        if context.bool("RoughEdges") {
            let warp = FractalWarpSettings {
                size: 0.05,
                seed,
                ..FractalWarpSettings::default()
            };
            snow = warps::fractal_warp(&snow, &warp, None, terrain.0, terrain.1);
        }

        let mut map = (*input).clone();
        if context.bool("FlowBreakage") {
            let erosion = if context.bool("Extreme") {
                ErosionSettings {
                    duration: 180.0,
                    downcutting: 1.0,
                    erosion_scale: 10.0,
                    suspended_load_amount: 0.0,
                    shape_detail_scale: 0.0,
                    seed,
                    ..ErosionSettings::default()
                }
            } else {
                ErosionSettings {
                    duration: 10.0,
                    erosion_scale: 15.0,
                    suspended_load_amount: 0.0,
                    shape_detail_scale: 0.1,
                    seed,
                    ..ErosionSettings::default()
                }
            };
            let flow = parallel_erosion::erode(&map, None, &erosion, terrain).flow.multiply(&snow);
            let flow = profiles::graphic_eq(&flow, [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.2]);
            map = combiner::subtract(&map, &flow, context.f32("FlowBreakageDepth"));
        }

        let drift = noise::DriftSettings {
            passes: 6,
            chaos: 0.05,
            scale_large: scale2,
            scale_small: context.f32("Breakage") * 0.5,
            turbulence_large: scale,
            turbulence_small: scale * 0.25,
            wind_direction_large: direction as f32,
            wind_direction_small: direction as f32 + 15.0,
            overlap_shapes: true,
            seed,
        };
        let drifts = noise::drift_noise(size, &drift, terrain);
        let layers = SandstoneSettings {
            passes: 3,
            iterations: 10,
            spacing: scale2 * 0.5,
            intensity: 0.8,
            convexity: 0.0,
            tilt: 0.5,
            direction,
            chaos: 0.36,
            seed,
            chipped: context.bool("Chipped"),
            chipping: 0.45,
        };
        let mut ice = rock_core::sandstone(&drifts, &layers, terrain).0;
        ice = ice.multiply(&snow);
        ice = ice.multiply_scalar(context.f32("Thickness") + 0.2);
        map = combiner::add(&map, &ice, 1.0);

        if context.bool("SecondaryBreakage") {
            let cracks = CrackSettings {
                octaves: 10,
                scale,
                seed: seed + 5,
                scale_x: 2.0,
                scale_y: 2.0,
                jitter: 0.7,
                warp_size: 0.6,
                warp_strength: 0.7,
                style: CrackStyle::Normal,
                depth: 0.2,
            };
            let cracks = noise::cracks(context.resolution, &cracks, terrain).multiply(&snow);
            map = map.subtract(&cracks.multiply_scalar(0.01));
        }
        if context.bool("DiagonalBreakage") {
            let breakage = SandstoneSettings {
                passes: context.i32("BreakageCount"),
                iterations: 10,
                spacing: 0.2,
                intensity: 0.5,
                convexity: 0.0,
                tilt: 0.5,
                direction: context.i32("DiagonalBreakageDirection"),
                chaos: 0.4,
                seed: seed + 1,
                chipped: false,
                chipping: 0.45,
            };
            let broken = rock_core::sandstone(&map, &breakage, terrain).0;
            map = combiner::min(&map, &broken, 1.0);
            map = combiner::max(&input, &map, 1.0);
        }
        if context.bool("Substructure") {
            let density = (100.0 * context.f32("SubstructureDensity")) as i32;
            let rocks = rock_core::noise(size, 3, density, (scale, scale + scale * 0.5), seed, terrain).multiply(&snow);
            map = map.add(&rocks);
            let occlusion = aspect::occlusion(&input, 9.5, 1, true);
            let shading = histogram::equalize(occlusion).multiply(&snow);
            map = combiner::mix(BlendMode::Multiply, 0.04 * context.f32("SubstructureDepth"), &map, &shading);
        }
        context.commit(map)?;
        context.commit_port("Snow", snow, RenderIntent::Default);
        Ok(())
    }

    /// `Glacier.GetCustomColorMap`: the snow reference (else the snow depth
    /// gathered from snow nodes) levelled, strengthened and squeezed into
    /// 0.6–1, grey rock up to white ice.
    fn custom_colour_map(&self, context: &BuildContext) -> Option<Map> {
        let reference = match context.input("Reference") {
            Some(reference) => (*reference).clone(),
            None => context.accumulation("Snow", "Depth", None, 0),
        };
        let mut ice = maths::autolevel(reference);
        maths::strengthen(&mut ice, 1.0);
        Some(maths::fit_range(&ice, 0.6, 1.0))
    }
}

/// Stones: small crater fields pressed upwards out of the map.
pub struct Stones;

impl NodeBuilder for Stones {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let scale = 0.001 + context.f32("Scale") * (0.02 - 0.001);
        let inverted = input.subtract_from(1.0);
        let field = craterfield::craterfield(&inverted, scale, context.f32("Density"), context.f32("Height") * 2.0, 0.0, 0.0, context.i32("Seed"));
        let stones = field.subtract_from(1.0);
        let mut mask = combiner::difference(&stones, &input, 1.0);
        maths::clamp(&mut mask);
        context.commit(stones)?;
        context.commit_port("Stones", mask, RenderIntent::Mask);
        Ok(())
    }
}

/// Scree: loose stones scattered where the terrain's flow gathers.
pub struct Scree;

impl NodeBuilder for Scree {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let guide = context.input("Guide");
        let seed = context.i32("Seed");
        let scale = 0.001 + context.f32("Scale") * (0.02 - 0.001);
        let depth = context.f32("Height") * 2.0;

        let mut stones = (*input).clone();
        for layer in 0..context.i32("Density") {
            let inverted = stones.subtract_from(1.0);
            let field = craterfield::craterfield(&inverted, scale, 0.4, depth, 0.0, 0.0, seed + layer);
            stones = field.subtract_from(1.0);
        }

        let settings = FlowSettings {
            flow_length: 1.0,
            flow_volume: context.f32("Edge"),
            seed,
        };
        let flow = flow::simulate(&input, guide.as_deref(), &settings);
        let mut coverage = maths::autolevel(flow);
        maths::strengthen(&mut coverage, context.f32("Spread"));
        let scree = masking::mask(&input, &stones, &coverage);

        let mut difference = combiner::difference(&scree, &input, 1.0);
        maths::clamp(&mut difference);
        context.commit(scree)?;
        context.commit_port("Stones", difference, RenderIntent::Mask);
        Ok(())
    }
}

/// Craggy: rocky crags embedded into the map (`Migrated.CraggyProcess`).
pub struct Craggy;

impl NodeBuilder for Craggy {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let terrain = (context.terrain_width(), context.terrain_height());
        let map = migrated::craggy(&input, context.i32("Seed"), context.f32("Size"), context.f32("Depth"), context.f32("Shape"), terrain);
        context.commit(map)
    }
}

/// Bulbous: rounded, bulb-like features (`Migrated.BulbousProcess`).
pub struct Bulbous;

impl NodeBuilder for Bulbous {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let map = migrated::bulbous(&input, context.f32("Scale"), context.f32("Jitter"));
        context.commit(map)
    }
}

/// Roughen: fine flow lines carved into the map (`Migrated.RoughenProcess`).
pub struct Roughen;

impl NodeBuilder for Roughen {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let map = migrated::roughen(&input, context.f32("Width"), context.f32("Depth"), context.bool("Oriented"), context.enum_name("Quality"));
        context.commit(map)
    }
}

/// One `Distress.GetCanyon`: clipped Voronoi cells, warped, recurved and
/// warped again. The scale jitter is drawn from the node seed, so every
/// canyon gets the same one.
#[allow(clippy::too_many_arguments)]
fn distress_canyon(size: usize, node_seed: i32, scale: f32, influence: f32, subduction: f32, seed: i32, stretch: (f32, f32), terrain: (f32, f32)) -> Map {
    let mut random = XorShift64Star::new(node_seed);
    let cells = VoronoiSettings {
        scale: random.next_float_range(scale - scale * 0.25, scale + scale * 0.25),
        function: CellularDistanceFunction::Euclidean,
        form: CellularReturnType::Distance2,
        warp: noise::perturb_for_warp(2),
        warp_frequency: 0.1,
        warp_amplitude: 0.3,
        warp_octaves: 12,
        seed,
        scale_x: stretch.0,
        scale_y: stretch.1,
        jitter: subduction,
    };
    let cells = maths::autolevel(noise::voronoi(size, &cells, Viewport::with_offset([0.0, 0.0])));
    let clipped = maths::clip_map(&cells, influence, 1.0);
    let first_warp = FractalWarpSettings {
        size: 0.2,
        strength: 0.8,
        octaves: 10,
        edges: transformer::EdgeBehaviour::Mirror,
        seed: seed + 101,
        ..FractalWarpSettings::default()
    };
    let warped = warps::fractal_warp(&clipped, &first_warp, None, terrain.0, terrain.1);
    let recurved = profiles::recurve(&warped, 1.0, 3, false, true, 0.01);
    let recurved = profiles::recurve(&recurved, 1.0, 3, false, true, 0.01);
    let second_warp = FractalWarpSettings {
        size: 0.35,
        strength: 0.75,
        octaves: 10,
        edges: transformer::EdgeBehaviour::Mirror,
        seed: seed + 102,
        ..FractalWarpSettings::default()
    };
    warps::fractal_warp(&recurved, &second_warp, None, terrain.0, terrain.1)
}

/// Distress: Voronoi canyons cut into the map.
pub struct Distress;

impl NodeBuilder for Distress {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let size = context.resolution.x;
        let seed = context.i32("Seed");
        let scale = context.f32("Scale");
        let influence = 0.5 + (0.5 - context.f32("Influence") * 0.5);
        let subduction = 1.0 - context.f32("Formation");
        let stretch = (context.f32("ScaleX"), context.f32("ScaleY"));
        let depth = context.f32("Depth");
        let terrain = (context.terrain_width(), context.terrain_height());
        let octaves = context.i32("Octaves").max(0);
        let canyons: Vec<Map> = (0..octaves)
            .map(|octave| distress_canyon(size, seed, scale, influence, subduction, seed + octave, stretch, terrain))
            .collect();

        let mut ground = Map::filled(context.resolution, 1.0);
        let mut random = XorShift64Star::new(seed);
        for canyon in &canyons {
            let mut shaped = canyon.clone();
            let chance = random.next_float();
            if chance as f64 > 0.33 {
                let inflate = chance as f64 > 0.66;
                shaped = profiles::recurve(canyon, 1.0, 3, inflate, false, 0.1);
                if chance as f64 > 0.9 {
                    shaped = profiles::recurve(canyon, 1.0, 3, inflate, false, 0.1);
                }
            }
            shaped = profiles::graphic_eq(&shaped, [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 2.0]);
            let amount = depth * 0.3 * random.next_float();
            ground = combiner::subtract(&ground, &shaped, amount);
        }
        context.commit(input.multiply(&ground))
    }
}

/// GroundTexture: rocky curvature detail, or Rough Perlin noise seeded
/// with the node id.
pub struct GroundTexture;

impl NodeBuilder for GroundTexture {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let style = migrated::GroundStyle::from_name(context.enum_name("Method"));
        let strength = context.f32("Strength");
        if style == migrated::GroundStyle::Rough {
            let parameters = NoisesPerlin {
                gain: 0.9,
                seed: context.node_id(),
                ..NoisesPerlin::default()
            };
            let noise = noise::noises_perlin(context.resolution.x, &parameters);
            let detail = noise.multiply_scalar(strength * 0.03);
            return context.commit(input.add(&detail));
        }
        let coverage = context.f32("Coverage");
        let density = context.f32("Density");
        let terrain_height = context.terrain_height();
        let map = migrated::ground_texture(&input, style, strength, coverage, density, terrain_height);
        context.commit(map)
    }
}

/// Crater's Old style: folded radial rings, thermally shaped.
fn crater_old(context: &mut BuildContext) -> Result<(), NodeError> {
    let size = context.resolution.x;
    let width = size as f32;
    let folds = context.f32("Formation") * 0.75;
    let x = context.f32("X");
    let y = context.f32("Y");
    let radius = width * context.f32("Scale") * 2.0;
    let rings = gradients::radial(size, width * x, width * y, radius, 1.0);
    let waveform = if context.enum_name("Rim") == "Soft" { FoldWaveform::Sine } else { FoldWaveform::Triangle };
    let folded = profiles::fold(&rings, None, waveform, folds, false);
    let shaped = profiles::thermal_shaper(&folded, 1.0, context.f32("Shape"));
    let lowered = maths::drop_to_zero(&shaped);
    let mut crater = maths::fit_range(&lowered, 0.0, context.f32("Height"));
    if context.is_connected("In") {
        let input = context.require_input("In")?;
        crater = crater.multiply(&input);
    }
    context.commit(crater)
}

/// MountainRange: three ridge layers, warped and eroded.
pub struct MountainRange;

impl NodeBuilder for MountainRange {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let settings = MountainRangeSettings {
            scale: context.f32("Scale"),
            height: context.f32("Height"),
            style: MountainRangeStyle::from_name(context.enum_name("Style")),
            bulk: MountainBulk::from_name(context.enum_name("Bulk")),
            seed: context.i32("Seed"),
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let map = landscapes::mountain_range(context.resolution, &settings, terrain);
        let map = mask_by_input(context, map);
        context.commit(map)
    }
}

/// DuneSea: Gabor dune fields, steered by the input.
pub struct DuneSea;

impl NodeBuilder for DuneSea {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let settings = DuneSeaSettings {
            dune_type: context.enum_value("DuneType") as usize,
            scale: context.f32("Scale"),
            height: context.f32("Height") * 0.25,
            direction: context.i32("Direction"),
            softness: context.i32("Softness"),
            sharpness: context.f32("Sharpness"),
            seed: context.i32("Seed"),
            chaos: context.enum_value("Chaos") as usize,
            undulation: context.enum_value("Undulation") as usize,
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let aniso = context.input("In");
        let map = landscapes::dune_sea(context.resolution, &settings, aniso.as_deref(), terrain);
        let map = mask_by_input(context, map);
        context.commit(map)
    }
}

/// Sand: fine ripples, either steered by the input terrain or embedded in
/// it.
pub struct Sand;

impl NodeBuilder for Sand {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.input("In");
        let warp_by_terrain = context.bool("WarpByTerrain");
        let aniso = if warp_by_terrain { input.as_deref() } else { None };
        let height = context.f32("Height") * 0.1;
        let mut map = landscapes::sand(context.resolution, context.f32("Scale"), context.f32("Chaos"), height, context.f32("Softness"), context.i32("Direction"), context.i32("Seed"), aniso);
        if let Some(input) = &input
            && !warp_by_terrain {
                map = combiner::embed(input, &map, 1.0, 0.0);
            }
        context.commit(map)
    }
}

/// Outcrops: scattered terraced rock domes, crumbled and chipped.
pub struct Outcrops;

impl NodeBuilder for Outcrops {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let settings = rock_core::OutcropSettings {
            variations: context.i32("Variations"),
            strata: context.f32("Strata"),
            density: context.f32("Density"),
            shape: context.f32("Shape"),
            chipped: context.bool("Chipped"),
            seed: context.i32("Seed"),
            size: context.float2("Size"),
            height: context.float2("Height"),
            rotation: context.float2("Rotation"),
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let map = rock_core::outcrops(&input, &settings, terrain);
        context.commit(map)
    }
}
