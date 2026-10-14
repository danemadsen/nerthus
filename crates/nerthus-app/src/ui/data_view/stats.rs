//! The Stats tab (`tabReadouts`): build statistics and memory buttons over
//! the readouts (`Readouts`) of the selection: the whole terrain when
//! nothing is selected, one node's state, ports and heights, or a sum of
//! several.

use egui::{Color32, Pos2, Rect, Stroke, StrokeKind, Ui, Vec2};
use nerthus_engine::{NodeState, RenderIntent};
use serde_json::Value;
use web_time::Instant;

use crate::document::ExportSettings;
use crate::editor::Editor;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::data_view::terrain::{matching_nodes, FilterCriteria, TerrainState};
use crate::ui::data_view::{toolbar, toolbar_divider, toolbar_row};
use crate::ui::widgets::icon_button;

/// The tab's state between frames.
#[derive(Default)]
pub struct StatsState {
    /// `ReadOut_Memory` shows until the selection changes (the selection
    /// it was asked for).
    memory: Option<Vec<i32>>,
}

/// One readout line: the header (right-aligned, `Accent-Muted-Gray`) and
/// the value (`Accent-Muted-Green` unless coloured); a header of `None` is
/// a value on its own line, an empty line a break.
enum Line {
    Pair(String, String, Option<Color32>),
    Value(String, Option<Color32>),
    Break,
}

pub fn show(ui: &mut Ui, area: Rect, editor: &Editor, state: &mut StatsState, terrain: &TerrainState, commands: &mut Vec<Command>) {
    let (inner, page_top) = toolbar(ui, area);
    let mut row = toolbar_row(ui, inner, false);
    if icon_button(&mut row, "histogram", "Build Statistics", None).clicked() {
        commands.push(Command::ShowBuildStatistics);
    }
    toolbar_divider(&mut row, 2.0, 4.0);
    let memory_clicked = icon_button(&mut row, "chart-pie", "Memory Statistics", None).clicked();
    if memory_clicked {
        state.memory = Some(editor.selection.clone());
    }
    if state.memory.as_ref().is_some_and(|selection| *selection != editor.selection) {
        state.memory = None;
    }

    let lines = if state.memory.is_some() {
        memory_lines(editor)
    } else {
        match editor.selection.len() {
            0 => terrain_lines(editor, terrain.filter),
            1 => node_lines(editor),
            _ => selection_lines(editor),
        }
    };

    // `readouts` (margin 8, 0, 8, 8) in a `ScrollViewer`.
    let page = Rect::from_min_max(Pos2::new(area.left() + 8.0, page_top), Pos2::new(area.right() - 8.0, area.bottom() - 8.0));
    let builder = egui::UiBuilder::new().max_rect(page);
    let mut page_ui = ui.new_child(builder);
    let mut scroll = egui::ScrollArea::vertical();
    scroll = scroll.id_salt("data-view-stats");
    scroll = scroll.auto_shrink([false, true]);
    scroll.show(&mut page_ui, |ui| readouts(ui, &lines));
}

/// The two `TextBlock`s side by side (monospace 11, padding 8; the headers
/// 16 in and right-aligned) on their bordered background.
fn readouts(ui: &mut Ui, lines: &[Line]) {
    let font = theme::monospace(11.0);
    let line_height = ui.painter().layout_no_wrap("0".to_string(), font.clone(), theme::ACCENT_MUTED_GRAY).size().y;
    let header_width = lines
        .iter()
        .filter_map(|line| match line {
            Line::Pair(header, ..) => Some(ui.painter().layout_no_wrap(format!("{header}: "), font.clone(), theme::ACCENT_MUTED_GRAY).size().x),
            _ => None,
        })
        .fold(0.0, f32::max);
    let width = ui.available_width();
    let value_left = 16.0 + 8.0 + header_width + 8.0 + 8.0;
    let value_width = (width - value_left - 8.0).max(20.0);

    // Lay the values out first: long ones wrap and take more lines.
    let mut rows: Vec<ReadoutRow> = Vec::new();
    for line in lines {
        let row = match line {
            Line::Pair(header, value, color) => {
                let header_galley = ui.painter().layout_no_wrap(format!("{header}: "), font.clone(), theme::ACCENT_MUTED_GRAY);
                let value_color = color.unwrap_or(theme::ACCENT_MUTED_GREEN);
                let value_galley = ui.painter().layout(value.clone(), font.clone(), value_color, value_width);
                ReadoutRow { header: Some(header_galley), value: Some(value_galley) }
            }
            Line::Value(value, color) => {
                let value_color = color.unwrap_or(theme::ACCENT_MUTED_GREEN);
                let value_galley = ui.painter().layout(value.clone(), font.clone(), value_color, value_width);
                ReadoutRow { header: None, value: Some(value_galley) }
            }
            Line::Break => ReadoutRow { header: None, value: None },
        };
        rows.push(row);
    }
    let row_height = |row: &ReadoutRow| row.value.as_ref().map_or(line_height, |galley| galley.size().y.max(line_height));
    let content_height: f32 = rows.iter().map(row_height).sum();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, content_height + 16.0), egui::Sense::hover());
    // `Window-Dark` behind, `Accent-Transparent-Green` around (corners 2).
    ui.painter().rect_filled(rect, 2.0, theme::WINDOW_DARK);
    ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, theme::ACCENT_TRANSPARENT_GREEN), StrokeKind::Inside);

    let mut y = rect.top() + 8.0;
    let header_right = rect.left() + 16.0 + 8.0 + header_width;
    for row in rows {
        let height = row_height(&row);
        if let Some(header) = row.header {
            let position = Pos2::new(header_right - header.size().x, y);
            ui.painter().galley(position, header, theme::ACCENT_MUTED_GRAY);
        }
        if let Some(value) = row.value {
            let position = Pos2::new(rect.left() + value_left, y);
            ui.painter().galley(position, value, theme::ACCENT_MUTED_GREEN);
        }
        y += height;
    }
}

/// A readout line as laid out.
struct ReadoutRow {
    header: Option<std::sync::Arc<egui::Galley>>,
    value: Option<std::sync::Arc<egui::Galley>>,
}

/// `ReadOut_None`: the terrain's (or the filter's) nodes, their cache,
/// the build resolution, what the build would write.
fn terrain_lines(editor: &Editor, filter: FilterCriteria) -> Vec<Line> {
    let ids: Vec<i32> = if filter == FilterCriteria::None { editor.document.terrain().nodes.keys().copied().collect() } else { matching_nodes(editor, filter) };
    let filtered = if filter == FilterCriteria::None { String::new() } else { format!(" (Filtered to {})", filter.humanized()) };
    let cache: usize = ids.iter().filter_map(|id| editor.node_caches.get(id)).map(|cache| cache.bytes).sum();
    let resolution = editor.document.build_definition().resolution;
    let area = resolution * resolution;
    let mut estimated = 0usize;
    let mut total_files = 0;
    let mut height_files = 0;
    let mut color_files = 0;
    for id in &ids {
        let Some(node) = editor.document.node(*id) else {
            continue;
        };
        let Some(export) = ExportSettings::of(node) else {
            continue;
        };
        // `Node.Exportables`: the outputs marked for export.
        let spec = nerthus_engine::Catalogue::get().node(&node.type_name);
        let port_intent = |name: &str| spec.and_then(|spec| spec.ports.iter().find(|port| port.name == name)).and_then(|port| port.render_intent.as_deref());
        let exportables: Vec<&nerthus_engine::PortData> = node.ports.iter().filter(|port| port.is_output() && port.is_exporting).collect();
        let colors = exportables.iter().filter(|port| port_intent(&port.name) == Some("Color")).count();
        total_files += exportables.len();
        color_files += colors;
        height_files += exportables.len() - colors;
        // Enabled exports only, colour ports
        // three channels, four bytes a value.
        if export.enabled {
            let channels = exportables.len() + 2 * colors;
            estimated += channels * area * std::mem::size_of::<f32>();
        }
    }
    vec![
        Line::Pair("Nodes".to_string(), format!("{}{filtered}", ids.len()), None),
        Line::Pair("Cache".to_string(), byte_size(cache), None),
        Line::Break,
        Line::Pair("Resolution".to_string(), resolution.to_string(), None),
        Line::Pair("Est. Size".to_string(), byte_size(estimated), None),
        Line::Pair("Total Files".to_string(), total_files.to_string(), None),
        Line::Pair("Height Files".to_string(), height_files.to_string(), None),
        Line::Pair("Color Files".to_string(), color_files.to_string(), None),
    ]
}

/// `ReadOut_SingleNode`: id, name (and type when renamed), state and build
/// time; then each cached port, the main output's heights, the cache.
fn node_lines(editor: &Editor) -> Vec<Line> {
    let Some(id) = editor.active_node() else {
        return terrain_lines(editor, FilterCriteria::None);
    };
    let Some(node) = editor.document.node(id) else {
        return terrain_lines(editor, FilterCriteria::None);
    };
    let spec = nerthus_engine::Catalogue::get().node(&node.type_name);
    let type_name = spec.map(|spec| spec.name.clone()).unwrap_or_else(|| node.type_name.clone());
    let name = if node.name == type_name { node.name.clone() } else { format!("{} ({type_name})", node.name) };
    let status = editor.status(id);
    let mut lines = vec![Line::Pair("ID".to_string(), id.to_string(), None), Line::Pair("Name".to_string(), name, Some(Color32::WHITE))];
    let state = match status.state {
        NodeState::Built => format!("Built {}", humanize_since(status.built_at)),
        other => state_name(other).to_string(),
    };
    lines.push(Line::Pair("State".to_string(), state, None));
    lines.push(Line::Pair("Build Time".to_string(), humanize_milliseconds(status.milliseconds), None));
    lines.push(Line::Break);
    if status.state == NodeState::Error {
        let error = status.error.clone().unwrap_or_default();
        lines.push(Line::Value(format!("Error:\n\n{error}"), Some(theme::ACCENT_RED)));
        return lines;
    }

    let cache = editor.node_caches.get(&id);
    for port in node.ports.iter().filter(|port| port.is_output()) {
        let cached = cache.and_then(|cache| cache.ports.iter().find(|cached| cached.name == port.name));
        match cached {
            Some(cached) => {
                let display = port.extra.get("DisplayName").and_then(Value::as_str).unwrap_or(&port.name).to_string();
                let size = resolution_text(cached.width, cached.height);
                let text = format!("{size} × {} ({})", cached.channels, intent_name(cached.intent));
                lines.push(Line::Pair(display, text, None));
            }
            None => lines.push(Line::Value("Cache not found.".to_string(), Some(theme::ACCENT_RED))),
        }
    }
    lines.push(Line::Break);
    if let Some((minimum, maximum)) = status.range {
        let height = editor.document.terrain().height;
        lines.push(Line::Pair("Min".to_string(), height_text(minimum, height), None));
        lines.push(Line::Pair("Max".to_string(), height_text(maximum, height), None));
        lines.push(Line::Pair("Range".to_string(), height_text(maximum - minimum, height), None));
        lines.push(Line::Break);
    }
    let bytes = cache.map_or(0, |cache| cache.bytes);
    lines.push(Line::Pair("Size".to_string(), byte_size(bytes), Some(theme::ACCENT_MUTED_BLUE)));
    lines
}

/// `ReadOut_MultipleNodes`: how many, their shared state, their build
/// time, their caches.
fn selection_lines(editor: &Editor) -> Vec<Line> {
    let selection = &editor.selection;
    let mut lines = vec![Line::Pair("Nodes".to_string(), format!("{} nodes selected.", selection.len()), Some(theme::ACCENT_MUTED_PURPLE))];
    let first_state = editor.status(selection[0]).state;
    if selection.iter().all(|id| editor.status(*id).state == first_state) {
        lines.push(Line::Pair("State".to_string(), state_name(first_state).to_string(), None));
    }
    let milliseconds: f32 = selection.iter().map(|id| editor.status(*id).milliseconds).sum();
    lines.push(Line::Pair("Build Time".to_string(), humanize_milliseconds(milliseconds), None));
    let caches: Vec<&crate::worker::NodeCache> = selection.iter().filter_map(|id| editor.node_caches.get(id)).collect();
    let count: usize = caches.iter().map(|cache| cache.ports.len()).sum();
    let bytes: usize = caches.iter().map(|cache| cache.bytes).sum();
    lines.push(Line::Pair("Total Caches".to_string(), count.to_string(), None));
    lines.push(Line::Pair("Total Size".to_string(), byte_size(bytes), None));
    lines
}

/// `ReadOut_Memory`: the cache, the rest of the process, the total.
fn memory_lines(editor: &Editor) -> Vec<Line> {
    let process = crate::memory::process_bytes().unwrap_or(0) as usize;
    let cache = editor.cache_bytes;
    let other = process.saturating_sub(cache);
    vec![
        Line::Pair("Total Cache".to_string(), gigabytes(cache), None),
        Line::Pair("Hot Memory".to_string(), gigabytes(cache), None),
        Line::Pair("Warm Memory".to_string(), gigabytes(0), None),
        Line::Pair("Other Uses".to_string(), gigabytes(other), None),
        Line::Pair("Total Memory".to_string(), gigabytes(process), None),
        Line::Break,
        Line::Pair(String::new(), "(Click button again to refresh)".to_string(), None),
    ]
}

/// `NodeState.ToString()`.
fn state_name(state: NodeState) -> &'static str {
    match state {
        NodeState::Unbuilt => "Unbuilt",
        NodeState::Built => "Built",
        NodeState::Error => "Error",
        NodeState::Bypassed => "Bypassed",
        NodeState::Baked => "Baked",
    }
}

/// `RenderIntent` as Humanizer writes it.
fn intent_name(intent: RenderIntent) -> &'static str {
    match intent {
        RenderIntent::Default => "Default",
        RenderIntent::Heightfield => "Heightfield",
        RenderIntent::Mask => "Mask",
        RenderIntent::Color => "Color",
        RenderIntent::Raw => "Raw",
    }
}

/// `Resolution.ToString()`: `{X} × {Y}`, grouped past 10000.
fn resolution_text(width: usize, height: usize) -> String {
    if width <= 10000 && height <= 10000 {
        return format!("{width} × {height}");
    }
    let width_digits = width.to_string();
    let height_digits = height.to_string();
    format!("{} × {}", grouped(&width_digits), grouped(&height_digits))
}

/// Digits with a comma every three from the right.
fn grouped(digits: &str) -> String {
    let mut text = String::new();
    for (index, digit) in digits.chars().enumerate() {
        let from_right = digits.len() - index;
        if index > 0 && from_right.is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

/// `{value:0.0000} / {value * height:##0,000.0}m`: the .NET format keeps
/// at least four whole digits, grouped.
fn height_text(value: f32, terrain_height: f32) -> String {
    let metres = value * terrain_height;
    let tenths = (metres.abs() as f64 * 10.0).round() as u64;
    let whole = tenths / 10;
    let fraction = tenths % 10;
    let digits = format!("{whole:04}");
    let grouped_digits = grouped(&digits);
    let sign = if metres < 0.0 && tenths > 0 { "-" } else { "" };
    format!("{value:.4} / {sign}{grouped_digits}.{fraction}m")
}

/// `ByteSize.ToString()`: the largest unit with a whole part, `0.##`
/// (nothing at all is "0 b", in bits).
pub(crate) fn byte_size(bytes: usize) -> String {
    let units = [("TB", 1u64 << 40), ("GB", 1 << 30), ("MB", 1 << 20), ("KB", 1 << 10), ("B", 1)];
    let value = bytes as u64;
    for (unit, size) in units {
        if value >= size {
            let amount = value as f64 / size as f64;
            let digits = nerthus_core::dotnet_format::format_decimals(amount, 2);
            return format!("{digits} {unit}");
        }
    }
    "0 b".to_string()
}

/// `Humanize("0.0 GB")`.
fn gigabytes(bytes: usize) -> String {
    let amount = bytes as f64 / (1u64 << 30) as f64;
    format!("{amount:.1} GB")
}

/// `TimeSpan.Humanize(1, maxUnit: Week, minUnit: Millisecond)`: the
/// largest whole unit.
pub(crate) fn humanize_milliseconds(milliseconds: f32) -> String {
    let total = milliseconds.max(0.0) as u64;
    let units = [("week", 7 * 24 * 3_600_000u64), ("day", 24 * 3_600_000), ("hour", 3_600_000), ("minute", 60_000), ("second", 1000)];
    for (unit, size) in units {
        if total >= size {
            return plural(total / size, unit);
        }
    }
    plural(total, "millisecond")
}

fn plural(count: u64, unit: &str) -> String {
    if count == 1 { format!("1 {unit}") } else { format!("{count} {unit}s") }
}

/// `DateTime.Humanize()` of when the node was built.
fn humanize_since(built_at: Option<Instant>) -> String {
    let Some(built_at) = built_at else {
        return "now".to_string();
    };
    let elapsed = built_at.elapsed();
    let seconds = elapsed.as_secs();
    if elapsed.as_millis() < 500 {
        return "now".to_string();
    }
    if seconds < 60 {
        return if seconds == 1 { "one second ago".to_string() } else { format!("{seconds} seconds ago") };
    }
    if seconds < 120 {
        return "a minute ago".to_string();
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes} minutes ago");
    }
    if minutes < 120 {
        return "an hour ago".to_string();
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours} hours ago");
    }
    if hours < 48 {
        return "yesterday".to_string();
    }
    format!("{} days ago", hours / 24)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heights_keep_four_grouped_digits() {
        assert_eq!(height_text(0.0, 2600.0), "0.0000 / 0,000.0m");
        assert_eq!(height_text(0.5, 2600.0), "0.5000 / 1,300.0m");
        assert_eq!(height_text(0.1, 1000.0), "0.1000 / 0,100.0m");
        assert_eq!(height_text(5.0, 2600.0), "5.0000 / 13,000.0m");
    }

    #[test]
    fn sizes_and_times_read_like_humanizer() {
        assert_eq!(byte_size(0), "0 b");
        assert_eq!(byte_size(4 * 1024 * 1024), "4 MB");
        assert_eq!(byte_size(1536 * 1024), "1.5 MB");
        assert_eq!(humanize_milliseconds(0.0), "0 milliseconds");
        assert_eq!(humanize_milliseconds(1.0), "1 millisecond");
        assert_eq!(humanize_milliseconds(1999.0), "1 second");
        assert_eq!(humanize_milliseconds(125_000.0), "2 minutes");
        assert_eq!(resolution_text(1024, 1024), "1024 × 1024");
        assert_eq!(resolution_text(16384, 16384), "16,384 × 16,384");
    }
}
