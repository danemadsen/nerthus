//! Assimp's ASE importer (`ASEParser.cpp`, `ASELoader.cpp`): 3ds max ASCII
//! scene exports. The parser walks `*TOKEN` lines section by section,
//! counting braces (an unknown token's first character is passed over,
//! so a brace right after its `*` doesn't count at the top level); the
//! mesh normals in the file are read but replaced by smoothing-group
//! normals. Each mesh is made unique (three vertices per face), split by
//! its material's sub-materials when it has any, and moved back into its
//! local space (the inverse of its node matrix; normals by the transposed
//! 3×3) every time its node turns up in the node graph, which nodes join
//! by their parent's name. A scene left without meshes gets the skeleton
//! mesh of that graph.
//!
//! Kept as Assimp reads: a face's smoothing groups set only bit 0 (any
//! group below 32; one starting with '9' isn't read); a mapping channel
//! numbered 1 is skipped and then parsed anyway; texture coordinate
//! channels after the first missing one stay zero; a node name over 1023
//! bytes is cut short in the graph, or (for a node without a known parent)
//! empty.
//!
//! Failures here where Assimp does undefined things: face indices past
//! the vertices, texture coordinates or colours, or never set (texture
//! and colour indices are uninitialised until a TFACE/CFACE sets them);
//! normals read for faces added after them; bone vertices past their
//! list; weights naming bones past the list; a target camera or light
//! with two or more children (its child list is shifted over itself and
//! the nodes are deleted twice); nesting deeper than the stack allows.

use crate::fast_atof::{fast_atoreal_move, strtoul10};
use crate::matrix::Matrix4;
use crate::node::Node;
use crate::parsing::{at, is_line_end, is_space_or_newline, skip_line, skip_spaces, skip_spaces_and_line_end, starts_with_ignore_case_at, text_file_to_buffer, token_match};
use crate::scene::{self, ImportError, Mesh, PRIMITIVE_TRIANGLE, Scene, fail};
use crate::skeleton;
use crate::smoothing_groups;
use crate::vector::Vector3;

/// `AI_MAX_NUMBER_OF_TEXTURECOORDS`.
const MAX_TEXTURE_COORDS: usize = 8;
/// `ASE::Face::DEFAULT_MATINDEX`.
const DEFAULT_MATERIAL: u32 = 0xFFFF_FFFF;
/// `AI_ASE_NEW_FILE_FORMAT` and `AI_ASE_OLD_FILE_FORMAT`.
const NEW_FILE_FORMAT: u32 = 200;
const OLD_FILE_FORMAT: u32 = 110;
/// `AI_MAXLEN`: an `aiString` holds at most one byte less.
const MAX_LENGTH: usize = 1024;
/// How deep groups, sub-materials and the node graph can nest before
/// Assimp's recursion through them would overflow the stack.
const MAX_DEPTH: usize = 4096;
/// The Windows build's element sizes, for the allocations counts ask for:
/// `ASE::Material`, `ASE::Face`, `aiVector3D`, `aiColor4D`, `ASE::Bone`,
/// `ASE::BoneVertex` and a bone weight.
const MATERIAL_SIZE: u64 = 672;
const FACE_SIZE: u64 = 132;
const VECTOR_SIZE: u64 = 12;
const COLOR_SIZE: u64 = 16;
const BONE_SIZE: u64 = 32;
const BONE_VERTEX_SIZE: u64 = 24;
const WEIGHT_SIZE: u64 = 8;

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// What an `ASE::Material` keeps that the meshes depend on.
#[derive(Clone, Default)]
struct Material {
    sub_materials: Vec<Material>,
}

/// `ASE::Face`, its texture and colour indices unset until a TFACE/CFACE
/// line sets them.
#[derive(Clone)]
struct Face {
    indices: [u32; 3],
    smooth_group: u32,
    uv_indices: [[Option<u32>; 3]; MAX_TEXTURE_COORDS],
    color_indices: [Option<u32>; 3],
    material: u32,
    face_index: u32,
}

impl Default for Face {
    fn default() -> Self {
        Self {
            indices: [0; 3],
            smooth_group: 0,
            uv_indices: [[None; 3]; MAX_TEXTURE_COORDS],
            color_indices: [None; 3],
            material: DEFAULT_MATERIAL,
            face_index: 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum NodeKind {
    Light,
    Camera,
    Mesh,
    Dummy,
}

/// `ASE::BaseNode` with the light and camera types and, for meshes, the
/// mesh data.
struct Object {
    kind: NodeKind,
    name: Vec<u8>,
    parent: Vec<u8>,
    transform: Matrix4,
    target_position: Vector3,
    target_light: bool,
    target_camera: bool,
    processed: bool,
    mesh: MeshData,
}

/// What `ASE::Mesh` adds.
struct MeshData {
    positions: Vec<Vector3>,
    faces: Vec<Face>,
    normal_count: usize,
    texture_coords: [Vec<Vector3>; MAX_TEXTURE_COORDS],
    uv_components: [u32; MAX_TEXTURE_COORDS],
    vertex_colors: Vec<[f32; 4]>,
    material_index: u32,
    bones: Vec<Vec<u8>>,
    /// Each bone vertex's weights' bone indices.
    bone_vertices: Vec<Vec<u32>>,
}

impl Object {
    fn new(kind: NodeKind, name: &[u8]) -> Self {
        let mesh = MeshData {
            positions: Vec::new(),
            faces: Vec::new(),
            normal_count: 0,
            texture_coords: Default::default(),
            uv_components: [2; MAX_TEXTURE_COORDS],
            vertex_colors: Vec::new(),
            material_index: DEFAULT_MATERIAL,
            bones: Vec::new(),
            bone_vertices: Vec::new(),
        };
        Self {
            kind,
            name: name.to_vec(),
            parent: Vec::new(),
            transform: Matrix4::IDENTITY,
            target_position: [f32::NAN, 0.0, 0.0],
            target_light: false,
            target_camera: false,
            processed: false,
            mesh,
        }
    }
}


/// What a section loop does after a byte: go on, or leave the section.
enum Step {
    Continue,
    Return,
}

/// `IsNumeric`.
fn is_numeric(byte: u8) -> bool {
    byte.is_ascii_digit() || byte == b'-' || byte == b'+'
}

/// `ASE::Parser`.
struct Parser {
    buffer: Vec<u8>,
    position: usize,
    end: usize,
    file_format: u32,
    materials: Vec<Material>,
    meshes: Vec<Object>,
    dummies: Vec<Object>,
    lights: Vec<Object>,
    cameras: Vec<Object>,
    depth: usize,
}

impl Parser {
    fn byte(&self) -> u8 {
        at(&self.buffer, self.position)
    }

    fn token(&mut self, token: &[u8]) -> bool {
        token_match(&self.buffer, &mut self.position, token)
    }

    /// `SkipSpaces(&mFilePtr, mEnd)`.
    fn skip_spaces(&mut self) -> bool {
        let (next, more) = skip_spaces(&self.buffer, self.position, self.end);
        self.position = next;
        more
    }

    fn skip_spaces_and_line_end(&mut self) {
        self.position = skip_spaces_and_line_end(&self.buffer, self.position, self.end).0;
    }

    fn enter(&mut self) -> Result<(), ImportError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return undefined("nesting too deep for the stack");
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    /// `SkipToNextToken`: on to the next `*`, `{` or `}`; false at the end.
    fn skip_to_next_token(&mut self) -> Result<bool, ImportError> {
        loop {
            if self.position >= self.buffer.len() {
                return undefined("a read past the file");
            }
            let byte = self.byte();
            if byte == b'*' || byte == b'}' || byte == b'{' {
                return Ok(true);
            }
            if byte == 0 {
                return Ok(false);
            }
            self.position += 1;
        }
    }

    /// `SkipSection`: past the section's closing brace.
    fn skip_section(&mut self) -> Result<(), ImportError> {
        let mut count = 0i32;
        loop {
            match self.byte() {
                b'}' => {
                    count -= 1;
                    if count == 0 {
                        self.position += 1;
                        self.skip_to_next_token()?;
                        return Ok(());
                    }
                }
                b'{' => count += 1,
                0 => return Ok(()),
                _ => {}
            }
            self.position += 1;
        }
    }

    /// `AI_ASE_HANDLE_TOP_LEVEL_SECTION`: braces count unless the byte
    /// follows a `*` that matched nothing; the end of the file ends it.
    fn top_level_step(&mut self, depth: &mut i32, after_star: bool) -> Result<Step, ImportError> {
        if !after_star {
            match self.byte() {
                b'{' => *depth += 1,
                b'}' => {
                    *depth -= 1;
                    if *depth == 0 {
                        self.position += 1;
                        self.skip_to_next_token()?;
                        return Ok(Step::Return);
                    }
                }
                _ => {}
            }
        }
        if self.byte() == 0 {
            return Ok(Step::Return);
        }
        self.position += 1;
        Ok(Step::Continue)
    }

    /// `AI_ASE_HANDLE_SECTION`: the end of the file is an error.
    fn section_step(&mut self, depth: &mut i32, what: &str) -> Result<Step, ImportError> {
        match self.byte() {
            b'{' => *depth += 1,
            b'}' => {
                *depth -= 1;
                if *depth == 0 {
                    self.position += 1;
                    self.skip_to_next_token()?;
                    return Ok(Step::Return);
                }
            }
            0 => return fail(format!("Encountered unexpected EOL while parsing a {what} chunk")),
            _ => {}
        }
        self.position += 1;
        Ok(Step::Continue)
    }

    /// `ParseLV4MeshLong`: 0 at a line end.
    fn parse_long(&mut self) -> u32 {
        if !self.skip_spaces() {
            return 0;
        }
        let (value, after) = strtoul10(&self.buffer, self.position);
        self.position = after;
        value
    }

    /// `ParseLV4MeshReal`/`ParseLV4MeshFloat`: 0 at a line end; text that
    /// isn't a number throws.
    fn parse_real(&mut self) -> Result<f32, ImportError> {
        if !self.skip_spaces() {
            return Ok(0.0);
        }
        let (value, after) = fast_atoreal_move(&self.buffer, self.position, true)?;
        self.position = after;
        Ok(value)
    }

    fn parse_real_triple(&mut self) -> Result<Vector3, ImportError> {
        let x = self.parse_real()?;
        let y = self.parse_real()?;
        let z = self.parse_real()?;
        Ok([x, y, z])
    }

    fn parse_long_triple(&mut self) -> [u32; 3] {
        let first = self.parse_long();
        let second = self.parse_long();
        let third = self.parse_long();
        [first, second, third]
    }

    /// `ParseString`: the text between double quotes; none at a line end,
    /// without a quote, or without the closing one before the end.
    fn parse_string(&mut self) -> Option<Vec<u8>> {
        if !self.skip_spaces() {
            return None;
        }
        if self.byte() != b'"' {
            return None;
        }
        self.position += 1;
        let start = self.position;
        let mut end = start;
        loop {
            match at(&self.buffer, end) {
                b'"' => break,
                0 => return None,
                _ => end += 1,
            }
        }
        let text = self.buffer[start..end].to_vec();
        self.position = end + 1;
        Some(text)
    }

    /// `if (!ParseString(out, …)) SkipToNextToken();`
    fn parse_string_into(&mut self, out: &mut Vec<u8>) -> Result<(), ImportError> {
        match self.parse_string() {
            Some(text) => *out = text,
            None => {
                self.skip_to_next_token()?;
            }
        }
        Ok(())
    }

    /// `Parse`: the top level (and a `*GROUP`'s contents).
    fn parse(&mut self) -> Result<(), ImportError> {
        self.enter()?;
        let mut depth = 0;
        loop {
            let mut after_star = false;
            if self.byte() == b'*' {
                self.position += 1;
                after_star = true;
                if self.token(b"3DSMAX_ASCIIEXPORT") {
                    let format = self.parse_long();
                    if format != 0 {
                        self.file_format = format;
                    }
                    continue;
                }
                if self.token(b"SCENE") {
                    self.parse_scene_block()?;
                    continue;
                }
                if self.token(b"GROUP") {
                    self.parse()?;
                    continue;
                }
                if self.token(b"MATERIAL_LIST") {
                    self.parse_material_list_block()?;
                    continue;
                }
                if self.token(b"GEOMOBJECT") {
                    let mut mesh = Object::new(NodeKind::Mesh, b"UNNAMED");
                    self.parse_object_block(&mut mesh)?;
                    self.meshes.push(mesh);
                    continue;
                }
                if self.token(b"HELPEROBJECT") {
                    let mut dummy = Object::new(NodeKind::Dummy, b"DUMMY");
                    self.parse_object_block(&mut dummy)?;
                    self.dummies.push(dummy);
                    continue;
                }
                if self.token(b"LIGHTOBJECT") {
                    let mut light = Object::new(NodeKind::Light, b"UNNAMED");
                    self.parse_object_block(&mut light)?;
                    self.lights.push(light);
                    continue;
                }
                if self.token(b"CAMERAOBJECT") {
                    let mut camera = Object::new(NodeKind::Camera, b"UNNAMED");
                    self.parse_object_block(&mut camera)?;
                    self.cameras.push(camera);
                    continue;
                }
                if self.token(b"COMMENT") {
                    self.parse_string();
                    continue;
                }
                if self.file_format < NEW_FILE_FORMAT && self.token(b"MESH_SOFTSKINVERTS") {
                    self.parse_soft_skin_block()?;
                }
            }
            if let Step::Return = self.top_level_step(&mut depth, after_star)? {
                self.leave();
                return Ok(());
            }
        }
    }

    /// `ParseLV1SoftSkinBlock` (old format files): per mesh name, its bone
    /// vertices, their weights naming bones (added as they appear).
    fn parse_soft_skin_block(&mut self) -> Result<(), ImportError> {
        loop {
            match self.byte() {
                b'}' => {
                    self.position += 1;
                    return Ok(());
                }
                0 => return Ok(()),
                b'{' => self.position += 1,
                _ => {
                    let start = self.position;
                    while !is_space_or_newline(self.byte()) {
                        self.position += 1;
                    }
                    if self.position != start {
                        let name = self.buffer[start..self.position].to_vec();
                        let found = self.meshes.iter().position(|mesh| mesh.name == name);
                        match found {
                            None => loop {
                                self.skip_spaces_and_line_end();
                                if self.byte() == b'}' {
                                    self.position += 1;
                                    return Ok(());
                                }
                                if !is_numeric(self.byte()) {
                                    break;
                                }
                                self.position = skip_line(&self.buffer, self.position, self.end).0;
                            },
                            Some(mesh_index) => self.parse_soft_skin_mesh(mesh_index)?,
                        }
                    }
                }
            }
            if self.byte() == 0 {
                return Ok(());
            }
            self.position += 1;
            self.skip_spaces_and_line_end();
        }
    }

    /// One mesh's part of the soft skin block.
    fn parse_soft_skin_mesh(&mut self, mesh_index: usize) -> Result<(), ImportError> {
        self.skip_spaces_and_line_end();
        let vertex_count = self.parse_long();
        scene::check_allocation(vertex_count as u64, BONE_VERTEX_SIZE)?;
        for _ in 0..vertex_count {
            self.skip_spaces_and_line_end();
            let weight_count = self.parse_long();
            scene::check_allocation(weight_count as u64, WEIGHT_SIZE)?;
            let mut weights = Vec::new();
            for _ in 0..weight_count {
                let bone = self.parse_string().unwrap_or_default();
                let mesh = &mut self.meshes[mesh_index].mesh;
                let index = match mesh.bones.iter().position(|name| *name == bone) {
                    Some(index) => index,
                    None => {
                        mesh.bones.push(bone);
                        mesh.bones.len() - 1
                    }
                };
                self.parse_real()?;
                weights.push(index as u32);
            }
            self.meshes[mesh_index].mesh.bone_vertices.push(weights);
        }
        Ok(())
    }

    /// `ParseLV1SceneBlock`.
    fn parse_scene_block(&mut self) -> Result<(), ImportError> {
        let mut depth = 0;
        loop {
            let mut after_star = false;
            if self.byte() == b'*' {
                self.position += 1;
                after_star = true;
                if self.token(b"SCENE_BACKGROUND_STATIC") || self.token(b"SCENE_AMBIENT_STATIC") {
                    self.parse_real_triple()?;
                    continue;
                }
                let long_tokens: [&[u8]; 4] = [b"SCENE_FIRSTFRAME", b"SCENE_LASTFRAME", b"SCENE_FRAMESPEED", b"SCENE_TICKSPERFRAME"];
                if long_tokens.iter().any(|token| self.token(token)) {
                    self.parse_long();
                    continue;
                }
            }
            if let Step::Return = self.top_level_step(&mut depth, after_star)? {
                return Ok(());
            }
        }
    }

    /// `ParseLV1MaterialListBlock`: the count grows the list; materials by
    /// index (past the count: the last); an unknown token at the first
    /// level ends the list there.
    fn parse_material_list_block(&mut self) -> Result<(), ImportError> {
        let mut depth = 0;
        let mut count = 0u32;
        let old_count = self.materials.len() as u32;
        loop {
            let mut after_star = false;
            if self.byte() == b'*' {
                self.position += 1;
                after_star = true;
                if self.token(b"MATERIAL_COUNT") {
                    count = self.parse_long();
                    if u32::MAX - old_count < count {
                        return Ok(());
                    }
                    self.resize_materials(old_count as u64 + count as u64)?;
                    continue;
                }
                if self.token(b"MATERIAL") {
                    if count == 0 {
                        count = 1;
                        self.resize_materials(old_count as u64 + 1)?;
                    }
                    let mut index = self.parse_long();
                    if index >= count {
                        index = count - 1;
                    }
                    let slot = (index + old_count) as usize;
                    let mut material = std::mem::take(&mut self.materials[slot]);
                    let result = self.parse_material_block(&mut material);
                    self.materials[slot] = material;
                    result?;
                    continue;
                }
                if depth == 1 {
                    self.position -= 1;
                    return Ok(());
                }
            }
            if let Step::Return = self.top_level_step(&mut depth, after_star)? {
                return Ok(());
            }
        }
    }

    /// `m_vMaterials.resize(count, Material("INVALID"))`.
    fn resize_materials(&mut self, count: u64) -> Result<(), ImportError> {
        if count > self.materials.len() as u64 {
            scene::check_allocation(count, MATERIAL_SIZE)?;
        }
        self.materials.resize(count as usize, Material::default());
        Ok(())
    }

    /// `ParseLV2MaterialBlock`: only the sub-materials matter here.
    fn parse_material_block(&mut self, material: &mut Material) -> Result<(), ImportError> {
        self.enter()?;
        let mut depth = 0;
        let mut sub_count = 0u32;
        loop {
            if self.byte() == b'*' {
                self.position += 1;
                if self.token(b"MATERIAL_NAME") {
                    let mut name = Vec::new();
                    self.parse_string_into(&mut name)?;
                    continue;
                }
                let triples: [&[u8]; 3] = [b"MATERIAL_AMBIENT", b"MATERIAL_DIFFUSE", b"MATERIAL_SPECULAR"];
                if triples.iter().any(|token| self.token(token)) {
                    self.parse_real_triple()?;
                    continue;
                }
                if self.token(b"MATERIAL_SHADING") {
                    let known: [&[u8]; 4] = [b"Blinn", b"Phong", b"Flat", b"Wire"];
                    if !known.iter().any(|token| self.token(token)) {
                        self.skip_to_next_token()?;
                    }
                    continue;
                }
                let reals: [&[u8]; 3] = [b"MATERIAL_TRANSPARENCY", b"MATERIAL_SELFILLUM", b"MATERIAL_SHINE"];
                if reals.iter().any(|token| self.token(token)) {
                    self.parse_real()?;
                    continue;
                }
                if self.token(b"MATERIAL_TWOSIDED") {
                    continue;
                }
                if self.token(b"MATERIAL_SHINESTRENGTH") {
                    self.parse_real()?;
                    continue;
                }
                let maps: [&[u8]; 5] = [b"MAP_DIFFUSE", b"MAP_AMBIENT", b"MAP_SPECULAR", b"MAP_OPACITY", b"MAP_SELFILLUM"];
                if maps.iter().any(|token| self.token(token)) {
                    self.parse_map_block()?;
                    continue;
                }
                // `*MAP_BUMP` has no `continue`: the tokens below are tried
                // after its block.
                if self.token(b"MAP_BUMP") {
                    self.parse_map_block()?;
                }
                if self.token(b"MAP_SHINESTRENGTH") {
                    self.parse_map_block()?;
                    continue;
                }
                if self.token(b"NUMSUBMTLS") {
                    sub_count = self.parse_long();
                    self.resize_sub_materials(material, sub_count)?;
                }
                if self.token(b"SUBMATERIAL") {
                    if sub_count == 0 {
                        sub_count = 1;
                        self.resize_sub_materials(material, 1)?;
                    }
                    let mut index = self.parse_long();
                    if index >= sub_count {
                        index = sub_count - 1;
                    }
                    if (index as usize) < material.sub_materials.len() {
                        let mut sub = std::mem::take(&mut material.sub_materials[index as usize]);
                        let result = self.parse_material_block(&mut sub);
                        material.sub_materials[index as usize] = sub;
                        result?;
                    }
                    continue;
                }
            }
            if let Step::Return = self.section_step(&mut depth, "*MATERIAL")? {
                self.leave();
                return Ok(());
            }
        }
    }

    fn resize_sub_materials(&mut self, material: &mut Material, count: u32) -> Result<(), ImportError> {
        if count as usize > material.sub_materials.len() {
            scene::check_allocation(count as u64, MATERIAL_SIZE)?;
        }
        material.sub_materials.resize(count as usize, Material::default());
        Ok(())
    }

    /// `ParseLV3MapBlock`: read past.
    fn parse_map_block(&mut self) -> Result<(), ImportError> {
        let mut depth = 0;
        let mut parse_path = true;
        loop {
            if self.byte() == b'*' {
                self.position += 1;
                if self.token(b"MAP_CLASS") {
                    let mut class = Vec::new();
                    self.parse_string_into(&mut class)?;
                    if class != b"Bitmap" && class != b"Normal Bump" {
                        parse_path = false;
                    }
                    continue;
                }
                if parse_path && self.token(b"BITMAP") {
                    let mut name = Vec::new();
                    self.parse_string_into(&mut name)?;
                    continue;
                }
                let reals: [&[u8]; 6] = [b"UVW_U_OFFSET", b"UVW_V_OFFSET", b"UVW_U_TILING", b"UVW_V_TILING", b"UVW_ANGLE", b"MAP_AMOUNT"];
                if reals.iter().any(|token| self.token(token)) {
                    self.parse_real()?;
                    continue;
                }
            }
            if let Step::Return = self.section_step(&mut depth, "*MAP_XXXXXX")? {
                return Ok(());
            }
        }
    }

    /// `ParseLV1ObjectBlock`.
    fn parse_object_block(&mut self, node: &mut Object) -> Result<(), ImportError> {
        let mut depth = 0;
        loop {
            let mut after_star = false;
            if self.byte() == b'*' {
                self.position += 1;
                after_star = true;
                if self.token(b"NODE_NAME") {
                    self.parse_string_into(&mut node.name)?;
                    continue;
                }
                if self.token(b"NODE_PARENT") {
                    self.parse_string_into(&mut node.parent)?;
                    continue;
                }
                if self.token(b"NODE_TM") {
                    self.parse_node_transform_block(node)?;
                    continue;
                }
                if self.token(b"TM_ANIMATION") {
                    self.parse_animation_block(node)?;
                    continue;
                }
                match node.kind {
                    NodeKind::Light => {
                        if self.token(b"LIGHT_SETTINGS") {
                            self.parse_settings_block(&[b"LIGHT_COLOR"], &[b"LIGHT_INTENS", b"LIGHT_HOTSPOT", b"LIGHT_FALLOFF"], "LIGHT_SETTINGS")?;
                            continue;
                        }
                        if self.token(b"LIGHT_TYPE") {
                            let types: [&[u8]; 4] = [b"omni", b"target", b"free", b"directional"];
                            let found = types.iter().position(|kind| starts_with_ignore_case_at(&self.buffer, self.position, kind));
                            if let Some(kind) = found {
                                node.target_light = kind == 1;
                            }
                            continue;
                        }
                    }
                    NodeKind::Camera => {
                        if self.token(b"CAMERA_SETTINGS") {
                            self.parse_settings_block(&[], &[b"CAMERA_NEAR", b"CAMERA_FAR", b"CAMERA_FOV"], "CAMERA_SETTINGS")?;
                            continue;
                        } else if self.token(b"CAMERA_TYPE") {
                            if starts_with_ignore_case_at(&self.buffer, self.position, b"target") {
                                node.target_camera = true;
                            } else if starts_with_ignore_case_at(&self.buffer, self.position, b"free") {
                                node.target_camera = false;
                            }
                            continue;
                        }
                    }
                    NodeKind::Mesh => {
                        if self.token(b"MESH") || self.token(b"MESH_SOFTSKIN") {
                            self.parse_mesh_block(&mut node.mesh)?;
                            continue;
                        }
                        if self.token(b"MATERIAL_REF") {
                            node.mesh.material_index = self.parse_long();
                            continue;
                        }
                    }
                    NodeKind::Dummy => {}
                }
            }
            if let Step::Return = self.top_level_step(&mut depth, after_star)? {
                return Ok(());
            }
        }
    }

    /// `ParseLV2LightSettingsBlock` and `ParseLV2CameraSettingsBlock`:
    /// read past.
    fn parse_settings_block(&mut self, triples: &[&[u8]], reals: &[&[u8]], what: &str) -> Result<(), ImportError> {
        let mut depth = 0;
        loop {
            if self.byte() == b'*' {
                self.position += 1;
                if triples.iter().any(|token| self.token(token)) {
                    self.parse_real_triple()?;
                    continue;
                }
                if reals.iter().any(|token| self.token(token)) {
                    self.parse_real()?;
                    continue;
                }
            }
            if let Step::Return = self.section_step(&mut depth, what)? {
                return Ok(());
            }
        }
    }

    /// `ParseLV2AnimationBlock`: tracks read past (a target channel's
    /// scaling and rotation tracks, and every track of a target channel on
    /// another node, skipped).
    fn parse_animation_block(&mut self, node: &Object) -> Result<(), ImportError> {
        // None: the channel is ignored; Some(true): the target channel.
        let mut channel: Option<bool> = Some(false);
        let mut depth = 0;
        loop {
            if self.byte() == b'*' {
                self.position += 1;
                if self.token(b"NODE_NAME") {
                    let mut name = Vec::new();
                    self.parse_string_into(&mut name)?;
                    if name.windows(7).any(|window| window == b".Target") {
                        let target_node = (node.kind == NodeKind::Camera && node.target_camera) || (node.kind == NodeKind::Light && node.target_light);
                        channel = if target_node { Some(true) } else { None };
                    }
                    continue;
                }
                let positions: [&[u8]; 3] = [b"CONTROL_POS_TRACK", b"CONTROL_POS_BEZIER", b"CONTROL_POS_TCB"];
                if positions.iter().any(|token| self.token(token)) {
                    if channel.is_none() {
                        self.skip_section()?;
                    } else {
                        self.parse_key_block(&[b"CONTROL_POS_SAMPLE", b"CONTROL_BEZIER_POS_KEY", b"CONTROL_TCB_POS_KEY"], false, "*CONTROL_POS_TRACK")?;
                    }
                    continue;
                }
                let scalings: [&[u8]; 3] = [b"CONTROL_SCALE_TRACK", b"CONTROL_SCALE_BEZIER", b"CONTROL_SCALE_TCB"];
                if scalings.iter().any(|token| self.token(token)) {
                    if channel != Some(false) {
                        self.skip_section()?;
                    } else {
                        self.parse_key_block(&[b"CONTROL_SCALE_SAMPLE", b"CONTROL_BEZIER_SCALE_KEY", b"CONTROL_TCB_SCALE_KEY"], false, "*CONTROL_POS_TRACK")?;
                    }
                    continue;
                }
                let rotations: [&[u8]; 3] = [b"CONTROL_ROT_TRACK", b"CONTROL_ROT_BEZIER", b"CONTROL_ROT_TCB"];
                if rotations.iter().any(|token| self.token(token)) {
                    if channel != Some(false) {
                        self.skip_section()?;
                    } else {
                        self.parse_key_block(&[b"CONTROL_ROT_SAMPLE", b"CONTROL_BEZIER_ROT_KEY", b"CONTROL_TCB_ROT_KEY"], true, "*CONTROL_ROT_TRACK")?;
                    }
                    continue;
                }
            }
            if let Step::Return = self.section_step(&mut depth, "TM_ANIMATION")? {
                return Ok(());
            }
        }
    }

    /// `ParseLV3PosAnimationBlock`, `…Scale…` and `…Rot…`: each key a frame
    /// number and three values (a fourth for rotations), read past. The
    /// three key tokens are all tried in turn.
    fn parse_key_block(&mut self, keys: &[&[u8]; 3], rotation: bool, what: &str) -> Result<(), ImportError> {
        let mut depth = 0;
        loop {
            if self.byte() == b'*' {
                self.position += 1;
                let mut matched = false;
                for key in keys {
                    if self.token(key) {
                        matched = true;
                    }
                }
                if matched {
                    self.parse_long();
                    self.parse_real_triple()?;
                    if rotation {
                        self.parse_real()?;
                    }
                }
            }
            if let Step::Return = self.section_step(&mut depth, what)? {
                return Ok(());
            }
        }
    }

    /// `ParseLV2NodeTransformBlock`: the node's rows (or, for a target
    /// camera or light, its target's position) when the block names it.
    fn parse_node_transform_block(&mut self, node: &mut Object) -> Result<(), ImportError> {
        let mut depth = 0;
        let mut mode = 0;
        loop {
            if self.byte() == b'*' {
                self.position += 1;
                if self.token(b"NODE_NAME") {
                    let mut name = Vec::new();
                    self.parse_string_into(&mut name)?;
                    let target = name.windows(7).position(|window| window == b".Target");
                    if name == node.name {
                        mode = 1;
                    } else if let Some(cut) = target
                        && node.name == name[..cut]
                    {
                        let target_node = (node.kind == NodeKind::Light && node.target_light) || (node.kind == NodeKind::Camera && node.target_camera);
                        if target_node {
                            mode = 2;
                        }
                    }
                    continue;
                }
                if mode != 0 {
                    if self.token(b"TM_ROW3") {
                        let row = self.parse_real_triple()?;
                        if mode == 1 {
                            node.transform.0[12..15].copy_from_slice(&row);
                        } else {
                            node.target_position = row;
                        }
                        continue;
                    }
                    if mode == 1 {
                        let rows: [&[u8]; 3] = [b"TM_ROW0", b"TM_ROW1", b"TM_ROW2"];
                        let mut matched_row = None;
                        for (index, row) in rows.iter().enumerate() {
                            if self.token(row) {
                                matched_row = Some(index);
                                break;
                            }
                        }
                        if let Some(index) = matched_row {
                            let row = self.parse_real_triple()?;
                            node.transform.0[index * 4..index * 4 + 3].copy_from_slice(&row);
                            continue;
                        }
                        let inherits: [&[u8]; 3] = [b"INHERIT_POS", b"INHERIT_ROT", b"INHERIT_SCL"];
                        if inherits.iter().any(|token| self.token(token)) {
                            self.parse_long_triple();
                            continue;
                        }
                    }
                }
            }
            if let Step::Return = self.section_step(&mut depth, "*NODE_TM")? {
                return Ok(());
            }
        }
    }

    /// `ParseLV2MeshBlock`.
    fn parse_mesh_block(&mut self, mesh: &mut MeshData) -> Result<(), ImportError> {
        let mut depth = 0;
        let mut vertex_count = 0u32;
        let mut face_count = 0u32;
        let mut texture_vertex_count = 0u32;
        let mut texture_face_count = 0u32;
        let mut color_vertex_count = 0u32;
        let mut color_face_count = 0u32;
        loop {
            if self.byte() == b'*' {
                self.position += 1;
                if self.token(b"MESH_NUMVERTEX") {
                    vertex_count = self.parse_long();
                    continue;
                }
                if self.token(b"MESH_NUMTVERTEX") {
                    texture_vertex_count = self.parse_long();
                    continue;
                }
                if self.token(b"MESH_NUMCVERTEX") {
                    color_vertex_count = self.parse_long();
                    continue;
                }
                if self.token(b"MESH_NUMFACES") {
                    face_count = self.parse_long();
                    continue;
                }
                if self.token(b"MESH_NUMTVFACES") {
                    texture_face_count = self.parse_long();
                    continue;
                }
                if self.token(b"MESH_NUMCVFACES") {
                    color_face_count = self.parse_long();
                    continue;
                }
                if self.token(b"MESH_VERTEX_LIST") {
                    self.parse_vertex_list_block(vertex_count, mesh)?;
                    continue;
                }
                if self.token(b"MESH_FACE_LIST") {
                    self.parse_face_list_block(face_count, mesh)?;
                    continue;
                }
                if self.token(b"MESH_TVERTLIST") {
                    self.parse_texture_vertex_list_block(texture_vertex_count, mesh, 0)?;
                    continue;
                }
                if self.token(b"MESH_TFACELIST") {
                    self.parse_index_face_list_block(texture_face_count, mesh, Some(0))?;
                    continue;
                }
                if self.token(b"MESH_CVERTLIST") {
                    self.parse_color_list_block(color_vertex_count, mesh)?;
                    continue;
                }
                if self.token(b"MESH_CFACELIST") {
                    self.parse_index_face_list_block(color_face_count, mesh, None)?;
                    continue;
                }
                if self.token(b"MESH_NORMALS") {
                    self.parse_normal_list_block(mesh)?;
                    continue;
                }
                if self.token(b"MESH_MAPPINGCHANNEL") {
                    let index = self.parse_long();
                    if index == 0 {
                        self.skip_section()?;
                    } else {
                        if index < 2 {
                            self.skip_section()?;
                        }
                        if index as usize > MAX_TEXTURE_COORDS {
                            self.skip_section()?;
                        } else {
                            self.parse_mapping_channel(index as usize - 1, mesh)?;
                        }
                        continue;
                    }
                }
                if self.token(b"MESH_ANIMATION") {
                    continue;
                }
                if self.token(b"MESH_WEIGHTS") {
                    self.parse_weights_block(mesh)?;
                    continue;
                }
            }
            if let Step::Return = self.section_step(&mut depth, "*MESH")? {
                return Ok(());
            }
        }
    }

    /// `ParseLV3MeshWeightsBlock`.
    fn parse_weights_block(&mut self, mesh: &mut MeshData) -> Result<(), ImportError> {
        let mut depth = 0;
        let mut vertex_count = 0u32;
        let mut bone_count = 0u32;
        loop {
            if self.byte() == b'*' {
                self.position += 1;
                if self.token(b"MESH_NUMVERTEX") {
                    vertex_count = self.parse_long();
                    continue;
                }
                if self.token(b"MESH_NUMBONE") {
                    bone_count = self.parse_long();
                    continue;
                }
                if self.token(b"MESH_BONE_LIST") {
                    self.parse_bones(bone_count, mesh)?;
                    continue;
                }
                if self.token(b"MESH_BONE_VERTEX_LIST") {
                    self.parse_bone_vertices(vertex_count, mesh)?;
                    continue;
                }
            }
            if let Step::Return = self.section_step(&mut depth, "*MESH_WEIGHTS")? {
                return Ok(());
            }
        }
    }

    /// `ParseLV4MeshBones`: the bone list grown to the count, names by
    /// index.
    fn parse_bones(&mut self, bone_count: u32, mesh: &mut MeshData) -> Result<(), ImportError> {
        if bone_count as usize > mesh.bones.len() {
            scene::check_allocation(bone_count as u64, BONE_SIZE)?;
        }
        mesh.bones.resize(bone_count as usize, b"UNNAMED".to_vec());
        let mut depth = 0;
        loop {
            if self.byte() == b'*' {
                self.position += 1;
                if self.token(b"MESH_BONE_NAME") && self.skip_spaces() {
                    let (index, after) = strtoul10(&self.buffer, self.position);
                    self.position = after;
                    if index >= bone_count {
                        continue;
                    }
                    let mut name = mesh.bones[index as usize].clone();
                    self.parse_string_into(&mut name)?;
                    mesh.bones[index as usize] = name;
                    continue;
                }
            }
            if let Step::Return = self.section_step(&mut depth, "*MESH_BONE_LIST")? {
                return Ok(());
            }
        }
    }

    /// `ParseLV4MeshBonesVertices`: each bone vertex (by index, clamped to
    /// the vertices) and its weights' bones (-1 marks an unused one).
    fn parse_bone_vertices(&mut self, vertex_count: u32, mesh: &mut MeshData) -> Result<(), ImportError> {
        if vertex_count as usize > mesh.bone_vertices.len() {
            scene::check_allocation(vertex_count as u64, BONE_VERTEX_SIZE)?;
        }
        mesh.bone_vertices.resize(vertex_count as usize, Vec::new());
        let mut depth = 0;
        loop {
            if self.byte() == b'*' {
                self.position += 1;
                if self.token(b"MESH_BONE_VERTEX") {
                    let (mut index, after) = strtoul10(&self.buffer, self.position);
                    self.position = after;
                    if index as usize >= mesh.positions.len() {
                        index = (mesh.positions.len() as u32).wrapping_sub(1);
                    }
                    self.parse_real_triple()?;
                    loop {
                        if !self.skip_spaces() {
                            break;
                        }
                        let (bone, after_bone) = strtoul10(&self.buffer, self.position);
                        self.position = after_bone;
                        if !self.skip_spaces() {
                            break;
                        }
                        let (_, after_weight) = fast_atoreal_move(&self.buffer, self.position, true)?;
                        self.position = after_weight;
                        if bone as i32 != -1 {
                            let Some(vertex) = mesh.bone_vertices.get_mut(index as usize) else {
                                return undefined("a bone vertex past the list");
                            };
                            vertex.push(bone);
                        }
                    }
                    continue;
                }
            }
            if let Step::Return = self.section_step(&mut depth, "*MESH_BONE_VERTEX")? {
                return Ok(());
            }
        }
    }

    /// `ParseLV3MeshVertexListBlock`: the vertices grown to the count,
    /// set by index (past the count: ignored).
    fn parse_vertex_list_block(&mut self, vertex_count: u32, mesh: &mut MeshData) -> Result<(), ImportError> {
        scene::check_allocation(vertex_count as u64, VECTOR_SIZE)?;
        mesh.positions.resize(vertex_count as usize, [0.0; 3]);
        let mut depth = 0;
        loop {
            if self.byte() == b'*' {
                self.position += 1;
                if self.token(b"MESH_VERTEX") {
                    let index = self.parse_long();
                    let value = self.parse_real_triple()?;
                    if index < vertex_count {
                        mesh.positions[index as usize] = value;
                    }
                    continue;
                }
            }
            if let Step::Return = self.section_step(&mut depth, "*MESH_VERTEX_LIST")? {
                return Ok(());
            }
        }
    }

    /// `ParseLV3MeshFaceListBlock`: the faces grown to the count, set by
    /// index (a face set this way has its texture and colour indices unset
    /// again).
    fn parse_face_list_block(&mut self, face_count: u32, mesh: &mut MeshData) -> Result<(), ImportError> {
        scene::check_allocation(face_count as u64, FACE_SIZE)?;
        mesh.faces.resize(face_count as usize, Face::default());
        let mut depth = 0;
        loop {
            if self.byte() == b'*' {
                self.position += 1;
                if self.token(b"MESH_FACE") {
                    let face = self.parse_mesh_face()?;
                    let index = face.face_index;
                    if index < face_count {
                        mesh.faces[index as usize] = face;
                    }
                    continue;
                }
            }
            if let Step::Return = self.section_step(&mut depth, "*MESH_FACE_LIST")? {
                return Ok(());
            }
        }
    }

    /// `ParseLV3MeshTListBlock`: a channel's coordinates, grown to the
    /// count and set by index; a third coordinate makes it 3D.
    fn parse_texture_vertex_list_block(&mut self, count: u32, mesh: &mut MeshData, channel: usize) -> Result<(), ImportError> {
        scene::check_allocation(count as u64, VECTOR_SIZE)?;
        mesh.texture_coords[channel].resize(count as usize, [0.0; 3]);
        let mut depth = 0;
        loop {
            if self.byte() == b'*' {
                self.position += 1;
                if self.token(b"MESH_TVERT") {
                    let index = self.parse_long();
                    let value = self.parse_real_triple()?;
                    if index < count {
                        mesh.texture_coords[channel][index as usize] = value;
                    }
                    if value[2] != 0.0 {
                        mesh.uv_components[channel] = 3;
                    }
                    continue;
                }
            }
            if let Step::Return = self.section_step(&mut depth, "*MESH_TVERT_LIST")? {
                return Ok(());
            }
        }
    }

    /// `ParseLV3MeshTFaceListBlock` (a texture channel's indices) and
    /// `ParseLV3MeshCFaceListBlock` (the colour indices, `channel` none).
    fn parse_index_face_list_block(&mut self, count: u32, mesh: &mut MeshData, channel: Option<usize>) -> Result<(), ImportError> {
        let (token, what): (&[u8], &str) = match channel {
            Some(_) => (b"MESH_TFACE", "*MESH_TFACE_LIST"),
            None => (b"MESH_CFACE", "*MESH_CFACE_LIST"),
        };
        let mut depth = 0;
        loop {
            if self.byte() == b'*' {
                self.position += 1;
                if self.token(token) {
                    let index = self.parse_long();
                    let values = self.parse_long_triple();
                    if index < count && (index as usize) < mesh.faces.len() {
                        let face = &mut mesh.faces[index as usize];
                        let indices = values.map(Some);
                        match channel {
                            Some(channel) => face.uv_indices[channel] = indices,
                            None => face.color_indices = indices,
                        }
                    }
                    continue;
                }
            }
            if let Step::Return = self.section_step(&mut depth, what)? {
                return Ok(());
            }
        }
    }

    /// `ParseLV3MappingChannel`.
    fn parse_mapping_channel(&mut self, channel: usize, mesh: &mut MeshData) -> Result<(), ImportError> {
        let mut depth = 0;
        let mut vertex_count = 0u32;
        let mut face_count = 0u32;
        loop {
            if self.byte() == b'*' {
                self.position += 1;
                if self.token(b"MESH_NUMTVERTEX") {
                    vertex_count = self.parse_long();
                    continue;
                }
                if self.token(b"MESH_NUMTVFACES") {
                    face_count = self.parse_long();
                    continue;
                }
                if self.token(b"MESH_TVERTLIST") {
                    self.parse_texture_vertex_list_block(vertex_count, mesh, channel)?;
                    continue;
                }
                if self.token(b"MESH_TFACELIST") {
                    self.parse_index_face_list_block(face_count, mesh, Some(channel))?;
                    continue;
                }
            }
            if let Step::Return = self.section_step(&mut depth, "*MESH_MAPPING_CHANNEL")? {
                return Ok(());
            }
        }
    }

    /// `ParseLV3MeshCListBlock`.
    fn parse_color_list_block(&mut self, count: u32, mesh: &mut MeshData) -> Result<(), ImportError> {
        scene::check_allocation(count as u64, COLOR_SIZE)?;
        mesh.vertex_colors.resize(count as usize, [0.0; 4]);
        let mut depth = 0;
        loop {
            if self.byte() == b'*' {
                self.position += 1;
                if self.token(b"MESH_VERTCOL") {
                    let index = self.parse_long();
                    let [r, g, b] = self.parse_real_triple()?;
                    if index < count {
                        mesh.vertex_colors[index as usize] = [r, g, b, 1.0];
                    }
                    continue;
                }
            }
            if let Step::Return = self.section_step(&mut depth, "*MESH_CVERTEX_LIST")? {
                return Ok(());
            }
        }
    }

    /// `ParseLV3MeshNormalListBlock`: the normals (replaced later) sized
    /// three per face; values read past.
    fn parse_normal_list_block(&mut self, mesh: &mut MeshData) -> Result<(), ImportError> {
        let count = mesh.faces.len() * 3;
        scene::check_allocation(count as u64, VECTOR_SIZE)?;
        mesh.normal_count = count;
        let mut face_index = u32::MAX;
        let mut depth = 0;
        loop {
            if self.byte() == b'*' {
                self.position += 1;
                if face_index != u32::MAX && self.token(b"MESH_VERTEXNORMAL") {
                    self.parse_long();
                    self.parse_real_triple()?;
                    continue;
                }
                if self.token(b"MESH_FACENORMAL") {
                    face_index = self.parse_long();
                    self.parse_real_triple()?;
                    continue;
                }
            }
            if let Step::Return = self.section_step(&mut depth, "*MESH_NORMALS")? {
                return Ok(());
            }
        }
    }

    /// `ParseLV4MeshFace`: `index: A: a B: b C: c`, then optionally
    /// `*MESH_SMOOTHING` groups and `*MESH_MTLID`; a malformed face stops
    /// at the next token with what it has.
    fn parse_mesh_face(&mut self) -> Result<Face, ImportError> {
        let mut face = Face::default();
        if !self.skip_spaces() {
            self.skip_to_next_token()?;
            return Ok(face);
        }
        let (face_index, after) = strtoul10(&self.buffer, self.position);
        self.position = after;
        face.face_index = face_index;
        if !self.skip_spaces() {
            self.skip_to_next_token()?;
            return Ok(face);
        }
        if self.byte() == b':' {
            self.position += 1;
        }
        for _ in 0..3 {
            if !self.skip_spaces() {
                self.skip_to_next_token()?;
                return Ok(face);
            }
            let corner = match self.byte() {
                b'A' | b'a' => 0,
                b'B' | b'b' => 1,
                b'C' | b'c' => 2,
                _ => {
                    self.skip_to_next_token()?;
                    return Ok(face);
                }
            };
            self.position += 1;
            if !self.skip_spaces() || self.byte() != b':' {
                self.skip_to_next_token()?;
                return Ok(face);
            }
            self.position += 1;
            if !self.skip_spaces() {
                self.skip_to_next_token()?;
                return Ok(face);
            }
            let (index, after_index) = strtoul10(&self.buffer, self.position);
            self.position = after_index;
            face.indices[corner] = index;
        }

        if !self.skip_to_star_on_line() {
            return Ok(face);
        }
        if self.token(b"*MESH_SMOOTHING") {
            if !self.skip_spaces() {
                self.skip_to_next_token()?;
                return Ok(face);
            }
            loop {
                let byte = self.byte();
                // (Assimp tests for a digit below '9'.)
                if (b'0'..=b'8').contains(&byte) {
                    let (value, after_value) = strtoul10(&self.buffer, self.position);
                    self.position = after_value;
                    if value < 32 {
                        // `1 << strtoul10(…)` reads on from after the value.
                        let (shift, after_shift) = strtoul10(&self.buffer, self.position);
                        self.position = after_shift;
                        face.smooth_group |= 1u32.wrapping_shl(shift);
                    }
                }
                self.skip_spaces();
                if self.byte() != b',' {
                    break;
                }
                self.position += 1;
                self.skip_spaces();
            }
        }
        if !self.skip_to_star_on_line() {
            return Ok(face);
        }
        if self.token(b"*MESH_MTLID") {
            if !self.skip_spaces() {
                self.skip_to_next_token()?;
                return Ok(face);
            }
            let (material, after_material) = strtoul10(&self.buffer, self.position);
            self.position = after_material;
            face.material = material;
        }
        Ok(face)
    }

    /// On to a `*` on the same line; false at the line's end.
    fn skip_to_star_on_line(&mut self) -> bool {
        loop {
            let byte = self.byte();
            if byte == b'*' {
                return true;
            }
            if is_line_end(byte) {
                return false;
            }
            self.position += 1;
        }
    }
}

/// `GenerateDefaultMaterial`: meshes without a material get a new last
/// one (made too when there are none at all).
fn generate_default_material(meshes: &mut [Object], materials: &mut Vec<Material>) {
    let mut needed = false;
    for mesh in meshes.iter_mut() {
        if mesh.mesh.material_index == DEFAULT_MATERIAL {
            mesh.mesh.material_index = materials.len() as u32;
            needed = true;
        }
    }
    if needed || materials.is_empty() {
        materials.push(Material::default());
    }
}

/// `BuildUniqueRepresentation`: three vertices per face, with their
/// texture coordinates (channels up to the first empty one; later ones
/// stay zero), colours and normals.
fn build_unique_representation(mesh: &mut MeshData) -> Result<(), ImportError> {
    let size = mesh.faces.len() * 3;
    let mut positions = Vec::with_capacity(size);
    let mut coords: [Vec<Vector3>; MAX_TEXTURE_COORDS] = Default::default();
    for (channel, list) in coords.iter_mut().enumerate() {
        if !mesh.texture_coords[channel].is_empty() {
            *list = vec![[0.0; 3]; size];
        }
    }
    let mut colors = Vec::new();
    let has_colors = !mesh.vertex_colors.is_empty();

    for (face_index, face) in mesh.faces.iter_mut().enumerate() {
        for corner in 0..3 {
            let current = positions.len();
            let Some(&position) = mesh.positions.get(face.indices[corner] as usize) else {
                return undefined("a face index past the vertices");
            };
            positions.push(position);
            for (channel, source) in mesh.texture_coords.iter().enumerate() {
                if source.is_empty() {
                    break;
                }
                let Some(index) = face.uv_indices[channel][corner] else {
                    return undefined("a texture coordinate index never set");
                };
                let Some(&coord) = source.get(index as usize) else {
                    return undefined("a texture coordinate index past the coordinates");
                };
                coords[channel][current] = coord;
            }
            if has_colors {
                let Some(index) = face.color_indices[corner] else {
                    return undefined("a colour index never set");
                };
                let Some(&color) = mesh.vertex_colors.get(index as usize) else {
                    return undefined("a colour index past the colours");
                };
                colors.push(color);
            }
            if mesh.normal_count != 0 && face_index * 3 + corner >= mesh.normal_count {
                return undefined("a normal read past the normals");
            }
            face.indices[corner] = current as u32;
        }
    }
    if mesh.normal_count != 0 {
        mesh.normal_count = size;
    }
    mesh.positions = positions;
    mesh.vertex_colors = colors;
    mesh.texture_coords = coords;
    Ok(())
}

/// The bone weights' bone indices must lie in the bone list (the output
/// bone lists are indexed by them).
fn check_weights<'a>(bone_count: usize, vertices: impl Iterator<Item = &'a Vec<u32>>) -> Result<(), ImportError> {
    for weights in vertices {
        if weights.iter().any(|&bone| bone as usize >= bone_count) {
            return undefined("a weight naming a bone past the bones");
        }
    }
    Ok(())
}

/// An output mesh and the object it belongs to.
struct OutputMesh {
    mesh: Mesh,
    object: usize,
}

/// `ConvertMeshes`: the mesh split by its material's sub-materials (faces
/// past them going to the last), or else whole.
fn convert_mesh(mesh: &mut MeshData, normals: &[Vector3], materials: &[Material], object: usize, output: &mut Vec<OutputMesh>) -> Result<(), ImportError> {
    if mesh.material_index as usize >= materials.len() {
        mesh.material_index = materials.len() as u32 - 1;
    }
    let sub_count = materials[mesh.material_index as usize].sub_materials.len();
    let channels: Vec<usize> = (0..MAX_TEXTURE_COORDS).filter(|&channel| !mesh.texture_coords[channel].is_empty()).collect();

    if sub_count == 0 {
        let mut out = Mesh { primitive_types: PRIMITIVE_TRIANGLE, ..Mesh::default() };
        if !mesh.faces.is_empty() && !mesh.positions.is_empty() {
            out.vertex_count = mesh.positions.len();
            out.vertices = Some(mesh.positions.clone());
            out.normals = Some(normals.to_vec());
            for &channel in &channels {
                out.texture_coords[channel] = Some(mesh.texture_coords[channel].clone());
                out.uv_components[channel] = mesh.uv_components[channel];
            }
            if !mesh.vertex_colors.is_empty() {
                out.colors[0] = Some(mesh.vertex_colors.clone());
            }
            let faces = mesh.faces.iter().map(|face| face.indices.to_vec()).collect();
            out.faces = Some(faces);
            if !mesh.bones.is_empty() && !mesh.bone_vertices.is_empty() {
                check_weights(mesh.bones.len(), mesh.bone_vertices.iter())?;
            }
        }
        output.push(OutputMesh { mesh: out, object });
        return Ok(());
    }

    let mut split: Vec<Vec<usize>> = vec![Vec::new(); sub_count];
    for (index, face) in mesh.faces.iter().enumerate() {
        let material = (face.material as usize).min(sub_count - 1);
        split[material].push(index);
    }
    for faces in &split {
        if faces.is_empty() {
            continue;
        }
        let mut vertices = Vec::with_capacity(faces.len() * 3);
        let mut out_normals = Vec::with_capacity(faces.len() * 3);
        let mut face_lists = Vec::with_capacity(faces.len());
        for &face in faces {
            let first = vertices.len() as u32;
            for &index in &mesh.faces[face].indices {
                vertices.push(mesh.positions[index as usize]);
                out_normals.push(normals[index as usize]);
                if !mesh.bones.is_empty()
                    && let Some(weights) = mesh.bone_vertices.get(index as usize)
                {
                    check_weights(mesh.bones.len(), std::iter::once(weights))?;
                }
            }
            face_lists.push(vec![first, first + 1, first + 2]);
        }
        let mut out = Mesh {
            primitive_types: PRIMITIVE_TRIANGLE,
            vertex_count: vertices.len(),
            vertices: Some(vertices),
            normals: Some(out_normals),
            faces: Some(face_lists),
            ..Mesh::default()
        };
        for &channel in &channels {
            let source = &mesh.texture_coords[channel];
            let coords = faces.iter().flat_map(|&face| mesh.faces[face].indices.map(|index| source[index as usize])).collect();
            out.texture_coords[channel] = Some(coords);
            out.uv_components[channel] = mesh.uv_components[channel];
        }
        if !mesh.vertex_colors.is_empty() {
            let colors = faces.iter().flat_map(|&face| mesh.faces[face].indices.map(|index| mesh.vertex_colors[index as usize])).collect();
            out.colors[0] = Some(colors);
        }
        output.push(OutputMesh { mesh: out, object });
    }
    Ok(())
}

/// `aiString::Set(const std::string &)`: a name over 1023 bytes leaves
/// it empty.
fn ai_string(name: &[u8]) -> Vec<u8> {
    if name.len() > MAX_LENGTH - 1 {
        return Vec::new();
    }
    name.to_vec()
}

/// `aiString::Set(const char *)`: a name cut to 1023 bytes.
fn ai_string_truncated(name: &[u8]) -> Vec<u8> {
    name[..name.len().min(MAX_LENGTH - 1)].to_vec()
}

/// `BuildNodes`' graph: the objects (lights, cameras, meshes, dummies)
/// under the nodes their parent names name.
struct Graph<'a> {
    objects: &'a mut [&'a mut Object],
    nodes: Vec<Node>,
    output: &'a mut [OutputMesh],
    depth: usize,
}

impl Graph<'_> {
    /// `AddMeshes`: the object's meshes moved back into its local space.
    fn add_meshes(&mut self, object: usize) {
        let transform = self.objects[object].transform;
        let inverse = transform.inverse();
        let m = transform.0;
        for out in self.output.iter_mut() {
            if out.object != object {
                continue;
            }
            if let Some(vertices) = out.mesh.vertices.as_mut() {
                for vertex in vertices.iter_mut() {
                    *vertex = inverse.transform(*vertex);
                }
            }
            if let Some(normals) = out.mesh.normals.as_mut() {
                for normal in normals.iter_mut() {
                    let [x, y, z] = *normal;
                    *normal = [m[0] * x + m[4] * y + m[8] * z, m[1] * x + m[5] * y + m[9] * z, m[2] * x + m[6] * y + m[10] * z];
                }
            }
        }
    }

    /// `AddNodes`: every object whose parent is `name` (none: the top
    /// level) under `parent`, placed relative to `matrix`, its own
    /// children under it (unless it repeats its parent's or grandparent's
    /// name), then its meshes, or a target node first among its children.
    fn add_nodes(&mut self, parent: usize, name: &[u8], matrix: Matrix4) -> Result<(), ImportError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return undefined("a node graph too deep for the stack");
        }
        let adjust = matrix.inverse();
        let mut children = Vec::new();
        for object in 0..self.objects.len() {
            let matches = if name.is_empty() { self.objects[object].parent.is_empty() } else { self.objects[object].parent == name };
            if !matches {
                continue;
            }
            self.objects[object].processed = true;
            let object_name = if self.objects[object].name.is_empty() { b"Unnamed_Node".to_vec() } else { self.objects[object].name.clone() };
            let node_name = ai_string_truncated(&object_name);
            let transform = adjust.product(&self.objects[object].transform);
            let node = self.nodes.len();
            self.nodes.push(Node::new(node_name.clone(), transform, Some(parent)));
            children.push(node);

            let grandparent = self.nodes[parent].parent.unwrap_or(parent);
            if node_name != self.nodes[parent].name && node_name != self.nodes[grandparent].name {
                let object_transform = self.objects[object].transform;
                self.add_nodes(node, &node_name, object_transform)?;
            }

            if self.objects[object].kind == NodeKind::Mesh {
                self.add_meshes(object);
            } else if !self.objects[object].target_position[0].is_nan() {
                let existing = self.nodes[node].children.clone();
                if existing.len() >= 2 {
                    return undefined("a target node shifted over two or more children");
                }
                let target = self.objects[object].target_position;
                let base = self.objects[object].transform.0;
                let mut offset = Matrix4::IDENTITY;
                offset.0[3] = target[0] - base[3];
                offset.0[7] = target[1] - base[7];
                offset.0[11] = target[2] - base[11];
                let target_node = self.nodes.len();
                self.nodes.push(Node::new(Vec::new(), offset, Some(node)));
                let mut shifted = vec![target_node];
                shifted.extend(existing);
                self.nodes[node].children = shifted;
            }
        }
        if !children.is_empty() {
            self.nodes[parent].children = children;
        }
        self.depth -= 1;
        Ok(())
    }
}

pub fn read(path: &str, bytes: &[u8]) -> Result<Scene, ImportError> {
    let buffer = text_file_to_buffer(bytes, false)?;
    let end = buffer.len();
    let default_format = match path.as_bytes().last() {
        Some(b'C') | Some(b'c') => OLD_FILE_FORMAT,
        _ => NEW_FILE_FORMAT,
    };
    let mut parser = Parser {
        buffer,
        position: 0,
        end,
        file_format: default_format,
        materials: Vec::new(),
        meshes: Vec::new(),
        dummies: Vec::new(),
        lights: Vec::new(),
        cameras: Vec::new(),
        depth: 0,
    };
    parser.parse()?;

    let Parser { mut materials, mut meshes, mut dummies, mut lights, mut cameras, .. } = parser;
    let mut output = Vec::new();
    if !meshes.is_empty() {
        generate_default_material(&mut meshes, &mut materials);
        for (index, mesh) in meshes.iter_mut().enumerate() {
            build_unique_representation(&mut mesh.mesh)?;
            let faces: Vec<([u32; 3], u32)> = mesh.mesh.faces.iter().map(|face| (face.indices, face.smooth_group)).collect();
            let normals = smoothing_groups::compute_normals(&mesh.mesh.positions, &faces);
            convert_mesh(&mut mesh.mesh, &normals, &materials, index, &mut output)?;
        }
        output.retain(|out| out.mesh.face_count() != 0);
    }

    // `BuildNodes`: the objects in one list, their matrices transposed.
    let light_count = lights.len();
    let camera_count = cameras.len();
    let mut objects: Vec<&mut Object> = Vec::new();
    objects.extend(lights.iter_mut());
    objects.extend(cameras.iter_mut());
    objects.extend(meshes.iter_mut());
    objects.extend(dummies.iter_mut());
    for object in objects.iter_mut() {
        object.transform = object.transform.transpose();
    }
    let mesh_offset = light_count + camera_count;
    for out in output.iter_mut() {
        out.object += mesh_offset;
    }

    let root = Node::new(b"<ASERoot>".to_vec(), Matrix4::IDENTITY, None);
    let top = Node::new(Vec::new(), Matrix4::IDENTITY, Some(0));
    let mut graph = Graph { objects: &mut objects, nodes: vec![root, top], output: &mut output, depth: 0 };
    graph.nodes[0].children = vec![1];
    graph.add_nodes(1, b"", Matrix4::IDENTITY)?;

    // Objects not placed whose parent no other object names hang under
    // the root (with no matrix of their own).
    let mut orphans = Vec::new();
    for object in 0..graph.objects.len() {
        if graph.objects[object].processed {
            continue;
        }
        let parent = &graph.objects[object].parent;
        let known = (0..graph.objects.len()).any(|other| other != object && graph.objects[other].name == *parent);
        if !known {
            orphans.push(object);
        }
    }
    for object in orphans {
        let name = ai_string(&graph.objects[object].name);
        let node = graph.nodes.len();
        graph.nodes.push(Node::new(name.clone(), Matrix4::IDENTITY, Some(0)));
        graph.nodes[0].children.push(node);
        graph.add_meshes(object);
        graph.add_nodes(node, &name, Matrix4::IDENTITY)?;
    }
    let mut nodes = std::mem::take(&mut graph.nodes);
    nodes[0].transform = Matrix4([1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]);

    let mut meshes_out: Vec<Mesh> = output.into_iter().map(|out| out.mesh).collect();
    if meshes_out.is_empty() {
        meshes_out.push(skeleton::build_mesh(&nodes, 0, false));
    }
    Ok(Scene { meshes: meshes_out })
}
