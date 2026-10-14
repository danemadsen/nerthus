//! The Console (Tools › Console, F12; `DebugConsole` on a `wpfConsole`):
//! a window of text with a prompt (`» `) that runs a few commands.
//! `help` (or `list`) lists `uninstall` (a joke), `help`, `list`, `cls`
//! and `clear`; the others are hidden: `printlog` (the session's log so
//! far), `copy`, more jokes (`eject` among them), and `error` and `crash`,
//! which crash Nerthus on purpose so that its crash handling can be tried.

use egui::{Id, Key, RichText, ScrollArea, TextEdit, Vec2};

use crate::theme;
use crate::ui::tool_window;

/// `wpfConsole.PromptText` is `⟫`, which Roboto Mono lacks; its nearest
/// is the guillemet.
const PROMPT: &str = "» ";

/// `wpfConsole.ReturnPrefix`: how replies are indented.
const REPLY_INDENT: &str = "    ";

/// `wpfConsole.InvalidCommand`.
const INVALID_COMMAND: &str = "is not a valid command. Enter 'list' to see applicable commands.";

/// Where the crocodile's prize is.
const CROCODILE_PRIZE: &str = "https://www.youtube.com/watch?v=tup0vsNo2wc";

/// A command the console knows, what `help` says of it, and whether `help`
/// leaves it out (`IsPrivate`).
struct ConsoleCommand {
    name: &'static str,
    description: &'static str,
    private: bool,
}

const fn hidden(name: &'static str, description: &'static str) -> ConsoleCommand {
    ConsoleCommand { name, description, private: true }
}

const fn listed(name: &'static str, description: &'static str) -> ConsoleCommand {
    ConsoleCommand { name, description, private: false }
}

/// The commands in the order `help` lists them: the window's, then
/// `uninstall`, then the console's own (added as it loads).
const COMMANDS: [ConsoleCommand; 15] = [
    hidden("printlog", "Shows the current log"),
    hidden("copy", ""),
    hidden("iddqd", ""),
    hidden("idclip", ""),
    hidden("crocodile", ""),
    hidden("idkfa", ""),
    hidden("eject", ""),
    hidden("error", ""),
    hidden("openpodbaydoors", ""),
    hidden("crash", "Internal use only"),
    listed("uninstall", ""),
    listed("help", "Lists all commands"),
    listed("list", "Lists all commands"),
    listed("cls", "Clears the console."),
    listed("clear", "Clears the console."),
];

/// The window while it is open.
pub struct ConsoleWindow {
    /// Everything shown so far (`txtLog`).
    output: String,
    input: String,
    /// The commands run, newest last, and where the arrow keys are in them.
    history: Vec<String>,
    history_position: Option<usize>,
    scroll_to_end: bool,
    /// How many characters fit on a line of the output, as last drawn.
    line_characters: usize,
}

impl ConsoleWindow {
    pub fn open() -> ConsoleWindow {
        let mut console = ConsoleWindow {
            output: String::new(),
            input: String::new(),
            history: Vec::new(),
            history_position: None,
            scroll_to_end: true,
            line_characters: 80,
        };
        console.append_splash();
        console
    }

    /// `SplashText`: the name and version, and how to get help.
    fn append_splash(&mut self) {
        let splash = format!("Nerthus {}\nType `help` to get a list of available commands.", env!("CARGO_PKG_VERSION"));
        self.append_line(&format!("{splash}\n\n"));
    }

    fn append(&mut self, text: &str) {
        self.output.push_str(text);
    }

    fn append_line(&mut self, text: &str) {
        self.output.push_str(text);
        self.output.push('\n');
    }

    /// `AppendSeparator`: a line of dashes five short of the output's
    /// width.
    fn append_separator(&mut self) {
        let count = self.line_characters.saturating_sub(5);
        self.append_line(&"-".repeat(count));
    }

    /// `ExecuteCommand`: the line (lower-cased) is echoed and run when its
    /// first word is a command; anything else is said not to be one.
    fn run(&mut self, ctx: &egui::Context, typed: &str) {
        let command_line = typed.to_lowercase();
        self.history.push(typed.to_string());
        self.history_position = None;
        let first_word = command_line.split(' ').next().unwrap_or_default().trim().to_string();
        let Some(command) = COMMANDS.iter().find(|command| command.name == first_word) else {
            self.append_line(&format!("{REPLY_INDENT}\"{typed}\" {INVALID_COMMAND}"));
            return;
        };
        self.append_line(&command_line);
        if command_line.contains(" /?") {
            self.append_line(&format!("{REPLY_INDENT}{}", command.description));
        } else {
            match command.name {
                "help" | "list" => self.list_commands(),
                "cls" | "clear" => {
                    self.output.clear();
                    self.append_splash();
                }
                _ => {}
            }
        }
        self.command_executed(ctx, command.name);
        if command_line == "crash" {
            panic!("Test crash");
        }
    }

    /// The commands `help` lists, then a separator.
    fn list_commands(&mut self) {
        let mut text = String::new();
        for command in COMMANDS.iter().filter(|command| !command.private) {
            text.push_str(&format!("{REPLY_INDENT}{}: {}\n", command.name, command.description));
        }
        self.append(&text);
        self.append_separator();
    }

    /// `Con_CommandExecuted`: what the console's own commands do.
    fn command_executed(&mut self, ctx: &egui::Context, name: &str) {
        match name {
            "printlog" => {
                let log = crate::logging::session_text();
                self.append(&log);
            }
            "copy" => ctx.copy_text(self.output.clone()),
            "iddqd" => self.append_line("God Mode enabled.\n\n\n\n\n\n\n\nOf course not! What do you think this is? DOOM?"),
            "idkfa" => self.append_line("Pew! Pew! Pew!"),
            "idclip" => self.append_line("You can run through nodes now."),
            "eject" => self.append_line("Local license removed."),
            "uninstall" => self.append_line("Command not supported in alpha mode."),
            "openpodbaydoors" => self.append_line("\n...\n...\n...\nI cannot do that, Dave."),
            "crocodile" => {
                self.append_line(CROCODILE);
                self.append_line(&format!("Easter egg unlocked. Go to {CROCODILE_PRIZE} to claim your prize."));
                self.append_separator();
            }
            "error" => panic!("Simulated uncaught error!"),
            _ => {}
        }
    }

    /// The arrow keys walk through the commands run.
    fn recall(&mut self, older: bool) {
        if self.history.is_empty() {
            return;
        }
        let last = self.history.len() - 1;
        let position = match (self.history_position, older) {
            (None, true) => last,
            (None, false) => return,
            (Some(position), true) => position.saturating_sub(1),
            (Some(position), false) if position >= last => {
                self.history_position = None;
                self.input.clear();
                return;
            }
            (Some(position), false) => position + 1,
        };
        self.history_position = Some(position);
        self.input = self.history[position].clone();
    }
}

const CROCODILE: &str = "                     _.---._     .---.\n            __...---' .---. `---'-.   `.\n        .-''__.--' _.'( | )`.  `.  `._ :\n      .'__-'_ .--'' ._`---'_.-.  `.   `-`.\n             ~ -._ -._``---. -.    `-._   `.\n                  ~ -.._ _ _ _ ..-_ `.  `-._``--.._\n                               -~ -._  `-.  -. `-._``--.._.--''. \n                                    ~ ~-.__     -._  `-.__   `. `. \n                                          ~~ ~---...__ _    ._ .` `. \n                                                      ~  ~--.....--~ \n\n";

/// The window (`Nerthus Debug Console`, 800 × 450). Returns false once closed.
pub fn show(ctx: &egui::Context, console: &mut ConsoleWindow) -> bool {
    let mut open = true;
    let mut window = egui::Window::new("Nerthus Debug Console");
    window = window.collapsible(false);
    window = window.resizable(true);
    window = tool_window::place(ctx, window, Id::new("debug-console"), Vec2::new(800.0, 450.0));
    window = window.open(&mut open);
    window.show(ctx, |ui| {
        let font = theme::monospace(13.0);
        let prompt_height = 26.0;
        let output_height = (ui.available_height() - prompt_height - 8.0).max(80.0);
        // `CalculateMaxCharsPerLine`: the output's width in W's.
        let glyph_width = ui.fonts_mut(|fonts| fonts.layout_no_wrap("W".to_string(), font.clone(), theme::LABEL_DEFAULT).size().x);
        if glyph_width > 0.0 {
            console.line_characters = (ui.available_width() / glyph_width) as usize;
        }
        let mut scroll = ScrollArea::vertical();
        scroll = scroll.id_salt("console-output");
        scroll = scroll.max_height(output_height);
        scroll = scroll.auto_shrink([false, false]);
        scroll = scroll.stick_to_bottom(true);
        scroll.show(ui, |ui| {
            let mut shown = console.output.as_str();
            let mut text = TextEdit::multiline(&mut shown);
            text = text.font(font.clone());
            text = text.desired_width(f32::INFINITY);
            text = text.frame(egui::Frame::NONE);
            ui.add(text);
            if console.scroll_to_end {
                ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                console.scroll_to_end = false;
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(PROMPT).font(font.clone()).color(theme::ACCENT_MUTED_GREEN));
            let mut field = TextEdit::singleline(&mut console.input);
            field = field.font(font.clone());
            field = field.desired_width(f32::INFINITY);
            field = field.frame(egui::Frame::NONE);
            let response = ui.add(field);
            if !response.has_focus() && !response.lost_focus() {
                response.request_focus();
            }
            // `txtCommand_KeyUp`: Enter runs what was typed, unless it is
            // blank.
            if response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
                let typed = std::mem::take(&mut console.input);
                response.request_focus();
                if !typed.trim().is_empty() {
                    console.scroll_to_end = true;
                    console.run(ui.ctx(), &typed);
                }
            }
            if response.has_focus() {
                if ui.input(|input| input.key_pressed(Key::ArrowUp)) {
                    console.recall(true);
                }
                if ui.input(|input| input.key_pressed(Key::ArrowDown)) {
                    console.recall(false);
                }
            }
        });
    });
    open
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(console: &mut ConsoleWindow, typed: &str) {
        let ctx = egui::Context::default();
        console.run(&ctx, typed);
    }

    #[test]
    fn help_lists_only_the_public_commands() {
        let mut console = ConsoleWindow::open();
        console.output.clear();
        run(&mut console, "HELP");
        let expected = "help\n    uninstall: \n    help: Lists all commands\n    list: Lists all commands\n    cls: Clears the console.\n    clear: Clears the console.\n";
        assert!(console.output.starts_with(expected), "{}", console.output);
        assert!(!console.output.contains("printlog"));
        assert!(!console.output.contains("eject"));
    }

    #[test]
    fn eject_and_uninstall_only_answer() {
        let mut console = ConsoleWindow::open();
        console.output.clear();
        run(&mut console, "eject");
        run(&mut console, "uninstall");
        assert_eq!(console.output, "eject\nLocal license removed.\nuninstall\nCommand not supported in alpha mode.\n");
    }

    #[test]
    fn unknown_commands_are_not_echoed() {
        let mut console = ConsoleWindow::open();
        console.output.clear();
        run(&mut console, "Foo bar");
        assert_eq!(console.output, "    \"Foo bar\" is not a valid command. Enter 'list' to see applicable commands.\n");
    }

    #[test]
    fn the_crocodile_points_to_its_prize() {
        let mut console = ConsoleWindow::open();
        console.line_characters = 12;
        run(&mut console, "crocodile");
        assert!(console.output.contains("Easter egg unlocked. Go to https://www.youtube.com/watch?v=tup0vsNo2wc to claim your prize.\n-------\n"));
    }

    #[test]
    fn clearing_shows_the_splash_again() {
        let mut console = ConsoleWindow::open();
        run(&mut console, "iddqd");
        run(&mut console, "cls");
        assert!(console.output.starts_with("Nerthus "));
        assert!(!console.output.contains("DOOM"));
    }

    #[test]
    #[should_panic(expected = "Simulated uncaught error!")]
    fn error_crashes_on_purpose() {
        let mut console = ConsoleWindow::open();
        run(&mut console, "error");
    }

    #[test]
    #[should_panic(expected = "Test crash")]
    fn crash_crashes_after_running() {
        let mut console = ConsoleWindow::open();
        run(&mut console, "crash");
    }
}
