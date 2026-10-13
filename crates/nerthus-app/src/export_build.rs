//! Build and Export from the editor: the saved project built on its own
//! thread, as `nerthus-build` would build the saved file, with its
//! progress for the build window.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender};
use nerthus_engine::macros::load_project_macros_from;
use nerthus_engine::{Engine, Project};
use nerthus_export::build::{self, BuildDefinition, BuildEvent, BuildReport, ExportPreferences};

/// What the build thread reports.
enum Message {
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

/// A build in progress or just finished.
pub struct ExportBuild {
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
        let cancel = Arc::new(AtomicBool::new(false));
        let thread_path = path.to_path_buf();
        let thread_cancel = cancel.clone();
        std::thread::Builder::new()
            .name("build-and-export".to_string())
            .spawn(move || run(&thread_path, &preferences, &thread_cancel, &sender))
            .expect("the build thread should start");
        let project_name = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
        Self {
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
            finished: false,
            cancelled: false,
            refusal: None,
            open_folder,
            started: Instant::now(),
            finished_at: None,
            names: Vec::new(),
            messages,
            cancel,
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
            // What a command-line build prints to its console; the build
            // window has its own log (the report keeps the text).
            BuildEvent::Printed(_) => {}
        }
    }

    /// Fraction of the nodes built.
    pub fn progress(&self) -> f32 {
        if self.finished {
            return 1.0;
        }
        if self.total == 0 {
            return 0.0;
        }
        self.built as f32 / self.total as f32
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
