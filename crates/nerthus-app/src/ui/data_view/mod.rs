//! The Data View (`DataViewArea`): the panel the graph toolbar's chevron
//! opens between the graph and its toolbar. Four tabs hang under it:
//! Terrain (the project's graphs, groups, nodes and notes as a tree),
//! Build (the build settings and the nodes marked for export), Stats
//! (readouts of the selection) and Vars (the project's variables).

mod build;
pub(super) mod stats;
mod terrain;
mod variables;

use egui::{Align, Color32, Layout, Pos2, Rect, Sense, Shape, Stroke, Ui, UiBuilder, Vec2};

use crate::editor::Editor;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::graph_canvas::GraphState;
use crate::ui::widgets::toolbar_background;

pub use terrain::{dimmed_nodes, FilterCriteria};

/// `DataViewColumn` while open: `1.05*` beside the graph's `5*`, at least
/// 340 wide.
pub const MIN_WIDTH: f32 = 340.0;
pub const DEFAULT_SHARE: f32 = 1.05 / 6.05;
/// The tab strip under the panel (`Grid Height="26"`) and the 25 the
/// panel's border stops above the bottom.
const TABS_HEIGHT: f32 = 26.0;
const BORDER_BOTTOM_MARGIN: f32 = 25.0;
/// `TabBorderThickness`.
const BORDER: f32 = 1.5;
/// `DataView-Border`.
const BORDER_COLOR: Color32 = Color32::from_rgb(0x4b, 0x42, 0x55);
/// `InfinityListBoxTopItemTemplate`: tabs at least 64 wide, 25 high,
/// overlapping by 9 (`Margin -8,0,-1,0`; the first tab `0,0,-1,0`).
const TAB_MIN_WIDTH: f32 = 64.0;
const TAB_HEIGHT: f32 = 25.0;
const TAB_OVERLAP: f32 = 9.0;

/// The Data View's tabs (`Tabs`); Dev stays hidden.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataTab {
    Terrain,
    Build,
    Stats,
    Vars,
}

const TABS: [(DataTab, &str); 4] = [(DataTab::Terrain, "Terrain"), (DataTab::Build, "Build"), (DataTab::Stats, "Stats"), (DataTab::Vars, "Vars")];

/// The panel's state between frames.
pub struct DataViewState {
    /// `IsExpanded` (`User.Options.ExpandDataView`).
    pub expanded: bool,
    pub tab: DataTab,
    /// The panel's share of the width it splits with the graph (the
    /// splitter moves it).
    pub share: f32,
    pub terrain: terrain::TerrainState,
    pub build: build::BuildState,
    pub stats: stats::StatsState,
    pub variables: variables::VariablesState,
}

impl Default for DataViewState {
    fn default() -> Self {
        Self {
            expanded: false,
            tab: DataTab::Terrain,
            share: DEFAULT_SHARE,
            terrain: terrain::TerrainState::default(),
            build: build::BuildState::default(),
            stats: stats::StatsState::default(),
            variables: variables::VariablesState::default(),
        }
    }
}

impl DataViewState {
    /// `ToggleCollapse`.
    pub fn toggle(&mut self) {
        self.expanded = !self.expanded;
    }

    /// `ShowTab`: opens the panel on a tab.
    pub fn show_tab(&mut self, tab: DataTab) {
        self.expanded = true;
        self.tab = tab;
    }

    /// A new document: what belonged to the old one goes.
    pub fn reset(&mut self) {
        self.terrain = terrain::TerrainState::default();
        self.build = build::BuildState::default();
        self.stats = stats::StatsState::default();
        self.variables = variables::VariablesState::default();
    }
}

/// Draws the panel into `rect` (`DataViewArea` inside its margin).
pub fn show(ui: &mut Ui, rect: Rect, editor: &mut Editor, state: &mut DataViewState, graph: &mut GraphState, commands: &mut Vec<Command>) {
    // `Root`: `Panel-Background`, its left corners rounded 8.
    let root_corners = egui::CornerRadius { nw: 8, ne: 0, sw: 8, se: 0 };
    ui.painter().rect_filled(rect, root_corners, theme::PANEL_BACKGROUND);

    let framed = Rect::from_min_max(rect.min, Pos2::new(rect.right(), rect.bottom() - BORDER_BOTTOM_MARGIN));
    let content = Rect::from_min_max(framed.min + Vec2::splat(BORDER), Pos2::new(framed.right() - BORDER, framed.bottom()));
    let mut content_ui = ui.new_child(UiBuilder::new().max_rect(content));
    content_ui.set_clip_rect(content.intersect(ui.clip_rect()));
    match state.tab {
        DataTab::Terrain => terrain::show(&mut content_ui, content, editor, &mut state.terrain, graph, commands),
        DataTab::Build => build::show(&mut content_ui, content, editor, &mut state.build, commands),
        DataTab::Stats => stats::show(&mut content_ui, content, editor, &mut state.stats, &state.terrain, commands),
        DataTab::Vars => variables::show(&mut content_ui, content, editor, &mut state.variables, commands),
    }
    frame_border(ui, framed);

    let strip = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - TABS_HEIGHT), rect.max);
    tab_strip(ui, strip, state);
}

/// The inner `Border`: 1.5 of `DataView-Border` down the sides and across
/// the top, its top-left corner rounded 7, open at the bottom where the
/// tab strip's line closes it.
fn frame_border(ui: &Ui, framed: Rect) {
    let half = BORDER / 2.0;
    let inner = Rect::from_min_max(framed.min + Vec2::splat(half), Pos2::new(framed.right() - half, framed.bottom()));
    let radius = 7.0 - half;
    let corner_center = Pos2::new(inner.left() + radius, inner.top() + radius);
    let mut points = vec![inner.left_bottom()];
    for step in 0..=6 {
        let angle = std::f32::consts::PI + step as f32 * std::f32::consts::FRAC_PI_2 / 6.0;
        let offset = Vec2::new(angle.cos(), angle.sin()) * radius;
        points.push(corner_center + offset);
    }
    points.push(inner.right_top());
    points.push(inner.right_bottom());
    let stroke = Stroke::new(BORDER, BORDER_COLOR);
    ui.painter().add(Shape::line(points, stroke));
}

/// The tabs (`GraphTabTopList`) hanging under the panel: the chosen one
/// outlined (the tab path flipped, `ScaleY=-1`) with its name in
/// `Label-Emphasis`; the others under a line of `DataView-Border` with
/// their names in `Label-Default`; `ExtraBorder` carries the line on.
fn tab_strip(ui: &mut Ui, strip: Rect, state: &mut DataViewState) {
    // `Margin="0,-1.5,0,0"` on the list and `0,1,0,0` on its panel.
    let top = strip.top() - 0.5;
    let line_y = top + BORDER / 2.0;
    let line_stroke = Stroke::new(BORDER, BORDER_COLOR);
    let mut x = strip.left();
    let mut selected_outline = None;
    for (index, (tab, name)) in TABS.iter().enumerate() {
        let first = index == 0;
        let selected = state.tab == *tab;
        let text_color = if selected { theme::LABEL_EMPHASIS } else { theme::LABEL_DEFAULT };
        let galley = ui.painter().layout_no_wrap(name.to_string(), theme::interface(12.0), text_color);
        let width = galley.size().x.max(TAB_MIN_WIDTH);
        let tab_rect = Rect::from_min_size(Pos2::new(x, top), Vec2::new(width, TAB_HEIGHT));
        let response = ui.interact(tab_rect, ui.id().with(("data-view-tab", index)), Sense::click());
        if response.clicked() {
            state.tab = *tab;
        }
        if selected {
            selected_outline = Some(tab_path(tab_rect, first));
        } else {
            // `bottomline`: from 8 in (the first tab: its left edge).
            let line_start = if first { tab_rect.left() } else { tab_rect.left() + 8.0 };
            let segment = [Pos2::new(line_start, line_y), Pos2::new(tab_rect.right(), line_y)];
            ui.painter().line_segment(segment, line_stroke);
        }
        let text_position = Pos2::new(tab_rect.center().x - galley.size().x / 2.0, tab_rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(text_position, galley, text_color);
        x = tab_rect.right() - TAB_OVERLAP;
    }
    // The chosen tab sits over its neighbours (`ZIndex 999`).
    if let Some(points) = selected_outline {
        ui.painter().add(Shape::line(points, line_stroke));
    }
    // `ExtraBorder`: from the last tab (which ends 1 past `x + 8`) on.
    let tabs_end = x + TAB_OVERLAP - 1.0;
    let extra = [Pos2::new(tabs_end, strip.top() + BORDER / 2.0), Pos2::new(strip.right(), strip.top() + BORDER / 2.0)];
    ui.painter().line_segment(extra, line_stroke);
}

/// The tab's `Path`, stretched to `rect` and flipped so its feet meet the
/// line at the top and its rounded corners hang below. The first tab
/// (`Tag="lefttab"`) has a straight left side.
fn tab_path(rect: Rect, first: bool) -> Vec<Pos2> {
    // In the path's own units (before the flip, y down from the top of a
    // 25-high tab): the stretch maps `left..right` onto the tab.
    let (left, right) = if first { (4.0, 78.0) } else { (0.0, 88.0) };
    let scale_x = rect.width() / (right - left);
    let at = |x: f32, y: f32| Pos2::new(rect.left() + (x - left) * scale_x, rect.top() + (25.0 - y));
    let quarter = std::f32::consts::FRAC_PI_2;
    let arc = |center: (f32, f32), from: f32, to: f32, points: &mut Vec<Pos2>| {
        for step in 0..=6 {
            let angle = from + (to - from) * step as f32 / 6.0;
            points.push(at(center.0 + 4.0 * angle.cos(), center.1 + 4.0 * angle.sin()));
        }
    };
    let mut points = Vec::new();
    if first {
        // M4,25 L4,4 A 8,0 L70,0 A 74,4 L74,21 A 78,25.
        points.push(at(4.0, 25.0));
        arc((8.0, 4.0), 2.0 * quarter, 3.0 * quarter, &mut points);
        arc((70.0, 4.0), 3.0 * quarter, 4.0 * quarter, &mut points);
        arc((78.0, 21.0), 2.0 * quarter, quarter, &mut points);
    } else {
        // M0,25 L8,25 A 12,21 L12,4 A 16,0 L72,0 A 76,4 L76,21 A 80,25 L88,25.
        points.push(at(0.0, 25.0));
        arc((8.0, 21.0), quarter, 0.0, &mut points);
        arc((16.0, 4.0), 2.0 * quarter, 3.0 * quarter, &mut points);
        arc((72.0, 4.0), 3.0 * quarter, 4.0 * quarter, &mut points);
        arc((80.0, 21.0), 2.0 * quarter, quarter, &mut points);
        points.push(at(88.0, 25.0));
    }
    points
}

/// A tab's toolbar (`Toolbar` docked at the top: margin `Toolbar.Margin`,
/// `Toolbar.MinHeight` high): returns its inner rectangle (inside the
/// border of 1 and `Toolbar.Padding`) and where the page under it starts.
fn toolbar(ui: &Ui, area: Rect) -> (Rect, f32) {
    let sizes = theme::sizes(ui.ctx());
    let margin = sizes.toolbar_margin;
    let bar = Rect::from_min_size(area.min + Vec2::splat(margin), Vec2::new(area.width() - 2.0 * margin, sizes.toolbar_min_height));
    toolbar_background(ui, bar, theme::PANEL_BORDER, 1.0);
    let [padding_across, padding_down] = sizes.toolbar_padding;
    let inner = bar.shrink(1.0).shrink2(Vec2::new(padding_across, padding_down));
    (inner, bar.bottom() + margin)
}

/// A child ui laying a toolbar out from its left (or, `from_right`, its
/// right) with no gaps.
fn toolbar_row(ui: &mut Ui, inner: Rect, from_right: bool) -> Ui {
    let layout = if from_right { Layout::right_to_left(Align::Center) } else { Layout::left_to_right(Align::Center) };
    let builder = UiBuilder::new().max_rect(inner).layout(layout);
    let mut row = ui.new_child(builder);
    row.spacing_mut().item_spacing = Vec2::ZERO;
    row
}

/// A `Separator` standing in a toolbar row: 24 high, 2 wide (its border of
/// 1 each side) in `Menu-Separator` at 0.2, with margins `before` and
/// `after` along the row.
fn toolbar_divider(ui: &mut Ui, before: f32, after: f32) {
    ui.add_space(before);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(2.0, 24.0), Sense::hover());
    let color = theme::MENU_SEPARATOR.gamma_multiply(0.2);
    ui.painter().rect_filled(rect, 0.0, color);
    ui.add_space(after);
}

/// A section's title bar (`TextBlock`: `Panel-Border`, semibold, padding
/// 12, 4).
fn section_title(ui: &mut Ui, title: &str) {
    let galley = ui.painter().layout_no_wrap(title.to_string(), theme::semibold(12.0), theme::LABEL_DEFAULT);
    let height = galley.size().y + 8.0;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::PANEL_BORDER);
    let position = Pos2::new(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(position, galley, theme::LABEL_DEFAULT);
}

/// `Button-Subtle`: `Button-Background`, padding 24, 6, at least 64 wide,
/// corners 2; `Button-Hover` under the pointer, `Button-Pressed` held.
fn subtle_button(ui: &mut Ui, text: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::interface(12.0), theme::LABEL_DEFAULT);
    let size = Vec2::new((galley.size().x + 48.0).max(64.0), galley.size().y + 12.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = if response.is_pointer_button_down_on() {
        theme::BUTTON_PRESSED
    } else if response.hovered() {
        theme::BUTTON_HOVER
    } else {
        theme::BUTTON_BACKGROUND
    };
    ui.painter().rect_filled(rect, 2.0, fill);
    let position = rect.center() - galley.size() / 2.0;
    ui.painter().galley(position, galley, theme::LABEL_DEFAULT);
    response
}
