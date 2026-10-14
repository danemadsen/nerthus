//! Assimp's Quake III importer (`MD3Loader.cpp`): frame 0 of a file read
//! in place, a mesh per surface (stored back to front), a vertex per
//! triangle corner (scaled by 1/64, latitude/longitude normals, texture
//! coordinates). Faces are wound 0, 2, 1 unless the surface's shader
//! culls `back`; the shader comes from `../../../scripts/<folder>.shader`
//! (or `<file>.shader`) beside the model, matched through the surface's
//! skin texture (`<name>_default.skin`) or first shader name. A file
//! named `lower`, `upper` or `head` (with an optional `_suffix`) loads
//! all three parts and joins their meshes when the tags match.
//!
//! Kept quirks: skin tokens that are prefixes of `tag_` are skipped; an
//! out-of-range vertex index throws; an empty surface leaves its slot
//! null, which only works for the first surfaces (others crash). Text
//! files are read as Windows text mode reads them (CR LF to LF, stopping
//! at Ctrl-Z). Reads past the file and endless skin scans are errors.

use crate::io::FileSystem;
use crate::parsing::{at, is_space_or_newline, remove_line_comments, skip_line, skip_spaces, skip_spaces_and_line_end, starts_with_ignore_case_at, token_match_i};
use crate::scene::{ImportError, Mesh, PRIMITIVE_TRIANGLE, Scene, fail};

const HEADER_SIZE: usize = 108;
const SURFACE_SIZE: u64 = 108;
const TAG_SIZE: u64 = 112;

/// `AI_MAX_ALLOC(MD3::Surface)` and `AI_MAX_ALLOC(aiMesh)`.
const MAX_SURFACES: u64 = 256 * 1024 * 1024 / 108;
const MAX_MESHES: u64 = 256 * 1024 * 1024 / 1320;

/// `MD3::LatLngNormalToVec3`: a normal packed as latitude and longitude
/// bytes.
pub(crate) fn lat_lng_normal(packed: u16) -> [f32; 3] {
    let step = 3.141926f32 * (1.0 / 128.0);
    let lat = ((packed >> 8) & 0xff) as f32 * step;
    let lng = (packed & 0xff) as f32 * step;
    [lat.cos() * lng.sin(), lat.sin() * lng.sin(), lng.cos()]
}

fn read_past_the_file<T>() -> Result<T, ImportError> {
    fail("undefined behaviour: a read past the file")
}

/// A C string at `offset` (to its NUL, which must be in the file).
fn c_string(data: &[u8], offset: u64) -> Result<&[u8], ImportError> {
    if offset >= data.len() as u64 {
        return read_past_the_file();
    }
    let rest = &data[offset as usize..];
    match rest.iter().position(|&byte| byte == 0) {
        Some(length) => Ok(&rest[..length]),
        None => read_past_the_file(),
    }
}

fn bytes_at<const N: usize>(data: &[u8], offset: u64) -> Result<[u8; N], ImportError> {
    if offset + N as u64 > data.len() as u64 {
        return read_past_the_file();
    }
    let start = offset as usize;
    Ok(data[start..start + N].try_into().unwrap())
}

fn word(data: &[u8], offset: u64) -> Result<u32, ImportError> {
    Ok(u32::from_le_bytes(bytes_at(data, offset)?))
}

/// A text file opened with `"rt"`: CR LF read as LF, Ctrl-Z ending it,
/// then a NUL.
fn text_mode(bytes: &[u8]) -> Vec<u8> {
    let mut text = Vec::with_capacity(bytes.len() + 1);
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == 0x1a {
            break;
        }
        if byte == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
            index += 1;
            continue;
        }
        text.push(byte);
        index += 1;
    }
    text.push(0);
    text
}

/// `GetNextToken`: blanks and line ends skipped, then up to the next.
fn next_token(buffer: &[u8], position: &mut usize) -> Vec<u8> {
    *position = skip_spaces_and_line_end(buffer, *position, buffer.len()).0;
    let start = *position;
    while !is_space_or_newline(at(buffer, *position)) {
        *position += 1;
    }
    buffer[start.min(buffer.len())..(*position).min(buffer.len())].to_vec()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cull {
    None,
    Clockwise,
    CounterClockwise,
}

struct ShaderBlock {
    name: Vec<u8>,
    cull: Cull,
}

/// `Q3Shader::LoadShader`: the blocks (names and cull modes), or `None`
/// when the file can't be opened.
fn load_shader(file_system: &FileSystem, path: &str) -> Result<Option<Vec<ShaderBlock>>, ImportError> {
    let Some(bytes) = file_system.open(path) else {
        return Ok(None);
    };
    let mut buffer = text_mode(&bytes);
    remove_line_comments(b"//", &mut buffer, b' ');
    let end = buffer.len();
    let mut blocks: Vec<ShaderBlock> = Vec::new();
    let mut current: Option<usize> = None;
    let mut position = 0;
    loop {
        let (next, goes_on) = skip_spaces_and_line_end(&buffer, position, end);
        position = next;
        if !goes_on {
            break;
        }
        if buffer[position] == b'{' {
            position += 1;
            let Some(block) = current else {
                // "Q3Shader: Unexpected shader section token '{'"
                return Ok(Some(blocks));
            };
            loop {
                let (next, goes_on) = skip_spaces_and_line_end(&buffer, position, end);
                position = next;
                if !goes_on {
                    break;
                }
                if buffer[position] == b'{' {
                    position += 1;
                    read_map_section(&buffer, &mut position)?;
                } else if buffer[position] == b'}' {
                    position += 1;
                    current = None;
                    break;
                } else if token_match_i(&buffer, &mut position, b"cull")? {
                    position = skip_spaces(&buffer, position, end).0;
                    if starts_with_ignore_case_at(&buffer, position, b"back") {
                        blocks[block].cull = Cull::CounterClockwise;
                    } else if starts_with_ignore_case_at(&buffer, position, b"front") {
                        blocks[block].cull = Cull::Clockwise;
                    } else if starts_with_ignore_case_at(&buffer, position, b"none")
                        || starts_with_ignore_case_at(&buffer, position, b"twosided")
                        || starts_with_ignore_case_at(&buffer, position, b"disable")
                    {
                        blocks[block].cull = Cull::None;
                    }
                }
                position = skip_line(&buffer, position, end).0;
            }
        } else {
            let name = next_token(&buffer, &mut position);
            blocks.push(ShaderBlock { name, cull: Cull::Clockwise });
            current = Some(blocks.len() - 1);
        }
        position = skip_line(&buffer, position, end).0;
    }
    Ok(Some(blocks))
}

/// A shader's `{ … }` texture map section: only how far it reads matters.
fn read_map_section(buffer: &[u8], position: &mut usize) -> Result<(), ImportError> {
    let end = buffer.len();
    loop {
        let (next, goes_on) = skip_spaces_and_line_end(buffer, *position, end);
        *position = next;
        if !goes_on {
            return Ok(());
        }
        if token_match_i(buffer, position, b"map")? || token_match_i(buffer, position, b"clampmap")? {
            next_token(buffer, position);
        } else if token_match_i(buffer, position, b"blendfunc")? {
            let source = next_token(buffer, position);
            if source != b"add" && source != b"filter" && source != b"blend" {
                next_token(buffer, position);
            }
        } else if token_match_i(buffer, position, b"alphafunc")? {
            next_token(buffer, position);
        } else if buffer[*position] == b'}' {
            *position += 1;
            return Ok(());
        }
        *position = skip_line(buffer, *position, end).0;
    }
}

/// A skin file's surface name and texture pairs.
type Skin = Vec<(Vec<u8>, Vec<u8>)>;

/// `Q3Shader::LoadSkin`: surface name and texture pairs (empty when the
/// file can't be opened). A scan that stops moving never ends in the
/// C++.
fn load_skin(file_system: &FileSystem, path: &str) -> Result<Skin, ImportError> {
    let Some(bytes) = file_system.open(path) else {
        return Ok(Vec::new());
    };
    let mut buffer = text_mode(&bytes);
    for byte in &mut buffer {
        if *byte == b',' {
            *byte = b' ';
        }
    }
    let end = buffer.len();
    let mut textures = Vec::new();
    let mut position = 0;
    while at(&buffer, position) != 0 {
        let before = position;
        position = skip_spaces_and_line_end(&buffer, position, end).0;
        let token = next_token(&buffer, &mut position);
        let compared = token.len().min(4);
        if token[..compared] == b"tag_"[..compared] {
            if position == before {
                return fail("MD3: a skin file the C++ reads forever");
            }
            continue;
        }
        let texture = next_token(&buffer, &mut position);
        textures.push((token, texture));
    }
    Ok(textures)
}

/// The last `\` in a C string, else its last `/` (`strrchr` twice).
fn last_separator(text: &[u8]) -> Option<usize> {
    text.iter().rposition(|&byte| byte == b'\\').or_else(|| text.iter().rposition(|&byte| byte == b'/'))
}

/// `MD3Importer::ConvertPath`: a texture path in the model's own folder
/// (or under `models`) loses its folders.
fn convert_path(texture: &[u8], header_name: &[u8]) -> Vec<u8> {
    let header_end = last_separator(header_name);
    if let Some(texture_end) = last_separator(texture) {
        let compared;
        let models = starts_with_ignore_case_at(texture, 0, b"models") && matches!(at(texture, 6), b'/' | b'\\');
        if models {
            compared = 6;
            if header_name.is_empty() {
                return texture[texture_end + 1..].to_vec();
            }
        } else {
            // A header name without separators: the distance from a null
            // pointer, bigger than any length.
            compared = header_end.unwrap_or(usize::MAX).min(texture_end);
        }
        // `_strnicmp`: equal up to `compared` bytes or a shared NUL.
        let mut equal = true;
        for index in 0..compared {
            let a = at(texture, index).to_ascii_lowercase();
            let b = at(header_name, index).to_ascii_lowercase();
            if a != b {
                equal = false;
                break;
            }
            if a == 0 {
                break;
            }
        }
        if equal {
            return texture[texture_end + 1..].to_vec();
        }
    }
    texture.to_vec()
}

/// One MD3 file: its meshes (in slot order, null for an empty surface
/// that wasn't among the first) and tag names.
struct Part {
    meshes: Vec<Option<Mesh>>,
    tags: Vec<Vec<u8>>,
}

/// The meshes of parts, failing on a null one (dereferenced later).
fn take_meshes(parts: Vec<Part>) -> Result<Vec<Mesh>, ImportError> {
    let mut meshes = Vec::new();
    for part in parts {
        for mesh in part.meshes {
            let Some(mesh) = mesh else {
                return fail("undefined behaviour: a null mesh after an empty surface");
            };
            meshes.push(mesh);
        }
    }
    Ok(meshes)
}

/// `MD3Importer::InternReadFile` without the multi-part handling.
fn read_single(path: &str, data: &[u8], files: &FileSystem) -> Result<Part, ImportError> {
    let file_size = data.len() as u64;
    if data.len() < HEADER_SIZE {
        return fail("MD3 File is too small.");
    }
    let field = |offset: u64| word(data, offset);
    let magic = field(0)?.to_le_bytes();
    if &magic != b"IDP3" && &magic != b"3PDI" {
        return fail("Invalid MD3 file: Magic bytes not found");
    }
    let frame_count = field(76)?;
    let tag_count = field(80)?;
    let surface_count = field(84)?;
    let frames_offset = field(92)?;
    let tags_offset = field(96)?;
    let surfaces_offset = field(100)?;
    let end_offset = field(104)?;
    if surface_count == 0 {
        return fail("Invalid md3 file: NUM_SURFACES is 0");
    }
    if frames_offset as u64 >= file_size || surfaces_offset as u64 >= file_size || end_offset as u64 > file_size {
        return fail("Invalid MD3 header: some offsets are outside the file");
    }
    if surface_count as u64 > MAX_SURFACES {
        return fail("Invalid MD3 header: too many surfaces, would overflow");
    }
    if surfaces_offset as u64 + surface_count as u64 * SURFACE_SIZE >= file_size {
        return fail("Invalid MD3 header: some surfaces are outside the file");
    }
    if frame_count == 0 {
        return fail("The requested frame is not existing the file");
    }
    if tags_offset as u64 + TAG_SIZE * tag_count as u64 > file_size {
        return fail("MD3 tags are outside the file");
    }
    if surface_count as u64 > MAX_MESHES {
        return fail("MD3: Too many surfaces, would run out of memory");
    }

    // The folder (with its separator) and the lowercase file name.
    let split = path.rfind(['/', '\\']).map_or(0, |separator| separator + 1);
    let folder = &path[..split];
    let filename = path[split..].to_ascii_lowercase();

    let skin_end = filename.rfind('_').or_else(|| filename.rfind('.')).unwrap_or(filename.len());
    let skin_path = format!("{folder}{}_default.skin", &filename[..skin_end]);
    let skins = load_skin(files, &skin_path)?;

    // `ReadShader`: the scripts folder three levels up, named after the
    // model's folder, else after the file.
    let folder_start = if folder.len() >= 2 { folder[..folder.len() - 1].rfind(['/', '\\']).map_or(0, |separator| separator + 1) } else { 0 };
    let model_name = if folder.len() >= 2 { &folder[folder_start..folder.len() - 1] } else { "" };
    let separator = std::path::MAIN_SEPARATOR;
    let scripts = format!("{folder}..{separator}..{separator}..{separator}scripts{separator}");
    let mut shaders = load_shader(files, &format!("{scripts}{model_name}.shader"))?;
    if shaders.is_none() {
        shaders = load_shader(files, &format!("{scripts}{filename}.shader"))?;
    }
    let mut shaders = shaders.unwrap_or_default();

    let header_name = c_string(data, 8)?.to_vec();
    for block in &mut shaders {
        block.name = convert_path(&block.name, &header_name);
    }

    let mut slots: Vec<Option<Mesh>> = (0..surface_count).map(|_| None).collect();
    let mut mesh_count = surface_count as usize;
    // `OFS_END` is signed: the chain can step back, even before the file.
    let mut surface_start = surfaces_offset as i64;
    for slot in (0..surface_count as usize).rev() {
        if surface_start < 0 || surface_start as u64 + SURFACE_SIZE > file_size {
            return read_past_the_file();
        }
        let surface = surface_start as u64;
        let surface_field = |index: u64| word(data, surface + 72 + index * 4);
        let shader_count = surface_field(1)?;
        let vertex_count = surface_field(2)?;
        let triangle_count = surface_field(3)?;
        let triangles_offset = surface_field(4)?;
        let shaders_offset = surface_field(5)?;
        let texcoords_offset = surface_field(6)?;
        let vertices_offset = surface_field(7)?;
        let next_offset = surface_field(8)? as i32 as i64;
        let position = surface as u32;
        let within = |offset: u32, count: u32, size: u64| offset.wrapping_add(position) as u64 + count as u64 * size > file_size;
        if within(triangles_offset, triangle_count, 12) || within(shaders_offset, shader_count, 68) || within(texcoords_offset, vertex_count, 8) || within(vertices_offset, vertex_count, 8) {
            return fail("Invalid MD3 surface header: some offsets are outside the file");
        }
        if vertex_count == 0 || triangle_count == 0 {
            surface_start += next_offset;
            mesh_count -= 1;
            continue;
        }

        let mut texture: Option<Vec<u8>> = None;
        if !skins.is_empty() {
            let name = c_string(data, surface + 4)?;
            if let Some((_, skin_texture)) = skins.iter().find(|(surface_name, _)| surface_name.as_slice() == name) {
                texture = Some(skin_texture.clone());
            }
        }
        if texture.is_none() && shader_count != 0 {
            texture = Some(c_string(data, surface + shaders_offset as u64)?.to_vec());
        }
        let converted = match &texture {
            Some(texture) => convert_path(texture, &header_name),
            None => Vec::new(),
        };
        let mut culls_back = false;
        if !shaders.is_empty() {
            let without_extension = match converted.iter().rposition(|&byte| byte == b'.') {
                Some(dot) => &converted[..dot],
                None => &converted[..],
            };
            if let Some(block) = shaders.iter().find(|block| block.name.eq_ignore_ascii_case(without_extension)) {
                culls_back = block.cull == Cull::CounterClockwise;
            }
        }

        let corner_count = triangle_count as usize * 3;
        let mut vertices = Vec::with_capacity(corner_count);
        let mut normals = Vec::with_capacity(corner_count);
        let mut coords = Vec::with_capacity(corner_count);
        let mut faces = Vec::with_capacity(triangle_count as usize);
        let triangles = surface + triangles_offset as u64;
        for triangle in 0..triangle_count as u64 {
            let first = vertices.len() as u32;
            for corner in 0..3u64 {
                let index = word(data, triangles + triangle * 12 + corner * 4)?;
                if index >= vertex_count {
                    return fail("MD3: Invalid vertex index");
                }
                let vertex: [u8; 8] = bytes_at(data, surface + vertices_offset as u64 + index as u64 * 8)?;
                let scale = 1.0f32 / 64.0;
                let x = i16::from_le_bytes([vertex[0], vertex[1]]) as f32 * scale;
                let y = i16::from_le_bytes([vertex[2], vertex[3]]) as f32 * scale;
                let z = i16::from_le_bytes([vertex[4], vertex[5]]) as f32 * scale;
                vertices.push([x, y, z]);
                normals.push(lat_lng_normal(u16::from_le_bytes([vertex[6], vertex[7]])));
                let uv: [u8; 8] = bytes_at(data, surface + texcoords_offset as u64 + index as u64 * 8)?;
                let u = f32::from_le_bytes([uv[0], uv[1], uv[2], uv[3]]);
                let v = f32::from_le_bytes([uv[4], uv[5], uv[6], uv[7]]);
                coords.push([u, 1.0 - v, 0.0]);
            }
            if culls_back {
                faces.push(vec![first, first + 1, first + 2]);
            } else {
                faces.push(vec![first, first + 2, first + 1]);
            }
        }
        let mut mesh = Mesh {
            primitive_types: PRIMITIVE_TRIANGLE,
            vertex_count: corner_count,
            vertices: Some(vertices),
            normals: Some(normals),
            faces: Some(faces),
            ..Mesh::default()
        };
        mesh.uv_components[0] = 2;
        mesh.texture_coords[0] = Some(coords);
        slots[slot] = Some(mesh);
        surface_start += next_offset;
    }
    if mesh_count == 0 {
        return fail("MD3: File contains no valid mesh");
    }
    slots.truncate(mesh_count);

    let mut tags = Vec::with_capacity(tag_count as usize);
    for tag in 0..tag_count as u64 {
        let name = c_string(data, tags_offset as u64 + tag * TAG_SIZE)?;
        // `aiString::Set` leaves a name of 1024 bytes or more empty.
        tags.push(if name.len() > 1023 { Vec::new() } else { name.to_vec() });
    }
    Ok(Part { meshes: slots, tags })
}

/// A part loaded on its own (`BatchLoader`, through the host file's
/// filter): the file must be there. A part that fails to load is left
/// out, unless its importer would have crashed or hung.
fn load_part(path: &str, files: &FileSystem) -> Result<Option<Part>, ImportError> {
    if !files.exists(path) {
        return Ok(None);
    }
    let Some(data) = files.open(path) else {
        return Ok(None);
    };
    let part_files = FileSystem::new(path, Some(files));
    match read_single(path, &data, &part_files) {
        Ok(part) => Ok(Some(part)),
        Err(error) if error.ends_process() => Err(error),
        Err(_) => Ok(None),
    }
}

/// `ReadMultipartFile`: the joined meshes, `None` to read the file alone.
fn read_multipart(path: &str, files: &FileSystem) -> Result<Option<Vec<Mesh>>, ImportError> {
    let split = path.rfind(['/', '\\']).map_or(0, |separator| separator + 1);
    let folder = &path[..split];
    let filename = path[split..].to_ascii_lowercase();
    let dot = filename.rfind('.').unwrap_or(filename.len());
    let underscore = filename.rfind('_').unwrap_or(dot);
    let part_name = &filename[..underscore];
    if part_name != "lower" && part_name != "upper" && part_name != "head" {
        return Ok(None);
    }
    let suffix = filename.get(underscore..dot).unwrap_or("");
    let names = ["lower", "upper", "head"];
    let mut parts = Vec::new();
    for name in names {
        match load_part(&format!("{folder}{name}{suffix}.md3"), files)? {
            Some(part) => parts.push(part),
            None => {
                if name == part_name {
                    return fail("MD3: failure to read multipart host file");
                }
                return Ok(None);
            }
        }
    }
    let has_tag = |part: &Part, tag: &[u8]| part.tags.iter().any(|name| name == tag);
    if !has_tag(&parts[0], b"tag_torso") || !has_tag(&parts[1], b"tag_head") {
        return Ok(None);
    }
    // Merging the parts reads every mesh.
    Ok(Some(take_meshes(parts)?))
}

pub fn read(path: &str, data: &[u8], files: &FileSystem) -> Result<Scene, ImportError> {
    if let Some(meshes) = read_multipart(path, files)? {
        return Ok(Scene { meshes });
    }
    let part = read_single(path, data, files)?;
    Ok(Scene { meshes: take_meshes(vec![part])? })
}
