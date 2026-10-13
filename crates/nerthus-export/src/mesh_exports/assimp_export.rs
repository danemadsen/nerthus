//! `MeshPort_Save` for every extension but `obj`: the native code builds an
//! `aiScene` (a "RootNode" with one mesh named "Mesh", or "PointCloud"
//! without faces, and a white "DefaultMaterial"), with vertex normals of
//! its own, then picks Assimp's exporter by the lowercased extension
//! ("gltf" → gltf2, "glb" → glb2, else the first exporter for the
//! extension) and exports.

use std::io;
use std::path::Path;

use nerthus_assimp::export::{export, format_for_extension, ExportMaterial, ExportScene};
use nerthus_assimp::scene::{Mesh as AiMesh, PRIMITIVE_POINT, PRIMITIVE_TRIANGLE};
use nerthus_nodes::maths;
use nerthus_nodes::mesh_rasterizer::Mesh;
use nerthus_nodes::x86::{addss, divss, libm_nan, mulss, sqrtss, subss};

use super::mesh_port::{Placement, SaveParameters};

/// `aiVector3D` length and `operator/=` as the native code inlines them: the
/// length from (y² + x²) + z², and unless it is zero each component times
/// its inverse.
fn normalized([x, y, z]: [f32; 3]) -> [f32; 3] {
    let y_squared = mulss(y, y);
    let x_squared = mulss(x, x);
    let z_squared = mulss(z, z);
    let squared_length = addss(addss(y_squared, x_squared), z_squared);
    let length = sqrtss(squared_length);
    if length == 0.0 {
        return [x, y, z];
    }
    let inverse = divss(1.0, length);
    [mulss(inverse, x), mulss(inverse, y), mulss(inverse, z)]
}

fn difference(to: [f32; 3], from: [f32; 3]) -> [f32; 3] {
    [subss(to[0], from[0]), subss(to[1], from[1]), subss(to[2], from[2])]
}

/// `acosf`, keeping x86 libm's NaNs.
fn acos(x: f32) -> f32 {
    libm_nan(maths::acos(x), x)
}

/// Angle-weighted vertex normals from each face's first
/// three corners (a quad's fourth corner gets nothing from it): the face
/// normal is (b − a)/|b − a| × (c − a), normalised, added at each corner
/// times the corner's angle, and the sums divided by the summed angles.
///
/// Every operation keeps the native operand order, so overflowing heights
/// give the same NaNs: `ab.y` and `bc.y` are negated before multiplying,
/// which turns x86's default NaN positive.
pub(crate) fn face_normals(vertices: &[[f32; 3]], faces: &[Vec<u32>]) -> Vec<[f32; 3]> {
    let mut sums = vec![[0.0f32; 3]; vertices.len()];
    let mut weights = vec![0.0f32; vertices.len()];
    for face in faces.iter().filter(|face| face.len() > 2) {
        let [a, b, c] = [face[0] as usize, face[1] as usize, face[2] as usize];
        let ab = normalized(difference(vertices[b], vertices[a]));
        let bc = normalized(difference(vertices[c], vertices[b]));
        let ca = normalized(difference(vertices[a], vertices[c]));
        let ac = difference(vertices[c], vertices[a]);

        let cross_x = subss(mulss(ab[1], ac[2]), mulss(ab[2], ac[1]));
        let cross_y = subss(mulss(ab[2], ac[0]), mulss(ac[2], ab[0]));
        let cross_z = subss(mulss(ac[1], ab[0]), mulss(ab[1], ac[0]));
        let normal = normalized([cross_x, cross_y, cross_z]);

        let dot_a = addss(addss(mulss(ca[0], ab[0]), mulss(ab[1], ca[1])), mulss(ab[2], ca[2]));
        let dot_b = subss(subss(mulss(-ab[1], bc[1]), mulss(ab[0], bc[0])), mulss(ab[2], bc[2]));
        let dot_c = subss(subss(mulss(-bc[1], ca[1]), mulss(ca[0], bc[0])), mulss(bc[2], ca[2]));
        let angle_a = acos(dot_a);
        let angle_b = acos(dot_b);
        let angle_c = acos(dot_c);

        for (corner, angle) in [(a, angle_a), (b, angle_b), (c, angle_c)] {
            for axis in 0..3 {
                let weighted = mulss(normal[axis], angle);
                sums[corner][axis] = addss(weighted, sums[corner][axis]);
            }
        }
        weights[a] = addss(angle_a, weights[a]);
        weights[b] = addss(angle_b, weights[b]);
        weights[c] = addss(angle_c, weights[c]);
    }
    sums.iter()
        .zip(&weights)
        .map(|(sum, &weight)| {
            let inverse = divss(1.0, weight);
            normalized([mulss(inverse, sum[0]), mulss(inverse, sum[1]), mulss(inverse, sum[2])])
        })
        .collect()
}

/// The scene for a mesh, vertices placed as the OBJ
/// writer places them, UVs (u, v, 0) with two components, triangles — or
/// each grid pair as a quad, though the mesh still says triangles.
pub(crate) fn build_scene(mesh: &Mesh, resolution: Option<(i32, i32)>, parameters: &SaveParameters) -> ExportScene {
    let placement = Placement::new(parameters, resolution);
    let vertices: Vec<[f32; 3]> = mesh.vertices.iter().map(|&vertex| placement.position(vertex)).collect();
    let mut ai_mesh = AiMesh { vertex_count: vertices.len(), material_index: 0, ..AiMesh::default() };
    if mesh.triangles.is_empty() {
        ai_mesh.name = "PointCloud".to_string();
        ai_mesh.primitive_types = PRIMITIVE_POINT;
        ai_mesh.vertices = Some(vertices);
    } else {
        ai_mesh.name = "Mesh".to_string();
        ai_mesh.primitive_types = PRIMITIVE_TRIANGLE;
        if parameters.uvs {
            let coordinates = mesh
                .vertices
                .iter()
                .map(|&vertex| {
                    let [u, v] = placement.texture_coordinate(vertex);
                    [u, v, 0.0]
                })
                .collect();
            ai_mesh.texture_coords[0] = Some(coordinates);
            ai_mesh.uv_components[0] = 2;
        }
        // Quads: count / 2 faces (an odd last triangle is dropped rather
        // than written past the array).
        let faces: Vec<Vec<u32>> = if parameters.quads {
            let (pairs, _odd_last) = mesh.triangles.as_chunks::<2>();
            pairs.iter().map(|[first, second]| vec![first[0], second[1], second[2], first[2]]).collect()
        } else {
            mesh.triangles.iter().map(|triangle| triangle.to_vec()).collect()
        };
        if parameters.normals {
            ai_mesh.normals = Some(face_normals(&vertices, &faces));
        }
        ai_mesh.vertices = Some(vertices);
        ai_mesh.faces = Some(faces);
    }
    ExportScene {
        root_name: "RootNode".to_string(),
        root_meshes: vec![0],
        meshes: vec![ai_mesh],
        materials: vec![ExportMaterial { name: "DefaultMaterial".to_string(), diffuse: [1.0, 1.0, 1.0, 1.0] }],
    }
}

/// The exporter for `path`'s extension.
fn exporter_for(path: &Path) -> Option<&'static str> {
    let text = path.to_string_lossy();
    let extension = text.rfind('.').map(|position| text[position + 1..].to_ascii_lowercase())?;
    if extension.is_empty() {
        return None;
    }
    match extension.as_str() {
        "gltf" => Some("gltf2"),
        "glb" => Some("glb2"),
        other => format_for_extension(other),
    }
}

/// Saves `mesh` through the Assimp exporter the extension picks. Returns
/// false where the export fails (no exporter for the extension).
pub fn save(path: &Path, mesh: &mut Mesh, resolution: Option<(i32, i32)>, parameters: &SaveParameters) -> io::Result<bool> {
    let Some(format) = exporter_for(path) else {
        return Ok(false);
    };
    let scene = build_scene(mesh, resolution, parameters);
    export(&scene, format, path).map_err(|error| io::Error::other(format!("Mesh export error: {error}")))?;
    Ok(true)
}
