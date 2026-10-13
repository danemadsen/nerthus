//! Routing: Chokepoint, Gate, Route, Compare, Switch, Reseed, the loop
//! nodes and the macro nodes.

use indexmap::IndexMap;
use nerthus_engine::automation::Variable;
use nerthus_engine::macros::MacroAsset;
use nerthus_engine::{check_required_ports, ApplicationMode, BuildContext, Map, NodeBuilder, NodeError, RenderIntent};
use rayon::prelude::*;
use serde_json::Value;

use crate::colour;
use crate::combiner;
use crate::histogram;
use crate::masking;
use crate::maths;

/// Chokepoint and Gate pass their input on unchanged; they exist to shape
/// the graph (build boundaries, switching points).
pub struct PassThrough;

impl NodeBuilder for PassThrough {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        context.commit((*input).clone())
    }
}

/// Passes on the input selected by `Choice` (0 is `In`, then `Input2`…).
pub struct Route;

impl NodeBuilder for Route {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let choice = context.i32("Choice").max(0) as usize;
        let names = context.input_names();
        let Some(port) = names.get(choice).map(|name| name.to_string()) else {
            return Err(NodeError::Failed(format!("Route has no input {choice}")));
        };
        let input = context.require_input(&port)?;
        context.commit((*input).clone())
    }
}

/// Compare: shows two maps split side by side (A/B) or their difference.
/// Without `In2` it compares against what the input's own source started
/// from.
pub struct Compare;

impl NodeBuilder for Compare {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let first = context.input("In");
        let second = context.input("In2").or_else(|| context.upstream_input("In"));
        let (a, b) = if context.bool("Swap") { (second, first) } else { (first, second) };
        let (Some(a), Some(b)) = (a, b) else {
            return Err(NodeError::Validation("Required port not connected.".to_string()));
        };

        if context.enum_name("Mode") == "Difference" {
            let difference = combiner::difference(&a, &b, 1.0);
            context.commit_port("InternalMask", difference, RenderIntent::Default);
            return context.commit((*a).clone());
        }

        let size = context.resolution.x;
        let left = (size as f32 * context.f32("Comparison")) as i32;
        let start = left.max(0) as usize;
        let perpendicular = context.bool("Perpendicular");
        let mut split = Map::new(context.resolution, 1);
        split.data_mut().par_chunks_mut(size).enumerate().for_each(|(row, values)| {
            if perpendicular {
                if row >= start {
                    values.fill(1.0);
                }
                return;
            }
            for value in values.iter_mut().skip(start) {
                *value = 1.0;
            }
        });
        let combined = masking::mask(&a, &b, &split);
        context.commit_port("InternalMask", split, RenderIntent::Default);
        context.commit(combined)
    }

    /// `Compare.GetCustomColorMap`: in Difference mode the difference of the
    /// two inputs, raw towards Comparison 0, levelled at 0.5 and equalized
    /// towards 1, from light grey to red; otherwise the split in light grey
    /// and red when Colorized, else plain light grey.
    fn custom_colour_map(&self, context: &BuildContext) -> Option<Map> {
        if context.enum_name("Mode") == "Difference" {
            let first = context.input("In");
            let second = context.input("In2").or_else(|| context.upstream_input("In"));
            let (a, b) = if context.bool("Swap") { (second?, first?) } else { (first?, second?) };
            let difference = combiner::difference(&a, &b, 1.0);
            let levelled = maths::autolevel(difference.clone());
            let comparison = context.f32("Comparison");
            let shown = if comparison <= 0.5 {
                combiner::blend_maps(&difference, &levelled, saturate(comparison * 2.0))
            } else {
                let equalized = histogram::equalize(difference.clone());
                combiner::blend_maps(&levelled, &equalized, saturate((comparison - 0.5) * 2.0))
            };
            return Some(colour::colorize(&shown, &COMPARE_COLOURS));
        }
        if context.bool("Colorized") {
            let split = context.own_output("InternalMask")?;
            return Some(colour::colorize(&split, &COMPARE_COLOURS));
        }
        let resolution = context.resolution;
        let plane = vec![0.8; resolution.area()];
        Some(Map::from_channels(resolution, vec![plane.clone(), plane.clone(), plane]))
    }
}

/// Compare's viewport colours: light grey to red.
const COMPARE_COLOURS: [[f32; 3]; 2] = [[0.8, 0.8, 0.8], [0.6, 0.2, 0.2]];

/// Clamps to [0, 1], NaN passing through.
fn saturate(value: f32) -> f32 {
    if value < 0.0 {
        return 0.0;
    }
    if value > 1.0 {
        return 1.0;
    }
    value
}

/// Switch: passes on `In`, or `Alternate` when `SwitchRoute` is on, with
/// the chosen map's render intent. Both inputs are required.
pub struct Switch;

impl NodeBuilder for Switch {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        // The message names `In` whichever input is missing.
        let (Some(main), Some(alternate)) = (context.input("In"), context.input("Alternate")) else {
            return Err(NodeError::Validation("Required port [In] not connected.".to_string()));
        };
        let chosen = if context.bool("SwitchRoute") { alternate } else { main };
        context.render_as(chosen.info.render_intent);
        context.commit((*chosen).clone())
    }
}

/// Reseed: rebuilds the node feeding it with this node's seed and passes
/// on that result. The variant is built under the original's id, so the
/// original's cached results are replaced too.
pub struct Reseed;

impl NodeBuilder for Reseed {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let Some(source) = context.source_node("In") else {
            return Ok(());
        };
        if !source.values.contains_key("Seed") {
            return Err(NodeError::Failed("Object reference not set to an instance of an object.".to_string()));
        }
        let seed = Value::from(context.i32("Seed"));
        let overrides = [("Seed", seed)];
        // The cache is then read under the original's id: without a new
        // `Out`, the original's stays.
        let variant = context.build_source_variant("In", &overrides)?;
        let result = match variant {
            Some(map) => map,
            None => match context.source_output("In", "Out") {
                Some(map) => (*map).clone(),
                None => return Ok(()),
            },
        };
        context.commit(result)
    }
}

/// LoopBegin: passes its input on. The engine first restarts the nodes on
/// its `Processes` port and, while a loop runs, feeds each pass's result
/// back out of it.
pub struct LoopBegin;

impl NodeBuilder for LoopBegin {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        context.commit((*input).clone())
    }
}

/// LoopEnd: passes its input on once the engine has run the loop (when it
/// is primed for this application). Its own passes inside the loop commit
/// nothing, and an unprimed command-line build leaves it without output.
pub struct LoopEnd;

impl NodeBuilder for LoopEnd {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        if context.loop_in_progress() {
            return Ok(());
        }
        let is_command_line = matches!(context.application_mode(), ApplicationMode::Swarm | ApplicationMode::SwarmAutomated);
        if is_command_line && !context.bool("PrimedBuild") {
            return Ok(());
        }
        let input = context.require_input("In")?;
        context.commit((*input).clone())
    }
}

/// The variable values a `Macro` node runs its macro with
/// (`Macro.SetupMacro`): its own, unless they are missing or not the
/// macro's variables (by name, in order), when the macro's are used.
fn macro_variables(saved: &Value, asset: &MacroAsset) -> IndexMap<String, Variable> {
    let mut variables = IndexMap::new();
    if let Some(entries) = saved.as_object() {
        for (name, variable) in entries.iter().filter(|(name, _)| !name.starts_with('$')) {
            variables.insert(name.clone(), Variable::from_json(variable));
        }
    }
    let own_names = variables.keys();
    let macro_names = asset.asset.automation.variables.keys();
    if variables.is_empty() || !own_names.eq(macro_names) {
        return asset.asset.automation.variables.clone();
    }
    variables
}

/// Macro: runs an installed macro's graph (`Macro.Build`). Its inputs go to
/// the macro's input ports by name and each output port's map comes out
/// of the node's port of the same name. Inside a macro's own graph it does
/// nothing (macros are skipped while a component asset is current).
pub struct Macro;

impl NodeBuilder for Macro {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        if context.building_component() {
            return Ok(());
        }
        let missing = context.node.ports.iter().any(|port| port.is_input() && port.is_required() && port.record.is_none());
        if missing {
            return Err(NodeError::Validation("Required port [In] not connected.".to_string()));
        }
        let input_ports: Vec<(String, bool)> = context.node.ports.iter().filter(|port| port.is_input()).map(|port| (port.name.clone(), port.is_required())).collect();
        let mut inputs = Vec::new();
        for (name, required) in input_ports {
            match context.input(&name) {
                Some(map) => inputs.push((name, map)),
                None if required => return Err(NodeError::Failed("Incoming data was null!".to_string())),
                None => {}
            }
        }

        let macro_id = context.string("MacroId").to_string();
        let Some(asset) = context.macro_asset(&macro_id).cloned() else {
            return Err(NodeError::Failed("Node failed Macro Asset is empty!".to_string()));
        };
        let variables = macro_variables(context.value("Variables"), &asset);
        let results = context.execute_macro(&asset, &variables, &inputs)?;
        for (name, map) in results {
            let has_port = context.node.port(&name).is_some_and(|port| port.is_output());
            if !has_port {
                return Err(NodeError::Failed("Sequence contains no matching element".to_string()));
            }
            let intent = map.info.render_intent;
            context.set_port_intent(&name, intent);
            context.commit_port(&name, map, intent);
        }
        Ok(())
    }
}

/// MacroPort: one of a macro's inputs or outputs. Passes its input on (or
/// its fallback map when nothing arrives), shown as its display intent.
/// An optional port is never invalid.
pub struct MacroPort;

impl NodeBuilder for MacroPort {
    fn check_validity(&self, context: &BuildContext) -> Result<(), NodeError> {
        if context.bool("IsOptional") {
            return Ok(());
        }
        check_required_ports(context)
    }

    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let given = context.input("In").map(|map| (*map).clone());
        let map = match given {
            Some(map) => Some(map),
            None => match context.enum_name("Fallback") {
                "Black" => Some(Map::filled(context.resolution, 0.0)),
                "White" => Some(Map::filled(context.resolution, 1.0)),
                "Grey" => Some(Map::filled(context.resolution, 0.5)),
                _ => None,
            },
        };
        let Some(mut map) = map else {
            return Ok(());
        };
        let intent = match context.enum_name("Display") {
            "Mask" => RenderIntent::Mask,
            "Color" => RenderIntent::Color,
            _ => RenderIntent::Heightfield,
        };
        map.info.render_intent = intent;
        context.render_as(intent);
        context.commit(map)
    }
}

/// MacroDefinition: the metadata node of a macro's own graph; building it
/// does nothing.
pub struct MacroDefinition;

impl NodeBuilder for MacroDefinition {
    fn build(&self, _context: &mut BuildContext) -> Result<(), NodeError> {
        Ok(())
    }
}
