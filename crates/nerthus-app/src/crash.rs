//! What happens when the editor panics while it draws or runs a command
//! (`App.Current_DispatcherUnhandledException`). By the time the panic is
//! caught, the panic hooks have logged it and written the crash logs and
//! the disaster recovery copy of the project. A crash while Nerthus was
//! still starting first asks whether to diagnose it, which opens the
//! troubleshooting page; then Nerthus asks whether to try and continue.
//! Yes goes on with a new log file; No starts Nerthus again with
//! Diagnostics open and lets the panic end this session.

use std::any::Any;

use egui::{Align, Layout, RichText};

use crate::theme;

/// How long the editor has to draw before it counts as started: the
/// viewport's launch and the timer that settles it.
const STARTUP_SECONDS: f64 = 3.0;

const DIAGNOSE_CAPTION: &str = "Diagnose Nerthus Crash?";
const DIAGNOSE_TEXT: &str = "Nerthus failed during startup. Diagnose this issue?\n\nIf this is the first time you are seeing this, please try starting Nerthus again. If this crash happens frequently, please click YES to diagnose this issue.";
const CONTINUE_CAPTION: &str = "Crash";
const CONTINUE_TEXT: &str = "A major error occurred. Would you like Nerthus to try and continue?\n\nYou should save your work (preferably as a new file) and restart Nerthus to ensure stability.";

/// Whether Nerthus is still starting (`O.IsStarting`).
#[derive(Default)]
pub struct Startup {
    first_frame: Option<f64>,
    started: bool,
}

impl Startup {
    /// A frame drawn without a crash, at this time on the interface's
    /// clock.
    pub fn note_frame(&mut self, now: f64) {
        let first_frame = *self.first_frame.get_or_insert(now);
        if now - first_frame >= STARTUP_SECONDS {
            self.started = true;
        }
    }

    pub fn starting(&self) -> bool {
        !self.started
    }
}

/// A caught panic, until its questions are answered.
pub struct Crash {
    message: String,
    /// "Diagnose this issue?" still waits for an answer.
    diagnose: bool,
}

impl Crash {
    pub fn caught(payload: &(dyn Any + Send), starting: bool) -> Crash {
        let message = panic_message(payload);
        Crash { message, diagnose: starting }
    }
}

/// The text a panic was raised with.
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        return (*text).to_string();
    }
    if let Some(text) = payload.downcast_ref::<String>() {
        return text.clone();
    }
    "Unknown panic".to_string()
}

/// The question waiting, in a message box of its own. Escape and clicks
/// outside leave it open: it has no Cancel.
pub fn show(ctx: &egui::Context, crash: &mut Option<Crash>) {
    let Some(waiting) = crash.as_mut() else {
        return;
    };
    let (caption, text) = if waiting.diagnose { (DIAGNOSE_CAPTION, DIAGNOSE_TEXT) } else { (CONTINUE_CAPTION, CONTINUE_TEXT) };
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("crash-message-box"));
    modal.show(ctx, |ui| {
        ui.set_width(420.0);
        ui.label(RichText::new(caption).font(theme::semibold(13.0)).color(theme::LABEL_EMPHASIS));
        ui.add_space(8.0);
        ui.label(RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT));
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("No").clicked() {
                answer = Some(false);
            }
            if ui.button("Yes").clicked() {
                answer = Some(true);
            }
        });
    });
    let Some(yes) = answer else {
        return;
    };
    if waiting.diagnose {
        waiting.diagnose = false;
        if yes {
            crate::docs::open_in_browser(crate::docs::STARTUP_TROUBLESHOOTING);
        }
        return;
    }
    if yes {
        *crash = None;
        crate::logging::resume_after_crash();
        return;
    }
    let message = std::mem::take(&mut waiting.message);
    start_diagnostics();
    // Not handled: the panic goes on and ends the session.
    std::panic::resume_unwind(Box::new(message));
}

/// Nerthus started again with Diagnostics open (`diagnostics.exe`).
fn start_diagnostics() {
    let program = match std::env::current_exe() {
        Ok(program) => program,
        Err(error) => {
            log::error!("Cannot start Diagnostics: {error}");
            return;
        }
    };
    let mut command = std::process::Command::new(program);
    command.arg("--diagnostics");
    if let Err(error) = command.spawn() {
        log::error!("Cannot start Diagnostics: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_ends_after_the_settling_time() {
        let mut startup = Startup::default();
        startup.note_frame(10.0);
        assert!(startup.starting());
        startup.note_frame(12.9);
        assert!(startup.starting());
        startup.note_frame(13.0);
        assert!(!startup.starting());
    }

    #[test]
    fn panic_messages_come_from_either_string() {
        let literal: Box<dyn Any + Send> = Box::new("Simulated uncaught error!");
        assert_eq!(panic_message(literal.as_ref()), "Simulated uncaught error!");
        let formatted: Box<dyn Any + Send> = Box::new(format!("Test {}", "crash"));
        assert_eq!(panic_message(formatted.as_ref()), "Test crash");
    }
}
