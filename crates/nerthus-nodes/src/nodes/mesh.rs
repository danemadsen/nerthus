//! The mesh nodes: Meshify, and those whose builds pass maps on for the
//! export step (Mesher, PointCloud, MeshWarp and TextureBaker).

use nerthus_engine::{BuildContext, Map, NodeBuilder, NodeData, NodeError, RenderIntent};
use serde_json::Value;

use crate::debris::DebrisPointCloud;
use crate::mesh_rasterizer;
use crate::meshify;
use crate::profiles;

/// Meshify: the terrain as a triangle mesh of `Vertices` points placed
/// where it bends, rendered back (`Meshify.Build`); `Hash` gives each
/// triangle a random value.
pub struct Meshify;

impl NodeBuilder for Meshify {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let mut texture = context.input("Texture").map(|map| (*map).clone());
        let point_count = context.f32("Vertices") as i32;
        let seed = context.i32("Seed");
        let detail_sensitivity = context.f32("DetailSensitivity");
        let terrain_width = context.terrain_width();
        let terrain_height = context.terrain_height();
        let mut probability = meshify::probability_mask(&input, detail_sensitivity, terrain_width, terrain_height);

        let mut points = vec![[0.0f32; 3]; point_count.max(0) as usize];
        if context.node.version() == 0 {
            meshify::place_points_v0(&input, &probability, &mut points, seed);
        } else {
            meshify::place_points_v1(&input, &mut probability, &mut points, seed);
        }

        let resolution = context.resolution;
        let width = resolution.x as i32;
        let height = resolution.y as i32;
        let (heights, hashes) = match texture.as_mut() {
            Some(texture) => {
                if texture.resolution() != resolution {
                    return Err(NodeError::Failed("Meshify's Texture must match the build resolution.".to_string()));
                }
                let painted = mesh_rasterizer::Texture::new(texture.channels_mut());
                mesh_rasterizer::render_point_cloud_textured(&points, width, height, painted)
            }
            None => mesh_rasterizer::render_point_cloud(&points, width, height),
        };
        context.commit(Map::from_data(resolution, heights))?;
        context.commit_port("Hash", Map::from_data(resolution, hashes), RenderIntent::Default);
        if let Some(texture) = texture {
            context.commit_port("Color", texture, RenderIntent::Color);
        }
        Ok(())
    }
}

/// Mesher: passes its input on for the build's mesh export, optionally
/// with its finest detail bands taken out (`Mesher.Build`).
pub struct Mesher;

impl NodeBuilder for Mesher {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let map = context.require_input("In")?;
        let reduced = match context.enum_name("ArtifactReduction") {
            "Low" => profiles::graphic_eq(&map, [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0]),
            "Med" => profiles::graphic_eq(&map, [1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0]),
            _ => (*map).clone(),
        };
        context.commit(reduced)
    }
}

/// PointCloud: passes its input on for the build's point export. Fed by a
/// Debris node that exports a point cloud, it keeps that node's rocks
/// for the export (`pointCloud = debris.pointCloud.ToArray()`); Debris
/// keeps none in the editor, where the export then fails.
pub struct PointCloud;

impl NodeBuilder for PointCloud {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let source = context.source_node("In");
        let exports_points = source.is_some_and(|node| {
            let flag = node.values.get("ExportPointCloud").and_then(Value::as_bool);
            node.type_name == "Debris" && flag == Some(true)
        });
        if exports_points {
            let Some(points) = context.source_memory::<DebrisPointCloud>("In") else {
                return Err(NodeError::Failed("Object reference not set to an instance of an object.".to_string()));
            };
            context.remember(DebrisPointCloud(points.0.clone()));
        }
        let map = context.require_input("In")?;
        context.commit((*map).clone())
    }
}

/// The input ports after `In` (`Ins.Skip(1)`).
fn extra_inputs(context: &BuildContext) -> Vec<String> {
    let inputs = context.node.ports.iter().filter(|port| port.is_input());
    inputs.skip(1).map(|port| port.name.clone()).collect()
}

/// MeshWarp's and TextureBaker's own validity check (it replaces the base
/// one): maps on their extra inputs must not be exported by their nodes.
fn check_extra_exports(context: &BuildContext, owner: &str) -> Result<(), NodeError> {
    let mut problems = Vec::new();
    for port in extra_inputs(context) {
        let Some(source) = context.source_node(&port) else {
            continue;
        };
        let save_definition = source.base.get("SaveDefinition");
        let exported = save_definition.and_then(|definition| definition.get("IsEnabled")).and_then(Value::as_bool).unwrap_or(false);
        if exported {
            let name = if source.name.is_empty() { "Connected nodes" } else { source.name.as_str() };
            problems.push(format!("{name} must be unmarked from Export to avoid conflict with {owner}'s export of the same file."));
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    Err(NodeError::Validation(problems.join("\n")))
}

/// MeshWarp and TextureBaker: the Mesher's map on, and each extra input
/// `In<n>` on to `Out<n>`, for their exports.
fn pass_mesher_and_extras(context: &mut BuildContext) -> Result<(), NodeError> {
    let from_mesher = context.source_type("In") == Some("Mesher");
    if !from_mesher {
        return Err(NodeError::Failed("Mesher node must be connected to the Mesher input.".to_string()));
    }
    let map = context.require_input("In")?;
    context.commit((*map).clone())?;
    for port in extra_inputs(context) {
        let Some(extra) = context.input(&port) else {
            return Err(NodeError::Validation(format!("Required port [{port}] not connected.")));
        };
        let intent = extra.info.render_intent;
        let output = port.replace("In", "Out");
        context.commit_port(&output, (*extra).clone(), intent);
    }
    Ok(())
}

/// `AddNewPort` of MeshWarp and TextureBaker: `In<n>` and `Out<n>` pairs,
/// up to a port count of 20.
fn add_port_pair(node: &mut NodeData) -> Option<String> {
    let count = node.values.get("PortCount").and_then(Value::as_i64).unwrap_or(2);
    if count >= 20 {
        return None;
    }
    let number = count / 2;
    let input = format!("In{number}");
    node.add_port(&input, "In");
    node.add_port(&format!("Out{number}"), "Out");
    node.values.insert("PortCount".to_string(), Value::from(count + 2));
    Some(input)
}

/// MeshWarp: see [`pass_mesher_and_extras`].
pub struct MeshWarp;

impl NodeBuilder for MeshWarp {
    fn check_validity(&self, context: &BuildContext) -> Result<(), NodeError> {
        check_extra_exports(context, "MeshWarp")
    }

    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        pass_mesher_and_extras(context)
    }

    fn add_new_port(&self, node: &mut NodeData) -> Option<String> {
        add_port_pair(node)
    }
}

/// TextureBaker: see [`pass_mesher_and_extras`].
pub struct TextureBaker;

impl NodeBuilder for TextureBaker {
    fn check_validity(&self, context: &BuildContext) -> Result<(), NodeError> {
        check_extra_exports(context, "TextureBaker")
    }

    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        pass_mesher_and_extras(context)
    }

    fn add_new_port(&self, node: &mut NodeData) -> Option<String> {
        add_port_pair(node)
    }
}
