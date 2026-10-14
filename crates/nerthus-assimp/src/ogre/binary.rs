//! The binary formats (`OgreBinarySerializer`): chunks of a 16-bit id and
//! a 32-bit length; a reader steps back over a header it does not take.

use std::collections::BTreeMap;

use super::{
    BoneAssignments, Bone, OPERATION_POINT_LIST, OPERATION_TRIANGLE_LIST, SEMANTIC_NORMAL, SEMANTIC_POSITION,
    SEMANTIC_TEXTURE_COORDINATES, Skeleton, TRACK_TRANSFORM, TYPE_FLOAT2, TYPE_FLOAT3, Track, ends_with, floats, triangle_mesh, type_size,
    undefined, xml_mesh,
};
use crate::io::FileSystem;
use crate::scene::{ImportError, Mesh, check_allocation, fail};
use crate::stream_reader::StreamReader;

const HEADER: u16 = 0x1000;
const MESH: u16 = 0x3000;
const SUBMESH: u16 = 0x4000;
const SUBMESH_OPERATION: u16 = 0x4010;
const SUBMESH_BONE_ASSIGNMENT: u16 = 0x4100;
const SUBMESH_TEXTURE_ALIAS: u16 = 0x4200;
const GEOMETRY: u16 = 0x5000;
const GEOMETRY_VERTEX_DECLARATION: u16 = 0x5100;
const GEOMETRY_VERTEX_ELEMENT: u16 = 0x5110;
const GEOMETRY_VERTEX_BUFFER: u16 = 0x5200;
const GEOMETRY_VERTEX_BUFFER_DATA: u16 = 0x5210;
const MESH_SKELETON_LINK: u16 = 0x6000;
const MESH_BONE_ASSIGNMENT: u16 = 0x7000;
const MESH_LOD: u16 = 0x8000;
const MESH_LOD_USAGE: u16 = 0x8100;
const MESH_LOD_MANUAL: u16 = 0x8110;
const MESH_LOD_GENERATED: u16 = 0x8120;
const MESH_BOUNDS: u16 = 0x9000;
const SUBMESH_NAME_TABLE: u16 = 0xA000;
const SUBMESH_NAME_TABLE_ELEMENT: u16 = 0xA100;
const EDGE_LISTS: u16 = 0xB000;
const EDGE_LIST_LOD: u16 = 0xB100;
const EDGE_GROUP: u16 = 0xB110;
const POSES: u16 = 0xC000;
const POSE: u16 = 0xC100;
const POSE_VERTEX: u16 = 0xC111;
const ANIMATIONS: u16 = 0xD000;
const ANIMATION: u16 = 0xD100;
const ANIMATION_BASEINFO: u16 = 0xD105;
const ANIMATION_TRACK: u16 = 0xD110;
const ANIMATION_MORPH_KEYFRAME: u16 = 0xD111;
const ANIMATION_POSE_KEYFRAME: u16 = 0xD112;
const ANIMATION_POSE_REF: u16 = 0xD113;
const TABLE_EXTREMES: u16 = 0xE000;

const SKELETON_BLENDMODE: u16 = 0x1010;
const SKELETON_BONE: u16 = 0x2000;
const SKELETON_BONE_PARENT: u16 = 0x3000;
const SKELETON_ANIMATION: u16 = 0x4000;
const SKELETON_ANIMATION_BASEINFO: u16 = 0x4010;
const SKELETON_ANIMATION_TRACK: u16 = 0x4100;
const SKELETON_ANIMATION_TRACK_KEYFRAME: u16 = 0x4110;
const SKELETON_ANIMATION_LINK: u16 = 0x5000;

/// The chunks `ReadMesh` takes.
const MESH_CHUNKS: [u16; 11] = [
    GEOMETRY,
    SUBMESH,
    MESH_SKELETON_LINK,
    MESH_BONE_ASSIGNMENT,
    MESH_LOD,
    MESH_BOUNDS,
    SUBMESH_NAME_TABLE,
    EDGE_LISTS,
    POSES,
    ANIMATIONS,
    TABLE_EXTREMES,
];

const MESH_VERSION: &[u8] = b"[MeshSerializer_v1.8]";
const SKELETON_VERSION_1_8: &[u8] = b"[Serializer_v1.80]";
const SKELETON_VERSION_1_1: &[u8] = b"[Serializer_v1.10]";

/// `MSTREAM_OVERHEAD_SIZE`, and the bone and key frame chunk lengths
/// without a scale.
const OVERHEAD_SIZE: u32 = 6;
const BONE_SIZE_WITHOUT_SCALE: u32 = OVERHEAD_SIZE + 2 + 4 * 7;
const KEYFRAME_SIZE_WITHOUT_SCALE: u32 = OVERHEAD_SIZE + 4 * 8;

/// `MemoryIOStream` over a buffer: a failed seek keeps the position, and a
/// read copies only whole elements.
pub(super) struct MemoryStream {
    data: Vec<u8>,
    position: usize,
}

impl MemoryStream {
    fn new(data: Vec<u8>) -> Self {
        MemoryStream { data, position: 0 }
    }

    /// `Seek(offset, aiOrigin_SET)`.
    fn seek(&mut self, offset: u64) {
        if offset <= self.data.len() as u64 {
            self.position = offset as usize;
        }
    }

    /// `Read(out, size, count)`: the elements copied into `out`.
    fn read(&mut self, out: &mut [u8], size: usize, count: usize) -> usize {
        let copied = count.min((self.data.len() - self.position) / size);
        let bytes = copied * size;
        out[..bytes].copy_from_slice(&self.data[self.position..self.position + bytes]);
        self.position += bytes;
        copied
    }
}

/// `VertexElement`.
#[derive(Clone, Copy)]
struct VertexElement {
    source: u16,
    kind: u16,
    semantic: u16,
    offset: u16,
    index: u16,
}

/// `VertexData`.
#[derive(Default)]
pub(super) struct VertexData {
    count: u32,
    elements: Vec<VertexElement>,
    /// The buffers by bind index, a later one replacing an earlier.
    buffers: BTreeMap<u16, MemoryStream>,
    bones: BoneAssignments,
}

impl VertexData {
    /// `VertexSize`: the sizes of a source's elements.
    fn vertex_size(&self, source: u16) -> u32 {
        let sizes = self.elements.iter().filter(|element| element.source == source).map(|element| type_size(element.kind));
        sizes.fold(0u32, u32::wrapping_add)
    }

    /// `GetVertexElement`: the first element of the semantic and index.
    fn element(&self, semantic: u16, index: u16) -> Option<VertexElement> {
        self.elements.iter().copied().find(|element| element.semantic == semantic && element.index == index)
    }
}

/// `SubMesh`.
pub(super) struct SubMesh {
    pub material: Vec<u8>,
    uses_shared: bool,
    face_count: u32,
    is_32bit: bool,
    indices: Option<MemoryStream>,
    vertex_data: Option<VertexData>,
    operation: u16,
    index: u32,
}

/// `Mesh`.
#[derive(Default)]
pub(super) struct BinaryMesh {
    shared: Option<VertexData>,
    pub submeshes: Vec<SubMesh>,
    pub skeleton_ref: Vec<u8>,
}

impl BinaryMesh {
    /// `GetSubMesh`: by the submesh's index.
    fn submesh(&self, index: u32) -> Option<&SubMesh> {
        self.submeshes.iter().find(|submesh| submesh.index == index)
    }
}

/// The serializer's reading state: the stream and the length of the last
/// chunk header.
struct Reader<'a> {
    stream: StreamReader<'a>,
    current_length: u32,
}

impl Reader<'_> {
    /// `AtEnd`.
    fn at_end(&self) -> bool {
        self.stream.remaining_size() == 0
    }

    /// `ReadHeader`: the id, and the length when asked.
    fn header(&mut self, read_length: bool) -> Result<u16, ImportError> {
        let id = self.stream.get_u2()?;
        if read_length {
            self.current_length = self.stream.get_u4()?;
        }
        Ok(id)
    }

    /// `RollbackHeader`.
    fn rollback(&mut self) -> Result<(), ImportError> {
        self.stream.inc_ptr(-(OVERHEAD_SIZE as i64))
    }

    /// `ReadLine`: up to a '\n' or the end.
    fn line(&mut self) -> Result<Vec<u8>, ImportError> {
        let mut line = Vec::new();
        while !self.at_end() {
            let byte = self.stream.get_u1()?;
            if byte == b'\n' {
                break;
            }
            line.push(byte);
        }
        Ok(line)
    }

    fn boolean(&mut self) -> Result<bool, ImportError> {
        Ok(self.stream.get_u1()? > 0)
    }

    /// `SkipBytes` / `IncPtr` with a byte count (as `intptr_t`).
    fn skip(&mut self, count: u64) -> Result<(), ImportError> {
        self.stream.inc_ptr(count as i64)
    }

    /// `ReadBytes`: the allocation, then the copy.
    fn bytes(&mut self, count: u64) -> Result<Vec<u8>, ImportError> {
        check_allocation(count, 1)?;
        Ok(self.stream.copy_and_advance(count as usize)?.to_vec())
    }

    /// `ReadMesh`.
    fn mesh(&mut self, mesh: &mut BinaryMesh) -> Result<(), ImportError> {
        self.boolean()?;
        if self.at_end() {
            return Ok(());
        }
        let mut id = self.header(true)?;
        while !self.at_end() && MESH_CHUNKS.contains(&id) {
            match id {
                GEOMETRY => {
                    let mut data = VertexData::default();
                    self.geometry(&mut data)?;
                    mesh.shared = Some(data);
                }
                SUBMESH => self.submesh(mesh)?,
                MESH_SKELETON_LINK => mesh.skeleton_ref = self.line()?,
                MESH_BONE_ASSIGNMENT => self.bone_assignment(mesh.shared.as_mut())?,
                MESH_LOD => self.lod_info(mesh)?,
                MESH_BOUNDS => self.skip(4 * 7)?,
                SUBMESH_NAME_TABLE => self.submesh_names(mesh)?,
                EDGE_LISTS => self.edge_lists()?,
                POSES => self.poses()?,
                ANIMATIONS => self.animations(mesh)?,
                // `m_currentLen - MSTREAM_OVERHEAD_SIZE` in 32-bit `long`s
                // (Windows), wrapping below the header's size.
                _ => self.skip(self.current_length.wrapping_sub(OVERHEAD_SIZE) as u64)?,
            }
            if !self.at_end() {
                id = self.header(true)?;
            }
        }
        if !self.at_end() {
            self.rollback()?;
        }
        Ok(())
    }

    /// `ReadMeshLodInfo`: skipped, but for its checks.
    fn lod_info(&mut self, mesh: &BinaryMesh) -> Result<(), ImportError> {
        self.line()?;
        let levels = self.stream.get_u2()?;
        let manual = self.boolean()?;
        for _ in 1..levels {
            if self.header(true)? != MESH_LOD_USAGE {
                return fail("M_MESH_LOD does not contain a M_MESH_LOD_USAGE for each LOD level");
            }
            self.skip(4)?;
            if manual {
                if self.header(true)? != MESH_LOD_MANUAL {
                    return fail("Manual M_MESH_LOD_USAGE does not contain M_MESH_LOD_MANUAL");
                }
                self.line()?;
                continue;
            }
            for _ in 0..mesh.submeshes.len() {
                if self.header(true)? != MESH_LOD_GENERATED {
                    return fail("Generated M_MESH_LOD_USAGE does not contain M_MESH_LOD_GENERATED");
                }
                let index_count = self.stream.get_u4()?;
                let is_32bit = self.boolean()?;
                if index_count > 0 {
                    let index_size: u64 = if is_32bit { 4 } else { 2 };
                    let length = (index_count as u64 * index_size) as u32;
                    self.skip(length as u64)?;
                }
            }
        }
        Ok(())
    }

    /// `ReadBoneAssignment`.
    fn bone_assignment(&mut self, data: Option<&mut VertexData>) -> Result<(), ImportError> {
        let Some(data) = data else {
            return fail("Cannot read bone assignments, vertex data is null.");
        };
        let vertex = self.stream.get_u4()?;
        let bone = self.stream.get_u2()?;
        self.stream.get_f4()?;
        data.bones.list.push((vertex, bone));
        Ok(())
    }

    /// `ReadSubMesh`.
    fn submesh(&mut self, mesh: &mut BinaryMesh) -> Result<(), ImportError> {
        let material = self.line()?;
        let uses_shared = self.boolean()?;
        let index_count = self.stream.get_u4()?;
        let is_32bit = self.boolean()?;
        let mut submesh = SubMesh {
            material,
            uses_shared,
            face_count: index_count / 3,
            is_32bit,
            indices: None,
            vertex_data: None,
            operation: OPERATION_POINT_LIST,
            index: 0,
        };
        if index_count > 0 {
            let index_size: u64 = if is_32bit { 4 } else { 2 };
            let length = (index_count as u64 * index_size) as u32;
            submesh.indices = Some(MemoryStream::new(self.bytes(length as u64)?));
        }
        if !uses_shared {
            if self.header(true)? != GEOMETRY {
                return fail("M_SUBMESH does not contain M_GEOMETRY, but shader geometry is set to false");
            }
            let mut data = VertexData::default();
            self.geometry(&mut data)?;
            submesh.vertex_data = Some(data);
        }
        if !self.at_end() {
            let mut id = self.header(true)?;
            while !self.at_end() && matches!(id, SUBMESH_OPERATION | SUBMESH_BONE_ASSIGNMENT | SUBMESH_TEXTURE_ALIAS) {
                match id {
                    SUBMESH_OPERATION => submesh.operation = self.stream.get_u2()?,
                    SUBMESH_BONE_ASSIGNMENT => self.bone_assignment(submesh.vertex_data.as_mut())?,
                    _ => {
                        self.line()?;
                        self.line()?;
                    }
                }
                if !self.at_end() {
                    id = self.header(true)?;
                }
            }
            if !self.at_end() {
                self.rollback()?;
            }
        }
        submesh.index = mesh.submeshes.len() as u32;
        mesh.submeshes.push(submesh);
        Ok(())
    }

    /// `ReadSubMeshNames`: each name's submesh must exist.
    fn submesh_names(&mut self, mesh: &BinaryMesh) -> Result<(), ImportError> {
        if self.at_end() {
            return Ok(());
        }
        let mut id = self.header(true)?;
        while !self.at_end() && id == SUBMESH_NAME_TABLE_ELEMENT {
            let index = self.stream.get_u2()?;
            if mesh.submesh(index as u32).is_none() {
                return fail(format!("Ogre Mesh does not include submesh {index} referenced in M_SUBMESH_NAME_TABLE_ELEMENT. Invalid mesh file."));
            }
            self.line()?;
            if !self.at_end() {
                id = self.header(true)?;
            }
        }
        if !self.at_end() {
            self.rollback()?;
        }
        Ok(())
    }

    /// `ReadGeometry`: the vertex count, declarations and buffers.
    fn geometry(&mut self, data: &mut VertexData) -> Result<(), ImportError> {
        data.count = self.stream.get_u4()?;
        if self.at_end() {
            return Ok(());
        }
        let mut id = self.header(true)?;
        while !self.at_end() && matches!(id, GEOMETRY_VERTEX_DECLARATION | GEOMETRY_VERTEX_BUFFER) {
            if id == GEOMETRY_VERTEX_DECLARATION {
                self.declaration(data)?;
            } else {
                self.vertex_buffer(data)?;
            }
            if !self.at_end() {
                id = self.header(true)?;
            }
        }
        if !self.at_end() {
            self.rollback()?;
        }
        Ok(())
    }

    /// `ReadGeometryVertexDeclaration`.
    fn declaration(&mut self, data: &mut VertexData) -> Result<(), ImportError> {
        if self.at_end() {
            return Ok(());
        }
        let mut id = self.header(true)?;
        while !self.at_end() && id == GEOMETRY_VERTEX_ELEMENT {
            let source = self.stream.get_u2()?;
            let kind = self.stream.get_u2()?;
            let semantic = self.stream.get_u2()?;
            let offset = self.stream.get_u2()?;
            let index = self.stream.get_u2()?;
            data.elements.push(VertexElement { source, kind, semantic, offset, index });
            if !self.at_end() {
                id = self.header(true)?;
            }
        }
        if !self.at_end() {
            self.rollback()?;
        }
        Ok(())
    }

    /// `ReadGeometryVertexBuffer`: its size must agree with the elements.
    fn vertex_buffer(&mut self, data: &mut VertexData) -> Result<(), ImportError> {
        let bind = self.stream.get_u2()?;
        let vertex_size = self.stream.get_u2()?;
        if self.header(true)? != GEOMETRY_VERTEX_BUFFER_DATA {
            return fail("M_GEOMETRY_VERTEX_BUFFER_DATA not found in M_GEOMETRY_VERTEX_BUFFER");
        }
        if data.vertex_size(bind) != vertex_size as u32 {
            return fail("Vertex buffer size does not agree with vertex declaration in M_GEOMETRY_VERTEX_BUFFER");
        }
        // `count * vertexSize` in unsigned ints.
        let length = data.count.wrapping_mul(vertex_size as u32);
        let bytes = self.bytes(length as u64)?;
        data.buffers.insert(bind, MemoryStream::new(bytes));
        Ok(())
    }

    /// `ReadEdgeList`: skipped, but for its checks.
    fn edge_lists(&mut self) -> Result<(), ImportError> {
        if self.at_end() {
            return Ok(());
        }
        let mut id = self.header(true)?;
        while !self.at_end() && id == EDGE_LIST_LOD {
            self.skip(2)?;
            let manual = self.boolean()?;
            if !manual {
                self.skip(1)?;
                let triangles = self.stream.get_u4()?;
                let groups = self.stream.get_u4()?;
                self.skip(48 * triangles as u64)?;
                for _ in 0..groups {
                    if self.header(true)? != EDGE_GROUP {
                        return fail("M_EDGE_GROUP not found in M_EDGE_LIST_LOD");
                    }
                    self.skip(12)?;
                    let edges = self.stream.get_u4()?;
                    self.skip(25 * edges as u64)?;
                }
            }
            if !self.at_end() {
                id = self.header(true)?;
            }
        }
        if !self.at_end() {
            self.rollback()?;
        }
        Ok(())
    }

    /// `ReadPoses` and `ReadPoseVertices`.
    fn poses(&mut self) -> Result<(), ImportError> {
        if self.at_end() {
            return Ok(());
        }
        let mut id = self.header(true)?;
        while !self.at_end() && id == POSE {
            self.line()?;
            self.stream.get_u2()?;
            let has_normals = self.boolean()?;
            if !self.at_end() {
                let mut vertex_id = self.header(true)?;
                while !self.at_end() && vertex_id == POSE_VERTEX {
                    self.stream.get_u4()?;
                    self.stream.copy_and_advance(12)?;
                    if has_normals {
                        self.stream.copy_and_advance(12)?;
                    }
                    if !self.at_end() {
                        vertex_id = self.header(true)?;
                    }
                }
                if !self.at_end() {
                    self.rollback()?;
                }
            }
            if !self.at_end() {
                id = self.header(true)?;
            }
        }
        if !self.at_end() {
            self.rollback()?;
        }
        Ok(())
    }

    /// `ReadAnimations`.
    fn animations(&mut self, mesh: &BinaryMesh) -> Result<(), ImportError> {
        if self.at_end() {
            return Ok(());
        }
        let mut id = self.header(true)?;
        while !self.at_end() && id == ANIMATION {
            self.line()?;
            self.stream.get_f4()?;
            self.animation(mesh)?;
            if !self.at_end() {
                id = self.header(true)?;
            }
        }
        if !self.at_end() {
            self.rollback()?;
        }
        Ok(())
    }

    /// `ReadAnimation`.
    fn animation(&mut self, mesh: &BinaryMesh) -> Result<(), ImportError> {
        if self.at_end() {
            return Ok(());
        }
        let mut id = self.header(true)?;
        if id == ANIMATION_BASEINFO {
            self.line()?;
            self.stream.get_f4()?;
            id = self.header(true)?;
        }
        while !self.at_end() && id == ANIMATION_TRACK {
            self.stream.get_u2()?;
            let target = self.stream.get_u2()?;
            self.key_frames(mesh, target)?;
            if !self.at_end() {
                id = self.header(true)?;
            }
        }
        if !self.at_end() {
            self.rollback()?;
        }
        Ok(())
    }

    /// `ReadAnimationKeyFrames`: a morph key frame holds a position (and
    /// normal) for each vertex of the track's vertex data, which must be
    /// there.
    fn key_frames(&mut self, mesh: &BinaryMesh, target: u16) -> Result<(), ImportError> {
        if self.at_end() {
            return Ok(());
        }
        let mut id = self.header(true)?;
        while !self.at_end() && matches!(id, ANIMATION_MORPH_KEYFRAME | ANIMATION_POSE_KEYFRAME) {
            if id == ANIMATION_MORPH_KEYFRAME {
                self.stream.get_f4()?;
                let has_normals = self.boolean()?;
                // `AssociatedVertexData`.
                let data = if target == 0 {
                    mesh.shared.as_ref()
                } else {
                    mesh.submesh(target as u32 - 1).and_then(|submesh| submesh.vertex_data.as_ref())
                };
                let Some(data) = data else {
                    return undefined("a morph key frame of vertex data that is not there");
                };
                let vertex_size: u64 = if has_normals { 24 } else { 12 };
                self.bytes(data.count as u64 * vertex_size)?;
            } else {
                self.stream.get_f4()?;
                if !self.at_end() {
                    let mut reference = self.header(true)?;
                    while !self.at_end() && reference == ANIMATION_POSE_REF {
                        self.stream.get_u2()?;
                        self.stream.get_f4()?;
                        if !self.at_end() {
                            reference = self.header(true)?;
                        }
                    }
                    if !self.at_end() {
                        self.rollback()?;
                    }
                }
            }
            if !self.at_end() {
                id = self.header(true)?;
            }
        }
        if !self.at_end() {
            self.rollback()?;
        }
        Ok(())
    }

    /// `ReadSkeleton`: bones (with contiguous ids from 0), parents and
    /// animations, then the bone matrices from each unparented bone.
    fn skeleton(&mut self) -> Result<Skeleton, ImportError> {
        if self.header(false)? != HEADER {
            return fail("Invalid Ogre Skeleton file header.");
        }
        let version = self.line()?;
        if version != SKELETON_VERSION_1_8 && version != SKELETON_VERSION_1_1 {
            return fail("Skeleton version not supported by this importer.");
        }
        let mut skeleton = Skeleton::default();
        while !self.at_end() {
            match self.header(true)? {
                SKELETON_BLENDMODE => {
                    self.stream.get_u2()?;
                }
                SKELETON_BONE => self.bone(&mut skeleton)?,
                SKELETON_BONE_PARENT => {
                    let child_id = self.stream.get_u2()?;
                    let parent_id = self.stream.get_u2()?;
                    match (skeleton.bone_by_id(child_id), skeleton.bone_by_id(parent_id)) {
                        (Some(child), Some(parent)) => skeleton.add_child(parent, child)?,
                        _ => return fail(format!("Failed to find bones for parenting: Child id {child_id} for parent id {parent_id}")),
                    }
                }
                SKELETON_ANIMATION => self.skeleton_animation(&mut skeleton)?,
                SKELETON_ANIMATION_LINK => {
                    self.line()?;
                    self.skip(12)?;
                }
                _ => {}
            }
        }
        skeleton.walk_from_roots("CalculateWorldMatrixAndDefaultPose")?;
        Ok(skeleton)
    }

    /// `ReadBone`: name, id, position, rotation and, in a longer chunk, a
    /// scale.
    fn bone(&mut self, skeleton: &mut Skeleton) -> Result<(), ImportError> {
        let name = self.line()?;
        let id = self.stream.get_u2()?;
        self.stream.copy_and_advance(12)?;
        self.stream.copy_and_advance(16)?;
        if self.current_length > BONE_SIZE_WITHOUT_SCALE {
            self.stream.copy_and_advance(12)?;
        }
        if id as usize != skeleton.bones.len() {
            return fail(format!("Ogre Skeleton bone indexes not contiguous. Error at bone index {id}"));
        }
        skeleton.bones.push(Bone::new(id, name));
        Ok(())
    }

    /// `ReadSkeletonAnimation` and its tracks.
    fn skeleton_animation(&mut self, skeleton: &mut Skeleton) -> Result<(), ImportError> {
        self.line()?;
        self.stream.get_f4()?;
        let mut tracks = Vec::new();
        if !self.at_end() {
            let mut id = self.header(true)?;
            if id == SKELETON_ANIMATION_BASEINFO {
                self.line()?;
                self.stream.get_f4()?;
                id = self.header(true)?;
            }
            while !self.at_end() && id == SKELETON_ANIMATION_TRACK {
                let bone_id = self.stream.get_u2()?;
                let Some(bone) = skeleton.bone_by_id(bone_id) else {
                    return fail(format!("Cannot read animation track, target bone {bone_id} not in target Skeleton"));
                };
                tracks.push(Track { kind: TRACK_TRANSFORM, bone: skeleton.bones[bone].name.clone() });
                let mut frame_id = self.header(true)?;
                while !self.at_end() && frame_id == SKELETON_ANIMATION_TRACK_KEYFRAME {
                    self.stream.get_f4()?;
                    self.stream.copy_and_advance(16)?;
                    self.stream.copy_and_advance(12)?;
                    if self.current_length > KEYFRAME_SIZE_WITHOUT_SCALE {
                        self.stream.copy_and_advance(12)?;
                    }
                    if !self.at_end() {
                        frame_id = self.header(true)?;
                    }
                }
                if !self.at_end() {
                    self.rollback()?;
                }
                if !self.at_end() {
                    id = self.header(true)?;
                }
            }
            if !self.at_end() {
                self.rollback()?;
            }
        }
        skeleton.animations.push(tracks);
        Ok(())
    }
}

/// `ImportMesh`: the header chunk and version line, then every `M_MESH`
/// chunk (other ids only have their header read).
pub(super) fn read_mesh(bytes: &[u8]) -> Result<BinaryMesh, ImportError> {
    let stream = StreamReader::new(bytes, false)?;
    let mut reader = Reader { stream, current_length: 0 };
    if reader.header(false)? != HEADER {
        return fail("Invalid Ogre Mesh file header.");
    }
    if reader.line()? != MESH_VERSION {
        return fail("Mesh version not supported by this importer. Run OgreMeshUpgrader tool on the file and try again.");
    }
    let mut mesh = BinaryMesh::default();
    while !reader.at_end() {
        if reader.header(true)? == MESH {
            reader.mesh(&mut mesh)?;
        }
    }
    Ok(mesh)
}

/// `OpenReader` and `ReadSkeleton` for a binary skeleton file: none when
/// the name does not end `.skeleton` or the file is not there.
pub(super) fn read_skeleton_file(files: &FileSystem, name: &[u8]) -> Result<Option<Skeleton>, ImportError> {
    if !ends_with(name, b".skeleton") || !files.exists_bytes(name) {
        return Ok(None);
    }
    let Some(bytes) = files.open_bytes(name) else {
        return fail("Failed to open skeleton file");
    };
    let stream = StreamReader::new(&bytes, false)?;
    let mut reader = Reader { stream, current_length: 0 };
    Ok(Some(reader.skeleton()?))
}

/// `OgreBinarySerializer::ImportSkeleton` for a binary mesh: an XML
/// skeleton is read with the document as its root, which always throws.
pub(super) fn import_skeleton(files: &FileSystem, name: &[u8]) -> Result<Option<Skeleton>, ImportError> {
    if name.is_empty() {
        return Ok(None);
    }
    if ends_with(name, b".skeleton.xml") {
        xml_mesh::read_skeleton_file(files, name, false)?;
        return Ok(None);
    }
    read_skeleton_file(files, name)
}

/// `SubMesh::ConvertToAssimpMesh`: a vertex per face corner, read from the
/// vertex buffers through their streams (a corner past a buffer keeps the
/// stream where it was and reads on from there).
fn convert_submesh(submesh: &mut SubMesh, shared: Option<&mut VertexData>, skeleton: Option<&Skeleton>) -> Result<Mesh, ImportError> {
    if submesh.operation != OPERATION_TRIANGLE_LIST {
        return fail(format!("Only mesh operation type OT_TRIANGLE_LIST is supported. Found {}", submesh.operation));
    }
    let data = if submesh.uses_shared { shared } else { submesh.vertex_data.as_mut() };
    let Some(data) = data else {
        return undefined("a submesh without vertex data");
    };
    let Some(positions) = data.element(SEMANTIC_POSITION, 0) else {
        return fail("Failed to import Ogre VertexElement::VES_POSITION. Mesh does not have vertex positions!");
    };
    if positions.kind != TYPE_FLOAT3 {
        return fail("Ogre Mesh position vertex element type != VertexElement::VET_FLOAT3. This is not supported.");
    }
    let normals = data.element(SEMANTIC_NORMAL, 0);
    if normals.is_some_and(|element| element.kind != TYPE_FLOAT3) {
        return fail("Ogre Mesh normal vertex element type != VertexElement::VET_FLOAT3. This is not supported.");
    }
    // The texture coordinate sets of a usable type with a buffer.
    let uv_sets: Vec<Option<VertexElement>> = (0..2)
        .map(|index| {
            let element = data.element(SEMANTIC_TEXTURE_COORDINATES, index)?;
            let has_buffer = data.buffers.contains_key(&element.source);
            let usable = matches!(element.kind, TYPE_FLOAT2 | TYPE_FLOAT3);
            (has_buffer && usable).then_some(element)
        })
        .collect();
    let normals = normals.filter(|element| data.buffers.contains_key(&element.source));

    let (mut mesh, vertex_count) = triangle_mesh(submesh.face_count)?;
    let mut vertices = vec![[0.0f32; 3]; vertex_count as usize];
    let mut normal_values = normals.map(|_| vec![[0.0f32; 3]; vertex_count as usize]);
    let mut uv_values: Vec<Option<Vec<[f32; 3]>>> = uv_sets.iter().map(|set| set.map(|_| vec![[0.0f32; 3]; vertex_count as usize])).collect();

    let weighs_bones = skeleton.is_some() && !data.bones.list.is_empty();
    let by_vertex = if weighs_bones { data.bones.by_vertex() } else { Default::default() };
    let index_size = if submesh.is_32bit { 4 } else { 2 };
    for face in 0..submesh.face_count as u64 {
        let Some(indices_stream) = submesh.indices.as_mut() else {
            return undefined("the indices of a submesh without an index buffer");
        };
        indices_stream.seek(face * index_size as u64 * 3);
        let mut indices = [0u32; 3];
        if submesh.is_32bit {
            let mut bytes = [0u8; 12];
            if indices_stream.read(&mut bytes, 4, 3) < 3 {
                return undefined("face indices left unset past their buffer");
            }
            for (slot, chunk) in indices.iter_mut().zip(bytes.as_chunks::<4>().0) {
                *slot = u32::from_le_bytes(*chunk);
            }
        } else {
            // One 16-bit index after another, a failed read keeping the
            // last.
            let mut last = [0u8; 2];
            for slot in &mut indices {
                indices_stream.read(&mut last, 2, 1);
                *slot = u16::from_le_bytes(last) as u32;
            }
        }
        for (corner, &source_index) in indices.iter().enumerate() {
            let target = (face * 3) as usize + corner;
            if weighs_bones {
                data.bones.map(&by_vertex, source_index, target as u32);
            }
            let vertex_index = source_index as u64;
            let mut read_element = |element: VertexElement, size: usize, out: &mut [f32; 3]| -> Result<(), ImportError> {
                let width = data.vertex_size(element.source) as u64;
                let Some(stream) = data.buffers.get_mut(&element.source) else {
                    return undefined("vertex data read from a buffer that is not there");
                };
                stream.seek(width * vertex_index + element.offset as u64);
                let mut bytes = [0u8; 12];
                if stream.read(&mut bytes[..size], size, 1) == 1 {
                    let values = floats(&bytes[..size]);
                    out[..size / 4].copy_from_slice(&values[..size / 4]);
                }
                Ok(())
            };
            read_element(positions, 12, &mut vertices[target])?;
            if let (Some(element), Some(values)) = (normals, normal_values.as_mut()) {
                read_element(element, 12, &mut values[target])?;
            }
            for (set, element) in uv_sets.iter().enumerate() {
                let (Some(element), Some(values)) = (element, uv_values[set].as_mut()) else {
                    continue;
                };
                let size = type_size(element.kind) as usize;
                read_element(*element, size, &mut values[target])?;
                values[target][1] = -values[target][1] + 1.0;
            }
        }
    }

    mesh.vertices = Some(vertices);
    mesh.normals = normal_values;
    for (set, values) in uv_values.into_iter().enumerate() {
        if let (Some(values), Some(element)) = (values, uv_sets[set]) {
            mesh.uv_components[set] = if element.kind == TYPE_FLOAT2 { 2 } else { 3 };
            mesh.texture_coords[set] = Some(values);
        }
    }
    if let Some(skeleton) = skeleton
        && weighs_bones
    {
        mesh.bones = data.bones.mesh_bones(vertex_count, skeleton)?;
    }
    Ok(mesh)
}

/// `Mesh::ConvertToAssimpScene`'s meshes: the submeshes in order.
pub(super) fn convert(mesh: &mut BinaryMesh, skeleton: Option<&Skeleton>) -> Result<Vec<Mesh>, ImportError> {
    let mut meshes = Vec::with_capacity(mesh.submeshes.len());
    for submesh in &mut mesh.submeshes {
        meshes.push(convert_submesh(submesh, mesh.shared.as_mut(), skeleton)?);
    }
    Ok(meshes)
}
