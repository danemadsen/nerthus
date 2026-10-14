//! Assimp's Neutral File Format importer (`NFFLoader.cpp`), both kinds.
//!
//! NFF: polygons (`p`, any line starting with `p`, so `pb`/`pn` become
//! empty polygons), polygon patches with normals (`pp`) and texture
//! coordinates (`tpp`), their vertices stored back to front; shading
//! (`f`, `shader`) splits meshes; spheres, polyhedra and cones are unit
//! `StandardShapes` (their placement goes to node transforms, which don't
//! reach the mesh). Missing values keep what the vector held before.
//!
//! NFF2 (`nff` first): objects of vertices with `0x` colours, `norm` and
//! `uv`, then faces with colours, textures, `both` and `matid` (from
//! external `mtable` files), a mesh per object, shading and material
//! (the last match wins). `0x` only matches as a token of its own.
//!
//! The C++ loops forever on a vertex attribute it doesn't know and on a
//! `_x` texture attribute without its second underscore; those, writes
//! past a mesh name and colour copies past the colours are errors here.

use crate::fast_atof::{fast_atoreal_move, strtoul10, strtoul16};
use crate::io::FileSystem;
use crate::parsing::{BUFFER_SIZE, at, get_next_line, is_line_end, is_space_or_newline, remove_line_comments, skip_spaces, text_file_to_buffer, token_match};
use crate::scene::{ImportError, Mesh, Scene, check_allocation, fail};
use crate::shapes;

/// `MeshInfo::MaxNameLen`.
const MAX_NAME_LENGTH: usize = 128;

fn endless<T>() -> Result<T, ImportError> {
    fail("NFF: a line the C++ reads forever")
}

/// `ShadingInfo` (the fields its `==` compares, plus what feeds them).
#[derive(Clone)]
struct Shading {
    color: [f32; 3],
    diffuse: [f32; 3],
    specular: [f32; 3],
    ambient: [f32; 3],
    emissive: [f32; 3],
    refraction: f32,
    texture: Vec<u8>,
    two_sided: bool,
    shaded: bool,
    opacity: f32,
    shininess: f32,
}

impl Default for Shading {
    fn default() -> Self {
        Shading {
            color: [0.6; 3],
            diffuse: [1.0; 3],
            specular: [1.0; 3],
            ambient: [0.0; 3],
            emissive: [0.0; 3],
            refraction: 1.0,
            texture: Vec::new(),
            two_sided: false,
            shaded: true,
            opacity: 1.0,
            shininess: 0.0,
        }
    }
}

impl Shading {
    /// `ShadingInfo::operator==`.
    fn same(&self, other: &Shading) -> bool {
        self.color == other.color
            && self.diffuse == other.diffuse
            && self.specular == other.specular
            && self.ambient == other.ambient
            && self.refraction == other.refraction
            && self.texture == other.texture
            && self.two_sided == other.two_sided
            && self.shaded == other.shaded
    }
}

#[derive(Clone, Default)]
struct MeshInfo {
    shading: Shading,
    vertices: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 3]>,
    faces: Vec<u32>,
    colors: Vec<[f32; 4]>,
    material: u32,
}

/// A line buffer and a cursor in it (`line` and `sz`).
struct Line {
    bytes: [u8; BUFFER_SIZE],
    cursor: usize,
}

impl Line {
    fn new() -> Self {
        Line { bytes: [0; BUFFER_SIZE], cursor: 0 }
    }

    fn at(&self, offset: usize) -> u8 {
        at(&self.bytes, self.cursor + offset)
    }

    fn skip_spaces(&mut self, end: usize) {
        self.cursor = skip_spaces(&self.bytes, self.cursor, end).0;
    }

    fn token(&mut self, token: &[u8]) -> bool {
        token_match(&self.bytes, &mut self.cursor, token)
    }

    /// `AI_NFF_PARSE_FLOAT`: blanks skipped, then a number unless the line
    /// ends (the value then stays).
    fn float(&mut self, value: &mut f32, end: usize) -> Result<(), ImportError> {
        self.skip_spaces(end);
        if !is_line_end(self.at(0)) {
            let (number, next) = fast_atoreal_move(&self.bytes, self.cursor, true)?;
            *value = number;
            self.cursor = next;
        }
        Ok(())
    }

    fn triple(&mut self, values: &mut [f32; 3], end: usize) -> Result<(), ImportError> {
        for value in values.iter_mut() {
            self.float(value, end)?;
        }
        Ok(())
    }

    /// The token at the cursor (to a blank or line end).
    fn word(&mut self) -> Vec<u8> {
        let start = self.cursor;
        while !is_space_or_newline(self.at(0)) {
            self.cursor += 1;
        }
        self.bytes[start..self.cursor].to_vec()
    }
}

/// `LoadNFF2MaterialTable`.
fn load_material_table(file_system: &FileSystem, path: &str) -> Result<Vec<Shading>, ImportError> {
    let mut table: Vec<Shading> = Vec::new();
    let Some(bytes) = file_system.open(path) else {
        return Ok(table);
    };
    let mut buffer = text_file_to_buffer(&bytes, false)?;
    remove_line_comments(b"//", &mut buffer, b' ');
    let mut position = 0;
    if !token_match(&buffer, &mut position, b"mat") {
        return Ok(table);
    }
    // The line end the C++ gives its line buffer.
    let end = 2096;
    let mut line = Line::new();
    let mut current: Option<usize> = None;
    while get_next_line(&buffer, &mut position, &mut line.bytes)? {
        line.cursor = 0;
        line.skip_spaces(end);
        if line.token(b"version") {
            continue;
        }
        if line.token(b"matdef") {
            table.push(Shading::default());
            current = Some(table.len() - 1);
            continue;
        }
        if line.token(b"valid") {
            continue;
        }
        if is_line_end(line.at(0)) {
            continue;
        }
        let Some(shader) = current else {
            continue;
        };
        let mut colour = [0.0f32; 3];
        if line.token(b"ambient") {
            line.triple(&mut colour, end)?;
            table[shader].ambient = colour;
        } else if line.token(b"diffuse") || line.token(b"ambientdiffuse") {
            line.triple(&mut colour, end)?;
            table[shader].diffuse = colour;
            table[shader].ambient = colour;
        } else if line.token(b"specular") {
            line.triple(&mut colour, end)?;
            table[shader].specular = colour;
        } else if line.token(b"emission") {
            line.triple(&mut colour, end)?;
            table[shader].emissive = colour;
        } else if line.token(b"shininess") {
            let mut value = table[shader].shininess;
            line.float(&mut value, end)?;
            table[shader].shininess = value;
        } else if line.token(b"opacity") {
            let mut value = table[shader].opacity;
            line.float(&mut value, end)?;
            table[shader].opacity = value;
        }
    }
    Ok(table)
}

/// `AI_NFF2_GET_NEXT_TOKEN`: the next line with something on it (the
/// cursor at its first non-blank); at the end the line stays.
fn next_token(buffer: &[u8], position: &mut usize, line: &mut Line) -> Result<(), ImportError> {
    loop {
        if !get_next_line(buffer, position, &mut line.bytes)? {
            return Ok(());
        }
        line.cursor = 0;
        line.skip_spaces(BUFFER_SIZE);
        if !is_line_end(line.at(0)) {
            return Ok(());
        }
    }
}

/// The material table path of an `mtable` token.
fn material_table_path(token: &[u8], file: &str) -> String {
    let mut path = String::from_utf8_lossy(token).into_owned();
    // `find_last_of(".mat")` looks for any of the four characters.
    if !path.contains(['.', 'm', 'a', 't']) {
        path.push_str(".mat");
    }
    let backslash = path.rfind('\\').filter(|&position| position != 0);
    let slash = path.rfind('/').filter(|&position| position != 0);
    if backslash.is_none() && slash.is_none() {
        let separator = file.rfind('\\').or_else(|| file.rfind('/'));
        if let Some(separator) = separator {
            path = format!("{}{}", &file[..separator + 1], path);
        }
    }
    path
}

/// The NFF2 objects into `meshes`.
fn read_nff2(buffer: &[u8], mut position: usize, file: &str, file_system: &FileSystem, meshes: &mut Vec<MeshInfo>) -> Result<(), ImportError> {
    let nan = f32::NAN;
    let mut line = Line::new();
    while get_next_line(buffer, &mut position, &mut line.bytes)? {
        line.cursor = 0;
        line.skip_spaces(BUFFER_SIZE);
        if line.token(b"version") {
            continue;
        }
        if line.token(b"viewpos") || line.token(b"viewdir") {
            let mut camera = [0.0f32; 3];
            line.triple(&mut camera, BUFFER_SIZE)?;
            continue;
        }
        if is_space_or_newline(line.at(0)) {
            continue;
        }

        // An object: its name, material tables, vertices and faces.
        let object_name = line.word();
        let object_start = meshes.len();
        let mut sub_mesh = 0u32;
        let mut material_table: Vec<Shading> = Vec::new();
        loop {
            next_token(buffer, &mut position, &mut line)?;
            if !line.token(b"mtable") {
                break;
            }
            line.skip_spaces(BUFFER_SIZE);
            let token = line.word();
            if !token.is_empty() {
                material_table = load_material_table(file_system, &material_table_path(&token, file))?;
            }
        }

        let (vertex_count, next) = strtoul10(&line.bytes, line.cursor);
        line.cursor = next;
        check_allocation(vertex_count as u64, 16)?;
        let mut positions: Vec<[f32; 3]> = Vec::new();
        let mut colors: Vec<[f32; 4]> = Vec::new();
        let mut normals: Vec<[f32; 3]> = Vec::new();
        let mut uvs: Vec<[f32; 3]> = Vec::new();
        let (mut has_normals, mut has_uvs, mut has_color) = (false, false, false);
        for _ in 0..vertex_count {
            next_token(buffer, &mut position, &mut line)?;
            let mut vertex = [0.0f32; 3];
            line.triple(&mut vertex, BUFFER_SIZE)?;
            positions.push(vertex);
            loop {
                line.skip_spaces(BUFFER_SIZE);
                if is_line_end(line.at(0)) {
                    break;
                }
                if line.token(b"0x") {
                    has_color = true;
                    let (value, next) = strtoul16(&line.bytes, line.cursor);
                    line.cursor = next;
                    let channel = |shift: u32| ((value >> shift) & 0xff) as f32 / 255.0;
                    colors.push([channel(16), channel(8), channel(0), 1.0]);
                } else if line.token(b"norm") {
                    has_normals = true;
                    line.triple(&mut vertex, BUFFER_SIZE)?;
                    normals.push(vertex);
                } else if line.token(b"uv") {
                    has_uvs = true;
                    line.float(&mut vertex[0], BUFFER_SIZE)?;
                    line.float(&mut vertex[1], BUFFER_SIZE)?;
                    vertex[2] = 0.0;
                    uvs.push(vertex);
                } else {
                    return endless();
                }
            }
            if normals.len() != positions.len() {
                normals.push([nan, 0.0, 0.0]);
            }
            if uvs.len() != positions.len() {
                uvs.push([nan, 0.0, 0.0]);
            }
            if colors.len() != positions.len() {
                colors.push([nan, 0.0, 0.0, 1.0]);
            }
        }
        next_token(buffer, &mut position, &mut line)?;
        if vertex_count == 0 {
            return fail("NFF2: There are zero vertices");
        }

        let (face_count, next) = strtoul10(&line.bytes, line.cursor);
        line.cursor = next;
        let mut indices: Vec<u32> = Vec::new();
        for _ in 0..face_count {
            next_token(buffer, &mut position, &mut line)?;
            line.cursor = 0;
            line.skip_spaces(BUFFER_SIZE);
            let (index_count, next) = strtoul10(&line.bytes, line.cursor);
            line.cursor = next;
            if index_count != 0 {
                // A face without a count keeps the previous face's indices.
                check_allocation(index_count as u64, 4)?;
                indices.resize(index_count as usize, 0);
                for slot in indices.iter_mut() {
                    line.skip_spaces(BUFFER_SIZE);
                    let (mut index, next) = strtoul10(&line.bytes, line.cursor);
                    line.cursor = next;
                    if index as usize >= positions.len() {
                        // "NFF2: Vertex index overflow"
                        index = 0;
                    }
                    *slot = index;
                }
            }

            let mut shading = Shading { color: [1.0; 3], ..Shading::default() };
            let mut material = 0u32;
            let mut face_colour = [1.0f32; 4];
            loop {
                line.skip_spaces(BUFFER_SIZE);
                if is_line_end(line.at(0)) {
                    break;
                }
                if line.token(b"0x") {
                    has_color = true;
                    let start = line.cursor;
                    let (value, next) = strtoul16(&line.bytes, line.cursor);
                    line.cursor = next;
                    if next - start > 3 {
                        face_colour[0] = ((value >> 16) & 0xff) as f32 / 255.0;
                        face_colour[1] = ((value >> 8) & 0xff) as f32 / 255.0;
                        face_colour[2] = (value & 0xff) as f32 / 255.0;
                    } else {
                        face_colour[0] = ((value >> 8) & 0xf) as f32 / 16.0;
                        face_colour[1] = ((value >> 4) & 0xf) as f32 / 16.0;
                        face_colour[2] = (value & 0xf) as f32 / 16.0;
                    }
                } else if line.at(0) == b'_' {
                    if matches!(line.at(1), b'v' | b'V') {
                        shading.shaded = false;
                    }
                    if line.at(1) == 0 || line.at(2) != b'_' {
                        return endless();
                    }
                    let name_start = line.cursor + 3;
                    while !is_space_or_newline(line.at(0)) {
                        line.cursor += 1;
                    }
                    if line.cursor > name_start {
                        shading.texture = line.bytes[name_start..line.cursor].to_vec();
                    }
                } else if line.token(b"both") {
                    shading.two_sided = true;
                } else if !material_table.is_empty() && line.token(b"matid") {
                    line.skip_spaces(BUFFER_SIZE);
                    let (value, next) = strtoul10(&line.bytes, line.cursor);
                    line.cursor = next;
                    material = value;
                    if material as usize >= material_table.len() {
                        // "NFF2: Material index overflow."
                        material = 0;
                    }
                    let from = &material_table[material as usize];
                    shading.ambient = from.ambient;
                    shading.diffuse = from.diffuse;
                    shading.emissive = from.emissive;
                    shading.opacity = from.opacity;
                    shading.specular = from.specular;
                    shading.shininess = from.shininess;
                } else {
                    // `SkipToken`.
                    line.skip_spaces(BUFFER_SIZE);
                    while !is_space_or_newline(line.at(0)) {
                        line.cursor += 1;
                    }
                }
            }

            let found = (object_start..meshes.len()).rfind(|&index| meshes[index].shading.same(&shading) && meshes[index].material == material);
            let mesh_index = match found {
                Some(index) => index,
                None => {
                    if !object_name.is_empty() {
                        let digits = sub_mesh.to_string().len();
                        if object_name.len() + digits + 1 > MAX_NAME_LENGTH {
                            return fail("undefined behaviour: a mesh name past its buffer");
                        }
                        sub_mesh += 1;
                    }
                    meshes.push(MeshInfo { shading: shading.clone(), material, ..MeshInfo::default() });
                    meshes.len() - 1
                }
            };
            if !indices.is_empty() {
                let mesh = &mut meshes[mesh_index];
                mesh.faces.push(indices.len() as u32);
                for &index in &indices {
                    let index = index as usize;
                    if has_color {
                        let colour = colors[index];
                        mesh.colors.push(if colour[0].is_nan() { face_colour } else { colour });
                    }
                    mesh.vertices.push(positions[index]);
                    if has_normals {
                        mesh.normals.push(normals[index]);
                    }
                    if has_uvs {
                        mesh.uvs.push(uvs[index]);
                    }
                }
            }
        }
        if face_count == 0 {
            return fail("NFF2: There are zero faces");
        }
    }
    Ok(())
}

/// Which polygon list a `p`, `pp` or `tpp` line adds to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Patch {
    Simple,
    Normals,
    UvAndNormals,
}

/// The classic NFF lines into the four mesh lists, in output order.
fn read_nff(buffer: &[u8], mut position: usize) -> Result<Vec<MeshInfo>, ImportError> {
    let mut simple: Vec<MeshInfo> = Vec::new();
    let mut locked: Vec<MeshInfo> = Vec::new();
    let mut with_normals: Vec<MeshInfo> = Vec::new();
    let mut with_uvs: Vec<MeshInfo> = Vec::new();
    let mut shading = Shading::default();
    let mut tessellation = 4u32;
    let mut line = Line::new();
    let end = BUFFER_SIZE;

    while get_next_line(buffer, &mut position, &mut line.bytes)? {
        line.cursor = 0;
        let first = line.bytes[0];
        if first == b'p' || line.token(b"tpp") {
            let patch = if first == b't' {
                Patch::UvAndNormals
            } else if line.bytes[1] == b'p' {
                line.cursor = 2;
                Patch::Normals
            } else {
                line.cursor = 1;
                Patch::Simple
            };
            let list = match patch {
                Patch::Simple => &mut simple,
                Patch::Normals => &mut with_normals,
                Patch::UvAndNormals => &mut with_uvs,
            };
            let index = match list.iter().position(|mesh| mesh.shading.same(&shading)) {
                Some(index) => index,
                None => {
                    list.push(MeshInfo { shading: shading.clone(), ..MeshInfo::default() });
                    list.len() - 1
                }
            };
            line.skip_spaces(end);
            let count = strtoul10(&line.bytes, line.cursor).0;
            let mesh = &mut list[index];
            let total = mesh.vertices.len() as u64 + count as u64;
            check_allocation(total, 12)?;
            mesh.vertices.resize(total as usize, [0.0; 3]);
            if patch != Patch::Simple {
                mesh.normals.resize(total as usize, [0.0; 3]);
            }
            if patch == Patch::UvAndNormals {
                mesh.uvs.resize(total as usize, [0.0; 3]);
            }
            for corner in 0..count as usize {
                if !get_next_line(buffer, &mut position, &mut line.bytes)? {
                    // "NFF: Unexpected EOF was encountered. Patch definition
                    // incomplete": every later line is missing too.
                    break;
                }
                let slot = total as usize - corner - 1;
                let mut vertex = [0.0f32; 3];
                line.cursor = 0;
                line.triple(&mut vertex, end)?;
                mesh.vertices[slot] = vertex;
                if patch != Patch::Simple {
                    line.triple(&mut vertex, end)?;
                    mesh.normals[slot] = vertex;
                }
                if patch == Patch::UvAndNormals {
                    // The coordinates may continue on the next lines.
                    line.skip_spaces(end);
                    if is_line_end(line.at(0)) {
                        get_next_line(buffer, &mut position, &mut line.bytes)?;
                        line.cursor = 0;
                    }
                    line.float(&mut vertex[0], end)?;
                    line.skip_spaces(end);
                    if is_line_end(line.at(0)) {
                        get_next_line(buffer, &mut position, &mut line.bytes)?;
                        line.cursor = 0;
                    }
                    line.float(&mut vertex[1], end)?;
                    vertex[1] = 1.0 - vertex[1];
                    mesh.uvs[slot] = vertex;
                }
            }
            mesh.faces.push(count);
        } else if line.token(b"f") {
            line.triple(&mut shading.color, end)?;
            line.float(&mut shading.diffuse[0], end)?;
            line.float(&mut shading.specular[0], end)?;
            let mut skipped = 0.0f32;
            line.float(&mut skipped, end)?;
            line.float(&mut skipped, end)?;
            line.float(&mut shading.refraction, end)?;
            shading.diffuse = [shading.diffuse[0]; 3];
            shading.specular = [shading.specular[0]; 3];
            line.skip_spaces(end);
            if !matches!(line.at(0), b'0'..=b'9' | b'-' | b'+') {
                let name = line.word();
                if !name.is_empty() {
                    shading.texture = name;
                }
            } else {
                line.float(&mut shading.ambient[0], end)?;
            }
        } else if line.token(b"shader") {
            line.skip_spaces(end);
            shading.texture = line.word();
        } else if line.token(b"l") {
            let mut values = [0.0f32; 3];
            line.triple(&mut values, end)?;
            let mut intensity = 1.0f32;
            line.float(&mut intensity, end)?;
            line.triple(&mut values, end)?;
        } else if line.token(b"s") || line.token(b"dod") || line.token(b"oct") || line.token(b"tet") || line.token(b"hex") {
            let kind = line.bytes[0];
            let kind = if kind == b'h' { b'x' } else { kind };
            let mut center = [0.0f32; 3];
            let mut radius = [1.0f32, f32::NAN, f32::NAN];
            line.triple(&mut center, end)?;
            line.triple(&mut radius, end)?;
            let mut mesh = MeshInfo { shading: shading.clone(), ..MeshInfo::default() };
            match kind {
                b's' => shapes::sphere(tessellation, &mut mesh.vertices)?,
                b'd' => shapes::dodecahedron(&mut mesh.vertices),
                b'o' => shapes::octahedron(&mut mesh.vertices),
                b't' => shapes::tetrahedron(&mut mesh.vertices),
                _ => {
                    shapes::hexahedron(&mut mesh.vertices, false);
                }
            }
            mesh.faces = vec![3; mesh.vertices.len() / 3];
            locked.push(mesh);
        } else if line.token(b"c") {
            let mut mesh = MeshInfo { shading: shading.clone(), ..MeshInfo::default() };
            if !get_next_line(buffer, &mut position, &mut line.bytes)? {
                // "NFF: Unexpected end of file (cone definition not
                // complete)": the reading stops.
                locked.push(mesh);
                break;
            }
            line.cursor = 0;
            let mut first_center = [0.0f32; 3];
            let mut first_radius = 0.0f32;
            line.triple(&mut first_center, end)?;
            line.float(&mut first_radius, end)?;
            if !get_next_line(buffer, &mut position, &mut line.bytes)? {
                locked.push(mesh);
                break;
            }
            line.cursor = 0;
            let mut second_center = [0.0f32; 3];
            let mut second_radius = 0.0f32;
            line.triple(&mut second_center, end)?;
            line.float(&mut second_radius, end)?;
            let direction = [second_center[0] - first_center[0], second_center[1] - first_center[1], second_center[2] - first_center[2]];
            let height = (direction[0] * direction[0] + direction[1] * direction[1] + direction[2] * direction[2]).sqrt();
            if height < 10e-3 {
                // "NFF: Cone height is close to zero": an empty mesh.
                locked.push(mesh);
                continue;
            }
            let segments = shapes::integer_pow(4, tessellation);
            shapes::cone(height, first_radius, second_radius, segments, false, &mut mesh.vertices)?;
            mesh.faces = vec![3; mesh.vertices.len() / 3];
            locked.push(mesh);
        } else if line.token(b"tess") {
            line.skip_spaces(end);
            tessellation = strtoul10(&line.bytes, line.cursor).0;
        } else if line.token(b"from") || line.token(b"at") || line.token(b"up") {
            let mut values = [0.0f32; 3];
            line.triple(&mut values, end)?;
        } else if line.token(b"angle") {
            let mut angle = 45.0f32;
            line.float(&mut angle, end)?;
        } else if line.token(b"resolution") {
            let mut value = 0.0f32;
            line.float(&mut value, end)?;
            line.float(&mut value, end)?;
        }
    }
    simple.extend(locked);
    simple.extend(with_normals);
    simple.extend(with_uvs);
    Ok(simple)
}

pub fn read(path: &str, bytes: &[u8], files: &FileSystem) -> Result<Scene, ImportError> {
    let mut buffer = text_file_to_buffer(bytes, false)?;
    let mut position = 0;
    let infos = if token_match(&buffer, &mut position, b"nff") {
        remove_line_comments(b"//", &mut buffer, b' ');
        let mut meshes = Vec::new();
        read_nff2(&buffer, position, path, files, &mut meshes)?;
        meshes
    } else {
        read_nff(&buffer, 0)?
    };

    let mut meshes = Vec::new();
    for info in infos.into_iter().filter(|info| !info.faces.is_empty()) {
        let vertex_count = info.vertices.len();
        let mut mesh = Mesh { vertex_count, ..Mesh::default() };
        if !info.colors.is_empty() {
            if info.colors.len() < vertex_count {
                return fail("undefined behaviour: colours copied past their array");
            }
            mesh.colors[0] = Some(info.colors[..vertex_count].to_vec());
        }
        if !info.normals.is_empty() {
            mesh.normals = Some(info.normals);
        }
        if !info.uvs.is_empty() {
            mesh.texture_coords[0] = Some(info.uvs);
        }
        let mut next = 0u32;
        let faces = info
            .faces
            .iter()
            .map(|&count| {
                let face = (next..next.wrapping_add(count)).collect();
                next = next.wrapping_add(count);
                face
            })
            .collect();
        mesh.vertices = Some(info.vertices);
        mesh.faces = Some(faces);
        meshes.push(mesh);
    }
    if meshes.is_empty() {
        return fail("NFF: No meshes loaded");
    }
    Ok(Scene { meshes })
}
