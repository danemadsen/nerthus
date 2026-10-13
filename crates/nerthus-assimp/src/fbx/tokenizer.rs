//! FBXTokenizer.cpp and FBXBinaryTokenizer.cpp: the token stream of an
//! ASCII or binary FBX file.

use crate::scene::{ImportError, fail};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenType {
    OpenBracket,
    CloseBracket,
    Data,
    Comma,
    Key,
}

/// A token: a byte range of the file. Binary tokens keep their data
/// record (type code first); `line` is the line (ASCII) or the offset
/// (binary).
#[derive(Clone, Copy, Debug)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: TokenType,
    pub line: u64,
    pub binary: bool,
}

/// `IsSpaceOrNewLine` (NUL and form feed count as line ends).
fn is_space_or_newline(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | 0 | 0x0c)
}

fn is_line_end(byte: u8) -> bool {
    matches!(byte, b'\r' | b'\n' | 0 | 0x0c)
}

/// `ProcessDataToken`: the pending token (no whitespace outside quotes,
/// quotes closed), if any.
fn process_data_token(tokens: &mut Vec<Token>, input: &[u8], begin: &mut Option<usize>, end: &mut Option<usize>, line: u64, kind: TokenType, must_have: bool) -> Result<(), ImportError> {
    match (*begin, *end) {
        (Some(start), Some(last)) => {
            let mut in_quotes = false;
            for &byte in &input[start..=last] {
                if byte == b'"' {
                    in_quotes = !in_quotes;
                }
                if !in_quotes && is_space_or_newline(byte) {
                    return fail("FBX-Tokenize: unexpected whitespace in token");
                }
            }
            if in_quotes {
                return fail("FBX-Tokenize: non-terminated double quotes");
            }
            tokens.push(Token { start, end: last + 1, kind, line, binary: false });
        }
        _ => {
            if must_have {
                return fail("FBX-Tokenize: unexpected character, expected data token");
            }
        }
    }
    *begin = None;
    *end = None;
    Ok(())
}

/// `Tokenize`: the text up to its first NUL. (A key's colon must follow it
/// directly: Assimp's lookahead for "Key :" never reaches the colon.)
pub fn tokenize_ascii(input: &[u8]) -> Result<Vec<Token>, ImportError> {
    let mut tokens = Vec::new();
    let mut line = 1u64;
    let mut comment = false;
    let mut in_quotes = false;
    let mut pending = false;
    let mut begin: Option<usize> = None;
    let mut end: Option<usize> = None;
    let mut cursor = 0usize;
    while cursor < input.len() && input[cursor] != 0 {
        let byte = input[cursor];
        if is_line_end(byte) {
            comment = false;
            line += 1;
        }
        if comment {
            cursor += 1;
            continue;
        }
        if in_quotes {
            if byte == b'"' {
                in_quotes = false;
                end = Some(cursor);
                process_data_token(&mut tokens, input, &mut begin, &mut end, line, TokenType::Data, false)?;
                pending = false;
            }
            cursor += 1;
            continue;
        }
        match byte {
            b'"' => {
                if begin.is_some() {
                    return fail("FBX-Tokenize: unexpected double-quote");
                }
                begin = Some(cursor);
                in_quotes = true;
                cursor += 1;
                continue;
            }
            b';' => {
                process_data_token(&mut tokens, input, &mut begin, &mut end, line, TokenType::Data, false)?;
                comment = true;
                cursor += 1;
                continue;
            }
            b'{' | b'}' => {
                process_data_token(&mut tokens, input, &mut begin, &mut end, line, TokenType::Data, false)?;
                let kind = if byte == b'{' { TokenType::OpenBracket } else { TokenType::CloseBracket };
                tokens.push(Token { start: cursor, end: cursor + 1, kind, line, binary: false });
                cursor += 1;
                continue;
            }
            b',' => {
                if pending {
                    process_data_token(&mut tokens, input, &mut begin, &mut end, line, TokenType::Data, true)?;
                }
                tokens.push(Token { start: cursor, end: cursor + 1, kind: TokenType::Comma, line, binary: false });
                cursor += 1;
                continue;
            }
            b':' => {
                if !pending {
                    return fail("FBX-Tokenize: unexpected colon");
                }
                process_data_token(&mut tokens, input, &mut begin, &mut end, line, TokenType::Key, true)?;
                cursor += 1;
                continue;
            }
            _ => {}
        }
        if is_space_or_newline(byte) {
            if begin.is_some() {
                process_data_token(&mut tokens, input, &mut begin, &mut end, line, TokenType::Data, false)?;
            }
            pending = false;
        } else {
            end = Some(cursor);
            if begin.is_none() {
                begin = Some(cursor);
            }
            pending = true;
        }
        cursor += 1;
    }
    Ok(tokens)
}

/// The binary reader's cursor with its bounds checks.
struct BinaryReader<'a> {
    input: &'a [u8],
    cursor: usize,
}

impl BinaryReader<'_> {
    fn word(&mut self, end: usize) -> Result<u32, ImportError> {
        if end.saturating_sub(self.cursor) < 4 || self.cursor > end {
            return fail("FBX-Tokenize: cannot ReadWord, out of bounds");
        }
        let value = u32::from_le_bytes(self.input[self.cursor..self.cursor + 4].try_into().unwrap());
        self.cursor += 4;
        Ok(value)
    }

    fn double_word(&mut self, end: usize) -> Result<u64, ImportError> {
        if end.saturating_sub(self.cursor) < 8 || self.cursor > end {
            return fail("FBX-Tokenize: cannot ReadDoubleWord, out of bounds");
        }
        let value = u64::from_le_bytes(self.input[self.cursor..self.cursor + 8].try_into().unwrap());
        self.cursor += 8;
        Ok(value)
    }

    fn byte(&mut self, end: usize) -> Result<u8, ImportError> {
        if self.cursor >= end {
            return fail("FBX-Tokenize: cannot ReadByte, out of bounds");
        }
        let value = self.input[self.cursor];
        self.cursor += 1;
        Ok(value)
    }

    /// `ReadString`: a one-byte (or four-byte) length, then the bytes.
    fn string(&mut self, end: usize, long_length: bool, allow_null: bool) -> Result<(usize, usize), ImportError> {
        let length_size = if long_length { 4 } else { 1 };
        if end.saturating_sub(self.cursor) < length_size {
            return fail("FBX-Tokenize: cannot ReadString, out of bounds reading length");
        }
        let length = if long_length { self.word(end)? } else { self.byte(end)? as u32 } as usize;
        if end.saturating_sub(self.cursor) < length {
            return fail("FBX-Tokenize: cannot ReadString, length is out of bounds");
        }
        let start = self.cursor;
        self.cursor += length;
        if !allow_null && self.input[start..self.cursor].contains(&0) {
            return fail("FBX-Tokenize: failed ReadString, unexpected NUL character in string");
        }
        Ok((start, self.cursor))
    }

    /// `ReadData`: one property record.
    fn data(&mut self, end: usize) -> Result<(usize, usize), ImportError> {
        if end.saturating_sub(self.cursor) < 1 {
            return fail("FBX-Tokenize: cannot ReadData, out of bounds reading length");
        }
        let kind = self.input[self.cursor];
        let start = self.cursor;
        self.cursor += 1;
        match kind {
            b'Y' => self.cursor += 2,
            b'C' => self.cursor += 1,
            b'I' | b'F' => self.cursor += 4,
            b'D' | b'L' => self.cursor += 8,
            b'R' => {
                let length = self.word(end)? as usize;
                self.cursor += length;
            }
            b'b' => self.cursor = end,
            b'f' | b'd' | b'l' | b'i' | b'c' => {
                let length = self.word(end)? as u64;
                let encoding = self.word(end)?;
                let compressed_length = self.word(end)? as u64;
                if encoding == 0 {
                    let stride = match kind {
                        b'f' | b'i' => 4u64,
                        b'd' | b'l' => 8,
                        _ => 1,
                    };
                    if (length * stride) as u32 as u64 != compressed_length {
                        return fail("FBX-Tokenize: cannot ReadData, calculated data stride differs from what the file claims");
                    }
                } else if encoding != 1 {
                    return fail("FBX-Tokenize: cannot ReadData, unknown encoding");
                }
                self.cursor += compressed_length as usize;
            }
            b'S' => {
                self.string(end, true, true)?;
            }
            _ => return fail("FBX-Tokenize: cannot ReadData, unexpected type code"),
        }
        if self.cursor > end {
            return fail("FBX-Tokenize: cannot ReadData, the remaining size is too small for the data type");
        }
        Ok((start, self.cursor))
    }

    /// `ReadScope`: one node record with its properties and nested nodes.
    fn scope(&mut self, tokens: &mut Vec<Token>, end: usize, is_64: bool) -> Result<bool, ImportError> {
        let end_offset = if is_64 { self.double_word(end)? } else { self.word(end)? as u64 };
        if end_offset == 0 {
            return Ok(false);
        }
        if end_offset > end as u64 {
            return fail("FBX-Tokenize: block offset is out of range");
        }
        if end_offset < self.cursor as u64 {
            return fail("FBX-Tokenize: block offset is negative out of range");
        }
        let end_offset = end_offset as usize;
        let property_count = if is_64 { self.double_word(end)? } else { self.word(end)? as u64 };
        let property_length = if is_64 { self.double_word(end)? } else { self.word(end)? as u64 };
        let (name_start, name_end) = self.string(end, false, false)?;
        tokens.push(Token { start: name_start, end: name_end, kind: TokenType::Key, line: self.cursor as u64, binary: true });

        let begin = self.cursor;
        if begin as u64 + property_length > end as u64 {
            return fail("FBX-Tokenize: property length out of bounds reading length ");
        }
        let properties_end = begin + property_length as usize;
        for index in 0..property_count {
            let (start, stop) = self.data(properties_end)?;
            tokens.push(Token { start, end: stop, kind: TokenType::Data, line: self.cursor as u64, binary: true });
            if index != property_count.wrapping_sub(1) {
                tokens.push(Token { start: self.cursor, end: self.cursor + 1, kind: TokenType::Comma, line: self.cursor as u64, binary: true });
            }
        }
        if self.cursor - begin != property_length as usize {
            return fail("FBX-Tokenize: property length not reached, something is wrong");
        }

        let sentinel = if is_64 { 8 * 3 + 1 } else { 4 * 3 + 1 };
        if self.cursor < end_offset {
            if end_offset - self.cursor < sentinel {
                return fail("FBX-Tokenize: insufficient padding bytes at block end");
            }
            tokens.push(Token { start: self.cursor, end: self.cursor + 1, kind: TokenType::OpenBracket, line: self.cursor as u64, binary: true });
            while self.cursor < end_offset - sentinel {
                self.scope(tokens, end_offset - sentinel, is_64)?;
            }
            tokens.push(Token { start: self.cursor, end: self.cursor + 1, kind: TokenType::CloseBracket, line: self.cursor as u64, binary: true });
            let padding = self.input.get(self.cursor..self.cursor + sentinel).unwrap_or(&[]);
            if padding.len() < sentinel || padding.iter().any(|&byte| byte != 0) {
                return fail("FBX-Tokenize: failed to read nested block sentinel, expected all bytes to be 0");
            }
            self.cursor += sentinel;
        }
        if self.cursor != end_offset {
            return fail("FBX-Tokenize: scope length not reached, something is wrong");
        }
        Ok(true)
    }
}

/// `TokenizeBinary` (`input` is the file plus Assimp's trailing NUL).
pub fn tokenize_binary(input: &[u8]) -> Result<Vec<Token>, ImportError> {
    let length = input.len();
    if length < 0x1b {
        return fail("FBX-Tokenize: file is too short");
    }
    if &input[..18] != b"Kaydara FBX Binary" {
        return fail("FBX-Tokenize: magic bytes not found");
    }
    let mut reader = BinaryReader { input, cursor: 18 };
    for _ in 0..5 {
        reader.byte(length)?;
    }
    let version = reader.word(length)?;
    let is_64 = version >= 7500;
    let mut tokens = Vec::new();
    while reader.cursor < length {
        if !reader.scope(&mut tokens, length, is_64)? {
            break;
        }
    }
    Ok(tokens)
}
