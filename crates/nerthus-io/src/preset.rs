//! Node presets (`.gpr`): XML with a misspelled `Paramaters` element.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use nerthus_core::ParamList;
use roxmltree::Document;

use crate::project_xml::{parse_param_list, write_param};

#[derive(Clone, Debug)]
pub struct Preset {
    pub applicable_type: String,
    pub name: String,
    pub description: String,
    pub params: ParamList,
    pub path: Option<PathBuf>,
}

pub fn read_preset(path: &Path) -> Result<Preset> {
    let text = std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let text = text.trim_start_matches('\u{feff}');
    let document = Document::parse(text)?;
    let root = document.root_element();
    let params_element = root.children().find(|node| node.is_element() && matches!(node.tag_name().name(), "Paramaters" | "Parameters"));
    Ok(Preset {
        applicable_type: root.attribute("ApplicableType").unwrap_or("").to_string(),
        name: root.attribute("Name").unwrap_or("").to_string(),
        description: root.attribute("Description").unwrap_or("").to_string(),
        params: parse_param_list(text, params_element),
        path: Some(path.to_path_buf()),
    })
}

pub fn write_preset(preset: &Preset, path: &Path) -> Result<()> {
    let mut output = String::new();
    output.push_str("<?xml version=\"1.0\"?>\r\n");
    output.push_str("<Preset xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"");
    output.push_str(&format!(" ApplicableType=\"{}\" Name=\"{}\" Description=\"{}\">\r\n  <Paramaters>\r\n", preset.applicable_type, preset.name, preset.description));
    for param in &preset.params {
        output.push_str("    ");
        write_param(&mut output, param);
        output.push_str("\r\n");
    }
    output.push_str("  </Paramaters>\r\n</Preset>");
    std::fs::write(path, output)?;
    Ok(())
}

/// Loads every `.gpr` in the given directories.
pub fn load_presets(directories: &[PathBuf]) -> Vec<Preset> {
    let mut presets = Vec::new();
    for directory in directories {
        let Ok(entries) = std::fs::read_dir(directory) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_preset = path.extension().and_then(|value| value.to_str()).is_some_and(|value| value.eq_ignore_ascii_case("gpr"));
            if !is_preset {
                continue;
            }
            if let Ok(preset) = read_preset(&path) {
                presets.push(preset);
            }
        }
    }
    presets.sort_by(|a, b| (a.applicable_type.as_str(), a.name.as_str()).cmp(&(b.applicable_type.as_str(), b.name.as_str())));
    presets
}

/// The preset applied to new nodes of a type (`PresetService.GetDefault`).
pub fn default_preset<'a>(presets: &'a [Preset], type_name: &str) -> Option<&'a Preset> {
    presets.iter().find(|preset| preset.applicable_type == type_name && preset.name == "Default")
}
