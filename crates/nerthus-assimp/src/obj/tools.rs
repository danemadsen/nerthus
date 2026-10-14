//! ObjTools.h and the parsing helpers the OBJ and MTL readers share, over
//! byte buffers addressed by index (`end` is the buffer's end, as the C++
//! iterators have it; reads past the buffer give 0).

use crate::fast_atof::fast_atof;
pub(crate) use crate::parsing::{atoi, trim};
use crate::scene::ImportError;

pub fn at(bytes: &[u8], index: usize) -> u8 {
    bytes.get(index).copied().unwrap_or(0)
}

/// `IsSpace`.
pub fn is_space(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

/// `IsLineEnd`.
pub fn is_line_end(byte: u8) -> bool {
    matches!(byte, b'\r' | b'\n' | 0 | 0x0C)
}

/// `IsSpaceOrNewLine`.
pub fn is_space_or_new_line(byte: u8) -> bool {
    is_space(byte) || is_line_end(byte)
}

/// `isEndOfBuffer`: at the end or the last element.
pub fn is_end_of_buffer(it: usize, end: usize) -> bool {
    it == end || it + 1 == end
}

/// `getNextWord`: past spaces and tabs (not past a line end).
pub fn next_word(bytes: &[u8], mut it: usize, end: usize) -> usize {
    while !is_end_of_buffer(it, end) {
        let byte = at(bytes, it);
        if !is_space_or_new_line(byte) || is_line_end(byte) {
            break;
        }
        it += 1;
    }
    it
}

/// `getNextToken`: past the current token, then to the next word.
pub fn next_token(bytes: &[u8], mut it: usize, end: usize) -> usize {
    while !is_end_of_buffer(it, end) {
        if is_space_or_new_line(at(bytes, it)) {
            break;
        }
        it += 1;
    }
    next_word(bytes, it, end)
}

/// `skipLine`: past the line end, then past the next line's leading
/// spaces and tabs.
pub fn skip_line(bytes: &[u8], mut it: usize, end: usize) -> usize {
    if it >= end {
        return it;
    }
    while !is_end_of_buffer(it, end) && !is_line_end(at(bytes, it)) {
        it += 1;
    }
    if it != end {
        it += 1;
    }
    while it != end && (at(bytes, it) == b'\t' || at(bytes, it) == b' ') {
        it += 1;
    }
    it
}

/// `getName`: the rest of the line (its trailing-space trim starts at the
/// line end, so trailing spaces stay).
pub fn get_name(bytes: &[u8], mut it: usize, end: usize) -> (usize, Vec<u8>) {
    if is_end_of_buffer(it, end) {
        return (end, Vec::new());
    }
    let start = it;
    while !is_end_of_buffer(it, end) && !is_line_end(at(bytes, it)) {
        it += 1;
    }
    while it > 0 && is_space(at(bytes, it)) {
        it -= 1;
    }
    it = it.max(start);
    let name = bytes[start..it.min(bytes.len())].to_vec();
    (it, name)
}

/// `getNameNoSpace`: the word at the cursor.
pub fn get_name_no_space(bytes: &[u8], mut it: usize, end: usize) -> (usize, Vec<u8>) {
    if is_end_of_buffer(it, end) {
        return (end, Vec::new());
    }
    let start = it;
    while !is_end_of_buffer(it, end) && !is_line_end(at(bytes, it)) && !is_space_or_new_line(at(bytes, it)) {
        it += 1;
    }
    while it > 0 && (is_end_of_buffer(it, end) || is_line_end(at(bytes, it)) || is_space_or_new_line(at(bytes, it))) {
        it -= 1;
    }
    it += 1;
    it = it.max(start);
    let name = bytes[start..it.min(bytes.len())].to_vec();
    (it, name)
}

/// `CopyNextWord` (ObjTools): the next word, at most `length - 1` bytes.
pub fn copy_next_word(bytes: &[u8], it: usize, end: usize, length: usize) -> (usize, Vec<u8>) {
    let mut it = next_word(bytes, it, end);
    let mut word = Vec::new();
    while !is_space_or_new_line(at(bytes, it)) && !is_end_of_buffer(it, end) {
        word.push(at(bytes, it));
        if word.len() == length - 1 {
            break;
        }
        it += 1;
    }
    (it, word)
}

/// `getFloat`: the next word through `fast_atof` (which throws on a
/// non-number, an empty word included).
pub fn get_float(bytes: &[u8], it: usize, end: usize) -> Result<(usize, f32), ImportError> {
    let (it, word) = copy_next_word(bytes, it, end, 1024);
    let value = fast_atof(&word)?;
    Ok((it, value))
}

/// `ASSIMP_strincmp(text + start, prefix, strlen(prefix)) == 0`.
pub fn starts_with_ignore_case(bytes: &[u8], start: usize, prefix: &[u8]) -> bool {
    prefix.iter().enumerate().all(|(offset, &letter)| at(bytes, start + offset).eq_ignore_ascii_case(&letter))
}

