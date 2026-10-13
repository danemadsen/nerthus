//! The erosion recipes (`Erosions`) built on the native droplet erosion.

use nerthus_engine::Map;
use nerthus_erosion::{ce, cs};

use crate::aspect;
use crate::combiner;
use crate::fast_blur;
use crate::landscapes::{self, Abc, RidgeSettings, TerrainSize};
use crate::lighting;
use crate::maths;
use crate::morph::{self, DistanceRtMode, DistanceRtSettings};
use crate::noise::{self, NoisesPerlin};
use crate::parallel_erosion::{self, ErosionMaps, ErosionSettings};
use crate::profiles;
use crate::random_noises;
use crate::sat_blur;
use crate::texturize;

fn erode(map: &Map, settings: ErosionSettings, terrain: TerrainSize) -> ErosionMaps {
    parallel_erosion::erode(map, None, &settings, terrain)
}

/// `low + t × (high − low)`.
fn lerp(t: f32, low: f32, high: f32) -> f32 {
    low + t * (high - low)
}

/// `Erosions.Hillify`: hills where a band-limited copy of the map rises
/// above it, smoothed by an EQ and optionally eroded. Returns the map and
/// the hill mask.
pub fn hillify(map: &Map, coverage: f32, aggressive: bool, eroded_surface: bool, seed: i32, terrain: TerrainSize) -> (Map, Map) {
    let original = map.clone();
    let equalized = if aggressive {
        let band = 1.1 - coverage;
        profiles::graphic_eq(map, [band, band * 0.5, 1.0, 0.0, 0.0, 0.0, 0.0])
    } else {
        let band = 1.0 - lerp(coverage, 0.3, 1.0);
        profiles::graphic_eq(map, [1.0, band, 1.0, 0.0, 0.0, 0.0, 0.0])
    };
    let mut hills = combiner::min(map, &equalized, 1.0);
    let raised = maths::fit_range(&original.subtract(&hills), 0.0, 1.0);
    let mask = aspect::threshold(&raised, 1e-6);
    if eroded_surface {
        let settings = ErosionSettings {
            duration: 35.855385,
            downcutting: 0.0,
            suspended_load_amount: 0.0,
            suspended_load_angle: 45.0,
            bed_load_amount: 1.0,
            bed_load_angle: 45.0,
            coarse_sediments_amount: 0.62170565,
            coarse_sediments_angle: 30.896067,
            seed,
            ..ErosionSettings::default()
        };
        let worn = erode(&hills, settings, terrain).height;
        hills = combiner::min(&hills, &worn, 1.0);
    }
    (hills, mask)
}

/// Settings of [`lichtenberg`].
#[derive(Clone, Copy, Debug)]
pub struct LichtenbergSettings {
    pub style: Abc,
    pub scale: f32,
    pub complexity: i32,
    pub depth: f32,
    pub expand: f32,
    pub bias: f32,
    pub seed: i32,
}

/// `Erosions.Lichtenberg`: the flow map of an eroded noise, carved into
/// the map as branching channels.
pub fn lichtenberg(map: &Map, settings: &LichtenbergSettings, terrain: TerrainSize) -> Map {
    let size = map.width();
    let seed = settings.seed;
    let scale = settings.scale;
    let source = match settings.style {
        Abc::A => {
            let parameters = NoisesPerlin {
                scale: lerp(scale, 0.6, 0.99),
                octaves: 7 - settings.complexity,
                gain: 0.5,
                seed,
                warp_type: 0,
                ..NoisesPerlin::default()
            };
            noise::noises_perlin(size, &parameters)
        }
        Abc::B => {
            let ridges = RidgeSettings {
                scale: lerp(scale, 0.8, 0.99),
                height: 0.6,
                definition: 0.4,
                seed,
                scale_x: 1.0,
                scale_y: 1.0,
            };
            landscapes::ridge(size, &ridges, terrain)
        }
        Abc::C => landscapes::slump(size, lerp(scale, 0.4, 0.99), Abc::A, Abc::A, seed, terrain),
    };
    let erosion = ErosionSettings {
        duration: settings.complexity as f32 / 6.0 * 180.0,
        erosion_scale: lerp(scale, 800.0, 2000.0),
        shape_detail_scale: 0.0,
        seed,
        ..ErosionSettings::default()
    };
    let mut channels = maths::autolevel(erode(&source, erosion, terrain).flow);
    if settings.bias != 0.0 {
        maths::strengthen(&mut channels, settings.bias);
    }
    let carved = channels.multiply_scalar(settings.depth * 0.5);
    let mut result = map.subtract(&carved);
    if settings.expand > 0.0 {
        let distance = DistanceRtSettings {
            mode: DistanceRtMode::Pyramid,
            directions: 32,
            skew: 0.0,
            angle: 0.0,
            angular_jitter: 0.0,
            falloff: lerp(settings.expand, 0.0, 0.3),
            falloff_jitter: 0.0,
            invert_input: false,
            invert_output: false,
            seed,
        };
        result = morph::distance_rt(&result, &distance, terrain);
    }
    result
}

/// What the sediment recipes return.
pub struct SedimentMaps {
    pub result: Map,
    pub sediments: Map,
    pub deposits: Map,
}

/// `Erosions.Sediments` ("Bulky"): erosion passes that only deposit, kept
/// where they raise the map.
pub fn sediments(map: &Map, passes: i32, scale: f32, seed: i32, terrain: TerrainSize) -> SedimentMaps {
    let first = ErosionSettings {
        duration: 100.0 * scale,
        downcutting: 0.0,
        erosion_scale: 2000.0,
        coarse_sediments_amount: 1.0,
        shape: 0.0,
        shape_detail_scale: 0.1,
        seed,
        ..ErosionSettings::default()
    };
    let eroded = erode(map, first, terrain);
    let mut raised = eroded.height;
    let mut deposits = eroded.deposition;
    for pass in 1..passes.max(1) {
        let settings = ErosionSettings {
            duration: 200.0 * scale,
            bed_load_amount: 0.2,
            shape_detail_scale: 0.0,
            seed: seed + pass,
            ..first
        };
        let base = combiner::max(map, &raised, 1.0);
        let eroded = erode(&base, settings, terrain);
        raised = eroded.height;
        deposits = combiner::max(&deposits, &eroded.deposition, 1.0);
    }
    raised = combiner::max(map, &raised, 1.0);
    let highest = combiner::max(map, &raised, 1.0);
    let sediments = maths::autolevel(combiner::subtract(&highest, map, 1.0));
    let deposits = deposits.multiply(&sediments);
    SedimentMaps {
        result: raised,
        sediments,
        deposits,
    }
}

/// `Erosions.SedimentsOld` ("Classic"): valleys filled by an inverted EQ
/// (style A) or a soil-driven disk blur (style B), optionally grainy.
pub fn sediments_classic(map: &Map, style_b: bool, scale: f32, grainy: bool) -> SedimentMaps {
    let mut filled = if !style_b {
        let mut inverted = map.clone();
        maths::invert(&mut inverted);
        let equalized = profiles::graphic_eq(&inverted, [1.0, 1.0 - scale, 1.0, 0.0, 0.0, 0.0, 0.0]);
        let mut lowest = combiner::min(&inverted, &equalized, 1.0);
        maths::invert(&mut lowest);
        maths::drop_to_zero(&lowest)
    } else {
        let soil = texturize::soil2(map, 0.25);
        let size = map.width();
        let blurred = sat_blur::disk_blur(map.data(), soil.data(), size, scale * 0.1 * size as f32, 24);
        Map::from_data(map.resolution(), blurred)
    };
    if grainy {
        let grain = random_noises::perlin(map.resolution(), 0.005);
        let rise = fast_blur::blur_relative(&filled.subtract(map), 0.5);
        let rise = maths::autolevel(rise);
        filled = filled.add(&grain.multiply(&rise));
    }
    // The difference is levelled in place before it is copied, so both
    // outputs are levelled.
    let difference = maths::autolevel(filled.subtract(map));
    SedimentMaps {
        result: filled,
        sediments: difference.clone(),
        deposits: difference,
    }
}

/// Settings of [`classic`] (`Erosions.Classic`), with its defaults.
#[derive(Clone, Copy, Debug)]
pub struct ClassicErosionSettings {
    pub duration: f32,
    pub rock_softness: f32,
    pub strength: f32,
    pub downcutting: f32,
    pub inhibition: f32,
    pub base_level: f32,
    pub feature_scale: i32,
    pub terrain_scale: i32,
    pub verticality: i32,
    pub debris: f32,
    pub volume: f32,
    pub sediment_removal: f32,
    /// `AreaEffect`: 0 erosion strength, 1 rock softness, 2 precipitation,
    /// 3 none.
    pub area_effect: i32,
    pub seed: i32,
    pub aggressive: bool,
}

impl Default for ClassicErosionSettings {
    fn default() -> Self {
        Self {
            duration: 0.04,
            rock_softness: 0.65,
            strength: 0.5,
            downcutting: 0.1,
            inhibition: 0.5,
            base_level: 0.0,
            feature_scale: 2000,
            terrain_scale: 10000,
            verticality: 2000,
            debris: 0.0,
            volume: 0.0,
            sediment_removal: 0.0,
            area_effect: 3,
            seed: -1,
            aggressive: true,
        }
    }
}

/// A copy clamped to [0, 1] (NaN stays).
fn clamped_plane(data: &[f32]) -> Vec<f32> {
    data.iter()
        .map(|&value| value.clamp(0.0, 1.0))
        .collect()
}

/// `Erosions.Classic`: the CE engine's droplet erosion. The area mask (zeros
/// when absent) and the sediment-removal mask (`sediment_removal` when
/// absent) are clamped first. Returns height, wear, deposit and flow.
/// The bias settings are ignored here (the area mask is never null), and
/// `RealScale` too: terrain scale and verticality always apply.
pub fn classic(map: &Map, settings: &ClassicErosionSettings, area_mask: Option<&Map>, removal_mask: Option<&Map>) -> [Map; 4] {
    let resolution = map.resolution();
    let aggressive = if settings.aggressive { 3.0f32 } else { 1.0 };
    let engine = ce::ErosionSettings {
        seed: settings.seed,
        size: settings.terrain_scale as f64,
        scale: settings.feature_scale as f64,
        duration: settings.duration as f64,
        rock_softness: settings.rock_softness as f64,
        fluvial_erosion_strength: (settings.strength * aggressive) as f64,
        downcutting: (settings.downcutting * aggressive) as f64,
        base_level: settings.base_level as f64,
        retardation_of_downcutting: settings.inhibition as f64,
        enable_thermal: false,
        thermal_erosion_strength: 0.0,
        talus_angle: 30.0,
        thermal_size: 0.0,
        flow_volume: settings.volume as f64,
        mask_by_shader: settings.area_effect != 3,
        masking_mode: settings.area_effect + 1,
        mask_as_thermal_erosion_strength: false,
        sediment_removal: settings.sediment_removal as f64,
        laminar_flow: false,
        randomness: settings.debris as f64,
        thermal_smoothness: 0.0,
        talus_settling: 0.0,
        thermal_detail: false,
        debris_size: 0.0,
        height_range: settings.verticality as f64,
    };
    let mut area = match area_mask {
        Some(mask) => mask.clone(),
        None => Map::new(resolution, 1),
    };
    let mut removal = match removal_mask {
        Some(mask) => mask.clone(),
        None => Map::filled(resolution, settings.sediment_removal),
    };
    maths::clamp_if_out_of_range(&mut area);
    maths::clamp_if_out_of_range(&mut removal);
    let output = ce::erode(&engine, map.data(), Some(area.data()), Some(removal.data()));
    [
        Map::from_data(resolution, output.height),
        Map::from_data(resolution, output.wear),
        Map::from_data(resolution, output.deposit),
        Map::from_data(resolution, output.flow),
    ]
}

/// Settings of [`thermal`] (`Simulations.Thermal`).
#[derive(Clone, Copy, Debug)]
pub struct ThermalSettings {
    pub duration: f32,
    pub strength: f32,
    pub anisotropy: f32,
    pub angle: f32,
    pub settling: f32,
    pub sediment_removal: f32,
    pub feature_scale: i32,
    pub terrain_scale: i32,
    pub verticality: i32,
}

/// `Simulations.Thermal`: the CE engine's talus erosion (thermal model 1, the
/// area mask scaling its strength). Returns height, deposit and wear.
pub fn thermal(map: &Map, settings: &ThermalSettings, area_mask: Option<&Map>, removal_mask: Option<&Map>) -> [Map; 3] {
    let resolution = map.resolution();
    let engine = ce::ErosionSettings {
        seed: 0,
        size: settings.terrain_scale as f64,
        scale: settings.feature_scale as f64,
        duration: settings.duration as f64,
        rock_softness: 1.0,
        fluvial_erosion_strength: 0.0,
        downcutting: 0.0,
        base_level: 0.0,
        retardation_of_downcutting: 0.0,
        enable_thermal: true,
        thermal_erosion_strength: settings.strength as f64,
        talus_angle: settings.angle as f64,
        thermal_size: settings.feature_scale as f64,
        flow_volume: 0.0,
        mask_by_shader: true,
        masking_mode: 1,
        mask_as_thermal_erosion_strength: true,
        sediment_removal: settings.sediment_removal as f64,
        laminar_flow: false,
        randomness: 0.65,
        thermal_smoothness: settings.anisotropy as f64,
        talus_settling: settings.settling as f64,
        thermal_detail: false,
        debris_size: 0.0,
        height_range: settings.verticality as f64,
    };
    let area = area_mask.map(|mask| clamped_plane(mask.data()));
    let removal = match removal_mask {
        Some(mask) => clamped_plane(mask.data()),
        None => vec![1.0; resolution.area()],
    };
    let output = ce::erode(&engine, map.data(), area.as_deref(), Some(&removal));
    [
        Map::from_data(resolution, output.height),
        Map::from_data(resolution, output.deposit),
        Map::from_data(resolution, output.wear),
    ]
}

/// Settings of [`snow`] (`Simulations.Snow`), with its defaults.
#[derive(Clone, Copy, Debug)]
pub struct SnowSettings {
    pub duration: f32,
    pub intensity: f32,
    pub settle_thaw: f32,
    pub melt: f32,
    pub snow_line: f32,
    /// Use the terrain's own size instead of `scale` and `verticality`.
    pub real_scale: bool,
    pub scale: f32,
    pub verticality: f32,
    pub slip_off: f32,
    pub adhered_mass: f32,
    pub model_scale: f32,
}

impl Default for SnowSettings {
    fn default() -> Self {
        Self {
            duration: 0.25,
            intensity: 0.25,
            settle_thaw: 0.25,
            melt: 0.0,
            snow_line: 0.0,
            real_scale: true,
            scale: 5000.0,
            verticality: 3200.0,
            slip_off: 38.0,
            adhered_mass: 12.0,
            model_scale: 5.0,
        }
    }
}

/// `Simulations.Snow`: the CS engine's snowfall (`Erosion1`). Returns the
/// terrain with snow, the snow depth, the hard mask and the soft mask.
pub fn snow(map: &Map, settings: &SnowSettings, snowfall_area: Option<&Map>, melt_area: Option<&Map>, terrain: TerrainSize) -> [Map; 4] {
    let resolution = map.resolution();
    let (size, height_range) = if settings.real_scale {
        (terrain.0 as i32, terrain.1 as i32)
    } else {
        (settings.scale as i32, settings.verticality as i32)
    };
    let engine = cs::SnowSettings {
        size: size as f64,
        scale: settings.model_scale as f64 * 4.0,
        snowfall_duration: settings.duration as f64,
        snowfall_intensity: settings.intensity as f64,
        post_snowfall_duration: settings.settle_thaw as f64,
        melting: settings.melt as f64,
        snow_line: (settings.snow_line * height_range as f32) as f64,
        enable_thermal: true,
        talus_angle: settings.slip_off as f64,
        flow_volume: settings.adhered_mass as f64,
        height_range: height_range as f64,
        snow_output: 3,
    };
    let snowfall = snowfall_area.map(|map| map.data());
    let melt = melt_area.map(|map| map.data());
    let output = cs::simulate(&engine, map.data(), snowfall, melt);
    [
        Map::from_data(resolution, output.height),
        Map::from_data(resolution, output.snow),
        Map::from_data(resolution, output.hard),
        Map::from_data(resolution, output.soft),
    ]
}

/// Settings of [`crumble`] (`Simulations.Crumble`), with its defaults.
#[derive(Clone, Copy, Debug)]
pub struct CrumbleSettings {
    pub duration: f32,
    pub strength: f32,
    pub coverage: f32,
    pub horizontal: f32,
    pub vertical: f32,
    pub rock_hardness: f32,
    pub edge: f32,
    pub downcutting: f32,
    pub depth: f32,
}

impl Default for CrumbleSettings {
    fn default() -> Self {
        Self {
            duration: 0.25,
            strength: 0.5,
            coverage: 0.75,
            horizontal: 0.45,
            vertical: 0.0,
            rock_hardness: 0.45,
            edge: 0.45,
            downcutting: 0.0,
            depth: 0.2,
        }
    }
}

/// `Simulations.Crumble`: the CS engine's snow run as rock wear, melting where
/// the noon sun at the `edge` latitude falls. Returns the worn terrain and
/// the wear.
pub fn crumble(heights: &Map, area: Option<&Map>, settings: &CrumbleSettings, terrain: TerrainSize) -> [Map; 2] {
    let melt = lerp(1.0 - settings.coverage, 0.0, 0.85);
    let duration = lerp(settings.duration, 0.01, 0.75);
    let horizontal = lerp(settings.horizontal, 1.0, 10.0);
    let intensity = lerp(settings.strength, 0.1, 1.0);
    let verticality = lerp(settings.rock_hardness, 500.0, 30000.0) as i32;
    let downcutting = settings.downcutting * 0.92 + duration * 0.08;
    let latitude = lerp(settings.edge, 0.0, 90.0);
    let ratio = lerp(settings.depth, 0.0, 0.2);

    let mut shaped = heights.clone();
    if settings.vertical != 0.0 {
        maths::strengthen(&mut shaped, settings.vertical);
    }
    let sun = lighting::SunlightSettings {
        day_begin: 80,
        day_end: 80,
        latitude,
        north: 0.0,
        air_density: 0.01,
    };
    let exposure = lighting::sunlight_integral(&shaped, &sun, terrain);
    let snow_settings = SnowSettings {
        duration,
        intensity,
        settle_thaw: 0.1 * horizontal,
        melt,
        snow_line: 0.0,
        real_scale: false,
        scale: 5000.0,
        verticality: verticality as f32,
        slip_off: 38.0,
        adhered_mass: 12.0,
        model_scale: horizontal,
    };
    let [_, mut wear, _, _] = snow(&shaped, &snow_settings, area, Some(&exposure), terrain);
    if downcutting != 0.0 {
        maths::strengthen(&mut wear, downcutting);
    }
    let wear = maths::autolevel(wear);
    let half_wear = wear.multiply_scalar(0.5);
    let worn = combiner::subtract(heights, &half_wear, ratio);
    [worn, wear]
}
