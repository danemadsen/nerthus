//! Build and Export from the editor:
//!
//! - the confirmation's choice carried out (`B.VerifyAndBuild` after its
//!   dialog): "Open folder after build finishes" noted, the project saved,
//!   the pre-build checks, and the offer to write out the editor's own
//!   maps when they are what the build would make (Save cached build);
//! - the build: the saved project built on its own thread, as
//!   `nerthus-build` would build the saved file, with its progress for
//!   the build window; or, for Close Nerthus and Build, `nerthus-build`
//!   itself in a window of its own while the editor closes;
//! - Save cached build: the build thread writes the maps it holds, with
//!   its progress for the progress window.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use nerthus_engine::macros::load_project_macros_from;
use nerthus_engine::{Engine, NodeData, NodeState, Project, Region, StateOverride, Terrain};
use nerthus_export::build::{self, BuildDefinition, BuildEvent, BuildReport, ExportPreferences, OutputType};
use web_time::Instant;

use crate::editor::{DialogRequest, Editor};
use crate::files::FileCommand;

/// What a build reports as it goes, from its own thread or, for Save
/// cached build, from the build thread.
pub(crate) enum Message {
    Event(BuildEvent),
    Names(Vec<(i32, String)>),
    /// Boxed: a report is far larger than the other messages.
    Finished(Box<Result<BuildReport, String>>),
}

/// A line of the build window's log.
#[derive(Clone, Debug, PartialEq)]
pub enum LogLine {
    Built { node: String, milliseconds: u64 },
    Failed { node: String, error: String },
    Wrote(PathBuf),
    Problem(String),
    /// A bucket starting, or a step after the buckets.
    Step(String),
}

/// The confirmation's choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildChoice {
    /// "Open folder after build finishes".
    pub open_folder: bool,
    /// Close Nerthus and Build.
    pub close: bool,
}

/// What came of the choice before anything is built.
#[derive(Clone, Debug, PartialEq)]
pub enum BuildStart {
    /// The project has no file yet: Save As is asked for, and the choice
    /// goes on once the project is saved.
    Saving,
    /// The project could not be saved (the editor has said why).
    NotSaved,
    /// The pre-build checks refused the build (`Validation failed`).
    Refused(String),
    /// The build can go ahead; `cached` when the editor's own maps can be
    /// written out instead (Save cached build).
    Ready { cached: bool },
}

/// The largest single-image build Save cached build is offered for.
const CACHED_EXPORT_LIMIT: usize = 8192;

/// The confirmation's choice carried out up to the build: "Open folder
/// after build finishes" noted in the build settings, the project saved
/// (Save As first for a new one), the pre-build checks run, and whether
/// the editor holds what the build would make.
pub fn prepare(editor: &mut Editor, choice: BuildChoice) -> BuildStart {
    if choice.open_folder {
        editor.document.set_build_setting("OpenFolder", serde_json::Value::Bool(true));
    }
    let Some(path) = editor.document.path.clone() else {
        editor.pending_build = Some(choice);
        editor.dialog = Some(DialogRequest::SaveAs);
        return BuildStart::Saving;
    };
    editor.save_to(&path);
    if editor.document.dirty {
        return BuildStart::NotSaved;
    }
    let definition = editor.document.build_definition();
    if let Err(refusal) = build::check(editor.document.terrain(), definition.build_type) {
        return BuildStart::Refused(refusal.to_string());
    }
    BuildStart::Ready { cached: holds_build(editor, &definition) }
}

/// Whether the editor holds what the build would make: the preview
/// resolution is the build's, the build writes single images of 8K at
/// most, and every node marked for export is built (or baked) with its
/// marked outputs at the build's size. The build thread must be running to
/// write them.
fn holds_build(editor: &Editor, definition: &BuildDefinition) -> bool {
    let same_size = editor.preview_resolution == definition.resolution;
    let single_image = definition.output_type == OutputType::SingleImage && definition.resolution <= CACHED_EXPORT_LIMIT;
    if !same_size || !single_image || !editor.engine_active {
        return false;
    }
    let terrain = editor.document.terrain();
    let exporting = build::exporting_nodes(terrain);
    let mut nodes = exporting.iter().filter_map(|id| terrain.nodes.get(id));
    nodes.all(|node| holds_node(editor, node, definition.resolution))
}

/// Whether the engine reported `node` built (or holds it baked) with each
/// of its marked outputs at `resolution`.
fn holds_node(editor: &Editor, node: &NodeData, resolution: usize) -> bool {
    let state = editor.status(node.id).state;
    let baked = StateOverride::of(node) == StateOverride::Baked;
    if !(state == NodeState::Built || state == NodeState::Baked || baked) {
        return false;
    }
    let Some(cache) = editor.node_caches.get(&node.id) else {
        return false;
    };
    let mut marked = node.ports.iter().filter(|port| port.is_output() && port.is_exporting);
    marked.all(|port| cache.ports.iter().any(|cached| cached.name == port.name && cached.width == resolution))
}

/// The build once the checks passed: here, with the build window, or —
/// Close Nerthus and Build — `nerthus-build` in a window of its own, the
/// editor then closing as File › Exit does. Why `nerthus-build` could not
/// start, if it could not.
pub fn run_build(editor: &mut Editor, close: bool) -> Result<(), String> {
    let Some(path) = editor.document.path.clone() else {
        return Ok(());
    };
    if !close {
        editor.launch_export_build(&path);
        return Ok(());
    }
    launch_command_line_build(&path).map_err(|error| format!("nerthus-build could not be started: {error}"))?;
    editor.report(format!("Building {} with nerthus-build", editor.document.file_name()));
    editor.files.requests.push(FileCommand::Exit);
    Ok(())
}

/// Save cached build: the build thread writes the maps it holds into a
/// new build folder, the progress window following it. `close` is the
/// choice it came from, for the normal build offered if it fails.
pub fn export_cached(editor: &mut Editor, close: bool) {
    let Some(project_path) = editor.document.path.clone() else {
        return;
    };
    let definition = editor.document.build_definition();
    let terrain = editor.document.terrain();
    let names = terrain.nodes.values().map(|node| (node.id, node.name.clone())).collect();
    let project_name = project_path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let (sender, messages) = crossbeam_channel::unbounded();
    let mut tracker = ExportBuild::following(project_name, definition.resolution, definition.open_folder, messages);
    tracker.kind = ExportKind::Cached { close };
    tracker.names = names;
    let job = CachedExportJob {
        definition,
        preferences: editor.user.preferences.export_preferences(),
        region: editor.previewed_region(),
        project_path,
        messages: sender,
    };
    editor.report("Exporting all assets, please wait this may take a while...");
    editor.pending_cached_export = Some(job);
    editor.export_build = Some(tracker);
    editor.needs_build = true;
}

/// Save cached build's work, done on the build thread, which holds the
/// editor's maps.
pub struct CachedExportJob {
    definition: BuildDefinition,
    preferences: ExportPreferences,
    /// The region the editor previews (`[Region]` in the destination).
    region: Option<Region>,
    project_path: PathBuf,
    messages: Sender<Message>,
}

impl CachedExportJob {
    /// Writes the maps `engine` holds for `terrain`'s nodes marked for
    /// export, telling the progress window how it goes.
    pub fn run(self, terrain: &Terrain, engine: &Engine) {
        let mut on_event = |event: BuildEvent| {
            let _ = self.messages.send(Message::Event(event));
        };
        let exported = build::export_built(terrain, engine, &self.project_path, &self.definition, &self.preferences, self.region.as_ref(), &mut on_event);
        if let Err(refusal) = &exported {
            log::error!("Cached export failed. {refusal}");
        }
        let result = exported.map_err(|refusal| refusal.to_string());
        let _ = self.messages.send(Message::Finished(Box::new(result)));
    }

    /// The export given up before it started, for `reason`.
    pub fn abandon(self, reason: &str) {
        let _ = self.messages.send(Message::Finished(Box::new(Err(reason.to_string()))));
    }
}

/// What an [`ExportBuild`] follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportKind {
    /// A build on its own thread.
    Build,
    /// Save cached build; `close` when it came from Close Nerthus and
    /// Build.
    Cached { close: bool },
}

/// A build in progress or just finished.
pub struct ExportBuild {
    pub kind: ExportKind,
    pub project_name: String,
    pub resolution: usize,
    pub folder: Option<PathBuf>,
    pub total: usize,
    pub built: usize,
    pub current: Option<String>,
    pub log: Vec<LogLine>,
    pub files: usize,
    pub failures: usize,
    /// The bucket a Hybrid or tiled build is on (from 1) and how many.
    pub bucket: Option<(usize, usize)>,
    /// The step after the buckets ("Blending Edges", …).
    pub stage: Option<String>,
    /// Save cached build: the node writing its maps (from 1) and how
    /// many.
    pub exporting: Option<(usize, usize)>,
    pub finished: bool,
    pub cancelled: bool,
    /// Why the build did not start.
    pub refusal: Option<String>,
    pub open_folder: bool,
    started: Instant,
    finished_at: Option<Instant>,
    names: Vec<(i32, String)>,
    messages: Receiver<Message>,
    cancel: Arc<AtomicBool>,
}

impl ExportBuild {
    /// Builds the project saved at `path`.
    pub fn start(path: &Path, resolution: usize, open_folder: bool, preferences: ExportPreferences) -> Self {
        let (sender, messages) = crossbeam_channel::unbounded();
        let project_name = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
        let build = Self::following(project_name, resolution, open_folder, messages);
        let thread_path = path.to_path_buf();
        let thread_cancel = build.cancel.clone();
        std::thread::Builder::new()
            .name("build-and-export".to_string())
            .spawn(move || run(&thread_path, &preferences, &thread_cancel, &sender))
            .expect("the build thread should start");
        build
    }

    /// A build that reports on `messages`.
    fn following(project_name: String, resolution: usize, open_folder: bool, messages: Receiver<Message>) -> Self {
        Self {
            kind: ExportKind::Build,
            project_name,
            resolution,
            folder: None,
            total: 0,
            built: 0,
            current: None,
            log: Vec::new(),
            files: 0,
            failures: 0,
            bucket: None,
            stage: None,
            exporting: None,
            finished: false,
            cancelled: false,
            refusal: None,
            open_folder,
            started: Instant::now(),
            finished_at: None,
            names: Vec::new(),
            messages,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn seconds(&self) -> f32 {
        let end = self.finished_at.unwrap_or_else(Instant::now);
        (end - self.started).as_secs_f32()
    }

    fn name(&self, node: i32) -> String {
        let known = self.names.iter().find(|(id, _)| *id == node);
        known.map(|(_, name)| name.clone()).unwrap_or_else(|| node.to_string())
    }

    /// Takes in what the thread reported; true when the build just
    /// finished.
    pub fn poll(&mut self) -> bool {
        let mut just_finished = false;
        while let Ok(message) = self.messages.try_recv() {
            match message {
                Message::Names(names) => self.names = names,
                Message::Event(event) => self.apply(event),
                Message::Finished(boxed) => {
                    let result = *boxed;
                    self.finished = true;
                    self.finished_at = Some(Instant::now());
                    self.current = None;
                    just_finished = true;
                    match result {
                        Ok(report) => {
                            self.cancelled = report.cancelled;
                            self.folder = Some(report.folder);
                        }
                        Err(refusal) => self.refusal = Some(refusal),
                    }
                }
            }
        }
        just_finished
    }

    fn apply(&mut self, event: BuildEvent) {
        match event {
            BuildEvent::Started { folder, nodes } => {
                self.folder = Some(folder);
                self.total = nodes;
            }
            BuildEvent::BucketStarted { index, count } => {
                self.bucket = Some((index + 1, count));
                self.log.push(LogLine::Step(format!("Bucket {} of {count}", index + 1)));
            }
            BuildEvent::Stage(stage) => {
                self.current = None;
                self.log.push(LogLine::Step(stage.clone()));
                self.stage = Some(stage);
            }
            BuildEvent::NodeStarted { node } => self.current = Some(self.name(node)),
            BuildEvent::NodeFinished { node, milliseconds, error } => {
                self.built += 1;
                let name = self.name(node);
                let line = match error {
                    Some(error) => {
                        self.failures += 1;
                        LogLine::Failed { node: name, error }
                    }
                    None => LogLine::Built { node: name, milliseconds },
                };
                self.log.push(line);
            }
            BuildEvent::FileWritten { path, .. } => {
                self.files += 1;
                self.log.push(LogLine::Wrote(path));
            }
            BuildEvent::FileFailed { node, error, .. } => {
                self.failures += 1;
                let name = self.name(node);
                self.log.push(LogLine::Problem(format!("{name}: {error}")));
            }
            BuildEvent::Exporting { node, index, count } => {
                self.exporting = Some((index, count));
                self.current = Some(self.name(node));
            }
            // What a command-line build prints to its console; the build
            // window has its own log (the report keeps the text).
            BuildEvent::Printed(_) => {}
        }
    }

    /// Fraction done: of the nodes built, or for Save cached build, of the
    /// nodes that started writing.
    pub fn progress(&self) -> f32 {
        if self.finished {
            return 1.0;
        }
        if let Some((index, count)) = self.exporting {
            return index as f32 / count.max(1) as f32;
        }
        if self.total == 0 {
            return 0.0;
        }
        self.built as f32 / self.total as f32
    }

    /// Finished with everything written.
    pub fn succeeded(&self) -> bool {
        self.finished && self.refusal.is_none() && !self.cancelled && self.failures == 0
    }

    /// What Save cached build's progress window says it is doing.
    pub fn exporting_text(&self) -> String {
        match (self.exporting, &self.current) {
            (Some((index, count)), Some(name)) => format!("Exporting [{index:02}/{count:02}] - {name}..."),
            _ => "Exporting all assets, please wait this may take a while...".to_string(),
        }
    }

    /// What the editor says once the build is done.
    pub fn summary(&self) -> String {
        if let ExportKind::Cached { .. } = self.kind {
            let summary = if self.succeeded() { "Build exported!" } else { "Cached export failed." };
            return summary.to_string();
        }
        if let Some(refusal) = &self.refusal {
            return format!("Build refused: {refusal}");
        }
        if self.cancelled {
            return "Build cancelled".to_string();
        }
        format!("Build finished in {:.1} s: {} files, {} problems", self.seconds(), self.files, self.failures)
    }

    /// Whether the build folder opens once the build is done (the build
    /// settings say so, and it built; a cached export must have written
    /// everything).
    pub fn opens_folder(&self) -> bool {
        let built = self.refusal.is_none() && !self.cancelled;
        let complete = self.kind == ExportKind::Build || self.failures == 0;
        self.open_folder && built && complete
    }
}

/// The build thread: the saved file opened as a command-line build opens
/// it, then built and exported.
fn run(path: &Path, preferences: &ExportPreferences, cancel: &AtomicBool, sender: &Sender<Message>) {
    let result = build(path, preferences, cancel, sender);
    let _ = sender.send(Message::Finished(Box::new(result)));
}

fn build(path: &Path, preferences: &ExportPreferences, cancel: &AtomicBool, sender: &Sender<Message>) -> Result<BuildReport, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    let project = Project::parse(&text).map_err(|error| format!("Cannot open {}: {error}", path.display()))?;
    let asset = &project.assets[0];
    let names = asset.terrain.nodes.values().map(|node| (node.id, node.name.clone())).collect();
    let _ = sender.send(Message::Names(names));

    let registry = Arc::new(nerthus_nodes::registry());
    let mut engine = Engine::new(registry);
    engine.automation = asset.automation.clone();
    let macro_ids = project.macro_ids();
    if !macro_ids.is_empty() {
        let folders = crate::files::macro_folders(Some(path));
        let (library, _problems) = load_project_macros_from(&macro_ids, &folders);
        engine.macros = library;
    }

    let mut definition = BuildDefinition::from_json(asset.extra.get("BuildDefinition"));
    build::set_up_region(&asset.terrain, &mut definition, None);
    let project_id = project.extra.get("Id").and_then(serde_json::Value::as_str).unwrap_or_default();
    let bakes = preferences.bake_store(path, project_id);
    let mut on_event = |event: BuildEvent| {
        let _ = sender.send(Message::Event(event));
    };
    build::run(&asset.terrain, &mut engine, path, &definition, preferences, Some(&bakes), cancel, &mut on_event).map_err(|refusal| refusal.to_string())
}

/// `nerthus-build`, beside the editor's own executable.
fn command_line_build_program() -> std::io::Result<PathBuf> {
    let editor = std::env::current_exe()?;
    let program = editor.with_file_name(format!("nerthus-build{}", std::env::consts::EXE_SUFFIX));
    if !program.is_file() {
        let message = format!("{} is missing", program.display());
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, message));
    }
    Ok(program)
}

/// Close Nerthus and Build's build: `nerthus-build build <project>
/// --pause` in a terminal window of its own, which stays until Enter is
/// pressed (or the build folder opens): Terminal on macOS, a new console
/// on Windows, the first terminal emulator found elsewhere (without one,
/// the build runs unseen, leaving its log and report).
pub fn launch_command_line_build(project: &Path) -> std::io::Result<()> {
    let program = command_line_build_program()?;
    launch_in_terminal(&program, project)
}

/// Single quotes for `sh`, any single quote inside closed, escaped and
/// reopened.
#[cfg(not(windows))]
fn shell_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// A script in the temporary folder that deletes itself as it starts,
/// then runs `nerthus-build build <project> --pause`.
#[cfg(unix)]
fn write_build_script(program: &Path, project: &Path, extension: &str) -> std::io::Result<PathBuf> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt;

    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|since| since.as_nanos()).unwrap_or(0);
    let script = std::env::temp_dir().join(format!("nerthus-build-{}-{stamp}.{extension}", std::process::id()));
    let program_text = shell_quoted(&program.to_string_lossy());
    let project_text = shell_quoted(&project.to_string_lossy());
    let text = format!("#!/bin/sh\nrm -f -- \"$0\"\nexec {program_text} build {project_text} --pause\n");
    let mut options = std::fs::File::options();
    options.write(true);
    options.create_new(true);
    options.mode(0o700);
    let mut file = options.open(&script)?;
    file.write_all(text.as_bytes())?;
    Ok(script)
}

/// A `.command` script opens in Terminal.
#[cfg(target_os = "macos")]
fn launch_in_terminal(program: &Path, project: &Path) -> std::io::Result<()> {
    let script = write_build_script(program, project, "command")?;
    std::process::Command::new("open").arg(&script).spawn()?;
    Ok(())
}

#[cfg(windows)]
fn launch_in_terminal(program: &Path, project: &Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;

    /// `CREATE_NEW_CONSOLE`: the build gets a console window of its own.
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let mut command = std::process::Command::new(program);
    command.arg("build").arg(project).arg("--pause");
    command.creation_flags(CREATE_NEW_CONSOLE);
    command.spawn()?;
    Ok(())
}

/// A browser page cannot start programs.
#[cfg(not(any(unix, windows)))]
fn launch_in_terminal(_program: &Path, _project: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, crate::web::unavailable("Running the command-line build")))
}

/// The first terminal emulator there is runs the script.
#[cfg(all(unix, not(target_os = "macos")))]
fn launch_in_terminal(program: &Path, project: &Path) -> std::io::Result<()> {
    let script = write_build_script(program, project, "sh")?;
    // Each terminal and how it is told to run a command.
    let preferred = std::env::var("TERMINAL").ok();
    let mut terminals: Vec<(String, &str)> = Vec::new();
    if let Some(terminal) = preferred {
        terminals.push((terminal, "-e"));
    }
    for (terminal, flag) in [("x-terminal-emulator", "-e"), ("gnome-terminal", "--"), ("konsole", "-e"), ("xfce4-terminal", "-x"), ("xterm", "-e")] {
        terminals.push((terminal.to_string(), flag));
    }
    for (terminal, flag) in terminals {
        let mut command = std::process::Command::new(&terminal);
        command.arg(flag).arg("sh").arg(&script);
        match command.spawn() {
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        }
    }
    let _ = std::fs::remove_file(&script);
    let mut command = std::process::Command::new(program);
    command.arg("build").arg(project);
    command.spawn()?;
    Ok(())
}

/// Opens a folder in the system's file browser.
pub fn open_folder(folder: &Path) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    if let Err(error) = std::process::Command::new(program).arg(folder).spawn() {
        log::warn!("cannot open {}: {error}", folder.display());
    }
}

/// Opens a web page in the default browser.
#[cfg(not(target_arch = "wasm32"))]
pub fn open_url(address: &str) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    if let Err(error) = std::process::Command::new(program).arg(address).spawn() {
        log::warn!("cannot open {address}: {error}");
    }
}

/// The web build opens it in a new tab.
#[cfg(target_arch = "wasm32")]
pub fn open_url(address: &str) {
    let opened = web_sys::window().and_then(|window| window.open_with_url_and_target(address, "_blank").ok().flatten());
    if opened.is_none() {
        log::warn!("cannot open {address}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::editor::NodeStatus;
    use crate::worker::{CachedPort, NodeCache};
    use nerthus_engine::RenderIntent;

    /// An editor holding a Perlin marked for export as a preview build at
    /// `resolution` left it (built, its output kept at that size), the
    /// build at 1024.
    fn editor_holding(resolution: usize) -> (Editor, i32) {
        let mut document = Document::new();
        let perlin = document.add_node("Perlin", [26000.0, 26000.0]).unwrap();
        document.toggle_export(&[perlin], nerthus_export::FileFormat::Png16);
        document.set_build_setting("Resolution", serde_json::json!(1024));
        let mut editor = Editor::new(document);
        editor.preview_resolution = resolution;
        let status = NodeStatus { state: NodeState::Built, ..NodeStatus::default() };
        editor.statuses.insert(perlin, status);
        let port = CachedPort { name: "Out".to_string(), width: resolution, height: resolution, channels: 1, intent: RenderIntent::Default };
        let cache = NodeCache { ports: vec![port], ..NodeCache::default() };
        editor.node_caches.insert(perlin, cache);
        (editor, perlin)
    }

    #[test]
    fn the_editors_maps_are_offered_when_they_are_the_builds() {
        let (editor, perlin) = editor_holding(1024);
        let node = editor.document.node(perlin).unwrap();
        let marked = node.ports.iter().any(|port| port.is_output() && port.is_exporting);
        assert!(marked);
        assert!(holds_build(&editor, &editor.document.build_definition()));

        // Previewed at another size, or not built: not offered.
        let (editor, _) = editor_holding(512);
        assert!(!holds_build(&editor, &editor.document.build_definition()));
        let (mut editor, perlin) = editor_holding(1024);
        editor.statuses.remove(&perlin);
        assert!(!holds_build(&editor, &editor.document.build_definition()));
        // Tiles, or a single image past 8K, or the engine paused: not offered.
        let (mut editor, _) = editor_holding(1024);
        editor.document.set_build_setting("OutputType", serde_json::json!("CroppedTiles"));
        assert!(!holds_build(&editor, &editor.document.build_definition()));
        let (mut editor, _) = editor_holding(16384);
        editor.document.set_build_setting("Resolution", serde_json::json!(16384));
        assert!(!holds_build(&editor, &editor.document.build_definition()));
        let (mut editor, _) = editor_holding(1024);
        editor.engine_active = false;
        assert!(!holds_build(&editor, &editor.document.build_definition()));
    }

    #[test]
    fn save_cached_build_writes_what_the_preview_built() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|since| since.as_nanos()).unwrap_or(0);
        let root = std::env::temp_dir().join(format!("nerthus-cached-build-{nanos}"));
        std::fs::create_dir_all(&root).unwrap();
        let mut document = Document::new();
        let perlin = document.add_node("Perlin", [26000.0, 26000.0]).unwrap();
        document.toggle_export(&[perlin], nerthus_export::FileFormat::Png16);
        document.set_build_setting("Resolution", serde_json::json!(256));
        document.set_build_setting("Destination", serde_json::json!(root.join("cached").to_string_lossy()));
        let project_path = root.join("Cached.terrain");
        let mut editor = Editor::new(document);
        editor.save_to(&project_path);
        editor.preview_resolution = 256;
        editor.select(Some(perlin));

        // The preview builds, then the build thread writes its maps.
        export_cached(&mut editor, false);
        let worker = crate::worker::BuildWorker::start();
        editor.request_build(&worker);
        let deadline = Instant::now() + std::time::Duration::from_secs(120);
        loop {
            let _ = worker.poll();
            let tracker = editor.export_build.as_mut().unwrap();
            if tracker.poll() {
                break;
            }
            assert!(Instant::now() < deadline, "the export never finished");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let tracker = editor.export_build.as_ref().unwrap();
        assert!(tracker.succeeded(), "{:?} {:?}", tracker.refusal, tracker.log);
        assert_eq!(tracker.files, 1);
        assert_eq!(tracker.summary(), "Build exported!");

        // What a build of the saved project writes.
        let text = std::fs::read_to_string(&project_path).unwrap();
        let project = Project::parse(&text).unwrap();
        let mut definition = BuildDefinition::from_json(project.assets[0].extra.get("BuildDefinition"));
        definition.destination = root.join("built").to_string_lossy().into_owned();
        let mut engine = Engine::new(Arc::new(nerthus_nodes::registry()));
        let cancel = AtomicBool::new(false);
        let preferences = ExportPreferences::default();
        build::run(project.terrain(), &mut engine, &project_path, &definition, &preferences, None, &cancel, &mut |_| {}).unwrap();
        let name = format!("{}_Out.png", project.terrain().nodes[&perlin].name);
        let cached = std::fs::read(root.join("cached").join(&name)).unwrap();
        let built = std::fs::read(root.join("built").join(&name)).unwrap();
        assert!(cached == built, "the cached export differs from the build");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_choice_saves_then_checks() {
        // A new project asks where to save first, and goes on afterwards.
        let (mut editor, _) = editor_holding(1024);
        let choice = BuildChoice { open_folder: true, close: true };
        assert_eq!(prepare(&mut editor, choice), BuildStart::Saving);
        assert_eq!(editor.pending_build, Some(choice));
        assert!(matches!(editor.dialog, Some(DialogRequest::SaveAs)));
        assert!(editor.document.build_definition().open_folder);

        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|since| since.as_nanos()).unwrap_or(0);
        let path = std::env::temp_dir().join(format!("nerthus-build-choice-{nanos}.terrain"));
        editor.save_to(&path);
        assert_eq!(prepare(&mut editor, choice), BuildStart::Ready { cached: true });

        // Nothing marked for export: refused, in the checks' words.
        let mut empty = Editor::new(Document::new());
        empty.save_to(&path);
        let refused = prepare(&mut empty, BuildChoice { open_folder: false, close: false });
        assert_eq!(refused, BuildStart::Refused("There are no nodes marked for export".to_string()));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    #[cfg(not(windows))]
    fn shell_quoting_survives_quotes_and_spaces() {
        assert_eq!(shell_quoted("/a b/c"), "'/a b/c'");
        assert_eq!(shell_quoted("it's"), "'it'\\''s'");
    }

    #[test]
    #[cfg(unix)]
    fn the_build_script_runs_the_build_and_goes() {
        // `echo` stands in for nerthus-build: the script hands it the
        // project, quotes and all.
        let project = Path::new("/projects/The Valley's Edge.terrain");
        let script = write_build_script(Path::new("/bin/echo"), project, "sh").unwrap();
        let output = std::process::Command::new("sh").arg(&script).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout), "build /projects/The Valley's Edge.terrain --pause\n");
        assert!(!script.exists());
    }

    #[test]
    fn cached_exports_say_what_they_write() {
        let (_sender, messages) = crossbeam_channel::unbounded();
        let mut tracker = ExportBuild::following("Canyon".to_string(), 1024, true, messages);
        tracker.kind = ExportKind::Cached { close: false };
        tracker.names = vec![(7, "Erosion".to_string())];
        assert_eq!(tracker.exporting_text(), "Exporting all assets, please wait this may take a while...");
        tracker.apply(BuildEvent::Exporting { node: 7, index: 1, count: 3 });
        assert_eq!(tracker.exporting_text(), "Exporting [01/03] - Erosion...");
        assert!((tracker.progress() - 1.0 / 3.0).abs() < 1e-6);
        tracker.finished = true;
        assert!(tracker.succeeded());
        assert_eq!(tracker.summary(), "Build exported!");
        assert!(tracker.opens_folder());
        tracker.failures = 1;
        assert_eq!(tracker.summary(), "Cached export failed.");
        assert!(!tracker.opens_folder());
    }
}
