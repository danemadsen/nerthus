//! The native `MathX_Compute` (the Math node): an ExprTk expression of
//! `x`, `y`, `a`, `b`, `c` and the project's variables, evaluated per
//! pixel in `float`.

use nerthus_engine::automation::{dotnet_float_string, VarType, VariableValue};
use rayon::prelude::*;

use crate::exprtk::{self, is_reserved_symbol, Expression, SymbolTable};

/// `int.Parse(text)` (invariant culture, `NumberStyles.Integer`).
fn dotnet_int_parse(text: &str) -> Option<i32> {
    let trimmed = text.trim_matches(|character: char| matches!(character, ' ' | '\t' | '\n' | '\r' | '\u{0B}' | '\u{0C}'));
    let digits = trimmed.strip_prefix('+').unwrap_or(trimmed);
    let unsigned = digits.strip_prefix('-').unwrap_or(digits);
    if unsigned.is_empty() || !unsigned.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// The variables string handed to `MathX_Compute`:
/// `Resolution=…;PX=…;` then each project variable it can write (floats,
/// ints and choices; booleans as `True`/`False`, which never parse).
pub fn variables_string(resolution: usize, variables: &[(String, VarType, VariableValue)]) -> String {
    let pixel = 1.0f32 / resolution as f32;
    let mut text = format!("Resolution={resolution};PX={};", dotnet_float_string(pixel));
    for (name, var_type, value) in variables {
        let written = match var_type {
            VarType::Bool => match value {
                VariableValue::Bool(flag) => Some(if *flag { "True".to_string() } else { "False".to_string() }),
                _ => None,
            },
            VarType::Choice | VarType::Int => dotnet_int_parse(&value.to_text()).map(|integer| integer.to_string()),
            VarType::Float => match value {
                VariableValue::Float(number) => Some(dotnet_float_string(*number)),
                _ => None,
            },
            _ => None,
        };
        if let Some(written) = written {
            text.push_str(&format!("{name}={written};"));
        }
    }
    text
}

/// `std::from_chars(float)` on the start of `text` (general format, no
/// leading `+` or whitespace): `None` where it reports an error.
fn from_chars(text: &str) -> Option<f32> {
    let bytes = text.as_bytes();
    let mut index = 0;
    if bytes.first() == Some(&b'-') {
        index += 1;
    }
    let rest = text[index..].to_ascii_lowercase();
    if rest.starts_with("inf") || rest.starts_with("nan") {
        let magnitude = if rest.starts_with("inf") { f32::INFINITY } else { f32::NAN };
        return Some(if index == 1 { -magnitude } else { magnitude });
    }
    let mut mantissa_digits = 0;
    let mut nonzero = false;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        nonzero |= bytes[index] != b'0';
        mantissa_digits += 1;
        index += 1;
    }
    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            nonzero |= bytes[index] != b'0';
            mantissa_digits += 1;
            index += 1;
        }
    }
    if mantissa_digits == 0 {
        return None;
    }
    // An exponent counts only when digits follow it.
    if index < bytes.len() && (bytes[index] == b'e' || bytes[index] == b'E') {
        let mut end = index + 1;
        if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
            end += 1;
        }
        let exponent_digits = bytes[end..].iter().take_while(|byte| byte.is_ascii_digit()).count();
        if exponent_digits > 0 {
            index = end + exponent_digits;
        }
    }
    let value: f32 = text[..index].parse().ok()?;
    // Overflow and underflow to zero are `result_out_of_range`.
    if value.is_infinite() || (value == 0.0 && nonzero) {
        return None;
    }
    Some(value)
}

/// `symbol_table::valid_symbol`.
fn valid_symbol(name: &str) -> bool {
    let bytes = name.as_bytes();
    let Some(first) = bytes.first() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    for (index, byte) in bytes.iter().enumerate().skip(1) {
        if byte.is_ascii_alphanumeric() || *byte == b'_' {
            continue;
        }
        if *byte == b'.' && index < bytes.len() - 1 {
            continue;
        }
        return false;
    }
    !is_reserved_symbol(name)
}

/// The variables string (`Resolution=1024;PX=0.0009765625;`
/// then the project's variables) parsed as `MathX_Compute` does: each
/// `name=value` whose value parses becomes a constant, unless the name is
/// invalid, reserved or taken.
pub fn symbol_table(variables: &str) -> SymbolTable {
    let mut table = SymbolTable { variables: ["x", "y", "a", "b", "c"].map(String::from).to_vec(), constants: Vec::new() };
    let length = variables.len();
    let mut position = 0;
    while position < length {
        let Some(equals) = variables[position..].find('=').map(|offset| position + offset) else {
            break;
        };
        let end = variables[equals..].find(';').map_or(length, |offset| equals + offset);
        let name = &variables[position..equals];
        let value = &variables[equals + 1..end];
        if let Some(value) = from_chars(value)
            && valid_symbol(name) && !table.exists(name) {
                table.constants.push((name.to_string(), value));
            }
        position = end + 1;
    }
    table
}

/// The coordinate `MathX_Compute` gives column (or row) `index` of `size`:
/// `[0, 1)` with normalized coordinates, else `[-1, 1)`.
fn coordinate(index: usize, size: usize, normalized: bool) -> f32 {
    let fraction = index as f32 / size as f32;
    if normalized {
        fraction
    } else {
        (fraction + fraction) - 1.0
    }
}

/// `MathX_Compute`: the expression over a `width` × `height` map, or
/// ExprTk's error.
pub fn compute(expression: &str, variables: &str, inputs: [Option<&[f32]>; 3], width: usize, height: usize, normalized: bool) -> Result<Vec<f32>, String> {
    let lowered = expression.to_ascii_lowercase();
    let table = symbol_table(variables);
    let compiled: Expression = exprtk::compile(&lowered, &table)?;
    let mut result = vec![0.0f32; width * height];
    result.par_chunks_mut(width).enumerate().for_each(|(row, line)| {
        let mut slots = vec![0.0f32; compiled.slot_count()];
        let y = coordinate(row, height, normalized);
        for (column, target) in line.iter_mut().enumerate() {
            let index = row * width + column;
            for (slot, input) in inputs.iter().enumerate() {
                if let Some(map) = input {
                    slots[2 + slot] = map[index];
                }
            }
            slots[0] = coordinate(column, width, normalized);
            slots[1] = y;
            *target = compiled.value(&mut slots);
        }
    });
    Ok(result)
}

/// The message shown: ExprTk's diagnostic without its `ERRnnn - `.
pub fn node_message(error: &str) -> String {
    let bytes = error.as_bytes();
    let numbered = bytes.len() > 6 && error.starts_with("ERR") && bytes[3..].iter().take_while(|byte| byte.is_ascii_digit()).count() > 0;
    if !numbered {
        return error.to_string();
    }
    let digits = bytes[3..].iter().take_while(|byte| byte.is_ascii_digit()).count();
    let rest = &error[3 + digits..];
    rest.strip_prefix(" - ").map_or_else(|| error.to_string(), str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variables_become_constants() {
        let table = symbol_table("Resolution=1024;PX=0.0009765625;Flag=True;x=3;Height=1E-05;");
        assert_eq!(table.constants.len(), 3);
        assert_eq!(table.constants[2], ("Height".to_string(), 1.0e-5));
    }

    #[test]
    fn messages_lose_their_number() {
        assert_eq!(node_message("ERR234 - Undefined symbol: 'q'"), "Undefined symbol: 'q'");
    }
}
