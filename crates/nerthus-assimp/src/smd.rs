//! Assimp's Valve SMD importer (`SMDLoader.cpp`, `.smd` and `.vta`): a
//! mesh per texture (the names the triangles give, matched in any case)
//! in the order the textures first appear, three vertices per triangle
//! with their normals and, unless the file has a vertex animation section,
//! texture coordinates. A file without triangles but with bones gives the
//! skeleton mesh of its bone hierarchy (each bone placed by its first
//! key); one with neither fails. The animation files a
//! `<name>_animation.txt` beside the file lists are parsed too: a missing
//! or failing one fails the import.
//!
//! Kept as Assimp parses: a vertex reads to its line's end, values it
//! lacks staying zero (a line of blanks is a vertex); "version" followed by a
//! line end stops the file; the nodes section ends at "end" in any case,
//! but only right at the cursor (an indented one names a bone); the vertex
//! animation section fills each triangle's corners 1, 2, 0 and drops the
//! last triangle unless it stopped after two corners (even a triangles
//! section's one when it read no vertex).
//!
//! Failures here where Assimp does undefined things: a triangle starting
//! at a form feed never moves on (Assimp adds triangles until memory runs
//! out); the nodes section's "end" just before the final NUL reads past the
//! buffer; a texture name of 1024 bytes or more is copied out of a
//! 1024-byte string; a bone hierarchy deeper than the stack allows.

use crate::fast_atof::{fast_atoreal_move, strtol10, strtoul10};
use crate::io::FileSystem;
use crate::matrix::Matrix4;
use crate::msvc;
use crate::node::Node;
use crate::parsing::{at, is_space_or_newline, skip_line, skip_spaces, skip_spaces_and_line_end, starts_with_ignore_case_at, text_file_to_buffer, token_match};
use crate::scene::{self, ImportError, Mesh, PRIMITIVE_TRIANGLE, Scene, fail};
use crate::skeleton;

/// `AI_CONFIG_IMPORT_GLOBAL_KEYFRAME`'s default: the vertex animation
/// section reads frame 0.
const CONFIG_FRAME_ID: u32 = 0;
/// `sizeof(SMD::Bone)` in the Windows build, and `asBones.reserve(20)`.
const BONE_SIZE: u64 = 144;
const BONE_RESERVE: usize = 20;
/// `sizeof(std::pair<unsigned int, float>)`, a bone link.
const LINK_SIZE: u64 = 8;
/// `AI_MAXLEN`: a longer texture name overruns the `aiString` it is
/// copied from.
const MAX_LENGTH: usize = 1024;
/// How deep a bone hierarchy can go before Assimp's recursion through it
/// would overflow the stack.
const MAX_DEPTH: usize = 4096;

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// What `SMD::Vertex` keeps that the meshes use.
#[derive(Clone, Copy, Default)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
}

/// `SMD::Face`.
#[derive(Clone, Copy, Default)]
struct Face {
    texture: u32,
    vertices: [Vertex; 3],
}

/// What `SMD::Bone` keeps that the skeleton mesh uses.
#[derive(Clone)]
struct Bone {
    name: Vec<u8>,
    parent: u32,
    /// `sAnim.asKeys[0].matrix`, once there is a key.
    first_key: Option<Matrix4>,
}

impl Default for Bone {
    fn default() -> Self {
        Self { name: Vec::new(), parent: u32::MAX, first_key: None }
    }
}

/// `ReadSmd` and its parse (`SMDImporter`'s per-file state).
struct Parser {
    buffer: Vec<u8>,
    end: usize,
    textures: Vec<Vec<u8>>,
    triangles: Vec<Face>,
    bones: Vec<Bone>,
    bone_capacity: usize,
    has_uvs: bool,
}

impl Parser {
    /// `ReadSmd` for a file's bytes: `TextFileToBuffer`, then `ParseFile`.
    fn read(bytes: &[u8]) -> Result<Parser, ImportError> {
        let buffer = text_file_to_buffer(bytes, false)?;
        let end = buffer.len();
        let mut parser = Parser {
            buffer,
            end,
            textures: Vec::new(),
            triangles: Vec::new(),
            bones: Vec::new(),
            bone_capacity: BONE_RESERVE,
            has_uvs: true,
        };
        parser.parse_file()?;
        Ok(parser)
    }

    /// `SMDI_PARSE_RETURN`: on to the next line.
    fn next_line(&self, position: usize) -> usize {
        skip_line(&self.buffer, position, self.end).0
    }

    /// `ParseUnsignedInt`: none at a line end.
    fn parse_unsigned_int(&self, position: usize) -> Option<(u32, usize)> {
        let (start, more) = skip_spaces(&self.buffer, position, self.end);
        if !more {
            return None;
        }
        Some(strtoul10(&self.buffer, start))
    }

    /// `ParseSignedInt`: none at a line end.
    fn parse_signed_int(&self, position: usize) -> Option<(i32, usize)> {
        let (start, more) = skip_spaces(&self.buffer, position, self.end);
        if !more {
            return None;
        }
        Some(strtol10(&self.buffer, start))
    }

    /// `ParseFloat`: none at a line end; text that isn't a number throws.
    fn parse_float(&self, position: usize) -> Result<Option<(f32, usize)>, ImportError> {
        let (start, more) = skip_spaces(&self.buffer, position, self.end);
        if !more {
            return Ok(None);
        }
        let (value, after) = fast_atoreal_move(&self.buffer, start, true)?;
        Ok(Some((value, after)))
    }

    /// `ParseFile`.
    fn parse_file(&mut self) -> Result<(), ImportError> {
        let mut position = 0;
        loop {
            let (next, more) = skip_spaces_and_line_end(&self.buffer, position, self.end);
            position = next;
            if !more {
                break;
            }
            if token_match(&self.buffer, &mut position, b"version") {
                let (next, more) = skip_spaces(&self.buffer, position, self.end);
                position = next;
                if !more {
                    break;
                }
                position = strtoul10(&self.buffer, position).1;
                continue;
            }
            if token_match(&self.buffer, &mut position, b"nodes") {
                position = self.parse_nodes_section(position)?;
                continue;
            }
            if token_match(&self.buffer, &mut position, b"triangles") {
                position = self.parse_triangles_section(position)?;
                continue;
            }
            if token_match(&self.buffer, &mut position, b"vertexanimation") {
                self.has_uvs = false;
                position = self.parse_vertex_animation_section(position)?;
                continue;
            }
            if token_match(&self.buffer, &mut position, b"skeleton") {
                position = self.parse_skeleton_section(position)?;
                continue;
            }
            position = self.next_line(position);
        }
        Ok(())
    }

    /// `ParseNodesSection`: node lines until "end" (any case) at the
    /// cursor; the four bytes it takes may reach past the final NUL.
    fn parse_nodes_section(&mut self, mut position: usize) -> Result<usize, ImportError> {
        loop {
            let at_end = starts_with_ignore_case_at(&self.buffer, position, b"end");
            if at_end && is_space_or_newline(at(&self.buffer, position + 3)) {
                position += 4;
                break;
            }
            position = self.parse_node_info(position)?;
        }
        if position >= self.buffer.len() {
            return undefined("a read past the file after the nodes section's end");
        }
        Ok(skip_spaces_and_line_end(&self.buffer, position, self.end).0)
    }

    /// `ParseNodeInfo`: a bone's index (growing the bones to it), its
    /// name (in quotes, or to a blank) and its parent's index.
    fn parse_node_info(&mut self, position: usize) -> Result<usize, ImportError> {
        let (position, _) = skip_spaces_and_line_end(&self.buffer, position, self.end);
        let Some((bone_index, after_index)) = self.parse_unsigned_int(position) else {
            return fail("Unexpected EOF/EOL while parsing bone index");
        };
        let (mut position, more) = skip_spaces(&self.buffer, after_index, self.end);
        if !more {
            return fail("Unexpected EOF/EOL while parsing bone index");
        }
        if bone_index == u32::MAX {
            return Ok(self.next_line(position));
        }
        let index = bone_index as usize;
        if index >= self.bones.len() {
            let needed = index + 1;
            if needed > self.bone_capacity {
                self.bone_capacity = msvc::vector_growth(self.bone_capacity, needed);
                scene::check_allocation(self.bone_capacity as u64, BONE_SIZE)?;
            }
            self.bones.resize(needed, Bone::default());
        }

        let quoted = at(&self.buffer, position) == b'"';
        if quoted {
            position += 1;
        }
        let mut name_end = position;
        let name_length;
        loop {
            let byte = at(&self.buffer, name_end);
            if quoted && byte == b'"' {
                name_length = name_end - position;
                name_end += 1;
                break;
            } else if !quoted && is_space_or_newline(byte) {
                name_length = name_end - position;
                break;
            } else if byte == 0 {
                return Ok(self.next_line(position));
            }
            name_end += 1;
        }
        self.bones[index].name = self.buffer[position..position + name_length].to_vec();
        position = name_end;

        if let Some((parent, after)) = self.parse_signed_int(position) {
            self.bones[index].parent = parent as u32;
            position = after;
        }
        Ok(self.next_line(position))
    }

    /// `ParseTrianglesSection`: triangles until "end".
    fn parse_triangles_section(&mut self, mut position: usize) -> Result<usize, ImportError> {
        loop {
            let (next, more) = skip_spaces_and_line_end(&self.buffer, position, self.end);
            position = next;
            if !more {
                break;
            }
            if token_match(&self.buffer, &mut position, b"end") {
                break;
            }
            let Some(after) = self.parse_triangle(position)? else {
                return fail("endless loop: a triangle starting at a form feed (Assimp adds triangles until memory runs out)");
            };
            position = after;
        }
        Ok(skip_spaces_and_line_end(&self.buffer, position, self.end).0)
    }

    /// `ParseTriangle`: the texture name, then three vertices. None when
    /// it starts at a line end the caller doesn't skip (a form feed),
    /// where it doesn't move on.
    fn parse_triangle(&mut self, position: usize) -> Result<Option<usize>, ImportError> {
        let (start, more) = skip_spaces(&self.buffer, position, self.end);
        if !more {
            self.triangles.push(Face::default());
            return Ok(None);
        }
        let mut name_end = start + 1;
        while !is_space_or_newline(at(&self.buffer, name_end)) {
            name_end += 1;
        }
        let name = self.buffer[start..name_end].to_vec();
        let texture = self.texture_index(name);

        let (mut position, _) = skip_spaces_and_line_end(&self.buffer, name_end, self.end);
        let mut vertices = [Vertex::default(); 3];
        for vertex in &mut vertices {
            let (parsed, after) = self.parse_vertex(position, false)?;
            *vertex = parsed;
            position = after;
        }
        self.triangles.push(Face { texture, vertices });
        Ok(Some(position))
    }

    /// `GetTextureIndex`: a texture's index, its name matched in any case,
    /// added when new.
    fn texture_index(&mut self, name: Vec<u8>) -> u32 {
        let existing = self.textures.iter().position(|texture| texture.eq_ignore_ascii_case(&name));
        if let Some(index) = existing {
            return index as u32;
        }
        self.textures.push(name);
        (self.textures.len() - 1) as u32
    }

    /// `ParseVertex`: the parent bone, position and normal, then (outside
    /// the vertex animation section) the texture coordinates and bone
    /// links, as far as the line goes; then the next line.
    fn parse_vertex(&self, position: usize, animation_section: bool) -> Result<(Vertex, usize), ImportError> {
        let (mut position, _) = skip_spaces(&self.buffer, position, self.end);
        let mut fields = [0.0f32; 8];
        let field_count = if animation_section { 6 } else { 8 };
        let vertex_from = |fields: &[f32; 8]| Vertex {
            position: [fields[0], fields[1], fields[2]],
            normal: [fields[3], fields[4], fields[5]],
            uv: [fields[6], fields[7]],
        };

        let Some((_, after_parent)) = self.parse_signed_int(position) else {
            return Ok((vertex_from(&fields), self.next_line(position)));
        };
        position = after_parent;
        for field in fields.iter_mut().take(field_count) {
            let Some((value, after)) = self.parse_float(position)? else {
                return Ok((vertex_from(&fields), self.next_line(position)));
            };
            *field = value;
            position = after;
        }
        let vertex = vertex_from(&fields);
        if animation_section {
            return Ok((vertex, self.next_line(position)));
        }

        let Some((link_count, after_count)) = self.parse_unsigned_int(position) else {
            return Ok((vertex, self.next_line(position)));
        };
        position = after_count;
        scene::check_allocation(link_count as u64, LINK_SIZE)?;
        for _ in 0..link_count {
            let Some((_, after_bone)) = self.parse_unsigned_int(position) else {
                return Ok((vertex, self.next_line(position)));
            };
            position = after_bone;
            let Some((_, after_weight)) = self.parse_float(position)? else {
                return Ok((vertex, self.next_line(position)));
            };
            position = after_weight;
        }
        Ok((vertex, self.next_line(position)))
    }

    /// `ParseVASection`: frame 0's vertices, three to a triangle (corners
    /// 1, 2, 0), until "end" or another frame.
    fn parse_vertex_animation_section(&mut self, mut position: usize) -> Result<usize, ImportError> {
        let mut corner = 0usize;
        loop {
            let (next, more) = skip_spaces_and_line_end(&self.buffer, position, self.end);
            position = next;
            if !more {
                break;
            }
            if token_match(&self.buffer, &mut position, b"end") {
                break;
            }
            if token_match(&self.buffer, &mut position, b"time") {
                let Some((time, after)) = self.parse_signed_int(position) else {
                    break;
                };
                position = after;
                if time as u32 != CONFIG_FRAME_ID {
                    break;
                }
                position = self.next_line(position);
            } else {
                if corner == 0 {
                    self.triangles.push(Face::default());
                }
                corner += 1;
                if corner == 3 {
                    corner = 0;
                }
                let (vertex, after) = self.parse_vertex(position, true)?;
                if let Some(triangle) = self.triangles.last_mut() {
                    triangle.vertices[corner] = vertex;
                }
                position = after;
            }
        }
        if corner != 2 && !self.triangles.is_empty() {
            self.triangles.pop();
        }
        Ok(skip_spaces_and_line_end(&self.buffer, position, self.end).0)
    }

    /// `ParseSkeletonSection`: "time" lines and bone keys until "end".
    fn parse_skeleton_section(&mut self, mut position: usize) -> Result<usize, ImportError> {
        loop {
            let (next, more) = skip_spaces_and_line_end(&self.buffer, position, self.end);
            position = next;
            if !more {
                break;
            }
            if token_match(&self.buffer, &mut position, b"end") {
                break;
            } else if token_match(&self.buffer, &mut position, b"time") {
                let Some((_, after)) = self.parse_signed_int(position) else {
                    break;
                };
                position = self.next_line(after);
            } else {
                position = self.parse_skeleton_element(position)?;
            }
        }
        Ok(position)
    }

    /// `ParseSkeletonElement`: a key for a known bone, its matrix the
    /// translation times `FromEulerAnglesXYZ` of the rotation (identity
    /// when the line runs out first).
    fn parse_skeleton_element(&mut self, position: usize) -> Result<usize, ImportError> {
        let Some((bone_index, after_index)) = self.parse_unsigned_int(position) else {
            return Ok(self.next_line(position));
        };
        let mut position = after_index;
        if bone_index as usize >= self.bones.len() {
            return Ok(self.next_line(position));
        }
        let mut values = [0.0f32; 6];
        let mut complete = true;
        for value in &mut values {
            match self.parse_float(position)? {
                Some((parsed, after)) => {
                    *value = parsed;
                    position = after;
                }
                None => {
                    complete = false;
                    break;
                }
            }
        }

        let mut matrix = Matrix4::IDENTITY;
        if complete {
            let [x, y, z, rotation_x, rotation_y, rotation_z] = values;
            let mut rotation = Matrix4::IDENTITY;
            rotation.set_euler_angles_xyz(rotation_x, rotation_y, rotation_z);
            let mut translation = Matrix4::IDENTITY;
            translation.0[3] = x;
            translation.0[7] = y;
            translation.0[11] = z;
            matrix = translation.product(&rotation);
        }
        let bone = &mut self.bones[bone_index as usize];
        if bone.first_key.is_none() {
            bone.first_key = Some(matrix);
        }
        Ok(self.next_line(position))
    }
}

/// `CreateOutputNodes`: a root with the bones under it by their parents
/// (depth first, in index order, each placed by its first key); with no
/// triangles, a lone top bone is the root itself.
fn output_nodes(bones: &[Bone], incomplete: bool) -> Result<(Vec<Node>, usize), ImportError> {
    let mut children_of: Vec<Vec<usize>> = vec![Vec::new(); bones.len()];
    let mut top = Vec::new();
    for (index, bone) in bones.iter().enumerate() {
        if bone.parent == u32::MAX {
            top.push(index);
        } else if let Some(children) = children_of.get_mut(bone.parent as usize) {
            children.push(index);
        }
    }

    let mut nodes = vec![Node::new(b"<SMD_root>".to_vec(), Matrix4::IDENTITY, None)];
    // (node, the bones under it, how deep it is)
    let mut pending = vec![(0usize, top, 0usize)];
    while let Some((node, bone_indices, depth)) = pending.pop() {
        if depth > MAX_DEPTH {
            return undefined("a bone hierarchy too deep for the stack");
        }
        for bone_index in bone_indices {
            let bone = &bones[bone_index];
            let transform = bone.first_key.unwrap_or(Matrix4::IDENTITY);
            let child = nodes.len();
            nodes.push(Node::new(bone.name.clone(), transform, Some(node)));
            nodes[node].children.push(child);
            let grandchildren = std::mem::take(&mut children_of[bone_index]);
            pending.push((child, grandchildren, depth + 1));
        }
    }

    let mut root = 0;
    if incomplete && nodes[0].children.len() == 1 {
        root = nodes[0].children[0];
        nodes[root].parent = None;
    }
    Ok((nodes, root))
}

/// `CreateOutputMeshes`: a mesh per texture (one, untextured, when there
/// are none) with each of its triangles' three vertices.
fn output_meshes(parser: &Parser) -> Result<Vec<Mesh>, ImportError> {
    let mesh_count = parser.textures.len().max(1);
    let mut face_lists: Vec<Vec<usize>> = vec![Vec::new(); mesh_count];
    for (index, triangle) in parser.triangles.iter().enumerate() {
        let Some(list) = face_lists.get_mut(triangle.texture as usize) else {
            return undefined("a texture index past the meshes");
        };
        list.push(index);
    }

    let mut meshes = Vec::with_capacity(mesh_count);
    for (material_index, face_list) in face_lists.iter().enumerate() {
        let vertex_count = face_list.len() * 3;
        let mut vertices = Vec::with_capacity(vertex_count);
        let mut normals = Vec::with_capacity(vertex_count);
        let mut coords = Vec::with_capacity(vertex_count);
        let mut faces = Vec::with_capacity(face_list.len());
        for &triangle_index in face_list {
            let triangle = &parser.triangles[triangle_index];
            let first = vertices.len() as u32;
            for vertex in &triangle.vertices {
                vertices.push(vertex.position);
                normals.push(vertex.normal);
                coords.push([vertex.uv[0], vertex.uv[1], 0.0]);
            }
            faces.push(vec![first, first + 1, first + 2]);
        }
        let mut mesh = Mesh {
            primitive_types: PRIMITIVE_TRIANGLE,
            vertex_count,
            vertices: Some(vertices),
            normals: Some(normals),
            faces: Some(faces),
            material_index: material_index as u32,
            ..Mesh::default()
        };
        if parser.has_uvs {
            mesh.uv_components[0] = 2;
            mesh.texture_coords[0] = Some(coords);
        }
        meshes.push(mesh);
    }
    Ok(meshes)
}

/// `DefaultIOSystem::absolutePath`: up to the last separator (the whole
/// path when it has none).
fn absolute_path(path: &[u8]) -> Vec<u8> {
    match path.iter().rposition(|&byte| byte == b'\\' || byte == b'/') {
        Some(separator) => path[..separator].to_vec(),
        None => path.to_vec(),
    }
}

/// `DefaultIOSystem::completeBaseName`: the file name less its last
/// extension.
fn complete_base_name(path: &[u8]) -> Vec<u8> {
    let file_name = match path.iter().rposition(|&byte| byte == b'\\' || byte == b'/') {
        Some(separator) => &path[separator + 1..],
        None => path,
    };
    match file_name.iter().rposition(|&byte| byte == b'.') {
        Some(dot) => file_name[..dot].to_vec(),
        None => file_name.to_vec(),
    }
}

/// `GetAnimationFileList`'s `strtok` parse of the list: per line, the
/// second word (or the only one) is a file's path.
fn animation_paths(buffer: &[u8]) -> Vec<Vec<u8>> {
    let text = match buffer.iter().position(|&byte| byte == 0) {
        Some(nul) => &buffer[..nul],
        None => buffer,
    };
    let mut paths = Vec::new();
    for line in text.split(|&byte| byte == b'\r' || byte == b'\n') {
        let mut words = line.split(|&byte| byte == b' ' || byte == b'\t').filter(|word| !word.is_empty());
        let Some(first) = words.next() else {
            continue;
        };
        let path = words.next().unwrap_or(first);
        paths.push(path.to_vec());
    }
    paths
}

/// `CreateOutputAnimations`' reading: the files `<name>_animation.txt`
/// lists, relative to the file's folder, each opened and parsed.
fn read_animation_files(path: &str, file_system: &FileSystem) -> Result<(), ImportError> {
    let base = absolute_path(path.as_bytes());
    let name = complete_base_name(path.as_bytes());
    let mut list_path = base.clone();
    list_path.push(b'/');
    list_path.extend_from_slice(&name);
    list_path.extend_from_slice(b"_animation.txt");
    let Some(list) = file_system.open_bytes(&list_path) else {
        return Ok(());
    };
    let buffer = text_file_to_buffer(&list, false)?;
    for animation_path in animation_paths(&buffer) {
        let mut full_path = base.clone();
        full_path.push(b'/');
        full_path.extend_from_slice(&animation_path);
        let Some(bytes) = file_system.open_bytes(&full_path) else {
            let shown_path = String::from_utf8_lossy(&full_path);
            return fail(format!("Failed to open SMD/VTA file {shown_path}."));
        };
        Parser::read(&bytes)?;
    }
    Ok(())
}

pub fn read(path: &str, bytes: &[u8], files: &FileSystem) -> Result<Scene, ImportError> {
    let parser = Parser::read(bytes)?;
    let incomplete = parser.triangles.is_empty();
    if incomplete && parser.bones.is_empty() {
        return fail("SMD: No triangles and no bones have been found in the file. This file seems to be invalid.");
    }

    let (nodes, root) = output_nodes(&parser.bones, incomplete)?;
    let mut meshes = Vec::new();
    if !incomplete {
        meshes = output_meshes(&parser)?;
        // `CreateOutputMaterials` copies `length + 5` bytes of the
        // 1028-byte `aiString` it set from the name.
        if parser.textures.iter().any(|texture| texture.len() >= MAX_LENGTH) {
            return undefined("a texture name copied past its string");
        }
    }
    read_animation_files(path, files)?;
    if incomplete {
        meshes.push(skeleton::build_mesh(&nodes, root, false));
    }
    Ok(Scene { meshes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Parser {
        Parser::read(text.as_bytes()).unwrap()
    }

    #[test]
    fn triangles_group_by_texture_in_any_case() {
        let parser = parse("version 1\ntriangles\nA.bmp\n0 1 2 3 0 0 1 0.5 0.5\n0 4 5 6 0 0 1 0 0\n0 7 8 9 0 0 1 1 1\na.BMP\n0 1 1 1 0 0 1 0 0\n0 2 2 2 0 0 1 0 0\n  \nend\n");
        assert_eq!(parser.textures.len(), 1);
        assert_eq!(parser.triangles.len(), 2);
        assert_eq!(parser.triangles[0].vertices[1].position, [4.0, 5.0, 6.0]);
        // The line of blanks is the second triangle's third vertex.
        assert_eq!(parser.triangles[1].vertices[2].position, [0.0, 0.0, 0.0]);
        assert!(parser.has_uvs);
    }

    #[test]
    fn vertex_animation_fills_corners_one_two_zero() {
        let parser = parse("vertexanimation\ntime 0\n0 1 1 1 0 0 1\n1 2 2 2 0 0 1\n2 3 3 3 0 0 1\n3 4 4 4 0 0 1\n4 5 5 5 0 0 1\nend\n");
        // Five vertices: the second triangle stopped after two corners and stays.
        assert_eq!(parser.triangles.len(), 2);
        assert_eq!(parser.triangles[0].vertices[1].position, [1.0, 1.0, 1.0]);
        assert_eq!(parser.triangles[0].vertices[0].position, [3.0, 3.0, 3.0]);
        assert_eq!(parser.triangles[1].vertices[2].position, [5.0, 5.0, 5.0]);
        assert!(!parser.has_uvs);
    }

    #[test]
    fn an_indented_end_names_a_bone() {
        let parser = parse("nodes\n0 \"root\" -1\n  end\nend\n");
        assert_eq!(parser.bones.len(), 1);
        assert_eq!(parser.bones[0].name, b"end");
    }

    #[test]
    fn faults() {
        assert!(Parser::read(b"triangles\n\x0cx\nend\n").is_err());
        assert!(Parser::read(b"nodes\n0 \"a\" -1\nend").is_err());
        assert!(Parser::read(b"triangles\nt\n0 x 1 1 0 0 1 0 0\nend\n").is_err());
    }
}
