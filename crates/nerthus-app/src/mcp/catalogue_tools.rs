//! The node catalogue for the model: finding node types, and what one
//! takes. Only the types the toolbox lists are offered.

use nerthus_engine::{Catalogue, NodeData, NodeSpec};
use serde_json::{json, Map, Value};

use super::arguments::Arguments;
use super::{values, Step, ToolResult};
use crate::ui::toolbox;

/// How many types a search answers with at most.
const SEARCH_LIMIT: usize = 20;

fn listed_types() -> impl Iterator<Item = &'static NodeSpec> {
    Catalogue::get().nodes().filter(|spec| toolbox::listed(spec))
}

pub(crate) fn display_name(spec: &NodeSpec) -> &str {
    spec.display_name.as_deref().unwrap_or(&spec.name)
}

/// A node type the toolbox lists, by type or display name, any case.
pub(crate) fn find_type(name: &str) -> Result<&'static NodeSpec, String> {
    let wanted = name.trim();
    let matches = |spec: &&NodeSpec| spec.key().eq_ignore_ascii_case(wanted) || spec.name.eq_ignore_ascii_case(wanted) || display_name(spec).eq_ignore_ascii_case(wanted);
    let mut found = listed_types().filter(matches);
    found.next().ok_or_else(|| format!("There is no node type \"{wanted}\"; search_node_types finds them."))
}

/// A description's first sentence.
fn first_sentence(description: Option<&str>) -> String {
    let text = description.unwrap_or_default().trim();
    let end = text.find(". ").map_or(text.len(), |end| end + 1);
    text[..end].to_string()
}

/// The words a type is found by: its keywords, as a list or as text.
fn keywords(spec: &NodeSpec) -> String {
    match &spec.keywords {
        Some(Value::Array(words)) => {
            let words: Vec<&str> = words.iter().filter_map(Value::as_str).collect();
            words.join(" ").to_lowercase()
        }
        Some(Value::String(words)) => words.to_lowercase(),
        _ => String::new(),
    }
}

/// How well a type matches a query, word by word: its name most, then its
/// keywords, family and category, then its description.
fn score(spec: &NodeSpec, query: &str) -> u32 {
    let key = spec.key().to_lowercase();
    let name = display_name(spec).to_lowercase();
    let keywords = keywords(spec);
    let family = spec.family.as_deref().unwrap_or_default().to_lowercase();
    let category = spec.toolbox.as_deref().unwrap_or_default().to_lowercase();
    let description = spec.description.as_deref().unwrap_or_default().to_lowercase();
    let mut total = 0;
    for word in query.to_lowercase().split_whitespace() {
        if key == word || name == word {
            total += 40;
        } else if key.contains(word) || name.contains(word) {
            total += 20;
        }
        if keywords.contains(word) {
            total += 15;
        }
        if family == word || category == word {
            total += 10;
        }
        if description.contains(word) {
            total += 5;
        }
    }
    total
}

fn type_summary(spec: &NodeSpec) -> Value {
    let mut entry = Map::new();
    entry.insert("type".to_string(), json!(spec.key()));
    let name = display_name(spec);
    if name != spec.key() {
        entry.insert("name".to_string(), json!(name));
    }
    entry.insert("category".to_string(), json!(spec.toolbox));
    entry.insert("description".to_string(), json!(first_sentence(spec.description.as_deref())));
    Value::Object(entry)
}

pub(crate) fn search_node_types(arguments: &Arguments) -> Result<Step, String> {
    let query = arguments.string("query")?.map(str::trim).filter(|query| !query.is_empty());
    let category = arguments.string("category")?;
    if query.is_none() && category.is_none() {
        let mut categories = Map::new();
        for name in toolbox::CATEGORIES {
            let in_category = listed_types().filter(|spec| spec.toolbox.as_deref() == Some(name));
            let types: Vec<&str> = in_category.map(NodeSpec::key).collect();
            if !types.is_empty() {
                categories.insert(name.to_string(), json!(types));
            }
        }
        let listing = json!({"categories": categories});
        return Ok(Step::Done(ToolResult::json(&listing)));
    }
    let in_category = |spec: &&NodeSpec| category.is_none_or(|category| spec.toolbox.as_deref().is_some_and(|toolbox| toolbox.eq_ignore_ascii_case(category)));
    let candidates = listed_types().filter(in_category);
    let mut scored: Vec<(u32, &NodeSpec)> = match query {
        Some(query) => candidates.map(|spec| (score(spec, query), spec)).filter(|(score, _)| *score > 0).collect(),
        None => candidates.map(|spec| (0, spec)).collect(),
    };
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.key().cmp(b.1.key())));
    let limit = if query.is_some() { SEARCH_LIMIT } else { scored.len() };
    let types: Vec<Value> = scored.iter().take(limit).map(|(_, spec)| type_summary(spec)).collect();
    let listing = json!({"types": types, "matches": scored.len()});
    Ok(Step::Done(ToolResult::json(&listing)))
}

pub(crate) fn describe_node_type(arguments: &Arguments) -> Result<Step, String> {
    let name = arguments.required_string("type")?;
    let spec = find_type(name)?;
    // A fresh node's ports, read as the graph reads them.
    let fresh = NodeData::new(spec, 0, 0);
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    for port in &fresh.ports {
        if port.is_input() {
            inputs.push(json!({"port": port.name, "required": port.is_required()}));
        } else {
            let kind = port.declared_intent.map(|intent| format!("{intent:?}"));
            outputs.push(json!({"port": port.name, "kind": kind}));
        }
    }
    let mut parameters = Vec::new();
    for parameter in spec.parameters.iter().filter(|parameter| values::settable(parameter).is_ok()) {
        let default = spec.defaults.get(&parameter.name);
        let entry = values::describe(parameter, default);
        parameters.push(Value::Object(entry));
    }
    let description = json!({
        "type": spec.key(),
        "name": display_name(spec),
        "category": spec.toolbox,
        "family": spec.family,
        "description": spec.description,
        "inputs": inputs,
        "outputs": outputs,
        "takes_mask": spec.is_maskable(),
        "adds_inputs": nerthus_engine::editing::can_add_ports(spec.key()),
        "requires_baking": spec.has_flag("RequiresBaking"),
        "parameters": parameters,
    });
    Ok(Step::Done(ToolResult::json(&description)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_are_found_by_name_and_words() {
        assert_eq!(find_type("mountain").unwrap().key(), "Mountain");
        assert!(find_type("NoSuchNode").is_err());
        let mountain = find_type("Mountain").unwrap();
        assert!(score(mountain, "mountain") > score(mountain, "voronoi"));
    }

    #[test]
    fn left_out_types_are_not_offered() {
        assert!(find_type("Unity").is_err());
    }

    #[test]
    fn descriptions_end_at_the_first_sentence() {
        assert_eq!(first_sentence(Some("One thing. Another thing.")), "One thing.");
        assert_eq!(first_sentence(None), "");
    }
}
