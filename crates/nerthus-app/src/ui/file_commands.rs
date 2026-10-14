//! File commands with their questions. New, Open, a recent or dropped
//! file, Repair File, Open Examples and Exit first ask whether to save
//! changes (`SavePrompt`: only with unsaved changes and an undo history;
//! Exit asks whenever there are unsaved changes, `Shell_Closing`), and
//! Open, New and Repair whether to close while nodes build
//! (`PreventCloseWhileBuilding`; Open asks it first, New and Repair after
//! the save question, in `SetupNewFile`).

use std::path::Path;

use crate::editor::Editor;
use crate::files::{self, OpenMode, ExportForm, FileAction, FileCommand, FileDialog, Pending};
use crate::ui::commands::Command;
use crate::ui::graph_canvas::ZoomRequest;
use crate::ui::prompt::Prompt;
use crate::ui::UiState;

pub fn run(command: FileCommand, editor: &mut Editor, ui_state: &mut UiState) {
    match command {
        FileCommand::New => request(FileAction::New, editor, ui_state),
        FileCommand::Open => request(FileAction::Open, editor, ui_state),
        FileCommand::OpenRecent(path) => request(FileAction::OpenPath(path), editor, ui_state),
        FileCommand::OpenLatest => {
            let latest = editor.user.recent.files().first().cloned();
            if let Some(path) = latest {
                request(FileAction::OpenPath(path), editor, ui_state);
            }
        }
        // `Shell_Drop`: the save question, the file, and the Start dialog
        // closed.
        FileCommand::Dropped(path) => {
            editor.files.start_dialog_open = false;
            request(FileAction::OpenPath(path), editor, ui_state);
        }
        FileCommand::SaveIncremental => save_incremental(editor),
        FileCommand::SetAutosaveInterval(minutes) => {
            editor.files.autosave.set_interval_minutes(minutes);
            editor.user.options.set_autosave_interval(editor.files.autosave.interval);
        }
        FileCommand::ResetAutosaveTimer => editor.files.autosave.reset_timer(),
        // `OpenAutosaveEntry` and `OpenAutoSave` open without asking.
        FileCommand::OpenAutosaveEntry(path) => {
            if path.is_file() {
                files::open_project(editor, &path);
            }
        }
        FileCommand::OpenAutosave => editor.files.dialog = Some(FileDialog::Open { folder: files::autosaves_folder() }),
        FileCommand::RevertToSaved => revert_to_saved(editor, ui_state),
        FileCommand::RevertConfirmed => {
            if let Some(path) = editor.document.path.clone() {
                files::open_project(editor, &path);
            }
        }
        FileCommand::RepairFile => request(FileAction::Repair, editor, ui_state),
        FileCommand::OpenExamples => request(FileAction::Examples, editor, ui_state),
        FileCommand::StartDialog => editor.files.start_dialog_open = true,
        FileCommand::ImportMacro => editor.files.dialog = Some(FileDialog::InstallMacro),
        FileCommand::ImportGraph => editor.files.dialog = Some(FileDialog::ImportGraph),
        FileCommand::ImportTorlet => editor.files.dialog = Some(FileDialog::ImportTorlet),
        FileCommand::ImportImage => {
            let tab = ui_state.graph.tab;
            let position = ui_state.graph.drop_position();
            editor.files.dialog = Some(FileDialog::ImportImage { tab, position });
        }
        // `Shell.ExportMacro`: the project saved, then the dialog.
        FileCommand::ExportMacro => {
            if let Some(path) = editor.document.path.clone() {
                editor.save_to(&path);
            }
            editor.files.export_macro = Some(ExportForm::for_project(editor));
        }
        FileCommand::ExportTorlet => {
            let tab = ui_state.graph.tab;
            editor.files.dialog = Some(FileDialog::ExportTorlet { tab });
        }
        FileCommand::Exit => request(FileAction::Exit, editor, ui_state),
        FileCommand::SaveFirst => save_first(editor, ui_state),
        // No to the save question, or a Save As it asked for went through.
        FileCommand::DiscardChanges => {
            if let Some(pending) = editor.files.pending.as_mut() {
                pending.save_answered = true;
            }
            advance(editor, ui_state);
        }
        FileCommand::ContinueWhileBuilding => {
            if let Some(pending) = editor.files.pending.as_mut() {
                pending.building_answered = true;
            }
            advance(editor, ui_state);
        }
        FileCommand::InstallMacro(path) => files::install(editor, &path),
        FileCommand::AddMacro(id) => {
            let position = ui_state.graph.drop_position();
            let tab = ui_state.graph.tab;
            files::add_macro(editor, &id, position, tab);
        }
        // The Start dialog's New Project: `SetupNewFile` (only the
        // building question).
        FileCommand::StartNew => {
            editor.files.start_dialog_open = false;
            let pending = Pending { action: FileAction::New, save_answered: true, building_answered: false };
            editor.files.pending = Some(pending);
            advance(editor, ui_state);
        }
        FileCommand::StartOpen => {
            editor.files.start_dialog_open = false;
            editor.files.dialog = Some(FileDialog::Open { folder: None });
        }
        FileCommand::OpenDirect { path, mode } => {
            editor.files.start_dialog_open = false;
            if files::open_project(editor, &path) {
                apply_open_mode(editor, mode);
            }
        }
        FileCommand::OpenExample { path, mode } => open_example(editor, &path, mode),
        FileCommand::ExportMacroChecked => export_macro_checked(editor),
        // `RecoverOrphans` runs before the graph is made, so a saved tab
        // that was one of the orphans' is the one shown.
        FileCommand::RecoverOrphans => {
            editor.document.recover_orphans();
            ui_state.graph.tab = editor.document.opening_tab();
        }
        FileCommand::DeleteOrphans => {
            editor.document.delete_orphans();
            editor.changed();
        }
        FileCommand::DeleteObsoleteBakes(folder) => files::delete_obsolete_bakes(editor, &folder),
    }
}

/// Starts an action that may need answers first.
fn request(action: FileAction, editor: &mut Editor, ui_state: &mut UiState) {
    let pending = Pending { action, save_answered: false, building_answered: false };
    editor.files.pending = Some(pending);
    advance(editor, ui_state);
}

/// Asks the next question the pending action needs, or does it.
fn advance(editor: &mut Editor, ui_state: &mut UiState) {
    let Some(pending) = editor.files.pending.clone() else {
        return;
    };
    let building = editor.building || editor.baking;
    let asks_building_first = pending.action == FileAction::Open;
    let asks_building_after = matches!(pending.action, FileAction::New | FileAction::Repair);
    if asks_building_first && building && !pending.building_answered {
        ask_building(ui_state);
        return;
    }
    if !pending.save_answered && needs_save_question(&pending.action, editor) {
        ui_state.prompt = Some(save_question());
        return;
    }
    if asks_building_after && building && !pending.building_answered {
        ask_building(ui_state);
        return;
    }
    editor.files.pending = None;
    perform(pending.action, editor);
}

/// The save question (`MboxFull(…, YesNoCancel, TaskDialogIcon.Warning)`):
/// Yes saves first, No goes on without saving.
pub fn save_question() -> Prompt {
    let text = "There are unsaved changes in the current terrain. Do you want to save the current terrain before proceeding? ";
    let yes = Command::File(FileCommand::SaveFirst);
    let no = Command::File(FileCommand::DiscardChanges);
    let mut question = Prompt::yes_no_cancel("Save terrain?", text, yes, no);
    question.warning = true;
    question
}

/// `SavePrompt`'s condition (unsaved changes and something to undo), or
/// for Exit `Shell_Closing`'s (unsaved changes).
fn needs_save_question(action: &FileAction, editor: &Editor) -> bool {
    if *action == FileAction::Exit {
        return editor.document.dirty;
    }
    editor.document.dirty && editor.document.can_undo()
}

/// `PreventCloseWhileBuilding`.
fn ask_building(ui_state: &mut UiState) {
    let text = "Nodes are building, are you sure you want close the existing project?";
    ui_state.prompt = Some(Prompt::new("Nodes are building?", text, Command::File(FileCommand::ContinueWhileBuilding)));
}

/// The save question's Yes (`SaveFile`): saved, then on; an untitled
/// project asks where first. When that save does not happen, nothing
/// goes on.
fn save_first(editor: &mut Editor, ui_state: &mut UiState) {
    let Some(path) = editor.document.path.clone() else {
        editor.files.dialog = Some(FileDialog::SaveAsThenContinue);
        return;
    };
    editor.save_to(&path);
    if editor.document.dirty {
        editor.files.pending = None;
        return;
    }
    if let Some(pending) = editor.files.pending.as_mut() {
        pending.save_answered = true;
    }
    advance(editor, ui_state);
}

/// What each action does once answered.
fn perform(action: FileAction, editor: &mut Editor) {
    match action {
        // `CloseFile`, `SetupNewFile`.
        FileAction::New => files::new_project(editor),
        FileAction::Open => editor.files.dialog = Some(FileDialog::Open { folder: None }),
        FileAction::OpenPath(path) => {
            files::open_project(editor, &path);
        }
        FileAction::Repair => {
            files::new_project(editor);
            editor.files.dialog = Some(FileDialog::Repair);
        }
        FileAction::Examples => editor.files.examples_open = true,
        // `CloseFile` as Nerthus closes: its previews written first when
        // the project keeps them.
        FileAction::Exit => {
            if !editor.exit_when_frozen() {
                editor.files.exit_now = true;
            }
        }
    }
}

/// `Shell.SaveIncremental`: a saved project is saved again under the next
/// numbered name (`Increment`), its bakes copied along; an unsaved one is
/// saved as usual.
fn save_incremental(editor: &mut Editor) {
    let saved = editor.document.path.clone().filter(|path| path.is_file());
    let Some(path) = saved else {
        editor.save();
        return;
    };
    let next = files::incremented_path(&path);
    editor.save_to(&next);
}

/// `Shell.RevertToSaved`.
fn revert_to_saved(editor: &mut Editor, ui_state: &mut UiState) {
    let saved = editor.document.path.as_deref().is_some_and(Path::is_file);
    if !saved {
        let text = "This file was not saved, so it cannot be reverted to the saved state.";
        ui_state.prompt = Some(Prompt::message("Cannot revert", text));
        return;
    }
    let text = "Are you sure you want to undo all changes and revert to the last saved state?";
    ui_state.prompt = Some(Prompt::new("Revert to saved state?", text, Command::File(FileCommand::RevertConfirmed)));
}

/// An example opened from the examples (`ExamplesWindow.B_Click` and its
/// menu): as an untitled copy named after it, kept out of the recent
/// files; at a 512 preview, or with the engine suspended, when asked.
fn open_example(editor: &mut Editor, path: &Path, mode: OpenMode) {
    editor.files.examples_open = false;
    editor.files.start_dialog_open = false;
    if files::open_example(editor, path) {
        apply_open_mode(editor, mode);
    }
}

/// `LoadingMechanism`: ForceLowResolution previews at 512,
/// SuspendEngine leaves the engine suspended.
fn apply_open_mode(editor: &mut Editor, mode: OpenMode) {
    match mode {
        OpenMode::Normal => {}
        OpenMode::LowResolution => editor.set_preview_resolution(512),
        OpenMode::SuspendEngine => editor.engine_active = false,
    }
}

/// Export Macro, its questions answered: the save dialog when it saves a
/// file (`ExportMacroDialog.BtnOK_OnClick`), else straight into the
/// collection.
fn export_macro_checked(editor: &mut Editor) {
    let Some(form) = editor.files.export_macro.as_ref() else {
        return;
    };
    if form.save_file {
        let id = form.macro_id();
        editor.files.dialog = Some(FileDialog::ExportMacro { id });
    } else {
        files::export_to(editor, None);
    }
}

/// What the interface does each frame for files: commands from outside it
/// run, a message waiting shows when no other is open, a tab an import
/// added comes into view, and an alert raised shows.
pub fn take_requests(editor: &mut Editor, ui_state: &mut UiState, commands: &mut Vec<Command>) {
    for request in std::mem::take(&mut editor.files.requests) {
        commands.push(Command::File(request));
    }
    if ui_state.prompt.is_none() && !editor.files.messages.is_empty() {
        let message = editor.files.messages.remove(0);
        let mut prompt = Prompt::message(&message.caption, &message.text);
        prompt.yes = message.yes.map(Command::File);
        prompt.no = message.no.map(Command::File);
        prompt.dismissed = message.dismissed.map(Command::File);
        prompt.footnote = message.footnote;
        ui_state.prompt = Some(prompt);
    }
    if let Some((tab, frame)) = editor.files.show_tab.take() {
        ui_state.graph.tab = tab;
        if frame {
            ui_state.graph.zoom_request = Some(ZoomRequest::Extents);
        }
    }
    if let Some(alert) = editor.files.alert.take() {
        ui_state.alert = Some(alert);
    }
}
