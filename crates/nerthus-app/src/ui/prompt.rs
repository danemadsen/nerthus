//! The message boxes: with Yes and No (`MBox(text, caption,
//! MessageBoxButton.YesNo)`, the command Yes runs), with Yes, No and
//! Cancel (`MboxFull(…, YesNoCancel)`, the save question), with OK
//! (`MessageBox.Show`), or with a task dialog's own button and Close. A
//! task dialog's footer shows under the buttons.
//!
//! Each is laid out as a task dialog is at 96 dpi: an upper panel with the
//! icon, the main instruction (the caption) and the text, then a line and
//! a strip with the buttons at its right, then the footer.

use egui::{Align, Color32, CornerRadius, Layout, Margin, RichText, Sense, Ui, Vec2};

use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::UiState;

/// The window's width inside its border.
const WIDTH: f32 = 350.0;
/// Round the upper panel.
const PANEL_MARGIN: i8 = 10;
/// The icon at the panel's top left, and the gap between it and the words.
const ICON_SIZE: f32 = 32.0;
const ICON_GAP: f32 = 10.0;
/// The main instruction's font size and line pitch, and the gap under it.
const HEADING_SIZE: f32 = 16.0;
const HEADING_LINE: f32 = 21.0;
const HEADING_GAP: f32 = 10.0;
/// The text's (and the footer's) font size and line pitch, and the space
/// kept above and below the text.
const TEXT_SIZE: f32 = 12.0;
const TEXT_LINE: f32 = 15.0;
const TEXT_MARGIN: f32 = 5.0;
/// Round the buttons in their strip.
const STRIP_MARGIN: Margin = Margin { left: 10, right: 11, top: 9, bottom: 10 };
/// A button's least size, the space either side of its words, and the gap
/// between buttons.
const BUTTON_MIN_SIZE: Vec2 = Vec2::new(70.0, 21.0);
const BUTTON_PADDING: f32 = 16.0;
const BUTTON_GAP: f32 = 8.0;
/// Round the footer's text.
const FOOTER_MARGIN: Margin = Margin { left: 10, right: 10, top: 8, bottom: 8 };

/// A question waiting for Yes or No, or a message waiting for OK.
#[derive(Clone, Debug, PartialEq)]
pub struct Prompt {
    pub caption: String,
    pub text: String,
    /// What Yes runs; none for a message.
    pub yes: Option<Command>,
    /// What No runs, when it does more than close the box.
    pub no: Option<Command>,
    /// What closing the box without an answer (Escape, a click outside)
    /// runs, when anything.
    pub dismissed: Option<Command>,
    /// Whether there is a Cancel button (Yes, No and Cancel).
    pub cancel: bool,
    /// The task dialog's footer (`MBox`'s footnote).
    pub footnote: Option<String>,
    /// What the Yes and No buttons say.
    pub yes_text: String,
    pub no_text: String,
    /// Whether the warning icon shows beside the main instruction
    /// (`TaskDialogIcon.Warning`).
    pub warning: bool,
}

impl Prompt {
    pub fn new(caption: &str, text: &str, yes: Command) -> Self {
        let mut prompt = Self::message(caption, text);
        prompt.yes = Some(yes);
        prompt
    }

    pub fn message(caption: &str, text: &str) -> Self {
        Self {
            caption: caption.to_string(),
            text: text.to_string(),
            yes: None,
            no: None,
            dismissed: None,
            cancel: false,
            footnote: None,
            yes_text: "Yes".to_string(),
            no_text: "No".to_string(),
            warning: false,
        }
    }

    /// Yes, No and Cancel, each with its own command but Cancel (which
    /// only closes the box).
    pub fn yes_no_cancel(caption: &str, text: &str, yes: Command, no: Command) -> Self {
        let mut prompt = Self::new(caption, text, yes);
        prompt.no = Some(no);
        prompt.cancel = true;
        prompt
    }

    /// A task dialog with a button of its own, which runs `command`, and
    /// Close.
    pub fn with_button(caption: &str, text: &str, button: &str, command: Command) -> Self {
        let mut prompt = Self::new(caption, text, command);
        prompt.yes_text = button.to_string();
        prompt.no_text = "Close".to_string();
        prompt
    }
}

/// The prompt, while one is open; Yes (and No, when it has a command)
/// queues its command.
pub fn show(ctx: &egui::Context, ui_state: &mut UiState, commands: &mut Vec<Command>) {
    let Some(prompt) = ui_state.prompt.as_ref() else {
        return;
    };
    let mut frame = egui::Frame::popup(&ctx.global_style());
    frame.inner_margin = Margin::ZERO;
    // The lower panel's corners follow the window's, inside its border.
    let border = frame.stroke.width as u8;
    let lower_radius = frame.corner_radius.sw.saturating_sub(border);

    let modal = egui::Modal::new(egui::Id::new("message-box")).frame(frame);
    let response = modal.show(ctx, |ui| {
        ui.set_width(WIDTH);
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        upper_panel(ui, prompt);
        lower_panel(ui, prompt, lower_radius)
    });

    // Asked once: asking takes the Escape press.
    let closing = response.should_close();
    let answer = response.inner;
    match answer {
        Some(Answer::Yes) => commands.extend(prompt.yes.clone()),
        Some(Answer::No) => commands.extend(prompt.no.clone()),
        Some(Answer::Cancel) => {}
        None if closing => commands.extend(prompt.dismissed.clone()),
        None => {}
    }
    if answer.is_some() || closing {
        ui_state.prompt = None;
    }
}

/// The icon, when there is one, beside the main instruction over the text.
/// The panel ends 10 under the icon or 15 under the text, whichever is
/// lower.
fn upper_panel(ui: &mut Ui, prompt: &Prompt) {
    let panel = egui::Frame::new().inner_margin(PANEL_MARGIN);
    panel.show(ui, |ui| {
        ui.horizontal_top(|ui| {
            if prompt.warning {
                let icon_size = Vec2::splat(ICON_SIZE);
                let (icon_rect, _) = ui.allocate_exact_size(icon_size, Sense::hover());
                icons::paint(ui, icon_rect, "alert-triangle", theme::ACCENT_YELLOW);
                ui.add_space(ICON_GAP);
            }
            ui.vertical(|ui| {
                if !prompt.caption.is_empty() {
                    let heading = styled(&prompt.caption, HEADING_SIZE, HEADING_LINE, theme::ACCENT_MUTED_BLUE);
                    ui.add(egui::Label::new(heading).wrap());
                    ui.add_space(HEADING_GAP);
                }
                ui.add_space(TEXT_MARGIN);
                if !prompt.text.is_empty() {
                    let text = styled(&prompt.text, TEXT_SIZE, TEXT_LINE, theme::LABEL_DEFAULT);
                    ui.add(egui::Label::new(text).wrap());
                }
                ui.add_space(TEXT_MARGIN);
            });
        });
    });
}

/// Under a line, the strip with the buttons, then the footer under a line
/// of its own. Returns the button clicked.
fn lower_panel(ui: &mut Ui, prompt: &Prompt, corner_radius: u8) -> Option<Answer> {
    divider(ui);
    let corners = CornerRadius { nw: 0, ne: 0, sw: corner_radius, se: corner_radius };
    let lower = egui::Frame::new().fill(theme::WINDOW_BACKGROUND).corner_radius(corners);
    let shown = lower.show(ui, |ui| {
        ui.set_width(ui.available_width());
        let strip = egui::Frame::new().inner_margin(STRIP_MARGIN);
        let clicked = strip.show(ui, |ui| buttons(ui, prompt));
        if let Some(footnote) = &prompt.footnote {
            divider(ui);
            let footer = egui::Frame::new().inner_margin(FOOTER_MARGIN);
            let text = styled(footnote, TEXT_SIZE, TEXT_LINE, theme::ACCENT_YELLOW);
            footer.show(ui, |ui| ui.add(egui::Label::new(text).wrap()));
        }
        clicked.inner
    });
    shown.inner
}

/// The buttons from the right: OK alone for a message, else Cancel (when
/// there is one), No and Yes. The row is aligned to its top: centred, it
/// would take all the height left in the window.
fn buttons(ui: &mut Ui, prompt: &Prompt) -> Option<Answer> {
    ui.spacing_mut().item_spacing.x = BUTTON_GAP;
    ui.spacing_mut().button_padding.x = BUTTON_PADDING;
    let row = ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        if prompt.yes.is_none() {
            return button(ui, "OK").then_some(Answer::No);
        }
        let mut answer = None;
        if prompt.cancel && button(ui, "Cancel") {
            answer = Some(Answer::Cancel);
        }
        if button(ui, &prompt.no_text) {
            answer = Some(Answer::No);
        }
        if button(ui, &prompt.yes_text) {
            answer = Some(Answer::Yes);
        }
        answer
    });
    row.inner
}

/// A button at least 70 by 21; whether it was clicked.
fn button(ui: &mut Ui, text: &str) -> bool {
    let button = egui::Button::new(text).min_size(BUTTON_MIN_SIZE);
    ui.add(button).clicked()
}

/// A line across the window, between its panels.
fn divider(ui: &mut Ui) {
    let size = Vec2::new(ui.available_width(), 1.0);
    let (line, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(line, 0.0, theme::MENU_BORDER);
}

/// `text` in the interface font at `size`, its lines `line` apart.
fn styled(text: &str, size: f32, line: f32, color: Color32) -> RichText {
    let rich = RichText::new(text).font(theme::interface(size)).color(color);
    rich.line_height(Some(line))
}

#[derive(Clone, Copy, PartialEq)]
enum Answer {
    Yes,
    No,
    Cancel,
}

#[cfg(test)]
mod tests {
    use egui::{Event, Key, Modifiers, Pos2, RawInput, Rect};

    use super::*;
    use crate::files::FileCommand;

    /// One frame of the prompt with `events`; the commands it queued.
    fn prompt_frame(context: &egui::Context, ui_state: &mut UiState, events: Vec<Event>) -> Vec<Command> {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let input = RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut commands = Vec::new();
        let output = context.run_ui(input, |ui| show(ui.ctx(), ui_state, &mut commands));
        output.drop_without_applying_deltas();
        commands
    }

    #[test]
    fn escape_closes_the_box_and_runs_what_dismissing_it_runs() {
        let context = egui::Context::default();
        let mut ui_state = UiState::default();
        let mut message = Prompt::message("Caption", "Text");
        message.dismissed = Some(Command::File(FileCommand::DiscardChanges));
        ui_state.prompt = Some(message);

        // The box is the top one from its second frame on.
        let first_frame = prompt_frame(&context, &mut ui_state, Vec::new());
        assert!(first_frame.is_empty());
        let escape = Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        let second_frame = prompt_frame(&context, &mut ui_state, vec![escape]);

        assert_eq!(second_frame, vec![Command::File(FileCommand::DiscardChanges)]);
        assert!(ui_state.prompt.is_none());
    }
}
