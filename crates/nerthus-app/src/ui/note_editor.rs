//! The Note Editor (`NoteWindow`, opened by a note's Edit Note or F2): the
//! note's Markdown on the left and how it reads on the right, in a 720
//! wide window; Close (or Escape, `IsCancel`) keeps the text.

use egui::{Align, Frame, Id, Key, Layout, Margin, RichText, ScrollArea, Stroke, Vec2};

use crate::editor::Editor;
use crate::theme;
use crate::ui::graph_canvas::GraphState;
use crate::ui::graph_notes::markdown_job;

/// `Width="720" Height="640"` less the grid's margin (20, 10, 20, 30).
const PANEL_SIZE: Vec2 = Vec2::new(680.0, 600.0);

pub fn show(ctx: &egui::Context, editor: &mut Editor, state: &mut GraphState) {
    let Some((id, mut text)) = state.editing_note.take() else {
        return;
    };
    let mut close = ctx.input(|input| input.key_pressed(Key::Escape));
    let mut frame = Frame::new();
    frame = frame.fill(theme::MENU_BACKGROUND);
    frame = frame.stroke(Stroke::new(1.0, theme::MENU_BORDER));
    frame = frame.corner_radius(6.0);
    frame = frame.inner_margin(Margin::same(13));
    let modal = egui::Modal::new(Id::new("note-editor")).frame(frame);
    modal.show(ctx, |ui| {
        ui.set_min_size(PANEL_SIZE - Vec2::splat(26.0));
        ui.set_max_width(PANEL_SIZE.x - 26.0);
        let caption = RichText::new("Note Editor").font(theme::semibold(13.0)).color(theme::LABEL_EMPHASIS);
        ui.label(caption);
        ui.add_space(6.0);
        let column_width = (ui.available_width() - 20.0) / 2.0;
        let column_height = PANEL_SIZE.y - 100.0;
        ui.horizontal_top(|ui| {
            ui.allocate_ui(Vec2::new(column_width, column_height), |ui| {
                let mut edit = egui::TextEdit::multiline(&mut text);
                edit = edit.font(theme::interface(12.0));
                edit = edit.desired_width(column_width);
                edit = edit.desired_rows(24);
                edit = edit.id(Id::new(("note-editor-text", id)));
                ScrollArea::vertical().id_salt("note-editor-input").max_height(column_height).show(ui, |ui| {
                    let response = ui.add(edit);
                    if !response.has_focus() && !response.lost_focus() {
                        response.request_focus();
                    }
                });
            });
            ui.add_space(10.0);
            ui.allocate_ui(Vec2::new(column_width, column_height), |ui| {
                ScrollArea::vertical().id_salt("note-editor-output").max_height(column_height).show(ui, |ui| {
                    let job = markdown_job(&text, 1.0, column_width - 10.0);
                    let galley = ui.painter().layout_job(job);
                    ui.label(galley);
                });
            });
        });
        ui.add_space(8.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let label = RichText::new("Close").font(theme::interface(12.0)).color(theme::LABEL_EMPHASIS);
            let mut button = egui::Button::new(label);
            button = button.fill(theme::BUTTON_BACKGROUND);
            button = button.stroke(Stroke::new(1.0, theme::BUTTON_IMPORTANT_BORDER));
            button = button.min_size(Vec2::new(80.0, 26.0));
            if ui.add(button).clicked() {
                close = true;
            }
        });
    });
    if close {
        // `btnCancel_Click`: the note takes the text.
        editor.document.set_note_text(id, &text);
        return;
    }
    state.editing_note = Some((id, text));
}
