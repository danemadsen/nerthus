//! The Terrain tab (`tabTerrain`): the lock and underlay buttons and the
//! filter menu over the tree (`Tree`) of the project's graphs, their
//! groups, nodes and notes (`RecreateTree`).

use std::collections::HashSet;

use egui::containers::menu::MenuConfig;
use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, UiKind, UiStackInfo, Vec2};
use nerthus_engine::{Catalogue, NodeData, NodeState};
use serde_json::Value;

use crate::document::{node_tab, ExportSettings};
use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::data_view::{toolbar, toolbar_divider, toolbar_row};
use crate::ui::graph_canvas::{node_color, node_context_menu, node_icon, GraphState, ZoomRequest};
use crate::ui::widgets::{icon_menu, menu_item, menu_separator};
use crate::user::IconState;

/// `DataView-Foreground` (every row's text) and `Bright` (bookmarked
/// nodes).
const MUTED_TEXT: Color32 = Color32::from_rgb(0xa0, 0xa0, 0xa0);
/// `DataView-Icon`: icons when `DataViewIcons` is Muted.
const MUTED_ICON: Color32 = Color32::from_rgb(0x50, 0x50, 0x50);
const BRIGHT_TEXT: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);
/// `TreeViewItemX`: the expander 8 in and 16 wide, `TreeView.DockPadding`
/// round the icon and name, a 14 icon with 8 after it, the header's
/// margin 0, 2.
const EXPANDER_LEFT: f32 = 8.0;
const EXPANDER_SIZE: f32 = 16.0;
const ROW_ICON: f32 = 14.0;
const ICON_GAP: f32 = 8.0;
const HEADER_MARGIN: f32 = 2.0;
/// `TVIGraph`'s padding (16, 4, 0, 4): where its children go.
const GRAPH_CHILD_INDENT: f32 = 16.0;
const GRAPH_CHILD_GAP: f32 = 4.0;
/// The tree's `ScrollViewer` (margin 5, padding 1).
const TREE_MARGIN: f32 = 6.0;
/// `Button-IconOnly` with `MinWidth="16"`: a 16 icon, padding 6.
const ICON_ONLY_BUTTON: f32 = 28.0;

/// `FilterCriteria`: what the tree and the graph show at full strength.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FilterCriteria {
    #[default]
    None,
    MarkedForExport,
    RequiresBaking,
    Errors,
    Heavy,
    Orphans,
    Bypassed,
    HasModifiers,
    Generators,
    Simulations,
    Colorizors,
    Exportable,
    Logic,
}

impl FilterCriteria {
    /// The name as Humanizer writes it (`EnumHumanizeExtensions.Humanize`).
    pub fn humanized(self) -> &'static str {
        match self {
            FilterCriteria::None => "None",
            FilterCriteria::MarkedForExport => "Marked for export",
            FilterCriteria::RequiresBaking => "Requires baking",
            FilterCriteria::Errors => "Errors",
            FilterCriteria::Heavy => "Heavy",
            FilterCriteria::Orphans => "Orphans",
            FilterCriteria::Bypassed => "Bypassed",
            FilterCriteria::HasModifiers => "Has modifiers",
            FilterCriteria::Generators => "Generators",
            FilterCriteria::Simulations => "Simulations",
            FilterCriteria::Colorizors => "Colorizors",
            FilterCriteria::Exportable => "Exportable",
            FilterCriteria::Logic => "Logic",
        }
    }
}

/// `miCriteriaMenu`'s entries after None: the criteria, header, icon and
/// accent (`BorderBrush`), with a separator before Generators.
const CRITERIA_ENTRIES: [(FilterCriteria, &str, &str, Option<&str>); 12] = [
    (FilterCriteria::MarkedForExport, "Marked for Export", "device-floppy", None),
    (FilterCriteria::RequiresBaking, "Requires Baking", "flame", None),
    (FilterCriteria::Errors, "Nodes with Errors", "alert-triangle", None),
    (FilterCriteria::Heavy, "Heavy Nodes", "clock", None),
    (FilterCriteria::Orphans, "Orphan Nodes", "unlink", None),
    (FilterCriteria::Bypassed, "Bypassed Nodes", "road-sign", None),
    (FilterCriteria::HasModifiers, "Has Modifiers", "modifiers", None),
    (FilterCriteria::Generators, "Generators", "point", Some("Primitive")),
    (FilterCriteria::Simulations, "Simulations", "point", Some("Simulate")),
    (FilterCriteria::Colorizors, "Colorizors", "point", Some("Colorize")),
    (FilterCriteria::Exportable, "Exportable", "point", Some("Output")),
    (FilterCriteria::Logic, "Logic", "point", Some("Utility")),
];

/// The tab's state between frames.
#[derive(Default)]
pub struct TerrainState {
    pub filter: FilterCriteria,
    /// Graphs folded shut (`Graph_{index}_Expanded` false), by index.
    folded_graphs: HashSet<usize>,
    /// Groups opened (a `TreeViewItem` starts shut), by id.
    open_groups: HashSet<i32>,
}

/// One row of the tree, as `TreeViewItemX` draws it.
struct Row {
    /// How far in the row starts (its parents' padding).
    indent: f32,
    /// The icon and its colour (`BorderBrush`); none for `IconState.Hide`.
    icon: Option<(String, Color32)>,
    text: String,
    text_color: Color32,
    tooltip: Option<String>,
    selected: bool,
    /// Rows with children: whether they show.
    expander: Option<bool>,
    /// Items (nodes and notes) show their marks on the right.
    marks: Marks,
}

/// `icons`: locked for preview, underlay, marked for export.
#[derive(Clone, Copy, Default)]
struct Marks {
    lock: bool,
    underlay: bool,
    savable: bool,
}

/// What a row was clicked for.
#[derive(Clone, Copy)]
enum RowTarget {
    Graph(usize),
    Group(i32),
    Node(i32),
    Note(i32),
}

/// A group (`Terrain.Groups`) as saved.
struct GroupInfo {
    id: i32,
    name: String,
    color: String,
    icon: String,
    children: Vec<i32>,
    graph_index: usize,
}

fn groups(editor: &Editor) -> Vec<GroupInfo> {
    let Some(Value::Object(saved)) = editor.document.terrain().extra.get("Groups") else {
        return Vec::new();
    };
    let mut list = Vec::new();
    for (key, group) in saved {
        let Ok(id) = key.parse::<i32>() else {
            continue;
        };
        let name = group.get("Name").and_then(Value::as_str).unwrap_or_default().to_string();
        let color = group.get("Color").and_then(Value::as_str).unwrap_or("Gray").to_string();
        let saved_icon = group.get("Icon").and_then(Value::as_str).unwrap_or_default();
        let icon = if saved_icon.is_empty() { "square-rounded".to_string() } else { saved_icon.to_string() };
        // `Children` as Newtonsoft writes a list (`$values`), or plain.
        let children_value = group.get("Children");
        let children_list = children_value.and_then(|children| children.get("$values").or(Some(children)));
        let children_items = children_list.and_then(Value::as_array);
        let children = children_items.map(|items| items.iter().filter_map(Value::as_i64).map(|id| id as i32).collect()).unwrap_or_default();
        let graph_index = group.get("GraphIndex").and_then(Value::as_u64).unwrap_or(0) as usize;
        list.push(GroupInfo { id, name, color, icon, children, graph_index });
    }
    list
}

/// A note's tree header: its Markdown without `#`, `*` and line breaks,
/// cut to 20 characters and `...`; the tooltip keeps the breaks.
fn note_header(markdown: &str) -> (String, String) {
    let tooltip: String = markdown.chars().filter(|character| !matches!(character, '#' | '*')).collect();
    let flat: String = tooltip.chars().filter(|character| !matches!(character, '\r' | '\n')).collect();
    let header = if flat.chars().count() > 20 {
        let cut: String = flat.chars().take(20).collect();
        format!("{cut}...")
    } else {
        flat
    };
    (header, tooltip)
}

fn note_position_x(fields: &serde_json::Map<String, Value>) -> f32 {
    let position = fields.get("Position");
    position.and_then(|position| position.get("X")).and_then(Value::as_f64).unwrap_or(0.0) as f32
}

/// The project's `State` member, as saved.
fn state_value<'a>(editor: &'a Editor, member: &str) -> Option<&'a Value> {
    let state = editor.document.project.assets[0].extra.get("State")?;
    state.get(member)
}

/// `TerrainState.NodeBookmarks`.
fn bookmarks(editor: &Editor) -> HashSet<i32> {
    let saved = state_value(editor, "NodeBookmarks");
    let list = saved.and_then(|value| value.get("$values").or(Some(value))).and_then(Value::as_array);
    list.map(|items| items.iter().filter_map(Value::as_i64).map(|id| id as i32).collect()).unwrap_or_default()
}

/// `TVINode.UpdateNodeCriteria`: whether a node meets the criteria.
/// `average_milliseconds` is every node's mean build time (`Heavy`).
fn matches(criteria: FilterCriteria, node: &NodeData, editor: &Editor, average_milliseconds: f32) -> bool {
    let spec = Catalogue::get().node(&node.type_name);
    let classification = spec.and_then(|spec| spec.classification.as_deref()).unwrap_or_default();
    match criteria {
        FilterCriteria::None => true,
        FilterCriteria::MarkedForExport => ExportSettings::of(node).is_some(),
        FilterCriteria::Generators => classification == "Generator",
        FilterCriteria::Simulations => spec.and_then(|spec| spec.toolbox.as_deref()) == Some("Simulate"),
        FilterCriteria::Colorizors => classification == "Colorizer",
        FilterCriteria::Exportable => node.ports.iter().filter(|port| port.is_output() && port.is_exporting).count() > 1,
        FilterCriteria::Logic => spec.and_then(|spec| spec.family.as_deref()) == Some("Routing"),
        FilterCriteria::Heavy => editor.status(node.id).milliseconds > average_milliseconds,
        FilterCriteria::Orphans => is_orphan(node, editor, classification),
        FilterCriteria::Errors => editor.status(node.id).state == NodeState::Error,
        FilterCriteria::RequiresBaking => spec.is_some_and(|spec| spec.has_flag("RequiresBaking")),
        FilterCriteria::Bypassed => node.base.get("StateOverride").and_then(Value::as_str) == Some("Bypassed"),
        FilterCriteria::HasModifiers => !node.modifiers.is_empty(),
    }
}

/// `FilterCriteria.Orphans`: nothing connected in or out, not an export
/// node, and not exporting.
fn is_orphan(node: &NodeData, editor: &Editor, classification: &str) -> bool {
    let nodes = &editor.document.terrain().nodes;
    let has_inputs = node.ports.iter().any(|port| port.is_input() && port.record.is_some());
    let feeds_others = nodes.values().any(|other| other.ports.iter().any(|port| port.record.as_ref().is_some_and(|record| record.from == node.id)));
    if has_inputs || feeds_others || classification == "Export" {
        return false;
    }
    !ExportSettings::of(node).is_some_and(|settings| settings.enabled)
}

fn average_build_milliseconds(editor: &Editor) -> f32 {
    let nodes = &editor.document.terrain().nodes;
    if nodes.is_empty() {
        return 0.0;
    }
    let total: f32 = nodes.keys().map(|id| editor.status(*id).milliseconds).sum();
    total / nodes.len() as f32
}

/// The nodes a filter leaves out, which the graph draws at half strength
/// (`NodeElement.Opacity` 0.5).
pub fn dimmed_nodes(editor: &Editor, criteria: FilterCriteria) -> HashSet<i32> {
    if criteria == FilterCriteria::None {
        return HashSet::new();
    }
    let average = average_build_milliseconds(editor);
    let nodes = editor.document.terrain().nodes.values();
    let unmatched = nodes.filter(|node| !matches(criteria, node, editor, average));
    unmatched.map(|node| node.id).collect()
}

/// The nodes a filter keeps (`MatchesCriteria`), for the Stats tab.
pub fn matching_nodes(editor: &Editor, criteria: FilterCriteria) -> Vec<i32> {
    let average = average_build_milliseconds(editor);
    let nodes = editor.document.terrain().nodes.values();
    let matched = nodes.filter(|node| matches(criteria, node, editor, average));
    matched.map(|node| node.id).collect()
}

pub fn show(ui: &mut Ui, area: Rect, editor: &mut Editor, state: &mut TerrainState, graph: &mut GraphState, commands: &mut Vec<Command>) {
    let (inner, page_top) = toolbar(ui, area);
    let mut left = toolbar_row(ui, inner, false);
    lock_button(&mut left, editor, graph, commands);
    underlay_button(&mut left, editor, graph, commands);
    let mut right = toolbar_row(ui, inner, true);
    criteria_menu(&mut right, state);
    toolbar_divider(&mut right, 8.0, 2.0);

    let page = Rect::from_min_max(Pos2::new(area.left(), page_top), area.max);
    let tree_area = page.shrink(TREE_MARGIN);
    let builder = egui::UiBuilder::new().max_rect(tree_area);
    let mut tree_ui = ui.new_child(builder);
    let mut scroll = egui::ScrollArea::vertical();
    scroll = scroll.id_salt("data-view-tree");
    scroll = scroll.auto_shrink([false, false]);
    scroll.show(&mut tree_ui, |ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        tree(ui, editor, state, graph, commands);
    });
}

/// `btnLocked`: `photo-sensor-3` in `Accent-Green`, at half strength
/// until a node's preview is locked; then it goes to that node.
fn lock_button(ui: &mut Ui, editor: &Editor, graph: &mut GraphState, commands: &mut Vec<Command>) {
    let locked = if editor.preview_locked { editor.preview_node } else { None };
    let locked_name = locked.and_then(|id| editor.document.node(id)).map(|node| node.name.clone());
    let tooltip = match &locked_name {
        Some(name) => format!("Go to {name} (Preview Locked)"),
        None => "Click to lock preview to current node".to_string(),
    };
    let response = toolbar_icon(ui, "photo-sensor-3", theme::ACCENT_GREEN, locked.is_some(), &tooltip);
    if !response.clicked() {
        return;
    }
    match locked {
        Some(id) => go_to_node(editor, graph, id, commands),
        None => {
            if editor.active_node().is_some() {
                commands.push(Command::LockPreview);
            }
        }
    }
}

/// `btnUnderlay`: `focus-centered` in `Accent-Magenta`, at half strength
/// until a node is the underlay; then it goes to that node.
fn underlay_button(ui: &mut Ui, editor: &Editor, graph: &mut GraphState, commands: &mut Vec<Command>) {
    let underlay = editor.document.underlay_node();
    let underlay_name = editor.document.node(underlay).map(|node| node.name.clone());
    let tooltip = match &underlay_name {
        Some(name) => format!("Go to {name} (Underlay / Height Reference)"),
        None => "Click to set current node as Underlay / Height Reference".to_string(),
    };
    let response = toolbar_icon(ui, "focus-centered", theme::ACCENT_MAGENTA, underlay_name.is_some(), &tooltip);
    if !response.clicked() {
        return;
    }
    if underlay_name.is_some() {
        go_to_node(editor, graph, underlay, commands);
    } else if let Some(node) = editor.active_node() {
        commands.push(Command::SetUnderlay(node));
    }
}

/// A `Button-IconOnly` (16 icon, padding 6) in its own colour at 0.5
/// until `active`; `Button-Hover` under the pointer.
fn toolbar_icon(ui: &mut Ui, icon: &str, color: Color32, active: bool, tooltip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(ICON_ONLY_BUTTON), Sense::click());
    let opacity = if active { 1.0 } else { 0.5 };
    if response.hovered() {
        ui.painter().rect_filled(rect, 2.0, theme::BUTTON_HOVER.gamma_multiply(opacity));
    }
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(icons::DEFAULT_SIZE));
    icons::paint(ui, icon_rect, icon, color.gamma_multiply(opacity));
    response.on_hover_text(tooltip)
}

/// `NodeElement.GoToLocation` and `Select`: the node's tab, centred.
fn go_to_node(editor: &Editor, graph: &mut GraphState, id: i32, commands: &mut Vec<Command>) {
    let Some(node) = editor.document.node(id) else {
        return;
    };
    graph.tab = node_tab(node);
    graph.zoom_request = Some(ZoomRequest::Center(id));
    commands.push(Command::SelectNode(id));
}

/// `miCriteriaMenu`: `filter`, or with a filter on `xmark-circle` in red.
fn criteria_menu(ui: &mut Ui, state: &mut TerrainState) {
    let filtering = state.filter != FilterCriteria::None;
    let (icon, tint) = if filtering { ("xmark-circle", Some(theme::ACCENT_RED)) } else { ("filter", None) };
    icon_menu(ui, icon, "Apply Filter", tint, |ui| {
        if criteria_item(ui, state.filter == FilterCriteria::None, None, None, "None") {
            state.filter = FilterCriteria::None;
        }
        menu_separator(ui);
        for (criteria, header, icon, category) in CRITERIA_ENTRIES {
            if criteria == FilterCriteria::Generators {
                menu_separator(ui);
            }
            let accent = category.map(theme::toolbox_color).unwrap_or(theme::ICON_DEFAULT);
            if criteria_item(ui, state.filter == criteria, Some(icon), Some(accent), header) {
                state.filter = criteria;
            }
        }
    });
}

/// A checkable entry: checked, the check (`Accent-Blue`) in place of its
/// icon. Returns whether it was clicked.
fn criteria_item(ui: &mut Ui, checked: bool, icon: Option<&str>, accent: Option<Color32>, header: &str) -> bool {
    let (shown_icon, shown_accent) = if checked { (Some("check"), Some(theme::ACCENT_BLUE)) } else { (icon, accent) };
    menu_item(ui, shown_icon, shown_accent, header, "").clicked()
}

/// `RecreateTree`: each graph tab with its groups (their nodes, then
/// notes, by position), then its other nodes by position, then its other
/// notes.
fn tree(ui: &mut Ui, editor: &Editor, state: &mut TerrainState, graph: &mut GraphState, commands: &mut Vec<Command>) {
    let average = average_build_milliseconds(editor);
    let icon_state = editor.user.options.data_view_icons();
    let bookmarked = bookmarks(editor);
    let underlay = editor.document.underlay_node();
    let all_groups = groups(editor);
    let notes = editor.document.notes();
    let active = editor.active_node();
    let mut clicked: Option<(RowTarget, bool)> = None;
    let mut context: Option<i32> = None;

    let tabs = editor.document.graph_tabs();
    // Hide Graph Tabs with a single tab leaves its row out (`Tag` "hidden":
    // the header row 0 high), its contents still in place.
    let graph_rows_hidden = editor.user.options.hide_tabs() && tabs.len() == 1;
    for tab in tabs {
        let mut placed: HashSet<i32> = HashSet::new();
        let tab_nodes: Vec<&NodeData> = editor.document.terrain().nodes.values().filter(|node| node_tab(node) == tab.index).collect();
        let tab_groups: Vec<&GroupInfo> = all_groups.iter().filter(|group| group.graph_index == tab.index).collect();
        let graph_matches = state.filter == FilterCriteria::None || tab_nodes.iter().any(|node| matches(state.filter, node, editor, average));
        if !graph_matches {
            continue;
        }

        let open = !state.folded_graphs.contains(&tab.index);
        let has_children = !tab_nodes.is_empty() || !tab_groups.is_empty() || notes.iter().any(|(_, fields)| note_tab(fields) == tab.index);
        let graph_row = Row {
            indent: 0.0,
            icon: Some(("home-simple".to_string(), theme::swatch_color(&tab.color))),
            text: tab.name.clone(),
            text_color: MUTED_TEXT,
            tooltip: Some(tab.name.clone()),
            selected: false,
            expander: has_children.then_some(open),
            marks: Marks::default(),
        };
        if !graph_rows_hidden {
            let response = row(ui, &graph_row, icon_state);
            if response.toggled {
                toggle(&mut state.folded_graphs, tab.index);
            }
            if response.clicked {
                clicked = Some((RowTarget::Graph(tab.index), false));
            }
        }
        if !open {
            continue;
        }
        ui.add_space(GRAPH_CHILD_GAP);

        for group in tab_groups {
            let members: Vec<&NodeData> = {
                let mut list: Vec<&NodeData> = group.children.iter().filter_map(|id| editor.document.node(*id)).collect();
                list.sort_by(|left, right| left.position[0].total_cmp(&right.position[0]));
                list
            };
            let group_matches = state.filter == FilterCriteria::None || members.iter().any(|node| matches(state.filter, node, editor, average));
            placed.insert(group.id);
            if !group_matches {
                for member in &members {
                    placed.insert(member.id);
                }
                continue;
            }
            let group_open = state.open_groups.contains(&group.id);
            let group_color = theme::swatch_color(&group.color);
            let group_top = ui.cursor().top();
            // `group` sits behind the header and its children.
            let group_frame = ui.painter().add(egui::Shape::Noop);
            let group_row = Row {
                indent: GRAPH_CHILD_INDENT,
                icon: Some((group.icon.clone(), group_color)),
                text: group.name.clone(),
                text_color: MUTED_TEXT,
                tooltip: None,
                selected: false,
                expander: Some(group_open),
                marks: Marks::default(),
            };
            let response = row(ui, &group_row, icon_state);
            if response.toggled {
                toggle(&mut state.open_groups, group.id);
            }
            if response.clicked {
                clicked = Some((RowTarget::Group(group.id), response.double_clicked));
            }
            for member in &members {
                placed.insert(member.id);
                if !group_open || !matches(state.filter, member, editor, average) {
                    continue;
                }
                let node_row = node_row(member, GRAPH_CHILD_INDENT, active, editor, &bookmarked, underlay);
                let response = row(ui, &node_row, icon_state);
                if response.clicked {
                    clicked = Some((RowTarget::Node(member.id), response.double_clicked));
                }
                if response.context {
                    context = Some(member.id);
                }
            }
            let mut group_notes: Vec<&(i32, &serde_json::Map<String, Value>)> = notes.iter().filter(|(id, _)| group.children.contains(id)).collect();
            group_notes.sort_by(|left, right| note_position_x(left.1).total_cmp(&note_position_x(right.1)));
            for (id, fields) in group_notes {
                placed.insert(*id);
                if !group_open {
                    continue;
                }
                let response = row(ui, &note_row(fields, GRAPH_CHILD_INDENT), icon_state);
                if response.clicked {
                    clicked = Some((RowTarget::Note(*id), response.double_clicked));
                }
            }
            // `group`: the frame around the group and its children (the
            // swatch at 0.33 inside, its outline, all at 0.5; margin 0, 2).
            let frame_top_left = Pos2::new(ui.max_rect().left() + GRAPH_CHILD_INDENT, group_top + 2.0);
            let frame_bottom_right = Pos2::new(ui.max_rect().right(), ui.cursor().top() - 2.0);
            let group_rect = Rect::from_min_max(frame_top_left, frame_bottom_right);
            // Muted or hidden icons mute the frame too.
            let (fill_color, outline_color) = if icon_state == IconState::Color { (group_color, group_color) } else { (MUTED_TEXT, MUTED_ICON) };
            let fill = fill_color.gamma_multiply(0.33 * 0.5);
            let outline = Stroke::new(1.0, outline_color.gamma_multiply(0.5));
            let frame_shape = egui::epaint::RectShape::new(group_rect, 3.0, fill, outline, StrokeKind::Inside);
            ui.painter().set(group_frame, frame_shape);
        }

        let mut loose: Vec<&NodeData> = tab_nodes.iter().copied().filter(|node| !placed.contains(&node.id)).collect();
        loose.sort_by(|left, right| left.position[0].total_cmp(&right.position[0]));
        for node in loose {
            if !matches(state.filter, node, editor, average) {
                continue;
            }
            let node_row = node_row(node, GRAPH_CHILD_INDENT, active, editor, &bookmarked, underlay);
            let response = row(ui, &node_row, icon_state);
            if response.clicked {
                clicked = Some((RowTarget::Node(node.id), response.double_clicked));
            }
            if response.context {
                context = Some(node.id);
            }
        }
        for (id, fields) in &notes {
            if placed.contains(id) || note_tab(fields) != tab.index {
                continue;
            }
            let response = row(ui, &note_row(fields, GRAPH_CHILD_INDENT), icon_state);
            if response.clicked {
                clicked = Some((RowTarget::Note(*id), response.double_clicked));
            }
        }
        ui.add_space(GRAPH_CHILD_GAP);
    }

    if let Some((target, double)) = clicked {
        row_clicked(editor, graph, target, double, commands);
    }
    // `OnMouseRightButtonUp`: selects the node and opens the graph's node
    // menu where the pointer is.
    if let Some(id) = context {
        commands.push(Command::SelectNode(id));
    }
    let open_command = context.map(|_| egui::SetOpenCommand::Bool(true));
    let info = UiStackInfo::new(UiKind::Menu).with_tag_value(MenuConfig::MENU_CONFIG_TAG, MenuConfig::default());
    let popup_id = ui.id().with("tree-node-menu");
    let mut popup = egui::Popup::new(popup_id, ui.ctx().clone(), egui::PopupAnchor::PointerFixed, ui.layer_id());
    popup = popup.kind(egui::PopupKind::Menu);
    popup = popup.layout(egui::Layout::top_down_justified(egui::Align::Min));
    popup = popup.info(info);
    popup = popup.open_memory(open_command);
    popup = popup.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    popup.show(|ui| node_context_menu(ui, editor, commands));
}

fn toggle<T: std::hash::Hash + Eq>(set: &mut HashSet<T>, key: T) {
    if !set.remove(&key) {
        set.insert(key);
    }
}

fn note_tab(fields: &serde_json::Map<String, Value>) -> usize {
    fields.get("GraphIndex").and_then(Value::as_u64).unwrap_or(0) as usize
}

/// `CreateNodeTreeItem`: the family icon in the node's colour, its name
/// (bright when bookmarked), and the lock, underlay and export marks.
fn node_row(node: &NodeData, indent: f32, active: Option<i32>, editor: &Editor, bookmarked: &HashSet<i32>, underlay: i32) -> Row {
    let text_color = if bookmarked.contains(&node.id) { BRIGHT_TEXT } else { MUTED_TEXT };
    let marks = Marks {
        lock: editor.preview_locked && editor.preview_node == Some(node.id),
        underlay: underlay == node.id,
        savable: node.base.get("SaveDefinition").is_some_and(|value| !value.is_null()),
    };
    Row {
        indent,
        icon: Some((node_icon(&node.type_name), node_color(node))),
        text: node.name.clone(),
        text_color,
        tooltip: None,
        selected: active == Some(node.id),
        expander: None,
        marks,
    }
}

/// A `TVINote`: `note` in the note's colour.
fn note_row(fields: &serde_json::Map<String, Value>, indent: f32) -> Row {
    let markdown = fields.get("Markdown").and_then(Value::as_str).unwrap_or_default();
    let (header, tooltip) = note_header(markdown);
    let color_name = fields.get("Color").and_then(Value::as_str).unwrap_or("Gray");
    Row {
        indent,
        icon: Some(("note".to_string(), theme::swatch_color(color_name))),
        text: header,
        text_color: MUTED_TEXT,
        tooltip: Some(tooltip),
        selected: false,
        expander: None,
        marks: Marks::default(),
    }
}

/// What happened to a row this frame.
struct RowResponse {
    clicked: bool,
    double_clicked: bool,
    context: bool,
    toggled: bool,
}

/// One `TreeViewItemX` header: the selection (`Accent-Transparent-Blue`
/// inset 1, corners 3: 0.4 chosen, 0.3 under the pointer, 0.8 both), the
/// expander, the icon, the name (12px, trimmed) and the marks.
fn row(ui: &mut Ui, row: &Row, icon_state: IconState) -> RowResponse {
    let line_height = ui.painter().layout_no_wrap("Ag".to_string(), theme::interface(12.0), row.text_color).size().y;
    let content_height = (line_height + 2.0 * HEADER_MARGIN).max(ROW_ICON);
    let [dock_across, dock_down] = theme::sizes(ui.ctx()).tree_padding;
    let height = content_height + 2.0 * dock_down;
    let width = ui.available_width();
    let (full, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let rect = Rect::from_min_max(Pos2::new(full.left() + row.indent, full.top()), full.max);
    let hovered = response.hovered();
    let highlight = match (row.selected, hovered) {
        (true, true) => Some(0.8),
        (true, false) => Some(0.4),
        (false, true) => Some(0.3),
        (false, false) => None,
    };
    if let Some(opacity) = highlight {
        ui.painter().rect_filled(rect.shrink(1.0), 3.0, theme::ACCENT_TRANSPARENT_BLUE.gamma_multiply(opacity));
    }

    let mut toggled = false;
    if let Some(open) = row.expander {
        let expander_rect = Rect::from_min_size(Pos2::new(rect.left() + EXPANDER_LEFT, rect.center().y - EXPANDER_SIZE / 2.0), Vec2::splat(EXPANDER_SIZE));
        let expander = ui.interact(expander_rect, response.id.with("expander"), Sense::click());
        let chevron_color = if expander.hovered() { theme::LABEL_EMPHASIS } else { theme::LABEL_DEFAULT };
        let chevron = if open { "chevron-down" } else { "chevron-right" };
        icons::paint(ui, expander_rect, chevron, chevron_color);
        toggled = expander.clicked() || response.double_clicked();
    }

    // `IconState`: the icon in its colour, in `DataView-Icon`, or left
    // out (its place kept).
    let mut x = rect.left() + EXPANDER_LEFT + EXPANDER_SIZE + dock_across;
    if let Some((icon, color)) = &row.icon {
        let shown_color = match icon_state {
            IconState::Color => Some(*color),
            IconState::Muted => Some(MUTED_ICON),
            IconState::Hide => None,
        };
        let icon_rect = Rect::from_min_size(Pos2::new(x, rect.center().y - ROW_ICON / 2.0), Vec2::splat(ROW_ICON));
        if let Some(shown_color) = shown_color {
            icons::paint(ui, icon_rect, icon, shown_color);
        }
        x += ROW_ICON + ICON_GAP;
    }
    let marks_width = paint_marks(ui, rect, row.marks);
    let text_color = if row.selected { theme::LABEL_EMPHASIS } else { row.text_color };
    let available = (rect.right() - dock_across - marks_width - x).max(0.0);
    let galley = trimmed_galley(ui, &row.text, text_color, available);
    let text_position = Pos2::new(x, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_position, galley, text_color);

    let response = match &row.tooltip {
        Some(tooltip) if !tooltip.is_empty() => response.on_hover_text(tooltip),
        _ => response,
    };
    RowResponse {
        clicked: response.clicked() && !toggled,
        double_clicked: response.double_clicked(),
        context: response.secondary_clicked(),
        toggled,
    }
}

/// `icons` (margin 0, 0, 8, 0): locked (`point` filled, 16), underlay and
/// export (`point`, 14), each with a margin of 2. Returns their width.
fn paint_marks(ui: &Ui, rect: Rect, marks: Marks) -> f32 {
    let shown = [
        (marks.lock, 16.0, theme::ACCENT_MUTED_GREEN, true),
        (marks.underlay, 14.0, theme::ACCENT_MUTED_MAGENTA, false),
        (marks.savable, 14.0, theme::ACCENT_MUTED_PRIMARY, false),
    ];
    let total: f32 = shown.iter().filter(|(on, ..)| *on).map(|(_, size, ..)| size + 4.0).sum();
    if total == 0.0 {
        return 0.0;
    }
    let mut x = rect.right() - 8.0 - total;
    for (on, size, color, filled) in shown {
        if !on {
            continue;
        }
        let icon_rect = Rect::from_min_size(Pos2::new(x + 2.0, rect.center().y - size / 2.0), Vec2::splat(size));
        if filled {
            icons::paint_filled(ui, icon_rect, "point", color);
        } else {
            icons::paint(ui, icon_rect, "point", color);
        }
        x += size + 4.0;
    }
    total + 8.0
}

/// The name cut with an ellipsis to fit (`TextTrimming="CharacterEllipsis"`).
fn trimmed_galley(ui: &Ui, text: &str, color: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(text.to_string(), egui::TextFormat::simple(theme::interface(12.0), color));
    job.wrap.max_width = width;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    ui.painter().layout_job(job)
}

/// `OnMouseLeftButtonUp` and `OnMouseDoubleClick` of the tree's items.
fn row_clicked(editor: &Editor, graph: &mut GraphState, target: RowTarget, double: bool, commands: &mut Vec<Command>) {
    match target {
        // `TVIGraph`: `SelectGraphTab`.
        RowTarget::Graph(index) => graph.tab = index,
        // `TVINode`: selects the node (and shows it); a double click goes
        // to it.
        RowTarget::Node(id) => {
            if double {
                go_to_node(editor, graph, id, commands);
            } else {
                commands.push(Command::SelectNode(id));
            }
        }
        // `TVINote`: selects the note; a double click goes to it.
        RowTarget::Note(id) => {
            let tab = editor.document.notes().iter().find(|(note, _)| *note == id).map(|(_, fields)| note_tab(fields));
            if let Some(tab) = tab {
                graph.tab = tab;
            }
            graph.selected_note = Some(id);
            if double {
                graph.zoom_request = Some(ZoomRequest::CenterNote(id));
            }
        }
        // `TVIGroup`: selects the group; a double click goes to it.
        RowTarget::Group(id) => {
            let select = crate::ui::graph_commands::GraphCommand::SelectGroup { id, go_to: double };
            commands.push(Command::Graph(select));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    #[test]
    fn note_headers_drop_markup_and_cut_at_twenty() {
        let (header, tooltip) = note_header("# Title\n**Bold** words");
        assert_eq!(header, " TitleBold words");
        assert_eq!(tooltip, " Title\nBold words");
        let (long, _) = note_header("We mix three CLUTer outputs with Mixer");
        assert_eq!(long, "We mix three CLUTer ...");
    }

    #[test]
    fn filters_match_like_tvinode() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let erosion = document.add_node("ErosionPro", [300.0, 0.0]).unwrap();
        let lonely = document.add_node("Perlin", [0.0, 300.0]).unwrap();
        document.connect(mountain, "Out", erosion, "In");
        let editor = Editor::new(document);

        let orphans = matching_nodes(&editor, FilterCriteria::Orphans);
        assert_eq!(orphans, vec![lonely]);

        let dimmed = dimmed_nodes(&editor, FilterCriteria::Generators);
        assert!(dimmed.contains(&erosion));
        assert!(!dimmed.contains(&mountain) && !dimmed.contains(&lonely));
        assert!(dimmed_nodes(&editor, FilterCriteria::None).is_empty());
    }
}
