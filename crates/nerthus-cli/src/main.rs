//! `nerthus-build`: Nerthus builds from the command line.

use std::fs;
use std::io::{BufRead, IsTerminal, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use nerthus_engine::macros::load_project_macros;
use nerthus_engine::{Engine, Map, Project, Resolution};
use nerthus_export::bake;
use nerthus_export::build::{self, BuildDefinition, BuildEvent, ExportPreferences, RunOptions};
use nerthus_export::swarm::{self, VariableAssignment};
use nerthus_export::{clamp_for_export, write_map, FileFormat, WriteOptions};

#[derive(Parser)]
#[command(name = "nerthus-build", version, about = "Build and export Nerthus terrain projects from the command line")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Builds a project and exports its marked nodes: a Standard build, a
    /// Hybrid or Tiled one a bucket at a time from the project's bakes
    /// (see `bake`), or a Region build of one of the terrain's regions, as
    /// the project's build settings say. Unless `--silent`, it leaves its
    /// report (`report.txt`, `report.json`) in the build folder.
    Build {
        file: PathBuf,
        /// Build resolution (defaults to the project's; a Region build
        /// builds at the region's size).
        #[arg(long)]
        resolution: Option<usize>,
        /// Builds this region of the terrain instead of the one the build
        /// settings name.
        #[arg(long, short = 'r')]
        region: Option<String>,
        /// Build folder (replaces the project's destination template).
        #[arg(long, alias = "buildpath")]
        output: Option<PathBuf>,
        /// Builds with this build profile (Build Settings › Profiles)
        /// instead of the saved build settings.
        #[arg(long, short = 'p')]
        profile: Option<String>,
        /// Sets a variable for this build, e.g. `-v Height=0.63`; repeat
        /// for more.
        #[arg(short = 'v', long = "variable", value_name = "NAME=VALUE")]
        variables: Vec<String>,
        /// Every variable's value, comma separated, in the order the
        /// project lists its variables.
        #[arg(long = "va", value_name = "VALUES", value_delimiter = ',')]
        values: Vec<String>,
        /// A `.json` object or a `.txt` file of `name=value` lines setting
        /// variables.
        #[arg(long)]
        vars: Option<PathBuf>,
        /// Builds a mutation of the terrain: every node's seed moved on by
        /// this number, also `[Mutation]` in the build folder's name.
        #[arg(long, default_value_t = 0)]
        seed: i32,
        /// Builds the baked nodes instead of reading their baked maps
        /// (Standard builds).
        #[arg(long, alias = "ignore-cache")]
        ignorecache: bool,
        /// Prints only what goes wrong, leaves no report in the build
        /// folder and never opens it.
        #[arg(long)]
        silent: bool,
        /// Asks which build profile to use when the project has more than
        /// one and none is given, and whether to open the build folder
        /// once built (needs a terminal).
        #[arg(long)]
        interactive: bool,
        /// Folder holding the project's macros.
        #[arg(long)]
        macros: Option<PathBuf>,
        /// Builds without applying bindings and expressions (a Community
        /// build).
        #[arg(long)]
        community: bool,
        /// The TIFF 32-bit Artist tag.
        #[arg(long)]
        artist: Option<String>,
        /// Compute on the CPU only, whatever devices the preferences name
        /// (as `cpuonly.dat` in the data folder does).
        #[arg(long)]
        cpu_only: bool,
        /// Changes a `BuildDefinition` member before building, e.g.
        /// `--set Type=Hybrid --set Subdivision=Faster --set
        /// BucketResolution=1024` (numbers and true/false as such, anything
        /// else as text).
        #[arg(long = "set", value_name = "MEMBER=VALUE")]
        settings: Vec<String>,
        /// PNG 8-bit's dither cache coarseness (ImageMagick's `CacheShift`):
        /// 2 as in Windows builds of ImageMagick (the default), 3 as in
        /// Apple builds.
        #[arg(long, value_parser = clap::value_parser!(u32).range(2..=7))]
        dither_cache_shift: Option<u32>,
    },
    /// Bakes nodes as Graph › Baking does: each node and everything
    /// upstream of it built at the bake resolution, the maps kept in the
    /// project's `Cache/Bakes` folder and the nodes marked `Baked` in the
    /// saved project. Without `--node`, bakes the nodes that need baking
    /// (Bake Required Nodes).
    Bake {
        file: PathBuf,
        /// A node to bake (by id); repeat for more.
        #[arg(long = "node")]
        nodes: Vec<i32>,
        /// Bake resolution (defaults to the project's `BakeResolution`).
        #[arg(long)]
        resolution: Option<usize>,
        /// Where to save the project (defaults to the project itself).
        #[arg(long)]
        save: Option<PathBuf>,
        /// Folder holding the project's macros.
        #[arg(long)]
        macros: Option<PathBuf>,
        /// Compute on the CPU only.
        #[arg(long)]
        cpu_only: bool,
    },
    /// Opens a project as File > Open does and saves it again (a check of
    /// the project writer).
    Roundtrip { file: PathBuf, output: PathBuf },
    /// Writes a map of raw floats (channels one after another) in one of
    /// the export formats, prepared as a build prepares it (clamped).
    Write {
        /// A `FileFormats` member name: EXR, PNG16, TIFF32, …
        format: String,
        /// Little-endian floats, `size × size` per channel.
        input: PathBuf,
        size: usize,
        output: PathBuf,
        /// The TIFF 32-bit Artist tag.
        #[arg(long)]
        artist: Option<String>,
        /// PNG 8-bit's dither cache coarseness (ImageMagick's `CacheShift`):
        /// 2 as in Windows builds of ImageMagick (the default), 3 as in
        /// Apple builds.
        #[arg(long, value_parser = clap::value_parser!(u32).range(2..=7))]
        dither_cache_shift: Option<u32>,
        /// The build ColorSpace a colour map is written in (a Colorspace
        /// member such as sRGB, Lab or CMYK; sRGB by default).
        #[arg(long)]
        color_space: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Build {
            file,
            resolution,
            region,
            output,
            profile,
            variables,
            values,
            vars,
            seed,
            ignorecache,
            silent,
            interactive,
            macros,
            community,
            artist,
            cpu_only,
            settings,
            dither_cache_shift,
        } => {
            load_devices(cpu_only, silent);
            let swarm = SwarmOptions { profile, variables, values, vars, seed, ignore_cache: ignorecache, silent, interactive };
            let options = BuildOptions { resolution, region, output, macros, community, artist, settings, dither_cache_shift, swarm };
            build(&file, options)
        }
        Command::Bake { file, nodes, resolution, save, macros, cpu_only } => {
            load_devices(cpu_only, false);
            bake(&file, &nodes, resolution, save, macros)
        }
        Command::Roundtrip { file, output } => roundtrip(&file, &output),
        Command::Write { format, input, size, output, artist, dither_cache_shift, color_space } => {
            let options = WriteFlags { artist, dither_cache_shift, color_space };
            write(&format, &input, size, &output, options)
        }
    }
}

/// The devices a build computes on, as the build host (`SwarmHost`) loads
/// them: none but the CPU in CPU-only mode or when the preferences
/// name none, else the ones they name (Options › Compute saves them).
/// `quiet` keeps it from saying so.
fn load_devices(cpu_only_asked: bool, quiet: bool) {
    let say = |line: String| {
        if !quiet {
            println!("{line}");
        }
    };
    let folder = nerthus_engine::paths::data_folder();
    let marked = folder.as_ref().is_some_and(|folder| folder.join(nerthus_compute::CPU_ONLY_FILE).exists());
    if cpu_only_asked || marked {
        nerthus_compute::set_cpu_only(true);
        say("CPU Only mode".to_string());
        return;
    }
    let backends = folder.map(|folder| saved_backends(&folder.join(nerthus_engine::paths::PREFERENCES_FILE))).unwrap_or_default();
    if backends.is_empty() {
        say("Loading only default device.".to_string());
        return;
    }
    say("Loading user preferred devices.".to_string());
    nerthus_compute::load_backends(&backends);
    for device in nerthus_compute::active_devices() {
        say(format!("    Activated {} ({})...", device.device_name, device.backend_name));
    }
}

/// `Compute.Backends` in the preferences file (a Newtonsoft list, with or
/// without `$values`); none when the file is missing or unreadable.
fn saved_backends(path: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(preferences) = serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}')) else {
        return Vec::new();
    };
    let backends = &preferences["Compute"]["Backends"];
    let list = backends.get("$values").unwrap_or(backends);
    let ids = list.as_array().map(|ids| ids.iter().filter_map(|id| id.as_str()).map(str::to_string).collect());
    ids.unwrap_or_default()
}

fn roundtrip(file: &Path, output: &Path) -> Result<()> {
    let text = fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let project = Project::parse(&text).with_context(|| format!("cannot open {}", file.display()))?;
    let saved = nerthus_engine::to_terrain_json(&project);
    fs::write(output, saved).with_context(|| format!("cannot write {}", output.display()))?;
    println!("wrote {}", output.display());
    Ok(())
}

/// What `build` takes from the command line.
struct BuildOptions {
    resolution: Option<usize>,
    region: Option<String>,
    output: Option<PathBuf>,
    macros: Option<PathBuf>,
    community: bool,
    artist: Option<String>,
    settings: Vec<String>,
    dither_cache_shift: Option<u32>,
    swarm: SwarmOptions,
}

/// The options of a command-line build beyond the build settings.
struct SwarmOptions {
    profile: Option<String>,
    variables: Vec<String>,
    values: Vec<String>,
    vars: Option<PathBuf>,
    seed: i32,
    ignore_cache: bool,
    silent: bool,
    interactive: bool,
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

/// Opens a folder in the system's file browser.
fn open_folder(folder: &Path) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    if let Err(error) = std::process::Command::new(program).arg(folder).spawn() {
        println!("Cannot open {}: {error}", folder.display());
    }
}

/// Where the variables come from: `-v` pairs first, else `--va` values,
/// else a `--vars` file.
fn variable_assignment(options: &SwarmOptions) -> Result<Option<VariableAssignment>> {
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
fn project_engine(project: &Project, macros: Option<PathBuf>) -> Engine {
    let asset = &project.assets[0];
    let registry = Arc::new(nerthus_nodes::registry());
    let mut engine = Engine::new(registry);
    engine.automation = asset.automation.clone();
    if let Some(directory) = macros {
        let (library, problems) = load_project_macros(&project.macro_ids(), &directory);
        for (id, problem) in problems {
            println!("macro {id} error: {problem}");
        }
        engine.macros = library;
    }
    engine
}

/// `Project.Id`.
fn project_id(project: &Project) -> String {
    project.extra.get("Id").and_then(serde_json::Value::as_str).unwrap_or_default().to_string()
}

/// The order of a command-line build: the project opened, the
/// variables assigned, the profile and region set up, the resolution and
/// build folder overridden, then the mutation; then the build, its report
/// and the build folder opened if asked.
fn build(file: &Path, options: BuildOptions) -> Result<()> {
    let BuildOptions { resolution, region, output, macros, community, artist, settings, dither_cache_shift, swarm: swarm_options } = options;
    let silent = swarm_options.silent;
    // Questions need someone at a terminal (Spectre.Console's
    // `Interactive`), and never in a silent build.
    let interactive = swarm_options.interactive && !silent && std::io::stdin().is_terminal();
    let say = |line: &str| {
        if !silent {
            println!("{line}");
        }
    };
    let text = fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let mut project = Project::parse(&text).with_context(|| format!("cannot open {}", file.display()))?;

    let mut engine = project_engine(&project, macros);
    if let Some(assignment) = variable_assignment(&swarm_options)? {
        let mut print = |line: String| say(&line);
        swarm::assign_variables(&mut engine.automation, &assignment, &mut print).map_err(anyhow::Error::msg)?;
    }
    if community {
        engine.automation.bindings.clear();
        engine.automation.expressions.clear();
    }

    let asset = &project.assets[0];
    let mut profile = swarm_options.profile.clone();
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
    let saved_definition = apply_settings(chosen_definition, &settings)?;
    let mut definition = BuildDefinition::from_json(Some(&saved_definition));
    let built_region = build::set_up_region(&asset.terrain, &mut definition, region.as_deref());
    match (&built_region, &region) {
        (Some(found), _) => say(&format!("Region build of {} at {}", found.name, definition.resolution)),
        (None, Some(name)) => say(&format!("The terrain has no region named {name}.")),
        (None, None) => {}
    }
    if let Some(resolution) = resolution {
        definition.resolution = resolution;
    }
    if let Some(output) = output {
        fs::create_dir_all(&output)?;
        let absolute = output.canonicalize()?;
        definition.destination = absolute.to_string_lossy().into_owned();
    }
    // The mutation the build is.
    if swarm_options.seed > 0 {
        nerthus_engine::mutation::mutate_terrain(project.terrain_mut(), swarm_options.seed);
        engine.mutation = swarm_options.seed;
        say(&format!("Mutation {}", swarm_options.seed));
    }
    let asset = &project.assets[0];

    let mut preferences = ExportPreferences::default();
    if let Some(artist) = artist {
        preferences.write.artist = artist;
    }
    if let Some(shift) = dither_cache_shift {
        preferences.write.dither_cache_shift = shift;
    }
    let project_path = file.canonicalize()?;
    let bakes = preferences.bake_store(&project_path, &project_id(&project));
    let cancel = AtomicBool::new(false);
    let mut on_event = |event: BuildEvent| match event {
        BuildEvent::Started { folder, nodes } => say(&format!("building {nodes} nodes into {}", folder.display())),
        BuildEvent::BucketStarted { index, count } => say(&format!("bucket {} of {count}", index + 1)),
        BuildEvent::Stage(stage) => say(&stage),
        BuildEvent::NodeStarted { .. } => {}
        BuildEvent::NodeFinished { node, milliseconds, error } => match error {
            Some(error) => println!("node {node} failed after {milliseconds} ms: {error}"),
            None => say(&format!("node {node} built in {milliseconds} ms")),
        },
        BuildEvent::FileWritten { path, .. } => say(&format!("wrote {}", path.display())),
        BuildEvent::FileFailed { node, path, error } => println!("node {node} could not write {}: {error}", path.display()),
        BuildEvent::Printed(text) => {
            if !silent {
                print!("{text}");
            }
        }
    };
    let run_options = RunOptions { silent, profile, ignore_cache: swarm_options.ignore_cache };
    let result = build::run_with(&asset.terrain, &mut engine, &project_path, &definition, &preferences, Some(&bakes), &run_options, &cancel, &mut on_event);
    let report = match result {
        Ok(report) => report,
        Err(refusal) => bail!("{refusal}"),
    };
    if let Some(summary) = &report.summary {
        print!("{summary}");
    }
    say(&format!("{} files in {}", report.files.len(), report.folder.display()));
    // The folder opened when the build settings say so,
    // or when asked.
    if !silent {
        let open = definition.open_folder || (interactive && ask_to_open(&report.folder)?);
        if open {
            open_folder(&report.folder);
        }
    }
    if !report.failures.is_empty() {
        bail!("{} nodes failed", report.failures.len());
    }
    Ok(())
}

fn bake(file: &Path, nodes: &[i32], resolution: Option<usize>, save: Option<PathBuf>, macros: Option<PathBuf>) -> Result<()> {
    let text = fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let mut project = Project::parse(&text).with_context(|| format!("cannot open {}", file.display()))?;
    let definition = BuildDefinition::from_json(project.assets[0].extra.get("BuildDefinition"));
    let size = resolution.unwrap_or(definition.bake_resolution);
    let project_path = file.canonicalize()?;
    let preferences = ExportPreferences::default();
    let store = preferences.bake_store(&project_path, &project_id(&project));
    let targets = if nodes.is_empty() { bake::required_nodes(project.terrain()) } else { nodes.to_vec() };
    if targets.is_empty() {
        println!("nothing to bake");
        return Ok(());
    }

    let mut engine = project_engine(&project, macros);
    engine.project_path = Some(project_path.clone());
    let none_built = |_: i32| None;
    for target in targets {
        let terrain = project.terrain().clone();
        let baked = match bake::bake_node(&mut engine, &terrain, target, Resolution::square(size), &store, &none_built) {
            Ok(baked) => baked,
            Err(error) => bail!("baking node {target}: {error}"),
        };
        println!("baked node {target} ({} nodes) at {size} into {}", baked.len(), store.folder().display());
        for id in baked {
            if let Some(node) = project.terrain_mut().nodes.get_mut(&id) {
                node.set_member("StateOverride", serde_json::Value::from("Baked"));
            }
        }
    }

    let destination = save.unwrap_or_else(|| file.to_path_buf());
    let saved = nerthus_engine::to_terrain_json(&project);
    fs::write(&destination, saved).with_context(|| format!("cannot write {}", destination.display()))?;
    println!("wrote {}", destination.display());
    Ok(())
}

/// What `write` takes from the command line besides the map.
struct WriteFlags {
    artist: Option<String>,
    dither_cache_shift: Option<u32>,
    color_space: Option<String>,
}

fn write(format_name: &str, input: &Path, size: usize, output: &Path, flags: WriteFlags) -> Result<()> {
    let Some(format) = FileFormat::from_name(format_name) else {
        bail!("unknown format {format_name}");
    };
    let bytes = fs::read(input).with_context(|| format!("cannot read {}", input.display()))?;
    let (words, _) = bytes.as_chunks::<4>();
    let values: Vec<f32> = words.iter().map(|word| f32::from_le_bytes(*word)).collect();
    let plane = size * size;
    if plane == 0 || !values.len().is_multiple_of(plane) {
        bail!("{} holds {} floats, not a whole number of {size}×{size} planes", input.display(), values.len());
    }
    let channels: Vec<Vec<f32>> = values.chunks(plane).map(<[f32]>::to_vec).collect();
    let map = Map::from_channels(Resolution::square(size), channels);
    let prepared = clamp_for_export(&map);

    let mut options = WriteOptions::default();
    if let Some(artist) = flags.artist {
        options.artist = artist;
    }
    if let Some(shift) = flags.dither_cache_shift {
        options.dither_cache_shift = shift;
    }
    if let Some(color_space) = flags.color_space {
        options.color_space = color_space;
    }
    write_map(format, output, &prepared, &options).with_context(|| format!("cannot write {}", output.display()))?;
    println!("wrote {}", output.display());
    Ok(())
}
