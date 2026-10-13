//! Profile nodes: Terraces, FractalTerraces, Shaper, ThermalShaper,
//! Curve, Deflate, Fold and Recurve.

use nerthus_engine::{BuildContext, NodeBuilder, NodeError, RenderIntent};
use serde_json::Value;

use crate::fractal_terraces::{self, FractalTerraceSettings, WarpStyle};
use crate::maths;
use crate::profiles::{self, FoldWaveform};
use crate::spline::IrregularSpline;
use crate::thermal_shaper::{self, ThermalShaperSettings};

pub struct Terraces;

impl NodeBuilder for Terraces {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let count = context.i32("NumTerraces").max(0) as usize;
        let uniformity = context.f32("Uniformity");
        let steepness = context.f32("Steepness");
        let intensity = context.f32("Intensity");
        let seed = context.i32("Seed");
        let map = profiles::terrace(&input, count, uniformity, steepness, intensity, seed, false);
        context.commit(map)
    }
}

pub struct Shaper;

impl NodeBuilder for Shaper {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let shape = context.f32("Shape");
        let local_effect = context.bool("LocalEffect");
        let local_area = context.f32("LocalArea");
        let fine_details = context.bool("MaintainFineDetails");
        let detail_size = context.f32("DetailSize");
        let map = profiles::shaper(&input, shape, local_effect, local_area, fine_details, detail_size);
        context.commit(map)
    }
}

/// Remaps heights through a user curve (`Migrated.CurveProcess`).
pub struct Curve;

/// `Curve.ExecuteFitCurve` (the Fit Curve command): eleven points from
/// (0, 0) to (1, the input's highest value), in steps of 0.1.
pub fn fit_curve(input: &nerthus_engine::Map) -> Vec<[f32; 2]> {
    let (_, maximum) = maths::min_max(input.data());
    let mut points = Vec::with_capacity(11);
    for step in 0..11 {
        let x = 0.1f32 * step as f32;
        let y = 0.1f32 * step as f32 * maximum;
        points.push([x, y]);
    }
    points
}

impl NodeBuilder for Curve {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let points: Vec<(f32, f32)> = context
            .value("CurveSpline")
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|item| {
                        let x = item.get("X").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                        let y = item.get("Y").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                        (x, y)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let relative = context.enum_name("Mode") == "Relative";

        let mut map = (*input).clone();
        let mut range = (1.0f32, 0.0f32);
        if relative {
            let (minimum, maximum) = maths::min_max(map.data());
            range = (maximum, minimum);
            map = maths::autolevel(map);
        }
        let spline = IrregularSpline::from_unsorted(&points);
        let mut curved = map.map_values(|height| spline.interpolate(height));
        if relative {
            // 0 maps back to the old maximum and 1 to the old minimum.
            maths::remap(&mut curved, 0.0, 1.0, range.0, range.1);
        }
        context.commit(curved)
    }
}

/// Flattens the large forms (`Profiles.GraphicEQ` on the lowest bands).
pub struct Deflate;

impl NodeBuilder for Deflate {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let amount = context.f32("Amount");
        let first = maths::dotnet_max(0.0, 1.0 - amount * 2.0);
        let gains = [first, 0.0 - amount, 1.0, 1.0, 1.0, 1.0, 1.0];
        let equalized = profiles::graphic_eq(&input, gains);
        let lowered = maths::drop_to_zero(&equalized);
        context.commit(lowered)
    }
}

/// Folds heights back on themselves (`Profiles.Sine/Triangle/Sawtooth`).
pub struct Fold;

impl NodeBuilder for Fold {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let mask_name = context.input_names().get(1).map(|name| name.to_string());
        let mask = mask_name.and_then(|name| context.input(&name));
        let waveform = FoldWaveform::from_name(context.enum_name("Waveform"));
        let folded = profiles::fold(&input, mask.as_deref(), waveform, context.f32("Folds"), context.bool("Symmetric"));
        context.commit(folded)
    }
}

/// Bends slopes inward or outward (`Profiles.Recurve`).
pub struct Recurve;

impl NodeBuilder for Recurve {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let outward = context.enum_name("Style") == "Outward";
        let recurved = profiles::recurve(&input, context.f32("Power"), context.i32("Iterations"), outward, true, context.f32("Scale"));
        context.commit(recurved)
    }
}

/// FractalTerraces: strata terraces with optional tilt and modulation
/// (`Profiles.FractalTerrace`).
pub struct FractalTerraces;

impl NodeBuilder for FractalTerraces {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let modulator = context.input("Modulation");
        let settings = FractalTerraceSettings {
            spacing: context.f32("Spacing"),
            octaves: context.i32("Octaves"),
            intensity: context.f32("Intensity"),
            shape: context.f32("Shape"),
            seed: context.i32("Seed"),
            shapes_separation: context.bool("ShapesSeparation"),
            macro_octaves: context.i32("MacroOctaves"),
            micro_shape: context.f32("MicroShape"),
            character: context.f32("Character"),
            thickness_uniformity: context.f32("ThicknessUniformity"),
            hardness_uniformity: context.f32("HardnessUniformity"),
            strata_details: context.f32("StrataDetails"),
            protect_range: context.bool("ProtectRange"),
            tilt: context.bool("ApplyTilt"),
            tilt_amount: context.f32("TiltAmount"),
            tilt_seed: context.i32("TiltSeed"),
            direction: context.i32("Direction"),
            warp_amount: context.f32("WarpAmount"),
            warp_size: context.f32("WarpSize"),
            warp_style: WarpStyle::from_name(context.enum_name("WarpStyle")),
        };
        let terrain = (context.terrain_width(), context.terrain_height());
        let (terraced, layers) = fractal_terraces::fractal_terrace(&input, modulator.as_deref(), &settings, terrain);
        context.commit(terraced)?;
        context.commit_port("Layers", layers, RenderIntent::Default);
        Ok(())
    }
}

/// Stratify: complex tilted strata (`Profiles.ComplexTerraces`).
pub struct Stratify;

impl NodeBuilder for Stratify {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let terrain = (context.terrain_width(), context.terrain_height());
        let (terraced, layers) = fractal_terraces::complex_terraces(
            &input,
            context.f32("Intensity"),
            context.f32("Shape"),
            context.f32("Spacing"),
            context.f32("TiltAmount"),
            context.i32("Direction"),
            context.i32("Octaves"),
            context.i32("Seed"),
            terrain,
        );
        context.commit(terraced)?;
        context.commit_port("Layers", layers, RenderIntent::Default);
        Ok(())
    }
}

/// ThermalShaper: the SYCL thermal shaper (`ThermalShaper_Process`),
/// reshaping slopes towards a talus-like profile over a Laplacian pyramid.
/// The Intensity input is read from its pixels.
pub struct ThermalShaper;

impl NodeBuilder for ThermalShaper {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let intensity = context.input("Intensity");
        let settings = ThermalShaperSettings {
            terrain_width: context.terrain_width(),
            terrain_height: context.terrain_height(),
            scale: context.f32("Scale"),
            influence: context.f32("Influence"),
            shape: context.f32("Shape"),
            microdetail: context.f32("MicrodetailPreservation"),
        };
        let shaped = thermal_shaper::process(&input, intensity.as_deref(), &settings);
        context.commit(shaped)
    }
}
