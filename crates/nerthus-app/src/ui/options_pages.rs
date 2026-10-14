//! Options' pages other than Compute (`GeneralTab`, `PathsTab`,
//! `ToolboxTab`, `GraphTab`, `NodeTab`, `DataViewTab`, `BuildTab`,
//! `CacheTab`, `ViewportTab`, `NavigationTab`, `LazyTab`,
//! `ExperimentalTab`). The pages work on copies of the user's options and
//! preferences: Accept Changes keeps every page's settings,
//! Cancel drops them, and Reset this page puts the page's settings back to
//! their defaults (to keep or drop in turn).
//!
//! Left out: the licence server, the Houdini server timeout, the old
//! memory pool, and sending crash reports and usage statistics (Nerthus
//! sends nothing; the boxes show, disabled).

use egui::{Align, Color32, Layout, RichText, Stroke, Ui, Vec2};
use serde_json::{json, Value};

use crate::theme;
use crate::ui::file_dialogs::{DialogKind, DialogRequest, FileDialogs};
use crate::ui::options_window::OptionsTab;
use crate::user::{Preferences, UserOptions};

/// A setting a page changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Setting {
    /// A member of `User.options`.
    Option(&'static str),
    /// A member of `Preferences.options`, in its section (empty for the
    /// top level).
    Preference(&'static str, &'static str),
    /// `verbose.dat` (Enable Verbose Logging, for the next start).
    VerboseFile,
}

use Setting::{Option as Opt, Preference as Pref};

const GENERAL: [Setting; 7] = [
    Opt("Monitor"),
    Opt("MoveLazyCursor"),
    Opt("ShowAdvancedTools"),
    Pref("", "ShowOrphanVariableWarning"),
    Setting::VerboseFile,
    Pref("Locations", "UseProxy"),
    Pref("Locations", "Proxy"),
];
const PATHS: [Setting; 3] = [Opt("ThumbnailStorage"), Pref("Locations", "Documents"), Pref("Locations", "Projects")];
const TOOLBOX: [Setting; 12] = [
    Opt("ToolboxStyle"),
    Opt("ToolboxCompact"),
    Opt("ToolboxLeftAlignLabels"),
    Opt("ToolboxMode"),
    Opt("ToolboxMenus"),
    Opt("ShowIconsInToolbox"),
    Opt("ToolboxHighlightHighUse"),
    Opt("ToolboxPinHighUse"),
    Opt("ShowFamilyName"),
    Opt("ShowIconsInSearchResults"),
    Opt("MaximumSearchResults"),
    Opt("StackedSatelliteMaps"),
];
const GRAPH: [Setting; 5] = [Opt("GridSize"), Opt("ConfineShortcuts"), Opt("HideTabs"), Opt("HideBookmarkShortcuts"), Opt("RecordGraphMovementUndos")];
const NODE: [Setting; 9] = [
    Opt("ClassicNodeStyle"),
    Opt("BuildShine"),
    Opt("ShowBuildTime"),
    Opt("PreciseValue"),
    Opt("AcceptOnLostFocus"),
    Opt("SliderSeed"),
    Opt("SnapToPorts"),
    Opt("DoubleClickConnect"),
    Opt("AlwaysShowMaskPort"),
];
const DATA_VIEW: [Setting; 2] = [Opt("DataViewIcons"), Opt("ExpandDataView")];
const BUILD: [Setting; 6] = [
    Pref("Locations", "Builds"),
    Pref("Defaults", "HeightfieldFormat"),
    Pref("Defaults", "ColorFormat"),
    Pref("Defaults", "MaskFormat"),
    Pref("Defaults", "RemovePrimaryPortName"),
    Pref("Cache", "OffloadBuildCache"),
];
const CACHE: [Setting; 7] = [
    Opt("ClearOldFiles"),
    Pref("Cache", "AlwaysSaveCache"),
    Pref("Cache", "SmartCache"),
    Pref("Cache", "NodeIdleThreshold"),
    Pref("Locations", "Cache"),
    Pref("Locations", "CentralizeCacheFreeze"),
    Pref("Locations", "CentralizeCacheBake"),
];
const VIEWPORT: [Setting; 9] = [
    Opt("ViewportQuality"),
    Opt("DontClearViewport"),
    Opt("ShowGrid"),
    Opt("ShowCompass"),
    Opt("ShowPlace"),
    Opt("PreventHiRes"),
    Opt("ViewportBackground"),
    Opt("MaskGradient"),
    Opt("GizmoType"),
];
const NAVIGATION: [Setting; 13] = [
    Opt("RotateButton"),
    Opt("PanButton"),
    Opt("ZoomButton"),
    Opt("AltRotate"),
    Opt("AltPan"),
    Opt("AltZoom"),
    Opt("RotateSensitivity"),
    Opt("PanSensitivity"),
    Opt("ZoomSensitivity"),
    Opt("InvertY"),
    Opt("FlipQE"),
    Opt("InvertZoom"),
    Opt("ShowNavigationHint"),
];
/// `LazyTab`'s rows: the member, its icon and its label.
const LAZY_ROWS: [(&str, &str, &str); 9] = [
    ("LazyViewport", "viewport2D", "2D Viewport"),
    ("LazyLighting", "sun", "Lighting Options"),
    ("LazyGraph", "graph", "Graph"),
    ("LazyProperties", "properties", "Properties"),
    ("LazyModifiers", "modifiers", "Modifiers"),
    ("LazyTree", "dataview-tree", "Node Tree"),
    ("LazyVariables", "variable", "Variables View"),
    ("LazyBuild", "build-settings", "Build View"),
    ("LazySettings", "settings", "Build Settings"),
];
const LAZY: [Setting; 9] = [
    Opt("LazyViewport"),
    Opt("LazyLighting"),
    Opt("LazyGraph"),
    Opt("LazyProperties"),
    Opt("LazyModifiers"),
    Opt("LazyTree"),
    Opt("LazyVariables"),
    Opt("LazyBuild"),
    Opt("LazySettings"),
];
const EXPERIMENTAL: [Setting; 7] = [
    Opt("CompactUI"),
    Opt("DisableAnimations"),
    Opt("NewAutolayout"),
    Opt("AdaptiveDebounce"),
    Opt("PopulateMRUOnStart"),
    Pref("Locations", "Custom"),
    Pref("Locations", "ShowCustomOnly"),
];

/// The settings each page changes.
fn page_settings(tab: OptionsTab) -> &'static [Setting] {
    match tab {
        OptionsTab::General => &GENERAL,
        OptionsTab::Paths => &PATHS,
        OptionsTab::Toolbox => &TOOLBOX,
        OptionsTab::Graph => &GRAPH,
        OptionsTab::Node => &NODE,
        OptionsTab::DataView => &DATA_VIEW,
        OptionsTab::Build => &BUILD,
        OptionsTab::Cache => &CACHE,
        OptionsTab::Viewport => &VIEWPORT,
        OptionsTab::Navigation => &NAVIGATION,
        OptionsTab::LazyMenu => &LAZY,
        OptionsTab::Experimental => &EXPERIMENTAL,
        OptionsTab::Compute => &[],
    }
}

const ALL_PAGES: [OptionsTab; 12] = [
    OptionsTab::General,
    OptionsTab::Paths,
    OptionsTab::Toolbox,
    OptionsTab::Graph,
    OptionsTab::Node,
    OptionsTab::DataView,
    OptionsTab::Build,
    OptionsTab::Cache,
    OptionsTab::Viewport,
    OptionsTab::Navigation,
    OptionsTab::LazyMenu,
    OptionsTab::Experimental,
];

/// The pages' copies of the settings (`LoadSettings`).
pub struct OptionsCopy {
    options: UserOptions,
    preferences: Preferences,
    verbose_at_start: bool,
    /// The monitors there are, the primary's index (`Screen.AllScreens`).
    monitors: (usize, Option<usize>),
}

impl OptionsCopy {
    pub fn load(options: &UserOptions, preferences: &Preferences) -> OptionsCopy {
        OptionsCopy {
            options: options.clone(),
            preferences: preferences.clone(),
            verbose_at_start: crate::logging::verbose_at_start(),
            monitors: monitor_count(),
        }
    }

    fn get(&self, setting: Setting) -> Value {
        match setting {
            Setting::Option(name) => self.options.value(name),
            Setting::Preference(section, name) => self.preferences.member(section, name),
            Setting::VerboseFile => Value::Bool(self.verbose_at_start),
        }
    }

    fn set(&mut self, setting: Setting, value: Value) {
        match setting {
            Setting::Option(name) => self.options.set_value(name, value),
            Setting::Preference(section, name) => self.preferences.set_member(section, name, value),
            Setting::VerboseFile => self.verbose_at_start = value.as_bool().unwrap_or(false),
        }
    }

    fn flag(&self, setting: Setting) -> bool {
        self.get(setting).as_bool().unwrap_or(false)
    }

    fn number(&self, setting: Setting) -> f64 {
        self.get(setting).as_f64().unwrap_or(0.0)
    }

    fn text(&self, setting: Setting) -> String {
        self.get(setting).as_str().unwrap_or_default().to_string()
    }

    /// `ResetDefaults`: the page's settings back to a fresh `Options` and
    /// `Preferences`.
    pub fn reset(&mut self, tab: OptionsTab) {
        let fresh_options = UserOptions::new();
        let fresh_preferences = Preferences::new();
        for setting in page_settings(tab) {
            let value = match *setting {
                Setting::Option(name) => fresh_options.value(name),
                Setting::Preference(section, name) => fresh_preferences.member(section, name),
                Setting::VerboseFile => Value::Bool(false),
            };
            self.set(*setting, value);
        }
    }

    /// `SaveSettings` for every page: each page's settings onto the
    /// user's.
    pub fn save(&self, options: &mut UserOptions, preferences: &mut Preferences) {
        for tab in ALL_PAGES {
            for setting in page_settings(tab) {
                let value = self.get(*setting);
                match *setting {
                    Setting::Option(name) => options.set_value(name, value),
                    Setting::Preference(section, name) => preferences.set_member(section, name, value),
                    Setting::VerboseFile => {
                        if let Err(error) = crate::logging::set_verbose_at_start(self.verbose_at_start) {
                            log::error!("Cannot set Verbose Logging for the next start: {error}");
                        }
                    }
                }
            }
        }
    }
}

/// The monitors Bevy found, kept as they change.
static MONITORS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static PRIMARY_MONITOR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);

fn monitor_count() -> (usize, Option<usize>) {
    let count = MONITORS.load(std::sync::atomic::Ordering::Relaxed);
    let primary = PRIMARY_MONITOR.load(std::sync::atomic::Ordering::Relaxed);
    (count, (primary != usize::MAX).then_some(primary))
}

/// Counts the monitors for General's "Start Nerthus on", numbered in the
/// order the system lists them (the window opens on one by that number).
pub fn count_monitors(
    monitors: bevy::prelude::Query<(bevy::prelude::Entity, Option<&bevy::window::PrimaryMonitor>), bevy::prelude::With<bevy::window::Monitor>>,
    listed: Option<bevy::prelude::Res<bevy::winit::WinitMonitors>>,
) {
    let count = monitors.iter().count();
    let mut primary = usize::MAX;
    let primary_entity = monitors.iter().find(|(_, is_primary)| is_primary.is_some()).map(|(entity, _)| entity);
    if let (Some(listed), Some(entity)) = (listed, primary_entity) {
        let handle = listed.find_entity(entity);
        let position = (0..count).position(|index| listed.nth(index) == handle);
        primary = position.unwrap_or(usize::MAX);
    }
    MONITORS.store(count, std::sync::atomic::Ordering::Relaxed);
    PRIMARY_MONITOR.store(primary, std::sync::atomic::Ordering::Relaxed);
}

/// What a page asks of the window around it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageRequest {
    /// General's Run Diagnostics and Factory Reset.
    RunDiagnostics,
}

/// Draws the page.
pub fn page(ui: &mut Ui, tab: OptionsTab, copy: &mut OptionsCopy, dialogs: &mut FileDialogs) -> Option<PageRequest> {
    take_folder_answers(copy, dialogs);
    let mut request = None;
    let mut scroll = egui::ScrollArea::vertical();
    scroll = scroll.id_salt(("options-page", tab_key(tab)));
    scroll = scroll.auto_shrink([false, false]);
    scroll.show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        match tab {
            OptionsTab::General => request = general(ui, copy),
            OptionsTab::Paths => paths(ui, copy, dialogs),
            OptionsTab::Toolbox => toolbox(ui, copy),
            OptionsTab::Graph => graph(ui, copy),
            OptionsTab::Node => node(ui, copy),
            OptionsTab::DataView => data_view(ui, copy),
            OptionsTab::Build => build(ui, copy, dialogs),
            OptionsTab::Cache => cache(ui, copy, dialogs),
            OptionsTab::Viewport => viewport(ui, copy),
            OptionsTab::Navigation => navigation(ui, copy),
            OptionsTab::LazyMenu => lazy(ui, copy),
            OptionsTab::Experimental => experimental(ui, copy),
            OptionsTab::Compute => {}
        }
    });
    request
}

fn tab_key(tab: OptionsTab) -> u8 {
    ALL_PAGES.iter().position(|candidate| *candidate == tab).unwrap_or(99) as u8
}

/// A section's title bar (`TextBlock` on `Panel-Border`, semibold).
fn section(ui: &mut Ui, title: &str, gap_after: f32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 24.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::PANEL_BORDER);
    let galley = ui.painter().layout_no_wrap(title.to_string(), theme::semibold(12.0), theme::LABEL_EMPHASIS);
    let position = egui::Pos2::new(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(position, galley, theme::LABEL_EMPHASIS);
    ui.add_space(gap_after);
}

/// A right-aligned label in a column of `width` (the pages' 96-wide
/// labels).
fn row_label(ui: &mut Ui, text: &str, width: f32) {
    ui.allocate_ui_with_layout(Vec2::new(width, 24.0), Layout::right_to_left(Align::Center), |ui| {
        ui.label(RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT));
    });
}

/// `ListBox-OptionsX`: one segment per choice (`OptionsX.Item.Padding`
/// round its text, `OptionsX.Item.Margin` apart); the one clicked.
fn options_x(ui: &mut Ui, choices: &[&str], current: Option<usize>, width: Option<f32>) -> Option<usize> {
    let mut chosen = None;
    let sizes = theme::sizes(ui.ctx());
    let gap = sizes.option_margin;
    let height = 16.0 + 2.0 * sizes.option_padding[1];
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
        ui.spacing_mut().button_padding.x = sizes.option_padding[0];
        let segment_width = width.map(|width| (width - gap * (choices.len() as f32 - 1.0)) / choices.len() as f32);
        for (index, name) in choices.iter().enumerate() {
            let selected = current == Some(index);
            let text_color = if selected { theme::LABEL_EMPHASIS } else { theme::LABEL_DEFAULT };
            let text = RichText::new(*name).font(theme::interface(11.0)).color(text_color);
            let mut button = egui::Button::new(text);
            button = button.fill(if selected { theme::OPTIONS_SELECTION } else { theme::BUTTON_BACKGROUND });
            button = button.stroke(Stroke::new(1.0, theme::BUTTON_BORDER));
            button = button.min_size(Vec2::new(segment_width.unwrap_or(0.0), height));
            if ui.add(button).clicked() && !selected {
                chosen = Some(index);
            }
        }
    });
    chosen
}

/// A labelled choice (`TextBlock` + `ListBox-OptionsX`).
fn choice(ui: &mut Ui, label: &str, label_width: f32, choices: &[&str], current: Option<usize>, width: Option<f32>) -> Option<usize> {
    let mut chosen = None;
    ui.horizontal(|ui| {
        row_label(ui, label, label_width);
        ui.add_space(8.0);
        chosen = options_x(ui, choices, current, width);
    });
    chosen
}

/// A check box `indent` from the left, bound to a flag.
fn check(ui: &mut Ui, copy: &mut OptionsCopy, setting: Setting, indent: f32, text: &str) {
    let mut checked = copy.flag(setting);
    ui.horizontal(|ui| {
        ui.add_space(indent);
        if ui.checkbox(&mut checked, RichText::new(text).font(theme::interface(12.0))).changed() {
            copy.set(setting, Value::Bool(checked));
        }
    });
}

/// A check box that shows but cannot change.
fn fixed_check(ui: &mut Ui, indent: f32, text: &str, checked: bool, why: &str) {
    let mut shown = checked;
    ui.horizontal(|ui| {
        ui.add_space(indent);
        ui.add_enabled(false, egui::Checkbox::new(&mut shown, RichText::new(text).font(theme::interface(12.0)))).on_disabled_hover_text(why);
    });
}

/// A dimmed note under a setting.
fn note(ui: &mut Ui, indent: f32, text: &str, color: Color32) {
    ui.horizontal(|ui| {
        ui.add_space(indent);
        let label = RichText::new(text).font(theme::interface(11.0)).color(color.gamma_multiply(0.75));
        ui.add(egui::Label::new(label).wrap());
    });
}

/// `FileBrowserControl`: a header, the folder's path and a button that
/// picks another.
fn folder_box(ui: &mut Ui, copy: &mut OptionsCopy, dialogs: &mut FileDialogs, setting: Setting, header: &str) {
    let key = folder_key(setting);
    let mut text = copy.text(setting);
    ui.horizontal(|ui| {
        row_label(ui, header, 92.0);
        ui.add_space(8.0);
        let mut field = egui::TextEdit::singleline(&mut text);
        field = field.font(theme::semibold(12.0));
        field = field.text_color(theme::TEXTBOX_FILE_FOREGROUND);
        field = field.desired_width(ui.available_width() - 40.0);
        if ui.add(field).changed() {
            copy.set(setting, Value::String(text.clone()));
        }
        if ui.add_enabled(!dialogs.pending(&key), egui::Button::new("…")).clicked() {
            let current = std::path::PathBuf::from(&text);
            let request = DialogRequest { key, title: header.to_string(), kind: DialogKind::PickFolder, directory: Some(current) };
            dialogs.ask(request);
        }
    });
}

/// The dialog key a folder box asks under.
fn folder_key(setting: Setting) -> String {
    match setting {
        Setting::Preference(section, name) => format!("options-folder-{section}-{name}"),
        Setting::Option(name) => format!("options-folder-{name}"),
        Setting::VerboseFile => "options-folder-verbose".to_string(),
    }
}

/// Folders picked since the last frame.
fn take_folder_answers(copy: &mut OptionsCopy, dialogs: &mut FileDialogs) {
    let folders = [
        Pref("Locations", "Documents"),
        Pref("Locations", "Projects"),
        Pref("Locations", "Builds"),
        Pref("Locations", "Cache"),
    ];
    for setting in folders {
        if let Some(Some(folder)) = dialogs.take(&folder_key(setting)) {
            copy.set(setting, Value::String(folder.to_string_lossy().into_owned()));
        }
    }
}

/// An enum member stored by name, as one of `names`.
fn named_choice(ui: &mut Ui, copy: &mut OptionsCopy, setting: Setting, label: &str, choices: &[&str], names: &[&str], width: Option<f32>) {
    let current = copy.text(setting);
    let index = names.iter().position(|name| *name == current);
    if let Some(chosen) = choice(ui, label, 96.0, choices, index, width) {
        copy.set(setting, Value::String(names[chosen].to_string()));
    }
}

/// An integer member stored as the choice's index.
fn indexed_choice(ui: &mut Ui, copy: &mut OptionsCopy, setting: Setting, label: &str, choices: &[&str], width: Option<f32>) {
    let current = copy.number(setting);
    let index = (current >= 0.0 && (current as usize) < choices.len()).then_some(current as usize);
    if let Some(chosen) = choice(ui, label, 96.0, choices, index, width) {
        copy.set(setting, json!(chosen));
    }
}

/// An integer member stored as one of `values`.
fn valued_choice(ui: &mut Ui, copy: &mut OptionsCopy, setting: Setting, label: &str, values: &[i64], width: Option<f32>) {
    let current = copy.number(setting) as i64;
    let names: Vec<String> = values.iter().map(i64::to_string).collect();
    let choices: Vec<&str> = names.iter().map(String::as_str).collect();
    let index = values.iter().position(|value| *value == current);
    if let Some(chosen) = choice(ui, label, 96.0, &choices, index, width) {
        copy.set(setting, json!(values[chosen]));
    }
}

/// `GeneralTab`.
fn general(ui: &mut Ui, copy: &mut OptionsCopy) -> Option<PageRequest> {
    let mut request = None;
    ui.add_space(8.0);
    section(ui, "General", 16.0);
    let (count, primary) = copy.monitors;
    let names: Vec<String> = (0..count.max(1)).map(|index| if Some(index) == primary { format!("Monitor {index} (Primary)") } else { format!("Monitor {index}") }).collect();
    let choices: Vec<&str> = names.iter().map(String::as_str).collect();
    let current = copy.number(Opt("Monitor"));
    let index = (current >= 0.0).then_some(current as usize);
    if let Some(chosen) = choice(ui, "Start Nerthus on", 96.0, &choices, index, None) {
        copy.set(Opt("Monitor"), json!(chosen));
    }
    ui.add_space(12.0);
    check(ui, copy, Opt("MoveLazyCursor"), 108.0, "Move cursor to target when using the Lazy Menu");
    ui.add_space(12.0);
    check(ui, copy, Opt("ShowAdvancedTools"), 108.0, "Show Expert Tools");
    ui.add_space(12.0);
    check(ui, copy, Pref("", "ShowOrphanVariableWarning"), 108.0, "Show warning for orphan variables");
    ui.separator();
    ui.add_space(8.0);
    fixed_check(ui, 108.0, "Check for updates on startup", true, "Nerthus always checks (Help › Check for Updates).");
    ui.add_space(8.0);
    check(ui, copy, Setting::VerboseFile, 108.0, "Enable Verbose Logging (Restart Nerthus to take effect)");
    note(ui, 144.0, "Use sparingly. It will create larger logs.", theme::LABEL_DEFAULT);
    ui.add_space(8.0);
    fixed_check(ui, 108.0, "Report crash reports anonymously", false, "Nerthus sends no reports.");
    ui.add_space(8.0);
    fixed_check(ui, 108.0, "Report anonymous usage statistics", false, "Nerthus sends no statistics.");
    ui.separator();
    ui.add_space(8.0);
    let mut proxy = copy.text(Pref("Locations", "Proxy"));
    let mut use_proxy = copy.flag(Pref("Locations", "UseProxy"));
    ui.horizontal(|ui| {
        ui.add_space(20.0);
        if ui.checkbox(&mut use_proxy, RichText::new("Use Web Proxy").font(theme::interface(12.0))).changed() {
            copy.set(Pref("Locations", "UseProxy"), Value::Bool(use_proxy));
        }
        ui.add_space(8.0);
        let mut field = egui::TextEdit::singleline(&mut proxy);
        field = field.desired_width(ui.available_width() - 8.0);
        if ui.add_enabled(use_proxy, field).changed() {
            copy.set(Pref("Locations", "Proxy"), Value::String(proxy.clone()));
        }
    });
    ui.separator();
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button("Open Data Folder").clicked()
            && let Some(folder) = crate::user::data_folder()
        {
            let _ = std::fs::create_dir_all(&folder);
            crate::export_build::open_folder(&folder);
        }
        ui.add_space(18.0);
        ui.separator();
        ui.add_space(18.0);
        if ui.button("Run Diagnostics").clicked() {
            request = Some(PageRequest::RunDiagnostics);
        }
        if ui.button("Factory Reset").clicked() {
            request = Some(PageRequest::RunDiagnostics);
        }
    });
    request
}

/// `PathsTab`.
fn paths(ui: &mut Ui, copy: &mut OptionsCopy, dialogs: &mut FileDialogs) {
    ui.add_space(8.0);
    section(ui, "Paths", 16.0);
    named_choice(ui, copy, Opt("ThumbnailStorage"), "File Thumbnails", &["Don't Save", "Central Data Folder", "Next to File"], &["None", "Central", "Local"], None);
    ui.add_space(12.0);
    folder_box(ui, copy, dialogs, Pref("Locations", "Documents"), "User Files");
    note(ui, 100.0, "This is where all Presets, Screenshots, and other Nerthus documents are stored.", theme::LABEL_DEFAULT);
    ui.add_space(16.0);
    folder_box(ui, copy, dialogs, Pref("Locations", "Projects"), "Project Files");
    note(ui, 100.0, "This is where all Nerthus project files are stored by default.", theme::LABEL_DEFAULT);
    ui.separator();
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(24.0);
        ui.label(RichText::new("TIPS").font(theme::semibold(12.0)).color(theme::LABEL_EMPHASIS));
    });
    note(ui, 24.0, "It is critical that any of these folders are not in a subfolder where Nerthus is installed.", theme::LABEL_EMPHASIS);
    note(ui, 24.0, "Projects are usually small, lightweight files. However, Builds are very large and heavy. Keeping them separate is recommended if you frequently backup either folder.", theme::LABEL_EMPHASIS);
}

/// `ToolboxTab`: Menus show their layout and organization, the Toolbox
/// its layout and labels.
fn toolbox(ui: &mut Ui, copy: &mut OptionsCopy) {
    ui.add_space(8.0);
    section(ui, "Toolbox", 16.0);
    named_choice(ui, copy, Opt("ToolboxStyle"), "Configuration", &["Menus", "Toolbox"], &["Menus", "Panel"], Some(220.0));
    ui.add_space(12.0);
    if copy.text(Opt("ToolboxStyle")) == "Panel" {
        let compact = copy.flag(Opt("ToolboxCompact"));
        if let Some(chosen) = choice(ui, "Layout", 96.0, &["Compact", "Expanded"], Some(usize::from(!compact)), Some(220.0)) {
            copy.set(Opt("ToolboxCompact"), Value::Bool(chosen == 0));
        }
        ui.add_space(8.0);
        check(ui, copy, Opt("ToolboxLeftAlignLabels"), 112.0, "Left-align labels");
    } else {
        named_choice(ui, copy, Opt("ToolboxMode"), "Layout", &["Compact", "Buttons", "Large"], &["Single", "Double", "Expanded"], Some(220.0));
        ui.add_space(8.0);
        named_choice(ui, copy, Opt("ToolboxMenus"), "Organization", &["Nested by Family", "Show All"], &["Nested", "Flat"], Some(220.0));
        ui.add_space(8.0);
        check(ui, copy, Opt("ShowIconsInToolbox"), 112.0, "Show icons in Toolbox");
    }
    ui.add_space(8.0);
    check(ui, copy, Opt("ToolboxHighlightHighUse"), 112.0, "Highlight most-used nodes");
    ui.add_space(4.0);
    check(ui, copy, Opt("ToolboxPinHighUse"), 112.0, "Pin most-used nodes");
    ui.add_space(8.0);
    section(ui, "Search", 8.0);
    // "Search Results" (margin 26, 6, 0, -25) shares the first check's row.
    let row_top = ui.cursor().top();
    let label_position = egui::pos2(ui.min_rect().left() + 26.0, row_top + 3.0);
    let label_galley = ui.painter().layout_no_wrap("Search Results".to_string(), theme::interface(12.0), theme::LABEL_DEFAULT);
    ui.painter().galley(label_position, label_galley, theme::LABEL_DEFAULT);
    check(ui, copy, Opt("ShowFamilyName"), 112.0, "Show Family Name");
    check(ui, copy, Opt("ShowIconsInSearchResults"), 112.0, "Show Icons");
    ui.add_space(8.0);
    valued_choice(ui, copy, Opt("MaximumSearchResults"), "Maximum Results", &[5, 10, 15], Some(220.0));
    ui.separator();
    check(ui, copy, Opt("StackedSatelliteMaps"), 112.0, "Show Satellite Maps List as a Single Stack");
}

/// `GraphTab`.
fn graph(ui: &mut Ui, copy: &mut OptionsCopy) {
    ui.add_space(8.0);
    section(ui, "Graph", 16.0);
    valued_choice(ui, copy, Opt("GridSize"), "Grid Snap Size", &[10, 20, 25, 50, 100], Some(220.0));
    ui.separator();
    ui.add_space(8.0);
    check(ui, copy, Opt("ConfineShortcuts"), 108.0, "Allow node shortcuts only when cursor is inside the graph");
    ui.add_space(12.0);
    check(ui, copy, Opt("HideTabs"), 108.0, "Hide Graph Tabs");
    ui.add_space(12.0);
    check(ui, copy, Opt("HideBookmarkShortcuts"), 108.0, "Hide Bookmark Shortcuts");
    ui.add_space(12.0);
    check(ui, copy, Opt("RecordGraphMovementUndos"), 108.0, "Record Graph Movements and Zoom in Undos");
}

/// `NodeTab`.
fn node(ui: &mut Ui, copy: &mut OptionsCopy) {
    ui.add_space(8.0);
    section(ui, "Node Properties", 16.0);
    let classic = copy.flag(Opt("ClassicNodeStyle"));
    if let Some(chosen) = choice(ui, "Node Style", 96.0, &["Muted", "Vibrant"], Some(usize::from(classic)), None) {
        copy.set(Opt("ClassicNodeStyle"), Value::Bool(chosen == 1));
    }
    ui.separator();
    ui.add_space(8.0);
    let flags = [
        ("BuildShine", "Show shine effect on Node when build completes"),
        ("ShowBuildTime", "Show build time on node"),
        ("PreciseValue", "Show values with high precision"),
        ("AcceptOnLostFocus", "Accept values when clicking outside of editable field"),
        ("SliderSeed", "Use slider for Seed instead of new Seed control"),
    ];
    for (member, text) in flags {
        check(ui, copy, Opt(member), 100.0, text);
        ui.add_space(8.0);
    }
    ui.separator();
    ui.add_space(8.0);
    check(ui, copy, Opt("SnapToPorts"), 100.0, "Snap connections to available ports");
    ui.add_space(8.0);
    check(ui, copy, Opt("DoubleClickConnect"), 100.0, "Double-click on port to connect");
    ui.add_space(8.0);
    check(ui, copy, Opt("AlwaysShowMaskPort"), 100.0, "Always Show Mask Ports");
    note(ui, 128.0, "You can drop a connection on the middle of a node, and select \"Add as Mask\" to add a Mask Port, instead of enabling this legacy option.\n\nThis is the new and recommended way of working. (Existing nodes and files will not be affected)", theme::LABEL_DEFAULT);
}

/// `DataViewTab`.
fn data_view(ui: &mut Ui, copy: &mut OptionsCopy) {
    ui.add_space(8.0);
    section(ui, "Data View", 16.0);
    named_choice(ui, copy, Opt("DataViewIcons"), "Icons", &["Colorful", "Muted", "Hide"], &["Color", "Muted", "Hide"], None);
    ui.add_space(12.0);
    check(ui, copy, Opt("ExpandDataView"), 112.0, "Always expand");
}

/// `BuildTab`'s format boxes (`LoadFileFormats`: every `FileFormats`
/// member).
fn format_choice(ui: &mut Ui, copy: &mut OptionsCopy, member: &'static str, header: &str) {
    let setting = Pref("Defaults", member);
    let current = copy.text(setting);
    ui.horizontal(|ui| {
        row_label(ui, header, 96.0);
        ui.add_space(8.0);
        let mut combo = egui::ComboBox::from_id_salt(("default-format", member));
        combo = combo.selected_text(current.as_str());
        combo = combo.width(220.0);
        combo.show_ui(ui, |ui| {
            for format in nerthus_export::FileFormat::ALL {
                let name = format.name();
                if ui.selectable_label(name == current, name).clicked() {
                    copy.set(setting, Value::String(name.to_string()));
                }
            }
        });
    });
}

/// `BuildTab`.
fn build(ui: &mut Ui, copy: &mut OptionsCopy, dialogs: &mut FileDialogs) {
    ui.add_space(8.0);
    section(ui, "Default File Formats", 16.0);
    folder_box(ui, copy, dialogs, Pref("Locations", "Builds"), "Build Output");
    note(ui, 100.0, "This is where output files from builds are stored. It is denoted by the <Builds> token in Build Settings > Destination.", theme::LABEL_DEFAULT);
    ui.separator();
    ui.add_space(8.0);
    format_choice(ui, copy, "HeightfieldFormat", "Heightfield");
    ui.add_space(8.0);
    format_choice(ui, copy, "ColorFormat", "Color Maps");
    ui.add_space(8.0);
    format_choice(ui, copy, "MaskFormat", "Masks");
    ui.add_space(12.0);
    check(ui, copy, Pref("Defaults", "RemovePrimaryPortName"), 36.0, "Remove Primary port name in Build output");
    ui.add_space(8.0);
    check(ui, copy, Pref("Cache", "OffloadBuildCache"), 36.0, "Purge unnecessary cache during build to conserve memory.");
}

/// `CacheTab`.
fn cache(ui: &mut Ui, copy: &mut OptionsCopy, dialogs: &mut FileDialogs) {
    ui.add_space(8.0);
    section(ui, "Logs and Autosaves", 16.0);
    check(ui, copy, Opt("ClearOldFiles"), 108.0, "Delete after 45 days");
    ui.add_space(8.0);
    section(ui, "Smart Cache", 16.0);
    check(ui, copy, Pref("Cache", "AlwaysSaveCache"), 108.0, "Always Cache to Disk on Close");
    ui.add_space(8.0);
    check(ui, copy, Pref("Cache", "SmartCache"), 108.0, "Use Smart Cache");
    note(ui, 144.0, "2K or 4K cache is offloaded from RAM to disk if unaccessed for the Idle Threshold. Cache is instantly loaded back to memory when required.", theme::LABEL_DEFAULT);
    note(ui, 144.0, "This is recommended when using SSD storage.", theme::ACCENT_STRONG_ORANGE);
    ui.add_space(8.0);
    let setting = Pref("Cache", "NodeIdleThreshold");
    let mut seconds = copy.number(setting) as i64;
    ui.horizontal(|ui| {
        row_label(ui, "Idle Threshold", 96.0);
        ui.add_space(8.0);
        let drag = egui::DragValue::new(&mut seconds).range(100..=60000).suffix(" seconds");
        if ui.add(drag).changed() {
            copy.set(setting, json!(seconds));
        }
    });
    ui.add_space(8.0);
    section(ui, "Cache Locations", 16.0);
    folder_box(ui, copy, dialogs, Pref("Locations", "Cache"), "Cache Output");
    note(ui, 108.0, "Recommend using SSD for Cache location.", theme::LABEL_DEFAULT);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(108.0);
        ui.label(RichText::new("Save in Central Cache Folder").font(theme::interface(12.0)).color(theme::LABEL_DEFAULT));
    });
    note(ui, 108.0, "Will save next to file in \\Cache subfolder when disabled.", theme::LABEL_DEFAULT);
    check(ui, copy, Pref("Locations", "CentralizeCacheFreeze"), 108.0, "Smart Cache and Cache to Disk");
    check(ui, copy, Pref("Locations", "CentralizeCacheBake"), 108.0, "Baked Cache");
}

/// `ViewportTab`.
fn viewport(ui: &mut Ui, copy: &mut OptionsCopy) {
    ui.add_space(8.0);
    section(ui, "Viewport", 16.0);
    indexed_choice(ui, copy, Opt("ViewportQuality"), "Antialiasing", &["None", "2x", "4x", "8x"], None);
    ui.add_space(12.0);
    let flags = [
        ("DontClearViewport", "Do not clear viewport when unselecting a node"),
        ("ShowGrid", "Show Grid"),
        ("ShowCompass", "Show Compass"),
        ("ShowPlace", "Show Base Marker"),
        ("PreventHiRes", "Conserve memory when displaying 4K terrains"),
    ];
    for (member, text) in flags {
        check(ui, copy, Opt(member), 108.0, text);
        ui.add_space(8.0);
    }
    let background = copy.get(Opt("ViewportBackground"));
    let component = |name: &str| background.get(name).and_then(Value::as_f64).unwrap_or(0.0).clamp(0.0, 1.0);
    let mut colour = [(component("R") * 255.0).round() as u8, (component("G") * 255.0).round() as u8, (component("B") * 255.0).round() as u8];
    ui.horizontal(|ui| {
        row_label(ui, "Background", 96.0);
        ui.add_space(8.0);
        if egui::color_picker::color_edit_button_srgb(ui, &mut colour).changed() {
            let value = json!({"R": colour[0] as f64 / 255.0, "G": colour[1] as f64 / 255.0, "B": colour[2] as f64 / 255.0});
            copy.set(Opt("ViewportBackground"), value);
        }
        // `btnResetViewportColor`: back to #050505.
        if ui.button("x").on_hover_text("Reset").clicked() {
            let value = UserOptions::new().value("ViewportBackground");
            copy.set(Opt("ViewportBackground"), value);
        }
    });
    ui.add_space(8.0);
    indexed_choice(ui, copy, Opt("MaskGradient"), "Mask Gradient", &["Greyscale", "Hot", "Red", "Blue", "Spectrum"], None);
    ui.add_space(8.0);
    indexed_choice(ui, copy, Opt("GizmoType"), "Gizmos", &["2D Viewport", "3D + 2D Viewport"], None);
}

/// `GetIndexByButton`: the list shows Left, Middle, Right; Unity numbers
/// the buttons Left 0, Right 1, Middle 2.
fn button_index(button: i64) -> usize {
    match button {
        1 => 2,
        2 => 1,
        _ => 0,
    }
}

/// `GetButtonByIndex`.
fn index_button(index: usize) -> i64 {
    match index {
        1 => 2,
        2 => 1,
        _ => 0,
    }
}

/// `NavigationTab`.
fn navigation(ui: &mut Ui, copy: &mut OptionsCopy) {
    ui.add_space(8.0);
    section(ui, "Mouse Navigation", 16.0);
    ui.horizontal(|ui| {
        ui.add_space(60.0);
        let headers = [("Mouse Button", 220.0), ("Hold Alt", 90.0), ("Sensitivity", 200.0)];
        for (header, width) in headers {
            ui.allocate_ui_with_layout(Vec2::new(width, 18.0), Layout::top_down(Align::Center), |ui| {
                ui.label(RichText::new(header).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT));
            });
        }
    });
    let rows = [
        ("Rotate", "RotateButton", "AltRotate", "RotateSensitivity", theme::ACCENT_MUTED_BLUE),
        ("Pan", "PanButton", "AltPan", "PanSensitivity", theme::ACCENT_MUTED_GREEN),
        ("Zoom", "ZoomButton", "AltZoom", "ZoomSensitivity", theme::ACCENT_MUTED_MAGENTA),
    ];
    for (label, button, alt, sensitivity, colour) in rows {
        ui.horizontal(|ui| {
            row_label(ui, label, 52.0);
            ui.add_space(8.0);
            let current = button_index(copy.number(Opt(button)) as i64);
            ui.allocate_ui_with_layout(Vec2::new(220.0, 26.0), Layout::left_to_right(Align::Center), |ui| {
                if let Some(chosen) = options_x(ui, &["Left", "Middle", "Right"], Some(current), Some(216.0)) {
                    copy.set(Opt(button), json!(index_button(chosen)));
                }
            });
            ui.allocate_ui_with_layout(Vec2::new(90.0, 26.0), Layout::top_down(Align::Center), |ui| {
                let mut held = copy.flag(Opt(alt));
                if ui.checkbox(&mut held, "").changed() {
                    copy.set(Opt(alt), Value::Bool(held));
                }
            });
            let mut value = copy.number(Opt(sensitivity)) as f32;
            let mut slider = egui::Slider::new(&mut value, 0.0..=2.0);
            slider = slider.show_value(false);
            slider = slider.trailing_fill(true);
            let response = ui.scope(|ui| {
                ui.visuals_mut().selection.bg_fill = colour;
                ui.add_sized(Vec2::new(200.0, 20.0), slider)
            });
            let response = response.inner.on_hover_text(format!("{label} Sensitivity"));
            if response.changed() {
                copy.set(Opt(sensitivity), json!(value as f64));
            }
        });
        ui.add_space(4.0);
    }
    ui.add_space(12.0);
    check(ui, copy, Opt("InvertY"), 60.0, "Invert Y axis");
    ui.add_space(8.0);
    check(ui, copy, Opt("FlipQE"), 60.0, "Flip Y Pan Direction for QE keys");
    note(ui, 96.0, "When disabled, Q pans up, E pans down. When enabled, the direction is reversed.", theme::LABEL_DEFAULT);
    ui.add_space(8.0);
    check(ui, copy, Opt("InvertZoom"), 60.0, "Invert zoom direction");
    ui.add_space(8.0);
    check(ui, copy, Opt("ShowNavigationHint"), 60.0, "Show viewport navigation hint for 10 seconds when Nerthus starts");
}

/// `LazyTab`: each part of the Lazy Menu hidden, muted or bright.
fn lazy(ui: &mut Ui, copy: &mut OptionsCopy) {
    ui.add_space(8.0);
    section(ui, "Lazy Menu", 8.0);
    // The grid's columns: 32 for the icon, the rest shared by the name and
    // the choice.
    let column = ((ui.available_width() - 32.0) / 2.0).max(120.0);
    for (member, icon, label) in LAZY_ROWS {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let (rect, _) = ui.allocate_exact_size(Vec2::new(32.0, 26.0), egui::Sense::hover());
            let icon_rect = egui::Rect::from_center_size(rect.center(), Vec2::splat(16.0));
            crate::icons::paint(ui, icon_rect, icon, theme::LABEL_DEFAULT);
            ui.allocate_ui_with_layout(Vec2::new(column, 26.0), Layout::left_to_right(Align::Center), |ui| {
                ui.set_min_size(Vec2::new(column, 26.0));
                ui.label(RichText::new(label).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT));
            });
            let current = copy.number(Opt(member));
            let index = (0.0..3.0).contains(&current).then_some(current as usize);
            if let Some(chosen) = options_x(ui, &["Hide", "Muted", "Bright"], index, Some(column - 8.0)) {
                copy.set(Opt(member), json!(chosen));
            }
        });
    }
}

/// `ExperimentalTab`.
fn experimental(ui: &mut Ui, copy: &mut OptionsCopy) {
    ui.add_space(8.0);
    section(ui, "Preview Options", 16.0);
    let flags = [
        ("CompactUI", "Compact Interface"),
        ("DisableAnimations", "Disable UI animations"),
        ("NewAutolayout", "Use new Auto Layout method"),
        ("AdaptiveDebounce", "Use Adaptive Debounce for Smoother Node Updates"),
        ("PopulateMRUOnStart", "Populate Recent List in Start Experience"),
    ];
    for (member, text) in flags {
        check(ui, copy, Opt(member), 16.0, text);
        ui.add_space(4.0);
    }
    ui.separator();
    ui.add_space(8.0);
    ui.label(RichText::new("Custom Locations to show in File Dialogs (one per line)").font(theme::interface(12.0)).color(theme::LABEL_DEFAULT));
    let setting = Pref("Locations", "Custom");
    let mut locations = copy.text(setting);
    let mut field = egui::TextEdit::multiline(&mut locations);
    field = field.desired_rows(8);
    field = field.desired_width(f32::INFINITY);
    if ui.add_sized(Vec2::new(ui.available_width(), 200.0), field).changed() {
        copy.set(setting, Value::String(locations));
    }
    ui.add_space(8.0);
    check(ui, copy, Pref("Locations", "ShowCustomOnly"), 0.0, "Show only these locations");
    note(ui, 0.0, "This reduces clutter in the directory tree. You can still access any location by typing a path, or using the breadcrumbs at the top.", theme::LABEL_DEFAULT);
}
