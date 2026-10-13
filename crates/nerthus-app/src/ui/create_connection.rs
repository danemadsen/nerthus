//! The Create Connection window (`CreateConnectionWindow`): a connection
//! drawn from one compact node's collapsed ports onto another's, its two
//! ends picked from lists of every output and every input.

use egui::{Align, Color32, Id, Key, Layout, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};

use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::graph_canvas::elided_galley;
use crate::ui::graph_commands::GraphCommand;
use crate::ui::widgets::{dialog_button, DIALOG_BUTTON_HEIGHT};

/// The window's `Border`: the window's 440 less the 20 its shadow takes
/// each side, and what lies inside its line (`Padding` 1 and the panel's
/// `Margin` 12).
const BORDER_WIDTH: f32 = 440.0 - 2.0 * 20.0;
const BORDER_INSIDE: f32 = 1.0 + 12.0;
/// The buttons' `StackPanel` margin.
const BUTTONS_MARGIN: f32 = 8.0;
/// `ListBoxItem-Port`: the text's padding, and the `port-connect` icon
/// before it.
const ROW_PADDING: Vec2 = Vec2::new(8.0, 4.0);
const ICON_SIZE: f32 = 16.0;
/// The margin inside each list (the `ListBox` template's `StackPanel`),
/// and each list's margin on the side facing the other.
const LIST_MARGIN: f32 = 4.0;

/// The window while it is open: the node whose outputs are listed, the
/// node whose inputs are, and the two ports picked.
#[derive(Clone, Debug, PartialEq)]
pub struct CreateConnection {
    source: i32,
    target: i32,
    output: Option<String>,
    input: Option<String>,
}

impl CreateConnection {
    /// `CreateConnectionWindow_Loaded`: pulled out of the outputs' side,
    /// the dragged node's outputs and the other node's inputs are listed;
    /// out of the inputs' side, the other way round (`IsReverse`).
    pub fn new(dragged: i32, other: i32, from_output: bool) -> Self {
        let (source, target) = if from_output { (dragged, other) } else { (other, dragged) };
        Self { source, target, output: None, input: None }
    }
}

/// A port as listed: its name, and whether it is connected (`Tag`
/// "connected").
struct ListedPort {
    name: String,
    connected: bool,
}

/// A node's outputs, or its inputs, in port order.
fn listed_ports(editor: &Editor, node: i32, outputs: bool) -> Vec<ListedPort> {
    let Some(data) = editor.document.node(node) else {
        return Vec::new();
    };
    let mut listed = Vec::new();
    for port in &data.ports {
        let on_side = if outputs { port.is_output() } else { port.is_input() };
        if !on_side {
            continue;
        }
        let connected = editor.document.port_connected(node, port);
        listed.push(ListedPort { name: port.name.clone(), connected });
    }
    listed
}

/// The window (440 wide): "From <node>:" over that node's outputs, "To
/// <node>:" over the other's inputs, then Create Connection and Cancel.
/// Create Connection (or Enter) joins the two picked, as a connection
/// made on the graph does; with either list unpicked it does nothing.
pub fn show(ctx: &egui::Context, editor: &Editor, window: &mut Option<CreateConnection>, commands: &mut Vec<Command>) {
    let Some(state) = window.as_mut() else {
        return;
    };
    let (Some(source), Some(target)) = (editor.document.node(state.source), editor.document.node(state.target)) else {
        *window = None;
        return;
    };
    let from_label = format!("From {}:", source.name);
    let to_label = format!("To {}:", target.name);
    let outputs = listed_ports(editor, state.source, true);
    let inputs = listed_ports(editor, state.target, false);
    let rows = outputs.len().max(inputs.len());
    let mut close = false;
    let mut create = false;
    let mut frame = egui::Frame::new();
    frame = frame.fill(theme::MENU_BACKGROUND);
    frame = frame.stroke(Stroke::new(1.0, theme::MENU_BORDER));
    frame = frame.corner_radius(6.0);
    frame = frame.inner_margin(BORDER_INSIDE);
    // `DropShadowEffect`: blur 20, depth 5 straight down, half opaque.
    let shadow = egui::Shadow { offset: [0, 5], blur: 20, spread: 0, color: Color32::from_black_alpha(128) };
    frame = frame.shadow(shadow);
    let mut modal = egui::Modal::new(Id::new("create-connection"));
    modal = modal.frame(frame);
    let response = modal.show(ctx, |ui| {
        ui.set_width(BORDER_WIDTH - 2.0 * (1.0 + BORDER_INSIDE));
        // The two lists' facing margins part them.
        ui.spacing_mut().item_spacing = Vec2::new(2.0 * LIST_MARGIN, 0.0);
        let label = |text: &str| RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT);
        ui.columns(2, |columns| {
            columns[0].label(label(&from_label));
            columns[1].label(label(&to_label));
        });
        ui.add_space(8.0);
        ui.columns(2, |columns| {
            port_list(&mut columns[0], &outputs, rows, &mut state.output);
            port_list(&mut columns[1], &inputs, rows, &mut state.input);
        });
        ui.add_space(8.0 + BUTTONS_MARGIN);
        // A row of the buttons' height: a layout given the rest of the
        // window would take it all.
        let buttons_row = Vec2::new(ui.available_width(), DIALOG_BUTTON_HEIGHT);
        ui.allocate_ui_with_layout(buttons_row, Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(BUTTONS_MARGIN);
            if dialog_button(ui, "Cancel", false).clicked() {
                close = true;
            }
            ui.add_space(8.0);
            if dialog_button(ui, "Create Connection", true).clicked() {
                create = true;
            }
        });
        ui.add_space(BUTTONS_MARGIN);
        if ui.input(|input| input.key_pressed(Key::Enter)) {
            create = true;
        }
    });
    if response.should_close() {
        close = true;
    }
    // `BtnCreate_OnClick`.
    if create && let (Some(output), Some(input)) = (&state.output, &state.input) {
        let connect = GraphCommand::ConnectPorts { from: state.source, from_port: output.clone(), to: state.target, to_port: input.clone() };
        commands.push(Command::Graph(connect));
        close = true;
    }
    if close {
        *window = None;
    }
}

/// A `ListBox` of ports, as tall as `rows` of them (the two lists stand
/// side by side in a `UniformGrid`); a click picks one.
fn port_list(ui: &mut Ui, ports: &[ListedPort], rows: usize, picked: &mut Option<String>) {
    let mut frame = egui::Frame::new();
    frame = frame.fill(theme::LISTBOX_BACKGROUND);
    frame = frame.inner_margin(LIST_MARGIN);
    frame.show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let height = row_height(ui);
        ui.set_min_height(rows as f32 * height);
        for port in ports {
            let chosen = picked.as_deref() == Some(port.name.as_str());
            if port_row(ui, port, chosen, height) {
                *picked = Some(port.name.clone());
            }
        }
    });
}

/// A row's height: the text and its padding, or the icon if taller.
fn row_height(ui: &Ui) -> f32 {
    let galley = ui.painter().layout_no_wrap("Ag".to_string(), theme::interface(11.0), theme::OPTIONS_LABEL);
    (galley.size().y + 2.0 * ROW_PADDING.y).max(ICON_SIZE)
}

/// `ListBoxItem-Port`: the green `port-connect` when the port is
/// connected (its room kept when not), then the name; dimmed under the
/// pointer and filled when picked. Returns whether it was clicked.
fn port_row(ui: &mut Ui, port: &ListedPort, chosen: bool, height: f32) -> bool {
    let size = Vec2::new(ui.available_width(), height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = response.on_hover_text(port.name.as_str());
    let hovered = response.hovered();
    if chosen {
        ui.painter().rect_filled(rect, 2.0, theme::OPTIONS_SELECTION);
    } else if hovered {
        ui.painter().rect_filled(rect, 2.0, theme::OPTIONS_SELECTION.gamma_multiply(0.4));
    }
    if port.connected {
        let icon_center = Pos2::new(rect.left() + ICON_SIZE / 2.0, rect.center().y);
        let icon_rect = Rect::from_center_size(icon_center, Vec2::splat(ICON_SIZE));
        icons::paint(ui, icon_rect, "port-connect", theme::ACCENT_GREEN);
    }
    let color = if chosen || hovered { theme::LABEL_EMPHASIS } else { theme::OPTIONS_LABEL };
    let text_left = rect.left() + ICON_SIZE + ROW_PADDING.x;
    let text_width = (rect.right() - ROW_PADDING.x - text_left).max(1.0);
    let galley = elided_galley(ui.painter(), &port.name, theme::interface(11.0), color, text_width);
    let text_position = Pos2::new(text_left, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_position, galley, color);
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lists_follow_the_side_dragged_from() {
        let window = CreateConnection::new(1, 2, true);
        assert_eq!((window.source, window.target), (1, 2));
        let reverse = CreateConnection::new(1, 2, false);
        assert_eq!((reverse.source, reverse.target), (2, 1));
    }

    /// A frame of the window, with a key pressed; the commands it queued.
    fn frame(context: &egui::Context, editor: &Editor, window: &mut Option<CreateConnection>, key: Key) -> Vec<Command> {
        let press = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0));
        let input = egui::RawInput { screen_rect: Some(screen), events: vec![press], ..Default::default() };
        let mut commands = Vec::new();
        let output = context.run_ui(input, |ui| show(ui.ctx(), editor, window, &mut commands));
        output.drop_without_applying_deltas();
        commands
    }

    #[test]
    fn enter_connects_the_two_picked_and_escape_cancels() {
        let mut document = crate::document::Document::new();
        let erosion = document.add_node("ErosionPro", [0.0, 0.0]).unwrap();
        let combine = document.add_node("Combine", [300.0, 0.0]).unwrap();
        let editor = Editor::new(document);
        let context = egui::Context::default();
        let mut window = Some(CreateConnection::new(erosion, combine, true));
        // With one list unpicked, Create Connection does nothing.
        window.as_mut().unwrap().output = Some("Wear".to_string());
        assert!(frame(&context, &editor, &mut window, Key::Enter).is_empty());
        assert!(window.is_some());
        window.as_mut().unwrap().input = Some("Input2".to_string());
        let commands = frame(&context, &editor, &mut window, Key::Enter);
        let connect = GraphCommand::ConnectPorts { from: erosion, from_port: "Wear".to_string(), to: combine, to_port: "Input2".to_string() };
        assert!(matches!(commands.as_slice(), [Command::Graph(made)] if *made == connect));
        assert!(window.is_none());

        let mut window = Some(CreateConnection::new(erosion, combine, true));
        assert!(frame(&context, &editor, &mut window, Key::Escape).is_empty());
        assert!(window.is_none());
    }

    #[test]
    fn every_port_on_a_side_is_listed() {
        let mut document = crate::document::Document::new();
        let erosion = document.add_node("ErosionPro", [0.0, 0.0]).unwrap();
        let combine = document.add_node("Combine", [300.0, 0.0]).unwrap();
        document.connect(erosion, "Wear", combine, "Input2");
        let editor = Editor::new(document);
        let describe = |ports: Vec<ListedPort>| ports.into_iter().map(|port| (port.name, port.connected)).collect::<Vec<_>>();
        let outputs = describe(listed_ports(&editor, erosion, true));
        let expected_outputs = [("Out", false), ("Flow", false), ("Wear", true), ("Deposits", false)];
        assert_eq!(outputs, expected_outputs.map(|(name, connected)| (name.to_string(), connected)));
        // The inputs, the mask's included.
        let inputs = describe(listed_ports(&editor, combine, false));
        let expected_inputs = [("In", false), ("Input2", true), ("Mask", false)];
        assert_eq!(inputs, expected_inputs.map(|(name, connected)| (name.to_string(), connected)));
    }
}
