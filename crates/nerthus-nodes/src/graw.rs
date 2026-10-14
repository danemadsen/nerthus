//! Pure Raw (`.graw`): a map's samples as little-endian floats, channels
//! interleaved, with a `.meta` beside the file — a JSON dictionary of
//! strings giving `Resolution`, `Channels` and `RenderIntent`, and `Min`
//! and `Max` when a one-channel map was stored stretched to another range
//! than [0, 1].
//!
//! The range is process-wide state: reading a file stored in another range
//! makes that range the stored range, and from then on one-channel maps
//! are written to `.graw` in it (exports and offloaded caches alike), so a
//! heightfield read from such a file goes back out in its own range.

use std::path::Path;
use std::sync::Mutex;

use nerthus_engine::{Map, RenderIntent, Resolution};
use serde_json::Value;

/// The range one-channel maps are stored in: (0, 1) until a file stored
/// in another range is read.
static STORED_RANGE: Mutex<[f32; 2]> = Mutex::new([0.0, 1.0]);

/// The range one-channel maps are written in, as (min, max).
pub fn stored_range() -> [f32; 2] {
    *STORED_RANGE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Makes `range` the range one-channel maps are written in.
pub fn set_stored_range(range: [f32; 2]) {
    *STORED_RANGE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = range;
}

/// A `.meta` as it reads back (`MetaDictionary`). Missing keys, and a
/// missing or unreadable file, give the dictionary's starting values:
/// one channel, resolution -1 (from the sample count), intent `Default`,
/// range (0, 1).
#[derive(Clone, Debug, PartialEq)]
pub struct GrawMeta {
    /// The side of the map; 0 when the entry is not a whole number.
    pub resolution: i32,
    /// 0 when the entry is not a whole number.
    pub channels: i32,
    pub render_intent: RenderIntent,
    pub min: f32,
    pub max: f32,
    /// The cache thumbprint an offloaded map was written with.
    pub guid: Option<String>,
}

impl Default for GrawMeta {
    fn default() -> Self {
        Self { resolution: -1, channels: 1, render_intent: RenderIntent::Default, min: 0.0, max: 1.0, guid: None }
    }
}

/// The `.meta` beside a `.graw` file.
pub fn meta_path(path: &Path) -> std::path::PathBuf {
    path.with_extension("meta")
}

/// Reads the `.meta` beside `path`, or `None` when there is none.
pub fn read_meta(path: &Path) -> Option<GrawMeta> {
    let meta_path = meta_path(path);
    if !meta_path.is_file() {
        return None;
    }
    let text = std::fs::read_to_string(&meta_path).unwrap_or_default();
    Some(parse_meta(&text))
}

/// A `.meta`'s text as a dictionary of strings; text that is not a JSON
/// object gives the starting values.
pub fn parse_meta(text: &str) -> GrawMeta {
    let mut meta = GrawMeta::default();
    let Ok(Value::Object(entries)) = serde_json::from_str::<Value>(text) else {
        return meta;
    };
    let entry = |key: &str| -> Option<String> {
        match entries.get(key)? {
            Value::String(text) => Some(text.clone()),
            Value::Null => None,
            other => Some(other.to_string()),
        }
    };
    if let Some(resolution) = entry("Resolution") {
        meta.resolution = dotnet_int(&resolution).unwrap_or(0);
    }
    if let Some(channels) = entry("Channels") {
        meta.channels = dotnet_int(&channels).unwrap_or(0);
    }
    if let Some(intent) = entry("RenderIntent") {
        meta.render_intent = render_intent(&intent);
    }
    if let Some(min) = entry("Min") {
        meta.min = dotnet_float(&min).unwrap_or(0.0);
    }
    if let Some(max) = entry("Max") {
        meta.max = dotnet_float(&max).unwrap_or(1.0);
    }
    meta.guid = entry("GUID");
    meta
}

/// `int.TryParse`: optional surrounding white space and sign, decimal
/// digits, within `i32`.
fn dotnet_int(text: &str) -> Option<i32> {
    let trimmed = text.trim();
    let digits = trimmed.strip_prefix('+').unwrap_or(trimmed);
    if digits.starts_with('+') {
        return None;
    }
    digits.parse().ok()
}

/// `float.TryParse` for the numbers a `.meta` holds.
fn dotnet_float(text: &str) -> Option<f32> {
    let trimmed = text.trim();
    let number = trimmed.strip_prefix('+').unwrap_or(trimmed);
    let lowered = number.to_ascii_lowercase();
    if lowered.contains("inf") || lowered.contains("nan") {
        return match lowered.as_str() {
            "infinity" | "∞" => Some(f32::INFINITY),
            "-infinity" | "-∞" => Some(f32::NEG_INFINITY),
            "nan" => Some(f32::NAN),
            _ => None,
        };
    }
    number.parse().ok()
}

/// `Enum.Parse(typeof(RenderIntent), …)`: a member's name, or its number;
/// anything else is `Default`.
fn render_intent(text: &str) -> RenderIntent {
    let trimmed = text.trim();
    if let Some(intent) = RenderIntent::from_name(trimmed) {
        return intent;
    }
    let numbered = [RenderIntent::Default, RenderIntent::Heightfield, RenderIntent::Mask, RenderIntent::Color, RenderIntent::Raw];
    match dotnet_int(trimmed) {
        Some(number) if (0..numbered.len() as i32).contains(&number) => numbered[number as usize],
        _ => RenderIntent::Default,
    }
}

/// A raw file's bytes as values of `size` bytes. A length that is not a
/// whole number of values is refused, as the block copy into the value
/// array refuses it.
pub fn raw_values(path: &Path, size: usize) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    if !bytes.len().is_multiple_of(size) {
        return Err("Offset and length were out of bounds for the array or count is greater than the number of elements from index to the end of the source collection.".to_string());
    }
    Ok(bytes)
}

/// The floats of a raw file.
pub fn read_floats(path: &Path) -> Result<Vec<f32>, String> {
    let bytes = raw_values(path, 4)?;
    let (words, _) = bytes.as_chunks::<4>();
    Ok(words.iter().map(|word| f32::from_le_bytes(*word)).collect())
}

/// One channel, square, its side the square root of the sample count taken
/// as a float (whatever does not fill the square is dropped).
pub fn single_channel(samples: Vec<f32>) -> Map {
    let side = (samples.len() as f32 as f64).sqrt() as usize;
    let mut samples = samples;
    samples.resize(side * side, 0.0);
    Map::from_data(Resolution::square(side), samples)
}

/// Samples as a map of `resolution` and `channels`: one channel when the
/// samples fit in one, otherwise interleaved channels. A resolution of -1
/// takes the side from the sample count, and so does 0 (an entry that is
/// not a number, for which the project's build bucket size would stand).
pub fn samples_to_map(samples: Vec<f32>, resolution: i32, channels: i32) -> Result<Map, String> {
    let mut size = resolution as i64;
    if size == 0 || size == -1 {
        let per_channel = samples.len() as f32 / channels as f32;
        size = (per_channel as f64).sqrt() as i64;
    }
    if size < 0 || channels <= 0 {
        return Err("The map's resolution or channel count is not valid.".to_string());
    }
    let size = size as usize;
    if samples.len() <= size * size {
        return Ok(single_channel(samples));
    }
    let channels = channels as usize;
    if samples.len() < size * size * channels {
        return Err("Source array was not long enough. Check the source index, length, and the array's lower bounds.".to_string());
    }
    Ok(Map::from_interleaved(Resolution::square(size), &samples, channels))
}

/// Reads a `.graw` and its `.meta`. A map stored in another range than
/// [0, 1] comes back stretched from its first channel's lowest and highest
/// values to [0, 1], and that range becomes the stored range.
pub fn read(path: &Path) -> Result<(Map, GrawMeta), String> {
    if !path.is_file() {
        return Err(format!("Could not find file '{}'.", path.display()));
    }
    let meta = read_meta(path).unwrap_or_default();
    let samples = read_floats(path)?;
    let mut map = samples_to_map(samples, meta.resolution, meta.channels)?;
    if meta.min != 0.0 || meta.max != 1.0 {
        let (low, high) = crate::maths::square_plane_min_max(map.channel(0), map.width());
        set_stored_range([meta.min, meta.max]);
        crate::maths::remap(&mut map, high, low, 1.0, 0.0);
    }
    map.info.render_intent = meta.render_intent;
    Ok((map, meta))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_entries_parse_as_dictionary_strings() {
        let text = "{\r\n  \"$id\": \"1\",\r\n  \"Channels\": \"3\",\r\n  \"Resolution\": \"2\",\r\n  \"RenderIntent\": \"Color\",\r\n  \"Min\": \"-10.5\",\r\n  \"Max\": \"200\",\r\n  \"GUID\": \"abc\"\r\n}";
        let meta = parse_meta(text);
        assert_eq!(meta.channels, 3);
        assert_eq!(meta.resolution, 2);
        assert_eq!(meta.render_intent, RenderIntent::Color);
        assert_eq!((meta.min, meta.max), (-10.5, 200.0));
        assert_eq!(meta.guid.as_deref(), Some("abc"));
        let broken = parse_meta("{\"Resolution\": \"many\", \"RenderIntent\": \"Odd\"}");
        assert_eq!(broken.resolution, 0);
        assert_eq!(broken.render_intent, RenderIntent::Default);
        assert_eq!(parse_meta("not json"), GrawMeta::default());
    }

    #[test]
    fn ranged_files_read_back_normalised_and_set_the_range() {
        let directory = std::env::temp_dir().join(format!("nerthus-graw-read-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("ranged.graw");
        let values = [10.0f32, 20.0, 30.0, 50.0];
        let bytes: Vec<u8> = values.iter().flat_map(|value| value.to_le_bytes()).collect();
        std::fs::write(&path, bytes).unwrap();
        std::fs::write(directory.join("ranged.meta"), "{\"Channels\":\"1\",\"Resolution\":\"2\",\"RenderIntent\":\"Heightfield\",\"Min\":\"10\",\"Max\":\"50\"}").unwrap();
        let (map, meta) = read(&path).unwrap();
        assert_eq!(meta.render_intent, RenderIntent::Heightfield);
        assert_eq!(map.info.render_intent, RenderIntent::Heightfield);
        // Two floats wide, so the scan's padding zero is the lowest value it
        // finds (square_plane_min_max), not 10.
        let scale = 1.0f32 / 50.0;
        assert_eq!(map.channel(0), &[10.0 * scale, 20.0 * scale, 30.0 * scale, 50.0 * scale]);
        assert_eq!(stored_range(), [10.0, 50.0]);
        set_stored_range([0.0, 1.0]);
        std::fs::write(&path, [0u8; 6]).unwrap();
        assert!(read(&path).is_err());
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
