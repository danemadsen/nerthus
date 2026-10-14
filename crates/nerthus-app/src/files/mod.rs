//! What the File menu and the title bar's file box do beyond a plain
//! open and save (`Shell`): opening with the usual checks, Save
//! Incremental, safety autosaves and recovery copies, the macro collection,
//! project thumbnails, the examples, and projects dropped on the window or
//! a close asked for by the system. The menus, prompts and windows that use
//! this live in `ui::file_commands` and `ui::file_windows`.

mod autosave;
mod collection;
mod thumbnail;

use std::path::{Path, PathBuf};

use bevy::ecs::system::NonSendMarker;
use bevy::prelude::*;
use bevy::window::{FileDragAndDrop, PrimaryWindow, WindowCloseRequested};

use crate::document::Document;
use crate::editor::Editor;
use crate::user::locations;

pub use autosave::{install_disaster_recovery, try_autosave, Autosave, AUTOSAVE_CHOICES};
pub use collection::{add_macro, all_recent, consume_documents, export_to, install, installed_macros, ExportForm, MacroCollection};
pub use thumbnail::thumbnail_path;

/// A folder in the user's Nerthus folder (`Paths.Autosaves`,
/// `Paths.Macros`, …).
pub fn data_subfolder(name: &str) -> Option<PathBuf> {
    crate::user::data_folder().map(|folder| folder.join(name))
}

/// `Paths.Autosaves`.
pub fn autosaves_folder() -> Option<PathBuf> {
    data_subfolder("Autosaves")
}

/// `Paths.Macros`: the macro collection.
pub fn macros_folder() -> Option<PathBuf> {
    data_subfolder("Macros")
}

/// Where a project's macros are looked for: the collection, then (as
/// Nerthus did before it had one) a `Macros` folder beside the project.
pub fn macro_folders(project: Option<&Path>) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = macros_folder().into_iter().collect();
    if let Some(folder) = project.and_then(Path::parent) {
        folders.push(folder.join("Macros"));
    }
    folders
}

/// Something File, the title bar or the system asked for. Most go through
/// `ui::file_commands::run`, which asks to save changes where needed.
#[derive(Clone, Debug, PartialEq)]
pub enum FileCommand {
    /// File › New (Ctrl N).
    New,
    /// File › Open… (Ctrl O).
    Open,
    /// A file from Open Recent, the latest one (Ctrl Shift O), or a
    /// project dropped on the window.
    OpenRecent(PathBuf),
    OpenLatest,
    Dropped(PathBuf),
    /// File › Save incremental and the file box's button.
    SaveIncremental,
    /// The Autosave menu: an interval in minutes (`Tag`), Reset Timer, an
    /// autosave of this session, Open Autosave… and Revert to Saved.
    SetAutosaveInterval(i64),
    ResetAutosaveTimer,
    OpenAutosaveEntry(PathBuf),
    OpenAutosave,
    RevertToSaved,
    RevertConfirmed,
    /// File › Repair File…, Open Examples…, Start Dialog.
    RepairFile,
    OpenExamples,
    StartDialog,
    /// File › Import and Export.
    ImportMacro,
    ImportGraph,
    ImportTorlet,
    ImportImage,
    ExportMacro,
    ExportTorlet,
    /// File › Exit, the window's close button.
    Exit,
    /// The save question's answers (`SavePromptResult`), and Yes to "Nodes
    /// are building".
    SaveFirst,
    DiscardChanges,
    ContinueWhileBuilding,
    /// The macro install question's Yes.
    InstallMacro(PathBuf),
    /// A macro from the toolbox's Macro menu, by id.
    AddMacro(String),
    /// The Start dialog and the examples: a new file with no questions
    /// (`SetupNewFile`), the open dialog (`OpenFileDialog`), a file opened
    /// as it is (`Shell.OpenFile`), an example opened as an untitled copy.
    StartNew,
    StartOpen,
    OpenDirect { path: PathBuf, mode: OpenMode },
    OpenExample { path: PathBuf, mode: OpenMode },
    /// Export Macro's questions are answered: save it.
    ExportMacroChecked,
    /// The orphan question's answers (`RecoverOrphans`): the orphans get
    /// tabs of their own, or are deleted.
    RecoverOrphans,
    DeleteOrphans,
    /// Yes to "Obsolete Bake Data Found": the folder goes.
    DeleteObsoleteBakes(PathBuf),
}

/// How a file from the Start dialog or the examples opens (their buttons'
/// menu: `LoadingMechanism`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenMode {
    Normal,
    /// "Force low resolution": the preview at 512.
    LowResolution,
    /// "Open and suspend engine": nothing builds until the engine resumes.
    SuspendEngine,
}

/// An action that waits on the usual questions: whether to save changes
/// (`SavePrompt`, or `Shell_Closing`'s for Exit) and, where it applies,
/// whether to close while nodes build (`PreventCloseWhileBuilding`).
#[derive(Clone, Debug, PartialEq)]
pub enum FileAction {
    New,
    Open,
    OpenPath(PathBuf),
    Repair,
    Examples,
    Exit,
}

/// A `FileAction` and the questions already answered.
#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    pub action: FileAction,
    pub save_answered: bool,
    pub building_answered: bool,
}

/// A file dialog `ui::file_commands` asked for (dialogs block, so a
/// main-thread system shows them).
#[derive(Clone, Debug, PartialEq)]
pub enum FileDialog {
    /// `OpenFileDialog`, from a folder (the autosaves for Open Autosave…).
    Open { folder: Option<PathBuf> },
    /// Save As for a pending action: it goes on once saved.
    SaveAsThenContinue,
    Repair,
    InstallMacro,
    ImportGraph,
    ImportTorlet,
    /// File › Import › Image: a File node for it on graph tab `tab` at
    /// `position`.
    ImportImage { tab: usize, position: [f32; 2] },
    /// File › Export › TORLET: graph tab `tab`.
    ExportTorlet { tab: usize },
    /// Export Macro's Save as file.
    ExportMacro { id: String },
}

/// A message for the interface to show (`MessageBox.Show`, `MBox`): with
/// OK, or with Yes and No when it asks something.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub caption: String,
    pub text: String,
    /// The task dialog's footer (`MBox`'s footnote).
    pub footnote: Option<String>,
    /// What Yes does; none for a plain message.
    pub yes: Option<FileCommand>,
    /// What No does, when it does more than close the box.
    pub no: Option<FileCommand>,
    /// What closing the box without an answer does, when anything.
    pub dismissed: Option<FileCommand>,
}

impl Message {
    pub fn new(caption: &str, text: &str) -> Self {
        Self { caption: caption.to_string(), text: text.to_string(), footnote: None, yes: None, no: None, dismissed: None }
    }
}

/// The files side of the editor (`Editor::files`).
#[derive(Default)]
pub struct FileState {
    pub pending: Option<Pending>,
    pub autosave: Autosave,
    pub macros: MacroCollection,
    pub dialog: Option<FileDialog>,
    /// Commands for `ui::file_commands` from outside the interface (a
    /// drop, the close button, a finished Save As).
    pub requests: Vec<FileCommand>,
    /// Messages to show.
    pub messages: Vec<Message>,
    /// Windows the interface shows: the Start dialog and the examples.
    pub start_dialog_open: bool,
    pub examples_open: bool,
    /// Export Macro's form, once a file has been chosen to save it to.
    pub export_macro: Option<ExportForm>,
    /// The project being saved needs a thumbnail.
    pub thumbnail_request: Option<PathBuf>,
    /// Exit was answered: the app closes this frame.
    pub exit_now: bool,
    /// A graph tab to show once an import added it, and whether to frame
    /// its nodes (`ImportTorlet` zooms to extents).
    pub show_tab: Option<(usize, bool)>,
    /// An alert for the interface to show (`ShowAlert`): a save that did
    /// not go through, a delete that failed.
    pub alert: Option<crate::ui::alert::Alert>,
}

impl FileState {
    pub fn show_message(&mut self, caption: &str, text: &str) {
        self.messages.push(Message::new(caption, text));
    }
}

/// `Shell.OpenFile(fileName)`: the session's autosaves forgotten, then the
/// file opened (`Session.Open`): a file whose macros are
/// not installed does not open, nor does one that cannot be read. Returns
/// whether it opened.
pub fn open_project(editor: &mut Editor, path: &Path) -> bool {
    open_with(editor, path, true)
}

/// An example opened as an untitled copy named after it, left out of the
/// recent files (`Session.IsExample`).
pub fn open_example(editor: &mut Editor, path: &Path) -> bool {
    if !open_with(editor, path, false) {
        return false;
    }
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().to_string());
    editor.document.path = None;
    editor.document.untitled_name = stem;
    true
}

fn open_with(editor: &mut Editor, path: &Path, remember: bool) -> bool {
    editor.files.start_dialog_open = false;
    editor.files.autosave.clear_entries();
    let mut document = match Document::open(path) {
        Ok(document) => document,
        Err(error) => {
            open_failed(editor, path, &format!("{error:#}"));
            return false;
        }
    };
    if let Some(missing) = missing_macro(&document, path) {
        let reason = format!("This file depends on the macro [{missing}], which is not installed. Please install this macro and try again.");
        open_failed(editor, path, &reason);
        return false;
    }
    let unknown = document.project.unknown_types.clone();
    autosave::save_migration_copy(&document, path);
    document.recover_save_definitions();
    editor.set_document(document);
    ask_about_orphans(editor);
    // An example opens as an untitled copy: nothing is asked about its
    // bake data, and it stays out of the recent files.
    if remember {
        ask_about_obsolete_bakes(editor, path);
        editor.user.add_recent(path);
        collection::add_to_all_recent(path);
    }
    editor.files.autosave.reset_timer();
    editor.report(format!("Opened {}", path.display()));
    if !unknown.is_empty() {
        editor.report(format!("Unknown node types: {}", unknown.join(", ")));
    }
    true
}

/// `Shell.RecoverOrphans`: nodes on graph tabs the project does not have
/// are offered tabs of their own; No deletes them. Closing the question
/// without an answer counts as Yes, as only No deletes.
fn ask_about_orphans(editor: &mut Editor) {
    if editor.document.orphan_nodes().is_empty() {
        return;
    }
    let mut question = Message::new(crate::document::ORPHANS_CAPTION, crate::document::ORPHANS_TEXT);
    question.yes = Some(FileCommand::RecoverOrphans);
    question.no = Some(FileCommand::DeleteOrphans);
    question.dismissed = Some(FileCommand::RecoverOrphans);
    editor.files.messages.push(question);
}

/// The question about bake data the older project format kept beside the
/// project, which no build reads any more.
pub const OBSOLETE_BAKES_CAPTION: &str = "Obsolete Bake Data Found";
pub const OBSOLETE_BAKES_TEXT: &str = "This project contains Baked Data from an older version. This bake is not compatible with the newer version, and you will need to re-bake any nodes.\n\nWould you like to delete this old data?";

/// Where the older format kept a project's bake data: `BakeData/<project
/// name>` beside the project.
pub fn obsolete_bakes_folder(project: &Path) -> Option<PathBuf> {
    let folder = project.parent()?;
    let stem = project.file_stem()?;
    Some(folder.join("BakeData").join(stem))
}

/// `Shell.OpenFile`: when the project has bake data in the older format,
/// it is offered for deletion, its size in the footer.
fn ask_about_obsolete_bakes(editor: &mut Editor, path: &Path) {
    let Some(folder) = obsolete_bakes_folder(path) else {
        return;
    };
    if !folder.is_dir() {
        return;
    }
    let size = folder_size(&folder);
    let size_text = crate::ui::byte_size(size as usize);
    let mut question = Message::new(OBSOLETE_BAKES_CAPTION, OBSOLETE_BAKES_TEXT);
    question.footnote = Some(format!("Obsolete data size is {size_text}"));
    question.yes = Some(FileCommand::DeleteObsoleteBakes(folder));
    editor.files.messages.push(question);
}

/// Yes to the obsolete bake question: the folder and all it holds go. A
/// failure is logged and shown as an alert that stays until closed.
pub fn delete_obsolete_bakes(editor: &mut Editor, folder: &Path) {
    let Err(error) = std::fs::remove_dir_all(folder) else {
        return;
    };
    log::error!("{error}");
    let alert = crate::ui::alert::Alert::new("Error!", &error.to_string(), crate::ui::alert::AlertKind::Error);
    editor.files.alert = Some(alert.staying());
}

/// The bytes of every file in a folder and its subfolders; a folder that
/// cannot be read ends the count with what was found so far.
fn folder_size(folder: &Path) -> u64 {
    let mut total = 0;
    let mut pending = vec![folder.to_path_buf()];
    while let Some(current) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            return total;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                return total;
            };
            let Ok(metadata) = entry.metadata() else {
                return total;
            };
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                total += metadata.len();
            }
        }
    }
    total
}

/// `FileInfo.IsReadOnly`: the read-only attribute on Windows. Elsewhere a
/// file is read-only when this user may read it but not write it, judged
/// by its owner's, group's or others' permissions as they apply.
pub fn is_read_only(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let mode = metadata.mode();
        let read_only = |read: u32, write: u32| mode & read != 0 && mode & write == 0;
        let user = read_only(0o400, 0o200);
        let group = read_only(0o040, 0o020);
        let other = read_only(0o004, 0o002);
        if user == group && group == other {
            return user;
        }
        // SAFETY: `geteuid` only reads the process's user id.
        let effective_user = unsafe { libc::geteuid() };
        if metadata.uid() == effective_user {
            return user;
        }
        if group == other {
            return group;
        }
        if is_member_of_group(metadata.gid()) {
            return group;
        }
        other
    }
    #[cfg(not(unix))]
    {
        metadata.permissions().readonly()
    }
}

/// Whether this process's user belongs to `group`: its effective group or
/// one of its supplementary groups.
#[cfg(unix)]
fn is_member_of_group(group: u32) -> bool {
    // SAFETY: `getegid` only reads the process's group id.
    let effective_group = unsafe { libc::getegid() };
    if effective_group == group {
        return true;
    }
    // SAFETY: a count of 0 only asks how many groups there are.
    let count = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
    if count <= 0 {
        return false;
    }
    let mut groups = vec![0 as libc::gid_t; count as usize];
    // SAFETY: the buffer holds `count` group ids.
    let filled = unsafe { libc::getgroups(count, groups.as_mut_ptr()) };
    if filled <= 0 {
        return false;
    }
    groups.truncate(filled as usize);
    groups.contains(&group)
}

/// `Session_FileOpenFailed`: "Failed to open file: …" with the reason.
fn open_failed(editor: &mut Editor, path: &Path, reason: &str) {
    let name = path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
    editor.report(format!("Failed to open {}: {reason}", path.display()));
    let caption = format!("Failed to open file: {name}");
    let text = format!("Failed to open file.\n\n{reason}\n\n{}", path.display());
    editor.files.show_message(&caption, &text);
}

/// The first macro the project uses that is neither in the collection nor
/// beside the project (`Session.Open`'s check of `Project.Macros`).
fn missing_macro(document: &Document, path: &Path) -> Option<String> {
    let folders = macro_folders(Some(path));
    let ids = document.project.macro_ids();
    ids.into_iter().find(|id| nerthus_engine::macros::find_macro_file(id, &folders).is_none())
}

/// `Shell.SetupNewFile`: a new, empty project; the session's autosaves
/// forgotten and the autosave timer started again.
pub fn new_project(editor: &mut Editor) {
    editor.set_document(Document::new());
    editor.files.autosave.clear_entries();
    editor.files.autosave.reset_timer();
}

/// After a save went through (`Shell.SaveFile`'s tail): the autosave
/// timer and the file name's reminder start again, the project's
/// thumbnail is taken, and the file joins the searchable recent files.
pub fn saved(editor: &mut Editor, path: &Path) {
    editor.files.autosave.reset_timer();
    editor.files.autosave.reminder = false;
    collection::add_to_all_recent(path);
    editor.files.thumbnail_request = Some(path.to_path_buf());
}

/// `Shell.Increment`: the next free name in
/// the project's folder — `Name001`, or the number after one it ends in
/// (`Name004` after `Name003`).
pub fn incremented_path(path: &Path) -> PathBuf {
    let folder = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let extension = path.extension().map(|extension| format!(".{}", extension.to_string_lossy())).unwrap_or_default();
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_default();
    if !path.exists() {
        return path.to_path_buf();
    }
    let characters: Vec<char> = stem.chars().collect();
    let tail: String = if characters.len() <= 3 { String::new() } else { characters[characters.len() - 3..].iter().collect() };
    let (base, mut number) = match dotnet_int_parse(&tail) {
        Some(number) => (characters[..characters.len() - 3].iter().collect::<String>(), number),
        None => (stem.clone(), 0),
    };
    if number == 0 {
        number = 1;
    }
    loop {
        let candidate = folder.join(format!("{base}{}{extension}", dotnet_format_000(number)));
        if !candidate.exists() {
            return candidate;
        }
        number += 1;
    }
}

/// `int.TryParse` (`NumberStyles.Integer`): optional white space around an
/// optional sign and digits.
fn dotnet_int_parse(text: &str) -> Option<i64> {
    let trimmed = text.trim_matches(|character: char| character.is_whitespace());
    let (negative, digits) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    if digits.is_empty() || !digits.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    let value: i64 = digits.parse().ok()?;
    Some(if negative { -value } else { value })
}

/// `ToString("000")`: at least three digits, the sign before them.
fn dotnet_format_000(number: i64) -> String {
    if number < 0 {
        return format!("-{:03}", -number);
    }
    format!("{number:03}")
}

/// Startup work: an old recovery copy kept aside (`RecoverOldFile`),
/// files older than 45 days cleared (`PassiveCleanup`, when Clear Old Files
/// is on), the macro collection read and the crash handler installed.
/// The Start dialog opens when asked (see `LaunchOptions::start_dialog`).
pub fn start(editor: &mut Editor, start_dialog: bool) {
    autosave::recover_old_file();
    if editor.user.options.clear_old_files() {
        autosave::passive_cleanup();
    }
    editor.files.macros.reload();
    editor.files.autosave.load_interval(editor.user.options.autosave_interval());
    install_disaster_recovery();
    editor.files.start_dialog_open = start_dialog;
}

/// The files plugin: the timers, drops, close requests, dialogs and
/// thumbnails.
pub struct FilePlugin;

impl Plugin for FilePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (tick, take_window_events));
        app.add_systems(Update, show_dialogs);
        app.add_systems(Update, (thumbnail::start_capture, thumbnail::finish_capture));
    }
}

/// `Shell.passiveTimer`'s work (every 5 seconds): the safety autosave and
/// the recovery copy. Also closes the app once Exit was answered.
fn tick(mut editor: ResMut<Editor>, time: Res<Time<Real>>, mut exit: MessageWriter<AppExit>) {
    if editor.files.exit_now {
        editor.files.exit_now = false;
        exit.write(AppExit::Success);
        return;
    }
    let now = time.elapsed_secs_f64();
    autosave::passive_tick(&mut editor, now);
}

/// Files dropped on the window (`Shell_Drop`: the first, when it is a
/// project) and the window's close button (`Shell_Closing`).
fn take_window_events(mut editor: ResMut<Editor>, mut drops: MessageReader<FileDragAndDrop>, mut closes: MessageReader<WindowCloseRequested>, primary: Query<(), With<PrimaryWindow>>) {
    let mut dropped = Vec::new();
    for event in drops.read() {
        if let FileDragAndDrop::DroppedFile { path_buf, .. } = event {
            dropped.push(path_buf.clone());
        }
    }
    // A project opens (`Shell_Drop`); a macro is offered for installing,
    // as Export Macro's text promises.
    let first = dropped.into_iter().next();
    if let Some(path) = first {
        let name = path.to_string_lossy().to_ascii_lowercase();
        if name.ends_with(".terrain") {
            editor.files.requests.push(FileCommand::Dropped(path));
        } else if name.ends_with(".macro") {
            collection::ask_to_install(&mut editor, &path);
        }
    }
    for request in closes.read() {
        if primary.contains(request.window) {
            editor.files.requests.push(FileCommand::Exit);
        }
    }
}

/// The open dialog's filter.
const PROJECT_FILTER: (&str, &[&str]) = ("Nerthus Project File (*.terrain)", &["terrain"]);

/// File dialogs block and must run on the main thread (macOS).
#[cfg(not(target_arch = "wasm32"))]
fn show_dialogs(_main_thread: NonSendMarker, mut editor: ResMut<Editor>) {
    let Some(request) = editor.files.dialog.take() else {
        return;
    };
    let editor = editor.as_mut();
    match request {
        FileDialog::Open { folder } => {
            // `OpenFileDialog(location)`: the folder asked for, else where
            // the last file was opened from, else the projects folder.
            let noted = editor.user.locations.folder(locations::OPEN);
            let start = folder.or(noted).unwrap_or_else(|| editor.user.preferences.projects_folder());
            let mut dialog = rfd::FileDialog::new();
            dialog = dialog.add_filter(PROJECT_FILTER.0, PROJECT_FILTER.1);
            if start.is_dir() {
                dialog = dialog.set_directory(&start);
            }
            let Some(path) = dialog.pick_file() else {
                return;
            };
            editor.user.locations.set_from_file(locations::OPEN, &path);
            open_project(editor, &path);
        }
        FileDialog::SaveAsThenContinue => {
            let file_name = editor.document.file_name();
            let mut dialog = rfd::FileDialog::new();
            dialog = dialog.add_filter(PROJECT_FILTER.0, PROJECT_FILTER.1);
            dialog = dialog.set_file_name(file_name);
            if let Some(folder) = editor.user.locations.folder(locations::SAVE) {
                dialog = dialog.set_directory(folder);
            }
            let Some(path) = dialog.save_file() else {
                // Cancelled: nothing more happens, so no changes are lost.
                editor.files.pending = None;
                return;
            };
            let path = with_extension(path, "terrain");
            editor.save_to(&path);
            if editor.document.dirty {
                editor.files.pending = None;
                return;
            }
            editor.files.requests.push(FileCommand::DiscardChanges);
        }
        FileDialog::Repair => {
            let mut dialog = rfd::FileDialog::new();
            dialog = dialog.add_filter(PROJECT_FILTER.0, PROJECT_FILTER.1);
            if let Some(folder) = editor.user.locations.folder(locations::OPEN) {
                dialog = dialog.set_directory(folder);
            }
            let Some(path) = dialog.pick_file() else {
                return;
            };
            repair(editor, &path);
        }
        FileDialog::InstallMacro => {
            let mut dialog = rfd::FileDialog::new();
            dialog = dialog.set_title("Install Macro");
            dialog = dialog.add_filter("Nerthus Macro (.macro)", &["macro"]);
            let Some(path) = dialog.pick_file() else {
                return;
            };
            collection::ask_to_install(editor, &path);
        }
        FileDialog::ImportGraph => {
            let mut dialog = rfd::FileDialog::new();
            dialog = dialog.add_filter("Nerthus Project File (*.terrain)", &["terrain"]);
            dialog = dialog.add_filter("Nerthus Graph Fragment (*.tfrag)", &["tfrag"]);
            if let Some(folder) = editor.user.locations.folder(locations::OPEN) {
                dialog = dialog.set_directory(folder);
            }
            let Some(path) = dialog.pick_file() else {
                return;
            };
            import_graph(editor, &path);
        }
        FileDialog::ImportTorlet => {
            let mut dialog = rfd::FileDialog::new();
            dialog = dialog.add_filter("Nerthus Graph Fragment (*.tfrag)", &["tfrag"]);
            if let Some(folder) = editor.user.locations.folder(locations::OPEN) {
                dialog = dialog.set_directory(folder);
            }
            let Some(path) = dialog.pick_file() else {
                return;
            };
            editor.user.locations.set_from_file(locations::OPEN, &path);
            import_graph(editor, &path);
        }
        FileDialog::ImportImage { tab, position } => {
            let mut dialog = rfd::FileDialog::new();
            dialog = dialog.add_filter("All Image Formats", &IMAGE_EXTENSIONS);
            let noted = editor.user.locations.folder(locations::OPEN);
            let start = noted.unwrap_or_else(|| editor.user.preferences.projects_folder());
            if start.is_dir() {
                dialog = dialog.set_directory(start);
            }
            let Some(path) = dialog.pick_file() else {
                return;
            };
            import_image(editor, &path, tab, position);
        }
        FileDialog::ExportTorlet { tab } => {
            let mut dialog = rfd::FileDialog::new();
            dialog = dialog.add_filter("Nerthus Graph Fragment (*.tfrag)", &["tfrag"]);
            if let Some(folder) = editor.user.locations.folder(locations::SAVE) {
                dialog = dialog.set_directory(folder);
            }
            let Some(path) = dialog.save_file() else {
                return;
            };
            let path = with_extension(path, "tfrag");
            editor.user.locations.set_from_file(locations::SAVE, &path);
            let fragment = editor.document.graph_fragment(tab);
            let text = nerthus_engine::to_fragment_json(&fragment);
            if let Err(error) = std::fs::write(&path, text) {
                editor.report(format!("Cannot write {}: {error}", path.display()));
            }
        }
        FileDialog::ExportMacro { id } => {
            let mut dialog = rfd::FileDialog::new();
            dialog = dialog.set_title("Export Macro");
            dialog = dialog.add_filter("Nerthus Macro (.macro)", &["macro"]);
            dialog = dialog.set_file_name(format!("{id}.macro"));
            let Some(path) = dialog.save_file() else {
                return;
            };
            let path = with_extension(path, "macro");
            let into_collection = macros_folder().is_some_and(|folder| path.starts_with(folder));
            if into_collection {
                let text = "You cannot export a macro directly into the Macros folder. Check \"Install in Nerthus\" to do this.\n\nPlease choose a different location.";
                editor.files.show_message("Invalid Location", text);
                return;
            }
            collection::export_to(editor, Some(path));
        }
    }
}

/// The web build has no file system: Open picks a project through the
/// browser (kept for the session under its name, `crate::web`), and saving
/// downloads; the other dialogs read and write files and folders.
#[cfg(target_arch = "wasm32")]
fn show_dialogs(mut editor: ResMut<Editor>, mut picking: Local<Option<crate::web::Picking>>) {
    if let Some(answer) = picking.as_ref().and_then(crate::web::Picking::poll) {
        *picking = None;
        if let Some(path) = answer {
            open_project(&mut editor, &path);
        }
    }
    let Some(request) = editor.files.dialog.take() else {
        return;
    };
    let editor = editor.as_mut();
    match request {
        FileDialog::Open { .. } => *picking = Some(crate::web::Picking::start(PROJECT_FILTER.0, PROJECT_FILTER.1)),
        FileDialog::SaveAsThenContinue => {
            let path = with_extension(PathBuf::from(editor.document.file_name()), "terrain");
            editor.save_to(&path);
            if editor.document.dirty {
                editor.files.pending = None;
                return;
            }
            editor.files.requests.push(FileCommand::DiscardChanges);
        }
        FileDialog::ExportTorlet { tab } => {
            let fragment = editor.document.graph_fragment(tab);
            let text = nerthus_engine::to_fragment_json(&fragment);
            if let Err(error) = crate::web::write(Path::new("Graph.tfrag"), text) {
                editor.report(format!("Cannot export the graph: {error}"));
            }
        }
        FileDialog::Repair => editor.report(crate::web::unavailable("Repair File")),
        FileDialog::InstallMacro | FileDialog::ExportMacro { .. } => editor.report(crate::web::unavailable("Installing and exporting macros")),
        FileDialog::ImportGraph | FileDialog::ImportTorlet | FileDialog::ImportImage { .. } => editor.report(crate::web::unavailable("Importing files")),
    }
}

/// The image formats File › Import › Image offers (the `FileFormats`
/// readers).
pub const IMAGE_EXTENSIONS: [&str; 21] = ["exr", "graw", "jpg", "png", "r16", "r32", "raw", "tif", "jpeg", "webp", "tiff", "svg", "rgb", "miff", "jp2", "hdr", "eps", "emf", "dds", "dng", "bmp"];

/// File › Import › Image (`ImportImage`): a File node reading the image,
/// in the middle of the graph's view, selected.
fn import_image(editor: &mut Editor, path: &Path, tab: usize, position: [f32; 2]) {
    let Some(id) = editor.document.add_node("File", position) else {
        return;
    };
    let file_name = serde_json::Value::String(path.to_string_lossy().to_string());
    editor.document.apply_build_update(id, "FileName", file_name);
    if tab != 0 {
        editor.document.apply_build_update(id, "GraphIndex", serde_json::Value::from(tab as i64));
    }
    editor.document.apply_node_display(&[id], &editor.user.options);
    editor.select(Some(id));
    editor.changed();
}

/// File › Import › Graph or TORLET (`ImportGraph`, `ImportTorlet`): a
/// project's or fragment's graph tabs added to this project, the last one
/// shown.
fn import_graph(editor: &mut Editor, path: &Path) {
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_default();
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            editor.report(format!("Cannot read {}: {error}", path.display()));
            return;
        }
    };
    let is_fragment = path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("tfrag"));
    let terrain = if is_fragment {
        nerthus_engine::fragment::parse_fragment(&text).map(|(terrain, _)| terrain)
    } else {
        nerthus_engine::Project::parse(&text).map(|project| project.assets[0].terrain.clone())
    };
    let terrain = match terrain {
        Ok(terrain) => terrain,
        Err(error) => {
            editor.report(format!("Cannot import {}: {error}", path.display()));
            return;
        }
    };
    if let Some(tab) = editor.document.import_terrain(&terrain, &stem) {
        editor.files.show_tab = Some((tab, is_fragment));
        editor.changed();
    }
}

/// File › Repair File… (`RepairFile`): a new project holding the chosen
/// file's graph, read as File › Open reads it and imported tab by tab, in
/// place of the new project's empty first tab.
fn repair(editor: &mut Editor, path: &Path) {
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_default();
    let project = std::fs::read_to_string(path).map_err(anyhow::Error::from).and_then(|text| nerthus_engine::Project::parse(&text).map_err(anyhow::Error::from));
    let project = match project {
        Ok(project) => project,
        Err(error) => {
            log::error!("Failed to repair file. {error:#}");
            editor.report("Fail: Repair attempt failed.");
            return;
        }
    };
    editor.document.import_terrain(project.terrain(), &stem);
    editor.document.delete_graph_tab(0);
    editor.changed();
    editor.report("Repaired: Repair attempt was successful.");
}

fn with_extension(path: PathBuf, extension: &str) -> PathBuf {
    if path.extension().is_some() {
        return path;
    }
    path.with_extension(extension)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incremental_names_count_up() {
        let folder = std::env::temp_dir().join(format!("nerthus-increment-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let project = folder.join("Ridge.terrain");
        assert_eq!(incremented_path(&project), project);
        std::fs::write(&project, "").unwrap();
        assert_eq!(incremented_path(&project), folder.join("Ridge001.terrain"));
        std::fs::write(folder.join("Ridge001.terrain"), "").unwrap();
        assert_eq!(incremented_path(&project), folder.join("Ridge002.terrain"));
        let numbered = folder.join("Ridge001.terrain");
        assert_eq!(incremented_path(&numbered), folder.join("Ridge002.terrain"));
        let short = folder.join("Abc.terrain");
        std::fs::write(&short, "").unwrap();
        assert_eq!(incremented_path(&short), folder.join("Abc001.terrain"));
        std::fs::remove_dir_all(&folder).unwrap();
        assert_eq!(dotnet_int_parse(" 12"), Some(12));
        assert_eq!(dotnet_int_parse("-12"), Some(-12));
        assert_eq!(dotnet_int_parse("a12"), None);
        assert_eq!(dotnet_format_000(-12), "-012");
    }

    #[test]
    fn obsolete_bake_data_sits_beside_the_project() {
        let folder = std::env::temp_dir().join(format!("nerthus-bakedata-{}", std::process::id()));
        let project = folder.join("Ridge.terrain");
        let bakes = obsolete_bakes_folder(&project).unwrap();
        assert_eq!(bakes, folder.join("BakeData").join("Ridge"));
        std::fs::create_dir_all(bakes.join("Nodes")).unwrap();
        std::fs::write(bakes.join("a.graw"), [0u8; 1000]).unwrap();
        std::fs::write(bakes.join("Nodes").join("b"), [0u8; 536]).unwrap();
        assert_eq!(folder_size(&bakes), 1536);
        assert_eq!(crate::ui::byte_size(1536), "1.5 KB");
        assert_eq!(crate::ui::byte_size(0), "0 b");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn read_only_follows_the_write_permission() {
        use std::os::unix::fs::PermissionsExt;
        let folder = std::env::temp_dir().join(format!("nerthus-readonly-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("Locked.terrain");
        std::fs::write(&path, "{}").unwrap();
        assert!(!is_read_only(&path));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
        assert!(is_read_only(&path));
        // Owned by this user: only the owner's permissions count.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o466)).unwrap();
        assert!(is_read_only(&path));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!is_read_only(&path));
        assert!(!is_read_only(&folder.join("missing.terrain")));
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
