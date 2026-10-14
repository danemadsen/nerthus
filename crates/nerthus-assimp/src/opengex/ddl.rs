//! The OpenDDL parser OpenGEX files are read with (`ODDLParser` 0.4.0):
//! a tree of structures, each with a type, at most one property, and the
//! values, references or data array list of its primitive children.
//!
//! The parser first rewrites its copy of the text without comments and
//! line breaks. The vector keeps its storage, so the file's own last bytes
//! stay after the new end, the last of them the NUL the text was given.
//! Many loops read the byte at the end, or run on past it, before they
//! compare positions: they see those stale bytes, and a read past the
//! storage is undefined behaviour, an error here.

use crate::parsing::{atoi, strtod};
use crate::scene::{ImportError, fail};

/// Structures nested deeper than this overflow the parser's recursion.
pub(super) const MAX_DEPTH: usize = 1024;

/// The root structure's index.
pub(super) const ROOT: usize = 0;

/// `Value::ValueType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ValueKind {
    Bool,
    Int8,
    Int16,
    Int32,
    Int64,
    UnsignedInt8,
    UnsignedInt16,
    UnsignedInt32,
    UnsignedInt64,
    Half,
    Float,
    Double,
    String,
    Ref,
}

/// The primitive type names, in the order they are tried (as prefixes).
const TYPE_NAMES: [(&[u8], ValueKind); 14] = [
    (b"bool", ValueKind::Bool),
    (b"int8", ValueKind::Int8),
    (b"int16", ValueKind::Int16),
    (b"int32", ValueKind::Int32),
    (b"int64", ValueKind::Int64),
    (b"unsigned_int8", ValueKind::UnsignedInt8),
    (b"unsigned_int16", ValueKind::UnsignedInt16),
    (b"unsigned_int32", ValueKind::UnsignedInt32),
    (b"unsigned_int64", ValueKind::UnsignedInt64),
    (b"half", ValueKind::Half),
    (b"float", ValueKind::Float),
    (b"double", ValueKind::Double),
    (b"string", ValueKind::String),
    (b"ref", ValueKind::Ref),
];

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// A `Value`: its type and its data bytes (`m_data`, `m_size` long).
pub(super) struct Value {
    pub(super) kind: ValueKind,
    data: Vec<u8>,
}

impl Value {
    /// `getFloat`: the first four bytes as a float.
    pub(super) fn float(&self) -> Result<f32, ImportError> {
        let Some(bytes) = self.data.first_chunk::<4>() else {
            return undefined("a value shorter than a float read as one");
        };
        Ok(f32::from_le_bytes(*bytes))
    }

    /// `getUnsignedInt32`: the value's bytes copied over a four-byte
    /// local, which a value of another size leaves part uninitialised or
    /// writes past.
    pub(super) fn unsigned_int32(&self) -> Result<u32, ImportError> {
        let Ok(bytes) = <[u8; 4]>::try_from(self.data.as_slice()) else {
            return undefined("a value not four bytes long read as an unsigned_int32");
        };
        Ok(u32::from_le_bytes(bytes))
    }

    /// `getString` read as a C string: the bytes before a NUL, which the
    /// value must hold.
    pub(super) fn c_string(&self) -> Result<&[u8], ImportError> {
        let Some(length) = self.data.iter().position(|&byte| byte == 0) else {
            return undefined("a value without a NUL read as a string");
        };
        Ok(&self.data[..length])
    }

    /// The data bytes, for comparisons that stop at the first difference.
    pub(super) fn bytes(&self) -> &[u8] {
        &self.data
    }
}

/// A `Text`: `None` for an empty one (its buffer is null), otherwise the
/// bytes `strncpy` copied, which stop at a NUL.
pub(super) type Text = Option<Vec<u8>>;

fn make_text(bytes: &[u8]) -> Text {
    if bytes.is_empty() {
        return None;
    }
    let length = bytes.iter().position(|&byte| byte == 0).unwrap_or(bytes.len());
    Some(bytes[..length].to_vec())
}

/// A `Property`: its key, and its value (none for a reference).
pub(super) struct Property {
    pub(super) key: Text,
    pub(super) value: Option<Value>,
}

/// One list of a `DataArrayList` (its values; none for references).
pub(super) struct DataList {
    pub(super) values: Vec<Value>,
}

/// A `DDLNode`. Empty lists stand for null pointers.
pub(super) struct Node {
    /// The type (a C string).
    pub(super) kind: Vec<u8>,
    pub(super) parent: Option<usize>,
    pub(super) children: Vec<usize>,
    pub(super) properties: Vec<Property>,
    /// The last value list given to the structure.
    pub(super) values: Vec<Value>,
    /// The last references given to it: each name's text.
    pub(super) references: Vec<Text>,
    /// The last data array list given to it.
    pub(super) data_lists: Vec<DataList>,
}

fn is_space(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

fn is_new_line(byte: u8) -> bool {
    byte == b'\n' || byte == b'\r'
}

fn is_separator(byte: u8) -> bool {
    is_space(byte) || matches!(byte, b',' | b'{' | b'}' | b'[' | b'(' | b')')
}

/// `isNotEndOfToken` without its position check.
fn ends_token(byte: u8) -> bool {
    matches!(byte, b'}' | b',' | b')') || is_space(byte)
}

/// `hex2Decimal`: 'a' to 'p' (any case) count from 10; other bytes give
/// 9999999.
fn hex_to_decimal(byte: u8) -> i32 {
    if byte.is_ascii_digit() {
        return (byte - b'0') as i32;
    }
    for offset in 0..16u8 {
        if byte == b'a' + offset || byte == b'A' + offset {
            return offset as i32 + 10;
        }
    }
    9_999_999
}

/// `atoll` and `strtoull` of the digits at `start` (both clamp).
fn decimal_values(bytes: &[u8], start: usize) -> (i64, u64) {
    let mut value = 0u128;
    let mut index = start;
    while let Some(&byte) = bytes.get(index) {
        if !byte.is_ascii_digit() {
            break;
        }
        let digit = (byte - b'0') as u128;
        value = (value * 10 + digit).min(u64::MAX as u128 + 1);
        index += 1;
    }
    let signed = value.min(i64::MAX as u128) as i64;
    let unsigned = value.min(u64::MAX as u128) as u64;
    (signed, unsigned)
}

/// `isCommentOpenTag` and the others over the original buffer, whose last
/// byte is a NUL: the bytes they look at past the first lie inside it.
fn byte_at(buffer: &[u8], index: usize) -> u8 {
    buffer.get(index).copied().unwrap_or(0)
}

fn is_comment_open(buffer: &[u8], index: usize) -> bool {
    buffer[index] == b'/' && index + 1 != buffer.len() && byte_at(buffer, index + 1) == b'*'
}

fn is_comment_close(buffer: &[u8], index: usize) -> bool {
    buffer[index] == b'*' && index + 1 != buffer.len() && byte_at(buffer, index + 1) == b'/'
}

/// `isComment`: "//", unless a letter and a '/' follow (a drive).
fn is_comment(buffer: &[u8], index: usize) -> bool {
    if buffer[index] != b'/' || index + 1 == buffer.len() || byte_at(buffer, index + 1) != b'/' {
        return false;
    }
    let drive = byte_at(buffer, index + 2);
    !(drive.is_ascii_alphabetic() && byte_at(buffer, index + 3) == b'/')
}

/// `normalizeBuffer`: the text without comments ("/*" closes at the first
/// "*/" from its own '*') and line breaks, over the start of the buffer.
fn normalize(buffer: &[u8]) -> (Vec<u8>, usize) {
    let length = buffer.len();
    let mut normalized = Vec::with_capacity(length);
    let mut index = 0;
    while index < length {
        if is_comment_open(buffer, index) {
            index += 1;
            while index < length && !is_comment_close(buffer, index) {
                index += 1;
            }
            index += 1;
        } else if !is_comment(buffer, index) && !is_new_line(buffer[index]) {
            normalized.push(buffer[index]);
        } else if is_comment(buffer, index) {
            index += 1;
            while index < length && buffer[index] != b'\n' {
                index += 1;
            }
        }
        index += 1;
    }
    let end = normalized.len();
    let mut memory = buffer.to_vec();
    memory[..end].copy_from_slice(&normalized);
    (memory, end)
}

/// `OpenDDLParser` over the normalized buffer. Positions are indices; the
/// C++ null position is `None`.
struct Parser {
    memory: Vec<u8>,
    end: usize,
    nodes: Vec<Node>,
    stack: Vec<usize>,
}

/// What `parse_data_list` read: the values and the last references.
struct DataListRead {
    values: Vec<Value>,
    references: Vec<Text>,
}

impl Parser {
    fn byte(&self, position: usize) -> Result<u8, ImportError> {
        match self.memory.get(position) {
            Some(&byte) => Ok(byte),
            None => undefined("a read past the parser's buffer"),
        }
    }

    fn top(&self) -> Option<usize> {
        self.stack.last().copied()
    }

    /// `DDLNode::create`: a node added to its parent's children.
    fn create_node(&mut self, kind: Vec<u8>, parent: Option<usize>) -> usize {
        let index = self.nodes.len();
        let node = Node {
            kind,
            parent,
            children: Vec::new(),
            properties: Vec::new(),
            values: Vec::new(),
            references: Vec::new(),
            data_lists: Vec::new(),
        };
        self.nodes.push(node);
        if let Some(parent) = parent {
            self.nodes[parent].children.push(index);
        }
        index
    }

    /// `logInvalidTokenError`: the text from `position` copied as a C
    /// string (it stops at the buffer's final NUL at the latest).
    fn log_invalid_token(&self, position: usize) -> Result<(), ImportError> {
        self.byte(position)?;
        Ok(())
    }

    /// `validate`: an empty text, or one starting with a letter or digit.
    fn validate(&self) -> bool {
        if self.end == 0 {
            return true;
        }
        let first = self.memory[0];
        first.is_ascii_alphabetic() || first.is_ascii_digit()
    }

    /// `lookForNextToken`: past blanks, line breaks and commas.
    fn look_for_next_token(&self, mut position: usize) -> Result<usize, ImportError> {
        while position != self.end {
            let byte = self.byte(position)?;
            if !(is_space(byte) || is_new_line(byte) || byte == b',') {
                break;
            }
            position += 1;
        }
        Ok(position)
    }

    /// `getNextToken`: the next token, at least one byte on.
    fn next_token(&self, position: usize) -> Result<usize, ImportError> {
        let next = self.look_for_next_token(position)?;
        if next == position {
            return Ok(position + 1);
        }
        Ok(next)
    }

    /// `getNextSeparator`.
    fn next_separator(&self, mut position: usize) -> Result<usize, ImportError> {
        while position != self.end && !is_separator(self.byte(position)?) {
            position += 1;
        }
        Ok(position)
    }

    /// Whether the bytes at `position` spell `text` (`strncmp`, which stops
    /// at the first difference).
    fn matches_at(&self, position: usize, text: &[u8]) -> Result<bool, ImportError> {
        for (offset, &expected) in text.iter().enumerate() {
            if self.byte(position + offset)? != expected {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// `parseNextNode`: a header, then its structure.
    fn parse_next_node(&mut self, position: Option<usize>, depth: usize) -> Result<Option<usize>, ImportError> {
        if depth > MAX_DEPTH {
            return undefined("structures nested too deep for the stack");
        }
        let after_header = self.parse_header(position)?;
        self.parse_structure(after_header, depth)
    }

    /// `parseHeader`: the type (a node under the stack's top, pushed; none
    /// for an empty type), its name, and a property list. Only a property
    /// followed by ')' is kept: commas are passed over as blanks, so a
    /// second property is an error.
    fn parse_header(&mut self, position: Option<usize>) -> Result<Option<usize>, ImportError> {
        let Some(position) = position else {
            return Ok(None);
        };
        if position == self.end {
            return Ok(Some(position));
        }
        let (after_identifier, identifier) = self.parse_identifier(position)?;
        let mut position = self.look_for_next_token(after_identifier)?;
        let Some(text) = identifier else {
            return Ok(Some(position));
        };
        let mut node = None;
        if let Some(kind) = text {
            let parent = self.top();
            let created = self.create_node(kind, parent);
            self.stack.push(created);
            node = Some(created);
        }

        // (A name only names the node, which nothing here reads.)
        let (after_name, _) = self.parse_name(position)?;
        position = self.look_for_next_token(after_name)?;
        let mut properties = Vec::new();
        if position != self.end && self.byte(position)? == b'(' {
            position += 1;
            while position != self.end && self.byte(position)? != b')' {
                let (after_property, property) = self.parse_property(position)?;
                position = self.look_for_next_token(after_property)?;
                if position == self.end {
                    break;
                }
                let byte = self.byte(position)?;
                if byte != b',' && byte != b')' {
                    self.log_invalid_token(position)?;
                    return Ok(None);
                }
                if let Some(property) = property
                    && byte != b','
                {
                    properties.push(property);
                }
            }
            if position != self.end {
                position += 1;
            }
        }
        if let Some(node) = node
            && !properties.is_empty()
        {
            self.nodes[node].properties = properties;
        }
        Ok(Some(position))
    }

    /// `parseStructure`: '{', bodies up to '}', then the stack's top popped
    /// (also when the text ended before any '{').
    fn parse_structure(&mut self, position: Option<usize>, depth: usize) -> Result<Option<usize>, ImportError> {
        let Some(position) = position else {
            return Ok(None);
        };
        if position == self.end {
            return Ok(Some(position));
        }
        let mut error = false;
        let mut position = self.look_for_next_token(position)?;
        if position != self.end {
            if self.byte(position)? != b'{' {
                position += 1;
                self.log_invalid_token(position)?;
                return Ok(None);
            }
            loop {
                let Some(next) = self.parse_structure_body(position, &mut error, depth)? else {
                    return Ok(None);
                };
                position = next;
                if position == self.end || self.byte(position)? == b'}' {
                    break;
                }
            }
            if position != self.end {
                position += 1;
            }
        }
        position = self.look_for_next_token(position)?;
        if !error {
            self.stack.pop();
        }
        Ok(Some(position))
    }

    /// `parseStructureBody`: a primitive structure (its data given to the
    /// stack's top), ending at its '}', or a nested structure.
    fn parse_structure_body(&mut self, position: usize, error: &mut bool, depth: usize) -> Result<Option<usize>, ImportError> {
        let mut position = position;
        let first = self.byte(position)?;
        if !first.is_ascii_digit() && !first.is_ascii_alphabetic() {
            position += 1;
        }
        position = self.look_for_next_token(position)?;
        let (after_type, kind, array_length) = self.parse_primitive_data_type(position)?;
        position = after_type;
        let Some(kind) = kind else {
            return self.parse_next_node(Some(position), depth + 1);
        };

        position = self.look_for_next_token(position)?;
        if self.byte(position)? == b'{' {
            if array_length == 1 {
                let (after_list, read) = self.parse_data_list(position, kind)?;
                position = after_list;
                if let Some(top) = self.top() {
                    if !read.values.is_empty() {
                        self.nodes[top].values = read.values;
                    }
                    if !read.references.is_empty() {
                        self.nodes[top].references = read.references;
                    }
                }
            } else if array_length > 1 {
                let (after_lists, lists) = self.parse_data_array_list(position, kind)?;
                position = after_lists;
                if let Some(top) = self.top()
                    && !lists.is_empty()
                {
                    self.nodes[top].data_lists = lists;
                }
            } else {
                *error = true;
            }
        }
        position = self.look_for_next_token(position)?;
        if position == self.end {
            return Ok(None);
        }
        if self.byte(position)? != b'}' {
            self.log_invalid_token(position)?;
            return Ok(None);
        }
        Ok(Some(position))
    }

    /// `parseIdentifier`: bytes up to a separator, line break, parenthesis
    /// or '$'. None at the end or before a digit.
    fn parse_identifier(&self, position: usize) -> Result<(usize, Option<Text>), ImportError> {
        if position == self.end {
            return Ok((position, None));
        }
        let mut position = self.look_for_next_token(position)?;
        if position == self.end {
            return Ok((position, None));
        }
        if self.byte(position)?.is_ascii_digit() {
            return Ok((position, None));
        }
        let start = position;
        while position != self.end {
            let byte = self.byte(position)?;
            if is_separator(byte) || is_new_line(byte) || byte == b'$' {
                break;
            }
            position += 1;
        }
        let text = make_text(&self.memory[start..position]);
        Ok((position, Some(text)))
    }

    /// `parseName`: '$' or '%', then an identifier.
    fn parse_name(&self, position: usize) -> Result<(usize, Option<Text>), ImportError> {
        if position == self.end {
            return Ok((position, None));
        }
        let position = self.look_for_next_token(position)?;
        let marker = self.byte(position)?;
        if marker != b'$' && marker != b'%' {
            return Ok((position, None));
        }
        self.parse_identifier(position + 1)
    }

    /// `parsePrimitiveDataType`: a type name as a prefix, then "[length]"
    /// (from the byte after '[' up to the next ']', read by `atoi`), else a
    /// length of 1.
    fn parse_primitive_data_type(&self, position: usize) -> Result<(usize, Option<ValueKind>, u64), ImportError> {
        if position == self.end {
            return Ok((position, None, 0));
        }
        let mut found = None;
        for (name, kind) in TYPE_NAMES {
            if self.matches_at(position, name)? {
                found = Some((name.len(), kind));
                break;
            }
        }
        let Some((name_length, kind)) = found else {
            let next = self.look_for_next_token(position)?;
            return Ok((next, None, 0));
        };

        let mut position = position + name_length;
        if self.byte(position)? != b'[' {
            return Ok((position, Some(kind), 1));
        }
        position += 1;
        let start = position;
        while position != self.end {
            position += 1;
            if self.byte(position)? == b']' {
                // (`size_t` from an `int`.)
                let array_length = atoi(&self.memory, start) as i64 as u64;
                return Ok((position + 1, Some(kind), array_length));
            }
        }
        Ok((position, None, 0))
    }

    /// `parseReference`: names separated by commas. A comma left at the end
    /// is read again and again: the loop never ends.
    fn parse_reference(&self, position: usize) -> Result<(usize, Vec<Text>), ImportError> {
        let mut names = Vec::new();
        if position == self.end {
            return Ok((position, names));
        }
        let (mut position, name) = self.parse_name(position)?;
        if let Some(name) = name {
            names.push(name);
        }
        while self.byte(position)? == b',' {
            if position == self.end {
                return fail("an endless loop over a reference list");
            }
            position = self.next_separator(position)?;
            if self.byte(position)? != b',' {
                break;
            }
            let (after_name, name) = self.parse_name(position)?;
            position = after_name;
            if let Some(name) = name {
                names.push(name);
            }
        }
        Ok((position, names))
    }

    /// `parseIntegerLiteral`: a token starting with a digit, read by
    /// `atoll` (signed types) or `strtoull` (unsigned ones) and cut to the
    /// type's size. A sign gives no value.
    fn parse_integer_literal(&self, position: usize, kind: ValueKind) -> Result<(usize, Option<Value>), ImportError> {
        if position == self.end {
            return Ok((position, None));
        }
        let start = self.look_for_next_token(position)?;
        let mut position = start;
        loop {
            let byte = self.byte(position)?;
            if is_separator(byte) || position == self.end {
                break;
            }
            position += 1;
        }
        if !self.byte(start)?.is_ascii_digit() {
            return Ok((position, None));
        }
        let (signed, unsigned) = decimal_values(&self.memory, start);
        let data = match kind {
            ValueKind::Int8 => (signed as i8).to_le_bytes().to_vec(),
            ValueKind::Int16 => (signed as i16).to_le_bytes().to_vec(),
            ValueKind::Int32 => (signed as i32).to_le_bytes().to_vec(),
            ValueKind::Int64 => signed.to_le_bytes().to_vec(),
            ValueKind::UnsignedInt8 => (unsigned as u8).to_le_bytes().to_vec(),
            ValueKind::UnsignedInt16 => (unsigned as u16).to_le_bytes().to_vec(),
            ValueKind::UnsignedInt32 => (unsigned as u32).to_le_bytes().to_vec(),
            ValueKind::UnsignedInt64 => unsigned.to_le_bytes().to_vec(),
            _ => return Ok((position, None)),
        };
        Ok((position, Some(Value { kind, data })))
    }

    /// `parseFloatingLiteral`: a "0x" token as `parseHexaLiteral` reads
    /// it, else one starting with a digit (or '-' and one) read by `atof`
    /// (as a float unless the type is double; halves are floats).
    fn parse_floating_literal(&self, position: usize, kind: ValueKind) -> Result<(usize, Option<Value>), ImportError> {
        if position == self.end {
            return Ok((position, None));
        }
        let start = self.look_for_next_token(position)?;
        let mut position = start;
        while position != self.end && !is_separator(self.byte(position)?) {
            position += 1;
        }
        if self.is_hex_literal(start)? {
            let (_, value) = self.parse_hexa_literal(start)?;
            return Ok((position, value));
        }

        let first = self.byte(start)?;
        let readable = if first.is_ascii_digit() {
            true
        } else if first == b'-' {
            self.byte(start + 1)?.is_ascii_digit()
        } else {
            false
        };
        if !readable {
            return Ok((position, None));
        }
        let number = strtod(&self.memory[start..]);
        let value = if kind == ValueKind::Double {
            let data = number.to_le_bytes().to_vec();
            Value { kind: ValueKind::Double, data }
        } else {
            let data = (number as f32).to_le_bytes().to_vec();
            Value { kind: ValueKind::Float, data }
        };
        Ok((position, Some(value)))
    }

    /// `parseStringLiteral`: '"' up to the next '"' (or the end), copied by
    /// `strncpy` (so up to a NUL) into a buffer one longer.
    fn parse_string_literal(&self, position: usize) -> Result<(usize, Option<Value>), ImportError> {
        if position == self.end {
            return Ok((position, None));
        }
        let mut position = self.look_for_next_token(position)?;
        if self.byte(position)? != b'"' {
            return Ok((position, None));
        }
        position += 1;
        let start = position;
        loop {
            let byte = self.byte(position)?;
            if byte == b'"' || position == self.end {
                break;
            }
            position += 1;
        }
        let text = &self.memory[start..position];
        let copied = text.iter().position(|&byte| byte == 0).unwrap_or(text.len());
        let mut data = vec![0u8; text.len() + 1];
        data[..copied].copy_from_slice(&text[..copied]);
        let value = Value { kind: ValueKind::String, data };
        Ok((position + 1, Some(value)))
    }

    /// `isHexLiteral`.
    fn is_hex_literal(&self, position: usize) -> Result<bool, ImportError> {
        if self.byte(position)? != b'0' || position + 1 == self.end {
            return Ok(false);
        }
        let marker = self.byte(position + 1)?;
        Ok(marker == b'x' || marker == b'X')
    }

    /// `parseHexaLiteral`: the digits after "0x" up to a separator shifted
    /// into an `int` (wrapping; see `hex_to_decimal`), stored sign-extended
    /// as an `unsigned_int64`.
    fn parse_hexa_literal(&self, position: usize) -> Result<(usize, Option<Value>), ImportError> {
        if position == self.end {
            return Ok((position, None));
        }
        let mut position = self.look_for_next_token(position)?;
        if self.byte(position)? != b'0' {
            return Ok((position, None));
        }
        position += 1;
        let marker = self.byte(position)?;
        if marker != b'x' && marker != b'X' {
            return Ok((position, None));
        }
        position += 1;
        let start = position;
        loop {
            let byte = self.byte(position)?;
            if is_separator(byte) || position == self.end {
                break;
            }
            position += 1;
        }
        let mut value = 0i32;
        for &byte in &self.memory[start..position] {
            let digit = hex_to_decimal(byte);
            value = ((value as u32) << 4) as i32 | digit;
        }
        let data = (value as i64 as u64).to_le_bytes().to_vec();
        let value = Value { kind: ValueKind::UnsignedInt64, data };
        Ok((position, Some(value)))
    }

    /// `isInteger`: an optional '-', then digits up to the token's end.
    fn is_integer(&self, position: usize) -> Result<bool, ImportError> {
        let mut position = position;
        if position != self.end && self.byte(position)? == b'-' {
            position += 1;
        }
        let mut result = false;
        loop {
            let byte = self.byte(position)?;
            if ends_token(byte) || position == self.end {
                break;
            }
            result = byte.is_ascii_digit();
            if !result {
                break;
            }
            position += 1;
        }
        Ok(result)
    }

    /// `isFloat`: an optional '-', digits, '.', then digits up to the
    /// token's end.
    fn is_float(&self, position: usize) -> Result<bool, ImportError> {
        let mut position = position;
        if position != self.end && self.byte(position)? == b'-' {
            position += 1;
        }
        let mut result = false;
        loop {
            let byte = self.byte(position)?;
            if ends_token(byte) || position == self.end {
                break;
            }
            if byte == b'.' {
                result = true;
                break;
            }
            result = byte.is_ascii_digit();
            if !result {
                return Ok(false);
            }
            position += 1;
        }
        if self.byte(position)? != b'.' {
            return Ok(false);
        }
        position += 1;
        loop {
            let byte = self.byte(position)?;
            if ends_token(byte) || position == self.end {
                break;
            }
            result = byte.is_ascii_digit();
            if !result {
                return Ok(false);
            }
            position += 1;
        }
        Ok(result)
    }

    /// `parseProperty`: "key = value", the value an integer, float, string
    /// or references (a missing blank after '=' skips the value's first
    /// byte).
    fn parse_property(&self, position: usize) -> Result<(usize, Option<Property>), ImportError> {
        if position == self.end {
            return Ok((position, None));
        }
        let position = self.look_for_next_token(position)?;
        let (after_key, key) = self.parse_identifier(position)?;
        let Some(key) = key else {
            return Ok((after_key, None));
        };
        let position = self.look_for_next_token(after_key)?;
        if position == self.end || self.byte(position)? != b'=' {
            return Ok((position, None));
        }
        let position = self.next_token(position + 1)?;

        let read = if self.is_integer(position)? {
            self.parse_integer_literal(position, ValueKind::Int32)?
        } else if self.is_float(position)? {
            self.parse_floating_literal(position, ValueKind::Float)?
        } else if self.byte(position)? == b'"' {
            self.parse_string_literal(position)?
        } else {
            let (after_names, names) = self.parse_reference(position)?;
            let property = if names.is_empty() { None } else { Some(Property { key, value: None }) };
            return Ok((after_names, property));
        };
        let (after_value, value) = read;
        let property = value.map(|value| Property { key, value: Some(value) });
        Ok((after_value, property))
    }

    /// `parseDataList`: '{', values (or references) up to '}'. A value
    /// that is followed by another separator than a comma, '}' or a blank
    /// ends the list there.
    fn parse_data_list(&self, position: usize, kind: ValueKind) -> Result<(usize, DataListRead), ImportError> {
        let mut read = DataListRead { values: Vec::new(), references: Vec::new() };
        if position == self.end {
            return Ok((position, read));
        }
        let mut position = self.look_for_next_token(position)?;
        if position == self.end || self.byte(position)? != b'{' {
            return Ok((position, read));
        }
        position += 1;
        while position != self.end && self.byte(position)? != b'}' {
            position = self.look_for_next_token(position)?;
            let mut current = None;
            match kind {
                ValueKind::Ref => {
                    let (after_names, names) = self.parse_reference(position)?;
                    position = after_names;
                    if !names.is_empty() {
                        read.references = names;
                    }
                }
                ValueKind::Int8
                | ValueKind::Int16
                | ValueKind::Int32
                | ValueKind::Int64
                | ValueKind::UnsignedInt8
                | ValueKind::UnsignedInt16
                | ValueKind::UnsignedInt32
                | ValueKind::UnsignedInt64 => {
                    (position, current) = self.parse_integer_literal(position, kind)?;
                }
                ValueKind::Half | ValueKind::Float | ValueKind::Double => {
                    (position, current) = self.parse_floating_literal(position, kind)?;
                }
                ValueKind::String => {
                    (position, current) = self.parse_string_literal(position)?;
                }
                ValueKind::Bool => {}
            }
            if let Some(value) = current {
                read.values.push(value);
            }
            position = self.next_separator(position)?;
            if position == self.end {
                break;
            }
            let byte = self.byte(position)?;
            if byte != b',' && byte != b'}' && !is_space(byte) {
                break;
            }
        }
        if position != self.end {
            position += 1;
        }
        Ok((position, read))
    }

    /// `parseDataArrayList`: '{', data lists separated by commas, then past
    /// the next token. Lists without values or references are dropped.
    fn parse_data_array_list(&self, position: usize, kind: ValueKind) -> Result<(usize, Vec<DataList>), ImportError> {
        let mut lists = Vec::new();
        if position == self.end {
            return Ok((position, lists));
        }
        let mut position = self.look_for_next_token(position)?;
        if self.byte(position)? != b'{' {
            return Ok((position, lists));
        }
        position += 1;
        loop {
            let (after_list, read) = self.parse_data_list(position, kind)?;
            position = after_list;
            if !read.values.is_empty() || !read.references.is_empty() {
                lists.push(DataList { values: read.values });
            }
            if self.byte(position)? != b',' || position == self.end {
                break;
            }
        }
        position = self.look_for_next_token(position)?;
        Ok((position + 1, lists))
    }
}

/// `OpenDDLParser::parse` over the text (with its final NUL): the nodes,
/// the root first, or `None` when the parse fails.
pub(super) fn parse(buffer: &[u8]) -> Result<Option<Vec<Node>>, ImportError> {
    let (memory, end) = normalize(buffer);
    let mut parser = Parser { memory, end, nodes: Vec::new(), stack: Vec::new() };
    if !parser.validate() {
        return Ok(None);
    }
    let root = parser.create_node(b"root".to_vec(), None);
    parser.stack.push(root);

    let mut position = 0;
    while position < parser.end {
        let Some(next) = parser.parse_next_node(Some(position), 0)? else {
            return Ok(None);
        };
        position = next;
    }
    Ok(Some(parser.nodes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(source: &str) -> Vec<u8> {
        let mut buffer = source.as_bytes().to_vec();
        buffer.push(0);
        buffer
    }

    #[test]
    fn comments_and_line_breaks_go_but_the_storage_keeps_the_tail() {
        let buffer = text("a // note\nb /* x */c\n");
        let (memory, end) = normalize(&buffer);
        assert_eq!(&memory[..end], b"a b c\0");
        assert_eq!(memory.len(), buffer.len());
        assert_eq!(&memory[end..], &buffer[end..]);
    }

    #[test]
    fn structures_values_and_lists() {
        let source = "Metric (key = \"up\") {string {\"z\"}}\nGeometryObject $g\n{\n\tMesh\n\t{\n\t\tfloat[3] {{1, 2.5, 0x3F800000}, {-1.0, 0, 0}}\n\t}\n}\n";
        let nodes = parse(&text(source)).unwrap().unwrap();
        let root = &nodes[ROOT];
        // The text's final NUL reads as a header of an empty type.
        assert_eq!(root.children.len(), 3);
        assert_eq!(nodes[root.children[2]].kind, b"");

        let metric = &nodes[root.children[0]];
        assert_eq!(metric.kind, b"Metric");
        assert_eq!(metric.properties.len(), 1);
        assert_eq!(metric.properties[0].key.as_deref(), Some(&b"key"[..]));
        assert_eq!(metric.properties[0].value.as_ref().unwrap().c_string().unwrap(), b"up");
        assert_eq!(metric.values[0].c_string().unwrap(), b"z");

        let object = &nodes[root.children[1]];
        let mesh = &nodes[object.children[0]];
        assert_eq!(mesh.kind, b"Mesh");
        assert_eq!(mesh.data_lists.len(), 2);
        let first = &mesh.data_lists[0].values;
        assert_eq!(first[0].float().unwrap(), 1.0);
        assert_eq!(first[1].float().unwrap(), 2.5);
        assert_eq!(first[2].kind, ValueKind::UnsignedInt64);
        assert_eq!(first[2].float().unwrap(), 1.0);
        assert_eq!(mesh.data_lists[1].values[0].float().unwrap(), -1.0);
    }

    #[test]
    fn two_properties_fail_the_parse() {
        let source = "Mesh (primitive = \"triangles\", lod = 1) {}\n";
        assert!(parse(&text(source)).unwrap().is_none());
    }

    #[test]
    fn an_empty_structure_fails_the_parse() {
        // Its '}' is read as a header without a type, then as a structure.
        assert!(parse(&text("Mesh {}\n")).unwrap().is_none());
    }

    #[test]
    fn a_value_glued_to_the_equals_sign_loses_its_first_byte() {
        // "lod =12" reads "2": parse_integer_literal starts one byte on.
        let source = "Mesh (lod =12) {float {1}}\n";
        let nodes = parse(&text(source)).unwrap().unwrap();
        let mesh = &nodes[nodes[ROOT].children[0]];
        let value = mesh.properties[0].value.as_ref().unwrap();
        assert_eq!(value.bytes(), &2i32.to_le_bytes());
    }

    #[test]
    fn small_values_are_not_floats() {
        let source = "Param {unsigned_int8 {7}}\n";
        let nodes = parse(&text(source)).unwrap().unwrap();
        let param = &nodes[nodes[ROOT].children[0]];
        assert!(param.values[0].float().is_err());
        assert!(param.values[0].unsigned_int32().is_err());
    }
}
