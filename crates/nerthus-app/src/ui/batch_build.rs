//! Project › Execute Batch Build… (`BatchBuildWindow`, "Batch Build
//! Manager"): a script of builds, one `nerthus-build` command line a
//! build, with a build profile, its region, a mutation and the project's
//! variables. Add Profile adds the profile chosen, Add All Profiles one
//! build each (the current settings first, mutations reset), Add Mutation
//! the chosen profile as the next mutation. The script can be edited,
//! saved, and started: it runs on its own while the editor carries on.
//!
//! The script, its file and the mutation count last until Nerthus closes,
//! whichever project is open.

use std::path::{Path, PathBuf};

use crossbeam_channel::Receiver;
use egui::{Align, Id, Layout, RichText, Stroke, Vec2};
use nerthus_engine::region;
use nerthus_export::swarm;

use crate::editor::Editor;
use crate::theme;
use crate::ui::file_dialogs::{DialogKind, DialogRequest, FileDialogs};
use crate::ui::tool_window;

/// `noProfile`.
const NO_PROFILE: &str = "Current Settings (No Profile)";

/// The key Save Batch File's dialog answers under.
const SAVE_KEY: &str = "batch-build-save";

/// What lasts between openings of the window.
#[derive(Default)]
pub struct BatchSession {
    /// `Seed`: the last mutation added.
    seed: i32,
    /// `BatchFile`: where the script was saved, if it was.
    file: Option<PathBuf>,
    /// `BatchFileContents`: the script as last kept.
    contents: String,
    /// A batch running: its end, and its file.
    running: Option<Receiver<Result<(), String>>>,
}

/// The window while it is open.
pub struct BatchWindow {
    /// `cboProfiles`: 0 for the current settings, else the profile's
    /// index from 1.
    profile: usize,
    /// The script as shown (`batch`).
    text: String,
}

/// The first line of a script (`REM Nerthus Batch Build Version …`, with
/// `#` for `REM` outside Windows).
fn script_header() -> String {
    let comment = if cfg!(windows) { "REM" } else { "#" };
    format!("{comment} Nerthus Batch Build Version {}", env!("CARGO_PKG_VERSION"))
}

/// `nerthus-build` beside the editor (where cargo builds both).
fn build_program() -> PathBuf {
    let name = if cfg!(windows) { "nerthus-build.exe" } else { "nerthus-build" };
    let current = std::env::current_exe().unwrap_or_default();
    current.with_file_name(name)
}

/// In quotes.
fn quoted(text: &str) -> String {
    format!("\"{text}\"")
}

/// In quotes when it holds a letter.
fn quoted_if_text(text: &str) -> String {
    if text.chars().any(char::is_alphabetic) { quoted(text) } else { text.to_string() }
}

impl BatchSession {
    /// Whether a batch is running.
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// `ReloadBatchQueue`: the script, or its first line when there is
    /// none.
    fn reload(&mut self) -> String {
        if self.contents.is_empty() {
            self.contents = script_header();
        }
        self.contents.clone()
    }

    /// Takes in a running batch's end.
    pub fn poll(&mut self, editor: &mut Editor) {
        let Some(running) = &self.running else {
            return;
        };
        let Ok(result) = running.try_recv() else {
            return;
        };
        self.running = None;
        match result {
            Ok(()) => editor.report("Batch Build completed."),
            Err(problem) => editor.report(format!("Batch Build failed: {problem}")),
        }
    }
}

impl BatchWindow {
    /// `BatchBuildWindow_Loaded`: the current settings chosen, the script
    /// shown.
    pub fn open(session: &mut BatchSession) -> BatchWindow {
        BatchWindow { profile: 0, text: session.reload() }
    }

    /// The profile chosen, by name (`None` for the current settings).
    fn chosen_profile(&self, names: &[String]) -> Option<String> {
        if self.profile == 0 {
            return None;
        }
        names.get(self.profile - 1).cloned()
    }

    /// `AddProfileToQueue`: the script as typed kept, and a build of the
    /// profile added.
    fn add(&mut self, session: &mut BatchSession, editor: &Editor, profile: Option<&str>) {
        session.contents = self.text.clone();
        let file = editor.document.path.as_ref().map(|path| path.to_string_lossy().into_owned()).unwrap_or_default();
        let mut line = format!("\n{} build {}", quoted(&build_program().to_string_lossy()), quoted(&file));
        let definition = profile.and_then(|name| editor.document.build_profile(name));
        if let (Some(name), Some(_)) = (profile, &definition) {
            line.push_str(&format!(" -p {}", quoted(name)));
        }
        let profile_json = profile.and_then(|name| swarm::profile_json(&editor.document.project.assets[0], name));
        if let Some(portion) = profile_json.and_then(region::portion) {
            line.push_str(&format!(" -r {}", quoted(&portion.name)));
        }
        if session.seed > 0 {
            line.push_str(&format!(" --seed {}", session.seed));
        }
        // Every variable with its value.
        let mut variables = String::new();
        for (name, variable) in &editor.document.project.assets[0].automation.variables {
            variables.push_str(&format!("-v {name}={} ", quoted_if_text(&variable.value.to_text())));
        }
        let variables = format!(" {variables}");
        line.push_str(variables.trim_end());
        session.contents.push_str(&line);
        self.text = session.contents.clone();
    }

    fn clear(&mut self, session: &mut BatchSession) {
        session.contents.clear();
        self.text = session.reload();
    }
}

/// Starts the script (`BtnStartBatchBuild_OnClick`): written to its file,
/// or a new temporary one, and run.
fn start(session: &mut BatchSession, text: &str, editor: &mut Editor) {
    editor.report("Batch Build started.");
    let extension = if cfg!(windows) { "bat" } else { "sh" };
    let file = match &session.file {
        Some(file) => file.clone(),
        None => {
            let name = format!("{}.{extension}", uuid_name());
            let file = std::env::temp_dir().join(name);
            session.file = Some(file.clone());
            file
        }
    };
    if let Err(error) = std::fs::write(&file, text) {
        editor.report(format!("Batch Build failed: cannot write {}: {error}", file.display()));
        return;
    }
    let (sender, receiver) = crossbeam_channel::bounded(1);
    let spawned = std::thread::Builder::new().name("batch build".to_string()).spawn(move || {
        let _ = sender.send(run_script(&file));
    });
    match spawned {
        Ok(_) => session.running = Some(receiver),
        Err(error) => editor.report(format!("Batch Build failed: {error}")),
    }
}

/// A name no other temporary script has.
fn uuid_name() -> String {
    let nanoseconds = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|since| since.as_nanos()).unwrap_or(0);
    format!("nerthus-batch-{}-{nanoseconds:x}", std::process::id())
}

/// Runs a script to its end.
fn run_script(file: &Path) -> Result<(), String> {
    let mut command = if cfg!(windows) { std::process::Command::new("cmd") } else { std::process::Command::new("sh") };
    if cfg!(windows) {
        command.arg("/C");
    }
    command.arg(file);
    let status = command.status().map_err(|error| error.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("the script ended with {status}")) }
}

/// The window. Returns false once closed.
pub fn show(ctx: &egui::Context, editor: &mut Editor, window: &mut BatchWindow, session: &mut BatchSession, dialogs: &mut FileDialogs) -> bool {
    if let Some(answer) = dialogs.take(SAVE_KEY)
        && let Some(path) = answer
    {
        match std::fs::write(&path, &window.text) {
            Ok(()) => {
                session.file = Some(path);
                editor.report("Batch Build file saved.");
            }
            Err(error) => editor.report(format!("Cannot save {}: {error}", path.display())),
        }
    }
    let names = editor.document.build_profiles();
    if window.profile > names.len() {
        window.profile = 0;
    }
    let mut open = true;
    let mut close = false;
    let mut frame = egui::Window::new("Batch Build Manager");
    frame = frame.collapsible(false);
    frame = frame.resizable(true);
    frame = tool_window::place(ctx, frame, Id::new("batch-build"), Vec2::new(750.0, 450.0));
    frame = frame.open(&mut open);
    frame.show(ctx, |ui| {
        ui.add_space(8.0);
        ui.label(RichText::new("Build Profile").font(theme::interface(12.0)).color(theme::LABEL_DEFAULT));
        let current = if window.profile == 0 { NO_PROFILE.to_string() } else { names[window.profile - 1].clone() };
        let mut combo = egui::ComboBox::from_id_salt("batch-profile");
        combo = combo.selected_text(current);
        combo = combo.width(ui.available_width());
        combo.show_ui(ui, |ui| {
            ui.selectable_value(&mut window.profile, 0, NO_PROFILE);
            for (index, name) in names.iter().enumerate() {
                ui.selectable_value(&mut window.profile, index + 1, name.as_str());
            }
        });
        ui.add_space(8.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let can_clear = session.contents != script_header();
            if ui.add_enabled(can_clear, egui::Button::new("Clear Batch Build")).on_hover_text("Clears the below Batch Build list.").clicked() {
                window.clear(session);
            }
            if ui.button("Add All Profiles").on_hover_text("Adds all profiles to batch build.").clicked() {
                // `BtnAddAllProfiles_OnClick`: a fresh script, mutations
                // from the start, the current settings then every profile.
                window.clear(session);
                session.seed = 0;
                window.add(session, editor, None);
                for name in &names {
                    window.add(session, editor, Some(name));
                }
            }
            if ui.button("Add Profile").on_hover_text("Adds the selected profile to batch build.").clicked() {
                let profile = window.chosen_profile(&names);
                window.add(session, editor, profile.as_deref());
            }
            if ui.button("Add Mutation").on_hover_text("Adds a mutation to the batch.").clicked() {
                session.seed += 1;
                let profile = window.chosen_profile(&names);
                window.add(session, editor, profile.as_deref());
            }
        });
        ui.add_space(16.0);
        let footer = 36.0;
        let mut script = egui::TextEdit::multiline(&mut window.text);
        script = script.font(theme::monospace(12.0));
        script = script.code_editor();
        script = script.desired_width(f32::INFINITY);
        let height = (ui.available_height() - footer).max(120.0);
        egui::ScrollArea::both().id_salt("batch-script").max_height(height).auto_shrink([false, false]).show(ui, |ui| {
            ui.add_sized(Vec2::new(ui.available_width(), height), script);
        });
        ui.add_space(8.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Close").clicked() {
                close = true;
            }
            let label = RichText::new("Start Batch Build").color(theme::LABEL_EMPHASIS);
            let mut start_button = egui::Button::new(label);
            start_button = start_button.stroke(Stroke::new(1.0, theme::BUTTON_IMPORTANT_BORDER));
            if ui.add_enabled(!session.is_running(), start_button).clicked() {
                start(session, &window.text.clone(), editor);
            }
            ui.add_space(28.0);
            if ui.add_enabled(!dialogs.pending(SAVE_KEY), egui::Button::new("Save Batch File")).clicked() {
                let (filter, extension) = if cfg!(windows) { ("Batch File (*.bat)", "bat") } else { ("Shell Script (*.sh)", "sh") };
                // `BtnSaveBatchFile_OnClick` starts where the last save went.
                let directory = editor.user.locations.folder(crate::user::locations::SAVE);
                let request = DialogRequest {
                    key: SAVE_KEY.to_string(),
                    title: "Save Batch File".to_string(),
                    kind: DialogKind::SaveFile { filter: filter.to_string(), extensions: vec![extension.to_string()], file_name: String::new() },
                    directory,
                };
                dialogs.ask(request);
            }
        });
    });
    !(close || !open)
}
