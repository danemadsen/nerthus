//! The progress window (`ProgressDialog`): a long operation's title, what
//! it is doing and how far it got, centred over the editor with a drop
//! shadow, the wait cursor while it shows. Cancel only puts the window
//! away; the work goes on.

use egui::{Align, Align2, Id, Layout, RichText, Stroke, Vec2};

use crate::editor::Editor;
use crate::theme;

/// `MinWidth`.
const WIDTH: f32 = 480.0;

pub fn show(ctx: &egui::Context, editor: &mut Editor) {
    let Some(window) = editor.progress.as_mut() else {
        return;
    };
    if window.hidden {
        return;
    }
    let mut cancelled = false;
    let mut area = egui::Area::new(Id::new("progress-window"));
    area = area.order(egui::Order::Foreground);
    area = area.anchor(Align2::CENTER_CENTER, Vec2::ZERO);
    area.show(ctx, |ui| {
        // `DropShadowEffect`: 20 blur, 5 down, half black.
        let shadow = egui::Shadow { offset: [0, 5], blur: 20, spread: 0, color: egui::Color32::from_black_alpha(128) };
        let mut frame = egui::Frame::new();
        frame = frame.fill(theme::MENU_BACKGROUND);
        frame = frame.stroke(Stroke::new(1.5, theme::ACCENT_MUTED_PURPLE));
        frame = frame.corner_radius(6.0);
        frame = frame.inner_margin(16.0);
        frame = frame.shadow(shadow);
        frame.show(ui, |ui| {
            ui.set_min_width(WIDTH);
            ui.set_max_width(WIDTH);
            let title = RichText::new(window.title()).font(theme::semibold(12.0)).color(theme::LABEL_EMPHASIS);
            ui.add(egui::Label::new(title).wrap());
            ui.add_space(4.0);
            let label = RichText::new(window.text.as_str()).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT);
            ui.add(egui::Label::new(label).wrap());
            ui.add_space(8.0);
            let mut bar = egui::ProgressBar::new(window.fraction);
            bar = bar.desired_height(2.0);
            ui.add(bar);
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                if ui.button("Cancel").clicked() {
                    cancelled = true;
                }
            });
        });
    });
    ctx.set_cursor_icon(egui::CursorIcon::Wait);
    ctx.request_repaint();
    if cancelled {
        window.hidden = true;
    }
}
