//! The graph grid (`GraphGrid`): the graph area (`GraphArea`: graph tabs
//! along the top, the node graph, the toolbox over its corner, the build
//! clock and progress at the bottom), the Data View beside it while open,
//! and the graph toolbar on the right.

use egui::{Align, CornerRadius, CursorIcon, Layout, Pos2, Rect, Sense, Shape, Stroke, Ui, UiBuilder, Vec2};

use crate::document::CombineKind;
use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::graph_commands::GraphCommand;
use crate::ui::graph_popups::swatch_grid;
use crate::ui::widgets::{icon_button, icon_menu, menu_caption, menu_item, menu_item_enabled, menu_separator, menu_style, submenu, toolbar_background_corners, toolbar_separator};
use crate::ui::{data_view, graph_canvas, menus, toolbox, UiState};
use crate::user::ToolboxStyle;

/// Height of the tab strip (`GraphTabsGrid`) and of a tab (`GraphTabItem`).
const TABS_HEIGHT: f32 = 26.0;
const TAB_HEIGHT: f32 = 25.0;
/// `TabBorderThickness`: tab outlines, the line under the tabs and the
/// graph's border.
const TAB_BORDER: f32 = 1.5;
/// `GraphBorder`'s top margin below the tabs.
const GRAPH_TOP: f32 = 25.0;
/// `GraphToolbarBorder` (`Toolbar`): a `Toolbar.ButtonSize` button,
/// `Toolbar.Padding` and a border of 1.5 each side.
fn toolbar_width(sizes: &theme::Sizes) -> f32 {
    sizes.toolbar_button + 2.0 * (sizes.toolbar_padding[0] + 1.5)
}
/// Each area's margin in the grid.
const AREA_MARGIN: f32 = 2.0;
/// `gsVertical1`: 4 wide at the graph column's right edge.
const SPLITTER: f32 = 4.0;
/// How narrow the graph may get beside the Data View.
const MIN_GRAPH: f32 = 160.0;

pub fn show(ui: &mut Ui, graph_grid: Rect, editor: &mut Editor, ui_state: &mut UiState, commands: &mut Vec<Command>) {
    // The toolbar column (auto): margin 2, or 0, 2, 2, 2 against the open
    // Data View (`ToggleCollapse`).
    let expanded = ui_state.data_view.expanded;
    let toolbar_left_margin = if expanded { 0.0 } else { AREA_MARGIN };
    let toolbar_column_width = toolbar_left_margin + toolbar_width(theme::sizes(ui.ctx())) + AREA_MARGIN;
    let toolbar_column = Rect::from_min_max(Pos2::new(graph_grid.right() - toolbar_column_width, graph_grid.top()), graph_grid.max);
    let toolbar_rect = Rect::from_min_max(toolbar_column.min + Vec2::new(toolbar_left_margin, AREA_MARGIN), toolbar_column.max - Vec2::splat(AREA_MARGIN));

    // The star columns: the graph `5*`, the Data View `1.05*` (at least
    // 340) while open.
    let stars = (toolbar_column.left() - graph_grid.left()).max(0.0);
    let data_width = if expanded { data_view_width(stars, ui_state.data_view.share) } else { 0.0 };
    let split = (toolbar_column.left() - data_width).round();
    let graph_column = Rect::from_min_max(graph_grid.min, Pos2::new(split, graph_grid.bottom()));
    let area = graph_column.shrink(AREA_MARGIN);
    ui.painter().rect_filled(area, 4.0, theme::PANEL_BACKGROUND);
    ui_state.graph.dimmed = data_view::dimmed_nodes(editor, ui_state.data_view.terrain.filter);

    // `Grid` (margin 0, 0, 1, 0): `GraphBorder` 25 down with its 1.5 sides
    // and bottom in the chosen tab's colour, the graph (margin 2) in it.
    let grid = Rect::from_min_max(area.min, area.max - Vec2::new(1.0, 0.0));
    let tabs_list = editor.document.graph_tabs();
    let selected_color = tabs_list.iter().find(|tab| tab.index == ui_state.graph.tab).map(|tab| theme::swatch_color(&tab.color)).unwrap_or(theme::TAB_BORDER);
    // `ShowHideGraphTabs`: Hide Graph Tabs with a single tab takes the tab
    // strip away. The border then reaches the top and closes round the
    // graph (1.5 all round, corners 3, `Accent-Muted-Gray`), and the
    // toolbox and the engine's outline move up with it.
    let tabs_hidden = editor.user.options.hide_tabs() && tabs_list.len() == 1;
    let graph_rect = if tabs_hidden {
        graph_border_closed(ui, grid);
        grid.shrink(TAB_BORDER + 2.0)
    } else {
        let border = Rect::from_min_max(grid.min + Vec2::new(0.0, GRAPH_TOP), grid.max);
        graph_border(ui, border, selected_color);
        Rect::from_min_max(border.min + Vec2::new(TAB_BORDER + 2.0, 2.0), border.max - Vec2::splat(TAB_BORDER + 2.0))
    };
    graph_canvas::show(ui, graph_rect, editor, &mut ui_state.graph, commands);
    // `Toolbox`: margin 0, 35, 0, 0 for the menus and 0, 16, 0, 24 for the
    // panel; none without tabs.
    let toolbox_area = if tabs_hidden {
        grid
    } else if editor.user.options.toolbox_style() == ToolboxStyle::Menus {
        Rect::from_min_max(grid.min + Vec2::new(0.0, 35.0), grid.max)
    } else {
        Rect::from_min_max(grid.min + Vec2::new(0.0, 16.0), grid.max - Vec2::new(0.0, 24.0))
    };
    toolbox::show(ui, toolbox_area, editor, &mut ui_state.graph, commands);
    if !tabs_hidden {
        tabs(ui, grid, editor, ui_state, selected_color, commands);
    }
    // `stackBookmarks` stays (margin 0, 0, 2, 0; 8 on the right without
    // tabs).
    let bookmarks_margin = if tabs_hidden { 8.0 } else { 2.0 };
    bookmark_buttons(ui, grid, bookmarks_margin, editor, commands);
    footer(ui, grid, editor);
    if !editor.engine_active {
        // `EngineRect`: margin 4, 28, 4, 4; 4 all round without tabs.
        let outline_top = if tabs_hidden { 4.0 } else { 28.0 };
        engine_suspended_outline(ui, grid, outline_top);
    }

    if expanded {
        let data_column = Rect::from_min_max(Pos2::new(split, graph_column.top()), Pos2::new(toolbar_column.left(), graph_column.bottom()));
        let data_rect = Rect::from_min_max(data_column.min + Vec2::splat(AREA_MARGIN), data_column.max - Vec2::new(0.0, AREA_MARGIN));
        data_view::show(ui, data_rect, editor, &mut ui_state.data_view, &mut ui_state.graph, commands);
        data_view_splitter(ui, graph_column, stars, ui_state);
        ui_state.graph.areas.data_view = Some(data_rect);
    } else {
        ui_state.graph.areas.data_view = None;
    }
    graph_toolbar(ui, toolbar_rect, editor, ui_state, commands);
}

/// The Data View's width in the star space: its share, at least 340 and
/// leaving the graph some room.
fn data_view_width(stars: f32, share: f32) -> f32 {
    let wanted = (stars * share).max(data_view::MIN_WIDTH);
    let room = (stars - MIN_GRAPH).max(data_view::MIN_WIDTH);
    wanted.min(room).min(stars)
}

/// `gsVertical1` (enabled while the Data View is open): dragging it moves
/// width between the graph and the Data View.
fn data_view_splitter(ui: &mut Ui, graph_column: Rect, stars: f32, ui_state: &mut UiState) {
    let handle = Rect::from_min_max(Pos2::new(graph_column.right() - SPLITTER, graph_column.top()), graph_column.max);
    let response = ui.interact(handle, ui.id().with("data-view-splitter"), Sense::drag());
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if !response.dragged() || stars <= 0.0 {
        return;
    }
    let current = data_view_width(stars, ui_state.data_view.share);
    let wanted = current - response.drag_delta().x;
    ui_state.data_view.share = (wanted / stars).clamp(0.0, 1.0);
}

/// `GraphBorder` without tabs: 1.5 all round, corners 3, in
/// `Accent-Muted-Gray`.
fn graph_border_closed(ui: &Ui, border: Rect) {
    let stroke = Stroke::new(TAB_BORDER, theme::ACCENT_MUTED_GRAY);
    ui.painter().rect_stroke(border, 3.0, stroke, egui::StrokeKind::Inside);
}

/// `GraphBorder`: left, right and bottom sides (1.5), bottom corners 3.
fn graph_border(ui: &Ui, border: Rect, color: egui::Color32) {
    let half = TAB_BORDER / 2.0;
    let inner = Rect::from_min_max(border.min + Vec2::new(half, 0.0), border.max - Vec2::splat(half));
    let radius = 3.0 - half;
    let mut points = vec![inner.left_top()];
    let bottom_left_center = Pos2::new(inner.left() + radius, inner.bottom() - radius);
    let bottom_right_center = Pos2::new(inner.right() - radius, inner.bottom() - radius);
    // Screen angles (y down): left is pi, the bottom pi/2, right 0.
    for step in 0..=6 {
        let angle = std::f32::consts::PI - step as f32 * std::f32::consts::FRAC_PI_2 / 6.0;
        points.push(bottom_left_center + Vec2::new(angle.cos(), angle.sin()) * radius);
    }
    for step in 0..=6 {
        let angle = std::f32::consts::FRAC_PI_2 - step as f32 * std::f32::consts::FRAC_PI_2 / 6.0;
        points.push(bottom_right_center + Vec2::new(angle.cos(), angle.sin()) * radius);
    }
    points.push(inner.right_top());
    ui.painter().add(Shape::line(points, Stroke::new(TAB_BORDER, color)));
}

/// `TabShape` (`TabMiddle`) for a tab `width` wide: flared feet at the
/// bottom, corners of 4 at the top (the SVG path `GetSvgPath` builds,
/// stretched to fill the tab).
fn tab_outline(rect: Rect) -> Vec<Pos2> {
    let path_width = (rect.width() - 12.0).max(0.0) + 8.0;
    let scale_x = rect.width() / path_width;
    let scale_y = rect.height() / 25.0;
    let at = |x: f32, y: f32| Pos2::new(rect.left() + x * scale_x, rect.top() + y * scale_y);
    let arc = |center: (f32, f32), from: f32, to: f32, points: &mut Vec<Pos2>| {
        for step in 0..=6 {
            let angle = from + (to - from) * step as f32 / 6.0;
            points.push(at(center.0 + 4.0 * angle.cos(), center.1 + 4.0 * angle.sin()));
        }
    };
    let right = path_width - 8.0;
    let half_pi = std::f32::consts::FRAC_PI_2;
    let pi = std::f32::consts::PI;
    let mut points = Vec::new();
    // The left foot: from (0, 25) round to (4, 21) about (0, 21).
    arc((0.0, 21.0), half_pi, 0.0, &mut points);
    // Up to the top-left corner about (8, 4).
    arc((8.0, 4.0), pi, pi + half_pi, &mut points);
    // Across to the top-right corner about (right, 4).
    arc((right, 4.0), -half_pi, 0.0, &mut points);
    // Down to the right foot about (right + 8, 21).
    arc((right + 8.0, 21.0), pi, half_pi, &mut points);
    points
}

/// The graph tabs (`GraphTabList`, 1 down): each at least 135 wide, its
/// colour bar 12 in and its name (monospace 11) 24 in. The chosen tab is
/// outlined in its colour and tinted with it; the others, and the rest of
/// the strip, stand on a line of the chosen tab's colour. Each tab's menu
/// offers Color, Rename and Delete; the strip's offers New Tab.
fn tabs(ui: &mut Ui, area: Rect, editor: &Editor, ui_state: &mut UiState, selected_color: egui::Color32, commands: &mut Vec<Command>) {
    let strip = Rect::from_min_size(area.min, Vec2::new(area.width(), TABS_HEIGHT));
    let strip_response = ui.interact(strip, ui.id().with("graph-tab-strip"), Sense::click());
    strip_response.context_menu(|ui| {
        menu_style(ui);
        if menu_item(ui, Some("square-rounded-plus-2"), Some(theme::ACCENT_BLUE), "New Tab", "").clicked() {
            commands.push(Command::NewGraphTab);
        }
    });

    let tabs = editor.document.graph_tabs();
    let only_tab = tabs.len() <= 1;
    let line_stroke = Stroke::new(TAB_BORDER, selected_color);
    let line_y = strip.top() + 1.0 + TAB_HEIGHT - TAB_BORDER / 2.0;
    let mut x = strip.left();
    for tab in &tabs {
        let swatch = theme::swatch_color(&tab.color);
        let selected = ui_state.graph.tab == tab.index;
        let galley = ui.painter().layout_no_wrap(tab.name.clone(), theme::monospace(11.0), theme::LABEL_EMPHASIS);
        let width = (galley.size().x + 48.0).max(135.0);
        let tab_rect = Rect::from_min_size(Pos2::new(x, strip.top() + 1.0), Vec2::new(width, TAB_HEIGHT));
        let response = ui.interact(tab_rect, ui.id().with(("graph-tab", tab.index)), Sense::click());
        if response.clicked() {
            ui_state.graph.tab = tab.index;
        }
        if response.double_clicked() {
            ui_state.graph.renaming_tab = Some((tab.index, tab.name.clone()));
        }
        // `CreateContextMenuForTab`: Tab to group, Rename, Delete, the colours.
        response.context_menu(|ui| {
            menu_style(ui);
            submenu(ui, Some("stack-push"), None, "Tab to group", |ui| {
                if let Some(target) = menus::tab_targets(ui, editor, tab.index) {
                    let command = GraphCommand::TabToGroup { from: tab.index, target };
                    commands.push(Command::Graph(command));
                }
            });
            menu_separator(ui);
            if menu_item(ui, None, None, "Rename", "").clicked() {
                ui_state.graph.renaming_tab = Some((tab.index, tab.name.clone()));
            }
            if menu_item_enabled(ui, Some("square-rounded-x"), Some(theme::ACCENT_RED), "Delete", "", !only_tab).clicked() {
                ui_state.graph.confirm_delete_tab = Some(tab.index);
            }
            menu_caption(ui, "Color");
            if let Some(swatch_name) = swatch_grid(ui) {
                commands.push(Command::SetGraphTabColor(tab.index, swatch_name));
                ui.close();
            }
        });

        let outline = tab_outline(tab_rect);
        if selected {
            let body = Rect::from_min_max(Pos2::new(tab_rect.left() + 4.0, tab_rect.top()), Pos2::new(tab_rect.right() - 4.0, tab_rect.bottom()));
            let corners = egui::CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 };
            ui.painter().rect_filled(body, corners, swatch.gamma_multiply(0.15));
            ui.painter().add(Shape::line(outline, Stroke::new(TAB_BORDER, swatch)));
        } else {
            if response.hovered() {
                ui.painter().rect_filled(tab_rect, 4.0, theme::GRAPH_TAB_BACKGROUND.gamma_multiply(0.15));
            }
            ui.painter().line_segment([Pos2::new(tab_rect.left(), line_y), Pos2::new(tab_rect.right(), line_y)], line_stroke);
        }
        let swatch_rect = Rect::from_min_max(Pos2::new(tab_rect.left() + 12.0, tab_rect.top() + 6.0), Pos2::new(tab_rect.left() + 15.0, tab_rect.bottom() - 6.0));
        ui.painter().rect_filled(swatch_rect, 1.5, swatch);

        let renaming = ui_state.graph.renaming_tab.as_ref().filter(|(index, _)| *index == tab.index).map(|(_, text)| text.clone());
        match renaming {
            Some(mut text) => {
                let edit_rect = Rect::from_min_max(Pos2::new(tab_rect.left() + 22.0, tab_rect.top() + 3.0), Pos2::new(tab_rect.right() - 6.0, tab_rect.bottom() - 3.0));
                let edit = egui::TextEdit::singleline(&mut text).font(theme::monospace(11.0));
                let edit_response = ui.put(edit_rect, edit);
                if !edit_response.has_focus() && !edit_response.lost_focus() {
                    edit_response.request_focus();
                }
                // An `EditablePopup`: a blank name is no name.
                match crate::ui::widgets::popup_outcome(ui, &edit_response) {
                    None => ui_state.graph.renaming_tab = Some((tab.index, text)),
                    Some(accepted) => {
                        if accepted && !text.trim().is_empty() {
                            commands.push(Command::RenameGraphTab(tab.index, text.trim().to_string()));
                        }
                        ui_state.graph.renaming_tab = None;
                    }
                }
            }
            None => {
                let text_position = Pos2::new(tab_rect.left() + 24.0, tab_rect.center().y - galley.size().y / 2.0);
                ui.painter().galley(text_position, galley, theme::LABEL_EMPHASIS);
            }
        }
        // Tabs overlap by 1 (`Margin 0,0,-1,0`).
        x += width - 1.0;
    }
    // `ExtraBorder`: the line on to the end of the strip.
    ui.painter().line_segment([Pos2::new(x, line_y), Pos2::new(strip.right(), line_y)], line_stroke);

    confirm_tab_delete(ui, &tabs, ui_state, commands);
}

/// `stackBookmarks` (`UpdateBookmarkMenu`): a small bar for each bookmark
/// at the right of the tab strip, `right_margin` in, in its node's toolbox
/// colour, its name as the tooltip; a click goes to the node.
/// `HideBookmarkShortcuts` hides them.
fn bookmark_buttons(ui: &mut Ui, area: Rect, right_margin: f32, editor: &Editor, commands: &mut Vec<Command>) {
    if editor.user.options.hide_bookmark_shortcuts() {
        return;
    }
    let bookmarks = editor.document.bookmarks();
    let nodes: Vec<&nerthus_engine::NodeData> = bookmarks.iter().filter_map(|id| editor.document.node(*id)).collect();
    // `BookmarkButton`: padding 2 around a 6 by 12 bar, margin 2, 0.
    let button_size = Vec2::new(6.0 + 4.0, 12.0 + 4.0);
    let mut right = area.right() - right_margin;
    let center_y = area.top() + TABS_HEIGHT / 2.0;
    for node in nodes.iter().rev() {
        let rect = Rect::from_min_max(Pos2::new(right - 2.0 - button_size.x, center_y - button_size.y / 2.0), Pos2::new(right - 2.0, center_y + button_size.y / 2.0));
        right = rect.left() - 2.0;
        let response = ui.interact(rect, ui.id().with(("bookmark-button", node.id)), Sense::click());
        if response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        let bar = rect.shrink(2.0);
        let color = graph_canvas::node_color(node);
        ui.painter().rect_filled(bar, 2.0, color);
        ui.painter().rect_stroke(bar, 2.0, Stroke::new(1.0, theme::BUTTON_BORDER_ALT), egui::StrokeKind::Inside);
        let response = response.on_hover_text(node.name.clone());
        if response.clicked() {
            commands.push(Command::GoToBookmark(node.id));
        }
    }
}

/// `MiDeleteGraphTab_Click`: asks before a tab and its nodes go.
fn confirm_tab_delete(ui: &mut Ui, tabs: &[crate::document::GraphTabInfo], ui_state: &mut UiState, commands: &mut Vec<Command>) {
    let Some(index) = ui_state.graph.confirm_delete_tab else {
        return;
    };
    let name = tabs.iter().find(|tab| tab.index == index).map(|tab| tab.name.clone()).unwrap_or_default();
    let modal = egui::Modal::new(egui::Id::new("confirm-delete-tab"));
    let response = modal.show(ui.ctx(), |ui| {
        ui.set_width(320.0);
        ui.label(egui::RichText::new(format!("Delete Group \"{name}\" ?")).font(theme::semibold(13.0)).color(theme::LABEL_EMPHASIS));
        ui.add_space(6.0);
        ui.label(format!("Are you sure you want to delete this Graph \"{name}\"?"));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button("Yes").clicked() {
                commands.push(Command::DeleteGraphTab(index));
                ui_state.graph.confirm_delete_tab = None;
            }
            if ui.button("No").clicked() {
                ui_state.graph.confirm_delete_tab = None;
            }
        });
    });
    if response.should_close() {
        ui_state.graph.confirm_delete_tab = None;
    }
}

/// The build strip's width while the graph has room for it.
const FOOTER_WIDTH: f32 = 200.0;
/// The build strip's inset from the graph's left and bottom edges (inside
/// the border), and from its right edge while the graph is narrower than
/// the strip.
const FOOTER_INSET: f32 = 1.75;
/// `ProgressX` in Lines mode: twenty ticks, 2 wide and 6 apart.
const FOOTER_TICKS: usize = 20;
const TICK_WIDTH: f32 = 2.0;
const TICK_SPACING: f32 = 6.0;
/// Room left after the last tick that fits.
const TICKS_END_MARGIN: f32 = 4.0;

/// The build strip at the bottom left of the graph `area`: 200 wide, or
/// narrower so it stays inside the graph's border when the graph is
/// narrow. None when the graph has no room for it at all.
fn footer_rect(area: Rect) -> Option<Rect> {
    let width = FOOTER_WIDTH.min(area.width() - 2.0 * FOOTER_INSET);
    if width <= 0.0 {
        return None;
    }
    let min = Pos2::new(area.left() + FOOTER_INSET, area.bottom() - FOOTER_INSET - 26.0);
    Some(Rect::from_min_size(min, Vec2::new(width, 26.0)))
}

/// How many of the progress ticks, the first centred at `start_x`, fit
/// before `end_x`: all twenty in the full strip, fewer in a narrower one.
fn ticks_that_fit(start_x: f32, end_x: f32) -> usize {
    let room = end_x - TICK_WIDTH / 2.0 - start_x;
    if room < 0.0 {
        return 0;
    }
    ((room / TICK_SPACING).floor() as usize + 1).min(FOOTER_TICKS)
}

/// The build clock and progress (`StateIcon`, `TotalTime`, `Progress`),
/// kept to the strip `footer_rect` gives.
fn footer(ui: &mut Ui, area: Rect, editor: &Editor) {
    let Some(footer) = footer_rect(area) else {
        return;
    };
    let mut strip = ui.new_child(UiBuilder::new().max_rect(footer));
    strip.set_clip_rect(footer.intersect(ui.clip_rect()));
    let corners = egui::CornerRadius { nw: 0, ne: 4, sw: 0, se: 4 };
    strip.painter().rect_filled(footer, corners, theme::PANEL_CLADDING);
    let icon_color = if editor.building { theme::ACCENT_PRIMARY } else { theme::ICON_DEFAULT };
    let icon_rect = Rect::from_min_size(footer.min + Vec2::new(8.0, 5.0), Vec2::splat(16.0));
    icons::paint(&strip, icon_rect, "activity-heartbeat", icon_color);

    let seconds = editor.build_seconds();
    let minutes = (seconds / 60.0).floor() as u32;
    let remainder = (seconds % 60.0).floor() as u32;
    let clock = format!("{minutes:02}:{remainder:02}");
    let galley = strip.painter().layout_no_wrap(clock, theme::monospace_semibold(9.0), theme::LABEL_DEFAULT);
    let clock_position = Pos2::new(icon_rect.right() + 4.0, footer.center().y - galley.size().y / 2.0);
    let clock_width = galley.size().x;
    strip.painter().galley(clock_position, galley, theme::LABEL_DEFAULT);

    let separator_x = clock_position.x + clock_width + 8.0;
    let stroke = Stroke::new(1.0, theme::MENU_SEPARATOR);
    strip.painter().line_segment([Pos2::new(separator_x, footer.center().y - 4.0), Pos2::new(separator_x, footer.center().y + 4.0)], stroke);

    // `ProgressX` in Lines mode: the ticks that fit, lit by progress.
    let start_x = separator_x + 8.0;
    let ticks = ticks_that_fit(start_x, footer.right() - TICKS_END_MARGIN);
    let progress = if editor.building { editor.build_progress } else { 0.0 };
    let lit = (progress * ticks as f32).round() as usize;
    for tick in 0..ticks {
        let tick_x = start_x + tick as f32 * TICK_SPACING;
        let color = if tick < lit { theme::ACCENT_PRIMARY } else { theme::PROGRESS_EMPTY };
        let top = Pos2::new(tick_x, footer.top() + 8.0);
        let bottom = Pos2::new(tick_x, footer.bottom() - 8.0);
        strip.painter().line_segment([top, bottom], Stroke::new(TICK_WIDTH, color));
    }
    if editor.building {
        ui.ctx().request_repaint();
    }
}

/// The crawling red outline while the engine is suspended (`EngineRect`),
/// `top` down.
fn engine_suspended_outline(ui: &Ui, area: Rect, top: f32) {
    let rect = Rect::from_min_max(area.min + Vec2::new(4.0, top), area.max - Vec2::splat(4.0));
    let points = vec![rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
    let time = ui.input(|input| input.time) as f32;
    let offset = (time * 16.0) % 6.0;
    let stroke = Stroke::new(1.5, theme::ACCENT_RED);
    let dashes = Shape::dashed_line_with_offset(&points, stroke, &[1.5], &[4.5], offset);
    ui.painter().extend(dashes);
    ui.ctx().request_repaint();
}

/// The toolbar right of the graph (`GraphToolbarBorder`, `Menu-Border`
/// 1.5): its corners square on the left while the Data View is open.
fn graph_toolbar(ui: &mut Ui, toolbar_rect: Rect, editor: &Editor, ui_state: &mut UiState, commands: &mut Vec<Command>) {
    let expanded = ui_state.data_view.expanded;
    let corners = if expanded { CornerRadius { nw: 0, ne: 4, sw: 0, se: 4 } } else { CornerRadius::same(4) };
    toolbar_background_corners(ui, toolbar_rect, corners, theme::MENU_BORDER, 1.5);
    let [padding_across, padding_down] = theme::sizes(ui.ctx()).toolbar_padding;
    let inner = toolbar_rect.shrink(1.5).shrink2(Vec2::new(padding_across, padding_down));

    let top_builder = UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Center));
    let mut top = ui.new_child(top_builder);
    top.spacing_mut().item_spacing.y = 0.0;
    // `btnExpandToggle`.
    let chevron = if expanded { "chevron-right" } else { "chevron-left" };
    if icon_button(&mut top, chevron, "Data View", None).clicked() {
        ui_state.data_view.toggle();
    }
    toolbar_separator(&mut top, 8.0, 8.0);
    icon_menu(&mut top, "bookmarks", "Node Bookmarks", None, |ui| {
        menus::bookmarks_menu(ui, editor, ui_state, commands);
    });
    icon_menu(&mut top, "tournament", "Node Commands", None, |ui| {
        if menu_item(ui, Some("arrow-left-rhombus"), Some(theme::ACCENT_MUTED_PURPLE), "Select upstream", "").clicked() {
            commands.push(Command::SelectUpstream);
        }
        if menu_item(ui, Some("arrow-right-rhombus"), Some(theme::ACCENT_MUTED_PURPLE), "Select downstream", "").clicked() {
            commands.push(Command::SelectDownstream);
        }
        menu_separator(ui);
        if menu_item(ui, Some("layers-linked"), None, "Group / Ungroup", "").clicked() {
            commands.push(Command::Graph(GraphCommand::GroupUngroup));
        }
        if menu_item(ui, Some("arrows-join-2"), Some(theme::ACCENT_MUTED_ORANGE), "Combine nodes", "").clicked() {
            commands.push(Command::Graph(GraphCommand::Combine(CombineKind::Toolbar)));
        }
        menu_separator(ui);
        submenu(ui, Some("keyframe"), None, "Convert port to portals", |ui| {
            menus::convert_port_items(ui, editor, commands);
        });
        menu_separator(ui);
        submenu(ui, None, None, "Move to Tab", |ui| {
            if let Some(target) = menus::tab_targets(ui, editor, ui_state.graph.tab) {
                commands.push(Command::Graph(GraphCommand::MoveToTab(target)));
            }
        });
    });
    toolbar_separator(&mut top, 8.0, 8.0);
    icon_menu(&mut top, "flame", "Bake", None, |ui| {
        menus::toolbar_bake_items(ui, editor, commands);
    });

    // Docked bottom, first at the bottom: Graph Commands, the engine
    // toggle (`Accent-Muted-Red`; `Accent-Red` outline while suspended),
    // Mutate and Refresh, with separators (margin 2, 8) between.
    let bottom_builder = UiBuilder::new().max_rect(inner).layout(Layout::bottom_up(Align::Center));
    let mut bottom = ui.new_child(bottom_builder);
    bottom.spacing_mut().item_spacing.y = 0.0;
    icon_menu(&mut bottom, "menu-deep", "Graph Commands", None, |ui| {
        menus::graph_commands(ui, editor, ui_state, commands);
    });
    toolbar_separator(&mut bottom, 8.0, 8.0);
    let engine_tip = if editor.engine_active { "Suspend Engine (Currently running)" } else { "Resume Engine (Currently suspended)" };
    if engine_toggle(&mut bottom, !editor.engine_active).on_hover_text(engine_tip).clicked() {
        commands.push(Command::ToggleEngine);
    }
    toolbar_separator(&mut bottom, 8.0, 8.0);
    // `btnMutate` and `btnRefresh`, each with its right-click menu.
    let mutate = icon_button(&mut bottom, "potion", "Mutate", None);
    if mutate.clicked() {
        commands.push(Command::Graph(GraphCommand::MutateButton));
    }
    mutate.context_menu(|ui| {
        menu_style(ui);
        if menu_item(ui, Some("potion"), None, "Mutate all nodes", "").clicked() {
            commands.push(Command::Graph(GraphCommand::MutateAll));
        }
    });
    let refresh = icon_button(&mut bottom, "rotate-360", "Refresh", None);
    if refresh.clicked() {
        commands.push(Command::Refresh);
    }
    refresh.context_menu(|ui| {
        menu_style(ui);
        if menu_item(ui, Some("rotate-360"), None, "Refresh all nodes", "").clicked() {
            commands.push(Command::RefreshAll);
        }
    });
}

/// `tglEngine`: the engine icon in `Accent-Muted-Red`; checked (the
/// engine suspended), the pressed fill with an `Accent-Red` outline.
fn engine_toggle(ui: &mut Ui, suspended: bool) -> egui::Response {
    let size = Vec2::splat(crate::ui::widgets::toolbar_button(ui));
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = response.hovered();
    if suspended && !hovered {
        ui.painter().rect_filled(rect, 4.0, theme::BUTTON_PRESSED);
    } else if hovered {
        ui.painter().rect_filled(rect, 4.0, theme::BUTTON_HOVER);
    }
    if suspended {
        ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.5, theme::ACCENT_RED), egui::StrokeKind::Inside);
    }
    let color = if hovered || suspended { theme::LABEL_EMPHASIS } else { theme::ACCENT_MUTED_RED };
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(icons::DEFAULT_SIZE));
    icons::paint(ui, icon_rect, "engine", color);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    /// The build strip painted at the bottom of a graph frame `width` wide,
    /// its clock at `seconds`: the strip and the shapes painted, and among
    /// them the progress ticks (the 2-wide lines).
    fn painted_footer(width: f32, seconds: f32) -> (Rect, Rect, Vec<egui::epaint::ClippedShape>, Vec<[Pos2; 2]>) {
        let context = egui::Context::default();
        theme::apply(&context);
        let mut editor = Editor::new(Document::new());
        editor.last_build_seconds = seconds;
        let area = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(width, 400.0));
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 960.0))), ..Default::default() };
        let mut output = context.run_ui(input, |ui| footer(ui, area, &editor));
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        let ticks = shapes.iter().filter_map(|clipped| match clipped.shape {
            Shape::LineSegment { points, stroke } if stroke.width == TICK_WIDTH => Some(points),
            _ => None,
        });
        let ticks = ticks.collect();
        (area, footer_rect(area).unwrap_or(Rect::NOTHING), shapes, ticks)
    }

    #[test]
    fn a_wide_graph_shows_the_whole_strip() {
        for seconds in [0.0, 99.0 * 60.0 + 59.0, 100.0 * 60.0] {
            let (area, strip, _, ticks) = painted_footer(800.0, seconds);
            assert_eq!(strip.width(), FOOTER_WIDTH);
            assert_eq!(strip.left(), area.left() + FOOTER_INSET);
            assert_eq!(strip.bottom(), area.bottom() - FOOTER_INSET);
            assert_eq!(ticks.len(), FOOTER_TICKS);
            assert!(ticks.iter().all(|points| points[0].x + TICK_WIDTH / 2.0 <= strip.right()));
        }
    }

    #[test]
    fn a_narrow_graph_keeps_the_strip_inside_its_border() {
        // The graph frame beside the Data View at `MIN_GRAPH` (less its
        // margins and the grid's 1 on the right), and narrower still when
        // the window leaves the Data View its 340 and no more.
        let at_min_graph = MIN_GRAPH - 2.0 * AREA_MARGIN - 1.0;
        for width in [at_min_graph, 120.0, 80.0, 40.0] {
            let (area, strip, shapes, ticks) = painted_footer(width, 0.0);
            let inside_border = area.right() - TAB_BORDER;
            assert!(strip.right() <= inside_border, "{width}: the strip ends at {} past {inside_border}", strip.right());
            assert!(ticks.len() < FOOTER_TICKS, "{width}: all {} ticks drawn", ticks.len());
            for points in &ticks {
                assert!(points[0].x + TICK_WIDTH / 2.0 <= strip.right() - TICKS_END_MARGIN, "{width}: a tick at {} past the strip", points[0].x);
            }
            for clipped in &shapes {
                assert!(strip.contains_rect(clipped.clip_rect), "{width}: {:?} painted outside the strip", clipped.shape);
            }
        }
        let (_, _, _, ticks) = painted_footer(at_min_graph, 0.0);
        assert!(!ticks.is_empty(), "the strip at the graph's minimum width still shows progress");
    }

    #[test]
    fn a_graph_with_no_room_paints_no_strip() {
        let (_, _, shapes, _) = painted_footer(2.0 * FOOTER_INSET, 0.0);
        assert!(shapes.is_empty());
    }

    #[test]
    fn ticks_fit_before_the_end() {
        assert_eq!(ticks_that_fit(100.0, 400.0), FOOTER_TICKS);
        assert_eq!(ticks_that_fit(100.0, 101.0), 1);
        assert_eq!(ticks_that_fit(100.0, 100.5), 0);
        assert_eq!(ticks_that_fit(100.0, 50.0), 0);
        // Ticks at 100, 106, 112 and 118: the last ends at 119.
        assert_eq!(ticks_that_fit(100.0, 120.0), 4);
        assert_eq!(ticks_that_fit(100.0, 118.5), 3);
    }
}
