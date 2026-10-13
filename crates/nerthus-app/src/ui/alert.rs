//! `Shell.ShowAlert`: a bar over the bottom left of the graph row (720
//! wide at most, 8 in) holding the alert's title in bold and its text,
//! filled and outlined in its type's colour, with a close button. It
//! fades in (0.15 s) and, unless told otherwise, fades out (0.5 s) after
//! 3.5 seconds (`DismissTimer`); a new alert takes the old one's place.

use egui::{Color32, Id, Pos2, Rect, Stroke, StrokeKind, UiBuilder, Vec2};

use crate::editor::Editor;
use crate::theme;
use crate::ui::widgets::icon_button;

/// `alert`'s `MaxWidth` and `Margin`.
const MAX_WIDTH: f32 = 720.0;
const MARGIN: f32 = 8.0;
/// `DismissTimer`'s interval, and the fades (`FadeInAndShow`,
/// `FadeOutAndCollapse`).
const DISMISS_SECONDS: f64 = 3.5;
const FADE_IN_SECONDS: f64 = 0.15;
const FADE_OUT_SECONDS: f64 = 0.5;
/// `btnDismissAlert`: a 32 icon button with a margin of 4.
const CLOSE_BUTTON: f32 = 32.0 + 8.0;

/// `AlertType`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AlertKind {
    Information,
    Warning,
    Error,
    Success,
}

impl AlertKind {
    /// The fill, the outline and the close button's colour
    /// (`Accent-Transparent-…`, `Accent-…`, `Accent-Strong-…`).
    fn colours(self) -> (Color32, Color32, Color32) {
        match self {
            AlertKind::Information => (theme::ACCENT_TRANSPARENT_BLUE, theme::ACCENT_BLUE, theme::ACCENT_STRONG_BLUE),
            AlertKind::Warning => (theme::ACCENT_TRANSPARENT_YELLOW, theme::ACCENT_YELLOW, theme::ACCENT_STRONG_YELLOW),
            AlertKind::Error => (theme::ACCENT_TRANSPARENT_RED, theme::ACCENT_RED, theme::ACCENT_STRONG_RED),
            AlertKind::Success => (theme::ACCENT_TRANSPARENT_GREEN, theme::ACCENT_GREEN, theme::ACCENT_STRONG_GREEN),
        }
    }
}

/// An alert while it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Alert {
    header: String,
    message: String,
    kind: AlertKind,
    auto_dismiss: bool,
    /// When it first showed, and when it started to fade out.
    shown_at: Option<f64>,
    hiding_since: Option<f64>,
}

impl Alert {
    pub fn new(header: &str, message: &str, kind: AlertKind) -> Self {
        Self { header: header.to_string(), message: message.to_string(), kind, auto_dismiss: true, shown_at: None, hiding_since: None }
    }

    /// `autoDismiss: false`: it stays until closed.
    pub fn staying(mut self) -> Self {
        self.auto_dismiss = false;
        self
    }
}

/// `CheckForOrphanVariables` (after several elements are deleted at once,
/// unless Options turns it off): the variables nothing is bound to, as a
/// warning.
pub fn orphan_variables_alert(editor: &Editor) -> Option<Alert> {
    let wanted = editor.user.preferences.member("", "ShowOrphanVariableWarning");
    if wanted.as_bool() == Some(false) {
        return None;
    }
    let orphans = editor.document.orphan_variables();
    if orphans.is_empty() {
        return None;
    }
    let names = orphans.join(",");
    let message = format!("{names}. You can disable this check in Options.");
    Some(Alert::new("Orphan Variables Detected", &message, AlertKind::Warning))
}

/// Draws the alert over `row` (the graph row) and forgets it once gone.
pub fn show(ctx: &egui::Context, row: Rect, alert: &mut Option<Alert>, animations: bool) {
    let Some(current) = alert.as_mut() else {
        return;
    };
    let now = ctx.input(|input| input.time);
    let shown_at = *current.shown_at.get_or_insert(now);
    if current.auto_dismiss && current.hiding_since.is_none() && now - shown_at >= DISMISS_SECONDS {
        current.hiding_since = Some(now);
    }
    let fade_in = if animations { ((now - shown_at) / FADE_IN_SECONDS).min(1.0) } else { 1.0 };
    let fade_out = match current.hiding_since {
        Some(since) if animations => 1.0 - ((now - since) / FADE_OUT_SECONDS).min(1.0),
        Some(_) => 0.0,
        None => 1.0,
    };
    if fade_out <= 0.0 {
        *alert = None;
        return;
    }
    let opacity = (fade_in * fade_out) as f32;
    let (fill, outline, close_colour) = current.kind.colours();

    // The text wraps at the widest the bar may be, less the margins and
    // the close button.
    let width_limit = (row.width() - 2.0 * MARGIN).min(MAX_WIDTH);
    let text_limit = (width_limit - 16.0 - CLOSE_BUTTON - 4.0).max(64.0);
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = text_limit;
    let title_format = egui::TextFormat::simple(theme::semibold(12.0), theme::LABEL_EMPHASIS);
    job.append(&current.header, 0.0, title_format);
    let text_format = egui::TextFormat::simple(theme::interface(12.0), theme::LABEL_EMPHASIS);
    job.append("   ", 0.0, text_format.clone());
    job.append(&current.message, 0.0, text_format);
    let galley = ctx.fonts_mut(|fonts| fonts.layout_job(job));
    let height = galley.size().y.max(CLOSE_BUTTON) + 16.0;
    let width = 16.0 + galley.size().x + CLOSE_BUTTON + 4.0;
    let bar = Rect::from_min_size(Pos2::new(row.left() + MARGIN, row.bottom() - MARGIN - height), Vec2::new(width, height));

    let mut dismissed = false;
    let area = egui::Area::new(Id::new("shell-alert")).fixed_pos(bar.min).order(egui::Order::Middle);
    area.show(ctx, |ui| {
        ui.set_opacity(opacity);
        let painter = ui.painter();
        painter.rect_filled(bar, 3.0, theme::WINDOW_DARK);
        painter.rect_filled(bar, 2.0, fill);
        painter.rect_stroke(bar, 2.0, Stroke::new(1.0, outline), StrokeKind::Inside);
        let text_top = bar.center().y - galley.size().y / 2.0;
        painter.galley(Pos2::new(bar.left() + 16.0, text_top), galley.clone(), theme::LABEL_EMPHASIS);
        let close = Rect::from_min_size(Pos2::new(bar.right() - 4.0 - CLOSE_BUTTON + 4.0, bar.top() + 8.0 + 4.0), Vec2::splat(32.0));
        let mut close_ui = ui.new_child(UiBuilder::new().max_rect(close));
        if icon_button(&mut close_ui, "close-x", "", Some(close_colour)).clicked() {
            dismissed = true;
        }
        // The bar takes the pointer over the graph.
        ui.allocate_rect(bar, egui::Sense::hover());
    });
    if dismissed && current.hiding_since.is_none() {
        current.hiding_since = Some(now);
    }
    ctx.request_repaint();
}
