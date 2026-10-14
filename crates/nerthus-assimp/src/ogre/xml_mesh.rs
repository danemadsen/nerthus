//! The XML formats (`OgreXmlSerializer`): a `<mesh>` of shared geometry,
//! submeshes (faces, geometry, bone assignments) and a skeleton link, read
//! from whatever document the parse left (its result is not checked); and
//! `<skeleton>` files of bones, a bone hierarchy and animations.

use super::{BoneAssignments, Bone, Skeleton, TRACK_TRANSFORM, Track, binary, ends_with, triangle_mesh, undefined};
use crate::io::FileSystem;
use crate::scene::{ImportError, MAX_TEXTURE_COORDS, Mesh, check_allocation, fail};
use crate::vector::Vector3;
use crate::xml::{self, Node as XmlNode};

/// `sizeof(aiFace)`, `sizeof(aiVector3D)` and `sizeof(std::vector)`.
const FACE_SIZE: u64 = 16;
const VECTOR_SIZE: u64 = 12;
const LIST_SIZE: u64 = 24;

/// `ThrowAttibuteError`.
fn attribute_error<T>(node: &XmlNode, name: &str, error: &str) -> Result<T, ImportError> {
    let node_name = String::from_utf8_lossy(node.name()).into_owned();
    fail(format!("{error} in node '{node_name}' and attribute '{name}'"))
}

/// An attribute that must be there.
fn attribute<'a>(node: &'a XmlNode, name: &str) -> Result<&'a [u8], ImportError> {
    match node.attribute(name) {
        Some(value) => Ok(value),
        None => attribute_error(node, name, "Not found"),
    }
}

/// `ReadAttribute<int32_t>`: `as_int`.
fn read_int(node: &XmlNode, name: &str) -> Result<i32, ImportError> {
    Ok(xml::as_int(attribute(node, name)?))
}

/// `ReadAttribute<uint32_t>`: a negative value throws.
fn read_uint(node: &XmlNode, name: &str) -> Result<u32, ImportError> {
    let value = read_int(node, name)?;
    if value < 0 {
        return attribute_error(node, name, "Found a negative number value where expecting a uint32_t value");
    }
    Ok(value as u32)
}

/// `ReadAttribute<uint16_t>`.
fn read_u16(node: &XmlNode, name: &str) -> Result<u16, ImportError> {
    Ok(read_int(node, name)? as u16)
}

/// `ReadAttribute<float>`: `as_float`.
fn read_float(node: &XmlNode, name: &str) -> Result<f32, ImportError> {
    Ok(xml::as_float(attribute(node, name)?))
}

/// `ReadAttribute<bool>`: "true" or "false" in any case.
fn read_bool(node: &XmlNode, name: &str) -> Result<bool, ImportError> {
    let value = attribute(node, name)?.to_ascii_lowercase();
    if value == b"true" {
        return Ok(true);
    }
    if value == b"false" {
        return Ok(false);
    }
    let encountered = String::from_utf8_lossy(&value).into_owned();
    attribute_error(node, name, &format!("Boolean value is expected to be 'true' or 'false', encountered '{encountered}'"))
}

/// Three float attributes, read in order.
fn read_vector(node: &XmlNode, names: [&str; 3]) -> Result<Vector3, ImportError> {
    Ok([read_float(node, names[0])?, read_float(node, names[1])?, read_float(node, names[2])?])
}

/// `VertexDataXml::uvs`: lists of texture coordinates, kept as runs of
/// lists made together. Every `<texcoord>` adds its value to every list
/// there is, so a list holds the values added since it was made.
#[derive(Default)]
struct UvSets {
    /// Every value added, in order.
    values: Vec<Vector3>,
    /// Runs of lists: how many, and how many values had been added when
    /// they were made.
    runs: Vec<(usize, usize)>,
}

impl UvSets {
    fn set_count(&self) -> usize {
        self.runs.iter().map(|&(lists, _)| lists).sum()
    }

    /// `resize`: lists dropped from the end, or new empty ones added.
    fn resize(&mut self, count: usize) {
        let mut remaining = count;
        let mut kept_runs = 0;
        for run in &mut self.runs {
            if remaining == 0 {
                break;
            }
            run.0 = run.0.min(remaining);
            remaining -= run.0;
            kept_runs += 1;
        }
        self.runs.truncate(kept_runs);
        if remaining > 0 {
            self.runs.push((remaining, self.values.len()));
        }
    }

    /// A value added to every list.
    fn push(&mut self, value: Vector3) {
        self.values.push(value);
    }

    /// The first list whose length is not `count`: its index and length.
    fn first_short(&self, count: usize) -> Option<(usize, usize)> {
        let mut set = 0;
        for &(lists, start) in &self.runs {
            let length = self.values.len() - start;
            if length != count {
                return Some((set, length));
            }
            set += lists;
        }
        None
    }

    /// The `index`th value of list `set`.
    fn value(&self, set: usize, index: usize) -> Option<Vector3> {
        let mut first_set = 0;
        for &(lists, start) in &self.runs {
            if set < first_set + lists {
                return self.values.get(start + index).copied();
            }
            first_set += lists;
        }
        None
    }
}

/// `VertexDataXml`.
#[derive(Default)]
pub(super) struct XmlVertexData {
    count: u32,
    positions: Vec<Vector3>,
    normals: Vec<Vector3>,
    tangents: usize,
    uvs: UvSets,
    bones: BoneAssignments,
}

/// `SubMeshXml`.
pub(super) struct XmlSubMesh {
    pub material: Vec<u8>,
    uses_shared: bool,
    face_count: u32,
    faces: Vec<[u32; 3]>,
    vertex_data: Option<XmlVertexData>,
}

/// `MeshXml`.
#[derive(Default)]
pub(super) struct XmlMesh {
    shared: Option<XmlVertexData>,
    pub submeshes: Vec<XmlSubMesh>,
    pub skeleton_ref: Vec<u8>,
}

/// `ReadGeometryVertexBuffer`: the flagged components of each `<vertex>`,
/// which must come to the vertex count.
fn read_vertex_buffer(node: &XmlNode, data: &mut XmlVertexData) -> Result<(), ImportError> {
    let positions = node.attribute("positions").is_some() && read_bool(node, "positions")?;
    let normals = node.attribute("normals").is_some() && read_bool(node, "normals")?;
    let tangents = node.attribute("tangents").is_some() && read_bool(node, "tangents")?;
    let uv_sets = if node.attribute("texture_coords").is_some() { read_uint(node, "texture_coords")? } else { 0 };
    if !positions && data.positions.is_empty() {
        return fail("Vertex buffer does not contain positions!");
    }
    let count = data.count as u64;
    if positions || normals || tangents {
        check_allocation(count, VECTOR_SIZE)?;
    }
    if uv_sets > 0 {
        check_allocation(uv_sets as u64, LIST_SIZE)?;
        data.uvs.resize(uv_sets as usize);
        check_allocation(count, VECTOR_SIZE)?;
    }

    for vertex in node.children.iter().filter(|child| child.name() == b"vertex") {
        for component in &vertex.children {
            let name = component.name();
            if positions && name == b"position" {
                data.positions.push(read_vector(component, ["x", "y", "z"])?);
            } else if normals && name == b"normal" {
                data.normals.push(read_vector(component, ["x", "y", "z"])?);
            } else if tangents && name == b"tangent" {
                read_vector(component, ["x", "y", "z"])?;
                data.tangents += 1;
            } else if uv_sets > 0 && name == b"texcoord" {
                // Read again for each list, to the same value.
                let u = read_float(component, "u")?;
                let v = -read_float(component, "v")? + 1.0;
                data.uvs.push([u, v, 0.0]);
            }
        }
    }

    let expected = data.count as usize;
    if data.positions.len() != expected {
        return fail(format!("Read only {} positions when should have read {expected}", data.positions.len()));
    }
    if normals && data.normals.len() != expected {
        return fail(format!("Read only {} normals when should have read {expected}", data.normals.len()));
    }
    if tangents && data.tangents != expected {
        return fail(format!("Read only {} tangents when should have read {expected}", data.tangents));
    }
    if let Some((set, length)) = data.uvs.first_short(expected) {
        return fail(format!("Read only {length} uvs for uv index {set} when should have read {expected}"));
    }
    Ok(())
}

/// `ReadGeometry`.
fn read_geometry(node: &XmlNode, data: &mut XmlVertexData) -> Result<(), ImportError> {
    data.count = read_uint(node, "vertexcount")?;
    for child in node.children.iter().filter(|child| child.name() == b"vertexbuffer") {
        read_vertex_buffer(child, data)?;
    }
    Ok(())
}

/// `ReadBoneAssignments`: the vertex data must be there.
fn read_bone_assignments(node: &XmlNode, data: Option<&mut XmlVertexData>) -> Result<(), ImportError> {
    let Some(data) = data else {
        return fail("Cannot read bone assignments, vertex data is null.");
    };
    for child in node.children.iter().filter(|child| child.name() == b"vertexboneassignment") {
        let vertex = read_uint(child, "vertexindex")?;
        let bone = read_u16(child, "boneindex")?;
        read_float(child, "weight")?;
        data.bones.list.push((vertex, bone));
    }
    Ok(())
}

/// `ReadSubMesh`.
fn read_submesh(node: &XmlNode, mesh: &mut XmlMesh) -> Result<(), ImportError> {
    let material = if node.attribute("material").is_some() { attribute(node, "material")?.to_vec() } else { Vec::new() };
    let uses_shared = node.attribute("usesharedvertices").is_some() && read_bool(node, "usesharedvertices")?;
    let mut submesh = XmlSubMesh { material, uses_shared, face_count: 0, faces: Vec::new(), vertex_data: None };
    for child in &node.children {
        let name = child.name();
        if name == b"faces" {
            submesh.face_count = read_uint(child, "count")?;
            check_allocation(submesh.face_count as u64, FACE_SIZE)?;
            for face in child.children.iter().filter(|face| face.name() == b"face") {
                let corners = [read_uint(face, "v1")?, read_uint(face, "v2")?, read_uint(face, "v3")?];
                submesh.faces.push(corners);
            }
            if submesh.faces.len() != submesh.face_count as usize {
                return fail(format!("Read only {} faces when should have read {}", submesh.faces.len(), submesh.face_count));
            }
        } else if name == b"geometry" {
            if submesh.uses_shared {
                return fail("Found <geometry> in <submesh> when use shared geometry is true. Invalid mesh file.");
            }
            let mut data = XmlVertexData::default();
            read_geometry(child, &mut data)?;
            submesh.vertex_data = Some(data);
        } else if name == b"boneassignments" {
            read_bone_assignments(child, submesh.vertex_data.as_mut())?;
        }
    }
    mesh.submeshes.push(submesh);
    Ok(())
}

/// `ImportMesh` from the document the parse leaves, failed or not.
pub(super) fn read_mesh(bytes: &[u8]) -> Result<XmlMesh, ImportError> {
    let document = xml::parse_partial(bytes);
    // The root is the document node, which has no name.
    let Some(start) = document.child("mesh") else {
        return fail("Root node is <> expecting <mesh>");
    };
    let mut mesh = XmlMesh::default();
    for child in &start.children {
        match child.name() {
            b"sharedgeometry" => {
                let mut data = XmlVertexData::default();
                read_geometry(child, &mut data)?;
                mesh.shared = Some(data);
            }
            b"submeshes" => {
                for submesh in child.children.iter().filter(|submesh| submesh.name() == b"submesh") {
                    read_submesh(submesh, &mut mesh)?;
                }
            }
            b"boneassignments" => read_bone_assignments(child, mesh.shared.as_mut())?,
            b"skeletonlink" => mesh.skeleton_ref = child.attribute("name").unwrap_or_default().to_vec(),
            _ => {}
        }
    }
    Ok(mesh)
}

/// `ReadBones`: bones with positions, rotations (each child an `<axis>`)
/// and scales, then in id order with ids 0 to n−1.
fn read_bones(node: &XmlNode, skeleton: &mut Skeleton) -> Result<(), ImportError> {
    for child in node.children.iter().filter(|child| child.name() == b"bone") {
        let id = read_u16(child, "id")?;
        let name = attribute(child, "name")?.to_vec();
        for part in &child.children {
            match part.name() {
                b"position" => {
                    read_vector(part, ["x", "y", "z"])?;
                }
                b"rotation" => {
                    read_float(part, "angle")?;
                    for axis in &part.children {
                        if axis.name() != b"axis" {
                            return fail(format!("No axis specified for bone rotation in bone {id}"));
                        }
                        read_vector(axis, ["x", "y", "z"])?;
                    }
                }
                b"scale" => {
                    if part.attribute("factor").is_some() {
                        read_float(part, "factor")?;
                    } else {
                        for axis in ["x", "y", "z"] {
                            if part.attribute(axis).is_some() {
                                read_float(part, axis)?;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        skeleton.bones.push(Bone::new(id, name));
    }
    skeleton.bones.sort_by_key(|bone| bone.id);
    for (index, bone) in skeleton.bones.iter().enumerate() {
        if bone.id != index as u16 {
            return fail(format!("Bone ids are not in sequence starting from 0. Missing index {index}"));
        }
    }
    Ok(())
}

/// `ReadBoneHierarchy`: parents by name, then the bone matrices.
fn read_bone_hierarchy(node: &XmlNode, skeleton: &mut Skeleton) -> Result<(), ImportError> {
    if skeleton.bones.is_empty() {
        return fail("Cannot read <bonehierarchy> for a Skeleton without bones");
    }
    for child in node.children.iter().filter(|child| child.name() == b"boneparent") {
        let name = attribute(child, "bone")?;
        let parent_name = attribute(child, "parent")?;
        match (skeleton.bone_by_name(name), skeleton.bone_by_name(parent_name)) {
            (Some(bone), Some(parent)) => skeleton.add_child(parent, bone)?,
            _ => {
                let name = String::from_utf8_lossy(name).into_owned();
                let parent_name = String::from_utf8_lossy(parent_name).into_owned();
                return fail(format!("Failed to find bones for parenting: Child {name} for parent {parent_name}"));
            }
        }
    }
    skeleton.walk_from_roots("CalculateWorldMatrixAndDefaultPose")
}

/// `ReadAnimationKeyFrames`.
fn read_key_frames(node: &XmlNode) -> Result<(), ImportError> {
    for frame in node.children.iter().filter(|frame| frame.name() == b"keyframe") {
        read_float(frame, "time")?;
        for part in &frame.children {
            match part.name() {
                b"translate" | b"scale" => {
                    read_vector(part, ["x", "y", "z"])?;
                }
                b"rotate" => {
                    read_float(part, "angle")?;
                    for axis in part.children.iter().filter(|axis| axis.name() == b"axis") {
                        read_vector(axis, ["x", "y", "z"])?;
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// `ReadAnimations`: every child of an animation must be `<tracks>`, and
/// of a track `<keyframes>`.
fn read_animations(node: &XmlNode, skeleton: &mut Skeleton) -> Result<(), ImportError> {
    if skeleton.bones.is_empty() {
        return fail("Cannot read <animations> for a Skeleton without bones");
    }
    for animation in node.children.iter().filter(|animation| animation.name() == b"animation") {
        let name = String::from_utf8_lossy(attribute(animation, "name")?).into_owned();
        read_float(animation, "length")?;
        let mut tracks = Vec::new();
        for list in &animation.children {
            if list.name() != b"tracks" {
                return fail(format!("No <tracks> found in <animation> {name}"));
            }
            for track in list.children.iter().filter(|track| track.name() == b"track") {
                let bone = attribute(track, "bone")?.to_vec();
                for frames in &track.children {
                    if frames.name() != b"keyframes" {
                        return fail(format!("No <keyframes> found in <track> {name}"));
                    }
                    read_key_frames(frames)?;
                }
                tracks.push(Track { kind: TRACK_TRANSFORM, bone });
            }
        }
        skeleton.animations.push(tracks);
    }
    Ok(())
}

/// `ReadSkeleton`: the node must be `<skeleton>`.
fn read_skeleton(node: &XmlNode) -> Result<Skeleton, ImportError> {
    if node.name() != b"skeleton" {
        let name = String::from_utf8_lossy(node.name()).into_owned();
        return fail(format!("Root node is <{name}> expecting <skeleton>"));
    }
    let mut skeleton = Skeleton::default();
    for child in &node.children {
        match child.name() {
            b"bones" => read_bones(child, &mut skeleton)?,
            b"bonehierarchy" => read_bone_hierarchy(child, &mut skeleton)?,
            b"animations" => read_animations(child, &mut skeleton)?,
            _ => {}
        }
    }
    Ok(skeleton)
}

/// `OpenXmlParser` and `ReadSkeleton`: none when the name does not end
/// `.skeleton.xml` or the file is not there; a file that does not parse
/// throws. The skeleton is read from the document itself, or (for an XML
/// mesh) from its first `<skeleton>` child.
pub(super) fn read_skeleton_file(files: &FileSystem, name: &[u8], find_element: bool) -> Result<Option<Skeleton>, ImportError> {
    if !ends_with(name, b".skeleton.xml") || !files.exists_bytes(name) {
        return Ok(None);
    }
    let file_name = String::from_utf8_lossy(name).into_owned();
    let Some(bytes) = files.open_bytes(name) else {
        return fail(format!("Failed to open skeleton file {file_name}"));
    };
    let Ok(document) = xml::parse(&bytes) else {
        return fail(format!("Failed to create XML reader for skeleton file {file_name}"));
    };
    let mut root = &document;
    if find_element
        && let Some(element) = document.children.iter().find(|child| child.name() == b"skeleton")
    {
        root = element;
    }
    Ok(Some(read_skeleton(root)?))
}

/// `OgreXmlSerializer::ImportSkeleton` for an XML mesh: a binary skeleton
/// first when the name ends `.skeleton`, else (or when that file is not
/// there) the name with `.xml` added.
pub(super) fn import_skeleton(files: &FileSystem, name: &mut Vec<u8>) -> Result<Option<Skeleton>, ImportError> {
    if name.is_empty() {
        return Ok(None);
    }
    if ends_with(name, b".skeleton") {
        if let Some(skeleton) = binary::read_skeleton_file(files, name)? {
            return Ok(Some(skeleton));
        }
        name.extend_from_slice(b".xml");
    }
    read_skeleton_file(files, name, true)
}

/// `SubMeshXml::ConvertToAssimpMesh`: a vertex per face corner, the
/// corners indexing the vertex data's lists (past their ends: undefined
/// behaviour).
fn convert_submesh(submesh: &mut XmlSubMesh, shared: Option<&mut XmlVertexData>, skeleton: Option<&Skeleton>) -> Result<Mesh, ImportError> {
    let (mut mesh, vertex_count) = triangle_mesh(submesh.face_count)?;
    let data = if submesh.uses_shared { shared } else { submesh.vertex_data.as_mut() };
    let Some(data) = data else {
        return undefined("a submesh without vertex data");
    };
    let uv_sets = data.uvs.set_count();
    if uv_sets > MAX_TEXTURE_COORDS {
        return undefined("texture coordinate sets written past the mesh's arrays");
    }

    let vertex_total = vertex_count as usize;
    let mut vertices = vec![[0.0f32; 3]; vertex_total];
    let mut normals = (!data.normals.is_empty()).then(|| vec![[0.0f32; 3]; vertex_total]);
    let mut uvs = vec![vec![[0.0f32; 3]; vertex_total]; uv_sets];
    let weighs_bones = skeleton.is_some() && !data.bones.list.is_empty();
    let by_vertex = if weighs_bones { data.bones.by_vertex() } else { Default::default() };
    for (face, corners) in submesh.faces.iter().enumerate() {
        for (corner, &source_index) in corners.iter().enumerate() {
            let target = face * 3 + corner;
            if weighs_bones {
                data.bones.map(&by_vertex, source_index, target as u32);
            }
            let index = source_index as usize;
            let Some(&position) = data.positions.get(index) else {
                return undefined("a face corner past the positions");
            };
            vertices[target] = position;
            // Normals and texture coordinates, when there are any, come to
            // as many as the positions.
            if let Some(normals) = normals.as_mut() {
                let Some(&normal) = data.normals.get(index) else {
                    return undefined("a face corner past the normals");
                };
                normals[target] = normal;
            }
            for (set, values) in uvs.iter_mut().enumerate() {
                let Some(uv) = data.uvs.value(set, index) else {
                    return undefined("a face corner past the texture coordinates");
                };
                values[target] = uv;
            }
        }
    }

    mesh.vertices = Some(vertices);
    mesh.normals = normals;
    for (set, values) in uvs.into_iter().enumerate() {
        mesh.uv_components[set] = 2;
        mesh.texture_coords[set] = Some(values);
    }
    if let Some(skeleton) = skeleton
        && weighs_bones
    {
        mesh.bones = data.bones.mesh_bones(vertex_count, skeleton)?;
    }
    Ok(mesh)
}

/// `MeshXml::ConvertToAssimpScene`'s meshes: the submeshes in order.
pub(super) fn convert(mesh: &mut XmlMesh, skeleton: Option<&Skeleton>) -> Result<Vec<Mesh>, ImportError> {
    let mut meshes = Vec::with_capacity(mesh.submeshes.len());
    for submesh in &mut mesh.submeshes {
        meshes.push(convert_submesh(submesh, mesh.shared.as_mut(), skeleton)?);
    }
    Ok(meshes)
}
