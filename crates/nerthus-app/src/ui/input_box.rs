//! The `InputBox`: a name asked for in a small window with OK and
//! Cancel. An empty name is refused ("Name cannot be empty."); with
//! `safe_filename`, so is one with characters a file name cannot hold.

use egui::{Align, Id, Key, Layout, RichText, Stroke, Vec2};

use crate::theme;

/// An input box while it is open.
#[derive(Clone, Debug, PartialEq)]
pub struct InputBox {
    /// What is asked for (`txbDescription`).
    pub description: String,
    pub text: String,
    /// `txtWarning`.
    warning: String,
    /// `EnforceSafeFilename`.
    pub safe_filename: bool,
    focused: bool,
}

/// What the user did.
#[derive(Clone, Debug, PartialEq)]
pub enum InputAnswer {
    Ok(String),
    Cancel,
}

/// `Path.GetInvalidFileNameChars()` on Windows (the strictest).
fn invalid_in_file_name(character: char) -> bool {
    matches!(character, '"' | '<' | '>' | '|' | ':' | '*' | '?' | '\\' | '/') || (character as u32) < 32
}

impl InputBox {
    pub fn new(description: &str, text: &str) -> InputBox {
        InputBox {
            description: description.to_string(),
            text: text.to_string(),
            warning: String::new(),
            safe_filename: false,
            focused: false,
        }
    }

    fn text_is_invalid(&self) -> bool {
        self.safe_filename && self.text.chars().any(invalid_in_file_name)
    }

    /// Draws the box; returns the answer once given.
    pub fn show(&mut self, ctx: &egui::Context, id: &str) -> Option<InputAnswer> {
        let mut answer = None;
        let modal = egui::Modal::new(Id::new(("input-box", id)));
        let response = modal.show(ctx, |ui| {
            ui.set_width(400.0);
            let description = RichText::new(self.description.as_str()).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT);
            ui.label(description);
            ui.add_space(2.0);
            let mut field = egui::TextEdit::singleline(&mut self.text);
            field = field.desired_width(f32::INFINITY);
            field = field.font(theme::interface(12.0));
            let field_response = ui.add(field);
            if !self.focused {
                field_response.request_focus();
                self.focused = true;
            }
            if field_response.changed() && self.text_is_invalid() {
                self.warning = "Invalid characters in name.".to_string();
            }
            let submitted = field_response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
            ui.add_space(4.0);
            if !self.warning.is_empty() {
                let warning = RichText::new(self.warning.as_str()).font(theme::interface(11.0)).color(theme::ACCENT_RED);
                ui.label(warning);
            }
            ui.add_space(8.0);
            let mut ok = submitted;
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                if ui.button("Cancel").clicked() {
                    answer = Some(InputAnswer::Cancel);
                }
                let label = RichText::new("OK").color(theme::LABEL_EMPHASIS);
                let mut button = egui::Button::new(label);
                button = button.stroke(Stroke::new(1.0, theme::BUTTON_IMPORTANT_BORDER));
                button = button.min_size(Vec2::new(64.0, 0.0));
                if ui.add(button).clicked() {
                    ok = true;
                }
            });
            // `BtnCreate_OnClick`.
            if ok && answer.is_none() {
                if self.text.is_empty() {
                    self.warning = "Name cannot be empty.".to_string();
                } else if !self.text_is_invalid() {
                    answer = Some(InputAnswer::Ok(self.text.clone()));
                }
            }
        });
        if answer.is_none() && response.should_close() {
            answer = Some(InputAnswer::Cancel);
        }
        answer
    }
}
