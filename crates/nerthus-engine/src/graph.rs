//! Graph evaluation: a node is built after
//! everything upstream of it (depth-first, inputs in port order), each node
//! commits its maps to a cache keyed by node and port, and downstream nodes
//! read their inputs from that cache.

use std::any::Any;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use indexmap::IndexMap;
use serde_json::Value;
use thiserror::Error;

use crate::automation::{Automation, ExpressionContext, Variable, VariableValue};
use crate::macros::{run_macro, MacroAsset, MacroLibrary};
use crate::catalogue::{Catalogue, NodeSpec};
use crate::map::{Map, RenderIntent, Resolution};
use crate::project::{ModifierData, NodeData, Terrain};

/// Why a node did not produce its outputs.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum NodeError {
    /// A `NodeValidationException` (missing or bad required inputs).
    #[error("{0}")]
    Validation(String),
    /// A `NodeFailedException`: the node reported a problem itself.
    #[error("{0}")]
    Failed(String),
    #[error("cancelled")]
    Cancelled,
    #[error("the {0} node is not implemented yet")]
    Unimplemented(String),
}

/// `NodeState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeState {
    Unbuilt,
    Built,
    Error,
    Bypassed,
    Baked,
}

/// `StateOverride`, from the node's saved `StateOverride` member.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateOverride {
    None,
    Bypassed,
    Baked,
}

impl StateOverride {
    pub fn of(node: &NodeData) -> Self {
        match node.base.get("StateOverride").and_then(Value::as_str) {
            Some("Bypassed") => Self::Bypassed,
            Some("Baked") => Self::Baked,
            _ => Self::None,
        }
    }
}

/// Engine-wide preferences that change build results.
#[derive(Clone, Debug, Default)]
pub struct EnginePreferences {
    /// `Preferences.AllowValuesBeyondRange`: skip the [0, 1] clamp on commit.
    pub allow_values_beyond_range: bool,
}

/// A node implementation.
pub trait NodeBuilder: Send + Sync {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError>;

    /// `Node.CheckValidity`: what stops the build before it starts (by
    /// default, required ports that are unconnected or deliver no map at
    /// the build resolution).
    fn check_validity(&self, context: &BuildContext) -> Result<(), NodeError> {
        check_required_ports(context)
    }

    /// `Node.AddNewPort`: adds the next input of a node with a growing
    /// list of inputs and returns its name, or `None` when there is no room
    /// (or the node has a fixed set of ports).
    fn add_new_port(&self, _node: &mut NodeData) -> Option<String> {
        None
    }

    /// `Node.GetCustomColorMap`: for a type with `CustomColorizationAttribute`,
    /// the colours the viewport shows instead of the previewed port's,
    /// made from what the node last built. `None` is a null colour map.
    fn custom_colour_map(&self, _context: &BuildContext) -> Option<Map> {
        None
    }

    /// `Node.GetCustomUnderlay`: for a type with `CustomUnderlayAttribute`,
    /// the node whose main output the viewport shows this node over.
    /// `None` names no node.
    fn custom_underlay(&self, _context: &BuildContext) -> Option<i32> {
        None
    }
}

/// A modifier implementation (`Modifier.Process`).
pub trait ModifierBuilder: Send + Sync {
    fn process(&self, modifier: &ModifierData, map: Map, context: &BuildContext) -> Result<Map, NodeError>;
}

/// Node and modifier implementations by type name.
#[derive(Default)]
pub struct Registry {
    nodes: HashMap<String, Box<dyn NodeBuilder>>,
    modifiers: HashMap<String, Box<dyn ModifierBuilder>>,
}

impl Registry {
    pub fn add_node(&mut self, type_name: &str, builder: impl NodeBuilder + 'static) {
        self.nodes.insert(type_name.to_string(), Box::new(builder));
    }

    pub fn add_modifier(&mut self, type_name: &str, builder: impl ModifierBuilder + 'static) {
        self.modifiers.insert(type_name.to_string(), Box::new(builder));
    }

    pub fn node(&self, type_name: &str) -> Option<&dyn NodeBuilder> {
        self.nodes.get(type_name).map(Box::as_ref)
    }

    pub fn modifier(&self, type_name: &str) -> Option<&dyn ModifierBuilder> {
        self.modifiers.get(type_name).map(Box::as_ref)
    }

    pub fn implements(&self, type_name: &str) -> bool {
        self.nodes.contains_key(type_name)
    }
}

/// Maps committed by built nodes, keyed by node id and port name.
#[derive(Default)]
pub struct Cache {
    maps: HashMap<i32, HashMap<String, Arc<Map>>>,
}

impl Cache {
    pub fn get(&self, node: i32, port: &str) -> Option<Arc<Map>> {
        self.maps.get(&node)?.get(port).cloned()
    }

    pub fn store(&mut self, node: i32, port: &str, map: Arc<Map>) {
        self.maps.entry(node).or_default().insert(port.to_string(), map);
    }

    pub fn remove_node(&mut self, node: i32) {
        self.maps.remove(&node);
    }

    pub fn ports(&self, node: i32) -> impl Iterator<Item = (&String, &Arc<Map>)> {
        self.maps.get(&node).into_iter().flatten()
    }

    /// The nodes with cached maps.
    pub fn nodes(&self) -> impl Iterator<Item = i32> + '_ {
        self.maps.keys().copied()
    }

    /// The bytes the cached maps hold, each map counted once however many
    /// ports share it (the status bar's memory figure).
    pub fn bytes(&self) -> usize {
        let mut seen = std::collections::HashSet::new();
        let mut total = 0;
        for map in self.maps.values().flat_map(|ports| ports.values()) {
            if seen.insert(Arc::as_ptr(map)) {
                total += map.byte_size();
            }
        }
        total
    }
}

/// Everything a node sees while it builds.
pub struct BuildContext<'a> {
    pub node: &'a NodeData,
    pub spec: &'static NodeSpec,
    pub resolution: Resolution,
    pub preferences: &'a EnginePreferences,
    terrain: &'a Terrain,
    cache: &'a Cache,
    states: &'a HashMap<i32, NodeState>,
    port_intents: &'a HashMap<i32, PortIntents>,
    underlay: i32,
    registry: &'a Arc<Registry>,
    committed: Vec<(String, Map)>,
    intent_overrides: HashMap<String, RenderIntent>,
    member_updates: Vec<(String, Value)>,
    /// Maps this build stores for other nodes (a Reseed variant's results
    /// land under the original node's id).
    side_commits: Vec<(i32, String, Map)>,
    automation: &'a Automation,
    /// Variables this build changes (the `Var` node).
    variable_updates: Vec<(String, VariableValue)>,
    /// Member changes earlier builds made that the caller has not applied
    /// to the project yet, as (node, member, value).
    pending_members: &'a [(i32, String, Value)],
    application_mode: ApplicationMode,
    /// Whether this node is a `LoopEnd` whose loop is running (its own
    /// builds inside the loop commit nothing).
    loop_in_progress: bool,
    macros: &'a MacroLibrary,
    /// Whether the graph building is a macro's (a component asset).
    building_component: bool,
    project_path: Option<&'a Path>,
    node_memory: &'a NodeMemory,
    /// The build's mutation, which a macro's graph builds with too.
    mutation: i32,
}

/// What nodes keep between their builds (a node object's own fields,
/// such as `FileSequence`'s loaded frames), by node.
pub(crate) type NodeMemory = Mutex<HashMap<i32, Arc<dyn Any + Send + Sync>>>;

/// The intent each output port of a node was left with by its last build
/// (`Port.RenderIntent`, which a port keeps), by port name.
pub type PortIntents = HashMap<String, RenderIntent>;

impl<'a> BuildContext<'a> {
    /// The node's id (`Node.Id`), which some nodes use as a seed.
    pub fn node_id(&self) -> i32 {
        self.node.id
    }

    /// The node's group (`Node.Group`, 0 for none).
    pub fn node_group(&self) -> i32 {
        self.node.group()
    }

    /// Terrain width in metres (`Terrain.Width`).
    pub fn terrain_width(&self) -> f32 {
        self.terrain.width
    }

    /// Terrain height in metres (`Terrain.Height`).
    pub fn terrain_height(&self) -> f32 {
        self.terrain.height
    }

    /// Names of the input ports in port order (`Node.Ins`).
    pub fn input_names(&self) -> Vec<&str> {
        self.node.ports.iter().filter(|port| port.is_input()).map(|port| port.name.as_str()).collect()
    }

    /// Changes the intent maps on `port` carry for this build
    /// (`Port.RenderIntent = …`).
    pub fn set_port_intent(&mut self, port: &str, intent: RenderIntent) {
        self.intent_overrides.insert(port.to_string(), intent);
    }

    /// `Out.RenderIntent` as the build has left it.
    pub fn out_intent(&self) -> RenderIntent {
        self.declared_intent("Out")
    }

    /// `Node.RenderIntentOverride` (the user's choice in the editor).
    pub fn intent_override(&self) -> RenderIntent {
        let name = self.node.base.get("RenderIntentOverride").and_then(Value::as_str);
        name.and_then(RenderIntent::from_name).unwrap_or_default()
    }

    /// Whether the node renders as a mask (`Out.RenderIntent == Mask ||
    /// RenderIntentOverride == Mask`), as the masking modifiers ask.
    pub fn renders_as_mask(&self) -> bool {
        self.out_intent() == RenderIntent::Mask || self.intent_override() == RenderIntent::Mask
    }

    /// `Node.IsHeightfield` for the node being built.
    pub fn is_heightfield_now(&self) -> bool {
        let intent_override = self.intent_override();
        let out_intent = self.out_intent();
        let explicit = matches!(intent_override, RenderIntent::Default | RenderIntent::Heightfield);
        if explicit && out_intent == RenderIntent::Heightfield {
            return true;
        }
        if intent_override != RenderIntent::Default || out_intent != RenderIntent::Default {
            return false;
        }
        let classification = self.spec.classification.as_deref().unwrap_or_default();
        !matches!(classification, "Mask" | "Colorizer")
    }

    /// `Node.RenderAsHeightfield/Mask/Color()`: the intent of the main output.
    pub fn render_as(&mut self, intent: RenderIntent) {
        self.set_port_intent("Out", intent);
    }

    /// Records a change the build makes to one of the node's own saved
    /// members (`RenderIntentOverride = …` inside `Build`). The engine
    /// hands these out through [`Engine::take_member_updates`].
    pub fn update_member(&mut self, name: &str, value: Value) {
        self.member_updates.push((name.to_string(), value));
    }

    /// `Node.SetIntent(intent, main: false)`: the intent of every output
    /// port except the primary one.
    pub fn render_secondary_as(&mut self, intent: RenderIntent) {
        let names: Vec<String> = self
            .node
            .ports
            .iter()
            .filter(|port| port.is_output() && !port.is_primary())
            .map(|port| port.name.clone())
            .collect();
        for name in names {
            self.intent_overrides.insert(name, intent);
        }
    }

    /// Gives every port the same intent (`Ports.ForEach(x => x.RenderIntent = …)`).
    pub fn set_all_port_intents(&mut self, intent: RenderIntent) {
        let names: Vec<String> = self.node.ports.iter().map(|port| port.name.clone()).collect();
        for name in names {
            self.intent_overrides.insert(name, intent);
        }
    }

    /// `Texturize.GetAccumulation`: the maximum of `port` over every built
    /// (or baked) node that accumulates `kind` (`Snow`, `Water`, `Shore`,
    /// `River`, `Debris`, `Trees`), zero where none does. Rivers report
    /// their water mask as `rivers_port` when given.
    pub fn accumulation(&self, kind: &str, port: &str, rivers_port: Option<&str>, restrict_group: i32) -> Map {
        let catalogue = Catalogue::get();
        let mut result = Map::new(self.resolution, 1);
        let mut ids: Vec<&i32> = self.terrain.nodes.keys().collect();
        ids.sort();
        for id in ids {
            let node = &self.terrain.nodes[id];
            let state = self.states.get(id).copied().unwrap_or(NodeState::Unbuilt);
            if !matches!(state, NodeState::Built | NodeState::Baked) {
                continue;
            }
            let Some(spec) = catalogue.node(&node.type_name) else {
                continue;
            };
            if !spec.accumulates(kind) {
                continue;
            }
            if restrict_group > 0 && node.group() != restrict_group {
                continue;
            }
            let port_name = match rivers_port {
                Some(rivers) if node.type_name.ends_with(".Rivers") || node.type_name == "Rivers" => rivers,
                _ => port,
            };
            if let Some(map) = self.cache.get(*id, port_name) {
                // `Combiner.Max(map, accumulation)`: the blend's `Max` mode.
                let first = map.channel(0);
                let maximum: Vec<f32> = result
                    .data()
                    .iter()
                    .zip(first)
                    .map(|(&current, &value)| if current > value { current } else { value })
                    .collect();
                result = Map::from_data(self.resolution, maximum);
            }
        }
        result
    }

    /// `Node.IsHeightfield` for a built node: its main output renders as a
    /// heightfield, or has no intent of its own and the node is not a
    /// mask or colour node (`NodeMetadata.IsSoftData`).
    fn is_heightfield(&self, node: &NodeData) -> bool {
        let intent_override = node.base.get("RenderIntentOverride").and_then(Value::as_str);
        let intent_override = intent_override.and_then(RenderIntent::from_name).unwrap_or_default();
        let built_intents = self.port_intents.get(&node.id);
        let out_intent = built_intents.and_then(|intents| intents.get("Out")).copied().unwrap_or_default();
        let explicit = matches!(intent_override, RenderIntent::Default | RenderIntent::Heightfield);
        if explicit && out_intent == RenderIntent::Heightfield {
            return true;
        }
        if intent_override != RenderIntent::Default || out_intent != RenderIntent::Default {
            return false;
        }
        let spec = Catalogue::get().node(&node.type_name);
        let classification = spec.and_then(|spec| spec.classification.as_deref()).unwrap_or_default();
        !matches!(classification, "Mask" | "Colorizer")
    }

    /// The editor's chosen underlay node (`TerrainState.UnderlayNode`), 0
    /// for none.
    pub fn underlay_node(&self) -> i32 {
        self.underlay
    }

    /// The heightfield a colour or mask node is shown over
    /// and falls back on. A node set to render as a heightfield gets its
    /// own last result; otherwise the editor's chosen underlay node, else
    /// the nearest built heightfield up the chain of `In` ports, else a
    /// flat zero map.
    pub fn underlay_heightfield(&self) -> Option<Arc<Map>> {
        let intent_override = self.node.base.get("RenderIntentOverride").and_then(Value::as_str);
        if intent_override == Some("Heightfield") {
            return self.cache.get(self.node.id, "Out");
        }
        if self.terrain.nodes.contains_key(&self.underlay) {
            return read_output(self.terrain, self.cache, self.states, self.underlay, "Out");
        }

        // Up the `In` chain, nearest first.
        let mut visited = HashSet::new();
        visited.insert(self.node.id);
        let mut current = self.node;
        while let Some(record) = current.ports.first().and_then(|port| port.record.as_ref()) {
            if !visited.insert(record.from) {
                break;
            }
            let Some(parent) = self.terrain.nodes.get(&record.from) else {
                break;
            };
            let state = self.states.get(&parent.id).copied().unwrap_or(NodeState::Unbuilt);
            let ignored = parent.base.get("IgnoreUnderlay").and_then(Value::as_bool).unwrap_or(false);
            if state == NodeState::Built && !ignored && self.is_heightfield(parent) {
                return read_output(self.terrain, self.cache, self.states, parent.id, "Out");
            }
            current = parent;
        }
        Some(Arc::new(Map::new(self.resolution, 1)))
    }

    /// The map feeding an input port, if connected and built
    /// (`Port.GetMap()`).
    pub fn input(&self, port: &str) -> Option<Arc<Map>> {
        let record = self.node.port(port)?.record.as_ref()?;
        read_output(self.terrain, self.cache, self.states, record.from, &record.from_port)
    }

    /// Whether an input port has a connection (`Port.IsConnected`).
    pub fn is_connected(&self, port: &str) -> bool {
        self.node.port(port).and_then(|port| port.record.as_ref()).is_some()
    }

    /// A required input (`NodeValidationException` when missing).
    pub fn require_input(&self, port: &str) -> Result<Arc<Map>, NodeError> {
        self.input(port).ok_or_else(|| NodeError::Validation(format!("Required port [{port}] not connected.")))
    }

    /// Whether `port` is fed by its source node's main output
    /// (`port.Connection.Target.IsPrimary()`).
    pub fn source_is_primary(&self, port: &str) -> bool {
        let Some(record) = self.node.port(port).and_then(|port| port.record.as_ref()) else {
            return false;
        };
        let source = self.terrain.nodes.get(&record.from);
        let source_port = source.and_then(|node| node.port(&record.from_port));
        source_port.is_some_and(|port| port.is_primary())
    }

    /// The asset's automation (variables, bindings, expressions).
    pub fn automation(&self) -> &Automation {
        self.automation
    }

    /// What kind of application is building.
    pub fn application_mode(&self) -> ApplicationMode {
        self.application_mode
    }

    /// Whether this `LoopEnd`'s loop is running, so this build is one of
    /// the loop's own passes.
    pub fn loop_in_progress(&self) -> bool {
        self.loop_in_progress
    }

    /// Whether this is a macro's graph building (`CurrentAsset.Type ==
    /// AssetType.Component`), where `Macro` nodes do nothing.
    pub fn building_component(&self) -> bool {
        self.building_component
    }

    /// What this node kept from its earlier builds.
    pub fn memory<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        let memory = self.node_memory.lock().ok()?;
        let kept = memory.get(&self.node.id)?.clone();
        kept.downcast::<T>().ok()
    }

    /// What the node feeding `port` kept from its builds.
    pub fn source_memory<T: Any + Send + Sync>(&self, port: &str) -> Option<Arc<T>> {
        let record = self.node.port(port)?.record.as_ref()?;
        let memory = self.node_memory.lock().ok()?;
        let kept = memory.get(&record.from)?.clone();
        kept.downcast::<T>().ok()
    }

    /// Keeps a value for this node's later builds — at once, so it stays
    /// even if this build then fails.
    pub fn remember<T: Any + Send + Sync>(&self, value: T) {
        if let Ok(mut memory) = self.node_memory.lock() {
            memory.insert(self.node.id, Arc::new(value));
        }
    }

    /// The project file being built, which relative file
    /// paths are relative to.
    pub fn project_path(&self) -> Option<&Path> {
        self.project_path
    }

    /// An installed macro by id (`Project.GetMacro`).
    pub fn macro_asset(&self, id: &str) -> Option<&Arc<MacroAsset>> {
        self.macros.get(id)
    }

    /// Runs a macro's graph with these variable values and inputs
    /// (`Macro.ExecuteMacro`), returning its outputs by name.
    pub fn execute_macro(&self, asset: &MacroAsset, variables: &IndexMap<String, Variable>, inputs: &[(String, Arc<Map>)]) -> Result<Vec<(String, Map)>, NodeError> {
        let mut engine = Engine::new(self.registry.clone());
        engine.preferences = self.preferences.clone();
        engine.application_mode = self.application_mode;
        engine.macros = self.macros.clone();
        engine.project_path = self.project_path.map(Path::to_path_buf);
        engine.mutation = self.mutation;
        run_macro(&mut engine, asset, variables, inputs, self.resolution)
    }

    /// What expressions see during this build.
    pub fn expression_context(&self) -> ExpressionContext {
        ExpressionContext { resolution: self.resolution, terrain_height: self.terrain.height }
    }

    /// The current value of a variable, with this build's own changes.
    pub fn variable_value(&self, name: &str) -> Option<VariableValue> {
        let updated = self.variable_updates.iter().rev().find(|(variable, _)| variable == name);
        if let Some((_, value)) = updated {
            return Some(value.clone());
        }
        self.automation.variables.get(name).map(|variable| variable.value.clone())
    }

    /// Changes a variable's value (`Variable.Value = …`); later nodes in
    /// the build see it.
    pub fn set_variable(&mut self, name: &str, value: VariableValue) {
        self.variable_updates.push((name.to_string(), value));
    }

    /// What this node's last build committed to `port`, for
    /// [`NodeBuilder::custom_colour_map`].
    pub fn own_output(&self, port: &str) -> Option<Arc<Map>> {
        self.cache.get(self.node.id, port)
    }

    pub fn source_output(&self, port: &str, output: &str) -> Option<Arc<Map>> {
        let record = self.node.port(port)?.record.as_ref()?;
        self.cache.get(record.from, output)
    }

    /// The node feeding `port` (`port.GetSourceNode()`), if connected.
    pub fn source_node(&self, port: &str) -> Option<&NodeData> {
        let record = self.node.port(port)?.record.as_ref()?;
        self.terrain.nodes.get(&record.from)
    }

    /// Builds a variant of the node feeding `port` (`Reseed`,
    /// `Node.Clone()`): a fresh node of the same type with the original's
    /// parameters and then `overrides`, no modifiers, and only its first
    /// port (`Node.In`) connected — to what the original's first port
    /// reads. It is built under the original's id, so its results replace
    /// the original's cached ones. Returns the variant's
    /// `Out`, or `None` when nothing feeds `port`.
    pub fn build_source_variant(&mut self, port: &str, overrides: &[(&str, Value)]) -> Result<Option<Map>, NodeError> {
        let terrain = self.terrain;
        let Some(record) = self.node.port(port).and_then(|port| port.record.as_ref()) else {
            return Ok(None);
        };
        let Some(original) = terrain.nodes.get(&record.from) else {
            return Ok(None);
        };
        let catalogue = Catalogue::get();
        let spec = catalogue.node(&original.type_name).ok_or_else(|| NodeError::Unimplemented(original.type_name.clone()))?;
        let builder = self.registry.node(&original.type_name).ok_or_else(|| NodeError::Unimplemented(original.type_name.clone()))?;

        let mut variant = NodeData::new(spec, original.id, 0);
        for parameter in spec.parameters.iter() {
            if let Some(value) = original.values.get(&parameter.name) {
                variant.values.insert(parameter.name.clone(), value.clone());
            }
        }
        // The node already holds what its own build bound to it.
        for (node, member, value) in self.pending_members {
            let is_parameter = spec.parameters.iter().any(|parameter| parameter.name == *member);
            if *node == original.id && is_parameter {
                variant.values.insert(member.clone(), value.clone());
            }
        }
        for (name, value) in overrides {
            variant.values.insert(name.to_string(), value.clone());
        }
        let original_first_input = original.ports.first().and_then(|port| port.record.clone());
        if let Some(first_port) = variant.ports.first_mut() {
            first_port.record = original_first_input;
        }

        let mut nested = BuildContext {
            node: &variant,
            spec,
            resolution: self.resolution,
            preferences: self.preferences,
            terrain,
            cache: self.cache,
            states: self.states,
            port_intents: self.port_intents,
            underlay: self.underlay,
            registry: self.registry,
            committed: Vec::new(),
            intent_overrides: HashMap::new(),
            member_updates: Vec::new(),
            side_commits: Vec::new(),
            automation: self.automation,
            variable_updates: Vec::new(),
            pending_members: self.pending_members,
            application_mode: self.application_mode,
            loop_in_progress: false,
            macros: self.macros,
            building_component: self.building_component,
            project_path: self.project_path,
            node_memory: self.node_memory,
            mutation: self.mutation,
        };
        builder.build(&mut nested)?;
        let committed = nested.committed;
        let out = committed.iter().find(|(name, _)| name == "Out").map(|(_, map)| map.clone());
        self.side_commits.extend(nested.side_commits);
        self.variable_updates.extend(nested.variable_updates);
        for (output, map) in committed {
            self.side_commits.push((original.id, output, map));
        }
        Ok(out)
    }

    /// The type of the node feeding `port` (`port.GetSourceNode()`), if
    /// connected.
    pub fn source_type(&self, port: &str) -> Option<&str> {
        let record = self.node.port(port)?.record.as_ref()?;
        let node = self.terrain.nodes.get(&record.from)?;
        Some(node.type_name.as_str())
    }

    /// The map feeding the first port of the node that feeds `port`
    /// (`port.GetSourceNode().In.GetMap()`): what that node started from.
    pub fn upstream_input(&self, port: &str) -> Option<Arc<Map>> {
        let record = self.node.port(port)?.record.as_ref()?;
        let source = self.terrain.nodes.get(&record.from)?;
        let source_record = source.ports.first()?.record.as_ref()?;
        read_output(self.terrain, self.cache, self.states, source_record.from, &source_record.from_port)
    }

    pub fn value(&self, name: &str) -> &Value {
        self.node.values.get(name).unwrap_or(&Value::Null)
    }

    pub fn f32(&self, name: &str) -> f32 {
        self.value(name).as_f64().unwrap_or(0.0) as f32
    }

    pub fn i32(&self, name: &str) -> i32 {
        self.value(name).as_i64().unwrap_or(0) as i32
    }

    pub fn bool(&self, name: &str) -> bool {
        self.value(name).as_bool().unwrap_or(false)
    }

    pub fn string(&self, name: &str) -> &str {
        self.value(name).as_str().unwrap_or("")
    }

    /// An enum parameter's member name.
    pub fn enum_name(&self, name: &str) -> &str {
        self.string(name)
    }

    /// An enum parameter's integer value.
    pub fn enum_value(&self, name: &str) -> i64 {
        let member = self.enum_name(name);
        let parameter = self.spec.parameter(name);
        parameter.and_then(|parameter| parameter.enum_value(member)).unwrap_or(0)
    }

    /// A `Float2` parameter (`{X, Y}`; missing members are zero).
    pub fn float2(&self, name: &str) -> [f32; 2] {
        let value = self.value(name);
        let x = value.get("X").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let y = value.get("Y").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        [x, y]
    }

    /// A `List<Float4>` parameter (`[{X, Y, Z, W}, …]`), or `None` when the
    /// member is null or missing.
    pub fn float4_list(&self, name: &str) -> Option<Vec<[f32; 4]>> {
        let items = self.value(name).as_array()?;
        let component = |item: &Value, key: &str| item.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let points = items
            .iter()
            .map(|item| [component(item, "X"), component(item, "Y"), component(item, "Z"), component(item, "W")])
            .collect();
        Some(points)
    }

    /// A `Color` parameter (`{R, G, B, A}`; missing members are zero).
    pub fn color(&self, name: &str) -> [f32; 4] {
        let value = self.value(name);
        let channel = |key: &str| value.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32;
        [channel("R"), channel("G"), channel("B"), channel("A")]
    }

    /// `Node.IsMaskable`: the node's class allows masks and `CanMask` holds.
    pub fn is_maskable(&self) -> bool {
        self.spec.is_maskable()
    }

    /// `Node.Commit(map, allowMasking, clamp)`: the node's main output.
    pub fn commit(&mut self, map: Map) -> Result<(), NodeError> {
        self.commit_with(map, true, true)
    }

    pub fn commit_with(&mut self, mut map: Map, allow_masking: bool, clamp: bool) -> Result<(), NodeError> {
        if !self.preferences.allow_values_beyond_range && clamp {
            clamp_if_out_of_range(&mut map);
        }

        if self.is_maskable() && allow_masking {
            let mask = self.input("Mask");
            let input = self.input("In");
            if let (Some(mask), Some(input)) = (mask, input) {
                map = blend_with_mask(&input, &map, &mask);
            }
        }

        if self.node.modifiers.is_empty() {
            self.commit_port("Out", map, RenderIntent::Default);
            return Ok(());
        }

        self.commit_port("Unmodified", map.clone(), RenderIntent::Default);
        let mut modified = map;
        let mut modifiers: Vec<&ModifierData> = self.node.modifiers.iter().collect();
        modifiers.sort_by_key(|modifier| modifier.values.get("Order").and_then(Value::as_i64).unwrap_or(0));
        for modifier in modifiers {
            let Some(builder) = self.registry.modifier(&modifier.type_name) else {
                return Err(NodeError::Unimplemented(format!("{} modifier", modifier.type_name)));
            };
            modified = builder.process(modifier, modified, self)?;
        }
        self.commit_port("Out", modified, RenderIntent::Default);
        Ok(())
    }

    /// `Node.Commit(portName, map, intent)`: a named output, tagged with the
    /// given intent or else the port's (or `Out`'s) intent.
    pub fn commit_port(&mut self, port: &str, mut map: Map, intent: RenderIntent) {
        map.info.render_intent = if intent == RenderIntent::Default { self.port_intent(port) } else { intent };
        self.committed.retain(|(name, _)| name != port);
        self.committed.push((port.to_string(), map));
    }

    /// A port's own intent (`Port.RenderIntent`): set during this build,
    /// else the one the port was created with (`Default` for ports read
    /// from a file); `None` when the node has no such port.
    fn port_own_intent(&self, port: &str) -> Option<RenderIntent> {
        if let Some(intent) = self.intent_overrides.get(port) {
            return Some(*intent);
        }
        let node_port = self.node.ports.iter().find(|candidate| candidate.name == port)?;
        Some(node_port.declared_intent.unwrap_or_default())
    }

    /// [`Self::port_own_intent`], `Default` when the port has none.
    fn declared_intent(&self, port: &str) -> RenderIntent {
        self.port_own_intent(port).unwrap_or_default()
    }

    /// Every output port's intent as the build leaves it, `Out` included
    /// whatever ports the node has.
    fn output_intents(&self) -> PortIntents {
        let mut intents = PortIntents::new();
        intents.insert("Out".to_string(), self.declared_intent("Out"));
        for port in self.node.ports.iter().filter(|port| port.is_output()) {
            intents.insert(port.name.clone(), self.declared_intent(&port.name));
        }
        intents
    }

    /// The intent a port's maps carry (`Commit(port, map)`): the port's
    /// own (`Default` included), else `Out`'s.
    fn port_intent(&self, port: &str) -> RenderIntent {
        let own = self.port_own_intent(port);
        own.or_else(|| self.port_own_intent("Out")).unwrap_or(RenderIntent::Default)
    }

    /// A map read back from what this node has committed so far.
    pub fn committed(&self, port: &str) -> Option<&Map> {
        self.committed.iter().find(|(name, _)| name == port).map(|(_, map)| map)
    }
}

/// Reads a node's output (`Node.GetMap(port)`). A bypassed node hands out
/// the main output of whatever feeds its `In` port, whichever of its own
/// outputs was asked for (`Node.GetCache` with `StateOverride.Bypassed`).
pub(crate) fn read_output(terrain: &Terrain, cache: &Cache, states: &HashMap<i32, NodeState>, node: i32, port: &str) -> Option<Arc<Map>> {
    let source = terrain.nodes.get(&node)?;
    if let Some(own_input) = source.port(port).filter(|candidate| candidate.is_input()) {
        let record = own_input.record.as_ref()?;
        return read_output(terrain, cache, states, record.from, &record.from_port);
    }
    if StateOverride::of(source) == StateOverride::Bypassed {
        let record = source.port("In")?.record.as_ref()?;
        return read_output(terrain, cache, states, record.from, "Out");
    }
    let state = states.get(&node).copied().unwrap_or(NodeState::Unbuilt);
    if state == NodeState::Error {
        return None;
    }
    cache.get(node, port)
}

/// `Map.ClampIfNeeded`: clamps to [0, 1] only when the first channel goes
/// outside that range. `max` then `min` turn NaN into 0, which
/// `clamp` would keep.
#[allow(clippy::manual_clamp)]
fn clamp_if_out_of_range(map: &mut Map) {
    let data = map.data();
    let mut maximum = f32::MIN;
    let mut minimum = f32::MAX;
    for &value in data {
        maximum = maximum.max(value);
        minimum = minimum.min(value);
    }
    if maximum <= 1.0 && minimum >= 0.0 {
        return;
    }
    for channel in 0..map.channel_count() {
        for value in map.channel_mut(channel) {
            *value = value.max(0.0).min(1.0);
        }
    }
}

/// `In * (1 - mask) + map * mask`, channel by channel (single-channel maps
/// broadcast).
fn blend_with_mask(input: &Map, map: &Map, mask: &Map) -> Map {
    let channel_count = input.channel_count().max(map.channel_count()).max(mask.channel_count());
    let resolution = map.resolution();
    let mut channels = Vec::with_capacity(channel_count);
    for channel in 0..channel_count {
        let input_plane = input.rgb_channel(channel);
        let map_plane = map.rgb_channel(channel);
        let mask_plane = mask.rgb_channel(channel);
        let blended: Vec<f32> = (0..resolution.area())
            .map(|index| {
                let kept = input_plane[index] * (1.0 - mask_plane[index]);
                let replaced = map_plane[index] * mask_plane[index];
                kept + replaced
            })
            .collect();
        channels.push(blended);
    }
    let mut result = Map::from_channels(resolution, channels);
    result.info = input.info;
    result
}

/// `BuildResult`.
#[derive(Clone, Debug, PartialEq)]
pub enum BuildResult {
    Succeeded,
    AlreadyBuilt,
    Skipped,
    Failed(NodeError),
}

/// Builds nodes of one terrain and keeps their results.
pub struct Engine {
    pub(crate) registry: Arc<Registry>,
    pub preferences: EnginePreferences,
    pub cache: Cache,
    /// The node the editor shows colour and mask nodes
    /// over (0 for none).
    pub underlay: i32,
    pub(crate) states: HashMap<i32, NodeState>,
    /// The intents each built node's output ports were left with.
    pub(crate) port_intents: HashMap<i32, PortIntents>,
    pub(crate) errors: HashMap<i32, NodeError>,
    pub(crate) member_updates: Vec<(i32, String, Value)>,
    /// The asset's variables, bindings and expressions. Builds apply the
    /// bindings and expressions to nodes and change variables (`Var`);
    /// the caller copies it from the asset and back.
    pub automation: Automation,
    /// The editor, or a command-line build.
    pub application_mode: ApplicationMode,
    /// `LoopEnd` nodes whose loop is running (`LoopEnd.LoopInProgress`).
    pub(crate) loops_in_progress: HashSet<i32>,
    /// Installed macros by id (`Project.MacroSource`), for `Macro` nodes.
    pub macros: MacroLibrary,
    /// Whether this engine builds a macro's graph.
    pub(crate) building_component: bool,
    /// The project file being built, if saved.
    pub project_path: Option<PathBuf>,
    pub(crate) node_memory: NodeMemory,
    /// The mutation the build is a variant of (0 for
    /// none), which a `LoopEnd` with Mutate Seeds adds to its loop's seeds
    /// before every pass ([`crate::mutation`]).
    pub mutation: i32,
}

/// `ApplicationMode`: what kind of application is building (loops run
/// in the editor only when primed for design time, on the command line
/// only when primed for builds).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ApplicationMode {
    #[default]
    Ui,
    Swarm,
    SwarmAutomated,
    ErosionStudio,
    Animator,
}

impl Engine {
    pub fn new(registry: Arc<Registry>) -> Self {
        Self {
            registry,
            preferences: EnginePreferences::default(),
            cache: Cache::default(),
            underlay: 0,
            states: HashMap::new(),
            port_intents: HashMap::new(),
            errors: HashMap::new(),
            member_updates: Vec::new(),
            automation: Automation::default(),
            application_mode: ApplicationMode::Ui,
            loops_in_progress: HashSet::new(),
            macros: MacroLibrary::new(),
            building_component: false,
            project_path: None,
            node_memory: NodeMemory::default(),
            mutation: 0,
        }
    }

    /// The node implementations this engine builds with (for another
    /// engine over the same nodes, such as a bake's).
    pub fn registry(&self) -> Arc<Registry> {
        self.registry.clone()
    }

    /// Changes builds made to their nodes' saved members, as (node, member,
    /// value), oldest first. The caller applies them to the project with
    /// [`NodeData::set_member`].
    pub fn take_member_updates(&mut self) -> Vec<(i32, String, Value)> {
        std::mem::take(&mut self.member_updates)
    }

    pub fn state(&self, node: i32) -> NodeState {
        self.states.get(&node).copied().unwrap_or(NodeState::Unbuilt)
    }

    /// What a node kept from its builds ([`BuildContext::remember`]).
    pub fn memory<T: Any + Send + Sync>(&self, node: i32) -> Option<Arc<T>> {
        let memory = self.node_memory.lock().ok()?;
        let kept = memory.get(&node)?.clone();
        kept.downcast::<T>().ok()
    }

    /// The heightfield the viewport shows a node's colours or mask over when
    /// the node is outside its build (`RendererBase.ResolveHeightfield`).
    pub fn underlay_heightfield(&self, terrain: &Terrain, node: i32, resolution: Resolution) -> Option<Arc<Map>> {
        let node = terrain.nodes.get(&node)?;
        let context = self.context_for(terrain, node, resolution).ok()?;
        context.underlay_heightfield()
    }

    /// The intents a built node's output ports were left with
    /// (`Port.RenderIntent`, what `RendererBase.GetRender` reads); `None`
    /// before its first build.
    pub fn port_intents(&self, node: i32) -> Option<&PortIntents> {
        self.port_intents.get(&node)
    }

    /// `Node.GetCustomColorMap()` of a built node, as the viewport asks for
    /// it when the type has `CustomColorizationAttribute`; `None` for a
    /// null colour map (or when the type has no colour map of its own).
    pub fn custom_colour_map(&self, terrain: &Terrain, node: i32, resolution: Resolution) -> Option<Map> {
        let node = terrain.nodes.get(&node)?;
        let current = self.current_node(node);
        let builder = self.registry.node(&current.type_name)?;
        let context = self.context_for(terrain, &current, resolution).ok()?;
        builder.custom_colour_map(&context)
    }

    /// What the viewport shows a node over when its type has
    /// `CustomUnderlayAttribute` (`RendererBase.ResolveHeightfield`): the
    /// cached main output of the node `Node.GetCustomUnderlay` names.
    /// `None` when it names none or that output is not cached.
    pub fn custom_underlay(&self, terrain: &Terrain, node: i32, resolution: Resolution) -> Option<Arc<Map>> {
        let node = terrain.nodes.get(&node)?;
        let current = self.current_node(node);
        let builder = self.registry.node(&current.type_name)?;
        let context = self.context_for(terrain, &current, resolution).ok()?;
        let source = builder.custom_underlay(&context)?;
        self.cache.get(source, "Out")
    }

    pub fn error(&self, node: i32) -> Option<&NodeError> {
        self.errors.get(&node)
    }

    /// Forgets a node's results and everything downstream of it. Baked
    /// nodes keep their maps.
    pub fn invalidate(&mut self, terrain: &Terrain, node: i32) {
        let mut pending = vec![node];
        let mut seen = HashSet::new();
        while let Some(current) = pending.pop() {
            if !seen.insert(current) {
                continue;
            }
            if !self.is_baked(current) {
                self.states.remove(&current);
                self.errors.remove(&current);
                self.cache.remove_node(current);
            }
            for candidate in terrain.nodes.values() {
                let feeds = candidate.ports.iter().any(|port| port.record.as_ref().is_some_and(|record| record.from == current));
                if feeds {
                    pending.push(candidate.id);
                }
            }
        }
    }

    /// Records that a node's maps were put in the cache from outside (a
    /// baked or imported result), so it is not rebuilt.
    pub fn mark_built(&mut self, node: i32) {
        self.states.insert(node, NodeState::Built);
    }

    /// A baked node's stored maps put in the cache (`Cache.Reload` setting
    /// each port's intent from its map): builds take
    /// them as they are while the node stays `Baked`.
    pub fn load_baked(&mut self, node: i32, outputs: Vec<(String, Map)>) {
        self.cache.remove_node(node);
        let mut intents = PortIntents::new();
        for (port, map) in outputs {
            intents.insert(port.clone(), map.info.render_intent);
            self.cache.store(node, &port, Arc::new(map));
        }
        self.port_intents.insert(node, intents);
        self.states.insert(node, NodeState::Baked);
        self.errors.remove(&node);
    }

    /// A baked node whose maps stay on disk (`NodeCacheStatus.Offloaded`,
    /// where bake loading leaves bakes nothing unbaked reads): builds skip
    /// it.
    pub fn offload_baked(&mut self, node: i32) {
        self.cache.remove_node(node);
        self.states.insert(node, NodeState::Baked);
        self.errors.remove(&node);
    }

    /// Unbaking on the engine's side: an unbaked node keeps the maps
    /// it has as an ordinary build result.
    pub fn unbake(&mut self, node: i32) {
        if self.states.get(&node) != Some(&NodeState::Baked) {
            return;
        }
        if self.cache.ports(node).next().is_some() {
            self.states.insert(node, NodeState::Built);
        } else {
            self.states.remove(&node);
        }
    }

    /// Whether builds take a node's maps from its bake (loaded or not).
    pub fn is_baked(&self, node: i32) -> bool {
        self.states.get(&node) == Some(&NodeState::Baked)
    }

    /// Every result forgotten except the bakes' (baked nodes are left
    /// alone).
    pub fn forget_unbaked(&mut self) {
        let unbaked: Vec<i32> = self.states.iter().filter(|(_, state)| **state != NodeState::Baked).map(|(node, _)| *node).collect();
        for node in unbaked {
            self.states.remove(&node);
            self.errors.remove(&node);
            self.cache.remove_node(node);
        }
        let cached: Vec<i32> = self.cache.nodes().filter(|node| self.states.get(node) != Some(&NodeState::Baked)).collect();
        for node in cached {
            self.cache.remove_node(node);
        }
    }

    /// Forgets what a node kept from its builds ([`BuildContext::remember`]).
    pub fn forget_memory(&mut self, node: i32) {
        if let Ok(memory) = self.node_memory.get_mut() {
            memory.remove(&node);
        }
    }

    /// Forgets every result (a new resolution, a reloaded project, …).
    pub fn clear(&mut self) {
        self.cache = Cache::default();
        self.states.clear();
        self.errors.clear();
        if let Ok(memory) = self.node_memory.get_mut() {
            memory.clear();
        }
    }

    /// The node and everything upstream of it, dependencies first
    /// (`GetBuildOrder`: depth-first, inputs in port order).
    pub fn build_order(terrain: &Terrain, node: i32) -> Vec<i32> {
        let mut visited = HashSet::new();
        let mut order = Vec::new();
        visit_upstream(terrain, node, &mut visited, &mut order);
        order
    }

    /// Builds `node` at `resolution` (`Builder.Build(node, context)`).
    pub fn build(&mut self, terrain: &Terrain, node: i32, resolution: Resolution) -> BuildResult {
        let mut result = BuildResult::Skipped;
        for current in Self::build_order(terrain, node) {
            result = self.build_one(terrain, current, resolution);
        }
        result
    }

    fn build_one(&mut self, terrain: &Terrain, id: i32, resolution: Resolution) -> BuildResult {
        let Some(node) = terrain.nodes.get(&id) else {
            return BuildResult::Skipped;
        };
        match StateOverride::of(node) {
            StateOverride::Bypassed => {
                self.states.insert(id, NodeState::Bypassed);
                return BuildResult::Skipped;
            }
            // The build loop skips baked nodes; their maps are whatever the bake
            // loading put in the cache. A baked node the engine was not
            // given a bake for (no files) builds as usual.
            StateOverride::Baked if self.is_baked(id) => {
                return BuildResult::AlreadyBuilt;
            }
            _ => {}
        }
        if self.state(id) == NodeState::Built {
            return BuildResult::AlreadyBuilt;
        }

        // The node as the build holds it: earlier builds' member changes applied.
        let current = self.current_node(node);
        let result = self.build_current(terrain, &current, resolution);
        match result {
            Ok(outputs) => {
                self.cache.remove_node(id);
                self.apply_outputs(id, outputs);
                self.states.insert(id, NodeState::Built);
                self.errors.remove(&id);
                BuildResult::Succeeded
            }
            Err(error) => {
                self.cache.remove_node(id);
                self.states.insert(id, NodeState::Error);
                self.errors.insert(id, error.clone());
                BuildResult::Failed(error)
            }
        }
    }

    /// One node's turn in the build loop: validity
    /// checks, then the bindings and expressions, then what the
    /// loop nodes do around their build, then the build.
    fn build_current(&mut self, terrain: &Terrain, node: &NodeData, resolution: Resolution) -> Result<NodeOutputs, NodeError> {
        self.check_node(terrain, node, resolution)?;
        let bound = self.bind_automation(terrain, node, resolution)?;
        let node = bound.as_ref().unwrap_or(node);
        match node.type_name.as_str() {
            "LoopBegin" => self.run_loop_begin_processes(terrain, node, resolution)?,
            "LoopEnd" => self.run_loop(terrain, node, resolution),
            _ => {}
        }
        self.run_node(terrain, node, resolution)
    }

    /// Stores what a build produced: its maps, maps it made for other
    /// nodes, its member and variable changes, its output's intent.
    pub(crate) fn apply_outputs(&mut self, id: i32, outputs: NodeOutputs) {
        for (port, map) in outputs.committed {
            self.cache.store(id, &port, Arc::new(map));
        }
        for (other, port, map) in outputs.side_commits {
            self.cache.store(other, &port, Arc::new(map));
        }
        for (member, value) in outputs.member_updates {
            self.member_updates.push((id, member, value));
        }
        for (name, value) in outputs.variable_updates {
            if let Some(variable) = self.automation.variables.get_mut(&name) {
                variable.value = value;
            }
        }
        self.port_intents.insert(id, outputs.port_intents);
    }

    /// A node with the member changes earlier builds made to it (builds
    /// change the node object itself) that the caller has not applied yet.
    pub(crate) fn current_node<'n>(&self, node: &'n NodeData) -> Cow<'n, NodeData> {
        let changes: Vec<&(i32, String, Value)> = self.member_updates.iter().filter(|(id, _, _)| *id == node.id).collect();
        if changes.is_empty() {
            return Cow::Borrowed(node);
        }
        let mut changed = node.clone();
        for (_, member, value) in changes {
            changed.set_member(member, value.clone());
        }
        Cow::Owned(changed)
    }

    /// The node as it builds once its bindings and expressions are applied
    /// (`None` when nothing changes), recording the changes for the caller.
    fn bind_automation(&mut self, terrain: &Terrain, node: &NodeData, resolution: Resolution) -> Result<Option<NodeData>, NodeError> {
        let catalogue = Catalogue::get();
        let Some(spec) = catalogue.node(&node.type_name) else {
            return Ok(None);
        };
        let context = ExpressionContext { resolution, terrain_height: terrain.height };
        let changes = self.automation.bind(node, spec, &context).map_err(NodeError::Failed)?;
        let mut bound: Option<NodeData> = None;
        for (member, value) in changes {
            if node.values.get(&member) == Some(&value) {
                continue;
            }
            let changed = bound.get_or_insert_with(|| node.clone());
            changed.values.insert(member.clone(), value.clone());
            self.member_updates.push((node.id, member, value));
        }
        Ok(bound)
    }

    /// The node's validity checks (required ports
    /// connected and delivering maps at the build resolution).
    fn check_node(&self, terrain: &Terrain, node: &NodeData, resolution: Resolution) -> Result<(), NodeError> {
        let builder = self.registry.node(&node.type_name).ok_or_else(|| NodeError::Unimplemented(node.type_name.clone()))?;
        let context = self.context_for(terrain, node, resolution)?;
        builder.check_validity(&context)
    }

    fn context_for<'c>(&'c self, terrain: &'c Terrain, node: &'c NodeData, resolution: Resolution) -> Result<BuildContext<'c>, NodeError> {
        let catalogue = Catalogue::get();
        let spec = catalogue.node(&node.type_name).ok_or_else(|| NodeError::Unimplemented(node.type_name.clone()))?;
        let context = BuildContext {
            node,
            spec,
            resolution,
            preferences: &self.preferences,
            terrain,
            cache: &self.cache,
            states: &self.states,
            port_intents: &self.port_intents,
            underlay: self.underlay,
            registry: &self.registry,
            committed: Vec::new(),
            intent_overrides: HashMap::new(),
            member_updates: Vec::new(),
            side_commits: Vec::new(),
            automation: &self.automation,
            variable_updates: Vec::new(),
            pending_members: &self.member_updates,
            application_mode: self.application_mode,
            loop_in_progress: self.loops_in_progress.contains(&node.id),
            macros: &self.macros,
            building_component: self.building_component,
            project_path: self.project_path.as_deref(),
            node_memory: &self.node_memory,
            mutation: self.mutation,
        };
        Ok(context)
    }

    /// `node.Commit(map)` for a node that is not building: the commit
    /// pipeline (clamp, masking, modifiers) alone.
    pub(crate) fn commit_for(&self, terrain: &Terrain, node: &NodeData, map: Map, resolution: Resolution) -> Result<Vec<(String, Map)>, NodeError> {
        let mut context = self.context_for(terrain, node, resolution)?;
        context.commit(map)?;
        Ok(context.committed)
    }

    /// Runs the node's builder (`Node.Build()`): what it commits and
    /// changes, nothing stored yet.
    pub(crate) fn run_node(&self, terrain: &Terrain, node: &NodeData, resolution: Resolution) -> Result<NodeOutputs, NodeError> {
        let builder = self.registry.node(&node.type_name).ok_or_else(|| NodeError::Unimplemented(node.type_name.clone()))?;
        let mut context = self.context_for(terrain, node, resolution)?;
        builder.build(&mut context)?;
        let port_intents = context.output_intents();
        let outputs = NodeOutputs {
            committed: context.committed,
            member_updates: context.member_updates,
            side_commits: context.side_commits,
            variable_updates: context.variable_updates,
            port_intents,
        };
        Ok(outputs)
    }

}

/// What one node build produced.
pub(crate) struct NodeOutputs {
    committed: Vec<(String, Map)>,
    member_updates: Vec<(String, Value)>,
    side_commits: Vec<(i32, String, Map)>,
    variable_updates: Vec<(String, VariableValue)>,
    port_intents: PortIntents,
}

fn visit_upstream(terrain: &Terrain, node: i32, visited: &mut HashSet<i32>, order: &mut Vec<i32>) {
    if !visited.insert(node) {
        return;
    }
    let Some(data) = terrain.nodes.get(&node) else {
        return;
    };
    for port in data.ports.iter().filter(|port| port.is_input()) {
        if let Some(record) = &port.record {
            visit_upstream(terrain, record.from, visited, order);
        }
    }
    order.push(node);
}

/// The base `Node.CheckValidity`: every required port is connected and
/// delivers a map at the build resolution.
pub fn check_required_ports(context: &BuildContext) -> Result<(), NodeError> {
    let mut problems = Vec::new();
    for port in context.node.ports.iter().filter(|port| port.is_required()) {
        if port.record.is_none() {
            problems.push(format!("The port {} is required and is not connected.", port.name));
            continue;
        }
        let map = context.input(&port.name);
        let good = map.as_ref().is_some_and(|map| map.resolution() == context.resolution);
        if !good {
            problems.push(format!("The port {} returned bad or no data.", port.name));
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    Err(NodeError::Validation(problems.join("\n")))
}
