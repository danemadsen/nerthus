//! FBXParser.cpp: the element/scope tree over the tokens, and token and
//! array parsing (ASCII through Assimp's `fast_atof`/`strtol10`, binary
//! records with zlib-compressed arrays).

use std::collections::BTreeMap;

use flate2::{Decompress, FlushDecompress, Status};

use super::tokenizer::{Token, TokenType};
use crate::fast_atof::{fast_atof, strtol10, strtol10_64, strtoul10_64};
use crate::scene::{ImportError, fail};

/// An element: its key token, data tokens and nested scope.
pub struct Element {
    pub key: usize,
    pub tokens: Vec<usize>,
    pub compound: Option<Box<Scope>>,
}

/// A scope: elements by key (a `std::multimap<std::string, Element*>`:
/// sorted by key bytes, insertion order within a key).
#[derive(Default)]
pub struct Scope {
    elements: BTreeMap<Vec<u8>, Vec<Element>>,
}

impl Scope {
    /// `operator[]`: the first element with the key.
    pub fn get(&self, key: &str) -> Option<&Element> {
        self.elements.get(key.as_bytes()).and_then(|list| list.first())
    }

    /// `GetCollection`.
    pub fn collection(&self, key: &str) -> &[Element] {
        self.collection_bytes(key.as_bytes())
    }

    pub fn collection_bytes(&self, key: &[u8]) -> &[Element] {
        self.elements.get(key).map_or(&[], Vec::as_slice)
    }

    pub fn count(&self, key: &str) -> usize {
        self.collection(key).len()
    }

    /// Every element in multimap order.
    pub fn all(&self) -> impl Iterator<Item = (&[u8], &Element)> {
        self.elements.iter().flat_map(|(key, list)| list.iter().map(move |element| (key.as_slice(), element)))
    }
}

/// The tokenized file.
pub struct File {
    pub input: Vec<u8>,
    pub tokens: Vec<Token>,
}

impl File {
    pub fn token(&self, index: usize) -> &Token {
        &self.tokens[index]
    }

    pub fn bytes(&self, index: usize) -> &[u8] {
        let token = &self.tokens[index];
        &self.input[token.start..token.end]
    }
}

struct Parser<'a> {
    file: &'a File,
    cursor: usize,
    current: Option<usize>,
}

impl Parser<'_> {
    fn advance(&mut self) -> Option<usize> {
        if self.cursor == self.file.tokens.len() {
            self.current = None;
        } else {
            self.current = Some(self.cursor);
            self.cursor += 1;
        }
        self.current
    }

    fn kind(&self, index: usize) -> TokenType {
        self.file.tokens[index].kind
    }

    /// `Element::Element`.
    fn element(&mut self, key: usize) -> Result<Element, ImportError> {
        let mut element = Element { key, tokens: Vec::new(), compound: None };
        loop {
            let Some(mut next) = self.advance() else {
                return fail("FBX-Parser: unexpected end of file, expected closing bracket");
            };
            if self.kind(next) == TokenType::Data {
                element.tokens.push(next);
                let previous = next;
                let Some(following) = self.advance() else {
                    return fail("FBX-Parser: unexpected end of file, expected bracket, comma or key");
                };
                next = following;
                let kind = self.kind(next);
                let next_line = self.file.tokens[next].line;
                let previous_line = self.file.tokens[previous].line;
                // Some exporters leave out the comma at line ends.
                if kind == TokenType::Data && next_line == previous_line.wrapping_add(1) {
                    element.tokens.push(next);
                    continue;
                }
                if !matches!(kind, TokenType::OpenBracket | TokenType::CloseBracket | TokenType::Comma | TokenType::Key) {
                    return fail("FBX-Parser: unexpected token; expected bracket, comma or key");
                }
            }
            if self.kind(next) == TokenType::OpenBracket {
                element.compound = Some(Box::new(self.scope(false)?));
                let closing = self.current;
                if closing.map(|index| self.kind(index)) != Some(TokenType::CloseBracket) {
                    return fail("FBX-Parser: expected closing bracket");
                }
                self.advance();
                return Ok(element);
            }
            if matches!(self.kind(next), TokenType::Key | TokenType::CloseBracket) {
                return Ok(element);
            }
        }
    }

    /// `Scope::Scope`.
    fn scope(&mut self, top_level: bool) -> Result<Scope, ImportError> {
        if !top_level {
            let opening = self.current.map(|index| self.kind(index));
            if opening != Some(TokenType::OpenBracket) {
                return fail("FBX-Parser: expected open bracket");
            }
        }
        let mut scope = Scope::default();
        let Some(mut next) = self.advance() else {
            return fail("FBX-Parser: unexpected end of file");
        };
        while self.kind(next) != TokenType::CloseBracket {
            if self.kind(next) != TokenType::Key {
                return fail("FBX-Parser: unexpected token, expected TOK_KEY");
            }
            let key = self.file.bytes(next).to_vec();
            if key.is_empty() {
                return fail("FBX-Parser: unexpected content: empty string.");
            }
            let element = self.element(next)?;
            scope.elements.entry(key).or_default().push(element);
            match self.current {
                Some(index) => next = index,
                None => {
                    if top_level {
                        return Ok(scope);
                    }
                    return fail("FBX-Parser: unexpected end of file");
                }
            }
        }
        Ok(scope)
    }
}

/// `Parser::Parser`: the root scope.
pub fn parse(file: &File) -> Result<Scope, ImportError> {
    let mut parser = Parser { file, cursor: 0, current: None };
    parser.scope(true)
}

fn parse_error<T>(message: &str) -> Result<T, ImportError> {
    fail(format!("FBX-Parser: {message}"))
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    let mut word = [0u8; 4];
    for (index, byte) in word.iter_mut().enumerate() {
        *byte = bytes.get(offset + index).copied().unwrap_or(0);
    }
    f32::from_le_bytes(word)
}

fn read_f64(bytes: &[u8], offset: usize) -> f64 {
    let mut word = [0u8; 8];
    for (index, byte) in word.iter_mut().enumerate() {
        *byte = bytes.get(offset + index).copied().unwrap_or(0);
    }
    f64::from_le_bytes(word)
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    let mut word = [0u8; 4];
    for (index, byte) in word.iter_mut().enumerate() {
        *byte = bytes.get(offset + index).copied().unwrap_or(0);
    }
    u32::from_le_bytes(word)
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    let mut word = [0u8; 8];
    for (index, byte) in word.iter_mut().enumerate() {
        *byte = bytes.get(offset + index).copied().unwrap_or(0);
    }
    u64::from_le_bytes(word)
}

impl File {
    /// `ParseTokenAsID`.
    pub fn id(&self, index: usize) -> Result<u64, ImportError> {
        let token = self.token(index);
        if token.kind != TokenType::Data {
            return parse_error("expected TOK_DATA token");
        }
        let bytes = self.bytes(index);
        if token.binary {
            if bytes.first() != Some(&b'L') {
                return parse_error("failed to parse ID, unexpected data type, expected L(ong) (binary)");
            }
            return Ok(read_u64(bytes, 1));
        }
        let mut length = bytes.len() as u32;
        let (value, end) = strtoul10_64(&self.input, token.start, Some(&mut length))?;
        if end.is_some_and(|end| end > token.end) {
            return parse_error("failed to parse ID (text)");
        }
        Ok(value)
    }

    /// `ParseTokenAsDim`.
    pub fn dimension(&self, index: usize) -> Result<u64, ImportError> {
        let token = self.token(index);
        if token.kind != TokenType::Data {
            return parse_error("expected TOK_DATA token");
        }
        let bytes = self.bytes(index);
        if token.binary {
            if bytes.first() != Some(&b'L') {
                return parse_error("failed to parse ID, unexpected data type, expected L(ong) (binary)");
            }
            return Ok(read_u64(bytes, 1));
        }
        if bytes.first() != Some(&b'*') {
            return parse_error("expected asterisk before array dimension");
        }
        let mut length = bytes.len() as u32;
        if length == 0 {
            return parse_error("expected valid integer number after asterisk");
        }
        let (value, end) = strtoul10_64(&self.input, token.start + 1, Some(&mut length))?;
        if end.is_some_and(|end| end > token.end) {
            return parse_error("failed to parse ID");
        }
        Ok(value)
    }

    /// `ParseTokenAsFloat` (ASCII values over 31 characters read as 0).
    pub fn float(&self, index: usize) -> Result<f32, ImportError> {
        let token = self.token(index);
        if token.kind != TokenType::Data {
            return parse_error("expected TOK_DATA token");
        }
        let bytes = self.bytes(index);
        if token.binary {
            return match bytes.first() {
                Some(b'F') => Ok(read_f32(bytes, 1)),
                Some(b'D') => Ok(read_f64(bytes, 1) as f32),
                _ => parse_error("failed to parse F(loat) or D(ouble), unexpected data type (binary)"),
            };
        }
        if bytes.len() > 31 {
            return Ok(0.0);
        }
        fast_atof(bytes)
    }

    /// `ParseTokenAsInt`.
    pub fn int(&self, index: usize) -> Result<i32, ImportError> {
        let token = self.token(index);
        if token.kind != TokenType::Data {
            return parse_error("expected TOK_DATA token");
        }
        let bytes = self.bytes(index);
        if token.binary {
            if bytes.first() != Some(&b'I') {
                return parse_error("failed to parse I(nt), unexpected data type (binary)");
            }
            return Ok(read_u32(bytes, 1) as i32);
        }
        let (value, end) = strtol10(&self.input, token.start);
        if end != token.end {
            return parse_error("failed to parse ID");
        }
        Ok(value)
    }

    /// `ParseTokenAsInt64`.
    pub fn int64(&self, index: usize) -> Result<i64, ImportError> {
        let token = self.token(index);
        if token.kind != TokenType::Data {
            return parse_error("expected TOK_DATA token");
        }
        let bytes = self.bytes(index);
        if token.binary {
            if bytes.first() != Some(&b'L') {
                return parse_error("failed to parse Int64, unexpected data type");
            }
            return Ok(read_u64(bytes, 1) as i64);
        }
        let mut length = bytes.len() as u32;
        let (value, end) = strtol10_64(&self.input, token.start, Some(&mut length))?;
        if end.is_some_and(|end| end > token.end) {
            return parse_error("failed to parse Int64 (text)");
        }
        Ok(value)
    }

    /// `ParseTokenAsString`.
    pub fn string(&self, index: usize) -> Result<Vec<u8>, ImportError> {
        let token = self.token(index);
        if token.kind != TokenType::Data {
            return parse_error("expected TOK_DATA token");
        }
        let bytes = self.bytes(index);
        if token.binary {
            if bytes.first() != Some(&b'S') {
                return parse_error("failed to parse S(tring), unexpected data type (binary)");
            }
            let length = read_u32(bytes, 1) as usize;
            let end = (5 + length).min(bytes.len());
            return Ok(bytes[5.min(bytes.len())..end].to_vec());
        }
        if bytes.len() < 2 {
            return parse_error("token is too short to hold a string");
        }
        if bytes[0] != b'"' || bytes[bytes.len() - 1] != b'"' {
            return parse_error("expected double quoted string");
        }
        Ok(bytes[1..bytes.len() - 1].to_vec())
    }

    /// `GetRequiredToken`.
    pub fn required_token(&self, element: &Element, index: usize) -> Result<usize, ImportError> {
        match element.tokens.get(index) {
            Some(&token) => Ok(token),
            None => parse_error(&format!("missing token at index {index}")),
        }
    }
}

/// `GetRequiredElement`.
pub fn required_element<'a>(scope: &'a Scope, key: &str) -> Result<&'a Element, ImportError> {
    match scope.get(key) {
        Some(element) => Ok(element),
        None => parse_error(&format!("did not find required element \"{key}\"")),
    }
}

/// `GetRequiredScope`.
pub fn required_scope(element: &Element) -> Result<&Scope, ImportError> {
    match element.compound.as_deref() {
        Some(scope) => Ok(scope),
        None => parse_error("expected compound scope"),
    }
}

/// A binary array record: its type code, count and decoded bytes
/// (`ReadBinaryDataArrayHead` + `ReadBinaryDataArray`), with the per-type
/// checks the callers make in their order.
struct BinaryArray {
    kind: u8,
    count: u32,
    data: Vec<u8>,
}

/// `ReadBinaryDataArrayHead`: the type and count.
fn binary_head(bytes: &[u8]) -> Result<(u8, u32), ImportError> {
    if bytes.len() < 5 {
        return parse_error("binary data array is too short, need five (5) bytes for type signature and element count");
    }
    Ok((bytes[0], read_u32(bytes, 1)))
}

/// `ReadBinaryDataArray`: raw or zlib-inflated (one `Z_FINISH` pass into a
/// zero-filled buffer of the full size).
fn binary_body(bytes: &[u8], kind: u8, count: u32) -> Result<Vec<u8>, ImportError> {
    let encoding = read_u32(bytes, 5);
    let compressed_length = read_u32(bytes, 9) as usize;
    let payload = &bytes[13.min(bytes.len())..];
    let stride = match kind {
        b'f' | b'i' => 4u32,
        b'd' | b'l' => 8,
        _ => 0,
    };
    let full_length = stride.wrapping_mul(count) as usize;
    let mut buffer = vec![0u8; full_length];
    if encoding == 0 {
        let copied = payload.len().min(full_length);
        buffer[..copied].copy_from_slice(&payload[..copied]);
    } else if encoding == 1 && compressed_length != 0 {
        let mut inflater = Decompress::new(true);
        let input = &payload[..compressed_length.min(payload.len())];
        let status = inflater.decompress(input, &mut buffer, FlushDecompress::Finish);
        if !matches!(status, Ok(Status::StreamEnd)) {
            return fail("Compression: Failure decompressing this file using gzip.");
        }
    }
    Ok(buffer)
}

impl File {
    /// The array element's binary record, or `None` for an ASCII array.
    fn binary_array(&self, element: &Element, multiple: u32, accepted: &[u8], negative_check: bool) -> Result<Option<BinaryArray>, ImportError> {
        let Some(&first) = element.tokens.first() else {
            return parse_error("unexpected empty element");
        };
        if !self.token(first).binary {
            return Ok(None);
        }
        let bytes = self.bytes(first);
        let (kind, count) = binary_head(bytes)?;
        if multiple > 1 && count % multiple != 0 {
            return parse_error("number of elements is not a multiple of the tuple size (binary)");
        }
        if count == 0 {
            return Ok(Some(BinaryArray { kind, count, data: Vec::new() }));
        }
        if !accepted.contains(&kind) {
            return parse_error("unexpected array type (binary)");
        }
        let data = binary_body(bytes, kind, count)?;
        let size = if kind == b'd' || kind == b'l' { 8u64 } else { 4 };
        if count as u64 * size != data.len() as u64 {
            return parse_error("Invalid read size (binary)");
        }
        if negative_check {
            let (words, _) = data.as_chunks::<4>();
            for word in words {
                if i32::from_le_bytes(*word) < 0 {
                    return parse_error("encountered negative integer index (binary)");
                }
            }
        }
        Ok(Some(BinaryArray { kind, count, data }))
    }

    /// The ASCII array's values element (`*N { a: … }`).
    fn ascii_values<'a>(&self, element: &'a Element) -> Result<&'a Element, ImportError> {
        self.dimension(element.tokens[0])?;
        let scope = required_scope(element)?;
        required_element(scope, "a")
    }

    fn binary_floats(array: &BinaryArray) -> Vec<f32> {
        let mut values = Vec::with_capacity(array.count as usize);
        for index in 0..array.count as usize {
            let value = if array.kind == b'd' { read_f64(&array.data, index * 8) as f32 } else { read_f32(&array.data, index * 4) };
            values.push(value);
        }
        values
    }

    /// `ParseVectorDataArray` for `aiVector3D`, `aiVector2D` and
    /// `aiColor4D` (`width` floats each).
    pub fn float_tuples(&self, element: &Element, width: usize) -> Result<Vec<Vec<f32>>, ImportError> {
        if let Some(array) = self.binary_array(element, width as u32, b"df", false)? {
            let values = Self::binary_floats(&array);
            return Ok(values.chunks_exact(width).map(<[f32]>::to_vec).collect());
        }
        let values = self.ascii_values(element)?;
        if values.tokens.len() % width != 0 {
            return parse_error("number of floats is not a multiple of the tuple size");
        }
        let mut tuples = Vec::with_capacity(values.tokens.len() / width);
        for group in values.tokens.chunks_exact(width) {
            let mut tuple = Vec::with_capacity(width);
            for &token in group {
                tuple.push(self.float(token)?);
            }
            tuples.push(tuple);
        }
        Ok(tuples)
    }

    pub fn vectors3(&self, element: &Element) -> Result<Vec<[f32; 3]>, ImportError> {
        let tuples = self.float_tuples(element, 3)?;
        Ok(tuples.iter().map(|tuple| [tuple[0], tuple[1], tuple[2]]).collect())
    }

    /// `ParseVectorDataArray` for `float`.
    pub fn floats(&self, element: &Element) -> Result<Vec<f32>, ImportError> {
        if let Some(array) = self.binary_array(element, 1, b"df", false)? {
            return Ok(Self::binary_floats(&array));
        }
        let values = self.ascii_values(element)?;
        values.tokens.iter().map(|&token| self.float(token)).collect()
    }

    /// `ParseVectorDataArray` for `int`.
    pub fn ints(&self, element: &Element) -> Result<Vec<i32>, ImportError> {
        if let Some(array) = self.binary_array(element, 1, b"i", false)? {
            let (words, _) = array.data.as_chunks::<4>();
            return Ok(words.iter().map(|word| i32::from_le_bytes(*word)).collect());
        }
        let values = self.ascii_values(element)?;
        values.tokens.iter().map(|&token| self.int(token)).collect()
    }

    /// `ParseVectorDataArray` for `unsigned int` (negatives fail).
    pub fn uints(&self, element: &Element) -> Result<Vec<u32>, ImportError> {
        if let Some(array) = self.binary_array(element, 1, b"i", true)? {
            let (words, _) = array.data.as_chunks::<4>();
            return Ok(words.iter().map(|word| u32::from_le_bytes(*word)).collect());
        }
        let values = self.ascii_values(element)?;
        let mut output = Vec::with_capacity(values.tokens.len());
        for &token in &values.tokens {
            let value = self.int(token)?;
            if value < 0 {
                return parse_error("encountered negative integer index");
            }
            output.push(value as u32);
        }
        Ok(output)
    }
}
