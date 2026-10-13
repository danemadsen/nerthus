//! Reading a heightfield as land rather than as a file: its relief in
//! metres, how much of it is gentle enough to build on, whether the edges
//! stand above the middle, how it splits into low, middle and high
//! ground, and its profile along each axis. From a node's output in the
//! editor or a file a build wrote.

use std::path::Path;

use nerthus_engine::Map;
use serde_json::{json, Value};

/// How many points each profile is cut down to.
const PROFILE_POINTS: usize = 16;

/// Heights as fractions of the terrain's full height (0 to 1 in range),
/// row by row as the 2D view shows them.
pub(crate) struct Heightfield {
    pub width: usize,
    pub height: usize,
    pub samples: Vec<f32>,
}

/// How a raw file's samples are stored when its extension leaves it
/// open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RawEncoding {
    Ushort,
    Half,
    Float,
}

impl RawEncoding {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "ushort" | "uint16" | "u16" => Some(Self::Ushort),
            "half" | "f16" | "float16" => Some(Self::Half),
            "float" | "f32" | "float32" => Some(Self::Float),
            _ => None,
        }
    }
}

/// The side of a square of `count` samples, if they make one.
fn square_side(count: usize) -> Option<usize> {
    let side = (count as f64).sqrt().round() as usize;
    (side > 0 && side * side == count).then_some(side)
}

fn square(samples: Vec<f32>) -> Result<Heightfield, String> {
    let side = square_side(samples.len()).ok_or_else(|| format!("{} samples do not make a square heightfield.", samples.len()))?;
    Ok(Heightfield { width: side, height: side, samples })
}

impl Heightfield {
    /// A map's first channel.
    pub fn of_map(map: &Map) -> Self {
        Self { width: map.width(), height: map.height(), samples: map.channel(0).to_vec() }
    }

    /// A heightfield file: raw heights (`.r16`, `.raw`, `.r32`, `.graw`)
    /// or an image Nerthus reads (PNG, TIFF, EXR, …), its first channel.
    /// `encoding` says how a `.raw` or `.r16` file is stored when it is not
    /// 16-bit whole numbers.
    pub fn load(path: &Path, encoding: Option<RawEncoding>) -> Result<Self, String> {
        let extension = path.extension().map(|extension| extension.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let raw = || std::fs::read(path).map_err(|error| format!("{} cannot be read: {error}", path.display()));
        let ushorts = |bytes: &[u8]| -> Vec<f32> { bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair) as f32 / u16::MAX as f32).collect() };
        let halves = |bytes: &[u8]| -> Vec<f32> { bytes.as_chunks::<2>().0.iter().map(|pair| half::f16::from_le_bytes(*pair).to_f32()).collect() };
        let floats = |bytes: &[u8]| -> Vec<f32> { bytes.as_chunks::<4>().0.iter().map(|word| f32::from_le_bytes(*word)).collect() };
        match extension.as_str() {
            "r16" | "raw" => {
                let bytes = raw()?;
                let as_shorts = square_side(bytes.len() / 2).is_some() && bytes.len() % 2 == 0;
                let as_floats = square_side(bytes.len() / 4).is_some() && bytes.len() % 4 == 0;
                let encoding = encoding.unwrap_or(if !as_shorts && as_floats { RawEncoding::Float } else { RawEncoding::Ushort });
                match encoding {
                    RawEncoding::Ushort => square(ushorts(&bytes)),
                    RawEncoding::Half => square(halves(&bytes)),
                    RawEncoding::Float => square(floats(&bytes)),
                }
            }
            "r32" => square(floats(&raw()?)),
            "graw" => {
                // Floats, one plane per channel; the `.meta` beside it says
                // how many.
                let bytes = raw()?;
                let all = floats(&bytes);
                let meta = std::fs::read_to_string(path.with_extension("meta")).ok().and_then(|text| serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')).ok());
                let channels = meta.as_ref().and_then(|meta| meta.get("Channels")).and_then(|value| value.as_str().and_then(|text| text.parse().ok()).or_else(|| value.as_u64())).unwrap_or(1).max(1) as usize;
                let plane = all.len() / channels;
                square(all[..plane].to_vec())
            }
            _ => {
                let image = nerthus_io::image_file::decode_image(path).map_err(|error| format!("{} cannot be read: {error:#}", path.display()))?;
                let channels = image.channels.max(1);
                let samples = image.data.chunks_exact(channels).map(|pixel| pixel[0]).collect();
                Ok(Self { width: image.width, height: image.height, samples })
            }
        }
    }

    /// The report: `metres_across` the terrain's width, `metres_up` the
    /// height a 1 stands for, `gentle_degrees` the steepest ground counted
    /// as gentle, `centre_fraction` the share of each side taken as the
    /// middle (the rest is the frame).
    pub fn report(&self, metres_across: f64, metres_up: f64, gentle_degrees: f64, centre_fraction: f64) -> Value {
        let (width, height) = (self.width, self.height);
        let finite = |value: f32| if value.is_finite() { value as f64 } else { 0.0 };
        let heights: Vec<f64> = self.samples.iter().map(|value| finite(*value) * metres_up).collect();
        let at = |row: usize, column: usize| heights[row * width + column];
        let step = metres_across / width.max(1) as f64;

        let low = heights.iter().copied().fold(f64::INFINITY, f64::min);
        let high = heights.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let relief = (high - low).max(0.0);
        let flat = relief <= metres_up * 1e-6;

        let margin_x = ((1.0 - centre_fraction.clamp(0.1, 1.0)) / 2.0 * width as f64) as usize;
        let margin_y = ((1.0 - centre_fraction.clamp(0.1, 1.0)) / 2.0 * height as f64) as usize;
        let third = relief / 3.0;
        let mut classes = [0usize; 3];
        let (mut centre, mut centre_gentle, mut gentle) = (0usize, 0usize, 0usize);
        let (mut centre_slope, mut centre_height, mut frame_height, mut frame) = (0.0, 0.0, 0.0, 0usize);
        for row in 0..height {
            for column in 0..width {
                let here = at(row, column);
                let across = if column + 1 < width { at(row, column + 1) - here } else { 0.0 };
                let down = if row + 1 < height { at(row + 1, column) - here } else { 0.0 };
                let slope = (across.hypot(down) / step).atan().to_degrees();
                let is_gentle = slope <= gentle_degrees;
                gentle += usize::from(is_gentle);
                let inside = (margin_y..height - margin_y).contains(&row) && (margin_x..width - margin_x).contains(&column);
                if inside {
                    centre += 1;
                    centre_gentle += usize::from(is_gentle);
                    centre_slope += slope;
                    centre_height += here;
                } else {
                    frame += 1;
                    frame_height += here;
                }
                let class = if flat {
                    1
                } else if here < low + third {
                    0
                } else if here < low + 2.0 * third {
                    1
                } else {
                    2
                };
                classes[class] += 1;
            }
        }
        let percent = |part: usize, whole: usize| if whole == 0 { 0.0 } else { round(part as f64 / whole as f64 * 100.0, 2) };
        let total = width * height;
        let edge_lift = if frame == 0 || centre == 0 { 0.0 } else { frame_height / frame as f64 - centre_height / centre as f64 };
        let rows: Vec<f64> = (0..height).map(|row| (0..width).map(|column| at(row, column)).sum::<f64>() / width.max(1) as f64).collect();
        let columns: Vec<f64> = (0..width).map(|column| (0..height).map(|row| at(row, column)).sum::<f64>() / height.max(1) as f64).collect();
        let shape = if flat {
            "flat: no relief at all"
        } else if edge_lift > metres_up * 0.05 {
            "a basin: the edges stand above the middle"
        } else if edge_lift < -metres_up * 0.05 {
            "a mound: the middle stands above the edges"
        } else {
            "even: little difference between the middle and the edges"
        };
        json!({
            "resolution": [width, height],
            "metres_across": metres_across,
            "metres_up": metres_up,
            "lowest_metres": round(if low.is_finite() { low } else { 0.0 }, 1),
            "highest_metres": round(if high.is_finite() { high } else { 0.0 }, 1),
            "relief_metres": round(relief, 1),
            "fills_range_percent": round(relief / metres_up.max(f64::MIN_POSITIVE) * 100.0, 2),
            "mean_metres": round(heights.iter().sum::<f64>() / total.max(1) as f64, 1),
            "is_flat": flat,
            "gentle_degrees": gentle_degrees,
            "centre_gentle_percent": percent(centre_gentle, centre),
            "overall_gentle_percent": percent(gentle, total),
            "centre_mean_slope_degrees": round(if centre == 0 { 0.0 } else { centre_slope / centre as f64 }, 2),
            "edge_lift_metres": round(edge_lift, 1),
            "height_classes": {
                "low_percent": percent(classes[0], total),
                "middle_percent": percent(classes[1], total),
                "high_percent": percent(classes[2], total),
            },
            "profile_top_to_bottom_metres": profile(&rows),
            "profile_left_to_right_metres": profile(&columns),
            "shape": shape,
        })
    }
}

fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

/// A profile cut down to [`PROFILE_POINTS`] means.
fn profile(values: &[f64]) -> Vec<f64> {
    if values.is_empty() {
        return Vec::new();
    }
    let per = values.len().div_ceil(PROFILE_POINTS);
    values.chunks(per).map(|chunk| round(chunk.iter().sum::<f64>() / chunk.len() as f64, 1)).collect()
}

/// Whether a file looks like a heightfield a build wrote.
pub(crate) fn is_heightfield_file(path: &Path) -> bool {
    let extension = path.extension().map(|extension| extension.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    matches!(extension.as_str(), "r16" | "raw" | "r32" | "graw" | "png" | "tif" | "tiff" | "exr")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(size: usize, height: impl Fn(usize, usize) -> f32) -> Heightfield {
        let samples = (0..size * size).map(|index| height(index / size, index % size)).collect();
        Heightfield { width: size, height: size, samples }
    }

    #[test]
    fn a_flat_field_says_so() {
        let report = field(32, |_, _| 0.5).report(5000.0, 2500.0, 5.0, 0.6);
        assert_eq!(report["is_flat"], json!(true));
        assert_eq!(report["centre_gentle_percent"], json!(100.0));
        assert_eq!(report["relief_metres"], json!(0.0));
    }

    #[test]
    fn a_basin_has_raised_edges_and_a_mound_a_raised_middle() {
        let basin = field(64, |row, column| {
            let across = (row as f32 - 31.5).abs() / 31.5;
            let down = (column as f32 - 31.5).abs() / 31.5;
            across.max(down)
        });
        let report = basin.report(5000.0, 1000.0, 5.0, 0.6);
        assert!(report["edge_lift_metres"].as_f64().unwrap() > 0.0);
        assert!(report["shape"].as_str().unwrap().starts_with("a basin"));
        let mound = field(64, |row, column| 1.0 - (row as f32 - 31.5).hypot(column as f32 - 31.5) / 45.0);
        assert!(mound.report(5000.0, 1000.0, 5.0, 0.6)["edge_lift_metres"].as_f64().unwrap() < 0.0);
    }

    #[test]
    fn raw_files_read_as_their_size_says() {
        let folder = std::env::temp_dir().join(format!("nerthus-terrain-report-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        // 4 × 4 16-bit samples.
        let shorts: Vec<u8> = (0..16u16).flat_map(|value| (value * 4000).to_le_bytes()).collect();
        let path = folder.join("height.r16");
        std::fs::write(&path, &shorts).unwrap();
        let read = Heightfield::load(&path, None).unwrap();
        assert_eq!((read.width, read.height), (4, 4));
        assert!((read.samples[15] - 60000.0 / 65535.0).abs() < 1e-6);
        // 4 × 4 floats in a .raw: 64 bytes, not 32 squared samples.
        let floats: Vec<u8> = (0..16).flat_map(|value| (value as f32 / 15.0).to_le_bytes()).collect();
        let path = folder.join("height.raw");
        std::fs::write(&path, &floats).unwrap();
        let read = Heightfield::load(&path, None).unwrap();
        assert_eq!(read.width, 4);
        assert!((read.samples[15] - 1.0).abs() < 1e-6);
        std::fs::remove_dir_all(&folder).ok();
    }
}
