//! `EditExpressionDialog` (480 wide, "Edit Expression"), from a
//! parameter's menu (Expression…): the property's expression in a
//! syntax-coloured box (`SyntaxBox`; it grows to five lines, then
//! scrolls), a note on `x`, and a button that shows examples. Accept
//! Changes (Enter outside the box) keeps the text; Close (Escape) and
//! Delete Expression close the dialog. Emptying the box and then accepting
//! or closing asks before the expression goes, as Delete Expression does;
//! No goes back to the dialog only from Accept Changes (Close and Delete
//! Expression are cancel buttons).

use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Color32, Id, Key, Layout, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::syntax_box;

/// The window (480) less its margin (12 each side).
const CONTENT_WIDTH: f32 = 480.0 - 24.0;
/// `TextBox-Border`.
const TEXTBOX_BORDER: Color32 = Color32::from_rgb(0x36, 0x36, 0x36);
/// The box's `MinHeight`, `MaxLines` and `FontSize`.
const BOX_MIN_HEIGHT: f32 = 80.0;
const BOX_MAX_LINES: f32 = 5.0;
const BOX_FONT_SIZE: f32 = 14.0;
/// The box's `Padding`.
const BOX_PADDING: f32 = 4.0;
/// A task dialog's width here, roughly.
const TASK_DIALOG_WIDTH: f32 = 420.0;
/// `ShowHelp_Click`'s examples.
const EXAMPLES: &str = "Some examples of expressions:\n\nInverting the value:\n        1 - x\n\nDividing by other variable:\n        x / Octaves\n\nReducing the power:\n        x * 0.01\n\nUse functions:\n        median(Octaves, Ridge, Scale, Seed)\n\nAdd with a random:\n        random() + x\n\nIf statements:\n        if(SomeVar, 0.3, 0.6)";

/// The dialog while it is open.
#[derive(Clone, Debug)]
pub struct ExpressionDialog {
    node: i32,
    property: String,
    text: String,
    /// Asking whether to delete the expression, and what asked.
    confirm: Option<Asker>,
    /// The examples are showing.
    examples: bool,
}

/// What asked whether to delete the expression: after No, Accept Changes
/// stays open and the others close.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Asker {
    Accept,
    Cancel,
}

/// What the dialog's buttons asked for this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    Accept,
    Close,
    Delete,
}

impl ExpressionDialog {
    /// `ShowExpression`, then `EditExpressionDialog_Loaded`: the box holds
    /// the property's expression, if it has one.
    pub fn open(editor: &Editor, node: i32, property: &str) -> Self {
        let text = editor.document.expression(node, property);
        Self { node, property: property.to_string(), text, confirm: None, examples: false }
    }

    fn has_expression(&self, editor: &Editor) -> bool {
        let key = format!("{}_{}", self.node, self.property);
        editor.document.automation().expressions.contains_key(&key)
    }

    fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

fn box_id() -> Id {
    Id::new("edit-expression-box")
}

/// A dialog's window: `Window-Background` with a margin of 12 (the
/// dialog's `DockPanel Margin="12"`).
fn window(ctx: &egui::Context, id: &str) -> egui::Modal {
    let mut frame = egui::Frame::popup(&ctx.global_style());
    frame.fill = theme::WINDOW_BACKGROUND;
    frame.inner_margin = egui::Margin::same(12);
    let modal = egui::Modal::new(Id::new(id));
    modal.frame(frame)
}

/// The dialog, while one is open, with the task dialogs it opens.
pub fn show(ctx: &egui::Context, editor: &mut Editor, open_dialog: &mut Option<ExpressionDialog>) {
    let Some(dialog) = open_dialog.as_mut() else {
        return;
    };
    if editor.document.node(dialog.node).is_none() {
        *open_dialog = None;
        return;
    }
    let mut action = None;
    let modal = window(ctx, "edit-expression");
    let response = modal.show(ctx, |ui| {
        ui.set_width(CONTENT_WIDTH);
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        let caption = RichText::new("Edit Expression").font(theme::semibold(13.0)).color(theme::LABEL_EMPHASIS);
        ui.label(caption);
        ui.add_space(8.0);
        expression_box(ui, &mut dialog.text);
        ui.add_space(6.0);
        ui.add(egui::Label::new(note()).wrap());
        if examples_button(ui).clicked() {
            dialog.examples = true;
        }
        // The buttons (`DockPanel`, margin 0, 24, 0, 0).
        ui.add_space(24.0);
        let row_height = button_size(ui, "Delete Expression").y;
        ui.allocate_ui_with_layout(Vec2::new(CONTENT_WIDTH, row_height), Layout::left_to_right(Align::Min), |ui| {
            if dialog_button(ui, "Delete Expression", ButtonStyle::Danger).clicked() {
                action = Some(Action::Delete);
            }
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                if dialog_button(ui, "Close", ButtonStyle::Plain).clicked() {
                    action = Some(Action::Close);
                }
                ui.add_space(8.0);
                if dialog_button(ui, "Accept Changes", ButtonStyle::Important).clicked() {
                    action = Some(Action::Accept);
                }
            });
        });
    });
    if response.is_top_modal {
        // `IsDefault`: Enter, where the box has not taken it, accepts.
        if ctx.input(|input| input.key_pressed(Key::Enter)) {
            action = action.or(Some(Action::Accept));
        }
        // `IsCancel`: Escape closes.
        if response.should_close() {
            action = action.or(Some(Action::Close));
        }
    }

    let mut close = false;
    match action {
        // `BtnOK_OnClick`.
        Some(Action::Accept) if dialog.is_empty() && dialog.has_expression(editor) => dialog.confirm = Some(Asker::Accept),
        Some(Action::Accept) if dialog.is_empty() => close = true,
        Some(Action::Accept) => {
            editor.document.set_expression(dialog.node, &dialog.property, &dialog.text);
            editor.changed();
            close = true;
        }
        // `btnClose_Click`.
        Some(Action::Close) if dialog.is_empty() && dialog.has_expression(editor) => dialog.confirm = Some(Asker::Cancel),
        Some(Action::Close) => close = true,
        // `BtnDelete_OnClick`.
        Some(Action::Delete) => dialog.confirm = Some(Asker::Cancel),
        None => {}
    }

    if dialog.examples {
        dialog.examples = show_examples(ctx);
    }
    if let Some(asker) = dialog.confirm {
        match confirm_delete(ctx) {
            Some(true) => {
                editor.document.set_expression(dialog.node, &dialog.property, "");
                editor.changed();
                close = true;
            }
            Some(false) => {
                dialog.confirm = None;
                close = asker == Asker::Cancel;
            }
            None => {}
        }
    }
    if close {
        *open_dialog = None;
    }
}

/// `txtExpression`: the expression in the `SyntaxBox` colours, monospace
/// 14 on `TextBox-Background` in a `TextBox-Border` frame, at least 80
/// tall and growing to five lines before it scrolls. The caret is
/// `Accent-Strong-Bright`.
fn expression_box(ui: &mut Ui, text: &mut String) {
    let font = theme::monospace(BOX_FONT_SIZE);
    let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
    let inner_min = BOX_MIN_HEIGHT - 2.0 * (BOX_PADDING + 1.0);
    let inner_max = (BOX_MAX_LINES * row_height).max(inner_min);
    let id = box_id();
    let changed_by_keys = syntax_box::take_keys(ui, id, text);

    let mut frame = egui::Frame::new();
    frame.fill = theme::TEXTBOX_BACKGROUND;
    frame.stroke = Stroke::new(1.0, TEXTBOX_BORDER);
    frame.inner_margin = egui::Margin::same(BOX_PADDING as i8);
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.visuals_mut().text_cursor.stroke.color = theme::ACCENT_BRIGHT;
        let mut layouter = |ui: &Ui, buffer: &dyn egui::TextBuffer, wrap_width: f32| {
            let mut job = syntax_box::highlight(buffer.as_str(), theme::monospace(BOX_FONT_SIZE));
            job.wrap.max_width = wrap_width;
            ui.fonts_mut(|fonts| fonts.layout_job(job))
        };
        let mut scroll = egui::ScrollArea::vertical();
        scroll = scroll.id_salt("edit-expression-scroll");
        scroll = scroll.max_height(inner_max);
        scroll = scroll.auto_shrink([false, true]);
        scroll.show(ui, |ui| {
            let mut edit = egui::TextEdit::multiline(text);
            edit = edit.id(id);
            edit = edit.font(font.clone());
            edit = edit.frame(egui::Frame::NONE);
            edit = edit.desired_width(f32::INFINITY);
            edit = edit.desired_rows(1);
            edit = edit.min_size(Vec2::new(0.0, inner_min));
            // Tab and Enter are the `SyntaxBox`'s.
            edit = edit.lock_focus(true);
            edit = edit.return_key(None);
            edit = edit.layouter(&mut layouter);
            ui.add(edit);
        });
    });
    if changed_by_keys {
        ui.ctx().request_repaint();
    }
}

/// "Reprocess any value…" (10, at 0.7), with `x` in `Accent-Muted-Teal`.
fn note() -> LayoutJob {
    let font = theme::interface(10.0);
    let plain = TextFormat::simple(font.clone(), theme::LABEL_DEFAULT.gamma_multiply(0.7));
    let marked = TextFormat::simple(font, theme::ACCENT_MUTED_TEAL.gamma_multiply(0.7));
    let mut job = LayoutJob::default();
    job.append("Reprocess any value (from the slider or bound variable). Use ", 0.0, plain.clone());
    job.append("x", 0.0, marked);
    job.append(" to represent the incoming value. You can also reference other variables.", 0.0, plain);
    job
}

/// `Button-IconX-Hover` with `info-square-rounded` (18, on the right):
/// shows the examples.
fn examples_button(ui: &mut Ui) -> egui::Response {
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(Vec2::new(width, 18.0), Sense::hover());
    let rect = Rect::from_min_size(Pos2::new(row.right() - 18.0, row.top()), Vec2::splat(18.0));
    let response = ui.interact(rect, Id::new("edit-expression-examples"), Sense::click());
    let colour = if response.hovered() { theme::ACCENT_GRAY } else { theme::ICON_DEFAULT };
    icons::paint(ui, rect, "info-square-rounded", colour);
    response.on_hover_text("Show examples of expressions")
}

#[derive(Clone, Copy, PartialEq)]
enum ButtonStyle {
    /// `Button`.
    Plain,
    /// `Button-Important`.
    Important,
    /// `Button-Danger`.
    Danger,
}

/// A button's size (`MinWidth` 87, `Padding` 24, 6).
fn button_size(ui: &Ui, text: &str) -> Vec2 {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::interface(12.0), theme::LABEL_DEFAULT);
    Vec2::new((galley.size().x + 48.0).max(87.0), galley.size().y + 12.0)
}

/// `Button-Template`: `Button-Background` (`Button-Hover` under the
/// pointer, `Button-Pressed` held), corners of 2, the text in
/// `Label-Default`, and the style's border.
fn dialog_button(ui: &mut Ui, text: &str, style: ButtonStyle) -> egui::Response {
    let size = button_size(ui, text);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = if response.is_pointer_button_down_on() {
        theme::BUTTON_PRESSED
    } else if response.hovered() {
        theme::BUTTON_HOVER
    } else {
        theme::BUTTON_BACKGROUND
    };
    let stroke = match style {
        ButtonStyle::Plain => Stroke::new(1.0, theme::BUTTON_BORDER),
        ButtonStyle::Important => Stroke::new(1.0, theme::BUTTON_IMPORTANT_BORDER),
        ButtonStyle::Danger => Stroke::new(1.5, theme::ACCENT_RED),
    };
    ui.painter().rect_filled(rect, 2.0, fill);
    ui.painter().rect_stroke(rect, 2.0, stroke, StrokeKind::Inside);
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::interface(12.0), theme::LABEL_DEFAULT);
    let position = rect.center() - galley.size() / 2.0;
    ui.painter().galley(position, galley, theme::LABEL_DEFAULT);
    response
}

/// A task dialog's caption, then its main instruction beside its icon.
fn task_heading(ui: &mut Ui, caption: &str, icon: &str, accent: Color32, heading: &str) {
    ui.label(RichText::new(caption).font(theme::interface(11.0)).color(theme::LABEL_DEFAULT));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
        icons::paint(ui, icon_rect, icon, accent);
        ui.add_space(6.0);
        ui.label(RichText::new(heading).font(theme::semibold(14.0)).color(accent));
    });
}

/// `ShowHelp_Click` (`TD.Ok`): the examples, with Close and a footnote.
/// Returns whether it stays open.
fn show_examples(ctx: &egui::Context) -> bool {
    let mut open = true;
    let modal = window(ctx, "expression-examples");
    let response = modal.show(ctx, |ui| {
        ui.set_width(TASK_DIALOG_WIDTH);
        task_heading(ui, "Expression Examples", "info-square-rounded", theme::ACCENT_MUTED_BLUE, "Expression Examples");
        ui.add_space(8.0);
        let text = RichText::new(EXAMPLES).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT);
        ui.add(egui::Label::new(text).wrap());
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if dialog_button(ui, "Close", ButtonStyle::Plain).clicked() {
                open = false;
            }
        });
        ui.add_space(10.0);
        ui.separator();
        // The footnote, with `TaskDialogIcon.Warning`.
        ui.horizontal(|ui| {
            let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(icons::DEFAULT_SIZE), Sense::hover());
            icons::paint(ui, icon_rect, "alert-triangle", theme::ACCENT_YELLOW);
            ui.add_space(4.0);
            let footnote = RichText::new("See documentation for more information.").font(theme::interface(11.0)).color(theme::LABEL_DEFAULT);
            ui.label(footnote);
        });
    });
    let entered = response.is_top_modal && ctx.input(|input| input.key_pressed(Key::Enter));
    open && !entered && !response.should_close()
}

/// "Are you sure you want to delete this expression?" (`TD.YesNo`, a
/// warning, No by default): Some(true) for Yes, Some(false) for No,
/// Escape or a click outside, None while it waits.
fn confirm_delete(ctx: &egui::Context) -> Option<bool> {
    let mut answer = None;
    let modal = window(ctx, "delete-expression");
    let response = modal.show(ctx, |ui| {
        ui.set_width(TASK_DIALOG_WIDTH);
        task_heading(ui, "Delete Expression", "alert-triangle", theme::ACCENT_YELLOW, "Are you sure you want to delete this expression?");
        ui.add_space(14.0);
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if dialog_button(ui, "No", ButtonStyle::Plain).clicked() {
                answer = Some(false);
            }
            ui.add_space(8.0);
            if dialog_button(ui, "Yes", ButtonStyle::Plain).clicked() {
                answer = Some(true);
            }
        });
    });
    if response.is_top_modal && ctx.input(|input| input.key_pressed(Key::Enter)) {
        answer = answer.or(Some(false));
    }
    if response.should_close() {
        answer = answer.or(Some(false));
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_note_marks_x() {
        let job = note();
        assert_eq!(job.text, "Reprocess any value (from the slider or bound variable). Use x to represent the incoming value. You can also reference other variables.");
        let teal = theme::ACCENT_MUTED_TEAL.gamma_multiply(0.7);
        let marked_sections = job.sections.iter().filter(|section| section.format.color == teal);
        let marked: Vec<&str> = marked_sections.map(|section| &job.text[section.byte_range.start.0..section.byte_range.end.0]).collect();
        assert_eq!(marked, vec!["x"]);
    }
}
