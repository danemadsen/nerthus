//! Runs the sun sweep of `nerthus_nodes::lighting` on a half-resolution
//! map for given sun directions, for comparing with the native ISPC sweep
//! through the native shim (`native-run sunsweep`).
//!
//! usage: sunsweep <width> <terrain width> <terrain height> <heights.raw> <suns.raw> <out directory>
//!
//! Writes nx.raw, ny.raw and nz.raw (the normals both sides use) and
//! light.raw (i32 per cell). suns.raw holds (x, y, z, intensity) floats.

use std::env;
use std::fs;
use std::path::Path;

fn read_floats(path: &str) -> Vec<f32> {
    let bytes = fs::read(path).expect("input should be readable");
    bytes.as_chunks::<4>().0.iter().map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])).collect()
}

fn write_floats(path: &Path, values: &[f32]) {
    let bytes: Vec<u8> = values.iter().flat_map(|value| value.to_le_bytes()).collect();
    fs::write(path, bytes).expect("output should be writable");
}

fn main() {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments[0] == "suns" {
        // suns <day begin> <day end> <latitude> <north> <suns.raw>
        let settings = nerthus_nodes::lighting::SunlightSettings {
            day_begin: arguments[1].parse().expect("day"),
            day_end: arguments[2].parse().expect("day"),
            latitude: arguments[3].parse().expect("latitude"),
            north: arguments[4].parse().expect("north"),
            air_density: 0.01,
        };
        let (_, suns) = nerthus_nodes::lighting::sun_directions(&settings);
        let values: Vec<f32> = suns.iter().flat_map(|(direction, intensity)| [direction[0], direction[1], direction[2], *intensity]).collect();
        write_floats(Path::new(&arguments[5]), &values);
        println!("{} suns", suns.len());
        return;
    }
    if arguments[0] == "downsample" {
        // downsample <width> <in.raw> <out.raw>
        let width: usize = arguments[1].parse().expect("width");
        let heights = read_floats(&arguments[2]);
        let coarse = nerthus_nodes::lighting::downsample(&heights, width, width);
        write_floats(Path::new(&arguments[3]), &coarse);
        return;
    }
    let width: usize = arguments[0].parse().expect("width");
    let terrain_width: f32 = arguments[1].parse().expect("terrain width");
    let terrain_height: f32 = arguments[2].parse().expect("terrain height");
    let heights = read_floats(&arguments[3]);
    let packed = read_floats(&arguments[4]);
    let directory = Path::new(&arguments[5]);
    fs::create_dir_all(directory).expect("output directory");

    let terrain = (terrain_width, terrain_height);
    let normals = nerthus_nodes::lighting::normals(&heights, width, width, terrain);
    write_floats(&directory.join("nx.raw"), &normals[0]);
    write_floats(&directory.join("ny.raw"), &normals[1]);
    write_floats(&directory.join("nz.raw"), &normals[2]);
    let suns: Vec<([f32; 3], f32)> = packed.as_chunks::<4>().0.iter().map(|sun| ([sun[0], sun[1], sun[2]], sun[3])).collect();
    let vertical_scale = terrain_width / (width as f32 * terrain_height);
    let light = nerthus_nodes::lighting::sweep_light(&heights, &normals, width, &suns, vertical_scale);
    let bytes: Vec<u8> = light.iter().flat_map(|value| value.to_le_bytes()).collect();
    fs::write(directory.join("light.raw"), bytes).expect("light should be writable");
    println!("zcoef {vertical_scale:?} ({:#x})", vertical_scale.to_bits());
}
