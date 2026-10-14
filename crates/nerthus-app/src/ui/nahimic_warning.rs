//! The Nahimic warning (`NahimicBlocker.ShowNahimicWarningMessage`): what
//! the software may do to Nerthus, with "Do not warn me again" and Close.
//! It shows by itself as Nerthus starts when Nahimic's service is running,
//! and again from "Nahimic Detected!" on the Start dialog or the status
//! bar.

use egui::{Align, Layout, RichText, Vec2};

use crate::icons;
use crate::theme;

const TITLE: &str = "WARNING!";
const HEADING: &str = "Nahimic may prevent Nerthus from working";
const TEXT: &str = "Nerthus has detected the 'Nahimic' service. It is an audio augmentation utility for games that comes pre-installed on many computers. Unfortunately, this is a very aggressive application that may try to disrupt Nerthus.\n\nIt may try to intercept Nerthus and in turn may break rendering, or make it impossible to use the Nerthus UI.\n\nWe recommend excluding Nerthus from it, or uninstalling the service completely. As it is an external application that forces itself onto Nerthus, we have no control over it.";

/// The warning while it shows.
#[derive(Default)]
pub struct NahimicWarning {
    /// "Do not warn me again" (`VerificationText`), unticked at first.
    do_not_warn: bool,
}

/// Draws the warning. Returns whether it stays open; once it closes with
/// the box ticked, it no longer shows by itself.
pub fn show(ctx: &egui::Context, warning: &mut NahimicWarning) -> bool {
    let mut closed = false;
    let modal = egui::Modal::new(egui::Id::new("nahimic-warning"));
    let response = modal.show(ctx, |ui| {
        ui.set_width(460.0);
        ui.label(RichText::new(TITLE).font(theme::semibold(13.0)).color(theme::LABEL_EMPHASIS));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), egui::Sense::hover());
            icons::paint(ui, icon_rect, "alert-triangle", theme::ACCENT_YELLOW);
            let heading = RichText::new(HEADING).font(theme::semibold(12.0)).color(theme::ACCENT_YELLOW);
            ui.label(heading);
        });
        ui.add_space(8.0);
        ui.label(RichText::new(TEXT).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let verification = RichText::new("Do not warn me again").font(theme::interface(12.0)).color(theme::LABEL_DEFAULT);
            ui.checkbox(&mut warning.do_not_warn, verification);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    closed = true;
                }
            });
        });
    });
    if !closed && !response.should_close() {
        return true;
    }
    if warning.do_not_warn {
        crate::nahimic::stop_warning();
    }
    false
}
