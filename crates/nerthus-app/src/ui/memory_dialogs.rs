//! Two windows about the engine's work and memory:
//!
//! - Baking's progress (`ProgressDialog("Baking", …)`), shown while Bake
//!   Selected runs. Cancel only puts the window away; the bake carries on
//!   and still says when it is done.
//! - Optimize Memory, behind Preview › Clean Memory (`OptimizeMemory`): how
//!   much memory the engine holds, and four ways to give some back, after
//!   which Nerthus says how much the process shrank.

use egui::{Align, Align2, Id, Layout, RichText, Stroke, Vec2};

use crate::editor::Editor;
use crate::theme;
use crate::ui::commands::Command;

/// Baking's progress window while it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct BakeProgress {
    /// `label`: what is being baked.
    text: String,
}

/// The bake's first words: how many nodes the bake builds (the
/// chain less the nodes already baked) and at what size.
pub fn bake_progress(editor: &Editor) -> Option<BakeProgress> {
    let node = editor.active_node()?;
    let terrain = editor.document.terrain();
    let name = terrain.nodes.get(&node).map(|data| data.name.clone()).unwrap_or_default();
    let resolution = editor.document.build_definition().bake_resolution;
    let chain = nerthus_engine::Engine::build_order(terrain, node);
    let baked = |id: &i32| terrain.nodes.get(id).is_some_and(|data| nerthus_engine::StateOverride::of(data) == nerthus_engine::StateOverride::Baked);
    let count = chain.iter().filter(|id| !baked(id)).count();
    let text = if count == 0 {
        format!("{name} is being baked. Please wait...")
    } else {
        format!("Preparing to bake {count} nodes at {resolution} x {resolution}...")
    };
    Some(BakeProgress { text })
}

/// The window, while the bake runs and it has not been put away.
pub fn show_bake_progress(ctx: &egui::Context, editor: &Editor, state: &mut Option<BakeProgress>) {
    if !editor.baking {
        *state = None;
    }
    let Some(progress) = state.as_ref() else {
        return;
    };
    let mut cancelled = false;
    let mut area = egui::Area::new(Id::new("bake-progress"));
    area = area.order(egui::Order::Foreground);
    area = area.anchor(Align2::CENTER_CENTER, Vec2::ZERO);
    area.show(ctx, |ui| {
        let mut frame = egui::Frame::new();
        frame = frame.fill(theme::MENU_BACKGROUND);
        frame = frame.stroke(Stroke::new(1.5, theme::ACCENT_MUTED_PURPLE));
        frame = frame.corner_radius(6.0);
        frame = frame.inner_margin(16.0);
        frame.show(ui, |ui| {
            ui.set_min_width(480.0);
            ui.set_max_width(480.0);
            let title = RichText::new("Baking").font(theme::semibold(12.0)).color(theme::LABEL_EMPHASIS);
            ui.label(title);
            ui.add_space(4.0);
            let label = RichText::new(progress.text.as_str()).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT);
            ui.add(egui::Label::new(label).wrap());
            ui.add_space(8.0);
            let mut bar = egui::ProgressBar::new(editor.build_progress.max(0.05));
            bar = bar.desired_height(2.0);
            bar = bar.animate(true);
            ui.add(bar);
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                if ui.button("Cancel").clicked() {
                    cancelled = true;
                }
            });
        });
    });
    ctx.set_cursor_icon(egui::CursorIcon::Wait);
    ctx.request_repaint();
    if cancelled {
        *state = None;
    }
}

/// What Optimize Memory does (its command links).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// "Optimize RAM": unused memory handed back.
    OptimizeRam,
    /// "Maximum RAM Recovery": also the views' renders of nodes that are
    /// gone.
    Maximum,
    /// "Delete Viewport Cache": every render of the view, drawn again.
    ViewportCache,
    /// "Delete All Cache": every node's maps, rebuilt as needed.
    AllCache,
}

/// The dialog's figures, taken as it opens (`UpdateMemoryUsage`).
#[derive(Clone, Debug, PartialEq)]
pub struct OptimizeMemory {
    /// The process's memory as it opened (`memAll`).
    process: u64,
    cache: u64,
}

impl OptimizeMemory {
    pub fn open(editor: &Editor) -> OptimizeMemory {
        let process = crate::memory::process_bytes().unwrap_or(editor.cache_bytes as u64);
        OptimizeMemory { process, cache: editor.cache_bytes as u64 }
    }
}

/// A memory report waiting for its moment: memory is measured again 1.5 s
/// after the recovery.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingReport {
    /// When to measure (egui's clock).
    at: f64,
    before: u64,
    /// "Memory" after Optimize Memory, "Cache" after Clear Cache.
    clearing_cache: bool,
}

impl PendingReport {
    pub fn after(now: f64, clearing_cache: bool) -> PendingReport {
        let before = crate::memory::process_bytes().unwrap_or(0);
        PendingReport { at: now + 1.5, before, clearing_cache }
    }
}

/// `ByteSize.Humanize("0.0 GB")`.
fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64)
}

/// The dialog (a task dialog): a command link per recovery, and Cancel.
/// Returns the recovery chosen.
pub fn show_optimize_memory(ctx: &egui::Context, state: &mut Option<OptimizeMemory>) -> Option<Recovery> {
    let figures = state.as_ref()?;
    let mut chosen = None;
    let mut cancelled = false;
    let modal = egui::Modal::new(Id::new("optimize-memory"));
    let response = modal.show(ctx, |ui| {
        ui.set_width(460.0);
        ui.label(RichText::new("Optimize Memory").font(theme::interface(11.0)).color(theme::LABEL_DEFAULT));
        ui.add_space(6.0);
        let size = crate::ui::data_view::stats::byte_size(figures.process as usize);
        let heading = format!("{size} currently allocated to Nerthus's terrain engine.");
        ui.label(RichText::new(heading).font(theme::semibold(14.0)).color(theme::ACCENT_YELLOW));
        ui.add_space(6.0);
        let text = "You can force Nerthus's engine to release as much unused RAM as possible or clear various caches to free up memory.\n\nPlease note if you find yourself using this too frequently, you may want to Bake some nodes instead to free up memory.";
        ui.add(egui::Label::new(RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT)).wrap());
        ui.add_space(10.0);
        let links = [
            (Recovery::OptimizeRam, "Optimize RAM", "Recover RAM while preserving performance."),
            (Recovery::Maximum, "Maximum RAM Recovery", "Extended RAM recovery."),
            (Recovery::ViewportCache, "Delete Viewport Cache", "Clear viewport cache. Does not require rebuild."),
            (Recovery::AllCache, "Delete All Cache", "WARNING: Requires all nodes to be rebuilt."),
        ];
        for (recovery, title, note) in links {
            if command_link(ui, title, note).clicked() {
                chosen = Some(recovery);
            }
        }
        ui.add_space(8.0);
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if ui.button("Cancel").clicked() {
                cancelled = true;
            }
        });
        ui.add_space(6.0);
        ui.separator();
        let other = figures.process.saturating_sub(figures.cache);
        let statistics = format!(
            "Memory Statistics\n\nTotal Cache: {}\nOther Uses: {}\nTotal Memory: {}",
            gigabytes(figures.cache),
            gigabytes(other),
            gigabytes(figures.process)
        );
        ui.label(RichText::new(statistics).font(theme::interface(11.0)).color(theme::LABEL_DEFAULT));
    });
    if chosen.is_some() || cancelled || response.should_close() {
        *state = None;
    }
    chosen
}

/// A task dialog's command link: an arrow, the title and its note.
fn command_link(ui: &mut egui::Ui, title: &str, note: &str) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 44.0), egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 3.0, theme::BUTTON_HOVER);
        ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.0, theme::BUTTON_BORDER), egui::StrokeKind::Inside);
    }
    let arrow_rect = egui::Rect::from_min_size(egui::Pos2::new(rect.left() + 8.0, rect.top() + 6.0), Vec2::splat(16.0));
    crate::icons::paint(ui, arrow_rect, "arrow-right-square", theme::ACCENT_GREEN);
    let title_galley = ui.painter().layout_no_wrap(title.to_string(), theme::semibold(13.0), theme::LABEL_EMPHASIS);
    ui.painter().galley(egui::Pos2::new(arrow_rect.right() + 8.0, rect.top() + 5.0), title_galley, theme::LABEL_EMPHASIS);
    let note_galley = ui.painter().layout_no_wrap(note.to_string(), theme::interface(11.0), theme::LABEL_DEFAULT);
    ui.painter().galley(egui::Pos2::new(arrow_rect.right() + 8.0, rect.top() + 24.0), note_galley, theme::LABEL_DEFAULT);
    response
}

/// The word on what a recovery or a cleared cache freed, once its moment
/// comes.
pub fn report_freed(now: f64, pending: &mut Option<PendingReport>, editor: &mut Editor) {
    let Some(report) = pending.as_ref() else {
        return;
    };
    if now < report.at {
        return;
    }
    let after = crate::memory::process_bytes().unwrap_or(report.before);
    let freed = report.before.saturating_sub(after);
    let message = if freed > 5000 {
        let amount = crate::ui::data_view::stats::byte_size(freed as usize);
        if report.clearing_cache { format!("Cache cleared. Approximately {amount} freed.") } else { format!("Optimization complete. Approximately {amount} freed.") }
    } else {
        "Unable to optimize memory usage further. Try again later".to_string()
    };
    editor.report(message);
    *pending = None;
}

/// The commands a recovery runs, besides handing memory back.
pub fn recovery_commands(recovery: Recovery) -> Vec<Command> {
    match recovery {
        Recovery::OptimizeRam => vec![Command::ReleaseMemory],
        Recovery::Maximum | Recovery::ViewportCache => vec![Command::DropViewportCache, Command::ReleaseMemory],
        Recovery::AllCache => vec![Command::ClearCacheConfirmed, Command::ReleaseMemory],
    }
}
