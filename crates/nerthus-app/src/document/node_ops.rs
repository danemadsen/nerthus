//! Node edits of the graph: inserting a node into a connection or a
//! chokepoint after an output, Replace, the three ways of combining nodes,
//! Mutate, the auto layout (F4) and cycling the render intent (T).

use std::collections::HashSet;

use indexmap::IndexMap;
use nerthus_engine::{Catalogue, NodeData};
use serde_json::{json, Value};

use super::links::{grid_inputs, grid_outputs, PortalState};
use super::{node_tab, Document};

/// The ways Combine nodes puts several nodes together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombineKind {
    /// Combine as Inputs (F8, `MultiCombine`).
    Inputs,
    /// Mixer as Color Layers (Ctrl F8, `MultiMixer`).
    Mixer,
    /// Layers as Normal Layers (Ctrl Shift F8, `MultiLayers`).
    Layers,
    /// The graph toolbar's Combine nodes (`MiCombineNodes_OnClick`).
    Toolbar,
}

/// `LayerEditorPanel.brushes` as they start.
const LAYER_COLORS: [&str; 15] = [
    "#d63939", "#f76707", "#f59f00", "#82c91e", "#2fb344", "#0ca678", "#17a2b8", "#228be6", "#066fd1", "#4263eb", "#7048e8", "#ae3ec9", "#d6336c", "#e64980", "#f783ac",
];

/// The gap the auto layout leaves between nodes.
const LAYOUT_GAP: f32 = 70.0;

impl Document {
    /// A new node of `type_name` at `position` on `tab`, part of an edit
    /// already named. Returns its id.
    pub(crate) fn place_node(&mut self, type_name: &str, position: [f32; 2], tab: usize) -> Option<i32> {
        let spec = Catalogue::get().node(type_name)?;
        let id = self.new_element_id(&HashSet::new());
        let seed = self.random_seed();
        let mut node = NodeData::new(spec, id, seed);
        node.position = position;
        if tab != 0 {
            node.set_member("GraphIndex", json!(tab));
        }
        self.project.terrain_mut().nodes.insert(id, node);
        self.mark_stale(id);
        self.dirty = true;
        Some(id)
    }

    /// `CreateInterjectedNode`: "Node X inserted in connection.", a node of
    /// `type_name` whose top-left is `corner`, fed by the connection's
    /// source and feeding its input; small when asked (chokepoints).
    pub fn insert_node_in_connection(&mut self, to: i32, to_port: &str, type_name: &str, corner: [f32; 2], small: bool) -> Option<i32> {
        let record = self.node(to).and_then(|node| node.port(to_port)).and_then(|port| port.record.clone())?;
        self.checkpoint(format!("Node {type_name} inserted in connection."));
        let tab = self.node(to).map(node_tab).unwrap_or(0);
        let inserted = self.place_node(type_name, corner, tab)?;
        if small && let Some(node) = self.project.terrain_mut().nodes.get_mut(&inserted) {
            node.set_member("NodeSize", json!("Small"));
        }
        let has_in = self.node(inserted).is_some_and(|node| node.port("In").is_some());
        let has_out = self.node(inserted).is_some_and(|node| node.port("Out").is_some());
        if has_in {
            self.link(record.from, &record.from_port, inserted, "In");
        }
        if has_out {
            self.write_portal_state(to, to_port, PortalState::None);
            self.link(inserted, "Out", to, to_port);
        }
        Some(inserted)
    }

    /// The node menu's Insert chokepoint at… (`MiInsertChokePoint`): a
    /// small Chokepoint 50 right of the node takes the output and feeds
    /// what it fed; those nodes move 100 right when they stood within 100
    /// of it (the new node's `ActualWidth` is read before it has been
    /// measured, so its width counts as 0).
    pub fn insert_chokepoint_at(&mut self, node: i32, port: &str, extent: &dyn Fn(i32) -> [f32; 2]) -> Option<i32> {
        let parent = self.node(node)?.clone();
        self.checkpoint("Chokepoint inserted.");
        let width = extent(node)[0];
        let corner = [parent.position[0] + width + 50.0, parent.position[1]];
        let chokepoint = self.place_node("Chokepoint", corner, node_tab(&parent))?;
        if let Some(added) = self.project.terrain_mut().nodes.get_mut(&chokepoint) {
            added.set_member("NodeSize", json!("Small"));
        }
        let fed: Vec<(i32, String, String)> = self.outgoing(node).into_iter().filter(|(target, _, output)| output == port && *target != chokepoint).collect();
        self.link(node, port, chokepoint, "In");
        let chokepoint_right = corner[0] + 100.0;
        for (target, input, _) in fed {
            let Some(position) = self.node(target).map(|data| data.position) else {
                continue;
            };
            if position[0] < chokepoint_right {
                let moved = [position[0] + 100.0, position[1]];
                self.move_nodes(&[(target, moved)], false);
            }
            self.link(chokepoint, "Out", target, &input);
        }
        Some(chokepoint)
    }

    /// `ReplaceNode(name)`: "Node X replaced with Y.", a fresh node of
    /// `type_name` in the old one's place keeping what fed and what read
    /// its ports of the same names, its mask, modifiers, export settings,
    /// underlay and preview lock; the old node goes. Returns the new
    /// node's id.
    pub fn replace_node(&mut self, id: i32, type_name: &str) -> Option<i32> {
        let old = self.node(id)?.clone();
        let spec = Catalogue::get().node(type_name)?;
        self.checkpoint(format!("Node {} replaced with {type_name}.", old.name));
        let tab = node_tab(&old);
        let replacement = self.place_node(type_name, old.position, tab)?;
        let old_spec = Catalogue::get().node(&old.type_name);
        let masked = old.port("Mask").is_some() && old_spec.is_some_and(|old_spec| old_spec.is_maskable());
        let reads: Vec<(i32, String, String)> = self.outgoing(id);
        let was_underlay = self.saved_underlay() == id;
        let group = self.group_of(id);
        {
            let node = self.project.terrain_mut().nodes.get_mut(&replacement)?;
            if masked && node.port("Mask").is_none() {
                node.add_port("Mask", "In");
            }
            // Inputs of the same name keep their sources.
            for port in node.ports.iter_mut().filter(|port| port.is_input()) {
                let kept = old.port(&port.name).and_then(|old_port| old_port.record.clone());
                if let Some(mut record) = kept {
                    record.to = replacement;
                    port.record = Some(record);
                }
            }
            node.modifiers = old.modifiers.clone();
            if let Some(mut definition) = old.base.get("SaveDefinition").cloned().filter(|value| !value.is_null()) {
                if let Value::Object(members) = &mut definition {
                    members.insert("Node".to_string(), json!(replacement));
                }
                node.base.insert("SaveDefinition".to_string(), definition);
                for port in node.ports.iter_mut().filter(|port| port.is_output()) {
                    if let Some(old_port) = old.port(&port.name) {
                        port.is_exporting = old_port.is_exporting;
                    }
                }
            }
        }
        let _ = spec;
        // `RecreateConnectionsElements`: the kept inputs pass their render
        // intent on as new connections do, then outputs of the same name
        // feed on.
        let replacement_node = self.node(replacement)?;
        let mut sources: Vec<(i32, String, String)> = Vec::new();
        for port in replacement_node.ports.iter().filter(|port| port.is_input()) {
            if let Some(record) = &port.record {
                sources.push((record.from, record.from_port.clone(), port.name.clone()));
            }
        }
        for (source, output, input) in sources {
            self.convey_render_intent(source, &output, replacement, &input);
        }
        // The replacement takes the preview lock.
        self.move_preview_lock(id, replacement);
        self.take_out_node(id);
        for (target, input, output) in reads {
            let has_output = self.node(replacement).is_some_and(|node| node.port(&output).is_some_and(|port| port.is_output()));
            if has_output {
                self.link(replacement, &output, target, &input);
            }
        }
        if was_underlay {
            self.set_underlay(replacement);
        }
        if let Some(group) = group {
            self.add_to_group(group, &[replacement]);
        }
        Some(replacement)
    }

    /// Combine nodes: a Combine, Mixer or Layers node 1.5 node widths right
    /// of the selection, half way down it, fed by each node's first output
    /// in order (top to bottom, then left to right). Combine as Inputs on
    /// one node feeds each of its outputs into the Combine instead. At most
    /// nine nodes are combined this way. Returns the new node.
    pub fn combine_nodes(&mut self, ids: &[i32], kind: CombineKind, extent: &dyn Fn(i32) -> [f32; 2]) -> Option<i32> {
        let mut nodes: Vec<NodeData> = ids.iter().filter_map(|id| self.node(*id).cloned()).collect();
        if nodes.is_empty() {
            return None;
        }
        let several_only = matches!(kind, CombineKind::Mixer | CombineKind::Layers);
        if several_only && nodes.len() < 2 {
            return None;
        }
        if kind == CombineKind::Inputs && nodes.len() == 1 {
            return self.combine_outputs(&nodes[0], extent);
        }
        match kind {
            CombineKind::Toolbar => nodes.sort_by(|left, right| left.position[1].total_cmp(&right.position[1])),
            _ => nodes.sort_by(|left, right| left.position[1].total_cmp(&right.position[1]).then(left.position[0].total_cmp(&right.position[0]))),
        }
        if kind != CombineKind::Toolbar && nodes.len() > 9 {
            return None;
        }
        let first = &nodes[0];
        let last = &nodes[nodes.len() - 1];
        let right = first.position[0].max(last.position[0]);
        let top = first.position[1].min(last.position[1]);
        let span = (top - first.position[1].max(last.position[1])).abs();
        let corner = [right + extent(first.id)[0] * 1.5, top + span / 2.0];
        let (type_name, label) = match kind {
            CombineKind::Inputs | CombineKind::Toolbar => ("Combine", "Nodes combined as inputs."),
            CombineKind::Mixer => ("Mixer", "Nodes combined as color layers."),
            CombineKind::Layers => ("Layers", "Nodes combined as normal layers."),
        };
        self.checkpoint(label);
        let tab = node_tab(first);
        let combined = self.place_node(type_name, corner, tab)?;
        let sources: Vec<(i32, String)> = nodes.iter().filter_map(|node| grid_outputs(node).first().map(|port| (node.id, port.name.clone()))).collect();
        match kind {
            CombineKind::Inputs | CombineKind::Toolbar => {
                self.grow_combine_inputs(combined, sources.len());
                let inputs = self.combine_inputs(combined);
                for (index, (source, output)) in sources.iter().enumerate() {
                    if let Some(input) = inputs.get(index) {
                        self.link(*source, output, combined, input);
                    }
                }
            }
            CombineKind::Mixer | CombineKind::Layers => {
                for (index, (source, output)) in sources.iter().enumerate() {
                    if index > 1 {
                        self.add_new_input(combined);
                    }
                    let input = if index == 0 { "In".to_string() } else { format!("Layer{index}") };
                    self.link(*source, output, combined, &input);
                }
                if kind == CombineKind::Mixer {
                    self.color_mixer_layers(combined);
                }
            }
        }
        Some(combined)
    }

    /// Combine as Inputs on one node: a Combine 1.5 widths right of it
    /// with an input for each output.
    fn combine_outputs(&mut self, node: &NodeData, extent: &dyn Fn(i32) -> [f32; 2]) -> Option<i32> {
        let outputs: Vec<String> = grid_outputs(node).iter().map(|port| port.name.clone()).collect();
        if outputs.len() < 2 {
            return None;
        }
        self.checkpoint("Nodes combined as inputs.");
        let corner = [node.position[0] + extent(node.id)[0] * 1.5, node.position[1]];
        let combined = self.place_node("Combine", corner, node_tab(node))?;
        self.grow_combine_inputs(combined, outputs.len());
        let inputs = self.combine_inputs(combined);
        for (index, output) in outputs.iter().enumerate() {
            if let Some(input) = inputs.get(index) {
                self.link(node.id, output, combined, input);
            }
        }
        Some(combined)
    }

    fn combine_inputs(&self, node: i32) -> Vec<String> {
        let Some(data) = self.node(node) else {
            return Vec::new();
        };
        grid_inputs(data).iter().map(|port| port.name.clone()).collect()
    }

    /// `Node.AddNewPort` until the node has `count` inputs (the mask
    /// aside).
    fn grow_combine_inputs(&mut self, node: i32, count: usize) {
        while self.combine_inputs(node).len() < count {
            if self.add_new_input(node).is_none() {
                break;
            }
        }
    }

    /// `Node.AddNewPort` as part of an edit already named: the new
    /// input's name, or nothing at the type's limit.
    pub(crate) fn add_new_input(&mut self, node: i32) -> Option<String> {
        let data = self.project.terrain_mut().nodes.get_mut(&node)?;
        let added = nerthus_engine::editing::add_new_port(data);
        if added.is_some() {
            self.mark_stale(node);
            self.dirty = true;
        }
        added
    }

    /// `MultiMixer`: layer N in `brushes[N]`, layer 15 in `brushes[0]`.
    fn color_mixer_layers(&mut self, node: i32) {
        let Some(data) = self.project.terrain_mut().nodes.get_mut(&node) else {
            return;
        };
        for layer in 1..=15usize {
            let brush = LAYER_COLORS[layer % LAYER_COLORS.len()];
            let Some(Value::Object(members)) = data.values.get_mut(&format!("Layer{layer}")) else {
                continue;
            };
            members.insert("Color".to_string(), hex_color(brush));
        }
    }

    /// `MutateNode` (Node › Mutate and the graph toolbar's Mutate): every
    /// seed of each node moved on by `mutation`.
    pub fn mutate_nodes(&mut self, ids: &[i32], mutation: i32) {
        let nodes: Vec<NodeData> = ids.iter().filter_map(|id| self.node(*id).cloned()).collect();
        if nodes.is_empty() {
            return;
        }
        let label = match nodes.as_slice() {
            [single] => format!("{} was mutated", single.name),
            _ => "Node(s) were mutated.".to_string(),
        };
        self.checkpoint(label);
        for node in nodes {
            let changes = nerthus_engine::mutation::mutated_seeds(&node, mutation);
            if let Some(target) = self.project.terrain_mut().nodes.get_mut(&node.id) {
                for (name, value) in changes {
                    target.values.insert(name, value);
                }
            }
            self.mark_stale(node.id);
        }
        self.dirty = true;
    }

    /// `GraphControl.CycleRenderIntent` (T): a colour node's override goes
    /// Mask from Default or Color, else back to Default; any other node's
    /// goes Mask from Default or Heightfield, else Heightfield.
    pub fn cycle_render_intent(&mut self, ids: &[i32], is_color: &dyn Fn(i32) -> bool) {
        let present: Vec<i32> = ids.iter().copied().filter(|id| self.node(*id).is_some()).collect();
        if present.is_empty() {
            return;
        }
        self.checkpoint("Render intent");
        for id in present {
            let current = self.node(id).and_then(|node| node.base.get("RenderIntentOverride")).and_then(Value::as_str).unwrap_or("Default").to_string();
            let next = if is_color(id) {
                if matches!(current.as_str(), "Default" | "Color") { "Mask" } else { "Default" }
            } else if matches!(current.as_str(), "Default" | "Heightfield") {
                "Mask"
            } else {
                "Heightfield"
            };
            if let Some(node) = self.project.terrain_mut().nodes.get_mut(&id) {
                node.set_member("RenderIntentOverride", json!(next));
            }
            self.mark_stale(id);
        }
    }
}

fn hex_color(text: &str) -> Value {
    let channel = |start: usize| u8::from_str_radix(&text[start..start + 2], 16).unwrap_or(0) as f32 / 255.0;
    let red = nerthus_engine::json::f32_value(channel(1));
    let green = nerthus_engine::json::f32_value(channel(3));
    let blue = nerthus_engine::json::f32_value(channel(5));
    json!({"R": red, "G": green, "B": blue})
}

/// The auto layout (F4, `LayoutGraph`).
pub struct Layout<'a> {
    document: &'a mut Document,
    tab: usize,
    extent: &'a dyn Fn(i32) -> [f32; 2],
    /// `verticalHeight`: how far down the layout has reached.
    vertical_height: f32,
    /// Use new Auto Layout method (`Options.NewAutolayout`): one node, or
    /// the whole tab, laid out on the block grid ([`MetaGrid`]).
    new_method: bool,
}

impl<'a> Layout<'a> {
    pub fn new(document: &'a mut Document, tab: usize, extent: &'a dyn Fn(i32) -> [f32; 2]) -> Self {
        Self { document, tab, extent, vertical_height: 0.0, new_method: false }
    }

    /// The layout with Use new Auto Layout method on or off.
    pub fn with_new_method(mut self, on: bool) -> Self {
        self.new_method = on;
        self
    }

    /// `LayoutGraph`: one node selected, what follows it laid out ("Layout
    /// node applied on Name"); more than two, what follows each of them
    /// within the selection; else the whole tab ("Layout node applied on
    /// all nodes"). The new method lays out the one node, or the whole tab
    /// from its leftmost node, on the block grid; more than two still go
    /// the old way.
    pub fn run(mut self, selection: &[i32]) {
        match selection.len() {
            1 => {
                let Some(name) = self.document.node(selection[0]).map(|node| node.name.clone()) else {
                    return;
                };
                self.document.checkpoint(format!("Layout node applied on {name}"));
                if self.new_method {
                    self.meta_grid(selection[0]);
                } else {
                    self.layout_node(selection[0], None);
                }
            }
            count if count > 2 => {
                self.document.checkpoint("Layout node applied on multiple selected nodes");
                for id in selection {
                    self.layout_node(*id, Some(selection));
                }
            }
            _ if self.new_method => {
                // The leftmost node of the tab starts it (`MinBy` X: the
                // first of equals).
                let leftmost = {
                    let nodes = self.document.terrain().nodes.values();
                    let on_tab = nodes.filter(|node| node_tab(node) == self.tab);
                    on_tab.min_by(|left, right| left.position[0].total_cmp(&right.position[0])).map(|node| node.id)
                };
                let Some(start) = leftmost else {
                    return;
                };
                self.document.checkpoint("Layout node applied on all nodes");
                self.meta_grid(start);
            }
            _ => self.layout_all(),
        }
    }

    /// `MetaGrid.StartLayout`: works out the blocks, then moves the nodes.
    fn meta_grid(&mut self, start: i32) {
        let nodes: Vec<&NodeData> = self.document.terrain().nodes.values().filter(|node| node_tab(node) == self.tab).collect();
        let Some(start_node) = nodes.iter().find(|node| node.id == start) else {
            return;
        };
        let mut grid = MetaGrid::default();
        grid.lay_out(start_node, &nodes);
        let corner = [start_node.position[0] - BLOCK_WIDTH, start_node.position[1] - BLOCK_WIDTH];
        let mut moves = Vec::new();
        for (id, (column, row)) in &grid.organized {
            let width = (self.extent)(*id)[0];
            let x = *column as f32 * BLOCK_WIDTH + (BLOCK_WIDTH * 0.5 - width / 2.0);
            let y = *row as f32 * BLOCK_HEIGHT;
            moves.push((*id, [x + corner[0], y + corner[1]]));
        }
        self.document.move_nodes(&moves, false);
    }

    /// The nodes of the tab reading from `node`, in the terrain's order.
    fn children(&self, node: i32) -> Vec<i32> {
        let nodes = self.document.terrain().nodes.values();
        let on_tab = nodes.filter(|candidate| node_tab(candidate) == self.tab);
        let reading = on_tab.filter(|candidate| candidate.ports.iter().any(|port| port.is_input() && port.record.as_ref().is_some_and(|record| record.from == node)));
        reading.map(|candidate| candidate.id).collect()
    }

    fn position(&self, node: i32) -> [f32; 2] {
        self.document.node(node).map(|data| data.position).unwrap_or_default()
    }

    fn place(&mut self, node: i32, position: [f32; 2]) {
        self.document.move_nodes(&[(node, position)], false);
    }

    /// `LayoutNode`: each node reading from `node` placed 70 right of it
    /// (or of its primary source, when that is further right), the first
    /// level with it and the others stepping down; then what follows each
    /// of them, all of it or (`within`) only the chosen.
    fn layout_node(&mut self, node: i32, within: Option<&[i32]>) {
        let children = self.children(node);
        let origin = self.position(node);
        let size = (self.extent)(node);
        let mut count = 1.0f32;
        for child in children {
            let mut x = origin[0] + size[0] + LAYOUT_GAP;
            let primary_source = self.document.node(child).and_then(|data| data.ports.iter().find(|port| port.is_input() && port.is_primary())).and_then(|port| port.record.clone());
            if let Some(record) = primary_source {
                let source_position = self.position(record.from);
                if x <= source_position[0] {
                    x = source_position[0] + (self.extent)(record.from)[0] + LAYOUT_GAP;
                }
            }
            let y = if count == 1.0 { origin[1] } else { origin[1] + size[1] + LAYOUT_GAP * count };
            if self.vertical_height < y {
                self.vertical_height = y;
            }
            // The first input the child feeds: a portal skips the child, a
            // node standing left of it moves right of it.
            let fed = self.document.outgoing(child).into_iter().next();
            if let Some((target, input, _)) = fed {
                if self.document.consumes_portal(target, &input) {
                    continue;
                }
                let target_position = self.position(target);
                let child_position = self.position(child);
                if target_position[0] < child_position[0] {
                    let moved = [child_position[0] + (self.extent)(target)[0] + LAYOUT_GAP, target_position[1]];
                    self.place(target, moved);
                    if self.vertical_height < moved[1] {
                        self.vertical_height = moved[1];
                    }
                }
            }
            self.place(child, [x, y]);
            let descend = within.is_none_or(|chosen| chosen.contains(&child));
            if descend {
                self.layout_node(child, None);
            }
            count += 1.0;
        }
    }

    /// `LayoutAllNodes`: the tab's nodes without connected inputs stacked
    /// in the first one's column, each laid out after.
    fn layout_all(&mut self) {
        let roots: Vec<i32> = {
            let nodes = self.document.terrain().nodes.values();
            let on_tab = nodes.filter(|node| node_tab(node) == self.tab);
            let roots = on_tab.filter(|node| !node.ports.iter().any(|port| port.is_input() && port.record.is_some()));
            roots.map(|node| node.id).collect()
        };
        let Some(first) = roots.first() else {
            return;
        };
        self.document.checkpoint("Layout node applied on all nodes");
        self.vertical_height = 0.0;
        let column = self.position(*first)[0];
        for root in roots {
            if self.vertical_height != 0.0 {
                let height = (self.extent)(root)[1];
                self.place(root, [column, self.vertical_height + height + LAYOUT_GAP]);
            }
            // Each root is laid out twice.
            self.layout_node(root, None);
            self.layout_node(root, None);
        }
    }
}

/// `MetaGrid`'s blocks: 288 wide and 196 high (`BlockWidth`,
/// `BlockHeight`).
const BLOCK_WIDTH: f32 = 288.0;
const BLOCK_HEIGHT: f32 = 196.0;

/// `MetaGrid`: the block each node laid out takes (column, row), in the
/// order they were placed, and the blocks taken.
#[derive(Default)]
struct MetaGrid {
    organized: IndexMap<i32, (i32, i32)>,
    blocks: HashSet<(i32, i32)>,
}

impl MetaGrid {
    /// The start node in block (1, 1), what follows it a column to the
    /// right each step (`LayoutChildren`), then the nodes with no input
    /// connected put left of the first node they feed (`ReorientPrimes`).
    /// `nodes` are the tab's, in the project's order.
    fn lay_out(&mut self, start: &NodeData, nodes: &[&NodeData]) {
        self.assign(start.id, (1, 1));
        self.lay_out_children(start, (1, 1), nodes);
        self.reorient_primes(start, nodes);
    }

    fn assign(&mut self, id: i32, block: (i32, i32)) {
        self.organized.insert(id, block);
        self.blocks.insert(block);
    }

    /// `GetNextSlot`: the first free block down from `block`.
    fn next_free_below(&self, block: (i32, i32)) -> (i32, i32) {
        let mut slot = block;
        while self.blocks.contains(&slot) {
            slot.1 += 1;
        }
        slot
    }

    /// `LayoutChildren`: the nodes whose primary input (`Ports[0]`) reads
    /// one of `node`'s outputs go one under another in the next column
    /// (or the furthest column `node`'s own sources have reached), from
    /// the first free block at or below `node`'s row; then theirs, each in
    /// turn.
    fn lay_out_children(&mut self, node: &NodeData, block: (i32, i32), nodes: &[&NodeData]) {
        let first = self.next_free_below((block.0 + 1, block.1));
        let mut column = first.0;
        for port in node.ports.iter().filter(|port| port.is_input()) {
            let source = port.record.as_ref().and_then(|record| self.organized.get(&record.from));
            if let Some(source_block) = source {
                column = column.max(source_block.0);
            }
        }
        let mut row = 0;
        let mut children: Vec<&NodeData> = Vec::new();
        for output in node.ports.iter().filter(|port| port.is_output()) {
            for reader in nodes.iter().copied() {
                let primary = reader.ports.first().and_then(|port| port.record.as_ref());
                let reads = primary.is_some_and(|record| record.from == node.id && record.from_port == output.name);
                if !reads {
                    continue;
                }
                self.assign(reader.id, (column, first.1 + row));
                row += 1;
                children.push(reader);
            }
        }
        for child in children {
            let child_block = self.organized.get(&child.id).copied().unwrap_or(block);
            self.lay_out_children(child, child_block, nodes);
        }
    }

    /// `ReorientPrimes`: each other node with no input connected goes in
    /// the first free block down from the one left of the highest node its
    /// main output (`Ports[1]`) feeds, when that node was laid out.
    fn reorient_primes(&mut self, start: &NodeData, nodes: &[&NodeData]) {
        for prime in nodes.iter().copied() {
            let connected = prime.ports.iter().any(|port| port.is_input() && port.record.is_some());
            if connected || prime.id == start.id {
                continue;
            }
            let Some(main_output) = prime.ports.get(1) else {
                continue;
            };
            let feeds = |reader: &&NodeData| reader.ports.iter().any(|port| port.is_input() && port.record.as_ref().is_some_and(|record| record.from == prime.id && record.from_port == main_output.name));
            let readers = nodes.iter().copied().filter(feeds);
            let fed = readers.min_by(|left, right| left.position[1].total_cmp(&right.position[1]));
            let Some(fed_block) = fed.and_then(|reader| self.organized.get(&reader.id)).copied() else {
                continue;
            };
            let slot = self.next_free_below((fed_block.0 - 1, fed_block.1));
            self.assign(prime.id, slot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(_: i32) -> [f32; 2] {
        [230.0, 80.0]
    }

    #[test]
    fn inserting_into_a_connection_routes_through_the_node() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let erosion = document.add_node("ErosionPro", [600.0, 0.0]).unwrap();
        document.connect(mountain, "Out", erosion, "In");
        let blur = document.insert_node_in_connection(erosion, "In", "Blur", [300.0, 0.0], false).unwrap();
        assert_eq!(document.node(blur).unwrap().port("In").unwrap().record.as_ref().unwrap().from, mountain);
        assert_eq!(document.node(erosion).unwrap().port("In").unwrap().record.as_ref().unwrap().from, blur);
        assert_eq!(document.undo_labels()[0], "Node Blur inserted in connection.");
    }

    #[test]
    fn replacing_keeps_connections_by_port_name() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let erosion = document.add_node("ErosionPro", [300.0, 0.0]).unwrap();
        let blur = document.add_node("Blur", [600.0, 0.0]).unwrap();
        document.connect(mountain, "Out", erosion, "In");
        document.connect(erosion, "Out", blur, "In");
        let replaced = document.replace_node(erosion, "Terraces").unwrap();
        assert!(document.node(erosion).is_none());
        let node = document.node(replaced).unwrap();
        assert_eq!(node.position, [300.0, 0.0]);
        assert_eq!(node.port("In").unwrap().record.as_ref().unwrap().from, mountain);
        assert_eq!(document.node(blur).unwrap().port("In").unwrap().record.as_ref().unwrap().from, replaced);
    }

    #[test]
    fn combining_feeds_a_combine_in_order() {
        let mut document = Document::new();
        let first = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let second = document.add_node("Perlin", [0.0, 200.0]).unwrap();
        let third = document.add_node("Ridge", [0.0, 400.0]).unwrap();
        let combine = document.combine_nodes(&[third, first, second], CombineKind::Inputs, &size).unwrap();
        let node = document.node(combine).unwrap();
        assert_eq!(node.position, [345.0, 200.0]);
        let sources: Vec<i32> = grid_inputs(node).iter().filter_map(|port| port.record.as_ref().map(|record| record.from)).collect();
        assert_eq!(sources, vec![first, second, third]);
    }

    #[test]
    fn mixers_colour_their_layers() {
        let mut document = Document::new();
        let first = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let second = document.add_node("Perlin", [0.0, 200.0]).unwrap();
        let mixer = document.combine_nodes(&[first, second], CombineKind::Mixer, &size).unwrap();
        let node = document.node(mixer).unwrap();
        assert_eq!(node.port("In").unwrap().record.as_ref().unwrap().from, first);
        assert_eq!(node.port("Layer1").unwrap().record.as_ref().unwrap().from, second);
        let red = node.values["Layer1"]["Color"]["R"].as_f64().unwrap();
        assert!((red - 0xf7 as f64 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn a_replacement_takes_and_passes_on_render_intents() {
        let mut document = Document::new();
        let erosion = document.add_node("ErosionPro", [0.0, 0.0]).unwrap();
        let blur = document.add_node("Blur", [300.0, 0.0]).unwrap();
        let after = document.add_node("Blur", [600.0, 0.0]).unwrap();
        document.connect(erosion, "Deposits", blur, "In");
        document.connect(blur, "Out", after, "In");
        document.set_member(after, "RenderIntentOverride", json!("Heightfield"), "Render intent", true);
        let replaced = document.replace_node(blur, "Combine").unwrap();
        let intent = |id: i32| document.node(id).unwrap().base.get("RenderIntentOverride").cloned();
        // Fed a mask, the new node is one, and passes that on.
        assert_eq!(intent(replaced), Some(json!("Mask")));
        assert_eq!(intent(after), Some(json!("Mask")));
    }

    #[test]
    fn render_intents_cycle_in_order() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let intent = |document: &Document| document.node(mountain).unwrap().base.get("RenderIntentOverride").and_then(Value::as_str).unwrap_or("Default").to_string();
        let never_color = |_: i32| false;
        document.cycle_render_intent(&[mountain], &never_color);
        assert_eq!(intent(&document), "Mask");
        document.cycle_render_intent(&[mountain], &never_color);
        assert_eq!(intent(&document), "Heightfield");
        document.cycle_render_intent(&[mountain], &never_color);
        assert_eq!(intent(&document), "Mask");
    }

    #[test]
    fn layout_places_children_to_the_right() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let erosion = document.add_node("ErosionPro", [50.0, 500.0]).unwrap();
        let blur = document.add_node("Blur", [10.0, 900.0]).unwrap();
        document.connect(mountain, "Out", erosion, "In");
        document.connect(mountain, "Out", blur, "In");
        Layout::new(&mut document, 0, &size).run(&[mountain]);
        assert_eq!(document.node(erosion).unwrap().position, [300.0, 0.0]);
        assert_eq!(document.node(blur).unwrap().position, [300.0, 220.0]);
    }

    #[test]
    fn the_new_layout_puts_nodes_on_blocks() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [1000.0, 1000.0]).unwrap();
        let perlin = document.add_node("Perlin", [900.0, 1400.0]).unwrap();
        let erosion = document.add_node("ErosionPro", [50.0, 500.0]).unwrap();
        let blur = document.add_node("Blur", [10.0, 900.0]).unwrap();
        let combine = document.add_node("Combine", [70.0, 90.0]).unwrap();
        document.connect(mountain, "Out", erosion, "In");
        document.connect(mountain, "Out", blur, "In");
        document.connect(erosion, "Out", combine, "In");
        document.connect(perlin, "Out", combine, "Input2");
        Layout::new(&mut document, 0, &size).with_new_method(true).run(&[mountain]);
        // The corner is a block's width up and left of where the start
        // node stood; each node is centred across its block (230 wide).
        let corner = [1000.0 - 288.0, 1000.0 - 288.0];
        let at = |column: f32, row: f32| [corner[0] + column * 288.0 + (144.0 - 115.0), corner[1] + row * 196.0];
        assert_eq!(document.node(mountain).unwrap().position, at(1.0, 1.0));
        assert_eq!(document.node(erosion).unwrap().position, at(2.0, 1.0));
        assert_eq!(document.node(blur).unwrap().position, at(2.0, 2.0));
        assert_eq!(document.node(combine).unwrap().position, at(3.0, 1.0));
        // Perlin feeds the combine but not its primary input: it goes left
        // of the combine, in the first free block down from there.
        assert_eq!(document.node(perlin).unwrap().position, at(2.0, 3.0));
        assert_eq!(document.undo_labels()[0], "Layout node applied on Mountain");
    }
}
