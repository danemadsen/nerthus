//! The Vars tab (`tabAutomation`): creating variables, edit mode,
//! grouping by node, the command line and deleting them in the toolbar;
//! below, an editor for each variable (`VariableEditorBase`) or, in edit
//! mode, one row per variable (`UnboundVariableEditor`) that its grip
//! drags to another place in the order.

use egui::{Color32, CursorIcon, Key, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};
use nerthus_engine::automation::{VarType, Variable, VariableValue};
use serde_json::{json, Value};

use crate::document::variable_name_problem;
use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::data_view::{toolbar, toolbar_divider, toolbar_row};
use crate::ui::editors::{dropdown_box, palette_color, range_slider, slider, toggle, RangeSpec, SliderSpec};
use crate::ui::slider_popups::{self, BoxWidth, TypingLook};
use crate::ui::widgets::{icon_button, icon_menu, icon_toggle, menu_caption, menu_item, menu_separator};

/// `Create Variable`'s entries: header, icon (`Accent-Muted-Teal`).
const VARIABLE_KINDS: [(&str, &str); 7] = [
    ("Float", "decimal"),
    ("Int", "number-123"),
    ("Range", "brackets-contain"),
    ("Choice", "selector"),
    ("Bool", "toggle-left"),
    ("String", "txt"),
    ("Color", "color-picker"),
];

/// `CONTROL_THICKNESS`: every editor's margin; colour pickers and
/// unbound rows take 24, 2, 4, 2.
const CONTROL_MARGIN: f32 = 4.0;
/// `Accent-Bright`: a decoration's name.
const ACCENT_BRIGHT: Color32 = Color32::from_rgb(0xdd, 0xed, 0xf2);
/// An unbound row's left margin: the style's 20 in edit mode, the 24
/// `VariableEditorBase.AddControl` gives it among the editors.
const ROW_MARGIN_LEFT: f32 = 20.0;
const EDITOR_ROW_MARGIN_LEFT: f32 = 24.0;
/// `UserControl_PreviewMouseDown`: a press this close to a row's left
/// edge (its grip) may drag the row.
const GRIP_REACH: f32 = 24.0;
/// `SystemParameters.MinimumHorizontalDragDistance` and
/// `MinimumVerticalDragDistance` (4 on Windows): how far a press moves
/// before it drags.
const DRAG_DISTANCE: f32 = 4.0;
/// `SystemParameters.CursorHeight` (32 on Windows): `GetInsertIndex`
/// aims half of it above the pointer.
const CURSOR_HEIGHT: f32 = 32.0;
/// `StackPanel_DragOver`: the entry a drop would land on is drawn at
/// this opacity.
const DROP_TARGET_OPACITY: f32 = 0.5;
/// `FadeIn(1.0)`: seconds the dropped row takes to fade in.
const DROP_FADE_SECONDS: f64 = 1.0;

/// The tab's state between frames.
#[derive(Default)]
pub struct VariablesState {
    /// `tglDesignTime`: one row per variable instead of its editor.
    design_time: bool,
    /// Delete all variables waits for Yes.
    confirm_delete_all: bool,
    reorder: Reorder,
}

/// Edit mode's rows dragged by their grips to new places
/// (`UserControl_PreviewMouseDown`, `UserControl_PreviewMouseMove`,
/// `StackPanel_DragOver`, `StackPanel_Drop`). Entries count `stackVars`'
/// children: the hint is 0 and the rows follow in order.
#[derive(Default)]
struct Reorder {
    /// `_mouseDownPosition`: the row whose grip the left button went down
    /// on, and where.
    press: Option<(usize, Pos2)>,
    /// The row being dragged (`DoDragDrop`'s data).
    dragged: Option<usize>,
    /// `_lastHoveredElement`: the entry the drop would land on.
    target: Option<usize>,
    /// The variable dropped last and when; its row fades in.
    fade: Option<(String, f64)>,
}

impl Reorder {
    /// The opacity `StackPanel_DragOver` leaves an entry at.
    fn target_opacity(&self, entry: usize) -> f32 {
        if self.target == Some(entry) { DROP_TARGET_OPACITY } else { 1.0 }
    }

    /// `FadeIn(1.0)` on the row dropped last: clear to opaque, evenly.
    fn fade_opacity(&self, key: &str, time: f64) -> f32 {
        match &self.fade {
            Some((faded, started)) if faded == key => {
                let progress = (time - started) / DROP_FADE_SECONDS;
                progress.clamp(0.0, 1.0) as f32
            }
            _ => 1.0,
        }
    }

    /// Forgets a fade that has run its course; true while one runs.
    fn fading(&mut self, time: f64) -> bool {
        let finished = self.fade.as_ref().is_some_and(|(_, started)| time - started >= DROP_FADE_SECONDS);
        if finished {
            self.fade = None;
        }
        self.fade.is_some()
    }

    /// `UserControl_PreviewMouseDown`: a left press on a row's grip may
    /// become a drag; one anywhere else on the row may not.
    fn press_row(&mut self, ui: &Ui, entry: usize, rect: Rect, response: &Response) {
        let pressed = ui.input(|input| input.pointer.primary_pressed());
        if !pressed || !response.is_pointer_button_down_on() {
            return;
        }
        let origin = ui.input(|input| input.pointer.press_origin());
        let on_grip = origin.filter(|origin| origin.x - rect.left() <= GRIP_REACH);
        self.press = on_grip.map(|origin| (entry, origin));
    }

    /// `UserControl_PreviewMouseMove`, `StackPanel_DragOver` and
    /// `StackPanel_Drop`: a press on a grip that moves far enough either
    /// way drags its row; over `zone` (the stack) the entry it would land
    /// on dims, and letting go there gives the move: the dragged entry and
    /// `GetInsertIndex` held to the entries.
    fn drag(&mut self, ui: &Ui, entries: &[Rect], zone: Rect) -> Option<(usize, usize)> {
        let pointer = ui.input(|input| input.pointer.latest_pos());
        let down = ui.input(|input| input.pointer.primary_down());
        if let Some((entry, origin)) = self.press {
            let moved = pointer.map(|position| (position - origin).abs());
            let far_enough = moved.is_some_and(|moved| moved.x >= DRAG_DISTANCE || moved.y >= DRAG_DISTANCE);
            if !down {
                self.press = None;
            } else if far_enough {
                self.press = None;
                self.dragged = Some(entry);
            }
        }
        let dragged = self.dragged?;
        // `QueryContinueDrag`: Escape gives up.
        if ui.input(|input| input.key_pressed(Key::Escape)) {
            self.end_drag();
            return None;
        }
        let over = pointer.filter(|position| zone.contains(*position));
        if let Some(position) = over {
            let index = insert_index(entries, position.y);
            self.target = (index < entries.len()).then_some(index);
        }
        let cursor = if over.is_some() { CursorIcon::Default } else { CursorIcon::NoDrop };
        ui.ctx().set_cursor_icon(cursor);
        if down {
            return None;
        }
        // Let go: a drop only when the release was seen over the stack.
        let released = ui.input(|input| input.pointer.primary_released());
        self.end_drag();
        let position = over.filter(|_| released)?;
        let last = entries.len().checked_sub(1)?;
        let index = insert_index(entries, position.y).min(last);
        // On entry 0, the hint, the drop fails and nothing changes.
        let moves = index != dragged && index != 0 && dragged <= last;
        moves.then_some((dragged, index))
    }

    /// The drag ends; whatever was dimmed is whole again.
    fn end_drag(&mut self) {
        self.dragged = None;
        self.target = None;
    }
}

/// How an `UnboundVariableEditor` sits and looks: edit mode keeps the
/// style's margin and border, `VariableEditorBase` moves it to 24 and
/// borders undefined variables in a toolbox colour.
#[derive(Clone, Copy)]
struct RowStyle {
    margin_left: f32,
    border: Color32,
    /// A row is being dragged: the face stops showing presses, as WPF's
    /// does while `DoDragDrop` holds the mouse.
    dragging: bool,
}

/// Where `VariableEditorBase` is in its colours: `colorFlip` moves on
/// with each expander, `j` with each editor.
struct Palette {
    flip: usize,
    count: usize,
}

impl Palette {
    /// `GetBrushString`: `ParamsY` for 0, `ParamsX` for 1, `ParamsZ` for 2.
    fn next_color(&mut self) -> Color32 {
        let palette = match self.flip {
            0 => 0,
            1 => 1,
            _ => 2,
        };
        let color = palette_color(palette, self.count % 8);
        self.count += 1;
        color
    }
}

pub fn show(ui: &mut Ui, area: Rect, editor: &mut Editor, state: &mut VariablesState, commands: &mut Vec<Command>) {
    let (inner, page_top) = toolbar(ui, area);
    let mut right = toolbar_row(ui, inner, true);
    if icon_button(&mut right, "square-rounded-x", "Delete all variables", Some(theme::ACCENT_RED)).clicked() && !editor.document.automation().variables.is_empty() {
        state.confirm_delete_all = true;
    }
    let mut row = toolbar_row(ui, inner, false);
    create_variable_menu(&mut row, editor, commands);
    if icon_toggle(&mut row, "pencil-code", "Toggle Edit Mode", state.design_time, theme::ACCENT_MUTED_GRAY).clicked() {
        state.design_time = !state.design_time;
        state.reorder = Reorder::default();
    }
    toolbar_divider(&mut row, 2.0, 4.0);
    let grouped = editor.document.group_variables_by_nodes();
    if icon_toggle(&mut row, "sort-ascending-shapes", "Group by Nodes", grouped, theme::ACCENT_MUTED_GRAY).clicked() {
        editor.document.set_group_variables_by_nodes(!grouped);
    }
    toolbar_divider(&mut row, 2.0, 4.0);
    if icon_button(&mut row, "terminal", "Show Build Command Line", None).clicked() {
        commands.push(Command::ShowBuildCommandLine);
    }
    toolbar_divider(&mut row, 2.0, 4.0);
    connected_nodes_menu(&mut row, commands);

    // `stackVars` (margin 0, 0, 16, 0) in a `ScrollViewer`.
    let page = Rect::from_min_max(Pos2::new(area.left(), page_top), Pos2::new(area.right() - 16.0, area.bottom()));
    let builder = egui::UiBuilder::new().max_rect(page);
    let mut page_ui = ui.new_child(builder);
    let mut scroll = egui::ScrollArea::vertical();
    scroll = scroll.id_salt("data-view-variables");
    scroll = scroll.auto_shrink([false, false]);
    scroll.show(&mut page_ui, |ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        variables_page(ui, editor, state, commands);
    });
    confirm_delete_all(ui, editor, state);
}

/// `RecreateVariables`.
fn variables_page(ui: &mut Ui, editor: &mut Editor, state: &mut VariablesState, commands: &mut Vec<Command>) {
    let mut variables: Vec<(String, Variable)> = editor.document.automation().variables.iter().map(|(key, variable)| (key.clone(), variable.clone())).collect();
    if variables.is_empty() {
        hint(ui, "Add a variable using the button above.", 0.5);
        return;
    }
    variables.sort_by_key(|(_, variable)| order_of(variable));
    if state.design_time {
        edit_mode_rows(ui, editor, &mut state.reorder, &variables, commands);
        return;
    }
    if editor.document.group_variables_by_nodes() {
        grouped_editors(ui, editor, &variables, commands);
    } else {
        ungrouped_editors(ui, editor, &variables, commands);
    }
}

/// A `TextBlock` with padding 24, wrapped. Returns its rect.
fn hint(ui: &mut Ui, text: &str, opacity: f32) -> Rect {
    let color = theme::LABEL_DEFAULT.gamma_multiply(opacity);
    let rich = egui::RichText::new(text).font(theme::interface(12.0)).color(color);
    let mut frame = egui::Frame::new();
    frame.inner_margin = egui::Margin::same(24);
    let shown = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(egui::Label::new(rich).wrap());
    });
    shown.response.rect
}

/// Edit mode: the hint, then an `UnboundVariableEditor` per variable in
/// order. A row's grip drags it to the place of the entry dimmed under
/// the pointer; the variables are then numbered in their new order and
/// the moved row fades in.
fn edit_mode_rows(ui: &mut Ui, editor: &mut Editor, reorder: &mut Reorder, variables: &[(String, Variable)], commands: &mut Vec<Command>) {
    let time = ui.input(|input| input.time);
    if reorder.fading(time) {
        ui.ctx().request_repaint();
    }
    let mut entries = Vec::with_capacity(variables.len() + 1);
    let hint_opacity = reorder.target_opacity(0);
    let hint_rect = hint(ui, "EDIT MODE: Left click a label or right-click a variable entry to edit.", hint_opacity);
    entries.push(hint_rect);

    let style = RowStyle {
        margin_left: ROW_MARGIN_LEFT,
        border: theme::BUTTON_BORDER,
        dragging: reorder.dragged.is_some(),
    };
    for (index, (key, variable)) in variables.iter().enumerate() {
        let entry = index + 1;
        let opacity = reorder.target_opacity(entry) * reorder.fade_opacity(key, time);
        let row = ui.scope(|ui| {
            ui.multiply_opacity(opacity);
            unbound_row(ui, editor, key, variable, style, commands)
        });
        let (rect, response) = row.inner;
        reorder.press_row(ui, entry, rect, &response);
        entries.push(rect);
    }

    // `stackVars` takes drops over the stack itself.
    let zone = ui.min_rect().intersect(ui.clip_rect());
    let Some((from, to)) = reorder.drag(ui, &entries, zone) else {
        return;
    };
    let keys: Vec<String> = variables.iter().map(|(key, _)| key.clone()).collect();
    let rearranged = dropped_order(&keys, from, to);
    editor.document.rearrange_variables(&rearranged);
    if !editor.user.options.disable_animations() {
        let dropped = keys[from - 1].clone();
        reorder.fade = Some((dropped, time));
        ui.ctx().request_repaint();
    }
}

/// `GetInsertIndex`: the first entry whose middle lies below the point
/// half a cursor's height above the pointer, or the count past the last.
fn insert_index(entries: &[Rect], pointer_y: f32) -> usize {
    let drop_y = pointer_y - CURSOR_HEIGHT / 2.0;
    let below = entries.iter().position(|entry| drop_y < entry.center().y);
    below.unwrap_or(entries.len())
}

/// `StackPanel_Drop` on the stack (the hint, then a row per key): the
/// dragged entry comes out and goes back in at `to`, or last when `to` is
/// past the end. Returns the keys in their new order.
fn dropped_order(keys: &[String], from: usize, to: usize) -> Vec<String> {
    let mut stack: Vec<Option<&String>> = Vec::with_capacity(keys.len() + 1);
    stack.push(None);
    stack.extend(keys.iter().map(Some));
    let moved = stack.remove(from);
    if to >= stack.len() {
        stack.push(moved);
    } else {
        stack.insert(to, moved);
    }
    let rows = stack.into_iter().flatten();
    rows.cloned().collect()
}

fn order_of(variable: &Variable) -> i64 {
    variable.json.get("Order").and_then(Value::as_i64).unwrap_or(0)
}

/// `GenerateUngroupedControls`: in order, under "Variables" until a group
/// decoration starts another expander; separator decorations rule a line.
fn ungrouped_editors(ui: &mut Ui, editor: &mut Editor, variables: &[(String, Variable)], commands: &mut Vec<Command>) {
    let mut palette = Palette { flip: 0, count: 0 };
    let mut sections: Vec<(String, Vec<&(String, Variable)>)> = Vec::new();
    for entry in variables {
        let variable = &entry.1;
        if variable.var_type == VarType::Decoration {
            let value = variable.value.to_text();
            if value == "StartGroup" {
                sections.push((variable.name.clone(), Vec::new()));
                continue;
            }
            if sections.is_empty() {
                sections.push(("Variables".to_string(), Vec::new()));
            }
            if let Some((_, members)) = sections.last_mut() {
                members.push(entry);
            }
            continue;
        }
        if sections.is_empty() {
            sections.push(("Variables".to_string(), Vec::new()));
        }
        if let Some((_, members)) = sections.last_mut() {
            members.push(entry);
        }
    }
    for (index, (title, members)) in sections.iter().enumerate() {
        palette.flip = (palette.flip + 1) % 3;
        variable_expander(ui, ("variables", index), title, |ui| {
            for (key, variable) in members {
                if variable.var_type == VarType::Decoration {
                    rule(ui);
                    continue;
                }
                let color = palette.next_color();
                variable_editor(ui, editor, key, variable, color, commands);
            }
        });
    }
}

/// `GenerateGroupedControls`: variables bound to one node under that
/// node's name (by the node's position), then "Others" bound to several,
/// then "Unbound".
fn grouped_editors(ui: &mut Ui, editor: &mut Editor, variables: &[(String, Variable)], commands: &mut Vec<Command>) {
    let bindings = editor.document.automation().bindings.clone();
    let bound_nodes = |key: &str| -> Vec<i32> { bindings.iter().filter(|binding| binding.variable == key).map(|binding| binding.node).collect() };
    let mut single: Vec<(i32, f32, &(String, Variable))> = Vec::new();
    let mut several: Vec<&(String, Variable)> = Vec::new();
    let mut unbound: Vec<&(String, Variable)> = Vec::new();
    for entry in variables {
        let nodes = bound_nodes(&entry.0);
        match nodes.as_slice() {
            [] => unbound.push(entry),
            [node] => {
                let x = editor.document.node(*node).map_or(0.0, |data| data.position[0]);
                single.push((*node, x, entry));
            }
            _ => several.push(entry),
        }
    }
    single.sort_by(|left, right| left.1.total_cmp(&right.1));

    let mut palette = Palette { flip: 0, count: 0 };
    let mut index = 0;
    let mut position = 0;
    while position < single.len() {
        let node = single[position].0;
        let name = editor.document.node(node).map(|data| data.name.clone()).unwrap_or_default();
        let end = single[position..].iter().position(|(other, ..)| *other != node).map_or(single.len(), |offset| position + offset);
        let members: Vec<&(String, Variable)> = single[position..end].iter().map(|(_, _, entry)| *entry).collect();
        palette.flip = (palette.flip + 1) % 3;
        variable_expander(ui, ("grouped", index), &name, |ui| {
            for (key, variable) in &members {
                let color = palette.next_color();
                variable_editor(ui, editor, key, variable, color, commands);
            }
        });
        index += 1;
        position = end;
    }
    for (title, members) in [("Others", several), ("Unbound", unbound)] {
        if members.is_empty() {
            continue;
        }
        palette.flip = (palette.flip + 1) % 3;
        variable_expander(ui, ("grouped", index), title, |ui| {
            for (key, variable) in &members {
                let color = palette.next_color();
                variable_editor(ui, editor, key, variable, color, commands);
            }
        });
        index += 1;
    }
}

/// `Expander-Variable`: the header (`ToggleButton-Expander-Variable`,
/// margin 8, 2: the chevron in `Expander-Header`, `Accent-Gray` under the
/// pointer, the title semibold), open from the start; the editors in a
/// stack with margin 0, 0, 4, 16.
fn variable_expander(ui: &mut Ui, key: (&str, usize), title: &str, contents: impl FnOnce(&mut Ui)) {
    let id = ui.id().with(("variable-expander", key));
    let open: bool = ui.data(|data| data.get_temp(id)).unwrap_or(true);
    let galley = ui.painter().layout_no_wrap(title.to_string(), theme::semibold(12.0), theme::LABEL_DEFAULT);
    let height = galley.size().y.max(16.0) + 4.0;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let chevron = if open { "chevron-down" } else { "chevron-right" };
    let chevron_color = if response.hovered() { theme::ACCENT_GRAY } else { theme::EXPANDER_HEADER };
    let chevron_rect = Rect::from_min_size(Pos2::new(rect.left() + 8.0, rect.center().y - 8.0), Vec2::splat(icons::DEFAULT_SIZE));
    icons::paint(ui, chevron_rect, chevron, chevron_color);
    let title_position = Pos2::new(chevron_rect.right() + 4.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(title_position, galley, theme::LABEL_DEFAULT);
    if response.clicked() {
        ui.data_mut(|data| data.insert_temp(id, !open));
    }
    if !open {
        return;
    }
    let mut frame = egui::Frame::new();
    frame.inner_margin = egui::Margin { left: 0, right: 4, top: 0, bottom: 16 };
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        contents(ui);
    });
}

/// `CreateSeparator`: a `Separator` with margin 16, 8.
fn rule(ui: &mut Ui) {
    ui.add_space(8.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    let line = [Pos2::new(rect.left() + 16.0, rect.center().y), Pos2::new(rect.right() - 16.0, rect.center().y)];
    ui.painter().line_segment(line, Stroke::new(1.0, theme::MENU_SEPARATOR.gamma_multiply(0.2)));
    ui.add_space(8.0);
}

/// `GetControl`: a slider for Float and Int, a range for Range, a
/// dropdown for Choice, a toggle for Bool, a colour for Color and an
/// unbound row for the rest; each with a margin of 4.
fn variable_editor(ui: &mut Ui, editor: &mut Editor, key: &str, variable: &Variable, color: Color32, commands: &mut Vec<Command>) {
    let id = ui.id().with(("variable", key));
    let float = |value: &VariableValue| value.to_single().unwrap_or(0.0) as f64;
    let min_value = variable.json.get("Min").map(VariableValue::from_json).unwrap_or(VariableValue::Null);
    let max_value = variable.json.get("Max").map(VariableValue::from_json).unwrap_or(VariableValue::Null);
    let mut frame = egui::Frame::new();
    frame.inner_margin = egui::Margin::same(CONTROL_MARGIN as i8);
    match variable.var_type {
        VarType::Float | VarType::Int => {
            let integer = variable.var_type == VarType::Int;
            let mut spec = SliderSpec::linear(float(&min_value), float(&max_value));
            spec.integer = integer;
            spec.inset = Some(0.0);
            let value = float(&variable.value);
            let changed = frame.show(ui, |ui| slider(ui, id, &variable.name, value, &spec, color)).inner;
            if let Some(changed) = changed {
                let saved = if integer { json!(changed.round() as i64) } else { nerthus_engine::json::f32_value(changed as f32) };
                editor.document.set_variable_value(key, saved);
                editor.changed();
            }
        }
        VarType::Range => {
            let [low, high] = match &variable.value {
                VariableValue::Float2(pair) => *pair,
                _ => [0.0, 1.0],
            };
            let mut spec = RangeSpec::linear(float(&min_value), float(&max_value));
            spec.inset = Some(0.0);
            let changed = frame.show(ui, |ui| range_slider(ui, id, &variable.name, low as f64, high as f64, &spec, color)).inner;
            if let Some((low, high)) = changed {
                let value = VariableValue::Float2([low as f32, high as f32]);
                editor.document.set_variable_value(key, value.to_json());
                editor.changed();
            }
        }
        VarType::Choice => {
            let options = choice_options(variable);
            let current = variable.value.to_int32().unwrap_or(0).max(0) as usize;
            let shown = options.get(current).cloned().unwrap_or_default();
            let entries: Vec<(String, String)> = options.iter().enumerate().map(|(index, option)| (index.to_string(), option.clone())).collect();
            let column = if ui.available_width() < 300.0 { 100.0 } else { 160.0 };
            let current_text = current.to_string();
            let chosen = frame.show(ui, |ui| dropdown_box(ui, id, Some(&variable.name), column, 26.0, &shown, &entries, &current_text, theme::interface(12.0))).inner;
            let chosen_index = chosen.and_then(|value| value.parse::<i64>().ok());
            if let Some(index) = chosen_index {
                editor.document.set_variable_value(key, json!(index));
                editor.changed();
            }
        }
        VarType::Bool => {
            let value = variable.value.to_boolean().unwrap_or(false);
            let changed = frame.show(ui, |ui| toggle(ui, id, &variable.name, value, theme::ACCENT_PRIMARY)).inner;
            if let Some(changed) = changed {
                editor.document.set_variable_value(key, json!(changed));
                editor.changed();
            }
        }
        VarType::Color => color_row(ui, editor, key, variable),
        VarType::Undefined | VarType::Decoration => {
            let style = RowStyle {
                margin_left: EDITOR_ROW_MARGIN_LEFT,
                border: unbound_border_color(variable),
                dragging: false,
            };
            unbound_row(ui, editor, key, variable, style, commands);
        }
    }
}

/// A Choice variable's options (`ExtraData`, a list of strings).
fn choice_options(variable: &Variable) -> Vec<String> {
    let extra = variable.json.get("ExtraData");
    let list = extra.and_then(|extra| extra.get("$values").or(Some(extra))).and_then(Value::as_array);
    list.map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// `ColorPicker` (margin 24, 2, 4, 2): the name and the colour.
fn color_row(ui: &mut Ui, editor: &mut Editor, key: &str, variable: &Variable) {
    let value = variable.json.get("Value").cloned().unwrap_or(Value::Null);
    let component = |name: &str| value.get(name).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let mut rgba = egui::Rgba::from_rgb(component("R"), component("G"), component("B"));
    let width = ui.available_width();
    let (outer, _) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::hover());
    let rect = Rect::from_min_max(outer.min + Vec2::new(24.0, 2.0), outer.max - Vec2::new(4.0, 2.0));
    let galley = ui.painter().layout_no_wrap(variable.name.clone(), theme::interface(12.0), theme::LABEL_DEFAULT);
    ui.painter().galley(Pos2::new(rect.left(), rect.center().y - galley.size().y / 2.0), galley, theme::LABEL_DEFAULT);
    let swatch = Rect::from_min_max(Pos2::new(rect.left() + 96.0, rect.top()), rect.max);
    let mut swatch_ui = ui.new_child(egui::UiBuilder::new().max_rect(swatch));
    swatch_ui.spacing_mut().interact_size = swatch.size();
    let changed = egui::color_picker::color_edit_button_rgba(&mut swatch_ui, &mut rgba, egui::color_picker::Alpha::Opaque).changed();
    if changed {
        let saved = json!({"R": rgba.r() as f64, "G": rgba.g() as f64, "B": rgba.b() as f64});
        editor.document.set_variable_value(key, saved);
        editor.changed();
    }
}

/// `UnboundVariableEditor` (margin left 20 or 24, 2, 4, 2; padding 16,
/// 8): the grip, the type's icon, the name in a 96 column and the value
/// (monospace in `Accent-Muted-Green`, floats `#0.00`) on a button face;
/// decorations show their name alone in `Accent-Bright`. Right-click
/// offers Edit and Delete. Returns the row's rect inside its margin and
/// its response.
fn unbound_row(ui: &mut Ui, editor: &mut Editor, key: &str, variable: &Variable, style: RowStyle, commands: &mut Vec<Command>) -> (Rect, Response) {
    let decoration = variable.var_type == VarType::Decoration;
    let width = ui.available_width();
    let line_height = ui.painter().layout_no_wrap("Ag".to_string(), theme::monospace(12.0), theme::LABEL_DEFAULT).size().y;
    let (outer, _) = ui.allocate_exact_size(Vec2::new(width, line_height + 16.0 + 4.0), Sense::hover());
    let rect = Rect::from_min_max(outer.min + Vec2::new(style.margin_left, 2.0), outer.max - Vec2::new(4.0, 2.0));
    // The margin is not part of the button.
    let id = ui.id().with(("unbound-variable", key));
    let response = ui.interact(rect, id, Sense::click_and_drag());
    if !decoration {
        let left_down = ui.input(|input| input.pointer.primary_down());
        let pressed = !style.dragging && left_down && response.is_pointer_button_down_on() && response.contains_pointer();
        let fill = if pressed {
            theme::BUTTON_PRESSED
        } else if response.hovered() {
            theme::BUTTON_HOVER
        } else {
            theme::BUTTON_BACKGROUND
        };
        ui.painter().rect_filled(rect, 2.0, fill);
        ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, style.border), StrokeKind::Inside);
    }
    // `grip`: 12 square at half opacity, its margin -11 into the padding.
    let grip_rect = Rect::from_min_size(Pos2::new(rect.left() + 16.0 - 11.0, rect.center().y - 6.0), Vec2::splat(12.0));
    icons::paint(ui, grip_rect, "grip-vertical", theme::ICON_DEFAULT.gamma_multiply(0.5));
    let mut x = grip_rect.right();
    if !decoration {
        let icon = type_icon(variable.var_type);
        let icon_rect = Rect::from_min_size(Pos2::new(x + 6.0, rect.center().y - 8.0), Vec2::splat(icons::DEFAULT_SIZE));
        icons::paint(ui, icon_rect, icon, theme::ICON_DEFAULT);
        x = icon_rect.right() + 6.0;
    }
    let header_color = if decoration { ACCENT_BRIGHT } else { theme::LABEL_DEFAULT };
    let header = ui.painter().layout_no_wrap(variable.name.clone(), theme::monospace(12.0), header_color);
    ui.painter().galley(Pos2::new(x + 6.0, rect.center().y - header.size().y / 2.0), header, header_color);
    if !decoration {
        let value = value_text(variable);
        let value_galley = ui.painter().layout_no_wrap(value, theme::monospace(12.0), theme::ACCENT_MUTED_GREEN);
        let value_position = Pos2::new(x + 6.0 + 96.0 + 8.0, rect.center().y - value_galley.size().y / 2.0);
        ui.painter().galley(value_position, value_galley, theme::ACCENT_MUTED_GREEN);
    }
    response.context_menu(|ui| {
        crate::ui::widgets::menu_style(ui);
        if menu_item(ui, None, None, "Edit...", "").clicked() {
            commands.push(Command::EditVariable(key.to_string()));
        }
        if menu_item(ui, None, Some(theme::ACCENT_RED), "Delete", "").clicked() {
            commands.push(Command::DeleteVariable(key.to_string()));
        }
    });
    // `OnClick`: left of 100 the name is typed over, further right the
    // value (decorations have none).
    let typing_id = id.with("typing");
    if response.clicked()
        && !style.dragging
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let on_name = pointer.x - rect.left() < 100.0;
        let typing = if on_name { Some(RowTyping::Name(key.to_string())) } else { (!decoration).then(|| RowTyping::Value(variable.value.to_text())) };
        if let Some(typing) = typing {
            open_row_typing(ui.ctx(), typing_id, typing);
        }
    }
    if let Some(typed) = row_typing(ui, typing_id, rect) {
        match typed {
            RowTyping::Name(name) if name != key => match variable_name_problem(&editor.document, key, &name) {
                Some(problem) => {
                    let caption = format!("Variable name [{name}] not allowed");
                    commands.push(Command::ShowMessage { caption, text: problem });
                }
                None => {
                    editor.document.rename_variable(key, &name);
                    editor.changed();
                }
            },
            RowTyping::Name(_) => {}
            RowTyping::Value(text) => {
                editor.document.set_variable_text(key, &text);
                editor.changed();
            }
        }
    }
    (rect, response)
}

/// What a row's `EditablePopup` is typing: a new name or a value.
#[derive(Clone, Debug)]
enum RowTyping {
    Name(String),
    Value(String),
}

/// The row's `EditablePopup` (monospace 16): over the name 48 in and 96
/// wide, over the value 112 in and as wide as the row less 128, starting
/// from what `typing` holds.
fn open_row_typing(ctx: &egui::Context, typing_id: egui::Id, typing: RowTyping) {
    let (field, text) = match typing {
        RowTyping::Name(text) => (RowField::Name, text),
        RowTyping::Value(text) => (RowField::Value, text),
    };
    ctx.data_mut(|data| data.insert_temp(typing_id.with("field"), field));
    slider_popups::open_typing(ctx, typing_id, text);
}

/// Which of a row's two texts its box is typing.
#[derive(Clone, Copy, Debug)]
enum RowField {
    Name,
    Value,
}

/// The row's typing box while it shows; `Some` once its text is taken
/// (`EditablePopup_EditSuccess`).
fn row_typing(ui: &mut Ui, typing_id: egui::Id, row: Rect) -> Option<RowTyping> {
    let field_id = typing_id.with("field");
    let field: RowField = ui.data(|data| data.get_temp(field_id))?;
    // In the popup's own units.
    let (offset, width) = match field {
        RowField::Name => (48.0, 96.0),
        RowField::Value => (112.0, (row.width() - 128.0).max(64.0)),
    };
    let popup_scale = slider_popups::popup_scale(ui.ctx());
    let look = TypingLook {
        top_left: Pos2::new(row.left() + offset * popup_scale, row.top()),
        width: BoxWidth::Fixed(width * popup_scale),
        font: slider_popups::value_font(),
    };
    let taken = slider_popups::typing_box(ui, typing_id, &look);
    if !slider_popups::typing_open(ui.ctx(), typing_id) {
        ui.data_mut(|data| data.remove::<RowField>(field_id));
    }
    let text = taken?;
    let typed = match field {
        RowField::Name => RowTyping::Name(text),
        RowField::Value => RowTyping::Value(text),
    };
    Some(typed)
}

/// `GetIconFromVar`.
fn type_icon(var_type: VarType) -> &'static str {
    match var_type {
        VarType::Undefined => "txt",
        VarType::Float => "decimal",
        VarType::Int => "number-123",
        VarType::Range => "brackets-contain",
        VarType::Choice => "selector",
        VarType::Bool => "toggle-left",
        VarType::Color => "color-picker",
        VarType::Decoration => "",
    }
}

/// `GetControl`'s border for an undefined variable, by `ExtraData`: none
/// in `Toolbox-Surface`, a File path (`1`) in `Toolbox-Primitive`, an
/// Export path (`2`) in `Toolbox-Output`. Anything else keeps
/// `Button-Border`, as every other type does.
fn unbound_border_color(variable: &Variable) -> Color32 {
    if variable.var_type != VarType::Undefined {
        return theme::BUTTON_BORDER;
    }
    match variable.json.get("ExtraData") {
        None | Some(Value::Null) => theme::toolbox_color("Surface"),
        Some(Value::String(text)) if text == "1" => theme::toolbox_color("Primitive"),
        Some(Value::String(text)) if text == "2" => theme::toolbox_color("Output"),
        Some(_) => theme::BUTTON_BORDER,
    }
}

/// `SetContentFromVariable`: floats as `#0.00`, the rest as text.
fn value_text(variable: &Variable) -> String {
    match &variable.value {
        VariableValue::Float(number) => format!("{number:.2}"),
        VariableValue::Float2([low, high]) => format!("{low}, {high}"),
        VariableValue::Null => String::new(),
        other => other.to_text(),
    }
}

/// `Create Variable`.
fn create_variable_menu(ui: &mut Ui, editor: &mut Editor, commands: &mut Vec<Command>) {
    icon_menu(ui, "variable-plus", "Create Variable", None, |ui| {
        let mut chosen = None;
        for (kind, icon) in VARIABLE_KINDS {
            if menu_item(ui, Some(icon), Some(theme::ACCENT_MUTED_TEAL), kind, "").clicked() {
                chosen = Some(kind);
            }
        }
        menu_separator(ui);
        if menu_item(ui, None, None, "Undefined", "").clicked() {
            chosen = Some("Undefined");
        }
        menu_separator(ui);
        menu_caption(ui, "UI Decorations");
        if menu_item(ui, None, None, "Separator", "").clicked() {
            chosen = Some("Separator");
        }
        if menu_item(ui, None, None, "Group", "").clicked() {
            chosen = Some("Group");
        }
        // `BtnAddVar_OnClick` opens Edit Variable on the new variable.
        if let Some(kind) = chosen {
            let key = editor.document.add_variable(kind);
            commands.push(Command::EditVariable(key));
        }
    });
}

/// `Create Connected Nodes`: MacroPorts on the selected nodes' ports
/// (`BtnCreateMacroPorts_OnClick`, by the entry's tag: `All`, `In`,
/// `Out`). Houdini bridges are left out.
fn connected_nodes_menu(ui: &mut Ui, commands: &mut Vec<Command>) {
    icon_menu(ui, "route-square", "Create Connected Nodes", None, |ui| {
        menu_caption(ui, "For Macros");
        let entries = [
            ("All Ports", "route-square", None, true, true),
            ("In Only", "arrow-left-square", Some(theme::ACCENT_MUTED_BLUE), true, false),
            ("Out Only", "arrow-right-square", Some(theme::ACCENT_MUTED_GREEN), false, true),
        ];
        for (header, icon, accent, inputs, outputs) in entries {
            if menu_item(ui, Some(icon), accent, header, "").clicked() {
                commands.push(Command::CreateMacroPorts { inputs, outputs });
            }
        }
    });
}

/// `BtnDeleteAllVariables_OnClick`'s question.
fn confirm_delete_all(ui: &mut Ui, editor: &mut Editor, state: &mut VariablesState) {
    if !state.confirm_delete_all {
        return;
    }
    let modal = egui::Modal::new(egui::Id::new("confirm-delete-variables"));
    let response = modal.show(ui.ctx(), |ui| {
        ui.set_width(340.0);
        let title = egui::RichText::new("Are you sure you want to delete all variables?").font(theme::semibold(13.0)).color(theme::LABEL_EMPHASIS);
        ui.label(title);
        ui.add_space(6.0);
        ui.label("All variables in the current project will be removed");
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button("Yes").clicked() {
                editor.document.delete_all_variables();
                editor.changed();
                state.confirm_delete_all = false;
            }
            if ui.button("No").clicked() {
                state.confirm_delete_all = false;
            }
        });
    });
    if response.should_close() {
        state.confirm_delete_all = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame of a row's typing box with these events.
    fn typing_frame(context: &egui::Context, typing_id: egui::Id, events: Vec<egui::Event>) -> Option<RowTyping> {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0));
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut typed = None;
        let output = context.run_ui(input, |ui| {
            let row = Rect::from_min_size(Pos2::new(0.0, 40.0), Vec2::new(300.0, 32.0));
            typed = row_typing(ui, typing_id, row);
        });
        output.drop_without_applying_deltas();
        typed
    }

    #[test]
    fn a_value_typed_over_a_row_is_taken_with_enter() {
        let context = egui::Context::default();
        let typing_id = egui::Id::new("row-typing-test");
        open_row_typing(&context, typing_id, RowTyping::Value("0.5".to_string()));
        assert!(typing_frame(&context, typing_id, Vec::new()).is_none());
        let typed = egui::Event::Text("7".to_string());
        assert!(typing_frame(&context, typing_id, vec![typed]).is_none());
        let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let taken = typing_frame(&context, typing_id, vec![enter]);
        assert!(matches!(&taken, Some(RowTyping::Value(text)) if text.ends_with('7')), "{taken:?}");
    }

    fn keys(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    /// Edit mode's stack: the hint 64 high, then rows 32 high with 2 above
    /// and below (rows at 66, 102 and 138 for three).
    fn stack(rows: usize) -> Vec<Rect> {
        let mut entries = vec![Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 64.0))];
        for row in 0..rows {
            let top = 66.0 + row as f32 * 36.0;
            entries.push(Rect::from_min_size(Pos2::new(20.0, top), Vec2::new(276.0, 32.0)));
        }
        entries
    }

    fn moved_to(x: f32, y: f32) -> egui::Event {
        egui::Event::PointerMoved(Pos2::new(x, y))
    }

    fn left_button(x: f32, y: f32, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: Pos2::new(x, y),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// One frame of three rows sensing the pointer as edit mode's do, the
    /// stack taking drops. Returns the move a drop made and the cursor.
    fn frame(context: &egui::Context, reorder: &mut Reorder, events: Vec<egui::Event>) -> (Option<(usize, usize)>, CursorIcon) {
        let entries = stack(3);
        let zone = Rect::from_min_max(Pos2::ZERO, Pos2::new(300.0, 172.0));
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0));
        let input = egui::RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        let mut dropped = None;
        let output = context.run_ui(input, |ui| {
            for (entry, rect) in entries.iter().enumerate().skip(1) {
                let id = egui::Id::new(("row", entry));
                let response = ui.interact(*rect, id, Sense::click_and_drag());
                reorder.press_row(ui, entry, *rect, &response);
            }
            dropped = reorder.drag(ui, &entries, zone);
        });
        let cursor = output.platform_output.cursor_icon;
        output.drop_without_applying_deltas();
        (dropped, cursor)
    }

    #[test]
    fn drops_aim_at_the_entry_under_the_pointer() {
        let entries = stack(3);
        assert_eq!(insert_index(&entries, 40.0), 0);
        assert_eq!(insert_index(&entries, 66.0), 1);
        assert_eq!(insert_index(&entries, 97.0), 1);
        // Between rows the next one is aimed at.
        assert_eq!(insert_index(&entries, 100.0), 2);
        assert_eq!(insert_index(&entries, 169.0), 3);
        assert_eq!(insert_index(&entries, 200.0), 4);
    }

    #[test]
    fn a_dropped_row_takes_the_place_it_was_dropped_on() {
        let names = keys(&["a", "b", "c", "d"]);
        assert_eq!(dropped_order(&names, 1, 3), keys(&["b", "c", "a", "d"]));
        assert_eq!(dropped_order(&names, 4, 2), keys(&["a", "d", "b", "c"]));
        assert_eq!(dropped_order(&names, 2, 4), keys(&["a", "c", "d", "b"]));
    }

    #[test]
    fn a_grip_drags_its_row_once_it_moves_four_points() {
        let context = egui::Context::default();
        let mut reorder = Reorder::default();
        frame(&context, &mut reorder, Vec::new());
        frame(&context, &mut reorder, vec![moved_to(25.0, 70.0), left_button(25.0, 70.0, true)]);
        frame(&context, &mut reorder, vec![moved_to(25.0, 73.0)]);
        assert_eq!(reorder.dragged, None);
        frame(&context, &mut reorder, vec![moved_to(25.0, 74.0)]);
        assert_eq!((reorder.dragged, reorder.target), (Some(1), Some(1)));
        let (dropped, cursor) = frame(&context, &mut reorder, vec![moved_to(25.0, 140.0)]);
        assert_eq!((dropped, cursor, reorder.target), (None, CursorIcon::Default, Some(3)));
        let (outside, cursor) = frame(&context, &mut reorder, vec![moved_to(350.0, 140.0)]);
        assert_eq!((outside, cursor, reorder.target), (None, CursorIcon::NoDrop, Some(3)));
        frame(&context, &mut reorder, vec![moved_to(25.0, 140.0)]);
        let (dropped, _) = frame(&context, &mut reorder, vec![left_button(25.0, 140.0, false)]);
        assert_eq!(dropped, Some((1, 3)));
        assert_eq!((reorder.dragged, reorder.target), (None, None));
    }

    #[test]
    fn rows_stay_put_off_the_grip_off_the_stack_on_the_hint_and_on_escape() {
        let context = egui::Context::default();
        let mut reorder = Reorder::default();
        frame(&context, &mut reorder, Vec::new());
        // 40 into the row is past the grip.
        frame(&context, &mut reorder, vec![moved_to(60.0, 110.0), left_button(60.0, 110.0, true)]);
        frame(&context, &mut reorder, vec![moved_to(60.0, 150.0)]);
        let (dropped, _) = frame(&context, &mut reorder, vec![left_button(60.0, 150.0, false)]);
        assert_eq!((dropped, reorder.dragged), (None, None));

        let drags = [(350.0, 150.0), (25.0, 30.0)];
        for (x, y) in drags {
            frame(&context, &mut reorder, vec![moved_to(25.0, 110.0), left_button(25.0, 110.0, true)]);
            frame(&context, &mut reorder, vec![moved_to(x, y)]);
            assert_eq!(reorder.dragged, Some(2));
            let (dropped, _) = frame(&context, &mut reorder, vec![left_button(x, y, false)]);
            assert_eq!(dropped, None);
        }

        frame(&context, &mut reorder, vec![moved_to(25.0, 110.0), left_button(25.0, 110.0, true)]);
        frame(&context, &mut reorder, vec![moved_to(25.0, 150.0)]);
        let escape = egui::Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&context, &mut reorder, vec![escape]);
        assert_eq!((reorder.dragged, reorder.target), (None, None));
        let (dropped, _) = frame(&context, &mut reorder, vec![left_button(25.0, 150.0, false)]);
        assert_eq!(dropped, None);
    }
}
