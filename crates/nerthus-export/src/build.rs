//! A build: the pre-build checks and the build folder, then by build type:
//!
//! - Standard: the baked nodes' maps loaded, then each node marked for
//!   export, right to left in the graph, built with everything upstream of
//!   it, every node that loop builds writing its marked ports straight
//!   after (`Node.PostBuild`: single images, or tiles cut from them). With
//!   Purge unnecessary cache during build on, results no node still to
//!   build needs go as the loop moves on.
//! - Hybrid and Tiled: the terrain built a bucket at a time, at
//!   the bucket's size plus its margins, from the baked maps cut for the
//!   bucket; each bucket's marked ports kept as `.bin` files in the build
//!   folder's `.BuildArtifacts`; then the buckets' edges blended into
//!   their neighbours and the buckets put back together into single
//!   images (Hybrid) or cut into tiles (Tiled).
//!
//! - Region: a region of the terrain (`set_up_region`) built
//!   on its own at its own size, from the baked maps cut to the region.
//!
//! Once built, a Standard or Region build whose folder is a new one each
//! time can be copied to `Latest` beside it ("Maintain a static folder
//! with the latest copy"), the report is left in the build folder
//! ([`crate::report`]), and a post-build script runs from the build
//! folder.
//!
//! Save cached build ([`export_built`]) builds nothing: it writes the maps
//! the editor holds, as a Standard build would write them.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use web_time::Instant;

use chrono::{DateTime, Local};

use nerthus_engine::region::{self, Region};
use nerthus_engine::{ApplicationMode, BuildResult, Catalogue, Engine, Map, NodeData, NodeState, Resolution, StateOverride, Terrain};
use nerthus_nodes::transformer::{self, AnchorPosition, BlendMode};
use serde_json::Value;

use crate::bake::{self, BakeFit, BakeStore};
use crate::destination::{place_in_folder, resolve_destination, to_path, DestinationContext};
use crate::format::{clamp_for_export, write_map, FileFormat, WriteOptions};
use crate::mesh_exports;
use crate::report::{self, ReportHeader};
use crate::swarm;
use crate::tiles::{self, Bucket, BucketGeometry, GridTile, OrganizeFiles, Side};

/// `BuildType`. The enum has no zero member, so a definition saved
/// without one reads as 0, which Nerthus builds as Standard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BuildType {
    #[default]
    Standard,
    Hybrid,
    Tiled,
    Region,
}

impl BuildType {
    pub fn name(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Hybrid => "Hybrid",
            Self::Tiled => "Tiled",
            Self::Region => "Region",
        }
    }
}

/// `OverwriteMode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverwriteMode {
    #[default]
    Overwrite,
    Increment,
}

impl OverwriteMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Overwrite => "Overwrite",
            Self::Increment => "Increment",
        }
    }
}

/// `HybridSubdivision`: the bucket size Build Settings offers, by name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Subdivision {
    #[default]
    None,
    Faster,
    Balanced,
    Slower,
}

impl Subdivision {
    pub const ALL: [Subdivision; 4] = [Subdivision::None, Subdivision::Faster, Subdivision::Balanced, Subdivision::Slower];

    pub fn from_name(name: &str) -> Self {
        match name {
            "Faster" => Self::Faster,
            "Balanced" => Self::Balanced,
            "Slower" => Self::Slower,
            _ => Self::None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Faster => "Faster",
            Self::Balanced => "Balanced",
            Self::Slower => "Slower",
        }
    }

    /// The bucket size each choice stands for (`GenerateBuckets`' tags;
    /// `None` is the whole resolution).
    pub fn bucket_resolution(self, resolution: usize) -> usize {
        match self {
            Self::None => resolution,
            Self::Faster => 1024,
            Self::Balanced => 2048,
            Self::Slower => 4096,
        }
    }

    /// Whether Build Settings enables the choice at `resolution`
    /// (`GenerateBuckets`): `None` up to 16K, the others when the
    /// resolution is a larger multiple of their bucket size.
    pub fn available_at(self, resolution: usize) -> bool {
        match self {
            Self::None => resolution <= 16384,
            Self::Faster => resolution > 1024,
            Self::Balanced => resolution.is_multiple_of(2048) && resolution > 2048,
            Self::Slower => resolution.is_multiple_of(4096) && resolution > 4096,
        }
    }
}

/// `OutputType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputType {
    #[default]
    SingleImage,
    CroppedTiles,
}

impl OutputType {
    pub fn name(self) -> &'static str {
        match self {
            Self::SingleImage => "SingleImage",
            Self::CroppedTiles => "CroppedTiles",
        }
    }
}

/// `BuildDefinition`'s constructor values for members Nerthus cannot use
/// at Newtonsoft's zero (a zero size would divide by zero, an empty tile
/// pattern names every tile alike).
pub const DEFAULT_TILE_RESOLUTION: usize = 1024;
pub const DEFAULT_BUCKET_RESOLUTION: usize = 2048;
pub const DEFAULT_BAKE_RESOLUTION: usize = 2048;
pub const DEFAULT_TILE_PATTERN: &str = "_y%Y%_x%X%";

/// The asset's `BuildDefinition`. Newtonsoft leaves out members at their
/// type's default and fills missing ones in with it, so a missing member
/// is false, zero, empty or the enum's first value (except where noted).
#[derive(Clone, Debug, PartialEq)]
pub struct BuildDefinition {
    pub build_type: BuildType,
    /// `Portion`: the region a Region build builds (a copy of the
    /// terrain's region of that name, or one no longer in the terrain).
    pub portion: Option<Region>,
    pub destination: String,
    pub overwrite_mode: OverwriteMode,
    pub subdivision: Subdivision,
    pub resolution: usize,
    pub bake_resolution: usize,
    pub tile_resolution: usize,
    pub bucket_resolution: usize,
    pub output_type: OutputType,
    pub number_of_tiles: usize,
    pub preserve_tiled_cache: bool,
    pub edge_blending: f32,
    pub flip_y: bool,
    pub overlapping_pixels: usize,
    pub tile_pad_digits: bool,
    pub tile_zero_index: bool,
    pub tile_pattern: String,
    pub organize_files: OrganizeFiles,
    /// `MaintainPrimeFolder`: keep a copy of the latest build as `Latest`.
    pub maintain_prime_folder: bool,
    pub open_folder: bool,
    pub copy_terrain: bool,
    pub remove_primary_port_name: bool,
    pub post_build_script: String,
    pub color_space: String,
}

impl Default for BuildDefinition {
    /// `new BuildDefinition()`.
    fn default() -> Self {
        Self {
            build_type: BuildType::Standard,
            portion: None,
            destination: String::new(),
            overwrite_mode: OverwriteMode::Overwrite,
            subdivision: Subdivision::None,
            resolution: 2048,
            bake_resolution: DEFAULT_BAKE_RESOLUTION,
            tile_resolution: DEFAULT_TILE_RESOLUTION,
            bucket_resolution: DEFAULT_BUCKET_RESOLUTION,
            output_type: OutputType::SingleImage,
            number_of_tiles: 3,
            preserve_tiled_cache: false,
            edge_blending: 0.25,
            flip_y: false,
            overlapping_pixels: 0,
            tile_pad_digits: false,
            tile_zero_index: true,
            tile_pattern: DEFAULT_TILE_PATTERN.to_string(),
            organize_files: OrganizeFiles::NodeSubFolder,
            maintain_prime_folder: false,
            open_folder: false,
            copy_terrain: false,
            remove_primary_port_name: false,
            post_build_script: String::new(),
            color_space: "sRGB".to_string(),
        }
    }
}

impl BuildDefinition {
    /// The asset's `BuildDefinition` as the project holds it.
    pub fn from_json(value: Option<&Value>) -> Self {
        let Some(value) = value else {
            return Self::default();
        };
        let text = |name: &str| value.get(name).and_then(Value::as_str).unwrap_or_default().to_string();
        let flag = |name: &str| value.get(name).and_then(Value::as_bool).unwrap_or(false);
        let size = |name: &str| value.get(name).and_then(Value::as_u64).unwrap_or(0) as usize;
        let build_type = match value.get("Type").and_then(Value::as_str) {
            Some("Hybrid") => BuildType::Hybrid,
            Some("Tiled") => BuildType::Tiled,
            Some("Region") => BuildType::Region,
            _ => BuildType::Standard,
        };
        let overwrite_mode = match value.get("OverwriteMode").and_then(Value::as_str) {
            Some("Increment") => OverwriteMode::Increment,
            _ => OverwriteMode::Overwrite,
        };
        let output_type = match value.get("OutputType").and_then(Value::as_str) {
            Some("CroppedTiles") => OutputType::CroppedTiles,
            _ => OutputType::SingleImage,
        };
        let or_default = |found: usize, fallback: usize| if found == 0 { fallback } else { found };
        // A missing resolution reads as the constructor's 2048 rather than
        // 0 (and the like for the other sizes).
        let resolution = or_default(size("Resolution"), 2048);
        let tile_pattern = value.get("TilePattern").and_then(Value::as_str).filter(|pattern| !pattern.is_empty()).unwrap_or(DEFAULT_TILE_PATTERN);
        let color_space = value.get("ColorSpace").and_then(Value::as_str).unwrap_or("Undefined").to_string();
        let edge_blending = value.get("EdgeBlending").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        Self {
            build_type,
            portion: region::portion(value),
            destination: text("Destination"),
            overwrite_mode,
            subdivision: Subdivision::from_name(value.get("Subdivision").and_then(Value::as_str).unwrap_or_default()),
            resolution,
            bake_resolution: or_default(size("BakeResolution"), DEFAULT_BAKE_RESOLUTION),
            tile_resolution: or_default(size("TileResolution"), DEFAULT_TILE_RESOLUTION),
            bucket_resolution: or_default(size("BucketResolution"), DEFAULT_BUCKET_RESOLUTION),
            output_type,
            number_of_tiles: size("NumberOfTiles"),
            preserve_tiled_cache: flag("PreserveTiledCache"),
            edge_blending,
            flip_y: flag("FlipY"),
            overlapping_pixels: size("OverlappingPixels"),
            tile_pad_digits: flag("TilePadDigits"),
            tile_zero_index: flag("TileZeroIndex"),
            tile_pattern: tile_pattern.to_string(),
            organize_files: OrganizeFiles::from_name(value.get("OrganizeFiles").and_then(Value::as_str).unwrap_or_default()),
            maintain_prime_folder: flag("MaintainPrimeFolder"),
            open_folder: flag("OpenFolder"),
            copy_terrain: flag("CopyTerrain"),
            remove_primary_port_name: flag("RemovePrimaryPortName"),
            post_build_script: text("PostBuildScript"),
            color_space,
        }
    }

    /// `WorldResolution`: the tiles' extent for a Tiled build, else the
    /// resolution.
    pub fn world_resolution(&self) -> usize {
        if self.build_type == BuildType::Tiled {
            return self.number_of_tiles * self.tile_resolution;
        }
        self.resolution
    }

    /// The bucket grid's sizes.
    pub fn geometry(&self) -> BucketGeometry {
        BucketGeometry {
            resolution: self.resolution,
            bucket_resolution: self.bucket_resolution,
            edge_blending: self.edge_blending,
            tile_resolution: self.tile_resolution,
        }
    }

    /// `NumberOfTiles` as a Standard build reads it: as saved (Build
    /// Settings keeps it at `Resolution / TileResolution`), or that ratio
    /// when the saved count is missing or no longer fits the resolution.
    pub fn tiles_per_side(&self) -> usize {
        let fitted = self.resolution / self.tile_resolution;
        let saved = self.number_of_tiles;
        if saved == 0 || saved * self.tile_resolution > self.resolution { fitted.max(1) } else { saved }
    }

    /// `Tile.TileSuffix(TilePattern)` for tile column `x`, row `y` of a
    /// grid of `tiles_per_side`.
    pub fn tile_suffix(&self, x: usize, y: usize, tiles_per_side: usize) -> String {
        let pad_digits = if self.tile_pad_digits { tiles_per_side.to_string().len() } else { 1 };
        tiles::tile_suffix(x, y, &self.tile_pattern, !self.tile_zero_index, pad_digits, tiles_per_side * 2, true)
    }
}

/// Settings from outside the project (`Preferences`).
#[derive(Clone, Debug)]
pub struct ExportPreferences {
    /// `Defaults.FilenameDelimiter`.
    pub filename_delimiter: String,
    /// `Paths.Builds`.
    pub builds_root: PathBuf,
    pub write: WriteOptions,
    /// `Locations.Cache` when `Locations.CentralizeCacheBake` is on: where
    /// baked maps are kept instead of beside the project.
    pub central_bake_cache: Option<PathBuf>,
    /// `AllowValuesBeyondRange`: node results are not clamped to 0–1.
    pub allow_values_beyond_range: bool,
    /// `Cache.OffloadBuildCache` (Purge unnecessary cache during build to
    /// conserve memory): results no node still to build needs are let go
    /// as the build goes.
    pub offload_build_cache: bool,
}

impl Default for ExportPreferences {
    fn default() -> Self {
        Self {
            filename_delimiter: "_".to_string(),
            builds_root: crate::destination::default_builds_root(),
            write: WriteOptions::default(),
            central_bake_cache: None,
            allow_values_beyond_range: false,
            offload_build_cache: false,
        }
    }
}

impl ExportPreferences {
    /// Where a project's baked maps are.
    pub fn bake_store(&self, project_path: &Path, project_id: &str) -> BakeStore {
        BakeStore::new(project_path, project_id, self.central_bake_cache.as_deref())
    }
}

/// What a build reports as it goes.
#[derive(Clone, Debug)]
pub enum BuildEvent {
    /// The build folder was made; `nodes` is how many node builds the
    /// build expects (per bucket, times the buckets).
    Started { folder: PathBuf, nodes: usize },
    /// A bucket starts (`index` from 0).
    BucketStarted { index: usize, count: usize },
    /// A step after the buckets (`Status.Start`'s texts: "Blending Edges",
    /// "Saving Data", …).
    Stage(String),
    NodeStarted { node: i32 },
    NodeFinished { node: i32, milliseconds: u64, error: Option<String> },
    FileWritten { node: i32, path: PathBuf },
    FileFailed { node: i32, path: PathBuf, error: String },
    /// Save cached build ([`export_built`]): a node marked for export is
    /// about to write its maps, the `index`th (from 1) of `count`.
    Exporting { node: i32, index: usize, count: usize },
    /// Text the build prints to its console as it builds (and keeps for
    /// `report.txt`): the header once the build folder is made, then the
    /// lines it writes as it goes.
    Printed(String),
}

/// The outcome of a build.
#[derive(Clone, Debug, Default)]
pub struct BuildReport {
    pub folder: PathBuf,
    pub files: Vec<PathBuf>,
    /// Nodes that failed to build or export, with why.
    pub failures: Vec<(i32, String)>,
    pub cancelled: bool,
    /// What the build printed as it went, as the console records it for
    /// `report.txt`: the header, then the lines written while
    /// building (nodes that failed, the steps of a bucketed build).
    pub console: String,
    /// The report printed once the build is done, unless the
    /// build was silent.
    pub summary: Option<String>,
    /// The header the build printed (and the report repeats).
    pub header: ReportHeader,
    /// The report of the build, as `report.json` holds it.
    pub swarm: Option<report::BuildReport>,
    /// The nodes the report counts (`NodeCount`).
    pub(crate) node_count: usize,
    /// What the build loop gave for the last exporting node.
    pub(crate) last_result: Option<String>,
    /// Each node's last build time in milliseconds (`Node.BuildTime`).
    pub(crate) build_times: HashMap<i32, u64>,
}

/// What the command line adds to a build.
#[derive(Clone, Debug, Default)]
pub struct RunOptions {
    /// `-silent`: no report is left in the build folder.
    pub silent: bool,
    /// The build profile the definition came from (`-profile`), which the
    /// report names.
    pub profile: Option<String>,
    /// `-ignorecache`: a Standard build leaves the baked maps on disk and
    /// builds the baked nodes as it builds the others.
    pub ignore_cache: bool,
}

/// Why a build did not start.
#[derive(Clone, Debug, PartialEq)]
pub enum BuildRefusal {
    /// The pre-build checks' refusals, in their usual words.
    Check(String),
    Unsupported(String),
    Folder(String),
}

impl std::fmt::Display for BuildRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildRefusal::Check(message) | BuildRefusal::Unsupported(message) | BuildRefusal::Folder(message) => formatter.write_str(message),
        }
    }
}

fn is_export_class(node: &NodeData) -> bool {
    let catalogue = Catalogue::get();
    catalogue.node(&node.type_name).is_some_and(|spec| spec.classification.as_deref() == Some("Export"))
}

/// The node's `SaveDefinition`, if it has one.
fn save_definition(node: &NodeData) -> Option<&Value> {
    node.base.get("SaveDefinition").filter(|value| !value.is_null())
}

/// Whether a built node exports.
pub fn exports(node: &NodeData) -> bool {
    if is_export_class(node) {
        return true;
    }
    let enabled = save_definition(node).and_then(|definition| definition.get("IsEnabled")).and_then(Value::as_bool);
    enabled.unwrap_or(false)
}

/// The nodes a build exports, in the project's order.
pub fn exporting_nodes(terrain: &Terrain) -> Vec<i32> {
    terrain.nodes.values().filter(|node| exports(node)).map(|node| node.id).collect()
}

/// The order a build takes the exporting nodes in: by their graph
/// position, right to left (`OrderByDescending(Position.X)`, stable).
pub fn exporting_nodes_by_position(terrain: &Terrain) -> Vec<i32> {
    let mut nodes: Vec<&NodeData> = terrain.nodes.values().filter(|node| exports(node)).collect();
    nodes.sort_by(|left, right| right.position[0].total_cmp(&left.position[0]));
    nodes.iter().map(|node| node.id).collect()
}

/// Every node the build visits, in the order it builds them.
pub fn build_plan(terrain: &Terrain) -> Vec<i32> {
    let mut plan = Vec::new();
    for target in exporting_nodes_by_position(terrain) {
        for node in Engine::build_order(terrain, target) {
            if !plan.contains(&node) {
                plan.push(node);
            }
        }
    }
    plan
}

/// The pre-build checks (the edition limits aside).
pub fn check(terrain: &Terrain, build_type: BuildType) -> Result<(), BuildRefusal> {
    if exporting_nodes(terrain).is_empty() {
        return Err(BuildRefusal::Check("There are no nodes marked for export".to_string()));
    }
    if build_type != BuildType::Standard && bake::has_unbaked_generators(terrain) {
        match build_type {
            BuildType::Tiled => {
                let message = "There are unbaked generator nodes in the graph. They must be baked before a Tiled Build can be executed.\n\nUse the command Graph menu > Baking > Bake Required Nodes";
                return Err(BuildRefusal::Check(message.to_string()));
            }
            BuildType::Hybrid => {
                let message = "There are unbaked generator nodes in the graph. They must be baked before a Hybrid Build can be executed.\n\nUse the command Graph menu > Baking > Bake Required Nodes\n\nAlternatively, you can turn off Subdivision to none in Build Settings.";
                return Err(BuildRefusal::Check(message.to_string()));
            }
            _ => {}
        }
    }
    // Nodes with a Terrain or Height port must have it connected.
    let mut unconnected = String::new();
    for node in terrain.nodes.values() {
        let port = node.ports.iter().find(|port| port.name == "Terrain").or_else(|| node.ports.iter().find(|port| port.name == "Height"));
        if let Some(port) = port
            && port.record.is_none()
        {
            unconnected.push_str(&format!("{} : {}\r\n", node.id, node.name));
        }
    }
    if !unconnected.is_empty() {
        let message = format!("The following Height dependent nodes are not fully connected. Build will not proceed.\n\n{unconnected}");
        return Err(BuildRefusal::Check(message));
    }
    Ok(())
}

/// The format a node exports in: its `SaveDefinition`'s (missing reads as
/// Pure Raw, Newtonsoft's default), or the Export node's own.
fn export_format(node: &NodeData) -> FileFormat {
    if node.type_name == "Export" {
        let name = node.values.get("Format").and_then(Value::as_str).unwrap_or("EXR");
        return FileFormat::from_name(name).unwrap_or(FileFormat::Exr);
    }
    let name = save_definition(node).and_then(|definition| definition.get("Format")).and_then(Value::as_str);
    name.and_then(FileFormat::from_name).unwrap_or(FileFormat::PureRaw)
}

/// `SaveDefinition.Filename` (empty for the Export node, whose build
/// gives it a blank one).
fn save_filename(node: &NodeData) -> String {
    let filename = save_definition(node).and_then(|definition| definition.get("Filename")).and_then(Value::as_str);
    filename.unwrap_or_default().to_string()
}

/// The file a port exports to (`Node.GetExportFilename(portName, suffix)`,
/// then made a file name in the build folder as `Node.Export` does),
/// `suffix` being a tile's.
fn export_path(node: &NodeData, port_name: &str, suffix: &str, format: FileFormat, folder: &Path, definition: &BuildDefinition) -> PathBuf {
    let increment = definition.overwrite_mode == OverwriteMode::Increment;
    if node.type_name == "Export" {
        // `Export.GetExportFilename`: by location, the node's name for a
        // file name, no port name.
        let location = node.values.get("Location").and_then(Value::as_str).unwrap_or("BuildFolder");
        let output_path = node.values.get("OutputPath").and_then(Value::as_str).unwrap_or_default();
        let extension = format.extension();
        let path = match location {
            "Custom" => place_in_folder(&format!("{output_path}\\{}{suffix}{extension}", node.name), folder, false),
            "Explicit" => place_in_folder(&format!("{output_path}{extension}"), folder, false),
            _ => folder.join(format!("{}{suffix}{extension}", node.name)),
        };
        let node_increment = node.values.get("IfFileExists").and_then(Value::as_str) == Some("Increment");
        let path = if node_increment && location != "Explicit" { crate::destination::next_free_file(&path) } else { path };
        if increment {
            return crate::destination::next_free_file(&path);
        }
        return path;
    }
    let name = format!("{}{port_name}{suffix}{}", save_filename(node), format.extension());
    place_in_folder(&name, folder, increment)
}

/// What runs before every build: the region named
/// `argument` (`nerthus-build --region`), else the one the definition's
/// `Portion` holds, looked up in `Terrain.Regions` by name. When the
/// terrain has it, the build becomes a Region build of that region, at
/// its size; otherwise nothing changes. Returns the region found.
pub fn set_up_region(terrain: &Terrain, definition: &mut BuildDefinition, argument: Option<&str>) -> Option<Region> {
    let portion_name = definition.portion.as_ref().map(|portion| portion.name.clone());
    let name = argument.map(str::to_string).or(portion_name)?;
    let regions = region::regions(terrain);
    let found = regions.into_iter().find(|candidate| candidate.name == name)?;
    definition.build_type = BuildType::Region;
    definition.resolution = found.size.max(0) as usize;
    definition.portion = Some(found.clone());
    Some(found)
}

/// Runs a build of the asset's terrain: `engine` builds (a fresh one, set
/// up with the registry, automation and macros), `project_path` is the
/// saved project, `bakes` where its baked maps are. `cancel` stops it
/// between nodes. The definition is the one `set_up_region` leaves, as
/// builds run after it. This is a build as the editor starts one:
/// not silent, no profile.
#[allow(clippy::too_many_arguments)]
pub fn run(
    terrain: &Terrain,
    engine: &mut Engine,
    project_path: &Path,
    definition: &BuildDefinition,
    preferences: &ExportPreferences,
    bakes: Option<&BakeStore>,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(BuildEvent),
) -> Result<BuildReport, BuildRefusal> {
    let options = RunOptions::default();
    run_with(terrain, engine, project_path, definition, preferences, bakes, &options, cancel, on_event)
}

/// [`run`] with what the command line asks for (`--silent`,
/// `--profile`, `--ignorecache`). Once built, the report is left in the
/// build folder (unless silent), then the post-build script runs.
#[allow(clippy::too_many_arguments)]
pub fn run_with(
    terrain: &Terrain,
    engine: &mut Engine,
    project_path: &Path,
    definition: &BuildDefinition,
    preferences: &ExportPreferences,
    bakes: Option<&BakeStore>,
    options: &RunOptions,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(BuildEvent),
) -> Result<BuildReport, BuildRefusal> {
    let start_time = Local::now();
    engine.preferences.allow_values_beyond_range = preferences.allow_values_beyond_range;
    let mut report = match definition.build_type {
        BuildType::Standard => run_standard(terrain, engine, project_path, definition, preferences, bakes, options, cancel, on_event)?,
        BuildType::Hybrid | BuildType::Tiled => run_bucketed(terrain, engine, project_path, definition, preferences, bakes, options, cancel, on_event)?,
        BuildType::Region => run_region(terrain, engine, project_path, definition, preferences, bakes, options, cancel, on_event)?,
    };
    finish_report(&mut report, terrain, definition, options, start_time);
    if !report.cancelled {
        run_post_build_script(&definition.post_build_script, &mut report, on_event);
    }
    Ok(report)
}

/// The report of the build made, printed (unless silent) and
/// left in the build folder as `report.txt` and `report.json`; a build
/// that failed or was cancelled also leaves `CRASH_LOG.txt`.
fn finish_report(report: &mut BuildReport, terrain: &Terrain, definition: &BuildDefinition, options: &RunOptions, start_time: DateTime<Local>) {
    let standard = definition.build_type == BuildType::Standard;
    let result = if report.cancelled {
        "Cancelled".to_string()
    } else if standard {
        report.last_result.clone().unwrap_or_else(|| "Failed".to_string())
    } else {
        "Success".to_string()
    };
    let mut seconds_taken = Vec::new();
    if standard {
        for node in terrain.nodes.values() {
            let milliseconds = report.build_times.get(&node.id).copied().unwrap_or(0);
            seconds_taken.push((node.name.clone(), report::milliseconds_to_ticks(milliseconds)));
        }
    }
    let swarm = report::BuildReport {
        resolution: if standard { definition.resolution } else { 0 },
        node_count: report.node_count,
        result,
        start_time,
        end_time: Local::now(),
        seconds_taken,
    };
    if !options.silent {
        let summary = report::report_text(&report.header, &swarm, standard);
        let recorded = format!("{}{summary}", report.console);
        if let Err(error) = report::write_report(&report.folder, &recorded, &swarm) {
            report.failures.push((0, format!("Could not save report to disk! {error}")));
        }
        report.summary = Some(summary);
    }
    if swarm.result == "Failed" || swarm.result == "Cancelled" {
        let mut log = report.console.clone();
        for (node, problem) in &report.failures {
            log.push_str(&format!("{node:03}: {problem}\n"));
        }
        let _ = fs::write(report.folder.join("CRASH_LOG.txt"), log);
    }
    report.swarm = Some(swarm);
}

/// The header for a build into `folder` (`to_build` is what it
/// counts: the nodes, or a bucketed build's buckets).
fn report_header(terrain: &Terrain, definition: &BuildDefinition, project_path: &Path, folder: &Path, options: &RunOptions, to_build: usize) -> ReportHeader {
    let build_type = match definition.build_type {
        BuildType::Standard if definition.output_type == OutputType::CroppedTiles => "Standard Cropped to Tiles",
        other => other.name(),
    };
    let tiled = definition.build_type == BuildType::Tiled || definition.output_type == OutputType::CroppedTiles;
    let tiles = tiled.then_some(definition.number_of_tiles * definition.number_of_tiles);
    let region = match (definition.build_type, &definition.portion) {
        (BuildType::Region, Some(region)) => Some((region.name.clone(), region.position)),
        _ => None,
    };
    ReportHeader {
        project_path: project_path.to_string_lossy().into_owned(),
        folder: folder.to_string_lossy().into_owned(),
        build_type: build_type.to_string(),
        resolution: definition.resolution,
        profile: options.profile.clone(),
        nodes: to_build,
        exporting: exporting_nodes(terrain).len(),
        tiles,
        region,
    }
}

/// When the build asks for it and its folder is a new one each build: a
/// copy of the folder as `Latest`.
fn maintain_prime_folder(definition: &BuildDefinition, session: &mut Session<'_>) {
    if !definition.maintain_prime_folder || !swarm::destination_varies(definition) {
        return;
    }
    if let Err(error) = swarm::copy_to_latest(&session.folder) {
        let message = format!("Cannot keep the latest copy of the build: {error}");
        let folder = session.folder.clone();
        session.fail(0, folder, message);
    }
}

/// The file a post-build script is written to before it runs:
/// `PostBuildScript.bat`, or a shell script outside Windows.
pub const POST_BUILD_SCRIPT_FILE: &str = if cfg!(windows) { "PostBuildScript.bat" } else { "PostBuildScript.sh" };

/// "Executing POST BUILD SCRIPTS...": once the build has run (whatever its
/// nodes did), a script that is not blank is written as is to the build
/// folder and started from there without waiting for it, as the Script tab
/// says it will be.
fn run_post_build_script(script: &str, report: &mut BuildReport, on_event: &mut dyn FnMut(BuildEvent)) {
    if script.trim().is_empty() {
        return;
    }
    on_event(BuildEvent::Stage("Executing POST BUILD SCRIPTS...".to_string()));
    let path = report.folder.join(POST_BUILD_SCRIPT_FILE);
    if let Err(error) = fs::write(&path, script) {
        let message = format!("Cannot write the post-build script: {error}");
        report.failures.push((0, message.clone()));
        on_event(BuildEvent::FileFailed { node: 0, path, error: message });
        return;
    }
    report.files.push(path.clone());
    on_event(BuildEvent::FileWritten { node: 0, path: path.clone() });
    let mut command = if cfg!(windows) { std::process::Command::new("cmd") } else { std::process::Command::new("sh") };
    if cfg!(windows) {
        command.arg("/C");
    }
    command.arg(&path);
    command.current_dir(&report.folder);
    if let Err(error) = command.spawn() {
        let message = format!("Cannot run the post-build script: {error}");
        report.failures.push((0, message.clone()));
        on_event(BuildEvent::FileFailed { node: 0, path, error: message });
    }
}

/// The build folder, made. `region` is the region a Region
/// build builds (`[Region]` in the destination), `mutation` the build's
/// mutation (`[Mutation]`).
fn make_folder(project_path: &Path, definition: &BuildDefinition, preferences: &ExportPreferences, region: Option<&Region>, mutation: i32) -> Result<PathBuf, BuildRefusal> {
    let mut context = DestinationContext::new(&definition.destination, project_path, definition.resolution);
    context.builds_root = preferences.builds_root.clone();
    context.region = region.map(|region| region.name.clone());
    context.mutation = mutation;
    let folder = resolve_destination(&context);
    fs::create_dir_all(&folder).map_err(|error| BuildRefusal::Folder(format!("Cannot create the build folder {}: {error}", folder.display())))?;
    Ok(folder)
}

/// `CopyTerrain`: the project file copied into the build folder.
fn copy_project(project_path: &Path, folder: &Path, report: &mut BuildReport) {
    let Some(name) = project_path.file_name() else {
        return;
    };
    log::debug!("Copying Terrain File");
    let copied = fs::copy(project_path, folder.join(name));
    if let Err(error) = copied {
        log::error!("{error}");
        report.failures.push((0, format!("Cannot copy the project into the build folder: {error}")));
    }
}

/// What a build carries from step to step.
struct Session<'a> {
    terrain: &'a Terrain,
    definition: &'a BuildDefinition,
    preferences: &'a ExportPreferences,
    folder: PathBuf,
    report: BuildReport,
    /// The exporting nodes that have written their files
    /// (`ExportState.HasExported`).
    exported: HashSet<i32>,
    on_event: &'a mut dyn FnMut(BuildEvent),
}

impl Session<'_> {
    fn event(&mut self, event: BuildEvent) {
        (self.on_event)(event);
    }

    fn fail(&mut self, node: i32, path: PathBuf, message: String) {
        self.report.failures.push((node, message.clone()));
        self.event(BuildEvent::FileFailed { node, path, error: message });
    }

    /// Text for the console, kept for the report.
    fn print(&mut self, text: String) {
        self.report.console.push_str(&text);
        self.event(BuildEvent::Printed(text));
    }

    /// The header the build prints once its folder is made.
    fn print_header(&mut self, header: ReportHeader, node_count: usize) {
        let text = report::header_text(&header);
        self.report.header = header;
        self.report.node_count = node_count;
        self.print(text);
    }
}

/// A Standard build.
#[allow(clippy::too_many_arguments)]
fn run_standard(
    terrain: &Terrain,
    engine: &mut Engine,
    project_path: &Path,
    definition: &BuildDefinition,
    preferences: &ExportPreferences,
    bakes: Option<&BakeStore>,
    options: &RunOptions,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(BuildEvent),
) -> Result<BuildReport, BuildRefusal> {
    check(terrain, BuildType::Standard)?;
    log::debug!("Creating Build Path");
    let folder = make_folder(project_path, definition, preferences, None, engine.mutation)?;

    engine.application_mode = ApplicationMode::Swarm;
    engine.project_path = Some(project_path.to_path_buf());
    let resolution = Resolution::square(definition.resolution);
    // The crucial bakes are loaded before a Standard build, fitted to the
    // build's resolution, unless told to ignore them (`--ignorecache`):
    // then the baked nodes build as the others do.
    if let Some(store) = bakes
        && !options.ignore_cache
    {
        let fit = BakeFit::Whole { resolution, high_quality: true };
        bake::load_bakes(engine, terrain, store, fit, false, true);
    }

    let mut session = Session {
        terrain,
        definition,
        preferences,
        folder: folder.clone(),
        report: BuildReport { folder: folder.clone(), ..BuildReport::default() },
        exported: HashSet::new(),
        on_event,
    };
    if definition.copy_terrain {
        copy_project(project_path, &folder, &mut session.report);
    }
    let header = report_header(terrain, definition, project_path, &folder, options, terrain.nodes.len());
    session.print_header(header, terrain.nodes.len());

    // Every result but the bakes' forgotten, then the exporting nodes right
    // to left.
    log::info!("Invalidating nodes");
    engine.forget_unbaked();
    let plan = build_plan(terrain);
    session.event(BuildEvent::Started { folder: folder.clone(), nodes: plan.len() });
    log::info!("Starting build");
    for target in exporting_nodes_by_position(terrain) {
        if let Some(node) = terrain.nodes.get(&target) {
            log::info!("Building {}", node.name);
        }
        let finished = build_chain(engine, terrain, target, resolution, cancel, &mut session, &mut |engine, session, node| {
            post_build_and_purge(node, engine, session);
        });
        if !finished {
            log::error!("Build cancelled.");
            session.report.cancelled = true;
            break;
        }
    }
    maintain_prime_folder(definition, &mut session);
    Ok(session.report)
}

/// Save cached build: the maps the editor already holds written as a
/// build would write them, with nothing built. The build folder is made
/// and the project copied into it when the build says so, then each node
/// marked for export, in the project's order, writes its marked ports
/// (`Node.PostBuild`). `engine` holds the maps, built at the build's
/// resolution; `region` is the region the editor previews (`[Region]` in
/// the destination). Unlike a build, it leaves no report, keeps no
/// `Latest` copy and runs no post-build script.
pub fn export_built(
    terrain: &Terrain,
    engine: &Engine,
    project_path: &Path,
    definition: &BuildDefinition,
    preferences: &ExportPreferences,
    region: Option<&Region>,
    on_event: &mut dyn FnMut(BuildEvent),
) -> Result<BuildReport, BuildRefusal> {
    let folder = make_folder(project_path, definition, preferences, region, engine.mutation)?;
    let mut session = Session {
        terrain,
        definition,
        preferences,
        folder: folder.clone(),
        report: BuildReport { folder: folder.clone(), ..BuildReport::default() },
        exported: HashSet::new(),
        on_event,
    };
    if definition.copy_terrain {
        copy_project(project_path, &folder, &mut session.report);
    }
    let exporting = exporting_nodes(terrain);
    let count = exporting.len();
    session.event(BuildEvent::Started { folder, nodes: count });
    for (position, id) in exporting.into_iter().enumerate() {
        let Some(node) = terrain.nodes.get(&id) else {
            continue;
        };
        session.event(BuildEvent::Exporting { node: id, index: position + 1, count });
        post_build(node, engine, &mut session, None);
    }
    Ok(session.report)
}

/// A Region build. The build first makes the region the current portion
/// and loads every bake, each cut to the region and resized to the build
/// resolution, the region's size. Then the checks and the build folder,
/// and the build itself, which forgets every node's maps but the baked
/// ones and builds the exporting nodes right to left, each writing single
/// images. Unlike a Standard build, it does not copy the project into the
/// build folder.
#[allow(clippy::too_many_arguments)]
fn run_region(
    terrain: &Terrain,
    engine: &mut Engine,
    project_path: &Path,
    definition: &BuildDefinition,
    preferences: &ExportPreferences,
    bakes: Option<&BakeStore>,
    options: &RunOptions,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(BuildEvent),
) -> Result<BuildReport, BuildRefusal> {
    check(terrain, BuildType::Region)?;
    if definition.resolution == 0 {
        return Err(BuildRefusal::Check("The region's size is 0.".to_string()));
    }
    let region = definition.portion.as_ref();
    let folder = make_folder(project_path, definition, preferences, region, engine.mutation)?;

    engine.application_mode = ApplicationMode::Swarm;
    engine.project_path = Some(project_path.to_path_buf());
    let resolution = Resolution::square(definition.resolution);
    if let Some(store) = bakes {
        let fit = BakeFit::Region { region, resolution, high_quality: true };
        bake::load_bakes(engine, terrain, store, fit, true, true);
    }

    let mut session = Session {
        terrain,
        definition,
        preferences,
        folder: folder.clone(),
        report: BuildReport { folder: folder.clone(), ..BuildReport::default() },
        exported: HashSet::new(),
        on_event,
    };
    let header = report_header(terrain, definition, project_path, &folder, options, terrain.nodes.len());
    session.print_header(header, terrain.nodes.len());
    engine.forget_unbaked();
    let plan = build_plan(terrain);
    session.event(BuildEvent::Started { folder: folder.clone(), nodes: plan.len() });
    for target in exporting_nodes_by_position(terrain) {
        let finished = build_chain(engine, terrain, target, resolution, cancel, &mut session, &mut |engine, session, node| {
            post_build_and_purge(node, engine, session);
        });
        if !finished {
            session.report.cancelled = true;
            break;
        }
    }
    maintain_prime_folder(definition, &mut session);
    Ok(session.report)
}

/// Builds an exporting node's chain: a bypassed node is skipped, a baked
/// one only exported, a built one left alone; otherwise every node of the
/// chain not built yet is built, and `exported` runs for each node built
/// and each baked node met. False when cancelled.
fn build_chain(
    engine: &mut Engine,
    terrain: &Terrain,
    target: i32,
    resolution: Resolution,
    cancel: &AtomicBool,
    session: &mut Session<'_>,
    exported: &mut dyn FnMut(&mut Engine, &mut Session<'_>, &NodeData),
) -> bool {
    let Some(target_node) = terrain.nodes.get(&target) else {
        return true;
    };
    // What the build loop gives back, which the report shows for the last
    // exporting node: `Success` (Succeeded), `AlreadyBuilt`, `Skipped`,
    // `Baked` or `Failed`.
    match StateOverride::of(target_node) {
        StateOverride::Bypassed => {
            session.report.last_result = Some("Skipped".to_string());
            return true;
        }
        StateOverride::Baked if engine.is_baked(target) => {
            session.report.last_result = Some("AlreadyBuilt".to_string());
            exported(engine, session, target_node);
            return true;
        }
        _ => {}
    }
    if engine.state(target) == NodeState::Built {
        session.report.last_result = Some("AlreadyBuilt".to_string());
        return true;
    }
    // The chain's result: the last node it got to, `Failed` before any.
    let mut chain_result = "Failed";
    for id in Engine::build_order(terrain, target) {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        let Some(node) = terrain.nodes.get(&id) else {
            continue;
        };
        if StateOverride::of(node) == StateOverride::Baked && engine.is_baked(id) {
            chain_result = "Baked";
            exported(engine, session, node);
            continue;
        }
        if engine.state(id) == NodeState::Built {
            continue;
        }
        session.event(BuildEvent::NodeStarted { node: id });
        let started = Instant::now();
        let result = engine.build(terrain, id, resolution);
        let milliseconds = started.elapsed().as_millis() as u64;
        session.report.build_times.insert(id, milliseconds);
        let error = match &result {
            BuildResult::Failed(error) => Some(error.to_string()),
            _ => None,
        };
        chain_result = match &result {
            BuildResult::Succeeded => "Success",
            BuildResult::AlreadyBuilt => "AlreadyBuilt",
            BuildResult::Skipped => "Skipped",
            BuildResult::Failed(_) => "Failed",
        };
        session.event(BuildEvent::NodeFinished { node: id, milliseconds, error: error.clone() });
        if let Some(error) = error {
            session.print(report::failed_node_line(id, &node.name));
            session.report.failures.push((id, error));
            continue;
        }
        if matches!(result, BuildResult::Succeeded) {
            exported(engine, session, node);
        }
    }
    session.report.last_result = Some(chain_result.to_string());
    true
}

/// The name a port's files carry: the delimiter and the port, or nothing
/// for the primary port when the build says so.
fn port_file_name(port: &str, definition: &BuildDefinition, preferences: &ExportPreferences) -> String {
    if port == "Out" && definition.remove_primary_port_name {
        return String::new();
    }
    format!("{}{port}", preferences.filename_delimiter)
}

/// `Node.PostBuild` for a node the build loop built: its files written
/// when it exports. With Purge unnecessary cache during build on, every
/// result the build no longer needs is let go before a node that exports
/// nothing returns, and again once an exporting node has written its
/// files.
fn post_build_and_purge(node: &NodeData, engine: &mut Engine, session: &mut Session<'_>) {
    let exporting = exports(node);
    let purging = session.preferences.offload_build_cache;
    if purging && !exporting {
        purge_unneeded(engine, session);
    }
    post_build(node, engine, session, None);
    if !exporting {
        return;
    }
    session.exported.insert(node.id);
    if purging {
        purge_unneeded(engine, session);
    }
}

/// The purge: every built node's results go unless the node still has to
/// export, is the underlay, is an accumulation that a node in the project
/// reads, or feeds a node not built yet.
fn purge_unneeded(engine: &mut Engine, session: &Session<'_>) {
    let terrain = session.terrain;
    let catalogue = Catalogue::get();
    let reads_accumulation = terrain.nodes.values().any(|node| catalogue.node(&node.type_name).is_some_and(|spec| spec.has_flag("UsesAccumulation")));
    let mut purged = Vec::new();
    for node in terrain.nodes.values() {
        if still_needed(node, engine, session, reads_accumulation) {
            continue;
        }
        purged.push((node.id, node.name.clone()));
    }
    for (id, name) in purged {
        if engine.cache.ports(id).next().is_none() {
            continue;
        }
        engine.cache.remove_node(id);
        log::debug!("Node {id} :: {name} purged.");
    }
}

/// Whether a node's results must stay for the rest of the build.
fn still_needed(node: &NodeData, engine: &Engine, session: &Session<'_>, reads_accumulation: bool) -> bool {
    if exports(node) && !session.exported.contains(&node.id) {
        return true;
    }
    if engine.state(node.id) != NodeState::Built {
        return true;
    }
    let spec = Catalogue::get().node(&node.type_name);
    if spec.is_some_and(|spec| spec.has_flag("PreventPurge")) {
        return true;
    }
    let accumulates = spec.is_some_and(|spec| spec.accumulator.is_some());
    if accumulates && reads_accumulation {
        return true;
    }
    if engine.underlay == node.id {
        return true;
    }
    let consumers = session.terrain.nodes.values().filter(|other| other.ports.iter().any(|port| port.is_input() && port.record.as_ref().is_some_and(|record| record.from == node.id)));
    let mut waiting = consumers.map(|other| engine.state(other.id));
    waiting.any(|state| state == NodeState::Unbuilt)
}

/// `Node.PostBuild`: every marked output port written, named
/// `<file name><delimiter><port>` (the primary port's name left out when
/// the build says so) — one image, or (Tiled Images without subdivision)
/// tiles cut from it. `tile` is the bucket a bucketed build is on
/// (`BuildContext.Tile`).
fn post_build(node: &NodeData, engine: &Engine, session: &mut Session<'_>, tile: Option<(i32, i32)>) {
    if !exports(node) {
        return;
    }
    // Mesher has its own PostBuild; PointCloud, TextureBaker and MeshWarp
    // have their own file names and Export.
    let folder = session.folder.clone();
    let context = mesh_exports::PostBuildContext {
        terrain: session.terrain,
        engine,
        folder: &folder,
        definition: session.definition,
        preferences: session.preferences,
        tile,
    };
    let mut on_file = |result: mesh_exports::FileResult| match result {
        mesh_exports::FileResult::Written(path) => {
            session.report.files.push(path.clone());
            session.event(BuildEvent::FileWritten { node: node.id, path });
        }
        mesh_exports::FileResult::Failed { path, message } => session.fail(node.id, path, message),
    };
    if mesh_exports::post_build(&context, node, &mut on_file) {
        return;
    }
    let definition = session.definition;
    let single = definition.output_type == OutputType::SingleImage || definition.build_type == BuildType::Region;
    for port in node.ports.iter().filter(|port| port.is_output() && port.is_exporting) {
        let port_name = port_file_name(&port.name, definition, session.preferences);
        let Some(map) = engine.cache.get(node.id, &port.name) else {
            let message = format!("{} has no {} output to export", node.name, port.name);
            let folder = session.folder.clone();
            session.fail(node.id, folder, message);
            continue;
        };
        if single {
            let format = export_format(node);
            let path = export_path(node, &port_name, "", format, &session.folder, definition);
            export_map(node, &port.name, &map, &path, None, session);
        } else {
            export_cropped_tiles(node, &port.name, &port_name, &map, session);
        }
    }
}

/// `Node.PostBuild`'s tiled branch: the map cut into `NumberOfTiles`²
/// tiles of `TileResolution`, each grown by a one-pixel strip from its
/// neighbours (`TileVertexBorder.Expand`, which keeps the first channel
/// only), written to the port's folder (by the save name) with
/// the tile's suffix. These tiles are never flipped.
fn export_cropped_tiles(node: &NodeData, port: &str, port_name: &str, map: &Map, session: &mut Session<'_>) {
    let definition = session.definition;
    let tile_size = definition.tile_resolution;
    let count = if definition.build_type == BuildType::Region { map.width() / tile_size } else { definition.tiles_per_side() };
    let folder = tiles::organized_folder(&session.folder, definition.organize_files, &save_filename(node), port);
    if let Err(error) = fs::create_dir_all(&folder) {
        session.fail(node.id, folder.clone(), format!("Cannot create {}: {error}", folder.display()));
        return;
    }
    let mut cut = Vec::with_capacity(count * count);
    for row in 0..count {
        for column in 0..count {
            let window = Resolution::square(tile_size);
            cut.push((column, row, transformer::crop(map, column * tile_size, row * tile_size, window)));
        }
    }
    let grid: Vec<GridTile<'_>> = cut.iter().map(|(x, y, map)| GridTile { x: *x, y: *y, map }).collect();
    let expanded = match tiles::expand_tiles(&grid, definition.tiles_per_side(), 1) {
        Ok(expanded) => expanded,
        Err(error) => {
            session.fail(node.id, folder, error);
            return;
        }
    };
    let format = export_format(node);
    for ((column, row, _), tile_map) in cut.iter().zip(expanded) {
        let suffix = definition.tile_suffix(*column, *row, definition.tiles_per_side());
        let path = export_path(node, port_name, &suffix, format, &folder, definition);
        export_map(node, port, &tile_map, &path, Some((*column, *row)), session);
    }
}

/// `Node.Export`: the map clamped and written in the
/// node's format. Every image a build writes goes through here.
fn export_map(node: &NodeData, port: &str, map: &Map, path: &Path, tile: Option<(usize, usize)>, session: &mut Session<'_>) {
    // The mesh nodes export only through their own PostBuild (see
    // `mesh_exports`); bucketed builds that reach here for them write
    // nothing.
    if mesh_exports::handles(node) {
        let message = format!("{} exports are only supported for single-image builds", node.type_name);
        session.fail(node.id, path.to_path_buf(), message);
        return;
    }
    let _ = (port, tile);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let format = export_format(node);
    let prepared = clamp_for_export(map);
    let mut options = session.preferences.write.clone();
    options.color_space = session.definition.color_space.clone();
    match write_map(format, path, &prepared, &options) {
        Ok(()) => {
            session.report.files.push(path.to_path_buf());
            session.event(BuildEvent::FileWritten { node: node.id, path: path.to_path_buf() });
        }
        Err(error) => session.fail(node.id, path.to_path_buf(), error.to_string()),
    }
}

/// A Hybrid or Tiled build.
#[allow(clippy::too_many_arguments)]
fn run_bucketed(
    terrain: &Terrain,
    engine: &mut Engine,
    project_path: &Path,
    definition: &BuildDefinition,
    preferences: &ExportPreferences,
    bakes: Option<&BakeStore>,
    options: &RunOptions,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(BuildEvent),
) -> Result<BuildReport, BuildRefusal> {
    // Nodes connected to nothing are deleted first, then the checks run on
    // what is left.
    let mut working = terrain.clone();
    for orphan in bake::orphans(terrain) {
        working.nodes.shift_remove(&orphan);
    }
    check(&working, definition.build_type)?;

    // `NumberOfTiles` follows the resolution here.
    let mut definition = definition.clone();
    definition.number_of_tiles = definition.resolution / definition.tile_resolution;
    let geometry = definition.geometry();
    if geometry.bucket_count() == 0 || geometry.tiles_per_bucket() == 0 {
        let message = format!(
            "The bucket size ({}) and tile size ({}) must not be larger than the resolution ({}).",
            definition.bucket_resolution, definition.tile_resolution, definition.resolution
        );
        return Err(BuildRefusal::Check(message));
    }

    // The build folder and its `.BuildArtifacts`.
    let folder = make_folder(project_path, &definition, preferences, None, engine.mutation)?;
    let artifacts = folder.join(".BuildArtifacts");
    fs::create_dir_all(&artifacts).map_err(|error| BuildRefusal::Folder(format!("Cannot create {}: {error}", artifacts.display())))?;

    engine.application_mode = ApplicationMode::Swarm;
    engine.project_path = Some(project_path.to_path_buf());
    let mut session = Session {
        terrain: &working,
        definition: &definition,
        preferences,
        folder: folder.clone(),
        report: BuildReport { folder: folder.clone(), ..BuildReport::default() },
        exported: HashSet::new(),
        on_event,
    };
    if definition.copy_terrain {
        copy_project(project_path, &folder, &mut session.report);
    }

    let grid = tiles::buckets(geometry.bucket_count());
    // The header counts the buckets (`BucketCount²`) as what it builds.
    let header = report_header(&working, &definition, project_path, &folder, options, grid.len());
    session.print_header(header, working.nodes.len());
    let plan = build_plan(&working);
    session.event(BuildEvent::Started { folder: folder.clone(), nodes: plan.len() * grid.len() });
    let size = Resolution::square(geometry.build_size());
    let targets = exporting_nodes_by_position(&working);
    for (index, bucket) in grid.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            log::error!("Build cancelled.");
            session.report.cancelled = true;
            return Ok(session.report);
        }
        session.event(BuildEvent::BucketStarted { index, count: grid.len() });
        // Every result but the bakes' forgotten, the bakes cut for this
        // bucket, then the exporting nodes built at the bucket's size.
        log::info!("Starting bucket build");
        engine.forget_unbaked();
        if let Some(store) = bakes {
            log::info!("Loading cache");
            let fit = BakeFit::Bucket { bucket, geometry: &geometry };
            bake::load_bakes(engine, &working, store, fit, true, true);
        }
        for target in &targets {
            let finished = build_chain(engine, &working, *target, size, cancel, &mut session, &mut |engine, session, node| {
                export_bucket_bin(node, engine, &artifacts, bucket, session);
            });
            if !finished {
                log::error!("Build cancelled.");
                session.report.cancelled = true;
                return Ok(session.report);
            }
        }
    }

    // The edges, then the build's own output, each step's end
    // written to the console.
    session.event(BuildEvent::Stage("Blending Edges".to_string()));
    if geometry.bucket_count() != 1 {
        blend_buckets(&artifacts, &grid, &geometry, &mut session);
    }
    session.print(report::console_line("Edges blended."));
    match definition.build_type {
        BuildType::Hybrid => {
            session.event(BuildEvent::Stage("Saving Data".to_string()));
            stitch_buckets(engine, &artifacts, &grid, &geometry, &mut session);
            session.print(report::console_line("Data written to files."));
        }
        BuildType::Tiled => {
            session.event(BuildEvent::Stage("Separating Data".to_string()));
            let separated = separate_tiles(&artifacts, &grid, &geometry, &mut session);
            session.print(report::console_line("Data separation completed."));
            session.event(BuildEvent::Stage("Matching Edges".to_string()));
            match_tile_edges(&separated, &mut session);
            session.print(report::console_line("Edges matched."));
            session.event(BuildEvent::Stage("Saving Data".to_string()));
            save_tiles(&separated, &mut session);
            session.print(report::console_line("Data written to files."));
        }
        _ => {}
    }
    session.event(BuildEvent::Stage("Cleaning up".to_string()));
    if !definition.preserve_tiled_cache {
        let _ = fs::remove_dir_all(&artifacts);
    }
    Ok(session.report)
}

/// `{id}_{name}_{port}`: what a node's port's bucket files are
/// named after.
fn bin_stem(node: &NodeData, port: &str) -> String {
    format!("{}_{}_{port}", node.id, node.name)
}

/// A bucket's file for `stem`.
fn bucket_bin(artifacts: &Path, stem: &str, position: (usize, usize)) -> PathBuf {
    artifacts.join(format!("{stem}_y{}_x{}.bin", position.1, position.0))
}

/// The exporting nodes and their marked ports, in the project's order
/// (`Node.Exportables`).
fn marked_ports(terrain: &Terrain) -> Vec<(&NodeData, Vec<String>)> {
    let mut marked = Vec::new();
    for node in terrain.nodes.values().filter(|node| exports(node)) {
        let ports = node.ports.iter().filter(|port| port.is_output() && port.is_exporting).map(|port| port.name.clone()).collect();
        marked.push((node, ports));
    }
    marked
}

/// `Node.ExportBucketBin`: each marked port of a node built for a bucket
/// written as it would be exported (clamped) to the bucket's `.bin`.
fn export_bucket_bin(node: &NodeData, engine: &Engine, artifacts: &Path, bucket: &Bucket, session: &mut Session<'_>) {
    for port in node.ports.iter().filter(|port| port.is_output() && port.is_exporting) {
        let Some(map) = engine.cache.get(node.id, &port.name) else {
            continue;
        };
        let prepared = clamp_for_export(&map);
        let path = bucket_bin(artifacts, &bin_stem(node, &port.name), (bucket.x, bucket.y));
        if let Err(error) = tiles::write_bin(&path, &prepared) {
            session.fail(node.id, path, error.to_string());
        }
    }
}

/// Every bucket's left edge blended into its left
/// neighbour's (the first column has none), then every top edge into the
/// neighbour above, rewriting the buckets' files in place.
fn blend_buckets(artifacts: &Path, grid: &[Bucket], geometry: &BucketGeometry, session: &mut Session<'_>) {
    let margin_size = geometry.size_with_margin();
    let edge = geometry.edge_size();
    let marked = marked_ports(session.terrain);
    for side in [Side::Left, Side::Top] {
        for bucket in grid {
            for (node, ports) in &marked {
                for port in ports {
                    let stem = bin_stem(node, port);
                    let path = bucket_bin(artifacts, &stem, (bucket.x, bucket.y));
                    // A missing bucket file (a node that failed) is skipped
                    // rather than stopping the whole build.
                    let Ok(mut map) = tiles::read_bin(&path, margin_size) else {
                        continue;
                    };
                    let neighbour = match side {
                        Side::Left if !bucket.position.is_left_edge() => Some(bucket.left()),
                        Side::Top if !bucket.position.is_top_edge() => Some(bucket.top()),
                        _ => None,
                    };
                    if let Some(position) = neighbour {
                        let Ok(edge_map) = tiles::read_bin(&bucket_bin(artifacts, &stem, position), margin_size) else {
                            continue;
                        };
                        tiles::blend_edge(&mut map, &edge_map, side, margin_size, edge);
                    }
                    if let Err(error) = tiles::write_bin(&path, &map) {
                        session.fail(node.id, path, error.to_string());
                    }
                }
            }
        }
    }
}

/// A bucket's own part: its margins cut away when the build has more than
/// one bucket.
fn bucket_core(map: &Map, geometry: &BucketGeometry) -> Map {
    if geometry.bucket_count() <= 1 {
        return map.clone();
    }
    let edge = geometry.edge_size();
    transformer::crop(map, edge, edge, Resolution::square(geometry.bucket_resolution))
}

/// Each exporting node's marked ports put back together from
/// the buckets (each bucket's own part pasted at its place), handed to
/// the engine's cache and exported as the node would be (`PostBuild`).
fn stitch_buckets(engine: &mut Engine, artifacts: &Path, grid: &[Bucket], geometry: &BucketGeometry, session: &mut Session<'_>) {
    let margin_size = geometry.size_with_margin();
    let world = geometry.resolution;
    let terrain = session.terrain;
    for (node, ports) in marked_ports(terrain) {
        let mut complete = true;
        for port in &ports {
            let stem = bin_stem(node, port);
            let mut whole: Option<Map> = None;
            for bucket in grid {
                let path = bucket_bin(artifacts, &stem, (bucket.x, bucket.y));
                let Ok(bucket_map) = tiles::read_bin(&path, margin_size) else {
                    complete = false;
                    continue;
                };
                let target = whole.get_or_insert_with(|| Map::new(Resolution::square(world), bucket_map.channel_count()));
                let core = bucket_core(&bucket_map, geometry);
                // `Bucket.TopLeft`: the bucket's corner over the world's width.
                let left = (bucket.x * geometry.bucket_resolution) as f32 / world as f32;
                let top = (bucket.y * geometry.bucket_resolution) as f32 / world as f32;
                transformer::insert(&core, target, (left, top), BlendMode::Blend, None, AnchorPosition::TopLeft);
            }
            if let Some(map) = whole {
                engine.cache.store(node.id, port, Arc::new(map));
            }
        }
        if !complete {
            let message = format!("{} has buckets missing; its files may be incomplete", node.name);
            session.report.failures.push((node.id, message));
        }
        post_build_and_purge(node, engine, session);
    }
}

/// One tile cut from a bucket: where it is kept, its name
/// suffix and its place in the whole tile grid.
struct SeparatedTile {
    file: PathBuf,
    suffix: String,
    world: (usize, usize),
}

/// One bucket's tiles of one node, by port in the order they
/// were cut.
struct NodeTiles {
    node: i32,
    ports: Vec<(String, Vec<SeparatedTile>)>,
}

/// Each bucket's own part cut into tiles (flipped top to bottom
/// with `FlipY`), each kept in a `t_<random>.bin`.
fn separate_tiles(artifacts: &Path, grid: &[Bucket], geometry: &BucketGeometry, session: &mut Session<'_>) -> Vec<NodeTiles> {
    let definition = session.definition;
    let margin_size = geometry.size_with_margin();
    let tile_size = geometry.tile_resolution;
    let per_bucket = geometry.tiles_per_bucket();
    let tiles_per_side = definition.number_of_tiles;
    let mut separated = Vec::new();
    for bucket in grid {
        for (node, ports) in marked_ports(session.terrain) {
            let mut entry = NodeTiles { node: node.id, ports: Vec::new() };
            for port in ports {
                let path = bucket_bin(artifacts, &bin_stem(node, &port), (bucket.x, bucket.y));
                let Ok(bucket_map) = tiles::read_bin(&path, margin_size) else {
                    continue;
                };
                let core = bucket_core(&bucket_map, geometry);
                let mut cut = Vec::with_capacity(per_bucket * per_bucket);
                for row in 0..per_bucket {
                    for column in 0..per_bucket {
                        let window = Resolution::square(tile_size);
                        let mut tile = transformer::crop(&core, column * tile_size, row * tile_size, window);
                        if definition.flip_y {
                            tile = nerthus_nodes::maths::flip_y(&tile);
                        }
                        let random = uuid::Uuid::new_v4().simple().to_string();
                        let file = artifacts.join(format!("t_{}.bin", &random[..8]));
                        if let Err(error) = tiles::write_bin(&file, &tile) {
                            session.fail(node.id, file.clone(), error.to_string());
                        }
                        // `Tile.Position`: the tile's column and row in the whole grid.
                        let world = (bucket.x * per_bucket + column, bucket.y * per_bucket + row);
                        let suffix = definition.tile_suffix(world.0, world.1, tiles_per_side);
                        cut.push(SeparatedTile { file, suffix, world });
                    }
                }
                entry.ports.push((port, cut));
            }
            separated.push(entry);
        }
    }
    separated
}

/// A tile file read back with the size it was written at: one channel
/// when its samples make a square (overlap strips leave tiles one
/// channel), three interleaved otherwise. The size the file holds is the
/// one read: taking it as `TileResolution` wide would overrun tiles with
/// overlap strips.
fn read_tile(path: &Path, tile_size: usize) -> std::io::Result<Map> {
    let samples = tiles::read_samples(path)?;
    let square = (samples.len() as f64).sqrt() as usize;
    if square * square == samples.len() && square != tile_size {
        return Ok(tiles::single_channel(samples));
    }
    Ok(tiles::samples_to_map(samples, tile_size))
}

/// With `OverlappingPixels`, every tile grown by that many
/// pixels from its right, bottom and corner neighbours — among the same
/// bucket's tiles only — and written back (flipped
/// again with `FlipY`).
fn match_tile_edges(separated: &[NodeTiles], session: &mut Session<'_>) {
    let definition = session.definition;
    let overlap = definition.overlapping_pixels;
    if overlap == 0 {
        return;
    }
    let tile_size = definition.tile_resolution;
    for entry in separated {
        for (_, list) in &entry.ports {
            let mut maps = Vec::with_capacity(list.len());
            for tile in list {
                match read_tile(&tile.file, tile_size) {
                    Ok(map) => maps.push(map),
                    Err(error) => {
                        session.fail(entry.node, tile.file.clone(), error.to_string());
                        return;
                    }
                }
            }
            let grid: Vec<GridTile<'_>> = list.iter().zip(&maps).map(|(tile, map)| GridTile { x: tile.world.0, y: tile.world.1, map }).collect();
            let expanded = match tiles::expand_tiles(&grid, definition.number_of_tiles, overlap) {
                Ok(expanded) => expanded,
                Err(error) => {
                    let folder = session.folder.clone();
                    session.fail(entry.node, folder, error);
                    continue;
                }
            };
            for (tile, map) in list.iter().zip(expanded) {
                let stored = if definition.flip_y { nerthus_nodes::maths::flip_y(&map) } else { map };
                if let Err(error) = tiles::write_bin(&tile.file, &stored) {
                    session.fail(entry.node, tile.file.clone(), error.to_string());
                }
            }
        }
    }
}

/// Every tile exported as its node would export it, into the
/// node's (and port's) folder (by the node's name), named with
/// the tile's suffix.
fn save_tiles(separated: &[NodeTiles], session: &mut Session<'_>) {
    let definition = session.definition;
    let tile_size = definition.tile_resolution;
    for entry in separated {
        let Some(node) = session.terrain.nodes.get(&entry.node) else {
            continue;
        };
        for (port, list) in &entry.ports {
            let folder = tiles::organized_folder(&session.folder, definition.organize_files, &node.name, port);
            let port_name = port_file_name(port, definition, session.preferences);
            let format = export_format(node);
            for tile in list {
                let map = match read_tile(&tile.file, tile_size) {
                    Ok(map) => map,
                    Err(error) => {
                        session.fail(node.id, tile.file.clone(), error.to_string());
                        continue;
                    }
                };
                if let Err(error) = fs::create_dir_all(&folder) {
                    session.fail(node.id, folder.clone(), error.to_string());
                    continue;
                }
                let path = export_path(node, &port_name, &tile.suffix, format, &folder, definition);
                export_map(node, port, &map, &path, Some(tile.world), session);
            }
        }
    }
}

/// The build folder a build would use now (for Build Settings' preview).
pub fn preview_destination(project_path: &Path, definition: &BuildDefinition, preferences: &ExportPreferences) -> PathBuf {
    let mut context = DestinationContext::new(&definition.destination, project_path, definition.resolution);
    context.builds_root = preferences.builds_root.clone();
    resolve_destination(&context)
}

/// A path string from the project as a path here.
pub fn project_path_text(text: &str) -> PathBuf {
    to_path(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerthus_engine::ConnectionRecord;
    use serde_json::json;

    #[test]
    fn missing_members_read_as_newtonsoft_fills_them() {
        let saved = json!({"Resolution": 4096, "TileResolution": 1024, "BucketResolution": 2048, "NumberOfTiles": 4, "EdgeBlending": 0.25, "TileZeroIndex": true, "TilePattern": "_y%Y%_x%X%", "OrganizeFiles": "NodeSubFolder", "Type": "Tiled", "Subdivision": "Balanced", "OutputType": "CroppedTiles"});
        let definition = BuildDefinition::from_json(Some(&saved));
        assert_eq!(definition.build_type, BuildType::Tiled);
        assert_eq!(definition.subdivision, Subdivision::Balanced);
        assert_eq!(definition.output_type, OutputType::CroppedTiles);
        assert_eq!(definition.organize_files, OrganizeFiles::NodeSubFolder);
        assert!(definition.tile_zero_index);
        assert!(!definition.flip_y);
        assert_eq!(definition.geometry().bucket_count(), 2);
        assert_eq!(definition.geometry().size_with_margin(), 3072);
        let bare = BuildDefinition::from_json(Some(&json!({})));
        assert_eq!(bare.edge_blending, 0.0);
        assert!(!bare.tile_zero_index);
        assert_eq!(bare.organize_files, OrganizeFiles::None);
        assert_eq!(bare.tile_resolution, DEFAULT_TILE_RESOLUTION);
    }

    /// A lone Perlin marked for export as `Perlin_Out.png`.
    fn exporting_terrain() -> Terrain {
        let spec = Catalogue::get().node("Perlin").expect("Perlin is in the catalogue");
        let mut perlin = NodeData::new(spec, 1, 1);
        let save = json!({"IsEnabled": true, "Filename": "Perlin", "Format": "PNG16"});
        perlin.base.insert("SaveDefinition".to_string(), save);
        let mut terrain = Terrain { id: String::new(), nodes: Default::default(), width: 5000.0, height: 2500.0, extra: serde_json::Map::new() };
        terrain.nodes.insert(1, perlin);
        terrain
    }

    #[test]
    fn a_cached_export_writes_what_a_build_writes() {
        let root = std::env::temp_dir().join(format!("nerthus-cached-export-{}", std::process::id()));
        let project_path = root.join("Cached.terrain");
        let terrain = exporting_terrain();
        let registry = Arc::new(nerthus_nodes::registry());
        let preferences = ExportPreferences::default();

        let mut built_definition = BuildDefinition { resolution: 64, ..BuildDefinition::default() };
        built_definition.destination = root.join("built").to_string_lossy().into_owned();
        let mut build_engine = Engine::new(registry.clone());
        let cancel = AtomicBool::new(false);
        let built = run(&terrain, &mut build_engine, &project_path, &built_definition, &preferences, None, &cancel, &mut |_| {}).unwrap();

        // The editor's engine holds the node built at the build's resolution.
        let mut exported_definition = built_definition.clone();
        exported_definition.destination = root.join("exported").to_string_lossy().into_owned();
        let mut editor_engine = Engine::new(registry);
        editor_engine.build(&terrain, 1, Resolution::square(64));
        let mut events = Vec::new();
        let exported = export_built(&terrain, &editor_engine, &project_path, &exported_definition, &preferences, None, &mut |event| events.push(event)).unwrap();

        assert!(exported.failures.is_empty(), "{:?}", exported.failures);
        assert_eq!(exported.files, vec![root.join("exported").join("Perlin_Out.png")]);
        assert_eq!(built.files, vec![root.join("built").join("Perlin_Out.png")]);
        assert_eq!(fs::read(&exported.files[0]).unwrap(), fs::read(&built.files[0]).unwrap());
        assert!(matches!(events[0], BuildEvent::Started { nodes: 1, .. }));
        assert!(matches!(events[1], BuildEvent::Exporting { node: 1, index: 1, count: 1 }));
        // Nothing but the images: no report, unlike the build.
        assert!(!root.join("exported").join("report.txt").exists());
        assert!(root.join("built").join("report.txt").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    /// A Perlin (1) feeding a Blur (2) marked for export as `Blur_Out.png`.
    fn chained_terrain() -> Terrain {
        let catalogue = Catalogue::get();
        let perlin = NodeData::new(catalogue.node("Perlin").expect("Perlin is in the catalogue"), 1, 1);
        let mut blur = NodeData::new(catalogue.node("Blur").expect("Blur is in the catalogue"), 2, 1);
        let save = json!({"IsEnabled": true, "Filename": "Blur", "Format": "PNG16"});
        blur.base.insert("SaveDefinition".to_string(), save);
        if let Some(input) = blur.ports.iter_mut().find(|port| port.name == "In") {
            input.record = Some(ConnectionRecord { from: 1, to: 2, from_port: "Out".to_string(), to_port: "In".to_string(), is_valid: true });
        }
        let mut terrain = Terrain { id: String::new(), nodes: Default::default(), width: 5000.0, height: 2500.0, extra: serde_json::Map::new() };
        terrain.nodes.insert(1, perlin);
        terrain.nodes.insert(2, blur);
        terrain
    }

    #[test]
    fn purging_lets_results_go_once_nothing_needs_them() {
        let root = std::env::temp_dir().join(format!("nerthus-purge-{}", std::process::id()));
        let project_path = root.join("Purge.terrain");
        let terrain = chained_terrain();
        let registry = Arc::new(nerthus_nodes::registry());
        let cancel = AtomicBool::new(false);
        let mut kept_files = Vec::new();
        for offload in [false, true] {
            let preferences = ExportPreferences { offload_build_cache: offload, ..ExportPreferences::default() };
            let mut definition = BuildDefinition { resolution: 64, ..BuildDefinition::default() };
            definition.destination = root.join(format!("offload-{offload}")).to_string_lossy().into_owned();
            let mut engine = Engine::new(registry.clone());
            let report = run(&terrain, &mut engine, &project_path, &definition, &preferences, None, &cancel, &mut |_| {}).unwrap();
            assert!(report.failures.is_empty(), "{:?}", report.failures);
            kept_files.push(fs::read(&report.files[0]).unwrap());
            // Both nodes built; purged, neither keeps its results.
            assert_eq!(engine.state(1), NodeState::Built);
            assert_eq!(engine.state(2), NodeState::Built);
            assert_eq!(engine.cache.ports(1).next().is_none(), offload);
            assert_eq!(engine.cache.ports(2).next().is_none(), offload);
        }
        assert_eq!(kept_files[0], kept_files[1]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn subdivisions_offered_like_build_settings() {
        assert!(Subdivision::Faster.available_at(2048));
        assert!(!Subdivision::Balanced.available_at(2048));
        assert!(Subdivision::Balanced.available_at(4096));
        assert!(!Subdivision::Slower.available_at(4096));
        assert!(Subdivision::Slower.available_at(8192));
        assert!(!Subdivision::None.available_at(32768));
    }
}
