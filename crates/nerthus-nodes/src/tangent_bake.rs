//! `MeshRasterizer_ComputeTangentSpaceNormals` (SYCL kernels), the
//! bake behind TextureBaker's and MeshWarp's exports.
//! For every pixel of a heightmap it follows the mesh's interpolated normal
//! from the mesh's surface to the heightmap's, keeps where it lands (the
//! warp) and expresses the heightmap's normal there in the mesh's tangent
//! space (the normal map).
//!
//! The host gives every vertex an angle-weighted normal and x tangent
//! (1, 0, ∂z/∂x) from the mesh as built. The kernels then scale the mesh to
//! pixels (`lambda_1`), store which triangle covers each pixel
//! (`render3_cpu<0,0,0,1>`), take the heightmap's normals with a Scharr
//! filter (`compute_global_normals`) and march each pixel's ray
//! (`lambda_3`). Every operation keeps the clang build's FMA contractions.
//! Triangles are rasterised in order, so where grown triangles overlap the
//! highest index wins (threads would race there).

use rayon::prelude::*;

use crate::maths;
use crate::mesh_rasterizer::{GrownTriangle, Mesh, MISS};
use crate::x86::{cvttss2si, maxss, minss, rsqrt};

/// The Newton step's constants.
const NEWTON_HALF: f32 = -0.5;
const NEWTON_THREE: f32 = -3.0;
/// `compute_global_normals`' z before it is divided by the terrain's height
/// in pixels.
const GLOBAL_NORMAL_Z: f32 = 32.0;
/// The ray's refinement once it has crossed the heightmap.
const BISECTION_STEPS: usize = 32;

/// What the bake produces at the heightmap's resolution.
pub struct TangentBake {
    /// The normal map: tangent, negated bitangent and normal components
    /// mapped from [−1, 1] to [0, 1].
    pub red: Vec<f32>,
    pub green: Vec<f32>,
    pub blue: Vec<f32>,
    /// Where each pixel's ray met the heightmap, in pixels.
    pub warp: Vec<[f32; 2]>,
}

/// `RSQRTSS` refined by one Newton step, as every kernel inlines it:
/// (r · −0.5) · fma(r, s · r, −3).
fn inverse_length(squared: f32) -> f32 {
    let estimate = rsqrt(squared);
    let refined = estimate.mul_add(squared * estimate, NEWTON_THREE);
    (estimate * NEWTON_HALF) * refined
}

/// x² first, then y² and z² fused in.
fn squared_length([x, y, z]: [f32; 3]) -> f32 {
    z.mul_add(z, y.mul_add(y, x * x))
}

fn scaled([x, y, z]: [f32; 3], factor: f32) -> [f32; 3] {
    [x * factor, y * factor, z * factor]
}

fn difference(to: [f32; 3], from: [f32; 3]) -> [f32; 3] {
    [to[0] - from[0], to[1] - from[1], to[2] - from[2]]
}

/// `MeshRasterizer_ComputeTangentSpaceNormals` for `mesh` (as
/// `HeightmapToMesh` builds it: x and y over [0, 1], heights times `ratio`)
/// over `heights` (`width` × `height`). `ratio` is the terrain's height over
/// its width.
pub fn compute_tangent_space_normals(mesh: &Mesh, heights: &[f32], width: usize, height: usize, ratio: f32) -> TangentBake {
    let pixel_count = width * height;
    if mesh.triangles.is_empty() || pixel_count == 0 {
        // The triangle buffer is unset here; there is nothing to follow.
        let half = vec![0.5; pixel_count];
        return TangentBake { red: half.clone(), green: half.clone(), blue: half, warp: vec![[0.0, 0.0]; pixel_count] };
    }
    let (normals, tangents) = vertex_frames(mesh);

    // `lambda_1`: the mesh in pixels, heights back in heightmap units.
    let width_f = width as f32;
    let height_f = height as f32;
    let inverse_ratio = 1.0 / ratio;
    let vertices: Vec<[f32; 3]> = mesh.vertices.iter().map(|&[x, y, z]| [width_f * x, height_f * y, inverse_ratio * z]).collect();

    let ids = triangle_ids(&vertices, &mesh.triangles, width as i32, height as i32);
    let global_z = GLOBAL_NORMAL_Z / (ratio * width_f);
    let global = global_normals(heights, width, height, global_z);

    let bake = PixelBake {
        width: width as i32,
        height: height as i32,
        ids: &ids,
        triangles: &mesh.triangles,
        tangents: &tangents,
        normals: &normals,
        vertices: &vertices,
        heights,
        scale: width_f * ratio,
        global: &global,
    };
    let pixels: Vec<([f32; 2], [f32; 3])> = (0..pixel_count)
        .into_par_iter()
        .map(|index| bake.pixel((index % width) as i32, (index / width) as i32))
        .collect();

    let mut result = TangentBake {
        red: Vec::with_capacity(pixel_count),
        green: Vec::with_capacity(pixel_count),
        blue: Vec::with_capacity(pixel_count),
        warp: Vec::with_capacity(pixel_count),
    };
    for (warp, [red, green, blue]) in pixels {
        result.warp.push(warp);
        result.red.push(red);
        result.green.push(green);
        result.blue.push(blue);
    }
    result
}

/// The host's vertex frames: per triangle, the unit normal of −(B − A) ×
/// (C − A) and the x tangent (1, 0, ∂z/∂x), each added at a corner times
/// the corner's angle, then normalised. The angle at the first corner is
/// π − A: the cosine comes from e1 · e3, where the edges leaving A are
/// e1 and −e3.
fn vertex_frames(mesh: &Mesh) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
    let vertex_count = mesh.vertices.len();
    let mut normals = vec![[0.0f32; 3]; vertex_count];
    let mut tangents = vec![[0.0f32; 3]; vertex_count];
    for triangle in &mesh.triangles {
        let [first, second, third] = triangle.map(|index| index as usize);
        let a = mesh.vertices[first];
        let b = mesh.vertices[second];
        let c = mesh.vertices[third];

        let e1 = difference(b, a);
        let e2 = difference(c, b);
        let e3 = difference(a, c);
        let n1 = scaled(e1, inverse_length(squared_length(e1)));
        let n2 = scaled(e2, inverse_length(squared_length(e2)));
        let n3 = scaled(e3, inverse_length(squared_length(e3)));
        let cos_a = n3[2].mul_add(n1[2], n3[1].mul_add(n1[1], n3[0] * n1[0]));
        let cos_b = (-n1[1]).mul_add(n2[1], -n1[2].mul_add(n2[2], n2[0] * n1[0]));
        let cos_c = (-n2[2]).mul_add(n3[2], -n3[0].mul_add(n2[0], n2[1] * n3[1]));

        let f2 = difference(c, a);
        let cross_x = e1[2].mul_add(f2[1], -(f2[2] * e1[1]));
        let cross_y = f2[2].mul_add(e1[0], -(e1[2] * f2[0]));
        let forward = e1[0] * f2[1];
        let backward = e1[1] * f2[0];
        let cross_z = backward - forward;
        let inverse = inverse_length(squared_length([cross_z, cross_x, cross_y]));
        let normal = [cross_x * inverse, cross_y * inverse, cross_z * inverse];

        let area = forward - backward;
        let inverse_area = 1.0 / area;
        let tangent_x = inverse_area * area;
        let tangent_z = cross_x * inverse_area;

        let angle_a = maths::acos(cos_a);
        let angle_b = maths::acos(cos_b);
        let angle_c = maths::acos(cos_c);
        let corners = [(first, angle_a), (second, angle_b), (third, angle_c)];
        for (corner, angle) in corners {
            for axis in 0..3 {
                normals[corner][axis] = angle.mul_add(normal[axis], normals[corner][axis]);
            }
        }
        for (corner, angle) in corners {
            tangents[corner][0] = angle.mul_add(tangent_x, tangents[corner][0]);
            tangents[corner][2] = tangent_z.mul_add(angle, tangents[corner][2]);
        }
    }
    for frame in normals.iter_mut().chain(tangents.iter_mut()) {
        let inverse = inverse_length(squared_length(*frame));
        *frame = scaled(*frame, inverse);
    }
    (normals, tangents)
}

/// `render3_cpu<0,0,0,1>`: every triangle facing the right way, in order,
/// writes its index at each pixel of its bounding box whose centre lies in
/// the grown triangle.
fn triangle_ids(vertices: &[[f32; 3]], triangles: &[[u32; 3]], width: i32, height: i32) -> Vec<u32> {
    let mut ids = vec![0u32; width.max(0) as usize * height.max(0) as usize];
    for (index, triangle) in triangles.iter().enumerate() {
        let a = vertices[triangle[0] as usize];
        let b = vertices[triangle[1] as usize];
        let c = vertices[triangle[2] as usize];
        let first = (c[1] - a[1]) * (b[0] - a[0]);
        let second = (c[0] - a[0]) * (b[1] - a[1]);
        if first > second {
            continue;
        }
        let low_y = cvttss2si(minss(a[1], minss(c[1], b[1])).floor());
        let high_y = cvttss2si(maxss(a[1], maxss(c[1], b[1])).ceil());
        let high_x = cvttss2si(maxss(a[0], maxss(b[0], c[0])).ceil());
        let row_start = low_y.max(0);
        let row_end = if high_y < height - 1 { high_y } else { height - 1 };
        let column_end = if high_x < width - 1 { high_x } else { width - 1 };
        if row_end < row_start {
            continue;
        }
        let low_x = cvttss2si(minss(a[0], minss(b[0], c[0])).floor());
        let column_start = low_x.max(0);
        if column_end < column_start {
            continue;
        }
        let grown = GrownTriangle::new(a, b, c);
        for y in row_start..=row_end {
            for x in column_start..=column_end {
                let depth = grown.sample_inlined(x, y, a, b, c);
                if depth > MISS {
                    let cell = width.wrapping_mul(y).wrapping_add(x) as u32 as usize;
                    ids[cell] = index as u32;
                }
            }
        }
    }
    ids
}

/// `compute_global_normals`: the heightmap's normal at each pixel from a
/// Scharr filter (3, 10, 3) over its clamped neighbours, with `z` up:
/// (−∂x, ∂y, z) normalised.
fn global_normals(heights: &[f32], width: usize, height: usize, z: f32) -> Vec<[f32; 3]> {
    let z_squared = z * z;
    (0..width * height)
        .into_par_iter()
        .map(|index| {
            let row = index / width;
            let column = index % width;
            let left = column - usize::from(column != 0);
            let right = (column + 1).min(width - 1);
            let above = row.saturating_sub(1) * width;
            let here = row * width;
            let below = (row + 1).min(height - 1) * width;

            let top_left = heights[above + left];
            let top = heights[above + column];
            let top_right = heights[above + right];
            let bottom_left = heights[below + left];
            let bottom = heights[below + column];
            let bottom_right = heights[below + right];
            let across = heights[here + right] - heights[here + left];
            let down = top - bottom;

            let corners_x = (top_right - (bottom_left + top_left)) + bottom_right;
            let slope_x = 3.0f32.mul_add(corners_x, across * 10.0);
            let corners_y = (top_right + top_left) - (bottom_left + bottom_right);
            let slope_y = 3.0f32.mul_add(corners_y, down * 10.0);
            let squared = slope_y.mul_add(slope_y, slope_x.mul_add(slope_x, z_squared));
            let inverse = inverse_length(squared);
            [inverse * -slope_x, slope_y * inverse, inverse * z]
        })
        .collect()
}

/// The four map cells around a sample point, as the per-pixel kernel finds
/// them: the first corner truncated and clamped into the map, the second
/// one past it (counting from −1 below zero) clamped too, and the fractions
/// taken from the clamped first corner.
struct Cell {
    top_left: usize,
    top_right: usize,
    bottom_left: usize,
    bottom_right: usize,
    fraction_x: f32,
    fraction_y: f32,
}

fn cell(x: f32, y: f32, width: i32, height: i32) -> Cell {
    let truncated_x = cvttss2si(x);
    let truncated_y = cvttss2si(y);
    let first_x = truncated_x.max(0).min(width - 1);
    let first_y = truncated_y.max(0).min(height - 1);
    let second_x = (truncated_x.max(-1) + 1).min(width - 1);
    let second_y = (truncated_y.max(-1) + 1).min(height - 1);
    let top = (first_y * width) as usize;
    let bottom = (second_y * width) as usize;
    Cell {
        top_left: top + first_x as usize,
        top_right: top + second_x as usize,
        bottom_left: bottom + first_x as usize,
        bottom_right: bottom + second_x as usize,
        fraction_x: x - first_x as f32,
        fraction_y: y - first_y as f32,
    }
}

/// `lambda_3`'s captures.
struct PixelBake<'a> {
    width: i32,
    height: i32,
    ids: &'a [u32],
    triangles: &'a [[u32; 3]],
    tangents: &'a [[f32; 3]],
    normals: &'a [[f32; 3]],
    vertices: &'a [[f32; 3]],
    heights: &'a [f32],
    /// The heightmap's units in pixels: the width times the ratio.
    scale: f32,
    global: &'a [[f32; 3]],
}

/// A ray from a pixel along the mesh's normal (or against it, whichever
/// way the heightmap lies).
struct Ray {
    origin: [f32; 2],
    direction: [f32; 3],
    /// The mesh's height at the pixel in pixels.
    start_height: f32,
    /// −1 marching down to the heightmap, +1 marching up to it.
    sign: f32,
}

impl PixelBake<'_> {
    /// How far the ray at `distance` still is from the heightmap, as the
    /// marching loop measures it (positive until it crosses).
    fn march_gap(&self, ray: &Ray, distance: f32) -> f32 {
        let x = distance.mul_add(ray.direction[0], ray.origin[0]);
        let y = distance.mul_add(ray.direction[1], ray.origin[1]);
        let cell = cell(x, y, self.width, self.height);
        let inverse_x = 1.0 - cell.fraction_x;
        let top = cell.fraction_x.mul_add(self.heights[cell.top_right], inverse_x * self.heights[cell.top_left]);
        let bottom_part = inverse_x.mul_add(self.heights[cell.bottom_left], -top);
        let negative_rise = (-cell.fraction_x).mul_add(self.heights[cell.bottom_right], -bottom_part);
        let negative_surface = cell.fraction_y.mul_add(negative_rise, -top);
        let lifted = distance.mul_add(ray.direction[2], ray.start_height);
        self.scale.mul_add(negative_surface, lifted) * ray.sign
    }

    /// The same gap as the bisection measures it (its bilinear sample
    /// fuses the other products).
    fn bisection_gap(&self, ray: &Ray, distance: f32) -> f32 {
        let x = distance.mul_add(ray.direction[0], ray.origin[0]);
        let y = distance.mul_add(ray.direction[1], ray.origin[1]);
        let cell = cell(x, y, self.width, self.height);
        let inverse_x = 1.0 - cell.fraction_x;
        let top = inverse_x.mul_add(self.heights[cell.top_left], cell.fraction_x * self.heights[cell.top_right]);
        let negative_bottom = (-inverse_x).mul_add(self.heights[cell.bottom_left], -(cell.fraction_x * self.heights[cell.bottom_right]));
        let fall = top + negative_bottom;
        let negative_surface = cell.fraction_y.mul_add(fall, -top);
        let lifted = distance.mul_add(ray.direction[2], ray.start_height);
        self.scale.mul_add(negative_surface, lifted) * ray.sign
    }

    /// The distance at which the ray meets the heightmap: whole pixels
    /// until it crosses, then 32 halvings — or the distance to the map's
    /// edge if it gets there first.
    fn hit_distance(&self, ray: &Ray) -> f32 {
        let pixel_x = ray.origin[0] + 0.5;
        let pixel_y = ray.origin[1] + 0.5;
        let reciprocal_x = 1.0 / ray.direction[0];
        let to_left = (-pixel_x) * reciprocal_x;
        let to_right = ((self.width - 1) as f32 - pixel_x) * reciprocal_x;
        let reciprocal_y = 1.0 / ray.direction[1];
        let to_top = (-pixel_y) * reciprocal_y;
        let to_bottom = ((self.height - 1) as f32 - pixel_y) * reciprocal_y;
        let reach = minss(maxss(to_left, to_right), maxss(to_top, to_bottom));

        let mut distance = 0.0f32;
        loop {
            if !(distance < reach) {
                return reach;
            }
            let previous = distance;
            distance += 1.0;
            let gap = self.march_gap(ray, distance);
            if gap > 0.0 {
                continue;
            }
            let mut low = previous;
            let mut high = distance;
            for _ in 0..BISECTION_STEPS {
                let middle = (low + high) * 0.5;
                if self.bisection_gap(ray, middle) > 0.0 {
                    low = middle;
                } else {
                    high = middle;
                }
            }
            return (low + high) * 0.5;
        }
    }

    /// `lambda_3` at one pixel: the warp and the normal map's colour.
    fn pixel(&self, x: i32, y: i32) -> ([f32; 2], [f32; 3]) {
        let index = (self.width * y + x) as usize;
        let triangle = self.triangles[self.ids[index] as usize];
        let [first, second, third] = triangle.map(|corner| corner as usize);
        let origin = [x as f32, y as f32];

        // The heightmap at the pixel.
        let here = cell(origin[0], origin[1], self.width, self.height);
        let inverse_x = 1.0 - here.fraction_x;
        let top = inverse_x.mul_add(self.heights[here.top_left], here.fraction_x * self.heights[here.top_right]);
        let bottom = inverse_x.mul_add(self.heights[here.bottom_left], here.fraction_x * self.heights[here.bottom_right]);
        let surface = here.fraction_y.mul_add(bottom - top, top);

        // The mesh at the pixel's centre.
        let a = self.vertices[first];
        let b = self.vertices[second];
        let c = self.vertices[third];
        let grown = GrownTriangle::new(a, b, c);
        let [u, v, w] = grown.weights_outlined(x, y);
        let interpolate = |values: &[[f32; 3]], axis: usize| u.mul_add(values[first][axis], w.mul_add(values[third][axis], v * values[second][axis]));
        let normal = [interpolate(self.normals, 0), interpolate(self.normals, 1), interpolate(self.normals, 2)];
        let start_height = self.scale * interpolate(self.vertices, 2);
        let scaled_surface = self.scale * surface;

        let (direction, sign) = if start_height > scaled_surface { ([-normal[0], -normal[1], -normal[2]], 1.0) } else { (normal, -1.0) };
        let ray = Ray { origin, direction, start_height, sign };
        let distance = self.hit_distance(&ray);

        let tangent = [interpolate(self.tangents, 0), interpolate(self.tangents, 1), interpolate(self.tangents, 2)];
        let hit_x = distance.mul_add(direction[0], origin[0] + 0.5);
        let hit_y = distance.mul_add(direction[1], origin[1] + 0.5);
        let warp_x = minss(maxss(hit_x, 0.0), self.width as f32 + -1.0);
        let warp_y = minss(maxss(hit_y, 0.0), self.height as f32 + -1.0);

        // The heightmap's normal where the ray landed.
        let landing = cell(warp_x + -0.5, warp_y + -0.5, self.width, self.height);
        let inverse_x = 1.0 - landing.fraction_x;
        let inverse_y = 1.0 - landing.fraction_y;
        let bottom_left_weight = inverse_x * landing.fraction_y;
        let bottom_right_weight = landing.fraction_y * landing.fraction_x;
        let top_left_weight = inverse_y * inverse_x;
        let top_right_weight = inverse_y * landing.fraction_x;
        let mut found = [0.0f32; 3];
        for (axis, value) in found.iter_mut().enumerate() {
            let sum = bottom_right_weight * self.global[landing.bottom_right][axis];
            let sum = bottom_left_weight.mul_add(self.global[landing.bottom_left][axis], sum);
            let sum = top_right_weight.mul_add(self.global[landing.top_right][axis], sum);
            *value = top_left_weight.mul_add(self.global[landing.top_left][axis], sum);
        }
        let found = scaled(found, inverse_length(squared_length(found)));

        // In the mesh's frame: tangent, bitangent (normal × tangent) and
        // normal, none of them renormalised after interpolation.
        let [nx, ny, nz] = normal;
        let [tx, ty, tz] = tangent;
        let bitangent_x = ny.mul_add(tz, -(nz * ty));
        let negated_bitangent_y = (-nz).mul_add(tx, nx * tz);
        let negated_bitangent_z = (-nx).mul_add(ty, ny * tx);
        let along_normal = found[2].mul_add(nz, found[1].mul_add(ny, found[0] * nx));
        let along_bitangent = (-found[2]).mul_add(negated_bitangent_z, (-found[1]).mul_add(negated_bitangent_y, found[0] * bitangent_x));
        let along_tangent = found[2].mul_add(tz, found[1].mul_add(ty, found[0] * tx));
        let inverse = inverse_length(squared_length([along_tangent, along_bitangent, along_normal]));

        let red = inverse.mul_add(along_tangent * 0.5, 0.5);
        let green = inverse.mul_add(along_bitangent * NEWTON_HALF, 0.5);
        let blue = inverse.mul_add(along_normal * 0.5, 0.5);
        ([warp_x, warp_y], [red, green, blue])
    }
}

/// The warp for one channel: each pixel sampled where its
/// ray landed (`Channel.InterpolateBilinear`). `warp` is the bake's, at
/// `warp_width` pixels per row.
pub fn warp_channel(channel: &[f32], width: usize, height: usize, warp: &[[f32; 2]], warp_width: usize) -> Vec<f32> {
    (0..width * height)
        .into_par_iter()
        .map(|index| {
            let position = warp[(index / width) * warp_width + index % width];
            interpolate_bilinear(channel, width, height, position)
        })
        .collect()
}

/// `Channel.InterpolateBilinear(Vector2)`: managed float arithmetic (no
/// fused operations) about the point half a pixel back, the fractions taken
/// before the corners are clamped into the map.
fn interpolate_bilinear(channel: &[f32], width: usize, height: usize, [x, y]: [f32; 2]) -> f32 {
    let x = x - 0.5;
    let y = y - 0.5;
    let column = cvttss2si(x);
    let row = cvttss2si(y);
    let fraction_x = x - column as f32;
    let fraction_y = y - row as f32;
    let last_column = width as i32 - 1;
    let last_row = height as i32 - 1;
    let column = column.max(0).min(last_column);
    let row = row.max(0).min(last_row);
    let next_column = (column + 1).min(last_column);
    let next_row = (row + 1).min(last_row);
    let stride = width as i32;
    let at = |cell_column: i32, cell_row: i32| channel[(cell_row * stride + cell_column) as usize];

    let top = at(column, row) * (1.0 - fraction_x) + at(next_column, row) * fraction_x;
    let bottom = at(column, next_row) * (1.0 - fraction_x) + at(next_column, next_row) * fraction_x;
    top * (1.0 - fraction_y) + bottom * fraction_y
}
