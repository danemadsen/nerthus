//! Ogre meshes (`OgreImporter`): binary `.mesh` files (serializer 1.8) and
//! XML `.mesh.xml` files. Each submesh has vertex data of its own or uses
//! the mesh's shared vertex data; a skeleton file (binary or XML) gives
//! the meshes bones; material files are read for how they can fail (they
//! change no mesh).

mod binary;
mod material;
mod xml_mesh;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::batch::{Graph, LoadedScene};
use crate::io::FileSystem;
use crate::scene::{ImportError, Mesh, PRIMITIVE_TRIANGLE, check_allocation, fail};
use crate::vector::Vector3;

/// `VertexElement::Semantic`s the conversion uses.
const SEMANTIC_POSITION: u16 = 1;
const SEMANTIC_NORMAL: u16 = 4;
const SEMANTIC_TEXTURE_COORDINATES: u16 = 7;

/// `VertexElement::Type`s the conversion takes.
const TYPE_FLOAT2: u16 = 1;
const TYPE_FLOAT3: u16 = 2;

/// `ISubMesh::OT_POINT_LIST`, a submesh's operation unless set, and
/// `OT_TRIANGLE_LIST`, the only one converted.
const OPERATION_POINT_LIST: u16 = 1;
const OPERATION_TRIANGLE_LIST: u16 = 4;

/// `VertexAnimationTrack::VAT_TRANSFORM`.
const TRACK_TRANSFORM: u16 = 3;

/// `sizeof(aiFace)` and `sizeof(aiVector3D)`.
const FACE_SIZE: u64 = 16;
const VECTOR_SIZE: u64 = 12;

/// Bone hierarchies deeper than this stand for the native recursion that
/// would overflow the stack.
const MAX_BONE_DEPTH: usize = 2048;

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// `EndsWith(s, suffix, false)`: the end of `s`, ignoring ASCII case.
fn ends_with(text: &[u8], suffix: &[u8]) -> bool {
    if text.is_empty() || suffix.is_empty() || text.len() < suffix.len() {
        return false;
    }
    text[text.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

/// `VertexElement::TypeSize`.
fn type_size(kind: u16) -> u32 {
    match kind {
        0 | 4 | 9 | 10 | 11 | 20 | 24 => 4,
        1 | 12 | 21 | 25 => 8,
        2 | 22 | 26 => 12,
        3 | 13 | 23 | 27 => 16,
        14 => 24,
        15 => 32,
        5 | 16 => 2,
        6 | 17 => 4,
        7 | 18 => 6,
        8 | 19 => 8,
        _ => 0,
    }
}

/// A bone (`Ogre::Bone`), as far as the import's failures depend on it.
pub(super) struct Bone {
    id: u16,
    name: Vec<u8>,
    /// The bone that adopted it (an index into the skeleton's bones).
    parent: Option<usize>,
    children: Vec<u16>,
}

impl Bone {
    fn new(id: u16, name: Vec<u8>) -> Self {
        Bone { id, name, parent: None, children: Vec::new() }
    }
}

/// A skeleton animation track: its type and the bone it moves.
pub(super) struct Track {
    kind: u16,
    bone: Vec<u8>,
}

/// `Ogre::Skeleton`: bones and the tracks of each animation.
#[derive(Default)]
pub(super) struct Skeleton {
    bones: Vec<Bone>,
    animations: Vec<Vec<Track>>,
}

impl Skeleton {
    /// `BoneById`.
    fn bone_by_id(&self, id: u16) -> Option<usize> {
        self.bones.iter().position(|bone| bone.id == id)
    }

    /// `BoneByName`.
    fn bone_by_name(&self, name: &[u8]) -> Option<usize> {
        self.bones.iter().position(|bone| bone.name == name)
    }

    /// `Bone::AddChild`: a bone takes a parent once.
    fn add_child(&mut self, parent: usize, child: usize) -> Result<(), ImportError> {
        if self.bones[child].parent.is_some() {
            let name = String::from_utf8_lossy(&self.bones[child].name).into_owned();
            return fail(format!("Attaching child Bone that is already parented: {name}"));
        }
        self.bones[child].parent = Some(parent);
        let child_id = self.bones[child].id;
        self.bones[parent].children.push(child_id);
        Ok(())
    }

    /// A walk over the bones below `bone` (`CalculateWorldMatrixAndDefaultPose`
    /// or `ConvertToAssimpNode`): every child id must name a bone.
    fn walk(&self, bone: usize, depth: usize, what: &str) -> Result<(), ImportError> {
        if depth > MAX_BONE_DEPTH {
            return undefined("a bone hierarchy too deep for the stack");
        }
        for &child_id in &self.bones[bone].children {
            let Some(child) = self.bone_by_id(child_id) else {
                return fail(format!("{what}: Failed to find child bone {child_id} for parent {}", self.bones[bone].id));
            };
            self.walk(child, depth + 1, what)?;
        }
        Ok(())
    }

    /// The same walk from each unparented bone.
    fn walk_from_roots(&self, what: &str) -> Result<(), ImportError> {
        for (index, bone) in self.bones.iter().enumerate() {
            if bone.parent.is_none() {
                self.walk(index, 0, what)?;
            }
        }
        Ok(())
    }

    /// `ConvertToAssimpScene`'s skeleton part: the bone nodes, then the
    /// animations, whose tracks must move a bone by name.
    fn check_conversion(&self) -> Result<(), ImportError> {
        if !self.bones.is_empty() {
            self.walk_from_roots("ConvertToAssimpNode")?;
        }
        for tracks in &self.animations {
            for track in tracks {
                if track.bone.is_empty() || track.kind != TRACK_TRANSFORM {
                    return fail("VertexAnimationTrack::ConvertToAssimpAnimationNode: Cannot convert track that has no target bone name or is not type of VAT_TRANSFORM");
                }
                if self.bone_by_name(&track.bone).is_none() {
                    let name = String::from_utf8_lossy(&track.bone).into_owned();
                    return fail(format!("VertexAnimationTrack::ConvertToAssimpAnimationNode: Failed to find bone {name} from parent Skeleton"));
                }
            }
        }
        Ok(())
    }
}

/// `IVertexData`'s bone assignments as the mesh bones use them: the
/// assignments (vertex, bone), and the bones each converted vertex took
/// (`AddVertexMapping`, adding up over the submeshes that share the data).
#[derive(Default)]
pub(super) struct BoneAssignments {
    list: Vec<(u32, u16)>,
    mapped: HashMap<u32, Vec<u16>>,
}

impl BoneAssignments {
    /// The bones assigned to each source vertex, in assignment order.
    fn by_vertex(&self) -> HashMap<u32, Vec<u16>> {
        let mut by_vertex: HashMap<u32, Vec<u16>> = HashMap::new();
        for &(vertex, bone) in &self.list {
            by_vertex.entry(vertex).or_default().push(bone);
        }
        by_vertex
    }

    /// `AddVertexMapping(old, new)`.
    fn map(&mut self, by_vertex: &HashMap<u32, Vec<u16>>, old: u32, new: u32) {
        let taken = self.mapped.entry(new).or_default();
        if let Some(bones) = by_vertex.get(&old) {
            taken.extend_from_slice(bones);
        }
    }

    /// The bones of a converted mesh (`AssimpBoneWeights` and
    /// `ReferencedBonesByWeights`): each assigned bone, in id order, with
    /// the vertices it weighs.
    fn mesh_bones(&self, vertex_count: u32, skeleton: &Skeleton) -> Result<Vec<Vec<u32>>, ImportError> {
        let mut weights: BTreeMap<u16, Vec<u32>> = BTreeMap::new();
        for vertex in 0..vertex_count {
            if let Some(bones) = self.mapped.get(&vertex) {
                for &bone in bones {
                    weights.entry(bone).or_default().push(vertex);
                }
            }
        }
        let referenced: BTreeSet<u16> = self.list.iter().map(|&(_, bone)| bone).collect();
        let mut bones = Vec::new();
        for bone_index in referenced {
            let Some(bone) = skeleton.bone_by_id(bone_index) else {
                return undefined("a weighted bone missing from the skeleton");
            };
            let id = skeleton.bones[bone].id;
            bones.push(weights.get(&id).cloned().unwrap_or_default());
        }
        Ok(bones)
    }
}

/// A converted mesh's frame: faces of three new vertices each.
fn triangle_mesh(face_count: u32) -> Result<(Mesh, u32), ImportError> {
    check_allocation(face_count as u64, FACE_SIZE)?;
    // `mNumFaces * 3` in unsigned ints.
    let vertex_count = face_count.wrapping_mul(3);
    check_allocation(vertex_count as u64, VECTOR_SIZE)?;
    let faces = (0..face_count).map(|face| vec![face * 3, face * 3 + 1, face * 3 + 2]).collect();
    let mesh = Mesh { primitive_types: PRIMITIVE_TRIANGLE, vertex_count: vertex_count as usize, faces: Some(faces), ..Mesh::default() };
    Ok((mesh, vertex_count))
}

/// The scene: the converted submeshes, in order (the bone nodes and
/// animations converted after them only for how they fail).
fn finish(meshes: Vec<Mesh>, skeleton: Option<&Skeleton>) -> Result<LoadedScene, ImportError> {
    let mut animations = 0;
    if let Some(skeleton) = skeleton {
        skeleton.check_conversion()?;
        animations = skeleton.animations.len();
    }
    Ok(LoadedScene { meshes, graph: Graph::Unknown, animations: Some(animations) })
}

/// `OgreImporter::InternReadFile`: binary for a path ending `.mesh`, XML
/// otherwise; then the skeleton, the materials, and the conversion.
pub(crate) fn read_graph(path: &str, bytes: &[u8], files: &FileSystem) -> Result<LoadedScene, ImportError> {
    if ends_with(path.as_bytes(), b".mesh") {
        let mut mesh = binary::read_mesh(bytes)?;
        let skeleton = binary::import_skeleton(files, &mesh.skeleton_ref)?;
        let materials: Vec<&[u8]> = mesh.submeshes.iter().map(|submesh| submesh.material.as_slice()).collect();
        material::read_materials(path, files, &materials)?;
        let meshes = binary::convert(&mut mesh, skeleton.as_ref())?;
        return finish(meshes, skeleton.as_ref());
    }
    let mut mesh = xml_mesh::read_mesh(bytes)?;
    let skeleton = xml_mesh::import_skeleton(files, &mut mesh.skeleton_ref)?;
    // `GetSubMesh` takes a 16-bit index: past 65536 submeshes, it finds an
    // earlier one again.
    let submesh_count = mesh.submeshes.len();
    let materials: Vec<&[u8]> = (0..submesh_count).map(|index| mesh.submeshes[index as u16 as usize].material.as_slice()).collect();
    material::read_materials(path, files, &materials)?;
    let meshes = xml_mesh::convert(&mut mesh, skeleton.as_ref())?;
    finish(meshes, skeleton.as_ref())
}

/// A vertex component read as little-endian floats.
fn floats(bytes: &[u8]) -> Vector3 {
    let mut vector = [0.0f32; 3];
    for (component, chunk) in vector.iter_mut().zip(bytes.as_chunks::<4>().0) {
        *component = f32::from_le_bytes(*chunk);
    }
    vector
}
