//! The open `.terrain` project and every edit made to it: node and
//! connection changes, undo history, and the nodes each edit leaves to be
//! rebuilt.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use nerthus_engine::region::{self, Region};
use nerthus_engine::{to_terrain_json, Catalogue, ConnectionRecord, NodeData, PortIntents, Project, Terrain};
use nerthus_export::build::BuildDefinition;
use nerthus_export::FileFormat;
use serde_json::{json, Value};

/// Where projects are read and written: files, or in the web build the
/// browser (the examples, the files picked, downloads).
#[cfg(not(target_arch = "wasm32"))]
use std::fs as project_files;
#[cfg(target_arch = "wasm32")]
use crate::web as project_files;

mod import;
mod macros;

mod clipboard;
mod deleting;
mod groups;
mod links;
mod node_ops;
mod notes;
mod macro_ports;
mod modifiers;
mod portals;
mod state;
mod variables;

pub use clipboard::NodeClipboard;
pub use deleting::DeleteRequest;
pub use modifiers::PINNED_ORDER;
pub use groups::{GroupData, GROUP_ICONS, GROUP_MIN_SIZE};
pub use links::{grid_inputs, grid_outputs, PortalState};
pub use node_ops::{CombineKind, Layout};
pub use portals::TabTarget;
pub use state::{ORPHANS_CAPTION, ORPHANS_TEXT};
pub use variables::{variable_name_problem, VariableLimits, STRING_LIST_TYPE};

/// The project format version Nerthus writes into project metadata.
pub const FORMAT_VERSION: &str = "2.3.1.0";

/// Edits kept for undo.
const UNDO_LIMIT: usize = 100;

/// A state of the project to return to, named after the edit that left it.
struct UndoEntry {
    label: String,
    project: Project,
    /// The node and property a value edit changed: further edits of the
    /// same one join this entry.
    merge_key: Option<(i32, String)>,
    /// Going back to it reopens the whole project, the graph's view with
    /// it (`ExecuteUndo` of a full record); a value edit only sets the
    /// value back.
    restores_view: bool,
}

/// A point in the undo history ([`Document::undo_mark`]).
#[derive(Clone, Copy, Debug)]
pub struct UndoMark {
    pushes: u64,
    dirty: bool,
}

pub struct Document {
    pub project: Project,
    pub path: Option<PathBuf>,
    /// The name an unsaved copy goes by (an example opened from the
    /// examples is named after it); `Untitled` when none.
    pub untitled_name: Option<String>,
    pub dirty: bool,
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
    /// Nodes whose results edits made stale since the engine last heard.
    stale: HashSet<i32>,
    /// Every result is stale (a different project).
    stale_all: bool,
    /// The intents built nodes' outputs were left with (`Port.RenderIntent`)
    /// as the engine last reported them, which new connections pass on.
    built_intents: HashMap<i32, PortIntents>,
    random: SmallRandom,
    /// Counts the undos and redos that put the graph tabs' views
    /// (`ViewportLocation`, `ZoomFactor`) back: the graph takes them again.
    view_generation: u64,
    /// Counts the undo entries ever pushed, so a run of edits can be told
    /// apart from what came before it ([`Document::undo_mark`]).
    pushes: u64,
}

impl Document {
    /// A new, empty project.
    pub fn new() -> Self {
        let mut random = SmallRandom::from_time();
        let project = blank_project(&mut random);
        Self::with_project(project, None, random)
    }

    fn with_project(project: Project, path: Option<PathBuf>, random: SmallRandom) -> Self {
        Self {
            project,
            path,
            untitled_name: None,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            stale: HashSet::new(),
            stale_all: true,
            built_intents: HashMap::new(),
            random,
            view_generation: 0,
            pushes: 0,
        }
    }

    pub fn open(path: &Path) -> Result<Self> {
        let text = project_files::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
        let project = Project::parse(&text).with_context(|| format!("cannot open {}", path.display()))?;
        let random = SmallRandom::from_time();
        Ok(Self::with_project(project, Some(path.to_path_buf()), random))
    }

    /// Writes the project as a `.terrain` file, stamping the save time.
    pub fn save(&mut self, path: &Path) -> Result<()> {
        let now = project_timestamp();
        stamp_saved(&mut self.project.extra, &now);
        stamp_saved(&mut self.project.terrain_mut().extra, &now);
        let text = to_terrain_json(&self.project);
        project_files::write(path, text).with_context(|| format!("cannot write {}", path.display()))?;
        self.path = Some(path.to_path_buf());
        self.dirty = false;
        Ok(())
    }

    /// The file name shown in the title bar (`Untitled.terrain` until saved).
    pub fn file_name(&self) -> String {
        let name = self.path.as_ref().and_then(|path| path.file_name()).map(|name| name.to_string_lossy().to_string());
        let untitled = || self.untitled_name.clone().unwrap_or_else(|| "Untitled.terrain".to_string());
        name.unwrap_or_else(untitled)
    }

    /// `Project.Id`: names the project's cache folders.
    pub fn project_id(&self) -> String {
        self.project.extra.get("Id").and_then(Value::as_str).unwrap_or_default().to_string()
    }

    pub fn terrain(&self) -> &Terrain {
        self.project.terrain()
    }

    pub fn node(&self, id: i32) -> Option<&NodeData> {
        self.terrain().nodes.get(&id)
    }

    /// Takes the nodes to rebuild (and whether everything is stale).
    pub fn take_stale(&mut self) -> (Vec<i32>, bool) {
        let all = std::mem::take(&mut self.stale_all);
        let nodes = self.stale.drain().collect();
        (nodes, all)
    }

    /// Rebuilds a node (and what follows) on the next build.
    pub fn mark_stale(&mut self, node: i32) {
        self.stale.insert(node);
    }

    /// Rebuilds everything on the next build.
    pub fn mark_all_stale(&mut self) {
        self.stale_all = true;
    }

    /// What the engine reports of the intents its builds left on each
    /// node's outputs (`Port.RenderIntent`).
    pub fn set_built_intents(&mut self, intents: HashMap<i32, PortIntents>) {
        self.built_intents = intents;
    }

    /// Remembers the current state before an edit named `label`.
    pub fn checkpoint(&mut self, label: impl Into<String>) {
        self.push_undo(label.into(), None);
    }

    fn push_undo(&mut self, label: String, merge_key: Option<(i32, String)>) {
        let restores_view = merge_key.is_none();
        let entry = UndoEntry {
            label,
            project: self.project.clone(),
            merge_key,
            restores_view,
        };
        self.undo.push(entry);
        self.pushes += 1;
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
    }

    /// Where the undo history stands, for [`Document::squash_undo`] and
    /// [`Document::abandon_edits`] once a run of edits is done.
    pub fn undo_mark(&self) -> UndoMark {
        UndoMark { pushes: self.pushes, dirty: self.dirty }
    }

    /// The undo entries pushed since `mark`, the edits of one command,
    /// become one named `label`: undoing it returns to the state before
    /// the first. False when nothing was pushed.
    pub fn squash_undo(&mut self, mark: UndoMark, label: impl Into<String>) -> bool {
        let added = self.pushes.saturating_sub(mark.pushes) as usize;
        let added = added.min(self.undo.len());
        if added == 0 {
            return false;
        }
        let first = self.undo.len() - added;
        self.undo.truncate(first + 1);
        let entry = &mut self.undo[first];
        entry.label = label.into();
        entry.merge_key = None;
        entry.restores_view = true;
        true
    }

    /// Takes back every edit since `mark`, leaving nothing to redo: the
    /// project as it was before them.
    pub fn abandon_edits(&mut self, mark: UndoMark) {
        let added = self.pushes.saturating_sub(mark.pushes) as usize;
        let added = added.min(self.undo.len());
        if added == 0 {
            return;
        }
        let first = self.undo.len() - added;
        let Some(entry) = self.undo.drain(first..).next() else {
            return;
        };
        let current = std::mem::replace(&mut self.project, entry.project);
        self.mark_changed_nodes(&current);
        self.view_generation += 1;
        self.dirty = mark.dirty;
    }

    /// Labels of the edits that can be undone, newest first.
    pub fn undo_labels(&self) -> Vec<String> {
        self.undo.iter().rev().map(|entry| entry.label.clone()).collect()
    }

    /// Labels of the edits that can be redone, next first.
    pub fn redo_labels(&self) -> Vec<String> {
        self.redo.iter().rev().map(|entry| entry.label.clone()).collect()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(entry) = self.undo.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.project, entry.project);
        self.mark_changed_nodes(&current);
        if entry.restores_view {
            self.view_generation += 1;
        }
        let redo_entry = UndoEntry { label: entry.label, project: current, merge_key: None, restores_view: entry.restores_view };
        self.redo.push(redo_entry);
        self.dirty = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(entry) = self.redo.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.project, entry.project);
        self.mark_changed_nodes(&current);
        if entry.restores_view {
            self.view_generation += 1;
        }
        let undo_entry = UndoEntry { label: entry.label, project: current, merge_key: None, restores_view: entry.restores_view };
        self.undo.push(undo_entry);
        self.dirty = true;
        true
    }

    /// Bumped by each undo or redo that puts the graph's views back.
    pub fn view_generation(&self) -> u64 {
        self.view_generation
    }

    /// Marks stale every node that differs between `previous` and now.
    fn mark_changed_nodes(&mut self, previous: &Project) {
        let before: HashMap<i32, u64> = previous.terrain().nodes.iter().map(|(id, node)| (*id, node_fingerprint(node))).collect();
        let after: HashMap<i32, u64> = self.terrain().nodes.iter().map(|(id, node)| (*id, node_fingerprint(node))).collect();
        let mut changed = Vec::new();
        for (id, fingerprint) in &after {
            if before.get(id) != Some(fingerprint) {
                changed.push(*id);
            }
        }
        for id in before.keys() {
            if !after.contains_key(id) {
                changed.push(*id);
            }
        }
        for id in changed {
            self.mark_stale(id);
        }
    }

    /// A node id no node, group or note has (three digits while they
    /// last; one pool keeps the three apart).
    fn new_node_id(&mut self) -> i32 {
        self.new_element_id(&HashSet::new())
    }

    /// `Global.GetRandomSeed`: `Random.Next(0, 65535)`.
    pub fn random_seed(&mut self) -> i32 {
        self.random.below(65535) as i32
    }

    /// A seed from 0 up to (not including) `limit`, as the seed box's dice
    /// draws one (`XorShift64Star.Next(0, 65536)`).
    pub fn random_seed_below(&mut self, limit: u32) -> i32 {
        self.random.below(limit as u64) as i32
    }

    /// Adds a node of `type_name` at `position` (canvas units).
    pub fn add_node(&mut self, type_name: &str, position: [f32; 2]) -> Option<i32> {
        let spec = Catalogue::get().node(type_name)?;
        self.checkpoint(format!("Add {}", spec.display_name.as_deref().unwrap_or(&spec.name)));
        let id = self.new_node_id();
        let seed = self.random_seed();
        let mut node = NodeData::new(spec, id, seed);
        node.position = position;
        self.project.terrain_mut().nodes.insert(id, node);
        self.mark_stale(id);
        Some(id)
    }

    /// Connects an output to an input, replacing the input's connection.
    pub fn connect(&mut self, from: i32, from_port: &str, to: i32, to_port: &str) {
        if from == to {
            return;
        }
        let Some(target) = self.node(to) else {
            return;
        };
        let already = target.port(to_port).and_then(|port| port.record.as_ref()).is_some_and(|record| record.from == from && record.from_port == from_port);
        if already {
            return;
        }
        self.checkpoint("Connect");
        let record = ConnectionRecord {
            from,
            to,
            from_port: from_port.to_string(),
            to_port: to_port.to_string(),
            is_valid: true,
        };
        let target = self.project.terrain_mut().nodes.get_mut(&to);
        if let Some(target) = target {
            if let Some(port) = target.port_mut(to_port) {
                port.record = Some(record);
            }
            // SBomber's UpdateUI: a new input once every input is used.
            nerthus_engine::editing::grow_inputs(target);
        }
        self.mark_stale(to);
        self.convey_render_intent(from, from_port, to, to_port);
    }

    /// `AddNewPort` (the Add Input command).
    pub fn add_input(&mut self, node: i32) -> bool {
        let can_add = self.node(node).is_some_and(|data| nerthus_engine::editing::can_add_ports(&data.type_name));
        if !can_add {
            return false;
        }
        self.checkpoint("Add Input");
        let added = self.project.terrain_mut().nodes.get_mut(&node).and_then(nerthus_engine::editing::add_new_port);
        if added.is_none() {
            self.undo.pop();
            return false;
        }
        self.mark_stale(node);
        true
    }

    /// A modifier toolbar toggle (`Modifier_Checked` / `Modifier_Unchecked`):
    /// a modifier by that name goes (`RemoveModifier` takes the node's
    /// toolbar-added one of the type), otherwise one is added; Min and Max
    /// exclude each other.
    pub fn toggle_modifier(&mut self, node: i32, name: &str) {
        use nerthus_engine::editing::{is_intrinsic, modifier_name, new_modifier};
        let Some(data) = self.node(node) else {
            return;
        };
        let present = data.modifiers.iter().any(|modifier| modifier_name(modifier) == name);
        let label = if present { format!("Remove {name}") } else { format!("Add {name}") };
        self.checkpoint(label);
        let Some(target) = self.project.terrain_mut().nodes.get_mut(&node) else {
            return;
        };
        let remove = |modifiers: &mut Vec<nerthus_engine::ModifierData>, type_name: &str| {
            let position = modifiers.iter().position(|modifier| modifier.type_name == type_name && is_intrinsic(modifier));
            position.map(|index| modifiers.remove(index)).is_some()
        };
        let changed = if present {
            remove(&mut target.modifiers, name)
        } else {
            let opposite = match name {
                "Min" => Some("Max"),
                "Max" => Some("Min"),
                _ => None,
            };
            if let Some(opposite) = opposite {
                let opposite_present = target.modifiers.iter().any(|modifier| modifier_name(modifier) == opposite);
                if opposite_present {
                    remove(&mut target.modifiers, opposite);
                }
            }
            let order = target.modifiers.len();
            match new_modifier(name, order) {
                Some(modifier) => {
                    target.modifiers.push(modifier);
                    true
                }
                None => false,
            }
        };
        if !changed {
            self.undo.pop();
            return;
        }
        self.mark_stale(node);
    }

    /// Sets a modifier parameter (`index` into the node's modifiers).
    pub fn set_modifier_value(&mut self, node: i32, index: usize, name: &str, value: Value) {
        let current = self.node(node).and_then(|data| data.modifiers.get(index)).map(|modifier| modifier.values.get(name) == Some(&value));
        if current != Some(false) {
            return;
        }
        let merge_key = (node, format!("modifier {index} {name}"));
        let continues = self.undo.last().is_some_and(|entry| entry.merge_key.as_ref() == Some(&merge_key));
        if !(continues && self.redo.is_empty()) {
            self.push_undo(format!("Change {name}"), Some(merge_key));
        }
        self.dirty = true;
        let modifier = self.project.terrain_mut().nodes.get_mut(&node).and_then(|data| data.modifiers.get_mut(index));
        if let Some(modifier) = modifier {
            modifier.values.insert(name.to_string(), value);
        }
        self.mark_stale(node);
    }

    /// Combine's Swap Inputs: the first two inputs trade sources (both
    /// must be connected).
    pub fn switch_inputs(&mut self, node: i32) {
        let Some(data) = self.node(node) else {
            return;
        };
        let inputs: Vec<String> = data.ports.iter().filter(|port| port.is_input()).map(|port| port.name.clone()).collect();
        let (Some(first), Some(second)) = (inputs.first(), inputs.get(1)) else {
            return;
        };
        let first_record = data.port(first).and_then(|port| port.record.clone());
        let second_record = data.port(second).and_then(|port| port.record.clone());
        let (Some(mut first_record), Some(mut second_record)) = (first_record, second_record) else {
            return;
        };
        self.checkpoint("Swap Inputs");
        std::mem::swap(&mut first_record.to_port, &mut second_record.to_port);
        let target = self.project.terrain_mut().nodes.get_mut(&node);
        if let Some(target) = target {
            if let Some(port) = target.port_mut(first) {
                port.record = Some(second_record);
            }
            if let Some(port) = target.port_mut(second) {
                port.record = Some(first_record);
            }
        }
        self.mark_stale(node);
    }

    pub fn disconnect(&mut self, to: i32, to_port: &str) {
        let connected = self.node(to).and_then(|node| node.port(to_port)).is_some_and(|port| port.record.is_some());
        if !connected {
            return;
        }
        self.checkpoint("Disconnect");
        if let Some(port) = self.project.terrain_mut().nodes.get_mut(&to).and_then(|node| node.port_mut(to_port)) {
            port.record = None;
        }
        self.mark_stale(to);
    }

    /// Whether following inputs upstream from `from` reaches `to` (a
    /// connection from `from` into `to` would close a loop).
    pub fn would_cycle(&self, from: i32, to: i32) -> bool {
        let nodes = &self.terrain().nodes;
        let mut pending = vec![from];
        let mut seen = HashSet::new();
        while let Some(current) = pending.pop() {
            if current == to {
                return true;
            }
            if !seen.insert(current) {
                continue;
            }
            let Some(node) = nodes.get(&current) else {
                continue;
            };
            for port in &node.ports {
                if let Some(record) = &port.record {
                    pending.push(record.from);
                }
            }
        }
        false
    }

    /// Sets a node property; consecutive edits of the same node's property
    /// (a slider drag) are one undo step.
    pub fn set_value(&mut self, node: i32, name: &str, value: Value) {
        let Some(current) = self.node(node) else {
            return;
        };
        if current.values.get(name) == Some(&value) {
            return;
        }
        let merge_key = (node, name.to_string());
        // After an undo the redo history must go, so the edit starts anew.
        let continues = self.undo.last().is_some_and(|entry| entry.merge_key.as_ref() == Some(&merge_key));
        let merges = continues && self.redo.is_empty();
        if !merges {
            self.push_undo(format!("Change {name}"), Some(merge_key));
        }
        self.dirty = true;
        if let Some(target) = self.project.terrain_mut().nodes.get_mut(&node) {
            target.values.insert(name.to_string(), value);
        }
        self.mark_stale(node);
    }

    /// Sets several node properties as one edit named `label`; false when
    /// none changed.
    pub fn set_values(&mut self, node: i32, values: &[(String, Value)], label: &str) -> bool {
        let Some(current) = self.node(node) else {
            return false;
        };
        let changes = values.iter().any(|(name, value)| current.values.get(name) != Some(value));
        if !changes {
            return false;
        }
        self.checkpoint(label);
        self.set_values_quietly(node, values);
        true
    }

    /// Sets node properties as part of the edit under way (no undo entry
    /// of their own).
    pub fn set_values_quietly(&mut self, node: i32, values: &[(String, Value)]) {
        if let Some(target) = self.project.terrain_mut().nodes.get_mut(&node) {
            for (name, value) in values {
                target.values.insert(name.clone(), value.clone());
            }
        }
        self.mark_stale(node);
    }

    /// Sets a node property as an edit of its own (a node command's
    /// result), named `label` in the undo history.
    pub fn set_value_as(&mut self, node: i32, name: &str, value: Value, label: &str) {
        let Some(current) = self.node(node) else {
            return;
        };
        if current.values.get(name) == Some(&value) {
            return;
        }
        self.checkpoint(label);
        if let Some(target) = self.project.terrain_mut().nodes.get_mut(&node) {
            target.values.insert(name.to_string(), value);
        }
        self.mark_stale(node);
    }

    /// Sets a `Node` base member (`StateOverride`, `NodeSize`, …).
    pub fn set_member(&mut self, node: i32, name: &str, value: Value, label: &str, rebuilds: bool) {
        self.checkpoint(label);
        if let Some(target) = self.project.terrain_mut().nodes.get_mut(&node) {
            target.set_member(name, value);
        }
        if rebuilds {
            self.mark_stale(node);
        }
    }

    /// A member a build changed (`Engine::take_member_updates`): saved with
    /// the project, not an edit of its own.
    pub fn apply_build_update(&mut self, node: i32, name: &str, value: Value) {
        if let Some(target) = self.project.terrain_mut().nodes.get_mut(&node) {
            target.set_member(name, value);
        }
    }

    /// Returns whether the name changed.
    pub fn rename(&mut self, node: i32, name: &str) -> bool {
        let Some(current) = self.node(node) else {
            return false;
        };
        if current.name == name {
            return false;
        }
        let label = format!("Node renamed: {} renamed to {name}.", current.name);
        self.checkpoint(label);
        if let Some(target) = self.project.terrain_mut().nodes.get_mut(&node) {
            target.name = name.to_string();
        }
        true
    }

    /// Moves nodes; `checkpoint` is false while a drag is under way.
    pub fn move_nodes(&mut self, moves: &[(i32, [f32; 2])], checkpoint: bool) {
        if checkpoint {
            self.checkpoint("Move");
        }
        self.dirty = true;
        let terrain = self.project.terrain_mut();
        for (id, position) in moves {
            if let Some(node) = terrain.nodes.get_mut(id) {
                node.position = *position;
            }
        }
    }

    /// `Reset`: the node's properties back to a fresh node's, its seeds kept.
    pub fn reset_node(&mut self, node: i32) {
        let Some(current) = self.node(node) else {
            return;
        };
        let Some(spec) = Catalogue::get().node(&current.type_name) else {
            return;
        };
        let mut values = spec.defaults.clone();
        for parameter in spec.parameters.iter().filter(|parameter| parameter.is_seed()) {
            if let Some(seed) = current.values.get(&parameter.name) {
                values.insert(parameter.name.clone(), seed.clone());
            }
        }
        self.checkpoint("Reset");
        if let Some(target) = self.project.terrain_mut().nodes.get_mut(&node) {
            target.values = values;
        }
        self.mark_stale(node);
    }

    /// Puts back a node's parameters and modifiers from an earlier copy of
    /// it (which copies every `[Parameter]` property, and
    /// the modifiers' clone). False when nothing changed.
    pub fn restore_node(&mut self, node: i32, state: &NodeData, label: &str) -> bool {
        let Some(current) = self.node(node) else {
            return false;
        };
        let Some(spec) = Catalogue::get().node(&current.type_name) else {
            return false;
        };
        let mut values = current.values.clone();
        for parameter in &spec.parameters {
            if let Some(value) = state.values.get(&parameter.name) {
                values.insert(parameter.name.clone(), value.clone());
            }
        }
        let unchanged = values == current.values && state.modifiers == current.modifiers;
        if unchanged {
            return false;
        }
        self.checkpoint(label);
        if let Some(target) = self.project.terrain_mut().nodes.get_mut(&node) {
            target.values = values;
            target.modifiers = state.modifiers.clone();
        }
        self.mark_stale(node);
        true
    }

    /// `TerrainState.Viewport` as saved: the renderer, the lighting and the
    /// camera.
    pub fn viewport_state(&self) -> Option<&serde_json::Map<String, Value>> {
        let state = self.project.assets[0].extra.get("State")?;
        state.get("Viewport")?.as_object()
    }

    /// Changes `TerrainState.Viewport`, made when missing. The viewport's
    /// state is not an edit to undo.
    pub fn edit_viewport_state(&mut self, edit: impl FnOnce(&mut serde_json::Map<String, Value>)) {
        let extra = &mut self.project.assets[0].extra;
        let state = extra.entry("State").or_insert_with(|| json!({}));
        let Some(members) = state.as_object_mut() else {
            return;
        };
        let viewport = members.entry("Viewport".to_string()).or_insert_with(|| json!({}));
        if !viewport.is_object() {
            *viewport = json!({});
        }
        if let Some(viewport) = viewport.as_object_mut() {
            edit(viewport);
        }
    }

    /// `Viewport.CameraPosition`: the camera Reset to Loaded returns to,
    /// as the message that sets it.
    pub fn saved_camera(&self) -> Vec<u8> {
        let saved = self.viewport_state().and_then(|viewport| viewport.get("CameraPosition"));
        let items = saved.map(nerthus_engine::json::collection_items).unwrap_or_default();
        items.iter().filter_map(Value::as_u64).map(|byte| byte as u8).collect()
    }

    /// Notes the camera Reset to Loaded returns to (Load Camera, Reset to
    /// Default). Not an edit to undo.
    pub fn set_saved_camera(&mut self, message: &[u8]) {
        let bytes: Vec<Value> = message.iter().map(|byte| json!(byte)).collect();
        self.edit_viewport_state(|viewport| {
            viewport.insert("CameraPosition".to_string(), Value::Array(bytes));
            crate::viewport_render::order_viewport_members(viewport);
        });
    }

    /// `TerrainState.UnderlayNode` as saved (0 for none).
    pub fn saved_underlay(&self) -> i32 {
        let state = self.project.assets[0].extra.get("State");
        let saved = state.and_then(|state| state.get("UnderlayNode")).and_then(Value::as_i64);
        saved.unwrap_or(0) as i32
    }

    /// The underlay builds use: the saved one when its node is there and
    /// can be one, as it is restored on load (`Shell` calls `SetUnderlay`).
    pub fn underlay_node(&self) -> i32 {
        let saved = self.saved_underlay();
        let usable = self.node(saved).is_some_and(can_be_underlay);
        if usable { saved } else { 0 }
    }

    /// `GraphControl.SetUnderlay`: the node becomes the underlay, or stops
    /// being it. Returns false when the node cannot be one. The state is
    /// not an edit to undo; every result is rebuilt over the new underlay.
    pub fn set_underlay(&mut self, id: i32) -> bool {
        let Some(node) = self.node(id) else {
            return false;
        };
        let current = self.underlay_node();
        let next = if current == id {
            0
        } else {
            if !can_be_underlay(node) {
                return false;
            }
            id
        };
        let extra = &mut self.project.assets[0].extra;
        let state = extra.entry("State").or_insert_with(|| json!({}));
        if let Value::Object(members) = state {
            members.insert("UnderlayNode".to_string(), json!(next));
        }
        self.dirty = true;
        self.mark_all_stale();
        true
    }

    /// `NodeElement.DetermineScale`: a node without a size of its own whose
    /// name the user's `NodeDisplay` lists takes that size, kept on the
    /// node. Not an edit to undo.
    pub fn apply_node_display(&mut self, ids: &[i32], options: &crate::user::UserOptions) {
        for id in ids {
            let Some(node) = self.project.terrain_mut().nodes.get_mut(id) else {
                continue;
            };
            let current = node.base.get("NodeSize").and_then(Value::as_str).unwrap_or("Default");
            if current != "Default" {
                continue;
            }
            let Some(size) = options.node_display(&node.name) else {
                continue;
            };
            node.set_member("NodeSize", json!(size));
        }
    }

    /// `NodeElement.CycleNodeSizes`: Small and Standard swap; a node with
    /// more than two inputs or outputs (`IsBigNode`) goes Compact, Standard,
    /// Small and round again. A node without a size starts from the user's
    /// `DefaultNodeSize`.
    pub fn cycle_node_size(&mut self, id: i32, default_size: &str) {
        let Some(node) = self.node(id) else {
            return;
        };
        let saved = node.base.get("NodeSize").and_then(Value::as_str).unwrap_or("Default");
        let current = if saved == "Default" { default_size } else { saved };
        let inputs = node.ports.iter().filter(|port| port.is_input()).count();
        let outputs = node.ports.iter().filter(|port| port.is_output()).count();
        let big = inputs > 2 || outputs > 2;
        let next = match (big, current) {
            (false, "Small") => "Standard",
            (false, "Standard") => "Small",
            (true, "Compact") => "Standard",
            (true, "Standard") => "Small",
            (true, "Small") => "Compact",
            (_, other) => other,
        };
        let next = next.to_string();
        self.set_member(id, "NodeSize", json!(next), "Node size", false);
    }

    /// `TerrainState.NodeBookmarks`, in the order they were added.
    pub fn bookmarks(&self) -> Vec<i32> {
        let state = self.project.assets[0].extra.get("State");
        let saved = state.and_then(|state| state.get("NodeBookmarks")).and_then(Value::as_array);
        let ids = saved.map(|items| items.iter().filter_map(Value::as_i64).map(|id| id as i32).collect());
        ids.unwrap_or_default()
    }

    fn bookmarks_mut(&mut self) -> Option<&mut Vec<Value>> {
        let extra = &mut self.project.assets[0].extra;
        let state = extra.entry("State").or_insert_with(|| json!({}));
        let members = state.as_object_mut()?;
        let bookmarks = members.entry("NodeBookmarks".to_string()).or_insert_with(|| json!([]));
        if !bookmarks.is_array() {
            *bookmarks = json!([]);
        }
        bookmarks.as_array_mut()
    }

    /// `AddBookmark_Click` and the node menu's Bookmark: an undo step even
    /// when the node was bookmarked already (the set keeps it once).
    pub fn add_bookmark(&mut self, id: i32) {
        let Some(name) = self.node(id).map(|node| node.name.clone()) else {
            return;
        };
        self.checkpoint(format!("Node {name} added to bookmark"));
        if let Some(bookmarks) = self.bookmarks_mut()
            && !bookmarks.iter().any(|bookmark| bookmark.as_i64() == Some(id as i64))
        {
            bookmarks.push(json!(id));
        }
    }

    /// Takes a bookmark away; `label` is the undo step's name (the node
    /// menu says "bookmark", the bookmarks menu "bookmarks").
    pub fn remove_bookmark(&mut self, id: i32, label: &str) {
        let name = self.node(id).map(|node| node.name.clone()).unwrap_or_default();
        self.checkpoint(format!("Node {name} removed from {label}"));
        if let Some(bookmarks) = self.bookmarks_mut() {
            bookmarks.retain(|bookmark| bookmark.as_i64() != Some(id as i64));
        }
    }

    /// Paste Properties: every property of `values` onto each target of the
    /// source's type.
    pub fn paste_properties(&mut self, targets: &[i32], type_name: &str, values: &indexmap::IndexMap<String, Value>) {
        let matching: Vec<i32> = targets.iter().copied().filter(|id| self.node(*id).is_some_and(|node| node.type_name == type_name)).collect();
        if matching.is_empty() {
            return;
        }
        self.checkpoint("Paste properties");
        for id in matching {
            if let Some(node) = self.project.terrain_mut().nodes.get_mut(&id) {
                for (name, value) in values {
                    node.values.insert(name.clone(), value.clone());
                }
            }
            self.mark_stale(id);
        }
    }

    /// The terrain's notes (`Terrain.Notes`) by id, as saved.
    pub fn notes(&self) -> Vec<(i32, &serde_json::Map<String, Value>)> {
        let Some(Value::Object(notes)) = self.terrain().extra.get("Notes") else {
            return Vec::new();
        };
        let mut list = Vec::new();
        for (key, note) in notes {
            let (Ok(id), Value::Object(fields)) = (key.parse::<i32>(), note) else {
                continue;
            };
            list.push((id, fields));
        }
        list
    }

    fn note_mut(&mut self, id: i32) -> Option<&mut serde_json::Map<String, Value>> {
        let notes = self.project.terrain_mut().extra.get_mut("Notes")?;
        match notes.get_mut(id.to_string())? {
            Value::Object(fields) => Some(fields),
            _ => None,
        }
    }

    /// Graph > Add Note: an empty grey text note (`Note` defaults, 200 by
    /// 200) on graph tab `tab`.
    pub fn add_note(&mut self, position: [f32; 2], tab: usize) -> i32 {
        self.checkpoint("Add Note");
        let mut id = self.new_node_id();
        while self.notes().iter().any(|(existing, _)| *existing == id) {
            id = self.new_node_id();
        }
        let mut note = json!({
            "Id": id,
            "Name": "Note",
            "Markdown": "",
            "Color": "Gray",
            "Position": {"X": position[0] as f64, "Y": position[1] as f64},
            "Size": {"X": 200.0, "Y": 200.0},
            "Type": "OnlyText",
        });
        if tab != 0 {
            note["GraphIndex"] = json!(tab);
        }
        let extra = &mut self.project.terrain_mut().extra;
        let notes = extra.entry("Notes".to_string()).or_insert_with(|| json!({}));
        if let Value::Object(notes) = notes {
            notes.insert(id.to_string(), note);
        }
        id
    }

    pub fn remove_note(&mut self, id: i32) {
        if self.note_mut(id).is_none() {
            return;
        }
        self.checkpoint("Delete Note");
        if let Some(Value::Object(notes)) = self.project.terrain_mut().extra.get_mut("Notes") {
            notes.remove(&id.to_string());
        }
    }

    /// Moves or resizes a note (`member` is `Position` or `Size`);
    /// `checkpoint` is false while a drag is under way.
    pub fn set_note_vector(&mut self, id: i32, member: &str, value: [f32; 2], checkpoint: bool) {
        if checkpoint {
            let label = if member == "Size" { "Resize Note" } else { "Move Note" };
            self.checkpoint(label);
        }
        self.dirty = true;
        if let Some(note) = self.note_mut(id) {
            note.insert(member.to_string(), json!({"X": value[0] as f64, "Y": value[1] as f64}));
        }
    }

    pub fn set_note_text(&mut self, id: i32, markdown: &str) {
        let unchanged = self.note_mut(id).is_none_or(|note| note.get("Markdown").and_then(Value::as_str) == Some(markdown));
        if unchanged {
            return;
        }
        self.checkpoint("Edit Note");
        if let Some(note) = self.note_mut(id) {
            note.insert("Markdown".to_string(), json!(markdown));
        }
    }

    /// Stores a tab's view (`ZoomFactor`, `ViewportLocation`) without
    /// making the project dirty: it is saved but it is not an edit.
    pub fn store_view_quietly(&mut self, index: usize, location: [f32; 2], zoom: f64) {
        let Some(tab) = self.graph_tab_mut(index) else {
            return;
        };
        tab.insert("ZoomFactor".to_string(), json!(zoom));
        tab.insert("ViewportLocation".to_string(), json!({"X": location[0] as f64, "Y": location[1] as f64}));
    }

    /// The graph tabs (`Terrain.GraphTabs`) in order.
    pub fn graph_tabs(&self) -> Vec<GraphTabInfo> {
        let tabs = self.terrain().extra.get("GraphTabs").and_then(Value::as_array);
        let tabs = tabs.map(Vec::as_slice).unwrap_or_default();
        tabs.iter().map(GraphTabInfo::from_json).collect()
    }

    fn graph_tab_mut(&mut self, index: usize) -> Option<&mut serde_json::Map<String, Value>> {
        let tabs = self.project.terrain_mut().extra.get_mut("GraphTabs")?.as_array_mut()?;
        tabs.iter_mut().filter_map(Value::as_object_mut).find(|tab| tab_index(tab) == index)
    }

    /// `GraphArea.CreateNewTab`: "Graph N", the next index, the swatch of
    /// that index, the usual start point.
    pub fn add_graph_tab(&mut self) -> usize {
        let tabs = self.graph_tabs();
        let name = format!("Graph {}", tabs.len() + 1);
        let index = tabs.iter().map(|tab| tab.index).max().map_or(0, |highest| highest + 1);
        self.checkpoint(format!("New graph tab ({name}) added"));
        let swatch = SWATCHES[index % SWATCHES.len()];
        let tab = json!({
            "Name": name,
            "Index": index,
            "Color": swatch,
            "ZoomFactor": 1.0,
            "ViewportLocation": {"X": 26000.0, "Y": 26000.0},
        });
        let extra = &mut self.project.terrain_mut().extra;
        let tabs = extra.entry("GraphTabs".to_string()).or_insert_with(|| json!([]));
        if let Value::Array(tabs) = tabs {
            tabs.push(tab);
        }
        index
    }

    pub fn rename_graph_tab(&mut self, index: usize, name: &str) {
        let unchanged = self.graph_tabs().iter().any(|tab| tab.index == index && tab.name == name);
        if unchanged || name.trim().is_empty() {
            return;
        }
        self.checkpoint("Rename graph tab");
        if let Some(tab) = self.graph_tab_mut(index) {
            tab.insert("Name".to_string(), json!(name));
        }
    }

    pub fn set_graph_tab_color(&mut self, index: usize, color: &str) {
        self.checkpoint("Graph tab color");
        if let Some(tab) = self.graph_tab_mut(index) {
            tab.insert("Color".to_string(), json!(color));
        }
    }

    /// `GraphArea.DeleteGraphTab`: the tab goes with its nodes and notes;
    /// returns the tab to show next (the one before, else the first).
    pub fn delete_graph_tab(&mut self, index: usize) -> usize {
        let tabs = self.graph_tabs();
        if tabs.len() <= 1 {
            return index;
        }
        self.checkpoint("Graph tab deleted");
        let doomed: Vec<i32> = self.terrain().nodes.values().filter(|node| node_tab(node) == index).map(|node| node.id).collect();
        let terrain = self.project.terrain_mut();
        terrain.nodes.retain(|id, _| !doomed.contains(id));
        for node in terrain.nodes.values_mut() {
            for port in node.ports.iter_mut() {
                if port.record.as_ref().is_some_and(|record| doomed.contains(&record.from)) {
                    port.record = None;
                }
            }
        }
        if let Some(Value::Object(notes)) = terrain.extra.get_mut("Notes") {
            notes.retain(|_, note| note.get("GraphIndex").and_then(Value::as_u64).unwrap_or(0) as usize != index);
        }
        if let Some(Value::Array(list)) = terrain.extra.get_mut("GraphTabs") {
            list.retain(|tab| tab.as_object().map(tab_index) != Some(index));
        }
        for id in doomed {
            self.mark_stale(id);
        }
        // The tab's groups go too (`DeleteGroupElement` for each).
        self.remove_tab_groups(index);
        let remaining = self.graph_tabs();
        let lowest = remaining.iter().map(|tab| tab.index).min().unwrap_or(0);
        let previous = index.saturating_sub(1).max(lowest);
        if remaining.iter().any(|tab| tab.index == previous) { previous } else { lowest }
    }

    /// The asset's `Automation` as saved, with its collections in place.
    fn automation_json_mut(&mut self) -> &mut serde_json::Map<String, Value> {
        let extra = &mut self.project.assets[0].extra;
        let automation = extra.entry("Automation".to_string()).or_insert_with(|| json!({}));
        if !automation.is_object() {
            *automation = json!({});
        }
        let Value::Object(members) = automation else {
            unreachable!("Automation was just made an object");
        };
        members.entry("Bindings".to_string()).or_insert_with(|| json!([]));
        members.entry("Expressions".to_string()).or_insert_with(|| json!({}));
        members.entry("Variables".to_string()).or_insert_with(|| json!({}));
        members
    }

    /// Re-reads the automation the builds use from what will be saved.
    fn refresh_automation(&mut self) {
        let asset = &mut self.project.assets[0];
        let saved = asset.extra.get("Automation").cloned().unwrap_or(Value::Null);
        let references = nerthus_engine::json::References::collect(&saved);
        asset.automation = nerthus_engine::automation::Automation::from_json(&saved, &references);
    }

    /// The automation the builds use.
    pub fn automation(&self) -> &nerthus_engine::automation::Automation {
        &self.project.assets[0].automation
    }

    fn remove_binding(&mut self, node: i32, property: &str) -> Option<String> {
        let automation = self.automation_json_mut();
        let Some(Value::Array(bindings)) = automation.get_mut("Bindings") else {
            return None;
        };
        let matches = |binding: &Value| binding.get("Node").and_then(Value::as_i64) == Some(node as i64) && binding.get("Property").and_then(Value::as_str) == Some(property);
        let position = bindings.iter().position(matches)?;
        let removed = bindings.remove(position);
        removed.get("Variable").and_then(Value::as_str).map(str::to_string)
    }

    /// `EditorMenu.Mi_Click`: a new variable named after the property
    /// (`Property_NodeId` when that name is taken), bound to it.
    pub fn bind_new_variable(&mut self, node: i32, property: &str) {
        let taken = self.automation().variables.contains_key(property);
        let key = if taken { format!("{property}_{node}") } else { property.to_string() };
        self.bind_variable(node, property, &key);
    }

    /// `EditorMenu.BindToVariable`: replaces any binding of the property,
    /// writes the variable from the property and binds it.
    pub fn bind_variable(&mut self, node: i32, property: &str, key: &str) {
        if self.variable_for_binding(node, property, key).is_none() {
            return;
        }
        self.checkpoint("Bind variable");
        self.write_binding(node, property, key);
    }

    /// `PropertyArea.BindProperties_OnClick`: each parameter of the node
    /// not bound yet gets a new variable named after it (`Name_NodeId` when
    /// the name is taken) and is bound to it, bar those with editors of
    /// their own (custom editors, gradients, commands, computed text, range
    /// clippers). Returns how many were bound.
    pub fn bind_all_properties(&mut self, node: i32) -> usize {
        let Some(data) = self.node(node) else {
            return 0;
        };
        let Some(spec) = Catalogue::get().node(&data.type_name) else {
            return 0;
        };
        const OWN_EDITORS: [&str; 6] = ["Custom", "GradientEditor", "Command", "CommandButton", "ComputedString", "RangeClipper"];
        let mut properties = Vec::new();
        for parameter in &spec.parameters {
            let bound = self.automation().bindings.iter().any(|binding| binding.node == node && binding.property == parameter.name);
            let own_editor = OWN_EDITORS.contains(&parameter.kind.as_str()) || parameter.editor.is_some() || parameter.value_type == nerthus_engine::ValueType::Float4List;
            if !bound && !own_editor {
                properties.push(parameter.name.clone());
            }
        }
        if properties.is_empty() {
            return 0;
        }
        self.checkpoint("Bind all properties");
        for property in &properties {
            let taken = self.automation().variables.contains_key(property);
            let key = if taken { format!("{property}_{node}") } else { property.clone() };
            self.write_binding(node, property, &key);
        }
        properties.len()
    }

    /// The variable a binding of `property` to `key` writes,
    /// tagged for the bridges' paths.
    fn variable_for_binding(&self, node: i32, property: &str, key: &str) -> Option<Value> {
        let data = self.node(node)?;
        let spec = Catalogue::get().node(&data.type_name)?;
        let parameter = spec.parameter(property)?;
        let current = data.values.get(property).cloned().unwrap_or(Value::Null);
        let order = self.automation().variables.len();
        let mut variable = nerthus_engine::automation::variable_for_property(key, parameter, &current, spec.defaults.get(property), order);
        let tag = match (data.type_name.as_str(), property) {
            ("File", "FileName") => Some("1"),
            ("Export", "OutputPath") => Some("2"),
            _ => None,
        };
        if let Some(tag) = tag {
            variable["ExtraData"] = json!(tag);
        }
        Some(variable)
    }

    /// Binds `property` to the variable `key`, written afresh from the
    /// property, in place of any binding it had.
    fn write_binding(&mut self, node: i32, property: &str, key: &str) {
        let Some(variable) = self.variable_for_binding(node, property, key) else {
            return;
        };
        self.remove_binding(node, property);
        let automation = self.automation_json_mut();
        if let Some(Value::Object(variables)) = automation.get_mut("Variables") {
            variables.insert(key.to_string(), variable);
        }
        if let Some(Value::Array(bindings)) = automation.get_mut("Bindings") {
            bindings.push(json!({"Node": node, "Property": property, "Variable": key}));
        }
        self.refresh_automation();
        self.mark_stale(node);
    }

    /// Unbinds a property, and deletes its variable when asked (which
    /// unbinds every other property bound to it too).
    pub fn unbind(&mut self, node: i32, property: &str, delete_variable: bool) {
        let bound = self.automation().bindings.iter().any(|binding| binding.node == node && binding.property == property);
        if !bound {
            return;
        }
        self.checkpoint("Unbind variable");
        let variable = self.remove_binding(node, property);
        if let (true, Some(variable)) = (delete_variable, variable) {
            let automation = self.automation_json_mut();
            if let Some(Value::Object(variables)) = automation.get_mut("Variables") {
                variables.remove(&variable);
            }
            if let Some(Value::Array(bindings)) = automation.get_mut("Bindings") {
                bindings.retain(|binding| binding.get("Variable").and_then(Value::as_str) != Some(variable.as_str()));
            }
        }
        self.refresh_automation();
        self.mark_stale(node);
    }

    /// The property's expression (`Expressions["{node}_{property}"]`).
    pub fn expression(&self, node: i32, property: &str) -> String {
        let key = format!("{node}_{property}");
        self.automation().expressions.get(&key).cloned().unwrap_or_default()
    }

    /// Sets or (when empty) removes a property's expression.
    pub fn set_expression(&mut self, node: i32, property: &str, text: &str) {
        if self.expression(node, property) == text.trim() {
            return;
        }
        self.checkpoint("Expression");
        let key = format!("{node}_{property}");
        let automation = self.automation_json_mut();
        if let Some(Value::Object(expressions)) = automation.get_mut("Expressions") {
            if text.trim().is_empty() {
                expressions.remove(&key);
            } else {
                expressions.insert(key, json!(text.trim()));
            }
        }
        self.refresh_automation();
        self.mark_stale(node);
    }

    /// `BtnAddVar_OnClick`: a new variable for a Create Variable entry
    /// (`Float`, `Int`, `Range`, `Choice`, `Bool`, `String`, `Color`,
    /// `Undefined`, `Separator` or `Group`) named `Var_` and three hex
    /// digits (`Separator_`, `Group_` for decorations), last in order.
    /// Returns its key.
    pub fn add_variable(&mut self, kind: &str) -> String {
        let suffix = format!("{:03x}", self.random.below(0x1000));
        let (prefix, var_type, label) = match kind {
            "Separator" => ("Separator_", "Decoration", "variable separator".to_string()),
            "Group" => ("Group_", "Decoration", "variable group".to_string()),
            "Float" | "Int" | "Range" | "Choice" | "Bool" | "Color" => ("Var_", kind, format!("variable Var_{suffix}")),
            _ => ("Var_", "Undefined", format!("variable Var_{suffix}")),
        };
        let key = format!("{prefix}{suffix}");
        let order = self.automation().variables.len();
        // Value, then Min and Max, then ExtraData.
        let (value, limits, extra) = match kind {
            "Float" => (json!(0.0), Some((json!(0.0), json!(1.0))), None),
            "Int" => (json!(0), Some((json!(0), json!(10))), None),
            "Range" => {
                let range = nerthus_engine::automation::VariableValue::Float2([0.0, 1.0]);
                (range.to_json(), Some((json!(0.0), json!(1.0))), None)
            }
            "Choice" => {
                let options = json!({"$type": STRING_LIST_TYPE, "$values": ["Option 1", "Option 2", "Option 3"]});
                (json!(0), None, Some(options))
            }
            "Bool" => (json!(false), None, None),
            "Color" => (json!({"R": 0.0, "G": 0.0, "B": 0.0}), None, None),
            "Separator" => (json!("Separator"), None, None),
            "Group" => (json!("StartGroup"), None, None),
            _ => (json!("undefined_value"), None, None),
        };
        // In `Variable`'s order; zeros (`Order` 0, `Type` Undefined) and
        // nulls are not written.
        let mut variable = serde_json::Map::new();
        if order != 0 {
            variable.insert("Order".to_string(), json!(order));
        }
        variable.insert("Name".to_string(), json!(key));
        variable.insert("Value".to_string(), value);
        if var_type != "Undefined" {
            variable.insert("Type".to_string(), json!(var_type));
        }
        if let Some((min, max)) = limits {
            variable.insert("Min".to_string(), min);
            variable.insert("Max".to_string(), max);
        }
        if let Some(extra) = extra {
            variable.insert("ExtraData".to_string(), extra);
        }
        self.checkpoint(format!("New {label} added"));
        let automation = self.automation_json_mut();
        if let Some(Value::Object(variables)) = automation.get_mut("Variables") {
            variables.insert(key.clone(), Value::Object(variable));
        }
        self.refresh_automation();
        key
    }

    /// A variable's value from its editor; consecutive edits of the same
    /// variable are one undo step. Every node bound to it rebuilds.
    pub fn set_variable_value(&mut self, key: &str, value: Value) {
        let saved = self.automation().variables.get(key).and_then(|variable| variable.json.get("Value").cloned());
        if saved.as_ref() == Some(&value) {
            return;
        }
        let merge_key = (-1, key.to_string());
        let continues = self.undo.last().is_some_and(|entry| entry.merge_key.as_ref() == Some(&merge_key));
        if !(continues && self.redo.is_empty()) {
            self.push_undo(format!("Change {key}"), Some(merge_key));
        }
        self.dirty = true;
        let automation = self.automation_json_mut();
        let variable = automation.get_mut("Variables").and_then(|variables| variables.get_mut(key));
        if let Some(Value::Object(members)) = variable {
            members.insert("Value".to_string(), value);
        }
        self.refresh_automation();
        let bound: Vec<i32> = self.automation().bindings.iter().filter(|binding| binding.variable == key).map(|binding| binding.node).collect();
        for node in bound {
            self.mark_stale(node);
        }
    }

    /// The variable goes, and every binding to it
    /// ("variable X deleted", or "variable separator deleted" and
    /// "variable group deleted" for decorations).
    pub fn delete_variable(&mut self, key: &str) {
        let Some(variable) = self.automation().variables.get(key) else {
            return;
        };
        let decoration = variable.var_type == nerthus_engine::automation::VarType::Decoration;
        let subject = match variable.value.to_text().as_str() {
            "Separator" if decoration => "variable separator".to_string(),
            "StartGroup" if decoration => "variable group".to_string(),
            _ if decoration => String::new(),
            _ => format!("variable {key}"),
        };
        self.checkpoint(format!("{subject} deleted"));
        self.remove_variable(key);
        self.refresh_automation();
    }

    /// Vars > Delete all variables.
    pub fn delete_all_variables(&mut self) {
        let keys: Vec<String> = self.automation().variables.keys().cloned().collect();
        if keys.is_empty() {
            return;
        }
        self.checkpoint("All variables deleted");
        for key in keys {
            self.remove_variable(&key);
        }
        self.refresh_automation();
    }

    fn remove_variable(&mut self, key: &str) {
        let bound: Vec<i32> = self.automation().bindings.iter().filter(|binding| binding.variable == key).map(|binding| binding.node).collect();
        let automation = self.automation_json_mut();
        if let Some(Value::Object(variables)) = automation.get_mut("Variables") {
            variables.remove(key);
        }
        if let Some(Value::Array(bindings)) = automation.get_mut("Bindings") {
            bindings.retain(|binding| binding.get("Variable").and_then(Value::as_str) != Some(key));
        }
        for node in bound {
            self.mark_stale(node);
        }
    }

    /// `StackPanel_Drop`: the variables take the order of `keys`, numbered
    /// from 1 like the rows of the Vars tab's edit mode (its hint is entry
    /// 0). Nothing rebuilds: the order only arranges the variables' editors.
    pub fn rearrange_variables(&mut self, keys: &[String]) {
        self.checkpoint("variables rearranged");
        let automation = self.automation_json_mut();
        if let Some(Value::Object(variables)) = automation.get_mut("Variables") {
            for (index, key) in keys.iter().enumerate() {
                let Some(Value::Object(members)) = variables.get_mut(key) else {
                    continue;
                };
                let order = json!(index + 1);
                if let Some(saved) = members.get_mut("Order") {
                    *saved = order;
                    continue;
                }
                // `Order` is `Variable`'s first member.
                let first = members.keys().take_while(|name| name.starts_with('$')).count();
                members.shift_insert(first, "Order".to_string(), order);
            }
        }
        self.refresh_automation();
    }

    /// `TerrainState.GroupVariablesByNodes`.
    pub fn group_variables_by_nodes(&self) -> bool {
        let state = self.project.assets[0].extra.get("State");
        state.and_then(|state| state.get("GroupVariablesByNodes")).and_then(Value::as_bool).unwrap_or(false)
    }

    pub fn set_group_variables_by_nodes(&mut self, grouped: bool) {
        let extra = &mut self.project.assets[0].extra;
        let state = extra.entry("State").or_insert_with(|| json!({}));
        if let Value::Object(members) = state {
            if grouped {
                members.insert("GroupVariablesByNodes".to_string(), json!(true));
            } else {
                members.shift_remove("GroupVariablesByNodes");
            }
        }
        self.dirty = true;
    }

    /// The nodes to another tab, nothing else changed (test projects; the
    /// editor's Move to Tab is `move_elements_to_tab`).
    #[cfg(test)]
    pub fn move_nodes_to_tab(&mut self, ids: &[i32], index: usize) {
        if ids.is_empty() {
            return;
        }
        self.checkpoint("Move to tab");
        let terrain = self.project.terrain_mut();
        for id in ids {
            if let Some(node) = terrain.nodes.get_mut(id) {
                node.set_member("GraphIndex", json!(index));
            }
        }
    }

    // Exports (`SaveDefinition`) and the build definition.

    /// The node's export settings, if it is marked for export.
    pub fn export_settings(&self, id: i32) -> Option<ExportSettings> {
        let node = self.node(id)?;
        ExportSettings::of(node)
    }

    fn set_export_settings(&mut self, id: i32, settings: Option<ExportSettings>) {
        let Some(node) = self.project.terrain_mut().nodes.get_mut(&id) else {
            return;
        };
        match settings {
            Some(settings) => {
                node.base.insert("SaveDefinition".to_string(), settings.to_json());
            }
            None => {
                node.base.shift_remove("SaveDefinition");
            }
        }
    }

    /// Mark for Export on each node: a marked node is unmarked, an
    /// unmarked one marked (enabled, named after the node, in the default
    /// heightfield format). Returns each node's name and whether it is now
    /// marked.
    pub fn toggle_export(&mut self, ids: &[i32], format: FileFormat) -> Vec<(String, bool)> {
        self.checkpoint("Node(s) marked for export.");
        let mut changes = Vec::new();
        for &id in ids {
            let Some(node) = self.node(id) else {
                continue;
            };
            let name = node.name.clone();
            let marked = ExportSettings::of(node).is_some();
            if marked {
                self.set_export_settings(id, None);
            } else {
                let settings = ExportSettings {
                    node: id,
                    filename: name.clone(),
                    format,
                    enabled: true,
                    disabled_in_profiles: Vec::new(),
                };
                self.set_export_settings(id, Some(settings));
                self.rename_duplicate_export(id);
            }
            changes.push((name, !marked));
        }
        changes
    }

    /// A marked node whose file name another exporting
    /// node already uses is renamed `Name-Id`.
    fn rename_duplicate_export(&mut self, id: i32) {
        let Some(settings) = self.export_settings(id) else {
            return;
        };
        let terrain = self.terrain();
        let taken = terrain.nodes.values().any(|other| {
            let same_name = ExportSettings::of(other).is_some_and(|other_settings| other_settings.filename == settings.filename);
            other.id != id && nerthus_export::build::exports(other) && same_name
        });
        if !taken {
            return;
        }
        let name = self.node(id).map(|node| node.name.clone()).unwrap_or_default();
        let renamed = ExportSettings { filename: format!("{name}-{id:03}"), ..settings };
        self.set_export_settings(id, Some(renamed));
    }

    /// Whether two exporting nodes would write the same file name.
    pub fn has_duplicate_export_names(&self) -> bool {
        let mut names = HashSet::new();
        for node in self.terrain().nodes.values() {
            let Some(settings) = ExportSettings::of(node) else {
                continue;
            };
            if !names.insert(settings.filename) {
                return true;
            }
        }
        false
    }

    /// Every duplicate file name fixed.
    pub fn fix_duplicate_export_names(&mut self) {
        self.checkpoint("Fix duplicate filenames");
        let exporting = nerthus_export::build::exporting_nodes(self.terrain());
        for id in exporting {
            self.rename_duplicate_export(id);
        }
    }

    /// A renamed node's export file renamed with it ("Update Save
    /// Definition"). It has no undo step of its own: undoing the node's
    /// rename puts the file name back too.
    pub fn rename_export_file(&mut self, id: i32, filename: &str) {
        let Some(settings) = self.export_settings(id) else {
            return;
        };
        let renamed = ExportSettings { filename: filename.to_string(), ..settings };
        self.set_export_settings(id, Some(renamed));
        self.dirty = true;
    }

    /// Changes one node's export settings.
    pub fn edit_export(&mut self, id: i32, label: &str, edit: impl FnOnce(&mut ExportSettings)) {
        let Some(mut settings) = self.export_settings(id) else {
            return;
        };
        self.checkpoint(label);
        edit(&mut settings);
        self.set_export_settings(id, Some(settings));
    }

    /// "Change formats for all nodes".
    pub fn set_all_export_formats(&mut self, format: FileFormat) {
        self.checkpoint("Change formats");
        let marked: Vec<i32> = self.terrain().nodes.values().filter(|node| ExportSettings::of(node).is_some()).map(|node| node.id).collect();
        for id in marked {
            if let Some(mut settings) = self.export_settings(id) {
                settings.format = format;
                self.set_export_settings(id, Some(settings));
            }
        }
    }

    /// Whether an output port exports (`Port.IsExporting`).
    pub fn set_port_exporting(&mut self, id: i32, port: &str, exporting: bool) {
        self.checkpoint("Export ports");
        let Some(node) = self.project.terrain_mut().nodes.get_mut(&id) else {
            return;
        };
        if let Some(target) = node.ports.iter_mut().find(|candidate| candidate.name == port) {
            target.is_exporting = exporting;
        }
    }

    /// The asset's build definition as builds read it.
    pub fn build_definition(&self) -> BuildDefinition {
        BuildDefinition::from_json(self.project.assets[0].extra.get("BuildDefinition"))
    }

    /// One `BuildDefinition` member, as projects save it: in the class's
    /// order, left out at its type's default.
    pub fn set_build_setting(&mut self, member: &str, value: Value) {
        self.set_build_settings(&[(member, value)]);
    }

    /// Several `BuildDefinition` members as one edit.
    pub fn set_build_settings(&mut self, members: &[(&str, Value)]) {
        let changed: Vec<&(&str, Value)> = members.iter().filter(|(member, value)| self.build_member_differs(member, value)).collect();
        if changed.is_empty() {
            return;
        }
        self.checkpoint("Build settings");
        for (member, value) in changed {
            self.write_build_member(member, value.clone());
        }
    }

    fn build_member_differs(&self, member: &str, value: &Value) -> bool {
        let current = self.project.assets[0].extra.get("BuildDefinition").and_then(|definition| definition.get(member));
        if is_newtonsoft_default(member, value) {
            return current.is_some();
        }
        current != Some(value)
    }

    fn write_build_member(&mut self, member: &str, value: Value) {
        let removing = is_newtonsoft_default(member, &value);
        let extra = &mut self.project.assets[0].extra;
        let definition = extra.entry("BuildDefinition").or_insert_with(|| json!({}));
        let Some(members) = definition.as_object_mut() else {
            return;
        };
        if removing {
            members.shift_remove(member);
        } else {
            members.insert(member.to_string(), value);
        }
        let mut ordered = serde_json::Map::new();
        for name in BUILD_DEFINITION_MEMBERS {
            if let Some(value) = members.get(name) {
                ordered.insert(name.to_string(), value.clone());
            }
        }
        for (name, value) in members.iter() {
            if !ordered.contains_key(name) {
                ordered.insert(name.clone(), value.clone());
            }
        }
        *members = ordered;
    }

    /// `Terrain.Regions`.
    pub fn regions(&self) -> Vec<Region> {
        region::regions(self.terrain())
    }

    /// `RegionManager.AddNewRegion`: a region a quarter of the terrain
    /// wide with its corner at `corner`, in a random swatch's colour
    /// (`ElementHelper.GetRandomSwatch`, kept as scRGB), named "Region 001"
    /// or, while that is taken, the next number (at most 100 tries).
    /// Returns its name.
    pub fn add_region(&mut self, corner: [f32; 2]) -> String {
        let mut regions = self.regions();
        let mut name = "Region 001".to_string();
        let mut tries = 0;
        while regions.iter().any(|region| region.name == name) && tries < 100 {
            tries += 1;
            name = format!("Region {}", nerthus_export::destination::next_counter(&name));
        }
        let swatch = SWATCHES[self.random.below(SWATCHES.len() as u64) as usize];
        let mut added = Region::new(&name);
        added.width = 0.25;
        added.position = corner;
        added.color = crate::theme::sc_rgb(crate::theme::swatch_color(swatch));
        regions.push(added);
        self.checkpoint("New region added");
        self.store_regions(&regions);
        name
    }

    /// `RegionManager.Rc_RegionDeletion`. The build definition keeps its
    /// `Portion` even when it is this region.
    pub fn delete_region(&mut self, name: &str) {
        let mut regions = self.regions();
        regions.retain(|region| region.name != name);
        self.checkpoint(format!("Region deleted {name}"));
        self.store_regions(&regions);
    }

    /// A region moved, resized, renamed or recoloured. There is no undo
    /// step for these (only for adding and deleting regions). The build
    /// definition's `Portion` follows its region: once saved and read back
    /// they are one object.
    pub fn update_region(&mut self, name: &str, updated: Region) {
        let mut regions = self.regions();
        let Some(slot) = regions.iter_mut().find(|region| region.name == name) else {
            return;
        };
        if *slot == updated {
            return;
        }
        let new_name = updated.name.clone();
        *slot = updated;
        self.store_regions(&regions);
        let follows = self.build_definition().portion.is_some_and(|portion| portion.name == name);
        let stored = self.regions().into_iter().find(|region| region.name == new_name);
        if follows && let Some(stored) = stored {
            self.write_build_member("Portion", region::portion_json(&stored));
        }
    }

    /// The regions as projects save them, `WorldWidth` (a computed member)
    /// as it is now.
    fn store_regions(&mut self, regions: &[Region]) {
        let world_width = self.build_definition().world_resolution();
        let mut stamped = regions.to_vec();
        for region in stamped.iter_mut() {
            region.set_world_width(world_width);
        }
        region::set_regions(self.project.terrain_mut(), &stamped);
        self.dirty = true;
    }

    /// The terrain's size in metres (`Terrain.Width`, `Terrain.Height`).
    pub fn set_terrain_size(&mut self, width: f32, height: f32) {
        let terrain = self.terrain();
        if terrain.width == width && terrain.height == height {
            return;
        }
        self.checkpoint("Terrain size");
        let terrain = self.project.terrain_mut();
        terrain.width = width;
        terrain.height = height;
        self.mark_all_stale();
    }
}

/// `BuildDefinition`'s members in declaration order (how Newtonsoft
/// writes them).
pub(crate) const BUILD_DEFINITION_MEMBERS: [&str; 26] = [
    "Type",
    "Portion",
    "Destination",
    "OverwriteMode",
    "Subdivision",
    "Resolution",
    "BakeResolution",
    "TileResolution",
    "BucketResolution",
    "OutputType",
    "NumberOfTiles",
    "PreserveTiledCache",
    "EdgeBlending",
    "FlipY",
    "OverlappingPixels",
    "TilePadDigits",
    "TileZeroIndex",
    "TilePattern",
    "OrganizeFiles",
    "PersistOnSave",
    "MaintainPrimeFolder",
    "PostBuildScript",
    "OpenFolder",
    "CopyTerrain",
    "RemovePrimaryPortName",
    "ColorSpace",
];

/// Whether `IgnoreAndPopulate` leaves a `BuildDefinition` member out: false,
/// zero, null, or an enum's first member.
fn is_newtonsoft_default(member: &str, value: &Value) -> bool {
    let first_members = [("OverwriteMode", "Overwrite"), ("Subdivision", "None"), ("OutputType", "SingleImage"), ("OrganizeFiles", "None"), ("ColorSpace", "Undefined")];
    match value {
        Value::Null => true,
        Value::Bool(flag) => !flag,
        Value::Number(number) => number.as_f64() == Some(0.0),
        Value::String(text) => first_members.iter().any(|(name, first)| *name == member && text == first),
        _ => false,
    }
}

/// `SetUnderlay`'s test: not rendered as colour, and a heightfield by
/// override or by not being soft data (`IsSoftData`: masks and
/// colorizers).
fn can_be_underlay(node: &NodeData) -> bool {
    let intent_override = node.base.get("RenderIntentOverride").and_then(Value::as_str).unwrap_or("Default");
    let spec = Catalogue::get().node(&node.type_name);
    let classification = spec.and_then(|spec| spec.classification.as_deref()).unwrap_or_default();
    let soft_data = matches!(classification, "Mask" | "Colorizer");
    intent_override != "Color" && (intent_override == "Heightfield" || !soft_data)
}

/// A node's `SaveDefinition`.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportSettings {
    pub node: i32,
    pub filename: String,
    pub format: FileFormat,
    pub enabled: bool,
    pub disabled_in_profiles: Vec<Value>,
}

impl ExportSettings {
    pub fn of(node: &NodeData) -> Option<Self> {
        let value = node.base.get("SaveDefinition").filter(|value| !value.is_null())?;
        let format_name = value.get("Format").and_then(Value::as_str);
        let profiles = value.get("DisabledInProfiles").and_then(Value::as_array).cloned().unwrap_or_default();
        let settings = Self {
            node: value.get("Node").and_then(Value::as_i64).unwrap_or(0) as i32,
            filename: value.get("Filename").and_then(Value::as_str).unwrap_or_default().to_string(),
            format: format_name.and_then(FileFormat::from_name).unwrap_or(FileFormat::PureRaw),
            enabled: value.get("IsEnabled").and_then(Value::as_bool).unwrap_or(false),
            disabled_in_profiles: profiles,
        };
        Some(settings)
    }

    /// As projects save it: members at their default left out.
    pub(crate) fn to_json(&self) -> Value {
        let mut members = serde_json::Map::new();
        if self.node != 0 {
            members.insert("Node".to_string(), json!(self.node));
        }
        members.insert("Filename".to_string(), json!(self.filename));
        if self.format != FileFormat::PureRaw {
            members.insert("Format".to_string(), json!(self.format.name()));
        }
        if self.enabled {
            members.insert("IsEnabled".to_string(), json!(true));
        }
        members.insert("DisabledInProfiles".to_string(), Value::Array(self.disabled_in_profiles.clone()));
        Value::Object(members)
    }
}

/// `ElementHelper.Swatches`.
pub const SWATCHES: [&str; 16] = [
    "Purple", "Brass", "LightBlue", "DarkPink", "Green", "Tan", "Blue", "Orange", "Gray", "Yellow", "Teal", "OrchidPink", "Aqua", "Umber", "LightPurple", "LightPink",
];

/// A graph tab as the editor uses it.
#[derive(Clone, Debug)]
pub struct GraphTabInfo {
    /// `GraphTab.Index`, what nodes' `GraphIndex` refers to.
    pub index: usize,
    pub name: String,
    pub color: String,
    pub zoom: f32,
    /// `ViewportLocation`, when the file has one.
    pub location: Option<[f32; 2]>,
}

impl GraphTabInfo {
    fn from_json(value: &Value) -> Self {
        let location = value.get("ViewportLocation").map(|point| {
            let x = point.get("X").and_then(Value::as_f64).unwrap_or(26000.0) as f32;
            let y = point.get("Y").and_then(Value::as_f64).unwrap_or(26000.0) as f32;
            [x, y]
        });
        Self {
            index: value.get("Index").and_then(Value::as_u64).unwrap_or(0) as usize,
            name: value.get("Name").and_then(Value::as_str).unwrap_or("Graph").to_string(),
            color: value.get("Color").and_then(Value::as_str).unwrap_or("Brass").to_string(),
            zoom: value.get("ZoomFactor").and_then(Value::as_f64).unwrap_or(1.0) as f32,
            location,
        }
    }
}

fn tab_index(tab: &serde_json::Map<String, Value>) -> usize {
    tab.get("Index").and_then(Value::as_u64).unwrap_or(0) as usize
}

/// `Node.GraphIndex`.
pub fn node_tab(node: &NodeData) -> usize {
    node.base.get("GraphIndex").and_then(Value::as_u64).unwrap_or(0) as usize
}

/// A hash of everything about a node that affects its build.
fn node_fingerprint(node: &NodeData) -> u64 {
    let mut hasher = DefaultHasher::new();
    node.type_name.hash(&mut hasher);
    let values = serde_json::to_string(&node.values).unwrap_or_default();
    values.hash(&mut hasher);
    let base = serde_json::to_string(&node.base).unwrap_or_default();
    base.hash(&mut hasher);
    for port in &node.ports {
        port.name.hash(&mut hasher);
        if let Some(record) = &port.record {
            record.from.hash(&mut hasher);
            record.from_port.hash(&mut hasher);
        }
    }
    for modifier in &node.modifiers {
        modifier.type_name.hash(&mut hasher);
        let modifier_values = serde_json::to_string(&modifier.values).unwrap_or_default();
        modifier_values.hash(&mut hasher);
    }
    hasher.finish()
}

/// The project date format (`yyyy-MM-dd HH:mm:ssZ`, UTC).
fn project_timestamp() -> String {
    let now = chrono::Utc::now();
    now.format("%Y-%m-%d %H:%M:%SZ").to_string()
}

fn stamp_saved(extra: &mut serde_json::Map<String, Value>, now: &str) {
    let Some(Value::Object(metadata)) = extra.get_mut("Metadata") else {
        return;
    };
    metadata.insert("DateLastSaved".to_string(), json!(now));
    metadata.insert("ModifiedVersion".to_string(), json!(FORMAT_VERSION));
}

/// The project `new Project()` gives: one terrain asset with the default
/// size and first graph tab (`new Terrain()`), build and view settings.
fn blank_project(random: &mut SmallRandom) -> Project {
    let now = project_timestamp();
    let metadata = json!({
        "Name": "",
        "Description": "",
        "Version": FORMAT_VERSION,
        "DateCreated": now,
        "DateLastBuilt": now,
        "DateLastSaved": now,
        "ModifiedVersion": FORMAT_VERSION,
    });
    let mut project_metadata = metadata.clone();
    project_metadata["Owner"] = json!("");
    let project_id = format!("{:08x}", random.next() as u32);
    let document = json!({
        "Assets": [{
            "Terrain": {
                "Id": random_guid(random),
                "Metadata": metadata,
                "Nodes": {},
                "Groups": {},
                "Notes": {},
                "GraphTabs": [{"Name": "Graph 1", "Color": "Brass", "ZoomFactor": 1.0, "ViewportLocation": {"X": 26000.0, "Y": 26000.0}}],
                "Width": 5000.0,
                "Height": 2500.0,
            },
            "Automation": {"Bindings": [], "Expressions": {}, "VariablesEx": {}, "Variables": {}},
            "BuildDefinition": {
                "Type": "Standard",
                "Resolution": 2048,
                "BakeResolution": 2048,
                "TileResolution": 1024,
                "BucketResolution": 2048,
                "NumberOfTiles": 3,
                "EdgeBlending": 0.25,
                "TileZeroIndex": true,
                "TilePattern": "_y%Y%_x%X%",
                "OrganizeFiles": "NodeSubFolder",
                "ColorSpace": "sRGB",
            },
            "State": {
                "BakeResolution": 2048,
                "PreviewResolution": 1024,
                "NodeBookmarks": [],
                "Viewport": {"Camera": {}, "RenderMode": "Realistic", "AmbientOcclusion": true, "Shadows": true},
            },
        }],
        "Id": project_id,
        "Branch": 1,
        "Metadata": project_metadata,
    });
    let text = document.to_string();
    Project::parse(&text).expect("the blank project template parses")
}

fn random_guid(random: &mut SmallRandom) -> String {
    let high = random.next();
    let low = random.next();
    let bytes: Vec<u8> = high.to_be_bytes().into_iter().chain(low.to_be_bytes()).collect();
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{}-{}-4{}-a{}-{}", &hex[0..8], &hex[8..12], &hex[13..16], &hex[17..20], &hex[20..32])
}

/// A small xorshift generator for node ids and seeds.
struct SmallRandom {
    state: u64,
}

impl SmallRandom {
    fn from_time() -> Self {
        let nanos = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|duration| duration.as_nanos() as u64).unwrap_or(0x9e37_79b9_7f4a_7c15);
        Self { state: nanos | 1 }
    }

    fn next(&mut self) -> u64 {
        let mut value = self.state;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.state = value;
        value
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound.max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_file(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|duration| duration.as_nanos()).unwrap_or(0);
        std::env::temp_dir().join(format!("nerthus-{name}-{nanos}.terrain"))
    }

    fn chain() -> (Document, i32, i32) {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [25100.0, 25100.0]).unwrap();
        let erosion = document.add_node("ErosionPro", [25400.0, 25100.0]).unwrap();
        document.connect(mountain, "Out", erosion, "In");
        (document, mountain, erosion)
    }

    #[test]
    fn node_sizes_cycle_like_cycle_node_sizes() {
        let (mut document, mountain, erosion) = chain();
        let size = |document: &Document, id: i32| document.node(id).unwrap().base.get("NodeSize").and_then(Value::as_str).unwrap_or("Default").to_string();
        // Two ports or fewer each way: Standard and Small swap.
        document.cycle_node_size(mountain, "Standard");
        assert_eq!(size(&document, mountain), "Small");
        document.cycle_node_size(mountain, "Standard");
        assert_eq!(size(&document, mountain), "Standard");
        // ErosionPro has four outputs (`IsBigNode`): Standard, Small, Compact.
        document.cycle_node_size(erosion, "Standard");
        assert_eq!(size(&document, erosion), "Small");
        document.cycle_node_size(erosion, "Standard");
        assert_eq!(size(&document, erosion), "Compact");
        document.cycle_node_size(erosion, "Standard");
        assert_eq!(size(&document, erosion), "Standard");
    }

    #[test]
    fn bookmarks_and_pasted_properties() {
        let (mut document, mountain, erosion) = chain();
        document.add_bookmark(erosion);
        document.add_bookmark(mountain);
        document.add_bookmark(erosion);
        assert_eq!(document.bookmarks(), vec![erosion, mountain]);
        document.remove_bookmark(erosion, "bookmarks");
        assert_eq!(document.bookmarks(), vec![mountain]);
        assert_eq!(document.undo_labels()[0], "Node ErosionPro removed from bookmarks");

        let other = document.add_node("Mountain", [25100.0, 25400.0]).unwrap();
        let mut values = document.node(mountain).unwrap().values.clone();
        values.insert("Scale".to_string(), json!(0.25));
        document.paste_properties(&[other, erosion], "Mountain", &values);
        assert_eq!(document.node(other).unwrap().values["Scale"], json!(0.25));
        // Only nodes of the source's type take them.
        assert!(!document.node(erosion).unwrap().values.contains_key("Scale"));
    }

    #[test]
    fn marking_for_export_toggles_and_keeps_names_apart() {
        let (mut document, mountain, erosion) = chain();
        document.rename(erosion, "Mountain");
        let marked = document.toggle_export(&[mountain, erosion], FileFormat::Exr);
        assert_eq!(marked, vec![("Mountain".to_string(), true), ("Mountain".to_string(), true)]);
        let first = document.export_settings(mountain).unwrap();
        let second = document.export_settings(erosion).unwrap();
        assert_eq!(first.filename, "Mountain");
        assert_eq!(first.format, FileFormat::Exr);
        assert!(first.enabled);
        // The second keeps its node apart as `Name-Id`.
        assert_eq!(second.filename, format!("Mountain-{erosion:03}"));
        assert!(!document.has_duplicate_export_names());

        // Saved with `IsEnabled` and the format written, the empty profile
        // list kept.
        let saved = document.node(mountain).unwrap().base.get("SaveDefinition").cloned().unwrap();
        assert_eq!(saved, json!({"Node": mountain, "Filename": "Mountain", "Format": "EXR", "IsEnabled": true, "DisabledInProfiles": []}));
        document.edit_export(mountain, "Export format", |settings| settings.format = FileFormat::PureRaw);
        let raw = document.node(mountain).unwrap().base.get("SaveDefinition").cloned().unwrap();
        assert_eq!(raw, json!({"Node": mountain, "Filename": "Mountain", "IsEnabled": true, "DisabledInProfiles": []}));

        let unmarked = document.toggle_export(&[mountain], FileFormat::Exr);
        assert_eq!(unmarked, vec![("Mountain".to_string(), false)]);
        assert!(document.export_settings(mountain).is_none());
        assert!(document.undo());
        assert!(document.export_settings(mountain).is_some());
    }

    #[test]
    fn build_settings_save_in_member_order() {
        let mut document = Document::new();
        document.set_build_setting("OverwriteMode", json!("Increment"));
        document.set_build_setting("Resolution", json!(4096));
        document.set_build_setting("OpenFolder", json!(true));
        let definition = document.build_definition();
        assert_eq!(definition.resolution, 4096);
        assert!(definition.open_folder);
        // Defaults are left out; members keep the class's order.
        document.set_build_setting("OverwriteMode", json!("Overwrite"));
        let saved = document.project.assets[0].extra.get("BuildDefinition").unwrap();
        let members: Vec<&String> = saved.as_object().unwrap().keys().collect();
        assert!(!members.iter().any(|name| *name == "OverwriteMode"));
        let resolution = members.iter().position(|name| *name == "Resolution").unwrap();
        let open_folder = members.iter().position(|name| *name == "OpenFolder").unwrap();
        assert!(resolution < open_folder);
    }

    #[test]
    fn a_new_project_saves_and_reopens() {
        let (mut document, mountain, erosion) = chain();
        document.set_value(erosion, "Duration", Value::from(0.25));
        let path = scratch_file("roundtrip");
        document.save(&path).unwrap();
        let reopened = Document::open(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert!(!document.dirty);
        let node = reopened.node(erosion).unwrap();
        assert_eq!(node.values["Duration"], Value::from(0.25));
        let record = node.port("In").and_then(|port| port.record.clone()).unwrap();
        assert_eq!((record.from, record.from_port.as_str()), (mountain, "Out"));
        assert_eq!(reopened.node(mountain).unwrap().position, [25100.0, 25100.0]);
    }

    #[test]
    fn undo_and_redo_walk_the_history() {
        let (mut document, _, erosion) = chain();
        assert_eq!(document.undo_labels().len(), 3);
        assert!(document.undo());
        assert!(document.node(erosion).unwrap().port("In").unwrap().record.is_none());
        assert!(document.redo());
        assert!(document.node(erosion).unwrap().port("In").unwrap().record.is_some());
        let (stale, _) = document.take_stale();
        assert!(stale.contains(&erosion));
    }

    #[test]
    fn value_edits_merge_per_node_and_property() {
        let (mut document, mountain, erosion) = chain();
        let before = document.undo_labels().len();
        document.set_value(erosion, "Duration", Value::from(0.2));
        document.set_value(erosion, "Duration", Value::from(0.3));
        assert_eq!(document.undo_labels().len(), before + 1);
        document.set_value(mountain, "Scale", Value::from(0.7));
        assert_eq!(document.undo_labels().len(), before + 2);
        document.undo();
        assert_eq!(document.node(erosion).unwrap().values["Duration"], Value::from(0.3));
    }

    #[test]
    fn connections_that_close_a_loop_are_found() {
        let (document, mountain, erosion) = chain();
        assert!(document.would_cycle(erosion, mountain));
        assert!(!document.would_cycle(mountain, erosion));
    }

    #[test]
    fn deleting_a_node_drops_connections_from_it() {
        let (mut document, mountain, erosion) = chain();
        document.delete_node(mountain);
        assert!(document.node(mountain).is_none());
        assert!(document.node(erosion).unwrap().port("In").unwrap().record.is_none());
    }

    #[test]
    fn duplicates_keep_their_own_connections() {
        let (mut document, mountain, erosion) = chain();
        let copies = document.duplicate_selection(&[mountain, erosion], 0);
        assert_eq!(copies.len(), 2);
        let erosion_copy = document.node(copies[1]).unwrap();
        let record = erosion_copy.port("In").unwrap().record.clone().unwrap();
        assert_eq!((record.from, record.to), (copies[0], copies[1]));
        let single = document.duplicate_selection(&[erosion], 0);
        let outside = document.node(single[0]).unwrap().port("In").unwrap().record.clone().unwrap();
        assert_eq!(outside.from, mountain);
    }

    /// Writes a small project to `$NERTHUS_SAMPLE_OUT` for checking a saved
    /// project by hand.
    #[test]
    #[ignore]
    fn write_sample_project() {
        let path = std::env::var("NERTHUS_SAMPLE_OUT").expect("set NERTHUS_SAMPLE_OUT");
        let (mut document, mountain, erosion) = chain();
        document.set_value(mountain, "Seed", Value::from(1234));
        document.set_value(erosion, "Seed", Value::from(4321));
        document.set_value(erosion, "Duration", Value::from(0.3));
        let blur = document.add_node("Blur", [25700.0, 25100.0]).unwrap();
        document.connect(erosion, "Out", blur, "In");
        document.save(Path::new(&path)).unwrap();
        println!("mountain {mountain} erosion {erosion} blur {blur}");
    }

    #[test]
    fn graph_tabs_come_and_go_with_their_nodes() {
        let (mut document, mountain, erosion) = chain();
        let index = document.add_graph_tab();
        assert_eq!(index, 1);
        let tabs = document.graph_tabs();
        assert_eq!((tabs[1].name.as_str(), tabs[1].color.as_str()), ("Graph 2", "Brass"));
        document.move_nodes_to_tab(&[mountain], index);
        assert_eq!(node_tab(document.node(mountain).unwrap()), 1);
        let next = document.delete_graph_tab(index);
        assert_eq!(next, 0);
        assert!(document.node(mountain).is_none());
        assert!(document.node(erosion).unwrap().port("In").unwrap().record.is_none());
        assert_eq!(document.graph_tabs().len(), 1);
        assert_eq!(document.delete_graph_tab(0), 0);
        assert_eq!(document.graph_tabs().len(), 1);
    }

    #[test]
    fn binding_writes_a_variable_and_a_binding() {
        let (mut document, _, erosion) = chain();
        document.set_value(erosion, "Duration", Value::from(0.3));
        document.bind_new_variable(erosion, "Duration");
        let automation = document.automation();
        assert_eq!(automation.bindings.len(), 1);
        assert_eq!(automation.bindings[0].variable, "Duration");
        assert_eq!(automation.variables["Duration"].value.to_single(), Ok(0.3));
        document.set_expression(erosion, "Duration", "x*2");
        assert_eq!(document.expression(erosion, "Duration"), "x*2");
        document.unbind(erosion, "Duration", true);
        assert!(document.automation().bindings.is_empty());
        assert!(document.automation().variables.is_empty());
        let path = scratch_file("automation");
        document.save(&path).unwrap();
        let reopened = Document::open(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(reopened.expression(erosion, "Duration"), "x*2");
    }

    #[test]
    fn rearranged_variables_are_numbered_from_one() {
        let mut document = Document::new();
        let first = document.add_variable("Float");
        let second = document.add_variable("Int");
        let third = document.add_variable("Separator");
        let rearranged = [third.clone(), first.clone(), second.clone()];
        document.rearrange_variables(&rearranged);
        assert_eq!(document.undo_labels()[0], "variables rearranged");
        let variables = &document.automation().variables;
        let order = |key: &str| variables[key].json.get("Order").and_then(Value::as_i64);
        assert_eq!((order(&third), order(&first), order(&second)), (Some(1), Some(2), Some(3)));
        let first_member = variables[&first].json.as_object().and_then(|members| members.keys().next().cloned());
        assert_eq!(first_member.as_deref(), Some("Order"));
        document.undo();
        let restored = &document.automation().variables[&first];
        assert!(restored.json.get("Order").is_none());

        // Saved with `Order` first after the `$id`.
        document.redo();
        let path = scratch_file("rearranged");
        document.save(&path).unwrap();
        let reopened = Document::open(&path).unwrap();
        std::fs::remove_file(&path).ok();
        let saved = &reopened.automation().variables[&first].json;
        let first_saved = saved.as_object().and_then(|members| members.keys().find(|name| !name.starts_with('$')).cloned());
        assert_eq!(first_saved.as_deref(), Some("Order"));
        assert_eq!(saved.get("Order").and_then(Value::as_i64), Some(2));
    }

    #[test]
    fn seeds_stay_below_65535() {
        let mut document = Document::new();
        for _ in 0..10_000 {
            let seed = document.random_seed();
            assert!((0..65535).contains(&seed));
        }
    }

    #[test]
    fn swapping_inputs_trades_sources() {
        let mut document = Document::new();
        let first = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let second = document.add_node("Mountain", [0.0, 200.0]).unwrap();
        let combine = document.add_node("Combine", [300.0, 0.0]).unwrap();
        document.connect(first, "Out", combine, "In");
        document.connect(second, "Out", combine, "Input2");
        document.switch_inputs(combine);
        let node = document.node(combine).unwrap();
        let input = node.port("In").unwrap().record.clone().unwrap();
        let input2 = node.port("Input2").unwrap().record.clone().unwrap();
        assert_eq!((input.from, input.to_port.as_str()), (second, "In"));
        assert_eq!((input2.from, input2.to_port.as_str()), (first, "Input2"));
    }
}
