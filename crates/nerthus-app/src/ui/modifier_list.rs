//! The node's modifiers (`EditorBase.CreateModifiers`, `ModifierListBox`),
//! in `Order`: each in a cladding box (`ModifierBorder`, padding 16, 4),
//! its name when it has no UI, otherwise its editors (`Tag="modifier"`:
//! no inset, 20 high after the first; with three editors the second and
//! third share a row). Colours restart at the first of the palette the
//! parameters left, which moves on after each modifier with editors
//! (`ParamsY` for 0, else `ParamsX`).
//!
//! Under the pointer a box brightens its grip (`DragButtonAdorner`) and
//! shows its close button (`ModifierAdorners`), which removes the modifier
//! (asking first unless Shift is held). A box dragged up or down by
//! anything but its editors, 28 points or more, drops onto another box's
//! place; Min and Max cannot be dragged, and nothing drags while Shift is
//! held.

use egui::{Color32, CursorIcon, Id, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2};
use nerthus_engine::editing::{has_ui, modifier_name, modifier_order};
use nerthus_engine::{Catalogue, ModifierData, NodeData, ParameterSpec};
use serde_json::Value;

use crate::document::PINNED_ORDER;
use crate::icons;
use crate::theme;
use crate::ui::editors::{self, palette_color, Edit, EditorContext};

/// GongSolutions' `MinimumVerticalDragDistance` as the list sets it (the
/// horizontal one is infinite).
const DRAG_DISTANCE: f32 = 28.0;
/// `DragButtonAdorner`: a 14px grip 2 in, at a quarter until hovered.
const GRIP_SIZE: f32 = 14.0;
const GRIP_RESTING_OPACITY: f32 = 0.25;
/// `ModifierAdorners`: a 16px close button in the top right corner.
const CLOSE_SIZE: f32 = 16.0;
/// `FadeInAndShow` and `FadeOut`'s default lengths.
const FADE_IN_SECONDS: f32 = 0.15;
const FADE_OUT_SECONDS: f32 = 0.5;
/// GongSolutions' insertion line: 2px gray with 5px triangles at its ends.
const INSERTION_COLOR: Color32 = Color32::from_rgb(0x80, 0x80, 0x80);
const INSERTION_TRIANGLE: f32 = 5.0;

/// What the list asks of the document.
#[derive(Default)]
pub struct ModifierEdits {
    /// Parameter edits: the modifier's index, the parameter, the edit.
    pub edits: Vec<(usize, String, Edit)>,
    /// A close button clicked: the modifier's place, and whether Shift
    /// was held (no question then).
    pub close: Option<(usize, bool)>,
    /// A box dropped: from its place onto another's.
    pub moved: Option<(usize, usize)>,
    /// Where each box was drawn, in order.
    pub boxes: Vec<Rect>,
}

/// A drag in progress, between frames.
#[derive(Clone, Default)]
struct ModifierDrag {
    /// The place pressed and where, until it moves far enough.
    press: Option<(usize, Pos2)>,
    /// The place being dragged.
    dragged: Option<usize>,
    /// Each box as laid out last frame; a box's own drag area lies under
    /// its editors, so it is placed before they are.
    boxes: Vec<Rect>,
}

/// Draws the list; `color_flip` is the palette the parameters left.
pub fn modifier_list(ui: &mut Ui, node: &NodeData, node_id: i32, color_flip: usize, area_width: f32) -> ModifierEdits {
    let mut result = ModifierEdits::default();
    let mut ordered: Vec<(usize, &ModifierData)> = node.modifiers.iter().enumerate().collect();
    ordered.sort_by_key(|(_, modifier)| modifier_order(modifier));
    if ordered.is_empty() {
        return result;
    }
    let list_id = ui.id().with(("modifier-list", node_id));
    let mut drag: ModifierDrag = ui.data(|data| data.get_temp(list_id)).unwrap_or_default();
    if drag.boxes.len() != ordered.len() {
        drag.boxes.clear();
    }
    let shift = ui.input(|input| input.modifiers.shift);
    let count = ordered.len();
    let mut made = 0usize;
    let mut color_flip = color_flip;
    let mut boxes = Vec::with_capacity(count);
    let mut frame_outer = egui::Frame::new();
    frame_outer.inner_margin = egui::Margin::same(4);
    frame_outer.show(ui, |ui| {
        for (place, (index, modifier)) in ordered.iter().copied().enumerate() {
            // The box's drag area, under everything drawn in it.
            let box_id = list_id.with(("box", place));
            if let Some(previous) = drag.boxes.get(place) {
                let area = ui.interact(*previous, box_id, Sense::drag());
                let pressed = area.is_pointer_button_down_on() && ui.input(|input| input.pointer.primary_pressed());
                if pressed && !shift && drag.dragged.is_none() {
                    let origin = ui.input(|input| input.pointer.press_origin());
                    drag.press = origin.map(|origin| (place, origin));
                }
            }
            let top = if place == 0 { 4 } else { 0 };
            let bottom = if place + 1 == count { 4 } else { 0 };
            let corners = egui::CornerRadius { nw: top, ne: top, sw: bottom, se: bottom };
            let mut frame = egui::Frame::new();
            frame.fill = theme::PANEL_CLADDING;
            frame.corner_radius = corners;
            frame.inner_margin = egui::Margin { left: 16, right: 16, top: 4, bottom: 4 };
            let shown = frame.show(ui, |ui| {
                ui.set_width(ui.available_width());
                modifier_editors(ui, node_id, index, modifier, color_flip, &mut made, area_width, &mut result.edits)
            });
            if shown.inner {
                color_flip = (color_flip + 1) % 3;
            }
            let rect = shown.response.rect;
            boxes.push(rect);
            let hovered = ui.rect_contains_pointer(rect) && drag.dragged.is_none();
            // `ModifierBorder`'s hover outline.
            if hovered {
                let outline = Stroke::new(1.5, Color32::from_rgba_unmultiplied(0xa2, 0xc5, 0xd2, 0x66));
                ui.painter().rect_stroke(rect, corners, outline, StrokeKind::Inside);
            }
            adorners(ui, box_id, rect, hovered, &mut result, place);
        }
    });
    result.moved = drag_and_drop(ui, &mut drag, &ordered, &boxes, shift);
    result.boxes = boxes.clone();
    drag.boxes = boxes;
    ui.data_mut(|data| data.insert_temp(list_id, drag));
    result
}

/// A modifier's editors, or its name when it has none; true when it had
/// editors (the palette moves on).
#[allow(clippy::too_many_arguments)]
fn modifier_editors(ui: &mut Ui, node_id: i32, index: usize, modifier: &ModifierData, color_flip: usize, made: &mut usize, area_width: f32, edits: &mut Vec<(usize, String, Edit)>) -> bool {
    let spec = Catalogue::get().modifier(&modifier.type_name);
    let parameters = spec.map(|spec| spec.parameters.as_slice()).unwrap_or_default();
    if !has_ui(modifier) || parameters.is_empty() {
        // The name in title case (margin 8, 2, 2, 2), painted so a press on
        // it can drag the box.
        let title = nerthus_engine::humanize::humanize_title(modifier_name(modifier));
        let galley = ui.painter().layout_no_wrap(title, theme::interface(11.0), theme::ICON_DEFAULT);
        let size = Vec2::new(ui.available_width(), galley.size().y + 4.0);
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        ui.painter().galley(rect.min + Vec2::new(8.0, 2.0), galley, theme::ICON_DEFAULT);
        return false;
    }
    let palette = if color_flip == 0 { 0 } else { 1 };
    // Three editors: the first on its own, then a row of two
    // (`UniformGrid`, one row of two columns).
    let paired = parameters.len() == 3;
    let mut cell = |ui: &mut Ui, position: usize, parameter: &ParameterSpec| {
        let color = palette_color(palette, *made % 8);
        *made += 1;
        let default = spec.and_then(|spec| spec.defaults.get(&parameter.name));
        let value = modifier.values.get(&parameter.name).cloned().or(default.cloned()).unwrap_or(Value::Null);
        let id = ui.id().with(("modifier", node_id, index, &parameter.name));
        let context = EditorContext { color, default, values: None, area_width, terrain_range: None };
        // Each editor has a 2px margin.
        let mut margin = egui::Frame::new();
        margin.inner_margin = egui::Margin::same(2);
        let edit = margin
            .show(ui, |ui| {
                let number = matches!(parameter.value_type, nerthus_engine::ValueType::Float | nerthus_engine::ValueType::Double | nerthus_engine::ValueType::Int);
                if number && !parameter.is_seed() {
                    modifier_slider(ui, id, parameter, &value, &context, position == 0)
                } else if matches!(parameter.value_type, nerthus_engine::ValueType::Float2 | nerthus_engine::ValueType::Int2) {
                    modifier_range(ui, id, parameter, &value, &context)
                } else {
                    editors::parameter(ui, id, parameter, &value, &context)
                }
            })
            .inner;
        edits.push((index, parameter.name.clone(), edit));
    };
    for (position, parameter) in parameters.iter().enumerate() {
        if paired && position == 1 {
            let rest = &parameters[1..];
            ui.columns(2, |columns| {
                for (column, parameter) in columns.iter_mut().zip(rest) {
                    column.spacing_mut().item_spacing = Vec2::ZERO;
                    cell(column, 1, parameter);
                }
            });
            break;
        }
        cell(ui, position, parameter);
    }
    true
}

/// A slider in a modifier (`Tag="modifier"`): no inset, no outline,
/// corners of 2; after the first, 20 high.
fn modifier_slider(ui: &mut Ui, id: Id, parameter: &ParameterSpec, value: &Value, context: &EditorContext, first: bool) -> Edit {
    let integer = parameter.value_type == nerthus_engine::ValueType::Int;
    let mut spec = editors::SliderSpec::of(parameter, context.default, integer);
    spec.inset = Some(0.0);
    spec.modifier = true;
    if !first {
        spec.height = Some(20.0);
    }
    let current = value.as_f64().unwrap_or(0.0);
    let label = parameter.display_name();
    match editors::slider(ui, id, &label, current, &spec, context.color) {
        Some(new_value) if integer => Edit::Set(serde_json::json!(new_value.round_ties_even() as i64)),
        Some(new_value) if parameter.value_type == nerthus_engine::ValueType::Float => Edit::Set(nerthus_engine::json::f32_value(new_value as f32)),
        Some(new_value) => Edit::Set(serde_json::json!(new_value)),
        None => Edit::None,
    }
}

/// A range in a modifier (`Tag="modifier"`): no outline, corners of 2,
/// padding 0, 0, 2, 0.
fn modifier_range(ui: &mut Ui, id: Id, parameter: &ParameterSpec, value: &Value, context: &EditorContext) -> Edit {
    let (min, max) = editors::range_of(parameter, 0.0, 1.0);
    let mut spec = editors::RangeSpec::linear(min, max);
    spec.integer = parameter.value_type == nerthus_engine::ValueType::Int2;
    spec.inset = Some(0.0);
    spec.modifier = true;
    spec.default = context.default.and_then(editors::range_pair);
    let (low, high) = editors::range_pair(value).unwrap_or((0.0, 0.0));
    let label = parameter.display_name();
    // The padding's 2 on the right.
    let room_size = Vec2::new(ui.available_width() - 2.0, f32::INFINITY);
    let room = Rect::from_min_size(ui.cursor().min, room_size);
    let mut range_ui = ui.new_child(egui::UiBuilder::new().max_rect(room));
    let changed = editors::range_slider(&mut range_ui, id, &label, low, high, &spec, context.color);
    ui.advance_cursor_after_rect(range_ui.min_rect());
    match changed {
        Some((low, high)) if spec.integer => Edit::Set(serde_json::json!({"X": low.round_ties_even() as i64, "Y": high.round_ties_even() as i64})),
        Some((low, high)) => Edit::Set(serde_json::json!({"X": nerthus_engine::json::f32_value(low as f32), "Y": nerthus_engine::json::f32_value(high as f32)})),
        None => Edit::None,
    }
}

/// The box's grip and close button, fading in while it is hovered.
fn adorners(ui: &mut Ui, box_id: Id, rect: Rect, hovered: bool, result: &mut ModifierEdits, place: usize) {
    let instant = theme::settings(ui.ctx()).disable_animations;
    let fade = |ui: &Ui, key: &str, target: f32| {
        let seconds = if instant {
            0.0
        } else if hovered {
            FADE_IN_SECONDS
        } else {
            FADE_OUT_SECONDS
        };
        ui.ctx().animate_value_with_time(box_id.with(key), target, seconds)
    };
    let grip_opacity = fade(ui, "grip", if hovered { 1.0 } else { GRIP_RESTING_OPACITY });
    let grip = Rect::from_min_size(Pos2::new(rect.left() + 2.0, rect.center().y - GRIP_SIZE / 2.0), Vec2::splat(GRIP_SIZE));
    icons::paint(ui, grip, "grip-vertical", theme::ICON_DEFAULT.gamma_multiply(grip_opacity));

    let close_opacity = fade(ui, "close", if hovered { 1.0 } else { 0.0 });
    if close_opacity <= 0.0 {
        return;
    }
    // `Close-Button-Adorner`: Marlett's cross in `Accent-Red` at 0.85,
    // `Label-Emphasis` under the pointer.
    let close = Rect::from_min_size(Pos2::new(rect.right() - CLOSE_SIZE, rect.top()), Vec2::splat(CLOSE_SIZE));
    let button = ui.interact(close, box_id.with("close"), Sense::click());
    let (color, opacity) = if button.hovered() { (theme::LABEL_EMPHASIS, 1.0) } else { (theme::ACCENT_RED, 0.85) };
    let stroke = Stroke::new(1.25, color.gamma_multiply(opacity * close_opacity));
    let arm = 2.5;
    let center = close.center();
    ui.painter().line_segment([center + Vec2::new(-arm, -arm), center + Vec2::new(arm, arm)], stroke);
    ui.painter().line_segment([center + Vec2::new(-arm, arm), center + Vec2::new(arm, -arm)], stroke);
    if button.clicked() {
        let shift = ui.input(|input| input.modifiers.shift);
        result.close = Some((place, shift));
    }
}

/// Starts, shows and ends a drag; the move once it drops on a box.
fn drag_and_drop(ui: &Ui, drag: &mut ModifierDrag, ordered: &[(usize, &ModifierData)], boxes: &[Rect], shift: bool) -> Option<(usize, usize)> {
    let (pointer, down, escaped) = ui.input(|input| (input.pointer.latest_pos(), input.pointer.primary_down(), input.key_pressed(egui::Key::Escape)));
    if let Some((place, origin)) = drag.press {
        let far_enough = pointer.is_some_and(|position| (position.y - origin.y).abs() > DRAG_DISTANCE);
        if !down || shift {
            drag.press = None;
        } else if far_enough {
            drag.press = None;
            drag.dragged = Some(place);
        }
    }
    let dragged = drag.dragged?;
    if escaped {
        drag.dragged = None;
        return None;
    }
    // Min and Max cannot be dragged anywhere.
    let pinned = ordered.get(dragged).is_some_and(|(_, modifier)| modifier_order(modifier) == PINNED_ORDER);
    let target = pointer.and_then(|position| boxes.iter().position(|rect| rect.contains(position)));
    if pinned {
        ui.ctx().set_cursor_icon(CursorIcon::NotAllowed);
    } else {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        if let Some(target) = target.filter(|target| *target != dragged) {
            insertion_line(ui, boxes[target], target > dragged);
        }
    }
    if down {
        ui.ctx().request_repaint();
        return None;
    }
    drag.dragged = None;
    let target = target?;
    (!pinned && target != dragged).then_some((dragged, target))
}

/// Where the dragged box will land: under the target when it moves down,
/// over it when it moves up.
fn insertion_line(ui: &Ui, target: Rect, below: bool) {
    let y = if below { target.bottom() } else { target.top() };
    let left = Pos2::new(target.left(), y);
    let right = Pos2::new(target.right(), y);
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("modifier-insertion")));
    painter.line_segment([left, right], Stroke::new(2.0, INSERTION_COLOR));
    let triangle = |tip: Pos2, inward: f32| {
        let base_x = tip.x - inward * INSERTION_TRIANGLE;
        let points = vec![tip, Pos2::new(base_x, y - INSERTION_TRIANGLE), Pos2::new(base_x, y + INSERTION_TRIANGLE)];
        Shape::convex_polygon(points, INSERTION_COLOR, Stroke::NONE)
    };
    painter.add(triangle(left + Vec2::new(INSERTION_TRIANGLE, 0.0), 1.0));
    painter.add(triangle(right - Vec2::new(INSERTION_TRIANGLE, 0.0), -1.0));
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerthus_engine::editing::new_modifier;

    /// A Perlin node with these modifiers, added in turn.
    fn node_with(modifiers: &[&str]) -> NodeData {
        let mut document = crate::document::Document::new();
        let id = document.add_node("Perlin", [0.0, 0.0]).unwrap();
        let mut node = document.node(id).unwrap().clone();
        for (order, name) in modifiers.iter().enumerate() {
            node.modifiers.push(new_modifier(name, order).unwrap());
        }
        node
    }

    fn themed_context() -> egui::Context {
        let context = egui::Context::default();
        theme::apply(&context);
        let output = context.run_ui(egui::RawInput::default(), |_| {});
        output.drop_without_applying_deltas();
        context
    }

    /// One frame of the list with these events.
    fn list_frame(context: &egui::Context, node: &NodeData, events: Vec<egui::Event>) -> ModifierEdits {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 600.0));
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut edits = ModifierEdits::default();
        let output = context.run_ui(input, |ui| {
            let mut list = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 600.0))));
            list.spacing_mut().item_spacing = Vec2::ZERO;
            edits = modifier_list(&mut list, node, node.id, 0, 300.0);
        });
        output.drop_without_applying_deltas();
        edits
    }

    fn button(pos: Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers }
    }

    #[test]
    fn the_close_button_shows_on_hover_and_asks_unless_shift_is_held() {
        let context = themed_context();
        let node = node_with(&["Blur", "Invert"]);
        let boxes = list_frame(&context, &node, Vec::new()).boxes;
        assert_eq!(boxes.len(), 2);
        let close = Pos2::new(boxes[1].right() - 8.0, boxes[1].top() + 8.0);
        // Hovered, the button fades in over a few frames.
        list_frame(&context, &node, vec![egui::Event::PointerMoved(close)]);
        for _ in 0..3 {
            list_frame(&context, &node, Vec::new());
        }
        list_frame(&context, &node, vec![button(close, true, egui::Modifiers::NONE)]);
        let edits = list_frame(&context, &node, vec![button(close, false, egui::Modifiers::NONE)]);
        assert_eq!(edits.close, Some((1, false)));
        let shift = egui::Modifiers::SHIFT;
        list_frame(&context, &node, vec![egui::Event::ModifiersChanged(shift), button(close, true, shift)]);
        let edits = list_frame(&context, &node, vec![button(close, false, shift)]);
        assert_eq!(edits.close, Some((1, true)));
    }

    #[test]
    fn a_box_dragged_far_enough_drops_onto_another() {
        let context = themed_context();
        let node = node_with(&["Blur", "Clamp", "Invert"]);
        let boxes = list_frame(&context, &node, Vec::new()).boxes;
        // The Invert box has no editors: all of it drags.
        let start = boxes[2].center();
        let target = boxes[0].center();
        list_frame(&context, &node, vec![egui::Event::PointerMoved(start)]);
        list_frame(&context, &node, vec![button(start, true, egui::Modifiers::NONE)]);
        // Not 28 yet: nothing drags, and letting go there moves nothing.
        let near = start - Vec2::new(0.0, 20.0);
        list_frame(&context, &node, vec![egui::Event::PointerMoved(near)]);
        list_frame(&context, &node, vec![egui::Event::PointerMoved(target)]);
        let edits = list_frame(&context, &node, vec![button(target, false, egui::Modifiers::NONE)]);
        assert_eq!(edits.moved, Some((2, 0)));
        // A press on an editor moves its value, not the box.
        let on_slider = boxes[0].center();
        list_frame(&context, &node, vec![egui::Event::PointerMoved(on_slider)]);
        list_frame(&context, &node, vec![button(on_slider, true, egui::Modifiers::NONE)]);
        list_frame(&context, &node, vec![egui::Event::PointerMoved(boxes[2].center())]);
        let edits = list_frame(&context, &node, vec![button(boxes[2].center(), false, egui::Modifiers::NONE)]);
        assert_eq!(edits.moved, None);
    }

    #[test]
    fn three_editors_put_the_last_two_side_by_side() {
        let context = themed_context();
        // Height's Range, then Falloff beside Distort: as tall as Slope's
        // Range over its Falloff.
        let node = node_with(&["Height", "Slope"]);
        let boxes = list_frame(&context, &node, Vec::new()).boxes;
        assert_eq!(boxes.len(), 2);
        assert!((boxes[0].height() - boxes[1].height()).abs() < 1.0, "{:?}", boxes);
    }
}
