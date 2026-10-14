//! Build Settings (`BuildSettingsWindow`): the build definition's region,
//! resolution, subdivision and output, destination and options, the tiles'
//! naming and layout, the nodes marked for export (`SavablesArea`), the
//! post-build script, the terrain's size, its regions and the build
//! profiles.

use egui::{Align, Color32, Id, Layout, RichText, Stroke, Ui, Vec2};
use nerthus_engine::region::portion_json;
use nerthus_engine::Region;
use nerthus_export::build::{preview_destination, BuildDefinition, OutputType, Subdivision};
use nerthus_export::destination::{TEMPLATE_PRESETS, TOKEN_HELP};
use nerthus_export::tiles::{tile_suffix, OrganizeFiles, TILE_PATTERN_PRESETS};
use nerthus_export::FileFormat;
use serde_json::{json, Value};

use crate::document::ExportSettings;
use crate::editor::Editor;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::input_box::{InputAnswer, InputBox};
use crate::ui::regions::{self, RegionsState};
use crate::ui::tool_commands::ToolCommand;
use crate::ui::tool_window;
use crate::ui::widgets::{icon_menu, link_button, menu_item, menu_separator};

/// `BuildSettingsWindow`'s tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    Resolution,
    Build,
    Tiles,
    Nodes,
    Script,
    Terrain,
    Regions,
    Profiles,
}

/// The tabs; Tiles is enabled with tiled output (`TileControlsVisibility`).
const TABS: [(SettingsTab, &str, bool); 8] = [
    (SettingsTab::Resolution, "Resolution", true),
    (SettingsTab::Build, "Build", true),
    (SettingsTab::Tiles, "Tiles", true),
    (SettingsTab::Nodes, "Nodes", true),
    (SettingsTab::Script, "Script", true),
    (SettingsTab::Terrain, "Terrain", true),
    (SettingsTab::Regions, "Regions", true),
    (SettingsTab::Profiles, "Profiles", true),
];

/// `cboBlend`: the margin each side of a bucket, as a share of it.
const BLENDINGS: [(f32, &str); 4] = [(0.0, "0"), (0.25, "25%"), (0.5, "50%"), (1.0, "100%")];

/// `cboTileOrganization`.
const ORGANIZATIONS: [(OrganizeFiles, &str); 3] = [
    (OrganizeFiles::None, "All files in same folder"),
    (OrganizeFiles::NodeSubFolder, "Folders for each Node"),
    (OrganizeFiles::NodeAndPortSubFolder, "Node folders with Port subfolders"),
];

/// `cboOverlapingPixels`: 0 to 16.
const MOST_OVERLAP: usize = 16;

/// `cboResolution`: the resolutions offered.
const RESOLUTIONS: [(usize, &str); 18] = [
    (512, "512 x 512"),
    (1024, "1K (1024)"),
    (2048, "2K (2048)"),
    (4096, "4K (4096)"),
    (8192, "8K (8192)"),
    (16384, "16K (16,384)"),
    (24576, "24K (24,576)"),
    (32768, "32K (32,768)"),
    (49152, "48K (49,152)"),
    (65536, "64K (65,536)"),
    (73728, "72K (73,728)"),
    (98304, "96K (98,304)"),
    (131072, "128K (131,072)"),
    (147456, "144K (147,456)"),
    (163840, "160K (163,840)"),
    (196608, "192K (196,608)"),
    (229376, "224K (229,376)"),
    (262144, "256K (262,144)"),
];

/// `Colorspace`, in order.
const COLOR_SPACES: [&str; 40] = [
    "Undefined", "CMY", "CMYK", "Gray", "HCL", "HCLp", "HSB", "HSI", "HSL", "HSV", "HWB", "Lab", "LCH", "LCHab", "LCHuv", "Log", "LMS", "Luv", "OHTA", "Rec601YCbCr", "Rec709YCbCr", "RGB", "scRGB", "sRGB", "Transparent", "XyY", "XYZ", "YCbCr", "YCC", "YDbDr", "YIQ", "YPbPr", "YUV", "LinearGray", "Jzazbz", "DisplayP3", "Adobe98", "ProPhoto", "Oklab", "Oklch",
];

/// The window's state while it is open.
#[derive(Clone, Debug)]
pub struct BuildSettingsState {
    pub tab: SettingsTab,
    /// The destination as typed (saved as it changes).
    destination: Option<String>,
    /// The tile suffix pattern as typed (saved as it changes).
    tile_pattern: Option<String>,
    /// File names being edited, by node.
    filenames: Vec<(i32, String)>,
    /// The subdivision, output and tile size were brought in step on
    /// opening.
    laid_out: bool,
    /// The Regions tab's canvas.
    pub regions: RegionsState,
    /// The Profiles tab: the profile selected (`lstProfiles`), the name
    /// Save As asks for, and the question waiting for Yes or No.
    profile: Option<String>,
    profile_name: Option<InputBox>,
    profile_question: Option<ProfileQuestion>,
}

/// What the Profiles tab asks before it changes a profile.
#[derive(Clone, Debug, PartialEq)]
enum ProfileQuestion {
    /// "A profile with this name already exists. Do you want to overwrite
    /// it?"
    Overwrite(String),
    /// "Are you sure you want to delete this profile?"
    Delete(String),
}

impl BuildSettingsState {
    pub fn new(tab: SettingsTab) -> Self {
        Self {
            tab,
            destination: None,
            tile_pattern: None,
            filenames: Vec::new(),
            laid_out: false,
            regions: RegionsState::default(),
            profile: None,
            profile_name: None,
            profile_question: None,
        }
    }

    /// The build settings changed under the window (a profile loaded):
    /// what it typed and laid out is read again.
    fn reload(&mut self) {
        self.destination = None;
        self.tile_pattern = None;
        self.laid_out = false;
    }
}

/// A section's title bar (`TextBlock` on `Panel-Border`, semibold).
fn section(ui: &mut Ui, title: &str) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 24.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::PANEL_BORDER);
    let galley = ui.painter().layout_no_wrap(title.to_string(), theme::semibold(12.0), theme::LABEL_EMPHASIS);
    let position = egui::Pos2::new(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(position, galley, theme::LABEL_EMPHASIS);
    ui.add_space(8.0);
}

/// `ListBox-OptionsX`: one segment per choice; returns the one clicked.
fn options(ui: &mut Ui, choices: &[(&str, bool)], current: usize) -> Option<usize> {
    let mut chosen = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(2.0, 2.0);
        for (index, (name, enabled)) in choices.iter().enumerate() {
            let selected = index == current;
            let text_color = if selected { theme::LABEL_EMPHASIS } else { theme::LABEL_DEFAULT };
            let text = RichText::new(*name).font(theme::interface(11.0)).color(text_color);
            let mut button = egui::Button::new(text);
            button = button.fill(if selected { theme::OPTIONS_SELECTION } else { theme::BUTTON_BACKGROUND });
            button = button.stroke(Stroke::new(1.0, theme::BUTTON_BORDER));
            button = button.min_size(Vec2::new(0.0, 24.0));
            if ui.add_enabled(*enabled, button).clicked() && !selected {
                chosen = Some(index);
            }
        }
    });
    chosen
}

fn caption(ui: &mut Ui, text: &str, color: Color32) {
    let label = RichText::new(text).font(theme::interface(12.0)).color(color);
    ui.label(label);
}

/// The window: tabs on the left, the tab's page, Execute Build, Copy
/// Command Line and Close below.
pub fn show(ctx: &egui::Context, editor: &mut Editor, state: &mut Option<BuildSettingsState>, commands: &mut Vec<Command>) {
    let Some(settings) = state.as_mut() else {
        return;
    };
    // The bucket and tile sizes follow the subdivision and output; they are
    // worked out once on opening, when the saved ones differ.
    if !settings.laid_out {
        settings.laid_out = true;
        let definition = editor.document.build_definition();
        let tiled = definition.output_type == OutputType::CroppedTiles;
        let region = selected_region(editor, &definition);
        apply_layout(editor, definition.resolution, definition.subdivision, tiled, definition.tile_resolution, region.as_ref());
    }
    let mut open = true;
    let mut close = false;
    let mut window = egui::Window::new("Build Settings and Regions");
    window = window.collapsible(false);
    window = window.resizable(true);
    window = tool_window::place(ctx, window, Id::new("build-settings"), Vec2::new(920.0, 640.0));
    window = window.open(&mut open);
    window.show(ctx, |ui| {
        let footer_height = 36.0;
        let body_height = (ui.available_height() - footer_height).max(200.0);
        ui.horizontal(|ui| {
            ui.set_min_height(body_height);
            let tiled = editor.document.build_definition().output_type == OutputType::CroppedTiles;
            if settings.tab == SettingsTab::Tiles && !tiled {
                settings.tab = SettingsTab::Resolution;
            }
            ui.vertical(|ui| {
                ui.set_width(120.0);
                for (tab, name, enabled) in TABS {
                    let selected = settings.tab == tab;
                    let enabled = if tab == SettingsTab::Tiles { tiled } else { enabled };
                    let response = ui.add_enabled(enabled, egui::Button::selectable(selected, name));
                    if response.clicked() {
                        settings.tab = tab;
                    }
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                let mut page = egui::ScrollArea::vertical();
                page = page.id_salt("build-settings-page");
                page = page.max_height(body_height);
                page = page.auto_shrink([false, false]);
                page.show(ui, |ui| match settings.tab {
                    SettingsTab::Resolution => resolution_tab(ui, editor),
                    SettingsTab::Build => build_tab(ui, editor, settings),
                    SettingsTab::Nodes => nodes_tab(ui, editor, settings, commands),
                    SettingsTab::Script => script_tab(ui, editor, commands),
                    SettingsTab::Terrain => terrain_tab(ui, editor),
                    SettingsTab::Tiles => tiles_tab(ui, editor, settings),
                    SettingsTab::Regions => regions::regions_tab(ui, editor, &mut settings.regions, commands),
                    SettingsTab::Profiles => profiles_tab(ui, editor, settings),
                });
            });
        });
        ui.separator();
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Close").clicked() {
                close = true;
            }
            if ui.button("Copy Command Line").clicked() {
                let file = editor.document.path.as_ref().map(|path| path.display().to_string()).unwrap_or_default();
                ui.ctx().copy_text(format!("nerthus-build build \"{file}\""));
                editor.report("Copied the build command line");
            }
            if ui.button("Execute Build").clicked() {
                commands.push(Command::BuildAndExport);
                close = true;
            }
        });
    });
    profile_dialogs(ctx, editor, settings);
    if close || !open {
        *state = None;
    }
}

/// The Profiles tab: the profiles on the left, the selected one's settings
/// on the right (`ShowProfile`), and Load, Save, Save As… and Delete below.
fn profiles_tab(ui: &mut Ui, editor: &mut Editor, settings: &mut BuildSettingsState) {
    section(ui, "Build Profiles");
    let names = editor.document.build_profiles();
    if settings.profile.as_ref().is_some_and(|selected| !names.contains(selected)) {
        settings.profile = None;
    }
    let list_height = (ui.available_height() - 48.0).max(240.0);
    ui.columns(2, |columns| {
        let list = egui::Frame::new().stroke(Stroke::new(1.0, theme::BUTTON_BORDER)).inner_margin(4.0);
        list.show(&mut columns[0], |ui| {
            ui.set_min_height(list_height);
            ui.set_width(ui.available_width());
            for name in &names {
                let selected = settings.profile.as_deref() == Some(name.as_str());
                if ui.selectable_label(selected, name.as_str()).clicked() {
                    settings.profile = Some(name.clone());
                }
            }
        });
        let summary = settings.profile.as_deref().and_then(|name| editor.document.build_profile(name)).map(|definition| nerthus_export::swarm::profile_summary(&definition));
        let mut shown = summary.unwrap_or_default().replace("\r\n", "\n");
        let mut field = egui::TextEdit::multiline(&mut shown);
        field = field.font(theme::monospace(14.0));
        field = field.text_color(theme::ACCENT_MUTED_GREEN);
        field = field.desired_width(f32::INFINITY);
        field = field.desired_rows(16);
        field = field.interactive(false);
        columns[1].add_sized(Vec2::new(columns[1].available_width(), list_height), field);
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let selected = settings.profile.clone();
        let chosen = selected.is_some();
        if ui.add_enabled(chosen, egui::Button::new("Load")).clicked()
            && let Some(name) = &selected
        {
            editor.document.load_build_profile(name);
            editor.report(format!("Loaded the build profile {name}"));
            settings.reload();
        }
        if ui.add_enabled(chosen, egui::Button::new("Save")).clicked()
            && let Some(name) = &selected
        {
            editor.document.save_build_profile(name);
        }
        if ui.button("Save As...").clicked() {
            let current = selected.clone().unwrap_or_default();
            settings.profile_name = Some(InputBox::new("Profile name", &current));
        }
        if ui.add_enabled(chosen, egui::Button::new("Delete")).clicked()
            && let Some(name) = selected
        {
            settings.profile_question = Some(ProfileQuestion::Delete(name));
        }
    });
}

/// Save As's name and the tab's questions, over the window.
fn profile_dialogs(ctx: &egui::Context, editor: &mut Editor, settings: &mut BuildSettingsState) {
    if let Some(input) = settings.profile_name.as_mut() {
        match input.show(ctx, "profile-name") {
            Some(InputAnswer::Ok(name)) => {
                settings.profile_name = None;
                if editor.document.build_profiles().contains(&name) {
                    settings.profile_question = Some(ProfileQuestion::Overwrite(name));
                } else {
                    editor.document.save_build_profile(&name);
                    settings.profile = None;
                }
            }
            Some(InputAnswer::Cancel) => settings.profile_name = None,
            None => {}
        }
        return;
    }
    let Some(question) = settings.profile_question.clone() else {
        return;
    };
    let (caption_text, text) = match &question {
        ProfileQuestion::Overwrite(name) => (format!("Overwrite {name}"), "A profile with this name already exists. Do you want to overwrite it?"),
        ProfileQuestion::Delete(name) => (format!("Delete {name}"), "Are you sure you want to delete this profile?"),
    };
    let mut answer = None;
    let modal = egui::Modal::new(Id::new("profile-question"));
    let response = modal.show(ctx, |ui| {
        ui.set_width(420.0);
        ui.label(RichText::new(caption_text).font(theme::semibold(13.0)).color(theme::LABEL_EMPHASIS));
        ui.add_space(8.0);
        ui.label(RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT));
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if ui.button("No").clicked() {
                answer = Some(false);
            }
            if ui.button("Yes").clicked() {
                answer = Some(true);
            }
        });
    });
    if answer == Some(true) {
        match &question {
            ProfileQuestion::Overwrite(name) => editor.document.save_build_profile(name),
            ProfileQuestion::Delete(name) => editor.document.delete_build_profile(name),
        }
        settings.profile = None;
    }
    if answer.is_some() || response.should_close() {
        settings.profile_question = None;
    }
}

/// What Build Settings keeps in step with the resolution, the subdivision,
/// the output and the tile size, as the window does between
/// `GenerateBuckets`, `BucketChanged`, `GenerateTiles`, `TileChanged` and
/// `SaveBuildDefinition`: a subdivision the resolution rules out falls
/// back (to None, or to the next choice when the chosen one is merely
/// unavailable), the bucket size follows the subdivision, the tile size is
/// one Build Settings offers for it, and the build type and output type
/// follow the choices (Hybrid or Tiled only with a subdivision). With a
/// region chosen the build is a Region build of single images.
fn layout_members(resolution: usize, subdivision: Subdivision, tiled: bool, tile_resolution: usize, region: Option<&Region>) -> Vec<(&'static str, Value)> {
    let too_small = match subdivision {
        Subdivision::Slower => resolution < 4096,
        Subdivision::Balanced => resolution < 2048,
        Subdivision::Faster => resolution < 1024,
        Subdivision::None => false,
    };
    let mut subdivision = if too_small { Subdivision::None } else { subdivision };
    if !subdivision.available_at(resolution) {
        let index = Subdivision::ALL.iter().position(|choice| *choice == subdivision).unwrap_or(0);
        subdivision = Subdivision::ALL.get(index + 1).copied().filter(|next| next.available_at(resolution)).unwrap_or(Subdivision::None);
    }
    let bucket_resolution = subdivision.bucket_resolution(resolution);
    let sizes = tile_sizes(resolution, bucket_resolution);
    let tile_resolution = if sizes.contains(&tile_resolution) { tile_resolution } else { sizes.first().copied().unwrap_or(tile_resolution) };
    let build_type = match (subdivision, tiled) {
        _ if region.is_some() => "Region",
        (Subdivision::None, _) => "Standard",
        (_, false) => "Hybrid",
        (_, true) => "Tiled",
    };
    let output_type = if tiled && region.is_none() { OutputType::CroppedTiles } else { OutputType::SingleImage };
    vec![
        ("Type", Value::from(build_type)),
        ("Subdivision", Value::from(subdivision.name())),
        ("Resolution", json!(resolution)),
        ("TileResolution", json!(tile_resolution)),
        ("BucketResolution", json!(bucket_resolution)),
        ("OutputType", Value::from(output_type.name())),
        ("NumberOfTiles", json!(resolution / tile_resolution.max(1))),
    ]
}

/// `GenerateTiles`: from the bucket's size (half the terrain without a
/// subdivision) down by halves, larger than 64.
fn tile_sizes(resolution: usize, bucket_resolution: usize) -> Vec<usize> {
    let mut size = if bucket_resolution == resolution { resolution / 2 } else { bucket_resolution };
    let mut sizes = Vec::new();
    while size > 64 {
        sizes.push(size);
        size /= 2;
    }
    sizes
}

fn apply_layout(editor: &mut Editor, resolution: usize, subdivision: Subdivision, tiled: bool, tile_resolution: usize, region: Option<&Region>) {
    let members = layout_members(resolution, subdivision, tiled, tile_resolution, region);
    editor.document.set_build_settings(&members);
}

/// The region the Region box shows (`LoadProjectArea`): the terrain's
/// region the definition's `Portion` names; none (Whole Terrain) when it
/// names none the terrain has.
fn selected_region(editor: &Editor, definition: &BuildDefinition) -> Option<Region> {
    let portion = definition.portion.as_ref()?;
    editor.document.regions().into_iter().find(|region| region.name == portion.name)
}

/// `cboResolution`: one of the resolutions offered.
fn resolution_choice(ui: &mut Ui, current: usize) -> Option<usize> {
    caption(ui, "Resolution", theme::LABEL_DEFAULT);
    let current_name = RESOLUTIONS.iter().find(|(size, _)| *size == current).map(|(_, name)| *name).unwrap_or("Custom");
    let mut chosen = None;
    let mut combo = egui::ComboBox::from_id_salt("build-resolution");
    combo = combo.selected_text(current_name);
    combo = combo.width(260.0);
    combo.show_ui(ui, |ui| {
        for (size, name) in RESOLUTIONS {
            if ui.selectable_label(size == current, name).clicked() {
                chosen = Some(size);
            }
        }
    });
    chosen
}

/// `cboRegionSelector` (`ApplyRegionSelection`, then `SaveBuildDefinition`):
/// Whole Terrain, or a region to build on its own as single images.
fn region_choice(ui: &mut Ui, editor: &mut Editor, definition: &BuildDefinition, selected: Option<&Region>) {
    caption(ui, "Region", theme::LABEL_DEFAULT);
    let regions = editor.document.regions();
    let current = selected.map(|region| region.name.clone()).unwrap_or_else(|| "Whole Terrain".to_string());
    let mut chosen: Option<Option<Region>> = None;
    let mut combo = egui::ComboBox::from_id_salt("build-region");
    combo = combo.selected_text(current);
    combo = combo.width(260.0);
    combo.show_ui(ui, |ui| {
        if ui.selectable_label(selected.is_none(), "Whole Terrain").clicked() {
            chosen = Some(None);
        }
        for region in &regions {
            let is_selected = selected.is_some_and(|selected| selected.name == region.name);
            if ui.selectable_label(is_selected, region.name.as_str()).clicked() {
                chosen = Some(Some(region.clone()));
            }
        }
    });
    let Some(choice) = chosen else {
        return;
    };
    let tiled = definition.output_type == OutputType::CroppedTiles;
    let mut members = layout_members(definition.resolution, definition.subdivision, tiled, definition.tile_resolution, choice.as_ref());
    let portion = choice.as_ref().map(portion_json).unwrap_or(Value::Null);
    members.push(("Portion", portion));
    editor.document.set_build_settings(&members);
}

fn resolution_tab(ui: &mut Ui, editor: &mut Editor) {
    section(ui, "Build Definition");
    let definition = editor.document.build_definition();
    let tiled = definition.output_type == OutputType::CroppedTiles;
    let selected = selected_region(editor, &definition);
    region_choice(ui, editor, &definition, selected.as_ref());
    ui.add_space(16.0);

    // With a region, the resolution is the region's size and there is no
    // subdivision or output to choose (`cboResolution_OnSelectionChanged`).
    if let Some(region) = selected {
        if let Some(size) = resolution_choice(ui, region.size.max(0) as usize) {
            let mut resized = region.clone();
            resized.size = size as i32;
            editor.document.update_region(&region.name, resized);
        }
        // `CheckRegionSelectBuildDefWarning`.
        if definition.resolution <= 2048 {
            ui.add_space(12.0);
            regions::warning(ui, "Regions require the base resolution of the terrain to be at least 4096.");
        }
        return;
    }
    if let Some(size) = resolution_choice(ui, definition.resolution) {
        apply_layout(editor, size, definition.subdivision, tiled, definition.tile_resolution, None);
    }

    ui.add_space(16.0);
    caption(ui, "Hybrid Subdivision", theme::LABEL_DEFAULT);
    let subdivisions: Vec<(&str, bool)> = Subdivision::ALL.iter().map(|choice| (choice.name(), choice.available_at(definition.resolution))).collect();
    let current_subdivision = Subdivision::ALL.iter().position(|choice| *choice == definition.subdivision).unwrap_or(0);
    if let Some(index) = options(ui, &subdivisions, current_subdivision) {
        apply_layout(editor, definition.resolution, Subdivision::ALL[index], tiled, definition.tile_resolution, None);
    }
    ui.add_space(16.0);
    caption(ui, "Output", theme::LABEL_DEFAULT);
    let outputs = [("Single Image", true), ("Tiled Images", true)];
    if let Some(index) = options(ui, &outputs, usize::from(tiled)) {
        apply_layout(editor, definition.resolution, definition.subdivision, index == 1, definition.tile_resolution, None);
    }

    if tiled {
        ui.add_space(16.0);
        tile_size_choice(ui, editor, &definition);
        if definition.subdivision != Subdivision::None {
            ui.add_space(16.0);
            blending_choice(ui, editor, &definition);
        }
    }

    // `UpdateTileLabels`.
    ui.add_space(8.0);
    let per_side = definition.resolution / definition.bucket_resolution.max(1);
    if per_side > 1 {
        let divisions = per_side * per_side;
        caption(ui, &format!("{divisions} divisions"), theme::MENU_SEPARATOR);
    }
    let tiles_per_side = definition.resolution / definition.tile_resolution.max(1);
    if tiled && tiles_per_side > 1 {
        let count = grouped(tiles_per_side * tiles_per_side);
        caption(ui, &format!("{tiles_per_side} x {tiles_per_side} ({count} tiles)"), theme::MENU_SEPARATOR);
    }

    // `CheckResolutionWarning`.
    if definition.resolution == 16384 && definition.subdivision == Subdivision::None {
        ui.add_space(12.0);
        caption(ui, "WARNING: 16K builds require an enormous amount of RAM.", theme::ACCENT_RED);
        caption(ui, "Complex builds may require over 128GB RAM, and GPU nodes may require over 12GB RAM.", theme::ACCENT_RED);
    }
}

/// `cboTile`: the sizes `GenerateTiles` offers.
fn tile_size_choice(ui: &mut Ui, editor: &mut Editor, definition: &BuildDefinition) {
    caption(ui, "Tile Size", theme::LABEL_DEFAULT);
    let sizes = tile_sizes(definition.resolution, definition.bucket_resolution);
    let mut chosen = None;
    let mut combo = egui::ComboBox::from_id_salt("build-tile-size");
    combo = combo.selected_text(format!("{0} x {0}", definition.tile_resolution));
    combo = combo.width(260.0);
    combo.show_ui(ui, |ui| {
        for size in sizes {
            if ui.selectable_label(size == definition.tile_resolution, format!("{size} x {size}")).clicked() {
                chosen = Some(size);
            }
        }
    });
    if let Some(size) = chosen {
        apply_layout(editor, definition.resolution, definition.subdivision, true, size, None);
    }
}

/// `cboBlend`: `EdgeBlending`.
fn blending_choice(ui: &mut Ui, editor: &mut Editor, definition: &BuildDefinition) {
    caption(ui, "Blending", theme::LABEL_DEFAULT);
    let current = BLENDINGS.iter().find(|(share, _)| *share == definition.edge_blending).map(|(_, name)| *name).unwrap_or("");
    let mut chosen = None;
    let mut combo = egui::ComboBox::from_id_salt("build-blending");
    combo = combo.selected_text(current);
    combo = combo.width(260.0);
    combo.show_ui(ui, |ui| {
        for (share, name) in BLENDINGS {
            if ui.selectable_label(share == definition.edge_blending, name).clicked() {
                chosen = Some(share);
            }
        }
    });
    if let Some(share) = chosen {
        editor.document.set_build_setting("EdgeBlending", json!(share));
    }
}

/// `###,###`.
fn grouped(value: usize) -> String {
    let digits = value.to_string();
    let mut text = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

/// The Tiles tab (`stackTiles`): how tiles are named and laid out.
fn tiles_tab(ui: &mut Ui, editor: &mut Editor, settings: &mut BuildSettingsState) {
    section(ui, "Tiled Output Settings");
    let definition = editor.document.build_definition();
    caption(ui, "Tile Suffix Pattern", theme::LABEL_DEFAULT);
    let typed = settings.tile_pattern.get_or_insert_with(|| definition.tile_pattern.clone());
    ui.horizontal(|ui| {
        let mut field = egui::TextEdit::singleline(typed);
        field = field.font(theme::monospace(12.0));
        field = field.text_color(theme::TEXTBOX_FILE_FOREGROUND);
        field = field.desired_width(220.0);
        if ui.add(field).changed() {
            editor.document.set_build_setting("TilePattern", Value::String(typed.clone()));
        }
        icon_menu(ui, "chevron-down", "Presets", None, |ui| {
            for preset in TILE_PATTERN_PRESETS {
                if menu_item(ui, None, None, preset, "").clicked() {
                    *typed = preset.to_string();
                    editor.document.set_build_setting("TilePattern", Value::String(typed.clone()));
                    ui.close();
                }
            }
        });
    });
    // `UpdateTilePatternPreview`: the first tile's suffix as the boxes
    // stand.
    let definition = editor.document.build_definition();
    let pad = if definition.tile_pad_digits { 2 } else { 1 };
    let buckets = definition.resolution / definition.bucket_resolution.max(1);
    let suffix = tile_suffix(0, 0, &definition.tile_pattern, !definition.tile_zero_index, pad, buckets, true);
    let preview = RichText::new(format!("FileName{suffix}.ext")).font(theme::monospace(11.0)).color(theme::MENU_SEPARATOR);
    ui.label(preview);

    ui.add_space(8.0);
    let mut pad_digits = definition.tile_pad_digits;
    if ui.checkbox(&mut pad_digits, "Add Leading Zeroes").changed() {
        editor.document.set_build_setting("TilePadDigits", Value::Bool(pad_digits));
    }
    let mut from_one = !definition.tile_zero_index;
    if ui.checkbox(&mut from_one, "Start numbering from 1").changed() {
        editor.document.set_build_setting("TileZeroIndex", Value::Bool(!from_one));
    }

    ui.add_space(16.0);
    caption(ui, "Organization", theme::LABEL_DEFAULT);
    let current = ORGANIZATIONS.iter().find(|(organize, _)| *organize == definition.organize_files).map(|(_, name)| *name).unwrap_or("");
    let mut chosen = None;
    let mut combo = egui::ComboBox::from_id_salt("build-tile-organization");
    combo = combo.selected_text(current);
    combo = combo.width(360.0);
    combo.show_ui(ui, |ui| {
        for (organize, name) in ORGANIZATIONS {
            if ui.selectable_label(organize == definition.organize_files, name).clicked() {
                chosen = Some(organize);
            }
        }
    });
    if let Some(organize) = chosen {
        editor.document.set_build_setting("OrganizeFiles", Value::from(organize.name()));
    }
    ui.separator();

    let mut flip = definition.flip_y;
    if ui.checkbox(&mut flip, "Flip Y axis").changed() {
        editor.document.set_build_setting("FlipY", Value::Bool(flip));
    }
    let mut preserve = definition.preserve_tiled_cache;
    if ui.checkbox(&mut preserve, "Preserve Cache in Build Folder").changed() {
        editor.document.set_build_setting("PreserveTiledCache", Value::Bool(preserve));
    }

    ui.add_space(16.0);
    caption(ui, "Overlap Pixels", theme::LABEL_DEFAULT);
    let mut chosen_overlap = None;
    let mut combo = egui::ComboBox::from_id_salt("build-tile-overlap");
    combo = combo.selected_text(definition.overlapping_pixels.to_string());
    combo = combo.width(360.0);
    combo.show_ui(ui, |ui| {
        for overlap in 0..=MOST_OVERLAP {
            if ui.selectable_label(overlap == definition.overlapping_pixels, overlap.to_string()).clicked() {
                chosen_overlap = Some(overlap);
            }
        }
    });
    if let Some(overlap) = chosen_overlap {
        editor.document.set_build_setting("OverlappingPixels", json!(overlap));
    }
}

fn build_tab(ui: &mut Ui, editor: &mut Editor, settings: &mut BuildSettingsState) {
    section(ui, "Export Settings");
    let definition = editor.document.build_definition();
    caption(ui, "Build Destination", theme::LABEL_DEFAULT);
    let typed = settings.destination.get_or_insert_with(|| definition.destination.clone());
    ui.horizontal(|ui| {
        let mut field = egui::TextEdit::singleline(typed);
        field = field.font(theme::semibold(12.0));
        field = field.text_color(theme::TEXTBOX_FILE_FOREGROUND);
        field = field.hint_text(nerthus_export::destination::DEFAULT_TEMPLATE);
        field = field.desired_width(ui.available_width() - 70.0);
        let response = ui.add(field);
        if response.changed() {
            editor.document.set_build_setting("Destination", Value::String(typed.clone()));
        }
        icon_menu(ui, "chevron-down", "Presets", None, |ui| {
            for preset in TEMPLATE_PRESETS {
                if menu_item(ui, None, None, preset, "").clicked() {
                    *typed = preset.to_string();
                    editor.document.set_build_setting("Destination", Value::String(typed.clone()));
                    ui.close();
                }
            }
        });
    });
    let preview = match &editor.document.path {
        Some(path) => {
            let preferences = editor.user.preferences.export_preferences();
            let definition = editor.document.build_definition();
            preview_destination(path, &definition, &preferences).display().to_string()
        }
        None => "Save the project to see where it builds.".to_string(),
    };
    let preview_text = RichText::new(preview).font(theme::monospace(11.0)).color(theme::MENU_SEPARATOR);
    ui.label(preview_text);

    ui.add_space(8.0);
    caption(ui, "ColorSpace", theme::LABEL_DEFAULT);
    let mut chosen_space = None;
    let current_space = definition.color_space.clone();
    let mut combo = egui::ComboBox::from_id_salt("build-color-space");
    combo = combo.selected_text(current_space.as_str());
    combo = combo.width(260.0);
    combo.show_ui(ui, |ui| {
        for name in COLOR_SPACES {
            if ui.selectable_label(name == current_space, name).clicked() {
                chosen_space = Some(name);
            }
        }
    });
    if let Some(name) = chosen_space {
        editor.document.set_build_setting("ColorSpace", Value::String(name.to_string()));
    }
    caption(ui, "Applies to 16-bit and 8-bit RGB outputs only.", theme::MENU_SEPARATOR);

    ui.add_space(16.0);
    let flags = [
        ("MaintainPrimeFolder", "Maintain a static folder with the latest copy", definition.maintain_prime_folder, true),
        ("OpenFolder", "Open folder after build finishes", definition.open_folder, true),
        ("CopyTerrain", "Copy the .terrain file to the build folder", definition.copy_terrain, true),
        ("RemovePrimaryPortName", "Remove Primary port name in Build output", definition.remove_primary_port_name, true),
    ];
    for (member, text, value, enabled) in flags {
        let mut checked = value;
        let response = ui.add_enabled(enabled, egui::Checkbox::new(&mut checked, text));
        if response.changed() {
            editor.document.set_build_setting(member, Value::Bool(checked));
        }
    }

    ui.add_space(16.0);
    caption(ui, "File overwrite mode", theme::LABEL_DEFAULT);
    let increment = definition.overwrite_mode == nerthus_export::build::OverwriteMode::Increment;
    let modes = [("Overwrite", true), ("Increment", true)];
    if let Some(choice) = options(ui, &modes, usize::from(increment)) {
        let name = if choice == 1 { "Increment" } else { "Overwrite" };
        editor.document.set_build_setting("OverwriteMode", Value::String(name.to_string()));
    }
    ui.add_space(8.0);
    let tokens = RichText::new(TOKEN_HELP).font(theme::monospace(11.0)).color(theme::MENU_SEPARATOR);
    ui.label(tokens);
}

/// `SavablesArea`: one row per marked node, by graph tab.
fn nodes_tab(ui: &mut Ui, editor: &mut Editor, settings: &mut BuildSettingsState, commands: &mut Vec<Command>) {
    section(ui, "Exportable Nodes");
    let marked: Vec<i32> = editor.document.terrain().nodes.values().filter(|node| ExportSettings::of(node).is_some()).map(|node| node.id).collect();
    if marked.is_empty() {
        caption(ui, "Mark nodes as Exportable (F3) to see them here.", theme::LABEL_DEFAULT);
        return;
    }
    ui.horizontal(|ui| {
        ui.menu_button("Change formats for all nodes", |ui| {
            for format in FileFormat::ALL {
                if ui.button(format.name()).clicked() {
                    editor.document.set_all_export_formats(format);
                    ui.close();
                }
            }
        });
        if editor.document.has_duplicate_export_names() {
            let warning = RichText::new("There are duplicate filenames! Click to fix them!").color(theme::ACCENT_RED);
            if ui.link(warning).clicked() {
                editor.document.fix_duplicate_export_names();
                settings.filenames.clear();
            }
        }
    });
    ui.add_space(4.0);
    for tab in editor.document.graph_tabs() {
        let in_tab: Vec<i32> = marked.iter().copied().filter(|id| editor.document.node(*id).is_some_and(|node| node_tab(node) == tab.index)).collect();
        let mut header = egui::CollapsingHeader::new(tab.name.as_str());
        header = header.id_salt(("export-tab", tab.index));
        header = header.default_open(true);
        header.show(ui, |ui| {
            for id in in_tab {
                savable(ui, editor, settings, id, commands);
            }
        });
    }
}

fn node_tab(node: &nerthus_engine::NodeData) -> usize {
    node.base.get("GraphIndex").and_then(Value::as_u64).unwrap_or(0) as usize
}

/// `Savable`: the node's toolbox colour, its ports menu, its file name
/// and its format.
fn savable(ui: &mut Ui, editor: &mut Editor, settings: &mut BuildSettingsState, id: i32, commands: &mut Vec<Command>) {
    let Some(node) = editor.document.node(id).cloned() else {
        return;
    };
    let Some(export) = ExportSettings::of(&node) else {
        return;
    };
    let catalogue = nerthus_engine::Catalogue::get();
    let category = catalogue.node(&node.type_name).and_then(|spec| spec.toolbox.clone()).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.set_min_height(30.0);
        let (bar, _) = ui.allocate_exact_size(Vec2::new(2.0, 26.0), egui::Sense::hover());
        ui.painter().rect_filled(bar, 0.0, theme::toolbox_color(&category));
        {
            icon_menu(ui, "menu-deep", "Ports", None, |ui| {
                if ui.button(format!("Node: {}", node.name)).clicked() {
                    commands.push(Command::SelectNode(id));
                    ui.close();
                }
                menu_separator(ui);
                let mut enabled = export.enabled;
                if ui.checkbox(&mut enabled, "Enabled").changed() {
                    editor.document.edit_export(id, "Export enabled", |settings| settings.enabled = enabled);
                }
                menu_separator(ui);
                for port in node.ports.iter().filter(|port| port.is_output()) {
                    let mut exporting = port.is_exporting;
                    let response = ui.add_enabled(export.enabled, egui::Checkbox::new(&mut exporting, port.name.as_str()));
                    if response.changed() {
                        editor.document.set_port_exporting(id, &port.name, exporting);
                    }
                }
                menu_separator(ui);
                let outputs: Vec<String> = node.ports.iter().filter(|port| port.is_output()).map(|port| port.name.clone()).collect();
                if ui.add_enabled(export.enabled, egui::Button::new("Primary Only")).clicked() {
                    for name in &outputs {
                        editor.document.set_port_exporting(id, name, name == "Out");
                    }
                }
                if ui.add_enabled(export.enabled, egui::Button::new("Mark all")).clicked() {
                    for name in &outputs {
                        editor.document.set_port_exporting(id, name, true);
                    }
                }
                if ui.add_enabled(export.enabled, egui::Button::new("Mark none")).clicked() {
                    for name in &outputs {
                        editor.document.set_port_exporting(id, name, false);
                    }
                }
            });
        }

        // The file name, saved on Enter or Tab (empty: the node's name);
        // Escape puts it back.
        let index = match settings.filenames.iter().position(|(node_id, _)| *node_id == id) {
            Some(index) => index,
            None => {
                settings.filenames.push((id, export.filename.clone()));
                settings.filenames.len() - 1
            }
        };
        let name_width = (ui.available_width() - 120.0).max(80.0);
        let mut field = egui::TextEdit::singleline(&mut settings.filenames[index].1);
        field = field.font(theme::semibold(12.0));
        field = field.desired_width(name_width);
        let response = ui.add_enabled(export.enabled, field);
        let committed = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Tab));
        let escaped = ui.input(|input| input.key_pressed(egui::Key::Escape));
        if committed {
            let typed = settings.filenames[index].1.trim().to_string();
            let filename = if typed.is_empty() { node.name.clone() } else { typed };
            settings.filenames[index].1 = filename.clone();
            editor.document.edit_export(id, "Export filename", |settings| settings.filename = filename);
        } else {
            // Escape, or the name changed elsewhere (undo, fixing
            // duplicates): show the saved name.
            let cancelled = response.lost_focus() && escaped;
            let stale = !response.has_focus() && settings.filenames[index].1 != export.filename;
            if cancelled || stale {
                settings.filenames[index].1 = export.filename.clone();
            }
        }

        let mut chosen = None;
        let mut combo = egui::ComboBox::from_id_salt(("export-format", id));
        combo = combo.selected_text(export.format.name());
        combo = combo.width(105.0);
        ui.add_enabled_ui(export.enabled, |ui| {
            combo.show_ui(ui, |ui| {
                for format in FileFormat::ALL {
                    let item = ui.selectable_label(format == export.format, format.name());
                    if item.on_hover_text(format.description()).clicked() {
                        chosen = Some(format);
                    }
                }
            });
        });
        if let Some(format) = chosen {
            editor.document.edit_export(id, "Export format", |settings| settings.format = format);
        }
    });
}

fn script_tab(ui: &mut Ui, editor: &mut Editor, commands: &mut Vec<Command>) {
    section(ui, "Post-Build Script");
    caption(ui, "Enter any command line instructions to execute after a successful build.", theme::LABEL_DEFAULT);
    caption(ui, "The working directory will be the current build path.", theme::LABEL_DEFAULT);
    ui.add_space(8.0);
    let mut script = editor.document.build_definition().post_build_script;
    let mut field = egui::TextEdit::multiline(&mut script);
    field = field.font(theme::monospace(14.0));
    field = field.text_color(theme::ACCENT_MUTED_GREEN);
    field = field.desired_rows(12);
    field = field.desired_width(f32::INFINITY);
    if ui.add(field).changed() {
        editor.document.set_build_setting("PostBuildScript", Value::String(script));
    }
    // "Learn more" (`Button-Link`, no padding): the documentation on
    // post-build scripts.
    ui.add_space(8.0);
    if link_button(ui, "Learn more", theme::interface(12.0), Vec2::ZERO).clicked() {
        commands.push(Command::Tools(ToolCommand::ShowDocs(crate::docs::POST_BUILD_SCRIPT.to_string())));
    }
}

fn terrain_tab(ui: &mut Ui, editor: &mut Editor) {
    section(ui, "Terrain Definition");
    let terrain = editor.document.terrain();
    let mut width = terrain.width;
    let mut height = terrain.height;
    let mut changed = false;
    ui.horizontal(|ui| {
        caption(ui, "Width", theme::LABEL_DEFAULT);
        let mut value = egui::DragValue::new(&mut width);
        value = value.range(100.0..=2_400_000.0);
        value = value.suffix(" m");
        changed |= ui.add(value).changed();
    });
    ui.horizontal(|ui| {
        caption(ui, "Height", theme::LABEL_DEFAULT);
        let mut value = egui::DragValue::new(&mut height);
        value = value.range(100.0..=10_000.0);
        value = value.suffix(" m");
        changed |= ui.add(value).changed();
    });
    if changed {
        editor.document.set_terrain_size(width.round(), height.round());
        editor.changed();
    }
    ui.add_space(12.0);
    caption(ui, "TIP:", theme::LABEL_EMPHASIS);
    caption(ui, "Think about your terrain size realistically. Most common mountains and hills tend to be quite smaller than what new users may anticipate. Precise size and height of your terrain will result in more realistic erosion and other simulations.", theme::LABEL_DEFAULT);
    ui.add_space(6.0);
    for line in ["Mt. Everest (Nepal): ~8800m tall", "Mt. Saint Helens (US): ~2500m tall", "Ben Nevis (Scotland): ~1300m tall", "Mt. Fuji (Japan): ~3700m tall"] {
        let text = RichText::new(line).font(theme::monospace(11.0)).color(theme::LABEL_DEFAULT);
        ui.label(text);
    }
    ui.add_space(6.0);
    caption(ui, "In practical terms, your meter/pixel ratio is more important in CG.", theme::LABEL_DEFAULT);
}
