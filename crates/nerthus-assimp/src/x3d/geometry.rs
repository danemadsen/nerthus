//! `X3DGeoHelper` and the shapes the X3D readers generate: arcs, boxes,
//! extrusions, and the meshes made from coordinate index lists, with the
//! colours, normals and texture coordinates added to them.

use crate::matrix::Matrix4;
use crate::msvc::{VectorCapacity, vector_growth};
use crate::scene::{ImportError, Mesh, PRIMITIVE_LINE, PRIMITIVE_POINT, PRIMITIVE_POLYGON, PRIMITIVE_TRIANGLE, check_allocation, fail};
use crate::vector::{Vector3, cross, dot, normalize, scale, sub};

/// `AI_MATH_TWO_PI_F` and `AI_MATH_HALF_PI_F`, from Assimp's literal.
#[allow(clippy::excessive_precision)]
pub(super) const TWO_PI: f32 = 3.141_592_653_8_f32 * 2.0;
#[allow(clippy::excessive_precision)]
pub(super) const HALF_PI: f32 = 3.141_592_653_8_f32 * 0.5;

/// `sizeof(aiFace)`, `sizeof(aiVector3D)`, `sizeof(aiColor4D)` and
/// `sizeof(int32_t)`.
const FACE_SIZE: u64 = 16;
const VECTOR_SIZE: u64 = 12;
const COLOR_SIZE: u64 = 16;
const INDEX_SIZE: u64 = 4;

pub(super) fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// `make_point2D`: a point on the circle in the XY plane.
fn point_2d(angle: f32, radius: f32) -> Vector3 {
    [radius * angle.cos(), radius * angle.sin(), 0.0]
}

/// `make_arc2D`: `segments` + 1 points from the start angle, over the
/// angle between the two (a full circle when that is zero or more than a
/// turn, closed by the first point again).
pub(super) fn arc_2d(start: f32, end: f32, radius: f32, segments: usize, points: &mut Vec<Vector3>) -> Result<(), ImportError> {
    if start < -TWO_PI || start > TWO_PI {
        return fail("GeometryHelper_Make_Arc2D.pStartAngle");
    }
    if end < -TWO_PI || end > TWO_PI {
        return fail("GeometryHelper_Make_Arc2D.pEndAngle");
    }
    if radius <= 0.0 {
        return fail("GeometryHelper_Make_Arc2D.pRadius");
    }
    let mut angle_full = (end - start).abs();
    if angle_full > TWO_PI || angle_full == 0.0 {
        angle_full = TWO_PI;
    }
    let angle_step = angle_full / segments as f32;
    for index in 0..=segments {
        let angle = start + index as f32 * angle_step;
        points.push(point_2d(angle, radius));
    }
    if angle_full == TWO_PI {
        points.push(points[0]);
    }
    Ok(())
}

/// `extend_point_to_line`: the points as segments, each inner point
/// ending one and starting the next.
pub(super) fn point_to_line(points: &[Vector3]) -> Result<Vec<Vector3>, ImportError> {
    if points.len() < 2 {
        return fail("GeometryHelper_Extend_PointToLine.pPoint.size() can not be less than 2.");
    }
    let last = points.len() - 1;
    let mut line = vec![points[0]];
    for &point in &points[1..last] {
        line.push(point);
        line.push(point);
    }
    line.push(points[last]);
    Ok(line)
}

/// `rect_parallel_epiped`: a box's six quads.
pub(super) fn box_quads(size: Vector3) -> Vec<Vector3> {
    let half_x = size[0] / 2.0;
    let half_y = size[1] / 2.0;
    let half_z = size[2] / 2.0;
    let (x1, x2) = (-half_x, half_x);
    let (y1, y2) = (-half_y, half_y);
    let (z1, z2) = (-half_z, half_z);
    let corners = [
        [x2, y1, z2],
        [x2, y2, z2],
        [x2, y2, z1],
        [x2, y1, z1],
        [x1, y1, z2],
        [x1, y2, z2],
        [x1, y2, z1],
        [x1, y1, z1],
    ];
    let quads = [[3, 2, 1, 0], [6, 7, 4, 5], [7, 3, 0, 4], [2, 6, 5, 1], [0, 1, 5, 4], [7, 6, 2, 3]];
    let mut vertices = Vec::with_capacity(24);
    for quad in quads {
        for corner in quad {
            vertices.push(corners[corner]);
        }
    }
    vertices
}

/// `GeometryHelper_Extrusion_CurveIsClosed` (dropping the tail and the
/// repeated last point): whether the curve comes back to its first point
/// at some point from the fourth on, the points after it repeating the
/// curve's start; the curve is cut there.
pub(super) fn close_curve<T: PartialEq + Copy>(curve: &mut Vec<T>) -> bool {
    let size = curve.len();
    if size < 4 {
        return false;
    }
    for start in 3..size {
        if curve[0] != curve[start] {
            continue;
        }
        let tail_repeats = (start + 1..size).zip(1..).all(|(later, earlier)| curve[earlier] == curve[later]);
        if tail_repeats {
            curve.truncate(start);
            return true;
        }
    }
    false
}

/// `GeometryHelper_Extrusion_GetNextY`.
fn extrusion_y(index: usize, spine: &[Vector3], closed: bool) -> Vector3 {
    let last = spine.len() - 1;
    let direction = if index == 0 || index == last {
        if closed {
            sub(spine[1], spine[last])
        } else if index == 0 {
            sub(spine[1], spine[0])
        } else {
            sub(spine[last], spine[last - 1])
        }
    } else {
        sub(spine[index + 1], spine[index - 1])
    };
    normalize(direction)
}

/// `aiVector3D::Equal` with `ai_epsilon`.
fn is_zero(vector: Vector3) -> bool {
    vector.iter().all(|component| component.abs() <= 1e-6)
}

/// `GeometryHelper_Extrusion_GetNextZ`: turned to agree with the previous
/// Z axis.
fn extrusion_z(index: usize, spine: &[Vector3], closed: bool, previous: Vector3) -> Vector3 {
    let last = spine.len() - 1;
    let mut direction;
    if spine.len() < 3 {
        direction = [0.0, 0.0, 1.0];
    } else if index == 0 {
        if closed {
            direction = cross(sub(spine[1], spine[0]), sub(spine[last], spine[0]));
        } else {
            direction = [0.0, 0.0, 0.0];
            let mut found = false;
            let mut next = 2;
            while next <= last && !found {
                direction = cross(sub(spine[next], spine[next - 1]), sub(spine[next - 2], spine[next - 1]));
                found = !is_zero(direction);
                next += 1;
            }
            if !found {
                direction = [0.0, 0.0, 1.0];
            }
        }
    } else if index == last {
        if closed {
            direction = cross(sub(spine[0], spine[index]), sub(spine[index - 1], spine[index]));
            if is_zero(direction) {
                direction = previous;
            }
        } else {
            direction = previous;
        }
    } else {
        direction = cross(sub(spine[index + 1], spine[index]), sub(spine[index - 1], spine[index]));
        if is_zero(direction) {
            direction = previous;
        }
    }
    if dot(direction, previous) < 0.0 {
        direction = [-direction[0], -direction[1], -direction[2]];
    }
    normalize(direction)
}

/// An extrusion's surface, as `readExtrusion` builds it from its fields
/// (already defaulted and padded).
pub(super) struct Extrusion {
    pub begin_cap: bool,
    pub end_cap: bool,
    pub ccw: bool,
    pub cross_section: Vec<[f32; 2]>,
    pub orientation: Vec<f32>,
    pub scale: Vec<[f32; 2]>,
    pub spine: Vec<Vector3>,
}

/// `MACRO_FACE_ADD_QUAD` and the face's delimiter.
fn add_quad(indices: &mut Vec<i32>, ccw: bool, corners: [usize; 4]) {
    let ordered = if ccw { corners } else { [corners[3], corners[2], corners[1], corners[0]] };
    for corner in ordered {
        indices.push(corner as i32);
    }
    indices.push(-1);
}

impl Extrusion {
    /// The coordinate indices and vertices: each spine point's cross
    /// section placed in its frame, caps, then quads between the sections.
    pub fn build(mut self) -> (Vec<i32>, Vec<Vector3>) {
        let cross_closed = close_curve(&mut self.cross_section);
        let spine_closed = close_curve(&mut self.spine);
        if spine_closed {
            self.begin_cap |= self.end_cap;
            self.end_cap = false;
        }

        let spine = &self.spine;
        let mut bases = Vec::with_capacity(spine.len());
        let mut axis_z = [0.0f32; 3];
        for index in 0..spine.len() {
            let axis_y = extrusion_y(index, spine, spine_closed);
            axis_z = extrusion_z(index, spine, spine_closed, axis_z);
            let axis_x = normalize(cross(axis_y, axis_z));
            let rotation = &self.orientation[index * 4..index * 4 + 4];
            let rotation_axis = [rotation[0], rotation[1], rotation[2]];
            let matrix = Matrix4::rotation(rotation[3], rotation_axis);
            let basis = [matrix.transform(axis_x), matrix.transform(axis_y), matrix.transform(axis_z)];
            bases.push(basis);
        }

        let mut point_sets = Vec::with_capacity(spine.len());
        for (spine_index, &spine_point) in spine.iter().enumerate() {
            let [scale_x, scale_y] = self.scale[spine_index];
            let scaling = Matrix4::scaling([scale_x, 0.0, scale_y]);
            let basis = &bases[spine_index];
            let mut section = Vec::with_capacity(self.cross_section.len());
            for &[cross_x, cross_y] in &self.cross_section {
                let scaled = scaling.transform([cross_x, 0.0, cross_y]);
                let along_x = scale(basis[0], scaled[0]);
                let along_y = scale(basis[1], scaled[1]);
                let along_z = scale(basis[2], scaled[2]);
                let summed = add(add(add(along_x, along_y), along_z), spine_point);
                section.push(summed);
            }
            point_sets.push(section);
        }

        let section_size = self.cross_section.len();
        let mut indices = Vec::new();
        if self.begin_cap {
            for corner in 0..section_size {
                indices.push(corner as i32);
            }
            indices.push(-1);
        }
        if self.end_cap {
            let first = (point_sets.len() - 1) * section_size;
            for corner in first..first + section_size {
                indices.push(corner as i32);
            }
            indices.push(-1);
        }
        let last_spine = spine.len() - 1;
        let last_cross = section_size - 1;
        for spine_index in 0..=last_spine {
            let right = if spine_index != last_spine {
                spine_index + 1
            } else if spine_closed {
                0
            } else {
                break;
            };
            for cross_index in 0..section_size {
                if cross_index != last_cross {
                    let corners = [
                        spine_index * section_size + cross_index,
                        right * section_size + cross_index,
                        right * section_size + cross_index + 1,
                        spine_index * section_size + cross_index + 1,
                    ];
                    add_quad(&mut indices, self.ccw, corners);
                } else if cross_closed {
                    let corners = [
                        spine_index * section_size + cross_index,
                        right * section_size + cross_index,
                        right * section_size,
                        spine_index * section_size,
                    ];
                    add_quad(&mut indices, self.ccw, corners);
                }
            }
        }

        let vertices = point_sets.into_iter().flatten().collect();
        (indices, vertices)
    }
}

fn add(a: Vector3, b: Vector3) -> Vector3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// The copy of a coordinate index list the face builders make, with a
/// final -1 added when it has none (growing the copy): its length.
fn closed_copy(indices: &[i32]) -> Result<(bool, usize), ImportError> {
    if indices.is_empty() {
        return undefined("the last of an empty coordinate index list");
    }
    let length = indices.len();
    check_allocation(length as u64, INDEX_SIZE)?;
    let closing = indices.last() != Some(&-1);
    if closing {
        check_allocation(vector_growth(length, length + 1) as u64, INDEX_SIZE)?;
    }
    Ok((closing, length + closing as usize))
}

/// `coordIdx_str2faces_arr`: faces split at -1 (a last one closed), their
/// primitive types; none when a face is empty. An empty list is read
/// before its start.
fn faces_from_indices(indices: &[i32]) -> Result<(Vec<Vec<u32>>, u32), ImportError> {
    let (closing, total) = closed_copy(indices)?;
    let mut face_capacity = VectorCapacity::new(FACE_SIZE);
    face_capacity.reserve(total / 3)?;
    let mut corner_capacity = VectorCapacity::new(INDEX_SIZE);
    corner_capacity.reserve(4)?;
    let mut faces = Vec::new();
    let mut primitive_types = 0;
    let mut corners: Vec<u32> = Vec::new();
    for &index in indices.iter().chain(closing.then_some(&-1)) {
        if index != -1 {
            corner_capacity.push_to(corners.len() + 1)?;
            corners.push(index as u32);
            continue;
        }
        primitive_types |= match corners.len() {
            0 => return Ok((Vec::new(), primitive_types)),
            1 => PRIMITIVE_POINT,
            2 => PRIMITIVE_LINE,
            3 => PRIMITIVE_TRIANGLE,
            _ => PRIMITIVE_POLYGON,
        };
        face_capacity.push_to(faces.len() + 1)?;
        faces.push(std::mem::take(&mut corners));
    }
    Ok((faces, primitive_types))
}

/// `coordIdx_str2lines_arr`: each polyline (split at -1) as segments;
/// none when a polyline has fewer than two points.
fn lines_from_indices(indices: &[i32]) -> Result<Vec<Vec<u32>>, ImportError> {
    let (closing, total) = closed_copy(indices)?;
    check_allocation((total / 2) as u64, FACE_SIZE)?;
    let mut lines = Vec::new();
    let all: Vec<i32> = indices.iter().copied().chain(closing.then_some(-1)).collect();
    let mut start = 0;
    for (position, &index) in all.iter().enumerate() {
        if index != -1 {
            continue;
        }
        let polyline = &all[start..position];
        if polyline.len() <= 1 {
            return Ok(Vec::new());
        }
        for pair in polyline.windows(2) {
            lines.push(vec![pair[0] as u32, pair[1] as u32]);
        }
        start = position + 1;
    }
    Ok(lines)
}

/// The mesh of `faces` over `vertices`.
fn mesh_of(faces: Vec<Vec<u32>>, vertices: &[Vector3], primitive_types: u32) -> Result<Mesh, ImportError> {
    check_allocation(faces.len() as u64, FACE_SIZE)?;
    check_allocation(vertices.len() as u64, VECTOR_SIZE)?;
    let mesh = Mesh {
        primitive_types,
        vertex_count: vertices.len(),
        vertices: Some(vertices.to_vec()),
        faces: Some(faces),
        ..Mesh::default()
    };
    Ok(mesh)
}

/// `make_mesh`: faces from the coordinate indices (unchecked against the
/// vertices).
pub(super) fn make_mesh(indices: &[i32], vertices: &[Vector3]) -> Result<Mesh, ImportError> {
    let (faces, primitive_types) = faces_from_indices(indices)?;
    if faces.is_empty() {
        return fail("Failed to create mesh, faces list is empty.");
    }
    mesh_of(faces, vertices, primitive_types)
}

/// `make_line_mesh`: line segments from the coordinate indices.
pub(super) fn make_line_mesh(indices: &[i32], vertices: &[Vector3]) -> Result<Mesh, ImportError> {
    let faces = lines_from_indices(indices)?;
    if faces.is_empty() {
        return fail("Failed to create mesh, faces list is empty.");
    }
    mesh_of(faces, vertices, PRIMITIVE_LINE)
}

fn faces(mesh: &Mesh) -> &[Vec<u32>] {
    mesh.faces.as_deref().unwrap_or_default()
}

/// `add_color(mesh, colors, perVertex)`: a colour per vertex, or each
/// face's colour on its corners (a corner past the vertices writes past
/// the colours).
pub(super) fn add_color(mesh: &mut Mesh, colors: &[[f32; 4]], per_vertex: bool) -> Result<(), ImportError> {
    let vertex_count = mesh.vertex_count;
    if per_vertex {
        if colors.len() < vertex_count {
            return fail(format!("MeshGeometry_AddColor1. Colors count({}) can not be less than Vertices count({vertex_count}).", colors.len()));
        }
        check_allocation(vertex_count as u64, COLOR_SIZE)?;
        mesh.colors[0] = Some(colors[..vertex_count].to_vec());
        return Ok(());
    }
    let face_count = mesh.face_count();
    if colors.len() < face_count {
        return fail(format!("MeshGeometry_AddColor1. Colors count({}) can not be less than Faces count({face_count}).", colors.len()));
    }
    check_allocation(vertex_count as u64, COLOR_SIZE)?;
    let mut values = vec![[0.0f32; 4]; vertex_count];
    for (face, &color) in faces(mesh).iter().zip(colors) {
        for &corner in face {
            let Some(slot) = values.get_mut(corner as usize) else {
                return undefined("a face colour written past the vertices");
            };
            *slot = color;
        }
    }
    mesh.colors[0] = Some(values);
    Ok(())
}

/// `add_color(mesh, coordIdx, colorIdx, colors, perVertex)`: the colours
/// as indexed (per vertex, through the coordinate indices), then added.
pub(super) fn add_indexed_color(mesh: &mut Mesh, coord_index: &[i32], color_index: &[i32], colors: &[[f32; 4]], per_vertex: bool) -> Result<(), ImportError> {
    if coord_index.is_empty() {
        return fail("MeshGeometry_AddColor2. pCoordIdx can not be empty.");
    }
    let vertex_count = mesh.vertex_count;
    let face_count = mesh.face_count();
    let targets;
    if per_vertex {
        if !color_index.is_empty() {
            if color_index.len() < coord_index.len() {
                return fail(format!(
                    "MeshGeometry_AddColor2. Colors indices count({}) can not be less than Coords indices count({}).",
                    color_index.len(),
                    coord_index.len()
                ));
            }
            let mut values = vec![[0.0f32; 4]; vertex_count];
            for (position, &color) in color_index.iter().enumerate() {
                if color == -1 {
                    continue;
                }
                let Some(&coordinate) = coord_index.get(position) else {
                    return undefined("a colour index read past the coordinate indices");
                };
                if coordinate as u32 > vertex_count as u32 {
                    return fail("MeshGeometry_AddColor2. Coordinate idx is out of range.");
                }
                if color as u32 > vertex_count as u32 {
                    return fail("MeshGeometry_AddColor2. Color idx is out of range.");
                }
                let Some(&value) = colors.get(color as u32 as usize) else {
                    return undefined("a colour read past the colours");
                };
                let Some(slot) = values.get_mut(coordinate as u32 as usize) else {
                    return undefined("a colour written past the vertices");
                };
                *slot = value;
            }
            targets = values;
        } else {
            if colors.len() < vertex_count {
                return fail(format!("MeshGeometry_AddColor2. Colors count({}) can not be less than Vertices count({vertex_count}).", colors.len()));
            }
            targets = colors[..vertex_count].to_vec();
        }
    } else if !color_index.is_empty() {
        if color_index.len() < face_count {
            return fail(format!(
                "MeshGeometry_AddColor2. Colors indices count({}) can not be less than Faces count({face_count}).",
                color_index.len()
            ));
        }
        let mut values = Vec::with_capacity(face_count);
        for &color in &color_index[..face_count] {
            if color as u32 > face_count as u32 {
                return fail("MeshGeometry_AddColor2. Face idx is out of range.");
            }
            let Some(&value) = colors.get(color as u32 as usize) else {
                return undefined("a colour read past the colours");
            };
            values.push(value);
        }
        targets = values;
    } else {
        if colors.len() < face_count {
            return fail(format!("MeshGeometry_AddColor2. Colors count({}) can not be less than Faces count({face_count}).", colors.len()));
        }
        targets = colors[..face_count].to_vec();
    }
    add_color(mesh, &targets, per_vertex)
}

/// `add_normal(mesh, coordIdx, normalIdx, normals, perVertex)`: normals per
/// vertex (as indexed, skipping -1, or in order), or per face (as indexed
/// or in order) on the face's corners.
pub(super) fn add_indexed_normal(mesh: &mut Mesh, coord_index: &[i32], normal_index: &[i32], normals: &[Vector3], per_vertex: bool) -> Result<(), ImportError> {
    let vertex_count = mesh.vertex_count;
    if per_vertex {
        if normal_index.is_empty() {
            if normals.len() != vertex_count {
                return fail("MeshGeometry_AddNormal. Normals and vertices count must be equal.");
            }
            check_allocation(vertex_count as u64, VECTOR_SIZE)?;
            mesh.normals = Some(normals.to_vec());
            return Ok(());
        }
        if normal_index.len() != coord_index.len() {
            return fail("Normals and Coords inidces count must be equal.");
        }
        let used: Vec<usize> = normal_index.iter().filter(|&&index| index != -1).map(|&index| index as isize as usize).collect();
        check_allocation(vertex_count as u64, VECTOR_SIZE)?;
        let mut values = vec![[0.0f32; 3]; vertex_count];
        for (slot, &index) in values.iter_mut().zip(&used) {
            let Some(&normal) = normals.get(index) else {
                return fail(format!("MeshGeometry_AddNormal. Normal index({index}) is out of range. Normals count: {}.", normals.len()));
            };
            *slot = normal;
        }
        mesh.normals = Some(values);
        return Ok(());
    }
    let face_count = mesh.face_count();
    let face_normals: Vec<usize> = if normal_index.is_empty() {
        (0..face_count).collect()
    } else {
        if face_count != normal_index.len() {
            return fail("Normals faces count must be equal to mesh faces count.");
        }
        normal_index.iter().map(|&index| index as isize as usize).collect()
    };
    check_allocation(vertex_count as u64, VECTOR_SIZE)?;
    let mut values = vec![[0.0f32; 3]; vertex_count];
    for (face, &normal_index) in faces(mesh).iter().zip(&face_normals) {
        let Some(&normal) = normals.get(normal_index) else {
            return undefined("a face normal read past the normals");
        };
        for &corner in face {
            let Some(slot) = values.get_mut(corner as usize) else {
                return undefined("a face normal written past the vertices");
            };
            *slot = normal;
        }
    }
    mesh.normals = Some(values);
    Ok(())
}

/// `add_normal(mesh, normals, perVertex)`.
pub(super) fn add_normal(mesh: &mut Mesh, normals: &[Vector3], per_vertex: bool) -> Result<(), ImportError> {
    let vertex_count = mesh.vertex_count;
    if per_vertex {
        if normals.len() != vertex_count {
            return fail("MeshGeometry_AddNormal. Normals and vertices count must be equal.");
        }
        check_allocation(vertex_count as u64, VECTOR_SIZE)?;
        mesh.normals = Some(normals.to_vec());
        return Ok(());
    }
    if normals.len() != mesh.face_count() {
        return fail("MeshGeometry_AddNormal. Normals and faces count must be equal.");
    }
    check_allocation(vertex_count as u64, VECTOR_SIZE)?;
    let mut values = vec![[0.0f32; 3]; vertex_count];
    for (face, &normal) in faces(mesh).iter().zip(normals) {
        for &corner in face {
            let Some(slot) = values.get_mut(corner as usize) else {
                return undefined("a face normal written past the vertices");
            };
            *slot = normal;
        }
    }
    mesh.normals = Some(values);
    Ok(())
}

/// `add_tex_coord(mesh, coordIdx, texCoordIdx, texCoords)`: texture
/// coordinates on the faces' corners, the faces from the texture
/// coordinate indices (whose faces must match the mesh's) or else the
/// coordinate indices.
pub(super) fn add_indexed_tex_coord(mesh: &mut Mesh, coord_index: &[i32], tex_coord_index: &[i32], coordinates: &[[f32; 2]]) -> Result<(), ImportError> {
    let values: Vec<Vector3> = coordinates.iter().map(|&[u, v]| [u, v, 0.0]).collect();
    let texture_faces = if !tex_coord_index.is_empty() {
        let (texture_faces, _) = faces_from_indices(tex_coord_index)?;
        if texture_faces.is_empty() {
            return fail("Failed to add texture coordinates to mesh, faces list is empty.");
        }
        if texture_faces.len() != mesh.face_count() {
            return fail("Texture coordinates faces count must be equal to mesh faces count.");
        }
        texture_faces
    } else {
        faces_from_indices(coord_index)?.0
    };
    let vertex_count = mesh.vertex_count;
    check_allocation(vertex_count as u64, VECTOR_SIZE)?;
    let mut target = vec![[0.0f32; 3]; vertex_count];
    for (face_index, texture_face) in texture_faces.iter().enumerate() {
        let Some(face) = faces(mesh).get(face_index) else {
            return undefined("a texture face read past the mesh's faces");
        };
        if face.len() != texture_face.len() {
            return fail(format!("Number of indices in texture face and mesh face must be equal. Invalid face index: {face_index}."));
        }
        for (&corner, &coordinate) in face.iter().zip(texture_face) {
            let Some(&value) = values.get(coordinate as usize) else {
                return fail("invalid vector subscript");
            };
            let Some(slot) = target.get_mut(corner as usize) else {
                return undefined("texture coordinates written past the vertices");
            };
            *slot = value;
        }
    }
    mesh.texture_coords[0] = Some(target);
    mesh.uv_components[0] = 2;
    Ok(())
}

/// `add_tex_coord(mesh, texCoords)`: one per vertex.
pub(super) fn add_tex_coord(mesh: &mut Mesh, coordinates: &[[f32; 2]]) -> Result<(), ImportError> {
    if coordinates.len() != mesh.vertex_count {
        return fail("MeshGeometry_AddTexCoord. Texture coordinates and vertices count must be equal.");
    }
    check_allocation(coordinates.len() as u64, VECTOR_SIZE)?;
    let values = coordinates.iter().map(|&[u, v]| [u, v, 0.0]).collect();
    mesh.texture_coords[0] = Some(values);
    mesh.uv_components[0] = 2;
    Ok(())
}
