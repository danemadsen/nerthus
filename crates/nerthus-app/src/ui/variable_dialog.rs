//! `EditVariableDialog` (480 wide, "Edit Variable (Float)"): the variable's
//! name, Min and Max for Float, Int and Range (`NumericTextBox`es), the
//! choices for Choice and the string type for Undefined. Accept Changes
//! (Enter) checks the name and writes the rest, renaming the variable when
//! its name changed; Cancel (Escape) leaves it as it was. It opens from a
//! row's Edit… and after Create Variable makes one.

use egui::text::{CCursor, CCursorRange};
use egui::{Align, CursorIcon, Id, Key, Layout, Pos2, Rect, RichText, Sense, Stroke, Ui, UiBuilder, Vec2};
use nerthus_engine::automation::{VarType, Variable, VariableValue};
use serde_json::Value;

use crate::document::{variable_name_problem, VariableLimits};
use crate::editor::Editor;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::editors::dropdown_box;

/// The window (480) less its margin (12 each side).
const CONTENT_WIDTH: f32 = 480.0 - 24.0;
/// `NumericTextBox`'s `Minimum` and `Maximum` here (`RecreateUI`).
const LIMIT: f32 = 65535.0;
/// A `NumericTextBox`'s header (`MinWidth="96"`) and box (96 at least).
const NUMBER_COLUMN: f32 = 96.0;
/// `Thumb_OnDragDelta`: how far a point of dragging the header moves the
/// value.
const DRAG_PER_POINT: f32 = 0.01;
const INTEGER_DRAG_PER_POINT: f32 = 3.3;
/// `cboStringType`'s entries.
const STRING_TYPES: [&str; 5] = ["Standard", "Input", "Output", "Input (Optional)", "Output (Optional)"];
const ROW_HEIGHT: f32 = 26.0;

/// A `NumericTextBox`: its value and the text shown for it.
#[derive(Clone, Debug)]
struct NumberField {
    value: f32,
    text: String,
    integer: bool,
    /// The value when its header started being dragged, and how far it
    /// has been dragged.
    drag: Option<(f32, f32)>,
}

impl NumberField {
    fn new(value: f32, integer: bool) -> Self {
        let mut field = Self { value, text: String::new(), integer, drag: None };
        field.text = field.value_text();
        field
    }

    /// `GetValueAsText`: `0.00`, or `0` for integers.
    fn value_text(&self) -> String {
        if self.integer {
            let rounded = self.value.round() as i64;
            rounded.to_string()
        } else {
            format!("{:.2}", self.value)
        }
    }

    /// Whether the text was typed since the value last took it.
    fn typed(&self) -> bool {
        self.text != self.value_text()
    }

    /// `Txt_OnLostFocus`: the text as a number held to the limits, or the
    /// value's text again when it is not one.
    fn commit(&mut self) {
        if let Ok(number) = self.text.trim().parse::<f32>() {
            self.value = number.clamp(-LIMIT, LIMIT);
        }
        self.text = self.value_text();
    }
}

/// The dialog while it is open.
#[derive(Clone, Debug)]
pub struct EditVariable {
    key: String,
    var_type: VarType,
    name: String,
    min: NumberField,
    max: NumberField,
    /// `varChoices`: the choices joined with ", ".
    choices: String,
    string_type: usize,
    focused: bool,
}

impl EditVariable {
    /// `RecreateUI`: the fields from the variable.
    fn new(key: &str, variable: &Variable) -> Self {
        let limit = |name: &str| variable.json.get(name).map(VariableValue::from_json).unwrap_or(VariableValue::Null);
        let integer = variable.var_type == VarType::Int;
        let min_value = if integer { limit("Min").to_int32().unwrap_or(0) as f32 } else { limit("Min").to_single().unwrap_or(0.0) };
        let max_value = if integer { limit("Max").to_int32().unwrap_or(0) as f32 } else { limit("Max").to_single().unwrap_or(0.0) };
        let extra = variable.json.get("ExtraData");
        let list = extra.and_then(|extra| extra.get("$values").or(Some(extra))).and_then(Value::as_array);
        let options: Vec<&str> = list.map(|items| items.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let joined = options.join(", ");
        let choices = joined.trim().trim_matches(',').to_string();
        // `Int32.TryParse(ExtraData)`, else Standard.
        let extra_text = extra.map(VariableValue::from_json).map(|value| value.to_text());
        let string_type = extra_text.and_then(|text| text.parse::<usize>().ok()).unwrap_or(0);
        Self {
            key: key.to_string(),
            var_type: variable.var_type,
            name: key.to_string(),
            min: NumberField::new(min_value, integer),
            max: NumberField::new(max_value, integer),
            choices,
            string_type,
            focused: false,
        }
    }

    /// `BtnOK_OnClick`'s changes for the variable's type.
    fn limits(&self) -> VariableLimits {
        match self.var_type {
            VarType::Float | VarType::Range => VariableLimits::Float { min: self.min.value, max: self.max.value },
            VarType::Int => {
                let min = (self.min.value as f64).round_ties_even() as i32;
                let max = (self.max.value as f64).round_ties_even() as i32;
                VariableLimits::Int { min, max }
            }
            VarType::Choice => {
                let parts = self.choices.split(',');
                let trimmed = parts.map(str::trim);
                let choices = trimmed.filter(|choice| !choice.is_empty()).map(str::to_string).collect();
                VariableLimits::Choices(choices)
            }
            VarType::Undefined => VariableLimits::StringType(self.string_type),
            VarType::Bool | VarType::Color | VarType::Decoration => VariableLimits::None,
        }
    }
}

fn window_id() -> Id {
    Id::new("edit-variable")
}

/// `new EditVariableDialog { Variable = key }.ShowDialog()`.
pub fn open(editor: &Editor, key: &str) -> Option<EditVariable> {
    let variable = editor.document.automation().variables.get(key)?;
    Some(EditVariable::new(key, variable))
}

/// The dialog, while one is open.
pub fn show(ctx: &egui::Context, editor: &mut Editor, open_dialog: &mut Option<EditVariable>, commands: &mut Vec<Command>) {
    let Some(dialog) = open_dialog.as_mut() else {
        return;
    };
    if !editor.document.automation().variables.contains_key(&dialog.key) {
        *open_dialog = None;
        return;
    }
    let mut accept = false;
    let mut close = false;
    // `Window-Background`, `DockPanel Margin="12"`.
    let mut frame = egui::Frame::popup(&ctx.global_style());
    frame.fill = theme::WINDOW_BACKGROUND;
    frame.inner_margin = egui::Margin::same(12);
    let mut modal = egui::Modal::new(window_id().with("modal"));
    modal = modal.frame(frame);
    let response = modal.show(ctx, |ui| {
        ui.set_width(CONTENT_WIDTH);
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        let title = format!("Edit Variable ({})", dialog.var_type.name());
        let caption = RichText::new(title).font(theme::semibold(13.0)).color(theme::LABEL_EMPHASIS);
        ui.label(caption);
        ui.add_space(8.0);
        name_row(ui, dialog);
        match dialog.var_type {
            VarType::Float | VarType::Range | VarType::Int => {
                ui.add_space(8.0);
                number_field(ui, window_id().with("min"), "Min", &mut dialog.min);
                ui.add_space(8.0);
                number_field(ui, window_id().with("max"), "Max", &mut dialog.max);
            }
            VarType::Choice => {
                ui.add_space(8.0);
                choices_row(ui, dialog);
            }
            VarType::Undefined => {
                ui.add_space(8.0);
                string_type_row(ui, dialog);
            }
            VarType::Bool | VarType::Color | VarType::Decoration => {}
        }
        // The buttons (margin 8, 24, 8, 8), on the right.
        ui.add_space(24.0);
        ui.allocate_ui_with_layout(Vec2::new(CONTENT_WIDTH, ROW_HEIGHT + 8.0), Layout::right_to_left(Align::Min), |ui| {
            ui.add_space(8.0);
            if dialog_button(ui, "Cancel", false).clicked() {
                close = true;
            }
            ui.add_space(8.0);
            if dialog_button(ui, "Accept Changes", true).clicked() {
                accept = true;
            }
        });
    });
    // `IsDefault`: Enter accepts while nothing covers the dialog.
    let entered = ctx.input(|input| input.key_pressed(Key::Enter));
    if response.is_top_modal && entered {
        accept = true;
    }
    if response.should_close() {
        close = true;
    }
    if accept && !close {
        close = accept_changes(editor, dialog, commands);
    }
    if close {
        *open_dialog = None;
    }
}

/// `BtnOK_OnClick`: a reserved or taken name is refused with a message
/// (and the dialog stays); otherwise the changes are written. Numbers
/// typed but not yet taken count. Returns whether the dialog closes.
fn accept_changes(editor: &mut Editor, dialog: &mut EditVariable, commands: &mut Vec<Command>) -> bool {
    if let Some(problem) = variable_name_problem(&editor.document, &dialog.key, &dialog.name) {
        let caption = format!("Variable name [{}] not allowed", dialog.name);
        commands.push(Command::ShowMessage { caption, text: problem });
        return false;
    }
    for field in [&mut dialog.min, &mut dialog.max] {
        if field.typed() {
            field.commit();
        }
    }
    let limits = dialog.limits();
    editor.document.edit_variable(&dialog.key, &dialog.name, &limits);
    editor.changed();
    true
}

/// "Variable Name" (margin 8) and its box, focused with its text chosen
/// when the dialog opens.
fn name_row(ui: &mut Ui, dialog: &mut EditVariable) {
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let label = RichText::new("Variable Name").font(theme::interface(12.0)).color(theme::LABEL_DEFAULT);
        ui.label(label);
        ui.add_space(8.0);
        let mut edit = egui::TextEdit::singleline(&mut dialog.name);
        edit = edit.font(theme::monospace(12.0));
        edit = edit.desired_width(ui.available_width());
        let response = ui.add(edit);
        if !dialog.focused {
            // `EditVariableDialog_Loaded`: `Focus`, then `SelectAll`.
            response.request_focus();
            let length = dialog.name.chars().count();
            let mut state = egui::TextEdit::load_state(ui.ctx(), response.id).unwrap_or_default();
            let everything = CCursorRange::two(CCursor::new(0), CCursor::new(length));
            state.cursor.set_char_range(Some(everything));
            state.store(ui.ctx(), response.id);
            dialog.focused = true;
        }
    });
}

/// `NumericTextBox`: the header right-aligned in its column (dragging it
/// moves the value), then the box 8 in, monospace in `Accent-Muted-Blue`
/// (`Accent-Muted-Green` for integers). Enter, Tab or leaving the box
/// takes its text.
fn number_field(ui: &mut Ui, id: Id, header: &str, field: &mut NumberField) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(CONTENT_WIDTH, ROW_HEIGHT), Sense::hover());
    let header_rect = Rect::from_min_size(rect.min, Vec2::new(NUMBER_COLUMN, rect.height()));
    let galley = ui.painter().layout_no_wrap(header.to_string(), theme::interface(12.0), theme::LABEL_DEFAULT);
    let header_position = Pos2::new(header_rect.right() - galley.size().x, header_rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(header_position, galley, theme::LABEL_DEFAULT);
    // `thumb`: invisible over the header.
    let thumb = ui.interact(header_rect, id.with("thumb"), Sense::drag());
    if thumb.hovered() || thumb.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if thumb.drag_started() {
        field.drag = Some((field.value, 0.0));
    }
    if thumb.dragged()
        && let Some((start, dragged)) = field.drag
    {
        let distance = dragged + thumb.drag_delta().x;
        let per_point = if field.integer { INTEGER_DRAG_PER_POINT } else { DRAG_PER_POINT };
        field.value = (start + distance * per_point).clamp(-LIMIT, LIMIT);
        field.text = field.value_text();
        field.drag = Some((start, distance));
    }
    if thumb.drag_stopped() {
        field.drag = None;
    }
    let box_rect = Rect::from_min_size(Pos2::new(header_rect.right() + 8.0, rect.top()), Vec2::new(NUMBER_COLUMN, rect.height()));
    let mut box_ui = ui.new_child(UiBuilder::new().max_rect(box_rect));
    let colour = if field.integer { theme::ACCENT_MUTED_GREEN } else { theme::ACCENT_MUTED_BLUE };
    let mut edit = egui::TextEdit::singleline(&mut field.text);
    edit = edit.id(id.with("text"));
    edit = edit.font(theme::monospace(12.0));
    edit = edit.text_color(colour);
    edit = edit.desired_width(NUMBER_COLUMN - 8.0);
    let response = box_ui.add(edit);
    if response.changed() {
        // `NumericOnlyBehavior.IsTextAllowed`: digits and a minus sign, and
        // a point for decimals.
        let allowed = |character: &char| character.is_ascii_digit() || *character == '-' || (*character == '.' && !field.integer);
        field.text = field.text.chars().filter(allowed).collect();
    }
    if response.lost_focus() {
        field.commit();
    }
}

/// `dockChoices`: "Choices" (margin 8) and its box, and the hint under it
/// on the right (10, at 0.7).
fn choices_row(ui: &mut Ui, dialog: &mut EditVariable) {
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let label = RichText::new("Choices").font(theme::interface(12.0)).color(theme::LABEL_DEFAULT);
        ui.label(label);
        ui.add_space(8.0);
        let mut edit = egui::TextEdit::singleline(&mut dialog.choices);
        edit = edit.font(theme::monospace(12.0));
        edit = edit.desired_width(ui.available_width());
        ui.add(edit);
    });
    ui.add_space(6.0);
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        let hint_colour = theme::LABEL_DEFAULT.gamma_multiply(0.7);
        let hint = RichText::new("Comma-separated list, only ASCII A-Z characters and spaces").font(theme::interface(10.0)).color(hint_colour);
        ui.label(hint);
    });
}

/// `cboStringType` (`DropdownX` "String Type").
fn string_type_row(ui: &mut Ui, dialog: &mut EditVariable) {
    let entries: Vec<(String, String)> = STRING_TYPES.iter().enumerate().map(|(index, name)| (index.to_string(), name.to_string())).collect();
    let shown = STRING_TYPES.get(dialog.string_type).copied().unwrap_or("");
    let current = dialog.string_type.to_string();
    let id = window_id().with("string-type");
    let chosen = dropdown_box(ui, id, Some("String Type"), 160.0, ROW_HEIGHT, shown, &entries, &current, theme::interface(12.0));
    if let Some(index) = chosen.and_then(|value| value.parse::<usize>().ok()) {
        dialog.string_type = index;
    }
}

/// `Button`, or with `important` `Button-Important`.
fn dialog_button(ui: &mut Ui, text: &str, important: bool) -> egui::Response {
    let label = RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_EMPHASIS);
    let border = if important { theme::BUTTON_IMPORTANT_BORDER } else { theme::BUTTON_BORDER };
    let mut widget = egui::Button::new(label);
    widget = widget.fill(theme::BUTTON_BACKGROUND);
    widget = widget.stroke(Stroke::new(1.0, border));
    widget = widget.min_size(Vec2::new(80.0, ROW_HEIGHT));
    ui.add(widget)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fields_come_from_the_variable_and_go_back_as_limits() {
        let choice = Variable::from_json(&json!({"Name": "Style", "Value": 0, "Type": "Choice", "ExtraData": {"$values": ["Simple", "Ridge"]}}));
        let mut dialog = EditVariable::new("Style", &choice);
        assert_eq!(dialog.choices, "Simple, Ridge");
        dialog.choices = " Simple ,, Ridge,Peak ".to_string();
        assert_eq!(dialog.limits(), VariableLimits::Choices(vec!["Simple".into(), "Ridge".into(), "Peak".into()]));

        let int = Variable::from_json(&json!({"Name": "Count", "Value": 1, "Type": "Int", "Min": 0, "Max": 10}));
        let mut dialog = EditVariable::new("Count", &int);
        assert_eq!((dialog.min.text.as_str(), dialog.max.text.as_str()), ("0", "10"));
        dialog.max.text = "99999".to_string();
        dialog.max.commit();
        assert_eq!(dialog.limits(), VariableLimits::Int { min: 0, max: 65535 });

        let text = Variable::from_json(&json!({"Name": "Path", "Value": "a", "ExtraData": "2"}));
        assert_eq!(EditVariable::new("Path", &text).limits(), VariableLimits::StringType(2));
    }
}
