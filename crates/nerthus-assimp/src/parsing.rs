//! Assimp's text helpers shared by the text importers: `ParsingUtils.h`
//! over a byte buffer and `BaseImporter::TextFileToBuffer` with its
//! `ConvertToUTF8`.
//!
//! The helpers read the buffer as C reads a NUL-terminated string: a byte
//! past the end reads as NUL, and a loop bounded by `end` still looks at
//! the byte it stops on.

use crate::scene::{ImportError, fail};

/// The byte at `index`, NUL past the end.
pub(crate) fn at(buffer: &[u8], index: usize) -> u8 {
    buffer.get(index).copied().unwrap_or(0)
}

/// Whether the bytes at `index` spell `text` (`strncmp(…) == 0`).
pub(crate) fn starts_with_at(buffer: &[u8], index: usize, text: &[u8]) -> bool {
    text.iter().enumerate().all(|(offset, &byte)| at(buffer, index + offset) == byte)
}

/// `IsSpace`.
pub(crate) fn is_space(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

/// `IsLineEnd`.
pub(crate) fn is_line_end(byte: u8) -> bool {
    byte == b'\r' || byte == b'\n' || byte == 0 || byte == 0x0c
}

/// `IsSpaceOrNewLine`.
pub(crate) fn is_space_or_newline(byte: u8) -> bool {
    is_space(byte) || is_line_end(byte)
}

/// `SkipSpaces`: past spaces and tabs; whether a line goes on there.
pub(crate) fn skip_spaces(buffer: &[u8], mut index: usize, end: usize) -> (usize, bool) {
    while is_space(at(buffer, index)) && index != end {
        index += 1;
    }
    (index, !is_line_end(at(buffer, index)))
}

/// `SkipSpacesAndLineEnd`: past blanks and line breaks; whether the text
/// goes on there.
pub(crate) fn skip_spaces_and_line_end(buffer: &[u8], mut index: usize, end: usize) -> (usize, bool) {
    while matches!(at(buffer, index), b' ' | b'\t' | b'\r' | b'\n') && index != end {
        index += 1;
    }
    (index, at(buffer, index) != 0)
}

/// `SkipLine`: to the next line; whether the text goes on.
pub(crate) fn skip_line(buffer: &[u8], mut index: usize, end: usize) -> (usize, bool) {
    while !matches!(at(buffer, index), b'\r' | b'\n' | 0) && index != end {
        index += 1;
    }
    while matches!(at(buffer, index), b'\r' | b'\n') && index != end {
        index += 1;
    }
    (index, at(buffer, index) != 0)
}

/// `std::isspace` in the C locale.
pub(crate) fn is_c_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

/// MSVC's `atoi`: leading whitespace, a sign, digits, clamped to `int`.
pub(crate) fn atoi(bytes: &[u8], start: usize) -> i32 {
    let mut index = start;
    while is_c_space(at(bytes, index)) {
        index += 1;
    }
    let negative = at(bytes, index) == b'-';
    if negative || at(bytes, index) == b'+' {
        index += 1;
    }
    let mut magnitude = 0u64;
    while at(bytes, index).is_ascii_digit() {
        let digit = (at(bytes, index) - b'0') as u64;
        magnitude = (magnitude * 10 + digit).min(1 << 32);
        index += 1;
    }
    if negative {
        if magnitude > 1 << 31 { i32::MIN } else { -(magnitude as i64) as i32 }
    } else if magnitude > i32::MAX as u64 {
        i32::MAX
    } else {
        magnitude as i32
    }
}

/// `ai_trim`.
pub(crate) fn trim(bytes: &[u8]) -> Vec<u8> {
    let start = bytes.iter().position(|&byte| !is_c_space(byte)).unwrap_or(bytes.len());
    let end = bytes.iter().rposition(|&byte| !is_c_space(byte)).map_or(start, |last| last + 1);
    bytes[start..end.max(start)].to_vec()
}

/// C's `strtod` (correctly rounded, as MSVC's and macOS's are): after
/// leading blanks, the longest prefix that reads as a number: a decimal
/// or hexadecimal one, or "inf", "infinity" or "nan" in any case, maybe
/// signed. 0 where there is none.
pub(crate) fn strtod(bytes: &[u8]) -> f64 {
    let mut index = 0;
    while is_c_space(at(bytes, index)) {
        index += 1;
    }
    let negative = at(bytes, index) == b'-';
    if matches!(at(bytes, index), b'+' | b'-') {
        index += 1;
    }
    let magnitude = if starts_with_ignore_case_at(bytes, index, b"inf") {
        f64::INFINITY
    } else if starts_with_ignore_case_at(bytes, index, b"nan") {
        f64::NAN
    } else if at(bytes, index) == b'0' && (at(bytes, index + 1) | 0x20) == b'x' && starts_hex_mantissa(bytes, index + 2) {
        hex_magnitude(bytes, index + 2)
    } else {
        decimal_magnitude(bytes, index)
    };
    if negative { -magnitude } else { magnitude }
}

/// Whether a hexadecimal mantissa starts here (a digit, or '.' and one).
fn starts_hex_mantissa(bytes: &[u8], index: usize) -> bool {
    let first = at(bytes, index);
    first.is_ascii_hexdigit() || (first == b'.' && at(bytes, index + 1).is_ascii_hexdigit())
}

/// A decimal number's magnitude: digits, '.', digits (at least one
/// digit), then an exponent if it has digits.
fn decimal_magnitude(bytes: &[u8], start: usize) -> f64 {
    match decimal_text(bytes, start) {
        Some((text, _)) => text.parse::<f64>().unwrap_or(0.0),
        None => 0.0,
    }
}

/// A hexadecimal number's magnitude ("0x" passed): hex digits, '.', hex
/// digits, then a binary exponent ("p") if it has digits, rounded to
/// nearest, ties to even.
fn hex_magnitude(bytes: &[u8], start: usize) -> f64 {
    let (mantissa, exponent, sticky) = hex_parts(bytes, start);
    binary_to_f64(mantissa, exponent, sticky)
}

/// A hexadecimal number's value ("0x" passed) as a 128-bit mantissa, a
/// binary exponent and whether nonzero digits fell below the mantissa.
fn hex_parts(bytes: &[u8], start: usize) -> (u128, i64, bool) {
    let mut index = start;
    let mut mantissa = 0u128;
    let mut exponent = 0i64;
    let mut sticky = false;
    let mut seen_point = false;
    loop {
        let byte = at(bytes, index);
        if byte == b'.' && !seen_point {
            seen_point = true;
            index += 1;
            continue;
        }
        let Some(digit) = (byte as char).to_digit(16) else {
            break;
        };
        if mantissa >> 120 == 0 {
            mantissa = (mantissa << 4) | digit as u128;
            if seen_point {
                exponent -= 4;
            }
        } else {
            sticky |= digit != 0;
            if !seen_point {
                exponent += 4;
            }
        }
        index += 1;
    }
    if (at(bytes, index) | 0x20) == b'p' {
        let mut exponent_index = index + 1;
        let exponent_negative = at(bytes, exponent_index) == b'-';
        if matches!(at(bytes, exponent_index), b'+' | b'-') {
            exponent_index += 1;
        }
        if at(bytes, exponent_index).is_ascii_digit() {
            let mut value = 0i64;
            while at(bytes, exponent_index).is_ascii_digit() {
                value = (value * 10 + (at(bytes, exponent_index) - b'0') as i64).min(1 << 40);
                exponent_index += 1;
            }
            exponent += if exponent_negative { -value } else { value };
        }
    }
    (mantissa, exponent, sticky)
}

/// `mantissa` × 2^`exponent` (plus a sticky bit below it) as the nearest
/// double, ties to even.
fn binary_to_f64(mantissa: u128, exponent: i64, sticky: bool) -> f64 {
    round_binary(mantissa, exponent, sticky, 53, -1022, 1023)
}

/// 2^`exponent`, for exponents a double holds (subnormal ones included).
fn power_of_two(exponent: i64) -> f64 {
    if exponent >= -1022 {
        f64::from_bits(((exponent + 1023) as u64) << 52)
    } else {
        f64::from_bits(1u64 << (exponent + 1074))
    }
}

/// `mantissa` × 2^`exponent` (plus a sticky bit below it) rounded to the
/// nearest value with `precision` significant bits and an exponent of at
/// most `maximum` (subnormal below `minimum`), ties to even, as a double;
/// infinity past the range.
fn round_binary(mantissa: u128, exponent: i64, sticky: bool, precision: i64, minimum: i64, maximum: i64) -> f64 {
    if mantissa == 0 {
        return 0.0;
    }
    let shift = mantissa.leading_zeros() as i64;
    let normalized = mantissa << shift;
    // The value is 1.f × 2^top, the leading bit at 127.
    let top = exponent - shift + 127;
    if top > maximum {
        return f64::INFINITY;
    }
    // How many of the 128 bits stay (all `precision` for a normal number,
    // fewer below).
    let kept = if top >= minimum { precision } else { precision - (minimum - top) };
    if kept <= 0 {
        // Below half the smallest subnormal, or at most half of it.
        let half_or_more = kept == 0 && (normalized > 1 << 127 || (normalized == 1 << 127 && sticky));
        return if half_or_more { power_of_two(minimum - precision + 1) } else { 0.0 };
    }
    let dropped = 128 - kept as u32;
    let mut bits = (normalized >> dropped) as u64;
    let remainder = normalized & ((1u128 << dropped) - 1);
    let half = 1u128 << (dropped - 1);
    let tie_goes_up = sticky || bits & 1 == 1;
    if remainder > half || (remainder == half && tie_goes_up) {
        bits += 1;
    }
    // Back to a value: `bits` × 2^(top − kept + 1), which is exact.
    let scale = power_of_two(top - kept + 1);
    bits as f64 * scale
}

/// `std::stof` as Microsoft's library runs it: `strtof` after leading
/// blanks (a decimal or hexadecimal number, "inf", "infinity" or "nan",
/// maybe signed, correctly rounded to a float), throwing when no number
/// starts the text or `errno` is `ERANGE`: past the largest float, or not
/// zero but rounding to zero.
pub(crate) fn stof(bytes: &[u8]) -> Result<f32, ImportError> {
    let mut index = 0;
    while is_c_space(at(bytes, index)) {
        index += 1;
    }
    let negative = at(bytes, index) == b'-';
    if matches!(at(bytes, index), b'+' | b'-') {
        index += 1;
    }
    let sign = if negative { -1.0f32 } else { 1.0 };
    if starts_with_ignore_case_at(bytes, index, b"inf") {
        return Ok(sign * f32::INFINITY);
    }
    if starts_with_ignore_case_at(bytes, index, b"nan") {
        return Ok(if negative { -f32::NAN } else { f32::NAN });
    }
    let (magnitude, nonzero) = if at(bytes, index) == b'0' && (at(bytes, index + 1) | 0x20) == b'x' && starts_hex_mantissa(bytes, index + 2) {
        let (mantissa, exponent, sticky) = hex_parts(bytes, index + 2);
        let value = round_binary(mantissa, exponent, sticky, 24, -126, 127) as f32;
        (value, mantissa != 0 || sticky)
    } else {
        let Some((text, nonzero)) = decimal_text(bytes, index) else {
            return fail("invalid stof argument");
        };
        (text.parse::<f32>().unwrap_or(0.0), nonzero)
    };
    if magnitude.is_infinite() || (magnitude == 0.0 && nonzero) {
        return fail("stof argument out of range");
    }
    Ok(sign * magnitude)
}

/// A decimal number's text as `decimal_magnitude` reads it, and whether a
/// digit of it is not zero; none without a digit.
fn decimal_text(bytes: &[u8], start: usize) -> Option<(String, bool)> {
    let mut index = start;
    let mut text = String::new();
    let mut nonzero = false;
    let mut digits = 0;
    while at(bytes, index).is_ascii_digit() {
        nonzero |= at(bytes, index) != b'0';
        text.push(at(bytes, index) as char);
        digits += 1;
        index += 1;
    }
    if at(bytes, index) == b'.' {
        index += 1;
        text.push('.');
        while at(bytes, index).is_ascii_digit() {
            nonzero |= at(bytes, index) != b'0';
            text.push(at(bytes, index) as char);
            digits += 1;
            index += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if (at(bytes, index) | 0x20) == b'e' {
        let mut exponent_index = index + 1;
        let mut exponent = String::from("e");
        if matches!(at(bytes, exponent_index), b'+' | b'-') {
            exponent.push(at(bytes, exponent_index) as char);
            exponent_index += 1;
        }
        if at(bytes, exponent_index).is_ascii_digit() {
            while at(bytes, exponent_index).is_ascii_digit() {
                exponent.push(at(bytes, exponent_index) as char);
                exponent_index += 1;
            }
            text.push_str(&exponent);
        }
    }
    Some((text, nonzero))
}

/// `CommentRemover::RemoveLineComments` over the C string: each `comment`
/// outside quotes is overwritten with `replacement` up to its line end. A
/// quote of either kind runs to the next quote of either kind.
pub(crate) fn remove_line_comments(comment: &[u8], buffer: &mut [u8], replacement: u8) {
    let length = buffer.iter().position(|&byte| byte == 0).unwrap_or(buffer.len());
    let comment_length = comment.len().min(length);
    let comment = &comment[..comment_length];
    let mut index = 0;
    while index < length {
        if buffer[index] == b'"' || buffer[index] == b'\'' {
            index += 1;
            while index < length && buffer[index] != b'"' && buffer[index] != b'\'' {
                index += 1;
            }
        }
        if length - index < comment_length {
            break;
        }
        if buffer[index..].starts_with(comment) {
            while index < length && !is_line_end(buffer[index]) {
                buffer[index] = replacement;
                index += 1;
            }
        }
        index += 1;
    }
}

/// `ASSIMP_strincmp(…) == 0` for the bytes at `index`: the same as `text`
/// in any (ASCII) case.
pub(crate) fn starts_with_ignore_case_at(buffer: &[u8], index: usize, text: &[u8]) -> bool {
    text.iter().enumerate().all(|(offset, byte)| at(buffer, index + offset).eq_ignore_ascii_case(byte))
}

/// `TokenMatch`: the token (exact case), then a blank or line end, which
/// is passed over too unless it is the NUL.
pub(crate) fn token_match(buffer: &[u8], position: &mut usize, token: &[u8]) -> bool {
    let length = token.len();
    if !starts_with_at(buffer, *position, token) || !is_space_or_newline(at(buffer, *position + length)) {
        return false;
    }
    if at(buffer, *position + length) != 0 {
        *position += length + 1;
    } else {
        *position += length;
    }
    true
}

/// `TokenMatchI`: the token in any case, then a blank or line end, which
/// is passed over too. Passing over the final NUL leaves the cursor past
/// the buffer; the callers all read there next, undefined behaviour, so
/// that fails here.
pub(crate) fn token_match_i(buffer: &[u8], position: &mut usize, token: &[u8]) -> Result<bool, ImportError> {
    let length = token.len();
    if !starts_with_ignore_case_at(buffer, *position, token) || !is_space_or_newline(at(buffer, *position + length)) {
        return Ok(false);
    }
    *position += length + 1;
    if *position >= buffer.len() {
        return fail("undefined behaviour: a read past the file");
    }
    Ok(true)
}

/// The byte at `index`, which must lie in the buffer (with its NUL): a
/// read past it is undefined behaviour.
pub(crate) fn checked_at(buffer: &[u8], index: usize) -> Result<u8, ImportError> {
    match buffer.get(index) {
        Some(&byte) => Ok(byte),
        None => fail("undefined behaviour: a read past the file"),
    }
}

/// `BufferSize`: the most bytes of a line `GetNextLine` copies.
pub(crate) const BUFFER_SIZE: usize = 4096;

/// `GetNextLine`: the line at `index` (up to a line end, at most
/// `BUFFER_SIZE` bytes) copied into `line` with a NUL after it, then the
/// line ends after it skipped; false at a NUL. Bytes after the NUL keep
/// what earlier lines left there. A write past the caller's array (`line`,
/// as long as it) is undefined behaviour, an error here.
pub(crate) fn get_next_line(buffer: &[u8], index: &mut usize, line: &mut [u8]) -> Result<bool, ImportError> {
    if at(buffer, *index) == 0 {
        return Ok(false);
    }
    let mut length = 0;
    while !is_line_end(at(buffer, *index)) && length < BUFFER_SIZE {
        let Some(slot) = line.get_mut(length) else {
            return fail("undefined behaviour: a line longer than its buffer");
        };
        *slot = at(buffer, *index);
        *index += 1;
        length += 1;
    }
    let Some(slot) = line.get_mut(length) else {
        return fail("undefined behaviour: a line longer than its buffer");
    };
    *slot = 0;
    while is_line_end(at(buffer, *index)) && at(buffer, *index) != 0 {
        *index += 1;
    }
    Ok(true)
}

/// `TextFileToBuffer`: the file through `ConvertToUTF8`, then a NUL. An
/// empty file fails unless allowed.
pub(crate) fn text_file_to_buffer(bytes: &[u8], allow_empty: bool) -> Result<Vec<u8>, ImportError> {
    if bytes.is_empty() && !allow_empty {
        return fail("File is empty");
    }
    let mut data = bytes.to_vec();
    if !data.is_empty() {
        convert_to_utf8(&mut data)?;
    }
    data.push(0);
    Ok(data)
}

/// `BaseImporter::ConvertToUTF8` as it behaves: files under 8 bytes fail;
/// a UTF-8 BOM is dropped; big-endian UTF-32 and UTF-16 files (by their
/// BOMs) are byte-swapped in place; the conversions themselves are
/// thrown away, but the UTF-32 one throws on an invalid code point (its
/// last word reaching past the data, read as zeros here).
pub(crate) fn convert_to_utf8(data: &mut Vec<u8>) -> Result<(), ImportError> {
    if data.len() < 8 {
        return fail("File is too small");
    }
    if data[..3] == [0xef, 0xbb, 0xbf] {
        data.drain(..3);
        return Ok(());
    }
    let first_word = |data: &[u8]| u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if first_word(data) == 0xfffe_0000 {
        let length = data.len();
        let mut padded = data.clone();
        padded.resize(length.div_ceil(4) * 4, 0);
        for word in padded.as_chunks_mut::<4>().0 {
            word.reverse();
        }
        data.copy_from_slice(&padded[..length]);
    }
    if first_word(data) == 0x0000_fffe {
        let word_count = data.len() / 4 + 1;
        for index in 0..word_count {
            let bytes: [u8; 4] = std::array::from_fn(|offset| at(data, index * 4 + offset));
            let code_point = i32::from_le_bytes(bytes) as u32;
            let surrogate = (0xd800..=0xdfff).contains(&code_point);
            if code_point > 0x10_ffff || surrogate {
                return fail("Internal error");
            }
        }
        return Ok(());
    }
    if u16::from_le_bytes([data[0], data[1]]) == 0xfffe {
        if !data.len().is_multiple_of(2) {
            return Ok(());
        }
        for pair in data.as_chunks_mut::<2>().0 {
            pair.swap(0, 1);
        }
    }
    // Little-endian UTF-16 (or the swapped big-endian): the conversion
    // reads bytes as code units, which never form surrogates, and is
    // discarded.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_boms_go_and_short_files_fail() {
        let mut data = b"\xef\xbb\xbfsolid x\n".to_vec();
        convert_to_utf8(&mut data).unwrap();
        assert_eq!(data, b"solid x\n");
        assert!(convert_to_utf8(&mut b"solid".to_vec()).is_err());
    }

    #[test]
    fn big_endian_utf16_is_swapped() {
        let mut data = vec![0xfe, 0xff, 0, b's', 0, b'o', 0, b'l'];
        convert_to_utf8(&mut data).unwrap();
        assert_eq!(data, vec![0xff, 0xfe, b's', 0, b'o', 0, b'l', 0]);
    }

    #[test]
    fn lines_are_copied_with_a_nul() {
        let buffer = b"ab\r\n\x0c\ncd\0";
        let mut line = [b'x'; 8];
        let mut index = 0;
        assert!(get_next_line(buffer, &mut index, &mut line).unwrap());
        assert_eq!(&line[..4], b"ab\0x");
        assert_eq!(index, 6);
        assert!(get_next_line(buffer, &mut index, &mut line).unwrap());
        assert_eq!(&line[..3], b"cd\0");
        assert!(!get_next_line(buffer, &mut index, &mut line).unwrap());
        let mut short = [0u8; 2];
        let mut index = 0;
        assert!(get_next_line(buffer, &mut index, &mut short).is_err());
    }

    #[test]
    fn stof_rounds_once_and_throws_out_of_range() {
        assert_eq!(stof(b" 0.1").unwrap(), 0.1f32);
        assert_eq!(stof(b"-2e1x").unwrap(), -20.0);
        assert_eq!(stof(b"0x1.8p1").unwrap(), 3.0);
        assert_eq!(stof(b"1e-40").unwrap(), 1e-40f32);
        assert!(stof(b"0x").unwrap() == 0.0);
        assert!(stof(b"abc").is_err());
        assert!(stof(b"1e39").is_err());
        assert!(stof(b"1e-50").is_err());
        assert_eq!(stof(b"0e-50").unwrap(), 0.0);
        assert!(stof(b"-inf").unwrap().is_infinite());
    }

    #[test]
    fn strtod_reads_the_longest_number() {
        assert_eq!(strtod(b"  1.5e2x"), 150.0);
        assert_eq!(strtod(b".5"), 0.5);
        assert_eq!(strtod(b"5."), 5.0);
        assert_eq!(strtod(b"-0").to_bits(), (-0.0f64).to_bits());
        assert_eq!(strtod(b"1e"), 1.0);
        assert_eq!(strtod(b"1e+"), 1.0);
        assert_eq!(strtod(b"abc"), 0.0);
        assert_eq!(strtod(b"."), 0.0);
        assert_eq!(strtod(b"-Infinity"), f64::NEG_INFINITY);
        assert!(strtod(b"nan").is_nan());
        assert_eq!(strtod(b"1e400"), f64::INFINITY);
        assert_eq!(strtod(b"0.1"), 0.1);
        assert_eq!(strtod(b"0x1p3"), 8.0);
        assert_eq!(strtod(b"0X.8"), 0.5);
        assert_eq!(strtod(b"0x"), 0.0);
        assert_eq!(strtod(b"0x1.fffffffffffff8p0"), 2.0);
        assert_eq!(strtod(b"0x1p-1074"), f64::from_bits(1));
        assert_eq!(strtod(b"0x1p-1075"), 0.0);
        assert_eq!(strtod(b"0x1.8p-1075"), f64::from_bits(1));
        assert_eq!(strtod(b"0x1p1024"), f64::INFINITY);
    }

    #[test]
    fn skipping_stops_at_the_end() {
        let buffer = b"  a\r\n b\0";
        assert_eq!(skip_spaces(buffer, 0, 7), (2, true));
        assert_eq!(skip_spaces_and_line_end(buffer, 3, 7), (6, true));
        assert_eq!(skip_line(buffer, 0, 7), (5, true));
    }
}
