//! `nerthus-build`: Nerthus builds from the command line.

mod build;
mod host;
mod logging;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use nerthus_engine::{Map, Project, Resolution};
use nerthus_export::bake;
use nerthus_export::build::BuildDefinition;
use nerthus_export::{clamp_for_export, write_map, FileFormat, WriteOptions};

use crate::build::{BuildArgs, BuildOptions, Console};
use crate::host::HostArgs;

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
    /// the project's build settings say. It builds with the preferences
    /// saved in the data folder (unless `--safemode`) and logs to
    /// `Logs/<time>-BUILD.txt` there. Unless `--silent`, it leaves its
    /// report (`report.txt`, `report.json`) in the build folder.
    Build(BuildArgs),
    /// Builds in the build host, which stays loaded between builds: the
    /// first run starts it (it builds the project given, if any, and
    /// waits), later runs hand their project and options to it and print
    /// its build's output. `--exit` closes it. `--verbose`, `--safemode`
    /// and `--cpu-only` count when the host starts; it asks no questions
    /// and never pauses.
    Host(HostArgs),
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

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Build(arguments) => return build::command(arguments),
        Command::Host(arguments) => {
            // What the run hands to a running host: its words after `host`.
            let words = std::env::args().skip(1).skip_while(|word| word != "host").skip(1).collect();
            return host::command(arguments, words);
        }
        Command::Bake { file, nodes, resolution, save, macros, cpu_only } => bake(&file, &nodes, resolution, save, macros, cpu_only),
        Command::Roundtrip { file, output } => roundtrip(&file, &output),
        Command::Write { format, input, size, output, artist, dither_cache_shift, color_space } => {
            let options = WriteFlags { artist, dither_cache_shift, color_space };
            write(&format, &input, size, &output, options)
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error:?}");
            ExitCode::FAILURE
        }
    }
}

fn roundtrip(file: &Path, output: &Path) -> Result<()> {
    let text = fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let project = Project::parse(&text).with_context(|| format!("cannot open {}", file.display()))?;
    let saved = nerthus_engine::to_terrain_json(&project);
    fs::write(output, saved).with_context(|| format!("cannot write {}", output.display()))?;
    println!("wrote {}", output.display());
    Ok(())
}

/// Bakes into the bake folder builds read: where the preferences keep
/// baked maps (beside the project unless they say otherwise).
fn bake(file: &Path, nodes: &[i32], resolution: Option<usize>, save: Option<PathBuf>, macros: Option<PathBuf>, cpu_only: bool) -> Result<()> {
    let options = BuildOptions { cpu_only, ..BuildOptions::default() };
    let mut console = Console::terminal(false);
    let environment = build::environment(&options, &mut console);
    let text = fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let mut project = Project::parse(&text).with_context(|| format!("cannot open {}", file.display()))?;
    let definition = BuildDefinition::from_json(project.assets[0].extra.get("BuildDefinition"));
    let size = resolution.unwrap_or(definition.bake_resolution);
    let project_path = file.canonicalize()?;
    let store = environment.preferences.bake_store(&project_path, &build::project_id(&project));
    let targets = if nodes.is_empty() { bake::required_nodes(project.terrain()) } else { nodes.to_vec() };
    if targets.is_empty() {
        println!("nothing to bake");
        return Ok(());
    }

    let mut engine = build::project_engine(&project, macros.as_deref(), &environment.registry, &mut console);
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
