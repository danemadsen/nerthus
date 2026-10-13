//! Runs the CE erosion port on a raw heightmap with the same JSON settings
//! the emulator runner feeds the native engine.
//!
//! Usage: `erosion-compare <settings.json> <height.raw> <output-prefix>`.

use std::env;
use std::fs;

use nerthus_erosion::ce::{erode, ErosionSettings};
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
    let settings = ErosionSettings {
        seed: number(&json, "seed") as i32,
        size: number(&json, "size"),
        scale: number(&json, "scale"),
        duration: number(&json, "duration"),
        rock_softness: number(&json, "rock_softness"),
        fluvial_erosion_strength: number(&json, "fluvial_erosion_strength"),
        downcutting: number(&json, "downcutting"),
        base_level: number(&json, "base_level"),
        retardation_of_downcutting: number(&json, "retardation_of_downcutting"),
        enable_thermal: flag(&json, "enable_thermal"),
        thermal_erosion_strength: number(&json, "thermal_erosion_strength"),
        talus_angle: number(&json, "talus_angle"),
        thermal_size: number(&json, "thermal_size"),
        flow_volume: number(&json, "flow_volume"),
        mask_by_shader: flag(&json, "mask_by_shader"),
        masking_mode: number(&json, "masking_mode") as i32,
        mask_as_thermal_erosion_strength: flag(&json, "mask_as_thermal_erosion_strength"),
        sediment_removal: number(&json, "sediment_removal"),
        laminar_flow: flag(&json, "laminar_flow"),
        randomness: number(&json, "randomness"),
        thermal_smoothness: number(&json, "thermal_smoothness"),
        talus_settling: number(&json, "talus_settling"),
        thermal_detail: flag(&json, "thermal_detail"),
        debris_size: number(&json, "debris_size"),
        height_range: number(&json, "height_range"),
    };
    let heights = read_raw(&arguments[1]);
    let mask = json["mask"].as_str().map(read_raw);
    let removal = json["sediment_removal_mask"].as_str().map(read_raw);
    let output = erode(&settings, &heights, mask.as_deref(), removal.as_deref());
    let prefix = &arguments[2];
    write_raw(&format!("{prefix}-height.raw"), &output.height);
    write_raw(&format!("{prefix}-deposit.raw"), &output.deposit);
    write_raw(&format!("{prefix}-wear.raw"), &output.wear);
    write_raw(&format!("{prefix}-flow.raw"), &output.flow);
}
