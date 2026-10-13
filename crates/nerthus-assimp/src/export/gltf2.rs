//! `glTF2Exporter` and `glTF2::AssetWriter` for the scenes the mesh exports make:
//! one buffer of accessor data (a `.bin` beside the `.gltf`, or the GLB's
//! body), materials as metallic-roughness, meshes of one primitive, the
//! node tree and a default scene.
//!
//! Assimp opens the `.gltf` in text mode ("wt"), so on Windows its line
//! ends are CRLF; Nerthus writes them so everywhere.

use std::fs::File;
use std::io::Write;
use std::path::Path;

use super::rapidjson::{write_compact, write_pretty, Json};
use super::{ExportError, ExportMaterial, ExportScene, VERSION_MAJOR, VERSION_MINOR, VERSION_REVISION};
use crate::scene::{Mesh, MAX_TEXTURE_COORDS, PRIMITIVE_LINE, PRIMITIVE_POINT, PRIMITIVE_POLYGON};

const COMPONENT_UNSIGNED_INT: u32 = 5125;
const COMPONENT_FLOAT: u32 = 5126;
const TARGET_ARRAY_BUFFER: u32 = 34962;
const TARGET_ELEMENT_ARRAY_BUFFER: u32 = 34963;

struct BufferView {
    offset: usize,
    length: usize,
    target: u32,
}

struct Accessor {
    buffer_view: usize,
    component_type: u32,
    count: usize,
    kind: &'static str,
    min: Vec<f64>,
    max: Vec<f64>,
}

struct Primitive {
    mode: i64,
    material: usize,
    indices: Option<usize>,
    position: Vec<usize>,
    normal: Vec<usize>,
    texcoord: Vec<usize>,
    ngon_encoded: bool,
}

struct GltfMesh {
    name: String,
    primitive: Primitive,
}

/// The exporter's state: the one buffer and what refers into it.
#[derive(Default)]
struct Asset {
    body: Vec<u8>,
    buffer_views: Vec<BufferView>,
    accessors: Vec<Accessor>,
}

/// The values `ExportData` reads: float or unsigned int components.
enum Data<'a> {
    Floats(&'a [f32]),
    Integers(&'a [u32]),
}

impl Data<'_> {
    fn value(&self, index: usize) -> f64 {
        match self {
            Data::Floats(values) => values[index] as f64,
            Data::Integers(values) => values[index] as f64,
        }
    }

    fn bytes(&self, index: usize) -> [u8; 4] {
        match self {
            Data::Floats(values) => values[index].to_le_bytes(),
            Data::Integers(values) => values[index].to_le_bytes(),
        }
    }
}

impl Asset {
    /// `ExportData`: `count` elements of `components_in` values, the first
    /// `components_out` of each appended to the buffer (offset aligned the
    /// way Assimp does it), with a buffer view and an accessor holding the
    /// per-component range of the finite values.
    fn export_data(&mut self, count: usize, data: Data<'_>, components_in: usize, components_out: usize, kind: &'static str, target: u32) -> Option<usize> {
        if count == 0 {
            return None;
        }
        let bytes_per_component = 4;
        let mut offset = self.body.len();
        let padding = offset % bytes_per_component;
        offset += padding;
        let length = count * components_out * bytes_per_component;
        self.body.resize(self.body.len() + length + padding, 0);
        self.buffer_views.push(BufferView { offset, length, target });

        let mut min = vec![f64::MAX; components_out];
        let mut max = vec![-f64::MAX; components_out];
        for element in 0..count {
            for component in 0..components_out {
                let value = data.value(element * components_in + component);
                if !value.is_finite() {
                    continue;
                }
                if value < min[component] {
                    min[component] = value;
                }
                if value > max[component] {
                    max[component] = value;
                }
            }
        }
        for element in 0..count {
            for component in 0..components_out {
                let at = offset + (element * components_out + component) * bytes_per_component;
                self.body[at..at + 4].copy_from_slice(&data.bytes(element * components_in + component));
            }
        }
        let component_type = match data {
            Data::Floats(_) => COMPONENT_FLOAT,
            Data::Integers(_) => COMPONENT_UNSIGNED_INT,
        };
        self.accessors.push(Accessor { buffer_view: self.buffer_views.len() - 1, component_type, count, kind, min, max });
        Some(self.accessors.len() - 1)
    }
}

fn flatten(values: &[[f32; 3]]) -> Vec<f32> {
    values.iter().flat_map(|value| value.iter().copied()).collect()
}

/// `aiVector3D::NormalizeSafe`.
fn normalize_safe(vector: [f32; 3]) -> [f32; 3] {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    if length > 0.0 {
        let inverse = 1.0 / length;
        return [vector[0] * inverse, vector[1] * inverse, vector[2] * inverse];
    }
    vector
}

/// `ExportMeshes` for one mesh (its texture coordinates flipped in v and
/// its normals normalised in place, as Assimp does to its copy).
fn export_mesh(asset: &mut Asset, mesh: &mut Mesh) -> GltfMesh {
    let count = mesh.vertex_count;
    let mut primitive = Primitive {
        mode: 4,
        material: mesh.material_index as usize,
        indices: None,
        position: Vec::new(),
        normal: Vec::new(),
        texcoord: Vec::new(),
        ngon_encoded: mesh.primitive_types & crate::scene::PRIMITIVE_NGON_ENCODING != 0,
    };
    if let Some(vertices) = &mesh.vertices {
        let values = flatten(vertices);
        if let Some(accessor) = asset.export_data(count, Data::Floats(&values), 3, 3, "VEC3", TARGET_ARRAY_BUFFER) {
            primitive.position.push(accessor);
        }
    }
    if let Some(normals) = &mut mesh.normals {
        for normal in normals.iter_mut() {
            *normal = normalize_safe(*normal);
        }
        let values = flatten(normals);
        if let Some(accessor) = asset.export_data(count, Data::Floats(&values), 3, 3, "VEC3", TARGET_ARRAY_BUFFER) {
            primitive.normal.push(accessor);
        }
    }
    for channel in 0..MAX_TEXTURE_COORDS {
        if !mesh.has_texture_coords(channel) {
            continue;
        }
        let components = mesh.uv_components[channel];
        let coordinates = mesh.texture_coords[channel].as_mut().unwrap();
        if components > 1 {
            for coordinate in coordinates.iter_mut() {
                coordinate[1] = 1.0 - coordinate[1];
            }
        }
        if components > 0 {
            let kind = if components == 2 { "VEC2" } else { "VEC3" };
            let out = if components == 2 { 2 } else { 3 };
            let values = flatten(coordinates);
            if let Some(accessor) = asset.export_data(count, Data::Floats(&values), 3, out, kind, TARGET_ARRAY_BUFFER) {
                primitive.texcoord.push(accessor);
            }
        }
    }
    if let Some(faces) = mesh.faces.as_ref().filter(|faces| !faces.is_empty()) {
        let per_face = faces[0].len();
        let mut indices = Vec::with_capacity(faces.len() * per_face);
        for face in faces {
            for corner in 0..per_face {
                indices.push(face.get(corner).copied().unwrap_or(0));
            }
        }
        primitive.indices = asset.export_data(indices.len(), Data::Integers(&indices), 1, 1, "SCALAR", TARGET_ELEMENT_ARRAY_BUFFER);
    }
    primitive.mode = match mesh.primitive_types {
        PRIMITIVE_POLYGON => 4,
        PRIMITIVE_LINE => 1,
        PRIMITIVE_POINT => 0,
        _ => 4,
    };
    GltfMesh { name: mesh.name.clone(), primitive }
}

fn number_array(values: &[f64], as_integers: bool) -> Json {
    let items = values.iter().map(|&value| if as_integers { Json::Int(value as i64) } else { Json::Double(value) }).collect();
    Json::Array(items)
}

fn float_array(values: &[f32]) -> Json {
    Json::Array(values.iter().map(|&value| Json::Double(value as f64)).collect())
}

/// `Write(Material)` for an exported material: metallic-roughness from the
/// diffuse colour (metallic 0, no textures or extensions with values).
fn material_json(material: &ExportMaterial) -> Json {
    let mut object = Json::object();
    if !material.name.is_empty() {
        object.add("name", Json::String(material.name.clone()));
    }
    let mut metallic_roughness = Json::object();
    if material.diffuse != [1.0, 1.0, 1.0, 1.0] {
        metallic_roughness.add("baseColorFactor", float_array(&material.diffuse));
    }
    metallic_roughness.add("metallicFactor", Json::Double(0.0));
    if !metallic_roughness.is_empty_object() {
        object.add("pbrMetallicRoughness", metallic_roughness);
    }
    object
}

/// `glTF2Exporter` then `AssetWriter::WriteFile` / `WriteGLBFile`.
pub fn write(scene: &mut ExportScene, path: &Path, binary: bool) -> Result<(), ExportError> {
    let mut asset = Asset::default();
    let meshes: Vec<GltfMesh> = scene.meshes.iter_mut().map(|mesh| export_mesh(&mut asset, mesh)).collect();

    let mut document = Json::object();
    let mut metadata = Json::object();
    metadata.add("version", Json::String("2.0".to_string()));
    let generator = format!("Open Asset Import Library (assimp v{VERSION_MAJOR}.{VERSION_MINOR}.{VERSION_REVISION:x})");
    metadata.add("generator", Json::String(generator));
    document.add("asset", metadata);
    // KHR_materials_volume: Assimp's GetMatVolume answers true when a
    // material has no thickness; FB_ngon_encoding is always on.
    let mut extensions = Vec::new();
    if !scene.materials.is_empty() {
        extensions.push(Json::String("KHR_materials_volume".to_string()));
    }
    extensions.push(Json::String("FB_ngon_encoding".to_string()));
    document.add("extensionsUsed", Json::Array(extensions));

    let accessors = asset
        .accessors
        .iter()
        .map(|accessor| {
            let mut object = Json::object();
            object.add("bufferView", Json::Uint(accessor.buffer_view as u64));
            object.add("byteOffset", Json::Uint(0));
            object.add("componentType", Json::Int(accessor.component_type as i64));
            object.add("count", Json::Uint(accessor.count as u64));
            object.add("type", Json::String(accessor.kind.to_string()));
            let integers = accessor.component_type != COMPONENT_FLOAT;
            object.add("max", number_array(&accessor.max, integers));
            object.add("min", number_array(&accessor.min, integers));
            object
        })
        .collect();
    if !asset.accessors.is_empty() {
        document.add("accessors", Json::Array(accessors));
    }

    // The buffer: named after the file without ".gltf" (FindUniqueID with
    // an empty name), its URI that plus ".bin"; GLB's body buffer is
    // written by WriteGLBFile instead.
    let path_text = path.to_string_lossy().into_owned();
    let prefix = match path_text.rfind(".gltf") {
        Some(position) => path_text[..position].to_string(),
        None => path_text.clone(),
    };
    let bin_path = format!("{prefix}.bin");
    let relative_uri = match bin_path.rfind(['/', '\\']) {
        Some(position) => bin_path[position + 1..].to_string(),
        None => bin_path.clone(),
    };
    if binary {
        document.add("buffers", Json::Array(Vec::new()));
    } else {
        let mut buffer = Json::object();
        buffer.add("byteLength", Json::Uint(asset.body.len() as u64));
        buffer.add("uri", Json::String(relative_uri));
        document.add("buffers", Json::Array(vec![buffer]));
    }

    let views = asset
        .buffer_views
        .iter()
        .map(|view| {
            let mut object = Json::object();
            object.add("buffer", Json::Uint(0));
            object.add("byteOffset", Json::Uint(view.offset as u64));
            object.add("byteLength", Json::Uint(view.length as u64));
            object.add("target", Json::Int(view.target as i64));
            object
        })
        .collect();
    if !asset.buffer_views.is_empty() {
        document.add("bufferViews", Json::Array(views));
    }

    if !scene.materials.is_empty() {
        let materials = scene.materials.iter().map(material_json).collect();
        document.add("materials", Json::Array(materials));
    }

    if !meshes.is_empty() {
        let objects = meshes
            .iter()
            .map(|mesh| {
                let mut object = Json::object();
                if !mesh.name.is_empty() {
                    object.add("name", Json::String(mesh.name.clone()));
                }
                let primitive = &mesh.primitive;
                let mut item = Json::object();
                if primitive.ngon_encoded {
                    let mut extension = Json::object();
                    extension.add("FB_ngon_encoding", Json::object());
                    item.add("extensions", extension);
                }
                item.add("mode", Json::Int(primitive.mode));
                if !scene.materials.is_empty() {
                    item.add("material", Json::Uint(primitive.material as u64));
                }
                if let Some(indices) = primitive.indices {
                    item.add("indices", Json::Uint(indices as u64));
                }
                let mut attributes = Json::object();
                if primitive.position.len() == 1 {
                    attributes.add("POSITION", Json::Uint(primitive.position[0] as u64));
                }
                if primitive.normal.len() == 1 {
                    attributes.add("NORMAL", Json::Uint(primitive.normal[0] as u64));
                }
                for (index, accessor) in primitive.texcoord.iter().enumerate() {
                    attributes.add(&format!("TEXCOORD_{index}"), Json::Uint(*accessor as u64));
                }
                item.add("attributes", attributes);
                object.add("primitives", Json::Array(vec![item]));
                object
            })
            .collect();
        document.add("meshes", Json::Array(objects));
    }

    let mut node = Json::object();
    if !scene.root_name.is_empty() {
        node.add("name", Json::String(scene.root_name.clone()));
    }
    if let Some(&first) = scene.root_meshes.first() {
        node.add("mesh", Json::Uint(first as u64));
    }
    document.add("nodes", Json::Array(vec![node]));
    let mut default_scene = Json::object();
    default_scene.add("nodes", Json::Array(vec![Json::Uint(0)]));
    document.add("scenes", Json::Array(vec![default_scene]));
    document.add("scene", Json::Uint(0));

    let io = |error: std::io::Error| ExportError(error.to_string());
    if binary {
        if !asset.body.is_empty() {
            let mut body = Json::object();
            body.add("byteLength", Json::Uint(asset.body.len() as u64));
            if let Some(Json::Array(buffers)) = document.member_mut("buffers") {
                buffers.push(body);
            }
        }
        let json = write_compact(&document);
        let json_length = (json.len() + 3) & !3;
        let body_length = (asset.body.len() + 3) & !3;
        let chunk_count = if asset.body.is_empty() { 1 } else { 2 };
        let total = 12 + chunk_count * 8 + json_length + if asset.body.is_empty() { 0 } else { body_length };
        let mut file = File::create(path).map_err(io)?;
        let mut bytes = Vec::with_capacity(total);
        bytes.extend_from_slice(b"glTF");
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&(total as u32).to_le_bytes());
        bytes.extend_from_slice(&(json_length as u32).to_le_bytes());
        bytes.extend_from_slice(b"JSON");
        bytes.extend_from_slice(json.as_bytes());
        bytes.resize(bytes.len() + json_length - json.len(), b' ');
        if !asset.body.is_empty() {
            bytes.extend_from_slice(&(body_length as u32).to_le_bytes());
            bytes.extend_from_slice(b"BIN\0");
            bytes.extend_from_slice(&asset.body);
            bytes.resize(bytes.len() + body_length - asset.body.len(), b' ');
        }
        file.write_all(&bytes).map_err(io)?;
        return Ok(());
    }
    let json = write_pretty(&document).replace('\n', "\r\n");
    std::fs::write(path, json.as_bytes()).map_err(io)?;
    std::fs::write(&bin_path, &asset.body).map_err(io)?;
    Ok(())
}
