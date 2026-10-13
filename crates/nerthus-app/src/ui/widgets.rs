//! The recurring controls: toolbars (`Toolbar` border style), icon
//! buttons (`Button-IconOnly`), icon menus (`MenuItem Tag="Icon"`), menu
//! entries (`Default-Menu-Item`) with an icon, an accent and a shortcut,
//! and dialog buttons.

use egui::containers::menu::{MenuButton, MenuConfig, SubMenu};
use egui::{Button, Color32, CornerRadius, FontId, Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::icons;
use crate::theme;

/// `Toolbar.ButtonSize` as the size theme has it.
pub fn toolbar_button(ui: &Ui) -> f32 {
    theme::sizes(ui.ctx()).toolbar_button
}

/// A dialog button's least height.
pub const DIALOG_BUTTON_HEIGHT: f32 = 26.0;
/// `Default-Menu-Item`: rows 23 high (11px text, padding 2, 4), a 30 icon
/// column and a 20 arrow column; menus at least 190 wide.
const MENU_ROW_HEIGHT: f32 = 23.0;
const MENU_ICON_COLUMN: f32 = 30.0;
const MENU_ARROW_COLUMN: f32 = 20.0;
const MENU_MIN_WIDTH: f32 = 190.0;

/// Paints a toolbar's cladding (`Panel-Cladding`, 4px corners).
pub fn toolbar_background(ui: &Ui, rect: Rect, border: Color32, border_width: f32) {
    toolbar_background_corners(ui, rect, CornerRadius::same(4), border, border_width);
}

/// A toolbar's cladding with its own corners (the graph toolbar squares
/// the side the Data View joins).
pub fn toolbar_background_corners(ui: &Ui, rect: Rect, corners: CornerRadius, border: Color32, border_width: f32) {
    let painter = ui.painter();
    painter.rect_filled(rect, corners, theme::PANEL_CLADDING);
    let stroke = Stroke::new(border_width, border);
    painter.rect_stroke(rect, corners, stroke, StrokeKind::Inside);
}

/// A square icon button (`ToolbarButton-IconX`: `Toolbar.ButtonSize`, a 16
/// icon brightening over a rounded hover fill).
pub fn icon_button(ui: &mut Ui, icon: &str, tooltip: &str, tint: Option<Color32>) -> Response {
    let size = Vec2::splat(toolbar_button(ui));
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = response.hovered();
    if response.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, 4.0, theme::BUTTON_PRESSED);
    } else if hovered {
        ui.painter().rect_filled(rect, 4.0, theme::BUTTON_HOVER);
    }
    let base = tint.unwrap_or(theme::ICON_DEFAULT);
    let color = if hovered && tint.is_none() { theme::LABEL_EMPHASIS } else { base };
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(icons::DEFAULT_SIZE));
    icons::paint(ui, icon_rect, icon, color);
    response.on_hover_text(tooltip)
}

/// `Button-IconOnly`: an icon of `icon_size` in a button of `size`; under
/// the pointer a dark fill and the muted grey.
pub fn icon_button_sized(ui: &mut Ui, icon: &str, tooltip: &str, tint: Option<Color32>, size: Vec2, icon_size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = response.hovered();
    if hovered {
        ui.painter().rect_filled(rect, 2.0, theme::BUTTON_HOVER);
    }
    let base = tint.unwrap_or(theme::LABEL_DEFAULT);
    let color = if hovered { theme::ACCENT_MUTED_GRAY } else { base };
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(icon_size));
    icons::paint(ui, icon_rect, icon, color);
    response.on_hover_text(tooltip)
}

/// A toggle shown as an icon button (`ToolbarToggleButton-IconX`): on, a
/// pressed fill with the accent's outline and a bright icon.
pub fn icon_toggle(ui: &mut Ui, icon: &str, tooltip: &str, on: bool, accent: Color32) -> Response {
    let size = Vec2::splat(toolbar_button(ui));
    icon_toggle_sized(ui, icon, tooltip, on, accent, size)
}

pub fn icon_toggle_sized(ui: &mut Ui, icon: &str, tooltip: &str, on: bool, accent: Color32, size: Vec2) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = response.hovered();
    let pressed = response.is_pointer_button_down_on();
    let fill = if pressed || (on && !hovered) {
        Some(theme::BUTTON_PRESSED)
    } else if hovered {
        Some(theme::BUTTON_HOVER)
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, 4.0, fill);
    }
    if on {
        ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.5, accent), StrokeKind::Inside);
    }
    let color = if (on || hovered) && !pressed { theme::LABEL_EMPHASIS } else { theme::ICON_DEFAULT };
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(icons::DEFAULT_SIZE));
    icons::paint(ui, icon_rect, icon, color);
    response.on_hover_text(tooltip)
}

/// An icon that opens a menu (`MenuItem Tag="Icon"`): `Toolbar.ButtonSize`
/// square, the `chevron-down-right` mark in its corner, `Menu-TopLevel`
/// under the pointer.
pub fn icon_menu(ui: &mut Ui, icon: &str, tooltip: &str, tint: Option<Color32>, contents: impl FnOnce(&mut Ui)) -> Response {
    let size = Vec2::splat(toolbar_button(ui));
    icon_menu_sized(ui, icon, tooltip, tint, size, contents)
}

pub fn icon_menu_sized(ui: &mut Ui, icon: &str, tooltip: &str, tint: Option<Color32>, size: Vec2, contents: impl FnOnce(&mut Ui)) -> Response {
    let response = icon_menu_button(ui, icon, tint, size, contents);
    // `_iconMenuArrow`: 8 square, margin 1, bottom right.
    let arrow_origin = response.rect.right_bottom() - Vec2::splat(1.0 + 8.0);
    let arrow = Rect::from_min_size(arrow_origin, Vec2::splat(8.0));
    icons::paint(ui, arrow, "chevron-down-right", theme::ICON_DEFAULT);
    response.on_hover_text(tooltip)
}

/// An icon button that opens a menu, without the corner mark (a
/// `Button-IconOnly` with a context menu).
pub fn icon_dropdown(ui: &mut Ui, icon: &str, tooltip: &str, tint: Option<Color32>, size: Vec2, contents: impl FnOnce(&mut Ui)) -> Response {
    let response = icon_menu_button(ui, icon, tint, size, contents);
    response.on_hover_text(tooltip)
}

fn icon_menu_button(ui: &mut Ui, icon: &str, tint: Option<Color32>, size: Vec2, contents: impl FnOnce(&mut Ui)) -> Response {
    let color = tint.unwrap_or(theme::ICON_DEFAULT);
    let image = icons::image(icon, icons::DEFAULT_SIZE, color);
    let mut button = match image {
        Some(image) => Button::image(image),
        None => Button::new(icon.to_string()),
    };
    button = button.frame_when_inactive(false);
    button = button.min_size(size);
    button = button.corner_radius(4);
    let menu = MenuButton::from_button(button).config(menu_config());
    let scope = ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        widgets.hovered.weak_bg_fill = theme::MENU_BACKGROUND_HOVER;
        widgets.hovered.bg_stroke = Stroke::NONE;
        widgets.open.weak_bg_fill = theme::MENU_BACKGROUND_HOVER;
        widgets.open.bg_stroke = Stroke::NONE;
        widgets.active.bg_stroke = Stroke::NONE;
        let (response, _) = menu.ui(ui, |ui| {
            menu_style(ui);
            contents(ui);
        });
        response
    });
    scope.inner
}

/// Menus close on a click inside.
pub fn menu_config() -> MenuConfig {
    MenuConfig::default()
}

/// Inside a menu: no gaps between rows, at least 190 wide.
pub fn menu_style(ui: &mut Ui) {
    let style = ui.style_mut();
    style.visuals.override_text_color = Some(theme::MENU_FOREGROUND);
    style.spacing.item_spacing = Vec2::ZERO;
    style.spacing.button_padding = egui::vec2(2.0, 4.0);
    ui.set_min_width(MENU_MIN_WIDTH);
}

/// How a menu row's text is drawn.
#[derive(Clone, Copy, Default)]
struct RowText {
    /// The text's colour instead of `Menu-Foreground`.
    color: Option<Color32>,
    /// `Tag="HEADER"`: `Typography.Capitals="AllSmallCaps"`.
    small_caps: bool,
}

/// The row's text as laid out: small caps are capitals at 4/5 size.
fn row_galley(ui: &Ui, text: &str, style: RowText, color: Color32) -> std::sync::Arc<egui::Galley> {
    if style.small_caps {
        return ui.painter().layout_no_wrap(text.to_uppercase(), theme::interface(11.0 * 0.8), color);
    }
    ui.painter().layout_no_wrap(text.to_string(), theme::interface(11.0), color)
}

/// One menu row: the hover fill (inset 1, corners 4), the icon centred in
/// its column, the text 2 past it (11px, white when highlighted), the
/// shortcut in monospace 10 at the right and, for submenus, the arrow.
fn menu_row(ui: &mut Ui, icon: Option<(&str, Color32)>, text: &str, shortcut: &str, enabled: bool, arrow: bool, open: bool) -> Response {
    menu_row_styled(ui, icon, text, RowText::default(), shortcut, enabled, arrow, open)
}

#[allow(clippy::too_many_arguments)]
fn menu_row_styled(ui: &mut Ui, icon: Option<(&str, Color32)>, text: &str, style: RowText, shortcut: &str, enabled: bool, arrow: bool, open: bool) -> Response {
    let text_width = row_galley(ui, text, style, Color32::WHITE).size().x;
    let shortcut_width = if shortcut.is_empty() { 0.0 } else { ui.painter().layout_no_wrap(shortcut.to_string(), theme::monospace(10.0), Color32::WHITE).size().x + 2.0 + 8.0 };
    let content_width = MENU_ICON_COLUMN + 2.0 + text_width + 2.0 + shortcut_width.max(MENU_ARROW_COLUMN);
    let width = ui.available_width().max(content_width).max(MENU_MIN_WIDTH);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, MENU_ROW_HEIGHT), sense);
    let highlighted = enabled && (response.hovered() || open);
    if highlighted {
        ui.painter().rect_filled(rect.shrink(1.0), 4.0, theme::MENU_BACKGROUND_HOVER);
    }
    let opacity = if enabled { 1.0 } else { 0.5 };
    if let Some((name, color)) = icon {
        let icon_center = Pos2::new(rect.left() + MENU_ICON_COLUMN / 2.0, rect.center().y);
        let icon_rect = Rect::from_center_size(icon_center, Vec2::splat(icons::DEFAULT_SIZE));
        icons::paint(ui, icon_rect, name, color.gamma_multiply(opacity));
    }
    let plain_color = if highlighted { theme::MENU_FOREGROUND_HOVER } else { theme::MENU_FOREGROUND };
    let text_color = style.color.unwrap_or(plain_color).gamma_multiply(opacity);
    let galley = row_galley(ui, text, style, text_color);
    let text_position = Pos2::new(rect.left() + MENU_ICON_COLUMN + 2.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_position, galley, text_color);
    if !shortcut.is_empty() {
        let shortcut_color = theme::MENU_GESTURE.gamma_multiply(0.75 * opacity);
        let shortcut_galley = ui.painter().layout_no_wrap(shortcut.to_string(), theme::monospace(10.0), shortcut_color);
        let shortcut_position = Pos2::new(rect.right() - 8.0 - shortcut_galley.size().x, rect.center().y - shortcut_galley.size().y / 2.0);
        ui.painter().galley(shortcut_position, shortcut_galley, shortcut_color);
    }
    if arrow {
        let arrow_center = Pos2::new(rect.right() - MENU_ARROW_COLUMN / 2.0, rect.center().y);
        let arrow_rect = Rect::from_center_size(arrow_center, Vec2::splat(icons::DEFAULT_SIZE));
        icons::paint(ui, arrow_rect, "chevron-right", theme::MENU_ICON.gamma_multiply(opacity));
    }
    response
}

/// A menu entry: icon (in its accent colour), text and shortcut.
pub fn menu_item(ui: &mut Ui, icon: Option<&str>, accent: Option<Color32>, text: &str, shortcut: &str) -> Response {
    menu_item_enabled(ui, icon, accent, text, shortcut, true)
}

pub fn menu_item_enabled(ui: &mut Ui, icon: Option<&str>, accent: Option<Color32>, text: &str, shortcut: &str, enabled: bool) -> Response {
    let tint = accent.unwrap_or(theme::MENU_ICON);
    let icon = icon.map(|name| (name, tint));
    let response = menu_row(ui, icon, text, shortcut, enabled, false, false);
    if response.clicked() {
        ui.close();
    }
    response
}

/// A checkable entry (`IsCheckable`): checked, the check (`Accent-Blue`)
/// stands where the icon goes.
pub fn check_item(ui: &mut Ui, checked: bool, text: &str, shortcut: &str) -> Response {
    let icon = if checked { Some(("check", theme::ACCENT_BLUE)) } else { None };
    let response = menu_row(ui, icon, text, shortcut, true, false, false);
    if response.clicked() {
        ui.close();
    }
    response
}

/// A menu entry with an icon that may be checked (`IsChecked`): then the
/// check in `Accent-Blue` takes the icon's place. Returns whether it was
/// clicked.
pub fn checked_icon_item(ui: &mut Ui, checked: bool, icon: Option<&str>, accent: Option<Color32>, text: &str, shortcut: &str) -> bool {
    let (shown_icon, shown_accent) = if checked { (Some("check"), Some(theme::ACCENT_BLUE)) } else { (icon, accent) };
    menu_item(ui, shown_icon, shown_accent, text, shortcut).clicked()
}

/// A greyed caption inside a menu (`IsEnabled="False"` headers).
pub fn menu_caption(ui: &mut Ui, text: &str) {
    menu_row(ui, None, text, "", false, false, false);
}

/// A submenu entry (`SubmenuHeader`): the row with its arrow, opening
/// `contents` beside it.
pub fn submenu<R>(ui: &mut Ui, icon: Option<&str>, accent: Option<Color32>, text: &str, contents: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let next_id = ui.next_auto_id();
    let open = egui::Popup::is_id_open(ui.ctx(), SubMenu::id_from_widget_id(next_id));
    let tint = accent.unwrap_or(theme::MENU_ICON);
    let icon = icon.map(|name| (name, tint));
    let response = menu_row(ui, icon, text, "", true, true, open);
    let shown = SubMenu::new().show(ui, &response, |ui| {
        menu_style(ui);
        contents(ui)
    });
    shown.map(|inner| inner.inner)
}

/// A family header with its submenu (the toolbox's `Tag="HEADER"` items):
/// icon and small-caps text in `color`.
pub fn header_submenu<R>(ui: &mut Ui, icon: &str, color: Color32, text: &str, contents: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let next_id = ui.next_auto_id();
    let open = egui::Popup::is_id_open(ui.ctx(), SubMenu::id_from_widget_id(next_id));
    let style = RowText { color: Some(color), small_caps: true };
    let response = menu_row_styled(ui, Some((icon, color)), text, style, "", true, true, open);
    let shown = SubMenu::new().show(ui, &response, |ui| {
        menu_style(ui);
        contents(ui)
    });
    shown.map(|inner| inner.inner)
}

/// `MenuItem.SeparatorStyleKey`: a full-width 1px rule (`Menu-Separator`
/// at 0.2) with 1 above and below.
pub fn menu_separator(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 3.0), Sense::hover());
    let y = rect.center().y;
    let stroke = Stroke::new(1.0, theme::MENU_SEPARATOR.gamma_multiply(0.2));
    ui.painter().line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], stroke);
}

/// A dialog's `Button`, or with `important` its `Button-Important`.
pub fn dialog_button(ui: &mut Ui, text: &str, important: bool) -> Response {
    let label = RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_EMPHASIS);
    let border = if important { theme::BUTTON_IMPORTANT_BORDER } else { theme::BUTTON_BORDER };
    let mut widget = Button::new(label);
    widget = widget.fill(theme::BUTTON_BACKGROUND);
    widget = widget.stroke(Stroke::new(1.0, border));
    widget = widget.min_size(Vec2::new(80.0, DIALOG_BUTTON_HEIGHT));
    ui.add(widget)
}

/// `Button-Link`: blue text (darker blue under the pointer) with the hand
/// cursor, inside `padding`.
pub fn link_button(ui: &mut Ui, text: &str, font: FontId, padding: Vec2) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, theme::ACCENT_BLUE);
    let (rect, response) = ui.allocate_exact_size(galley.size() + padding * 2.0, Sense::click());
    let color = if response.hovered() { theme::ACCENT_STRONG_BLUE } else { theme::ACCENT_BLUE };
    ui.painter().galley(rect.min + padding, galley, color);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A rule across a vertical toolbar (`Separator` with margins `above` and
/// `below`, 2 in from each side).
pub fn toolbar_separator(ui: &mut Ui, above: f32, below: f32) {
    let width = ui.available_width() - 4.0;
    ui.add_space(above);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), Sense::hover());
    let stroke = Stroke::new(1.0, theme::MENU_SEPARATOR.gamma_multiply(0.5));
    ui.painter().line_segment([rect.left_center(), rect.right_center()], stroke);
    ui.add_space(below);
}

/// How an `EditablePopup`'s text box was left: Enter or Tab take the
/// text, Escape drops it, and a click elsewhere takes it only with Accept
/// values when clicking outside of editable field (`AcceptOnLostFocus`).
/// None while it still has the keyboard.
pub fn popup_outcome(ui: &Ui, response: &Response) -> Option<bool> {
    if !response.lost_focus() {
        return None;
    }
    let keys = ui.input(|input| (input.key_pressed(egui::Key::Enter), input.key_pressed(egui::Key::Tab), input.key_pressed(egui::Key::Escape)));
    let (entered, tabbed, escaped) = keys;
    if escaped {
        return Some(false);
    }
    if entered || tabbed {
        return Some(true);
    }
    Some(theme::settings(ui.ctx()).accept_on_lost_focus)
}
