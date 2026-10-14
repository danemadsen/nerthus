//! Assimp's `StandardShapes`: unit solids as triangle lists (positions
//! only), in its float operation order (`ai_real(x)` literals are
//! doubles rounded to floats).

use crate::scene::{ImportError, Mesh, PRIMITIVE_LINE, PRIMITIVE_POINT, PRIMITIVE_POLYGON, PRIMITIVE_TRIANGLE, check_allocation};
use crate::vector::{Vector3, normalize, square_length};

/// `sizeof(aiFace)` and `sizeof(aiVector3D)`.
const FACE_SIZE: u64 = 16;
const VECTOR_SIZE: u64 = 12;

fn scale(vector: Vector3, factor: f32) -> Vector3 {
    [factor * vector[0], factor * vector[1], factor * vector[2]]
}

/// `vector / f`: times `1/f`.
fn divide(vector: Vector3, divisor: f32) -> Vector3 {
    scale(vector, 1.0 / divisor)
}

/// `integer_pow` on 32-bit unsigned integers.
pub(crate) fn integer_pow(base: u32, power: u32) -> u32 {
    let mut result = 1u32;
    for _ in 0..power.min(64) {
        result = result.wrapping_mul(base);
    }
    result
}

/// `Subdivide`: each triangle split in four, the new points pushed out to
/// the first position's distance from the origin.
fn subdivide(positions: &mut Vec<Vector3>) {
    let radius = square_length(positions[0]).sqrt();
    let original = positions.len();
    let mut index = 0;
    while index < original {
        let (a, b, c) = (positions[index], positions[index + 1], positions[index + 2]);
        let v1 = scale(normalize([a[0] + b[0], a[1] + b[1], a[2] + b[2]]), radius);
        let v2 = scale(normalize([a[0] + c[0], a[1] + c[1], a[2] + c[2]]), radius);
        let v3 = scale(normalize([b[0] + c[0], b[1] + c[1], b[2] + c[2]]), radius);
        positions[index] = v1;
        positions[index + 1] = v3;
        positions[index + 2] = v2;
        positions.extend_from_slice(&[v1, v2, a, v2, v3, c, v3, v1, b]);
        index += 3;
    }
}

/// `MakeIcosahedron`.
pub(crate) fn icosahedron(positions: &mut Vec<Vector3>) {
    let t = (1.0f32 + 2.236067977f64 as f32) / 2.0;
    let s = (1.0f32 + t * t).sqrt();
    let v = [
        divide([t, 1.0, 0.0], s),
        divide([-t, 1.0, 0.0], s),
        divide([t, -1.0, 0.0], s),
        divide([-t, -1.0, 0.0], s),
        divide([1.0, 0.0, t], s),
        divide([1.0, 0.0, -t], s),
        divide([-1.0, 0.0, t], s),
        divide([-1.0, 0.0, -t], s),
        divide([0.0, t, 1.0], s),
        divide([0.0, -t, 1.0], s),
        divide([0.0, t, -1.0], s),
        divide([0.0, -t, -1.0], s),
    ];
    let triangles = [
        [0, 8, 4],
        [0, 5, 10],
        [2, 4, 9],
        [2, 11, 5],
        [1, 6, 8],
        [1, 10, 7],
        [3, 9, 6],
        [3, 7, 11],
        [0, 10, 8],
        [1, 8, 10],
        [2, 9, 11],
        [3, 11, 9],
        [4, 2, 0],
        [5, 0, 2],
        [6, 1, 3],
        [7, 3, 1],
        [8, 6, 4],
        [9, 4, 6],
        [10, 5, 7],
        [11, 7, 5],
    ];
    for triangle in triangles {
        positions.extend(triangle.iter().map(|&corner| v[corner]));
    }
}

/// `MakeDodecahedron` as triangles (each pentagon a fan), with Assimp's
/// literals as written.
#[allow(clippy::excessive_precision)]
pub(crate) fn dodecahedron(positions: &mut Vec<Vector3>) {
    let a = 1.0f32 / (1.7320508f64 as f32);
    let b = ((3.0f32 - 2.23606797f64 as f32) / 6.0).sqrt();
    let c = ((3.0f32 + 2.23606797f32) / 6.0).sqrt();
    let v = [
        [a, a, a],
        [a, a, -a],
        [a, -a, a],
        [a, -a, -a],
        [-a, a, a],
        [-a, a, -a],
        [-a, -a, a],
        [-a, -a, -a],
        [b, c, 0.0],
        [-b, c, 0.0],
        [b, -c, 0.0],
        [-b, -c, 0.0],
        [c, 0.0, b],
        [c, 0.0, -b],
        [-c, 0.0, b],
        [-c, 0.0, -b],
        [0.0, b, c],
        [0.0, -b, c],
        [0.0, b, -c],
        [0.0, -b, -c],
    ];
    let pentagons = [
        [0, 8, 9, 4, 16],
        [0, 12, 13, 1, 8],
        [0, 16, 17, 2, 12],
        [8, 1, 18, 5, 9],
        [12, 2, 10, 3, 13],
        [16, 4, 14, 6, 17],
        [9, 5, 15, 14, 4],
        [6, 11, 10, 2, 17],
        [3, 19, 18, 1, 13],
        [7, 15, 5, 18, 19],
        [7, 11, 6, 14, 15],
        [7, 19, 3, 10, 11],
    ];
    for p in pentagons {
        for triangle in [[p[0], p[1], p[2]], [p[0], p[2], p[3]], [p[0], p[3], p[4]]] {
            positions.extend(triangle.iter().map(|&corner| v[corner]));
        }
    }
}

/// `MakeOctahedron`.
pub(crate) fn octahedron(positions: &mut Vec<Vector3>) {
    let v = [[1.0f32, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]];
    let triangles = [[4, 0, 2], [4, 2, 1], [4, 1, 3], [4, 3, 0], [5, 2, 0], [5, 1, 2], [5, 3, 1], [5, 0, 3]];
    for triangle in triangles {
        positions.extend(triangle.iter().map(|&corner| v[corner]));
    }
}

/// `MakeTetrahedron`, with Assimp's rounded √2 and √6.
#[allow(clippy::approx_constant)]
pub(crate) fn tetrahedron(positions: &mut Vec<Vector3>) {
    let third = 1.0f32 / 3.0;
    let a = (1.41421f64 as f32) * third;
    let b = (2.4494f64 as f32) * third;
    let v = [[0.0f32, 0.0, 1.0], [2.0 * a, 0.0, -third], [-a, b, -third], [-a, -b, -third]];
    let triangles = [[0, 1, 2], [0, 2, 3], [0, 3, 1], [1, 3, 2]];
    for triangle in triangles {
        positions.extend(triangle.iter().map(|&corner| v[corner]));
    }
}

/// `MakeHexahedron`: six quads, or each quad as two triangles. The
/// vertices a face has.
pub(crate) fn hexahedron(positions: &mut Vec<Vector3>, polygons: bool) -> u32 {
    let length = 1.0f32 / (1.73205080f64 as f32);
    let corners = [
        [-1.0f32, -1.0, -1.0],
        [1.0, -1.0, -1.0],
        [1.0, 1.0, -1.0],
        [-1.0, 1.0, -1.0],
        [-1.0, -1.0, 1.0],
        [1.0, -1.0, 1.0],
        [1.0, 1.0, 1.0],
        [-1.0, 1.0, 1.0],
    ];
    let v = corners.map(|corner| scale(corner, length));
    let quads = [[0, 3, 2, 1], [0, 1, 5, 4], [0, 4, 7, 3], [6, 5, 1, 2], [6, 2, 3, 7], [6, 7, 4, 5]];
    for q in quads {
        if polygons {
            positions.extend(q.iter().map(|&corner| v[corner]));
            continue;
        }
        for triangle in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
            positions.extend(triangle.iter().map(|&corner| v[corner]));
        }
    }
    if polygons { 4 } else { 3 }
}

/// `MakeSphere`: an icosahedron subdivided `tessellation` times (failing
/// as the C++'s allocations would past 2 GiB).
pub(crate) fn sphere(tessellation: u32, positions: &mut Vec<Vector3>) -> Result<(), ImportError> {
    let final_count = 60u64.saturating_mul(4u64.saturating_pow(tessellation.min(32)));
    check_allocation(final_count, 12)?;
    icosahedron(positions);
    for _ in 0..tessellation {
        subdivide(positions);
    }
    Ok(())
}

/// `MakeCircle`: `tessellation` triangles around the origin in the XZ
/// plane (none for fewer than three segments or a zero radius).
pub(crate) fn circle(radius: f32, tessellation: u32, positions: &mut Vec<Vector3>) -> Result<(), ImportError> {
    if tessellation < 3 || radius == 0.0 {
        return Ok(());
    }
    let radius = radius.abs();
    check_allocation(tessellation as u64 * 3, VECTOR_SIZE)?;
    let two_pi = std::f64::consts::TAU as f32;
    let delta = two_pi / tessellation as f32;
    let mut s = 1.0f32;
    let mut t = 0.0f32;
    let mut angle = 0.0f32;
    while angle < two_pi {
        positions.push([s * radius, 0.0, t * radius]);
        angle += delta;
        s = angle.cos();
        t = angle.sin();
        positions.push([s * radius, 0.0, t * radius]);
        positions.push([0.0, 0.0, 0.0]);
    }
    Ok(())
}

/// `MakeMesh(positions, numIndices)`: every position a vertex, and faces
/// of `index_count` positions each, in order (a short last run left
/// over); none without positions or with no indices per face.
pub(crate) fn make_mesh(positions: &[Vector3], index_count: u32) -> Result<Option<Mesh>, ImportError> {
    if positions.is_empty() || index_count == 0 {
        return Ok(None);
    }
    let primitive_types = match index_count {
        1 => PRIMITIVE_POINT,
        2 => PRIMITIVE_LINE,
        3 => PRIMITIVE_TRIANGLE,
        _ => PRIMITIVE_POLYGON,
    };
    let face_count = positions.len() as u32 / index_count;
    check_allocation(face_count as u64, FACE_SIZE)?;
    let mut faces = Vec::with_capacity(face_count as usize);
    for face in 0..face_count {
        let first = face * index_count;
        let corners: Vec<u32> = (first..first + index_count).collect();
        faces.push(corners);
    }
    check_allocation(positions.len() as u64, VECTOR_SIZE)?;
    let mesh = Mesh {
        primitive_types,
        vertex_count: positions.len(),
        vertices: Some(positions.to_vec()),
        faces: Some(faces),
        ..Mesh::default()
    };
    Ok(Some(mesh))
}

/// `MakeCone`: `tessellation` segments around, the wider end at
/// -height/2 (the winding flipped when the second radius was wider), and
/// caps on the ends unless `open`.
pub(crate) fn cone(height: f32, radius1: f32, radius2: f32, tessellation: u32, open: bool, positions: &mut Vec<Vector3>) -> Result<(), ImportError> {
    if tessellation < 3 || height == 0.0 {
        return Ok(());
    }
    let mut radius1 = radius1.abs();
    let mut radius2 = radius2.abs();
    let mut half_height = height / 2.0;
    let mut flip_from = Some(positions.len());
    if radius2 > radius1 {
        std::mem::swap(&mut radius1, &mut radius2);
        half_height = -half_height;
    } else {
        flip_from = None;
    }
    if (radius1 as f64) < (radius2 - radius1) as f64 * 10e-3 {
        radius1 = 0.0;
    }
    let caps = if open { 0 } else { 3 * if radius1 != 0.0 { 2 } else { 1 } };
    check_allocation(tessellation as u64 * (6 + caps), VECTOR_SIZE)?;
    let two_pi = std::f64::consts::TAU as f32;
    let delta = two_pi / tessellation as f32;
    let mut s = 1.0f32;
    let mut t = 0.0f32;
    let mut angle = 0.0f32;
    while angle < two_pi {
        let v1 = [s * radius1, -half_height, t * radius1];
        let v2 = [s * radius2, half_height, t * radius2];
        let next = angle + delta;
        let s2 = next.cos();
        let t2 = next.sin();
        let v3 = [s2 * radius2, half_height, t2 * radius2];
        let v4 = [s2 * radius1, -half_height, t2 * radius1];
        positions.extend_from_slice(&[v1, v2, v3, v4, v1, v3]);
        if !open {
            positions.push([s * radius2, half_height, t * radius2]);
            positions.push([s2 * radius2, half_height, t2 * radius2]);
            positions.push([0.0, half_height, 0.0]);
            if radius1 != 0.0 {
                positions.push([s * radius1, -half_height, t * radius1]);
                positions.push([s2 * radius1, -half_height, t2 * radius1]);
                positions.push([0.0, -half_height, 0.0]);
            }
        }
        s = s2;
        t = t2;
        angle = next;
    }
    if let Some(start) = flip_from {
        let mut index = start;
        while index < positions.len() {
            positions.swap(index, index + 1);
            index += 3;
        }
    }
    Ok(())
}
