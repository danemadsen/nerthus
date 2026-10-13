//! The message boxes: with Yes and No (`MBox(text, caption,
//! MessageBoxButton.YesNo)`, the command Yes runs), with Yes, No and
//! Cancel (`MboxFull(…, YesNoCancel)`, the save question), or with OK
//! (`MessageBox.Show`). A task dialog's footer shows under the buttons.

use egui::{Align, Layout, RichText};

use crate::theme;
use crate::ui::commands::Command;
use crate::ui::UiState;

/// A question waiting for Yes or No, or a message waiting for OK.
#[derive(Clone, Debug, PartialEq)]
pub struct Prompt {
    pub caption: String,
    pub text: String,
    /// What Yes runs; none for a message.
    pub yes: Option<Command>,
    /// What No runs, when it does more than close the box.
    pub no: Option<Command>,
    /// Whether there is a Cancel button (Yes, No and Cancel).
    pub cancel: bool,
    /// The task dialog's footer (`MBox`'s footnote).
    pub footnote: Option<String>,
}

impl Prompt {
    pub fn new(caption: &str, text: &str, yes: Command) -> Self {
        Self { caption: caption.to_string(), text: text.to_string(), yes: Some(yes), no: None, cancel: false, footnote: None }
    }

    pub fn message(caption: &str, text: &str) -> Self {
        Self { caption: caption.to_string(), text: text.to_string(), yes: None, no: None, cancel: false, footnote: None }
    }

    /// Yes, No and Cancel, each with its own command but Cancel (which
    /// only closes the box).
    pub fn yes_no_cancel(caption: &str, text: &str, yes: Command, no: Command) -> Self {
        Self { caption: caption.to_string(), text: text.to_string(), yes: Some(yes), no: Some(no), cancel: true, footnote: None }
    }
}

/// The prompt, while one is open; Yes (and No, when it has a command)
/// queues its command.
pub fn show(ctx: &egui::Context, ui_state: &mut UiState, commands: &mut Vec<Command>) {
    let Some(prompt) = ui_state.prompt.as_ref() else {
        return;
    };
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("message-box"));
    let response = modal.show(ctx, |ui| {
        ui.set_width(420.0);
        let caption = RichText::new(prompt.caption.as_str());
        ui.label(caption.font(theme::semibold(13.0)).color(theme::LABEL_EMPHASIS));
        ui.add_space(8.0);
        let text = RichText::new(prompt.text.as_str());
        ui.label(text.font(theme::interface(12.0)).color(theme::LABEL_DEFAULT));
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if prompt.yes.is_none() {
                if ui.button("OK").clicked() {
                    answer = Some(Answer::No);
                }
                return;
            }
            if prompt.cancel && ui.button("Cancel").clicked() {
                answer = Some(Answer::Cancel);
            }
            if ui.button("No").clicked() {
                answer = Some(Answer::No);
            }
            if ui.button("Yes").clicked() {
                answer = Some(Answer::Yes);
            }
        });
        if let Some(footnote) = &prompt.footnote {
            ui.add_space(10.0);
            ui.separator();
            let footnote = RichText::new(footnote.as_str()).font(theme::interface(11.0)).color(theme::ACCENT_YELLOW);
            ui.label(footnote);
        }
    });
    match answer {
        Some(Answer::Yes) => commands.extend(prompt.yes.clone()),
        Some(Answer::No) => commands.extend(prompt.no.clone()),
        Some(Answer::Cancel) | None => {}
    }
    if answer.is_some() || response.should_close() {
        ui_state.prompt = None;
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Answer {
    Yes,
    No,
    Cancel,
}
