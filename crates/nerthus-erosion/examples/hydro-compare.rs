//! Runs the Hydrologic port on a raw heightmap with the same JSON settings
//! the emulator runner feeds the native engine.
//!
//! Usage: `hydro-compare <settings.json> <height.raw> <output-prefix>`.

use std::env;
use std::fs;

use nerthus_erosion::hydro::{hydrological_correction, rivers, RiverSettings};
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

fn main() {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let text = fs::read_to_string(&arguments[0]).expect("settings should be readable");
    let json: Value = serde_json::from_str(&text).expect("settings should be JSON");
    let resolution = json["resolution"].as_u64().expect("resolution") as usize;
    let downcutting = json["downcutting"].as_f64().expect("downcutting") as f32;
    let heights = read_raw(&arguments[1]);
    let prefix = &arguments[2];
    if json["mode"].as_str() == Some("rivers") {
        let settings = RiverSettings {
            seed: json["seed"].as_i64().expect("seed") as i32,
            width: json["width"].as_f64().expect("width") as f32,
            depth: json["depth"].as_f64().expect("depth") as f32,
            headwaters: json["headwaters"].as_i64().expect("headwaters") as i32,
            water: json["water"].as_f64().expect("water") as f32,
            render_surface: json["render_surface"].as_bool().expect("render_surface"),
            downcutting,
        };
        let density = json["headwaters_density"].as_str().map(read_raw);
        let output = rivers(&settings, &heights, resolution, density.as_deref());
        write_raw(&format!("{prefix}-results.raw"), &output.heights);
        write_raw(&format!("{prefix}-depth.raw"), &output.depth);
        write_raw(&format!("{prefix}-surface.raw"), &output.surface);
        write_raw(&format!("{prefix}-direction.raw"), &output.direction);
        return;
    }
    let results = hydrological_correction(&heights, resolution, downcutting);
    write_raw(&format!("{prefix}-results.raw"), &results);
}
