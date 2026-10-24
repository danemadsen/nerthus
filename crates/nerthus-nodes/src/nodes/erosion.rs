//! The erosion family built on the native `ParallelErosion_Erode`.

use nerthus_engine::{BuildContext, Map, NodeBuilder, NodeError, RenderIntent, Resolution};
use nerthus_erosion::hydro;

use crate::aspect;
use crate::cellular3d::{self, Cellular3dSettings};
use crate::colour;
use crate::colour_transport::{self, ColourTransportSettings};
use crate::combiner;
use crate::erosions::{self, ClassicErosionSettings, LichtenbergSettings, ThermalSettings};
use crate::fast_blur;
use crate::flow::{self, FlowSettings};
use crate::fractal_terraces;
use crate::gradients::{self, EdgeBehaviour};
use crate::histogram;
use crate::lake::{self, LakeSettings};
use crate::landscapes::Abc;
use crate::lighting;
use crate::masking;
use crate::maths;
use crate::migrated;
use crate::noise::{self, WaveShineSettings};
use crate::others;
use crate::parallel_erosion::{self, ErosionMaps, ErosionSettings};
use crate::profiles;
use crate::random::XorShift64Star;
use crate::random_noises;
use crate::rock_core;
use crate::shade::{AmbientOcclusionModel, LightRig};
use crate::simulations;
use crate::surfacer;
use crate::thermal;
use crate::transformer::{self, ResizeMethod};
use crate::warps::{self, FractalWarpSettings};
use crate::waters::{self, LakesSettings, SeaArrangement, SeaSettings};
use crate::x86;

/// Clamps to [0, 1].
fn saturate(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

/// ErosionPro: droplet erosion with suspended, bed and coarse load.
pub struct ErosionPro;

impl NodeBuilder for ErosionPro {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        context.render_secondary_as(RenderIntent::Mask);
        let mut inhibitors = if context.bool("Enable") { Some(erosion_pro_inhibitors(context, &input)) } else { None };
        let precipitation = context.input_at(1);
        if let (Some(inhibitors), Some(precipitation)) = (inhibitors.as_mut(), precipitation.as_deref()) {
            // `Process(mapB, (i, a) => i * a)`: the inhibitors keep their one
            // channel and take the precipitation's first.
            let rain = precipitation.data();
            for (value, &amount) in inhibitors.data_mut().iter_mut().zip(rain) {
                *value *= amount;
            }
        }
        let precipitation_map: Option<&Map> = inhibitors.as_ref().or(precipitation.as_deref());

        let no_erosion = context.bool("NoErosion");
        let shaping = |value: f32| if no_erosion { 0.0 } else { value };
        let deposition_boost = context.f32("DepositionBoost");
        let settings = ErosionSettings {
            duration: context.f32("Duration"),
            downcutting: context.f32("Downcutting"),
            downcutting_type: 0.0,
            erosion_scale: context.f32("ErosionScale"),
            suspended_load_amount: context.f32("SuspendedLoadDischargeAmount"),
            suspended_load_angle: context.f32("SuspendedLoadDischargeAngle"),
            bed_load_amount: context.f32("BedLoadDischargeAmount"),
            bed_load_angle: context.f32("BedLoadDischargeAngle"),
            coarse_sediments_amount: context.f32("CoarseSedimentsDischargeAmount"),
            coarse_sediments_angle: context.f32("CoarseSedimentsDischargeAngle"),
            shape: shaping(context.f32("Shape")),
            shape_sharpness: shaping(context.f32("ShapeSharpness")),
            shape_detail_scale: shaping(context.f32("ShapeDetailScale")),
            seed: context.i32("Seed"),
            deposition_boost: saturate(deposition_boost * 2.0),
            extra_deposition_boost: saturate((deposition_boost - 0.5) * 2.0),
            no_erosion,
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let report = |fraction| context.report_progress(fraction);
        let maps = parallel_erosion::erode_with_progress(&input, precipitation_map, &settings, terrain, &report);
        context.commit(maps.height)?;
        context.commit_port("Flow", maps.flow, RenderIntent::Default);
        context.commit_port("Deposits", maps.deposition, RenderIntent::Default);
        context.commit_port("Wear", maps.wear, RenderIntent::Default);
        Ok(())
    }
}

/// The inhibitors (`BuildInhibitors`), the orographic influence: rain
/// falls on the slopes and altitudes in range and, when it is directional,
/// only where the terrain does not shadow it from the rain's direction. A
/// stronger rain shadow lowers that "sun" towards the horizon and narrows
/// its disk.
fn erosion_pro_inhibitors(context: &BuildContext, heights: &Map) -> Map {
    let terrain = (context.terrain_width(), context.terrain_height());
    let [slope_min, slope_max] = context.float2("Slope");
    let [altitude_min, altitude_max] = context.float2("Altitude");
    let slope = aspect::slope(heights, slope_min, slope_max, 0.0, terrain.0, terrain.1);
    let altitude = aspect::height_normalized(heights, altitude_min, altitude_max, 0.0);
    let mut inhibitors = altitude.multiply(&slope);

    if context.bool("DirectionalPrecipitation") {
        let rain_shadow = context.f32("RainShadow");
        let shadowing = rain_shadow * (2.0 - rain_shadow);
        let rig = LightRig {
            shadow_quality: 5,
            hybrid_ao_octaves: 2,
            hybrid_ao_quality: 4,
            sun_angular_diameter: 20.0 * (1.0 - shadowing),
            direct_lighting: true,
            shadows: true,
            ambient_lighting: false,
            ambient_occlusion: false,
            ao_model: AmbientOcclusionModel::Hybrid,
            ..LightRig::default()
        };
        let direction = context.i32("Direction") as f32;
        let sky = rig.sky(direction, (1.0 - shadowing) * 80.0);
        let lit = rig.shade(&sky, heights, None, terrain);
        inhibitors = inhibitors.multiply(&lit.shadow);
    }

    if context.bool("Reverse") {
        maths::invert(&mut inhibitors);
    }
    inhibitors
}

/// `Erosions.PE` with the defaults for everything `settings` leaves out.
fn erode(map: &Map, settings: ErosionSettings, context: &BuildContext) -> ErosionMaps {
    let terrain = (context.terrain_width(), context.terrain_height());
    let report = |fraction| context.report_progress(fraction);
    parallel_erosion::erode_with_progress(map, None, &settings, terrain, &report)
}

/// EasyErosion: preset chains of `Erosions.PE`.
pub struct EasyErosion;

impl NodeBuilder for EasyErosion {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let mut map = (*input).clone();
        let influence = context.f32("Influence");
        let seed = context.i32("Seed");
        let bias_angle = match context.enum_name("BiasAngle") {
            "E" => 90.0,
            "S" => 180.0,
            "W" => 270.0,
            _ => 0.0,
        };
        let biased = context.enum_name("BiasAngle") != "X";
        let mut bias = None;
        if biased {
            let size = context.resolution.x;
            let center = size as f32 / 2.0;
            let gradient = gradients::linear(size, center, center, size as f32, bias_angle, EdgeBehaviour::Clip);
            let tilt = gradient.multiply_scalar(0.33);
            map = map.add(&tilt);
            bias = Some(tilt);
        }

        let defaults = ErosionSettings::default();
        let mut output = match context.enum_name("Style") {
            "Simple" => {
                let first = ErosionSettings {
                    duration: 10.0 + 10.0 * influence,
                    bed_load_amount: 0.6,
                    seed: seed + 1,
                    ..defaults
                };
                map = erode(&map, first, context).height;
                let last = ErosionSettings {
                    duration: 6.0 + 6.0 * influence,
                    coarse_sediments_amount: 0.35,
                    coarse_sediments_angle: 45.0,
                    seed: seed + 2,
                    ..defaults
                };
                erode(&map, last, context)
            }
            "Ancient" => {
                let passes = [
                    ErosionSettings { duration: 10.0 + 10.0 * influence, bed_load_amount: influence, seed: seed + 2, ..defaults },
                    ErosionSettings { duration: 10.0 + 10.0 * influence, seed: seed + 3, ..defaults },
                    ErosionSettings { duration: 40.0, bed_load_amount: influence, seed: seed + 4, ..defaults },
                    ErosionSettings { duration: 60.0, shape: influence, seed: seed + 5, ..defaults },
                    ErosionSettings { duration: 80.0, seed: seed + 6, ..defaults },
                ];
                for pass in passes {
                    map = erode(&map, pass, context).height;
                }
                let last = ErosionSettings { duration: 10.0, seed: seed + 7, ..defaults };
                erode(&map, last, context)
            }
            "Ancient2" => {
                let passes = [
                    ErosionSettings { duration: 10.0, seed: seed + 1, ..defaults },
                    ErosionSettings {
                        duration: 25.0 + (15.0 + influence),
                        downcutting: 0.23,
                        downcutting_type: 1.0,
                        erosion_scale: 226.0,
                        shape: 0.8,
                        seed: seed + 2,
                        ..defaults
                    },
                    ErosionSettings {
                        duration: 33.0,
                        shape: 0.89,
                        shape_sharpness: 0.5 + influence * 0.5,
                        seed: seed + 3,
                        ..defaults
                    },
                    ErosionSettings { duration: 33.0, seed: seed + 4, ..defaults },
                ];
                for pass in passes {
                    map = erode(&map, pass, context).height;
                }
                let last = ErosionSettings {
                    duration: 10.0,
                    erosion_scale: 1280.0,
                    coarse_sediments_amount: 0.5,
                    seed: seed + 5,
                    ..defaults
                };
                erode(&map, last, context)
            }
            "Alpine" => {
                let settings = ErosionSettings {
                    duration: 8.0 + 15.0 * (influence * 0.5),
                    suspended_load_amount: 0.45 + influence / 3.0,
                    shape: 0.95,
                    shape_sharpness: 0.97,
                    shape_detail_scale: 1.0 - influence,
                    seed,
                    ..defaults
                };
                erode(&map, settings, context)
            }
            "Rocky" => {
                let terrain = (context.terrain_width(), context.terrain_height());
                let octaves = (influence * 500.0) as i32;
                let rocks = rock_core::noise(map.width(), 1, octaves, (0.4, 1.0), seed, terrain);
                map = map.add(&rocks);
                let first = ErosionSettings {
                    duration: influence / 3.0 * 33.0,
                    seed: seed + 2,
                    ..defaults
                };
                map = erode(&map, first, context).height;
                let last = ErosionSettings {
                    duration: 10.0,
                    seed: seed + 3,
                    ..defaults
                };
                erode(&map, last, context)
            }
            "Flows" => {
                let settings = ErosionSettings {
                    duration: 30.0 + 40.0 * influence,
                    downcutting: 0.64,
                    erosion_scale: 888.0,
                    suspended_load_amount: 0.8,
                    suspended_load_angle: 45.0,
                    bed_load_amount: 0.6 + 0.4 * influence,
                    bed_load_angle: 45.0 + 20.0 * (influence * 0.1),
                    coarse_sediments_amount: 0.8,
                    coarse_sediments_angle: 45.0,
                    shape: 0.61765105,
                    seed,
                    ..defaults
                };
                erode(&map, settings, context)
            }
            "Exposed" => {
                let settings = ErosionSettings {
                    duration: 30.0 + 20.0 * influence,
                    downcutting: 0.64,
                    erosion_scale: 323.0 + 200.0 * influence,
                    suspended_load_amount: 0.80821735,
                    suspended_load_angle: 36.278553,
                    bed_load_amount: 0.8365995,
                    bed_load_angle: 36.917152,
                    coarse_sediments_amount: 0.86295444,
                    coarse_sediments_angle: 39.38032,
                    shape: 0.61765105,
                    seed,
                    ..defaults
                };
                let mut maps = erode(&map, settings, context);
                maps.height = combiner::min(&map, &maps.height, 1.0);
                maps
            }
            "Flows2" => {
                let tilt = 0.05 + 0.1 * influence;
                let direction = context.i32("Direction");
                let terrain = (context.terrain_width(), context.terrain_height());
                map = fractal_terraces::complex_terraces(&map, 0.5, 0.0, 0.02, tilt, direction, 12, seed, terrain).0;
                let settings = ErosionSettings {
                    duration: 30.0 + 45.0 * influence,
                    downcutting: 0.5,
                    erosion_scale: 850.0,
                    suspended_load_amount: 0.80821735,
                    suspended_load_angle: 36.278553,
                    bed_load_amount: 0.8365995,
                    bed_load_angle: 36.917152,
                    coarse_sediments_amount: 0.86295444,
                    coarse_sediments_angle: 39.38032,
                    shape: 0.61765105,
                    seed: seed + 5,
                    ..defaults
                };
                erode(&map, settings, context)
            }
            "Strata" => {
                let spacing = 0.02 + 0.05 * influence;
                let direction = context.i32("Direction");
                let terrain = (context.terrain_width(), context.terrain_height());
                map = fractal_terraces::complex_terraces(&map, 0.5, -0.25, spacing, 0.1, direction, 12, seed + 5, terrain).0;
                let settings = ErosionSettings {
                    duration: 60.0,
                    downcutting: 0.5,
                    erosion_scale: 850.0,
                    suspended_load_amount: 0.80821735,
                    suspended_load_angle: 36.278553,
                    bed_load_amount: 0.8365995,
                    bed_load_angle: 36.917152,
                    coarse_sediments_amount: 0.86295444,
                    coarse_sediments_angle: 39.38032,
                    shape: 0.61765105,
                    seed: seed + 15,
                    ..defaults
                };
                let eroded = erode(&map, settings, context);
                map = combiner::max(&map, &eroded.height, 1.0);
                let last = ErosionSettings {
                    duration: 33.0,
                    seed: seed + 22,
                    ..defaults
                };
                erode(&map, last, context)
            }
            "Flows3" => {
                let settings = ErosionSettings {
                    duration: influence * 100.0,
                    downcutting: 0.34,
                    erosion_scale: 1500.0,
                    suspended_load_amount: 0.8,
                    suspended_load_angle: 66.0,
                    bed_load_amount: 0.8,
                    bed_load_angle: 66.0,
                    coarse_sediments_amount: 0.8,
                    coarse_sediments_angle: 66.0,
                    seed: seed + 2,
                    ..defaults
                };
                erode(&map, settings, context)
            }
            "Withered" => {
                let settings = ErosionSettings {
                    duration: influence * 44.0,
                    downcutting: 1.0,
                    erosion_scale: 2000.0,
                    suspended_load_amount: 0.0,
                    shape: 0.0,
                    seed,
                    ..defaults
                };
                erode(&map, settings, context)
            }
            "SoftSoil" => {
                let settings = ErosionSettings {
                    duration: influence * 44.485607,
                    downcutting: 1.0,
                    erosion_scale: 2000.0,
                    suspended_load_angle: 3.6187322,
                    bed_load_amount: 775.0 / 32767.0,
                    bed_load_angle: 45.0,
                    coarse_sediments_amount: 0.043924853,
                    shape: 1.0,
                    shape_sharpness: 0.041897554,
                    shape_detail_scale: 0.09866198,
                    seed,
                    ..defaults
                };
                erode(&map, settings, context)
            }
            "SoftSoil2" => {
                let settings = ErosionSettings {
                    duration: 6.0 * influence,
                    downcutting: 0.9682472,
                    erosion_scale: 2000.0,
                    bed_load_amount: 0.30747163,
                    coarse_sediments_amount: 0.64379454,
                    shape: 1.0,
                    shape_sharpness: 0.75260484,
                    shape_detail_scale: 0.029095542,
                    seed: 30074,
                    ..defaults
                };
                erode(&map, settings, context)
            }
            "Dessicated" => {
                let settings = ErosionSettings {
                    duration: influence * 65.0,
                    downcutting: 0.2,
                    bed_load_amount: 0.85889983,
                    coarse_sediments_amount: 0.9095824,
                    shape: 0.86700904,
                    shape_sharpness: 0.9399919,
                    seed,
                    ..defaults
                };
                erode(&map, settings, context)
            }
            "Thin" => {
                let settings = ErosionSettings {
                    duration: influence * 10.0,
                    downcutting: 0.7757805,
                    downcutting_type: 0.45749426,
                    erosion_scale: 226.59332,
                    bed_load_amount: 0.85889983,
                    coarse_sediments_amount: 0.9095824,
                    shape: 0.3601838,
                    shape_sharpness: 0.85281795,
                    shape_detail_scale: 0.10271658,
                    seed,
                    ..defaults
                };
                erode(&map, settings, context)
            }
            // No style leaves the result array null.
            _ => return Err(NodeError::Failed("Object reference not set to an instance of an object.".to_string())),
        };

        if let Some(tilt) = bias {
            output.height = output.height.subtract(&tilt);
        }
        context.commit(output.height)?;
        context.commit_port("Flow", output.flow, RenderIntent::Default);
        context.commit_port("Deposits", output.deposition, RenderIntent::Default);
        context.commit_port("Wear", output.wear, RenderIntent::Default);
        Ok(())
    }
}

/// Hillify: rounded hills grown out of the map's low-frequency shape.
pub struct Hillify;

impl NodeBuilder for Hillify {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let aggressive = context.enum_name("Creep") == "Aggressive";
        let eroded_surface = context.enum_name("Surface") == "Eroded";
        let terrain = (context.terrain_width(), context.terrain_height());
        let (hills, mask) = erosions::hillify(&input, context.f32("Coverage"), aggressive, eroded_surface, context.i32("Seed"), terrain);
        context.commit(hills)?;
        context.commit_port("Hills", mask, RenderIntent::Mask);
        Ok(())
    }
}

/// Lichtenberg: branching channels from an eroded noise's flow.
pub struct Lichtenberg;

impl NodeBuilder for Lichtenberg {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let style = match context.enum_name("Style") {
            "B" => Abc::B,
            "C" => Abc::C,
            _ => Abc::A,
        };
        let settings = LichtenbergSettings {
            style,
            scale: context.f32("Scale"),
            complexity: context.i32("Complexity"),
            depth: context.f32("Depth"),
            expand: context.f32("Expand"),
            bias: context.f32("Bias"),
            seed: context.i32("Seed"),
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let map = erosions::lichtenberg(&input, &settings, terrain);
        context.commit(map)
    }
}

/// Sediments: deposits that fill the map's low ground.
pub struct Sediments;

impl NodeBuilder for Sediments {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let scale = context.f32("Scale");
        let maps = match context.enum_name("Type") {
            "Classic" => {
                let style_b = context.enum_name("Style") == "B";
                erosions::sediments_classic(&input, style_b, scale, context.bool("GrainyDeposits"))
            }
            "Fine" => {
                let settings = thermal::ThermalSettings {
                    duration: scale,
                    strength: 0.4,
                    anisotropy: 0.8,
                    talus_angle: context.f32("Angle"),
                    erosion_scale: (20 * context.i32("Passes")) as f32,
                    heightmap_size: context.terrain_width(),
                    heightmap_height: context.terrain_height(),
                    sediment_removal: 0.0,
                };
                let eroded = thermal::simulate(&input, None, None, &settings);
                let raised = eroded.heights.subtract(&input);
                erosions::SedimentMaps {
                    result: combiner::max(&eroded.heights, &input, 1.0),
                    sediments: histogram::equalize(raised),
                    deposits: eroded.deposition,
                }
            }
            _ => {
                context.render_secondary_as(RenderIntent::Mask);
                let terrain = (context.terrain_width(), context.terrain_height());
                let report = |pass: i32, passes: i32| {
                    context.report_progress(pass as f32 / passes as f32);
                    // Gaea repeats pass 1 for the second pass: its first
                    // erosion runs before the loop starts at index 1.
                    let displayed_pass = pass.max(1);
                    context.report_progress_text(format!("Rendering pass {displayed_pass} of {passes}"));
                };
                erosions::sediments_with_progress(&input, context.i32("Passes"), scale, context.node_id(), terrain, report)
            }
        };
        context.commit(maps.result)?;
        context.commit_port("Sediments", maps.sediments, RenderIntent::Mask);
        context.commit_port("Deposits", maps.deposits, RenderIntent::Mask);
        Ok(())
    }
}

/// Shear: erosion and warps that shear the map's slopes.
pub struct Shear;

impl NodeBuilder for Shear {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let strength = context.f32("Strength");
        let seed = context.i32("Seed");
        let terrain = (context.terrain_width(), context.terrain_height());
        let lerp = |low: f32, high: f32| low + strength * (high - low);

        let first = ErosionSettings {
            duration: 10.0,
            downcutting: 0.03176105,
            bed_load_amount: 0.22638194,
            shape_detail_scale: 0.25,
            seed,
            ..ErosionSettings::default()
        };
        let mut map = (*input).clone();
        let worn = erode(&map, first, context).height;
        map = combiner::max(&map, &worn, 1.0);
        let wide = FractalWarpSettings {
            size: lerp(0.2, 0.8) * 2.0,
            strength: lerp(0.2, 0.8),
            iterations: 2,
            seed,
            ..FractalWarpSettings::default()
        };
        let warped = warps::fractal_warp(&map, &wide, None, terrain.0, terrain.1);
        map = combiner::min(&map, &warped, 1.0);
        let fine = FractalWarpSettings {
            size: lerp(0.4, 0.9),
            strength: lerp(0.4, 0.5),
            seed: seed + 5,
            ..FractalWarpSettings::default()
        };
        let warped = warps::fractal_warp(&map, &fine, None, terrain.0, terrain.1);
        map = combiner::min(&map, &warped, 1.0);
        let last = ErosionSettings {
            duration: 50.0,
            downcutting: 0.02,
            seed: seed + 1,
            ..ErosionSettings::default()
        };
        let worn = erode(&map, last, context).height;
        map = combiner::max(&map, &worn, 1.0);
        context.commit(map)
    }
}

/// Wizard2: `Erosions.PE` with settings picked from four presets per
/// control (`Generic4`: Low, Med, High, Ultra).
pub struct Wizard2;

impl NodeBuilder for Wizard2 {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let level = |name: &str| match context.enum_name(name) {
            "Med" => 1,
            "High" => 2,
            "Ultra" => 3,
            _ => 0,
        };
        let power = level("Power");
        let depth = level("Depth");
        let scale = level("Scale");
        let deposits = level("Deposits");
        let flow = level("Flow");
        let shape = level("Shape");
        let detail = level("Detail");
        let settings = ErosionSettings {
            duration: [10.0, 20.0, 45.0, 80.0][power] + [10.0, 20.0, 30.0, 40.0][deposits] * 0.5 + [10.0, 20.0, 30.0, 40.0][flow] * 0.5,
            downcutting: [0.1, 0.25, 0.3, 0.6][depth],
            downcutting_type: 0.0,
            erosion_scale: [400.0, 800.0, 1200.0, 2500.0][scale],
            suspended_load_amount: [0.4, 0.75, 0.8, 1.0][deposits],
            suspended_load_angle: [15.0, 20.0, 33.0, 45.0][deposits],
            bed_load_amount: [0.4, 0.6, 0.7, 0.99][flow] * 0.5 + [0.3, 0.4, 0.6, 0.8][deposits] * 0.5,
            bed_load_angle: [15.0, 20.0, 33.0, 45.0][flow],
            coarse_sediments_amount: [0.13, 0.24, 0.6, 0.6][flow] + [0.13, 0.25, 0.3, 0.4][deposits],
            coarse_sediments_angle: [15.0, 20.0, 33.0, 45.0][flow],
            shape: [0.2, 0.15, 0.3, 0.6][shape],
            shape_sharpness: [0.8, 0.6, 0.3, 0.1][shape],
            shape_detail_scale: [0.0, 0.3, 0.6, 1.0][detail],
            seed: context.i32("Seed"),
            ..ErosionSettings::default()
        };
        let maps = erode(&input, settings, context);
        context.commit(maps.height)?;
        context.commit_port("Flow", maps.flow, RenderIntent::Default);
        context.commit_port("Deposits", maps.deposition, RenderIntent::Default);
        context.commit_port("Wear", maps.wear, RenderIntent::Default);
        Ok(())
    }
}

/// Anastomosis: braided channels from a pipe-model flow map, carved into
/// the terrain. The smoothing pass fails below 512 pixels.
pub struct Anastomosis;

impl NodeBuilder for Anastomosis {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let size = context.resolution.x;
        let rain = context.f32("Impact") * 0.005;
        let width = context.f32("Width");
        let mut depth = context.f32("Depth") * 0.5;
        let iterations = match context.enum_value("Downcutting") {
            0 => {
                depth *= 0.35;
                3
            }
            1 => {
                depth *= 0.75;
                6
            }
            2 => 20,
            3 => 80,
            4 => 120,
            5 => 200,
            6 => 400,
            7 => 800,
            8 => 2000,
            _ => 6,
        };

        let reduced = context.enum_value("Quality") < 3;
        let working_size = migrated::relative_quality_size(context.enum_name("Quality"), size);
        let heights = if reduced {
            transformer::resize_with(&input, Resolution::square(working_size), ResizeMethod::HighQuality)
        } else {
            (*input).clone()
        };
        let mut flow = maths::autolevel(simulations::create_flow_map(&heights, iterations, rain, false));
        if reduced {
            flow = transformer::resize_with(&flow, context.resolution, ResizeMethod::HighQuality);
        }
        maths::strengthen(&mut flow, width);
        if context.bool("SmoothEdges") {
            let flow_width = flow.width() as i32;
            for _ in 0..2 {
                let smoothed = others::smoothed_median_blur(&flow, 2, 2, flow_width);
                flow = smoothed.ok_or_else(|| NodeError::Failed("Index was outside the bounds of the array.".to_string()))?;
            }
        }

        context.render_secondary_as(RenderIntent::Mask);
        context.commit_port("Flow", flow.clone(), RenderIntent::Mask);
        let carved = combiner::subtract(&input, &flow, depth * 0.05);
        context.commit(carved)
    }
}

/// HydroFix: fills pits and carves drainage so every cell flows off the
/// map (the native `HydrologicalCorrection`).
pub struct HydroFix;

impl NodeBuilder for HydroFix {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let corrected = hydro::hydrological_correction(input.data(), input.width(), context.f32("Downcutting"));
        context.commit(Map::from_data(input.resolution(), corrected))
    }
}

/// Rivers: river networks traced down the terrain from headwaters and
/// carved in (the native `Rivers`).
pub struct Rivers;

impl NodeBuilder for Rivers {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_secondary_as(RenderIntent::Mask);
        let input = context.require_input("In")?;
        let headwaters = context.input("Headwaters");
        // River Valley Width: pyramid levels added or taken away.
        const OCTAVE_CORRECTIONS: [i32; 5] = [-4, -2, 0, 2, 4];
        let valley_width = context.enum_value("RiverValleyWidth").clamp(0, 4) as usize;
        let settings = hydro::RiverSettings {
            seed: context.i32("Seed"),
            width: context.f32("Width"),
            depth: context.f32("Depth"),
            headwaters: context.i32("Headwaters"),
            water: context.f32("Water"),
            render_surface: context.bool("RenderSurface"),
            downcutting: context.f32("Downcutting"),
            octave_correction: OCTAVE_CORRECTIONS[valley_width],
        };
        let size = input.width();
        let output = hydro::rivers(&settings, input.data(), size, headwaters.as_ref().map(|map| map.data()));
        let resolution = input.resolution();
        let direction: Vec<Vec<f32>> = (0..3).map(|channel| output.direction.iter().skip(channel).step_by(3).copied().collect()).collect();
        context.commit(Map::from_data(resolution, output.heights))?;
        context.commit_port("Rivers", Map::from_data(resolution, output.rivers), RenderIntent::Default);
        context.commit_port("Depth", Map::from_data(resolution, output.depth), RenderIntent::Default);
        context.commit_port("Surface", Map::from_data(resolution, output.surface), RenderIntent::Heightfield);
        context.commit_port("Direction", Map::from_channels(resolution, direction), RenderIntent::Color);
        Ok(())
    }

    /// `Rivers.GetCustomColorMap`: the levelled river depth, grey banks to
    /// light blue water.
    fn custom_colour_map(&self, context: &BuildContext) -> Option<Map> {
        let depth = context.own_output("Depth")?;
        let levelled_depth = maths::autolevel((*depth).clone());
        Some(colour::colorize(&levelled_depth, &RIVER_WATER))
    }
}

/// Erosion: the CE engine's classic droplet erosion (`Erosions.Classic`).
pub struct Erosion;

impl NodeBuilder for Erosion {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let area = context.input("AreaMask");
        let removal = context.input("SedimentRemoval");
        let settings = ClassicErosionSettings {
            duration: context.f32("Duration"),
            rock_softness: context.f32("RockSoftness"),
            strength: context.f32("Strength"),
            downcutting: context.f32("Downcutting"),
            inhibition: context.f32("Inhibition"),
            base_level: context.f32("BaseLevel"),
            feature_scale: context.i32("FeatureScale"),
            terrain_scale: context.i32("TerrainScale"),
            verticality: context.i32("Verticality"),
            debris: context.f32("Debris"),
            volume: context.f32("Volume"),
            sediment_removal: context.f32("SedimentRemoval"),
            area_effect: context.enum_value("AreaEffect") as i32,
            seed: context.i32("Seed"),
            aggressive: context.bool("AggressiveMode"),
        };
        let [height, wear, deposits, flow] = erosions::classic(&input, &settings, area.as_deref(), removal.as_deref());
        context.commit(height)?;
        context.commit_port("Wear", wear, RenderIntent::Default);
        context.commit_port("Deposits", deposits, RenderIntent::Default);
        context.commit_port("Flow", flow, RenderIntent::Default);
        Ok(())
    }
}

/// `Wizard.GetRecipeStandard`: one of four values for a `Generic4` level.
fn generic4(context: &BuildContext, name: &str, values: [f32; 4]) -> f32 {
    let level = context.enum_value(name).clamp(0, 3) as usize;
    values[level]
}

/// Wizard: classic erosion from a handful of four-step choices. The
/// Material (randomness) recipe is computed but never passed on, and the
/// wear and flow maps go to each other's ports.
pub struct Wizard;

impl NodeBuilder for Wizard {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let mut map = (*context.require_input("In")?).clone();
        let bulk = match context.enum_name("Bulk") {
            "Med" => 0.25,
            "High" => 0.6,
            "Ultra" => 0.75,
            _ => 0.0,
        };
        if bulk as f64 > 0.0 {
            maths::strengthen(&mut map, bulk);
        }
        let settings = ClassicErosionSettings {
            duration: generic4(context, "Strength", [0.04, 0.1, 0.2, 0.35]),
            rock_softness: generic4(context, "Density", [0.1, 0.35, 0.6, 1.0]),
            strength: 0.5,
            downcutting: generic4(context, "ChannelDepth", [0.0, 0.1, 0.4, 0.9]),
            inhibition: generic4(context, "Deposits", [0.0, 0.33, 0.75, 1.0]),
            base_level: generic4(context, "ChannelWidth", [0.0, 1.0, 2.0, 5.0]),
            feature_scale: 1200,
            terrain_scale: context.terrain_width() as i32,
            verticality: context.terrain_height() as i32,
            debris: 0.0,
            volume: 0.0,
            sediment_removal: generic4(context, "Removal", [0.0, 0.15, 0.4, 1.0]),
            area_effect: 3,
            seed: context.i32("Seed"),
            aggressive: true,
        };
        let [height, wear, deposits, flow] = erosions::classic(&map, &settings, None, None);
        context.commit(height)?;
        context.commit_port("Flow", wear, RenderIntent::Default);
        context.commit_port("Deposits", deposits, RenderIntent::Default);
        context.commit_port("Wear", flow, RenderIntent::Default);
        Ok(())
    }
}

/// Thermal: the CE engine's talus erosion. The deposit and wear maps go
/// to each other's ports.
pub struct Thermal;

impl NodeBuilder for Thermal {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let area = context.input("AreaMask");
        let removal = context.input("SedimentRemoval");
        let settings = ThermalSettings {
            duration: context.f32("Duration"),
            strength: context.f32("Strength"),
            anisotropy: context.f32("Anisotropy"),
            angle: context.f32("Angle"),
            settling: context.f32("Settling"),
            sediment_removal: context.f32("SedimentRemoval"),
            feature_scale: context.i32("FeatureScale"),
            terrain_scale: context.i32("TerrainScale"),
            verticality: context.i32("Verticality"),
        };
        let [height, deposit, wear] = erosions::thermal(&input, &settings, area.as_deref(), removal.as_deref());
        context.commit(height)?;
        context.commit_port("Wear", deposit, RenderIntent::Default);
        context.commit_port("Deposits", wear, RenderIntent::Default);
        Ok(())
    }
}

/// Thermal2: the SYCL talus erosion (`ThermalSimulation`), simulated
/// coarse to fine over a Laplacian pyramid.
pub struct Thermal2;

impl NodeBuilder for Thermal2 {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.input("In").ok_or_else(|| NodeError::Failed("Data missing.".to_string()))?;
        let area = context.input("AreaMask");
        let removal = context.input("SedimentRemoval");
        let settings = thermal::ThermalSettings {
            duration: context.f32("Duration"),
            strength: context.f32("Strength"),
            anisotropy: context.f32("Anisotropy"),
            talus_angle: context.f32("Angle"),
            erosion_scale: context.f32("FeatureScale"),
            heightmap_size: context.terrain_width(),
            heightmap_height: context.terrain_height(),
            sediment_removal: context.f32("SedimentRemoval"),
        };
        let maps = thermal::simulate(&input, area.as_deref(), removal.as_deref(), &settings);
        context.commit(maps.heights)?;
        // The ports' own intents: Mask on a new node, Default on one read
        // from a project file.
        context.commit_port("Deposits", maps.deposition, RenderIntent::Default);
        context.commit_port("Wear", maps.wear, RenderIntent::Default);
        Ok(())
    }
}

/// Snow: the CS engine's snowfall, settling, melt and slip-off.
pub struct Snow;

impl NodeBuilder for Snow {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let snowfall_area = context.input("SnowMap");
        let mut melt_area = context.input("MeltMap").map(|map| (*map).clone());
        context.render_secondary_as(RenderIntent::Mask);
        if melt_area.is_none() && context.enum_name("MeltType") == "Directional" {
            // Melting follows the sunlight of mid-February to late May at
            // 65° latitude.
            let sun = lighting::SunlightSettings {
                day_begin: 36,
                day_end: 146,
                latitude: 65.0,
                north: context.f32("Direction"),
                air_density: 0.01,
            };
            let terrain = (context.terrain_width(), context.terrain_height());
            let light = lighting::sunlight_integral(&input, &sun, terrain);
            let equalized = histogram::equalize(light);
            let blurred = fast_blur::blur_relative(&equalized, 0.05);
            let mut melt = blurred.add_scalar(context.f32("MeltRemnants"));
            maths::clamp(&mut melt);
            melt_area = Some(melt);
        }
        let settings = erosions::SnowSettings {
            duration: context.f32("Duration"),
            intensity: context.f32("Intensity"),
            settle_thaw: context.f32("SettleThaw"),
            melt: context.f32("Melt"),
            snow_line: context.f32("SnowLine"),
            real_scale: context.bool("RealScale"),
            scale: context.i32("TerrainScale") as f32,
            verticality: context.i32("Verticality") as f32,
            slip_off: context.f32("SlipOffAngle"),
            adhered_mass: context.f32("AdheredSnowMass"),
            model_scale: context.f32("ModelScale"),
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let [height, depth, hard, soft] = erosions::snow(&input, &settings, snowfall_area.as_deref(), melt_area.as_ref(), terrain);
        context.commit(height)?;
        context.commit_port("Depth", depth, RenderIntent::Default);
        context.commit_port("Hard", hard, RenderIntent::Default);
        context.commit_port("Snow", soft, RenderIntent::Default);
        Ok(())
    }

    fn custom_colour_map(&self, context: &BuildContext) -> Option<Map> {
        snow_colours(context)
    }
}

/// `GetCustomColorMap` of Snow and Snowfield: the snow mask squeezed into
/// 0.6–1, light grey ground up to white snow.
fn snow_colours(context: &BuildContext) -> Option<Map> {
    let snow = context.own_output("Snow")?;
    Some(maths::fit_range(&snow, 0.6, 1.0))
}

/// The slope a Snowfield (or Crumble) tilts the terrain by before
/// simulating: a linear gradient towards `Direction`, or none for X.
fn direction_tilt(context: &BuildContext) -> Option<Map> {
    let azimuth = match context.enum_name("Direction") {
        "E" => 90.0,
        "S" => 180.0,
        "W" => 270.0,
        "X" => return None,
        _ => 0.0,
    };
    let size = context.resolution.x;
    let half = size as f32 / 2.0;
    Some(gradients::linear(size, half, half, size as f32, azimuth, EdgeBehaviour::Clip))
}

/// Snowfield: several snowfall cascades above rising snow lines, each
/// warped and randomized, then built up, optionally eroded into flows and
/// textured.
pub struct Snowfield;

impl NodeBuilder for Snowfield {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        context.render_secondary_as(RenderIntent::Mask);
        let resolution = context.resolution;
        let terrain = (context.terrain_width(), context.terrain_height());
        let seed = context.i32("Seed");
        let tilt = direction_tilt(context);
        let mut snowy = match &tilt {
            Some(tilt) => input.add(tilt),
            None => (*input).clone(),
        };
        let mut hard = Map::new(resolution, 1);
        let mut soft = Map::new(resolution, 1);

        let mut random = XorShift64Star::new(seed);
        let mut randomize = |value: f32, range: f32| random.next_float_range(value - value * range, value + value * range);
        let cascades = context.i32("Cascades");
        let snow_line = context.f32("SnowLine");
        let span = if context.bool("AlternateSnowfall") { snow_line } else { 1.0 - snow_line };
        let step = span / cascades as f32;
        let taper = if context.bool("TaperSnowfall") { 0.2 * context.f32("TaperAmount") } else { 0.0 };
        let intensity = context.f32("Intensity") * 0.5;
        for cascade in 0..cascades {
            context.report_progress(cascade as f32 / cascades as f32);
            context.report_progress_text(format!("Simulating cascade {cascade:02} of {cascades:02}"));
            let line = snow_line + step * cascade as f32;
            let threshold = randomize(line, 0.25);
            let above = aspect::threshold(&input, threshold);
            let warp = FractalWarpSettings {
                seed: seed + cascade,
                ..FractalWarpSettings::default()
            };
            let warped = warps::fractal_warp(&above, &warp, None, terrain.0, terrain.1);
            let snowfall_area = fast_blur::blur_relative(&warped, 8.0);
            let cascade_taper = taper * (cascades as f32 / (cascades as f32 - cascade as f32));
            let settings = erosions::SnowSettings {
                duration: randomize(context.f32("Duration"), 0.25),
                intensity: randomize(intensity - cascade_taper, 0.25),
                settle_thaw: randomize(context.f32("SettleThaw"), 0.25),
                melt: randomize(context.f32("Melt"), 0.25),
                snow_line: 0.0,
                real_scale: true,
                scale: 5000.0,
                verticality: 2500.0,
                slip_off: randomize(context.f32("SlipOffAngle"), 0.25),
                adhered_mass: randomize(context.f32("AdheredSnowMass"), 0.25),
                model_scale: randomize(5.0, 1.0),
            };
            let [height, _, cascade_hard, cascade_soft] = erosions::snow(&snowy, &settings, Some(&snowfall_area), None, terrain);
            hard = combiner::max(&cascade_hard, &hard, 1.0);
            soft = combiner::max(&cascade_soft, &soft, 1.0);
            snowy = height;
        }
        if let Some(tilt) = &tilt {
            snowy = snowy.subtract(tilt);
        }

        let buildup = snowy.subtract(&input);
        let mut buildup_mask = maths::autolevel(buildup);
        maths::strengthen(&mut buildup_mask, 1.0);
        let mut rounded = profiles::recurve(&snowy, 1.0, 3, false, false, 0.1);
        if context.bool("SharpBuildup") {
            rounded = profiles::recurve(&rounded, 1.0, 3, false, false, 0.1);
        }
        snowy = masking::mask(&snowy, &rounded, &buildup_mask);
        if context.bool("Flows") {
            let flows_depth = context.f32("FlowsDepth");
            let flows = ClassicErosionSettings {
                duration: 0.04,
                rock_softness: 0.1,
                strength: flows_depth,
                downcutting: 0.2 * flows_depth,
                inhibition: 0.0,
                base_level: 0.0,
                feature_scale: 15,
                terrain_scale: 10000,
                verticality: 2000,
                area_effect: 2,
                seed,
                ..ClassicErosionSettings::default()
            };
            let [eroded, _, _, _] = erosions::classic(&snowy, &flows, Some(&buildup_mask), None);
            let bands = profiles::graphic_eq(&snowy, [0.0, 2.0, 2.0, 2.0, 0.0, 0.0, 0.0]);
            let mut flow_mask = maths::autolevel(bands);
            maths::strengthen(&mut flow_mask, -0.28);
            let flow_mask = histogram::equalize(flow_mask);
            snowy = masking::mask(&snowy, &eroded, &flow_mask);
        }
        let surface_details = context.f32("SurfaceDetails");
        if surface_details > 0.0 {
            snowy = migrated::ground_texture(&snowy, migrated::GroundStyle::Harsh, surface_details, 0.33, 0.0, terrain.1);
        }
        let added = snowy.subtract(&input);
        snowy = input.add(&added);
        snowy = combiner::max(&snowy, &input, 1.0);
        let depth = snowy.subtract(&input);
        context.commit(snowy)?;
        context.commit_port("Depth", depth, RenderIntent::Default);
        context.commit_port("Hard", hard, RenderIntent::Default);
        context.commit_port("Snow", soft, RenderIntent::Default);
        Ok(())
    }

    fn custom_colour_map(&self, context: &BuildContext) -> Option<Map> {
        snow_colours(context)
    }
}

/// Sea: water below a level, with shaped beaches and cliffs.
pub struct Sea;

impl NodeBuilder for Sea {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_secondary_as(RenderIntent::Mask);
        let input = context.require_input("In")?;
        let edge = context.input("Edge");
        let arrangement = if context.enum_name("SeaArrangement") == "Surrounding" { SeaArrangement::Surrounding } else { SeaArrangement::Global };
        let settings = SeaSettings {
            arrangement,
            level: context.f32("Level"),
            coastal_erosion: context.bool("CoastalErosion"),
            shore_size: context.f32("ShoreSize"),
            shore_height: context.f32("ShoreHeight"),
            variation: context.f32("Variation"),
            uniform_variations: context.bool("UniformVariations"),
            extra_cliff_details: context.bool("ExtraCliffDetails"),
            render_surface: context.bool("RenderSurface"),
        };
        let maps = waters::sea(&input, edge.as_deref(), &settings);
        context.commit(maps.height)?;
        context.commit_port("Water", maps.water, RenderIntent::Default);
        context.commit_port("Depth", maps.depth, RenderIntent::Default);
        context.commit_port("Shore", maps.shore, RenderIntent::Default);
        context.commit_port("Surface", maps.surface, RenderIntent::Heightfield);
        Ok(())
    }

    /// `Sea.GetCustomColorMap`: turquoise shallows to deep teal by the
    /// levelled depth, with sun glints, over sandy shores; plain grey when
    /// there is no water.
    fn custom_colour_map(&self, context: &BuildContext) -> Option<Map> {
        let water = context.own_output("Water")?;
        if is_dry(&water) {
            return Some(Map::filled(context.resolution, 0.5));
        }
        let depth = context.own_output("Depth")?;
        let mut levelled_depth = maths::autolevel((*depth).clone());
        maths::strengthen(&mut levelled_depth, 0.5);
        let shore = context.own_output("Shore")?;
        let ground = colour::colorize(&shore, &SHORE_SAND);
        let sea = colour::colorize(&levelled_depth, &SEA_WATER);
        let glints = water_glints(levelled_depth.width(), context);
        let glinting_sea = sea.add(&glints);
        Some(masking::mask(&ground, &glinting_sea, &water))
    }
}

/// Lake: water filling the terrain's basins. The Advanced type runs the
/// CW engine's rain simulation; the Simple type floods from a point (the
/// native `Lake_Create`).
pub struct Lake;

impl Lake {
    /// The Simple type: the native `Lake_Create` from the point at X, Y.
    fn build_simple(context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let resolution = context.resolution;
        let across = (context.f32("X") * 0.5 + 0.5) * (resolution.x as f32 - 0.01);
        let down = (context.f32("Y") * 0.5 + 0.5) * (resolution.y as f32 - 0.01);
        let x = x86::cvttss2si(across);
        let y = x86::cvttss2si(down);
        // A point outside the map (X and Y beyond ±1 by a binding) fails.
        let inside = (0..resolution.x as i32).contains(&x) && (0..resolution.y as i32).contains(&y);
        if !inside {
            return Err(NodeError::Failed("The lake's point lies outside the terrain.".to_string()));
        }
        let settings = LakeSettings {
            x,
            y,
            level: context.f32("WaterLevel"),
            render_surface: context.bool("RenderSurface"),
        };
        let maps = lake::create(&input, &settings);
        context.commit(maps.heights)?;
        context.commit_port("Depth", maps.depth, RenderIntent::Default);
        context.commit_port("Shore", Map::new(resolution, 1), RenderIntent::Default);
        context.commit_port("Surface", maps.surface, RenderIntent::Default);
        context.commit_port("Water", maps.water, RenderIntent::Heightfield);
        Ok(())
    }
}

impl NodeBuilder for Lake {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_secondary_as(RenderIntent::Mask);
        if context.enum_name("Type") != "Advanced" {
            return Lake::build_simple(context);
        }
        let input = context.require_input("In")?;
        let precipitation = context.input("Precipitation");
        let settings = LakesSettings {
            precipitation: context.f32("Precipitation"),
            small_lakes: context.f32("SmallLakes"),
            flood_control: context.bool("FloodControl"),
            water_floor: context.f32("WaterFloor"),
            shore_size: context.f32("ShoreSize"),
            altitude_bias: context.f32("AltitudeBias"),
            size_bias: context.f32("SizeBias"),
            render_surface: context.bool("RenderSurface"),
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let maps = waters::lakes(&input, precipitation.as_deref(), &settings, terrain);
        context.commit(maps.height)?;
        context.commit_port("Depth", maps.depth, RenderIntent::Default);
        context.commit_port("Shore", maps.shore, RenderIntent::Default);
        context.commit_port("Surface", maps.surface, RenderIntent::Default);
        context.commit_port("Water", maps.water, RenderIntent::Heightfield);
        Ok(())
    }

    /// `Lake.GetCustomColorMap`: the water light to dark blue by depth,
    /// with sun glints, over sandy shores (Advanced, plain grey when it
    /// holds no water) or neutral grey ground (Simple, depth strengthened).
    fn custom_colour_map(&self, context: &BuildContext) -> Option<Map> {
        let water = context.own_output("Water")?;
        if context.enum_name("Type") == "Advanced" {
            if is_dry(&water) {
                return Some(Map::filled(context.resolution, 0.5));
            }
            let depth = context.own_output("Depth")?;
            let shore = context.own_output("Shore")?;
            let ground = colour::colorize(&shore, &SHORE_SAND);
            let lake = colour::colorize(&depth, &LAKE_WATER);
            let glints = water_glints(depth.width(), context);
            let glinting_lake = lake.add(&glints);
            return Some(masking::mask(&ground, &glinting_lake, &water));
        }
        let depth = context.own_output("Depth")?;
        let mut strong_depth = (*depth).clone();
        maths::strengthen(&mut strong_depth, 1.0);
        let ground = Map::filled(context.resolution, 0.5);
        let lake = colour::colorize(&strong_depth, &LAKE_WATER);
        let glints = water_glints(strong_depth.width(), context);
        let glinting_lake = lake.add(&glints);
        Some(masking::mask(&ground, &glinting_lake, &water))
    }
}

const LIGHT_BLUE: [f32; 3] = [0.271, 0.749, 1.0];
const DARK_BLUE: [f32; 3] = [0.171, 0.349, 0.6];

/// Lake water: light blue in the shallows, dark blue from a sixth of the
/// depth down.
const LAKE_WATER: [[f32; 3]; 7] = [LIGHT_BLUE, DARK_BLUE, DARK_BLUE, DARK_BLUE, DARK_BLUE, DARK_BLUE, DARK_BLUE];

/// Sea water: (120, 195, 214) in the shallows to (22, 65, 74) at depth.
const SEA_WATER: [[f32; 3]; 2] = [[120.0 / 255.0, 195.0 / 255.0, 214.0 / 255.0], [22.0 / 255.0, 65.0 / 255.0, 74.0 / 255.0]];

/// River depth: grey banks to light blue.
const RIVER_WATER: [[f32; 3]; 2] = [[0.45, 0.45, 0.45], LIGHT_BLUE];

/// Lake and Sea shores: grey ground to sand.
const SHORE_SAND: [[f32; 3]; 2] = [[0.45, 0.45, 0.45], [0.871, 0.749, 0.63]];

/// Lake's and Sea's test for no water at all (`Data.All(x => x < 0.001f)`).
fn is_dry(water: &Map) -> bool {
    water.data().iter().all(|&value| value < 0.001)
}

/// The sun glints Lake and Sea add to their water
/// (`Noises.WaveShine(resolution, 0, 4, 0, 1f, 0.025f, 0.5f, 0f, 0.1f, 0.1f)`).
fn water_glints(size: usize, context: &BuildContext) -> Map {
    let waves = WaveShineSettings {
        size: 0.025,
        wind_patch_intensity: 0.1,
        ..WaveShineSettings::default()
    };
    let terrain = (context.terrain_width(), context.terrain_height());
    noise::wave_shine(size, &waves, terrain)
}

/// Crumble: rock wear where snow would gather and melt, optionally tilted
/// towards a direction.
pub struct Crumble;

impl NodeBuilder for Crumble {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let area = context.input("AreaMask");
        let tilt = direction_tilt(context);
        let tilted = match &tilt {
            Some(tilt) => input.add(tilt),
            None => (*input).clone(),
        };
        let settings = erosions::CrumbleSettings {
            duration: context.f32("Duration"),
            strength: context.f32("Strength"),
            coverage: context.f32("Coverage"),
            horizontal: context.f32("Horizontal"),
            vertical: context.f32("Vertical"),
            rock_hardness: context.f32("RockHardness"),
            edge: context.f32("Edge"),
            downcutting: context.f32("Downcutting"),
            depth: context.f32("Depth"),
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let [mut worn, wear] = erosions::crumble(&tilted, area.as_deref(), &settings, terrain);
        if let Some(tilt) = &tilt {
            worn = worn.subtract(tilt);
        }
        context.commit(worn)?;
        context.commit_port("Wear", wear, RenderIntent::Default);
        Ok(())
    }
}

/// IceFloe: plates of ice floating on water, as Cellular3D cells whose
/// surface follows the water depth.
pub struct IceFloe;

impl NodeBuilder for IceFloe {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_secondary_as(RenderIntent::Mask);
        let input = context.require_input("In")?;
        let Some(water_depth) = context.input("WaterDepth") else {
            return Err(NodeError::Validation("Required port [Guide] not connected.".to_string()));
        };
        let colour = context.input("Color");

        // The depth, levelled, drives both the cells' surface (deep water
        // low unless reversed) and their gap.
        let mut gap_guide = maths::autolevel((*water_depth).clone());
        let mut surface_guide = gap_guide.clone();
        if !context.bool("Reverse") {
            maths::invert(&mut surface_guide);
        }
        if context.bool("Reverse2") {
            maths::invert(&mut gap_guide);
        }
        maths::strengthen(&mut surface_guide, context.f32("Shaper1"));
        maths::strengthen(&mut gap_guide, context.f32("Shaper2"));

        let size = context.f32("Size") * 0.1;
        let settings = Cellular3dSettings {
            cells: 1.0 / size,
            gap: context.f32("Density"),
            scale: [1.0, 1.0, 10.0 * context.f32("Layers")],
            jitter: [1.0, 1.0, 1.0],
            seed: context.i32("Seed") as u32,
            terrain_width: context.terrain_width(),
            terrain_height: context.terrain_height(),
        };
        let (cells, _) = cellular3d::render(&surface_guide, Some(&gap_guide), &settings);
        let rounded = profiles::recurve(&cells, 0.5, 3, true, false, 0.1);
        let water = aspect::threshold(&water_depth, 0.0001);
        let mut ice = rounded.multiply(&water);

        // The ice is levelled in place for the colour map, so a connected
        // colour also changes the ice's height.
        let colour_map = match colour {
            Some(colour) => {
                ice = maths::autolevel(ice);
                combiner::mix(combiner::BlendMode::Screen, 1.0, &colour, &ice)
            }
            None => Map::new(context.resolution, 3),
        };
        context.commit_port("ColorMap", colour_map, RenderIntent::Default);

        let ice = ice.multiply_scalar(context.f32("Thickness") * 0.005);
        let floating = input.add(&ice);
        context.commit(floating)?;
        context.commit_port("Ice", ice, RenderIntent::Default);
        Ok(())
    }
}

/// Dusting: a light dusting of snow (or dust) on gentle, high, sun-facing
/// ground, drifted by flow and spread by colour transport.
pub struct Dusting;

impl NodeBuilder for Dusting {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let texture = context.input("Texture");
        let seed = context.i32("Seed");
        let (terrain_width, terrain_height) = (context.terrain_width(), context.terrain_height());

        let mut heights = (*input).clone();
        if context.bool("Gritty") {
            let rocky = surfacer::rocky(&heights, 0.0001, 0.0, terrain_height);
            heights = combiner::screen(&heights, &rocky, 0.00036);
        }
        let melt = context.f32("Melt");
        let coverage = context.float2("SlopeCoverage");
        let slope_low = coverage[0] * 60.0;
        let slope_high = coverage[1] * 60.0;
        let slope = aspect::slope(&heights, slope_low, 30.0, slope_high - slope_high * melt, terrain_width, terrain_height);
        let altitude = aspect::height_normalized(&heights, context.f32("Snowline"), 1.0, context.f32("Falloff"));
        let facing = if context.bool("Directional") {
            let angle = aspect::angle(&heights, context.f32("Direction") * 360.0, melt, 1.0, context.f32("Coverage"));
            fast_blur::blur_relative(&angle, 0.3)
        } else {
            Map::filled(context.resolution, 0.5)
        };
        let flow_settings = FlowSettings {
            flow_length: 1.0,
            flow_volume: 1.0,
            seed,
        };
        let flow = flow::simulate(&heights, None, &flow_settings);
        let mut facing = combiner::add(&facing, &flow, 1.0);
        maths::clamp(&mut facing);

        let strength = context.f32("Strength");
        let peaks = context.f32("Peaks");
        let exposed = slope.multiply(&altitude).multiply(&facing);
        let deposit = if peaks > 0.0 {
            let bands = profiles::graphic_eq(&heights, [0.0, 2.0, 2.0, 2.0, 2.0, 1.0, 0.0]);
            let mut summits = maths::autolevel(bands);
            maths::strengthen(&mut summits, -1.0 + peaks);
            let summits = maths::autolevel(summits);
            exposed.add(&summits).multiply_scalar(strength)
        } else {
            exposed.multiply_scalar(strength)
        };

        let transport = ColourTransportSettings {
            transport_distance: context.f32("Flow") * 0.25,
            sediment_density: 1.0,
            blend: 0.5,
            flow_volume: 0.02,
            colour_hold: 0.15,
            laminar_flow: true,
            seed,
        };
        let carried = colour_transport::simulate(&heights, &deposit, None, None, &transport);
        let noise = random_noises::perlin(context.resolution, 1.0);
        let mut dust = combiner::mix(combiner::BlendMode::Divide2, 1.0, &carried, &noise);
        maths::clamp(&mut dust);
        maths::strengthen(&mut dust, 0.0 - (0.3 + melt * (1.0 - 0.3)));
        let artistic = context.f32("ArtisticControl");
        if artistic > 0.0 {
            let steep = aspect::slope(&heights, 0.0, 90.0 * artistic, 0.0, terrain_width, terrain_height);
            dust = dust.subtract(&steep);
            maths::clamp(&mut dust);
        }

        match texture {
            Some(texture) => {
                context.render_as(RenderIntent::Color);
                let dusted = combiner::max(&texture, &dust, 1.0);
                context.commit(dusted)
            }
            None => {
                let source = context.source_type("In").unwrap_or_default();
                let from_snow = matches!(source, "Snow" | "Snowfield") || source.ends_with(".Snow") || source.ends_with(".Snowfield");
                if context.bool("IncludeExistingSnow") && from_snow
                    && let Some(snow) = context.source_output("In", "Snow") {
                        dust = dust.add(&snow);
                    }
                context.render_as(RenderIntent::Mask);
                context.commit(dust)
            }
        }
    }
}
