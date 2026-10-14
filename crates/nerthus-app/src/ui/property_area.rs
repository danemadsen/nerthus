//! The property area (`PropertyArea`): the selected node's header and
//! menu, its commands, its parameters in group panels (`EditorBase`
//! expanders) and the modifier toolbar at the bottom, laid out at the size
//! theme's sizes (`Size.Default.xaml`, or `Size.Compact.xaml` with Compact
//! Interface).

use egui::{Align, Color32, Layout, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2};
use nerthus_engine::json::f32_value;
use nerthus_engine::{Catalogue, ParameterSpec, StateOverride};
use serde_json::Value;

use crate::editor::{DialogRequest, Editor};
use crate::icons;
use crate::node_ui::node_ui;
use crate::theme;
use crate::ui::bound_variable::{self, Bound};
use crate::ui::commands::Command;
use crate::ui::editors::{self, palette_color, Edit, EditorContext};
use crate::ui::graph_canvas::{node_color, node_icon};
use crate::ui::modifier_list::modifier_list;
use crate::ui::node_states::{self, StateAction};
use crate::ui::tool_commands::ToolCommand;
use crate::ui::widgets::{check_item, icon_menu, icon_menu_sized, link_button, menu_caption, menu_item, menu_item_enabled, menu_separator, submenu, toolbar_background};
use nerthus_engine::automation::VarType;

/// The modifier toolbar's `WrapPanel` margin.
const MODIFIER_PANEL_MARGIN: f32 = 4.0;
/// `ToggleButton-Expander-DownHeader`'s first column: the chevron's.
const GROUP_CHEVRON_COLUMN: f32 = 26.0;
/// `EditorBase.CONTROL_THICKNESS`: every editor row's margin.
const ROW_MARGIN: i8 = 4;
/// The parameter menu (`EditorMenu`): a 12px keyframe 4 from the editor.
const MENU_BUTTON_WIDTH: f32 = 16.0;
/// The editor scroller's right padding.
const SCROLL_PADDING: f32 = 8.0;
/// `Modifier-Toolbar` borders: padding and margin 4 around 24px toggles.
const GROUP_PADDING: f32 = 4.0;
const GROUP_MARGIN: f32 = 4.0;
const MODIFIER_BUTTON: f32 = 24.0;
/// `btnAddModifier`: 32 by 24.
const ADD_BUTTON_WIDTH: f32 = 32.0;
/// The Gate's and a baked node's panels, remembered apart from the
/// parameters' groups.
const GATE_PANEL_KEY: usize = usize::MAX - 1;
const BAKED_PANEL_KEY: usize = usize::MAX - 2;
/// Their buttons' padding and margin (16 across, 8 down).
const PANEL_BUTTON_PADDING: f32 = 8.0;
const PANEL_BUTTON_MARGIN: Vec2 = Vec2::new(16.0, 8.0);

/// The modifier toggles: icon, tooltip, modifier type, their gaps (left
/// margins in `ModifierStack`) and accent (`Accent-Muted-Green`, else the
/// style's `Accent-Muted-Blue`).
struct ModifierToggle {
    icon: &'static str,
    tooltip: &'static str,
    type_name: &'static str,
    gap: f32,
    accent: Color32,
}

const fn modifier_toggle(icon: &'static str, tooltip: &'static str, type_name: &'static str, gap: f32, accent: Color32) -> ModifierToggle {
    ModifierToggle { icon, tooltip, type_name, gap, accent }
}

const MODIFIER_TOGGLES: [&[ModifierToggle]; 3] = [
    &[
        modifier_toggle("x-autolevel", "Autolevel", "Autolevel", 0.0, theme::ACCENT_MUTED_GREEN),
        modifier_toggle("x-shaper", "Shaper", "Shaper", 4.0, theme::ACCENT_MUTED_GREEN),
        modifier_toggle("ruler-measure-2", "Height Remap", "Multiplier", 4.0, theme::ACCENT_MUTED_GREEN),
    ],
    &[
        modifier_toggle("x-drop", "Drop", "Drop", 2.0, theme::ACCENT_MUTED_GREEN),
        modifier_toggle("x-invert", "Invert", "Invert", 0.0, theme::ACCENT_MUTED_GREEN),
    ],
    &[
        modifier_toggle("minimize", "Min", "Min", 4.0, theme::ACCENT_MUTED_BLUE),
        modifier_toggle("maximize", "Max", "Max", 4.0, theme::ACCENT_MUTED_BLUE),
    ],
];

pub fn show(ui: &mut Ui, rect: Rect, editor: &mut Editor, commands: &mut Vec<Command>) {
    crate::ui::presets::show_create_window(ui.ctx(), editor);
    crate::ui::math_editor::show(ui.ctx(), editor);
    crate::ui::painter::show(ui.ctx(), editor);
    ui.painter().rect_filled(rect, 4.0, theme::PANEL_BACKGROUND);
    // `PropertyBorder`: a `Toolbar` (margin `Toolbar.Margin`, at least
    // `Toolbar.MinHeight` high).
    let sizes = theme::sizes(ui.ctx());
    let header_margin = sizes.toolbar_margin;
    let header = Rect::from_min_size(rect.min + Vec2::splat(header_margin), Vec2::new(rect.width() - 2.0 * header_margin, sizes.toolbar_min_height));
    let mut actions: Vec<StateAction> = Vec::new();
    header_toolbar(ui, header, editor, commands, &mut actions);
    if node_states::node_changed(ui, editor, editor.active_node()) {
        editor.tiled_preview = false;
    }

    let Some(node_id) = editor.active_node() else {
        return;
    };
    if editor.document.node(node_id).is_none() {
        return;
    }
    node_states::note_shown_node(ui, editor, node_id);

    let mut body_top = header.bottom() + header_margin;
    if let Some(row) = command_row(ui, rect, body_top, editor, node_id, commands) {
        body_top = row.bottom() + sizes.property_border[3];
    }
    let error = editor.status(node_id).error;
    if let Some(error) = error {
        body_top = warning_row(ui, rect, body_top, &error);
    }

    let modifiers_width = rect.width() - 2.0 * MODIFIER_PANEL_MARGIN;
    let measure = Rect::from_min_size(Pos2::ZERO, Vec2::new(modifiers_width, 0.0));
    let (_, modifiers_height) = layout_modifier_groups(measure);
    let modifiers_top = rect.bottom() - MODIFIER_PANEL_MARGIN - modifiers_height;
    let modifiers_rect = Rect::from_min_max(Pos2::new(rect.left() + MODIFIER_PANEL_MARGIN, modifiers_top), rect.max - Vec2::splat(MODIFIER_PANEL_MARGIN));
    let mut body = Rect::from_min_max(Pos2::new(rect.left(), body_top), Pos2::new(rect.right(), modifiers_rect.top()));
    if let Some(alert) = descendant_alert(editor, node_id) {
        let alert_rect = Rect::from_min_size(body.min + Vec2::new(16.0, 4.0), Vec2::new(body.width() - 32.0, 40.0));
        notice(ui, alert_rect, alert, theme::ACCENT_TRANSPARENT_ORANGE, theme::ACCENT_STRONG_ORANGE);
        body.min.y = alert_rect.bottom() + 4.0;
    }
    let can_add_ports = editor.document.node(node_id).is_some_and(|node| nerthus_engine::editing::can_add_ports(&node.type_name));
    let port_note_id = ui.id().with("port-note-dismissed");
    let port_note_dismissed: bool = ui.data(|data| data.get_temp(port_note_id)).unwrap_or(false);
    if can_add_ports && !port_note_dismissed {
        let note_rect = Rect::from_min_max(Pos2::new(body.left() + 16.0, body.bottom() - 4.0 - 30.0), Pos2::new(body.right() - 16.0, body.bottom() - 4.0));
        match port_note(ui, note_rect) {
            Some(PortNoteClick::Dismiss) => {
                ui.data_mut(|data| data.insert_temp(port_note_id, true));
            }
            Some(PortNoteClick::MoreInfo) => {
                commands.push(Command::Tools(ToolCommand::ShowDocs(crate::docs::ADD_PORTS.to_string())));
            }
            None => {}
        }
        body.max.y = note_rect.top() - 4.0;
    }
    let needs_height = editor.document.node(node_id).is_some_and(node_states::needs_height);
    if needs_height {
        let note_rect = Rect::from_min_size(body.min + Vec2::new(16.0, 4.0), Vec2::new(body.width() - 32.0, 26.0));
        if height_note(ui, note_rect) {
            actions.push(StateAction::ConnectHeight);
        }
        body.min.y = note_rect.bottom() + 4.0;
    }

    let builder = UiBuilder::new().max_rect(body).layout(Layout::top_down(Align::Min));
    let mut body_ui = ui.new_child(builder);
    let mut scroll = ScrollArea::vertical();
    scroll = scroll.id_salt(("properties", node_id));
    scroll = scroll.auto_shrink([false, false]);
    let area_width = rect.width();
    scroll.show(&mut body_ui, |ui| {
        let content_width = (ui.available_width() - SCROLL_PADDING).max(0.0);
        ui.set_width(content_width);
        parameters(ui, editor, node_id, area_width, commands);
    });

    modifier_toolbar(ui, modifiers_rect, editor, node_id, commands, &mut actions);
    parameter_dialogs(ui, editor);
    for action in actions {
        node_states::apply(ui, editor, node_id, action, commands);
    }
}

/// The command row (`CommandBorder`, a toolbar with the margins
/// `Property.Border.Thickness`, `top` being under the header's): a button
/// per command parameter the node shows (`ToolbarButton-IconXText`, icons
/// in `ParamsA-Color1..4`), then "Toggle Gizmo" for nodes with a
/// `[Gizmo]`.
fn command_row(ui: &mut Ui, area: Rect, top: f32, editor: &mut Editor, node_id: i32, commands: &mut Vec<Command>) -> Option<Rect> {
    let node = editor.document.node(node_id)?;
    let spec = Catalogue::get().node(&node.type_name)?;
    let asset = &editor.document.project.assets[0];
    let rules = node_ui(node, &asset.terrain.nodes, &asset.automation);
    let entries: Vec<&ParameterSpec> = spec.parameters.iter().filter(|parameter| parameter.kind == "Command" && !parameter.hidden && rules.visible(&parameter.name)).collect();
    let has_gizmo = spec.gizmo.is_some();
    if entries.is_empty() && !has_gizmo {
        return None;
    }
    let sizes = theme::sizes(ui.ctx());
    let [left, above, right, _] = sizes.property_border;
    let row = Rect::from_min_size(Pos2::new(area.left() + left, top + above), Vec2::new(area.width() - left - right, sizes.toolbar_min_height));
    toolbar_background(ui, row, theme::PANEL_BORDER, 1.0);
    // `Toolbar.Padding` inside the 1px border.
    let [padding_across, padding_down] = sizes.toolbar_padding;
    let inner = row.shrink(1.0).shrink2(Vec2::new(padding_across, padding_down));
    let builder = UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center));
    let mut bar = ui.new_child(builder);
    bar.spacing_mut().item_spacing.x = 0.0;
    const COMMAND_COLORS: [u32; 4] = [0xe4ba4a, 0xa9b871, 0x6eb699, 0xe4ba4a];
    for (index, parameter) in entries.iter().enumerate() {
        let icon = parameter.command.as_ref().and_then(Value::as_str).unwrap_or("square-rounded");
        // The `[Name]`, else the property name humanized (sentence case),
        // which is also the tooltip.
        let label = parameter.label.clone().unwrap_or_else(|| nerthus_engine::humanize::humanize(&parameter.name));
        let color_value = COMMAND_COLORS[(index + 1) % 4];
        let color = Color32::from_rgb((color_value >> 16) as u8, (color_value >> 8) as u8, color_value as u8);
        let response = icon_text_button(&mut bar, icon, &label, color, None);
        let response = response.on_hover_text(&label);
        if response.clicked() {
            open_command_window(ui, editor, node_id, &parameter.name, commands);
        }
    }
    if has_gizmo {
        let response = icon_text_button(&mut bar, "gizmo", "Toggle Gizmo", theme::ACCENT_MUTED_PURPLE, Some(editor.gizmo_active));
        if response.on_hover_text("Toggle Gizmo").clicked() {
            editor.gizmo_active = !editor.gizmo_active;
        }
    }
    Some(row)
}

/// A command button clicked: the commands that open a window of their own
/// (`OpenEditor` for Math), else the node command.
fn open_command_window(ui: &Ui, editor: &Editor, node_id: i32, name: &str, commands: &mut Vec<Command>) {
    let type_name = editor.document.node(node_id).map(|node| node.type_name.as_str()).unwrap_or_default();
    match (type_name, name) {
        ("MathX", "OpenEditor") => crate::ui::math_editor::open(ui.ctx(), editor, node_id),
        ("Draw", "OpenPainter") => crate::ui::painter::open(ui.ctx(), editor, node_id),
        ("Construction", "OpenGodMode") => commands.push(Command::OpenConstructor(node_id)),
        _ => commands.push(Command::NodeAction(name.to_string())),
    }
}

/// `ToolbarButton-IconXText` (and `ToggleButton-IconXText` when `checked`
/// is given): `Toolbar.ButtonSize` high, the icon 8 in, the text 30 in.
fn icon_text_button(ui: &mut Ui, icon: &str, text: &str, accent: Color32, checked: Option<bool>) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::interface(12.0), theme::LABEL_DEFAULT);
    let width = 30.0 + galley.size().x + 8.0;
    let height = theme::sizes(ui.ctx()).toolbar_button;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let hovered = response.hovered();
    let pressed = response.is_pointer_button_down_on();
    let on = checked == Some(true);
    let fill = if pressed || (on && !hovered) {
        Some(theme::BUTTON_PRESSED)
    } else if hovered {
        Some(theme::BUTTON_HOVER)
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, 4.0, fill);
    }
    if on {
        ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.5, accent), StrokeKind::Inside);
    }
    let icon_color = if hovered || on { theme::LABEL_EMPHASIS } else { accent };
    let icon_rect = Rect::from_min_size(Pos2::new(rect.left() + 8.0, rect.center().y - 8.0), Vec2::splat(icons::DEFAULT_SIZE));
    icons::paint(ui, icon_rect, icon, icon_color);
    ui.painter().galley(Pos2::new(rect.left() + 30.0, rect.center().y - galley.size().y / 2.0), galley, theme::LABEL_DEFAULT);
    response
}

/// `WarningBorder`: the node's error in red, a toolbar (margins
/// `Property.Border.Thickness`, padding `Toolbar.Padding`) with the text
/// `Property.Border.Padding` in.
fn warning_row(ui: &mut Ui, area: Rect, top: f32, error: &str) -> f32 {
    let sizes = theme::sizes(ui.ctx());
    let [left, above, right, below] = sizes.property_border;
    let width = area.width() - left - right;
    let text_inset = sizes.toolbar_padding[0] + sizes.property_padding[0];
    let galley = ui.painter().layout(error.to_string(), theme::interface(12.0), theme::ACCENT_RED, width - 2.0 - 2.0 * text_inset);
    let height = (galley.size().y + 2.0 + 2.0 * sizes.property_padding[1] + 2.0 * sizes.toolbar_padding[1]).max(sizes.toolbar_min_height);
    let row = Rect::from_min_size(Pos2::new(area.left() + left, top + above), Vec2::new(width, height));
    toolbar_background(ui, row, theme::PANEL_BORDER, 1.0);
    let text_top = row.center().y - galley.size().y / 2.0;
    ui.painter().galley(Pos2::new(row.left() + 1.0 + text_inset, text_top), galley, theme::ACCENT_RED);
    row.bottom() + below
}

/// `DescendantAlert`: the preview is locked to a node this one does not
/// feed.
fn descendant_alert(editor: &Editor, node_id: i32) -> Option<&'static str> {
    if !editor.preview_locked {
        return None;
    }
    let locked = editor.preview_node?;
    let upstream = nerthus_engine::Engine::build_order(editor.document.terrain(), locked);
    if upstream.contains(&node_id) {
        return None;
    }
    Some("This node is not a descendant of the LOCKED node.\nChanges will not show in the current preview.")
}

/// A tinted note box (`Accent-Transparent-…` fill, `Accent-Strong-…` border).
fn notice(ui: &mut Ui, rect: Rect, text: &str, fill: egui::Color32, border: egui::Color32) {
    ui.painter().rect_filled(rect, 4.0, fill);
    ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.5, border), egui::StrokeKind::Inside);
    let mut job = egui::text::LayoutJob::simple(text.to_string(), theme::interface(10.0), theme::LABEL_EMPHASIS, rect.width() - 12.0);
    job.halign = Align::Center;
    let galley = ui.painter().layout_job(job);
    let position = Pos2::new(rect.center().x, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(position, galley, theme::LABEL_EMPHASIS);
}

/// What was clicked on the note that a node can add inputs.
enum PortNoteClick {
    Dismiss,
    MoreInfo,
}

/// `InfoPortAdd`: the node can add inputs; its close button dismisses
/// it, "More Info" opens the documentation about adding ports.
fn port_note(ui: &mut Ui, rect: Rect) -> Option<PortNoteClick> {
    ui.painter().rect_filled(rect, 4.0, theme::ACCENT_TRANSPARENT_BLUE);
    ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.5, theme::ACCENT_STRONG_BLUE), egui::StrokeKind::Inside);
    let builder = UiBuilder::new().max_rect(rect.shrink2(Vec2::new(6.0, 2.0))).layout(Layout::left_to_right(Align::Center));
    let mut row = ui.new_child(builder);
    let text = RichText::new("This node can add additional input ports.").font(theme::interface(10.0)).color(theme::LABEL_EMPHASIS);
    row.label(text);
    let mut clicked = None;
    row.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let (close_rect, close) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
        let color = if close.hovered() { theme::LABEL_EMPHASIS } else { theme::BUTTON_BACKGROUND };
        icons::paint(ui, close_rect.shrink(2.0), "x", color);
        if close.clicked() {
            clicked = Some(PortNoteClick::Dismiss);
        }
        // `Button-Link`, padding 2, 4 before the close button.
        ui.add_space(4.0);
        if link_button(ui, "More Info", theme::interface(12.0), Vec2::splat(2.0)).clicked() {
            clicked = Some(PortNoteClick::MoreInfo);
        }
    });
    clicked
}

/// `InfoHeightRef`: a green note that a Height connection is needed, with
/// its "Connect for me!" link (10 semibold, `Accent-Bright`) on the right.
/// True when the link is clicked.
fn height_note(ui: &mut Ui, rect: Rect) -> bool {
    ui.painter().rect_filled(rect, 4.0, theme::ACCENT_TRANSPARENT_GREEN);
    ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.5, theme::ACCENT_STRONG_GREEN), egui::StrokeKind::Inside);
    let builder = UiBuilder::new().max_rect(rect.shrink2(Vec2::new(6.0, 2.0))).layout(Layout::left_to_right(Align::Center));
    let mut row = ui.new_child(builder);
    let text = RichText::new("A Height connection is required.").font(theme::interface(9.0)).color(theme::LABEL_EMPHASIS);
    row.label(text);
    let mut clicked = false;
    row.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let link_galley = ui.painter().layout_no_wrap("Connect for me!".to_string(), theme::semibold(10.0), theme::ACCENT_BRIGHT);
        let (link_rect, link) = ui.allocate_exact_size(link_galley.size() + Vec2::new(8.0, 4.0), Sense::click());
        let color = if link.hovered() { theme::LABEL_EMPHASIS } else { theme::ACCENT_BRIGHT };
        ui.painter().galley(link_rect.min + Vec2::new(4.0, 2.0), link_galley, color);
        if link.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        clicked = link.clicked();
    });
    clicked
}

/// The header toolbar (`PropertyBorder`): the node's icon and name (the
/// family menu, 11px, with its corner arrow), then the presets and node
/// menus on the right.
fn header_toolbar(ui: &mut Ui, rect: Rect, editor: &Editor, commands: &mut Vec<Command>, actions: &mut Vec<StateAction>) {
    toolbar_background(ui, rect, theme::PANEL_BORDER, 1.0);
    let sizes = theme::sizes(ui.ctx());
    let [padding_across, padding_down] = sizes.toolbar_padding;
    let inner = rect.shrink(1.0).shrink2(Vec2::new(padding_across, padding_down));
    let node = editor.active_node().and_then(|id| editor.document.node(id));

    // `mnuIcon` (margin 8, 0, 4, 0), `mnuText` (margin 4, 3, 4, 4) and the
    // 8px `chevron-down-right` (margin 8, 1, 1, 1) at the bottom.
    let (icon_name, icon_color, name) = match node {
        // `mnuText`: the name, and the type when the name is not it.
        Some(node) if node.name == node.type_name => (node_icon(&node.type_name), node_color(node), node.name.clone()),
        Some(node) => (node_icon(&node.type_name), node_color(node), format!("{} ({})", node.name, node.type_name)),
        None => ("square-rounded".to_string(), theme::ICON_DEFAULT, "(Select a node)".to_string()),
    };
    let icon_rect = Rect::from_min_size(Pos2::new(inner.left() + 8.0, inner.center().y - 8.0), Vec2::splat(icons::DEFAULT_SIZE));
    icons::paint(ui, icon_rect, &icon_name, icon_color);
    let name_galley = ui.painter().layout_no_wrap(name, theme::interface(11.0), theme::MENU_FOREGROUND);
    let name_rect = Rect::from_min_size(Pos2::new(icon_rect.right() + 8.0, inner.center().y - name_galley.size().y / 2.0), name_galley.size());
    ui.painter().galley(name_rect.min, name_galley, theme::MENU_FOREGROUND);
    let arrow_rect = Rect::from_min_size(Pos2::new(name_rect.right() + 4.0 + 8.0, inner.bottom() - 1.0 - 8.0 - 4.0), Vec2::splat(8.0));
    icons::paint(ui, arrow_rect, "chevron-down-right", theme::ICON_DEFAULT);
    if let Some(node) = node {
        let spec = Catalogue::get().node(&node.type_name);
        let family = spec.and_then(|spec| spec.family.clone()).unwrap_or_default();
        let description = spec.and_then(|spec| spec.description.clone()).unwrap_or_default();
        let name_area = Rect::from_min_max(icon_rect.min, Pos2::new(arrow_rect.right(), inner.bottom()));
        let response = ui.interact(name_area, ui.id().with("family-menu"), Sense::click());
        let response = response.on_hover_text(format!("{family}\n\n{description}"));
        crate::ui::graph_popups::family_menu(&response, editor, node, commands);
    }

    let has_node = node.is_some();
    let intent = node.and_then(|node| node.base.get("RenderIntentOverride")).and_then(Value::as_str).unwrap_or("Default").to_string();
    // Two icon menus docked right, each `Toolbar.ButtonSize` wide with
    // `Toolbar.MenuMargin` either side.
    let button = sizes.toolbar_button;
    let menu_margin = sizes.toolbar_menu_margin;
    let menu_rect = Rect::from_center_size(Pos2::new(inner.right() - (button / 2.0 + menu_margin), inner.center().y), Vec2::splat(button));
    let presets_rect = menu_rect.translate(Vec2::new(-(button + 2.0 * menu_margin), 0.0));
    let saved_state = node_states::has_saved_state(ui);
    let mut menu_ui = ui.new_child(UiBuilder::new().max_rect(menu_rect));
    icon_menu(&mut menu_ui, "menu", "Node", None, |ui| {
        menu_caption(ui, "Node State");
        state_items(ui, has_node, saved_state, commands, actions);
        menu_separator(ui);
        if menu_item(ui, None, None, "Bind all properties", "").clicked() && has_node {
            actions.push(StateAction::BindAll);
        }
        menu_separator(ui);
        menu_caption(ui, "Render Intent (Override)");
        for choice in ["Default", "Heightfield", "Mask"] {
            if check_item(ui, intent == choice, choice, "").clicked() && has_node {
                let static_choice: &'static str = match choice {
                    "Heightfield" => "Heightfield",
                    "Mask" => "Mask",
                    _ => "Default",
                };
                commands.push(Command::SetRenderIntent(static_choice));
            }
        }
        menu_separator(ui);
        if menu_item(ui, None, None, "Show Help for this Node", "").clicked() {
            commands.push(Command::Tools(ToolCommand::ShowNodeHelp));
        }
    });
    let mut presets_ui = ui.new_child(UiBuilder::new().max_rect(presets_rect));
    icon_menu(&mut presets_ui, "cube-sphere", "Presets", None, |ui| {
        crate::ui::presets::presets_menu(ui, editor, commands);
    });
}

/// One `EditorBase` expander's worth of parameters, with each editor's
/// accent.
struct ParameterGroup<'a> {
    title: String,
    /// Which `[Group]` started it (the parameter's index), for the
    /// expander's remembered state.
    key: usize,
    expanded: bool,
    entries: Vec<(&'a ParameterSpec, Color32)>,
}

/// The parameters as `EditorBase.CreateParameters` groups them: an
/// expander titled with the node type until the first `[Group]`, then one
/// per `[Group]`. Each `[Group]` moves the editor colours to the next
/// palette (`colorFlip`, Processors start one along) and each editor made
/// takes the next colour of it.
/// Also returns the palette the last group left (`colorFlip`), where the
/// modifiers carry on.
fn parameter_groups<'a>(spec: &'a nerthus_engine::NodeSpec) -> (Vec<ParameterGroup<'a>>, usize) {
    let processor = spec.classification.as_deref() == Some("Processor");
    let mut color_flip = if processor { 1 } else { 0 };
    let mut made = 0usize;
    let mut groups: Vec<ParameterGroup> = Vec::new();
    for (index, parameter) in spec.parameters.iter().enumerate() {
        if parameter.group_start {
            color_flip = (color_flip + 1) % 3;
            let group = ParameterGroup {
                title: parameter.group.clone().unwrap_or_default(),
                key: index,
                expanded: !parameter.group_collapsed,
                entries: Vec::new(),
            };
            groups.push(group);
        }
        if parameter.hidden {
            continue;
        }
        if groups.is_empty() {
            let group = ParameterGroup {
                title: spec.name.clone(),
                key: usize::MAX,
                expanded: true,
                entries: Vec::new(),
            };
            groups.push(group);
        }
        // `Command` parameters have no editor (they are the command row).
        if parameter.kind == "Command" {
            continue;
        }
        let color = palette_color(color_flip, made % 8);
        made += 1;
        if let Some(group) = groups.last_mut() {
            group.entries.push((parameter, color));
        }
    }
    (groups, color_flip)
}

/// The parameters in their group panels, then the modifiers; applies the
/// edits made. A Gate shows its bake panel instead, and a baked node its
/// own (`EditorBase.GenerateControls`).
fn parameters(ui: &mut Ui, editor: &mut Editor, node_id: i32, area_width: f32, commands: &mut Vec<Command>) {
    let Some(node) = editor.document.node(node_id) else {
        return;
    };
    let Some(spec) = Catalogue::get().node(&node.type_name) else {
        ui.label("Unknown node type");
        return;
    };
    ui.spacing_mut().item_spacing = Vec2::ZERO;
    let state = StateOverride::of(node);
    if node.type_name == "Gate" {
        gate_panel(ui, node_id, state, commands);
        return;
    }
    if state == StateOverride::Baked {
        baked_panel(ui, node_id, commands);
        return;
    }
    let asset = &editor.document.project.assets[0];
    let rules = node_ui(node, &asset.terrain.nodes, &asset.automation);
    let values = node.values.clone();
    let (groups, last_color_flip) = parameter_groups(spec);
    // What terrain-mark sliders mark: the node's own output as last built.
    let terrain_range = editor.status(node_id).range.map(|(lowest, highest)| (lowest as f64, highest as f64));
    // What a bound parameter's expression of its variable is worked out
    // against.
    let expression_context = nerthus_engine::automation::ExpressionContext {
        resolution: nerthus_engine::Resolution::square(editor.preview_resolution),
        terrain_height: asset.terrain.height,
    };
    let mut edits: Vec<(String, Edit)> = Vec::new();
    // Each seed box's id, for the seeds its dice draws.
    let mut seed_boxes: std::collections::HashMap<String, egui::Id> = std::collections::HashMap::new();
    for group in &groups {
        group_panel(ui, (node_id, group.key), &group.title, group.expanded, |ui| {
            for (parameter, color) in &group.entries {
                let shown = parameter.visible && rules.visible(&parameter.name);
                if !shown {
                    continue;
                }
                if parameter.separator {
                    separator(ui);
                }
                let computed = rules.texts.get(&parameter.name).map(|text| Value::String(text.clone()));
                let value = computed.or_else(|| values.get(&parameter.name).cloned()).unwrap_or(Value::Null);
                let id = ui.id().with(("parameter", node_id, &parameter.name));
                let enabled = parameter.enabled && rules.enabled(&parameter.name);
                let context = EditorContext {
                    color: *color,
                    default: spec.defaults.get(&parameter.name),
                    values: Some(&values),
                    area_width,
                    terrain_range,
                };
                take_launch_right_click(ui.ctx(), &parameter.name, id);
                // A bound parameter shows its variable, whatever its own
                // editor would be (`EditorControlFactory.GetControl` asks
                // first).
                let binding = asset.automation.bindings.iter().find(|binding| binding.node == node_id && binding.property == parameter.name);
                if let Some(binding) = binding {
                    let bound = Bound {
                        node: node_id,
                        parameter,
                        key: &binding.variable,
                        automation: &asset.automation,
                        context: &expression_context,
                    };
                    let tooltip = bound_variable::tooltip(parameter, &binding.variable);
                    let bind_type = bind_type(&asset.automation, node_id, &parameter.name);
                    let automated = Automated::of(&asset.automation, node_id, &parameter.name);
                    let edit = editor_row(ui, enabled, Some(&tooltip), bind_type, &automated, |ui| bound_variable::bound_variable(ui, id, &bound));
                    edits.push((parameter.name.clone(), edit));
                    continue;
                }
                // Labels stand alone (`EditorLabel`, `ComputedString`).
                let is_label = parameter.value_type == nerthus_engine::ValueType::Byte || parameter.kind == "ComputedString";
                if is_label {
                    let row = ui.add_enabled_ui(enabled, |ui| editors::parameter(ui, id, parameter, &value, &context));
                    if let Some(help) = &parameter.description {
                        row.response.on_hover_text(help);
                    }
                    edits.push((parameter.name.clone(), row.inner));
                    continue;
                }
                let custom_row = ui.scope(|ui| custom_editor(ui, id, parameter, &value, node, &asset.terrain.nodes));
                if let Some(custom) = custom_row.inner {
                    if let Some(help) = &parameter.description {
                        custom_row.response.on_hover_text(help);
                    }
                    edits.extend(custom);
                    continue;
                }
                if parameter.is_seed() {
                    seed_boxes.insert(parameter.name.clone(), id);
                }
                let bind_type = bind_type(&asset.automation, node_id, &parameter.name);
                let automated = Automated::of(&asset.automation, node_id, &parameter.name);
                // An option bar without a label is named by its tooltip
                // until the documentation gives it one.
                let unlabelled = parameter.kind == "HorizontalListNoLabel" && area_width > 300.0;
                let unlabelled_name = unlabelled.then_some(parameter.name.as_str());
                let help = parameter.description.as_deref().or(unlabelled_name);
                let edit = editor_row(ui, enabled, help, bind_type, &automated, |ui| editors::parameter(ui, id, parameter, &value, &context));
                edits.push((parameter.name.clone(), edit));
            }
        });
    }
    let modifier_edits = modifier_list(ui, node, node_id, last_color_flip, area_width);
    ui.add_space(16.0);

    if let Some((place, shift)) = modifier_edits.close {
        commands.push(Command::RemoveModifier { node: node_id, place, ask: !shift });
    }
    if let Some((from, to)) = modifier_edits.moved
        && editor.document.move_modifier(node_id, from, to)
    {
        editor.changed();
    }
    for (index, name, edit) in modifier_edits.edits {
        let Some(modifier) = editor.document.node(node_id).and_then(|data| data.modifiers.get(index)) else {
            continue;
        };
        let value = match edit {
            Edit::Set(value) => Some(value),
            Edit::Reset => Catalogue::get().modifier(&modifier.type_name).and_then(|spec| spec.defaults.get(&name).cloned()),
            _ => None,
        };
        if let Some(value) = value {
            editor.document.set_modifier_value(node_id, index, &name, value);
            editor.modifier_changed();
        }
    }

    for (name, edit) in edits {
        match edit {
            Edit::None => {}
            Edit::Set(value) => {
                editor.document.set_value(node_id, &name, value);
                editor.parameter_changed(node_id);
            }
            Edit::Reset => {
                // `EditorMenu.ResetProperty`: the type's default value.
                let default = spec.defaults.get(&name).cloned();
                if let Some(default) = default {
                    editor.document.set_value(node_id, &name, default);
                    editor.parameter_changed(node_id);
                }
            }
            Edit::Command => commands.push(Command::NodeAction(name)),
            Edit::BoundText(text) => {
                // `VariableEditor.EditablePopup_EditSuccess`: the text must
                // make a value of the parameter's type.
                let bindings = &editor.document.automation().bindings;
                let binding = bindings.iter().find(|binding| binding.node == node_id && binding.property == name);
                let key = binding.map(|binding| binding.variable.clone());
                let converts = spec.parameter(&name).is_some_and(|parameter| bound_variable::converts(parameter, &text));
                match key {
                    Some(key) if converts => {
                        editor.document.set_variable_text(&key, &text);
                        editor.changed();
                    }
                    Some(_) => {
                        let caption = "Invalid value".to_string();
                        let text = "The value was not valid for this property.".to_string();
                        commands.push(Command::ShowMessage { caption, text });
                    }
                    None => {}
                }
            }
            Edit::ShowVariables => commands.push(Command::ShowDataTab(crate::ui::data_view::DataTab::Vars)),
            Edit::ShowDocs(target) => commands.push(Command::Tools(ToolCommand::ShowDocs(target.to_string()))),
            Edit::AddInput => {
                if editor.document.add_input(node_id) {
                    editor.changed();
                }
            }
            Edit::BindNew => {
                editor.document.bind_new_variable(node_id, &name);
                editor.changed();
            }
            Edit::BindTo(variable) => {
                editor.document.bind_variable(node_id, &name, &variable);
                editor.changed();
            }
            Edit::AskUnbind => {
                ui.data_mut(|data| data.get_temp_mut_or_default::<ParameterDialogs>(dialogs_id()).unbind = Some((node_id, name.clone())));
            }
            Edit::Expression => commands.push(Command::EditExpression { node: node_id, property: name.clone() }),
            Edit::NewSeed => {
                // `XorShift64Star.Next(0, 65536)`.
                let seed = editor.document.random_seed_below(65536);
                editor.document.set_value(node_id, &name, Value::from(seed));
                editor.parameter_changed(node_id);
                if let Some(id) = seed_boxes.get(&name) {
                    editors::remember_drawn_seed(ui.ctx(), *id, seed as i64);
                }
            }
            Edit::SeedText(text) => {
                // `EditablePopup_EditSuccess`: the text as a formula, then
                // `Convert.ToInt32`.
                let terrain = editor.document.terrain();
                let context = nerthus_engine::automation::ExpressionContext {
                    resolution: nerthus_engine::Resolution::square(editor.preview_resolution),
                    terrain_height: terrain.height,
                };
                let typed = editor.document.automation().calculate_number(&text, &context);
                if let Some(seed) = typed.and_then(convert_to_int32) {
                    editor.document.set_value(node_id, &name, Value::from(seed));
                    editor.parameter_changed(node_id);
                }
            }
            Edit::Browse => {
                let folder = spec.parameter(&name).is_some_and(|parameter| parameter.kind == "Folder");
                let extensions = file_extensions(&spec.name);
                let current = values.get(&name).and_then(Value::as_str).unwrap_or_default().to_string();
                editor.dialog = Some(DialogRequest::BrowseFile { node: node_id, parameter: name, extensions, folder, current });
            }
        }
    }
}

fn launch_right_click_id() -> egui::Id {
    egui::Id::new("right-click-at-launch")
}

/// `--show slider-popup:<property>:<along>`: the shown node's slider for
/// the property is right-clicked that share along its width (0.5 if not
/// given) when it next draws.
pub fn right_click_at_launch(ctx: &egui::Context, request: &str) {
    let (property, along) = request.split_once(':').unwrap_or((request, "0.5"));
    let along: f32 = along.parse().unwrap_or(0.5);
    ctx.data_mut(|data| data.insert_temp(launch_right_click_id(), (property.to_string(), along)));
}

/// Hands a launch right-click to the slider `id` when it is for `property`.
fn take_launch_right_click(ctx: &egui::Context, property: &str, id: egui::Id) {
    let request: Option<(String, f32)> = ctx.data(|data| data.get_temp(launch_right_click_id()));
    if let Some((wanted, along)) = request
        && wanted.eq_ignore_ascii_case(property)
    {
        ctx.data_mut(|data| data.remove::<(String, f32)>(launch_right_click_id()));
        crate::ui::slider_popups::request_right_click(ctx, id, along);
    }
}

/// `CreateBakeNodeUI`: a Gate shows a "Gate Node" panel and nothing else:
/// Bake Node, or Unbake Node once it is baked.
fn gate_panel(ui: &mut Ui, node_id: i32, state: StateOverride, commands: &mut Vec<Command>) {
    group_panel(ui, (node_id, GATE_PANEL_KEY), "Gate Node", true, |ui| match state {
        StateOverride::None => {
            if panel_button(ui, "Bake Node") {
                commands.push(Command::BakeSelected);
            }
        }
        StateOverride::Baked => {
            if panel_button(ui, "Unbake Node") {
                commands.push(Command::UnbakeNode(node_id));
            }
        }
        StateOverride::Bypassed => {}
    });
}

/// `CreateBakedUI`: a baked node shows a "Baked Node" panel in place of
/// its parameters and modifiers, with Load Offloaded Cache.
fn baked_panel(ui: &mut Ui, node_id: i32, commands: &mut Vec<Command>) {
    group_panel(ui, (node_id, BAKED_PANEL_KEY), "Baked Node", true, |ui| {
        if panel_button(ui, "Load Offloaded Cache") {
            commands.push(Command::LoadOffloadedCache(node_id));
        }
    });
}

/// A panel's `Button` (padding 8, margin 16, 8, 16, 8), as wide as the
/// panel: `Button-Background` (darker under the pointer and while
/// pressed) in a 1px `Button-Border`, corners of 2.
fn panel_button(ui: &mut Ui, text: &str) -> bool {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::interface(12.0), theme::LABEL_DEFAULT);
    let height = galley.size().y + 2.0 * PANEL_BUTTON_PADDING + 2.0 * PANEL_BUTTON_MARGIN.y;
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let button = row.shrink2(PANEL_BUTTON_MARGIN);
    let response = ui.interact(button, ui.id().with(("panel-button", text)), Sense::click());
    let fill = if response.is_pointer_button_down_on() {
        theme::BUTTON_PRESSED
    } else if response.hovered() {
        theme::BUTTON_HOVER
    } else {
        theme::BUTTON_BACKGROUND
    };
    ui.painter().rect_filled(button, 2.0, fill);
    ui.painter().rect_stroke(button, 2.0, Stroke::new(1.0, theme::BUTTON_BORDER), StrokeKind::Inside);
    let text_position = button.center() - galley.size() / 2.0;
    ui.painter().galley(text_position, galley, theme::LABEL_DEFAULT);
    response.clicked()
}

/// `Convert.ToInt32(float)`: the nearest integer, halves to even; `None`
/// where .NET throws (out of range, NaN).
fn convert_to_int32(value: f32) -> Option<i32> {
    let value = value as f64;
    let in_range = (-2147483648.5..2147483647.5).contains(&value);
    if !in_range {
        return None;
    }
    Some(value.round_ties_even() as i32)
}

/// An editor row (`DockPanel`, margin 4): the editor, then its parameter
/// menu docked right and centred down the row.
fn editor_row(ui: &mut Ui, enabled: bool, help: Option<&str>, bind_type: BindType, automated: &Automated, control: impl FnOnce(&mut Ui) -> Edit) -> Edit {
    let mut frame = egui::Frame::new();
    frame.inner_margin = egui::Margin::same(ROW_MARGIN);
    let row = frame.show(ui, |ui| {
        let control_width = (ui.available_width() - MENU_BUTTON_WIDTH).max(40.0);
        let top_left = ui.cursor().min;
        let control_rect = Rect::from_min_size(top_left, Vec2::new(control_width, 0.0));
        let builder = UiBuilder::new().max_rect(Rect::from_min_size(top_left, Vec2::new(control_width, f32::INFINITY))).layout(Layout::top_down(Align::Min));
        let mut control_ui = ui.new_child(builder);
        control_ui.spacing_mut().item_spacing = Vec2::ZERO;
        control_ui.set_width(control_width);
        let control_edit = control_ui.add_enabled_ui(enabled, control).inner;
        let used = control_ui.min_rect();
        let height = used.height().max(1.0);
        let menu_rect = Rect::from_min_size(Pos2::new(control_rect.right(), top_left.y), Vec2::new(MENU_BUTTON_WIDTH, height));
        let menu_edit = parameter_menu(ui, menu_rect, enabled, bind_type, automated);
        ui.allocate_rect(Rect::from_min_size(top_left, Vec2::new(control_width + MENU_BUTTON_WIDTH, height)), Sense::hover());
        menu_edit.unwrap_or(control_edit)
    });
    if let Some(help) = help {
        row.response.on_hover_text(help);
    }
    row.inner
}

/// Parameters with a `[CustomEditor]`: the SatelliteMap library list, the curve
/// editor and Mixer's layers; `None` for the rest. Edits name the
/// property they change.
fn custom_editor(ui: &mut Ui, id: egui::Id, parameter: &ParameterSpec, value: &Value, node: &nerthus_engine::NodeData, nodes: &indexmap::IndexMap<i32, nerthus_engine::NodeData>) -> Option<Vec<(String, Edit)>> {
    let name = parameter.name.clone();
    // `Dictionary<string, Variable>` (Macro) gets the variable editors.
    let editor = if parameter.value_type == nerthus_engine::ValueType::Dictionary { Some("VariableEditorControl") } else { parameter.editor.as_deref() };
    match editor? {
        "SatelliteMapListBox" => {
            let library = node.values.get("Library").and_then(Value::as_str).unwrap_or("Rock");
            let current = value.as_i64().unwrap_or(-1);
            let chosen = editors::library_list(ui, id, library, current);
            let edit = chosen.map_or(Edit::None, |index| Edit::Set(Value::from(index)));
            Some(vec![(name, edit)])
        }
        "LayerEditorPanel" => Some(crate::ui::mixer_layers::show(ui, id, node, nodes, &node.values)),
        "VariableEditorControl" => {
            let changed = editors::macro_variables(ui, id, value, theme::ACCENT_PRIMARY);
            let edit = changed.map_or(Edit::None, Edit::Set);
            Some(vec![(name, edit)])
        }
        "CurveEditor" => {
            let points: Vec<[f32; 2]> = value
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(|item| {
                            let x = item.get("X").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                            let y = item.get("Y").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                            [x, y]
                        })
                        .collect()
                })
                .unwrap_or_default();
            let Some(edited) = editors::curve_editor(ui, id, &points) else {
                return Some(Vec::new());
            };
            let list: Vec<Value> = edited.iter().map(|[x, y]| serde_json::json!({"X": f32_value(*x), "Y": f32_value(*y)})).collect();
            Some(vec![(name, Edit::Set(Value::Array(list)))])
        }
        _ => None,
    }
}

/// How a parameter is automated (`EditorMenu.BindType`).
#[derive(Clone, Copy, PartialEq)]
enum BindType {
    None,
    Variable,
    Expression,
    Both,
}

fn bind_type(automation: &nerthus_engine::automation::Automation, node: i32, property: &str) -> BindType {
    let bound = automation.bindings.iter().any(|binding| binding.node == node && binding.property == property);
    let key = format!("{node}_{property}");
    let expression = automation.expressions.get(&key).is_some_and(|text| !text.trim().is_empty());
    match (bound, expression) {
        (true, true) => BindType::Both,
        (true, false) => BindType::Variable,
        (false, true) => BindType::Expression,
        (false, false) => BindType::None,
    }
}

/// What the parameter menu needs to know about the property's automation.
struct Automated {
    bound_to: Option<String>,
    /// The project's variables (decorations left out) and their types.
    variables: Vec<(String, VarType)>,
    has_expression: bool,
}

impl Automated {
    fn of(automation: &nerthus_engine::automation::Automation, node: i32, property: &str) -> Self {
        let bound_to = automation.bindings.iter().find(|binding| binding.node == node && binding.property == property).map(|binding| binding.variable.clone());
        let variables = automation.variables.iter().filter(|(_, variable)| variable.var_type != VarType::Decoration).map(|(name, variable)| (name.clone(), variable.var_type)).collect();
        let key = format!("{node}_{property}");
        let has_expression = automation.expressions.get(&key).is_some_and(|text| !text.trim().is_empty());
        Self { bound_to, variables, has_expression }
    }
}

/// `EditorMenu`: a 12px keyframe (4 from the editor), filled and coloured
/// when the parameter is bound, at 0.75 under the pointer and 0.5 when
/// disabled, with the parameter's menu (`Ctx_Opened`): Bind to (a new
/// variable, any variable, Unbind), Reset (not while bound), Expression.
fn parameter_menu(ui: &mut Ui, rect: Rect, enabled: bool, bind_type: BindType, automated: &Automated) -> Option<Edit> {
    let response = ui.interact(rect, ui.id().with(("parameter-menu", rect.min.x as i32, rect.min.y as i32)), Sense::click());
    let base = match bind_type {
        BindType::None => theme::ICON_DEFAULT,
        BindType::Variable => theme::ACCENT_MUTED_PURPLE,
        BindType::Expression => theme::ACCENT_MUTED_TEAL,
        BindType::Both => theme::ACCENT_MUTED_ORANGE,
    };
    let opacity = if !enabled {
        0.5
    } else if response.hovered() {
        0.75
    } else {
        1.0
    };
    let color = base.gamma_multiply(opacity);
    let icon_rect = Rect::from_center_size(Pos2::new(rect.left() + 4.0 + 6.0, rect.center().y), Vec2::splat(12.0));
    if bind_type == BindType::None {
        icons::paint(ui, icon_rect, "keyframe", color);
    } else {
        icons::paint_filled(ui, icon_rect, "keyframe", color);
    }
    let bound = automated.bound_to.is_some();
    // A bound property takes expressions only through a number variable.
    let bound_type = automated.bound_to.as_ref().and_then(|name| automated.variables.iter().find(|(candidate, _)| candidate == name)).map(|(_, kind)| *kind);
    let expression_allowed = bound_type.is_none_or(|kind| matches!(kind, VarType::Float | VarType::Int));
    let mut edit = None;
    egui::Popup::menu(&response).show(|ui| {
        crate::ui::widgets::menu_style(ui);
        menu_caption(ui, "EDIT PARAMETER");
        submenu(ui, None, None, "Bind to", |ui| {
            crate::ui::widgets::menu_style(ui);
            if menu_item(ui, Some("variable-plus"), Some(theme::ACCENT_GREEN), "New variable", "").clicked() {
                edit = Some(Edit::BindNew);
            }
            if !automated.variables.is_empty() {
                menu_separator(ui);
            }
            for (name, _) in &automated.variables {
                let checked = automated.bound_to.as_deref() == Some(name.as_str());
                if crate::ui::widgets::check_item(ui, checked, name, "").clicked() {
                    edit = Some(Edit::BindTo(name.clone()));
                }
            }
            if bound {
                menu_separator(ui);
                if menu_item(ui, Some("xmark-circle"), Some(theme::ACCENT_RED), "Unbind", "").clicked() {
                    edit = Some(Edit::AskUnbind);
                }
            }
        });
        if menu_item_enabled(ui, None, None, "Reset", "", !bound).clicked() {
            edit = Some(Edit::Reset);
        }
        menu_separator(ui);
        if menu_item_enabled(ui, if automated.has_expression { Some("check") } else { None }, Some(theme::ACCENT_MUTED_TEAL), "Expression...", "", expression_allowed).clicked() {
            edit = Some(Edit::Expression);
        }
        menu_separator(ui);
        if menu_item(ui, None, None, "Help with Automation", "").clicked() {
            edit = Some(Edit::ShowDocs(crate::docs::PROPERTY_AUTOMATION));
        }
    });
    edit
}

/// Dialogs the parameter menu opens, kept between frames.
#[derive(Clone, Default)]
struct ParameterDialogs {
    /// `Mx2_Click`: unbind only, or unbind and delete the variable.
    unbind: Option<(i32, String)>,
}

fn dialogs_id() -> egui::Id {
    egui::Id::new("parameter-dialogs")
}

/// Shows the open parameter dialogs and applies their answers.
fn parameter_dialogs(ui: &mut Ui, editor: &mut Editor) {
    let mut dialogs: ParameterDialogs = ui.data(|data| data.get_temp(dialogs_id())).unwrap_or_default();
    if let Some((node, property)) = dialogs.unbind.clone() {
        let modal = egui::Modal::new(egui::Id::new("unbind-variable"));
        let response = modal.show(ui.ctx(), |ui| {
            ui.set_width(360.0);
            ui.label(RichText::new(format!("Unbind {property} and delete variable?")).font(theme::semibold(13.0)).color(theme::LABEL_EMPHASIS));
            ui.add_space(6.0);
            ui.label(format!("You are about to unbind {property}. You can choose to keep the underlying variable it was bound to, or delete it."));
            ui.label(RichText::new("If you delete the variable, it will unbind any other properties it was bound to.").color(theme::ICON_DEFAULT));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Unbind only").clicked() {
                    editor.document.unbind(node, &property, false);
                    editor.changed();
                    dialogs.unbind = None;
                }
                if ui.button("Unbind and delete variable").clicked() {
                    editor.document.unbind(node, &property, true);
                    editor.changed();
                    dialogs.unbind = None;
                }
                if ui.button("Cancel").clicked() {
                    dialogs.unbind = None;
                }
            });
        });
        if response.should_close() {
            dialogs.unbind = None;
        }
    }
    ui.data_mut(|data| data.insert_temp(dialogs_id(), dialogs));
}

/// `Separator` (`Separator.Margin`: 16 across, 8 or 2 down).
fn separator(ui: &mut Ui) {
    let gap = theme::sizes(ui.ctx()).separator_margin;
    ui.add_space(gap);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    let line = [Pos2::new(rect.left() + 16.0, rect.center().y), Pos2::new(rect.right() - 16.0, rect.center().y)];
    ui.painter().line_segment(line, Stroke::new(1.0, theme::MENU_SEPARATOR));
    ui.add_space(gap);
}

/// A group panel (`Expander`: `#303030`, margin `Expander.Margin`, padding
/// `Expander.Padding`): a header `Expander.Toggle.MinHeight` high
/// (`ToggleButton-Expander-DownHeader`: the chevron in a 26 column at
/// `Expander.Toggle.IconMargin`, the title in 12px semibold 42 in), then
/// the editors (margin 0, 0, 4, `Editor.BottomMargin`) while open.
pub(crate) fn group_panel(ui: &mut Ui, key: (i32, usize), title: &str, expanded: bool, contents: impl FnOnce(&mut Ui)) {
    let id = ui.id().with(("group", key));
    let open: bool = ui.data(|data| data.get_temp(id)).unwrap_or(expanded);
    let sizes = theme::sizes(ui.ctx());
    let [margin_left, margin_top, margin_right, margin_bottom] = sizes.expander_margin;
    let mut frame = egui::Frame::new();
    frame.fill = theme::EXPANDER_BACKGROUND;
    frame.outer_margin = egui::Margin { left: margin_left as i8, right: margin_right as i8, top: margin_top as i8, bottom: margin_bottom as i8 };
    frame.inner_margin = egui::Margin::same(sizes.expander_padding as i8);
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        let header_height = sizes.expander_header;
        let (header, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), header_height), Sense::click());
        let chevron = if open { "chevron-down" } else { "chevron-right" };
        // Centred in what its margin leaves of the column, or against the
        // margin where it does not fit.
        let [icon_left, icon_top] = sizes.expander_icon_margin;
        let room = GROUP_CHEVRON_COLUMN - icon_left;
        let chevron_left = header.left() + icon_left + ((room - 16.0) / 2.0).max(0.0);
        let chevron_top = header.top() + icon_top + (header_height - icon_top - 16.0) / 2.0;
        let chevron_rect = Rect::from_min_size(Pos2::new(chevron_left, chevron_top), Vec2::splat(16.0));
        icons::paint(ui, chevron_rect, chevron, theme::EXPANDER_HEADER);
        let galley = ui.painter().layout_no_wrap(title.to_string(), theme::semibold(12.0), theme::EXPANDER_HEADER);
        let title_position = Pos2::new(header.left() + GROUP_CHEVRON_COLUMN + 16.0, header.center().y - galley.size().y / 2.0);
        ui.painter().galley(title_position, galley, theme::EXPANDER_HEADER);
        if response.clicked() {
            ui.data_mut(|data| data.insert_temp(id, !open));
        }
        if open {
            let mut content = egui::Frame::new();
            content.inner_margin = egui::Margin { left: 0, right: 4, top: 0, bottom: sizes.editor_bottom as i8 };
            content.show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                contents(ui);
            });
        }
    });
}

/// Files a file parameter offers (`FileBrowser`): the mesh formats for
/// Object nodes, images for the rest.
fn file_extensions(type_name: &str) -> Vec<String> {
    const MESHES: [&str; 7] = ["obj", "fbx", "dae", "gltf", "glb", "ply", "xyz"];
    const IMAGES: [&str; 21] = [
        "exr", "graw", "jpg", "png", "r16", "r32", "raw", "tif", "jpeg", "webp", "tiff", "svg", "rgb", "miff", "jp2", "hdr", "eps", "emf", "dds", "dng", "bmp",
    ];
    let extensions: &[&str] = if type_name == "Object" { &MESHES } else { &IMAGES };
    extensions.iter().map(|extension| extension.to_string()).collect()
}

/// Widths of the four groups: the add menu, then the toggles with their
/// margins.
fn modifier_group_widths() -> [f32; 4] {
    let toggles = |group: &[ModifierToggle]| group.iter().map(|toggle| toggle.gap + MODIFIER_BUTTON).sum::<f32>();
    let inner = [ADD_BUTTON_WIDTH, toggles(MODIFIER_TOGGLES[0]), toggles(MODIFIER_TOGGLES[1]), toggles(MODIFIER_TOGGLES[2])];
    inner.map(|width| width + 2.0 * GROUP_PADDING + 2.0)
}

/// Places the groups in `area` as a `WrapPanel` (margin 4) does: it
/// wraps at the full width, the state menu docking right of whatever room
/// is left; returns them and the height they need.
fn layout_modifier_groups(area: Rect) -> (Vec<Rect>, f32) {
    let left = area.left() + GROUP_MARGIN;
    let right = area.right() - GROUP_MARGIN;
    let group_height = MODIFIER_BUTTON + 2.0 * GROUP_PADDING + 2.0;
    let row_height = group_height + 2.0 * GROUP_MARGIN;
    let mut x = left;
    let mut y = area.top() + GROUP_MARGIN;
    let mut groups = Vec::new();
    for width in modifier_group_widths() {
        let outer = width + 2.0 * GROUP_MARGIN;
        if x > left && x + outer > right {
            x = left;
            y += row_height;
        }
        let group = Rect::from_min_size(Pos2::new(x + GROUP_MARGIN, y + GROUP_MARGIN), Vec2::new(width, group_height));
        groups.push(group);
        x += outer;
    }
    let height = y + row_height + GROUP_MARGIN - area.top();
    (groups, height)
}

/// `ModifierToggleButton-IconX`: 24 square, a 14px icon; on, a pressed
/// fill and the accent's outline; under the pointer, the hover fill.
fn modifier_toggle_button(ui: &mut Ui, toggle: &ModifierToggle, on: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(MODIFIER_BUTTON), Sense::click());
    let hovered = response.hovered();
    let pressed = response.is_pointer_button_down_on();
    let fill = if pressed || (on && !hovered) {
        Some(theme::BUTTON_PRESSED)
    } else if hovered {
        Some(theme::BUTTON_HOVER)
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, 4.0, fill);
    }
    if on {
        ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.5, toggle.accent), StrokeKind::Inside);
    }
    let icon_color = if (hovered || on) && !pressed { theme::LABEL_EMPHASIS } else { theme::ICON_DEFAULT };
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(14.0));
    icons::paint(ui, icon_rect, toggle.icon, icon_color);
    response.on_hover_text(toggle.tooltip)
}

/// The modifier stack (`ModifierStack`): the add menu and toggles for the
/// common modifiers in their bordered groups, the node-state menu right.
fn modifier_toolbar(ui: &mut Ui, rect: Rect, editor: &Editor, node_id: i32, commands: &mut Vec<Command>, actions: &mut Vec<StateAction>) {
    let modifiers = editor.document.node(node_id).map(|node| node.modifiers.as_slice()).unwrap_or_default();
    let present: Vec<&str> = modifiers.iter().map(nerthus_engine::editing::modifier_name).collect();
    let (groups, _) = layout_modifier_groups(rect);
    for group in &groups {
        toolbar_background(ui, *group, theme::PANEL_BORDER, 1.0);
    }

    // `btnAddModifier`: teal while the node has modifiers; its entries are
    // checked by the modifiers present.
    let add_rect = Rect::from_min_size(groups[0].min + Vec2::splat(1.0 + GROUP_PADDING), Vec2::new(ADD_BUTTON_WIDTH, MODIFIER_BUTTON));
    let mut add_ui = ui.new_child(UiBuilder::new().max_rect(add_rect));
    let add_tint = if modifiers.is_empty() { None } else { Some(theme::ACCENT_TEAL) };
    let add_response = icon_menu_sized(&mut add_ui, "square-rounded-plus-2", "Add Modifiers", add_tint, add_rect.size(), |ui| {
        let groups: [&[(&str, &str, &'static str)]; 6] = [
            &[("x-influence", "Influence", "Influence"), ("x-difference", "Difference", "Difference")],
            &[("x-slope", "Mask by Slope", "Slope"), ("x-height", "Mask by Height", "Height")],
            &[("x-clamp", "Clamp", "Clamp"), ("x-clip", "Clip", "Clip")],
            &[("x-equalize", "Equalize", "Equalize"), ("", "Strong", "Strong")],
            &[("x-blur", "Blur", "Blur"), ("x-warp", "Warp", "Warp"), ("x-threshold", "Threshold", "Threshold")],
            &[("x-match", "Match Height", "MatchHeight")],
        ];
        for (index, entries) in groups.iter().enumerate() {
            if index > 0 {
                menu_separator(ui);
            }
            for (icon, text, type_name) in entries.iter().copied() {
                let checked = present.contains(&type_name);
                let shown_icon = if checked { Some("check") } else if icon.is_empty() { None } else { Some(icon) };
                let accent = if checked { theme::ACCENT_TEAL } else { theme::ACCENT_MUTED_GRAY };
                if menu_item(ui, shown_icon, Some(accent), text, "").clicked() {
                    commands.push(Command::ToggleModifier(type_name));
                }
            }
        }
    });
    // M (`btnAddModifier.IsSubmenuOpen`): the menu opens as if clicked.
    if crate::ui::tool_commands::add_modifiers_wanted(ui.ctx()) {
        let popup_id = egui::Popup::default_response_id(&add_response);
        egui::Popup::open_id(ui.ctx(), popup_id);
    }

    for (group, toggles) in groups[1..].iter().zip(MODIFIER_TOGGLES) {
        let inner = group.shrink(1.0 + GROUP_PADDING);
        let builder = UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center));
        let mut group_ui = ui.new_child(builder);
        group_ui.spacing_mut().item_spacing.x = 0.0;
        for toggle in toggles {
            group_ui.add_space(toggle.gap);
            let on = present.contains(&toggle.type_name);
            if modifier_toggle_button(&mut group_ui, toggle, on).clicked() {
                commands.push(Command::ToggleModifier(toggle.type_name));
            }
        }
    }

    // The node-state menu (margin 8, 8, 12, 8).
    let state_rect = Rect::from_min_size(Pos2::new(rect.right() - 12.0 - 32.0, rect.top() + 8.0), Vec2::splat(32.0));
    let saved_state = node_states::has_saved_state(ui);
    let mut state_ui = ui.new_child(UiBuilder::new().max_rect(state_rect));
    icon_menu(&mut state_ui, "arrow-back-up", "Node State", None, |ui| {
        state_items(ui, true, saved_state, commands, actions);
    });
}

/// The node-state items both menus have: Revert Changes, Reset to
/// Defaults, and Save State (`mnuState1`, `mnuState2`), which reads Load
/// State with an upward arrow while a state is saved.
fn state_items(ui: &mut Ui, has_node: bool, saved_state: bool, commands: &mut Vec<Command>, actions: &mut Vec<StateAction>) {
    if menu_item(ui, Some("arrow-back-up-double"), None, "Revert Changes", "").clicked() && has_node {
        actions.push(StateAction::Revert);
    }
    if menu_item(ui, Some("restore"), None, "Reset to Defaults", "").clicked() && has_node {
        commands.push(Command::ResetNode);
    }
    let (state_icon, state_text) = if saved_state { ("arrow-bar-to-up", "Load State") } else { ("arrow-bar-to-down", "Save State") };
    if menu_item(ui, Some(state_icon), Some(theme::ACCENT_MUTED_PURPLE), state_text, "").clicked() && has_node {
        actions.push(StateAction::ToggleSavedState);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::ui::commands::run;
    use crate::ui::UiState;
    use bevy::prelude::Window;
    use serde_json::json;

    #[test]
    fn a_modifier_close_button_asks_unless_shift_is_held() {
        let mut document = Document::new();
        let perlin = document.add_node("Perlin", [26000.0, 26000.0]).unwrap();
        document.toggle_modifier(perlin, "Blur");
        document.toggle_modifier(perlin, "Invert");
        let mut editor = Editor::new(document);
        let mut ui_state = UiState::default();
        let mut window = Window::default();
        run(vec![Command::RemoveModifier { node: perlin, place: 0, ask: true }], &mut editor, &mut ui_state, &mut window);
        let question = ui_state.prompt.take().expect("it asks first");
        assert_eq!(question.caption, "Delete Modifier?");
        assert_eq!(question.text, "Are you sure you want to delete this modifier?");
        assert_eq!(editor.document.node(perlin).unwrap().modifiers.len(), 2);
        run(vec![question.yes.expect("Yes removes it")], &mut editor, &mut ui_state, &mut window);
        let left: Vec<String> = editor.document.node(perlin).unwrap().modifiers.iter().map(|modifier| modifier.type_name.clone()).collect();
        assert_eq!(left, ["Invert"]);
        // Shift held: straight away.
        run(vec![Command::RemoveModifier { node: perlin, place: 0, ask: false }], &mut editor, &mut ui_state, &mut window);
        assert!(ui_state.prompt.is_none());
        assert!(editor.document.node(perlin).unwrap().modifiers.is_empty());
    }

    /// Mountain, then a baked Gate, then a baked Blur.
    fn baked_chain() -> (Editor, i32, i32) {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [26000.0, 26000.0]).unwrap();
        let gate = document.add_node("Gate", [26300.0, 26000.0]).unwrap();
        let blur = document.add_node("Blur", [26600.0, 26000.0]).unwrap();
        document.connect(mountain, "Out", gate, "In");
        document.connect(gate, "Out", blur, "In");
        document.set_member(gate, "StateOverride", json!("Baked"), "Bake", true);
        document.set_member(blur, "StateOverride", json!("Baked"), "Bake", true);
        let mut editor = Editor::new(document);
        editor.select(Some(gate));
        (editor, gate, blur)
    }

    #[test]
    fn the_gate_panel_unbakes_after_asking_and_leaves_the_file_unsaved() {
        let (mut editor, gate, blur) = baked_chain();
        let mut ui_state = UiState::default();
        let mut window = Window::default();
        run(vec![Command::UnbakeNode(gate)], &mut editor, &mut ui_state, &mut window);
        let question = ui_state.prompt.take().expect("it asks first");
        assert_eq!(question.caption, "Unbake Node?");
        assert_eq!(question.text, "Are you sure you want to unbake this node?\n\nAny baked child nodes will also be unbaked.");
        assert_eq!(question.yes, Some(Command::UnbakeNodeConfirmed(gate)));
        run(vec![Command::UnbakeNodeConfirmed(gate)], &mut editor, &mut ui_state, &mut window);
        for node in [gate, blur] {
            let data = editor.document.node(node).unwrap();
            assert_eq!(StateOverride::of(data), StateOverride::None);
        }
        assert!(editor.document.dirty);
        assert!(editor.dialog.is_none(), "no save is asked for");
    }

    #[test]
    fn a_region_preview_keeps_a_gate_baked() {
        let (mut editor, gate, _) = baked_chain();
        editor.preview_region = Some("Region 1".to_string());
        let mut ui_state = UiState::default();
        let mut window = Window::default();
        run(vec![Command::UnbakeNode(gate)], &mut editor, &mut ui_state, &mut window);
        let refusal = ui_state.prompt.take().expect("it refuses");
        assert_eq!(refusal.caption, "Unbaking Critical Node");
        assert_eq!(refusal.yes, None);
    }

    #[test]
    fn load_offloaded_cache_reads_the_node_and_its_baked_descendants() {
        let (mut editor, gate, blur) = baked_chain();
        let mut ui_state = UiState::default();
        let mut window = Window::default();
        run(vec![Command::LoadOffloadedCache(gate)], &mut editor, &mut ui_state, &mut window);
        assert_eq!(editor.pending_load_bakes(), [gate, blur]);
    }

    #[test]
    fn groups_cycle_palettes_like_editor_base() {
        let catalogue = Catalogue::get();
        let perlin = catalogue.node("Perlin").expect("Perlin");
        let (groups, _) = parameter_groups(perlin);
        let titles: Vec<&str> = groups.iter().map(|group| group.title.as_str()).collect();
        assert_eq!(titles, ["Noise", "Warp", "Transform"]);
        // Noise starts the ParamsX palette (index 1); Scale takes its first
        // colour, Octaves the second.
        assert_eq!(groups[0].entries[0].1, palette_color(1, 0));
        assert_eq!(groups[0].entries[1].1, palette_color(1, 1));
    }
}
