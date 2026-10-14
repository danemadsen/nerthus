//! File dialogs the newer windows ask for (Batch Build's Save Batch File,
//! Options' folders, …). Dialogs block and on macOS must run on the main
//! thread, so the interface queues a request under a key and a main-thread
//! system answers it; the window takes the answer by its key on a later
//! frame.

use std::path::PathBuf;

use bevy::ecs::system::NonSendMarker;
use bevy::prelude::ResMut;

use crate::ui::UiState;

/// What to ask for.
#[derive(Clone, Debug, PartialEq)]
pub enum DialogKind {
    /// A file to save, with a filter's name and its extensions.
    SaveFile { filter: String, extensions: Vec<String>, file_name: String },
    PickFolder,
}

/// A dialog waiting to be shown.
#[derive(Clone, Debug, PartialEq)]
pub struct DialogRequest {
    pub key: String,
    pub title: String,
    pub kind: DialogKind,
    /// Where it starts.
    pub directory: Option<PathBuf>,
}

/// The queue and the answers not taken yet.
#[derive(Default)]
pub struct FileDialogs {
    requests: Vec<DialogRequest>,
    answers: Vec<(String, Option<PathBuf>)>,
}

impl FileDialogs {
    pub fn ask(&mut self, request: DialogRequest) {
        self.requests.push(request);
    }

    /// The answer to the dialog asked for under `key`, once given (`Some(None)`
    /// when cancelled).
    pub fn take(&mut self, key: &str) -> Option<Option<PathBuf>> {
        let index = self.answers.iter().position(|(answered, _)| answered == key)?;
        Some(self.answers.remove(index).1)
    }

    /// Whether a dialog is waiting or showing for `key`.
    pub fn pending(&self, key: &str) -> bool {
        self.requests.iter().any(|request| request.key == key)
    }
}

/// Shows the queued dialogs, one a frame.
#[cfg(not(target_arch = "wasm32"))]
pub fn run(_main_thread: NonSendMarker, mut ui_state: ResMut<UiState>) {
    let Some(request) = ui_state.file_dialogs.requests.first().cloned() else {
        return;
    };
    let mut dialog = rfd::FileDialog::new();
    if !request.title.is_empty() {
        dialog = dialog.set_title(&request.title);
    }
    if let Some(directory) = request.directory.as_ref().filter(|directory| directory.is_dir()) {
        dialog = dialog.set_directory(directory);
    }
    let picked = match &request.kind {
        DialogKind::SaveFile { filter, extensions, file_name } => {
            let names: Vec<&str> = extensions.iter().map(String::as_str).collect();
            dialog = dialog.add_filter(filter, &names);
            if !file_name.is_empty() {
                dialog = dialog.set_file_name(file_name);
            }
            dialog.save_file()
        }
        DialogKind::PickFolder => dialog.pick_folder(),
    };
    let dialogs = &mut ui_state.file_dialogs;
    dialogs.requests.remove(0);
    dialogs.answers.push((request.key, picked));
}

/// The web build has no files or folders to choose: each dialog is
/// answered as cancelled.
#[cfg(target_arch = "wasm32")]
pub fn run(mut ui_state: ResMut<UiState>) {
    let dialogs = &mut ui_state.file_dialogs;
    for request in std::mem::take(&mut dialogs.requests) {
        dialogs.answers.push((request.key, None));
    }
}
