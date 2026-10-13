//! Parameter editors (`EditorControlFactory`): which control a parameter
//! gets and how each draws, at the default size (`Size.Default.xaml`).
//! Values are the node's saved JSON.

use egui::{Color32, CornerRadius, Id, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};
use nerthus_engine::json::f32_value;
use nerthus_engine::{ParameterSpec, ValueType};
use serde_json::{json, Value};

use crate::icons;
use crate::theme;

/// `Slider.MinHeight`.
pub const SLIDER_HEIGHT: f32 = 26.0;
/// `Slider.Margin`: a slider's bar starts this far into its row.
pub const SLIDER_INSET: f32 = 20.0;
/// While a slider holds its default, its shading, thumb and value show at
/// this opacity (`SliderX.AnimateSlider`).
const DEFAULT_OPACITY: f32 = 0.4;
/// The thumb: a 2px bar with a 2px margin each side (`Slider-Thumb`).
const THUMB_WIDTH: f32 = 6.0;
/// `Column.Size`: the header column of dropdowns and toggles.
const HEADER_COLUMN: f32 = 160.0;
/// `OptionsX.Item.MinHeight`.
const OPTIONS_HEIGHT: f32 = 32.0;
/// An option's cell: text 11 high, `OptionsX.Item.Padding` 4 and
/// `OptionsX.Item.Margin` 2 above and below.
const OPTION_ROW_HEIGHT: f32 = 27.0;
/// `NudgeButton`.
const NUDGE_BUTTON: Vec2 = Vec2::new(28.0, 24.0);

/// `ParamsY`, `ParamsX` and `ParamsZ`: the palettes editors cycle through
/// (`EditorBase.colorFlip`), eight colours each.
const PALETTES: [[u32; 8]; 3] = [
    [0xe4ba4a, 0xeaa649, 0xec924e, 0xea7f55, 0xe36e5e, 0xd85f68, 0xc95371, 0xb64a79],
    [0xe4ba4a, 0xc5bf51, 0xa7c25f, 0x8ac271, 0x70c185, 0x59be97, 0x4abaa7, 0x47b4b3],
    [0x99e9f2, 0x66d9e8, 0x22b8cf, 0x15aabf, 0x1098ad, 0x0c8599, 0x0b7285, 0x456f7d],
];

pub fn palette_color(palette: usize, index: usize) -> Color32 {
    let value = PALETTES[palette % 3][index % 8];
    let red = (value >> 16) as u8;
    let green = (value >> 8) as u8;
    let blue = value as u8;
    Color32::from_rgb(red, green, blue)
}

/// What a parameter edit produced.
pub enum Edit {
    None,
    Set(Value),
    /// Draw a new seed (`RandomBox.NewRandomSeed`).
    NewSeed,
    /// A seed typed into the seed's popup (`EditablePopup_EditSuccess`),
    /// still to be worked out as a formula.
    SeedText(String),
    Browse,
    /// Back to the type's default (`EditorMenu.ResetProperty`).
    Reset,
    /// `AddNewPort` (Mixer's "Add layer").
    AddInput,
    /// Bind to a new variable (`EditorMenu.Mi_Click`).
    BindNew,
    /// Bind to this variable (`Mv_Click`).
    BindTo(String),
    /// Ask how to unbind (`Mx2_Click`).
    AskUnbind,
    /// Edit the parameter's expression.
    Expression,
    /// `CommandX`: run the node's command.
    Command,
    Unavailable(&'static str),
}

/// What shapes an editor besides its value.
pub struct EditorContext<'a> {
    /// The accent (`ParamsY/X/Z-Color{n}`, by `EditorBase.colorFlip`).
    pub color: Color32,
    /// The type's default; a slider holding it is shaded lighter.
    pub default: Option<&'a Value>,
    /// `O.PropertyArea.ActualWidth`: enums get option bars only when the
    /// property area is wider than 300.
    pub area_width: f32,
}

/// Draws the control for one parameter.
pub fn parameter(ui: &mut Ui, id: Id, spec: &ParameterSpec, value: &Value, context: &EditorContext) -> Edit {
    let display_name = spec.display_name();
    let label: &str = &display_name;
    match spec.value_type {
        ValueType::Float | ValueType::Double => {
            let current = value.as_f64().unwrap_or(0.0);
            let slider_spec = SliderSpec::of(spec, context.default, false);
            match slider(ui, id, label, current, &slider_spec, context.color) {
                Some(new_value) if spec.value_type == ValueType::Float => Edit::Set(f32_value(new_value as f32)),
                Some(new_value) => Edit::Set(json!(new_value)),
                None => Edit::None,
            }
        }
        // A byte property is an information line: its description
        // (`EditorLabel`, margins 22, 8, 8, 8).
        ValueType::Byte => {
            let description = spec.description.as_deref().unwrap_or_default();
            information(ui, description, egui::Margin { left: 22, right: 8, top: 8, bottom: 8 }, None);
            Edit::None
        }
        ValueType::Int => {
            let current = value.as_f64().unwrap_or(0.0);
            if spec.is_seed() {
                return seed_box(ui, id, label, current as i64);
            }
            let slider_spec = SliderSpec::of(spec, context.default, true);
            match slider(ui, id, label, current, &slider_spec, context.color) {
                Some(new_value) => Edit::Set(json!(new_value.round_ties_even() as i64)),
                None => Edit::None,
            }
        }
        ValueType::Bool if spec.kind == "CommandButton" => command_button(ui, label),
        ValueType::Bool => {
            let current = value.as_bool().unwrap_or(false);
            match toggle(ui, id, label, current, theme::ACCENT_PRIMARY) {
                Some(new_value) => Edit::Set(json!(new_value)),
                None => Edit::None,
            }
        }
        ValueType::Enum => {
            let current = value.as_str().unwrap_or_default().to_string();
            let style = if context.area_width > 300.0 { OptionsStyle::of(&spec.kind) } else { None };
            let chosen = match style {
                Some(style) => options(ui, label, spec, &current, style),
                None => dropdown(ui, id, label, spec, &current),
            };
            match chosen {
                Some(name) => Edit::Set(json!(name)),
                None => Edit::None,
            }
        }
        ValueType::Float2 | ValueType::Int2 => {
            let low = value.get("X").and_then(Value::as_f64).unwrap_or(0.0);
            let high = value.get("Y").and_then(Value::as_f64).unwrap_or(0.0);
            let (min, max) = range_of(spec, 0.0, 1.0);
            let integer = spec.value_type == ValueType::Int2;
            let range = RangeSpec { min, max, integer, inset: SLIDER_INSET };
            match range_slider(ui, id, label, low, high, &range, context.color) {
                Some((low, high)) if integer => Edit::Set(json!({"X": low.round_ties_even() as i64, "Y": high.round_ties_even() as i64})),
                Some((low, high)) => Edit::Set(json!({"X": f32_value(low as f32), "Y": f32_value(high as f32)})),
                None => Edit::None,
            }
        }
        ValueType::String => match spec.kind.as_str() {
            // The node's own text (margins 22, 16, 22, 16), formatted by
            // its `TextFormat(right aligned, monospace, colour)`.
            "ComputedString" => {
                let text = value.as_str().unwrap_or_default();
                if !text.is_empty() {
                    information(ui, text, egui::Margin { left: 22, right: 22, top: 16, bottom: 16 }, spec.text_format.as_deref());
                }
                Edit::None
            }
            "TextBox" => text_box(ui, id, label, value.as_str().unwrap_or_default()),
            _ => file_browser(ui, label, value.as_str().unwrap_or_default()),
        },
        ValueType::Color => color_editor(ui, label, value),
        ValueType::Float4List => crate::ui::gradient_editor::gradient_editor(ui, id, value),
        _ => {
            caption(ui, label, "(edited in a later version)");
            Edit::None
        }
    }
}

pub(crate) fn range_of(spec: &ParameterSpec, default_min: f64, default_max: f64) -> (f64, f64) {
    let min = spec.min.unwrap_or(default_min);
    let max = spec.max.unwrap_or(default_max);
    if max > min { (min, max) } else { (default_min, default_max) }
}

/// Tick marks along a slider's bar (`SliderX.TickType`, integer steps).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Ticks {
    #[default]
    None,
    /// Every whole number: integer sliders whose maximum is under 100.
    Steps,
    /// Round values along a `[Curve]` (`TicksPlacement.Ticks`, 50 at most).
    Curve,
    /// 0, 90, 180 and 270.
    Degrees360,
    /// 0, 30, 45, 60 and 90.
    Degrees90,
}

/// How a `SliderX` maps and shows its value.
#[derive(Clone, Debug)]
pub struct SliderSpec {
    pub min: f64,
    pub max: f64,
    /// The default: shading, thumb and value dim while it is held.
    pub default: Option<f64>,
    /// `[Curve]`: 0 is linear, otherwise the bar follows
    /// (e^(c·t) − 1) / (e^c − 1).
    pub curve: f64,
    pub integer: bool,
    pub ticks: Ticks,
    /// `Slider.Margin`'s left padding (none in modifier and layer editors).
    pub inset: f32,
    pub height: f32,
}

impl SliderSpec {
    /// A plain slider over `min..max`.
    pub fn linear(min: f64, max: f64) -> Self {
        SliderSpec {
            min,
            max,
            default: None,
            curve: 0.0,
            integer: false,
            ticks: Ticks::None,
            inset: SLIDER_INSET,
            height: SLIDER_HEIGHT,
        }
    }

    /// A parameter's slider (`SliderEditor_Loaded`, `SliderX.SetupControl`).
    pub fn of(spec: &ParameterSpec, default: Option<&Value>, integer: bool) -> Self {
        let fallback_max = if integer { 100.0 } else { 1.0 };
        let (min, max) = range_of(spec, 0.0, fallback_max);
        let first_curve = spec.curve.as_ref().and_then(|values| values.first());
        let curve = first_curve.and_then(Value::as_f64).unwrap_or(0.0);
        let first_tick = spec.ticks.as_ref().and_then(|values| values.first());
        let tick_type = first_tick.and_then(Value::as_str);
        let ticks = if curve != 0.0 {
            Ticks::Curve
        } else {
            match tick_type {
                Some("Degrees360") => Ticks::Degrees360,
                Some("Degrees90") => Ticks::Degrees90,
                _ if integer && max < 100.0 => Ticks::Steps,
                _ => Ticks::None,
            }
        };
        let mut slider_spec = SliderSpec::linear(min, max);
        slider_spec.default = default.and_then(Value::as_f64);
        slider_spec.curve = curve;
        slider_spec.integer = integer;
        slider_spec.ticks = ticks;
        slider_spec
    }

    /// Where `value` sits along the bar, 0 to 1 (`SliderX.To01`, in
    /// single precision).
    fn position(&self, value: f64) -> f64 {
        let span = self.max - self.min;
        if span <= 0.0 {
            return 0.0;
        }
        let fraction = ((value - self.min) / span).clamp(0.0, 1.0);
        if self.curve == 0.0 {
            return fraction;
        }
        let curve = self.curve as f32;
        let scaled = fraction as f32 * (curve.exp() - 1.0) + 1.0;
        (scaled.ln() / curve) as f64
    }

    /// The value at a position along the bar (`SliderX.ExpValue`).
    fn value_at(&self, position: f64) -> f64 {
        let position = position.clamp(0.0, 1.0);
        let fraction = if self.curve == 0.0 {
            position
        } else {
            let curve = self.curve as f32;
            let curved = ((position as f32 * curve).exp() - 1.0) / (curve.exp() - 1.0);
            curved as f64
        };
        let value = self.min + fraction * (self.max - self.min);
        // `Math.Round` and `Convert.ToInt32` round halves to even.
        if self.integer { value.round_ties_even() } else { value }
    }

    /// Tick positions, 0 to 1, besides the two ends the bar always marks.
    fn tick_positions(&self) -> Vec<f64> {
        let span = self.max - self.min;
        if span <= 0.0 {
            return Vec::new();
        }
        let at_values = |values: &[f64]| values.iter().map(|value| (value - self.min) / span).collect::<Vec<f64>>();
        match self.ticks {
            Ticks::None => Vec::new(),
            Ticks::Steps => {
                let count = span.floor() as usize;
                (1..count).map(|step| step as f64 / span).collect()
            }
            Ticks::Degrees360 => at_values(&[0.0, 90.0, 180.0, 270.0]),
            Ticks::Degrees90 => at_values(&[0.0, 30.0, 45.0, 60.0, 90.0]),
            Ticks::Curve => curve_ticks(self.min as f32, self.max as f32, self.curve as f32, 50),
        }
    }

    /// The nearest tick to `position` (`IsSnapToTickEnabled` with Shift).
    fn snap(&self, position: f64) -> f64 {
        let mut ticks = self.tick_positions();
        ticks.push(0.0);
        ticks.push(1.0);
        let nearest = ticks.into_iter().min_by(|a, b| (a - position).abs().total_cmp(&(b - position).abs()));
        nearest.unwrap_or(position)
    }

    /// `FormattedString`: two decimals, whole numbers for integers.
    fn format(&self, value: f64) -> String {
        if self.integer {
            format!("{}", value as i64)
        } else {
            format!("{value:.2}")
        }
    }
}

/// `TicksPlacement.Ticks`: round values for a curved slider, as positions.
fn curve_ticks(min: f32, max: f32, curve: f32, max_ticks: usize) -> Vec<f64> {
    let values = if curve < 6.0 { regular_tick_values(min, max, max_ticks) } else { irregular_tick_values(min, max, max_ticks) };
    values.iter().map(|value| curve_position(*value, min, max, curve)).collect()
}

/// `TicksPlacement.LogValue` (double precision).
fn curve_position(value: f64, min: f32, max: f32, curve: f32) -> f64 {
    let fraction = (value - min as f64) / (max - min) as f64;
    if curve == 0.0 {
        return fraction;
    }
    let curve = curve as f64;
    (fraction * (curve.exp() - 1.0) + 1.0).ln() / curve
}

/// `TicksRegular`: the first of 1, 2, 2.5, 5, 10 (scaled to the range's
/// decade) that keeps the count under `max_ticks`.
fn regular_tick_values(min: f32, max: f32, max_ticks: usize) -> Vec<f64> {
    const STEPS: [f64; 5] = [1.0, 2.0, 2.5, 5.0, 10.0];
    let rough = ((max - min) / max_ticks as f32) as f64;
    let magnitude = 10f64.powf(rough.log10().floor());
    let scaled = rough / magnitude;
    let index = STEPS.iter().position(|step| *step >= scaled).unwrap_or(STEPS.len() - 1);
    let step = STEPS[index] * magnitude;
    let mut value = (min as f64 / step).ceil() * step;
    if value == min as f64 {
        value += step;
    }
    let mut values = Vec::new();
    while value < max as f64 {
        values.push(value);
        value += step;
    }
    values
}

/// `TicksIrregular` (curves of 6 and more): steps of 1, 2, 2.5 or 10 that
/// grow tenfold at each decade.
fn irregular_tick_values(min: f32, max: f32, max_ticks: usize) -> Vec<f64> {
    const STEPS: [f64; 4] = [1.0, 2.0, 2.5, 10.0];
    let exponent = (min as f64).log10();
    let decade = exponent.floor();
    let magnitude = 10f64.powf(decade);
    if magnitude.is_nan() || magnitude <= 0.0 {
        return Vec::new();
    }
    let mut values = Vec::new();
    for base_step in STEPS {
        values.clear();
        let mut step = base_step * magnitude;
        let mut value = step;
        if decade == exponent {
            value += step;
        }
        let mut next_decade = magnitude * 10.0;
        while value <= min as f64 {
            value += step;
        }
        while value < max as f64 {
            values.push(value);
            value += step;
            if value == next_decade {
                step *= 10.0;
                next_decade *= 10.0;
                if value < step {
                    values.push(value);
                    value = step;
                }
            } else if value > next_decade {
                next_decade *= 10.0;
                value = next_decade;
            }
        }
        if values.len() <= max_ticks {
            break;
        }
    }
    values.sort_by(f64::total_cmp);
    values
}

/// Commands of the slider's right-click bar (`NudgePopup`), in its order.
#[derive(Clone, Copy, PartialEq)]
enum Nudge {
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

const NUDGES: [(Nudge, &str, &str); 9] = [
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

/// `NudgePopup_CommandClicked`.
fn nudged(nudge: Nudge, value: f64, spec: &SliderSpec) -> f64 {
    let tenth = (spec.max - spec.min) / 10.0;
    let new_value = match nudge {
        Nudge::Half => value / 2.0,
        Nudge::Double => value * 2.0,
        Nudge::Min => spec.min,
        Nudge::Down10 => value - tenth,
        Nudge::Down1 => value - tenth / 10.0,
        Nudge::Default => spec.default.unwrap_or(value),
        Nudge::Up1 => value + tenth / 10.0,
        Nudge::Up10 => value + tenth,
        Nudge::Max => spec.max,
    };
    let clamped = new_value.clamp(spec.min, spec.max);
    if spec.integer { clamped.round_ties_even() } else { clamped }
}

/// `SliderX`: a bar with the name on the left and the value on the right,
/// both in the accent; the part up to the value is shaded and a thin bar
/// marks it. Click or drag to set (Shift snaps to the ticks), double-click
/// to type, right-click for the nudge bar.
pub fn slider(ui: &mut Ui, id: Id, label: &str, value: f64, spec: &SliderSpec, color: Color32) -> Option<f64> {
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(Vec2::new(width, spec.height), Sense::hover());
    let content = Rect::from_min_max(Pos2::new(row.left() + spec.inset, row.top()), row.max);
    let response = ui.interact(content, id.with("slider"), Sense::click_and_drag());
    let enabled = ui.is_enabled();
    // `SliderEditor_IsEnabledChanged`: a disabled slider shows at half.
    let control_opacity = if enabled { 1.0 } else { 0.5 };
    let is_default = spec.default.is_some_and(|default| (value - default).abs() < 1e-5);
    let value_opacity = control_opacity * if is_default { DEFAULT_OPACITY } else { 1.0 };

    // `border`: the bar's frame, a pixel over the control above and below.
    let border = content.expand2(Vec2::new(0.0, 1.0));
    let painter = ui.painter();
    painter.rect_filled(border, 4.0, theme::SLIDER_BACKGROUND.gamma_multiply(control_opacity));
    painter.rect_stroke(border, 4.0, Stroke::new(1.0, theme::SLIDER_BORDER.gamma_multiply(control_opacity)), StrokeKind::Inside);

    // `_slider` (margin 3.5, 1): the shading up to the thumb (margin 1, 2,
    // 0, 2, 20% of the accent), the ticks and the thumb.
    let track = content.shrink2(Vec2::new(3.5, 1.0));
    let position = spec.position(value) as f32;
    let travel = (track.width() - THUMB_WIDTH).max(0.0);
    let thumb_left = track.left() + position * travel;
    let shading = Rect::from_min_max(Pos2::new(track.left() + 1.0, track.top() + 2.0), Pos2::new(thumb_left, track.bottom() - 2.0));
    if shading.width() > 0.0 {
        let corners = CornerRadius { nw: 3, sw: 3, ne: 0, se: 0 };
        painter.rect_filled(shading, corners, color.gamma_multiply(0.2 * value_opacity));
    }
    let tick_y = track.bottom() - 3.0 - 1.0;
    let tick_color = color.gamma_multiply(0.75 * value_opacity);
    if spec.ticks != Ticks::None {
        let mut tick_positions = spec.tick_positions();
        tick_positions.push(0.0);
        tick_positions.push(1.0);
        for tick in tick_positions {
            let x = (track.left() + THUMB_WIDTH / 2.0 + tick as f32 * travel).floor();
            let tick_rect = Rect::from_min_size(Pos2::new(x, tick_y), Vec2::splat(1.0));
            painter.rect_filled(tick_rect, 0.0, tick_color);
        }
    }
    let thumb = Rect::from_min_max(Pos2::new(thumb_left + 2.0, track.top() + 2.0), Pos2::new(thumb_left + 4.0, track.bottom() - 2.0));
    painter.rect_filled(thumb, 0.0, color.gamma_multiply(value_opacity));
    if response.hovered() {
        painter.rect_filled(thumb, 0.0, Color32::WHITE.gamma_multiply(0.3));
    }

    // `_displayName` (margin 10, -1, 8, 0) and `_valueDisplay` (18 high,
    // margin 0, 3, 10, 1, monospace 13).
    // The value keeps its room; the name gets what is left, trimmed, and
    // nothing leaves the bar.
    let text_painter = painter.with_clip_rect(content);
    let shown = spec.format(value);
    let shown_color = color.gamma_multiply(value_opacity);
    let value_galley = painter.layout_no_wrap(shown.clone(), theme::monospace(13.0), shown_color);
    let value_box_top = content.top() + 3.0 + (content.height() - 4.0 - 18.0) / 2.0;
    let value_rect = Rect::from_min_size(Pos2::new(content.right() - 10.0 - value_galley.size().x, value_box_top), Vec2::new(value_galley.size().x, 18.0));
    text_painter.galley(value_rect.min, value_galley, shown_color);
    let label_color = color.gamma_multiply(control_opacity);
    let label_left = content.left() + 10.0;
    let label_room = value_rect.left() - 8.0 - label_left;
    let label_galley = fitted_line(ui, label, theme::interface(12.0), label_color, label_room);
    let label_top = content.top() - 1.0 + (content.height() + 1.0 - label_galley.size().y) / 2.0;
    text_painter.galley(Pos2::new(label_left, label_top), label_galley, label_color);

    if let Some(typed) = slider_typing(ui, id, &shown, value_rect, spec) {
        return typed;
    }
    if let Some(nudge) = nudge_bar(ui, id, &response, content, spec, color) {
        let new_value = nudged(nudge, value, spec);
        return if (new_value - value).abs() > f64::EPSILON { Some(new_value) } else { None };
    }
    if response.double_clicked() {
        ui.data_mut(|data| data.insert_temp(id.with("typing"), spec.format(value)));
        return None;
    }
    let pressed = response.is_pointer_button_down_on() || response.dragged();
    if !pressed || !enabled {
        return None;
    }
    let pointer = response.interact_pointer_pos()?;
    let raw_position = ((pointer.x - track.left() - THUMB_WIDTH / 2.0) / travel.max(1.0)).clamp(0.0, 1.0) as f64;
    let shift = ui.input(|input| input.modifiers.shift);
    let snapped = if shift && !spec.integer { spec.snap(raw_position) } else { raw_position };
    let new_value = spec.value_at(snapped);
    if (new_value - value).abs() > f64::EPSILON { Some(new_value) } else { None }
}

/// The double-click editor (`EditablePopup`): monospace 16 over the value,
/// 16 wider than it (64 at least); Enter or leaving it commits.
/// `Some(change)` while typing.
fn slider_typing(ui: &mut Ui, id: Id, shown: &str, value_rect: Rect, spec: &SliderSpec) -> Option<Option<f64>> {
    let typing_id = id.with("typing");
    let mut text: String = ui.data(|data| data.get_temp(typing_id))?;
    let width = (value_rect.width() + 16.0).max(64.0);
    let top_left = Pos2::new(value_rect.right() + 8.0 - width, value_rect.top() - 8.0);
    let area = egui::Area::new(typing_id.with("area")).fixed_pos(top_left).order(egui::Order::Foreground);
    let response = area.show(ui.ctx(), |ui| {
        let mut frame = egui::Frame::new();
        frame.fill = theme::TEXTBOX_BACKGROUND;
        frame.stroke = Stroke::new(1.0, theme::TEXTBOX_FOCUSED_BORDER);
        frame.corner_radius = CornerRadius::same(2);
        frame.show(ui, |ui| {
            let mut edit = egui::TextEdit::singleline(&mut text);
            edit = edit.font(theme::monospace(16.0));
            edit = edit.horizontal_align(egui::Align::RIGHT);
            edit = edit.desired_width(width - 8.0);
            edit = edit.frame(egui::Frame::NONE);
            let edit_response = ui.add(edit);
            edit_response.request_focus();
            edit_response
        })
    });
    let edit_response = response.inner.inner;
    if edit_response.lost_focus() {
        ui.data_mut(|data| data.remove::<String>(typing_id));
        let escaped = ui.input(|input| input.key_pressed(egui::Key::Escape));
        if escaped || text.trim() == shown {
            return Some(None);
        }
        let parsed = text.trim().parse::<f64>().ok();
        let committed = parsed.map(|number| {
            let clamped = number.clamp(spec.min, spec.max);
            if spec.integer { clamped.round_ties_even() } else { clamped }
        });
        return Some(committed);
    }
    ui.data_mut(|data| data.insert_temp(typing_id, text));
    Some(None)
}

/// The right-click bar (`NudgePopup`): nine buttons in the accent over the
/// slider, closed when the pointer leaves it.
fn nudge_bar(ui: &mut Ui, id: Id, response: &egui::Response, content: Rect, spec: &SliderSpec, color: Color32) -> Option<Nudge> {
    let open_id = id.with("nudge-open");
    if response.secondary_clicked() && ui.is_enabled() {
        ui.data_mut(|data| data.insert_temp(open_id, true));
    }
    let open: bool = ui.data(|data| data.get_temp(open_id)).unwrap_or(false);
    if !open {
        return None;
    }
    let bar_width = NUDGE_BUTTON.x * NUDGES.len() as f32;
    let top_left = Pos2::new(content.left() - 2.0, content.center().y - NUDGE_BUTTON.y / 2.0 - 2.0);
    let area = egui::Area::new(open_id.with("area")).fixed_pos(top_left).order(egui::Order::Foreground);
    let mut chosen = None;
    let shown = area.show(ui.ctx(), |ui| {
        let mut frame = egui::Frame::new();
        frame.fill = theme::NUDGE_BACKGROUND;
        frame.corner_radius = CornerRadius::same(3);
        frame.inner_margin = egui::Margin::same(2);
        frame.show(ui, |ui| {
            ui.set_width(bar_width);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (nudge, icon, tooltip) in NUDGES {
                    let (rect, button) = ui.allocate_exact_size(NUDGE_BUTTON, Sense::click());
                    let tint = if button.hovered() { theme::LABEL_EMPHASIS } else { color };
                    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(icons::DEFAULT_SIZE));
                    icons::paint(ui, icon_rect, icon, tint);
                    let disabled_default = nudge == Nudge::Default && spec.default.is_none();
                    if button.on_hover_text(tooltip).clicked() && !disabled_default {
                        chosen = Some(nudge);
                    }
                }
            });
        });
    });
    let pointer_inside = shown.response.contains_pointer() || response.contains_pointer();
    if !pointer_inside {
        ui.data_mut(|data| data.insert_temp(open_id, false));
    }
    chosen
}

/// How a `SliderY` maps its two values.
#[derive(Clone, Debug)]
pub struct RangeSpec {
    pub min: f64,
    pub max: f64,
    pub integer: bool,
    /// `Slider.Margin` (none in the layer editor).
    pub inset: f32,
}

/// `SliderY`: a range with two thumbs; the low value on the left, the
/// name in the middle and the high value on the right, all in the accent.
pub fn range_slider(ui: &mut Ui, id: Id, label: &str, low: f64, high: f64, spec: &RangeSpec, color: Color32) -> Option<(f64, f64)> {
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(Vec2::new(width, SLIDER_HEIGHT), Sense::hover());
    let content = Rect::from_min_max(Pos2::new(row.left() + spec.inset, row.top()), row.max);
    let response = ui.interact(content, id.with("range"), Sense::click_and_drag());
    let opacity = if ui.is_enabled() { 1.0 } else { 0.5 };

    // `border` reaches two pixels past the right edge.
    let border = Rect::from_min_max(content.min, content.max + Vec2::new(2.0, 0.0));
    let painter = ui.painter();
    painter.rect_filled(border, 4.0, theme::SLIDER_BACKGROUND.gamma_multiply(opacity));
    painter.rect_stroke(border, 4.0, Stroke::new(1.0, theme::SLIDER_BORDER.gamma_multiply(opacity)), StrokeKind::Inside);

    // The two `SliderBase`s (margin 2): their thumbs, and the shaded range
    // between them (`PART_RangeTrack`, margin 2, 6).
    let track = content.shrink(2.0);
    let span = (spec.max - spec.min).max(f64::EPSILON);
    let travel = (track.width() - THUMB_WIDTH).max(0.0);
    let thumb_left = |value: f64| track.left() + (((value - spec.min) / span).clamp(0.0, 1.0) as f32) * travel;
    let low_left = thumb_left(low);
    let high_left = thumb_left(high);
    let band = Rect::from_min_max(Pos2::new(low_left + THUMB_WIDTH / 2.0, content.top() + 6.0), Pos2::new(high_left + THUMB_WIDTH / 2.0, content.bottom() - 6.0));
    if band.width() > 0.0 {
        painter.rect_filled(band, 0.0, color.gamma_multiply(0.2 * opacity));
    }
    for left in [low_left, high_left] {
        let thumb = Rect::from_min_max(Pos2::new(left + 2.0, track.top() + 2.0), Pos2::new(left + 4.0, track.bottom() - 2.0));
        painter.rect_filled(thumb, 0.0, color.gamma_multiply(opacity));
    }

    // The two values keep their room; the name sits between them,
    // trimmed, and nothing leaves the bar.
    let text_painter = painter.with_clip_rect(content);
    let text_color = color.gamma_multiply(opacity);
    let format = |value: f64| if spec.integer { format!("{}", value.round_ties_even() as i64) } else { format!("{value:.2}") };
    let low_galley = painter.layout_no_wrap(format(low), theme::monospace(13.0), text_color);
    let high_galley = painter.layout_no_wrap(format(high), theme::monospace(13.0), text_color);
    let low_top = content.top() + 2.0 + (content.height() - 3.0 - 15.0) / 2.0;
    let low_right = content.left() + 8.0 + low_galley.size().x;
    let high_left = content.right() - 8.0 - high_galley.size().x;
    text_painter.galley(Pos2::new(content.left() + 8.0, low_top), low_galley, text_color);
    text_painter.galley(Pos2::new(high_left, low_top), high_galley, text_color);
    let name_room = (high_left - low_right - 16.0).min(content.width());
    let name_galley = fitted_line(ui, label, theme::interface(12.0), text_color, name_room);
    text_painter.galley(content.center() - name_galley.size() / 2.0, name_galley, text_color);

    let active_id = id.with("thumb");
    if response.drag_started() || response.clicked() {
        let pointer_x = response.interact_pointer_pos().map(|pointer| pointer.x).unwrap_or(low_left);
        let grabs_low = (pointer_x - low_left).abs() <= (pointer_x - high_left).abs();
        ui.data_mut(|data| data.insert_temp(active_id, grabs_low));
    }
    let pressed = response.is_pointer_button_down_on() || response.dragged();
    if !pressed || !ui.is_enabled() {
        return None;
    }
    let pointer = response.interact_pointer_pos()?;
    let grabs_low: bool = ui.data(|data| data.get_temp(active_id)).unwrap_or(true);
    let fraction = ((pointer.x - track.left() - THUMB_WIDTH / 2.0) / travel.max(1.0)).clamp(0.0, 1.0) as f64;
    let mut value = spec.min + span * fraction;
    if spec.integer {
        value = value.round_ties_even();
    }
    let (new_low, new_high) = if grabs_low { (value.min(high), high) } else { (low, value.max(low)) };
    let changed = (new_low - low).abs() > f64::EPSILON || (new_high - high).abs() > f64::EPSILON;
    if changed { Some((new_low, new_high)) } else { None }
}

/// One line of text no wider than `max_width`, trimmed with an ellipsis
/// where it would run over (`TextTrimming="CharacterEllipsis"`).
pub fn fitted_line(ui: &Ui, text: &str, font: egui::FontId, color: Color32, max_width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap.max_width = max_width.max(0.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    ui.painter().layout_job(job)
}

/// A right-aligned header label in its column, vertically centred
/// (`PART_DisplayName`, margin 0, 0, 8, 0).
fn header_label(ui: &Ui, column: Rect, text: &str, color: Color32) {
    let available = (column.width() - 8.0).max(0.0);
    let galley = fitted_line(ui, text, theme::interface(12.0), color, available);
    let position = Pos2::new(column.right() - 8.0 - galley.size().x, column.center().y - galley.size().y / 2.0);
    ui.painter().galley(position, galley, color);
}

/// `RandomBox`: the name (80 wide), a button showing the seed that draws a
/// new one (`Button-Random`, tagged `dice`) or, right-clicked, lets the
/// seed be typed, and the seed menu.
fn seed_box(ui: &mut Ui, id: Id, label: &str, seed: i64) -> Edit {
    let mut edit = Edit::None;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::hover());
    // `dock` keeps 8 clear on the right.
    let dock = Rect::from_min_max(rect.min, Pos2::new(rect.right() - 8.0, rect.bottom()));

    let header = Rect::from_min_size(dock.min, Vec2::new(88.0, dock.height()));
    header_label(ui, header, label, theme::LABEL_DEFAULT);

    // `PART_MenuButton` (`Button-IconOnly`, 18 + 6 a side, margin 4).
    let menu_rect = Rect::from_min_size(Pos2::new(dock.right() - 4.0 - 30.0, dock.top()), Vec2::new(30.0, dock.height()));
    let button_rect = Rect::from_min_max(Pos2::new(header.right() + 8.0, dock.top()), Pos2::new(menu_rect.left() - 4.0, dock.bottom()));
    let button = ui.interact(button_rect, id.with("seed-button"), Sense::click());
    let hovered = button.hovered();
    let fill = if hovered { theme::BUTTON_HOVER } else { theme::ICON_DARK };
    ui.painter().rect_filled(button_rect, 2.0, fill);
    ui.painter().rect_stroke(button_rect, 2.0, Stroke::new(1.5, theme::OPTIONS_SELECTION), StrokeKind::Inside);
    // Padding 4: the dice (18, margin 4) and the seed (margin 6, 1, 0, 0).
    let icon_rect = Rect::from_min_size(Pos2::new(button_rect.left() + 4.0 + 4.0, button_rect.center().y - 9.0), Vec2::splat(18.0));
    icons::paint(ui, icon_rect, "dice", theme::ACCENT_MUTED_GRAY);
    let text_color = if hovered { theme::ACCENT_MUTED_GREEN } else { theme::LABEL_DEFAULT };
    let seed_galley = ui.painter().layout_no_wrap(seed.to_string(), theme::monospace(12.0), text_color);
    let seed_position = Pos2::new(icon_rect.right() + 6.0, button_rect.center().y - seed_galley.size().y / 2.0 + 0.5);
    // The button clips what does not fit (WPF's layout clip).
    let inside_button = button_rect.shrink(1.5);
    ui.painter().with_clip_rect(inside_button).galley(seed_position, seed_galley, text_color);
    // `NewRandomSeed`: the drawn seed is also remembered here (`LocalMemory`).
    let drawn_id = id.with("drawn-seeds");
    let saved_id = id.with("saved-seeds");
    let enabled = ui.is_enabled();
    if button.clicked() && enabled {
        edit = Edit::NewSeed;
    }
    // `PART_Textbox_MouseRightButtonUp`: an `EditablePopup` over the seed.
    let typing_id = id.with("seed-typing");
    if button.secondary_clicked() && enabled {
        ui.data_mut(|data| data.insert_temp(typing_id, seed.to_string()));
    }
    let button = button.on_hover_text("New seed");
    if let Some(typed) = seed_typing(ui, typing_id, button.rect) {
        edit = typed;
    }

    let mut menu_ui = ui.new_child(egui::UiBuilder::new().max_rect(menu_rect));
    crate::ui::widgets::icon_dropdown(&mut menu_ui, "category-plus", "Seeds", None, menu_rect.size(), |ui| {
        if let Some(chosen) = seed_memory_menu(ui, saved_id, drawn_id, seed) {
            edit = Edit::Set(json!(chosen));
        }
    });
    edit
}

/// Notes a seed the dice drew in the box's `LocalMemory`.
pub fn remember_drawn_seed(ctx: &egui::Context, id: Id, seed: i64) {
    let drawn_id = id.with("drawn-seeds");
    ctx.data_mut(|data| data.get_temp_mut_or_default::<Vec<i64>>(drawn_id).push(seed));
}

/// `RandomBox.UpdateMenu`: the node's saved seeds (`Node.SeedMemory`), the
/// seeds the dice drew in this box, "Add Current to Memory" and "Clear
/// Memory". Returns a seed picked from the lists.
fn seed_memory_menu(ui: &mut Ui, saved_id: Id, drawn_id: Id, seed: i64) -> Option<i64> {
    use crate::ui::widgets::{menu_caption, menu_item, menu_separator};
    let mut chosen = None;
    let saved: Vec<i64> = ui.data(|data| data.get_temp(saved_id)).unwrap_or_default();
    if !saved.is_empty() {
        menu_caption(ui, "Saved Seeds");
        for memory in &saved {
            if menu_item(ui, None, None, &memory.to_string(), "").clicked() {
                chosen = Some(*memory);
            }
        }
        menu_separator(ui);
    }
    let drawn: Vec<i64> = ui.data(|data| data.get_temp(drawn_id)).unwrap_or_default();
    if !drawn.is_empty() {
        menu_caption(ui, "Recent Seeds");
        let mut listed: Vec<i64> = Vec::new();
        for memory in drawn {
            if listed.contains(&memory) {
                continue;
            }
            listed.push(memory);
            if menu_item(ui, None, None, &memory.to_string(), "").clicked() {
                chosen = Some(memory);
            }
        }
        menu_separator(ui);
    }
    if menu_item(ui, Some("square-plus-2"), Some(theme::ACCENT_GREEN), "Add Current to Memory", "").clicked() && !saved.contains(&seed) {
        ui.data_mut(|data| data.get_temp_mut_or_default::<Vec<i64>>(saved_id).push(seed));
    }
    if menu_item(ui, None, None, "Clear Memory", "").clicked() {
        ui.data_mut(|data| data.remove::<Vec<i64>>(saved_id));
    }
    chosen
}

/// The seed's `EditablePopup` (`PlacementMode.RelativePoint` 4 above the
/// button, 32 wider, 64 at least; monospace 16): Enter, Tab or leaving it
/// commits the text, Escape drops it. `Some` once it closes with text.
fn seed_typing(ui: &mut Ui, typing_id: Id, button: Rect) -> Option<Edit> {
    let mut text: String = ui.data(|data| data.get_temp(typing_id))?;
    let width = (button.width() + 32.0).max(64.0);
    let top_left = Pos2::new(button.left(), button.top() - 4.0);
    let area = egui::Area::new(typing_id.with("area")).fixed_pos(top_left).order(egui::Order::Foreground);
    let response = area.show(ui.ctx(), |ui| {
        let mut frame = egui::Frame::new();
        frame.fill = theme::TEXTBOX_BACKGROUND;
        frame.stroke = Stroke::new(1.0, theme::TEXTBOX_FOCUSED_BORDER);
        frame.corner_radius = CornerRadius::same(2);
        frame.show(ui, |ui| {
            let mut edit = egui::TextEdit::singleline(&mut text);
            edit = edit.font(theme::monospace(16.0));
            edit = edit.desired_width(width - 8.0);
            edit = edit.frame(egui::Frame::NONE);
            let edit_response = ui.add(edit);
            edit_response.request_focus();
            edit_response
        })
    });
    let edit_response = response.inner.inner;
    if edit_response.lost_focus() {
        ui.data_mut(|data| data.remove::<String>(typing_id));
        let escaped = ui.input(|input| input.key_pressed(egui::Key::Escape));
        if escaped {
            return Some(Edit::None);
        }
        return Some(Edit::SeedText(text));
    }
    ui.data_mut(|data| data.insert_temp(typing_id, text));
    None
}

/// `HasShrunk`: how far a labelled editor has narrowed.
#[derive(Clone, Copy, PartialEq)]
enum Shrunk {
    None,
    Little,
    Lot,
}

/// `ToggleX`: the name in the header column (160; 100 when narrow) and
/// the switch (`ToggleButton`: 26 by 14, the accent when on).
pub fn toggle(ui: &mut Ui, id: Id, label: &str, value: bool, accent: Color32) -> Option<bool> {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 20.0), Sense::hover());
    let shrunk = if width < 300.0 { Shrunk::Little } else { Shrunk::None };
    let column = if shrunk == Shrunk::None { HEADER_COLUMN } else { 100.0 };
    let enabled = ui.is_enabled();
    let header = Rect::from_min_size(rect.min, Vec2::new(column.min(width), rect.height()));
    // Disabled: the control at 0.8, its name at 0.3, the switch at 0.33.
    let label_color = theme::LABEL_DEFAULT.gamma_multiply(if enabled { 1.0 } else { 0.3 });
    header_label(ui, header, label, label_color);

    let switch = Rect::from_min_size(Pos2::new(header.right(), rect.center().y - 7.0), Vec2::new(26.0, 14.0));
    let response = ui.interact(switch.expand(3.0), id.with("toggle"), Sense::click());
    let switch_opacity = if enabled { 1.0 } else { 0.8 * 0.33 };
    let background = if value { accent } else { theme::TOGGLE_BACKGROUND };
    ui.painter().rect_filled(switch, 7.0, background.gamma_multiply(switch_opacity));
    let knob_x = if value { switch.right() - 2.0 - 5.0 } else { switch.left() + 2.0 + 5.0 };
    let knob_color = if response.hovered() { theme::TOGGLE_HOVER } else { theme::TOGGLE_INDICATOR };
    ui.painter().circle_filled(Pos2::new(knob_x, switch.center().y), 5.0, knob_color.gamma_multiply(switch_opacity));
    if response.clicked() && enabled { Some(!value) } else { None }
}

/// Which `OptionsX` template an enum gets.
#[derive(Clone, Copy, PartialEq)]
enum OptionsStyle {
    /// `HorizontalList`: the name in an 80 column, the options in one row.
    Labelled,
    /// `HorizontalListNoLabel`: just the row, 18 in.
    NoLabel,
    /// `OptionGrid` / `OptionsGridDehumanized`: three to a row, 18 in.
    Grid { humanized: bool },
}

impl OptionsStyle {
    fn of(kind: &str) -> Option<Self> {
        match kind {
            "HorizontalList" => Some(OptionsStyle::Labelled),
            "HorizontalListNoLabel" => Some(OptionsStyle::NoLabel),
            "OptionGrid" => Some(OptionsStyle::Grid { humanized: true }),
            "OptionsGridDehumanized" => Some(OptionsStyle::Grid { humanized: false }),
            _ => None,
        }
    }
}

/// `OptionsX`: every member as a cell of one box (`ListBox-OptionsX`); the
/// chosen one lifted (`#3f3f3f`) and bright. Narrower than 300 it wraps to
/// two columns, narrower than 200 to one.
fn options(ui: &mut Ui, label: &str, spec: &ParameterSpec, current: &str, style: OptionsStyle) -> Option<String> {
    let width = ui.available_width();
    let shrunk = if width < 200.0 {
        Shrunk::Lot
    } else if width < 300.0 {
        Shrunk::Little
    } else {
        Shrunk::None
    };
    let columns = match (style, shrunk) {
        (_, Shrunk::Lot) => 1,
        (_, Shrunk::Little) => 2,
        (OptionsStyle::Grid { .. }, Shrunk::None) => 3,
        (_, Shrunk::None) => spec.enum_names.len().max(1),
    };
    let humanized = !matches!(style, OptionsStyle::Grid { humanized: false });
    let entries: Vec<String> = spec.enum_names.iter().map(|name| if humanized { spec.enum_display(name) } else { name.clone() }).collect();
    let selected = spec.enum_names.iter().position(|name| name == current);
    let enabled = ui.is_enabled();
    let opacity = if enabled { 1.0 } else { 0.3 };

    let list_left = match style {
        OptionsStyle::Labelled => 80.0,
        _ => 18.0,
    };
    let rows = entries.len().max(1).div_ceil(columns);
    let list_height = (rows as f32 * OPTION_ROW_HEIGHT + 4.0).max(OPTIONS_HEIGHT);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, list_height), Sense::hover());
    if style == OptionsStyle::Labelled {
        let header_height = if shrunk == Shrunk::Lot { OPTIONS_HEIGHT } else { rect.height() };
        let header = Rect::from_min_size(rect.min, Vec2::new(list_left, header_height));
        header_label(ui, header, label, theme::LABEL_DEFAULT.gamma_multiply(opacity));
    }
    let list = Rect::from_min_max(Pos2::new(rect.left() + list_left, rect.top()), rect.max);
    let id = ui.id().with(("options", &spec.name));
    let picked = option_cells(ui, id, list, &entries, selected, columns, 11.0);
    picked.map(|index| spec.enum_names[index].clone())
}

/// A `ListBox-OptionsX` laid out by the caller: `entries` in `columns`
/// columns at `font_size`; returns the index clicked when it is not the
/// selected one.
pub fn option_list(ui: &mut Ui, id: Id, entries: &[String], selected: Option<usize>, columns: usize, font_size: f32) -> Option<usize> {
    let width = ui.available_width();
    let columns = columns.max(1);
    let rows = entries.len().max(1).div_ceil(columns);
    // An item: the text's line, padding 4 and margin 2 above and below.
    let row_height = font_size * 1.36 + 8.0 + 4.0;
    let height = rows as f32 * row_height + 4.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    option_cells(ui, id, rect, entries, selected, columns, font_size)
}

/// Paints the box and its cells into `list` (border 1, grid margin 2, 1,
/// item margin 2) and handles clicks.
fn option_cells(ui: &mut Ui, id: Id, list: Rect, entries: &[String], selected: Option<usize>, columns: usize, font_size: f32) -> Option<usize> {
    let enabled = ui.is_enabled();
    let opacity = if enabled { 1.0 } else { 0.3 };
    ui.painter().rect_filled(list, 4.0, theme::OPTIONS_BACKGROUND);
    ui.painter().rect_stroke(list, 4.0, Stroke::new(1.0, theme::OPTIONS_BORDER), StrokeKind::Inside);
    let columns = columns.max(1);
    let rows = entries.len().max(1).div_ceil(columns);
    let grid = list.shrink(1.0).shrink2(Vec2::new(2.0, 1.0));
    let cell_width = grid.width() / columns as f32;
    let cell_height = grid.height() / rows as f32;
    let mut chosen = None;
    for (index, entry) in entries.iter().enumerate() {
        let column = index % columns;
        let row = index / columns;
        let cell_origin = Pos2::new(grid.left() + column as f32 * cell_width, grid.top() + row as f32 * cell_height);
        let cell = Rect::from_min_size(cell_origin, Vec2::new(cell_width, cell_height));
        let item = cell.shrink(2.0);
        let response = ui.interact(item, id.with(index), Sense::click());
        let is_selected = selected == Some(index);
        let hovered = response.hovered() && enabled;
        if is_selected {
            ui.painter().rect_filled(item, 2.0, theme::OPTIONS_SELECTION.gamma_multiply(opacity));
        } else if hovered {
            ui.painter().rect_filled(item, 2.0, theme::OPTIONS_SELECTION.gamma_multiply(0.4));
        }
        let text_color = if is_selected || hovered { theme::LABEL_EMPHASIS } else { theme::OPTIONS_LABEL };
        let shown_color = text_color.gamma_multiply(opacity);
        let galley = fitted_line(ui, entry, theme::interface(font_size), shown_color, item.width() - 16.0);
        ui.painter().galley(item.center() - galley.size() / 2.0, galley, shown_color);
        if response.clicked() && enabled && !is_selected {
            chosen = Some(index);
        }
    }
    chosen
}

/// `DropdownX`: the name in the header column (160; 100 narrower than 300;
/// none narrower than 150) and a 26-high box (`ToggleButton-ComboBox`)
/// opening the list.
fn dropdown(ui: &mut Ui, id: Id, label: &str, spec: &ParameterSpec, current: &str) -> Option<String> {
    let names: Vec<(String, String)> = spec.enum_names.iter().map(|name| (name.clone(), spec.enum_display(name))).collect();
    let shown = spec.enum_display(current);
    dropdown_row(ui, id, label, &names, &shown, current)
}

/// `DropdownX` over (value, text) entries; returns a value picked other
/// than `current`.
pub fn dropdown_row(ui: &mut Ui, id: Id, label: &str, entries: &[(String, String)], shown: &str, current: &str) -> Option<String> {
    let width = ui.available_width();
    let shrunk = if width < 150.0 {
        Shrunk::Lot
    } else if width < 300.0 {
        Shrunk::Little
    } else {
        Shrunk::None
    };
    let column = match shrunk {
        Shrunk::None => HEADER_COLUMN,
        Shrunk::Little => 100.0,
        Shrunk::Lot => 18.0,
    };
    let header = if shrunk == Shrunk::Lot { None } else { Some(label) };
    let chosen = dropdown_box(ui, id, header, column, 26.0, shown, entries, current, theme::interface(12.0));
    chosen.filter(|name| name != current)
}

/// The dropdown box and its list, shared by `DropdownX` (26 high) and
/// `LiteComboBox` (20): `entries` are (value, text) shown in `font`;
/// returns the picked value.
#[allow(clippy::too_many_arguments)]
pub fn dropdown_box(ui: &mut Ui, id: Id, header: Option<&str>, column: f32, height: f32, shown: &str, entries: &[(String, String)], current: &str, font: egui::FontId) -> Option<String> {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let enabled = ui.is_enabled();
    let opacity = if enabled { 1.0 } else { 0.5 };
    let header_rect = Rect::from_min_size(rect.min, Vec2::new(column.min(width), rect.height()));
    if let Some(header) = header {
        header_label(ui, header_rect, header, theme::LABEL_DEFAULT.gamma_multiply(opacity));
    }
    let box_rect = Rect::from_min_max(Pos2::new(header_rect.right(), rect.top()), rect.max);
    let response = ui.interact(box_rect, id.with("dropdown"), Sense::click());
    let fill = if response.hovered() && enabled { theme::DROPDOWN_HOVER } else { theme::DROPDOWN_BACKGROUND };
    ui.painter().rect_filled(box_rect, 2.0, fill.gamma_multiply(opacity));
    ui.painter().rect_stroke(box_rect, 2.0, Stroke::new(1.0, theme::DROPDOWN_BORDER), StrokeKind::Inside);
    // The arrow (M0,0 L4,4 L8,0) 7 from the right; the text 12 in.
    let arrow_right = box_rect.right() - 7.0;
    let arrow_top = box_rect.center().y - 2.0;
    let arrow = vec![Pos2::new(arrow_right - 8.0, arrow_top), Pos2::new(arrow_right - 4.0, arrow_top + 4.0), Pos2::new(arrow_right, arrow_top)];
    ui.painter().add(egui::Shape::convex_polygon(arrow, theme::LABEL_DEFAULT.gamma_multiply(opacity), Stroke::NONE));
    let text_color = theme::LABEL_DEFAULT.gamma_multiply(opacity);
    let mut job = egui::text::LayoutJob::simple_singleline(shown.to_string(), font.clone(), text_color);
    job.wrap.max_width = (box_rect.width() - 12.0 - 23.0).max(0.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(Pos2::new(box_rect.left() + 12.0, box_rect.center().y - galley.size().y / 2.0), galley, text_color);

    if !enabled {
        return None;
    }
    let mut chosen = None;
    let mut frame = egui::Frame::new();
    frame.fill = theme::POPUP_BACKGROUND;
    frame.stroke = Stroke::new(1.0, theme::POPUP_BORDER);
    frame.corner_radius = CornerRadius::same(3);
    frame.inner_margin = egui::Margin::symmetric(2, 0);
    let mut popup = egui::Popup::from_toggle_button_response(&response);
    popup = popup.frame(frame);
    popup = popup.gap(0.0);
    popup = popup.width(box_rect.width());
    popup.show(|ui| {
        ui.set_min_width(box_rect.width() - 4.0);
        let mut scroll = egui::ScrollArea::vertical();
        scroll = scroll.max_height(340.0);
        scroll.show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (value, text) in entries {
                // `ComboBoxItem`: 25 high, text 10 in, hover fill inset 2.
                let (item, item_response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 25.0), Sense::click());
                let hovered = item_response.hovered();
                if hovered {
                    ui.painter().rect_filled(item.shrink(2.0), 2.0, theme::ACCENT_MUTED_GRAY);
                }
                let is_current = value == current;
                let color = if hovered || is_current { theme::LABEL_EMPHASIS } else { theme::LABEL_DEFAULT };
                let room = item.width() - 10.0 - 4.0;
                let galley = fitted_line(ui, text, font.clone(), color, room);
                ui.painter().galley(Pos2::new(item.left() + 10.0, item.center().y - galley.size().y / 2.0), galley, color);
                if item_response.clicked() {
                    chosen = Some(value.clone());
                    ui.close();
                }
            }
        });
    });
    chosen
}

/// `FileBrowser`: the name (90 column), the file name in blue on a dark
/// field, and a folder button.
fn file_browser(ui: &mut Ui, label: &str, path: &str) -> Edit {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 32.0), Sense::hover());
    let header = Rect::from_min_size(rect.min, Vec2::new(90.0, rect.height()));
    header_label(ui, header, label, theme::LABEL_DEFAULT);
    let field = Rect::from_min_max(Pos2::new(header.right(), rect.top() + 3.0), Pos2::new(rect.right() - 30.0, rect.bottom() - 3.0));
    ui.painter().rect_filled(field, 3.0, theme::TEXTBOX_BACKGROUND);
    ui.painter().rect_stroke(field, 3.0, Stroke::new(1.0, theme::TEXTBOX_FILE_BORDER), StrokeKind::Inside);
    let shown = std::path::Path::new(path).file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
    let galley = fitted_line(ui, &shown, theme::interface(12.0), theme::TEXTBOX_FILE_FOREGROUND, field.width() - 11.0);
    ui.painter().galley(Pos2::new(field.left() + 9.0, field.center().y - galley.size().y / 2.0), galley, theme::TEXTBOX_FILE_FOREGROUND);
    let field_response = ui.interact(field, ui.id().with(("file-field", label)), Sense::hover());
    if !path.is_empty() {
        field_response.on_hover_text(path);
    }
    let button_rect = Rect::from_min_max(Pos2::new(rect.right() - 30.0, rect.top()), rect.max);
    let mut button_ui = ui.new_child(egui::UiBuilder::new().max_rect(button_rect));
    let browse = crate::ui::widgets::icon_button(&mut button_ui, "folder", "Browse...", None);
    if browse.clicked() { Edit::Browse } else { Edit::None }
}

/// `CommandX`: a button that runs the node's command (margin 22, 8, 8, 8;
/// padding 24, 6).
fn command_button(ui: &mut Ui, label: &str) -> Edit {
    // `Button-Template`: disabled, a darker fill and the text at half.
    let enabled = ui.is_enabled();
    let text_color = theme::LABEL_DEFAULT.gamma_multiply(if enabled { 1.0 } else { 0.5 });
    let fill = if enabled { theme::BUTTON_BACKGROUND } else { theme::BUTTON_DISABLED_BACKGROUND };
    let text = RichText::new(label).font(theme::interface(12.0)).color(text_color);
    let mut button = egui::Button::new(text);
    button = button.fill(fill);
    button = button.stroke(Stroke::new(1.0, theme::BUTTON_BORDER_ALT));
    button = button.corner_radius(2);
    button = button.min_size(Vec2::new(87.0, 0.0));
    let mut clicked = false;
    let mut frame = egui::Frame::new();
    frame.inner_margin = egui::Margin { left: 22, right: 8, top: 8, bottom: 8 };
    frame.show(ui, |ui| {
        ui.spacing_mut().button_padding = Vec2::new(24.0, 6.0);
        clicked = ui.add(button).clicked();
    });
    if clicked { Edit::Command } else { Edit::None }
}

/// `ColorPickerEditor` (margin 20, 4, 4, 4): the name and the colour.
fn color_editor(ui: &mut Ui, label: &str, value: &Value) -> Edit {
    let current = color_of(value);
    let mut rgba = egui::Rgba::from_rgba_unmultiplied(current[0], current[1], current[2], current[3]);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::hover());
    let header = Rect::from_min_size(rect.min, Vec2::new(90.0 + 20.0, rect.height()));
    header_label(ui, header, label, theme::LABEL_DEFAULT);
    let swatch = Rect::from_min_max(Pos2::new(header.right(), rect.top() + 4.0), Pos2::new(rect.right() - 4.0, rect.bottom() - 4.0));
    let mut swatch_ui = ui.new_child(egui::UiBuilder::new().max_rect(swatch));
    swatch_ui.spacing_mut().interact_size = swatch.size();
    let changed = egui::color_picker::color_edit_button_rgba(&mut swatch_ui, &mut rgba, egui::color_picker::Alpha::Opaque).changed();
    if !changed {
        return Edit::None;
    }
    let [red, green, blue, alpha] = rgba.to_rgba_unmultiplied();
    let mut updated = value.clone();
    set_color(&mut updated, [red, green, blue, alpha]);
    Edit::Set(updated)
}

/// `EditorLabel`: wrapped text in the label colour (12).
fn information(ui: &mut Ui, text: &str, margin: egui::Margin, format: Option<&[Value]>) {
    let right_aligned = format.and_then(|format| format.first()).and_then(Value::as_bool).unwrap_or(false);
    let monospace = format.and_then(|format| format.get(1)).and_then(Value::as_bool).unwrap_or(false);
    let font = if monospace { theme::monospace(12.0) } else { theme::interface(12.0) };
    let rich = RichText::new(text).font(font).color(theme::LABEL_DEFAULT);
    let frame = egui::Frame::new().inner_margin(margin);
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        let layout = if right_aligned { egui::Layout::top_down(egui::Align::Max) } else { egui::Layout::top_down(egui::Align::Min) };
        ui.with_layout(layout, |ui| {
            ui.add(egui::Label::new(rich).wrap());
        });
    });
}

/// `TextEditor` (margin 20, 2, 4, 2; a 90 header column): edits a copy
/// while focused and commits it when focus leaves.
fn text_box(ui: &mut Ui, id: Id, label: &str, current: &str) -> Edit {
    let buffer_id = id.with("text-buffer");
    let mut buffer: String = ui.data(|data| data.get_temp(buffer_id)).unwrap_or_else(|| current.to_string());
    let mut frame = egui::Frame::new();
    frame.inner_margin = egui::Margin { left: 20, right: 4, top: 2, bottom: 2 };
    let response = frame
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                let (header, _) = ui.allocate_exact_size(Vec2::new(90.0, 26.0), Sense::hover());
                header_label(ui, header, label, theme::LABEL_DEFAULT);
                let mut edit = egui::TextEdit::multiline(&mut buffer);
                edit = edit.font(theme::monospace(12.0));
                edit = edit.text_color(theme::ACCENT_MUTED_BLUE);
                edit = edit.desired_rows(2);
                edit = edit.desired_width(ui.available_width());
                ui.add(edit)
            })
            .inner
        })
        .inner;
    if response.has_focus() {
        ui.data_mut(|data| data.insert_temp(buffer_id, buffer));
        return Edit::None;
    }
    ui.data_mut(|data| data.remove::<String>(buffer_id));
    if response.lost_focus() && buffer != current {
        return Edit::Set(json!(buffer));
    }
    Edit::None
}

/// A read-only value beside its name (90 header column).
fn caption(ui: &mut Ui, label: &str, text: &str) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, SLIDER_HEIGHT), Sense::hover());
    let header = Rect::from_min_size(rect.min, Vec2::new(90.0, rect.height()));
    header_label(ui, header, label, theme::LABEL_DEFAULT);
    let room = rect.right() - 8.0 - header.right();
    let galley = fitted_line(ui, text, theme::interface(12.0), theme::LABEL_EMPHASIS, room);
    ui.painter().galley(Pos2::new(header.right(), rect.center().y - galley.size().y / 2.0), galley, theme::LABEL_EMPHASIS);
}

fn color_of(value: &Value) -> [f32; 4] {
    let component = |names: &[&str], fallback: f32| names.iter().find_map(|name| value.get(*name).and_then(Value::as_f64)).map(|number| number as f32).unwrap_or(fallback);
    [component(&["R", "X"], 0.0), component(&["G", "Y"], 0.0), component(&["B", "Z"], 0.0), component(&["A", "W"], 1.0)]
}

fn set_color(value: &mut Value, rgba: [f32; 4]) {
    let uses_xyzw = value.get("X").is_some();
    let names = if uses_xyzw { ["X", "Y", "Z", "W"] } else { ["R", "G", "B", "A"] };
    if !value.is_object() {
        *value = json!({});
    }
    for (name, component) in names.iter().zip(rgba) {
        value[*name] = f32_value(component);
    }
}

/// `SatMapListBox`: the palettes of the node's library as 72 by 28 strips
/// filling the width (`VirtualizingWrapPanel`, uniform), the chosen one
/// framed in yellow with its number; at most 360 tall.
pub fn library_list(ui: &mut Ui, id: Id, library: &str, current: i64) -> Option<i64> {
    let Some(palettes) = nerthus_nodes::colour_library::category(library) else {
        caption(ui, "Library", "(not available)");
        return None;
    };
    let mut chosen = None;
    let mut frame = egui::Frame::new();
    frame.fill = theme::PANEL_CLADDING;
    frame.stroke = Stroke::new(1.0, theme::LISTBOX_BORDER);
    frame.corner_radius = CornerRadius::same(4);
    frame.inner_margin = egui::Margin { left: 4, right: 2, top: 4, bottom: 4 };
    frame.outer_margin = egui::Margin { left: 16, right: 0, top: 0, bottom: 24 };
    frame.show(ui, |ui| {
        let mut scroll = egui::ScrollArea::vertical();
        scroll = scroll.id_salt(id.with("library"));
        scroll = scroll.max_height(360.0 - 8.0);
        scroll.show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let width = ui.available_width();
            let columns = ((width / 72.0).floor() as usize).max(1);
            let item_width = width / columns as f32;
            for (row, chunk) in palettes.chunks(columns).enumerate() {
                ui.horizontal(|ui| {
                    for (column, palette) in chunk.iter().enumerate() {
                        let index = (row * columns + column) as i64;
                        let (rect, response) = ui.allocate_exact_size(Vec2::new(item_width, 28.0), Sense::click());
                        let texture = palette_texture(ui.ctx(), library, index as usize, palette);
                        let strip = rect.shrink(1.0);
                        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                        ui.painter().image(texture.id(), strip, uv, Color32::WHITE);
                        if index == current {
                            ui.painter().rect_stroke(rect.shrink(0.5), 0.0, Stroke::new(1.0, theme::ACCENT_YELLOW), StrokeKind::Inside);
                            let label = ui.painter().layout_no_wrap(format!("{index:03}"), theme::interface(10.0), theme::LABEL_DEFAULT);
                            let label_rect = Rect::from_min_size(Pos2::new(rect.right() - 8.0 - label.size().x - 8.0, rect.center().y - label.size().y / 2.0 - 2.0), label.size() + Vec2::new(8.0, 4.0));
                            ui.painter().rect_filled(label_rect, 0.0, Color32::BLACK);
                            ui.painter().galley(label_rect.min + Vec2::new(4.0, 2.0), label, theme::LABEL_DEFAULT);
                        }
                        if response.clicked() && index != current {
                            chosen = Some(index);
                        }
                    }
                });
            }
        });
    });
    chosen
}

/// A palette as a 512 by 1 texture, made once.
fn palette_texture(context: &egui::Context, library: &str, index: usize, palette: &nerthus_nodes::colour_library::Palette) -> egui::TextureHandle {
    let key = Id::new(("palette-texture", library.to_string(), index));
    if let Some(texture) = context.data(|data| data.get_temp::<egui::TextureHandle>(key)) {
        return texture;
    }
    let pixels: Vec<Color32> = palette.iter().map(|[red, green, blue]| Color32::from_rgb(*red, *green, *blue)).collect();
    let image = egui::ColorImage::new([palette.len(), 1], pixels);
    let texture = context.load_texture(format!("palette-{library}-{index}"), image, egui::TextureOptions::LINEAR);
    context.data_mut(|data| data.insert_temp(key, texture.clone()));
    texture
}

/// What the curve editor remembers between frames.
#[derive(Clone, Default)]
struct CurveDrag {
    /// Which points cannot leave their x: the ones at (0,0) and (1,1) when
    /// the editor opened (`ControlPointThumb.Terminal`).
    terminal: Vec<bool>,
    dragging: Option<usize>,
    offset: Vec2,
    pressed_at: Pos2,
    just_added: bool,
}

/// `CurveEditor` (a `Spline1DCanvas`): 164 tall in a purple-bordered box.
/// Clicking empty space adds a point, dragging moves one (terminal points
/// only up and down), clicking a point without moving removes it.
pub fn curve_editor(ui: &mut Ui, id: Id, points: &[[f32; 2]]) -> Option<Vec<[f32; 2]>> {
    let memory_id = id.with("curve-drag");
    let mut drag: CurveDrag = ui.data(|data| data.get_temp(memory_id)).unwrap_or_default();
    if drag.terminal.len() != points.len() {
        drag.terminal = points.iter().map(|point| *point == [0.0, 0.0] || *point == [1.0, 1.0]).collect();
    }
    let mut points = points.to_vec();
    let mut changed = false;

    let mut frame = egui::Frame::new();
    frame.fill = theme::PANEL_CLADDING;
    frame.stroke = Stroke::new(1.5, theme::ACCENT_MUTED_PURPLE);
    frame.outer_margin = egui::Margin { left: 8, right: 0, top: 8, bottom: 8 };
    frame.show(ui, |ui| {
        let width = ui.available_width().min(256.0);
        let (canvas, response) = ui.allocate_exact_size(Vec2::new(width, 164.0), Sense::click_and_drag());
        // The thumbs may sit over the border, as on a WPF canvas.
        let painter = ui.painter_at(canvas.expand(8.0));
        let to_screen = |point: [f32; 2]| Pos2::new(canvas.left() + point[0] * canvas.width(), canvas.top() + (1.0 - point[1]) * canvas.height());
        let to_point = |position: Pos2| [(position.x - canvas.left()) / canvas.width(), 1.0 - (position.y - canvas.top()) / canvas.height()];

        // `GridTileBrush`.
        let grid_painter = ui.painter_at(canvas);
        for step in 1..4 {
            let fraction = step as f32 / 4.0;
            let x = canvas.left() + fraction * canvas.width();
            let y = canvas.top() + fraction * canvas.height();
            let stroke = Stroke::new(1.0, theme::GRID_COARSE);
            grid_painter.line_segment([Pos2::new(x, canvas.top()), Pos2::new(x, canvas.bottom())], stroke);
            grid_painter.line_segment([Pos2::new(canvas.left(), y), Pos2::new(canvas.right(), y)], stroke);
        }

        let pointer = response.interact_pointer_pos().or_else(|| response.hover_pos());
        let hit = |position: Pos2, points: &[[f32; 2]]| points.iter().position(|point| to_screen(*point).distance(position) <= 8.0);

        if (response.drag_started() || (response.is_pointer_button_down_on() && drag.dragging.is_none() && ui.input(|input| input.pointer.primary_pressed())))
            && let Some(position) = pointer {
                drag.pressed_at = position;
                match hit(position, &points) {
                    Some(index) => {
                        drag.dragging = Some(index);
                        drag.offset = to_screen(points[index]) - position;
                        drag.just_added = false;
                    }
                    None => {
                        points.push(to_point(position));
                        drag.terminal.push(false);
                        drag.dragging = Some(points.len() - 1);
                        drag.offset = Vec2::ZERO;
                        drag.just_added = true;
                        changed = true;
                    }
                }
            }

        if let (Some(index), Some(position)) = (drag.dragging, pointer)
            && (response.dragged() || response.is_pointer_button_down_on()) {
                let target = position + drag.offset;
                let terminal = drag.terminal.get(index).copied().unwrap_or(false);
                let mut x = if terminal { to_screen(points[index]).x } else { target.x };
                let right_limit = canvas.right() - if terminal { 0.0 } else { 0.1 };
                x = x.clamp(canvas.left(), right_limit);
                let y = target.y.clamp(canvas.top(), canvas.bottom());
                let moved = to_point(Pos2::new(x, y));
                if moved != points[index] {
                    points[index] = moved;
                    changed = true;
                }
            }

        let released = ui.input(|input| input.pointer.primary_released());
        if released
            && let Some(index) = drag.dragging.take() {
                let still = pointer.is_some_and(|position| position.distance(drag.pressed_at) < 1.0);
                let terminal = drag.terminal.get(index).copied().unwrap_or(false);
                if still && !drag.just_added && !terminal {
                    points.remove(index);
                    drag.terminal.remove(index);
                    changed = true;
                }
            }

        // The curve as the build evaluates it.
        let pairs: Vec<(f32, f32)> = points.iter().map(|point| (point[0], point[1])).collect();
        let spline = nerthus_nodes::spline::IrregularSpline::from_unsorted(&pairs);
        let samples: Vec<Pos2> = (0..=128)
            .map(|step| {
                let x = step as f32 / 128.0;
                let y = spline.interpolate(x).clamp(-0.5, 1.5);
                to_screen([x, y])
            })
            .collect();
        painter.add(egui::Shape::line(samples, Stroke::new(2.0, Color32::from_rgb(208, 208, 128))));

        // `ControlPointThumb`s.
        let hover = response.hover_pos().and_then(|position| hit(position, &points));
        for (index, point) in points.iter().enumerate() {
            let center = to_screen(*point);
            painter.circle_stroke(center, 7.5, Stroke::new(5.0, Color32::BLACK.gamma_multiply(0.5)));
            let color = if hover == Some(index) || drag.dragging == Some(index) { Color32::from_rgb(0xbb, 0xff, 0xbb) } else { Color32::from_rgb(208, 208, 128) };
            painter.circle_stroke(center, 6.0, Stroke::new(2.0, color));
        }
    });

    ui.data_mut(|data| data.insert_temp(memory_id, drag));
    if changed { Some(points) } else { None }
}

/// `VariableEditorControl` for a Macro: an editor per variable by its
/// type (`VariableEditorBase.AddControl`), changing each `Value` in place.
/// Returns the whole changed dictionary.
pub fn macro_variables(ui: &mut Ui, id: Id, variables: &Value, color: Color32) -> Option<Value> {
    let entries = variables.as_object()?;
    let mut changed = variables.clone();
    let mut any = false;
    for (name, variable) in entries {
        if name.starts_with('$') {
            continue;
        }
        let kind = variable.get("Type").and_then(Value::as_str).unwrap_or("Undefined");
        let label = variable.get("Name").and_then(Value::as_str).unwrap_or(name);
        let value = variable.get("Value").cloned().unwrap_or(Value::Null);
        let min = variable.get("Min").and_then(Value::as_f64).unwrap_or(0.0);
        let max = variable.get("Max").and_then(Value::as_f64).unwrap_or(1.0);
        let variable_id = id.with(("variable", name));
        let new_value = match kind {
            "Float" => {
                let spec = SliderSpec::linear(min, max);
                slider(ui, variable_id, label, value.as_f64().unwrap_or(0.0), &spec, color).map(|number| f32_value(number as f32))
            }
            "Int" => {
                let mut spec = SliderSpec::linear(min, max);
                spec.integer = true;
                slider(ui, variable_id, label, value.as_f64().unwrap_or(0.0), &spec, color).map(|number| json!(number.round_ties_even() as i64))
            }
            "Range" => {
                let low = value.get("X").and_then(Value::as_f64).unwrap_or(min);
                let high = value.get("Y").and_then(Value::as_f64).unwrap_or(max);
                let spec = RangeSpec { min, max, integer: false, inset: SLIDER_INSET };
                range_slider(ui, variable_id, label, low, high, &spec, color).map(|(low, high)| {
                    let mut range = value.clone();
                    if !range.is_object() {
                        range = json!({});
                    }
                    range["X"] = f32_value(low as f32);
                    range["Y"] = f32_value(high as f32);
                    range
                })
            }
            "Choice" => {
                let choices: Vec<String> = string_list(variable.get("ExtraData"));
                let current = value.as_i64().unwrap_or(0).max(0) as usize;
                let entries: Vec<(String, String)> = choices.iter().enumerate().map(|(index, choice)| (index.to_string(), choice.clone())).collect();
                let shown = choices.get(current).cloned().unwrap_or_default();
                let picked = dropdown_box(ui, variable_id, Some(label), HEADER_COLUMN, 26.0, &shown, &entries, &current.to_string(), theme::interface(12.0));
                picked.and_then(|index| index.parse::<i64>().ok()).filter(|index| *index as usize != current).map(|index| json!(index))
            }
            "Bool" => toggle(ui, variable_id, label, value.as_bool().unwrap_or(false), theme::ACCENT_PRIMARY).map(|flag| json!(flag)),
            "Decoration" => None,
            _ => {
                caption(ui, label, "");
                None
            }
        };
        if let Some(new_value) = new_value {
            changed[name]["Value"] = new_value;
            any = true;
        }
    }
    if any { Some(changed) } else { None }
}

/// A saved `List<string>` (`$values` or a plain array).
pub fn string_list(value: Option<&Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    let items = value.get("$values").unwrap_or(value);
    items.as_array().map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_text_is_trimmed_to_its_room() {
        let context = egui::Context::default();
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0));
        let input = egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        };
        let mut trimmed = None;
        let mut whole = None;
        let output = context.run_ui(input, |ui| {
            trimmed = Some(fitted_line(ui, "Suspended Load", theme::interface(12.0), Color32::WHITE, 40.0));
            whole = Some(fitted_line(ui, "Seed", theme::interface(12.0), Color32::WHITE, 200.0));
        });
        output.drop_without_applying_deltas();
        let trimmed = trimmed.unwrap();
        assert!(trimmed.size().x <= 40.0);
        // The galley keeps the job's text; the glyphs show what is drawn.
        let last_glyph = trimmed.rows[0].glyphs.last().map(|glyph| glyph.chr);
        assert_eq!(last_glyph, Some('…'));
        let whole = whole.unwrap();
        let drawn: String = whole.rows[0].glyphs.iter().map(|glyph| glyph.chr).collect();
        assert_eq!(drawn, "Seed");
    }

    #[test]
    fn curve_positions_round_trip() {
        let mut spec = SliderSpec::linear(1.0, 250.0);
        spec.curve = 7.0;
        for value in [1.0, 2.0, 10.0, 125.0, 250.0] {
            let position = spec.position(value);
            let back = spec.value_at(position);
            assert!((back - value).abs() < 1e-3 * value.max(1.0), "{value} -> {position} -> {back}");
        }
    }

    #[test]
    fn regular_ticks_pick_round_steps() {
        // 0..1 over at most 50 ticks: a step of 0.02.
        let values = regular_tick_values(0.0, 1.0, 50);
        assert!((49..=50).contains(&values.len()), "{}", values.len());
        assert!((values[0] - 0.02).abs() < 1e-9);
        // 0..20: 0.4 rounds up to 0.5.
        let values = regular_tick_values(0.0, 20.0, 50);
        assert!((values[0] - 0.5).abs() < 1e-9);
        assert_eq!(values.len(), 39);
    }

    #[test]
    fn irregular_ticks_grow_by_decade() {
        // ThermalShaper's Scale: 1..250, curve 7.
        let values = irregular_tick_values(1.0, 250.0, 50);
        assert!(values.len() <= 50, "{values:?}");
        assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(values.first().is_some_and(|first| *first > 1.0));
        assert!(values.last().is_some_and(|last| *last < 250.0));
    }

    #[test]
    fn integer_sliders_tick_each_step() {
        let mut spec = SliderSpec::linear(1.0, 14.0);
        spec.integer = true;
        spec.ticks = Ticks::Steps;
        assert_eq!(spec.tick_positions().len(), 12);
    }

    #[test]
    fn nudges_clamp_and_round() {
        let mut spec = SliderSpec::linear(0.0, 10.0);
        spec.default = Some(4.0);
        assert_eq!(nudged(Nudge::Double, 8.0, &spec), 10.0);
        assert_eq!(nudged(Nudge::Up10, 5.0, &spec), 6.0);
        assert_eq!(nudged(Nudge::Default, 5.0, &spec), 4.0);
        spec.integer = true;
        assert_eq!(nudged(Nudge::Half, 5.0, &spec), 2.0);
    }
}
