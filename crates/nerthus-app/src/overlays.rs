//! The viewport's overlays (Viewport Helpers › Overlay): a pattern or an
//! image of your own laid over the shaded terrain (`B.ApplyOverlay`, an
//! Overlay blend mixed in at the chosen opacity) to judge scale and
//! placement. The patterns (labelled cells, checkers, dots, lines,
//! quadrants, a UV ramp) are drawn at the render's size.
//!
//! The choice is the project's (`TerrainState.Viewport`: `OverlayOpacity`
//! and `OverlayImage` as indices, `CustomOverlayFile`).

use std::path::Path;
use std::sync::{Arc, Mutex};

use nerthus_engine::{Map, Resolution};
use nerthus_nodes::transformer::{self, ResizeMethod};
use serde_json::{Map as JsonMap, Value};

/// `OverlayOpacity`: the menu's entries and what each mixes in.
pub const OVERLAY_OPACITIES: [(&str, f32); 6] = [("0%", 0.0), ("10%", 0.1), ("25%", 0.25), ("50%", 0.5), ("75%", 0.75), ("100%", 1.0)];
/// `OverlayImage`'s entries (`[Description]`s), the last an image of your
/// own.
pub const OVERLAY_IMAGES: [&str; 9] = ["Colors1", "Colors2", "Dots", "Dark", "Light", "Lines", "Quads", "UV", "Custom"];
pub const CUSTOM_OVERLAY: usize = 8;
/// `OverlayChanged`: picking an overlay while none shows shows it at 50%.
pub const SHOWN_OPACITY: usize = 3;
/// `[DefaultValue(3)]`: Dark.
const DEFAULT_IMAGE: usize = 3;

/// The project's overlay.
#[derive(Clone, Debug, PartialEq)]
pub struct OverlaySettings {
    /// Index into [`OVERLAY_OPACITIES`].
    pub opacity: usize,
    /// Index into [`OVERLAY_IMAGES`].
    pub image: usize,
    pub custom_file: String,
}

impl Default for OverlaySettings {
    fn default() -> Self {
        Self { opacity: 0, image: DEFAULT_IMAGE, custom_file: String::new() }
    }
}

impl OverlaySettings {
    /// What a saved `Viewport` holds; members left out are at their
    /// `[DefaultValue]`.
    pub fn from_state(viewport: Option<&JsonMap<String, Value>>) -> Self {
        let member = |name: &str| viewport.and_then(|viewport| viewport.get(name));
        let index = |name: &str, default: usize, count: usize| {
            let saved = member(name).and_then(Value::as_u64).map(|value| value as usize);
            saved.filter(|value| *value < count).unwrap_or(default)
        };
        let custom_file = member("CustomOverlayFile").and_then(Value::as_str).unwrap_or_default().to_string();
        Self {
            opacity: index("OverlayOpacity", 0, OVERLAY_OPACITIES.len()),
            image: index("OverlayImage", DEFAULT_IMAGE, OVERLAY_IMAGES.len()),
            custom_file,
        }
    }

    /// Writes the overlay into a saved `Viewport`, leaving out what is at
    /// its default.
    pub fn write_to(&self, viewport: &mut JsonMap<String, Value>) {
        if self.opacity == 0 {
            viewport.remove("OverlayOpacity");
        } else {
            viewport.insert("OverlayOpacity".to_string(), Value::from(self.opacity as u64));
        }
        if self.image == DEFAULT_IMAGE {
            viewport.remove("OverlayImage");
        } else {
            viewport.insert("OverlayImage".to_string(), Value::from(self.image as u64));
        }
        if self.custom_file.is_empty() {
            viewport.remove("CustomOverlayFile");
        } else {
            viewport.insert("CustomOverlayFile".to_string(), Value::from(self.custom_file.clone()));
        }
        crate::viewport_render::order_viewport_members(viewport);
    }

    /// `B.OverlayOpacity`.
    pub fn opacity_value(&self) -> f32 {
        OVERLAY_OPACITIES.get(self.opacity).map_or(0.0, |(_, value)| *value)
    }
}

/// What a render lays over itself: the overlay, and the part of the
/// terrain shown (a previewed region: left, top and side as fractions of
/// the terrain's width).
#[derive(Clone, Debug, PartialEq)]
pub struct OverlayJob {
    pub settings: OverlaySettings,
    pub portion: Option<[f32; 3]>,
}

/// The overlay made last, kept while the render's size and the choice
/// stay the same (`B.OverlayMap`, `PreviewOverlayMap`).
struct MadeOverlay {
    job: OverlayJob,
    resolution: Resolution,
    map: Option<Arc<Map>>,
}

static MADE: Mutex<Option<MadeOverlay>> = Mutex::new(None);

/// `B.ApplyOverlay`: each channel mixed towards the Overlay blend of the
/// render and the overlay, by the opacity. Renders the overlay cannot be
/// made for stay as they are.
pub fn apply(render: Map, job: &OverlayJob) -> Map {
    let opacity = job.settings.opacity_value();
    if opacity == 0.0 {
        return render;
    }
    let Some(overlay) = overlay_for(job, render.resolution()) else {
        return render;
    };
    let mut channels = render.into_channels();
    for (index, channel) in channels.iter_mut().enumerate() {
        let layer = overlay.rgb_channel(index);
        for (value, over) in channel.iter_mut().zip(layer) {
            let blended = if *value < 0.5 { 2.0 * *value * over } else { 1.0 - 2.0 * (1.0 - *value) * (1.0 - over) };
            *value = *value * (1.0 - opacity) + blended * opacity;
        }
    }
    let resolution = overlay.resolution();
    Map::from_channels(resolution, channels)
}

/// The overlay at `resolution`, made again only when something changed.
fn overlay_for(job: &OverlayJob, resolution: Resolution) -> Option<Arc<Map>> {
    let mut made = MADE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(cached) = made.as_ref()
        && cached.job == *job
        && cached.resolution == resolution
    {
        return cached.map.clone();
    }
    let map = make_overlay(job, resolution).map(Arc::new);
    *made = Some(MadeOverlay { job: job.clone(), resolution, map: map.clone() });
    map
}

fn make_overlay(job: &OverlayJob, resolution: Resolution) -> Option<Map> {
    if job.settings.image == CUSTOM_OVERLAY {
        return custom_overlay(&job.settings.custom_file, job.portion, resolution);
    }
    Some(pattern_map(job.settings.image, job.portion, resolution))
}

/// `AssignOverlayFile` for Custom: the image, cut to the previewed region
/// and stretched to the render (`LinearFast`); nothing when it cannot be
/// read.
fn custom_overlay(file: &str, portion: Option<[f32; 3]>, resolution: Resolution) -> Option<Map> {
    if file.is_empty() {
        return None;
    }
    let loaded = nerthus_nodes::file_maps::load_map_file(Path::new(file), true, false).ok()?;
    let mut image = loaded;
    if let Some([left, top, side]) = portion {
        let width = image.width() as f32;
        let height = image.height() as f32;
        let x = (left * width).floor().clamp(0.0, width - 1.0) as usize;
        let y = (top * height).floor().clamp(0.0, height - 1.0) as usize;
        let cut_width = ((side * width).round() as usize).clamp(1, image.width() - x);
        let cut_height = ((side * height).round() as usize).clamp(1, image.height() - y);
        image = transformer::crop(&image, x, y, Resolution::new(cut_width, cut_height));
    }
    if image.resolution() != resolution {
        image = transformer::resize_with(&image, resolution, ResizeMethod::LinearFast);
    }
    Some(image)
}

/// One of Nerthus' patterns over the terrain (or the previewed part of
/// it), a sample in each pixel's middle.
fn pattern_map(image: usize, portion: Option<[f32; 3]>, resolution: Resolution) -> Map {
    let [left, top, side] = portion.unwrap_or([0.0, 0.0, 1.0]);
    let width = resolution.x.max(1);
    let height = resolution.y.max(1);
    let pixel = side / width as f32;
    let mut channels = vec![vec![0.0f32; width * height]; 3];
    for y in 0..height {
        let v = top + (y as f32 + 0.5) / height as f32 * side;
        for x in 0..width {
            let u = left + (x as f32 + 0.5) / width as f32 * side;
            let colour = pattern(image, u, v, pixel);
            let index = y * width + x;
            for (channel, value) in channels.iter_mut().zip(colour) {
                channel[index] = value;
            }
        }
    }
    Map::from_channels(Resolution::new(width, height), channels)
}

/// The pattern's colour at `(u, v)` (0 to 1 across the terrain, `v` down);
/// `pixel` is a pixel's size there, which lines are measured in.
fn pattern(image: usize, u: f32, v: f32, pixel: f32) -> [f32; 3] {
    let cells = Cells::of(u, v, 8.0);
    match image {
        0 => {
            // Colors1: every cell its own colour, outlined and labelled.
            let hue = (cells.column as f32 * 41.0 + cells.row as f32 * 23.0) % 360.0;
            let saturation = 0.45 + 0.05 * (cells.row % 3) as f32;
            let value = 0.5 + 0.04 * (cells.column % 4) as f32;
            let base = hsv(hue, saturation, value);
            let outlined = cells.outline(base, 1.5 * pixel * 8.0, 0.6);
            cells.label(outlined, [0.95; 3])
        }
        1 => {
            // Colors2: a colour a column, a finer grid over it.
            let base = hsv(cells.column as f32 * 45.0, 0.42, 0.66 - 0.025 * cells.row as f32);
            let fine = Cells::of(u, v, 32.0);
            let gridded = fine.outline(base, pixel * 32.0, 0.9);
            let outlined = cells.outline(gridded, 1.5 * pixel * 8.0, 0.75);
            cells.label(outlined, [0.18; 3])
        }
        2 => {
            // Dots: a grey checker, a coloured dot in each square.
            let grey = if cells.checker() { 0.30 } else { 0.36 };
            let hue = ((cells.column * 3 + cells.row * 5) % 8) as f32 * 45.0;
            cells.dot([grey; 3], 0.1, hsv(hue, 0.5, 0.72))
        }
        3 => {
            // Dark: a fine dark checker with faint dots.
            let fine = Cells::of(u, v, 32.0);
            let grey = if fine.checker() { 0.22 } else { 0.26 };
            fine.dot([grey; 3], 0.08, [0.4; 3])
        }
        4 => {
            // Light: a lighter checker with coloured dots.
            let fine = Cells::of(u, v, 16.0);
            let grey = if fine.checker() { 0.55 } else { 0.62 };
            let hue = ((fine.column + fine.row * 3) % 6) as f32 * 60.0;
            fine.dot([grey; 3], 0.11, hsv(hue, 0.45, 0.85))
        }
        5 => {
            // Lines: a blue-grey sheet ruled in three weights.
            let base = [0.64, 0.7, 0.72];
            let fine = Cells::of(u, v, 32.0);
            let quarters = Cells::of(u, v, 2.0);
            let ruled = fine.outline(base, pixel * 32.0, 0.92);
            let ruled = cells.outline(ruled, 1.5 * pixel * 8.0, 0.75);
            let ruled = quarters.outline(ruled, 2.5 * pixel * 2.0, 0.45);
            cells.label(ruled, [0.48, 0.53, 0.55])
        }
        6 => {
            // Quads: a colour a quarter, its cells checkered and labelled.
            let quarter = (cells.column / 4, cells.row / 4);
            let base = match quarter {
                (0, 0) => [0.82, 0.38, 0.28],
                (1, 0) => [0.38, 0.62, 0.52],
                (0, _) => [0.3, 0.52, 0.66],
                _ => [0.9, 0.8, 0.58],
            };
            let shade = if cells.checker() { 1.0 } else { 0.94 };
            let checkered = base.map(|channel| channel * shade);
            let outlined = cells.outline(checkered, 1.5 * pixel * 8.0, 0.85);
            cells.label(outlined, [0.97; 3])
        }
        _ => {
            // UV: red across, green up, labelled cells.
            let base = [u.clamp(0.0, 1.0), (1.0 - v).clamp(0.0, 1.0), 0.08];
            let outlined = cells.outline(base, 1.5 * pixel * 8.0, 0.8);
            let luminance = 0.3 * base[0] + 0.59 * base[1] + 0.11 * base[2];
            let ink = if luminance > 0.45 { [0.08; 3] } else { [0.92; 3] };
            cells.label(outlined, ink)
        }
    }
}

/// Where a point falls in a grid of `count` by `count` cells: the cell and
/// the point within it (0 to 1).
struct Cells {
    column: usize,
    row: usize,
    x: f32,
    y: f32,
}

impl Cells {
    fn of(u: f32, v: f32, count: f32) -> Self {
        let scaled_u = (u * count).clamp(0.0, count - 0.0001);
        let scaled_v = (v * count).clamp(0.0, count - 0.0001);
        Self { column: scaled_u as usize, row: scaled_v as usize, x: scaled_u.fract(), y: scaled_v.fract() }
    }

    fn checker(&self) -> bool {
        (self.column + self.row).is_multiple_of(2)
    }

    /// `colour` darkened to `shade` within `width` (in cells) of the
    /// cell's edges.
    fn outline(&self, colour: [f32; 3], width: f32, shade: f32) -> [f32; 3] {
        let edge = self.x.min(1.0 - self.x).min(self.y).min(1.0 - self.y);
        if edge < width / 2.0 { colour.map(|channel| channel * shade) } else { colour }
    }

    /// A dot of `radius` (in cells) in the cell's middle.
    fn dot(&self, colour: [f32; 3], radius: f32, dot: [f32; 3]) -> [f32; 3] {
        let dx = self.x - 0.5;
        let dy = self.y - 0.5;
        if dx * dx + dy * dy < radius * radius { dot } else { colour }
    }

    /// The cell's name (row letter, column number: A1 top left) in the
    /// middle of the cell.
    fn label(&self, colour: [f32; 3], ink: [f32; 3]) -> [f32; 3] {
        let letter = b'A' + self.row.min(7) as u8;
        let digit = b'1' + self.column.min(7) as u8;
        // Two glyphs of 5 by 7 dots, a dot apart, a dot being 1/40 cell.
        let dot = 1.0 / 40.0;
        let left = 0.5 - 5.5 * dot;
        let top = 0.5 - 3.5 * dot;
        let column = ((self.x - left) / dot).floor();
        let row = ((self.y - top) / dot).floor();
        if !(0.0..11.0).contains(&column) || !(0.0..7.0).contains(&row) {
            return colour;
        }
        let column = column as usize;
        let row = row as usize;
        let (glyph, glyph_column) = if column < 5 { (letter, column) } else if column > 5 { (digit, column - 6) } else { return colour };
        let rows = glyph_rows(glyph);
        let lit = rows[row] & (0b10000 >> glyph_column) != 0;
        if lit { ink } else { colour }
    }
}

/// Nerthus' 5 by 7 dot glyphs for the cell names.
fn glyph_rows(glyph: u8) -> [u8; 7] {
    match glyph {
        b'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        b'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        b'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        b'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        b'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        b'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        b'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111],
        b'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        b'1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        b'2' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
        b'3' => [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
        b'4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        b'5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        b'6' => [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        b'7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        _ => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
    }
}

/// A colour from hue (degrees), saturation and value.
fn hsv(hue: f32, saturation: f32, value: f32) -> [f32; 3] {
    let sector = (hue.rem_euclid(360.0)) / 60.0;
    let chroma = value * saturation;
    let second = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
    let floor = value - chroma;
    let (red, green, blue) = match sector as usize {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };
    [red + floor, green + floor, blue + floor]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn settings_round_trip_through_the_viewport() {
        let mut viewport = JsonMap::new();
        let settings = OverlaySettings { opacity: 3, image: 8, custom_file: "/tmp/grid.png".to_string() };
        settings.write_to(&mut viewport);
        assert_eq!(viewport.get("OverlayOpacity"), Some(&json!(3)));
        assert_eq!(OverlaySettings::from_state(Some(&viewport)), settings);
        OverlaySettings::default().write_to(&mut viewport);
        assert!(viewport.is_empty());
        assert_eq!(OverlaySettings::from_state(None).image, 3);
    }

    #[test]
    fn the_overlay_blend_mixes_by_opacity() {
        let render = Map::from_channels(Resolution::square(1), vec![vec![0.25], vec![0.75], vec![0.5]]);
        let job = OverlayJob { settings: OverlaySettings { opacity: 5, image: 7, custom_file: String::new() }, portion: None };
        let blended = apply(render, &job);
        // The UV pattern at the middle (a cell corner, outlined), fully
        // mixed in: below a half the layers multiply, above they screen.
        let over = pattern(7, 0.5, 0.5, 1.0);
        assert!((blended.channel(0)[0] - 2.0 * 0.25 * over[0]).abs() < 1e-6);
        assert!((blended.channel(1)[0] - (1.0 - 2.0 * 0.25 * (1.0 - over[1]))).abs() < 1e-6);
        assert!((blended.channel(2)[0] - (1.0 - 2.0 * 0.5 * (1.0 - over[2]))).abs() < 1e-6);
    }

    #[test]
    fn cells_are_named_from_the_top_left() {
        let cells = Cells::of(0.99, 0.01, 8.0);
        assert_eq!((cells.column, cells.row), (7, 0));
        // The middle of A1's "A": its crossbar row is lit at its middle.
        let a1 = Cells { column: 0, row: 0, x: 0.5 - 5.5 / 40.0 + 2.5 / 40.0, y: 0.5 - 3.5 / 40.0 + 3.5 / 40.0 };
        assert_eq!(a1.label([0.0; 3], [1.0; 3]), [1.0; 3]);
    }
}
