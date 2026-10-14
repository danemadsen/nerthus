//! The Additive Manufacturing File format (`AMFImporter`), XML: objects of
//! meshes (a vertex list and a volume of triangles), materials, grayscale
//! textures, constellations of object instances and metadata, first read
//! into a tree of elements.
//!
//! Each object's meshes become a mesh per texture set of their volume's
//! triangles: its vertices are those the triangles use, in index order,
//! coloured by the first of vertex, volume, object and material colour; a
//! triangle's own colour gives it corners of its own. Only a mesh's first
//! `<vertices>` and first `<volume>` are read, and texture coordinates come
//! from the `<texmap>`'s own text (its `<utex1>` … children only have to
//! be there).

use crate::parsing::{atoi, trim};
use crate::scene::{ImportError, Mesh, PRIMITIVE_TRIANGLE, Scene, check_allocation, fail};
use crate::xml::{self, Node as XmlNode};

/// The units `<amf unit="…">` may name (lowercased).
const UNITS: [&[u8]; 6] = [b"inch", b"millimeters", b"millimeter", b"meter", b"feet", b"micron"];

/// `AI_MAXLEN − 1`: the longest `aiString`.
const MAX_STRING: usize = 1023;

/// Node trees copied into constellations deeper than this overflow the
/// stack of the recursive copy.
const MAX_DEPTH: usize = 4096;

/// More copied nodes than this run out of memory.
const MAX_NODES: u64 = 1 << 24;

/// `std::vector::at` past the end (`std::out_of_range`).
fn out_of_range<T>() -> Result<T, ImportError> {
    fail("invalid vector subscript")
}

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Root,
    Object,
    Mesh,
    Vertices,
    Vertex,
    Coordinates,
    Volume,
    Triangle,
    Color,
    TexMap,
    Texture,
    Material,
    Constellation,
    Instance,
    Metadata,
}

#[derive(Clone, Debug, Default)]
enum Payload {
    #[default]
    None,
    Color([f32; 4]),
    Coordinates([f32; 3]),
    Volume {
        material_id: Vec<u8>,
    },
    Instance {
        object_id: Vec<u8>,
    },
    /// The corners as `atoi` read them (`None`: never set, uninitialised).
    Triangle {
        corners: [Option<u32>; 3],
    },
    TexMap {
        coordinates: [[f32; 3]; 3],
        ids: [Vec<u8>; 4],
    },
    Texture {
        width: u32,
        height: u32,
        depth: u32,
        size: usize,
    },
}

/// An `AMFNodeElementBase`.
struct Element {
    kind: Kind,
    id: Vec<u8>,
    parent: Option<usize>,
    children: Vec<usize>,
    payload: Payload,
}

/// The parse (`ParseNode_*`): elements in an arena, the current one, and
/// the textures in the order they were read (`mNodeElement_List`).
struct Parser {
    elements: Vec<Element>,
    current: usize,
    textures: Vec<usize>,
}

impl Parser {
    /// A new element whose parent is the current one (not yet one of its
    /// children).
    fn create(&mut self, kind: Kind, id: Vec<u8>, payload: Payload) -> usize {
        let parent = if self.elements.is_empty() { None } else { Some(self.current) };
        self.elements.push(Element { kind, id, parent, children: Vec::new(), payload });
        self.elements.len() - 1
    }

    /// `mNodeElement_Cur->Child.push_back(element)`.
    fn attach(&mut self, element: usize) {
        self.elements[self.current].children.push(element);
    }

    /// `ParseHelper_Node_Enter`.
    fn enter(&mut self, element: usize) {
        self.attach(element);
        self.current = element;
    }

    /// `ParseHelper_Node_Exit`.
    fn exit(&mut self) {
        if let Some(parent) = self.elements[self.current].parent {
            self.current = parent;
        }
    }

    /// `ParseNode_Root` on `<amf>`: its unit trimmed and lowercased
    /// (`ai_tolower` on a `std::string`).
    fn parse_root(&mut self, amf: &XmlNode) -> Result<usize, ImportError> {
        let unit = trim(amf.attribute("unit").unwrap_or_default()).to_ascii_lowercase();
        if !unit.is_empty() && !UNITS.contains(&unit.as_slice()) {
            return fail(format!("Attribute \"{}\" in node <unit> has incorrect value.", lossy(&unit)));
        }
        let root = self.create(Kind::Root, Vec::new(), Payload::None);
        self.current = root;
        for child in &amf.children {
            match child.name() {
                b"object" => self.parse_object(child)?,
                b"material" => self.parse_material(child)?,
                b"texture" => self.parse_texture(child)?,
                b"constellation" => self.parse_constellation(child)?,
                b"metadata" => self.parse_metadata(),
                _ => {}
            }
            self.current = root;
        }
        Ok(root)
    }

    fn parse_constellation(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        let id = node.attribute("id").unwrap_or_default().to_vec();
        let element = self.create(Kind::Constellation, id, Payload::None);
        self.enter(element);
        for child in &node.children {
            match child.name() {
                b"instance" => self.parse_instance(child)?,
                b"metadata" => self.parse_metadata(),
                _ => {}
            }
        }
        self.exit();
        Ok(())
    }

    /// `<instance objectid="…">` (its offsets and rotations place nodes
    /// only).
    fn parse_instance(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        let object_id = node.attribute("objectid").unwrap_or_default().to_vec();
        if object_id.is_empty() {
            return fail("\"objectid\" in <instance> must be defined.");
        }
        let element = self.create(Kind::Instance, Vec::new(), Payload::Instance { object_id });
        self.enter(element);
        self.exit();
        Ok(())
    }

    fn parse_object(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        let id = node.attribute("id").unwrap_or_default().to_vec();
        let element = self.create(Kind::Object, id, Payload::None);
        self.enter(element);
        for child in &node.children {
            match child.name() {
                b"color" => self.parse_color(child)?,
                b"mesh" => self.parse_mesh(child)?,
                b"metadata" => self.parse_metadata(),
                _ => {}
            }
        }
        self.exit();
        Ok(())
    }

    fn parse_metadata(&mut self) {
        let element = self.create(Kind::Metadata, Vec::new(), Payload::None);
        self.attach(element);
    }

    /// `<mesh>`: its first `<vertices>` and first `<volume>`; with neither,
    /// the mesh is its parent's child twice.
    fn parse_mesh(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        let element = self.create(Kind::Mesh, Vec::new(), Payload::None);
        self.enter(element);
        let vertices = node.child("vertices");
        if let Some(vertices) = vertices {
            self.parse_vertices(vertices)?;
        }
        let volume = node.child("volume");
        if let Some(volume) = volume {
            self.parse_volume(volume)?;
        }
        self.exit();
        if vertices.is_none() && volume.is_none() {
            self.attach(element);
        }
        Ok(())
    }

    fn parse_vertices(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        let element = self.create(Kind::Vertices, Vec::new(), Payload::None);
        self.enter(element);
        for child in &node.children {
            if child.name() == b"vertex" {
                self.parse_vertex(child)?;
            }
        }
        self.exit();
        Ok(())
    }

    /// `<vertex>`: its first `<color>` and first `<coordinates>`; with
    /// neither, the vertex is in the list twice.
    fn parse_vertex(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        let element = self.create(Kind::Vertex, Vec::new(), Payload::None);
        self.enter(element);
        let color = node.child("color");
        if let Some(color) = color {
            self.parse_color(color)?;
        }
        let coordinates = node.child("coordinates");
        if let Some(coordinates) = coordinates {
            self.parse_coordinates(coordinates);
        }
        self.exit();
        if color.is_none() && coordinates.is_none() {
            self.attach(element);
        }
        Ok(())
    }

    /// `<coordinates>`: `<x>`, `<y>` and `<z>`. (Their names go through
    /// `ai_tolower`, whose one-character template takes the `const char*`
    /// and returns it as it is: the case counts.)
    fn parse_coordinates(&mut self, node: &XmlNode) {
        let mut coordinate = [0.0f32; 3];
        for child in &node.children {
            let axis = match child.name() {
                b"x" => 0,
                b"y" => 1,
                b"z" => 2,
                _ => continue,
            };
            coordinate[axis] = xml::as_float(child.text());
        }
        let element = self.create(Kind::Coordinates, Vec::new(), Payload::Coordinates(coordinate));
        self.enter(element);
        self.exit();
    }

    /// `<volume materialid="…">`: one colour at most, triangles.
    fn parse_volume(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        let material_id = node.attribute("materialid").unwrap_or_default().to_vec();
        let element = self.create(Kind::Volume, Vec::new(), Payload::Volume { material_id });
        self.enter(element);
        let mut color_read = false;
        for child in &node.children {
            match child.name() {
                b"color" => {
                    if color_read {
                        return fail("\"color\" node can be used only once in color. Description: Only one color can be defined for <volume>.");
                    }
                    self.parse_color(child)?;
                    color_read = true;
                }
                b"triangle" => self.parse_triangle(child)?,
                b"metadata" | b"volume" => self.parse_metadata(),
                _ => {}
            }
        }
        self.exit();
        Ok(())
    }

    /// `<triangle>`: `<v1>` … `<v3>` through `atoi`, one colour at most,
    /// texture maps.
    fn parse_triangle(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        let element = self.create(Kind::Triangle, Vec::new(), Payload::None);
        self.enter(element);
        let mut corners: [Option<u32>; 3] = [None; 3];
        let mut color_read = false;
        for child in &node.children {
            match child.name() {
                b"color" => {
                    if color_read {
                        return fail("\"color\" node can be used only once in color. Description: Only one color can be defined for <triangle>.");
                    }
                    self.parse_color(child)?;
                    color_read = true;
                }
                b"texmap" => self.parse_texmap(child, false)?,
                b"map" => self.parse_texmap(child, true)?,
                b"v1" | b"v2" | b"v3" => {
                    let corner = (child.name()[1] - b'1') as usize;
                    let text = trim(child.text());
                    corners[corner] = Some(atoi(&text, 0) as u32);
                }
                _ => {}
            }
        }
        self.elements[element].payload = Payload::Triangle { corners };
        self.exit();
        Ok(())
    }

    /// `<color>`: `<r>`, `<g>`, `<b>` (all needed) and `<a>` (1 while
    /// unread).
    fn parse_color(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        let element = self.create(Kind::Color, Vec::new(), Payload::None);
        self.enter(element);
        let mut read = [false; 4];
        let mut color = [0.0f32; 4];
        for child in &node.children {
            let channel = match child.name() {
                b"r" => Some(0),
                b"g" => Some(1),
                b"b" => Some(2),
                b"a" => Some(3),
                _ => None,
            };
            if let Some(channel) = channel {
                read[channel] = true;
                color[channel] = xml::as_float(child.text());
            }
            if !read[3] {
                color[3] = 1.0;
            }
        }
        self.elements[element].payload = Payload::Color(color);
        self.exit();
        if !(read[0] && read[1] && read[2]) {
            return fail("Not all color components are defined.");
        }
        Ok(())
    }

    fn parse_material(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        let id = node.attribute("id").unwrap_or_default().to_vec();
        let element = self.create(Kind::Material, id, Payload::None);
        self.enter(element);
        for child in &node.children {
            match child.name() {
                b"color" => self.parse_color(child)?,
                b"metadata" => self.parse_metadata(),
                _ => {}
            }
        }
        self.exit();
        Ok(())
    }

    /// `<texture id width height depth type tiled>` with base64 grayscale
    /// texels. A missing depth comes from the data's size (the width ×
    /// height product is 32-bit).
    fn parse_texture(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        let id = node.attribute("id").unwrap_or_default().to_vec();
        let width = node.attribute("width").map_or(0, xml::as_uint);
        let height = node.attribute("height").map_or(0, xml::as_uint);
        let depth = node.attribute("depth").map_or(0, xml::as_uint);
        let kind = node.attribute("type").unwrap_or_default();
        let encoded = trim(node.text());
        if id.is_empty() {
            return fail("ID for texture must be defined.");
        }
        if width < 1 {
            return fail("Invalid width for texture.");
        }
        if height < 1 {
            return fail("Invalid height for texture.");
        }
        if kind != b"grayscale" {
            return fail("Invalid type for texture.");
        }
        if encoded.is_empty() {
            return fail("Texture data not defined.");
        }
        let data = decode_base64(&encoded)?;
        let area = width.wrapping_mul(height);
        let mut full_depth = depth;
        if depth == 0 {
            if area == 0 {
                return undefined("a texture's depth divided by a zero area");
            }
            full_depth = (data.len() / area as usize) as u32;
        }
        let expected = area.wrapping_mul(full_depth);
        if expected as usize != data.len() {
            return fail("Texture has incorrect data size.");
        }
        let payload = Payload::Texture { width, height, depth, size: data.len() };
        let element = self.create(Kind::Texture, id, payload);
        self.attach(element);
        self.textures.push(element);
        Ok(())
    }

    /// `<texmap>` (or the old `<map>`) with r/g/b/a texture ids. The new
    /// form needs `<utex1>` … `<vtex3>` children but takes every coordinate
    /// from its own text; the old one reads attributes "u", "u2", "u3",
    /// "v1", "v2", "v3" (v3 into the first corner) and is never attached.
    fn parse_texmap(&mut self, node: &XmlNode, old_names: bool) -> Result<(), ImportError> {
        let mut ids: [Vec<u8>; 4] = Default::default();
        for (name, value) in &node.attributes {
            let slot = match name.as_slice() {
                b"rtexid" => 0,
                b"gtexid" => 1,
                b"btexid" => 2,
                b"atexid" => 3,
                _ => continue,
            };
            ids[slot] = value.clone();
        }
        if ids[0].is_empty() && ids[1].is_empty() && ids[2].is_empty() {
            return fail("ParseNode_TexMap. At least one texture ID must be defined.");
        }
        if node.children.is_empty() {
            return fail("Invalid children definition.");
        }

        let mut read = [false; 6];
        let mut coordinates = [[0.0f32; 3]; 3];
        let mut element = None;
        if !old_names {
            let texmap = self.create(Kind::TexMap, Vec::new(), Payload::None);
            self.enter(texmap);
            let own = xml::as_float(node.text());
            for child in &node.children {
                let (flag, corner, axis) = match child.name() {
                    b"utex1" => (0, 0, 0),
                    b"utex2" => (1, 1, 0),
                    b"utex3" => (2, 2, 0),
                    b"vtex1" => (3, 0, 1),
                    b"vtex2" => (4, 1, 1),
                    b"vtex3" => (5, 2, 1),
                    _ => continue,
                };
                read[flag] = true;
                coordinates[corner][axis] = own;
            }
            self.exit();
            element = Some(texmap);
        } else {
            for (name, value) in &node.attributes {
                let (flag, corner, axis) = match name.as_slice() {
                    b"u" => (0, 0, 0),
                    b"u2" => (1, 1, 0),
                    b"u3" => (2, 2, 0),
                    b"v1" => (3, 0, 1),
                    b"v2" => (4, 1, 1),
                    b"v3" => (5, 0, 1),
                    _ => continue,
                };
                read[flag] = true;
                coordinates[corner][axis] = xml::as_float(value);
            }
        }
        if read.iter().any(|&flag| !flag) {
            return fail("Not all texture coordinates are defined.");
        }
        if let Some(texmap) = element {
            self.elements[texmap].payload = Payload::TexMap { coordinates, ids };
        }
        Ok(())
    }
}

/// `ParseHelper_Decode_Base64`: up to the first '=', other bytes skipped;
/// a last partial group decodes with padding read as index 0xFF.
fn decode_base64(input: &[u8]) -> Result<Vec<u8>, ImportError> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    if !input.len().is_multiple_of(4) {
        return fail("Base64-encoded data must have size multiply of four.");
    }
    let index_of = |byte: u8| ALPHABET.iter().position(|&letter| letter == byte).map_or(0xFF, |position| position as u8);
    let decode = |quad: [u8; 4]| -> [u8; 3] {
        let [a, b, c, d] = quad.map(u32::from);
        [((a << 2) + ((b & 0x30) >> 4)) as u8, (((b & 0x0F) << 4) + ((c & 0x3C) >> 2)) as u8, (((c & 0x03) << 6) + d) as u8]
    };

    let mut output = Vec::with_capacity(input.len() / 4 * 3);
    let mut quad = [0u8; 4];
    let mut count = 0;
    for &byte in input {
        if byte == b'=' {
            break;
        }
        if !(byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'/') {
            continue;
        }
        quad[count] = index_of(byte);
        count += 1;
        if count == 4 {
            output.extend_from_slice(&decode(quad));
            count = 0;
        }
    }
    if count != 0 {
        for slot in quad.iter_mut().skip(count) {
            *slot = index_of(0);
        }
        let bytes = decode(quad);
        output.extend_from_slice(&bytes[..count - 1]);
    }
    Ok(output)
}

/// A volume's triangle as `SComplexFace`: its corners, colour and texture
/// map elements.
#[derive(Clone)]
struct ComplexFace {
    corners: [u32; 3],
    color: Option<usize>,
    texmap: Option<usize>,
}

/// `SPP_Material`: its id and last colour (unset: an uninitialised
/// pointer).
struct ConvertedMaterial {
    id: Vec<u8>,
    color: Option<usize>,
}

/// A node of the output graph, as far as constellations need them: its
/// name (`aiString`), how many nodes its tree has, and how deep it is.
struct BuiltNode {
    name: Vec<u8>,
    size: u64,
    depth: usize,
}

/// An `aiString` set from a `std::string` (left empty when too long).
fn node_name(id: &[u8]) -> Vec<u8> {
    if id.len() > MAX_STRING {
        return Vec::new();
    }
    id.to_vec()
}

/// The scene build (`Postprocess_*`).
struct Builder<'a> {
    parser: &'a Parser,
    materials: Vec<ConvertedMaterial>,
    /// Converted texture ids ("r_g_b_a"), whose positions are material
    /// indices.
    textures: Vec<Vec<u8>>,
    meshes: Vec<Mesh>,
}

impl Builder<'_> {
    fn element(&self, index: usize) -> &Element {
        &self.parser.elements[index]
    }

    fn color_of(&self, element: usize) -> [f32; 4] {
        match self.element(element).payload {
            Payload::Color(color) => color,
            _ => [0.0; 4],
        }
    }

    /// `PostprocessHelper_CreateMeshDataArray`: the positions of the
    /// vertices with coordinates, and each vertex element's colour.
    fn mesh_data(&self, mesh: usize) -> (Vec<[f32; 3]>, Vec<Option<usize>>) {
        let vertices = self.element(mesh).children.iter().rev().find(|&&child| self.element(child).kind == Kind::Vertices);
        let Some(&vertices) = vertices else {
            return (Vec::new(), Vec::new());
        };
        let vertex_list = &self.element(vertices).children;
        let mut positions = Vec::with_capacity(vertex_list.len());
        let mut colors = vec![None; vertex_list.len()];
        let mut color_index = 0;
        for &vertex in vertex_list {
            if self.element(vertex).kind != Kind::Vertex {
                continue;
            }
            colors[color_index] = None;
            for &part in &self.element(vertex).children {
                match &self.element(part).payload {
                    Payload::Coordinates(coordinate) => positions.push(*coordinate),
                    Payload::Color(_) => colors[color_index] = Some(part),
                    _ => {}
                }
            }
            color_index += 1;
        }
        (positions, colors)
    }

    /// `Vertex_CalculateColor`: the vertex's colour, else the volume's,
    /// the object's, the material's (mid grey when all zero), else none.
    fn vertex_color(&self, index: usize, vertex_colors: &[Option<usize>], volume_color: Option<usize>, object_color: Option<usize>, material: Option<&ConvertedMaterial>) -> Result<[f32; 4], ImportError> {
        if let Some(&Some(color)) = vertex_colors.get(index) {
            return Ok(self.color_of(color));
        }
        if let Some(color) = volume_color {
            return Ok(self.color_of(color));
        }
        if let Some(color) = object_color {
            return Ok(self.color_of(color));
        }
        let Some(material) = material else {
            return Ok([0.0; 4]);
        };
        let Some(color) = material.color else {
            return undefined("a material without a colour");
        };
        let mut value = self.color_of(color);
        if value.iter().all(|&channel| channel == 0.0) {
            value = [0.5, 0.5, 0.5, 1.0];
        }
        Ok(value)
    }

    /// `PostprocessHelper_GetTextureID_Or_Create`: the converted texture
    /// for the ids, made when new: each named texture must exist and all
    /// have one size, and their data are interleaved (each read with
    /// `at`).
    fn texture_index(&mut self, ids: &[Vec<u8>; 4]) -> Result<u32, ImportError> {
        if ids.iter().all(|id| id.is_empty()) {
            return fail("PostprocessHelper_GetTextureID_Or_Create. At least one texture ID must be defined.");
        }
        let mut key = Vec::new();
        for (index, id) in ids.iter().enumerate() {
            if index != 0 {
                key.push(b'_');
            }
            key.extend_from_slice(id);
        }
        if let Some(position) = self.textures.iter().position(|existing| *existing == key) {
            return Ok(position as u32);
        }

        let mut found: [Option<usize>; 4] = [None; 4];
        for (slot, id) in ids.iter().enumerate() {
            if id.is_empty() {
                continue;
            }
            let texture = self.parser.textures.iter().find(|&&texture| self.element(texture).id == *id);
            let Some(&texture) = texture else {
                return fail(format!("Not found node with name \"{}\".", lossy(id)));
            };
            found[slot] = Some(texture);
        }
        let mut dimensions = Vec::new();
        let mut sizes = [0usize; 4];
        for (slot, texture) in found.iter().enumerate() {
            let Some(texture) = texture else {
                continue;
            };
            if let Payload::Texture { width, height, depth, size } = self.element(*texture).payload {
                dimensions.push((width, height, depth));
                sizes[slot] = size;
            }
        }
        if dimensions.windows(2).any(|pair| pair[0] != pair[1]) {
            return fail("PostprocessHelper_GetTextureID_Or_Create. Source texture must has the same size.");
        }

        let present = found.map(|texture| texture.is_some());
        let total: usize = sizes.iter().sum();
        check_allocation(total as u64, 1)?;
        let step = present.iter().filter(|&&is_present| is_present).count();
        let green_offset = present[0] as usize;
        let blue_offset = present[0] as usize + present[1] as usize;
        let offsets = [0, green_offset, blue_offset, step - 1];
        for slot in 0..4 {
            if !present[slot] {
                continue;
            }
            let offset = offsets[slot];
            let reads = if offset < total { (total - offset).div_ceil(step) } else { 0 };
            if reads > sizes[slot] {
                return out_of_range();
            }
        }
        self.textures.push(key);
        Ok((self.textures.len() - 1) as u32)
    }

    /// One mesh of `Postprocess_BuildMeshSet`: the faces' vertices in index
    /// order, coloured triangles' corners copied, then texture coordinates
    /// (a vertex met with another coordinate copied too).
    fn build_mesh(&mut self, faces: &[ComplexFace], positions: &[[f32; 3]], vertex_colors: &[Option<usize>], volume_color: Option<usize>, object_color: Option<usize>, material: Option<usize>) -> Result<(), ImportError> {
        let corner_limit = faces.len() * 6;
        let mut distinct: Vec<u32> = faces.iter().flat_map(|face| face.corners).collect();
        distinct.sort_unstable();
        distinct.dedup();

        let material = material.map(|index| &self.materials[index]);
        let mut vertices = Vec::with_capacity(distinct.len());
        let mut colors = Vec::with_capacity(distinct.len());
        for &index in &distinct {
            let Some(&position) = positions.get(index as usize) else {
                return out_of_range();
            };
            vertices.push(position);
            let color = self.vertex_color(index as usize, vertex_colors, volume_color, object_color, material)?;
            colors.push(color);
        }
        let mut corners: Vec<[u32; 3]> = Vec::with_capacity(faces.len());
        for face in faces {
            let renumbered = face.corners.map(|corner| distinct.partition_point(|&value| value < corner) as u32);
            corners.push(renumbered);
        }

        for (face, entry) in faces.iter().zip(corners.iter_mut()) {
            let Some(color) = face.color else {
                continue;
            };
            let face_color = self.color_of(color);
            for corner in entry.iter_mut() {
                let source = vertices[*corner as usize];
                vertices.push(source);
                colors.push(face_color);
                *corner = (vertices.len() - 1) as u32;
            }
        }

        let mut material_index = 0;
        let mut coordinates = None;
        if let Some(first_texmap) = faces[0].texmap {
            check_allocation(corner_limit as u64, 1)?;
            let mut used = vec![false; corner_limit];
            let ids = match &self.element(first_texmap).payload {
                Payload::TexMap { ids, .. } => ids.clone(),
                _ => Default::default(),
            };
            material_index = self.texture_index(&ids)?;
            check_allocation(corner_limit as u64, 12)?;
            let mut texture_coordinates = vec![[0.0f32; 3]; corner_limit];
            let mut next = vertices.len();
            for (face, entry) in faces.iter().zip(corners.iter_mut()) {
                let face_coordinates = match face.texmap.map(|texmap| &self.element(texmap).payload) {
                    Some(Payload::TexMap { coordinates, .. }) => *coordinates,
                    _ => [[0.0; 3]; 3],
                };
                for (corner, coordinate) in entry.iter_mut().zip(face_coordinates) {
                    let index = *corner as usize;
                    let Some(seen) = used.get_mut(index) else {
                        return undefined("a vertex past the texture coordinate flags");
                    };
                    if !*seen {
                        texture_coordinates[index] = coordinate;
                        *seen = true;
                    } else if texture_coordinates[index] != coordinate {
                        vertices.push(vertices[index]);
                        colors.push(colors[index]);
                        if next >= corner_limit {
                            return out_of_range();
                        }
                        texture_coordinates[next] = coordinate;
                        *corner = next as u32;
                        next += 1;
                    }
                }
            }
            texture_coordinates.truncate(next);
            coordinates = Some(texture_coordinates);
        }

        let face_lists = corners.iter().map(|entry| entry.to_vec()).collect();
        let mut mesh = Mesh {
            primitive_types: PRIMITIVE_TRIANGLE,
            vertex_count: vertices.len(),
            vertices: Some(vertices),
            faces: Some(face_lists),
            material_index,
            ..Mesh::default()
        };
        mesh.colors[0] = Some(colors);
        if let Some(texture_coordinates) = coordinates {
            mesh.texture_coords[0] = Some(texture_coordinates);
            mesh.uv_components[0] = 2;
        }
        self.meshes.push(mesh);
        Ok(())
    }

    /// `Postprocess_BuildMeshSet`: each volume's triangles, split by their
    /// texture map ids (in order of first use), one mesh each.
    fn build_mesh_set(&mut self, mesh: usize, positions: &[[f32; 3]], vertex_colors: &[Option<usize>], object_color: Option<usize>) -> Result<(), ImportError> {
        let parser = self.parser;
        for &volume in &parser.elements[mesh].children {
            let Payload::Volume { material_id } = &parser.elements[volume].payload else {
                continue;
            };
            let mut material = None;
            if !material_id.is_empty() {
                let Some(found) = self.materials.iter().position(|converted| converted.id == *material_id) else {
                    return fail(format!("Not found node with name \"{}\".", lossy(material_id)));
                };
                material = Some(found);
            }

            let mut volume_color = None;
            let mut faces = Vec::new();
            for &child in &parser.elements[volume].children {
                match &parser.elements[child].payload {
                    Payload::Color(_) => volume_color = Some(child),
                    Payload::Triangle { corners } => {
                        let mut face = ComplexFace { corners: [0; 3], color: None, texmap: None };
                        for &part in &parser.elements[child].children {
                            match parser.elements[part].kind {
                                Kind::Color => face.color = Some(part),
                                Kind::TexMap => face.texmap = Some(part),
                                _ => {}
                            }
                        }
                        for (slot, corner) in corners.iter().enumerate() {
                            let Some(corner) = corner else {
                                return undefined("a triangle corner never given");
                            };
                            face.corners[slot] = *corner;
                        }
                        faces.push(face);
                    }
                    _ => {}
                }
            }

            for group in self.split_by_texture(faces) {
                self.build_mesh(&group, positions, vertex_colors, volume_color, object_color, material)?;
            }
        }
        Ok(())
    }

    /// The texture map ids of a face (none without a map).
    fn texture_ids(&self, face: &ComplexFace) -> Option<[Vec<u8>; 4]> {
        let texmap = face.texmap?;
        match &self.element(texmap).payload {
            Payload::TexMap { ids, .. } => Some(ids.clone()),
            _ => Some(Default::default()),
        }
    }

    /// `PostprocessHelper_SplitFacesByTextureID`: groups of faces with
    /// equal texture ids, by first face.
    fn split_by_texture(&self, mut faces: Vec<ComplexFace>) -> Vec<Vec<ComplexFace>> {
        let mut groups = Vec::new();
        while !faces.is_empty() {
            let first_ids = self.texture_ids(&faces[0]);
            let (group, rest): (Vec<ComplexFace>, Vec<ComplexFace>) = faces.into_iter().partition(|face| self.texture_ids(face) == first_ids);
            groups.push(group);
            faces = rest;
        }
        groups
    }

    /// `Postprocess_BuildNodeAndObject`: the colour so far applies to the
    /// meshes after it.
    fn build_object(&mut self, object: usize) -> Result<(), ImportError> {
        let parser = self.parser;
        let mut object_color = None;
        for &child in &parser.elements[object].children {
            match parser.elements[child].kind {
                Kind::Color => object_color = Some(child),
                Kind::Mesh => {
                    let (positions, colors) = self.mesh_data(child);
                    self.build_mesh_set(child, &positions, &colors, object_color)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// `Postprocess_BuildConstellation`: a node per instance, holding a
    /// copy of the object or constellation node it names.
    fn build_constellation(&self, constellation: usize, nodes: &mut Vec<BuiltNode>) -> Result<(), ImportError> {
        let mut size = 1u64;
        let mut depth = 1usize;
        let mut instances = 0;
        for &child in &self.element(constellation).children {
            let element = self.element(child);
            if element.kind == Kind::Metadata {
                continue;
            }
            let Payload::Instance { object_id } = &element.payload else {
                return fail("Only <instance> nodes can be in <constellation>.");
            };
            let key = &object_id[..object_id.len().min(MAX_STRING)];
            let Some(found) = nodes.iter().find(|node| node.name == key) else {
                return fail(format!("Not found node with name \"{}\".", lossy(object_id)));
            };
            size = size.saturating_add(1).saturating_add(found.size);
            depth = depth.max(found.depth + 2);
            instances += 1;
        }
        if instances == 0 {
            return fail("<constellation> must have at least one <instance>.");
        }
        if size > MAX_NODES {
            return fail("Internal error");
        }
        if depth > MAX_DEPTH {
            return undefined("a constellation too deep for the stack");
        }
        let name = node_name(&self.element(constellation).id);
        nodes.push(BuiltNode { name, size, depth });
        Ok(())
    }

    /// `Postprocess_BuildScene`: materials, then objects, then
    /// constellations, from the root's children.
    fn build(&mut self, root: usize) -> Result<(), ImportError> {
        let parser = self.parser;
        let children = &parser.elements[root].children;
        for &child in children {
            if parser.elements[child].kind != Kind::Material {
                continue;
            }
            let color = parser.elements[child].children.iter().rev().find(|&&part| parser.elements[part].kind == Kind::Color).copied();
            let id = parser.elements[child].id.clone();
            self.materials.push(ConvertedMaterial { id, color });
        }

        let mut nodes = Vec::new();
        for &child in children {
            if parser.elements[child].kind != Kind::Object {
                continue;
            }
            self.build_object(child)?;
            let name = node_name(&parser.elements[child].id);
            nodes.push(BuiltNode { name, size: 1, depth: 1 });
        }
        for &child in children {
            if parser.elements[child].kind == Kind::Constellation {
                self.build_constellation(child, &mut nodes)?;
            }
        }
        Ok(())
    }
}

/// `AMFImporter::InternReadFile`: the first element named "amf" anywhere
/// is the root.
pub fn read(path: &str, bytes: &[u8]) -> Result<Scene, ImportError> {
    let Ok(document) = xml::parse(bytes) else {
        return fail(format!("Failed to create XML reader for file {path}."));
    };
    let Some(amf) = document.find_node("amf") else {
        return fail("Root node \"amf\" not found.");
    };
    let mut parser = Parser { elements: Vec::new(), current: 0, textures: Vec::new() };
    let root = parser.parse_root(amf)?;
    let mut builder = Builder { parser: &parser, materials: Vec::new(), textures: Vec::new(), meshes: Vec::new() };
    builder.build(root)?;
    let meshes = builder.meshes;
    Ok(Scene { meshes })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUBE_CORNER: &str = r#"<?xml version="1.0"?>
<amf unit="millimeter">
  <object id="0">
    <mesh>
      <vertices>
        <vertex><coordinates><x>0</x><y>0</y><z>0</z></coordinates></vertex>
        <vertex><coordinates><x>9</x><y>9</y><z>9</z></coordinates></vertex>
        <vertex><coordinates><x>1</x><y>0</y><z>0</z></coordinates><color><r>1</r><g>0</g><b>0</b></color></vertex>
        <vertex><coordinates><x>0</x><y>1</y><z>0</z></coordinates></vertex>
      </vertices>
      <volume>
        <triangle><v1>0</v1><v2>2</v2><v3>3</v3></triangle>
        <triangle><v1>3</v1><v2>2</v2><v3>0</v3><color><r>0</r><g>0</g><b>1</b><a>0.5</a></color></triangle>
      </volume>
    </mesh>
  </object>
</amf>"#;

    #[test]
    fn used_vertices_in_index_order_and_triangle_colours() {
        let scene = read("cube.amf", CUBE_CORNER.as_bytes()).unwrap();
        assert_eq!(scene.meshes.len(), 1);
        let mesh = &scene.meshes[0];
        // Vertex 1 is unused; the coloured triangle has corners of its own.
        let expected = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 0.0]];
        assert_eq!(mesh.vertices, Some(expected));
        assert_eq!(mesh.faces, Some(vec![vec![0, 1, 2], vec![3, 4, 5]]));
        let colors = mesh.colors[0].as_ref().unwrap();
        assert_eq!(colors[1], [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(colors[0], [0.0; 4]);
        assert_eq!(colors[3], [0.0, 0.0, 1.0, 0.5]);
    }

    #[test]
    fn quirks_fail_as_assimp_fails() {
        // A colour without blue.
        let text = CUBE_CORNER.replace("<b>0</b></color></vertex>", "</color></vertex>");
        assert_eq!(read("a.amf", text.as_bytes()).unwrap_err().0, "Not all color components are defined.");
        // A triangle past the vertices.
        let text = CUBE_CORNER.replace("<v3>3</v3>", "<v3>7</v3>");
        assert!(read("a.amf", text.as_bytes()).is_err());
        // An unknown unit.
        let text = CUBE_CORNER.replace("millimeter", "Furlong");
        assert!(read("a.amf", text.as_bytes()).is_err());
    }

    #[test]
    fn base64_stops_at_padding() {
        assert_eq!(decode_base64(b"TWFu").unwrap(), b"Man");
        assert_eq!(decode_base64(b"TWE=").unwrap(), b"Ma");
        assert!(decode_base64(b"TWE").is_err());
    }
}
