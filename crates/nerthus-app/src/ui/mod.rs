//! The editor window (`Shell`): the title bar on top,
//! the status bar below, and between them the viewport over the graph on
//! the left (five parts) and the property area on the right (one part),
//! with splitters between them.

pub(crate) mod alert;
mod artifact_explorer;
pub mod batch_build;
mod bound_variable;
mod build_settings;
mod build_window;
mod colour_picker;
mod command_line_window;
mod commands;
mod console_window;
mod constructor;
mod create_connection;
mod data_view;
pub mod diagnostics_window;
mod docs_window;
pub(crate) mod editors;
mod expression_dialog;
mod file_commands;
pub mod file_dialogs;
pub(crate) mod file_windows;
pub mod floating_viewport;
mod formulas;
mod gizmos;
mod gradient_editor;
mod graph_area;
pub mod graph_canvas;
mod graph_commands;
mod graph_groups;
mod graph_notes;
mod graph_popups;
mod graph_search;
mod help_windows;
mod input_box;
mod markdown;
mod math_editor;
mod memory_dialogs;
mod menus;
mod mixer_layers;
mod modifier_list;
mod nahimic_warning;
mod node_states;
mod note_editor;
pub mod onboard;
pub mod options_pages;
mod options_window;
mod painter;
mod presets;
mod progress_window;
mod prompt;
mod property_area;
mod regions;
pub(crate) mod slider_popups;
mod statistics_window;
mod status_bar;
mod syntax_box;
mod title_bar;
mod tool_commands;
mod tool_window;
pub(crate) mod toolbox;
mod turntable_window;
mod variable_dialog;
pub mod viewport_2d;
mod viewport_area;
mod widgets;

pub(crate) use data_view::stats::byte_size;

use std::panic::AssertUnwindSafe;

use bevy::camera::Viewport;
use bevy::diagnostic::{DiagnosticsStore, SystemInformationDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::{egui, EguiContexts};
use egui::{CursorIcon, Key, KeyboardShortcut, LayerId, Modifiers, Pos2, Rect, Sense, Ui, UiBuilder, Vec2};
use bevy::ecs::system::SystemParam;
use bevy::math::URect;
use nerthus_render::{CameraRequest, NavigationSettings, Screenshots, StrategyCamera, ViewportCamera, ViewportInput, ViewportSettings};

use crate::document::CombineKind;
use crate::editor::Editor;
use crate::theme;
use crate::user::UserOptions;
use crate::viewport_render::{MaskGradient, RenderQuality};
use commands::Command;
use graph_canvas::GraphState;
use graph_commands::GraphCommand;

/// Interface state kept between frames.
#[derive(Resource)]
pub struct UiState {
    pub graph: GraphState,
    /// The 2D viewport, docked and floating.
    pub viewport_2d: viewport_2d::Viewport2DState,
    /// The Data View beside the graph.
    pub data_view: data_view::DataViewState,
    /// The left column's share of the width (`Root` columns `5*` and `1*`).
    pub split_x: f32,
    /// The viewport's share of the left column's height (rows `*` and `*`).
    pub split_y: f32,
    pub maximized: bool,
    pub sun_open: bool,
    /// The Build and Export confirmation, while it shows.
    pub build_confirm: Option<build_window::BuildConfirm>,
    /// Build Settings, while open.
    pub build_settings: Option<build_settings::BuildSettingsState>,
    /// The Batch Build Manager, while open, and what it keeps until
    /// Nerthus closes.
    pub batch_build: Option<batch_build::BatchWindow>,
    pub batch_session: batch_build::BatchSession,
    /// File dialogs waiting for the main thread, and their answers.
    pub file_dialogs: file_dialogs::FileDialogs,
    /// Baking's progress window, Optimize Memory, and what a recovery
    /// freed, said once measured.
    pub bake_progress: Option<memory_dialogs::BakeProgress>,
    pub optimize_memory: Option<memory_dialogs::OptimizeMemory>,
    pub memory_report: Option<memory_dialogs::PendingReport>,
    /// The interface's clock (egui's time) as of this frame.
    pub clock: f64,
    /// A Yes/No question (`MBox`), while open.
    pub prompt: Option<prompt::Prompt>,
    /// The alert over the graph row (`ShowAlert`), while it shows.
    pub alert: Option<alert::Alert>,
    /// Edit Variable, while open.
    pub variable_dialog: Option<variable_dialog::EditVariable>,
    /// Edit Expression (a parameter's Expression…), while open.
    pub expression_dialog: Option<expression_dialog::ExpressionDialog>,
    /// Record Turntable Animation, while open, and what it remembers for
    /// the project.
    pub turntable: Option<turntable_window::TurntableWindow>,
    pub turntable_memory: turntable_window::TurntableMemory,
    /// Construction's Constructor, while open.
    pub constructor: Option<constructor::ConstructorWindow>,
    /// The Data View's Build Command Line and Build Statistics windows,
    /// while open.
    pub command_line: Option<command_line_window::CommandLineWindow>,
    pub statistics: Option<statistics_window::StatisticsWindow>,
    /// Options, while open, and the page it showed last (`LastOptionsTab`).
    pub options: Option<options_window::OptionsState>,
    pub last_options_tab: options_window::OptionsTab,
    /// `--show`, opened once the options are in.
    pub launch_show: Option<String>,
    /// Tools' windows (the Console, the Artifact Explorer, Run
    /// Diagnostics), Help's, the update checks, and the first-run setup;
    /// whether to show it and check for updates as Nerthus starts.
    pub console: Option<console_window::ConsoleWindow>,
    pub artifact_explorer: Option<artifact_explorer::ArtifactExplorer>,
    pub diagnostics: Option<diagnostics_window::DiagnosticsWindow>,
    pub help: help_windows::HelpWindows,
    /// The documentation, while open.
    pub docs: Option<docs_window::DocsWindow>,
    pub updates: crate::updates::Updates,
    pub onboard: Option<onboard::Onboard>,
    /// The Nahimic warning, while it shows.
    pub nahimic_warning: Option<nahimic_warning::NahimicWarning>,
    pub onboard_at_launch: bool,
    pub check_updates_at_launch: bool,
    /// What's New shows by itself if this version has not shown it yet,
    /// once the first-run setup is answered (`VersionFirstRun`).
    pub whats_new_at_launch: bool,
    /// `--show` of a window that needs the editor to open.
    tools_at_launch: Vec<tool_commands::ToolCommand>,
    commands_at_launch: Vec<Command>,
    /// What the floating viewport's window asked for, run with the main
    /// window's commands.
    floating_commands: Vec<Command>,
    /// The 3D view was clicked last, so its keys move the camera.
    viewport_focused: bool,
    /// The navigation settings have gone to the view (`SetInputOptions`),
    /// and until when its hint shows.
    navigation_sent: bool,
    navigation_hint_until: Option<f64>,
    /// `--show screenshot`: F9 once the first render is in, at this scale.
    screenshot_at_launch: Option<u32>,
    /// `--show region:<name>`: the region previewed once the project is in.
    region_at_launch: Option<String>,
    /// `--show slider-popup:<property>:<along>`: the shown node's slider for
    /// that property right-clicked that share along, once the preview is in.
    right_click_at_launch: Option<String>,
    /// `--show` of a graph menu or search, opened once the graph is laid
    /// out (`graph_commands::open_at_launch`).
    graph_at_launch: Option<String>,
    /// The document the views were made for (`Editor::document_serial`).
    document_serial: u64,
    styled: bool,
    /// A caught panic waiting for its questions to be answered, and
    /// whether Nerthus is still starting.
    pub crash: Option<crate::crash::Crash>,
    startup: crate::crash::Startup,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            graph: GraphState::default(),
            viewport_2d: viewport_2d::Viewport2DState::default(),
            data_view: data_view::DataViewState::default(),
            split_x: 5.0 / 6.0,
            split_y: 0.5,
            maximized: false,
            sun_open: false,
            build_confirm: None,
            build_settings: None,
            batch_build: None,
            batch_session: batch_build::BatchSession::default(),
            file_dialogs: file_dialogs::FileDialogs::default(),
            bake_progress: None,
            optimize_memory: None,
            memory_report: None,
            clock: 0.0,
            prompt: None,
            alert: None,
            variable_dialog: None,
            expression_dialog: None,
            turntable: None,
            turntable_memory: turntable_window::TurntableMemory::default(),
            constructor: None,
            command_line: None,
            statistics: None,
            options: None,
            last_options_tab: options_window::OptionsTab::General,
            launch_show: None,
            console: None,
            artifact_explorer: None,
            diagnostics: None,
            help: help_windows::HelpWindows::default(),
            docs: None,
            updates: crate::updates::Updates::default(),
            onboard: None,
            nahimic_warning: None,
            onboard_at_launch: false,
            check_updates_at_launch: false,
            whats_new_at_launch: false,
            tools_at_launch: Vec::new(),
            commands_at_launch: Vec::new(),
            floating_commands: Vec::new(),
            viewport_focused: false,
            navigation_sent: false,
            navigation_hint_until: None,
            screenshot_at_launch: None,
            region_at_launch: None,
            right_click_at_launch: None,
            graph_at_launch: None,
            document_serial: 0,
            styled: false,
            crash: None,
            startup: crate::crash::Startup::default(),
        }
    }
}

impl UiState {
    /// What the user's options say about the interface as Nerthus starts:
    /// the Data View, the graph's grid and the viewport's elements.
    fn apply_options(&mut self, options: &UserOptions, settings: &mut ViewportSettings) {
        self.data_view.expanded = options.expand_data_view();
        self.graph.show_grid = options.show_graph_grid();
        self.graph.snap_to_grid = options.snap_to_grid();
        self.graph.grid_unit = options.grid_size();
        let (grid, bounds, compass) = options.viewport_elements();
        settings.grid = grid;
        settings.bounds = bounds;
        settings.compass = compass;
        settings.background = options.viewport_background();
    }

    /// Keeps the options in step with the interface (they are written as
    /// the toggles change).
    fn store_options(&self, options: &mut UserOptions, settings: &ViewportSettings) {
        options.set_expand_data_view(self.data_view.expanded);
        options.set_show_graph_grid(self.graph.show_grid);
        options.set_snap_to_grid(self.graph.snap_to_grid);
        options.set_viewport_elements(settings.grid, settings.bounds, settings.compass);
    }

    /// The windows `--show` asked for that open with the editor's help.
    fn take_tools_at_launch(&mut self) -> Vec<tool_commands::ToolCommand> {
        std::mem::take(&mut self.tools_at_launch)
    }

    /// A new document: graph views come from it again.
    pub fn reset_views(&mut self) {
        self.graph.reset();
        self.data_view.reset();
        // The batch build script is cleared with the project.
        self.turntable_memory = turntable_window::TurntableMemory::default();
    }

    /// `--show`: `build-confirm`, `build-settings` with an optional `:tab`
    /// (resolution, build, tiles, nodes, script, terrain, regions),
    /// `data-view` with an optional `:tab` (terrain, build, stats, vars),
    /// `options` with an optional `:page` (`compute`, `data-view`, …),
    /// `lighting`, `region:<name>` (that region previewed),
    /// `slider-popup:<property>:<along>` (the shown node's slider for the
    /// property right-clicked that share along it), `screenshot`
    /// with an optional `:scale` (F9 once the view has rendered), Tools'
    /// and Help's windows (`console`, `artifacts`, `diagnostics`,
    /// `shortcuts`, `whats-new` with an optional `:notes`, `about`,
    /// `docs` with an optional `:<page>[#heading]`, `docs-search:<words>`,
    /// `updates`, or `updates:sample` for a made-up newer release), the
    /// first-run setup on a page (`onboard:preferences`), the Nahimic
    /// warning and its "Nahimic Detected!" (`nahimic`, or `nahimic:quiet`
    /// for those alone), or one of the graph's menus
    /// (`graph_commands::open_at_launch`).
    pub fn open_at_launch(&mut self, show: &str) {
        let (panel, detail) = show.split_once(':').unwrap_or((show, ""));
        match panel {
            "build-confirm" => self.build_confirm = Some(build_window::BuildConfirm { open_folder: false }),
            "build-settings" => {
                let tab = match detail {
                    "build" => build_settings::SettingsTab::Build,
                    "tiles" => build_settings::SettingsTab::Tiles,
                    "nodes" => build_settings::SettingsTab::Nodes,
                    "script" => build_settings::SettingsTab::Script,
                    "terrain" => build_settings::SettingsTab::Terrain,
                    "regions" => build_settings::SettingsTab::Regions,
                    _ => build_settings::SettingsTab::Resolution,
                };
                self.build_settings = Some(build_settings::BuildSettingsState::new(tab));
            }
            "data-view" => {
                let tab = match detail {
                    "build" => data_view::DataTab::Build,
                    "stats" => data_view::DataTab::Stats,
                    "vars" => data_view::DataTab::Vars,
                    _ => data_view::DataTab::Terrain,
                };
                self.data_view.show_tab(tab);
            }
            "options" => {
                let tab = options_window::OptionsTab::from_name(detail).unwrap_or(options_window::OptionsTab::General);
                self.options = Some(options_window::OptionsState::new(tab));
            }
            "console" => self.console = Some(console_window::ConsoleWindow::open()),
            "diagnostics" => self.diagnostics = Some(diagnostics_window::DiagnosticsWindow::open()),
            "shortcuts" => self.help.shortcuts = true,
            "whats-new" => self.help.whats_new = Some(if detail == "notes" { 2 } else { 0 }),
            // As if Nahimic was found running: the warning, and "Nahimic
            // Detected!" (`nahimic:quiet` without the warning).
            "nahimic" => crate::nahimic::pretend_detected(detail != "quiet"),
            "about" => self.help.about = true,
            "docs" => {
                let target = if detail.is_empty() { crate::docs::HOME } else { detail };
                self.docs = Some(docs_window::DocsWindow::open(target));
            }
            "docs-search" => {
                let mut window = docs_window::DocsWindow::open(crate::docs::HOME);
                window.search_for(detail);
                self.docs = Some(window);
            }
            "artifacts" => self.tools_at_launch.push(tool_commands::ToolCommand::ShowArtifactExplorer),
            "updates" if detail == "sample" => self.help.update = Some(help_windows::UpdateDialog::sample()),
            "onboard" => self.onboard = Some(onboard::Onboard::open_at(detail)),
            "updates" => self.tools_at_launch.push(tool_commands::ToolCommand::CheckForUpdates),
            "lighting" => self.sun_open = true,
            "skybox" => self.graph.toggle_skybox = true,
            "viewport-2d" | "map-viewer" | "gizmo" | "pick" | "measure" => self.viewport_2d.open_at_launch(panel),
            "edit-variable" => self.commands_at_launch.push(Command::EditVariable(detail.to_string())),
            // `edit-expression:<node id>.<property>`.
            "edit-expression" => {
                let (node, property) = detail.split_once('.').unwrap_or((detail, ""));
                if let Ok(node) = node.parse() {
                    self.commands_at_launch.push(Command::EditExpression { node, property: property.to_string() });
                }
            }
            "command-line" => self.commands_at_launch.push(Command::ShowBuildCommandLine),
            "bake-selected" => self.commands_at_launch.push(Command::BakeSelected),
            "statistics" => self.commands_at_launch.push(Command::ShowBuildStatistics),
            "flat" => self.commands_at_launch.push(Command::ToggleFlatView),
            "floating" => self.commands_at_launch.push(Command::SetViewportFloating(true)),
            "turntable" => self.commands_at_launch.push(Command::ShowTurntable),
            "constructor" => self.commands_at_launch.push(Command::OpenConstructor(-1)),
            "record-turntable" => self.commands_at_launch.push(Command::RecordTurntable(detail.parse().unwrap_or(10))),
            "overlay" => self.commands_at_launch.push(Command::SetOverlayImage(detail.parse().unwrap_or(3))),
            "alert" => {
                let text = "Var_1a2,Var_3b4. You can disable this check in Options.";
                self.alert = Some(alert::Alert::new("Orphan Variables Detected", text, alert::AlertKind::Warning).staying());
            }
            "region" => self.region_at_launch = Some(detail.to_string()),
            "slider-popup" => self.right_click_at_launch = Some(detail.to_string()),
            "start-dialog" => self.commands_at_launch.push(Command::File(crate::files::FileCommand::StartDialog)),
            "examples" => self.commands_at_launch.push(Command::File(crate::files::FileCommand::OpenExamples)),
            "export-macro" => self.commands_at_launch.push(Command::File(crate::files::FileCommand::ExportMacro)),
            "save-question" => self.prompt = Some(file_commands::save_question()),
            "screenshot" => self.screenshot_at_launch = Some(detail.parse().unwrap_or(1)),
            _ if graph_commands::opens_at_launch(panel) => self.graph_at_launch = Some(show.to_string()),
            _ => {}
        }
    }
}

/// `Root` grid margins.
const ROOT_MARGIN: f32 = 4.0;
/// `Shell.passiveTimer`'s period in seconds.
const NAVIGATION_TIMER: f64 = 5.0;
const AREA_MARGIN: f32 = 2.0;
const SPLITTER: f32 = 4.0;
/// The property area's narrowest: room for a seed box's six digits and a
/// slider's name beside its value. Sensible floors for the rest.
const MIN_PROPERTIES: f32 = 260.0;
const MIN_LEFT: f32 = 300.0;
const MIN_ROW: f32 = 120.0;

/// Draws the editor, runs what it asked for and fits the 3D view into the
/// viewport area. A panic on the way is caught and asked about
/// (`crate::crash`); until that is answered, only the question shows.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    mut contexts: EguiContexts,
    mut editor: ResMut<Editor>,
    mut ui_state: ResMut<UiState>,
    mut settings: ResMut<ViewportSettings>,
    mut input: ResMut<ViewportInput>,
    mut link: ViewportLink,
    camera: Single<(&mut Camera, &StrategyCamera), With<ViewportCamera>>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
    diagnostics: Res<DiagnosticsStore>,
) -> Result {
    let ctx = contexts.ctx_mut()?.clone();
    if ui_state.crash.is_some() {
        crate::crash::show(&ctx, &mut ui_state.crash);
        return Ok(());
    }
    let (mut camera, strategy) = camera.into_inner();
    let editor_frame = AssertUnwindSafe(|| {
        draw_editor(&ctx, &mut editor, &mut ui_state, &mut settings, &mut input, &mut link, &mut camera, strategy, &mut window, &diagnostics)
    });
    let drawn = std::panic::catch_unwind(editor_frame);
    match drawn {
        Ok(result) => {
            let now = ctx.input(|state| state.time);
            ui_state.startup.note_frame(now);
            result
        }
        Err(payload) => {
            let starting = ui_state.startup.starting();
            ui_state.crash = Some(crate::crash::Crash::caught(payload.as_ref(), starting));
            ctx.request_repaint();
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_editor(
    ctx: &egui::Context,
    editor: &mut Editor,
    ui_state: &mut UiState,
    settings: &mut ViewportSettings,
    input: &mut ViewportInput,
    link: &mut ViewportLink,
    camera: &mut Camera,
    strategy: &StrategyCamera,
    window: &mut Window,
    diagnostics: &DiagnosticsStore,
) -> Result {
    // Fonts set now take effect next frame, so the first frame only
    // installs them.
    if !ui_state.styled {
        theme::apply(ctx);
        // The interface scale comes from the options alone (Tools >
        // Interface Scale); egui's own Ctrl +/- zoom stays off.
        ctx.options_mut(|options| options.zoom_with_keyboard = false);
        egui_extras::install_image_loaders(ctx);
        ui_state.styled = true;
        ui_state.apply_options(&editor.user.options, settings);
        tool_commands::start(editor, ui_state);
        if let Some(show) = ui_state.launch_show.take() {
            // Several panels, comma-separated.
            for panel in show.split(',') {
                ui_state.open_at_launch(panel);
            }
        }
        ctx.request_repaint();
        return Ok(());
    }
    // `UIScaler`: the whole shell scales by `UiScale`.
    ctx.set_zoom_factor(editor.user.options.ui_scale());
    theme::set_settings(ctx, editor.user.options.interface_settings());
    formulas::publish(ctx, editor);
    if ui_state.document_serial != editor.document_serial {
        ui_state.reset_views();
        // The project opens on the graph tab it was saved showing.
        ui_state.graph.tab = editor.document.opening_tab();
        ui_state.document_serial = editor.document_serial;
    }
    // `--show` windows open once the first render is in, so what they
    // report on has been built (at once when nothing is previewed).
    let nothing_to_render = editor.preview_node.is_none() && !editor.building && !editor.needs_build;
    let launch_ready = editor.rendered() || nothing_to_render;
    let mut pending: Vec<Command> = if launch_ready { std::mem::take(&mut ui_state.commands_at_launch) } else { Vec::new() };
    // What the floating viewport asked for last frame.
    pending.append(&mut ui_state.floating_commands);
    let floating = editor.user.options.viewport_floating();
    // Modal dialogs keep the shell's keys from it while they show.
    if !tool_commands::modal_open(ui_state) {
        // Allow node shortcuts only when cursor is inside the graph.
        let node_keys = !editor.user.options.confine_shortcuts() || pointer_over_graph(ctx, &ui_state.graph);
        shortcuts(ctx, node_keys, &mut pending);
        tool_commands::shortcuts(ctx, &mut pending);
    }

    let full = ctx.content_rect();
    let builder = UiBuilder::new().layer_id(LayerId::background()).max_rect(full);
    let mut root = Ui::new(ctx.clone(), "nerthus-root".into(), builder);

    let title_rect = Rect::from_min_size(full.min, Vec2::new(full.width(), title_bar::HEIGHT));
    let status_rect = Rect::from_min_max(Pos2::new(full.left(), full.bottom() - status_bar::HEIGHT), full.max);
    // Where the tool windows stay too.
    let body = tool_window::body(ctx);
    // Painted once the 3D view's place is known: everything but that.
    let background = root.painter().add(egui::Shape::Noop);

    let grid = body.shrink(ROOT_MARGIN);
    let layout = split_layout(&mut root, grid, ui_state, floating);

    editor.camera = strategy.values();
    if strategy.panning() {
        ctx.set_cursor_icon(CursorIcon::Move);
    }
    let now = ctx.input(|state| state.time);
    send_navigation(editor, ui_state, &mut link.navigation, now);
    let hint_text = viewport_area::navigation_hint_text(&link.navigation);
    let hint_shows = ui_state.navigation_hint_until.is_some_and(|until| now < until);
    let overlays = viewport_area::ViewOverlays {
        north_angle: strategy.north_angle(),
        navigation_hint: hint_shows.then_some(hint_text.as_str()),
    };
    // Floating, the viewport area is drawn in its own window.
    let host = if floating {
        Rect::NOTHING
    } else {
        let viewport_rect = layout.viewport.shrink(AREA_MARGIN);
        let interface = &mut *ui_state;
        viewport_area::show(&mut root, viewport_rect, editor, settings, &mut interface.sun_open, &mut interface.viewport_2d, &overlays, &mut pending)
    };
    root.painter().set(background, surround(body, host));
    graph_area::show(&mut root, layout.graph, editor, ui_state, &mut pending);
    editor.graph_region = physical_region(ui_state.graph.rect, ctx.pixels_per_point());
    let properties_rect = layout.properties.shrink(AREA_MARGIN);
    if editor.rendered()
        && let Some(request) = ui_state.right_click_at_launch.take()
    {
        property_area::right_click_at_launch(ctx, &request);
    }
    property_area::show(&mut root, properties_rect, editor, &mut pending);
    tool_commands::forget_add_modifiers(ctx);
    // The Constructor, whose previews need the maps feeding its node.
    let constructor_open = match ui_state.constructor.as_mut() {
        Some(window) => constructor::show(ctx, editor, window),
        None => false,
    };
    if !constructor_open {
        ui_state.constructor = None;
    }
    let constructor_sources = ui_state.constructor.as_ref().map(|window| window.sources(editor)).unwrap_or_default();
    viewport_2d::show_floating(ctx, editor, &mut ui_state.viewport_2d, &constructor_sources, &mut pending);
    // `RendererQuality.FastAO` while a gizmo or Draw's painter is held.
    let constructor_held = ui_state.constructor.as_ref().is_some_and(constructor::ConstructorWindow::gizmo_held);
    let held = ui_state.viewport_2d.gizmo_held() || painter::painting(ctx) || constructor_held;
    let quality = if held { RenderQuality::Fast } else { RenderQuality::Normal };
    editor.set_render_quality(quality);
    ui_state.graph.areas.graph = layout.graph;
    ui_state.graph.areas.properties = properties_rect;
    // Floating, the docked 2D view is in the viewport's own window.
    ui_state.graph.areas.viewport_2d = if floating { None } else { ui_state.viewport_2d.docked_rect() };
    if let Some(show) = ui_state.graph_at_launch.take() {
        graph_commands::open_at_launch(&show, editor, ui_state);
    }
    move_lazy_cursor(ctx, &mut ui_state.graph, window);
    // Lazy Menu › Toggle Skybox (`sky.ShowSkybox`).
    if std::mem::take(&mut ui_state.graph.toggle_skybox) {
        settings.skybox = !settings.skybox;
    }

    title_bar::show(&mut root, title_rect, editor, ui_state, &mut pending);
    // The process's footprint, else Bevy's sampled memory (GiB).
    let gibibyte = 1024.0 * 1024.0 * 1024.0;
    let sampled = diagnostics.get(&SystemInformationDiagnosticsPlugin::PROCESS_MEM_USAGE).and_then(|diagnostic| diagnostic.smoothed());
    let process_memory = crate::memory::process_bytes().map(|bytes| bytes as f64 / gibibyte).or(sampled);
    status_bar::show(&mut root, status_rect, editor, process_memory, &mut pending);

    build_window::confirm(ctx, editor, ui_state, &mut pending);
    build_window::progress(ctx, editor, &mut pending);
    build_settings::show(ctx, editor, &mut ui_state.build_settings, &mut pending);
    batch_window(ctx, editor, ui_state);
    ui_state.clock = now;
    memory_dialogs::show_bake_progress(ctx, editor, &mut ui_state.bake_progress);
    progress_window::show(ctx, editor);
    if let Some(recovery) = memory_dialogs::show_optimize_memory(ctx, &mut ui_state.optimize_memory) {
        pending.extend(memory_dialogs::recovery_commands(recovery));
    }
    memory_dialogs::report_freed(now, &mut ui_state.memory_report, editor);
    if ui_state.memory_report.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
    file_windows::show(ctx, editor, ui_state, &mut pending);
    note_editor::show(ctx, editor, &mut ui_state.graph);
    create_connection::show(ctx, editor, &mut ui_state.graph.create_connection, &mut pending);
    variable_dialog::show(ctx, editor, &mut ui_state.variable_dialog, &mut pending);
    expression_dialog::show(ctx, editor, &mut ui_state.expression_dialog);
    command_line_window::show(ctx, &mut ui_state.command_line);
    statistics_window::show(ctx, &mut ui_state.statistics, &mut pending);
    turntable(ctx, editor, ui_state, &mut link.turntable);
    gizmos::place_3d_handle(editor, &mut link.handle, &link.preview, &mut pending);
    // `alert` spans the graph row, under the properties too.
    let graph_row = Rect::from_min_max(layout.graph.min, Pos2::new(layout.properties.right(), layout.graph.bottom()));
    let animations = !editor.user.options.disable_animations();
    alert::show(ctx, graph_row, &mut ui_state.alert, animations);
    prompt::show(ctx, ui_state, &mut pending);
    let interface = &mut *ui_state;
    let options_outcome = options_window::show(ctx, editor, &mut interface.options, &mut interface.last_options_tab, &mut interface.file_dialogs);
    if options_outcome.run_diagnostics {
        pending.push(Command::Tools(tool_commands::ToolCommand::ShowDiagnostics));
    }
    if let Some(target) = options_outcome.docs {
        pending.push(Command::Tools(tool_commands::ToolCommand::ShowDocs(target.to_string())));
    }
    let settings_reset = tool_commands::show_windows(ctx, editor, ui_state, &mut pending);
    // Accepted options (or settings back to their defaults) reach the
    // interface and the view's mask gradient at once.
    if options_outcome.accepted || settings_reset {
        ui_state.apply_options(&editor.user.options, settings);
        let gradient = MaskGradient::from_index(editor.user.options.mask_gradient());
        editor.set_mask_gradient(gradient);
    }

    if let Some(name) = ui_state.region_at_launch.take() {
        pending.push(Command::PreviewRegion(Some(name)));
    }
    if let Some(scale) = ui_state.screenshot_at_launch
        && editor.rendered()
    {
        ui_state.screenshot_at_launch = None;
        pending.push(Command::SetScreenshotScale(scale));
        pending.push(Command::SaveScreenshot);
    }

    // Previews show the usual port when the preview changes.
    if !editor.render_requested() && !editor.outputs.is_empty() {
        editor.show_preview();
    }

    commands::run(pending, editor, ui_state, window);
    // What the editor shows goes with the project (`TerrainState`), and
    // the preview follows the project's lock through undo and redo.
    let selected = editor.active_node();
    editor.document.note_selected_node(selected);
    editor.document.note_selected_tab(ui_state.graph.tab);
    editor.follow_preview_lock();
    if let Some(values) = editor.camera_request.take() {
        link.camera_request.apply = Some(values);
    }
    let requests = std::mem::take(&mut editor.screenshots);
    link.screenshots.requests.extend(requests);
    for finished in std::mem::take(&mut link.screenshots.finished) {
        match finished {
            Ok(path) => editor.report(format!("Saved screenshot {}", path.display())),
            Err(problem) => editor.report(format!("The screenshot was not saved: {problem}")),
        }
    }
    // The sky follows the lighting's sun and fog.
    let lighting = editor.lighting;
    let sky_moved = settings.sun_azimuth != lighting.sun_azimuth || settings.sun_elevation != lighting.sun_elevation || settings.fog_density != lighting.fog_density;
    if sky_moved {
        settings.sun_azimuth = lighting.sun_azimuth;
        settings.sun_elevation = lighting.sun_elevation;
        settings.fog_density = lighting.fog_density;
    }
    ui_state.store_options(&mut editor.user.options, settings);
    editor.user.save_if_changed();

    if floating {
        return Ok(());
    }
    // Points to physical pixels: the window's scale and the interface's.
    let physical_size = UVec2::new(window.physical_width(), window.physical_height());
    fit_camera(camera, ctx.pixels_per_point(), physical_size, host);
    let pointer = ctx.input(|state| state.pointer.hover_pos());
    let over_view = pointer.is_some_and(|pointer| host.contains(pointer));
    // Windows (Build Settings, a build's progress) cover the view.
    let layer = pointer.and_then(|pointer| ctx.layer_id_at(pointer));
    let over_window = layer.is_some_and(|layer| layer.order != egui::Order::Background);
    input.enabled = over_view && !over_window && !ctx.egui_is_using_pointer() && !ctx.any_popup_open();
    // The view keeps the keyboard from a click in it until a click
    // elsewhere; shortcuts with Ctrl or Cmd stay the editor's.
    if ctx.input(|state| state.pointer.any_pressed()) {
        ui_state.viewport_focused = over_view && !over_window;
    }
    let modifiers = ctx.input(|state| state.modifiers);
    input.keys = ui_state.viewport_focused && !ctx.egui_wants_keyboard_input() && !modifiers.command && !modifiers.ctrl;
    Ok(())
}

/// Record Turntable Animation's window while open, the recording it
/// starts (`BtnRecord_OnClick`) and how that ends (`Capture Complete!`).
fn turntable(ctx: &egui::Context, editor: &mut Editor, interface: &mut UiState, recorder: &mut nerthus_render::Turntable) {
    if let Some(window) = interface.turntable.as_mut() {
        let answer = turntable_window::show(ctx, window, &mut interface.turntable_memory, &mut interface.file_dialogs);
        match answer {
            Some(turntable_window::TurntableAnswer::Record(request)) => {
                interface.turntable = None;
                let folder = request.base.parent().map(std::path::Path::to_path_buf);
                if let Some(Err(error)) = folder.map(std::fs::create_dir_all) {
                    let alert = alert::Alert::new("Error!", &error.to_string(), alert::AlertKind::Error);
                    interface.alert = Some(alert);
                    return;
                }
                recorder.request = Some(request);
                let text = "Please wait and do not interrupt animation capture.";
                interface.alert = Some(alert::Alert::new("Capture Started", text, alert::AlertKind::Information).staying());
            }
            Some(turntable_window::TurntableAnswer::Closed) => interface.turntable = None,
            None => {}
        }
    }
    if recorder.recording() {
        ctx.request_repaint();
    }
    match recorder.outcome.take() {
        Some(nerthus_render::TurntableOutcome::Finished(_)) => {
            interface.alert = Some(alert::Alert::new("Capture Complete!", "Turntable animation capture completed.", alert::AlertKind::Success));
            editor.report("Animation Complete.");
        }
        Some(nerthus_render::TurntableOutcome::Failed(problem)) => {
            interface.alert = Some(alert::Alert::new("Error!", &problem, alert::AlertKind::Error).staying());
        }
        None => {}
    }
}

/// The 3D view's frame rate while it has the focus but the Antialiasing
/// choice says nothing, and while it does not have it (`lowFPS`).
const UNFOCUSED_FRAME_RATE: u32 = 15;

/// `Comms.SetQuality` and `OnApplicationFocus`: the 3D view draws at
/// `rate` frames a second while it has the focus and at 15 while it has
/// not. Nerthus draws the view with everything else, so the whole window
/// waits that long between frames when nothing (input, an animation, a
/// build) asks for one sooner.
pub fn frame_pacing(rate: u32) -> bevy::winit::WinitSettings {
    let focused = std::time::Duration::from_secs_f64(1.0 / rate.max(1) as f64);
    let unfocused = std::time::Duration::from_secs_f64(1.0 / UNFOCUSED_FRAME_RATE as f64);
    bevy::winit::WinitSettings {
        focused_mode: bevy::winit::UpdateMode::reactive(focused),
        unfocused_mode: bevy::winit::UpdateMode::reactive_low_power(unfocused),
    }
}

/// Keeps the frame rate to Antialiasing's (`ViewportQuality`: None 20,
/// 2x 30, 4x and 8x 60) while the 3D view has the focus (clicked last, or
/// under the pointer), 15 otherwise.
pub fn pace_frames(editor: Res<Editor>, ui_state: Res<UiState>, input: Res<ViewportInput>, mut winit: ResMut<bevy::winit::WinitSettings>) {
    let view_focused = ui_state.viewport_focused || input.enabled;
    let rate = if view_focused { editor.user.options.viewport_frame_rate() } else { UNFOCUSED_FRAME_RATE };
    let wanted = frame_pacing(rate);
    if winit.focused_mode != wanted.focused_mode {
        winit.focused_mode = wanted.focused_mode;
    }
}

/// The Batch Build Manager while open, and a batch running in the
/// background.
fn batch_window(ctx: &egui::Context, editor: &mut Editor, ui_state: &mut UiState) {
    ui_state.batch_session.poll(editor);
    if ui_state.batch_session.is_running() {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    let Some(window) = ui_state.batch_build.as_mut() else {
        return;
    };
    let still_open = batch_build::show(ctx, editor, window, &mut ui_state.batch_session, &mut ui_state.file_dialogs);
    if !still_open {
        ui_state.batch_build = None;
    }
}

/// What the interface hands the 3D view: navigation settings, camera
/// jumps and screenshots.
#[derive(SystemParam)]
pub struct ViewportLink<'w> {
    navigation: ResMut<'w, NavigationSettings>,
    camera_request: ResMut<'w, CameraRequest>,
    screenshots: ResMut<'w, Screenshots>,
    turntable: ResMut<'w, nerthus_render::Turntable>,
    handle: ResMut<'w, nerthus_render::PositionHandle>,
    preview: Res<'w, nerthus_render::TerrainPreview>,
}

/// A rectangle of the interface in the window's physical pixels.
fn physical_region(rect: Rect, pixels_per_point: f32) -> Option<URect> {
    if !rect.is_positive() {
        return None;
    }
    let scaled = rect * pixels_per_point;
    let left = scaled.left().max(0.0).round() as u32;
    let top = scaled.top().max(0.0).round() as u32;
    let right = scaled.right().max(0.0).round() as u32;
    let bottom = scaled.bottom().max(0.0).round() as u32;
    Some(URect::new(left, top, right, bottom))
}

/// `SetInputOptions`: the view takes the navigation settings when it
/// starts and whenever they change, and shows its hint until the third
/// tick after of the five-second passive timer.
fn send_navigation(editor: &Editor, ui_state: &mut UiState, navigation: &mut NavigationSettings, now: f64) {
    let wanted = editor.user.options.navigation();
    if ui_state.navigation_sent && *navigation == wanted {
        return;
    }
    ui_state.navigation_sent = true;
    *navigation = wanted;
    if editor.user.options.show_navigation_hint() {
        let ticks = (now / NAVIGATION_TIMER).floor();
        ui_state.navigation_hint_until = Some((ticks + 3.0) * NAVIGATION_TIMER);
    }
}

/// The window background around `hole`, where the 3D view shows through.
fn surround(area: Rect, hole: Rect) -> egui::Shape {
    let hole = hole.intersect(area);
    // No 3D view here (it floats): all of it.
    if !hole.is_positive() {
        return egui::Shape::rect_filled(area, 0.0, theme::WINDOW_BACKGROUND);
    }
    let strips = [
        Rect::from_min_max(area.min, Pos2::new(area.right(), hole.top())),
        Rect::from_min_max(Pos2::new(area.left(), hole.bottom()), area.max),
        Rect::from_min_max(Pos2::new(area.left(), hole.top()), Pos2::new(hole.left(), hole.bottom())),
        Rect::from_min_max(Pos2::new(hole.right(), hole.top()), Pos2::new(area.right(), hole.bottom())),
    ];
    let shapes = strips.into_iter().filter(|strip| strip.is_positive()).map(|strip| egui::Shape::rect_filled(strip, 0.0, theme::WINDOW_BACKGROUND));
    egui::Shape::Vec(shapes.collect())
}

/// The three areas of the `Root` grid.
struct Areas {
    viewport: Rect,
    graph: Rect,
    properties: Rect,
}

/// Splits the grid and lets the two splitters move it.
/// Floating, the viewport leaves the left column to the graph
/// (`GraphGrid` spanning both rows, `gsHorizontal1` hidden).
fn split_layout(ui: &mut Ui, grid: Rect, state: &mut UiState, floating: bool) -> Areas {
    let left_width = (grid.width() * state.split_x).clamp(MIN_LEFT.min(grid.width()), (grid.width() - MIN_PROPERTIES).max(MIN_LEFT));
    // Splits land on whole pixels, as WPF's layout rounding puts them.
    let column_split = (grid.left() + left_width).round();
    let left = Rect::from_min_max(grid.min, Pos2::new(column_split, grid.bottom()));
    let properties = Rect::from_min_max(Pos2::new(column_split, grid.top()), grid.max);
    let top_height = (left.height() * state.split_y).clamp(MIN_ROW.min(left.height()), (left.height() - MIN_ROW).max(MIN_ROW));
    let row_split = (left.top() + top_height).round();

    let vertical = Rect::from_center_size(Pos2::new(column_split, grid.center().y), Vec2::new(SPLITTER, grid.height()));
    let vertical_response = ui.interact(vertical, ui.id().with("splitter-columns"), Sense::drag());
    if vertical_response.hovered() || vertical_response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if vertical_response.dragged() {
        let delta = vertical_response.drag_delta().x;
        state.split_x = ((left_width + delta) / grid.width()).clamp(0.2, 0.95);
    }
    if floating {
        let viewport = Rect::from_min_max(left.min, Pos2::new(left.right(), left.top()));
        return Areas { viewport, graph: left, properties };
    }
    let viewport = Rect::from_min_max(left.min, Pos2::new(left.right(), row_split));
    let graph = Rect::from_min_max(Pos2::new(left.left(), row_split), left.max);
    let horizontal = Rect::from_center_size(Pos2::new(left.center().x, row_split), Vec2::new(left.width(), SPLITTER));
    let horizontal_response = ui.interact(horizontal, ui.id().with("splitter-rows"), Sense::drag());
    if horizontal_response.hovered() || horizontal_response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
    }
    if horizontal_response.dragged() {
        let delta = horizontal_response.drag_delta().y;
        state.split_y = ((top_height + delta) / left.height()).clamp(0.1, 0.9);
    }
    Areas { viewport, graph, properties }
}

/// The 3D camera draws only into the viewport host.
fn fit_camera(camera: &mut Camera, scale: f32, physical_size: UVec2, host: Rect) {
    let physical_width = physical_size.x;
    let physical_height = physical_size.y;
    let left = (host.left() * scale).max(0.0) as u32;
    let top = (host.top() * scale).max(0.0) as u32;
    let width = ((host.width() * scale).max(1.0) as u32).min(physical_width.saturating_sub(left).max(1));
    let height = ((host.height() * scale).max(1.0) as u32).min(physical_height.saturating_sub(top).max(1));
    let viewport = Viewport {
        physical_position: UVec2::new(left, top),
        physical_size: UVec2::new(width, height),
        ..default()
    };
    camera.viewport = Some(viewport);
}

/// `SetCursorPosition`: the pointer to the middle of the area a Lazy Menu
/// entry opened, in the window's logical pixels (points times the
/// interface's zoom).
fn move_lazy_cursor(ctx: &egui::Context, graph: &mut GraphState, window: &mut Window) {
    let Some(target) = graph.lazy_cursor.take() else {
        return;
    };
    let areas = graph.areas;
    let rect = match target {
        graph_canvas::LazyTarget::Viewport2D => areas.viewport_2d,
        graph_canvas::LazyTarget::Graph => Some(areas.graph),
        graph_canvas::LazyTarget::Properties | graph_canvas::LazyTarget::Modifiers => Some(areas.properties),
        graph_canvas::LazyTarget::DataView => areas.data_view,
    };
    let Some(rect) = rect.filter(|rect| rect.is_positive()) else {
        return;
    };
    let zoom = ctx.zoom_factor();
    let center = rect.center();
    let logical = bevy::math::Vec2::new(center.x * zoom, center.y * zoom);
    window.set_cursor_position(Some(logical));
}

/// Whether the pointer is over the graph itself (`graph.IsMouseOver`):
/// inside its canvas as last laid out, not over the toolbox or a window.
fn pointer_over_graph(ctx: &egui::Context, graph: &GraphState) -> bool {
    let Some(pointer) = ctx.input(|input| input.pointer.hover_pos()) else {
        return false;
    };
    if !graph.rect.contains(pointer) || graph.toolbox_rect.contains(pointer) {
        return false;
    }
    let layer = ctx.layer_id_at(pointer);
    layer.is_none_or(|layer| layer.order == egui::Order::Background)
}

/// The keys `Shell_KeyUp` handles after its `ConfineShortcuts` check: the
/// node and graph editing ones.
fn node_key(command: &Command) -> bool {
    let graph_editing = match command {
        Command::Graph(graph_command) => !matches!(graph_command, GraphCommand::FindNode | GraphCommand::LazyMenu | GraphCommand::NodeSearch | GraphCommand::QuickNavigation),
        _ => false,
    };
    let node_editing = matches!(
        command,
        Command::Duplicate
            | Command::SelectAll
            | Command::ResetNode
            | Command::ZoomExtents
            | Command::ZoomReset
            | Command::ZoomSelection
            | Command::Delete
            | Command::Rename
            | Command::Refresh
            | Command::MarkForExport
            | Command::Bypass
            | Command::ToggleUnderlay
            | Command::ToggleExpansion
    );
    graph_editing || node_editing
}

/// The keyboard shortcuts. Graph keys only act when no text field has
/// the keyboard, and the node keys only when `node_keys` allows them.
fn shortcuts(ctx: &egui::Context, node_keys: bool, pending: &mut Vec<Command>) {
    let command = Modifiers::COMMAND;
    let command_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    let command_alt = Modifiers::COMMAND | Modifiers::ALT;
    let bindings = [
        (KeyboardShortcut::new(command_alt, Key::O), Command::ShowOptions(None)),
        (KeyboardShortcut::new(command, Key::N), Command::NewProject),
        // Shifted shortcuts come first: a plain one matches with Shift too.
        (KeyboardShortcut::new(command_shift, Key::O), Command::OpenLatest),
        (KeyboardShortcut::new(command, Key::O), Command::Open),
        (KeyboardShortcut::new(command_shift, Key::S), Command::SaveAs),
        (KeyboardShortcut::new(command_shift, Key::B), Command::BuildAndExport),
        (KeyboardShortcut::new(command, Key::B), Command::BuildSettings),
        (KeyboardShortcut::new(command, Key::S), Command::Save),
        (KeyboardShortcut::new(command_shift, Key::Z), Command::Redo),
        (KeyboardShortcut::new(command, Key::Z), Command::Undo),
        (KeyboardShortcut::new(command, Key::Y), Command::Redo),
        (KeyboardShortcut::new(command, Key::D), Command::Duplicate),
        (KeyboardShortcut::new(command, Key::A), Command::SelectAll),
        (KeyboardShortcut::new(command, Key::R), Command::ResetNode),
        (KeyboardShortcut::new(command, Key::Num0), Command::ZoomExtents),
        (KeyboardShortcut::new(command, Key::Num1), Command::ZoomReset),
        (KeyboardShortcut::new(command, Key::Num2), Command::ZoomSelection),
        (KeyboardShortcut::new(command, Key::F1), Command::ShowDataTab(data_view::DataTab::Terrain)),
        (KeyboardShortcut::new(command, Key::F3), Command::ShowDataTab(data_view::DataTab::Build)),
        (KeyboardShortcut::new(command, Key::F9), Command::SaveViewportScreenshotAs),
        (KeyboardShortcut::new(command, Key::F6), Command::ShowLocked2DViewport),
        (KeyboardShortcut::new(command, Key::M), Command::Show2DView),
    ];
    let typing = ctx.egui_wants_keyboard_input();
    for (shortcut, action) in bindings {
        let text_editing = matches!(action, Command::Undo | Command::Redo | Command::SelectAll);
        if typing && text_editing {
            continue;
        }
        if !node_keys && node_key(&action) {
            continue;
        }
        if ctx.input_mut(|input| input.consume_shortcut(&shortcut)) {
            pending.push(action);
        }
    }
    if typing {
        return;
    }
    // The graph's (`Shell.OnPreviewKeyDown`); Shift and Ctrl together come
    // first.
    let graph_bindings = [
        (KeyboardShortcut::new(command, Key::X), GraphCommand::Cut),
        (KeyboardShortcut::new(command, Key::C), GraphCommand::Copy),
        (KeyboardShortcut::new(command, Key::V), GraphCommand::Paste),
        (KeyboardShortcut::new(command, Key::G), GraphCommand::GroupSelection),
        (KeyboardShortcut::new(command, Key::T), GraphCommand::FindNode),
        (KeyboardShortcut::new(command, Key::P), GraphCommand::ConvertConnectionsToPortals),
        (KeyboardShortcut::new(command, Key::Q), GraphCommand::ReplaceSearch),
        (KeyboardShortcut::new(command_shift, Key::F8), GraphCommand::Combine(CombineKind::Layers)),
        (KeyboardShortcut::new(command, Key::F8), GraphCommand::Combine(CombineKind::Mixer)),
        (KeyboardShortcut::new(Modifiers::NONE, Key::F8), GraphCommand::Combine(CombineKind::Inputs)),
        (KeyboardShortcut::new(Modifiers::NONE, Key::F4), GraphCommand::AutoLayout),
        (KeyboardShortcut::new(Modifiers::NONE, Key::F1), GraphCommand::LazyMenu),
        (KeyboardShortcut::new(Modifiers::NONE, Key::T), GraphCommand::CycleRenderIntent),
        (KeyboardShortcut::new(Modifiers::NONE, Key::P), GraphCommand::ManagePortals),
        (KeyboardShortcut::new(Modifiers::NONE, Key::Tab), GraphCommand::NodeSearch),
        (KeyboardShortcut::new(Modifiers::NONE, Key::Comma), GraphCommand::QuickNavigation),
        (KeyboardShortcut::new(Modifiers::NONE, Key::F), GraphCommand::LockSelection),
    ];
    for (shortcut, action) in graph_bindings {
        let command = Command::Graph(action);
        if !node_keys && node_key(&command) {
            continue;
        }
        if ctx.input_mut(|input| input.consume_shortcut(&shortcut)) {
            pending.push(command);
        }
    }
    let plain = [
        (Key::Delete, Command::Delete),
        (Key::Backspace, Command::Delete),
        (Key::F2, Command::Rename),
        (Key::F5, Command::Refresh),
        (Key::F3, Command::MarkForExport),
        (Key::B, Command::Bypass),
        (Key::G, Command::ToggleUnderlay),
        (Key::F11, Command::ToggleExpansion),
        (Key::F9, Command::SaveScreenshot),
        (Key::F6, Command::Toggle2DViewport),
        (Key::Z, Command::ToggleGizmo),
        (Key::V, Command::ToggleFlatView),
    ];
    for (key, action) in plain {
        if !node_keys && node_key(&action) {
            continue;
        }
        let shortcut = KeyboardShortcut::new(Modifiers::NONE, key);
        if ctx.input_mut(|input| input.consume_shortcut(&shortcut)) {
            pending.push(action);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The commands the shortcuts give for `events`, the pointer over the
    /// graph or not, with the node keys confined or not.
    fn pressed(events: Vec<egui::Event>, node_keys: bool) -> Vec<Command> {
        let context = egui::Context::default();
        let input = egui::RawInput { events, ..Default::default() };
        let mut pending = Vec::new();
        let output = context.run_ui(input, |ui| shortcuts(ui.ctx(), node_keys, &mut pending));
        output.drop_without_applying_deltas();
        pending
    }

    fn key(key: Key, modifiers: Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    #[test]
    fn confined_node_keys_need_the_graph() {
        let delete = vec![key(Key::Delete, Modifiers::NONE)];
        assert_eq!(pressed(delete.clone(), true), vec![Command::Delete]);
        assert!(pressed(delete, false).is_empty());
        // Keys handled before the check go anywhere.
        let save = vec![key(Key::S, Modifiers::COMMAND)];
        assert_eq!(pressed(save, false), vec![Command::Save]);
        let search = vec![key(Key::Tab, Modifiers::NONE)];
        assert_eq!(pressed(search, false), vec![Command::Graph(GraphCommand::NodeSearch)]);
        let copy = vec![key(Key::C, Modifiers::COMMAND)];
        assert!(pressed(copy, false).is_empty());
    }
}
