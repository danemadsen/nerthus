//! The status bar (`Shell` `StatusBar`, padding 8, 0, 8, 2): the latest
//! message on the left; on the right, what the engine is doing, the
//! compute device and memory (`Button-IconText`, monospace). The device
//! button (`btnCPU`) shows only in CPU-only mode, or when the benchmark's
//! fastest device is off; it opens Options at Compute.

use egui::{Align, Layout, Pos2, Rect, RichText, Sense, Ui, UiBuilder, Vec2};

use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::help_windows;
use crate::ui::options_window::OptionsTab;
use crate::ui::tool_commands::ToolCommand;

pub const HEIGHT: f32 = 20.0;

/// `BrushName.Teal`: the memory readout, once a version has started for
/// the first time.
const MEMORY_COLOR: egui::Color32 = egui::Color32::from_rgb(0x47, 0xb4, 0xb3);

/// Whether `eco.dat` was in the data folder as Nerthus started.
static ECO_MARKED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// `SetupInterface`: notes whether a version has started for the first
/// time before (`eco.dat`), as the interface is set up and before this
/// start's What's New check can write it.
pub fn note_eco_mark() {
    ECO_MARKED.get_or_init(|| crate::user::data_folder().is_some_and(|folder| folder.join(help_windows::ECO_FILE).exists()));
}

/// The memory readout is teal once a version has started for the first
/// time before this start, the label colour until then.
fn memory_colour() -> egui::Color32 {
    note_eco_mark();
    let marked = ECO_MARKED.get().copied().unwrap_or(false);
    if marked { MEMORY_COLOR } else { theme::LABEL_DEFAULT }
}

/// `process_gib` is the process's memory (`SystemInformationDiagnosticsPlugin`).
pub fn show(ui: &mut Ui, rect: Rect, editor: &Editor, process_gib: Option<f64>, commands: &mut Vec<Command>) {
    ui.painter().rect_filled(rect, 0.0, theme::WINDOW_BACKGROUND);
    let content = Rect::from_min_max(rect.min + Vec2::new(8.0, 0.0), rect.max - Vec2::new(8.0, 2.0));
    let builder = UiBuilder::new().max_rect(content).layout(Layout::left_to_right(Align::Center));
    let mut bar = ui.new_child(builder);

    let message = editor.messages.last().cloned().unwrap_or_default();
    let message_text = RichText::new(message).font(theme::interface(10.0)).color(theme::LABEL_DEFAULT);
    bar.add(egui::Label::new(message_text).truncate());

    bar.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        // `btnMemory`: the cached maps against the process, as Humanizer's
        // "0.0 GB" writes them.
        let gigabytes = |bytes: f64| bytes / (1024.0 * 1024.0 * 1024.0);
        let cached = gigabytes(editor.cache_bytes as f64);
        let memory = match process_gib {
            Some(process) => format!("{cached:.1} GB / {process:.1} GB"),
            None => format!("{cached:.1} GB"),
        };
        icon_text(ui, "binary", &memory, 11.0, memory_colour());
        let device = if editor.compute.cpu_only { Some("CPU Only") } else { editor.compute.fastest_disabled.as_deref() };
        if let Some(device) = device
            && icon_text(ui, "cpu-2", device, 10.0, theme::LABEL_DEFAULT).clicked()
        {
            commands.push(Command::ShowOptions(Some(OptionsTab::Compute)));
        }
        // `NahimicWarning`: shows the warning again.
        if crate::nahimic::detected() {
            ui.add_space(8.0);
            let warning_text = RichText::new("⚠ Nahimic Detected!").font(theme::interface(10.0)).color(theme::LABEL_DEFAULT);
            let warning = ui.add(egui::Label::new(warning_text).sense(Sense::click()));
            let warning = warning.on_hover_cursor(egui::CursorIcon::PointingHand);
            if warning.clicked() {
                commands.push(Command::Tools(ToolCommand::ShowNahimicWarning));
            }
        }
        let state = if !editor.engine_active {
            "Engine suspended"
        } else if editor.building {
            "Building"
        } else {
            "Idle"
        };
        ui.add_space(8.0);
        let state_text = RichText::new(state).font(theme::interface(10.0)).color(theme::LABEL_DEFAULT);
        ui.label(state_text);
    });
}

/// `Button-IconText` (padding 8, 2; the icon as wide as `MinWidth` 16;
/// the text 6 past it in monospace).
fn icon_text(ui: &mut Ui, icon: &str, text: &str, size: f32, color: egui::Color32) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::monospace(size), color);
    let width = 8.0 + 16.0 + 6.0 + galley.size().x + 8.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, rect_height(ui)), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 2.0, theme::BUTTON_HOVER);
    }
    let tint = if response.hovered() { theme::ACCENT_MUTED_GRAY } else { color };
    let icon_rect = Rect::from_min_size(Pos2::new(rect.left() + 8.0, rect.center().y - 8.0), Vec2::splat(16.0));
    icons::paint(ui, icon_rect, icon, tint);
    let text_position = Pos2::new(icon_rect.right() + 6.0, rect.center().y - galley.size().y / 2.0 + 0.5);
    ui.painter().galley(text_position, galley, tint);
    response
}

fn rect_height(ui: &Ui) -> f32 {
    ui.available_height().max(16.0)
}
