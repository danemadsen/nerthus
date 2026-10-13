//! The Nerthus side of the oracle's `meshexport` command:
//!
//!   mesh-export grid|adaptive <in.raw> <size> <out file> [Option=Value…]
//!   mesh-export points <in.raw> <size> <out file> [Format=…] [Scale=…] [Centered=…] [IgnoreZeroValues=…]
//!
//! Options as `MeshExportOptions` names them (Vertices, LODCount, Scale,
//! Centered, CreateTextureCoordinates, CreateNormals, CreateWalls,
//! UVOrigin, TopLeft=x,y, BottomRight=x,y, Topology). The terrain is
//! the default, 5000 × 2500 m. Set NERTHUS_MESH_DUMP to get the
//! `.meshdump` files the oracle writes.

use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;

use nerthus_engine::{Map, Resolution};
use nerthus_export::mesh_exports::mesh_port::UvOrigin;
use nerthus_export::mesh_exports::{export_adaptive, export_grid, map_points, write_points, FileResult, MeshExportOptions, TerrainSize};

fn read_map(path: &str, size: usize) -> Map {
    let bytes = std::fs::read(path).expect("cannot read the input map");
    let (words, _) = bytes.as_chunks::<4>();
    let values: Vec<f32> = words.iter().map(|&word| f32::from_le_bytes(word)).collect();
    let plane = size * size;
    let first = values[..plane].to_vec();
    Map::from_data(Resolution::square(size), first)
}

fn pair(text: &str) -> [f32; 2] {
    let (x, y) = text.split_once(',').expect("x,y");
    [x.parse().expect("x"), y.parse().expect("y")]
}

fn options(settings: &HashMap<String, String>) -> MeshExportOptions {
    let get = |name: &str, default: &str| settings.get(name).cloned().unwrap_or_else(|| default.to_string());
    MeshExportOptions {
        scale: get("Scale", "Normalized"),
        centered: get("Centered", "true") == "true",
        uvs: get("CreateTextureCoordinates", "false") == "true",
        walls: get("CreateWalls", "false") == "true",
        lod_count: get("LODCount", "1").parse().expect("LODCount"),
        vertices: get("Vertices", "256").parse().expect("Vertices"),
        uv_origin: UvOrigin::from_name(&get("UVOrigin", "BottomLeft")),
        normals: get("CreateNormals", "false") == "true",
        top_left: pair(&get("TopLeft", "0,0")),
        bottom_right: pair(&get("BottomRight", "1,1")),
    }
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.len() < 4 {
        eprintln!("usage: mesh-export grid|adaptive|points <in.raw> <size> <out file> [Option=Value…]");
        return ExitCode::from(1);
    }
    let mode = arguments[0].as_str();
    let size: usize = arguments[2].parse().expect("size");
    let map = read_map(&arguments[1], size);
    let output = Path::new(&arguments[3]);
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).expect("cannot create the output folder");
    }
    let settings: HashMap<String, String> = arguments[4..]
        .iter()
        .map(|setting| {
            let (key, value) = setting.split_once('=').expect("Key=Value");
            (key.to_string(), value.to_string())
        })
        .collect();
    let terrain = TerrainSize::new(5000.0, 2500.0);
    let mut failed = false;
    let mut on_file = |result: FileResult| match result {
        FileResult::Written(path) => println!("wrote {}", path.display()),
        FileResult::Failed { path, message } => {
            failed = true;
            eprintln!("failed {}: {message}", path.display());
        }
    };
    match mode {
        "grid" => {
            let quads = settings.get("Topology").map(String::as_str) == Some("Quads");
            export_grid(output, &map, quads, &options(&settings), terrain, &mut on_file);
        }
        "adaptive" => export_adaptive(output, &map, &options(&settings), terrain, &mut on_file),
        "points" => {
            let ignore_zero = settings.get("IgnoreZeroValues").map(String::as_str) != Some("false");
            let points = map_points(&map, ignore_zero);
            let format = settings.get("Format").cloned().unwrap_or_else(|| "PLY".to_string());
            let scale = match settings.get("Scale").map(String::as_str) {
                Some("Meter") => terrain.width,
                Some("Kilometer") => terrain.width * 0.001,
                _ => 1.0,
            };
            let centered = settings.get("Centered").map(String::as_str) != Some("false");
            match write_points(output, &points, &format, scale, centered) {
                Ok(written) => println!("points written: {written}"),
                Err(error) => {
                    failed = true;
                    eprintln!("failed: {error}");
                }
            }
        }
        other => {
            eprintln!("unknown mode {other}");
            return ExitCode::from(1);
        }
    }
    if failed { ExitCode::from(2) } else { ExitCode::SUCCESS }
}
