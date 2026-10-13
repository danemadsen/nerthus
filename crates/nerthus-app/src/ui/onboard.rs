//! The first-run setup (`Onboard`): shown when the data folder has no
//! `onboard.dat`, or with `--onboard`, before the editor is used. Three
//! pages: where documents, projects and builds go; the navigation and
//! toolbox styles; and what the compute devices said. "Let's get
//! started!" keeps the choices, takes them at once and writes
//! `onboard.dat`. `--skip-onboard` writes the file without asking, and
//! `--screenshot` runs skip the window.

use std::path::Path;

use crossbeam_channel::{Receiver, TryRecvError};
use egui::{Align, Color32, Layout, Rect, RichText, Sense, Stroke, Ui, Vec2};
use serde_json::{json, Value};

use crate::editor::Editor;
use crate::theme;
use crate::ui::file_dialogs::{DialogKind, DialogRequest, FileDialogs};
use crate::ui::help_windows::{self, SideTab, ACCENT_ORANGE};
use crate::user::ToolboxStyle;

/// The file whose presence says the setup is done.
pub const ONBOARD_FILE: &str = "onboard.dat";

const TABS: [SideTab; 3] = [
    SideTab { title: "Project Paths", brush: theme::ACCENT_PRIMARY },
    SideTab { title: "Preferences", brush: theme::ACCENT_MUTED_TEAL },
    SideTab { title: "GPU Compute", brush: ACCENT_ORANGE },
];

/// The folders the first page can change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Folder {
    Documents,
    Projects,
    Builds,
}

impl Folder {
    fn member(self) -> &'static str {
        match self {
            Folder::Documents => "Documents",
            Folder::Projects => "Projects",
            Folder::Builds => "Builds",
        }
    }

    fn dialog_key(self) -> String {
        format!("onboard-{}", self.member())
    }

    /// The folder dialog's description.
    fn description(self) -> &'static str {
        match self {
            Folder::Documents => "Select a folder to store Nerthus User Files.",
            Folder::Projects => "Select a folder to store your projects.",
            Folder::Builds => "Select a folder to store your Build Output.",
        }
    }
}

/// What the compute devices said (`Devices.txt`).
enum GpuCheck {
    Running(Receiver<String>),
    Done { usable: bool, text: String },
}

/// The window while it shows.
pub struct Onboard {
    tab: usize,
    /// `btnLetsGo`'s text.
    next_label: &'static str,
    /// "Place PROJECTS and BUILDS in different locations."
    separate_folders: bool,
    /// `lstNavStyle`: Nerthus Defaults or Industry Standard (chosen at
    /// first).
    navigation: usize,
    /// `lstToolboxStyle`: Docked Panel or Compact Buttons (chosen at
    /// first).
    toolbox: usize,
    gpu: GpuCheck,
    /// A folder under Nerthus's own, refused.
    refused: bool,
}

/// Whether the setup shows as Nerthus starts; with `--skip-onboard` it is
/// marked done instead (`App.ShowOnboarding`).
pub fn should_show(forced: bool, skipped: bool, screenshot: bool) -> bool {
    let marker = crate::user::data_folder().map(|folder| folder.join(ONBOARD_FILE));
    let done = marker.as_ref().is_some_and(|marker| marker.exists());
    if skipped {
        if !done {
            mark_done();
        }
        return false;
    }
    if forced {
        return true;
    }
    !done && !screenshot
}

/// Writes `onboard.dat`.
fn mark_done() {
    let Some(folder) = crate::user::data_folder() else {
        return;
    };
    let written = std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(folder.join(ONBOARD_FILE), "!"));
    if let Err(error) = written {
        log::warn!("cannot write {ONBOARD_FILE}: {error}");
    }
}

impl Onboard {
    /// `Onboard_Loaded`: the devices checked in the background.
    pub fn open(cpu_only: bool) -> Onboard {
        let gpu = if cpu_only {
            let text = "Nerthus is in CPU-only mode. You can turn it off in Options › Compute.".to_string();
            GpuCheck::Done { usable: false, text }
        } else {
            start_gpu_check()
        };
        Onboard {
            tab: 0,
            next_label: "Next: Select your Preferences",
            separate_folders: false,
            navigation: 1,
            toolbox: 1,
            gpu,
            refused: false,
        }
    }

    /// `--show onboard:<page>`: the setup on the page named (`paths`,
    /// `preferences` or `gpu`).
    pub fn open_at(page: &str) -> Onboard {
        let mut onboard = Onboard::open(nerthus_compute::is_cpu_only());
        onboard.tab = match page {
            "preferences" => 1,
            "gpu" => 2,
            _ => 0,
        };
        onboard.next_label = next_label(onboard.tab);
        onboard
    }

    fn poll_gpu(&mut self) {
        let GpuCheck::Running(receiver) = &self.gpu else {
            return;
        };
        let lines = match receiver.try_recv() {
            Ok(lines) => lines,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => String::new(),
        };
        self.gpu = gpu_answer(&lines);
    }
}

/// On a worker thread: on Windows, the NVIDIA profile when the NVIDIA
/// driver is there; then "Evaluating devices...": the devices listed. It
/// sends what they had to say about the ones they cannot use.
fn start_gpu_check() -> GpuCheck {
    let (sender, receiver) = crossbeam_channel::bounded(1);
    let spawned = std::thread::Builder::new().name("onboard devices".to_string()).spawn(move || {
        if cfg!(windows) {
            log::info!("Creating NVidia profiles...");
            if let Err(nerthus_compute::ProfileError::Failed(_)) = nerthus_compute::enforce_optimal_settings() {
                log::error!("NV Failed.");
            }
        }
        log::info!("Evaluating devices...");
        let (_, problems) = nerthus_compute::available_devices();
        let _ = sender.send(problems);
    });
    match spawned {
        Ok(_) => GpuCheck::Running(receiver),
        Err(error) => GpuCheck::Done { usable: false, text: error.to_string() },
    }
}

/// No problems: every device can compute. Otherwise the last device that
/// had one is named.
fn gpu_answer(problems: &str) -> GpuCheck {
    let Some(last) = problems.lines().rev().find(|line| !line.trim().is_empty()) else {
        let text = "All devices enabled for Compute. You can edit them anytime in Options.".to_string();
        return GpuCheck::Done { usable: true, text };
    };
    let device = last.split_once(": ").map(|(device, _)| device).unwrap_or(last).trim();
    let text = format!("Your CPU or GPU drivers may be out of date. We recommend updating them before starting Nerthus.\n\nPossible driver issue in: {device}");
    GpuCheck::Done { usable: false, text }
}

/// `Tabs_OnSelectionChanged`: what the button says on each page.
fn next_label(tab: usize) -> &'static str {
    match tab {
        0 => "Next: Select your preferences",
        1 => "Next: Select your GPU options",
        _ => "Let's get started!",
    }
}

/// The window. Returns whether it is still open.
pub fn show(ctx: &egui::Context, editor: &mut Editor, onboard: &mut Onboard, dialogs: &mut FileDialogs) -> bool {
    onboard.poll_gpu();
    if matches!(onboard.gpu, GpuCheck::Running(_)) {
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
    take_folder_answers(editor, onboard, dialogs);
    let mut finished = false;
    // The full window covers the editor, which waits behind it.
    let screen = ctx.content_rect();
    let mut veil = egui::Area::new(egui::Id::new("onboard-veil"));
    veil = veil.order(egui::Order::Middle);
    veil = veil.fixed_pos(screen.min);
    veil.show(ctx, |ui| {
        let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
        ui.painter().rect_filled(rect, 0.0, Color32::from_black_alpha(160));
    });
    let window = help_windows::dialog_window(ctx, "onboard", Vec2::new(1240.0, 720.0));
    window.show(ctx, |ui| {
        let area = ui.max_rect();
        ui.set_min_size(area.size());
        paint_backdrop(ui, area);

        // The welcome (margin 72, 64).
        let welcome = Rect::from_min_size(area.min + Vec2::new(72.0, 64.0), Vec2::new(area.width() - 144.0, 80.0));
        let mut welcome_ui = region(ui, welcome, Layout::top_down(Align::Min));
        welcome_ui.label(RichText::new("Welcome to Nerthus!").font(theme::interface(32.0)).color(Color32::WHITE));
        let subtitle = "Before you get started, you can quickly set up your preferred workspace.";
        welcome_ui.label(RichText::new(subtitle).font(theme::interface(14.0)).color(theme::LABEL_DEFAULT));

        // The tabs (margin 72, 160, 20, 90): headers on the left, the page
        // beside them.
        let body = Rect::from_min_max(area.min + Vec2::new(72.0, 160.0), area.max - Vec2::new(20.0, 90.0));
        let headers = Rect::from_min_size(body.min, Vec2::new(200.0, body.height()));
        let mut headers_ui = region(ui, headers, Layout::top_down(Align::Min));
        if help_windows::side_tabs(&mut headers_ui, &TABS, &mut onboard.tab, 200.0) {
            onboard.next_label = next_label(onboard.tab);
        }
        // The tab's margin 24 on the right, the page's 8 and padding 8.
        let page_min = egui::pos2(headers.right() + 24.0 + 16.0, body.top() + 8.0);
        let page_rect = Rect::from_min_max(page_min, body.max - Vec2::splat(8.0));
        let mut page_ui = region(ui, page_rect, Layout::top_down(Align::Min));
        match onboard.tab {
            0 => paths_page(&mut page_ui, editor, onboard, dialogs),
            1 => preferences_page(&mut page_ui, onboard),
            _ => gpu_page(&mut page_ui, onboard),
        }

        // "You can change any choices…" (margin 32, 0, 0, 32) and the
        // button (margin 0, 0, 64, 48).
        let note = "You can change any choices in the Options dialog later.";
        let note_galley = ui.painter().layout_no_wrap(note.to_string(), theme::interface(12.0), theme::LABEL_DEFAULT);
        let note_position = egui::pos2(area.left() + 32.0, area.bottom() - 32.0 - note_galley.size().y);
        ui.painter().galley(note_position, note_galley, theme::LABEL_DEFAULT);
        let button_corner = area.right_bottom() - Vec2::new(64.0, 48.0);
        let button_rect = Rect::from_min_max(button_corner - Vec2::new(400.0, 48.0), button_corner);
        let mut button_ui = region(ui, button_rect, Layout::right_to_left(Align::Center));
        let next = help_windows::important_button(&mut button_ui, onboard.next_label, 14.0, 0.0, Vec2::new(24.0, 12.0));
        if next.clicked() {
            if onboard.tab + 1 < TABS.len() {
                onboard.tab += 1;
                onboard.next_label = next_label(onboard.tab);
            } else {
                finished = true;
            }
        }
    });
    refusal(ctx, onboard);
    if finished {
        finish(editor, onboard);
        return false;
    }
    true
}

/// A part of the window laid out on its own.
fn region(ui: &mut Ui, rect: Rect, layout: Layout) -> Ui {
    let builder = egui::UiBuilder::new().max_rect(rect).layout(layout);
    ui.new_child(builder)
}

/// The faint logo behind the pages (at 5% opacity, running off the bottom
/// right).
fn paint_backdrop(ui: &Ui, area: Rect) {
    let logo_corner = area.min + Vec2::new(area.width() * 0.45, 95.0);
    let logo_rect = Rect::from_min_size(logo_corner, Vec2::splat(area.height() * 1.1));
    let logo = egui::Image::new(egui::include_image!("../../assets/nerthus.svg"));
    let faded = logo.tint(Color32::from_white_alpha(13));
    faded.paint_at(ui, logo_rect);
}

/// A path as the link buttons show it (`Button-Link`, monospace 16).
fn folder_link(ui: &mut Ui, path: &Path, color: Color32) -> bool {
    let text = RichText::new(path.display().to_string()).font(theme::monospace(16.0)).color(color);
    let response = ui.add(egui::Label::new(text).sense(Sense::click()));
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A paragraph under a setting (13 px, margin 24, 12, 360, 0).
fn paragraph(ui: &mut Ui, text: &str) {
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(24.0);
        let width = (ui.available_width() - 360.0).max(240.0);
        ui.allocate_ui_with_layout(Vec2::new(width, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.add(egui::Label::new(RichText::new(text).font(theme::interface(13.0)).color(theme::LABEL_DEFAULT)).wrap());
        });
    });
}

fn heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(theme::interface(14.0)).color(theme::LABEL_DEFAULT));
}

/// Project Paths.
fn paths_page(ui: &mut Ui, editor: &mut Editor, onboard: &mut Onboard, dialogs: &mut FileDialogs) {
    let preferences = &editor.user.preferences;
    let documents = preferences.documents_folder();
    let projects = preferences.projects_folder();
    let builds = preferences.builds_folder();
    heading(ui, "Nerthus Storage Folder");
    if folder_link(ui, &documents, theme::ACCENT_MUTED_GREEN) {
        ask_folder(dialogs, Folder::Documents, &documents);
    }
    paragraph(ui, "This is where all Presets, Screenshots, and other Nerthus documents are stored.");
    ui.add_space(24.0);
    let label = RichText::new("Place PROJECTS and BUILDS in different locations.").font(theme::interface(14.0));
    ui.checkbox(&mut onboard.separate_folders, label);
    if onboard.separate_folders {
        ui.add_space(36.0);
        heading(ui, "Projects");
        if folder_link(ui, &projects, theme::ACCENT_MUTED_BLUE) {
            ask_folder(dialogs, Folder::Projects, &projects);
        }
        paragraph(ui, "This is where all Nerthus project files are stored by default.");
        ui.add_space(36.0);
        heading(ui, "Build Output");
        if folder_link(ui, &builds, theme::ACCENT_GRAY) {
            ask_folder(dialogs, Folder::Builds, &builds);
        }
    }
    paragraph(ui, "Projects are usually small, lightweight files. However, Builds are very large and heavy. Keeping them separate is recommended if you frequently backup either folder.");
}

fn ask_folder(dialogs: &mut FileDialogs, folder: Folder, current: &Path) {
    let request = DialogRequest {
        key: folder.dialog_key(),
        title: folder.description().to_string(),
        kind: DialogKind::PickFolder,
        directory: Some(current.to_path_buf()),
    };
    dialogs.ask(request);
}

/// The folders picked: refused under Nerthus's own folder; the storage
/// folder takes `Projects` and `Builds` with it unless they are kept apart
/// (`BtnFolder_OnClick`).
fn take_folder_answers(editor: &mut Editor, onboard: &mut Onboard, dialogs: &mut FileDialogs) {
    for folder in [Folder::Documents, Folder::Projects, Folder::Builds] {
        let Some(Some(picked)) = dialogs.take(&folder.dialog_key()) else {
            continue;
        };
        if under_installation(&picked) {
            onboard.refused = true;
            continue;
        }
        let preferences = &mut editor.user.preferences;
        preferences.set_member("Locations", folder.member(), path_value(&picked));
        if folder == Folder::Documents && !onboard.separate_folders {
            preferences.set_member("Locations", "Projects", path_value(&picked.join("Projects")));
            preferences.set_member("Locations", "Builds", path_value(&picked.join("Builds")));
        }
    }
}

fn path_value(path: &Path) -> Value {
    json!(path.to_string_lossy())
}

/// Whether the folder is in Nerthus's installation folder (the
/// executable's), compared without case.
fn under_installation(picked: &Path) -> bool {
    let Ok(program) = std::env::current_exe() else {
        return false;
    };
    let Some(installation) = program.parent() else {
        return false;
    };
    let picked_text = picked.to_string_lossy().to_lowercase();
    let installation_text = installation.to_string_lossy().to_lowercase();
    picked_text.starts_with(&installation_text)
}

/// "Invalid Location".
fn refusal(ctx: &egui::Context, onboard: &mut Onboard) {
    if !onboard.refused {
        return;
    }
    let modal = egui::Modal::new(egui::Id::new("onboard-invalid-location"));
    let response = modal.show(ctx, |ui| {
        ui.set_width(380.0);
        ui.label(RichText::new("Invalid Location").font(theme::semibold(14.0)).color(theme::ACCENT_MUTED_BLUE));
        ui.add_space(8.0);
        ui.label("You cannot place Nerthus Projects or User Files under the Nerthus Installation Folder.");
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if ui.button("OK").clicked() {
                onboard.refused = false;
            }
        });
    });
    if response.should_close() {
        onboard.refused = false;
    }
}

/// A `ListBox-OptionsX` item: a bold first line and a plain second, the
/// selected one on the selection brush.
fn option_item(ui: &mut Ui, size: Vec2, title: &str, detail: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 2.0, theme::OPTIONS_SELECTION);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 2.0, theme::OPTIONS_SELECTION.gamma_multiply(0.4));
    }
    let color = if selected || response.hovered() { theme::LABEL_EMPHASIS } else { theme::LABEL_DEFAULT };
    let title_galley = ui.painter().layout_no_wrap(title.to_string(), theme::semibold(14.0), color);
    let detail_galley = ui.painter().layout_no_wrap(detail.to_string(), theme::interface(14.0), color);
    let height = title_galley.size().y + detail_galley.size().y;
    let top = rect.center().y - height / 2.0;
    let title_position = egui::pos2(rect.center().x - title_galley.size().x / 2.0, top);
    let detail_position = egui::pos2(rect.center().x - detail_galley.size().x / 2.0, top + title_galley.size().y);
    ui.painter().galley(title_position, title_galley, color);
    ui.painter().galley(detail_position, detail_galley, color);
    response
}

/// Preferences: the navigation and toolbox styles.
fn preferences_page(ui: &mut Ui, onboard: &mut Onboard) {
    heading(ui, "Navigation Style");
    ui.add_space(8.0);
    let navigation_choices = [
        ("Nerthus Defaults", "LMB Orbit / MMB Pan / RMB Zoom"),
        ("Industry Standard", "Alt+LMB Orbit / Alt+MMB Pan / Alt+RMB Zoom"),
    ];
    let list_width = ui.available_width().min(840.0);
    list_frame(ui, |ui| {
        for (index, (title, detail)) in navigation_choices.iter().enumerate() {
            let size = Vec2::new((list_width - 8.0) / 2.0, 56.0);
            if option_item(ui, size, title, detail, onboard.navigation == index).clicked() {
                onboard.navigation = index;
            }
        }
    });
    ui.add_space(24.0);
    heading(ui, "Toolbox Style");
    ui.add_space(8.0);
    list_frame(ui, |ui| {
        let styles = [
            ("Docked Panel", "Recommended for Beginners", ToolboxStyle::Panel),
            ("Compact Buttons", "For experts and reduced UI space", ToolboxStyle::Menus),
        ];
        for (index, (title, detail, style)) in styles.into_iter().enumerate() {
            if toolbox_item(ui, title, detail, style, onboard.toolbox == index).clicked() {
                onboard.toolbox = index;
            }
        }
    });
}

/// `ListBox-OptionsX`'s box: its items in a row.
fn list_frame(ui: &mut Ui, contents: impl FnOnce(&mut Ui)) {
    let mut frame = egui::Frame::new();
    frame = frame.fill(theme::OPTIONS_BACKGROUND);
    frame = frame.stroke(Stroke::new(1.0, theme::OPTIONS_BORDER));
    frame = frame.corner_radius(4.0);
    frame = frame.inner_margin(egui::Margin::symmetric(2, 1));
    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            contents(ui);
        });
    });
}

/// A toolbox style: its name over a sketch of it, bordered in teal when
/// chosen.
fn toolbox_item(ui: &mut Ui, title: &str, detail: &str, style: ToolboxStyle, selected: bool) -> egui::Response {
    let size = Vec2::new(340.0, 200.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let border = if selected { theme::ACCENT_TEAL } else { Color32::TRANSPARENT };
    if response.hovered() && !selected {
        ui.painter().rect_filled(rect, 4.0, theme::OPTIONS_SELECTION.gamma_multiply(0.4));
    }
    ui.painter().rect_stroke(rect.shrink(1.0), 4.0, Stroke::new(2.0, border), egui::StrokeKind::Inside);
    let color = if selected || response.hovered() { theme::LABEL_EMPHASIS } else { theme::LABEL_DEFAULT };
    let title_galley = ui.painter().layout_no_wrap(title.to_string(), theme::semibold(14.0), color);
    let detail_galley = ui.painter().layout_no_wrap(detail.to_string(), theme::interface(14.0), color);
    let text_left = rect.left() + 10.0;
    let title_height = title_galley.size().y;
    ui.painter().galley(egui::pos2(text_left, rect.top() + 10.0), title_galley, color);
    ui.painter().galley(egui::pos2(text_left, rect.top() + 10.0 + title_height), detail_galley, color);
    let sketch = Rect::from_min_max(egui::pos2(rect.left() + 10.0, rect.top() + 60.0), rect.max - Vec2::splat(10.0));
    paint_toolbox_sketch(ui, sketch, style);
    response
}

/// A sketch of the graph area with the toolbox docked as a panel of node
/// rows, or as a strip of buttons.
fn paint_toolbox_sketch(ui: &Ui, rect: Rect, style: ToolboxStyle) {
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, theme::WINDOW_BACKGROUND);
    let line = Stroke::new(1.0, theme::BUTTON_BORDER);
    match style {
        ToolboxStyle::Panel => {
            let panel = Rect::from_min_size(rect.min + Vec2::splat(6.0), Vec2::new(90.0, rect.height() - 12.0));
            painter.rect_filled(panel, 2.0, theme::PANEL_BACKGROUND);
            for row in 0..8 {
                let top = panel.top() + 6.0 + row as f32 * 15.0;
                if top + 9.0 > panel.bottom() {
                    break;
                }
                let swatch = Rect::from_min_size(egui::pos2(panel.left() + 6.0, top), Vec2::splat(9.0));
                painter.rect_filled(swatch, 2.0, theme::toolbox_color(TOOLBOX_SKETCH[row % TOOLBOX_SKETCH.len()]));
                let bar = Rect::from_min_size(egui::pos2(swatch.right() + 6.0, top + 2.0), Vec2::new(54.0, 5.0));
                painter.rect_filled(bar, 2.0, theme::LABEL_DEFAULT.gamma_multiply(0.5));
            }
            let node = Rect::from_center_size(egui::pos2(rect.center().x + 50.0, rect.center().y), Vec2::new(60.0, 36.0));
            painter.rect_stroke(node, 3.0, line, egui::StrokeKind::Inside);
        }
        ToolboxStyle::Menus => {
            let strip = Rect::from_min_size(rect.min + Vec2::splat(6.0), Vec2::new(rect.width() - 12.0, 22.0));
            painter.rect_filled(strip, 2.0, theme::PANEL_BACKGROUND);
            for button in 0..10 {
                let left = strip.left() + 6.0 + button as f32 * 20.0;
                if left + 14.0 > strip.right() {
                    break;
                }
                let icon = Rect::from_min_size(egui::pos2(left, strip.top() + 4.0), Vec2::splat(14.0));
                painter.rect_filled(icon, 3.0, theme::toolbox_color(TOOLBOX_SKETCH[button % TOOLBOX_SKETCH.len()]));
            }
            let node = Rect::from_center_size(egui::pos2(rect.center().x, rect.center().y + 14.0), Vec2::new(60.0, 36.0));
            painter.rect_stroke(node, 3.0, line, egui::StrokeKind::Inside);
        }
    }
}

/// Toolbox categories whose colours the sketches use.
const TOOLBOX_SKETCH: [&str; 5] = ["Primitive", "Terrain", "Simulate", "Surface", "Data"];

/// GPU Compute: what the devices said; the box is for show.
fn gpu_page(ui: &mut Ui, onboard: &Onboard) {
    let (usable, text) = match &onboard.gpu {
        GpuCheck::Running(_) => (false, "Determining hardware..."),
        GpuCheck::Done { usable, text } => (*usable, text.as_str()),
    };
    let mut checked = usable;
    let label = RichText::new("Use GPU for compute").font(theme::interface(14.0));
    let checkbox = egui::Checkbox::new(&mut checked, label);
    ui.add_enabled(false, checkbox);
    ui.add_space(24.0);
    ui.add(egui::Label::new(RichText::new(text).font(theme::interface(14.0)).color(theme::ACCENT_GRAY)).wrap());
}

/// `btnLetsGo_Click` on the last page: Industry Standard turns on the Alt
/// navigation, Docked Panel the panel toolbox; the settings are saved and
/// `onboard.dat` written.
fn finish(editor: &mut Editor, onboard: &Onboard) {
    let options = &mut editor.user.options;
    if onboard.navigation == 1 {
        options.set_value("AltPan", Value::Bool(true));
        options.set_value("AltZoom", Value::Bool(true));
        options.set_value("AltRotate", Value::Bool(true));
    }
    if onboard.toolbox == 0 {
        options.set_toolbox_style(ToolboxStyle::Panel);
    }
    options.mark_changed();
    editor.user.preferences.mark_changed();
    editor.user.save_if_changed();
    mark_done();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_without_problems_are_all_usable() {
        let answer = gpu_answer("");
        assert!(matches!(answer, GpuCheck::Done { usable: true, .. }));
    }

    #[test]
    fn the_last_problem_names_its_device() {
        let answer = gpu_answer("A (Vulkan): too old\nB (Metal): no float atomics\n");
        let GpuCheck::Done { usable, text } = answer else {
            panic!("still running");
        };
        assert!(!usable);
        assert!(text.ends_with("Possible driver issue in: B (Metal)"));
    }

    #[test]
    fn the_button_reads_as_the_page_needs() {
        assert_eq!(next_label(0), "Next: Select your preferences");
        assert_eq!(next_label(2), "Let's get started!");
    }
}
