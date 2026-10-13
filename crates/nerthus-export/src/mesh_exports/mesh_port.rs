//! `MeshPort_Save` and `MeshPort_SavePointCloud` from the native mesh code:
//! the mesh a heightmap or Sophia became, written as OBJ by its own writer
//! or handed to Assimp's exporter for every other extension (the scene
//! built, then the format picked); point clouds as PLY.
//!
//! Numbers print with `std::ostream << float` (six
//! significant digits, `%g`). Files opened without `ios::binary` (ASCII
//! PLY) get Windows' CRLF line ends from the C runtime.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use nerthus_nodes::mesh_rasterizer::{compute_normals, Mesh};

use super::assimp_export;
use super::numbers::ostream_float;

/// `UVOrigin`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UvOrigin {
    TopLeft,
    #[default]
    BottomLeft,
}

impl UvOrigin {
    pub fn from_name(name: &str) -> Self {
        if name == "TopLeft" { UvOrigin::TopLeft } else { UvOrigin::BottomLeft }
    }
}

/// What the managed side hands `MeshPort_Save` and
/// `MeshPort_SavePointCloud`.
#[derive(Clone, Copy, Debug)]
pub struct SaveParameters {
    /// The tile's corner in mesh units.
    pub top_left: [f32; 2],
    pub bottom_right: [f32; 2],
    /// Units per terrain width.
    pub scale: f32,
    pub uv_origin: UvOrigin,
    /// The pivot at the centre rather than a corner.
    pub centered: bool,
    pub normals: bool,
    pub uvs: bool,
    /// The grid's triangle pairs written as quads.
    pub quads: bool,
    /// Binary PLY.
    pub binary: bool,
}

impl SaveParameters {
    /// A range with no extent becomes the
    /// unit square from its corner.
    pub fn new(scale: f32, top_left: [f32; 2], bottom_right: [f32; 2]) -> Self {
        let bottom_right = if bottom_right == top_left { [top_left[0] + 1.0, top_left[1] + 1.0] } else { bottom_right };
        Self {
            top_left,
            bottom_right,
            scale,
            uv_origin: UvOrigin::TopLeft,
            centered: false,
            normals: false,
            uvs: false,
            quads: false,
            binary: false,
        }
    }
}

/// Where a mesh vertex lands in the file. Vertices are
/// (x, y) over the unit square and z the height in terrain widths; files
/// get (x, height, y).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Placement {
    /// −0.5 with a centred pivot.
    pub offset: f32,
    /// `|left − right| × scale`: what heights are multiplied by.
    pub height_scale: f32,
    pub scale: f32,
    pub top_left_origin: bool,
    /// Texel-centred UVs over the mesh's grid (1, 1, 0, 0 without one).
    pub u_scale: f32,
    pub v_scale: f32,
    pub u_offset: f32,
    pub v_offset: f32,
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Placement {
    pub fn new(parameters: &SaveParameters, resolution: Option<(i32, i32)>) -> Self {
        let offset = if parameters.centered { -0.5 } else { 0.0 };
        let width = (parameters.top_left[0] - parameters.bottom_right[0]).abs();
        let (u_scale, v_scale, u_offset, v_offset) = match resolution {
            Some((columns, rows)) => {
                let columns = columns as f32;
                let rows = rows as f32;
                ((columns - 1.0) / columns, (rows - 1.0) / rows, 0.5 / columns, 0.5 / rows)
            }
            None => (1.0, 1.0, 0.0, 0.0),
        };
        Self {
            offset,
            height_scale: width * parameters.scale,
            scale: parameters.scale,
            top_left_origin: parameters.uv_origin == UvOrigin::TopLeft,
            u_scale,
            v_scale,
            u_offset,
            v_offset,
            left: parameters.top_left[0],
            top: parameters.top_left[1],
            right: parameters.bottom_right[0],
            bottom: parameters.bottom_right[1],
        }
    }

    /// (x, height, y) as written.
    pub fn position(&self, vertex: [f32; 3]) -> [f32; 3] {
        let x = (((self.right - self.left) * vertex[0] + self.left) + self.offset) * self.scale;
        let height = self.height_scale * vertex[2];
        let y = (((self.bottom - self.top) * vertex[1] + self.top) + self.offset) * self.scale;
        [x, height, y]
    }

    /// (u, v) for a vertex: flipped unless the origin is the top left.
    pub fn texture_coordinate(&self, vertex: [f32; 3]) -> [f32; 2] {
        let u = self.u_scale * vertex[0] + self.u_offset;
        let along = if self.top_left_origin { vertex[1] } else { 1.0 - vertex[1] };
        let v = along * self.v_scale + self.v_offset;
        [u, v]
    }
}

/// The text after the path's last dot, empty without one.
fn extension_after_dot(path: &Path) -> String {
    let text = path.to_string_lossy();
    match text.rfind('.') {
        Some(position) => text[position + 1..].to_string(),
        None => String::new(),
    }
}

/// `MeshPort_Save`: OBJ by its own writer, any other extension through
/// Assimp. `resolution` is the grid the mesh was made from (UVs are
/// texel-centred over it). Returns whether a file was written (Assimp
/// failures are logged).
pub fn save_mesh(path: &Path, mesh: &mut Mesh, resolution: Option<(i32, i32)>, parameters: &SaveParameters) -> io::Result<bool> {
    // The writer's own check: a range with no extent grows by one.
    let mut parameters = *parameters;
    if parameters.bottom_right[0] == parameters.top_left[0] && parameters.bottom_right[1] == parameters.top_left[1] {
        parameters.bottom_right[0] += 1.0;
        parameters.bottom_right[1] += 1.0;
    }
    if extension_after_dot(path) == "obj" {
        if parameters.normals {
            compute_normals(mesh);
        }
        if dumps_requested() {
            write_mesh_dump(path, mesh, resolution, &parameters)?;
        }
        write_obj(path, mesh, resolution, &parameters)?;
        return Ok(true);
    }
    assimp_export::save(path, mesh, resolution, &parameters)
}

/// `NERTHUS_MESH_DUMP`: also write `<file>.meshdump`, what the writers
/// start from, laid out as the comparison oracle writes it, for bit-exact
/// comparisons.
fn dumps_requested() -> bool {
    std::env::var_os("NERTHUS_MESH_DUMP").is_some()
}

fn dump_path(path: &Path) -> std::path::PathBuf {
    let mut text = path.as_os_str().to_os_string();
    text.push(".meshdump");
    std::path::PathBuf::from(text)
}

/// The save parameters' 29 bytes, zero-padded to 32.
fn parameter_bytes(parameters: &SaveParameters) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    let floats = [parameters.top_left[0], parameters.top_left[1], parameters.bottom_right[0], parameters.bottom_right[1], parameters.scale];
    for (index, value) in floats.iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    let origin: i32 = if parameters.uv_origin == UvOrigin::TopLeft { 0 } else { 1 };
    bytes[20..24].copy_from_slice(&origin.to_le_bytes());
    let flags = [parameters.centered, parameters.normals, parameters.uvs, parameters.quads, parameters.binary];
    for (index, value) in flags.iter().enumerate() {
        bytes[24 + index] = *value as u8;
    }
    bytes
}

fn write_mesh_dump(path: &Path, mesh: &Mesh, resolution: Option<(i32, i32)>, parameters: &SaveParameters) -> io::Result<()> {
    let mut output = BufWriter::new(File::create(dump_path(path))?);
    output.write_all(b"NMSH")?;
    output.write_all(&1u32.to_le_bytes())?;
    output.write_all(&parameter_bytes(parameters))?;
    output.write_all(&(mesh.vertices.len() as u32).to_le_bytes())?;
    for vertex in &mesh.vertices {
        for value in vertex {
            output.write_all(&value.to_le_bytes())?;
        }
    }
    output.write_all(&(mesh.triangles.len() as u32).to_le_bytes())?;
    for triangle in &mesh.triangles {
        for value in triangle {
            output.write_all(&value.to_le_bytes())?;
        }
    }
    let (width, height) = resolution.unwrap_or((-1, -1));
    output.write_all(&width.to_le_bytes())?;
    output.write_all(&height.to_le_bytes())?;
    let normals: &[[f32; 3]] = if parameters.normals { &mesh.normals } else { &[] };
    output.write_all(&(normals.len() as u32).to_le_bytes())?;
    for normal in normals {
        for value in normal {
            output.write_all(&value.to_le_bytes())?;
        }
    }
    output.flush()
}

fn write_point_dump(path: &Path, points: &[[f32; 3]], parameters: &SaveParameters) -> io::Result<()> {
    let mut output = BufWriter::new(File::create(dump_path(path))?);
    output.write_all(b"NPCL")?;
    output.write_all(&1u32.to_le_bytes())?;
    output.write_all(&parameter_bytes(parameters))?;
    output.write_all(&(points.len() as u32).to_le_bytes())?;
    for point in points {
        for value in point {
            output.write_all(&value.to_le_bytes())?;
        }
    }
    output.flush()
}

fn write_line_ending(output: &mut impl Write, crlf: bool) -> io::Result<()> {
    if crlf { output.write_all(b"\r\n") } else { output.write_all(b"\n") }
}

/// A header, `v` lines, then `vt` (with UVs), `vn` (with
/// normals, which no face refers to) and `f` lines — triangles, or each
/// pair of grid triangles as a quad. The file is opened in binary mode.
fn write_obj(path: &Path, mesh: &Mesh, resolution: Option<(i32, i32)>, parameters: &SaveParameters) -> io::Result<()> {
    let placement = Placement::new(parameters, resolution);
    let file = File::create(path)?;
    let mut output = BufWriter::with_capacity(1 << 20, file);

    let vertex_count = mesh.vertices.len();
    let triangle_count = mesh.triangles.len();
    let face_count = if parameters.quads { triangle_count as u32 >> 1 } else { triangle_count as u32 };
    output.write_all(b"# OBJ file generated by Nerthus\n")?;
    writeln!(output, "# Vertices: {}", vertex_count as u32)?;
    write!(output, "# Faces: {face_count}\n\n")?;

    for &vertex in &mesh.vertices {
        let [x, height, y] = placement.position(vertex);
        writeln!(output, "v {} {} {}", ostream_float(x), ostream_float(height), ostream_float(y))?;
    }
    if parameters.uvs {
        for &vertex in &mesh.vertices {
            let [u, v] = placement.texture_coordinate(vertex);
            writeln!(output, "vt {} {}", ostream_float(u), ostream_float(v))?;
        }
    }
    if parameters.normals {
        for normal in &mesh.normals {
            writeln!(output, "vn {} {} {}", ostream_float(normal[0]), ostream_float(normal[2]), ostream_float(normal[1]))?;
        }
    }

    let with_uvs = parameters.uvs;
    let corner = |index: u32| -> String {
        let number = index.wrapping_add(1);
        if with_uvs { format!("{number}/{number}") } else { number.to_string() }
    };
    if parameters.quads {
        let mut first = 0usize;
        while first < triangle_count {
            let a = mesh.triangles[first];
            // The pair's second triangle: an odd count would read past the
            // end; a grid always has pairs.
            let b = mesh.triangles.get(first + 1).copied().unwrap_or([0, 0, 0]);
            writeln!(output, "f {} {} {} {}", corner(a[0]), corner(b[1]), corner(b[2]), corner(a[2]))?;
            first += 2;
        }
    } else {
        for triangle in &mesh.triangles {
            writeln!(output, "f {} {} {}", corner(triangle[0]), corner(triangle[1]), corner(triangle[2]))?;
        }
    }
    output.flush()
}

/// `MeshPort_SavePointCloud`: PLY with (x, height, y)
/// float vertices; ASCII PLY is a text-mode file (CRLF). Returns false
/// when the extension is not `ply` (the `xyz` branch is never used:
/// PointCloud writes XYZ itself).
pub fn save_point_cloud(path: &Path, points: &[[f32; 3]], parameters: &SaveParameters) -> io::Result<bool> {
    // The managed side's `MeshPort_SavePointCloud` call (and the oracle's
    // dump) comes before the extension check.
    if dumps_requested() {
        write_point_dump(path, points, parameters)?;
    }
    if extension_after_dot(path) != "ply" {
        return Ok(false);
    }
    let offset = if parameters.centered { -0.5 } else { 0.0 };
    let height_scale = (parameters.bottom_right[0] - parameters.top_left[0]).abs() * parameters.scale;
    let left = parameters.top_left[0];
    let top = parameters.top_left[1];
    let right = parameters.bottom_right[0];
    let bottom = parameters.bottom_right[1];
    let place = |point: [f32; 3]| -> [f32; 3] {
        let x = (((right - left) * point[0] + left) + offset) * parameters.scale;
        let height = height_scale * point[2];
        let y = ((((bottom - top) * point[1]) + top) + offset) * parameters.scale;
        [x, height, y]
    };

    let crlf = !parameters.binary;
    let file = File::create(path)?;
    let mut output = BufWriter::with_capacity(1 << 20, file);
    output.write_all(b"ply")?;
    write_line_ending(&mut output, crlf)?;
    if parameters.binary {
        output.write_all(b"format binary_little_endian 1.0")?;
    } else {
        output.write_all(b"format ascii 1.0")?;
    }
    write_line_ending(&mut output, crlf)?;
    write!(output, "element vertex {}", points.len() as u32)?;
    write_line_ending(&mut output, crlf)?;
    for axis in ["x", "y", "z"] {
        write!(output, "property float {axis}")?;
        write_line_ending(&mut output, crlf)?;
    }
    output.write_all(b"end_header")?;
    write_line_ending(&mut output, crlf)?;

    if parameters.binary {
        for &point in points {
            for value in place(point) {
                output.write_all(&value.to_le_bytes())?;
            }
        }
    } else {
        for &point in points {
            let [x, height, y] = place(point);
            write!(output, "{} {} {}", ostream_float(x), ostream_float(height), ostream_float(y))?;
            write_line_ending(&mut output, crlf)?;
        }
    }
    output.flush()?;
    Ok(true)
}
