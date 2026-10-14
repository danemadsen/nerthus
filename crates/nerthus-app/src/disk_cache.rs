//! Previews cached to disk (`Cache.Offload` and
//! `Cache.Reload`), in the project's `Cache\Primary\<file>_<id>` folder (or
//! the central cache folder): a node's maps written as `<node>_<port>.bin`
//! with a `.meta`, and dropped from memory until a build needs them again,
//! when they are read back and their files deleted.
//!
//! - Smart Cache (Options › Cache): while the engine is idle, every five
//!   seconds, the 2K and 4K previews of nodes nothing has used for the Idle
//!   Threshold go to disk.
//! - Bake › Cache to Disk › When closing file (`BuildDefinition.PersistOnSave`,
//!   or Options' Always Cache to Disk on Close): every built preview is
//!   written as the project closes and read back as it opens
//!   again, so the graph need not build again.
//! - When Idle (until Nerthus closes): every 100 seconds
//!   of idling, the previews built longer ago than the Idle Threshold go
//!   to disk.
//! - Delete Cache deletes the files; Reload SmartCache to RAM
//!   reads every preview on disk back.
//!
//! Baked and bypassed nodes are never cached this way: their maps come
//! from their bakes.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Sender;
use nerthus_engine::{Catalogue, Engine, NodeState, Resolution, StateOverride, Terrain};
use nerthus_export::bake::BakeStore;
use web_time::Instant;

use crate::progress::WorkProgress;

/// Cache to Disk › When Idle, which lasts until Nerthus
/// closes.
static FREEZE_WHEN_IDLE: AtomicBool = AtomicBool::new(false);

pub fn freeze_when_idle() -> bool {
    FREEZE_WHEN_IDLE.load(Ordering::Relaxed)
}

pub fn set_freeze_when_idle(on: bool) {
    FREEZE_WHEN_IDLE.store(on, Ordering::Relaxed);
}

/// The build thread's pause between passive passes (`Shell.passiveTimer`).
pub const PULSE: Duration = Duration::from_secs(5);

/// Passes between When Idle's freezes (`passiveCount > 20`).
const PULSES_PER_FREEZE: u32 = 20;

/// What the build thread needs to cache on its own between builds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiskCacheSettings {
    /// The project's previews folder (none until the project is saved).
    pub store: Option<BakeStore>,
    /// Smart Cache is on and the preview is larger than 1K
    /// (`PassiveTimer_Pulsed`).
    pub smart_cache: bool,
    /// `CacheOptions.NodeIdleThreshold`, in seconds.
    pub idle_threshold: u64,
    /// Cache to Disk › When Idle.
    pub freeze_when_idle: bool,
}

/// What the editor asks the build thread to do with the disk cache.
#[derive(Clone, Debug)]
pub enum DiskCacheTask {
    /// The project closing: every built preview written,
    /// unless its copy on disk is current. `done` hears when it is over.
    Freeze { store: BakeStore, terrain: Arc<Terrain>, done: Option<Sender<()>> },
    /// The project opening: what it left on disk read back.
    Thaw,
    /// The files deleted.
    Delete,
    /// Everything on disk read back.
    Reload,
}

/// The editor's side: tasks waiting for the next request, and whether
/// the project's previews were already written for Nerthus to close.
#[derive(Debug, Default)]
pub struct EditorDiskCache {
    pub tasks: Vec<DiskCacheTask>,
    pub frozen_for_exit: bool,
}

/// The build thread's side: whose maps are on disk instead of in memory,
/// whose copy on disk is the one in memory, and when each node was last
/// built or used.
#[derive(Default)]
pub struct DiskCache {
    offloaded: HashSet<i32>,
    current_on_disk: HashSet<i32>,
    last_access: HashMap<i32, Instant>,
    last_built: HashMap<i32, Instant>,
    pulses: u32,
}

/// Whether a node's previews may go to disk: built as an ordinary node.
fn cacheable(engine: &Engine, terrain: &Terrain, node: i32) -> bool {
    let Some(data) = terrain.nodes.get(&node) else {
        return false;
    };
    StateOverride::of(data) == StateOverride::None && engine.state(node) == NodeState::Built
}

impl DiskCache {
    /// The engine forgot everything (another project, resolution or
    /// region); the files stay.
    pub fn clear(&mut self) {
        self.offloaded.clear();
        self.current_on_disk.clear();
        self.last_access.clear();
        self.last_built.clear();
    }

    /// Nodes a build read (`Cache.LastAccess`).
    pub fn used(&mut self, nodes: &[i32]) {
        let now = Instant::now();
        for node in nodes {
            self.last_access.insert(*node, now);
        }
    }

    /// A node built again (`Node.LastUpdate`): what is on disk for it is
    /// out of date.
    pub fn rebuilt(&mut self, node: i32) {
        let now = Instant::now();
        self.current_on_disk.remove(&node);
        self.last_built.insert(node, now);
        self.last_access.insert(node, now);
    }

    /// After nodes went stale: an offloaded node that no longer counts as
    /// built has its files deleted.
    pub fn forget_stale(&mut self, engine: &Engine, terrain: &Terrain, store: Option<&BakeStore>) {
        let stale: Vec<i32> = self.offloaded.iter().copied().filter(|node| engine.state(*node) != NodeState::Built).collect();
        for node in stale {
            self.offloaded.remove(&node);
            if let (Some(store), Some(data)) = (store, terrain.nodes.get(&node)) {
                store.delete_node(data);
            }
        }
        self.current_on_disk.retain(|node| engine.state(*node) == NodeState::Built);
    }

    /// `Cache.Data`'s reload for a build: the offloaded nodes it reads (its
    /// chain), and the accumulating nodes any build may look up, back in
    /// memory. A node whose files are gone builds again.
    pub fn reload_for(&mut self, engine: &mut Engine, terrain: &Terrain, store: Option<&BakeStore>, chain: &[i32]) {
        if self.offloaded.is_empty() {
            return;
        }
        let catalogue = Catalogue::get();
        let accumulates = |node: i32| {
            let spec = terrain.nodes.get(&node).and_then(|data| catalogue.node(&data.type_name));
            spec.is_some_and(|spec| spec.accumulator.is_some())
        };
        let wanted: Vec<i32> = self.offloaded.iter().copied().filter(|node| chain.contains(node) || accumulates(*node)).collect();
        for node in wanted {
            self.reload(engine, terrain, store, node);
        }
    }

    /// `Cache.Reload` for one node: its maps read back, its files deleted
    /// (no preview file is kept once it is read).
    fn reload(&mut self, engine: &mut Engine, terrain: &Terrain, store: Option<&BakeStore>, node: i32) {
        self.offloaded.remove(&node);
        self.current_on_disk.remove(&node);
        let Some(data) = terrain.nodes.get(&node) else {
            return;
        };
        let maps = store.and_then(|store| store.read_node(data));
        match maps {
            Some(maps) => {
                for (port, map) in maps {
                    engine.cache.store(node, &port, Arc::new(map));
                }
                self.last_access.insert(node, Instant::now());
            }
            None => engine.invalidate(terrain, node),
        }
        if let Some(store) = store {
            store.delete_node(data);
        }
    }

    /// `Cache.Offload` for each of a node's maps: written (unless the copy
    /// on disk is current) and dropped from memory.
    fn offload(&mut self, engine: &mut Engine, terrain: &Terrain, store: &BakeStore, node: i32) -> bool {
        if !self.write(engine, store, node) {
            return false;
        }
        engine.cache.remove_node(node);
        self.offloaded.insert(node);
        let _ = terrain;
        true
    }

    /// A node's maps written to disk, unless the copy there is current
    /// (`IsDifferentFromDisk`).
    fn write(&mut self, engine: &Engine, store: &BakeStore, node: i32) -> bool {
        if self.current_on_disk.contains(&node) {
            return true;
        }
        let maps: Vec<(String, Arc<nerthus_engine::Map>)> = engine.cache.ports(node).map(|(port, map)| (port.clone(), map.clone())).collect();
        if maps.is_empty() {
            return false;
        }
        for (port, map) in maps {
            if let Err(error) = store.write(node, &port, &map) {
                log::error!("Freeze failed. {error}");
                return false;
            }
        }
        self.current_on_disk.insert(node);
        true
    }

    /// The build thread idled for a pulse: Smart Cache's offload, and every
    /// twenty pulses When Idle's freeze.
    pub fn pulse(&mut self, engine: &mut Engine, terrain: &Terrain, settings: &DiskCacheSettings) {
        let Some(store) = settings.store.as_ref() else {
            return;
        };
        let threshold = Duration::from_secs(settings.idle_threshold);
        if settings.smart_cache {
            // Nodes all of whose maps nothing used for the
            // threshold.
            let idle: Vec<i32> = terrain
                .nodes
                .keys()
                .copied()
                .filter(|node| cacheable(engine, terrain, *node) && !self.offloaded.contains(node))
                .filter(|node| self.last_access.get(node).is_none_or(|used| used.elapsed() > threshold))
                .collect();
            if !idle.is_empty() {
                log::info!("Offloading [{}] old caches.", idle.len());
            }
            for node in idle {
                self.offload(engine, terrain, store, node);
            }
        }
        self.pulses += 1;
        if self.pulses > PULSES_PER_FREEZE {
            self.pulses = 0;
            if settings.freeze_when_idle {
                self.freeze_old(engine, terrain, store, settings.idle_threshold);
            }
        }
    }

    /// The nodes built longer ago than the threshold (in whole
    /// minutes, `NodeIdleThreshold / 60`) offloaded.
    fn freeze_old(&mut self, engine: &mut Engine, terrain: &Terrain, store: &BakeStore, idle_threshold: u64) {
        log::info!("Freezing old previews");
        let minutes = Duration::from_secs(idle_threshold / 60 * 60);
        let old: Vec<i32> = terrain
            .nodes
            .keys()
            .copied()
            .filter(|node| cacheable(engine, terrain, *node) && !self.offloaded.contains(node))
            .filter(|node| self.last_built.get(node).is_none_or(|built| built.elapsed() > minutes))
            .collect();
        for node in old {
            self.offload(engine, terrain, store, node);
        }
    }

    /// A project closing: every built preview written, reporting how far
    /// it got and each node it writes.
    pub fn freeze_all(&mut self, engine: &Engine, terrain: &Terrain, store: &BakeStore, report: &mut dyn FnMut(WorkProgress)) {
        log::info!("Caching previews");
        let built: Vec<i32> = terrain.nodes.keys().copied().filter(|node| cacheable(engine, terrain, *node)).collect();
        let count = built.len();
        for (index, node) in built.into_iter().enumerate() {
            report(WorkProgress::Fraction(index as f32 / count as f32));
            if self.offloaded.contains(&node) || self.current_on_disk.contains(&node) {
                continue;
            }
            let name = terrain.nodes.get(&node).map(|data| data.name.clone()).unwrap_or_default();
            report(WorkProgress::Text(format!("Freezing {name}...")));
            self.write(engine, store, node);
        }
    }

    /// Every unbaked node whose maps the project left on disk,
    /// at the preview's size, read back and taken as built (reporting how
    /// far it got). Returns them.
    pub fn thaw(&mut self, engine: &mut Engine, terrain: &Terrain, store: &BakeStore, resolution: Resolution, report: &mut dyn FnMut(WorkProgress)) -> Vec<i32> {
        if !store.folder().is_dir() {
            return Vec::new();
        }
        log::info!("Loading Frozen Cache...");
        let mut thawed = Vec::new();
        let count = terrain.nodes.len();
        for (index, node) in terrain.nodes.values().enumerate() {
            report(WorkProgress::Fraction(index as f32 / count as f32));
            if StateOverride::of(node) != StateOverride::None || engine.state(node.id) == NodeState::Built {
                continue;
            }
            let Some(maps) = store.read_node(node) else {
                continue;
            };
            let fits = maps.iter().all(|(_, map)| map.width() == resolution.x && map.height() == resolution.y);
            store.delete_node(node);
            if !fits || maps.is_empty() {
                continue;
            }
            for (port, map) in maps {
                engine.cache.store(node.id, &port, Arc::new(map));
            }
            engine.mark_built(node.id);
            self.last_access.insert(node.id, Instant::now());
            self.last_built.insert(node.id, Instant::now());
            thawed.push(node.id);
        }
        thawed
    }

    /// Every node's files deleted; nodes whose maps were only on
    /// disk build again. False when the project has no such folder.
    pub fn delete(&mut self, engine: &mut Engine, terrain: &Terrain, store: &BakeStore) -> bool {
        let existed = store.folder().is_dir();
        for node in terrain.nodes.values() {
            if StateOverride::of(node) == StateOverride::None {
                store.delete_node(node);
            }
        }
        let lost: Vec<i32> = self.offloaded.drain().collect();
        for node in lost {
            engine.invalidate(terrain, node);
        }
        self.current_on_disk.clear();
        if existed {
            let _ = remove_empty_folder(store.folder());
        }
        existed
    }

    /// Every offloaded node read back.
    pub fn reload_all(&mut self, engine: &mut Engine, terrain: &Terrain, store: Option<&BakeStore>) {
        let offloaded: Vec<i32> = self.offloaded.iter().copied().collect();
        for node in offloaded {
            self.reload(engine, terrain, store, node);
        }
    }
}

/// The folder, if nothing is left in it.
fn remove_empty_folder(folder: &Path) -> std::io::Result<()> {
    if std::fs::read_dir(folder)?.next().is_none() {
        std::fs::remove_dir(folder)?;
    }
    Ok(())
}

/// A request's freezes (for projects that closed), before the
/// engine forgets their previews; the progress window hears how they go
/// and when they are done.
pub fn freeze_closing(disk: &mut DiskCache, engine: &Engine, tasks: &[DiskCacheTask], events: &Sender<crate::worker::BuildEvent>) {
    let mut report = |progress: WorkProgress| {
        let _ = events.send(crate::worker::BuildEvent::Progress(progress));
    };
    let mut froze = false;
    for task in tasks {
        if let DiskCacheTask::Freeze { store, terrain, done } = task {
            disk.freeze_all(engine, terrain, store, &mut report);
            froze = true;
            if let Some(done) = done {
                let _ = done.send(());
            }
        }
    }
    if froze {
        report(WorkProgress::Frozen);
    }
}

/// A request's other tasks, for its own project: thawed nodes reported as
/// built, Delete Cache's outcome said.
pub fn run_tasks(
    disk: &mut DiskCache,
    engine: &mut Engine,
    terrain: &Terrain,
    resolution: Resolution,
    settings: &DiskCacheSettings,
    tasks: Vec<DiskCacheTask>,
    events: &Sender<crate::worker::BuildEvent>,
) {
    for task in tasks {
        match task {
            DiskCacheTask::Freeze { .. } => {}
            DiskCacheTask::Thaw => {
                let Some(store) = settings.store.as_ref() else {
                    continue;
                };
                let mut report = |progress: WorkProgress| {
                    let _ = events.send(crate::worker::BuildEvent::Progress(progress));
                };
                for node in disk.thaw(engine, terrain, store, resolution, &mut report) {
                    let range = engine.cache.get(node, "Out").filter(|map| map.channel_count() == 1).map(|map| nerthus_nodes::maths::min_max(map.data()));
                    let finished = crate::worker::BuildEvent::NodeFinished { node, milliseconds: 0.0, state: NodeState::Built, error: None, range };
                    let _ = events.send(finished);
                }
            }
            DiskCacheTask::Delete => {
                let deleted = settings.store.as_ref().is_some_and(|store| disk.delete(engine, terrain, store));
                let message = if deleted { "Passive cache deleted." } else { "Passive cache was not found." };
                let _ = events.send(crate::worker::BuildEvent::DiskCache(message.to_string()));
            }
            DiskCacheTask::Reload => disk.reload_all(engine, terrain, settings.store.as_ref()),
        }
    }
}

impl crate::editor::Editor {
    /// Where the project's previews go on disk, once it is saved.
    pub fn preview_store(&self) -> Option<BakeStore> {
        let path = self.document.path.as_ref()?;
        let central = self.user.preferences.central_preview_cache();
        Some(BakeStore::previews(path, &self.document.project_id(), central.as_deref()))
    }

    /// What the build thread caches on its own (`PassiveTimer_Pulsed`:
    /// Smart Cache only for a saved project previewed larger than 1K).
    pub fn disk_cache_settings(&self) -> DiskCacheSettings {
        let (smart_cache, _, idle_threshold) = self.user.preferences.cache_options();
        let store = self.preview_store();
        let saved = self.document.path.as_ref().is_some_and(|path| path.is_file());
        DiskCacheSettings {
            store,
            smart_cache: smart_cache && saved && self.preview_resolution > 1024,
            idle_threshold,
            freeze_when_idle: freeze_when_idle(),
        }
    }

    /// Bake › Cache to Disk › When closing file (`BuildDefinition.PersistOnSave`).
    pub fn persist_on_save(&self) -> bool {
        let definition = self.document.project.assets[0].extra.get("BuildDefinition");
        definition.and_then(|definition| definition.get("PersistOnSave")).and_then(serde_json::Value::as_bool).unwrap_or(false)
    }

    /// `Shell.CloseFile`'s freeze, for the project about to close: its
    /// previews and where they go, when the project or the options ask
    /// for it and the project is saved without changes since (the
    /// previews are those of the saved project).
    pub fn closing_freeze(&self) -> Option<(BakeStore, Arc<Terrain>)> {
        let (_, always, _) = self.user.preferences.cache_options();
        if !(always || self.persist_on_save()) {
            return None;
        }
        let saved = self.document.path.as_ref().is_some_and(|path| path.is_file());
        if !saved || self.document.dirty {
            return None;
        }
        let store = self.preview_store()?;
        Some((store, Arc::new(self.document.terrain().clone())))
    }

    /// The project is closing (another opens, or a new one starts): its
    /// previews go to disk first if asked.
    pub fn freeze_on_close(&mut self) {
        if let Some((store, terrain)) = self.closing_freeze() {
            self.disk_cache.tasks.push(DiskCacheTask::Freeze { store, terrain, done: None });
        }
    }

    /// The project just opened: what it left on disk is read back
    /// (started after every open).
    pub fn thaw_on_open(&mut self) {
        if self.document.path.is_some() {
            self.disk_cache.tasks.push(DiskCacheTask::Thaw);
        }
    }
}

/// Nerthus is closing (`Shell.CloseFile` as the window closes): the
/// project's previews written before it goes, waiting up to a minute for
/// the build thread.
pub fn freeze_on_exit(mut exits: bevy::prelude::MessageReader<bevy::prelude::AppExit>, mut editor: bevy::prelude::ResMut<crate::editor::Editor>, worker: bevy::prelude::Res<crate::worker::BuildWorker>) {
    if exits.read().next().is_none() {
        return;
    }
    // Exit already wrote them, with its progress window.
    if editor.disk_cache.frozen_for_exit {
        return;
    }
    let Some((store, terrain)) = editor.closing_freeze() else {
        return;
    };
    let (done, finished) = crossbeam_channel::bounded(1);
    editor.disk_cache.tasks.push(DiskCacheTask::Freeze { store, terrain, done: Some(done) });
    let suspended = !editor.engine_active;
    editor.engine_active = true;
    editor.request_build(&worker);
    editor.engine_active = !suspended;
    let _ = finished.recv_timeout(Duration::from_secs(60));
}
