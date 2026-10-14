//! Assimp's `ColladaLoader::InternReadFile` (ColladaLoader.cpp): the node
//! hierarchy, one `aiMesh` per (mesh or controller, sub-mesh, material),
//! the material and animation passes for the errors they can raise, and
//! `SkeletonMeshBuilder` when no mesh results.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::parser::{
    Accessor, Animation, Controller, ControllerType, Data, GARBAGE_INDEX, Mesh, Node, Parser, SubMesh, Transform, TransformType, UpDirection,
};
use crate::matrix::Matrix4;
use crate::node::Node as AiNode;
use crate::scene::{self, ImportError, MAX_COLOR_SETS, MAX_TEXTURE_COORDS, Scene, fail};
use crate::skeleton;

/// Recursion the C++ would only end with a stack overflow.
const MAX_DEPTH: usize = 2048;

/// Animation time steps before the C++ would run out of memory.
const MAX_ANIMATION_STEPS: u64 = 1 << 24;

/// `AI_MAXLEN - 1`: longer strings leave an `aiString` empty.
const MAX_AI_STRING: usize = 1023;

/// An animation channel matched to a node (`ChannelEntry`).
struct ChannelEntry<'a> {
    times: &'a [u8],
    values: &'a [u8],
    transform_index: usize,
    sub_element: usize,
}

struct Loader<'a> {
    parser: &'a Parser,
    meshes: Vec<scene::Mesh>,
    mesh_names: Vec<Vec<u8>>,
    target_mesh_names: Vec<Vec<u8>>,
    mesh_index_by_id: HashMap<(Vec<u8>, usize, Vec<u8>), usize>,
    material_index_by_name: BTreeMap<Vec<u8>, usize>,
    material_effects: Vec<Vec<u8>>,
    node_name_counter: u64,
    nodes: Vec<AiNode>,
}

/// `aiString::Set(std::string)`: unchanged (empty here) when too long.
fn ai_string_set(value: &[u8]) -> Vec<u8> {
    if value.len() > MAX_AI_STRING { Vec::new() } else { value.to_vec() }
}

/// `std::string::find` (an empty needle matches at the start).
fn find_bytes(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from > haystack.len() {
        return None;
    }
    if needle.is_empty() {
        return Some(from);
    }
    haystack[from..].windows(needle.len()).position(|window| window == needle).map(|position| position + from)
}

/// `std::string::substr(position, count)` for a valid position.
fn substring(value: &[u8], position: usize, count: usize) -> Vec<u8> {
    let end = position.saturating_add(count).min(value.len());
    value[position..end].to_vec()
}

/// `ColladaParser::ResolveLibraryReference`.
fn resolve<'a, T>(library: &'a BTreeMap<Vec<u8>, T>, key: &[u8]) -> Result<&'a T, ImportError> {
    match library.get(key) {
        Some(value) => Ok(value),
        None => fail("Unable to resolve library reference."),
    }
}

/// `ColladaLoader::ReadFloat`, `None` past the data.
fn read_float(accessor: &Accessor, data: &Data, index: u64, offset: u64) -> Option<f32> {
    let position = accessor.stride.wrapping_mul(index).wrapping_add(accessor.offset).wrapping_add(offset);
    data.values.get(position as usize).copied()
}

/// `std::min` / `std::max` argument order: `(b < a) ? b : a`.
fn min_f32(a: f32, b: f32) -> f32 {
    if b < a { b } else { a }
}

/// `static_cast<int>` of a double under MSVC: out of range is `INT_MIN`.
fn msvc_double_to_int(value: f64) -> i32 {
    if value.is_nan() || value >= 2147483648.0 || value < -2147483648.0 {
        return i32::MIN;
    }
    value as i32
}

/// `FindNode`: by name or id, depth first.
fn find_node(parser: &Parser, node: usize, name: &[u8]) -> Option<usize> {
    let collada = &parser.nodes[node];
    if collada.name == name || collada.id == name {
        return Some(node);
    }
    for &child in &collada.children {
        if let Some(found) = find_node(parser, child, name) {
            return Some(found);
        }
    }
    None
}

/// `CalculateResultTransform` (skew steps are ignored).
fn calculate_result_transform(transforms: &[Transform]) -> Matrix4 {
    let mut result = Matrix4::IDENTITY;
    for transform in transforms {
        let values = &transform.values;
        let step = match transform.kind {
            TransformType::LookAt => Matrix4::look_at(values),
            TransformType::Rotate => {
                let angle = values[3] * std::f32::consts::PI / 180.0;
                Matrix4::rotation(angle, [values[0], values[1], values[2]])
            }
            TransformType::Translate => Matrix4::translation([values[0], values[1], values[2]]),
            TransformType::Scale => {
                let mut scaling = Matrix4::IDENTITY;
                scaling.0[0] = values[0];
                scaling.0[5] = values[1];
                scaling.0[10] = values[2];
                scaling
            }
            TransformType::Skew => continue,
            TransformType::Matrix => Matrix4(*values),
        };
        result.multiply_assign(&step);
    }
    result
}

impl<'a> Loader<'a> {
    /// `BuildMaterials`: materials whose effect exists, in id order.
    fn build_materials(&mut self) {
        for (id, material) in &self.parser.materials {
            if !self.parser.effects.contains_key(&material.effect) {
                continue;
            }
            self.material_index_by_name.insert(id.clone(), self.material_effects.len());
            self.material_effects.push(material.effect.clone());
        }
    }

    /// `FindNameForNode`: id, else sid, else a counted placeholder.
    fn find_name_for_node(&mut self, node: &Node) -> Vec<u8> {
        if !node.id.is_empty() {
            return node.id.clone();
        }
        if !node.sid.is_empty() {
            return node.sid.clone();
        }
        let name = format!("$ColladaAutoName$_{}", self.node_name_counter);
        self.node_name_counter += 1;
        name.into_bytes()
    }

    /// `BuildHierarchy`: children, then instanced nodes, then this node's
    /// meshes (so meshes are created in post-order).
    fn build_hierarchy(&mut self, node: usize, depth: usize) -> Result<usize, ImportError> {
        if depth > MAX_DEPTH {
            return fail("Collada: node instances recurse without end");
        }
        let parser = self.parser;
        let collada = &parser.nodes[node];
        let name = self.find_name_for_node(collada);
        let transform = calculate_result_transform(&collada.transforms);
        self.nodes.push(AiNode { name: ai_string_set(&name), transform, parent: None, children: Vec::new() });
        let ai_node = self.nodes.len() - 1;

        let mut instances = Vec::new();
        for instance in &collada.node_instances {
            let library_node = parser.node_library.get(instance).copied();
            let found = library_node.or_else(|| find_node(parser, parser.root?, instance));
            if let Some(found) = found {
                instances.push(found);
            }
        }
        for &child in collada.children.iter().chain(instances.iter()) {
            let ai_child = self.build_hierarchy(child, depth + 1)?;
            self.nodes[ai_child].parent = Some(ai_node);
            self.nodes[ai_node].children.push(ai_child);
        }
        self.build_meshes_for_node(collada)?;
        Ok(ai_node)
    }

    /// `findMesh`: a created mesh or morph target with the name.
    fn find_mesh(&self, id: &[u8]) -> bool {
        if id.is_empty() {
            return false;
        }
        self.mesh_names.iter().any(|name| name == id) || self.target_mesh_names.iter().any(|name| name == id)
    }

    /// `BuildMeshesForNode` (sub-mesh offsets only advance when a mesh is
    /// created, as in the C++).
    fn build_meshes_for_node(&mut self, node: &Node) -> Result<(), ImportError> {
        let parser = self.parser;
        for instance in &node.meshes {
            let (source, controller) = match parser.meshes.get(&instance.mesh_or_controller) {
                Some(mesh) => (mesh, None),
                None => {
                    let Some(controller) = parser.controllers.get(&instance.mesh_or_controller) else {
                        continue;
                    };
                    let Some(mesh) = parser.meshes.get(&controller.mesh_id) else {
                        continue;
                    };
                    (mesh, Some(controller))
                }
            };
            let mut vertex_start = 0u64;
            let mut face_start = 0u64;
            for (index, sub_mesh) in source.sub_meshes.iter().enumerate() {
                if sub_mesh.face_count == 0 {
                    continue;
                }
                let binding = instance.materials.get(&sub_mesh.material);
                let mesh_material = match binding {
                    Some(binding) => binding.material.clone(),
                    None => instance.materials.values().next().map(|first| first.material.clone()).unwrap_or_default(),
                };
                // `newMats[matIdx]` with no materials reads garbage.
                if let Some(binding) = binding
                    && binding.has_inputs
                    && self.material_effects.is_empty()
                {
                    return fail("Collada: vertex input binding without materials");
                }
                let key = (instance.mesh_or_controller.clone(), index, mesh_material);
                if self.mesh_index_by_id.contains_key(&key) {
                    continue;
                }
                let (mesh, mut name) = self.create_mesh(source, sub_mesh, controller, vertex_start, face_start, 0)?;
                self.mesh_index_by_id.insert(key, self.meshes.len());
                vertex_start += mesh.vertex_count as u64;
                face_start += sub_mesh.face_count;
                if name.is_empty() {
                    name = ai_string_set(&instance.mesh_or_controller);
                }
                self.meshes.push(mesh);
                self.mesh_names.push(name);
            }
        }
        Ok(())
    }

    /// `CreateMesh`: the sub-mesh's slice of the streams, then the morph
    /// and skin checks.
    fn create_mesh(
        &mut self,
        source: &Mesh,
        sub_mesh: &SubMesh,
        controller: Option<&Controller>,
        vertex_start: u64,
        face_start: u64,
        depth: usize,
    ) -> Result<(scene::Mesh, Vec<u8>), ImportError> {
        if depth > MAX_DEPTH {
            return fail("Collada: morph targets recurse without end");
        }
        let name = ai_string_set(&source.id);
        let mut mesh = scene::Mesh::default();
        if source.positions.is_empty() {
            return Ok((mesh, name));
        }

        let face_end = face_start.wrapping_add(sub_mesh.face_count);
        if face_end > source.face_sizes.len() as u64 || face_end < face_start {
            return fail("Collada: sub-mesh faces outside the mesh");
        }
        let face_sizes = &source.face_sizes[face_start as usize..face_end as usize];
        let vertex_count = face_sizes.iter().fold(0u64, |sum, &size| sum.wrapping_add(size));
        let vertex_end = vertex_start.wrapping_add(vertex_count);
        if vertex_end > source.positions.len() as u64 || vertex_end < vertex_start {
            return fail("Collada: sub-mesh vertices outside the mesh");
        }
        let range = vertex_start as usize..vertex_end as usize;
        let end = vertex_end as usize;
        mesh.vertex_count = vertex_count as usize;
        mesh.vertices = Some(source.positions[range.clone()].to_vec());
        if source.normals.len() >= end {
            mesh.normals = Some(source.normals[range.clone()].to_vec());
        }
        if source.tangents.len() >= end {
            mesh.tangents = Some(source.tangents[range.clone()].to_vec());
        }
        if source.bitangents.len() >= end {
            mesh.bitangents = Some(source.bitangents[range.clone()].to_vec());
        }
        for set in 0..MAX_TEXTURE_COORDS {
            if source.texture_coords[set].len() >= end {
                mesh.texture_coords[set] = Some(source.texture_coords[set][range.clone()].to_vec());
                mesh.uv_components[set] = source.uv_components[set];
            }
        }
        let mut color_set = 0;
        for set in 0..MAX_COLOR_SETS {
            if source.colors[set].len() >= end {
                mesh.colors[color_set] = Some(source.colors[set][range.clone()].to_vec());
                color_set += 1;
            }
        }
        let mut faces = Vec::with_capacity(face_sizes.len());
        let mut vertex = 0u32;
        for &size in face_sizes {
            let face: Vec<u32> = (0..size as u32).map(|offset| vertex.wrapping_add(offset)).collect();
            vertex = vertex.wrapping_add(size as u32);
            faces.push(face);
        }
        mesh.faces = Some(faces);

        self.check_morph_targets(source, depth)?;
        if let Some(controller) = controller
            && controller.kind == ControllerType::Skin
        {
            self.check_skin(source, controller, vertex_start, vertex_count)?;
        }
        Ok((mesh, name))
    }

    /// The morph part of `CreateMesh`: every controller's mesh must
    /// resolve, and morphs of a mesh with this one's name need their
    /// targets (which are created when missing).
    fn check_morph_targets(&mut self, source: &Mesh, depth: usize) -> Result<(), ImportError> {
        let parser = self.parser;
        for controller in parser.controllers.values() {
            let base = resolve(&parser.meshes, &controller.mesh_id)?;
            if controller.kind != ControllerType::Morph || base.name != source.name {
                continue;
            }
            let target_accessor = resolve(&parser.accessors, &controller.morph_target)?;
            let weight_accessor = resolve(&parser.accessors, &controller.morph_weight)?;
            let target_data = resolve(&parser.data, &target_accessor.source)?;
            let weight_data = resolve(&parser.data, &weight_accessor.source)?;
            if !target_data.is_string_array {
                return fail("target data must contain id. ");
            }
            if weight_data.is_string_array {
                return fail("target weight data must not be textual ");
            }
            for target_id in &target_data.strings {
                let target = resolve(&parser.meshes, target_id)?;
                if self.find_mesh(&target.id) {
                    continue;
                }
                if target.sub_meshes.len() > 1 {
                    return fail("Morphing target mesh must be a single");
                }
                let Some(first) = target.sub_meshes.first() else {
                    return fail("invalid vector subscript");
                };
                let (_, target_name) = self.create_mesh(target, first, None, 0, 0, depth + 1)?;
                self.target_mesh_names.push(target_name);
            }
        }
        Ok(())
    }

    /// The skin part of `CreateMesh`: the joint, matrix and weight sources
    /// must resolve and agree, and the weights of this sub-mesh's
    /// vertices must stay inside their arrays.
    fn check_skin(&self, source: &Mesh, controller: &Controller, vertex_start: u64, vertex_count: u64) -> Result<(), ImportError> {
        let parser = self.parser;
        let joint_names_accessor = resolve(&parser.accessors, &controller.joint_name_source)?;
        let joint_names = resolve(&parser.data, &joint_names_accessor.source)?;
        let joint_matrix_accessor = resolve(&parser.accessors, &controller.joint_offset_matrix_source)?;
        let joint_matrices = resolve(&parser.data, &joint_matrix_accessor.source)?;
        resolve(&parser.accessors, &controller.weight_input_joints.accessor)?;
        if controller.weight_input_joints.accessor != controller.joint_name_source {
            return fail("Temporary implementational laziness. If you read this, please report to the author.");
        }
        let weights_accessor = resolve(&parser.accessors, &controller.weight_input_weights.accessor)?;
        let weights = resolve(&parser.data, &weights_accessor.source)?;
        if !joint_names.is_string_array || joint_matrices.is_string_array || weights.is_string_array {
            return fail("Data type mismatch while resolving mesh joints");
        }
        if controller.weight_input_joints.offset != 0 || controller.weight_input_weights.offset != 1 {
            return fail("Unsupported vertex_weight addressing scheme. ");
        }

        let bone_count = joint_names.strings.len();
        let mut bone_has_weights = vec![false; bone_count];
        let mut weight_starts = Vec::with_capacity(controller.weight_counts.len());
        let mut cursor = 0u64;
        for &count in &controller.weight_counts {
            weight_starts.push(cursor);
            cursor = cursor.wrapping_add(count);
        }
        for vertex in vertex_start..vertex_start.wrapping_add(vertex_count) {
            let original = source.face_position_indices.get(vertex as usize).copied().unwrap_or(GARBAGE_INDEX);
            let Some(&start) = weight_starts.get(original as usize) else {
                return fail("Collada: vertex weights outside their arrays");
            };
            let pair_count = controller.weight_counts[original as usize];
            for pair in 0..pair_count {
                let Some(&(joint, vertex_index)) = controller.weights.get(start.wrapping_add(pair) as usize) else {
                    return fail("Collada: vertex weights outside their arrays");
                };
                let mut weight = 1.0f32;
                if !weights.values.is_empty() {
                    let Some(value) = read_float(weights_accessor, weights, vertex_index, 0) else {
                        return fail("Collada: vertex weights outside their arrays");
                    };
                    weight = value;
                }
                if weight > 0.0 {
                    let Some(has_weights) = bone_has_weights.get_mut(joint as usize) else {
                        return fail("Collada: vertex weights outside their arrays");
                    };
                    *has_weights = true;
                }
            }
        }
        // Bones without weights are dropped; the others read their name.
        for (bone, has_weights) in bone_has_weights.iter().enumerate() {
            if !has_weights {
                continue;
            }
            let position = joint_names_accessor.stride.wrapping_mul(bone as u64).wrapping_add(joint_names_accessor.offset);
            if position >= joint_names.strings.len() as u64 {
                return fail("Collada: joint name outside its array");
            }
        }
        Ok(())
    }

    /// `FillMaterials`: each texture slot's sampler chain must end at an
    /// image with data or a file name, or at no image (a param cycle never
    /// ends in the C++).
    fn fill_materials(&self) -> Result<(), ImportError> {
        let parser = self.parser;
        for effect_id in &self.material_effects {
            let effect = &parser.effects[effect_id];
            let samplers = [&effect.ambient, &effect.emissive, &effect.specular, &effect.diffuse, &effect.bump, &effect.transparent, &effect.reflective];
            for sampler in samplers {
                if sampler.is_empty() {
                    continue;
                }
                let mut name = sampler.clone();
                let mut visited = BTreeSet::new();
                while let Some(reference) = effect.params.get(&name) {
                    if !visited.insert(name.clone()) {
                        return fail("Collada: effect parameters reference each other");
                    }
                    name = reference.clone();
                }
                if let Some(image) = parser.images.get(&name)
                    && image.data_size == 0
                    && image.file_name.is_empty()
                {
                    return fail("Collada: Invalid texture, no data or file reference given");
                }
            }
        }
        Ok(())
    }

    /// `CollectNodes`: pre-order.
    fn collect_nodes(&self, node: usize, nodes: &mut Vec<usize>) {
        nodes.push(node);
        for &child in &self.nodes[node].children {
            self.collect_nodes(child, nodes);
        }
    }

    /// `StoreAnimations`: sub-animations first, then the animation's own
    /// channels.
    fn store_animations(&self, animation: usize, nodes: &[usize]) -> Result<(), ImportError> {
        let parser = self.parser;
        let animation = &parser.animations[animation];
        for &sub in &animation.sub_animations {
            self.store_animations(sub, nodes)?;
        }
        if !animation.channels.is_empty() {
            self.create_animation(animation, nodes)?;
        }
        Ok(())
    }

    /// `CreateAnimation`, for what can throw, crash or never finish: the
    /// channel targets, the sources, and the key-time loop.
    fn create_animation(&self, animation: &Animation, nodes: &[usize]) -> Result<(), ImportError> {
        let parser = self.parser;
        let Some(root) = parser.root else {
            return Ok(());
        };
        for &ai_node in nodes {
            let node_name = &self.nodes[ai_node].name;
            let Some(source_node) = find_node(parser, root, node_name) else {
                continue;
            };
            let source_node = &parser.nodes[source_node];
            let mut entries = Vec::new();
            for channel in &animation.channels {
                let target = channel.target.as_slice();
                let mut entry = ChannelEntry { times: &channel.source_times, values: &channel.source_values, transform_index: 0, sub_element: 0 };
                let Some(slash) = find_bytes(target, b"/", 0) else {
                    let Some(target_position) = find_bytes(target, &source_node.id, 0) else {
                        continue;
                    };
                    if target_position + animation.name.len() > target.len() {
                        return fail("invalid string position");
                    }
                    entries.push(entry);
                    continue;
                };
                if find_bytes(target, b"/", slash + 1).is_some() {
                    continue;
                }
                if target[..slash] != source_node.id[..] {
                    continue;
                }
                let mut transform_id;
                if let Some(dot) = find_bytes(target, b".", 0) {
                    if find_bytes(target, b".", dot + 1).is_some() {
                        continue;
                    }
                    transform_id = substring(target, slash + 1, dot.wrapping_sub(slash).wrapping_sub(1));
                    match &target[dot + 1..] {
                        b"ANGLE" => entry.sub_element = 3,
                        b"X" => entry.sub_element = 0,
                        b"Y" => entry.sub_element = 1,
                        b"Z" => entry.sub_element = 2,
                        _ => {}
                    }
                } else {
                    transform_id = target[slash + 1..].to_vec();
                }
                if let Some(bracket) = find_bytes(target, b"(", 0) {
                    transform_id = substring(target, slash + 1, bracket.wrapping_sub(slash).wrapping_sub(1));
                    let element = &target[bracket..];
                    let patterns: [&[u8]; 16] = [
                        b"(0)(0)", b"(1)(0)", b"(2)(0)", b"(3)(0)", b"(0)(1)", b"(1)(1)", b"(2)(1)", b"(3)(1)", b"(0)(2)", b"(1)(2)", b"(2)(2)",
                        b"(3)(2)", b"(0)(3)", b"(1)(3)", b"(2)(3)", b"(3)(3)",
                    ];
                    if let Some(sub_element) = patterns.iter().position(|pattern| *pattern == element) {
                        entry.sub_element = sub_element;
                    }
                }
                entry.transform_index = usize::MAX;
                for (index, transform) in source_node.transforms.iter().enumerate() {
                    if transform.id == transform_id {
                        entry.transform_index = index;
                    }
                }
                if entry.transform_index == usize::MAX && find_bytes(&transform_id, b"morph-weights", 0).is_none() {
                    continue;
                }
                entries.push(entry);
            }
            if entries.is_empty() {
                continue;
            }
            self.run_key_times(source_node, &entries)?;
        }
        Ok(())
    }

    /// The key-time walk of `CreateAnimation`: data resolution, writes
    /// into the node's transform chain, and the stepping from key to key
    /// (with sub-sampling of large rotations).
    fn run_key_times(&self, node: &Node, entries: &[ChannelEntry]) -> Result<(), ImportError> {
        let parser = self.parser;
        let mut resolved = Vec::with_capacity(entries.len());
        let mut start_time = 1e20f32;
        for entry in entries {
            let time_accessor = resolve(&parser.accessors, entry.times)?;
            let time_data = resolve(&parser.data, &time_accessor.source)?;
            let value_accessor = resolve(&parser.accessors, entry.values)?;
            let value_data = resolve(&parser.data, &value_accessor.source)?;
            if time_accessor.count != value_accessor.count {
                return fail("Time count / value count mismatch in animation channel.");
            }
            if time_accessor.count > 0 {
                let Some(first) = read_float(time_accessor, time_data, 0, 0) else {
                    return fail("Collada: key time outside its data");
                };
                start_time = min_f32(start_time, first);
            }
            resolved.push((entry, time_accessor, time_data, value_accessor, value_data));
        }
        if resolved[0].1.count == 0 {
            return Ok(());
        }

        let transforms = &node.transforms;
        let mut time = start_time;
        let mut steps = 0u64;
        loop {
            for &(entry, _, _, value_accessor, _) in &resolved {
                if value_accessor.size > 16 {
                    return fail("Collada: animation values overrun their buffer");
                }
                if value_accessor.size == 0 {
                    continue;
                }
                let Some(_) = transforms.get(entry.transform_index) else {
                    return fail("Collada: animation channel without its transform");
                };
                if entry.sub_element as u64 + value_accessor.size > 16 {
                    return fail("Collada: animation values overrun their transform");
                }
            }

            let mut next_time = 1e20f32;
            for &(entry, time_accessor, time_data, value_accessor, value_data) in &resolved {
                let count = time_accessor.count;
                let mut position = 0u64;
                while position < count {
                    let Some(key_time) = read_float(time_accessor, time_data, position, 0) else {
                        return fail("Collada: key time outside its data");
                    };
                    if key_time > time {
                        next_time = min_f32(next_time, key_time);
                        break;
                    }
                    position += 1;
                }
                let kind = match transforms.get(entry.transform_index) {
                    Some(transform) => Some(transform.kind),
                    None if transforms.is_empty() => return fail("Collada: animation channel without its transform"),
                    None => None,
                };
                if entry.sub_element != 3 || position == 0 || position >= count {
                    continue;
                }
                let Some(kind) = kind else {
                    return fail("Collada: animation channel without its transform");
                };
                if kind != TransformType::Rotate {
                    continue;
                }
                let reads = [
                    read_float(value_accessor, value_data, position, 0),
                    read_float(value_accessor, value_data, position - 1, 0),
                    read_float(time_accessor, time_data, position, 0),
                    read_float(time_accessor, time_data, position - 1, 0),
                ];
                let [Some(current_angle), Some(last_angle), Some(current_time), Some(last_time)] = reads else {
                    return fail("Collada: key outside its data");
                };
                let progress = (current_angle - last_angle) * (time - last_time) / (current_time - last_time);
                let last_evaluated = last_angle + progress;
                let delta = (current_angle - last_evaluated).abs();
                if delta as f64 >= 180.0 {
                    let sample_count = msvc_double_to_int((delta as f64 / 90.0).floor());
                    if current_time != time {
                        let next_sample = time + (current_time - time) / sample_count as f32;
                        next_time = min_f32(next_time, next_sample);
                    }
                }
            }

            if next_time as f64 > 1e19 {
                break;
            }
            time = next_time;
            steps += 1;
            if steps > MAX_ANIMATION_STEPS {
                return fail("Collada: animation keys never end");
            }
        }
        Ok(())
    }
}

/// `ColladaLoader::InternReadFile` after parsing.
pub fn load(parser: &Parser) -> Result<Scene, ImportError> {
    let Some(root) = parser.root else {
        return fail("Collada: File came out empty. Something is wrong here.");
    };
    let mut loader = Loader {
        parser,
        meshes: Vec::new(),
        mesh_names: Vec::new(),
        target_mesh_names: Vec::new(),
        mesh_index_by_id: HashMap::new(),
        material_index_by_name: BTreeMap::new(),
        material_effects: Vec::new(),
        node_name_counter: 0,
        nodes: Vec::new(),
    };
    loader.build_materials();
    let ai_root = loader.build_hierarchy(root, 0)?;
    loader.fill_materials()?;

    let unit = parser.unit_size;
    let mut unit_scale = Matrix4::IDENTITY;
    unit_scale.0[0] = unit;
    unit_scale.0[5] = unit;
    unit_scale.0[10] = unit;
    loader.nodes[ai_root].transform.multiply_assign(&unit_scale);
    let up_conversion = match parser.up_direction {
        UpDirection::X => Some(Matrix4([0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0])),
        UpDirection::Z => Some(Matrix4([1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0])),
        UpDirection::Y => None,
    };
    if let Some(conversion) = up_conversion {
        loader.nodes[ai_root].transform.multiply_assign(&conversion);
    }

    let mut nodes = Vec::new();
    loader.collect_nodes(ai_root, &mut nodes);
    loader.store_animations(parser.root_animation, &nodes)?;

    let mut meshes = std::mem::take(&mut loader.meshes);
    for (mesh, name) in meshes.iter_mut().zip(&loader.mesh_names) {
        mesh.name = String::from_utf8_lossy(name).into_owned();
    }
    if meshes.is_empty() {
        meshes.push(skeleton::build_mesh(&loader.nodes, ai_root, false));
    }
    Ok(Scene { meshes })
}
