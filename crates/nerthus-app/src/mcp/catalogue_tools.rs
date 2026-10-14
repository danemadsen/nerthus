//! The node catalogue for the model: finding node types, and what one
//! takes. Only the types the toolbox lists are offered.

use nerthus_engine::format_names::format_node_name;
use nerthus_engine::{Catalogue, ModifierSpec, NodeData, NodeSpec};
use serde_json::{json, Map, Value};

use super::arguments::Arguments;
use super::knowledge::{self, READS_NODE_INPUT};
use super::tools::MODIFIERS;
use super::{values, Step, ToolContext, ToolResult};
use crate::ui::toolbox;

/// How many types a search answers with at most.
const SEARCH_LIMIT: usize = 20;

fn listed_types() -> impl Iterator<Item = &'static NodeSpec> {
    Catalogue::get().nodes().filter(|spec| toolbox::listed(spec))
}

pub(crate) fn display_name(spec: &NodeSpec) -> &str {
    spec.display_name.as_deref().unwrap_or(&spec.name)
}

/// How many types each part of a summary or suggestion names at most.
const RELATED_LIMIT: usize = 5;

/// A node type the toolbox lists, by type or display name, any case, or
/// by the name `.terrain` files give it (`Erosion2` for ErosionPro).
pub(crate) fn find_type(name: &str) -> Result<&'static NodeSpec, String> {
    let wanted = name.trim();
    let matches = |spec: &&NodeSpec| {
        let key = spec.key();
        let format_name = format_node_name(key);
        key.eq_ignore_ascii_case(wanted) || format_name.eq_ignore_ascii_case(wanted) || spec.name.eq_ignore_ascii_case(wanted) || display_name(spec).eq_ignore_ascii_case(wanted)
    };
    // A type's own name before another's file name (Erosion is a type of
    // its own, not ErosionPro's).
    let exact = listed_types().find(|spec| spec.key().eq_ignore_ascii_case(wanted));
    if let Some(spec) = exact.or_else(|| listed_types().find(matches)) {
        return Ok(spec);
    }
    match similar_type(wanted) {
        Some(similar) => Err(format!("There is no node type \"{wanted}\"; did you mean {similar}? search_node_types finds them.")),
        None => Err(format!("There is no node type \"{wanted}\"; search_node_types finds them.")),
    }
}

/// Letters and digits only, lower case.
fn squeezed(text: &str) -> String {
    text.chars().filter(|character| character.is_alphanumeric()).collect::<String>().to_lowercase()
}

/// The edit distance between two words.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, left) in a.chars().enumerate() {
        let mut current = vec![i + 1];
        for (j, right) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(left != *right);
            current.push(substitution.min(previous[j + 1] + 1).min(current[j] + 1));
        }
        previous = current;
    }
    previous[b.len()]
}

/// The listed type whose name is nearest `wanted` (a plural, a typo, a
/// name one type stands in for), when one is near enough.
pub(crate) fn similar_type(wanted: &str) -> Option<&'static str> {
    let wanted = squeezed(wanted);
    if wanted.len() < 3 {
        return None;
    }
    let mut best: Option<(usize, &'static str)> = None;
    for spec in listed_types() {
        for name in [spec.key(), display_name(spec)] {
            let candidate = squeezed(name);
            let mut score = distance(&wanted, &candidate);
            if candidate.starts_with(&wanted) || wanted.starts_with(&candidate) {
                score = score.min(1);
            }
            if best.is_none_or(|(best_score, _)| score < best_score) {
                best = Some((score, spec.key()));
            }
        }
    }
    let limit = (wanted.len() / 4).clamp(1, 3);
    best.filter(|(score, _)| *score <= limit).map(|(_, key)| key)
}

/// A modifier type by name, or by the label of one of its parameters
/// ("Height Remap"), any case.
pub(crate) fn find_modifier(name: &str) -> Option<&'static ModifierSpec> {
    let wanted = squeezed(name);
    let catalogue = Catalogue::get();
    let by_name = MODIFIERS.iter().filter_map(|name| catalogue.modifier(name)).find(|spec| squeezed(&spec.name) == wanted || squeezed(&spec.type_name) == wanted);
    by_name.or_else(|| {
        let mut all = MODIFIERS.iter().filter_map(|name| catalogue.modifier(name));
        all.find(|spec| spec.parameters.iter().any(|parameter| squeezed(&parameter.display_name()) == wanted))
    })
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
    let usage = knowledge::usage();
    let mut parameters = Vec::new();
    for parameter in spec.parameters.iter().filter(|parameter| values::settable(parameter).is_ok()) {
        let default = spec.defaults.get(&parameter.name);
        let mut entry = values::describe(parameter, default);
        if let Some(chosen) = usage.parameter(spec.key(), &parameter.name) {
            entry.insert("in_examples".to_string(), chosen);
        }
        parameters.push(Value::Object(entry));
    }
    let mut description = json!({
        "type": spec.key(),
        "name": display_name(spec),
        "category": spec.toolbox,
        "family": spec.family,
        "description": spec.description,
        "inputs": inputs,
        "outputs": outputs,
        "generator": knowledge::is_generator(spec),
        "takes_mask": spec.is_maskable(),
        "adds_inputs": nerthus_engine::editing::can_add_ports(spec.key()),
        "requires_baking": spec.has_flag("RequiresBaking"),
        "parameters": parameters,
        "in_examples": examples_summary(spec.key()),
    });
    let format_name = format_node_name(spec.key());
    if format_name != spec.key() {
        description["file_type_name"] = json!(format_name);
    }
    if let Some(code) = &spec.short_code {
        description["short_code"] = json!(code);
    }
    let words = keywords(spec);
    if !words.is_empty() {
        description["keywords"] = json!(words.split_whitespace().collect::<Vec<_>>());
    }
    if spec.has_flag("RequiresBaking") {
        description["baking_note"] = json!("Bake it (Graph › Baking) before a Hybrid or Tiled build; a Standard build builds it as usual.");
    }
    if parameters_curve(spec) {
        description["curve_note"] = json!("Parameters with a curve move non-linearly: halfway along the slider is not half the effect.");
    }
    if let Some(measured) = knowledge::load_calibration().and_then(|calibration| knowledge::measured(&calibration, spec.key())) {
        description["measured"] = measured;
    }
    Ok(Step::Done(ToolResult::json(&description)))
}

fn parameters_curve(spec: &NodeSpec) -> bool {
    spec.parameters.iter().any(|parameter| parameter.curve.is_some() && values::settable(parameter).is_ok())
}

/// How the bundled examples use a type: in how many, and what usually
/// feeds it and what it usually feeds.
pub(crate) fn examples_summary(type_name: &str) -> Value {
    let usage = knowledge::usage();
    let fed_by: Vec<Value> = usage.fed_by(type_name).into_iter().take(RELATED_LIMIT).map(|(from, from_port, into, times)| json!({"type": from, "from_port": from_port, "into_port": into, "times": times})).collect();
    let feeds: Vec<Value> = usage.feeds(type_name).into_iter().take(RELATED_LIMIT).map(|(from_port, to, into, times)| json!({"type": to, "from_port": from_port, "into_port": into, "times": times})).collect();
    json!({
        "projects": usage.project_count(type_name),
        "of": knowledge::bundled_projects().len(),
        "usually_fed_by": fed_by,
        "usually_feeds": feeds,
    })
}

/// A modifier for the model: what it does to its node, and its parameters.
fn modifier_summary(spec: &ModifierSpec) -> Value {
    let parameters: Vec<Value> = spec.parameters.iter().map(|parameter| Value::Object(values::describe(parameter, spec.defaults.get(&parameter.name)))).collect();
    let reads_input = READS_NODE_INPUT.contains(&spec.type_name.as_str());
    let role = if reads_input {
        "Works off the node's input (a mask or a combiner): on a node nothing feeds it makes an empty or unchanged result."
    } else {
        "Changes the node's own output; works on generators too."
    };
    let mut summary = json!({"modifier": spec.name, "reads_node_input": reads_input, "role": role, "parameters": parameters});
    match spec.name.as_str() {
        "Min" => summary["excludes"] = json!("Max"),
        "Max" => summary["excludes"] = json!("Min"),
        _ => {}
    }
    summary
}

pub(crate) fn describe_modifier(arguments: &Arguments) -> Result<Step, String> {
    let wanted = arguments.string("modifier")?.map(str::trim).filter(|name| !name.is_empty() && *name != "*");
    let label = arguments.string("label")?.map(str::trim).filter(|label| !label.is_empty());
    let catalogue = Catalogue::get();
    let Some(wanted) = wanted.or(label) else {
        let all: Vec<Value> = MODIFIERS.iter().filter_map(|name| catalogue.modifier(name)).map(modifier_summary).collect();
        let listing = json!({"modifiers": all, "note": "set_modifier adds, sets and removes them."});
        return Ok(Step::Done(ToolResult::json(&listing)));
    };
    let spec = find_modifier(wanted).ok_or_else(|| format!("No modifier is named or has a parameter labelled \"{wanted}\"; the modifiers are {}.", MODIFIERS.join(", ")))?;
    let mut summary = modifier_summary(spec);
    let matched = spec.parameters.iter().find(|parameter| squeezed(&parameter.display_name()) == squeezed(wanted) && squeezed(&spec.name) != squeezed(wanted));
    if let Some(parameter) = matched {
        summary["matched_parameter"] = json!(parameter.name);
    }
    Ok(Step::Done(ToolResult::json(&summary)))
}

/// The types the open project (or `current_nodes`) uses.
fn current_types(arguments: &Arguments, context: &ToolContext) -> Result<Vec<String>, String> {
    if let Some(items) = arguments.array("current_nodes")? {
        let mut types = Vec::new();
        for item in items {
            let name = item.as_str().ok_or("current_nodes lists node types by name.")?;
            types.push(find_type(name).map(|spec| spec.key().to_string()).unwrap_or_else(|_| name.to_string()));
        }
        return Ok(types);
    }
    Ok(context.editor.document.terrain().nodes.values().map(|node| node.type_name.clone()).collect())
}

/// The most used types of a toolbox category in the examples (all of
/// them when the examples use none).
fn favourites(category: &str, family: Option<&str>) -> Vec<&'static str> {
    let usage = knowledge::usage();
    let in_category = |spec: &&NodeSpec| spec.toolbox.as_deref() == Some(category) && family.is_none_or(|family| spec.family.as_deref() == Some(family));
    let mut types: Vec<&'static NodeSpec> = listed_types().filter(in_category).collect();
    types.sort_by(|a, b| usage.project_count(b.key()).cmp(&usage.project_count(a.key())).then_with(|| a.key().cmp(b.key())));
    types.into_iter().take(3).map(NodeSpec::key).collect()
}

pub(crate) fn suggest_nodes(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let limit = arguments.integer("limit")?.unwrap_or(8).clamp(1, 30) as usize;
    let types = current_types(arguments, context)?;
    let has = |predicate: &dyn Fn(&NodeSpec) -> bool| types.iter().filter_map(|name| Catalogue::get().node(name)).any(predicate);
    let mut outcome = Map::new();

    // What usually follows the node asked about.
    if let Some(node) = arguments.optional_node("node", context.editor)? {
        let data = context.editor.document.node(node).ok_or("The node is gone.")?;
        let follows: Vec<Value> = knowledge::usage()
            .feeds(&data.type_name)
            .into_iter()
            .take(limit)
            .map(|(from_port, to, into, times)| json!({"type": to, "from_port": from_port, "into_port": into, "times_in_examples": times}))
            .collect();
        outcome.insert("after_node".to_string(), json!({"node": node, "type": data.type_name, "usually_followed_by": follows}));
    }

    // Stages a terrain usually has that this one lacks.
    let mut missing = Vec::new();
    if !has(&knowledge::is_generator) {
        let generators: Vec<&str> = favourites("Terrain", None).into_iter().chain(favourites("Primitive", None)).collect();
        missing.push(json!({"stage": "a generator to start the terrain from", "types": generators}));
    }
    if !has(&|spec| spec.family.as_deref() == Some("Erosion")) {
        missing.push(json!({"stage": "erosion, for natural detail", "types": favourites("Simulate", Some("Erosion"))}));
    }
    if !has(&|spec| spec.toolbox.as_deref() == Some("Colorize")) {
        missing.push(json!({"stage": "colour, for a texture", "types": favourites("Colorize", None)}));
    }
    let exported = arguments.array("current_nodes")?.is_none() && !nerthus_export::build::exporting_nodes(context.editor.document.terrain()).is_empty();
    if !exported && !has(&|spec| spec.classification.as_deref() == Some("Export")) {
        missing.push(json!({"stage": "an output: mark_for_export marks nodes for Build and Export", "types": []}));
    }
    outcome.insert("missing_stages".to_string(), json!(missing));

    // Types the words call for.
    if let Some(words) = arguments.string("context")?.map(str::trim).filter(|words| !words.is_empty()) {
        let mut scored: Vec<(u32, &NodeSpec)> = listed_types().filter(|spec| !types.iter().any(|name| name == spec.key())).map(|spec| (score(spec, words), spec)).filter(|(score, _)| *score > 0).collect();
        let usage = knowledge::usage();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| usage.project_count(b.1.key()).cmp(&usage.project_count(a.1.key()))).then_with(|| a.1.key().cmp(b.1.key())));
        let fitting: Vec<Value> = scored.iter().take(limit).map(|(_, spec)| type_summary(spec)).collect();
        outcome.insert("for_context".to_string(), json!(fitting));
    }
    outcome.insert("current_node_count".to_string(), json!(types.len()));
    Ok(Step::Done(ToolResult::json(&Value::Object(outcome))))
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
    fn file_names_and_near_misses_find_types() {
        assert_eq!(find_type("Erosion2").unwrap().key(), "ErosionPro");
        assert_eq!(find_type("SatMap").unwrap().key(), "SatelliteMap");
        assert_eq!(similar_type("Sediment"), Some("Sediments"));
        assert_eq!(similar_type("Rockmap"), Some("RockMap"));
        assert!(find_type("Montain").unwrap_err().contains("Mountain"));
        assert_eq!(similar_type("Zzzzzzzz"), None);
    }

    #[test]
    fn modifiers_are_found_by_name_or_label() {
        assert_eq!(find_modifier("multiplier").unwrap().name, "Multiplier");
        assert_eq!(find_modifier("Blur").unwrap().name, "Blur");
        assert_eq!(find_modifier("Height Remap").unwrap().name, "Multiplier");
        assert!(find_modifier("nonsense").is_none());
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
