//! `aiVector3D`/`aiVector2D` arithmetic in Assimp's operation order.

use std::cmp::Ordering;

pub type Vector3 = [f32; 3];

pub fn sub(a: Vector3, b: Vector3) -> Vector3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// `operator*` of two vectors: `x·x' + y·y' + z·z'`, left to right.
pub fn dot(a: Vector3, b: Vector3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `operator^`.
pub fn cross(a: Vector3, b: Vector3) -> Vector3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// `vector * f` (`f·x, f·y, f·z`).
pub fn scale(vector: Vector3, factor: f32) -> Vector3 {
    [factor * vector[0], factor * vector[1], factor * vector[2]]
}

/// `SquareLength`.
pub fn square_length(vector: Vector3) -> f32 {
    vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]
}

/// `aiVector3D::Normalize`: unchanged when the length is zero, else
/// multiplied by the reciprocal length.
pub fn normalize(vector: Vector3) -> Vector3 {
    let length = square_length(vector).sqrt();
    if length == 0.0 {
        return vector;
    }
    let inverse = 1.0 / length;
    [vector[0] * inverse, vector[1] * inverse, vector[2] * inverse]
}

/// `aiVector3D::NormalizeSafe`: multiplied by the reciprocal length when
/// the length is positive.
pub fn normalize_safe(vector: Vector3) -> Vector3 {
    let length = square_length(vector).sqrt();
    let positive = length.partial_cmp(&0.0) == Some(Ordering::Greater);
    if !positive {
        return vector;
    }
    let inverse = 1.0 / length;
    [vector[0] * inverse, vector[1] * inverse, vector[2] * inverse]
}

/// `aiVector2D::Normalize`: both components divided by the length.
pub fn normalize_2d(vector: [f32; 2]) -> [f32; 2] {
    let length = (vector[0] * vector[0] + vector[1] * vector[1]).sqrt();
    [vector[0] / length, vector[1] / length]
}
