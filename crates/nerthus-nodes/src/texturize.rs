//! The texture helpers (`Texturize`).

use nerthus_engine::Map;

use crate::aspect;
use crate::combiner;
use crate::fast_blur;
use crate::flow::{self, FlowSettings};
use crate::transformer::EdgeBehaviour;
use crate::maths;
use crate::noise::{self, NoisesPerlin, PerlinType};
use crate::profiles;
use crate::warps::{self, FractalWarpSettings, IterativeWarpMode, WarpNoiseType};

/// `Texturize.Soil2`: where five ever wider blurs rise above the map,
/// levelled to 0..1.
pub fn soil2(map: &Map, soil: f32) -> Map {
    let mut accumulated = Map::new(map.resolution(), 1);
    let mut blurred = map.clone();
    for pass in 0..5 {
        blurred = fast_blur::blur_relative(&blurred, soil / 2.0 * (pass + 1) as f32);
        accumulated = blurred.subtract(map).add(&accumulated);
    }
    maths::autolevel(accumulated)
}

/// The parameters of `Texturize.TextureBase`.
pub struct TextureBaseSettings {
    pub slope: f32,
    pub scale: f32,
    pub soil: f32,
    pub chaos: f32,
    pub patches: f32,
    pub seed: i32,
    /// Nodes saved before the slope masks changed use the old ones.
    pub old_slopes: bool,
}

/// `Texturize.TextureBase`: a texture mask from slopes, flow, soil bands
/// and warped noise patches. `guide` steers where the flow gathers.
pub fn texture_base(map: &Map, guide: Option<&Map>, settings: &TextureBaseSettings, terrain: (f32, f32)) -> Map {
    let (terrain_width, terrain_height) = terrain;
    let (gentle, steep) = if settings.old_slopes {
        let gentle = aspect::slope_normalized_old(map, 0.1, settings.slope * 0.25, 0.3);
        let steep = aspect::slope_normalized_old(map, 0.6, 1.0, 0.3);
        (gentle, steep)
    } else {
        let gentle = aspect::slope(map, 9.0, settings.slope * 0.25 * 90.0, 27.000_002, terrain_width, terrain_height);
        let steep = aspect::slope(map, 54.000_004, 90.0, 27.000_002, terrain_width, terrain_height);
        (gentle, steep)
    };

    let flow_settings = FlowSettings {
        flow_length: settings.scale,
        flow_volume: settings.soil,
        seed: settings.seed,
    };
    let flow = flow::simulate(map, guide, &flow_settings);
    let mut flow = maths::autolevel(flow);
    let flow_contrast = -1.0 + settings.scale * (1.0 - -1.0);
    maths::strengthen(&mut flow, flow_contrast);

    let noise_settings = NoisesPerlin {
        scale: 0.5 * settings.scale,
        octaves: 12,
        gain: 0.65,
        kind: PerlinType::Fbm,
        seed: settings.seed + 1,
        ..NoisesPerlin::default()
    };
    let noise = noise::noises_perlin(map.width(), &noise_settings);
    let noise = maths::autolevel(noise);
    let slopes = gentle.add(&steep);
    let noisy_slopes = combiner::add(&noise, &slopes, 0.5);
    let patches = combiner::blend_maps(&noisy_slopes, &gentle, 0.65);

    let warp_settings = FractalWarpSettings {
        size: settings.chaos * 0.09,
        strength: settings.patches * 3.0,
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
        seed: settings.seed + 2,
        ..FractalWarpSettings::default()
    };
    let warped = warps::fractal_warp(&patches, &warp_settings, None, terrain_width, terrain_height);
    let texture = combiner::min(&patches, &warped, 1.0);

    let soil_bands = [0.0, 0.0, 0.0, 0.0, settings.soil * 0.5, settings.soil * 1.0, settings.soil * 2.0];
    let soil = maths::autolevel(profiles::graphic_eq(map, soil_bands));
    let soiled = combiner::blend_maps(&texture, &soil, settings.soil);
    let texture = combiner::add(&soiled, &flow, 0.7);
    let soil_share = soil.multiply_scalar(0.1);
    texture.add(&soil_share)
}
