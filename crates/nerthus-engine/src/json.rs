//! Reading project JSON. `.terrain` files are written with Newtonsoft.Json,
//! whose reader is lenient (trailing commas, comments, single quotes, bare
//! `NaN`/`Infinity`), and some existing files rely on that. The text is first
//! normalised to strict JSON, then parsed with serde_json.

use std::collections::HashMap;

use serde_json::Value;

/// Parses JSON the way Newtonsoft's `JsonTextReader` accepts it.
pub fn parse_lenient(text: &str) -> Result<Value, serde_json::Error> {
    let strict = normalise(text);
    serde_json::from_str(&strict)
}

/// Rewrites Newtonsoft-only syntax into strict JSON.
fn normalise(text: &str) -> String {
    let without_bom = text.strip_prefix('\u{feff}').unwrap_or(text);
    let characters: Vec<char> = without_bom.chars().collect();
    let mut output = String::with_capacity(without_bom.len());
    let mut index = 0;

    while index < characters.len() {
        let character = characters[index];

        if character == '"' || character == '\'' {
            index = copy_string(&characters, index, &mut output);
            continue;
        }

        if character == '/' && index + 1 < characters.len() {
            let next = characters[index + 1];
            if next == '/' {
                index = skip_line_comment(&characters, index);
                continue;
            }
            if next == '*' {
                index = skip_block_comment(&characters, index);
                continue;
            }
        }

        if character == ',' && closes_after(&characters, index + 1) {
            index += 1;
            continue;
        }

        if (character.is_ascii_alphabetic() || character == '-')
            && let Some((replacement, length)) = special_number(&characters, index) {
                output.push_str(replacement);
                index += length;
                continue;
            }

        output.push(character);
        index += 1;
    }

    output
}

/// Copies a quoted string starting at `start`, turning single quotes into
/// double quotes. Returns the index after the closing quote.
fn copy_string(characters: &[char], start: usize, output: &mut String) -> usize {
    let quote = characters[start];
    output.push('"');
    let mut index = start + 1;
    while index < characters.len() {
        let character = characters[index];
        if character == '\\' && index + 1 < characters.len() {
            let escaped = characters[index + 1];
            // `\'` is only meaningful inside single-quoted strings.
            if escaped == '\'' {
                output.push('\'');
            } else {
                output.push('\\');
                output.push(escaped);
            }
            index += 2;
            continue;
        }
        if character == quote {
            output.push('"');
            return index + 1;
        }
        if character == '"' {
            output.push_str("\\\"");
        } else {
            output.push(character);
        }
        index += 1;
    }
    output.push('"');
    index
}

fn skip_line_comment(characters: &[char], start: usize) -> usize {
    let mut index = start;
    while index < characters.len() && characters[index] != '\n' {
        index += 1;
    }
    index
}

fn skip_block_comment(characters: &[char], start: usize) -> usize {
    let mut index = start + 2;
    while index + 1 < characters.len() {
        if characters[index] == '*' && characters[index + 1] == '/' {
            return index + 2;
        }
        index += 1;
    }
    characters.len()
}

/// Whether only whitespace and comments separate `start` from a closing
/// bracket (a trailing comma).
fn closes_after(characters: &[char], start: usize) -> bool {
    let mut index = start;
    while index < characters.len() {
        let character = characters[index];
        if character.is_whitespace() {
            index += 1;
            continue;
        }
        if character == '/' && index + 1 < characters.len() {
            if characters[index + 1] == '/' {
                index = skip_line_comment(characters, index);
                continue;
            }
            if characters[index + 1] == '*' {
                index = skip_block_comment(characters, index);
                continue;
            }
        }
        return character == '}' || character == ']';
    }
    false
}

/// Bare `NaN`, `Infinity` and `-Infinity` become the strings Newtonsoft
/// itself writes for them.
fn special_number(characters: &[char], start: usize) -> Option<(&'static str, usize)> {
    const SPECIALS: [(&str, &str); 3] = [("-Infinity", "\"-Infinity\""), ("Infinity", "\"Infinity\""), ("NaN", "\"NaN\"")];
    if start > 0 && (characters[start - 1].is_ascii_alphanumeric() || characters[start - 1] == '_') {
        return None;
    }
    for (token, replacement) in SPECIALS {
        let length = token.chars().count();
        if start + length > characters.len() {
            continue;
        }
        let candidate: String = characters[start..start + length].iter().collect();
        let after = characters.get(start + length);
        let ends_token = after.is_none_or(|character| !(character.is_ascii_alphanumeric() || *character == '_'));
        if candidate == token && ends_token {
            return Some((replacement, length));
        }
    }
    None
}

/// Newtonsoft's reference bookkeeping (`PreserveReferencesHandling.All`):
/// every object or array may carry a `$id`, and `{"$ref": id}` stands for
/// the object with that id.
pub struct References<'a> {
    by_id: HashMap<String, &'a Value>,
}

impl<'a> References<'a> {
    pub fn collect(root: &'a Value) -> Self {
        let mut by_id = HashMap::new();
        collect_ids(root, &mut by_id);
        Self { by_id }
    }

    /// The object a value stands for: itself, or the target of its `$ref`.
    pub fn resolve(&self, value: &'a Value) -> &'a Value {
        let Some(reference) = value.get("$ref").and_then(Value::as_str) else {
            return value;
        };
        self.by_id.get(reference).copied().unwrap_or(value)
    }

    pub fn get(&self, id: &str) -> Option<&'a Value> {
        self.by_id.get(id).copied()
    }
}

fn collect_ids<'a>(value: &'a Value, by_id: &mut HashMap<String, &'a Value>) {
    match value {
        Value::Object(members) => {
            if let Some(id) = members.get("$id").and_then(Value::as_str) {
                by_id.insert(id.to_string(), value);
            }
            for member in members.values() {
                collect_ids(member, by_id);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_ids(item, by_id);
            }
        }
        _ => {}
    }
}

/// The elements of a Newtonsoft collection, which is written either as a
/// plain array or, with references preserved, as `{"$id", "$values": [...]}`.
pub fn collection_items(value: &Value) -> &[Value] {
    if let Some(items) = value.as_array() {
        return items;
    }
    let values = value.get("$values").and_then(Value::as_array);
    values.map(Vec::as_slice).unwrap_or(&[])
}

/// The class a `$type` names, without its namespace or assembly (`Canyon`
/// for `<namespace>.Canyon, <assembly>`). Types are matched on this alone,
/// so files whose types carry another namespace read the same.
pub fn type_class_name(dotnet_type: &str) -> &str {
    let type_path = dotnet_type.split(',').next().unwrap_or(dotnet_type).trim();
    type_path.rsplit('.').next().unwrap_or(type_path)
}

/// Removes Newtonsoft's bookkeeping members (`$id`, `$type`, …) from a copy
/// of an object tree, keeping `$values` contents as plain arrays.
pub fn strip_bookkeeping(value: &Value) -> Value {
    match value {
        Value::Object(members) => {
            if let Some(values) = members.get("$values") {
                return strip_bookkeeping(values);
            }
            let mut stripped = serde_json::Map::new();
            for (key, member) in members {
                if key == "$id" || key == "$type" {
                    continue;
                }
                stripped.insert(key.clone(), strip_bookkeeping(member));
            }
            Value::Object(stripped)
        }
        Value::Array(items) => Value::Array(items.iter().map(strip_bookkeeping).collect()),
        other => other.clone(),
    }
}

/// Removes reference ids (`$id`) from a copy of an object tree, keeping
/// `$type`: plain collections become arrays, typed ones stay
/// `{"$type", "$values"}`.
pub fn strip_reference_ids(value: &Value) -> Value {
    match value {
        Value::Object(members) => {
            let has_type = members.contains_key("$type");
            if let Some(values) = members.get("$values")
                && !has_type {
                    return strip_reference_ids(values);
                }
            let mut stripped = serde_json::Map::new();
            for (key, member) in members {
                if key == "$id" {
                    continue;
                }
                stripped.insert(key.clone(), strip_reference_ids(member));
            }
            Value::Object(stripped)
        }
        Value::Array(items) => Value::Array(items.iter().map(strip_reference_ids).collect()),
        other => other.clone(),
    }
}

/// A `float` member as Newtonsoft writes it: its shortest round-trip
/// digits (`0.3`, not `0.30000001192092896`), read back as the same f32.
pub fn f32_value(value: f32) -> Value {
    let digits = value.to_string();
    let number = digits.parse::<f64>().unwrap_or(value as f64);
    Value::from(number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_newtonsoft_leniency() {
        let text = "\u{feff}{ 'a': 1, // comment\n \"b\": [1, 2,], /* block */ \"c\": NaN, \"d\": -Infinity, }";
        let value = parse_lenient(text).expect("lenient JSON parses");
        assert_eq!(value["a"], 1);
        assert_eq!(value["b"], serde_json::json!([1, 2]));
        assert_eq!(value["c"], "NaN");
        assert_eq!(value["d"], "-Infinity");
    }

    #[test]
    fn strings_are_left_alone() {
        let value = parse_lenient(r#"{"text": "a, ] // not a comment", "quote": 'it\'s'}"#).expect("parses");
        assert_eq!(value["text"], "a, ] // not a comment");
        assert_eq!(value["quote"], "it's");
    }

    #[test]
    fn references_resolve() {
        let value = parse_lenient(r#"{"$id": "1", "a": {"$id": "2", "x": 3}, "b": {"$ref": "2"}}"#).expect("parses");
        let references = References::collect(&value);
        assert_eq!(references.resolve(&value["b"])["x"], 3);
    }
}
