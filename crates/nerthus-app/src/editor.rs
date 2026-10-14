//! Editor state: the document, the selection, what the viewport previews
//! and how the build of every node went.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bevy::ecs::system::NonSendMarker;
use bevy::prelude::{App, DetectChanges, Plugin, Res, ResMut, Resource, Single, Update, Window, With};
use nerthus_engine::{Map, NodeState, PortIntents, RenderIntent, Resolution, StateOverride};
use nerthus_export::bake::{self, BakeStore};
use bevy::math::URect;
use nerthus_render::{CameraValues, ScreenshotRequest, TerrainPreview};
use web_time::Instant;

use crate::compute::ComputeState;
use crate::document::Document;
use crate::export_build::ExportBuild;
use crate::launch::LaunchOptions;
use crate::progress::{ProgressStep, ProgressWindow, StepEnd, WorkProgress};
use crate::user::User;
pub use crate::viewport_render::RenderMode;
use crate::viewport_camera;
use crate::viewport_render::{Colorfield, MaskEnhancement, MaskGradient, RenderJob, RenderQuality, ViewportLighting, ViewportRenderer};
use crate::worker::{BakeJob, BuildEvent, BuildRequest, BuildWorker, NodeCache};

/// Preview resolutions offered (`Preview > Resolution`).
pub const PREVIEW_RESOLUTIONS: [usize; 4] = [512, 1024, 2048, 4096];

/// `ShowRegionUnbakeWarning`'s message.
pub const REGION_UNBAKE_WARNING: &str = "You cannot unbake this node. It requires to be baked for use in Regions.\n\nTo unbake it, you should switch to Standard preview and then unbake it. You will then need to bake it again before Regions can be used again.";

/// `CloseFile`'s progress window.
const FREEZING_TITLE: &str = "Freezing Cached Data to Disk";

/// One node's build as the graph shows it.
#[derive(Clone, Debug)]
pub struct NodeStatus {
    pub state: NodeState,
    pub building: bool,
    pub milliseconds: f32,
    pub error: Option<String>,
    /// When it last finished building (`Node.LastUpdate`).
    pub built_at: Option<Instant>,
    /// The lowest and highest value of a one-channel main output.
    pub range: Option<(f32, f32)>,
}

impl Default for NodeStatus {
    fn default() -> Self {
        Self {
            state: NodeState::Unbuilt,
            building: false,
            milliseconds: 0.0,
            error: None,
            built_at: None,
            range: None,
        }
    }
}

/// Copy Properties' source (`Shell.SourceNode`): the node, its type, its
/// values as copied, and which document it was in.
#[derive(Clone, Debug)]
pub struct CopiedNode {
    pub id: i32,
    pub type_name: String,
    pub values: indexmap::IndexMap<String, serde_json::Value>,
    pub document_serial: u64,
}

/// What stops a bake before it starts, each with the message box that
/// says so.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BakeRefusal {
    /// Bake Selected on a Macro.
    MacroNode,
    /// A node the bake builds is bypassed.
    BypassedChain,
    /// A node the bake builds is a Macro.
    MacroInChain,
}

impl BakeRefusal {
    pub fn caption(self) -> &'static str {
        match self {
            BakeRefusal::MacroNode => "Cannot bake macro node",
            BakeRefusal::BypassedChain => "Cannot Start Baking",
            BakeRefusal::MacroInChain => "Macro Detected in Bake Chain",
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            BakeRefusal::MacroNode => "Selected node is a Macro and cannot be baked.",
            BakeRefusal::BypassedChain => "Some nodes required for baking are currently bypassed. Please enable those nodes and try again.",
            BakeRefusal::MacroInChain => "This node depends on a Macro and cannot be baked.",
        }
    }
}

/// A file dialog the interface asked for; dialogs block, so a main-thread
/// system shows them.
#[derive(Clone, Debug)]
pub enum DialogRequest {
    SaveAs,
    /// A file parameter's folder button, from the path it holds.
    BrowseFile { node: i32, parameter: String, extensions: Vec<String>, folder: bool, current: String },
    /// The Data View's `btnBrowse`: the folder builds go under, starting
    /// from the destination typed when it is a folder.
    BuildDestination { current: String },
    /// Viewport Options > Load Camera and Save Camera (the camera as it
    /// stood when asked).
    LoadCamera,
    SaveCamera { camera: CameraValues },
    /// Save Viewport Screenshot as… (Ctrl F9) and Save Graph Screenshot
    /// as… (the graph's part of the window, in physical pixels).
    SaveScreenshot { graph_region: Option<URect>, supersample: u32 },
    /// Synth's Export JSON: where to save the node's colours
    /// (`Synth_ExportColors`).
    ExportSynthColours { node: i32 },
    /// The 2D viewport's Save to file (`BtnSave_OnClick`): its image as an
    /// 8-bit PNG.
    Save2DView { colours: Arc<nerthus_render::TerrainColours> },
    /// Overlay › Custom: the image to lay over the view.
    CustomOverlay,
}

#[derive(Resource)]
pub struct Editor {
    pub document: Document,
    /// The user's options, preferences and recent files.
    pub user: User,
    /// The node presets in the presets folder (`PresetService.Presets`).
    pub presets: crate::presets::PresetLibrary,
    /// Counts the documents opened (each `set_document`), so views made for
    /// one are not kept for the next.
    pub document_serial: u64,
    /// The node Copy Properties took (Paste Properties reads it).
    pub copied_node: Option<CopiedNode>,
    /// Nodes Cut or Copy took, until they are pasted (`IsPasteActive`).
    pub node_clipboard: Option<crate::document::NodeClipboard>,
    /// How often the graph was mutated this session.
    pub mutation: i32,
    /// Selected nodes; the last is the one the property area shows.
    pub selection: Vec<i32>,
    /// The node the viewport shows: the selection unless locked (`F`).
    pub preview_node: Option<i32>,
    pub preview_locked: bool,
    pub preview_port: String,
    pub preview_resolution: usize,
    /// The region the editor previews (`CurrentPortion.Region`), by name;
    /// none for the whole terrain.
    pub preview_region: Option<String>,
    /// The renderer (`B.Current`) and what it reads: its quality
    /// (`Quality`, quick while a gizmo or Draw's painter is held), its
    /// mask enhancement (`AutolevelMasks`), the mask gradient
    /// (`B.ClutMap`) and the project's lighting.
    pub render_mode: RenderMode,
    render_quality: RenderQuality,
    pub mask_enhancement: MaskEnhancement,
    pub mask_gradient: MaskGradient,
    pub lighting: ViewportLighting,
    /// The overlay laid over renders (Viewport Helpers › Overlay).
    pub overlay: crate::overlays::OverlaySettings,
    /// The last render asked for and the last one shown, and whether the
    /// last one asked for was lit quickly.
    render_generation: u64,
    shown_render: u64,
    rendered_quickly: bool,
    /// The colours of the render shown last (`B.Current.GetRender`), which
    /// the Regions tab shows behind the regions.
    pub render_colours: Option<Arc<nerthus_render::TerrainColours>>,
    needs_render: bool,
    /// Where the 3D view's camera is, as of this frame, and where it was
    /// asked to jump (`SetCamera`).
    pub camera: CameraValues,
    pub camera_request: Option<CameraValues>,
    /// Screenshots: how many times the view's size the 3D view's are
    /// (`screenshotSupersample`), those asked for, and where the graph is
    /// in the window as of this frame.
    pub screenshot_supersample: u32,
    pub screenshots: Vec<ScreenshotRequest>,
    pub graph_region: Option<URect>,
    /// Builds run (`Engine Active`).
    pub engine_active: bool,
    /// The property area's "Toggle Gizmo" (`tglGizmo`): off again whenever
    /// the selection changes.
    pub gizmo_active: bool,
    /// Nodes to shine since the graph last looked (`NodeElement.Shine`):
    /// each node built well with the renderer at `Normal`, and a bookmark
    /// gone to.
    pub shines: Vec<i32>,
    /// The viewport was cleared (`B.ClearViewport`) and nothing has been
    /// previewed since; the cleared view is still to be shown, in the
    /// light it was cleared in, while `pending_clear` holds it.
    viewport_cleared: bool,
    pending_clear: Option<ViewportLighting>,
    /// When a debounced parameter edit's build is due.
    build_due: Option<Instant>,
    /// V: the preview laid flat, its colours kept (`Shell.isFlat`), until
    /// the next build finishes.
    pub flat_preview: bool,
    /// Seamless's Preview Tiling (`PropertyArea.IsTiledPreview`): the 3D
    /// view repeats the map twice each way, until another node comes into
    /// the property area.
    pub tiled_preview: bool,
    pub statuses: HashMap<i32, NodeStatus>,
    /// The previewed node's maps by port, and what colours drape over.
    pub outputs: Vec<(String, Arc<Map>)>,
    pub underlay: Option<Arc<Map>>,
    /// What the previewed node is shown over when its type has its own
    /// underlay (`Node.GetCustomUnderlay`).
    pub custom_underlay: Option<Arc<Map>>,
    /// The previewed node's own colours (`Node.GetCustomColorMap()`) when
    /// its type has them.
    pub custom_colours: Option<Arc<Map>>,
    /// The intents the previewed node's output ports were left with
    /// (`Port.RenderIntent`).
    pub port_intents: PortIntents,
    pub preview_error: Option<String>,
    pub building: bool,
    pub needs_build: bool,
    /// Esc, or suspending the engine: the running build stops at its next
    /// node.
    pub cancel_build: bool,
    generation: u64,
    /// The last build request that was cancelled rather than finished.
    cancelled_generation: u64,
    build_started: Option<Instant>,
    /// Seconds the last preview build took (the graph footer's clock).
    pub last_build_seconds: f32,
    /// Fraction of the current build's nodes done.
    pub build_progress: f32,
    /// What the engine's cached maps hold (the status bar's memory).
    pub cache_bytes: usize,
    /// What the engine keeps of each node, as of the last build.
    pub node_caches: HashMap<i32, NodeCache>,
    build_total: usize,
    build_done: usize,
    pub messages: Vec<String>,
    pub dialog: Option<DialogRequest>,
    /// Macros the open project uses, loaded for the next build.
    pending_macros: Option<nerthus_engine::macros::MacroLibrary>,
    /// The Build and Export running or last run.
    pub export_build: Option<ExportBuild>,
    /// A Build and Export choice waiting for Save As, and once the project
    /// is saved, waiting to go on.
    pub(crate) pending_build: Option<crate::export_build::BuildChoice>,
    pub resume_build: Option<crate::export_build::BuildChoice>,
    /// Save cached build's work for the next request.
    pub(crate) pending_cached_export: Option<crate::export_build::CachedExportJob>,
    /// Bake work for the next request: nodes to bake, nodes just unbaked,
    /// and Force load baked cache.
    pending_bake: Option<BakeJob>,
    pending_unbake: Vec<i32>,
    pending_load_all_bakes: bool,
    /// Baked nodes whose bakes are read back from disk (Load Offloaded
    /// Cache).
    pending_load_bakes: Vec<i32>,
    /// A request goes to the build thread even with nothing to build, for
    /// its preparation alone (a project closed or opened: previews written,
    /// bakes loaded, previews read back).
    pending_preparation: bool,
    /// Nodes whose kept data the next build drops (Purge Cache).
    pending_forget: Vec<i32>,
    /// Node commands for the build thread (Fit Curve, Synthesize Colors).
    pending_node_commands: Vec<crate::worker::NodeCommand>,
    /// Nodes floating 2D views show (`MapViewer.NodeId`), and their maps as
    /// the builds left them; the revision counts the maps taken in.
    watched: Vec<i32>,
    watched_refresh: bool,
    pub watched_outputs: HashMap<i32, crate::worker::WatchedNode>,
    pub watched_revision: u64,
    /// What the disk cache does next (Smart Cache, Cache to Disk).
    pub disk_cache: crate::disk_cache::EditorDiskCache,
    /// A bake is running on the build thread.
    pub baking: bool,
    /// The compute devices (CPU-only mode, devices loading at start).
    pub compute: ComputeState,
    /// What File and the file box do beyond opening and saving: pending
    /// questions, autosaves, the macro collection, thumbnails.
    pub files: crate::files::FileState,
    /// The MCP server as Tools › MCP Server shows it.
    pub mcp: crate::mcp::McpStatus,
    /// A long operation's progress window, while it shows
    /// (`ProgressDialog`).
    pub progress: Option<ProgressWindow>,
    /// Exit waits for the project's previews to be written.
    exit_after_freeze: bool,
}

impl Editor {
    pub fn new(document: Document) -> Self {
        let preview_resolution = preview_resolution_of(&document);
        let mut editor = Self {
            document,
            user: User::in_memory(),
            presets: crate::presets::PresetLibrary::default(),
            document_serial: 1,
            copied_node: None,
            node_clipboard: None,
            mutation: 0,
            selection: Vec::new(),
            preview_node: None,
            preview_locked: false,
            preview_port: "Out".to_string(),
            preview_resolution,
            preview_region: None,
            render_mode: RenderMode::Realistic,
            render_quality: RenderQuality::Normal,
            mask_enhancement: MaskEnhancement::None,
            mask_gradient: MaskGradient::default(),
            lighting: ViewportLighting::default(),
            overlay: crate::overlays::OverlaySettings::default(),
            render_generation: 0,
            shown_render: 0,
            rendered_quickly: false,
            render_colours: None,
            needs_render: false,
            camera: CameraValues::DEFAULT,
            camera_request: None,
            screenshot_supersample: 1,
            screenshots: Vec::new(),
            graph_region: None,
            engine_active: true,
            gizmo_active: false,
            shines: Vec::new(),
            viewport_cleared: false,
            pending_clear: None,
            build_due: None,
            flat_preview: false,
            tiled_preview: false,
            statuses: HashMap::new(),
            outputs: Vec::new(),
            underlay: None,
            custom_underlay: None,
            custom_colours: None,
            port_intents: PortIntents::new(),
            preview_error: None,
            building: false,
            needs_build: false,
            cancel_build: false,
            generation: 0,
            cancelled_generation: 0,
            build_started: None,
            last_build_seconds: 0.0,
            build_progress: 0.0,
            cache_bytes: 0,
            node_caches: HashMap::new(),
            build_total: 0,
            build_done: 0,
            messages: Vec::new(),
            dialog: None,
            pending_macros: None,
            export_build: None,
            pending_build: None,
            resume_build: None,
            pending_cached_export: None,
            pending_bake: None,
            pending_unbake: Vec::new(),
            pending_load_all_bakes: false,
            pending_load_bakes: Vec::new(),
            pending_preparation: false,
            pending_forget: Vec::new(),
            pending_node_commands: Vec::new(),
            watched: Vec::new(),
            watched_refresh: false,
            watched_outputs: HashMap::new(),
            watched_revision: 0,
            disk_cache: crate::disk_cache::EditorDiskCache::default(),
            baking: false,
            compute: ComputeState::default(),
            files: crate::files::FileState::default(),
            mcp: crate::mcp::McpStatus::default(),
            progress: None,
            exit_after_freeze: false,
        };
        editor.load_viewport_state();
        // `SetupNewFile` starts with the viewport cleared.
        editor.clear_viewport();
        editor.select_initial_node();
        editor
    }

    pub fn report(&mut self, message: impl Into<String>) {
        let message = message.into();
        log::info!("{message}");
        self.messages.push(message);
    }

    pub fn window_title(&self) -> String {
        let marker = if self.document.dirty { "*" } else { "" };
        format!("{}{marker} - Nerthus", self.document.file_name())
    }

    /// Replaces the document (File > New, Open).
    pub fn set_document(&mut self, document: Document) {
        self.freeze_on_close();
        // `CloseFile` clears the viewport whatever the options say, in the
        // closing project's light.
        self.clear_viewport();
        self.preview_resolution = preview_resolution_of(&document);
        self.document = document;
        self.document_serial += 1;
        self.selection.clear();
        self.preview_node = None;
        self.preview_locked = false;
        self.preview_region = None;
        self.statuses.clear();
        self.node_caches.clear();
        self.outputs.clear();
        self.underlay = None;
        self.custom_underlay = None;
        self.custom_colours = None;
        self.port_intents.clear();
        self.preview_error = None;
        self.watched_outputs.clear();
        let ids: Vec<i32> = self.document.terrain().nodes.keys().copied().collect();
        self.document.apply_node_display(&ids, &self.user.options);
        self.load_viewport_state();
        self.load_macros();
        if self.document.path.is_none() {
            // `SetupNewFile` clears it again, in the new project's light.
            self.pending_clear = Some(self.lighting);
        }
        self.select_initial_node();
        self.thaw_on_open();
        // The disk cache's tasks go with the next request, preview or not.
        self.needs_build = self.preview_node.is_some() || !self.disk_cache.tasks.is_empty();
        let opening = self.document.path.is_some();
        self.show_switch_progress(opening);
    }

    /// Shows the progress window for `steps`, in place of any other.
    pub fn show_progress(&mut self, steps: Vec<ProgressStep>) {
        self.progress = Some(ProgressWindow::new(steps));
    }

    /// The progress a project's closing and opening show: `CloseFile`'s
    /// freeze when it has one, then `Session_FileOpened`'s "Opening
    /// file..." for an opened project. Both end as the build thread gets
    /// through the next request's preparation, which goes even with
    /// nothing to build.
    fn show_switch_progress(&mut self, opening: bool) {
        let freezing = self.disk_cache.tasks.iter().any(|task| matches!(task, crate::disk_cache::DiskCacheTask::Freeze { .. }));
        let mut steps = Vec::new();
        if freezing {
            steps.push(ProgressStep::new(FREEZING_TITLE, StepEnd::Frozen));
        }
        if opening {
            steps.push(ProgressStep::new("Opening file...", StepEnd::Prepared { after: self.generation }));
        }
        if steps.is_empty() {
            return;
        }
        self.pending_preparation = true;
        self.needs_build = true;
        self.show_progress(steps);
    }

    /// Exit (`Shell_Closing`'s `CloseFile`): when the project's previews
    /// go to disk as it closes, they are written first, with the progress
    /// window, and Nerthus closes once they are. False when there is
    /// nothing to write.
    pub fn exit_when_frozen(&mut self) -> bool {
        let Some((store, terrain)) = self.closing_freeze() else {
            return false;
        };
        self.disk_cache.tasks.push(crate::disk_cache::DiskCacheTask::Freeze { store, terrain, done: None });
        self.exit_after_freeze = true;
        self.needs_build = true;
        self.show_progress(vec![ProgressStep::new(FREEZING_TITLE, StepEnd::Frozen)]);
        true
    }

    /// What the build thread says of a long operation, for the progress
    /// window; a freeze Exit waited for lets Nerthus close.
    fn take_progress(&mut self, progress: WorkProgress) {
        if progress == WorkProgress::Frozen && self.exit_after_freeze {
            self.exit_after_freeze = false;
            self.disk_cache.frozen_for_exit = true;
            self.files.exit_now = true;
        }
        let Some(window) = self.progress.as_mut() else {
            return;
        };
        window.take(&progress);
        if window.is_done() {
            self.progress = None;
        }
    }

    /// The Region Manager has its preview: its progress step ends.
    pub fn region_preview_ready(&mut self) {
        let Some(window) = self.progress.as_mut() else {
            return;
        };
        window.region_preview_ready();
        if window.is_done() {
            self.progress = None;
        }
    }

    /// Opens a project as `Shell.OpenFile` does (see
    /// `files::open_project`).
    pub fn open(&mut self, path: &Path) {
        crate::files::open_project(self, path);
    }

    pub fn save_to(&mut self, path: &Path) {
        // `SaveFile` leaves a read-only file alone: the changes stay
        // unsaved, an alert says so until closed, and a safety autosave is
        // tried.
        if path.exists() && crate::files::is_read_only(path) {
            log::warn!("File not saved. Read-only location at {}", path.display());
            let text = "The file was not saved as it is read-only. You still have unsaved changes. Try saving to a new location.";
            let alert = crate::ui::alert::Alert::new("File not saved!", text, crate::ui::alert::AlertKind::Error);
            self.files.alert = Some(alert.staying());
            crate::files::try_autosave(self);
            return;
        }
        // Save As keeps the bakes: the old file's bake folder is copied to
        // the new one's once saved (`oldCachePath`).
        let renamed = self.document.path.as_deref().is_some_and(|current| current != path);
        let old_bakes = if renamed { self.bake_store() } else { None };
        // `SaveFile`: the selection as it stands, then
        // `SanitizeStateObject`.
        let selected = self.active_node();
        self.document.note_selected_node(selected);
        self.document.sanitize_state();
        match self.document.save(path) {
            Ok(()) => {
                self.user.add_recent(path);
                // `SaveFile` notes the folder of every save for Save As.
                self.user.locations.set_from_file(crate::user::locations::SAVE, path);
                self.report(format!("Saved {}", path.display()));
                crate::files::saved(self, path);
            }
            Err(error) => {
                self.files.show_message("Nerthus", "There was an error saving the file.");
                log::error!("{error:#}");
                return;
            }
        }
        let new_bakes = self.bake_store();
        if let (Some(old), Some(new)) = (old_bakes, new_bakes)
            && old.folder().is_dir()
            && old != new
            && let Err(error) = copy_folder(old.folder(), new.folder())
        {
            self.report(format!("The baked maps could not be copied to {}: {error}", new.folder().display()));
        }
    }

    /// The confirmation's Start Build: a checked "Open folder after build
    /// finishes" is noted in the build settings, the file saved, then what
    /// was saved built.
    pub fn start_export_build(&mut self, open_folder: bool) {
        if open_folder {
            self.document.set_build_setting("OpenFolder", serde_json::Value::Bool(true));
        }
        match self.document.path.clone() {
            Some(path) => {
                self.save_to(&path);
                if self.document.dirty {
                    return;
                }
                self.launch_export_build(&path);
            }
            None => {
                self.pending_build = Some(crate::export_build::BuildChoice { open_folder, close: false });
                self.dialog = Some(DialogRequest::SaveAs);
            }
        }
    }

    pub(crate) fn launch_export_build(&mut self, path: &Path) {
        // Builds are written to folders, and run on a thread of their own:
        // the web build has neither.
        if cfg!(target_arch = "wasm32") {
            self.report("Build and Export is not available in the web version of Nerthus.");
            return;
        }
        // As the build will see it: a region it builds sets the resolution.
        let mut definition = self.document.build_definition();
        let region = nerthus_export::build::set_up_region(self.document.terrain(), &mut definition, None);
        let resolution = definition.resolution;
        let built = match &region {
            Some(region) => format!("{} ({})", self.document.file_name(), region.name),
            None => self.document.file_name(),
        };
        self.report(format!("Building {built} at {resolution} x {resolution}"));
        let preferences = self.user.preferences.export_preferences();
        self.export_build = Some(ExportBuild::start(path, resolution, definition.open_folder, preferences));
    }

    /// Takes in a running Build and Export's progress.
    fn poll_export_build(&mut self) {
        let Some(build) = self.export_build.as_mut() else {
            return;
        };
        if !build.poll() {
            return;
        }
        let summary = build.summary();
        let open = build.opens_folder();
        let folder = build.folder.clone();
        self.report(summary);
        if let (true, Some(folder)) = (open, folder)
            && folder.is_dir()
        {
            crate::export_build::open_folder(&folder);
        }
    }

    /// Project > Open Build Folder: this project's folder under the
    /// builds folder when there is one, else the builds folder.
    pub fn open_build_folder(&mut self) {
        let root = nerthus_export::destination::default_builds_root();
        let project = root.join(self.document.file_name().trim_end_matches(".terrain"));
        let folder = if project.is_dir() { project } else { root };
        if let Err(error) = std::fs::create_dir_all(&folder) {
            self.report(format!("Cannot open {}: {error}", folder.display()));
            return;
        }
        crate::export_build::open_folder(&folder);
    }

    /// Saves to the current file, or asks for one.
    pub fn save(&mut self) {
        match self.document.path.clone() {
            Some(path) => self.save_to(&path),
            None => self.dialog = Some(DialogRequest::SaveAs),
        }
    }

    /// The project's macros (`M.EnsureMacros`), from the macro collection
    /// or a `Macros` folder beside the project, with the macros they depend
    /// on; dependencies the project did not list join its list.
    fn load_macros(&mut self) {
        let ids = self.document.project.macro_ids();
        if ids.is_empty() {
            self.pending_macros = Some(Default::default());
            return;
        }
        let folders = crate::files::macro_folders(self.document.path.as_deref());
        let ensured = nerthus_engine::macros::ensure_project_macros(&ids, &folders);
        for (id, problem) in ensured.problems {
            self.report(format!("Macro {id} could not be loaded: {problem}"));
        }
        self.document.list_project_macros(&ensured.added);
        self.pending_macros = Some(ensured.library);
    }

    /// Reads the project's macros again for the next build (a macro was
    /// added).
    pub fn reload_macros(&mut self) {
        self.load_macros();
    }

    /// What the project was saved showing (`Session_FileOpened`): the
    /// node the preview is locked to (`Reconstruct`: the first node marked
    /// locked) and the selected node (`TerrainState.SelectedNode`), when
    /// it is still there; otherwise nothing is selected.
    fn select_initial_node(&mut self) {
        let locked = self.document.settle_preview_lock();
        if let Some(locked) = locked {
            self.preview_locked = true;
            self.preview(Some(locked));
        }
        let saved = self.document.saved_selected_node();
        let chosen = saved.filter(|id| self.document.node(*id).is_some());
        self.selection.clear();
        if let Some(id) = chosen {
            self.selection.push(id);
            if !self.preview_locked {
                self.preview(Some(id));
            }
        }
    }

    pub fn active_node(&self) -> Option<i32> {
        self.selection.last().copied()
    }

    /// Selects one node (`None` clears the selection).
    pub fn select(&mut self, node: Option<i32>) {
        if self.selection.last().copied() != node {
            self.gizmo_active = false;
        }
        self.selection.clear();
        match node {
            Some(id) => {
                self.selection.push(id);
                if !self.preview_locked {
                    self.preview(Some(id));
                }
            }
            None => self.selection_without_one_node(),
        }
    }

    /// A group chosen in place of the nodes: the nodes are let go but the
    /// preview stays (`Graph_SelectionChanged` hands a group on as a group,
    /// so `PropertyArea.ChangeNode` never runs).
    pub fn select_group(&mut self) {
        if !self.selection.is_empty() {
            self.gizmo_active = false;
        }
        self.selection.clear();
    }

    /// Adds a node to or removes it from the selection (Ctrl-click).
    pub fn toggle_selected(&mut self, node: i32) {
        match self.selection.iter().position(|id| *id == node) {
            Some(position) => {
                self.selection.remove(position);
            }
            None => self.selection.push(node),
        }
        self.follow_selection();
    }

    pub fn set_selection(&mut self, nodes: Vec<i32>) {
        self.selection = nodes;
        self.follow_selection();
    }

    /// `PropertyArea.ChangeNode`: a single node selected is previewed;
    /// none, or several, leave the preview.
    fn follow_selection(&mut self) {
        match self.selection.as_slice() {
            [only] => {
                let only = *only;
                if !self.preview_locked {
                    self.preview(Some(only));
                }
            }
            _ => self.selection_without_one_node(),
        }
    }

    /// No node, or several, selected: the viewport is cleared
    /// (`B.ClearViewport`) unless a preview is locked or Do not clear
    /// viewport when unselecting a node is on.
    fn selection_without_one_node(&mut self) {
        if self.preview_locked || self.user.options.dont_clear_viewport() {
            return;
        }
        self.clear_viewport();
    }

    /// `B.ClearViewport`: nothing previewed; the 3D and 2D views show a
    /// flat terrain lit by the clearing renderer, once `process_builds`
    /// gets to it.
    pub fn clear_viewport(&mut self) {
        if self.viewport_cleared {
            return;
        }
        self.preview(None);
        self.outputs.clear();
        self.underlay = None;
        self.custom_underlay = None;
        self.custom_colours = None;
        self.port_intents.clear();
        self.preview_error = None;
        // A render on its way is of the preview gone.
        self.needs_render = false;
        self.shown_render = self.render_generation;
        self.viewport_cleared = true;
        self.pending_clear = Some(self.lighting);
    }

    /// Shows a node in the viewport.
    pub fn preview(&mut self, node: Option<i32>) {
        if self.preview_node == node {
            return;
        }
        if node.is_some() {
            self.viewport_cleared = false;
        }
        self.preview_node = node;
        self.preview_port = "Out".to_string();
        self.needs_build = node.is_some();
    }

    /// `GraphControl.LockPreview` (F, Lock Preview to Node, the node menu's
    /// Lock Preview): the node's preview lock is toggled as an edit that
    /// is saved with the project; locking takes the lock from whichever
    /// node had it.
    pub fn lock_preview(&mut self, node: i32) {
        let Some(name) = self.document.node(node).map(|data| data.name.clone()) else {
            return;
        };
        let Some(locked) = self.document.toggle_preview_lock(node) else {
            return;
        };
        let message = if locked { format!("Preview locked to {name}.") } else { "Preview lock removed.".to_string() };
        self.report(message);
        self.follow_preview_lock();
    }

    /// The preview follows the project's lock (`Node.IsLocked`), which
    /// undo, redo and deleting nodes change with the rest of the project:
    /// locked, the locked node is shown; unlocked, the selection.
    pub fn follow_preview_lock(&mut self) {
        let locked = self.document.locked_node();
        let shown = if self.preview_locked { self.preview_node } else { None };
        if locked == shown {
            return;
        }
        match locked {
            Some(node) => {
                self.preview_locked = true;
                self.preview(Some(node));
            }
            None => {
                self.preview_locked = false;
                let active = self.active_node();
                self.preview(active);
            }
        }
    }

    pub fn set_preview_resolution(&mut self, resolution: usize) {
        if self.preview_resolution == resolution {
            return;
        }
        self.preview_resolution = resolution;
        self.needs_build = self.preview_node.is_some();
    }

    /// A parameter edited in the property area (`EditorBase.Control_Changed`):
    /// the node builds once its edits pause for the debounce
    /// (`UpdateEventFired`), 100 ms; with Use Adaptive Debounce for
    /// Smoother Node Updates, by how long the node last took
    /// (`GetDebounceLimit`).
    pub fn parameter_changed(&mut self, node: i32) {
        let wait = if self.user.options.adaptive_debounce() { adaptive_debounce(self.status(node).milliseconds) } else { DEBOUNCE };
        self.build_after(wait);
    }

    /// A modifier's parameter edited (`ModifierUpdateEventFired`): 100 ms.
    pub fn modifier_changed(&mut self) {
        self.build_after(DEBOUNCE);
    }

    /// Builds `wait` after the last call (now, for no wait).
    fn build_after(&mut self, wait: Duration) {
        if self.preview_node.is_none() {
            return;
        }
        if wait.is_zero() {
            self.build_due = None;
            self.needs_build = true;
            return;
        }
        self.build_due = Some(Instant::now() + wait);
    }

    /// Call after editing the document: rebuilds what the edit touched.
    pub fn changed(&mut self) {
        self.needs_build = self.preview_node.is_some();
    }

    /// Where the project's baked maps are, once it is saved.
    pub fn bake_store(&self) -> Option<BakeStore> {
        let path = self.document.path.as_ref()?;
        let preferences = self.user.preferences.export_preferences();
        Some(preferences.bake_store(path, &self.document.project_id()))
    }

    /// `ValidateBakeChain`: what stops `nodes` from being baked, node by
    /// node. Baking a node builds it and everything upstream of it; none
    /// of those may be bypassed, then none may be a Macro.
    pub fn bake_chain_refusal(&self, nodes: &[i32]) -> Option<BakeRefusal> {
        let terrain = self.document.terrain();
        for node in nodes {
            let chain = nerthus_engine::Engine::build_order(terrain, *node);
            let chain_nodes: Vec<&nerthus_engine::NodeData> = chain.iter().filter_map(|id| terrain.nodes.get(id)).collect();
            let bypassed = chain_nodes.iter().any(|data| StateOverride::of(data) == StateOverride::Bypassed);
            if bypassed {
                return Some(BakeRefusal::BypassedChain);
            }
            let has_macro = chain_nodes.iter().any(|data| data.type_name == "Macro");
            if has_macro {
                return Some(BakeRefusal::MacroInChain);
            }
        }
        None
    }

    /// `ValidateCurrentNodeBake`: what stops Bake Selected on `node`, which
    /// must not be a Macro itself, then its chain as above.
    pub fn bake_selected_refusal(&self, node: i32) -> Option<BakeRefusal> {
        let is_macro = self.document.node(node).is_some_and(|data| data.type_name == "Macro");
        if is_macro {
            return Some(BakeRefusal::MacroNode);
        }
        self.bake_chain_refusal(&[node])
    }

    /// Saves before baking (`SaveFile`); false when the project has
    /// no file yet ("You must save this file to bake it.").
    fn save_for_bake(&mut self) -> bool {
        let Some(path) = self.document.path.clone() else {
            self.report("The file must be saved before baking. You must save this file to bake it.");
            self.dialog = Some(DialogRequest::SaveAs);
            return false;
        };
        self.save_to(&path);
        !self.document.dirty
    }

    /// Graph › Baking › Bake Selected (`BakeSelectedNode`) up to the bake,
    /// once [`Self::bake_selected_refusal`] has let it through: the node to
    /// bake once the project is saved.
    pub fn prepare_bake_selected(&mut self) -> Option<i32> {
        let node = self.active_node()?;
        if !self.save_for_bake() {
            return None;
        }
        Some(node)
    }

    /// Graph › Baking › Bake Required Nodes as if confirmed
    /// (`miBakeGates_Click`, for `--show bake-required`): every node that
    /// needs baking, orphans aside. A refusal goes to the status bar.
    pub fn bake_required(&mut self) {
        let nodes = bake::required_nodes(self.document.terrain());
        if let Some(refusal) = self.bake_chain_refusal(&nodes) {
            self.report(format!("{}: {}", refusal.caption(), refusal.text()));
            return;
        }
        if let Some(nodes) = self.prepare_bake_required() {
            self.start_bake(nodes);
        }
    }

    /// Bake Required Nodes up to the bake, once their chains were checked
    /// ([`Self::bake_chain_refusal`]) and the question answered: the nodes
    /// to bake once the project is saved.
    pub fn prepare_bake_required(&mut self) -> Option<Vec<i32>> {
        let nodes = bake::required_nodes(self.document.terrain());
        if nodes.is_empty() {
            self.report("No nodes need baking.");
            return None;
        }
        if !self.save_for_bake() {
            return None;
        }
        Some(nodes)
    }

    pub(crate) fn start_bake(&mut self, nodes: Vec<i32>) {
        let resolution = self.document.build_definition().bake_resolution;
        self.report(format!("Please wait... Baking at {resolution} x {resolution} may take some time depending on the number of nodes."));
        self.pending_bake = Some(BakeJob { nodes, resolution });
        self.baking = true;
        self.needs_build = true;
    }

    /// A bake's end: the baked nodes marked and the file saved, or the
    /// error shown.
    fn finish_bake(&mut self, nodes: Vec<i32>, error: Option<String>) {
        self.baking = false;
        if let Some(error) = error {
            self.report(format!("Error! {error}"));
            return;
        }
        for node in &nodes {
            self.document.apply_build_update(*node, "StateOverride", serde_json::Value::from("Baked"));
        }
        self.document.dirty = true;
        if let Some(path) = self.document.path.clone() {
            self.save_to(&path);
        }
        self.report("Success! Baking finished successfully");
        self.needs_build = self.preview_node.is_some();
    }

    /// Unbakes `nodes`: they keep their maps as
    /// ordinary results, their files are deleted, and the file is saved.
    fn unbake(&mut self, nodes: Vec<i32>) {
        if nodes.is_empty() {
            return;
        }
        for node in &nodes {
            self.document.apply_build_update(*node, "StateOverride", serde_json::Value::from("None"));
        }
        self.document.dirty = true;
        self.pending_unbake.extend(nodes);
        self.needs_build = true;
        if let Some(path) = self.document.path.clone() {
            self.save_to(&path);
        }
    }

    /// Unbake Selected, once confirmed: the node and the
    /// baked nodes downstream of it.
    pub fn unbake_selected(&mut self) {
        let Some(node) = self.active_node() else {
            return;
        };
        let terrain = self.document.terrain();
        let mut nodes = vec![node];
        for id in downstream_nodes(terrain, node) {
            let baked = terrain.nodes.get(&id).is_some_and(|data| StateOverride::of(data) == StateOverride::Baked);
            if baked {
                nodes.push(id);
            }
        }
        self.unbake(nodes);
    }

    /// Unbake All, once confirmed.
    pub fn unbake_all(&mut self) {
        let nodes = self.baked_nodes();
        self.unbake(nodes);
    }

    /// The nodes marked `Baked`.
    pub fn baked_nodes(&self) -> Vec<i32> {
        let terrain = self.document.terrain();
        terrain.nodes.values().filter(|node| StateOverride::of(node) == StateOverride::Baked).map(|node| node.id).collect()
    }

    /// The region being previewed, as the terrain has it now; none (the
    /// whole terrain) once it is gone.
    pub fn previewed_region(&self) -> Option<nerthus_engine::Region> {
        let name = self.preview_region.as_ref()?;
        self.document.regions().into_iter().find(|region| &region.name == name)
    }

    /// `Shell.MiRegion_Click`'s test: regions preview only once some node
    /// that requires baking is baked.
    pub fn can_preview_regions(&self) -> bool {
        let terrain = self.document.terrain();
        let mut needing = terrain.nodes.values().filter(|node| nerthus_export::bake::requires_baking(node));
        needing.any(|node| StateOverride::of(node) == StateOverride::Baked)
    }

    /// `Shell.UpdateRegionPreview` (a region) or `StandardPreview` (none):
    /// the bakes cut to the region, or whole again, and the preview built
    /// again.
    pub fn set_preview_region(&mut self, name: Option<String>) {
        if self.preview_region == name {
            return;
        }
        self.preview_region = name;
        self.document.mark_all_stale();
        self.needs_build = true;
        self.needs_render = true;
    }

    /// `RegionManager`'s `RegionUpdated` while a region is previewed: the
    /// region moved or changed size, so its bakes are cut again.
    pub fn region_changed(&mut self, name: &str) {
        if self.preview_region.as_deref() != Some(name) {
            return;
        }
        self.document.mark_all_stale();
        self.needs_build = true;
    }

    /// A node's Purge Cache (`PropertyArea.Editor_CommandExecuted`): the
    /// data it keeps between builds goes (a File node's loaded image) and
    /// it builds again (`B.SafelyBuildNode`).
    pub fn purge_cache(&mut self, node: i32) {
        self.pending_forget.push(node);
        self.document.mark_stale(node);
        self.changed();
    }

    /// The nodes the floating 2D views show: newly shown ones get their maps
    /// with the next build (the views wait for a build that leaves them
    /// built).
    pub fn watch_nodes(&mut self, nodes: Vec<i32>) {
        if nodes == self.watched {
            return;
        }
        let added = nodes.iter().any(|node| !self.watched.contains(node));
        self.watched_outputs.retain(|node, _| nodes.contains(node));
        self.watched = nodes;
        if added {
            self.watched_refresh = true;
            self.needs_build = true;
        }
    }

    /// A node command that works from the node's input (Curve's Fit Curve,
    /// Synth's Synthesize Colors): the build thread works out the new value
    /// and the editor applies it as an edit.
    pub fn run_node_command(&mut self, node: i32, name: &str) {
        let command = crate::worker::NodeCommand { node, name: name.to_string() };
        self.pending_node_commands.push(command);
        self.needs_build = true;
    }

    /// `MiApplyPreset_Click`: the selected node's type's preset of that
    /// name (`PresetService.ApplyPreset`).
    pub fn apply_preset(&mut self, name: &str) {
        let Some(node) = self.active_node() else {
            return;
        };
        let Some(type_name) = self.document.node(node).map(|data| data.type_name.clone()) else {
            return;
        };
        let Some(spec) = nerthus_engine::Catalogue::get().node(&type_name) else {
            return;
        };
        let Some(preset) = self.presets.named(name, &spec.name) else {
            return;
        };
        let values = crate::presets::preset_values(preset, spec);
        if self.document.set_values(node, &values, "Apply Preset") {
            self.changed();
        }
    }

    /// `Shell.G_BeforeNodeAdded`: a new node takes its type's `_default`
    /// preset, as part of being added.
    pub fn apply_default_preset(&mut self, node: i32) {
        let Some(type_name) = self.document.node(node).map(|data| data.type_name.clone()) else {
            return;
        };
        let Some(spec) = nerthus_engine::Catalogue::get().node(&type_name) else {
            return;
        };
        let Some(preset) = self.presets.default_for(&spec.name) else {
            return;
        };
        let values = crate::presets::preset_values(preset, spec);
        self.document.set_values_quietly(node, &values);
    }

    /// Force load baked cache (`miBakeLoadCache_Click`): every bake
    /// loaded, with the progress window; the project counts as changed.
    /// A baked node's Load Offloaded Cache: its bake, and those of the
    /// baked nodes downstream of it, read back from disk; what reads them
    /// builds again.
    pub fn load_offloaded_cache(&mut self, node: i32) {
        let mut nodes = vec![node];
        nodes.extend(self.baked_downstream(node));
        self.pending_load_bakes.extend(nodes);
        self.needs_build = true;
    }

    /// The Gate's Unbake Node, once confirmed (from the property area):
    /// the node and the baked nodes downstream of it unbake as with
    /// Unbake Selected, but the file is left unsaved.
    pub fn unbake_node(&mut self, node: i32) {
        let mut nodes = vec![node];
        nodes.extend(self.baked_downstream(node));
        for id in &nodes {
            self.document.apply_build_update(*id, "StateOverride", serde_json::Value::from("None"));
        }
        self.document.dirty = true;
        self.pending_unbake.extend(nodes);
        self.needs_build = true;
    }

    /// The bakes Load Offloaded Cache has yet to send to the build.
    #[cfg(test)]
    pub fn pending_load_bakes(&self) -> &[i32] {
        &self.pending_load_bakes
    }

    /// The baked nodes downstream of `node`.
    fn baked_downstream(&self, node: i32) -> Vec<i32> {
        let terrain = self.document.terrain();
        let downstream = downstream_nodes(terrain, node).into_iter();
        let baked = downstream.filter(|id| terrain.nodes.get(id).is_some_and(|data| StateOverride::of(data) == StateOverride::Baked));
        baked.collect()
    }

    /// `O.ShowRegionUnbakeWarning`'s test: while a region is previewed,
    /// unbaking is refused when a baked node needs baking (or `node`
    /// does).
    pub fn region_needs_bakes(&self, node: Option<i32>) -> bool {
        if self.preview_region.is_none() {
            return false;
        }
        let terrain = self.document.terrain();
        let critical = |data: &nerthus_engine::NodeData| nerthus_engine::Catalogue::get().node(&data.type_name).is_some_and(|spec| spec.has_flag("RequiresBaking"));
        let baked_critical = terrain.nodes.values().any(|data| StateOverride::of(data) == StateOverride::Baked && critical(data));
        let node_critical = node.and_then(|id| terrain.nodes.get(&id)).is_some_and(critical);
        baked_critical || node_critical
    }

    pub fn load_baked_cache(&mut self) {
        self.pending_load_all_bakes = true;
        self.needs_build = true;
        self.document.dirty = true;
        let step = ProgressStep::new("Loading Baked Cache", StepEnd::Prepared { after: self.generation });
        self.show_progress(vec![step]);
    }

    /// The Bake menu's resolution, once any bakes it costs were dropped
    /// (`MiBakeResolution_OnClick`: `BuildDefinition.BakeResolution`).
    pub fn set_bake_resolution(&mut self, resolution: usize, unbake_first: bool) {
        if unbake_first {
            let nodes = self.baked_nodes();
            for node in &nodes {
                self.document.apply_build_update(*node, "StateOverride", serde_json::Value::from("None"));
            }
            self.pending_unbake.extend(nodes);
            self.needs_build = true;
        }
        self.document.set_build_setting("BakeResolution", serde_json::json!(resolution));
    }

    pub fn status(&self, node: i32) -> NodeStatus {
        self.statuses.get(&node).cloned().unwrap_or_default()
    }

    /// Counts the preview builds asked for.
    pub fn build_generation(&self) -> u64 {
        self.generation
    }

    /// Whether the preview builds asked for after `since` (a
    /// `build_generation`) are over, and whether the last of them was
    /// cancelled rather than finished; none while one is still to come.
    pub fn build_outcome_since(&self, since: u64) -> Option<bool> {
        if self.needs_build || self.building || self.generation <= since {
            return None;
        }
        Some(self.cancelled_generation == self.generation)
    }

    /// Whether the newest render asked for has reached the 3D view.
    pub fn render_settled(&self) -> bool {
        !self.needs_render && self.shown_render >= self.render_generation
    }

    /// Seconds the running build has taken so far, or the last one took.
    pub fn build_seconds(&self) -> f32 {
        match self.build_started {
            Some(started) if self.building => started.elapsed().as_secs_f32(),
            _ => self.last_build_seconds,
        }
    }

    pub(crate) fn request_build(&mut self, worker: &BuildWorker) {
        // The build takes every edit made so far, debounced ones too.
        self.build_due = None;
        let bake_work = self.pending_bake.is_some() || !self.pending_unbake.is_empty() || self.pending_load_all_bakes || !self.pending_node_commands.is_empty();
        let bake_work = bake_work || !self.pending_load_bakes.is_empty();
        let bake_work = bake_work || !self.disk_cache.tasks.is_empty();
        let bake_work = bake_work || self.watched_refresh || self.pending_preparation;
        let bake_work = bake_work || self.pending_cached_export.is_some();
        if let Some(target) = self.preview_node
            && !self.document.terrain().nodes.contains_key(&target)
        {
            self.preview_node = None;
        }
        if self.preview_node.is_none() && !bake_work {
            return;
        }
        let target = self.preview_node.unwrap_or(0);
        // Exit's freeze goes even with the engine suspended.
        if !self.engine_active && !self.exit_after_freeze {
            // Nothing the progress window waits for comes while it is.
            if let Some(window) = self.progress.as_mut() {
                window.abandon_work();
                if window.is_done() {
                    self.progress = None;
                }
            }
            if self.pending_bake.take().is_some() {
                self.baking = false;
                self.report("Error! Bake failed. Ensure all nodes are valid and can build properly. (The engine is paused.)");
            }
            if let Some(job) = self.pending_cached_export.take() {
                job.abandon("The engine is paused.");
            }
            return;
        }
        self.generation += 1;
        self.pending_preparation = false;
        let (stale, clear) = self.document.take_stale();
        for node in &stale {
            self.statuses.remove(node);
        }
        if clear {
            self.statuses.clear();
        }
        let asset = &self.document.project.assets[0];
        let request = BuildRequest {
            generation: self.generation,
            terrain: Arc::new(asset.terrain.clone()),
            automation: asset.automation.clone(),
            project_path: self.document.path.clone(),
            target,
            resolution: Resolution::square(self.preview_resolution),
            stale,
            clear,
            macros: self.pending_macros.take(),
            underlay: self.document.underlay_node(),
            bakes: self.bake_store(),
            bake: self.pending_bake.take(),
            unbake: std::mem::take(&mut self.pending_unbake),
            load_all_bakes: std::mem::take(&mut self.pending_load_all_bakes),
            load_bakes: std::mem::take(&mut self.pending_load_bakes),
            region: self.previewed_region(),
            forget: std::mem::take(&mut self.pending_forget),
            node_commands: std::mem::take(&mut self.pending_node_commands),
            watched: self.watched.clone(),
            watched_refresh: std::mem::take(&mut self.watched_refresh),
            disk_cache: self.disk_cache_settings(),
            disk_cache_tasks: std::mem::take(&mut self.disk_cache.tasks),
            cached_export: self.pending_cached_export.take(),
            engine_preferences: self.user.preferences.engine_preferences(),
        };
        let order = nerthus_engine::Engine::build_order(&asset.terrain, target);
        self.build_total = order.iter().filter(|id| self.status(**id).state != NodeState::Built).count();
        self.build_done = 0;
        self.build_progress = 0.0;
        worker.request(request);
        self.building = true;
        self.build_started = Some(Instant::now());
    }

    fn handle_event(&mut self, event: BuildEvent) {
        match event {
            BuildEvent::NodeStarted(node) => {
                let status = self.statuses.entry(node).or_default();
                status.building = true;
            }
            BuildEvent::NodeFinished { node, milliseconds, state, error, range } => {
                // `Build_BuildFinished`: a node that built (`Succeeded`)
                // shines unless the renderer is drawing quickly.
                if state == NodeState::Built && error.is_none() && self.render_quality == RenderQuality::Normal {
                    self.shines.push(node);
                }
                let status = NodeStatus {
                    state,
                    building: false,
                    milliseconds,
                    error,
                    built_at: Some(Instant::now()),
                    range,
                };
                self.statuses.insert(node, status);
                self.build_done += 1;
                let total = self.build_total.max(1);
                self.build_progress = (self.build_done as f32 / total as f32).min(1.0);
            }
            BuildEvent::MemberUpdates(updates) => {
                for (node, member, value) in updates {
                    self.document.apply_build_update(node, &member, value);
                }
            }
            BuildEvent::Superseded => {}
            BuildEvent::Cancelled { generation } => {
                if generation != self.generation {
                    return;
                }
                self.cancelled_generation = generation;
                self.building = false;
                if let Some(started) = self.build_started.take() {
                    self.last_build_seconds = started.elapsed().as_secs_f32();
                }
                self.report("Preview build cancelled.");
            }
            BuildEvent::Baked { nodes, error } => self.finish_bake(nodes, error),
            BuildEvent::DiskCache(message) => self.report(message),
            BuildEvent::Progress(progress) => self.take_progress(progress),
            BuildEvent::Watched(watched) => {
                self.watched_outputs.insert(watched.node, watched);
                self.watched_revision += 1;
            }
            BuildEvent::NodeCommandDone { node, name, property, value } => {
                // The command's button text names the edit.
                let type_name = self.document.node(node).map(|data| data.type_name.clone()).unwrap_or_default();
                let spec = nerthus_engine::Catalogue::get().node(&type_name);
                let parameter = spec.and_then(|spec| spec.parameter(&name));
                let label = parameter.map(|parameter| parameter.display_name().to_string()).unwrap_or(name);
                self.document.set_value_as(node, &property, value, &label);
                self.changed();
            }
            BuildEvent::Finished { generation, target, outputs, underlay, custom_underlay, custom_colours, port_intents, error, cache_bytes, node_caches } => {
                self.cache_bytes = cache_bytes;
                let built_intents = node_caches.iter().map(|(id, cache)| (*id, cache.port_intents.clone())).collect();
                self.document.set_built_intents(built_intents);
                self.node_caches = node_caches;
                if generation != self.generation {
                    return;
                }
                // `Build_BuildFinished`: the preview stands up again.
                self.flat_preview = false;
                self.building = false;
                self.build_progress = 1.0;
                if let Some(started) = self.build_started.take() {
                    self.last_build_seconds = started.elapsed().as_secs_f32();
                }
                if Some(target) != self.preview_node {
                    return;
                }
                self.outputs = outputs;
                self.underlay = underlay;
                self.custom_underlay = custom_underlay;
                self.custom_colours = custom_colours;
                self.port_intents = port_intents;
                self.preview_error = error;
                self.show_preview();
            }
        }
    }

    /// The previewed node and port, with what its last build left.
    pub fn preview_source(&self) -> RenderSource<'_> {
        RenderSource {
            node: self.preview_node,
            port: &self.preview_port,
            outputs: &self.outputs,
            underlay: self.underlay.as_ref(),
            custom_underlay: self.custom_underlay.as_ref(),
            custom_colours: self.custom_colours.as_ref(),
            port_intents: &self.port_intents,
        }
    }

    /// How the viewport shows a port's map: its own intent, or for
    /// `Default` the one the node type declares for the port (ports read
    /// from a file carry none).
    fn display_intent(&self, source: &RenderSource, map: &Map) -> RenderIntent {
        let intent = map.info.render_intent;
        if intent != RenderIntent::Default {
            return intent;
        }
        let node = source.node.and_then(|id| self.document.terrain().nodes.get(&id));
        let spec = node.and_then(|node| nerthus_engine::Catalogue::get().node(&node.type_name));
        let port = spec.and_then(|spec| spec.ports.iter().find(|port| port.name == source.port));
        let declared = port.and_then(|port| port.render_intent.as_deref()).and_then(RenderIntent::from_name);
        declared.unwrap_or(intent)
    }

    /// `RendererBase.GetRender`'s intent: the node's `RenderIntentOverride`
    /// when it has one, else the port's as its node's last build left it.
    /// Without that (a node not built here), the map's own or the declared
    /// intent, a map of three channels whose port declares nothing shown
    /// as colours.
    fn render_intent(&self, source: &RenderSource, map: &Map) -> RenderIntent {
        let node = source.node.and_then(|id| self.document.terrain().nodes.get(&id));
        let saved_override = node.and_then(|node| node.base.get("RenderIntentOverride")).and_then(serde_json::Value::as_str);
        let intent_override = saved_override.and_then(RenderIntent::from_name).unwrap_or_default();
        if intent_override != RenderIntent::Default {
            return intent_override;
        }
        // The port's intent as its last build left it (`Port.RenderIntent`).
        if let Some(live) = source.port_intents.get(source.port) {
            return *live;
        }
        let intent = self.display_intent(source, map);
        if intent == RenderIntent::Default && map.channel_count() >= 3 {
            return RenderIntent::Color;
        }
        intent
    }

    /// Whether a node shows its own colours (`NodeMetadata.HasCustomColorization`),
    /// unless the 2D view shows another of its ports (`B.SecondaryDataDisplay`).
    fn shows_custom_colours(&self, source: &RenderSource) -> bool {
        if source.port != "Out" {
            return false;
        }
        let node = source.node.and_then(|id| self.document.terrain().nodes.get(&id));
        let spec = node.and_then(|node| nerthus_engine::Catalogue::get().node(&node.type_name));
        spec.is_some_and(|spec| spec.has_flag("CustomColorization"))
    }

    /// Whether a node is shown over a map of its own choosing
    /// (`NodeMetadata.HasCustomUnderlay`).
    fn has_custom_underlay(&self, source: &RenderSource) -> bool {
        let node = source.node.and_then(|id| self.document.terrain().nodes.get(&id));
        let spec = node.and_then(|node| nerthus_engine::Catalogue::get().node(&node.type_name));
        spec.is_some_and(|spec| spec.custom_underlay)
    }

    /// Whether the previewed port shows as a heightfield (`Node.IsHeightfield`
    /// for the main output).
    pub fn previews_heightfield(&self) -> bool {
        let source = self.preview_source();
        let Some(map) = source.map() else {
            return false;
        };
        let intent = self.render_intent(&source, &map);
        matches!(intent, RenderIntent::Default | RenderIntent::Heightfield)
    }

    /// Renders the previewed port into the viewport once the renderer gets
    /// to it (`B.Rerender`).
    pub fn show_preview(&mut self) {
        self.needs_render = true;
    }

    /// Whether a render has reached the 3D view yet.
    pub fn rendered(&self) -> bool {
        self.shown_render > 0
    }

    /// Whether the preview has been sent to the renderer yet.
    pub fn render_requested(&self) -> bool {
        self.render_generation > 0
    }

    /// `RendererBase.GetRender(node, port)`'s inputs. `ResolveHeightfield`:
    /// a heightfield shapes the terrain itself, unless its node has an
    /// underlay of its own (Mixer: the editor's underlay, else what feeds
    /// its Terrain port), which shapes it instead; another port of a node
    /// whose main output is a heightfield lies over that output; anything
    /// else lies over the underlay, else a flat map.
    /// `ResolveColorfield`: masks, colours and raw data are draped, and a
    /// node with its own colours (`HasCustomColorization`) drapes those
    /// instead, or nothing when it has none.
    fn render_job(&mut self) -> Option<RenderJob> {
        self.render_generation += 1;
        let generation = self.render_generation;
        let job = self.render_job_for(&self.preview_source(), generation)?;
        self.rendered_quickly = job.lit_quickly();
        Some(job)
    }

    /// The renderer's quality (`B.Current.Quality`): `Fast` while a gizmo
    /// or Draw's painter is held, `Normal` once it is let go, when a
    /// preview lit quickly is rendered again.
    pub fn set_render_quality(&mut self, quality: RenderQuality) {
        if self.render_quality == quality {
            return;
        }
        self.render_quality = quality;
        if quality == RenderQuality::Normal && self.rendered_quickly {
            self.show_preview();
        }
    }

    pub fn render_quality(&self) -> RenderQuality {
        self.render_quality
    }

    /// The render job for any node and port (a floating 2D view's too).
    pub fn render_job_for(&self, source: &RenderSource, generation: u64) -> Option<RenderJob> {
        let map = source.map()?;
        let intent = self.render_intent(source, &map);
        let shows_heights = matches!(intent, RenderIntent::Default | RenderIntent::Heightfield);
        let main_intent = source.port_intents.get("Out").copied().unwrap_or_default();
        let main_shows_heights = matches!(main_intent, RenderIntent::Default | RenderIntent::Heightfield);
        let main_output = source.outputs.iter().find(|(port, _)| port == "Out").map(|(_, main)| main.clone());
        // A node's own underlay replaces its own heights, even when that
        // underlay is missing.
        let own = shows_heights.then(|| map.clone());
        let resolved = if self.has_custom_underlay(source) { source.custom_underlay.cloned() } else { own };
        let underlay = if source.port != "Out" && main_shows_heights && main_output.is_some() {
            main_output
        } else {
            resolved.or_else(|| source.underlay.cloned())
        };
        let heightfield = underlay.unwrap_or_else(|| Arc::new(Map::new(map.resolution(), 1)));
        let drapes = matches!(intent, RenderIntent::Mask | RenderIntent::Color | RenderIntent::Raw);
        let mut colorfield = drapes.then(|| Colorfield::Port(map.clone()));
        if self.shows_custom_colours(source) {
            colorfield = source.custom_colours.cloned().map(Colorfield::Custom);
        }
        let terrain = self.document.terrain();
        let width = if terrain.width > 0.0 { terrain.width } else { 5000.0 };
        let height = if terrain.height > 0.0 { terrain.height } else { 2500.0 };
        let job = RenderJob {
            generation,
            heightfield,
            colorfield,
            intent,
            mode: self.render_mode,
            quality: self.render_quality,
            mask_enhancement: self.mask_enhancement,
            mask_gradient: self.mask_gradient,
            lighting: self.lighting,
            terrain: (width, height),
            overlay: self.overlay_job(),
        };
        Some(job)
    }

    /// Takes in the newest render: the terrain and its colours, shown
    /// together as `B.TransmitTerrain` sends them. Nothing is sent when a
    /// render has no colours.
    fn show_render(&mut self, renderer: &ViewportRenderer, preview: &mut TerrainPreview) {
        let Some(view) = renderer.poll() else {
            return;
        };
        if view.generation <= self.shown_render {
            return;
        }
        self.shown_render = view.generation;
        let Some(colours) = view.colours else {
            return;
        };
        // `Comms.SetHeight(Terrain.Ratio)`: a previewed region stands as
        // tall against its width as the whole terrain against the terrain's.
        let terrain = self.document.terrain();
        let width = if terrain.width > 0.0 { terrain.width } else { 5000.0 };
        let height = if terrain.height > 0.0 { terrain.height } else { 2500.0 };
        let region = self.previewed_region();
        preview.width = width;
        preview.height = nerthus_engine::region::terrain_ratio(width, height, region.as_ref()) * width;
        // Laid flat (V), the terrain keeps its colours on a plane
        // (`TransmitTerrain` of a black map).
        let heights = if self.flat_preview { Arc::new(Map::new(view.heights.resolution(), 1)) } else { view.heights };
        preview.heights = Some(heights);
        self.render_colours = Some(colours.clone());
        preview.colours = Some(colours);
        preview.revision += 1;
    }

    /// The cleared viewport (`B.ClearViewport`): a flat map at the preview
    /// resolution, in the colour the clearing renderer lights it.
    fn show_cleared(&mut self, preview: &mut TerrainPreview) {
        let Some(lighting) = self.pending_clear.take() else {
            return;
        };
        let resolution = self.preview_resolution;
        let colours = Arc::new(crate::viewport_render::cleared_colours(&lighting, resolution));
        preview.heights = Some(Arc::new(Map::new(Resolution::square(resolution), 1)));
        self.render_colours = Some(colours.clone());
        preview.colours = Some(colours);
        preview.revision += 1;
    }

    /// The renderer and lighting the project was saved with
    /// (`TerrainState.Viewport`); a project's renderer starts without mask
    /// enhancement, as `SetRender…` makes a new one.
    fn load_viewport_state(&mut self) {
        let viewport = self.document.viewport_state();
        if let Some(viewport) = viewport {
            let saved_mode = viewport.get("RenderMode");
            if let Some(mode) = RenderMode::from_state(saved_mode) {
                self.render_mode = mode;
            }
        }
        self.lighting = ViewportLighting::from_state(viewport);
        self.overlay = crate::overlays::OverlaySettings::from_state(viewport);
        self.mask_enhancement = MaskEnhancement::None;
    }

    /// `ToggleFlatView` (V): with a node previewed, the preview lies flat or
    /// stands up again.
    pub fn toggle_flat_view(&mut self) {
        if self.preview_node.is_none() {
            return;
        }
        self.flat_preview = !self.flat_preview;
        self.show_preview();
    }

    /// What renders lay over themselves (`B.ShowOverlay` once the opacity
    /// is above 0%), cut to the previewed region.
    fn overlay_job(&self) -> Option<crate::overlays::OverlayJob> {
        if self.overlay.opacity_value() == 0.0 {
            return None;
        }
        let region = self.previewed_region();
        let portion = region.map(|region| [region.position[0], region.position[1], region.width]);
        let job = crate::overlays::OverlayJob { settings: self.overlay.clone(), portion };
        Some(job)
    }

    /// `OverlayOpacityChanged`, `OverlayChanged`: the project notes the
    /// overlay and the view renders again (`Node.Build(sky)`).
    pub fn set_overlay(&mut self, overlay: crate::overlays::OverlaySettings) {
        if self.overlay == overlay {
            return;
        }
        self.document.edit_viewport_state(|viewport| overlay.write_to(viewport));
        self.document.dirty = true;
        self.overlay = overlay;
        self.show_preview();
    }

    /// `SetRenderRaw`, `SetRenderClay`, `SetRenderRealistic`: a new renderer
    /// (so no mask enhancement), noted in the project, and a render.
    pub fn set_render_mode(&mut self, mode: RenderMode) {
        self.render_mode = mode;
        self.mask_enhancement = MaskEnhancement::None;
        self.document.edit_viewport_state(|viewport| crate::viewport_render::write_render_mode(viewport, mode));
        self.show_preview();
    }

    /// `MaskPostNone`, `MaskPostAutolevel`, `MaskPostEqualize`: nothing for
    /// a heightfield's main output.
    pub fn set_mask_enhancement(&mut self, enhancement: MaskEnhancement) {
        if self.previews_heightfield() {
            return;
        }
        self.mask_enhancement = enhancement;
        self.document.edit_viewport_state(|viewport| crate::viewport_render::write_autolevel_masks(viewport, enhancement));
        self.show_preview();
    }

    /// The lighting panel changed (`SkyEditor_Updated`, `UpdateSky`): the
    /// project notes it and the view renders again.
    pub fn set_lighting(&mut self, lighting: ViewportLighting) {
        if self.lighting == lighting {
            return;
        }
        self.lighting = lighting;
        self.document.edit_viewport_state(|viewport| lighting.write_to(viewport));
        self.document.dirty = true;
        self.show_preview();
    }

    /// The lighting panel's mask gradient (`PrivateSky.Build` sets
    /// `B.ClutMap`).
    pub fn set_mask_gradient(&mut self, gradient: MaskGradient) {
        if self.mask_gradient == gradient {
            return;
        }
        self.mask_gradient = gradient;
        self.show_preview();
    }

    /// `ResetCamera` (Reset to Loaded): back to the project's saved camera,
    /// when it has one.
    pub fn reset_camera_to_loaded(&mut self) {
        let saved = self.document.saved_camera();
        if let Some(camera) = viewport_camera::camera_from_message(&saved) {
            self.camera_request = Some(camera);
        }
    }

    /// `DefaultCamera` (Reset to Default): the starting view, which the
    /// project then keeps as its camera.
    pub fn reset_camera_to_default(&mut self) {
        let camera = CameraValues::DEFAULT;
        let message = viewport_camera::camera_message(&camera);
        self.document.set_saved_camera(&message);
        self.camera_request = Some(camera);
    }

    /// `LoadCamera(filename)`: a camera file's view, which the project
    /// then keeps as its camera.
    fn load_camera_file(&mut self, path: &Path) {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) => {
                self.report(format!("Cannot read {}: {error}", path.display()));
                return;
            }
        };
        let lines: Vec<&str> = text.lines().collect();
        match viewport_camera::camera_from_lines(&lines) {
            Ok(camera) => {
                let message = viewport_camera::camera_message(&camera);
                self.document.set_saved_camera(&message);
                self.camera_request = Some(camera);
            }
            Err(problem) => self.report(format!("Cannot load the camera: {problem}")),
        }
    }

    /// `SaveCamera(data)`: the camera's values, a line each.
    fn save_camera_file(&mut self, path: &Path, camera: &CameraValues) {
        let lines = viewport_camera::camera_file_lines(camera);
        let mut text = lines.join("\r\n");
        text.push_str("\r\n");
        if let Err(error) = std::fs::write(path, text) {
            self.report(format!("Cannot save {}: {error}", path.display()));
        }
    }

    /// `AutoScreenshot` (F9): the 3D view straight into the screenshots
    /// folder as `<project>_<yyyy-MM-dd_HH-mm-ss>.png`.
    pub fn save_screenshot(&mut self) {
        let folder = self.user.preferences.screenshots_folder();
        if let Err(error) = std::fs::create_dir_all(&folder) {
            self.report(format!("Cannot make {}: {error}", folder.display()));
            return;
        }
        let file_name = self.document.file_name();
        let stem = Path::new(&file_name).file_stem();
        let project = stem.map(|stem| stem.to_string_lossy().to_string()).unwrap_or_default();
        let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
        let path = folder.join(format!("{project}_{stamp}.png"));
        let request = ScreenshotRequest::Viewport { path, supersample: self.screenshot_supersample };
        self.screenshots.push(request);
    }

    /// Open Screenshots Folder.
    pub fn open_screenshots_folder(&mut self) {
        let folder = self.user.preferences.screenshots_folder();
        if let Err(error) = std::fs::create_dir_all(&folder) {
            self.report(format!("Cannot open {}: {error}", folder.display()));
            return;
        }
        crate::export_build::open_folder(&folder);
    }

    /// Where camera files are looked for: the project's folder.
    fn camera_folder(&self) -> Option<PathBuf> {
        let folder = self.document.path.as_ref().and_then(|path| path.parent());
        folder.filter(|folder| folder.is_dir()).map(Path::to_path_buf)
    }
}

/// A node and port to render, with what the node's last build left
/// (`RendererBase.GetRender(node, port)`'s inputs).
pub struct RenderSource<'a> {
    pub node: Option<i32>,
    pub port: &'a str,
    /// The node's maps by port.
    pub outputs: &'a [(String, Arc<Map>)],
    /// The heightfield colour and mask outputs drape over.
    pub underlay: Option<&'a Arc<Map>>,
    /// What a type with its own underlay is shown over
    /// (`Node.GetCustomUnderlay`).
    pub custom_underlay: Option<&'a Arc<Map>>,
    /// `Node.GetCustomColorMap()`, for types with their own colours.
    pub custom_colours: Option<&'a Arc<Map>>,
    pub port_intents: &'a PortIntents,
}

impl RenderSource<'_> {
    /// The port's map, else the node's first.
    pub fn map(&self) -> Option<Arc<Map>> {
        let chosen = self.outputs.iter().find(|(port, _)| port == self.port);
        let first = self.outputs.first();
        chosen.or(first).map(|(_, map)| map.clone())
    }
}

/// `debounceInterval`: how long a parameter edit waits for the next.
const DEBOUNCE: Duration = Duration::from_millis(100);

/// `GetDebounceLimit`: no wait for a node that took under 100 ms, 100 ms up
/// to a second, 500 up to five and 1000 past that.
fn adaptive_debounce(build_milliseconds: f32) -> Duration {
    let wait = if build_milliseconds < 100.0 {
        0
    } else if build_milliseconds > 5000.0 {
        1000
    } else if build_milliseconds > 1000.0 {
        500
    } else {
        100
    };
    Duration::from_millis(wait)
}

/// The project's preview resolution (`State.PreviewResolution`), 1024 if
/// it has none.
fn preview_resolution_of(document: &Document) -> usize {
    let state = document.project.assets[0].extra.get("State");
    let saved = state.and_then(|state| state.get("PreviewResolution")).and_then(|value| value.as_u64()).map(|value| value as usize);
    saved.filter(|value| PREVIEW_RESOLUTIONS.contains(value)).unwrap_or(1024)
}

pub struct EditorPlugin {
    pub options: LaunchOptions,
}

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        let worker = BuildWorker::start();
        let renderer = ViewportRenderer::start();
        let mut editor = Editor::new(Document::new());
        editor.user = User::load(self.options.safe_mode);
        if let Some(address) = self.options.proxy.as_deref()
            && !self.options.safe_mode
        {
            editor.user.set_proxy(address);
        }
        // `SetupServices`: the presets in `Paths.Presets`.
        let presets_folder = editor.user.preferences.documents_folder().join("Presets");
        editor.presets = crate::presets::PresetLibrary::load(presets_folder);
        crate::files::start(&mut editor, self.options.start_dialog());
        editor.compute = ComputeState::start(self.options.cpu_only, self.options.safe_mode, &editor.user);
        editor.mask_gradient = MaskGradient::from_index(editor.user.options.mask_gradient());
        editor.load_macros();
        if let Some(path) = &self.options.project {
            editor.open(path);
        }
        if let Some(query) = &self.options.preview {
            let lowered = query.to_lowercase();
            let nodes = &editor.document.terrain().nodes;
            let found = nodes.values().find(|node| node.name.to_lowercase() == lowered || node.type_name.to_lowercase() == lowered || node.id.to_string() == lowered).map(|node| node.id);
            match found {
                Some(id) => editor.select(Some(id)),
                None => editor.report(format!("No node named {query}")),
            }
        }
        // The preview builds; opening may have asked for a request already
        // (its preparation: bakes loaded, previews read back).
        editor.needs_build |= editor.preview_node.is_some();
        if self.options.show.as_deref() == Some("build") {
            editor.start_export_build(false);
        }
        // Bake Required Nodes as if confirmed (for checking baking without
        // clicking through the menu).
        if self.options.show.as_deref() == Some("bake-required") {
            editor.bake_required();
        }
        app.insert_resource(editor);
        app.insert_resource(worker);
        app.insert_resource(renderer);
        app.add_systems(Update, (process_builds, update_window_title));
        app.add_systems(Update, show_dialogs);
    }
}

fn process_builds(
    mut editor: ResMut<Editor>,
    worker: Res<BuildWorker>,
    renderer: Res<ViewportRenderer>,
    mut preview: ResMut<TerrainPreview>,
    mut redraw: bevy::ecs::message::MessageWriter<bevy::window::RequestRedraw>,
) {
    let editing = editor.as_mut();
    editing.compute.poll(&mut editing.user);
    for event in worker.poll() {
        editor.handle_event(event);
    }
    editor.poll_export_build();
    if std::mem::take(&mut editor.cancel_build) {
        worker.cancel();
    }
    if editor.build_due.is_some_and(|due| Instant::now() >= due) {
        editor.build_due = None;
        editor.needs_build = editor.preview_node.is_some();
    }
    // Frames keep coming while a debounced build waits, so it goes when due.
    if editor.build_due.is_some() {
        redraw.write(bevy::window::RequestRedraw);
    }
    if editor.needs_build {
        editor.needs_build = false;
        editor.request_build(&worker);
    }
    if editor.needs_render {
        editor.needs_render = false;
        if let Some(job) = editor.render_job() {
            renderer.request(job);
        }
    }
    editor.show_render(&renderer, &mut preview);
    editor.show_cleared(&mut preview);
    // The Region Manager's "Loading preview" lasts until the render is in
    // (or a build means there is none to wait for).
    if editor.render_settled() || editor.building {
        editor.region_preview_ready();
    }
    if preview.tiled != editor.tiled_preview {
        preview.tiled = editor.tiled_preview;
        preview.revision += 1;
    }
    // Conserve memory when displaying 4K terrains (`SendViewportPreferences`).
    let conserve_memory = editor.user.options.prevent_hi_res();
    if preview.conserve_memory != conserve_memory {
        preview.conserve_memory = conserve_memory;
    }
}

fn update_window_title(editor: Res<Editor>, mut window: Single<&mut Window, With<bevy::window::PrimaryWindow>>) {
    if !editor.is_changed() {
        return;
    }
    let title = editor.window_title();
    if window.title != title {
        window.title = title;
    }
}

/// File dialogs block and must run on the main thread (macOS).
#[cfg(not(target_arch = "wasm32"))]
fn show_dialogs(_main_thread: NonSendMarker, mut editor: ResMut<Editor>) {
    let Some(request) = editor.dialog.take() else {
        return;
    };
    match request {
        DialogRequest::SaveAs => {
            let file_name = editor.document.file_name();
            let mut dialog = rfd::FileDialog::new();
            dialog = dialog.add_filter("Nerthus Project File (*.terrain)", &["terrain"]);
            dialog = dialog.set_file_name(file_name);
            if let Some(folder) = editor.user.locations.folder(crate::user::locations::SAVE) {
                dialog = dialog.set_directory(folder);
            }
            let pending_build = editor.pending_build.take();
            if let Some(path) = dialog.save_file() {
                let path = with_extension(path, "terrain");
                editor.save_to(&path);
                let saved = !editor.document.dirty;
                // The Build and Export choice goes on (its checks and
                // questions) now that there is a file.
                if let (true, Some(choice)) = (saved, pending_build) {
                    editor.resume_build = Some(choice);
                }
            }
        }
        DialogRequest::BrowseFile { node, parameter, extensions, folder, current } => {
            // `BrowseButton_Click`: all the formats together, then each
            // alone; it opens where the file (or folder) held is.
            let mut dialog = rfd::FileDialog::new();
            if !extensions.is_empty() {
                let names: Vec<&str> = extensions.iter().map(String::as_str).collect();
                let listed: Vec<String> = extensions.iter().map(|extension| format!(".{extension}")).collect();
                let all_label = format!("All Files ({})", listed.join(","));
                dialog = dialog.add_filter(all_label, &names);
                for extension in &names {
                    dialog = dialog.add_filter(format!("*.{extension}"), &[*extension]);
                }
            }
            let current = Path::new(&current);
            let held = if folder { current.is_dir() } else { current.is_file() };
            let parent = current.parent().filter(|parent| held && !parent.as_os_str().is_empty());
            if let Some(parent) = parent {
                dialog = dialog.set_directory(parent);
            }
            let picked = if folder { dialog.pick_folder() } else { dialog.pick_file() };
            let Some(path) = picked else {
                return;
            };
            let value = serde_json::Value::String(path.to_string_lossy().to_string());
            editor.document.set_value(node, &parameter, value);
            editor.changed();
        }
        DialogRequest::LoadCamera => {
            let mut dialog = rfd::FileDialog::new();
            dialog = dialog.set_title("Load Camera settings...");
            dialog = dialog.add_filter("Nerthus Camera", &["cam"]);
            if let Some(folder) = editor.camera_folder() {
                dialog = dialog.set_directory(folder);
            }
            if let Some(path) = dialog.pick_file() {
                editor.load_camera_file(&path);
            }
        }
        DialogRequest::SaveCamera { camera } => {
            let mut dialog = rfd::FileDialog::new();
            dialog = dialog.set_title("Save Camera settings as...");
            dialog = dialog.add_filter("Nerthus Camera", &["cam"]);
            if let Some(folder) = editor.camera_folder() {
                dialog = dialog.set_directory(folder);
            }
            if let Some(path) = dialog.save_file() {
                let path = with_extension(path, "cam");
                editor.save_camera_file(&path, &camera);
            }
        }
        DialogRequest::SaveScreenshot { graph_region, supersample } => {
            let title = if graph_region.is_some() { "Save Graph Screenshot" } else { "Save Viewport Screenshot" };
            let folder = editor.user.preferences.screenshots_folder();
            let mut dialog = rfd::FileDialog::new();
            dialog = dialog.set_title(title);
            dialog = dialog.add_filter("PNG (.png)", &["png"]);
            if folder.is_dir() {
                dialog = dialog.set_directory(&folder);
            }
            let Some(path) = dialog.save_file() else {
                return;
            };
            let path = with_extension(path, "png");
            let request = match graph_region {
                Some(region) => ScreenshotRequest::WindowRegion { path, region },
                None => ScreenshotRequest::Viewport { path, supersample },
            };
            editor.screenshots.push(request);
        }
        DialogRequest::CustomOverlay => {
            // `OverlayChanged` for Custom: any image that can be read; giving up
            // leaves the overlay as it was.
            let mut dialog = rfd::FileDialog::new();
            dialog = dialog.add_filter("All Image Formats", &crate::files::IMAGE_EXTENSIONS);
            let Some(path) = dialog.pick_file() else {
                return;
            };
            let mut overlay = editor.overlay.clone();
            overlay.image = crate::overlays::CUSTOM_OVERLAY;
            overlay.custom_file = path.display().to_string();
            if overlay.opacity == 0 {
                overlay.opacity = crate::overlays::SHOWN_OPACITY;
            }
            editor.set_overlay(overlay);
        }
        DialogRequest::Save2DView { colours } => {
            let mut dialog = rfd::FileDialog::new();
            dialog = dialog.set_title("Save 2D Preview");
            dialog = dialog.add_filter("PNG 8-bit (*.png)", &["png"]);
            let Some(path) = dialog.save_file() else {
                return;
            };
            let path = with_extension(path, "png");
            let saved = image::save_buffer(&path, &colours.rgb, colours.width as u32, colours.height as u32, image::ExtendedColorType::Rgb8);
            if let Err(error) = saved {
                editor.report(format!("Cannot save {}: {error}", path.display()));
            }
        }
        DialogRequest::ExportSynthColours { node } => {
            let mut dialog = rfd::FileDialog::new();
            dialog = dialog.set_title("Save Color Map to File");
            dialog = dialog.add_filter("JSON File (*.json)", &["json"]);
            dialog = dialog.add_filter("CSV File (*.csv)", &["csv"]);
            let Some(path) = dialog.save_file() else {
                return;
            };
            let path = with_extension(path, "json");
            let colours = editor.document.node(node).map(synth_colours).unwrap_or_default();
            if let Err(error) = crate::node_files::write_synth_colours(&path, &colours) {
                editor.report(format!("Failed to save file. {error}"));
            }
        }
        DialogRequest::BuildDestination { current } => {
            let mut dialog = rfd::FileDialog::new();
            dialog = dialog.set_title("Select the folder under which multiple build folders will be stored and organized.");
            let current_folder = PathBuf::from(&current);
            if !current.is_empty() && current_folder.is_dir() {
                dialog = dialog.set_directory(&current_folder);
            }
            let Some(folder) = dialog.pick_folder() else {
                return;
            };
            let value = serde_json::Value::String(folder.to_string_lossy().to_string());
            editor.document.set_build_setting("Destination", value);
        }
    }
}

/// The web build saves by downloading, so Save As and the 2D view's Save
/// to file name the download and ask nothing; the other dialogs choose
/// files and folders, which a browser page does not have.
#[cfg(target_arch = "wasm32")]
fn show_dialogs(mut editor: ResMut<Editor>) {
    let Some(request) = editor.dialog.take() else {
        return;
    };
    match request {
        DialogRequest::SaveAs => {
            // Build and Export does not follow: it writes to folders.
            editor.pending_build = None;
            let path = with_extension(PathBuf::from(editor.document.file_name()), "terrain");
            editor.save_to(&path);
        }
        DialogRequest::Save2DView { colours } => {
            let mut png = Vec::new();
            let encoder = image::codecs::png::PngEncoder::new(&mut png);
            let encoded = image::ImageEncoder::write_image(encoder, &colours.rgb, colours.width as u32, colours.height as u32, image::ExtendedColorType::Rgb8);
            let saved = encoded.map_err(|error| error.to_string()).and_then(|()| crate::web::write(Path::new("2D Preview.png"), &png).map_err(|error| error.to_string()));
            if let Err(error) = saved {
                editor.report(format!("Cannot save the 2D preview: {error}"));
            }
        }
        DialogRequest::BrowseFile { .. } | DialogRequest::CustomOverlay => editor.report(crate::web::unavailable("Reading files from disk")),
        DialogRequest::BuildDestination { .. } => editor.report(crate::web::unavailable("Build and Export")),
        DialogRequest::LoadCamera | DialogRequest::SaveCamera { .. } => editor.report(crate::web::unavailable("Loading and saving cameras")),
        DialogRequest::SaveScreenshot { .. } => editor.report(crate::web::unavailable("Saving screenshots")),
        DialogRequest::ExportSynthColours { .. } => editor.report(crate::web::unavailable("Exporting colours")),
    }
}

/// A Synth's colours (`Synth.Data`): red, green, blue and position each.
fn synth_colours(node: &nerthus_engine::NodeData) -> Vec<[f32; 4]> {
    let items = node.values.get("Data").and_then(serde_json::Value::as_array);
    let component = |item: &serde_json::Value, key: &str| item.get(key).and_then(serde_json::Value::as_f64).unwrap_or(0.0) as f32;
    let colours = items.map(|items| items.iter().map(|item| [component(item, "X"), component(item, "Y"), component(item, "Z"), component(item, "W")]).collect());
    colours.unwrap_or_default()
}

fn with_extension(path: PathBuf, extension: &str) -> PathBuf {
    if path.extension().is_some() {
        return path;
    }
    path.with_extension(extension)
}

/// Every node downstream of `node`, nearest first.
pub fn downstream_nodes(terrain: &nerthus_engine::Terrain, node: i32) -> Vec<i32> {
    let mut found = Vec::new();
    let mut pending = vec![node];
    let mut seen = std::collections::HashSet::from([node]);
    while let Some(current) = pending.pop() {
        for candidate in terrain.nodes.values() {
            let reads = candidate.ports.iter().any(|port| port.is_input() && port.record.as_ref().is_some_and(|record| record.from == current));
            if reads && seen.insert(candidate.id) {
                found.push(candidate.id);
                pending.push(candidate.id);
            }
        }
    }
    found
}

/// Every file under `from` copied to the same place under
/// `to`.
fn copy_folder(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_folder(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_debounce_follows_the_build_time() {
        assert_eq!(adaptive_debounce(40.0), Duration::ZERO);
        assert_eq!(adaptive_debounce(100.0), Duration::from_millis(100));
        assert_eq!(adaptive_debounce(1000.0), Duration::from_millis(100));
        assert_eq!(adaptive_debounce(1000.5), Duration::from_millis(500));
        assert_eq!(adaptive_debounce(5000.0), Duration::from_millis(500));
        assert_eq!(adaptive_debounce(9000.0), Duration::from_millis(1000));
    }

    #[test]
    fn deselecting_clears_the_viewport_unless_asked_not_to() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [26000.0, 26000.0]).unwrap();
        let perlin = document.add_node("Perlin", [26300.0, 26000.0]).unwrap();
        let mut editor = Editor::new(document);
        // A project starts cleared.
        assert!(editor.pending_clear.take().is_some());
        editor.select(Some(mountain));
        assert_eq!(editor.preview_node, Some(mountain));
        editor.select(None);
        assert_eq!(editor.preview_node, None);
        assert!(editor.pending_clear.take().is_some());
        // Several selected leave the preview too.
        editor.select(Some(perlin));
        assert!(!editor.viewport_cleared);
        editor.set_selection(vec![mountain, perlin]);
        assert_eq!(editor.preview_node, None);
        // A group chosen lets the nodes go but keeps the preview.
        editor.select(Some(perlin));
        editor.pending_clear = None;
        editor.select_group();
        assert!(editor.selection.is_empty());
        assert_eq!(editor.preview_node, Some(perlin));
        assert!(editor.pending_clear.is_none());
        // `DontClearViewport`: the last preview stays.
        editor.user.options.set_value("DontClearViewport", serde_json::json!(true));
        editor.select(Some(perlin));
        editor.select(None);
        assert_eq!(editor.preview_node, Some(perlin));
        assert!(editor.pending_clear.is_none());
        // A new project clears it whatever the options say.
        editor.set_document(Document::new());
        assert_eq!(editor.preview_node, None);
        assert!(editor.pending_clear.is_some());
    }

    /// A Mixer previewed, its Terrain fed by a Perlin, as a build of it
    /// would leave the editor (`underlay` the general one).
    fn mixer_editor() -> (Editor, i32, i32) {
        let mut document = Document::new();
        let base = document.add_node("Mountain", [26000.0, 26000.0]).unwrap();
        let colours = document.add_node("SatelliteMap", [26300.0, 26000.0]).unwrap();
        let relief = document.add_node("Perlin", [26300.0, 26300.0]).unwrap();
        let mixer = document.add_node("Mixer", [26600.0, 26000.0]).unwrap();
        document.connect(base, "Out", colours, "In");
        document.connect(colours, "Out", mixer, "In");
        document.connect(relief, "Out", mixer, "Terrain");
        let mut editor = Editor::new(document);
        editor.select(Some(mixer));
        let resolution = Resolution::square(4);
        let colour_map = Map::from_channels(resolution, vec![vec![0.5; 16]; 3]);
        editor.outputs = vec![("Out".to_string(), Arc::new(colour_map))];
        editor.port_intents = PortIntents::from([("Out".to_string(), RenderIntent::Color)]);
        editor.underlay = Some(Arc::new(Map::filled(resolution, 0.25)));
        (editor, mixer, colours)
    }

    #[test]
    fn a_mixer_lies_over_its_own_underlay_else_the_general_one() {
        let (mut editor, mixer, colours) = mixer_editor();
        assert_eq!(editor.preview_node, Some(mixer));
        let own = Arc::new(Map::filled(Resolution::square(4), 0.75));
        editor.custom_underlay = Some(own.clone());
        let job = editor.render_job_for(&editor.preview_source(), 1).unwrap();
        assert!(Arc::ptr_eq(&job.heightfield, &own));

        // Its own underlay missing: the general one.
        editor.custom_underlay = None;
        let job = editor.render_job_for(&editor.preview_source(), 2).unwrap();
        assert!(Arc::ptr_eq(&job.heightfield, editor.underlay.as_ref().unwrap()));

        // Other types never look at it.
        editor.custom_underlay = Some(own);
        let mut source = editor.preview_source();
        source.node = Some(colours);
        let job = editor.render_job_for(&source, 3).unwrap();
        assert!(Arc::ptr_eq(&job.heightfield, editor.underlay.as_ref().unwrap()));
    }

    #[test]
    fn a_project_reopens_on_its_selection_and_preview_lock() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [26000.0, 26000.0]).unwrap();
        let erosion = document.add_node("ErosionPro", [26300.0, 26000.0]).unwrap();
        document.connect(mountain, "Out", erosion, "In");
        let mut editor = Editor::new(document);
        // Nothing was saved selected: nothing is.
        assert!(editor.selection.is_empty());
        assert_eq!(editor.preview_node, None);

        editor.select(Some(mountain));
        editor.lock_preview(mountain);
        assert!(editor.preview_locked);
        editor.select(Some(erosion));
        assert_eq!(editor.preview_node, Some(mountain));

        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|duration| duration.as_nanos()).unwrap_or(0);
        let path = std::env::temp_dir().join(format!("nerthus-reopen-{nanos}.terrain"));
        editor.save_to(&path);
        let reopened = Editor::new(Document::open(&path).unwrap());
        std::fs::remove_file(&path).ok();
        assert_eq!(reopened.selection, vec![erosion]);
        assert!(reopened.preview_locked);
        assert_eq!(reopened.preview_node, Some(mountain));

        // Undoing the lock frees the preview for the selection.
        editor.document.undo();
        editor.follow_preview_lock();
        assert!(!editor.preview_locked);
        assert_eq!(editor.preview_node, Some(erosion));
    }

    #[test]
    fn a_quick_render_is_made_again_once_nothing_is_held() {
        let (mut editor, _, _) = mixer_editor();
        editor.render_mode = RenderMode::Realistic;
        editor.set_render_quality(RenderQuality::Fast);
        let job = editor.render_job().unwrap();
        assert_eq!(job.quality, RenderQuality::Fast);
        editor.needs_render = false;
        editor.set_render_quality(RenderQuality::Normal);
        assert!(editor.needs_render);
        assert_eq!(editor.render_job().unwrap().quality, RenderQuality::Normal);

        // Clay ignores the quality: nothing to make again.
        editor.render_mode = RenderMode::Clay;
        editor.set_render_quality(RenderQuality::Fast);
        editor.render_job().unwrap();
        editor.needs_render = false;
        editor.set_render_quality(RenderQuality::Normal);
        assert!(!editor.needs_render);
    }

    #[test]
    fn bakes_are_refused_for_bypassed_nodes_and_macros_upstream() {
        let mut document = Document::new();
        let macro_node = document.add_node("Macro", [26000.0, 26000.0]).unwrap();
        let gate = document.add_node("Gate", [26300.0, 26000.0]).unwrap();
        let mountain = document.add_node("Mountain", [26000.0, 26400.0]).unwrap();
        let blur = document.add_node("Blur", [26300.0, 26400.0]).unwrap();
        document.connect(macro_node, "Out", gate, "In");
        document.connect(mountain, "Out", blur, "In");
        let mut editor = Editor::new(document);
        assert_eq!(editor.bake_selected_refusal(macro_node), Some(BakeRefusal::MacroNode));
        assert_eq!(editor.bake_selected_refusal(gate), Some(BakeRefusal::MacroInChain));
        assert_eq!(editor.bake_selected_refusal(blur), None);

        // The node itself counts as upstream.
        editor.document.set_member(blur, "StateOverride", serde_json::json!("Bypassed"), "Bypass", true);
        assert_eq!(editor.bake_selected_refusal(blur), Some(BakeRefusal::BypassedChain));
        editor.document.set_member(blur, "StateOverride", serde_json::json!("None"), "Bypass", true);
        editor.document.set_member(mountain, "StateOverride", serde_json::json!("Bypassed"), "Bypass", true);
        assert_eq!(editor.bake_selected_refusal(blur), Some(BakeRefusal::BypassedChain));

        // Node by node: the first node with a problem names it.
        assert_eq!(editor.bake_chain_refusal(&[gate, blur]), Some(BakeRefusal::MacroInChain));
        assert_eq!(editor.bake_chain_refusal(&[blur, gate]), Some(BakeRefusal::BypassedChain));
        assert_eq!(editor.bake_chain_refusal(&[]), None);
    }
}
