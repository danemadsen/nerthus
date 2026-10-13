//! Assimp's `ColladaParser` (ColladaParser.cpp at commit 97257a3b): the
//! libraries of a COLLADA document read from pugixml's tree, with the
//! quirks that decide whether an import throws. What only feeds lights,
//! cameras, metadata or material colours is skipped unless it can throw.
//!
//! Behaviour the C++ leaves undefined (reads past a vector, a null
//! accessor, unbounded loops) is reported as an error.

use std::collections::{BTreeMap, BTreeSet};

use crate::fast_atof::{fast_atoreal_move, strtol10, strtoul10};
use crate::scene::{ImportError, MAX_COLOR_SETS, MAX_TEXTURE_COORDS, fail};
use crate::xml::Node as XmlNode;

/// Allocations the C++ makes from file counts; larger ones are treated as
/// failing.
const MAX_ELEMENTS: u64 = 1 << 28;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatVersion {
    V1_5,
    V1_4,
    V1_3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpDirection {
    X,
    Y,
    Z,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransformType {
    LookAt,
    Rotate,
    Translate,
    Scale,
    Skew,
    Matrix,
}

impl TransformType {
    /// `sNumParameters`.
    fn parameter_count(self) -> usize {
        match self {
            TransformType::LookAt => 9,
            TransformType::Rotate => 4,
            TransformType::Translate => 3,
            TransformType::Scale => 3,
            TransformType::Skew => 7,
            TransformType::Matrix => 16,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Transform {
    pub id: Vec<u8>,
    pub kind: TransformType,
    pub values: [f32; 16],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputType {
    Invalid,
    Vertex,
    Position,
    Normal,
    Texcoord,
    Color,
    Tangent,
    Bitangent,
}

/// An `<input>`; `resolved` stands for `mResolved`, which always points
/// at the accessor library's entry for `accessor`.
#[derive(Clone, Debug)]
pub struct InputChannel {
    pub kind: InputType,
    pub index: u64,
    pub offset: u64,
    pub accessor: Vec<u8>,
    pub resolved: bool,
}

impl Default for InputChannel {
    fn default() -> Self {
        InputChannel { kind: InputType::Invalid, index: 0, offset: 0, accessor: Vec::new(), resolved: false }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Data {
    pub is_string_array: bool,
    pub values: Vec<f32>,
    pub strings: Vec<Vec<u8>>,
}

/// An `<accessor>`; `data_resolved` stands for `mData`, which points at
/// the data library's entry for `source`.
#[derive(Clone, Debug, Default)]
pub struct Accessor {
    pub count: u64,
    pub size: u64,
    pub offset: u64,
    pub stride: u64,
    pub params: Vec<Vec<u8>>,
    pub sub_offset: [u64; 4],
    pub source: Vec<u8>,
    pub data_resolved: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SubMesh {
    pub material: Vec<u8>,
    pub face_count: u64,
}

/// A face position index read from before the index array (no VERTEX
/// input): heap garbage.
pub const GARBAGE_INDEX: u64 = u64::MAX;

#[derive(Clone, Debug)]
pub struct Mesh {
    pub id: Vec<u8>,
    pub name: Vec<u8>,
    pub vertex_id: Vec<u8>,
    pub per_vertex_data: Vec<InputChannel>,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 3]>,
    pub bitangents: Vec<[f32; 3]>,
    pub texture_coords: [Vec<[f32; 3]>; MAX_TEXTURE_COORDS],
    pub colors: [Vec<[f32; 4]>; MAX_COLOR_SETS],
    pub uv_components: [u32; MAX_TEXTURE_COORDS],
    pub face_sizes: Vec<u64>,
    pub face_position_indices: Vec<u64>,
    pub sub_meshes: Vec<SubMesh>,
}

impl Mesh {
    fn new(id: Vec<u8>) -> Self {
        Mesh {
            id,
            name: Vec::new(),
            vertex_id: Vec::new(),
            per_vertex_data: Vec::new(),
            positions: Vec::new(),
            normals: Vec::new(),
            tangents: Vec::new(),
            bitangents: Vec::new(),
            texture_coords: Default::default(),
            colors: Default::default(),
            uv_components: [2; MAX_TEXTURE_COORDS],
            face_sizes: Vec::new(),
            face_position_indices: Vec::new(),
            sub_meshes: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrimitiveType {
    Lines,
    LineStrip,
    Triangles,
    TriStrips,
    TriFans,
    Polylist,
    Polygon,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ControllerType {
    #[default]
    Skin,
    Morph,
}

#[derive(Clone, Debug, Default)]
pub struct Controller {
    pub kind: ControllerType,
    pub mesh_id: Vec<u8>,
    pub joint_name_source: Vec<u8>,
    pub bind_shape_matrix: [f32; 16],
    pub joint_offset_matrix_source: Vec<u8>,
    pub weight_input_joints: InputChannel,
    pub weight_input_weights: InputChannel,
    pub weight_counts: Vec<u64>,
    pub weights: Vec<(u64, u64)>,
    pub morph_target: Vec<u8>,
    pub morph_weight: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct Material {
    pub effect: Vec<u8>,
}

/// An effect's parameters (`sid` → reference) and the sampler names its
/// colour slots refer to.
#[derive(Clone, Debug, Default)]
pub struct Effect {
    pub params: BTreeMap<Vec<u8>, Vec<u8>>,
    pub emissive: Vec<u8>,
    pub ambient: Vec<u8>,
    pub diffuse: Vec<u8>,
    pub specular: Vec<u8>,
    pub transparent: Vec<u8>,
    pub bump: Vec<u8>,
    pub reflective: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct Image {
    pub file_name: Vec<u8>,
    pub data_size: usize,
}

/// An `<instance_material>`: the target material and whether it binds
/// vertex inputs (`SemanticMappingTable::mMap` not empty).
#[derive(Clone, Debug, Default)]
pub struct MaterialBinding {
    pub material: Vec<u8>,
    pub has_inputs: bool,
}

#[derive(Clone, Debug, Default)]
pub struct MeshInstance {
    pub mesh_or_controller: Vec<u8>,
    pub materials: BTreeMap<Vec<u8>, MaterialBinding>,
}

#[derive(Clone, Debug, Default)]
pub struct Node {
    pub name: Vec<u8>,
    pub id: Vec<u8>,
    pub sid: Vec<u8>,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub transforms: Vec<Transform>,
    pub meshes: Vec<MeshInstance>,
    pub node_instances: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Default)]
pub struct AnimationChannel {
    pub target: Vec<u8>,
    pub source_times: Vec<u8>,
    pub source_values: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct Animation {
    pub name: Vec<u8>,
    pub channels: Vec<AnimationChannel>,
    pub sub_animations: Vec<usize>,
}

/// `ColladaParser`: nodes and animations live in arenas (`root_animation`
/// is `mAnims`).
pub struct Parser {
    pub format: FormatVersion,
    pub unit_size: f32,
    pub up_direction: UpDirection,
    pub data: BTreeMap<Vec<u8>, Data>,
    pub accessors: BTreeMap<Vec<u8>, Accessor>,
    pub meshes: BTreeMap<Vec<u8>, Mesh>,
    pub nodes: Vec<Node>,
    pub node_library: BTreeMap<Vec<u8>, usize>,
    pub images: BTreeMap<Vec<u8>, Image>,
    pub effects: BTreeMap<Vec<u8>, Effect>,
    pub materials: BTreeMap<Vec<u8>, Material>,
    pub controllers: BTreeMap<Vec<u8>, Controller>,
    pub root: Option<usize>,
    pub animations: Vec<Animation>,
    pub root_animation: usize,
    animation_library: BTreeMap<Vec<u8>, usize>,
    animation_clips: Vec<(Vec<u8>, Vec<Vec<u8>>)>,
}

fn at(bytes: &[u8], index: usize) -> u8 {
    bytes.get(index).copied().unwrap_or(0)
}

/// `std::isspace` in the C locale.
fn is_c_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

/// `ai_trim`.
fn trim(bytes: &[u8]) -> Vec<u8> {
    let start = bytes.iter().position(|&byte| !is_c_space(byte)).unwrap_or(bytes.len());
    let end = bytes.iter().rposition(|&byte| !is_c_space(byte)).map_or(start, |last| last + 1);
    bytes[start..end.max(start)].to_vec()
}

/// `XmlParser::getValueAsString`: the first text child, trimmed.
fn value_as_string(node: &XmlNode) -> Vec<u8> {
    trim(node.text())
}

/// `SkipSpacesAndLineEnd`.
fn skip_spaces_and_line_end(bytes: &[u8], mut index: usize) -> usize {
    while matches!(at(bytes, index), b' ' | b'\t' | b'\r' | b'\n') {
        index += 1;
    }
    index
}

/// `IsSpaceOrNewLine`.
fn is_space_or_new_line(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | 0 | 0x0C)
}

/// pugixml's `string_to_integer<unsigned int>`.
fn string_to_integer(value: &[u8], minimum: u32, maximum: u32) -> u32 {
    let mut index = 0;
    while matches!(at(value, index), b' ' | b'\t' | b'\n' | b'\r') {
        index += 1;
    }
    let negative = at(value, index) == b'-';
    if matches!(at(value, index), b'+' | b'-') {
        index += 1;
    }
    let mut result = 0u32;
    
    let overflow = if at(value, index) == b'0' && (at(value, index + 1) | b' ') == b'x' {
        index += 2;
        while at(value, index) == b'0' {
            index += 1;
        }
        let start = index;
        loop {
            let byte = at(value, index);
            if byte.wrapping_sub(b'0') < 10 {
                result = result.wrapping_mul(16).wrapping_add((byte - b'0') as u32);
            } else if (byte | b' ').wrapping_sub(b'a') < 6 {
                result = result.wrapping_mul(16).wrapping_add(((byte | b' ') - b'a' + 10) as u32);
            } else {
                break;
            }
            index += 1;
        }
        index - start > 8
    } else {
        while at(value, index) == b'0' {
            index += 1;
        }
        let start = index;
        while at(value, index).wrapping_sub(b'0') < 10 {
            result = result.wrapping_mul(10).wrapping_add((at(value, index) - b'0') as u32);
            index += 1;
        }
        let digits = index - start;
        let lead = at(value, start);
        let fits = digits == 10 && (lead < b'4' || (lead == b'4' && result >> 31 != 0));
        digits >= 10 && !fits
    };
    if negative {
        let limit = 0u32.wrapping_sub(minimum);
        if overflow || result > limit { minimum } else { 0u32.wrapping_sub(result) }
    } else if overflow || result > maximum {
        maximum
    } else {
        result
    }
}

/// `xml_attribute::as_string()`: "" when missing.
fn attribute_string<'a>(node: &'a XmlNode, name: &str) -> &'a [u8] {
    node.attribute(name).unwrap_or(b"")
}

/// `xml_attribute::as_int()` of a present attribute.
fn attribute_int(node: &XmlNode, name: &str) -> Option<i32> {
    let value = node.attribute(name)?;
    Some(string_to_integer(value, i32::MIN as u32, i32::MAX as u32) as i32)
}

/// `xml_attribute::as_uint()` of a present attribute.
fn attribute_uint(node: &XmlNode, name: &str) -> Option<u32> {
    let value = node.attribute(name)?;
    Some(string_to_integer(value, 0, u32::MAX))
}

/// The text after a reference's '#' (`c_str() + 1`).
fn after_hash(reference: &[u8]) -> Vec<u8> {
    reference.get(1..).unwrap_or(&[]).to_vec()
}

/// `readUrlAttribute`: empty without a `url`, else the reference after
/// its required '#'.
fn read_url_attribute(node: &XmlNode) -> Result<Vec<u8>, ImportError> {
    let Some(url) = node.attribute("url") else {
        return Ok(Vec::new());
    };
    if at(url, 0) != b'#' {
        return fail("Unknown reference format");
    }
    Ok(after_hash(url))
}

/// `std::string::substr(1, size - 1)`, which throws on an empty string.
fn substring_from_one(value: &[u8]) -> Result<Vec<u8>, ImportError> {
    if value.is_empty() {
        return fail("invalid string position");
    }
    Ok(value[1..].to_vec())
}

/// `strtoul16`.
fn strtoul16(bytes: &[u8]) -> u32 {
    let mut value = 0u32;
    for &byte in bytes {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'A'..=b'F' => byte - b'A' + 10,
            b'a'..=b'f' => byte - b'a' + 10,
            _ => break,
        };
        value = (value << 4).wrapping_add(digit as u32);
    }
    value
}

/// `aiString(std::string)` + `UriDecodePath` + `C_Str()`.
fn uri_decode_path(path: &[u8]) -> Vec<u8> {
    let mut data = path[..path.len().min(1023)].to_vec();
    if data.starts_with(b"file://") {
        data.drain(..7);
    }
    if at(&data, 0) == b'/' && at(&data, 1).is_ascii_alphabetic() && at(&data, 2) == b':' {
        data.remove(0);
    }
    let mut decoded = Vec::with_capacity(data.len());
    let mut index = 0;
    while index < data.len() {
        if data[index] == b'%' && index + 3 < data.len() {
            let digits = [data[index + 1], data[index + 2]];
            decoded.push((strtoul16(&digits) & 0xFF) as u8);
            index += 3;
        } else {
            decoded.push(data[index]);
            index += 1;
        }
    }
    if let Some(end) = decoded.iter().position(|&byte| byte == 0) {
        decoded.truncate(end);
    }
    decoded
}

/// `GetTypeForSemantic`.
fn type_for_semantic(semantic: &[u8]) -> InputType {
    match semantic {
        b"POSITION" => InputType::Position,
        b"TEXCOORD" => InputType::Texcoord,
        b"NORMAL" => InputType::Normal,
        b"COLOR" => InputType::Color,
        b"VERTEX" => InputType::Vertex,
        b"BINORMAL" | b"TEXBINORMAL" => InputType::Bitangent,
        b"TANGENT" | b"TEXTANGENT" => InputType::Tangent,
        _ => InputType::Invalid,
    }
}

/// `ReadInputChannel`.
fn read_input_channel(node: &XmlNode, channels: &mut Vec<InputChannel>) -> Result<(), ImportError> {
    let mut channel = InputChannel::default();
    let semantic = attribute_string(node, "semantic");
    channel.kind = type_for_semantic(semantic);
    let source = attribute_string(node, "source");
    if at(source, 0) != b'#' {
        return fail("Unknown reference format in url in source attribute of <input> element.");
    }
    channel.accessor = after_hash(source);
    if let Some(offset) = attribute_uint(node, "offset") {
        channel.offset = offset as u64;
    }
    if (channel.kind == InputType::Texcoord || channel.kind == InputType::Color)
        && let Some(set) = attribute_uint(node, "set") {
            channel.index = set as u64;
        }
    if channel.kind != InputType::Invalid {
        channels.push(channel);
    }
    Ok(())
}

/// Reads `count` numbers with `fast_atoreal_move` (each preceded by
/// whitespace skipping when `skip_before`, followed by it otherwise).
fn read_floats(text: &[u8], count: usize, skip_before: bool) -> Result<Vec<f32>, ImportError> {
    let mut values = Vec::with_capacity(count);
    let mut position = 0;
    for _ in 0..count {
        if skip_before {
            position = skip_spaces_and_line_end(text, position);
        }
        let (value, end) = parse_real(text, position)?;
        values.push(value);
        position = end;
        if !skip_before {
            position = skip_spaces_and_line_end(text, position);
        }
    }
    Ok(values)
}

/// `fast_atoreal_move<ai_real>`, naming the text it fails on.
fn parse_real(text: &[u8], position: usize) -> Result<(f32, usize), ImportError> {
    fast_atoreal_move(text, position, true).map_err(|error| {
        let rest = &text[position.min(text.len())..];
        let shown = String::from_utf8_lossy(&rest[..rest.len().min(24)]);
        ImportError(format!("{} (at \"{shown}\")", error.0))
    })
}

/// `vector::resize` to a count from the file.
fn checked_count(count: u64) -> Result<usize, ImportError> {
    if count > MAX_ELEMENTS {
        return fail("bad allocation");
    }
    Ok(count as usize)
}

impl Parser {
    /// The parser constructor after the XML parse: a document without a
    /// `COLLADA` child leaves every library empty.
    pub fn new(document: &XmlNode) -> Result<Parser, ImportError> {
        let mut parser = Parser {
            format: FormatVersion::V1_5,
            unit_size: 1.0,
            up_direction: UpDirection::Y,
            data: BTreeMap::new(),
            accessors: BTreeMap::new(),
            meshes: BTreeMap::new(),
            nodes: Vec::new(),
            node_library: BTreeMap::new(),
            images: BTreeMap::new(),
            effects: BTreeMap::new(),
            materials: BTreeMap::new(),
            controllers: BTreeMap::new(),
            root: None,
            animations: vec![Animation::default()],
            root_animation: 0,
            animation_library: BTreeMap::new(),
            animation_clips: Vec::new(),
        };
        if let Some(collada) = document.child("COLLADA") {
            parser.read_contents(collada)?;
        }
        Ok(parser)
    }

    fn read_contents(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        if let Some(version) = node.attribute("version") {
            if version.starts_with(b"1.5") {
                self.format = FormatVersion::V1_5;
            } else if version.starts_with(b"1.4") {
                self.format = FormatVersion::V1_4;
            } else if version.starts_with(b"1.3") {
                self.format = FormatVersion::V1_3;
            }
        }
        self.read_structure(node)
    }

    fn read_structure(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        for child in &node.children {
            match child.name() {
                b"asset" => self.read_asset_info(child),
                b"library_animations" => self.read_animation_library(child)?,
                b"library_animation_clips" => self.read_animation_clip_library(child)?,
                b"library_controllers" => self.read_controller_library(child)?,
                b"library_images" => self.read_image_library(child),
                b"library_materials" => self.read_material_library(child)?,
                b"library_effects" => self.read_effect_library(child)?,
                b"library_geometries" => self.read_geometry_library(child)?,
                b"library_visual_scenes" => self.read_scene_library(child)?,
                b"library_lights" => self.read_light_library(child)?,
                b"library_nodes" => self.read_scene_node(child, None)?,
                b"scene" => self.read_scene(child)?,
                _ => {}
            }
        }
        self.post_process_root_animations();
        self.post_process_controllers()
    }

    /// `ReadAssetInfo`: the unit size and up axis.
    fn read_asset_info(&mut self, node: &XmlNode) {
        for child in &node.children {
            match child.name() {
                b"unit" => {
                    self.unit_size = 1.0;
                    if let Some(meter) = child.attribute("meter") {
                        // A parse error is only logged.
                        if let Ok((value, _)) = fast_atoreal_move(meter, 0, true) {
                            self.unit_size = value;
                        }
                    }
                }
                b"up_axis" => {
                    let axis = value_as_string(child);
                    self.up_direction = match axis.as_slice() {
                        b"X_UP" => UpDirection::X,
                        b"Z_UP" => UpDirection::Z,
                        _ => UpDirection::Y,
                    };
                }
                _ => {}
            }
        }
    }

    /// `ReadAnimationClipLibrary`, which reads `<instance_animation>`s
    /// straight under the library (and stores the clip after each child
    /// once it has one).
    fn read_animation_clip_library(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        let name = match (node.attribute("name"), node.attribute("id")) {
            (Some(name), _) => name.to_vec(),
            (None, Some(id)) => id.to_vec(),
            (None, None) => format!("animation_{}", self.animation_clips.len()).into_bytes(),
        };
        let mut urls = Vec::new();
        for child in &node.children {
            if child.name() == b"instance_animation" {
                urls.push(read_url_attribute(child)?);
            }
            if !urls.is_empty() {
                self.animation_clips.push((name.clone(), urls.clone()));
            }
        }
        Ok(())
    }

    /// `PostProcessControllers`: each controller's mesh id follows chains
    /// of controllers (a cycle never ends in the C++).
    fn post_process_controllers(&mut self) -> Result<(), ImportError> {
        let ids: Vec<Vec<u8>> = self.controllers.keys().cloned().collect();
        for id in ids {
            let mut mesh_id = self.controllers[&id].mesh_id.clone();
            if mesh_id.is_empty() {
                continue;
            }
            let mut visited = BTreeSet::new();
            while let Some(next) = self.controllers.get(&mesh_id) {
                if !visited.insert(mesh_id.clone()) {
                    return fail("Collada: cyclic controller references");
                }
                mesh_id = next.mesh_id.clone();
            }
            self.controllers.get_mut(&id).unwrap().mesh_id = mesh_id;
        }
        Ok(())
    }

    /// `Animation::CollectChannelsRecursively`.
    fn collect_channels(&self, animation: usize, channels: &mut Vec<AnimationChannel>) {
        channels.extend(self.animations[animation].channels.iter().cloned());
        for &sub in &self.animations[animation].sub_animations {
            self.collect_channels(sub, channels);
        }
    }

    /// `PostProcessRootAnimations`.
    fn post_process_root_animations(&mut self) {
        if self.animation_clips.is_empty() {
            let root = self.root_animation;
            self.combine_single_channel_animations(root);
            return;
        }
        let mut clips = Vec::new();
        for (name, urls) in self.animation_clips.clone() {
            let mut channels = Vec::new();
            for url in &urls {
                if let Some(&animation) = self.animation_library.get(url) {
                    self.collect_channels(animation, &mut channels);
                }
            }
            let clip = Animation { name, channels, sub_animations: Vec::new() };
            self.animations.push(clip);
            clips.push(self.animations.len() - 1);
        }
        let root = Animation { name: Vec::new(), channels: Vec::new(), sub_animations: clips };
        self.animations.push(root);
        self.root_animation = self.animations.len() - 1;
    }

    /// `CombineSingleChannelAnimationsRecursively` (whose name fallback
    /// writes the root's name, not the parent's).
    fn combine_single_channel_animations(&mut self, parent: usize) {
        let mut targets = BTreeSet::new();
        let mut different = true;
        let subs = self.animations[parent].sub_animations.clone();
        for &animation in &subs {
            let root = self.root_animation;
            if self.animations[root].name.is_empty() {
                self.animations[root].name = self.animations[animation].name.clone();
            }
            self.combine_single_channel_animations(animation);
            let channels = &self.animations[animation].channels;
            if different && channels.len() == 1 && !targets.contains(&channels[0].target) {
                targets.insert(channels[0].target.clone());
            } else {
                different = false;
            }
        }
        if different {
            for &animation in &subs {
                let channel = self.animations[animation].channels[0].clone();
                self.animations[parent].channels.push(channel);
            }
            self.animations[parent].sub_animations.clear();
        }
    }

    fn read_animation_library(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        for child in &node.children {
            if child.name() == b"animation" {
                let root = self.root_animation;
                self.read_animation(child, root)?;
            }
        }
        Ok(())
    }

    /// `ReadAnimation`: a container of sub-animations and/or channels
    /// (keyed by sampler id in a `std::map`).
    fn read_animation(&mut self, node: &XmlNode, parent: usize) -> Result<(), ImportError> {
        let name = node.attribute("name").unwrap_or(b"animation").to_vec();
        let id = node.attribute("id").map(<[u8]>::to_vec);
        let mut channels: BTreeMap<Vec<u8>, AnimationChannel> = BTreeMap::new();
        let mut animation: Option<usize> = None;
        for child in &node.children {
            match child.name() {
                b"animation" => {
                    let container = match animation {
                        Some(container) => container,
                        None => {
                            let container = self.new_animation(name.clone(), parent);
                            animation = Some(container);
                            container
                        }
                    };
                    self.read_animation(child, container)?;
                }
                b"source" => self.read_source(child)?,
                b"sampler" => {
                    if let Some(sampler_id) = child.attribute("id") {
                        let channel = channels.entry(sampler_id.to_vec()).or_default();
                        read_animation_sampler(child, channel)?;
                    }
                }
                b"channel" => {
                    let mut source = attribute_string(child, "source").to_vec();
                    let target = attribute_string(child, "target");
                    if at(&source, 0) == b'#' {
                        source.remove(0);
                    }
                    if let Some(channel) = channels.get_mut(&source) {
                        channel.target = target.to_vec();
                    }
                }
                _ => {}
            }
        }
        if !channels.is_empty() {
            let container = match animation {
                Some(container) => container,
                None => self.new_animation(name, parent),
            };
            self.animations[container].channels.extend(channels.into_values());
            if let Some(id) = id {
                self.animation_library.insert(id, container);
            }
        }
        Ok(())
    }

    fn new_animation(&mut self, name: Vec<u8>, parent: usize) -> usize {
        let animation = Animation { name, channels: Vec::new(), sub_animations: Vec::new() };
        self.animations.push(animation);
        let index = self.animations.len() - 1;
        self.animations[parent].sub_animations.push(index);
        index
    }

    fn read_controller_library(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        for child in &node.children {
            if child.name() != b"controller" {
                continue;
            }
            if let Some(id) = child.attribute("id") {
                let mut controller = Controller::default();
                self.read_controller(child, &mut controller)?;
                self.controllers.insert(id.to_vec(), controller);
            }
        }
        Ok(())
    }

    /// `ReadController` (pre-order over the controller's elements).
    fn read_controller(&mut self, node: &XmlNode, controller: &mut Controller) -> Result<(), ImportError> {
        for current in node.descendants() {
            match current.name() {
                b"morph" => {
                    controller.kind = ControllerType::Morph;
                    controller.mesh_id = substring_from_one(attribute_string(current, "source"))?;
                }
                b"skin" => {
                    if let Some(source) = current.attribute("source") {
                        controller.mesh_id = substring_from_one(source)?;
                    }
                }
                b"bind_shape_matrix" => {
                    let text = value_as_string(current);
                    let values = read_floats(&text, 16, true)?;
                    controller.bind_shape_matrix.copy_from_slice(&values);
                }
                b"source" => self.read_source(current)?,
                b"joints" => read_controller_joints(current, controller)?,
                b"vertex_weights" => read_controller_weights(current, controller)?,
                b"targets" => {
                    // The loop examines the controller's first child, not
                    // the targets' children.
                    let Some(first) = node.children.first() else {
                        continue;
                    };
                    if first.name() == b"input" {
                        let semantic = attribute_string(first, "semantic");
                        let source = after_hash(attribute_string(first, "source"));
                        if semantic == b"MORPH_TARGET" {
                            controller.morph_target = source;
                        } else if semantic == b"MORPH_WEIGHT" {
                            controller.morph_weight = source;
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn read_image_library(&mut self, node: &XmlNode) {
        for child in &node.children {
            if child.name() == b"image"
                && let Some(id) = child.attribute("id") {
                    let mut image = Image::default();
                    self.read_image(child, &mut image);
                    self.images.insert(id.to_vec(), image);
                }
        }
    }

    /// `ReadImage`: only whether a file name or embedded data results.
    fn read_image(&self, node: &XmlNode, image: &mut Image) {
        for child in &node.children {
            if child.name() != b"init_from" {
                continue;
            }
            if self.format == FormatVersion::V1_4 {
                image.file_name = uri_decode_path(child.text());
                if image.file_name.is_empty() {
                    image.file_name = b"unknown_texture".to_vec();
                }
            } else if self.format == FormatVersion::V1_5 {
                let reference = child.child("ref");
                let hex = child.child("hex");
                if let Some(reference) = reference {
                    image.file_name = uri_decode_path(&value_as_string(reference));
                } else if let Some(hex) = hex
                    && image.file_name.is_empty()
                {
                    let value = value_as_string(hex);
                    let digits = value.iter().take_while(|&&byte| !is_space_or_new_line(byte)).count();
                    image.data_size = digits * 2;
                }
            }
        }
    }

    /// `ReadMaterialLibrary`: every child node becomes a material, keyed
    /// by its (possibly missing) id.
    fn read_material_library(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        for child in &node.children {
            let id = attribute_string(child, "id").to_vec();
            let mut material = Material::default();
            for grandchild in &child.children {
                if grandchild.name() == b"instance_effect" {
                    material.effect = read_url_attribute(grandchild)?;
                }
            }
            self.materials.insert(id, material);
        }
        Ok(())
    }

    /// `ReadLightLibrary`: a light's `<color>` must hold three numbers.
    fn read_light_library(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        for child in &node.children {
            if child.name() != b"light" || child.attribute("id").is_none() {
                continue;
            }
            for current in child.descendants() {
                if current.name() == b"color" {
                    let text = value_as_string(current);
                    read_floats(&text, 3, false)?;
                }
            }
        }
        Ok(())
    }

    fn read_effect_library(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        for child in &node.children {
            if child.name() != b"effect" {
                continue;
            }
            let id = attribute_string(child, "id").to_vec();
            let mut effect = Effect::default();
            for profile in &child.children {
                if profile.name() == b"profile_COMMON" {
                    self.read_effect_profile_common(profile, &mut effect)?;
                }
            }
            self.effects.insert(id, effect);
        }
        Ok(())
    }

    /// `ReadEffectProfileCommon` (pre-order over the profile).
    fn read_effect_profile_common(&mut self, node: &XmlNode, effect: &mut Effect) -> Result<(), ImportError> {
        for current in node.descendants() {
            match current.name() {
                b"newparam" => {
                    let sid = attribute_string(current, "sid").to_vec();
                    let reference = self.read_effect_param(current)?;
                    effect.params.insert(sid, reference);
                }
                b"image" if self.format == FormatVersion::V1_4 => {
                    let id = attribute_string(current, "id").to_vec();
                    let mut image = Image::default();
                    self.read_image(current, &mut image);
                    self.images.insert(id, image);
                }
                b"emission" => read_effect_color(current, &mut effect.emissive)?,
                b"ambient" => read_effect_color(current, &mut effect.ambient)?,
                b"diffuse" => read_effect_color(current, &mut effect.diffuse)?,
                b"specular" => read_effect_color(current, &mut effect.specular)?,
                b"reflective" => read_effect_color(current, &mut effect.reflective)?,
                b"transparent" => read_effect_color(current, &mut effect.transparent)?,
                b"bump" => read_effect_color(current, &mut effect.bump)?,
                _ => {}
            }
        }
        Ok(())
    }

    /// `ReadEffectParam`: the reference a surface or sampler names.
    fn read_effect_param(&self, node: &XmlNode) -> Result<Vec<u8>, ImportError> {
        let mut reference = Vec::new();
        for child in &node.children {
            match child.name() {
                b"surface" => {
                    if let Some(init) = child.child("init_from") {
                        reference = value_as_string(init);
                    }
                }
                b"sampler2D" if self.format == FormatVersion::V1_4 || self.format == FormatVersion::V1_3 => {
                    if let Some(source) = child.child("source") {
                        reference = value_as_string(source);
                    }
                }
                b"sampler2D" => {
                    if let Some(image) = child.child("instance_image") {
                        let url = attribute_string(image, "url");
                        if at(url, 0) != b'#' {
                            return fail("Unsupported URL format in instance_image");
                        }
                        reference = after_hash(url);
                    }
                }
                _ => {}
            }
        }
        Ok(reference)
    }

    /// `ReadGeometryLibrary`: geometries with an id already read are
    /// skipped.
    fn read_geometry_library(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        for child in &node.children {
            if child.name() != b"geometry" {
                continue;
            }
            let id = attribute_string(child, "id").to_vec();
            if self.meshes.contains_key(&id) {
                continue;
            }
            let mut mesh = Mesh::new(id.clone());
            if let Some(name) = child.attribute("name") {
                mesh.name = name.to_vec();
            }
            for mesh_node in &child.children {
                if mesh_node.name() == b"mesh" {
                    self.read_mesh(mesh_node, &mut mesh)?;
                }
            }
            self.meshes.insert(id, mesh);
        }
        Ok(())
    }

    /// `ReadMesh` (pre-order over the mesh).
    fn read_mesh(&mut self, node: &XmlNode, mesh: &mut Mesh) -> Result<(), ImportError> {
        for current in node.descendants() {
            match current.name() {
                b"source" => self.read_source(current)?,
                b"vertices" => read_vertex_data(current, mesh)?,
                b"triangles" | b"lines" | b"linestrips" | b"polygons" | b"polylist" | b"trifans" | b"tristrips" => {
                    self.read_index_data(current, mesh)?
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// `ReadSource` (pre-order over the source).
    fn read_source(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        let source_id = attribute_string(node, "id").to_vec();
        for current in node.descendants() {
            match current.name() {
                b"float_array" | b"IDREF_array" | b"Name_array" => self.read_data_array(current)?,
                b"technique_common" => {
                    if let Some(accessor) = current.child("accessor") {
                        self.read_accessor(accessor, &source_id)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// `ReadDataArray`: `count` values or names (replacing any array with
    /// the same id).
    fn read_data_array(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        let is_string_array = node.name() == b"IDREF_array" || node.name() == b"Name_array";
        let id = attribute_string(node, "id").to_vec();
        let count = attribute_uint(node, "count").unwrap_or(0);
        let text = value_as_string(node);
        let mut data = Data { is_string_array, values: Vec::new(), strings: Vec::new() };
        let mut position = 0;
        checked_count(count as u64)?;
        for _ in 0..count {
            if at(&text, position) == 0 {
                return fail("Expected more values while reading array contents.");
            }
            if is_string_array {
                let start = position;
                while !is_space_or_new_line(at(&text, position)) {
                    position += 1;
                }
                data.strings.push(text[start..position].to_vec());
            } else {
                let (value, end) = parse_real(&text, position)?;
                data.values.push(value);
                position = end;
            }
            position = skip_spaces_and_line_end(&text, position);
        }
        self.data.insert(id, data);
        Ok(())
    }

    /// `ReadAccessor`, stored under the source's id.
    fn read_accessor(&mut self, node: &XmlNode, source_id: &[u8]) -> Result<(), ImportError> {
        let source = attribute_string(node, "source");
        if at(source, 0) != b'#' {
            return fail("Unknown reference format in url in source attribute of <accessor> element.");
        }
        let count = attribute_int(node, "count").unwrap_or(0);
        let offset = attribute_uint(node, "offset").unwrap_or(0);
        let stride = attribute_uint(node, "stride").unwrap_or(1);
        let mut accessor = Accessor {
            count: count as i64 as u64,
            size: 0,
            offset: offset as u64,
            stride: stride as u64,
            params: Vec::new(),
            sub_offset: [0; 4],
            source: after_hash(source),
            data_resolved: false,
        };
        for current in node.descendants() {
            if current.name() != b"param" {
                continue;
            }
            let mut name = Vec::new();
            if let Some(param_name) = current.attribute("name") {
                name = param_name.to_vec();
                let slot = match param_name {
                    b"X" | b"R" | b"S" | b"U" => Some(0),
                    b"Y" | b"G" | b"T" | b"V" => Some(1),
                    b"Z" | b"B" | b"P" => Some(2),
                    b"A" => Some(3),
                    _ => None,
                };
                if let Some(slot) = slot {
                    accessor.sub_offset[slot] = accessor.params.len() as u64;
                }
            }
            if let Some(kind) = current.attribute("type") {
                accessor.size += if kind == b"float4x4" { 16 } else { 1 };
            }
            accessor.params.push(name);
        }
        self.accessors.insert(source_id.to_vec(), accessor);
        Ok(())
    }

    /// `ReadIndexData`: one sub-mesh from a primitive element.
    fn read_index_data(&mut self, node: &XmlNode, mesh: &mut Mesh) -> Result<(), ImportError> {
        let mut vcount: Vec<u64> = Vec::new();
        let mut per_index: Vec<InputChannel> = Vec::new();
        let primitive_count = attribute_uint(node, "count").unwrap_or(0) as u64;
        let mut actual_primitives = 0u64;
        let material = node.attribute("material").unwrap_or(b"").to_vec();
        let kind = match node.name() {
            b"lines" => PrimitiveType::Lines,
            b"linestrips" => PrimitiveType::LineStrip,
            b"polygons" => PrimitiveType::Polygon,
            b"polylist" => PrimitiveType::Polylist,
            b"triangles" => PrimitiveType::Triangles,
            b"trifans" => PrimitiveType::TriFans,
            _ => PrimitiveType::TriStrips,
        };
        for current in node.descendants() {
            match current.name() {
                b"input" => read_input_channel(current, &mut per_index)?,
                b"vcount" => {
                    if primitive_count == 0 {
                        continue;
                    }
                    let text = value_as_string(current);
                    checked_count(primitive_count)?;
                    let mut position = skip_spaces_and_line_end(&text, 0);
                    for _ in 0..primitive_count {
                        if at(&text, position) == 0 {
                            return fail("Expected more values while reading <vcount> contents.");
                        }
                        let (value, end) = strtoul10(&text, position);
                        vcount.push(value as u64);
                        position = skip_spaces_and_line_end(&text, end);
                    }
                }
                b"p" => {
                    let count = self.read_primitives(current, mesh, &mut per_index, primitive_count, &vcount, kind)?;
                    actual_primitives = actual_primitives.wrapping_add(count);
                }
                b"extra" | b"ph" => {}
                _ => return fail("Unexpected sub element in primitive tag"),
            }
        }
        mesh.sub_meshes.push(SubMesh { material, face_count: actual_primitives });
        Ok(())
    }

    /// Resolves a channel's accessor and its data once, checking the data
    /// covers it.
    fn resolve_channel(&mut self, channel: &mut InputChannel) -> Result<(), ImportError> {
        let Some(accessor) = self.accessors.get_mut(&channel.accessor) else {
            return fail("Unable to resolve library reference.");
        };
        channel.resolved = true;
        if accessor.data_resolved {
            return Ok(());
        }
        let Some(data) = self.data.get(&accessor.source) else {
            return fail("Unable to resolve library reference.");
        };
        accessor.data_resolved = true;
        let data_size = accessor.offset.wrapping_add(accessor.count.wrapping_mul(accessor.stride));
        if data_size > data.values.len() as u64 {
            return fail("Not enough data for accessor");
        }
        Ok(())
    }

    /// `ReadPrimitives`: one `<p>`'s faces into the mesh streams.
    fn read_primitives(
        &mut self,
        node: &XmlNode,
        mesh: &mut Mesh,
        per_index: &mut [InputChannel],
        primitive_count: u64,
        vcount: &[u64],
        kind: PrimitiveType,
    ) -> Result<u64, ImportError> {
        let mut offset_count = 1u64;
        let mut per_vertex_offset = u64::MAX;
        for channel in per_index.iter() {
            offset_count = offset_count.max(channel.offset + 1);
            if channel.kind == InputType::Vertex {
                per_vertex_offset = channel.offset;
            }
        }
        let expected_points = match kind {
            PrimitiveType::Polylist => vcount.iter().fold(0u64, |sum, &count| sum.wrapping_add(count)),
            PrimitiveType::Lines => primitive_count.wrapping_mul(2),
            PrimitiveType::Triangles => primitive_count.wrapping_mul(3),
            _ => 0,
        };

        let mut indices: Vec<u64> = Vec::new();
        if primitive_count > 0 {
            let text = value_as_string(node);
            let mut position = skip_spaces_and_line_end(&text, 0);
            while at(&text, position) != 0 {
                let (value, end) = strtol10(&text, position);
                indices.push(value.max(0) as u64);
                let next = skip_spaces_and_line_end(&text, end);
                if next == position {
                    // No progress: the C++ pushes zeros until memory runs out.
                    return fail("bad allocation");
                }
                position = next;
            }
        }

        let mut primitive_count = primitive_count;
        let index_count = indices.len() as u64;
        if expected_points > 0 && index_count != expected_points.wrapping_mul(offset_count) {
            if kind == PrimitiveType::Lines {
                primitive_count = (index_count / offset_count) / 2;
            } else {
                return fail("Expected different index count in <p> element.");
            }
        } else if expected_points == 0 && !index_count.is_multiple_of(offset_count) {
            return fail("Expected different index count in <p> element.");
        }

        for channel in mesh.per_vertex_data.iter_mut() {
            if channel.resolved {
                continue;
            }
            self.resolve_channel(channel)?;
        }
        for channel in per_index.iter_mut() {
            if channel.resolved {
                continue;
            }
            if channel.kind == InputType::Vertex {
                if channel.accessor != mesh.vertex_id {
                    return fail("Unsupported vertex referencing scheme.");
                }
                continue;
            }
            self.resolve_channel(channel)?;
        }

        let vertex_count = index_count / offset_count;
        let face_count = match kind {
            PrimitiveType::TriFans | PrimitiveType::Polygon => 1,
            PrimitiveType::TriStrips => vertex_count.wrapping_sub(2),
            PrimitiveType::LineStrip => vertex_count.wrapping_sub(1),
            _ => primitive_count,
        };
        // `mFaceSize.reserve` throws `length_error` for wrapped counts.
        checked_count(face_count)?;

        let per_vertex = std::mem::take(&mut mesh.per_vertex_data);
        let layout = IndexLayout { offset_count, per_vertex_offset, indices: &indices, per_vertex: &per_vertex, per_index };
        let mut polylist_start = 0u64;
        for primitive in 0..face_count {
            let point_count = match kind {
                PrimitiveType::Lines => {
                    for vertex in 0..2 {
                        self.copy_vertex(&layout, vertex, 2, primitive, mesh)?;
                    }
                    2
                }
                PrimitiveType::LineStrip => {
                    for vertex in 0..2 {
                        self.copy_vertex(&layout, vertex, 1, primitive, mesh)?;
                    }
                    2
                }
                PrimitiveType::Triangles => {
                    for vertex in 0..3 {
                        self.copy_vertex(&layout, vertex, 3, primitive, mesh)?;
                    }
                    3
                }
                PrimitiveType::TriStrips => {
                    let order: [u64; 3] = if primitive % 2 != 0 { [1, 0, 2] } else { [0, 1, 2] };
                    for vertex in order {
                        self.copy_vertex(&layout, vertex, 1, primitive, mesh)?;
                    }
                    3
                }
                PrimitiveType::Polylist => {
                    let Some(&point_count) = vcount.get(primitive as usize) else {
                        return fail("Collada: polylist without <vcount>");
                    };
                    for vertex in 0..point_count {
                        self.copy_vertex(&layout, polylist_start + vertex, 1, 0, mesh)?;
                    }
                    polylist_start += point_count;
                    point_count
                }
                PrimitiveType::TriFans | PrimitiveType::Polygon => {
                    let point_count = vertex_count;
                    for vertex in 0..point_count {
                        self.copy_vertex(&layout, vertex, point_count, primitive, mesh)?;
                    }
                    point_count
                }
            };
            mesh.face_sizes.push(point_count);
        }
        mesh.per_vertex_data = per_vertex;
        Ok(face_count)
    }

    /// `CopyVertex`.
    fn copy_vertex(&self, layout: &IndexLayout, vertex: u64, point_count: u64, primitive: u64, mesh: &mut Mesh) -> Result<(), ImportError> {
        let primitive_base = primitive.wrapping_mul(layout.offset_count).wrapping_mul(point_count);
        let base = primitive_base.wrapping_add(vertex.wrapping_mul(layout.offset_count));
        let index_at = |offset: u64| layout.indices.get(base.wrapping_add(offset) as usize).copied();
        let position_index = index_at(layout.per_vertex_offset);
        for channel in layout.per_vertex {
            let Some(local_index) = position_index else {
                return fail("Collada: vertex index outside the index list");
            };
            self.extract_data_object(channel, local_index, mesh)?;
        }
        for channel in layout.per_index {
            let Some(local_index) = index_at(channel.offset) else {
                return fail("Collada: vertex index outside the index list");
            };
            self.extract_data_object(channel, local_index, mesh)?;
        }
        let face_position = match position_index {
            Some(index) => index,
            // Only `indices[-1]` (no VERTEX input, first vertex) reads
            // outside; anything else runs past the end of the list.
            None if base == 0 && layout.per_vertex_offset == u64::MAX => GARBAGE_INDEX,
            None => return fail("Collada: vertex index outside the index list"),
        };
        mesh.face_position_indices.push(face_position);
        Ok(())
    }

    /// `ExtractDataObjectFromChannel`.
    fn extract_data_object(&self, channel: &InputChannel, local_index: u64, mesh: &mut Mesh) -> Result<(), ImportError> {
        if channel.kind == InputType::Vertex {
            return Ok(());
        }
        let Some(accessor) = self.accessors.get(&channel.accessor) else {
            return fail("Collada: unresolved input channel");
        };
        if local_index >= accessor.count {
            return fail("Invalid data index in primitive specification");
        }
        if !accessor.data_resolved {
            return fail("Collada: accessor redefined after use");
        }
        let data = &self.data[&accessor.source];
        let base = accessor.offset.wrapping_add(local_index.wrapping_mul(accessor.stride));
        let mut object = [0.0f32; 4];
        for (component, value) in object.iter_mut().enumerate() {
            let position = base.wrapping_add(accessor.sub_offset[component]);
            let Some(&read) = data.values.get(position as usize) else {
                return fail("Collada: accessor reads outside its data");
            };
            *value = read;
        }
        let position_count = mesh.positions.len() as u64;
        let target = position_count.wrapping_sub(1);
        match channel.kind {
            InputType::Position => mesh.positions.push([object[0], object[1], object[2]]),
            InputType::Normal => pad_and_push(&mut mesh.normals, target, [0.0, 1.0, 0.0], [object[0], object[1], object[2]])?,
            InputType::Tangent => pad_and_push(&mut mesh.tangents, target, [1.0, 0.0, 0.0], [object[0], object[1], object[2]])?,
            InputType::Bitangent => pad_and_push(&mut mesh.bitangents, target, [0.0, 0.0, 1.0], [object[0], object[1], object[2]])?,
            InputType::Texcoord => {
                if channel.index < MAX_TEXTURE_COORDS as u64 {
                    let set = channel.index as usize;
                    pad_and_push(&mut mesh.texture_coords[set], target, [0.0; 3], [object[0], object[1], object[2]])?;
                    if accessor.sub_offset[2] != 0 || accessor.sub_offset[3] != 0 {
                        mesh.uv_components[set] = 3;
                    }
                }
            }
            InputType::Color => {
                if channel.index < MAX_COLOR_SETS as u64 {
                    let mut color = [0.0, 0.0, 0.0, 1.0];
                    for component in 0..accessor.size {
                        let source = accessor.sub_offset.get(component as usize).copied().unwrap_or(u64::MAX);
                        if component >= 4 || source >= 4 {
                            return fail("Collada: colour accessor outside its object");
                        }
                        color[component as usize] = object[source as usize];
                    }
                    let set = channel.index as usize;
                    pad_and_push(&mut mesh.colors[set], target, [0.0, 0.0, 0.0, 1.0], color)?;
                }
            }
            InputType::Invalid | InputType::Vertex => {}
        }
        Ok(())
    }

    /// `ReadSceneLibrary`.
    fn read_scene_library(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        for child in &node.children {
            if child.name() != b"visual_scene" {
                continue;
            }
            let id = attribute_string(child, "id").to_vec();
            let name = child.attribute("name").unwrap_or(b"Scene").to_vec();
            let scene_node = Node { name, id: id.clone(), ..Node::default() };
            self.nodes.push(scene_node);
            let index = self.nodes.len() - 1;
            self.node_library.insert(id, index);
            self.read_scene_node(child, Some(index))?;
        }
        Ok(())
    }

    /// `ReadSceneNode`; without a parent (`<library_nodes>`), only child
    /// nodes are read, into the node library.
    fn read_scene_node(&mut self, node: &XmlNode, parent: Option<usize>) -> Result<(), ImportError> {
        for child in &node.children {
            let name = child.name();
            if name == b"node" {
                let mut collada_node = Node::default();
                if let Some(id) = child.attribute("id") {
                    collada_node.id = id.to_vec();
                }
                if let Some(sid) = child.attribute("sid") {
                    collada_node.sid = sid.to_vec();
                }
                if let Some(node_name) = child.attribute("name") {
                    collada_node.name = node_name.to_vec();
                }
                collada_node.parent = parent;
                let id = collada_node.id.clone();
                self.nodes.push(collada_node);
                let index = self.nodes.len() - 1;
                match parent {
                    Some(parent) => self.nodes[parent].children.push(index),
                    None => {
                        self.node_library.insert(id, index);
                    }
                }
                self.read_scene_node(child, Some(index))?;
                continue;
            }
            let Some(parent) = parent else {
                continue;
            };
            let transform = match name {
                b"lookat" => Some(TransformType::LookAt),
                b"matrix" => Some(TransformType::Matrix),
                b"rotate" => Some(TransformType::Rotate),
                b"scale" => Some(TransformType::Scale),
                b"skew" => Some(TransformType::Skew),
                b"translate" => Some(TransformType::Translate),
                _ => None,
            };
            if let Some(kind) = transform {
                let id = child.attribute("sid").unwrap_or(b"").to_vec();
                let text = value_as_string(child);
                let parsed = read_floats(&text, kind.parameter_count(), true)?;
                let mut values = [0.0f32; 16];
                values[..parsed.len()].copy_from_slice(&parsed);
                self.nodes[parent].transforms.push(Transform { id, kind, values });
                continue;
            }
            match name {
                b"instance_node" => {
                    if let Some(url) = child.attribute("url") {
                        // A reference without '#' is only logged.
                        if at(url, 0) == b'#' {
                            self.nodes[parent].node_instances.push(after_hash(url));
                        }
                    }
                }
                b"instance_geometry" | b"instance_controller" => {
                    let instance = read_node_geometry(child)?;
                    self.nodes[parent].meshes.push(instance);
                }
                b"instance_light" | b"instance_camera" => {
                    if let Some(url) = child.attribute("url")
                        && at(url, 0) != b'#' {
                            return fail("Unknown reference format in instance element");
                        }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// `ReadScene`: the single instanced visual scene becomes the root.
    fn read_scene(&mut self, node: &XmlNode) -> Result<(), ImportError> {
        for child in &node.children {
            if child.name() != b"instance_visual_scene" {
                continue;
            }
            if self.root.is_some() {
                return fail("Invalid scene containing multiple root nodes in <instance_visual_scene> element");
            }
            let url = attribute_string(child, "url");
            if at(url, 0) != b'#' {
                return fail("Unknown reference format in <instance_visual_scene> element");
            }
            let Some(&scene) = self.node_library.get(&after_hash(url)) else {
                return fail("Unable to resolve visual_scene reference in <instance_visual_scene> element.");
            };
            self.root = Some(scene);
        }
        Ok(())
    }
}

/// The index list, its layout and the channels `CopyVertex` reads.
struct IndexLayout<'a> {
    offset_count: u64,
    per_vertex_offset: u64,
    indices: &'a [u64],
    per_vertex: &'a [InputChannel],
    per_index: &'a [InputChannel],
}

/// The padding `ExtractDataObjectFromChannel` does before a push: up to
/// one less than the position count (a wrapped count makes `insert`
/// throw `length_error`).
fn pad_and_push<T: Copy>(stream: &mut Vec<T>, target: u64, padding: T, value: T) -> Result<(), ImportError> {
    let length = stream.len() as u64;
    if length < target {
        let missing = target - length;
        if missing > MAX_ELEMENTS {
            return fail("vector too long");
        }
        stream.extend(std::iter::repeat_n(padding, missing as usize));
    }
    stream.push(value);
    Ok(())
}

/// `ReadVertexData`: every child of `<vertices>` must be an `<input>`.
fn read_vertex_data(node: &XmlNode, mesh: &mut Mesh) -> Result<(), ImportError> {
    if let Some(id) = node.attribute("id") {
        mesh.vertex_id = id.to_vec();
    }
    for child in &node.children {
        if child.name() == b"input" {
            read_input_channel(child, &mut mesh.per_vertex_data)?;
        } else {
            return fail("Unexpected sub element in tag <vertices>");
        }
    }
    Ok(())
}

/// `ReadAnimationSampler`: the time and value sources.
fn read_animation_sampler(node: &XmlNode, channel: &mut AnimationChannel) -> Result<(), ImportError> {
    for child in &node.children {
        if child.name() != b"input" {
            continue;
        }
        let Some(semantic) = child.attribute("semantic") else {
            continue;
        };
        let Some(source) = child.attribute("source") else {
            continue;
        };
        if at(source, 0) != b'#' {
            return fail("Unsupported URL format");
        }
        match semantic {
            b"INPUT" => channel.source_times = after_hash(source),
            b"OUTPUT" => channel.source_values = after_hash(source),
            _ => {}
        }
    }
    Ok(())
}

/// `ReadControllerJoints`.
fn read_controller_joints(node: &XmlNode, controller: &mut Controller) -> Result<(), ImportError> {
    for child in &node.children {
        if child.name() != b"input" {
            continue;
        }
        let semantic = attribute_string(child, "semantic");
        let source = attribute_string(child, "source");
        if at(source, 0) != b'#' {
            return fail("Unsupported URL format in source attribute of <joints> data <input> element");
        }
        match semantic {
            b"JOINT" => controller.joint_name_source = after_hash(source),
            b"INV_BIND_MATRIX" => controller.joint_offset_matrix_source = after_hash(source),
            _ => return fail("Unknown semantic in <joints> data <input> element"),
        }
    }
    Ok(())
}

/// `ReadControllerWeights`: the `<vcount>` text is not trimmed, so a
/// leading space reads as a zero count.
fn read_controller_weights(node: &XmlNode, controller: &mut Controller) -> Result<(), ImportError> {
    let vertex_count = attribute_int(node, "count").unwrap_or(0);
    if vertex_count < 0 {
        return fail("vector too long");
    }
    let vertex_count_size = checked_count(vertex_count as u64)?;
    controller.weight_counts.resize(vertex_count_size, 0);
    for child in &node.children {
        match child.name() {
            b"input" => {
                let semantic = attribute_string(child, "semantic");
                let source = attribute_string(child, "source");
                let offset = attribute_int(child, "offset").unwrap_or(0);
                if at(source, 0) != b'#' {
                    return fail("Unsupported URL format in source attribute of <vertex_weights> data <input> element");
                }
                let channel = InputChannel { offset: offset as i64 as u64, accessor: after_hash(source), ..InputChannel::default() };
                match semantic {
                    b"JOINT" => controller.weight_input_joints = channel,
                    b"WEIGHT" => controller.weight_input_weights = channel,
                    _ => return fail("Unknown semantic in <vertex_weights> data <input> element"),
                }
            }
            b"vcount" if vertex_count > 0 => {
                let text = child.text();
                let mut position = 0;
                let mut weight_count = 0u64;
                for count in controller.weight_counts.iter_mut() {
                    if at(text, position) == 0 {
                        return fail("Out of data while reading <vcount>");
                    }
                    let (value, end) = strtoul10(text, position);
                    *count = value as u64;
                    weight_count += value as u64;
                    position = skip_spaces_and_line_end(text, end);
                }
                let weight_count_size = checked_count(weight_count)?;
                controller.weights.resize(weight_count_size, (0, 0));
            }
            b"v" if vertex_count > 0 => {
                let text = value_as_string(child);
                let mut position = 0;
                for pair in controller.weights.iter_mut() {
                    position = skip_spaces_and_line_end(&text, position);
                    let (joint, end) = strtoul10(&text, position);
                    position = skip_spaces_and_line_end(&text, end);
                    if at(&text, position) == 0 {
                        return fail("Out of data while reading <vertex_weights>");
                    }
                    let (weight, end) = strtoul10(&text, position);
                    position = skip_spaces_and_line_end(&text, end);
                    *pair = (joint as u64, weight as u64);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// `ReadEffectColor` (pre-order): a `<color>` must hold four numbers; a
/// `<texture>` names the sampler.
fn read_effect_color(node: &XmlNode, sampler: &mut Vec<u8>) -> Result<(), ImportError> {
    for current in node.descendants() {
        match current.name() {
            b"color" => {
                let text = value_as_string(current);
                read_floats(&text, 4, false)?;
            }
            b"texture" => {
                if let Some(texture) = current.attribute("texture") {
                    *sampler = texture.to_vec();
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// `ReadNodeGeometry`: the referenced mesh or controller and its material
/// bindings (a binding without `target` reuses the previous URL).
fn read_node_geometry(node: &XmlNode) -> Result<MeshInstance, ImportError> {
    let mut url = attribute_string(node, "url").to_vec();
    if at(&url, 0) != b'#' {
        return fail("Unknown reference format");
    }
    let mut instance = MeshInstance { mesh_or_controller: after_hash(&url), materials: BTreeMap::new() };
    for child in &node.children {
        if child.name() != b"bind_material" {
            continue;
        }
        let Some(technique) = child.child("technique_common") else {
            continue;
        };
        let Some(first) = technique.children.iter().position(|sibling| sibling.name() == b"instance_material") else {
            continue;
        };
        for binding in &technique.children[first..] {
            if binding.name() != b"instance_material" {
                continue;
            }
            let group = attribute_string(binding, "symbol").to_vec();
            if let Some(target) = binding.attribute("target") {
                url = target.to_vec();
            }
            let material = if at(&url, 0) == b'#' { after_hash(&url) } else { url.clone() };
            let has_inputs = binding.children.iter().any(|input| input.name() == b"bind_vertex_input");
            instance.materials.insert(group, MaterialBinding { material, has_inputs });
        }
    }
    Ok(instance)
}
