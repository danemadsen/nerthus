//! PointCloud's export: `PointCloud.GetExportFilename` and
//! `PointCloud.Export`. The points are the map's non-zero pixels
//! or, behind a Debris node with Export Point Cloud, the
//! rocks Debris placed. XYZ is written by the managed writer, PLY by the
//! native one (`MeshPort_SavePointCloud`); "Unreal CSV" writes nothing.
//!
//! There are no edition caps on large maps: they are
//! licensing and do not apply.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::Arc;

use nerthus_engine::{Map, NodeData};
use nerthus_nodes::debris::DebrisPointCloud;
use serde_json::Value;

use super::mesh_port::{save_point_cloud, SaveParameters};
use super::numbers::dotnet_single;
use super::{flag, scale_factor, text, FileResult, PostBuildContext};
use crate::build::OverwriteMode;
use crate::destination::place_in_folder;

/// `PointCloud.GetExportFilename`: the export file name with the format's
/// extension, by the save name (or the node's name without one).
fn export_path(context: &PostBuildContext<'_>, node: &NodeData, port_name: &str) -> std::path::PathBuf {
    let extension = if text(node, "Format", "PLY") == "XYZ" { ".xyz" } else { ".ply" };
    let definition = node.base.get("SaveDefinition").filter(|value| !value.is_null());
    let base = match definition {
        Some(definition) => definition.get("Filename").and_then(Value::as_str).unwrap_or_default().to_string(),
        None => node.name.clone(),
    };
    let increment = context.definition.overwrite_mode == OverwriteMode::Increment;
    place_in_folder(&format!("{base}{port_name}{extension}"), context.folder, increment)
}

/// `Node.PostBuild` for one of PointCloud's ports.
pub(super) fn export_port(context: &PostBuildContext<'_>, node: &NodeData, port_name: &str, map: Option<Arc<Map>>, on_file: &mut dyn FnMut(FileResult)) {
    let path = export_path(context, node, port_name);
    let Some(map) = map else {
        on_file(FileResult::Failed { path, message: format!("{} has no map to export", node.name) });
        return;
    };
    let debris = context.engine.memory::<DebrisPointCloud>(node.id);
    let points = match debris {
        Some(points) => points.0.clone(),
        None => map_points(&map, flag(node, "IgnoreZeroValues", true)),
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let format = text(node, "Format", "PLY");
    let scale = scale_factor(text(node, "Scale", "Normalized"), context.terrain.width);
    let centered = flag(node, "CenterPivot", true);
    match write_points(&path, &points, format, scale, centered) {
        Ok(true) => on_file(FileResult::Written(path)),
        Ok(false) => {}
        Err(error) => on_file(FileResult::Failed { path, message: error.to_string() }),
    }
}

/// `PointCloud.Export`'s writing: XYZ by the managed writer, PLY (ASCII or
/// binary) by the native one; "CSV" writes nothing.
/// `scale` is the `MeshScale` factor. Returns whether a file was written.
pub fn write_points(path: &Path, points: &[[f32; 3]], format: &str, scale: f32, centered: bool) -> io::Result<bool> {
    match format {
        "XYZ" => write_xyz(path, points).map(|()| true),
        "CSV" => Ok(false),
        _ => {
            let mut parameters = SaveParameters::new(scale, [0.0, 0.0], [1.0, 1.0]);
            parameters.centered = centered;
            parameters.binary = format == "PLY_BINARY";
            save_point_cloud(path, points, &parameters)
        }
    }
}

/// A point per pixel, row by
/// row, at (x / width, y / width, value), zero values left out when asked.
/// The rows are walked by the map's height for both axes.
pub fn map_points(map: &Map, ignore_zero: bool) -> Vec<[f32; 3]> {
    let width = map.width() as f32;
    let side = map.height();
    let mut points = Vec::with_capacity(map.width() * map.height());
    for row in 0..side {
        for column in 0..side {
            let value = map.get(column, row);
            if !ignore_zero || value != 0.0 {
                points.push([column as f32 / width, row as f32 / width, value]);
            }
        }
    }
    points
}

/// XYZ: "x y z" per point with .NET's float text, in a
/// `StreamWriter` (UTF-8 without a byte order mark, Windows line ends).
fn write_xyz(path: &Path, points: &[[f32; 3]]) -> io::Result<()> {
    let file = File::create(path)?;
    let mut output = BufWriter::with_capacity(1 << 20, file);
    for point in points {
        write!(output, "{} {} {}\r\n", dotnet_single(point[0]), dotnet_single(point[1]), dotnet_single(point[2]))?;
    }
    output.flush()
}
