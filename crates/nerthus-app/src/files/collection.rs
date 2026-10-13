//! The macro collection (`Paths.Macros` and `M`): the macros
//! installed in the user's Nerthus folder, listed in the toolbox's Macro
//! menu by author; installing one (File › Import › Macro), adding one to
//! the project, and exporting the project as one (File › Export ›
//! Macro). Also the list of every project seen, for the Start dialog's
//! search (`MRU_SUPER`).

use std::path::{Path, PathBuf};

use nerthus_engine::macros::{compare_versions, parse_version, write_macro_file, MacroAsset, MacroMetadata, FORMAT_VERSION};

use super::{FileCommand, Message};
use crate::editor::Editor;

/// An installed macro.
#[derive(Clone, Debug)]
pub struct MacroEntry {
    pub path: PathBuf,
    pub metadata: MacroMetadata,
}

/// The installed macros (`Session.Macros`, filled by `M.LoadAllMetadata`).
#[derive(Clone, Debug, Default)]
pub struct MacroCollection {
    entries: Vec<MacroEntry>,
}

/// The collection as the toolbox lists it: the installed macros' metadata
/// the toolbox reads (`Session.Macros`), filled on each reload.
static INSTALLED: std::sync::RwLock<Vec<(String, Vec<MacroEntry>)>> = std::sync::RwLock::new(Vec::new());

/// The installed macros by author, as last read.
pub fn installed_macros() -> Vec<(String, Vec<MacroEntry>)> {
    INSTALLED.read().map(|installed| installed.clone()).unwrap_or_default()
}

impl MacroCollection {
    /// `M.LoadAllMetadata`: every `.macro` in the collection folder, by
    /// file name; a macro whose id came already, or without a name, is
    /// left out (`LoadMetadata`).
    pub fn reload(&mut self) {
        self.entries.clear();
        let Some(folder) = super::macros_folder() else {
            return;
        };
        let Ok(listing) = std::fs::read_dir(&folder) else {
            return;
        };
        let mut files: Vec<PathBuf> = listing.flatten().map(|entry| entry.path()).collect();
        files.retain(|path| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("macro")));
        files.sort();
        for path in files {
            let metadata = match MacroAsset::read_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    log::warn!("cannot read {}: {error}", path.display());
                    continue;
                }
            };
            let id = metadata.id();
            if metadata.name.is_empty() || self.get(&id).is_some() {
                continue;
            }
            self.entries.push(MacroEntry { path, metadata });
        }
        if let Ok(mut installed) = INSTALLED.write() {
            *installed = self.by_author();
        }
    }

    pub fn get(&self, id: &str) -> Option<&MacroEntry> {
        self.entries.iter().find(|entry| entry.metadata.id() == id)
    }

    /// The toolbox's Macro menu: authors (`NodeMetadata.Family`) and their
    /// macros, each by name.
    pub fn by_author(&self) -> Vec<(String, Vec<MacroEntry>)> {
        let mut authors: std::collections::BTreeMap<String, Vec<MacroEntry>> = std::collections::BTreeMap::new();
        for entry in &self.entries {
            authors.entry(entry.metadata.author.clone()).or_default().push(entry.clone());
        }
        let mut grouped: Vec<(String, Vec<MacroEntry>)> = authors.into_iter().collect();
        for (_, entries) in grouped.iter_mut() {
            entries.sort_by_key(|entry| entry.metadata.id());
        }
        grouped
    }
}

/// `M.LoadMacro`'s checks: not into its own project, made in a format
/// version no newer than this one.
fn load_macro(editor: &Editor, path: &Path) -> Result<MacroAsset, String> {
    let asset = MacroAsset::read(path).map_err(|error| error.to_string())?;
    if asset.metadata.source == editor.document.project_id() {
        return Err("You cannot add a Macro to its own source project.".to_string());
    }
    let made = asset.metadata.format_version_numbers().unwrap_or_default();
    let current = parse_version(FORMAT_VERSION).unwrap_or_default();
    let too_new = compare_versions(&made, &current) == std::cmp::Ordering::Greater;
    if too_new || made.first() != Some(&2) {
        return Err("This macro is not meant to be used with this version of Nerthus.".to_string());
    }
    Ok(asset)
}

/// `Shell.ImportMacro` after the dialog: the macro's details, and a
/// question whether to install it.
pub fn ask_to_install(editor: &mut Editor, path: &Path) {
    let asset = match load_macro(editor, path) {
        Ok(asset) => asset,
        Err(problem) => {
            let problem = if problem.contains("base32") || problem.contains("decoded") { "Older configuration. Re-export your macro from the original project.".to_string() } else { problem };
            let message = Message::new("Nerthus", &format!("The macro could not be loaded.\n\nError: {problem}."));
            editor.files.messages.push(message);
            return;
        }
    };
    let metadata = &asset.metadata;
    let mut text = format!("Macro details:\n\nName: {}\nVersion: {}\nAuthor: {} ({})\n", metadata.name, metadata.version, metadata.author, metadata.url);
    if !metadata.dependent_macros.is_empty() {
        text.push_str("\nDependent Macros:\n");
        text.push_str(&metadata.dependent_macros.join(","));
        text.push_str("\nThese must be installed as well or this macro will not function.\n");
    }
    let mut question = Message::new(&format!("Install Macro {}?", metadata.name), &text);
    question.footnote = Some("WARNING: Do not install macros from sources you do not trust.".to_string());
    question.yes = Some(FileCommand::InstallMacro(path.to_path_buf()));
    editor.files.messages.push(question);
}

/// The install question's Yes: the file copied into the collection (in
/// place of one of its name) and the collection read again.
pub fn install(editor: &mut Editor, path: &Path) {
    let Some(folder) = super::macros_folder() else {
        return;
    };
    let Some(name) = path.file_name() else {
        return;
    };
    let target = folder.join(name);
    let copied = std::fs::create_dir_all(&folder).and_then(|()| {
        if target.exists() {
            std::fs::remove_file(&target)?;
        }
        std::fs::copy(path, &target).map(|_| ())
    });
    if let Err(error) = copied {
        editor.report(format!("The macro could not be installed: {error}"));
        return;
    }
    editor.files.macros.reload();
}

/// A macro from the toolbox (`CreateSearchedNode` for a macro): a Macro
/// node for it, the macro read into the project's macros for builds. A
/// macro the project does not have yet brings the macros it depends on
/// (`M.AddToProject`), and is not added when one is not installed.
pub fn add_macro(editor: &mut Editor, id: &str, position: [f32; 2], tab: usize) {
    let folders = super::macro_folders(editor.document.path.as_deref());
    let path = editor.files.macros.get(id).map(|entry| entry.path.clone()).or_else(|| nerthus_engine::macros::find_macro_file(id, &folders));
    let Some(path) = path else {
        editor.report(format!("Macro {id} is not installed."));
        return;
    };
    let asset = match load_macro(editor, &path) {
        Ok(asset) => asset,
        Err(problem) => {
            editor.report(problem);
            return;
        }
    };
    let mut dependencies = Vec::new();
    let listed = editor.document.project.macro_ids().iter().any(|listed| listed == id);
    if !listed {
        let find = |dependency: &str| nerthus_engine::macros::find_macro_file(dependency, &folders);
        let mut problems = Vec::new();
        let loaded = match nerthus_engine::macros::load_dependencies(&asset, &find, &mut problems) {
            Ok(loaded) => loaded,
            Err(error) => {
                editor.report(error.to_string());
                return;
            }
        };
        for (_, problem) in problems {
            editor.report(problem.to_string());
        }
        dependencies = loaded.iter().map(|dependency| dependency.metadata.id()).collect();
    }
    let Some(node) = editor.document.add_macro_node(&asset, &dependencies, position, tab) else {
        return;
    };
    editor.document.apply_node_display(&[node], &editor.user.options);
    editor.reload_macros();
    editor.select(Some(node));
    editor.changed();
}

/// Export Macro's form (`ExportMacroDialog`), filled from what was typed
/// for this project last time.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportForm {
    pub name: String,
    pub version: String,
    pub author: String,
    pub url: String,
    pub description: String,
    pub save_file: bool,
    pub install: bool,
}

impl ExportForm {
    /// `ExportMacroDialog_Loaded`.
    pub fn for_project(editor: &Editor) -> Self {
        let document = &editor.document;
        Self {
            name: document.project_metadata("_LastFriendlyName", "MacroName"),
            version: document.project_metadata("_LastVersion", "1.0"),
            author: document.project_metadata("_LastAuthor", "YourName"),
            url: document.project_metadata("_LastURL", env!("CARGO_PKG_REPOSITORY")),
            description: document.project_metadata("_LastDescription", "Describe your macro here. It will show up in the Toolbox tooltip."),
            save_file: true,
            install: true,
        }
    }

    /// `UpdateProperties`: the name and author as ids allow
    /// (`SafeString`).
    pub fn friendly_name(&self) -> String {
        safe_string(&self.name)
    }

    pub fn author_name(&self) -> String {
        safe_string(&self.author)
    }

    /// `ExportMacroDialog.MacroId`.
    pub fn macro_id(&self) -> String {
        format!("{}.{}", self.author_name(), self.friendly_name())
    }

    /// The version as `System.Version` reads it (the box takes a number
    /// such as 1.5; a whole number gets `.0`).
    pub fn normalized_version(&self) -> String {
        let trimmed = self.version.trim();
        if parse_version(trimmed).is_some() {
            return trimmed.to_string();
        }
        if trimmed.parse::<u32>().is_ok() {
            return format!("{trimmed}.0");
        }
        "1.0.0.0".to_string()
    }

    /// `ExportMacroDialog_Closing`: what was typed, kept in the project's
    /// metadata for next time.
    pub fn remember(&self, editor: &mut Editor) {
        let document = &mut editor.document;
        document.set_project_metadata("_LastFriendlyName", &self.friendly_name());
        document.set_project_metadata("_LastAuthor", &self.author_name());
        document.set_project_metadata("_LastDescription", &self.description);
        document.set_project_metadata("_LastURL", &self.url);
        document.set_project_metadata("_LastVersion", &self.version);
    }
}

/// `StringExtensions.SafeString`: letters, digits, spaces, dashes and
/// underscores kept.
fn safe_string(text: &str) -> String {
    text.chars().filter(|character| character.is_alphanumeric() || matches!(character, ' ' | '-' | '_')).collect()
}

/// Export Macro's Export Macro button, once the form passed its checks:
/// to `file` (Save as file), into the collection (Install in Nerthus), or
/// both; then the collection read again and the project saved.
pub fn export_to(editor: &mut Editor, file: Option<PathBuf>) {
    let Some(form) = editor.files.export_macro.take() else {
        return;
    };
    form.remember(editor);
    let mut metadata = MacroMetadata::for_export(&form.friendly_name(), &form.author_name(), &form.url, &form.normalized_version(), &form.description, &editor.document.project_id());
    // The macros its Macro nodes run (`DependentMacros`). The export
    // checks refuse a macro inside a macro, so this stays empty.
    metadata.dependent_macros = editor.document.used_macros();
    let metadata_value: serde_json::Value = serde_json::from_str(&metadata.to_json()).unwrap_or_default();
    let asset_json = editor.document.macro_asset_json(&metadata_value);
    let temporary = std::env::temp_dir().join(format!("nerthus-macro-{}.macro", std::process::id()));
    let target = file.clone().unwrap_or_else(|| temporary.clone());
    if let Err(error) = write_macro_file(&target, &metadata, &asset_json) {
        editor.report(format!("Failed to export Macro. {error}"));
        return;
    }
    if form.install
        && let Some(folder) = super::macros_folder()
    {
        let installed = folder.join(format!("{}.macro", metadata.id()));
        let copied = std::fs::create_dir_all(&folder).and_then(|()| std::fs::copy(&target, &installed).map(|_| ()));
        if let Err(error) = copied {
            editor.report(format!("Failed to export Macro. {error}"));
        }
    }
    if file.is_none() {
        let _ = std::fs::remove_file(&temporary);
    }
    editor.files.macros.reload();
    editor.save();
}

/// `MRU_SUPER` (`recent-files-all.mru`): every project opened or saved, newest
/// first, at most 5000, for the Start dialog's search.
const ALL_RECENT_FILE: &str = "recent-files-all.mru";
const ALL_RECENT_LIMIT: usize = 5000;

fn all_recent_path() -> Option<PathBuf> {
    crate::user::data_folder().map(|folder| folder.join(ALL_RECENT_FILE))
}

pub fn all_recent() -> Vec<PathBuf> {
    let Some(path) = all_recent_path() else {
        return Vec::new();
    };
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines().filter(|line| !line.trim().is_empty()).map(PathBuf::from).collect()
}

/// `MRU_SUPER.AddItem`.
pub fn add_to_all_recent(project: &Path) {
    let mut files = all_recent();
    files.retain(|file| file != project);
    files.insert(0, project.to_path_buf());
    files.truncate(ALL_RECENT_LIMIT);
    write_all_recent(&files);
}

fn write_all_recent(files: &[PathBuf]) {
    let Some(path) = all_recent_path() else {
        return;
    };
    let lines: Vec<String> = files.iter().map(|file| file.to_string_lossy().to_string()).collect();
    let mut text = lines.join("\n");
    text.push('\n');
    if let Some(folder) = path.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    if let Err(error) = std::fs::write(&path, text) {
        log::warn!("cannot write {}: {error}", path.display());
    }
}

/// `User.ConsumeSuperMRU`: every project under the documents folder joins
/// the list.
pub fn consume_documents(documents: &Path) {
    let mut found = Vec::new();
    collect_projects(documents, &mut found);
    if found.is_empty() {
        return;
    }
    let mut files = all_recent();
    for project in found {
        files.retain(|file| *file != project);
        files.insert(0, project);
    }
    files.truncate(ALL_RECENT_LIMIT);
    write_all_recent(&files);
}

fn collect_projects(folder: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_projects(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "terrain") {
            found.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_ids_and_versions() {
        let form = ExportForm {
            name: "My Macro!".to_string(),
            version: "2".to_string(),
            author: "Ann/B".to_string(),
            url: String::new(),
            description: String::new(),
            save_file: true,
            install: true,
        };
        assert_eq!(form.macro_id(), "AnnB.My Macro");
        assert_eq!(form.normalized_version(), "2.0");
        let decimal = ExportForm { version: "1.5".to_string(), ..form };
        assert_eq!(decimal.normalized_version(), "1.5");
    }
}
