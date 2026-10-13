//! Runs the TS thermal shaper port on a raw heightmap with the same JSON
//! settings the emulator runner feeds the native engine.
//!
//! Usage: `ts-compare <settings.json> <height.raw> <output-prefix>`.

use std::env;
use std::fs;

use nerthus_erosion::ts::thermal_shape;
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
    let shape = json["shape"].as_f64().expect("shape");
    let heights = read_raw(&arguments[1]);
    let results = thermal_shape(&heights, resolution, shape);
    write_raw(&format!("{}-results.raw", arguments[2]), &results);
}
