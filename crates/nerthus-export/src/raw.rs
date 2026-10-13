//! The raw formats: plain floats, 16-bit unsigned and half floats, and
//! `.graw` with its `.meta`.
//!
//! Files are truncated as they are opened, so writing over a longer file
//! leaves none of its tail behind.

use std::fs;
use std::io;
use std::path::Path;

use half::f16;
use nerthus_engine::automation::dotnet_float_string;
use nerthus_engine::Map;

/// The samples the raw writers take: one channel as it is, several
/// interleaved.
fn samples(map: &Map) -> Vec<f32> {
    if map.channel_count() <= 1 {
        return map.channel(0).to_vec();
    }
    map.to_interleaved()
}

/// The floats as they are, little-endian.
pub fn write_float(path: &Path, map: &Map) -> io::Result<()> {
    let samples = samples(map);
    let mut bytes = Vec::with_capacity(samples.len() * 4);
    for value in samples {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    fs::write(path, bytes)
}

/// `(ushort)(65535f * v)`, truncated.
pub fn write_ushort(path: &Path, map: &Map) -> io::Result<()> {
    let samples = samples(map);
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for value in samples {
        let scaled = 65535.0f32 * value;
        let sample = scaled as u16;
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    fs::write(path, bytes)
}

/// `(Half)v * Half.MaxValue`. .NET multiplies halves in
/// single precision and rounds the product back to a half.
pub fn write_half(path: &Path, map: &Map) -> io::Result<()> {
    let samples = samples(map);
    let maximum = f16::MAX.to_f32();
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for value in samples {
        let half_value = f16::from_f32(value);
        let product = half_value.to_f32() * maximum;
        let sample = f16::from_f32(product);
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    fs::write(path, bytes)
}

/// The floats, then a `.meta` beside them. A single
/// channel is stored in `range` when that is not (0, 1) (stretched from
/// [0, 1]), and the `.meta` records the range.
pub fn write_graw(path: &Path, map: &Map, range: [f32; 2]) -> io::Result<()> {
    write_graw_with(path, map, range, &[])
}

/// [`write_graw`] with more `.meta` entries after the usual ones (the caches'
/// `GUID`).
pub fn write_graw_with(path: &Path, map: &Map, range: [f32; 2], extra: &[(&str, String)]) -> io::Result<()> {
    let remapped = map.channel_count() == 1 && (range[0] != 0.0 || range[1] != 1.0);
    if remapped {
        let scale = (range[1] - range[0]) / (1.0f32 - 0.0f32);
        let offset = range[0] - 0.0f32 * scale;
        let mut stored = map.clone();
        stored.apply(|value| value * scale + offset);
        write_float(path, &stored)?;
    } else {
        write_float(path, map)?;
    }

    // `MetaDictionary` starts with these keys, so they keep this order.
    let mut entries = vec![
        ("Channels", map.channel_count().to_string()),
        ("Resolution", map.width().to_string()),
        ("RenderIntent", map.info.render_intent.name().to_string()),
        ("Min", "0".to_string()),
        ("Max", "1".to_string()),
    ];
    if remapped {
        entries[3].1 = dotnet_float_string(range[0]);
        entries[4].1 = dotnet_float_string(range[1]);
    }
    for (key, value) in extra {
        entries.push((*key, value.clone()));
    }
    let meta = meta_json(&entries);
    let meta_path = path.with_extension("meta");
    fs::write(meta_path, meta)
}

/// Newtonsoft's indented output for a `MetaDictionary` (`$id` first, as
/// `PreserveReferencesHandling.All` writes it, and Windows line breaks).
fn meta_json(entries: &[(&str, String)]) -> String {
    let mut lines = vec!["  \"$id\": \"1\"".to_string()];
    for (key, value) in entries {
        let quoted_key = serde_json::to_string(key).unwrap_or_default();
        let quoted_value = serde_json::to_string(value).unwrap_or_default();
        lines.push(format!("  {quoted_key}: {quoted_value}"));
    }
    let body = lines.join(",\r\n");
    format!("{{\r\n{body}\r\n}}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerthus_engine::Resolution;

    #[test]
    fn meta_matches_the_saved_layout() {
        let directory = std::env::temp_dir().join(format!("nerthus-graw-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("ramp.graw");
        let map = Map::from_data(Resolution::square(2), vec![0.0, 0.25, 0.5, 1.0]);
        write_graw(&path, &map, [0.0, 1.0]).unwrap();
        let meta = fs::read_to_string(directory.join("ramp.meta")).unwrap();
        let expected = "{\r\n  \"$id\": \"1\",\r\n  \"Channels\": \"1\",\r\n  \"Resolution\": \"2\",\r\n  \"RenderIntent\": \"Heightfield\",\r\n  \"Min\": \"0\",\r\n  \"Max\": \"1\"\r\n}";
        assert_eq!(meta, expected);
        let bytes = fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 16);
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn half_samples_round_like_dotnet() {
        let directory = std::env::temp_dir().join(format!("nerthus-half-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("half.raw");
        let map = Map::from_data(Resolution::new(3, 1), vec![0.0, 0.5, 1.0]);
        write_half(&path, &map).unwrap();
        let bytes = fs::read(&path).unwrap();
        let halves: Vec<f32> = bytes.chunks(2).map(|pair| f16::from_le_bytes([pair[0], pair[1]]).to_f32()).collect();
        assert_eq!(halves, vec![0.0, 32752.0, 65504.0]);
        fs::remove_dir_all(&directory).unwrap();
    }
}
