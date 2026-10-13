//! The build command: a project built and exported as the command-line
//! build does it, in this process or in the build host
//! ([`crate::host`]). A run first loads the user's preferences (or none,
//! in Safe Mode) and the compute devices they name; the build host does
//! that once for all its builds.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, IsTerminal, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use clap::Args;
use nerthus_engine::macros::load_project_macros;
use nerthus_engine::{Engine, Project, Registry};
use nerthus_export::build::{self, BuildDefinition, BuildEvent, ExportPreferences, RunOptions};
use nerthus_export::preferences::{self, PreferenceValues};
use nerthus_export::swarm::{self, VariableAssignment};

use crate::logging::{self, LogKind};

/// A build's options besides the project.
#[derive(Args, Clone, Debug, Default)]
pub struct BuildOptions {
    /// Build resolution (defaults to the project's; a Region build
    /// builds at the region's size).
    #[arg(long)]
    pub resolution: Option<usize>,
    /// Builds this region of the terrain instead of the one the build
    /// settings name.
    #[arg(long, short = 'r')]
    pub region: Option<String>,
    /// Build folder (replaces the project's destination template).
    #[arg(long, alias = "buildpath")]
    pub output: Option<PathBuf>,
    /// Builds with this build profile (Build Settings › Profiles)
    /// instead of the saved build settings.
    #[arg(long, short = 'p')]
    pub profile: Option<String>,
    /// Sets a variable for this build, e.g. `-v Height=0.63`; repeat
    /// for more.
    #[arg(short = 'v', long = "variable", value_name = "NAME=VALUE")]
    pub variables: Vec<String>,
    /// Every variable's value, comma separated, in the order the
    /// project lists its variables.
    #[arg(long = "va", value_name = "VALUES", value_delimiter = ',')]
    pub values: Vec<String>,
    /// A `.json` object or a `.txt` file of `name=value` lines setting
    /// variables.
    #[arg(long)]
    pub vars: Option<PathBuf>,
    /// Builds a mutation of the terrain: every node's seed moved on by
    /// this number, also `[Mutation]` in the build folder's name.
    #[arg(long, default_value_t = 0)]
    pub seed: i32,
    /// Builds the baked nodes instead of reading their baked maps
    /// (Standard builds).
    #[arg(long, alias = "ignore-cache")]
    pub ignorecache: bool,
    /// Prints only what goes wrong, leaves no report in the build
    /// folder and never opens it.
    #[arg(long)]
    pub silent: bool,
    /// Asks which build profile to use when the project has more than
    /// one and none is given, and whether to open the build folder
    /// once built (needs a terminal).
    #[arg(long)]
    pub interactive: bool,
    /// Folder holding the project's macros.
    #[arg(long)]
    pub macros: Option<PathBuf>,
    /// Builds without applying bindings and expressions (a Community
    /// build).
    #[arg(long)]
    pub community: bool,
    /// The TIFF 32-bit Artist tag.
    #[arg(long)]
    pub artist: Option<String>,
    /// Compute on the CPU only, whatever devices the preferences name
    /// (as `cpuonly.dat` in the data folder does).
    #[arg(long)]
    pub cpu_only: bool,
    /// Changes a `BuildDefinition` member before building, e.g.
    /// `--set Type=Hybrid --set Subdivision=Faster --set
    /// BucketResolution=1024` (numbers and true/false as such, anything
    /// else as text).
    #[arg(long = "set", value_name = "MEMBER=VALUE")]
    pub settings: Vec<String>,
    /// PNG 8-bit's dither cache coarseness (ImageMagick's `CacheShift`):
    /// 2 as in Windows builds of ImageMagick (the default), 3 as in
    /// Apple builds.
    #[arg(long, value_parser = clap::value_parser!(u32).range(2..=7))]
    pub dither_cache_shift: Option<u32>,
    /// Verbose logging: the run's log file (`Logs` in the data folder)
    /// keeps the debugging messages too, and is named `…--VERBOSE.txt`.
    #[arg(long)]
    pub verbose: bool,
    /// Safe Mode: builds with the default preferences instead of the
    /// saved ones, computing on the default device.
    #[arg(long)]
    pub safemode: bool,
    /// Waits for Enter before exiting unless the build folder opens (for
    /// a build in a window of its own, which closes when it exits).
    #[arg(long)]
    pub pause: bool,
}

/// `nerthus-build build`'s arguments.
#[derive(Args, Clone, Debug)]
pub struct BuildArgs {
    pub file: PathBuf,
    #[command(flatten)]
    pub options: BuildOptions,
}

/// Where a build's output goes: this terminal, and for a build handed to
/// the build host, also the run that handed it over. That run hears one
/// line per line of output, `:` and the text for standard output, `!`
/// and the text for errors, then `/exit` and the exit code.
pub struct Console {
    silent: bool,
    client: Option<Client>,
}

/// The run a build was handed over by.
struct Client {
    stream: TcpStream,
    /// Standard output and errors since their last line end.
    output: String,
    errors: String,
}

impl Client {
    /// Each whole line of `text` sent with `kind` in front, the rest kept
    /// for later; false when the run has gone.
    fn send(&mut self, kind: char, text: &str) -> bool {
        let partial = if kind == '!' { &mut self.errors } else { &mut self.output };
        partial.push_str(text);
        let Some(end) = partial.rfind('\n') else {
            return true;
        };
        let complete: String = partial.drain(..=end).collect();
        let mut lines = String::new();
        for line in complete.lines() {
            lines.push(kind);
            lines.push_str(line);
            lines.push('\n');
        }
        self.stream.write_all(lines.as_bytes()).is_ok()
    }
}

impl Console {
    /// Output to this terminal only.
    pub fn terminal(silent: bool) -> Self {
        Self { silent, client: None }
    }

    /// Output to this terminal and to the run that sent the build.
    pub fn handed_over(stream: TcpStream) -> Self {
        let client = Client { stream, output: String::new(), errors: String::new() };
        Self { silent: false, client: Some(client) }
    }

    pub fn set_silent(&mut self, silent: bool) {
        self.silent = silent;
    }

    /// A line printed even in a silent build (what goes wrong while
    /// building).
    pub fn line(&mut self, text: &str) {
        println!("{text}");
        self.send(':', &format!("{text}\n"));
    }

    /// A line printed unless the build is silent.
    pub fn say(&mut self, text: &str) {
        if !self.silent {
            self.line(text);
        }
    }

    /// Text as it is (a build's console text), unless the build is silent.
    pub fn text(&mut self, text: &str) {
        if self.silent {
            return;
        }
        print!("{text}");
        let _ = std::io::stdout().flush();
        self.send(':', text);
    }

    /// Why a build failed, on the error stream.
    pub fn problem(&mut self, text: &str) {
        eprintln!("{text}");
        self.send('!', &format!("{text}\n"));
    }

    /// The run that handed the build over hears the rest of the output
    /// and the build's exit code, which ends its wait.
    pub fn finish(&mut self, code: u8) {
        let Some(client) = self.client.as_mut() else {
            return;
        };
        let mut ending = String::new();
        let output = std::mem::take(&mut client.output);
        if !output.is_empty() {
            ending.push_str(&format!(":{output}\n"));
        }
        let errors = std::mem::take(&mut client.errors);
        if !errors.is_empty() {
            ending.push_str(&format!("!{errors}\n"));
        }
        ending.push_str(&format!("/exit {code}\n"));
        if client.stream.write_all(ending.as_bytes()).is_err() {
            self.client = None;
        }
    }

    /// Output for the run that handed the build over; it may have gone,
    /// and then the build carries on without it.
    fn send(&mut self, kind: char, text: &str) {
        let gone = self.client.as_mut().is_some_and(|client| !client.send(kind, text));
        if gone {
            self.client = None;
        }
    }
}

/// What a run's builds share (every build of the build host): the
/// preferences they build with and the nodes.
pub struct Environment {
    pub preferences: ExportPreferences,
    pub registry: Arc<Registry>,
}

/// Loads what builds need: Safe Mode's announcement, or the user's saved
/// preferences (the defaults when they cannot be read); then the compute
/// devices.
pub fn environment(options: &BuildOptions, console: &mut Console) -> Environment {
    let mut values = PreferenceValues::new();
    if options.safemode {
        console.say("Entering Safe Mode!");
        log::warn!("SAFE MODE: User initiated Safe Mode.");
    } else if let Some(folder) = nerthus_engine::paths::data_folder() {
        match preferences::load(&folder.join(nerthus_engine::paths::PREFERENCES_FILE)) {
            Ok(saved) => values = saved,
            Err(problem) => {
                log::error!("Failed to load preferences.");
                log::debug!("{problem}");
            }
        }
    }
    logging::log_details();
    load_devices(options.cpu_only, options.safemode, options.verbose, &values, console);
    let preferences = ExportPreferences::from_preferences(&values);
    let registry = Arc::new(nerthus_nodes::registry());
    Environment { preferences, registry }
}

/// The devices builds compute on: none but the CPU in CPU-only mode
/// (`--cpu-only`, or `cpuonly.dat` in the data folder), only the default
/// one in Safe Mode or when the preferences name none, else the ones they
/// name (Options › Compute saves them). `verbose` also logs every device
/// there is.
pub fn load_devices(cpu_only_asked: bool, safe_mode: bool, verbose: bool, values: &PreferenceValues, console: &mut Console) {
    let folder = nerthus_engine::paths::data_folder();
    let marked = folder.as_ref().is_some_and(|folder| folder.join(nerthus_compute::CPU_ONLY_FILE).exists());
    if cpu_only_asked || marked {
        nerthus_compute::set_cpu_only(true);
        log::info!("CPU Only mode");
        console.say("CPU Only mode");
        return;
    }
    let backends = if safe_mode { Vec::new() } else { preferences::compute_backends(values) };
    if backends.is_empty() {
        log::info!("Loading only default device.");
        console.say("Loading only default device.");
    } else {
        log::info!("Loading user preferred devices.");
        console.say("Loading user preferred devices.");
        nerthus_compute::load_backends(&backends);
    }
    if verbose {
        let (mut available, _) = nerthus_compute::available_devices();
        available.sort_by(|left, right| {
            let left_key = (&left.device_name, &left.backend_name);
            let right_key = (&right.device_name, &right.backend_name);
            left_key.cmp(&right_key)
        });
        log::debug!("Available Devices");
        for device in available {
            log::debug!("  {} ({})", device.device_name, device.backend_name);
        }
    }
    log::debug!("Active Devices");
    for device in nerthus_compute::active_devices() {
        log::debug!("  {} ({})", device.device_name, device.backend_name);
        console.say(&format!("    Activated {} ({})...", device.device_name, device.backend_name));
    }
}

/// How a build that ran ended.
pub struct BuildOutcome {
    /// The build folder was opened.
    pub opened_folder: bool,
    /// Nodes that failed to build or export.
    pub failures: usize,
}

/// What the profile question offers besides the profiles: the saved build
/// settings rather than a profile.
const NO_PROFILE: &str = "None / Last Saved Settings";

/// The profile question (`--interactive`, more than one profile, none
/// given): which profile to build with, or none.
fn ask_for_profile(names: &[String]) -> Result<Option<String>> {
    println!();
    println!("Select Build Profile");
    println!("  0. {NO_PROFILE}");
    for (index, name) in names.iter().enumerate() {
        println!("  {}. {name}", index + 1);
    }
    loop {
        print!("> ");
        std::io::stdout().flush()?;
        let mut answer = String::new();
        if std::io::stdin().lock().read_line(&mut answer)? == 0 {
            return Ok(None);
        }
        match answer.trim().parse::<usize>() {
            Ok(0) => return Ok(None),
            Ok(number) if number <= names.len() => return Ok(Some(names[number - 1].clone())),
            _ => println!("Choose a number from 0 to {}.", names.len()),
        }
    }
}

/// The folder question: whether to open the build folder.
fn ask_to_open(folder: &Path) -> Result<bool> {
    println!("{}", folder.display());
    print!("Open output folder? [y/n] (y): ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    let answer = answer.trim().to_ascii_lowercase();
    Ok(answer.is_empty() || answer == "y" || answer == "yes")
}

/// `--pause`'s wait at the end of a run.
fn wait_for_enter() {
    print!("Press Enter to exit...");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    let _ = std::io::stdin().lock().read_line(&mut answer);
}

/// Opens a folder in the system's file browser.
fn open_folder(folder: &Path, console: &mut Console) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    if let Err(error) = std::process::Command::new(program).arg(folder).spawn() {
        console.line(&format!("Cannot open {}: {error}", folder.display()));
    }
}

/// Where the variables come from: `-v` pairs first, else `--va` values,
/// else a `--vars` file.
fn variable_assignment(options: &BuildOptions) -> Result<Option<VariableAssignment>> {
    if !options.variables.is_empty() {
        let mut pairs = Vec::new();
        for argument in &options.variables {
            let pair = swarm::parse_pair(argument).map_err(anyhow::Error::msg)?;
            if pairs.iter().any(|(name, _): &(String, String)| *name == pair.0) {
                bail!("The variable {} is given more than once.", pair.0);
            }
            pairs.push(pair);
        }
        return Ok(Some(VariableAssignment::Pairs(pairs)));
    }
    if !options.values.is_empty() {
        return Ok(Some(VariableAssignment::Values(options.values.clone())));
    }
    Ok(options.vars.clone().map(VariableAssignment::File))
}

/// `MEMBER=VALUE` applied to the `BuildDefinition` as saved: numbers and
/// booleans as JSON, anything else as a string.
fn apply_settings(saved: Option<&serde_json::Value>, settings: &[String]) -> Result<serde_json::Value> {
    let mut definition = saved.cloned().unwrap_or_else(|| serde_json::json!({}));
    let Some(members) = definition.as_object_mut() else {
        bail!("the project's BuildDefinition is not an object");
    };
    for setting in settings {
        let Some((member, value)) = setting.split_once('=') else {
            bail!("--set takes MEMBER=VALUE, not {setting}");
        };
        let parsed = serde_json::from_str::<serde_json::Value>(value).ok().filter(|parsed| parsed.is_number() || parsed.is_boolean());
        let value = parsed.unwrap_or_else(|| serde_json::Value::String(value.to_string()));
        members.insert(member.to_string(), value);
    }
    Ok(definition)
}

/// A build's engine: the registry, the asset's variables and the
/// project's macros.
pub fn project_engine(project: &Project, macros: Option<&Path>, registry: &Arc<Registry>, console: &mut Console) -> Engine {
    let asset = &project.assets[0];
    let mut engine = Engine::new(registry.clone());
    engine.automation = asset.automation.clone();
    if let Some(directory) = macros {
        let (library, problems) = load_project_macros(&project.macro_ids(), directory);
        for (id, problem) in problems {
            console.line(&format!("macro {id} error: {problem}"));
        }
        engine.macros = library;
    }
    engine
}

/// `Project.Id`.
pub fn project_id(project: &Project) -> String {
    project.extra.get("Id").and_then(serde_json::Value::as_str).unwrap_or_default().to_string()
}

/// The order of a command-line build: the project opened, the variables
/// assigned, the profile and region set up, the resolution and build
/// folder overridden, then the mutation; then the build, its report and
/// the build folder opened when the settings say so or the user wants it.
/// `questions` allows `--interactive`'s questions (never in the build
/// host).
pub fn run_build(file: &Path, options: &BuildOptions, environment: &Environment, console: &mut Console, questions: bool) -> Result<BuildOutcome> {
    let silent = options.silent;
    // Questions need someone at a terminal (Spectre.Console's
    // `Interactive`), and never in a silent build.
    let interactive = questions && options.interactive && !silent && std::io::stdin().is_terminal();
    log::info!("Opening {}", file.display());
    let text = fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let mut project = Project::parse(&text).with_context(|| format!("cannot open {}", file.display()))?;

    let mut engine = project_engine(&project, options.macros.as_deref(), &environment.registry, console);
    if let Some(assignment) = variable_assignment(options)? {
        let mut print = |line: String| console.say(&line);
        swarm::assign_variables(&mut engine.automation, &assignment, &mut print).map_err(anyhow::Error::msg)?;
    }
    if options.community {
        engine.automation.bindings.clear();
        engine.automation.expressions.clear();
    }

    let asset = &project.assets[0];
    let mut profile = options.profile.clone();
    let profile_names = swarm::profile_names(asset);
    if profile.is_none() && interactive && profile_names.len() > 1 {
        profile = ask_for_profile(&profile_names)?;
    }
    let chosen_definition = match &profile {
        Some(name) => match swarm::profile_json(asset, name) {
            Some(definition) => Some(definition),
            None => bail!("The project has no build profile named {name}."),
        },
        None => asset.extra.get("BuildDefinition"),
    };
    let saved_definition = apply_settings(chosen_definition, &options.settings)?;
    let mut definition = BuildDefinition::from_json(Some(&saved_definition));
    let built_region = build::set_up_region(&asset.terrain, &mut definition, options.region.as_deref());
    match (&built_region, &options.region) {
        (Some(found), _) => console.say(&format!("Region build of {} at {}", found.name, definition.resolution)),
        (None, Some(name)) => console.say(&format!("The terrain has no region named {name}.")),
        (None, None) => {}
    }
    if let Some(resolution) = options.resolution {
        definition.resolution = resolution;
    }
    if let Some(output) = &options.output {
        fs::create_dir_all(output)?;
        let absolute = output.canonicalize()?;
        definition.destination = absolute.to_string_lossy().into_owned();
    }
    // The mutation the build is.
    if options.seed > 0 {
        nerthus_engine::mutation::mutate_terrain(project.terrain_mut(), options.seed);
        engine.mutation = options.seed;
        console.say(&format!("Mutation {}", options.seed));
    }
    let asset = &project.assets[0];

    let mut preferences = environment.preferences.clone();
    if let Some(artist) = &options.artist {
        preferences.write.artist = artist.clone();
    }
    if let Some(shift) = options.dither_cache_shift {
        preferences.write.dither_cache_shift = shift;
    }
    let project_path = file.canonicalize()?;
    let bakes = preferences.bake_store(&project_path, &project_id(&project));
    let names: HashMap<i32, String> = asset.terrain.nodes.values().map(|node| (node.id, node.name.clone())).collect();
    let name = |node: i32| names.get(&node).cloned().unwrap_or_default();
    let cancel = AtomicBool::new(false);
    let mut on_event = |event: BuildEvent| match event {
        BuildEvent::Started { folder, nodes } => {
            logging::set_build_folder(Some(&folder));
            log::info!("Building {nodes} nodes into {}", folder.display());
            console.say(&format!("building {nodes} nodes into {}", folder.display()));
        }
        BuildEvent::BucketStarted { index, count } => console.say(&format!("bucket {} of {count}", index + 1)),
        BuildEvent::Stage(stage) => {
            log::info!("{stage}");
            console.say(&stage);
        }
        BuildEvent::NodeStarted { node } => log::info!("[{node}] {} - Build Started", name(node)),
        BuildEvent::NodeFinished { node, milliseconds, error } => match error {
            Some(error) => {
                log::info!("[{node}] {} - Build Failed", name(node));
                log::warn!("{node} - {} failed: {error}", name(node));
                console.line(&format!("node {node} failed after {milliseconds} ms: {error}"));
            }
            None => {
                log::info!("[{node}] {} - Build Finished", name(node));
                console.say(&format!("node {node} built in {milliseconds} ms"));
            }
        },
        BuildEvent::FileWritten { node, path } => {
            log::info!("{} - Export Finished - {}", name(node), path.display());
            console.say(&format!("wrote {}", path.display()));
        }
        BuildEvent::FileFailed { node, path, error } => {
            log::error!("{node} could not write {}: {error}", path.display());
            console.line(&format!("node {node} could not write {}: {error}", path.display()));
        }
        BuildEvent::Exporting { .. } => {}
        BuildEvent::Printed(text) => console.text(&text),
    };
    let run_options = RunOptions { silent, profile, ignore_cache: options.ignorecache };
    let result = build::run_with(&asset.terrain, &mut engine, &project_path, &definition, &preferences, Some(&bakes), &run_options, &cancel, &mut on_event);
    logging::set_build_folder(None);
    let report = match result {
        Ok(report) => report,
        Err(refusal) => bail!("{refusal}"),
    };
    if let Some(summary) = &report.summary {
        console.text(summary);
    }
    console.say(&format!("{} files in {}", report.files.len(), report.folder.display()));
    // The folder opened when the build settings say so,
    // or when asked.
    let mut opened_folder = false;
    if !silent {
        let open = definition.open_folder || (interactive && ask_to_open(&report.folder)?);
        if open {
            open_folder(&report.folder, console);
            opened_folder = true;
        }
    }
    Ok(BuildOutcome { opened_folder, failures: report.failures.len() })
}

/// A build's exit code, with what went wrong said: 0 when every node
/// built and exported, else 1.
pub fn exit_code(outcome: &Result<BuildOutcome>, console: &mut Console) -> u8 {
    match outcome {
        Ok(outcome) if outcome.failures == 0 => 0,
        Ok(outcome) => {
            let message = format!("{} nodes failed", outcome.failures);
            log::error!("Build failed, {message}");
            console.problem(&format!("Error: {message}"));
            1
        }
        Err(error) => {
            log::error!("Build failed, {error:#}");
            console.problem(&format!("Error: {error:?}"));
            1
        }
    }
}

/// `nerthus-build build`: the log started, the preferences and devices
/// loaded, the project built; then, with `--pause`, a wait for Enter
/// unless the build folder opened.
pub fn command(arguments: BuildArgs) -> ExitCode {
    let options = &arguments.options;
    logging::start(LogKind::Build, options.verbose);
    let mut console = Console::terminal(options.silent);
    let environment = environment(options, &mut console);
    let outcome = run_build(&arguments.file, options, &environment, &mut console, true);
    let code = exit_code(&outcome, &mut console);
    let opened = outcome.as_ref().is_ok_and(|outcome| outcome.opened_folder);
    if options.pause && !options.silent && !opened {
        wait_for_enter();
    }
    ExitCode::from(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    /// A console whose client end is a socket pair; returns what the
    /// client heard once the console finishes with `code`.
    fn heard(write: impl FnOnce(&mut Console), code: u8) -> Vec<String> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let client = TcpStream::connect(address).unwrap();
        let (server, _) = listener.accept().unwrap();
        let mut console = Console::handed_over(server);
        write(&mut console);
        console.finish(code);
        drop(console);
        BufReader::new(client).lines().map(Result::unwrap).collect()
    }

    #[test]
    fn handed_over_output_goes_line_by_line() {
        let lines = heard(
            |console| {
                console.say("Loading only default device.");
                console.text("Building");
                console.text(" 3 nodes\nhalf");
                console.problem("Error: no");
            },
            1,
        );
        assert_eq!(lines, [":Loading only default device.", ":Building 3 nodes", "!Error: no", ":half", "/exit 1"]);
    }

    #[test]
    fn silent_builds_hand_over_only_what_goes_wrong() {
        let lines = heard(
            |console| {
                console.set_silent(true);
                console.say("wrote a.png");
                console.text("report");
                console.line("node 3 failed after 2 ms: bad");
            },
            0,
        );
        assert_eq!(lines, [":node 3 failed after 2 ms: bad", "/exit 0"]);
    }

    #[test]
    fn settings_set_members_by_type() {
        let saved = serde_json::json!({"Resolution": 1024});
        let settings = ["Resolution=2048".to_string(), "Type=Hybrid".to_string(), "FlipY=true".to_string()];
        let applied = apply_settings(Some(&saved), &settings).unwrap();
        assert_eq!(applied, serde_json::json!({"Resolution": 2048, "Type": "Hybrid", "FlipY": true}));
        assert!(apply_settings(None, &["Resolution".to_string()]).is_err());
    }
}
