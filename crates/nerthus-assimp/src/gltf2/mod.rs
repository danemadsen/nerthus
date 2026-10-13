//! `glTF2Importer` (Assimp's glTF 2.0 importer): every primitive of every
//! loaded mesh becomes an `aiMesh`. Indexed primitives keep only the
//! vertices their indices use, in first-use order; strips and fans are
//! unrolled with Assimp's orderings; a primitive left without faces fails
//! the import. Skinned nodes and animations are checked the way their
//! import reads them, since a bad accessor there fails the import too.

pub mod asset;

use std::collections::HashMap;

use asset::{
    Asset, COMPONENT_FLOAT, COMPONENT_UNSIGNED_BYTE, COMPONENT_UNSIGNED_SHORT, MODE_LINE_LOOP, MODE_LINE_STRIP, MODE_LINES, MODE_POINTS,
    MODE_TRIANGLE_FAN, MODE_TRIANGLE_STRIP, MODE_TRIANGLES, PATH_ROTATION, PATH_SCALE, PATH_TRANSLATION, PATH_WEIGHTS, component_size,
};

use crate::scene::{
    ImportError, MAX_COLOR_SETS, MAX_TEXTURE_COORDS, Mesh, PRIMITIVE_LINE, PRIMITIVE_POINT, PRIMITIVE_TRIANGLE, Scene, fail,
};
use crate::vector::{cross, scale};

/// `glTF2Importer::CanRead` with the signature check: the document (and
/// GLB header) reads and its version starts with '2'.
pub fn can_read(bytes: &[u8]) -> bool {
    let Ok(document) = asset::read_document(bytes) else {
        return false;
    };
    let Ok(json) = asset::parse_json(&document.json) else {
        return false;
    };
    asset::check_version(&json).is_ok()
}

/// The bytes an accessor's elements are read from (`GetPointer`) with
/// `GetStride` and `GetMaxByteSize`; reads past the data give zeros.
struct Source<'a> {
    data: &'a [u8],
    element_size: usize,
    stride: usize,
    max_size: usize,
}

impl Source<'_> {
    fn element(&self, index: usize, output: &mut [u8]) {
        let start = index.wrapping_mul(self.stride);
        for (offset, byte) in output.iter_mut().enumerate().take(self.element_size) {
            *byte = self.data.get(start.wrapping_add(offset)).copied().unwrap_or(0);
        }
    }
}

fn source<'a>(asset: &'a Asset, slot: usize) -> Result<Option<Source<'a>>, ImportError> {
    let accessor = asset.accessor(slot);
    let element_size = accessor.components as usize * component_size(accessor.component_type)?;
    let view = accessor.buffer_view.map(|view| asset.buffer_view(view));
    let stride = match view {
        Some(view) if view.byte_stride != 0 => view.byte_stride as usize,
        _ => element_size,
    };
    let data: &[u8] = if let Some(sparse) = accessor.sparse.as_deref() {
        // An empty std::vector's data() is null.
        if sparse.is_empty() {
            return Ok(None);
        }
        sparse
    } else {
        let Some(view) = view else {
            return Ok(None);
        };
        let Some(bytes) = asset.buffer(view.buffer).data.as_deref() else {
            return Ok(None);
        };
        let start = accessor.byte_offset.wrapping_add(view.byte_offset);
        bytes.get(start..).unwrap_or(&[])
    };
    let max_size = match (view, accessor.sparse.as_ref()) {
        (Some(view), _) => view.byte_length,
        (None, Some(sparse)) => sparse.len(),
        (None, None) => 0,
    };
    Ok(Some(Source { data, element_size, stride, max_size }))
}

/// `Accessor::ExtractData<T>` with `size_of::<T>() == target_size`: each
/// element's bytes copied into a zeroed `T`.
fn extract(asset: &Asset, slot: usize, target_size: usize, remap: Option<&[u32]>) -> Result<Vec<Vec<u8>>, ImportError> {
    let accessor = asset.accessor(slot);
    let Some(source) = source(asset, slot)? else {
        return fail(format!("GLTF2: data is null when extracting data from {}", accessor.id));
    };
    if source.element_size > target_size {
        return fail(format!("GLTF: elemSize {} > targetElemSize {target_size} in {}", source.element_size, accessor.id));
    }
    let used = remap.map_or(accessor.count as usize, <[u32]>::len);
    let mut output = vec![vec![0u8; target_size]; used];
    match remap {
        Some(remap) => {
            let max_index = (source.max_size / source.stride) as u32;
            for (element, &index) in output.iter_mut().zip(remap) {
                if index >= max_index {
                    return fail(format!("GLTF: index*stride > maxSize in {}", accessor.id));
                }
                source.element(index as usize, element);
            }
        }
        None => {
            if used.wrapping_mul(source.stride) > source.max_size {
                return fail(format!("GLTF: count*stride > maxSize in {}", accessor.id));
            }
            for (index, element) in output.iter_mut().enumerate() {
                source.element(index, element);
            }
        }
    }
    Ok(output)
}

fn floats<const N: usize>(bytes: &[u8]) -> [f32; N] {
    std::array::from_fn(|index| f32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap()))
}

fn extract_vectors(asset: &Asset, slot: usize, remap: Option<&[u32]>) -> Result<Vec<[f32; 3]>, ImportError> {
    let elements = extract(asset, slot, 12, remap)?;
    Ok(elements.iter().map(|bytes| floats::<3>(bytes)).collect())
}

/// `Accessor::Indexer::GetUInt`.
fn index_value(source: &Source, index: usize) -> Result<u32, ImportError> {
    if index.wrapping_mul(source.stride) >= source.max_size {
        return fail(format!("GLTF: Invalid index {index}, count out of range"));
    }
    let mut bytes = [0u8; 4];
    let size = source.element_size.min(4);
    source.element(index, &mut bytes[..size]);
    // `element` fills `element_size` bytes at most; clear the rest.
    for byte in bytes.iter_mut().skip(size) {
        *byte = 0;
    }
    Ok(u32::from_le_bytes(bytes))
}

/// The faces of one primitive (`SetFaceAndAdvance*` drop faces with an
/// index past the vertices).
struct FaceBuilder {
    vertex_count: u32,
    faces: Vec<Vec<u32>>,
}

impl FaceBuilder {
    fn add(&mut self, corners: &[u32]) {
        if corners.iter().any(|&corner| corner >= self.vertex_count) {
            return;
        }
        self.faces.push(corners.to_vec());
    }
}

/// The primitive's faces from its index list (`indices`: the remapped
/// index buffer, or every vertex in order when `indexed` is false) in
/// Assimp's per-mode order; `None` for an unknown mode. Where Assimp's face
/// count underflows (the allocation throws) or it reads past the list, the
/// import fails.
fn build_faces(mode: i32, indices: &[u32], vertex_count: u32, indexed: bool) -> Result<Option<Vec<Vec<u32>>>, ImportError> {
    let count = indices.len();
    let mut builder = FaceBuilder { vertex_count, faces: Vec::new() };
    // A single unindexed vertex in LINE_LOOP mode closes on itself.
    if !indexed && mode == MODE_LINE_LOOP && count == 1 {
        builder.add(&[0, 0]);
        return Ok(Some(builder.faces));
    }
    match mode {
        MODE_POINTS => {
            for &index in indices {
                builder.add(&[index]);
            }
        }
        MODE_LINES => {
            for pair in indices.as_chunks::<2>().0 {
                builder.add(pair);
            }
        }
        MODE_LINE_LOOP | MODE_LINE_STRIP => {
            if count < 2 {
                return fail("bad allocation");
            }
            builder.add(&[indices[0], indices[1]]);
            for index in 2..count {
                builder.add(&[indices[index - 1], indices[index]]);
            }
            if mode == MODE_LINE_LOOP {
                let Some(first) = builder.faces.first().map(|face| face[0]) else {
                    return fail("GLTF: line loop without a first segment");
                };
                builder.add(&[indices[count - 1], first]);
            }
        }
        MODE_TRIANGLES => {
            for triangle in indices.as_chunks::<3>().0 {
                builder.add(triangle);
            }
        }
        MODE_TRIANGLE_STRIP => {
            if count < 2 {
                return fail("bad allocation");
            }
            for face in 0..count - 2 {
                if (face + 1) % 2 == 0 {
                    builder.add(&[indices[face + 1], indices[face], indices[face + 2]]);
                } else {
                    builder.add(&[indices[face], indices[face + 1], indices[face + 2]]);
                }
            }
        }
        MODE_TRIANGLE_FAN => {
            if count < 3 {
                return fail("bad allocation");
            }
            builder.add(&[indices[0], indices[1], indices[2]]);
            for face in 1..count - 2 {
                builder.add(&[indices[0], indices[face + 1], indices[face + 2]]);
            }
        }
        _ => return Ok(None),
    }
    Ok(Some(builder.faces))
}

/// `ImportMeshes`: one `aiMesh` per primitive, with the vertex remapping
/// tables kept for the skin check.
fn import_meshes(asset: &Asset) -> Result<(Vec<Mesh>, Vec<Vec<u32>>), ImportError> {
    let mut meshes = Vec::new();
    let mut remapping_tables = Vec::new();
    for mesh in asset.meshes() {
        for (primitive_index, primitive) in mesh.primitives.iter().enumerate() {
            let attributes = &primitive.attributes;
            let position = attributes.position.first().copied().flatten();
            let all_vertices = position.map_or(0, |slot| asset.accessor(slot).count as usize);

            let mut index_buffer = Vec::new();
            let mut remap: Option<Vec<u32>> = None;
            if let Some(indices) = primitive.indices {
                let count = asset.accessor(indices).count as usize;
                let Some(indexer) = source(asset, indices)? else {
                    return fail(format!("GLTF: Invalid accessor without data in mesh {}", mesh.id));
                };
                let mut table = Vec::new();
                let mut reverse: Vec<u32> = Vec::new();
                index_buffer.reserve(count);
                for element in 0..count {
                    let index = index_value(&indexer, element)?;
                    if index as usize >= all_vertices {
                        index_buffer.push(index);
                        continue;
                    }
                    if index as usize >= reverse.len() {
                        reverse.resize(index as usize + 1, u32::MAX);
                    }
                    if reverse[index as usize] == u32::MAX {
                        reverse[index as usize] = table.len() as u32;
                        table.push(index);
                    }
                    index_buffer.push(reverse[index as usize]);
                }
                remap = Some(table);
            }
            let remap_slice = remap.as_deref();

            let mut output = Mesh { name: mesh.name.clone(), ..Mesh::default() };
            if mesh.primitives.len() > 1 {
                output.name = format!("{}-{primitive_index}", if mesh.name.is_empty() { &mesh.id } else { &mesh.name });
            }
            output.primitive_types = match primitive.mode {
                MODE_POINTS => PRIMITIVE_POINT,
                MODE_LINES | MODE_LINE_LOOP | MODE_LINE_STRIP => PRIMITIVE_LINE,
                MODE_TRIANGLES | MODE_TRIANGLE_STRIP | MODE_TRIANGLE_FAN => PRIMITIVE_TRIANGLE,
                _ => 0,
            };

            if let Some(slot) = position {
                let vertices = extract_vectors(asset, slot, remap_slice)?;
                output.vertex_count = vertices.len();
                output.vertices = Some(vertices);
            }
            if let Some(slot) = attributes.normal.first().copied().flatten()
                && asset.accessor(slot).count as usize == all_vertices {
                    let normals = extract_vectors(asset, slot, remap_slice)?;
                    if let Some(tangent_slot) = attributes.tangent.first().copied().flatten()
                        && asset.accessor(tangent_slot).count as usize == all_vertices {
                            let tangents = extract(asset, tangent_slot, 16, remap_slice)?;
                            let mut xyz = Vec::with_capacity(output.vertex_count);
                            let mut bitangents = Vec::with_capacity(output.vertex_count);
                            for vertex in 0..output.vertex_count {
                                let tangent = floats::<4>(&tangents[vertex]);
                                let direction = [tangent[0], tangent[1], tangent[2]];
                                xyz.push(direction);
                                bitangents.push(scale(cross(normals[vertex], direction), tangent[3]));
                            }
                            output.tangents = Some(xyz);
                            output.bitangents = Some(bitangents);
                        }
                    output.normals = Some(normals);
                }
            for (set, slot) in attributes.color.iter().take(MAX_COLOR_SETS).enumerate() {
                let Some(slot) = *slot else {
                    continue;
                };
                let accessor = asset.accessor(slot);
                if accessor.count as usize != all_vertices {
                    continue;
                }
                let colors: Option<Vec<[f32; 4]>> = match accessor.component_type {
                    COMPONENT_FLOAT => {
                        let elements = extract(asset, slot, 16, remap_slice)?;
                        Some(elements.iter().map(|bytes| floats::<4>(bytes)).collect())
                    }
                    COMPONENT_UNSIGNED_BYTE => {
                        let elements = extract(asset, slot, 4, remap_slice)?;
                        Some(elements.iter().map(|bytes| std::array::from_fn(|channel| bytes[channel] as f32 / 255.0)).collect())
                    }
                    COMPONENT_UNSIGNED_SHORT => {
                        let elements = extract(asset, slot, 8, remap_slice)?;
                        let unpack = |bytes: &Vec<u8>| -> [f32; 4] {
                            std::array::from_fn(|channel| u16::from_le_bytes([bytes[channel * 2], bytes[channel * 2 + 1]]) as f32 / 65535.0)
                        };
                        Some(elements.iter().map(unpack).collect())
                    }
                    _ => None,
                };
                output.colors[set] = colors;
            }
            for (set, slot) in attributes.texcoord.iter().take(MAX_TEXTURE_COORDS).enumerate() {
                let Some(slot) = *slot else {
                    continue;
                };
                let accessor = asset.accessor(slot);
                if accessor.count as usize != all_vertices {
                    continue;
                }
                let mut coordinates = extract_vectors(asset, slot, remap_slice)?;
                output.uv_components[set] = accessor.components;
                for coordinate in coordinates.iter_mut().take(output.vertex_count) {
                    coordinate[1] = 1.0 - coordinate[1];
                }
                output.texture_coords[set] = Some(coordinates);
            }

            check_targets(asset, primitive, &output, all_vertices, remap_slice)?;

            let all_indices: Vec<u32>;
            let indices: &[u32] = if primitive.indices.is_some() {
                &index_buffer
            } else {
                all_indices = (0..output.vertex_count as u32).collect();
                &all_indices
            };
            if let Some(faces) = build_faces(primitive.mode, indices, output.vertex_count as u32, primitive.indices.is_some())? {
                if faces.is_empty() {
                    return fail(format!("Mesh \"{}\" has no faces", output.name));
                }
                output.faces = Some(faces);
            }
            meshes.push(output);
            remapping_tables.push(remap.unwrap_or_default());
        }
    }
    Ok((meshes, remapping_tables))
}

/// The morph target reads `ImportMeshes` makes (their values do not reach
/// the base mesh).
fn check_targets(asset: &Asset, primitive: &asset::Primitive, mesh: &Mesh, all_vertices: usize, remap: Option<&[u32]>) -> Result<(), ImportError> {
    for target in &primitive.targets {
        let needs_normals = !target.normal.is_empty() && mesh.has_normals();
        let needs_tangents = !target.tangent.is_empty() && mesh.has_tangents_and_bitangents();
        if !target.position.is_empty() {
            let Some(slot) = target.position[0] else {
                return fail("GLTF: morph target without POSITION_0");
            };
            if asset.accessor(slot).count as usize == all_vertices {
                extract(asset, slot, 12, remap)?;
            }
        }
        if needs_normals {
            let Some(slot) = target.normal[0] else {
                return fail("GLTF: morph target without NORMAL_0");
            };
            if asset.accessor(slot).count as usize == all_vertices {
                extract(asset, slot, 12, remap)?;
            }
        }
        if needs_tangents && needs_normals {
            let Some(slot) = target.tangent[0] else {
                return fail("GLTF: morph target without TANGENT_0");
            };
            if asset.accessor(slot).count as usize == all_vertices {
                let base = primitive.attributes.tangent[0].unwrap();
                extract(asset, base, 16, remap)?;
                extract(asset, slot, 12, remap)?;
            }
        }
    }
    Ok(())
}

/// The reads `ImportNode` makes for a skinned node's mesh
/// (`BuildVertexWeightMapping` and the inverse bind matrices).
fn check_skinned_node(asset: &Asset, node: usize, mesh_offsets: &[usize], remapping_tables: &[Vec<u32>]) -> Result<(), ImportError> {
    let node_data = &asset.nodes()[node];
    for &child in &node_data.children {
        check_skinned_node(asset, child, mesh_offsets, remapping_tables)?;
    }
    let (Some(mesh), Some(skin)) = (node_data.mesh, node_data.skin) else {
        return Ok(());
    };
    let Some(skin) = asset.skins().get(skin as usize) else {
        return Ok(());
    };
    for (primitive_index, primitive) in asset.meshes()[mesh].primitives.iter().enumerate() {
        let table = &remapping_tables[mesh_offsets[mesh] + primitive_index];
        let remap = if table.is_empty() { None } else { Some(table.as_slice()) };
        let attributes = &primitive.attributes;
        let weight_first = attributes.weight.first().copied().flatten();
        let joint_first = attributes.joint.first().copied().flatten();
        if let (Some(weight_first), Some(joint_first)) = (weight_first, joint_first)
            && asset.accessor(weight_first).count == asset.accessor(joint_first).count {
                for slot in attributes.weight.iter().flatten() {
                    extract(asset, *slot, 16, remap)?;
                }
                let joint = asset.accessor(joint_first);
                let joint_size = joint.components as usize * component_size(joint.component_type)?;
                let target = if joint_size == 4 { 4 } else { 8 };
                for slot in attributes.joint.iter().flatten() {
                    extract(asset, *slot, target, remap)?;
                }
                if attributes.joint.len() < attributes.weight.len() {
                    return fail("GLTF: fewer joint than weight sets");
                }
            }
        let Some(matrices) = skin.inverse_bind_matrices else {
            return fail("GLTF: skin without inverse bind matrices");
        };
        extract(asset, matrices, 64, None)?;
    }
    Ok(())
}

/// The reads `ImportAnimations` makes (`GatherSamplers`, then
/// `CreateNodeAnim`/`CreateMeshMorphAnim` per animated node).
fn check_animations(asset: &Asset) -> Result<(), ImportError> {
    for animation in asset.animations() {
        let mut groups: HashMap<usize, [Option<usize>; 4]> = HashMap::new();
        for channel in &animation.channels {
            if channel.sampler < 0 || channel.sampler >= animation.samplers.len() as i64 {
                continue;
            }
            let sampler = &animation.samplers[channel.sampler as usize];
            let (Some(input), Some(output)) = (sampler.input, sampler.output) else {
                continue;
            };
            if asset.accessor(input).count > asset.accessor(output).count {
                continue;
            }
            let node = channel.node.unwrap_or(0);
            let group = groups.entry(node).or_default();
            let path = match channel.path {
                PATH_TRANSLATION => 0,
                PATH_ROTATION => 1,
                PATH_SCALE => 2,
                PATH_WEIGHTS => 3,
                _ => continue,
            };
            group[path] = Some(channel.sampler as usize);
        }
        for (node, group) in groups {
            if node >= asset.nodes().len() {
                return fail("GLTF: animation of a missing node");
            }
            let output_sizes = [12, 16, 12, 4];
            for (path, sampler) in group.iter().enumerate() {
                let Some(sampler) = sampler else {
                    continue;
                };
                let sampler = &animation.samplers[*sampler];
                let input = sampler.input.unwrap();
                let output = sampler.output.unwrap();
                extract(asset, input, 4, None)?;
                extract(asset, output, output_sizes[path], None)?;
                if path == 3 && asset.accessor(input).count == 0 {
                    return fail("GLTF: morph animation without keys");
                }
            }
        }
    }
    Ok(())
}

/// `glTF2Importer::InternReadFile` for the meshes.
pub fn read(path: &str, bytes: &[u8]) -> Result<Scene, ImportError> {
    let document = asset::read_document(bytes)?;
    let json = asset::parse_json(&document.json)?;
    let asset = Asset::load(path, &json, document.body, document.binary)?;

    let (meshes, remapping_tables) = import_meshes(&asset)?;

    let Some(scene) = asset.scene_definition() else {
        return fail("GLTF: No scene");
    };
    let mut mesh_offsets = Vec::with_capacity(asset.meshes().len());
    let mut offset = 0;
    for mesh in asset.meshes() {
        mesh_offsets.push(offset);
        offset += mesh.primitives.len();
    }
    for &root in &scene.nodes {
        check_skinned_node(&asset, root, &mesh_offsets, &remapping_tables)?;
    }
    check_animations(&asset)?;
    Ok(Scene { meshes })
}
