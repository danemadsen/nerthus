//! The toolbox (`Toolbox`) over the graph's left side, in two styles:
//! menus (one button per toolbox category, `ToolboxMenuItem`, as
//! long icons, compact icon pairs or expanded with names, each opening the
//! category's families nested or as a flat list) or a panel of node
//! buttons in columns, by category or not. Its right-click menu holds the
//! settings; the most used nodes may come first and stand out. An entry
//! clicked adds its node after the selected one; one dragged onto the
//! graph lands there (onto a node, it follows it). The same menus add
//! nodes from the graph's context menu.

use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

use egui::containers::menu::MenuConfig;
use egui::{Color32, CursorIcon, Pos2, Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, UiKind, UiStackInfo, Vec2};
use nerthus_engine::{Catalogue, NodeSpec, Registry};

use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::graph_canvas::GraphState;
use crate::ui::graph_commands::GraphCommand;
use crate::ui::widgets::{check_item, checked_icon_item, header_submenu, menu_caption, menu_item, menu_separator, menu_style, submenu};
use crate::user::{ToolboxMode, ToolboxStyle, UserOptions};

/// The toolbox categories, in toolbox order (`NodeCategory`).
pub const CATEGORIES: [&str; 10] = ["Primitive", "Terrain", "Modify", "Surface", "Simulate", "Derive", "Colorize", "Output", "Utility", "Macro"];

/// Nodes that hand terrain to other programs, which Nerthus leaves out.
const LEFT_OUT: [&str; 3] = ["HoudiniOut", "Unity", "Unreal"];

/// `ToolboxMenuItem`: a 28 icon column, the name with margins 0, 10, 5, 10.
const ICON_COLUMN: f32 = 28.0;
const TEXT_PADDING: f32 = 10.0;
/// `ToolboxButtonSize`, and the menus' width when compact or expanded
/// (`GetWidthForMode`).
const BUTTON_SIZE: f32 = 36.0;
const WIDE: f32 = BUTTON_SIZE * 2.3;
/// `ToolboxButton-Hover` and `ToolboxButton-Pressed`.
const BUTTON_HOVER: Color32 = Color32::from_rgb(0x33, 0x33, 0x33);
const BUTTON_OPEN: Color32 = Color32::from_rgb(0x16, 0x16, 0x16);
/// The panel style's columns are about this wide (`ResizeToolbox`).
const PANEL_COLUMN: f32 = 80.0;
/// `GridSplitter` (10 wide) and how far a drag goes before it counts.
const SPLITTER: f32 = 10.0;
const SPLITTER_STEP: f32 = 10.0;

/// What the toolbox keeps between frames: the panel's closed categories
/// and a resize under way.
#[derive(Clone, Debug, Default)]
pub struct ToolboxState {
    collapsed: HashSet<&'static str>,
    resizing: Option<f32>,
}

/// Where a menu of nodes was opened from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
    /// The toolbox: an entry adds its node after the selected one
    /// (`CreateNodeOnCanvas`) or is dragged onto the graph.
    Toolbox,
    /// The graph's context menu: at the right-click.
    Canvas,
}

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(nerthus_nodes::registry)
}

pub(crate) fn listed(spec: &NodeSpec) -> bool {
    let hidden = spec.has_flag("Hidden") || spec.has_flag("Dev") || spec.has_flag("Obsolete");
    !hidden && !LEFT_OUT.contains(&spec.key())
}

/// A category's nodes in toolbox order: by type name, or (Pin To Top) the
/// most used first (`AddFamilyMembers`).
fn category_nodes(category: &str, options: &UserOptions, usage: &dyn Fn(&str) -> i32) -> Vec<&'static NodeSpec> {
    let in_category = Catalogue::get().nodes().filter(|spec| spec.toolbox.as_deref() == Some(category) && listed(spec));
    let mut nodes: Vec<&'static NodeSpec> = in_category.collect();
    nodes.sort_by(|left, right| left.key().cmp(right.key()));
    if options.toolbox_pin_high_use() {
        nodes.sort_by_key(|spec| std::cmp::Reverse(usage(spec.key())));
    }
    nodes
}

/// The nodes a category offers, by family (families by name).
fn category_families(category: &str, options: &UserOptions, usage: &dyn Fn(&str) -> i32) -> BTreeMap<String, Vec<&'static NodeSpec>> {
    let mut families: BTreeMap<String, Vec<&'static NodeSpec>> = BTreeMap::new();
    for spec in category_nodes(category, options, usage) {
        let family = spec.family.clone().unwrap_or_else(|| "Other".to_string());
        families.entry(family).or_default().push(spec);
    }
    families
}

fn node_label(spec: &NodeSpec) -> &str {
    spec.display_name.as_deref().unwrap_or(&spec.name)
}

/// `GetAverageUsage`: the mean use of every node type, as of the start.
fn average_usage(editor: &Editor) -> f64 {
    let usage = editor.user.usage.startup_usage();
    let nodes: Vec<&NodeSpec> = Catalogue::get().nodes().collect();
    if nodes.is_empty() {
        return 1.0;
    }
    let total: i64 = nodes.iter().map(|spec| usage.get(spec.key()).copied().unwrap_or(0) as i64).sum();
    total as f64 / nodes.len() as f64
}

/// Everything an entry needs to know of the settings and the usage.
struct EntryLook<'a> {
    editor: &'a Editor,
    average: f64,
}

impl EntryLook<'_> {
    fn usage(&self, type_name: &str) -> i32 {
        self.editor.user.usage.usage(type_name)
    }

    /// Highlight: entries used less than the average are faint.
    fn faint(&self, spec: &NodeSpec) -> bool {
        self.editor.user.options.toolbox_highlight_high_use() && (self.usage(spec.key()) as f64) < self.average
    }
}

/// One node entry (`AddFamilyMembers`): its `x-` icon when it has one (and
/// icons are shown), the name, the short code as the shortcut text and
/// the description as the tooltip; dimmed when Nerthus cannot build it,
/// faint when highlighting and little used.
fn node_entry(ui: &mut Ui, spec: &'static NodeSpec, look: &EntryLook, origin: Origin, state: &mut GraphState, commands: &mut Vec<Command>) {
    let implemented = registry().implements(spec.key());
    let icon_name = format!("x-{}", spec.name.to_lowercase());
    let show_icon = look.editor.user.options.show_icons_in_toolbox() && icons::exists(&icon_name);
    let icon = if show_icon { Some(icon_name.as_str()) } else { None };
    let short_code = spec.short_code.clone().unwrap_or_default();
    let faint = look.faint(spec);
    let mut response = ui
        .add_enabled_ui(implemented, |ui| {
            if faint {
                ui.multiply_opacity(0.5);
            }
            menu_item(ui, icon, None, node_label(spec), &short_code)
        })
        .inner;
    if let Some(description) = &spec.description {
        response = response.on_hover_text(description);
    }
    if !implemented {
        response = response.on_disabled_hover_text("Not available in Nerthus yet");
    }
    // `Member_PreviewMouseMove`: pressed and moved, the entry is dragged.
    let pressed_here = implemented && response.is_pointer_button_down_on();
    let moved = ui.input(|input| input.pointer.is_decidedly_dragging());
    if origin == Origin::Toolbox && pressed_here && moved {
        state.toolbox_drag = Some(spec.key().to_string());
        ui.close();
        return;
    }
    if !response.clicked() {
        return;
    }
    let command = match origin {
        Origin::Toolbox => Command::Graph(GraphCommand::ToolboxCreate(spec.key().to_string())),
        Origin::Canvas => Command::AddNode { type_name: spec.key().to_string(), position: None },
    };
    commands.push(command);
}

/// A category's families (`CreateFamilyMenu`): nested, each a header in
/// the category's colour opening its nodes; flat, the headers as captions
/// over their nodes, separated.
fn category_menu(ui: &mut Ui, category: &str, look: &EntryLook, origin: Origin, state: &mut GraphState, commands: &mut Vec<Command>) {
    menu_style(ui);
    if category == "Macro" {
        macro_menu(ui, commands);
        return;
    }
    let color = theme::toolbox_color(category);
    let options = &look.editor.user.options;
    let usage = |name: &str| look.usage(name);
    let families = category_families(category, options, &usage);
    let nested = options.toolbox_nested();
    for (index, (family, nodes)) in families.into_iter().enumerate() {
        let family_icon = format!("family-{}", family.to_lowercase());
        let icon = if icons::exists(&family_icon) { family_icon } else { "square-rounded".to_string() };
        if nested {
            header_submenu(ui, &icon, color, &family, |ui| {
                for spec in nodes {
                    node_entry(ui, spec, look, origin, state, commands);
                }
            });
            continue;
        }
        if index > 0 {
            menu_separator(ui);
        }
        family_caption(ui, &icon, color, &family);
        for spec in nodes {
            node_entry(ui, spec, look, origin, state, commands);
        }
    }
}

/// A flat list's family header (`Tag="HEADER"`, disabled): its icon and
/// small capitals in the category's colour.
fn family_caption(ui: &mut Ui, icon: &str, color: Color32, family: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width().max(190.0), 23.0), Sense::hover());
    let icon_rect = Rect::from_center_size(Pos2::new(rect.left() + 15.0, rect.center().y), Vec2::splat(icons::DEFAULT_SIZE));
    icons::paint(ui, icon_rect, icon, color);
    let galley = ui.painter().layout_no_wrap(family.to_uppercase(), theme::interface(11.0 * 0.8), color);
    ui.painter().galley(Pos2::new(rect.left() + 32.0, rect.center().y - galley.size().y / 2.0), galley, color);
}

/// The Macro category: the installed macros by author
/// (`NodeMetadata.Family`), each by name with the `x-macro` icon and its
/// description as the tooltip.
fn macro_menu(ui: &mut Ui, commands: &mut Vec<Command>) {
    let color = theme::toolbox_color("Macro");
    let authors = crate::files::installed_macros();
    if authors.is_empty() {
        menu_caption(ui, "No macros installed");
        return;
    }
    for (author, entries) in authors {
        header_submenu(ui, "x-macro", color, &author, |ui| {
            for entry in entries {
                let mut response = menu_item(ui, Some("x-macro"), None, &entry.metadata.name, "");
                if !entry.metadata.description.is_empty() {
                    response = response.on_hover_text(&entry.metadata.description);
                }
                if response.clicked() {
                    commands.push(Command::File(crate::files::FileCommand::AddMacro(entry.metadata.id())));
                }
            }
        });
    }
}

/// Every category as a submenu (the graph's context menu).
pub fn add_node_menus(ui: &mut Ui, editor: &Editor, state: &mut GraphState, commands: &mut Vec<Command>) {
    let look = EntryLook { editor, average: average_usage(editor) };
    for category in CATEGORIES {
        let color = theme::toolbox_color(category);
        let icon = format!("toolbox-{}", category.to_lowercase());
        submenu(ui, Some(&icon), Some(color), category, |ui| category_menu(ui, category, &look, Origin::Canvas, state, commands));
    }
}

/// The toolbox at the top-left of `area` (`Toolbox`'s place in the graph
/// area, its margin taken off).
pub fn show(ui: &mut Ui, area: Rect, editor: &mut Editor, state: &mut GraphState, commands: &mut Vec<Command>) {
    match editor.user.options.toolbox_style() {
        ToolboxStyle::Menus => show_menus(ui, area, editor, state, commands),
        ToolboxStyle::Panel => show_panel(ui, area, editor, state, commands),
    }
}

/// The menus style (`ScrollViewer`, margin 10): Long is one column of
/// icons, Compact two, Expanded one of icons with names.
fn show_menus(ui: &mut Ui, area: Rect, editor: &mut Editor, state: &mut GraphState, commands: &mut Vec<Command>) {
    let mode = editor.user.options.toolbox_mode();
    let font = theme::semibold(10.0);
    let line_height = ui.painter().layout_no_wrap("Primitive".to_string(), font, theme::LABEL_DEFAULT).size().y;
    let button_height = line_height + 2.0 * TEXT_PADDING;
    let (width, button_width, per_row) = match mode {
        ToolboxMode::Single => (BUTTON_SIZE, BUTTON_SIZE, 1),
        ToolboxMode::Double => (WIDE, BUTTON_SIZE, 2),
        ToolboxMode::Expanded => (WIDE, WIDE, 1),
    };
    let rows = CATEGORIES.len().div_ceil(per_row);
    let column = Rect::from_min_size(area.min + Vec2::new(10.0, 10.0), Vec2::new(width, rows as f32 * button_height));
    let look = EntryLook { editor, average: average_usage(editor) };
    let mut settings: Vec<SettingsChange> = Vec::new();
    for (index, category) in CATEGORIES.iter().enumerate() {
        let row = (index / per_row) as f32;
        let place = (index % per_row) as f32;
        let corner = column.min + Vec2::new(place * button_width, row * button_height);
        let rect = Rect::from_min_size(corner, Vec2::new(button_width, button_height));
        category_button(ui, rect, category, mode, &look, state, commands, &mut settings);
    }
    resize_menus(ui, column, mode, &mut editor.user.options, state);
    apply_settings(&mut editor.user.options, state, settings);
    state.toolbox_rect = column;
}

/// The toolbox's right edge (`GridSplitter`): dragged wider or narrower,
/// the menus switch between Long, Compact and Expanded (`ResizeToolbox`).
fn resize_menus(ui: &mut Ui, column: Rect, mode: ToolboxMode, options: &mut UserOptions, state: &mut GraphState) {
    let handle = Rect::from_min_size(Pos2::new(column.right(), column.top()), Vec2::new(SPLITTER, column.height()));
    let response = ui.interact(handle, ui.id().with("toolbox-splitter"), Sense::drag());
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if !response.dragged() {
        state.toolbox.resizing = None;
        return;
    }
    let travelled = state.toolbox.resizing.unwrap_or(0.0) + response.drag_delta().x;
    if travelled.abs() <= SPLITTER_STEP {
        state.toolbox.resizing = Some(travelled);
        return;
    }
    state.toolbox.resizing = Some(0.0);
    let width = column.width() + travelled;
    let next = match mode {
        ToolboxMode::Single if width > BUTTON_SIZE * 2.0 => ToolboxMode::Double,
        ToolboxMode::Double if width < BUTTON_SIZE => ToolboxMode::Single,
        ToolboxMode::Double if width > BUTTON_SIZE * 4.0 => ToolboxMode::Expanded,
        ToolboxMode::Expanded if width < BUTTON_SIZE * 2.0 => ToolboxMode::Double,
        _ => mode,
    };
    if next != mode {
        options.set_toolbox_mode(next);
    }
}

/// A `ToolboxMenuItem`: `MainRectangle` (margin 1, `ToolboxButton-Background`
/// at 0.8, corners 2), the category icon in its colour (centred, or in a
/// 28 column beside the name when expanded), the name in 10px semibold;
/// its menu opens to the right, its right-click menu holds the settings.
#[allow(clippy::too_many_arguments)]
fn category_button(ui: &mut Ui, rect: Rect, category: &str, mode: ToolboxMode, look: &EntryLook, state: &mut GraphState, commands: &mut Vec<Command>, settings: &mut Vec<SettingsChange>) {
    let response = ui.interact(rect, ui.id().with(("toolbox-category", category)), Sense::click());
    let popup_id = egui::Popup::default_response_id(&response);
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let hovered = response.hovered();
    let fill = if open {
        BUTTON_OPEN
    } else if hovered {
        BUTTON_HOVER
    } else {
        theme::TOOLBOX_BUTTON_BACKGROUND.gamma_multiply(0.8)
    };
    let body = rect.shrink(1.0);
    ui.painter().rect_filled(body, 2.0, fill);
    ui.painter().rect_stroke(body, 2.0, Stroke::new(1.0, theme::TOOLBOX_BUTTON_BACKGROUND), StrokeKind::Inside);
    let color = theme::toolbox_color(category);
    let icon = format!("toolbox-{}", category.to_lowercase());
    let expanded = mode == ToolboxMode::Expanded;
    let icon_center = if expanded { Pos2::new(rect.left() + ICON_COLUMN / 2.0, rect.center().y) } else { rect.center() };
    icons::paint(ui, Rect::from_center_size(icon_center, Vec2::splat(icons::DEFAULT_SIZE)), &icon, color);
    if expanded {
        let text_color = if hovered { theme::MENU_FOREGROUND_HOVER } else { theme::LABEL_DEFAULT };
        let galley = ui.painter().layout_no_wrap(category.to_string(), theme::semibold(10.0), text_color);
        ui.painter().galley(Pos2::new(rect.left() + ICON_COLUMN, rect.center().y - galley.size().y / 2.0), galley, text_color);
    }
    let response = response.on_hover_text(category);
    response.context_menu(|ui| settings_menu(ui, look.editor, settings));

    // The menu (`_SubMenuPopup`, placed right, margin 6).
    let info = UiStackInfo::new(UiKind::Menu).with_tag_value(MenuConfig::MENU_CONFIG_TAG, MenuConfig::default());
    let mut popup = egui::Popup::menu(&response);
    popup = popup.align(egui::RectAlign::RIGHT_START);
    popup = popup.gap(6.0);
    popup = popup.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    popup = popup.info(info);
    popup.show(|ui| category_menu(ui, category, look, Origin::Toolbox, state, commands));
}

/// The panel style (`GeneratePanelStyleToolbox`, on `Panel-Background`,
/// the area's height): node buttons in columns, under each category's
/// expander when categorized.
fn show_panel(ui: &mut Ui, area: Rect, editor: &mut Editor, state: &mut GraphState, commands: &mut Vec<Command>) {
    let options = &editor.user.options;
    let columns = options.toolbox_columns();
    let width = columns as f32 * PANEL_COLUMN;
    let panel = Rect::from_min_max(area.min, Pos2::new(area.left() + width + 10.0, area.bottom()));
    ui.painter().rect_filled(panel, 0.0, theme::PANEL_BACKGROUND);
    let look = EntryLook { editor, average: average_usage(editor) };
    let mut settings: Vec<SettingsChange> = Vec::new();
    let background = ui.interact(panel, ui.id().with("toolbox-panel"), Sense::click());
    background.context_menu(|ui| settings_menu(ui, look.editor, &mut settings));
    let inner = panel.shrink2(Vec2::new(5.0, 0.0));
    let builder = UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min));
    let mut panel_ui = ui.new_child(builder);
    panel_ui.set_clip_rect(panel);
    let categorized = look.editor.user.options.toolbox_categorized();
    ScrollArea::vertical().id_salt("toolbox-panel-scroll").max_height(inner.height()).show(&mut panel_ui, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        if categorized {
            for category in CATEGORIES {
                if category == "Macro" {
                    continue;
                }
                let open = !state.toolbox.collapsed.contains(category);
                ui.add_space(5.0);
                if expander_header(ui, category, open, &look, &mut settings) {
                    toggle(&mut state.toolbox.collapsed, category);
                }
                if open {
                    panel_grid(ui, &[category], columns, &look, state, commands, &mut settings);
                }
                ui.add_space(5.0);
            }
        } else {
            let categories: Vec<&str> = CATEGORIES.iter().copied().filter(|category| *category != "Macro").collect();
            panel_grid(ui, &categories, columns, &look, state, commands, &mut settings);
        }
    });
    resize_panel(ui, panel, &mut editor.user.options, state);
    apply_settings(&mut editor.user.options, state, settings);
    state.toolbox_rect = panel;
}

fn toggle(set: &mut HashSet<&'static str>, category: &'static str) {
    if !set.remove(category) {
        set.insert(category);
    }
}

/// `Expander-Toolbox`: the chevron (grey, `Accent-Gray` under the
/// pointer) and the category's name. Returns whether it was clicked.
fn expander_header(ui: &mut Ui, category: &str, open: bool, look: &EntryLook, settings: &mut Vec<SettingsChange>) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), Sense::click());
    let chevron = if open { "chevron-down" } else { "chevron-right" };
    let tint = if response.hovered() { theme::ACCENT_GRAY } else { theme::EXPANDER_HEADER };
    let icon_rect = Rect::from_min_size(Pos2::new(rect.left() + 2.0, rect.center().y - 8.0), Vec2::splat(icons::DEFAULT_SIZE));
    icons::paint(ui, icon_rect, chevron, tint);
    let galley = ui.painter().layout_no_wrap(category.to_string(), theme::semibold(11.0), theme::LABEL_DEFAULT);
    ui.painter().galley(Pos2::new(rect.left() + 22.0, rect.center().y - galley.size().y / 2.0), galley, theme::LABEL_DEFAULT);
    response.context_menu(|ui| settings_menu(ui, look.editor, settings));
    response.clicked()
}

/// `AddCategoryButtonsToPanel` for some categories, in a uniform grid:
/// each node a subtle button (10px, the category's colour, centred or
/// left), margin 2 and padding 6 (compact: 1 and 4, 2); highlighting, the
/// little used at 0.75, the rest semibold.
#[allow(clippy::too_many_arguments)]
fn panel_grid(ui: &mut Ui, categories: &[&str], columns: usize, look: &EntryLook, state: &mut GraphState, commands: &mut Vec<Command>, settings: &mut Vec<SettingsChange>) {
    let options = &look.editor.user.options;
    let compact = options.toolbox_compact();
    let left_aligned = options.toolbox_left_align();
    let highlight = options.toolbox_highlight_high_use();
    let margin = if compact { 1.0 } else { 2.0 };
    let padding = if compact { Vec2::new(4.0, 2.0) } else { Vec2::splat(6.0) };
    let usage = |name: &str| look.usage(name);
    let mut entries: Vec<(&'static NodeSpec, Color32)> = Vec::new();
    for category in categories {
        let color = theme::toolbox_color(category);
        for spec in category_nodes(category, options, &usage) {
            entries.push((spec, color));
        }
    }
    let line_height = ui.painter().layout_no_wrap("Mountain".to_string(), theme::interface(10.0), Color32::WHITE).size().y;
    let cell_height = line_height + 2.0 * padding.y + 2.0 * margin;
    let rows = entries.len().div_ceil(columns.max(1));
    let (grid, _) = ui.allocate_exact_size(Vec2::new(columns as f32 * PANEL_COLUMN, rows as f32 * cell_height), Sense::hover());
    for (index, (spec, color)) in entries.into_iter().enumerate() {
        let column = (index % columns) as f32;
        let row = (index / columns) as f32;
        let cell = Rect::from_min_size(grid.min + Vec2::new(column * PANEL_COLUMN, row * cell_height), Vec2::new(PANEL_COLUMN, cell_height));
        let button = cell.shrink(margin);
        let implemented = registry().implements(spec.key());
        let sense = if implemented { Sense::click_and_drag() } else { Sense::hover() };
        let response = ui.interact(button, ui.id().with(("toolbox-node", spec.key())), sense);
        let fill = if response.is_pointer_button_down_on() {
            theme::BUTTON_PRESSED
        } else if response.hovered() {
            theme::BUTTON_HOVER
        } else {
            theme::BUTTON_BACKGROUND
        };
        ui.painter().rect_filled(button, 2.0, fill);
        let faint = highlight && (look.usage(spec.key()) as f64) < look.average;
        let mut opacity = if faint { 0.75 } else { 1.0 };
        if !implemented {
            opacity *= 0.5;
        }
        let font = if highlight && !faint { theme::semibold(10.0) } else { theme::interface(10.0) };
        let text_color = color.gamma_multiply(opacity);
        let galley = crate::ui::graph_canvas::elided_galley(ui.painter(), node_label(spec), font, text_color, button.width() - 2.0 * padding.x);
        let x = if left_aligned { button.left() + padding.x } else { button.center().x - galley.size().x / 2.0 };
        ui.painter().galley(Pos2::new(x, button.center().y - galley.size().y / 2.0), galley, text_color);
        let mut response = match &spec.description {
            Some(description) => response.on_hover_text(description),
            None => response,
        };
        if !implemented {
            response = response.on_hover_text("Not available in Nerthus yet");
        }
        response.context_menu(|ui| settings_menu(ui, look.editor, settings));
        if response.drag_started_by(egui::PointerButton::Primary) {
            state.toolbox_drag = Some(spec.key().to_string());
        } else if response.clicked() {
            commands.push(Command::Graph(GraphCommand::ToolboxCreate(spec.key().to_string())));
        }
    }
}

/// The panel's right edge: its columns follow the width, one per 80, from
/// one to six (`ResizeToolbox`).
fn resize_panel(ui: &mut Ui, panel: Rect, options: &mut UserOptions, state: &mut GraphState) {
    let handle = Rect::from_min_size(Pos2::new(panel.right() - SPLITTER / 2.0, panel.top()), Vec2::new(SPLITTER, panel.height()));
    let response = ui.interact(handle, ui.id().with("toolbox-panel-splitter"), Sense::drag());
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if !response.dragged() {
        state.toolbox.resizing = None;
        return;
    }
    let travelled = state.toolbox.resizing.unwrap_or(0.0) + response.drag_delta().x;
    if travelled.abs() <= SPLITTER_STEP {
        state.toolbox.resizing = Some(travelled);
        return;
    }
    state.toolbox.resizing = Some(0.0);
    let width = options.toolbox_columns() as f32 * PANEL_COLUMN + travelled;
    let columns = ((width / PANEL_COLUMN) as usize).clamp(1, 6);
    options.set_toolbox_columns(columns);
}

/// A setting the toolbox's menu changed.
enum SettingsChange {
    Style(ToolboxStyle),
    Nested(bool),
    Mode(ToolboxMode),
    ExpandAll(bool),
    Compact(bool),
    Categorized(bool),
    LeftAlign(bool),
    Pin(bool),
    Highlight(bool),
}

/// `RecreateContextMenu`: the style; for menus the list style and size,
/// for the panel its options; then the high use items.
fn settings_menu(ui: &mut Ui, editor: &Editor, settings: &mut Vec<SettingsChange>) {
    menu_style(ui);
    let options = &editor.user.options;
    let style = options.toolbox_style();
    menu_caption(ui, "Style");
    if checked_icon_item(ui, style == ToolboxStyle::Panel, Some("toolbox-panel"), None, "Panel", "") {
        settings.push(SettingsChange::Style(ToolboxStyle::Panel));
    }
    if checked_icon_item(ui, style == ToolboxStyle::Menus, Some("toolbox-menus"), None, "Menus", "") {
        settings.push(SettingsChange::Style(ToolboxStyle::Menus));
    }
    match style {
        ToolboxStyle::Menus => {
            menu_separator(ui);
            menu_caption(ui, "Menu Style");
            let nested = options.toolbox_nested();
            if checked_icon_item(ui, !nested, Some("list"), None, "Flat List", "") {
                settings.push(SettingsChange::Nested(false));
            }
            if checked_icon_item(ui, nested, Some("list-tree"), None, "Nested", "") {
                settings.push(SettingsChange::Nested(true));
            }
            menu_separator(ui);
            menu_caption(ui, "Toolbox Size");
            let mode = options.toolbox_mode();
            let sizes = [(ToolboxMode::Single, "Long"), (ToolboxMode::Double, "Compact"), (ToolboxMode::Expanded, "Expanded")];
            for (size, text) in sizes {
                if check_item(ui, mode == size, text, "").clicked() {
                    settings.push(SettingsChange::Mode(size));
                }
            }
        }
        ToolboxStyle::Panel => {
            let categorized = options.toolbox_categorized();
            if categorized {
                menu_separator(ui);
                if menu_item(ui, None, None, "Expand All", "").clicked() {
                    settings.push(SettingsChange::ExpandAll(true));
                }
                if menu_item(ui, None, None, "Collapse All", "").clicked() {
                    settings.push(SettingsChange::ExpandAll(false));
                }
            }
            menu_separator(ui);
            menu_caption(ui, "Options");
            let compact = options.toolbox_compact();
            if check_item(ui, compact, "Compact Mode", "").clicked() {
                settings.push(SettingsChange::Compact(!compact));
            }
            if check_item(ui, categorized, "Categorized", "").clicked() {
                settings.push(SettingsChange::Categorized(!categorized));
            }
            let left_aligned = options.toolbox_left_align();
            if check_item(ui, left_aligned, "Align Left", "").clicked() {
                settings.push(SettingsChange::LeftAlign(!left_aligned));
            }
        }
    }
    menu_separator(ui);
    menu_caption(ui, "High Use Items");
    let pinned = options.toolbox_pin_high_use();
    if check_item(ui, pinned, "Pin To Top", "").clicked() {
        settings.push(SettingsChange::Pin(!pinned));
    }
    let highlighted = options.toolbox_highlight_high_use();
    if check_item(ui, highlighted, "Highlight", "").clicked() {
        settings.push(SettingsChange::Highlight(!highlighted));
    }
}

fn apply_settings(options: &mut UserOptions, state: &mut GraphState, settings: Vec<SettingsChange>) {
    for change in settings {
        match change {
            SettingsChange::Style(style) => options.set_toolbox_style(style),
            SettingsChange::Nested(nested) => options.set_toolbox_nested(nested),
            SettingsChange::Mode(mode) => options.set_toolbox_mode(mode),
            SettingsChange::ExpandAll(true) => state.toolbox.collapsed.clear(),
            SettingsChange::ExpandAll(false) => state.toolbox.collapsed = CATEGORIES.iter().copied().collect(),
            SettingsChange::Compact(on) => options.set_toolbox_compact(on),
            SettingsChange::Categorized(on) => options.set_toolbox_categorized(on),
            SettingsChange::LeftAlign(on) => options.set_toolbox_left_align(on),
            SettingsChange::Pin(on) => options.set_toolbox_pin_high_use(on),
            SettingsChange::Highlight(on) => options.set_toolbox_highlight_high_use(on),
        }
    }
}
