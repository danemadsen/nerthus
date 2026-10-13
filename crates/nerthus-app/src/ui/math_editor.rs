//! The Math node's editor window (`EditMath`, the node's Open Editor
//! command): its expression in a syntax-coloured box (`SyntaxBox` rules),
//! checked half a second after each change (`RunValidation` runs the
//! expression over a 16 × 16 map), with Apply and Run, Accept Changes and
//! Cancel. The window stays on top and does not block the editor.

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, Id, RichText, Stroke, Vec2};
use nerthus_engine::automation::VariableValue;
use serde_json::Value;

use crate::editor::Editor;
use crate::icons;
use crate::theme;

/// `debounceInterval`.
const DEBOUNCE_SECONDS: f64 = 0.5;
/// The map the check runs over.
const CHECK_SIZE: usize = 16;

const KEYWORDS: [&str; 10] = ["if", "else", "return", "while", "for", "break", "continue", "switch", "case", "default"];
const DECLARATIONS: [&str; 7] = ["var", "local", "constant", "infix", "prefix", "postfix", "operator"];
const FUNCTIONS: [&str; 24] = [
    "abs", "acos", "asin", "atan", "atan2", "ceil", "cos", "cosh", "exp", "floor", "ln", "log", "log10", "max", "min", "pow", "rand", "round", "sin", "sinh", "sqrt", "tan", "tanh", "trunc",
];
const OPERATORS: [char; 13] = ['+', '-', '/', '*', '%', '=', '!', '^', '|', '&', '<', '>', ','];
const MUTED_YELLOW: Color32 = Color32::from_rgb(0xcb, 0xcd, 0x77);

/// The window while it is open.
#[derive(Clone, Debug)]
struct MathEditor {
    node: i32,
    text: String,
    /// `oldExpression`: what Cancel puts back.
    old_expression: String,
    /// When the text last changed, for the delayed check.
    changed_at: Option<f64>,
    /// `result`: the check's message and whether it passed.
    result: Option<(String, bool)>,
}

fn window_id() -> Id {
    Id::new("math-editor")
}

/// `Editor_CommandExecuted("OpenEditor")`: the window for a Math node.
pub fn open(ctx: &egui::Context, editor: &Editor, node: i32) {
    let expression = editor.document.node(node).and_then(|data| data.values.get("Expression")).and_then(Value::as_str).unwrap_or_default().to_string();
    let state = MathEditor { node, text: expression.clone(), old_expression: expression, changed_at: None, result: None };
    ctx.data_mut(|data| data.insert_temp(window_id(), state));
}

/// Draws the window while it is open and applies its buttons.
pub fn show(ctx: &egui::Context, editor: &mut Editor) {
    let Some(mut state) = ctx.data(|data| data.get_temp::<MathEditor>(window_id())) else {
        return;
    };
    if editor.document.node(state.node).is_none() {
        ctx.data_mut(|data| data.remove::<MathEditor>(window_id()));
        return;
    }
    let now = ctx.input(|input| input.time);
    if let Some(changed_at) = state.changed_at {
        if now - changed_at >= DEBOUNCE_SECONDS {
            state.changed_at = None;
            state.result = Some(check_expression(editor, state.node, &state.text));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(DEBOUNCE_SECONDS - (now - changed_at)));
        }
    }

    let mut open = true;
    let mut action = None;
    let mut window = egui::Window::new("Edit Math Node");
    window = window.id(window_id().with("window"));
    window = window.open(&mut open);
    // 480 × 400 at the interface scale (the window's `UIScaler`).
    window = window.default_size(Vec2::new(480.0, 400.0));
    window = window.collapsible(false);
    window = window.order(egui::Order::Foreground);
    window = window.frame(egui::Frame::window(&ctx.global_style()).fill(theme::WINDOW_BACKGROUND).inner_margin(12));
    window.show(ctx, |ui| {
        let bottom_height = 24.0 + 28.0;
        let help = RichText::new("Use a, b, c to access inputs. Use x, y for coordinates.").font(theme::interface(12.0)).color(theme::LABEL_DEFAULT.gamma_multiply(0.7));
        let editor_height = (ui.available_height() - bottom_height - 60.0).max(80.0);
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
            let mut job = highlight(text.as_str());
            job.wrap.max_width = wrap_width;
            ui.fonts_mut(|fonts| fonts.layout_job(job))
        };
        ui.add_space(8.0);
        let mut text_edit = egui::TextEdit::multiline(&mut state.text);
        text_edit = text_edit.font(theme::monospace(14.0));
        text_edit = text_edit.desired_width(f32::INFINITY);
        text_edit = text_edit.code_editor();
        text_edit = text_edit.layouter(&mut layouter);
        let scroll = egui::ScrollArea::vertical().max_height(editor_height).id_salt("math-expression");
        let response = scroll.show(ui, |ui| ui.add_sized(Vec2::new(ui.available_width(), editor_height), text_edit)).inner;
        if response.changed() {
            state.changed_at = Some(now);
        }
        ui.add_space(8.0);
        ui.add(egui::Label::new(help).wrap());
        if let Some((message, valid)) = &state.result {
            let accent = if *valid { theme::ACCENT_MUTED_GREEN } else { theme::ACCENT_MUTED_RED };
            let result = RichText::new(message.as_str()).font(theme::monospace_bold(12.0)).color(accent.gamma_multiply(0.7));
            ui.add_space(6.0);
            ui.add(egui::Label::new(result).wrap());
        }
        ui.add_space(24.0);
        ui.horizontal(|ui| {
            if apply_button(ui).clicked() {
                action = Some(MathAction::Apply);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if dialog_button(ui, "Cancel", false).clicked() {
                    action = Some(MathAction::Cancel);
                }
                ui.add_space(8.0);
                if dialog_button(ui, "Accept Changes", true).clicked() {
                    action = Some(MathAction::Accept);
                }
            });
        });
    });
    if !open {
        action = action.or(Some(MathAction::Cancel));
    }
    let node = state.node;
    match action {
        // `BtnValidate_OnClick`: the expression set and the node built.
        Some(MathAction::Apply) => set_expression(editor, node, &state.text),
        // `BtnOK_OnClick`.
        Some(MathAction::Accept) => {
            set_expression(editor, node, &state.text);
            ctx.data_mut(|data| data.remove::<MathEditor>(window_id()));
            return;
        }
        // `btnClose_Click`: back to what it was when the window opened.
        Some(MathAction::Cancel) => {
            set_expression(editor, node, &state.old_expression);
            ctx.data_mut(|data| data.remove::<MathEditor>(window_id()));
            return;
        }
        None => {}
    }
    ctx.data_mut(|data| data.insert_temp(window_id(), state));
}

#[derive(Clone, Copy, PartialEq)]
enum MathAction {
    Apply,
    Accept,
    Cancel,
}

fn set_expression(editor: &mut Editor, node: i32, text: &str) {
    let current = editor.document.node(node).and_then(|data| data.values.get("Expression")).and_then(Value::as_str).unwrap_or_default().to_string();
    if current == text {
        return;
    }
    editor.document.set_value(node, "Expression", Value::from(text));
    editor.changed();
}

/// `RunValidation`: `MathX.RunComputation` over a 16 × 16 map of zeros
/// with the project's variables (at the preview's
/// resolution): ExprTk's message, or "Expression is valid.".
fn check_expression(editor: &Editor, node: i32, text: &str) -> (String, bool) {
    let automation = editor.document.automation();
    let variables: Vec<(String, nerthus_engine::automation::VarType, VariableValue)> = automation.variables.values().map(|variable| (variable.name.clone(), variable.var_type, variable.value.clone())).collect();
    let variables_text = nerthus_nodes::mathx::variables_string(editor.preview_resolution, &variables);
    let normalized = editor.document.node(node).and_then(|data| data.values.get("NormalizedCoordinates")).and_then(Value::as_bool).unwrap_or(false);
    let zeros = vec![0.0f32; CHECK_SIZE * CHECK_SIZE];
    let inputs = [Some(zeros.as_slice()), Some(zeros.as_slice()), Some(zeros.as_slice())];
    match nerthus_nodes::mathx::compute(text, &variables_text, inputs, CHECK_SIZE, CHECK_SIZE, normalized) {
        Ok(_) => ("Expression is valid.".to_string(), true),
        Err(error) => (nerthus_nodes::mathx::node_message(&error), false),
    }
}

/// The `SyntaxBox` rules, each painting over the ones before it: keywords,
/// declarations and functions (whole words), operators (anywhere),
/// numbers, brackets, strings, then line and block comments.
fn highlight(text: &str) -> LayoutJob {
    let characters: Vec<char> = text.chars().collect();
    let mut colours = vec![theme::LABEL_EMPHASIS; characters.len()];
    let paint = |colours: &mut Vec<Color32>, from: usize, to: usize, colour: Color32| {
        for slot in colours.iter_mut().take(to).skip(from) {
            *slot = colour;
        }
    };
    let is_word = |character: char| character.is_alphanumeric() || character == '_';
    // Whole words.
    let mut index = 0;
    while index < characters.len() {
        if !is_word(characters[index]) {
            index += 1;
            continue;
        }
        let start = index;
        while index < characters.len() && is_word(characters[index]) {
            index += 1;
        }
        let word: String = characters[start..index].iter().collect();
        let colour = if KEYWORDS.contains(&word.as_str()) {
            Some(theme::ACCENT_MUTED_BLUE)
        } else if DECLARATIONS.contains(&word.as_str()) {
            Some(theme::ACCENT_MUTED_TEAL)
        } else if FUNCTIONS.contains(&word.as_str()) {
            Some(theme::ACCENT_MUTED_PURPLE)
        } else {
            None
        };
        if let Some(colour) = colour {
            paint(&mut colours, start, index, colour);
        }
    }
    for (position, character) in characters.iter().enumerate() {
        if OPERATORS.contains(character) {
            colours[position] = theme::ACCENT_MUTED_RED;
        }
    }
    // `\b\d+\.?\d*\b`.
    let mut index = 0;
    while index < characters.len() {
        let at_boundary = index == 0 || !is_word(characters[index - 1]);
        if !(at_boundary && characters[index].is_ascii_digit()) {
            index += 1;
            continue;
        }
        let start = index;
        while index < characters.len() && characters[index].is_ascii_digit() {
            index += 1;
        }
        if index < characters.len() && characters[index] == '.' {
            index += 1;
            while index < characters.len() && characters[index].is_ascii_digit() {
                index += 1;
            }
        }
        let at_end = index >= characters.len() || !is_word(characters[index]);
        if at_end {
            paint(&mut colours, start, index, theme::ACCENT_GRAY);
        }
    }
    for (position, character) in characters.iter().enumerate() {
        if matches!(character, '(' | ')' | '[' | ']' | '{' | '}') {
            colours[position] = MUTED_YELLOW;
        }
    }
    // Strings, then comments.
    let mut index = 0;
    while index < characters.len() {
        if characters[index] != '"' {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        while index < characters.len() && characters[index] != '"' && characters[index] != '\n' {
            index += if characters[index] == '\\' { 2 } else { 1 };
        }
        if index < characters.len() && characters[index] == '"' {
            index += 1;
            paint(&mut colours, start, index, theme::ACCENT_BRIGHT);
        }
    }
    let mut index = 0;
    while index + 1 < characters.len() {
        if characters[index] == '/' && characters[index + 1] == '/' {
            let end = characters[index..].iter().position(|character| *character == '\n').map_or(characters.len(), |offset| index + offset);
            paint(&mut colours, index, end, theme::ACCENT_MUTED_GREEN);
            index = end;
            continue;
        }
        if characters[index] == '/' && characters[index + 1] == '*' {
            let closing = (index + 2..characters.len().saturating_sub(1)).find(|position| characters[*position] == '*' && characters[*position + 1] == '/');
            if let Some(closing) = closing {
                paint(&mut colours, index, closing + 2, theme::ACCENT_MUTED_GREEN);
                index = closing + 2;
                continue;
            }
        }
        index += 1;
    }

    let mut job = LayoutJob::default();
    let font = theme::monospace(14.0);
    let mut start = 0;
    while start < characters.len() {
        let colour = colours[start];
        let mut end = start;
        while end < characters.len() && colours[end] == colour {
            end += 1;
        }
        let piece: String = characters[start..end].iter().collect();
        job.append(&piece, 0.0, TextFormat::simple(font.clone(), colour));
        start = end;
    }
    job
}

/// `Button-IconText-Bordered` with `player-play`: Apply and Run.
fn apply_button(ui: &mut egui::Ui) -> egui::Response {
    let galley = ui.painter().layout_no_wrap("Apply and Run".to_string(), theme::interface(12.0), theme::LABEL_EMPHASIS);
    let size = Vec2::new(8.0 + 16.0 + 6.0 + galley.size().x + 10.0, 28.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let fill = if response.hovered() { theme::BUTTON_HOVER } else { theme::BUTTON_BACKGROUND };
    ui.painter().rect_filled(rect, 2.0, fill);
    ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, theme::BUTTON_BORDER), egui::StrokeKind::Inside);
    let icon_rect = egui::Rect::from_min_size(egui::Pos2::new(rect.left() + 8.0, rect.center().y - 8.0), Vec2::splat(16.0));
    icons::paint(ui, icon_rect, "player-play", theme::ACCENT_GREEN);
    ui.painter().galley(egui::Pos2::new(icon_rect.right() + 6.0, rect.center().y - galley.size().y / 2.0), galley, theme::LABEL_EMPHASIS);
    response
}

/// `Button`, or with `important` `Button-Important`.
fn dialog_button(ui: &mut egui::Ui, text: &str, important: bool) -> egui::Response {
    let label = RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_EMPHASIS);
    let border = if important { theme::BUTTON_IMPORTANT_BORDER } else { theme::BUTTON_BORDER };
    let mut widget = egui::Button::new(label);
    widget = widget.fill(theme::BUTTON_BACKGROUND);
    widget = widget.stroke(Stroke::new(1.0, border));
    widget = widget.min_size(Vec2::new(80.0, 28.0));
    ui.add(widget)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_paint_over_each_other() {
        let job = highlight("sin(a) + 2 // twice");
        let section_text = |section: &egui::text::LayoutSection| job.text[section.byte_range.start.0..section.byte_range.end.0].to_string();
        let colour_of = |text: &str| job.sections.iter().find(|section| section_text(section) == text).map(|section| section.format.color);
        assert_eq!(colour_of("sin"), Some(theme::ACCENT_MUTED_PURPLE));
        assert_eq!(colour_of("("), Some(MUTED_YELLOW));
        assert_eq!(colour_of("2"), Some(theme::ACCENT_GRAY));
        assert_eq!(colour_of("// twice"), Some(theme::ACCENT_MUTED_GREEN));
    }
}
