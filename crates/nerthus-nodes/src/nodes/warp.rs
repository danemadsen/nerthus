//! The Warp family: Warp, DirectionalWarp and TriplanarDisplacement.

use nerthus_engine::{BuildContext, Map, NodeBuilder, NodeError, RenderIntent};

use crate::mesh_rasterizer::{self, TriplanarSettings};
use crate::transformer::EdgeBehaviour;
use crate::warps::{self, FractalWarpSettings, IterativeWarpMode, WarpNoiseType};

/// Displaces the input with fractal noise.
pub struct Warp;

impl NodeBuilder for Warp {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let modulator_name = context.input_names().get(1).map(|name| name.to_string());
        let modulator = modulator_name.and_then(|name| context.input(&name));
        let mode = match context.enum_name("Mode") {
            "Bitmap" => IterativeWarpMode::Real,
            "VectorFieldIntegral" => IterativeWarpMode::Integral,
            _ => IterativeWarpMode::Virtual,
        };
        let edges = if context.enum_name("EdgeBehaviour") == "Mirror" { EdgeBehaviour::Mirror } else { EdgeBehaviour::Edge };
        let settings = FractalWarpSettings {
            size: context.f32("Size"),
            strength: context.f32("Strength"),
            persist_strength: true,
            z_coefficient: context.f32("ZScale"),
            noise_type: WarpNoiseType::from_name(context.enum_name("NoiseType")),
            perturbation: context.f32("Perturbation"),
            octaves: context.i32("Complexity"),
            roughness: context.f32("Roughness"),
            normalized: context.bool("Normalized"),
            iterations: context.i32("Iterations"),
            mode,
            edges,
            seed: context.i32("Seed"),
            modulation: context.f32("Modulation"),
            direction: context.f32("ModulationDirection"),
            jitter: 0.45,
        };
        let width = context.terrain_width();
        let height = context.terrain_height();

        if input.channel_count() == 1 {
            let warped = warps::fractal_warp(&input, &settings, modulator.as_deref(), width, height);
            context.render_as(RenderIntent::Heightfield);
            return context.commit(warped);
        }
        let mut channels = Vec::with_capacity(input.channel_count());
        for channel in 0..input.channel_count() {
            let plane = Map::from_data(input.resolution(), input.channel(channel).to_vec());
            let warped = warps::fractal_warp(&plane, &settings, modulator.as_deref(), width, height);
            channels.push(warped.into_data());
        }
        context.render_as(RenderIntent::Color);
        let combined = Map::from_channels(input.resolution(), channels);
        context.commit(combined)
    }
}

/// DirectionalWarp: pushes each pixel along one direction by how far
/// `Custom` is from mid grey.
pub struct DirectionalWarp;

impl NodeBuilder for DirectionalWarp {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let custom = context.require_input("Custom")?;
        let strength = context.f32("Strength");
        let direction = context.f32("Direction");
        let edges = EdgeBehaviour::from_name(context.enum_name("EdgeBehaviour"));
        let warped = warps::direction_warp_map(&input, &custom, strength, direction, edges);
        context.commit(warped)
    }
}

/// TriplanarDisplacement: the input as a mesh with smoothed vertex
/// normals, pushed along them by three maps projected onto the XY, XZ and
/// YZ planes, rendered back (`TriplanarDisplacement.Build`). Missing maps
/// fall back to the first connected one. High quality meshes adaptively
/// and renders antialiased.
pub struct TriplanarDisplacement;

impl NodeBuilder for TriplanarDisplacement {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let xy = context.input("XY");
        let xz = context.input("XZ");
        let yz = context.input("YZ");
        let first_connected = xy.clone().or_else(|| xz.clone()).or_else(|| yz.clone());
        let Some(first_connected) = first_connected else {
            return Err(NodeError::Validation("Required port [In] not connected.".to_string()));
        };
        let xy = xy.unwrap_or_else(|| first_connected.clone());
        let xz = xz.unwrap_or_else(|| first_connected.clone());
        let yz = yz.unwrap_or(first_connected);

        let resolution = context.resolution;
        let pixel_count = resolution.x * resolution.y;
        let maps_fit = [&xy, &xz, &yz].iter().all(|map| map.data().len() >= pixel_count);
        if !maps_fit {
            return Err(NodeError::Failed("TriplanarDisplacement's maps must match the build resolution.".to_string()));
        }

        let high_quality = context.enum_name("Quality") == "High";
        let settings = TriplanarSettings {
            width: resolution.x as i32,
            height: resolution.y as i32,
            height_ratio: context.terrain_height() / context.terrain_width(),
            strength: context.f32("Strength"),
            offset: context.f32("Offset"),
            horizontal: context.bool("Horizontal"),
            antialiasing: high_quality,
        };
        let terrain_width = context.terrain_width();
        let terrain_height = context.terrain_height();
        let mesh = if high_quality {
            mesh_rasterizer::adaptive_heightmap_to_mesh(input.data(), input.width(), input.height(), terrain_width, terrain_height, 16)
        } else {
            mesh_rasterizer::heightmap_to_mesh_with_normals(input.data(), input.width(), input.height(), terrain_width, terrain_height, 6)
        };
        let maps = [xy.data(), xz.data(), yz.data()];
        let rendered = mesh_rasterizer::render_mesh_triplanar(&mesh, &settings, maps);
        context.commit(Map::from_data(resolution, rendered))
    }
}
