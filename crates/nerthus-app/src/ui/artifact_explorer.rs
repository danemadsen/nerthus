//! Tools › Artifact Explorer… (`ArtifactExplorerWindow`): the folders
//! Nerthus leaves files in, each with its size and number of files (worked
//! out in the background), a link that opens it, and Delete…, which cleans
//! up all its files or those older than 45 days. Builds can only be opened
//! (`IsImportant`). Cleaned-up files go to the Trash (the Recycle Bin)
//! where the system has one.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crossbeam_channel::Receiver;
use egui::{Align, Color32, Id, Layout, RichText, Stroke, Vec2};

use crate::editor::Editor;
use crate::theme;

/// `ArtifactControl`: a folder and what it holds once counted.
struct Artifact {
    title: &'static str,
    path: PathBuf,
    important: bool,
    /// Bytes and files, once counted.
    counted: Option<(u64, u64)>,
    counting: Option<Receiver<(u64, u64)>>,
}

impl Artifact {
    fn new(title: &'static str, path: PathBuf, important: bool) -> Artifact {
        let mut artifact = Artifact { title, path, important, counted: None, counting: None };
        artifact.count();
        artifact
    }

    /// `UpdateLabels`: the folder walked on a worker.
    fn count(&mut self) {
        self.counted = None;
        let path = self.path.clone();
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let spawned = std::thread::Builder::new().name("artifact size".to_string()).spawn(move || {
            let _ = sender.send(folder_size(&path));
        });
        if spawned.is_ok() {
            self.counting = Some(receiver);
        }
    }

    fn poll(&mut self) {
        let Some(counting) = &self.counting else {
            return;
        };
        if let Ok(counted) = counting.try_recv() {
            self.counted = Some(counted);
            self.counting = None;
        }
    }
}

/// Every file under `folder`: their bytes and their number.
fn folder_size(folder: &Path) -> (u64, u64) {
    let mut bytes = 0;
    let mut files = 0;
    let Ok(entries) = std::fs::read_dir(folder) else {
        return (0, 0);
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            let (inner_bytes, inner_files) = folder_size(&entry.path());
            bytes += inner_bytes;
            files += inner_files;
        } else if let Ok(metadata) = entry.metadata() {
            bytes += metadata.len();
            files += 1;
        }
    }
    (bytes, files)
}

/// What Delete… asks (`TD.Show("Delete Files", "Cleanup Options", …)`).
struct Cleanup {
    index: usize,
}

/// The window while it is open.
pub struct ArtifactExplorer {
    artifacts: Vec<Artifact>,
    cleanup: Option<Cleanup>,
}

impl ArtifactExplorer {
    /// `AddArtifacts`: the folders, in order.
    pub fn open(editor: &Editor) -> ArtifactExplorer {
        let data = crate::user::data_folder().unwrap_or_default();
        let preferences = &editor.user.preferences;
        let documents = preferences.documents_folder();
        let artifacts = vec![
            Artifact::new("Autosaves", data.join("Autosaves"), false),
            Artifact::new("Thumbnails", data.join("Thumbnails"), false),
            Artifact::new("Logs", data.join("Logs"), false),
            Artifact::new("Projects", preferences.projects_folder(), false),
            Artifact::new("Builds", preferences.builds_folder(), true),
            Artifact::new("Presets", documents.join("Presets"), false),
            Artifact::new("Screenshots", preferences.screenshots_folder(), false),
            Artifact::new("Updates", data.join("Updates"), false),
        ];
        ArtifactExplorer { artifacts, cleanup: None }
    }

    fn refresh(&mut self) {
        for artifact in &mut self.artifacts {
            artifact.count();
        }
    }
}

/// `ByteSize.ToString()`: the largest whole unit, up to two decimals.
fn byte_size(bytes: u64) -> String {
    crate::ui::data_view::stats::byte_size(bytes as usize)
}

/// `###,##0`.
fn grouped(count: u64) -> String {
    let digits = count.to_string();
    let mut text = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

/// The size's colour by gigabytes: green below 15, yellow below 30,
/// orange below 50, red beyond.
fn size_colour(bytes: u64) -> Color32 {
    let gigabytes = bytes / 1024 / 1024 / 1024;
    if gigabytes < 15 {
        theme::ACCENT_GREEN
    } else if gigabytes < 30 {
        theme::ACCENT_YELLOW
    } else if gigabytes < 50 {
        theme::ACCENT_STRONG_ORANGE
    } else {
        theme::ACCENT_RED
    }
}

/// Every file under the folder last
/// written more than `days` ago (all of them for 0) put in the Trash; the
/// folders stay. Returns how many files went.
fn clean_up(folder: &Path, days: u64) -> usize {
    let mut removed = 0;
    let Ok(entries) = std::fs::read_dir(folder) else {
        return 0;
    };
    let limit = Duration::from_secs(days * 86_400);
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            removed += clean_up(&path, days);
            continue;
        }
        let modified = entry.metadata().and_then(|metadata| metadata.modified());
        let age = modified.ok().and_then(|modified| SystemTime::now().duration_since(modified).ok()).unwrap_or_default();
        if age < limit {
            continue;
        }
        match crate::ui::artifact_explorer::trash(&path) {
            Ok(()) => removed += 1,
            Err(error) => log::error!("File cleanup failed. {}: {error}", path.display()),
        }
    }
    removed
}

/// The Trash: `~/.Trash` on macOS, the freedesktop trash elsewhere on
/// Unix. Where there is none, the file is deleted.
pub fn trash(path: &Path) -> std::io::Result<()> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    if cfg!(target_os = "macos")
        && let Some(home) = &home
    {
        let trash = home.join(".Trash");
        let target = free_name(&trash, &name);
        return std::fs::rename(path, target);
    }
    if cfg!(all(unix, not(target_os = "macos")))
        && let Some(home) = &home
    {
        let trash = home.join(".local/share/Trash");
        let files = trash.join("files");
        let info = trash.join("info");
        std::fs::create_dir_all(&files)?;
        std::fs::create_dir_all(&info)?;
        let target = free_name(&files, &name);
        let stored = target.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let deleted = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S");
        let description = format!("[Trash Info]\nPath={}\nDeletionDate={deleted}\n", path.display());
        std::fs::write(info.join(format!("{stored}.trashinfo")), description)?;
        return std::fs::rename(path, target);
    }
    std::fs::remove_file(path)
}

/// `name` in `folder`, numbered when taken.
fn free_name(folder: &Path, name: &str) -> PathBuf {
    let first = folder.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) => (stem.to_string(), format!(".{extension}")),
        None => (name.to_string(), String::new()),
    };
    let mut number = 2;
    loop {
        let candidate = folder.join(format!("{stem} {number}{extension}"));
        if !candidate.exists() {
            return candidate;
        }
        number += 1;
    }
}

/// The window. Returns false once closed.
pub fn show(ctx: &egui::Context, editor: &mut Editor, explorer: &mut ArtifactExplorer) -> bool {
    for artifact in &mut explorer.artifacts {
        artifact.poll();
    }
    if explorer.artifacts.iter().any(|artifact| artifact.counting.is_some()) {
        ctx.request_repaint_after(Duration::from_millis(200));
    }
    let mut open = true;
    let mut close = false;
    let mut window = egui::Window::new("Artifact Explorer");
    window = window.id(Id::new("artifact-explorer"));
    window = window.collapsible(false);
    window = window.resizable(false);
    window = window.default_width(640.0);
    // `CenterScreen`, kept centred as the counts come in.
    window = window.anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO);
    window = window.open(&mut open);
    window.show(ctx, |ui| {
        ui.set_width(608.0);
        ui.label(RichText::new("Artifacts Explorer").font(theme::semibold(12.0)).color(theme::LABEL_EMPHASIS));
        let text = "Explore and evaluate various artifacts, files, and residual information generated by Nerthus. You can cleanup individual artifact clusters - either all of them, or just those older than 45 days.";
        ui.add(egui::Label::new(RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT)).wrap());
        ui.add_space(24.0);
        // The header's grid (margin 4): Location 200 wide, the two counts
        // sharing the rest less the buttons' 90, right-aligned 16 and 24
        // short of their columns' ends.
        let widths = column_widths(ui.available_width() - 8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(4.0);
            let headers = [("Location", 0.0), ("Total Size", 16.0), ("Number of Files", 24.0)];
            for (column, (header, margin)) in headers.into_iter().enumerate() {
                let layout = if column == 0 { Layout::left_to_right(Align::Center) } else { Layout::right_to_left(Align::Center) };
                ui.allocate_ui_with_layout(Vec2::new(widths[column], 18.0), layout, |ui| {
                    ui.set_min_size(Vec2::new(widths[column], 18.0));
                    ui.add_space(margin);
                    ui.label(RichText::new(header).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT));
                });
            }
        });
        let mut frame = egui::Frame::new();
        frame = frame.fill(theme::PANEL_BORDER);
        frame = frame.stroke(Stroke::new(1.0, theme::BUTTON_BORDER));
        frame = frame.corner_radius(4.0);
        frame = frame.inner_margin(egui::Margin::symmetric(12, 10));
        frame.show(ui, |ui| {
            let widths = column_widths(ui.available_width());
            for (index, artifact) in explorer.artifacts.iter().enumerate() {
                artifact_row(ui, artifact, widths, index, &mut explorer.cleanup);
            }
        });
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if ui.button("Refresh").clicked() {
                explorer.refresh();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        });
    });
    cleanup_dialog(ctx, editor, explorer);
    open && !close
}

/// The grid's columns (`200`, `*`, `*`, `90`) across this width.
fn column_widths(total: f32) -> [f32; 4] {
    let star = ((total - 200.0 - 90.0) / 2.0).max(60.0);
    [200.0, star, star, 90.0]
}

/// One folder's row.
fn artifact_row(ui: &mut egui::Ui, artifact: &Artifact, widths: [f32; 4], index: usize, cleanup: &mut Option<Cleanup>) {
    let exists = artifact.path.is_dir();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.allocate_ui_with_layout(Vec2::new(widths[0], 22.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_min_size(Vec2::new(widths[0], 22.0));
            ui.spacing_mut().item_spacing.x = 6.0;
            let icon_rect = ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::hover()).0;
            crate::icons::paint(ui, icon_rect, "folder", theme::LABEL_DEFAULT);
            let title = if exists { artifact.title.to_string() } else { format!("{} (NOT FOUND!)", artifact.title) };
            let link = ui.link(RichText::new(title).font(theme::interface(12.0)));
            if link.clicked() && exists {
                crate::export_build::open_folder(&artifact.path);
            }
            link.on_hover_text(artifact.path.display().to_string());
        });
        let (size_text, size_colour, files_text) = match (exists, artifact.counted) {
            (false, _) => (String::new(), theme::LABEL_DEFAULT, String::new()),
            (true, None) => ("Calculating...".to_string(), theme::LABEL_DEFAULT, "Calculating...".to_string()),
            (true, Some((bytes, files))) => (byte_size(bytes), size_colour(bytes), format!("{} files", grouped(files))),
        };
        ui.allocate_ui_with_layout(Vec2::new(widths[1], 22.0), Layout::right_to_left(Align::Center), |ui| {
            ui.set_min_size(Vec2::new(widths[1], 22.0));
            ui.add_space(16.0);
            ui.label(RichText::new(size_text).font(theme::monospace(11.0)).color(size_colour));
        });
        ui.allocate_ui_with_layout(Vec2::new(widths[2], 22.0), Layout::right_to_left(Align::Center), |ui| {
            ui.set_min_size(Vec2::new(widths[2], 22.0));
            ui.add_space(24.0);
            ui.label(RichText::new(files_text).font(theme::monospace(11.0)).color(theme::ACCENT_GRAY));
        });
        ui.allocate_ui_with_layout(Vec2::new(widths[3], 22.0), Layout::right_to_left(Align::Center), |ui| {
            ui.set_min_size(Vec2::new(widths[3], 22.0));
            let label = RichText::new("Delete...").color(theme::ACCENT_RED);
            let enabled = exists && !artifact.important;
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                *cleanup = Some(Cleanup { index });
            }
        });
    });
}

/// "Which files would you like to clean up?": Delete All, Old Files Only
/// (45 days), or Cancel.
fn cleanup_dialog(ctx: &egui::Context, editor: &mut Editor, explorer: &mut ArtifactExplorer) {
    let Some(cleanup) = explorer.cleanup.as_ref() else {
        return;
    };
    let index = cleanup.index;
    let path = explorer.artifacts[index].path.clone();
    let mut chosen: Option<Option<u64>> = None;
    let modal = egui::Modal::new(Id::new("artifact-cleanup"));
    let response = modal.show(ctx, |ui| {
        ui.set_width(420.0);
        ui.label(RichText::new("Delete Files").font(theme::interface(11.0)).color(theme::LABEL_DEFAULT));
        ui.add_space(4.0);
        ui.label(RichText::new("Cleanup Options").font(theme::semibold(14.0)).color(theme::ACCENT_MUTED_BLUE));
        ui.add_space(6.0);
        let text = format!("Which files would you like to clean up?\n\nLocation: {}", path.display());
        ui.add(egui::Label::new(RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT)).wrap());
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if ui.button("Cancel").clicked() {
                chosen = Some(None);
            }
            if ui.button("Old Files Only (45 days)").clicked() {
                chosen = Some(Some(45));
            }
            if ui.button("Delete All").clicked() {
                chosen = Some(Some(0));
            }
        });
        ui.add_space(8.0);
        ui.separator();
        let footer = if cfg!(windows) { "Files are deleted." } else { "Safe Action: Files are not deleted, but sent to the Trash." };
        ui.label(RichText::new(footer).font(theme::interface(11.0)).color(theme::LABEL_DEFAULT));
    });
    if response.should_close() && chosen.is_none() {
        chosen = Some(None);
    }
    let Some(choice) = chosen else {
        return;
    };
    explorer.cleanup = None;
    if let Some(days) = choice {
        ctx.set_cursor_icon(egui::CursorIcon::Wait);
        let removed = clean_up(&path, days);
        editor.report(format!("{removed} files cleaned up from {}", path.display()));
        explorer.artifacts[index].count();
    }
}
