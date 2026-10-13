//! `Shell.ShowCommandLine` (Vars › Show Build Command Line): a task dialog
//! with the command that builds the project from a prompt, one argument
//! to a line: `nerthus-build`, the project, and every variable as
//! `-v name=value`. Copy to Clipboard copies it as one line; Close only
//! closes.

use std::path::PathBuf;

use egui::{Align, Id, Layout, RichText, Stroke, Vec2};

use crate::editor::Editor;
use crate::icons;
use crate::theme;

/// `TaskDialog.Width` (400 dialog units) in points, roughly.
const WIDTH: f32 = 600.0;

/// The dialog while it shows: the command as shown and as copied.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandLineWindow {
    shown: String,
    copied: String,
}

/// In quotes.
fn quoted(text: &str) -> String {
    format!("\"{text}\"")
}

/// In quotes when it holds a letter.
fn quoted_if_text(text: &str) -> String {
    if text.chars().any(char::is_alphabetic) { quoted(text) } else { text.to_string() }
}

/// `nerthus-build` beside the running program.
fn builder_path() -> PathBuf {
    let name = if cfg!(windows) { "nerthus-build.exe" } else { "nerthus-build" };
    let program = std::env::current_exe().unwrap_or_default();
    let folder = program.parent().map(PathBuf::from).unwrap_or_default();
    folder.join(name)
}

impl CommandLineWindow {
    pub fn new(editor: &Editor) -> Self {
        let builder = builder_path().display().to_string();
        let project = editor.document.path.as_ref().map(|path| path.display().to_string()).unwrap_or_default();
        let variables: Vec<String> = editor.document.automation().variables.iter().map(|(key, variable)| format!("-v {key}={}", quoted_if_text(&variable.value.to_text()))).collect();
        Self::from_parts(&builder, &project, &variables)
    }

    /// `content` (indented lines) and `text` (one line, what is copied).
    fn from_parts(builder: &str, project: &str, variables: &[String]) -> Self {
        let mut shown = format!("{}\n    build {}", quoted(builder), quoted(project));
        let mut copied = format!("{} build {}", quoted(builder), quoted(project));
        for variable in variables {
            shown.push_str("\n    ");
            shown.push_str(variable);
            copied.push(' ');
            copied.push_str(variable);
        }
        Self { shown, copied }
    }
}

/// The dialog; Copy to Clipboard copies the command and closes it, as
/// Close and Escape do without copying.
pub fn show(ctx: &egui::Context, window: &mut Option<CommandLineWindow>) {
    let Some(dialog) = window.as_ref() else {
        return;
    };
    let mut copy = false;
    let mut close = false;
    let mut frame = egui::Frame::popup(&ctx.global_style());
    frame.fill = theme::WINDOW_BACKGROUND;
    frame.inner_margin = egui::Margin::same(12);
    let mut modal = egui::Modal::new(Id::new("build-command-line"));
    modal = modal.frame(frame);
    let response = modal.show(ctx, |ui| {
        ui.set_width(WIDTH);
        let instruction = RichText::new("Build Command Line").font(theme::semibold(15.0)).color(theme::ACCENT_BLUE);
        ui.label(instruction);
        ui.add_space(10.0);
        let content = RichText::new(dialog.shown.as_str()).font(theme::interface(12.0)).color(theme::LABEL_EMPHASIS);
        ui.add(egui::Label::new(content).wrap());
        ui.add_space(16.0);
        ui.allocate_ui_with_layout(Vec2::new(WIDTH, 26.0), Layout::right_to_left(Align::Center), |ui| {
            if task_button(ui, "Close").clicked() {
                close = true;
            }
            ui.add_space(8.0);
            if task_button(ui, "Copy to Clipboard").clicked() {
                copy = true;
            }
        });
        ui.add_space(10.0);
        ui.separator();
        // `Footer` with `TaskDialogIcon.Information`.
        ui.horizontal(|ui| {
            let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(icons::DEFAULT_SIZE), egui::Sense::hover());
            icons::paint(ui, icon_rect, "info-square-rounded", theme::ACCENT_BLUE);
            let footer = RichText::new("You can use this command line directly from any command prompt.").font(theme::interface(11.0)).color(theme::LABEL_DEFAULT);
            ui.label(footer);
        });
    });
    if copy {
        ctx.copy_text(dialog.copied.clone());
    }
    if copy || close || response.should_close() {
        *window = None;
    }
}

fn task_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let label = RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_EMPHASIS);
    let mut widget = egui::Button::new(label);
    widget = widget.fill(theme::BUTTON_BACKGROUND);
    widget = widget.stroke(Stroke::new(1.0, theme::BUTTON_BORDER));
    widget = widget.min_size(Vec2::new(80.0, 26.0));
    ui.add(widget)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_is_shown_a_line_an_argument_and_copied_as_one() {
        let variables = vec![format!("-v Height={}", quoted_if_text("0.5")), format!("-v Style={}", quoted_if_text("Ridge"))];
        let window = CommandLineWindow::from_parts("/bin/nerthus-build", "/p/a b.terrain", &variables);
        assert_eq!(window.shown, "\"/bin/nerthus-build\"\n    build \"/p/a b.terrain\"\n    -v Height=0.5\n    -v Style=\"Ridge\"");
        assert_eq!(window.copied, "\"/bin/nerthus-build\" build \"/p/a b.terrain\" -v Height=0.5 -v Style=\"Ridge\"");
    }
}
