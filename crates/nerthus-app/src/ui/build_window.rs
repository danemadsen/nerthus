//! Build and Export: the confirmation (`B.VerifyAndBuild`'s task dialog)
//! and the window that follows a build (the build's console).

use egui::{Align, Color32, Layout, RichText, ScrollArea, Ui, Vec2};
use nerthus_engine::StateOverride;
use nerthus_export::build::BuildType;

use crate::editor::Editor;
use crate::export_build::LogLine;
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::UiState;

/// The build log's height in the progress window.
const LOG_HEIGHT: f32 = 240.0;

/// The confirmation's state while it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildConfirm {
    /// "Open folder after build finishes".
    pub open_folder: bool,
}

/// A command link (`TaskDialogButtonStyle.CommandLinks`): an arrow, the
/// title and a note under it.
fn command_link(ui: &mut Ui, title: &str, note: &str) -> bool {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 46.0), egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 3.0, theme::MENU_BACKGROUND_HOVER);
    }
    let arrow_color = if response.hovered() { theme::ACCENT_BLUE } else { theme::ACCENT_MUTED_BLUE };
    let arrow = egui::Rect::from_min_size(rect.min + Vec2::new(8.0, 8.0), Vec2::splat(16.0));
    icons::paint(ui, arrow, "chevron-right", arrow_color);
    let title_position = rect.min + Vec2::new(32.0, 6.0);
    let title_galley = ui.painter().layout_no_wrap(title.to_string(), theme::semibold(13.0), theme::LABEL_EMPHASIS);
    ui.painter().galley(title_position, title_galley, theme::LABEL_EMPHASIS);
    let note_position = rect.min + Vec2::new(32.0, 26.0);
    let note_galley = ui.painter().layout_no_wrap(note.to_string(), theme::interface(11.0), theme::LABEL_DEFAULT);
    ui.painter().galley(note_position, note_galley, theme::LABEL_DEFAULT);
    response.clicked()
}

/// "Build and Export?": what will be built, a link to the build settings,
/// Start Build or Cancel.
pub fn confirm(ctx: &egui::Context, editor: &Editor, ui_state: &mut UiState, commands: &mut Vec<Command>) {
    let Some(confirm) = ui_state.build_confirm.as_mut() else {
        return;
    };
    let definition = editor.document.build_definition();
    let terrain = editor.document.terrain();
    // `Total nodes to build`: the nodes neither baked nor bypassed.
    let unlocked = terrain.nodes.values().filter(|node| StateOverride::of(node) == StateOverride::None).count();
    let mut start = false;
    let mut cancel = false;
    let modal = egui::Modal::new(egui::Id::new("build-and-export"));
    let response = modal.show(ctx, |ui| {
        ui.set_width(460.0);
        let window_title = RichText::new("Build and Export?");
        ui.label(window_title.font(theme::interface(11.0)).color(theme::LABEL_DEFAULT));
        ui.add_space(6.0);
        // A Region build builds the region at its size.
        let region = definition.portion.as_ref().filter(|_| definition.build_type == BuildType::Region);
        let what = if region.is_some() { "Region" } else { "Terrain" };
        let size = region.map(|region| region.size.max(0) as usize).unwrap_or(definition.resolution);
        let instruction = RichText::new(format!("Build {what} at {size} x {size}?"));
        ui.label(instruction.font(theme::semibold(16.0)).color(theme::ACCENT_MUTED_BLUE));
        ui.add_space(8.0);
        ui.label(format!("Would you like to build this {what}? The build will export any files marked for save or output nodes."));
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.label("Adjust your ");
            if ui.link("build settings").clicked() {
                commands.push(Command::BuildSettings);
            }
            ui.label(", export file formats, and other details before proceeding. You can also access them in the Project menu.");
        });
        ui.add_space(6.0);
        ui.label(format!("Total nodes to build: {unlocked:02}"));
        ui.add_space(10.0);
        if command_link(ui, "Start Build", "Build the saved project and export its marked nodes") {
            start = true;
        }
        ui.add_space(8.0);
        ui.separator();
        ui.horizontal(|ui| {
            ui.checkbox(&mut confirm.open_folder, "Open folder after build finishes");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    });
    if start {
        let open_folder = confirm.open_folder;
        commands.push(Command::StartBuild { open_folder });
        ui_state.build_confirm = None;
        return;
    }
    if cancel || response.should_close() {
        ui_state.build_confirm = None;
    }
}

/// The build's progress: nodes built, files written, problems; Cancel
/// while it runs, Open Folder and Close after.
pub fn progress(ctx: &egui::Context, editor: &Editor, commands: &mut Vec<Command>) {
    let Some(build) = editor.export_build.as_ref() else {
        return;
    };
    let title = format!("Building {} at {} x {}", build.project_name, build.resolution, build.resolution);
    let mut open = true;
    let mut window = egui::Window::new(title);
    window = window.id(egui::Id::new("build-progress"));
    window = window.collapsible(false);
    window = window.resizable(true);
    window = window.default_size(Vec2::new(560.0, 380.0));
    window = window.pivot(egui::Align2::CENTER_CENTER);
    window = window.default_pos(ctx.content_rect().center());
    window = window.open(&mut open);
    window.show(ctx, |ui| {
            let status = if let Some(refusal) = &build.refusal {
                RichText::new(refusal.as_str()).color(theme::ACCENT_RED)
            } else if build.cancelled {
                RichText::new("Build cancelled").color(theme::ACCENT_YELLOW)
            } else if build.finished {
                let summary = format!("Built in {:.1} s: {} files written, {} problems", build.seconds(), build.files, build.failures);
                let color = if build.failures == 0 { theme::ACCENT_MUTED_GREEN } else { theme::ACCENT_YELLOW };
                RichText::new(summary).color(color)
            } else if let (None, Some(stage)) = (&build.current, &build.stage) {
                RichText::new(stage.as_str()).color(theme::LABEL_EMPHASIS)
            } else {
                let current = build.current.clone().unwrap_or_else(|| "Starting".to_string());
                let position = (build.built + 1).min(build.total.max(1));
                let bucket = build.bucket.map(|(index, count)| format!("Bucket {index} of {count}, ")).unwrap_or_default();
                RichText::new(format!("{bucket}Node {position} of {}: {current}", build.total)).color(theme::LABEL_EMPHASIS)
            };
            ui.label(status);
            if let Some(folder) = &build.folder {
                let location = RichText::new(folder.display().to_string());
                ui.label(location.font(theme::monospace(11.0)).color(theme::TEXTBOX_FILE_FOREGROUND));
            }
            ui.add_space(4.0);
            progress_bar(ui, build.progress());
            ui.add_space(6.0);

            let mut log = ScrollArea::vertical();
            log = log.max_height(LOG_HEIGHT);
            log = log.stick_to_bottom(true);
            log = log.auto_shrink([false, false]);
            log.show(ui, |ui| {
                for line in &build.log {
                    log_line(ui, line);
                }
            });

            ui.separator();
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if build.finished {
                        if ui.button("Close").clicked() {
                            commands.push(Command::CloseBuildWindow);
                        }
                        let can_open = build.folder.as_ref().is_some_and(|folder| folder.is_dir());
                        if ui.add_enabled(can_open, egui::Button::new("Open Folder")).clicked()
                            && let Some(folder) = &build.folder
                        {
                            commands.push(Command::OpenFolder(folder.clone()));
                        }
                    } else if ui.button("Cancel").clicked() {
                        commands.push(Command::CancelBuild);
                    }
                    let elapsed = RichText::new(format!("{:.0} s", build.seconds()));
                    ui.label(elapsed.font(theme::monospace(11.0)).color(theme::LABEL_DEFAULT));
                });
            });
            if !build.finished {
                ui.ctx().request_repaint();
            }
        });
    if !open {
        if build.finished {
            commands.push(Command::CloseBuildWindow);
        } else {
            commands.push(Command::CancelBuild);
        }
    }
}

fn progress_bar(ui: &mut Ui, fraction: f32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 4.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::PROGRESS_EMPTY);
    let mut filled = rect;
    filled.set_width(rect.width() * fraction.clamp(0.0, 1.0));
    ui.painter().rect_filled(filled, 0.0, theme::ACCENT_PRIMARY);
}

fn log_line(ui: &mut Ui, line: &LogLine) {
    let (text, color): (String, Color32) = match line {
        LogLine::Built { node, milliseconds } => (format!("{node}  {milliseconds} ms"), theme::LABEL_DEFAULT),
        LogLine::Failed { node, error } => (format!("{node} failed: {error}"), theme::ACCENT_RED),
        LogLine::Wrote(path) => {
            let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            (format!("  wrote {name}"), theme::ACCENT_MUTED_GREEN)
        }
        LogLine::Problem(problem) => (problem.clone(), theme::ACCENT_YELLOW),
        LogLine::Step(step) => (step.clone(), theme::ACCENT_MUTED_BLUE),
    };
    let line_text = RichText::new(text);
    ui.label(line_text.font(theme::monospace(11.0)).color(color));
}
