//! The painted strokes (`VectorMask`): the Draw node's brush strokes,
//! saved as `StrokeData` and rasterized the way they are re-rendered on
//! load.

use std::io::Read;

use base64::Engine as _;
use flate2::read::GzDecoder;
use rayon::prelude::*;
use serde_json::Value;

use crate::mathv;
use crate::x86;

/// Lanes painted at once (`Vector<float>.Count` on AVX2).
const LANES: usize = 8;

/// `VectorMask.Stroke.Brush`.
#[derive(Clone, Copy, Debug)]
pub struct Brush {
    pub intensity: f32,
    pub radius: f32,
    pub hardness: f32,
}

/// One stroke: its points (0..1 of the map) and a brush per point.
#[derive(Clone, Debug, Default)]
pub struct Stroke {
    pub points: Vec<[f32; 2]>,
    pub brushes: Vec<Brush>,
}

/// The cubic smoothstep `x²(3 − 2x)`.
fn smoothstep(x: f32) -> f32 {
    x * x * (3.0 + x * -2.0)
}

/// `MathV.Clamp(x, 0, 1)`.
fn saturate(x: f32) -> f32 {
    mathv::min(mathv::max(x, 0.0), 1.0)
}

/// `SegmentContext`: one segment in pixels.
struct Segment {
    start_x: f32,
    start_y: f32,
    start_radius: f32,
    end_radius: f32,
    start_hardness: f32,
    end_hardness: f32,
    start_intensity: f32,
    end_intensity: f32,
    length: f32,
    length_inverse: f32,
    direction_x: f32,
    direction_y: f32,
    resolution: f32,
}

impl Segment {
    /// The brush footprint at a pixel: (falloff, intensity, distance from
    /// the segment start) for the closest point on the segment.
    fn footprint(&self, x: f32, y: f32) -> (f32, f32, f32) {
        let offset_x = x - self.start_x;
        let offset_y = y - self.start_y;
        let mut along = self.direction_x * offset_x + self.direction_y * offset_y;
        along = mathv::min(along, self.length);
        along = mathv::max(along, 0.0);
        let closest_x = self.start_x + self.direction_x * along;
        let closest_y = self.start_y + self.direction_y * along;
        let share = along * self.length_inverse;
        let radius = mathv::mix(self.start_radius, self.end_radius, share) * self.resolution;
        let to_closest_x = closest_x - x;
        let to_closest_y = closest_y - y;
        let distance = (to_closest_x * to_closest_x + to_closest_y * to_closest_y).sqrt();
        let start_distance = (offset_x * offset_x + offset_y * offset_y).sqrt();
        let hardness = mathv::mix(self.start_hardness, self.end_hardness, share);
        let intensity = mathv::mix(self.start_intensity, self.end_intensity, share);
        let inner = hardness * radius;
        let falloff = smoothstep(saturate((distance - inner) / (radius - inner)));
        (falloff, intensity, start_distance)
    }

    /// `Stroke.DrawFirst`: the stroke's first segment over the map.
    fn draw_first(&self, value: f32, x: f32, y: f32) -> f32 {
        let (falloff, intensity, _) = self.footprint(x, y);
        mathv::mix(intensity, value, falloff)
    }

    /// `Stroke.Draw`: a later segment, which first takes back the start
    /// cap the previous segment already painted.
    fn draw(&self, value: f32, x: f32, y: f32) -> f32 {
        let (falloff, intensity, start_distance) = self.footprint(x, y);
        let cap_radius = self.start_radius * self.resolution;
        let cap_inner = self.start_hardness * cap_radius;
        let cap = 1.0 - smoothstep(saturate((start_distance - cap_inner) / (cap_radius - cap_inner)));
        let mut underneath = (value - self.start_intensity * cap) / (1.0 - cap + 1e-26);
        underneath = mathv::max(underneath, 0.0);
        underneath = mathv::min(underneath, 1.0);
        mathv::mix(intensity, underneath, falloff)
    }
}

impl Stroke {
    /// `Stroke.RenderSegment` over the whole map, with the viewport at its
    /// default (the unit square).
    fn render_segment(&self, end_index: usize, first: bool, data: &mut [f32], size: usize) {
        let start_index = end_index.saturating_sub(1);
        let start = self.points[start_index];
        let end = self.points[end_index];
        let start_brush = self.brushes[start_index.min(self.brushes.len() - 1)];
        let end_brush = self.brushes[end_index.min(self.brushes.len() - 1)];

        let low = [
            mathv::min(start[0] - start_brush.radius, end[0] - end_brush.radius),
            mathv::min(start[1] - start_brush.radius, end[1] - end_brush.radius),
        ];
        let high = [
            mathv::max(start[0] + start_brush.radius, end[0] + end_brush.radius),
            mathv::max(start[1] + start_brush.radius, end[1] + end_brush.radius),
        ];
        let scale = size as f32;
        let last = size as i32 - 1;
        let top = x86::cvttss2si(low[1] * scale);
        let bottom = x86::cvttss2si(high[1] * scale);
        if top > last || bottom < 0 {
            return;
        }
        let left = x86::cvttss2si(low[0] * scale).clamp(0, last) as usize;
        let right = x86::cvttss2si(high[0] * scale).clamp(0, last) as usize;
        let top = top.clamp(0, last) as usize;
        let bottom = bottom.clamp(0, last) as usize;

        let mut direction = [end[0] - start[0], end[1] - start[1]];
        let length = (direction[0] * direction[0] + direction[1] * direction[1]).sqrt();
        let mut length_inverse = 1.0;
        if length > 0.0 {
            length_inverse = 1.0 / length;
            direction = [direction[0] / length, direction[1] / length];
        }
        let segment = Segment {
            start_x: start[0] * scale,
            start_y: start[1] * scale,
            start_radius: start_brush.radius,
            end_radius: end_brush.radius,
            start_hardness: start_brush.hardness,
            end_hardness: end_brush.hardness,
            start_intensity: start_brush.intensity,
            end_intensity: end_brush.intensity,
            length: length * scale,
            length_inverse: length_inverse / scale,
            direction_x: direction[0],
            direction_y: direction[1],
            resolution: scale,
        };

        // Whole vectors from `left` while they fit, then the leftover
        // pixels up to `right`; whole vectors may paint past `right`.
        let overhang = if right + LANES > size { (right + LANES) % size } else { 0 };
        let vector_end = if overhang > 0 { right - overhang + 1 } else { right + 1 };
        let rows = data[top * size..(bottom + 1) * size].par_chunks_mut(size).enumerate();
        rows.for_each(|(offset, row)| {
            let y = (top + offset) as f32;
            let paint = |row: &mut [f32], column: usize| {
                let x = column as f32;
                row[column] = if first { segment.draw_first(row[column], x, y) } else { segment.draw(row[column], x, y) };
            };
            let mut column = left;
            while column < vector_end {
                for lane in column..column + LANES {
                    paint(row, lane);
                }
                column += LANES;
            }
            for leftover in column..=right {
                paint(row, leftover);
            }
        });
    }

    /// `Stroke.RenderTo`: the first segment, then every later one but the
    /// last (the final segment is skipped when saved strokes are
    /// re-rendered).
    fn render(&self, data: &mut [f32], size: usize) {
        if self.points.is_empty() {
            return;
        }
        let first_end = 1.min(self.points.len() - 1);
        self.render_segment(first_end, true, data, size);
        for end_index in 2..self.points.len().saturating_sub(1) {
            self.render_segment(end_index, false, data, size);
        }
    }
}

/// `Stroke.AddPoint(position, map)` as the Painter paints live: the
/// segment ending at the stroke's point `end_index` (a dot for the first
/// point, `DrawFirst`; later segments `Draw` over what is there).
pub fn paint_segment(stroke: &Stroke, end_index: usize, data: &mut [f32], size: usize) {
    if end_index >= stroke.points.len() || stroke.brushes.is_empty() {
        return;
    }
    stroke.render_segment(end_index, end_index == 0, data, size);
}

/// `VectorMask.RenderTo` onto a cleared map.
pub fn render(strokes: &[Stroke], size: usize) -> Vec<f32> {
    let mut data = vec![0.0f32; size * size];
    for stroke in strokes {
        stroke.render(&mut data, size);
    }
    data
}

/// Writes `Draw.StrokeData` as `Draw.OnSaving` does: each stroke as a
/// `Stroke` (`Points`, then its first brush's `Hardness`, `Radius` and
/// `Intensity`; zeros left out), the list compressed (the UTF-8 length,
/// little-endian, then gzip, all in base64).
pub fn stroke_data(strokes: &[Stroke]) -> String {
    let mut json = String::from("[");
    for (index, stroke) in strokes.iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        let brush = stroke.brushes.first().copied().unwrap_or(Brush { intensity: 1.0, radius: 0.01, hardness: 0.5 });
        let mut members: Vec<String> = Vec::new();
        let points: Vec<String> = stroke
            .points
            .iter()
            .map(|[x, y]| {
                let mut coordinates = Vec::new();
                if *x != 0.0 {
                    coordinates.push(format!("\"X\":{}", newtonsoft_float(*x)));
                }
                if *y != 0.0 {
                    coordinates.push(format!("\"Y\":{}", newtonsoft_float(*y)));
                }
                format!("{{{}}}", coordinates.join(","))
            })
            .collect();
        members.push(format!("\"Points\":[{}]", points.join(",")));
        for (name, value) in [("Hardness", brush.hardness), ("Radius", brush.radius), ("Intensity", brush.intensity)] {
            if value != 0.0 {
                members.push(format!("\"{name}\":{}", newtonsoft_float(value)));
            }
        }
        json.push('{');
        json.push_str(&members.join(","));
        json.push('}');
    }
    json.push(']');
    compress_text(&json)
}

fn compress_text(text: &str) -> String {
    use std::io::Write;
    let bytes = text.as_bytes();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = encoder.write_all(bytes);
    let compressed = encoder.finish().unwrap_or_default();
    let mut framed = Vec::with_capacity(compressed.len() + 4);
    framed.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    framed.extend_from_slice(&compressed);
    base64::engine::general_purpose::STANDARD.encode(framed)
}

/// `JsonConvert.ToString(float)`.
fn newtonsoft_float(value: f32) -> String {
    let text = nerthus_engine::automation::dotnet_float_string(value);
    if text.contains('.') || text.contains('E') || !value.is_finite() {
        return text;
    }
    format!("{text}.0")
}

/// Reads `Draw.StrokeData`: base64 of a little-endian length followed by
/// gzipped JSON, a list of `{Points: [{X, Y}…], Hardness, Radius,
/// Intensity}` (members at their default are left out).
pub fn parse_stroke_data(text: &str) -> Result<Vec<Stroke>, String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(text.trim()).map_err(|error| error.to_string())?;
    if bytes.len() < 4 {
        return Err("stroke data is too short".to_string());
    }
    let mut json = String::new();
    GzDecoder::new(&bytes[4..]).read_to_string(&mut json).map_err(|error| error.to_string())?;
    let items: Vec<Value> = serde_json::from_str(&json).map_err(|error| error.to_string())?;

    let number = |value: &Value, key: &str| value.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let strokes = items
        .iter()
        .map(|item| {
            let brush = Brush {
                intensity: number(item, "Intensity"),
                radius: number(item, "Radius"),
                hardness: number(item, "Hardness"),
            };
            let points: Vec<[f32; 2]> = item
                .get("Points")
                .and_then(Value::as_array)
                .map(|points| points.iter().map(|point| [number(point, "X"), number(point, "Y")]).collect())
                .unwrap_or_default();
            // `Stroke.AddPoint` keeps one brush per point.
            let brushes = if points.is_empty() { vec![brush] } else { vec![brush; points.len()] };
            Stroke { points, brushes }
        })
        .collect();
    Ok(strokes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stroke_data_round_trips() {
        let brush = Brush { intensity: 0.5, radius: 0.025, hardness: 1.0 };
        let stroke = Stroke { points: vec![[0.25, 0.5], [0.0, 0.75]], brushes: vec![brush] };
        let text = stroke_data(std::slice::from_ref(&stroke));
        let read = parse_stroke_data(&text).expect("reads back");
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].points, stroke.points);
        assert_eq!(read[0].brushes[0].radius, 0.025);
        assert_eq!(read[0].brushes[0].hardness, 1.0);
    }
}
