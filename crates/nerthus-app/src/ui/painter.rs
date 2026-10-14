//! Draw's Painter (`Painter`, the node's Open Painter command): the
//! viewport's render as a square canvas to paint the node's strokes on.
//! Click to paint, hold Shift to erase (the height drops to 0 while it is
//! held), the wheel changes the brush's size, Ctrl + wheel its hardness
//! and Alt + wheel its height. Fast mode (on by default) shows the strokes
//! as a grey overlay while painting and builds once the stroke ends; 2D
//! mode renders the node as a mask. Each stroke is one edit of the node's
//! `StrokeData`. The window stays on top.

use std::sync::Arc;

use egui::{Color32, ColorImage, Id, Pos2, Rect, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui, Vec2};
use nerthus_nodes::vector_mask::{self, Brush};
use serde_json::Value;

use crate::editor::Editor;
use crate::theme;
use crate::ui::tool_window;

/// `Width="666"`, `Height="666"` (minimums too).
const WINDOW_SIZE: f32 = 666.0;
/// The toolbar under the canvas: 32-high slider boxes with margin 4.
const TOOLBAR_HEIGHT: f32 = 48.0;
/// Fast mode's overlay is painted at most this large.
const OVERLAY_MAX: usize = 1024;
/// `RenderIfTime`: outside fast mode the node builds while painting at
/// most this often.
const BUILD_INTERVAL: f64 = 0.1;
/// The instructions fade 0.5 s after opening, over 3 s.
const INSTRUCTIONS_SHOWN: f64 = 3.5;
const INSTRUCTIONS: &str = "Click to paint | Hold Shift to erase\nWheel change Size | Ctrl + Wheel change Hardness | Alt + Wheel change Height";

/// The brush settings (`sliSize`, `sliHardness`, `sliIntensity`, `btnLite`),
/// kept for the session.
#[derive(Clone, Copy, Debug)]
struct PainterSettings {
    size: f32,
    hardness: f32,
    intensity: f32,
    lite: bool,
}

impl Default for PainterSettings {
    fn default() -> Self {
        Self { size: 0.25, hardness: 0.5, intensity: 0.5, lite: true }
    }
}

/// The window while it is open.
#[derive(Clone)]
struct PainterState {
    node: i32,
    strokes: Vec<vector_mask::Stroke>,
    /// Fast mode's overlay (the strokes painted live) and its side.
    overlay: Vec<f32>,
    overlay_size: usize,
    overlay_texture: Option<TextureHandle>,
    overlay_dirty: bool,
    background: Option<(usize, TextureHandle)>,
    painting: bool,
    /// Shift held: the height the brush had (`oldIntensity`).
    erasing: Option<f32>,
    /// The instructions shown again (Show Instructions) at this time.
    instructions_from: f64,
    last_build: f64,
}

fn window_id() -> Id {
    Id::new("painter")
}

fn settings_id() -> Id {
    Id::new("painter-settings")
}

fn painting_id() -> Id {
    Id::new("painter-painting")
}

/// Whether a stroke is under way (`Image_MouseDown` until `Image_MouseUp`).
pub fn painting(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(painting_id())).unwrap_or(false)
}

/// `Editor_CommandExecuted("OpenPainter")`: the window for a Draw node.
pub fn open(ctx: &egui::Context, editor: &Editor, node: i32) {
    let stroke_data = editor.document.node(node).and_then(|data| data.values.get("StrokeData")).and_then(Value::as_str).unwrap_or_default().to_string();
    let strokes = if stroke_data.trim().is_empty() { Vec::new() } else { vector_mask::parse_stroke_data(&stroke_data).unwrap_or_default() };
    let overlay_size = editor.preview_resolution.min(OVERLAY_MAX);
    let now = ctx.input(|input| input.time);
    let mut state = PainterState {
        node,
        strokes,
        overlay: vec![0.0; overlay_size * overlay_size],
        overlay_size,
        overlay_texture: None,
        overlay_dirty: true,
        background: None,
        painting: false,
        erasing: None,
        instructions_from: now,
        last_build: 0.0,
    };
    let rendered = vector_mask::render(&state.strokes, overlay_size);
    state.overlay = rendered;
    ctx.data_mut(|data| data.insert_temp(window_id(), state));
}

/// Draws the window while it is open.
pub fn show(ctx: &egui::Context, editor: &mut Editor) {
    let Some(mut state) = ctx.data(|data| data.get_temp::<PainterState>(window_id())) else {
        return;
    };
    if editor.document.node(state.node).is_none_or(|node| node.type_name != "Draw") {
        ctx.data_mut(|data| data.remove::<PainterState>(window_id()));
        ctx.data_mut(|data| data.remove::<bool>(painting_id()));
        return;
    }
    let mut settings: PainterSettings = ctx.data(|data| data.get_temp(settings_id())).unwrap_or_default();
    let mut open = true;
    let mut window = egui::Window::new("Painter");
    window = window.open(&mut open);
    window = window.min_size(Vec2::new(WINDOW_SIZE, WINDOW_SIZE));
    window = window.collapsible(false);
    window = window.order(egui::Order::Foreground);
    window = window.frame(egui::Frame::window(&ctx.global_style()).fill(theme::MENU_BACKGROUND).inner_margin(0));
    window = tool_window::place(ctx, window, window_id().with("window"), Vec2::new(WINDOW_SIZE, WINDOW_SIZE + TOOLBAR_HEIGHT));
    window.show(ctx, |ui| {
        let available = ui.available_size();
        let side = available.x.min(available.y - TOOLBAR_HEIGHT).max(64.0);
        let (canvas_area, _) = ui.allocate_exact_size(Vec2::new(available.x, side), Sense::hover());
        let canvas = Rect::from_center_size(Pos2::new(canvas_area.center().x, canvas_area.top() + side / 2.0), Vec2::splat(side));
        canvas_view(ui, canvas, editor, &mut state, &mut settings);
        toolbar(ui, editor, &mut state, &mut settings);
    });
    ctx.data_mut(|data| data.insert_temp(settings_id(), settings));
    if !open {
        ctx.data_mut(|data| data.remove::<PainterState>(window_id()));
        ctx.data_mut(|data| data.remove::<bool>(painting_id()));
        return;
    }
    ctx.data_mut(|data| data.insert_temp(painting_id(), state.painting));
    ctx.data_mut(|data| data.insert_temp(window_id(), state));
}

/// The canvas: the render, fast mode's overlay while painting, the
/// instructions and the brush cursor; the painting itself.
fn canvas_view(ui: &mut Ui, canvas: Rect, editor: &mut Editor, state: &mut PainterState, settings: &mut PainterSettings) {
    let now = ui.input(|input| input.time);
    // `Redraw`: the viewport's render, not refreshed in
    // fast mode while a stroke is under way.
    let refresh = !(settings.lite && state.painting);
    if refresh && let Some(colours) = &editor.render_colours {
        let key = Arc::as_ptr(colours) as usize;
        if state.background.as_ref().map(|(shown, _)| *shown) != Some(key) {
            let image = ColorImage::from_rgb([colours.width, colours.height], &colours.rgb);
            let texture = ui.ctx().load_texture("painter-render", image, TextureOptions::LINEAR);
            state.background = Some((key, texture));
        }
    }
    let painter = ui.painter_at(canvas);
    painter.rect_filled(canvas, 0.0, Color32::BLACK);
    if let Some((_, texture)) = &state.background {
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        painter.image(texture.id(), canvas, uv, Color32::WHITE);
    }
    if settings.lite && state.painting {
        if state.overlay_dirty || state.overlay_texture.is_none() {
            let height = editor.document.node(state.node).and_then(|node| node.values.get("Height")).and_then(Value::as_f64).unwrap_or(1.0) as f32;
            let grey: Vec<u8> = state.overlay.iter().map(|value| (value * height.min(1.0) * 255.0).clamp(0.0, 255.0) as u8).collect();
            let image = ColorImage::from_gray([state.overlay_size, state.overlay_size], &grey);
            let texture = ui.ctx().load_texture("painter-overlay", image, TextureOptions::LINEAR);
            state.overlay_texture = Some(texture);
            state.overlay_dirty = false;
        }
        if let Some(texture) = &state.overlay_texture {
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            painter.image(texture.id(), canvas, uv, Color32::WHITE.gamma_multiply(0.4));
        }
    }

    // Shift erases (`Painter_KeyDown`/`KeyUp`).
    let shift = ui.input(|input| input.modifiers.shift);
    if shift && state.erasing.is_none() {
        state.erasing = Some(settings.intensity);
        settings.intensity = 0.0;
    } else if !shift && let Some(old) = state.erasing.take() {
        settings.intensity = old;
    }

    let response = ui.interact(canvas, window_id().with("canvas"), Sense::click_and_drag());
    let hovered = response.hovered() || response.dragged();
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::None);
        wheel(ui, settings);
    }
    let pointer = ui.input(|input| input.pointer.interact_pos().or(input.pointer.hover_pos()));
    let to_map = |position: Pos2| [((position.x - canvas.left()) / canvas.width()).clamp(0.0, 1.0), ((position.y - canvas.top()) / canvas.height()).clamp(0.0, 1.0)];
    let primary_down = ui.input(|input| input.pointer.primary_down());
    if response.drag_started_by(egui::PointerButton::Primary) || (response.is_pointer_button_down_on() && !state.painting && primary_down) {
        if let Some(position) = pointer {
            begin_stroke(editor, state, settings, to_map(position));
        }
    } else if state.painting && primary_down {
        if let Some(position) = pointer {
            add_point(state, to_map(position));
            let builds_now = !settings.lite && now - state.last_build > BUILD_INTERVAL;
            if builds_now {
                write_strokes(editor, state);
                state.last_build = now;
            }
        }
    } else if state.painting {
        // `Image_MouseUp`: the stroke ends and the node builds.
        state.painting = false;
        write_strokes(editor, state);
    }

    // The instructions (`instructions`), at the canvas's bottom right.
    let shown_for = now - state.instructions_from;
    if shown_for < INSTRUCTIONS_SHOWN {
        let fade = if shown_for < 0.5 { 1.0 } else { (1.0 - (shown_for - 0.5) / 3.0).max(0.0) as f32 };
        let galley = painter.layout(INSTRUCTIONS.to_string(), theme::monospace(11.0), theme::LABEL_EMPHASIS, canvas.width() - 60.0);
        let box_size = galley.size() + Vec2::splat(16.0);
        let box_rect = Rect::from_min_size(canvas.max - Vec2::splat(18.0) - box_size, box_size);
        painter.rect_filled(box_rect, 4.0, theme::WINDOW_DARK.gamma_multiply(0.7 * fade));
        painter.galley(box_rect.min + Vec2::splat(8.0), galley, theme::LABEL_EMPHASIS.gamma_multiply(fade));
        ui.ctx().request_repaint();
    }

    // The cursor: a blurred red ring the brush's size (blue-violet while
    // erasing), filled while painting.
    if hovered && let Some(position) = pointer {
        let radius = canvas.width() * settings.size * 0.2 / 2.0;
        let colour = if state.erasing.is_some() { Color32::from_rgb(0x8a, 0x2b, 0xe2) } else { Color32::from_rgb(255, 0, 0) };
        if state.painting {
            painter.circle_filled(position, radius, colour.gamma_multiply(0.5));
        }
        painter.circle_stroke(position, radius, Stroke::new(4.0, colour.gamma_multiply(0.25)));
        painter.circle_stroke(position, radius, Stroke::new(1.5, colour));
    }
}

/// `Image_MouseWheel`: size, or with Ctrl the hardness, or with Alt the
/// height, by 0.05 a notch.
fn wheel(ui: &Ui, settings: &mut PainterSettings) {
    let scroll = crate::ui::viewport_2d::wheel_direction(ui);
    let modifiers = ui.input(|input| input.modifiers);
    if scroll == 0.0 {
        return;
    }
    let step = if scroll > 0.0 { 0.05 } else { -0.05 };
    if modifiers.alt {
        settings.intensity = (settings.intensity + step).clamp(0.0, 1.0);
    } else if modifiers.ctrl || modifiers.command {
        settings.hardness = (settings.hardness + step).clamp(0.0, 1.0);
    } else {
        settings.size = (settings.size + step).clamp(0.0001, 1.0);
    }
}

/// `Image_MouseDown` → `Draw.AddStroke`: a stroke with the brush as the
/// sliders have it (`Draw.UpdateBrush`: the radius a tenth of the size).
fn begin_stroke(editor: &mut Editor, state: &mut PainterState, settings: &PainterSettings, point: [f32; 2]) {
    // The project as it was before the stroke, for undo.
    editor.document.checkpoint("Paint stroke");
    let brush = Brush { intensity: settings.intensity, radius: settings.size * 0.1, hardness: settings.hardness };
    let stroke = vector_mask::Stroke { points: Vec::new(), brushes: vec![brush] };
    state.strokes.push(stroke);
    state.painting = true;
    add_point(state, point);
}

/// `Draw.AddPoint`: the point added and its segment painted on the
/// overlay.
fn add_point(state: &mut PainterState, point: [f32; 2]) {
    let Some(stroke) = state.strokes.last_mut() else {
        return;
    };
    if stroke.points.last() == Some(&point) {
        return;
    }
    stroke.points.push(point);
    let end_index = stroke.points.len() - 1;
    vector_mask::paint_segment(stroke, end_index, &mut state.overlay, state.overlay_size);
    state.overlay_dirty = true;
}

/// The strokes into the node's `StrokeData` (`Draw.OnSaving`), as part of
/// the stroke's edit, and the node built (`ForceBuild`).
fn write_strokes(editor: &mut Editor, state: &PainterState) {
    let text = if state.strokes.is_empty() { Value::Null } else { Value::from(vector_mask::stroke_data(&state.strokes)) };
    let values = vec![("StrokeData".to_string(), text)];
    editor.document.set_values_quietly(state.node, &values);
    editor.changed();
}

/// The toolbar: Clear Canvas, Undo Last Stroke, fast mode, 2D mode, the
/// Size, Hardness and Height sliders, and Show Instructions on the right.
fn toolbar(ui: &mut Ui, editor: &mut Editor, state: &mut PainterState, settings: &mut PainterSettings) {
    let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), TOOLBAR_HEIGHT), Sense::hover());
    let inner = bar.shrink(4.0);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.spacing_mut().item_spacing.x = 4.0;
    let button_size = Vec2::splat(32.0);
    if crate::ui::widgets::icon_button_sized(&mut child, "x", "Clear Canvas", None, button_size, 16.0).clicked() {
        // `Draw.Clear`.
        editor.document.checkpoint("Clear Canvas");
        state.strokes.clear();
        state.overlay.fill(0.0);
        state.overlay_dirty = true;
        write_strokes(editor, state);
    }
    if crate::ui::widgets::icon_button_sized(&mut child, "arrow-back-up", "Undo Last Stroke", None, button_size, 16.0).clicked() && !state.strokes.is_empty() {
        // `Draw.Undo`.
        editor.document.checkpoint("Undo Last Stroke");
        state.strokes.pop();
        state.overlay = vector_mask::render(&state.strokes, state.overlay_size);
        state.overlay_dirty = true;
        write_strokes(editor, state);
    }
    if crate::ui::widgets::icon_toggle_sized(&mut child, "bolt", "Toggle Fast Mode (Recommended)", settings.lite, theme::ACCENT_MUTED_PRIMARY, button_size).clicked() {
        settings.lite = !settings.lite;
    }
    let mask_mode = editor.document.node(state.node).and_then(|node| node.base.get("RenderIntentOverride")).and_then(Value::as_str) == Some("Mask");
    if crate::ui::widgets::icon_toggle_sized(&mut child, "scribble", "Toggle 2D Mode", mask_mode, theme::ACCENT_PRIMARY, button_size).clicked() {
        // `BtnMask_Checked`/`Unchecked`: the node renders as a mask or a
        // heightfield.
        let intent = if mask_mode { "Heightfield" } else { "Mask" };
        editor.document.set_member(state.node, "RenderIntentOverride", Value::from(intent), "Render intent", true);
        editor.changed();
    }
    let info_width = 32.0 + 4.0;
    let sliders_width = (child.available_width() - info_width).max(90.0);
    let slider_width = sliders_width / 3.0 - 8.0;
    slider_box(&mut child, "Size", &mut settings.size, (0.0001, 1.0), theme::ACCENT_MUTED_PRIMARY, theme::ACCENT_MUTED_PRIMARY, slider_width);
    slider_box(&mut child, "Hardness", &mut settings.hardness, (0.0, 1.0), theme::ACCENT_MUTED_GRAY, theme::ACCENT_MUTED_GRAY, slider_width);
    let mut intensity = state.erasing.map(|_| 0.0).unwrap_or(settings.intensity);
    if slider_box(&mut child, "Height", &mut intensity, (0.0, 1.0), theme::ACCENT_MUTED_GRAY, theme::ACCENT_RED, slider_width) && state.erasing.is_none() {
        settings.intensity = intensity;
    }
    if crate::ui::widgets::icon_button_sized(&mut child, "info-square-rounded", "Show Instructions", None, button_size, 16.0).clicked() {
        state.instructions_from = child.input(|input| input.time) - 0.5;
    }
}

/// A slider in a rounded 32-high box (`Tag="modifier"`), its name on the
/// right; true when it moved.
fn slider_box(ui: &mut Ui, label: &str, value: &mut f32, range: (f32, f32), border: Color32, fill: Color32, width: f32) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width.max(60.0), 32.0), Sense::click_and_drag());
    let painter = ui.painter();
    let fraction = ((*value - range.0) / (range.1 - range.0)).clamp(0.0, 1.0);
    let filled = Rect::from_min_max(rect.min, Pos2::new(rect.left() + fraction * rect.width(), rect.bottom()));
    painter.rect_filled(filled.shrink(1.5), 3.0, fill.gamma_multiply(0.35));
    painter.rect_stroke(rect, 4.0, Stroke::new(1.5, border), StrokeKind::Inside);
    let galley = painter.layout_no_wrap(label.to_string(), theme::interface(12.0), theme::LABEL_DEFAULT);
    painter.galley(Pos2::new(rect.right() - 8.0 - galley.size().x, rect.center().y - galley.size().y / 2.0), galley, theme::LABEL_DEFAULT);
    let mut moved = false;
    if let Some(pointer) = response.interact_pointer_pos()
        && (response.dragged() || response.is_pointer_button_down_on())
    {
        let picked = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let new_value = range.0 + picked * (range.1 - range.0);
        if (new_value - *value).abs() > f32::EPSILON {
            *value = new_value;
            moved = true;
        }
    }
    moved
}

