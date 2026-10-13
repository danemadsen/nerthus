//! `glTF2::Asset` loading (glTF2Asset.inl): the JSON document, the GLB
//! container, and the objects the default scene, the skins and the
//! animations reach, each read once in Assimp's order (a node's children
//! before its mesh) with its type checks.

use std::collections::{HashMap, HashSet};

use crate::base64;
use crate::io::FileSystem;
use crate::json::{self, Value};
use crate::scene::{ImportError, fail};

pub const COMPONENT_BYTE: i32 = 5120;
pub const COMPONENT_UNSIGNED_BYTE: i32 = 5121;
pub const COMPONENT_SHORT: i32 = 5122;
pub const COMPONENT_UNSIGNED_SHORT: i32 = 5123;
pub const COMPONENT_UNSIGNED_INT: i32 = 5125;
pub const COMPONENT_FLOAT: i32 = 5126;

pub const MODE_POINTS: i32 = 0;
pub const MODE_LINES: i32 = 1;
pub const MODE_LINE_LOOP: i32 = 2;
pub const MODE_LINE_STRIP: i32 = 3;
pub const MODE_TRIANGLES: i32 = 4;
pub const MODE_TRIANGLE_STRIP: i32 = 5;
pub const MODE_TRIANGLE_FAN: i32 = 6;

pub const PATH_TRANSLATION: i32 = 0;
pub const PATH_ROTATION: i32 = 1;
pub const PATH_SCALE: i32 = 2;
pub const PATH_WEIGHTS: i32 = 3;

/// `ComponentTypeSize`.
pub fn component_size(component_type: i32) -> Result<usize, ImportError> {
    match component_type {
        COMPONENT_SHORT | COMPONENT_UNSIGNED_SHORT => Ok(2),
        COMPONENT_UNSIGNED_INT | COMPONENT_FLOAT => Ok(4),
        COMPONENT_BYTE | COMPONENT_UNSIGNED_BYTE => Ok(1),
        _ => fail(format!("GLTF: Unsupported Component Type {component_type}")),
    }
}

/// `AttribType::FromString` → component count (unknown names are SCALAR).
fn type_components(name: &[u8]) -> u32 {
    match name {
        b"VEC2" => 2,
        b"VEC3" => 3,
        b"VEC4" | b"MAT2" => 4,
        b"MAT3" => 9,
        b"MAT4" => 16,
        _ => 1,
    }
}

fn unexpected_type<T>(expected: &str, member: &str) -> Result<T, ImportError> {
    fail(format!("GLTF: Member \"{member}\" was not a {expected} value"))
}

/// The `Find*InContext` helpers: absent members are fine, members of the
/// wrong type fail the import (non-objects have no members).
pub fn find_string<'a>(value: &'a Value, member: &str) -> Result<Option<&'a Value>, ImportError> {
    find_checked(value, member, Value::is_string, "string")
}

pub fn find_number<'a>(value: &'a Value, member: &str) -> Result<Option<&'a Value>, ImportError> {
    find_checked(value, member, Value::is_number, "number")
}

pub fn find_uint<'a>(value: &'a Value, member: &str) -> Result<Option<&'a Value>, ImportError> {
    find_checked(value, member, Value::is_uint, "uint")
}

pub fn find_array<'a>(value: &'a Value, member: &str) -> Result<Option<&'a Value>, ImportError> {
    find_checked(value, member, Value::is_array, "array")
}

pub fn find_object<'a>(value: &'a Value, member: &str) -> Result<Option<&'a Value>, ImportError> {
    find_checked(value, member, Value::is_object, "object")
}

fn find_checked<'a>(value: &'a Value, member: &str, check: fn(&Value) -> bool, expected: &str) -> Result<Option<&'a Value>, ImportError> {
    if !value.is_object() {
        return Ok(None);
    }
    let Some(found) = value.member(member) else {
        return Ok(None);
    };
    if !check(found) {
        return unexpected_type(expected, member);
    }
    Ok(Some(found))
}

/// `FindExtensionInContext`.
fn find_extension<'a>(value: &'a Value, extension: &str) -> Result<Option<&'a Value>, ImportError> {
    let Some(list) = find_object(value, "extensions")? else {
        return Ok(None);
    };
    find_object(list, extension)
}

/// `ReadMember` with the generic `ReadHelper` (an `Int` value cast).
fn read_int(object: &Value, member: &str) -> Option<i32> {
    if !object.is_object() {
        return None;
    }
    let value = object.member(member)?;
    if !value.is_int() {
        return None;
    }
    Some(value.get_int())
}

/// `MemberOrDefault<size_t>`: on Windows `size_t` is `uint64_t`, read with
/// `IsUint64`.
fn read_size(object: &Value, member: &str, default: usize) -> usize {
    if !object.is_object() {
        return default;
    }
    match object.member(member) {
        Some(value) if value.is_uint64() => value.get_uint64() as usize,
        _ => default,
    }
}

/// `ReadMember<const char*>` (the string up to a NUL).
fn read_c_string<'a>(object: &'a Value, member: &str) -> Option<&'a [u8]> {
    if !object.is_object() {
        return None;
    }
    let value = object.member(member)?;
    if !value.is_string() {
        return None;
    }
    Some(value.c_str())
}

/// `glTFCommon::Util::ParseDataURI`: `data:[type][;charset=…][;base64],…`.
pub(crate) struct DataUri<'a> {
    pub base64: bool,
    pub media_type: &'a [u8],
    pub data: &'a [u8],
}

pub(crate) fn parse_data_uri(uri: &[u8]) -> Option<DataUri<'_>> {
    if !uri.starts_with(b"data:") {
        return None;
    }
    let length = uri.len();
    let mut position = 5;
    let mut base64 = false;
    let mut media_type: &[u8] = b"text/plain";
    if position < length && uri[position] != b';' && uri[position] != b',' {
        while position < length && uri[position] != b';' && uri[position] != b',' {
            position += 1;
        }
        media_type = &uri[5..position];
    }
    while position < length && uri[position] == b';' {
        position += 1;
        let start = position;
        while position < length && uri[position] != b';' && uri[position] != b',' {
            position += 1;
        }
        if uri[start..].starts_with(b"base64") {
            base64 = true;
        }
    }
    if position < length {
        position += 1;
        return Some(DataUri { base64, media_type, data: &uri[position..] });
    }
    // No comma: Assimp resets the parameters and points the data just past
    // "data:".
    Some(DataUri { base64: false, media_type: b"text/plain", data: &uri[5.min(length)..] })
}

/// A lazily read dictionary (`LazyDict<T>`).
struct Dict<'a, T> {
    name: &'static str,
    array: Option<&'a Value>,
    objects: Vec<T>,
    by_original: HashMap<u32, usize>,
    in_progress: HashSet<u32>,
}

enum Lookup<'a> {
    Loaded(usize),
    Read(&'a Value),
}

impl<'a, T> Dict<'a, T> {
    fn new(name: &'static str) -> Self {
        Dict { name, array: None, objects: Vec::new(), by_original: HashMap::new(), in_progress: HashSet::new() }
    }

    /// `AttachToDocument` for a top-level dictionary.
    fn attach(&mut self, document: &'a Value) -> Result<(), ImportError> {
        self.array = find_array(document, self.name)?;
        Ok(())
    }

    /// The first half of `Retrieve`: an already read object, or the JSON
    /// object to read (bounds, type and recursion checked).
    fn lookup(&mut self, index: u32) -> Result<Lookup<'a>, ImportError> {
        if let Some(&slot) = self.by_original.get(&index) {
            return Ok(Lookup::Loaded(slot));
        }
        let Some(array) = self.array else {
            return fail(format!("GLTF: Missing section \"{}\"", self.name));
        };
        let elements = array.elements();
        if index as usize >= elements.len() {
            return fail(format!("GLTF: Array index {index} is out of bounds ({}) for \"{}\"", elements.len(), self.name));
        }
        let object = &elements[index as usize];
        if !object.is_object() {
            return fail(format!("GLTF: Object at index {index} in array \"{}\" is not a JSON object", self.name));
        }
        if self.in_progress.contains(&index) {
            return fail(format!("GLTF: Object at index {index} in array \"{}\" has recursive reference to itself", self.name));
        }
        self.in_progress.insert(index);
        Ok(Lookup::Read(object))
    }

    /// The second half: `ReadExtensions`/`ReadExtras` checks, then `Add`.
    fn add(&mut self, index: u32, object: &Value, value: T) -> Result<usize, ImportError> {
        find_object(object, "extensions")?;
        find_object(object, "extras")?;
        let slot = self.objects.len();
        self.objects.push(value);
        self.by_original.insert(index, slot);
        self.in_progress.remove(&index);
        Ok(slot)
    }

    /// `Create` (the GLB body buffer).
    fn create(&mut self, value: T) -> usize {
        let slot = self.objects.len();
        self.objects.push(value);
        self.by_original.insert(slot as u32, slot);
        slot
    }

    pub fn objects(&self) -> &[T] {
        &self.objects
    }
}

pub struct Buffer {
    pub byte_length: usize,
    pub data: Option<Vec<u8>>,
}

pub struct BufferView {
    pub buffer: usize,
    pub byte_offset: usize,
    pub byte_length: usize,
    pub byte_stride: u32,
}

pub struct Accessor {
    pub id: String,
    pub buffer_view: Option<usize>,
    pub byte_offset: usize,
    pub component_type: i32,
    pub count: u32,
    pub components: u32,
    /// The patched copy of a sparse accessor's data.
    pub sparse: Option<Vec<u8>>,
}

#[derive(Default)]
pub struct Attributes {
    pub position: Vec<Option<usize>>,
    pub normal: Vec<Option<usize>>,
    pub tangent: Vec<Option<usize>>,
    pub texcoord: Vec<Option<usize>>,
    pub color: Vec<Option<usize>>,
    pub joint: Vec<Option<usize>>,
    pub jointmatrix: Vec<Option<usize>>,
    pub weight: Vec<Option<usize>>,
}

#[derive(Default)]
pub struct Target {
    pub position: Vec<Option<usize>>,
    pub normal: Vec<Option<usize>>,
    pub tangent: Vec<Option<usize>>,
}

pub struct Primitive {
    pub mode: i32,
    pub indices: Option<usize>,
    pub material: Option<usize>,
    pub attributes: Attributes,
    pub targets: Vec<Target>,
}

pub struct Mesh {
    pub id: String,
    pub name: String,
    pub primitives: Vec<Primitive>,
}

pub struct Node {
    pub mesh: Option<usize>,
    pub skin: Option<u32>,
    pub children: Vec<usize>,
}

pub struct Skin {
    pub inverse_bind_matrices: Option<usize>,
    pub joints: Vec<usize>,
}

pub struct AnimationSampler {
    pub input: Option<usize>,
    pub output: Option<usize>,
}

pub struct Channel {
    pub sampler: i64,
    pub node: Option<usize>,
    pub path: i32,
}

pub struct Animation {
    pub samplers: Vec<AnimationSampler>,
    pub channels: Vec<Channel>,
}

pub struct SceneDefinition {
    pub nodes: Vec<usize>,
}

#[derive(Default)]
struct ExtensionsUsed {
    specular_glossiness: bool,
    specular: bool,
    lights_punctual: bool,
    texture_transform: bool,
    sheen: bool,
    clearcoat: bool,
    transmission: bool,
    volume: bool,
    ior: bool,
    emissive_strength: bool,
}

pub struct Asset<'a> {
    directory: String,
    file_system: FileSystem,
    extensions: ExtensionsUsed,
    accessors: Dict<'a, Accessor>,
    animations: Dict<'a, Animation>,
    buffers: Dict<'a, Buffer>,
    buffer_views: Dict<'a, BufferView>,
    cameras: Dict<'a, ()>,
    lights: Dict<'a, ()>,
    images: Dict<'a, ()>,
    materials: Dict<'a, ()>,
    meshes: Dict<'a, Mesh>,
    nodes: Dict<'a, Node>,
    samplers: Dict<'a, ()>,
    scenes: Dict<'a, SceneDefinition>,
    skins: Dict<'a, Skin>,
    textures: Dict<'a, ()>,
    pub scene: Option<usize>,
}

/// The JSON text and the GLB body of a file, before parsing.
pub struct Document {
    pub json: Vec<u8>,
    pub body: Option<Vec<u8>>,
    pub binary: bool,
}

/// `CheckMagicToken(…, "glTF", …)`: also accepts the byte-swapped token.
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && (&bytes[..4] == b"glTF" || &bytes[..4] == b"FTlg")
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes(slice.try_into().unwrap()))
}

/// `ReadDocument`: the JSON (with the GLB header and chunks checked) and
/// the BIN chunk (`LoadFromStream`), length checks included.
pub fn read_document(bytes: &[u8]) -> Result<Document, ImportError> {
    if !is_binary(bytes) {
        if bytes.len() >= u32::MAX as usize {
            return fail("GLTF: JSON size greater than 4GB");
        }
        if bytes.len() < 2 {
            return fail("GLTF: No JSON file contents");
        }
        return Ok(Document { json: bytes.to_vec(), body: None, binary: false });
    }
    if bytes.len() < 12 {
        return fail("GLTF: Unable to read the file header");
    }
    if &bytes[..4] != b"glTF" {
        return fail("GLTF: Invalid binary glTF file");
    }
    let version = read_u32(bytes, 4).unwrap();
    let total_length = read_u32(bytes, 8).unwrap() as usize;
    if version != 2 {
        return fail("GLTF: Unsupported binary glTF version");
    }
    let (Some(json_length), Some(json_type)) = (read_u32(bytes, 12), read_u32(bytes, 16)) else {
        return fail("GLTF: Unable to read JSON chunk");
    };
    if json_type != 0x4E4F_534A {
        return fail("GLTF: JSON chunk missing");
    }
    let json_length = json_length as usize;
    let Some(json) = bytes.get(20..20 + json_length) else {
        return fail("GLTF: Could not read the file contents");
    };
    let padding = ((json_length + 3) & !3) - json_length;
    let body_offset = 12 + 8 + json_length + padding + 8;
    let mut body_length = 0usize;
    if total_length >= body_offset {
        let chunk = 20 + json_length + padding;
        let (Some(length), Some(kind)) = (read_u32(bytes, chunk), read_u32(bytes, chunk + 4)) else {
            return fail("GLTF: Unable to read BIN chunk");
        };
        if kind != 0x004E_4942 {
            return fail("GLTF: BIN chunk missing");
        }
        body_length = length as usize;
    }
    if json_length < 2 {
        return fail("GLTF: No JSON file contents");
    }
    let mut body = None;
    if body_length > 0 {
        if body_length > bytes.len() {
            return fail("GLTF: Invalid byteLength exceeds size of actual data.");
        }
        let Some(data) = bytes.get(body_offset..body_offset + body_length) else {
            return fail("GLTF: Unable to read gltf file");
        };
        body = Some(data.to_vec());
    }
    Ok(Document { json: json.to_vec(), body, binary: true })
}

/// `ParseInsitu` and the root check.
pub fn parse_json(json: &[u8]) -> Result<Value, ImportError> {
    let value = match json::parse(json) {
        Ok(value) => value,
        Err(error) => return fail(format!("GLTF: JSON parse error, offset {}: {}", error.offset, error.code)),
    };
    if !value.is_object() {
        return fail("GLTF: JSON document root must be a JSON object");
    }
    Ok(value)
}

/// `AssetMetadata::Read`: the version must start with '2'.
pub fn check_version(document: &Value) -> Result<(), ImportError> {
    let mut version: &[u8] = &[];
    if let Some(asset) = find_object(document, "asset")? {
        if let Some(text) = find_string(asset, "version")? {
            version = text.c_str();
        }
        find_object(asset, "profile")?;
    }
    if version.first() != Some(&b'2') {
        return fail(format!("GLTF: Unsupported glTF version: {}", String::from_utf8_lossy(version)));
    }
    Ok(())
}

/// The string list in `member` (`ReadExtensionsUsed`/`Required`).
fn extension_names(document: &Value, member: &str) -> Result<Vec<Vec<u8>>, ImportError> {
    let Some(list) = find_array(document, member)? else {
        return Ok(Vec::new());
    };
    let names = list.elements().iter().filter(|value| value.is_string()).map(|value| value.c_str().to_vec()).collect();
    Ok(names)
}

/// `getCurrentAssetDir`.
fn asset_directory(path: &str) -> String {
    let slash = path.rfind('/');
    let backslash = path.rfind('\\');
    let position = match (slash, backslash) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    match position {
        Some(position) => path[..=position].to_string(),
        None => String::new(),
    }
}

impl<'a> Asset<'a> {
    /// `Asset::Load` after the document is read.
    pub fn load(path: &str, document: &'a Value, body: Option<Vec<u8>>, binary: bool) -> Result<Self, ImportError> {
        let mut asset = Asset {
            directory: asset_directory(path),
            file_system: FileSystem::new(path),
            extensions: ExtensionsUsed::default(),
            accessors: Dict::new("accessors"),
            animations: Dict::new("animations"),
            buffers: Dict::new("buffers"),
            buffer_views: Dict::new("bufferViews"),
            cameras: Dict::new("cameras"),
            lights: Dict::new("lights"),
            images: Dict::new("images"),
            materials: Dict::new("materials"),
            meshes: Dict::new("meshes"),
            nodes: Dict::new("nodes"),
            samplers: Dict::new("samplers"),
            scenes: Dict::new("scenes"),
            skins: Dict::new("skins"),
            textures: Dict::new("textures"),
            scene: None,
        };
        if binary {
            let byte_length = body.as_ref().map_or(0, Vec::len);
            asset.buffers.create(Buffer { byte_length, data: body });
        }

        check_version(document)?;
        let used = extension_names(document, "extensionsUsed")?;
        let has = |name: &str| used.iter().any(|entry| entry.as_slice() == name.as_bytes());
        asset.extensions = ExtensionsUsed {
            specular_glossiness: has("KHR_materials_pbrSpecularGlossiness"),
            specular: has("KHR_materials_specular"),
            lights_punctual: has("KHR_lights_punctual"),
            texture_transform: has("KHR_texture_transform"),
            sheen: has("KHR_materials_sheen"),
            clearcoat: has("KHR_materials_clearcoat"),
            transmission: has("KHR_materials_transmission"),
            volume: has("KHR_materials_volume"),
            ior: has("KHR_materials_ior"),
            emissive_strength: has("KHR_materials_emissive_strength"),
        };
        let required = extension_names(document, "extensionsRequired")?;
        if required.iter().any(|entry| entry.as_slice() == b"KHR_draco_mesh_compression") {
            return fail("GLTF: Draco mesh compression not supported.");
        }

        asset.accessors.attach(document)?;
        asset.animations.attach(document)?;
        asset.buffers.attach(document)?;
        asset.buffer_views.attach(document)?;
        asset.cameras.attach(document)?;
        if let Some(extensions) = find_object(document, "extensions")?
            && let Some(container) = find_object(extensions, "KHR_lights_punctual")? {
                asset.lights.array = find_array(container, "lights")?;
            }
        asset.images.attach(document)?;
        asset.materials.attach(document)?;
        asset.meshes.attach(document)?;
        asset.nodes.attach(document)?;
        asset.samplers.attach(document)?;
        asset.scenes.attach(document)?;
        asset.skins.attach(document)?;
        asset.textures.attach(document)?;

        let mut scene_index = 0u32;
        if let Some(value) = find_uint(document, "scene")? {
            scene_index = value.get_uint();
        }
        if let Some(scenes) = find_array(document, "scenes")?
            && (scene_index as usize) < scenes.elements().len() {
                asset.scene = Some(asset.retrieve_scene(scene_index)?);
            }
        if let Some(skins) = find_array(document, "skins")? {
            for index in 0..skins.elements().len() as u32 {
                asset.retrieve_skin(index)?;
            }
        }
        if let Some(animations) = find_array(document, "animations")? {
            for index in 0..animations.elements().len() as u32 {
                asset.retrieve_animation(index)?;
            }
        }
        Ok(asset)
    }

    pub fn meshes(&self) -> &[Mesh] {
        self.meshes.objects()
    }

    pub fn nodes(&self) -> &[Node] {
        self.nodes.objects()
    }

    pub fn skins(&self) -> &[Skin] {
        self.skins.objects()
    }

    pub fn animations(&self) -> &[Animation] {
        self.animations.objects()
    }

    pub fn scene_definition(&self) -> Option<&SceneDefinition> {
        self.scene.map(|slot| &self.scenes.objects()[slot])
    }

    pub fn accessor(&self, slot: usize) -> &Accessor {
        &self.accessors.objects()[slot]
    }

    fn retrieve_buffer(&mut self, index: u32) -> Result<usize, ImportError> {
        let object = match self.buffers.lookup(index)? {
            Lookup::Loaded(slot) => return Ok(slot),
            Lookup::Read(object) => object,
        };
        let stated_length = read_size(object, "byteLength", 0);
        let mut buffer = Buffer { byte_length: stated_length, data: None };
        match find_string(object, "uri")? {
            None => {
                if stated_length > 0 {
                    return fail("GLTF: buffer with non-zero length missing the \"uri\" attribute");
                }
            }
            Some(uri) => {
                let uri_bytes = uri.as_bytes();
                if let Some(data_uri) = parse_data_uri(uri_bytes) {
                    if data_uri.base64 {
                        let decoded = base64::decode(data_uri.data)?;
                        buffer.byte_length = decoded.len();
                        if stated_length > 0 && buffer.byte_length != stated_length {
                            return fail("GLTF: buffer length differs from its byteLength");
                        }
                        // Decoding fewer than four characters gives a null pointer.
                        if !decoded.is_empty() {
                            buffer.data = Some(decoded);
                        }
                    } else {
                        if stated_length != data_uri.data.len() {
                            return fail("GLTF: buffer length differs from its byteLength");
                        }
                        buffer.data = Some(data_uri.data.to_vec());
                    }
                } else if buffer.byte_length > 0 {
                    let mut directory = self.directory.clone();
                    if !directory.is_empty() && !directory.ends_with('/') {
                        directory.push('/');
                    }
                    let file_name = format!("{directory}{}", String::from_utf8_lossy(uri.c_str()));
                    let Some(contents) = self.file_system.open(&file_name) else {
                        return fail(format!("GLTF: could not open referenced file \"{file_name}\""));
                    };
                    if buffer.byte_length > contents.len() {
                        return fail("GLTF: Invalid byteLength exceeds size of actual data.");
                    }
                    buffer.data = Some(contents[..buffer.byte_length].to_vec());
                }
            }
        }
        self.buffers.add(index, object, buffer)
    }

    fn retrieve_buffer_view(&mut self, index: u32) -> Result<usize, ImportError> {
        let object = match self.buffer_views.lookup(index)? {
            Lookup::Loaded(slot) => return Ok(slot),
            Lookup::Read(object) => object,
        };
        let Some(buffer_value) = find_uint(object, "buffer")? else {
            return fail("GLTF: Buffer view without valid buffer.");
        };
        let buffer = self.retrieve_buffer(buffer_value.get_uint())?;
        let byte_offset = read_size(object, "byteOffset", 0);
        let byte_length = read_size(object, "byteLength", 0);
        let byte_stride = read_int(object, "byteStride").map_or(0, |stride| stride as u32);
        let buffer_length = self.buffers.objects()[buffer].byte_length;
        if byte_offset.wrapping_add(byte_length) > buffer_length {
            return fail(format!("GLTF: Buffer view with offset/length ({byte_offset}/{byte_length}) is out of range."));
        }
        let view = BufferView { buffer, byte_offset, byte_length, byte_stride };
        self.buffer_views.add(index, object, view)
    }

    /// `BufferView::GetPointerAndTailSize`: the buffer data from
    /// `offset` within the view on (empty when there is none).
    pub fn view_tail(&self, view: usize, offset: usize) -> Option<&[u8]> {
        let view = &self.buffer_views.objects()[view];
        let buffer = &self.buffers.objects()[view.buffer];
        let data = buffer.data.as_ref()?;
        let start = offset.wrapping_add(view.byte_offset);
        if start >= buffer.byte_length {
            return None;
        }
        Some(&data[start.min(data.len())..buffer.byte_length.min(data.len())])
    }

    pub fn buffer_view(&self, slot: usize) -> &BufferView {
        &self.buffer_views.objects()[slot]
    }

    pub fn buffer(&self, slot: usize) -> &Buffer {
        &self.buffers.objects()[slot]
    }

    fn retrieve_accessor(&mut self, index: u32) -> Result<usize, ImportError> {
        let object = match self.accessors.lookup(index)? {
            Lookup::Loaded(slot) => return Ok(slot),
            Lookup::Read(object) => object,
        };
        let mut accessor = Accessor {
            id: format!("accessors[{index}]"),
            buffer_view: None,
            byte_offset: 0,
            component_type: COMPONENT_BYTE,
            count: 0,
            components: 1,
            sparse: None,
        };
        if let Some(view) = find_uint(object, "bufferView")? {
            accessor.buffer_view = Some(self.retrieve_buffer_view(view.get_uint())?);
        }
        accessor.byte_offset = read_size(object, "byteOffset", 0);
        accessor.component_type = read_int(object, "componentType").unwrap_or(COMPONENT_BYTE);
        let Some(count) = find_uint(object, "count")? else {
            return fail(format!("A count value is required, when reading {}", accessor.id));
        };
        accessor.count = count.get_uint();
        accessor.components = read_c_string(object, "type").map_or(1, type_components);

        if let Some(view_slot) = accessor.buffer_view {
            let component = component_size(accessor.component_type)? as u64;
            let byte_length = component * accessor.count as u64;
            if byte_length < accessor.count as u64 {
                return fail("GLTF: Accessor is out of range.");
            }
            let view = self.buffer_view(view_slot);
            let buffer_length = self.buffer(view.buffer).byte_length as u64;
            let end = (accessor.byte_offset as u64).wrapping_add(byte_length);
            if end > view.byte_length as u64 || (view.byte_offset as u64).wrapping_add(end) > buffer_length {
                return fail("GLTF: Accessor with offset/length is out of range.");
            }
        }

        if let Some(sparse) = find_object(object, "sparse")? {
            let sparse_count = read_int(sparse, "count").map_or(0, |count| count as u32);
            let mut indices_view = None;
            let mut indices_offset = 0;
            let indices_type;
            if let Some(indices) = find_object(sparse, "indices")? {
                let Some(view) = find_uint(indices, "bufferView")? else {
                    return fail(format!("A bufferView value is required, when reading {}", accessor.id));
                };
                indices_view = Some(self.retrieve_buffer_view(view.get_uint())?);
                indices_offset = read_size(indices, "byteOffset", 0);
                indices_type = read_int(indices, "componentType").unwrap_or(COMPONENT_BYTE);
            } else {
                indices_type = read_int(sparse, "componentType").unwrap_or(COMPONENT_UNSIGNED_SHORT);
            }
            let mut values_view = None;
            let mut values_offset = 0;
            if let Some(values) = find_object(sparse, "values")? {
                let Some(view) = find_uint(values, "bufferView")? else {
                    return fail(format!("A bufferView value is required, when reading {}", accessor.id));
                };
                values_view = Some(self.retrieve_buffer_view(view.get_uint())?);
                values_offset = read_size(values, "byteOffset", 0);
            }

            let element_size = accessor.components as usize * component_size(accessor.component_type)?;
            let data_size = accessor.count as usize * element_size;
            let mut data = vec![0u8; data_size];
            if let Some(view) = accessor.buffer_view {
                let tail = self.view_tail(view, accessor.byte_offset).unwrap_or(&[]);
                if data_size > tail.len() {
                    return fail(format!("Invalid buffer when reading {}", accessor.id));
                }
                data.copy_from_slice(&tail[..data_size]);
            }

            // `Sparse::PatchData`.
            let index_size = component_size(indices_type)?;
            let (Some(indices_view), Some(values_view)) = (indices_view, values_view) else {
                return fail("Invalid sparse accessor.");
            };
            let indices = self.view_tail(indices_view, indices_offset).unwrap_or(&[]);
            if sparse_count as usize * index_size > indices.len() {
                return fail("Invalid sparse accessor. Indices outside allocated memory.");
            }
            let values = self.view_tail(values_view, values_offset).unwrap_or(&[]);
            if element_size * sparse_count as usize > values.len() {
                return fail("Invalid sparse accessor. Indices outside allocated memory.");
            }
            for entry in 0..sparse_count as usize {
                let at = entry * index_size;
                let target = match indices_type {
                    COMPONENT_UNSIGNED_BYTE => indices[at] as usize,
                    COMPONENT_UNSIGNED_SHORT => u16::from_le_bytes([indices[at], indices[at + 1]]) as usize,
                    COMPONENT_UNSIGNED_INT => u32::from_le_bytes(indices[at..at + 4].try_into().unwrap()) as usize,
                    _ => return fail("Unsupported component type in index."),
                };
                let offset = target * element_size;
                if offset + element_size > data.len() {
                    return fail("Invalid sparse accessor. Byte offset for patching points outside allocated memory.");
                }
                let source = &values[entry * element_size..(entry + 1) * element_size];
                data[offset..offset + element_size].copy_from_slice(source);
            }
            accessor.sparse = Some(data);
        }
        self.accessors.add(index, object, accessor)
    }

    /// `SetTextureProperties` through `ReadTextureProperty`.
    fn read_texture_property(&mut self, values: &'a Value, property: &str, scaled: Option<&str>) -> Result<(), ImportError> {
        let Some(property_value) = values.member(property).filter(|_| values.is_object()) else {
            return Ok(());
        };
        if self.extensions.texture_transform
            && let Some(transform) = find_extension(property_value, "KHR_texture_transform")? {
                find_array(transform, "offset")?;
                find_array(transform, "scale")?;
            }
        if let Some(texture) = find_uint(property_value, "index")? {
            self.retrieve_texture(texture.get_uint())?;
        }
        find_uint(property_value, "texCoord")?;
        if let Some(member) = scaled {
            find_number(property_value, member)?;
        }
        Ok(())
    }

    fn retrieve_material(&mut self, index: u32) -> Result<usize, ImportError> {
        let object = match self.materials.lookup(index)? {
            Lookup::Loaded(slot) => return Ok(slot),
            Lookup::Read(object) => object,
        };
        if let Some(metallic) = find_object(object, "pbrMetallicRoughness")? {
            self.read_texture_property(metallic, "baseColorTexture", None)?;
            self.read_texture_property(metallic, "metallicRoughnessTexture", None)?;
        }
        self.read_texture_property(object, "normalTexture", Some("scale"))?;
        self.read_texture_property(object, "occlusionTexture", Some("strength"))?;
        self.read_texture_property(object, "emissiveTexture", None)?;
        if let Some(extensions) = find_object(object, "extensions")? {
            let groups: [(bool, &str, &[&str]); 7] = [
                (self.extensions.specular_glossiness, "KHR_materials_pbrSpecularGlossiness", &["diffuseTexture", "specularGlossinessTexture"]),
                (self.extensions.specular, "KHR_materials_specular", &["specularTexture", "specularColorTexture"]),
                (self.extensions.sheen, "KHR_materials_sheen", &["sheenColorTexture", "sheenRoughnessTexture"]),
                (self.extensions.clearcoat, "KHR_materials_clearcoat", &["clearcoatTexture", "clearcoatRoughnessTexture", "clearcoatNormalTexture"]),
                (self.extensions.transmission, "KHR_materials_transmission", &["transmissionTexture"]),
                (self.extensions.volume, "KHR_materials_volume", &["thicknessTexture"]),
                (self.extensions.ior, "KHR_materials_ior", &[]),
            ];
            for (enabled, name, textures) in groups {
                if !enabled {
                    continue;
                }
                if let Some(extension) = find_object(extensions, name)? {
                    for texture in textures {
                        let scaled = if *texture == "clearcoatNormalTexture" { Some("scale") } else { None };
                        self.read_texture_property(extension, texture, scaled)?;
                    }
                }
            }
            if self.extensions.emissive_strength {
                find_object(extensions, "KHR_materials_emissive_strength")?;
            }
            find_object(extensions, "KHR_materials_unlit")?;
        }
        self.materials.add(index, object, ())
    }

    fn retrieve_texture(&mut self, index: u32) -> Result<usize, ImportError> {
        let object = match self.textures.lookup(index)? {
            Lookup::Loaded(slot) => return Ok(slot),
            Lookup::Read(object) => object,
        };
        if let Some(source) = find_uint(object, "source")? {
            self.retrieve_image(source.get_uint())?;
        }
        if let Some(sampler) = find_uint(object, "sampler")? {
            self.retrieve_sampler(sampler.get_uint())?;
        }
        self.textures.add(index, object, ())
    }

    fn retrieve_image(&mut self, index: u32) -> Result<usize, ImportError> {
        let object = match self.images.lookup(index)? {
            Lookup::Loaded(slot) => return Ok(slot),
            Lookup::Read(object) => object,
        };
        if let Some(uri) = find_string(object, "uri")? {
            if let Some(data_uri) = parse_data_uri(uri.as_bytes())
                && data_uri.base64 {
                    base64::decode(data_uri.data)?;
                }
        } else if let Some(view) = find_uint(object, "bufferView")? {
            let view = self.retrieve_buffer_view(view.get_uint())?;
            let mime_type = find_string(object, "mimeType")?;
            let has_mime_type = mime_type.is_some_and(|value| !value.c_str().is_empty());
            if !has_mime_type {
                return fail("GLTF2: image does not have a URI, so it must have a valid bufferView and mimetype");
            }
            let view = self.buffer_view(view);
            if self.buffer(view.buffer).data.is_none() && view.byte_length > 0 {
                return fail("GLTF2: image data is missing");
            }
        } else {
            return fail("GLTF2: image should have either a URI of a bufferView and mimetype");
        }
        self.images.add(index, object, ())
    }

    fn retrieve_sampler(&mut self, index: u32) -> Result<usize, ImportError> {
        let object = match self.samplers.lookup(index)? {
            Lookup::Loaded(slot) => return Ok(slot),
            Lookup::Read(object) => object,
        };
        self.samplers.add(index, object, ())
    }

    fn retrieve_camera(&mut self, index: u32) -> Result<usize, ImportError> {
        let object = match self.cameras.lookup(index)? {
            Lookup::Loaded(slot) => return Ok(slot),
            Lookup::Read(object) => object,
        };
        let orthographic = read_c_string(object, "type") == Some(b"orthographic".as_slice());
        let parameters = if orthographic { "orthographic" } else { "perspective" };
        if find_object(object, parameters)?.is_none() {
            return fail("GLTF: Camera missing its parameters");
        }
        self.cameras.add(index, object, ())
    }

    fn retrieve_light(&mut self, index: u32) -> Result<usize, ImportError> {
        let object = match self.lights.lookup(index)? {
            Lookup::Loaded(slot) => return Ok(slot),
            Lookup::Read(object) => object,
        };
        let kind = object.member("type").filter(|value| value.is_string()).map(Value::as_bytes);
        let spot = !matches!(kind, Some(b"directional") | Some(b"point"));
        if spot && find_object(object, "spot")?.is_none() {
            return fail("GLTF: Light missing its spot parameters");
        }
        self.lights.add(index, object, ())
    }

    /// One attribute entry (`GetAttribVector`): the semantic, the index
    /// after its underscore (`atoi`), contiguous from 0.
    fn read_attribute(&mut self, list: &mut Vec<Option<usize>>, name: &[u8], prefix_length: usize, accessor: u32, strict: bool) -> Result<(), ImportError> {
        let index = if name.get(prefix_length) == Some(&b'_') { msvc_atoi(&name[prefix_length + 1..]) as i64 as usize } else { 0 };
        if strict && list.len() != index {
            return fail("GLTF: Invalid attribute in mesh. All indices for indexed attribute semantics must start with 0 and be continuous positive integers: TEXCOORD_0, TEXCOORD_1, etc.");
        }
        // A negative or huge target index makes `resize` throw.
        if index >= 1 << 31 {
            return fail("bad allocation");
        }
        if list.len() <= index {
            list.resize(index + 1, None);
        }
        list[index] = Some(self.retrieve_accessor(accessor)?);
        Ok(())
    }

    fn retrieve_mesh(&mut self, index: u32) -> Result<usize, ImportError> {
        let object = match self.meshes.lookup(index)? {
            Lookup::Loaded(slot) => return Ok(slot),
            Lookup::Read(object) => object,
        };
        let id = format!("meshes[{index}]");
        let mut name = String::new();
        if let Some(value) = object.member("name").filter(|value| value.is_string()) {
            name = String::from_utf8_lossy(value.c_str()).into_owned();
        }
        let mut primitives = Vec::new();
        if let Some(list) = find_array(object, "primitives")? {
            for primitive in list.elements() {
                let mode = read_int(primitive, "mode").unwrap_or(MODE_TRIANGLES);
                let mut indices = None;
                if let Some(value) = find_uint(primitive, "indices")? {
                    indices = Some(self.retrieve_accessor(value.get_uint())?);
                }
                let mut material = None;
                if let Some(value) = find_uint(primitive, "material")? {
                    material = Some(self.retrieve_material(value.get_uint())?);
                }
                let mut attributes = Attributes::default();
                if let Some(list) = find_object(primitive, "attributes")? {
                    for (key, value) in list.members() {
                        if !value.is_uint() {
                            continue;
                        }
                        let name = c_str(key);
                        let semantics: [(&[u8], &mut Vec<Option<usize>>); 8] = [
                            (b"POSITION", &mut attributes.position),
                            (b"NORMAL", &mut attributes.normal),
                            (b"TANGENT", &mut attributes.tangent),
                            (b"TEXCOORD", &mut attributes.texcoord),
                            (b"COLOR", &mut attributes.color),
                            (b"JOINTS", &mut attributes.joint),
                            (b"JOINTMATRIX", &mut attributes.jointmatrix),
                            (b"WEIGHTS", &mut attributes.weight),
                        ];
                        for (prefix, list) in semantics {
                            if name.starts_with(prefix) {
                                self.read_attribute(list, name, prefix.len(), value.get_uint(), true)?;
                                break;
                            }
                        }
                    }
                }
                let mut targets = Vec::new();
                if let Some(list) = find_array(primitive, "targets")? {
                    targets.resize_with(list.elements().len(), Target::default);
                    for (target_index, target) in list.elements().iter().enumerate() {
                        if !target.is_object() {
                            continue;
                        }
                        for (key, value) in target.members() {
                            if !value.is_uint() {
                                continue;
                            }
                            let name = c_str(key);
                            let entry = &mut targets[target_index];
                            let semantics: [(&[u8], &mut Vec<Option<usize>>); 3] = [
                                (b"POSITION", &mut entry.position),
                                (b"NORMAL", &mut entry.normal),
                                (b"TANGENT", &mut entry.tangent),
                            ];
                            for (prefix, list) in semantics {
                                if name.starts_with(prefix) {
                                    self.read_attribute(list, name, prefix.len(), value.get_uint(), false)?;
                                    break;
                                }
                            }
                        }
                    }
                }
                if let Some(extras) = find_object(primitive, "extras")? {
                    find_array(extras, "targetNames")?;
                }
                primitives.push(Primitive { mode, indices, material, attributes, targets });
            }
        }
        find_array(object, "weights")?;
        if let Some(extras) = find_object(object, "extras")? {
            find_array(extras, "targetNames")?;
        }
        self.meshes.add(index, object, Mesh { id, name, primitives })
    }

    fn retrieve_node(&mut self, index: u32) -> Result<usize, ImportError> {
        let object = match self.nodes.lookup(index)? {
            Lookup::Loaded(slot) => return Ok(slot),
            Lookup::Read(object) => object,
        };
        let mut children = Vec::new();
        if let Some(list) = find_array(object, "children")? {
            for child in list.elements() {
                if child.is_uint() {
                    children.push(self.retrieve_node(child.get_uint())?);
                }
            }
        }
        find_array(object, "matrix")?;
        let mut mesh = None;
        if let Some(value) = find_uint(object, "mesh")? {
            mesh = Some(self.retrieve_mesh(value.get_uint())?);
        }
        let skin = find_uint(object, "skin")?.map(Value::get_uint);
        if let Some(value) = find_uint(object, "camera")? {
            self.retrieve_camera(value.get_uint())?;
        }
        if let Some(extensions) = find_object(object, "extensions")?
            && self.extensions.lights_punctual
                && let Some(extension) = find_object(extensions, "KHR_lights_punctual")?
                    && let Some(light) = find_uint(extension, "light")? {
                        self.retrieve_light(light.get_uint())?;
                    }
        self.nodes.add(index, object, Node { mesh, skin, children })
    }

    fn retrieve_scene(&mut self, index: u32) -> Result<usize, ImportError> {
        let object = match self.scenes.lookup(index)? {
            Lookup::Loaded(slot) => return Ok(slot),
            Lookup::Read(object) => object,
        };
        find_string(object, "name")?;
        let mut nodes = Vec::new();
        if let Some(list) = find_array(object, "nodes")? {
            for node in list.elements() {
                if node.is_uint() {
                    nodes.push(self.retrieve_node(node.get_uint())?);
                }
            }
        }
        self.scenes.add(index, object, SceneDefinition { nodes })
    }

    fn retrieve_skin(&mut self, index: u32) -> Result<usize, ImportError> {
        let object = match self.skins.lookup(index)? {
            Lookup::Loaded(slot) => return Ok(slot),
            Lookup::Read(object) => object,
        };
        let mut inverse_bind_matrices = None;
        if let Some(value) = find_uint(object, "inverseBindMatrices")? {
            inverse_bind_matrices = Some(self.retrieve_accessor(value.get_uint())?);
        }
        let mut joints = Vec::new();
        if let Some(list) = find_array(object, "joints")? {
            for joint in list.elements() {
                if joint.is_uint() {
                    joints.push(self.retrieve_node(joint.get_uint())?);
                }
            }
        }
        self.skins.add(index, object, Skin { inverse_bind_matrices, joints })
    }

    fn retrieve_animation(&mut self, index: u32) -> Result<usize, ImportError> {
        let object = match self.animations.lookup(index)? {
            Lookup::Loaded(slot) => return Ok(slot),
            Lookup::Read(object) => object,
        };
        let mut samplers = Vec::new();
        if let Some(list) = find_array(object, "samplers")? {
            for sampler in list.elements() {
                let mut entry = AnimationSampler { input: None, output: None };
                if let Some(value) = find_uint(sampler, "input")? {
                    entry.input = Some(self.retrieve_accessor(value.get_uint())?);
                }
                if let Some(value) = find_uint(sampler, "output")? {
                    entry.output = Some(self.retrieve_accessor(value.get_uint())?);
                }
                find_string(sampler, "interpolation")?;
                samplers.push(entry);
            }
        }
        let mut channels = Vec::new();
        if let Some(list) = find_array(object, "channels")? {
            for channel in list.elements() {
                let mut entry = Channel { sampler: -1, node: None, path: PATH_TRANSLATION };
                if let Some(value) = find_uint(channel, "sampler")? {
                    entry.sampler = value.get_uint() as i32 as i64;
                }
                if let Some(target) = find_object(channel, "target")? {
                    if let Some(node) = find_uint(target, "node")? {
                        entry.node = Some(self.retrieve_node(node.get_uint())?);
                    }
                    if let Some(path) = find_string(target, "path")? {
                        entry.path = match path.c_str() {
                            b"translation" => PATH_TRANSLATION,
                            b"rotation" => PATH_ROTATION,
                            b"scale" => PATH_SCALE,
                            b"weights" => PATH_WEIGHTS,
                            _ => entry.path,
                        };
                    }
                }
                channels.push(entry);
            }
        }
        self.animations.add(index, object, Animation { samplers, channels })
    }
}

/// A member name read as a C string.
fn c_str(bytes: &[u8]) -> &[u8] {
    let end = bytes.iter().position(|&byte| byte == 0).unwrap_or(bytes.len());
    &bytes[..end]
}

/// UCRT `atoi`: optional whitespace and sign, digits, clamped to `int`.
pub fn msvc_atoi(text: &[u8]) -> i32 {
    let mut position = 0;
    while position < text.len() && matches!(text[position], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        position += 1;
    }
    let mut negative = false;
    if position < text.len() && (text[position] == b'+' || text[position] == b'-') {
        negative = text[position] == b'-';
        position += 1;
    }
    let mut value: i64 = 0;
    while position < text.len() && text[position].is_ascii_digit() {
        value = (value * 10 + (text[position] - b'0') as i64).min(i64::from(i32::MAX) + 1);
        position += 1;
    }
    let signed = if negative { -value } else { value };
    signed.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}
