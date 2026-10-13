//! `GradientEditor` (CLUTer's `GradientData`, Synth's `Data`): Actipro's
//! `GradientBrushSlider` (the gradient with a thumb per stop: click the
//! bar to add a stop, drag a thumb to move it, drag it off to remove it)
//! above the `ColorEditor` for the selected stop. Stops are `Float4`s of
//! scRGB red, green, blue and the offset; edits are written half a second
//! after the last change (`debounceInterval`), as one list.

use egui::{Color32, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use nerthus_engine::json::f32_value;
use serde_json::{json, Value};

use crate::theme;
use crate::ui::colour_picker::{colour_picker, linear_to_srgb, srgb_to_linear};
use crate::ui::editors::Edit;

/// `debounceInterval`.
const DEBOUNCE_SECONDS: f64 = 0.5;
/// The slider (`Height="64"`): the bar, then the thumbs under it.
const SLIDER_HEIGHT: f32 = 64.0;
const BAR_HEIGHT: f32 = 36.0;
const THUMB_SIZE: Vec2 = Vec2::new(12.0, 16.0);
/// How far below the slider a thumb is dragged to remove its stop.
const REMOVE_DISTANCE: f32 = 32.0;

/// A stop: scRGB red, green, blue and the offset.
type Stop = [f32; 4];

/// What the editor keeps between frames.
#[derive(Clone, Default)]
struct GradientState {
    selected: usize,
    /// The stops as edited, until they are written.
    pending: Option<Vec<Stop>>,
    changed_at: f64,
    /// The thumb being dragged, and whether it is off the slider.
    dragging: Option<(usize, bool)>,
}

/// The gradient's stops from the saved list (`List<Float4>`); none for a
/// missing list, which shows black to white.
fn stops_of(value: &Value) -> Option<Vec<Stop>> {
    let items = value.as_array()?;
    let component = |item: &Value, key: &str| item.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let stops = items.iter().map(|item| [component(item, "X"), component(item, "Y"), component(item, "Z"), component(item, "W")]).collect();
    Some(stops)
}

fn stops_value(stops: &[Stop]) -> Value {
    let items: Vec<Value> = stops.iter().map(|[x, y, z, w]| json!({"X": f32_value(*x), "Y": f32_value(*y), "Z": f32_value(*z), "W": f32_value(*w)})).collect();
    Value::Array(items)
}

/// A stop's colour as the screen shows it (`Color.ScR` → `R`).
fn stop_colour(stop: &Stop) -> [u8; 3] {
    [linear_to_srgb(stop[0]), linear_to_srgb(stop[1]), linear_to_srgb(stop[2])]
}

/// The gradient's colour at `offset`, interpolated between the stops in
/// sRGB as WPF's `LinearGradientBrush` draws it.
fn colour_at(stops: &[Stop], offset: f32) -> [u8; 3] {
    let mut sorted: Vec<&Stop> = stops.iter().collect();
    sorted.sort_by(|left, right| left[3].total_cmp(&right[3]));
    let Some(first) = sorted.first() else {
        return [0, 0, 0];
    };
    if offset <= first[3] {
        return stop_colour(first);
    }
    for pair in sorted.windows(2) {
        let (left, right) = (pair[0], pair[1]);
        if offset <= right[3] {
            let span = (right[3] - left[3]).max(f32::EPSILON);
            let fraction = (offset - left[3]) / span;
            let from = stop_colour(left);
            let to = stop_colour(right);
            let mix = |index: usize| (from[index] as f32 + (to[index] as f32 - from[index] as f32) * fraction).round() as u8;
            return [mix(0), mix(1), mix(2)];
        }
    }
    sorted.last().map(|last| stop_colour(last)).unwrap_or([0, 0, 0])
}

/// The editor (margin 8); `Edit::Set` with the whole list once the edits
/// settle.
pub fn gradient_editor(ui: &mut Ui, id: Id, value: &Value) -> Edit {
    let mut state: GradientState = ui.data(|data| data.get_temp(id)).unwrap_or_default();
    let saved = stops_of(value);
    let black_to_white = vec![[0.0, 0.0, 0.0, 0.0], [1.0, 1.0, 1.0, 1.0]];
    let mut stops = state.pending.clone().or(saved.clone()).unwrap_or(black_to_white);
    state.selected = state.selected.min(stops.len().saturating_sub(1));
    let now = ui.input(|input| input.time);
    let mut edited = false;

    let mut frame = egui::Frame::new();
    frame.inner_margin = egui::Margin::same(8);
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        let width = ui.available_width();
        let (slider, _) = ui.allocate_exact_size(Vec2::new(width, SLIDER_HEIGHT), Sense::hover());
        let bar = Rect::from_min_max(Pos2::new(slider.left() + 4.0 + THUMB_SIZE.x / 2.0, slider.top() + 4.0), Pos2::new(slider.right() - 4.0 - THUMB_SIZE.x / 2.0, slider.top() + 4.0 + BAR_HEIGHT));
        paint_bar(ui, bar, &stops);

        // Thumbs: pressed to select and drag, the bar clicked to add.
        let offset_at = |x: f32| ((x - bar.left()) / bar.width()).clamp(0.0, 1.0);
        let thumb_rect = |stop: &Stop| Rect::from_min_size(Pos2::new(bar.left() + stop[3].clamp(0.0, 1.0) * bar.width() - THUMB_SIZE.x / 2.0, bar.bottom() + 6.0), THUMB_SIZE);
        let mut thumb_pressed = None;
        for (index, stop) in stops.iter().enumerate() {
            let rect = thumb_rect(stop);
            let response = ui.interact(rect, id.with(("thumb", index)), Sense::click_and_drag());
            if response.is_pointer_button_down_on() && state.dragging.is_none() {
                thumb_pressed = Some(index);
            }
        }
        if let Some(index) = thumb_pressed {
            state.selected = index;
            state.dragging = Some((index, false));
        }
        let pointer = ui.input(|input| input.pointer.interact_pos());
        let pointer_down = ui.input(|input| input.pointer.primary_down());
        if let (Some((index, _)), Some(pointer)) = (state.dragging, pointer)
            && index < stops.len()
        {
            if pointer_down {
                let off_slider = pointer.y > slider.bottom() + REMOVE_DISTANCE || pointer.y < slider.top() - REMOVE_DISTANCE;
                let new_offset = offset_at(pointer.x);
                if (stops[index][3] - new_offset).abs() > f32::EPSILON && !off_slider {
                    stops[index][3] = new_offset;
                    edited = true;
                }
                state.dragging = Some((index, off_slider && stops.len() > 2));
            } else {
                // Released: a thumb dragged off the slider takes its stop.
                if state.dragging.is_some_and(|(_, off)| off) {
                    stops.remove(index);
                    state.selected = state.selected.min(stops.len() - 1);
                    edited = true;
                }
                state.dragging = None;
            }
        } else if !pointer_down {
            state.dragging = None;
        }
        let bar_response = ui.interact(bar, id.with("bar"), Sense::click());
        if bar_response.clicked()
            && let Some(position) = bar_response.interact_pointer_pos()
        {
            let offset = offset_at(position.x);
            let colour = colour_at(&stops, offset);
            let stop = [srgb_to_linear(colour[0]), srgb_to_linear(colour[1]), srgb_to_linear(colour[2]), offset];
            stops.push(stop);
            state.selected = stops.len() - 1;
            edited = true;
        }
        for (index, stop) in stops.iter().enumerate() {
            let removing = state.dragging == Some((index, true));
            paint_thumb(ui, thumb_rect(stop), stop_colour(stop), index == state.selected, removing);
        }

        // `ColorEditor` for the selected stop (margin 0, 8).
        ui.add_space(8.0);
        if let Some(stop) = stops.get(state.selected).copied()
            && let Some(colour) = colour_picker(ui, id.with("colour"), stop_colour(&stop))
        {
            let selected = &mut stops[state.selected];
            selected[0] = srgb_to_linear(colour[0]);
            selected[1] = srgb_to_linear(colour[1]);
            selected[2] = srgb_to_linear(colour[2]);
            edited = true;
        }
        ui.add_space(8.0);
    });

    if edited {
        state.pending = Some(stops.clone());
        state.changed_at = now;
    }
    let mut edit = Edit::None;
    if let Some(pending) = state.pending.clone() {
        let waited = now - state.changed_at;
        if waited >= DEBOUNCE_SECONDS && state.dragging.is_none() {
            state.pending = None;
            if saved.as_ref() != Some(&pending) {
                edit = Edit::Set(stops_value(&pending));
            }
        } else {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64((DEBOUNCE_SECONDS - waited).max(0.05)));
        }
    }
    ui.data_mut(|data| data.insert_temp(id, state));
    edit
}

/// The gradient, sampled every couple of points along a strip of
/// vertex-coloured quads.
fn paint_bar(ui: &Ui, bar: Rect, stops: &[Stop]) {
    let columns = (bar.width() / 2.0).max(1.0) as usize;
    let mut mesh = egui::Mesh::default();
    for column in 0..=columns {
        let offset = column as f32 / columns as f32;
        let [red, green, blue] = colour_at(stops, offset);
        let colour = Color32::from_rgb(red, green, blue);
        let x = bar.left() + offset * bar.width();
        mesh.colored_vertex(Pos2::new(x, bar.top()), colour);
        mesh.colored_vertex(Pos2::new(x, bar.bottom()), colour);
    }
    for column in 0..columns {
        let top = (column * 2) as u32;
        mesh.add_triangle(top, top + 1, top + 2);
        mesh.add_triangle(top + 1, top + 3, top + 2);
    }
    ui.painter().add(egui::Shape::mesh(mesh));
    ui.painter().rect_stroke(bar, 2.0, Stroke::new(1.0, theme::BUTTON_BORDER), StrokeKind::Outside);
}

/// A stop's thumb: a pointer up to the bar, filled with its colour; the
/// selected one outlined in the accent, one about to go faded.
fn paint_thumb(ui: &Ui, rect: Rect, colour: [u8; 3], selected: bool, removing: bool) {
    let painter = ui.painter();
    let fill = Color32::from_rgb(colour[0], colour[1], colour[2]);
    let fill = if removing { fill.gamma_multiply(0.3) } else { fill };
    let outline = if selected { theme::ACCENT_PRIMARY } else { theme::LABEL_DEFAULT };
    let tip = Pos2::new(rect.center().x, rect.top() - 5.0);
    let pointer = vec![tip, Pos2::new(rect.left(), rect.top()), Pos2::new(rect.right(), rect.top())];
    painter.add(egui::Shape::convex_polygon(pointer, outline, Stroke::NONE));
    painter.rect_filled(rect, 2.0, fill);
    painter.rect_stroke(rect, 2.0, Stroke::new(if selected { 2.0 } else { 1.0 }, outline), StrokeKind::Inside);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_colours_interpolate_in_srgb() {
        let stops = vec![[0.0, 0.0, 0.0, 0.0], [1.0, 1.0, 1.0, 1.0]];
        assert_eq!(colour_at(&stops, 0.5), [128, 128, 128]);
        assert_eq!(colour_at(&stops, -1.0), [0, 0, 0]);
        let value = stops_value(&stops);
        assert_eq!(stops_of(&value), Some(stops));
    }
}
