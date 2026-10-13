//! Command-line options: `nerthus [project] [--preview <node>] [--show <panel>]
//! [--size <width>x<height>] [--monitor <index>] [--position <x>,<y>]
//! [--screenshot <png>] [--cpu-only] [--verbose] [--onboard] [--skip-onboard]
//! [--mcp] [--mcp-port <port>] [--diagnostics]`.
//!
//! `--screenshot` waits for the preview to build, saves an image of the
//! Nerthus window (and nothing else on screen) and quits; it exists for
//! checking the editor without a person at the screen. `--show` opens a
//! window first: `build-settings[:<tab>]`, `build-confirm`, or `build`
//! (a Build and Export, captured once it finishes). `--size` sets the
//! window's size in logical pixels, for comparing captures; `--monitor`
//! centres the window on that display, `--position` puts its top-left
//! corner at a desktop position. `--cpu-only` keeps every computation on
//! the CPU, as `cpuonly.dat` in the data folder does. `--verbose` turns
//! Verbose Logging on, as `verbose.dat` in the data folder does.
//! `--onboard` and `--skip-onboard` show the first-run setup, or mark it
//! done without showing it. `--mcp` starts the MCP server for this
//! session (as `mcp.dat` in the data folder does for every session);
//! `--mcp-port` starts it on that port. `--diagnostics` opens Diagnostics
//! at once (as a crash that is not continued does).

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot, ScreenshotCaptured};
use nerthus_render::TerrainPreview;

use crate::editor::Editor;

/// The editor window (`Shell`) opens at 1600 × 960.
pub const DEFAULT_WINDOW_SIZE: (u32, u32) = (1600, 960);

#[derive(Default)]
pub struct LaunchOptions {
    pub project: Option<PathBuf>,
    /// Display name or type of the node to preview first.
    pub preview: Option<String>,
    /// A window to open at start (`--show`).
    pub show: Option<String>,
    pub screenshot: Option<PathBuf>,
    /// Window size in logical pixels (`--size 1920x1044`).
    pub size: Option<(u32, u32)>,
    /// The display to open on (`--monitor 1`).
    pub monitor: Option<usize>,
    /// The window's top-left corner on the desktop (`--position -276,-1080`).
    pub position: Option<(i32, i32)>,
    /// `--cpu-only`: no GPU computes, whatever the preferences say.
    pub cpu_only: bool,
    /// `--verbose`: Verbose Logging for this session.
    pub verbose: bool,
    /// `--onboard` and `--skip-onboard`.
    pub onboard: bool,
    pub skip_onboard: bool,
    /// `--mcp` and `--mcp-port`.
    pub mcp: bool,
    pub mcp_port: Option<u16>,
}

impl LaunchOptions {
    pub fn from_args() -> Self {
        let mut options = LaunchOptions::default();
        let mut arguments = std::env::args().skip(1);
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--preview" => options.preview = arguments.next(),
                "--show" => options.show = arguments.next(),
                "--screenshot" => options.screenshot = arguments.next().map(PathBuf::from),
                "--size" => options.size = arguments.next().as_deref().and_then(parse_size),
                "--monitor" => options.monitor = arguments.next().and_then(|text| text.parse().ok()),
                "--position" => options.position = arguments.next().as_deref().and_then(parse_position),
                "--cpu-only" => options.cpu_only = true,
                "--verbose" => options.verbose = true,
                "--onboard" => options.onboard = true,
                "--skip-onboard" => options.skip_onboard = true,
                "--mcp" => options.mcp = true,
                "--mcp-port" => {
                    options.mcp = true;
                    options.mcp_port = arguments.next().and_then(|text| text.parse().ok());
                }
                "--diagnostics" => options.show_also("diagnostics"),
                _ => options.project = Some(PathBuf::from(argument)),
            }
        }
        options
    }

    /// Adds a window to open at start to those `--show` names.
    fn show_also(&mut self, panel: &str) {
        self.show = match self.show.take() {
            Some(shown) => Some(format!("{shown},{panel}")),
            None => Some(panel.to_string()),
        };
    }
}

/// `-276,-1080` → (-276, -1080).
fn parse_position(text: &str) -> Option<(i32, i32)> {
    let (x, y) = text.split_once(',')?;
    let x = x.trim().parse().ok()?;
    let y = y.trim().parse().ok()?;
    Some((x, y))
}

/// `1920x1044` → (1920, 1044).
fn parse_size(text: &str) -> Option<(u32, u32)> {
    let (width, height) = text.split_once('x')?;
    let width = width.trim().parse().ok()?;
    let height = height.trim().parse().ok()?;
    Some((width, height))
}

/// Seconds to wait once the preview is shown, so the mesh and shadows
/// settle.
const SETTLE_SECONDS: f64 = 1.5;
/// Seconds after which the capture is taken whatever the build is doing.
const GIVE_UP_SECONDS: f64 = 120.0;
/// Seconds to wait for the image to come back from the GPU.
const SAVE_SECONDS: f64 = 30.0;

#[derive(Resource)]
struct Capture {
    path: PathBuf,
    /// When the editor became ready (real seconds), and when the shot was
    /// asked for.
    ready_at: Option<f64>,
    taken_at: Option<f64>,
    /// A node was named to preview: wait for its build.
    expects_preview: bool,
    /// The image has come back from the GPU and been written.
    saved: bool,
}

pub struct CapturePlugin {
    pub path: PathBuf,
    pub expects_preview: bool,
}

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        let capture = Capture {
            path: self.path.clone(),
            ready_at: None,
            taken_at: None,
            expects_preview: self.expects_preview,
            saved: false,
        };
        app.insert_resource(capture);
        app.add_systems(Update, capture_window);
    }
}

#[allow(clippy::too_many_arguments)]
fn capture_window(
    mut commands: Commands,
    mut capture: ResMut<Capture>,
    editor: Res<Editor>,
    preview: Res<TerrainPreview>,
    time: Res<Time<Real>>,
    floating: Query<Entity, With<crate::ui::floating_viewport::FloatingViewportWindow>>,
    mut exit: MessageWriter<AppExit>,
) {
    let now = time.elapsed_secs_f64();
    let preview_shown = preview.revision > 0 || editor.preview_error.is_some();
    let preview_settled = if capture.expects_preview { editor.preview_node.is_some() && preview_shown } else { editor.preview_node.is_none() || preview_shown };
    let exported = editor.export_build.as_ref().is_none_or(|build| build.finished);
    let ready = !editor.building && exported && preview_settled;
    if ready && capture.ready_at.is_none() {
        capture.ready_at = Some(now);
    }
    let settled = capture.ready_at.is_some_and(|ready_at| now - ready_at >= SETTLE_SECONDS);
    let gave_up = now >= GIVE_UP_SECONDS;
    if capture.taken_at.is_none() && (settled || gave_up) {
        let path = capture.path.clone();
        // The floating viewport's window as `<name>-floating.png` beside it.
        if let Some(window) = floating.iter().next() {
            let stem = path.file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_default();
            let floating_path = path.with_file_name(format!("{stem}-floating.png"));
            let mut floating_screenshot = commands.spawn(Screenshot::window(window));
            floating_screenshot.observe(save_to_disk(floating_path));
        }
        let mut screenshot = commands.spawn(Screenshot::primary_window());
        screenshot.observe(save_to_disk(path));
        screenshot.observe(|_: On<ScreenshotCaptured>, mut capture: ResMut<Capture>| capture.saved = true);
        capture.taken_at = Some(now);
    }
    // The read-back is asynchronous: leave once it has been written.
    let waited_too_long = capture.taken_at.is_some_and(|taken_at| now - taken_at >= SAVE_SECONDS);
    if capture.taken_at.is_some() && (capture.saved || waited_too_long) {
        exit.write(AppExit::Success);
    }
}
