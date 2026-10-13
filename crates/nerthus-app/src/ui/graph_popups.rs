//! The menus the graph opens where they are needed: a connection's menu
//! (`_ctxConnection`), a collapsed port's ports (`ShowPortsMenu`), the
//! quick navigation (comma), Manage Portals (P), Insert chokepoint at…,
//! a collapsed group's portals, the Lazy Menu (F1) and the node search;
//! and the colour and icon grids the group, note and tab menus share.

use egui::{Align, Color32, Id, Layout, LayerId, Order, Popup, PopupAnchor, PopupCloseBehavior, PopupKind, Pos2, Rect, Sense, Ui, Vec2};
use nerthus_engine::humanize::humanize_title;
use nerthus_engine::Catalogue;

use crate::document::{grid_outputs, PortalState, SWATCHES};
use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::graph_canvas::GraphState;
use crate::ui::graph_commands::GraphCommand;
use crate::ui::graph_search::SearchPopup;
use crate::ui::widgets::{menu_caption, menu_item, menu_item_enabled, menu_separator, menu_style, submenu};

/// A menu's least width (`Default-Menu-Item`'s menus).
const MENU_WIDTH: f32 = 190.0;
/// Manage Portals' columns (`Width="157"`).
const PORTALS_COLUMN: f32 = 157.0;

/// What is open over the graph.
#[derive(Clone, Debug)]
pub enum GraphPopup {
    Search(SearchPopup),
    /// A connection right-clicked.
    Connection { to: i32, to_port: String, at: Pos2 },
    /// A dragged connection let go on a compact node's collapsed port.
    Ports { from: i32, from_port: String, from_output: bool, node: i32, at: Pos2 },
    /// `ShowQuickNavigationMenu`, right of the node.
    Navigation { node: i32, at: Pos2 },
    /// `ShowPortalsMenu` at the node's corner.
    Portals { node: i32, at: Pos2 },
    /// `ShowOutPortsMenu`: the node's outputs to put a chokepoint after.
    ChokepointAt { node: i32, at: Pos2 },
    /// `ShowGroupPortalMenu`: a connection let go on a collapsed group.
    GroupPortals { group: i32, to: i32, to_port: String, at: Pos2 },
    /// `ctxLazyBoy` at the pointer.
    Lazy { at: Pos2 },
}

impl GraphPopup {
    fn anchor(&self) -> Pos2 {
        match self {
            Self::Search(search) => search.at,
            Self::Connection { at, .. } | Self::Ports { at, .. } | Self::Navigation { at, .. } | Self::Portals { at, .. } | Self::ChokepointAt { at, .. } | Self::GroupPortals { at, .. } | Self::Lazy { at } => *at,
        }
    }

    /// Each kind of menu has its own popup, so one opening as another
    /// closes is new.
    fn key(&self) -> &'static str {
        match self {
            Self::Search(_) => "search",
            Self::Connection { .. } => "connection",
            Self::Ports { .. } => "ports",
            Self::Navigation { .. } => "navigation",
            Self::Portals { .. } => "portals",
            Self::ChokepointAt { .. } => "chokepoint",
            Self::GroupPortals { .. } => "group-portals",
            Self::Lazy { .. } => "lazy",
        }
    }
}

/// Shows what is open; it stays open until it closes itself, something
/// is chosen or a click lands elsewhere.
pub fn show(ui: &mut Ui, editor: &mut Editor, state: &mut GraphState, commands: &mut Vec<Command>) {
    let Some(mut popup) = state.popup.take() else {
        return;
    };
    let open = match &mut popup {
        GraphPopup::Search(search) => crate::ui::graph_search::show(ui, editor, search, commands),
        other => menu_popup(ui, editor, state, other, commands),
    };
    // A choice may have opened another popup already.
    if open && state.popup.is_none() {
        state.popup = Some(popup);
    }
}

fn menu_popup(ui: &Ui, editor: &Editor, state: &GraphState, popup: &GraphPopup, commands: &mut Vec<Command>) -> bool {
    let id = Id::new(("graph-popup", popup.key()));
    let layer = LayerId::new(Order::Foreground, id);
    let anchor = PopupAnchor::Position(popup.anchor());
    // A new area takes the room it is given; menus start at their least
    // width (Manage Portals: two 157 wide columns) and grow to fit.
    let width = if matches!(popup, GraphPopup::Portals { .. }) { 2.0 * PORTALS_COLUMN } else { MENU_WIDTH };
    let mut open = true;
    let mut menu = Popup::new(id, ui.ctx().clone(), anchor, layer);
    menu = menu.kind(PopupKind::Menu);
    menu = menu.width(width);
    menu = menu.layout(Layout::top_down_justified(Align::Min));
    menu = menu.close_behavior(PopupCloseBehavior::CloseOnClickOutside);
    menu = menu.open_bool(&mut open);
    menu.show(|ui| {
        menu_style(ui);
        match popup {
            GraphPopup::Connection { to, to_port, .. } => connection_menu(ui, editor, state, *to, to_port, commands),
            GraphPopup::Ports { from, from_port, from_output, node, .. } => ports_menu(ui, editor, *from, from_port, *from_output, *node, commands),
            GraphPopup::Navigation { node, .. } => navigation_menu(ui, editor, *node, commands),
            GraphPopup::Portals { node, .. } => portals_menu(ui, editor, *node, commands),
            GraphPopup::ChokepointAt { node, .. } => chokepoint_menu(ui, editor, *node, commands),
            GraphPopup::GroupPortals { group, to, to_port, .. } => group_portals_menu(ui, editor, *group, *to, to_port, commands),
            GraphPopup::Lazy { .. } => lazy_menu(ui, editor, state, commands),
            GraphPopup::Search(_) => {}
        }
    });
    open
}

/// `_ctxConnection`: Convert to Portal, Insert node (one element chosen
/// at most), Insert chokepoint, Delete Connection.
fn connection_menu(ui: &mut Ui, editor: &Editor, state: &GraphState, to: i32, to_port: &str, commands: &mut Vec<Command>) {
    if menu_item(ui, None, None, "Convert to Portal", "").clicked() {
        commands.push(Command::Graph(GraphCommand::ConvertConnectionsToPortals));
    }
    let chosen = editor.selection.len() + state.selected_connections.len() + state.selected_groups.len() + usize::from(state.selected_note.is_some());
    let insert = GraphCommand::InsertNodeSearch { to, to_port: to_port.to_string() };
    if menu_item_enabled(ui, None, None, "Insert node", "", chosen <= 1).clicked() {
        commands.push(Command::Graph(insert));
    }
    if menu_item(ui, None, None, "Insert chokepoint", "").clicked() {
        let chokepoint = GraphCommand::InsertChokepoint { to, to_port: to_port.to_string() };
        commands.push(Command::Graph(chokepoint));
    }
    menu_separator(ui);
    if menu_item(ui, Some("square-rounded-x"), Some(theme::ACCENT_RED), "Delete Connection", "Del").clicked() {
        commands.push(Command::Delete);
    }
}

/// A port row of a ports menu (`CreatePortMenuItem`): its name, and the
/// green `port-connect` when it is connected.
fn port_row(ui: &mut Ui, label: &str, connected: bool) -> bool {
    let (icon, accent) = if connected { (Some("port-connect"), Some(theme::ACCENT_MUTED_GREEN)) } else { (None, None) };
    menu_item(ui, icon, accent, label, "").clicked()
}

fn display_label(port: &nerthus_engine::PortData) -> String {
    let display = port.extra.get("DisplayName").and_then(serde_json::Value::as_str);
    display.unwrap_or(&port.name).to_string()
}

/// `ShowPortsMenu(node, hidePrimary: true)`: the node's name, its ports
/// on the side the connection reaches (the primary left out), then Add
/// as Mask and Add new port when they apply.
fn ports_menu(ui: &mut Ui, editor: &Editor, from: i32, from_port: &str, from_output: bool, node: i32, commands: &mut Vec<Command>) {
    let Some(data) = editor.document.node(node) else {
        return;
    };
    menu_caption(ui, &data.name);
    menu_separator(ui);
    let ports: Vec<&nerthus_engine::PortData> = if from_output {
        data.ports.iter().filter(|port| port.is_input() && !port.is_primary()).collect()
    } else {
        data.ports.iter().filter(|port| port.is_output() && !port.is_primary()).collect()
    };
    for port in ports {
        let connected = editor.document.port_connected(node, port);
        if !port_row(ui, &display_label(port), connected) {
            continue;
        }
        let command = if from_output {
            GraphCommand::ConnectPorts { from, from_port: from_port.to_string(), to: node, to_port: port.name.clone() }
        } else {
            GraphCommand::ConnectPorts { from: node, from_port: port.name.clone(), to: from, to_port: from_port.to_string() }
        };
        commands.push(Command::Graph(command));
    }
    if !from_output {
        return;
    }
    let spec = Catalogue::get().node(&data.type_name);
    let maskable = spec.is_some_and(|spec| spec.is_maskable()) && data.port("Mask").is_none();
    if maskable {
        menu_separator(ui);
        if menu_item(ui, Some("mask"), None, "Add as Mask", "").clicked() {
            commands.push(Command::Graph(GraphCommand::AddMaskAndConnect { from, from_port: from_port.to_string(), to: node }));
        }
    }
    let mut trial = data.clone();
    let can_add = nerthus_engine::editing::can_add_ports(&data.type_name) && nerthus_engine::editing::add_new_port(&mut trial).is_some();
    if can_add {
        menu_separator(ui);
        if menu_item(ui, Some("copy-plus"), None, "Add new port", "").clicked() {
            commands.push(Command::Graph(GraphCommand::AddPortAndConnect { from, from_port: from_port.to_string(), to: node }));
        }
    }
}

/// `ShowQuickNavigationMenu`: "Navigate to", then what feeds each input
/// (`Port | Node`, an arrow left) and what each output feeds (an arrow
/// right); "Node is not connected" when nothing does.
fn navigation_menu(ui: &mut Ui, editor: &Editor, node: i32, commands: &mut Vec<Command>) {
    let Some(data) = editor.document.node(node) else {
        return;
    };
    let mut entries: Vec<(String, i32, bool)> = Vec::new();
    for port in data.ports.iter().filter(|port| port.is_input()) {
        let Some(record) = &port.record else { continue };
        let Some(source) = editor.document.node(record.from) else { continue };
        entries.push((format!("{} | {}", record.from_port, source.name), source.id, true));
    }
    for port in data.ports.iter().filter(|port| port.is_output()) {
        for (target, input, output) in editor.document.outgoing(node) {
            if output != port.name {
                continue;
            }
            let Some(target_node) = editor.document.node(target) else { continue };
            entries.push((format!("{input} | {}", target_node.name), target, false));
        }
    }
    if entries.is_empty() {
        menu_caption(ui, "Node is not connected");
        return;
    }
    menu_caption(ui, "Navigate to");
    for (label, target, upstream) in entries {
        let icon = if upstream { "arrow-left-rhombus" } else { "arrow-right-rhombus" };
        if menu_item(ui, Some(icon), Some(theme::ACCENT_MUTED_PURPLE), &label, "").clicked() {
            commands.push(Command::Graph(GraphCommand::Navigate(target)));
        }
    }
}

/// `ShowPortalsMenu`: the inputs to connect to a portal (with Disconnect
/// beside one that consumes) on the left; the outputs to make portals of
/// and the portals to remove on the right. Shift makes a portal through
/// a chokepoint.
fn portals_menu(ui: &mut Ui, editor: &Editor, node: i32, commands: &mut Vec<Command>) {
    let Some(data) = editor.document.node(node) else {
        return;
    };
    let shift = ui.input(|input| input.modifiers.shift);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(PORTALS_COLUMN);
            menu_style(ui);
            ui.set_min_width(PORTALS_COLUMN);
            let inputs: Vec<&nerthus_engine::PortData> = data.ports.iter().filter(|port| port.is_input()).collect();
            if !inputs.is_empty() {
                menu_caption(ui, "Connect to Portal");
            }
            for port in inputs {
                let consuming = PortalState::of(port) == PortalState::Consuming;
                let accent = if consuming { theme::ACCENT_MUTED_GREEN } else { theme::ACCENT_MUTED_GRAY };
                if !consuming {
                    if menu_item(ui, Some("port-connect"), Some(accent), &port.name, "").clicked() {
                        commands.push(Command::Graph(GraphCommand::PortalSearch { node, port: port.name.clone() }));
                    }
                    continue;
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    ui.set_min_width(PORTALS_COLUMN);
                    let width = ui.available_width() - 32.0;
                    let row = ui.allocate_ui(Vec2::new(width, 23.0), |ui| {
                        ui.set_min_width(width);
                        menu_item(ui, Some("port-connect"), Some(accent), &port.name, "").clicked()
                    });
                    if row.inner {
                        commands.push(Command::Graph(GraphCommand::PortalSearch { node, port: port.name.clone() }));
                    }
                    let disconnect = disconnect_button(ui).on_hover_text("Disconnect");
                    if disconnect.clicked() {
                        commands.push(Command::Graph(GraphCommand::DisconnectPortalInput { node, port: port.name.clone() }));
                        ui.close();
                    }
                });
            }
        });
        ui.vertical(|ui| {
            ui.set_width(PORTALS_COLUMN);
            menu_style(ui);
            ui.set_min_width(PORTALS_COLUMN);
            let outputs: Vec<&nerthus_engine::PortData> = data.ports.iter().filter(|port| port.is_output()).collect();
            let plain: Vec<&&nerthus_engine::PortData> = outputs.iter().filter(|port| PortalState::of(port) != PortalState::Publishing).collect();
            let publishing: Vec<&&nerthus_engine::PortData> = outputs.iter().filter(|port| PortalState::of(port) == PortalState::Publishing).collect();
            if !plain.is_empty() {
                menu_caption(ui, "Convert to Portal...");
            }
            for port in &plain {
                if menu_item(ui, Some("plus"), Some(theme::ACCENT_MUTED_GREEN), &port.name, "").clicked() {
                    let publish = GraphCommand::PublishPort { node, port: port.name.clone(), through_chokepoint: shift };
                    commands.push(Command::Graph(publish));
                }
            }
            if publishing.is_empty() {
                return;
            }
            if !plain.is_empty() {
                menu_separator(ui);
            }
            menu_caption(ui, "Remove Portal");
            for port in &publishing {
                if menu_item(ui, Some("square-rounded-x"), Some(theme::ACCENT_MUTED_RED), &port.name, "").clicked() {
                    commands.push(Command::Graph(GraphCommand::RemovePortal { node, port: port.name.clone() }));
                }
            }
        });
    });
}

/// The 32 wide `square-rounded-x` beside a consuming input.
fn disconnect_button(ui: &mut Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(32.0, 23.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect.shrink(1.0), 4.0, theme::MENU_BACKGROUND_HOVER);
    }
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(icons::DEFAULT_SIZE));
    icons::paint(ui, icon_rect, "square-rounded-x", theme::ACCENT_MUTED_RED);
    response
}

/// `ShowOutPortsMenu`: "Insert chokepoint at...", then each output.
fn chokepoint_menu(ui: &mut Ui, editor: &Editor, node: i32, commands: &mut Vec<Command>) {
    let Some(data) = editor.document.node(node) else {
        return;
    };
    menu_caption(ui, "Insert chokepoint at...");
    menu_separator(ui);
    for port in grid_outputs(data) {
        if port_row(ui, &display_label(port), is_fed(editor, node, &port.name)) {
            commands.push(Command::Graph(GraphCommand::InsertChokepointAt { node, port: port.name.clone() }));
        }
    }
}

/// Whether an output feeds anything (`Port.IsConnected`).
fn is_fed(editor: &Editor, node: i32, output: &str) -> bool {
    let outgoing = editor.document.outgoing(node);
    outgoing.iter().any(|(_, _, port)| port == output)
}

/// `ShowGroupPortalMenu`: the portals the collapsed group's nodes
/// publish (`Node : Port`), to connect the dragged input to.
fn group_portals_menu(ui: &mut Ui, editor: &Editor, group: i32, to: i32, to_port: &str, commands: &mut Vec<Command>) {
    let Some(data) = editor.document.group(group) else {
        return;
    };
    let mut portals = Vec::new();
    for child in &data.children {
        let Some(node) = editor.document.node(*child) else { continue };
        for port in node.ports.iter().filter(|port| port.is_output() && PortalState::of(port) == PortalState::Publishing) {
            portals.push((node.id, node.name.clone(), port.name.clone()));
        }
    }
    if portals.is_empty() {
        menu_caption(ui, "No published portals.");
        return;
    }
    menu_caption(ui, "Published portals.");
    for (node, name, port) in portals {
        if menu_item(ui, None, None, &format!("{name} : {port}"), "").clicked() {
            let connect = GraphCommand::ConnectToPortal { from: node, from_port: port, to, to_port: to_port.to_string() };
            commands.push(Command::Graph(connect));
        }
    }
}

/// A Lazy Menu entry (`CreateLazyMenuItem`): its icon in its colour, faint
/// at level 1 (`BrushSet.Transparent`), hidden at 0.
struct LazyEntry {
    text: &'static str,
    icon: &'static str,
    tag: &'static str,
    color: Color32,
    option: &'static str,
}

const LAZY_ORANGE: Color32 = Color32::from_rgb(0xdc, 0x70, 0x4a);

fn lazy_entries() -> [LazyEntry; 13] {
    let entry = |text, icon, tag, color, option| LazyEntry { text, icon, tag, color, option };
    [
        entry("2D Viewport", "viewport2D", "2d", LAZY_ORANGE, "LazyViewport"),
        entry("Lighting Options", "Brightness-Slider-Horizontal", "lighting", LAZY_ORANGE, "LazyLighting"),
        entry("Graph", "graph", "graph", theme::ACCENT_BLUE, "LazyGraph"),
        entry("Properties", "properties", "prop", theme::ACCENT_BLUE, "LazyProperties"),
        entry("Modifiers", "modifiers", "modifiers", theme::ACCENT_BLUE, "LazyModifiers"),
        entry("Node Tree", "dataview-tree", "datatree", theme::ACCENT_GREEN, "LazyTree"),
        entry("Build View", "Pattern", "databuild", theme::ACCENT_GREEN, "LazyBuild"),
        entry("Variables View", "variable", "variables", theme::ACCENT_GREEN, "LazyVariables"),
        entry("Build Settings", "build-settings", "buildsettings", theme::ACCENT_GRAY, "LazySettings"),
        entry("1 Visualize Data (Render Off)", "render-0", "render-raw", theme::ACCENT_YELLOW, "LazySettings"),
        entry("2 Clay Render", "render-1", "render-clay", theme::ACCENT_YELLOW, "LazySettings"),
        entry("3 Realistic Renderer", "lens", "render-realistic", theme::ACCENT_YELLOW, "LazySettings"),
        entry("Toggle Skybox", "sunset-2", "toggle-skybox", theme::ACCENT_YELLOW, "LazySettings"),
    ]
}

/// `CreateLazyBoyMenu`: Bookmarks, then the areas, views and renderers in
/// their bands, a band's separator only when something of it shows.
fn lazy_menu(ui: &mut Ui, editor: &Editor, state: &GraphState, commands: &mut Vec<Command>) {
    submenu(ui, None, None, "Bookmarks", |ui| {
        bookmark_items(ui, editor, state, commands);
    });
    menu_separator(ui);
    let entries = lazy_entries();
    let level = |entry: &LazyEntry| editor.user.options.lazy_level(entry.option);
    let bands: [&[usize]; 5] = [&[0, 1], &[2, 3, 4], &[5, 6, 7], &[8], &[9, 10, 11, 12]];
    for (band_index, band) in bands.iter().enumerate() {
        let mut shown = false;
        for index in band.iter().copied() {
            let entry = &entries[index];
            let entry_level = level(entry);
            if entry_level <= 0 {
                continue;
            }
            shown = true;
            let color = if entry_level == 1 { entry.color.gamma_multiply(0.4) } else { entry.color };
            if menu_item(ui, Some(entry.icon), Some(color), entry.text, "").clicked() {
                commands.push(Command::Graph(GraphCommand::LazyAction(entry.tag)));
            }
        }
        let last = band_index == bands.len() - 1;
        if shown && !last {
            menu_separator(ui);
        }
    }
}

/// The bookmarks as Graph › Bookmarks lists them.
fn bookmark_items(ui: &mut Ui, editor: &Editor, state: &GraphState, commands: &mut Vec<Command>) {
    if menu_item(ui, Some("bookmark-plus"), None, "Add bookmark", "").clicked() {
        commands.push(Command::AddBookmark);
    }
    let bookmarks = editor.document.bookmarks();
    if bookmarks.is_empty() {
        return;
    }
    menu_separator(ui);
    for id in bookmarks {
        let Some(node) = editor.document.node(id) else {
            continue;
        };
        let removing = state.bookmark_removing == Some(id);
        let header = if removing { "Remove?" } else { node.name.as_str() };
        let icon = crate::ui::graph_canvas::node_icon(&node.type_name);
        let response = menu_item(ui, Some(&icon), Some(crate::ui::graph_canvas::node_color(node)), header, "");
        if response.secondary_clicked() {
            commands.push(Command::ArmBookmarkRemoval(id));
        } else if response.clicked() {
            let command = if removing { Command::RemoveBookmark(id) } else { Command::GoToBookmark(id) };
            commands.push(command);
        }
    }
}

/// The swatches of a colour menu (`ColorSwatchMenu` items in a 150 wide
/// `WrapPanel`): squares of 20 with 2 around each, the hover tint inset 3.
/// Returns the swatch clicked.
pub fn swatch_grid(ui: &mut Ui) -> Option<&'static str> {
    const CELL: f32 = 24.0;
    const COLUMNS: usize = 6;
    let rows = SWATCHES.len().div_ceil(COLUMNS);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(150.0, rows as f32 * CELL), Sense::hover());
    let mut chosen = None;
    for (index, name) in SWATCHES.iter().enumerate() {
        let column = (index % COLUMNS) as f32;
        let row = (index / COLUMNS) as f32;
        let cell = Rect::from_min_size(rect.min + Vec2::new(column * CELL, row * CELL), Vec2::splat(CELL));
        let swatch = cell.shrink(2.0);
        let response = ui.interact(swatch, ui.id().with(("swatch", index)), Sense::click());
        ui.painter().rect_filled(swatch, 0.0, theme::swatch_color(name));
        if response.hovered() {
            ui.painter().rect_filled(swatch.shrink(3.0), 0.0, theme::MENU_BACKGROUND_HOVER.gamma_multiply(0.4));
        }
        let response = response.on_hover_text(humanize_title(name));
        if response.clicked() {
            chosen = Some(*name);
        }
    }
    chosen
}

/// The icons of the group menu (`IconsList` in a 150 wide `WrapPanel`):
/// cells of 25, the icon green under the pointer (`Hover-IconX`).
pub fn icon_grid(ui: &mut Ui, choices: &[&'static str]) -> Option<&'static str> {
    const CELL: f32 = 25.0;
    const COLUMNS: usize = 6;
    let rows = choices.len().div_ceil(COLUMNS);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(150.0, rows as f32 * CELL), Sense::hover());
    let mut chosen = None;
    for (index, name) in choices.iter().enumerate() {
        let column = (index % COLUMNS) as f32;
        let row = (index / COLUMNS) as f32;
        let cell = Rect::from_min_size(rect.min + Vec2::new(column * CELL, row * CELL), Vec2::splat(CELL));
        let response = ui.interact(cell, ui.id().with(("group-icon", index)), Sense::click());
        if response.hovered() {
            ui.painter().rect_filled(cell.shrink(3.0), 0.0, theme::MENU_BACKGROUND_HOVER.gamma_multiply(0.4));
        }
        let tint = if response.hovered() { theme::ACCENT_GREEN } else { theme::MENU_ICON };
        let icon_rect = Rect::from_center_size(cell.center(), Vec2::splat(icons::DEFAULT_SIZE));
        icons::paint(ui, icon_rect, name, tint);
        if response.clicked() {
            chosen = Some(*name);
        }
    }
    chosen
}

/// `PopulateFamilyItems` (the node's name in the property area): the
/// nodes of its family in its toolbox category, each by type with its
/// short code, to replace it with; then Other node… (the search).
pub fn family_menu(name: &egui::Response, editor: &Editor, node: &nerthus_engine::NodeData, commands: &mut Vec<Command>) {
    let mut popup = Popup::menu(name);
    popup = popup.align(egui::RectAlign::BOTTOM_START);
    popup = popup.close_behavior(PopupCloseBehavior::CloseOnClickOutside);
    popup.show(|ui| {
        menu_style(ui);
        let catalogue = Catalogue::get();
        let Some(spec) = catalogue.node(&node.type_name) else {
            return;
        };
        let show_icons = editor.user.options.show_icons_in_search_results();
        let single = editor.selection.len() <= 1;
        let members = catalogue.nodes().filter(|member| member.family == spec.family && member.toolbox == spec.toolbox);
        for member in members {
            let listed = !member.has_flag("Hidden") && !member.has_flag("Dev") && !member.has_flag("Obsolete");
            if !listed || !crate::ui::graph_search::buildable(member.key()) {
                continue;
            }
            let short_code = member.short_code.clone().unwrap_or_default();
            let padded = format!("{short_code:>3}");
            let icon = crate::ui::graph_canvas::node_icon(member.key());
            let category = member.toolbox.as_deref().unwrap_or("Utility");
            let shown_icon = show_icons.then_some(icon.as_str());
            let clicked = menu_item(ui, shown_icon, Some(theme::toolbox_color(category)), member.key(), &padded).clicked();
            if clicked && single && member.key() != node.type_name {
                let pick = crate::ui::graph_commands::SearchPick { type_name: member.key().to_string(), preset: None };
                commands.push(Command::Graph(GraphCommand::ReplaceWith { node: node.id, pick }));
            }
        }
        menu_separator(ui);
        if menu_item(ui, None, None, "Other node...", "").clicked() {
            commands.push(Command::Graph(GraphCommand::ReplaceSearch));
        }
    });
}
