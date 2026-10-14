//! Tools' and Help's commands (the Console, Verbose Logging, the Artifact
//! Explorer, Run Diagnostics, the help windows, the documentation and
//! update checks), the
//! keys the shell gives them (`Shell.OnPreviewKeyDown`: F12, backtick,
//! Esc and M), and drawing their windows and the first-run setup.

use egui::{Key, KeyboardShortcut, Modifiers};

use crate::editor::Editor;
use crate::ui::artifact_explorer::{self, ArtifactExplorer};
use crate::ui::commands::Command;
use crate::ui::console_window::{self, ConsoleWindow};
use crate::ui::diagnostics_window::{self, DiagnosticsRequest, DiagnosticsWindow};
use crate::ui::docs_window::{self, DocsWindow};
use crate::ui::help_windows::{self, UpdateDialog};
use crate::ui::nahimic_warning::{self, NahimicWarning};
use crate::ui::onboard::{self, Onboard};
use crate::ui::prompt::Prompt;
use crate::ui::status_bar;
use crate::ui::UiState;
use crate::updates::{DownloadOutcome, ProxySettings};
use crate::user::Preferences;

#[derive(Clone, Debug, PartialEq)]
pub enum ToolCommand {
    /// Tools › Console (F12, `ShowDebugConsole`).
    ShowConsole,
    /// Tools › Artifact Explorer….
    ShowArtifactExplorer,
    /// Tools › Run Diagnostics… and Help › Diagnostics….
    ShowDiagnostics,
    /// Tools › Verbose Logging, asked first; and its Yes.
    ToggleVerboseLogging,
    SetVerboseLogging(bool),
    /// Tools › MCP Server: on or off, now and for the next start.
    ToggleMcpServer,
    /// Help › Shortcut Reference, What's new in this version?, Check for
    /// updates and About Nerthus (and the title bar's version and Update
    /// buttons).
    ShowShortcuts,
    ShowWhatsNew,
    CheckForUpdates,
    ShowAbout,
    /// Help › User Guide and Node Reference, and the help links around
    /// the editor: the documentation at a page (`path#heading`).
    ShowDocs(String),
    /// The property area's Show Help for this Node: the selected node's
    /// page (`MiHelp_OnClick`).
    ShowNodeHelp,
    /// The downloaded update's installer run; with `close`, Nerthus
    /// closes at once (Yes to "Close Nerthus before proceeding?").
    InstallUpdate { path: std::path::PathBuf, close: bool },
    /// Esc: the preview build stops; with Ctrl the
    /// cancellation is reset.
    CancelPreviewBuild,
    ResetCancellation,
    /// "Nahimic Detected!" on the Start dialog or the status bar: the
    /// Nahimic warning.
    ShowNahimicWarning,
}

/// M asks the property area to open its Add Modifiers menu, as a flag in
/// egui's memory for the frame.
const ADD_MODIFIERS_ID: &str = "open-add-modifiers";

pub fn run(command: ToolCommand, editor: &mut Editor, ui_state: &mut UiState) {
    match command {
        ToolCommand::ShowConsole => {
            if ui_state.console.is_none() {
                ui_state.console = Some(ConsoleWindow::open());
            }
        }
        ToolCommand::ShowArtifactExplorer => {
            if ui_state.artifact_explorer.is_none() {
                ui_state.artifact_explorer = Some(ArtifactExplorer::open(editor));
            }
        }
        ToolCommand::ShowDiagnostics => {
            if ui_state.diagnostics.is_none() {
                ui_state.diagnostics = Some(DiagnosticsWindow::open());
            }
        }
        // `BtnVerbose_OnChecked` and `OnUnchecked`.
        ToolCommand::ToggleVerboseLogging => {
            let prompt = if crate::logging::verbose() {
                let text = "Are you sure you want to disable Verbose logging? This will create a new log file for this session.";
                Prompt::new("Switch to Standard logging", text, Command::Tools(ToolCommand::SetVerboseLogging(false)))
            } else {
                let text = "Are you sure you want to enable Verbose logging? This will create a new log file for this session.\n\nThis is mostly useful if you need to provide detailed technical logs to Tech Support.";
                Prompt::new("Switch to Verbose logging", text, Command::Tools(ToolCommand::SetVerboseLogging(true)))
            };
            ui_state.prompt = Some(prompt);
        }
        ToolCommand::SetVerboseLogging(verbose) => crate::logging::switch_verbose(verbose),
        ToolCommand::ToggleMcpServer => editor.mcp.toggle_requested = true,
        ToolCommand::ShowShortcuts => ui_state.help.shortcuts = true,
        ToolCommand::ShowWhatsNew => {
            if ui_state.help.whats_new.is_none() {
                ui_state.help.whats_new = Some(0);
            }
        }
        ToolCommand::ShowAbout => ui_state.help.about = true,
        ToolCommand::ShowDocs(target) => open_docs(ui_state, &target),
        ToolCommand::ShowNodeHelp => {
            let Some(node) = editor.active_node().and_then(|id| editor.document.node(id)) else {
                return;
            };
            log::info!("Open help for {}", node.type_name);
            let library = crate::docs::library();
            let target = library.node_page(&node.type_name).unwrap_or(crate::docs::NODE_REFERENCE).to_string();
            open_docs(ui_state, &target);
        }
        ToolCommand::InstallUpdate { path, close } => {
            crate::updates::run_installer(&path);
            // Closing does not ask about unsaved changes: the question
            // said they would be lost.
            if close {
                editor.files.exit_now = true;
            }
        }
        ToolCommand::CheckForUpdates => {
            let proxy = proxy_settings(&editor.user.preferences);
            match ui_state.updates.ask(proxy) {
                Some(answer) => ui_state.help.update = Some(UpdateDialog::new(answer)),
                None => editor.report("Checking for updates..."),
            }
        }
        ToolCommand::CancelPreviewBuild => {
            if editor.building {
                editor.cancel_build = true;
            }
        }
        ToolCommand::ResetCancellation => {
            log::info!("Manual reset of Cancellation Token.");
            editor.cancel_build = false;
        }
        ToolCommand::ShowNahimicWarning => {
            if ui_state.nahimic_warning.is_none() {
                ui_state.nahimic_warning = Some(NahimicWarning::default());
            }
        }
    }
}

/// Download Update's download: started when asked for; once it is in,
/// "Close Nerthus before proceeding?" (Yes closes Nerthus and runs the
/// installer, No only runs it).
fn update_download(editor: &mut Editor, ui_state: &mut UiState) {
    if let Some(token) = ui_state.help.download_requested.take() {
        let proxy = proxy_settings(&editor.user.preferences);
        ui_state.updates.start_download(&token, proxy);
    }
    match ui_state.updates.poll_download() {
        Some(DownloadOutcome::Finished(path)) => {
            let text = "Close Nerthus before proceeding? Unsaved work will be lost!\n\nOr click NO, save your work, close Nerthus, and then proceed with the installer.";
            let yes = Command::Tools(ToolCommand::InstallUpdate { path: path.clone(), close: true });
            let no = Command::Tools(ToolCommand::InstallUpdate { path, close: false });
            let mut prompt = Prompt::new("Close Nerthus?", text, yes);
            prompt.no = Some(no.clone());
            prompt.dismissed = Some(no);
            ui_state.prompt = Some(prompt);
        }
        Some(DownloadOutcome::Failed(error)) => editor.report(format!("The update could not be downloaded. {error}")),
        None => {}
    }
}

/// The documentation at `target`, in its window (opened if need be).
fn open_docs(ui_state: &mut UiState, target: &str) {
    match ui_state.docs.as_mut() {
        Some(window) => window.go(target),
        None => ui_state.docs = Some(DocsWindow::open(target)),
    }
}

/// Whether M asked for the Add Modifiers menu this frame (the property
/// area opens it when it shows a node, `btnAddModifier.IsSubmenuOpen`).
pub fn add_modifiers_wanted(ctx: &egui::Context) -> bool {
    let id = egui::Id::new(ADD_MODIFIERS_ID);
    ctx.data_mut(|data| data.remove_temp::<bool>(id)).unwrap_or(false)
}

/// The frame is over: an unanswered M is dropped.
pub fn forget_add_modifiers(ctx: &egui::Context) {
    let id = egui::Id::new(ADD_MODIFIERS_ID);
    ctx.data_mut(|data| data.remove::<bool>(id));
}

/// `Locations.UseProxy` and `Locations.Proxy`.
pub fn proxy_settings(preferences: &Preferences) -> ProxySettings {
    let use_proxy = preferences.member("Locations", "UseProxy").as_bool().unwrap_or(false);
    let proxy = preferences.member("Locations", "Proxy");
    let proxy = proxy.as_str().unwrap_or_default().to_string();
    ProxySettings { use_proxy, proxy }
}

/// The shell's keys for these commands (`Shell.OnPreviewKeyDown`); none
/// while a text field has the keyboard.
pub fn shortcuts(ctx: &egui::Context, pending: &mut Vec<Command>) {
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    // Esc is left for the graph and the windows too: the shell does not
    // handle it (it also ends a drag in the graph).
    let (escape, command_held) = ctx.input(|input| (input.key_pressed(Key::Escape), input.modifiers.command));
    if escape && command_held {
        pending.push(Command::Tools(ToolCommand::ResetCancellation));
    } else if escape {
        pending.push(Command::Tools(ToolCommand::CancelPreviewBuild));
    }
    let console = KeyboardShortcut::new(Modifiers::NONE, Key::F12);
    if ctx.input_mut(|input| input.consume_shortcut(&console)) {
        pending.push(Command::Tools(ToolCommand::ShowConsole));
    }
    // The backtick (`Key.Oem3`): suspend or resume.
    let backtick = KeyboardShortcut::new(Modifiers::NONE, Key::Backtick);
    if ctx.input_mut(|input| input.consume_shortcut(&backtick)) {
        pending.push(Command::ToggleEngine);
    }
    let add_modifiers = KeyboardShortcut::new(Modifiers::NONE, Key::M);
    if ctx.input_mut(|input| input.consume_shortcut(&add_modifiers)) {
        let id = egui::Id::new(ADD_MODIFIERS_ID);
        ctx.data_mut(|data| data.insert_temp(id, true));
    }
}

/// The first frame's work: the memory readout's colour noted, the
/// first-run setup, and the update check made at start.
pub fn start(editor: &Editor, ui_state: &mut UiState) {
    status_bar::note_eco_mark();
    if ui_state.onboard_at_launch {
        ui_state.onboard = Some(Onboard::open(editor.compute.cpu_only));
    }
    if ui_state.check_updates_at_launch {
        let proxy = proxy_settings(&editor.user.preferences);
        ui_state.updates.start(proxy, false);
    }
}

/// Whether a window that keeps the editor's keys to itself is up (the
/// modal dialogs).
pub fn modal_open(ui_state: &UiState) -> bool {
    ui_state.onboard.is_some() || ui_state.artifact_explorer.is_some() || ui_state.help.any_open() || ui_state.nahimic_warning.is_some()
}

/// Draws the windows open, and an update check's answer once it is in.
/// Returns whether Run Diagnostics put the settings back to their
/// defaults, for the interface to take them in.
pub fn show_windows(ctx: &egui::Context, editor: &mut Editor, ui_state: &mut UiState, pending: &mut Vec<Command>) -> bool {
    let mut settings_reset = false;
    for command in ui_state.take_tools_at_launch() {
        run(command, editor, ui_state);
    }
    if ui_state.whats_new_at_launch && ui_state.onboard.is_none() {
        ui_state.whats_new_at_launch = false;
        if help_windows::version_first_run() {
            ui_state.help.whats_new = Some(0);
            ui_state.help.whats_new_first_run = true;
        }
    }
    // The check made as Nerthus starts found Nahimic's service running.
    if crate::nahimic::take_warning_request() && ui_state.nahimic_warning.is_none() {
        ui_state.nahimic_warning = Some(NahimicWarning::default());
    }
    if let Some(answer) = ui_state.updates.poll() {
        ui_state.help.update = Some(UpdateDialog::new(answer));
    }
    update_download(editor, ui_state);
    if ui_state.updates.checking() || ui_state.updates.downloading() {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    if let Some(console) = ui_state.console.as_mut()
        && !console_window::show(ctx, console)
    {
        ui_state.console = None;
    }
    if let Some(explorer) = ui_state.artifact_explorer.as_mut()
        && !artifact_explorer::show(ctx, editor, explorer)
    {
        ui_state.artifact_explorer = None;
    }
    if let Some(diagnostics) = ui_state.diagnostics.as_mut() {
        let (open, request) = diagnostics_window::show(ctx, editor, diagnostics);
        match request {
            Some(DiagnosticsRequest::Restart) => pending.push(Command::Exit),
            Some(DiagnosticsRequest::SettingsReset) => {
                settings_reset = true;
                ui_state.onboard = Some(Onboard::open(editor.compute.cpu_only));
            }
            None => {}
        }
        if !open {
            ui_state.diagnostics = None;
        }
    }
    help_windows::show(ctx, &mut ui_state.help);
    if let Some(window) = ui_state.docs.as_mut()
        && !docs_window::show(ctx, window)
    {
        ui_state.docs = None;
    }
    if let Some(setup) = ui_state.onboard.as_mut()
        && !onboard::show(ctx, editor, setup, &mut ui_state.file_dialogs)
    {
        ui_state.onboard = None;
    }
    if let Some(warning) = ui_state.nahimic_warning.as_mut()
        && !nahimic_warning::show(ctx, warning)
    {
        ui_state.nahimic_warning = None;
    }
    settings_reset
}
