//! Logging as `Logging` sets it up: every message of the session
//! kept in memory (the Console's `printlog`, the crash log), and written to
//! a log file in the data folder's `Logs` (`<yyyy-MM-dd_HH-mm-ss>.txt`, or
//! `…--VERBOSE.txt` with Verbose Logging, which also keeps the debugging
//! messages). Verbose Logging is on when `verbose.dat` is in the data
//! folder (Options › General) or Nerthus starts with
//! `--verbose`, and Tools › Verbose Logging switches it for the session,
//! starting a new log file.

use std::fmt::Write as _;
use std::fs::File;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use bevy::app::App;
use bevy::log::tracing::field::{Field, Visit};
use bevy::log::tracing::{Event, Level, Subscriber};
use bevy::log::tracing_subscriber::filter::FilterFn;
use bevy::log::tracing_subscriber::layer::Context;
use bevy::log::tracing_subscriber::{self, Layer};
use bevy::log::{BoxedFmtLayer, BoxedLayer};

/// The file in the data folder that turns Verbose Logging on at start.
pub const VERBOSE_FILE: &str = "verbose.dat";

/// How much of the session the memory log keeps (the oldest goes first).
const MEMORY_LIMIT: usize = 8 << 20;

static VERBOSE: AtomicBool = AtomicBool::new(false);
static SESSION: OnceLock<Mutex<SessionLog>> = OnceLock::new();

/// The session's log: the text in memory and the file being written.
#[derive(Default)]
struct SessionLog {
    text: String,
    file: Option<File>,
    path: Option<PathBuf>,
}

fn session() -> &'static Mutex<SessionLog> {
    SESSION.get_or_init(|| Mutex::new(SessionLog::default()))
}

/// The data folder's `Logs`.
pub fn logs_folder() -> Option<PathBuf> {
    crate::user::data_folder().map(|folder| folder.join("Logs"))
}

/// Whether Verbose Logging is on.
pub fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

/// Whether Verbose Logging is set to be on at start (Options › General).
pub fn verbose_at_start() -> bool {
    crate::user::data_folder().is_some_and(|folder| folder.join(VERBOSE_FILE).exists())
}

/// Options › General's "Enable Verbose Logging": the flag file written or
/// deleted, for the next start.
pub fn set_verbose_at_start(on: bool) -> std::io::Result<()> {
    let Some(folder) = crate::user::data_folder() else {
        return Ok(());
    };
    let path = folder.join(VERBOSE_FILE);
    if on {
        std::fs::create_dir_all(&folder)?;
        std::fs::write(path, "!")
    } else if path.exists() {
        std::fs::remove_file(path)
    } else {
        Ok(())
    }
}

/// The session's messages so far.
pub fn session_text() -> String {
    session().lock().map(|log| log.text.clone()).unwrap_or_default()
}

/// The log file being written.
pub fn current_log_file() -> Option<PathBuf> {
    session().lock().ok().and_then(|log| log.path.clone())
}

/// `Logging.Initialize(verbose)`: a new log file for the rest of the
/// session, then the session's id and what it runs on.
pub fn start(verbose_logging: bool) {
    VERBOSE.store(verbose_logging, Ordering::Relaxed);
    open_file(verbose_logging);
    log::info!("Session: {}", uuid_text());
    log::info!("Version {}", env!("CARGO_PKG_VERSION"));
    log::info!("OS Version {} {}", std::env::consts::OS, std::env::consts::ARCH);
    let machine = std::env::var("HOSTNAME").or_else(|_| std::env::var("COMPUTERNAME")).unwrap_or_default();
    if !machine.is_empty() {
        log::info!("Machine name is {machine}");
    }
    if let Ok(path) = std::env::current_exe() {
        log::info!("Process path {}", path.display());
    }
    if let Ok(directory) = std::env::current_dir() {
        log::info!("Executing in {}", directory.display());
    }
    let cores = std::thread::available_parallelism().map(usize::from).unwrap_or(1);
    log::info!("{cores} cores");
}

/// Tools › Verbose Logging, once confirmed (`BtnVerbose_OnChecked` and
/// `OnUnchecked`): the log closed and a new one started at the new level.
pub fn switch_verbose(verbose_logging: bool) {
    if verbose_logging {
        log::warn!("Switching to Verbose Logging. Closing File.");
    } else {
        log::warn!("Switching to Standard Logging. Closing File.");
    }
    VERBOSE.store(verbose_logging, Ordering::Relaxed);
    open_file(verbose_logging);
    if !verbose_logging {
        log::warn!("This is a mid-session Verbose file. See previous log for additional information on this session.");
    }
}

/// Going on after a crash (`Logging.Initialize(false)`): a new log file
/// at the standard level.
pub fn resume_after_crash() {
    VERBOSE.store(false, Ordering::Relaxed);
    open_file(false);
    log::info!("RESUMING AFTER CRASH");
}

/// A random id for the session.
fn uuid_text() -> String {
    let nanoseconds = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|since| since.as_nanos()).unwrap_or(0);
    let process = process_id() as u128;
    let mixed = nanoseconds ^ (process << 64) ^ 0x9e37_79b9_7f4a_7c15_f39c_c060_5ced_c834;
    let hex = format!("{mixed:032x}");
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

#[cfg(not(target_arch = "wasm32"))]
fn process_id() -> u32 {
    std::process::id()
}

/// A browser page has no process id.
#[cfg(target_arch = "wasm32")]
fn process_id() -> u32 {
    0
}

fn open_file(verbose_logging: bool) {
    let Some(folder) = logs_folder() else {
        return;
    };
    let _ = std::fs::create_dir_all(&folder);
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
    let suffix = if verbose_logging { "--VERBOSE" } else { "" };
    let path = folder.join(format!("{stamp}{suffix}.txt"));
    let file = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok();
    if let Ok(mut log) = session().lock() {
        log.file = file;
        log.path = Some(path);
    }
}

/// Serilog's three-letter levels.
fn level_name(level: &Level) -> &'static str {
    match *level {
        Level::TRACE => "VRB",
        Level::DEBUG => "DBG",
        Level::INFO => "INF",
        Level::WARN => "WRN",
        Level::ERROR => "ERR",
    }
}

/// Whether a level is at least Information (`LogEventLevel.Information`).
fn informational(level: &Level) -> bool {
    matches!(*level, Level::INFO | Level::WARN | Level::ERROR)
}

/// A message's text: its `message` field, then the others.
#[derive(Default)]
struct MessageText {
    message: String,
    others: String,
}

impl Visit for MessageText {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else if !field.name().starts_with("log.") {
            let _ = write!(self.others, " {}={value}", field.name());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else if !field.name().starts_with("log.") {
            let _ = write!(self.others, " {}={value:?}", field.name());
        }
    }
}

/// The layer that keeps the session's messages.
struct SessionLayer;

impl<S: Subscriber> Layer<S> for SessionLayer {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let level = *event.metadata().level();
        let mut text = MessageText::default();
        event.record(&mut text);
        let stamp = chrono::Local::now().format("%H:%M:%S");
        let name = level_name(&level);
        let message = format!("{}{}", text.message, text.others);
        let Ok(mut log) = session().lock() else {
            return;
        };
        let _ = writeln!(log.text, "{stamp} {name} {message}");
        if log.text.len() > MEMORY_LIMIT {
            let cut = log.text.len() - MEMORY_LIMIT / 2;
            let boundary = log.text[cut..].find('\n').map(|offset| cut + offset + 1).unwrap_or(cut);
            log.text.drain(..boundary);
        }
        if verbose() || informational(&level) {
            let line = format!("[{stamp}] {name} {message}\n");
            if let Some(file) = log.file.as_mut() {
                let _ = file.write_all(line.as_bytes());
            }
        }
    }
}

/// `LogPlugin::custom_layer`: the session log.
pub fn session_layer(_app: &mut App) -> Option<BoxedLayer> {
    Some(Box::new(SessionLayer))
}

/// `LogPlugin::fmt_layer`: the terminal's output, debugging messages only
/// with Verbose Logging.
pub fn terminal_layer(_app: &mut App) -> Option<BoxedFmtLayer> {
    let filter = FilterFn::new(|metadata| verbose() || informational(metadata.level()));
    let layer = tracing_subscriber::fmt::Layer::default().with_writer(std::io::stderr).with_filter(filter);
    Some(Box::new(layer))
}

/// A crash's logs (the unhandled exception handlers): the panic logged as
/// fatal, then the session so far written to `Logs/CRASH_<time>.txt` and
/// the panic with its backtrace to `Logs/CRASH_Error_<time>.txt`.
pub fn install_crash_log() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let error = format!("{info}\n{backtrace}");
        record_fatal(&format!("Unhandled Exception: {info}"));
        // The log may be locked by the thread that panicked.
        let kept = session().try_lock().map(|log| log.text.clone()).unwrap_or_default();
        if let Some(folder) = logs_folder() {
            let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
            let _ = std::fs::create_dir_all(&folder);
            let _ = std::fs::write(folder.join(format!("CRASH_{stamp}.txt")), kept);
            let _ = std::fs::write(folder.join(format!("CRASH_Error_{stamp}.txt")), error);
        }
        previous(info);
    }));
}

/// A fatal message (`Log.Fatal`), written straight into the session's log
/// rather than through the logger, which the panicking thread may hold.
fn record_fatal(message: &str) {
    let Ok(mut log) = session().try_lock() else {
        return;
    };
    let stamp = chrono::Local::now().format("%H:%M:%S");
    let _ = writeln!(log.text, "{stamp} FTL {message}");
    let line = format!("[{stamp}] FTL {message}\n");
    if let Some(file) = log.file.as_mut() {
        let _ = file.write_all(line.as_bytes());
    }
}
