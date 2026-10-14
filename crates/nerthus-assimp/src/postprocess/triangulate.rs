//! `TriangulateProcess::TriangulateMesh`: quads split along the diagonal
//! from a concave corner (else corner 0), larger polygons ear-clipped in
//! the plane their Newell normal is most aligned with, and every emitted
//! triangle rotated where needed so its first index differs from the
//! previous polygon's ("NGON encoding").

use crate::scene::{ImportError, Mesh, PRIMITIVE_NGON_ENCODING, PRIMITIVE_POLYGON, PRIMITIVE_TRIANGLE, fail};
use crate::vector::{dot, normalize, normalize_2d, sub};

/// `AI_MATH_PI_F` (3.1415926538f, which is π in single precision).
const PI: f32 = std::f32::consts::PI;
/// `ai_epsilon`.
const EPSILON: f32 = 1e-6;

/// `NGONEncoder`: a triangle whose first index equals the previous
/// polygon's would read as part of it, so it gets rotated.
struct NgonEncoder {
    last_first_index: u32,
}

impl NgonEncoder {
    fn is_same_as_last(&self, triangle: &[u32]) -> bool {
        triangle[0] == self.last_first_index
    }

    /// `ngonEncodeTriangle`: rotated right (index 2 first) when needed.
    fn encode_triangle(&mut self, triangle: &mut [u32]) {
        if self.is_same_as_last(triangle) {
            triangle.swap(0, 2);
            triangle.swap(1, 2);
        }
        self.last_first_index = triangle[0];
    }

    /// `ngonEncodeQuad`: both halves fan from the opposite corner instead.
    fn encode_quad(&mut self, first: &mut [u32], second: &mut [u32]) {
        if self.is_same_as_last(first) {
            first.swap(0, 2);
            first.swap(1, 2);
            second.swap(1, 2);
            second.swap(0, 2);
        }
        self.last_first_index = first[0];
    }
}

/// `GetArea2D` in double precision.
fn area_2d(first: [f32; 2], second: [f32; 2], third: [f32; 2]) -> f64 {
    let a = first[0] as f64 * (third[1] as f64 - second[1] as f64);
    let b = second[0] as f64 * (first[1] as f64 - third[1] as f64);
    let c = third[0] as f64 * (second[1] as f64 - first[1] as f64);
    0.5 * ((a + b) + c)
}

/// `OnLeftSideOfLine2D(p0, p1, p2)`: 1 left, −1 right, 0 within epsilon.
fn left_side(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2]) -> i32 {
    let area = area_2d(p0, p2, p1);
    if area.abs() < EPSILON as f64 {
        return 0;
    }
    if area > 0.0 {
        return 1;
    }
    -1
}

/// `PointInTriangle2D`.
fn point_in_triangle(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], point: [f32; 2]) -> bool {
    let first = left_side(p0, p1, point);
    let second = left_side(p1, p2, point);
    let third = left_side(p2, p0, point);
    first >= 0 && second >= 0 && third >= 0
}

/// `NewellNormal<3,3,3>` over the polygon's corners.
fn newell_normal(points: &[[f32; 3]]) -> [f32; 3] {
    let count = points.len();
    let at = |index: usize| points[index % count];
    let mut sum_xy = 0.0f32;
    let mut sum_yz = 0.0f32;
    let mut sum_zx = 0.0f32;
    for corner in 0..count {
        let current = at(corner + 1);
        let low = at(corner);
        let high = at(corner + 2);
        sum_xy += current[0] * (high[1] - low[1]);
        sum_yz += current[1] * (high[2] - low[2]);
        sum_zx += current[2] * (high[0] - low[0]);
    }
    [sum_yz, sum_zx, sum_xy]
}

/// The quad's fanning corner: the first whose two angles to the diagonal
/// add up past π.
fn quad_start(vertices: &[[f32; 3]], face: &[u32]) -> usize {
    for corner in 0..4 {
        let previous = vertices[face[(corner + 3) % 4] as usize];
        let opposite = vertices[face[(corner + 2) % 4] as usize];
        let next = vertices[face[(corner + 1) % 4] as usize];
        let point = vertices[face[corner] as usize];
        let left = normalize(sub(previous, point));
        let diagonal = normalize(sub(opposite, point));
        let right = normalize(sub(next, point));
        let angle = dot(left, diagonal).acos() + dot(right, diagonal).acos();
        if angle > PI {
            return corner;
        }
    }
    0
}

/// Ear clipping of one polygon (its corners as positions into `face`);
/// a polygon without an ear keeps the triangles found so far.
fn clip_ears(vertices: &[[f32; 3]], face: &[u32]) -> Vec<[u32; 3]> {
    let max = face.len();
    let corners: Vec<[f32; 3]> = face.iter().map(|&index| vertices[index as usize]).collect();
    let normal = newell_normal(&corners);
    let ax = if normal[0] > 0.0 { normal[0] } else { -normal[0] };
    let ay = if normal[1] > 0.0 { normal[1] } else { -normal[1] };
    let az = if normal[2] > 0.0 { normal[2] } else { -normal[2] };
    let mut first_axis = 0;
    let mut second_axis = 1;
    let mut inverse = normal[2];
    if ax > ay {
        if ax > az {
            first_axis = 1;
            second_axis = 2;
            inverse = normal[0];
        }
    } else if ay > az {
        first_axis = 2;
        second_axis = 0;
        inverse = normal[1];
    }
    if inverse < 0.0 {
        std::mem::swap(&mut first_axis, &mut second_axis);
    }
    let points: Vec<[f32; 2]> = corners.iter().map(|corner| [corner[first_axis], corner[second_axis]]).collect();

    let mut done = vec![false; max];
    let mut triangles = Vec::new();
    let mut remaining = max;
    let mut previous = max - 1;
    let mut next = 0usize;
    let mut ear;
    while remaining > 3 {
        let mut wraps = 0;
        ear = next;
        loop {
            next = ear + 1;
            loop {
                if next >= max {
                    next = 0;
                }
                if !done[next] {
                    break;
                }
                next += 1;
            }
            if next < ear {
                wraps += 1;
                if wraps == 2 {
                    break;
                }
            }
            let p1 = points[ear];
            let p0 = points[previous];
            let p2 = points[next];
            let is_ear = 'test: {
                if left_side(p0, p2, p1) == 1 {
                    break 'test false;
                }
                let left = normalize_2d([p0[0] - p1[0], p0[1] - p1[1]]);
                let right = normalize_2d([p2[0] - p1[0], p2[1] - p1[1]]);
                let product = left[0] * right[0] + left[1] * right[1];
                if (product - 1.0).abs() < EPSILON || (product + 1.0).abs() < EPSILON {
                    break 'test false;
                }
                let blocked = points.iter().any(|&point| point != p1 && point != p2 && point != p0 && point_in_triangle(p0, p1, p2, point));
                !blocked
            };
            if is_ear {
                break;
            }
            previous = ear;
            ear = next;
        }
        if wraps == 2 {
            return triangles;
        }
        triangles.push([previous as u32, ear as u32, next as u32]);
        done[ear] = true;
        remaining -= 1;
    }
    if remaining > 0 {
        let open: Vec<u32> = (0..max).filter(|&corner| !done[corner]).map(|corner| corner as u32).collect();
        triangles.push([open[0], open[1], open[2]]);
    }
    triangles
}

pub fn triangulate_mesh(mesh: &mut Mesh) -> Result<(), ImportError> {
    let Some(faces) = mesh.faces.as_ref() else {
        return Ok(());
    };
    if mesh.primitive_types == 0 {
        if faces.iter().all(|face| face.len() == 3) {
            return Ok(());
        }
    } else if mesh.primitive_types & PRIMITIVE_POLYGON == 0 {
        return Ok(());
    }
    let output_count: usize = faces.iter().map(|face| if face.len() <= 3 { 1 } else { face.len() - 2 }).sum();
    if output_count == faces.len() {
        return Ok(());
    }
    mesh.primitive_types |= PRIMITIVE_TRIANGLE;
    mesh.primitive_types &= !PRIMITIVE_POLYGON;
    mesh.primitive_types |= PRIMITIVE_NGON_ENCODING;

    let vertices = mesh.vertices.as_deref().unwrap_or(&[]);
    let mut encoder = NgonEncoder { last_first_index: u32::MAX };
    let mut output: Vec<Vec<u32>> = Vec::with_capacity(output_count);
    for face in faces {
        if face.len() <= 3 {
            let mut copy = face.clone();
            if copy.len() == 3 {
                encoder.encode_triangle(&mut copy);
            }
            output.push(copy);
            continue;
        }
        // Larger faces read their corners' positions (a mesh without any
        // has a null array).
        if face.iter().any(|&index| index as usize >= vertices.len()) {
            return fail("undefined behaviour: a polygon's corner past the positions");
        }
        if face.len() == 4 {
            let start = quad_start(vertices, face);
            let mut first = vec![face[start], face[(start + 1) % 4], face[(start + 2) % 4]];
            let mut second = vec![face[start], face[(start + 2) % 4], face[(start + 3) % 4]];
            encoder.encode_quad(&mut first, &mut second);
            output.push(first);
            output.push(second);
            continue;
        }
        for corners in clip_ears(vertices, face) {
            let mut triangle = vec![face[corners[0] as usize], face[corners[1] as usize], face[corners[2] as usize]];
            encoder.encode_triangle(&mut triangle);
            output.push(triangle);
        }
    }
    mesh.faces = Some(output);
    Ok(())
}
