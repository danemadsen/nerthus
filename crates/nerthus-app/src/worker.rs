//! Background builds. One thread owns the engine (and its cached
//! results); the editor sends it the node to preview and the nodes edits
//! made stale, and hears back per node and with the finished maps. A newer
//! request stops the running build between nodes, as does a cancel (Esc,
//! or suspending the engine).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use bevy::prelude::Resource;
use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};
use nerthus_engine::automation::Automation;
use nerthus_engine::macros::MacroLibrary;
use nerthus_engine::{BuildResult, Catalogue, Engine, Map, NodeState, PortIntents, Region, RenderIntent, Resolution, StateOverride, Terrain};
use nerthus_export::bake::{self, BakeFit, BakeStore};
use serde_json::Value;

use crate::disk_cache::{self, DiskCache, DiskCacheSettings, DiskCacheTask};

/// What to build.
pub struct BuildRequest {
    pub generation: u64,
    pub terrain: Arc<Terrain>,
    pub automation: Automation,
    pub project_path: Option<PathBuf>,
    pub target: i32,
    pub resolution: Resolution,
    /// Nodes whose results are stale (they and everything downstream are
    /// rebuilt).
    pub stale: Vec<i32>,
    /// Forget every result (another project, another resolution).
    pub clear: bool,
    /// A new project's macros, when they change.
    pub macros: Option<MacroLibrary>,
    /// `TerrainState.UnderlayNode`: the heightfield colour and mask nodes
    /// fall back on (0 for none).
    pub underlay: i32,
    /// Where the project's baked maps are (none until it is saved).
    pub bakes: Option<BakeStore>,
    /// Nodes to bake before building (Graph › Baking).
    pub bake: Option<BakeJob>,
    /// Nodes just unbaked: they keep their maps as ordinary results and
    /// their files go.
    pub unbake: Vec<i32>,
    /// Force load baked cache: every bake loaded, not only the ones an
    /// unbaked node reads.
    pub load_all_bakes: bool,
    /// The region the editor previews (`CurrentPortion.Region`): the bakes
    /// are cut to it.
    pub region: Option<Region>,
    /// Nodes whose kept data goes (Purge Cache: a File node's loaded
    /// image, `CacheCollection.Remove("File")`).
    pub forget: Vec<i32>,
    /// Node commands that work from the node's inputs, run once the build
    /// is done (Fit Curve, Synthesize Colors).
    pub node_commands: Vec<NodeCommand>,
    /// Where previews go on disk and what the build thread does with them
    /// while it idles (Smart Cache, Cache to Disk › When Idle).
    pub disk_cache: DiskCacheSettings,
    /// What the editor asked of the disk cache: a closing project's
    /// previews written, the opening one's read back, Delete Cache, Reload
    /// SmartCache to RAM.
    pub disk_cache_tasks: Vec<DiskCacheTask>,
    /// Nodes whose maps the editor shows besides the target's (floating 2D
    /// views): each is reported when a build leaves it built anew, or every
    /// one now when `watched_refresh` is set.
    pub watched: Vec<i32>,
    pub watched_refresh: bool,
}

/// A node command the build thread runs: the node and the command
/// parameter's name (`FitCurve`, `SynthColors`).
#[derive(Clone, Debug, PartialEq)]
pub struct NodeCommand {
    pub node: i32,
    pub name: String,
}

/// A node's maps as a build left them, for a view of it.
#[derive(Clone)]
pub struct WatchedNode {
    pub node: i32,
    pub outputs: Vec<(String, Arc<Map>)>,
    pub underlay: Option<Arc<Map>>,
    pub custom_underlay: Option<Arc<Map>>,
    pub custom_colours: Option<Arc<Map>>,
    pub port_intents: PortIntents,
}

/// What to bake: each node (and everything upstream of it) in turn, at
/// `resolution`.
#[derive(Clone, Debug, PartialEq)]
pub struct BakeJob {
    pub nodes: Vec<i32>,
    pub resolution: usize,
}

/// What the engine keeps of a node (the Stats tab's readouts).
#[derive(Clone, Debug, Default)]
pub struct NodeCache {
    /// Each cached port: its name, width, height, channels and intent.
    pub ports: Vec<CachedPort>,
    /// The bytes its maps hold, each map once.
    pub bytes: usize,
    /// The intents its outputs were left with (`Port.RenderIntent`).
    pub port_intents: PortIntents,
}

impl NodeCache {
    /// The intent its main output was left with (`Port.RenderIntent` of
    /// `Out`), which `Node.IsHeightfield` reads.
    pub fn out_intent(&self) -> Option<RenderIntent> {
        self.port_intents.get("Out").copied()
    }
}

#[derive(Clone, Debug)]
pub struct CachedPort {
    pub name: String,
    pub width: usize,
    pub height: usize,
    pub channels: usize,
    pub intent: RenderIntent,
}

/// What the worker reports.
pub enum BuildEvent {
    NodeStarted(i32),
    NodeFinished {
        node: i32,
        milliseconds: f32,
        state: NodeState,
        error: Option<String>,
        /// The lowest and highest value of a one-channel main output.
        range: Option<(f32, f32)>,
    },
    /// Members builds changed, to save with the project.
    MemberUpdates(Vec<(i32, String, Value)>),
    Finished {
        generation: u64,
        target: i32,
        /// The target's maps by port.
        outputs: Vec<(String, Arc<Map>)>,
        /// The heightfield colour and mask outputs drape over.
        underlay: Option<Arc<Map>>,
        /// The map the target is shown over when its type has its own
        /// underlay (`CustomUnderlayAttribute`); `None` otherwise or when
        /// that map is missing.
        custom_underlay: Option<Arc<Map>>,
        /// `Node.GetCustomColorMap()` when the target's type has its own
        /// colours (`CustomColorizationAttribute`); `None` otherwise or
        /// when it is null.
        custom_colours: Option<Arc<Map>>,
        /// The intents the target's output ports were left with
        /// (`Port.RenderIntent`); empty when it was never built.
        port_intents: PortIntents,
        error: Option<String>,
        /// What the engine's cached maps hold, in bytes.
        cache_bytes: usize,
        /// What it keeps of each node.
        node_caches: HashMap<i32, NodeCache>,
    },
    /// A newer request took over before the build finished.
    Superseded,
    /// The build was cancelled before it finished.
    Cancelled { generation: u64 },
    /// A bake finished: the nodes now baked (to mark `Baked` and save), or
    /// why it failed.
    Baked { nodes: Vec<i32>, error: Option<String> },
    /// A node command's result: the property it sets and the value, for the
    /// editor to apply as an edit (nothing when the node's input is not
    /// there).
    NodeCommandDone { node: i32, name: String, property: String, value: Value },
    /// A watched node's maps (`BuildRequest::watched`).
    Watched(WatchedNode),
    /// What the disk cache has to say (Delete Cache's outcome).
    DiskCache(String),
}

#[derive(Resource)]
pub struct BuildWorker {
    requests: Sender<BuildRequest>,
    events: Receiver<BuildEvent>,
    /// Set to stop the running build at the next node.
    cancel: Arc<AtomicBool>,
}

impl BuildWorker {
    pub fn start() -> Self {
        let (request_sender, request_receiver) = unbounded::<BuildRequest>();
        let (event_sender, event_receiver) = unbounded::<BuildEvent>();
        let cancel = Arc::new(AtomicBool::new(false));
        let thread_cancel = cancel.clone();
        let spawned = thread::Builder::new().name("nerthus-build".to_string()).spawn(move || {
            run(request_receiver, event_sender, thread_cancel);
        });
        if let Err(error) = spawned {
            log::error!("cannot start the build thread: {error}");
        }
        Self {
            requests: request_sender,
            events: event_receiver,
            cancel,
        }
    }

    /// Stops the running build before its next node (it then says
    /// `Cancelled`); nothing when no build runs.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn request(&self, request: BuildRequest) {
        let _ = self.requests.send(request);
    }

    pub fn poll(&self) -> Vec<BuildEvent> {
        self.events.try_iter().collect()
    }
}

/// Folds requests that queued up into the newest, keeping every stale node
/// and clear of the ones it replaces.
fn newest(mut request: BuildRequest, requests: &Receiver<BuildRequest>) -> BuildRequest {
    while let Ok(mut newer) = requests.try_recv() {
        newer.stale.append(&mut request.stale);
        newer.clear |= request.clear;
        if newer.macros.is_none() {
            newer.macros = request.macros.take();
        }
        if newer.bake.is_none() {
            newer.bake = request.bake.take();
        }
        let mut unbake = std::mem::take(&mut request.unbake);
        unbake.append(&mut newer.unbake);
        newer.unbake = unbake;
        newer.load_all_bakes |= request.load_all_bakes;
        newer.forget.append(&mut request.forget);
        newer.watched_refresh |= request.watched_refresh;
        let mut node_commands = std::mem::take(&mut request.node_commands);
        node_commands.append(&mut newer.node_commands);
        newer.node_commands = node_commands;
        let mut disk_cache_tasks = std::mem::take(&mut request.disk_cache_tasks);
        disk_cache_tasks.append(&mut newer.disk_cache_tasks);
        newer.disk_cache_tasks = disk_cache_tasks;
        request = newer;
    }
    request
}

fn run(requests: Receiver<BuildRequest>, events: Sender<BuildEvent>, cancel: Arc<AtomicBool>) {
    let registry = Arc::new(nerthus_nodes::registry());
    let mut engine = Engine::new(registry);
    let mut resolution = Resolution::default();
    let mut region: Option<Region> = None;
    let mut carried: Option<BuildRequest> = None;
    // The disk cache, and the project and settings it idles with.
    let mut disk = DiskCache::default();
    let mut idle_terrain: Option<Arc<Terrain>> = None;
    let mut idle_settings = DiskCacheSettings::default();
    loop {
        let request = match carried.take() {
            Some(request) => request,
            None => match requests.recv_timeout(disk_cache::PULSE) {
                Ok(request) => request,
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(terrain) = &idle_terrain {
                        disk.pulse(&mut engine, terrain, &idle_settings);
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            },
        };
        let mut request = newest(request, &requests);
        // A cancel is for the build running when it was asked.
        cancel.store(false, Ordering::Relaxed);
        // A closing project's previews go to disk before anything forgets
        // them.
        let disk_tasks = std::mem::take(&mut request.disk_cache_tasks);
        disk_cache::freeze_closing(&mut disk, &engine, &disk_tasks);

        if let Some(macros) = request.macros.take() {
            engine.macros = macros;
        }
        // Another underlay changes what colour and mask nodes fall back on;
        // another region, what the bakes give.
        let underlay_changed = engine.underlay != request.underlay;
        let region_changed = region != request.region;
        let cleared = request.clear || request.resolution != resolution || underlay_changed || region_changed;
        if cleared {
            engine.clear();
            disk.clear();
            resolution = request.resolution;
            engine.underlay = request.underlay;
            region = request.region.clone();
        }
        for node in request.forget.drain(..) {
            engine.forget_memory(node);
        }
        for node in &request.stale {
            engine.invalidate(&request.terrain, *node);
        }
        disk.forget_stale(&engine, &request.terrain, request.disk_cache.store.as_ref());
        engine.automation = request.automation.clone();
        engine.project_path = request.project_path.clone();

        let mut terrain = request.terrain.clone();
        let bakes_changed = run_bakes(&mut engine, &mut terrain, &mut request, resolution, &events);
        if let Some(store) = &request.bakes
            && (cleared || bakes_changed || request.load_all_bakes)
        {
            load_editor_bakes(&mut engine, &terrain, store, resolution, region.as_ref(), request.load_all_bakes);
        }
        disk_cache::run_tasks(&mut disk, &mut engine, &terrain, resolution, &request.disk_cache, disk_tasks, &events);
        let order = Engine::build_order(&terrain, request.target);
        let mut read = order.clone();
        if request.underlay != 0 {
            read.extend(Engine::build_order(&terrain, request.underlay));
        }
        disk.reload_for(&mut engine, &terrain, request.disk_cache.store.as_ref(), &read);
        idle_terrain = Some(terrain.clone());
        idle_settings = request.disk_cache.clone();
        let mut superseded = false;
        let mut cancelled = false;
        let mut built_now = std::collections::HashSet::new();
        for node in order {
            if engine.state(node) == NodeState::Built {
                continue;
            }
            if !requests.is_empty() {
                superseded = true;
                break;
            }
            if cancel.swap(false, Ordering::Relaxed) {
                cancelled = true;
                break;
            }
            let _ = events.send(BuildEvent::NodeStarted(node));
            let started = Instant::now();
            let result = engine.build(&terrain, node, resolution);
            disk.rebuilt(node);
            built_now.insert(node);
            let milliseconds = started.elapsed().as_secs_f32() * 1000.0;
            let error = match result {
                BuildResult::Failed(error) => Some(error.to_string()),
                _ => engine.error(node).map(ToString::to_string),
            };
            let state = engine.state(node);
            let range = main_range(&engine, node);
            let finished = BuildEvent::NodeFinished { node, milliseconds, state, error, range };
            let _ = events.send(finished);
        }

        // The node is asked for its colours as it renders, as its build
        // left it (member changes not yet handed back).
        let custom_colours = if superseded || cancelled { None } else { target_custom_colours(&engine, &terrain, request.target, resolution) };
        let updates = engine.take_member_updates();
        if !updates.is_empty() {
            let _ = events.send(BuildEvent::MemberUpdates(updates));
        }
        if cancelled {
            log::info!("Build cancelled.");
            let _ = events.send(BuildEvent::Cancelled { generation: request.generation });
            continue;
        }
        if superseded {
            let _ = events.send(BuildEvent::Superseded);
            // Its stale nodes are already invalidated; the next request
            // builds from here.
            request.stale.clear();
            request.clear = false;
            carried = requests.recv().ok();
            if carried.is_none() {
                return;
            }
            continue;
        }

        for command in std::mem::take(&mut request.node_commands) {
            if let Some(done) = run_node_command(&mut engine, &terrain, &command, resolution) {
                let _ = events.send(done);
            }
        }
        disk.used(&read);

        let target = request.target;
        let mut outputs: Vec<(String, Arc<Map>)> = engine.cache.ports(target).map(|(port, map)| (port.clone(), map.clone())).collect();
        outputs.sort_by_key(|(port, _)| port != "Out");
        let error = engine.error(target).map(ToString::to_string);
        let underlay = engine.underlay_heightfield(&terrain, target, request.resolution);
        let custom_underlay = engine.custom_underlay(&terrain, target, request.resolution);
        let port_intents = engine.port_intents(target).cloned().unwrap_or_default();
        let cache_bytes = engine.cache.bytes();
        let node_caches = node_caches(&engine);
        let finished = BuildEvent::Finished {
            generation: request.generation,
            target,
            outputs,
            underlay,
            custom_underlay,
            custom_colours,
            port_intents,
            error,
            cache_bytes,
            node_caches,
        };
        let _ = events.send(finished);
        for node in std::mem::take(&mut request.watched) {
            let rebuilt = built_now.contains(&node);
            if !(rebuilt || request.watched_refresh) {
                continue;
            }
            if let Some(watched) = watched_node(&engine, &terrain, node, resolution) {
                let _ = events.send(BuildEvent::Watched(watched));
            }
        }
    }
}

/// The request's unbakes and bake, before its build. Unbaked nodes keep
/// their maps (loaded from disk first if they were left there) as
/// ordinary results and their files are deleted; a bake
/// builds on its own engine at the bake resolution, starting from the
/// editor's results when they are at that resolution, and the nodes it
/// baked are marked in `terrain` so the bakes load below. True when
/// anything changed.
fn run_bakes(engine: &mut Engine, terrain: &mut Arc<Terrain>, request: &mut BuildRequest, resolution: Resolution, events: &Sender<BuildEvent>) -> bool {
    let Some(store) = request.bakes.clone() else {
        if request.bake.take().is_some() {
            let error = Some("You must save this file to bake it.".to_string());
            let _ = events.send(BuildEvent::Baked { nodes: Vec::new(), error });
        }
        return false;
    };
    let mut changed = false;
    for id in std::mem::take(&mut request.unbake) {
        let Some(node) = terrain.nodes.get(&id) else {
            continue;
        };
        let has_maps = engine.cache.ports(id).next().is_some();
        if engine.is_baked(id)
            && !has_maps
            && let Some(maps) = store.read_node(node)
        {
            let fit = editor_fit(resolution, request.region.as_ref());
            let fitted = maps.into_iter().filter_map(|(port, map)| bake::fit_baked(&map, fit).map(|fitted| (port, fitted))).collect();
            engine.load_baked(id, fitted);
        }
        engine.unbake(id);
        store.delete_node(node);
        changed = true;
    }

    let Some(job) = request.bake.take() else {
        return changed;
    };
    let mut bake_engine = Engine::new(engine.registry());
    bake_engine.automation = request.automation.clone();
    bake_engine.macros = engine.macros.clone();
    bake_engine.project_path = request.project_path.clone();
    let bake_resolution = Resolution::square(job.resolution);
    // Results cut to a previewed region are no part of a bake.
    let same_resolution = bake_resolution == resolution && request.region.is_none();
    let editor_engine: &Engine = engine;
    let built = |id: i32| -> Option<Vec<(String, Arc<Map>)>> {
        if !same_resolution || editor_engine.state(id) != NodeState::Built {
            return None;
        }
        Some(editor_engine.cache.ports(id).map(|(port, map)| (port.clone(), map.clone())).collect())
    };
    let mut baked_nodes = Vec::new();
    let mut working = (**terrain).clone();
    let mut error = None;
    for node in &job.nodes {
        match bake::bake_node(&mut bake_engine, &working, *node, bake_resolution, &store, &built) {
            Ok(baked) => {
                for id in baked {
                    if let Some(data) = working.nodes.get_mut(&id) {
                        data.set_member("StateOverride", Value::from("Baked"));
                    }
                    if !baked_nodes.contains(&id) {
                        baked_nodes.push(id);
                    }
                }
            }
            Err(failure) => {
                error = Some(failure.to_string());
                break;
            }
        }
    }
    if error.is_some() {
        // Nothing is marked when a bake fails; the files written stay
        // unused.
        baked_nodes.clear();
    }
    for id in &baked_nodes {
        engine.cache.remove_node(*id);
        engine.unbake(*id);
        engine.invalidate(&working, *id);
    }
    if error.is_none() {
        *terrain = Arc::new(working);
    }
    let changed_now = !baked_nodes.is_empty();
    let _ = events.send(BuildEvent::Baked { nodes: baked_nodes, error });
    changed || changed_now
}

/// How the editor fits a bake: resized to the preview
/// resolution (bilinear), cut to the previewed region first if there is
/// one.
fn editor_fit(resolution: Resolution, region: Option<&Region>) -> BakeFit<'_> {
    match region {
        Some(region) => BakeFit::Region { region: Some(region), resolution, high_quality: false },
        None => BakeFit::Whole { resolution, high_quality: false },
    }
}

/// Bake loading in the editor: the bakes an unbaked node reads (every
/// bake with `all`) loaded at the preview resolution (bilinear, as the
/// editor fits them, cut to the previewed region), the rest left on disk;
/// what reads them is built again.
fn load_editor_bakes(engine: &mut Engine, terrain: &Terrain, store: &BakeStore, resolution: Resolution, region: Option<&Region>, all: bool) {
    let fit = editor_fit(resolution, region);
    let loaded = bake::load_bakes(engine, terrain, store, fit, all, false);
    for id in loaded {
        engine.invalidate(terrain, id);
    }
    // Nodes no longer baked in the project are no longer baked here.
    let unbaked: Vec<i32> = terrain.nodes.values().filter(|node| StateOverride::of(node) != StateOverride::Baked && engine.is_baked(node.id)).map(|node| node.id).collect();
    for id in unbaked {
        engine.unbake(id);
    }
}

/// A built node's maps and what the viewport needs to render them.
fn watched_node(engine: &Engine, terrain: &Terrain, node: i32, resolution: Resolution) -> Option<WatchedNode> {
    if !matches!(engine.state(node), NodeState::Built | NodeState::Baked) {
        return None;
    }
    let mut outputs: Vec<(String, Arc<Map>)> = engine.cache.ports(node).map(|(port, map)| (port.clone(), map.clone())).collect();
    outputs.sort_by_key(|(port, _)| port != "Out");
    let watched = WatchedNode {
        node,
        outputs,
        underlay: engine.underlay_heightfield(terrain, node, resolution),
        custom_underlay: engine.custom_underlay(terrain, node, resolution),
        custom_colours: target_custom_colours(engine, terrain, node, resolution),
        port_intents: engine.port_intents(node).cloned().unwrap_or_default(),
    };
    Some(watched)
}

/// `RendererBase.ResolveColorfield`'s `Node.GetCustomColorMap()` for a
/// built target whose type has `CustomColorizationAttribute`.
fn target_custom_colours(engine: &Engine, terrain: &Terrain, target: i32, resolution: Resolution) -> Option<Arc<Map>> {
    let node = terrain.nodes.get(&target)?;
    let spec = Catalogue::get().node(&node.type_name)?;
    let built = matches!(engine.state(target), NodeState::Built | NodeState::Baked);
    if !spec.has_flag("CustomColorization") || !built {
        return None;
    }
    let colours = engine.custom_colour_map(terrain, target, resolution)?;
    Some(Arc::new(colours))
}

/// The value range of a node's main output when it has one channel (the Stats
/// tab's Min, Max and Range).
fn main_range(engine: &Engine, node: i32) -> Option<(f32, f32)> {
    let main = engine.cache.get(node, "Out")?;
    if main.channel_count() != 1 {
        return None;
    }
    Some(nerthus_nodes::maths::min_max(main.data()))
}

/// Runs a node command against the node's input as the builds left it,
/// building what feeds the node first (`In.GetMap()` reads the cache):
/// Curve's Fit Curve (`Curve.ExecuteFitCurve`) and Synth's Synthesize
/// Colors (`Synth.ExtractSamples`).
fn run_node_command(engine: &mut Engine, terrain: &Terrain, command: &NodeCommand, resolution: Resolution) -> Option<BuildEvent> {
    let node = terrain.nodes.get(&command.node)?;
    let record = node.port("In")?.record.clone()?;
    for upstream in Engine::build_order(terrain, record.from) {
        if engine.state(upstream) != NodeState::Built {
            engine.build(terrain, upstream, resolution);
        }
    }
    let input = engine.cache.get(record.from, &record.from_port)?;
    let float = |value: f32| nerthus_engine::json::f32_value(value);
    let (property, value) = match (node.type_name.as_str(), command.name.as_str()) {
        ("Curve", "FitCurve") => {
            let points = nerthus_nodes::nodes::fit_curve(&input);
            let list: Vec<Value> = points.iter().map(|[x, y]| serde_json::json!({"X": float(*x), "Y": float(*y)})).collect();
            ("CurveSpline", Value::Array(list))
        }
        ("Synth", "SynthColors") => {
            let samples_name = node.values.get("Samples").and_then(Value::as_str).unwrap_or("_64");
            let samples = nerthus_nodes::nodes::synth_sample_count(samples_name)?;
            let number = |name: &str| node.values.get(name).and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let seed = node.values.get("Seed").and_then(Value::as_i64).unwrap_or(0) as i32;
            let colours = nerthus_nodes::nodes::synth_samples(&input, samples, number("Fuzzy"), number("Offset"), seed);
            let list: Vec<Value> = colours.iter().map(|[x, y, z, w]| serde_json::json!({"X": float(*x), "Y": float(*y), "Z": float(*z), "W": float(*w)})).collect();
            ("Data", Value::Array(list))
        }
        _ => return None,
    };
    let done = BuildEvent::NodeCommandDone {
        node: command.node,
        name: command.name.clone(),
        property: property.to_string(),
        value,
    };
    Some(done)
}

fn node_caches(engine: &Engine) -> HashMap<i32, NodeCache> {
    let mut caches = HashMap::new();
    for node in engine.cache.nodes() {
        let port_intents = engine.port_intents(node).cloned().unwrap_or_default();
        let mut cache = NodeCache { port_intents, ..NodeCache::default() };
        let mut seen = std::collections::HashSet::new();
        for (name, map) in engine.cache.ports(node) {
            let port = CachedPort {
                name: name.clone(),
                width: map.width(),
                height: map.height(),
                channels: map.channel_count(),
                intent: map.info.render_intent,
            };
            cache.ports.push(port);
            if seen.insert(Arc::as_ptr(map)) {
                cache.bytes += map.byte_size();
            }
        }
        caches.insert(node, cache);
    }
    caches
}
