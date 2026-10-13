//! The options the graph, its search and the toolbox read
//! (`Options`).

use serde_json::json;

use super::UserOptions;

/// `ToolboxStyle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolboxStyle {
    Menus,
    Panel,
}

/// `ToolboxMode`: how wide the menus' buttons are (Long, Compact and
/// Expanded in the toolbox's menu).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolboxMode {
    Single,
    Double,
    Expanded,
}

impl ToolboxMode {
    fn name(self) -> &'static str {
        match self {
            Self::Single => "Single",
            Self::Double => "Double",
            Self::Expanded => "Expanded",
        }
    }
}

impl UserOptions {
    /// `ToolboxStyle`.
    pub fn toolbox_style(&self) -> ToolboxStyle {
        match self.name("ToolboxStyle") {
            "Panel" => ToolboxStyle::Panel,
            _ => ToolboxStyle::Menus,
        }
    }

    pub fn set_toolbox_style(&mut self, style: ToolboxStyle) {
        let name = match style {
            ToolboxStyle::Menus => "Menus",
            ToolboxStyle::Panel => "Panel",
        };
        self.set("ToolboxStyle", json!(name));
    }

    /// `ToolboxMode`.
    pub fn toolbox_mode(&self) -> ToolboxMode {
        match self.name("ToolboxMode") {
            "Single" => ToolboxMode::Single,
            "Double" => ToolboxMode::Double,
            _ => ToolboxMode::Expanded,
        }
    }

    pub fn set_toolbox_mode(&mut self, mode: ToolboxMode) {
        self.set("ToolboxMode", json!(mode.name()));
    }

    /// `ToolboxMenus`: families nested in submenus (`Nested`) or listed
    /// flat under headers (`Flat`).
    pub fn toolbox_nested(&self) -> bool {
        self.name("ToolboxMenus") != "Flat"
    }

    pub fn set_toolbox_nested(&mut self, nested: bool) {
        let name = if nested { "Nested" } else { "Flat" };
        self.set("ToolboxMenus", json!(name));
    }

    /// `ShowIconsInToolbox`.
    pub fn show_icons_in_toolbox(&self) -> bool {
        self.flag("ShowIconsInToolbox")
    }

    /// `ToolboxPinHighUse` (Pin To Top).
    pub fn toolbox_pin_high_use(&self) -> bool {
        self.flag("ToolboxPinHighUse")
    }

    pub fn set_toolbox_pin_high_use(&mut self, on: bool) {
        self.set_flag("ToolboxPinHighUse", on);
    }

    /// `ToolboxHighlightHighUse` (Highlight).
    pub fn toolbox_highlight_high_use(&self) -> bool {
        self.flag("ToolboxHighlightHighUse")
    }

    pub fn set_toolbox_highlight_high_use(&mut self, on: bool) {
        self.set_flag("ToolboxHighlightHighUse", on);
    }

    /// `ToolboxLeftAlignLabels` (Align Left).
    pub fn toolbox_left_align(&self) -> bool {
        self.flag("ToolboxLeftAlignLabels")
    }

    pub fn set_toolbox_left_align(&mut self, on: bool) {
        self.set_flag("ToolboxLeftAlignLabels", on);
    }

    /// `ToolboxCompact` (Compact Mode).
    pub fn toolbox_compact(&self) -> bool {
        self.flag("ToolboxCompact")
    }

    pub fn set_toolbox_compact(&mut self, on: bool) {
        self.set_flag("ToolboxCompact", on);
    }

    /// `ToolboxCategorized` (Categorized).
    pub fn toolbox_categorized(&self) -> bool {
        self.flag("ToolboxCategorized")
    }

    pub fn set_toolbox_categorized(&mut self, on: bool) {
        self.set_flag("ToolboxCategorized", on);
    }

    /// `ToolboxColumns`: the panel's columns (1 to 6).
    pub fn toolbox_columns(&self) -> usize {
        (self.number("ToolboxColumns") as usize).clamp(1, 6)
    }

    pub fn set_toolbox_columns(&mut self, columns: usize) {
        self.set("ToolboxColumns", json!(columns.clamp(1, 6)));
    }

    /// `Favorites`: what the node search offers before anything is typed,
    /// until enough has been used.
    pub fn favorites(&self) -> Vec<String> {
        self.strings("Favorites")
    }

    /// `ShowFamilyName`: search results show their family.
    pub fn show_family_name(&self) -> bool {
        self.flag("ShowFamilyName")
    }

    /// `ShowIconsInSearchResults`.
    pub fn show_icons_in_search_results(&self) -> bool {
        self.flag("ShowIconsInSearchResults")
    }

    /// `MaximumSearchResults`: more go into "more...".
    pub fn maximum_search_results(&self) -> usize {
        self.number("MaximumSearchResults").max(1.0) as usize
    }

    /// `SnapToItems` (Graph Surface › Snap to Items).
    pub fn snap_to_items(&self) -> bool {
        self.flag("SnapToItems")
    }

    pub fn set_snap_to_items(&mut self, on: bool) {
        self.set_flag("SnapToItems", on);
    }

    /// `SnapToPorts`: a dragged connection snaps to ports near it.
    pub fn snap_to_ports(&self) -> bool {
        self.flag("SnapToPorts")
    }

    /// `DoubleClickConnect`: a double-clicked port starts a connection the
    /// next click ends.
    pub fn double_click_connect(&self) -> bool {
        self.flag("DoubleClickConnect")
    }

    /// `HideBookmarkShortcuts`: no bookmark buttons beside the graph tabs.
    pub fn hide_bookmark_shortcuts(&self) -> bool {
        self.flag("HideBookmarkShortcuts")
    }

    /// `AlwaysShowMaskPort`: copies of maskable nodes get a mask port.
    pub fn always_show_mask_port(&self) -> bool {
        self.flag("AlwaysShowMaskPort")
    }

    /// `MoveLazyCursor`: the Lazy Menu moves the pointer to what it opens.
    pub fn move_lazy_cursor(&self) -> bool {
        self.flag("MoveLazyCursor")
    }

    /// The Lazy Menu's entries (`LazyViewport`, `LazyGraph`, …): 0 hides
    /// one, 1 shows it with a faint icon, 2 with its colour.
    pub fn lazy_level(&self, name: &str) -> i64 {
        self.number(name) as i64
    }

    /// `ShowAdvancedTools`: the Variables view is there.
    pub fn show_advanced_tools(&self) -> bool {
        self.flag("ShowAdvancedTools")
    }
}
