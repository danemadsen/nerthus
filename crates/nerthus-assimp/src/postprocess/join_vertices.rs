//! `JoinVerticesProcess::ProcessMesh`: used vertices that match an earlier
//! one (positions, normals, tangents, bitangents, texture coordinates and
//! colors each within 1e-5, squared) are replaced by it; unused vertices
//! are dropped.
//!
//! The lookup is a `std::unordered_map` keyed by the exact position bits
//! but compared with the epsilon test. An MSVC build compares the new
//! vertex with every vertex already in its bucket (`hash & (buckets − 1)`,
//! buckets the next power of two ≥ the vertex count, at least 8), oldest
//! first, so near-equal vertices whose hashes merely share a bucket merge
//! too.

use crate::scene::{ImportError, MAX_COLOR_SETS, MAX_TEXTURE_COORDS, Mesh, fail};

/// `epsilon * epsilon` with `epsilon = 1e-5f`.
const SQUARE_EPSILON: f32 = 1e-5 * 1e-5;

/// `Vertex`: every stream of one vertex, absent ones zero.
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    tangent: [f32; 3],
    bitangent: [f32; 3],
    texture_coords: [[f32; 3]; MAX_TEXTURE_COORDS],
    colors: [[f32; 4]; MAX_COLOR_SETS],
}

impl Vertex {
    /// `Vertex(const aiMesh*, idx)`: texture coordinate and color sets are
    /// read from 0 up to the first missing one.
    fn from_mesh(mesh: &Mesh, index: usize) -> Self {
        let mut vertex = Vertex {
            position: mesh.vertices.as_ref().unwrap()[index],
            normal: [0.0; 3],
            tangent: [0.0; 3],
            bitangent: [0.0; 3],
            texture_coords: [[0.0; 3]; MAX_TEXTURE_COORDS],
            colors: [[0.0; 4]; MAX_COLOR_SETS],
        };
        if mesh.has_normals() {
            vertex.normal = mesh.normals.as_ref().unwrap()[index];
        }
        if mesh.has_tangents_and_bitangents() {
            vertex.tangent = mesh.tangents.as_ref().unwrap()[index];
            vertex.bitangent = mesh.bitangents.as_ref().unwrap()[index];
        }
        let mut set = 0;
        while mesh.has_texture_coords(set) {
            vertex.texture_coords[set] = mesh.texture_coords[set].as_ref().unwrap()[index];
            set += 1;
        }
        let mut set = 0;
        while mesh.has_vertex_colors(set) {
            vertex.colors[set] = mesh.colors[set].as_ref().unwrap()[index];
            set += 1;
        }
        vertex
    }
}

fn square_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let x = a[0] - b[0];
    let y = a[1] - b[1];
    let z = a[2] - b[2];
    x * x + y * y + z * z
}

/// `GetColorDifference`.
fn color_difference(a: [f32; 4], b: [f32; 4]) -> f32 {
    let r = a[0] - b[0];
    let g = a[1] - b[1];
    let bl = a[2] - b[2];
    let alpha = a[3] - b[3];
    r * r + g * g + bl * bl + alpha * alpha
}

/// `areVerticesEqual` (NaN differences count as equal: only `>` rejects).
fn vertices_equal(left: &Vertex, right: &Vertex, uv_channels: usize, color_channels: usize) -> bool {
    if square_distance(left.position, right.position) > SQUARE_EPSILON {
        return false;
    }
    if square_distance(left.normal, right.normal) > SQUARE_EPSILON {
        return false;
    }
    if square_distance(left.tangent, right.tangent) > SQUARE_EPSILON {
        return false;
    }
    if square_distance(left.bitangent, right.bitangent) > SQUARE_EPSILON {
        return false;
    }
    for set in 0..uv_channels {
        if square_distance(left.texture_coords[set], right.texture_coords[set]) > SQUARE_EPSILON {
            return false;
        }
    }
    for set in 0..color_channels {
        if color_difference(left.colors[set], right.colors[set]) > SQUARE_EPSILON {
            return false;
        }
    }
    true
}

/// MSVC's `std::hash<float>`: FNV-1a over the bytes, −0 as 0.
fn hash_float(value: f32) -> u64 {
    let key = if value == 0.0 { 0.0f32 } else { value };
    let mut hash = 14_695_981_039_346_656_037u64;
    for byte in key.to_le_bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    hash
}

/// Assimp's `hash_combine` over the position components.
fn hash_position(position: [f32; 3]) -> u64 {
    let mut seed = 0u64;
    for component in position {
        let mixed = hash_float(component).wrapping_add(0x9e37_79b9).wrapping_add(seed << 6).wrapping_add(seed >> 2);
        seed ^= mixed;
    }
    seed
}

pub fn process_mesh(mesh: &mut Mesh) -> Result<(), ImportError> {
    if !mesh.has_positions() || !mesh.has_faces() {
        return Ok(());
    }
    let vertex_count = mesh.vertex_count;
    let faces = mesh.faces.as_ref().unwrap();
    let mut used = vec![false; vertex_count];
    for face in faces {
        for &index in face {
            used[index as usize] = true;
        }
    }

    // Every stream read for the used vertices must hold them (a shorter
    // one is read past its end).
    let mut shortest = usize::MAX;
    let streams_3 = [&mesh.vertices, &mesh.normals, &mesh.tangents, &mesh.bitangents];
    for values in streams_3.into_iter().flatten() {
        shortest = shortest.min(values.len());
    }
    for values in mesh.texture_coords.iter().map_while(Option::as_ref) {
        shortest = shortest.min(values.len());
    }
    for values in mesh.colors.iter().map_while(Option::as_ref) {
        shortest = shortest.min(values.len());
    }
    if used.iter().rposition(|&is_used| is_used).is_some_and(|highest| highest >= shortest) {
        return fail("undefined behaviour: a vertex stream shorter than the vertices used");
    }

    let uv_channels = mesh.uv_channel_count();
    let color_channels = mesh.color_channel_count();
    let bucket_count = vertex_count.next_power_of_two().max(8);
    let mask = bucket_count as u64 - 1;
    // Each bucket lists (vertex, new index) oldest first.
    let mut buckets: Vec<Vec<(Vertex, u32)>> = vec![Vec::new(); bucket_count];
    let mut unique = Vec::with_capacity(vertex_count);
    let mut replace = vec![u32::MAX; vertex_count];
    // Whether a vertex became a unique one (rather than joining one, or
    // going unused).
    let mut kept = vec![false; vertex_count];
    for index in 0..vertex_count {
        if !used[index] {
            continue;
        }
        let vertex = Vertex::from_mesh(mesh, index);
        let bucket = &mut buckets[(hash_position(vertex.position) & mask) as usize];
        let found = bucket.iter().find(|(existing, _)| vertices_equal(&vertex, existing, uv_channels, color_channels));
        match found {
            Some(&(_, new_index)) => replace[index] = new_index,
            None => {
                let new_index = unique.len() as u32;
                bucket.push((vertex, new_index));
                replace[index] = new_index;
                kept[index] = true;
                unique.push(index);
            }
        }
    }

    let gather_3 = |stream: &Option<Vec<[f32; 3]>>| stream.as_ref().map(|values| unique.iter().map(|&index| values[index]).collect::<Vec<_>>());
    mesh.vertices = gather_3(&mesh.vertices);
    mesh.normals = gather_3(&mesh.normals);
    mesh.tangents = gather_3(&mesh.tangents);
    mesh.bitangents = gather_3(&mesh.bitangents);
    let mut set = 0;
    while mesh.has_vertex_colors(set) {
        let colors = mesh.colors[set].as_ref().unwrap();
        mesh.colors[set] = Some(unique.iter().map(|&index| colors[index]).collect());
        set += 1;
    }
    let mut set = 0;
    while mesh.has_texture_coords(set) {
        let coordinates = mesh.texture_coords[set].as_ref().unwrap();
        mesh.texture_coords[set] = Some(unique.iter().map(|&index| coordinates[index]).collect());
        set += 1;
    }
    mesh.vertex_count = unique.len();
    for face in mesh.faces.as_mut().unwrap() {
        for index in face.iter_mut() {
            *index = replace[*index as usize];
        }
    }

    // A bone keeps the weights of vertices that became unique ones (all of
    // its old weights when none did). A weight past the vertices reads
    // past the replacement table.
    for bone in &mut mesh.bones {
        let mut weights = Vec::with_capacity(bone.len());
        for &vertex in bone.iter() {
            let Some(&is_kept) = kept.get(vertex as usize) else {
                return fail("undefined behaviour: a bone weight past the vertices");
            };
            if is_kept {
                weights.push(replace[vertex as usize]);
            }
        }
        if !weights.is_empty() {
            *bone = weights;
        }
    }
    Ok(())
}
