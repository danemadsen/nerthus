//! Noise primitives: Perlin and Voronoi.

use nerthus_engine::{BuildContext, Map, NodeBuilder, NodeError, RenderIntent, Resolution};
use nerthus_noise::fast_noise::CellularDistanceFunction;

use crate::aspect;
use crate::combiner;
use crate::fast_blur;
use crate::gradients;
use crate::maths;
use crate::multi_fractal::{self, MultiFractalSettings, MultifractalType, VariationType};
use crate::random::XorShift64Star;
use crate::transformer::{self, AnchorPosition, ResizeMethod, TransformQuality};
use crate::random_noises;
use crate::warps::{self, FractalWarpSettings};
use crate::noise::{self, CellularSettings, CrackSettings, CrackStyle, DriftSettings, GaborSettings, NoisesPerlin, PerlinType, Viewport, VoronoiSettings, WaveShineSettings};

/// The height clamp and input mask every noise primitive applies.
fn finish(context: &BuildContext, mut map: Map) -> Map {
    let clamp = context.f32("Clamp");
    if clamp < 1.0 {
        map = map.multiply_scalar(clamp);
    }
    if let Some(input) = context.input("In") {
        map = map.multiply(&input);
    }
    map
}

pub struct Perlin;

impl NodeBuilder for Perlin {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let parameters = NoisesPerlin {
            scale: context.f32("Scale"),
            octaves: context.i32("Octaves"),
            gain: context.f32("Gain"),
            kind: PerlinType::from_name(context.enum_name("Type")),
            seed: context.i32("Seed"),
            warp_type: context.enum_value("WarpType"),
            frequency: context.f32("Frequency"),
            amplitude: context.f32("Amplitude"),
            warp_octaves: context.i32("WarpOctaves"),
            scale_x: context.f32("ScaleX"),
            scale_y: context.f32("ScaleY"),
            x: 1.0 - context.f32("X"),
            y: context.f32("Y"),
        };
        let map = noise::noises_perlin(context.resolution.x, &parameters);
        let map = finish(context, map);
        context.commit(map)
    }
}

pub struct Voronoi;

impl NodeBuilder for Voronoi {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let function = match context.enum_name("Function") {
            "Euclidean" => CellularDistanceFunction::Euclidean,
            "Manhattan" => CellularDistanceFunction::Manhattan,
            _ => CellularDistanceFunction::Natural,
        };
        let settings = VoronoiSettings {
            scale: context.f32("Scale"),
            function,
            form: noise::voronoi_form(context.enum_name("Form")),
            warp: noise::perturb_for_warp(context.enum_value("WarpType")),
            warp_frequency: context.f32("Frequency"),
            warp_amplitude: context.f32("Amplitude"),
            warp_octaves: context.i32("WarpOctaves"),
            seed: context.i32("Seed"),
            scale_x: context.f32("ScaleX"),
            scale_y: context.f32("ScaleY"),
            jitter: context.f32("Jitter"),
        };
        let viewport = Viewport::with_offset([1.0 - context.f32("X"), context.f32("Y")]);
        let raw = noise::voronoi(context.resolution.x, &settings, viewport);
        let leveled = maths::autolevel(raw);
        let map = maths::drop_to_zero(&leveled);
        let map = finish(context, map);
        context.commit(map)
    }
}

/// The engine's own cellular noise with metric morphing, axis jitter and
/// anisotropy.
pub struct Cellular;

impl NodeBuilder for Cellular {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let settings = CellularSettings {
            size: context.f32("Size"),
            density: context.i32("Density"),
            jitter: context.f32("Jitter"),
            distance_metric: context.enum_value("DistanceMetric") as i32,
            metric_morph: context.f32("MetricMorph"),
            axes_angular_jitter: context.f32("CoordinateAxesJitter"),
            clustering: context.f32("CAJClustering"),
            anisotropy: context.f32("Anisotropy"),
            distance_function: context.enum_value("DistanceFunction") as i32,
            seed: context.i32("Seed"),
        };
        let mut map = noise::cellular(context.resolution.x, &settings);
        if let Some(input) = context.input("In") {
            map = map.multiply(&input);
        }
        context.commit(map)
    }
}

/// Per-pixel noise (`Noise`), optionally combined with the input.
pub struct Noise;

impl NodeBuilder for Noise {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.input("In");
        let height = context.f32("Height");
        let seed = context.i32("Seed");
        let density = context.f32("Density");
        let resolution = context.resolution;
        let mut map = match context.enum_name("NoiseType") {
            "Perlin" => random_noises::perlin(resolution, height),
            "Gaussian" => random_noises::gaussian(resolution, seed, 0.0, height),
            "Fixed" => {
                let count = (5_000_000.0 * density) as i32;
                random_noises::fixed(resolution, count, context.bool("RelativeDensity"), 0.0, height, seed)
            }
            "Micro" => {
                let Some(source) = input.as_ref() else {
                    return Err(NodeError::Failed("Micro noise needs an input.".to_string()));
                };
                let settings = FractalWarpSettings {
                    size: 0.001,
                    strength: density * 10.0,
                    octaves: 2,
                    roughness: 1.0,
                    seed,
                    ..FractalWarpSettings::default()
                };
                let warped = warps::fractal_warp(source, &settings, None, context.terrain_width(), context.terrain_height());
                combiner::blend_maps(source, &warped, height)
            }
            _ => random_noises::uniform(resolution, 0.0, height, seed),
        };
        if let Some(input) = input {
            map = match context.enum_name("BlendMode") {
                "Multiply" => map.multiply(&input),
                "Subtract" => input.subtract(&map),
                _ => map.add(&input),
            };
            maths::clamp(&mut map);
        }
        context.commit(map)
    }
}

/// Three thresholded Perlin layers (`CutNoise`).
pub struct CutNoise;

impl NodeBuilder for CutNoise {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let size = context.resolution.x;
        let seed = context.i32("Seed");
        let scale = context.f32("Scale");
        let layer = |scale: f32, octaves: i32, seed: i32| {
            let parameters = NoisesPerlin {
                scale,
                octaves,
                gain: 0.5,
                seed,
                ..NoisesPerlin::default()
            };
            noise::noises_perlin(size, &parameters)
        };
        let first = layer(scale * 1.2, 1, seed);
        let second = layer(scale, 2, seed + 1);
        let third = layer(scale * 1.5, 1, seed + 2);
        let lower = combiner::min(&second, &third, 1.0);
        let lowest = combiner::min(&first, &lower, 1.0);
        let mut map = lowest.multiply_scalar(0.25);
        if context.is_connected("In") {
            let input = context.require_input("In")?;
            map = map.multiply(&input);
        }
        context.commit(map)
    }
}

/// Sparse Gabor noise (`Noises.Gabor`).
pub struct Gabor;

impl NodeBuilder for Gabor {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let modulation_name = context.input_names().get(1).map(|name| name.to_string());
        let modulation = modulation_name.and_then(|name| context.input(&name));
        let settings = GaborSettings {
            size: context.f32("Size"),
            entropy: context.f32("Entropy"),
            anisotropy: context.f32("Anisotropy"),
            direction: context.f32("AnisotropyAzimuth"),
            seed: context.i32("Seed"),
        };
        let modulation_data = modulation.as_ref().map(|map| map.data());
        let mut map = noise::gabor(context.resolution.x, &settings, modulation_data);
        map = map.multiply_scalar(context.f32("Gain"));
        if let Some(input) = context.input("In") {
            map = map.multiply(&input);
        }
        context.commit(map)
    }
}

/// Cracks from Voronoi edges (`Noises.Cracks`).
pub struct Cracks;

impl NodeBuilder for Cracks {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let settings = CrackSettings {
            octaves: context.i32("Octaves"),
            scale: context.f32("Scale"),
            seed: context.i32("Seed"),
            scale_x: context.f32("ScaleX"),
            scale_y: context.f32("ScaleY"),
            jitter: context.f32("Jitter"),
            warp_size: context.f32("WarpSize"),
            warp_strength: context.f32("WarpStrength"),
            style: CrackStyle::from_name(context.enum_name("Style")),
            depth: context.f32("Depth"),
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let mut map = noise::cracks(context.resolution, &settings, terrain);
        maths::clamp(&mut map);
        maths::invert(&mut map);
        if let Some(input) = context.input("In") {
            map = map.multiply(&input);
        }
        context.commit(map)
    }
}

/// DotNoise: dots (or squares) scattered by random size and height.
pub struct DotNoise;

impl NodeBuilder for DotNoise {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let half = context.resolution.x as f32 / 2.0;
        let stamp_size = half as i32 as usize;
        let stamp = if context.enum_name("Style") == "Dot" {
            gradients::radial(stamp_size, 0.5 * half, 0.5 * half, 0.5 * half, 1.0)
        } else {
            Map::filled(Resolution::square(stamp_size), 0.5)
        };
        let mode = transformer::BlendMode::from_name(context.enum_name("BlendMode"));
        let [low, high] = context.float2("Size");
        let height = context.f32("Height");
        let mut dots = Map::new(context.resolution, 1);
        let mut random = XorShift64Star::new(context.i32("Seed"));
        let count = (context.i32("Iterations") as f32 * context.f32("Multiplier")) as i32;
        for _ in 0..count {
            let mut lift = random.next_float_range(0.0, height);
            let mut size = random.next_float();
            size = low + (high - low) * size;
            lift *= size;
            let position = (random.next_float(), random.next_float());
            transformer::multi_transform(&stamp, &mut dots, position, (size, size), 0.0, lift, mode, TransformQuality::High, None, true, AnchorPosition::Center);
        }
        match context.input("In") {
            Some(input) => context.commit(input.add(&dots)),
            None => context.commit(dots),
        }
    }
}

/// LineNoise: a random 1D line stretched across the map.
pub struct LineNoise;

impl NodeBuilder for LineNoise {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let length = match context.enum_name("BucketSize") {
            "X2" => 1024,
            "X3" => 2048,
            "X4" => 4096,
            _ => 512,
        };
        let clamp = context.f32("Clamp");
        let mut random = XorShift64Star::new(context.i32("Seed"));
        let values: Vec<f32> = (0..length).map(|_| random.next_float_range(0.0, clamp)).collect();
        let line = Map::from_data(Resolution::new(length, 1), values);
        let mut map = transformer::resize_with(&line, context.resolution, ResizeMethod::LinearFast);
        match context.enum_name("Style") {
            "Soft" => map = fast_blur::blur_relative(&map, 6.0 * 0.25),
            "Flat" => {
                map = maths::autolevel(fast_blur::blur_relative(&map, 6.0 * 0.25));
                map = aspect::threshold(&map, random.next_float());
                map = map.multiply_scalar(clamp);
            }
            "Vague" => {
                map = maths::autolevel(fast_blur::blur_relative(&map, 10.0 * 0.25));
                map = map.multiply_scalar(0.8);
                map = map.multiply_scalar(clamp);
            }
            _ => {}
        }
        let direction = context.i32("Direction");
        if direction != 0 {
            map = transformer::rotate(&map, direction as f32, false, transformer::EdgeBehaviour::Mirror, true);
        }
        if let Some(input) = context.input("In") {
            map = map.multiply(&input);
        }
        context.commit(map)
    }
}

/// MultiFractal: multifractal Perlin noise (`Noises.MultiFractal`).
pub struct MultiFractal;

impl NodeBuilder for MultiFractal {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let perturb = match context.enum_name("Perturb") {
            "Simple" => 1,
            "Complex" => 2,
            _ => 0,
        };
        let settings = MultiFractalSettings {
            noise_type: MultifractalType::from_name(context.enum_name("NoiseType")),
            octaves: context.i32("Octaves"),
            size: context.f32("Size"),
            feature_size: context.f32("FeatureSize"),
            fractal_gain: context.f32("FractalGain"),
            edge_smoothing: context.f32("EdgeSmoothing"),
            seed: context.i32("Seed"),
            variation_type: VariationType::from_name(context.enum_name("VariationType")),
            variation: context.f32("Variation"),
            smoothness: context.f32("Smoothness"),
            variation_contrast: context.f32("VariationContrast"),
            damping: context.f32("Damping"),
            bias: context.f32("Bias"),
            perturb,
            relative_warp_size: context.f32("RelativeWarpSize"),
            warp_strength: context.f32("WarpStrength"),
            warp_complexity: context.i32("WarpComplexity"),
            warp_roughness: context.f32("WarpRoughness"),
            warp_attenuation: context.f32("WarpAttenuation"),
            warp_iterations: context.i32("WarpIterations"),
            anisotropy: context.f32("Anisotropy"),
            relative_warp_anisotropy: context.f32("RelativeWarpAnisotropy"),
            rotation: context.f32("Rotation"),
            offset: [context.f32("OffsetX"), context.f32("OffsetY")],
            top_left: [1.0 / 3.0, 0.0],
            bottom_right: [2.0 / 3.0, 1.0 / 3.0],
        };
        let mut map = multi_fractal::multi_fractal(context.resolution, &settings);
        if let Some(input) = context.input("In") {
            map = map.multiply(&input);
        }
        context.commit(map)
    }
}

/// DriftNoise: wind-blown drifts (`Noises.DriftNoise`).
pub struct DriftNoise;

impl NodeBuilder for DriftNoise {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let settings = DriftSettings {
            passes: context.i32("Passes"),
            chaos: context.f32("Chaos"),
            scale_large: context.f32("ScaleLarge"),
            scale_small: context.f32("ScaleSmall"),
            turbulence_large: context.f32("TurbulenceLarge"),
            turbulence_small: context.f32("TurbulenceSmall"),
            wind_direction_large: context.f32("WindDirectionLarge"),
            wind_direction_small: context.f32("WindDirectionSmall"),
            overlap_shapes: context.bool("OverlapShapes"),
            seed: context.i32("Seed"),
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let drifts = noise::drift_noise(context.resolution.x, &settings, terrain);
        match context.input("In") {
            Some(input) => context.commit(input.add(&drifts)),
            None => context.commit(drifts),
        }
    }
}

/// WaveShine: animated sun glints on rippling water.
pub struct WaveShine;

impl NodeBuilder for WaveShine {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.render_as(RenderIntent::Mask);
        let settings = WaveShineSettings {
            frame: context.i32("Frame"),
            octaves: context.i32("Octaves"),
            seed: context.i32("Seed"),
            animation_speed: context.f32("AnimationSpeed"),
            size: context.f32("Size"),
            amplitude: context.f32("Amplitude"),
            direction: context.f32("Direction"),
            speed: context.f32("Speed"),
            wind_patch_intensity: context.f32("WindPatchIntensity"),
            wind_patch_animation_speed: context.f32("WindPatchAnimationSpeed"),
            wind_patch_size: context.f32("WindPatchSize"),
            contrast: context.f32("Contrast"),
            brightness: context.f32("Brightness"),
            sun_azimuth: context.f32("SunAzimuth"),
            sun_elevation: context.f32("SunElevation"),
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let mut map = noise::wave_shine(context.resolution.x, &settings, terrain);
        if context.is_connected("In") {
            let input = context.require_input("In")?;
            map = map.multiply(&input);
        }
        context.commit(map)
    }
}
