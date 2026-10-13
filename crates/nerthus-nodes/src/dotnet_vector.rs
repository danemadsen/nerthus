//! `System.Numerics.Vector3` arithmetic as .NET 8 evaluates it on x64:
//! `Dot` adds the products left to right (`dpps`) and `Normalize` divides
//! each component by the length.

pub type Vector3 = [f32; 3];

/// `Vector3.Dot`.
pub fn dot(a: Vector3, b: Vector3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `Vector3.Length`.
pub fn length(vector: Vector3) -> f32 {
    dot(vector, vector).sqrt()
}

/// `Vector3.Normalize`.
pub fn normalize(vector: Vector3) -> Vector3 {
    let length = length(vector);
    [vector[0] / length, vector[1] / length, vector[2] / length]
}

/// `a + b`.
pub fn add(a: Vector3, b: Vector3) -> Vector3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// `a * b`, component by component.
pub fn multiply(a: Vector3, b: Vector3) -> Vector3 {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}
