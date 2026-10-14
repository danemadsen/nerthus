//! Parameter editors (`EditorControlFactory`): which control a parameter
//! gets and how each draws, at the default size (`Size.Default.xaml`).
//! Values are the node's saved JSON.

use egui::{Color32, CornerRadius, Id, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};
use nerthus_engine::json::f32_value;
use nerthus_engine::{ParameterSpec, ValueType};
use serde_json::{json, Value};

use crate::icons;
use crate::theme;
use crate::ui::slider_popups::{self, BoxWidth, Nudge, NudgeLook, RangeEnd, TypingLook};

/// While a slider holds its default, its shading, thumb and value show at
/// this opacity (`SliderX.AnimateSlider`).
const DEFAULT_OPACITY: f32 = 0.4;
/// The thumb: a 2px bar with a 2px margin each side (`Slider-Thumb`).
const THUMB_WIDTH: f32 = 6.0;
/// The line of an option's 11-point text.
const OPTION_LINE: f32 = 15.0;
/// A slider's typing box: no narrower than this (`MinWidth`).
const SLIDER_TYPING_MIN_WIDTH: f32 = 64.0;
/// A range slider's typing boxes: this wide in the editor's points (`48 *
/// UiScale` in the popup's own units).
const RANGE_TYPING_WIDTH: f32 = 48.0;
/// The largest seed (`ParameterAttribute(Parameters.Seed)`'s `Max`).
const SEED_MAX: f64 = 65535.0;

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
    /// Open the documentation at this page (Help with Automation).
    ShowDocs(&'static str),
    /// A bound parameter's variable given this text
    /// (`VariableEditor.EditablePopup_EditSuccess`).
    BoundText(String),
    /// A bound parameter double-clicked: the Data View's variables.
    ShowVariables,
}

/// What shapes an editor besides its value.
pub struct EditorContext<'a> {
    /// The accent (`ParamsY/X/Z-Color{n}`, by `EditorBase.colorFlip`).
    pub color: Color32,
    /// The type's default; a slider holding it is shaded lighter.
    pub default: Option<&'a Value>,
    /// The node's values, for editors that show other properties (the
    /// palette behind SatelliteMap's Range).
    pub values: Option<&'a indexmap::IndexMap<String, Value>>,
    /// `O.PropertyArea.ActualWidth`: enums get option bars only when the
    /// property area is wider than 300.
    pub area_width: f32,
    /// The lowest and highest value of the shown node's main output
    /// (`B.GetTerrainRange`), which terrain-mark sliders mark.
    pub terrain_range: Option<(f64, f64)>,
}

/// Draws the control for one parameter.
pub fn parameter(ui: &mut Ui, id: Id, spec: &ParameterSpec, value: &Value, context: &EditorContext) -> Edit {
    let display_name = spec.display_name();
    let label: &str = &display_name;
    match spec.value_type {
        ValueType::Float | ValueType::Double => {
            let current = value.as_f64().unwrap_or(0.0);
            let mut slider_spec = SliderSpec::of(spec, context.default, false);
            slider_spec.terrain_range = context.terrain_range;
            match slider(ui, id, label, current, &slider_spec, context.color) {
                Some(new_value) if spec.value_type == ValueType::Float => Edit::Set(f32_value(new_value as f32)),
                Some(new_value) => Edit::Set(json!(new_value)),
                None => Edit::None,
            }
        }
        // A byte property is an information line: its `[Description]`
        // (`EditorLabel`, margins 22, 8, 8, 8).
        ValueType::Byte => {
            let text = spec.information.as_deref().unwrap_or_default();
            information(ui, text, egui::Margin { left: 22, right: 8, top: 8, bottom: 8 }, None);
            Edit::None
        }
        ValueType::Int => {
            let current = value.as_f64().unwrap_or(0.0);
            let seed_slider = spec.is_seed() && theme::settings(ui.ctx()).seed_sliders;
            if spec.is_seed() && !seed_slider {
                return seed_box(ui, id, label, current as i64);
            }
            let mut slider_spec = SliderSpec::of(spec, context.default, true);
            // A seed's `ParameterAttribute` runs from 0 to 65535.
            if seed_slider {
                slider_spec.min = 0.0;
                slider_spec.max = SEED_MAX;
                slider_spec.ticks = Ticks::None;
            }
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
            let mut range = RangeSpec::linear(min, max);
            range.integer = integer;
            range.default = context.default.and_then(range_pair);
            // `Parameters.RangeClipper` gets a palette range slider.
            let look = if spec.kind == "RangeClipper" {
                RangeLook::Palette(range_palette(ui.ctx(), context.values))
            } else {
                RangeLook::Accent { label, color: context.color }
            };
            match styled_range_slider(ui, id, low, high, &range, &look) {
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
            _ => file_browser(ui, id, label, value.as_str().unwrap_or_default()),
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

/// A saved range (`Float2`, `Int2`): its low and high values.
pub(crate) fn range_pair(value: &Value) -> Option<(f64, f64)> {
    let low = value.get("X").and_then(Value::as_f64)?;
    let high = value.get("Y").and_then(Value::as_f64)?;
    Some((low, high))
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
    /// The shown node's lowest and highest heights
    /// (`SetupTerrainMarks`: Snow's Snow Line, Trees' Snowline).
    TerrainMarks,
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
    /// `Slider.Margin`'s left padding (none in modifier and layer editors)
    /// and the height; `None` is the size theme's (`Slider.Margin`,
    /// `Slider.MinHeight`).
    pub inset: Option<f32>,
    pub height: Option<f32>,
    /// What `Ticks::TerrainMarks` marks: the shown node's lowest and
    /// highest heights, once it has been built.
    pub terrain_range: Option<(f64, f64)>,
    /// `Tag="modifier"`: a borderless bar with smaller corners.
    pub modifier: bool,
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
            inset: None,
            height: None,
            terrain_range: None,
            modifier: false,
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
                Some("TerrainMarks") => Ticks::TerrainMarks,
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
            // Marks on or past the ends are left to the ends' own.
            Ticks::TerrainMarks => {
                let Some((lowest, highest)) = self.terrain_range else {
                    return Vec::new();
                };
                let inside: Vec<f64> = [lowest, highest].into_iter().filter(|value| *value > self.min && *value < self.max).collect();
                at_values(&inside)
            }
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

    /// `FormattedString`: two decimals, or four when `precise`; whole
    /// numbers for integers.
    fn format(&self, value: f64, precise: bool) -> String {
        if self.integer {
            format!("{}", value as i64)
        } else if precise {
            format!("{value:.4}")
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

/// `NudgePopup_CommandClicked`, in single precision as the slider keeps
/// its value: Down 10 and Up 10 step a tenth of the range, Down 1 and
/// Up 1 a hundredth, and the result is held to the range (`SetValue`).
fn nudged(nudge: Nudge, value: f64, spec: &SliderSpec) -> f64 {
    let value = value as f32;
    let min = spec.min as f32;
    let max = spec.max as f32;
    let tenth = (max - min) / 10.0;
    let new_value = match nudge {
        Nudge::Half => value * 0.5,
        Nudge::Double => value * 2.0,
        Nudge::Min => min,
        Nudge::Down10 => value - tenth,
        Nudge::Down1 => value - tenth / 10.0,
        Nudge::Default => spec.default.map_or(value, |default| default as f32),
        Nudge::Up1 => value + tenth / 10.0,
        Nudge::Up10 => value + tenth,
        Nudge::Max => max,
    };
    let clamped = new_value.clamp(min, max) as f64;
    if spec.integer { clamped.round_ties_even() } else { clamped }
}

/// `EditablePopup_EditSuccess`: the typed text worked out as a formula
/// (the project's variables, `Resolution`, `PX`, `m()`…) and held to the
/// range; nothing when it does not work out. A formula that comes to NaN
/// changes nothing either.
fn typed_value(ctx: &egui::Context, text: &str, spec: &SliderSpec) -> Option<f64> {
    let number = crate::ui::formulas::evaluate(ctx, text)?;
    if number.is_nan() {
        return None;
    }
    let clamped = number.clamp(spec.min as f32, spec.max as f32) as f64;
    Some(if spec.integer { clamped.round_ties_even() } else { clamped })
}

/// `SliderX`: a bar with the name on the left and the value on the right,
/// both in the accent; the part up to the value is shaded and a thin bar
/// marks it. Click or drag to set (Shift snaps to the ticks). A
/// right-click on the value opens a box to type a value or a formula in;
/// anywhere else, the nudge bar.
pub fn slider(ui: &mut Ui, id: Id, label: &str, value: f64, spec: &SliderSpec, color: Color32) -> Option<f64> {
    let sizes = theme::sizes(ui.ctx());
    let width = ui.available_width();
    let height = spec.height.unwrap_or(sizes.slider_height);
    let inset = spec.inset.unwrap_or(sizes.slider_inset);
    let (row, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    // The whole control takes right-clicks, its padding too.
    let row_response = ui.interact(row, id.with("slider-row"), Sense::click());
    let content = Rect::from_min_max(Pos2::new(row.left() + inset, row.top()), row.max);
    let response = ui.interact(content, id.with("slider"), Sense::click_and_drag());
    let enabled = ui.is_enabled();
    let nudge_id = id.with("nudge");
    let typing_id = id.with("typing");
    // `SliderEditor_IsEnabledChanged`: a disabled slider shows at half.
    let control_opacity = if enabled { 1.0 } else { 0.5 };
    let is_default = spec.default.is_some_and(|default| (value - default).abs() < 1e-5);
    let value_opacity = control_opacity * if is_default { DEFAULT_OPACITY } else { 1.0 };

    // `border`: the bar's frame, a pixel over the control above and below,
    // outlined `Slider.BorderStroke.Thickness` wide (in a modifier,
    // corners of 2 and no outline).
    let border = content.expand2(Vec2::new(0.0, 1.0));
    let corner = if spec.modifier { 2.0 } else { 4.0 };
    let painter = ui.painter();
    painter.rect_filled(border, corner, theme::SLIDER_BACKGROUND.gamma_multiply(control_opacity));
    if sizes.slider_border > 0.0 && !spec.modifier {
        let outline = Stroke::new(sizes.slider_border, theme::SLIDER_BORDER.gamma_multiply(control_opacity));
        painter.rect_stroke(border, corner, outline, StrokeKind::Inside);
    }

    // `_slider` (margin 3.5, 1): the shading up to the thumb (margin
    // `Slider.InnerMargin`, 20% of the accent), the ticks and the thumb.
    let track = content.shrink2(Vec2::new(3.5, 1.0));
    let position = spec.position(value) as f32;
    let travel = (track.width() - THUMB_WIDTH).max(0.0);
    let thumb_left = track.left() + position * travel;
    let [shading_left, shading_top, _, shading_bottom] = sizes.slider_shading_margin;
    let shading_min = Pos2::new(track.left() + shading_left, track.top() + shading_top);
    let shading = Rect::from_min_max(shading_min, Pos2::new(thumb_left, track.bottom() - shading_bottom));
    if shading.width() > 0.0 {
        let corners = CornerRadius { nw: 3, sw: 3, ne: 0, se: 0 };
        painter.rect_filled(shading, corners, color.gamma_multiply(0.2 * value_opacity));
    }
    let tick_y = track.bottom() - 3.0 - 1.0;
    let tick_color = color.gamma_multiply(0.75 * value_opacity);
    // An integer slider's steps are marked only with `ShowTicks`.
    let settings = theme::settings(ui.ctx());
    let ticks_shown = spec.ticks != Ticks::None && (spec.ticks != Ticks::Steps || settings.show_ticks);
    if ticks_shown {
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
    // nothing leaves the bar. The name hides while the nudge bar shows
    // (`NudgePopup_Opened`).
    let text_painter = painter.with_clip_rect(content);
    let shown = spec.format(value, settings.precise_values);
    let shown_color = color.gamma_multiply(value_opacity);
    let value_galley = painter.layout_no_wrap(shown, theme::monospace(13.0), shown_color);
    let value_box_top = content.top() + 3.0 + (content.height() - 4.0 - 18.0) / 2.0;
    let value_rect = Rect::from_min_size(Pos2::new(content.right() - 10.0 - value_galley.size().x, value_box_top), Vec2::new(value_galley.size().x, 18.0));
    text_painter.galley(value_rect.min, value_galley, shown_color);
    let nudging = slider_popups::nudge_open(ui.ctx(), nudge_id).is_some();
    if !nudging {
        let label_color = color.gamma_multiply(control_opacity);
        let label_left = content.left() + 10.0;
        let label_room = value_rect.left() - 8.0 - label_left;
        let label_galley = fitted_line(ui, label, theme::interface(12.0), label_color, label_room);
        let label_top = content.top() - 1.0 + (content.height() + 1.0 - label_galley.size().y) / 2.0;
        text_painter.galley(Pos2::new(label_left, label_top), label_galley, label_color);
    }

    // `SliderX_MouseRightButtonUp`: from a tenth of the height before the
    // value's text on, a box to type in; before that, the nudge bar. Both
    // start from the value at four decimals.
    let precise = spec.format(value, true);
    let asked = slider_popups::take_right_click(ui.ctx(), id);
    let right_clicked = response.secondary_clicked() || row_response.secondary_clicked() || asked.is_some();
    if right_clicked && enabled {
        let pointer = ui.input(|input| input.pointer.interact_pos()).map_or(row.left(), |pointer| pointer.x);
        let pointer_x = asked.map_or(pointer, |along| row.left() + along * row.width());
        let typing_from = row.width() - value_rect.width() + row.height() * 0.1;
        if pointer_x - row.left() < typing_from {
            slider_popups::open_nudge(ui.ctx(), nudge_id, precise.clone(), None);
        } else {
            slider_popups::open_typing(ui.ctx(), typing_id, precise.clone());
        }
    }
    // The box: 8 left of and above the value, as wide as its text and 64
    // at least (the popup's own units).
    let popup_scale = slider_popups::popup_scale(ui.ctx());
    let typing_look = TypingLook {
        top_left: value_rect.min - Vec2::splat(8.0 * popup_scale),
        width: BoxWidth::AtLeast(SLIDER_TYPING_MIN_WIDTH * popup_scale),
        font: slider_popups::value_font(),
    };
    if let Some(text) = slider_popups::typing_box(ui, typing_id, &typing_look) {
        // Taking the text it opened with changes nothing.
        if text.trim() == precise {
            return None;
        }
        return typed_value(ui.ctx(), &text, spec).filter(|new_value| (new_value - value).abs() > f64::EPSILON);
    }
    // The bar: 2 left of and above the slider's bar, as wide as the whole
    // control.
    let nudge_look = NudgeLook {
        top_left: track.min - Vec2::splat(2.0 * popup_scale),
        width: row.width(),
        color,
        has_default: spec.default.is_some(),
    };
    let nudge = slider_popups::nudge_bar(ui, nudge_id, &nudge_look);
    if let Some(command) = nudge.command {
        let new_value = nudged(command, value, spec);
        // The bar's box shows the value it came to.
        slider_popups::set_nudge_text(ui.ctx(), nudge_id, spec.format(new_value, true));
        return ((new_value - value).abs() > f64::EPSILON).then_some(new_value);
    }
    if let Some(text) = nudge.typed {
        return typed_value(ui.ctx(), &text, spec).filter(|new_value| (new_value - value).abs() > f64::EPSILON);
    }

    // Only the left button sets the value.
    let primary_down = ui.input(|input| input.pointer.primary_down());
    let pressed = response.is_pointer_button_down_on() && primary_down;
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

/// How a `SliderY` maps its two values.
#[derive(Clone, Debug)]
pub struct RangeSpec {
    pub min: f64,
    pub max: f64,
    pub integer: bool,
    /// `Slider.Margin` (none in the layer editor); `None` is the size
    /// theme's.
    pub inset: Option<f32>,
    /// The type's default range (`DefaultValue`), for the nudge bar's
    /// Default.
    pub default: Option<(f64, f64)>,
    /// `Tag="modifier"`: a borderless bar with smaller corners.
    pub modifier: bool,
}

impl RangeSpec {
    /// A range over `min..max`.
    pub fn linear(min: f64, max: f64) -> Self {
        RangeSpec {
            min,
            max,
            integer: false,
            inset: None,
            default: None,
            modifier: false,
        }
    }
}

/// How a `SliderY` is drawn.
pub enum RangeLook<'a> {
    /// The `SliderY` style: a dark bar with the thumbs, the values and the
    /// name in the accent, and the range shaded in it.
    Accent { label: &'a str, color: Color32 },
    /// The `RangeSlider` style of a palette range slider (SatelliteMap's
    /// and SuperColor's Range): 46 high over the node's palette, the thumbs
    /// and values in white smoke, the range outlined in black and no name.
    /// Without a palette it keeps the style's fill, the primary accent.
    Palette(Option<PaletteFill>),
}

/// The palette stretched across a palette range slider.
pub struct PaletteFill {
    texture: egui::TextureId,
    /// `Reverse`: high to low.
    reversed: bool,
}

/// `Height` of a palette range slider.
const PALETTE_SLIDER_HEIGHT: f32 = 46.0;
/// The `BorderBrush` a palette range slider is given: `Brushes.WhiteSmoke`.
const PALETTE_SLIDER_FOREGROUND: Color32 = Color32::from_rgb(0xf5, 0xf5, 0xf5);

/// The palette a palette range slider shows: item `LibraryItem` of the
/// node's `Library`, reversed when `Reverse` is on. None while the item is
/// not in the library (an unset item is -1).
fn range_palette(context: &egui::Context, values: Option<&indexmap::IndexMap<String, Value>>) -> Option<PaletteFill> {
    let values = values?;
    let library = values.get("Library").and_then(Value::as_str)?;
    let item = values.get("LibraryItem").and_then(Value::as_i64)?;
    let index = usize::try_from(item).ok()?;
    let palettes = nerthus_nodes::colour_library::category(library)?;
    let palette = palettes.get(index)?;
    let texture = palette_texture(context, library, index, palette);
    let reversed = values.get("Reverse").and_then(Value::as_bool).unwrap_or(false);
    Some(PaletteFill { texture: texture.id(), reversed })
}

/// `SliderY`: a range with two thumbs; the low value on the left, the
/// name in the middle and the high value on the right, all in the accent.
pub fn range_slider(ui: &mut Ui, id: Id, label: &str, low: f64, high: f64, spec: &RangeSpec, color: Color32) -> Option<(f64, f64)> {
    let look = RangeLook::Accent { label, color };
    styled_range_slider(ui, id, low, high, spec, &look)
}

/// A `SliderY` in either look. Drag a thumb, or hold Shift and drag to
/// move the whole range. A right-click on the left fifth opens a box to
/// type the low value (a formula, like a slider's), on the right fifth the
/// high one; in between, the nudge bar for the end on that side.
pub fn styled_range_slider(ui: &mut Ui, id: Id, low: f64, high: f64, spec: &RangeSpec, look: &RangeLook) -> Option<(f64, f64)> {
    let sizes = theme::sizes(ui.ctx());
    let width = ui.available_width();
    let height = match look {
        RangeLook::Accent { .. } => sizes.slider_height,
        RangeLook::Palette(_) => PALETTE_SLIDER_HEIGHT,
    };
    let inset = spec.inset.unwrap_or(sizes.slider_inset);
    let (row, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    // The whole control takes right-clicks, its padding too.
    let row_response = ui.interact(row, id.with("range-row"), Sense::click());
    let content = Rect::from_min_max(Pos2::new(row.left() + inset, row.top()), row.max);
    let response = ui.interact(content, id.with("range"), Sense::click_and_drag());
    let enabled = ui.is_enabled();
    let opacity = if enabled { 1.0 } else { 0.5 };
    let accent = match look {
        RangeLook::Accent { color, .. } => *color,
        RangeLook::Palette(_) => PALETTE_SLIDER_FOREGROUND,
    };
    let nudge_id = id.with("nudge");
    let low_typing_id = id.with("typing-low");
    let high_typing_id = id.with("typing-high");

    // `border` reaches two pixels past the right edge (in a modifier,
    // corners of 2 and no outline).
    let border = Rect::from_min_max(content.min, content.max + Vec2::new(2.0, 0.0));
    let corner = if spec.modifier { 2.0 } else { 4.0 };
    let painter = ui.painter();
    let border_stroke = Stroke::new(1.0, theme::SLIDER_BORDER.gamma_multiply(opacity));
    match look {
        RangeLook::Accent { .. } => {
            painter.rect_filled(border, corner, theme::SLIDER_BACKGROUND.gamma_multiply(opacity));
        }
        // The palette fills the bar from end to end (an `ImageBrush`).
        RangeLook::Palette(Some(fill)) => {
            let uv = if fill.reversed {
                Rect::from_min_max(Pos2::new(1.0, 0.0), Pos2::new(0.0, 1.0))
            } else {
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))
            };
            let tint = Color32::WHITE.gamma_multiply(opacity);
            let shape = egui::epaint::RectShape::filled(border, corner, tint).with_texture(fill.texture, uv);
            painter.add(shape);
        }
        RangeLook::Palette(None) => {
            painter.rect_filled(border, corner, theme::ACCENT_PRIMARY.gamma_multiply(opacity));
        }
    }
    if !spec.modifier {
        painter.rect_stroke(border, corner, border_stroke, StrokeKind::Inside);
    }

    // The two values keep their room; the name sits between them,
    // trimmed, and nothing leaves the bar. The range (`Panel.ZIndex` 50)
    // and the thumbs (100) lie over them. The name hides while the nudge
    // bar shows (`NudgePopup_Opened`).
    let text_painter = painter.with_clip_rect(content);
    let text_color = accent.gamma_multiply(opacity);
    let precise = theme::settings(ui.ctx()).precise_values;
    let low_text = range_text(low, spec.integer, precise);
    let high_text = range_text(high, spec.integer, precise);
    let low_galley = painter.layout_no_wrap(low_text.clone(), theme::monospace(13.0), text_color);
    let high_galley = painter.layout_no_wrap(high_text.clone(), theme::monospace(13.0), text_color);
    // `PART_ValueDisplay` and `PART_MaxValueDisplay`: 15 high, 8 in.
    let value_top = content.top() + 2.0 + (content.height() - 3.0 - 15.0) / 2.0;
    let low_rect = Rect::from_min_size(Pos2::new(content.left() + 8.0, value_top), Vec2::new(low_galley.size().x, 15.0));
    let high_rect = Rect::from_min_size(Pos2::new(content.right() - 8.0 - high_galley.size().x, value_top), Vec2::new(high_galley.size().x, 15.0));
    text_painter.galley(low_rect.min, low_galley, text_color);
    text_painter.galley(high_rect.min, high_galley, text_color);
    let nudge_end = slider_popups::nudge_open(ui.ctx(), nudge_id).flatten();
    if let RangeLook::Accent { label, .. } = look
        && nudge_end.is_none()
    {
        let name_room = (high_rect.left() - low_rect.right() - 16.0).min(content.width());
        let name_galley = fitted_line(ui, label, theme::interface(12.0), text_color, name_room);
        text_painter.galley(content.center() - name_galley.size() / 2.0, name_galley, text_color);
    }
    // Each value's `ToolTip` is its text.
    ui.interact(low_rect, id.with("low-value"), Sense::hover()).on_hover_text(low_text);
    ui.interact(high_rect, id.with("high-value"), Sense::hover()).on_hover_text(high_text);

    // The two `SliderBase`s (margin 2) and their thumbs.
    let track = content.shrink(2.0);
    let span = (spec.max - spec.min).max(f64::EPSILON);
    let share = |value: f64| ((value - spec.min) / span).clamp(0.0, 1.0) as f32;
    let travel = (track.width() - THUMB_WIDTH).max(0.0);
    let thumb_left = |value: f64| track.left() + share(value) * travel;
    let low_left = thumb_left(low);
    let high_left = thumb_left(high);
    // `PART_RangeTrack` as `UpdateUIElements` places it: from 8 in plus the
    // low value's share of the width less 8, to the high value's share,
    // 4 from the top and the bottom.
    let usable = content.width() - 8.0;
    let band_left = content.left() + 8.0 + share(low) * usable;
    let band_width = ((share(high) - share(low)) * usable - 8.0).max(0.0);
    let band = Rect::from_min_size(Pos2::new(band_left, content.top() + 4.0), Vec2::new(band_width, content.height() - 8.0));
    if band.width() > 0.0 {
        match look {
            RangeLook::Accent { .. } => {
                painter.rect_filled(band, 0.0, accent.gamma_multiply(0.2 * opacity));
            }
            RangeLook::Palette(_) => {
                let outline = Stroke::new(1.5, Color32::BLACK.gamma_multiply(opacity));
                painter.rect_stroke(band, 0.0, outline, StrokeKind::Inside);
            }
        }
    }
    for left in [low_left, high_left] {
        let thumb = Rect::from_min_max(Pos2::new(left + 2.0, track.top() + 2.0), Pos2::new(left + 4.0, track.bottom() - 2.0));
        painter.rect_filled(thumb, 0.0, accent.gamma_multiply(opacity));
    }

    // `SliderY_MouseRightButtonUp`, by where along the whole control.
    let asked = slider_popups::take_right_click(ui.ctx(), id);
    let right_clicked = response.secondary_clicked() || row_response.secondary_clicked() || asked.is_some();
    if right_clicked && enabled {
        let pointer = ui.input(|input| input.pointer.interact_pos()).map_or(row.left(), |pointer| pointer.x);
        let pointer_x = asked.map_or(pointer, |along| row.left() + along * row.width());
        let along = (pointer_x - row.left()) as f64 / row.width() as f64;
        match range_click_target(along) {
            Some(RangeClick::TypeLow) => slider_popups::open_typing(ui.ctx(), low_typing_id, slider_popups::short_decimal_text(low as f32)),
            Some(RangeClick::NudgeLow) => slider_popups::open_nudge(ui.ctx(), nudge_id, range_text(low, spec.integer, true), Some(RangeEnd::Low)),
            Some(RangeClick::NudgeHigh) => slider_popups::open_nudge(ui.ctx(), nudge_id, range_text(high, spec.integer, true), Some(RangeEnd::High)),
            Some(RangeClick::TypeHigh) => slider_popups::open_typing(ui.ctx(), high_typing_id, slider_popups::short_decimal_text(high as f32)),
            None => {}
        }
    }
    // The boxes (48 wide at the interface scale): the low one 8 left of
    // and above its value, the high one 38 right of its value's start
    // (offsets in the popup's own units).
    let popup_scale = slider_popups::popup_scale(ui.ctx());
    let changed = |new_low: f64, new_high: f64| {
        let moved = (new_low - low).abs() > f64::EPSILON || (new_high - high).abs() > f64::EPSILON;
        moved.then_some((new_low, new_high))
    };
    let low_look = TypingLook {
        top_left: low_rect.min + Vec2::new(-8.0, -8.0) * popup_scale,
        width: BoxWidth::Fixed(RANGE_TYPING_WIDTH),
        font: slider_popups::value_font(),
    };
    if let Some(text) = slider_popups::typing_box(ui, low_typing_id, &low_look) {
        let typed = typed_range(ui.ctx(), &text, low, high, spec, RangeEnd::Low)?;
        return changed(typed.0, typed.1);
    }
    let high_look = TypingLook {
        top_left: high_rect.min + Vec2::new(38.0, -8.0) * popup_scale,
        width: BoxWidth::Fixed(RANGE_TYPING_WIDTH),
        font: slider_popups::value_font(),
    };
    if let Some(text) = slider_popups::typing_box(ui, high_typing_id, &high_look) {
        let typed = typed_range(ui.ctx(), &text, low, high, spec, RangeEnd::High)?;
        return changed(typed.0, typed.1);
    }
    // The bar covers the control from 2 before the sliders' corner (they
    // sit 2 in), as wide as the whole control; its buttons take the
    // style's accent.
    let button_color = match look {
        RangeLook::Accent { color, .. } => *color,
        RangeLook::Palette(_) => theme::ACCENT_PRIMARY,
    };
    let nudge_look = NudgeLook {
        top_left: content.min + Vec2::splat(2.0 - 2.0 * popup_scale),
        width: row.width(),
        color: button_color,
        has_default: spec.default.is_some(),
    };
    let nudge = slider_popups::nudge_bar(ui, nudge_id, &nudge_look);
    if let (Some(command), Some(end)) = (nudge.command, nudge_end) {
        let (new_low, new_high) = range_nudged(command, low, high, spec, end);
        let stepped = if end == RangeEnd::Low { new_low } else { new_high };
        slider_popups::set_nudge_text(ui.ctx(), nudge_id, range_text(stepped, spec.integer, true));
        return changed(new_low, new_high);
    }
    if let (Some(text), Some(end)) = (nudge.typed, nudge_end) {
        let typed = typed_range(ui.ctx(), &text, low, high, spec, end)?;
        return changed(typed.0, typed.1);
    }

    // The left button: Shift held as it goes down moves the whole range
    // (`CenterDrag`); otherwise it takes the nearer thumb.
    let active_id = id.with("thumb");
    let moving_id = id.with("moving");
    let (pressed_now, primary_down, shift) = ui.input(|input| (input.pointer.primary_pressed(), input.pointer.primary_down(), input.modifiers.shift));
    let held = response.is_pointer_button_down_on() && primary_down;
    if !held || !enabled {
        ui.data_mut(|data| data.remove::<f32>(moving_id));
        return None;
    }
    let pointer = response.interact_pointer_pos()?;
    if pressed_now {
        if shift {
            ui.data_mut(|data| data.insert_temp(moving_id, pointer.x));
            return None;
        }
        ui.data_mut(|data| data.remove::<f32>(moving_id));
        let grabs_low = (pointer.x - low_left).abs() <= (pointer.x - high_left).abs();
        ui.data_mut(|data| data.insert_temp(active_id, grabs_low));
    }
    if let Some(last_x) = ui.data(|data| data.get_temp::<f32>(moving_id)) {
        ui.data_mut(|data| data.insert_temp(moving_id, pointer.x));
        // It moves only while Shift is held, by the pointer's travel over
        // the whole control.
        if !shift {
            return None;
        }
        let delta = (pointer.x - last_x) as f64 / row.width().max(1.0) as f64 * (spec.max - spec.min);
        let (mut new_low, mut new_high) = center_drag(low, high, delta, spec);
        if spec.integer {
            new_low = new_low.round_ties_even();
            new_high = new_high.round_ties_even();
        }
        return changed(new_low, new_high);
    }
    let grabs_low: bool = ui.data(|data| data.get_temp(active_id)).unwrap_or(true);
    let fraction = ((pointer.x - track.left() - THUMB_WIDTH / 2.0) / travel.max(1.0)).clamp(0.0, 1.0) as f64;
    let mut value = spec.min + span * fraction;
    if spec.integer {
        value = value.round_ties_even();
    }
    let (new_low, new_high) = if grabs_low { (value.min(high), high) } else { (low, value.max(low)) };
    changed(new_low, new_high)
}

/// What a right-click does where it lands along a range slider, as a
/// share of its width (`SliderY_MouseRightButtonUp`): exactly a fifth,
/// half or four fifths along does nothing.
#[derive(Clone, Copy, PartialEq, Debug)]
enum RangeClick {
    TypeLow,
    NudgeLow,
    NudgeHigh,
    TypeHigh,
}

fn range_click_target(along: f64) -> Option<RangeClick> {
    if along > 0.0 && along < 0.2 {
        Some(RangeClick::TypeLow)
    } else if along > 0.2 && along < 0.5 {
        Some(RangeClick::NudgeLow)
    } else if along > 0.5 && along < 0.8 {
        Some(RangeClick::NudgeHigh)
    } else if along > 0.8 && along < 1.0 {
        Some(RangeClick::TypeHigh)
    } else {
        None
    }
}

/// A range slider's value as it shows (`FormattedString`): two decimals,
/// four when `precise`, whole numbers for integers.
fn range_text(value: f64, integer: bool, precise: bool) -> String {
    if integer {
        format!("{}", value.round_ties_even() as i64)
    } else if precise {
        format!("{value:.4}")
    } else {
        format!("{value:.2}")
    }
}

/// `SetValue` (each end held to the range), then `UpdateUIElements`, which
/// keeps the high thumb at or above the low one.
fn held_range(low: f32, high: f32, spec: &RangeSpec) -> (f64, f64) {
    let min = spec.min as f32;
    let max = spec.max as f32;
    let low = low.clamp(min, max);
    let high = high.clamp(min, max).max(low);
    (low as f64, high as f64)
}

/// `SliderY.EditablePopup_EditSuccess`: the typed formula's value for
/// one end; nothing when it does not work out (or comes to NaN).
fn typed_range(ctx: &egui::Context, text: &str, low: f64, high: f64, spec: &RangeSpec, end: RangeEnd) -> Option<(f64, f64)> {
    let number = crate::ui::formulas::evaluate(ctx, text)?;
    if number.is_nan() {
        return None;
    }
    let held = match end {
        RangeEnd::Low => held_range(number, high as f32, spec),
        RangeEnd::High => held_range(low as f32, number, spec),
    };
    Some(held)
}

/// `SliderY.NudgePopup_CommandClicked`: Down 10 and Up 10 step a
/// hundredth of the range, Down 1 and Up 1 a thousandth; Default puts the
/// whole range back first.
fn range_nudged(nudge: Nudge, low: f64, high: f64, spec: &RangeSpec, end: RangeEnd) -> (f64, f64) {
    let min = spec.min as f32;
    let max = spec.max as f32;
    let step = (max - min) / 100.0;
    let mut low = low as f32;
    let mut high = high as f32;
    let default = spec.default.map(|(default_low, default_high)| (default_low as f32, default_high as f32));
    if nudge == Nudge::Default
        && let Some((default_low, default_high)) = default
    {
        (low, high) = (default_low.clamp(min, max), default_high.clamp(min, max));
    }
    let current = if end == RangeEnd::Low { low } else { high };
    let default_end = default.map(|(default_low, default_high)| if end == RangeEnd::Low { default_low } else { default_high });
    let stepped = match nudge {
        Nudge::Half => current * 0.5,
        Nudge::Double => current * 2.0,
        Nudge::Min => min,
        Nudge::Down10 => current - step,
        Nudge::Down1 => current - step / 10.0,
        Nudge::Default => default_end.unwrap_or(current),
        Nudge::Up1 => current + step / 10.0,
        Nudge::Up10 => current + step,
        Nudge::Max => max,
    };
    match end {
        RangeEnd::Low => held_range(stepped, high, spec),
        RangeEnd::High => held_range(low, stepped, spec),
    }
}

/// `CenterDrag`: the range moved by `delta`, keeping its width and
/// stopping at either end.
fn center_drag(low: f64, high: f64, delta: f64, spec: &RangeSpec) -> (f64, f64) {
    let width = high - low;
    let new_low = low + delta;
    let new_high = new_low + width;
    if new_low < spec.min {
        (spec.min, spec.min + width)
    } else if new_high > spec.max {
        (spec.max - width, spec.max)
    } else {
        (new_low, new_high)
    }
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
    // `PART_Textbox_MouseRightButtonUp`: an `EditablePopup` 4 above the
    // button, 32 wider (64 at least), its text taken as a formula.
    let typing_id = id.with("seed-typing");
    if button.secondary_clicked() && enabled {
        slider_popups::open_typing(ui.ctx(), typing_id, seed.to_string());
    }
    let button = button.on_hover_text("New seed");
    let popup_scale = slider_popups::popup_scale(ui.ctx());
    let typing_look = TypingLook {
        top_left: Pos2::new(button.rect.left(), button.rect.top() - 4.0 * popup_scale),
        width: BoxWidth::Fixed(((button.rect.width() + 32.0) * popup_scale).max(64.0 * popup_scale)),
        font: slider_popups::value_font(),
    };
    if let Some(text) = slider_popups::typing_box(ui, typing_id, &typing_look) {
        edit = Edit::SeedText(text);
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
    let column = if shrunk == Shrunk::None { theme::sizes(ui.ctx()).header_column } else { 100.0 };
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
    // A row: the text's line, `OptionsX.Item.Padding` and
    // `OptionsX.Item.Margin` above and below; at least
    // `OptionsX.Item.MinHeight` in all.
    let sizes = theme::sizes(ui.ctx());
    let row_height = OPTION_LINE + 2.0 * sizes.option_padding[1] + 2.0 * sizes.option_margin;
    let list_height = (rows as f32 * row_height + 4.0).max(sizes.options_min_height);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, list_height), Sense::hover());
    if style == OptionsStyle::Labelled {
        let header_height = if shrunk == Shrunk::Lot { sizes.options_min_height } else { rect.height() };
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
    // An item: the text's line, `OptionsX.Item.Padding` and
    // `OptionsX.Item.Margin` above and below.
    let sizes = theme::sizes(ui.ctx());
    let row_height = font_size * 1.36 + 2.0 * sizes.option_padding[1] + 2.0 * sizes.option_margin;
    let height = rows as f32 * row_height + 4.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    option_cells(ui, id, rect, entries, selected, columns, font_size)
}

/// Paints the box and its cells into `list` (border 1, grid margin 2, 1,
/// item margin `OptionsX.Item.Margin`) and handles clicks.
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
    let sizes = theme::sizes(ui.ctx());
    let mut chosen = None;
    for (index, entry) in entries.iter().enumerate() {
        let column = index % columns;
        let row = index / columns;
        let cell_origin = Pos2::new(grid.left() + column as f32 * cell_width, grid.top() + row as f32 * cell_height);
        let cell = Rect::from_min_size(cell_origin, Vec2::new(cell_width, cell_height));
        let item = cell.shrink(sizes.option_margin);
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
        let galley = fitted_line(ui, entry, theme::interface(font_size), shown_color, item.width() - 2.0 * sizes.option_padding[0]);
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
        Shrunk::None => theme::sizes(ui.ctx()).header_column,
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

/// `FileBrowser`: the name (90 column), the path in blue on a dark field,
/// and a folder button. Clicking the path types a new one
/// (`PART_FileTextBox`): Enter, Tab or clicking away takes it unless it
/// is empty, Escape puts the old one back. Files dragged over the control
/// light its border (`IsDragOver`); dropped on it, the first one's path is
/// taken, whatever the file (`OnDrop`).
fn file_browser(ui: &mut Ui, id: Id, label: &str, path: &str) -> Edit {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 32.0), Sense::hover());
    let header = Rect::from_min_size(rect.min, Vec2::new(90.0, rect.height()));
    header_label(ui, header, label, theme::LABEL_DEFAULT);
    let field = Rect::from_min_max(Pos2::new(header.right(), rect.top() + 3.0), Pos2::new(rect.right() - 30.0, rect.bottom() - 3.0));
    let under_pointer = ui.rect_contains_pointer(rect);
    let dragged_over = under_pointer && ui.input(|input| !input.raw.hovered_files.is_empty());
    let border = if dragged_over { theme::ACCENT_PRIMARY } else { theme::TEXTBOX_FILE_BORDER };
    ui.painter().rect_filled(field, 3.0, theme::TEXTBOX_BACKGROUND);
    ui.painter().rect_stroke(field, 3.0, Stroke::new(1.0, border), StrokeKind::Inside);
    let dropped = ui.input(|input| input.raw.dropped_files.first().map(|file| file.path().to_string_lossy().to_string()));
    let dropped = dropped.filter(|_| under_pointer);

    // `PART_FileTextBlock` (margins 9, 0, 2, 0; one line, centred): a
    // click starts typing.
    let font = theme::interface(12.0);
    let line_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
    let text_rect = Rect::from_min_max(Pos2::new(field.left() + 9.0, field.center().y - line_height / 2.0), Pos2::new(field.right() - 2.0, field.center().y + line_height / 2.0));
    let typing_id = id.with("typed-path");
    let edit_id = id.with("path-box");
    // A drop ends any typing.
    if dropped.is_some() {
        ui.data_mut(|data| data.remove::<String>(typing_id));
        ui.memory_mut(|memory| memory.surrender_focus(edit_id));
    }
    let typed: Option<String> = ui.data(|data| data.get_temp(typing_id));
    let mut edit = Edit::None;
    match typed {
        Some(text) => edit = path_box(ui, edit_id, typing_id, field, text, path),
        None => {
            // The box appears next frame: one made under this click would
            // count it as a click elsewhere and give the focus up.
            let text_response = ui.interact(text_rect, id.with("path-text"), Sense::click());
            if text_response.clicked() {
                ui.data_mut(|data| data.insert_temp(typing_id, path.to_string()));
                ui.memory_mut(|memory| memory.request_focus(edit_id));
            }
            let shown = truncate_path(ui, path, &font, text_rect.width());
            let galley = fitted_line(ui, &shown, font.clone(), theme::TEXTBOX_FILE_FOREGROUND, text_rect.width());
            ui.painter().galley(Pos2::new(text_rect.left(), field.center().y - galley.size().y / 2.0), galley, theme::TEXTBOX_FILE_FOREGROUND);
            if !path.is_empty() {
                text_response.on_hover_text(path);
            }
        }
    }

    let button_rect = Rect::from_min_max(Pos2::new(rect.right() - 30.0, rect.top()), rect.max);
    let mut button_ui = ui.new_child(egui::UiBuilder::new().max_rect(button_rect));
    let browse = crate::ui::widgets::icon_button(&mut button_ui, "folder", "Browse...", None);
    if let Some(dropped) = dropped {
        return Edit::Set(Value::String(dropped));
    }
    if browse.clicked() {
        return Edit::Browse;
    }
    edit
}

/// The file field's text box (`TextBox-File`: no fill, a 1px border blue
/// while hovered or focused, corner 3, padding 3, 2; blue text and a
/// white caret), as wide as the field and centred down it. Leaving it
/// sets the typed path (`HandleTextBox`), unless it was empty or Escape
/// left it (`ResetTextBox`).
fn path_box(ui: &mut Ui, edit_id: Id, typing_id: Id, field: Rect, mut text: String, path: &str) -> Edit {
    let font = theme::interface(12.0);
    let line_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
    let box_height = line_height + 2.0 * 2.0 + 2.0 * 1.0;
    let box_rect = Rect::from_center_size(field.center(), Vec2::new(field.width(), box_height));
    let mut box_ui = ui.new_child(egui::UiBuilder::new().max_rect(box_rect));
    box_ui.visuals_mut().text_cursor.stroke.color = Color32::WHITE;
    let mut text_edit = egui::TextEdit::singleline(&mut text);
    text_edit = text_edit.id(edit_id);
    let mut frame = egui::Frame::new();
    frame.inner_margin = egui::Margin::symmetric(4, 3);
    text_edit = text_edit.frame(frame);
    text_edit = text_edit.font(font);
    text_edit = text_edit.text_color(theme::TEXTBOX_FILE_FOREGROUND);
    text_edit = text_edit.desired_width(box_rect.width() - 8.0);
    let response = box_ui.put(box_rect, text_edit);
    if response.has_focus() || response.hovered() {
        ui.painter().rect_stroke(box_rect, 3.0, Stroke::new(1.0, theme::TEXTBOX_FOCUSED_BORDER), StrokeKind::Inside);
    }

    if response.has_focus() {
        ui.data_mut(|data| data.insert_temp(typing_id, text));
        return Edit::None;
    }
    // Focus went elsewhere, or the field was hidden while typing.
    ui.data_mut(|data| data.remove::<String>(typing_id));
    let escaped = ui.input(|input| input.key_pressed(egui::Key::Escape));
    let taken = response.lost_focus() && !escaped && !text.is_empty() && text != path;
    if taken { Edit::Set(Value::String(text)) } else { Edit::None }
}

/// `TruncatePath`: the path when it fits `width`; otherwise its root,
/// "..." and the longest end of the rest that fits beside them (all of
/// it when not even nothing fits).
fn truncate_path(ui: &Ui, path: &str, font: &egui::FontId, width: f32) -> String {
    let text_width = |text: &str| ui.fonts_mut(|fonts| fonts.layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE).size().x);
    if text_width(path) < width {
        return path.to_string();
    }
    let root = path_root(path);
    let room = width - text_width(&format!("{root}..."));
    let rest = &path[root.len()..];
    let mut kept = rest;
    let starts: Vec<usize> = rest.char_indices().map(|(index, _)| index).collect();
    let mut candidates = starts.into_iter().chain(std::iter::once(rest.len()));
    let fitting = candidates.find(|start| text_width(&rest[*start..]) <= room);
    if let Some(start) = fitting {
        kept = &rest[start..];
    }
    format!("{root}...{kept}")
}

/// `Path.GetPathRoot`: the drive and separator a path starts with, if any.
fn path_root(path: &str) -> &str {
    let mut root_length = 0;
    for component in std::path::Path::new(path).components() {
        match component {
            std::path::Component::Prefix(prefix) => root_length += prefix.as_os_str().len(),
            std::path::Component::RootDir => {
                root_length += 1;
                break;
            }
            _ => break,
        }
    }
    path.get(..root_length).unwrap_or_default()
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

/// `TextEditor` (margin 20, 2, 4, 2): the name in a 90 header column and
/// the value in a text box that only shows it (`TextBox`: #101010 with a
/// #363636 line, padding 4, corners 3). A click opens an `EditablePopup`
/// over the value (monospace 16, as wide as the value, 96 at least),
/// which closes as [`crate::ui::widgets::popup_outcome`] says.
fn text_box(ui: &mut Ui, id: Id, label: &str, current: &str) -> Edit {
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::hover());
    let control = Rect::from_min_max(row.min + Vec2::new(20.0, 2.0), row.max - Vec2::new(4.0, 2.0));
    let response = ui.interact(control, id.with("text-editor"), Sense::click());
    let header = Rect::from_min_size(control.min, Vec2::new(90.0, control.height()));
    header_label(ui, header, label, theme::LABEL_DEFAULT);
    let field = Rect::from_min_max(Pos2::new(header.right(), control.top()), control.max);
    let opacity = if ui.is_enabled() { 1.0 } else { 0.5 };
    ui.painter().rect_filled(field, 3.0, theme::TEXTBOX_BACKGROUND.gamma_multiply(opacity));
    ui.painter().rect_stroke(field, 3.0, Stroke::new(1.0, theme::TEXTBOX_BORDER.gamma_multiply(opacity)), StrokeKind::Inside);
    let text_color = theme::LABEL_DEFAULT.gamma_multiply(opacity);
    let galley = ui.painter().layout_no_wrap(current.to_string(), theme::interface(12.0), text_color);
    let text_area = field.shrink(1.0 + 4.0);
    let text_position = Pos2::new(text_area.left(), field.center().y - galley.size().y / 2.0);
    ui.painter().with_clip_rect(text_area).galley(text_position, galley, text_color);

    let typing_id = id.with("text-typing");
    if response.clicked() && ui.is_enabled() {
        slider_popups::open_typing(ui.ctx(), typing_id, current.to_string());
    }
    // 90 in from the control, as wide as it less 90 (96 at least), in
    // the popup's own units.
    let popup_scale = slider_popups::popup_scale(ui.ctx());
    let typing_look = TypingLook {
        top_left: Pos2::new(control.left() + 90.0 * popup_scale, control.top()),
        width: BoxWidth::Fixed(((control.width() - 90.0) * popup_scale).max(96.0 * popup_scale)),
        font: slider_popups::value_font(),
    };
    match slider_popups::typing_box(ui, typing_id, &typing_look) {
        Some(text) if text != current => Edit::Set(json!(text)),
        _ => Edit::None,
    }
}

/// A read-only value beside its name (90 header column).
fn caption(ui: &mut Ui, label: &str, text: &str) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, theme::sizes(ui.ctx()).slider_height), Sense::hover());
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

/// `SatelliteMapListBox`: the palettes of the node's library as 72 by 28
/// strips filling the width (`VirtualizingWrapPanel`, uniform), or one
/// full-width strip a row (`SatelliteMapListBoxStacked`, a `StackPanel`,
/// with Show Satellite Maps List as a Single Stack); the chosen one framed
/// in yellow with its number; at most 360 tall.
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
            let stacked = theme::settings(ui.ctx()).stacked_palettes;
            let columns = if stacked { 1 } else { ((width / 72.0).floor() as usize).max(1) };
            let item_width = width / columns as f32;
            // Stacked, a strip is as tall as its item's template: the
            // hidden number (monospace 10, padding 2) in the border's
            // padding of 4 and its line.
            let item_height = if stacked { 27.0 } else { 28.0 };
            for (row, chunk) in palettes.chunks(columns).enumerate() {
                ui.horizontal(|ui| {
                    for (column, palette) in chunk.iter().enumerate() {
                        let index = (row * columns + column) as i64;
                        let (rect, response) = ui.allocate_exact_size(Vec2::new(item_width, item_height), Sense::click());
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
                let spec = RangeSpec::linear(min, max);
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
                let column = theme::sizes(ui.ctx()).header_column;
                let picked = dropdown_box(ui, variable_id, Some(label), column, 26.0, &shown, &entries, &current.to_string(), theme::interface(12.0));
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
    use nerthus_engine::Catalogue;

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

    const FILE_ROW: Rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 32.0));

    /// One frame of a file field holding `path`, with these events.
    fn file_frame(context: &egui::Context, path: &str, events: Vec<egui::Event>) -> Edit {
        file_frame_dropping(context, path, events, Vec::new())
    }

    /// A file dropped on the window.
    #[derive(Debug)]
    struct Dropped(std::path::PathBuf);

    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &std::path::Path {
            &self.0
        }

        fn bytes(&self) -> Result<Vec<u8>, String> {
            Ok(Vec::new())
        }
    }

    /// One frame of a file field in which these files drop.
    fn file_frame_dropping(context: &egui::Context, path: &str, events: Vec<egui::Event>, dropped: Vec<&str>) -> Edit {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0));
        let dropped_files: Vec<egui::DroppedFileHandle> = dropped.into_iter().map(|file| std::sync::Arc::new(Dropped(file.into())) as egui::DroppedFileHandle).collect();
        let input = egui::RawInput { screen_rect: Some(screen), events, dropped_files, ..Default::default() };
        let mut edit = Edit::None;
        let output = context.run_ui(input, |ui| {
            let mut row = ui.new_child(egui::UiBuilder::new().max_rect(FILE_ROW));
            edit = file_browser(&mut row, Id::new("file-test"), "File Name", path);
        });
        output.drop_without_applying_deltas();
        edit
    }

    /// A context with the editor's fonts, which take effect a frame after
    /// they are set.
    fn themed_context() -> egui::Context {
        let context = egui::Context::default();
        theme::apply(&context);
        let input = egui::RawInput::default();
        let output = context.run_ui(input, |_| {});
        output.drop_without_applying_deltas();
        context
    }

    /// Clicks the field's path, then sends `keys` one frame each.
    fn type_into_file_field(path: &str, keys: Vec<egui::Event>) -> Vec<Edit> {
        let context = themed_context();
        let on_text = Pos2::new(200.0, 16.0);
        let button = |pressed: bool| egui::Event::PointerButton { pos: on_text, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        file_frame(&context, path, vec![egui::Event::PointerMoved(on_text)]);
        file_frame(&context, path, vec![button(true)]);
        file_frame(&context, path, vec![button(false)]);
        keys.into_iter().map(|key| file_frame(&context, path, vec![key])).collect()
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
    }

    #[test]
    fn a_typed_path_is_taken_on_enter_and_escape_keeps_the_old_one() {
        let typed = egui::Event::Text("/maps/height.psd".to_string());
        let edits = type_into_file_field("", vec![typed.clone(), key(egui::Key::Enter)]);
        assert!(matches!(&edits[0], Edit::None));
        assert!(matches!(&edits[1], Edit::Set(Value::String(path)) if path == "/maps/height.psd"), "{:?}", std::mem::discriminant(&edits[1]));

        let edits = type_into_file_field("", vec![typed, key(egui::Key::Escape)]);
        assert!(edits.iter().all(|edit| matches!(edit, Edit::None)));
        // Nothing typed: the old path stays.
        let edits = type_into_file_field("", vec![key(egui::Key::Enter)]);
        assert!(edits.iter().all(|edit| matches!(edit, Edit::None)));
    }

    #[test]
    fn files_dropped_on_a_file_field_set_its_path() {
        let context = themed_context();
        let on_field = Pos2::new(200.0, 16.0);
        file_frame(&context, "/old.png", vec![egui::Event::PointerMoved(on_field)]);
        let edit = file_frame_dropping(&context, "/old.png", Vec::new(), vec!["/maps/height.psd", "/maps/other.png"]);
        assert!(matches!(&edit, Edit::Set(Value::String(path)) if path == "/maps/height.psd"));
        // Dropped below the field: not its.
        let below = Pos2::new(200.0, 120.0);
        file_frame(&context, "/old.png", vec![egui::Event::PointerMoved(below)]);
        let edit = file_frame_dropping(&context, "/old.png", Vec::new(), vec!["/maps/height.psd"]);
        assert!(matches!(edit, Edit::None));
    }

    #[test]
    fn long_paths_keep_their_root_and_end() {
        let context = themed_context();
        let input = egui::RawInput::default();
        let mut shown = Vec::new();
        let output = context.run_ui(input, |ui| {
            let font = theme::interface(12.0);
            let path = "/Users/someone/Documents/Terrains/Heightfields/Alps/height.png";
            shown.push(truncate_path(ui, path, &font, 2000.0));
            shown.push(truncate_path(ui, path, &font, 120.0));
            shown.push(truncate_path(ui, "relative/folder/height.png", &font, 60.0));
            shown.push(truncate_path(ui, path, &font, 1.0));
        });
        output.drop_without_applying_deltas();
        assert_eq!(shown[0], "/Users/someone/Documents/Terrains/Heightfields/Alps/height.png");
        assert!(shown[1].starts_with("/...") && shown[1].ends_with("/Alps/height.png") || shown[1].ends_with("height.png"), "{}", shown[1]);
        assert!(shown[1].len() < shown[0].len());
        let end = shown[2].strip_prefix("...").unwrap_or_default();
        assert!(!end.is_empty() && "relative/folder/height.png".ends_with(end), "{}", shown[2]);
        // Not even "..." fits: the whole rest follows it.
        assert_eq!(shown[3], "/.../Users/someone/Documents/Terrains/Heightfields/Alps/height.png".replacen("/.../", "/...", 1));
    }

    /// Where the sliders under test sit: 300 wide and 26 high, 40 down so
    /// their popups have room above.
    const SLIDER_ROW: Rect = Rect::from_min_max(Pos2::new(0.0, 40.0), Pos2::new(300.0, 66.0));

    fn slider_id() -> Id {
        Id::new("slider-test")
    }

    fn press(pos: Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE }
    }

    /// One frame with these events, these modifiers held.
    fn frame_input(events: Vec<egui::Event>, modifiers: egui::Modifiers) -> egui::RawInput {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0));
        let mut all_events = vec![egui::Event::ModifiersChanged(modifiers)];
        all_events.extend(events);
        egui::RawInput { screen_rect: Some(screen), events: all_events, ..Default::default() }
    }

    /// One frame of a slider holding `value`; what it changed to.
    fn slider_frame(context: &egui::Context, value: f64, spec: &SliderSpec, events: Vec<egui::Event>) -> Option<f64> {
        let mut changed = None;
        let output = context.run_ui(frame_input(events, egui::Modifiers::NONE), |ui| {
            let mut row = ui.new_child(egui::UiBuilder::new().max_rect(SLIDER_ROW));
            changed = slider(&mut row, slider_id(), "Scale", value, spec, Color32::WHITE);
        });
        output.drop_without_applying_deltas();
        changed
    }

    /// A right-click at `at`, a frame per event; what each frame changed.
    fn right_click(context: &egui::Context, value: f64, spec: &SliderSpec, at: Pos2) -> Vec<Option<f64>> {
        let events = [egui::Event::PointerMoved(at), press(at, egui::PointerButton::Secondary, true), press(at, egui::PointerButton::Secondary, false)];
        events.into_iter().map(|event| slider_frame(context, value, spec, vec![event])).collect()
    }

    /// A left click at `at`, a frame per event.
    fn left_click(context: &egui::Context, value: f64, spec: &SliderSpec, at: Pos2) -> Vec<Option<f64>> {
        let events = [egui::Event::PointerMoved(at), press(at, egui::PointerButton::Primary, true), press(at, egui::PointerButton::Primary, false)];
        events.into_iter().map(|event| slider_frame(context, value, spec, vec![event])).collect()
    }

    #[test]
    fn a_value_typed_on_the_number_is_worked_out_as_a_formula() {
        let context = themed_context();
        let mut editor = crate::editor::Editor::new(crate::document::Document::new());
        editor.preview_resolution = 512;
        crate::ui::formulas::publish(&context, &editor);
        let spec = SliderSpec::linear(0.0, 1.0);
        // On the value: the typing box, starting from four decimals.
        let changes = right_click(&context, 0.25, &spec, Pos2::new(292.0, 53.0));
        assert!(changes.iter().all(Option::is_none));
        let typing_id = slider_id().with("typing");
        assert_eq!(slider_popups::typing_text(&context, typing_id).as_deref(), Some("0.2500"));
        assert_eq!(slider_frame(&context, 0.25, &spec, vec![egui::Event::Text("PX * 256".to_string())]), None);
        assert_eq!(slider_frame(&context, 0.25, &spec, vec![key(egui::Key::Enter)]), Some(0.5));
        assert!(slider_popups::typing_text(&context, typing_id).is_none());

        // Past the range: held to it. Nonsense: nothing.
        right_click(&context, 0.25, &spec, Pos2::new(292.0, 53.0));
        slider_frame(&context, 0.25, &spec, vec![egui::Event::Text("3 * 2".to_string())]);
        assert_eq!(slider_frame(&context, 0.25, &spec, vec![key(egui::Key::Enter)]), Some(1.0));
        right_click(&context, 0.25, &spec, Pos2::new(292.0, 53.0));
        slider_frame(&context, 0.25, &spec, vec![egui::Event::Text("two".to_string())]);
        assert_eq!(slider_frame(&context, 0.25, &spec, vec![key(egui::Key::Enter)]), None);
        // Escape leaves the value alone.
        right_click(&context, 0.25, &spec, Pos2::new(292.0, 53.0));
        slider_frame(&context, 0.25, &spec, vec![egui::Event::Text("0.75".to_string())]);
        assert_eq!(slider_frame(&context, 0.25, &spec, vec![key(egui::Key::Escape)]), None);
        assert!(slider_popups::typing_text(&context, typing_id).is_none());
    }

    #[test]
    fn a_right_click_off_the_number_opens_the_nudge_bar() {
        let context = themed_context();
        let spec = SliderSpec::linear(0.0, 1.0);
        let nudge_id = slider_id().with("nudge");
        // A right press does not move the value.
        let changes = right_click(&context, 0.25, &spec, Pos2::new(100.0, 53.0));
        assert!(changes.iter().all(Option::is_none), "{changes:?}");
        assert_eq!(slider_popups::nudge_text(&context, nudge_id).as_deref(), Some("0.2500"));
        // Up 10, the eighth button: a tenth of the range, and the box
        // follows.
        let bar_left = SLIDER_ROW.left() + 20.0 + 3.5 - 2.0;
        let up_ten = Pos2::new(bar_left + 7.0 * 28.0 + 14.0, SLIDER_ROW.top() + 1.0 - 2.0 + 13.0);
        let changes = left_click(&context, 0.25, &spec, up_ten);
        let changed = changes.into_iter().flatten().next().expect("Up 10 changes the value");
        assert!((changed - 0.35).abs() < 1e-6, "{changed}");
        assert_eq!(slider_popups::nudge_text(&context, nudge_id).as_deref(), Some("0.3500"));
        // The box takes a formula with Enter.
        let field = Pos2::new(bar_left + 9.0 * 28.0 + 6.0 + 20.0, up_ten.y);
        left_click(&context, 0.35, &spec, field);
        slider_frame(&context, 0.35, &spec, vec![egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND }]);
        slider_frame(&context, 0.35, &spec, vec![egui::Event::Text("1/8".to_string())]);
        assert_eq!(slider_frame(&context, 0.35, &spec, vec![key(egui::Key::Enter)]), Some(0.125));
        assert!(slider_popups::nudge_text(&context, nudge_id).is_none());
        // Leaving it closes it.
        right_click(&context, 0.25, &spec, Pos2::new(100.0, 53.0));
        slider_frame(&context, 0.25, &spec, vec![egui::Event::PointerMoved(Pos2::new(120.0, 50.0))]);
        assert!(slider_popups::nudge_text(&context, nudge_id).is_some());
        slider_frame(&context, 0.25, &spec, vec![egui::Event::PointerMoved(Pos2::new(120.0, 150.0))]);
        assert!(slider_popups::nudge_text(&context, nudge_id).is_none());
    }

    #[test]
    fn a_popup_whose_slider_went_away_is_gone() {
        let context = themed_context();
        let spec = SliderSpec::linear(0.0, 1.0);
        let typing_id = slider_id().with("typing");
        right_click(&context, 0.25, &spec, Pos2::new(292.0, 53.0));
        assert!(slider_popups::typing_text(&context, typing_id).is_some());
        // Two passes without the slider (another node shown).
        for _ in 0..2 {
            let output = context.run_ui(frame_input(Vec::new(), egui::Modifiers::NONE), |_| {});
            output.drop_without_applying_deltas();
        }
        slider_frame(&context, 0.25, &spec, Vec::new());
        assert!(slider_popups::typing_text(&context, typing_id).is_none());
    }

    #[test]
    fn nudges_step_in_single_precision() {
        let mut spec = SliderSpec::linear(0.0, 1.0);
        spec.default = Some(0.5);
        assert_eq!(nudged(Nudge::Up1, 0.3, &spec), (0.3f32 + 0.1f32 / 10.0) as f64);
        assert_eq!(nudged(Nudge::Default, 0.3, &spec), 0.5);
        spec.default = None;
        assert_eq!(nudged(Nudge::Default, 0.3, &spec), 0.3f32 as f64);
    }

    #[test]
    fn terrain_marks_fall_between_the_ends() {
        let mut spec = SliderSpec::linear(0.0, 1.0);
        spec.ticks = Ticks::TerrainMarks;
        assert!(spec.tick_positions().is_empty());
        spec.terrain_range = Some((0.2, 1.0));
        assert_eq!(spec.tick_positions(), [0.2]);
        spec.terrain_range = Some((0.1, 0.75));
        assert_eq!(spec.tick_positions(), [0.1, 0.75]);
        let snow = Catalogue::get().node("Snow").and_then(|node| node.parameter("SnowLine")).cloned().unwrap();
        assert_eq!(SliderSpec::of(&snow, None, false).ticks, Ticks::TerrainMarks);
    }

    /// One frame of a range slider holding `low..high`.
    fn range_frame(context: &egui::Context, range: (f64, f64), spec: &RangeSpec, events: Vec<egui::Event>, modifiers: egui::Modifiers) -> Option<(f64, f64)> {
        let mut changed = None;
        let output = context.run_ui(frame_input(events, modifiers), |ui| {
            let mut row = ui.new_child(egui::UiBuilder::new().max_rect(SLIDER_ROW));
            changed = range_slider(&mut row, slider_id(), "Range", range.0, range.1, spec, Color32::WHITE);
        });
        output.drop_without_applying_deltas();
        changed
    }

    fn range_right_click(context: &egui::Context, range: (f64, f64), spec: &RangeSpec, x: f32) {
        let at = Pos2::new(x, 53.0);
        let events = [egui::Event::PointerMoved(at), press(at, egui::PointerButton::Secondary, true), press(at, egui::PointerButton::Secondary, false)];
        for event in events {
            assert_eq!(range_frame(context, range, spec, vec![event], egui::Modifiers::NONE), None);
        }
    }

    #[test]
    fn a_range_types_either_end_by_where_it_is_right_clicked() {
        let context = themed_context();
        let spec = RangeSpec::linear(0.0, 1.0);
        let range = (0.2, 0.6);
        // The left fifth types the low end ("##0.0##"); past the high end,
        // the high end follows.
        range_right_click(&context, range, &spec, 30.0);
        assert_eq!(slider_popups::typing_text(&context, slider_id().with("typing-low")).as_deref(), Some("0.2"));
        range_frame(&context, range, &spec, vec![egui::Event::Text("0.8".to_string())], egui::Modifiers::NONE);
        let typed = range_frame(&context, range, &spec, vec![key(egui::Key::Enter)], egui::Modifiers::NONE);
        assert_eq!(typed.map(|(low, high)| (low as f32, high as f32)), Some((0.8, 0.8)));
        // The right fifth types the high end; below the low end it stops
        // there.
        range_right_click(&context, range, &spec, 290.0);
        assert_eq!(slider_popups::typing_text(&context, slider_id().with("typing-high")).as_deref(), Some("0.6"));
        range_frame(&context, range, &spec, vec![egui::Event::Text("0.1".to_string())], egui::Modifiers::NONE);
        let typed = range_frame(&context, range, &spec, vec![key(egui::Key::Enter)], egui::Modifiers::NONE);
        assert_eq!(typed.map(|(low, high)| (low as f32, high as f32)), Some((0.2, 0.2)));
        // Between: the nudge bar for the nearer end.
        range_right_click(&context, range, &spec, 100.0);
        assert_eq!(slider_popups::nudge_open(&context, slider_id().with("nudge")), Some(Some(RangeEnd::Low)));
        assert_eq!(slider_popups::nudge_text(&context, slider_id().with("nudge")).as_deref(), Some("0.2000"));
    }

    #[test]
    fn right_click_targets_split_the_range_in_fifths() {
        assert_eq!(range_click_target(0.1), Some(RangeClick::TypeLow));
        assert_eq!(range_click_target(0.2), None);
        assert_eq!(range_click_target(0.3), Some(RangeClick::NudgeLow));
        assert_eq!(range_click_target(0.5), None);
        assert_eq!(range_click_target(0.7), Some(RangeClick::NudgeHigh));
        assert_eq!(range_click_target(0.8), None);
        assert_eq!(range_click_target(0.9), Some(RangeClick::TypeHigh));
        assert_eq!(range_click_target(0.0), None);
        assert_eq!(range_click_target(1.0), None);
    }

    #[test]
    fn range_nudges_step_a_hundredth_and_keep_the_order() {
        let mut spec = RangeSpec::linear(0.0, 1.0);
        spec.default = Some((0.1, 0.9));
        let (low, high) = range_nudged(Nudge::Up10, 0.2, 0.6, &spec, RangeEnd::Low);
        assert_eq!((low as f32, high as f32), (0.2f32 + 0.01f32, 0.6));
        let (low, high) = range_nudged(Nudge::Max, 0.2, 0.6, &spec, RangeEnd::Low);
        assert_eq!((low, high), (1.0, 1.0));
        let (low, high) = range_nudged(Nudge::Min, 0.2, 0.6, &spec, RangeEnd::High);
        assert_eq!((low as f32, high as f32), (0.2, 0.2));
        // Default puts the whole range back.
        let (low, high) = range_nudged(Nudge::Default, 0.3, 0.4, &spec, RangeEnd::High);
        assert_eq!((low as f32, high as f32), (0.1, 0.9));
    }

    #[test]
    fn shift_drags_move_the_whole_range() {
        let context = themed_context();
        let spec = RangeSpec::linear(0.0, 1.0);
        let range = (0.2, 0.6);
        let shift = egui::Modifiers::SHIFT;
        let start = Pos2::new(150.0, 53.0);
        let pressed = egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: shift };
        assert_eq!(range_frame(&context, range, &spec, vec![egui::Event::PointerMoved(start)], shift), None);
        assert_eq!(range_frame(&context, range, &spec, vec![pressed], shift), None);
        // 30 of the control's 300: a tenth of the range.
        let moved = range_frame(&context, range, &spec, vec![egui::Event::PointerMoved(Pos2::new(180.0, 53.0))], shift);
        let (low, high) = moved.expect("the range moves");
        assert!((low - 0.3).abs() < 1e-6 && (high - 0.7).abs() < 1e-6, "{low} {high}");
        // Stopped at the end, keeping its width.
        let (low, high) = center_drag(0.7, 0.9, 0.3, &spec);
        assert!((low - 0.8).abs() < 1e-9 && high == 1.0, "{low} {high}");
        let (low, high) = center_drag(0.1, 0.3, -0.3, &spec);
        assert!(low == 0.0 && (high - 0.2).abs() < 1e-9, "{low} {high}");
    }
}
