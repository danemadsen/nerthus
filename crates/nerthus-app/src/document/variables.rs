//! Variable edits from the Vars tab beyond their values: the Edit Variable
//! dialog (`EditVariableDialog.BtnOK_OnClick`), renaming,
//! a value typed over a row, and the variables nothing
//! is bound to (`DataViewArea.CheckForOrphanVariables`).

use nerthus_engine::automation::{VarType, Variable, VariableValue};
use nerthus_engine::json::f32_value;
use serde_json::{json, Map, Value};

use super::Document;

/// Names the dialog refuses, the letters formulas use.
pub const RESERVED_VARIABLE_NAMES: [&str; 7] = ["x", "y", "z", "w", "a", "b", "c"];

/// `Variable`'s members in declaration order, which Newtonsoft writes.
const VARIABLE_MEMBERS: [&str; 8] = ["Order", "Name", "Value", "Type", "Min", "Max", "Default", "ExtraData"];

/// `List<string>` as `TypeNameHandling.Auto` writes it into an `object`.
pub const STRING_LIST_TYPE: &str = "System.Collections.Generic.List`1[[System.String, System.Private.CoreLib]], System.Private.CoreLib";

/// What Edit Variable changes besides the name, by the variable's type.
#[derive(Clone, Debug, PartialEq)]
pub enum VariableLimits {
    /// Bool, Color and decorations: the name alone.
    None,
    /// Float and Range (`numMin.Value`, `numMax.Value`).
    Float { min: f32, max: f32 },
    /// Int (`Convert.ToInt32` of each box).
    Int { min: i32, max: i32 },
    /// Choice: the comma-separated list, split and trimmed.
    Choices(Vec<String>),
    /// Undefined: `cboStringType`'s index (Standard, Input, Output, Input
    /// (Optional), Output (Optional)).
    StringType(usize),
}

/// Why a name cannot be a variable's: one of the reserved letters or
/// shorter than two characters (`BtnOK_OnClick`), or another variable's
/// already, which the renamed variable would otherwise replace.
pub fn variable_name_problem(document: &Document, key: &str, name: &str) -> Option<String> {
    let reserved = RESERVED_VARIABLE_NAMES.contains(&name) || name.encode_utf16().count() < 2;
    if reserved {
        return Some("This variable name is reserved for internal use. Please use a different variable name.".to_string());
    }
    let taken = name != key && document.automation().variables.contains_key(name);
    if taken {
        return Some("Another variable already has this name. Please use a different variable name.".to_string());
    }
    None
}

/// Sets one of a variable's members; a new one goes where `Variable`
/// declares it.
fn set_variable_member(members: &mut Map<String, Value>, name: &str, value: Value) {
    if let Some(saved) = members.get_mut(name) {
        *saved = value;
        return;
    }
    let rank = |member: &str| VARIABLE_MEMBERS.iter().position(|candidate| *candidate == member);
    let wanted = rank(name);
    let later = members.keys().position(|member| matches!((rank(member), wanted), (Some(existing), Some(wanted)) if existing > wanted));
    match later {
        Some(index) => {
            members.shift_insert(index, name.to_string(), value);
        }
        None => {
            members.insert(name.to_string(), value);
        }
    }
}

/// The text `Value` `RecastToType` turns into the variable type's
/// value (the default when it cannot).
fn recast_text(variable: &Value, text: &str) -> Value {
    let mut typed = variable.clone();
    if let Value::Object(members) = &mut typed {
        members.insert("Value".to_string(), json!(text));
    }
    let recast = Variable::from_json(&typed);
    match recast.value {
        VariableValue::Float(number) => f32_value(number),
        other => other.to_json(),
    }
}

/// Edit Variable's limits written into the saved variable.
fn apply_limits(members: &mut Map<String, Value>, limits: &VariableLimits) {
    match limits {
        VariableLimits::None => {}
        VariableLimits::Float { min, max } => {
            set_variable_member(members, "Min", f32_value(*min));
            set_variable_member(members, "Max", f32_value(*max));
        }
        VariableLimits::Int { min, max } => {
            set_variable_member(members, "Min", json!(min));
            set_variable_member(members, "Max", json!(max));
        }
        VariableLimits::Choices(choices) => {
            // Only a list already there takes the new choices.
            let extra = members.get("ExtraData");
            let is_list = extra.is_some_and(|extra| extra.is_array() || extra.get("$values").is_some());
            if is_list {
                let list = json!({"$type": STRING_LIST_TYPE, "$values": choices});
                set_variable_member(members, "ExtraData", list);
            }
        }
        VariableLimits::StringType(index) => set_variable_member(members, "ExtraData", json!(index.to_string())),
    }
}

impl Document {
    /// The saved variable under `key`.
    fn variable_json_mut(&mut self, key: &str) -> Option<&mut Map<String, Value>> {
        let automation = self.automation_json_mut();
        let variable = automation.get_mut("Variables").and_then(|variables| variables.get_mut(key));
        match variable {
            Some(Value::Object(members)) => Some(members),
            _ => None,
        }
    }

    /// Renames a variable without an undo step: the variable takes `name` as
    /// its key too, keeping its place among the variables, and the
    /// bindings to it follow. Expressions keep the name they were written
    /// with.
    fn rename_variable_quietly(&mut self, key: &str, name: &str) {
        let automation = self.automation_json_mut();
        if let Some(Value::Object(variables)) = automation.get_mut("Variables") {
            let Some(index) = variables.keys().position(|candidate| candidate == key) else {
                return;
            };
            let Some(mut variable) = variables.shift_remove(key) else {
                return;
            };
            if let Value::Object(members) = &mut variable {
                set_variable_member(members, "Name", json!(name));
            }
            variables.shift_insert(index, name.to_string(), variable);
        }
        if let Some(Value::Array(bindings)) = automation.get_mut("Bindings") {
            for binding in bindings.iter_mut() {
                let bound_here = binding.get("Variable").and_then(Value::as_str) == Some(key);
                if bound_here && let Value::Object(members) = binding {
                    members.insert("Variable".to_string(), json!(name));
                }
            }
        }
    }

    /// The variable a renamed node offers to rename after itself: when
    /// exactly one variable is bound to the node, its key and the node's
    /// name, or `Name_Id` when another variable has that name. None when
    /// the variable already has the name, or when `Name_Id` is taken too
    /// (the rename would replace that variable).
    pub fn bound_variable_rename(&self, node: i32) -> Option<(String, String)> {
        let name = self.node(node)?.name.clone();
        let automation = self.automation();
        let mut bindings = automation.bindings.iter().filter(|binding| binding.node == node);
        let binding = bindings.next()?;
        if bindings.next().is_some() {
            return None;
        }
        let key = binding.variable.clone();
        if key == name || !automation.variables.contains_key(&key) {
            return None;
        }
        let target = if automation.variables.contains_key(&name) { format!("{name}_{node}") } else { name };
        if automation.variables.contains_key(&target) {
            return None;
        }
        Some((key, target))
    }

    /// A name typed over a row's label (`EditablePopup_EditSuccess`):
    /// "Variable X renamed to Y".
    pub fn rename_variable(&mut self, key: &str, name: &str) {
        let known = self.automation().variables.contains_key(key);
        if !known || key == name {
            return;
        }
        self.checkpoint(format!("Variable {key} renamed to {name}"));
        self.rename_variable_quietly(key, name);
        self.refresh_automation();
    }

    /// Edit Variable's Accept Changes: the limits for the variable's type,
    /// then the new name (one undo step, named after the rename when there
    /// is one). Range variables keep their limits too. Returns the
    /// variable's key afterwards.
    pub fn edit_variable(&mut self, key: &str, name: &str, limits: &VariableLimits) -> String {
        let Some(variable) = self.automation().variables.get(key) else {
            return key.to_string();
        };
        let original = variable.json.clone();
        let mut edited = original.clone();
        if let Value::Object(members) = &mut edited {
            apply_limits(members, limits);
        }
        let renamed = name != key;
        if !renamed && edited == original {
            return key.to_string();
        }
        let label = if renamed { format!("Variable {key} renamed to {name}") } else { format!("Variable {key} edited") };
        self.checkpoint(label);
        if let (Some(saved), Value::Object(members)) = (self.variable_json_mut(key), edited) {
            *saved = members;
        }
        if renamed {
            self.rename_variable_quietly(key, name);
        }
        self.refresh_automation();
        name.to_string()
    }

    /// A value typed over a row: "Variable X value
    /// changed to Y", the text taking the variable's type. Every node
    /// bound to it rebuilds.
    pub fn set_variable_text(&mut self, key: &str, text: &str) {
        let Some(variable) = self.automation().variables.get(key) else {
            return;
        };
        let value = recast_text(&variable.json, text);
        self.checkpoint(format!("Variable {key} value changed to {text}"));
        if let Some(members) = self.variable_json_mut(key) {
            set_variable_member(members, "Value", value);
        }
        self.refresh_automation();
        let bound: Vec<i32> = self.automation().bindings.iter().filter(|binding| binding.variable == key).map(|binding| binding.node).collect();
        for node in bound {
            self.mark_stale(node);
        }
    }

    /// `CheckForOrphanVariables`: the variables (not decorations) no
    /// binding names, by key.
    pub fn orphan_variables(&self) -> Vec<String> {
        let automation = self.automation();
        let unbound = |variable: &Variable| automation.bindings.iter().all(|binding| binding.variable != variable.name);
        let orphans = automation.variables.iter().filter(|(_, variable)| variable.var_type != VarType::Decoration && unbound(variable));
        orphans.map(|(key, _)| key.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_members_go_where_variable_declares_them() {
        let mut members = Map::new();
        members.insert("Name".to_string(), json!("Var_1"));
        members.insert("Value".to_string(), json!(0.5));
        members.insert("ExtraData".to_string(), json!("1"));
        set_variable_member(&mut members, "Max", json!(1.0));
        set_variable_member(&mut members, "Type", json!("Float"));
        set_variable_member(&mut members, "Min", json!(0.0));
        set_variable_member(&mut members, "Order", json!(3));
        let order: Vec<&str> = members.keys().map(String::as_str).collect();
        assert_eq!(order, ["Order", "Name", "Value", "Type", "Min", "Max", "ExtraData"]);
    }

    #[test]
    fn a_renamed_node_offers_its_one_variable_its_name() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        assert_eq!(document.bound_variable_rename(mountain), None);
        document.bind_new_variable(mountain, "Height");
        document.rename(mountain, "Peak");
        assert_eq!(document.bound_variable_rename(mountain), Some(("Height".to_string(), "Peak".to_string())));

        // Another variable has the name: `Name_Id`, unless that is taken too.
        let other = document.add_variable("Float");
        document.rename_variable(&other, "Peak");
        let fallback = format!("Peak_{mountain}");
        assert_eq!(document.bound_variable_rename(mountain), Some(("Height".to_string(), fallback.clone())));
        let another = document.add_variable("Float");
        document.rename_variable(&another, &fallback);
        assert_eq!(document.bound_variable_rename(mountain), None);

        // Named after the node already: nothing to offer.
        document.rename(mountain, "Height");
        assert_eq!(document.bound_variable_rename(mountain), None);
        // Two variables bound: nothing either.
        document.rename(mountain, "Crest");
        assert!(document.bound_variable_rename(mountain).is_some());
        document.bind_new_variable(mountain, "Scale");
        assert_eq!(document.bound_variable_rename(mountain), None);
    }

    #[test]
    fn typed_values_take_the_variable_type() {
        let int = json!({"Name": "Count", "Value": 1, "Type": "Int", "Min": 0, "Max": 10});
        assert_eq!(recast_text(&int, "7"), json!(7));
        let float = json!({"Name": "Amount", "Value": 0.5, "Type": "Float", "Min": 0.0, "Max": 1.0});
        assert_eq!(recast_text(&float, "0.25"), json!(0.25));
        let text = json!({"Name": "Path", "Value": "a"});
        assert_eq!(recast_text(&text, "b"), json!("b"));
    }
}
