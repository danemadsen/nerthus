//! Nerthus: a node-based terrain editor. The editor is egui drawn over
//! Bevy's 3D viewport.

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
mod files;
mod icons;
mod launch;
mod logging;
mod mcp;
mod memory;
mod node_files;
mod node_ui;
mod overlays;
mod presets;
mod progress;
mod theme;
mod ui;
mod updates;
mod user;
mod viewport_camera;
mod viewport_render;
mod worker;

use bevy::camera::visibility::RenderLayers;
use bevy::camera::CameraOutputMode;
use bevy::prelude::*;
use bevy::render::render_resource::BlendState;
use bevy_egui::{EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass, PrimaryEguiContext};
use nerthus_render::TerrainRenderPlugin;

fn main() {
    let options = launch::LaunchOptions::from_args();
    let size = options.size.unwrap_or(launch::DEFAULT_WINDOW_SIZE);
    let position = match (options.position, options.monitor) {
        (Some((x, y)), _) => WindowPosition::At(IVec2::new(x, y)),
        (None, Some(index)) => WindowPosition::Centered(MonitorSelection::Index(index)),
        (None, None) => WindowPosition::Automatic,
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
    // terminal shown debugging messages only with Verbose Logging.
    let log_plugin = bevy::log::LogPlugin {
        level: bevy::log::Level::DEBUG,
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
    ui_state.onboard_at_launch = ui::onboard::should_show(options.onboard, options.skip_onboard, screenshot);
    ui_state.check_updates_at_launch = !screenshot;
    let mcp = mcp::McpPlugin {
        enabled: options.mcp,
        port: options.mcp_port,
        screenshot_run: screenshot,
    };
    app.add_plugins(mcp);
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
