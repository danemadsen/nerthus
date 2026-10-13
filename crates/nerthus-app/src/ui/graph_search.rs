//! The graph's search (`SearchBase`, a menu topped by its text box):
//! nodes and presets to add (Tab, a connection dropped on the graph), to
//! replace a node with (Ctrl Q) or to insert into a connection; the
//! project's nodes to go to (Ctrl T); published portals to connect to.
//! Enter takes the first result, the arrows walk them, a comma adds the
//! first to a list of nodes to make in a chain (`IsMultiSelect`).

use egui::{Align, Event, Frame, Id, Key, Layout, LayerId, Margin, Modifiers, Order, Popup, PopupAnchor, PopupCloseBehavior, PopupKind, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use nerthus_engine::{Catalogue, NodeSpec, Registry};

use crate::document::{node_tab, PortalState};
use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::graph_canvas::{node_color, node_icon};
use crate::ui::graph_commands::{GraphCommand, NodeSource, SearchPick};
use crate::ui::widgets::{menu_style, submenu};

/// Nodes that hand terrain to other programs, which Nerthus leaves out.
const LEFT_OUT: [&str; 3] = ["HoudiniOut", "Unity", "Unreal"];
/// `MinWidth` of a result.
const RESULT_WIDTH: f32 = 230.0;
const RESULT_HEIGHT: f32 = 23.0;
/// `Label-Search-Family`.
const FAMILY_COLOR: egui::Color32 = egui::Color32::from_rgb(0x99, 0x99, 0x99);
/// `Brushes.SlateGray` at 0.6: a result's short code.
const SHORT_CODE_COLOR: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x45, 0x4d, 0x56, 0x99);
/// What a search with nothing typed falls back to (`NodeSearchManager`'s
/// catch).
const FALLBACK_FAVORITES: [&str; 6] = ["Mountain", "Ridge", "Perlin", "ErosionPro", "Adjust", "Combine"];

/// What the search is for.
#[derive(Clone, Debug, PartialEq)]
pub enum SearchMode {
    /// Nodes (and presets) to add, joined to `source`.
    Nodes { source: NodeSource },
    /// The node to replace `node` with (`IsReplaceMode`).
    Replace { node: i32 },
    /// The node to insert into a connection (`IsInterjectMode`).
    Interject { to: i32, to_port: String },
    /// The project's nodes, to go to one (`NavigateExistingNodes`).
    Navigate,
    /// The published portals, to feed an input from one.
    Portals { to: i32, to_port: String },
}

/// The open search.
#[derive(Clone, Debug)]
pub struct SearchPopup {
    pub mode: SearchMode,
    /// Where it opened (screen) and that point on the canvas (`MousePos`).
    pub at: Pos2,
    pub canvas: [f32; 2],
    text: String,
    /// The result the arrow keys reached (none: the text box).
    highlighted: Option<usize>,
    focused: bool,
}

impl SearchPopup {
    pub fn new(mode: SearchMode, at: Pos2, canvas: [f32; 2]) -> Self {
        Self {
            mode,
            at,
            canvas,
            text: String::new(),
            highlighted: None,
            focused: false,
        }
    }

    /// Text as though typed (`ShowNodeSearch(text)`).
    pub fn type_text(&mut self, text: &str) {
        self.text = text.to_string();
    }
}

/// A search result (`FoundItem`).
#[derive(Clone, Debug)]
enum Found {
    /// A node type, by catalogue key.
    Node(&'static NodeSpec),
    /// A preset: its name and node type.
    Preset { name: String, node_type: String },
    /// A node of the project.
    Existing(i32),
    /// A published portal (node, port).
    Portal(i32, String),
    /// A graph tab's caption among the portals.
    TabCaption(String, egui::Color32),
}

fn registry() -> &'static Registry {
    static REGISTRY: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(nerthus_nodes::registry)
}

/// `NodeMetadata.SearchWords`: name, family, keywords and short code in
/// lower case, split at spaces.
fn search_words(spec: &NodeSpec) -> Vec<String> {
    let keywords = match &spec.keywords {
        Some(serde_json::Value::Array(words)) => words.iter().filter_map(serde_json::Value::as_str).collect::<Vec<&str>>().join(""),
        Some(serde_json::Value::String(words)) => words.clone(),
        _ => String::new(),
    };
    let family = spec.family.clone().unwrap_or_default();
    let short_code = spec.short_code.clone().unwrap_or_default();
    let line = format!("{} {family} {keywords} {short_code}", spec.key());
    let lower = line.to_lowercase();
    lower.split(' ').map(str::to_string).collect()
}

/// Whether Nerthus builds the node type.
pub fn buildable(type_name: &str) -> bool {
    registry().implements(type_name)
}

/// The node types the search offers: shown in the toolbox and built by
/// Nerthus.
fn searchable(spec: &NodeSpec) -> bool {
    let hidden = spec.has_flag("Hidden") || spec.has_flag("Dev") || spec.has_flag("Obsolete");
    !hidden && !LEFT_OUT.contains(&spec.key()) && buildable(spec.key())
}

/// `NodeSearchManager.SearchMetadata`.
fn node_results(editor: &Editor, mode: &SearchMode, typed: &str) -> Vec<Found> {
    let catalogue = Catalogue::get();
    let usage = |name: &str| editor.user.usage.usage(name);
    let mut search_text = typed.trim().to_string();
    let presets_first = search_text.starts_with('_');
    if presets_first {
        search_text = search_text.trim_start_matches('_').to_string();
    }
    if search_text.is_empty() {
        let names: Vec<String> = match mode {
            // A dragged connection: what followed that port most.
            SearchMode::Nodes { source: NodeSource::Port { node, port, .. } } => {
                let from_name = editor.document.node(*node).map(|data| data.name.clone()).unwrap_or_default();
                editor.user.usage.suggestions(&from_name, port, 5)
            }
            _ => top_used(editor),
        };
        let mut found: Vec<&'static NodeSpec> = names.iter().filter_map(|name| catalogue.node(name)).filter(|spec| searchable(spec)).collect();
        found.sort_by_key(|spec| std::cmp::Reverse(usage(spec.key())));
        return found.into_iter().map(Found::Node).collect();
    }
    let mut results: Vec<(Found, [bool; 4], i64)> = Vec::new();
    for spec in catalogue.nodes().filter(|spec| searchable(spec)) {
        let name = spec.key();
        let short_code = spec.short_code.as_deref().unwrap_or("");
        let words = search_words(spec);
        let exact = name.eq_ignore_ascii_case(&search_text) || short_code.eq_ignore_ascii_case(&search_text);
        let matches = exact || short_code == search_text || name.contains(search_text.as_str()) || words.iter().any(|word| word.contains(search_text.as_str())) || name.to_lowercase().starts_with(&search_text);
        if !matches {
            continue;
        }
        let word_equal = words.contains(&search_text);
        let starts = name.to_lowercase().starts_with(&search_text);
        results.push((Found::Node(spec), [false, exact, word_equal, starts], usage(name) as i64));
    }
    for preset in editor.presets.all() {
        let Some(spec) = catalogue.node(&preset.applicable_type) else { continue };
        let short_code = preset.short_code();
        let exact = preset.name.eq_ignore_ascii_case(&search_text) || short_code.eq_ignore_ascii_case(&search_text);
        let matches = exact || preset.name.contains(search_text.as_str()) || preset.applicable_type.eq_ignore_ascii_case(&search_text) || preset.name.to_lowercase().starts_with(&search_text);
        if !matches || !searchable(spec) {
            continue;
        }
        let starts = preset.name.to_lowercase().starts_with(&search_text);
        let found = Found::Preset { name: preset.name.clone(), node_type: spec.key().to_string() };
        results.push((found, [presets_first, exact, false, starts], usage(&preset.name) as i64));
    }
    // Stable, as LINQ's `OrderByDescending(...).ThenByDescending(...)` is.
    results.sort_by(|left, right| right.1.cmp(&left.1).then(right.2.cmp(&left.2)));
    results.into_iter().map(|(found, _, _)| found).collect()
}

/// `topUsedNodes`: the ten types added most once more than ten types
/// have been, else the favourites.
fn top_used(editor: &Editor) -> Vec<String> {
    if editor.user.usage.used_type_count() > 10 {
        return editor.user.usage.most_used(10);
    }
    let favorites = editor.user.options.favorites();
    if favorites.is_empty() {
        return FALLBACK_FAVORITES.iter().map(|name| name.to_string()).collect();
    }
    let mut unique = Vec::new();
    for favorite in favorites {
        if !unique.contains(&favorite) {
            unique.push(favorite);
        }
    }
    unique
}

/// `ExistingNodeSearchManager.SearchNodes`: the project's nodes named or
/// numbered like the text, exact first, then by place left to right.
fn existing_results(editor: &Editor, typed: &str) -> Vec<Found> {
    let search_text = typed.trim().to_lowercase();
    let mut results: Vec<(i32, [bool; 3], f32)> = Vec::new();
    for node in editor.document.terrain().nodes.values() {
        let lower = node.name.to_lowercase();
        let named = node.name.eq_ignore_ascii_case(&search_text);
        let numbered = node.id.to_string() == search_text;
        let matches = named || numbered || lower.contains(&search_text) || lower.starts_with(&search_text);
        if !matches {
            continue;
        }
        results.push((node.id, [named, numbered, lower.starts_with(&search_text)], node.position[0]));
    }
    results.sort_by(|left, right| right.1.cmp(&left.1).then(left.2.total_cmp(&right.2)));
    results.into_iter().map(|(id, _, _)| Found::Existing(id)).collect()
}

/// `PortalSearchManager.SearchPortals`: each tab's published portals named
/// like the text (the node being fed left out), under the tab's name when
/// the project has more than one tab.
fn portal_results(editor: &Editor, typed: &str, exclude: i32) -> Vec<Found> {
    let search_text = typed.trim().to_lowercase();
    let tabs = editor.document.graph_tabs();
    let mut by_tab: Vec<(usize, Vec<Found>)> = Vec::new();
    for tab in &tabs {
        let mut found = Vec::new();
        let on_tab = editor.document.terrain().nodes.values().filter(|node| node_tab(node) == tab.index && node.id != exclude);
        for node in on_tab {
            let publishing = node.ports.iter().filter(|port| port.is_output() && PortalState::of(port) == PortalState::Publishing);
            for port in publishing {
                let named = node.name.to_lowercase().contains(&search_text) || port.name.to_lowercase().contains(&search_text);
                if named {
                    found.push(Found::Portal(node.id, port.name.clone()));
                }
            }
        }
        by_tab.push((tab.index, found));
    }
    let total: usize = by_tab.iter().map(|(_, found)| found.len()).sum();
    let captions = tabs.len() > 1 && total > 1;
    let mut results = Vec::new();
    for (index, found) in by_tab {
        if found.is_empty() {
            continue;
        }
        if captions && let Some(tab) = tabs.iter().find(|tab| tab.index == index) {
            results.push(Found::TabCaption(tab.name.clone(), theme::swatch_color(&tab.color)));
        }
        results.extend(found);
    }
    results
}

fn results(editor: &Editor, popup: &SearchPopup) -> Vec<Found> {
    match &popup.mode {
        SearchMode::Nodes { .. } | SearchMode::Replace { .. } | SearchMode::Interject { .. } => {
            // With commas, the search is for the last name.
            let last = popup.text.rsplit(',').next().unwrap_or("");
            node_results(editor, &popup.mode, &last.to_lowercase())
        }
        SearchMode::Navigate => existing_results(editor, &popup.text),
        SearchMode::Portals { to, .. } => portal_results(editor, &popup.text, *to),
    }
}

/// What a result is called in the text box's list of names (`FoundItem.Name`).
fn found_name(found: &Found) -> String {
    match found {
        Found::Node(spec) => spec.key().to_string(),
        Found::Preset { name, .. } => name.clone(),
        _ => String::new(),
    }
}

/// Shows the search; returns whether it stays open.
pub fn show(ui: &mut Ui, editor: &mut Editor, popup: &mut SearchPopup, commands: &mut Vec<Command>) -> bool {
    // `SearchTextBox_KeyDown`: a comma adds the first result to the list
    // instead of being typed; Enter takes one; the arrows walk them.
    let comma = ui.input_mut(|input| {
        let pressed = input.consume_key(Modifiers::NONE, Key::Comma);
        if pressed {
            input.events.retain(|event| !matches!(event, Event::Text(text) if text == ","));
        }
        pressed
    });
    let enter = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
    let down = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown));
    let up = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp));
    let found = results(editor, popup);
    let choosable: Vec<usize> = found.iter().enumerate().filter(|(_, item)| !matches!(item, Found::TabCaption(..))).map(|(index, _)| index).collect();
    let multi = matches!(popup.mode, SearchMode::Nodes { .. });

    if comma && multi && let Some(first) = choosable.first() {
        add_selection(popup, &found[*first]);
    }
    if down {
        popup.highlighted = next_choice(&choosable, popup.highlighted, true);
    }
    if up {
        popup.highlighted = next_choice(&choosable, popup.highlighted, false);
    }
    if enter {
        let chosen = popup.highlighted.or_else(|| choosable.first().copied());
        let picked = chosen.and_then(|index| found.get(index));
        if multi && popup.text.contains(',') {
            choose_many(editor, popup, picked, commands);
            return false;
        }
        if let Some(item) = picked {
            choose(editor, popup, item, commands);
        }
        return false;
    }

    let id = Id::new("graph-search");
    let layer = LayerId::new(Order::Foreground, id);
    let mut open = true;
    let mut chosen: Option<Found> = None;
    let mut menu = Popup::new(id, ui.ctx().clone(), PopupAnchor::Position(popup.at), layer);
    menu = menu.kind(PopupKind::Menu);
    menu = menu.width(RESULT_WIDTH + 30.0);
    menu = menu.layout(Layout::top_down_justified(Align::Min));
    menu = menu.close_behavior(PopupCloseBehavior::CloseOnClickOutside);
    menu = menu.open_bool(&mut open);
    menu.show(|ui| {
        menu_style(ui);
        ui.set_min_width(RESULT_WIDTH + 30.0);
        search_box(ui, popup);
        let limit = editor.user.options.maximum_search_results();
        let mut counted = 0;
        let mut overflow: Vec<usize> = Vec::new();
        for (index, item) in found.iter().enumerate() {
            if let Found::TabCaption(name, color) = item {
                tab_caption(ui, name, *color);
                continue;
            }
            counted += 1;
            if counted > limit {
                overflow.push(index);
                continue;
            }
            let highlighted = popup.highlighted == Some(index);
            if result_row(ui, editor, item, highlighted) {
                chosen = Some(item.clone());
            }
        }
        if overflow.is_empty() {
            return;
        }
        submenu(ui, None, None, "more...", |ui| {
            for index in overflow {
                if result_row(ui, editor, &found[index], false) {
                    chosen = Some(found[index].clone());
                }
            }
        });
    });
    if let Some(item) = chosen {
        if multi && popup.text.contains(',') {
            choose_many(editor, popup, Some(&item), commands);
        } else {
            choose(editor, popup, &item, commands);
        }
        return false;
    }
    open
}

/// The arrows: Down from the text box to the first result, Up to the
/// last; on through the results, round again.
fn next_choice(choosable: &[usize], current: Option<usize>, down: bool) -> Option<usize> {
    if choosable.is_empty() {
        return None;
    }
    let position = current.and_then(|index| choosable.iter().position(|choice| *choice == index));
    let next = match (position, down) {
        (None, true) => 0,
        (None, false) => choosable.len() - 1,
        (Some(at), true) => (at + 1) % choosable.len(),
        (Some(at), false) => (at + choosable.len() - 1) % choosable.len(),
    };
    Some(choosable[next])
}

/// `AddSelection`: the name being typed becomes the result's, and a comma
/// follows for the next.
fn add_selection(popup: &mut SearchPopup, found: &Found) {
    let name = found_name(found);
    if name.is_empty() {
        return;
    }
    let kept = match popup.text.rfind(',') {
        Some(comma) => popup.text[..=comma].to_string(),
        None => String::new(),
    };
    popup.text = format!("{kept}{name}, ");
    popup.highlighted = None;
    popup.focused = false;
}

/// `CreateMultiSelectedNodes`: every name of the list that is a node type
/// or a preset, made in a chain.
fn choose_many(editor: &Editor, popup: &mut SearchPopup, last: Option<&Found>, commands: &mut Vec<Command>) {
    if !popup.text.trim_end().ends_with(',')
        && let Some(last) = last
    {
        add_selection(popup, last);
    }
    let catalogue = Catalogue::get();
    let mut picks = Vec::new();
    for name in popup.text.split(',') {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        if let Some(spec) = catalogue.node(name).filter(|spec| searchable(spec)) {
            picks.push(SearchPick { type_name: spec.key().to_string(), preset: None });
            continue;
        }
        if let Some(preset) = editor.presets.all().iter().find(|preset| preset.name == name) {
            picks.push(SearchPick { type_name: preset.applicable_type.clone(), preset: Some(preset.name.clone()) });
        }
    }
    let SearchMode::Nodes { source } = &popup.mode else {
        return;
    };
    let create = GraphCommand::CreateNodes { picks, at: popup.canvas, source: source.clone() };
    commands.push(Command::Graph(create));
}

/// `SearchX_ItemSelected` for one result.
fn choose(editor: &Editor, popup: &SearchPopup, found: &Found, commands: &mut Vec<Command>) {
    let pick = match found {
        Found::Node(spec) => Some(SearchPick { type_name: spec.key().to_string(), preset: None }),
        Found::Preset { name, node_type } => Some(SearchPick { type_name: node_type.clone(), preset: Some(name.clone()) }),
        _ => None,
    };
    let command = match (&popup.mode, pick, found) {
        (SearchMode::Nodes { source }, Some(pick), _) => {
            let source = match source {
                // `if (SelectedItem is NodeElement) ConnectNodes = true`.
                NodeSource::None => editor.active_node().map_or(NodeSource::None, NodeSource::Selected),
                other => other.clone(),
            };
            GraphCommand::CreateNodes { picks: vec![pick], at: popup.canvas, source }
        }
        (SearchMode::Replace { node }, Some(pick), _) => GraphCommand::ReplaceWith { node: *node, pick },
        (SearchMode::Interject { to, to_port }, Some(pick), _) => GraphCommand::InsertNodeInConnection { to: *to, to_port: to_port.clone(), type_name: pick.type_name, corner: popup.canvas },
        (SearchMode::Navigate, _, Found::Existing(id)) => GraphCommand::Navigate(*id),
        (SearchMode::Portals { to, to_port }, _, Found::Portal(node, port)) => GraphCommand::ConnectToPortal { from: *node, from_port: port.clone(), to: *to, to_port: to_port.clone() },
        _ => return,
    };
    commands.push(Command::Graph(command));
}

/// The text box (`TextBox-Search`): dark, its border blue while it has
/// the keyboard, corners 3.
fn search_box(ui: &mut Ui, popup: &mut SearchPopup) {
    let mut frame = Frame::new();
    frame = frame.fill(theme::TEXTBOX_BACKGROUND);
    frame = frame.stroke(Stroke::new(1.0, theme::TEXTBOX_FOCUSED_BORDER));
    frame = frame.corner_radius(3.0);
    frame = frame.inner_margin(Margin::same(4));
    frame = frame.outer_margin(Margin::symmetric(1, 2));
    frame.show(ui, |ui| {
        let mut edit = egui::TextEdit::singleline(&mut popup.text);
        edit = edit.frame(Frame::NONE);
        edit = edit.font(theme::interface(12.0));
        edit = edit.text_color(theme::LABEL_EMPHASIS);
        edit = edit.desired_width(RESULT_WIDTH);
        edit = edit.id(Id::new("graph-search-text"));
        let response = ui.add(edit);
        if !popup.focused {
            response.request_focus();
            // The caret at the end, as `AddSelection` leaves it.
            let length = popup.text.chars().count();
            let mut state = egui::TextEdit::load_state(ui.ctx(), response.id).unwrap_or_default();
            let range = egui::text::CCursorRange::one(egui::text::CCursor::new(length));
            state.cursor.set_char_range(Some(range));
            state.store(ui.ctx(), response.id);
            popup.focused = true;
        }
        if response.changed() {
            popup.highlighted = None;
        }
    });
}

/// A grey tab name over its portals.
fn tab_caption(ui: &mut Ui, name: &str, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width().max(RESULT_WIDTH), RESULT_HEIGHT), Sense::hover());
    let galley = ui.painter().layout_no_wrap(name.to_string(), theme::interface(11.0), color.gamma_multiply(0.7));
    ui.painter().galley(Pos2::new(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0), galley, color);
}

/// One result (`CreateNodeMenuItem` and its kin): the family icon in the
/// category colour, the name, and on the right the family and short code
/// (a preset: its node type in teal and `⁋`). Returns whether it was
/// clicked.
fn result_row(ui: &mut Ui, editor: &Editor, found: &Found, highlighted: bool) -> bool {
    let show_icons = editor.user.options.show_icons_in_search_results();
    let show_family = editor.user.options.show_family_name();
    let catalogue = Catalogue::get();
    let (icon, color, name, right, right_color, short_code, tooltip) = match found {
        Found::Node(spec) => {
            let category = spec.toolbox.as_deref().unwrap_or("Utility");
            let display = spec.display_name.clone().unwrap_or_else(|| spec.name.clone());
            let family = if show_family { spec.family.clone().unwrap_or_default() } else { String::new() };
            let code = spec.short_code.clone().unwrap_or_default();
            (node_icon(spec.key()), theme::toolbox_color(category), display, family, FAMILY_COLOR, code, spec.description.clone())
        }
        Found::Preset { name, node_type } => {
            let spec = catalogue.node(node_type);
            let category = spec.and_then(|spec| spec.toolbox.as_deref()).unwrap_or("Utility");
            let node_name = spec.map(|spec| spec.display_name.clone().unwrap_or_else(|| spec.name.clone())).unwrap_or_default();
            (node_icon(node_type), theme::toolbox_color(category), name.clone(), node_name, theme::ACCENT_MUTED_TEAL, "⁋".to_string(), None)
        }
        Found::Existing(id) => {
            let Some(node) = editor.document.node(*id) else { return false };
            (node_icon(&node.type_name), node_color(node), node.name.clone(), String::new(), FAMILY_COLOR, String::new(), None)
        }
        Found::Portal(id, port) => {
            let Some(node) = editor.document.node(*id) else { return false };
            let primary = node.port(port).is_some_and(|data| data.is_primary());
            let suffix = if primary { String::new() } else { format!(" - {port}") };
            (node_icon(&node.type_name), node_color(node), format!("{} {suffix}", node.name), String::new(), FAMILY_COLOR, String::new(), None)
        }
        Found::TabCaption(..) => return false,
    };
    let width = ui.available_width().max(RESULT_WIDTH + 30.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, RESULT_HEIGHT), Sense::click());
    let hovered = response.hovered() || highlighted;
    if hovered {
        ui.painter().rect_filled(rect.shrink(1.0), 4.0, theme::MENU_BACKGROUND_HOVER);
    }
    if show_icons {
        let icon_rect = Rect::from_center_size(Pos2::new(rect.left() + 15.0, rect.center().y), Vec2::splat(icons::DEFAULT_SIZE));
        icons::paint(ui, icon_rect, &icon, color);
    }
    let name_color = if hovered { theme::MENU_FOREGROUND_HOVER } else { theme::LABEL_EMPHASIS };
    let name_galley = ui.painter().layout_no_wrap(name, theme::interface(12.0), name_color);
    ui.painter().galley(Pos2::new(rect.left() + 32.0, rect.center().y - name_galley.size().y / 2.0), name_galley, name_color);
    let mut right_edge = rect.right() - 8.0;
    if !short_code.is_empty() {
        let padded = format!("{short_code:>3}");
        let code_galley = ui.painter().layout_no_wrap(padded, theme::monospace(10.0), SHORT_CODE_COLOR);
        right_edge -= code_galley.size().x;
        ui.painter().galley(Pos2::new(right_edge, rect.center().y - code_galley.size().y / 2.0 + 1.0), code_galley, SHORT_CODE_COLOR);
        right_edge -= 4.0;
    }
    if !right.is_empty() {
        let right_galley = ui.painter().layout_no_wrap(right, theme::interface(10.0), right_color);
        right_edge -= right_galley.size().x;
        ui.painter().galley(Pos2::new(right_edge, rect.center().y - right_galley.size().y / 2.0 + 1.0), right_galley, right_color);
    }
    let response = match tooltip {
        Some(text) => response.on_hover_text(text),
        None => response,
    };
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_walk_the_results_round() {
        let choices = [1, 2, 4];
        assert_eq!(next_choice(&choices, None, true), Some(1));
        assert_eq!(next_choice(&choices, None, false), Some(4));
        assert_eq!(next_choice(&choices, Some(4), true), Some(1));
        assert_eq!(next_choice(&choices, Some(1), false), Some(4));
        assert_eq!(next_choice(&[], None, true), None);
    }

    #[test]
    fn search_words_hold_name_family_keywords_and_code() {
        let spec = Catalogue::get().node("Adjust").unwrap();
        let words = search_words(spec);
        assert!(words.contains(&"adjust".to_string()));
        assert!(words.contains(&"autolevel".to_string()));
    }

    #[test]
    fn a_comma_takes_the_first_result_into_the_list() {
        let mut popup = SearchPopup::new(SearchMode::Navigate, Pos2::ZERO, [0.0, 0.0]);
        popup.text = "Mountain, ero".to_string();
        let erosion = Catalogue::get().node("ErosionPro").unwrap();
        add_selection(&mut popup, &Found::Node(erosion));
        // `String.Remove(text, LastIndexOf(',') + 1)`: the space after the
        // kept comma goes too.
        assert_eq!(popup.text, "Mountain,ErosionPro, ");
    }
}
