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

    /// `aiMatrix4x4(scaling, rotation, position)`: the quaternion's
    /// rotation (`GetMatrix`) with its rows scaled, then the position.
    pub fn compose(scaling: Vector3, rotation: [f32; 4], position: Vector3) -> Matrix4 {
        let [x, y, z, w] = rotation;
        let rows = [
            [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
            [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
            [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
        ];
        let mut matrix = Matrix4::IDENTITY;
        for (row, values) in rows.iter().enumerate() {
            for (column, value) in values.iter().enumerate() {
                matrix.0[row * 4 + column] = value * scaling[row];
            }
            matrix.0[row * 4 + 3] = position[row];
        }
        matrix
    }

    /// `Transpose`.
    pub fn transpose(&self) -> Matrix4 {
        let mut result = *self;
        for row in 0..4 {
            for column in 0..4 {
                result.0[row * 4 + column] = self.0[column * 4 + row];
            }
        }
        result
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

    /// `Scaling`.
    pub fn scaling(vector: Vector3) -> Matrix4 {
        let mut matrix = Matrix4::IDENTITY;
        matrix.0[0] = vector[0];
        matrix.0[5] = vector[1];
        matrix.0[10] = vector[2];
        matrix
    }

    /// `FromEulerAnglesXYZ` (`cosf`/`sinf`): the rotation about z, then y,
    /// then x written over the upper 3×3; the rest is left as it was.
    pub fn set_euler_angles_xyz(&mut self, x: f32, y: f32, z: f32) {
        let cx = x.cos();
        let sx = x.sin();
        let cy = y.cos();
        let sy = y.sin();
        let cz = z.cos();
        let sz = z.sin();
        let m = &mut self.0;
        m[0] = cz * cy;
        m[1] = cz * sy * sx - sz * cx;
        m[2] = sz * sx + cz * sy * cx;
        m[4] = sz * cy;
        m[5] = cz * cx + sz * sy * sx;
        m[6] = sz * sy * cx - cz * sx;
        m[8] = -sy;
        m[9] = cy * sx;
        m[10] = cy * cx;
    }

    /// `RotationX` (`cosf`/`sinf`).
    pub fn rotation_x(angle: f32) -> Matrix4 {
        let cos = angle.cos();
        let sin = angle.sin();
        let mut matrix = Matrix4::IDENTITY;
        matrix.0[5] = cos;
        matrix.0[10] = cos;
        matrix.0[9] = sin;
        matrix.0[6] = -sin;
        matrix
    }

    /// `RotationY` (`cosf`/`sinf`).
    pub fn rotation_y(angle: f32) -> Matrix4 {
        let cos = angle.cos();
        let sin = angle.sin();
        let mut matrix = Matrix4::IDENTITY;
        matrix.0[0] = cos;
        matrix.0[10] = cos;
        matrix.0[2] = sin;
        matrix.0[8] = -sin;
        matrix
    }

    /// `RotationZ` (`cosf`/`sinf`).
    pub fn rotation_z(angle: f32) -> Matrix4 {
        let cos = angle.cos();
        let sin = angle.sin();
        let mut matrix = Matrix4::IDENTITY;
        matrix.0[0] = cos;
        matrix.0[5] = cos;
        matrix.0[4] = sin;
        matrix.0[1] = -sin;
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

/// `aiQuaternion(pitch, yaw, roll)` from Euler angles: (x, y, z, w).
pub fn euler_quaternion(pitch: f32, yaw: f32, roll: f32) -> [f32; 4] {
    let sin_pitch = (pitch * 0.5).sin();
    let cos_pitch = (pitch * 0.5).cos();
    let sin_yaw = (yaw * 0.5).sin();
    let cos_yaw = (yaw * 0.5).cos();
    let sin_roll = (roll * 0.5).sin();
    let cos_roll = (roll * 0.5).cos();
    let cos_pitch_cos_yaw = cos_pitch * cos_yaw;
    let sin_pitch_sin_yaw = sin_pitch * sin_yaw;
    let x = sin_roll * cos_pitch_cos_yaw - cos_roll * sin_pitch_sin_yaw;
    let y = cos_roll * sin_pitch * cos_yaw + sin_roll * cos_pitch * sin_yaw;
    let z = cos_roll * cos_pitch * sin_yaw - sin_roll * sin_pitch * cos_yaw;
    let w = cos_roll * cos_pitch_cos_yaw + sin_roll * sin_pitch_sin_yaw;
    [x, y, z, w]
}
