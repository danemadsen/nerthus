//! What the loop nodes do to the build around their own builds
//! (`LoopBegin.Build`, `LoopEnd.Build`), which needs the engine.
//!
//! A loop is `LoopBegin → … → LoopEnd`. The graph builds once as usual;
//! then `LoopEnd` runs `Loops - 1` more passes, each feeding its input back
//! out of `LoopBegin`, forgetting everything downstream of `LoopBegin` and
//! rebuilding its own upstream. The nodes on each loop node's `Processes`
//! port run around that: `LoopBegin` resets `Var` nodes to their defaults
//! (and rebuilds other nodes), `LoopEnd` runs them before every pass (a
//! `Var` there counts the passes).
//!
//! A reset `Var` holds its default as text, so a `Var` modifying a float
//! or integer variable fails on the next pass and ends the loop
//! (`LoopBegin` is still marked "under construction").
//! `MutateSeeds` adds the build's mutation to the seeds
//! of the loop's nodes before every pass after the first; it is 0, and so
//! changes nothing, unless the build is a mutation (`nerthus-build --seed`).

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::Value;

use crate::automation::VariableValue;
use crate::graph::{read_output, ApplicationMode, Engine, NodeError, NodeState, StateOverride};
use crate::map::{Map, Resolution};
use crate::project::{NodeData, Terrain};

/// Every node downstream of `node` (through any input),
/// not including it.
fn downstream(terrain: &Terrain, node: i32) -> Vec<i32> {
    let mut found = Vec::new();
    let mut stack = vec![node];
    let mut seen = HashSet::from([node]);
    while let Some(current) = stack.pop() {
        found.push(current);
        for candidate in terrain.nodes.values() {
            let feeds = candidate
                .ports
                .iter()
                .filter(|port| port.is_input())
                .any(|port| port.record.as_ref().is_some_and(|record| record.from == current));
            if feeds && seen.insert(candidate.id) {
                stack.push(candidate.id);
            }
        }
    }
    found.retain(|id| *id != node);
    found
}

/// The node feeding one of `node`'s ports (`port.GetSourceNode()`).
fn source_of(terrain: &Terrain, node: &NodeData, port: &str) -> Option<i32> {
    let record = node.port(port)?.record.as_ref()?;
    terrain.nodes.contains_key(&record.from).then_some(record.from)
}

/// The nodes feeding a `Combine`'s inputs, in port order (`Combine.Ins`).
fn combined_sources(terrain: &Terrain, combine: &NodeData) -> Vec<i32> {
    let inputs = combine.ports.iter().filter(|port| port.is_input());
    let sources = inputs.filter_map(|port| port.record.as_ref().map(|record| record.from));
    sources.filter(|id| terrain.nodes.contains_key(id)).collect()
}

impl Engine {
    /// Whether a `LoopEnd` loops in this application (`Primed` in the
    /// editor, `PrimedBuild` on the command line).
    fn loop_runs(&self, loop_end: &NodeData) -> bool {
        let flag = |name: &str| loop_end.values.get(name).and_then(Value::as_bool).unwrap_or(false);
        match self.application_mode {
            ApplicationMode::Ui => flag("Primed"),
            ApplicationMode::Swarm | ApplicationMode::SwarmAutomated => flag("PrimedBuild"),
            _ => true,
        }
    }

    /// Forgets a node's results (a baked node keeps them).
    pub(crate) fn unbuild(&mut self, terrain: &Terrain, id: i32) {
        let Some(node) = terrain.nodes.get(&id) else {
            return;
        };
        let state_override = StateOverride::of(node);
        if state_override == StateOverride::Baked {
            return;
        }
        if state_override != StateOverride::Bypassed {
            self.states.insert(id, NodeState::Unbuilt);
        }
        self.cache.remove_node(id);
        self.errors.remove(&id);
    }

    /// `node.Build()` called directly rather than through the build loop:
    /// no validity checks or bindings and no change of state; its maps
    /// replace its cached ones and its changes apply.
    pub(crate) fn build_directly(&mut self, terrain: &Terrain, id: i32, resolution: Resolution) -> Result<(), NodeError> {
        let Some(node) = terrain.nodes.get(&id) else {
            return Ok(());
        };
        let current = self.current_node(node);
        let outputs = self.run_node(terrain, &current, resolution)?;
        self.apply_outputs(id, outputs);
        Ok(())
    }

    /// `Var.SetToDefaultValue`: a modifying `Var` puts its default value
    /// (text) into its variable.
    fn reset_variable(&mut self, var: &NodeData) -> Result<(), NodeError> {
        let mode = var.values.get("EditMode").and_then(Value::as_str).unwrap_or("Modify");
        if mode != "Modify" {
            return Ok(());
        }
        let name = var.values.get("VariableName").and_then(Value::as_str).unwrap_or_default();
        let default = var.values.get("DefaultValue").and_then(Value::as_str).unwrap_or_default();
        let Some(variable) = self.automation.variables.get_mut(name) else {
            return Err(NodeError::Failed(format!("The given key '{name}' was not present in the dictionary.")));
        };
        variable.value = VariableValue::Text(default.to_string());
        Ok(())
    }

    /// One node on `LoopBegin`'s `Processes` port: a `Var` is reset, any
    /// other node forgotten and rebuilt directly.
    fn restart_process(&mut self, terrain: &Terrain, id: i32, resolution: Resolution) -> Result<(), NodeError> {
        let Some(node) = terrain.nodes.get(&id) else {
            return Ok(());
        };
        if node.type_name == "Var" {
            let current = self.current_node(node).into_owned();
            return self.reset_variable(&current);
        }
        self.unbuild(terrain, id);
        self.build_directly(terrain, id, resolution)
    }

    /// `LoopBegin.Build`, before it passes its input on: restarts the
    /// node on its `Processes` port (each input of a `Combine` there).
    pub(crate) fn run_loop_begin_processes(&mut self, terrain: &Terrain, loop_begin: &NodeData, resolution: Resolution) -> Result<(), NodeError> {
        let Some(process) = source_of(terrain, loop_begin, "Processes") else {
            return Ok(());
        };
        let process_node = &terrain.nodes[&process];
        if process_node.type_name == "Combine" {
            for source in combined_sources(terrain, process_node) {
                self.restart_process(terrain, source, resolution)?;
            }
            return Ok(());
        }
        self.restart_process(terrain, process, resolution)
    }

    /// The node on `LoopEnd`'s `Processes` port built directly (each input
    /// of a `Combine` there).
    fn run_loop_end_processes(&mut self, terrain: &Terrain, process: Option<i32>, resolution: Resolution) -> Result<(), NodeError> {
        let Some(process) = process else {
            return Ok(());
        };
        let process_node = &terrain.nodes[&process];
        if process_node.type_name == "Combine" {
            for source in combined_sources(terrain, process_node) {
                self.build_directly(terrain, source, resolution)?;
            }
            return Ok(());
        }
        self.build_directly(terrain, process, resolution)
    }

    /// The node's seeds moved on by the build's mutation, as
    /// changes the build makes to its members.
    fn mutate_seeds(&mut self, node: &NodeData) {
        let current = self.current_node(node).into_owned();
        for (member, value) in crate::mutation::mutated_seeds(&current, self.mutation) {
            self.member_updates.push((node.id, member, value));
        }
    }

    /// `loopBegin.Commit(map)`: the map through `LoopBegin`'s commit
    /// (clamp, masking, modifiers) into its cached outputs.
    fn commit_as(&mut self, terrain: &Terrain, id: i32, map: &Map, resolution: Resolution) -> Result<(), NodeError> {
        let node = self.current_node(&terrain.nodes[&id]).into_owned();
        let committed = self.commit_for(terrain, &node, map.clone(), resolution)?;
        for (port, map) in committed {
            self.cache.store(id, &port, Arc::new(map));
        }
        Ok(())
    }

    /// `LoopEnd.Build`'s loop, run before the build passes its (by then
    /// final) input on.
    pub(crate) fn run_loop(&mut self, terrain: &Terrain, loop_end: &NodeData, resolution: Resolution) {
        let id = loop_end.id;
        if self.loops_in_progress.contains(&id) || !self.loop_runs(loop_end) {
            return;
        }
        let order = Engine::build_order(terrain, id);
        let loop_begin = order.iter().rev().copied().find(|node| terrain.nodes[node].type_name == "LoopBegin");
        let Some(loop_begin) = loop_begin else {
            return;
        };
        let loops = loop_end.values.get("Loops").and_then(Value::as_i64).unwrap_or(2);
        let mutate_seeds = loop_end.values.get("MutateSeeds").and_then(Value::as_bool).unwrap_or(true);
        let process = source_of(terrain, loop_end, "Processes");
        let input = loop_end.port("In").and_then(|port| port.record.clone());

        self.loops_in_progress.insert(id);
        for _ in 1..loops {
            // Any failure ends the loop (it is caught), keeping what the
            // passes so far made.
            if self.run_loop_end_processes(terrain, process, resolution).is_err() {
                break;
            }
            let fed_back = input.as_ref().and_then(|record| read_output(terrain, &self.cache, &self.states, record.from, &record.from_port));
            let Some(fed_back) = fed_back else {
                break;
            };
            if self.commit_as(terrain, loop_begin, &fed_back, resolution).is_err() {
                break;
            }
            for node in downstream(terrain, loop_begin) {
                if Some(node) == process {
                    continue;
                }
                if mutate_seeds && self.mutation != 0 {
                    self.mutate_seeds(&terrain.nodes[&node]);
                }
                self.unbuild(terrain, node);
            }
            self.build(terrain, id, resolution);
        }
        self.loops_in_progress.remove(&id);
    }
}
