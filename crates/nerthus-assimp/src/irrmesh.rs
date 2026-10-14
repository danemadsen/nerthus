//! Irrlicht's XML meshes (`IRRMeshImporter`): a `<mesh>` of buffers, each
//! a material, a vertex list (one vertex a line: position, normal, ARGB
//! colour, texture coordinate, and a second coordinate or a tangent and
//! bitangent by the list's type) and indices. Every index gets a vertex of
//! its own; an index past the vertices reads vertex 0.

use crate::fast_atof::{fast_atoreal_move, strtoul10, strtoul16};
use crate::parsing::{skip_line, skip_spaces, skip_spaces_and_line_end};
use crate::scene::{ImportError, Mesh, PRIMITIVE_TRIANGLE, Scene, check_allocation, fail};
use crate::vector::Vector3;
use crate::xml::{self, Node as XmlNode};

/// `AI_IRRMESH_MAT_*` and `AI_IRRMESH_EXTRA_2ND_TEXTURE`.
const MAT_TRANS_VERTEX_ALPHA: u32 = 0x1;
const MAT_LIGHTMAP: u32 = 0x2;
const MAT_NORMALMAP_SOLID: u32 = 0x100;
const MAT_SOLID_2LAYER: u32 = 0x10000;
const EXTRA_2ND_TEXTURE: u32 = 0x100000;

// Element sizes (MSVC, x64).
const VECTOR_SIZE: u64 = 12;
const COLOR_SIZE: u64 = 16;
const FACE_SIZE: u64 = 16;

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// The value of a property element's "name" and "value" attributes (in any
/// case; the last of each).
fn property(node: &XmlNode) -> (Vec<u8>, Option<Vec<u8>>) {
    let mut name = Vec::new();
    let mut value = None;
    for (key, text) in &node.attributes {
        if key.eq_ignore_ascii_case(b"name") {
            name = text.clone();
        } else if key.eq_ignore_ascii_case(b"value") {
            value = Some(text.clone());
        }
    }
    (name, value)
}

/// `IrrlichtBase::ParseMaterial`, as far as meshes need it: the material
/// flags (its type, and whether a second texture is in use). A `<float>`
/// property's value must read as a number.
pub(crate) fn parse_material(node: &XmlNode) -> Result<u32, ImportError> {
    let mut flags = 0u32;
    let mut textures = 0;
    for child in &node.children {
        let element = child.name();
        if element.eq_ignore_ascii_case(b"float") {
            for (key, text) in &child.attributes {
                if key.eq_ignore_ascii_case(b"value") {
                    fast_atoreal_move(text, 0, true)?;
                }
            }
            continue;
        }
        if !element.eq_ignore_ascii_case(b"texture") && !element.eq_ignore_ascii_case(b"enum") {
            continue;
        }
        let (name, value) = property(child);
        let value = value.unwrap_or_default();
        if value.is_empty() {
            continue;
        }
        if name == b"Type" {
            flags = match value.as_slice() {
                b"trans_vertex_alpha" => MAT_TRANS_VERTEX_ALPHA,
                b"lightmap" => MAT_LIGHTMAP,
                b"solid_2layer" => MAT_SOLID_2LAYER,
                b"lightmap_m2" => MAT_LIGHTMAP | 0x4,
                b"lightmap_m4" => MAT_LIGHTMAP | 0x8,
                b"lightmap_light" => MAT_LIGHTMAP | 0x10,
                b"lightmap_light_m2" => MAT_LIGHTMAP | 0x20,
                b"lightmap_light_m4" => MAT_LIGHTMAP | 0x40,
                b"lightmap_add" => MAT_LIGHTMAP | 0x80,
                b"normalmap_solid" | b"parallaxmap_solid" => MAT_NORMALMAP_SOLID,
                b"normalmap_trans_vertex_alpha" | b"parallaxmap_trans_vertex_alpha" => MAT_NORMALMAP_SOLID | MAT_TRANS_VERTEX_ALPHA,
                b"normalmap_trans_add" | b"parallaxmap_trans_add" => MAT_NORMALMAP_SOLID | 0x2,
                _ => flags,
            };
        }
        match name.as_slice() {
            b"Texture1" => textures += 1,
            b"Texture2" if textures == 1 => {
                if flags & (MAT_LIGHTMAP | MAT_NORMALMAP_SOLID | MAT_SOLID_2LAYER) != 0 {
                    textures += 1;
                    flags |= EXTRA_2ND_TEXTURE;
                }
            }
            b"Texture3" if textures == 2 => textures += 1,
            b"Texture4" if textures == 3 => textures += 1,
            _ => {}
        }
    }
    Ok(flags)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VertexFormat {
    Standard,
    TwoCoordinates,
    Tangents,
}

/// A buffer's vertex lists as read.
#[derive(Default)]
struct Buffer {
    positions: Vec<Vector3>,
    normals: Vec<Vector3>,
    tangents: Vec<Vector3>,
    bitangents: Vec<Vector3>,
    colors: Vec<[f32; 4]>,
    uvs: Vec<Vector3>,
    second_uvs: Vec<Vector3>,
}

/// `ColorFromARGBPacked`.
fn color_from_argb(packed: u32) -> [f32; 4] {
    let channel = |shift: u32| ((packed >> shift) & 0xff) as f32 / 255.0;
    [channel(16), channel(8), channel(0), channel(24)]
}

/// A cursor over the vertex text, read as `fast_atoreal_move` and
/// `SkipSpaces` read it.
struct Reader<'a> {
    text: &'a [u8],
    position: usize,
}

impl Reader<'_> {
    fn real(&mut self) -> Result<f32, ImportError> {
        let (value, next) = fast_atoreal_move(self.text, self.position, true)?;
        self.position = next;
        Ok(value)
    }

    fn skip_spaces(&mut self) {
        self.position = skip_spaces(self.text, self.position, self.text.len()).0;
    }

    /// A number, then the blanks after it.
    fn real_then_spaces(&mut self) -> Result<f32, ImportError> {
        let value = self.real()?;
        self.skip_spaces();
        Ok(value)
    }
}

/// `ParseBufferVertices`: a vertex a line, at least one; once a colour
/// differs from the one before, colours are in use (for every later
/// buffer too).
fn parse_vertices(text: &[u8], format: VertexFormat, buffer: &mut Buffer, use_colors: &mut bool) -> Result<(), ImportError> {
    let mut reader = Reader { text, position: 0 };
    let end = text.len();
    loop {
        reader.position = skip_spaces_and_line_end(text, reader.position, end).0;
        let position = [reader.real_then_spaces()?, reader.real_then_spaces()?, reader.real_then_spaces()?];
        buffer.positions.push(position);
        let normal = [reader.real_then_spaces()?, reader.real_then_spaces()?, reader.real_then_spaces()?];
        buffer.normals.push(normal);

        let (packed, next) = strtoul16(text, reader.position);
        reader.position = next;
        let color = color_from_argb(packed);
        if buffer.colors.last().is_some_and(|&last| last != color) {
            *use_colors = true;
        }
        buffer.colors.push(color);
        reader.skip_spaces();

        let u = reader.real_then_spaces()?;
        let v = reader.real_then_spaces()?;
        buffer.uvs.push([u, 1.0 - v, 0.0]);

        match format {
            VertexFormat::TwoCoordinates => {
                let u = reader.real_then_spaces()?;
                let v = reader.real()?;
                buffer.second_uvs.push([u, 1.0 - v, 0.0]);
            }
            VertexFormat::Tangents => {
                let x = reader.real_then_spaces()?;
                let z = reader.real_then_spaces()?;
                let y = reader.real_then_spaces()?;
                buffer.tangents.push([x, -y, z]);
                let x = reader.real_then_spaces()?;
                let z = reader.real_then_spaces()?;
                let y = reader.real_then_spaces()?;
                buffer.bitangents.push([x, -y, z]);
            }
            VertexFormat::Standard => {}
        }

        let (next, more) = skip_line(text, reader.position, end);
        reader.position = next;
        if !more {
            return Ok(());
        }
    }
}

/// An `int` attribute (pugixml's `as_int`, 0 when missing).
fn int_attribute(node: &XmlNode, name: &str) -> i32 {
    node.attribute(name).map_or(0, xml::as_int)
}

/// A stream copied per index, made only when its list is as long as the
/// positions.
fn stream_for<T>(list: &[T], positions: usize) -> Option<Vec<T>> {
    if list.len() != positions {
        return None;
    }
    Some(Vec::new())
}

/// The `<indices indexCount="n">` of a buffer: n vertices (n / 3 faces),
/// each index's vertex copied to the next one. Faces past the indices
/// keep no indices; a face left part-filled has uninitialised ones.
fn build_mesh(indices_node: &XmlNode, buffer: &Buffer, use_colors: bool, material_index: u32) -> Result<Option<Mesh>, ImportError> {
    let vertex_count = int_attribute(indices_node, "indexCount") as u32 as usize;
    if vertex_count == 0 {
        return Ok(None);
    }
    let face_count = vertex_count / 3;
    check_allocation(face_count as u64, FACE_SIZE)?;
    check_allocation(vertex_count as u64, VECTOR_SIZE)?;
    if buffer.colors.len() == buffer.positions.len() && use_colors {
        check_allocation(vertex_count as u64, COLOR_SIZE)?;
    }

    let positions_count = buffer.positions.len();
    let mut normals = stream_for(&buffer.normals, positions_count);
    let mut tangents = stream_for(&buffer.tangents, positions_count);
    let mut bitangents = stream_for(&buffer.bitangents, positions_count);
    let mut colors = if use_colors { stream_for(&buffer.colors, positions_count) } else { None };
    let mut uvs = stream_for(&buffer.uvs, positions_count);
    let mut second_uvs = stream_for(&buffer.second_uvs, positions_count);
    let mut vertices: Vec<Vector3> = Vec::new();

    let text = indices_node.text();
    let end = text.len();
    let mut position = 0;
    let mut faces: Vec<Vec<u32>> = Vec::with_capacity(face_count);
    let mut corner = 0;
    loop {
        let (next, more) = skip_spaces_and_line_end(text, position, end);
        position = next;
        if !more {
            break;
        }
        if faces.len() == face_count && corner == 0 {
            // "Too many indices".
            break;
        }
        if corner == 0 {
            faces.push(Vec::with_capacity(3));
        }
        let (value, next) = strtoul10(text, position);
        position = next;
        let mut index = value as usize;
        if index >= positions_count {
            index = 0;
        }
        if index >= positions_count {
            return undefined("an index into an empty vertex list");
        }
        let total = vertices.len() as u32;
        if let Some(face) = faces.last_mut() {
            face.push(total);
        }
        vertices.push(buffer.positions[index]);
        let copy = |stream: &mut Option<Vec<Vector3>>, list: &[Vector3]| {
            if let Some(stream) = stream {
                stream.push(list[index]);
            }
        };
        copy(&mut normals, &buffer.normals);
        copy(&mut tangents, &buffer.tangents);
        copy(&mut bitangents, &buffer.bitangents);
        if let Some(stream) = colors.as_mut() {
            stream.push(buffer.colors[index]);
        }
        copy(&mut uvs, &buffer.uvs);
        copy(&mut second_uvs, &buffer.second_uvs);
        corner += 1;
        if corner == 3 {
            corner = 0;
        }
    }
    if corner != 0 {
        return undefined("a face read from uninitialised indices");
    }
    while faces.len() < face_count {
        faces.push(Vec::new());
    }

    // Vertices past the indices read are zero.
    let pad = |stream: &mut Option<Vec<Vector3>>| {
        if let Some(stream) = stream {
            stream.resize(vertex_count, [0.0; 3]);
        }
    };
    vertices.resize(vertex_count, [0.0; 3]);
    pad(&mut normals);
    pad(&mut tangents);
    pad(&mut bitangents);
    pad(&mut uvs);
    pad(&mut second_uvs);
    if let Some(stream) = colors.as_mut() {
        stream.resize(vertex_count, [0.0; 4]);
    }

    let mut mesh = Mesh {
        primitive_types: PRIMITIVE_TRIANGLE,
        vertex_count,
        vertices: Some(vertices),
        normals,
        tangents,
        bitangents,
        faces: Some(faces),
        material_index,
        ..Mesh::default()
    };
    mesh.colors[0] = colors;
    mesh.texture_coords[0] = uvs;
    mesh.texture_coords[1] = second_uvs;
    Ok(Some(mesh))
}

/// `IRRMeshImporter::InternReadFile`.
pub fn read(path: &str, bytes: &[u8]) -> Result<Scene, ImportError> {
    let Ok(document) = xml::parse(bytes) else {
        return fail(format!("XML parse error while loading IRRMESH file {path}"));
    };
    let mut meshes = Vec::new();
    let mut use_colors = false;
    if let Some(mesh_node) = document.child("mesh") {
        for buffer_node in &mesh_node.children {
            if !buffer_node.name().eq_ignore_ascii_case(b"buffer") {
                continue;
            }
            let Some(material_node) = buffer_node.child("material") else {
                continue;
            };
            let flags = parse_material(material_node)?;

            let mut buffer = Buffer::default();
            if let Some(vertices_node) = buffer_node.child("vertices") {
                let vertex_count = int_attribute(vertices_node, "vertexCount");
                if vertex_count == 0 {
                    continue;
                }
                if vertex_count < 0 {
                    return fail("vector too long");
                }
                check_allocation(vertex_count as u64, VECTOR_SIZE)?;
                check_allocation(vertex_count as u64, COLOR_SIZE)?;
                let kind = vertices_node.attribute("type").unwrap_or_default();
                let format = if kind.eq_ignore_ascii_case(b"2tcoords") {
                    VertexFormat::TwoCoordinates
                } else if kind.eq_ignore_ascii_case(b"tangents") {
                    VertexFormat::Tangents
                } else if kind.eq_ignore_ascii_case(b"standard") {
                    VertexFormat::Standard
                } else {
                    continue;
                };
                parse_vertices(vertices_node.text(), format, &mut buffer, &mut use_colors)?;
            }

            let mut mesh = None;
            if let Some(indices_node) = buffer_node.child("indices") {
                let material_index = meshes.len() as u32;
                mesh = build_mesh(indices_node, &buffer, use_colors, material_index)?;
                if mesh.is_none() {
                    continue;
                }
            }
            // The opacity is taken from the first vertex colour.
            if flags & MAT_TRANS_VERTEX_ALPHA != 0 && !use_colors && buffer.colors.is_empty() {
                return undefined("the first colour of an empty vertex list");
            }
            if let Some(mesh) = mesh {
                meshes.push(mesh);
            }
        }
    }
    if meshes.is_empty() {
        return fail("IRRMESH: Unable to read a mesh from this file");
    }
    Ok(Scene { meshes })
}
