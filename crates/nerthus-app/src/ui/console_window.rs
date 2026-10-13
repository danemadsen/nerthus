//! The Console (Tools › Console, F12; `DebugConsole`): a window of text
//! with a prompt (`» `) that runs a few commands, chiefly `printlog`, the
//! session's log so far, and a few jokes.

use egui::{Id, Key, RichText, ScrollArea, TextEdit, Vec2};

use crate::theme;

/// `wpfConsole.PromptText` is `⟫`, which Roboto Mono lacks; its nearest
/// is the guillemet.
const PROMPT: &str = "» ";

/// `wpfConsole.ReturnPrefix`: how replies are indented.
const REPLY_INDENT: &str = "    ";

/// A command the console knows, and what `help` says of it.
const COMMANDS: [(&str, &str); 4] = [
    ("printlog", "Shows the current log"),
    ("copy", "Copies everything in the console"),
    ("clear", "Clears the console"),
    ("help", "Lists the commands"),
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
}

impl ConsoleWindow {
    /// `SplashText`: the name and version, and how to get help.
    pub fn open() -> ConsoleWindow {
        let output = format!("Nerthus {}\nType `help` to get a list of available commands.\n\n", env!("CARGO_PKG_VERSION"));
        ConsoleWindow { output, input: String::new(), history: Vec::new(), history_position: None, scroll_to_end: true }
    }

    fn reply(&mut self, text: &str) {
        for line in text.lines() {
            self.output.push_str(REPLY_INDENT);
            self.output.push_str(line);
            self.output.push('\n');
        }
    }

    /// `Con_CommandExecuted`: the command echoed (`ShowExecutedCommand`),
    /// then run.
    fn run(&mut self, ctx: &egui::Context, typed: &str) {
        let command = typed.trim().to_string();
        self.output.push_str(PROMPT);
        self.output.push_str(&command);
        self.output.push('\n');
        if command.is_empty() {
            return;
        }
        self.history.push(command.clone());
        self.history_position = None;
        match command.to_ascii_lowercase().as_str() {
            "help" => {
                let mut text = String::new();
                for (name, description) in COMMANDS {
                    text.push_str(&format!("{name:<12}{description}\n"));
                }
                self.reply(&text);
            }
            "printlog" => {
                let log = crate::logging::session_text();
                self.output.push_str(&log);
            }
            "copy" => ctx.copy_text(self.output.clone()),
            "clear" | "cls" => self.output.clear(),
            "iddqd" => self.reply("God Mode enabled.\n\n\n\n\n\n\n\nOf course not! What do you think this is? DOOM?"),
            "idkfa" => self.reply("Pew! Pew! Pew!"),
            "idclip" => self.reply("You can run through nodes now."),
            "openpodbaydoors" => self.reply("\n...\n...\n...\nI cannot do that, Dave."),
            "crocodile" => self.output.push_str(CROCODILE),
            other => self.reply(&format!("Unknown command: {other}. Type `help` to get a list of available commands.")),
        }
        self.output.push('\n');
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
    window = window.id(Id::new("debug-console"));
    window = window.collapsible(false);
    window = window.resizable(true);
    window = window.default_size(Vec2::new(800.0, 450.0));
    window = window.open(&mut open);
    window.show(ctx, |ui| {
        let font = theme::monospace(13.0);
        let prompt_height = 26.0;
        let output_height = (ui.available_height() - prompt_height - 8.0).max(80.0);
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
            if response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
                let typed = std::mem::take(&mut console.input);
                console.run(ui.ctx(), &typed);
                console.scroll_to_end = true;
                response.request_focus();
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
