//! Assimp's DXF importer (`DXFLoader.cpp`, ASCII only): group code and
//! value line pairs; BLOCKS and ENTITIES give polylines (polyface meshes
//! by their one-based vertex indices, other polylines as segments), 3D
//! faces and lines; INSERTs in ENTITIES copy a block's geometry (moved by
//! the block's base, the insertion point and scale; rotation ignored). A
//! mesh per layer, in the order layers first appear, every face's
//! vertices written out with their colours (the AutoCAD index colours,
//! grey by default).
//!
//! Kept as Assimp reads: the line splitter loses a file's last byte when
//! a trimmed break stops on it; a final group code without its value
//! keeps the value before; values compare up to a NUL; a value starting
//! with '{' skips lines to one starting with '}' or an empty one; numbers
//! that don't parse throw; only a polyline's first non-zero flags count.

use std::collections::BTreeMap;

use crate::fast_atof::{fast_atof, strtol10, strtoul10};
use crate::line_splitter::LineSplitter;
use crate::matrix::Matrix4;
use crate::scene::{self, ImportError, Mesh, PRIMITIVE_LINE, PRIMITIVE_POINT, PRIMITIVE_POLYGON, PRIMITIVE_TRIANGLE, Scene, fail};

/// `AI_DXF_BINARY_IDENT` with its NUL.
const BINARY_IDENT: &[u8; 22] = b"AutoCAD Binary DXF\r\n\x1a\0";
/// `AI_DXF_DEFAULT_COLOR`.
const DEFAULT_COLOR: [f32; 4] = [0.6, 0.6, 0.6, 0.6];
/// `AI_DXF_ENTITIES_MAGIC_BLOCK`.
const ENTITIES_BLOCK: &[u8] = b"$ASSIMP_ENTITIES_MAGIC";
const POLYLINE_CLOSED: u32 = 0x1;
const POLYLINE_POLYFACE_MESH: u32 = 0x40;
/// `sizeof(aiVector3D)` and `sizeof(unsigned int)`, for the reserves the
/// vertex and face counts ask for.
const POSITION_SIZE: u64 = 12;
const INDEX_SIZE: u64 = 4;

/// `g_aclrDxfIndexColors`: the AutoCAD Color Index.
const INDEX_COLORS: [[f32; 4]; 256] = [
    [0.0, 0.0, 0.0, 1.0], [1.0, 0.0, 0.0, 1.0], [1.0, 1.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0],
    [0.0, 1.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 0.0, 1.0, 1.0], [1.0, 1.0, 1.0, 1.0],
    [0.3, 0.3, 0.3, 1.0], [0.5, 0.5, 0.5, 1.0], [1.0, 0.0, 0.0, 1.0], [1.0, 0.7, 0.7, 1.0],
    [0.7, 0.0, 0.0, 1.0], [0.7, 0.5, 0.5, 1.0], [0.5, 0.0, 0.0, 1.0], [0.5, 0.3, 0.3, 1.0],
    [0.4, 0.0, 0.0, 1.0], [0.4, 0.3, 0.3, 1.0], [0.3, 0.0, 0.0, 1.0], [0.3, 0.2, 0.2, 1.0],
    [1.0, 0.2, 0.0, 1.0], [1.0, 0.7, 0.7, 1.0], [0.7, 0.2, 0.0, 1.0], [0.7, 0.6, 0.5, 1.0],
    [0.5, 0.1, 0.0, 1.0], [0.5, 0.4, 0.3, 1.0], [0.4, 0.1, 0.0, 1.0], [0.4, 0.3, 0.3, 1.0],
    [0.3, 0.1, 0.0, 1.0], [0.3, 0.2, 0.2, 1.0], [1.0, 0.5, 0.0, 1.0], [1.0, 0.8, 0.7, 1.0],
    [0.7, 0.4, 0.0, 1.0], [0.7, 0.6, 0.5, 1.0], [0.5, 0.3, 0.0, 1.0], [0.5, 0.4, 0.3, 1.0],
    [0.4, 0.2, 0.0, 1.0], [0.4, 0.3, 0.3, 1.0], [0.3, 0.2, 0.0, 1.0], [0.3, 0.3, 0.2, 1.0],
    [1.0, 0.7, 0.0, 1.0], [1.0, 0.9, 0.7, 1.0], [0.7, 0.6, 0.0, 1.0], [0.7, 0.7, 0.5, 1.0],
    [0.5, 0.4, 0.0, 1.0], [0.5, 0.5, 0.3, 1.0], [0.4, 0.3, 0.0, 1.0], [0.4, 0.4, 0.3, 1.0],
    [0.3, 0.2, 0.0, 1.0], [0.3, 0.3, 0.2, 1.0], [1.0, 1.0, 0.0, 1.0], [1.0, 1.0, 0.7, 1.0],
    [0.7, 0.7, 0.0, 1.0], [0.7, 0.7, 0.5, 1.0], [0.5, 0.5, 0.0, 1.0], [0.5, 0.5, 0.3, 1.0],
    [0.4, 0.4, 0.0, 1.0], [0.4, 0.4, 0.3, 1.0], [0.3, 0.3, 0.0, 1.0], [0.3, 0.3, 0.2, 1.0],
    [0.7, 1.0, 0.0, 1.0], [0.9, 1.0, 0.7, 1.0], [0.6, 0.7, 0.0, 1.0], [0.7, 0.7, 0.5, 1.0],
    [0.4, 0.5, 0.0, 1.0], [0.5, 0.5, 0.3, 1.0], [0.3, 0.4, 0.0, 1.0], [0.4, 0.4, 0.3, 1.0],
    [0.2, 0.3, 0.0, 1.0], [0.3, 0.3, 0.2, 1.0], [0.5, 1.0, 0.0, 1.0], [0.8, 1.0, 0.7, 1.0],
    [0.4, 0.7, 0.0, 1.0], [0.6, 0.7, 0.5, 1.0], [0.3, 0.5, 0.0, 1.0], [0.4, 0.5, 0.3, 1.0],
    [0.2, 0.4, 0.0, 1.0], [0.3, 0.4, 0.3, 1.0], [0.2, 0.3, 0.0, 1.0], [0.3, 0.3, 0.2, 1.0],
    [0.2, 1.0, 0.0, 1.0], [0.7, 1.0, 0.7, 1.0], [0.2, 0.7, 0.0, 1.0], [0.6, 0.7, 0.5, 1.0],
    [0.1, 0.5, 0.0, 1.0], [0.4, 0.5, 0.3, 1.0], [0.1, 0.4, 0.0, 1.0], [0.3, 0.4, 0.3, 1.0],
    [0.1, 0.3, 0.0, 1.0], [0.2, 0.3, 0.2, 1.0], [0.0, 1.0, 0.0, 1.0], [0.7, 1.0, 0.7, 1.0],
    [0.0, 0.7, 0.0, 1.0], [0.5, 0.7, 0.5, 1.0], [0.0, 0.5, 0.0, 1.0], [0.3, 0.5, 0.3, 1.0],
    [0.0, 0.4, 0.0, 1.0], [0.3, 0.4, 0.3, 1.0], [0.0, 0.3, 0.0, 1.0], [0.2, 0.3, 0.2, 1.0],
    [0.0, 1.0, 0.2, 1.0], [0.7, 1.0, 0.7, 1.0], [0.0, 0.7, 0.2, 1.0], [0.5, 0.7, 0.6, 1.0],
    [0.0, 0.5, 0.1, 1.0], [0.3, 0.5, 0.4, 1.0], [0.0, 0.4, 0.1, 1.0], [0.3, 0.4, 0.3, 1.0],
    [0.0, 0.3, 0.1, 1.0], [0.2, 0.3, 0.2, 1.0], [0.0, 1.0, 0.5, 1.0], [0.7, 1.0, 0.8, 1.0],
    [0.0, 0.7, 0.4, 1.0], [0.5, 0.7, 0.6, 1.0], [0.0, 0.5, 0.3, 1.0], [0.3, 0.5, 0.4, 1.0],
    [0.0, 0.4, 0.2, 1.0], [0.3, 0.4, 0.3, 1.0], [0.0, 0.3, 0.2, 1.0], [0.2, 0.3, 0.3, 1.0],
    [0.0, 1.0, 0.7, 1.0], [0.7, 1.0, 0.9, 1.0], [0.0, 0.7, 0.6, 1.0], [0.5, 0.7, 0.7, 1.0],
    [0.0, 0.5, 0.4, 1.0], [0.3, 0.5, 0.5, 1.0], [0.0, 0.4, 0.3, 1.0], [0.3, 0.4, 0.4, 1.0],
    [0.0, 0.3, 0.2, 1.0], [0.2, 0.3, 0.3, 1.0], [0.0, 1.0, 1.0, 1.0], [0.7, 1.0, 1.0, 1.0],
    [0.0, 0.7, 0.7, 1.0], [0.5, 0.7, 0.7, 1.0], [0.0, 0.5, 0.5, 1.0], [0.3, 0.5, 0.5, 1.0],
    [0.0, 0.4, 0.4, 1.0], [0.3, 0.4, 0.4, 1.0], [0.0, 0.3, 0.3, 1.0], [0.2, 0.3, 0.3, 1.0],
    [0.0, 0.7, 1.0, 1.0], [0.7, 0.9, 1.0, 1.0], [0.0, 0.6, 0.7, 1.0], [0.5, 0.7, 0.7, 1.0],
    [0.0, 0.4, 0.5, 1.0], [0.3, 0.5, 0.5, 1.0], [0.0, 0.3, 0.4, 1.0], [0.3, 0.4, 0.4, 1.0],
    [0.0, 0.2, 0.3, 1.0], [0.2, 0.3, 0.3, 1.0], [0.0, 0.5, 1.0, 1.0], [0.7, 0.8, 1.0, 1.0],
    [0.0, 0.4, 0.7, 1.0], [0.5, 0.6, 0.7, 1.0], [0.0, 0.3, 0.5, 1.0], [0.3, 0.4, 0.5, 1.0],
    [0.0, 0.2, 0.4, 1.0], [0.3, 0.3, 0.4, 1.0], [0.0, 0.2, 0.3, 1.0], [0.2, 0.3, 0.3, 1.0],
    [0.0, 0.2, 1.0, 1.0], [0.7, 0.7, 1.0, 1.0], [0.0, 0.2, 0.7, 1.0], [0.5, 0.6, 0.7, 1.0],
    [0.0, 0.1, 0.5, 1.0], [0.3, 0.4, 0.5, 1.0], [0.0, 0.1, 0.4, 1.0], [0.3, 0.3, 0.4, 1.0],
    [0.0, 0.1, 0.3, 1.0], [0.2, 0.2, 0.3, 1.0], [0.0, 0.0, 1.0, 1.0], [0.7, 0.7, 1.0, 1.0],
    [0.0, 0.0, 0.7, 1.0], [0.5, 0.5, 0.7, 1.0], [0.0, 0.0, 0.5, 1.0], [0.3, 0.3, 0.5, 1.0],
    [0.0, 0.0, 0.4, 1.0], [0.3, 0.3, 0.4, 1.0], [0.0, 0.0, 0.3, 1.0], [0.2, 0.2, 0.3, 1.0],
    [0.2, 0.0, 1.0, 1.0], [0.7, 0.7, 1.0, 1.0], [0.2, 0.0, 0.7, 1.0], [0.6, 0.5, 0.7, 1.0],
    [0.1, 0.0, 0.5, 1.0], [0.4, 0.3, 0.5, 1.0], [0.1, 0.0, 0.4, 1.0], [0.3, 0.3, 0.4, 1.0],
    [0.1, 0.0, 0.3, 1.0], [0.2, 0.2, 0.3, 1.0], [0.5, 0.0, 1.0, 1.0], [0.8, 0.7, 1.0, 1.0],
    [0.4, 0.0, 0.7, 1.0], [0.6, 0.5, 0.7, 1.0], [0.3, 0.0, 0.5, 1.0], [0.4, 0.3, 0.5, 1.0],
    [0.2, 0.0, 0.4, 1.0], [0.3, 0.3, 0.4, 1.0], [0.2, 0.0, 0.3, 1.0], [0.3, 0.2, 0.3, 1.0],
    [0.7, 0.0, 1.0, 1.0], [0.9, 0.7, 1.0, 1.0], [0.6, 0.0, 0.7, 1.0], [0.7, 0.5, 0.7, 1.0],
    [0.4, 0.0, 0.5, 1.0], [0.5, 0.3, 0.5, 1.0], [0.3, 0.0, 0.4, 1.0], [0.4, 0.3, 0.4, 1.0],
    [0.2, 0.0, 0.3, 1.0], [0.3, 0.2, 0.3, 1.0], [1.0, 0.0, 1.0, 1.0], [1.0, 0.7, 1.0, 1.0],
    [0.7, 0.0, 0.7, 1.0], [0.7, 0.5, 0.7, 1.0], [0.5, 0.0, 0.5, 1.0], [0.5, 0.3, 0.5, 1.0],
    [0.4, 0.0, 0.4, 1.0], [0.4, 0.3, 0.4, 1.0], [0.3, 0.0, 0.3, 1.0], [0.3, 0.2, 0.3, 1.0],
    [1.0, 0.0, 0.7, 1.0], [1.0, 0.7, 0.9, 1.0], [0.7, 0.0, 0.6, 1.0], [0.7, 0.5, 0.7, 1.0],
    [0.5, 0.0, 0.4, 1.0], [0.5, 0.3, 0.5, 1.0], [0.4, 0.0, 0.3, 1.0], [0.4, 0.3, 0.4, 1.0],
    [0.3, 0.0, 0.2, 1.0], [0.3, 0.2, 0.3, 1.0], [1.0, 0.0, 0.5, 1.0], [1.0, 0.7, 0.8, 1.0],
    [0.7, 0.0, 0.4, 1.0], [0.7, 0.5, 0.6, 1.0], [0.5, 0.0, 0.3, 1.0], [0.5, 0.3, 0.4, 1.0],
    [0.4, 0.0, 0.2, 1.0], [0.4, 0.3, 0.3, 1.0], [0.3, 0.0, 0.2, 1.0], [0.3, 0.2, 0.3, 1.0],
    [1.0, 0.0, 0.2, 1.0], [1.0, 0.7, 0.7, 1.0], [0.7, 0.0, 0.2, 1.0], [0.7, 0.5, 0.6, 1.0],
    [0.5, 0.0, 0.1, 1.0], [0.5, 0.3, 0.4, 1.0], [0.4, 0.0, 0.1, 1.0], [0.4, 0.3, 0.3, 1.0],
    [0.3, 0.0, 0.1, 1.0], [0.3, 0.2, 0.2, 1.0], [0.2, 0.2, 0.2, 1.0], [0.3, 0.3, 0.3, 1.0],
    [0.4, 0.4, 0.4, 1.0], [0.5, 0.5, 0.5, 1.0], [0.7, 0.7, 0.7, 1.0], [1.0, 1.0, 1.0, 1.0],
];

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// A value as `c_str()` gives it: up to its first NUL.
fn c_string(value: &[u8]) -> &[u8] {
    match value.iter().position(|&byte| byte == 0) {
        Some(nul) => &value[..nul],
        None => value,
    }
}

/// `DXF::LineReader`: the file's group code and value line pairs.
struct LineReader<'a> {
    splitter: LineSplitter<'a>,
    group_code: i32,
    value: Vec<u8>,
    end: u32,
}

impl<'a> LineReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { splitter: LineSplitter::new(bytes, false, true), group_code: 0, value: Vec::new(), end: 0 }
    }

    /// `Is(gc, what)`.
    fn is(&self, group_code: i32, what: &[u8]) -> bool {
        self.group_code == group_code && c_string(&self.value) == what
    }

    /// `End`.
    fn at_end(&self) -> bool {
        self.end > 1
    }

    fn value_as_unsigned(&self) -> u32 {
        strtoul10(&self.value, 0).0
    }

    fn value_as_signed(&self) -> i32 {
        strtol10(&self.value, 0).0
    }

    /// `ValueAsFloat`: `fast_atof`, which throws on a value that isn't a
    /// number.
    fn value_as_float(&self) -> Result<f32, ImportError> {
        fast_atof(c_string(&self.value))
    }

    /// `operator++`: the next pair; once the lines run out, one more
    /// round before `End`.
    fn advance(&mut self) {
        if self.end != 0 {
            if self.end == 1 {
                self.end += 1;
            }
            return;
        }
        self.read_pair();
        if !self.splitter.is_valid() {
            self.end = 1;
        }
    }

    /// The `try` block of `operator++`: the splitter's `logic_error` (no
    /// lines left) stops it where it is.
    fn read_pair(&mut self) {
        self.group_code = strtol10(self.splitter.current(), 0).0;
        if !self.splitter.advance() {
            return;
        }
        self.value = self.splitter.current().to_vec();
        if !self.splitter.advance() {
            return;
        }
        if self.value.first() != Some(&b'{') {
            return;
        }
        loop {
            let line = self.splitter.current();
            if line.is_empty() || line[0] == b'}' {
                break;
            }
            if !self.splitter.advance() {
                return;
            }
        }
        self.splitter.advance();
    }
}

/// `DXF::PolyLine` (also a 3D face or line).
#[derive(Clone, Default)]
struct PolyLine {
    positions: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
    counts: Vec<u32>,
    flags: u32,
    layer: Vec<u8>,
}

/// `DXF::InsertBlock`.
struct InsertBlock {
    position: [f32; 3],
    scale: [f32; 3],
    angle: f32,
    name: Vec<u8>,
}

/// `DXF::Block`.
#[derive(Default)]
struct Block {
    lines: Vec<PolyLine>,
    insertions: Vec<InsertBlock>,
    name: Vec<u8>,
    base: [f32; 3],
}

/// `SkipSection` and `ParseHeader`: on to the section's end.
fn skip_section(reader: &mut LineReader) {
    while !reader.at_end() && !reader.is(0, b"ENDSEC") {
        reader.advance();
    }
}

/// `ParseBlocks`.
fn parse_blocks(reader: &mut LineReader, blocks: &mut Vec<Block>) -> Result<(), ImportError> {
    while !reader.at_end() && !reader.is(0, b"ENDSEC") {
        if reader.is(0, b"BLOCK") {
            reader.advance();
            parse_block(reader, blocks)?;
            continue;
        }
        reader.advance();
    }
    Ok(())
}

/// `ParseBlock`: its name, base point and geometry up to ENDBLK (an
/// INSERT skips the rest).
fn parse_block(reader: &mut LineReader, blocks: &mut Vec<Block>) -> Result<(), ImportError> {
    let mut block = Block::default();
    while !reader.at_end() && !reader.is(0, b"ENDBLK") {
        match reader.group_code {
            2 => block.name = reader.value.clone(),
            10 => block.base[0] = reader.value_as_float()?,
            20 => block.base[1] = reader.value_as_float()?,
            30 => block.base[2] = reader.value_as_float()?,
            _ => {}
        }
        if reader.is(0, b"POLYLINE") {
            reader.advance();
            parse_polyline(reader, &mut block)?;
            continue;
        }
        if reader.is(0, b"INSERT") {
            while !reader.at_end() && !reader.is(0, b"ENDBLK") {
                reader.advance();
            }
            break;
        }
        if reader.is(0, b"3DFACE") || reader.is(0, b"LINE") || reader.is(0, b"3DLINE") {
            reader.advance();
            parse_3d_face(reader, &mut block)?;
            continue;
        }
        reader.advance();
    }
    blocks.push(block);
    Ok(())
}

/// `ParseEntities`: the geometry and insertions up to ENDSEC, as a block
/// of its own.
fn parse_entities(reader: &mut LineReader, blocks: &mut Vec<Block>) -> Result<(), ImportError> {
    let mut block = Block { name: ENTITIES_BLOCK.to_vec(), ..Block::default() };
    while !reader.at_end() && !reader.is(0, b"ENDSEC") {
        if reader.is(0, b"POLYLINE") {
            reader.advance();
            parse_polyline(reader, &mut block)?;
            continue;
        }
        if reader.is(0, b"INSERT") {
            reader.advance();
            parse_insertion(reader, &mut block)?;
            continue;
        }
        if reader.is(0, b"3DFACE") || reader.is(0, b"LINE") || reader.is(0, b"3DLINE") {
            reader.advance();
            parse_3d_face(reader, &mut block)?;
            continue;
        }
        reader.advance();
    }
    blocks.push(block);
    Ok(())
}

/// `ParseInsertion`: up to the next entity.
fn parse_insertion(reader: &mut LineReader, block: &mut Block) -> Result<(), ImportError> {
    let mut insertion = InsertBlock { position: [0.0; 3], scale: [1.0; 3], angle: 0.0, name: Vec::new() };
    while !reader.at_end() && reader.group_code != 0 {
        match reader.group_code {
            2 => insertion.name = reader.value.clone(),
            10 => insertion.position[0] = reader.value_as_float()?,
            20 => insertion.position[1] = reader.value_as_float()?,
            30 => insertion.position[2] = reader.value_as_float()?,
            41 => insertion.scale[0] = reader.value_as_float()?,
            42 => insertion.scale[1] = reader.value_as_float()?,
            43 => insertion.scale[2] = reader.value_as_float()?,
            50 => insertion.angle = reader.value_as_float()?,
            _ => {}
        }
        reader.advance();
    }
    block.insertions.push(insertion);
    Ok(())
}

/// `ParsePolyLine`: its vertices up to SEQEND (or ENDSEC), its first
/// non-zero flags, layer and the counts it reserves for; a polyface mesh
/// short of three vertices or indices is dropped, and a plain polyline's
/// vertices pair up into segments (closed: the last back to the first).
fn parse_polyline(reader: &mut LineReader, block: &mut Block) -> Result<(), ImportError> {
    let mut line = PolyLine::default();
    while !reader.at_end() && !reader.is(0, b"ENDSEC") {
        if reader.is(0, b"VERTEX") {
            reader.advance();
            parse_polyline_vertex(reader, &mut line)?;
            if reader.is(0, b"SEQEND") {
                break;
            }
            continue;
        }
        match reader.group_code {
            70 => {
                if line.flags == 0 {
                    line.flags = reader.value_as_signed() as u32;
                }
            }
            71 => {
                let vertex_guess = reader.value_as_signed() as u32;
                scene::check_allocation(vertex_guess as u64, POSITION_SIZE)?;
            }
            72 => {
                let index_guess = reader.value_as_signed() as u32;
                scene::check_allocation(index_guess as u64, INDEX_SIZE)?;
            }
            8 => line.layer = reader.value.clone(),
            _ => {}
        }
        reader.advance();
    }

    if line.flags & POLYLINE_POLYFACE_MESH != 0 {
        if line.positions.len() < 3 || line.indices.len() < 3 {
            return Ok(());
        }
    } else if line.indices.is_empty() && line.counts.is_empty() {
        let pair_count = (line.positions.len() / 2) as u32;
        for pair in 0..pair_count {
            line.indices.push(pair * 2);
            line.indices.push(pair * 2 + 1);
            line.counts.push(2);
        }
        if line.flags & POLYLINE_CLOSED != 0 {
            let last = (line.positions.len() as u32).wrapping_sub(1);
            line.indices.push(last);
            line.indices.push(0);
            line.counts.push(2);
        }
    }
    block.lines.push(line);
    Ok(())
}

/// `ParsePolyLineVertex`: a face's (up to four, one-based, signs
/// dropped) vertex indices, zero ones left out; or else a vertex.
fn parse_polyline_vertex(reader: &mut LineReader, line: &mut PolyLine) -> Result<(), ImportError> {
    let mut index_count = 0usize;
    let mut indices = [0u32; 4];
    let mut position = [0.0f32; 3];
    let mut color = DEFAULT_COLOR;
    while !reader.at_end() {
        if reader.group_code == 0 {
            break;
        }
        match reader.group_code {
            10 => position[0] = reader.value_as_float()?,
            20 => position[1] = reader.value_as_float()?,
            30 => position[2] = reader.value_as_float()?,
            71..=74 => {
                if index_count < 4 {
                    let index = reader.value_as_signed();
                    indices[index_count] = index.wrapping_abs() as u32;
                    index_count += 1;
                }
            }
            62 => color = INDEX_COLORS[(reader.value_as_unsigned() % 256) as usize],
            _ => {}
        }
        reader.advance();
    }

    if index_count == 0 {
        line.positions.push(position);
        line.colors.push(color);
        return Ok(());
    }
    line.counts.push(index_count as u32);
    for &index in &indices[..index_count] {
        if index != 0 {
            line.indices.push(index - 1);
            continue;
        }
        let Some(count) = line.counts.last_mut() else {
            return undefined("a face count taken from none");
        };
        *count -= 1;
        if *count == 0 {
            line.counts.pop();
        }
    }
    Ok(())
}

/// `Parse3DFace` (also LINE and 3DLINE): two to four corners; the fourth
/// counts only when it differs from the third, and the third and fourth
/// only when the first two are there.
fn parse_3d_face(reader: &mut LineReader, block: &mut Block) -> Result<(), ImportError> {
    let mut corners = [[0.0f32; 3]; 4];
    let mut color = DEFAULT_COLOR;
    let mut layer = Vec::new();
    // `b[]`: third corner, fourth corner, first corner, second corner.
    let mut seen = [false; 4];
    while !reader.at_end() {
        if reader.group_code == 0 {
            break;
        }
        match reader.group_code {
            8 => layer = reader.value.clone(),
            10 | 20 | 30 => {
                corners[0][(reader.group_code / 10 - 1) as usize] = reader.value_as_float()?;
                seen[2] = true;
            }
            11 | 21 | 31 => {
                corners[1][(reader.group_code / 10 - 1) as usize] = reader.value_as_float()?;
                seen[3] = true;
            }
            12 | 22 | 32 => {
                corners[2][(reader.group_code / 10 - 1) as usize] = reader.value_as_float()?;
                seen[0] = true;
            }
            13 | 23 | 33 => {
                corners[3][(reader.group_code / 10 - 1) as usize] = reader.value_as_float()?;
                seen[1] = true;
            }
            62 => color = INDEX_COLORS[(reader.value_as_unsigned() % 256) as usize],
            _ => {}
        }
        reader.advance();
    }

    if corners[3] == corners[2] {
        seen[1] = false;
    }
    if (seen[1] && !seen[0]) || !seen[2] || !seen[3] {
        return Ok(());
    }
    let count = 2 + seen[0] as usize + seen[1] as usize;
    let mut line = PolyLine { layer, ..PolyLine::default() };
    line.counts.push(count as u32);
    for corner in &corners[..count] {
        line.indices.push(line.positions.len() as u32);
        line.positions.push(*corner);
        line.colors.push(color);
    }
    block.lines.push(line);
    Ok(())
}

/// `aiVector3D::Length`.
fn length(vector: [f32; 3]) -> f32 {
    let square = vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2];
    square.sqrt()
}

/// `ExpandBlockReferences`: each insertion's block (the last of its name)
/// copied into ENTITIES, as many lines as it had when its turn came,
/// moved by `Translation(-base) × Translation(position) × Scaling(scale)`
/// unless all of those are neutral.
fn expand_block_references(blocks: &mut [Block], entities: usize, by_name: &BTreeMap<Vec<u8>, usize>) {
    let insertions = std::mem::take(&mut blocks[entities].insertions);
    for insertion in &insertions {
        let Some(&source) = by_name.get(&insertion.name) else {
            continue;
        };
        let base = blocks[source].base;
        let scale = insertion.scale;
        let neutral_scale = scale[0] == 1.0 && scale[1] == 1.0 && scale[2] == 1.0;
        let moved = length(base) != 0.0 || !neutral_scale || insertion.angle != 0.0 || length(insertion.position) != 0.0;
        let mut transform = Matrix4::translation([-base[0], -base[1], -base[2]]);
        transform.multiply_assign(&Matrix4::translation(insertion.position));
        transform.multiply_assign(&Matrix4::scaling(scale));

        let copies = blocks[source].lines.clone();
        for mut line in copies {
            if moved {
                for position in &mut line.positions {
                    *position = transform.transform(*position);
                }
            }
            blocks[entities].lines.push(line);
        }
    }
    blocks[entities].insertions = insertions;
}

/// One layer's mesh: every face's vertices written out in turn.
fn layer_mesh(lines: &[PolyLine], members: &[usize]) -> Result<Mesh, ImportError> {
    let mut vertices = Vec::new();
    let mut colors = Vec::new();
    let mut faces = Vec::new();
    let mut primitive_types = 0;
    for &member in members {
        let line = &lines[member];
        let mut cursor = 0usize;
        for &count in &line.counts {
            let mut face = Vec::with_capacity(count as usize);
            for _ in 0..count {
                face.push(vertices.len() as u32);
                let Some(&index) = line.indices.get(cursor) else {
                    return undefined("a face index read past the indices");
                };
                cursor += 1;
                let index = index as usize;
                if index >= line.positions.len() {
                    return fail("DXF: vertex index out of bounds");
                }
                vertices.push(line.positions[index]);
                colors.push(line.colors[index]);
            }
            primitive_types |= match count {
                1 => PRIMITIVE_POINT,
                2 => PRIMITIVE_LINE,
                3 => PRIMITIVE_TRIANGLE,
                _ => PRIMITIVE_POLYGON,
            };
            faces.push(face);
        }
    }
    let mut mesh = Mesh { primitive_types, vertex_count: vertices.len(), vertices: Some(vertices), faces: Some(faces), ..Mesh::default() };
    mesh.colors[0] = Some(colors);
    Ok(mesh)
}

/// `ConvertMeshes`: ENTITIES (the first block of that name) with its
/// insertions expanded, a mesh per layer of its lines with vertices.
fn convert_meshes(mut blocks: Vec<Block>) -> Result<Vec<Mesh>, ImportError> {
    if blocks.is_empty() {
        return fail("DXF: no data blocks loaded");
    }
    let mut by_name = BTreeMap::new();
    let mut entities = None;
    for (index, block) in blocks.iter().enumerate() {
        by_name.insert(block.name.clone(), index);
        if entities.is_none() && block.name == ENTITIES_BLOCK {
            entities = Some(index);
        }
    }
    let Some(entities) = entities else {
        return fail("DXF: no ENTITIES data block loaded");
    };
    expand_block_references(&mut blocks, entities, &by_name);

    let lines = &blocks[entities].lines;
    let mut layers: BTreeMap<&[u8], usize> = BTreeMap::new();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if line.positions.is_empty() {
            continue;
        }
        match layers.get(line.layer.as_slice()) {
            Some(&group) => groups[group].push(index),
            None => {
                layers.insert(line.layer.as_slice(), groups.len());
                groups.push(vec![index]);
            }
        }
    }
    if groups.is_empty() {
        return fail("DXF: this file contains no 3d data");
    }
    let mut meshes = Vec::with_capacity(groups.len());
    for members in &groups {
        meshes.push(layer_mesh(lines, members)?);
    }
    Ok(meshes)
}

pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    let mut head = [0u8; 22];
    let head_length = bytes.len().min(head.len());
    head[..head_length].copy_from_slice(&bytes[..head_length]);
    if &head == BINARY_IDENT {
        return fail("DXF: Binary files are not supported at the moment");
    }
    if bytes.is_empty() {
        return fail("StreamReader: File is empty or EOF is already reached");
    }

    let mut reader = LineReader::new(bytes);
    let mut blocks = Vec::new();
    while !reader.at_end() {
        if reader.is(2, b"BLOCKS") {
            parse_blocks(&mut reader, &mut blocks)?;
            continue;
        }
        if reader.is(2, b"ENTITIES") {
            parse_entities(&mut reader, &mut blocks)?;
            continue;
        }
        let skipped = reader.is(2, b"CLASSES") || reader.is(2, b"TABLES") || reader.is(2, b"HEADER");
        if skipped {
            skip_section(&mut reader);
            continue;
        }
        if reader.is(0, b"EOF") {
            break;
        }
        reader.advance();
    }
    let meshes = convert_meshes(blocks)?;
    Ok(Scene { meshes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faces_and_polylines_by_layer() {
        let text = "0\nSECTION\n2\nENTITIES\n0\n3DFACE\n8\nB\n10\n0\n20\n0\n30\n0\n11\n1\n21\n0\n31\n0\n12\n1\n22\n1\n32\n0\n13\n1\n23\n1\n33\n0\n62\n1\n0\nPOLYLINE\n8\nA\n70\n1\n0\nVERTEX\n10\n0\n20\n0\n30\n0\n0\nVERTEX\n10\n5\n20\n0\n30\n0\n0\nSEQEND\n0\nENDSEC\n0\nEOF\n";
        let scene = read(text.as_bytes()).unwrap();
        assert_eq!(scene.meshes.len(), 2);
        // The fourth corner repeats the third: a triangle, in red.
        assert_eq!(scene.meshes[0].faces.as_ref().unwrap(), &vec![vec![0, 1, 2]]);
        assert_eq!(scene.meshes[0].colors[0].as_ref().unwrap()[0], [1.0, 0.0, 0.0, 1.0]);
        // Two vertices, closed: two segments.
        assert_eq!(scene.meshes[1].faces.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn faults() {
        assert!(read(b"0\nSECTION\n2\nENTITIES\n0\nENDSEC\n0\nEOF\n").is_err());
        assert!(read(b"0\nSECTION\n2\nENTITIES\n0\n3DFACE\n10\nx\n0\nENDSEC\n").is_err());
        assert!(read(b"AutoCAD Binary DXF\r\n\x1a").is_err());
    }
}
