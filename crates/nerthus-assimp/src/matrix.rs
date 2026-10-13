//! `aiMatrix4x4` (row-major `a1 … d4`) in Assimp's float operation order.

use crate::vector::{Vector3, cross, normalize};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix4(pub [f32; 16]);

impl Matrix4 {
    pub const IDENTITY: Matrix4 = Matrix4([1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]);

    /// `operator*=`: `self = self × m`, each entry `m.x1·self.r1 + …`.
    pub fn multiply_assign(&mut self, m: &Matrix4) {
        let a = self.0;
        let b = m.0;
        let mut result = [0.0f32; 16];
        for row in 0..4 {
            for column in 0..4 {
                let first = b[column] * a[row * 4];
                let second = b[4 + column] * a[row * 4 + 1];
                let third = b[8 + column] * a[row * 4 + 2];
                let fourth = b[12 + column] * a[row * 4 + 3];
                result[row * 4 + column] = ((first + second) + third) + fourth;
            }
        }
        self.0 = result;
    }

    pub fn product(&self, m: &Matrix4) -> Matrix4 {
        let mut result = *self;
        result.multiply_assign(m);
        result
    }

    /// `Determinant`.
    pub fn determinant(&self) -> f32 {
        let [a1, a2, a3, a4, b1, b2, b3, b4, c1, c2, c3, c4, d1, d2, d3, d4] = self.0;
        let terms = [
            a1 * b2 * c3 * d4,
            -(a1 * b2 * c4 * d3),
            a1 * b3 * c4 * d2,
            -(a1 * b3 * c2 * d4),
            a1 * b4 * c2 * d3,
            -(a1 * b4 * c3 * d2),
            -(a2 * b3 * c4 * d1),
            a2 * b3 * c1 * d4,
            -(a2 * b4 * c1 * d3),
            a2 * b4 * c3 * d1,
            -(a2 * b1 * c3 * d4),
            a2 * b1 * c4 * d3,
            a3 * b4 * c1 * d2,
            -(a3 * b4 * c2 * d1),
            a3 * b1 * c2 * d4,
            -(a3 * b1 * c4 * d2),
            a3 * b2 * c4 * d1,
            -(a3 * b2 * c1 * d4),
            -(a4 * b1 * c2 * d3),
            a4 * b1 * c3 * d2,
            -(a4 * b2 * c3 * d1),
            a4 * b2 * c1 * d3,
            -(a4 * b3 * c1 * d2),
            a4 * b3 * c2 * d1,
        ];
        // Left to right: a − b is a + (−b) exactly.
        let mut sum = terms[0];
        for term in &terms[1..] {
            sum += *term;
        }
        sum
    }

    /// `Inverse` (all NaN when singular).
    pub fn inverse(&self) -> Matrix4 {
        let det = self.determinant();
        if det == 0.0 {
            return Matrix4([f32::NAN; 16]);
        }
        let invdet = 1.0 / det;
        let [a1, a2, a3, a4, b1, b2, b3, b4, c1, c2, c3, c4, d1, d2, d3, d4] = self.0;
        let n = -invdet;
        Matrix4([
            invdet * ((b2 * (c3 * d4 - c4 * d3) + b3 * (c4 * d2 - c2 * d4)) + b4 * (c2 * d3 - c3 * d2)),
            n * ((a2 * (c3 * d4 - c4 * d3) + a3 * (c4 * d2 - c2 * d4)) + a4 * (c2 * d3 - c3 * d2)),
            invdet * ((a2 * (b3 * d4 - b4 * d3) + a3 * (b4 * d2 - b2 * d4)) + a4 * (b2 * d3 - b3 * d2)),
            n * ((a2 * (b3 * c4 - b4 * c3) + a3 * (b4 * c2 - b2 * c4)) + a4 * (b2 * c3 - b3 * c2)),
            n * ((b1 * (c3 * d4 - c4 * d3) + b3 * (c4 * d1 - c1 * d4)) + b4 * (c1 * d3 - c3 * d1)),
            invdet * ((a1 * (c3 * d4 - c4 * d3) + a3 * (c4 * d1 - c1 * d4)) + a4 * (c1 * d3 - c3 * d1)),
            n * ((a1 * (b3 * d4 - b4 * d3) + a3 * (b4 * d1 - b1 * d4)) + a4 * (b1 * d3 - b3 * d1)),
            invdet * ((a1 * (b3 * c4 - b4 * c3) + a3 * (b4 * c1 - b1 * c4)) + a4 * (b1 * c3 - b3 * c1)),
            invdet * ((b1 * (c2 * d4 - c4 * d2) + b2 * (c4 * d1 - c1 * d4)) + b4 * (c1 * d2 - c2 * d1)),
            n * ((a1 * (c2 * d4 - c4 * d2) + a2 * (c4 * d1 - c1 * d4)) + a4 * (c1 * d2 - c2 * d1)),
            invdet * ((a1 * (b2 * d4 - b4 * d2) + a2 * (b4 * d1 - b1 * d4)) + a4 * (b1 * d2 - b2 * d1)),
            n * ((a1 * (b2 * c4 - b4 * c2) + a2 * (b4 * c1 - b1 * c4)) + a4 * (b1 * c2 - b2 * c1)),
            n * ((b1 * (c2 * d3 - c3 * d2) + b2 * (c3 * d1 - c1 * d3)) + b3 * (c1 * d2 - c2 * d1)),
            invdet * ((a1 * (c2 * d3 - c3 * d2) + a2 * (c3 * d1 - c1 * d3)) + a3 * (c1 * d2 - c2 * d1)),
            n * ((a1 * (b2 * d3 - b3 * d2) + a2 * (b3 * d1 - b1 * d3)) + a3 * (b1 * d2 - b2 * d1)),
            invdet * ((a1 * (b2 * c3 - b3 * c2) + a2 * (b3 * c1 - b1 * c3)) + a3 * (b1 * c2 - b2 * c1)),
        ])
    }

    /// `aiMatrix4x4 * aiVector3D`.
    pub fn transform(&self, vector: Vector3) -> Vector3 {
        let m = &self.0;
        let [x, y, z] = vector;
        [
            ((m[0] * x + m[1] * y) + m[2] * z) + m[3],
            ((m[4] * x + m[5] * y) + m[6] * z) + m[7],
            ((m[8] * x + m[9] * y) + m[10] * z) + m[11],
        ]
    }

    pub fn translation(vector: Vector3) -> Matrix4 {
        let mut matrix = Matrix4::IDENTITY;
        matrix.0[3] = vector[0];
        matrix.0[7] = vector[1];
        matrix.0[11] = vector[2];
        matrix
    }

    /// `Rotation(angle, axis)` (`cosf`/`sinf`).
    pub fn rotation(angle: f32, axis: Vector3) -> Matrix4 {
        let c = angle.cos();
        let s = angle.sin();
        let t = 1.0 - c;
        let [x, y, z] = axis;
        Matrix4([
            t * x * x + c,
            t * x * y - s * z,
            t * x * z + s * y,
            0.0,
            t * x * y + s * z,
            t * y * y + c,
            t * y * z - s * x,
            0.0,
            t * x * z - s * y,
            t * y * z + s * x,
            t * z * z + c,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ])
    }

    /// The `lookat` transform Collada nodes may carry.
    pub fn look_at(values: &[f32]) -> Matrix4 {
        let position = [values[0], values[1], values[2]];
        let target = [values[3], values[4], values[5]];
        let up = normalize([values[6], values[7], values[8]]);
        let direction = normalize([target[0] - position[0], target[1] - position[1], target[2] - position[2]]);
        let right = normalize(cross(direction, up));
        Matrix4([
            right[0], up[0], -direction[0], position[0], //
            right[1], up[1], -direction[1], position[1], //
            right[2], up[2], -direction[2], position[2], //
            0.0, 0.0, 0.0, 1.0,
        ])
    }
}
