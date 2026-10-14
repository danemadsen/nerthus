//! The main menu (`Shell` menu bar): File, Edit, Graph, Node, Preview,
//! Project, Tools and Help.

use egui::Ui;
use nerthus_engine::StateOverride;

use crate::document::{node_tab, CombineKind, PortalState, TabTarget};
use crate::editor::{Editor, RenderMode, PREVIEW_RESOLUTIONS};
use crate::files::FileCommand;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::graph_commands::GraphCommand;
use crate::ui::tool_commands::ToolCommand;
use crate::ui::widgets::{check_item, menu_caption, menu_item, menu_item_enabled, menu_separator, menu_style, submenu};
use crate::ui::graph_canvas::{node_color, node_icon};
use crate::ui::UiState;

/// File (`shell.xaml`'s File menu).
pub fn file_menu(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    menu_style(ui);
    if menu_item(ui, Some("file-broken"), Some(theme::ACCENT_PRIMARY), "New", "Ctrl N").clicked() {
        commands.push(Command::NewProject);
    }
    if menu_item(ui, Some("folder"), None, "Open…", "Ctrl O").clicked() {
        commands.push(Command::Open);
    }
    submenu(ui, None, None, "Open Recent", |ui| {
        menu_style(ui);
        recent_files(ui, editor, commands);
    });
    if menu_item(ui, None, None, "Open Examples…", "").clicked() {
        commands.push(Command::File(FileCommand::OpenExamples));
    }
    if menu_item(ui, Some("input-spark"), Some(theme::ACCENT_YELLOW), "Start Dialog", "").clicked() {
        commands.push(Command::File(FileCommand::StartDialog));
    }
    menu_separator(ui);
    if menu_item(ui, Some("device-floppy"), Some(theme::ACCENT_BLUE), "Save…", "Ctrl S").clicked() {
        commands.push(Command::Save);
    }
    if menu_item(ui, Some("device-floppy"), None, "Save as…", "Ctrl Shift S").clicked() {
        commands.push(Command::SaveAs);
    }
    if menu_item(ui, Some("copy-plus"), None, "Save incremental", "").clicked() {
        commands.push(Command::File(FileCommand::SaveIncremental));
    }
    menu_separator(ui);
    // The web build has no files to repair or import, nor macros to
    // export, and its window is the browser's tab.
    let web = cfg!(target_arch = "wasm32");
    if !web && menu_item(ui, None, None, "Repair File…", "").clicked() {
        commands.push(Command::File(FileCommand::RepairFile));
    }
    if !web {
        submenu(ui, None, None, "Import", |ui| {
            menu_style(ui);
            let items = [("Macro…", FileCommand::ImportMacro), ("Graph…", FileCommand::ImportGraph), ("TORLET…", FileCommand::ImportTorlet), ("Image…", FileCommand::ImportImage)];
            for (text, command) in items {
                if menu_item(ui, None, None, text, "").clicked() {
                    commands.push(Command::File(command));
                }
            }
        });
    }
    submenu(ui, None, None, "Export", |ui| {
        menu_style(ui);
        let items = [("Macro…", FileCommand::ExportMacro), ("TORLET…", FileCommand::ExportTorlet)];
        for (text, command) in items {
            let shown = !web || command != FileCommand::ExportMacro;
            if shown && menu_item(ui, None, None, text, "").clicked() {
                commands.push(Command::File(command));
            }
        }
    });
    menu_item_enabled(ui, Some("file-barcode"), None, "Share…", "", false);
    if !web {
        menu_separator(ui);
        if menu_item(ui, None, Some(theme::ACCENT_RED), "Exit", "Alt F4").clicked() {
            commands.push(Command::Exit);
        }
    }
}

pub fn edit_menu(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    menu_style(ui);
    let can_undo = editor.document.can_undo();
    let can_redo = editor.document.can_redo();
    if menu_item_enabled(ui, Some("arrow-back-up"), None, "Undo", "Ctrl Z", can_undo).clicked() {
        commands.push(Command::Undo);
    }
    if menu_item_enabled(ui, Some("arrow-forward-up"), None, "Redo", "Ctrl Y", can_redo).clicked() {
        commands.push(Command::Redo);
    }
    menu_separator(ui);
    let has_selection = !editor.selection.is_empty();
    // `MiEdit_SubmenuOpened`: Cut and Copy for nodes, Paste once something
    // was cut or copied (`IsPasteActive`), Copy Properties for the node
    // shown and Paste Properties onto one of the copied node's type.
    if menu_item_enabled(ui, None, None, "Cut", "Ctrl X", has_selection).clicked() {
        commands.push(Command::Graph(GraphCommand::Cut));
    }
    if menu_item_enabled(ui, None, None, "Copy", "Ctrl C", has_selection).clicked() {
        commands.push(Command::Graph(GraphCommand::Copy));
    }
    let can_paste = editor.node_clipboard.is_some();
    if menu_item_enabled(ui, Some("clipboard-copy"), None, "Paste", "Ctrl V", can_paste).clicked() {
        commands.push(Command::Graph(GraphCommand::Paste));
    }
    menu_separator(ui);
    let active = editor.active_node().and_then(|id| editor.document.node(id));
    if menu_item_enabled(ui, None, None, "Copy Properties", "", active.is_some()).clicked() {
        commands.push(Command::CopyProperties);
    }
    let copied_type = editor.copied_node.as_ref().map(|copied| copied.type_name.as_str());
    let can_paste_properties = active.is_some_and(|node| copied_type == Some(node.type_name.as_str()));
    if menu_item_enabled(ui, None, None, "Paste Properties", "", can_paste_properties).clicked() {
        commands.push(Command::Graph(GraphCommand::PastePropertiesToActive));
    }
    menu_separator(ui);
    if menu_item_enabled(ui, None, None, "Rename", "F2", has_selection).clicked() {
        commands.push(Command::Rename);
    }
    if menu_item_enabled(ui, Some("stack"), None, "Duplicate", "Ctrl D", has_selection).clicked() {
        commands.push(Command::Duplicate);
    }
    menu_separator(ui);
    if menu_item_enabled(ui, Some("square-rounded-x"), Some(theme::ACCENT_RED), "Delete", "Del", has_selection).clicked() {
        commands.push(Command::Delete);
    }
}

pub fn graph_menu(ui: &mut Ui, editor: &Editor, ui_state: &UiState, commands: &mut Vec<Command>) {
    menu_style(ui);
    graph_commands(ui, editor, ui_state, commands);
    menu_separator(ui);
    bake_items(ui, editor, commands);
    menu_separator(ui);
    submenu(ui, None, None, "Bookmarks", |ui| {
        menu_style(ui);
        bookmarks_menu(ui, editor, ui_state, commands);
    });
}

/// `CreateBookmarkMenu` (Graph > Bookmarks and the graph toolbar's
/// bookmarks): Add bookmark, then each bookmarked node by name with its
/// family icon in its toolbox colour. A click goes to the node; a right
/// click turns the entry into "Remove?", which a click then removes.
pub fn bookmarks_menu(ui: &mut Ui, editor: &Editor, ui_state: &UiState, commands: &mut Vec<Command>) {
    if menu_item(ui, Some("bookmark-plus"), None, "Add bookmark", "").clicked() {
        commands.push(Command::AddBookmark);
    }
    let bookmarks = editor.document.bookmarks();
    if bookmarks.is_empty() {
        return;
    }
    menu_separator(ui);
    for id in bookmarks {
        let Some(node) = editor.document.node(id) else {
            continue;
        };
        let removing = ui_state.graph.bookmark_removing == Some(id);
        let header = if removing { "Remove?" } else { node.name.as_str() };
        let icon = node_icon(&node.type_name);
        let response = menu_item(ui, Some(&icon), Some(node_color(node)), header, "");
        if response.secondary_clicked() {
            commands.push(Command::ArmBookmarkRemoval(id));
        } else if response.clicked() {
            let command = if removing { Command::RemoveBookmark(id) } else { Command::GoToBookmark(id) };
            commands.push(command);
        }
    }
}

/// `New Graph Tab` to `Snap to Items`, shared with the graph toolbar.
pub fn graph_commands(ui: &mut Ui, editor: &Editor, ui_state: &UiState, commands: &mut Vec<Command>) {
    if menu_item(ui, Some("square-rounded-plus-2"), Some(theme::ACCENT_BLUE), "New Graph Tab", "").clicked() {
        commands.push(Command::NewGraphTab);
    }
    if menu_item(ui, Some("note"), Some(theme::ACCENT_STRONG_YELLOW), "Add Note", "").clicked() {
        commands.push(Command::AddNote);
    }
    menu_separator(ui);
    menu_caption(ui, "Zoom");
    if menu_item(ui, Some("zoom-scan"), None, "Extents", "Ctrl 0").clicked() {
        commands.push(Command::ZoomExtents);
    }
    if menu_item(ui, None, None, "Reset Zoom", "Ctrl 1").clicked() {
        commands.push(Command::ZoomReset);
    }
    if menu_item(ui, None, None, "Selection", "Ctrl 2").clicked() {
        commands.push(Command::ZoomSelection);
    }
    menu_separator(ui);
    menu_caption(ui, "Graph Surface");
    if check_item(ui, ui_state.graph.show_grid, "Show Grid", "").clicked() {
        commands.push(Command::ToggleGrid);
    }
    if check_item(ui, ui_state.graph.snap_to_grid, "Snap to Grid", "").clicked() {
        commands.push(Command::ToggleSnapToGrid);
    }
    if check_item(ui, editor.user.options.snap_to_items(), "Snap to Items", "").clicked() {
        commands.push(Command::Graph(GraphCommand::ToggleSnapToItems));
    }
}

/// `PrepareMoveToMenu` (and a tab's Tab to group): every other tab, then
/// New Graph. Returns the one chosen.
pub fn tab_targets(ui: &mut Ui, editor: &Editor, current: usize) -> Option<TabTarget> {
    menu_style(ui);
    let tabs = editor.document.graph_tabs();
    let mut chosen = None;
    for tab in tabs.iter().filter(|tab| tab.index != current) {
        if menu_item(ui, None, None, &tab.name, "").clicked() {
            chosen = Some(TabTarget::Existing(tab.index));
        }
    }
    if tabs.len() > 1 {
        menu_separator(ui);
    }
    if menu_item(ui, None, None, "New Graph", "").clicked() {
        chosen = Some(TabTarget::New);
    }
    chosen
}

/// `PopulateConvertToPortal`: the selected node's primary output, then its
/// others by name; a portal already shows the red `square-rounded-x`. A
/// click makes the port a portal or stops it being one.
pub fn convert_port_items(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    menu_style(ui);
    let Some(node) = editor.active_node().and_then(|id| editor.document.node(id)) else {
        return;
    };
    let mut outputs: Vec<&nerthus_engine::PortData> = node.ports.iter().filter(|port| port.is_output()).collect();
    outputs.sort_by_key(|port| (!port.is_primary(), port.name.clone()));
    for port in outputs {
        let portal = PortalState::of(port).is_portal();
        let (icon, accent) = if portal { (Some("square-rounded-x"), Some(theme::ACCENT_RED)) } else { (None, None) };
        if menu_item(ui, icon, accent, &port.name, "").clicked() {
            commands.push(Command::Graph(GraphCommand::TogglePortPortal { node: node.id, port: port.name.clone() }));
        }
    }
}

/// The Bake menu, shared with the graph toolbar (`BakeMenu_OnSubmenuOpened`:
/// Bake Selected for a node that is not baked, with what it would bake;
/// Unbake Selected for a baked one; Force load and Unbake All when any
/// node is baked; the bake resolution checked).
pub fn bake_items(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    bake_menu(ui, editor, commands, false);
}

/// The graph toolbar's Bake menu, which also offers Reload SmartCache to
/// RAM (`miReload`).
pub fn toolbar_bake_items(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    bake_menu(ui, editor, commands, true);
}

fn bake_menu(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>, reload_item: bool) {
    // Bakes and the disk cache are files, which the web build cannot keep.
    if cfg!(target_arch = "wasm32") {
        return;
    }
    submenu(ui, None, None, "Bake", |ui| {
        menu_style(ui);
        let selected = editor.active_node().and_then(|id| editor.document.node(id));
        let selected_baked = selected.is_some_and(|node| StateOverride::of(node) == StateOverride::Baked);
        let any_baked = !editor.baked_nodes().is_empty();
        let definition = editor.document.build_definition();
        let can_bake = selected.is_some() && !selected_baked;
        let bake_label = match selected {
            Some(node) if can_bake => bake_estimate(editor, node.id, definition.bake_resolution),
            _ => "Bake Selected".to_string(),
        };
        if menu_item_enabled(ui, Some("matchstick"), None, &bake_label, "", can_bake).clicked() {
            commands.push(Command::BakeSelected);
        }
        if menu_item(ui, Some("file-shred"), None, "Bake Required Nodes", "").clicked() {
            commands.push(Command::BakeRequiredNodes);
        }
        if menu_item_enabled(ui, None, None, "Force load baked cache", "", any_baked).clicked() {
            commands.push(Command::ForceLoadBakedCache);
        }
        menu_separator(ui);
        if menu_item_enabled(ui, Some("fire-extinguisher"), None, "Unbake Selected", "", selected_baked).clicked() {
            commands.push(Command::UnbakeSelected);
        }
        if menu_item_enabled(ui, Some("fire-hydrant"), Some(theme::ACCENT_MUTED_RED), "Unbake All", "", any_baked).clicked() {
            commands.push(Command::UnbakeAll);
        }
        menu_separator(ui);
        menu_caption(ui, "Resolution");
        for size in [1024, 2048, 4096, 8192, 16384] {
            if check_item(ui, size == definition.bake_resolution, &format!("{size} x {size}"), "").clicked() {
                commands.push(Command::SetBakeResolution(size));
            }
        }
        menu_separator(ui);
        menu_separator(ui);
        if reload_item {
            if menu_item(ui, None, None, "Reload SmartCache to RAM", "").clicked() {
                commands.push(Command::ReloadSmartCache);
            }
            menu_separator(ui);
        }
        menu_caption(ui, "Cache to Disk");
        if check_item(ui, editor.persist_on_save(), "When closing file", "").clicked() {
            commands.push(Command::TogglePersistOnSave);
        }
        if check_item(ui, crate::disk_cache::freeze_when_idle(), "When Idle", "").clicked() {
            commands.push(Command::ToggleFreezeWhenIdle);
        }
        if menu_item(ui, Some("trash-x"), Some(theme::ACCENT_MUTED_RED), "Delete Cache", "").clicked() {
            commands.push(Command::DeleteDiskCache);
        }
    });
}

/// `EstimateBakeSizeForMenu`: "Bake Selected (<nodes in
/// the chain> nodes, <megabytes>)", each output four bytes a value and
/// three channels for colour, counted in whole megabytes.
fn bake_estimate(editor: &Editor, node: i32, resolution: usize) -> String {
    let terrain = editor.document.terrain();
    let chain = nerthus_engine::Engine::build_order(terrain, node);
    let area = (resolution * resolution) as u64;
    let catalogue = nerthus_engine::Catalogue::get();
    let mut bytes = 0u64;
    for id in &chain {
        let Some(data) = terrain.nodes.get(id) else {
            continue;
        };
        let spec = catalogue.node(&data.type_name);
        for port in data.ports.iter().filter(|port| port.is_output()) {
            let intent = spec.and_then(|spec| spec.ports.iter().find(|candidate| candidate.name == port.name)).and_then(|spec_port| spec_port.render_intent.as_deref());
            let values = if intent == Some("Color") { area * 3 } else { area };
            bytes += values * 4;
        }
    }
    let megabytes = bytes / 1024 / 1024;
    let size = super::data_view::stats::byte_size((megabytes << 20) as usize);
    format!("Bake Selected ({} nodes, {size})", chain.len())
}

pub fn node_menu(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    menu_style(ui);
    let has_selection = !editor.selection.is_empty();
    if menu_item_enabled(ui, Some("rotate-360"), Some(theme::ACCENT_GREEN), "Refresh", "F5", has_selection).clicked() {
        commands.push(Command::Refresh);
    }
    menu_separator(ui);
    if menu_item_enabled(ui, Some("road-sign"), None, "Bypass", "B", has_selection).clicked() {
        commands.push(Command::Bypass);
    }
    if menu_item_enabled(ui, None, None, "Lock Preview to Node", "F", has_selection).clicked() {
        commands.push(Command::LockPreview);
    }
    // The item is checked when the node exports (`miExport.IsChecked`).
    let exporting = editor.active_node().and_then(|id| editor.document.export_settings(id)).is_some_and(|settings| settings.enabled);
    let export_accent = if exporting { Some(theme::ACCENT_PRIMARY) } else { None };
    if menu_item_enabled(ui, Some("device-floppy"), export_accent, "Mark for Export", "F3", has_selection).clicked() {
        commands.push(Command::MarkForExport);
    }
    if menu_item_enabled(ui, None, None, "Reset", "Ctrl R", has_selection).clicked() {
        commands.push(Command::ResetNode);
    }
    if menu_item_enabled(ui, None, None, "Mutate", "", has_selection).clicked() {
        commands.push(Command::Graph(GraphCommand::Mutate));
    }
    menu_separator(ui);
    if menu_item_enabled(ui, Some("picture-in-picture"), Some(theme::ACCENT_MUTED_BLUE), "Locked 2D Viewport", "Ctrl F6", has_selection).clicked() {
        commands.push(Command::ShowLocked2DViewport);
    }
    menu_separator(ui);
    if menu_item(ui, Some("arrow-left-rhombus"), Some(theme::ACCENT_PURPLE), "Select Upsteam", "").clicked() {
        commands.push(Command::SelectUpstream);
    }
    if menu_item(ui, Some("arrow-right-rhombus"), Some(theme::ACCENT_PURPLE), "Select Downstream", "").clicked() {
        commands.push(Command::SelectDownstream);
    }
    menu_separator(ui);
    if menu_item(ui, Some("layers-linked"), None, "Group / Ungroup", "").clicked() {
        commands.push(Command::Graph(GraphCommand::GroupUngroup));
    }
    submenu(ui, Some("arrows-join-2"), Some(theme::ACCENT_MUTED_ORANGE), "Combine nodes", |ui| {
        menu_style(ui);
        let kinds = [("Combine as Inputs", CombineKind::Inputs), ("Mixer as Color Layers", CombineKind::Mixer), ("Layers as Normal Layers", CombineKind::Layers)];
        for (item, kind) in kinds {
            if menu_item(ui, None, None, item, "").clicked() {
                commands.push(Command::Graph(GraphCommand::Combine(kind)));
            }
        }
    });
    menu_separator(ui);
    submenu(ui, Some("keyframe"), None, "Convert port to portals", |ui| {
        convert_port_items(ui, editor, commands);
    });
    menu_separator(ui);
    // The tab shown is the selected node's.
    let current_tab = editor.active_node().and_then(|id| editor.document.node(id)).map_or(0, node_tab);
    submenu(ui, None, None, "Move to Tab", |ui| {
        if let Some(target) = tab_targets(ui, editor, current_tab) {
            commands.push(Command::Graph(GraphCommand::MoveToTab(target)));
        }
    });
    menu_separator(ui);
    submenu(ui, None, None, "Presets", |ui| {
        menu_style(ui);
        crate::ui::presets::presets_menu(ui, editor, commands);
    });
}

pub fn preview_menu(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    menu_style(ui);
    menu_caption(ui, "Resolution");
    for size in PREVIEW_RESOLUTIONS {
        let checked = editor.preview_resolution == size;
        if check_item(ui, checked, &format!("{size} x {size}"), "").clicked() {
            commands.push(Command::SetPreviewResolution(size));
        }
    }
    menu_separator(ui);
    // `miRegions` and `sepRegion` (`RegionPreviewMenuVisibility`), once
    // the terrain has regions.
    if crate::ui::regions::menus_visible(editor) {
        submenu(ui, None, None, "Regions", |ui| crate::ui::regions::regions_menu(ui, editor, commands, true));
        menu_separator(ui);
    }
    let engine_text = if editor.engine_active { "Engine Active" } else { "Engine Suspended" };
    if menu_item(ui, Some("engine"), Some(theme::ACCENT_GREEN), engine_text, "").clicked() {
        commands.push(Command::ToggleEngine);
    }
    if menu_item(ui, Some("hexagon-off"), Some(theme::ACCENT_BLUE), "Clean Memory", "").clicked() {
        commands.push(Command::CleanMemory);
    }
    if menu_item(ui, None, None, "Clear Cache", "").clicked() {
        commands.push(Command::ClearCache);
    }
    menu_separator(ui);
    if menu_item(ui, None, None, "Rebuild All", "").clicked() {
        commands.push(Command::RefreshAll);
    }
    menu_separator(ui);
    // Screenshots and turntable frames are saved as files, which the web
    // build cannot write.
    if !cfg!(target_arch = "wasm32") {
        menu_caption(ui, "Screenshot");
        if menu_item(ui, Some("camera"), Some(theme::ACCENT_STRONG_GREEN), "Save Screenshot", "F9").clicked() {
            commands.push(Command::SaveScreenshot);
        }
        submenu(ui, None, None, "Screenshot Options", |ui| {
            menu_style(ui);
            crate::ui::viewport_area::screenshot_options(ui, editor, commands);
        });
        menu_separator(ui);
        if menu_item(ui, Some("view-360-number"), None, "Record Turntable Animation", "").clicked() {
            commands.push(Command::ShowTurntable);
        }
        menu_separator(ui);
    }
    menu_caption(ui, "Preview Renderer");
    let modes = [(RenderMode::Data, "Visualize Data (Render Off)"), (RenderMode::Clay, "Clay Render"), (RenderMode::Realistic, "Realistic Render")];
    for (mode, text) in modes {
        if check_item(ui, editor.render_mode == mode, text, "").clicked() {
            commands.push(Command::SetRenderMode(mode));
        }
    }
}

pub fn project_menu(ui: &mut Ui, commands: &mut Vec<Command>) {
    menu_style(ui);
    // Builds are written to folders, which the web build does not have.
    let web = cfg!(target_arch = "wasm32");
    if !web {
        if menu_item(ui, Some("atom2"), Some(theme::ACCENT_GREEN), "Build and Export...", "Ctrl Shift B").clicked() {
            commands.push(Command::BuildAndExport);
        }
        if menu_item(ui, None, Some(theme::ACCENT_MAGENTA), "Execute Batch Build...", "").clicked() {
            commands.push(Command::BatchBuild);
        }
        menu_item_enabled(ui, Some("timeline-event-plus"), Some(theme::ACCENT_MAGENTA), "Execute Network Build...", "", false);
        menu_separator(ui);
    }
    if menu_item(ui, Some("settings"), Some(theme::ACCENT_BLUE), "Build Settings...", "Ctrl B").clicked() {
        commands.push(Command::BuildSettings);
    }
    menu_separator(ui);
    if !web {
        menu_caption(ui, "Build Stack");
        if menu_item(ui, None, None, "Open Build Folder", "").clicked() {
            commands.push(Command::OpenBuildFolder);
        }
        menu_separator(ui);
    }
    menu_caption(ui, "Reports");
    // `ShowStatistics`: the Data View's Build Statistics window.
    if menu_item(ui, Some("histogram"), None, "Statistics", "").clicked() {
        commands.push(Command::ShowBuildStatistics);
    }
}

/// `RecreateMRU`: "Recent Files", then each recent file that is still
/// there, numbered, by name, with its path and change time as the tooltip;
/// then Open Autosave….
fn recent_files(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    menu_caption(ui, "Recent Files");
    let existing = editor.user.recent.files().iter().filter(|file| file.is_file());
    for (index, file) in existing.enumerate() {
        let icon = format!("number-{}-small", index + 1);
        let name = file.file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_default();
        let response = menu_item(ui, Some(&icon), None, &name, "");
        let modified = file.metadata().and_then(|metadata| metadata.modified()).ok();
        let modified_text = modified.map(|time| chrono::DateTime::<chrono::Local>::from(time).format("%-m/%-d/%Y %-I:%M:%S %p").to_string()).unwrap_or_default();
        let response = response.on_hover_text(format!("{}\nModified: {modified_text}", file.display()));
        if response.clicked() {
            commands.push(Command::OpenRecent(file.clone()));
        }
    }
    menu_separator(ui);
    if menu_item(ui, None, None, "Open Autosave...", "").clicked() {
        commands.push(Command::File(FileCommand::OpenAutosave));
    }
}

/// `miUiScale`: the scales on offer (`UIScaleMenu_Click`); its title
/// shows the current one (`UpdateUIScale`).
const UI_SCALES: [(&str, f32); 8] = [("80%", 0.8), ("100%", 1.0), ("110%", 1.1), ("120%", 1.2), ("125%", 1.25), ("130%", 1.3), ("150%", 1.5), ("175%", 1.75)];

pub fn tools_menu(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    menu_style(ui);
    let percent = (editor.user.options.ui_scale() * 100.0).round();
    let title = format!("Interface Scale ({percent}%)");
    submenu(ui, None, None, &title, |ui| {
        menu_style(ui);
        for (text, scale) in UI_SCALES {
            if menu_item(ui, None, None, text, "").clicked() {
                commands.push(Command::SetUiScale(scale));
            }
        }
    });
    menu_separator(ui);
    // Diagnostics restart Nerthus and read its folders, the log files and
    // the MCP server's socket are not there in a browser: the web build
    // keeps the Console.
    let web = cfg!(target_arch = "wasm32");
    if !web {
        if menu_item(ui, Some("report-medical"), None, "Run Diagnostics...", "").clicked() {
            commands.push(Command::Tools(ToolCommand::ShowDiagnostics));
        }
        if menu_item(ui, None, None, "Artifact Explorer...", "").clicked() {
            commands.push(Command::Tools(ToolCommand::ShowArtifactExplorer));
        }
        menu_separator(ui);
    }
    if menu_item(ui, Some("terminal-2"), Some(theme::ACCENT_MUTED_GREEN), "Console", "F12").clicked() {
        commands.push(Command::Tools(ToolCommand::ShowConsole));
    }
    if !web {
        if check_item(ui, crate::logging::verbose(), "Verbose Logging", "").clicked() {
            commands.push(Command::Tools(ToolCommand::ToggleVerboseLogging));
        }
        let mcp_hint = match &editor.mcp.address {
            Some(address) => format!("Listening on {address}"),
            None => "Lets AI assistants (MCP clients such as Claude Code) edit and build the open project".to_string(),
        };
        let mcp_item = check_item(ui, editor.mcp.address.is_some(), "MCP Server", "");
        if mcp_item.on_hover_text(mcp_hint).clicked() {
            commands.push(Command::Tools(ToolCommand::ToggleMcpServer));
        }
    }
    menu_separator(ui);
    if menu_item(ui, Some("settings-2"), Some(theme::ACCENT_MUTED_PURPLE), "Options...", "Ctrl Alt O").clicked() {
        commands.push(Command::ShowOptions(None));
    }
}

pub fn help_menu(ui: &mut Ui, commands: &mut Vec<Command>) {
    menu_style(ui);
    if menu_item(ui, Some("help-square-rounded"), Some(theme::ACCENT_BLUE), "User Guide", "").clicked() {
        commands.push(Command::Tools(ToolCommand::ShowDocs(crate::docs::HOME.to_string())));
    }
    if menu_item(ui, Some("info-square-rounded"), None, "Node Reference", "").clicked() {
        commands.push(Command::Tools(ToolCommand::ShowDocs(crate::docs::NODE_REFERENCE.to_string())));
    }
    if menu_item(ui, Some("keyboard"), None, "Shortcut Reference", "").clicked() {
        commands.push(Command::Tools(ToolCommand::ShowShortcuts));
    }
    menu_separator(ui);
    if menu_item(ui, None, None, "What's new in this version?", "").clicked() {
        commands.push(Command::Tools(ToolCommand::ShowWhatsNew));
    }
    // The web version is always the one on the page, and has no
    // diagnostics (see the Tools menu).
    if !cfg!(target_arch = "wasm32") {
        if menu_item(ui, None, None, "Check for updates", "").clicked() {
            commands.push(Command::Tools(ToolCommand::CheckForUpdates));
        }
        menu_separator(ui);
        if menu_item(ui, Some("report-medical"), None, "Diagnostics...", "").clicked() {
            commands.push(Command::Tools(ToolCommand::ShowDiagnostics));
        }
    }
    menu_separator(ui);
    if menu_item(ui, Some("versions"), None, "About Nerthus", "").clicked() {
        commands.push(Command::Tools(ToolCommand::ShowAbout));
    }
}
