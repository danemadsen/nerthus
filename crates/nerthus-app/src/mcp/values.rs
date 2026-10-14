//! Parameter values: what a tool is given, stored as the property area's
//! editors store it (numbers clamped to the slider's range as typing into
//! it does, floats at single precision, enums by member name, colours as
//! scRGB), and parameters described for the model.

use nerthus_engine::json::f32_value;
use nerthus_engine::{ModifierSpec, NodeSpec, ParameterSpec, ValueType};
use serde_json::{json, Map, Value};

use super::arguments::{boolean_of, number_of, pair_of};
use crate::ui::editors::{range_of, SliderSpec};

/// What a stored value came to, and a note when it differs from what was
/// given (clamped, rounded).
pub(crate) struct Converted {
    pub value: Value,
    pub note: Option<String>,
}

impl Converted {
    fn exact(value: Value) -> Self {
        Self { value, note: None }
    }
}

/// Whether a tool may set the parameter: hidden ones, command buttons and
/// the panel's notes and computed text are not values.
pub(crate) fn settable(parameter: &ParameterSpec) -> Result<(), String> {
    let name = &parameter.name;
    if parameter.hidden {
        return Err(format!("{name} is kept by the node itself."));
    }
    if matches!(parameter.kind.as_str(), "Command" | "CommandButton") {
        return Err(format!("{name} is a button in the property panel, not a value."));
    }
    if parameter.value_type == ValueType::Byte || parameter.kind == "ComputedString" {
        return Err(format!("{name} is text the property panel shows, not a value."));
    }
    if matches!(parameter.value_type, ValueType::Dictionary | ValueType::Object) {
        return Err(format!("{name} has its own editor and cannot be set here."));
    }
    Ok(())
}

/// A node type's parameter by name or label, any case.
pub(crate) fn find_parameter<'a>(spec: &'a NodeSpec, name: &str) -> Result<&'a ParameterSpec, String> {
    let found = spec.parameters.iter().find(|parameter| parameter.name.eq_ignore_ascii_case(name) || parameter.display_name().eq_ignore_ascii_case(name));
    found.ok_or_else(|| {
        let names: Vec<&str> = spec.parameters.iter().filter(|parameter| settable(parameter).is_ok()).map(|parameter| parameter.name.as_str()).collect();
        format!("{} has no parameter {name}; its parameters are {}.", spec.name, names.join(", "))
    })
}

/// A modifier type's parameter by name, any case.
pub(crate) fn find_modifier_parameter<'a>(spec: &'a ModifierSpec, name: &str) -> Result<&'a ParameterSpec, String> {
    let found = spec.parameters.iter().find(|parameter| parameter.name.eq_ignore_ascii_case(name));
    found.ok_or_else(|| {
        let names: Vec<&str> = spec.parameters.iter().map(|parameter| parameter.name.as_str()).collect();
        if names.is_empty() {
            return format!("The {} modifier has no parameters.", spec.name);
        }
        format!("The {} modifier has no parameter {name}; its parameters are {}.", spec.name, names.join(", "))
    })
}

/// `given` as `parameter` stores it; `current` is the value it has now
/// (colours keep their member names and alpha).
pub(crate) fn convert(parameter: &ParameterSpec, given: &Value, current: Option<&Value>) -> Result<Converted, String> {
    settable(parameter)?;
    let name = &parameter.name;
    match parameter.value_type {
        ValueType::Float | ValueType::Double => {
            let number = number_of(given).ok_or_else(|| format!("{name} takes a number."))?;
            let slider = SliderSpec::of(parameter, None, false);
            let clamped = number.clamp(slider.min, slider.max);
            let value = if parameter.value_type == ValueType::Float { f32_value(clamped as f32) } else { json!(clamped) };
            let note = (clamped != number).then(|| format!("{name} was clamped to {clamped} (its range is {} to {}).", slider.min, slider.max));
            Ok(Converted { value, note })
        }
        ValueType::Int if parameter.is_seed() => {
            let number = number_of(given).ok_or_else(|| format!("{name} takes a whole number."))?;
            let seed = number.round().clamp(0.0, i32::MAX as f64) as i64;
            Ok(Converted::exact(json!(seed)))
        }
        ValueType::Int => {
            let number = number_of(given).ok_or_else(|| format!("{name} takes a whole number."))?;
            let slider = SliderSpec::of(parameter, None, true);
            let whole = number.clamp(slider.min, slider.max).round_ties_even();
            let note = (whole != number).then(|| format!("{name} was set to {whole} (whole numbers from {} to {}).", slider.min, slider.max));
            Ok(Converted { value: json!(whole as i64), note })
        }
        ValueType::Bool => {
            let flag = boolean_of(given).ok_or_else(|| format!("{name} takes true or false."))?;
            Ok(Converted::exact(json!(flag)))
        }
        ValueType::Enum => {
            let member = enum_member(parameter, given)?;
            Ok(Converted::exact(json!(member)))
        }
        ValueType::Float2 | ValueType::Int2 => range_value(parameter, given),
        ValueType::Color => {
            let rgb = colour_of(given).ok_or_else(|| format!("{name} takes a colour: \"#rrggbb\", or [red, green, blue] in scRGB (0 to 1)."))?;
            Ok(Converted::exact(colour_value(rgb, current)))
        }
        ValueType::String => match given {
            Value::String(text) => Ok(Converted::exact(json!(text))),
            _ => Err(format!("{name} takes text.")),
        },
        ValueType::Float2List => curve_value(name, given),
        ValueType::Float4List => gradient_value(name, given),
        _ => Err(format!("{name} cannot be set here.")),
    }
}

/// An enum member by name or label (any case, spaces aside), or by its
/// number.
fn enum_member(parameter: &ParameterSpec, given: &Value) -> Result<String, String> {
    let squeeze = |text: &str| text.chars().filter(|character| character.is_alphanumeric()).collect::<String>().to_lowercase();
    let found = match given {
        Value::String(text) => {
            let wanted = squeeze(text);
            let matches = |member: &&String| squeeze(member.as_str()) == wanted || squeeze(&parameter.enum_display(member.as_str())) == wanted;
            parameter.enum_names.iter().find(matches).cloned()
        }
        Value::Number(_) => {
            let number = number_of(given).unwrap_or(f64::NAN) as i64;
            parameter.enum_name(number).map(str::to_string)
        }
        _ => None,
    };
    found.ok_or_else(|| format!("{} takes one of: {}.", parameter.name, parameter.enum_names.join(", ")))
}

/// A range (`[low, high]`), each end clamped as the range slider clamps
/// it, low first.
fn range_value(parameter: &ParameterSpec, given: &Value) -> Result<Converted, String> {
    let name = &parameter.name;
    let [first, second] = pair_of(given).ok_or_else(|| format!("{name} takes a range: [low, high]."))?;
    let (min, max) = range_of(parameter, 0.0, 1.0);
    let low = first.min(second).clamp(min, max);
    let high = first.max(second).clamp(min, max);
    let clamped = low != first.min(second) || high != first.max(second);
    let note = clamped.then(|| format!("{name} was clamped to [{low}, {high}] (its range is {min} to {max})."));
    let value = if parameter.value_type == ValueType::Int2 {
        json!({"X": low.round_ties_even() as i64, "Y": high.round_ties_even() as i64})
    } else {
        json!({"X": f32_value(low as f32), "Y": f32_value(high as f32)})
    };
    Ok(Converted { value, note })
}

/// A colour as scRGB floats: `#rrggbb` (sRGB, converted), `[r, g, b]` or
/// an object's `R`, `G` and `B` (already scRGB).
pub(crate) fn colour_of(given: &Value) -> Option<[f32; 3]> {
    match given {
        Value::String(text) => {
            let hex = text.trim().trim_start_matches('#');
            if hex.len() != 6 {
                return None;
            }
            let packed = u32::from_str_radix(hex, 16).ok()?;
            let [_, red, green, blue] = packed.to_be_bytes();
            let colour = egui::Color32::from_rgb(red, green, blue);
            Some(crate::theme::sc_rgb(colour))
        }
        Value::Array(items) if items.len() >= 3 => {
            let red = number_of(&items[0])? as f32;
            let green = number_of(&items[1])? as f32;
            let blue = number_of(&items[2])? as f32;
            Some([red, green, blue])
        }
        Value::Object(members) => {
            let component = |names: [&str; 2]| names.iter().find_map(|name| members.get(*name)).and_then(number_of).map(|number| number as f32);
            Some([component(["R", "r"])?, component(["G", "g"])?, component(["B", "b"])?])
        }
        _ => None,
    }
}

/// The colour written as the colour editor writes it: in the members the
/// value has (`X`… when it has those), its alpha kept.
fn colour_value(rgb: [f32; 3], current: Option<&Value>) -> Value {
    let mut value = current.filter(|current| current.is_object()).cloned().unwrap_or_else(|| json!({}));
    let uses_xyzw = value.get("X").is_some();
    let names = if uses_xyzw { ["X", "Y", "Z"] } else { ["R", "G", "B"] };
    for (name, component) in names.iter().zip(rgb) {
        value[*name] = f32_value(component);
    }
    value
}

/// A curve's points: `[[x, y], …]`, each within 0 to 1, sorted along x.
fn curve_value(name: &str, given: &Value) -> Result<Converted, String> {
    let problem = || format!("{name} takes a curve: two or more [x, y] points from 0 to 1.");
    let items = given.as_array().filter(|items| items.len() >= 2).ok_or_else(problem)?;
    let mut points = Vec::new();
    for item in items {
        let [x, y] = pair_of(item).ok_or_else(problem)?;
        points.push([x.clamp(0.0, 1.0) as f32, y.clamp(0.0, 1.0) as f32]);
    }
    points.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let list: Vec<Value> = points.iter().map(|[x, y]| json!({"X": f32_value(*x), "Y": f32_value(*y)})).collect();
    Ok(Converted::exact(Value::Array(list)))
}

/// A gradient's stops: `{"color": "#rrggbb", "position": 0.5}` objects or
/// `[r, g, b, position]` lists (scRGB), sorted by position.
fn gradient_value(name: &str, given: &Value) -> Result<Converted, String> {
    let problem = || format!("{name} takes a gradient: two or more stops, each {{\"color\": \"#rrggbb\", \"position\": 0 to 1}}.");
    let items = given.as_array().filter(|items| items.len() >= 2).ok_or_else(problem)?;
    let mut stops = Vec::new();
    for item in items {
        let stop = gradient_stop(item).ok_or_else(problem)?;
        stops.push(stop);
    }
    stops.sort_by(|a, b| a[3].total_cmp(&b[3]));
    let list: Vec<Value> = stops.iter().map(|[x, y, z, w]| json!({"X": f32_value(*x), "Y": f32_value(*y), "Z": f32_value(*z), "W": f32_value(*w)})).collect();
    Ok(Converted::exact(Value::Array(list)))
}

fn gradient_stop(item: &Value) -> Option<[f32; 4]> {
    match item {
        Value::Array(components) if components.len() == 4 => {
            let red = number_of(&components[0])? as f32;
            let green = number_of(&components[1])? as f32;
            let blue = number_of(&components[2])? as f32;
            let position = number_of(&components[3])? as f32;
            Some([red, green, blue, position.clamp(0.0, 1.0)])
        }
        Value::Object(members) => {
            if let (Some(x), Some(y), Some(z), Some(w)) = (members.get("X"), members.get("Y"), members.get("Z"), members.get("W")) {
                return Some([number_of(x)? as f32, number_of(y)? as f32, number_of(z)? as f32, number_of(w)?.clamp(0.0, 1.0) as f32]);
            }
            let colour = members.get("color").or_else(|| members.get("colour"))?;
            let [red, green, blue] = colour_of(colour)?;
            let position = members.get("position").and_then(number_of)?;
            Some([red, green, blue, position.clamp(0.0, 1.0) as f32])
        }
        _ => None,
    }
}

/// The parameter's kind as the model is told it.
fn type_label(parameter: &ParameterSpec) -> &'static str {
    match parameter.value_type {
        ValueType::Float | ValueType::Double => "number",
        ValueType::Int if parameter.is_seed() => "seed",
        ValueType::Int => "integer",
        ValueType::Bool => "boolean",
        ValueType::Enum => "enum",
        ValueType::Float2 => "range",
        ValueType::Int2 => "integer range",
        ValueType::Color => "colour",
        ValueType::String => match parameter.kind.as_str() {
            "TextBox" => "text",
            "Folder" => "folder",
            _ => "file",
        },
        ValueType::Float2List => "curve",
        ValueType::Float4List => "gradient",
        _ => "object",
    }
}

/// A parameter for the model: name, label, type, limits, options and
/// description, with its default when given.
pub(crate) fn describe(parameter: &ParameterSpec, default: Option<&Value>) -> Map<String, Value> {
    let mut entry = Map::new();
    entry.insert("name".to_string(), json!(parameter.name));
    let label = parameter.display_name();
    if label != parameter.name {
        entry.insert("label".to_string(), json!(label));
    }
    entry.insert("type".to_string(), json!(type_label(parameter)));
    match parameter.value_type {
        ValueType::Float | ValueType::Double => {
            let slider = SliderSpec::of(parameter, None, false);
            entry.insert("min".to_string(), json!(slider.min));
            entry.insert("max".to_string(), json!(slider.max));
        }
        ValueType::Int if !parameter.is_seed() => {
            let slider = SliderSpec::of(parameter, None, true);
            entry.insert("min".to_string(), json!(slider.min));
            entry.insert("max".to_string(), json!(slider.max));
        }
        ValueType::Float2 | ValueType::Int2 => {
            let (min, max) = range_of(parameter, 0.0, 1.0);
            entry.insert("min".to_string(), json!(min));
            entry.insert("max".to_string(), json!(max));
        }
        ValueType::Enum => {
            entry.insert("options".to_string(), json!(parameter.enum_names));
        }
        _ => {}
    }
    if let Some(default) = default.filter(|default| !default.is_null()) {
        entry.insert("default".to_string(), default.clone());
    }
    // The slider's exponent: the value does not follow the slider evenly.
    if let Some(curve) = parameter.curve.as_ref().filter(|curve| !curve.is_empty()) {
        entry.insert("curve".to_string(), json!(curve));
    }
    if let Some(description) = &parameter.description {
        entry.insert("description".to_string(), json!(description));
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerthus_engine::Catalogue;

    fn parameter(node: &str, name: &str) -> &'static ParameterSpec {
        let spec = Catalogue::get().node(node).unwrap();
        spec.parameter(name).unwrap()
    }

    #[test]
    fn numbers_are_clamped_as_typing_clamps_them() {
        let scale = parameter("Mountain", "Scale");
        let converted = convert(scale, &json!(0.3), None).unwrap();
        assert_eq!(converted.value, json!(0.3));
        assert!(converted.note.is_none());
        let clamped = convert(scale, &json!("7"), None).unwrap();
        assert_eq!(clamped.value, json!(1.0));
        assert!(clamped.note.is_some());
    }

    #[test]
    fn enums_take_names_in_any_case() {
        let style = parameter("Mountain", "Style");
        assert_eq!(convert(style, &json!("alpine"), None).unwrap().value, json!("Alpine"));
        assert!(convert(style, &json!("Volcanic"), None).is_err());
    }

    #[test]
    fn colours_keep_their_members() {
        let current = json!({"R": 0.0, "G": 0.0, "B": 0.0, "A": 0.5});
        let value = colour_value([1.0, 0.5, 0.25], Some(&current));
        assert_eq!(value, json!({"R": 1.0, "G": 0.5, "B": 0.25, "A": 0.5}));
        assert_eq!(colour_of(&json!("#ffffff")), Some([1.0, 1.0, 1.0]));
    }

    #[test]
    fn curves_are_sorted_and_clamped() {
        let converted = curve_value("Curve", &json!([[1.0, 2.0], [0.0, 0.0], [0.5, 0.25]])).unwrap();
        assert_eq!(converted.value, json!([{"X": 0.0, "Y": 0.0}, {"X": 0.5, "Y": 0.25}, {"X": 1.0, "Y": 1.0}]));
    }
}
