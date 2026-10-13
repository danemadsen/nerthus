//! The `ColorEditor`: Actipro's `SpectrumColorPicker` (a saturation and
//! brightness square beside a hue strip), a `ColorComponentSlider` with
//! its value for each of red, green and blue, and the colour as hex text.
//! Colours are sRGB bytes, as WPF's `Color.R/G/B`; projects keep them as
//! scRGB floats (`Color.ScR/ScG/ScB`), converted with WPF's own functions.

use egui::{Color32, Id, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use crate::theme;

/// The spectrum's height and the hue strip's width.
const SPECTRUM_HEIGHT: f32 = 140.0;
const HUE_WIDTH: f32 = 20.0;
/// A component row: the slider, then its value box (50 wide).
const ROW_HEIGHT: f32 = 24.0;
const VALUE_WIDTH: f32 = 50.0;

/// `Color.sRgbToScRgb`.
pub fn srgb_to_linear(byte: u8) -> f32 {
    let value = byte as f32 / 255.0;
    if value.is_nan() || value <= 0.0 {
        0.0
    } else if value <= 0.04045 {
        value / 12.92
    } else if value < 1.0 {
        (((value as f64) + 0.055) / 1.055).powf(2.4) as f32
    } else {
        1.0
    }
}

/// `Color.ScRgbTosRgb`.
pub fn linear_to_srgb(value: f32) -> u8 {
    if value.is_nan() || value <= 0.0 {
        0
    } else if value <= 0.0031308 {
        ((255.0 * value * 12.92) + 0.5) as u8
    } else if value < 1.0 {
        ((255.0 * ((1.055 * (value as f64).powf(1.0 / 2.4) as f32) - 0.055)) + 0.5) as u8
    } else {
        255
    }
}

/// HSV (hue in degrees) of an sRGB colour, as the spectrum shows it.
fn to_hsv(rgb: [u8; 3]) -> [f32; 3] {
    let [red, green, blue] = rgb.map(|byte| byte as f32 / 255.0);
    let maximum = red.max(green).max(blue);
    let minimum = red.min(green).min(blue);
    let delta = maximum - minimum;
    let hue = if delta == 0.0 {
        0.0
    } else if maximum == red {
        60.0 * ((green - blue) / delta).rem_euclid(6.0)
    } else if maximum == green {
        60.0 * ((blue - red) / delta + 2.0)
    } else {
        60.0 * ((red - green) / delta + 4.0)
    };
    let saturation = if maximum == 0.0 { 0.0 } else { delta / maximum };
    [hue, saturation, maximum]
}

fn from_hsv(hsv: [f32; 3]) -> [u8; 3] {
    let [hue, saturation, value] = hsv;
    let chroma = value * saturation;
    let sector = (hue / 60.0).rem_euclid(6.0);
    let second = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
    let (red, green, blue) = match sector as u32 {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };
    let offset = value - chroma;
    [red, green, blue].map(|component| ((component + offset) * 255.0).round().clamp(0.0, 255.0) as u8)
}

/// What the picker keeps between frames: the hue and saturation while
/// they cannot be read back from the colour (black, greys), and the text
/// being typed.
#[derive(Clone, Default)]
struct PickerState {
    hsv: Option<[f32; 3]>,
    /// The colour `hsv` was taken from.
    shown: [u8; 3],
    hex: Option<String>,
    components: [Option<String>; 3],
}

/// The picker for one colour; returns the colour when changed.
pub fn colour_picker(ui: &mut Ui, id: Id, rgb: [u8; 3]) -> Option<[u8; 3]> {
    let mut state: PickerState = ui.data(|data| data.get_temp(id)).unwrap_or_default();
    if state.hsv.is_none() || state.shown != rgb {
        let mut hsv = to_hsv(rgb);
        // Keep the hue (and saturation) the colour no longer says.
        if let Some(previous) = state.hsv {
            if hsv[1] == 0.0 || hsv[2] == 0.0 {
                hsv[0] = previous[0];
            }
            if hsv[2] == 0.0 {
                hsv[1] = previous[1];
            }
        }
        state.hsv = Some(hsv);
        state.shown = rgb;
    }
    let mut hsv = state.hsv.unwrap_or_default();
    let mut changed = None;

    // `SpectrumColorPicker`.
    let width = ui.available_width();
    let (area, _) = ui.allocate_exact_size(Vec2::new(width, SPECTRUM_HEIGHT), Sense::hover());
    let square = Rect::from_min_max(area.min, Pos2::new(area.right() - HUE_WIDTH - 8.0, area.bottom()));
    let hue_strip = Rect::from_min_max(Pos2::new(area.right() - HUE_WIDTH, area.top()), area.max);
    paint_square(ui, square, hsv[0]);
    paint_hue_strip(ui, hue_strip);
    let square_response = ui.interact(square, id.with("square"), Sense::click_and_drag());
    if let Some(pointer) = square_response.interact_pointer_pos()
        && (square_response.dragged() || square_response.clicked() || square_response.is_pointer_button_down_on())
    {
        hsv[1] = ((pointer.x - square.left()) / square.width()).clamp(0.0, 1.0);
        hsv[2] = 1.0 - ((pointer.y - square.top()) / square.height()).clamp(0.0, 1.0);
        changed = Some(from_hsv(hsv));
    }
    let hue_response = ui.interact(hue_strip, id.with("hue"), Sense::click_and_drag());
    if let Some(pointer) = hue_response.interact_pointer_pos()
        && (hue_response.dragged() || hue_response.clicked() || hue_response.is_pointer_button_down_on())
    {
        hsv[0] = (((pointer.y - hue_strip.top()) / hue_strip.height()).clamp(0.0, 1.0) * 360.0).min(359.9);
        changed = Some(from_hsv(hsv));
    }
    // The markers: a ring at the colour, a bar at the hue.
    let marker = Pos2::new(square.left() + hsv[1] * square.width(), square.top() + (1.0 - hsv[2]) * square.height());
    let ring_colour = if hsv[2] > 0.5 { Color32::BLACK } else { Color32::WHITE };
    ui.painter().circle_stroke(marker, 5.0, Stroke::new(1.5, ring_colour));
    let hue_y = hue_strip.top() + hsv[0] / 360.0 * hue_strip.height();
    let hue_marker = Rect::from_center_size(Pos2::new(hue_strip.center().x, hue_y), Vec2::new(hue_strip.width() + 4.0, 3.0));
    ui.painter().rect_stroke(hue_marker, 1.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Middle);
    ui.add_space(8.0);

    // `GetSlider` for R, G and B.
    let names = ["R", "G", "B"];
    let accents = [theme::ACCENT_RED, theme::ACCENT_GREEN, theme::ACCENT_BLUE];
    let current = changed.unwrap_or(rgb);
    for component in 0..3 {
        let edited = component_row(ui, id.with(names[component]), current, component, accents[component], &mut state.components[component]);
        if let Some(value) = edited {
            let mut updated = changed.unwrap_or(rgb);
            updated[component] = value;
            changed = Some(updated);
        }
    }
    ui.add_space(4.0);

    // The colour as `#RRGGBB` (monospace 14, `Accent-Primary`).
    let shown_colour = changed.unwrap_or(rgb);
    let hex_text = format!("#{:02X}{:02X}{:02X}", shown_colour[0], shown_colour[1], shown_colour[2]);
    let mut hex = state.hex.clone().unwrap_or(hex_text.clone());
    let mut field = egui::TextEdit::singleline(&mut hex);
    field = field.font(theme::monospace(14.0));
    field = field.text_color(theme::ACCENT_PRIMARY);
    field = field.desired_width(f32::INFINITY);
    let response = ui.add(field);
    if response.has_focus() {
        state.hex = Some(hex.clone());
    } else {
        state.hex = None;
    }
    if response.lost_focus()
        && let Some(parsed) = parse_hex(&hex)
    {
        changed = Some(parsed);
    }

    if let Some(new_colour) = changed {
        let mut new_hsv = to_hsv(new_colour);
        if new_hsv[1] == 0.0 || new_hsv[2] == 0.0 {
            new_hsv[0] = hsv[0];
        }
        if new_hsv[2] == 0.0 {
            new_hsv[1] = hsv[1];
        }
        state.hsv = Some(new_hsv);
        state.shown = new_colour;
    }
    ui.data_mut(|data| data.insert_temp(id, state));
    changed.filter(|new_colour| *new_colour != rgb)
}

/// A component's slider (its colour's range for that component) and its
/// value box.
fn component_row(ui: &mut Ui, id: Id, rgb: [u8; 3], component: usize, accent: Color32, typing: &mut Option<String>) -> Option<u8> {
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(Vec2::new(width, ROW_HEIGHT), Sense::hover());
    let slider = Rect::from_min_max(Pos2::new(row.left(), row.top() + 6.0), Pos2::new(row.right() - VALUE_WIDTH - 8.0, row.bottom() - 6.0));
    let steps = 32;
    for step in 0..steps {
        let from = step as f32 / steps as f32;
        let to = (step + 1) as f32 / steps as f32;
        let mut colour = rgb;
        colour[component] = (from * 255.0).round() as u8;
        let cell = Rect::from_min_max(Pos2::new(slider.left() + from * slider.width(), slider.top()), Pos2::new(slider.left() + to * slider.width() + 0.5, slider.bottom()));
        ui.painter().rect_filled(cell, 0.0, Color32::from_rgb(colour[0], colour[1], colour[2]));
    }
    ui.painter().rect_stroke(slider, 0.0, Stroke::new(1.0, theme::BUTTON_BORDER), egui::StrokeKind::Outside);
    let thumb_x = slider.left() + rgb[component] as f32 / 255.0 * slider.width();
    let thumb = Rect::from_center_size(Pos2::new(thumb_x, slider.center().y), Vec2::new(4.0, slider.height() + 6.0));
    ui.painter().rect_filled(thumb, 1.0, theme::LABEL_EMPHASIS);
    let response = ui.interact(slider.expand2(Vec2::new(0.0, 4.0)), id.with("slider"), Sense::click_and_drag());
    let mut edited = None;
    if let Some(pointer) = response.interact_pointer_pos()
        && (response.dragged() || response.clicked() || response.is_pointer_button_down_on())
    {
        let fraction = ((pointer.x - slider.left()) / slider.width()).clamp(0.0, 1.0);
        edited = Some((fraction * 255.0).round() as u8);
    }
    let value_rect = Rect::from_min_max(Pos2::new(row.right() - VALUE_WIDTH, row.top() + 1.0), Pos2::new(row.right(), row.bottom() - 1.0));
    let mut text = typing.clone().unwrap_or_else(|| rgb[component].to_string());
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(value_rect));
    let mut field = egui::TextEdit::singleline(&mut text);
    field = field.font(theme::monospace(12.0));
    field = field.text_color(accent);
    field = field.desired_width(VALUE_WIDTH - 8.0);
    let field_response = child.add(field);
    if field_response.has_focus() {
        *typing = Some(text.clone());
    } else {
        *typing = None;
    }
    if field_response.lost_focus()
        && let Ok(value) = text.trim().parse::<u8>()
    {
        edited = Some(value);
    }
    edited
}

/// `#RRGGBB`, `RRGGBB` or `#AARRGGBB` (the alpha dropped).
fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let digits = text.trim().trim_start_matches('#');
    let digits = match digits.len() {
        6 => digits,
        8 => &digits[2..],
        _ => return None,
    };
    let component = |index: usize| u8::from_str_radix(&digits[index * 2..index * 2 + 2], 16).ok();
    Some([component(0)?, component(1)?, component(2)?])
}

/// The saturation (left to right) and brightness (top to bottom) square at
/// a hue.
fn paint_square(ui: &Ui, rect: Rect, hue: f32) {
    let columns = 24;
    let rows = 16;
    let mut mesh = egui::Mesh::default();
    for row in 0..=rows {
        for column in 0..=columns {
            let saturation = column as f32 / columns as f32;
            let value = 1.0 - row as f32 / rows as f32;
            let [red, green, blue] = from_hsv([hue, saturation, value]);
            let position = Pos2::new(rect.left() + saturation * rect.width(), rect.top() + (1.0 - value) * rect.height());
            mesh.colored_vertex(position, Color32::from_rgb(red, green, blue));
        }
    }
    let stride = columns + 1;
    for row in 0..rows {
        for column in 0..columns {
            let corner = (row * stride + column) as u32;
            let below = corner + stride as u32;
            mesh.add_triangle(corner, corner + 1, below);
            mesh.add_triangle(corner + 1, below + 1, below);
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, theme::BUTTON_BORDER), egui::StrokeKind::Outside);
}

/// The hue strip, red at the top round to red at the bottom.
fn paint_hue_strip(ui: &Ui, rect: Rect) {
    let steps = 36;
    let mut mesh = egui::Mesh::default();
    for step in 0..=steps {
        let fraction = step as f32 / steps as f32;
        let [red, green, blue] = from_hsv([fraction * 360.0 % 360.0, 1.0, 1.0]);
        let colour = Color32::from_rgb(red, green, blue);
        let y = rect.top() + fraction * rect.height();
        mesh.colored_vertex(Pos2::new(rect.left(), y), colour);
        mesh.colored_vertex(Pos2::new(rect.right(), y), colour);
    }
    for step in 0..steps {
        let top = (step * 2) as u32;
        mesh.add_triangle(top, top + 1, top + 2);
        mesh.add_triangle(top + 1, top + 3, top + 2);
    }
    ui.painter().add(egui::Shape::mesh(mesh));
    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, theme::BUTTON_BORDER), egui::StrokeKind::Outside);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wpf_colour_conversions_round_trip() {
        for byte in 0..=255u8 {
            assert_eq!(linear_to_srgb(srgb_to_linear(byte)), byte);
        }
        assert_eq!(srgb_to_linear(255), 1.0);
        assert_eq!(linear_to_srgb(0.5), 188);
    }

    #[test]
    fn hsv_round_trips_and_hex_parses() {
        for colour in [[255, 0, 0], [12, 200, 99], [0, 0, 0], [128, 128, 128]] {
            assert_eq!(from_hsv(to_hsv(colour)), colour);
        }
        assert_eq!(parse_hex("#FF8000"), Some([255, 128, 0]));
        assert_eq!(parse_hex("ccff8000"), Some([255, 128, 0]));
        assert_eq!(parse_hex("#F80"), None);
    }
}
