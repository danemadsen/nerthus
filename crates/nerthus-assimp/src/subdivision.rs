//! Assimp's Catmull-Clark subdivider (`Subdivision.cpp`), recursive
//! refinement over several meshes at once: positions within an epsilon
//! (1e-4 of the meshes' bounds) count as one point, every face corner
//! becomes a quad (face point, edge points, the moved original point).
//! Meshes that are only points and lines pass through.

use std::collections::HashMap;

use crate::scene::{ImportError, MAX_COLOR_SETS, MAX_TEXTURE_COORDS, Mesh, PRIMITIVE_LINE, PRIMITIVE_POINT, PRIMITIVE_POLYGON, check_allocation};
use crate::spatial_sort::SpatialSort;
use crate::vector::{Vector3, square_length, sub};

/// `Assimp::Vertex`: every vertex stream, with component-wise arithmetic.
#[derive(Clone, Copy, Default)]
struct Vertex {
    position: Vector3,
    normal: Vector3,
    tangent: Vector3,
    bitangent: Vector3,
    texcoords: [Vector3; MAX_TEXTURE_COORDS],
    colors: [[f32; 4]; MAX_COLOR_SETS],
}

fn add3(a: Vector3, b: Vector3) -> Vector3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale3(a: Vector3, f: f32) -> Vector3 {
    [f * a[0], f * a[1], f * a[2]]
}

impl Vertex {
    /// `Vertex(mesh, index)`: the streams the mesh has.
    fn from_mesh(mesh: &Mesh, index: usize) -> Self {
        let mut vertex = Vertex { position: mesh.vertices.as_ref().unwrap()[index], ..Vertex::default() };
        if mesh.has_normals() {
            vertex.normal = mesh.normals.as_ref().unwrap()[index];
        }
        if mesh.has_tangents_and_bitangents() {
            vertex.tangent = mesh.tangents.as_ref().unwrap()[index];
            vertex.bitangent = mesh.bitangents.as_ref().unwrap()[index];
        }
        let mut set = 0;
        while mesh.has_texture_coords(set) {
            vertex.texcoords[set] = mesh.texture_coords[set].as_ref().unwrap()[index];
            set += 1;
        }
        let mut set = 0;
        while mesh.has_vertex_colors(set) {
            vertex.colors[set] = mesh.colors[set].as_ref().unwrap()[index];
            set += 1;
        }
        vertex
    }

    fn add(&self, other: &Vertex) -> Vertex {
        let mut result = Vertex {
            position: add3(self.position, other.position),
            normal: add3(self.normal, other.normal),
            tangent: add3(self.tangent, other.tangent),
            bitangent: add3(self.bitangent, other.bitangent),
            ..Vertex::default()
        };
        for set in 0..MAX_TEXTURE_COORDS {
            result.texcoords[set] = add3(self.texcoords[set], other.texcoords[set]);
        }
        for set in 0..MAX_COLOR_SETS {
            let (a, b) = (self.colors[set], other.colors[set]);
            result.colors[set] = [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]];
        }
        result
    }

    fn scale(&self, factor: f32) -> Vertex {
        let mut result = Vertex {
            position: scale3(self.position, factor),
            normal: scale3(self.normal, factor),
            tangent: scale3(self.tangent, factor),
            bitangent: scale3(self.bitangent, factor),
            ..Vertex::default()
        };
        for set in 0..MAX_TEXTURE_COORDS {
            result.texcoords[set] = scale3(self.texcoords[set], factor);
        }
        for set in 0..MAX_COLOR_SETS {
            let c = self.colors[set];
            result.colors[set] = [factor * c[0], factor * c[1], factor * c[2], factor * c[3]];
        }
        result
    }

    /// `SortBack`: into the streams the output mesh has.
    fn sort_back(&self, mesh: &mut Mesh, index: usize) {
        mesh.vertices.as_mut().unwrap()[index] = self.position;
        if let Some(normals) = mesh.normals.as_mut() {
            normals[index] = self.normal;
        }
        if let (Some(tangents), Some(bitangents)) = (mesh.tangents.as_mut(), mesh.bitangents.as_mut()) {
            tangents[index] = self.tangent;
            bitangents[index] = self.bitangent;
        }
        for set in 0..MAX_TEXTURE_COORDS {
            let Some(coords) = mesh.texture_coords[set].as_mut() else {
                break;
            };
            coords[index] = self.texcoords[set];
        }
        for set in 0..MAX_COLOR_SETS {
            let Some(colors) = mesh.colors[set].as_mut() else {
                break;
            };
            colors[index] = self.colors[set];
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Edge {
    edge_point: Vertex,
    midpoint: Vertex,
    references: u32,
}

/// `MAKE_EDGE_HASH`: the larger point number low, the smaller high.
fn edge_hash(a: u32, b: u32) -> u64 {
    let (high, low) = if a < b { (a, b) } else { (b, a) };
    low as u64 ^ ((high as u64) << 32)
}

/// `ComputePositionEpsilon` over several meshes.
fn position_epsilon(meshes: &[Mesh]) -> f32 {
    let mut minimum = [1e10f32; 3];
    let mut maximum = [-1e10f32; 3];
    for mesh in meshes {
        let mut low = [1e10f32; 3];
        let mut high = [-1e10f32; 3];
        for position in mesh.vertices.iter().flatten().take(mesh.vertex_count) {
            for axis in 0..3 {
                low[axis] = if low[axis] < position[axis] { low[axis] } else { position[axis] };
                high[axis] = if position[axis] < high[axis] { high[axis] } else { position[axis] };
            }
        }
        for axis in 0..3 {
            minimum[axis] = if low[axis] < minimum[axis] { low[axis] } else { minimum[axis] };
            maximum[axis] = if maximum[axis] < high[axis] { high[axis] } else { maximum[axis] };
        }
    }
    square_length(sub(maximum, minimum)).sqrt() * 1e-4
}

/// `InternSubdivide`: one refinement level over all meshes, then the
/// next.
fn subdivide_level(meshes: &[Mesh], levels: u32) -> Result<Vec<Mesh>, ImportError> {
    // 0. A global numbering of faces and vertices; points by position.
    let mut spatial = SpatialSort::new();
    let mut offsets = Vec::with_capacity(meshes.len());
    let mut total_faces = 0u32;
    let mut total_vertices = 0u32;
    for mesh in meshes {
        let positions = mesh.vertices.as_deref().unwrap_or(&[]);
        spatial.append(&positions[..mesh.vertex_count.min(positions.len())]);
        offsets.push((total_faces, total_vertices));
        total_faces = total_faces.wrapping_add(mesh.face_count() as u32);
        total_vertices = total_vertices.wrapping_add(mesh.vertex_count as u32);
    }
    spatial.finalize();
    let (map, unique_count) = spatial.generate_mapping_table(position_epsilon(meshes));
    let point = |mesh: usize, vertex: u32| map[(offsets[mesh].1 + vertex) as usize];

    // 1. Face points.
    let mut centroids = Vec::with_capacity(total_faces as usize);
    let mut corner_total = 0usize;
    for mesh in meshes {
        for face in mesh.faces.iter().flatten() {
            let mut centroid = Vertex::default();
            for &index in face {
                centroid = centroid.add(&Vertex::from_mesh(mesh, index as usize));
            }
            centroid = centroid.scale(1.0 / face.len() as f32);
            centroids.push(centroid);
            corner_total += face.len();
        }
    }

    // 2. Edge points: the two ends and up to two neighbouring face points.
    let mut edges: HashMap<u64, Edge> = HashMap::new();
    for (mesh_index, mesh) in meshes.iter().enumerate() {
        for (face_index, face) in mesh.faces.iter().flatten().enumerate() {
            for corner in 0..face.len() {
                let first = face[corner];
                let second = face[if corner == face.len() - 1 { 0 } else { corner + 1 }];
                let key = edge_hash(point(mesh_index, first), point(mesh_index, second));
                let edge = edges.entry(key).or_default();
                edge.references += 1;
                if edge.references <= 2 {
                    if edge.references == 1 {
                        let ends = Vertex::from_mesh(mesh, first as usize).add(&Vertex::from_mesh(mesh, second as usize));
                        edge.edge_point = ends;
                        edge.midpoint = ends.scale(0.5);
                    }
                    let centroid = centroids[(offsets[mesh_index].0 as usize) + face_index];
                    edge.edge_point = edge.edge_point.add(&centroid);
                }
            }
        }
    }
    // 3. Averaged.
    for edge in edges.values_mut() {
        edge.edge_point = edge.edge_point.scale(1.0 / (edge.references as f32 + 2.0));
    }

    // 4. The faces around each point, in face order.
    let mut adjacent_count = vec![0u32; map.len()];
    let mut adjacent_start = vec![0u32; map.len() + 1];
    let mut adjacent = vec![0u32; corner_total];
    for (mesh_index, mesh) in meshes.iter().enumerate() {
        for face in mesh.faces.iter().flatten() {
            for &index in face {
                adjacent_count[point(mesh_index, index) as usize] += 1;
            }
        }
    }
    let mut running = 0u32;
    for (group, &count) in adjacent_count.iter().enumerate() {
        adjacent_start[group + 1] = running;
        running += count;
    }
    for (mesh_index, mesh) in meshes.iter().enumerate() {
        for (face_index, face) in mesh.faces.iter().flatten().enumerate() {
            for &index in face {
                let group = point(mesh_index, index) as usize;
                let slot = adjacent_start[group + 1] as usize;
                adjacent[slot] = offsets[mesh_index].0 + face_index as u32;
                adjacent_start[group + 1] += 1;
            }
        }
    }

    // 5. A quad per face corner.
    let mut new_points: Vec<Option<Vertex>> = vec![None; unique_count as usize];
    let mut output = Vec::with_capacity(meshes.len());
    for (mesh_index, mesh) in meshes.iter().enumerate() {
        let face_count: usize = mesh.faces.iter().flatten().map(Vec::len).sum();
        check_allocation(face_count as u64, 16)?;
        check_allocation(face_count as u64 * 4, 12)?;
        let vertex_count = face_count * 4;
        let mut out = Mesh {
            primitive_types: PRIMITIVE_POLYGON,
            material_index: mesh.material_index,
            vertex_count,
            vertices: Some(vec![[0.0; 3]; vertex_count]),
            ..Mesh::default()
        };
        if mesh.has_normals() {
            out.normals = Some(vec![[0.0; 3]; vertex_count]);
        }
        if mesh.has_tangents_and_bitangents() {
            out.tangents = Some(vec![[0.0; 3]; vertex_count]);
            out.bitangents = Some(vec![[0.0; 3]; vertex_count]);
        }
        let mut set = 0;
        while mesh.has_texture_coords(set) {
            out.texture_coords[set] = Some(vec![[0.0; 3]; vertex_count]);
            out.uv_components[set] = mesh.uv_components[set];
            set += 1;
        }
        let mut set = 0;
        while mesh.has_vertex_colors(set) {
            out.colors[set] = Some(vec![[0.0; 4]; vertex_count]);
            set += 1;
        }
        let mut faces = Vec::with_capacity(face_count);
        let mut next = 0u32;
        for (face_index, face) in mesh.faces.iter().flatten().enumerate() {
            let count = face.len();
            for corner in 0..count {
                let centroid = centroids[(offsets[mesh_index].0 as usize) + face_index];
                let indices = vec![next, next + 2, next + 3, next + 1];
                centroid.sort_back(&mut out, next as usize);
                let original = point(mesh_index, face[corner]);
                let after = face[if corner == count - 1 { 0 } else { corner + 1 }];
                let before = face[if corner == 0 { count - 1 } else { corner - 1 }];
                let left = edges.get(&edge_hash(original, point(mesh_index, after))).copied().unwrap_or_default();
                let right = edges.get(&edge_hash(original, point(mesh_index, before))).copied().unwrap_or_default();
                left.edge_point.sort_back(&mut out, next as usize + 1);
                right.edge_point.sort_back(&mut out, next as usize + 2);

                if new_points[original as usize].is_none() {
                    let start = adjacent_start[original as usize] as usize;
                    let around = adjacent_count[original as usize] as usize;
                    let moved = if around < 3 {
                        Vertex::from_mesh(mesh, face[corner] as usize)
                    } else {
                        let mut face_sum = Vertex::default();
                        let mut edge_sum = Vertex::default();
                        for &global_face in &adjacent[start..start + around] {
                            face_sum = face_sum.add(&centroids[global_face as usize]);
                            let owner = (1..=meshes.len()).find(|&candidate| candidate == meshes.len() || offsets[candidate].0 > global_face).map(|candidate| candidate - 1);
                            let Some(owner) = owner else {
                                continue;
                            };
                            let owner_face = &meshes[owner].faces.as_ref().unwrap()[(global_face - offsets[owner].0) as usize];
                            let found = (0..owner_face.len()).find(|&m| point(owner, owner_face[m]) == original);
                            if let Some(m) = found {
                                let previous = owner_face[if m == 0 { owner_face.len() - 1 } else { m - 1 }];
                                let following = owner_face[if m == owner_face.len() - 1 { 0 } else { m + 1 }];
                                let first = edges.get(&edge_hash(original, point(owner, previous))).copied().unwrap_or_default();
                                let second = edges.get(&edge_hash(original, point(owner, following))).copied().unwrap_or_default();
                                edge_sum = edge_sum.add(&first.midpoint.add(&second.midpoint));
                            }
                        }
                        let divisor = around as f32;
                        let inverse_square = 1.0 / (divisor * divisor);
                        let own = Vertex::from_mesh(mesh, face[corner] as usize).scale((divisor - 3.0) / divisor);
                        own.add(&edge_sum.scale(inverse_square)).add(&face_sum.scale(inverse_square))
                    };
                    new_points[original as usize] = Some(moved);
                }
                new_points[original as usize].unwrap().sort_back(&mut out, next as usize + 3);
                // Corners: centroid, right edge, moved point, left edge.
                faces.push(indices);
                next += 4;
            }
        }
        out.faces = Some(faces);
        output.push(out);
    }

    if levels > 1 {
        return subdivide_level(&output, levels - 1);
    }
    Ok(output)
}

/// `CatmullClarkSubdivider::Subdivide` with the input discarded: `levels`
/// refinements of the meshes that aren't only points and lines.
pub(crate) fn catmull_clark(meshes: Vec<Mesh>, levels: u32) -> Result<Vec<Mesh>, ImportError> {
    if levels == 0 {
        return Ok(meshes);
    }
    let passes_through = |mesh: &Mesh| mesh.primitive_types != 0 && mesh.primitive_types & (PRIMITIVE_LINE | PRIMITIVE_POINT) == mesh.primitive_types;
    let worked: Vec<usize> = (0..meshes.len()).filter(|&index| !passes_through(&meshes[index])).collect();
    if worked.is_empty() {
        return Ok(meshes);
    }
    let inputs: Vec<Mesh> = worked.iter().map(|&index| meshes[index].clone()).collect();
    let outputs = subdivide_level(&inputs, levels)?;
    let mut meshes = meshes;
    for (slot, output) in worked.into_iter().zip(outputs) {
        meshes[slot] = output;
    }
    Ok(meshes)
}
