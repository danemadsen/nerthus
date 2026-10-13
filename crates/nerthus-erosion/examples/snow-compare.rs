//! Runs the CS snow port on a raw heightmap with the same JSON settings the
//! emulator runner feeds the native engine.
//!
//! Usage: `snow-compare <settings.json> <height.raw> <output-prefix>`.

use std::env;
use std::fs;

use nerthus_erosion::cs::{simulate, simulate_with_snapshot, SnowSettings};
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

fn flag(settings: &Value, name: &str) -> bool {
    settings[name].as_bool().unwrap_or_else(|| panic!("setting {name} should be a boolean"))
}

fn main() {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let text = fs::read_to_string(&arguments[0]).expect("settings should be readable");
    let json: Value = serde_json::from_str(&text).expect("settings should be JSON");
    let settings = SnowSettings {
        size: number(&json, "size"),
        scale: number(&json, "scale"),
        snowfall_duration: number(&json, "snowfall_duration"),
        snowfall_intensity: number(&json, "snowfall_intensity"),
        post_snowfall_duration: number(&json, "post_snowfall_duration"),
        melting: number(&json, "melting"),
        snow_line: number(&json, "snow_line"),
        enable_thermal: flag(&json, "enable_thermal"),
        talus_angle: number(&json, "talus_angle"),
        flow_volume: number(&json, "flow_volume"),
        height_range: number(&json, "height_range"),
        snow_output: number(&json, "snow_output") as i32,
    };
    let heights = read_raw(&arguments[1]);
    let snowfall = json["snowfall_area"].as_str().map(read_raw);
    let melt = json["melt_area"].as_str().map(read_raw);
    let prefix = &arguments[2];
    if let Some(pass) = arguments.get(3) {
        let pass: usize = pass.parse().expect("pass should be a number");
        let (_, snapshot) = simulate_with_snapshot(&settings, &heights, pass);
        let snapshot = snapshot.expect("the run should reach that slip pass");
        write_raw(&format!("{prefix}-height.raw"), &snapshot.height);
        write_raw(&format!("{prefix}-work.raw"), &snapshot.work);
        write_raw(&format!("{prefix}-snow.raw"), &snapshot.snow);
        return;
    }
    let output = simulate(&settings, &heights, snowfall.as_deref(), melt.as_deref());
    write_raw(&format!("{prefix}-height.raw"), &output.height);
    write_raw(&format!("{prefix}-snow.raw"), &output.snow);
    write_raw(&format!("{prefix}-soft.raw"), &output.soft);
    write_raw(&format!("{prefix}-hard.raw"), &output.hard);
}
