//! The node graph (`GraphControl`): dark nodes
//! with the toolbox category's colour on the title, icon, ports and
//! outgoing connections, borders that show the build state, "circuit"
//! connections, groups and notes behind the nodes, and the mouse
//! handling (drag to move or connect, marquee selection, Ctrl Shift drag
//! to push elements aside, wheel zoom, middle or right drag to pan, a
//! connection dropped into the node search or onto another output, files
//! and toolbox entries dropped onto the graph).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use egui::epaint::{PathShape, PathStroke};
use egui::{Color32, CursorIcon, Key, PointerButton, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, Vec2};
use nerthus_engine::{Catalogue, NodeData, NodeState, PortData};
use serde_json::Value;

use crate::document::{grid_inputs, grid_outputs, node_tab, Document, ExportSettings, GroupData};
use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::create_connection::CreateConnection;
use crate::ui::graph_commands::{GraphCommand, NodeSource};
use crate::ui::graph_groups::{self, GroupDrag, GroupState};
use crate::ui::graph_popups::GraphPopup;
use crate::ui::graph_search::{SearchMode, SearchPopup};
use crate::ui::widgets::{checked_icon_item, menu_item, menu_item_enabled, menu_separator, menu_style, submenu};

/// `NodeElement` template sizes (Standard, Compact, Small).
const STANDARD_WIDTH: f32 = 230.0;
const STANDARD_MIN_HEIGHT: f32 = 80.0;
const COMPACT_WIDTH: f32 = 170.0;
const COMPACT_HEIGHT: f32 = 64.0;
const SMALL_SIZE: f32 = 64.0;
/// Port rows (`RowDefinition` 16) and the port grids' margins.
const PORT_ROW: f32 = 16.0;
const PORTS_TOP: f32 = 40.0;
const PORTS_BOTTOM: f32 = 20.0;
const PORTS_SIDE: f32 = 8.0;
/// `PortElement` and its dot (`ControlBorder`).
const PORT_WIDTH: f32 = 20.0;
const PORT_HEIGHT: f32 = 12.0;
const PORT_DOT: f32 = 8.0;
/// `ConnectionElement.PortGap`.
const PORT_GAP: f32 = 10.0;
/// The circuit's horizontal run out of and into ports.
const CIRCUIT_RUN: f32 = 24.0;
/// Below this zoom nodes draw simplified (`IsZoomedOut`).
pub(crate) const SIMPLIFIED_ZOOM: f32 = 0.4;
const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 2.0;
/// How far from a port a dragged connection still snaps to it.
const SNAP_DISTANCE: f32 = 18.0;
/// `SnapToOtherShapes`: how near (canvas units) an edge or a centre snaps.
const SNAP_ITEMS_DISTANCE: f32 = 36.0;
/// `Line-SnapLine-Stroke`.
const SNAP_LINE: Color32 = Color32::from_rgb(0x60, 0x53, 0x8f);
/// `CreateCollapsedPortElement`: the port standing in for a compact or
/// small node's ports past the first when it has more than two.
pub const COLLAPSED_PORT: &str = "CollapsedPort";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZoomRequest {
    Extents,
    Reset,
    Selection,
    /// `PanToElementLocation`: a node centred, the zoom kept.
    Center(i32),
    /// The same for a note.
    CenterNote(i32),
    /// The same for a group.
    CenterGroup(i32),
}

/// A graph tab's view: the canvas point at the top-left corner
/// (`ViewportLocation`) and the scale (`ZoomFactor`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphView {
    pub location: Vec2,
    pub zoom: f32,
}

/// `GraphTab`'s defaults.
impl Default for GraphView {
    fn default() -> Self {
        Self {
            location: Vec2::new(26000.0, 26000.0),
            zoom: 1.0,
        }
    }
}

impl GraphView {
    pub(crate) fn to_screen(self, origin: Pos2, point: Pos2) -> Pos2 {
        origin + (point.to_vec2() - self.location) * self.zoom
    }

    pub(crate) fn to_canvas(self, origin: Pos2, point: Pos2) -> Pos2 {
        let offset = (point - origin) / self.zoom;
        (self.location + offset).to_pos2()
    }

    pub(crate) fn rect_to_screen(self, origin: Pos2, rect: Rect) -> Rect {
        Rect::from_min_max(self.to_screen(origin, rect.min), self.to_screen(origin, rect.max))
    }
}

/// Where the editor's areas were this frame, for the Lazy Menu to move
/// the pointer into (`SetCursorPosition`).
#[derive(Clone, Copy, Debug)]
pub struct GraphAreas {
    /// The docked 2D view, while it shows in the shell.
    pub viewport_2d: Option<Rect>,
    pub graph: Rect,
    pub properties: Rect,
    pub data_view: Option<Rect>,
}

impl Default for GraphAreas {
    fn default() -> Self {
        Self { viewport_2d: None, graph: Rect::NOTHING, properties: Rect::NOTHING, data_view: None }
    }
}

/// What a Lazy Menu entry moves the pointer to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LazyTarget {
    Viewport2D,
    Graph,
    Properties,
    Modifiers,
    DataView,
}

/// An interaction in progress.
#[derive(Clone, Debug)]
enum Drag {
    /// Moving the selection: where each node, the note and each group
    /// started and where the pointer started. `primary` is the node
    /// grabbed, which Snap to Items aligns.
    Elements {
        pointer_start: Pos2,
        primary: i32,
        nodes: Vec<(i32, [f32; 2])>,
        note: Option<(i32, [f32; 2])>,
        groups: Vec<(i32, [f32; 2])>,
    },
    /// Selecting with a rectangle (screen coordinates).
    Marquee { start: Pos2, additive: bool },
    /// A connection pulled out of an output, or backwards out of an input.
    /// A `sticky` one (a port double-clicked, `IsClickClick`) ends at the
    /// next click instead of the release. One pulled out of a compact
    /// node's collapsed ports (`collapsed`) starts at the first of them.
    FromOutput { node: i32, port: String, sticky: bool, collapsed: bool },
    FromInput { node: i32, port: String, sticky: bool, collapsed: bool },
    /// Ctrl Shift on empty graph (`DragArrange`): where the push began
    /// (canvas units), and what lay to the right of it then.
    Push { press: Pos2, elements: PushedElements },
}

/// What a push moves, and where each started (canvas units). A group's
/// nodes and notes move with the group rather than on their own.
#[derive(Clone, Debug, Default, PartialEq)]
struct PushedElements {
    nodes: Vec<(i32, [f32; 2])>,
    notes: Vec<(i32, [f32; 2])>,
    groups: Vec<(i32, [f32; 2])>,
}

impl PushedElements {
    fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.notes.is_empty() && self.groups.is_empty()
    }
}

/// The graph's interaction state between frames.
pub struct GraphState {
    /// The shown tab's `Index`.
    pub tab: usize,
    /// Each tab's view, by `Index`.
    pub views: HashMap<usize, GraphView>,
    drag: Option<Drag>,
    /// Clicked connections (by the input each feeds): Delete, Insert node,
    /// Convert to Portal.
    pub selected_connections: Vec<(i32, String)>,
    /// Clicked groups.
    pub selected_groups: Vec<i32>,
    pub show_grid: bool,
    pub snap_to_grid: bool,
    /// `GridUnit` (`Options.GridSize`): what nodes snap to.
    pub grid_unit: f32,
    pub renaming: Option<i32>,
    rename_text: String,
    pub zoom_request: Option<ZoomRequest>,
    /// Canvas point of the last right-click, where added nodes go.
    pub context_point: Option<Pos2>,
    /// The canvas widget's screen rectangle last frame.
    pub rect: Rect,
    /// Whether the views came from the document yet.
    views_loaded: bool,
    pub selected_note: Option<i32>,
    pub note_drag: Option<crate::ui::graph_notes::NoteDrag>,
    /// The note the Note Editor shows and its text so far.
    pub editing_note: Option<(i32, String)>,
    /// The tab being renamed and its name so far.
    pub renaming_tab: Option<(usize, String)>,
    /// The tab whose deletion awaits confirmation.
    pub confirm_delete_tab: Option<usize>,
    /// Nodes the Data View's filter leaves out, drawn at half strength.
    pub dimmed: HashSet<i32>,
    /// The bookmark whose menu entry reads "Remove?" (right-clicked).
    pub bookmark_removing: Option<i32>,
    /// A menu or the search open over the graph.
    pub popup: Option<GraphPopup>,
    /// The Create Connection window, while open.
    pub create_connection: Option<CreateConnection>,
    /// Where the pointer is, while it is over the graph (screen).
    pub pointer: Option<Pos2>,
    /// Shift held this frame (Ungroup then takes the group's nodes too).
    pub shift_held: bool,
    /// `Options.SnapToPorts`: a dragged connection takes a port near the
    /// pointer, not only one under it.
    snap_to_ports: bool,
    /// The group being renamed and its name so far.
    pub renaming_group: Option<(i32, String)>,
    /// A group being moved or resized.
    pub group_drag: Option<GroupDrag>,
    /// What each group shows while an element is dragged over or out of
    /// it (`GroupState`).
    pub group_states: HashMap<i32, GroupState>,
    /// Snap to Items' lines (canvas units) while a node is dragged.
    snap_lines: Vec<[Pos2; 2]>,
    /// A toolbox entry being dragged onto the graph (its node type), and
    /// the toolbox's own state.
    pub toolbox_drag: Option<String>,
    pub toolbox: crate::ui::toolbox::ToolboxState,
    /// Where the toolbox was last frame: the wheel over it is the
    /// toolbox's, not the graph's.
    pub toolbox_rect: Rect,
    /// The editor's areas this frame, and where the Lazy Menu asked the
    /// pointer to go.
    pub areas: GraphAreas,
    pub lazy_cursor: Option<LazyTarget>,
    /// Lazy Menu › Toggle Skybox, for the viewport's settings.
    pub toggle_skybox: bool,
    /// Nodes shining (`NodeElement.Shine`) and when each started (egui's
    /// clock).
    pub shines: HashMap<i32, f64>,
    /// The document's view generation the views were last taken at.
    view_generation: u64,
}

impl Default for GraphState {
    fn default() -> Self {
        Self {
            tab: 0,
            views: HashMap::new(),
            drag: None,
            selected_connections: Vec::new(),
            selected_groups: Vec::new(),
            show_grid: false,
            snap_to_grid: false,
            grid_unit: 25.0,
            renaming: None,
            rename_text: String::new(),
            zoom_request: None,
            context_point: None,
            rect: Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)),
            views_loaded: false,
            selected_note: None,
            note_drag: None,
            editing_note: None,
            renaming_tab: None,
            confirm_delete_tab: None,
            dimmed: HashSet::new(),
            bookmark_removing: None,
            popup: None,
            create_connection: None,
            pointer: None,
            shift_held: false,
            snap_to_ports: true,
            renaming_group: None,
            group_drag: None,
            group_states: HashMap::new(),
            snap_lines: Vec::new(),
            toolbox_drag: None,
            toolbox: crate::ui::toolbox::ToolboxState::default(),
            toolbox_rect: Rect::NOTHING,
            areas: GraphAreas::default(),
            lazy_cursor: None,
            toggle_skybox: false,
            shines: HashMap::new(),
            view_generation: 0,
        }
    }
}

impl GraphState {
    pub fn view(&self) -> GraphView {
        self.views.get(&self.tab).copied().unwrap_or_default()
    }

    fn view_mut(&mut self) -> &mut GraphView {
        self.views.entry(self.tab).or_default()
    }

    /// Forgets views so the next frame reads them from the document.
    pub fn reset(&mut self) {
        self.views_loaded = false;
        self.tab = 0;
        self.drag = None;
        self.selected_connections.clear();
        self.selected_groups.clear();
        self.renaming = None;
        self.selected_note = None;
        self.note_drag = None;
        self.editing_note = None;
        self.popup = None;
        self.create_connection = None;
        self.renaming_group = None;
        self.group_drag = None;
        self.group_states.clear();
        self.snap_lines.clear();
    }

    /// The connections, groups and note the graph has chosen go (the
    /// nodes are the editor's selection).
    pub fn clear_element_selection(&mut self) {
        self.selected_connections.clear();
        self.selected_groups.clear();
        self.selected_note = None;
    }

    /// Where a node added without a position goes: the last right-click,
    /// else the middle of the view.
    pub fn drop_position(&self) -> [f32; 2] {
        if let Some(point) = self.context_point {
            return [point.x, point.y];
        }
        let view = self.view();
        let center = view.location + self.rect.size() / (2.0 * view.zoom);
        [center.x - STANDARD_WIDTH / 2.0, center.y - STANDARD_MIN_HEIGHT / 2.0]
    }

    /// An undo or redo that reopened the project brings its tabs' views
    /// back with it (`GraphTabSelectionChanged(IsLoading: true)` after the
    /// terrain is read again).
    fn follow_restored_views(&mut self, editor: &Editor) {
        let generation = editor.document.view_generation();
        if self.view_generation == generation {
            return;
        }
        self.view_generation = generation;
        for tab in editor.document.graph_tabs() {
            let Some(location) = tab.location else {
                continue;
            };
            let view = GraphView {
                location: Vec2::new(location[0], location[1]),
                zoom: tab.zoom.clamp(MIN_ZOOM, MAX_ZOOM),
            };
            self.views.insert(tab.index, view);
        }
    }

    fn load_views(&mut self, editor: &Editor) {
        if self.views_loaded {
            return;
        }
        self.views_loaded = true;
        let tabs = editor.document.graph_tabs();
        self.views.clear();
        for tab in &tabs {
            let location = tab.location.unwrap_or([26000.0, 26000.0]);
            let view = GraphView {
                location: Vec2::new(location[0], location[1]),
                zoom: tab.zoom.clamp(MIN_ZOOM, MAX_ZOOM),
            };
            self.views.insert(tab.index, view);
        }
        self.tab = tabs.first().map_or(0, |tab| tab.index);
        if tabs.first().is_some_and(|tab| tab.location.is_none()) {
            self.zoom_request = Some(ZoomRequest::Extents);
        }
    }
}

/// A port as laid out on its node (canvas units).
#[derive(Clone, Debug)]
pub(crate) struct PortLayout {
    pub name: String,
    label: String,
    pub is_input: bool,
    connected: bool,
    /// Where connections attach (the dot's outer edge).
    pub anchor: Pos2,
    /// The `PortElement` (20 by 12).
    element: Rect,
    dot: Rect,
    hit: Rect,
    portal: Option<Portal>,
    /// `PortElement.FriendlyName`: what a portal pill reads.
    portal_name: String,
    /// A port of a compact node's collapsed side past the first: placed in
    /// the second row, where its connections attach, but not shown.
    hidden: bool,
    /// The collapsed side's stand-in (`PortElementType.Collapsed`).
    collapsed: bool,
}

/// `PortalState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Portal {
    Publishing,
    Consuming,
}

fn portal_state(port: &PortData) -> Option<Portal> {
    match port.extra.get("PortalState").and_then(Value::as_str) {
        Some("Publishing") => Some(Portal::Publishing),
        Some("Consuming") => Some(Portal::Consuming),
        _ => None,
    }
}

/// A node as laid out on the canvas.
#[derive(Clone, Debug)]
pub(crate) struct NodeLayout {
    pub id: i32,
    pub rect: Rect,
    size: NodeSize,
    color: Color32,
    icon: String,
    inputs: Vec<PortLayout>,
    outputs: Vec<PortLayout>,
    mask: Option<PortLayout>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeSize {
    Standard,
    Compact,
    Small,
}

/// The node's `NodeSize`; a node without one draws at the user's
/// `DefaultNodeSize` (`DetermineScale`).
fn node_size(node: &NodeData, default_size: &str) -> NodeSize {
    let saved = node.base.get("NodeSize").and_then(Value::as_str).unwrap_or("Default");
    let size = if saved == "Default" { default_size } else { saved };
    match size {
        "Compact" => NodeSize::Compact,
        "Small" => NodeSize::Small,
        _ => NodeSize::Standard,
    }
}

/// The colour of a node's toolbox category.
pub fn node_color(node: &NodeData) -> Color32 {
    let spec = Catalogue::get().node(&node.type_name);
    let category = spec.and_then(|spec| spec.toolbox.as_deref()).unwrap_or("Utility");
    theme::toolbox_color(category)
}

/// `IconAssigner`: `x-<name>` when there is one, else the family's icon
/// (`Plugin`: `family-<family>`), else `square-rounded`.
pub fn node_icon(type_name: &str) -> String {
    let spec = Catalogue::get().node(type_name);
    let name = spec.map(|spec| spec.name.to_lowercase()).unwrap_or_else(|| type_name.to_lowercase());
    let icon = format!("x-{name}");
    if icons::exists(&icon) {
        return icon;
    }
    let family = spec.and_then(|spec| spec.family.as_deref()).map(str::to_lowercase);
    if let Some(family) = family {
        let family_icon = format!("family-{family}");
        if icons::exists(&family_icon) {
            return family_icon;
        }
    }
    "square-rounded".to_string()
}

fn port_label(port: &PortData) -> String {
    let display = port.extra.get("DisplayName").and_then(Value::as_str);
    display.unwrap_or(&port.name).to_string()
}

/// The port rows a side takes: a compact or small node shows two at most
/// (the first port and the collapsed rest).
fn side_rows(count: usize, compact: bool) -> usize {
    if compact { count.min(2) } else { count }
}

pub(crate) fn layout_node(node: &NodeData, default_size: &str) -> NodeLayout {
    let size = node_size(node, default_size);
    let compact = size != NodeSize::Standard;
    let inputs = grid_inputs(node);
    let outputs = grid_outputs(node);
    let mask_port = node.ports.iter().find(|port| port.is_input() && port.name == "Mask");
    let input_rows = side_rows(inputs.len(), compact);
    let output_rows = side_rows(outputs.len(), compact);
    let rows = input_rows.max(output_rows) as f32;
    let (width, height, ports_top) = match size {
        NodeSize::Standard => (STANDARD_WIDTH, (PORTS_TOP + rows * PORT_ROW + PORTS_BOTTOM).max(STANDARD_MIN_HEIGHT), PORTS_TOP),
        NodeSize::Compact => (COMPACT_WIDTH, COMPACT_HEIGHT, (COMPACT_HEIGHT - rows * PORT_ROW) / 2.0),
        NodeSize::Small => (SMALL_SIZE, SMALL_SIZE, (SMALL_SIZE - rows * PORT_ROW) / 2.0),
    };
    let origin = Pos2::new(node.position[0], node.position[1]);
    let rect = Rect::from_min_size(origin, Vec2::new(width, height));

    let place = |name: &str, row: usize, is_input: bool| {
        let row_top = rect.top() + ports_top + row as f32 * PORT_ROW;
        let element_top = row_top + (PORT_ROW - PORT_HEIGHT) / 2.0;
        let element_left = if is_input { rect.left() + PORTS_SIDE } else { rect.right() - PORTS_SIDE - PORT_WIDTH };
        let element = Rect::from_min_size(Pos2::new(element_left, element_top), Vec2::new(PORT_WIDTH, PORT_HEIGHT));
        let dot_left = if is_input { element.left() } else { element.right() - PORT_DOT };
        let dot = Rect::from_min_size(Pos2::new(dot_left, element.center().y - PORT_DOT / 2.0), Vec2::splat(PORT_DOT));
        // RefreshPoints: the element's centre, `PortGap` outwards.
        let anchor_x = if is_input { element.center().x - PORT_GAP } else { element.center().x + PORT_GAP };
        let anchor = Pos2::new(anchor_x, element.center().y);
        let hit = element.expand2(Vec2::new(4.0, 2.0));
        PortLayout {
            name: name.to_string(),
            label: String::new(),
            is_input,
            connected: false,
            anchor,
            element,
            dot,
            hit,
            portal: None,
            portal_name: String::new(),
            hidden: false,
            collapsed: false,
        }
    };
    // `ConfigurePorts`: a compact or small node puts its first port in the
    // first row and the rest in the second; past two, the second row shows
    // the collapsed stand-in only.
    let side = |ports: &[&PortData], is_input: bool| {
        let collapsing = compact && ports.len() > 2;
        let mut layouts = Vec::new();
        for (index, port) in ports.iter().enumerate() {
            let row = if compact { index.min(1) } else { index };
            let mut layout = place(&port.name, row, is_input);
            layout.label = port_label(port);
            layout.connected = port.record.is_some();
            layout.portal = portal_state(port);
            layout.hidden = collapsing && index > 0;
            layouts.push(layout);
        }
        if collapsing {
            let mut stand_in = place(COLLAPSED_PORT, 1, is_input);
            stand_in.collapsed = true;
            layouts.push(stand_in);
        }
        layouts
    };
    let input_layouts = side(&inputs, true);
    let output_layouts = side(&outputs, false);
    let mask = mask_port.map(|port| {
        let element = Rect::from_min_size(Pos2::new(rect.left() + 6.0, rect.bottom() + 6.0), Vec2::new(PORT_WIDTH, PORT_HEIGHT));
        let dot = Rect::from_min_size(Pos2::new(element.left(), element.center().y - PORT_DOT / 2.0), Vec2::splat(PORT_DOT));
        let anchor = Pos2::new(element.center().x - PORT_GAP, element.center().y);
        PortLayout {
            name: port.name.clone(),
            label: port_label(port),
            is_input: true,
            connected: port.record.is_some(),
            anchor,
            element,
            dot,
            hit: element.expand(3.0),
            portal: portal_state(port),
            portal_name: String::new(),
            hidden: false,
            collapsed: false,
        }
    });

    NodeLayout {
        id: node.id,
        rect,
        size,
        color: node_color(node),
        icon: node_icon(&node.type_name),
        inputs: input_layouts,
        outputs: output_layouts,
        mask,
    }
}

/// A node's width and height on the canvas (`ActualWidth`,
/// `ActualHeight`), which placements and the layout read.
pub fn node_extent(node: &NodeData, default_size: &str) -> [f32; 2] {
    let layout = layout_node(node, default_size);
    [layout.rect.width(), layout.rect.height()]
}

impl NodeLayout {
    /// A port by name, hidden ones included (connections attach to them).
    pub(crate) fn port(&self, name: &str, is_input: bool) -> Option<&PortLayout> {
        if is_input {
            if let Some(mask) = self.mask.as_ref().filter(|mask| mask.name == name) {
                return Some(mask);
            }
            return self.inputs.iter().find(|port| port.name == name);
        }
        self.outputs.iter().find(|port| port.name == name)
    }

    /// The ports shown.
    fn ports(&self) -> impl Iterator<Item = &PortLayout> {
        let all = self.inputs.iter().chain(self.outputs.iter()).chain(self.mask.iter());
        all.filter(|port| !port.hidden)
    }
}

/// `PortElement.SetFriendlyName`: a publishing portal reads its node's
/// name, a consuming one its source's (`Node | Port` when the source port
/// is not the primary output).
fn name_portals(layouts: &mut [NodeLayout], editor: &Editor) {
    let nodes = &editor.document.terrain().nodes;
    for layout in layouts.iter_mut() {
        let Some(node) = nodes.get(&layout.id) else { continue };
        let ports = layout.inputs.iter_mut().chain(layout.outputs.iter_mut()).chain(layout.mask.iter_mut());
        for port in ports {
            match port.portal {
                Some(Portal::Publishing) => port.portal_name = node.name.clone(),
                Some(Portal::Consuming) => {
                    let record = node.port(&port.name).and_then(|data| data.record.as_ref());
                    let source = record.and_then(|record| nodes.get(&record.from).map(|source| (source, record.from_port.as_str())));
                    let Some((source, source_port)) = source else { continue };
                    let primary_out = source.port(source_port).is_some_and(|data| data.is_primary() && data.is_output());
                    port.portal_name = if primary_out { source.name.clone() } else { format!("{} | {source_port}", source.name) };
                }
                None => {}
            }
        }
    }
}

/// A connection as drawn: from an output to the input it feeds.
struct ConnectionLayout {
    to: i32,
    to_port: String,
    points: Vec<Pos2>,
    color: Color32,
}

/// The circuit path (`ConnectionType.Circuit`): out along the port, across,
/// and in; close nodes get the short "proximity" kink.
fn circuit_points(start: Pos2, end: Pos2, target_standard: bool) -> Vec<Pos2> {
    let close = target_standard && end.x < start.x + 60.0 && (end.y - start.y).abs() < 60.0;
    if close {
        return vec![start, Pos2::new(start.x + 8.0, start.y), Pos2::new(end.x - 8.0, end.y)];
    }
    vec![start, Pos2::new(start.x + CIRCUIT_RUN, start.y), Pos2::new(end.x - CIRCUIT_RUN, end.y), end]
}

fn distance_to_segment(point: Pos2, start: Pos2, end: Pos2) -> f32 {
    let segment = end - start;
    let length_squared = segment.length_sq();
    if length_squared <= f32::EPSILON {
        return point.distance(start);
    }
    let along = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
    let closest = start + segment * along;
    point.distance(closest)
}

/// `LineIntersectsRect`: whether the straight line between a connection's
/// ends crosses or lies in the rectangle.
fn segment_meets_rect(start: Pos2, end: Pos2, rect: Rect) -> bool {
    if rect.contains(start) || rect.contains(end) {
        return true;
    }
    let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
    for index in 0..4 {
        let edge_start = corners[index];
        let edge_end = corners[(index + 1) % 4];
        if segments_cross(start, end, edge_start, edge_end) {
            return true;
        }
    }
    false
}

fn segments_cross(first_start: Pos2, first_end: Pos2, second_start: Pos2, second_end: Pos2) -> bool {
    let side = |origin: Pos2, toward: Pos2, point: Pos2| (toward - origin).x * (point - origin).y - (toward - origin).y * (point - origin).x;
    let first_a = side(first_start, first_end, second_start);
    let first_b = side(first_start, first_end, second_end);
    let second_a = side(second_start, second_end, first_start);
    let second_b = side(second_start, second_end, first_end);
    first_a * first_b < 0.0 && second_a * second_b < 0.0
}

/// Points around a rounded rectangle (for dashed borders).
fn rounded_outline(rect: Rect, radius: f32) -> Vec<Pos2> {
    let radius = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let corners = [
        (Pos2::new(rect.right() - radius, rect.top() + radius), -90.0f32),
        (Pos2::new(rect.right() - radius, rect.bottom() - radius), 0.0),
        (Pos2::new(rect.left() + radius, rect.bottom() - radius), 90.0),
        (Pos2::new(rect.left() + radius, rect.top() + radius), 180.0),
    ];
    let mut points = vec![Pos2::new(rect.left() + radius, rect.top())];
    for (center, start_angle) in corners {
        for step in 0..=4 {
            let angle = (start_angle + step as f32 * 22.5).to_radians();
            points.push(center + Vec2::new(angle.cos(), angle.sin()) * radius);
        }
    }
    points.push(Pos2::new(rect.left() + radius, rect.top()));
    points
}

/// A border as WPF strokes it: solid, or dashed with the dash array in
/// stroke widths (`1 2`, `2 4`, `4 6`).
fn border(painter: &egui::Painter, rect: Rect, radius: f32, stroke: Stroke, dashes: Option<(f32, f32)>) {
    match dashes {
        None => {
            painter.rect_stroke(rect, radius, stroke, StrokeKind::Inside);
        }
        Some((dash, gap)) => {
            let inset = rect.shrink(stroke.width / 2.0);
            let outline = rounded_outline(inset, radius);
            let shapes = Shape::dashed_line(&outline, stroke, dash * stroke.width, gap * stroke.width);
            painter.extend(shapes);
        }
    }
}

/// The groups of the shown tab and, for each node and note a collapsed
/// group hides, that group's box.
struct GroupFrame {
    groups: Vec<GroupData>,
    hidden_in: HashMap<i32, Rect>,
}

fn group_frame(editor: &Editor, tab: usize) -> GroupFrame {
    let all = editor.document.groups();
    let groups: Vec<GroupData> = all.into_iter().filter(|group| group.graph_index == tab).collect();
    let mut hidden_in = HashMap::new();
    for group in groups.iter().filter(|group| group.collapsed) {
        for child in &group.children {
            hidden_in.insert(*child, group.bounds());
        }
    }
    GroupFrame { groups, hidden_in }
}

pub fn show(ui: &mut Ui, rect: Rect, editor: &mut Editor, state: &mut GraphState, commands: &mut Vec<Command>) {
    // Everything the graph draws or hit-tests keeps to its rectangle:
    // icons and rename boxes drawn through `ui` clip there as the painter
    // does, and a node, group or note panned past the edge takes no
    // clicks beyond it.
    let outer_clip = ui.clip_rect();
    ui.shrink_clip_rect(rect);
    state.load_views(editor);
    state.follow_restored_views(editor);
    state.rect = rect;
    take_shines(ui, editor, state);
    let hover = ui.input(|input| input.pointer.hover_pos());
    state.pointer = hover.filter(|point| rect.contains(*point));
    state.shift_held = ui.input(|input| input.modifiers.shift);
    state.snap_to_ports = editor.user.options.snap_to_ports();
    let origin = rect.min;
    let painter = ui.painter_at(rect);

    // Lay out this tab's nodes, less those collapsed groups hide.
    let tab = state.tab;
    let frame = group_frame(editor, tab);
    let default_size = editor.user.options.default_node_size().to_string();
    let tab_nodes = editor.document.terrain().nodes.values().filter(|node| node_tab(node) == tab);
    let shown_nodes = tab_nodes.filter(|node| !frame.hidden_in.contains_key(&node.id));
    let mut layouts: Vec<NodeLayout> = shown_nodes.map(|node| layout_node(node, &default_size)).collect();
    name_portals(&mut layouts, editor);
    let by_id: HashMap<i32, usize> = layouts.iter().enumerate().map(|(index, layout)| (layout.id, index)).collect();

    apply_zoom_request(state, &layouts, &frame.groups, editor, rect);

    // The background takes clicks, drags and the context menu.
    let background_id = ui.id().with("graph-background");
    let background = ui.interact(rect, background_id, Sense::click_and_drag());
    let pan_started = handle_pan_and_zoom(ui, &background, state, rect);
    // Record Graph Movements and Zoom in Undos: a pan begins an undo step,
    // the view as it was (`GraphModified("Graph panned")`).
    if pan_started && editor.user.options.record_graph_movements() {
        editor.document.checkpoint("Graph panned");
    }

    let view = state.view();
    let zoom = view.zoom;
    if state.show_grid {
        draw_grid(&painter, rect, &view);
    }
    if let Some(Drag::Push { press, .. }) = &state.drag {
        let pointer = ui.input(|input| input.pointer.interact_pos());
        draw_push_band(&painter, rect, &view, origin, *press, pointer);
    }

    let connections = layout_connections(editor, &layouts, &by_id, &frame, &view, origin, tab);
    let pointer = ui.input(|input| input.pointer.hover_pos());
    let pointer_on_graph = pointer.filter(|point| rect.contains(*point));
    let hovered_connection = pointer_on_graph.filter(|_| state.drag.is_none()).and_then(|point| {
        connections.iter().position(|connection| connection.points.windows(2).any(|segment| distance_to_segment(point, segment[0], segment[1]) < 5.0))
    });
    for (index, connection) in connections.iter().enumerate() {
        let selected = state.selected_connections.iter().any(|(to, port)| *to == connection.to && *port == connection.to_port);
        let hovered = hovered_connection == Some(index);
        let width = if selected || hovered { 3.0 } else { 2.0 };
        let stroke = PathStroke::new((width * zoom).max(1.0), connection.color);
        let path = PathShape::line(connection.points.clone(), stroke);
        painter.add(path);
    }

    // Groups (`ZIndex` 40) over the connections, then notes (45), then
    // the nodes.
    graph_groups::show(ui, &painter, editor, state, &frame.groups, origin, commands);
    let hidden_elements: HashSet<i32> = frame.hidden_in.keys().copied().collect();
    crate::ui::graph_notes::show(ui, &painter, editor, state, origin, &hidden_elements, commands);

    // Nodes, selected ones on top.
    let mut order: Vec<usize> = (0..layouts.len()).collect();
    order.sort_by_key(|index| editor.selection.contains(&layouts[*index].id));
    let mut node_responses: Vec<(usize, Response)> = Vec::new();
    for index in order.iter().copied() {
        let layout = &layouts[index];
        let screen = view.rect_to_screen(origin, layout.rect);
        if !screen.intersects(rect.expand(40.0)) {
            continue;
        }
        let response = ui.interact(screen, ui.id().with(("node", layout.id)), Sense::click_and_drag());
        if state.dimmed.contains(&layout.id) {
            // `TVINode.UpdateNodeCriteria`: `NodeElement.Opacity` 0.5.
            let mut dimmed_ui = ui.new_child(UiBuilder::new());
            dimmed_ui.multiply_opacity(0.5);
            let dimmed_painter = dimmed_ui.painter_at(rect);
            draw_node(&dimmed_ui, &dimmed_painter, editor, layout, screen, zoom);
        } else {
            draw_node(ui, &painter, editor, layout, screen, zoom);
        }
        node_responses.push((index, response));
    }

    // Ports above their nodes.
    let mut port_responses: Vec<(usize, PortLayout, Response)> = Vec::new();
    for index in order.iter().copied() {
        let layout = &layouts[index];
        for port in layout.ports() {
            let screen = view.rect_to_screen(origin, port.hit);
            if !screen.intersects(rect) {
                continue;
            }
            let id = ui.id().with(("port", layout.id, &port.name, port.is_input));
            let response = ui.interact(screen, id, Sense::click_and_drag());
            if response.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
            }
            port_responses.push((index, port.clone(), response));
        }
    }
    draw_ports(&painter, &layouts, &view, origin, state, pointer_on_graph);
    draw_shines(ui, &painter, state, &layouts, &view, origin);

    handle_port_drags(ui, editor, state, &layouts, &frame.groups, &port_responses, origin, commands);
    handle_node_interaction(ui, editor, state, &layouts, &frame.groups, &node_responses, origin, commands);
    handle_background(ui, editor, state, &layouts, &frame.groups, &background, &connections, hovered_connection, origin, commands);
    handle_drops(ui, editor, state, &layouts, commands);
    draw_drag_overlay(ui, &painter, state, &layouts, origin, pointer);
    rename_overlay(ui, editor, state, &layouts, origin, commands);
    graph_groups::rename_overlay(ui, editor, state, &frame.groups, origin);
    crate::ui::graph_popups::show(ui, editor, state, commands);
    ui.set_clip_rect(outer_clip);

    // Remember the view for saving.
    let view = state.view();
    editor.document.store_view_quietly(state.tab, [view.location.x, view.location.y], view.zoom as f64);
}

/// Every connection into this tab's nodes from this tab's nodes. One out
/// of a collapsed group starts at the group's right side, one into it ends
/// at its left (`GroupExit`, `GroupEnter`); those inside it are not drawn.
fn layout_connections(editor: &Editor, layouts: &[NodeLayout], by_id: &HashMap<i32, usize>, frame: &GroupFrame, view: &GraphView, origin: Pos2, tab: usize) -> Vec<ConnectionLayout> {
    let mut connections = Vec::new();
    let nodes = &editor.document.terrain().nodes;
    for node in nodes.values().filter(|node| node_tab(node) == tab) {
        let target_box = frame.hidden_in.get(&node.id);
        let target_layout = by_id.get(&node.id).map(|index| &layouts[*index]);
        for port in &node.ports {
            let Some(record) = &port.record else { continue };
            let Some(source) = nodes.get(&record.from).filter(|source| node_tab(source) == tab) else { continue };
            let source_box = frame.hidden_in.get(&source.id);
            if let (Some(source_box), Some(target_box)) = (source_box, target_box)
                && source_box == target_box
            {
                continue;
            }
            let source_layout = by_id.get(&source.id).map(|index| &layouts[*index]);
            let source_port = source_layout.and_then(|layout| layout.port(&record.from_port, false));
            let target_port = target_layout.and_then(|layout| layout.port(&port.name, true));
            // Portal to portal: no line (`RedrawConnections`).
            let source_publishes = source_port.is_some_and(|port| port.portal == Some(Portal::Publishing));
            let target_consumes = target_port.is_some_and(|port| port.portal == Some(Portal::Consuming));
            if source_publishes && target_consumes {
                continue;
            }
            let start = match (source_box, source_port) {
                (Some(group_box), _) => Pos2::new(group_box.right(), group_box.center().y),
                (None, Some(source_port)) => source_port.anchor,
                (None, None) => continue,
            };
            let end = match (target_box, target_port) {
                (Some(group_box), _) => Pos2::new(group_box.left(), group_box.center().y),
                (None, Some(target_port)) => target_port.anchor,
                (None, None) => continue,
            };
            let screen_start = view.to_screen(origin, start);
            let screen_end = view.to_screen(origin, end);
            let standard = target_layout.is_some_and(|layout| layout.size == NodeSize::Standard);
            let points = circuit_points(screen_start, screen_end, standard);
            let color = node_color(source);
            let connection = ConnectionLayout {
                to: node.id,
                to_port: port.name.clone(),
                points,
                color,
            };
            connections.push(connection);
        }
    }
    connections
}

/// The screen ends of the connection into `to_port` (`StartPoint`,
/// `EndPoint`), when both nodes show on the graph.
pub fn connection_ends(editor: &Editor, state: &GraphState, to: i32, to_port: &str) -> Option<(Pos2, Pos2)> {
    let target = editor.document.node(to)?;
    let record = target.port(to_port)?.record.clone()?;
    let source = editor.document.node(record.from)?;
    let default_size = editor.user.options.default_node_size().to_string();
    let source_layout = layout_node(source, &default_size);
    let target_layout = layout_node(target, &default_size);
    let start = source_layout.port(&record.from_port, false)?.anchor;
    let end = target_layout.port(to_port, true)?.anchor;
    let view = state.view();
    let origin = state.rect.min;
    Some((view.to_screen(origin, start), view.to_screen(origin, end)))
}

fn apply_zoom_request(state: &mut GraphState, layouts: &[NodeLayout], groups: &[GroupData], editor: &Editor, rect: Rect) {
    let Some(request) = state.zoom_request.take() else {
        return;
    };
    let tab = state.tab;
    let view = state.view_mut();
    if request == ZoomRequest::Reset {
        let center = view.location + rect.size() / (2.0 * view.zoom);
        view.zoom = 1.0;
        view.location = center - rect.size() / 2.0;
        return;
    }
    let centred = match request {
        ZoomRequest::Center(id) => layouts.iter().find(|layout| layout.id == id).map(|layout| layout.rect),
        ZoomRequest::CenterNote(id) => crate::ui::graph_notes::note_rect(editor, tab, id),
        ZoomRequest::CenterGroup(id) => groups.iter().find(|group| group.id == id).map(GroupData::bounds),
        _ => None,
    };
    if let Some(element) = centred {
        let visible = rect.size() / view.zoom;
        view.location = element.center().to_vec2() - visible / 2.0;
        return;
    }
    let chosen: Vec<&NodeLayout> = match request {
        ZoomRequest::Selection => layouts.iter().filter(|layout| editor.selection.contains(&layout.id)).collect(),
        _ => layouts.iter().collect(),
    };
    let Some(first) = chosen.first() else {
        return;
    };
    let mut bounds = first.rect;
    for layout in &chosen {
        bounds = bounds.union(layout.rect);
    }
    let bounds = bounds.expand(60.0);
    let zoom_x = rect.width() / bounds.width();
    let zoom_y = rect.height() / bounds.height();
    let zoom = zoom_x.min(zoom_y).clamp(MIN_ZOOM, 1.0);
    view.zoom = zoom;
    let visible = rect.size() / zoom;
    view.location = bounds.center().to_vec2() - visible / 2.0;
}

/// Pans with the middle button (anywhere on the graph) or a right drag on
/// its background, and zooms with the wheel. Returns whether a pan began
/// (the middle button pressed on the graph, as `PanButton` goes down, or
/// a right drag started).
fn handle_pan_and_zoom(ui: &Ui, background: &Response, state: &mut GraphState, rect: Rect) -> bool {
    let pointer = ui.input(|input| input.pointer.hover_pos());
    let over = pointer.is_some_and(|point| rect.contains(point));
    let over_toolbox = pointer.is_some_and(|point| state.toolbox_rect.contains(point));
    let on_graph_layer = pointer.and_then(|point| ui.ctx().layer_id_at(point)).is_none_or(|layer| layer.order == egui::Order::Background);
    let middle_pressed = ui.input(|input| input.pointer.button_pressed(PointerButton::Middle));
    let pan_started = (middle_pressed && over && !over_toolbox && on_graph_layer) || background.drag_started_by(PointerButton::Secondary);
    let panning = background.dragged_by(PointerButton::Middle) || background.dragged_by(PointerButton::Secondary);
    if panning {
        let delta = background.drag_delta();
        let view = state.view_mut();
        view.location -= delta / view.zoom;
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    }
    // Node drags pan too when the pointer is held with the middle button.
    let middle_held = ui.input(|input| input.pointer.button_down(PointerButton::Middle));
    if middle_held && over && !panning {
        let delta = ui.input(|input| input.pointer.delta());
        let view = state.view_mut();
        view.location -= delta / view.zoom;
    }
    if !over || over_toolbox || ui.ctx().any_popup_open() {
        return pan_started;
    }
    let (scroll, pinch) = ui.input(|input| (input.smooth_scroll_delta.y, input.zoom_delta()));
    let factor = (scroll * 0.0025).exp() * pinch;
    if (factor - 1.0).abs() < 1e-4 {
        return pan_started;
    }
    let Some(point) = pointer else {
        return pan_started;
    };
    let view = state.view_mut();
    let before = view.to_canvas(rect.min, point);
    view.zoom = (view.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    let after_offset = (point - rect.min) / view.zoom;
    view.location = before.to_vec2() - after_offset;
    pan_started
}

/// `SnapElementToGrid`: to the nearer multiple of the unit, halves down
/// (the remainder keeps the position's sign, as .NET's `%` does).
fn snap_to_unit(position: f32, unit: f32) -> f32 {
    let offset = position % unit;
    let half = unit * 0.5;
    let shift = if offset <= half { -offset } else { unit - offset };
    position + shift
}

fn draw_grid(painter: &egui::Painter, rect: Rect, view: &GraphView) {
    let levels = [(10.0, theme::GRID_FINE), (100.0, theme::GRID_COARSE), (500.0, theme::GRID_MEGA)];
    for (spacing, color) in levels {
        let screen_spacing = spacing * view.zoom;
        if screen_spacing < 6.0 {
            continue;
        }
        let stroke = Stroke::new(1.0, color);
        let first_x = (view.location.x / spacing).floor() * spacing;
        let mut x = first_x;
        loop {
            let screen_x = rect.left() + (x - view.location.x) * view.zoom;
            if screen_x > rect.right() {
                break;
            }
            painter.line_segment([Pos2::new(screen_x, rect.top()), Pos2::new(screen_x, rect.bottom())], stroke);
            x += spacing;
        }
        let first_y = (view.location.y / spacing).floor() * spacing;
        let mut y = first_y;
        loop {
            let screen_y = rect.top() + (y - view.location.y) * view.zoom;
            if screen_y > rect.bottom() {
                break;
            }
            painter.line_segment([Pos2::new(rect.left(), screen_y), Pos2::new(rect.right(), screen_y)], stroke);
            y += spacing;
        }
    }
}

/// How the node's build state shows on its main border: colour, dashes
/// (in stroke widths) and the title colour it forces.
/// The outline a node's state gives it (the `NodeState` triggers on
/// `MainBorder`), its dashes and the colour its name and icon take. Built
/// and building nodes keep the node style's own outline (`None`).
fn state_border(state: NodeState, building: bool) -> (Option<Color32>, Option<(f32, f32)>, Option<Color32>) {
    if building {
        return (None, None, None);
    }
    match state {
        NodeState::Built => (None, None, None),
        NodeState::Unbuilt => (Some(theme::NODE_BORDER_SELECTED), Some((1.0, 2.0)), None),
        NodeState::Error => (Some(theme::ACCENT_RED), Some((1.0, 2.0)), Some(theme::ACCENT_STRONG_RED)),
        NodeState::Bypassed => (Some(theme::ACCENT_PURPLE), Some((2.0, 4.0)), Some(theme::ACCENT_MUTED_PURPLE)),
        NodeState::Baked => (Some(theme::ACCENT_MUTED_GRAY), Some((1.0, 2.0)), None),
    }
}

fn draw_node(ui: &Ui, painter: &egui::Painter, editor: &Editor, layout: &NodeLayout, screen: Rect, zoom: f32) {
    let Some(node) = editor.document.node(layout.id) else {
        return;
    };
    let status = editor.status(layout.id);
    let bypassed = node.base.get("StateOverride").and_then(Value::as_str) == Some("Bypassed");
    let state = if bypassed { NodeState::Bypassed } else { status.state };
    let selected = editor.selection.contains(&layout.id);
    let (state_color, dashes, forced_title) = state_border(state, status.building);
    let mut title_color = forced_title.unwrap_or(layout.color);
    if state == NodeState::Bypassed {
        title_color = title_color.gamma_multiply(0.8);
    }
    // The node style (`UseModernTemplate`): Muted outlines nodes in
    // `Menu-Border` and a chosen one in its colour on #0F1011; Vibrant
    // outlines them in their colour and a chosen one in the primary accent
    // on #050505.
    let vibrant = editor.user.options.vibrant_nodes();
    let style_border = if vibrant { layout.color } else { theme::MENU_BORDER };
    let selection_border = if vibrant { theme::ACCENT_PRIMARY } else { layout.color };
    let selected_fill = if vibrant { theme::NODE_BACKGROUND_SELECTED_VIBRANT } else { theme::NODE_BACKGROUND_SELECTED };
    let radius = 4.0 * zoom;
    let fill = if status.building {
        theme::NODE_BACKGROUND_BUILDING
    } else if selected {
        selected_fill
    } else {
        theme::NODE_BACKGROUND
    };
    painter.rect_filled(screen, radius, fill);
    let stroke_width = (2.0 * zoom).max(1.0);
    let main_color = if selected { theme::NODE_BORDER_SELECTED } else { state_color.unwrap_or(style_border) };
    border(painter, screen, radius, Stroke::new(stroke_width, main_color), dashes);
    if selected {
        let selection_stroke = Stroke::new(stroke_width, selection_border);
        painter.rect_stroke(screen, radius, selection_stroke, StrokeKind::Inside);
    }

    let simplified = zoom < SIMPLIFIED_ZOOM;
    if simplified {
        let font = theme::semibold((24.0 * zoom).max(6.0));
        let galley = painter.layout(node.name.clone(), font, title_color, screen.width() - 8.0);
        let position = screen.center() - galley.size() / 2.0;
        painter.galley(position, galley, title_color);
        if status.building {
            building_bar(ui, painter, screen, zoom, 6.0, 3.0, layout.color);
        }
        return;
    }

    // Progress line while building (`PART_Progress`).
    if status.building {
        building_bar(ui, painter, screen, zoom, 3.0, 1.0, layout.color);
    }

    let (icon_offset, text_offset, text_center) = match layout.size {
        NodeSize::Standard => (Vec2::new(18.0, 14.0), Vec2::new(50.0, 14.0), false),
        NodeSize::Compact => (Vec2::new(45.0, (COMPACT_HEIGHT - 20.0) / 2.0), Vec2::new(80.0, 0.0), true),
        NodeSize::Small => (Vec2::new((SMALL_SIZE - 20.0) / 2.0, (SMALL_SIZE - 20.0) / 2.0), Vec2::ZERO, false),
    };
    let icon_rect = Rect::from_min_size(screen.min + icon_offset * zoom, Vec2::splat(20.0 * zoom));
    icons::paint(ui, icon_rect, &layout.icon, title_color);

    if layout.size != NodeSize::Small {
        let font = theme::semibold(13.0 * zoom);
        let max_width = if layout.size == NodeSize::Standard { 100.0 * zoom } else { (COMPACT_WIDTH - 105.0) * zoom };
        let galley = elided_galley(painter, &node.name, font, title_color, max_width);
        let mut position = screen.min + text_offset * zoom;
        if text_center {
            position.y = screen.center().y - galley.size().y / 2.0;
        }
        painter.galley(position, galley, title_color);
    } else {
        let font = theme::semibold(10.0 * zoom);
        let galley = elided_galley(painter, &node.name, font, title_color, SMALL_SIZE * zoom);
        let position = Pos2::new(screen.left() + 4.0 * zoom, screen.bottom() + 2.0 * zoom);
        painter.galley(position, galley, title_color);
    }

    // `PART_IconStack` (margin 0, 3, 1, 3), right-aligned at the bottom:
    // the build time (monospace 10, `ParamsN-Color6`, margin 4,
    // `Options.ShowBuildTime`), then locked, then marked for export.
    let mut icons_shown: Vec<(&str, Color32, f32)> = Vec::new();
    let locked = editor.preview_locked && editor.preview_node == Some(layout.id);
    if locked {
        icons_shown.push(("photo-sensor-3", theme::ACCENT_GREEN, 16.0));
    }
    if node.base.get("SaveDefinition").is_some_and(|value| !value.is_null()) {
        icons_shown.push(("floppy", theme::ACCENT_PRIMARY, 14.0));
    }
    let mut right = screen.right() - 1.0 * zoom;
    let stack_bottom = screen.bottom() - 3.0 * zoom;
    // `Options.ShowBuildTime`: without it the time is an empty text.
    let time_text = if editor.user.options.show_build_time() { format!("{:.2}s", status.milliseconds / 1000.0) } else { String::new() };
    let time_galley = painter.layout_no_wrap(time_text, theme::monospace(10.0 * zoom), BUILD_TIME_COLOR);
    // The stack is as tall as the time with its margins; icons centre in it.
    let stack_height = time_galley.size().y + 8.0 * zoom;
    let stack_center = stack_bottom - stack_height / 2.0;
    for (icon, color, size) in icons_shown.into_iter().rev() {
        let icon_size = size * zoom;
        let icon_rect = Rect::from_min_size(Pos2::new(right - 2.0 * zoom - icon_size, stack_center - icon_size / 2.0), Vec2::splat(icon_size));
        icons::paint(ui, icon_rect, icon, color);
        right -= icon_size + 2.0 * zoom;
    }
    let time_position = Pos2::new(right - 4.0 * zoom - time_galley.size().x, stack_center - time_galley.size().y / 2.0);
    painter.galley(time_position, time_galley, BUILD_TIME_COLOR);
}

/// `ParamsN-Color6`: a node's build time.
const BUILD_TIME_COLOR: Color32 = Color32::from_rgb(0x49, 0x50, 0x57);

/// How long a shine lasts (`ShineBoard`'s longest animation).
const SHINE_SECONDS: f64 = 0.5;
/// The band's opacity at its peak (`#60FFE01E`).
const SHINE_PEAK: f32 = 0x60 as f32 / 255.0;

/// `NodeElement.Shine`: the nodes the editor asks to shine start now,
/// unless Show shine effect on Node when build completes is off, a gizmo
/// is on (`O.IsShineDisabled`) or the interface's animations are off.
fn take_shines(ui: &Ui, editor: &mut Editor, state: &mut GraphState) {
    let asked = std::mem::take(&mut editor.shines);
    let options = &editor.user.options;
    if asked.is_empty() || !options.build_shine() || options.disable_animations() || editor.gizmo_active {
        return;
    }
    let now = ui.input(|input| input.time);
    for id in asked {
        state.shines.insert(id, now);
    }
}

/// `BuildShine` (`ShineRect`: margin 1, corners 4, over everything the
/// node shows): the node's colour seen through a band (`OpacityMask`:
/// transparent, #60, transparent) that sweeps across it left to right.
fn draw_shines(ui: &Ui, painter: &egui::Painter, state: &mut GraphState, layouts: &[NodeLayout], view: &GraphView, origin: Pos2) {
    if state.shines.is_empty() {
        return;
    }
    let now = ui.input(|input| input.time);
    state.shines.retain(|_, started| now - *started < SHINE_SECONDS);
    for layout in layouts {
        let Some(started) = state.shines.get(&layout.id) else {
            continue;
        };
        let stops = shine_stops((now - started) as f32);
        let screen = view.rect_to_screen(origin, layout.rect).shrink(view.zoom);
        let mesh = shine_mesh(screen, 4.0 * view.zoom, layout.color, stops);
        painter.add(Shape::mesh(mesh));
    }
    ui.ctx().request_repaint();
}

/// The band's stops `seconds` in, across the node (`ShineBoard`): from
/// -0.65, -0.5 and -0.49 to 1.25 (over 0.5 s), 1.3 and 1.31 (0.45 s).
fn shine_stops(seconds: f32) -> [f32; 3] {
    let along = |from: f32, to: f32, duration: f32| from + (to - from) * (seconds / duration).min(1.0);
    [along(-0.65, 1.25, 0.5), along(-0.5, 1.3, 0.45), along(-0.49, 1.31, 0.45)]
}

/// The band's opacity at `x` across the node: rising from the first stop
/// to its peak at the second, falling to nothing at the third.
fn shine_opacity(x: f32, stops: [f32; 3]) -> f32 {
    let [rise, peak, fall] = stops;
    if x <= rise || x >= fall {
        return 0.0;
    }
    if x < peak {
        return SHINE_PEAK * (x - rise) / (peak - rise);
    }
    SHINE_PEAK * (fall - x) / (fall - peak)
}

/// The band over a rounded rectangle: vertical strips meeting at the
/// stops (so the opacity is exact between them) and stepping round the
/// corners.
fn shine_mesh(rect: Rect, radius: f32, color: Color32, stops: [f32; 3]) -> egui::Mesh {
    let width = rect.width().max(1.0);
    let radius = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let corner = radius / width;
    let mut columns: Vec<f32> = vec![0.0, 1.0];
    columns.extend(stops.iter().copied().filter(|stop| *stop > 0.0 && *stop < 1.0));
    for step in 1..6 {
        let along = corner * step as f32 / 6.0;
        columns.push(along);
        columns.push(1.0 - along);
    }
    columns.sort_by(f32::total_cmp);
    columns.dedup();
    // How far a corner's curve keeps the band from the top and bottom.
    let inset = |x: f32| {
        let from_edge = (x * width).min((1.0 - x) * width);
        if from_edge >= radius {
            return 0.0;
        }
        let across = radius - from_edge;
        radius - (radius * radius - across * across).max(0.0).sqrt()
    };
    let mut mesh = egui::Mesh::default();
    for (index, x) in columns.iter().enumerate() {
        let screen_x = rect.left() + x * width;
        let shade = color.gamma_multiply(shine_opacity(*x, stops));
        let edge = inset(*x);
        mesh.colored_vertex(Pos2::new(screen_x, rect.top() + edge), shade);
        mesh.colored_vertex(Pos2::new(screen_x, rect.bottom() - edge), shade);
        if index > 0 {
            let top = (index * 2) as u32;
            mesh.add_triangle(top - 2, top - 1, top);
            mesh.add_triangle(top - 1, top + 1, top);
        }
    }
    mesh
}

/// The thin animated line across a building node's top.
fn building_bar(ui: &Ui, painter: &egui::Painter, screen: Rect, zoom: f32, margin: f32, height: f32, color: Color32) {
    let time = ui.input(|input| input.time) as f32;
    let track = Rect::from_min_size(screen.min + Vec2::splat(margin * zoom), Vec2::new(screen.width() - 2.0 * margin * zoom, (height * zoom).max(1.0)));
    painter.rect_filled(track, 0.0, theme::PROGRESS_EMPTY);
    let phase = (time * 0.8).fract();
    let width = track.width() * 0.3;
    let left = track.left() + (track.width() + width) * phase - width;
    let bar = Rect::from_min_max(Pos2::new(left.max(track.left()), track.top()), Pos2::new((left + width).min(track.right()), track.bottom()));
    if bar.width() > 0.0 {
        painter.rect_filled(bar, 0.0, color);
    }
    ui.ctx().request_repaint();
}

/// Text cut with an ellipsis to `max_width` (`TextTrimming.CharacterEllipsis`).
pub(crate) fn elided_galley(painter: &egui::Painter, text: &str, font: egui::FontId, color: Color32, max_width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(text.to_string(), egui::TextFormat::simple(font, color));
    job.wrap = egui::text::TextWrapping {
        max_width,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    painter.layout_job(job)
}

fn draw_ports(painter: &egui::Painter, layouts: &[NodeLayout], view: &GraphView, origin: Pos2, state: &GraphState, pointer: Option<Pos2>) {
    let zoom = view.zoom;
    let simplified = zoom < SIMPLIFIED_ZOOM;
    let snap_target = pointer.and_then(|point| snapping_port(layouts, view, origin, state, point));
    for layout in layouts {
        let mut painter = painter.clone();
        if state.dimmed.contains(&layout.id) {
            painter.multiply_opacity(0.5);
        }
        let painter = &painter;
        for port in layout.ports() {
            let dot = view.rect_to_screen(origin, port.dot);
            let hit = view.rect_to_screen(origin, port.hit);
            let hovered = pointer.is_some_and(|point| hit.contains(point));
            let snapping = snap_target.as_ref().is_some_and(|(node, name, is_input)| *node == layout.id && *name == port.name && *is_input == port.is_input);
            let is_mask = layout.mask.as_ref().is_some_and(|mask| mask.name == port.name) && port.is_input;
            let base = if is_mask { theme::ACCENT_MUTED_MAGENTA } else { layout.color };
            let opacity = if port.connected {
                1.0
            } else if hovered || snapping {
                0.8
            } else {
                0.4
            };
            let fill = base.gamma_multiply(opacity);
            if let Some(portal) = port.portal {
                draw_portal(painter, port, portal, base, view, origin, simplified);
            }
            let dot_shape = if simplified { Rect::from_center_size(dot.center(), Vec2::new(24.0, 16.0) * zoom) } else { dot };
            let corner = if simplified { 0.0 } else { dot.width() / 2.0 };
            painter.rect_filled(dot_shape, corner, fill);
            if snapping {
                let outline = if port.connected { theme::ACCENT_RED } else { theme::ACCENT_BLUE };
                painter.rect_stroke(dot_shape, corner, Stroke::new(2.0, outline), StrokeKind::Outside);
            }
            // Compact and small nodes hide their port labels (`HideLabel`).
            let label_hidden = layout.size != NodeSize::Standard || port.collapsed;
            if simplified || label_hidden || port.portal.is_some() {
                continue;
            }
            let label_color = if hovered || snapping { theme::LABEL_EMPHASIS } else { theme::PORT_LABEL };
            let font = theme::semibold(10.0 * zoom);
            let galley = painter.layout_no_wrap(port.label.clone(), font, label_color);
            let y = dot.center().y - galley.size().y / 2.0;
            let x = if port.is_input { dot.left() + 18.0 * zoom } else { dot.right() - 18.0 * zoom - galley.size().x };
            painter.galley(Pos2::new(x, y), galley, label_color);
        }
    }
}

/// A portal port (`PortElement` with `IsPortal`): a pill in the port's
/// colour reaching out of the node with the friendly name, and a dark ring
/// around the dot.
fn draw_portal(painter: &egui::Painter, port: &PortLayout, portal: Portal, color: Color32, view: &GraphView, origin: Pos2, simplified: bool) {
    let zoom = view.zoom;
    let element = view.rect_to_screen(origin, port.element);
    let font = theme::interface(11.0 * zoom);
    let (text_left, text_right, available) = match portal {
        Portal::Consuming => (6.0, 16.0, 114.0),
        Portal::Publishing => (18.0, 6.0, 75.0),
    };
    let text_width = painter.layout_no_wrap(port.portal_name.clone(), font.clone(), theme::PORTAL_FOREGROUND).size().x;
    let width = (text_width + (text_left + text_right) * zoom).clamp(36.0 * zoom, available * zoom);
    let height = 18.0 * zoom;
    let center_y = element.center().y;
    let pill = match portal {
        Portal::Consuming => {
            let right = element.left() + 14.0 * zoom;
            Rect::from_min_max(Pos2::new(right - width, center_y - height / 2.0), Pos2::new(right, center_y + height / 2.0))
        }
        Portal::Publishing => {
            let left = element.left() + 5.0 * zoom;
            Rect::from_min_max(Pos2::new(left, center_y - height / 2.0), Pos2::new(left + width, center_y + height / 2.0))
        }
    };
    let radius = if simplified { 0.0 } else { 9.0 * zoom };
    painter.rect_filled(pill, radius, color);
    painter.rect_stroke(pill, radius, Stroke::new(2.0 * zoom, theme::NODE_BACKGROUND), StrokeKind::Inside);
    let dot_center = view.rect_to_screen(origin, port.dot).center();
    painter.circle_filled(dot_center, 5.0 * zoom, theme::NODE_BACKGROUND);
    if simplified {
        return;
    }
    let name_width = pill.width() - (text_left + text_right) * zoom;
    let galley = elided_galley(painter, &port.portal_name, font, theme::PORTAL_FOREGROUND, name_width.max(1.0));
    let position = Pos2::new(pill.left() + text_left * zoom, center_y - galley.size().y / 2.0);
    painter.galley(position, galley, theme::PORTAL_FOREGROUND);
}

/// The port a dragged connection would be let go on (`HandleSnapToPorts`):
/// any port of another node, outputs as well as inputs. Let go outside
/// the graph, it is none: a port past the edge is not there to take it.
fn snapping_port(layouts: &[NodeLayout], view: &GraphView, origin: Pos2, state: &GraphState, point: Pos2) -> Option<(i32, String, bool)> {
    if !state.rect.contains(point) {
        return None;
    }
    let source_node = match &state.drag {
        Some(Drag::FromOutput { node, .. } | Drag::FromInput { node, .. }) => *node,
        _ => return None,
    };
    let mut best: Option<(f32, (i32, String, bool))> = None;
    for layout in layouts {
        if layout.id == source_node {
            continue;
        }
        for port in layout.ports() {
            let anchor = view.to_screen(origin, port.dot.center());
            let distance = anchor.distance(point);
            let near = if state.snap_to_ports { distance <= SNAP_DISTANCE.max(12.0 * view.zoom) } else { view.rect_to_screen(origin, port.hit).contains(point) };
            if !near {
                continue;
            }
            let better = best.as_ref().is_none_or(|(best_distance, _)| distance < *best_distance);
            if better {
                best = Some((distance, (layout.id, port.name.clone(), port.is_input)));
            }
        }
    }
    best.map(|(_, port)| port)
}

/// The real port a collapsed stand-in drags from (`InPort(1)`,
/// `OutPort(1)`), else the port itself.
fn drag_port_name(editor: &Editor, node: i32, port: &PortLayout) -> Option<String> {
    if !port.collapsed {
        return Some(port.name.clone());
    }
    let data = editor.document.node(node)?;
    let ports = if port.is_input { grid_inputs(data) } else { grid_outputs(data) };
    ports.get(1).map(|second| second.name.clone())
}

/// A dragged connection as it is let go: the port it was pulled from.
struct DraggedConnection {
    node: i32,
    port: String,
    from_output: bool,
    /// Pulled out of a compact node's collapsed ports.
    collapsed: bool,
}

/// What a dragged connection let go on a port does
/// (`NodeElement_OnConnectionManipulationCompleted`, `CreateConnection`).
#[derive(Clone, Debug, PartialEq)]
enum PortDrop {
    /// An output feeds an input.
    Connect { from: i32, from_port: String, to: i32, to_port: String },
    /// Output on output: a new Combine takes both (`CombineOutPorts`).
    CombineOutputs { dragged: i32, dragged_port: String, other: i32, other_port: String },
    /// On a collapsed stand-in: that node's ports to choose from
    /// (`ShowPortsMenu(ToNode, hidePrimary: true)`).
    PortsMenu { node: i32 },
    /// From collapsed ports onto another node's: both ports picked in the
    /// Create Connection window (`ShowCollapsedOutOutMenu`).
    CreateConnection { node: i32 },
    /// Input on input, or on its own node (`IsConnectionValid`).
    Nothing,
}

/// Where a connection let go on a node's `port` ends up.
fn port_drop(dragged: &DraggedConnection, node: i32, port: &str, is_input: bool) -> PortDrop {
    if node == dragged.node {
        return PortDrop::Nothing;
    }
    if port == COLLAPSED_PORT {
        if dragged.collapsed {
            return PortDrop::CreateConnection { node };
        }
        return PortDrop::PortsMenu { node };
    }
    let dragged_port = dragged.port.clone();
    let port = port.to_string();
    match (dragged.from_output, is_input) {
        (true, true) => PortDrop::Connect { from: dragged.node, from_port: dragged_port, to: node, to_port: port },
        (true, false) => PortDrop::CombineOutputs { dragged: dragged.node, dragged_port, other: node, other_port: port },
        (false, false) => PortDrop::Connect { from: node, from_port: port, to: dragged.node, to_port: dragged_port },
        (false, true) => PortDrop::Nothing,
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_port_drags(ui: &Ui, editor: &mut Editor, state: &mut GraphState, layouts: &[NodeLayout], groups: &[GroupData], ports: &[(usize, PortLayout, Response)], origin: Pos2, commands: &mut Vec<Command>) {
    let double_click_connect = editor.user.options.double_click_connect();
    for (index, port, response) in ports {
        let node = layouts[*index].id;
        let collapsed = port.collapsed;
        // `PortElement.OnMouseDoubleClick` (Double-Click Connect): a
        // connection follows the pointer until the next click.
        if double_click_connect && response.double_clicked() {
            let Some(name) = drag_port_name(editor, node, port) else { continue };
            state.drag = if port.is_input {
                Some(Drag::FromInput { node, port: name, sticky: true, collapsed })
            } else {
                Some(Drag::FromOutput { node, port: name, sticky: true, collapsed })
            };
            continue;
        }
        if !response.drag_started_by(PointerButton::Primary) {
            continue;
        }
        let Some(name) = drag_port_name(editor, node, port) else { continue };
        if !port.is_input {
            state.drag = Some(Drag::FromOutput { node, port: name, sticky: false, collapsed });
            continue;
        }
        // Pulling a connected input detaches its connection. A connection
        // from a node the project no longer has (one of an unknown type,
        // dropped on open) is not drawn, so the input acts unconnected. The
        // collapsed stand-in is no connection's end.
        let record = editor.document.node(node).and_then(|data| data.port(&name)).and_then(|data| data.record.clone());
        let source_exists = record.as_ref().is_some_and(|record| editor.document.node(record.from).is_some());
        match record {
            Some(record) if source_exists && !collapsed => {
                editor.document.disconnect(node, &name);
                editor.changed();
                state.drag = Some(Drag::FromOutput { node: record.from, port: record.from_port, sticky: false, collapsed: false });
            }
            _ => state.drag = Some(Drag::FromInput { node, port: name, sticky: false, collapsed }),
        }
    }

    let sticky = matches!(state.drag, Some(Drag::FromOutput { sticky: true, .. } | Drag::FromInput { sticky: true, .. }));
    let ended = if sticky {
        let started_now = ports.iter().any(|(_, _, response)| response.double_clicked());
        let clicked = ui.input(|input| input.pointer.primary_clicked());
        clicked && !started_now
    } else {
        ui.input(|input| input.pointer.primary_released())
    };
    if !ended {
        return;
    }
    let Some(drag) = state.drag.clone() else {
        return;
    };
    let dragged = match drag {
        Drag::FromOutput { node, port, collapsed, .. } => DraggedConnection { node, port, from_output: true, collapsed },
        Drag::FromInput { node, port, collapsed, .. } => DraggedConnection { node, port, from_output: false, collapsed },
        _ => return,
    };
    let pointer = ui.input(|input| input.pointer.interact_pos());
    let view = state.view();
    let target = pointer.and_then(|point| snapping_port(layouts, &view, origin, state, point));
    state.drag = None;
    if let Some((target_node, target_port, target_is_input)) = target {
        let at = pointer.unwrap_or(origin);
        let dropped = port_drop(&dragged, target_node, &target_port, target_is_input);
        match dropped {
            PortDrop::Connect { from, from_port, to, to_port } => {
                let command = GraphCommand::ConnectPorts { from, from_port, to, to_port };
                commands.push(Command::Graph(command));
            }
            PortDrop::CombineOutputs { dragged: dragged_node, dragged_port, other, other_port } => {
                let command = GraphCommand::CombineOutputs { dragged: dragged_node, dragged_port, other, other_port };
                commands.push(Command::Graph(command));
            }
            PortDrop::PortsMenu { node } => {
                state.popup = Some(GraphPopup::Ports { from: dragged.node, from_port: dragged.port, from_output: dragged.from_output, node, at });
            }
            PortDrop::CreateConnection { node } => {
                state.create_connection = Some(CreateConnection::new(dragged.node, node, dragged.from_output));
            }
            PortDrop::Nothing => {}
        }
        return;
    }
    let Some(point) = pointer.filter(|point| state.rect.contains(*point)) else {
        return;
    };
    // Let go on one of its own node's ports: nothing (`IsConnectionValid`).
    let own_layout = layouts.iter().find(|layout| layout.id == dragged.node);
    let on_own_port = own_layout.is_some_and(|layout| layout.ports().any(|port| view.rect_to_screen(origin, port.hit).contains(point)));
    if on_own_port {
        return;
    }
    let canvas = view.to_canvas(origin, point);
    // Dropped on a collapsed group: the group's portals are offered.
    let collapsed_group = groups.iter().find(|group| group.collapsed && group.contains([canvas.x, canvas.y]));
    if let Some(group) = collapsed_group {
        if dragged.from_output {
            return;
        }
        state.popup = Some(GraphPopup::GroupPortals { group: group.id, to: dragged.node, to_port: dragged.port, at: point });
        return;
    }
    // `OnConnectionDraggedOut`: the node search, to add a node joined to
    // the dragged port.
    let source = NodeSource::Port { node: dragged.node, port: dragged.port, output: dragged.from_output };
    let search = SearchPopup::new(SearchMode::Nodes { source }, point, [canvas.x, canvas.y]);
    state.popup = Some(GraphPopup::Search(search));
}

#[allow(clippy::too_many_arguments)]
fn handle_node_interaction(ui: &Ui, editor: &mut Editor, state: &mut GraphState, layouts: &[NodeLayout], groups: &[GroupData], nodes: &[(usize, Response)], origin: Pos2, commands: &mut Vec<Command>) {
    let view = state.view();
    let modifiers = ui.input(|input| input.modifiers);
    for (index, response) in nodes {
        let id = layouts[*index].id;
        let tooltip = node_tooltip(editor, id);
        let response = response.clone().on_hover_text(tooltip);
        if response.clicked() {
            if modifiers.command || modifiers.shift {
                editor.toggle_selected(id);
            } else {
                state.clear_element_selection();
                editor.select(Some(id));
            }
        }
        // `NodeElement.OnMouseDoubleClick`: `CycleNodeSizes`.
        if response.double_clicked() {
            commands.push(Command::CycleNodeSize(id));
        }
        if response.drag_started_by(PointerButton::Primary) && state.drag.is_none() {
            if !editor.selection.contains(&id) {
                state.clear_element_selection();
                editor.select(Some(id));
            }
            let pointer = response.interact_pointer_pos().unwrap_or(origin);
            let pointer_start = view.to_canvas(origin, pointer);
            start_element_drag(editor, state, id, pointer_start);
        }
        response.context_menu(|ui| {
            if !editor.selection.contains(&id) {
                editor.select(Some(id));
            }
            node_context_menu(ui, editor, commands);
        });
    }
    drag_elements(ui, editor, state, layouts, groups, origin);
}

/// The drag of the selection begins: locked elements stay (`Locked`), and
/// a group's nodes move with the group rather than on their own.
fn start_element_drag(editor: &mut Editor, state: &mut GraphState, primary: i32, pointer_start: Pos2) {
    let groups: Vec<(i32, [f32; 2])> = state.selected_groups.iter().filter_map(|id| editor.document.group(*id)).filter(|group| !group.locked).map(|group| (group.id, group.position)).collect();
    let carried: HashSet<i32> = groups.iter().filter_map(|(id, _)| editor.document.group(*id)).flat_map(|group| group.children).collect();
    let mut nodes = Vec::new();
    for selected in &editor.selection {
        if carried.contains(selected) || editor.document.node_locked_in_group(*selected) {
            continue;
        }
        if let Some(node) = editor.document.node(*selected) {
            nodes.push((*selected, node.position));
        }
    }
    let note = state.selected_note.filter(|note| !carried.contains(note)).and_then(|note| note_corner(editor, note).map(|corner| (note, corner)));
    editor.document.checkpoint("Move");
    state.drag = Some(Drag::Elements { pointer_start, primary, nodes, note, groups });
}

fn note_corner(editor: &Editor, id: i32) -> Option<[f32; 2]> {
    let notes = editor.document.notes();
    let fields = notes.iter().find(|(note, _)| *note == id).map(|(_, fields)| *fields)?;
    let position = fields.get("Position")?;
    let x = position.get("X").and_then(Value::as_f64)? as f32;
    let y = position.get("Y").and_then(Value::as_f64)? as f32;
    Some([x, y])
}

fn drag_elements(ui: &Ui, editor: &mut Editor, state: &mut GraphState, layouts: &[NodeLayout], groups: &[GroupData], origin: Pos2) {
    let Some(Drag::Elements { pointer_start, primary, nodes, note, groups: dragged_groups }) = state.drag.clone() else {
        return;
    };
    let view = state.view();
    let dragging = ui.input(|input| input.pointer.primary_down());
    let pointer = ui.input(|input| input.pointer.interact_pos());
    if let Some(pointer) = pointer {
        let current = view.to_canvas(origin, pointer);
        let delta = current - pointer_start;
        let place = |start: [f32; 2]| {
            let mut x = start[0] + delta.x;
            let mut y = start[1] + delta.y;
            if state.snap_to_grid {
                x = snap_to_unit(x, state.grid_unit);
                y = snap_to_unit(y, state.grid_unit);
            }
            [x, y]
        };
        let mut moves: Vec<(i32, [f32; 2])> = nodes.iter().map(|(id, start)| (*id, place(*start))).collect();
        // Snap to Items aligns the node grabbed; the others keep their
        // places beside it.
        state.snap_lines.clear();
        let primary_move = moves.iter().find(|(id, _)| *id == primary).map(|(_, position)| *position);
        if editor.user.options.snap_to_items()
            && let Some(position) = primary_move
            && let Some(layout) = layouts.iter().find(|layout| layout.id == primary)
        {
            let size = layout.rect.size();
            let moving: HashSet<i32> = nodes.iter().map(|(id, _)| *id).collect();
            let visible = Rect::from_min_size(view.location.to_pos2(), state.rect.size() / view.zoom);
            let others: Vec<Rect> = layouts.iter().filter(|other| !moving.contains(&other.id) && visible.contains(other.rect.min)).map(|other| other.rect).collect();
            let (offset, lines) = snap_to_items(Rect::from_min_size(Pos2::new(position[0], position[1]), size), &others);
            for (_, moved) in moves.iter_mut() {
                moved[0] += offset.x;
                moved[1] += offset.y;
            }
            state.snap_lines = lines;
        }
        editor.document.move_nodes(&moves, false);
        if let Some((id, start)) = note {
            let moved = place(start);
            editor.document.set_note_vector(id, "Position", moved, false);
        }
        for (id, start) in &dragged_groups {
            let moved = place(*start);
            editor.document.move_group(*id, moved, false);
        }
        update_group_states(editor, state, groups, primary, current);
    }
    if dragging {
        return;
    }
    // `NodeElement_MoveCompleted`: what was dragged out of its group's box
    // leaves it; what was dragged into one joins it.
    let tab = state.tab;
    let mut settled: Vec<(i32, [f32; 2], usize)> = Vec::new();
    for (id, _) in &nodes {
        if let Some(node) = editor.document.node(*id) {
            settled.push((*id, node.position, tab));
        }
    }
    if let Some((id, _)) = note
        && let Some(corner) = note_corner(editor, id)
    {
        settled.push((id, corner, tab));
    }
    editor.document.settle_in_groups(&settled);
    state.drag = None;
    state.snap_lines.clear();
    state.group_states.clear();
}

/// While a node is dragged: the group under the pointer that would take
/// it shows `Incoming`, the group it would leave `Outgoing`.
fn update_group_states(editor: &Editor, state: &mut GraphState, groups: &[GroupData], primary: i32, pointer: Pos2) {
    state.group_states.clear();
    let owner = editor.document.group_of(primary);
    for group in groups {
        let over = group.contains([pointer.x, pointer.y]);
        let owns = owner == Some(group.id);
        if over && !owns {
            state.group_states.insert(group.id, GroupState::Incoming);
        } else if !over && owns {
            state.group_states.insert(group.id, GroupState::Outgoing);
        }
    }
}

/// `PrepareToPushElements`: every node, group and note of the tab whose
/// corner lies right of `x`, with where it is. Nodes and notes of a group
/// that is pushed go with it, not on their own.
fn elements_to_push(document: &Document, tab: usize, x: f32) -> PushedElements {
    let mut elements = PushedElements::default();
    let mut carried: HashSet<i32> = HashSet::new();
    for group in document.groups() {
        if group.graph_index != tab || group.position[0] <= x {
            continue;
        }
        carried.extend(group.children.iter().copied());
        elements.groups.push((group.id, group.position));
    }
    for node in document.terrain().nodes.values() {
        if node_tab(node) != tab || node.position[0] <= x || carried.contains(&node.id) {
            continue;
        }
        elements.nodes.push((node.id, node.position));
    }
    for (id, fields) in document.notes() {
        let graph_index = fields.get("GraphIndex").and_then(Value::as_u64).unwrap_or(0) as usize;
        let position = fields.get("Position");
        let coordinate = |axis: &str| position.and_then(|position| position.get(axis)).and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let corner = [coordinate("X"), coordinate("Y")];
        if graph_index != tab || corner[0] <= x || carried.contains(&id) {
            continue;
        }
        elements.notes.push((id, corner));
    }
    elements
}

/// A push begins at `press` (canvas units): one undo step for it, when
/// there is anything to push.
fn start_push(editor: &mut Editor, state: &mut GraphState, press: Pos2) {
    let elements = elements_to_push(&editor.document, state.tab, press.x);
    if !elements.is_empty() {
        editor.document.checkpoint("Push");
    }
    state.drag = Some(Drag::Push { press, elements });
}

/// `PushElements`: what lay right of where the push began moves across
/// with the pointer (back too), until the button comes up.
fn push_elements(ui: &Ui, editor: &mut Editor, state: &mut GraphState, origin: Pos2) {
    let Some(Drag::Push { press, elements }) = state.drag.clone() else {
        return;
    };
    let view = state.view();
    let pointer = ui.input(|input| input.pointer.interact_pos());
    if let Some(pointer) = pointer {
        let shift = view.to_canvas(origin, pointer).x - press.x;
        let pushed = |start: [f32; 2]| [start[0] + shift, start[1]];
        for (id, start) in &elements.groups {
            editor.document.move_group(*id, pushed(*start), false);
        }
        let moves: Vec<(i32, [f32; 2])> = elements.nodes.iter().map(|(id, start)| (*id, pushed(*start))).collect();
        editor.document.move_nodes(&moves, false);
        for (id, start) in &elements.notes {
            editor.document.set_note_vector(*id, "Position", pushed(*start), false);
        }
    }
    ui.ctx().set_cursor_icon(CursorIcon::ResizeEast);
    if !ui.input(|input| input.pointer.primary_down()) {
        state.drag = None;
    }
}

/// `VerticalDragRect`: from where the push began to the pointer, the
/// graph's whole height, behind everything else.
fn draw_push_band(painter: &egui::Painter, rect: Rect, view: &GraphView, origin: Pos2, press: Pos2, pointer: Option<Pos2>) {
    let Some(pointer) = pointer else {
        return;
    };
    let left = view.to_screen(origin, press).x;
    if pointer.x <= left {
        return;
    }
    // Past the graph's top and bottom, so only the sides' lines show.
    let top_left = Pos2::new(left, rect.top() - 2.0);
    let bottom_right = Pos2::new(pointer.x, rect.bottom() + 2.0);
    let band = Rect::from_min_max(top_left, bottom_right);
    // The rectangle's `Opacity` 0.75 over its fill and line.
    let fill = theme::ACCENT_TRANSPARENT_PURPLE.gamma_multiply(0.75);
    let line = Stroke::new(view.zoom, theme::ACCENT_PURPLE.gamma_multiply(0.75));
    painter.rect_filled(band, 0.0, fill);
    painter.rect_stroke(band, 0.0, line, StrokeKind::Inside);
}

/// `SnapToOtherShapes`: the dragged box's edges and centres against each
/// other node's, closest first within 36; returns the shift and the lines
/// that show it.
fn snap_to_items(dragged: Rect, others: &[Rect]) -> (Vec2, Vec<[Pos2; 2]>) {
    let mut best_x: Option<(f32, f32, f32, Rect)> = None;
    let mut best_y: Option<(f32, f32, f32, Rect)> = None;
    let consider = |best: &mut Option<(f32, f32, f32, Rect)>, distance: f32, shift: f32, line_at: f32, other: Rect| {
        let closest = best.map_or(SNAP_ITEMS_DISTANCE, |(found, _, _, _)| found);
        if distance < closest {
            *best = Some((distance, shift, line_at, other));
        }
    };
    for other in others {
        // Top to top, bottom to bottom.
        consider(&mut best_y, (dragged.top() - other.top()).abs(), other.top() - dragged.top(), other.top(), *other);
        consider(&mut best_y, (dragged.bottom() - other.bottom()).abs(), other.bottom() - dragged.bottom(), other.bottom(), *other);
        // Right against left, left against right, then the centres.
        consider(&mut best_x, (dragged.right() - other.left()).abs(), other.left() - dragged.right(), other.left(), *other);
        consider(&mut best_x, (dragged.left() - other.right()).abs(), other.right() - dragged.left(), other.right(), *other);
        consider(&mut best_x, (dragged.center().x - other.center().x).abs(), other.center().x - dragged.center().x, other.center().x, *other);
        consider(&mut best_y, (dragged.center().y - other.center().y).abs(), other.center().y - dragged.center().y, other.center().y, *other);
    }
    let shift_x = best_x.map_or(0.0, |(_, shift, _, _)| shift);
    let shift_y = best_y.map_or(0.0, |(_, shift, _, _)| shift);
    let placed = dragged.translate(Vec2::new(shift_x, shift_y));
    let mut lines = Vec::new();
    if let Some((_, _, x, other)) = best_x {
        let top = placed.top().min(other.top());
        let bottom = placed.bottom().max(other.bottom());
        lines.push([Pos2::new(x, top), Pos2::new(x, bottom)]);
    }
    if let Some((_, _, y, other)) = best_y {
        let left = placed.left().min(other.left());
        let right = placed.right().max(other.right());
        lines.push([Pos2::new(left, y), Pos2::new(right, y)]);
    }
    (Vec2::new(shift_x, shift_y), lines)
}

/// The node's type, how long its last build took and why it failed.
fn node_tooltip(editor: &Editor, id: i32) -> String {
    let Some(node) = editor.document.node(id) else {
        return String::new();
    };
    let spec = Catalogue::get().node(&node.type_name);
    let type_name = spec.and_then(|spec| spec.display_name.clone()).unwrap_or_else(|| node.type_name.clone());
    let status = editor.status(id);
    let mut lines = vec![format!("{} ({type_name})", node.name)];
    if status.state == NodeState::Built {
        lines.push(format!("Built in {:.0} ms", status.milliseconds));
    }
    if let Some(error) = status.error {
        lines.push(error);
    }
    lines.join("\n")
}

/// The node's right-click menu (`_ctxNodeMenu` as `PrepareNodeMenu`
/// builds it). With several nodes selected only what applies to all shows
/// (`RefreshNodeMenu`), and check marks stand for the selection
/// (`_ctxNodeMenu_Opened`).
pub fn node_context_menu(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    menu_style(ui);
    let several = editor.selection.len() > 1;
    let active = editor.active_node();
    let node = active.and_then(|id| editor.document.node(id));
    let selected_nodes: Vec<&NodeData> = editor.selection.iter().filter_map(|id| editor.document.node(*id)).collect();
    let bypassed = |node: &NodeData| node.base.get("StateOverride").and_then(Value::as_str) == Some("Bypassed");

    if menu_item(ui, None, None, "Refresh", "F5").clicked() {
        commands.push(Command::Refresh);
    }
    if !several {
        if menu_item(ui, Some("rotate-360"), Some(theme::ACCENT_BLUE), "Reset", "Ctrl R").clicked() {
            commands.push(Command::ResetNode);
        }
        if menu_item(ui, None, None, "Rename", "F2").clicked() {
            commands.push(Command::Rename);
        }
    }
    if menu_item(ui, Some("stack"), None, "Duplicate", "Ctrl D").clicked() {
        commands.push(Command::Duplicate);
    }
    menu_separator(ui);

    let all_bypassed = !selected_nodes.is_empty() && selected_nodes.iter().all(|node| bypassed(node));
    if checked_icon_item(ui, all_bypassed, Some("road-sign"), None, "Bypass", "B") {
        commands.push(Command::Bypass);
    }
    if !several {
        let id = active.unwrap_or_default();
        let bookmarked = editor.document.bookmarks().contains(&id);
        if checked_icon_item(ui, bookmarked, None, None, "Bookmark", "") {
            commands.push(Command::ToggleBookmark);
        }
        let locked = editor.preview_locked && editor.preview_node == active;
        if checked_icon_item(ui, locked, None, None, "Lock Preview", "F") {
            commands.push(Command::LockPreview);
        }
        let underlay = active.is_some() && editor.document.underlay_node() == id;
        if checked_icon_item(ui, underlay, None, None, "Use as Underlay", "G") {
            commands.push(Command::ToggleUnderlay);
        }
        let ignored = node.and_then(|node| node.base.get("IgnoreUnderlay")).and_then(Value::as_bool).unwrap_or(false);
        if checked_icon_item(ui, ignored, None, None, "Exclude from Underlay", "") {
            commands.push(Command::ToggleIgnoreUnderlay);
        }
    }
    // One node: exporting and enabled; several: all marked.
    let exporting = if several {
        selected_nodes.iter().all(|node| ExportSettings::of(node).is_some())
    } else {
        node.and_then(ExportSettings::of).is_some_and(|settings| settings.enabled)
    };
    if checked_icon_item(ui, exporting, Some("device-floppy"), Some(theme::ACCENT_BLUE), "Mark for Export", "F3") {
        commands.push(Command::MarkForExport);
    }
    menu_separator(ui);

    if !several {
        if menu_item(ui, Some("replace"), Some(theme::ACCENT_MUTED_GREEN), "Replace", "Ctrl Q").clicked() {
            commands.push(Command::Graph(GraphCommand::ReplaceSearch));
        }
        if menu_item(ui, None, None, "Insert chokepoint", "").clicked()
            && let Some(id) = active
        {
            commands.push(Command::Graph(GraphCommand::ChokepointMenu(id)));
        }
        if menu_item(ui, None, None, "Manage Portals", "P").clicked() {
            commands.push(Command::Graph(GraphCommand::ManagePortals));
        }
    } else {
        submenu(ui, None, None, "Combine into", |ui| {
            let entries = [
                ("x-combine", "Combine as Inputs", "F8", crate::document::CombineKind::Inputs),
                ("x-mixer", "Mixer as Color Layers", "Ctrl F8", crate::document::CombineKind::Mixer),
                ("x-layers", "Layers as Normal Layers", "Ctrl Shift F8", crate::document::CombineKind::Layers),
            ];
            for (icon, text, shortcut, kind) in entries {
                if menu_item(ui, Some(icon), None, text, shortcut).clicked() {
                    commands.push(Command::Graph(GraphCommand::Combine(kind)));
                }
            }
        });
    }
    if menu_item(ui, Some("square-rounded-x"), Some(theme::ACCENT_RED), "Delete", "Del").clicked() {
        commands.push(Command::Delete);
    }
    if several {
        return;
    }
    menu_separator(ui);
    if menu_item(ui, None, None, "Copy Properties", "").clicked() {
        commands.push(Command::CopyProperties);
    }
    // Only onto a node of the copied node's type.
    let copied_type = editor.copied_node.as_ref().map(|copied| copied.type_name.as_str());
    let can_paste = node.is_some_and(|node| copied_type == Some(node.type_name.as_str()));
    if menu_item_enabled(ui, None, None, "Paste Properties", "", can_paste).clicked() {
        commands.push(Command::PasteProperties);
    }
    menu_separator(ui);
    if menu_item(ui, Some("picture-in-picture"), Some(theme::ACCENT_MUTED_BLUE), "Show Locked 2D Viewport", "").clicked() {
        commands.push(Command::ShowLocked2DViewport);
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_background(
    ui: &Ui,
    editor: &mut Editor,
    state: &mut GraphState,
    layouts: &[NodeLayout],
    groups: &[GroupData],
    background: &Response,
    connections: &[ConnectionLayout],
    hovered_connection: Option<usize>,
    origin: Pos2,
    commands: &mut Vec<Command>,
) {
    let view = state.view();
    let modifiers = ui.input(|input| input.modifiers);
    let additive = modifiers.command || modifiers.shift;
    // Ctrl and Shift together push elements aside instead of selecting.
    let pushing = modifiers.command && modifiers.shift;
    let hovered = hovered_connection.map(|index| &connections[index]);
    if background.clicked() {
        state.renaming = None;
        match hovered {
            Some(connection) => {
                let key = (connection.to, connection.to_port.clone());
                if additive {
                    toggle_connection(state, key);
                } else {
                    state.clear_element_selection();
                    editor.select(None);
                    state.selected_connections = vec![key];
                }
            }
            None if pushing => {}
            None => {
                state.clear_element_selection();
                editor.select(None);
            }
        }
    }
    // A connection right-clicked: its menu (`_ctxConnection`).
    if background.secondary_clicked()
        && let Some(point) = background.interact_pointer_pos()
    {
        state.context_point = Some(view.to_canvas(origin, point));
        if let Some(connection) = hovered {
            let key = (connection.to, connection.to_port.clone());
            if !state.selected_connections.contains(&key) {
                state.clear_element_selection();
                editor.select(None);
                state.selected_connections = vec![key.clone()];
            }
            state.popup = Some(GraphPopup::Connection { to: key.0, to_port: key.1, at: point });
        }
    }
    if background.drag_started_by(PointerButton::Primary) && state.drag.is_none() {
        if pushing {
            // The push reaches back to where the button went down.
            let press = ui.input(|input| input.pointer.press_origin()).unwrap_or(origin);
            let canvas = view.to_canvas(origin, press);
            start_push(editor, state, canvas);
        } else {
            let start = background.interact_pointer_pos().unwrap_or(origin);
            state.drag = Some(Drag::Marquee { start, additive });
        }
    }
    push_elements(ui, editor, state, origin);
    if let Some(Drag::Marquee { start, additive }) = state.drag.clone() {
        let released = !ui.input(|input| input.pointer.primary_down());
        if released {
            let end = ui.input(|input| input.pointer.interact_pos()).unwrap_or(start);
            let screen_box = Rect::from_two_pos(start, end);
            select_in_box(editor, state, layouts, groups, connections, screen_box, additive);
            state.drag = None;
        }
    }
    let on_connection = hovered.is_some();
    if !on_connection {
        background.context_menu(|ui| {
            canvas_context_menu(ui, editor, state, commands);
        });
    }
    // Keys while the pointer is over the graph and nothing types.
    let typing = ui.ctx().egui_wants_keyboard_input();
    let over = ui.input(|input| input.pointer.hover_pos()).is_some_and(|point| state.rect.contains(point));
    if !typing && over && ui.input(|input| input.key_pressed(Key::Escape)) {
        state.drag = None;
        state.selected_connections.clear();
    }
}

fn toggle_connection(state: &mut GraphState, key: (i32, String)) {
    if let Some(position) = state.selected_connections.iter().position(|selected| *selected == key) {
        state.selected_connections.remove(position);
    } else {
        state.selected_connections.push(key);
    }
}

/// `SelectItemsInRect`: nodes and notes the box touches, groups wholly in
/// it, connections crossing it; locked ones stay out.
#[allow(clippy::too_many_arguments)]
fn select_in_box(editor: &mut Editor, state: &mut GraphState, layouts: &[NodeLayout], groups: &[GroupData], connections: &[ConnectionLayout], screen_box: Rect, additive: bool) {
    let view = state.view();
    let origin = state.rect.min;
    let canvas_box = Rect::from_two_pos(view.to_canvas(origin, screen_box.min), view.to_canvas(origin, screen_box.max));
    let mut chosen: Vec<i32> = if additive { editor.selection.clone() } else { Vec::new() };
    for layout in layouts {
        let touched = layout.rect.intersects(canvas_box);
        if touched && !chosen.contains(&layout.id) && !editor.document.node_locked_in_group(layout.id) {
            chosen.push(layout.id);
        }
    }
    if !additive {
        state.clear_element_selection();
    }
    for group in groups.iter().filter(|group| !group.locked) {
        let inside = canvas_box.contains_rect(group.bounds());
        if inside && !state.selected_groups.contains(&group.id) {
            state.selected_groups.push(group.id);
        }
    }
    for connection in connections {
        let (Some(start), Some(end)) = (connection.points.first(), connection.points.last()) else { continue };
        let key = (connection.to, connection.to_port.clone());
        if segment_meets_rect(*start, *end, screen_box) && !state.selected_connections.contains(&key) {
            state.selected_connections.push(key);
        }
    }
    let tab = state.tab;
    if state.selected_note.is_none() {
        let notes = editor.document.notes();
        let touched_note = notes.iter().map(|(id, _)| *id).find(|id| crate::ui::graph_notes::note_rect(editor, tab, *id).is_some_and(|rect| rect.intersects(canvas_box)));
        state.selected_note = touched_note;
    }
    editor.set_selection(chosen);
}

/// Right-click on empty graph: add nodes by category and family.
fn canvas_context_menu(ui: &mut Ui, editor: &Editor, state: &mut GraphState, commands: &mut Vec<Command>) {
    menu_style(ui);
    crate::ui::toolbox::add_node_menus(ui, editor, state, commands);
    menu_separator(ui);
    if menu_item(ui, Some("note"), Some(theme::ACCENT_STRONG_YELLOW), "Add Note", "").clicked() {
        commands.push(Command::AddNote);
    }
    if menu_item(ui, Some("zoom-scan"), None, "Zoom to Extents", "Ctrl 0").clicked() {
        commands.push(Command::ZoomExtents);
    }
}

/// Files dropped on the graph (`DropFiles`), where they land, and a
/// toolbox entry let go over it (`canvas_Drop`, onto a node to follow it).
fn handle_drops(ui: &Ui, editor: &Editor, state: &mut GraphState, layouts: &[NodeLayout], commands: &mut Vec<Command>) {
    let view = state.view();
    let origin = state.rect.min;
    let latest = ui.input(|input| input.pointer.latest_pos());
    let dropped: Vec<PathBuf> = ui.input(|input| input.raw.dropped_files.iter().map(|file| file.path().to_path_buf()).collect());
    // Files dropped elsewhere (a file field, say) are not the graph's; where
    // the system can't say where they landed, they go to its middle.
    let landing = match latest {
        Some(point) if ui.rect_contains_pointer(state.rect) => Some(point),
        Some(_) => None,
        None => Some(state.rect.center()),
    };
    if let (false, Some(point)) = (dropped.is_empty(), landing) {
        let canvas = view.to_canvas(origin, point);
        commands.push(Command::Graph(GraphCommand::DropFiles { files: dropped, at: [canvas.x, canvas.y] }));
    }
    if state.toolbox_drag.is_none() {
        return;
    }
    let released = ui.input(|input| input.pointer.primary_released() || !input.pointer.primary_down());
    if !released {
        return;
    }
    let Some(type_name) = state.toolbox_drag.take() else {
        return;
    };
    let Some(point) = latest.filter(|point| state.rect.contains(*point)) else {
        return;
    };
    let canvas = view.to_canvas(origin, point);
    let onto = layouts.iter().rev().find(|layout| layout.rect.contains(canvas)).map(|layout| layout.id);
    let _ = editor;
    commands.push(Command::Graph(GraphCommand::ToolboxDrop { type_name, at: [canvas.x, canvas.y], onto }));
}

fn draw_drag_overlay(ui: &Ui, painter: &egui::Painter, state: &GraphState, layouts: &[NodeLayout], origin: Pos2, pointer: Option<Pos2>) {
    let view = state.view();
    for line in &state.snap_lines {
        let start = view.to_screen(origin, line[0]);
        let end = view.to_screen(origin, line[1]);
        painter.line_segment([start, end], Stroke::new(1.0, SNAP_LINE));
    }
    if let (Some(type_name), Some(point)) = (&state.toolbox_drag, pointer) {
        let label = Catalogue::get().node(type_name).map(|spec| spec.display_name.clone().unwrap_or_else(|| spec.name.clone())).unwrap_or_else(|| type_name.clone());
        let galley = painter.layout_no_wrap(label, theme::semibold(11.0), theme::LABEL_EMPHASIS);
        let tag = Rect::from_min_size(point + Vec2::new(12.0, 8.0), galley.size() + Vec2::new(12.0, 6.0));
        painter.rect_filled(tag, 3.0, theme::MENU_BACKGROUND);
        painter.rect_stroke(tag, 3.0, Stroke::new(1.0, theme::MENU_BORDER), StrokeKind::Inside);
        painter.galley(tag.min + Vec2::new(6.0, 3.0), galley, theme::LABEL_EMPHASIS);
    }
    match &state.drag {
        Some(Drag::Marquee { start, .. }) => {
            let end = pointer.unwrap_or(*start);
            let marquee = Rect::from_two_pos(*start, end);
            painter.rect_filled(marquee, 0.0, theme::SELECTION_FILL);
            painter.rect_stroke(marquee, 0.0, Stroke::new(1.0, theme::SELECTION_STROKE), StrokeKind::Inside);
        }
        Some(Drag::FromOutput { node, port, .. }) | Some(Drag::FromInput { node, port, .. }) => {
            let is_input = matches!(state.drag, Some(Drag::FromInput { .. }));
            let Some(layout) = layouts.iter().find(|layout| layout.id == *node) else { return };
            let Some(port_layout) = layout.port(port, is_input) else { return };
            let anchor = view.to_screen(origin, port_layout.anchor);
            let Some(end) = pointer else { return };
            // `ConnectionLine`: a dashed gradient line that crawls.
            let time = ui.input(|input| input.time) as f32;
            let offset = (time * 16.0) % 5.0;
            let points = [anchor, end];
            let mid = theme::CONNECTION_LINE_START.lerp_to_gamma(theme::CONNECTION_LINE_END, 0.5);
            let stroke = Stroke::new(1.5, mid);
            let dashes = Shape::dashed_line_with_offset(&points, stroke, &[2.0 * 1.5], &[3.0 * 1.5], offset);
            painter.extend(dashes);
            ui.ctx().request_repaint();
        }
        _ => {}
    }
}

/// The inline name editor (F2): an `EditablePopup`, closing as
/// [`crate::ui::widgets::popup_outcome`] says; a blank name is no name.
/// The name taken goes to [`GraphCommand::RenameNode`].
fn rename_overlay(ui: &mut Ui, editor: &Editor, state: &mut GraphState, layouts: &[NodeLayout], origin: Pos2, commands: &mut Vec<Command>) {
    let Some(id) = state.renaming else {
        return;
    };
    let Some(layout) = layouts.iter().find(|layout| layout.id == id) else {
        state.renaming = None;
        return;
    };
    if state.rename_text.is_empty()
        && let Some(node) = editor.document.node(id)
    {
        state.rename_text = node.name.clone();
    }
    let view = state.view();
    let top_left = view.to_screen(origin, layout.rect.min + Vec2::new(46.0, 10.0));
    let size = Vec2::new(150.0 * view.zoom.max(0.6), 22.0);
    let rect = Rect::from_min_size(top_left, size);
    let edit = egui::TextEdit::singleline(&mut state.rename_text).font(theme::semibold(13.0));
    let response = ui.put(rect, edit);
    response.request_focus();
    let Some(accepted) = crate::ui::widgets::popup_outcome(ui, &response) else {
        return;
    };
    let name = state.rename_text.trim().to_string();
    if accepted && !name.is_empty() {
        let rename = GraphCommand::RenameNode { node: id, name };
        commands.push(Command::Graph(rename));
    }
    state.renaming = None;
    state.rename_text.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shine_sweeps_left_to_right() {
        // Before it starts the band is off the left edge; half way it is
        // across the node; at the end off the right.
        let start = shine_stops(0.0);
        assert_eq!(start, [-0.65, -0.5, -0.49]);
        assert_eq!(shine_opacity(0.0, start), 0.0);
        let middle = shine_stops(0.25);
        assert!(shine_opacity(middle[1], middle) == SHINE_PEAK);
        assert!(shine_opacity(0.0, middle) == 0.0);
        let rising = (middle[0] + middle[1]) / 2.0;
        assert!((shine_opacity(rising, middle) - SHINE_PEAK / 2.0).abs() < 1e-5);
        let end = shine_stops(0.5);
        assert!((end[0] - 1.25).abs() < 1e-6 && (end[1] - 1.3).abs() < 1e-6 && (end[2] - 1.31).abs() < 1e-6);
        assert_eq!(shine_opacity(1.0, end), 0.0);
        let mesh = shine_mesh(Rect::from_min_size(Pos2::ZERO, Vec2::new(230.0, 80.0)), 4.0, Color32::WHITE, middle);
        assert!(mesh.vertices.len() >= 4 && mesh.indices.len().is_multiple_of(3));
    }

    #[test]
    fn snapping_follows_snap_element_to_grid() {
        assert_eq!(snap_to_unit(26012.0, 25.0), 26000.0);
        assert_eq!(snap_to_unit(26012.5, 25.0), 26000.0);
        assert_eq!(snap_to_unit(26013.0, 25.0), 26025.0);
        assert_eq!(snap_to_unit(-30.0, 25.0), -25.0);
    }

    #[test]
    fn items_snap_to_the_nearest_edges() {
        let other = Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(230.0, 80.0));
        let dragged = Rect::from_min_size(Pos2::new(250.0, 10.0), Vec2::new(230.0, 80.0));
        let (shift, lines) = snap_to_items(dragged, &[other]);
        // Left against the other's right (20 away), tops level (10 away).
        assert_eq!(shift, Vec2::new(-20.0, -10.0));
        assert_eq!(lines.len(), 2);
        let far = Rect::from_min_size(Pos2::new(600.0, 300.0), Vec2::new(230.0, 80.0));
        assert_eq!(snap_to_items(far, &[other]).0, Vec2::ZERO);
    }

    #[test]
    fn connections_let_go_on_ports() {
        let dragged = |port: &str, from_output: bool, collapsed: bool| DraggedConnection { node: 1, port: port.to_string(), from_output, collapsed };
        let out = dragged("Out", true, false);
        let connect = PortDrop::Connect { from: 1, from_port: "Out".to_string(), to: 2, to_port: "In".to_string() };
        assert_eq!(port_drop(&out, 2, "In", true), connect);
        let combine = PortDrop::CombineOutputs { dragged: 1, dragged_port: "Out".to_string(), other: 2, other_port: "Wear".to_string() };
        assert_eq!(port_drop(&out, 2, "Wear", false), combine);
        assert_eq!(port_drop(&out, 2, COLLAPSED_PORT, false), PortDrop::PortsMenu { node: 2 });
        assert_eq!(port_drop(&out, 1, "Wear", false), PortDrop::Nothing);
        // Backwards out of an input: an output feeds it, an input does not.
        let input = dragged("In", false, false);
        let backwards = PortDrop::Connect { from: 2, from_port: "Out".to_string(), to: 1, to_port: "In".to_string() };
        assert_eq!(port_drop(&input, 2, "Out", false), backwards);
        assert_eq!(port_drop(&input, 2, "In", true), PortDrop::Nothing);
        // Collapsed ports onto collapsed ports: both picked in the window.
        let collapsed = dragged("Input2", false, true);
        assert_eq!(port_drop(&collapsed, 2, COLLAPSED_PORT, true), PortDrop::CreateConnection { node: 2 });
        let onto_port = PortDrop::Connect { from: 2, from_port: "Out".to_string(), to: 1, to_port: "Input2".to_string() };
        assert_eq!(port_drop(&collapsed, 2, "Out", false), onto_port);
    }

    const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1200.0, 800.0));

    /// One frame of the graph with these events, the modifiers held.
    fn graph_frame(context: &egui::Context, editor: &mut Editor, state: &mut GraphState, events: Vec<egui::Event>, modifiers: egui::Modifiers) -> Vec<Command> {
        let mut held_then_events = vec![egui::Event::ModifiersChanged(modifiers)];
        held_then_events.extend(events);
        let input = egui::RawInput { screen_rect: Some(SCREEN), events: held_then_events, ..Default::default() };
        let mut commands = Vec::new();
        let output = context.run_ui(input, |ui| show(ui, SCREEN, editor, state, &mut commands));
        output.drop_without_applying_deltas();
        commands
    }

    /// A file dropped on the window.
    #[derive(Debug)]
    struct Dropped(PathBuf);

    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &std::path::Path {
            &self.0
        }

        fn bytes(&self) -> Result<Vec<u8>, String> {
            Ok(Vec::new())
        }
    }

    /// One frame of the graph in which `file` drops, the pointer moving to
    /// `at` first when the system says where it is.
    fn drop_frame(context: &egui::Context, editor: &mut Editor, state: &mut GraphState, file: &str, at: Option<Pos2>) -> Vec<Command> {
        let events = at.map(egui::Event::PointerMoved).into_iter().collect();
        let dropped: egui::DroppedFileHandle = std::sync::Arc::new(Dropped(PathBuf::from(file)));
        let input = egui::RawInput { screen_rect: Some(SCREEN), events, dropped_files: vec![dropped], ..Default::default() };
        let mut commands = Vec::new();
        let output = context.run_ui(input, |ui| show(ui, SCREEN, editor, state, &mut commands));
        output.drop_without_applying_deltas();
        commands
    }

    #[test]
    fn dropped_files_land_where_they_drop_on_the_graph() {
        let file = "/maps/height.png";
        let dropped_at = |state: &GraphState, point: Pos2| {
            let canvas = state.view().to_canvas(state.rect.min, point);
            let drop = GraphCommand::DropFiles { files: vec![PathBuf::from(file)], at: [canvas.x, canvas.y] };
            vec![Command::Graph(drop)]
        };
        let mut editor = Editor::new(Document::new());
        let mut state = GraphState::default();
        let context = themed_context();
        graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
        // On the graph: where the pointer is.
        let point = Pos2::new(300.0, 200.0);
        let commands = drop_frame(&context, &mut editor, &mut state, file, Some(point));
        assert_eq!(commands, dropped_at(&state, point));
        // Somewhere else: not the graph's.
        let elsewhere = Pos2::new(SCREEN.right() + 50.0, 100.0);
        assert!(drop_frame(&context, &mut editor, &mut state, file, Some(elsewhere)).is_empty());
        // Where the system can't say: the middle.
        let context = themed_context();
        graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
        let commands = drop_frame(&context, &mut editor, &mut state, file, None);
        assert_eq!(commands, dropped_at(&state, state.rect.center()));
    }

    /// A context with the editor's fonts, which take effect a frame after
    /// they are set.
    fn themed_context() -> egui::Context {
        let context = egui::Context::default();
        theme::apply(&context);
        let input = egui::RawInput { screen_rect: Some(SCREEN), ..Default::default() };
        let output = context.run_ui(input, |_| {});
        output.drop_without_applying_deltas();
        context
    }

    /// A press at `from`, a drag to `to` and the release there.
    fn drag(context: &egui::Context, editor: &mut Editor, state: &mut GraphState, from: Pos2, to: Pos2, modifiers: egui::Modifiers) -> Vec<Command> {
        let button = |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers };
        graph_frame(context, editor, state, vec![egui::Event::PointerMoved(from)], modifiers);
        graph_frame(context, editor, state, vec![button(from, true)], modifiers);
        let past_threshold = from + Vec2::new(12.0, 0.0);
        graph_frame(context, editor, state, vec![egui::Event::PointerMoved(past_threshold)], modifiers);
        graph_frame(context, editor, state, vec![egui::Event::PointerMoved(to)], modifiers);
        graph_frame(context, editor, state, vec![button(to, false)], modifiers)
    }

    /// The screen point of a port's dot.
    fn port_dot(editor: &Editor, state: &GraphState, node: i32, port: &str, is_input: bool) -> Pos2 {
        let data = editor.document.node(node).unwrap();
        let layout = layout_node(data, editor.user.options.default_node_size());
        let dot = layout.port(port, is_input).unwrap().dot.center();
        state.view().to_screen(state.rect.min, dot)
    }

    #[test]
    fn panning_is_an_undo_step_that_brings_the_view_back() {
        let mut document = Document::new();
        document.add_node("Mountain", [26100.0, 26100.0]).unwrap();
        let mut editor = Editor::new(document);
        let mut state = GraphState::default();
        let context = themed_context();
        graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
        let before = state.view();
        let middle = |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: PointerButton::Middle, pressed, modifiers: egui::Modifiers::NONE };
        let from = Pos2::new(600.0, 600.0);
        let to = Pos2::new(500.0, 450.0);
        graph_frame(&context, &mut editor, &mut state, vec![egui::Event::PointerMoved(from)], egui::Modifiers::NONE);
        graph_frame(&context, &mut editor, &mut state, vec![middle(from, true)], egui::Modifiers::NONE);
        graph_frame(&context, &mut editor, &mut state, vec![egui::Event::PointerMoved(from + Vec2::new(-12.0, 0.0))], egui::Modifiers::NONE);
        graph_frame(&context, &mut editor, &mut state, vec![egui::Event::PointerMoved(to)], egui::Modifiers::NONE);
        graph_frame(&context, &mut editor, &mut state, vec![middle(to, false)], egui::Modifiers::NONE);
        assert_ne!(state.view().location, before.location);
        assert_eq!(editor.document.undo_labels()[0], "Graph panned");
        // Undone, the view goes back to where the pan began.
        assert!(editor.document.undo());
        graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
        assert_eq!(state.view().location, before.location);

        // Off, panning is no edit.
        editor.user.options.set_value("RecordGraphMovementUndos", serde_json::json!(false));
        let labels = editor.document.undo_labels().len();
        graph_frame(&context, &mut editor, &mut state, vec![middle(from, true)], egui::Modifiers::NONE);
        graph_frame(&context, &mut editor, &mut state, vec![middle(from, false)], egui::Modifiers::NONE);
        assert_eq!(editor.document.undo_labels().len(), labels);
    }

    #[test]
    fn an_output_dragged_onto_another_output_combines_the_two() {
        let mut document = Document::new();
        let first = document.add_node("Mountain", [26100.0, 26100.0]).unwrap();
        let second = document.add_node("ErosionPro", [26100.0, 26400.0]).unwrap();
        let mut editor = Editor::new(document);
        let mut state = GraphState::default();
        let context = themed_context();
        graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
        let from = port_dot(&editor, &state, first, "Out", false);
        let to = port_dot(&editor, &state, second, "Wear", false);
        let commands = drag(&context, &mut editor, &mut state, from, to, egui::Modifiers::NONE);
        let combine = GraphCommand::CombineOutputs { dragged: first, dragged_port: "Out".to_string(), other: second, other_port: "Wear".to_string() };
        assert_eq!(commands, vec![Command::Graph(combine)]);
    }

    #[test]
    fn collapsed_ports_dragged_onto_collapsed_ports_open_create_connection() {
        let mut document = Document::new();
        let erosion = document.add_node("ErosionPro", [26100.0, 26100.0]).unwrap();
        let combine = document.add_node("Combine", [26500.0, 26300.0]).unwrap();
        document.add_input(combine);
        for node in [erosion, combine] {
            document.set_member(node, "NodeSize", serde_json::json!("Compact"), "Node size", false);
        }
        let mut editor = Editor::new(document);
        let mut state = GraphState::default();
        let context = themed_context();
        graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
        let from = port_dot(&editor, &state, erosion, COLLAPSED_PORT, false);
        let to = port_dot(&editor, &state, combine, COLLAPSED_PORT, true);
        let commands = drag(&context, &mut editor, &mut state, from, to, egui::Modifiers::NONE);
        assert!(commands.is_empty());
        assert_eq!(state.create_connection, Some(CreateConnection::new(erosion, combine, true)));
    }

    #[test]
    fn ctrl_shift_dragging_the_graph_pushes_what_lies_right() {
        let mut document = Document::new();
        let left = document.add_node("Mountain", [26100.0, 26100.0]).unwrap();
        let right = document.add_node("Mountain", [26700.0, 26100.0]).unwrap();
        let mut editor = Editor::new(document);
        let mut state = GraphState::default();
        let context = themed_context();
        graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
        let view = state.view();
        let origin = state.rect.min;
        // Between the two nodes, on empty graph.
        let from = view.to_screen(origin, Pos2::new(26450.0, 26140.0));
        let to = from + Vec2::new(150.0, 30.0);
        let pushing = egui::Modifiers { shift: true, ctrl: true, command: true, ..Default::default() };
        let commands = drag(&context, &mut editor, &mut state, from, to, pushing);
        assert!(commands.is_empty());
        let shift = view.to_canvas(origin, to).x - view.to_canvas(origin, from).x;
        assert_eq!(editor.document.node(left).unwrap().position, [26100.0, 26100.0]);
        let pushed = editor.document.node(right).unwrap().position;
        assert!((pushed[0] - (26700.0 + shift)).abs() < 1e-3 && pushed[1] == 26100.0, "{pushed:?}");
        assert_eq!(editor.document.undo_labels()[0], "Push");
        assert!(state.drag.is_none());
    }

    #[test]
    fn pushes_take_what_lies_right_of_the_pointer() {
        let mut document = Document::new();
        let left = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let right = document.add_node("Mountain", [500.0, 0.0]).unwrap();
        // A group right of the point takes its node and note along.
        let grouped = document.add_node("Blur", [800.0, 50.0]).unwrap();
        let note = document.add_note([820.0, 200.0], 0);
        let group = document.add_group(&[grouped, note], [780.0, 0.0], [300.0, 300.0], 0);
        // A node right of the point in a group left of it goes on its own.
        let straggler = document.add_node("Blur", [300.0, 400.0]).unwrap();
        let home = document.add_group(&[straggler], [100.0, 350.0], [300.0, 200.0], 0);
        let loose_note = document.add_note([400.0, 700.0], 0);
        // Other tabs stay as they are.
        let elsewhere = document.add_node("Mountain", [900.0, 0.0]).unwrap();
        document.project.terrain_mut().nodes.get_mut(&elsewhere).unwrap().set_member("GraphIndex", serde_json::json!(1));

        let pushed = elements_to_push(&document, 0, 200.0);
        let mut nodes: Vec<i32> = pushed.nodes.iter().map(|(id, _)| *id).collect();
        nodes.sort_unstable();
        let mut expected = vec![right, straggler];
        expected.sort_unstable();
        assert_eq!(nodes, expected);
        assert!(!nodes.contains(&left) && !nodes.contains(&grouped));
        assert_eq!(pushed.groups, vec![(group, [780.0, 0.0])]);
        assert!(!pushed.groups.iter().any(|(id, _)| *id == home));
        assert_eq!(pushed.notes, vec![(loose_note, [400.0, 700.0])]);
        assert!(elements_to_push(&document, 0, 2000.0).is_empty());
    }

    #[test]
    fn compact_nodes_collapse_their_extra_ports() {
        let mut document = Document::new();
        let combine = document.add_node("Combine", [0.0, 0.0]).unwrap();
        document.add_input(combine);
        let mut node = document.node(combine).unwrap().clone();
        node.set_member("NodeSize", serde_json::json!("Compact"));
        let layout = layout_node(&node, "Standard");
        let shown: Vec<&str> = layout.inputs.iter().filter(|port| !port.hidden).map(|port| port.name.as_str()).collect();
        assert_eq!(shown, vec!["In", COLLAPSED_PORT]);
        assert_eq!(layout.rect.height(), COMPACT_HEIGHT);
    }

    /// The graph amid the editor's other panels (the 3D view and its
    /// toolbar, the tab strip, the Data View, the status bar), whose
    /// widgets come before the graph's as the editor lays them out: one
    /// under the whole window stands for them.
    const GRAPH: Rect = Rect::from_min_max(Pos2::new(200.0, 300.0), Pos2::new(1100.0, 750.0));

    /// One frame of the graph at `GRAPH` with these events: the shapes
    /// painted, what of the pointer the panels around it took, and the
    /// graph's commands.
    fn frame_amid_panels(context: &egui::Context, editor: &mut Editor, state: &mut GraphState, events: Vec<egui::Event>) -> (Vec<egui::epaint::ClippedShape>, Response, Vec<Command>) {
        let input = egui::RawInput { screen_rect: Some(SCREEN), events, ..Default::default() };
        let mut commands = Vec::new();
        let mut panels = None;
        let mut output = context.run_ui(input, |ui| {
            panels = Some(ui.interact(SCREEN, egui::Id::new("panels"), Sense::click_and_drag()));
            show(ui, GRAPH, editor, state, &mut commands);
        });
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        (shapes, panels.unwrap(), commands)
    }

    #[test]
    fn elements_panned_past_the_edge_stay_inside_the_graph() {
        let mut editor = Editor::new(Document::new());
        let mut state = GraphState::default();
        let context = themed_context();
        egui_extras::install_image_loaders(&context);
        frame_amid_panels(&context, &mut editor, &mut state, Vec::new());

        // A node with its icon above the graph's top edge; a locked group
        // and an icon note with theirs left of its left edge.
        let view = state.view();
        let canvas_at = |screen: Pos2| {
            let point = view.to_canvas(GRAPH.min, screen);
            [point.x, point.y]
        };
        let node_corner = Pos2::new(GRAPH.left() + 300.0, GRAPH.top() - 40.0);
        let node = editor.document.add_node("Mountain", canvas_at(node_corner)).unwrap();
        let group_corner = Pos2::new(GRAPH.left() - 100.0, GRAPH.top() + 100.0);
        let group = editor.document.add_group(&[], canvas_at(group_corner), [300.0, 200.0], 0);
        editor.document.set_group_locked(group, true);
        let note_corner = Pos2::new(GRAPH.left() - 40.0, GRAPH.top() + 250.0);
        let note = editor.document.add_note(canvas_at(note_corner), 0);
        editor.document.toggle_note_type(note);

        // Nothing shows outside the graph.
        let (shapes, _, _) = frame_amid_panels(&context, &mut editor, &mut state, Vec::new());
        for clipped in &shapes {
            let bounds = clipped.shape.visual_bounding_rect();
            let shown = clipped.clip_rect.intersect(bounds);
            assert!(!shown.is_positive() || GRAPH.contains_rect(shown), "{shown:?} shows outside the graph");
        }

        // A click on the node's hidden part is the 3D view's.
        let hidden_part = node_corner + Vec2::new(100.0, 20.0);
        let button = |pressed: bool| egui::Event::PointerButton { pos: hidden_part, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame_amid_panels(&context, &mut editor, &mut state, vec![egui::Event::PointerMoved(hidden_part)]);
        frame_amid_panels(&context, &mut editor, &mut state, vec![button(true)]);
        let (_, panels, _) = frame_amid_panels(&context, &mut editor, &mut state, vec![button(false)]);
        assert!(panels.clicked());
        assert!(!editor.selection.contains(&node));
    }

    #[test]
    fn hidden_parts_leave_hover_clicks_and_drags_to_the_panels_around() {
        let mut editor = Editor::new(Document::new());
        let mut state = GraphState::default();
        let context = themed_context();
        frame_amid_panels(&context, &mut editor, &mut state, Vec::new());

        // A node across the top edge, one just right of the graph (inside
        // the 40 nodes are kept for), one with its input past the left
        // edge, a group across the left edge and a text note across the
        // bottom right corner, and a node well inside.
        let view = state.view();
        let canvas_at = |x: f32, y: f32| {
            let point = view.to_canvas(GRAPH.min, Pos2::new(x, y));
            [point.x, point.y]
        };
        let document = &mut editor.document;
        document.add_node("Mountain", canvas_at(500.0, 260.0)).unwrap();
        document.add_node("Mountain", canvas_at(1120.0, 450.0)).unwrap();
        let with_input = document.add_node("ErosionPro", canvas_at(170.0, 630.0)).unwrap();
        document.add_group(&[], canvas_at(100.0, 320.0), [300.0, 270.0], 0);
        let note = document.add_note(canvas_at(950.0, 650.0), 0);
        document.set_note_vector(note, "Size", [200.0, 120.0], false);
        let inside = document.add_node("Mountain", canvas_at(600.0, 450.0)).unwrap();
        frame_amid_panels(&context, &mut editor, &mut state, Vec::new());
        let undo_steps = editor.document.undo_labels().len();

        let hidden_input = port_dot(&editor, &state, with_input, "In", true);
        let hidden_parts = [
            ("the node across the top", Pos2::new(600.0, 280.0)),
            ("the node beside the graph", Pos2::new(1140.0, 470.0)),
            ("the input past the left edge", hidden_input),
            ("the group's header", Pos2::new(150.0, 330.0)),
            ("the group's menu button", Pos2::new(120.0, 338.0)),
            ("the group's resize thumb", Pos2::new(110.0, 580.0)),
            ("the note", Pos2::new(1050.0, 760.0)),
            ("the note's resize thumb", Pos2::new(1140.0, 760.0)),
        ];
        let button = |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        let mut commands = Vec::new();
        for (part, point) in hidden_parts {
            assert!(!GRAPH.contains(point), "{part} shows in the graph");
            let mut frame = |events: Vec<egui::Event>| {
                let (_, panels, graph_commands) = frame_amid_panels(&context, &mut editor, &mut state, events);
                commands.extend(graph_commands);
                panels
            };
            // Hovered, without the graph's tooltip; clicked; dragged.
            assert!(frame(vec![egui::Event::PointerMoved(point)]).hovered(), "{part} takes the hover");
            frame(vec![button(point, true)]);
            assert!(frame(vec![button(point, false)]).clicked(), "{part} takes the click");
            frame(vec![button(point, true)]);
            let away = point + Vec2::new(0.0, -30.0);
            assert!(frame(vec![egui::Event::PointerMoved(away)]).dragged(), "{part} takes the drag");
            frame(vec![button(away, false)]);
        }
        assert!(commands.is_empty(), "{commands:?}");
        assert!(editor.selection.is_empty() && state.selected_groups.is_empty() && state.selected_note.is_none());
        assert!(state.drag.is_none() && state.popup.is_none());
        assert_eq!(editor.document.undo_labels().len(), undo_steps);

        // A connection let go on the hidden input joins nothing.
        let from = port_dot(&editor, &state, inside, "Out", false);
        let mut frame = |events: Vec<egui::Event>| frame_amid_panels(&context, &mut editor, &mut state, events).2;
        frame(vec![egui::Event::PointerMoved(from)]);
        frame(vec![button(from, true)]);
        frame(vec![egui::Event::PointerMoved(from + Vec2::new(12.0, 0.0))]);
        frame(vec![egui::Event::PointerMoved(hidden_input)]);
        assert!(frame(vec![button(hidden_input, false)]).is_empty());
        assert!(state.drag.is_none() && state.popup.is_none());
    }
}
