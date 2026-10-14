//! `StatisticsWindow` ("Statistics", 1200 by 720): two `DonutChart`s on
//! the left, each node's build time and the memory its cache takes, and
//! on the right every node with both and how long it took (`hh:mm:ss.fff`),
//! slowest first, under a TOTAL row. Clicking a slice chooses its node in
//! the list; double-clicking a slice or a row goes to the node in the
//! graph and selects it.

use egui::{Color32, FontId, Id, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};

use crate::editor::Editor;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::data_view::stats::humanize_milliseconds;
use crate::ui::editors::palette_color;
use crate::ui::tool_window;

/// `DonutChart.OuterRadius` and `InnerRadius`.
const OUTER_RADIUS: f32 = 150.0;
const INNER_RADIUS: f32 = 100.0;
/// The slices' outline (`Color.FromRgb(45, 15, 65)`).
const SLICE_OUTLINE: Color32 = Color32::from_rgb(45, 15, 65);
/// A slice under the pointer (`IsMouseOver` → `Opacity` 0.8).
const HOVER_OPACITY: f32 = 0.8;
/// `statview`'s columns.
const NAME_COLUMN: f32 = 250.0;
const SIZE_COLUMN: f32 = 140.0;
const TIME_COLUMN: f32 = 180.0;
/// `ListViewItem`'s padding around 14-point text.
const ROW_PADDING: f32 = 4.0;
/// `ListBox-Background`, and a chosen row (`DarkGray2`).
const LIST_BACKGROUND: Color32 = Color32::from_rgb(0x16, 0x16, 0x16);
const SELECTED_ROW: Color32 = Color32::from_rgb(0x32, 0x32, 0x32);
/// The charts' colours, dark violet through red and orange to pale yellow
/// (matplotlib's magma, CC0, from 0.2 to 1), sampled once a slice.
const RAMP: [[u8; 3]; 9] = [
    [59, 15, 112],
    [100, 26, 128],
    [140, 41, 129],
    [183, 55, 121],
    [222, 73, 104],
    [247, 112, 92],
    [254, 159, 109],
    [254, 207, 146],
    [252, 253, 191],
];
const HINT: &str = "Click any Chart item to select a node in the List. Double-click any Chart or List item to select the node in the graph.";

/// A slice: the node's name, its share and id.
#[derive(Clone, Debug, PartialEq)]
struct Slice {
    name: String,
    value: f64,
    id: i32,
}

/// A row of the list: name, megabytes, milliseconds and the node (0 for
/// TOTAL).
#[derive(Clone, Debug, PartialEq)]
struct Row {
    name: String,
    megabytes: i64,
    milliseconds: f64,
    id: i32,
}

/// `ChartMode`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ChartMode {
    Time,
    Memory,
}

/// What a chart or the list was asked.
enum Picked {
    Nothing,
    Selected(i32),
    GoTo(i32),
}

/// The window's figures, gathered when it opens (`StatisticsWindow_Loaded`).
#[derive(Clone, Debug, PartialEq)]
pub struct StatisticsWindow {
    times: Vec<Slice>,
    sizes: Vec<Slice>,
    rows: Vec<Row>,
    selected: Option<i32>,
    open: bool,
}

impl StatisticsWindow {
    pub fn new(editor: &Editor) -> Self {
        let mut times = Vec::new();
        let mut sizes = Vec::new();
        let mut rows = Vec::new();
        for node in editor.document.terrain().nodes.values() {
            let bytes = editor.node_caches.get(&node.id).map_or(0, |cache| cache.bytes);
            let megabytes = (bytes / 1024 / 1024) as i64;
            let milliseconds = editor.status(node.id).milliseconds as f64;
            rows.push(Row { name: node.name.clone(), megabytes, milliseconds, id: node.id });
            times.push(Slice { name: node.name.clone(), value: milliseconds, id: node.id });
            sizes.push(Slice { name: node.name.clone(), value: megabytes as f64, id: node.id });
        }
        let total_megabytes = rows.iter().map(|row| row.megabytes).sum();
        let total_milliseconds = rows.iter().map(|row| row.milliseconds).sum();
        rows.push(Row { name: "TOTAL".to_string(), megabytes: total_megabytes, milliseconds: total_milliseconds, id: 0 });
        // `OrderByDescending`, which keeps ties in order.
        times.sort_by(|left, right| right.value.total_cmp(&left.value));
        sizes.sort_by(|left, right| right.value.total_cmp(&left.value));
        rows.sort_by(|left, right| right.milliseconds.total_cmp(&left.milliseconds));
        Self { times, sizes, rows, selected: None, open: true }
    }
}

/// The colour of slice `index` of `count`: the ramp at the slice's middle,
/// as a ramp shrunk to `count` pixels gives it.
fn slice_colour(index: usize, count: usize) -> Color32 {
    let position = (index as f32 + 0.5) / count.max(1) as f32;
    let scaled = position.clamp(0.0, 1.0) * (RAMP.len() - 1) as f32;
    let lower = (scaled.floor() as usize).min(RAMP.len() - 2);
    let fraction = scaled - lower as f32;
    let mix = |channel: usize| {
        let from = RAMP[lower][channel] as f32;
        let to = RAMP[lower + 1][channel] as f32;
        (from + (to - from) * fraction).round() as u8
    };
    Color32::from_rgb(mix(0), mix(1), mix(2))
}

/// Humanizer's `ByteSize.Humanize()` of so many megabytes: the largest
/// unit holding at least one, to two decimals (`0.##`).
pub(crate) fn humanize_megabytes(megabytes: i64) -> String {
    let bytes = megabytes as f64 * 1024.0 * 1024.0;
    let units = [("TB", 1024f64.powi(4)), ("GB", 1024f64.powi(3)), ("MB", 1024f64.powi(2)), ("KB", 1024.0), ("B", 1.0)];
    for (symbol, size) in units {
        if bytes.abs() >= size {
            let rounded = (bytes / size * 100.0).round() / 100.0;
            let digits = format!("{rounded:.2}");
            let trimmed = digits.trim_end_matches('0').trim_end_matches('.');
            return format!("{trimmed} {symbol}");
        }
    }
    "0 b".to_string()
}

/// A slice's caption under its name: its time (`TimeSpan.Humanize` with
/// "milliseconds" shortened to "ms") or its size.
fn slice_text(slice: &Slice, mode: ChartMode) -> String {
    match mode {
        ChartMode::Time => humanize_milliseconds(slice.value as f32).replace("milliseconds", "ms"),
        ChartMode::Memory => humanize_megabytes(slice.value as i64),
    }
}

/// `{0:hh\:mm\:ss\.fff}` in its four colours (`ParamsZ-Color1`, `2`, `4`
/// and `8`), the separators in the list's own.
fn time_cell(milliseconds: f64) -> Vec<(String, Color32)> {
    let whole = milliseconds.max(0.0).floor() as u64;
    let hours = whole / 3_600_000 % 24;
    let minutes = whole / 60_000 % 60;
    let seconds = whole / 1000 % 60;
    let fraction = whole % 1000;
    vec![
        (format!("{hours:02}"), palette_color(2, 0)),
        (":".to_string(), theme::LABEL_DEFAULT),
        (format!("{minutes:02}"), palette_color(2, 1)),
        (":".to_string(), theme::LABEL_DEFAULT),
        (format!("{seconds:02}"), palette_color(2, 3)),
        (".".to_string(), theme::LABEL_DEFAULT),
        (format!("{fraction:03}"), palette_color(2, 7)),
    ]
}

/// The window while it is open; what is double-clicked is gone to.
pub fn show(ctx: &egui::Context, window: &mut Option<StatisticsWindow>, commands: &mut Vec<Command>) {
    let Some(statistics) = window.as_mut() else {
        return;
    };
    let mut open = statistics.open;
    let mut picked = Vec::new();
    let mut frame = egui::Frame::window(&ctx.global_style());
    frame.fill = theme::WINDOW_BACKGROUND;
    frame.inner_margin = egui::Margin::same(8);
    let mut statistics_window = egui::Window::new("Statistics");
    statistics_window = statistics_window.open(&mut open);
    statistics_window = statistics_window.collapsible(false);
    statistics_window = statistics_window.frame(frame);
    statistics_window = tool_window::place(ctx, statistics_window, Id::new("statistics-window"), Vec2::new(1200.0, 720.0));
    statistics_window.show(ctx, |ui| {
        let area = ui.available_rect_before_wrap();
        ui.allocate_rect(area, Sense::hover());
        let middle = area.center();
        let left_top = Rect::from_min_max(area.min, Pos2::new(middle.x, middle.y));
        let left_bottom = Rect::from_min_max(Pos2::new(area.left(), middle.y), Pos2::new(middle.x, area.bottom()));
        let right = Rect::from_min_max(Pos2::new(middle.x, area.top()), Pos2::new(area.right(), area.bottom() - 36.0));
        picked.push(chart_cell(ui, left_top, "Build Time", &statistics.times, ChartMode::Time));
        picked.push(chart_cell(ui, left_bottom, "Cache Size", &statistics.sizes, ChartMode::Memory));
        picked.push(statistics_list(ui, right, &statistics.rows, &mut statistics.selected));
        // The hint along the bottom (margin 4).
        let hint = ui.painter().layout(HINT.to_string(), theme::interface(12.0), theme::LABEL_DEFAULT, area.width() - 8.0);
        let hint_position = Pos2::new(area.center().x - hint.size().x / 2.0, area.bottom() - 4.0 - hint.size().y);
        ui.painter().galley(hint_position, hint, theme::LABEL_DEFAULT);
    });
    for pick in picked {
        match pick {
            Picked::Nothing => {}
            Picked::Selected(id) => statistics.selected = Some(id),
            Picked::GoTo(id) => {
                // `Chart_ItemDoubleClicked` chooses the row too. TOTAL has
                // no node.
                statistics.selected = Some(id);
                if id != 0 {
                    commands.push(Command::GoToNode(id));
                }
            }
        }
    }
    statistics.open = open;
    if !open {
        *window = None;
    }
}

/// A chart's cell: its title (14, bold) at the top, the donut in the
/// middle and, over its hole, the slice under the pointer.
fn chart_cell(ui: &mut Ui, cell: Rect, title: &str, slices: &[Slice], mode: ChartMode) -> Picked {
    let title_galley = ui.painter().layout_no_wrap(title.to_string(), theme::semibold(14.0), theme::LABEL_EMPHASIS);
    let title_position = Pos2::new(cell.center().x - title_galley.size().x / 2.0, cell.top());
    ui.painter().galley(title_position, title_galley, theme::LABEL_EMPHASIS);
    let centre = cell.center();
    let total: f64 = slices.iter().map(|slice| slice.value).sum();
    if total <= 0.0 || !total.is_finite() {
        return Picked::Nothing;
    }
    let chart_rect = Rect::from_center_size(centre, Vec2::splat(OUTER_RADIUS * 2.0));
    let response = ui.interact(chart_rect, ui.id().with(("donut", title)), Sense::click());
    let hovered = response.hover_pos().and_then(|pointer| slice_at(pointer, centre, slices, total));

    let mut start = 0.0f64;
    for (index, slice) in slices.iter().enumerate() {
        let sweep = slice.value / total * 360.0;
        let colour = slice_colour(index, slices.len());
        let fill = if hovered == Some(index) { colour.gamma_multiply(HOVER_OPACITY) } else { colour };
        paint_slice(ui, centre, start as f32, sweep as f32, fill);
        start += sweep;
    }
    if let Some(index) = hovered {
        let slice = &slices[index];
        let caption = format!("{}\n{}", slice.name, slice_text(slice, mode));
        let mut job = egui::text::LayoutJob::simple(caption, theme::interface(12.0), theme::LABEL_DEFAULT, INNER_RADIUS * 2.0 - 16.0);
        job.halign = egui::Align::Center;
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        let position = Pos2::new(centre.x, centre.y - galley.size().y / 2.0);
        ui.painter().galley(position, galley, theme::LABEL_DEFAULT);
    }
    let Some(index) = hovered else {
        return Picked::Nothing;
    };
    let id = slices[index].id;
    if response.double_clicked() {
        return Picked::GoTo(id);
    }
    if response.clicked() {
        return Picked::Selected(id);
    }
    Picked::Nothing
}

/// The slice under `pointer`: in the ring, by its angle clockwise from the
/// top.
fn slice_at(pointer: Pos2, centre: Pos2, slices: &[Slice], total: f64) -> Option<usize> {
    let offset = pointer - centre;
    let distance = offset.length();
    if !(INNER_RADIUS..=OUTER_RADIUS).contains(&distance) {
        return None;
    }
    let angle = (offset.y.atan2(offset.x).to_degrees() as f64 + 90.0).rem_euclid(360.0);
    let mut start = 0.0;
    for (index, slice) in slices.iter().enumerate() {
        let sweep = slice.value / total * 360.0;
        if angle >= start && angle < start + sweep {
            return Some(index);
        }
        start += sweep;
    }
    None
}

/// `GetPoint`: on the circle of `radius`, `angle` degrees clockwise from
/// the top.
fn point_at(centre: Pos2, angle: f32, radius: f32) -> Pos2 {
    let radians = (angle - 90.0).to_radians();
    Pos2::new(centre.x + radius * radians.cos(), centre.y + radius * radians.sin())
}

/// A slice of the ring from `start` through `sweep` degrees, filled and
/// outlined.
fn paint_slice(ui: &Ui, centre: Pos2, start: f32, sweep: f32, fill: Color32) {
    if sweep <= 0.0 {
        return;
    }
    let steps = ((sweep / 2.0).ceil() as usize).max(2);
    let mut outer = Vec::with_capacity(steps + 1);
    let mut inner = Vec::with_capacity(steps + 1);
    for step in 0..=steps {
        let angle = start + sweep * step as f32 / steps as f32;
        outer.push(point_at(centre, angle, OUTER_RADIUS));
        inner.push(point_at(centre, angle, INNER_RADIUS));
    }
    let mut mesh = egui::Mesh::default();
    for (outer_point, inner_point) in outer.iter().zip(&inner) {
        mesh.colored_vertex(*outer_point, fill);
        mesh.colored_vertex(*inner_point, fill);
    }
    for step in 0..steps as u32 {
        let first = step * 2;
        mesh.add_triangle(first, first + 1, first + 2);
        mesh.add_triangle(first + 1, first + 3, first + 2);
    }
    ui.painter().add(Shape::mesh(mesh));
    let mut outline = outer;
    outline.extend(inner.into_iter().rev());
    ui.painter().add(Shape::closed_line(outline, Stroke::new(1.0, SLICE_OUTLINE)));
}

/// `statview`: the column headers, then a row a node (14 points, padding
/// 4); a click chooses a row, a double click goes to its node.
fn statistics_list(ui: &mut Ui, rect: Rect, rows: &[Row], selected: &mut Option<i32>) -> Picked {
    ui.painter().rect_filled(rect, 0.0, LIST_BACKGROUND);
    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, theme::LISTBOX_BORDER), egui::StrokeKind::Inside);
    let font = theme::interface(14.0);
    let mono = theme::monospace(14.0);
    let line_height = ui.painter().layout_no_wrap("Ag".to_string(), font.clone(), theme::LABEL_DEFAULT).size().y;
    let row_height = line_height + ROW_PADDING * 2.0;
    let header_rect = Rect::from_min_size(rect.min + Vec2::new(1.0, 1.0), Vec2::new(rect.width() - 2.0, row_height));
    let headers = [("Name", NAME_COLUMN), ("Size in Memory", SIZE_COLUMN), ("Build Time", TIME_COLUMN)];
    let mut x = header_rect.left() + 2.0 + ROW_PADDING;
    for (header, width) in headers {
        let galley = ui.painter().layout_no_wrap(header.to_string(), font.clone(), theme::LABEL_EMPHASIS);
        ui.painter().galley(Pos2::new(x + 2.0, header_rect.center().y - galley.size().y / 2.0), galley, theme::LABEL_EMPHASIS);
        x += width;
    }

    let body = Rect::from_min_max(Pos2::new(rect.left() + 1.0, header_rect.bottom()), rect.max - Vec2::splat(1.0));
    let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body));
    let mut picked = Picked::Nothing;
    let mut scroll = egui::ScrollArea::vertical();
    scroll = scroll.id_salt("statistics-list");
    scroll = scroll.auto_shrink([false, false]);
    scroll.show(&mut body_ui, |ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        for row in rows {
            let (row_rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_height), Sense::click());
            if *selected == Some(row.id) {
                ui.painter().rect_filled(row_rect, 0.0, SELECTED_ROW);
            }
            let text_y = row_rect.center().y;
            let name_left = row_rect.left() + 2.0 + ROW_PADDING + 2.0;
            paint_cell(ui, &row.name, font.clone(), theme::LABEL_DEFAULT, Pos2::new(name_left, text_y), false, NAME_COLUMN - 8.0);
            let size_right = row_rect.left() + 2.0 + ROW_PADDING + NAME_COLUMN + SIZE_COLUMN - 4.0;
            let size_text = format!("{} MB", row.megabytes);
            paint_cell(ui, &size_text, mono.clone(), theme::LABEL_DEFAULT, Pos2::new(size_right, text_y), true, SIZE_COLUMN);
            let time_right = size_right + TIME_COLUMN;
            paint_time(ui, row.milliseconds, mono.clone(), Pos2::new(time_right, text_y));
            if response.double_clicked() {
                picked = Picked::GoTo(row.id);
            } else if response.clicked() {
                picked = Picked::Selected(row.id);
            }
        }
    });
    picked
}

/// One cell's text, left-aligned from `anchor` or right-aligned to it.
fn paint_cell(ui: &Ui, text: &str, font: FontId, colour: Color32, anchor: Pos2, right: bool, width: f32) {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, colour);
    job.wrap.max_width = width;
    job.wrap.max_rows = 1;
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    let left = if right { anchor.x - galley.size().x } else { anchor.x };
    ui.painter().galley(Pos2::new(left, anchor.y - galley.size().y / 2.0), galley, colour);
}

/// The build time, right-aligned to `anchor`, in its colours.
fn paint_time(ui: &Ui, milliseconds: f64, font: FontId, anchor: Pos2) {
    let mut job = egui::text::LayoutJob::default();
    for (part, colour) in time_cell(milliseconds) {
        let format = egui::TextFormat::simple(font.clone(), colour);
        job.append(&part, 0.0, format);
    }
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    let position = Pos2::new(anchor.x - galley.size().x, anchor.y - galley.size().y / 2.0);
    ui.painter().galley(position, galley, theme::LABEL_DEFAULT);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_times_read_as_written() {
        assert_eq!(humanize_megabytes(0), "0 b");
        assert_eq!(humanize_megabytes(16), "16 MB");
        assert_eq!(humanize_megabytes(1536), "1.5 GB");
        let parts: Vec<String> = time_cell(3_723_004.9).into_iter().map(|(text, _)| text).collect();
        assert_eq!(parts.concat(), "01:02:03.004");
    }

    #[test]
    fn slices_are_found_clockwise_from_the_top() {
        let slices = vec![Slice { name: "a".into(), value: 3.0, id: 1 }, Slice { name: "b".into(), value: 1.0, id: 2 }];
        let centre = Pos2::new(200.0, 200.0);
        // Right of the centre is 90 degrees round: the first slice (0-270).
        assert_eq!(slice_at(Pos2::new(325.0, 200.0), centre, &slices, 4.0), Some(0));
        // Left is 270: the second.
        assert_eq!(slice_at(Pos2::new(75.0, 199.0), centre, &slices, 4.0), Some(1));
        assert_eq!(slice_at(Pos2::new(200.0, 200.0), centre, &slices, 4.0), None);
        assert_eq!(slice_colour(0, 1), Color32::from_rgb(222, 73, 104));
    }
}
