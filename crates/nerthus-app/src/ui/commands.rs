//! What menus, toolbars and shortcuts ask for. Commands collect while the
//! interface draws and run afterwards, when the window and the editor are
//! free to change.

use bevy::prelude::Window;
use serde_json::Value;

use crate::document::DeleteRequest;
use crate::editor::{BakeRefusal, DialogRequest, Editor, RenderMode};
use crate::export_build::{BuildChoice, BuildStart};
use crate::viewport_render::{MaskEnhancement, MaskGradient, ViewportLighting};
use crate::ui::build_settings::{BuildSettingsState, SettingsTab};
use crate::ui::build_window::BuildConfirm;
use crate::ui::data_view::{DataTab, FilterCriteria};
use crate::ui::graph_commands::GraphCommand;
use crate::ui::options_window::{OptionsState, OptionsTab};
use crate::ui::prompt::Prompt;
use crate::ui::UiState;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    NewProject,
    Open,
    Save,
    SaveAs,
    Exit,
    Undo,
    Redo,
    Delete,
    Duplicate,
    SelectAll,
    Rename,
    SelectUpstream,
    SelectDownstream,
    ZoomExtents,
    ZoomReset,
    ZoomSelection,
    ToggleGrid,
    ToggleSnapToGrid,
    Refresh,
    RefreshAll,
    Bypass,
    LockPreview,
    MarkForExport,
    ResetNode,
    SetPreviewResolution(usize),
    SetRenderMode(RenderMode),
    /// The viewport's Mask Enhancement.
    SetMaskEnhancement(MaskEnhancement),
    /// The lighting panel's lighting, mask gradient and sky.
    SetLighting(ViewportLighting),
    SetMaskGradient(MaskGradient),
    /// The lighting panel's Reset Sky (the panel turns the sky off).
    ResetSky,
    /// Viewport Options: Load Camera, Save Camera, Reset to Loaded and
    /// Reset to Default.
    LoadCamera,
    SaveCamera,
    ResetCameraToLoaded,
    ResetCameraToDefault,
    /// Save Screenshot (F9), Save Viewport Screenshot as… (Ctrl F9), Save
    /// Graph Screenshot as…, Open Screenshots Folder, and the 3D view's
    /// screenshot resolution (1x, 2x, 4x).
    SaveScreenshot,
    SaveViewportScreenshotAs,
    SaveGraphScreenshotAs,
    OpenScreenshotsFolder,
    SetScreenshotScale(u32),
    ToggleEngine,
    ClearCache,
    /// `CycleNodeSizes`: a node double-clicked.
    CycleNodeSize(i32),
    /// `ToggleExpansion` (F11): every selected node's size cycles.
    ToggleExpansion,
    SetRenderIntent(&'static str),
    AddNode { type_name: String, position: Option<[f32; 2]> },
    /// A node's command parameter (`PropertyArea.Editor_CommandExecuted`).
    NodeAction(String),
    /// A modifier toolbar toggle or Add Modifiers entry.
    ToggleModifier(&'static str),
    /// A modifier's close button: the modifier at that place in the list
    /// goes, after "Delete Modifier?" unless Shift was held.
    RemoveModifier { node: i32, place: usize, ask: bool },
    /// The Gate's Unbake Node (`PropertyArea.Editor_CommandExecuted`):
    /// asked first, then the node and its baked descendants unbake (the
    /// file is not saved).
    UnbakeNode(i32),
    UnbakeNodeConfirmed(i32),
    /// A baked node's Load Offloaded Cache: its bake and its baked
    /// descendants' read back from disk.
    LoadOffloadedCache(i32),
    AddNote,
    NewGraphTab,
    RenameGraphTab(usize, String),
    SetGraphTabColor(usize, &'static str),
    DeleteGraphTab(usize),
    WindowMinimize,
    WindowMaximize,
    WindowDrag,
    /// Selects one node (and shows it).
    SelectNode(i32),
    /// Makes the node the underlay, or stops it being one.
    SetUnderlay(i32),
    /// `GraphControl.ToggleUnderlay` (G): the same for the selected node.
    ToggleUnderlay,
    /// `MiIgnoreUnderlay_Click`: the selected node's `IgnoreUnderlay`.
    ToggleIgnoreUnderlay,
    /// `DataViewArea.ShowTab`: opens the Data View on a tab.
    ShowDataTab(DataTab),
    /// The node menu's Bookmark: the selected node in or out.
    ToggleBookmark,
    /// Bookmarks > Add bookmark: the selected node.
    AddBookmark,
    /// A bookmark clicked: its node selected and framed.
    GoToBookmark(i32),
    /// A bookmark right-clicked: it reads "Remove?" until clicked.
    ArmBookmarkRemoval(i32),
    RemoveBookmark(i32),
    /// The node menu's Copy and Paste Properties.
    CopyProperties,
    PasteProperties,
    /// File > Open Recent.
    OpenRecent(std::path::PathBuf),
    /// Ctrl Shift O: the latest recent file.
    OpenLatest,
    /// Tools > Interface Scale.
    SetUiScale(f32),
    /// Project > Build and Export: the confirmation.
    BuildAndExport,
    /// Preview › Clear Cache, once confirmed; Clean Memory's dialog
    /// (Optimize Memory) and what its choices run: unused memory handed
    /// back, the view's renders dropped.
    ClearCacheConfirmed,
    /// Bake › Cache to Disk: When closing file, When Idle, Delete Cache,
    /// and Reload SmartCache to RAM.
    TogglePersistOnSave,
    ToggleFreezeWhenIdle,
    DeleteDiskCache,
    ReloadSmartCache,
    CleanMemory,
    ReleaseMemory,
    DropViewportCache,
    /// The confirmation's Start Build, or Close Nerthus and Build: saved,
    /// checked, then offered Save cached build when the editor holds what
    /// the build would make.
    StartBuild { open_folder: bool, close: bool },
    /// The build itself, once checked (here, or in `nerthus-build` as the
    /// editor closes), Save cached build, and the normal build offered
    /// when that export fails.
    RunBuild { close: bool },
    ExportCachedBuild { close: bool },
    CachedExportFailed { close: bool },
    /// Project › Execute Batch Build… (`MnuBatchBuild_OnClick`).
    BatchBuild,
    CancelBuild,
    CloseBuildWindow,
    BuildSettings,
    OpenBuildFolder,
    OpenFolder(std::path::PathBuf),
    /// Graph › Baking: Bake Selected, Bake Required Nodes, Force load
    /// baked cache, Unbake Selected, Unbake All, the bake resolution; and
    /// what their questions' Yes runs.
    BakeSelected,
    BakeRequiredNodes,
    BakeRequiredNodesConfirmed,
    /// The memory warning's Yes: the bake goes ahead (with Bake Selected's
    /// progress window when `selected`).
    BakeAnyway { nodes: Vec<i32>, selected: bool },
    /// "Cannot Start Baking"'s Filter Bypassed nodes: the Data View
    /// filtered to the bypassed nodes.
    FilterBypassedNodes,
    ForceLoadBakedCache,
    UnbakeSelected,
    UnbakeSelectedConfirmed,
    UnbakeAll,
    UnbakeAllConfirmed,
    SetBakeResolution(usize),
    SetBakeResolutionConfirmed(usize),
    /// The region previews: a region, or the whole terrain
    /// (`Shell.MiRegion_Click`, `MiStandard_Click`).
    PreviewRegion(Option<String>),
    /// "Save Current Node preview" (`Shell.SaveCurrentNodePreview`).
    SaveRegionReference,
    /// "Edit Regions": Build Settings on its Regions tab
    /// (`ShowRegionsEditor`).
    EditRegions,
    /// The Regions tab: Add Region with its corner here (fractions of the
    /// terrain's width), Delete this Region and what its Yes runs, and a
    /// rename to a name another region has.
    AddRegion([f32; 2]),
    DeleteRegion(String),
    DeleteRegionConfirmed(String),
    RegionNameTaken,
    /// The graph's own commands (`ui::graph_commands`): the clipboard,
    /// searches, groups, portals, combining, the Lazy Menu and the rest.
    Graph(crate::ui::graph_commands::GraphCommand),
    /// File and the file box (`ui::file_commands`): what may ask to save
    /// changes first, autosaves, imports and exports, macros.
    File(crate::files::FileCommand),
    /// A preset applied to the selected node (Presets menus).
    ApplyPreset(String),
    /// The 2D viewport: docked (F6), floating for the selected node
    /// (Ctrl F6), Ctrl M's floating view (`Show2DView`: of the selected
    /// node, else the Map Viewer shown or hidden), its port (`RenderPort`,
    /// which the 3D view shows too), picking and measuring (the Measure
    /// menu) and Save to file.
    Toggle2DViewport,
    ShowLocked2DViewport,
    Show2DView,
    SetPreviewPort(String),
    TogglePickHeight,
    ToggleMeasure,
    Save2DView(std::sync::Arc<nerthus_render::TerrainColours>),
    /// A line in the status bar (`O.UpdateStatus`).
    Report(String),
    /// Toggle Gizmo (Z) for the selected node, and what a gizmo drag set.
    ToggleGizmo,
    GizmoEdit(crate::ui::gizmos::GizmoEdit),
    /// A message box with OK (`MBox(text, caption, MessageBoxButton.OK)`).
    ShowMessage { caption: String, text: String },
    /// Vars: Edit Variable for a variable (a row's Edit…, a new variable),
    /// and a variable's Delete, which asks first (`MnuDelete_Click`), and
    /// what Yes runs.
    EditVariable(String),
    DeleteVariable(String),
    DeleteVariableConfirmed(String),
    /// A parameter's Expression…: Edit Expression for a node's property
    /// (`EditorMenu.ShowExpression`).
    EditExpression { node: i32, property: String },
    /// Vars › Create Connected Nodes › For Macros: MacroPorts on the
    /// selected nodes' inputs, outputs or both (`BtnCreateMacroPorts_OnClick`).
    CreateMacroPorts { inputs: bool, outputs: bool },
    /// The Data View's Show Build Command Line and Build Statistics.
    ShowBuildCommandLine,
    ShowBuildStatistics,
    /// A node gone to in the graph and selected (`GoToLocation`, `Select`).
    GoToNode(i32),
    /// V: the preview laid flat or stood up again (`ToggleFlatView`).
    ToggleFlatView,
    /// Viewport Options › Embedded or Floating (`ViewportConfig`).
    SetViewportFloating(bool),
    /// Construction's Open Constructor (`OpenGodMode`).
    OpenConstructor(i32),
    /// Preview › Record Turntable Animation (`MnuRecordAnimation_OnClick`),
    /// and a recording of so many frames started from it at once
    /// (`--show record-turntable:<frames>`).
    ShowTurntable,
    RecordTurntable(u32),
    /// Viewport Helpers › Overlay: its opacity and its image (Custom asks
    /// for a file).
    SetOverlayOpacity(usize),
    SetOverlayImage(usize),
    /// Tools › Options… (`ShowOptionsDialog`), at a page or the last one.
    ShowOptions(Option<OptionsTab>),
    /// Tools' and Help's windows and their keys (`ui::tool_commands`).
    Tools(crate::ui::tool_commands::ToolCommand),
}

/// What the web build leaves out, by the command that asks for it: what
/// needs files and folders, other programs or the network. Menus leave
/// these out there too; this covers their keys and buttons elsewhere.
#[cfg(target_arch = "wasm32")]
fn web_unavailable(command: &Command) -> Option<&'static str> {
    use crate::files::FileCommand;
    use crate::ui::tool_commands::ToolCommand;
    let feature = match command {
        Command::Exit => "Exit",
        Command::BuildAndExport | Command::StartBuild { .. } | Command::RunBuild { .. } | Command::ExportCachedBuild { .. } | Command::BatchBuild | Command::ShowBuildCommandLine => "Build and Export",
        Command::OpenBuildFolder | Command::OpenFolder(_) | Command::OpenScreenshotsFolder => "Opening folders",
        Command::BakeSelected | Command::BakeRequiredNodes | Command::BakeRequiredNodesConfirmed | Command::BakeAnyway { .. } => "Baking",
        Command::TogglePersistOnSave | Command::ToggleFreezeWhenIdle | Command::DeleteDiskCache | Command::ReloadSmartCache => "The disk cache",
        Command::LoadCamera | Command::SaveCamera => "Loading and saving cameras",
        Command::SaveScreenshot | Command::SaveViewportScreenshotAs | Command::SaveGraphScreenshotAs => "Saving screenshots",
        Command::ShowTurntable | Command::RecordTurntable(_) => "Recording turntables",
        Command::File(FileCommand::RepairFile) => "Repair File",
        Command::File(FileCommand::ImportMacro | FileCommand::ImportGraph | FileCommand::ImportTorlet | FileCommand::ImportImage) => "Importing files",
        Command::File(FileCommand::ExportMacro) => "Exporting macros",
        Command::Tools(ToolCommand::ShowArtifactExplorer) => "The Artifact Explorer",
        Command::Tools(ToolCommand::ShowDiagnostics) => "Diagnostics",
        Command::Tools(ToolCommand::ToggleVerboseLogging | ToolCommand::SetVerboseLogging(_)) => "Verbose logging",
        Command::Tools(ToolCommand::ToggleMcpServer) => "The MCP server",
        Command::Tools(ToolCommand::CheckForUpdates | ToolCommand::InstallUpdate { .. }) => "Updating",
        _ => return None,
    };
    Some(feature)
}

/// Runs the commands the interface collected this frame.
pub fn run(commands: Vec<Command>, editor: &mut Editor, ui_state: &mut UiState, window: &mut Window) {
    for command in commands {
        run_one(command, editor, ui_state, window);
    }
}

fn run_one(command: Command, editor: &mut Editor, ui_state: &mut UiState, window: &mut Window) {
    #[cfg(target_arch = "wasm32")]
    if let Some(feature) = web_unavailable(&command) {
        editor.report(crate::web::unavailable(feature));
        return;
    }
    match command {
        Command::NewProject => crate::ui::file_commands::run(crate::files::FileCommand::New, editor, ui_state),
        Command::Open => crate::ui::file_commands::run(crate::files::FileCommand::Open, editor, ui_state),
        Command::Save => editor.save(),
        Command::SaveAs => editor.dialog = Some(DialogRequest::SaveAs),
        Command::Exit => crate::ui::file_commands::run(crate::files::FileCommand::Exit, editor, ui_state),
        Command::Undo => {
            if editor.document.undo() {
                editor.selection.retain(|id| editor.document.terrain().nodes.contains_key(id));
                ui_state.graph.clear_element_selection();
                editor.changed();
            }
        }
        Command::Redo => {
            if editor.document.redo() {
                editor.selection.retain(|id| editor.document.terrain().nodes.contains_key(id));
                ui_state.graph.clear_element_selection();
                editor.changed();
            }
        }
        Command::NewGraphTab => {
            let index = editor.document.add_graph_tab();
            ui_state.graph.tab = index;
        }
        Command::RenameGraphTab(index, name) => editor.document.rename_graph_tab(index, &name),
        Command::SetGraphTabColor(index, color) => editor.document.set_graph_tab_color(index, color),
        Command::DeleteGraphTab(index) => {
            let next = editor.document.delete_graph_tab(index);
            ui_state.graph.tab = next;
            editor.selection.retain(|id| editor.document.terrain().nodes.contains_key(id));
            editor.changed();
        }
        Command::AddNote => {
            let position = ui_state.graph.drop_position();
            let tab = ui_state.graph.tab;
            let id = editor.document.add_note(position, tab);
            ui_state.graph.clear_element_selection();
            ui_state.graph.selected_note = Some(id);
        }
        Command::Delete => delete_selection(editor, ui_state),
        Command::Duplicate => duplicate_selection(editor, ui_state),
        Command::SelectAll => {
            let tab = ui_state.graph.tab;
            let nodes: Vec<i32> = editor.document.terrain().nodes.values().filter(|node| crate::document::node_tab(node) == tab).map(|node| node.id).collect();
            editor.set_selection(nodes);
        }
        // F2: a node renamed in place, a group too; a note opens the Note
        // Editor.
        Command::Rename => {
            if let Some(node) = editor.active_node() {
                ui_state.graph.renaming = Some(node);
            } else if let Some(group) = ui_state.graph.selected_groups.first().copied() {
                crate::ui::graph_commands::run(GraphCommand::RenameGroup(group), editor, ui_state);
            } else if let Some(note) = ui_state.graph.selected_note {
                crate::ui::graph_commands::run(GraphCommand::EditNote(note), editor, ui_state);
            }
        }
        Command::SelectUpstream => {
            let Some(start) = editor.active_node() else { return };
            let upstream = nerthus_engine::Engine::build_order(editor.document.terrain(), start);
            editor.set_selection(upstream);
        }
        Command::SelectDownstream => {
            let Some(start) = editor.active_node() else { return };
            let downstream = downstream_of(editor, start);
            editor.set_selection(downstream);
        }
        Command::ZoomExtents => ui_state.graph.zoom_request = Some(crate::ui::graph_canvas::ZoomRequest::Extents),
        Command::ZoomReset => ui_state.graph.zoom_request = Some(crate::ui::graph_canvas::ZoomRequest::Reset),
        Command::ZoomSelection => ui_state.graph.zoom_request = Some(crate::ui::graph_canvas::ZoomRequest::Selection),
        Command::ToggleGrid => ui_state.graph.show_grid = !ui_state.graph.show_grid,
        Command::ToggleSnapToGrid => ui_state.graph.snap_to_grid = !ui_state.graph.snap_to_grid,
        Command::Refresh => {
            if let Some(node) = editor.active_node() {
                editor.document.mark_stale(node);
                editor.changed();
            }
        }
        Command::RefreshAll => {
            editor.document.mark_all_stale();
            editor.changed();
        }
        Command::Bypass => {
            for node in editor.selection.clone() {
                let bypassed = editor.document.node(node).and_then(|data| data.base.get("StateOverride")).and_then(Value::as_str) == Some("Bypassed");
                let state = if bypassed { "None" } else { "Bypassed" };
                editor.document.set_member(node, "StateOverride", Value::from(state), "Bypass", true);
            }
            editor.changed();
        }
        // `LockNodePreview`: the selected node's.
        Command::LockPreview => {
            if let Some(node) = editor.active_node() {
                editor.lock_preview(node);
            }
        }
        Command::MarkForExport => mark_for_export(editor),
        Command::ResetNode => {
            if let Some(node) = editor.active_node() {
                editor.document.reset_node(node);
                editor.changed();
            }
        }
        Command::SetPreviewResolution(resolution) => editor.set_preview_resolution(resolution),
        Command::SetRenderMode(mode) => editor.set_render_mode(mode),
        Command::SetMaskEnhancement(enhancement) => editor.set_mask_enhancement(enhancement),
        Command::SetLighting(lighting) => editor.set_lighting(lighting),
        Command::SetMaskGradient(gradient) => editor.set_mask_gradient(gradient),
        Command::LoadCamera => editor.dialog = Some(DialogRequest::LoadCamera),
        Command::SaveCamera => {
            let camera = editor.camera;
            editor.dialog = Some(DialogRequest::SaveCamera { camera });
        }
        Command::ResetCameraToLoaded => editor.reset_camera_to_loaded(),
        Command::ResetCameraToDefault => editor.reset_camera_to_default(),
        Command::SaveScreenshot => editor.save_screenshot(),
        Command::SaveViewportScreenshotAs => {
            let supersample = editor.screenshot_supersample;
            editor.dialog = Some(DialogRequest::SaveScreenshot { graph_region: None, supersample });
        }
        Command::SaveGraphScreenshotAs => {
            let graph_region = editor.graph_region;
            if graph_region.is_some() {
                editor.dialog = Some(DialogRequest::SaveScreenshot { graph_region, supersample: 1 });
            }
        }
        Command::OpenScreenshotsFolder => editor.open_screenshots_folder(),
        Command::SetScreenshotScale(scale) => editor.screenshot_supersample = scale,
        Command::ResetSky => {
            // Every parameter back to its own default.
            editor.set_lighting(ViewportLighting::panel_defaults());
            editor.set_mask_gradient(MaskGradient::Red);
        }
        Command::ToggleEngine => {
            editor.engine_active = !editor.engine_active;
            if editor.engine_active {
                editor.changed();
            } else if editor.building {
                // Suspending stops the running build.
                editor.cancel_build = true;
            }
        }
        // `ClearCache`: asked first.
        Command::ClearCache => {
            let text = "This will remove all node previews and require all nodes to be rebuilt.";
            ui_state.prompt = Some(Prompt::new("Are you sure you want to clear all cache?", text, Command::ClearCacheConfirmed));
        }
        Command::ClearCacheConfirmed => {
            editor.document.mark_all_stale();
            editor.statuses.clear();
            editor.changed();
            let now = window_time(ui_state);
            ui_state.memory_report = Some(crate::ui::memory_dialogs::PendingReport::after(now, true));
        }
        // `MiBakePersistData_OnClick`: unticked, the files go.
        Command::TogglePersistOnSave => {
            let persist = !editor.persist_on_save();
            editor.document.set_build_setting("PersistOnSave", Value::Bool(persist));
            if !persist {
                editor.disk_cache.tasks.push(crate::disk_cache::DiskCacheTask::Delete);
                editor.needs_build = true;
            }
        }
        // `MiPassiveBakingWhenIdle_OnChecked`: the file is marked changed.
        Command::ToggleFreezeWhenIdle => {
            crate::disk_cache::set_freeze_when_idle(!crate::disk_cache::freeze_when_idle());
            editor.document.dirty = true;
            editor.needs_build = editor.preview_node.is_some();
        }
        Command::DeleteDiskCache => {
            editor.disk_cache.tasks.push(crate::disk_cache::DiskCacheTask::Delete);
            editor.needs_build = true;
        }
        Command::ReloadSmartCache => {
            editor.disk_cache.tasks.push(crate::disk_cache::DiskCacheTask::Reload);
            editor.needs_build = true;
        }
        // Preview › Clean Memory (`OptimizeMemory`).
        Command::CleanMemory => ui_state.optimize_memory = Some(crate::ui::memory_dialogs::OptimizeMemory::open(editor)),
        Command::ReleaseMemory => {
            crate::memory::release_unused();
            let now = window_time(ui_state);
            ui_state.memory_report = Some(crate::ui::memory_dialogs::PendingReport::after(now, false));
        }
        Command::DropViewportCache => {
            editor.render_colours = None;
            editor.show_preview();
        }
        Command::CycleNodeSize(node) => {
            let default_size = editor.user.options.default_node_size().to_string();
            editor.document.cycle_node_size(node, &default_size);
        }
        Command::ToggleExpansion => {
            let default_size = editor.user.options.default_node_size().to_string();
            for node in editor.selection.clone() {
                editor.document.cycle_node_size(node, &default_size);
            }
        }
        Command::SetRenderIntent(intent) => {
            if let Some(node) = editor.active_node() {
                editor.document.set_member(node, "RenderIntentOverride", Value::from(intent), "Render intent", true);
                editor.changed();
            }
        }
        Command::AddNode { type_name, position } => {
            let position = position.unwrap_or_else(|| ui_state.graph.drop_position());
            let tab = ui_state.graph.tab;
            if let Some(id) = editor.document.add_node(&type_name, position) {
                editor.apply_default_preset(id);
                if tab != 0 {
                    editor.document.apply_build_update(id, "GraphIndex", Value::from(tab as i64));
                }
                editor.document.apply_node_display(&[id], &editor.user.options);
                editor.select(Some(id));
                editor.changed();
            }
        }
        Command::NodeAction(name) => node_command(editor, &name),
        Command::ToggleModifier(name) => {
            if let Some(node) = editor.active_node() {
                editor.document.toggle_modifier(node, name);
                editor.changed();
            }
        }
        Command::RemoveModifier { node, place, ask: true } => {
            let text = "Are you sure you want to delete this modifier?";
            let confirmed = Command::RemoveModifier { node, place, ask: false };
            ui_state.prompt = Some(Prompt::new("Delete Modifier?", text, confirmed));
        }
        Command::RemoveModifier { node, place, ask: false } => {
            if editor.document.remove_modifier_at(node, place) {
                editor.changed();
            }
        }
        Command::UnbakeNode(node) => {
            // `O.ShowRegionUnbakeWarning`: a region preview needs its
            // critical bakes.
            if editor.region_needs_bakes(Some(node)) {
                let caption = "Unbaking Critical Node";
                ui_state.prompt = Some(Prompt::message(caption, crate::editor::REGION_UNBAKE_WARNING));
            } else {
                let text = "Are you sure you want to unbake this node?\n\nAny baked child nodes will also be unbaked.";
                ui_state.prompt = Some(Prompt::new("Unbake Node?", text, Command::UnbakeNodeConfirmed(node)));
            }
        }
        Command::UnbakeNodeConfirmed(node) => editor.unbake_node(node),
        Command::LoadOffloadedCache(node) => editor.load_offloaded_cache(node),
        Command::WindowMinimize => window.set_minimized(true),
        Command::WindowMaximize => {
            ui_state.maximized = !ui_state.maximized;
            window.set_maximized(ui_state.maximized);
        }
        Command::WindowDrag => window.start_drag_move(),
        Command::SelectNode(id) => editor.select(Some(id)),
        Command::ShowDataTab(tab) => ui_state.data_view.show_tab(tab),
        Command::ToggleBookmark => {
            let Some(node) = editor.active_node() else { return };
            if editor.document.bookmarks().contains(&node) {
                editor.document.remove_bookmark(node, "bookmark");
            } else {
                editor.document.add_bookmark(node);
            }
        }
        Command::AddBookmark => {
            if let Some(node) = editor.active_node() {
                editor.document.add_bookmark(node);
            }
        }
        Command::GoToBookmark(id) => {
            let Some(tab) = editor.document.node(id).map(crate::document::node_tab) else { return };
            ui_state.graph.tab = tab;
            ui_state.graph.bookmark_removing = None;
            editor.select(Some(id));
            // `GoToBookmark`: the node shines as the view goes to it.
            editor.shines.push(id);
            ui_state.graph.zoom_request = Some(crate::ui::graph_canvas::ZoomRequest::Selection);
        }
        Command::ArmBookmarkRemoval(id) => ui_state.graph.bookmark_removing = Some(id),
        Command::RemoveBookmark(id) => {
            editor.document.remove_bookmark(id, "bookmarks");
            ui_state.graph.bookmark_removing = None;
        }
        Command::CopyProperties => {
            let Some(id) = editor.active_node() else { return };
            let Some(node) = editor.document.node(id) else { return };
            let copied = crate::editor::CopiedNode {
                id,
                type_name: node.type_name.clone(),
                values: node.values.clone(),
                document_serial: editor.document_serial,
            };
            editor.copied_node = Some(copied);
        }
        Command::PasteProperties => paste_properties(editor),
        Command::OpenRecent(path) => crate::ui::file_commands::run(crate::files::FileCommand::OpenRecent(path), editor, ui_state),
        Command::OpenLatest => crate::ui::file_commands::run(crate::files::FileCommand::OpenLatest, editor, ui_state),
        Command::SetUiScale(scale) => editor.user.options.set_ui_scale(scale),
        Command::ToggleUnderlay => {
            if let Some(node) = editor.active_node() {
                run_one(Command::SetUnderlay(node), editor, ui_state, window);
            }
        }
        Command::ToggleIgnoreUnderlay => {
            if let Some(node) = editor.active_node() {
                let ignored = editor.document.node(node).and_then(|data| data.base.get("IgnoreUnderlay")).and_then(Value::as_bool).unwrap_or(false);
                editor.document.set_member(node, "IgnoreUnderlay", Value::Bool(!ignored), "Exclude from Underlay", true);
                editor.changed();
            }
        }
        Command::SetUnderlay(id) => {
            if editor.document.set_underlay(id) {
                editor.changed();
            } else {
                let alert = crate::ui::alert::Alert::new("Underlay not set", "This node is not suitable for Underlay usage.", crate::ui::alert::AlertKind::Information);
                ui_state.alert = Some(alert);
            }
        }
        Command::BuildAndExport => {
            let running = editor.export_build.as_ref().is_some_and(|build| !build.finished);
            if running {
                editor.report("A build is already running");
            } else {
                let open_folder = editor.document.build_definition().open_folder;
                ui_state.build_confirm = Some(BuildConfirm { open_folder });
            }
        }
        Command::StartBuild { open_folder, close } => {
            editor.resume_build = None;
            let choice = BuildChoice { open_folder, close };
            match crate::export_build::prepare(editor, choice) {
                BuildStart::Saving | BuildStart::NotSaved => {}
                BuildStart::Refused(message) => ui_state.prompt = Some(Prompt::message("Validation failed", &message)),
                BuildStart::Ready { cached: true } => {
                    let text = "All nodes marked for export seem to be built. Would you like to try to save the current build directly?";
                    let mut question = Prompt::new("Save cached build?", text, Command::ExportCachedBuild { close });
                    question.no = Some(Command::RunBuild { close });
                    question.footnote = Some("Please note, some advanced features such as Prime Folders may not work.".to_string());
                    ui_state.prompt = Some(question);
                }
                BuildStart::Ready { cached: false } => run_build(editor, ui_state, close),
            }
        }
        Command::RunBuild { close } => run_build(editor, ui_state, close),
        Command::ExportCachedBuild { close } => crate::export_build::export_cached(editor, close),
        Command::CachedExportFailed { close } => {
            editor.export_build = None;
            let text = "Nerthus failed to export cached build. Would you like to proceed with a normal build?";
            ui_state.prompt = Some(Prompt::new("Try a normal build?", text, Command::RunBuild { close }));
        }
        Command::BatchBuild => {
            let saved = editor.document.path.as_ref().is_some_and(|path| path.is_file());
            if !saved {
                ui_state.prompt = Some(Prompt::message("Save File to Proceed", "You must save the file before executing a Batch Build."));
            } else if ui_state.batch_build.is_none() {
                let window = crate::ui::batch_build::BatchWindow::open(&mut ui_state.batch_session);
                ui_state.batch_build = Some(window);
            }
        }
        Command::CancelBuild => {
            if let Some(build) = &editor.export_build {
                build.cancel();
            }
        }
        Command::CloseBuildWindow => editor.export_build = None,
        Command::BuildSettings => {
            if ui_state.build_settings.is_none() {
                ui_state.build_settings = Some(BuildSettingsState::new(SettingsTab::Resolution));
            }
        }
        Command::OpenBuildFolder => editor.open_build_folder(),
        Command::OpenFolder(folder) => crate::export_build::open_folder(&folder),
        Command::BakeSelected => {
            let refusal = editor.active_node().and_then(|node| editor.bake_selected_refusal(node));
            if let Some(refusal) = refusal {
                ui_state.prompt = Some(bake_refusal_prompt(refusal));
            } else if let Some(node) = editor.prepare_bake_selected() {
                match crate::ui::memory_dialogs::bake_memory_warning(editor, node) {
                    Some(mut warning) => {
                        warning.yes = Some(Command::BakeAnyway { nodes: vec![node], selected: true });
                        ui_state.prompt = Some(warning);
                    }
                    None => start_bake(editor, ui_state, vec![node], true),
                }
            }
        }
        // The nodes' chains are checked before the question.
        Command::BakeRequiredNodes => {
            let nodes = nerthus_export::bake::required_nodes(editor.document.terrain());
            match editor.bake_chain_refusal(&nodes) {
                Some(refusal) => ui_state.prompt = Some(bake_refusal_prompt(refusal)),
                None => {
                    let text = "Are you sure you want to bake all nodes that require baking?";
                    ui_state.prompt = Some(Prompt::new("Bake required nodes?", text, Command::BakeRequiredNodesConfirmed));
                }
            }
        }
        Command::BakeRequiredNodesConfirmed => {
            if let Some(nodes) = editor.prepare_bake_required() {
                // The warning weighs the node shown in the properties (its
                // chain), not the nodes being baked.
                let warning = editor.active_node().and_then(|node| crate::ui::memory_dialogs::bake_memory_warning(editor, node));
                match warning {
                    Some(mut warning) => {
                        warning.yes = Some(Command::BakeAnyway { nodes, selected: false });
                        ui_state.prompt = Some(warning);
                    }
                    None => start_bake(editor, ui_state, nodes, false),
                }
            }
        }
        Command::BakeAnyway { nodes, selected } => start_bake(editor, ui_state, nodes, selected),
        Command::FilterBypassedNodes => ui_state.data_view.terrain.filter = FilterCriteria::Bypassed,
        Command::ForceLoadBakedCache => editor.load_baked_cache(),
        Command::UnbakeSelected => {
            let text = "Are you sure you want to unbake this node?\n\nAny baked child nodes will also be unbaked.File will be saved after unbake.";
            ui_state.prompt = Some(Prompt::new("Unbake Node?", text, Command::UnbakeSelectedConfirmed));
        }
        Command::UnbakeSelectedConfirmed => editor.unbake_selected(),
        Command::UnbakeAll => {
            let text = "Are you sure you want to unbake all node? File will be saved after unbake.";
            ui_state.prompt = Some(Prompt::new("Unbake everything?", text, Command::UnbakeAllConfirmed));
        }
        Command::UnbakeAllConfirmed => editor.unbake_all(),
        Command::SetBakeResolution(resolution) => {
            let current = editor.document.build_definition().bake_resolution;
            if resolution != current && !editor.baked_nodes().is_empty() {
                let caption = "Changing baking resolution will unbake any baked nodes!";
                ui_state.prompt = Some(Prompt::new(caption, "Are you sure you want to continue?", Command::SetBakeResolutionConfirmed(resolution)));
            } else {
                editor.set_bake_resolution(resolution, false);
            }
        }
        Command::SetBakeResolutionConfirmed(resolution) => editor.set_bake_resolution(resolution, true),
        Command::PreviewRegion(None) => editor.set_preview_region(None),
        Command::PreviewRegion(Some(name)) => {
            if editor.can_preview_regions() {
                editor.set_preview_region(Some(name));
            } else {
                let text = "There are nodes which must be baked before Regions can be activated. See the Data View filter \"Requires Baking\" for details.";
                ui_state.prompt = Some(Prompt::message("Nodes must be baked", text));
            }
        }
        Command::SaveRegionReference => crate::ui::regions::save_reference(editor),
        Command::EditRegions => {
            let mut settings = BuildSettingsState::new(SettingsTab::Regions);
            if let Some(open) = ui_state.build_settings.take() {
                settings = open;
                settings.tab = SettingsTab::Regions;
            }
            ui_state.build_settings = Some(settings);
        }
        Command::AddRegion(corner) => {
            editor.document.add_region(corner);
        }
        Command::DeleteRegion(name) => {
            let text = "Are you sure you want to remove current region";
            ui_state.prompt = Some(Prompt::new("Remove region", text, Command::DeleteRegionConfirmed(name)));
        }
        Command::DeleteRegionConfirmed(name) => {
            // `Rc_RegionDeletion`: a region being previewed goes back to
            // the whole terrain first.
            if editor.preview_region.as_deref() == Some(name.as_str()) {
                editor.set_preview_region(None);
            }
            editor.document.delete_region(&name);
        }
        Command::RegionNameTaken => {
            let text = "A region with this name already exists. Please choose a different name.";
            ui_state.prompt = Some(Prompt::message("Duplicate Name", text));
        }
        Command::Graph(graph_command) => crate::ui::graph_commands::run(graph_command, editor, ui_state),
        Command::File(command) => crate::ui::file_commands::run(command, editor, ui_state),
        Command::ShowMessage { caption, text } => ui_state.prompt = Some(Prompt::message(&caption, &text)),
        Command::EditVariable(key) => ui_state.variable_dialog = crate::ui::variable_dialog::open(editor, &key),
        Command::EditExpression { node, property } => {
            let dialog = crate::ui::expression_dialog::ExpressionDialog::open(editor, node, &property);
            ui_state.expression_dialog = Some(dialog);
        }
        Command::DeleteVariable(key) => {
            let caption = format!("Are you sure you want to delete the [{key}] variable?");
            let text = "All node properties linked to this variable will revert to normal.";
            ui_state.prompt = Some(Prompt::new(&caption, text, Command::DeleteVariableConfirmed(key)));
        }
        Command::DeleteVariableConfirmed(key) => {
            editor.document.delete_variable(&key);
            editor.changed();
        }
        Command::CreateMacroPorts { inputs, outputs } => {
            let selected = editor.selection.clone();
            let caches = &editor.node_caches;
            let built_intent = |node: i32, port: &str| {
                let cache = caches.get(&node)?;
                let cached = cache.ports.iter().find(|cached| cached.name == port)?;
                Some(cached.intent)
            };
            let made = editor.document.create_macro_ports(&selected, inputs, outputs, built_intent);
            if !made.is_empty() {
                editor.document.apply_node_display(&made, &editor.user.options);
                editor.changed();
            }
        }
        Command::ShowBuildCommandLine => ui_state.command_line = Some(crate::ui::command_line_window::CommandLineWindow::new(editor)),
        Command::ShowBuildStatistics => ui_state.statistics = Some(crate::ui::statistics_window::StatisticsWindow::new(editor)),
        Command::ToggleFlatView => editor.toggle_flat_view(),
        Command::OpenConstructor(node) => {
            // `--show constructor` asks for the chosen node's.
            let node = if node < 0 { editor.active_node().unwrap_or(node) } else { node };
            let open_already = ui_state.constructor.as_ref().is_some_and(|window| window.node() == node);
            if !open_already {
                ui_state.constructor = Some(crate::ui::constructor::ConstructorWindow::new(node));
            }
        }
        Command::ShowTurntable => {
            let window = crate::ui::turntable_window::TurntableWindow::new(editor, &ui_state.turntable_memory);
            ui_state.turntable = Some(window);
        }
        Command::RecordTurntable(frames) => {
            let mut window = crate::ui::turntable_window::TurntableWindow::new(editor, &ui_state.turntable_memory);
            window.press_record(frames);
            ui_state.turntable = Some(window);
        }
        Command::SetViewportFloating(floating) => {
            // The lighting panel belongs to the window being left.
            ui_state.sun_open = false;
            editor.user.options.set_viewport_floating(floating);
        }
        Command::SetOverlayOpacity(index) => {
            let mut overlay = editor.overlay.clone();
            overlay.opacity = index;
            editor.set_overlay(overlay);
        }
        Command::SetOverlayImage(index) => {
            if index == crate::overlays::CUSTOM_OVERLAY {
                editor.dialog = Some(crate::editor::DialogRequest::CustomOverlay);
                return;
            }
            // `OverlayChanged`: an overlay picked while none shows shows
            // at 50%.
            let mut overlay = editor.overlay.clone();
            overlay.image = index;
            if overlay.opacity == 0 {
                overlay.opacity = crate::overlays::SHOWN_OPACITY;
            }
            editor.set_overlay(overlay);
        }
        Command::GoToNode(id) => {
            let Some(node) = editor.document.node(id) else { return };
            ui_state.graph.tab = crate::document::node_tab(node);
            ui_state.graph.zoom_request = Some(crate::ui::graph_canvas::ZoomRequest::Center(id));
            editor.select(Some(id));
        }
        Command::ApplyPreset(name) => editor.apply_preset(&name),
        Command::Toggle2DViewport => ui_state.viewport_2d.toggle_docked(),
        Command::ShowLocked2DViewport => {
            if let Some(node) = editor.active_node() {
                ui_state.viewport_2d.open_floating(node);
            }
        }
        Command::Show2DView => ui_state.viewport_2d.show_2d_view(editor),
        Command::SetPreviewPort(port) => {
            editor.preview_port = port;
            editor.show_preview();
        }
        Command::TogglePickHeight => ui_state.viewport_2d.toggle_picking(editor),
        Command::ToggleMeasure => ui_state.viewport_2d.toggle_measuring(editor),
        Command::Save2DView(colours) => editor.dialog = Some(DialogRequest::Save2DView { colours }),
        Command::Report(text) => editor.report(text),
        Command::ToggleGizmo => {
            let has_gizmo = editor.active_node().and_then(|node| editor.document.node(node)).and_then(|node| crate::ui::gizmos::GizmoKind::of(&node.type_name)).is_some();
            if has_gizmo {
                editor.gizmo_active = !editor.gizmo_active;
            }
        }
        Command::GizmoEdit(edit) => {
            // One undo step a drag (`Gizmo2DHit` as it moves).
            if edit.started {
                editor.document.checkpoint("Gizmo");
            }
            if !edit.values.is_empty() {
                editor.document.set_values_quietly(edit.node, &edit.values);
                editor.changed();
            }
        }
        Command::ShowOptions(tab) => {
            if ui_state.options.is_none() {
                let tab = tab.unwrap_or(ui_state.last_options_tab);
                ui_state.options = Some(OptionsState::new(tab));
            }
        }
        Command::Tools(command) => crate::ui::tool_commands::run(command, editor, ui_state),
    }
}

/// The interface's clock, as the last frame saw it.
fn window_time(ui_state: &UiState) -> f64 {
    ui_state.clock
}

/// `_miPaste_Click`: the source node's properties as they are now (as
/// copied when it is gone or in another document) onto each selected node
/// of its type.
fn paste_properties(editor: &mut Editor) {
    let Some(copied) = editor.copied_node.clone() else {
        return;
    };
    let same_document = copied.document_serial == editor.document_serial;
    let live = editor.document.node(copied.id).filter(|node| same_document && node.type_name == copied.type_name);
    let values = live.map(|node| node.values.clone()).unwrap_or(copied.values);
    let targets = editor.selection.clone();
    editor.document.paste_properties(&targets, &copied.type_name, &values);
    editor.changed();
}

/// `GraphControl.MarkForExport` (F3): each selected node marked or
/// unmarked, and said so in the status bar.
fn mark_for_export(editor: &mut Editor) {
    let selected = editor.selection.clone();
    if selected.is_empty() {
        return;
    }
    let format = editor.user.preferences.heightfield_format();
    let changes = editor.document.toggle_export(&selected, format);
    for (name, marked) in changes {
        let state = if marked { "marked" } else { "unmarked" };
        editor.report(format!("{name} {state} for export."));
    }
}

/// `Editor_CommandExecuted` for the selected node: Add Input, Swap
/// Inputs (Combine) and Randomize (SatelliteMap only).
fn node_command(editor: &mut Editor, name: &str) {
    let Some(node) = editor.active_node() else {
        return;
    };
    let type_name = editor.document.node(node).map(|data| data.type_name.clone()).unwrap_or_default();
    match (name, type_name.as_str()) {
        ("AddInput", _) => {
            if editor.document.add_input(node) {
                editor.changed();
            }
        }
        ("SwitchInputs", "Combine") => {
            editor.document.switch_inputs(node);
            editor.changed();
        }
        ("PurgeCache", "File" | "TileInput") => editor.purge_cache(node),
        ("FitCurve", "Curve") | ("SynthColors", "Synth") => editor.run_node_command(node, name),
        ("ExportJSON", "Synth") => editor.dialog = Some(DialogRequest::ExportSynthColours { node }),
        // `Node_ToggleTiling` (`Comms.Send(36, …)`).
        ("ToggleTiling", _) => editor.tiled_preview = !editor.tiled_preview,
        // A library item of -1 makes the build draw one, as Randomize does.
        ("Randomize", "SatelliteMap") => {
            editor.document.set_value(node, "LibraryItem", Value::from(-1));
            editor.document.mark_stale(node);
            editor.changed();
        }
        ("Randomize", _) => {}
        _ => {
            let label = nerthus_engine::humanize::humanize_title(name);
            editor.report(format!("{label} is not available in Nerthus yet"));
        }
    }
}

fn downstream_of(editor: &Editor, start: i32) -> Vec<i32> {
    let nodes = &editor.document.terrain().nodes;
    let mut found = vec![start];
    let mut index = 0;
    while index < found.len() {
        let current = found[index];
        for node in nodes.values() {
            let feeds = node.ports.iter().any(|port| port.record.as_ref().is_some_and(|record| record.from == current));
            if feeds && !found.contains(&node.id) {
                found.push(node.id);
            }
        }
        index += 1;
    }
    found
}

/// `GraphControl.DuplicateNode` (Ctrl D): copies of the selected nodes
/// (and the connections between them) beside the originals, chosen.
fn duplicate_selection(editor: &mut Editor, ui_state: &mut UiState) {
    let selected = editor.selection.clone();
    if selected.is_empty() {
        return;
    }
    let tab = ui_state.graph.tab;
    let copies = editor.document.duplicate_selection(&selected, tab);
    ui_state.graph.clear_element_selection();
    editor.set_selection(copies);
    editor.changed();
}

/// `ExecuteDelete` (Del): the chosen nodes, connections, groups (their
/// nodes too with Shift) and note, one on its own or several at once.
fn delete_selection(editor: &mut Editor, ui_state: &mut UiState) {
    let graph = &mut ui_state.graph;
    let request = DeleteRequest {
        nodes: editor.selection.clone(),
        connections: std::mem::take(&mut graph.selected_connections),
        groups: std::mem::take(&mut graph.selected_groups),
        notes: graph.selected_note.take().into_iter().collect(),
        with_group_children: graph.shift_held,
    };
    editor.document.delete_elements(&request);
    // `ExecuteMassDelete` ends with `AfterNodeDeleted(null)`, which checks
    // for variables nothing is bound to any more; a single delete does not.
    if request.count() > 1
        && let Some(alert) = crate::ui::alert::orphan_variables_alert(editor)
    {
        ui_state.alert = Some(alert);
    }
    editor.selection.retain(|id| editor.document.terrain().nodes.contains_key(id));
    let preview_gone = editor.preview_node.is_some_and(|node| editor.document.node(node).is_none());
    if preview_gone {
        editor.preview_node = None;
        editor.preview_locked = false;
    }
    editor.changed();
}

/// The build once checked; when `nerthus-build` cannot be started for
/// Close Nerthus and Build, the reason shows as a refusal would.
fn run_build(editor: &mut Editor, ui_state: &mut UiState, close: bool) {
    if let Err(message) = crate::export_build::run_build(editor, close) {
        ui_state.prompt = Some(Prompt::message("Validation failed", &message));
    }
}

/// A refused bake's message box. The bypassed nodes' is a task dialog
/// whose own button filters the Data View to them.
fn bake_refusal_prompt(refusal: BakeRefusal) -> Prompt {
    match refusal {
        BakeRefusal::BypassedChain => Prompt::with_button(refusal.caption(), refusal.text(), "Filter Bypassed nodes", Command::FilterBypassedNodes),
        BakeRefusal::MacroNode | BakeRefusal::MacroInChain => Prompt::message(refusal.caption(), refusal.text()),
    }
}

/// A bake started; Bake Selected's has its progress window
/// (`BakeSelectedNode`'s).
fn start_bake(editor: &mut Editor, ui_state: &mut UiState, nodes: Vec<i32>, selected: bool) {
    editor.start_bake(nodes);
    if selected && editor.baking {
        ui_state.bake_progress = crate::ui::memory_dialogs::bake_progress(editor);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde_json::json;

    use super::*;
    use crate::document::Document;

    #[test]
    fn a_bypassed_node_upstream_offers_to_filter_the_data_view() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [26000.0, 26000.0]).unwrap();
        let blur = document.add_node("Blur", [26300.0, 26000.0]).unwrap();
        document.connect(mountain, "Out", blur, "In");
        document.set_member(mountain, "StateOverride", json!("Bypassed"), "Bypass", true);
        let mut editor = Editor::new(document);
        editor.select(Some(blur));
        let mut ui_state = UiState::default();
        let mut window = Window::default();
        run(vec![Command::BakeSelected], &mut editor, &mut ui_state, &mut window);

        let refusal = ui_state.prompt.take().expect("the bake is refused");
        assert_eq!(refusal.caption, "Cannot Start Baking");
        assert_eq!(refusal.text, "Some nodes required for baking are currently bypassed. Please enable those nodes and try again.");
        assert_eq!((refusal.yes_text.as_str(), refusal.no_text.as_str()), ("Filter Bypassed nodes", "Close"));
        assert_eq!(refusal.no, None);
        assert!(!editor.baking);

        // The button leaves only the bypassed node at full strength.
        let filter = refusal.yes.expect("the button runs a command");
        run(vec![filter], &mut editor, &mut ui_state, &mut window);
        assert_eq!(ui_state.data_view.terrain.filter, FilterCriteria::Bypassed);
        let dimmed = crate::ui::data_view::dimmed_nodes(&editor, ui_state.data_view.terrain.filter);
        assert_eq!(dimmed, HashSet::from([blur]));
    }

    #[test]
    fn bake_required_nodes_checks_the_chains_before_asking() {
        let mut document = Document::new();
        let macro_node = document.add_node("Macro", [26000.0, 26000.0]).unwrap();
        let mountain = document.add_node("Mountain", [26000.0, 26400.0]).unwrap();
        let gate = document.add_node("Gate", [26300.0, 26000.0]).unwrap();
        document.connect(macro_node, "Out", gate, "In");
        let mut editor = Editor::new(document);
        let mut ui_state = UiState::default();
        let mut window = Window::default();
        run(vec![Command::BakeRequiredNodes], &mut editor, &mut ui_state, &mut window);
        let refusal = ui_state.prompt.take().expect("the bake is refused");
        assert_eq!(refusal.caption, "Macro Detected in Bake Chain");
        assert_eq!(refusal.text, "This node depends on a Macro and cannot be baked.");
        assert_eq!(refusal.yes, None);

        // Fed by the Mountain instead, the question comes.
        editor.document.connect(mountain, "Out", gate, "In");
        run(vec![Command::BakeRequiredNodes], &mut editor, &mut ui_state, &mut window);
        let question = ui_state.prompt.take().expect("the question comes");
        assert_eq!(question.caption, "Bake required nodes?");
        assert_eq!(question.yes, Some(Command::BakeRequiredNodesConfirmed));
    }

    #[test]
    fn a_macro_is_not_baked_itself() {
        let mut document = Document::new();
        let macro_node = document.add_node("Macro", [26000.0, 26000.0]).unwrap();
        let mut editor = Editor::new(document);
        editor.select(Some(macro_node));
        let mut ui_state = UiState::default();
        let mut window = Window::default();
        run(vec![Command::BakeSelected], &mut editor, &mut ui_state, &mut window);
        let refusal = ui_state.prompt.take().expect("the bake is refused");
        assert_eq!(refusal.caption, "Cannot bake macro node");
        assert_eq!(refusal.text, "Selected node is a Macro and cannot be baked.");
        assert_eq!(refusal.yes, None);
    }
}
