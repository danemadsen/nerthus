//! Nerthus: a node-based terrain editor. The editor is egui drawn over
//! Bevy's 3D viewport.

// The web build leaves out what a browser page cannot do (files, other
// programs, the network, GPU compute), so code only those use goes unused.
#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports))]

mod app_icon;
mod build_profiles;
mod compute;
mod constructor_preview;
mod crash;
mod disk_cache;
mod docs;
mod document;
mod drop_position;
mod editor;
mod export_build;
mod file_association;
mod files;
mod icons;
mod launch;
mod logging;
mod mcp;
mod memory;
mod nahimic;
mod node_files;
mod node_ui;
mod overlays;
mod presets;
mod progress;
mod start_monitor;
mod theme;
mod ui;
mod updates;
mod user;
mod viewport_camera;
mod viewport_render;
#[cfg(target_arch = "wasm32")]
mod web;
mod worker;

use bevy::camera::visibility::RenderLayers;
use bevy::camera::CameraOutputMode;
use bevy::prelude::*;
use bevy::render::render_resource::BlendState;
use bevy_egui::{EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass, PrimaryEguiContext};
use nerthus_render::TerrainRenderPlugin;

fn main() {
    let options = launch::LaunchOptions::from_args();
    // `App.ShowHelp`: `-?` lists the options and Nerthus goes no further.
    if options.help {
        launch::show_usage();
        return;
    }
    let size = options.size.unwrap_or(launch::DEFAULT_WINDOW_SIZE);
    // A monitor is chosen once Bevy knows them (`StartMonitorPlugin`).
    let position = match options.position {
        Some((x, y)) => WindowPosition::At(IVec2::new(x, y)),
        None => WindowPosition::Automatic,
    };
    // The editor draws its own title bar (`WindowChrome`); on macOS the
    // native frame stays for resizing and shadows, without title or buttons.
    let window = Window {
        title: "Nerthus".to_string(),
        resolution: size.into(),
        position,
        decorations: cfg!(target_os = "macos"),
        titlebar_transparent: true,
        titlebar_show_title: false,
        titlebar_show_buttons: false,
        fullsize_content_view: true,
        // The web build draws into the page's canvas (web/index.html),
        // filling the browser window.
        #[cfg(target_arch = "wasm32")]
        canvas: Some("#nerthus".to_string()),
        #[cfg(target_arch = "wasm32")]
        fit_canvas_to_parent: true,
        ..default()
    };
    // The close button asks to save changes first (`Shell_Closing`), so
    // the window closes only once that is answered.
    let window_plugin = WindowPlugin {
        primary_window: Some(window),
        close_when_requested: false,
        ..default()
    };
    // The session's log (`Logging.Initialize`): every message kept, the
    // terminal shown debugging messages only with Verbose Logging. The web
    // build's log goes to the browser's console as well, where debugging
    // messages would slow the page.
    let log_level = if cfg!(target_arch = "wasm32") { bevy::log::Level::INFO } else { bevy::log::Level::DEBUG };
    let log_plugin = bevy::log::LogPlugin {
        level: log_level,
        custom_layer: logging::session_layer,
        fmt_layer: logging::terminal_layer,
        ..default()
    };
    let plugins = DefaultPlugins.set(window_plugin).set(log_plugin);
    // egui gets its own full-window camera; the 3D camera only covers the
    // viewport area.
    let egui_settings = EguiGlobalSettings {
        auto_create_primary_context: false,
        ..default()
    };

    let mut app = App::new();
    app.add_plugins(plugins);
    logging::start(options.verbose || logging::verbose_at_start());
    logging::install_crash_log();
    for argument in &options.ignored {
        log::warn!("Ignoring unknown argument: {argument}");
    }
    // `OnStartup`'s system work (on Windows): `.terrain` files registered,
    // then `NahimicCountermeasures`.
    file_association::register_project_files();
    nahimic::countermeasures();
    app.add_plugins(app_icon::AppIconPlugin);
    app.insert_resource(egui_settings);
    app.add_plugins(EguiPlugin::default());
    app.add_plugins(drop_position::DropPositionPlugin);
    app.add_plugins(bevy::diagnostic::SystemInformationDiagnosticsPlugin);
    app.add_plugins(TerrainRenderPlugin);
    if let Some(path) = options.screenshot.clone() {
        let capture = launch::CapturePlugin {
            path,
            expects_preview: options.preview.is_some(),
        };
        app.add_plugins(capture);
    }
    let mut ui_state = ui::UiState::default();
    ui_state.launch_show = options.show.clone();
    // `App.ShowOnboarding`, and the update check the start window makes.
    let screenshot = options.screenshot.is_some();
    // The web build keeps no settings, and has no folders or GPUs to set
    // up: it starts with the defaults.
    ui_state.onboard_at_launch = ui::onboard::should_show(options.onboard, options.skip_onboard, screenshot) && !cfg!(target_arch = "wasm32");
    // The web version is always the one on the page.
    ui_state.check_updates_at_launch = !screenshot && !cfg!(target_arch = "wasm32");
    ui_state.whats_new_at_launch = options.greets();
    let mcp = mcp::McpPlugin {
        enabled: options.mcp,
        port: options.mcp_port,
        screenshot_run: screenshot,
    };
    app.add_plugins(mcp);
    // Maximised on the monitor chosen, or centred on `--monitor`, unless
    // the window was put somewhere.
    if options.position.is_none() && (options.maximised() || options.monitor.is_some()) {
        let placement = start_monitor::StartMonitorPlugin {
            monitor: options.monitor,
            maximised: options.maximised(),
        };
        app.add_plugins(placement);
    }
    app.add_plugins(editor::EditorPlugin { options });
    app.add_plugins(files::FilePlugin);
    app.insert_resource(ui_state);
    app.add_systems(Startup, spawn_ui_camera);
    app.add_systems(EguiPrimaryContextPass, ui::draw);
    app.add_systems(Update, ui::floating_viewport::manage);
    // The frame rate the Antialiasing choice gives the 3D view.
    app.insert_resource(ui::frame_pacing(30));
    app.add_systems(Update, ui::pace_frames);
    app.add_systems(ui::floating_viewport::FloatingViewportPass, ui::floating_viewport::draw);
    app.add_systems(Update, ui::file_dialogs::run);
    app.add_systems(Update, ui::options_pages::count_monitors);
    app.add_systems(Last, disk_cache::freeze_on_exit);
    // Run Diagnostics' restarts start the new Nerthus once the previews
    // are on disk.
    app.add_systems(Last, ui::diagnostics_window::restart_on_exit.after(disk_cache::freeze_on_exit));
    app.run();
}

fn spawn_ui_camera(mut commands: Commands) {
    let output_mode = CameraOutputMode::Write {
        blend_state: Some(BlendState::ALPHA_BLENDING),
        clear_color: ClearColorConfig::None,
    };
    // Above the 3D view (0) and its gizmo handle (1).
    let camera = Camera {
        order: 2,
        output_mode,
        clear_color: ClearColorConfig::Custom(Color::NONE),
        ..default()
    };
    commands.spawn((PrimaryEguiContext, Camera2d, RenderLayers::none(), camera));
}
