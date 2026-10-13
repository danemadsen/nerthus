//! Tools › Run Diagnostics… ("Watson"): troubleshooting tasks to choose
//! from, run with Start.
//!
//! - Create a diagnostics package: the logs, the settings and what Nerthus
//!   runs on, zipped into the documents folder.
//! - Run Nerthus in Verbose Mode, or with CPU only: Nerthus starts again
//!   that way.
//! - Reset to factory defaults: the settings go back to their defaults
//!   (backed up first if asked), and the first-run setup shows again.
//! - Clean up autosaves, caches and logs.
//! - Run Benchmark: every compute device scored.
//!
//! There are no licence or Houdini tasks.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crossbeam_channel::Receiver;
use egui::{Align, Id, Layout, RichText, Vec2};

use crate::editor::Editor;
use crate::theme;

/// The tasks, in Watson's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Task {
    Package,
    Verbose,
    CpuOnly,
    FactoryReset,
    Cleanup,
    Benchmark,
}

const TASKS: [(Task, &str); 6] = [
    (Task::Package, "Create a diagnostics package (required for technical support)"),
    (Task::Verbose, "Run Nerthus in Verbose Mode"),
    (Task::CpuOnly, "Run Nerthus with CPU only (Recommend for crashes)"),
    (Task::FactoryReset, "Reset to factory defaults (Clean install)"),
    (Task::Cleanup, "Cleanup autosaves, caches, and logs"),
    (Task::Benchmark, "Run Benchmark"),
];

/// The command line Nerthus starts again with once it has closed.
static RESTART: Mutex<Option<Vec<String>>> = Mutex::new(None);

/// The window while it is open.
pub struct DiagnosticsWindow {
    task: Task,
    /// "Backup current settings" (factory reset).
    backup: bool,
    /// What the tasks have said.
    output: String,
    benchmark: Option<Receiver<Option<String>>>,
}

impl DiagnosticsWindow {
    pub fn open() -> DiagnosticsWindow {
        DiagnosticsWindow { task: Task::Package, backup: true, output: String::new(), benchmark: None }
    }

    fn say(&mut self, line: impl AsRef<str>) {
        self.output.push_str(line.as_ref());
        self.output.push('\n');
    }
}

/// What the window asks of the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticsRequest {
    /// Close Nerthus (it then starts again as set up).
    Restart,
    /// The settings were reset: the interface takes them in.
    SettingsReset,
}

/// The window. Returns whether it is still open, and a request.
pub fn show(ctx: &egui::Context, editor: &mut Editor, window: &mut DiagnosticsWindow) -> (bool, Option<DiagnosticsRequest>) {
    poll_benchmark(window);
    if window.benchmark.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
    let mut request = None;
    let mut open = true;
    let mut close = false;
    let mut frame = egui::Window::new("Nerthus Diagnostics");
    frame = frame.id(Id::new("diagnostics"));
    frame = frame.collapsible(false);
    frame = frame.resizable(false);
    frame = frame.anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO);
    frame = frame.open(&mut open);
    frame.show(ctx, |ui| {
        ui.set_width(540.0);
        let text = "This program is designed to create diagnostics logs for technical support, find and fix potential issues in your Nerthus installation, and perform other tasks such as benchmarks.";
        ui.add(egui::Label::new(RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT)).wrap());
        ui.add_space(12.0);
        ui.label(RichText::new("Troubleshooting").font(theme::semibold(12.0)).color(theme::LABEL_EMPHASIS));
        ui.add_space(4.0);
        for (task, label) in TASKS {
            ui.radio_value(&mut window.task, task, RichText::new(label).font(theme::interface(12.0)));
            if task == Task::FactoryReset && window.task == Task::FactoryReset {
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    ui.checkbox(&mut window.backup, "Backup current settings");
                });
            }
        }
        ui.add_space(12.0);
        let mut output = window.output.as_str();
        let mut field = egui::TextEdit::multiline(&mut output);
        field = field.font(theme::monospace(11.0));
        field = field.desired_rows(8);
        field = field.desired_width(f32::INFINITY);
        egui::ScrollArea::vertical().max_height(180.0).stick_to_bottom(true).show(ui, |ui| ui.add(field));
        ui.add_space(8.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Close").clicked() {
                close = true;
            }
            let busy = window.benchmark.is_some();
            let started = ui.add_enabled_ui(!busy, |ui| crate::ui::help_windows::important_button(ui, "Start", 12.0, 87.0, Vec2::new(24.0, 6.0)));
            if started.inner.clicked() {
                request = start(editor, window);
            }
        });
    });
    (open && !close, request)
}

/// Start: the chosen task.
fn start(editor: &mut Editor, window: &mut DiagnosticsWindow) -> Option<DiagnosticsRequest> {
    match window.task {
        Task::Package => {
            match create_package(editor) {
                Ok(path) => {
                    window.say(format!("Diagnostics package created: {}", path.display()));
                    if let Some(folder) = path.parent() {
                        crate::export_build::open_folder(folder);
                    }
                }
                Err(error) => window.say(format!("Error: {error}")),
            }
            None
        }
        Task::Verbose => {
            restart_with(&["--verbose"]);
            Some(DiagnosticsRequest::Restart)
        }
        Task::CpuOnly => {
            if let Some(folder) = crate::user::data_folder() {
                let marked = std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(folder.join(nerthus_compute::CPU_ONLY_FILE), "!"));
                if let Err(error) = marked {
                    window.say(format!("Error: {error}"));
                    return None;
                }
            }
            restart_with(&[]);
            Some(DiagnosticsRequest::Restart)
        }
        Task::FactoryReset => match factory_reset(editor, window.backup) {
            Ok(backup) => {
                match backup {
                    Some(folder) => window.say(format!("Settings reset. The old settings are in {}", folder.display())),
                    None => window.say("Settings reset."),
                }
                Some(DiagnosticsRequest::SettingsReset)
            }
            Err(error) => {
                let folder = crate::user::data_folder().unwrap_or_default();
                window.say(format!("Settings could not be reset. Error: {error}\n\nManually delete the folder: {}", folder.display()));
                None
            }
        },
        Task::Cleanup => {
            let removed = cleanup();
            window.say(format!("Cleanup finished: {removed} files deleted."));
            None
        }
        Task::Benchmark => {
            window.say("Starting benchmark...");
            let (sender, receiver) = crossbeam_channel::unbounded();
            let spawned = std::thread::Builder::new().name("benchmark".to_string()).spawn(move || {
                let report = |line: String| {
                    let _ = sender.send(Some(line));
                };
                let scores = nerthus_compute::scan(report);
                if let Some(folder) = crate::user::data_folder() {
                    let _ = nerthus_compute::write_scores(&folder, &scores);
                }
                let _ = sender.send(None);
            });
            match spawned {
                Ok(_) => window.benchmark = Some(receiver),
                Err(error) => window.say(format!("Error: {error}")),
            }
            None
        }
    }
}

/// The benchmark's lines as they come; at its end, the scores.
fn poll_benchmark(window: &mut DiagnosticsWindow) {
    let Some(benchmark) = &window.benchmark else {
        return;
    };
    let mut lines = Vec::new();
    let mut finished = false;
    for message in benchmark.try_iter() {
        match message {
            Some(line) => lines.push(line),
            None => finished = true,
        }
    }
    for line in lines {
        window.say(line);
    }
    if finished {
        window.benchmark = None;
        window.say("\nBenchmark Results");
        let scores = crate::user::data_folder().map(|folder| nerthus_compute::read_scores(&folder)).unwrap_or_default();
        for (device, score) in scores.iter() {
            window.say(format!("{device}: {score:.0}"));
        }
        window.say("The higher the score, the better the performance.");
    }
}

/// Starts Nerthus again with these options once it has closed.
fn restart_with(extra: &[&str]) {
    // The options the restart sets are taken out of the command line
    // first.
    let dropped = ["--verbose", "--cpu-only", "--onboard"];
    let mut arguments: Vec<String> = std::env::args().skip(1).filter(|argument| !dropped.contains(&argument.as_str())).collect();
    for argument in extra {
        arguments.push(argument.to_string());
    }
    if let Ok(mut restart) = RESTART.lock() {
        *restart = Some(arguments);
    }
}

/// Starts the new Nerthus as this one closes.
pub fn restart_on_exit(mut exits: bevy::prelude::MessageReader<bevy::prelude::AppExit>) {
    if exits.read().next().is_none() {
        return;
    }
    let Some(arguments) = RESTART.lock().ok().and_then(|mut restart| restart.take()) else {
        return;
    };
    let Ok(program) = std::env::current_exe() else {
        return;
    };
    if let Err(error) = std::process::Command::new(program).args(arguments).spawn() {
        log::error!("Cannot start Nerthus again: {error}");
    }
}

/// The diagnostics package: `Nerthus_Diagnostics_<time>.zip` in the
/// documents folder.
fn create_package(editor: &Editor) -> std::io::Result<PathBuf> {
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    entries.push(("summary.json".to_string(), summary(editor).into_bytes()));
    entries.push(("Devices.txt".to_string(), devices().into_bytes()));
    entries.push(("SessionLog.txt".to_string(), crate::logging::session_text().into_bytes()));
    if let Some(folder) = crate::user::data_folder() {
        let settings = ["user.options", nerthus_engine::paths::PREFERENCES_FILE, "recent-files.mru", nerthus_compute::BENCHMARK_FILE];
        for name in settings {
            if let Ok(bytes) = std::fs::read(folder.join(name)) {
                entries.push((format!("Settings/{name}"), bytes));
            }
        }
        // The ten newest logs.
        let logs = folder.join("Logs");
        let mut files: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
        for entry in std::fs::read_dir(&logs).into_iter().flatten().flatten() {
            let modified = entry.metadata().and_then(|metadata| metadata.modified()).unwrap_or(std::time::UNIX_EPOCH);
            files.push((modified, entry.path()));
        }
        files.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
        for (_, path) in files.into_iter().take(10) {
            if let (Some(name), Ok(bytes)) = (path.file_name(), std::fs::read(&path)) {
                entries.push((format!("Logs/{}", name.to_string_lossy()), bytes));
            }
        }
    }
    let folder = editor.user.preferences.documents_folder();
    std::fs::create_dir_all(&folder)?;
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
    let path = folder.join(format!("Nerthus_Diagnostics_{stamp}.zip"));
    let borrowed: Vec<(&str, &[u8])> = entries.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice())).collect();
    std::fs::write(&path, stored_zip(&borrowed))?;
    Ok(path)
}

/// `summary.json`: the version and the machine.
fn summary(editor: &Editor) -> String {
    let cores = std::thread::available_parallelism().map(usize::from).unwrap_or(1);
    let memory = crate::memory::process_bytes().unwrap_or(0);
    let value = serde_json::json!({
        "Nerthus Version": env!("CARGO_PKG_VERSION"),
        "OS": std::env::consts::OS,
        "Architecture": std::env::consts::ARCH,
        "Cores": cores,
        "Process Memory": memory,
        "Engine Cache": editor.cache_bytes,
        "Verbose Logging": crate::logging::verbose(),
        "CPU Only": nerthus_compute::is_cpu_only(),
        "Project": editor.document.path.as_ref().map(|path| path.display().to_string()),
    });
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

/// `Devices.txt`: the compute devices and which are on.
fn devices() -> String {
    let mut text = String::new();
    let active: Vec<String> = nerthus_compute::active_devices().into_iter().map(|device| device.id).collect();
    let (devices, problems) = nerthus_compute::available_devices();
    for device in devices {
        let state = if active.contains(&device.id) { "active" } else { "inactive" };
        text.push_str(&format!(" DEVICE: {} ({}) [{state}]\n    {}\n", device.device_name, device.backend_name, device.id));
    }
    if !problems.is_empty() {
        text.push_str(&format!("\n{problems}\n"));
    }
    text
}

/// Settings back to their defaults, the old ones copied to
/// `Settings Backup <time>` first if asked. Returns the backup's folder.
fn factory_reset(editor: &mut Editor, backup: bool) -> std::io::Result<Option<PathBuf>> {
    let Some(folder) = crate::user::data_folder() else {
        return Ok(None);
    };
    let names = ["user.options", "preferences.options", "onboard.dat", crate::logging::VERBOSE_FILE, nerthus_compute::CPU_ONLY_FILE];
    let mut backup_folder = None;
    if backup {
        let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
        let target = folder.join(format!("Settings Backup {stamp}"));
        std::fs::create_dir_all(&target)?;
        for name in names {
            let source = folder.join(name);
            if source.exists() {
                std::fs::copy(&source, target.join(name))?;
            }
        }
        backup_folder = Some(target);
    }
    for name in names {
        let path = folder.join(name);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    editor.user.options = crate::user::UserOptions::new();
    editor.user.preferences = crate::user::Preferences::new();
    editor.user.options.mark_changed();
    editor.user.preferences.mark_changed();
    Ok(backup_folder)
}

/// The data folder's `Logs` (but the log being written), `Cache` and
/// `Autosaves` emptied, as Watson deletes them. Returns how many files went.
fn cleanup() -> usize {
    let mut removed = 0;
    let current_log = crate::logging::current_log_file();
    for name in ["Logs", "Cache", "Autosaves"] {
        if let Some(folder) = crate::files::data_subfolder(name) {
            removed += delete_files(&folder, current_log.as_deref());
        }
    }
    removed
}

fn delete_files(folder: &Path, keep: Option<&Path>) -> usize {
    let mut removed = 0;
    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            removed += delete_files(&path, keep);
            let _ = std::fs::remove_dir(&path);
            continue;
        }
        if Some(path.as_path()) == keep {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// A zip archive of files stored as they are (no compression).
fn stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut archive = Vec::new();
    let mut directory = Vec::new();
    for (name, contents) in entries {
        let crc = crc32(contents);
        let offset = archive.len() as u32;
        let name_bytes = name.as_bytes();
        let size = contents.len() as u32;
        let header = |signature: u32, central: bool| {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&signature.to_le_bytes());
            if central {
                bytes.extend_from_slice(&20u16.to_le_bytes());
            }
            bytes.extend_from_slice(&20u16.to_le_bytes());
            bytes.extend_from_slice(&0x0800u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0x21u16.to_le_bytes());
            bytes.extend_from_slice(&crc.to_le_bytes());
            bytes.extend_from_slice(&size.to_le_bytes());
            bytes.extend_from_slice(&size.to_le_bytes());
            bytes.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes
        };
        archive.extend_from_slice(&header(0x0403_4b50, false));
        archive.extend_from_slice(name_bytes);
        archive.extend_from_slice(contents);
        directory.extend_from_slice(&header(0x0201_4b50, true));
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u32.to_le_bytes());
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name_bytes);
    }
    let directory_offset = archive.len() as u32;
    let count = entries.len() as u16;
    archive.extend_from_slice(&directory);
    archive.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes());
    archive.extend_from_slice(&count.to_le_bytes());
    archive.extend_from_slice(&count.to_le_bytes());
    archive.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    archive.extend_from_slice(&directory_offset.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes());
    archive
}

/// CRC-32 (IEEE), as zip files carry it.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_the_standard() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    /// Archive tools read what Nerthus zips (`unzip -t`).
    #[test]
    #[ignore]
    fn write_sample_zip() {
        let path = std::env::var("NERTHUS_SAMPLE_OUT").expect("set NERTHUS_SAMPLE_OUT");
        let long_text = "Nerthus log line\n".repeat(500);
        let entries: [(&str, &[u8]); 3] = [("summary.json", b"{}"), ("Logs/one.txt", long_text.as_bytes()), ("Settings/user.options", b"")];
        std::fs::write(path, stored_zip(&entries)).unwrap();
    }

    #[test]
    fn zip_has_its_entries() {
        let archive = stored_zip(&[("a.txt", b"hello"), ("b/c.txt", b"world")]);
        assert_eq!(&archive[0..4], &0x0403_4b50u32.to_le_bytes());
        let end = archive.len() - 22;
        assert_eq!(&archive[end..end + 4], &0x0605_4b50u32.to_le_bytes());
        assert_eq!(u16::from_le_bytes([archive[end + 10], archive[end + 11]]), 2);
    }
}
