//! Command-line options (`Arguments`): `nerthus [project] [--no-start]
//! [--safe-mode] [--proxy <url>] [--cpu-only] [--verbose] [--onboard]
//! [--skip-onboard] [--mcp] [--mcp-port <port>] [--diagnostics] [-?]`, and
//! for checking the editor `[--preview <node>] [--show <panel>]
//! [--size <width>x<height>] [--monitor <index>] [--position <x>,<y>]
//! [--screenshot <png>]`.
//!
//! Names are case-insensitive, take one dash or two, and may leave out the
//! hyphens between words (`--no-start` is `-NoStart`); on Windows `/`
//! starts one too. A value follows its option, or comes after `:` or `=`
//! (`-Proxy:http://host:8080`). The first argument that is not an option
//! is the project to open; later ones, and options Nerthus does not know,
//! are ignored (and logged).
//!
//! `-?` (or `--help`) lists the options instead of starting. `--no-start`
//! skips the Start dialog, and with it What's New. `--safe-mode` starts
//! with the default preferences and options. `--proxy` turns the proxy on
//! with that address and saves it. `--cpu-only` keeps every computation on
//! the CPU, as `cpuonly.dat` in the data folder does. `--verbose` turns
//! Verbose Logging on, as `verbose.dat` in the data folder does.
//! `--onboard` and `--skip-onboard` show the first-run setup, or mark it
//! done without showing it. `--mcp` starts the MCP server for this
//! session (as `mcp.dat` in the data folder does for every session);
//! `--mcp-port` starts it on that port. `--diagnostics` opens Diagnostics
//! at once (as a crash that is not continued does).
//!
//! `--screenshot` waits for the preview to build, saves an image of the
//! Nerthus window (and nothing else on screen) and quits; it exists for
//! checking the editor without a person at the screen. `--show` opens a
//! window first: `build-settings[:<tab>]`, `build-confirm`, or `build`
//! (a Build and Export, captured once it finishes). Either skips the
//! Start dialog. The window opens maximised on its monitor unless
//! `--size` (its size in logical pixels, for comparing captures),
//! `--position` (its top-left corner on the desktop) or `--screenshot` is
//! given. `--monitor` opens it on that display instead of the one Options
//! › General chooses, centred there when it is not maximised.

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
    /// The display to open on (`--monitor 1`), instead of the one Options ›
    /// General › Start Nerthus on chooses.
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
    /// `--no-start`: no Start dialog.
    pub no_start: bool,
    /// `--safe-mode`: the default preferences and options for this session.
    pub safe_mode: bool,
    /// `--proxy <url>`: the proxy to turn on and save.
    pub proxy: Option<String>,
    /// `-?`: the options listed instead of starting.
    pub help: bool,
    /// The arguments ignored: options Nerthus does not know, and paths
    /// after the project's.
    pub ignored: Vec<String>,
}

impl LaunchOptions {
    pub fn from_args() -> Self {
        Self::parse(std::env::args().skip(1))
    }

    fn parse(arguments: impl IntoIterator<Item = String>) -> Self {
        let mut options = LaunchOptions::default();
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            let Some((name, written_value)) = option_name(&argument) else {
                if options.project.is_none() {
                    options.project = Some(PathBuf::from(argument));
                } else {
                    options.ignored.push(argument);
                }
                continue;
            };
            // A switch is on unless written `:false`; a value is written
            // after the name or is the next argument.
            let switched_on = written_value.as_deref().is_none_or(|text| !text.eq_ignore_ascii_case("false"));
            let mut value = || written_value.clone().or_else(|| arguments.next());
            match name.as_str() {
                "preview" => options.preview = value(),
                "show" => options.show = value(),
                "screenshot" => options.screenshot = value().map(PathBuf::from),
                "size" => options.size = value().as_deref().and_then(parse_size),
                "monitor" => options.monitor = value().and_then(|text| text.parse().ok()),
                "position" => options.position = value().as_deref().and_then(parse_position),
                "cpuonly" => options.cpu_only = switched_on,
                "verbose" => options.verbose = switched_on,
                "onboard" => options.onboard = switched_on,
                "skiponboard" => options.skip_onboard = switched_on,
                "mcp" => options.mcp = switched_on,
                "mcpport" => {
                    options.mcp = true;
                    options.mcp_port = value().and_then(|text| text.parse().ok());
                }
                "diagnostics" => options.show_also("diagnostics"),
                "nostart" => options.no_start = switched_on,
                "safemode" => options.safe_mode = switched_on,
                "proxy" => options.proxy = value().filter(|address| !address.is_empty()),
                "?" | "h" | "help" => options.help = true,
                _ => options.ignored.push(argument),
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

    /// A run that checks the editor (`--screenshot`, `--show`).
    fn checking(&self) -> bool {
        self.screenshot.is_some() || self.show.is_some()
    }

    /// The start experience runs (`Shell_Loaded` without `NoStart`): the
    /// Start dialog shows, or the project given opens in its place, and
    /// What's New shows the first time a version starts.
    pub fn greets(&self) -> bool {
        !self.no_start && !self.checking()
    }

    /// The Start dialog shows: the start experience, with no project to
    /// open instead.
    pub fn start_dialog(&self) -> bool {
        self.greets() && self.project.is_none()
    }

    /// The window opens maximised on its monitor (`Shell_Loaded`), unless
    /// its size or place is given, or the run is a capture.
    pub fn maximised(&self) -> bool {
        self.size.is_none() && self.position.is_none() && self.screenshot.is_none()
    }
}

/// An option's name, lower-cased and without its prefix (`--`, `-`, or on
/// Windows `/`) or the hyphens and underscores between words, and the
/// value written after `:` or `=`. None for an argument that is not an
/// option.
fn option_name(argument: &str) -> Option<(String, Option<String>)> {
    let unprefixed = if let Some(rest) = argument.strip_prefix("--") {
        rest
    } else if let Some(rest) = argument.strip_prefix('-') {
        rest
    } else if cfg!(windows)
        && let Some(rest) = argument.strip_prefix('/')
    {
        rest
    } else {
        return None;
    };
    let (written_name, written_value) = match unprefixed.find([':', '=']) {
        Some(at) => (&unprefixed[..at], Some(unprefixed[at + 1..].to_string())),
        None => (unprefixed, None),
    };
    let kept = written_name.chars().filter(|character| *character != '-' && *character != '_');
    let name = kept.flat_map(char::to_lowercase).collect();
    Some((name, written_value))
}

/// What `-?` lists (`App.ShowHelp`'s usage).
pub const USAGE: &str = "\
Usage: nerthus [project] [options]

    project
        The project to open.

    --no-start
        Skips the Start dialog.

    --safe-mode
        Starts with the default preferences and options.

    --proxy <url>
        Turns the proxy on with this address, and saves it.

    --cpu-only
        Computes everything on the CPU.

    --verbose
        Turns Verbose Logging on for this session.

    --onboard
        Shows the first-run setup.

    --skip-onboard
        Marks the first-run setup done without showing it.

    --mcp
        Starts the MCP server for this session.

    --mcp-port <port>
        Starts the MCP server on this port.

    --diagnostics
        Opens Diagnostics.

    -?, --help
        Shows this help.

For checking the editor:

    --preview <node>
        Previews this node of the project.

    --show <window>
        Opens a window, such as build-settings or options:general.

    --screenshot <png>
        Saves a picture of the window once the preview is built, and quits.

    --size <width>x<height>, --position <x>,<y>, --monitor <index>
        Sizes and places the window.

Names are case-insensitive and may leave out the dashes between words:
-NoStart is --no-start.
";

/// `App.ShowHelp`: the options, printed and in a message box, for
/// Nerthus to quit after.
pub fn show_usage() {
    print!("{USAGE}");
    let description = format!("Command Line Help\n\n{USAGE}");
    let mut dialog = rfd::MessageDialog::new();
    dialog = dialog.set_title("Nerthus - CLI Usage");
    dialog = dialog.set_description(description);
    dialog = dialog.set_level(rfd::MessageLevel::Info);
    dialog = dialog.set_buttons(rfd::MessageButtons::Ok);
    dialog.show();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> LaunchOptions {
        let owned = arguments.iter().map(|argument| argument.to_string());
        LaunchOptions::parse(owned)
    }

    #[test]
    fn options_read_in_either_spelling() {
        let options = parse(&["-NoStart", "--safe-mode", "-PROXY", "http://proxy:8080", "--Cpu-Only", "-verbose:false"]);
        assert!(options.no_start);
        assert!(options.safe_mode);
        assert_eq!(options.proxy.as_deref(), Some("http://proxy:8080"));
        assert!(options.cpu_only);
        assert!(!options.verbose);
    }

    #[test]
    fn values_can_follow_a_colon_or_equals_sign() {
        let options = parse(&["-Proxy:http://proxy:8080", "--mcp-port=47199"]);
        assert_eq!(options.proxy.as_deref(), Some("http://proxy:8080"));
        assert_eq!(options.mcp_port, Some(47199));
        assert!(options.mcp);
    }

    #[test]
    fn an_empty_proxy_changes_nothing() {
        let options = parse(&["-Proxy", ""]);
        assert_eq!(options.proxy, None);
    }

    #[test]
    fn unknown_options_and_later_paths_are_ignored() {
        let options = parse(&["first.terrain", "-Bogus", "second.terrain", "--"]);
        assert_eq!(options.project, Some(PathBuf::from("first.terrain")));
        assert_eq!(options.ignored, ["-Bogus", "second.terrain", "--"]);
    }

    #[test]
    fn the_question_mark_asks_for_help() {
        assert!(parse(&["-?"]).help);
        assert!(parse(&["--help"]).help);
        assert!(!parse(&["project.terrain"]).help);
    }

    #[test]
    fn the_start_dialog_greets_plain_starts_and_options() {
        assert!(parse(&[]).start_dialog());
        assert!(parse(&["--verbose", "-SafeMode"]).start_dialog());
        assert!(!parse(&["-NoStart"]).start_dialog());
        assert!(!parse(&["--screenshot", "out.png"]).start_dialog());
        let with_project = parse(&["project.terrain"]);
        assert!(!with_project.start_dialog());
        assert!(with_project.greets());
    }

    #[test]
    fn the_window_is_maximised_unless_placed_or_captured() {
        assert!(parse(&[]).maximised());
        assert!(parse(&["--monitor", "1"]).maximised());
        assert!(!parse(&["--size", "1600x960"]).maximised());
        assert!(!parse(&["--position", "-276,-1080"]).maximised());
        assert!(!parse(&["--screenshot", "out.png"]).maximised());
    }
}
