//! Ogre material scripts (`OgreImporter::ReadMaterial`), read through a
//! `std::stringstream` of words, lines and numbers. A material changes no
//! mesh, so only how reading one ends matters: a file under 8 bytes, or a
//! `$shininess_*` value that is not a number, fails the import, and a block
//! still open where the text ends (or a colour or `tex_coord_set` that is
//! not a number) is read forever.

use crate::fast_atof::fast_atof;
use crate::io::FileSystem;
use crate::parsing::{convert_to_utf8, strtod};
use crate::scene::{ImportError, fail};

/// `m_userDefinedMaterialLibFile`'s default, the last file tried.
const MATERIAL_LIBRARY: &[u8] = b"Scene.material";

fn endless<T>(block: &str) -> Result<T, ImportError> {
    fail(format!("endless loop: a material {block} block left open"))
}

/// `isspace` in the "C" locale.
fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// The `std::stringstream`: the text and the stream's end-of-file and fail
/// states. A read from a stream not good fails and changes nothing.
struct TextStream {
    text: Vec<u8>,
    position: usize,
    eof: bool,
    failed: bool,
}

impl TextStream {
    fn good(&self) -> bool {
        !self.eof && !self.failed
    }

    fn at_end(&self) -> bool {
        self.position == self.text.len()
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.position).copied()
    }

    /// The `sentry`: whether the read goes ahead, with leading whitespace
    /// skipped when asked (running into the end fails the stream).
    fn sentry(&mut self, skip_whitespace: bool) -> bool {
        if !self.good() {
            self.failed = true;
            return false;
        }
        if skip_whitespace {
            while self.peek().is_some_and(is_space) {
                self.position += 1;
            }
            if self.at_end() {
                self.eof = true;
                self.failed = true;
                return false;
            }
        }
        true
    }

    /// `>> string`: the next word into `word`, which a failed read leaves.
    fn word(&mut self, word: &mut Vec<u8>) {
        if !self.sentry(true) {
            return;
        }
        word.clear();
        while let Some(byte) = self.peek() {
            if is_space(byte) {
                return;
            }
            word.push(byte);
            self.position += 1;
        }
        self.eof = true;
    }

    /// `getline`: the rest of the line, its '\n' taken but not kept.
    fn line(&mut self) -> Vec<u8> {
        let mut line = Vec::new();
        if !self.sentry(false) {
            return line;
        }
        let mut taken = 0;
        loop {
            let Some(byte) = self.peek() else {
                self.eof = true;
                break;
            };
            self.position += 1;
            taken += 1;
            if byte == b'\n' {
                break;
            }
            line.push(byte);
        }
        if taken == 0 {
            self.failed = true;
        }
        line
    }

    /// The next byte, if it is one of `choices`, taken.
    fn take_any(&mut self, choices: &[u8]) -> Option<u8> {
        let byte = self.peek().filter(|byte| choices.contains(byte))?;
        self.position += 1;
        Some(byte)
    }

    /// Digits taken (hexadecimal ones when asked): how many, and whether
    /// one was not zero.
    fn take_digits(&mut self, hexadecimal: bool) -> (usize, bool) {
        let mut count = 0;
        let mut nonzero = false;
        while let Some(byte) = self.peek() {
            let is_digit = if hexadecimal { byte.is_ascii_hexdigit() } else { byte.is_ascii_digit() };
            if !is_digit {
                break;
            }
            nonzero |= byte != b'0';
            count += 1;
            self.position += 1;
        }
        (count, nonzero)
    }

    /// `>> float`, as the Microsoft library's `num_get` reads it: a sign,
    /// digits around a point (hexadecimal ones after "0x") and an
    /// exponent, then `strtof`. No digit, an exponent mark with no digits
    /// after it, or a value past the float's range fails.
    fn float(&mut self) {
        if !self.sentry(true) {
            return;
        }
        let start = self.position;
        let read = self.float_field(start);
        if !read {
            self.failed = true;
        }
        if self.at_end() {
            self.eof = true;
        }
    }

    /// `_Parse_fp_with_locale` in the "C" locale and the conversion:
    /// whether the field read as a float.
    fn float_field(&mut self, start: usize) -> bool {
        self.take_any(b"+-");
        let mut hexadecimal = false;
        let mut seen_digit = false;
        if self.take_any(b"0").is_some() {
            if self.at_end() {
                return true;
            }
            if self.take_any(b"xX").is_some() {
                hexadecimal = true;
            } else {
                seen_digit = true;
            }
        }
        let (whole_digits, whole_nonzero) = self.take_digits(hexadecimal);
        seen_digit |= whole_digits > 0;
        self.take_any(b".");
        let (fraction_digits, fraction_nonzero) = self.take_digits(hexadecimal);
        seen_digit |= fraction_digits > 0;
        let marks: &[u8] = if hexadecimal { b"pP" } else { b"eE" };
        if seen_digit && self.take_any(marks).is_some() {
            self.take_any(b"+-");
            let (exponent_digits, _) = self.take_digits(false);
            seen_digit = exponent_digits > 0;
        }
        if !seen_digit {
            return false;
        }
        // `strtof` sets `ERANGE` for a value that overflows, or that is
        // not zero but comes to zero.
        let value = strtod(&self.text[start..self.position]) as f32;
        let nonzero = whole_nonzero || fraction_nonzero;
        let out_of_range = value.is_infinite() || (value == 0.0 && nonzero);
        !out_of_range
    }

    /// `>> int`: a sign and decimal digits, whose value must fit a 32-bit
    /// `long` (and so an `int`).
    fn int(&mut self) {
        if !self.sentry(true) {
            return;
        }
        let negative = self.take_any(b"+-") == Some(b'-');
        let digits_start = self.position;
        let (digits, _) = self.take_digits(false);
        let all_digits = &self.text[digits_start..self.position];
        let first_significant = all_digits.iter().position(|&digit| digit != b'0').unwrap_or(all_digits.len());
        let significant = &all_digits[first_significant..];
        let mut value: u64 = 0;
        for &digit in significant.iter().take(11) {
            value = value * 10 + (digit - b'0') as u64;
        }
        let limit: u64 = if negative { 1 << 31 } else { (1 << 31) - 1 };
        let fits = digits > 0 && significant.len() <= 10 && value <= limit;
        if !fits {
            self.failed = true;
        }
        if self.at_end() {
            self.eof = true;
        }
    }
}

/// `$diffuse_map`'s and the like's value: a leading '"' and the last byte
/// dropped.
fn strip_quotes(word: &mut Vec<u8>) {
    if word.first() != Some(&b'"') {
        return;
    }
    if word.len() < 2 {
        word.clear();
        return;
    }
    word.pop();
    word.remove(0);
}

/// `ReadTextureUnit`: up to its `}`.
fn read_texture_unit(stream: &mut TextStream) -> Result<(), ImportError> {
    let mut line_part = Vec::new();
    stream.word(&mut line_part);
    if line_part != b"{" {
        return Ok(());
    }
    while line_part != b"}" {
        if !stream.good() {
            return endless("texture_unit");
        }
        stream.word(&mut line_part);
        if line_part == b"//" {
            stream.line();
            continue;
        }
        if line_part == b"texture" {
            stream.word(&mut line_part);
        } else if line_part == b"tex_coord_set" {
            stream.int();
        }
    }
    Ok(())
}

/// `ReadPass`: colours and texture units, up to its `}`.
fn read_pass(stream: &mut TextStream) -> Result<(), ImportError> {
    let mut line_part = Vec::new();
    stream.word(&mut line_part);
    if line_part != b"{" {
        return Ok(());
    }
    while line_part != b"}" {
        if !stream.good() {
            return endless("pass");
        }
        stream.word(&mut line_part);
        if line_part == b"//" {
            stream.line();
            continue;
        }
        if matches!(line_part.as_slice(), b"ambient" | b"diffuse" | b"specular" | b"emissive") {
            stream.float();
            stream.float();
            stream.float();
        } else if line_part == b"texture_unit" {
            stream.line();
            read_texture_unit(stream)?;
        }
    }
    Ok(())
}

/// `ReadTechnique`: passes, up to its `}`.
fn read_technique(stream: &mut TextStream) -> Result<(), ImportError> {
    let mut line_part = Vec::new();
    stream.word(&mut line_part);
    if line_part != b"{" {
        return Ok(());
    }
    while line_part != b"}" {
        if !stream.good() {
            return endless("technique");
        }
        stream.word(&mut line_part);
        if line_part == b"//" {
            stream.line();
            continue;
        }
        if line_part == b"pass" {
            stream.line();
            read_pass(stream)?;
        }
    }
    Ok(())
}

/// The block of the material named `name`: techniques, and `set` lines
/// (whose value is read into the word the block ends on).
fn read_material_block(stream: &mut TextStream, line_part: &mut Vec<u8>) -> Result<(), ImportError> {
    while line_part != b"}" {
        if !stream.good() {
            return endless("material");
        }
        stream.word(line_part);
        if line_part == b"technique" {
            stream.line();
            read_technique(stream)?;
        }
        if line_part != b"set" {
            continue;
        }
        stream.word(line_part);
        match line_part.as_slice() {
            b"$colormap" | b"$normalmap" => stream.word(line_part),
            b"$diffuse_map" | b"$specular_map" | b"$normal_map" | b"$light_map" => {
                stream.word(line_part);
                strip_quotes(line_part);
            }
            b"$shininess_strength" | b"$shininess_exponent" => {
                stream.word(line_part);
                fast_atof(line_part)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// `ReadMaterial`'s parse: each `material <name>` line, then (for the one
/// named `name`) its block, which must start on the next line.
fn parse(stream: &mut TextStream, name: &[u8]) -> Result<(), ImportError> {
    let mut line_part = Vec::new();
    stream.word(&mut line_part);
    while !stream.eof {
        // A stream that failed short of its end only does so in a block,
        // which then never ends.
        if line_part == b"//" {
            stream.line();
            stream.word(&mut line_part);
            continue;
        }
        if line_part != b"material" {
            stream.word(&mut line_part);
            continue;
        }
        stream.word(&mut line_part);
        if line_part != name {
            stream.word(&mut line_part);
            continue;
        }
        stream.line();
        stream.word(&mut line_part);
        if line_part != b"{" {
            return Ok(());
        }
        read_material_block(stream, &mut line_part)?;
        stream.word(&mut line_part);
    }
    Ok(())
}

/// `ReadMaterial`: the first of `<name>.material`, the mesh's path up to
/// its last ".mesh" plus ".material", and `Scene.material` that opens; an
/// empty file is skipped.
fn read_material(path: &str, files: &FileSystem, name: &[u8]) -> Result<(), ImportError> {
    let mut named = name.to_vec();
    named.extend_from_slice(b".material");
    let path_bytes = path.as_bytes();
    let stem_end = path_bytes.windows(5).rposition(|window| window == b".mesh").unwrap_or(path_bytes.len());
    let mut beside = path_bytes[..stem_end].to_vec();
    beside.extend_from_slice(b".material");
    let candidates = [named, beside, MATERIAL_LIBRARY.to_vec()];
    let Some(mut data) = candidates.iter().find_map(|candidate| files.open_bytes(candidate)) else {
        return Ok(());
    };
    if data.is_empty() {
        return Ok(());
    }
    convert_to_utf8(&mut data)?;
    // `ss << &data[0]`: the text up to its first NUL.
    if let Some(end) = data.iter().position(|&byte| byte == 0) {
        data.truncate(end);
    }
    let mut stream = TextStream { text: data, position: 0, eof: false, failed: false };
    parse(&mut stream, name)
}

/// `ReadMaterials`: the material of each submesh that names one, in order.
pub(super) fn read_materials(path: &str, files: &FileSystem, materials: &[&[u8]]) -> Result<(), ImportError> {
    for name in materials {
        if !name.is_empty() {
            read_material(path, files, name)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(text: &str) -> TextStream {
        TextStream { text: text.as_bytes().to_vec(), position: 0, eof: false, failed: false }
    }

    #[test]
    fn words_lines_and_numbers_set_the_states() {
        let mut text = stream("  one two\nthree");
        let mut word = Vec::new();
        text.word(&mut word);
        assert_eq!(word, b"one");
        assert_eq!(text.line(), b" two");
        text.word(&mut word);
        assert_eq!(word, b"three");
        assert!(text.eof && !text.failed);
        text.word(&mut word);
        assert_eq!(word, b"three");
        assert!(text.failed);

        let mut numbers = stream("1.5 -2e3 0x1p4 .5 7");
        for _ in 0..4 {
            numbers.float();
        }
        assert!(numbers.good());
        numbers.float();
        assert!(numbers.eof && !numbers.failed);

        let mut bad = stream("1e x");
        bad.float();
        assert!(bad.failed && !bad.eof);
        let mut overflow = stream("1e39 ");
        overflow.float();
        assert!(overflow.failed);
        let mut trailing = stream("1.0f ");
        trailing.float();
        assert!(trailing.good());
        assert_eq!(trailing.peek(), Some(b'f'));

        let mut ints = stream("00000000000000000042 2147483648 ");
        ints.int();
        assert!(ints.good());
        ints.int();
        assert!(ints.failed);
    }

    #[test]
    fn open_blocks_never_end() {
        let mut open = stream("material m\n{\n technique\n {\n pass\n {\n diffuse 1 1 1");
        assert!(parse(&mut open, b"m").unwrap_err().ends_process());
        let mut closed = stream("material m\n{\n technique\n {\n pass\n {\n diffuse 1 1 1\n }\n }\n}\n");
        parse(&mut closed, b"m").unwrap();
        let mut not_number = stream("material m\n{\n set $shininess_strength x\n}\n");
        assert!(parse(&mut not_number, b"m").is_err());
        let mut quoted_end = stream("material m\n{\n set $diffuse_map \"}\" \n");
        parse(&mut quoted_end, b"m").unwrap();
    }
}
