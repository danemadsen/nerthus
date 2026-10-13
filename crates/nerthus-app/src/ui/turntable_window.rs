//! Preview › Record Turntable Animation (`TurntableWindow`, 640 wide):
//! where the frames go (the first shown as `…_0000.png`), how many there
//! are (10 to 10000, 360 to begin with), their size and whether the grid
//! is left out. Record Animation closes it and starts the recording; the
//! path and the frame count are kept until another project is opened
//! (`TurntablePath`, `TurntableFrames`).

use std::path::PathBuf;

use egui::{Align, Id, Layout, RichText, Stroke, Vec2};
use nerthus_render::TurntableRequest;

use crate::editor::Editor;
use crate::theme;
use crate::ui::file_dialogs::{DialogKind, DialogRequest, FileDialogs};

const WIDTH: f32 = 640.0 - 48.0;
/// `cboResolution`'s sizes; 1920 by 1080 to begin with.
const RESOLUTIONS: [(u32, u32); 5] = [(1280, 720), (1280, 1280), (1920, 1080), (2560, 1440), (3840, 2160)];
const DEFAULT_RESOLUTION: usize = 2;
/// `NumericOnlyBehavior Default="360" Max="10000" Min="10"`.
const DEFAULT_FRAMES: u32 = 360;
const FRAME_RANGE: (u32, u32) = (10, 10000);
const DIALOG_KEY: &str = "turntable-path";

/// The window while it is open.
pub struct TurntableWindow {
    path: String,
    frames: String,
    resolution: usize,
    hide_helpers: bool,
    /// Record Animation pressed without a click (`--show`).
    pressed: bool,
}

/// What the window remembers until another project opens.
#[derive(Clone, Debug, Default)]
pub struct TurntableMemory {
    path: Option<String>,
    frames: Option<String>,
}

/// What the window asks for once it closes.
pub enum TurntableAnswer {
    Record(TurntableRequest),
    Closed,
}

impl TurntableWindow {
    /// `TurntableWindow_Loaded`: what was used last, or the screenshots
    /// folder's `<project>/<project>-xxxx-Turntable.png`.
    pub fn new(editor: &mut Editor, memory: &TurntableMemory) -> Self {
        let path = match &memory.path {
            Some(path) => path.clone(),
            None => {
                let folder = editor.user.preferences.screenshots_folder();
                let file_name = editor.document.file_name();
                let stem = std::path::Path::new(&file_name).file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_default();
                let tag = format!("{:04x}", editor.document.random_seed_below(0x10000));
                let path = folder.join(&stem).join(format!("{stem}-{tag}-Turntable.png"));
                path.display().to_string()
            }
        };
        let frames = memory.frames.clone().unwrap_or_else(|| DEFAULT_FRAMES.to_string());
        Self { path, frames, resolution: DEFAULT_RESOLUTION, hide_helpers: false, pressed: false }
    }

    /// Record Animation, at once, with so many frames.
    pub fn press_record(&mut self, frames: u32) {
        self.frames = frames.to_string();
        self.pressed = true;
    }

    /// `txtFrames` held to its range (`ContraintValue`).
    fn frame_count(&self) -> u32 {
        let parsed = self.frames.trim().parse::<u32>().unwrap_or(DEFAULT_FRAMES);
        parsed.clamp(FRAME_RANGE.0, FRAME_RANGE.1)
    }
}

/// The window; an answer once Record Animation is pressed or it closes.
pub fn show(ctx: &egui::Context, window: &mut TurntableWindow, memory: &mut TurntableMemory, dialogs: &mut FileDialogs) -> Option<TurntableAnswer> {
    if let Some(Some(picked)) = dialogs.take(DIALOG_KEY) {
        window.path = picked.display().to_string();
    }
    let mut record = false;
    let mut frame = egui::Frame::popup(&ctx.global_style());
    frame.fill = theme::PANEL_BACKGROUND;
    frame.inner_margin = egui::Margin::same(24);
    let mut modal = egui::Modal::new(Id::new("turntable-window"));
    modal = modal.frame(frame);
    let response = modal.show(ctx, |ui| {
        ui.set_width(WIDTH);
        // The heading on `Panel-Border` (padding 12, 4).
        let mut heading_frame = egui::Frame::new();
        heading_frame.fill = theme::PANEL_BORDER;
        heading_frame.inner_margin = egui::Margin::symmetric(12, 4);
        heading_frame.show(ui, |ui| {
            ui.set_width(WIDTH - 24.0);
            let heading = RichText::new("Turntable Animation Settings").font(theme::semibold(12.0)).color(theme::LABEL_DEFAULT);
            ui.label(heading);
        });
        ui.add_space(16.0);
        // `fbcTarget`: "Output Folder", the path and its browse button.
        ui.horizontal(|ui| {
            let header = RichText::new("Output Folder").font(theme::interface(12.0)).color(theme::LABEL_DEFAULT);
            ui.label(header);
            ui.add_space(8.0);
            let mut field = egui::TextEdit::singleline(&mut window.path);
            field = field.font(theme::semibold(12.0));
            field = field.text_color(theme::TEXTBOX_FILE_FOREGROUND);
            field = field.desired_width(ui.available_width() - 40.0);
            ui.add(field);
            if ui.add_enabled(!dialogs.pending(DIALOG_KEY), egui::Button::new("…")).clicked() {
                let current = PathBuf::from(&window.path);
                let file_name = current.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
                let kind = DialogKind::SaveFile { filter: "PNG File (*.png)".to_string(), extensions: vec!["png".to_string()], file_name };
                let request = DialogRequest { key: DIALOG_KEY.to_string(), title: "Output Folder".to_string(), kind, directory: current.parent().map(PathBuf::from) };
                dialogs.ask(request);
            }
        });
        // `UpdatePathPreview`: the first frame's name.
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            let preview = window.path.replace(".png", "_0000.png");
            let preview = RichText::new(preview).font(theme::monospace(12.0)).color(theme::LABEL_DEFAULT);
            // A 640 window: a long path is cut short.
            ui.add(egui::Label::new(preview).truncate());
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let label = RichText::new("Total Frames").font(theme::interface(12.0)).color(theme::LABEL_DEFAULT);
            ui.label(label);
            ui.add_space(8.0);
            let mut frames = egui::TextEdit::singleline(&mut window.frames);
            frames = frames.font(theme::monospace(12.0));
            frames = frames.text_color(theme::ACCENT_MUTED_BLUE);
            frames = frames.desired_width(72.0);
            let frames_response = ui.add(frames);
            if frames_response.changed() {
                window.frames.retain(|character| character.is_ascii_digit());
            }
            if frames_response.lost_focus() {
                window.frames = window.frame_count().to_string();
            }
            ui.add_space(8.0);
            let (width, height) = RESOLUTIONS[window.resolution];
            let mut combo = egui::ComboBox::from_id_salt("turntable-resolution");
            combo = combo.selected_text(format!("{width} x {height}"));
            combo = combo.width(118.0);
            combo.show_ui(ui, |ui| {
                for (index, (width, height)) in RESOLUTIONS.iter().enumerate() {
                    ui.selectable_value(&mut window.resolution, index, format!("{width} x {height}"));
                }
            });
            ui.add_space(8.0);
            let hide = RichText::new("Hide Grid and Compass").font(theme::interface(12.0)).color(theme::LABEL_DEFAULT);
            ui.checkbox(&mut window.hide_helpers, hide);
        });
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(16.0);
        ui.allocate_ui_with_layout(Vec2::new(WIDTH, 26.0), Layout::top_down(Align::Center), |ui| {
            let label = RichText::new("Record Animation").font(theme::interface(12.0)).color(theme::LABEL_EMPHASIS);
            let mut button = egui::Button::new(label);
            button = button.fill(theme::BUTTON_BACKGROUND);
            button = button.stroke(Stroke::new(1.0, theme::BUTTON_IMPORTANT_BORDER));
            button = button.min_size(Vec2::new(120.0, 26.0));
            if ui.add(button).clicked() {
                record = true;
            }
        });
    });
    record |= std::mem::take(&mut window.pressed);
    let closing = record || response.should_close();
    if !closing {
        return None;
    }
    // `TurntableWindow_Closing`.
    memory.path = Some(window.path.clone());
    memory.frames = Some(window.frame_count().to_string());
    if !record {
        return Some(TurntableAnswer::Closed);
    }
    let (width, height) = RESOLUTIONS[window.resolution];
    let request = TurntableRequest {
        base: PathBuf::from(window.path.replace(".png", "")),
        frames: window.frame_count(),
        size: bevy::math::UVec2::new(width, height),
        hide_helpers: window.hide_helpers,
    };
    Some(TurntableAnswer::Record(request))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_counts_stay_in_range() {
        let mut window = TurntableWindow { path: String::new(), frames: "5".to_string(), resolution: DEFAULT_RESOLUTION, hide_helpers: false, pressed: false };
        assert_eq!(window.frame_count(), 10);
        window.frames = "".to_string();
        assert_eq!(window.frame_count(), 360);
        window.frames = "120000".to_string();
        assert_eq!(window.frame_count(), 10000);
    }
}
