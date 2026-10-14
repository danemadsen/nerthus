//! Assimp's Quake III BSP importer (`Q3BSPFileImporter.cpp` and
//! `Q3BSPFileParser.cpp`): a `.pk3` (zip) archive's map, the one a comma
//! in the path names (from the comma, less the path's last character) or
//! else the first `maps/….bsp` by name. A mesh per texture and lightmap
//! pair, in the order of their `"texture.lightmap"` keys as strings: the
//! polygon and triangle mesh faces' vertices (three per face, in order),
//! normals and both texture coordinate sets. A missing map gives an empty
//! scene.
//!
//! The lumps are read without bounds checks (sizes sign-extended into
//! allocations); faces are allocated a third of each face's vertex count
//! and filled three vertices at a time, so a face left with fewer, a
//! write past the faces or vertices, a read outside the file or the
//! index list, a texture number past the textures and a texture name
//! without its NUL are undefined behaviour, errors here.

use std::collections::BTreeMap;

use crate::scene::{ImportError, Mesh, PRIMITIVE_TRIANGLE, Scene, check_allocation, fail};
use crate::zip::Archive;

const VERTEX_SIZE: u64 = 44;
const FACE_SIZE: u64 = 104;
const TEXTURE_SIZE: u64 = 72;
const LIGHTMAP_SIZE: u64 = 128 * 128 * 3;

const LUMP_TEXTURES: usize = 1;
const LUMP_VERTICES: usize = 10;
const LUMP_MESH_VERTICES: usize = 11;
const LUMP_FACES: usize = 13;
const LUMP_LIGHTMAPS: usize = 14;
const LUMP_ENTITIES: usize = 0;

const POLYGON: i32 = 1;
const TRIANGLE_MESH: i32 = 3;

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// The bytes `offset..offset + size` of the map, which must be there.
fn slice(data: &[u8], offset: u64, size: u64) -> Result<&[u8], ImportError> {
    let end = offset.checked_add(size);
    match end {
        Some(end) if end <= data.len() as u64 => Ok(&data[offset as usize..end as usize]),
        _ => undefined("a read past the map"),
    }
}

fn int_at(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
}

fn float_at(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
}

struct Vertex {
    position: [f32; 3],
    texcoord: [f32; 2],
    lightmap: [f32; 2],
    normal: [f32; 3],
}

struct Face {
    texture: i32,
    kind: i32,
    first_vertex: i32,
    first_index: i32,
    index_count: i32,
    lightmap: i32,
}

struct Model {
    vertices: Vec<Vertex>,
    indices: Vec<i32>,
    faces: Vec<Face>,
    /// Each texture's 72 bytes.
    textures: Vec<Vec<u8>>,
}

/// A lump's element count: its size, sign-extended, over the element
/// size (a negative size makes a failed allocation).
fn element_count(size: i32, element_size: u64) -> Result<u64, ImportError> {
    let count = (size as i64 as u64) / element_size;
    check_allocation(count, 8)?;
    Ok(count)
}

/// `Q3BSPFileParser::parseFile`: `None` when the map isn't one.
fn parse(data: &[u8]) -> Result<Option<Model>, ImportError> {
    if data.is_empty() {
        return Ok(None);
    }
    let header = slice(data, 0, 4)?;
    if header != b"IBSP" {
        return Ok(None);
    }
    let lump_bytes = slice(data, 8, 17 * 8)?;
    let lump = |index: usize| (int_at(lump_bytes, index * 8), int_at(lump_bytes, index * 8 + 4));

    let (vertex_offset, vertex_size) = lump(LUMP_VERTICES);
    let (index_offset, index_size) = lump(LUMP_MESH_VERTICES);
    let (face_offset, face_size) = lump(LUMP_FACES);
    let (texture_offset, texture_size) = lump(LUMP_TEXTURES);
    let (lightmap_offset, lightmap_size) = lump(LUMP_LIGHTMAPS);
    let (entity_offset, entity_size) = lump(LUMP_ENTITIES);
    let vertex_count = element_count(vertex_size, VERTEX_SIZE)?;
    let index_count = (index_size as i64 as u64) / 4;
    check_allocation(index_count, 4)?;
    let face_count = element_count(face_size, FACE_SIZE)?;
    let texture_count = element_count(texture_size, TEXTURE_SIZE)?;
    let lightmap_count = element_count(lightmap_size, LIGHTMAP_SIZE)?;

    let offset = |value: i32| value as i64 as u64;
    let mut vertices = Vec::with_capacity(vertex_count as usize);
    for index in 0..vertex_count {
        let bytes = slice(data, offset(vertex_offset).wrapping_add(index * VERTEX_SIZE), VERTEX_SIZE)?;
        vertices.push(Vertex {
            position: [float_at(bytes, 0), float_at(bytes, 4), float_at(bytes, 8)],
            texcoord: [float_at(bytes, 12), float_at(bytes, 16)],
            lightmap: [float_at(bytes, 20), float_at(bytes, 24)],
            normal: [float_at(bytes, 28), float_at(bytes, 32), float_at(bytes, 36)],
        });
    }
    // `memcpy(&indices[0], …, size)`: with no whole index, a few bytes
    // copied to a null pointer.
    let copied = index_size as i64 as u64;
    if index_count == 0 && copied > 0 {
        return undefined("indices copied to a null pointer");
    }
    let index_bytes = slice(data, offset(index_offset), copied)?;
    let indices = (0..index_count as usize).map(|index| int_at(index_bytes, index * 4)).collect();
    let mut faces = Vec::with_capacity(face_count as usize);
    for index in 0..face_count {
        let bytes = slice(data, offset(face_offset).wrapping_add(index * FACE_SIZE), FACE_SIZE)?;
        faces.push(Face {
            texture: int_at(bytes, 0),
            kind: int_at(bytes, 8),
            first_vertex: int_at(bytes, 12),
            first_index: int_at(bytes, 20),
            index_count: int_at(bytes, 24),
            lightmap: int_at(bytes, 28),
        });
    }
    let mut textures = Vec::with_capacity(texture_count as usize);
    for index in 0..texture_count {
        textures.push(slice(data, offset(texture_offset).wrapping_add(index * TEXTURE_SIZE), TEXTURE_SIZE)?.to_vec());
    }
    for index in 0..lightmap_count {
        slice(data, offset(lightmap_offset).wrapping_add(index * LIGHTMAP_SIZE), LIGHTMAP_SIZE)?;
    }
    check_allocation(entity_size as i64 as u64, 1)?;
    if entity_size > 0 {
        slice(data, offset(entity_offset), entity_size as u64)?;
    }
    Ok(Some(Model { vertices, indices, faces, textures }))
}

/// A face being filled: three slots, set or not.
struct OpenFace {
    indices: [Option<u32>; 3],
}

/// `CreateTopology` for one texture and lightmap pair.
fn build_mesh(model: &Model, faces: &[usize]) -> Result<Option<Mesh>, ImportError> {
    let is_mesh_face = |face: &Face| face.kind == POLYGON || face.kind == TRIANGLE_MESH;
    let vertex_total = faces.iter().filter(|&&face| is_mesh_face(&model.faces[face])).fold(0u64, |sum, &face| sum.wrapping_add(model.faces[face].index_count as i64 as u64));
    if vertex_total == 0 {
        return Ok(None);
    }
    if !faces.iter().any(|&face| model.faces[face].index_count > 0) {
        return Ok(None);
    }
    let triangle_total = faces.iter().fold(0u64, |sum, &face| sum.wrapping_add((model.faces[face].index_count / 3) as i64 as u64));
    check_allocation(triangle_total, 16)?;
    check_allocation(vertex_total, 12)?;
    let face_limit = triangle_total as usize;
    let vertex_limit = vertex_total as usize;

    let mut slots: Vec<OpenFace> = Vec::new();
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut texcoords = Vec::new();
    let mut lightmaps = Vec::new();
    for &face_index in faces {
        let face = &model.faces[face_index];
        if face.index_count <= 0 || !is_mesh_face(face) {
            continue;
        }
        // `getNextFace`: none once the faces are used up.
        if slots.len() >= face_limit {
            continue;
        }
        slots.push(OpenFace { indices: [None; 3] });
        let mut current = Some(slots.len() - 1);
        let mut corner = 0usize;
        for step in 0..face.index_count as u64 {
            let list_index = (face.first_index as i64 as u64).wrapping_add(step);
            let Some(&offset) = model.indices.get(list_index as usize).filter(|_| list_index < model.indices.len() as u64) else {
                return undefined("a read past the index list");
            };
            let index = face.first_vertex.wrapping_add(offset) as i64 as u64;
            let Some(vertex) = model.vertices.get(index as usize).filter(|_| index < model.vertices.len() as u64) else {
                continue;
            };
            let vertex_index = positions.len() as u32;
            if corner > 2 {
                corner = 0;
                current = None;
                if slots.len() < face_limit {
                    slots.push(OpenFace { indices: [Some(vertex_index), None, None] });
                    current = Some(slots.len() - 1);
                }
            } else {
                let Some(slot) = current else {
                    return undefined("a face index written through a null face");
                };
                slots[slot].indices[corner] = Some(vertex_index);
            }
            if positions.len() >= vertex_limit {
                return undefined("a vertex written past the vertices");
            }
            positions.push(vertex.position);
            normals.push(vertex.normal);
            texcoords.push([vertex.texcoord[0], vertex.texcoord[1], 0.0]);
            lightmaps.push([vertex.lightmap[0], vertex.lightmap[1], 0.0]);
            corner += 1;
        }
    }
    let mut face_lists = Vec::with_capacity(face_limit);
    for slot in &slots {
        let indices: Option<Vec<u32>> = slot.indices.iter().copied().collect();
        let Some(indices) = indices else {
            return undefined("a face left with uninitialised indices");
        };
        face_lists.push(indices);
    }
    // Faces counted but never reached have no indices.
    face_lists.resize(face_limit, Vec::new());
    positions.resize(vertex_limit, [0.0; 3]);
    normals.resize(vertex_limit, [0.0; 3]);
    texcoords.resize(vertex_limit, [0.0; 3]);
    lightmaps.resize(vertex_limit, [0.0; 3]);

    let mut mesh = Mesh {
        primitive_types: PRIMITIVE_TRIANGLE,
        vertex_count: vertex_limit,
        vertices: Some(positions),
        normals: Some(normals),
        faces: Some(face_lists),
        ..Mesh::default()
    };
    mesh.uv_components[0] = 2;
    mesh.uv_components[1] = 2;
    mesh.texture_coords[0] = Some(texcoords);
    mesh.texture_coords[1] = Some(lightmaps);
    Ok(Some(mesh))
}

/// Whether `text` holds `part` (`std::string::find`).
fn contains(text: &[u8], part: &[u8]) -> bool {
    text.windows(part.len()).any(|window| window == part)
}

/// `findFirstMapInArchive`.
fn first_map(archive: &Archive) -> Option<Vec<u8>> {
    let names = archive.names();
    names.into_iter().find(|name| crate::registry::name_extension(name) == b"bsp" && contains(name, b"maps/") && contains(name, b".bsp"))
}

pub fn read(path: &str, bytes: &[u8]) -> Result<Scene, ImportError> {
    let Some(archive) = Archive::open(bytes)? else {
        return fail("Failed to open file.");
    };
    let path_bytes = path.as_bytes();
    let map_name = match path_bytes.iter().rposition(|&byte| byte == b',') {
        Some(comma) => path_bytes[comma..path_bytes.len() - 1].to_vec(),
        None => Vec::new(),
    };
    let map_name = if map_name.is_empty() {
        match first_map(&archive) {
            Some(name) => name,
            None => return Ok(Scene { meshes: Vec::new() }),
        }
    } else {
        map_name
    };
    // `readData`: `Exists` doesn't simplify the name, `Open` does.
    if !archive.exists(&map_name) {
        return Ok(Scene { meshes: Vec::new() });
    }
    let Some(data) = archive.read(&map_name)? else {
        return Ok(Scene { meshes: Vec::new() });
    };
    let Some(model) = parse(&data)? else {
        return Ok(Scene { meshes: Vec::new() });
    };

    // `createMaterialMap`: faces by their keys, in string order.
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, face) in model.faces.iter().enumerate() {
        groups.entry(format!("{}.{}", face.texture, face.lightmap)).or_default().push(index);
    }
    let mut meshes = Vec::new();
    for faces in groups.values() {
        if let Some(mesh) = build_mesh(&model, faces)? {
            meshes.push(mesh);
        }
    }
    // `createMaterials`: every key's texture is looked up, and
    // `importTextureFromArchive` extracts the first of its name with
    // `.jpg`, `.png` or `.tga` that the archive has (which can only fail
    // the import by being too big to allocate).
    for faces in groups.values() {
        let texture = model.faces[faces[0]].texture;
        if texture == -1 {
            continue;
        }
        let Some(record) = model.textures.get(texture as usize).filter(|_| texture >= 0) else {
            return undefined("a texture number past the textures");
        };
        let Some(name_end) = record.iter().position(|&byte| byte == 0) else {
            return undefined("a texture name read past its record");
        };
        let texture_name = &record[..name_end];
        for extension in [b".jpg", b".png", b".tga"] {
            let mut file_name = texture_name.to_vec();
            file_name.extend_from_slice(extension);
            if archive.exists(&file_name) {
                archive.read(&file_name)?;
                break;
            }
        }
    }
    Ok(Scene { meshes })
}
