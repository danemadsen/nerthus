//! Open Game Engine Exchange files (`OpenGEXImporter`): the OpenDDL tree
//! walked from the root, a mesh per `Mesh` structure.
//!
//! Structure types are matched as prefixes of the grammar's names, the
//! first match winning ("Material" is a prefix of "MaterialRef", so
//! materials are never read). Vertex arrays fill one container shared by
//! every mesh; an index array makes a corner of its own for each index,
//! three per index list, from whatever the container holds. A failed parse
//! leaves an empty scene.

mod ddl;

use ddl::{DataList, Node, Property, ROOT, Value, ValueKind};

use crate::batch::{Graph, LoadedScene, SceneGraph};
use crate::matrix::Matrix4;
use crate::node::Node as SceneNode;
use crate::parsing::text_file_to_buffer;
use crate::scene::{ImportError, Mesh, PRIMITIVE_LINE, PRIMITIVE_POINT, PRIMITIVE_POLYGON, PRIMITIVE_TRIANGLE, Scene, check_allocation, fail};
use crate::vector::Vector3;

/// `sizeof(aiFace)`.
const FACE_SIZE: u64 = 16;

/// `sizeof(aiVector3D)`.
const VECTOR_SIZE: u64 = 12;

/// The metric keys `handleMetricNode` stores a value for.
const METRIC_KEYS: [&[u8]; 4] = [b"distance", b"angle", b"time", b"up"];

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// `Grammar::TokenType`, as far as the importer acts on it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Token {
    Metric,
    Name,
    ObjectRef,
    MaterialRef,
    GeometryNode,
    CameraNode,
    LightNode,
    GeometryObject,
    CameraObject,
    LightObject,
    Transform,
    Mesh,
    VertexArray,
    IndexArray,
    Color,
    Param,
    Texture,
    /// Metric keys, attenuations and unknown types: nothing happens.
    Ignored,
}

/// The grammar's names in `matchTokenType`'s order ("Material", which
/// never wins, left out).
const TOKENS: [(&[u8], Token); 19] = [
    (b"Metric", Token::Metric),
    (b"Name", Token::Name),
    (b"ObjectRef", Token::ObjectRef),
    (b"MaterialRef", Token::MaterialRef),
    (b"key", Token::Ignored),
    (b"GeometryNode", Token::GeometryNode),
    (b"CameraNode", Token::CameraNode),
    (b"LightNode", Token::LightNode),
    (b"GeometryObject", Token::GeometryObject),
    (b"CameraObject", Token::CameraObject),
    (b"LightObject", Token::LightObject),
    (b"Transform", Token::Transform),
    (b"Mesh", Token::Mesh),
    (b"VertexArray", Token::VertexArray),
    (b"IndexArray", Token::IndexArray),
    (b"Color", Token::Color),
    (b"Param", Token::Param),
    (b"Texture", Token::Texture),
    (b"Atten", Token::Ignored),
];

/// `matchTokenType`: the first grammar name the type is a prefix of (an
/// empty type matches "Metric").
fn match_token(kind: &[u8]) -> Token {
    for (name, token) in TOKENS {
        if name.starts_with(kind) {
            return token;
        }
    }
    Token::Ignored
}

/// `findPropertyByName`: the first property whose key starts with `name`
/// (`strncmp` over the name's length).
fn find_property<'a>(node: &'a Node, name: &[u8]) -> Result<Option<&'a Property>, ImportError> {
    for property in &node.properties {
        let Some(key) = &property.key else {
            return undefined("a property key without text");
        };
        if key.starts_with(name) {
            return Ok(Some(property));
        }
    }
    Ok(None)
}

/// `propId2StdString`: the property's key and, for a string value, the
/// value (a reference property has no value to look at).
fn property_strings(property: &Property) -> Result<(&[u8], &[u8]), ImportError> {
    let Some(key) = &property.key else {
        return undefined("a property key without text");
    };
    let Some(value) = &property.value else {
        return undefined("a reference property's value read");
    };
    let text = if value.kind == ValueKind::String { value.c_string()? } else { b"" };
    Ok((key.as_slice(), text))
}

/// `ASSIMP_strincmp(literal, value, strlen(literal)) == 0` over the value's
/// bytes as a C string.
fn starts_with_ignore_case(value: &Value, literal: &[u8]) -> Result<bool, ImportError> {
    let bytes = value.bytes();
    for (index, &expected) in literal.iter().enumerate() {
        let Some(&byte) = bytes.get(index) else {
            return undefined("a value without a NUL read as a string");
        };
        if byte.to_ascii_lowercase() != expected {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The number of lists `countDataArrayListItems` counts: all of them when
/// the first holds values, none otherwise.
fn list_count(lists: &[DataList]) -> usize {
    match lists.first() {
        Some(first) if !first.values.is_empty() => lists.len(),
        _ => 0,
    }
}

/// `fillVector3`: two or three values (another read past the second's
/// null successor).
fn vector3(values: &[Value]) -> Result<Vector3, ImportError> {
    let (Some(first), Some(second)) = (values.first(), values.get(1)) else {
        return undefined("a vector with fewer than two values");
    };
    let x = first.float()?;
    let y = second.float()?;
    let z = match values.get(2) {
        Some(third) => third.float()?,
        None => 0.0,
    };
    Ok([x, y, z])
}

/// `copyVectorArray`.
fn vectors(lists: &[DataList], count: usize) -> Result<Vec<Vector3>, ImportError> {
    let mut vectors = Vec::with_capacity(count);
    for list in &lists[..count] {
        vectors.push(vector3(&list.values)?);
    }
    Ok(vectors)
}

/// `fillColor4`: four values, else an error naming how many there were.
fn color4(values: &[Value]) -> Result<[f32; 4], ImportError> {
    let mut color = [0.0f32; 4];
    for (index, channel) in color.iter_mut().enumerate() {
        let Some(value) = values.get(index) else {
            if index == 0 {
                return undefined("a colour without values");
            }
            return fail(format!("OpenGEX: Not enough values to fill 4-element color, only {index}"));
        };
        *channel = value.float()?;
    }
    Ok(color)
}

/// `copyColor4DArray`: every colour from the first list (the loop never
/// moves on).
fn colors(lists: &[DataList], count: usize) -> Result<Vec<[f32; 4]>, ImportError> {
    let mut colors = Vec::with_capacity(count);
    for _ in 0..count {
        colors.push(color4(&lists[0].values)?);
    }
    Ok(colors)
}

/// `VertexContainer`: the last vertex arrays read, whatever mesh they
/// were in.
#[derive(Default)]
struct VertexContainer {
    positions: Vec<Vector3>,
    colors: Vec<[f32; 4]>,
    normals: Vec<Vector3>,
    texture_coords: Vec<Vector3>,
}

/// The importer's state as far as meshes and failures depend on it.
struct Importer<'a> {
    nodes: &'a [Node],
    /// `m_currentNode` is set (by any geometry, camera or light node).
    has_current_node: bool,
    /// `m_currentCamera` is set.
    has_camera: bool,
    current_mesh: Option<usize>,
    vertices: VertexContainer,
    meshes: Vec<Mesh>,
}

impl Importer<'_> {
    /// `handleNodes`.
    fn handle_nodes(&mut self, parent: usize, depth: usize) -> Result<(), ImportError> {
        if depth > ddl::MAX_DEPTH {
            return undefined("structures nested too deep for the stack");
        }
        let nodes = self.nodes;
        for &child in &nodes[parent].children {
            let node = &nodes[child];
            match match_token(&node.kind) {
                Token::Metric => self.handle_metric(node)?,
                Token::Name => self.handle_name(node)?,
                Token::ObjectRef | Token::MaterialRef => self.handle_reference(node)?,
                Token::GeometryNode | Token::LightNode => {
                    self.has_current_node = true;
                    self.handle_nodes(child, depth + 1)?;
                }
                Token::CameraNode => {
                    self.has_camera = true;
                    self.has_current_node = true;
                    self.handle_nodes(child, depth + 1)?;
                }
                Token::GeometryObject | Token::CameraObject => self.handle_nodes(child, depth + 1)?,
                Token::LightObject => {
                    // The light's type, read as a string.
                    if let Some(property) = find_property(node, b"type")?
                        && let Some(value) = &property.value
                    {
                        value.c_string()?;
                    }
                    self.handle_nodes(child, depth + 1)?;
                }
                Token::Transform => self.handle_transform(node)?,
                Token::Mesh => self.handle_mesh(node, child, depth)?,
                Token::VertexArray => self.handle_vertex_array(node)?,
                Token::IndexArray => self.handle_index_array(node)?,
                Token::Color => self.handle_color(node)?,
                Token::Param => self.handle_param(node)?,
                Token::Texture => self.handle_texture(node)?,
                Token::Ignored => {}
            }
        }
        Ok(())
    }

    /// `handleMetricNode` (for the root's children): a known key's value
    /// must be a float, int32 or string.
    fn handle_metric(&self, node: &Node) -> Result<(), ImportError> {
        if node.parent != Some(ROOT) {
            return Ok(());
        }
        for property in &node.properties {
            let Some(value) = &property.value else {
                return undefined("a reference property's value read");
            };
            if value.kind != ValueKind::String {
                continue;
            }
            let key = value.c_string()?;
            if !METRIC_KEYS.contains(&key) {
                continue;
            }
            let Some(metric) = node.values.first() else {
                continue;
            };
            if !matches!(metric.kind, ValueKind::Float | ValueKind::Int32 | ValueKind::String) {
                return fail("OpenGEX: invalid data type for Metric node.");
            }
        }
        Ok(())
    }

    /// `handleNameNode`.
    fn handle_name(&self, node: &Node) -> Result<(), ImportError> {
        if !self.has_current_node {
            return fail("No current node for name.");
        }
        if let Some(value) = node.values.first()
            && value.kind != ValueKind::String
        {
            return fail("OpenGEX: invalid data type for value in node name.");
        }
        Ok(())
    }

    /// `handleObjectRefNode` and `handleMaterialRefNode`: the names are
    /// copied as C strings.
    fn handle_reference(&self, node: &Node) -> Result<(), ImportError> {
        if !self.has_current_node {
            return fail("No parent node for name.");
        }
        if node.references.iter().any(Option::is_none) {
            return undefined("a reference name without text");
        }
        Ok(())
    }

    /// `handleTransformNode`: sixteen values in the first list, read as
    /// floats.
    fn handle_transform(&self, node: &Node) -> Result<(), ImportError> {
        if !self.has_current_node {
            return fail("No parent node for name.");
        }
        let Some(first) = node.data_lists.first() else {
            return Ok(());
        };
        if first.values.len() != 16 {
            return fail("Invalid number of data for transform matrix.");
        }
        for value in &first.values {
            value.float()?;
        }
        Ok(())
    }

    /// `handleMeshNode`: a new mesh (the current one from now on), its
    /// primitive type from a first property named "primitive".
    fn handle_mesh(&mut self, node: &Node, index: usize, depth: usize) -> Result<(), ImportError> {
        let mesh_index = self.meshes.len();
        self.meshes.push(Mesh::default());
        self.current_mesh = Some(mesh_index);
        if let Some(property) = node.properties.first() {
            let (name, value) = property_strings(property)?;
            if name == b"primitive" {
                let primitive_type = match value {
                    b"points" => PRIMITIVE_POINT,
                    b"lines" => PRIMITIVE_LINE,
                    b"triangles" => PRIMITIVE_TRIANGLE,
                    b"quads" => PRIMITIVE_POLYGON,
                    _ => 0,
                };
                self.meshes[mesh_index].primitive_types |= primitive_type;
            }
        }
        self.handle_nodes(index, depth + 1)
    }

    /// `handleVertexArrayNode`: the attribute named by the first property's
    /// value replaces the container's.
    fn handle_vertex_array(&mut self, node: &Node) -> Result<(), ImportError> {
        let Some(property) = node.properties.first() else {
            return Ok(());
        };
        let (_, attribute) = property_strings(property)?;
        if !matches!(attribute, b"position" | b"color" | b"normal" | b"texcoord") {
            return Ok(());
        }
        let lists = &node.data_lists;
        if lists.is_empty() {
            return Ok(());
        }
        let count = list_count(lists);
        match attribute {
            b"position" => self.vertices.positions = vectors(lists, count)?,
            b"color" => self.vertices.colors = colors(lists, count)?,
            b"normal" => self.vertices.normals = vectors(lists, count)?,
            _ => self.vertices.texture_coords = vectors(lists, count)?,
        }
        Ok(())
    }

    /// `handleIndexArrayNode`: the current mesh's faces and vertices from
    /// the first three indices of each list. Normals and texture
    /// coordinates (when the container has any) are allocated per corner,
    /// colours per colour; a stream left out keeps the mesh's old one.
    fn handle_index_array(&mut self, node: &Node) -> Result<(), ImportError> {
        let Some(mesh_index) = self.current_mesh else {
            return fail("No current mesh for index data found.");
        };
        let lists = &node.data_lists;
        if lists.is_empty() {
            return Ok(());
        }
        let face_count = list_count(lists);
        let vertex_count = face_count * 3;
        check_allocation(face_count as u64, FACE_SIZE)?;
        check_allocation(vertex_count as u64, VECTOR_SIZE)?;

        let container = &self.vertices;
        let mut positions = Vec::with_capacity(vertex_count);
        let mut colors = (!container.colors.is_empty()).then(|| vec![[0.0f32; 4]; container.colors.len()]);
        let mut normals = (!container.normals.is_empty()).then(|| Vec::with_capacity(vertex_count));
        let mut texture_coords = (!container.texture_coords.is_empty()).then(|| Vec::with_capacity(vertex_count));
        let mut faces = Vec::with_capacity(face_count);
        let mut corner = 0usize;
        for list in &lists[..face_count] {
            let mut face = Vec::with_capacity(3);
            for slot in 0..3 {
                let Some(value) = list.values.get(slot) else {
                    return undefined("an index list with fewer than three indices");
                };
                // (`const int idx`: an index past 2^31 is negative.)
                let index = value.unsigned_int32()? as i32;
                let Ok(index) = usize::try_from(index) else {
                    return undefined("a negative index");
                };
                let Some(&position) = container.positions.get(index) else {
                    return undefined("an index past the positions");
                };
                positions.push(position);
                if let Some(colors) = colors.as_mut() {
                    let (Some(&color), Some(target)) = (container.colors.get(index), colors.get_mut(corner)) else {
                        return undefined("an index or corner past the colours");
                    };
                    *target = color;
                }
                if let Some(normals) = normals.as_mut() {
                    let Some(&normal) = container.normals.get(index) else {
                        return undefined("an index past the normals");
                    };
                    normals.push(normal);
                }
                if let Some(texture_coords) = texture_coords.as_mut() {
                    let Some(&coordinate) = container.texture_coords.get(index) else {
                        return undefined("an index past the texture coordinates");
                    };
                    texture_coords.push(coordinate);
                }
                face.push(corner as u32);
                corner += 1;
            }
            faces.push(face);
        }

        let mesh = &mut self.meshes[mesh_index];
        mesh.vertex_count = vertex_count;
        mesh.vertices = Some(positions);
        mesh.faces = Some(faces);
        if let Some(colors) = colors {
            mesh.colors[0] = Some(colors);
        }
        if let Some(normals) = normals {
            mesh.normals = Some(normals);
        }
        if let Some(texture_coords) = texture_coords {
            mesh.texture_coords[0] = Some(texture_coords);
        }
        Ok(())
    }

    /// `handleColorNode`: three or four values read (the colour is never
    /// stored: the key it is typed by always starts with "attrib").
    fn handle_color(&self, node: &Node) -> Result<(), ImportError> {
        let Some(property) = find_property(node, b"attrib")? else {
            return Ok(());
        };
        if property.value.is_none() {
            return Ok(());
        }
        let Some(first) = node.data_lists.first() else {
            return Ok(());
        };
        let channels = if first.values.len() == 3 { 3 } else { 4 };
        for index in 0..channels {
            let Some(value) = first.values.get(index) else {
                return undefined("a colour with fewer than four values");
            };
            value.float()?;
        }
        Ok(())
    }

    /// `handleParamNode`: the value read as a float; "fov", "near" and
    /// "far" (any case, as prefixes) are stored in the current camera.
    fn handle_param(&self, node: &Node) -> Result<(), ImportError> {
        let Some(property) = find_property(node, b"attrib")? else {
            return Ok(());
        };
        let Some(attribute) = &property.value else {
            return Ok(());
        };
        let Some(value) = node.values.first() else {
            return Ok(());
        };
        value.float()?;
        let mut camera_parameter = false;
        for name in [&b"fov"[..], b"near", b"far"] {
            if starts_with_ignore_case(attribute, name)? {
                camera_parameter = true;
                break;
            }
        }
        if camera_parameter && !self.has_camera {
            return undefined("a camera parameter without a camera");
        }
        Ok(())
    }

    /// `handleTextureNode`: the value read as a string (its type is
    /// compared by pointer, so nothing is stored).
    fn handle_texture(&self, node: &Node) -> Result<(), ImportError> {
        let Some(property) = find_property(node, b"attrib")? else {
            return Ok(());
        };
        if property.value.is_none() {
            return Ok(());
        }
        if let Some(value) = node.values.first() {
            value.c_string()?;
        }
        Ok(())
    }
}

/// `OpenGEXImporter::InternReadFile`.
pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    let loaded = read_graph(bytes)?;
    Ok(Scene { meshes: loaded.meshes })
}

/// `InternReadFile`, with whether the scene got a root node: a file that
/// does not parse leaves it without one.
pub(crate) fn read_graph(bytes: &[u8]) -> Result<LoadedScene, ImportError> {
    let buffer = text_file_to_buffer(bytes, false)?;
    let Some(nodes) = ddl::parse(&buffer)? else {
        return Ok(LoadedScene { meshes: Vec::new(), graph: Graph::Missing, animations: Some(0) });
    };
    let mut importer = Importer {
        nodes: &nodes,
        has_current_node: false,
        has_camera: false,
        current_mesh: None,
        vertices: VertexContainer::default(),
        meshes: Vec::new(),
    };
    importer.handle_nodes(ROOT, 0)?;
    // A file without node structures leaves the root node alone; the
    // importer makes no animations.
    let graph = if importer.has_current_node {
        Graph::Unknown
    } else {
        let root = SceneNode::new(Vec::new(), Matrix4::IDENTITY, None);
        Graph::Known(SceneGraph { nodes: vec![root], root: 0 })
    };
    Ok(LoadedScene { meshes: importer.meshes, graph, animations: Some(0) })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRIANGLE: &str = "GeometryObject $g\n{\n\tMesh (primitive = \"triangles\")\n\t{\n\t\tVertexArray (attrib = \"position\") {float[3] {{0, 0, 0}, {1, 0, 0}, {0, 1, 0}}}\n\t\tIndexArray {unsigned_int32[3] {{0, 1, 2}}}\n\t}\n}\n";

    #[test]
    fn a_triangle() {
        let scene = read(TRIANGLE.as_bytes()).unwrap();
        assert_eq!(scene.meshes.len(), 1);
        let mesh = &scene.meshes[0];
        assert_eq!(mesh.primitive_types, PRIMITIVE_TRIANGLE);
        assert_eq!(mesh.vertices.as_ref().unwrap(), &vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        assert_eq!(mesh.faces.as_ref().unwrap(), &vec![vec![0, 1, 2]]);
    }

    #[test]
    fn token_prefixes() {
        assert!(match_token(b"") == Token::Metric);
        assert!(match_token(b"Material") == Token::MaterialRef);
        assert!(match_token(b"Geometry") == Token::GeometryNode);
        assert!(match_token(b"Te") == Token::Texture);
        assert!(match_token(b"MeshX") == Token::Ignored);
    }

    #[test]
    fn sixteen_bit_indices_are_undefined() {
        let source = TRIANGLE.replace("unsigned_int32", "unsigned_int16");
        assert!(read(source.as_bytes()).is_err());
    }

    #[test]
    fn a_failed_parse_is_an_empty_scene() {
        let source = TRIANGLE.replace("(primitive = \"triangles\")", "(primitive = \"triangles\", lod = 1)");
        let scene = read(source.as_bytes()).unwrap();
        assert!(scene.meshes.is_empty());
    }
}
