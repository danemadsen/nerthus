//! The build host: a `nerthus-build` that stays loaded and builds the
//! projects later runs hand to it. The first `nerthus-build host` becomes
//! the host: it loads the preferences, the compute devices and the nodes
//! once, builds the project it was given (if any) and waits for more.
//! Later runs hand their project and options to it, print what its build
//! prints and exit with the build's exit code; `nerthus-build host --exit`
//! closes it. The options about the run itself (`--verbose`, `--safemode`,
//! `--cpu-only`) count when the host starts, and the host asks no
//! questions.
//!
//! One host runs per user. It holds the lock on `build-host.lock` in the
//! data folder while it runs, and listens on a loopback port that
//! `build-host.dat` names along with a random key; only this user can read
//! the file, and a run must send the key before anything else. A run then
//! sends one line, its working folder and its arguments separated by
//! U+001F, and hears the build's output as [`Console`] sends it.

use std::fs::{self, File, TryLockError};
use std::io::{BufRead, BufReader, Write as _};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{Args, Parser};

use crate::build::{self, BuildOptions, Console, Environment};
use crate::logging::{self, LogKind};

/// The files in the data folder: the lock the host holds, and where it
/// listens.
const LOCK_FILE: &str = "build-host.lock";
const ADDRESS_FILE: &str = "build-host.dat";
/// Between the fields of a request.
const SEPARATOR: char = '\u{1f}';
/// How long a run waits for a starting host to listen.
const CONNECT_PATIENCE: Duration = Duration::from_secs(10);
/// How long the host waits for a run to say what it wants.
const REQUEST_PATIENCE: Duration = Duration::from_secs(5);
/// `--selfcheck`'s exit code once the host started.
const SELF_CHECK_CODE: u8 = 102;

/// `nerthus-build host`'s arguments.
#[derive(Args, Clone, Debug)]
pub struct HostArgs {
    /// A project to build: in the host this run starts, or handed to the
    /// running host.
    pub file: Option<PathBuf>,
    #[command(flatten)]
    pub options: BuildOptions,
    /// Closes the running build host.
    #[arg(long)]
    pub exit: bool,
    /// Starts the host's environment (preferences, devices and nodes),
    /// then exits with code 102: a check that the host can start.
    #[arg(long)]
    pub selfcheck: bool,
}

/// A request's arguments, read as `nerthus-build host` reads its own.
#[derive(Parser)]
#[command(name = "nerthus-build host", no_binary_name = true)]
struct Request {
    #[command(flatten)]
    arguments: HostArgs,
}

/// `nerthus-build host`: this run becomes the host, or hands its build to
/// the host that is running. `words` are its arguments after `host`.
pub fn command(arguments: HostArgs, words: Vec<String>) -> ExitCode {
    let Some(folder) = nerthus_engine::paths::data_folder() else {
        eprintln!("Error: there is no data folder for the build host to work from");
        return ExitCode::FAILURE;
    };
    if let Err(error) = fs::create_dir_all(&folder) {
        eprintln!("Error: cannot create {}: {error}", folder.display());
        return ExitCode::FAILURE;
    }
    let lock_path = folder.join(LOCK_FILE);
    let mut lock_options = File::options();
    lock_options.write(true);
    lock_options.create(true);
    lock_options.truncate(false);
    let lock = match lock_options.open(&lock_path) {
        Ok(lock) => lock,
        Err(error) => {
            eprintln!("Error: cannot open {}: {error}", lock_path.display());
            return ExitCode::FAILURE;
        }
    };
    match lock.try_lock() {
        Ok(()) if arguments.exit => {
            println!("No build host is running.");
            ExitCode::SUCCESS
        }
        Ok(()) => serve(&arguments, &folder, lock),
        Err(TryLockError::WouldBlock) => hand_over(&folder, &words),
        Err(TryLockError::Error(error)) => {
            eprintln!("Error: cannot lock {}: {error}", lock_path.display());
            ExitCode::FAILURE
        }
    }
}

/// The host: listening first (so runs can queue up while it starts), then
/// the environment loaded, its own project built, and the requests
/// answered one at a time until one says `--exit`. `_lock` is held until
/// it closes.
fn serve(arguments: &HostArgs, folder: &Path, _lock: File) -> ExitCode {
    let started = Instant::now();
    let options = &arguments.options;
    logging::start(LogKind::Host, options.verbose);
    let listener = match TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("Error: the build host cannot listen: {error}");
            return ExitCode::FAILURE;
        }
    };
    let key = uuid::Uuid::new_v4().simple().to_string();
    let port = listener.local_addr().map(|address| address.port()).unwrap_or_default();
    if let Err(error) = write_address(folder, port, &key) {
        eprintln!("Error: cannot write {}: {error}", folder.join(ADDRESS_FILE).display());
        return ExitCode::FAILURE;
    }
    log::info!("Build host listening on port {port}");

    let mut console = Console::terminal(options.silent);
    let environment = build::environment(options, &mut console);
    let milliseconds = started.elapsed().as_millis();
    console.line(&format!("Took {}ms to initialize.", grouped(milliseconds)));
    if arguments.selfcheck {
        remove_address(folder);
        return ExitCode::from(SELF_CHECK_CODE);
    }
    if let Some(file) = &arguments.file {
        let outcome = build::run_build(file, options, &environment, &mut console, false);
        build::exit_code(&outcome, &mut console);
    }
    console.line("Idle. Awaiting command...");
    for incoming in listener.incoming() {
        let Ok(stream) = incoming else {
            continue;
        };
        match answer(stream, &key, &environment) {
            Answer::Exit => break,
            Answer::Answered => console.line("Idle. Awaiting command..."),
            Answer::TurnedAway => {}
        }
    }
    remove_address(folder);
    log::info!("Build host closed.");
    ExitCode::SUCCESS
}

/// What became of a request.
enum Answer {
    Answered,
    /// It said `--exit`.
    Exit,
    /// It had no key, or not the host's.
    TurnedAway,
}

/// One request: the key checked, the arguments read as the command line
/// would read them (paths relative to the run's working folder), the
/// build run, its output and exit code sent back.
fn answer(stream: TcpStream, key: &str, environment: &Environment) -> Answer {
    let _ = stream.set_read_timeout(Some(REQUEST_PATIENCE));
    let Ok(reading) = stream.try_clone() else {
        return Answer::TurnedAway;
    };
    let mut reader = BufReader::new(reading);
    let mut given_key = String::new();
    if reader.read_line(&mut given_key).is_err() || given_key.trim_end() != key {
        log::warn!("A connection without the build host's key was turned away.");
        return Answer::TurnedAway;
    }
    let mut request = String::new();
    if reader.read_line(&mut request).is_err() {
        return Answer::TurnedAway;
    }
    let _ = stream.set_read_timeout(None);
    let mut fields = request.trim_end_matches(['\r', '\n']).split(SEPARATOR);
    let working_folder = PathBuf::from(fields.next().unwrap_or_default());
    let words: Vec<String> = fields.map(str::to_string).collect();

    let mut console = Console::handed_over(stream);
    let joined = words.join(" ");
    log::info!("Build host received: {joined}");
    console.line(&format!("Build host received: {joined}"));
    let arguments = match Request::try_parse_from(&words) {
        Ok(request) => request.arguments,
        Err(error) => {
            let shown = error.render().to_string();
            let asked = matches!(error.kind(), clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion);
            if asked {
                console.text(&shown);
                console.finish(0);
            } else {
                console.problem(shown.trim_end());
                console.finish(2);
            }
            return Answer::Answered;
        }
    };
    if arguments.exit {
        console.line("Closing the build host.");
        console.finish(0);
        return Answer::Exit;
    }
    let Some(file) = &arguments.file else {
        console.problem("ERROR! No project to build.");
        console.finish(2);
        return Answer::Answered;
    };
    let file = in_folder(&working_folder, file);
    if !file.is_file() {
        console.problem("ERROR! File not found.");
        console.finish(1);
        return Answer::Answered;
    }
    let mut options = arguments.options;
    options.output = options.output.map(|path| in_folder(&working_folder, &path));
    options.vars = options.vars.map(|path| in_folder(&working_folder, &path));
    options.macros = options.macros.map(|path| in_folder(&working_folder, &path));
    console.set_silent(options.silent);
    let outcome = build::run_build(&file, &options, environment, &mut console, false);
    let code = build::exit_code(&outcome, &mut console);
    console.finish(code);
    Answer::Answered
}

/// `path` as the run that sent it meant it: relative to its working
/// folder.
fn in_folder(folder: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    folder.join(path)
}

/// A run handing its build to the host: connected (waiting for a host
/// that is still starting), the request sent, the build's output printed
/// as it comes, then the build's exit code.
fn hand_over(folder: &Path, words: &[String]) -> ExitCode {
    if words.iter().any(|word| word.contains(['\n', '\r', SEPARATOR])) {
        eprintln!("Error: the build host cannot take arguments with line breaks");
        return ExitCode::FAILURE;
    }
    let deadline = Instant::now() + CONNECT_PATIENCE;
    let (mut stream, key) = loop {
        if let Some((port, key)) = read_address(folder) {
            let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
            if let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_secs(2)) {
                break (stream, key);
            }
        }
        if Instant::now() >= deadline {
            eprintln!("Error: the build host is not answering");
            return ExitCode::FAILURE;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let working_folder = std::env::current_dir().unwrap_or_default();
    let mut request = working_folder.to_string_lossy().into_owned();
    for word in words {
        request.push(SEPARATOR);
        request.push_str(word);
    }
    let message = format!("{key}\n{request}\n");
    if let Err(error) = stream.write_all(message.as_bytes()) {
        eprintln!("Error: cannot reach the build host: {error}");
        return ExitCode::FAILURE;
    }
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else {
            break;
        };
        if let Some(text) = line.strip_prefix(':') {
            println!("{text}");
        } else if let Some(text) = line.strip_prefix('!') {
            eprintln!("{text}");
        } else if let Some(code) = line.strip_prefix("/exit") {
            let code = code.trim().parse::<u8>().unwrap_or(1);
            return ExitCode::from(code);
        } else {
            eprintln!("Error: the build host's answer is not understood: {line}");
            return ExitCode::FAILURE;
        }
    }
    eprintln!("Error: the build host closed the connection before the build ended");
    ExitCode::FAILURE
}

/// `build-host.dat`: the port the host listens on and its key, readable
/// by this user only, replaced in one step.
fn write_address(folder: &Path, port: u16, key: &str) -> std::io::Result<()> {
    let path = folder.join(ADDRESS_FILE);
    let temporary = folder.join(format!("{ADDRESS_FILE}.{}", std::process::id()));
    let mut options = File::options();
    options.write(true);
    options.create(true);
    options.truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(format!("{port} {key}\n").as_bytes())?;
    drop(file);
    fs::rename(&temporary, &path)
}

/// The port and key `build-host.dat` holds.
fn read_address(folder: &Path) -> Option<(u16, String)> {
    let text = fs::read_to_string(folder.join(ADDRESS_FILE)).ok()?;
    let (port, key) = text.trim().split_once(' ')?;
    Some((port.parse().ok()?, key.to_string()))
}

fn remove_address(folder: &Path) {
    let _ = fs::remove_file(folder.join(ADDRESS_FILE));
}

/// A whole number with thousands separators (`###,###,###`).
fn grouped(value: u128) -> String {
    let digits = value.to_string();
    let mut text = String::new();
    for (index, digit) in digits.chars().enumerate() {
        let left = digits.len() - index;
        if index > 0 && left.is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_file_round_trips() {
        let folder = std::env::temp_dir().join(format!("nerthus-build-host-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        assert_eq!(read_address(&folder), None);
        write_address(&folder, 52815, "0123abcd").unwrap();
        assert_eq!(read_address(&folder), Some((52815, "0123abcd".to_string())));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(folder.join(ADDRESS_FILE)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        remove_address(&folder);
        assert_eq!(read_address(&folder), None);
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn requests_read_like_the_command_line() {
        let words = ["Canyon.terrain", "--resolution", "2048", "-v", "Height=0.5", "--silent"];
        let request = Request::try_parse_from(words).unwrap();
        assert_eq!(request.arguments.file, Some(PathBuf::from("Canyon.terrain")));
        assert_eq!(request.arguments.options.resolution, Some(2048));
        assert_eq!(request.arguments.options.variables, ["Height=0.5"]);
        assert!(request.arguments.options.silent);
        assert!(!request.arguments.exit);
        let exit = Request::try_parse_from(["--exit"]).unwrap();
        assert!(exit.arguments.exit);
        assert!(Request::try_parse_from(["--no-such-option"]).is_err());
    }

    #[test]
    fn relative_paths_are_the_runs() {
        let folder = Path::new("/work/projects");
        assert_eq!(in_folder(folder, Path::new("Canyon.terrain")), PathBuf::from("/work/projects/Canyon.terrain"));
        assert_eq!(in_folder(folder, Path::new("/elsewhere/Canyon.terrain")), PathBuf::from("/elsewhere/Canyon.terrain"));
    }

    #[test]
    fn milliseconds_are_grouped() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1234), "1,234");
        assert_eq!(grouped(1234567), "1,234,567");
    }
}
