//! The mesh rasteriser: `MeshRasterizer` (SYCL kernels on AdaptiveCpp's
//! OpenMP device) and the meshes it renders, built by the native
//! `mesh_port`.
//!
//! `render_mesh` fits a mesh into the target, rotates, scales and offsets
//! it, then rasterises every triangle as a height surface, keeping the
//! highest one per pixel over a cleared (zero) heightmap.

use std::sync::atomic::{AtomicU32, Ordering};

use rayon::prelude::*;

use crate::{delaunator, gpu, sycl};
use crate::x86::{cvttss2si, maxss, minss, rsqrt};

/// A triangle mesh as `interopmesh::Mesh` holds it (normals only when
/// computed).
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub vertices: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
    pub normals: Vec<[f32; 3]>,
}

/// `mesh_port::heightmap_to_mesh` without normals: one vertex per pixel at
/// (column / (width − 1), row / (height − 1), height × terrain height /
/// terrain width), two triangles per quad.
pub fn heightmap_to_mesh(heights: &[f32], width: usize, height: usize, terrain_width: f32, terrain_height: f32) -> Mesh {
    let ratio = terrain_height / terrain_width;
    let column_span = (width as u32).wrapping_sub(1) as f32;
    let row_span = (height as u32).wrapping_sub(1) as f32;

    let mut vertices = Vec::with_capacity(width * height);
    for row in 0..height {
        let y = row as f32 / row_span;
        for column in 0..width {
            let x = column as f32 / column_span;
            let z = heights[row * width + column] * ratio;
            vertices.push([x, y, z]);
        }
    }

    let quad_count = width.saturating_sub(1) * height.saturating_sub(1);
    let mut triangles = Vec::with_capacity(quad_count * 2);
    for row in 0..height.saturating_sub(1) {
        for column in 0..width.saturating_sub(1) {
            let index = (row * width + column) as u32;
            let below = index + width as u32;
            triangles.push([index, below, index + 1]);
            triangles.push([index + 1, below, below + 1]);
        }
    }
    Mesh { vertices, triangles, normals: Vec::new() }
}

/// `mesh_port::heightmap_to_mesh` with its normals flag: the grid mesh,
/// then vertex normals smoothed `smoothing` times.
pub fn heightmap_to_mesh_with_normals(heights: &[f32], width: usize, height: usize, terrain_width: f32, terrain_height: f32, smoothing: u32) -> Mesh {
    let mut mesh = heightmap_to_mesh(heights, width, height, terrain_width, terrain_height);
    compute_normals(&mut mesh);
    smooth_normals(&mut mesh, smoothing);
    mesh
}

/// `shm::heightmap_to_mesh_adaptive_subdivision` (with its normals flag):
/// the grid mesh (both coordinates over `width − 1`) with every edge
/// longer than `2 / width` split evenly, each cell triangulated along its
/// split edges, then vertex normals smoothed `smoothing` times.
pub fn adaptive_heightmap_to_mesh(heights: &[f32], width: usize, height: usize, terrain_width: f32, terrain_height: f32, smoothing: u32) -> Mesh {
    let columns = width as u32;
    let rows = height as u32;
    let ratio = terrain_height / terrain_width;
    let span = columns.wrapping_sub(1) as f32;
    let mut vertices: Vec<[f32; 3]> = (0..rows)
        .into_par_iter()
        .flat_map_iter(|row| {
            (0..columns).map(move |column| {
                let index = columns.wrapping_mul(row).wrapping_add(column) as usize;
                [column as i32 as f32 / span, row as i32 as f32 / span, heights[index] * ratio]
            })
        })
        .collect();

    // Horizontal edges row by row, then vertical edges column by column;
    // the inserted vertices follow the grid in that order.
    let threshold = 2.0 / columns as f32;
    let horizontal_count = rows as usize * width.saturating_sub(1);
    let vertical_count = width * height.saturating_sub(1);
    let mut edges = Vec::with_capacity(horizontal_count + vertical_count);
    let mut inserted = Vec::new();
    for row in 0..rows {
        for column in 0..columns.saturating_sub(1) {
            let start = columns * row + column;
            edges.push(split_edge(&vertices, &mut inserted, start, start + 1, threshold));
        }
    }
    for column in 0..columns {
        for row in 0..rows.saturating_sub(1) {
            let start = columns * row + column;
            edges.push(split_edge(&vertices, &mut inserted, start, start + columns, threshold));
        }
    }
    vertices.extend_from_slice(&inserted);

    let cell_rows: Vec<Vec<[u32; 3]>> = (0..rows.saturating_sub(1))
        .into_par_iter()
        .map(|row| {
            let mut triangles = Vec::with_capacity(width * 5);
            let row = row as usize;
            let row_width = width - 1;
            let row_height = height - 1;
            for column in 0..row_width {
                let top = &edges[row_width * row + column];
                let left = &edges[row_height * column + horizontal_count + row];
                let bottom = &edges[column + row_width * (row + 1)];
                let right = &edges[(column + 1) * row_height + horizontal_count + row];
                triangulate_cell(&vertices, [top, left, bottom, right], &mut triangles);
            }
            triangles
        })
        .collect();
    let triangles = cell_rows.concat();

    let mut mesh = Mesh { vertices, triangles, normals: Vec::new() };
    compute_normals(&mut mesh);
    smooth_normals(&mut mesh, smoothing);
    mesh
}

/// A grid edge of the adaptive mesh: its two grid vertices and the
/// `count` vertices inserted along it, numbered from `first`.
#[derive(Clone, Copy, Debug, Default)]
struct SplitEdge {
    start: u32,
    end: u32,
    first: u32,
    count: u32,
}

impl SplitEdge {
    /// The vertex `position` steps along the edge, walking from `start`
    /// (`forward`) or from `end`; `u32::MAX` past the far end.
    fn vertex(&self, forward: bool, position: u32) -> u32 {
        let along = if forward { position } else { self.count.wrapping_sub(position).wrapping_add(1) };
        if along == 0 {
            return self.start;
        }
        if self.count < along {
            if along == self.count.wrapping_add(1) {
                return self.end;
            }
            return u32::MAX;
        }
        self.first.wrapping_sub(1).wrapping_add(along)
    }
}

/// The distance between two points, (Δx² + Δy²) + Δz² under the root.
fn distance(from: [f32; 3], to: [f32; 3]) -> f32 {
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    let dz = to[2] - from[2];
    ((dx * dx + dy * dy) + dz * dz).sqrt()
}

/// An edge longer than `threshold` gets
/// `trunc(length / threshold)` vertices at even steps along it.
fn split_edge(vertices: &[[f32; 3]], inserted: &mut Vec<[f32; 3]>, start: u32, end: u32, threshold: f32) -> SplitEdge {
    let mut edge = SplitEdge { start, end, first: 0, count: 0 };
    let from = vertices[start as usize];
    let to = vertices[end as usize];
    let delta = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let length = distance(from, to);
    if !(threshold < length) {
        return edge;
    }
    edge.count = cvttss2si(length / threshold) as u32;
    edge.first = (vertices.len() + inserted.len()) as u32;
    let mut step = 1u32;
    while step <= edge.count {
        let fraction = step as f32 / edge.count.wrapping_add(1) as f32;
        let point = [fraction * delta[0] + from[0], fraction * delta[1] + from[1], fraction * delta[2] + from[2]];
        inserted.push(point);
        step += 1;
    }
    edge
}

/// The radius-ratio quality
/// `(s − a)(s − b)(s − c) / abc` of a triangle (0 when degenerate).
fn triangle_quality(first: [f32; 3], second: [f32; 3], third: [f32; 3]) -> f32 {
    let first_side = distance(first, second);
    let second_side = distance(second, third);
    let third_side = distance(third, first);
    let product = (second_side * first_side) * third_side;
    if product == 0.0 {
        return 0.0;
    }
    let half_perimeter = ((second_side + first_side) + third_side) * 0.5;
    let area_terms = ((half_perimeter - second_side) * (half_perimeter - first_side)) * (half_perimeter - third_side);
    area_terms / product
}

/// A cell bounded by its top, left, bottom and right
/// edges (walked as one loop) is fanned out from a corner, with two
/// cursors stepping away from it in opposite directions; each step adds
/// the better of the two candidate triangles until the cursors meet. A
/// candidate ending on the opposite corner scores −1, and the cell is left
/// unfinished when neither scores above zero.
fn triangulate_cell(vertices: &[[f32; 3]], edges: [&SplitEdge; 4], triangles: &mut Vec<[u32; 3]>) {
    let mut forward = [false; 4];
    for index in 0..4 {
        let edge = edges[index];
        let next = edges[(index + 1) & 3];
        if edge.end == next.start || edge.end == next.end {
            forward[index] = true;
        } else if edge.start == next.start || edge.start == next.end {
            forward[index] = false;
        }
    }
    let corners: [u32; 4] = std::array::from_fn(|index| if forward[index] { edges[index].start } else { edges[index].end });
    let point = |index: u32| vertices[index as usize];
    let first_diagonal = distance(point(corners[0]), point(corners[2]));
    let second_diagonal = distance(point(corners[1]), point(corners[3]));

    let side = usize::from(first_diagonal < second_diagonal);
    let start_corner = edges[side].vertex(forward[side], 0);
    let mut back_edge = (side + 3) & 3;
    let mut back_position = edges[back_edge].count;
    let mut ahead_edge = side;
    let mut ahead_position = 1;
    if edges[side].count == 0 {
        ahead_edge = side + 1;
        ahead_position = 0;
    }
    if back_position == u32::MAX {
        back_edge = (back_edge + 3) & 3;
        back_position = edges[back_edge].count;
    }
    let first_back = edges[back_edge].vertex(forward[back_edge], back_position);
    let first_ahead = edges[ahead_edge].vertex(forward[ahead_edge], ahead_position);
    triangles.push([start_corner, first_ahead, first_back]);

    let opposite = corners[side + 2];
    loop {
        let ahead = edges[ahead_edge].vertex(forward[ahead_edge], ahead_position);
        let back = edges[back_edge].vertex(forward[back_edge], back_position);

        let stepped = ahead_position.wrapping_add(1);
        let ahead_count = edges[ahead_edge].count;
        let next_ahead_edge = if ahead_count < stepped { (ahead_edge + 1) & 3 } else { ahead_edge };
        let next_ahead_position = if stepped <= ahead_count { stepped } else { 0 };
        let (next_back_edge, next_back_position) = if back_position == 0 {
            let previous = (back_edge + 3) & 3;
            (previous, edges[previous].count)
        } else {
            (back_edge, back_position - 1)
        };
        let next_ahead = edges[next_ahead_edge].vertex(forward[next_ahead_edge], next_ahead_position);
        let next_back = edges[next_back_edge].vertex(forward[next_back_edge], next_back_position);
        if next_ahead == next_back {
            triangles.push([ahead, next_ahead, back]);
            return;
        }

        let ahead_quality = if next_ahead != opposite { triangle_quality(point(ahead), point(next_ahead), point(back)) } else { -1.0 };
        let back_quality = if next_back != opposite { triangle_quality(point(ahead), point(back), point(next_back)) } else { -1.0 };
        if ahead_quality >= back_quality && ahead_quality > 0.0 {
            triangles.push([ahead, next_ahead, back]);
            ahead_edge = next_ahead_edge;
            ahead_position = next_ahead_position;
        } else if back_quality > 0.0 {
            triangles.push([next_back, back, ahead]);
            back_edge = next_back_edge;
            back_position = next_back_position;
        } else {
            return;
        }
    }
}

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let length_squared = (vector[0] * vector[0] + vector[1] * vector[1]) + vector[2] * vector[2];
    let inverse = 1.0 / length_squared.sqrt();
    [inverse * vector[0], inverse * vector[1], inverse * vector[2]]
}

/// Each face normal weighted by the angle at each corner
/// (at the first corner the exterior angle is taken), divided by the
/// summed angles and normalised.
pub fn compute_normals(mesh: &mut Mesh) {
    let count = mesh.vertices.len();
    let mut sums = vec![[0.0f32; 3]; count];
    let mut weights = vec![0.0f32; count];
    for triangle in &mesh.triangles {
        let [a, b, c] = triangle.map(|index| index as usize);
        let p0 = mesh.vertices[a];
        let p1 = mesh.vertices[b];
        let p2 = mesh.vertices[c];
        let e = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let d = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        let cross_z = d[0] * e[1] - d[1] * e[0];
        let cross_y = d[2] * e[0] - d[0] * e[2];
        let cross_x = d[1] * e[2] - d[2] * e[1];

        let first = normalize(e);
        let second = normalize([p2[0] - p1[0], p2[1] - p1[1], p2[2] - p1[2]]);
        let third = normalize([p0[0] - p2[0], p0[1] - p2[1], p0[2] - p2[2]]);
        let face = normalize([cross_x, cross_y, cross_z]);

        let angle_a = crate::maths::acos((third[0] * first[0] + third[1] * first[1]) + third[2] * first[2]);
        let angle_b = crate::maths::acos(((0.0 - first[0]) * second[0] + (0.0 - first[1]) * second[1]) + (0.0 - first[2]) * second[2]);
        let angle_c = crate::maths::acos(((0.0 - second[0]) * third[0] + (0.0 - second[1]) * third[1]) + (0.0 - second[2]) * third[2]);
        for (corner, angle) in [(a, angle_a), (b, angle_b), (c, angle_c)] {
            for axis in 0..3 {
                sums[corner][axis] += angle * face[axis];
            }
        }
        weights[a] += angle_a;
        weights[b] += angle_b;
        weights[c] += angle_c;
    }
    mesh.normals = sums
        .iter()
        .zip(&weights)
        .map(|(sum, &weight)| {
            let averaged = [sum[0] / weight, sum[1] / weight, sum[2] / weight];
            normalize(averaged)
        })
        .collect();
}

/// `smoothen_normals_gather`: each pass replaces every normal with the
/// normalised sum of its triangle neighbours' normals (in triangle order,
/// shared edges counted once per triangle) plus half its own.
pub fn smooth_normals(mesh: &mut Mesh, passes: u32) {
    let count = mesh.vertices.len();
    let mut neighbours: Vec<Vec<u32>> = vec![Vec::new(); count];
    for triangle in &mesh.triangles {
        let [a, b, c] = *triangle;
        neighbours[a as usize].push(b);
        neighbours[a as usize].push(c);
        neighbours[b as usize].push(a);
        neighbours[b as usize].push(c);
        neighbours[c as usize].push(a);
        neighbours[c as usize].push(b);
    }
    for _ in 0..passes {
        let normals = &mesh.normals;
        let smoothed: Vec<[f32; 3]> = (0..count)
            .into_par_iter()
            .map(|vertex| {
                let mut sum = [0.0f32; 3];
                for &neighbour in &neighbours[vertex] {
                    let normal = normals[neighbour as usize];
                    for axis in 0..3 {
                        sum[axis] += normal[axis];
                    }
                }
                let own = normals[vertex];
                let blended = [0.5 * own[0] + sum[0], 0.5 * own[1] + sum[1], 0.5 * own[2] + sum[2]];
                let length_squared = (blended[1] * blended[1] + blended[0] * blended[0]) + blended[2] * blended[2];
                let inverse = 1.0 / length_squared.sqrt();
                [inverse * blended[0], inverse * blended[1], inverse * blended[2]]
            })
            .collect();
        mesh.normals = smoothed;
    }
}

/// `MeshRasterizer.Parameters` as `RenderMesh` reads them.
#[derive(Clone, Debug)]
pub struct RenderSettings {
    pub width: i32,
    pub height: i32,
    /// Terrain height / terrain width.
    pub height_ratio: f32,
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
    pub scale: f32,
    pub offset: [f32; 3],
    pub drop_to_floor: bool,
    pub antialiasing: bool,
}

/// `MeshRasterizer_RenderMesh`: the rendered heightmap (`width × height`).
/// Antialiasing renders at twice the resolution and box-downsamples. Runs
/// on the GPU when one is active (see [`sycl`]).
pub fn render_mesh(mesh: &Mesh, settings: &RenderSettings) -> Vec<f32> {
    let gpu = |device: &nerthus_compute::GpuDevice| gpu::mesh_rasterizer::render_mesh(device, mesh, settings);
    let cpu = || render_mesh_cpu(mesh, settings);
    sycl::run("MeshRasterizer_RenderMesh", settings.width.max(0) as usize, sycl::PREFERRED, gpu, cpu)
}

/// [`render_mesh`] on the CPU.
pub fn render_mesh_cpu(mesh: &Mesh, settings: &RenderSettings) -> Vec<f32> {
    let mut width = settings.width;
    let mut height = settings.height;
    if settings.antialiasing {
        width *= 2;
        height *= 2;
    }

    let mut vertices = mesh.vertices.clone();
    process_vertices(&mut vertices, width, height, settings);
    let rendered = rasterize(&vertices, &mesh.triangles, width, height);
    if settings.antialiasing {
        return downsample_box(&rendered, settings.width as usize, settings.height as usize);
    }
    rendered
}

/// UCRT's `sinf` and `cosf`, taken as the double functions rounded.
fn ucrt_sinf(x: f32) -> f32 {
    (x as f64).sin() as f32
}

fn ucrt_cosf(x: f32) -> f32 {
    (x as f64).cos() as f32
}

struct Bounds {
    min: [f32; 3],
    max: [f32; 3],
}

/// `MeshRasterizer::get_bounds`: a min/max reduction per axis, all zero for
/// an empty mesh.
fn bounds(vertices: &[[f32; 3]]) -> Bounds {
    if vertices.is_empty() {
        return Bounds { min: [0.0; 3], max: [0.0; 3] };
    }
    let mut result = Bounds { min: [f32::MAX; 3], max: [-f32::MAX; 3] };
    for vertex in vertices {
        for (axis, &value) in vertex.iter().enumerate() {
            if value < result.min[axis] {
                result.min[axis] = value;
            }
            if value > result.max[axis] {
                result.max[axis] = value;
            }
        }
    }
    result
}

/// The rotation matrix `process_vertices` builds (row-major, 4×4): a
/// quaternion from half-angle sines and cosines of the three angles
/// (degrees), normalised with RSQRTSS and one Newton step, then expanded.
fn euler_matrix(pitch: f32, yaw: f32, roll: f32) -> [f32; 16] {
    const HALF_DEGREE_IN_RADIANS: f32 = f32::from_bits(0x3C0E_FA35);

    let half_pitch = HALF_DEGREE_IN_RADIANS * pitch;
    let sin_pitch = ucrt_sinf(half_pitch);
    let cos_pitch = ucrt_cosf(half_pitch);
    let half_yaw = yaw * HALF_DEGREE_IN_RADIANS;
    let sin_yaw = ucrt_sinf(half_yaw);
    let cos_yaw = ucrt_cosf(half_yaw);
    let half_roll = roll * HALF_DEGREE_IN_RADIANS;
    let sin_roll = ucrt_sinf(half_roll);
    let cos_roll = ucrt_cosf(half_roll);

    let cos_yaw_roll = cos_yaw * cos_roll;
    let sin_roll_negated_yaw = sin_roll * -sin_yaw;
    let sin_yaw_cos_roll = sin_yaw * cos_roll;
    let sin_roll_cos_yaw = sin_roll * cos_yaw;
    let w = cos_yaw_roll.mul_add(cos_pitch, -(sin_roll_negated_yaw * sin_pitch));
    let x = sin_pitch.mul_add(cos_yaw_roll, sin_roll_negated_yaw * cos_pitch);
    let y = sin_roll_cos_yaw.mul_add(sin_pitch, sin_yaw_cos_roll * cos_pitch);
    let z = sin_roll_cos_yaw.mul_add(cos_pitch, -(sin_yaw_cos_roll * sin_pitch));

    let length_squared = w.mul_add(w, x.mul_add(x, y.mul_add(y, z * z)));
    let (mut qw, mut qx, mut qy, mut qz) = (1.0f32, 0.0f32, 0.0f32, 0.0f32);
    // UCOMISS + JZ: zero and NaN both keep the identity.
    let degenerate = length_squared == 0.0 || length_squared.is_nan();
    if !degenerate {
        let estimate = rsqrt(length_squared);
        let refinement = estimate.mul_add(length_squared * estimate, -3.0);
        let inverse_length = (estimate * -0.5) * refinement;
        qz = inverse_length * z;
        qy = inverse_length * y;
        qx = inverse_length * x;
        qw = inverse_length * w;
    }

    let xx = qx * qx;
    let yy = qy * qy;
    let zz = qz * qz;
    let zx = qz * qx;
    let yx = qy * qx;
    let zy = qz * qy;
    let wx = qw * qx;
    let wy = qw * qy;
    let wz = qw * qz;

    let mut matrix = [0.0f32; 16];
    matrix[0] = (-(zz + yy)).mul_add(2.0, 1.0);
    let m1 = yx + wz;
    matrix[1] = m1 + m1;
    let m2 = zx - wy;
    matrix[2] = m2 + m2;
    let m4 = yx - wz;
    matrix[4] = m4 + m4;
    matrix[5] = (-(zz + xx)).mul_add(2.0, 1.0);
    let m6 = zy + wx;
    matrix[6] = m6 + m6;
    let m8 = zx + wy;
    matrix[8] = m8 + m8;
    let m9 = zy - wx;
    matrix[9] = m9 + m9;
    matrix[10] = (-(yy + xx)).mul_add(2.0, 1.0);
    matrix[15] = 1.0;
    matrix
}

/// `MeshRasterizer::process_vertices`: fits the bounds into the target (the
/// smallest of the three axis fits), rotates about the bounds' centre,
/// scales and offsets, with heights in units of the target width × the
/// height ratio. Dropping to the floor then shifts every coordinate — x and
/// y too — by the lowest height less the z offset.
pub(crate) fn process_vertices(vertices: &mut [[f32; 3]], width: i32, height: i32, settings: &RenderSettings) {
    let width_f = width as f32;
    let height_f = height as f32;
    let depth_f = width_f * settings.height_ratio;
    let extent = bounds(vertices);

    let fits = [
        width_f / (extent.max[0] - extent.min[0]),
        height_f / (extent.max[1] - extent.min[1]),
        depth_f / (extent.max[2] - extent.min[2]),
    ];
    let mut fit = fits[0];
    for &candidate in &fits[1..] {
        fit = minss(candidate, fit);
    }

    let matrix = euler_matrix(settings.pitch, -settings.yaw, settings.roll);
    let centre = [
        0.5 * (extent.min[0] + extent.max[0]),
        0.5 * (extent.min[1] + extent.max[1]),
        0.5 * (extent.min[2] + extent.max[2]),
    ];
    let scale = fit * settings.scale;
    let translation = [
        width_f * (0.5 + settings.offset[0]),
        height_f * (0.5 - settings.offset[1]),
        depth_f * (0.5 + settings.offset[2]),
    ];
    let inverse_depth = 1.0 / depth_f;

    for vertex in vertices.iter_mut() {
        let dx = vertex[0] - centre[0];
        let dy = vertex[1] - centre[1];
        let dz = vertex[2] - centre[2];
        let rotated_x = dz.mul_add(matrix[2], dy.mul_add(matrix[1], matrix[0].mul_add(dx, matrix[3])));
        let rotated_y = dz.mul_add(matrix[6], dy.mul_add(matrix[5], matrix[4].mul_add(dx, matrix[7])));
        let rotated_z = dz.mul_add(matrix[10], dy.mul_add(matrix[9], matrix[8].mul_add(dx, matrix[11])));
        vertex[0] = rotated_x.mul_add(scale, translation[0]);
        vertex[1] = rotated_y.mul_add(scale, translation[1]);
        vertex[2] = rotated_z.mul_add(scale, translation[2]) * inverse_depth;
    }

    if !settings.drop_to_floor {
        return;
    }
    let moved = bounds(vertices);
    let lift = moved.min[2] - settings.offset[2];
    for vertex in vertices.iter_mut() {
        vertex[0] -= lift;
        vertex[1] -= lift;
        vertex[2] -= lift;
    }
}

/// Triangles whose bounding box reaches this many pixels are skipped.
const TRIANGLE_PIXEL_LIMIT: u32 = 0x1000_0000;

/// `render`'s first kernel: the pixels of a triangle's bounding box, rounded
/// outwards (0 at or above the limit).
pub(crate) fn candidate_count(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> u32 {
    let count = box_pixel_count(a, b, c);
    if count >= TRIANGLE_PIXEL_LIMIT { 0 } else { count }
}

/// The textured renderer's count kernel: the same box, without the limit.
pub(crate) fn box_pixel_count(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> u32 {
    let low_y = cvttss2si(minss(a[1], minss(b[1], c[1])).floor());
    let high_y = cvttss2si(maxss(a[1], maxss(b[1], c[1])).ceil());
    let low_x = cvttss2si(minss(a[0], minss(b[0], c[0])).floor());
    let high_x = cvttss2si(maxss(a[0], maxss(b[0], c[0])).ceil());
    let rows = high_y.wrapping_sub(low_y).wrapping_add(1);
    let columns = high_x.wrapping_sub(low_x).wrapping_add(1);
    rows.wrapping_mul(columns).wrapping_abs() as u32
}

const ONE_THIRD: f32 = f32::from_bits(0x3EAA_AAAB);
const GROWTH: f32 = f32::from_bits(0x3F80_A3D7);
pub(crate) const MISS: f32 = -f32::MAX;

/// The part of the barycentric test both kernel paths share: the triangle
/// grown by 0.5% about its centroid, the point's offset from the grown first
/// corner, and the height there (or `MISS` outside).
pub(crate) struct GrownTriangle {
    centre: [f32; 2],
    corner: [f32; 2],
    first_edge: [f32; 2],
    second_edge: [f32; 2],
}

impl GrownTriangle {
    pub(crate) fn new(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> Self {
        let centre_x = ((b[0] + a[0]) + c[0]) * ONE_THIRD;
        let centre_y = ((b[1] + a[1]) + c[1]) * ONE_THIRD;
        let corner_x = GROWTH * (a[0] - centre_x);
        let corner_y = GROWTH * (a[1] - centre_y);
        GrownTriangle {
            centre: [centre_x, centre_y],
            corner: [corner_x, corner_y],
            first_edge: [GROWTH.mul_add(b[0] - centre_x, -corner_x), GROWTH.mul_add(b[1] - centre_y, -corner_y)],
            second_edge: [GROWTH.mul_add(c[0] - centre_x, -corner_x), GROWTH.mul_add(c[1] - centre_y, -corner_y)],
        }
    }

    /// The barycentric weights (u, v, w) of the corners a, b, c at a point
    /// given its dot products with the two edges.
    fn weights(&self, first_dot: f32, second_dot: f32) -> [f32; 3] {
        let [e1x, e1y] = self.first_edge;
        let [e2x, e2y] = self.second_edge;
        let second_second = e2y.mul_add(e2y, e2x * e2x);
        let first_second = e1y.mul_add(e2y, e2x * e1x);
        let first_first = e1y.mul_add(e1y, e1x * e1x);
        let denominator = second_second.mul_add(first_first, -(first_second * first_second));
        let v_numerator = first_dot.mul_add(second_second, -(second_dot * first_second));
        let inverse = 1.0 / denominator;
        let v = v_numerator * inverse;
        let w_numerator = second_dot.mul_add(first_first, -(first_dot * first_second));
        let w = w_numerator * inverse;
        let u = 1.0 - (w + v);
        [u, v, w]
    }

    /// The height at a point given its dot products with the two edges.
    fn height(&self, first_dot: f32, second_dot: f32, a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
        let [u, v, w] = self.weights(first_dot, second_dot);
        // UCOMISS + JC: negative weights and NaN both miss.
        let inside = u >= 0.0 && v >= 0.0 && w >= 0.0;
        if !inside {
            return MISS;
        }
        u.mul_add(a[2], w.mul_add(c[2], v * b[2]))
    }

    /// The pixel centre's dot products with the two edges, as the
    /// out-of-line code computes them.
    fn dots_outlined(&self, x: i32, y: i32) -> (f32, f32) {
        let point_x = x as f32 + 0.5;
        let point_y = y as f32 + 0.5;
        let dx = point_x - (self.centre[0] + self.corner[0]);
        let dy = point_y - (self.centre[1] + self.corner[1]);
        let second_dot = dy.mul_add(self.second_edge[1], dx * self.second_edge[0]);
        let first_dot = self.first_edge[1].mul_add(dy, dx * self.first_edge[0]);
        (first_dot, second_dot)
    }

    /// The out-of-line test (`render`'s plain region, three or fewer
    /// candidates in all): the point is the pixel centre.
    fn sample_outlined(&self, x: i32, y: i32, a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
        let (first_dot, second_dot) = self.dots_outlined(x, y);
        self.height(first_dot, second_dot, a, b, c)
    }

    /// The pixel centre's barycentric weights, inside the triangle or not
    /// (the tangent-space bake's per-pixel kernel).
    pub(crate) fn weights_outlined(&self, x: i32, y: i32) -> [f32; 3] {
        let (first_dot, second_dot) = self.dots_outlined(x, y);
        self.weights(first_dot, second_dot)
    }

    /// The test as inlined in the chunked region (more than three
    /// candidates): the half-pixel offset is reassociated and the dot
    /// products fuse the other product.
    pub(crate) fn sample_inlined(&self, x: i32, y: i32, a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
        let dx = x as f32 + (0.5 - (self.centre[0] + self.corner[0]));
        let dy = (0.5 - (self.centre[1] + self.corner[1])) + y as f32;
        let second_dot = dx.mul_add(self.second_edge[0], dy * self.second_edge[1]);
        let first_dot = self.first_edge[0].mul_add(dx, dy * self.first_edge[1]);
        self.height(first_dot, second_dot, a, b, c)
    }
}

/// `MeshRasterizer::render<1,0,1,0>`: counts each triangle's candidate
/// pixels, scans the counts (an `i32` running sum), then tests every
/// candidate against its triangle and keeps the highest surface over zero.
/// Candidates are numbered row by row across a box whose origin and width
/// truncate the corners (the counts round them outwards instead).
fn rasterize(vertices: &[[f32; 3]], triangles: &[[u32; 3]], width: i32, height: i32) -> Vec<f32> {
    let pixel_count = width.max(0) as usize * height.max(0) as usize;
    let heightmap: Vec<AtomicU32> = (0..pixel_count).map(|_| AtomicU32::new(0)).collect();
    if triangles.is_empty() {
        return vec![0.0; pixel_count];
    }

    let corners = |triangle: &[u32; 3]| {
        let a = vertices[triangle[0] as usize];
        let b = vertices[triangle[1] as usize];
        let c = vertices[triangle[2] as usize];
        (a, b, c)
    };
    let counts: Vec<u32> = triangles
        .par_iter()
        .map(|triangle| {
            let (a, b, c) = corners(triangle);
            candidate_count(a, b, c)
        })
        .collect();
    let mut running = 0i32;
    for &count in &counts {
        running = running.wrapping_add(count as i32);
    }
    let total = running as i64 as u64;
    let inlined = total > 3;

    triangles.par_iter().zip(counts.par_iter()).for_each(|(triangle, &count)| {
        let (a, b, c) = corners(triangle);
        let low_x = cvttss2si(minss(minss(a[0], b[0]), c[0]));
        let high_x = cvttss2si(maxss(maxss(a[0], b[0]), c[0]));
        let low_y = cvttss2si(minss(minss(a[1], b[1]), c[1]));
        let box_width = high_x.wrapping_sub(low_x).wrapping_add(1) as u32;
        if box_width == 0 {
            return;
        }
        let grown = GrownTriangle::new(a, b, c);
        for local in 0..count {
            let x = ((local % box_width) as i32).wrapping_add(low_x);
            if x < 0 {
                continue;
            }
            let y = ((local / box_width) as i32).wrapping_add(low_y);
            if y < 0 || x >= width || y >= height {
                continue;
            }
            let surface = if inlined { grown.sample_inlined(x, y, a, b, c) } else { grown.sample_outlined(x, y, a, b, c) };
            if !(surface > MISS) {
                continue;
            }
            let index = width.wrapping_mul(y).wrapping_add(x) as u32 as usize;
            let cell = &heightmap[index];
            let mut current = cell.load(Ordering::Relaxed);
            while surface > f32::from_bits(current) {
                match cell.compare_exchange_weak(current, surface.to_bits(), Ordering::Relaxed, Ordering::Relaxed) {
                    Ok(_) => break,
                    Err(actual) => current = actual,
                }
            }
        }
    });

    heightmap.into_iter().map(|cell| f32::from_bits(cell.into_inner())).collect()
}

/// A texture the point-cloud renderer samples and recolours, through
/// `Map.R/G/B`: the channel index clamps to the last channel, so a
/// grey map's single plane serves as all three (and is written three times,
/// blue last).
pub struct Texture<'a> {
    pub(crate) channels: Vec<&'a mut [f32]>,
}

impl<'a> Texture<'a> {
    pub fn new(channels: Vec<&'a mut [f32]>) -> Self {
        assert!(!channels.is_empty(), "a texture needs a channel");
        Texture { channels }
    }

    fn plane(&self, component: usize) -> usize {
        component.min(self.channels.len() - 1)
    }

    fn sample(&self, pixel: usize) -> [f32; 3] {
        [0, 1, 2].map(|component| self.channels[self.plane(component)][pixel])
    }

    fn paint(&mut self, pixel: usize, colour: [f32; 3]) {
        for (component, value) in colour.into_iter().enumerate() {
            let plane = self.plane(component);
            self.channels[plane][pixel] = value;
        }
    }
}

/// The thirteen points `RenderPointCloud`'s colour kernel samples in a
/// triangle (the centroid at 0.333, then weightings 0.6/0.2, 0.4/0.2,
/// 0.7/0.15 and 0.5/0.25), with the kernel's FMA and sum order.
fn colour_sample_points(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [[f32; 2]; 13] {
    let third = f32::from_bits(0x3EAA_7EFA);
    let [ax, ay, _] = a;
    let [bx, by, _] = b;
    let [cx, cy, _] = c;
    let (ax2, ay2, bx2, by2, cx2, cy2) = (ax * 0.2, ay * 0.2, bx * 0.2, by * 0.2, cx * 0.2, cy * 0.2);
    let (ax4, ay4, bx4, by4, cx4, cy4) = (ax * 0.4, ay * 0.4, bx * 0.4, by * 0.4, cx * 0.4, cy * 0.4);
    let (ax15, ay15, bx15, by15, cx15, cy15) = (ax * 0.15, ay * 0.15, bx * 0.15, by * 0.15, cx * 0.15, cy * 0.15);
    let (axq, ayq, bxq, byq, cxq, cyq) = (ax * 0.25, ay * 0.25, bx * 0.25, by * 0.25, cx * 0.25, cy * 0.25);
    [
        [((ax + bx) + cx) * third, ((by + ay) + cy) * third],
        [ax.mul_add(0.6, bx2) + cx2, ay.mul_add(0.6, by2) + cy2],
        [bx.mul_add(0.6, ax2) + cx2, by.mul_add(0.6, ay2) + cy2],
        [cx.mul_add(0.6, ax2 + bx2), cy.mul_add(0.6, by2 + ay2)],
        [(bx4 + ax4) + cx2, (by4 + ay4) + cy2],
        [(ax4 + bx2) + cx4, cy4 + (ay4 + by2)],
        [(bx4 + ax2) + cx4, cy4 + (by4 + ay2)],
        [cx15 + ax.mul_add(0.7, bx15), ay.mul_add(0.7, by15) + cy15],
        [cx15 + bx.mul_add(0.7, ax15), by.mul_add(0.7, ay15) + cy15],
        [cx.mul_add(0.7, bx15 + ax15), cy.mul_add(0.7, by15 + ay15)],
        [ax.mul_add(0.5, bxq) + cxq, ay.mul_add(0.5, byq) + cyq],
        [bx.mul_add(0.5, axq) + cxq, by.mul_add(0.5, ayq) + cyq],
        [cx.mul_add(0.5, bxq + axq), cy.mul_add(0.5, byq + ayq)],
    ]
}

/// `RenderPointCloud`'s colour kernel: of the texture colours at the
/// thirteen sample points, the one closest to the others (largest sum of
/// `1 / (|Δ|² + 0.001)`; the first on ties, black if none is positive).
fn triangle_colour(texture: &Texture, a: [f32; 3], b: [f32; 3], c: [f32; 3], width: i32, height: i32) -> [f32; 3] {
    let points = colour_sample_points(a, b, c);
    let mut colours = [[0.0f32; 3]; 13];
    for (colour, point) in colours.iter_mut().zip(points) {
        let x = cvttss2si(point[0]).max(0).min(width - 1);
        let y = cvttss2si(point[1]).max(0).min(height - 1);
        let pixel = y.wrapping_mul(width).wrapping_add(x) as usize;
        *colour = texture.sample(pixel);
    }
    let inverse_distance = |from: [f32; 3], to: [f32; 3]| {
        let red = from[0] - to[0];
        let green = from[1] - to[1];
        let blue = from[2] - to[2];
        let squared = blue.mul_add(blue, green.mul_add(green, red.mul_add(red, 0.001)));
        1.0 / squared
    };
    let mut best = 0.0f32;
    let mut chosen = [0.0f32; 3];
    for candidate in colours {
        let terms: Vec<f32> = colours.iter().map(|&other| inverse_distance(candidate, other)).collect();
        let first = (terms[2] + terms[3]) + (terms[0] + terms[1]);
        let second = first + (terms[6] + (terms[5] + terms[4]));
        let third = second + (terms[10] + (terms[9] + (terms[8] + terms[7])));
        let total = (terms[11] + terms[12]) + third;
        if total > best {
            best = total;
            chosen = candidate;
        }
    }
    chosen
}

/// `MeshRasterizer_RenderPointCloud` without a texture: points within a
/// pixel of the border move ten pixels outside it (so the triangulation
/// covers the map), the points are Delaunay-triangulated, and each
/// triangle's surface is written over a cleared map, later triangles
/// overwriting earlier ones where the 0.5% growth makes them overlap (the
/// kernels store without atomics, so threads race there). The second map is
/// a per-triangle hash in [0, 1).
pub fn render_point_cloud(points: &[[f32; 3]], width: i32, height: i32) -> (Vec<f32>, Vec<f32>) {
    render_point_cloud_with(points, width, height, None)
}

/// [`render_point_cloud`] with a texture: each triangle takes one colour
/// from it (see [`triangle_colour`]) and paints its pixels, and its pixel
/// counts are not limited.
pub fn render_point_cloud_textured(points: &[[f32; 3]], width: i32, height: i32, texture: Texture) -> (Vec<f32>, Vec<f32>) {
    render_point_cloud_with(points, width, height, Some(texture))
}

/// The point cloud's mesh: points within a pixel of the border moved ten
/// pixels outside it, and their Delaunay triangles (none when the points
/// do not span a triangle).
pub(crate) fn cloud_mesh(points: &[[f32; 3]], width: i32, height: i32) -> Option<Mesh> {
    let width_f = width as f32;
    let height_f = height as f32;
    let mut vertices = Vec::with_capacity(points.len());
    let mut coords = Vec::with_capacity(points.len() * 2);
    for point in points {
        let x = push_outward(point[0], width_f);
        let y = push_outward(point[1], height_f);
        coords.push(x as f64);
        coords.push(y as f64);
        vertices.push([x, y, point[2]]);
    }
    let flat = delaunator::triangulate(&coords)?;
    let triangles: Vec<[u32; 3]> = flat.as_chunks::<3>().0.iter().map(|triangle| [triangle[0] as u32, triangle[1] as u32, triangle[2] as u32]).collect();
    Some(Mesh { vertices, triangles, normals: Vec::new() })
}

/// A point cloud rendered on the GPU (see [`gpu::mesh_rasterizer`]).
pub struct CloudRender {
    pub heights: Vec<f32>,
    pub hashes: Vec<f32>,
    /// The texture's planes after painting, when it has one.
    pub painted: Option<Vec<Vec<f32>>>,
}

/// Renders the point cloud on the GPU when one is active (see [`sycl`]),
/// painting the texture with the result; otherwise on the CPU.
fn render_point_cloud_with(points: &[[f32; 3]], width: i32, height: i32, mut texture: Option<Texture>) -> (Vec<f32>, Vec<f32>) {
    let planes: Option<Vec<&[f32]>> = texture.as_ref().map(|texture| texture.channels.iter().map(|channel| &**channel).collect());
    let gpu = |device: &nerthus_compute::GpuDevice| gpu::mesh_rasterizer::render_point_cloud(device, points, width, height, planes.as_deref()).map(Some);
    let rendered = sycl::run("MeshRasterizer_RenderPointCloud", width.max(0) as usize, sycl::PREFERRED, gpu, || None);
    drop(planes);
    let Some(rendered) = rendered else {
        return render_point_cloud_cpu(points, width, height, texture);
    };
    if let (Some(texture), Some(painted)) = (texture.as_mut(), rendered.painted) {
        for (channel, plane) in texture.channels.iter_mut().zip(painted) {
            channel.copy_from_slice(&plane);
        }
    }
    (rendered.heights, rendered.hashes)
}

/// [`render_point_cloud`] or [`render_point_cloud_textured`] on the CPU.
pub fn render_point_cloud_cpu(points: &[[f32; 3]], width: i32, height: i32, mut texture: Option<Texture>) -> (Vec<f32>, Vec<f32>) {
    let pixel_count = width.max(0) as usize * height.max(0) as usize;
    let mut heights = vec![0.0f32; pixel_count];
    let mut hashes = vec![0.0f32; pixel_count];
    let Some(Mesh { vertices, triangles, .. }) = cloud_mesh(points, width, height) else {
        return (heights, hashes);
    };

    let colours: Vec<[f32; 3]> = match &texture {
        Some(texture) => triangles
            .iter()
            .map(|triangle| {
                let a = vertices[triangle[0] as usize];
                let b = vertices[triangle[1] as usize];
                let c = vertices[triangle[2] as usize];
                triangle_colour(texture, a, b, c, width, height)
            })
            .collect(),
        None => Vec::new(),
    };

    for (index, triangle) in triangles.iter().enumerate() {
        let a = vertices[triangle[0] as usize];
        let b = vertices[triangle[1] as usize];
        let c = vertices[triangle[2] as usize];
        let count = if texture.is_some() { box_pixel_count(a, b, c) } else { candidate_count(a, b, c) };
        if count == 0 {
            continue;
        }
        let low_x = cvttss2si(minss(minss(a[0], b[0]), c[0]));
        let high_x = cvttss2si(maxss(maxss(a[0], b[0]), c[0]));
        let low_y = cvttss2si(minss(minss(a[1], b[1]), c[1]));
        let box_width = high_x.wrapping_sub(low_x).wrapping_add(1) as u32;
        if box_width == 0 {
            continue;
        }
        let grown = GrownTriangle::new(a, b, c);
        let hash = triangle_hash(index as u32);
        for local in 0..count {
            let x = ((local % box_width) as i32).wrapping_add(low_x);
            if x < 0 {
                continue;
            }
            let y = ((local / box_width) as i32).wrapping_add(low_y);
            if y < 0 || x >= width || y >= height {
                continue;
            }
            let surface = grown.sample_outlined(x, y, a, b, c);
            if !(surface > MISS) {
                continue;
            }
            let pixel = width.wrapping_mul(y).wrapping_add(x) as u32 as usize;
            heights[pixel] = surface;
            hashes[pixel] = hash;
            if let Some(texture) = texture.as_mut() {
                texture.paint(pixel, colours[index]);
            }
        }
    }
    (heights, hashes)
}

/// Points within a pixel of the low edge move to −10, those within a
/// pixel of the high edge to `extent + 9`.
fn push_outward(value: f32, extent: f32) -> f32 {
    let lowered = if value < 1.0 { -10.0 } else { value };
    if lowered < extent - 1.0 { lowered } else { extent + 9.0 }
}

/// The raster kernel's triangle hash: xorshift32 of the index (a fixed
/// seed for triangle 0), times 0x2545F381, minus one, over 2³².
fn triangle_hash(index: u32) -> f32 {
    let mut state = if index != 0 { index } else { 0xA235_1342 };
    state ^= state << 13;
    state ^= state >> 17;
    state ^= state << 5;
    let mixed = state.wrapping_mul(0x2545_F381).wrapping_sub(1);
    mixed as f32 * f32::from_bits(0x2F80_0000)
}

/// `MeshRasterizer_RenderMeshWithTriplanarDisplacement`'s parameters.
#[derive(Clone, Debug)]
pub struct TriplanarSettings {
    pub width: i32,
    pub height: i32,
    pub height_ratio: f32,
    pub strength: f32,
    pub offset: f32,
    pub horizontal: bool,
    pub antialiasing: bool,
}

/// What `process_vertices_with_displacement` samples and scales by.
struct Displacement<'a> {
    width_f: f32,
    height_f: f32,
    sample_scale: f32,
    map_width: i32,
    map_height: i32,
    maps: [&'a [f32]; 3],
    strength: f32,
    offset: f32,
    inverse_ratio: f32,
}

/// A map sample's integer corners: the truncated coordinate clamped into
/// the map, and the next one (from −1 for negative coordinates).
fn sample_corners(coordinate: f32, last: i32) -> (i32, i32) {
    let whole = cvttss2si(coordinate);
    let low = whole.max(0).min(last);
    let start = if whole < 0 { -1 } else { whole };
    let high = start.wrapping_add(1).min(last);
    (low, high)
}

impl Displacement<'_> {
    fn at(&self, map: usize, column: i32, row: i32) -> f32 {
        self.maps[map][row.wrapping_mul(self.map_width).wrapping_add(column) as usize]
    }

    /// The three bilinear samples (XY at (x, y), XZ at (x, z), YZ at
    /// (y, z)), each with the kernel's FMA pattern.
    fn samples(&self, vertex: [f32; 3]) -> (f32, f32, f32) {
        let [x, y, z] = vertex;
        let x_scale = self.sample_scale * self.width_f;
        let u = x.mul_add(x_scale, -0.5);
        let v = (y * self.height_f).mul_add(self.sample_scale, -0.5);
        let w = z.mul_add(x_scale, -0.5);
        let v_yz = y.mul_add(x_scale, -0.5);
        let (x0, x1) = sample_corners(u, self.map_width - 1);
        let (y0, y1) = sample_corners(v, self.map_height - 1);
        let (z0, z1) = sample_corners(w, self.map_height - 1);
        let (ya, yb) = sample_corners(v_yz, self.map_width - 1);
        let fz = w - z0 as f32;

        let fy_yz = v_yz - ya as f32;
        let rest_yz = 1.0 - fy_yz;
        let top_yz = fy_yz.mul_add(self.at(2, yb, z0), rest_yz * self.at(2, ya, z0));
        let partial_yz = (-rest_yz).mul_add(self.at(2, ya, z1), top_yz);
        let difference_yz = fy_yz.mul_add(self.at(2, yb, z1), -partial_yz);
        let yz = fz.mul_add(difference_yz, top_yz);

        let fx = u - x0 as f32;
        let rest_x = 1.0 - fx;
        let top_xz = fx.mul_add(self.at(1, x1, z0), rest_x * self.at(1, x0, z0));
        let partial_xz = (-rest_x).mul_add(self.at(1, x0, z1), top_xz);
        let difference_xz = fx.mul_add(self.at(1, x1, z1), -partial_xz);
        let xz = fz.mul_add(difference_xz, top_xz);

        let top_xy = rest_x.mul_add(self.at(0, x0, y0), fx * self.at(0, x1, y0));
        let bottom_xy = rest_x.mul_add(self.at(0, x0, y1), fx * self.at(0, x1, y1));
        let fy = v - y0 as f32;
        let xy = fy.mul_add(bottom_xy - top_xy, top_xy);
        (xy, xz, yz)
    }

    /// `process_vertices_with_displacement<0>`: weights n⁴/Σn⁴, the whole
    /// normal moves.
    fn displace(&self, vertex: [f32; 3], normal: [f32; 3]) -> [f32; 3] {
        let (xy, xz, yz) = self.samples(vertex);
        let [nx, ny, nz] = normal;
        let (sx, sy, sz) = (nx * nx, ny * ny, nz * nz);
        let (qx, qy, qz) = (sx * sx, sy * sy, sz * sz);
        let sum = (qy + qx) + qz;
        let inverse = 1.0 / sum;
        let positive = 0.0 < sum;
        let weight_x = if positive { qx * inverse } else { 0.0 };
        let (weight_y, weight_z) = if positive { (qy * inverse, qz * inverse) } else { (0.0, 0.0) };
        let distance = weight_x.mul_add(yz, xz.mul_add(weight_y, xy.mul_add(weight_z, self.offset)));
        let shift = distance * self.strength;
        let [x, y, z] = vertex;
        [nx.mul_add(shift, x) * self.width_f, ny.mul_add(shift, y) * self.height_f, nz.mul_add(shift, z) * self.inverse_ratio]
    }

    /// `process_vertices_with_displacement<1>` (Horizontal): weights
    /// |n|/Σ|n|, only the horizontal unit normal moves (RSQRTSS + Newton).
    fn displace_horizontally(&self, vertex: [f32; 3], normal: [f32; 3]) -> [f32; 3] {
        let (xy, xz, yz) = self.samples(vertex);
        let [nx, ny, nz] = normal;
        let horizontal_squared = ny.mul_add(ny, nx * nx);
        let estimate = rsqrt(horizontal_squared);
        let refinement = estimate.mul_add(horizontal_squared * estimate, -3.0);
        let inverse_horizontal = (-0.5 * estimate) * refinement;
        let (ax, ay, az) = (nx.abs(), ny.abs(), nz.abs());
        let sum = (ay + ax) + az;
        let inverse = 1.0 / sum;
        let positive = 0.0 < sum;
        let weight_z = if positive { az * inverse } else { 0.0 };
        let weight_y = if positive { ay * inverse } else { 0.0 };
        let weight_x = if positive { ax * inverse } else { 0.0 };
        let distance = weight_x.mul_add(yz, weight_y.mul_add(xz, weight_z.mul_add(xy, self.offset)));
        let shift = distance * self.strength;
        let flat = 0.0 < horizontal_squared;
        let hx = if flat { inverse_horizontal * nx } else { nx };
        let hy = if flat { ny * inverse_horizontal } else { ny };
        let [x, y, z] = vertex;
        [hx.mul_add(shift, x) * self.width_f, hy.mul_add(shift, y) * self.height_f, z * self.inverse_ratio]
    }
}

/// `MeshRasterizer_RenderMeshWithTriplanarDisplacement`: the mesh (in its
/// own unit space) displaced by the three maps along its normals, scaled to
/// the target and drawn with `render3_cpu`. Antialiasing renders at twice
/// the resolution and box-downsamples; the maps stay at `width × height`.
/// Runs on a discrete GPU when one is active (see [`sycl`]).
pub fn render_mesh_triplanar(mesh: &Mesh, settings: &TriplanarSettings, maps: [&[f32]; 3]) -> Vec<f32> {
    let gpu = |device: &nerthus_compute::GpuDevice| gpu::mesh_rasterizer::render_mesh_triplanar(device, mesh, settings, maps);
    let cpu = || render_mesh_triplanar_cpu(mesh, settings, maps);
    sycl::run("MeshRasterizer_RenderMeshWithTriplanarDisplacement", settings.width.max(0) as usize, sycl::DISCRETE_OR_CPU, gpu, cpu)
}

/// The render size of a triplanar render: twice the target with
/// antialiasing.
pub(crate) fn triplanar_size(settings: &TriplanarSettings) -> (i32, i32) {
    if settings.antialiasing {
        return (settings.width * 2, settings.height * 2);
    }
    (settings.width, settings.height)
}

/// `process_vertices_with_displacement`: the mesh's vertices displaced by
/// the three maps along their normals and scaled to the render size.
pub(crate) fn displaced_vertices(mesh: &Mesh, settings: &TriplanarSettings, maps: [&[f32]; 3]) -> Vec<[f32; 3]> {
    let (width, height) = triplanar_size(settings);
    let displacement = Displacement {
        width_f: width as f32,
        height_f: height as f32,
        sample_scale: settings.width as f32 / width as f32,
        map_width: settings.width,
        map_height: settings.height,
        maps,
        strength: settings.strength,
        offset: settings.offset,
        inverse_ratio: 1.0 / settings.height_ratio,
    };
    mesh.vertices
        .par_iter()
        .zip(mesh.normals.par_iter())
        .map(|(&vertex, &normal)| {
            if settings.horizontal { displacement.displace_horizontally(vertex, normal) } else { displacement.displace(vertex, normal) }
        })
        .collect()
}

/// [`render_mesh_triplanar`] on the CPU.
pub fn render_mesh_triplanar_cpu(mesh: &Mesh, settings: &TriplanarSettings, maps: [&[f32]; 3]) -> Vec<f32> {
    let (width, height) = triplanar_size(settings);
    let vertices = displaced_vertices(mesh, settings, maps);
    let rendered = render3(&vertices, &mesh.triangles, width, height);
    if settings.antialiasing {
        return downsample_box(&rendered, settings.width as usize, settings.height as usize);
    }
    rendered
}

/// `render3_cpu<1,0,1,0>`: each triangle facing the right way (by the sign
/// of its 2D cross product) is tested at every pixel of its bounding box
/// (rounded outwards, clamped to the target) and kept where highest.
fn render3(vertices: &[[f32; 3]], triangles: &[[u32; 3]], width: i32, height: i32) -> Vec<f32> {
    let pixel_count = width.max(0) as usize * height.max(0) as usize;
    let heightmap: Vec<AtomicU32> = (0..pixel_count).map(|_| AtomicU32::new(0)).collect();
    triangles.par_iter().for_each(|triangle| {
        let a = vertices[triangle[0] as usize];
        let b = vertices[triangle[1] as usize];
        let c = vertices[triangle[2] as usize];
        let first = (b[0] - a[0]) * (c[1] - a[1]);
        let second = (c[0] - a[0]) * (b[1] - a[1]);
        if first > second {
            return;
        }
        let low_y = cvttss2si(minss(a[1], minss(c[1], b[1])).floor());
        let high_y = cvttss2si(maxss(a[1], maxss(c[1], b[1])).ceil());
        let high_x = cvttss2si(maxss(a[0], maxss(b[0], c[0])).ceil());
        let row_start = low_y.max(0);
        let row_end = if high_y < height - 1 { high_y } else { height - 1 };
        let column_end = if high_x < width - 1 { high_x } else { width - 1 };
        if row_end < row_start {
            return;
        }
        let low_x = cvttss2si(minss(a[0], minss(b[0], c[0])).floor());
        let column_start = low_x.max(0);
        if column_end < column_start {
            return;
        }
        let grown = GrownTriangle::new(a, b, c);
        for y in row_start..=row_end {
            for x in column_start..=column_end {
                let surface = grown.sample_inlined(x, y, a, b, c);
                if !(surface > MISS) {
                    continue;
                }
                let cell = &heightmap[width.wrapping_mul(y).wrapping_add(x) as u32 as usize];
                let mut current = cell.load(Ordering::Relaxed);
                while surface > f32::from_bits(current) {
                    match cell.compare_exchange_weak(current, surface.to_bits(), Ordering::Relaxed, Ordering::Relaxed) {
                        Ok(_) => break,
                        Err(actual) => current = actual,
                    }
                }
            }
        }
    });
    heightmap.into_iter().map(|cell| f32::from_bits(cell.into_inner())).collect()
}

/// `resampling::downsample_box`: the 2×2 mean. AdaptiveCpp's region runs
/// each row in chunks of 1024 columns; AVX lanes add the two row pairs
/// first, the scalar code (the last 1–8 columns of a chunk, every column of
/// a chunk under 9, and whole rows under 4 columns) adds in order.
pub fn downsample_box(fine: &[f32], width: usize, height: usize) -> Vec<f32> {
    let fine_width = width * 2;
    let mut coarse = vec![0.0f32; width * height];
    coarse.par_chunks_mut(width.max(1)).enumerate().for_each(|(row, line)| {
        if row >= height {
            return;
        }
        let top = &fine[(2 * row) * fine_width..];
        let bottom = &fine[(2 * row + 1) * fine_width..];
        let mut start = 0;
        while start < width {
            let end = (start + 1024).min(width);
            let length = end - start;
            let mut vector_end = start;
            if width > 3 && length >= 9 {
                let tail = if length % 8 == 0 { 8 } else { length % 8 };
                vector_end = end - tail;
            }
            for column in start..end {
                let t0 = top[2 * column];
                let t1 = top[2 * column + 1];
                let b0 = bottom[2 * column];
                let b1 = bottom[2 * column + 1];
                let sum = if column < vector_end { (t0 + t1) + (b0 + b1) } else { ((t1 + t0) + b0) + b1 };
                line[column] = sum * 0.25;
            }
            start = end;
        }
    });
    coarse
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_rotation() {
        let matrix = euler_matrix(0.0, 0.0, 0.0);
        let expected = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        assert_eq!(matrix, expected);
    }

    #[test]
    fn grid_mesh_layout() {
        let heights = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
        let mesh = heightmap_to_mesh(&heights, 3, 2, 8000.0, 4000.0);
        assert_eq!(mesh.vertices[4], [0.5, 1.0, 2.0]);
        assert_eq!(mesh.triangles, vec![[0, 3, 1], [1, 3, 4], [1, 4, 2], [2, 4, 5]]);
    }

    #[test]
    fn counts_round_outwards() {
        let count = candidate_count([0.5, 0.5, 0.0], [2.5, 0.5, 0.0], [0.5, 1.5, 0.0]);
        assert_eq!(count, 4 * 3);
    }
}
