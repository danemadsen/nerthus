//! Renders one scene with `nerthus_nodes::shade` for comparison with the
//! native ShadeIspc run through the native shim (`native-run shade`).
//!
//! usage: shadesweep <size> <azimuth> <elevation> <shadow quality> <angular diameter> <ao model 0|1|2> <in.raw> <out directory>
//!
//! Prints the sun direction as hex floats (for native-run) and writes the
//! soft-shadow samples (dx/dy/dz/offset_x/offset_y.raw) and the rendered
//! maps (diffuse/shadow/occlusion/red/green/blue.raw).

use std::env;
use std::fs;
use std::path::Path;

use nerthus_engine::{Map, Resolution};
use nerthus_nodes::shade::{self, AmbientOcclusionModel, LightRig};

fn write_raw(path: &Path, values: &[f32]) {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    fs::write(path, bytes).expect("output should be writable");
}

fn main() {
    let arguments: Vec<String> = env::args().collect();
    let size: usize = arguments[1].parse().expect("size");
    let azimuth: f32 = arguments[2].parse().expect("azimuth");
    let elevation: f32 = arguments[3].parse().expect("elevation");
    let shadow_quality: i32 = arguments[4].parse().expect("shadow quality");
    let angular_diameter: f32 = arguments[5].parse().expect("angular diameter");
    let ao_model = match arguments[6].as_str() {
        "0" => AmbientOcclusionModel::Raytraced,
        "1" => AmbientOcclusionModel::Hybrid,
        _ => AmbientOcclusionModel::Local,
    };
    let bytes = fs::read(&arguments[7]).expect("input should be readable");
    let heights: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])).collect();
    let output = Path::new(&arguments[8]);
    fs::create_dir_all(output).expect("output directory");

    let rig = LightRig {
        shadow_quality,
        sun_angular_diameter: angular_diameter,
        ao_model,
        raytraced_ao_quality: 4,
        hybrid_ao_quality: 4,
        hybrid_ao_octaves: 2,
        automatic_colors: false,
        sun_color: [1.0, 1.0, 1.0],
        ambient_color: [1.0, 1.0, 1.0],
        ..LightRig::default()
    };
    let sky = rig.sky(azimuth, elevation);
    let [x, y, z] = sky.sun_direction;
    println!("{x:e} {y:e} {z:e}");

    let (count, samples) = shade::shadow_samples(sky.sun_direction, shadow_quality, angular_diameter);
    eprintln!("{} samples averaged over {count}", samples.len());
    let planes: [(&str, Vec<f32>); 5] = [
        ("dx.raw", samples.iter().map(|sample| sample.direction[0]).collect()),
        ("dy.raw", samples.iter().map(|sample| sample.direction[1]).collect()),
        ("dz.raw", samples.iter().map(|sample| sample.direction[2]).collect()),
        ("offset_x.raw", samples.iter().map(|sample| sample.offset.0).collect()),
        ("offset_y.raw", samples.iter().map(|sample| sample.offset.1).collect()),
    ];
    for (name, values) in &planes {
        write_raw(&output.join(name), values);
    }

    // Packed samples (dx, dy, dz, offset x, offset y) for `native-run
    // shadowsweep`, and every sample's own lit counts.
    let mut packed = Vec::with_capacity(samples.len() * 5);
    let mut counts = Vec::with_capacity(samples.len() * size * size);
    let vertical_scale = 5000.0 / (size as f32 * 2500.0);
    for sample in &samples {
        packed.extend_from_slice(&[sample.direction[0], sample.direction[1], sample.direction[2], sample.offset.0, sample.offset.1]);
        let sample_counts = shade::shadow_sample_counts(&heights, size, sample, vertical_scale);
        counts.extend(sample_counts.iter().map(|&count| f32::from_bits(count as u32)));
    }
    write_raw(&output.join("samples.raw"), &packed);
    write_raw(&output.join("counts.raw"), &counts);

    let heightmap = Map::from_data(Resolution::square(size), heights);
    let maps = rig.shade(&sky, &heightmap, None, (5000.0, 2500.0));
    write_raw(&output.join("diffuse.raw"), maps.diffuse.data());
    write_raw(&output.join("shadow.raw"), maps.shadow.data());
    write_raw(&output.join("occlusion.raw"), maps.occlusion.data());
    for (channel, name) in ["red.raw", "green.raw", "blue.raw"].iter().enumerate() {
        write_raw(&output.join(name), maps.color.channel(channel));
    }
}
