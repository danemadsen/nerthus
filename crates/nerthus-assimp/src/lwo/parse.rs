//! The chunks of LWO files: LWO2 and later (`LoadLWO2File`, LWO3 nesting
//! FORMs) and LWOB (`LoadLWOBFile`), as far as meshes or failures depend
//! on them.

use super::{Face, Layer, Model, Reader, Surface, Texture, VertexMap, fail, fourcc, undefined, validate_length, ImportError, FORM, MAX_CHAIN, RGB, RGBA};
use crate::scene::check_allocation;

const LWOB: u32 = fourcc(b"LWOB");
const LWO2: u32 = fourcc(b"LWO2");
const LWO3: u32 = fourcc(b"LWO3");
const LXOB: u32 = fourcc(b"LXOB");

const LAYR: u32 = fourcc(b"LAYR");
const PNTS: u32 = fourcc(b"PNTS");
const VMAP: u32 = fourcc(b"VMAP");
const VMAD: u32 = fourcc(b"VMAD");
const POLS: u32 = fourcc(b"POLS");
const PTAG: u32 = fourcc(b"PTAG");
const TAGS: u32 = fourcc(b"TAGS");
const SRFS: u32 = fourcc(b"SRFS");
const SURF: u32 = fourcc(b"SURF");
const CLIP: u32 = fourcc(b"CLIP");
const ENVL: u32 = fourcc(b"ENVL");
const SMGP: u32 = fourcc(b"SMGP");

const TXUV: u32 = fourcc(b"TXUV");
const WGHT: u32 = fourcc(b"WGHT");
const MNVW: u32 = fourcc(b"MNVW");
const NORM: u32 = fourcc(b"NORM");

const COLR: u32 = fourcc(b"COLR");
const DIFF: u32 = fourcc(b"DIFF");
const SPEC: u32 = fourcc(b"SPEC");
const TRAN: u32 = fourcc(b"TRAN");
const ADTR: u32 = fourcc(b"ADTR");
const LINE: u32 = fourcc(b"LINE");
const GLOS: u32 = fourcc(b"GLOS");
const BUMP: u32 = fourcc(b"BUMP");
const CLRH: u32 = fourcc(b"CLRH");
const RIND: u32 = fourcc(b"RIND");
const SIDE: u32 = fourcc(b"SIDE");
const SMAN: u32 = fourcc(b"SMAN");
const VCOL: u32 = fourcc(b"VCOL");
const BLOK: u32 = fourcc(b"BLOK");
const LUMI: u32 = fourcc(b"LUMI");
const FLAG: u32 = fourcc(b"FLAG");
const REFL: u32 = fourcc(b"REFL");

const PROC: u32 = fourcc(b"PROC");
const GRAD: u32 = fourcc(b"GRAD");
const IMAP: u32 = fourcc(b"IMAP");
const SHDR: u32 = fourcc(b"SHDR");
const CHAN: u32 = fourcc(b"CHAN");
const ENAB: u32 = fourcc(b"ENAB");
const OPAC: u32 = fourcc(b"OPAC");
const PROJ: u32 = fourcc(b"PROJ");
const WRAP: u32 = fourcc(b"WRAP");
const AXIS: u32 = fourcc(b"AXIS");
const IMAG: u32 = fourcc(b"IMAG");
const WRPH: u32 = fourcc(b"WRPH");
const WRPW: u32 = fourcc(b"WRPW");
const FUNC: u32 = fourcc(b"FUNC");

const STIL: u32 = fourcc(b"STIL");
const ISEQ: u32 = fourcc(b"ISEQ");
const XREF: u32 = fourcc(b"XREF");
const NEGA: u32 = fourcc(b"NEGA");

const TYPE: u32 = fourcc(b"TYPE");
const PRE: u32 = fourcc(b"PRE ");
const POST: u32 = fourcc(b"POST");
const KEY: u32 = fourcc(b"KEY ");
const SPAN: u32 = fourcc(b"SPAN");

const NODS: u32 = fourcc(b"NODS");
const NNDS: u32 = fourcc(b"NNDS");
const NTAG: u32 = fourcc(b"NTAG");
const NDTA: u32 = fourcc(b"NDTA");
const ENTR: u32 = fourcc(b"ENTR");
const NAME: u32 = fourcc(b"NAME");
const TAG: u32 = fourcc(b"TAG ");
const VALU: u32 = fourcc(b"VALU");

const CTEX: u32 = fourcc(b"CTEX");
const DTEX: u32 = fourcc(b"DTEX");
const STEX: u32 = fourcc(b"STEX");
const BTEX: u32 = fourcc(b"BTEX");
const TTEX: u32 = fourcc(b"TTEX");
const TIMG: u32 = fourcc(b"TIMG");
const TVAL: u32 = fourcc(b"TVAL");
const TFLG: u32 = fourcc(b"TFLG");

/// LWO3 node data chunks skipped whole.
const NODE_DATA_SKIPPED: [&[u8; 4]; 35] = [
    b"VERS", b"ENUM", b"IBGC", b"IOPC", b"IIMG", b"TXTR", b"IFAL", b"ISCL", b"IPOS", b"IROT", b"IBMP", b"IUTD", b"IVTD",
    b"IPIX", b"IMIP", b"IMOD", b"AMOD", b"IINV", b"INCR", b"IAXS", b"IFOT", b"ITIM", b"IWRL", b"IUTI", b"IUVI", b"IINX",
    b"IINY", b"IINZ", b"IREF", b"IMST", b"IMAP", b"IUTL", b"IVTL", b"VPVL", b"VPRM",
];

/// `SMAN` set by an LWOB surface flag.
const LWOB_SMOOTHING: f32 = 1.56207;

/// A layer's vertex map lists.
#[derive(Clone, Copy, PartialEq)]
enum MapList {
    Uvs,
    Weights,
    SubWeights,
    Colors,
    Normals,
}

/// The texture list of a channel type (colour, diffuse, specular,
/// glossiness, opacity, bump, reflection).
fn texture_list(kind: u32) -> Option<usize> {
    match kind {
        COLR => Some(0),
        DIFF => Some(1),
        SPEC => Some(2),
        GLOS => Some(3),
        TRAN => Some(4),
        BUMP => Some(5),
        REFL => Some(6),
        _ => None,
    }
}

struct Parser {
    reader: Reader,
    model: Model,
    lwo2: bool,
    lwo3: bool,
    lxob: bool,
    unnamed: u32,
    /// `configLayerIndex`: the one layer to load, counted from 1
    /// (`u32::MAX` loads them all).
    layer_index: u32,
    /// Whether the current layer's geometry chunks are skipped.
    skip: bool,
}

impl Parser {
    fn layer(&mut self) -> &mut Layer {
        self.model.layers.last_mut().expect("the default layer is always there")
    }

    fn map(&mut self, list: MapList, index: usize) -> &mut VertexMap {
        let layer = self.layer();
        match list {
            MapList::Uvs => &mut layer.uvs[index],
            MapList::Weights => &mut layer.weights[index],
            MapList::SubWeights => &mut layer.sub_weights[index],
            MapList::Colors => &mut layer.colors[index],
            MapList::Normals => &mut layer.normals,
        }
    }

    /// `LoadLWOPoints`: written over the start of the point list, which
    /// grows by their count (with as many referrer slots for LWO2).
    fn points(&mut self, length: u32) -> Result<(), ImportError> {
        if !length.is_multiple_of(12) {
            return fail("LWO2: Points chunk length is not multiple of vertexLen (12)");
        }
        let lwo2 = self.lwo2 || self.lwo3;
        let count = length / 12;
        let mut values = Vec::with_capacity(count as usize);
        let start = self.reader.cursor;
        for point in 0..count as i64 {
            let bytes = self.reader.bytes(start + 12 * point, 12)?;
            let x = f32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            let y = f32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
            let z = f32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
            values.push([x, y, z]);
        }
        let layer = self.layer();
        let size = (layer.points.len() as u32).wrapping_add(count);
        check_allocation(size as u64, 12)?;
        layer.points.resize(size as usize, [0.0; 3]);
        if lwo2 {
            layer.referrers.resize(size as usize, u32::MAX);
        }
        layer.points[..values.len()].copy_from_slice(&values);
        Ok(())
    }

    /// `FindEntry`: the map of this name, or a new one.
    fn find_entry(&mut self, list: MapList, name: &[u8]) -> usize {
        let layer = self.layer();
        let (maps, dims, color) = match list {
            MapList::Uvs => (&mut layer.uvs, 2, false),
            MapList::Weights => (&mut layer.weights, 1, false),
            MapList::SubWeights => (&mut layer.sub_weights, 1, false),
            _ => (&mut layer.colors, 4, true),
        };
        if let Some(found) = maps.iter().position(|map| map.name == name) {
            return found;
        }
        let mut map = VertexMap::new(dims, color);
        map.name = name.to_vec();
        maps.push(map);
        maps.len() - 1
    }

    /// `LoadLWO2VertexMap`: values per point (VMAP) or per point of a
    /// polygon (VMAD, which copies a point already assigned in that map
    /// into a new one for the polygon).
    fn vertex_map(&mut self, length: u32, per_polygon: bool) -> Result<(), ImportError> {
        let end = self.reader.cursor + length as i64;
        validate_length(length, "VMAP", 6)?;
        let kind = self.reader.u4()?;
        let dims = self.reader.u2()? as u32;
        let name = self.reader.string(length)?;
        let (list, index) = match kind {
            TXUV if dims == 2 => (MapList::Uvs, self.find_entry(MapList::Uvs, &name)),
            WGHT if dims == 1 => (MapList::Weights, self.find_entry(MapList::Weights, &name)),
            MNVW if dims == 1 => (MapList::SubWeights, self.find_entry(MapList::SubWeights, &name)),
            RGB | RGBA if dims == 3 || dims == 4 => (MapList::Colors, self.find_entry(MapList::Colors, &name)),
            NORM => {
                let layer = self.layer();
                if name != b"vert_normals" || dims != 3 || !layer.normals.name.is_empty() {
                    return Ok(());
                }
                layer.normals.name = name.clone();
                (MapList::Normals, 0)
            }
            _ => return Ok(()),
        };
        let point_count = self.layer().points.len() as u32;
        let face_count = self.layer().faces.len() as u32;
        self.map(list, index).allocate(point_count)?;
        let map_dims = self.map(list, index).dims;
        let read_count = dims.min(map_dims);
        let skipped = (dims - read_count) << 2;

        while self.reader.cursor < end {
            let point_offset = self.layer().point_offset;
            let mut point = self.reader.variable_int()?.wrapping_add(point_offset);
            if point >= point_count {
                self.reader.cursor += (map_dims << 2) as i64;
                continue;
            }
            if per_polygon {
                let face_offset = self.layer().face_offset;
                let polygon = self.reader.variable_int()?.wrapping_add(face_offset);
                let Some(&assigned) = self.map(list, index).assigned.get(point as usize) else {
                    return undefined("a vertex map flag read past its points");
                };
                if assigned {
                    if polygon >= face_count {
                        self.reader.cursor += (map_dims << 2) as i64;
                        continue;
                    }
                    if let Some(copy) = self.copy_polygon_points(polygon, point)? {
                        point = copy;
                    }
                }
            }
            let mut values = Vec::with_capacity(read_count as usize);
            for _ in 0..read_count {
                values.push(self.reader.f4()?);
            }
            self.assign(list, index, point, &values)?;
            self.reader.cursor += skipped as i64;
        }
        Ok(())
    }

    /// The VMAD copy: every corner of the polygon that is the point (or a
    /// copy of it) becomes a new copy of its own point; the last new point.
    fn copy_polygon_points(&mut self, polygon: u32, point: u32) -> Result<Option<u32>, ImportError> {
        let layer = self.layer();
        let face = &layer.faces[polygon as usize];
        let count = face.count as usize;
        let Some(mut indices) = face.indices.clone() else {
            if count == 0 {
                return Ok(None);
            }
            return undefined("the corners of a face without indices");
        };
        let mut target = point;
        let mut copied = None;
        for slot in indices.iter_mut().take(count) {
            let source = *slot;
            // Whether the corner is the point or one of its copies.
            let mut walker = target;
            let mut steps = 0usize;
            let found = loop {
                if walker == source {
                    break true;
                }
                let Some(&next) = layer.referrers.get(walker as usize) else {
                    return undefined("a point referrer read past the points");
                };
                walker = next;
                if walker == u32::MAX {
                    break false;
                }
                steps += 1;
                if steps > MAX_CHAIN {
                    return undefined("a chain of point copies too long for the stack");
                }
            };
            if !found {
                continue;
            }
            layer.referrers.push(u32::MAX);
            let new_point = layer.points.len() as u32;
            target = new_point;
            *slot = new_point;
            link(&mut layer.referrers, source, new_point)?;
            let Some(&position) = layer.points.get(source as usize) else {
                return undefined("a point copied from past the points");
            };
            layer.points.push(position);
            for map in layer.colors.iter_mut().chain(layer.uvs.iter_mut()).chain(layer.weights.iter_mut()).chain(layer.sub_weights.iter_mut()) {
                map.copy_entry(source)?;
            }
            layer.normals.copy_entry(source)?;
            copied = Some(new_point);
        }
        layer.faces[polygon as usize].indices = Some(indices);
        Ok(copied)
    }

    /// `DoRecursiveVMAPAssignment`: the values for the point and its
    /// copies.
    fn assign(&mut self, list: MapList, index: usize, point: u32, values: &[f32]) -> Result<(), ImportError> {
        let mut point = point;
        let mut depth = 0usize;
        loop {
            let map = self.map(list, index);
            if point as usize >= map.assigned.len() {
                return fail("Bad index");
            }
            map.assigned[point as usize] = true;
            let dims = map.dims as usize;
            for (component, &value) in values.iter().enumerate() {
                let Some(slot) = map.raw.get_mut(point as usize * dims + component) else {
                    return undefined("a vertex map value written past the map");
                };
                *slot = value;
            }
            let Some(&next) = self.layer().referrers.get(point as usize) else {
                return undefined("a point referrer read past the points");
            };
            if next == u32::MAX {
                return Ok(());
            }
            point = next;
            depth += 1;
            if depth > MAX_CHAIN {
                return undefined("a chain of point copies too long for the stack");
            }
        }
    }

    /// `LoadLWO2Polygons`: counted, then (re)written from the layer's first
    /// face on, the list cut or grown to the count.
    fn polygons(&mut self, length: u32) -> Result<(), ImportError> {
        let end = self.reader.cursor + length as i64;
        let kind = self.reader.u4()?;
        let start = self.reader.cursor;
        let mut faces = 0u32;
        while self.reader.cursor < end {
            let count = self.reader.u2()? & 0x03FF;
            faces = faces.wrapping_add(1);
            for _ in 0..count {
                self.reader.variable_int()?;
            }
        }
        if faces == 0 {
            return Ok(());
        }
        self.reader.cursor = start;
        check_allocation(faces as u64, 40)?;
        self.layer().faces.resize(faces as usize, Face::new(kind));
        let mut face = 0usize;
        while self.reader.cursor < end {
            let count = self.reader.u2()? & 0x03FF;
            if count == 0 {
                return fail("LWO2: Encountered invalid face record with zero indices");
            }
            let mut indices = Vec::with_capacity(count as usize);
            for _ in 0..count {
                let point_offset = self.layer().point_offset;
                let mut index = self.reader.variable_int()?.wrapping_add(point_offset);
                let point_count = self.layer().points.len() as u32;
                if index > point_count {
                    index = point_count.wrapping_sub(1);
                }
                indices.push(index);
            }
            let target = &mut self.layer().faces[face];
            target.count = count as u32;
            target.indices = Some(indices);
            face += 1;
        }
        Ok(())
    }

    /// `LoadLWO2PolygonTags`: surface tags or smoothing groups of faces.
    fn polygon_tags(&mut self, length: u32) -> Result<(), ImportError> {
        let end = self.reader.cursor + length as i64;
        validate_length(length, "PTAG", 4)?;
        let kind = self.reader.u4()?;
        if kind != SURF && kind != SMGP {
            return Ok(());
        }
        while self.reader.cursor < end {
            let face_offset = self.layer().face_offset;
            let face = self.reader.variable_int()?.wrapping_add(face_offset);
            let value = self.reader.u2()? as u32;
            let layer = self.layer();
            let Some(target) = layer.faces.get_mut(face as usize) else {
                continue;
            };
            if kind == SURF {
                target.surface = value;
            } else {
                target.smooth_group = value;
            }
        }
        Ok(())
    }

    /// `LoadLWOTags`: NUL-ended names padded to even lengths (the byte after
    /// a NUL is never taken for one).
    fn tags(&mut self, length: u32) -> Result<(), ImportError> {
        let mut current = self.reader.cursor;
        let mut last = current;
        let end = current + length as i64;
        while current < end {
            if self.reader.byte(current)? == 0 {
                let size = current - last;
                if size > 0 {
                    let tag = self.reader.bytes(last, size as usize)?.to_vec();
                    self.model.tags.push(tag);
                }
                current += if size & 1 == 1 { 1 } else { 2 };
                last = current;
            }
            current += 1;
        }
        Ok(())
    }

    /// `LoadLWO2Surface`: a name, maybe the name of a surface it copies,
    /// then sub-chunks.
    fn surface2(&mut self, size: u32) -> Result<(), ImportError> {
        let end = self.reader.cursor + size as i64;
        self.model.surfaces.push(Surface::default());
        let name = self.reader.string(size)?;
        self.surface_mut().name = name;
        self.copy_derived(end)?;
        loop {
            if self.reader.cursor + 6 >= end {
                break;
            }
            let (kind, length) = self.reader.sub_chunk()?;
            let length = length as u32;
            if self.reader.cursor + length as i64 > end {
                return fail("LWO2: Invalid surface chunk length");
            }
            let next = self.reader.cursor + length as i64;
            match kind {
                COLR => {
                    validate_length(length, "COLR", 12)?;
                    self.floats(3)?;
                }
                DIFF | SPEC | ADTR | GLOS | BUMP | CLRH | RIND => {
                    let name = String::from_utf8_lossy(&kind.to_be_bytes()).to_string();
                    validate_length(length, &name, 4)?;
                    self.reader.f4()?;
                }
                TRAN => {
                    // (Skipped once the transparency is exactly 1e11.)
                    if self.surface_mut().transparency != 10e10 {
                        validate_length(length, "TRAN", 4)?;
                        let value = self.reader.f4()?;
                        self.surface_mut().transparency = value;
                    }
                }
                LINE | SIDE => {
                    let name = String::from_utf8_lossy(&kind.to_be_bytes()).to_string();
                    validate_length(length, &name, 2)?;
                    self.reader.u2()?;
                }
                SMAN => {
                    validate_length(length, "SMAN", 4)?;
                    let angle = self.reader.f4()?.abs();
                    self.surface_mut().smooth_angle = angle;
                }
                VCOL => {
                    validate_length(length, "VCOL", 12)?;
                    self.reader.f4()?;
                    self.reader.variable_int()?;
                    let map_type = self.reader.u4()?;
                    let remaining = (next - self.reader.cursor) as u32;
                    let map = self.reader.string(remaining)?;
                    let surface = self.surface_mut();
                    surface.color_map_type = map_type;
                    surface.color_map = map;
                }
                BLOK => {
                    validate_length(length, "BLOK", 4)?;
                    let (block_kind, block_length) = self.reader.sub_chunk()?;
                    match block_kind {
                        PROC | GRAD | IMAP => self.texture_block(block_kind, block_length as u32, length)?,
                        SHDR => self.shader_block(length)?,
                        _ => {}
                    }
                }
                _ => {}
            }
            self.reader.cursor = next;
        }
        Ok(())
    }

    fn surface_mut(&mut self) -> &mut Surface {
        self.model.surfaces.last_mut().expect("a surface was just pushed")
    }

    fn floats(&mut self, count: usize) -> Result<(), ImportError> {
        for _ in 0..count {
            self.reader.f4()?;
        }
        Ok(())
    }

    /// The derived surface's name: a copy of the first earlier surface of
    /// that name (its name included).
    fn copy_derived(&mut self, end: i64) -> Result<(), ImportError> {
        let remaining = (end - self.reader.cursor) as u32;
        let derived = self.reader.string(remaining)?;
        if derived.is_empty() {
            return Ok(());
        }
        let last = self.model.surfaces.len() - 1;
        if let Some(source) = self.model.surfaces[..last].iter().position(|surface| surface.name == derived) {
            self.model.surfaces[last] = self.model.surfaces[source].clone();
        }
        Ok(())
    }

    /// `LoadLWO2TextureBlock`: the header, then the image map (procedural
    /// and gradient textures are unusable), kept in the list of its
    /// channel.
    fn texture_block(&mut self, kind: u32, header_length: u32, size: u32) -> Result<(), ImportError> {
        let mut texture = Texture::default();
        let channel = self.texture_header(header_length, &mut texture)?;
        let size = size.wrapping_sub(header_length + 6);
        match kind {
            PROC | GRAD => texture.usable = false,
            _ => self.image_map(size, &mut texture)?,
        }
        if let Some(list) = texture_list(channel) {
            self.surface_mut().textures[list].push(texture);
        }
        Ok(())
    }

    /// `LoadLWO2TextureHeader`: the ordinal, then the channel, whether it
    /// is enabled and its opacity. Returns the channel.
    fn texture_header(&mut self, size: u32, texture: &mut Texture) -> Result<u32, ImportError> {
        let end = self.reader.cursor + size as i64;
        self.reader.string(size)?;
        let mut channel = 0u32;
        loop {
            if self.reader.cursor + 6 >= end {
                break;
            }
            let (kind, length) = self.reader.sub_chunk()?;
            if self.reader.cursor + length as i64 > end {
                return fail("LWO2: Invalid texture header chunk length");
            }
            let next = self.reader.cursor + length as i64;
            match kind {
                CHAN => channel = self.reader.u4()?,
                ENAB => texture.enabled = self.reader.u2()? != 0,
                OPAC => {
                    self.reader.u2()?;
                    self.reader.f4()?;
                }
                _ => {}
            }
            self.reader.cursor = next;
        }
        Ok(channel)
    }

    /// `LoadLWO2ImageMap`: projection, wrapping, axis, image and UV map.
    fn image_map(&mut self, size: u32, texture: &mut Texture) -> Result<(), ImportError> {
        let end = self.reader.cursor + size as i64;
        loop {
            if self.reader.cursor + 6 >= end {
                break;
            }
            let (kind, length) = self.reader.sub_chunk()?;
            if self.reader.cursor + length as i64 > end {
                return fail("LWO2: Invalid SURF.BLOCK chunk length");
            }
            let next = self.reader.cursor + length as i64;
            match kind {
                PROJ => texture.mapping = self.reader.u2()? as u32,
                WRAP => {
                    self.reader.u2()?;
                    self.reader.u2()?;
                }
                AXIS | IMAG => {
                    self.reader.u2()?;
                }
                VMAP => texture.uv_map = self.reader.string(length as u32)?,
                WRPH | WRPW => {
                    self.reader.f4()?;
                }
                _ => {}
            }
            self.reader.cursor = next;
        }
        Ok(())
    }

    /// `LoadLWO2ShaderBlock` (shaders make nothing a mesh shows).
    fn shader_block(&mut self, size: u32) -> Result<(), ImportError> {
        let end = self.reader.cursor + size as i64;
        self.reader.string(size)?;
        loop {
            if self.reader.cursor + 6 >= end {
                break;
            }
            let (kind, length) = self.reader.sub_chunk()?;
            if self.reader.cursor + length as i64 > end {
                return fail("LWO2: Invalid shader header chunk length");
            }
            let next = self.reader.cursor + length as i64;
            match kind {
                ENAB => {
                    self.reader.u2()?;
                }
                FUNC => {
                    self.reader.string(length as u32)?;
                }
                _ => {}
            }
            self.reader.cursor = next;
        }
        Ok(())
    }

    /// `LoadLWO3Surface`: 8 bytes in, a name, maybe a copied surface, then
    /// chunks (nodes, sidedness, smoothing angle).
    fn surface3(&mut self, size: u32) -> Result<(), ImportError> {
        self.reader.cursor += 8;
        let end = self.reader.cursor + size as i64 - 12;
        self.model.surfaces.push(Surface::default());
        let name = self.reader.string(size)?;
        self.surface_mut().name = name;
        self.copy_derived(end)?;
        loop {
            if self.reader.cursor + 8 >= end {
                break;
            }
            let (kind, length, skip) = self.reader.chunk_or_form()?;
            if self.reader.cursor + length as i64 > end {
                return fail("LWO3: cannot read length; LoadLWO3Surface");
            }
            let next = self.reader.cursor + length as i64;
            self.reader.cursor += skip;
            match kind {
                NODS => self.node_blocks(length, NNDS, "LWO3: cannot read length; LoadNodalBlocks")?,
                SIDE => {
                    validate_length(length, "SIDE", 2)?;
                    self.reader.u2()?;
                }
                SMAN => {
                    validate_length(length, "SMAN", 4)?;
                    let angle = self.reader.f4()?.abs();
                    self.surface_mut().smooth_angle = angle;
                }
                _ => {}
            }
            self.reader.cursor = next;
        }
        Ok(())
    }

    /// `LoadNodalBlocks`, `LoadNodes` and `LoadNodeTag`: chunks (or FORMs)
    /// of which one type goes a level deeper.
    fn node_blocks(&mut self, size: u32, inner: u32, message: &str) -> Result<(), ImportError> {
        let end = self.reader.cursor + size as i64;
        loop {
            if self.reader.cursor + 8 >= end {
                break;
            }
            let (kind, length, skip) = self.reader.chunk_or_form()?;
            if self.reader.cursor + length as i64 > end {
                return fail(message);
            }
            let next = self.reader.cursor + length as i64;
            self.reader.cursor += skip;
            if kind == inner {
                match inner {
                    NNDS => self.node_blocks(length, NTAG, "LWO3: cannot read length; LoadNodes")?,
                    NTAG => self.node_blocks(length, NDTA, "LWO3: cannot read length; LoadNodeTag")?,
                    _ => self.node_data(length)?,
                }
            }
            self.reader.cursor = next;
        }
        Ok(())
    }

    /// `LoadNodeData`: skipped chunks, entries of named values; an unknown
    /// chunk leaves the cursor past its header.
    fn node_data(&mut self, size: u32) -> Result<(), ImportError> {
        let end = self.reader.cursor + size as i64;
        loop {
            if self.reader.cursor + 8 >= end {
                break;
            }
            let (kind, length, skip) = self.reader.chunk_or_form()?;
            if self.reader.cursor + length as i64 > end {
                return fail("LWO3: INVALID LENGTH; LoadNodeData");
            }
            let next = self.reader.cursor + length as i64;
            self.reader.cursor += skip;
            if NODE_DATA_SKIPPED.iter().any(|code| fourcc(code) == kind) {
                self.reader.cursor = next;
            } else if kind == ENTR {
                self.node_entry(next, end)?;
            }
        }
        Ok(())
    }

    /// The `ENTR` chunk of `LoadNodeData`.
    fn node_entry(&mut self, next: i64, end: i64) -> Result<(), ImportError> {
        loop {
            if self.reader.cursor + 8 >= next {
                break;
            }
            let (kind, length, skip) = self.reader.chunk_or_form()?;
            if self.reader.cursor + length as i64 > end {
                return fail("LWO3: cannot read length;");
            }
            let entry_next = self.reader.cursor + length as i64;
            self.reader.cursor += skip;
            match kind {
                FLAG | TAG => self.reader.cursor = entry_next,
                NAME => {
                    self.reader.string(length)?;
                }
                VALU => {
                    self.reader.cursor += 8;
                    let value_type = self.reader.string(8)?;
                    match value_type.as_slice() {
                        b"int" => {
                            self.reader.u4()?;
                        }
                        b"double" => {
                            self.reader.u8_value()?;
                        }
                        b"vparam" => {
                            self.reader.cursor += 24;
                            self.reader.f8()?;
                        }
                        b"vparam3" => {
                            self.reader.cursor += 24;
                            for _ in 0..3 {
                                self.reader.f8()?;
                            }
                        }
                        _ => {}
                    }
                    self.reader.cursor = entry_next;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// `LoadLWO2Clip` and `LoadLWO3Clip` (clips make nothing a mesh shows).
    fn clip(&mut self, length: u32) -> Result<(), ImportError> {
        validate_length(length, "CLIP", if self.lwo3 { 12 } else { 10 })?;
        self.reader.u4()?;
        let (kind, mut length) = if self.lwo3 {
            self.reader.chunk()?
        } else {
            let (kind, length) = self.reader.sub_chunk()?;
            (kind, length as u32)
        };
        match kind {
            STIL => {
                validate_length(length, "STIL", 1)?;
                self.reader.string(length)?;
            }
            ISEQ => {
                validate_length(length, "ISEQ", 16)?;
                self.reader.u1()?;
                self.reader.cursor += 1;
                self.reader.u2()?;
                self.reader.cursor += 4;
                self.reader.u2()?;
                self.reader.cursor += 4;
                let first = self.reader.string(length)?;
                let removed = (first.len() as u16 as u32).wrapping_add(1);
                length = if self.lwo3 { length.wrapping_sub(removed) } else { (length as u16).wrapping_sub(removed as u16) as u32 };
                self.reader.string(length)?;
            }
            XREF => {
                validate_length(length, "XREF", 4)?;
                self.reader.u4()?;
            }
            NEGA => {
                validate_length(length, "NEGA", 2)?;
                self.reader.u2()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// `LoadLWO2Envelope` and `LoadLWO3Envelope` (envelopes make nothing a
    /// mesh shows).
    fn envelope(&mut self, length: u32) -> Result<(), ImportError> {
        let end = self.reader.cursor + length as i64;
        validate_length(length, "ENVL", 4)?;
        self.reader.variable_int()?;
        if self.lxob && !self.lwo3 && self.reader.u4()? != 0 {
            self.reader.cursor -= 4;
        }
        let lwo3 = self.lwo3;
        let header = if lwo3 { 8 } else { 6 };
        let mut keys = 0usize;
        loop {
            if self.reader.cursor + header >= end {
                break;
            }
            let (kind, length) = if lwo3 {
                self.reader.chunk()?
            } else {
                let (kind, length) = self.reader.sub_chunk()?;
                (kind, length as u32)
            };
            if self.reader.cursor + length as i64 > end {
                return fail(if lwo3 { "LWO3: Invalid envelope chunk length" } else { "LWO2: Invalid envelope chunk length" });
            }
            let next = self.reader.cursor + length as i64;
            let extra = if lwo3 { 2 } else { 0 };
            match kind {
                TYPE => {
                    validate_length(length, "TYPE", 2 + extra)?;
                    self.reader.cursor += 1;
                    self.reader.byte(self.reader.cursor)?;
                    self.reader.cursor += 1;
                }
                PRE | POST => {
                    validate_length(length, if kind == PRE { "PRE" } else { "POST" }, 2 + extra)?;
                    self.reader.u2()?;
                }
                KEY => {
                    validate_length(length, "KEY", 8 + extra)?;
                    keys += 1;
                    self.reader.f4()?;
                    self.reader.f4()?;
                }
                SPAN => {
                    validate_length(length, "SPAN", 4 + extra)?;
                    if keys >= 2 {
                        self.reader.u4()?;
                    }
                }
                _ => {}
            }
            self.reader.cursor = next;
        }
        Ok(())
    }

    /// `LoadLWO2File`.
    fn lwo2_file(&mut self, size: u32) -> Result<(), ImportError> {
        let end = self.reader.cursor + size as i64;
        loop {
            if self.reader.cursor + 8 > end {
                break;
            }
            let (kind, length, skip) = self.reader.chunk_or_form()?;
            if self.reader.cursor + length as i64 > end {
                return fail("LWO2: Chunk length points behind the file");
            }
            let next = self.reader.cursor + length as i64;
            self.reader.cursor += skip;
            if length == 0 {
                self.reader.cursor = next;
                continue;
            }
            match kind {
                LAYR => self.layer_chunk(length, next)?,
                PNTS if !self.skip => {
                    let old = self.layer().points.len() as u32;
                    self.points(length)?;
                    self.layer().point_offset = old;
                }
                VMAD | VMAP if !self.skip => {
                    let layer = self.layer();
                    let skip_vmad = kind == VMAD && layer.faces.is_empty();
                    if !skip_vmad && !layer.points.is_empty() {
                        self.vertex_map(length, kind == VMAD)?;
                    }
                }
                POLS if !self.skip => {
                    let old = self.layer().faces.len() as u32;
                    self.polygons(length)?;
                    self.layer().face_offset = old;
                }
                PTAG if !self.skip => {
                    if !self.layer().faces.is_empty() {
                        self.polygon_tags(length)?;
                    }
                }
                TAGS => {
                    if self.model.tags.is_empty() {
                        self.tags(length)?;
                    }
                }
                SURF => {
                    if self.lwo3 {
                        self.surface3(length)?;
                    } else {
                        self.surface2(length)?;
                    }
                }
                CLIP => self.clip(length)?,
                ENVL => self.envelope(length)?,
                _ => {}
            }
            self.reader.cursor = next;
        }
        Ok(())
    }

    /// A `LAYR` chunk: index, pivot, name (or "Layer_N"), maybe a parent.
    /// With one layer asked for, the others' geometry is skipped.
    fn layer_chunk(&mut self, length: u32, next: i64) -> Result<(), ImportError> {
        self.model.layers.push(Layer::new());
        validate_length(length, "LAYR", 16)?;
        let index = self.reader.u2()?;
        self.skip = self.layer_index != u32::MAX && self.layer_index.wrapping_sub(1) != index as u32;
        self.reader.cursor += 2;
        self.floats(3)?;
        let mut name = self.reader.string(length - 16)?;
        if name.is_empty() {
            name = format!("Layer_{}", self.unnamed).into_bytes();
            self.unnamed += 1;
        }
        let parent = if self.reader.cursor + 2 <= next { self.reader.u2()? } else { 0xFFFF };
        let skip = self.skip;
        let layer = self.layer();
        layer.index = index;
        layer.name = name;
        layer.parent = parent;
        layer.skip = skip;
        Ok(())
    }

    /// `LoadLWOBFile`.
    fn lwob_file(&mut self, size: u32) -> Result<(), ImportError> {
        let end = self.reader.cursor + size as i64;
        loop {
            if self.reader.cursor + 8 > end {
                break;
            }
            let (kind, length) = self.reader.chunk()?;
            if self.reader.cursor + length as i64 > end {
                return fail("LWOB: Invalid chunk length");
            }
            let next = self.reader.cursor + length as i64;
            match kind {
                PNTS => {
                    if self.layer().points.is_empty() {
                        self.points(length)?;
                    }
                }
                POLS => {
                    if self.layer().faces.is_empty() {
                        self.lwob_polygons(length)?;
                    }
                }
                SRFS => {
                    if self.model.tags.is_empty() {
                        self.tags(length)?;
                    }
                }
                SURF => self.lwob_surface(length)?,
                _ => {}
            }
            self.reader.cursor = next;
        }
        Ok(())
    }

    /// `LoadLWOBPolygons`: the chunk's 16-bit words swapped in place (an odd
    /// length swaps the byte after it too), faces counted, then copied.
    fn lwob_polygons(&mut self, length: u32) -> Result<(), ImportError> {
        let start = self.reader.cursor;
        let end = start + length as i64;
        let mut word = start;
        while word < end {
            self.reader.swap2(word)?;
            word += 2;
        }
        let mut cursor = start;
        let mut faces = 0u32;
        count_lwob(&self.reader, &mut cursor, end, u32::MAX, &mut faces, 0)?;
        if faces == 0 {
            return Ok(());
        }
        check_allocation(faces as u64, 40)?;
        let mut list = vec![Face::new(super::FACE); faces as usize];
        let mut cursor = start;
        let mut next_face = 0usize;
        let point_count = self.layer().points.len() as u32;
        copy_lwob(&self.reader, &mut cursor, end, u32::MAX, &mut list, &mut next_face, point_count, 0)?;
        self.layer().faces = list;
        Ok(())
    }

    /// `LoadLWOBSurface`: sub-chunks (a length past the surface is cut to
    /// it), legacy textures and their image and flags.
    fn lwob_surface(&mut self, size: u32) -> Result<(), ImportError> {
        let end = self.reader.cursor + size as i64;
        self.model.surfaces.push(Surface::default());
        let name = self.reader.string(size)?;
        self.surface_mut().name = name;
        let mut texture: Option<(usize, usize)> = None;
        loop {
            if self.reader.cursor + 6 >= end {
                break;
            }
            let (kind, mut length) = self.reader.sub_chunk()?;
            if self.reader.cursor + length as i64 > end {
                length = (end - self.reader.cursor) as u16;
            }
            let length = length as u32;
            let next = self.reader.cursor + length as i64;
            match kind {
                COLR => {
                    validate_length(length, "COLR", 3)?;
                    for _ in 0..3 {
                        self.reader.u1()?;
                    }
                }
                DIFF | SPEC | LUMI | TRAN | GLOS => {
                    let name = String::from_utf8_lossy(&kind.to_be_bytes()).to_string();
                    validate_length(length, &name, 2)?;
                    self.reader.u2()?;
                }
                FLAG => {
                    validate_length(length, "FLAG", 2)?;
                    if self.reader.u2()? & 0x4 != 0 {
                        self.surface_mut().smooth_angle = LWOB_SMOOTHING;
                    }
                }
                SMAN => {
                    validate_length(length, "SMAN", 4)?;
                    let angle = self.reader.f4()?.abs();
                    self.surface_mut().smooth_angle = angle;
                }
                CTEX | DTEX | STEX | BTEX | TTEX => {
                    let list = match kind {
                        CTEX => 0,
                        DTEX => 1,
                        STEX => 2,
                        BTEX => 5,
                        _ => 4,
                    };
                    texture = Some(self.lwob_texture(list, length)?);
                }
                TIMG => {
                    if texture.is_some() {
                        self.reader.string(length)?;
                    }
                }
                TVAL => {
                    validate_length(length, "TVAL", 1)?;
                    if texture.is_some() {
                        self.reader.u1()?;
                    }
                }
                TFLG => {
                    validate_length(length, "TFLG", 2)?;
                    if texture.is_some() {
                        self.reader.u2()?;
                    }
                }
                _ => {}
            }
            self.reader.cursor = next;
        }
        Ok(())
    }

    /// `SetupNewTextureLWOB`: a texture whose type names its projection
    /// ("Image Map" ones; others stay UV-mapped).
    fn lwob_texture(&mut self, list: usize, size: u32) -> Result<(usize, usize), ImportError> {
        let kind = self.reader.string(size)?;
        let mut texture = Texture::default();
        let contains = |needle: &[u8]| kind.windows(needle.len()).any(|window| window == needle);
        if contains(b"Image Map") {
            let projections: [(&[u8], u32); 5] = [(b"Planar", 0), (b"Cylindrical", 1), (b"Spherical", 2), (b"Cubic", 3), (b"Front", 4)];
            if let Some(&(_, mapping)) = projections.iter().find(|(name, _)| contains(name)) {
                texture.mapping = mapping;
            }
        }
        let textures = &mut self.surface_mut().textures[list];
        textures.push(texture);
        Ok((list, textures.len() - 1))
    }
}

/// `CountVertsAndFacesLWOB` over the swapped words: a count, its indices,
/// a surface; a negative surface is followed by a count of detail
/// polygons.
fn count_lwob(reader: &Reader, cursor: &mut i64, end: i64, max: u32, faces: &mut u32, depth: usize) -> Result<(), ImportError> {
    if depth > MAX_CHAIN {
        return undefined("detail polygons nested too deep for the stack");
    }
    let mut remaining = max;
    while *cursor < end {
        if remaining == 0 {
            break;
        }
        remaining -= 1;
        // (`uint16_t` pointer differences.)
        if (end - *cursor) >> 1 < 2 {
            return fail("LWOB: Unexpected end of file");
        }
        let count = u16::from_le_bytes(reader.array(*cursor)?);
        *cursor += 2;
        if (end - *cursor) >> 1 < 1 + count as i64 {
            return fail("LWOB: Unexpected end of file");
        }
        *faces = faces.wrapping_add(1);
        *cursor += 2 * count as i64;
        let surface = i16::from_le_bytes(reader.array(*cursor)?);
        *cursor += 2;
        if surface < 0 {
            let details = u16::from_le_bytes(reader.array(*cursor)?);
            *cursor += 2;
            count_lwob(reader, cursor, end, details as u32, faces, depth + 1)?;
        }
    }
    Ok(())
}

/// `CopyFaceIndicesLWOB`: a face whose indices would reach the end keeps
/// its count without indices, and the copy stops.
#[allow(clippy::too_many_arguments)]
fn copy_lwob(
    reader: &Reader,
    cursor: &mut i64,
    end: i64,
    max: u32,
    faces: &mut [Face],
    next_face: &mut usize,
    point_count: u32,
    depth: usize,
) -> Result<(), ImportError> {
    if depth > MAX_CHAIN {
        return undefined("detail polygons nested too deep for the stack");
    }
    let mut remaining = max;
    while *cursor < end {
        if remaining == 0 {
            break;
        }
        remaining -= 1;
        let Some(face) = faces.get_mut(*next_face) else {
            return undefined("a face written past the face list");
        };
        *next_face += 1;
        let count = u16::from_le_bytes(reader.array(*cursor)?);
        *cursor += 2;
        face.count = count as u32;
        if count != 0 {
            if *cursor + 2 * count as i64 >= end {
                break;
            }
            let mut indices = Vec::with_capacity(count as usize);
            for _ in 0..count {
                let mut index = u16::from_le_bytes(reader.array(*cursor)?) as u32;
                *cursor += 2;
                if index > point_count {
                    index = point_count.wrapping_sub(1);
                }
                indices.push(index);
            }
            face.indices = Some(indices);
        }
        let mut surface = i16::from_le_bytes(reader.array(*cursor)?);
        *cursor += 2;
        let face_slot = *next_face - 1;
        if surface < 0 {
            surface = surface.wrapping_neg();
            let details = u16::from_le_bytes(reader.array(*cursor)?);
            *cursor += 2;
            if *cursor < end {
                copy_lwob(reader, cursor, end, details as u32, faces, next_face, point_count, depth + 1)?;
            }
        }
        faces[face_slot].surface = (surface as i32 - 1) as u32;
    }
    Ok(())
}

/// `AddToSingleLinkedList`: `target` at the end of `source`'s chain of
/// copies.
fn link(referrers: &mut [u32], source: u32, target: u32) -> Result<(), ImportError> {
    let mut current = source;
    for _ in 0..=MAX_CHAIN {
        let Some(next) = referrers.get_mut(current as usize) else {
            return undefined("a point referrer read past the points");
        };
        if *next == u32::MAX {
            *next = target;
            return Ok(());
        }
        current = *next;
    }
    undefined("a chain of point copies too long for the stack")
}

/// The header and the chunks (`InternReadFile` up to the meshes).
pub(super) fn read_chunks(bytes: &[u8], layer_index: u32) -> Result<Model, ImportError> {
    let mut reader = Reader { data: bytes.to_vec(), cursor: 0 };
    let (kind, _) = reader.chunk()?;
    if kind != FORM {
        return fail("The file is not an IFF file: FORM chunk is missing");
    }
    let file_type = reader.u4()?;
    let mut default_layer = Layer::new();
    default_layer.name = b"<LWODefault>".to_vec();
    default_layer.index = 1;
    let model = Model { layers: vec![default_layer], tags: Vec::new(), surfaces: Vec::new() };
    let mut parser = Parser { reader, model, lwo2: false, lwo3: false, lxob: false, unnamed: 0, layer_index, skip: false };
    let size = bytes.len() as u32 - 12;
    match file_type {
        LWOB => parser.lwob_file(size)?,
        LWO2 | LWO3 | LXOB => {
            parser.lxob = file_type == LXOB;
            parser.lwo3 = file_type == LWO3;
            parser.lwo2 = file_type != LWO3;
            parser.lwo2_file(size)?;
            // The asked-for layer and the default one, nothing else.
            if layer_index != u32::MAX {
                let loaded = parser.model.layers.iter().filter(|layer| !layer.skip).count();
                if loaded != 2 {
                    return fail("LWO2: The requested layer was not found");
                }
            }
        }
        _ => {
            let name = String::from_utf8_lossy(&file_type.to_be_bytes()).to_string();
            return fail(format!("Unknown LWO sub format: {name}"));
        }
    }
    Ok(parser.model)
}
