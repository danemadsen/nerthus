//! Graph notes (`NoteElement`): a box tinted with the note's swatch (10%
//! fill, 1px border) showing its Markdown, or just a note icon for
//! `Icon` notes (their text as a tooltip). Notes move by dragging, resize
//! by the corner thumb, flip between icon and text on double-click, and
//! offer Edit Note, their colour and Delete Note on right-click.

use std::collections::HashSet;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2};
use serde_json::Value;

use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::graph_canvas::{GraphState, GraphView};
use crate::ui::graph_commands::GraphCommand;
use crate::ui::graph_popups::swatch_grid;
use crate::ui::widgets::{menu_caption, menu_item, menu_separator, menu_style};

/// `NoteElement`'s least height, and the least width of a text note
/// (`OnlyText`: `MinWidth` 100).
const MIN_HEIGHT: f32 = 60.0;
const MIN_TEXT_WIDTH: f32 = 100.0;
const THUMB: f32 = 20.0;
/// An `Icon` note: `SetNoteType` makes it 60 by 35, which its least
/// height of 60 overrules.
const ICON_BOX: f32 = 60.0;

/// A note in progress of being moved or resized.
#[derive(Clone, Copy, Debug)]
pub struct NoteDrag {
    id: i32,
    resizing: bool,
    pointer_start: Pos2,
    start: [f32; 2],
}

struct NoteLayout {
    id: i32,
    rect: Rect,
    color: Color32,
    markdown: String,
    icon_only: bool,
}

fn vector(fields: &serde_json::Map<String, Value>, name: &str, fallback: [f32; 2]) -> [f32; 2] {
    let Some(value) = fields.get(name) else {
        return fallback;
    };
    let component = |key: &str, default: f32| value.get(key).and_then(Value::as_f64).map_or(default, |number| number as f32);
    [component("X", fallback[0]), component("Y", fallback[1])]
}

fn layouts(editor: &Editor, tab: usize) -> Vec<NoteLayout> {
    let mut notes = Vec::new();
    for (id, fields) in editor.document.notes() {
        let graph_index = fields.get("GraphIndex").and_then(Value::as_u64).unwrap_or(0) as usize;
        if graph_index != tab {
            continue;
        }
        let position = vector(fields, "Position", [0.0, 0.0]);
        let size = vector(fields, "Size", [200.0, 200.0]);
        let color_name = fields.get("Color").and_then(Value::as_str).unwrap_or("Gray");
        let markdown = fields.get("Markdown").and_then(Value::as_str).unwrap_or_default().to_string();
        // `Type` missing from the file is `Icon`, `NoteType`'s first member.
        let icon_only = fields.get("Type").and_then(Value::as_str) != Some("OnlyText");
        let origin = Pos2::new(position[0], position[1]);
        let rect = if icon_only { Rect::from_min_size(origin, Vec2::splat(ICON_BOX)) } else { Rect::from_min_size(origin, Vec2::new(size[0], size[1])) };
        let note = NoteLayout {
            id,
            rect,
            color: theme::swatch_color(color_name),
            markdown,
            icon_only,
        };
        notes.push(note);
    }
    notes
}

/// A note's rectangle on the canvas (`CenterNote`), when it is on `tab`.
pub fn note_rect(editor: &Editor, tab: usize, id: i32) -> Option<Rect> {
    let notes = layouts(editor, tab);
    notes.iter().find(|note| note.id == id).map(|note| note.rect)
}

/// The note's Markdown as wrapped text: `#` headings in semibold, list
/// items with bullets, emphasis markers dropped.
pub(crate) fn markdown_job(markdown: &str, zoom: f32, width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    let body = TextFormat::simple(theme::interface(12.0 * zoom), theme::LABEL_DEFAULT);
    let heading = TextFormat::simple(theme::semibold(14.0 * zoom), theme::LABEL_EMPHASIS);
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        let (text, format) = if let Some(rest) = trimmed.strip_prefix('#') {
            (rest.trim_start_matches('#').trim().to_string(), heading.clone())
        } else if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            (format!("• {rest}"), body.clone())
        } else {
            (trimmed.to_string(), body.clone())
        };
        let plain = text.replace("**", "").replace("__", "").replace('`', "");
        job.append(&plain, 0.0, format);
        job.append("\n", 0.0, body.clone());
    }
    job
}

fn screen_rect(view: &GraphView, origin: Pos2, rect: Rect) -> Rect {
    Rect::from_min_max(view.to_screen(origin, rect.min), view.to_screen(origin, rect.max))
}

/// Draws this tab's notes (those collapsed groups hide aside) and handles
/// their mouse input.
pub fn show(ui: &mut Ui, painter: &egui::Painter, editor: &mut Editor, state: &mut GraphState, origin: Pos2, hidden: &HashSet<i32>, commands: &mut Vec<Command>) {
    let tab = state.tab;
    let view = state.view();
    let zoom = view.zoom;
    let notes: Vec<NoteLayout> = layouts(editor, tab).into_iter().filter(|note| !hidden.contains(&note.id)).collect();
    let mut responses: Vec<(i32, Response, Option<Response>)> = Vec::new();
    for note in &notes {
        let screen = screen_rect(&view, origin, note.rect);
        if !screen.intersects(state.rect) {
            continue;
        }
        draw(ui, painter, note, screen, zoom, state.selected_note == Some(note.id));
        let response = ui.interact(screen, ui.id().with(("note", note.id)), Sense::click_and_drag());
        let response = if note.icon_only && !note.markdown.is_empty() { response.on_hover_text(note.markdown.clone()) } else { response };
        let thumb = if note.icon_only {
            None
        } else {
            let thumb_rect = Rect::from_min_size(screen.max - Vec2::splat(THUMB * zoom), Vec2::splat(THUMB * zoom));
            let thumb_response = ui.interact(thumb_rect, ui.id().with(("note-thumb", note.id)), Sense::drag());
            if thumb_response.hovered() || thumb_response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
            }
            Some(thumb_response)
        };
        responses.push((note.id, response, thumb));
    }

    let modifiers = ui.input(|input| input.modifiers);
    for (id, response, thumb) in &responses {
        if response.clicked() {
            if !(modifiers.command || modifiers.shift) {
                state.clear_element_selection();
                editor.select(None);
            }
            state.selected_note = Some(*id);
        }
        // `NoteElement_MouseDoubleClick`: icon and text swap.
        if response.double_clicked() {
            commands.push(Command::Graph(GraphCommand::ToggleNoteType(*id)));
        }
        response.context_menu(|ui| note_menu(ui, *id, commands));
        let pointer = response.interact_pointer_pos();
        if let (Some(thumb), Some(note)) = (thumb, notes.iter().find(|note| note.id == *id))
            && thumb.drag_started()
        {
            let start = [note.rect.width(), note.rect.height()];
            let pointer_start = thumb.interact_pointer_pos().map(|point| view.to_canvas(origin, point)).unwrap_or_default();
            editor.document.set_note_vector(*id, "Size", start, true);
            state.note_drag = Some(NoteDrag { id: *id, resizing: true, pointer_start, start });
        }
        if response.drag_started_by(egui::PointerButton::Primary)
            && state.note_drag.is_none()
            && let (Some(point), Some(note)) = (pointer, notes.iter().find(|note| note.id == *id))
        {
            let start = [note.rect.left(), note.rect.top()];
            editor.document.set_note_vector(*id, "Position", start, true);
            state.selected_note = Some(*id);
            state.note_drag = Some(NoteDrag { id: *id, resizing: false, pointer_start: view.to_canvas(origin, point), start });
        }
    }

    let Some(drag) = state.note_drag else {
        return;
    };
    let held = ui.input(|input| input.pointer.primary_down());
    let mut corner = drag.start;
    if let Some(point) = ui.input(|input| input.pointer.interact_pos()) {
        let delta = view.to_canvas(origin, point) - drag.pointer_start;
        if drag.resizing {
            let size = [(drag.start[0] + delta.x).max(MIN_TEXT_WIDTH), (drag.start[1] + delta.y).max(MIN_HEIGHT)];
            editor.document.set_note_vector(drag.id, "Size", size, false);
        } else {
            corner = [drag.start[0] + delta.x, drag.start[1] + delta.y];
            editor.document.set_note_vector(drag.id, "Position", corner, false);
        }
    }
    if held {
        return;
    }
    // `NoteElement_MoveCompleted`: in or out of a group by its corner.
    if !drag.resizing {
        editor.document.settle_in_groups(&[(drag.id, corner, tab)]);
    }
    state.note_drag = None;
}

/// `ctxNote`: Edit Note, the colours, Delete Note.
fn note_menu(ui: &mut Ui, id: i32, commands: &mut Vec<Command>) {
    menu_style(ui);
    if menu_item(ui, None, None, "Edit Note", "").clicked() {
        commands.push(Command::Graph(GraphCommand::EditNote(id)));
    }
    menu_caption(ui, "Color");
    if let Some(color) = swatch_grid(ui) {
        commands.push(Command::Graph(GraphCommand::SetNoteColor(id, color)));
        ui.close();
    }
    if menu_item(ui, None, None, "Delete Note", "").clicked() {
        commands.push(Command::Graph(GraphCommand::DeleteNote(id)));
    }
    menu_separator(ui);
}

fn draw(ui: &Ui, painter: &egui::Painter, note: &NoteLayout, screen: Rect, zoom: f32, selected: bool) {
    painter.rect_filled(screen, 0.0, note.color.gamma_multiply(0.1));
    painter.rect_stroke(screen, 0.0, Stroke::new(1.0, note.color), StrokeKind::Inside);
    if selected {
        painter.rect_stroke(screen.expand(1.0), 0.0, Stroke::new(1.0, theme::ACCENT_MUTED_PRIMARY), StrokeKind::Outside);
    }
    if note.icon_only {
        let icon_rect = Rect::from_center_size(screen.center(), Vec2::splat(24.0 * zoom));
        icons::paint(ui, icon_rect, "note", note.color);
        return;
    }
    let inner = screen.shrink(8.0 * zoom);
    if inner.width() > 4.0 && zoom > 0.15 {
        let job = markdown_job(&note.markdown, zoom, inner.width());
        let galley = painter.layout_job(job);
        let clipped = painter.with_clip_rect(inner.intersect(painter.clip_rect()));
        clipped.galley(inner.min, galley, theme::LABEL_DEFAULT);
    }
    // `NoteThumb`: a 12px triangle in the corner.
    let corner = screen.max;
    let size = 12.0 * zoom;
    let triangle = vec![Pos2::new(corner.x, corner.y - size), corner, Pos2::new(corner.x - size, corner.y)];
    painter.add(Shape::convex_polygon(triangle, note.color, Stroke::NONE));
}
