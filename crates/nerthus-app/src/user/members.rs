//! The settings files are Newtonsoft JSON written with
//! `DefaultValueHandling.IgnoreAndPopulate`, `PreserveReferencesHandling.All`
//! and enums as names: a member at its default is left
//! out, and a member a file lacks takes that default when read (the
//! `[DefaultValue]`, else the type's own default, not what the constructor
//! sets). Objects carry `$id`s, lists `$values`. Each settings class is a
//! table of its members in declaration order.

use serde_json::{Map, Value};

/// What a member holds.
#[derive(Clone, Copy)]
pub enum Kind {
    /// A number, flag, text or enum name; also a struct such as `Color`.
    Value,
    /// A `List<string>`, written as an object with `$id` and `$values`.
    List,
    /// A dictionary of names (`NodeDisplay`), written with an `$id`; a
    /// file's entries go over the ones the constructor adds.
    Dictionary,
    /// A nested settings object with its own members.
    Object(&'static [Member]),
}

/// One member of a settings class.
pub struct Member {
    pub name: &'static str,
    pub kind: Kind,
    /// What a new instance holds (JSON text).
    pub initial: &'static str,
    /// What Newtonsoft leaves out when writing and fills in when a file
    /// lacks the member (JSON text).
    pub omitted: &'static str,
}

/// A plain member.
pub const fn value(name: &'static str, initial: &'static str, omitted: &'static str) -> Member {
    Member { name, kind: Kind::Value, initial, omitted }
}

/// A list of strings (never left out: its default is `null`).
pub const fn list(name: &'static str, initial: &'static str) -> Member {
    Member { name, kind: Kind::List, initial, omitted: "null" }
}

/// A dictionary of names.
pub const fn dictionary(name: &'static str, initial: &'static str) -> Member {
    Member { name, kind: Kind::Dictionary, initial, omitted: "null" }
}

/// A nested object.
pub const fn object(name: &'static str, members: &'static [Member]) -> Member {
    Member { name, kind: Kind::Object(members), initial: "null", omitted: "null" }
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

/// A new instance: every member as the constructor leaves it.
pub fn initial_values(members: &[Member]) -> Map<String, Value> {
    let mut values = Map::new();
    for member in members {
        let initial = match member.kind {
            Kind::Object(nested) => Value::Object(initial_values(nested)),
            _ => parse(member.initial),
        };
        values.insert(member.name.to_string(), initial);
    }
    values
}

/// Reads a saved object: the members it has, the rest at their default
/// (`Populate`). Nested objects and lists start afresh when a file lacks
/// them.
pub fn read(members: &[Member], saved: &Value) -> Map<String, Value> {
    let mut values = Map::new();
    for member in members {
        let found = saved.get(member.name);
        let read_value = match (member.kind, found) {
            (Kind::Object(nested), Some(found)) => Value::Object(read(nested, found)),
            (Kind::Object(nested), None) => Value::Object(initial_values(nested)),
            (Kind::List, Some(found)) => read_list(found),
            (Kind::List, None) => parse(member.initial),
            (Kind::Dictionary, found) => read_dictionary(member, found),
            (Kind::Value, Some(found)) => found.clone(),
            (Kind::Value, None) => parse(member.omitted),
        };
        values.insert(member.name.to_string(), read_value);
    }
    values
}

/// A list as written (`{"$id", "$values"}`) or plain.
fn read_list(saved: &Value) -> Value {
    let items = saved.get("$values").unwrap_or(saved);
    match items {
        Value::Array(items) => Value::Array(items.clone()),
        _ => Value::Array(Vec::new()),
    }
}

/// `ObjectCreationHandling.Replace`: a new dictionary (with the
/// constructor's entries), then the file's entries over them.
fn read_dictionary(member: &Member, saved: Option<&Value>) -> Value {
    let mut entries = match parse(member.initial) {
        Value::Object(entries) => entries,
        _ => Map::new(),
    };
    if let Some(Value::Object(saved_entries)) = saved {
        for (key, entry) in saved_entries {
            if key.starts_with('$') {
                continue;
            }
            entries.insert(key.clone(), entry.clone());
        }
    }
    Value::Object(entries)
}

/// Writes an object as Newtonsoft does: its `$id` (numbered in the order
/// objects are met), then the members in declaration order, leaving out
/// nulls and those at their default.
pub fn write(members: &[Member], values: &Map<String, Value>, next_id: &mut usize) -> Value {
    let mut written = Map::new();
    written.insert("$id".to_string(), Value::String(next_id.to_string()));
    *next_id += 1;
    for member in members {
        let Some(current) = values.get(member.name) else {
            continue;
        };
        if current.is_null() {
            continue;
        }
        let saved = match member.kind {
            Kind::Object(nested) => {
                let nested_values = current.as_object().cloned().unwrap_or_default();
                write(nested, &nested_values, next_id)
            }
            Kind::List => {
                let mut list = Map::new();
                list.insert("$id".to_string(), Value::String(next_id.to_string()));
                *next_id += 1;
                list.insert("$values".to_string(), current.clone());
                Value::Object(list)
            }
            Kind::Dictionary => {
                let mut dictionary = Map::new();
                dictionary.insert("$id".to_string(), Value::String(next_id.to_string()));
                *next_id += 1;
                if let Value::Object(entries) = current {
                    for (key, entry) in entries {
                        dictionary.insert(key.clone(), entry.clone());
                    }
                }
                Value::Object(dictionary)
            }
            Kind::Value => {
                let omitted = parse(member.omitted);
                if same(current, &omitted) {
                    continue;
                }
                current.clone()
            }
        };
        written.insert(member.name.to_string(), saved);
    }
    Value::Object(written)
}

/// `MiscellaneousUtils.ValueEquals`: numbers by value whatever their type,
/// structs member by member.
pub fn same(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => left.as_f64() == right.as_f64(),
        (Value::Object(left), Value::Object(right)) => {
            let same_keys = left.len() == right.len();
            same_keys && left.iter().all(|(key, value)| right.get(key).is_some_and(|other| same(value, other)))
        }
        (Value::Array(left), Value::Array(right)) => left.len() == right.len() && left.iter().zip(right).all(|(left, right)| same(left, right)),
        _ => left == right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NESTED: [Member; 2] = [value("Port", "35771", "0"), list("KnownWorkers", "[]")];
    const MEMBERS: [Member; 6] = [
        value("Version", "5", "5"),
        value("Scale", "1.2", "0"),
        value("Shown", "true", "true"),
        dictionary("Sizes", "{\"Combine\":\"Small\"}"),
        object("Network", &NESTED),
        value("Colour", "{\"R\":0.5,\"G\":0.5,\"B\":0.5}", "{\"R\":0.0,\"G\":0.0,\"B\":0.0}"),
    ];

    #[test]
    fn writes_like_newtonsoft() {
        let mut values = initial_values(&MEMBERS);
        values.insert("Shown".to_string(), json!(false));
        let mut next_id = 1;
        let written = write(&MEMBERS, &values, &mut next_id);
        let expected = json!({
            "$id": "1",
            "Scale": 1.2,
            "Shown": false,
            "Sizes": {"$id": "2", "Combine": "Small"},
            "Network": {"$id": "3", "Port": 35771, "KnownWorkers": {"$id": "4", "$values": []}},
            "Colour": {"R": 0.5, "G": 0.5, "B": 0.5},
        });
        assert_eq!(written, expected);
    }

    #[test]
    fn missing_members_take_their_defaults() {
        let saved = json!({"$id": "1", "Sizes": {"$id": "2", "Clip": "Compact"}, "Network": {"$id": "3", "KnownWorkers": ["a"]}});
        let values = read(&MEMBERS, &saved);
        // `[DefaultValue]` or the type's default, not the constructor's.
        assert_eq!(values["Scale"], json!(0));
        assert_eq!(values["Shown"], json!(true));
        assert_eq!(values["Sizes"], json!({"Combine": "Small", "Clip": "Compact"}));
        assert_eq!(values["Network"], json!({"Port": 0, "KnownWorkers": ["a"]}));
        assert_eq!(values["Colour"], json!({"R": 0.0, "G": 0.0, "B": 0.0}));
    }

    #[test]
    fn numbers_compare_by_value() {
        assert!(same(&json!(1), &json!(1.0)));
        assert!(!same(&json!(1), &json!(true)));
        assert!(same(&json!({"R": 0, "G": 0.0}), &json!({"G": 0, "R": 0.0})));
    }
}
