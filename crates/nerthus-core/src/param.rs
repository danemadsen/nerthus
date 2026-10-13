//! Node parameters, mirroring the `Parameter` hierarchy.
//!
//! Lookups are keyed by name *and* type because names are reused across
//! types (for example the post-process list has both a boolean and a double
//! called "Influence").

use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParamType {
    Double,
    Integer,
    Color,
    String,
    Choice,
    Boolean,
    Guid,
    Object,
    Divider,
    Stroke,
    Range,
    Curve,
}

impl ParamType {
    /// The `xsi:type` used in the project XML.
    pub fn xml_name(self) -> &'static str {
        match self {
            ParamType::Double => "PDouble",
            ParamType::Integer => "PInt",
            ParamType::Color => "PColor",
            ParamType::String => "PString",
            ParamType::Choice => "PChoice",
            ParamType::Boolean => "PBoolean",
            ParamType::Guid => "PGuid",
            ParamType::Object => "PObject",
            ParamType::Divider => "PTitle",
            ParamType::Stroke => "PStrokes",
            ParamType::Range => "PRange",
            ParamType::Curve => "PCurve",
        }
    }

    pub fn from_xml_name(name: &str) -> Option<Self> {
        let value = match name {
            "PDouble" => ParamType::Double,
            "PInt" => ParamType::Integer,
            "PColor" => ParamType::Color,
            "PString" => ParamType::String,
            "PChoice" => ParamType::Choice,
            "PBoolean" => ParamType::Boolean,
            "PGuid" => ParamType::Guid,
            "PTitle" => ParamType::Divider,
            "PStrokes" => ParamType::Stroke,
            "PRange" => ParamType::Range,
            "PCurve" => ParamType::Curve,
            _ => return None,
        };
        Some(value)
    }
}

/// Linear (scRGB) colour as stored by `PColor.Underlying`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl Rgba {
    pub const BLACK: Rgba = Rgba::new(0.0, 0.0, 0.0, 1.0);
    pub const WHITE: Rgba = Rgba::new(1.0, 1.0, 1.0, 1.0);

    pub const fn new(r: f64, g: f64, b: f64, a: f64) -> Self {
        Self { r, g, b, a }
    }

    /// Builds from 8-bit sRGB components (WPF `Color.FromRgb` then `ScR`).
    pub fn from_srgb8(r: u8, g: u8, b: u8) -> Self {
        Self::new(srgb_to_linear(r as f64 / 255.0), srgb_to_linear(g as f64 / 255.0), srgb_to_linear(b as f64 / 255.0), 1.0)
    }

    /// 8-bit sRGB components (WPF `Color.R/G/B` of an scRGB colour).
    pub fn to_srgb8(self) -> [u8; 3] {
        let encode = |value: f64| (linear_to_srgb(value.clamp(0.0, 1.0)) * 255.0).round() as u8;
        [encode(self.r), encode(self.g), encode(self.b)]
    }
}

pub fn srgb_to_linear(value: f64) -> f64 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

pub fn linear_to_srgb(value: f64) -> f64 {
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurvePoint {
    pub x: f32,
    pub y: f32,
}

impl CurvePoint {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// One painted stroke (used by the Draw node).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Stroke {
    pub points: Vec<StrokePoint>,
    pub size: f64,
    pub hardness: f64,
    pub opacity: f64,
    pub tool: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct StrokePoint {
    pub x: f64,
    pub y: f64,
    pub pressure: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ParamValue {
    Title,
    Bool(bool),
    Int(i32),
    Double(f64),
    Choice(i32),
    Color(Rgba),
    String(String),
    Guid(Uuid),
    Range(f64, f64),
    Curve(Vec<CurvePoint>),
    Strokes(Vec<Stroke>),
}

impl ParamValue {
    pub fn param_type(&self) -> ParamType {
        match self {
            ParamValue::Title => ParamType::Divider,
            ParamValue::Bool(_) => ParamType::Boolean,
            ParamValue::Int(_) => ParamType::Integer,
            ParamValue::Double(_) => ParamType::Double,
            ParamValue::Choice(_) => ParamType::Choice,
            ParamValue::Color(_) => ParamType::Color,
            ParamValue::String(_) => ParamType::String,
            ParamValue::Guid(_) => ParamType::Guid,
            ParamValue::Range(_, _) => ParamType::Range,
            ParamValue::Curve(_) => ParamType::Curve,
            ParamValue::Strokes(_) => ParamType::Stroke,
        }
    }
}

/// How a numeric slider presents its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DisplayType {
    #[default]
    Normal,
    Percentage,
    Integer,
    Degrees,
    Meters,
}

/// What a string parameter edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StringKind {
    #[default]
    Text,
    OpenFile,
    SaveFile,
    Folder,
    Gradient,
    Hidden,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub display_name: Option<String>,
    pub value: ParamValue,
    pub default: ParamValue,
    pub min: f64,
    pub max: f64,
    pub choices: Vec<String>,
    /// Slider accent colour as `#rrggbb`.
    pub brush: Option<String>,
    pub display_type: DisplayType,
    pub string_kind: StringKind,
    pub file_filter: Option<String>,
    pub danger: Option<f64>,
    pub danger_text: Option<String>,
    pub visible: bool,
    pub enabled: bool,
    pub is_open: bool,
    /// Editor hint: leave a gap above this parameter (`StartGroup`).
    pub start_group: bool,
    pub advanced: bool,
    pub ignore_hash: bool,
    pub tooltip: Option<String>,
    /// Serialized `Auto` attribute (automation binding).
    pub auto: Option<String>,
    /// Serialized `Expr` attribute (expression).
    pub expr: Option<String>,
    pub owner: Uuid,
    /// Original XML of values we keep verbatim (e.g. painted strokes).
    pub raw_xml: Option<String>,
}

impl Param {
    fn with_value(name: &str, value: ParamValue) -> Self {
        Self {
            name: name.to_string(),
            display_name: None,
            default: value.clone(),
            value,
            min: 0.0,
            max: 1.0,
            choices: Vec::new(),
            brush: None,
            display_type: DisplayType::Normal,
            string_kind: StringKind::Text,
            file_filter: None,
            danger: None,
            danger_text: None,
            visible: true,
            enabled: true,
            is_open: false,
            start_group: false,
            advanced: false,
            ignore_hash: false,
            tooltip: None,
            auto: None,
            expr: None,
            owner: Uuid::nil(),
            raw_xml: None,
        }
    }

    /// `PTitle(name, isOpen)`
    pub fn title(name: &str) -> Self {
        Self::with_value(name, ParamValue::Title)
    }

    pub fn title_open(name: &str, is_open: bool) -> Self {
        let mut param = Self::title(name);
        param.is_open = is_open;
        param
    }

    /// `PBoolean(name, default)`
    pub fn boolean(name: &str, default: bool) -> Self {
        Self::with_value(name, ParamValue::Bool(default))
    }

    /// `PDouble(name, default)` with the default range [0, 1] shown as a percentage.
    pub fn double(name: &str, default: f64) -> Self {
        Self::double_range(name, default, 0.0, 1.0)
    }

    /// `PDouble(name, default, min, max)`; a [*, 1] range displays as a percentage.
    pub fn double_range(name: &str, default: f64, min: f64, max: f64) -> Self {
        let mut param = Self::with_value(name, ParamValue::Double(default));
        param.min = min;
        param.max = max;
        if max == 1.0 {
            param.display_type = DisplayType::Percentage;
        }
        param
    }

    /// `PDouble(name, default, min, max, neverPercentage: true)`
    pub fn double_plain(name: &str, default: f64, min: f64, max: f64) -> Self {
        let mut param = Self::with_value(name, ParamValue::Double(default));
        param.min = min;
        param.max = max;
        param
    }

    /// `PInt(name, default, min = 0, max = 10)`
    pub fn int(name: &str, default: i32, min: i32, max: i32) -> Self {
        let mut param = Self::with_value(name, ParamValue::Int(default));
        param.min = min as f64;
        param.max = max as f64;
        param.display_type = DisplayType::Integer;
        param
    }

    /// `PChoice(name, default, choices...)`
    pub fn choice(name: &str, default: i32, choices: &[&str]) -> Self {
        let mut param = Self::with_value(name, ParamValue::Choice(default));
        param.choices = choices.iter().map(|choice| choice.to_string()).collect();
        param
    }

    pub fn color(name: &str, default: Rgba) -> Self {
        Self::with_value(name, ParamValue::Color(default))
    }

    pub fn string(name: &str, default: &str) -> Self {
        Self::with_value(name, ParamValue::String(default.to_string()))
    }

    pub fn file(name: &str, filter: &str) -> Self {
        let mut param = Self::string(name, "");
        param.string_kind = StringKind::OpenFile;
        param.file_filter = Some(filter.to_string());
        param
    }

    pub fn guid(name: &str) -> Self {
        Self::with_value(name, ParamValue::Guid(Uuid::nil()))
    }

    /// `PRange(name, a, b, min = 0, max = 1)`
    pub fn range(name: &str, a: f64, b: f64, min: f64, max: f64) -> Self {
        let mut param = Self::with_value(name, ParamValue::Range(a, b));
        param.min = min;
        param.max = max;
        param
    }

    pub fn curve(name: &str, default: Vec<CurvePoint>) -> Self {
        let mut param = Self::with_value(name, ParamValue::Curve(default.clone()));
        param.value = ParamValue::Curve(default);
        param
    }

    pub fn strokes(name: &str) -> Self {
        Self::with_value(name, ParamValue::Strokes(Vec::new()))
    }

    /// Builder: slider accent colour.
    pub fn with_brush(mut self, brush: &str) -> Self {
        self.brush = Some(brush.to_string());
        self
    }

    pub fn with_display_name(mut self, display_name: &str) -> Self {
        self.display_name = Some(display_name.to_string());
        self
    }

    pub fn with_display_type(mut self, display_type: DisplayType) -> Self {
        self.display_type = display_type;
        self
    }

    pub fn with_danger(mut self, danger: f64, text: &str) -> Self {
        self.danger = Some(danger);
        self.danger_text = Some(text.to_string());
        self
    }

    /// Builder: show a choice expanded as buttons (`IsOpen` on non-titles).
    pub fn expanded(mut self) -> Self {
        self.is_open = true;
        self
    }

    /// Builder: editor group break before this parameter.
    pub fn starts_group(mut self) -> Self {
        self.start_group = true;
        self
    }

    pub fn advanced(mut self) -> Self {
        self.advanced = true;
        self
    }

    pub fn hidden(mut self) -> Self {
        self.visible = false;
        self
    }

    pub fn ignore_hash(mut self) -> Self {
        self.ignore_hash = true;
        self
    }

    pub fn with_tooltip(mut self, tooltip: &str) -> Self {
        self.tooltip = Some(tooltip.to_string());
        self
    }

    pub fn with_string_kind(mut self, kind: StringKind) -> Self {
        self.string_kind = kind;
        self
    }

    pub fn param_type(&self) -> ParamType {
        self.value.param_type()
    }

    pub fn label(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.name)
    }

    pub fn is_default(&self) -> bool {
        match (&self.value, &self.default) {
            (ParamValue::Title, _) => true,
            (ParamValue::Guid(_), _) | (ParamValue::Strokes(_), _) => true,
            (ParamValue::Double(value), ParamValue::Double(default)) => value == default,
            (value, default) => value == default,
        }
    }

    /// Hash fragment, used for cache keys.
    pub fn hash_fragment(&self) -> String {
        match &self.value {
            ParamValue::Title => String::new(),
            ParamValue::Bool(value) => if *value { "1".into() } else { "0".into() },
            ParamValue::Int(value) | ParamValue::Choice(value) => value.to_string(),
            ParamValue::Double(value) => value.to_string(),
            ParamValue::Color(color) => format!("{},{},{},{}", color.r * 255.0, color.g * 255.0, color.b * 255.0, color.a * 255.0),
            ParamValue::String(value) => value.clone(),
            ParamValue::Guid(value) => value.to_string(),
            ParamValue::Range(a, b) => format!("{a}{b}"),
            ParamValue::Curve(points) => {
                let sum: f32 = points.iter().map(|point| point.x + point.y).sum();
                sum.to_string()
            }
            ParamValue::Strokes(strokes) => format!("{}", strokes.len()),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParamList {
    pub items: Vec<Param>,
}

impl ParamList {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn from_vec(items: Vec<Param>) -> Self {
        Self { items }
    }

    pub fn push(&mut self, param: Param) {
        self.items.push(param);
    }

    pub fn extend(&mut self, params: impl IntoIterator<Item = Param>) {
        self.items.extend(params);
    }

    pub fn iter(&self) -> impl Iterator<Item = &Param> {
        self.items.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Param> {
        self.items.iter_mut()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn find(&self, name: &str, param_type: ParamType) -> Option<&Param> {
        self.items.iter().find(|param| param.name == name && param.param_type() == param_type)
    }

    pub fn find_mut(&mut self, name: &str, param_type: ParamType) -> Option<&mut Param> {
        self.items.iter_mut().find(|param| param.name == name && param.param_type() == param_type)
    }

    /// First parameter with this name regardless of type.
    pub fn find_any(&self, name: &str) -> Option<&Param> {
        self.items.iter().find(|param| param.name == name)
    }

    pub fn find_any_mut(&mut self, name: &str) -> Option<&mut Param> {
        self.items.iter_mut().find(|param| param.name == name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.items.iter().any(|param| param.name == name)
    }

    pub fn bool(&self, name: &str) -> bool {
        self.bool_or(name, false)
    }

    pub fn bool_or(&self, name: &str, fallback: bool) -> bool {
        match self.find(name, ParamType::Boolean).map(|param| &param.value) {
            Some(ParamValue::Bool(value)) => *value,
            _ => fallback,
        }
    }

    pub fn double(&self, name: &str) -> f64 {
        self.double_or(name, 0.0)
    }

    pub fn double_or(&self, name: &str, fallback: f64) -> f64 {
        match self.find(name, ParamType::Double).map(|param| &param.value) {
            Some(ParamValue::Double(value)) => *value,
            _ => fallback,
        }
    }

    /// The double parameter `name` narrowed to a float.
    pub fn float(&self, name: &str) -> f32 {
        self.double(name) as f32
    }

    pub fn float_or(&self, name: &str, fallback: f32) -> f32 {
        self.double_or(name, fallback as f64) as f32
    }

    pub fn int(&self, name: &str) -> i32 {
        self.int_or(name, 0)
    }

    pub fn int_or(&self, name: &str, fallback: i32) -> i32 {
        match self.find(name, ParamType::Integer).map(|param| &param.value) {
            Some(ParamValue::Int(value)) => *value,
            _ => fallback,
        }
    }

    pub fn choice(&self, name: &str) -> i32 {
        self.choice_or(name, 0)
    }

    pub fn choice_or(&self, name: &str, fallback: i32) -> i32 {
        match self.find(name, ParamType::Choice).map(|param| &param.value) {
            Some(ParamValue::Choice(value)) => *value,
            _ => fallback,
        }
    }

    pub fn color(&self, name: &str) -> Rgba {
        match self.find_any(name).map(|param| &param.value) {
            Some(ParamValue::Color(value)) => *value,
            _ => Rgba::BLACK,
        }
    }

    pub fn string(&self, name: &str) -> String {
        match self.find_any(name).map(|param| &param.value) {
            Some(ParamValue::String(value)) => value.clone(),
            _ => String::new(),
        }
    }

    pub fn guid(&self, name: &str) -> Uuid {
        match self.find_any(name).map(|param| &param.value) {
            Some(ParamValue::Guid(value)) => *value,
            _ => Uuid::nil(),
        }
    }

    pub fn range(&self, name: &str) -> (f64, f64) {
        match self.find_any(name).map(|param| &param.value) {
            Some(ParamValue::Range(a, b)) => (*a, *b),
            _ => (0.0, 0.0),
        }
    }

    pub fn curve(&self, name: &str) -> Vec<CurvePoint> {
        match self.find(name, ParamType::Curve).map(|param| &param.value) {
            Some(ParamValue::Curve(points)) => points.clone(),
            _ => Vec::new(),
        }
    }

    pub fn strokes(&self, name: &str) -> Vec<Stroke> {
        match self.find(name, ParamType::Stroke).map(|param| &param.value) {
            Some(ParamValue::Strokes(strokes)) => strokes.clone(),
            _ => Vec::new(),
        }
    }

    pub fn set(&mut self, name: &str, value: ParamValue) -> bool {
        let param_type = value.param_type();
        match self.find_mut(name, param_type) {
            Some(param) => {
                param.value = value;
                true
            }
            None => false,
        }
    }

    pub fn set_visible(&mut self, name: &str, param_type: ParamType, visible: bool) {
        if let Some(param) = self.find_mut(name, param_type) {
            param.visible = visible;
        }
    }

    /// Makes every parameter with this name (any type) visible or hidden.
    pub fn set_visible_any(&mut self, name: &str, visible: bool) {
        for param in self.items.iter_mut().filter(|param| param.name == name) {
            param.visible = visible;
        }
    }

    pub fn any_non_default(&self) -> bool {
        self.items.iter().any(|param| !param.is_default())
    }

    /// Copies serialized values onto this definition list (matched by name + type).
    /// Mirrors how a node is rebuilt from its definition and then given its saved values.
    pub fn apply_values_from(&mut self, saved: &ParamList) {
        for saved_param in &saved.items {
            let param_type = saved_param.param_type();
            if param_type == ParamType::Divider {
                continue;
            }
            // Range values are never restored.
            if matches!(param_type, ParamType::Range | ParamType::Object) {
                continue;
            }
            if let Some(target) = self.find_mut(&saved_param.name, param_type) {
                target.value = coerce_value(target, &saved_param.value);
                target.auto = saved_param.auto.clone();
                target.expr = saved_param.expr.clone();
                target.owner = saved_param.owner;
                target.raw_xml = saved_param.raw_xml.clone();
                if param_type == ParamType::Integer && saved_param.name == "Seed" {
                    target.default = saved_param.default.clone();
                }
            }
        }
    }

    /// Applies preset values as both value and default.
    pub fn apply_preset(&mut self, preset: &ParamList) {
        for preset_param in &preset.items {
            let param_type = preset_param.param_type();
            let skip = matches!(param_type, ParamType::Divider | ParamType::Object)
                || (param_type == ParamType::Integer && preset_param.name == "Seed");
            if skip {
                continue;
            }
            if let Some(target) = self.find_mut(&preset_param.name, param_type) {
                target.value = preset_param.value.clone();
                if !matches!(param_type, ParamType::Guid | ParamType::Stroke) {
                    target.default = preset_param.value.clone();
                }
            }
        }
    }

    /// Hash string used to key caches.
    pub fn hash_string(&self) -> String {
        let mut text = String::new();
        for param in &self.items {
            if param.param_type() == ParamType::Divider || param.ignore_hash {
                continue;
            }
            text.push_str(&param.hash_fragment());
        }
        text
    }
}

fn coerce_value(target: &Param, saved: &ParamValue) -> ParamValue {
    match (&target.value, saved) {
        (ParamValue::Curve(_), ParamValue::Curve(points)) if points.is_empty() => target.value.clone(),
        // Choices clamp to the current list (the index may outlive removed entries).
        (ParamValue::Choice(_), ParamValue::Choice(index)) => {
            let last = target.choices.len() as i32 - 1;
            ParamValue::Choice((*index).min(last.max(0)))
        }
        _ => saved.clone(),
    }
}

impl<'a> IntoIterator for &'a ParamList {
    type Item = &'a Param;
    type IntoIter = std::slice::Iter<'a, Param>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb8_colours_round_trip_through_scrgb() {
        for value in 0..=255u8 {
            let color = Rgba::from_srgb8(value, value, value);
            assert_eq!(color.to_srgb8(), [value, value, value]);
        }
    }
}
