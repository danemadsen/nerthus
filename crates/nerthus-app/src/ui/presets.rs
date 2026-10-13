//! Node presets in the interface: the Presets menus of the property area
//! and the Node menu (`Shell.LoadNodePresets`) and the Create Preset window
//! (`CreatePresetWindow`).

use egui::{Align, Id, Key, Layout, RichText};

use crate::editor::Editor;
use crate::presets;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::widgets::{dialog_button, menu_item, menu_item_enabled, menu_separator};

/// `LoadNodePresets`: Create Preset, then the presets for the selected
/// node's type (each with its description as the tooltip) or "No presets
/// found...".
pub fn presets_menu(ui: &mut egui::Ui, editor: &Editor, commands: &mut Vec<Command>) {
    let node = editor.active_node().and_then(|id| editor.document.node(id));
    let spec = node.and_then(|node| nerthus_engine::Catalogue::get().node(&node.type_name));
    let node_type = spec.map(|spec| spec.name.as_str()).unwrap_or("none");
    if menu_item(ui, None, None, "Create Preset", "").clicked()
        && let Some(node) = node
    {
        open_create_window(ui.ctx(), node.id);
    }
    let presets = editor.presets.of_type(node_type);
    if presets.is_empty() {
        menu_item_enabled(ui, None, None, "No presets found...", "", false);
        return;
    }
    menu_separator(ui);
    for preset in presets {
        let response = menu_item(ui, None, None, &preset.name, "");
        let response = match &preset.description {
            Some(description) if !description.is_empty() => response.on_hover_text(description),
            _ => response,
        };
        if response.clicked() {
            commands.push(Command::ApplyPreset(preset.name.clone()));
        }
    }
}

/// The Create Preset window while it is open.
#[derive(Clone, Debug, Default)]
struct CreatePreset {
    node: i32,
    name: String,
    description: String,
    /// `chkDefault`.
    default: bool,
    /// `txtWarning`.
    warning: String,
    focused: bool,
}

fn window_id() -> Id {
    Id::new("create-preset")
}

/// `MiCreatePreset_Click`: the window for a node.
pub fn open_create_window(ctx: &egui::Context, node: i32) {
    let state = CreatePreset { node, ..Default::default() };
    ctx.data_mut(|data| data.insert_temp(window_id(), state));
}

/// `CreatePresetWindow` (440 wide): Name, Description (100 high), "Make
/// these the default settings for this type of node." (which names the
/// preset `_default`), the warning line, and Create Preset / Cancel.
pub fn show_create_window(ctx: &egui::Context, editor: &mut Editor) {
    let Some(mut state) = ctx.data(|data| data.get_temp::<CreatePreset>(window_id())) else {
        return;
    };
    let Some(node) = editor.document.node(state.node).cloned() else {
        ctx.data_mut(|data| data.remove::<CreatePreset>(window_id()));
        return;
    };
    let Some(spec) = nerthus_engine::Catalogue::get().node(&node.type_name) else {
        return;
    };
    let mut close = false;
    let mut create = false;
    let modal = egui::Modal::new(window_id().with("modal"));
    let response = modal.show(ctx, |ui| {
        ui.set_width(440.0 - 2.0 * 20.0);
        ui.spacing_mut().item_spacing.y = 2.0;
        let label = |text: &str| RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT);
        ui.label(label("Name"));
        let mut name_field = egui::TextEdit::singleline(&mut state.name);
        name_field = name_field.desired_width(f32::INFINITY);
        name_field = name_field.font(theme::interface(12.0));
        name_field = name_field.hint_text("Enter the name of the preset");
        let name_response = ui.add(name_field);
        if !state.focused {
            name_response.request_focus();
            state.focused = true;
        }
        if name_response.changed() {
            state.warning.clear();
        }
        ui.add_space(6.0);
        ui.label(label("Description"));
        let mut description_field = egui::TextEdit::multiline(&mut state.description);
        description_field = description_field.desired_width(f32::INFINITY);
        description_field = description_field.desired_rows(6);
        description_field = description_field.font(theme::interface(12.0));
        description_field = description_field.hint_text("Enter the description for preset");
        ui.add(description_field);
        ui.add_space(8.0);
        let checkbox_text = label("Make these the default settings for this type of node.");
        if ui.checkbox(&mut state.default, checkbox_text).changed() && state.default {
            // `ChkDefault_OnChecked` (there is no unchecked handler).
            state.name = "_default".to_string();
            state.warning = format!("This will apply to all [{}] nodes. Delete the \"_default.preset\" file in the presets folder to remove these defaults.", spec.name);
        }
        ui.add_space(4.0);
        if !state.warning.is_empty() {
            let warning = RichText::new(state.warning.as_str()).font(theme::interface(12.0)).color(theme::ACCENT_RED);
            ui.add(egui::Label::new(warning).wrap());
        }
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if dialog_button(ui, "Cancel", false).clicked() {
                close = true;
            }
            ui.add_space(8.0);
            if dialog_button(ui, "Create Preset", true).clicked() {
                create = true;
            }
        });
        let entered = ui.input(|input| input.key_pressed(Key::Enter)) && !ui.memory(|memory| memory.focused().is_some_and(|focused| focused != name_response.id));
        if entered {
            create = true;
        }
    });
    if response.should_close() {
        close = true;
    }
    if create {
        // `BtnCreate_OnClick`.
        if state.name.is_empty() {
            state.warning = "Preset Name cannot be empty.".to_string();
        } else {
            let label = make_safe(&state.name);
            let description = Some(state.description.as_str());
            let mut preset = presets::create_preset(&node, spec, &state.name, description);
            if let Err(error) = editor.presets.save(&mut preset, &label) {
                editor.report(format!("Failed to create Preset. {error}"));
            }
            close = true;
        }
    }
    if close {
        ctx.data_mut(|data| data.remove::<CreatePreset>(window_id()));
    } else {
        ctx.data_mut(|data| data.insert_temp(window_id(), state));
    }
}

/// `CreatePresetWindow.MakeSafe`: characters a (Windows) file name cannot
/// hold become underscores.
fn make_safe(name: &str) -> String {
    let unsafe_character = |character: char| matches!(character, '"' | '<' | '>' | '|' | ':' | '*' | '?' | '\\' | '/') || (character as u32) < 32;
    name.chars().map(|character| if unsafe_character(character) { '_' } else { character }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_characters_become_underscores() {
        assert_eq!(make_safe("Rough: v2/3?"), "Rough_ v2_3_");
    }
}
