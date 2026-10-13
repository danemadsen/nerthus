//! A tool's arguments: a JSON object, read leniently (numbers and
//! booleans may come as text), with problems answered as tool errors the
//! model can act on.

use serde_json::{Map, Value};

use crate::editor::Editor;

pub(crate) struct Arguments<'a> {
    members: &'a Map<String, Value>,
}

impl<'a> Arguments<'a> {
    pub fn of(value: &'a Value) -> Result<Self, String> {
        static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
        match value {
            Value::Object(members) => Ok(Self { members }),
            Value::Null => Ok(Self { members: EMPTY.get_or_init(Map::new) }),
            _ => Err("The arguments must be a JSON object.".to_string()),
        }
    }

    /// An argument given (null counts as not given).
    pub fn value(&self, name: &str) -> Option<&'a Value> {
        self.members.get(name).filter(|value| !value.is_null())
    }

    pub fn string(&self, name: &str) -> Result<Option<&'a str>, String> {
        match self.value(name) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.as_str())),
            Some(_) => Err(format!("{name} must be text.")),
        }
    }

    pub fn required_string(&self, name: &str) -> Result<&'a str, String> {
        let text = self.string(name)?;
        text.ok_or_else(|| format!("{name} is required."))
    }

    pub fn boolean(&self, name: &str) -> Result<Option<bool>, String> {
        let Some(value) = self.value(name) else {
            return Ok(None);
        };
        let flag = boolean_of(value).ok_or_else(|| format!("{name} must be true or false."))?;
        Ok(Some(flag))
    }

    pub fn number(&self, name: &str) -> Result<Option<f64>, String> {
        let Some(value) = self.value(name) else {
            return Ok(None);
        };
        let number = number_of(value).ok_or_else(|| format!("{name} must be a number."))?;
        Ok(Some(number))
    }

    pub fn integer(&self, name: &str) -> Result<Option<i64>, String> {
        let Some(number) = self.number(name)? else {
            return Ok(None);
        };
        if number.fract() != 0.0 {
            return Err(format!("{name} must be a whole number."));
        }
        Ok(Some(number as i64))
    }

    pub fn object(&self, name: &str) -> Result<Option<&'a Map<String, Value>>, String> {
        match self.value(name) {
            None => Ok(None),
            Some(Value::Object(members)) => Ok(Some(members)),
            Some(_) => Err(format!("{name} must be a JSON object.")),
        }
    }

    pub fn array(&self, name: &str) -> Result<Option<&'a Vec<Value>>, String> {
        match self.value(name) {
            None => Ok(None),
            Some(Value::Array(items)) => Ok(Some(items)),
            Some(_) => Err(format!("{name} must be a list.")),
        }
    }

    /// A canvas position: `[x, y]` or `{"x": …, "y": …}`.
    pub fn point(&self, name: &str) -> Result<Option<[f32; 2]>, String> {
        let Some(value) = self.value(name) else {
            return Ok(None);
        };
        let point = pair_of(value).ok_or_else(|| format!("{name} must be [x, y]."))?;
        Ok(Some([point[0] as f32, point[1] as f32]))
    }

    /// A node, by id or name.
    pub fn node(&self, name: &str, editor: &Editor) -> Result<i32, String> {
        let value = self.value(name).ok_or_else(|| format!("{name} is required."))?;
        find_node(editor, value)
    }

    pub fn optional_node(&self, name: &str, editor: &Editor) -> Result<Option<i32>, String> {
        let Some(value) = self.value(name) else {
            return Ok(None);
        };
        let node = find_node(editor, value)?;
        Ok(Some(node))
    }
}

/// A number, or text that reads as one.
pub(crate) fn number_of(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// `true`, `false`, their text, or 1 and 0.
pub(crate) fn boolean_of(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(flag) => Some(*flag),
        Value::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Some(true),
            "false" | "no" | "off" | "0" => Some(false),
            _ => None,
        },
        Value::Number(number) => match number.as_f64() {
            Some(1.0) => Some(true),
            Some(0.0) => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// Two numbers: `[a, b]`, or an object's `x` and `y` (any case), or its
/// `min` and `max`.
pub(crate) fn pair_of(value: &Value) -> Option<[f64; 2]> {
    match value {
        Value::Array(items) if items.len() == 2 => {
            let first = number_of(&items[0])?;
            let second = number_of(&items[1])?;
            Some([first, second])
        }
        Value::Object(members) => {
            let member = |names: &[&str]| names.iter().find_map(|name| members.get(*name)).and_then(number_of);
            let first = member(&["X", "x", "min", "low"])?;
            let second = member(&["Y", "y", "max", "high"])?;
            Some([first, second])
        }
        _ => None,
    }
}

/// A node by id (a number, or text holding one) or by its name, which must
/// be unique (any case).
pub(crate) fn find_node(editor: &Editor, value: &Value) -> Result<i32, String> {
    find_in(&editor.document, value)
}

/// [`find_node`] in any project.
pub(crate) fn find_in(document: &crate::document::Document, value: &Value) -> Result<i32, String> {
    find_in_terrain(document.terrain(), value)
}

/// [`find_node`] in any terrain.
pub(crate) fn find_in_terrain(terrain: &nerthus_engine::Terrain, value: &Value) -> Result<i32, String> {
    let nodes = &terrain.nodes;
    let id = match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => return Err("A node is given by its id (a number) or its name.".to_string()),
    };
    if let Some(id) = id {
        let id = id as i32;
        if nodes.contains_key(&id) {
            return Ok(id);
        }
        if value.is_number() {
            return Err(format!("No node has the id {id}; list_nodes shows the nodes."));
        }
    }
    let name = value.as_str().unwrap_or_default().trim();
    let named: Vec<i32> = nodes.values().filter(|node| node.name.eq_ignore_ascii_case(name)).map(|node| node.id).collect();
    match named.as_slice() {
        [single] => Ok(*single),
        [] => Err(format!("No node is named \"{name}\"; list_nodes shows the nodes.")),
        several => {
            let ids: Vec<String> = several.iter().map(i32::to_string).collect();
            Err(format!("Several nodes are named \"{name}\" (ids {}); give an id.", ids.join(", ")))
        }
    }
}
