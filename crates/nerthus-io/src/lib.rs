//! File formats: `.tor` projects, `.gpr` presets, heightmap import/export.

pub mod build;
pub mod coders;
pub mod heightmap;
pub mod image_file;
pub mod mesh;
pub mod nrbf;
pub mod outputs;
pub mod pixel_cache;
pub mod preset;
pub mod project_xml;
pub mod resources;
pub mod tor;

use std::path::Path;

use anyhow::{Context, Result};
use nerthus_core::Project;

/// Loads a `.tor` (or plain XML) project file. Node behaviour is not
/// attached; rehydrate nodes through a registry before evaluating.
pub fn load_project(path: &Path) -> Result<Project> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let xml = tor::decode(&bytes)?;
    let xml = normalize_legacy_xml(&xml);
    let mut project = project_xml::read_project(&xml)?;
    if project.name.is_empty()
        && let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
            project.name = stem.to_string();
        }
    Ok(project)
}

/// Saves a project as a `.tor` file (or plain XML when the extension is `.xml`).
pub fn save_project(project: &Project, path: &Path) -> Result<()> {
    let xml = project_xml::write_project(project);
    let is_xml = path.extension().and_then(|value| value.to_str()).is_some_and(|value| value.eq_ignore_ascii_case("xml"));
    if is_xml {
        std::fs::write(path, xml)?;
        return Ok(());
    }
    let encoded = tor::encode(&xml)?;
    std::fs::write(path, encoded).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

/// The load-time fix-ups.
fn normalize_legacy_xml(xml: &str) -> String {
    xml.replace("<LayerBase ", "<Node ")
        .replace("</LayerBase>", "</Node>")
        .replace("EasyErosion", "Wizard")
}
