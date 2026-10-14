//! `VariableEditor`: a parameter bound to a variable shows the variable in
//! place of its own editor: the variable's key, then its value (through
//! the parameter's expression when it reads as a number). A click opens a
//! box to type the variable's value, a double-click the Data View's
//! variables (`TabVars`).

use egui::{Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use nerthus_engine::automation::{dotnet_float_string, Automation, ExpressionContext, VariableValue};
use nerthus_engine::{ParameterSpec, ValueType};

use crate::theme;
use crate::ui::editors::Edit;
use crate::ui::slider_popups::{self, BoxWidth, TypingLook};

/// The control's margin in its row (20, 0, 4, 0) and padding (16, 6).
const MARGIN_LEFT: f32 = 20.0;
const MARGIN_RIGHT: f32 = 4.0;
const PADDING: Vec2 = Vec2::new(16.0, 6.0);
/// The key's column (96 wide, 8 before the value).
const KEY_WIDTH: f32 = 96.0;
const KEY_GAP: f32 = 8.0;
/// The typing box: 112 in, as wide as the control less 128.
const TYPING_LEFT: f32 = 112.0;
const TYPING_TRIM: f32 = 128.0;

/// What the control needs to know about the binding.
pub struct Bound<'a> {
    pub node: i32,
    pub parameter: &'a ParameterSpec,
    /// The variable's key (`VarBinding.Variable`).
    pub key: &'a str,
    pub automation: &'a Automation,
    pub context: &'a ExpressionContext,
}

/// Draws the control; a click types a new value (`Edit::BoundText` once
/// taken), a double-click asks for the variables.
pub fn bound_variable(ui: &mut Ui, id: Id, bound: &Bound) -> Edit {
    let font = theme::monospace(12.0);
    let line = ui.fonts_mut(|fonts| fonts.row_height(&font));
    let height = line + 2.0 * PADDING.y + 2.0;
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let control = Rect::from_min_max(Pos2::new(row.left() + MARGIN_LEFT, row.top()), Pos2::new(row.right() - MARGIN_RIGHT, row.bottom()));
    let response = ui.interact(control, id.with("bound"), Sense::click());
    let enabled = ui.is_enabled();

    // `Button-Background`, darker under the pointer and while pressed;
    // disabled, `Button-Disabled-Background` and the text at half.
    let fill = if !enabled {
        theme::BUTTON_DISABLED_BACKGROUND
    } else if response.is_pointer_button_down_on() {
        theme::BUTTON_PRESSED
    } else if response.hovered() {
        theme::BUTTON_HOVER
    } else {
        theme::BUTTON_BACKGROUND
    };
    let painter = ui.painter();
    painter.rect_filled(control, 2.0, fill);
    painter.rect_stroke(control, 2.0, Stroke::new(1.0, theme::BUTTON_BORDER), StrokeKind::Inside);
    let content = control.shrink(1.0).shrink2(PADDING);
    let text_opacity = if enabled { 1.0 } else { 0.5 };
    let key_color = theme::LABEL_DEFAULT;
    let key_galley = painter.layout_no_wrap(bound.key.to_string(), font.clone(), key_color);
    let key_rect = Rect::from_min_size(content.min, Vec2::new(KEY_WIDTH, content.height()));
    let key_top = content.center().y - key_galley.size().y / 2.0;
    painter.with_clip_rect(key_rect.intersect(control)).galley(Pos2::new(key_rect.left(), key_top), key_galley, key_color);
    let value_color = theme::ACCENT_MUTED_GREEN.gamma_multiply(text_opacity);
    let value_galley = painter.layout_no_wrap(shown_value(bound), font, value_color);
    let value_left = key_rect.right() + KEY_GAP;
    let value_area = Rect::from_min_max(Pos2::new(value_left, control.top()), control.max);
    painter.with_clip_rect(value_area).galley(Pos2::new(value_left, key_top), value_galley, value_color);

    let typing_id = id.with("bound-typing");
    let mut edit = Edit::None;
    if enabled && response.double_clicked() {
        slider_popups::close_typing(ui.ctx(), typing_id);
        edit = Edit::ShowVariables;
    } else if enabled && response.clicked() {
        slider_popups::open_typing(ui.ctx(), typing_id, raw_value(bound));
    }
    // In the popup's own units.
    let popup_scale = slider_popups::popup_scale(ui.ctx());
    let look = TypingLook {
        top_left: Pos2::new(control.left() + TYPING_LEFT * popup_scale, control.top()),
        width: BoxWidth::Fixed(((control.width() - TYPING_TRIM) * popup_scale).max(32.0 * popup_scale)),
        font: slider_popups::value_font(),
    };
    if let Some(text) = slider_popups::typing_box(ui, typing_id, &look)
        && text != raw_value(bound)
    {
        edit = Edit::BoundText(text);
    }
    edit
}

/// The tooltip: the parameter's help when it has some, otherwise what it
/// is bound to.
pub fn tooltip(parameter: &ParameterSpec, key: &str) -> String {
    if let Some(help) = parameter.description.as_ref().filter(|help| !help.is_empty()) {
        return help.clone();
    }
    let property = nerthus_engine::humanize::humanize_title(&parameter.name);
    format!("{property} bound to the variable [{key}]")
}

/// `Variable.Value?.ToString()`: what the typing box starts from.
fn raw_value(bound: &Bound) -> String {
    let variable = bound.automation.variables.get(bound.key);
    variable.map(|variable| variable.value.to_text()).unwrap_or_default()
}

/// The control's content: the variable's value as text (`<NULL>` without
/// one), or, when that reads as a number, the parameter's expression of it.
fn shown_value(bound: &Bound) -> String {
    let Some(variable) = bound.automation.variables.get(bound.key) else {
        return String::new();
    };
    let text = match &variable.value {
        VariableValue::Null => "<NULL>".to_string(),
        value => value.to_text(),
    };
    let Ok(number) = VariableValue::Text(text.clone()).to_single() else {
        return text;
    };
    let expressed = bound.automation.apply_expression(bound.node, &bound.parameter.name, number, bound.context);
    dotnet_float_string(expressed)
}

/// `Convert.ChangeType(text, PropertyType)`: whether the typed text makes
/// a value of the parameter's type. Enums, ranges, colours and lists
/// never do from text.
pub fn converts(parameter: &ParameterSpec, text: &str) -> bool {
    let value = VariableValue::Text(text.to_string());
    match parameter.value_type {
        ValueType::Float | ValueType::Double => value.to_single().is_ok(),
        ValueType::Int => value.to_int32().is_ok(),
        ValueType::Byte => value.to_int32().is_ok_and(|integer| u8::try_from(integer).is_ok()),
        ValueType::Bool => value.to_boolean().is_ok(),
        ValueType::String => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerthus_engine::Catalogue;

    fn parameter(node: &str, name: &str) -> ParameterSpec {
        let spec = Catalogue::get().node(node).unwrap();
        spec.parameter(name).unwrap().clone()
    }

    #[test]
    fn typed_values_must_convert_to_the_parameter_type() {
        let scale = parameter("Perlin", "Scale");
        assert!(converts(&scale, "0.5"));
        assert!(converts(&scale, " 2 "));
        assert!(!converts(&scale, "1/2"));
        let octaves = parameter("Perlin", "Octaves");
        assert!(converts(&octaves, "7"));
        assert!(!converts(&octaves, "7.5"));
        let enum_parameter = Catalogue::get().node("Perlin").unwrap().parameters.iter().find(|parameter| parameter.value_type == ValueType::Enum).cloned();
        if let Some(enum_parameter) = enum_parameter {
            assert!(!converts(&enum_parameter, "0"));
        }
    }

    fn float_variable(key: &str, value: f32) -> Automation {
        let mut automation = Automation::default();
        let json = serde_json::json!({"Name": key, "Value": value, "Type": "Float"});
        automation.variables.insert(key.to_string(), nerthus_engine::automation::Variable::from_json(&json));
        automation
    }

    fn themed_context() -> egui::Context {
        let context = egui::Context::default();
        theme::apply(&context);
        let output = context.run_ui(egui::RawInput::default(), |_| {});
        output.drop_without_applying_deltas();
        context
    }

    /// One frame of the control with these events.
    fn bound_frame(context: &egui::Context, bound: &Bound, events: Vec<egui::Event>) -> Edit {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0));
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut edit = Edit::None;
        let output = context.run_ui(input, |ui| {
            let mut row = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(Pos2::new(0.0, 40.0), Vec2::new(300.0, 40.0))));
            edit = bound_variable(&mut row, Id::new("bound-test"), bound);
        });
        output.drop_without_applying_deltas();
        edit
    }

    fn click(context: &egui::Context, bound: &Bound, at: Pos2) -> Vec<Edit> {
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        let events = [egui::Event::PointerMoved(at), press(true), press(false)];
        events.into_iter().map(|event| bound_frame(context, bound, vec![event])).collect()
    }

    #[test]
    fn the_value_shows_through_the_expression() {
        let scale = parameter("Perlin", "Scale");
        let mut automation = float_variable("Var_1", 0.5);
        let context = ExpressionContext { resolution: nerthus_engine::Resolution::square(256), terrain_height: 1000.0 };
        let bound = Bound { node: 7, parameter: &scale, key: "Var_1", automation: &automation, context: &context };
        assert_eq!(shown_value(&bound), "0.5");
        automation.expressions.insert("7_Scale".to_string(), "x * 3".to_string());
        let bound = Bound { node: 7, parameter: &scale, key: "Var_1", automation: &automation, context: &context };
        assert_eq!(shown_value(&bound), "1.5");
        let missing = Bound { node: 7, parameter: &scale, key: "Var_9", automation: &automation, context: &context };
        assert_eq!(shown_value(&missing), "");
    }

    #[test]
    fn a_click_types_the_value_and_a_double_click_asks_for_the_variables() {
        let ui_context = themed_context();
        let scale = parameter("Perlin", "Scale");
        let automation = float_variable("Var_1", 0.5);
        let context = ExpressionContext { resolution: nerthus_engine::Resolution::square(256), terrain_height: 1000.0 };
        let bound = Bound { node: 7, parameter: &scale, key: "Var_1", automation: &automation, context: &context };
        let on_control = Pos2::new(60.0, 55.0);
        let edits = click(&ui_context, &bound, on_control);
        assert!(edits.iter().all(|edit| matches!(edit, Edit::None)));
        assert_eq!(slider_popups::typing_text(&ui_context, Id::new("bound-test").with("bound-typing")).as_deref(), Some("0.5"));
        bound_frame(&ui_context, &bound, vec![egui::Event::Text("0.75".to_string())]);
        let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let edit = bound_frame(&ui_context, &bound, vec![enter]);
        assert!(matches!(&edit, Edit::BoundText(text) if text == "0.75"));
        // Two clicks in a row (half a second after the last): the
        // variables, and no box.
        for _ in 0..30 {
            bound_frame(&ui_context, &bound, Vec::new());
        }
        let first = click(&ui_context, &bound, on_control);
        let second = click(&ui_context, &bound, on_control);
        assert!(first.iter().chain(&second).any(|edit| matches!(edit, Edit::ShowVariables)));
        assert!(slider_popups::typing_text(&ui_context, Id::new("bound-test").with("bound-typing")).is_none());
    }

    #[test]
    fn the_tooltip_names_the_variable_unless_there_is_help() {
        let mut scale = parameter("Perlin", "Scale");
        scale.description = None;
        assert_eq!(tooltip(&scale, "Var_1"), "Scale bound to the variable [Var_1]");
        scale.description = Some("How big.".to_string());
        assert_eq!(tooltip(&scale, "Var_1"), "How big.");
    }
}
