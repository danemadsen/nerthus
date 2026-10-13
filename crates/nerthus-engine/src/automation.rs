//! The project automation (`Automation`): variables, their bindings to
//! node parameters and parameter expressions, and how a build applies
//! them.
//!
//! Variables keep the .NET object they hold, because the node code unboxes
//! it: the `Var` node's `(float)variable.Value` only works on a boxed
//! float, and modifying an `Int` variable leaves a float behind.

use indexmap::IndexMap;
use serde_json::{json, Value};

use crate::catalogue::{NodeSpec, ParameterSpec, ValueType};
use crate::format_names::VARIABLE_FLOAT2_TYPE;
use crate::jace::{Calculator, JaceError};
use crate::json::{collection_items, References};
use crate::map::Resolution;
use crate::project::NodeData;

/// `VarType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VarType {
    #[default]
    Undefined,
    Float,
    Int,
    Range,
    Choice,
    Bool,
    Color,
    Decoration,
}

impl VarType {
    const NAMES: [&'static str; 8] = ["Undefined", "Float", "Int", "Range", "Choice", "Bool", "Color", "Decoration"];
    const ALL: [VarType; 8] = [
        VarType::Undefined,
        VarType::Float,
        VarType::Int,
        VarType::Range,
        VarType::Choice,
        VarType::Bool,
        VarType::Color,
        VarType::Decoration,
    ];

    /// From its saved name or number.
    fn parse(value: &Value) -> VarType {
        let index = match value {
            Value::String(name) => VarType::NAMES.iter().position(|candidate| candidate == name),
            Value::Number(number) => number.as_u64().map(|number| number as usize),
            _ => None,
        };
        index.and_then(|index| VarType::ALL.get(index).copied()).unwrap_or_default()
    }

    pub fn name(self) -> &'static str {
        let index = VarType::ALL.iter().position(|candidate| *candidate == self).unwrap_or(0);
        VarType::NAMES[index]
    }
}

/// The boxed .NET value a variable holds.
#[derive(Clone, Debug, PartialEq)]
pub enum VariableValue {
    Null,
    Bool(bool),
    Int(i32),
    /// Integers as Newtonsoft reads them into `object`.
    Long(i64),
    Float(f32),
    /// Fractions as Newtonsoft reads them into `object`.
    Double(f64),
    Text(String),
    Float2([f32; 2]),
    /// Anything else (a `JObject`), as written.
    Other(Value),
}

/// Why a conversion of a variable's value failed (`InvalidCastException`,
/// `FormatException`, `OverflowException`).
#[derive(Clone, Debug, PartialEq)]
pub struct ConversionError;

impl VariableValue {
    /// What Newtonsoft deserializes into an `object` member.
    pub fn from_json(value: &Value) -> VariableValue {
        match value {
            Value::Null => VariableValue::Null,
            Value::Bool(flag) => VariableValue::Bool(*flag),
            Value::Number(number) => match number.as_i64() {
                Some(integer) => VariableValue::Long(integer),
                None => VariableValue::Double(number.as_f64().unwrap_or(0.0)),
            },
            Value::String(text) => VariableValue::Text(text.clone()),
            Value::Object(members) => {
                let is_float2 = members.get("$type").and_then(Value::as_str).is_some_and(|name| name.contains("Float2"));
                if is_float2 {
                    let component = |name: &str| members.get(name).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                    return VariableValue::Float2([component("X"), component("Y")]);
                }
                VariableValue::Other(value.clone())
            }
            Value::Array(_) => VariableValue::Other(value.clone()),
        }
    }

    /// How the project file holds it.
    pub fn to_json(&self) -> Value {
        match self {
            VariableValue::Null => Value::Null,
            VariableValue::Bool(flag) => json!(flag),
            VariableValue::Int(integer) => json!(integer),
            VariableValue::Long(integer) => json!(integer),
            VariableValue::Float(number) => json!(*number as f64),
            VariableValue::Double(number) => json!(number),
            VariableValue::Text(text) => json!(text),
            VariableValue::Float2([x, y]) => json!({"$type": VARIABLE_FLOAT2_TYPE, "X": *x as f64, "Y": *y as f64}),
            VariableValue::Other(value) => value.clone(),
        }
    }

    /// `Convert.ToSingle(object)`.
    pub fn to_single(&self) -> Result<f32, ConversionError> {
        match self {
            VariableValue::Null => Ok(0.0),
            VariableValue::Bool(flag) => Ok(if *flag { 1.0 } else { 0.0 }),
            VariableValue::Int(integer) => Ok(*integer as f32),
            VariableValue::Long(integer) => Ok(*integer as f32),
            VariableValue::Float(number) => Ok(*number),
            VariableValue::Double(number) => Ok(*number as f32),
            VariableValue::Text(text) => parse_dotnet_double(text).map(|number| number as f32).ok_or(ConversionError),
            VariableValue::Float2(_) | VariableValue::Other(_) => Err(ConversionError),
        }
    }

    /// `Convert.ToDouble(object)`.
    fn to_double(&self) -> Result<f64, ConversionError> {
        match self {
            VariableValue::Float(number) => Ok(*number as f64),
            VariableValue::Double(number) => Ok(*number),
            VariableValue::Text(text) => parse_dotnet_double(text).ok_or(ConversionError),
            other => other.to_single().map(f64::from),
        }
    }

    /// `Convert.ToInt32(object)`: fractions round half to even; out of
    /// range is an error.
    pub fn to_int32(&self) -> Result<i32, ConversionError> {
        let rounded = |number: f64| {
            let rounded = number.round_ties_even();
            if rounded.is_nan() || rounded < i32::MIN as f64 || rounded > i32::MAX as f64 {
                return Err(ConversionError);
            }
            Ok(rounded as i32)
        };
        match self {
            VariableValue::Null => Ok(0),
            VariableValue::Bool(flag) => Ok(*flag as i32),
            VariableValue::Int(integer) => Ok(*integer),
            VariableValue::Long(integer) => i32::try_from(*integer).map_err(|_| ConversionError),
            VariableValue::Float(number) => rounded(*number as f64),
            VariableValue::Double(number) => rounded(*number),
            VariableValue::Text(text) => text.trim().parse::<i32>().map_err(|_| ConversionError),
            VariableValue::Float2(_) | VariableValue::Other(_) => Err(ConversionError),
        }
    }

    /// `Convert.ToBoolean(object)`.
    pub fn to_boolean(&self) -> Result<bool, ConversionError> {
        match self {
            VariableValue::Null => Ok(false),
            VariableValue::Bool(flag) => Ok(*flag),
            VariableValue::Int(integer) => Ok(*integer != 0),
            VariableValue::Long(integer) => Ok(*integer != 0),
            VariableValue::Float(number) => Ok(*number != 0.0),
            VariableValue::Double(number) => Ok(*number != 0.0),
            VariableValue::Text(text) => match text.trim().to_ascii_lowercase().as_str() {
                "true" => Ok(true),
                "false" => Ok(false),
                _ => Err(ConversionError),
            },
            VariableValue::Float2(_) | VariableValue::Other(_) => Err(ConversionError),
        }
    }

    /// `object.ToString()` (invariant culture).
    pub fn to_text(&self) -> String {
        match self {
            VariableValue::Null => String::new(),
            VariableValue::Bool(flag) => if *flag { "True" } else { "False" }.to_string(),
            VariableValue::Int(integer) => integer.to_string(),
            VariableValue::Long(integer) => integer.to_string(),
            VariableValue::Float(number) => dotnet_float_string(*number),
            VariableValue::Double(number) => dotnet_double_string(*number),
            VariableValue::Text(text) => text.clone(),
            VariableValue::Float2([x, y]) => format!("{}, {}", dotnet_float_string(*x), dotnet_float_string(*y)),
            VariableValue::Other(value) => value.to_string(),
        }
    }

    /// `Variable.ConvertToFloat2`: a `Float2`, or text (`{"X":…}` or `x,y`).
    fn to_float2(&self) -> Result<[f32; 2], ConversionError> {
        match self {
            VariableValue::Float2(pair) => Ok(*pair),
            VariableValue::Text(text) => {
                let components = parse_components(text, &["X", "Y"])?;
                Ok([components[0], components[1]])
            }
            _ => Err(ConversionError),
        }
    }

    /// `Variable.ConvertToFloat3`.
    fn to_float3(&self) -> Result<[f32; 3], ConversionError> {
        match self {
            VariableValue::Text(text) => {
                let components = parse_components(text, &["X", "Y", "Z"])?;
                Ok([components[0], components[1], components[2]])
            }
            _ => Err(ConversionError),
        }
    }

    /// `Variable.ConvertToColor` for colours given as `{"R":…}` or `r,g,b`.
    fn to_color(&self) -> Result<[f32; 3], ConversionError> {
        match self {
            VariableValue::Text(text) => {
                let components = parse_components(text, &["R", "G", "B"])?;
                Ok([components[0], components[1], components[2]])
            }
            _ => Err(ConversionError),
        }
    }
}

/// `{"X": 1, "Y": 2}` (missing members are 0) or comma-separated numbers.
fn parse_components(text: &str, names: &[&str]) -> Result<Vec<f32>, ConversionError> {
    let trimmed = text.trim();
    if trimmed.starts_with('{') {
        let object: Value = serde_json::from_str(trimmed).map_err(|_| ConversionError)?;
        let components = names.iter().map(|name| object.get(*name).and_then(Value::as_f64).unwrap_or(0.0) as f32).collect();
        return Ok(components);
    }
    let parts: Vec<&str> = trimmed.split(',').collect();
    if parts.len() != names.len() {
        return Err(ConversionError);
    }
    let mut components = Vec::with_capacity(parts.len());
    for part in parts {
        let number = parse_dotnet_double(part).ok_or(ConversionError)?;
        components.push(number as f32);
    }
    Ok(components)
}

/// `double.Parse` (invariant culture, `Float | AllowThousands`).
fn parse_dotnet_double(text: &str) -> Option<f64> {
    let cleaned: String = text.trim().chars().filter(|character| *character != ',').collect();
    match cleaned.as_str() {
        "NaN" => Some(f64::NAN),
        "Infinity" | "∞" => Some(f64::INFINITY),
        "-Infinity" | "-∞" => Some(f64::NEG_INFINITY),
        _ => {
            let has_digit = cleaned.chars().any(|character| character.is_ascii_digit());
            let plain = cleaned.chars().all(|character| character.is_ascii_digit() || matches!(character, '.' | '-' | '+' | 'e' | 'E'));
            if !has_digit || !plain {
                return None;
            }
            cleaned.parse::<f64>().ok()
        }
    }
}

/// Shortest round-trip digits and decimal exponent of a finite non-zero
/// number, as Rust's `{:e}` prints them.
fn shortest_digits(scientific: &str) -> (String, i32) {
    let (mantissa, exponent) = scientific.split_once('e').expect("exponent form");
    let digits = mantissa.chars().filter(|character| character.is_ascii_digit()).collect();
    (digits, exponent.parse().expect("an exponent"))
}

/// .NET's general format of shortest digits: exponent form when the
/// decimal point would sit more than `precision` digits in, or more than
/// three zeros before the digits (`1E-05`, `1E+09`).
fn dotnet_general(negative: bool, digits: &str, exponent: i32, precision: i32) -> String {
    let mut text = String::new();
    if negative {
        text.push('-');
    }
    let point = exponent + 1;
    if point > precision || point < -3 {
        text.push_str(&digits[..1]);
        if digits.len() > 1 {
            text.push('.');
            text.push_str(&digits[1..]);
        }
        let sign = if exponent < 0 { '-' } else { '+' };
        text.push('E');
        text.push(sign);
        text.push_str(&format!("{:02}", exponent.abs()));
        return text;
    }
    if point <= 0 {
        text.push_str("0.");
        for _ in 0..-point {
            text.push('0');
        }
        text.push_str(digits);
        return text;
    }
    let point = point as usize;
    if digits.len() <= point {
        text.push_str(digits);
        for _ in digits.len()..point {
            text.push('0');
        }
        return text;
    }
    text.push_str(&digits[..point]);
    text.push('.');
    text.push_str(&digits[point..]);
    text
}

/// `float.ToString(CultureInfo.InvariantCulture)` (.NET Core 3.0+).
pub fn dotnet_float_string(value: f32) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if value == 0.0 {
        return if value.is_sign_negative() { "-0" } else { "0" }.to_string();
    }
    let scientific = format!("{:e}", value.abs());
    let (digits, exponent) = shortest_digits(&scientific);
    dotnet_general(value < 0.0, &digits, exponent, 9)
}

/// `double.ToString(CultureInfo.InvariantCulture)` (.NET Core 3.0+).
pub fn dotnet_double_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if value == 0.0 {
        return if value.is_sign_negative() { "-0" } else { "0" }.to_string();
    }
    let scientific = format!("{:e}", value.abs());
    let (digits, exponent) = shortest_digits(&scientific);
    dotnet_general(value < 0.0, &digits, exponent, 15)
}

/// One project variable (`Variable`).
#[derive(Clone, Debug)]
pub struct Variable {
    pub name: String,
    pub var_type: VarType,
    pub value: VariableValue,
    pub default: VariableValue,
    /// Everything as written (`Min`, `Max`, `ExtraData`, `Order`, …).
    pub json: Value,
}

impl Variable {
    /// A saved variable, recast to its type as loading does.
    pub fn from_json(value: &Value) -> Variable {
        let name = value.get("Name").and_then(Value::as_str).unwrap_or_default().to_string();
        let var_type = value.get("Type").map(VarType::parse).unwrap_or_default();
        let raw_value = value.get("Value").map(VariableValue::from_json).unwrap_or(VariableValue::Null);
        let raw_default = value.get("Default").map(VariableValue::from_json).unwrap_or(VariableValue::Null);
        let raw_min = value.get("Min").map(VariableValue::from_json).unwrap_or(VariableValue::Null);
        let raw_max = value.get("Max").map(VariableValue::from_json).unwrap_or(VariableValue::Null);
        let mut variable = Variable { name, var_type, value: raw_value, default: raw_default, json: value.clone() };
        variable.recast_to_type(&raw_min, &raw_max);
        variable
    }

    /// `Variable.RecastToType`, run on every variable when a project
    /// opens: the value takes the variable type's .NET type, or the
    /// default when it cannot.
    fn recast_to_type(&mut self, min: &VariableValue, max: &VariableValue) {
        if self.try_recast(min, max).is_err() {
            self.value = self.default.clone();
        }
    }

    fn try_recast(&mut self, min: &VariableValue, max: &VariableValue) -> Result<(), ConversionError> {
        match self.var_type {
            VarType::Int => {
                min.to_int32()?;
                max.to_int32()?;
                self.value = VariableValue::Int(self.value.to_int32()?);
                self.default = VariableValue::Int(self.default.to_int32()?);
            }
            VarType::Float => {
                min.to_single()?;
                max.to_single()?;
                self.value = VariableValue::Float(self.value.to_single()?);
                self.default = VariableValue::Float(self.default.to_single()?);
            }
            VarType::Range => {
                min.to_single()?;
                max.to_single()?;
                if let VariableValue::Text(_) = self.value {
                    self.value = VariableValue::Float2(self.value.to_float2()?);
                }
                if let VariableValue::Text(_) = self.default {
                    self.default = VariableValue::Float2(self.default.to_float2()?);
                }
            }
            VarType::Bool => {
                self.value = VariableValue::Bool(self.value.to_boolean()?);
                self.default = VariableValue::Bool(self.default.to_boolean()?);
            }
            _ => {}
        }
        Ok(())
    }
}

/// `VarBinding`: a node parameter that takes a variable's value.
#[derive(Clone, Debug, PartialEq)]
pub struct VarBinding {
    pub node: i32,
    pub property: String,
    pub variable: String,
}

/// What expressions are evaluated against.
#[derive(Clone, Copy, Debug)]
pub struct ExpressionContext {
    /// The build resolution (`Resolution`, `PX`).
    pub resolution: Resolution,
    /// The terrain's height in metres (`m()`, `cm()`, `km()`).
    pub terrain_height: f32,
}

/// An asset's `Automation`.
#[derive(Clone, Debug, Default)]
pub struct Automation {
    /// By name, in the project's order.
    pub variables: IndexMap<String, Variable>,
    pub bindings: Vec<VarBinding>,
    /// Parameter expressions by `"{node id}_{parameter}"`.
    pub expressions: IndexMap<String, String>,
}

impl Automation {
    pub fn from_json(value: &Value, references: &References) -> Automation {
        let mut automation = Automation::default();
        if let Some(Value::Object(variables)) = value.get("Variables").map(|variables| references.resolve(variables)) {
            for (key, variable) in variables {
                if key.starts_with('$') {
                    continue;
                }
                let variable = references.resolve(variable);
                automation.variables.insert(key.clone(), Variable::from_json(variable));
            }
        }
        if let Some(bindings) = value.get("Bindings").map(|bindings| references.resolve(bindings)) {
            for binding in collection_items(bindings) {
                let binding = references.resolve(binding);
                let node = binding.get("Node").and_then(Value::as_i64).unwrap_or(0) as i32;
                let property = binding.get("Property").and_then(Value::as_str).unwrap_or_default().to_string();
                let variable = binding.get("Variable").and_then(Value::as_str).unwrap_or_default().to_string();
                automation.bindings.push(VarBinding { node, property, variable });
            }
        }
        if let Some(Value::Object(expressions)) = value.get("Expressions").map(|expressions| references.resolve(expressions)) {
            for (key, expression) in expressions {
                if key.starts_with('$') {
                    continue;
                }
                if let Some(text) = expression.as_str() {
                    automation.expressions.insert(key.clone(), text.to_string());
                }
            }
        }
        automation
    }

    /// The configured Jace engine with the float, integer and
    /// boolean variables as constants, then `Resolution` and `PX`.
    fn calculate(&self, formula: &str, context: &ExpressionContext) -> Result<f64, JaceError> {
        let mut calculator = Calculator::new(context.terrain_height);
        let unconvertible = || JaceError::Parse("The variable's value could not be converted.".to_string());
        for variable in self.variables.values() {
            let constant = match variable.var_type {
                VarType::Float | VarType::Int => variable.value.to_single().map_err(|_| unconvertible())? as f64,
                VarType::Bool => {
                    let flag = variable.value.to_boolean().map_err(|_| unconvertible())?;
                    if flag {
                        1.0
                    } else {
                        0.0
                    }
                }
                _ => continue,
            };
            calculator.add_constant(&variable.name, constant)?;
        }
        let columns = context.resolution.x;
        calculator.add_constant("Resolution", columns as f64)?;
        let pixel = 1.0f32 / columns as f32;
        calculator.add_constant("PX", pixel as f64)?;
        calculator.calculate(formula)
    }

    /// The formula engine as the editors call it on typed text: the formula's
    /// value as a float, or `None` when it does not work out.
    pub fn calculate_number(&self, formula: &str, context: &ExpressionContext) -> Option<f32> {
        self.calculate(formula, context).ok().map(|value| value as f32)
    }

    /// The expression with every `x` replaced by the
    /// value's text, or the value unchanged when anything fails.
    pub fn evaluate(&self, expression: &str, value: f32, context: &ExpressionContext) -> f32 {
        let value_text = dotnet_float_string(value);
        let formula = expression.replace('x', &value_text);
        match self.calculate(&formula, context) {
            Ok(result) => result as f32,
            Err(_) => value,
        }
    }

    /// The parameter's expression of `value`, if it has
    /// one.
    pub fn apply_expression(&self, node: i32, parameter: &str, value: f32, context: &ExpressionContext) -> f32 {
        let key = format!("{node}_{parameter}");
        match self.expressions.get(&key) {
            Some(expression) => self.evaluate(expression, value, context),
            None => value,
        }
    }

    /// What runs before each node builds: the values its bound
    /// parameters take from their variables (floats through their
    /// expressions), or — for a node without bindings — each float
    /// parameter with an expression replaced by the expression of its
    /// value. Bindings whose value does not convert are skipped (only
    /// logged); a binding to a missing variable fails the build.
    pub fn bind(&self, node: &NodeData, spec: &NodeSpec, context: &ExpressionContext) -> Result<Vec<(String, Value)>, String> {
        let mut changes = Vec::new();
        let bindings: Vec<&VarBinding> = self.bindings.iter().filter(|binding| binding.node == node.id).collect();
        if !bindings.is_empty() {
            for binding in bindings {
                let Some(parameter) = spec.parameters.iter().find(|parameter| parameter.name == binding.property) else {
                    continue;
                };
                let Some(variable) = self.variables.get(&binding.variable) else {
                    return Err(format!("The given key '{}' was not present in the dictionary.", binding.variable));
                };
                if let Ok(value) = self.bound_value(node.id, parameter, variable, context) {
                    changes.push((parameter.name.clone(), value));
                }
            }
            return Ok(changes);
        }

        for parameter in spec.parameters.iter().filter(|parameter| parameter.value_type == ValueType::Float) {
            let key = format!("{}_{}", node.id, parameter.name);
            if !self.expressions.contains_key(&key) {
                continue;
            }
            let current = node.values.get(&parameter.name).and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let result = self.apply_expression(node.id, &parameter.name, current, context);
            changes.push((parameter.name.clone(), json!(result as f64)));
        }
        Ok(changes)
    }

    /// The value a binding gives a parameter of each type.
    fn bound_value(&self, node: i32, parameter: &ParameterSpec, variable: &Variable, context: &ExpressionContext) -> Result<Value, ConversionError> {
        let value = &variable.value;
        let converted = match parameter.value_type {
            ValueType::Float => {
                let number = value.to_single()?;
                let result = self.apply_expression(node, &parameter.name, number, context);
                json!(result as f64)
            }
            ValueType::Float2 => {
                let [x, y] = value.to_float2()?;
                json!({"X": x as f64, "Y": y as f64})
            }
            ValueType::Float3 => {
                let [x, y, z] = value.to_float3()?;
                json!({"X": x as f64, "Y": y as f64, "Z": z as f64})
            }
            ValueType::Color => {
                let [red, green, blue] = value.to_color()?;
                json!({"R": red as f64, "G": green as f64, "B": blue as f64})
            }
            ValueType::Enum if variable.var_type == VarType::Choice => {
                let index: i64 = value.to_text().trim().parse().map_err(|_| ConversionError)?;
                match parameter.enum_name(index) {
                    Some(name) => json!(name),
                    None => json!(index),
                }
            }
            // `Convert.ChangeType(value, propertyType)`.
            ValueType::Int => json!(value.to_int32()?),
            ValueType::Byte => {
                let integer = value.to_int32()?;
                let byte = u8::try_from(integer).map_err(|_| ConversionError)?;
                json!(byte)
            }
            ValueType::Double => json!(value.to_double()?),
            ValueType::Bool => json!(value.to_boolean()?),
            ValueType::String => json!(value.to_text()),
            _ => return Err(ConversionError),
        };
        Ok(converted)
    }
}

/// The variable a property gets when it is bound to a
/// new one, as saved (`Order`, `Name`, `Value`, `Type`, `Min`, `Max`,
/// `Default`, `ExtraData`; members at their .NET default left out).
/// `current` is the property's value, `default` its type's default.
pub fn variable_for_property(key: &str, parameter: &ParameterSpec, current: &Value, default: Option<&Value>, order: usize) -> Value {
    let float = |value: f64| crate::json::f32_value(value as f32);
    let min = parameter.min.unwrap_or(0.0);
    let max = parameter.max.unwrap_or(0.0);
    let mut variable = serde_json::Map::new();
    if order != 0 {
        variable.insert("Order".to_string(), json!(order));
    }
    variable.insert("Name".to_string(), json!(key));
    match parameter.value_type {
        ValueType::Int | ValueType::Byte => {
            let fallback = default.and_then(Value::as_i64).unwrap_or(0);
            variable.insert("Value".to_string(), json!(current.as_i64().unwrap_or(fallback)));
            variable.insert("Type".to_string(), json!("Int"));
            variable.insert("Min".to_string(), json!(min as i64));
            variable.insert("Max".to_string(), json!(max as i64));
            variable.insert("Default".to_string(), json!(fallback));
        }
        ValueType::Float | ValueType::Double => {
            let fallback = default.and_then(Value::as_f64).unwrap_or(0.0);
            variable.insert("Value".to_string(), float(current.as_f64().unwrap_or(fallback)));
            variable.insert("Type".to_string(), json!("Float"));
            variable.insert("Min".to_string(), float(min));
            variable.insert("Max".to_string(), float(max));
            variable.insert("Default".to_string(), float(fallback));
        }
        ValueType::Float2 => {
            let component = |name: &str, fallback: f64| current.get(name).and_then(Value::as_f64).unwrap_or(fallback) as f32;
            let value = VariableValue::Float2([component("X", min), component("Y", max)]);
            variable.insert("Value".to_string(), value.to_json());
            variable.insert("Type".to_string(), json!("Range"));
            variable.insert("Min".to_string(), float(min));
            variable.insert("Max".to_string(), float(max));
            variable.insert("Default".to_string(), VariableValue::Float2([min as f32, max as f32]).to_json());
        }
        ValueType::Bool => {
            let fallback = default.and_then(Value::as_bool).unwrap_or(false);
            variable.insert("Value".to_string(), json!(current.as_bool().unwrap_or(fallback)));
            variable.insert("Type".to_string(), json!("Bool"));
            variable.insert("Default".to_string(), json!(fallback));
        }
        ValueType::String => {
            if !current.is_null() {
                variable.insert("Value".to_string(), current.clone());
            }
        }
        ValueType::Enum => {
            let member = current.as_str().unwrap_or_default();
            variable.insert("Value".to_string(), json!(parameter.enum_value(member).unwrap_or(0)));
            variable.insert("Type".to_string(), json!("Choice"));
            let names = json!({
                "$type": "System.Collections.Generic.List`1[[System.String, System.Private.CoreLib]], System.Private.CoreLib",
                "$values": parameter.enum_names,
            });
            variable.insert("ExtraData".to_string(), names);
        }
        ValueType::Color => {
            // Binding names colour variables after the property.
            variable.insert("Name".to_string(), json!(parameter.name));
            variable.insert("Value".to_string(), current.clone());
            variable.insert("Type".to_string(), json!("Color"));
        }
        _ => {}
    }
    Value::Object(variable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_print_like_dotnet() {
        let cases = [
            (0.5f32, "0.5"),
            (1e-5, "1E-05"),
            (1e-4, "0.0001"),
            (0.0001234, "0.0001234"),
            (1234567.0, "1234567"),
            (123456789.0, "123456790"),
            (1e8, "100000000"),
            (1e9, "1E+09"),
            (1.2345678e10, "1.2345678E+10"),
            (-0.0, "-0"),
            (f32::MAX, "3.4028235E+38"),
            (1.0e-45, "1E-45"),
            (0.1, "0.1"),
            (1.0 / 3.0, "0.33333334"),
            (-2.75, "-2.75"),
            (9.9999e-5, "9.9999E-05"),
        ];
        for (value, expected) in cases {
            assert_eq!(dotnet_float_string(value), expected, "{value:e}");
        }
    }

    #[test]
    fn bound_floats_make_float_variables() {
        let spec = crate::catalogue::Catalogue::get().node("ErosionPro").unwrap();
        let parameter = spec.parameter("Duration").unwrap();
        let variable = variable_for_property("Duration", parameter, &json!(0.3), spec.defaults.get("Duration"), 2);
        let keys: Vec<&str> = variable.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["Order", "Name", "Value", "Type", "Min", "Max", "Default"]);
        assert_eq!(variable["Type"], json!("Float"));
        let parsed = Variable::from_json(&variable);
        assert_eq!(parsed.var_type, VarType::Float);
        assert_eq!(parsed.value.to_single(), Ok(0.3));
    }

    #[test]
    fn expressions_substitute_the_value() {
        let automation = Automation::default();
        let context = ExpressionContext { resolution: Resolution::square(1024), terrain_height: 2500.0 };
        assert_eq!(automation.evaluate("x*2", 0.25, &context), 0.5);
        assert_eq!(automation.evaluate("x*Resolution", 0.5, &context), 512.0);
        assert_eq!(automation.evaluate("2", 7.0, &context), 2.0);
        // `max` loses its x: "ma0.5(0.5,1)" does not parse.
        assert_eq!(automation.evaluate("max(x,1)", 0.5, &context), 0.5);
        assert_eq!(automation.evaluate("m(1250)", 0.0, &context), 0.5);
    }
}
