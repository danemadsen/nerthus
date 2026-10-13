//! Runs the CW water port on a raw heightmap with the same JSON settings
//! the emulator runner feeds the native engine.
//!
//! Usage: `water-compare <settings.json> <height.raw> <output-prefix>`.

use std::env;
use std::fs;

use nerthus_erosion::cw::{edge_fill, erode_lakes, LakeSettings};
use serde_json::Value;

fn read_raw(path: &str) -> Vec<f32> {
    let bytes = fs::read(path).expect("raw file should be readable");
    let mut values = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.as_chunks::<4>().0 {
        let raw = [chunk[0], chunk[1], chunk[2], chunk[3]];
        values.push(f32::from_le_bytes(raw));
    }
    values
}

fn write_raw(path: &str, values: &[f32]) {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    fs::write(path, bytes).expect("output should be writable");
}

fn number(settings: &Value, name: &str) -> f64 {
    settings[name].as_f64().unwrap_or_else(|| panic!("setting {name} should be a number"))
}

fn main() {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let text = fs::read_to_string(&arguments[0]).expect("settings should be readable");
    let json: Value = serde_json::from_str(&text).expect("settings should be JSON");
    let heights = read_raw(&arguments[1]);
    let resolution = number(&json, "resolution") as usize;
    let mode = json["mode"].as_str().unwrap_or("lake");
    let output = if mode == "edge" {
        let mask = json["mask"].as_str().map(read_raw);
        let level = number(&json, "level") as f32;
        edge_fill(&heights, resolution, level, mask.as_deref())
    } else {
        let settings = LakeSettings {
            resolution,
            precipitation: number(&json, "precipitation"),
            water_floor: number(&json, "water_floor"),
            lake_size_bias: number(&json, "lake_size_bias"),
            shore_size: number(&json, "shore_size"),
            altitude_bias: number(&json, "altitude_bias"),
            size_bias: number(&json, "size_bias"),
            size: number(&json, "size"),
            height_range: number(&json, "height_range"),
        };
        let area = json["precipitation_area"].as_str().map(read_raw);
        erode_lakes(&settings, &heights, area.as_deref())
    };
    let prefix = &arguments[2];
    write_raw(&format!("{prefix}-results.raw"), &output.results);
    write_raw(&format!("{prefix}-depth.raw"), &output.depth);
    write_raw(&format!("{prefix}-shore.raw"), &output.shore);
    write_raw(&format!("{prefix}-surface.raw"), &output.surface);
}
