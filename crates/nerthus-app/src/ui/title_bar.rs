//! The window's top bar (`Shell` `MenuBar`, 6 down): the brand, the main
//! menu, the file box (preview resolution and region, file name, save
//! buttons, autosave), the undo history, the version and the window
//! buttons. Empty space drags the window.

use egui::containers::menu::MenuButton;
use egui::{Align, Color32, Layout, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2};

use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::menus;
use crate::ui::tool_commands::ToolCommand;
use crate::files::{FileCommand, AUTOSAVE_CHOICES};
use crate::ui::widgets::{checked_icon_item, icon_button_sized, icon_menu, menu_caption, menu_item, menu_item_enabled, menu_separator, menu_style};
use crate::ui::UiState;

pub const HEIGHT: f32 = 36.0;
/// `MenuBar`'s top margin.
const BAR_TOP: f32 = 6.0;
/// A top-level menu item: at least 64 wide, overlapping its neighbours by
/// 5 (`Margin -5,0,-5,0`), its text 8 + 5 in each side.
const TOP_LEVEL_MIN_WIDTH: f32 = 64.0;
const TOP_LEVEL_OVERLAP: f32 = 5.0;
const TOP_LEVEL_HEIGHT: f32 = 22.0;

pub fn show(ui: &mut Ui, rect: Rect, editor: &Editor, ui_state: &UiState, commands: &mut Vec<Command>) {
    ui.painter().rect_filled(rect, 0.0, theme::WINDOW_BACKGROUND);

    // Behind everything: a press on empty space drags the window.
    let drag_response = ui.interact(rect, ui.id().with("title-drag"), Sense::click_and_drag());
    if drag_response.drag_started() {
        commands.push(Command::WindowDrag);
    }
    if drag_response.double_clicked() {
        commands.push(Command::WindowMaximize);
    }

    let content = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + BAR_TOP), rect.max);
    let builder = UiBuilder::new().max_rect(content).layout(Layout::left_to_right(Align::Center));
    let mut bar = ui.new_child(builder);
    bar.spacing_mut().item_spacing.x = 0.0;

    brand(&mut bar);
    menu_bar(&mut bar, editor, ui_state, commands);
    // `Menu` margin 0, 0, 12, 0 and the file box's own margin 6.
    bar.add_space(12.0 + 6.0);
    file_box(&mut bar, editor, commands);
    bar.add_space(6.0 + 5.0);
    undo_menu(&mut bar, editor, commands);

    bar.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        // The browser's tab is the web build's window.
        if !cfg!(target_arch = "wasm32") {
            window_buttons(ui, commands);
            ui.add_space(24.0);
        }
        // `bAbout` (About Nerthus), and `bUpdate` once a newer version is
        // known.
        if version_button(ui, concat!("Nerthus ", env!("CARGO_PKG_VERSION")), theme::LABEL_DEFAULT, true).clicked() {
            commands.push(Command::Tools(ToolCommand::ShowAbout));
        }
        // While the update downloads, and after, the button counts it and
        // can't be clicked.
        if let Some(label) = ui_state.updates.download_label() {
            ui.add_space(8.0);
            version_button(ui, &label, theme::LABEL_DEFAULT, false);
        } else if ui_state.updates.update_available() {
            ui.add_space(8.0);
            if version_button(ui, "Update", theme::ACCENT_PRIMARY, true).clicked() {
                commands.push(Command::Tools(ToolCommand::CheckForUpdates));
            }
        }
        ui.add_space(24.0);
    });
}

/// The name button (`Button-Link`, padding 16, 6): the name in the header font.
fn brand(ui: &mut Ui) {
    let galley = ui.painter().layout_no_wrap("NERTHUS".to_string(), theme::semibold(16.0), theme::ACCENT_PRIMARY);
    let size = galley.size() + Vec2::new(32.0, 12.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let position = Pos2::new(rect.left() + 16.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(position, galley, theme::ACCENT_PRIMARY);
}

const MENU_NAMES: [&str; 8] = ["File", "Edit", "Graph", "Node", "Preview", "Project", "Tools", "Help"];

/// A top-level item's slot: at least 64 wide less its overlaps, its text
/// 8 + 5 in from each side.
fn menu_slot(ui: &Ui, name: &str) -> Vec2 {
    let text_width = ui.painter().layout_no_wrap(name.to_string(), theme::interface(11.0), Color32::WHITE).size().x;
    let item_width = (text_width + 2.0 * (8.0 + 5.0)).max(TOP_LEVEL_MIN_WIDTH);
    Vec2::new(item_width - 2.0 * TOP_LEVEL_OVERLAP, TOP_LEVEL_HEIGHT)
}

/// The main menu: each top-level item takes its slot, its text centred,
/// `Menu-TopLevel` under the pointer. `MenuBar` fills what it is given, so
/// it gets just its items' width.
fn menu_bar(ui: &mut Ui, editor: &Editor, ui_state: &UiState, commands: &mut Vec<Command>) {
    let width: f32 = MENU_NAMES.iter().map(|name| menu_slot(ui, name).x).sum();
    let size = Vec2::new(width, TOP_LEVEL_HEIGHT);
    ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| menu_items(ui, editor, ui_state, commands));
}

fn menu_items(ui: &mut Ui, editor: &Editor, ui_state: &UiState, commands: &mut Vec<Command>) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.spacing_mut().button_padding = Vec2::ZERO;
        ui.visuals_mut().widgets.hovered.weak_bg_fill = theme::MENU_BACKGROUND_HOVER;
        ui.visuals_mut().widgets.open.weak_bg_fill = theme::MENU_BACKGROUND_HOVER;
        ui.visuals_mut().widgets.hovered.bg_stroke = Stroke::NONE;
        ui.visuals_mut().widgets.hovered.corner_radius = egui::CornerRadius::same(4);
        ui.visuals_mut().widgets.open.corner_radius = egui::CornerRadius::same(4);
        for name in MENU_NAMES {
            let text = RichText::new(name).font(theme::interface(11.0)).color(theme::MENU_FOREGROUND);
            let slot = menu_slot(ui, name);
            let mut button = egui::Button::new(text);
            button = button.min_size(slot);
            button = button.frame_when_inactive(false);
            let menu = MenuButton::from_button(button);
            menu.ui(ui, |ui| {
                menu_style(ui);
                match name {
                    "File" => menus::file_menu(ui, editor, commands),
                    "Edit" => menus::edit_menu(ui, editor, commands),
                    "Graph" => menus::graph_menu(ui, editor, ui_state, commands),
                    "Node" => menus::node_menu(ui, editor, commands),
                    "Preview" => menus::preview_menu(ui, editor, commands),
                    "Project" => menus::project_menu(ui, commands),
                    "Tools" => menus::tools_menu(ui, editor, commands),
                    _ => menus::help_menu(ui, commands),
                }
            });
        }
    });
}

/// The file box (`MinWidth 180`, padding 1, `Panel-Background`, corners
/// 3): the resolution and region menus, the file name (monospace 11 bold,
/// margin 8, 4), save, save incremental and the autosave menu.
fn file_box(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    let mut frame = egui::Frame::new();
    frame.fill = theme::PANEL_BACKGROUND;
    frame.corner_radius = egui::CornerRadius::same(3);
    frame.inner_margin = egui::Margin::same(1);
    frame.show(ui, |ui| {
        ui.set_min_width(180.0 - 2.0);
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            crate::ui::viewport_area::resolution_menu(ui, editor, commands);
            // `miRegionsPreview`, once the terrain has regions.
            if crate::ui::regions::menus_visible(editor) {
                let tooltip = crate::ui::regions::preview_tooltip(editor);
                icon_menu(ui, "artboard", &tooltip, Some(theme::ACCENT_GREEN), |ui| {
                    crate::ui::regions::regions_menu(ui, editor, commands, true);
                });
            }
            let name = editor.document.file_name();
            // `TryAutosave` turns the name red once the autosave interval
            // passed with unsaved changes; a save turns it back.
            let name_color = if editor.files.autosave.reminder { theme::ACCENT_MUTED_RED } else { theme::LABEL_DEFAULT };
            let galley = ui.painter().layout_no_wrap(name, theme::monospace_bold(11.0), name_color);
            let name_width = galley.size().x.max(140.0);
            let (name_rect, _) = ui.allocate_exact_size(Vec2::new(name_width + 16.0, galley.size().y + 8.0), Sense::hover());
            ui.painter().galley(Pos2::new(name_rect.left() + 8.0, name_rect.center().y - galley.size().y / 2.0), galley, name_color);
            // `Button-IconOnly` (14 + padding 6), margin 0, 0, 2, 0.
            let button_size = Vec2::splat(14.0 + 12.0);
            if icon_button_sized(ui, "floppy", "Save", None, button_size, 14.0).clicked() {
                commands.push(Command::Save);
            }
            ui.add_space(2.0);
            if icon_button_sized(ui, "copy-plus", "Save Incremental", None, button_size, 14.0).clicked() {
                commands.push(Command::File(FileCommand::SaveIncremental));
            }
            ui.add_space(2.0);
            icon_menu(ui, "clock-rotate-right", "Autosave", None, |ui| autosave_menu(ui, editor, commands));
        });
    });
}

/// The Autosave menu: the interval (`SetAutosaveInterval`; the one in use
/// checked), Reset Timer, this session's latest five autosaves to revert
/// to (`mnuAutosaveEntry1`…`5`, "Autosave Entry" and disabled when there
/// are fewer), Open Autosave… and Revert to Saved.
fn autosave_menu(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    menu_style(ui);
    menu_caption(ui, "Remind me to save every");
    for (minutes, text, icon) in AUTOSAVE_CHOICES {
        let checked = editor.files.autosave.is_interval(minutes);
        if checked_icon_item(ui, checked, icon, None, text, "") {
            commands.push(Command::File(FileCommand::SetAutosaveInterval(minutes)));
        }
    }
    if menu_item(ui, Some("time-duration-off"), Some(theme::ACCENT_MUTED_RED), "Reset Timer", "").clicked() {
        commands.push(Command::File(FileCommand::ResetAutosaveTimer));
    }
    menu_separator(ui);
    menu_caption(ui, "Revert to");
    let entries = editor.files.autosave.menu_entries();
    for slot in 0..5 {
        let icon = format!("number-{}-small", slot + 1);
        match entries.get(slot) {
            Some((label, path)) => {
                if menu_item(ui, Some(&icon), None, label, "").clicked() {
                    commands.push(Command::File(FileCommand::OpenAutosaveEntry(path.clone())));
                }
            }
            None => {
                menu_item_enabled(ui, Some(&icon), None, "Autosave Entry", "", false);
            }
        }
    }
    menu_separator(ui);
    if menu_item(ui, Some("folder"), Some(theme::ACCENT_MUTED_GREEN), "Open Autosave...", "").clicked() {
        commands.push(Command::File(FileCommand::OpenAutosave));
    }
    menu_separator(ui);
    if menu_item(ui, Some("restore"), Some(theme::ACCENT_MUTED_RED), "Revert to Saved", "").clicked() {
        commands.push(Command::File(FileCommand::RevertToSaved));
    }
}

/// The undo button: its menu lists the edits that can be undone.
fn undo_menu(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    icon_menu(ui, "arrow-back-up", "Undo", None, |ui| {
        menu_style(ui);
        let labels = editor.document.undo_labels();
        if labels.is_empty() {
            menu_caption(ui, "No undo history");
            return;
        }
        for (steps, label) in labels.iter().enumerate().take(20) {
            if menu_item(ui, None, None, label, "").clicked() {
                for _ in 0..=steps {
                    commands.push(Command::Undo);
                }
            }
        }
    });
}

/// `bAbout` and `bUpdate`: plain buttons (`Button`: padding 24, 6, at
/// least 87 wide) in 10px semibold.
fn version_button(ui: &mut Ui, text: &str, color: Color32, enabled: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::semibold(10.0), color);
    let width = (galley.size().x + 48.0).max(87.0);
    let height = galley.size().y + 12.0;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), sense);
    let fill = if !enabled {
        theme::BUTTON_BACKGROUND
    } else if response.is_pointer_button_down_on() {
        theme::BUTTON_PRESSED
    } else if response.hovered() {
        theme::BUTTON_HOVER
    } else {
        theme::BUTTON_BACKGROUND
    };
    ui.painter().rect_filled(rect, 2.0, fill);
    ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, theme::BUTTON_BORDER), StrokeKind::Inside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    response
}

/// Minimize, maximize and close (`Button-System`: Marlett's `0`, `1` and
/// `r` with padding 10 and margins 2, at 40% until hovered; close red).
fn window_buttons(ui: &mut Ui, commands: &mut Vec<Command>) {
    ui.add_space(2.0);
    if system_button(ui, SystemGlyph::Close).clicked() {
        commands.push(Command::Exit);
    }
    ui.add_space(4.0);
    if system_button(ui, SystemGlyph::Maximize).clicked() {
        commands.push(Command::WindowMaximize);
    }
    ui.add_space(4.0);
    if system_button(ui, SystemGlyph::Minimize).clicked() {
        commands.push(Command::WindowMinimize);
    }
    ui.add_space(2.0);
}

#[derive(Clone, Copy, PartialEq)]
enum SystemGlyph {
    Minimize,
    Maximize,
    Close,
}

fn system_button(ui: &mut Ui, glyph: SystemGlyph) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(29.0, 29.0), Sense::click());
    let hovered = response.hovered();
    let base = match glyph {
        SystemGlyph::Close => theme::ACCENT_RED,
        _ if hovered => theme::LABEL_EMPHASIS,
        _ => theme::LABEL_DEFAULT,
    };
    let color = if hovered { base } else { base.gamma_multiply(0.4) };
    let center = rect.center();
    let painter = ui.painter();
    match glyph {
        SystemGlyph::Minimize => {
            let bar = Rect::from_center_size(center + Vec2::new(0.0, 3.0), Vec2::new(8.0, 2.0));
            painter.rect_filled(bar, 0.0, color);
        }
        SystemGlyph::Maximize => {
            let frame = Rect::from_center_size(center, Vec2::splat(9.0));
            painter.rect_stroke(frame, 0.0, Stroke::new(1.0, color), StrokeKind::Inside);
            let top = Rect::from_min_size(frame.min, Vec2::new(frame.width(), 2.0));
            painter.rect_filled(top, 0.0, color);
        }
        SystemGlyph::Close => {
            let stroke = Stroke::new(1.5, color);
            let half = 4.0;
            painter.line_segment([center + Vec2::new(-half, -half), center + Vec2::new(half, half)], stroke);
            painter.line_segment([center + Vec2::new(-half, half), center + Vec2::new(half, -half)], stroke);
        }
    }
    let _ = icons::DEFAULT_SIZE;
    response
}
