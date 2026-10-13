//! What the command-line build (`Program`) does to a project before and
//! after its build, beyond the build itself: the variables its command line
//! assigns (`-v`, `--va`, `--vars`), the build profile it builds
//! with (`-p`), and the copy of the build folder it keeps as
//! `Latest` ("Maintain a static folder with the latest copy").
//! The mutation `--seed` asks for is [`nerthus_engine::mutation`].

use std::fs;
use std::path::{Path, PathBuf};

use nerthus_engine::automation::{Automation, VariableValue};
use nerthus_engine::Asset;
use serde_json::Value;

use crate::build::{BuildDefinition, BuildType, OutputType};
use crate::destination::DEFAULT_TEMPLATE;

/// How a command line assigns the project's variables.
#[derive(Clone, Debug, PartialEq)]
pub enum VariableAssignment {
    /// `-v name=value`, repeated.
    Pairs(Vec<(String, String)>),
    /// `-va`: a value for every variable, in the project's order.
    Values(Vec<String>),
    /// `-vars`: a `.json` object or a `.txt` file of `name=value` lines.
    File(PathBuf),
}

/// The variables assigned as text (`Variable.Value` is set to
/// the string; bindings convert it as they read it). `say` hears what the
/// build prints. `-v` naming a variable the project lacks and `--va`
/// with the wrong number of values stop the build; a `--vars` file that
/// cannot be read or names a missing variable is reported, and the
/// variables assigned before the problem stay assigned.
pub fn assign_variables(automation: &mut Automation, assignment: &VariableAssignment, say: &mut dyn FnMut(String)) -> Result<(), String> {
    match assignment {
        VariableAssignment::Pairs(pairs) => {
            log::info!("Parsing Variables from V");
            say("Assigning variables...".to_string());
            for (name, value) in pairs {
                let Some(variable) = automation.variables.get_mut(name) else {
                    return Err(format!("The given key '{name}' was not present in the dictionary."));
                };
                variable.value = VariableValue::Text(value.clone());
                say(format!("    Variable [{name}] set to [{value}]"));
            }
            Ok(())
        }
        VariableAssignment::Values(values) => {
            log::info!("Parsing Variables from VA");
            if values.len() != automation.variables.len() {
                let message = "Variables passed by -va argument must be equal in count to the number of variables declared in the terrain file.\n\nIf count is correct, check syntax.";
                return Err(message.to_string());
            }
            say("Assigning variables...".to_string());
            for ((name, variable), value) in automation.variables.iter_mut().zip(values) {
                variable.value = VariableValue::Text(value.clone());
                say(format!("    Variable [{name}] set to [{value}]"));
            }
            Ok(())
        }
        VariableAssignment::File(path) => {
            if !path.is_file() {
                return Ok(());
            }
            log::info!("Parsing Variables from VARS");
            say(format!("Loading variables from {}", path.display()));
            if let Err(problem) = assign_from_file(automation, path, say) {
                log::error!("Failed to parse variables. {problem}");
                say(format!("Failed to parse variables. {problem}"));
            }
            Ok(())
        }
    }
}

/// The `-vars` file's pairs assigned in its order.
fn assign_from_file(automation: &mut Automation, path: &Path, say: &mut dyn FnMut(String)) -> Result<(), String> {
    log::info!("Parsing from source: {}", path.display());
    let extension = path.extension().map(|extension| extension.to_string_lossy().into_owned());
    let pairs = match extension.as_deref() {
        Some("json") => read_json_pairs(path),
        Some("txt") => read_text_pairs(path)?,
        _ => return Ok(()),
    };
    for (name, value) in pairs {
        let Some(variable) = automation.variables.get_mut(&name) else {
            return Err(format!("The given key '{name}' was not present in the dictionary."));
        };
        variable.value = VariableValue::Text(value.clone());
        say(format!("    Variable [{name}] set to [{value}]"));
    }
    Ok(())
}

/// A JSON object's members as text (`JToken.ToString()`:
/// strings as they are, other values as Newtonsoft writes them). A file
/// that is not a JSON object gives nothing.
fn read_json_pairs(path: &Path) -> Vec<(String, String)> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let parsed: Result<serde_json::Map<String, Value>, _> = serde_json::from_str(text.trim_start_matches('\u{feff}'));
    let Ok(members) = parsed else {
        log::error!("Failed to parse JSON: {}", path.display());
        return Vec::new();
    };
    let mut pairs = Vec::new();
    for (name, value) in members {
        let text = match value {
            Value::String(text) => text,
            Value::Bool(true) => "True".to_string(),
            Value::Bool(false) => "False".to_string(),
            Value::Null => String::new(),
            Value::Number(number) => number.to_string(),
            other => serde_json::to_string_pretty(&other).unwrap_or_default(),
        };
        pairs.push((name, text));
    }
    pairs
}

/// Each line split at its `=`s, the first part the name
/// and the second the value. A line without `=` or a name given twice is
/// an error.
fn read_text_pairs(path: &Path) -> Result<Vec<(String, String)>, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut pairs: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let mut parts = line.split('=');
        let name = parts.next().unwrap_or_default().to_string();
        let Some(value) = parts.next() else {
            return Err("Index was outside the bounds of the array.".to_string());
        };
        if pairs.iter().any(|(existing, _)| *existing == name) {
            return Err(format!("An item with the same key has already been added. Key: {name}"));
        }
        pairs.push((name, value.to_string()));
    }
    Ok(pairs)
}

/// `-v name=value` as the command line reads it: split at the first `=`.
pub fn parse_pair(argument: &str) -> Result<(String, String), String> {
    match argument.split_once('=') {
        Some((name, value)) => Ok((name.to_string(), value.to_string())),
        None => Err(format!("The value '{argument}' is not a valid name=value pair.")),
    }
}

/// `Asset.BuildProfiles`: the names of the profiles, in the file's order.
pub fn profile_names(asset: &Asset) -> Vec<String> {
    let profiles = asset.extra.get("BuildProfiles").and_then(Value::as_object);
    profiles.map(|profiles| profiles.keys().cloned().collect()).unwrap_or_default()
}

/// A profile's build definition as saved.
pub fn profile_json<'a>(asset: &'a Asset, name: &str) -> Option<&'a Value> {
    asset.extra.get("BuildProfiles")?.as_object()?.get(name)
}

/// The build profile: `Asset.BuildDefinition` replaced by the profile's
/// (the build stops with an error when the project has no such profile).
pub fn profile_definition(asset: &Asset, name: &str) -> Result<BuildDefinition, String> {
    match profile_json(asset, name) {
        Some(definition) => Ok(BuildDefinition::from_json(Some(definition))),
        None => Err(format!("The project has no build profile named {name}.")),
    }
}

/// Whether the build folder is a new one each build (its
/// template numbers or timestamps it), which "Maintain a static folder
/// with the latest copy" needs.
pub fn destination_varies(definition: &BuildDefinition) -> bool {
    let template = if definition.destination.is_empty() { DEFAULT_TEMPLATE } else { definition.destination.as_str() };
    template.contains("[+++]") || template.contains("[Timestamp]")
}

/// The build folder copied beside itself as `Latest`, replacing
/// the last copy (every folder made, every file copied).
pub fn copy_to_latest(folder: &Path) -> std::io::Result<PathBuf> {
    let parent = folder.parent().unwrap_or(folder);
    let latest = parent.join("Latest");
    if latest.is_dir() {
        fs::remove_dir_all(&latest)?;
    }
    copy_tree(folder, &latest)?;
    Ok(latest)
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// `.NET`'s `bool.ToString()`.
fn dotnet_bool(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

/// `BuildSettingsWindow.ShowProfile`: a profile's settings as Build
/// Settings › Profiles lists them.
pub fn profile_summary(definition: &BuildDefinition) -> String {
    let mut lines: Vec<String> = Vec::new();
    let resolution = match (&definition.portion, definition.build_type) {
        (Some(region), BuildType::Region) => region.size.max(0) as usize,
        _ => definition.resolution,
    };
    lines.push(format!("Build Resolution: {resolution}"));
    match definition.build_type {
        BuildType::Standard => lines.push("Build Type: Standard".to_string()),
        BuildType::Hybrid => lines.push("Build Type: Hybrid".to_string()),
        BuildType::Tiled => lines.push("Build Type: Tiled".to_string()),
        BuildType::Region => {
            lines.push("Build Type: Region".to_string());
            let name = definition.portion.as_ref().map(|region| region.name.clone()).unwrap_or_default();
            lines.push(name);
        }
    }
    lines.push(format!("Subdivision: {}", definition.subdivision.name()));
    lines.push(format!("Output Type: {}", definition.output_type.name()));
    lines.push("------------------------------------".to_string());
    if definition.output_type == OutputType::CroppedTiles {
        let tiles = definition.number_of_tiles * definition.number_of_tiles;
        // `{n:###,###}`: nothing for 0, thousands separated.
        let tiles_text = if tiles == 0 { String::new() } else { group_thousands(tiles) };
        lines.push(format!("Tile Resolution: {}", definition.tile_resolution));
        lines.push(format!("Edge Blending: {}", nerthus_engine::automation::dotnet_float_string(definition.edge_blending)));
        lines.push(format!("Number of Tiles: {} ({tiles_text} tiles)", definition.number_of_tiles));
        lines.push(format!("Overlapping Pixels: {}", definition.overlapping_pixels));
        lines.push(format!("Tile Pad Digits: {}", dotnet_bool(definition.tile_pad_digits)));
        lines.push(format!("Tile Zero Index: {}", dotnet_bool(definition.tile_zero_index)));
        lines.push(format!("Flip Y: {}", dotnet_bool(definition.flip_y)));
        lines.push(format!("Tile Pattern: {}", definition.tile_pattern));
        lines.push("------------------------------------".to_string());
    }
    lines.push(format!("Organize Files: {}", definition.organize_files.name()));
    lines.push(format!("Preserve Tiled Cache: {}", dotnet_bool(definition.preserve_tiled_cache)));
    lines.push(format!("Remove Primary Port Name: {}", dotnet_bool(definition.remove_primary_port_name)));
    lines.push(format!("Maintain Prime Folder: {}", dotnet_bool(definition.maintain_prime_folder)));
    lines.push(format!("Open Folder: {}", dotnet_bool(definition.open_folder)));
    lines.push(format!("Copy Terrain: {}", dotnet_bool(definition.copy_terrain)));
    if !definition.post_build_script.is_empty() {
        lines.push(format!("Post Build Script: {}", definition.post_build_script));
    }
    lines.push(format!("Destination: {}", definition.destination));
    lines.push(format!("Overwrite Mode: {} ", definition.overwrite_mode.name()));
    let mut text = lines.join("\r\n");
    text.push_str("\r\n");
    text
}

/// A count with `,` between its thousands (`###,###`).
fn group_thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        let from_end = digits.len() - index;
        if index > 0 && from_end.is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerthus_engine::automation::{VarType, Variable};

    fn automation() -> Automation {
        let mut automation = Automation::default();
        for name in ["Height", "Rough"] {
            let variable = Variable {
                name: name.to_string(),
                var_type: VarType::Float,
                value: VariableValue::Double(0.5),
                default: VariableValue::Double(0.5),
                json: Value::Null,
            };
            automation.variables.insert(name.to_string(), variable);
        }
        automation
    }

    #[test]
    fn pairs_assign_text() {
        let mut automation = automation();
        let pairs = VariableAssignment::Pairs(vec![("Rough".to_string(), "0.75".to_string())]);
        let mut said = Vec::new();
        assign_variables(&mut automation, &pairs, &mut |line| said.push(line)).unwrap();
        assert_eq!(automation.variables["Rough"].value, VariableValue::Text("0.75".to_string()));
        assert_eq!(said, ["Assigning variables...", "    Variable [Rough] set to [0.75]"]);
    }

    #[test]
    fn values_need_one_for_each_variable() {
        let mut automation = automation();
        let short = VariableAssignment::Values(vec!["1".to_string()]);
        assert!(assign_variables(&mut automation, &short, &mut |_| {}).is_err());
        let full = VariableAssignment::Values(vec!["1".to_string(), "2".to_string()]);
        assign_variables(&mut automation, &full, &mut |_| {}).unwrap();
        assert_eq!(automation.variables["Height"].value, VariableValue::Text("1".to_string()));
        assert_eq!(automation.variables["Rough"].value, VariableValue::Text("2".to_string()));
    }

    #[test]
    fn unknown_pair_stops() {
        let mut automation = automation();
        let pairs = VariableAssignment::Pairs(vec![("Missing".to_string(), "1".to_string())]);
        assert!(assign_variables(&mut automation, &pairs, &mut |_| {}).is_err());
    }

    #[test]
    fn thousands_group() {
        assert_eq!(group_thousands(9), "9");
        assert_eq!(group_thousands(1024), "1,024");
        assert_eq!(group_thousands(1048576), "1,048,576");
    }

    #[test]
    fn default_destination_varies() {
        let definition = BuildDefinition::default();
        assert!(destination_varies(&definition));
        let fixed = BuildDefinition { destination: "C:\\Builds\\Fixed".to_string(), ..BuildDefinition::default() };
        assert!(!destination_varies(&fixed));
    }
}
