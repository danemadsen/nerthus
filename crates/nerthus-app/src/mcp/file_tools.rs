//! Project files outside the editor: a project's text (the open one as it
//! would be saved, or a file's), the projects in a folder, and a file
//! repaired or changed in place without opening it in the editor, each
//! change checked as the editor checks it.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use base64::Engine as _;
use serde_json::{json, Value};

use super::arguments::{boolean_of, find_in, Arguments};
use super::authoring_tools::{build_members, example_files, parse_format};
use super::build_tools::format_key;
use super::edit_tools::converted_parameters;
use super::review_tools::{check_document, fix_document};
use super::{Step, ToolContext, ToolResult};
use crate::document::Document;
use crate::editor::Editor;

/// How many files a folder listing names at most.
const LISTING_LIMIT: usize = 500;

/// A `.terrain` file given by absolute path, which must exist.
fn terrain_file(given: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(given.trim());
    if path.is_relative() {
        return Err("path must be absolute.".to_string());
    }
    let is_terrain = path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("terrain"));
    if !is_terrain {
        return Err(format!("{} is not a .terrain project.", path.display()));
    }
    if !path.is_file() {
        return Err(format!("There is no file at {}.", path.display()));
    }
    Ok(path)
}

/// File tools leave the open project's file to the editor's own tools.
fn refuse_open_file(editor: &Editor, path: &Path, instead: &str) -> Result<(), String> {
    let open = editor.document.path.as_deref().and_then(|open| open.canonicalize().ok());
    if open.is_some() && open == path.canonicalize().ok() {
        return Err(format!("{} is the project open in the editor; {instead}, then save_project.", path.display()));
    }
    Ok(())
}

/// A copy of the file beside it, `name.terrain.backup`.
fn back_up(path: &Path) -> Result<PathBuf, String> {
    let backup = path.with_extension("terrain.backup");
    std::fs::copy(path, &backup).map_err(|error| format!("The backup {} cannot be written: {error}", backup.display()))?;
    Ok(backup)
}

fn modified_text(path: &Path) -> Option<String> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let since = modified.duration_since(UNIX_EPOCH).ok()?;
    let time = chrono::DateTime::from_timestamp(since.as_secs() as i64, 0)?;
    Some(time.with_timezone(&chrono::Local).to_rfc3339())
}

fn file_entry(path: &Path) -> Value {
    let bytes = std::fs::metadata(path).map(|data| data.len()).ok();
    json!({"name": path.file_name().map(|name| name.to_string_lossy().into_owned()), "path": path, "bytes": bytes, "modified": modified_text(path)})
}

pub(crate) fn get_project_file(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let encoding = arguments.string("encoding")?.unwrap_or("raw");
    if !matches!(encoding, "raw" | "base64") {
        return Err("encoding is raw or base64.".to_string());
    }
    let (name, path, bytes, unsaved) = match arguments.string("path")? {
        Some(given) => {
            let path = terrain_file(given)?;
            let bytes = std::fs::read(&path).map_err(|error| format!("{} cannot be read: {error}", path.display()))?;
            let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            (name, Some(path), bytes, None)
        }
        None => {
            let document = &context.editor.document;
            let text = nerthus_engine::to_terrain_json(&document.project);
            (document.file_name(), document.path.clone(), text.into_bytes(), Some(document.dirty))
        }
    };
    let mut outcome = json!({"name": name, "file": path, "bytes": bytes.len(), "encoding": encoding});
    if let Some(unsaved) = unsaved {
        outcome["open_project"] = json!(true);
        outcome["unsaved_changes"] = json!(unsaved);
    }
    if encoding == "base64" {
        outcome["content_base64"] = json!(base64::engine::general_purpose::STANDARD.encode(&bytes));
    } else {
        outcome["content"] = json!(String::from_utf8_lossy(&bytes));
    }
    Ok(Step::Done(ToolResult::json(&outcome)))
}

/// The `.terrain` files in `folder` (and its folders when `recursive`),
/// newest first.
fn terrain_files(folder: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(current) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if recursive {
                    pending.push(path);
                }
            } else if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("terrain")) {
                found.push(path);
            }
        }
    }
    let modified = |path: &PathBuf| std::fs::metadata(path).and_then(|data| data.modified()).ok();
    found.sort_by_key(|path| std::cmp::Reverse(modified(path)));
    found
}

pub(crate) fn list_projects(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let recursive = arguments.boolean("recursive")?.unwrap_or(false);
    if let Some(given) = arguments.string("directory")? {
        let folder = PathBuf::from(given.trim());
        if folder.is_relative() || !folder.is_dir() {
            return Err(format!("There is no folder at {} (give an absolute path).", folder.display()));
        }
        let files = terrain_files(&folder, recursive);
        let listed: Vec<Value> = files.iter().take(LISTING_LIMIT).map(|path| file_entry(path)).collect();
        let outcome = json!({"directory": folder, "count": files.len(), "files": listed});
        return Ok(Step::Done(ToolResult::json(&outcome)));
    }
    let editor = &*context.editor;
    let recent: Vec<Value> = editor.user.recent.files().iter().filter(|path| path.is_file()).map(|path| file_entry(path)).collect();
    let mut outcome = json!({"recent": recent});
    if let Some(folder) = editor.document.path.as_deref().and_then(Path::parent) {
        let files = terrain_files(folder, recursive);
        let listed: Vec<Value> = files.iter().take(LISTING_LIMIT).map(|path| file_entry(path)).collect();
        outcome["open_project_folder"] = json!({"directory": folder, "files": listed});
    }
    let examples: Vec<String> = example_files().iter().filter_map(|path| path.file_stem().map(|stem| stem.to_string_lossy().into_owned())).collect();
    outcome["examples"] = json!(examples);
    Ok(Step::Done(ToolResult::json(&outcome)))
}

/// The members a file's nodes carry before it is read: each node's
/// `Version` and whether it keeps `SaveDefinitions`, by id.
fn raw_nodes(text: &str) -> Vec<(i64, Option<i64>, bool)> {
    let Ok(document) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else {
        return Vec::new();
    };
    let nodes = document.pointer("/Assets/$values/0/Terrain/Nodes").and_then(Value::as_object);
    nodes
        .into_iter()
        .flatten()
        .filter(|(key, _)| !key.starts_with('$'))
        .filter_map(|(_, node)| {
            let id = node.get("Id").and_then(Value::as_i64)?;
            Some((id, node.get("Version").and_then(Value::as_i64), node.get("SaveDefinitions").is_some()))
        })
        .collect()
}

pub(crate) fn repair_project_file(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let path = terrain_file(arguments.required_string("path")?)?;
    refuse_open_file(context.editor, &path, "validate_graph with fix: true repairs it")?;
    let backup_wanted = arguments.boolean("create_backup")?.unwrap_or(true);
    let write = arguments.boolean("write")?.unwrap_or(true);
    let drop_unknown = arguments.boolean("drop_unknown_nodes")?.unwrap_or(false);
    let text = std::fs::read_to_string(&path).map_err(|error| format!("{} cannot be read: {error}", path.display()))?;
    let before = raw_nodes(&text);
    let mut document = Document::open(&path).map_err(|error| format!("{error:#}"))?;

    // What reading it already put right.
    let mut repairs = Vec::new();
    if nerthus_engine::migrate::rewrite_text(&text) != text {
        repairs.push("renamed retired names (Trees2 → Trees, MaskLayer ports → MaskOut, the Sqrt blend → Power)".to_string());
    }
    for (id, version, kept_list) in &before {
        let Some(node) = document.node(*id as i32) else {
            continue;
        };
        if version.unwrap_or(0) < node.version() as i64 {
            repairs.push(format!("{} ({id}): migrated from version {} to {}", node.name, version.unwrap_or(0), node.version()));
        }
        if *kept_list {
            repairs.push(format!("{} ({id}): its SaveDefinitions list became its SaveDefinition", node.name));
        }
    }
    let unnamed_saves = document.terrain().nodes.values().filter(|node| node.base.get("SaveDefinition").and_then(|definition| definition.get("Node")).and_then(Value::as_i64) != Some(node.id as i64) && nerthus_export::build::exports(node) && node.type_name != "Export").count();
    document.recover_save_definitions();
    if unnamed_saves > 0 {
        repairs.push(format!("{unnamed_saves} export marks given their node"));
    }
    repairs.extend(fix_document(&mut document));

    let mut warnings = Vec::new();
    let unknown = document.project.unknown_types.clone();
    if !unknown.is_empty() {
        warnings.push(format!("Nodes of types Nerthus does not know cannot be repaired: {}. {}", unknown.join(", "), if drop_unknown { "They are left out of the repaired file." } else { "The file is left alone unless drop_unknown_nodes is true (they would be left out)." }));
    }
    let findings = check_document(&document, false);
    let mut outcome = json!({"file": path, "repairs": repairs, "warnings": warnings, "remaining": findings.to_json()});
    let can_write = unknown.is_empty() || drop_unknown;
    let changed = !repairs.is_empty() || (drop_unknown && !unknown.is_empty());
    if !changed {
        outcome["changed"] = json!(false);
        outcome["message"] = json!("Nothing to repair; the file was left as it was.");
        return Ok(Step::Done(ToolResult::json(&outcome)));
    }
    if !write || !can_write {
        outcome["changed"] = json!(false);
        outcome["message"] = json!(if write { "The file was left as it was." } else { "write is false: the repairs were only listed." });
        return Ok(Step::Done(ToolResult::json(&outcome)));
    }
    if backup_wanted {
        outcome["backup"] = json!(back_up(&path)?);
    }
    document.save(&path).map_err(|error| format!("{error:#}"))?;
    outcome["changed"] = json!(true);
    Ok(Step::Done(ToolResult::json(&outcome)))
}

pub(crate) fn patch_project_file(arguments: &Arguments, context: &mut ToolContext) -> Result<Step, String> {
    let path = terrain_file(arguments.required_string("path")?)?;
    refuse_open_file(context.editor, &path, "set_parameters, mark_for_export and set_project_settings change it")?;
    let backup_wanted = arguments.boolean("create_backup")?.unwrap_or(true);
    let empty = Vec::new();
    let patches = arguments.array("patches")?.unwrap_or(&empty);
    let settings = build_members(arguments.object("build_settings")?)?;
    if patches.is_empty() && settings.is_empty() {
        return Err("Give patches or build_settings.".to_string());
    }
    let mut document = Document::open(&path).map_err(|error| format!("{error:#}"))?;
    let catalogue = nerthus_engine::Catalogue::get();
    let mut applied = Vec::new();
    let mut rejected = Vec::new();
    let mut notes = Vec::new();
    for (index, patch) in patches.iter().enumerate() {
        let Some(patch) = patch.as_object() else {
            rejected.push(format!("Patch {}: a patch is an object naming its node.", index + 1));
            continue;
        };
        let Some(node_value) = patch.get("node").or_else(|| patch.get("node_id")) else {
            rejected.push(format!("Patch {}: node is required.", index + 1));
            continue;
        };
        let id = match find_in(&document, node_value) {
            Ok(id) => id,
            Err(problem) => {
                rejected.push(format!("Patch {}: {problem}", index + 1));
                continue;
            }
        };
        let Some(node) = document.node(id) else {
            continue;
        };
        let who = format!("{} ({id})", node.name);
        let Some(spec) = catalogue.node(&node.type_name) else {
            rejected.push(format!("{who}: its type is unknown."));
            continue;
        };
        // Each parameter on its own, so one bad value does not cost the rest.
        let mut values = Vec::new();
        for (name, value) in patch.get("parameters").or_else(|| patch.get("properties")).and_then(Value::as_object).into_iter().flatten() {
            let single: serde_json::Map<String, Value> = [(name.clone(), value.clone())].into_iter().collect();
            match converted_parameters(spec, &single, Some(node)) {
                Ok(converted) => {
                    notes.extend(converted.notes.into_iter().map(|note| format!("{who}: {note}")));
                    values.extend(converted.values);
                }
                Err(problem) => rejected.push(format!("{who}: {problem}")),
            }
        }
        let rename = patch.get("name").and_then(Value::as_str).map(str::trim).filter(|name| !name.is_empty()).map(str::to_string);
        let bypass = patch.get("bypassed").and_then(boolean_of);
        let export = patch.get("export").or_else(|| patch.get("save_definition"));
        if !values.is_empty() {
            for (name, value) in &values {
                applied.push(format!("{who}: {name} = {value}"));
            }
            document.set_values(id, &values, "Patch");
        }
        if let Some(name) = rename {
            document.rename(id, &name);
            applied.push(format!("{who}: renamed {name}"));
        }
        if let Some(bypassed) = bypass {
            let state = if bypassed { "Bypassed" } else { "None" };
            document.set_member(id, "StateOverride", json!(state), "Bypass", true);
            applied.push(format!("{who}: {}", if bypassed { "bypassed" } else { "not bypassed" }));
        }
        match export {
            None | Some(Value::Null) => {}
            Some(given) => match export_patch(&mut document, id, given) {
                Ok(done) => applied.push(format!("{who}: {done}")),
                Err(problem) => rejected.push(format!("{who}: {problem}")),
            },
        }
    }
    if !settings.is_empty() {
        document.set_build_settings(&settings);
        for (member, value) in &settings {
            applied.push(format!("build setting {member} = {value}"));
        }
    }
    let mut outcome = json!({"file": path, "applied": applied, "rejected": rejected});
    if !notes.is_empty() {
        outcome["notes"] = json!(notes);
    }
    if applied.is_empty() {
        outcome["changed"] = json!(false);
        outcome["message"] = json!("Nothing was changed; the file was left as it was.");
        return Ok(Step::Done(ToolResult::json(&outcome)));
    }
    if backup_wanted {
        outcome["backup"] = json!(back_up(&path)?);
    }
    document.save(&path).map_err(|error| format!("{error:#}"))?;
    outcome["changed"] = json!(true);
    Ok(Step::Done(ToolResult::json(&outcome)))
}

/// A patch's export: `false` unmarks the node, `true` marks it, an object
/// (`enabled`, `file_name`, `format`) sets its mark. What it did.
fn export_patch(document: &mut Document, id: i32, given: &Value) -> Result<String, String> {
    let (enabled, file_name, format) = match given {
        Value::Bool(flag) => (*flag, None, None),
        Value::Object(object) => {
            let enabled = object.get("enabled").or_else(|| object.get("IsEnabled")).and_then(boolean_of).unwrap_or(true);
            let file_name = object.get("file_name").or_else(|| object.get("filename")).and_then(Value::as_str).map(str::trim).filter(|name| !name.is_empty()).map(str::to_string);
            let format = object.get("format").and_then(Value::as_str).map(parse_format).transpose()?;
            (enabled, file_name, format)
        }
        _ => return Err("export is true, false or {\"file_name\", \"format\", \"enabled\"}.".to_string()),
    };
    let marked = document.export_settings(id);
    if !enabled {
        if marked.is_some() {
            document.toggle_export(&[id], nerthus_export::FileFormat::PureRaw);
        }
        return Ok("unmarked for export".to_string());
    }
    if marked.is_none() {
        document.toggle_export(&[id], format.unwrap_or(nerthus_export::FileFormat::Png16));
    }
    document.edit_export(id, "Export settings", |settings| {
        if let Some(format) = format {
            settings.format = format;
        }
        if let Some(name) = &file_name {
            settings.filename = name.clone();
        }
        settings.enabled = true;
    });
    let settings = document.export_settings(id).ok_or("The node could not be marked for export.")?;
    Ok(format!("marked for export as {} ({})", settings.filename, format_key(settings.format)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_project(name: &str) -> (PathBuf, i32) {
        let folder = std::env::temp_dir().join(format!("nerthus-file-tools-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [26000.0, 26000.0]).unwrap();
        let path = folder.join(format!("{name}.terrain"));
        document.save(&path).unwrap();
        (path, mountain)
    }

    #[test]
    fn raw_nodes_are_read_before_migration() {
        let (path, mountain) = temporary_project("raw");
        let text = std::fs::read_to_string(&path).unwrap();
        let nodes = raw_nodes(&text);
        assert!(nodes.iter().any(|(id, _, _)| *id == mountain as i64));
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn exports_are_patched() {
        let (path, mountain) = temporary_project("export");
        let mut document = Document::open(&path).unwrap();
        let done = export_patch(&mut document, mountain, &json!({"filename": "height", "format": "UshortRaw16"})).unwrap();
        assert!(done.contains("height"), "{done}");
        let settings = document.export_settings(mountain).unwrap();
        assert_eq!(settings.format, nerthus_export::FileFormat::UshortRaw16);
        export_patch(&mut document, mountain, &json!(false)).unwrap();
        assert!(document.export_settings(mountain).is_none());
        assert!(export_patch(&mut document, mountain, &json!({"format": "bmp"})).is_err());
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}
