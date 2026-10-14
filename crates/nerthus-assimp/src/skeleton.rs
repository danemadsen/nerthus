//! `SkeletonMeshBuilder`: a mesh standing for a node hierarchy, for scenes
//! that come out without meshes. A node with children gets a pointer to
//! each child (none to a child at its origin); a leaf, or every node when
//! only knobs are asked for, gets an eight-sided knob sized by its
//! distance from its parent. Each node's part is moved by the inverse of
//! its bone offset (the inverse transforms of the node and its parents).

use crate::node::Node;
use crate::scene::Mesh;
use crate::vector::{Vector3, cross, dot, normalize, scale, square_length, sub};

/// `CreateGeometry`.
fn create_geometry(nodes: &[Node], node: usize, knobs_only: bool, vertices: &mut Vec<Vector3>, faces: &mut Vec<[u32; 3]>) {
    let start = vertices.len();
    let ai_node = &nodes[node];
    if !ai_node.children.is_empty() && !knobs_only {
        for &child in &ai_node.children {
            let child_transform = &nodes[child].transform.0;
            let child_position = [child_transform[3], child_transform[7], child_transform[11]];
            let distance = square_length(child_position).sqrt();
            if distance < 1e-6 {
                continue;
            }
            let up = normalize(child_position);
            let mut orthogonal = [1.0, 0.0, 0.0];
            if dot(orthogonal, up).abs() as f64 > 0.99 {
                orthogonal = [0.0, 1.0, 0.0];
            }
            let front = normalize(cross(up, orthogonal));
            let side = normalize(cross(front, up));
            let negative_front = [-front[0], -front[1], -front[2]];
            let negative_side = [-side[0], -side[1], -side[2]];
            let back = scale(scale(negative_front, distance), 0.1);
            let forward = scale(scale(front, distance), 0.1);
            let left = scale(scale(negative_side, distance), 0.1);
            let right = scale(scale(side, distance), 0.1);
            let local = vertices.len() as u32;
            let pointer = [back, child_position, left, left, child_position, forward, forward, child_position, right, right, child_position, back];
            vertices.extend_from_slice(&pointer);
            for face in 0..4 {
                let first = local + face * 3;
                faces.push([first, first + 1, first + 2]);
            }
        }
    } else {
        let own = &ai_node.transform.0;
        let own_position = [own[3], own[7], own[11]];
        let size = square_length(own_position).sqrt() * 0.18;
        let knob: [Vector3; 24] = [
            [-size, 0.0, 0.0],
            [0.0, size, 0.0],
            [0.0, 0.0, -size],
            [0.0, size, 0.0],
            [size, 0.0, 0.0],
            [0.0, 0.0, -size],
            [size, 0.0, 0.0],
            [0.0, -size, 0.0],
            [0.0, 0.0, -size],
            [0.0, -size, 0.0],
            [-size, 0.0, 0.0],
            [0.0, 0.0, -size],
            [-size, 0.0, 0.0],
            [0.0, 0.0, size],
            [0.0, size, 0.0],
            [0.0, size, 0.0],
            [0.0, 0.0, size],
            [size, 0.0, 0.0],
            [size, 0.0, 0.0],
            [0.0, 0.0, size],
            [0.0, -size, 0.0],
            [0.0, -size, 0.0],
            [0.0, 0.0, size],
            [-size, 0.0, 0.0],
        ];
        vertices.extend_from_slice(&knob);
        let first_vertex = start as u32;
        for face in 0..8 {
            let first = first_vertex + face * 3;
            faces.push([first, first + 1, first + 2]);
        }
    }

    if vertices.len() > start {
        // The bone offset is the inverse chain of transforms; its inverse
        // moves the vertices.
        let mut offset = ai_node.transform.inverse();
        let mut parent = ai_node.parent;
        while let Some(current) = parent {
            let parent_inverse = nodes[current].transform.inverse();
            offset = parent_inverse.product(&offset);
            parent = nodes[current].parent;
        }
        let bone_to_mesh = offset.inverse();
        for vertex in &mut vertices[start..] {
            *vertex = bone_to_mesh.transform(*vertex);
        }
    }
    for &child in &ai_node.children {
        create_geometry(nodes, child, knobs_only, vertices, faces);
    }
}

/// `SkeletonMeshBuilder` from `root` (`CreateGeometry`, then `CreateMesh`
/// with per-face, unnormalised normals).
pub(crate) fn build_mesh(nodes: &[Node], root: usize, knobs_only: bool) -> Mesh {
    let mut vertices = Vec::new();
    let mut faces = Vec::new();
    create_geometry(nodes, root, knobs_only, &mut vertices, &mut faces);
    let mut normals = vec![[0.0f32; 3]; vertices.len()];
    for face in &faces {
        let [first, second, third] = face.map(|index| vertices[index as usize]);
        let mut normal = cross(sub(third, first), sub(second, first));
        if (square_length(normal).sqrt() as f64) < 1e-5 {
            normal = [1.0, 0.0, 0.0];
        }
        for &index in face {
            normals[index as usize] = normal;
        }
    }
    let face_lists = faces.iter().map(|face| face.to_vec()).collect();
    Mesh { vertex_count: vertices.len(), vertices: Some(vertices), normals: Some(normals), faces: Some(face_lists), ..Mesh::default() }
}
