//! Assimp's 3DS importer (`3DSLoader.cpp`, `3DSConverter.cpp`, with the
//! smoothing-group normals of `SmoothingGroups.inl` over `SGSpatialSort`).
//! Chunks nest by size, each level read with the stream's limit at its
//! chunk's end (a chunk running past its parent leaves the next read past
//! the limit, which throws). Every mesh's faces have their indices
//! clamped (to the vertices, then to the texture coordinates), are made
//! unique and get normals averaged over vertices near each other that
//! share a smoothing group. Faces without a known material get the last
//! grey, untextured "default" material, or one made for them. Each mesh is
//! split by material; a mesh a keyframer node names is moved back into its
//! local space (the inverse of its first namesake's matrix; normals by the
//! plain transpose; x mirrored for a negative determinant; less the node's
//! pivot).
//!
//! Failures here where Assimp does undefined things: a node hierarchy
//! deeper than the stack allows.

use crate::matrix::Matrix4;
use crate::scene::{self, ImportError, Mesh, PRIMITIVE_TRIANGLE, Scene, fail};
use crate::smoothing_groups;
use crate::stream_reader::StreamReader;
use crate::vector::{Vector3, sub};

/// `sizeof(Discreet3DS::Chunk)`.
const CHUNK_HEADER: u64 = 6;
/// How deep a node hierarchy can go before Assimp's recursion through it
/// would overflow the stack.
const MAX_DEPTH: usize = 4096;
/// A face whose material isn't set (`NotSet`).
const NOT_SET: u32 = 0xcdcd_cdcd;
/// `sizeof(aiVectorKey)`, `sizeof(aiQuatKey)` and `sizeof(aiFloatKey)`,
/// for the key counts reserved.
const VECTOR_KEY_SIZE: u64 = 24;
const QUAT_KEY_SIZE: u64 = 24;
const FLOAT_KEY_SIZE: u64 = 16;

const CHUNK_VERSION: u16 = 0x0002;
const CHUNK_RGBF: u16 = 0x0010;
const CHUNK_RGBB: u16 = 0x0011;
const CHUNK_LINRGBB: u16 = 0x0012;
const CHUNK_LINRGBF: u16 = 0x0013;
const CHUNK_PERCENTW: u16 = 0x0030;
const CHUNK_PERCENTF: u16 = 0x0031;
const CHUNK_PERCENTD: u16 = 0x0032;
const CHUNK_PRJ: u16 = 0xC23D;
const CHUNK_MAIN: u16 = 0x4D4D;
const CHUNK_OBJMESH: u16 = 0x3D3D;
const CHUNK_AMBCOLOR: u16 = 0x2100;
const CHUNK_BIT_MAP: u16 = 0x1100;
const CHUNK_OBJBLOCK: u16 = 0x4000;
const CHUNK_TRIMESH: u16 = 0x4100;
const CHUNK_VERTLIST: u16 = 0x4110;
const CHUNK_FACELIST: u16 = 0x4120;
const CHUNK_FACEMAT: u16 = 0x4130;
const CHUNK_MAPLIST: u16 = 0x4140;
const CHUNK_SMOOLIST: u16 = 0x4150;
const CHUNK_TRMATRIX: u16 = 0x4160;
const CHUNK_LIGHT: u16 = 0x4600;
const CHUNK_CAMERA: u16 = 0x4700;
const CHUNK_MASTER_SCALE: u16 = 0x0100;
const CHUNK_MAT_MATERIAL: u16 = 0xAFFF;
const CHUNK_MAT_MATNAME: u16 = 0xA000;
const CHUNK_MAT_AMBIENT: u16 = 0xA010;
const CHUNK_MAT_DIFFUSE: u16 = 0xA020;
const CHUNK_MAT_SPECULAR: u16 = 0xA030;
const CHUNK_MAT_SHININESS: u16 = 0xA040;
const CHUNK_MAT_SHININESS_PERCENT: u16 = 0xA041;
const CHUNK_MAT_SHADING: u16 = 0xA100;
const CHUNK_MAT_SELF_ILLUM: u16 = 0xA080;
const CHUNK_MAT_SELF_ILPCT: u16 = 0xA084;
const CHUNK_MAT_TRANSPARENCY: u16 = 0xA050;
const CHUNK_MAT_TEXTURE: u16 = 0xA200;
const CHUNK_MAT_OPACMAP: u16 = 0xA210;
const CHUNK_MAT_REFLMAP: u16 = 0xA220;
const CHUNK_MAT_SELFIMAP: u16 = 0xA33D;
const CHUNK_MAT_BUMPMAP: u16 = 0xA230;
const CHUNK_MAT_SPECMAP: u16 = 0xA204;
const CHUNK_MAT_MAT_SHINMAP: u16 = 0xA33C;
const CHUNK_MAT_MAP_USCALE: u16 = 0xA354;
const CHUNK_MAT_MAP_VSCALE: u16 = 0xA356;
const CHUNK_MAT_MAP_UOFFSET: u16 = 0xA358;
const CHUNK_MAT_MAP_VOFFSET: u16 = 0xA35A;
const CHUNK_MAT_MAP_ANG: u16 = 0xA35C;
const CHUNK_MAT_MAP_TILING: u16 = 0xA351;
const CHUNK_MAPFILE: u16 = 0xA300;
const CHUNK_MAT_TWO_SIDE: u16 = 0xA081;
const CHUNK_KEYFRAMER: u16 = 0xB000;
const CHUNK_TRACKINFO: u16 = 0xB002;
const CHUNK_TRACKOBJNAME: u16 = 0xB010;
const CHUNK_TRACKDUMMYOBJNAME: u16 = 0xB011;
const CHUNK_TRACKPIVOT: u16 = 0xB013;
const CHUNK_TRACKPOS: u16 = 0xB020;
const CHUNK_TRACKROTATE: u16 = 0xB021;
const CHUNK_TRACKSCALE: u16 = 0xB022;
const CHUNK_TRACKCAMERA: u16 = 0xB003;
const CHUNK_TRACKFOV: u16 = 0xB023;
const CHUNK_TRACKROLL: u16 = 0xB024;
const CHUNK_TRACKCAMTGT: u16 = 0xB004;
const CHUNK_TRACKLIGHT: u16 = 0xB005;
const CHUNK_TRACKLIGTGT: u16 = 0xB006;
const CHUNK_TRACKSPOTL: u16 = 0xB007;
const CHUNK_DL_MULTIPLIER: u16 = 0x465B;
const CHUNK_DL_ATTENUATE: u16 = 0x4625;
const CHUNK_DL_SPOTLIGHT: u16 = 0x4610;
const CHUNK_CAM_RANGES: u16 = 0x4720;

const KEY_USE_TENS: u32 = 0x1;
const KEY_USE_CONT: u32 = 0x2;
const KEY_USE_BIAS: u32 = 0x4;
const KEY_USE_EASE_TO: u32 = 0x8;
const KEY_USE_EASE_FROM: u32 = 0x10;

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// `D3DS::Face`.
#[derive(Clone, Copy, Default)]
struct Face {
    indices: [u32; 3],
    smooth_group: u32,
}

/// `D3DS::Mesh`.
struct Mesh3ds {
    name: Vec<u8>,
    positions: Vec<Vector3>,
    faces: Vec<Face>,
    normals: Vec<Vector3>,
    texture_coords: Vec<Vector3>,
    face_materials: Vec<u32>,
    matrix: Matrix4,
}

/// What `D3DS::Material` keeps that the meshes depend on: its name,
/// diffuse colour, and the map names `ContainsTextures` looks at
/// (diffuse, bump, opacity, emissive, specular, shininess).
struct Material {
    name: Vec<u8>,
    diffuse: [f32; 3],
    map_names: [Vec<u8>; 6],
}

impl Material {
    fn new(name: Vec<u8>, diffuse: [f32; 3]) -> Self {
        Self { name, diffuse, map_names: Default::default() }
    }

    /// `ContainsTextures`.
    fn has_textures(&self) -> bool {
        self.map_names.iter().any(|map_name| !map_name.is_empty())
    }
}

/// What `D3DS::Node` keeps that the meshes depend on.
struct Node {
    name: Vec<u8>,
    parent: Option<usize>,
    children: Vec<usize>,
    hierarchy_position: i16,
    instance_count: i32,
    pivot: Vector3,
}

/// A chunk opened by `begin_chunk`.
struct OpenChunk {
    flag: u16,
    size: i32,
    old_limit: u32,
}

/// `Discreet3DSImporter`'s parse state.
struct Parser<'a> {
    stream: StreamReader<'a>,
    meshes: Vec<Mesh3ds>,
    materials: Vec<Material>,
    nodes: Vec<Node>,
    current_node: usize,
    last_node_index: i16,
    is_prj: bool,
}

impl<'a> Parser<'a> {
    /// `ReadChunk`: a chunk's id and size; contents past the file throw.
    fn read_chunk(&mut self) -> Result<(u16, u32), ImportError> {
        let flag = self.stream.get_u2()?;
        let size = self.stream.get_u4()?;
        let content = (size as u64).wrapping_sub(CHUNK_HEADER);
        if content > self.stream.remaining_size() as u64 {
            return fail("Chunk is too large");
        }
        Ok((flag, size))
    }

    /// `ASSIMP_3DS_BEGIN_CHUNK`: the next chunk with contents, the read
    /// limit set to its end; none when less than a header remains.
    fn begin_chunk(&mut self) -> Result<Option<OpenChunk>, ImportError> {
        loop {
            if (self.stream.remaining_size_to_limit() as u64) < CHUNK_HEADER {
                return Ok(None);
            }
            let (flag, size) = self.read_chunk()?;
            let chunk_size = (size as u64).wrapping_sub(CHUNK_HEADER) as i32;
            if chunk_size <= 0 {
                continue;
            }
            let limit = self.stream.current_pos().wrapping_add(chunk_size) as u32;
            let old_limit = self.stream.set_read_limit(limit)?;
            return Ok(Some(OpenChunk { flag, size: chunk_size, old_limit }));
        }
    }

    /// `ASSIMP_3DS_END_CHUNK`: on to the chunk's end with the outer limit
    /// back; true when nothing remains to it.
    fn end_chunk(&mut self, chunk: &OpenChunk) -> Result<bool, ImportError> {
        self.stream.skip_to_read_limit();
        self.stream.set_read_limit(chunk.old_limit)?;
        Ok(self.stream.remaining_size_to_limit() == 0)
    }

    /// A string read to its NUL (`while (stream->GetI1())`).
    fn read_name(&mut self) -> Result<Vec<u8>, ImportError> {
        let mut name = Vec::new();
        loop {
            let byte = self.stream.get_u1()?;
            if byte == 0 {
                return Ok(name);
            }
            name.push(byte);
        }
    }

    fn skip_floats(&mut self, count: usize) -> Result<(), ImportError> {
        for _ in 0..count {
            self.stream.get_f4()?;
        }
        Ok(())
    }

    /// `ParseMainChunk`.
    fn parse_main_chunk(&mut self) -> Result<(), ImportError> {
        while let Some(chunk) = self.begin_chunk()? {
            match chunk.flag {
                CHUNK_PRJ => self.is_prj = true,
                CHUNK_MAIN => self.parse_editor_chunk()?,
                _ => {}
            }
            if self.end_chunk(&chunk)? {
                return Ok(());
            }
        }
        Ok(())
    }

    /// `ParseEditorChunk`.
    fn parse_editor_chunk(&mut self) -> Result<(), ImportError> {
        while let Some(chunk) = self.begin_chunk()? {
            match chunk.flag {
                CHUNK_OBJMESH => self.parse_object_chunk()?,
                CHUNK_KEYFRAMER => self.parse_keyframe_chunk()?,
                CHUNK_VERSION => {
                    self.stream.get_i2()?;
                }
                _ => {}
            }
            if self.end_chunk(&chunk)? {
                return Ok(());
            }
        }
        Ok(())
    }

    /// `ParseObjectChunk`.
    fn parse_object_chunk(&mut self) -> Result<(), ImportError> {
        while let Some(chunk) = self.begin_chunk()? {
            match chunk.flag {
                CHUNK_OBJBLOCK => {
                    let name = self.read_name()?;
                    self.parse_chunk(&name)?;
                }
                CHUNK_MAT_MATERIAL => {
                    let name = format!("UNNAMED_{}", self.materials.len()).into_bytes();
                    self.materials.push(Material::new(name, [0.6, 0.6, 0.6]));
                    self.parse_material_chunk()?;
                }
                CHUNK_AMBCOLOR => {
                    self.parse_color_chunk(true)?;
                }
                CHUNK_BIT_MAP => {
                    self.read_name()?;
                }
                CHUNK_MASTER_SCALE => {
                    self.stream.get_f4()?;
                }
                _ => {}
            }
            if self.end_chunk(&chunk)? {
                return Ok(());
            }
        }
        Ok(())
    }

    /// `ParseChunk`: an object's mesh, light or camera.
    fn parse_chunk(&mut self, name: &[u8]) -> Result<(), ImportError> {
        while let Some(chunk) = self.begin_chunk()? {
            match chunk.flag {
                CHUNK_TRIMESH => {
                    let mesh = Mesh3ds {
                        name: name.to_vec(),
                        positions: Vec::new(),
                        faces: Vec::new(),
                        normals: Vec::new(),
                        texture_coords: Vec::new(),
                        face_materials: Vec::new(),
                        matrix: Matrix4::IDENTITY,
                    };
                    self.meshes.push(mesh);
                    self.parse_mesh_chunk()?;
                }
                CHUNK_LIGHT => {
                    self.skip_floats(3)?;
                    if !self.is_prj {
                        self.parse_light_chunk()?;
                    }
                }
                CHUNK_CAMERA => {
                    self.skip_floats(8)?;
                    if !self.is_prj {
                        self.parse_camera_chunk()?;
                    }
                }
                _ => {}
            }
            if self.end_chunk(&chunk)? {
                return Ok(());
            }
        }
        Ok(())
    }

    /// `ParseLightChunk`.
    fn parse_light_chunk(&mut self) -> Result<(), ImportError> {
        while let Some(chunk) = self.begin_chunk()? {
            match chunk.flag {
                CHUNK_DL_SPOTLIGHT => self.skip_floats(5)?,
                CHUNK_DL_MULTIPLIER | CHUNK_DL_ATTENUATE => self.skip_floats(1)?,
                CHUNK_RGBF | CHUNK_LINRGBF => self.skip_floats(3)?,
                _ => {}
            }
            if self.end_chunk(&chunk)? {
                return Ok(());
            }
        }
        Ok(())
    }

    /// `ParseCameraChunk`.
    fn parse_camera_chunk(&mut self) -> Result<(), ImportError> {
        while let Some(chunk) = self.begin_chunk()? {
            if chunk.flag == CHUNK_CAM_RANGES {
                self.skip_floats(2)?;
            }
            if self.end_chunk(&chunk)? {
                return Ok(());
            }
        }
        Ok(())
    }

    /// `ParseKeyframeChunk`.
    fn parse_keyframe_chunk(&mut self) -> Result<(), ImportError> {
        while let Some(chunk) = self.begin_chunk()? {
            let track = matches!(
                chunk.flag,
                CHUNK_TRACKCAMTGT | CHUNK_TRACKSPOTL | CHUNK_TRACKCAMERA | CHUNK_TRACKINFO | CHUNK_TRACKLIGHT | CHUNK_TRACKLIGTGT
            );
            if track {
                self.parse_hierarchy_chunk(chunk.flag)?;
            }
            if self.end_chunk(&chunk)? {
                return Ok(());
            }
        }
        Ok(())
    }

    /// `FindNode`: the first node of that name, depth first from the root.
    fn find_node(&self, name: &[u8]) -> Option<usize> {
        let mut pending = vec![0usize];
        while let Some(node) = pending.pop() {
            if self.nodes[node].name == name {
                return Some(node);
            }
            for &child in self.nodes[node].children.iter().rev() {
                pending.push(child);
            }
        }
        None
    }

    /// `Node::push_back`.
    fn adopt(&mut self, parent: usize, child: usize) {
        self.nodes[parent].children.push(child);
        self.nodes[child].parent = Some(parent);
    }

    /// `InverseNodeSearch`: up from `current` to a node at the new node's
    /// hierarchy position, whose parent (or itself, at the top) takes it;
    /// the root when there is none.
    fn inverse_node_search(&mut self, node: usize, current: usize) {
        let mut candidate = Some(current);
        while let Some(index) = candidate {
            if self.nodes[index].hierarchy_position == self.nodes[node].hierarchy_position {
                let parent = self.nodes[index].parent.unwrap_or(index);
                self.adopt(parent, node);
                return;
            }
            candidate = self.nodes[index].parent;
        }
        self.adopt(0, node);
    }

    /// `SkipTCBInfo`.
    fn skip_tcb_info(&mut self) -> Result<(), ImportError> {
        let flags = self.stream.get_i2()? as i32 as u32;
        let masks = [KEY_USE_TENS, KEY_USE_BIAS, KEY_USE_CONT, KEY_USE_EASE_FROM, KEY_USE_EASE_TO];
        for mask in masks {
            if flags & mask != 0 {
                self.stream.inc_ptr(4)?;
            }
        }
        Ok(())
    }

    /// A key track: its count (reserved for), then keys of a frame number,
    /// TCB info and `values` floats.
    fn skip_keys(&mut self, count: u32, key_size: u64, values: usize) -> Result<(), ImportError> {
        scene::check_allocation(count as u64, key_size)?;
        for _ in 0..count {
            self.stream.get_u4()?;
            self.skip_tcb_info()?;
            self.skip_floats(values)?;
        }
        Ok(())
    }

    /// `ParseHierarchyChunk`: node names placing nodes in the hierarchy,
    /// dummy names, pivots, and key tracks (read past).
    fn parse_hierarchy_chunk(&mut self, parent: u16) -> Result<(), ImportError> {
        while let Some(chunk) = self.begin_chunk()? {
            match chunk.flag {
                CHUNK_TRACKOBJNAME => self.parse_track_object_name(parent)?,
                CHUNK_TRACKDUMMYOBJNAME => {
                    let name = self.read_name()?;
                    let current = self.current_node;
                    if self.nodes[current].name == b"$$$DUMMY" {
                        self.nodes[current].name = name;
                    }
                }
                CHUNK_TRACKPIVOT => {
                    if parent == CHUNK_TRACKINFO {
                        let x = self.stream.get_f4()?;
                        let y = self.stream.get_f4()?;
                        let z = self.stream.get_f4()?;
                        let current = self.current_node;
                        self.nodes[current].pivot = [x, y, z];
                    }
                }
                CHUNK_TRACKPOS => {
                    self.stream.inc_ptr(10)?;
                    let count = self.stream.get_i4()? as u32;
                    self.skip_keys(count, VECTOR_KEY_SIZE, 3)?;
                }
                CHUNK_TRACKROLL => {
                    if parent == CHUNK_TRACKCAMERA {
                        self.stream.inc_ptr(10)?;
                        let count = self.stream.get_i4()? as u32;
                        self.skip_keys(count, FLOAT_KEY_SIZE, 1)?;
                    }
                }
                CHUNK_TRACKFOV => {}
                CHUNK_TRACKROTATE => {
                    self.stream.inc_ptr(10)?;
                    let count = self.stream.get_i4()? as u32;
                    self.skip_keys(count, QUAT_KEY_SIZE, 4)?;
                }
                CHUNK_TRACKSCALE => {
                    self.stream.inc_ptr(10)?;
                    let count = self.stream.get_i2()? as i32 as u32;
                    self.stream.inc_ptr(2)?;
                    self.skip_keys(count, VECTOR_KEY_SIZE, 3)?;
                }
                _ => {}
            }
            if self.end_chunk(&chunk)? {
                return Ok(());
            }
        }
        Ok(())
    }

    /// The `CHUNK_TRACKOBJNAME` case: a node already named that becomes
    /// the current one (an object instance under TRACKINFO gets a new node
    /// numbered after it); a new node goes beside the current one at its
    /// hierarchy position, under it at a deeper one, or back up the tree.
    fn parse_track_object_name(&mut self, parent: u16) -> Result<(), ImportError> {
        let name = self.read_name()?;
        if let Some(found) = self.find_node(&name) {
            if parent != CHUNK_TRACKINFO {
                self.current_node = found;
                return Ok(());
            }
            self.nodes[found].instance_count += 1;
        }
        let node = self.nodes.len();
        self.nodes.push(Node { name, parent: None, children: Vec::new(), hierarchy_position: 0, instance_count: 1, pivot: [0.0; 3] });
        self.stream.inc_ptr(4)?;
        let hierarchy = (self.stream.get_i2()? as u16).wrapping_add(1);
        self.nodes[node].hierarchy_position = hierarchy as i16;

        let current = self.current_node;
        if self.nodes[current].hierarchy_position as i32 == hierarchy as i32 {
            let Some(current_parent) = self.nodes[current].parent else {
                return undefined("a node beside the root");
            };
            self.adopt(current_parent, node);
            self.last_node_index = self.last_node_index.wrapping_add(1);
        } else if hierarchy as i32 >= self.last_node_index as i32 {
            self.adopt(current, node);
            self.last_node_index = hierarchy as i16;
        } else {
            self.inverse_node_search(node, current);
            self.last_node_index = self.last_node_index.wrapping_add(1);
        }
        self.current_node = node;
        Ok(())
    }

    /// `ParseFaceChunk`: smoothing groups and material assignments after
    /// a face list.
    fn parse_face_chunk(&mut self) -> Result<(), ImportError> {
        while let Some(chunk) = self.begin_chunk()? {
            match chunk.flag {
                CHUNK_SMOOLIST => {
                    let count = (chunk.size as u32 / 4) as usize;
                    let Some(mesh) = self.meshes.last() else {
                        return undefined("a face chunk without a mesh");
                    };
                    if count > mesh.faces.len() {
                        return fail("3DS: More smoothing groups than faces");
                    }
                    for index in 0..count {
                        let group = self.stream.get_u4()?;
                        if let Some(mesh) = self.meshes.last_mut() {
                            mesh.faces[index].smooth_group = group;
                        }
                    }
                }
                CHUNK_FACEMAT => {
                    let name = self.read_name()?;
                    let found = self.materials.iter().position(|material| !material.name.is_empty() && material.name.eq_ignore_ascii_case(&name));
                    let material = found.map_or(NOT_SET, |index| index as u32);
                    let count = self.stream.get_u2()?;
                    for _ in 0..count {
                        let face = self.stream.get_u2()? as usize;
                        if let Some(mesh) = self.meshes.last_mut()
                            && let Some(slot) = mesh.face_materials.get_mut(face)
                        {
                            *slot = material;
                        }
                    }
                }
                _ => {}
            }
            if self.end_chunk(&chunk)? {
                return Ok(());
            }
        }
        Ok(())
    }

    /// `ParseMeshChunk`: vertices, the mesh matrix, texture coordinates and
    /// faces (with their face chunks inside).
    fn parse_mesh_chunk(&mut self) -> Result<(), ImportError> {
        while let Some(chunk) = self.begin_chunk()? {
            match chunk.flag {
                CHUNK_VERTLIST => {
                    let count = self.stream.get_u2()?;
                    for _ in 0..count {
                        let x = self.stream.get_f4()?;
                        let y = self.stream.get_f4()?;
                        let z = self.stream.get_f4()?;
                        self.last_mesh()?.positions.push([x, y, z]);
                    }
                }
                CHUNK_TRMATRIX => {
                    let mut values = [0.0f32; 12];
                    for value in &mut values {
                        *value = self.stream.get_f4()?;
                    }
                    // a1 b1 c1 a2 b2 c2 a3 b3 c3 a4 b4 c4
                    let matrix = &mut self.last_mesh()?.matrix.0;
                    for (column, chunk) in values.chunks(3).enumerate() {
                        for (row, &value) in chunk.iter().enumerate() {
                            matrix[row * 4 + column] = value;
                        }
                    }
                }
                CHUNK_MAPLIST => {
                    let count = self.stream.get_u2()?;
                    for _ in 0..count {
                        let u = self.stream.get_f4()?;
                        let v = self.stream.get_f4()?;
                        self.last_mesh()?.texture_coords.push([u, v, 0.0]);
                    }
                }
                CHUNK_FACELIST => {
                    let count = self.stream.get_u2()?;
                    for _ in 0..count {
                        let first = self.stream.get_u2()? as u32;
                        let second = self.stream.get_u2()? as u32;
                        let third = self.stream.get_u2()? as u32;
                        self.stream.inc_ptr(2)?;
                        let face = Face { indices: [first, second, third], smooth_group: 0 };
                        self.last_mesh()?.faces.push(face);
                    }
                    let mesh = self.last_mesh()?;
                    let face_count = mesh.faces.len();
                    mesh.face_materials.resize(face_count, NOT_SET);
                    let remaining = self.stream.remaining_size_to_limit() as u32 as i32;
                    if remaining > CHUNK_HEADER as i32 {
                        self.parse_face_chunk()?;
                    }
                }
                _ => {}
            }
            if self.end_chunk(&chunk)? {
                return Ok(());
            }
        }
        Ok(())
    }

    fn last_mesh(&mut self) -> Result<&mut Mesh3ds, ImportError> {
        match self.meshes.last_mut() {
            Some(mesh) => Ok(mesh),
            None => undefined("a mesh chunk without a mesh"),
        }
    }

    fn last_material(&mut self) -> Result<&mut Material, ImportError> {
        match self.materials.last_mut() {
            Some(material) => Ok(material),
            None => undefined("a material chunk without a material"),
        }
    }

    /// `ParseMaterialChunk`.
    fn parse_material_chunk(&mut self) -> Result<(), ImportError> {
        while let Some(chunk) = self.begin_chunk()? {
            match chunk.flag {
                CHUNK_MAT_MATNAME => {
                    let name = self.read_name()?;
                    if !name.is_empty() {
                        self.last_material()?.name = name;
                    }
                }
                CHUNK_MAT_DIFFUSE => {
                    let mut color = self.parse_color_chunk(false)?;
                    if color[0].is_nan() {
                        color = [1.0, 1.0, 1.0];
                    }
                    self.last_material()?.diffuse = color;
                }
                CHUNK_MAT_SPECULAR | CHUNK_MAT_AMBIENT | CHUNK_MAT_SELF_ILLUM => {
                    self.parse_color_chunk(false)?;
                }
                CHUNK_MAT_TRANSPARENCY | CHUNK_MAT_SHININESS | CHUNK_MAT_SHININESS_PERCENT | CHUNK_MAT_SELF_ILPCT => {
                    self.parse_percentage_chunk()?;
                }
                CHUNK_MAT_SHADING => {
                    self.stream.get_i2()?;
                }
                CHUNK_MAT_TEXTURE | CHUNK_MAT_BUMPMAP | CHUNK_MAT_OPACMAP | CHUNK_MAT_SELFIMAP | CHUNK_MAT_SPECMAP | CHUNK_MAT_MAT_SHINMAP => {
                    let slot = match chunk.flag {
                        CHUNK_MAT_TEXTURE => 0,
                        CHUNK_MAT_BUMPMAP => 1,
                        CHUNK_MAT_OPACMAP => 2,
                        CHUNK_MAT_SELFIMAP => 3,
                        CHUNK_MAT_SPECMAP => 4,
                        _ => 5,
                    };
                    if let Some(map_name) = self.parse_texture_chunk()? {
                        self.last_material()?.map_names[slot] = map_name;
                    }
                }
                CHUNK_MAT_REFLMAP => {
                    self.parse_texture_chunk()?;
                }
                CHUNK_MAT_TWO_SIDE => {}
                _ => {}
            }
            if self.end_chunk(&chunk)? {
                return Ok(());
            }
        }
        Ok(())
    }

    /// `ParseTextureChunk`: the map's file name, if one is given (the last
    /// one counts), and its settings, read past.
    fn parse_texture_chunk(&mut self) -> Result<Option<Vec<u8>>, ImportError> {
        let mut map_name = None;
        while let Some(chunk) = self.begin_chunk()? {
            match chunk.flag {
                CHUNK_MAPFILE => map_name = Some(self.read_name()?),
                CHUNK_PERCENTD => {
                    self.stream.get_f8()?;
                }
                CHUNK_PERCENTF | CHUNK_MAT_MAP_USCALE | CHUNK_MAT_MAP_VSCALE | CHUNK_MAT_MAP_UOFFSET | CHUNK_MAT_MAP_VOFFSET | CHUNK_MAT_MAP_ANG => {
                    self.stream.get_f4()?;
                }
                CHUNK_PERCENTW | CHUNK_MAT_MAP_TILING => {
                    self.stream.get_i2()?;
                }
                _ => {}
            }
            if self.end_chunk(&chunk)? {
                break;
            }
        }
        Ok(map_name)
    }

    /// `ParsePercentageChunk`: a chunk header and its value, if it is a
    /// percentage.
    fn parse_percentage_chunk(&mut self) -> Result<(), ImportError> {
        let (flag, _) = self.read_chunk()?;
        if flag == CHUNK_PERCENTF {
            self.stream.get_f4()?;
        } else if flag == CHUNK_PERCENTW {
            self.stream.get_i2()?;
        }
        Ok(())
    }

    /// `ParseColorChunk`: the first colour (or, when accepted, percentage)
    /// chunk, others skipped; NaN when it is too small.
    fn parse_color_chunk(&mut self, accept_percent: bool) -> Result<[f32; 3], ImportError> {
        let error = [f32::NAN; 3];
        loop {
            let (flag, size) = self.read_chunk()?;
            let difference = size.wrapping_sub(CHUNK_HEADER as u32);
            match flag {
                CHUNK_LINRGBF | CHUNK_RGBF => {
                    if 12 > difference {
                        return Ok(error);
                    }
                    let r = self.stream.get_f4()?;
                    let g = self.stream.get_f4()?;
                    let b = self.stream.get_f4()?;
                    return Ok([r, g, b]);
                }
                CHUNK_LINRGBB | CHUNK_RGBB => {
                    if 3 > difference {
                        return Ok(error);
                    }
                    let inverse = 1.0f32 / 255.0f32;
                    let r = self.stream.get_u1()? as f32 * inverse;
                    let g = self.stream.get_u1()? as f32 * inverse;
                    let b = self.stream.get_u1()? as f32 * inverse;
                    return Ok([r, g, b]);
                }
                CHUNK_PERCENTF => {
                    if accept_percent && 4 <= difference {
                        let value = self.stream.get_f4()?;
                        return Ok([value; 3]);
                    }
                    return Ok(error);
                }
                CHUNK_PERCENTW => {
                    if accept_percent && 1 <= difference {
                        let value = self.stream.get_u1()? as f32 / 255.0;
                        return Ok([value; 3]);
                    }
                    return Ok(error);
                }
                _ => self.stream.inc_ptr(difference as i64)?,
            }
        }
    }
}

/// `CheckIndices`: indices past the vertices (then past the texture
/// coordinates, when there are any) clamped to the last one.
fn check_indices(mesh: &mut Mesh3ds) {
    let position_count = mesh.positions.len() as u32;
    let coord_count = mesh.texture_coords.len() as u32;
    for face in &mut mesh.faces {
        for index in &mut face.indices {
            if *index >= position_count {
                *index = position_count.wrapping_sub(1);
            }
            if coord_count != 0 && *index >= coord_count {
                *index = coord_count - 1;
            }
        }
    }
}

/// `MakeUnique`: three vertices (and texture coordinates) per face.
fn make_unique(mesh: &mut Mesh3ds) {
    let mut positions = Vec::with_capacity(mesh.faces.len() * 3);
    let mut coords = Vec::new();
    let has_coords = !mesh.texture_coords.is_empty();
    for face in &mut mesh.faces {
        for index in &mut face.indices {
            positions.push(mesh.positions[*index as usize]);
            if has_coords {
                coords.push(mesh.texture_coords[*index as usize]);
            }
            *index = (positions.len() - 1) as u32;
        }
    }
    mesh.positions = positions;
    mesh.texture_coords = coords;
}

/// `ComputeNormalsWithSmoothingsGroups` on a mesh.
fn compute_normals(mesh: &mut Mesh3ds) {
    let faces: Vec<([u32; 3], u32)> = mesh.faces.iter().map(|face| (face.indices, face.smooth_group)).collect();
    mesh.normals = smoothing_groups::compute_normals(&mesh.positions, &faces);
}

/// `ReplaceDefaultMaterial`: faces without a known material get the last
/// material named like "default" that is grey and untextured, or a new
/// one.
fn replace_default_material(meshes: &mut [Mesh3ds], materials: &mut Vec<Material>) {
    let material_count = materials.len() as u32;
    let mut default = NOT_SET;
    for (index, material) in materials.iter().enumerate() {
        let lowered = material.name.to_ascii_lowercase();
        let named_default = lowered.windows(7).any(|window| window == b"default");
        if !named_default {
            continue;
        }
        let [r, g, b] = material.diffuse;
        if r != g || r != b {
            continue;
        }
        if material.has_textures() {
            continue;
        }
        default = index as u32;
    }
    if default == NOT_SET {
        default = material_count;
    }

    let mut replaced = 0usize;
    for mesh in meshes.iter_mut() {
        for slot in &mut mesh.face_materials {
            if *slot == NOT_SET || *slot >= material_count {
                *slot = default;
                replaced += 1;
            }
        }
    }
    if replaced != 0 && default == material_count {
        materials.push(Material::new(b"%%%DEFAULT".to_vec(), [0.3, 0.3, 0.3]));
    }
}

/// An output mesh and the 3DS mesh it came from.
struct OutputMesh {
    mesh: Mesh,
    source: usize,
    moved: bool,
}

/// `ConvertMeshes`: each mesh split by material, in material order.
fn convert_meshes(meshes: &[Mesh3ds], material_count: usize) -> Result<Vec<OutputMesh>, ImportError> {
    let mut output = Vec::new();
    let mut face_total = 0usize;
    for (source, mesh) in meshes.iter().enumerate() {
        let mut split: Vec<Vec<usize>> = vec![Vec::new(); material_count];
        for (face, &material) in mesh.face_materials.iter().enumerate() {
            let Some(list) = split.get_mut(material as usize) else {
                return undefined("a material index past the materials");
            };
            list.push(face);
        }
        for faces in &split {
            if faces.is_empty() {
                continue;
            }
            face_total += faces.len();
            let mut vertices = Vec::with_capacity(faces.len() * 3);
            let mut normals = Vec::with_capacity(faces.len() * 3);
            let mut coords = Vec::new();
            let mut face_lists = Vec::with_capacity(faces.len());
            for &face in faces {
                let first = vertices.len() as u32;
                for &index in &mesh.faces[face].indices {
                    vertices.push(mesh.positions[index as usize]);
                    normals.push(mesh.normals[index as usize]);
                    if !mesh.texture_coords.is_empty() {
                        coords.push(mesh.texture_coords[index as usize]);
                    }
                }
                face_lists.push(vec![first, first + 1, first + 2]);
            }
            let mut out = Mesh {
                primitive_types: PRIMITIVE_TRIANGLE,
                vertex_count: vertices.len(),
                vertices: Some(vertices),
                normals: Some(normals),
                faces: Some(face_lists),
                ..Mesh::default()
            };
            if !mesh.texture_coords.is_empty() {
                out.texture_coords[0] = Some(coords);
            }
            output.push(OutputMesh { mesh: out, source, moved: false });
        }
    }
    if face_total == 0 {
        return fail("No faces loaded. The mesh is empty");
    }
    Ok(output)
}

/// `AddNodeToGraph`'s mesh part over the node tree (depth first): the
/// meshes named like a node, not yet moved, go back into their local
/// space.
fn move_meshes_to_nodes(nodes: &[Node], meshes: &[Mesh3ds], output: &mut [OutputMesh]) -> Result<(), ImportError> {
    let mut pending = vec![(0usize, 0usize)];
    while let Some((node, depth)) = pending.pop() {
        if depth > MAX_DEPTH {
            return undefined("a node hierarchy too deep for the stack");
        }
        let name = &nodes[node].name;
        let matching: Vec<usize> = (0..output.len()).filter(|&index| meshes[output[index].source].name == *name).collect();
        if let Some(&first) = matching.first() {
            let matrix = meshes[output[first].source].matrix;
            let inverse = matrix.inverse();
            let transposed = matrix.transpose();
            let mirrored = matrix.determinant() < 0.0;
            let pivot = nodes[node].pivot;
            let has_pivot = pivot[0] != 0.0 || pivot[1] != 0.0 || pivot[2] != 0.0;
            for &index in &matching {
                let out = &mut output[index];
                if out.moved {
                    continue;
                }
                out.moved = true;
                let (Some(vertices), Some(normals)) = (out.mesh.vertices.as_mut(), out.mesh.normals.as_mut()) else {
                    continue;
                };
                for (vertex, normal) in vertices.iter_mut().zip(normals.iter_mut()) {
                    *vertex = inverse.transform(*vertex);
                    *normal = transposed.transform(*normal);
                }
                if mirrored {
                    for (vertex, normal) in vertices.iter_mut().zip(normals.iter_mut()) {
                        vertex[0] *= -1.0;
                        normal[0] *= -1.0;
                    }
                }
                if has_pivot {
                    for vertex in vertices.iter_mut() {
                        *vertex = sub(*vertex, pivot);
                    }
                }
            }
        }
        for &child in nodes[node].children.iter().rev() {
            pending.push((child, depth + 1));
        }
    }
    Ok(())
}

pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    let stream = StreamReader::new(bytes, false)?;
    if stream.remaining_size() < 16 {
        return fail("3DS file is either empty or corrupt");
    }
    let root = Node { name: b"UNNAMED".to_vec(), parent: None, children: Vec::new(), hierarchy_position: -1, instance_count: 1, pivot: [0.0; 3] };
    let mut parser = Parser { stream, meshes: Vec::new(), materials: Vec::new(), nodes: vec![root], current_node: 0, last_node_index: -1, is_prj: false };
    parser.parse_main_chunk()?;

    for mesh in &mut parser.meshes {
        if !mesh.faces.is_empty() && mesh.positions.is_empty() {
            return fail("3DS file contains faces but no vertices");
        }
        check_indices(mesh);
        make_unique(mesh);
        compute_normals(mesh);
    }
    replace_default_material(&mut parser.meshes, &mut parser.materials);
    let mut output = convert_meshes(&parser.meshes, parser.materials.len())?;
    if !parser.nodes[0].children.is_empty() {
        move_meshes_to_nodes(&parser.nodes, &parser.meshes, &mut output)?;
    }
    let meshes = output.into_iter().map(|out| out.mesh).collect();
    Ok(Scene { meshes })
}
