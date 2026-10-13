//! Groups on the graph (`GroupElement`): a box tinted with the group's
//! swatch, its header (grabbed to move the group and its nodes, double-
//! clicked to collapse it to a 60 square), its icon, name and menu, the
//! lock, and the invisible resize thumbs in the bottom corners.

use egui::{CornerRadius, CursorIcon, Id, PointerButton, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::document::{GroupData, GROUP_ICONS, GROUP_MIN_SIZE};
use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::graph_canvas::{elided_galley, GraphState, SIMPLIFIED_ZOOM};
use crate::ui::graph_commands::GraphCommand;
use crate::ui::graph_popups::{icon_grid, swatch_grid};
use crate::ui::widgets::{check_item, menu_caption, menu_item, menu_separator, menu_style};

/// `PART_Header`: the strip along the top a group is grabbed by (all of a
/// collapsed group).
const HEADER_HEIGHT: f32 = 30.0;
/// `PART_Thumb` and `PART_Thumb_Left`.
const THUMB: f32 = 20.0;
/// `PART_GroupMenu` (margin 8) and its 14 `menu` icon.
const MENU_BUTTON: Vec2 = Vec2::new(24.0, 20.5);
const MENU_MARGIN: f32 = 8.0;
/// `IsSmallSize`: narrower than this, the menu button goes.
const SMALL_WIDTH: f32 = 75.0;
/// `ElementsPadding`: the room a resized group keeps past its children.
const ELEMENTS_PADDING: f32 = 20.0;

/// `GroupState` while an element is dragged: it would join the group
/// (`Accent-Muted-Green`) or leave it (`Accent-Muted-Orange`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupState {
    Incoming,
    Outgoing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GroupDragKind {
    Move,
    /// `PART_Thumb` (bottom right) and `PART_Thumb_Left` (bottom left).
    ResizeRight,
    ResizeLeft,
}

/// A group being moved or resized: where the pointer and the group
/// started, and the box its children take.
#[derive(Clone, Copy, Debug)]
pub struct GroupDrag {
    id: i32,
    kind: GroupDragKind,
    pointer_start: Pos2,
    position: [f32; 2],
    size: [f32; 2],
    children: Option<Rect>,
}

/// `GetGroupBounds`: the box the group's nodes and notes take.
fn children_bounds(editor: &Editor, group: &GroupData) -> Option<Rect> {
    let default_size = editor.user.options.default_node_size().to_string();
    let mut bounds: Option<Rect> = None;
    for child in &group.children {
        let rect = match editor.document.node(*child) {
            Some(node) => {
                let extent = crate::ui::graph_canvas::node_extent(node, &default_size);
                Some(Rect::from_min_size(Pos2::new(node.position[0], node.position[1]), Vec2::new(extent[0], extent[1])))
            }
            None => crate::ui::graph_notes::note_rect(editor, group.graph_index, *child),
        };
        let Some(rect) = rect else { continue };
        bounds = Some(bounds.map_or(rect, |bounds| bounds.union(rect)));
    }
    bounds
}

/// Draws this tab's groups and handles their mouse input.
pub fn show(ui: &mut Ui, painter: &egui::Painter, editor: &mut Editor, state: &mut GraphState, groups: &[GroupData], origin: Pos2, commands: &mut Vec<Command>) {
    let view = state.view();
    let zoom = view.zoom;
    let modifiers = ui.input(|input| input.modifiers);
    for group in groups {
        let screen = view.rect_to_screen(origin, group.bounds());
        if !screen.intersects(state.rect.expand(40.0)) {
            continue;
        }
        let selected = state.selected_groups.contains(&group.id);
        let group_state = state.group_states.get(&group.id).copied();
        draw(ui, painter, group, screen, zoom, selected, group_state);

        // The header grabs, selects and (double-clicked) collapses.
        let header_height = if group.collapsed { screen.height() } else { (HEADER_HEIGHT * zoom).min(screen.height()) };
        let header = Rect::from_min_size(screen.min, Vec2::new(screen.width(), header_height));
        let header_response = ui.interact(header, ui.id().with(("group-header", group.id)), Sense::click_and_drag());
        let tooltip_text = group.name.clone();
        let header_response = header_response.on_hover_text(tooltip_text);
        if header_response.clicked() {
            select_group(editor, state, group.id, modifiers.command || modifiers.shift);
        }
        if header_response.double_clicked() {
            commands.push(Command::Graph(GraphCommand::ToggleGroupCollapsed(group.id)));
        }
        if header_response.drag_started_by(PointerButton::Primary) && state.group_drag.is_none() && !group.locked {
            if !state.selected_groups.contains(&group.id) {
                select_group(editor, state, group.id, false);
            }
            start_drag(ui, editor, state, group, GroupDragKind::Move, &header_response, origin);
        }

        // The menu button (hidden while collapsed, zoomed out or narrow).
        let zoomed_out = zoom < SIMPLIFIED_ZOOM;
        let small = group.size[0] < SMALL_WIDTH;
        if !group.collapsed && !zoomed_out && !small {
            let button = Rect::from_min_size(screen.min + Vec2::splat(MENU_MARGIN * zoom), MENU_BUTTON * zoom);
            let button_response = ui.interact(button, ui.id().with(("group-menu", group.id)), Sense::click());
            if button_response.hovered() {
                painter.rect_filled(button, 2.0 * zoom, theme::MENU_BACKGROUND_HOVER.gamma_multiply(0.6));
            }
            let icon_rect = Rect::from_center_size(button.center(), Vec2::splat(14.0 * zoom));
            icons::paint(ui, icon_rect, "menu", theme::swatch_color(&group.color));
            if button_response.clicked() {
                select_group(editor, state, group.id, false);
            }
            group_menu_popup(&button_response, group, commands);
        }

        // The resize thumbs (no resizing while locked or collapsed).
        if !group.collapsed && !group.locked {
            let thumb_size = Vec2::splat(THUMB * zoom);
            let right = Rect::from_min_size(screen.max - thumb_size, thumb_size);
            let left = Rect::from_min_size(Pos2::new(screen.left(), screen.bottom() - thumb_size.y), thumb_size);
            let thumbs = [(right, GroupDragKind::ResizeRight, CursorIcon::ResizeNwSe), (left, GroupDragKind::ResizeLeft, CursorIcon::ResizeNeSw)];
            for (rect, kind, cursor) in thumbs {
                let response = ui.interact(rect, ui.id().with(("group-thumb", group.id, kind == GroupDragKind::ResizeLeft)), Sense::drag());
                if response.hovered() || response.dragged() {
                    ui.ctx().set_cursor_icon(cursor);
                }
                if response.drag_started_by(PointerButton::Primary) && state.group_drag.is_none() {
                    start_drag(ui, editor, state, group, kind, &response, origin);
                }
            }
        }
    }
    continue_drag(ui, editor, state, origin);
}

/// `GroupElement.Select`: the group alone, or (Ctrl, Shift) in or out of
/// the selection.
fn select_group(editor: &mut Editor, state: &mut GraphState, id: i32, additive: bool) {
    if additive {
        if let Some(position) = state.selected_groups.iter().position(|selected| *selected == id) {
            state.selected_groups.remove(position);
        } else {
            state.selected_groups.push(id);
        }
        return;
    }
    state.clear_element_selection();
    editor.select_group();
    state.selected_groups = vec![id];
}

fn start_drag(ui: &Ui, editor: &mut Editor, state: &mut GraphState, group: &GroupData, kind: GroupDragKind, response: &Response, origin: Pos2) {
    let view = state.view();
    let pointer = response.interact_pointer_pos().or_else(|| ui.input(|input| input.pointer.interact_pos())).unwrap_or(origin);
    let pointer_start = view.to_canvas(origin, pointer);
    let label = if kind == GroupDragKind::Move { "Move" } else { "Group resized" };
    editor.document.checkpoint(label);
    let drag = GroupDrag {
        id: group.id,
        kind,
        pointer_start,
        position: group.position,
        size: group.size,
        children: children_bounds(editor, group),
    };
    state.group_drag = Some(drag);
}

/// `UpdateChildrenPosition` and `ResizeThumb_DragDelta` as the pointer
/// moves: a moved group takes its nodes and notes along; a resized one
/// keeps at least 270 square and 20 past its children, and its left edge
/// never passes them.
fn continue_drag(ui: &Ui, editor: &mut Editor, state: &mut GraphState, origin: Pos2) {
    let Some(drag) = state.group_drag else {
        return;
    };
    let view = state.view();
    if let Some(pointer) = ui.input(|input| input.pointer.interact_pos()) {
        let current = view.to_canvas(origin, pointer);
        let delta = current - drag.pointer_start;
        let min_height = drag.children.map_or(0.0, |children| children.bottom() - drag.position[1] + ELEMENTS_PADDING);
        let height = (drag.size[1] + delta.y).max(GROUP_MIN_SIZE[1]).max(min_height);
        match drag.kind {
            GroupDragKind::Move => {
                let moved = [drag.position[0] + delta.x, drag.position[1] + delta.y];
                editor.document.move_group(drag.id, moved, false);
            }
            GroupDragKind::ResizeRight => {
                let min_width = drag.children.map_or(0.0, |children| children.right() - drag.position[0] + ELEMENTS_PADDING);
                let width = (drag.size[0] + delta.x).max(GROUP_MIN_SIZE[0]).max(min_width);
                editor.document.set_group_bounds(drag.id, drag.position, [width, height]);
            }
            GroupDragKind::ResizeLeft => {
                let right = drag.position[0] + drag.size[0];
                let mut left = (drag.position[0] + delta.x).min(right - GROUP_MIN_SIZE[0]);
                if let Some(children) = drag.children {
                    left = left.min(children.left() - 1.0);
                }
                let width = right - left;
                editor.document.set_group_bounds(drag.id, [left, drag.position[1]], [width, height]);
            }
        }
    }
    if !ui.input(|input| input.pointer.primary_down()) {
        state.group_drag = None;
    }
}

/// The group's look (`GroupElement` template).
fn draw(ui: &Ui, painter: &egui::Painter, group: &GroupData, screen: Rect, zoom: f32, selected: bool, group_state: Option<GroupState>) {
    let color = theme::swatch_color(&group.color);
    let radius = 3.0 * zoom;
    // `PART_HitRect` (0.10) and `PART_Header` (0.05, none while collapsed).
    painter.rect_filled(screen, radius, color.gamma_multiply(0.10));
    if !group.collapsed {
        let header = Rect::from_min_size(screen.min, Vec2::new(screen.width(), (HEADER_HEIGHT * zoom).min(screen.height())));
        let corners = CornerRadius { nw: radius as u8, ne: radius as u8, sw: 0, se: 0 };
        painter.rect_filled(header, corners, color.gamma_multiply(0.05));
    }
    if selected {
        painter.rect_stroke(screen, radius, Stroke::new(1.0, theme::ACCENT_MUTED_PRIMARY), StrokeKind::Inside);
    }
    // `PART_IndicatorBorder` (margin 1, 2 wide).
    let indicator = match group_state {
        Some(GroupState::Incoming) => Some(theme::ACCENT_MUTED_GREEN),
        Some(GroupState::Outgoing) => Some(theme::ACCENT_MUTED_ORANGE),
        None => None,
    };
    if let Some(indicator) = indicator {
        painter.rect_stroke(screen.shrink(1.0), radius, Stroke::new(2.0, indicator), StrokeKind::Inside);
    }
    let icon = if group.icon.is_empty() { "square-rounded" } else { group.icon.as_str() };
    let title_color = if group.locked { color.gamma_multiply(0.5) } else { color };
    let zoomed_out = zoom < SIMPLIFIED_ZOOM;
    if group.collapsed {
        if !zoomed_out {
            let icon_rect = Rect::from_center_size(screen.center(), Vec2::splat(icons::DEFAULT_SIZE * zoom));
            icons::paint(ui, icon_rect, icon, color);
            // The name below the box (margin 4, 0, 0, -16; 10px; 60 wide).
            let galley = elided_galley(painter, &group.name, theme::interface(10.0 * zoom), title_color, 60.0 * zoom);
            let position = Pos2::new(screen.left() + 4.0 * zoom, screen.bottom() + 16.0 * zoom - galley.size().y);
            painter.galley(position, galley, title_color);
        }
    } else if !zoomed_out {
        let icon_rect = Rect::from_min_size(screen.min + Vec2::new(35.0, 7.0) * zoom, Vec2::splat(14.0 * zoom));
        icons::paint(ui, icon_rect, icon, color);
        let galley = elided_galley(painter, &group.name, theme::interface(12.0 * zoom), title_color, 150.0 * zoom);
        let position = screen.min + Vec2::new(55.0, 6.0) * zoom;
        painter.galley(position, galley, title_color);
    }
    // `PART_Lock` (margin 6, bottom left).
    if group.locked {
        let size = icons::DEFAULT_SIZE * zoom;
        let lock_rect = Rect::from_min_size(Pos2::new(screen.left() + 6.0 * zoom, screen.bottom() - 6.0 * zoom - size), Vec2::splat(size));
        icons::paint(ui, lock_rect, "lock", color);
    }
}

/// The group's menu (`ctxGroup`, opened below its menu button): Group to
/// Tab, Rename, Lock, then the colours and icons to choose from.
fn group_menu_popup(button: &Response, group: &GroupData, commands: &mut Vec<Command>) {
    let mut popup = egui::Popup::menu(button);
    popup = popup.align(egui::RectAlign::BOTTOM_START);
    popup = popup.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    popup.show(|ui| {
        menu_style(ui);
        if menu_item(ui, Some("stack-pop"), None, "Group to Tab", "").clicked() {
            commands.push(Command::Graph(GraphCommand::GroupToTab(group.id)));
        }
        menu_separator(ui);
        if menu_item(ui, None, None, "Rename", "F2").clicked() {
            commands.push(Command::Graph(GraphCommand::RenameGroup(group.id)));
        }
        if check_item(ui, group.locked, "Lock", "Ctrl L").clicked() {
            commands.push(Command::Graph(GraphCommand::SetGroupLocked(group.id, !group.locked)));
        }
        menu_caption(ui, "Color");
        if let Some(color) = swatch_grid(ui) {
            commands.push(Command::Graph(GraphCommand::SetGroupColor(group.id, color)));
            ui.close();
        }
        menu_caption(ui, "Icon");
        if let Some(icon) = icon_grid(ui, &GROUP_ICONS) {
            commands.push(Command::Graph(GraphCommand::SetGroupIcon(group.id, icon)));
            ui.close();
        }
    });
}

/// `RenameGroup`: the name edited in place over the title (an
/// `EditablePopup`, closing as [`crate::ui::widgets::popup_outcome`]
/// says); a blank name is no name.
pub fn rename_overlay(ui: &mut Ui, editor: &mut Editor, state: &mut GraphState, groups: &[GroupData], origin: Pos2) {
    let Some((id, mut text)) = state.renaming_group.take() else {
        return;
    };
    let Some(group) = groups.iter().find(|group| group.id == id) else {
        return;
    };
    let view = state.view();
    let screen = view.rect_to_screen(origin, group.bounds());
    let top_left = if group.collapsed { Pos2::new(screen.left(), screen.bottom() + 2.0) } else { screen.min + Vec2::new(52.0, 4.0) * view.zoom };
    let rect = Rect::from_min_size(top_left, Vec2::new(160.0, 22.0));
    let edit = egui::TextEdit::singleline(&mut text).font(theme::interface(12.0)).id(Id::new(("group-rename", id)));
    let response = ui.put(rect, edit);
    if !response.has_focus() && !response.lost_focus() {
        response.request_focus();
    }
    let Some(accepted) = crate::ui::widgets::popup_outcome(ui, &response) else {
        state.renaming_group = Some((id, text));
        return;
    };
    if accepted && !text.trim().is_empty() {
        editor.document.rename_group(id, &text);
    }
}
