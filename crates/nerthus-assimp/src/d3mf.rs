//! 3D Manufacturing Format packages (`D3MFImporter`): a zip archive whose
//! `_rels/.rels` names the model part, an XML `<model>` of resources
//! (objects with meshes and components, base materials, texture
//! coordinate and colour groups) and the build items placing objects.
//!
//! Every object mesh becomes a mesh, in reading order, once the model has
//! a `<build>`. A triangle's `pid`/`p1`…`p3` and an object's
//! `pid`/`pindex` give vertices texture coordinates or colours from a
//! group, into arrays sized by the vertex count at that moment; indices
//! into groups, base materials and those arrays go unchecked (undefined
//! behaviour here, errors). An object whose id is taken leaves its meshes
//! out of the scene's list: a null mesh.

use std::collections::BTreeMap;

use crate::parsing::{atoi, is_c_space, strtod};
use crate::registry::name_extension;
use crate::scene::{ImportError, Mesh, PRIMITIVE_TRIANGLE, Scene, fail};
use crate::xml::{self, Node as XmlNode};
use crate::zip::Archive;

/// `IdNotSet`.
const ID_NOT_SET: i32 = -1;

const ROOT_RELATIONSHIPS: &[u8] = b"_rels/.rels";
const MODEL_RELATIONSHIP_TYPE: &[u8] = b"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel";

/// Object graphs (components) deeper than this overflow the stack of
/// `addObjectToNode`; a cycle never ends.
const MAX_DEPTH: usize = 2048;

/// More nodes than this run out of memory.
const MAX_NODES: u64 = 1 << 24;

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// `atoi` of an attribute (`getNodeAttribute` into an `int`), if present.
fn atoi_attribute(node: &XmlNode, name: &str) -> Option<i32> {
    node.attribute(name).map(|value| atoi(value, 0))
}

/// `getIntAttribute` (pugixml's `as_int`), if present.
fn int_attribute(node: &XmlNode, name: &str) -> Option<i32> {
    node.attribute(name).map(xml::as_int)
}

/// `strtol(text, nullptr, 16)` on a two-character string.
fn strtol_hex(text: &[u8]) -> i64 {
    let mut index = 0;
    while index < text.len() && is_c_space(text[index]) {
        index += 1;
    }
    let negative = text.get(index) == Some(&b'-');
    if matches!(text.get(index), Some(b'+' | b'-')) {
        index += 1;
    }
    let mut value = 0i64;
    while let Some(digit) = text.get(index).and_then(|&byte| (byte as char).to_digit(16)) {
        value = value * 16 + digit as i64;
        index += 1;
    }
    if negative { -value } else { value }
}

/// `parseColor`: "#RRGGBB" or "#RRGGBBAA" (alpha 0 without it), each
/// channel `strtol` over 255.
fn parse_color(text: &[u8]) -> Option<[f32; 4]> {
    if text.len() != 7 && text.len() != 9 {
        return None;
    }
    if text[0] != b'#' {
        return None;
    }
    let channel = |first: usize| strtol_hex(&text[first..first + 2]) as f32 / 255.0;
    let mut color = [channel(1), channel(3), channel(5), 0.0];
    if text.len() == 9 {
        color[3] = channel(7);
    }
    Some(color)
}

/// `parseTransformMatrix`: numbers split at spaces, each `std::stof`
/// (none read, or out of float's range, throws); the first twelve are
/// taken unchecked.
fn parse_transform(text: &[u8]) -> Result<(), ImportError> {
    let mut count = 0;
    for token in text.split(|&byte| byte == b' ') {
        if token.is_empty() {
            continue;
        }
        stof(token)?;
        count += 1;
    }
    if count < 12 {
        return undefined("a transform of fewer than 12 numbers");
    }
    Ok(())
}

/// `std::stof`: `strtof`, which must read something and stay within
/// float's range.
fn stof(token: &[u8]) -> Result<(), ImportError> {
    let mut start = 0;
    while start < token.len() && is_c_space(token[start]) {
        start += 1;
    }
    let rest = &token[start..];
    let sign = usize::from(matches!(rest.first(), Some(b'+' | b'-')));
    let after_sign = &rest[sign..];
    let starts_number = after_sign.first().is_some_and(u8::is_ascii_digit)
        || (after_sign.first() == Some(&b'.') && after_sign.get(1).is_some_and(u8::is_ascii_digit))
        || after_sign.len() >= 3 && (after_sign[..3].eq_ignore_ascii_case(b"inf") || after_sign[..3].eq_ignore_ascii_case(b"nan"));
    if !starts_number {
        return fail("invalid stof argument");
    }
    let value = strtod(token);
    let infinite_text = after_sign[..3.min(after_sign.len())].eq_ignore_ascii_case(b"inf");
    let overflow = (value as f32).is_infinite() && !infinite_text;
    let magnitude = value.abs();
    let underflow = magnitude != 0.0 && magnitude < f32::MIN_POSITIVE as f64;
    if overflow || underflow {
        return fail("stof argument out of range");
    }
    Ok(())
}

/// A mesh as read: its streams, the texture coordinate and colour arrays
/// as long as they were made.
#[derive(Default)]
struct MeshBuild {
    name: String,
    positions: Vec<[f32; 3]>,
    faces: Vec<[u32; 3]>,
    has_faces: bool,
    texture_coords: Option<Vec<[f32; 3]>>,
    colors: Option<Vec<[f32; 4]>>,
}

/// `D3MF::Resource`s by id.
enum Resource {
    Object { components: Vec<i32> },
    BaseMaterials { count: usize },
    TextureGroup { coordinates: Vec<[f32; 2]> },
    ColorGroup { colors: Vec<[f32; 4]> },
}

/// `XmlSerializer`.
#[derive(Default)]
struct Serializer {
    resources: BTreeMap<i32, Resource>,
    /// Meshes by their index; those of objects left out stay `None`.
    meshes: Vec<Option<MeshBuild>>,
}

impl Serializer {
    /// `mResourcesDictionnary.insert`: an id already taken keeps its
    /// resource.
    fn insert(&mut self, id: i32, resource: Resource) -> bool {
        if self.resources.contains_key(&id) {
            return false;
        }
        self.resources.insert(id, resource);
        true
    }

    /// `ImportVertices`: the vertex list replaced.
    fn import_vertices(node: &XmlNode, mesh: &mut MeshBuild) {
        let mut positions = Vec::new();
        for child in &node.children {
            if child.name() != b"vertex" {
                continue;
            }
            let coordinate = |name: &str| strtod(child.attribute(name).unwrap_or_default()) as f32;
            positions.push([coordinate("x"), coordinate("y"), coordinate("z")]);
        }
        mesh.positions = positions;
    }

    /// `ImportTriangles`: the faces replaced. A triangle with a `pid` and
    /// a `p1`…`p3` sets the mesh's material, or its corners' texture
    /// coordinates or colours (arrays made the size of the vertex list
    /// now).
    fn import_triangles(&self, node: &XmlNode, mesh: &mut MeshBuild) -> Result<(), ImportError> {
        let mut faces = Vec::new();
        for child in &node.children {
            if child.name() != b"triangle" {
                continue;
            }
            let pid = atoi_attribute(child, "pid");
            let corner = |name: &str| atoi(child.attribute(name).unwrap_or_default(), 0) as u32;
            let corners = [corner("v1"), corner("v2"), corner("v3")];
            let property = |name: &str| int_attribute(child, name).unwrap_or(ID_NOT_SET);
            let indices = [property("p1"), property("p2"), property("p3")];
            faces.push(corners);
            let Some(pid) = pid else {
                continue;
            };
            if indices.iter().all(|&index| index == ID_NOT_SET) {
                continue;
            }
            match self.resources.get(&pid) {
                Some(Resource::BaseMaterials { count }) => {
                    for &index in &indices {
                        if index != ID_NOT_SET && (index < 0 || index as usize >= *count) {
                            return undefined("a base material index past the materials");
                        }
                    }
                }
                Some(Resource::TextureGroup { coordinates }) => {
                    let vertex_count = mesh.positions.len();
                    let texture_coords = mesh.texture_coords.get_or_insert_with(|| vec![[0.0, 0.0, ID_NOT_SET as f32]; vertex_count]);
                    for (slot, &index) in indices.iter().enumerate() {
                        if index == ID_NOT_SET {
                            continue;
                        }
                        let Some(coordinate) = usize::try_from(index).ok().and_then(|index| coordinates.get(index)) else {
                            return undefined("a texture coordinate index past the group");
                        };
                        let Some(target) = texture_coords.get_mut(corners[slot] as usize) else {
                            return undefined("a texture coordinate written past its array");
                        };
                        *target = [coordinate[0], coordinate[1], 0.0];
                    }
                }
                Some(Resource::ColorGroup { colors }) => {
                    let vertex_count = mesh.positions.len();
                    let mesh_colors = mesh.colors.get_or_insert_with(|| vec![[0.0; 4]; vertex_count]);
                    for (slot, &index) in indices.iter().enumerate() {
                        if index == ID_NOT_SET {
                            continue;
                        }
                        let Some(color) = usize::try_from(index).ok().and_then(|index| colors.get(index)) else {
                            return undefined("a colour index past the group");
                        };
                        let Some(target) = mesh_colors.get_mut(corners[slot] as usize) else {
                            return undefined("a colour written past its array");
                        };
                        *target = *color;
                    }
                }
                _ => {}
            }
        }
        mesh.faces = faces;
        mesh.has_faces = true;
        Ok(())
    }

    /// `ReadMesh` (`<vertices>` and `<triangles>` in any order and number).
    fn read_mesh(&self, node: &XmlNode) -> Result<MeshBuild, ImportError> {
        let mut mesh = MeshBuild::default();
        for child in &node.children {
            match child.name() {
                b"vertices" => Self::import_vertices(child, &mut mesh),
                b"triangles" => self.import_triangles(child, &mut mesh)?,
                _ => {}
            }
        }
        Ok(mesh)
    }

    /// An object's `pid` and `pindex` applied to its mesh: the material,
    /// texture coordinates for every vertex (or those still unset), or
    /// colours for every vertex (when it has none yet).
    fn apply_object_property(&self, pid: i32, pindex: i32, mesh: &mut MeshBuild) -> Result<(), ImportError> {
        let vertex_count = mesh.positions.len();
        let index = usize::try_from(pindex).ok();
        match self.resources.get(&pid) {
            Some(Resource::BaseMaterials { count }) => {
                if index.is_none_or(|index| index >= *count) {
                    return undefined("a base material index past the materials");
                }
            }
            Some(Resource::TextureGroup { coordinates }) => {
                let coordinate = index.and_then(|index| coordinates.get(index));
                match mesh.texture_coords.as_mut() {
                    None => {
                        if vertex_count == 0 {
                            mesh.texture_coords = Some(Vec::new());
                            return Ok(());
                        }
                        let Some(coordinate) = coordinate else {
                            return undefined("a texture coordinate index past the group");
                        };
                        mesh.texture_coords = Some(vec![[coordinate[0], coordinate[1], 0.0]; vertex_count]);
                    }
                    Some(texture_coords) => {
                        if texture_coords.len() < vertex_count {
                            return undefined("texture coordinates read past their array");
                        }
                        for value in texture_coords.iter_mut().take(vertex_count) {
                            if value[2] >= 0.0 {
                                continue;
                            }
                            let Some(coordinate) = coordinate else {
                                return undefined("a texture coordinate index past the group");
                            };
                            *value = [coordinate[0], coordinate[1], 0.0];
                        }
                    }
                }
            }
            Some(Resource::ColorGroup { colors }) if mesh.colors.is_none() => {
                let color = index.and_then(|index| colors.get(index));
                let Some(color) = color.copied().or((vertex_count == 0).then_some([0.0; 4])) else {
                    return undefined("a colour index past the group");
                };
                mesh.colors = Some(vec![color; vertex_count]);
            }
            _ => {}
        }
        Ok(())
    }

    /// `ReadObject`: needs an id; its meshes (each counted, whether or not
    /// the object makes it into the dictionary) and components.
    fn read_object(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        let Some(id) = atoi_attribute(node, "id") else {
            return Ok(());
        };
        let pid = atoi_attribute(node, "pid");
        let pindex = atoi_attribute(node, "pindex");
        let mut components = Vec::new();
        let mut mesh_indices = Vec::new();
        for child in &node.children {
            match child.name() {
                b"mesh" => {
                    let mut mesh = self.read_mesh(child)?;
                    mesh.name = id.to_string();
                    if let (Some(pid), Some(pindex)) = (pid, pindex) {
                        self.apply_object_property(pid, pindex, &mut mesh)?;
                    }
                    mesh_indices.push(self.meshes.len());
                    self.meshes.push(Some(mesh));
                }
                b"components" => {
                    for component in &child.children {
                        if component.name() != b"component" {
                            continue;
                        }
                        if let Some(transform) = component.attribute("transform") {
                            parse_transform(transform)?;
                        }
                        if let Some(object_id) = atoi_attribute(component, "objectid") {
                            components.push(object_id);
                        }
                    }
                }
                _ => {}
            }
        }
        if !self.insert(id, Resource::Object { components }) {
            for index in mesh_indices {
                self.meshes[index] = None;
            }
        }
        Ok(())
    }

    fn read_base_materials(&mut self, node: &XmlNode) {
        let Some(id) = atoi_attribute(node, "id") else {
            return;
        };
        let count = node.children.iter().filter(|child| child.name() == b"base").count();
        self.insert(id, Resource::BaseMaterials { count });
    }

    /// `ReadTextureGroup`: `<m:tex2coord u v>` entries (a missing value
    /// repeats the last one read).
    fn read_texture_group(&mut self, node: &XmlNode) {
        let Some(id) = int_attribute(node, "id") else {
            return;
        };
        let mut coordinates = Vec::new();
        let mut value = 0.0f64;
        for child in &node.children {
            if child.name() != b"m:tex2coord" {
                continue;
            }
            if let Some(u) = child.attribute("u") {
                value = strtod(u);
            }
            let x = value as f32;
            if let Some(v) = child.attribute("v") {
                value = strtod(v);
            }
            let y = value as f32;
            coordinates.push([x, y]);
        }
        self.insert(id, Resource::TextureGroup { coordinates });
    }

    /// `ReadColorGroup`: the `<m:color color="#…">` entries that parse.
    fn read_color_group(&mut self, node: &XmlNode) {
        let Some(id) = int_attribute(node, "id") else {
            return;
        };
        let mut colors = Vec::new();
        for child in &node.children {
            if child.name() != b"m:color" {
                continue;
            }
            if let Some(color) = parse_color(child.attribute("color").unwrap_or_default()) {
                colors.push(color);
            }
        }
        self.insert(id, Resource::ColorGroup { colors });
    }

    /// `addObjectToNode` from a build item: the object and, depth first,
    /// the objects its components name.
    fn place_object(&self, object_id: i32) -> Result<(), ImportError> {
        let mut pending = vec![(object_id, 1usize)];
        let mut nodes = 0u64;
        while let Some((id, depth)) = pending.pop() {
            if depth > MAX_DEPTH {
                return undefined("an object graph too deep for the stack");
            }
            nodes += 1;
            if nodes > MAX_NODES {
                return fail("Internal error");
            }
            let Some(Resource::Object { components }) = self.resources.get(&id) else {
                continue;
            };
            for &component in components.iter().rev() {
                if matches!(self.resources.get(&component), Some(Resource::Object { .. })) {
                    pending.push((component, depth + 1));
                }
            }
        }
        Ok(())
    }

    /// `ImportXml`: the resources, then (only with a `<build>`) the items
    /// and the meshes.
    fn import(&mut self, document: &XmlNode) -> Result<Vec<Mesh>, ImportError> {
        let Some(model) = document.child("model") else {
            return Ok(Vec::new());
        };
        if let Some(resources) = model.child("resources") {
            for child in &resources.children {
                match child.name() {
                    b"m:texture2dgroup" => self.read_texture_group(child),
                    b"object" => self.read_object(child)?,
                    b"basematerials" => self.read_base_materials(child),
                    b"m:colorgroup" => self.read_color_group(child),
                    _ => {}
                }
            }
        }
        let Some(build) = model.child("build") else {
            return Ok(Vec::new());
        };
        for item in &build.children {
            if item.name() != b"item" {
                continue;
            }
            let object_id = atoi_attribute(item, "objectid").unwrap_or(ID_NOT_SET);
            if !matches!(self.resources.get(&object_id), Some(Resource::Object { .. })) {
                continue;
            }
            if let Some(transform) = item.attribute("transform") {
                parse_transform(transform)?;
            }
            self.place_object(object_id)?;
        }

        let mut meshes = Vec::with_capacity(self.meshes.len());
        for slot in std::mem::take(&mut self.meshes) {
            let Some(build) = slot else {
                return undefined("a null mesh (its object's id was taken)");
            };
            meshes.push(output_mesh(build)?);
        }
        Ok(meshes)
    }
}

/// The scene's mesh. Texture coordinates shorter than the vertices are
/// written past their end when the preprocessing zeroes their third
/// components.
fn output_mesh(build: MeshBuild) -> Result<Mesh, ImportError> {
    let vertex_count = build.positions.len();
    let mut mesh = Mesh { name: build.name, vertex_count, vertices: Some(build.positions), ..Mesh::default() };
    if build.has_faces {
        mesh.primitive_types = PRIMITIVE_TRIANGLE;
        mesh.faces = Some(build.faces.iter().map(|corners| corners.to_vec()).collect());
    }
    if let Some(texture_coords) = build.texture_coords {
        if texture_coords.len() < vertex_count {
            return undefined("texture coordinates shorter than the vertices");
        }
        mesh.texture_coords[0] = Some(texture_coords);
        mesh.uv_components[0] = 2;
    }
    mesh.colors[0] = build.colors;
    Ok(mesh)
}

/// `OpcPackageRelationshipReader` and `ReadPackageRootRelationship`: the
/// target of the first relationship of the model type (with an id, type
/// and target), or "" when the part is no XML.
fn root_relationship(bytes: &[u8]) -> Result<Vec<u8>, ImportError> {
    let Ok(document) = xml::parse(bytes) else {
        return Ok(Vec::new());
    };
    for relationships in &document.children {
        if relationships.name() != b"Relationships" {
            continue;
        }
        for relationship in &relationships.children {
            if relationship.name() != b"Relationship" {
                continue;
            }
            let id = relationship.attribute("Id").unwrap_or_default();
            let kind = relationship.attribute("Type").unwrap_or_default();
            let target = relationship.attribute("Target").unwrap_or_default();
            if id.is_empty() || kind.is_empty() || target.is_empty() {
                continue;
            }
            if kind == MODEL_RELATIONSHIP_TYPE {
                return Ok(target.to_vec());
            }
        }
    }
    fail(format!("Cannot find {}", lossy(MODEL_RELATIONSHIP_TYPE)))
}

/// `IsEmbeddedTexture`: a .jpg, .png or .jpeg not named "thumbnail".
fn is_embedded_texture(name: &[u8]) -> bool {
    let extension = name_extension(name);
    let image = matches!(extension.as_slice(), b"jpg" | b"png" | b"jpeg");
    image && !name.windows(9).any(|window| window == b"thumbnail")
}

/// `D3MFImporter::InternReadFile`: the archive's files in name order (the
/// relationships naming the root part, embedded textures read), then the
/// root part (no XML: nothing).
pub fn read(path: &str, bytes: &[u8]) -> Result<Scene, ImportError> {
    let Some(archive) = Archive::open(bytes)? else {
        return fail(format!("Failed to open file {path}."));
    };
    let mut root = None;
    for name in archive.names() {
        if name == ROOT_RELATIONSHIPS {
            let Some(relationships) = archive.read(&name)? else {
                continue;
            };
            let mut root_file = root_relationship(&relationships)?;
            if root_file.first() == Some(&b'/') {
                root_file.remove(0);
                if root_file.first() == Some(&b'/') {
                    root_file.remove(0);
                }
            }
            let Some(part) = archive.read(&root_file)? else {
                return fail(format!("Cannot open root-file in archive : {}", lossy(&root_file)));
            };
            root = Some(part);
        } else if is_embedded_texture(&name) {
            archive.read(&name)?;
        }
    }

    let Some(root) = root else {
        return Ok(Scene::default());
    };
    let Ok(document) = xml::parse(&root) else {
        return Ok(Scene::default());
    };
    let mut serializer = Serializer::default();
    let meshes = serializer.import(&document)?;
    Ok(Scene { meshes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_and_transforms() {
        assert_eq!(parse_color(b"#ff8000"), Some([1.0, 128.0 / 255.0, 0.0, 0.0]));
        assert_eq!(parse_color(b"#0000ff80").map(|color| color[3]), Some(128.0 / 255.0));
        assert_eq!(parse_color(b"#-f0000"), Some([-15.0 / 255.0, 0.0, 0.0, 0.0]));
        assert_eq!(parse_color(b"ff8000"), None);
        assert!(parse_transform(b"1 0 0 0 1 0 0 0 1 0 0 0").is_ok());
        assert!(parse_transform(b"1 0 0 0 1 0 0 0 1 0 0").is_err());
        assert!(parse_transform(b"1 0 0 0 1 0 0 0 1 0 0 x").is_err());
        assert!(parse_transform(b"1  0 0 0 1 0 0 0 1 0 0 1e39").is_err());
    }
}
