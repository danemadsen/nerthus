//! The log of a `nerthus-build` run. Everything logged is kept in memory,
//! for a crash's log; what is informational or worse (everything, with
//! `--verbose`) also goes to a log file in the data folder's `Logs`:
//! `<yyyy-MM-dd_HH-mm-ss>-BUILD.txt` for a build, `…-BUILDHOST.txt` for the
//! build host, each `…--VERBOSE.txt` with `--verbose`. A crash leaves the
//! log so far as `Logs/CRASH_BUILD_<time>.txt` (`CRASH_BUILDHOST_…` for the
//! host), and as `CRASH_LOG.txt` in the folder of the build it stopped.

use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use log::{Level, LevelFilter, Log, Metadata, Record};

/// How much of the run the memory log keeps (the oldest goes first).
const MEMORY_LIMIT: usize = 8 << 20;

/// What kind of run the log is of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogKind {
    Build,
    Host,
}

impl LogKind {
    /// The name the run's files carry.
    fn tag(self) -> &'static str {
        match self {
            LogKind::Build => "BUILD",
            LogKind::Host => "BUILDHOST",
        }
    }
}

/// The run's log: the text in memory, the file being written and the
/// folder of the build running, if any.
#[derive(Default)]
struct LogState {
    text: String,
    file: Option<File>,
    build_folder: Option<PathBuf>,
}

struct RunLog {
    verbose: bool,
    state: Mutex<LogState>,
}

static LOG: OnceLock<RunLog> = OnceLock::new();

/// The data folder's `Logs`.
pub fn logs_folder() -> Option<PathBuf> {
    nerthus_engine::paths::data_folder().map(|folder| folder.join("Logs"))
}

/// The log file's name for a run started at `stamp`.
fn file_name(kind: LogKind, verbose: bool, stamp: &str) -> String {
    let suffix = if verbose { "--VERBOSE" } else { "" };
    format!("{stamp}-{}{suffix}.txt", kind.tag())
}

/// Serilog's three-letter levels.
fn level_name(level: Level) -> &'static str {
    match level {
        Level::Trace => "VRB",
        Level::Debug => "DBG",
        Level::Info => "INF",
        Level::Warn => "WRN",
        Level::Error => "ERR",
    }
}

/// Whether a message is kept at all: the GPU libraries' chatter is left
/// out unless it reports a problem.
fn kept(metadata: &Metadata) -> bool {
    let target = metadata.target();
    if target.starts_with("wgpu") {
        return metadata.level() <= Level::Error;
    }
    if target.starts_with("naga") {
        return metadata.level() <= Level::Warn;
    }
    true
}

impl Log for RunLog {
    fn enabled(&self, metadata: &Metadata) -> bool {
        kept(metadata)
    }

    fn log(&self, record: &Record) {
        if !kept(record.metadata()) {
            return;
        }
        let stamp = chrono::Local::now().format("%H:%M:%S");
        let name = level_name(record.level());
        let message = record.args();
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let line = format!("{stamp} {name} {message}\n");
        state.text.push_str(&line);
        if state.text.len() > MEMORY_LIMIT {
            let cut = state.text.len() - MEMORY_LIMIT / 2;
            let boundary = state.text[cut..].find('\n').map(|offset| cut + offset + 1).unwrap_or(cut);
            state.text.drain(..boundary);
        }
        if self.verbose || record.level() <= Level::Info {
            let file_line = format!("[{stamp}] {name} {message}\n");
            if let Some(file) = state.file.as_mut() {
                let _ = file.write_all(file_line.as_bytes());
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut state) = self.state.lock()
            && let Some(file) = state.file.as_mut()
        {
            let _ = file.flush();
        }
    }
}

/// Starts the run's log (once; later calls do nothing) and its crash log.
/// Returns the log file, when one could be made.
pub fn start(kind: LogKind, verbose: bool) -> Option<PathBuf> {
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let path = logs_folder().map(|folder| folder.join(file_name(kind, verbose, &stamp)));
    let file = path.as_deref().and_then(open_log);
    let opened = file.is_some();
    let state = LogState { file, ..LogState::default() };
    let run_log = RunLog { verbose, state: Mutex::new(state) };
    if LOG.set(run_log).is_err() {
        return None;
    }
    if let Some(installed) = LOG.get() {
        let _ = log::set_logger(installed);
        log::set_max_level(LevelFilter::Trace);
    }
    install_crash_log(kind);
    if opened { path } else { None }
}

/// The log file, made with its folder; none when it cannot be.
fn open_log(path: &Path) -> Option<File> {
    std::fs::create_dir_all(path.parent()?).ok()?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true);
    options.append(true);
    options.open(path).ok()
}

/// What the run is and runs on (the first lines of every log).
pub fn log_details() {
    log::info!("Version {}", env!("CARGO_PKG_VERSION"));
    log::info!("OS Version {} {}", std::env::consts::OS, std::env::consts::ARCH);
    let machine = gethostname::gethostname();
    log::info!("Machine name is {}", machine.to_string_lossy());
    if let Ok(path) = std::env::current_exe() {
        log::info!("Process path {}", path.display());
    }
    if let Ok(directory) = std::env::current_dir() {
        log::info!("Executing in {}", directory.display());
    }
    let cores = std::thread::available_parallelism().map(usize::from).unwrap_or(1);
    log::info!("{cores} cores");
}

/// The folder of the build that is running, for a crash's `CRASH_LOG.txt`
/// (none once it is over).
pub fn set_build_folder(folder: Option<&Path>) {
    let Some(run_log) = LOG.get() else {
        return;
    };
    if let Ok(mut state) = run_log.state.lock() {
        state.build_folder = folder.map(Path::to_path_buf);
    }
}

/// A crash's log: the run so far, with what happened, written to
/// `Logs/CRASH_<kind>_<time>.txt` and to the running build's folder.
fn install_crash_log(kind: LogKind) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // The log may be locked by the thread that panicked.
        let locked = LOG.get().and_then(|run_log| run_log.state.try_lock().ok());
        let (kept_text, build_folder) = match locked {
            Some(state) => (state.text.clone(), state.build_folder.clone()),
            None => (String::new(), None),
        };
        let text = format!("{kept_text}\n{info}\n");
        if let Some(folder) = logs_folder() {
            let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
            let _ = std::fs::create_dir_all(&folder);
            let _ = std::fs::write(folder.join(format!("CRASH_{}_{stamp}.txt", kind.tag())), &text);
        }
        if let Some(folder) = build_folder {
            let _ = std::fs::write(folder.join("CRASH_LOG.txt"), &text);
        }
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_files_are_named_by_time_and_kind() {
        let stamp = "2026-10-13_14-05-12";
        assert_eq!(file_name(LogKind::Build, false, stamp), "2026-10-13_14-05-12-BUILD.txt");
        assert_eq!(file_name(LogKind::Build, true, stamp), "2026-10-13_14-05-12-BUILD--VERBOSE.txt");
        assert_eq!(file_name(LogKind::Host, false, stamp), "2026-10-13_14-05-12-BUILDHOST.txt");
    }

    #[test]
    fn gpu_library_chatter_is_left_out() {
        let debug = Metadata::builder().level(Level::Debug).target("wgpu_core::device").build();
        let error = Metadata::builder().level(Level::Error).target("wgpu_hal::metal").build();
        let warning = Metadata::builder().level(Level::Warn).target("naga::front").build();
        let ours = Metadata::builder().level(Level::Trace).target("nerthus_export::swarm").build();
        assert!(!kept(&debug));
        assert!(kept(&error));
        assert!(kept(&warning));
        assert!(kept(&ours));
    }
}
