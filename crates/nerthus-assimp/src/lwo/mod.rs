//! LightWave and Modo objects (`LWOImporter`): LWOB, LWO2, LWO3 and LXOB
//! files, big-endian IFF chunks.
//!
//! Each layer's faces are sorted by surface into a mesh per surface, a
//! vertex per face corner. A surface's texture blocks decide which UV
//! maps a mesh gets (`FindUVChannels`), its vertex colour map which colour
//! maps; normals come from a Modo normal map or from the faces: per face
//! when the surface has no smoothing angle, smoothed over equal positions
//! of a smoothing group for angles from 3 radians up, and zero between
//! (the smoothed sums are computed there but never stored).
//!
//! Reads go through a copy of the file with unchecked offsets: a read
//! outside it is undefined behaviour, an error here.

mod parse;

use crate::smoothing_groups::SgSpatialSort;
use crate::scene::{ImportError, MAX_COLOR_SETS, MAX_TEXTURE_COORDS, Mesh, Scene, check_allocation, fail};
use crate::vector::{Vector3, cross, normalize, sub};

/// Chains of copied points longer than this overflow the recursive walks
/// along them.
const MAX_CHAIN: usize = 16384;

/// `sizeof(aiVector3D)`.
const VECTOR_SIZE: u64 = 12;

/// IFF type codes (big-endian).
const fn fourcc(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}

const FORM: u32 = fourcc(b"FORM");
const FACE: u32 = fourcc(b"FACE");
const PTCH: u32 = fourcc(b"PTCH");
const BONE: u32 = fourcc(b"BONE");
const SUBD: u32 = fourcc(b"SUBD");
const RGB: u32 = fourcc(b"RGB ");
const RGBA: u32 = fourcc(b"RGBA");

/// `LWO::Texture::UV`.
const MAPPING_UV: u32 = 5;

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// `AI_LWO_VALIDATE_CHUNK_LENGTH`.
fn validate_length(length: u32, name: &str, size: u32) -> Result<(), ImportError> {
    if length < size {
        return fail(format!("LWO: {name} chunk is too small"));
    }
    Ok(())
}

/// The importer's copy of the file and its cursor (`mFileBuffer`), an
/// offset from the copy's start that may point anywhere. Multi-byte values
/// are big-endian; some chunks are byte-swapped in place.
pub(crate) struct Reader {
    data: Vec<u8>,
    cursor: i64,
}

impl Reader {
    fn bytes(&self, position: i64, count: usize) -> Result<&[u8], ImportError> {
        let inside = position >= 0 && (position as u64).checked_add(count as u64).is_some_and(|end| end <= self.data.len() as u64);
        if !inside {
            return undefined("a read past the file");
        }
        let start = position as usize;
        Ok(&self.data[start..start + count])
    }

    fn array<const N: usize>(&self, position: i64) -> Result<[u8; N], ImportError> {
        let mut array = [0u8; N];
        array.copy_from_slice(self.bytes(position, N)?);
        Ok(array)
    }

    fn byte(&self, position: i64) -> Result<u8, ImportError> {
        Ok(self.array::<1>(position)?[0])
    }

    /// `GetU1`.
    fn u1(&mut self) -> Result<u8, ImportError> {
        let value = self.byte(self.cursor)?;
        self.cursor += 1;
        Ok(value)
    }

    /// `GetU2`.
    fn u2(&mut self) -> Result<u16, ImportError> {
        let value = u16::from_be_bytes(self.array(self.cursor)?);
        self.cursor += 2;
        Ok(value)
    }

    /// `GetU4`.
    fn u4(&mut self) -> Result<u32, ImportError> {
        let value = u32::from_be_bytes(self.array(self.cursor)?);
        self.cursor += 4;
        Ok(value)
    }

    /// `GetU8`.
    fn u8_value(&mut self) -> Result<u64, ImportError> {
        let value = u64::from_be_bytes(self.array(self.cursor)?);
        self.cursor += 8;
        Ok(value)
    }

    /// `GetF4`.
    fn f4(&mut self) -> Result<f32, ImportError> {
        Ok(f32::from_bits(self.u4()?))
    }

    /// `GetF8`, cut to a float.
    fn f8(&mut self) -> Result<f32, ImportError> {
        Ok(f64::from_bits(self.u8_value()?) as f32)
    }

    /// `ReadVSizedIntLWO2`: two bytes, or 0xFF and three more.
    fn variable_int(&mut self) -> Result<u32, ImportError> {
        let first = self.u1()? as u32;
        if first != 0xFF {
            let second = self.u1()? as u32;
            return Ok((first << 8) | second);
        }
        let high = self.u1()? as u32;
        let middle = self.u1()? as u32;
        let low = self.u1()? as u32;
        Ok((high << 16) | (middle << 8) | low)
    }

    /// `GetS0`: bytes up to a NUL (or `max` of them), then past the NUL
    /// and the padding to an even length.
    fn string(&mut self, max: u32) -> Result<Vec<u8>, ImportError> {
        let start = self.cursor;
        let mut count = 0u64;
        while self.byte(self.cursor)? != 0 {
            count += 1;
            if count > max as u64 {
                break;
            }
            self.cursor += 1;
        }
        let length = (self.cursor - start) as usize;
        let text = self.bytes(start, length)?.to_vec();
        self.cursor += if length & 1 == 1 { 1 } else { 2 };
        Ok(text)
    }

    /// `IFF::LoadChunk`: type and 32-bit length.
    fn chunk(&mut self) -> Result<(u32, u32), ImportError> {
        let kind = self.u4()?;
        let length = self.u4()?;
        Ok((kind, length))
    }

    /// `IFF::LoadSubChunk`: type and 16-bit length.
    fn sub_chunk(&mut self) -> Result<(u32, u16), ImportError> {
        let kind = self.u4()?;
        let length = self.u2()?;
        Ok((kind, length))
    }

    /// A chunk header, or a FORM's (the cursor back on it: its length, its
    /// type, 4 more bytes to skip after the end is taken).
    fn chunk_or_form(&mut self) -> Result<(u32, u32, i64), ImportError> {
        let (kind, length) = self.chunk()?;
        if kind != FORM {
            return Ok((kind, length, 0));
        }
        self.cursor -= 8;
        self.cursor += 4;
        let length = u32::from_be_bytes(self.array(self.cursor)?);
        self.cursor += 4;
        let kind = u32::from_be_bytes(self.array(self.cursor)?);
        Ok((kind, length, 4))
    }

    /// `ByteSwap::Swap2` of the 16-bit word at `position`.
    fn swap2(&mut self, position: i64) -> Result<(), ImportError> {
        self.bytes(position, 2)?;
        let start = position as usize;
        self.data.swap(start, start + 1);
        Ok(())
    }
}

/// `LWO::Face`: corner count and indices (null when a cut LWOB record left
/// the count alone), surface tag, smoothing group and type.
#[derive(Clone)]
pub(crate) struct Face {
    count: u32,
    indices: Option<Vec<u32>>,
    surface: u32,
    smooth_group: u32,
    kind: u32,
}

impl Face {
    fn new(kind: u32) -> Face {
        Face { count: 0, indices: None, surface: 0, smooth_group: 0, kind }
    }
}

/// `VMapEntry`: per-point values of a vertex map, and which are assigned.
#[derive(Clone, Default)]
pub(crate) struct VertexMap {
    name: Vec<u8>,
    dims: u32,
    raw: Vec<f32>,
    assigned: Vec<bool>,
    /// A colour map (`VColorChannel`): alphas start at 1.
    color: bool,
}

impl VertexMap {
    fn new(dims: u32, color: bool) -> VertexMap {
        VertexMap { dims, color, ..VertexMap::default() }
    }

    /// `Allocate` (once, while its data is empty).
    fn allocate(&mut self, count: u32) -> Result<(), ImportError> {
        if !self.raw.is_empty() {
            return Ok(());
        }
        let values = count.wrapping_mul(self.dims);
        check_allocation(values as u64, 4)?;
        self.raw = vec![0.0; values as usize];
        if self.color {
            for chunk in self.raw.as_chunks_mut::<4>().0 {
                chunk[3] = 1.0;
            }
        }
        self.assigned.resize(count as usize, false);
        Ok(())
    }

    /// `CreateNewEntry`: a copy of point `source`'s values for a new point
    /// (and `source` marked assigned).
    fn copy_entry(&mut self, source: u32) -> Result<(), ImportError> {
        if self.name.is_empty() {
            return Ok(());
        }
        let Some(flag) = self.assigned.get_mut(source as usize) else {
            return undefined("a vertex map entry copied from past the map");
        };
        *flag = true;
        self.assigned.push(false);
        for component in 0..self.dims as usize {
            let Some(&value) = self.raw.get(source as usize * self.dims as usize + component) else {
                return undefined("a vertex map entry copied from past the map");
            };
            self.raw.push(value);
        }
        Ok(())
    }
}

/// `LWO::Layer`.
pub(crate) struct Layer {
    points: Vec<Vector3>,
    referrers: Vec<u32>,
    weights: Vec<VertexMap>,
    sub_weights: Vec<VertexMap>,
    colors: Vec<VertexMap>,
    uvs: Vec<VertexMap>,
    normals: VertexMap,
    faces: Vec<Face>,
    face_offset: u32,
    point_offset: u32,
    parent: u16,
    index: u16,
    name: Vec<u8>,
}

impl Layer {
    fn new() -> Layer {
        Layer {
            points: Vec::new(),
            referrers: Vec::new(),
            weights: Vec::new(),
            sub_weights: Vec::new(),
            colors: Vec::new(),
            uvs: Vec::new(),
            normals: VertexMap::new(3, false),
            faces: Vec::new(),
            face_offset: 0,
            point_offset: 0,
            parent: 0,
            index: 0xffff,
            name: Vec::new(),
        }
    }
}

/// A surface texture as far as UV map choice goes.
#[derive(Clone)]
pub(crate) struct Texture {
    enabled: bool,
    usable: bool,
    mapping: u32,
    uv_map: Vec<u8>,
}

impl Default for Texture {
    fn default() -> Self {
        Texture { enabled: true, usable: true, mapping: MAPPING_UV, uv_map: b"unknown".to_vec() }
    }
}

/// `LWO::Surface` as far as meshes depend on it (its seven texture lists:
/// colour, diffuse, specular, glossiness, opacity, bump, reflection).
#[derive(Clone)]
pub(crate) struct Surface {
    name: Vec<u8>,
    smooth_angle: f32,
    transparency: f32,
    color_map: Vec<u8>,
    color_map_type: u32,
    textures: [Vec<Texture>; 7],
}

impl Default for Surface {
    fn default() -> Self {
        Surface {
            name: Vec::new(),
            smooth_angle: 0.0,
            transparency: 0.0,
            color_map: Vec::new(),
            color_map_type: RGBA,
            textures: Default::default(),
        }
    }
}

/// What the chunks leave behind.
pub(crate) struct Model {
    layers: Vec<Layer>,
    tags: Vec<Vec<u8>>,
    surfaces: Vec<Surface>,
}

/// `ASSIMP_stricmp(a, b) == 0`.
fn same_ignoring_case(a: &[u8], b: &[u8]) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// `FindUVChannels` over one texture list: whether a usable UV-mapped
/// texture names the map.
fn names_map(textures: &[Texture], name: &[u8]) -> bool {
    textures.iter().any(|texture| texture.enabled && texture.usable && texture.mapping == MAPPING_UV && texture.uv_map == name)
}

/// Whether point `index` of a map is assigned and not zero (two components
/// for UVs, a colour other than opaque black).
fn map_value_set(map: &VertexMap, index: u32, zero: &[f32]) -> Result<bool, ImportError> {
    let Some(&assigned) = map.assigned.get(index as usize) else {
        return undefined("a vertex map read past its points");
    };
    if !assigned {
        return Ok(false);
    }
    let start = index as usize * zero.len();
    let Some(values) = map.raw.get(start..start + zero.len()) else {
        return undefined("a vertex map read past its points");
    };
    Ok(values.iter().zip(zero).any(|(value, zero)| value != zero))
}

/// The corners of the faces in `sorted`.
fn corners<'a>(layer: &'a Layer, sorted: &'a [u32]) -> impl Iterator<Item = Result<u32, ImportError>> + 'a {
    sorted.iter().flat_map(move |&face| {
        let face = &layer.faces[face as usize];
        let indices: Vec<Result<u32, ImportError>> = match &face.indices {
            Some(indices) => indices.iter().map(|&index| Ok(index)).collect(),
            None if face.count == 0 => Vec::new(),
            None => vec![undefined("the corners of a face without indices")],
        };
        indices
    })
}

/// `FindUVChannels`: the UV maps a mesh uses, those a texture of the
/// surface names first (each such insertion copies the slot it lands on
/// over the rest), the others after, eight at most.
fn find_uv_channels(surface: &Surface, sorted: &[u32], layer: &Layer) -> Result<Vec<u32>, ImportError> {
    let mut out = [u32::MAX; MAX_TEXTURE_COORDS];
    let mut next = 0usize;
    let mut extra = 0usize;
    let mut unused = 0usize;
    for (channel, map) in layer.uvs.iter().enumerate() {
        let mut found = false;
        for corner in corners(layer, sorted) {
            if map_value_set(map, corner?, &[0.0, 0.0])? {
                found = true;
                break;
            }
        }
        if !found || extra >= MAX_TEXTURE_COORDS {
            continue;
        }
        let named = surface.textures.iter().any(|list| names_map(list, &map.name));
        if named {
            if unused != 0 {
                let limit = extra.min(MAX_TEXTURE_COORDS - 1);
                for slot in next..limit {
                    out[slot + 1] = out[slot];
                }
            }
            extra += 1;
            out[next] = channel as u32;
            next += 1;
        } else {
            out[extra] = channel as u32;
            extra += 1;
            unused += 1;
        }
    }
    let used = out.iter().position(|&channel| channel == u32::MAX).unwrap_or(MAX_TEXTURE_COORDS);
    if extra < MAX_TEXTURE_COORDS {
        return Ok(out[..extra].to_vec());
    }
    Ok(out[..used].to_vec())
}

/// `FindVCChannels`: the surface's own colour map first (copying the first
/// slot over the others), then maps with a colour on these faces.
fn find_color_channels(surface: &Surface, sorted: &[u32], layer: &Layer) -> Result<Vec<u32>, ImportError> {
    let mut out = [u32::MAX; MAX_COLOR_SETS];
    let mut next = 0usize;
    for (channel, map) in layer.colors.iter().enumerate() {
        if surface.color_map == map.name {
            let limit = next.min(MAX_COLOR_SETS - 1);
            for slot in 0..limit {
                out[slot + 1] = out[slot];
            }
            out[0] = channel as u32;
            next += 1;
            continue;
        }
        for corner in corners(layer, sorted) {
            if map_value_set(map, corner?, &[0.0, 0.0, 0.0, 1.0])? {
                if next < MAX_COLOR_SETS {
                    out[next] = channel as u32;
                    next += 1;
                }
                break;
            }
        }
    }
    if next > MAX_COLOR_SETS {
        return undefined("a colour channel index written past its array");
    }
    Ok(out[..next].to_vec())
}

/// `ComputePositionEpsilon` of one mesh.
fn position_epsilon(positions: &[Vector3]) -> f32 {
    let mut low = [1e10f32; 3];
    let mut high = [-1e10f32; 3];
    for position in positions {
        for axis in 0..3 {
            low[axis] = if low[axis] < position[axis] { low[axis] } else { position[axis] };
            high[axis] = if position[axis] < high[axis] { high[axis] } else { position[axis] };
        }
    }
    let extent = sub(high, low);
    (extent[0] * extent[0] + extent[1] * extent[1] + extent[2] * extent[2]).sqrt() * 1e-4
}

/// `ComputeNormals` (see the module notes).
fn compute_normals(positions: &[Vector3], faces: &[Vec<u32>], smoothing_groups: &[u32], smooth_angle: f32) -> Vec<Vector3> {
    let count = positions.len();
    let mut normals = vec![[0.0f32; 3]; count];
    let mut face_normals = vec![[0.0f32; 3]; count];
    {
        let out = if smooth_angle == 0.0 { &mut normals } else { &mut face_normals };
        for face in faces {
            if face.len() < 3 {
                continue;
            }
            let first = positions[face[0] as usize];
            let second = positions[face[1] as usize];
            let last = positions[face[face.len() - 1] as usize];
            let normal = normalize(cross(sub(second, first), sub(last, first)));
            for &index in face {
                out[index as usize] = normal;
            }
        }
    }
    if smooth_angle == 0.0 {
        return normals;
    }
    let epsilon = position_epsilon(positions);
    let mut sort = SgSpatialSort::new();
    for (face, &group) in faces.iter().zip(smoothing_groups) {
        for &index in face {
            sort.add(positions[index as usize], index, group);
        }
    }
    sort.prepare();
    // (Below 3 radians nothing is stored; a NaN angle smooths.)
    if smooth_angle.partial_cmp(&3.0) != Some(std::cmp::Ordering::Less) {
        let mut done = vec![false; count];
        let mut results = Vec::new();
        for (face, &group) in faces.iter().zip(smoothing_groups) {
            for &index in face {
                if done[index as usize] {
                    continue;
                }
                sort.find_positions(positions[index as usize], group, epsilon, true, &mut results);
                let mut sum = [0.0f32; 3];
                for &found in &results {
                    let normal = face_normals[found as usize];
                    sum = [sum[0] + normal[0], sum[1] + normal[1], sum[2] + normal[2]];
                }
                for &found in &results {
                    normals[found as usize] = sum;
                    done[found as usize] = true;
                }
            }
        }
    }
    normals.iter().map(|&normal| normalize(normal)).collect()
}

/// One surface's mesh of a layer: a vertex per face corner, the UV and
/// colour maps chosen, Modo normals or computed ones. The faces' indices
/// are renumbered in place.
fn build_mesh(layer: &mut Layer, sorted: &[u32], surface: &Surface, surface_index: u32) -> Result<Mesh, ImportError> {
    let mut vertex_count = 0u32;
    for &face in sorted {
        vertex_count = vertex_count.wrapping_add(layer.faces[face as usize].count);
    }
    check_allocation(vertex_count as u64, VECTOR_SIZE)?;
    let uv_channels = find_uv_channels(surface, sorted, layer)?;
    let color_channels = find_color_channels(surface, sorted, layer)?;
    let has_normals = !layer.normals.name.is_empty();

    let capacity = vertex_count as usize;
    let mut positions = Vec::with_capacity(capacity);
    let mut normals = Vec::with_capacity(if has_normals { capacity } else { 0 });
    let mut uvs: Vec<Vec<Vector3>> = uv_channels.iter().map(|_| Vec::with_capacity(capacity)).collect();
    let mut colors: Vec<Vec<[f32; 4]>> = color_channels.iter().map(|_| Vec::with_capacity(capacity)).collect();
    let mut faces = Vec::with_capacity(sorted.len());
    let mut smoothing_groups = Vec::with_capacity(sorted.len());
    let mut vertex = 0u32;
    for &face_index in sorted {
        let face = &mut layer.faces[face_index as usize];
        smoothing_groups.push(face.smooth_group);
        let count = face.count;
        let Some(indices) = face.indices.take() else {
            if count != 0 {
                return undefined("the corners of a face without indices");
            }
            faces.push(Vec::new());
            continue;
        };
        let mut renumbered = Vec::with_capacity(indices.len());
        for &index in &indices {
            let Some(&point) = layer.points.get(index as usize) else {
                return undefined("a face corner past the points");
            };
            positions.push(point);
            for (target, &channel) in uvs.iter_mut().zip(&uv_channels) {
                let map = &layer.uvs[channel as usize];
                let start = index as usize * 2;
                let Some(values) = map.raw.get(start..start + 2) else {
                    return undefined("a UV map read past its points");
                };
                target.push([values[0], values[1], 0.0]);
            }
            if has_normals {
                let start = index as usize * 3;
                let Some(values) = layer.normals.raw.get(start..start + 3) else {
                    return undefined("a normal map read past its points");
                };
                normals.push([values[0], values[1], -values[2]]);
            }
            for (target, &channel) in colors.iter_mut().zip(&color_channels) {
                let map = &layer.colors[channel as usize];
                let start = index as usize * 4;
                let Some(values) = map.raw.get(start..start + 4) else {
                    return undefined("a colour map read past its points");
                };
                let mut color = [values[0], values[1], values[2], values[3]];
                if surface.color_map_type == RGB {
                    color[3] = 1.0;
                }
                target.push(color);
            }
            renumbered.push(vertex);
            vertex += 1;
        }
        faces.push(renumbered);
    }

    let mut mesh = Mesh { material_index: surface_index, vertex_count: positions.len(), ..Mesh::default() };
    if !has_normals {
        normals = compute_normals(&positions, &faces, &smoothing_groups, surface.smooth_angle);
    }
    for (set, coordinates) in uvs.into_iter().enumerate() {
        mesh.uv_components[set] = 2;
        mesh.texture_coords[set] = Some(coordinates);
    }
    for (set, values) in colors.into_iter().enumerate() {
        mesh.colors[set] = Some(values);
    }
    mesh.vertices = Some(positions);
    mesh.normals = Some(normals);
    mesh.faces = Some(faces);
    Ok(mesh)
}

/// `GenerateNodeGraph`, as far as it can fail: each layer node goes under
/// a pivot node (keyed `-(index + 2)`, dropped when that key is a layer's),
/// the pivot under its layer's parent layer or the root, which must get a
/// child.
fn node_graph_has_root_children(nodes: &std::collections::BTreeMap<u16, u16>) -> bool {
    nodes.iter().any(|(&index, &parent)| {
        let pivot = ((index as i32 + 2).wrapping_neg()) as u16;
        !nodes.contains_key(&pivot) && !nodes.contains_key(&parent)
    })
}

/// `InternReadFile` after the chunks: meshes per layer and surface, then
/// the node graph check and the conversion to left-handed coordinates with
/// the winding flipped.
fn build_scene(model: &mut Model) -> Result<Scene, ImportError> {
    // `ResolveTags`: each tag's first surface of that name (any case).
    let mapping: Vec<u32> = model
        .tags
        .iter()
        .map(|tag| {
            let found = model.surfaces.iter().position(|surface| same_ignoring_case(tag, &surface.name));
            found.map_or(u32::MAX, |index| index as u32)
        })
        .collect();

    let mut meshes = Vec::new();
    let mut nodes = std::collections::BTreeMap::new();
    let mut default_surface = u32::MAX;
    for layer_index in 0..model.layers.len() {
        let mesh_start = meshes.len();
        let layer = &mut model.layers[layer_index];
        if !layer.faces.is_empty() && !layer.points.is_empty() {
            let mut sorted: Vec<Vec<u32>> = vec![Vec::new(); model.surfaces.len() + 1];
            for (index, face) in layer.faces.iter().enumerate() {
                if !matches!(face.kind, FACE | PTCH | BONE | SUBD) {
                    continue;
                }
                let mut surface = face.surface;
                if surface as usize >= model.tags.len() {
                    surface = u32::MAX;
                }
                if surface != u32::MAX {
                    surface = mapping[surface as usize];
                }
                if surface == u32::MAX {
                    if default_surface == u32::MAX {
                        default_surface = model.surfaces.len() as u32;
                        let fallback = Surface { name: b"LWODefaultSurface".to_vec(), ..Surface::default() };
                        model.surfaces.push(fallback);
                    }
                    surface = default_surface;
                }
                sorted[surface as usize].push(index as u32);
            }
            if default_surface == u32::MAX {
                sorted.pop();
            }
            for surface_index in 0..model.surfaces.len() {
                let Some(faces) = sorted.get(surface_index) else {
                    break;
                };
                if faces.is_empty() {
                    continue;
                }
                let surface = model.surfaces[surface_index].clone();
                let mesh = build_mesh(layer, faces, &surface, surface_index as u32)?;
                meshes.push(mesh);
            }
        }
        let layer = &model.layers[layer_index];
        if layer.name != b"<LWODefault>" || meshes.len() > mesh_start {
            nodes.insert(layer.index, layer.parent);
        }
    }
    if nodes.is_empty() || meshes.is_empty() {
        return fail("LWO: No meshes loaded");
    }
    if !node_graph_has_root_children(&nodes) {
        return fail("LWO: Unable to build a valid node graph");
    }

    // `MakeLeftHandedProcess` and `FlipWindingOrderProcess`.
    for mesh in &mut meshes {
        let count = mesh.vertex_count;
        for position in mesh.vertices.iter_mut().flatten().take(count) {
            position[2] = -position[2];
        }
        for normal in mesh.normals.iter_mut().flatten().take(count) {
            normal[2] = -normal[2];
        }
        for face in mesh.faces.iter_mut().flatten() {
            face.reverse();
        }
    }
    Ok(Scene { meshes })
}

/// `LWOImporter::InternReadFile`.
pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    if bytes.len() < 12 {
        return fail("LWO: The file is too small to contain the IFF header");
    }
    let mut model = parse::read_chunks(bytes)?;
    build_scene(&mut model)
}
