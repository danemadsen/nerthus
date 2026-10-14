//! The popups editors open over themselves: a box for typing a value
//! (`EditablePopup`; sliders, seeds, text properties, bound variables and
//! the Vars tab's rows use it), and a slider's nudge bar (`NudgePopup`):
//! nine buttons that step the value beside a box of its own.
//!
//! Both are windows of their own, which the interface scale does not
//! reach: what is in them draws at 100% whatever the scale. Callers give
//! places and widths in the editor's points, converting offsets and widths
//! given in a popup's own units with [`popup_scale`].
//!
//! The typing box closes on Enter or Tab (taking its text), on Escape, and
//! on a press anywhere else (taking its text only with Accept on Lost
//! Focus). The nudge bar closes on Escape, on Enter or Tab in its box
//! (taking the text), when the pointer leaves it, or on a press elsewhere
//! before any of its buttons was used.

use egui::text::{CCursor, CCursorRange};
use egui::{Color32, FontId, Id, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use crate::icons;
use crate::theme;

/// A slider's nudge commands (`NudgeCommand`), in the bar's order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Nudge {
    Half,
    Double,
    Min,
    Down10,
    Down1,
    Default,
    Up1,
    Up10,
    Max,
}

/// The bar's buttons: command, icon and tooltip (the command's name,
/// humanized in title case).
pub const NUDGES: [(Nudge, &str, &str); 9] = [
    (Nudge::Half, "math-1-divide-2", "Half"),
    (Nudge::Double, "multiplier-2x", "Double"),
    (Nudge::Min, "arrow-bar-to-left", "Min"),
    (Nudge::Down10, "chevrons-left", "Down 10"),
    (Nudge::Down1, "chevron-left", "Down 1"),
    (Nudge::Default, "rotate-360", "Default"),
    (Nudge::Up1, "chevron-right", "Up 1"),
    (Nudge::Up10, "chevrons-right", "Up 10"),
    (Nudge::Max, "arrow-bar-to-right", "Max"),
];

/// `NudgeButton`: 28 by 24, a 16px icon.
const NUDGE_BUTTON: Vec2 = Vec2::new(28.0, 24.0);
/// The margin between the buttons and the bar's box.
const NUDGE_GAP: f32 = 6.0;
/// `NudgePopup.MinWidth`.
const NUDGE_MIN_WIDTH: f32 = 96.0;
/// The `TextBox` style: a 1px border round 4 of padding, corners of 3.
const TEXT_BOX_BORDER: f32 = 1.0;
const TEXT_BOX_PADDING: f32 = 4.0;

/// What a popup's own sizes are multiplied by to draw at 100%: the
/// inverse of the interface scale.
pub fn popup_scale(ctx: &egui::Context) -> f32 {
    1.0 / ctx.zoom_factor()
}

/// How wide a typing box is (in the editor's points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoxWidth {
    /// Exactly this wide.
    Fixed(f32),
    /// As wide as its text needs, but no narrower than this.
    AtLeast(f32),
}

/// Where a typing box shows and how it looks.
#[derive(Clone, Debug)]
pub struct TypingLook {
    pub top_left: Pos2,
    pub width: BoxWidth,
    pub font: FontId,
}

/// A typing box while it shows: what has been typed, and whether the
/// box has been given the keyboard yet (`ShowPopup` focuses it and
/// selects its text once).
#[derive(Clone, Debug, Default)]
struct Typing {
    text: String,
    focused: bool,
    /// The pass it last showed in.
    shown: u64,
}

/// Opens the typing box `id` names, starting from `text`.
pub fn open_typing(ctx: &egui::Context, id: Id, text: String) {
    let shown = ctx.cumulative_pass_nr();
    let typing = Typing { text, focused: false, shown };
    ctx.data_mut(|data| data.insert_temp(id, typing));
}

/// A popup last shown before the previous pass belonged to an editor no
/// longer drawn (another node shown, a panel closed): it is gone.
fn went_away(ctx: &egui::Context, shown: u64) -> bool {
    shown + 1 < ctx.cumulative_pass_nr()
}

pub fn typing_open(ctx: &egui::Context, id: Id) -> bool {
    ctx.data(|data| data.get_temp::<Typing>(id)).is_some()
}

pub fn close_typing(ctx: &egui::Context, id: Id) {
    ctx.data_mut(|data| data.remove::<Typing>(id));
}

/// Shows the typing box `id` names while it is open (its font at 100%,
/// in a `TextBox`: a 1px blue border round 4 of padding); the text once
/// it closes taking it.
pub fn typing_box(ui: &Ui, id: Id, look: &TypingLook) -> Option<String> {
    let mut typing: Typing = ui.data(|data| data.get_temp(id))?;
    if went_away(ui.ctx(), typing.shown) {
        close_typing(ui.ctx(), id);
        return None;
    }
    typing.shown = ui.ctx().cumulative_pass_nr();
    let text_id = id.with("text");
    let scale = popup_scale(ui.ctx());
    let font = FontId::new(look.font.size * scale, look.font.family.clone());
    let border = TEXT_BOX_BORDER * scale;
    let padding = TEXT_BOX_PADDING * scale;
    let chrome = 2.0 * (border + padding);
    let galley = ui.painter().layout_no_wrap(typing.text.clone(), font.clone(), theme::LABEL_EMPHASIS);
    let line = ui.fonts_mut(|fonts| fonts.row_height(&font));
    let outer_width = match look.width {
        BoxWidth::Fixed(width) => width,
        // The caret's room past the text.
        BoxWidth::AtLeast(least) => (galley.size().x + 4.0 * scale + chrome).max(least),
    };
    let size = Vec2::new(outer_width, line + chrome);
    let area = egui::Area::new(id.with("area")).fixed_pos(look.top_left).order(egui::Order::Foreground);
    let shown = area.show(ui.ctx(), |ui| {
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        ui.painter().rect_filled(rect, 3.0 * scale, theme::TEXTBOX_BACKGROUND);
        ui.painter().rect_stroke(rect, 3.0 * scale, Stroke::new(border, theme::TEXTBOX_FOCUSED_BORDER), egui::StrokeKind::Inside);
        let inner = rect.shrink(border + padding);
        let mut box_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner));
        let mut edit = egui::TextEdit::singleline(&mut typing.text);
        edit = edit.id(text_id);
        edit = edit.font(font.clone());
        edit = edit.text_color(theme::LABEL_EMPHASIS);
        edit = edit.desired_width(inner.width());
        edit = edit.frame(egui::Frame::NONE);
        edit = edit.margin(egui::Margin::ZERO);
        box_ui.add(edit)
    });
    let response = shown.inner;
    if !typing.focused {
        give_keyboard(ui.ctx(), &response, &typing.text);
        typing.focused = true;
        ui.data_mut(|data| data.insert_temp(id, typing));
        return None;
    }
    let pressed_elsewhere = pressed_outside(ui, shown.response.rect);
    let outcome = if pressed_elsewhere { Some(theme::settings(ui.ctx()).accept_on_lost_focus) } else { crate::ui::widgets::popup_outcome(ui, &response) };
    let Some(accepted) = outcome else {
        ui.data_mut(|data| data.insert_temp(id, typing));
        return None;
    };
    close_typing(ui.ctx(), id);
    response.surrender_focus();
    accepted.then_some(typing.text)
}

/// A right-click asked for from outside (the launch checks): the slider
/// `id` names takes one this share along its width when it next draws.
pub fn request_right_click(ctx: &egui::Context, id: Id, along: f32) {
    ctx.data_mut(|data| data.insert_temp(id.with("asked-right-click"), along));
}

/// The right-click asked for the slider `id` names, once.
pub fn take_right_click(ctx: &egui::Context, id: Id) -> Option<f32> {
    let key = id.with("asked-right-click");
    let along: Option<f32> = ctx.data(|data| data.get_temp(key));
    if along.is_some() {
        ctx.data_mut(|data| data.remove::<f32>(key));
    }
    along
}

/// Which end of a range slider a nudge bar steps.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RangeEnd {
    Low,
    High,
}

/// A nudge bar while it shows.
#[derive(Clone, Debug)]
struct NudgeBar {
    text: String,
    focused: bool,
    /// The range end it steps; none on a single slider.
    end: Option<RangeEnd>,
    /// A button was used (`Command_Click` sets `StaysOpen`): presses
    /// elsewhere no longer close it.
    stays_open: bool,
    /// The pointer has been over the bar: leaving it now closes it
    /// (`NudgesPopup_MouseLeave`).
    entered: bool,
    /// The pass it last showed in.
    shown: u64,
}

/// Opens the nudge bar `id` names (`NudgePopup.ShowPopup`), its box
/// starting from `text`.
pub fn open_nudge(ctx: &egui::Context, id: Id, text: String, end: Option<RangeEnd>) {
    let shown = ctx.cumulative_pass_nr();
    let bar = NudgeBar { text, focused: false, end, stays_open: false, entered: false, shown };
    ctx.data_mut(|data| data.insert_temp(id, bar));
}

/// The open nudge bar's range end (`Some(None)` on a single slider).
pub fn nudge_open(ctx: &egui::Context, id: Id) -> Option<Option<RangeEnd>> {
    let bar = ctx.data(|data| data.get_temp::<NudgeBar>(id))?;
    (!went_away(ctx, bar.shown)).then_some(bar.end)
}

/// Puts the stepped value in the bar's box.
pub fn set_nudge_text(ctx: &egui::Context, id: Id, text: String) {
    ctx.data_mut(|data| {
        if let Some(mut bar) = data.get_temp::<NudgeBar>(id) {
            bar.text = text;
            data.insert_temp(id, bar);
        }
    });
}

pub fn close_nudge(ctx: &egui::Context, id: Id) {
    ctx.data_mut(|data| data.remove::<NudgeBar>(id));
}

/// What happened on a nudge bar this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NudgeOutcome {
    pub command: Option<Nudge>,
    /// The box's text, taken with Enter or Tab.
    pub typed: Option<String>,
}

/// Where a nudge bar shows and how.
#[derive(Clone, Copy, Debug)]
pub struct NudgeLook {
    pub top_left: Pos2,
    /// In the editor's points: the slider's whole width (`ActualWidth *
    /// UiScale` in the popup's own units).
    pub width: f32,
    /// The buttons' colour (the slider's).
    pub color: Color32,
    /// Whether the slider has a default for the Default button.
    pub has_default: bool,
}

/// Shows the nudge bar `id` names while it is open: the buttons on the
/// left and the box filling the rest (on a range's low end the other way
/// round, `SetTextBoxLocationLeft`).
pub fn nudge_bar(ui: &Ui, id: Id, look: &NudgeLook) -> NudgeOutcome {
    let mut outcome = NudgeOutcome::default();
    let Some(mut bar) = ui.data(|data| data.get_temp::<NudgeBar>(id)) else {
        return outcome;
    };
    if went_away(ui.ctx(), bar.shown) {
        close_nudge(ui.ctx(), id);
        return outcome;
    }
    bar.shown = ui.ctx().cumulative_pass_nr();
    let scale = popup_scale(ui.ctx());
    let button_size = NUDGE_BUTTON * scale;
    let gap = NUDGE_GAP * scale;
    let border = TEXT_BOX_BORDER * scale;
    let padding = TEXT_BOX_PADDING * scale;
    let font = theme::interface(12.0 * scale);
    let line = ui.fonts_mut(|fonts| fonts.row_height(&font));
    let height = (line + 2.0 * (border + padding)).max(button_size.y);
    let buttons_width = button_size.x * NUDGES.len() as f32;
    // As wide as asked (96 at least), and never narrower than its buttons.
    let width = look.width.max(NUDGE_MIN_WIDTH * scale).max(buttons_width);
    let box_width = (width - buttons_width - gap).max(0.0);
    let box_left = bar.end == Some(RangeEnd::Low);
    let (buttons_x, box_x) = if box_left { (box_width + gap, 0.0) } else { (0.0, buttons_width + gap) };
    let text_id = id.with("text");
    let area = egui::Area::new(id.with("area")).fixed_pos(look.top_left).order(egui::Order::Foreground);
    let shown = area.show(ui.ctx(), |ui| {
        let (whole, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        // `border`: the buttons on the nudge background.
        let buttons = Rect::from_min_size(whole.min + Vec2::new(buttons_x, 0.0), Vec2::new(buttons_width, height));
        ui.painter().rect_filled(buttons, 3.0 * scale, theme::NUDGE_BACKGROUND);
        for (index, (nudge, icon, tooltip)) in NUDGES.into_iter().enumerate() {
            let left = buttons.left() + index as f32 * button_size.x;
            let cell = Rect::from_min_size(Pos2::new(left, buttons.center().y - button_size.y / 2.0), button_size);
            let button = ui.interact(cell, id.with(("nudge", index)), Sense::click());
            let tint = if button.hovered() || button.is_pointer_button_down_on() { theme::LABEL_EMPHASIS } else { look.color };
            let icon_rect = Rect::from_center_size(cell.center(), Vec2::splat(icons::DEFAULT_SIZE * scale));
            icons::paint(ui, icon_rect, icon, tint);
            let usable = nudge != Nudge::Default || look.has_default;
            if button.on_hover_text(tooltip).clicked() && usable {
                outcome.command = Some(nudge);
            }
        }
        if box_width <= 2.0 * (border + padding) {
            return None;
        }
        let field = Rect::from_min_size(whole.min + Vec2::new(box_x, 0.0), Vec2::new(box_width, height));
        let focused = ui.memory(|memory| memory.has_focus(text_id));
        let border_color = if focused { theme::TEXTBOX_FOCUSED_BORDER } else { theme::TEXTBOX_BORDER };
        ui.painter().rect_filled(field, 3.0 * scale, theme::TEXTBOX_BACKGROUND);
        ui.painter().rect_stroke(field, 3.0 * scale, Stroke::new(border, border_color), egui::StrokeKind::Inside);
        let inner = field.shrink(border + padding);
        let mut box_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner));
        let text_color = if focused { theme::LABEL_EMPHASIS } else { theme::LABEL_DEFAULT };
        let mut edit = egui::TextEdit::singleline(&mut bar.text);
        edit = edit.id(text_id);
        edit = edit.font(font.clone());
        edit = edit.text_color(text_color);
        edit = edit.desired_width(inner.width());
        edit = edit.frame(egui::Frame::NONE);
        edit = edit.margin(egui::Margin::ZERO);
        Some(box_ui.add(edit))
    });
    let bar_rect = shown.response.rect;
    if outcome.command.is_some() {
        bar.stays_open = true;
    }
    if let Some(response) = shown.inner {
        if !bar.focused {
            give_keyboard(ui.ctx(), &response, &bar.text);
            bar.focused = true;
        } else if response.lost_focus() {
            let (entered, tabbed, escaped) = ui.input(|input| (input.key_pressed(egui::Key::Enter), input.key_pressed(egui::Key::Tab), input.key_pressed(egui::Key::Escape)));
            if escaped {
                close_nudge(ui.ctx(), id);
                return outcome;
            }
            if entered || tabbed {
                outcome.typed = Some(bar.text);
                close_nudge(ui.ctx(), id);
                return outcome;
            }
        }
    } else if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        close_nudge(ui.ctx(), id);
        return outcome;
    }
    // `NudgesPopup_MouseLeave`, and a press elsewhere while `StaysOpen`
    // is off.
    let pointer = ui.input(|input| input.pointer.latest_pos());
    let over = pointer.is_some_and(|position| bar_rect.contains(position));
    let left = bar.entered && !over;
    bar.entered |= over;
    let pressed_elsewhere = !bar.stays_open && pressed_outside(ui, bar_rect);
    if left || pressed_elsewhere {
        close_nudge(ui.ctx(), id);
        return outcome;
    }
    ui.data_mut(|data| data.insert_temp(id, bar));
    outcome
}

/// `Keyboard.Focus`, then `SelectAll`.
fn give_keyboard(ctx: &egui::Context, response: &egui::Response, text: &str) {
    response.request_focus();
    let length = text.chars().count();
    let mut state = egui::TextEdit::load_state(ctx, response.id).unwrap_or_default();
    let everything = CCursorRange::two(CCursor::new(0), CCursor::new(length));
    state.cursor.set_char_range(Some(everything));
    state.store(ctx, response.id);
}

/// A button went down this frame outside `rect`.
fn pressed_outside(ui: &Ui, rect: Rect) -> bool {
    ui.input(|input| {
        let pressed = input.pointer.any_pressed();
        let origin = input.pointer.press_origin();
        pressed && origin.is_some_and(|origin| !rect.contains(origin))
    })
}

/// The open typing box's text.
#[cfg(test)]
pub fn typing_text(ctx: &egui::Context, id: Id) -> Option<String> {
    ctx.data(|data| data.get_temp::<Typing>(id)).map(|typing| typing.text)
}

/// The open nudge bar's text.
#[cfg(test)]
pub fn nudge_text(ctx: &egui::Context, id: Id) -> Option<String> {
    ctx.data(|data| data.get_temp::<NudgeBar>(id)).map(|bar| bar.text)
}

/// `float.ToString("##0.0##")`: one to three decimals, the value first
/// taken to seven significant digits, halves rounded away from zero; a
/// negative value keeps its sign even when it comes to nothing.
pub fn short_decimal_text(value: f32) -> String {
    if !value.is_finite() {
        return non_finite_text(value);
    }
    // Seven significant digits, as the float formats for custom patterns.
    let scientific = format!("{:.6e}", value.abs());
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let digits: Vec<u8> = mantissa.bytes().filter(u8::is_ascii_digit).map(|digit| digit - b'0').collect();
    let sign = if value.is_sign_negative() { "-" } else { "" };
    // From a million up every digit is a whole one.
    let point = exponent + 1;
    if point >= digits.len() as i32 {
        let whole: String = digits.iter().map(|digit| char::from(b'0' + digit)).collect();
        let zeros = "0".repeat((point - digits.len() as i32) as usize);
        return format!("{sign}{whole}{zeros}.0");
    }
    // The digits as a whole number of thousandths, halves rounded up.
    let mut thousandths: u64 = 0;
    let mut round_up = false;
    for (index, digit) in digits.iter().enumerate() {
        let place = point - index as i32;
        if place > -3 {
            thousandths = thousandths * 10 + *digit as u64;
        } else if place == -3 {
            round_up = *digit >= 5;
            break;
        }
    }
    // Digits that stop short of the thousandths still scale to them.
    let last_place = point - digits.len() as i32;
    if last_place > -3 {
        let missing = (last_place + 3).min(18) as u32;
        thousandths *= 10u64.pow(missing);
    }
    if round_up {
        thousandths += 1;
    }
    let whole = thousandths / 1000;
    let mut fraction = format!("{:03}", thousandths % 1000);
    while fraction.len() > 1 && fraction.ends_with('0') {
        fraction.pop();
    }
    format!("{sign}{whole}.{fraction}")
}

/// `NaN`, `Infinity` and `-Infinity`, as .NET writes them.
fn non_finite_text(value: f32) -> String {
    if value.is_nan() {
        "NaN".to_string()
    } else if value > 0.0 {
        "Infinity".to_string()
    } else {
        "-Infinity".to_string()
    }
}

/// The box a slider's value is typed into: monospace 16 (`Font-Monospace`).
pub fn value_font() -> FontId {
    theme::monospace(16.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_decimals_round_like_the_custom_format() {
        assert_eq!(short_decimal_text(0.25), "0.25");
        assert_eq!(short_decimal_text(0.1), "0.1");
        assert_eq!(short_decimal_text(1.0), "1.0");
        assert_eq!(short_decimal_text(0.0), "0.0");
        assert_eq!(short_decimal_text(0.12345), "0.123");
        assert_eq!(short_decimal_text(12.5), "12.5");
        assert_eq!(short_decimal_text(1234.5), "1234.5");
        // Exactly half a thousandth rounds away from zero.
        assert_eq!(short_decimal_text(0.0625), "0.063");
        assert_eq!(short_decimal_text(-0.0625), "-0.063");
        assert_eq!(short_decimal_text(0.9999), "1.0");
        assert_eq!(short_decimal_text(-0.0001), "-0.0");
        assert_eq!(short_decimal_text(-0.0), "-0.0");
        assert_eq!(short_decimal_text(90.0), "90.0");
        assert_eq!(short_decimal_text(0.0001), "0.0");
        assert_eq!(short_decimal_text(20.0), "20.0");
        assert_eq!(short_decimal_text(1234567.0), "1234567.0");
        assert_eq!(short_decimal_text(-3.0e20), "-300000000000000000000.0");
        assert_eq!(short_decimal_text(f32::NAN), "NaN");
        // Checked against .NET.
        assert_eq!(short_decimal_text(12345678.0), "12345680.0");
        assert_eq!(short_decimal_text(123.4565), "123.457");
        assert_eq!(short_decimal_text(0.1875), "0.188");
        assert_eq!(short_decimal_text(0.00025), "0.0");
        assert_eq!(short_decimal_text(0.0005), "0.001");
        assert_eq!(short_decimal_text(-0.0005), "-0.001");
        assert_eq!(short_decimal_text(0.99995), "1.0");
        assert_eq!(short_decimal_text(0.3333333), "0.333");
    }
}
