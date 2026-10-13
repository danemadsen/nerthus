//! Nodes that blend several maps: Combine, Mask, Layers and Mixer.

use nerthus_engine::{BuildContext, Map, NodeBuilder, NodeData, NodeError, RenderIntent};
use serde_json::Value;

use crate::aspect;
use crate::combiner::{self, BlendMode};
use crate::histogram;
use crate::maths;
use crate::masking;
use crate::warps::{self, FractalWarpSettings};

/// The message when a connected input has no map to give
/// (`NullReferenceException` inside `Build`).
fn missing_map(port: &str) -> NodeError {
    NodeError::Failed(format!("The port {port} has no data."))
}

/// `Node.AddNewPort` for nodes that number their extra inputs: adds one
/// input port after the existing ones and counts it in `PortCount`.
pub(crate) fn add_numbered_port(node: &mut NodeData, prefix: &str, limit: i64) -> Option<String> {
    let count = node.values.get("PortCount").and_then(Value::as_i64).unwrap_or(0);
    if count >= limit {
        return None;
    }
    let name = format!("{prefix}{}", count + 1);
    node.add_port(&name, "In");
    node.values.insert("PortCount".to_string(), Value::from(count + 1));
    Some(name)
}

/// The Combine node's `Enhance` option (`ProcessInput`).
fn enhance(map: Map, mode: &str) -> Map {
    match mode {
        "Autolevel" => maths::autolevel(map),
        "Equalize" => histogram::equalize(map),
        _ => map,
    }
}

/// Blends every other input into `In`, one after the other.
pub struct Combine;

impl NodeBuilder for Combine {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input_names: Vec<String> = context.input_names().into_iter().map(str::to_string).collect();
        let second_connected = input_names.get(1).is_some_and(|name| context.is_connected(name));
        if !context.is_connected("In") && second_connected {
            return Ok(());
        }

        let enhance_mode = context.enum_name("Enhance").to_string();
        let Some(input) = context.input("In") else {
            return Err(NodeError::Validation("The port In returned bad or no data.".to_string()));
        };
        let base = enhance((*input).clone(), &enhance_mode);
        if base.channel_count() > 1 {
            context.render_as(RenderIntent::Color);
            context.update_member("RenderIntentOverride", Value::from("Color"));
        }

        let mode = BlendMode::from_name(context.enum_name("Mode"));
        let ratio = context.f32("Ratio");
        let mut combined = base.clone();
        for name in input_names.iter().skip(1) {
            if name == "Mask" {
                continue;
            }
            let layer = if context.is_connected(name) {
                let map = context.input(name).ok_or_else(|| missing_map(name))?;
                (*map).clone()
            } else {
                Map::new(context.resolution, 1)
            };
            let layer = enhance(layer, &enhance_mode);
            combined = combiner::mix(mode, ratio, &combined, &layer);
        }

        match context.enum_name("Clamp") {
            "Clamp" => maths::clamp(&mut combined),
            "Extend" => {
                combined = maths::lift_above_zero(combined);
                maths::clamp(&mut combined);
            }
            _ => {}
        }
        if let Some(mask) = context.input("Mask") {
            combined = masking::mask(&base, &combined, &mask);
        }
        context.commit_with(combined, false, false)
    }

    fn add_new_port(&self, node: &mut NodeData) -> Option<String> {
        add_numbered_port(node, "Input", 10)
    }
}

/// Blends the node's input back over what the upstream node started from
/// (or over `In2`), through `Mask`.
pub struct Mask;

impl NodeBuilder for Mask {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let background = if !context.source_is_primary("In") {
            Some(Map::new(context.resolution, 1))
        } else if context.is_connected("In2") {
            context.input("In2").map(|map| (*map).clone())
        } else {
            context.upstream_input("In").map(|map| (*map).clone())
        };
        let foreground = context.input("In");
        let mask = match context.input("Mask") {
            Some(map) => (*map).clone(),
            None => Map::filled(context.resolution, 1.0),
        };

        let (Some(background), Some(foreground)) = (background, foreground) else {
            return Ok(());
        };
        let masked = masking::mask(&background, &foreground, &mask);
        if foreground.channel_count() == 3 {
            context.render_as(RenderIntent::Color);
        }
        context.commit_with(masked, false, true)
    }
}

/// Stacks up to seven layers on `In`, each through its own mask.
pub struct Layers;

impl NodeBuilder for Layers {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        let mut map = (*input).clone();
        let blends = ["Blend2", "Blend3", "Blend4", "Blend5", "Blend6", "Blend7", "Blend8"].map(|name| context.f32(name));
        for layer_number in 1..8 {
            let Some(layer) = context.input(&format!("Layer{layer_number}")) else {
                continue;
            };
            let blend = blends[layer_number - 1];
            map = match context.input(&format!("Mask{layer_number}")) {
                Some(mask) => {
                    let weighted_mask = mask.multiply_scalar(blend);
                    masking::mask(&map, &layer, &weighted_mask)
                }
                None => map.add(&layer),
            };
        }
        map = maths::fit_range(&map, 0.0, 1.0);
        if map.channel_count() > 1 {
            context.render_as(RenderIntent::Color);
        }
        context.commit(map)
    }

    fn add_new_port(&self, node: &mut NodeData) -> Option<String> {
        let count = node.values.get("PortCount").and_then(Value::as_i64).unwrap_or(0);
        if count >= 14 {
            return None;
        }
        let layer_number = count / 2 + 1;
        let layer = format!("Layer{layer_number}");
        let mask = format!("Mask{layer_number}");
        node.add_port(&layer, "In");
        node.add_port(&mask, "In");
        node.values.insert("PortCount".to_string(), Value::from(count + 2));
        Some(layer)
    }
}

/// One of the Mixer's fifteen `BlendLayer`s.
struct BlendLayer {
    index: usize,
    order: i64,
    /// `MaskSource`: Constant, Height, Slope, Both or Custom.
    mask: String,
    blend_mode: BlendMode,
    blending: f32,
    range: [f32; 2],
    range2: [f32; 2],
    falloff: f32,
    falloff2: f32,
    warp: f32,
    size: f32,
    color: [f32; 3],
    is_visible: bool,
    is_simplified: bool,
}

impl BlendLayer {
    /// Reads a saved layer; members it leaves out keep `BlendLayer`'s own
    /// defaults.
    fn from_value(value: &Value, fallback_index: usize) -> Self {
        let number = |name: &str, default: f64| value.get(name).and_then(Value::as_f64).unwrap_or(default) as f32;
        let flag = |name: &str, default: bool| value.get(name).and_then(Value::as_bool).unwrap_or(default);
        let pair = |name: &str| {
            let member = value.get(name);
            let component = |key: &str| member.and_then(|pair| pair.get(key)).and_then(Value::as_f64).unwrap_or(0.0) as f32;
            match member {
                Some(_) => [component("X"), component("Y")],
                None => [0.0, 1.0],
            }
        };
        let colour = value.get("Color");
        let channel = |key: &str| colour.and_then(|colour| colour.get(key)).and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let blend_mode = value.get("BlendMode").and_then(Value::as_str).unwrap_or("Blend");
        Self {
            index: value.get("Index").and_then(Value::as_u64).map(|index| index as usize).unwrap_or(fallback_index),
            order: value.get("Order").and_then(Value::as_i64).unwrap_or(0),
            mask: value.get("Mask").and_then(Value::as_str).unwrap_or("Height").to_string(),
            blend_mode: BlendMode::from_name(blend_mode),
            blending: number("Blending", 1.0),
            range: pair("Range"),
            range2: pair("Range2"),
            falloff: number("Falloff", 0.0001),
            falloff2: number("Falloff2", 0.0001),
            warp: number("Warp", 0.0),
            size: number("Size", 0.25),
            color: [channel("R"), channel("G"), channel("B")],
            is_visible: flag("IsVisible", true),
            is_simplified: flag("IsSimplified", false),
        }
    }

    /// The fractal warp a layer's height or slope mask gets.
    fn warp_settings(&self, size: f32) -> FractalWarpSettings {
        FractalWarpSettings {
            size,
            strength: self.warp,
            seed: self.index as i32,
            ..FractalWarpSettings::default()
        }
    }
}

/// Mixer: paints colour layers over a colour map, each through a mask from
/// the terrain's height, slope, both, a constant or a custom map.
pub struct Mixer;

impl Mixer {
    /// `Mixer.GetMask`.
    fn mask(context: &BuildContext, layer: &BlendLayer) -> Result<Map, NodeError> {
        let terrain = match context.input("Terrain") {
            Some(map) => map,
            None => context.underlay_heightfield().ok_or_else(|| NodeError::Validation("Required port [Terrain] not connected.".to_string()))?,
        };
        let custom = context.input(&format!("Mask{}", layer.index + 1));
        let (terrain_width, terrain_height) = (context.terrain_width(), context.terrain_height());
        let resolution = context.resolution;
        let mask = match layer.mask.as_str() {
            "Constant" => custom.map(|map| (*map).clone()).unwrap_or_else(|| Map::filled(resolution, 1.0)),
            "Height" | "Slope" => {
                let height_based = layer.mask == "Height";
                let mut mask = if height_based {
                    aspect::height_normalized(&terrain, layer.range[0], layer.range[1], layer.falloff)
                } else {
                    aspect::slope_normalized_old(&terrain, layer.range[0], layer.range[1], layer.falloff)
                };
                if layer.warp > 0.0 {
                    let warp_size = if height_based { layer.size } else { 0.5 };
                    mask = warps::fractal_warp(&mask, &layer.warp_settings(warp_size), None, terrain_width, terrain_height);
                }
                mask = mask.multiply_scalar(layer.blending);
                if let Some(custom) = &custom {
                    mask = mask.multiply(custom);
                }
                mask
            }
            "Both" => {
                let whole_height = layer.range == [0.0, 1.0];
                let whole_slope = layer.range2 == [0.0, 1.0];
                let mut height_mask = None;
                if !whole_height {
                    let mut mask = aspect::height_normalized(&terrain, layer.range[0], layer.range[1], layer.falloff);
                    if layer.warp > 0.0 {
                        mask = warps::fractal_warp(&mask, &layer.warp_settings(layer.size), None, terrain_width, terrain_height);
                    }
                    if let Some(custom) = &custom {
                        mask = mask.multiply(custom);
                    }
                    height_mask = Some(mask);
                }
                let mut slope_mask = None;
                if !whole_slope {
                    let mut mask = aspect::slope_normalized_old(&terrain, layer.range2[0], layer.range2[1], layer.falloff2);
                    mask = mask.multiply_scalar(layer.blending);
                    if let Some(custom) = &custom {
                        mask = mask.multiply(custom);
                    }
                    if let Some(height_mask) = &height_mask {
                        mask = mask.multiply(height_mask);
                    }
                    slope_mask = Some(mask);
                }
                match (whole_height, whole_slope) {
                    (true, true) => Map::filled(resolution, 1.0),
                    (true, false) => slope_mask.expect("the slope mask exists"),
                    (false, true) => height_mask.expect("the height mask exists"),
                    (false, false) => slope_mask.expect("the slope mask exists"),
                }
            }
            "Custom" => custom.map(|map| (*map).clone()).unwrap_or_else(|| Map::new(resolution, 1)),
            _ => Map::new(resolution, 1),
        };
        Ok(mask)
    }

    /// `Mixer.ProcessLayer`.
    fn process_layer(context: &mut BuildContext, layer: &BlendLayer, base: Option<Map>) -> Result<Option<Map>, NodeError> {
        let port = format!("Layer{}", layer.index + 1);
        if context.node.port(&port).is_none() {
            return Ok(base);
        }
        let flat = !context.is_connected(&port) || context.bool("ShowSimplified") || layer.is_simplified;
        let colour = if flat {
            let channels = layer.color.iter().map(|&value| vec![value; context.resolution.area()]).collect();
            Some(Map::from_channels(context.resolution, channels))
        } else {
            context.input(&port).map(|map| (*map).clone())
        };
        if colour.as_ref().is_some_and(|map| map.channel_count() == 1) {
            return Err(NodeError::Failed("This node requires color input, not heightfields.".to_string()));
        }
        let mask = Self::mask(context, layer)?;
        if context.bool("PreserveMasks") {
            context.commit_port(&format!("MaskOut{}", layer.index + 1), mask.clone(), RenderIntent::Mask);
        }
        let (Some(base), Some(colour)) = (base, colour) else {
            return Err(NodeError::Failed("Object reference not set to an instance of an object.".to_string()));
        };
        let blended = combiner::mix(layer.blend_mode, layer.blending, &base, &colour);
        Ok(Some(masking::mask(&base, &blended, &mask)))
    }
}

impl NodeBuilder for Mixer {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let mut map = if context.bool("ShowSimplified") {
            let channels = vec![vec![0.2f32; context.resolution.area()]; 3];
            Some(Map::from_channels(context.resolution, channels))
        } else {
            context.input("In").map(|map| (*map).clone())
        };
        if map.as_ref().is_some_and(|map| map.channel_count() == 1) {
            return Err(NodeError::Failed("This node requires color input, not heightfields.".to_string()));
        }

        let mut layers: Vec<BlendLayer> = (0..15)
            .map(|index| {
                let value = context.value(&format!("Layer{}", index + 1));
                BlendLayer::from_value(value, index)
            })
            .collect();
        // `OrderBy` is stable.
        layers.sort_by_key(|layer| layer.order);
        for layer in layers.iter().filter(|layer| layer.is_visible) {
            map = Self::process_layer(context, layer, map)?;
        }

        let Some(map) = map else {
            return Err(NodeError::Failed("Object reference not set to an instance of an object.".to_string()));
        };
        let map = maths::fit_range(&map, 0.0, 1.0);
        context.render_as(RenderIntent::Color);
        context.commit(map)
    }

    fn add_new_port(&self, node: &mut NodeData) -> Option<String> {
        let count = node.values.get("PortCount").and_then(Value::as_i64).unwrap_or(0);
        if count >= 30 {
            return None;
        }
        let layer_number = count / 2 + 1;
        let layer = format!("Layer{layer_number}");
        node.add_port(&layer, "In");
        node.add_port(&format!("Mask{layer_number}"), "In");
        node.add_port(&format!("MaskOut{layer_number}"), "Out");
        node.values.insert("PortCount".to_string(), Value::from(count + 2));
        Some(layer)
    }

    /// `Mixer.GetCustomUnderlay`: the editor's underlay node when one is
    /// set, else the node wired into Terrain.
    fn custom_underlay(&self, context: &BuildContext) -> Option<i32> {
        let underlay = context.underlay_node();
        if underlay > 0 {
            return Some(underlay);
        }
        context.source_node("Terrain").map(|source| source.id)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use indexmap::IndexMap;
    use nerthus_engine::{Catalogue, ConnectionRecord, Engine, Resolution, Terrain};

    use super::*;

    fn node(type_name: &str, id: i32) -> NodeData {
        let spec = Catalogue::get().node(type_name).expect("the type exists");
        NodeData::new(spec, id, 1234)
    }

    fn wire(to: &mut NodeData, to_port: &str, from: i32) {
        let record = ConnectionRecord { from, to: to.id, from_port: "Out".to_string(), to_port: to_port.to_string(), is_valid: true };
        to.port_mut(to_port).expect("the port exists").record = Some(record);
    }

    #[test]
    fn a_mixer_is_shown_over_its_terrain_unless_an_underlay_is_set() {
        let base = node("Mountain", 1);
        let relief = node("Perlin", 2);
        let mut colours = node("SatelliteMap", 3);
        wire(&mut colours, "In", 1);
        let mut mixer = node("Mixer", 4);
        wire(&mut mixer, "In", 3);
        wire(&mut mixer, "Terrain", 2);
        let nodes: IndexMap<i32, NodeData> = [base, relief, colours, mixer].into_iter().map(|node| (node.id, node)).collect();
        let terrain = Terrain { id: String::new(), nodes, width: 5000.0, height: 2500.0, extra: Default::default() };
        let mut engine = Engine::new(Arc::new(crate::registry()));
        let resolution = Resolution::square(64);
        engine.build(&terrain, 4, resolution);

        let relief_map = engine.cache.get(2, "Out").expect("the Terrain input built");
        let shown = engine.custom_underlay(&terrain, 4, resolution).expect("the Mixer names its Terrain input");
        assert!(Arc::ptr_eq(&shown, &relief_map));

        // The editor's underlay comes first.
        engine.underlay = 1;
        let base_map = engine.cache.get(1, "Out").expect("the base built");
        let shown = engine.custom_underlay(&terrain, 4, resolution).expect("the Mixer names the underlay");
        assert!(Arc::ptr_eq(&shown, &base_map));

        // Other types name none.
        assert!(engine.custom_underlay(&terrain, 3, resolution).is_none());
    }

    #[test]
    fn a_mixer_without_a_terrain_names_nothing() {
        let base = node("Mountain", 1);
        let mut colours = node("SatelliteMap", 2);
        wire(&mut colours, "In", 1);
        let mut mixer = node("Mixer", 3);
        wire(&mut mixer, "In", 2);
        let nodes: IndexMap<i32, NodeData> = [base, colours, mixer].into_iter().map(|node| (node.id, node)).collect();
        let terrain = Terrain { id: String::new(), nodes, width: 5000.0, height: 2500.0, extra: Default::default() };
        let mut engine = Engine::new(Arc::new(crate::registry()));
        let resolution = Resolution::square(64);
        engine.build(&terrain, 3, resolution);
        assert!(engine.custom_underlay(&terrain, 3, resolution).is_none());
    }
}
