//! The interface options (`Options`, saved as `user.options`): every
//! member, with its constructor value and its Newtonsoft default. Nerthus
//! acts on the ones it has a use for so far.

use serde_json::{json, Map, Value};

use crate::user::members::{dictionary, initial_values, list, read, value, write, Member};

/// `Options`' members in declaration order.
const MEMBERS: [Member; 91] = [
    value("OptionsVersion", "5", "5"),
    value("CompactUI", "false", "false"),
    value("UiScale", "1.2", "0"),
    value("Monitor", "-1", "-1"),
    value("ViewportConfig", "\"Embedded\"", "\"Embedded\""),
    value("InvertY", "true", "true"),
    value("InvertZoom", "false", "false"),
    value("AltRotate", "false", "false"),
    value("AltPan", "false", "false"),
    value("AltZoom", "false", "false"),
    value("RotateButton", "0", "0"),
    value("PanButton", "2", "2"),
    value("ZoomButton", "1", "1"),
    value("RotateSensitivity", "1.0", "1.0"),
    value("PanSensitivity", "1.0", "1.0"),
    value("ZoomSensitivity", "1.0", "1.0"),
    value("InclineSensitivity", "1.0", "1.0"),
    value("KeyRotateSensitivity", "1.0", "1.0"),
    value("PreciseValue", "false", "false"),
    value("ShowTicks", "true", "true"),
    value("AlwaysShowMaskPort", "false", "false"),
    // `BuildResolution` has no member at 0, so these are always written.
    value("DefaultBuildResolution", "\"x4096\"", "0"),
    value("DefaultPreviewResolution", "\"x1024\"", "0"),
    value("DefaultRenderResolution", "\"x2048\"", "0"),
    value("ShowRegionReferenceImage", "false", "false"),
    value("RegionReferenceImageOpacity", "0.25", "0.0"),
    value("ToolboxMode", "\"Expanded\"", "\"Expanded\""),
    value("ToolboxMenus", "\"Nested\"", "\"Nested\""),
    value("ShowIconsInToolbox", "true", "true"),
    list("Favorites", "[\"Mountain\", \"Ridge\", \"Perlin\", \"ErosionPro\", \"Adjust\", \"Combine\"]"),
    value("ShowAdvancedTools", "true", "true"),
    value("ExpandDataView", "true", "true"),
    value("ShowNodeAlerts", "true", "true"),
    value("ShowBuildTime", "true", "true"),
    value("MoveLazyCursor", "true", "true"),
    value("ViewportBackground", "{\"R\": 0.01960784, \"G\": 0.01960784, \"B\": 0.01960784}", "{\"R\": 0.0, \"G\": 0.0, \"B\": 0.0}"),
    value("ShowGrid", "true", "true"),
    value("ShowCompass", "true", "true"),
    value("ShowPlace", "true", "true"),
    value("RecordGraphMovementUndos", "true", "true"),
    value("ShowFamilyName", "true", "true"),
    value("ShowIconsInSearchResults", "true", "true"),
    value("MaximumSearchResults", "10", "10"),
    value("GridSize", "25", "25"),
    value("ShowGraphGrid", "false", "false"),
    value("SnapToGrid", "false", "false"),
    value("SnapToItems", "true", "true"),
    value("SnapToPorts", "true", "true"),
    value("DefaultNodeSize", "\"Standard\"", "\"Default\""),
    dictionary(
        "NodeDisplay",
        "{\"Combine\": \"Small\", \"Constant\": \"Small\", \"Shaper\": \"Small\", \"Adjust\": \"Small\", \"Invert\": \"Small\", \"Clamp\": \"Small\", \"Clip\": \"Small\", \"Mask\": \"Small\", \"Autolevel\": \"Small\"}",
    ),
    value("DataViewIcons", "\"Color\"", "\"Color\""),
    value("PreventHiRes", "false", "false"),
    value("ViewportQuality", "5", "5"),
    value("LazyViewport", "2", "2"),
    value("LazyLighting", "2", "2"),
    value("LazyGraph", "2", "2"),
    value("LazyProperties", "2", "2"),
    value("LazyModifiers", "2", "2"),
    value("LazyTree", "2", "2"),
    value("LazyVariables", "2", "2"),
    value("LazyBuild", "2", "2"),
    value("LazySettings", "2", "2"),
    value("FlipQE", "false", "false"),
    value("BuildShine", "true", "true"),
    value("ClassicNodeStyle", "false", "false"),
    value("DoubleClickConnect", "true", "false"),
    value("HideTabs", "false", "false"),
    value("HideBookmarkShortcuts", "false", "false"),
    value("DontClearViewport", "false", "false"),
    value("ConfineShortcuts", "false", "false"),
    value("ClearOldFiles", "true", "true"),
    value("StackedSatelliteMaps", "false", "false"),
    value("DisableAnimations", "false", "false"),
    value("ThumbnailStorage", "\"Central\"", "\"Central\""),
    value("AdaptiveDebounce", "false", "false"),
    value("MaskGradient", "0", "0"),
    value("NewAutolayout", "false", "false"),
    value("ScreenDeviceName", "null", "null"),
    value("ToolboxPinHighUse", "false", "false"),
    value("ToolboxHighlightHighUse", "false", "false"),
    value("ToolboxLeftAlignLabels", "true", "true"),
    value("ToolboxCompact", "true", "true"),
    value("ToolboxStyle", "\"Menus\"", "\"Menus\""),
    value("PopulateMRUOnStart", "true", "true"),
    value("ToolboxColumns", "2", "2"),
    value("ToolboxCategorized", "true", "true"),
    value("AcceptOnLostFocus", "false", "false"),
    value("SliderSeed", "false", "false"),
    value("ShowNavigationHint", "true", "true"),
    value("AutosaveInterval", "5", "0"),
    value("GizmoType", "0", "0"),
];

/// `IconState`: how the Data View's tree shows its icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconState {
    Color,
    Muted,
    Hide,
}

/// The options as they stand, and whether they changed since saved.
#[derive(Clone, Debug)]
pub struct UserOptions {
    values: Map<String, Value>,
    changed: bool,
}

impl Default for UserOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl UserOptions {
    /// `new Options()`.
    pub fn new() -> Self {
        Self { values: initial_values(&MEMBERS), changed: false }
    }

    /// The options a saved file holds.
    pub fn from_json(saved: &Value) -> Self {
        Self { values: read(&MEMBERS, saved), changed: false }
    }

    /// The file's text (`Formatting.Indented`).
    pub fn to_json(&self) -> String {
        let mut next_id = 1;
        let written = write(&MEMBERS, &self.values, &mut next_id);
        serde_json::to_string_pretty(&written).unwrap_or_default()
    }

    pub fn changed(&self) -> bool {
        self.changed
    }

    pub fn mark_saved(&mut self) {
        self.changed = false;
    }

    /// Saves on the next chance even though nothing was edited.
    pub fn mark_changed(&mut self) {
        self.changed = true;
    }

    pub(super) fn flag(&self, name: &str) -> bool {
        self.values.get(name).and_then(Value::as_bool).unwrap_or(false)
    }

    pub(super) fn set_flag(&mut self, name: &str, on: bool) {
        self.set(name, json!(on));
    }

    pub(super) fn number(&self, name: &str) -> f64 {
        self.values.get(name).and_then(Value::as_f64).unwrap_or(0.0)
    }

    pub(super) fn name(&self, name: &str) -> &str {
        self.values.get(name).and_then(Value::as_str).unwrap_or_default()
    }

    /// A member's value as it stands (null when missing), for Options'
    /// pages, which work on members by name.
    pub fn value(&self, name: &str) -> Value {
        self.values.get(name).cloned().unwrap_or(Value::Null)
    }

    /// Sets a member by name (Options' pages).
    pub fn set_value(&mut self, name: &str, value: Value) {
        self.set(name, value);
    }

    /// A list member's entries (`Favorites`).
    pub(super) fn strings(&self, name: &str) -> Vec<String> {
        let list = self.values.get(name).and_then(Value::as_array);
        list.map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
    }

    pub(super) fn set(&mut self, name: &str, value: Value) {
        if self.values.get(name) == Some(&value) {
            return;
        }
        self.values.insert(name.to_string(), value);
        self.changed = true;
    }

    /// `UiScale`: the interface's scale (1.2 at first). A file
    /// without it would give 0; that and anything wild fall back to 1.
    pub fn ui_scale(&self) -> f32 {
        let scale = self.number("UiScale") as f32;
        if (0.5..=3.0).contains(&scale) { scale } else { 1.0 }
    }

    pub fn set_ui_scale(&mut self, scale: f32) {
        let rounded = (scale as f64 * 100.0).round() / 100.0;
        self.set("UiScale", json!(rounded));
    }

    /// `Monitor`: the monitor Nerthus starts on (General's "Start Nerthus
    /// on"), -1 for the primary one.
    pub fn monitor(&self) -> i64 {
        self.number("Monitor") as i64
    }

    /// `GizmoType`: gizmos in the 3D view as well as the 2D one (`3D + 2D
    /// Viewport`).
    pub fn gizmos_in_3d(&self) -> bool {
        self.value("GizmoType").as_i64() == Some(1)
    }

    /// `ViewportConfig`: the viewport in a window of its own (`Floating`)
    /// rather than the main one (`Embedded`).
    pub fn viewport_floating(&self) -> bool {
        self.value("ViewportConfig").as_str() == Some("Floating")
    }

    pub fn set_viewport_floating(&mut self, floating: bool) {
        let config = if floating { "Floating" } else { "Embedded" };
        self.set("ViewportConfig", json!(config));
    }

    /// `ShowRegionReferenceImage`: the Regions tab shows the saved
    /// reference picture behind the regions.
    pub fn show_region_reference(&self) -> bool {
        self.flag("ShowRegionReferenceImage")
    }

    pub fn set_show_region_reference(&mut self, shown: bool) {
        self.set_flag("ShowRegionReferenceImage", shown);
    }

    /// `RegionReferenceImageOpacity`: what shows behind the regions, at
    /// this opacity (0 reads as 0.25, as `RegionManager_Loaded` makes it).
    pub fn region_reference_opacity(&self) -> f32 {
        let opacity = self.number("RegionReferenceImageOpacity") as f32;
        if opacity == 0.0 { 0.25 } else { opacity }
    }

    pub fn set_region_reference_opacity(&mut self, opacity: f32) {
        self.set("RegionReferenceImageOpacity", nerthus_engine::json::f32_value(opacity));
    }

    /// `ExpandDataView`: the Data View is open.
    pub fn expand_data_view(&self) -> bool {
        self.flag("ExpandDataView")
    }

    pub fn set_expand_data_view(&mut self, expanded: bool) {
        self.set_flag("ExpandDataView", expanded);
    }

    /// `DisableAnimations` (the flag `WPF.FadeIn` checks): fades jump
    /// straight to their end.
    pub fn disable_animations(&self) -> bool {
        self.flag("DisableAnimations")
    }

    /// `ShowGraphGrid` and `SnapToGrid` (Graph > Graph Surface).
    pub fn show_graph_grid(&self) -> bool {
        self.flag("ShowGraphGrid")
    }

    pub fn set_show_graph_grid(&mut self, shown: bool) {
        self.set_flag("ShowGraphGrid", shown);
    }

    pub fn snap_to_grid(&self) -> bool {
        self.flag("SnapToGrid")
    }

    pub fn set_snap_to_grid(&mut self, snapping: bool) {
        self.set_flag("SnapToGrid", snapping);
    }

    /// `GridSize`: what nodes snap to (`GraphControl.GridUnit`).
    pub fn grid_size(&self) -> f32 {
        let size = self.number("GridSize") as f32;
        if size > 0.0 { size } else { 25.0 }
    }

    /// The viewport's elements: `ShowGrid`, `ShowPlace` (Bounds) and
    /// `ShowCompass`.
    pub fn viewport_elements(&self) -> (bool, bool, bool) {
        (self.flag("ShowGrid"), self.flag("ShowPlace"), self.flag("ShowCompass"))
    }

    pub fn set_viewport_elements(&mut self, grid: bool, bounds: bool, compass: bool) {
        self.set_flag("ShowGrid", grid);
        self.set_flag("ShowPlace", bounds);
        self.set_flag("ShowCompass", compass);
    }

    /// `ViewportBackground`: behind the terrain when there is no sky.
    pub fn viewport_background(&self) -> [f32; 3] {
        let color = self.values.get("ViewportBackground");
        let component = |name: &str| color.and_then(|color| color.get(name)).and_then(Value::as_f64).unwrap_or(0.0) as f32;
        [component("R"), component("G"), component("B")]
    }

    /// How the mouse and keys drive the 3D view, as `SetInputOptions`
    /// sends them to the viewport.
    pub fn navigation(&self) -> nerthus_render::NavigationSettings {
        let button = |name: &str| nerthus_render::NavigationSettings::button(self.number(name) as i64);
        let sensitivity = |name: &str| self.number(name) as f32;
        nerthus_render::NavigationSettings {
            invert_y: self.flag("InvertY"),
            invert_zoom: self.flag("InvertZoom"),
            alt_rotate: self.flag("AltRotate"),
            alt_pan: self.flag("AltPan"),
            alt_zoom: self.flag("AltZoom"),
            rotate_button: button("RotateButton"),
            pan_button: button("PanButton"),
            zoom_button: button("ZoomButton"),
            flip_qe: self.flag("FlipQE"),
            rotate_sensitivity: sensitivity("RotateSensitivity"),
            pan_sensitivity: sensitivity("PanSensitivity"),
            zoom_sensitivity: sensitivity("ZoomSensitivity"),
            incline_sensitivity: sensitivity("InclineSensitivity"),
            key_rotate_sensitivity: sensitivity("KeyRotateSensitivity"),
        }
    }

    /// `ShowNavigationHint`: the 3D view lists its mouse controls for a
    /// while when it starts.
    pub fn show_navigation_hint(&self) -> bool {
        self.flag("ShowNavigationHint")
    }

    /// `MaskGradient`: the `MaskGradient` masks start out in, by index.
    pub fn mask_gradient(&self) -> i64 {
        self.values.get("MaskGradient").and_then(Value::as_i64).unwrap_or(0)
    }

    /// `ShowBuildTime`: nodes show how long they took.
    pub fn show_build_time(&self) -> bool {
        self.flag("ShowBuildTime")
    }

    /// `DataViewIcons`.
    pub fn data_view_icons(&self) -> IconState {
        match self.name("DataViewIcons") {
            "Muted" => IconState::Muted,
            "Hide" => IconState::Hide,
            _ => IconState::Color,
        }
    }

    /// `DefaultNodeSize`: how nodes without a size of their own draw.
    pub fn default_node_size(&self) -> &str {
        self.name("DefaultNodeSize")
    }

    /// `NodeDisplay`: the size a node named so starts at.
    pub fn node_display(&self, node_name: &str) -> Option<&str> {
        let entries = self.values.get("NodeDisplay")?;
        entries.get(node_name).and_then(Value::as_str)
    }

    /// `AutosaveInterval`: seconds between safety autosaves as last chosen
    /// in the Autosave menu (a fresh `Options` holds 5).
    pub fn autosave_interval(&self) -> i64 {
        self.values.get("AutosaveInterval").and_then(Value::as_i64).unwrap_or(300)
    }

    pub fn set_autosave_interval(&mut self, seconds: i64) {
        self.set("AutosaveInterval", json!(seconds));
    }

    /// `ThumbnailStorage` (Options › Paths › File Thumbnails): `None`,
    /// `Central` (the data folder's `Thumbnails`) or `Local` (beside the
    /// project).
    pub fn thumbnail_storage(&self) -> &str {
        self.name("ThumbnailStorage")
    }

    /// `ClearOldFiles`: autosaves, logs and thumbnails older than 45 days
    /// go at start.
    pub fn clear_old_files(&self) -> bool {
        self.flag("ClearOldFiles")
    }

    /// `PopulateMRUOnStart`: the Start dialog lists recent files.
    pub fn populate_recent_on_start(&self) -> bool {
        self.flag("PopulateMRUOnStart")
    }

    /// `PreciseValue`: sliders show four decimals rather than two.
    pub fn precise_value(&self) -> bool {
        self.flag("PreciseValue")
    }

    /// `ShowTicks`: integer sliders under 100 mark each step (no page in
    /// Options shows it).
    pub fn show_ticks(&self) -> bool {
        self.flag("ShowTicks")
    }

    /// `SliderSeed`: seeds get a slider instead of the seed box.
    pub fn slider_seed(&self) -> bool {
        self.flag("SliderSeed")
    }

    /// `StackedSatelliteMaps`: the palette list is one strip a row.
    pub fn stacked_satellite_maps(&self) -> bool {
        self.flag("StackedSatelliteMaps")
    }

    /// `ClassicNodeStyle` (Node Style › Vibrant): nodes are outlined in
    /// their colour, and a chosen one in the primary accent.
    pub fn vibrant_nodes(&self) -> bool {
        self.flag("ClassicNodeStyle")
    }

    /// `BuildShine`: a node shines once it has built.
    pub fn build_shine(&self) -> bool {
        self.flag("BuildShine")
    }

    /// `AcceptOnLostFocus`: a value typed into a popup is kept when the
    /// popup closes by a click elsewhere, not only by Enter or Tab.
    pub fn accept_on_lost_focus(&self) -> bool {
        self.flag("AcceptOnLostFocus")
    }

    /// `ConfineShortcuts`: the node keys work only with the pointer over
    /// the graph.
    pub fn confine_shortcuts(&self) -> bool {
        self.flag("ConfineShortcuts")
    }

    /// `HideTabs`: no tab strip while the project has a single graph tab.
    pub fn hide_tabs(&self) -> bool {
        self.flag("HideTabs")
    }

    /// `RecordGraphMovementUndos`: panning the graph is an undo step.
    pub fn record_graph_movements(&self) -> bool {
        self.flag("RecordGraphMovementUndos")
    }

    /// `DontClearViewport`: the viewport keeps the last preview when no
    /// node, or more than one, is selected.
    pub fn dont_clear_viewport(&self) -> bool {
        self.flag("DontClearViewport")
    }

    /// `PreventHiRes` (Conserve memory when displaying 4K terrains): a 4K
    /// terrain is drawn with the 2K mesh.
    pub fn prevent_hi_res(&self) -> bool {
        self.flag("PreventHiRes")
    }

    /// `ViewportQuality` (Antialiasing): the frames a second the 3D view
    /// draws while it has the focus (`SetQuality`): None 20, 2x 30, 4x and
    /// 8x 60. Anything else leaves the view's own 30.
    pub fn viewport_frame_rate(&self) -> u32 {
        match self.values.get("ViewportQuality").and_then(Value::as_i64) {
            Some(0) => 20,
            Some(1) => 30,
            Some(2) | Some(3) => 60,
            _ => 30,
        }
    }

    /// `CompactUI` (Compact Interface): the shell drawn at the compact
    /// sizes (`Size.Compact`).
    pub fn compact_ui(&self) -> bool {
        self.flag("CompactUI")
    }

    /// `NewAutolayout`: the auto layout places nodes on a grid of blocks.
    pub fn new_autolayout(&self) -> bool {
        self.flag("NewAutolayout")
    }

    /// `AdaptiveDebounce`: how long a parameter edit waits before its node
    /// builds follows how long the node took last time.
    pub fn adaptive_debounce(&self) -> bool {
        self.flag("AdaptiveDebounce")
    }

    /// What the controls read from the options as they draw.
    pub fn interface_settings(&self) -> crate::theme::InterfaceSettings {
        crate::theme::InterfaceSettings {
            compact: self.compact_ui(),
            precise_values: self.precise_value(),
            show_ticks: self.show_ticks(),
            accept_on_lost_focus: self.accept_on_lost_focus(),
            seed_sliders: self.slider_seed(),
            stacked_palettes: self.stacked_satellite_maps(),
            disable_animations: self.disable_animations(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_options_leave_out_their_defaults() {
        let options = UserOptions::new();
        let written: Value = serde_json::from_str(&options.to_json()).unwrap();
        let members: Vec<&str> = written.as_object().unwrap().keys().map(String::as_str).collect();
        // Members at their default stay out (`IgnoreAndPopulate`).
        assert!(!members.contains(&"ExpandDataView"));
        assert!(!members.contains(&"OptionsVersion"));
        // The rest, in declaration order, with `$id`s.
        let expected = [
            "$id",
            "UiScale",
            "DefaultBuildResolution",
            "DefaultPreviewResolution",
            "DefaultRenderResolution",
            "RegionReferenceImageOpacity",
            "Favorites",
            "ViewportBackground",
            "DefaultNodeSize",
            "NodeDisplay",
            "DoubleClickConnect",
            "AutosaveInterval",
        ];
        assert_eq!(members, expected);
        assert_eq!(written["Favorites"]["$id"], json!("2"));
        assert_eq!(written["NodeDisplay"]["$id"], json!("3"));
        assert_eq!(written["UiScale"], json!(1.2));
    }

    #[test]
    fn options_round_trip() {
        let mut options = UserOptions::new();
        options.set_expand_data_view(false);
        options.set_ui_scale(1.0);
        options.set_show_graph_grid(true);
        assert!(options.changed());
        let saved: Value = serde_json::from_str(&options.to_json()).unwrap();
        let read_back = UserOptions::from_json(&saved);
        assert!(!read_back.expand_data_view());
        assert_eq!(read_back.ui_scale(), 1.0);
        assert!(read_back.show_graph_grid());
        assert_eq!(read_back.node_display("Combine"), Some("Small"));
        assert_eq!(read_back.default_node_size(), "Standard");
        assert_eq!(read_back.data_view_icons(), IconState::Color);
    }
}
