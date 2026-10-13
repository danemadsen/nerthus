//! Build-time context shared by every node evaluation (with the bits of
//! `BuildService` that nodes read).

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;

/// `CacheLevel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CacheLevel {
    Preview,
    Intermediate,
    Final,
}

/// Physical definition of the terrain (`Definition`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainDefinition {
    /// Maximum height in metres.
    pub height: f32,
    /// Terrain width in metres.
    pub scale: f32,
}

impl Default for TerrainDefinition {
    fn default() -> Self {
        Self {
            height: 2600.0,
            scale: 5000.0,
        }
    }
}

/// Tiled-build information. Non-tiled builds use `TileInfo::single`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileInfo {
    pub is_tiled: bool,
    /// Tile index (x, y).
    pub position: (i32, i32),
    /// Tile origin in pixels of the full world.
    pub position_pixels: (i32, i32),
    /// Resolution of the whole world — noise frequency is relative to it.
    pub world_size: usize,
    pub tile_count: i32,
}

impl TileInfo {
    pub fn single(size: usize) -> Self {
        Self {
            is_tiled: false,
            position: (0, 0),
            position_pixels: (0, 0),
            world_size: size,
            tile_count: 1,
        }
    }
}

pub type ProgressFn = dyn Fn(f64) + Send + Sync;

/// Project-level data nodes can read (painted masks, imported images, files).
pub trait ResourceProvider: Send + Sync {
    /// Encoded image bytes of a mask/image resource, by resource id or owning node id.
    fn image_bytes(&self, id: uuid::Uuid) -> Option<std::sync::Arc<Vec<u8>>>;
    /// Raw float data of a mesh/array resource.
    fn floats(&self, id: uuid::Uuid) -> Option<std::sync::Arc<Vec<f32>>>;
    /// Folder relative file paths resolve against (the project's folder).
    fn base_dir(&self) -> Option<std::path::PathBuf>;
}

#[derive(Clone)]
pub struct BuildContext {
    pub size: usize,
    pub level: CacheLevel,
    pub soft_update: bool,
    pub tile: TileInfo,
    pub definition: TerrainDefinition,
    /// True when running headless builds ("BuildSwarm" mode).
    pub is_build_mode: bool,
    pub cancel: Arc<AtomicBool>,
    pub progress: Option<Arc<ProgressFn>>,
    pub resources: Option<Arc<dyn ResourceProvider>>,
    /// Project build range (`Build.Range.Min/Max`), used by remapping nodes.
    pub build_range: (f64, f64),
}

impl BuildContext {
    pub fn new(size: usize) -> Self {
        Self {
            size,
            level: CacheLevel::Preview,
            soft_update: false,
            tile: TileInfo::single(size),
            definition: TerrainDefinition::default(),
            is_build_mode: false,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: None,
            resources: None,
            build_range: (0.0, 1.0),
        }
    }

    /// Resolves a possibly relative path against the project folder.
    pub fn resolve_path(&self, path: &str) -> std::path::PathBuf {
        let candidate = std::path::PathBuf::from(path.replace('\\', "/"));
        if candidate.is_absolute() {
            return candidate;
        }
        match self.resources.as_ref().and_then(|resources| resources.base_dir()) {
            Some(base) => base.join(candidate),
            None => candidate,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn report_progress(&self, fraction: f64) {
        if let Some(progress) = &self.progress {
            progress(fraction);
        }
    }

    /// Resolution noise frequencies are normalised against (`IsTiledBuild ? LC : size`).
    pub fn noise_base_size(&self) -> usize {
        if self.tile.is_tiled { self.tile.world_size } else { self.size }
    }

    /// Ratio used everywhere the nodes scale by `size / 512`.
    pub fn scale_512(&self) -> f32 {
        self.size as f32 / 512.0
    }

    pub fn pixel_count(&self) -> usize {
        self.size * self.size
    }
}

/// Per-node scratch state that survives between evaluations (file caches etc.).
#[derive(Default)]
pub struct NodeState {
    slot: Mutex<Option<Box<dyn Any + Send>>>,
}

impl NodeState {
    pub fn with<T: Any + Send + Default, R>(&self, function: impl FnOnce(&mut T) -> R) -> R {
        let mut guard = self.slot.lock();
        let needs_init = !matches!(guard.as_ref(), Some(value) if value.is::<T>());
        if needs_init {
            *guard = Some(Box::new(T::default()));
        }
        let value = guard.as_mut().and_then(|boxed| boxed.downcast_mut::<T>()).expect("state type checked above");
        function(value)
    }

    pub fn clear(&self) {
        *self.slot.lock() = None;
    }
}

impl std::fmt::Debug for NodeState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("NodeState")
    }
}
