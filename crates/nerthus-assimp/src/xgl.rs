//! Assimp's XGL importer (`XGLLoader.cpp`): the first element named
//! exactly `WORLD` (a lowercase `<world>` finds none and fails), its
//! objects, meshes and materials. A mesh's faces (`f`) and lines (`l`)
//! need a material each and go into an output mesh per material and
//! normal/UV presence, ordered by that key; a mesh without them is still
//! an empty output mesh. `.zgl` files are raw deflate after two bytes.
//!
//! Kept quirks: `<p>` always defines a point (its point-primitive branch
//! is unreachable); lighting is never read; a transform reads a nested
//! `TRANSFORM` element; vectors stop early at a missing comma, and a
//! comma between digits is a decimal point. Node transforms don't reach
//! the mesh, but their reads can fail.

use std::collections::{BTreeMap, HashMap};

use crate::compression::inflate_raw;
use crate::fast_atof::{fast_atoreal_move, strtoul10};
use crate::parsing::{at, skip_spaces};
use crate::registry::get_extension;
use crate::scene::{ImportError, Mesh, Scene, fail};
use crate::xml::{self, Node as XmlNode};

/// `ErrorId`.
const ERROR_ID: u32 = !0;

/// Object nesting the C++ recursion would only end with a stack overflow.
const MAX_DEPTH: usize = 2048;

#[derive(Default)]
struct TempMesh {
    points: HashMap<u32, [f32; 3]>,
    normals: HashMap<u32, [f32; 3]>,
    uvs: HashMap<u32, [f32; 2]>,
}

#[derive(Clone, Copy, Default)]
struct TempFace {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    has_normal: bool,
    has_uv: bool,
}

#[derive(Default)]
struct TempMaterialMesh {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    vertex_counts: Vec<u32>,
    primitive_types: u32,
}

#[derive(Default)]
struct Scope {
    meshes: Vec<Mesh>,
    /// `meshes`, the multimap of mesh ids.
    mesh_ids: Vec<u32>,
    /// `materials`: an id's latest material, by its linear index.
    materials: HashMap<u32, u32>,
    material_count: u32,
}

/// The lowercase name (`ai_stdStrToLower`).
fn lower_name(node: &XmlNode) -> Vec<u8> {
    node.name().to_ascii_lowercase()
}

/// `getValueAsString`: the node's text, trimmed of C-locale whitespace.
fn text_value(node: &XmlNode) -> Vec<u8> {
    let text = node.text();
    let is_space = |byte: &u8| matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r');
    let start = text.iter().position(|byte| !is_space(byte)).unwrap_or(text.len());
    let end = text.iter().rposition(|byte| !is_space(byte)).map_or(start, |last| last + 1);
    text[start..end].to_vec()
}

/// `ReadFloat`.
fn read_float(node: &XmlNode) -> Result<f32, ImportError> {
    let text = text_value(node);
    let (start, goes_on) = skip_spaces(&text, 0, text.len());
    if !goes_on {
        // "unexpected EOL, failed to parse index element"
        return Ok(0.0);
    }
    Ok(fast_atoreal_move(&text, start, true)?.0)
}

/// `ReadIndexFromText`: `ErrorId` when there is no number.
fn read_index_from_text(node: &XmlNode) -> u32 {
    let text = text_value(node);
    let (start, goes_on) = skip_spaces(&text, 0, text.len());
    if !goes_on {
        return ERROR_ID;
    }
    let (value, end) = strtoul10(&text, start);
    if end == start {
        return ERROR_ID;
    }
    value
}

/// `ReadVec3` (`N` = 3) and `ReadVec2` (`N` = 2): comma-separated
/// numbers, stopping early (with what was read for three, nothing for
/// two) at a missing value or comma.
fn read_vector<const N: usize>(node: &XmlNode) -> Result<[f32; N], ImportError> {
    let text = text_value(node);
    let mut values = [0.0f32; N];
    let mut position = 0;
    for index in 0..N {
        let (start, goes_on) = skip_spaces(&text, position, text.len());
        if !goes_on {
            return Ok(if N == 3 { values } else { [0.0; N] });
        }
        let (value, next) = fast_atoreal_move(&text, start, true)?;
        values[index] = value;
        position = skip_spaces(&text, next, text.len()).0;
        if index != N - 1 && at(&text, position) != b',' {
            return Ok(if N == 3 { values } else { [0.0; N] });
        }
        position += 1;
    }
    Ok(values)
}

/// `ReadIDAttr`: the first attribute named `id` in any case, as an int.
fn read_id_attribute(node: &XmlNode) -> u32 {
    for (name, value) in &node.attributes {
        if name.eq_ignore_ascii_case(b"id") {
            return xml::as_int(value) as u32;
        }
    }
    ERROR_ID
}

/// `ReadMaterial`: its colours and values (read for their failures).
fn read_material(node: &XmlNode, scope: &mut Scope) -> Result<u32, ImportError> {
    let id = read_id_attribute(node);
    for child in &node.children {
        match lower_name(child).as_slice() {
            b"amb" | b"diff" | b"spec" | b"emiss" => {
                read_vector::<3>(child)?;
            }
            b"alpha" | b"shine" => {
                read_float(child)?;
            }
            _ => {}
        }
    }
    scope.materials.insert(id, scope.material_count);
    scope.material_count += 1;
    Ok(id)
}

/// `ResolveMaterialRef`: an inline `mat` (named exactly that), else the
/// node's text as a material id.
fn resolve_material_ref(node: &XmlNode, scope: &mut Scope) -> Result<u32, ImportError> {
    if node.name() == b"mat" {
        read_material(node, scope)?;
        return Ok(scope.material_count - 1);
    }
    let id = read_index_from_text(node);
    match scope.materials.get(&id) {
        Some(&index) => Ok(index),
        None => fail("<matref> index out of range"),
    }
}

/// `ReadFaceVertex`.
fn read_face_vertex(node: &XmlNode, mesh: &TempMesh, out: &mut TempFace) -> Result<(), ImportError> {
    let mut has_position = false;
    for child in &node.children {
        match lower_name(child).as_slice() {
            b"pref" => {
                let id = read_index_from_text(child);
                let Some(&point) = mesh.points.get(&id) else {
                    return fail("point index out of range");
                };
                out.position = point;
                has_position = true;
            }
            b"nref" => {
                let id = read_index_from_text(child);
                let Some(&normal) = mesh.normals.get(&id) else {
                    return fail("normal index out of range");
                };
                out.normal = normal;
                out.has_normal = true;
            }
            b"tcref" => {
                let id = read_index_from_text(child);
                let Some(&uv) = mesh.uvs.get(&id) else {
                    return fail("uv index out of range");
                };
                out.uv = uv;
                out.has_uv = true;
            }
            b"p" => out.position = read_vector::<3>(child)?,
            b"n" => out.normal = read_vector::<3>(child)?,
            b"tc" => out.uv = read_vector::<2>(child)?,
            _ => {}
        }
    }
    if !has_position {
        return fail("missing <pref> in <fvN> element");
    }
    Ok(())
}

/// `ReadVertices`: the face's vertices and its material.
fn read_vertices(node: &XmlNode, mesh: &TempMesh, faces: &mut [TempFace; 3], has: &mut [bool; 3], scope: &mut Scope) -> Result<u32, ImportError> {
    let mut material = ERROR_ID;
    for child in &node.children {
        match lower_name(child).as_slice() {
            b"fv1" | b"lv1" | b"pv1" => {
                read_face_vertex(child, mesh, &mut faces[0])?;
                has[0] = true;
            }
            b"fv2" | b"lv2" => {
                read_face_vertex(child, mesh, &mut faces[1])?;
                has[1] = true;
            }
            b"fv3" => {
                read_face_vertex(child, mesh, &mut faces[2])?;
                has[2] = true;
            }
            b"mat" | b"matref" => material = resolve_material_ref(child, scope)?,
            _ => {}
        }
    }
    Ok(material)
}

/// A point, normal or texture coordinate definition's id (`ID`, exact).
fn definition_id(node: &XmlNode) -> Option<u32> {
    node.attribute("ID").map(|value| xml::as_int(value) as u32)
}

/// `ToOutputMesh`.
fn to_output_mesh(source: TempMaterialMesh) -> Mesh {
    let vertex_count = source.positions.len();
    let mut mesh = Mesh { vertex_count, primitive_types: source.primitive_types, ..Mesh::default() };
    if !source.normals.is_empty() {
        mesh.normals = Some(source.normals);
    }
    if !source.uvs.is_empty() {
        mesh.uv_components[0] = 2;
        let coords = source.uvs.iter().take(vertex_count).map(|uv| [uv[0], uv[1], 0.0]).collect();
        mesh.texture_coords[0] = Some(coords);
    }
    let mut next = 0u32;
    let mut faces = Vec::with_capacity(source.vertex_counts.len());
    for &count in &source.vertex_counts {
        faces.push((next..next + count).collect());
        next += count;
    }
    mesh.vertices = Some(source.positions);
    mesh.faces = Some(faces);
    mesh
}

/// `ReadMesh`, appending its output meshes.
fn read_mesh(node: &XmlNode, scope: &mut Scope) -> Result<(), ImportError> {
    let mut temp = TempMesh::default();
    let mut material = 99999u32;
    let mut created = false;
    let mut by_material: BTreeMap<u32, (u32, TempMaterialMesh)> = BTreeMap::new();
    let mesh_id = read_id_attribute(node);
    for child in &node.children {
        let name = lower_name(child);
        match name.as_slice() {
            b"mat" => material = read_material(child, scope)?,
            b"p" => {
                if let Some(id) = definition_id(child) {
                    temp.points.insert(id, read_vector::<3>(child)?);
                }
            }
            b"n" => {
                if let Some(id) = definition_id(child) {
                    temp.normals.insert(id, read_vector::<3>(child)?);
                }
            }
            b"tc" => {
                if let Some(id) = definition_id(child) {
                    temp.uvs.insert(id, read_vector::<2>(child)?);
                }
            }
            b"f" | b"l" => {
                let vertex_count = if name == b"f" { 3 } else { 2 };
                let mut faces = [TempFace::default(); 3];
                let mut has = [false; 3];
                let material_id = read_vertices(child, &temp, &mut faces, &mut has, scope)?;
                if material_id == ERROR_ID {
                    return fail("missing material index");
                }
                let mut with_normals = false;
                let mut with_uvs = false;
                for index in 0..vertex_count {
                    if !has[index] {
                        return fail("missing face vertex data");
                    }
                    with_normals |= faces[index].has_normal;
                    with_uvs |= faces[index].has_uv;
                }
                let key = material_id | (with_normals as u32) << 31 | (with_uvs as u32) << 30;
                let (_, mesh) = by_material.entry(key).or_default();
                created = true;
                for face in faces.iter().take(vertex_count) {
                    mesh.positions.push(face.position);
                    if with_normals {
                        mesh.normals.push(face.normal);
                    }
                    if with_uvs {
                        mesh.uvs.push(face.uv);
                    }
                    mesh.primitive_types |= 1 << (vertex_count - 1);
                }
                mesh.vertex_counts.push(vertex_count as u32);
            }
            _ => {}
        }
    }
    if !created {
        by_material.entry(mesh_id).or_insert((material, TempMaterialMesh::default()));
    }
    for (_, (_, mesh)) in by_material {
        scope.meshes.push(to_output_mesh(mesh));
        if mesh_id != ERROR_ID {
            scope.mesh_ids.push(mesh_id);
        } else {
            scope.mesh_ids.push(ERROR_ID);
        }
    }
    Ok(())
}

/// `ReadTrafo`, for its failures: the values of a nested `TRANSFORM`.
fn read_transform(node: &XmlNode) -> Result<(), ImportError> {
    let Some(transform) = node.child("TRANSFORM") else {
        return Ok(());
    };
    for child in &transform.children {
        match lower_name(child).as_slice() {
            b"forward" | b"up" | b"position" => {
                read_vector::<3>(child)?;
            }
            b"scale" => {
                read_float(child)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// `ReadObject`.
fn read_object(node: &XmlNode, scope: &mut Scope, depth: usize) -> Result<(), ImportError> {
    if depth > MAX_DEPTH {
        return fail("XGL: objects nested too deep");
    }
    for child in &node.children {
        match lower_name(child).as_slice() {
            b"mesh" => read_mesh(child, scope)?,
            b"mat" => {
                if read_material(child, scope)? == ERROR_ID {
                    return fail("Invalid material id detected.");
                }
            }
            b"object" => read_object(child, scope, depth + 1)?,
            b"meshref" => {
                let id = read_index_from_text(child);
                // Meshes without an id aren't in the multimap.
                let known = scope.mesh_ids.iter().any(|&mesh_id| mesh_id == id && mesh_id != ERROR_ID);
                if !known {
                    return fail("<meshref> index out of range");
                }
            }
            b"transform" => read_transform(child)?,
            _ => {}
        }
    }
    Ok(())
}

pub fn read(path: &str, bytes: &[u8]) -> Result<Scene, ImportError> {
    let data = if get_extension(path) == "zgl" {
        if bytes.is_empty() {
            return fail("StreamReader: File is empty or EOF is already reached");
        }
        if bytes.len() < 2 {
            return fail("End of file or read limit was reached");
        }
        inflate_raw(&bytes[2..])?
    } else {
        bytes.to_vec()
    };
    let Ok(document) = xml::parse(&data) else {
        return fail("XML parse error while loading XGL file");
    };
    let mut scope = Scope::default();
    if let Some(world) = document.find_node("WORLD") {
        read_object(world, &mut scope, 0)?;
    }
    if scope.meshes.is_empty() || scope.material_count == 0 {
        return fail("failed to extract data from XGL file, no meshes loaded");
    }
    Ok(Scene { meshes: scope.meshes })
}
