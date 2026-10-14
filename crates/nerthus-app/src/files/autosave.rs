//! Autosaves (`Shell.TryAutosave`, `Session.Autosave`): a safety copy of
//! the project in the autosaves folder each time the interval chosen in the
//! title bar's Autosave menu passes, listed in that menu to revert to; a
//! recovery copy (`Recovery.terrain`) whenever the project changed, kept
//! aside at the next start; a copy written when Nerthus crashes; and a
//! backup of files from older format versions before they are migrated.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use nerthus_engine::macros::{compare_versions, parse_version, FORMAT_VERSION};
use web_time::Instant;

use crate::document::Document;
use crate::editor::Editor;

/// The Autosave menu's intervals (`mnuSaveTime*`, minutes in their `Tag`):
/// minutes, header and icon. Never is 999 minutes.
pub const AUTOSAVE_CHOICES: [(i64, &str, Option<&str>); 5] = [
    (5, "5 minutes", Some("time-duration-5")),
    (10, "10 minutes", Some("time-duration-10")),
    (15, "15 minutes", Some("time-duration-15")),
    (30, "30 minutes", Some("time-duration-30")),
    (999, "Never", None),
];

/// `passiveTimer`'s period.
const PASSIVE_SECONDS: f64 = 5.0;
/// "Revert to" lists the latest five autosaves.
const MENU_ENTRIES: usize = 5;
/// `PassiveCleanup`: files older than this many days go.
const CLEANUP_DAYS: u64 = 45;

/// The session's autosave state.
pub struct Autosave {
    /// Seconds between safety autosaves (`Session.AutosaveInterval`).
    pub interval: i64,
    /// When the interval last started again (`Session.LastAutosave`): a
    /// save, an autosave, Reset Timer, a new or opened file.
    last_reset: Instant,
    /// This session's autosaves, oldest first: "Autosave N @ HH:mm:ss" and
    /// the file (`Session.Autosaves`).
    entries: Vec<(String, PathBuf)>,
    /// The number the next autosave's entry takes (`AutosaveIndex`).
    next_index: usize,
    /// The interval passed with unsaved changes: the file name shows red
    /// (`Accent-Muted-Red`) until saved.
    pub reminder: bool,
    /// When the passive timer last ran (seconds of real time).
    last_tick: Option<f64>,
    /// The project as the last recovery copy holds it.
    recovery_hash: Option<u64>,
}

impl Default for Autosave {
    fn default() -> Self {
        Self {
            interval: 600,
            last_reset: Instant::now(),
            entries: Vec::new(),
            next_index: 1,
            reminder: false,
            last_tick: None,
            recovery_hash: None,
        }
    }
}

impl Autosave {
    /// `LoadAutosaveInterval`: the saved interval when it is one the menu
    /// offers, else 10 minutes. (Never is one of them.)
    pub fn load_interval(&mut self, saved: i64) {
        let offered = AUTOSAVE_CHOICES.iter().any(|(minutes, _, _)| minutes * 60 == saved);
        self.interval = if offered { saved } else { 600 };
    }

    /// `SetAutosaveInterval`: the interval, and the timer started again.
    pub fn set_interval_minutes(&mut self, minutes: i64) {
        self.interval = minutes * 60;
        self.reset_timer();
    }

    /// Whether the menu item for `minutes` is checked.
    pub fn is_interval(&self, minutes: i64) -> bool {
        self.interval == minutes * 60
    }

    /// `MnuSaveReset_OnClick` and the other places that set `LastAutosave`
    /// to now.
    pub fn reset_timer(&mut self) {
        self.last_reset = Instant::now();
    }

    /// `ClearAutoSaves`: a new or opened file starts a fresh list (the
    /// numbering goes on).
    pub fn clear_entries(&mut self) {
        self.entries.clear();
    }

    /// The latest five autosaves, newest first (`RecreateAutosaveMenu`).
    pub fn menu_entries(&self) -> Vec<(String, PathBuf)> {
        self.entries.iter().rev().take(MENU_ENTRIES).cloned().collect()
    }

    fn add_entry(&mut self, path: PathBuf) {
        let time = chrono::Local::now().format("%H:%M:%S");
        let label = format!("Autosave {} @ {time}", self.next_index);
        self.entries.push((label, path));
        self.next_index += 1;
    }
}

/// `Shell.PassiveTimer_Pulsed`'s file work, every five seconds:
/// `TryAutosave`, then the recovery copy when the project changed.
pub fn passive_tick(editor: &mut Editor, now: f64) {
    let due = editor.files.autosave.last_tick.is_none_or(|last| now - last >= PASSIVE_SECONDS);
    if !due {
        return;
    }
    editor.files.autosave.last_tick = Some(now);
    try_autosave(editor);
    write_recovery(editor);
}

/// `TryAutosave`: once the interval has passed since the timer last
/// started, a safety autosave; with unsaved changes, the file name turns
/// red.
pub fn try_autosave(editor: &mut Editor) {
    let elapsed = editor.files.autosave.last_reset.elapsed();
    let interval = Duration::from_secs(editor.files.autosave.interval.max(0) as u64);
    if elapsed <= interval {
        return;
    }
    autosave(editor, "Safety Autosave", None);
    if editor.document.dirty {
        editor.files.autosave.reminder = true;
    }
}

/// `Session.Autosave(reason, filename)`: the project as it stands in the
/// autosaves folder (`<project>_<yyyy-MM-dd_HH-mm-ss>.terrain` unless
/// named), with a `.reason` file beside it; noted in the menu and the
/// timer started again. A failure is logged and tried again next time.
pub fn autosave(editor: &mut Editor, reason: &str, file: Option<PathBuf>) {
    let Some(folder) = super::autosaves_folder() else {
        return;
    };
    let path = file.unwrap_or_else(|| folder.join(format!("{}_{}.terrain", project_stem(&editor.document), timestamp())));
    let text = nerthus_engine::to_terrain_json(&editor.document.project);
    let written = std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(&path, text));
    if let Err(error) = written {
        log::error!("Autosave failed. {error}");
        return;
    }
    if !reason.is_empty() {
        let reason_path = PathBuf::from(format!("{}.reason", path.display()));
        if let Err(error) = std::fs::write(&reason_path, reason) {
            log::error!("Autosave failed. {error}");
        }
    }
    log::info!("Autosaving {}", path.display());
    editor.files.autosave.reset_timer();
    editor.files.autosave.add_entry(path);
}

/// The file name the project goes by, less `.terrain` (`Untitled` until
/// saved).
fn project_stem(document: &Document) -> String {
    let name = document.file_name();
    name.strip_suffix(".terrain").unwrap_or(&name).to_string()
}

/// The file stamp (`yyyy-MM-dd_HH-mm-ss`, local time).
fn timestamp() -> String {
    chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string()
}

/// The project as the crash handler writes it: its name and its JSON as of
/// the last passive tick that saw it change.
struct Snapshot {
    stem: String,
    text: String,
}

static SNAPSHOT: Mutex<Option<Snapshot>> = Mutex::new(None);

/// `Recovery.terrain` (`Undo.FullRecord` of the latest edit): written on
/// a thread whenever the project has an undo history and changed since the
/// last copy: the project as it is.
fn write_recovery(editor: &mut Editor) {
    if !editor.document.can_undo() {
        return;
    }
    let text = nerthus_engine::to_terrain_json(&editor.document.project);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    let hash = hasher.finish();
    if editor.files.autosave.recovery_hash == Some(hash) {
        return;
    }
    editor.files.autosave.recovery_hash = Some(hash);
    let stem = project_stem(&editor.document);
    if let Ok(mut snapshot) = SNAPSHOT.lock() {
        *snapshot = Some(Snapshot { stem, text: text.clone() });
    }
    let Some(folder) = super::autosaves_folder() else {
        return;
    };
    std::thread::spawn(move || {
        let written = std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(folder.join("Recovery.terrain"), text));
        if written.is_err() {
            log::error!("Unable to save recovery autosave.");
        }
    });
}

/// `RecoverOldFile`: at start, a recovery copy left by the last session is
/// kept as `Recovery<its time>.terrain` (unless that exists already).
pub fn recover_old_file() {
    let Some(folder) = super::autosaves_folder() else {
        return;
    };
    let recovery = folder.join("Recovery.terrain");
    let Ok(modified) = recovery.metadata().and_then(|metadata| metadata.modified()) else {
        return;
    };
    let stamp = chrono::DateTime::<chrono::Local>::from(modified).format("%Y-%m-%d_%H-%M-%S");
    let kept = folder.join(format!("Recovery{stamp}.terrain"));
    if !kept.exists()
        && let Err(error) = std::fs::copy(&recovery, &kept)
    {
        log::error!("Cannot keep the recovery copy: {error}");
    }
}

/// `PassiveCleanup`: logs, autosaves and thumbnails older than 45 days.
pub fn passive_cleanup() {
    for name in ["Logs", "Autosaves", "Thumbnails"] {
        if let Some(folder) = super::data_subfolder(name) {
            delete_old_files(&folder, CLEANUP_DAYS);
        }
    }
}

/// Every file under `folder` last written more than
/// `days` ago.
fn delete_old_files(folder: &Path, days: u64) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let limit = Duration::from_secs(days * 86_400);
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            delete_old_files(&path, days);
            continue;
        }
        let modified = entry.metadata().and_then(|metadata| metadata.modified());
        let age = modified.ok().and_then(|modified| SystemTime::now().duration_since(modified).ok());
        if age.is_some_and(|age| age > limit) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// `Shell.SaveMigrationCopy`: a file in an older format version (its
/// `Metadata.Version`) is copied to the autosaves folder as
/// `Migration_<name>_<time>.terrain` before it is migrated.
pub fn save_migration_copy(document: &Document, path: &Path) {
    let metadata = document.project.extra.get("Metadata");
    let version = metadata.and_then(|metadata| metadata.get("Version")).and_then(serde_json::Value::as_str);
    let Some(version) = version else {
        return;
    };
    let (Some(made), Some(current)) = (parse_version(version), parse_version(FORMAT_VERSION)) else {
        return;
    };
    if compare_versions(&made, &current) != std::cmp::Ordering::Less {
        return;
    }
    let Some(folder) = super::autosaves_folder() else {
        return;
    };
    log::info!("File was last saved with {version}. Migrating to updated format, and saving migration backup to Autosave folder.");
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_default();
    let copy = folder.join(format!("Migration_{stem}_{}.terrain", timestamp()));
    let copied = std::fs::create_dir_all(&folder).and_then(|()| std::fs::copy(path, &copy));
    if let Err(error) = copied {
        log::error!("Failed saving migratory copy. {error}");
    }
}

/// `App.OnDispatcherUnhandledException`'s "Disaster Recovery" autosave:
/// when Nerthus panics, the project (as of its last change) is written to
/// the autosaves folder as `DISASTER_RECOVERY_<name>_<time>.terrain`
/// before the program goes down.
pub fn install_disaster_recovery() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write_disaster_recovery();
        previous(info);
    }));
}

fn write_disaster_recovery() {
    let Ok(snapshot) = SNAPSHOT.try_lock() else {
        return;
    };
    let Some(snapshot) = snapshot.as_ref() else {
        return;
    };
    let Some(folder) = super::autosaves_folder() else {
        return;
    };
    let path = folder.join(format!("DISASTER_RECOVERY_{}_{}.terrain", snapshot.stem, timestamp()));
    let written = std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(&path, &snapshot.text));
    if written.is_ok() {
        let _ = std::fs::write(format!("{}.reason", path.display()), "Disaster Recovery");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals_load_as_the_menu_offers_them() {
        let mut autosave = Autosave::default();
        // A fresh `Options` holds 5, which is not an interval the menu has.
        autosave.load_interval(5);
        assert_eq!(autosave.interval, 600);
        autosave.load_interval(900);
        assert!(autosave.is_interval(15));
        autosave.load_interval(999 * 60);
        assert!(autosave.is_interval(999));
    }

    #[test]
    fn menu_lists_the_latest_five_newest_first() {
        let mut autosave = Autosave::default();
        for index in 0..7 {
            autosave.add_entry(PathBuf::from(format!("/autosaves/{index}.terrain")));
        }
        let entries = autosave.menu_entries();
        assert_eq!(entries.len(), 5);
        assert!(entries[0].0.starts_with("Autosave 7 @ "));
        assert_eq!(entries[0].1, PathBuf::from("/autosaves/6.terrain"));
        autosave.clear_entries();
        autosave.add_entry(PathBuf::from("/autosaves/next.terrain"));
        assert!(autosave.menu_entries()[0].0.starts_with("Autosave 8 @ "));
    }

    #[test]
    fn old_files_are_cleared() {
        let folder = std::env::temp_dir().join(format!("nerthus-cleanup-{}", std::process::id()));
        std::fs::create_dir_all(folder.join("inner")).unwrap();
        let fresh = folder.join("inner").join("fresh.txt");
        std::fs::write(&fresh, "x").unwrap();
        delete_old_files(&folder, 0);
        // Nothing is older than zero days by more than a moment, except
        // what was just written: it goes too once its age passes zero.
        std::thread::sleep(Duration::from_millis(20));
        delete_old_files(&folder, 0);
        assert!(!fresh.exists());
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
