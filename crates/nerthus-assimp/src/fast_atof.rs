//! Assimp's number parsing (fast_atof.h): `strtoul10`, `strtol10`,
//! `strtoul10_64`, `strtol10_64` and `fast_atoreal_move<float>`, which is
//! not correctly rounded: the integer part and up to 15 fraction digits
//! (scaled in double) are added in float, and an exponent multiplies by
//! `powf(10, e)`. A ',' also counts as the decimal point.

use crate::scene::{ImportError, fail};

/// `fast_atof_table`: 10^-n for n = 0 … 15 as double literals.
const FRACTION_SCALE: [f64; 16] = [
    0.0, 0.1, 0.01, 0.001, 0.0001, 0.00001, 0.000001, 0.0000001, 0.00000001, 0.000000001, 0.0000000001, 0.00000000001, 0.000000000001,
    0.0000000000001, 0.00000000000001, 0.000000000000001,
];

fn at(bytes: &[u8], index: usize) -> u8 {
    bytes.get(index).copied().unwrap_or(0)
}

/// `strtoul10`: decimal digits into a wrapping `unsigned int`.
pub fn strtoul10(bytes: &[u8], start: usize) -> (u32, usize) {
    let mut value = 0u32;
    let mut position = start;
    while at(bytes, position).is_ascii_digit() {
        value = value.wrapping_mul(10).wrapping_add((at(bytes, position) - b'0') as u32);
        position += 1;
    }
    (value, position)
}

/// `strtoul8`: octal digits, wrapping.
pub fn strtoul8(bytes: &[u8], start: usize) -> (u32, usize) {
    let mut value = 0u32;
    let mut index = start;
    while let Some(&byte) = bytes.get(index) {
        if !(b'0'..=b'7').contains(&byte) {
            break;
        }
        value = (value << 3).wrapping_add((byte - b'0') as u32);
        index += 1;
    }
    (value, index)
}

/// `strtoul16`: hex digits, wrapping.
pub fn strtoul16(bytes: &[u8], start: usize) -> (u32, usize) {
    let mut value = 0u32;
    let mut index = start;
    while let Some(&byte) = bytes.get(index) {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'A'..=b'F' => byte - b'A' + 10,
            b'a'..=b'f' => byte - b'a' + 10,
            _ => break,
        };
        value = (value << 4).wrapping_add(digit as u32);
        index += 1;
    }
    (value, index)
}

/// `strtoul_cppstyle`: `0x` hex, a leading `0` octal, else decimal.
pub fn strtoul_cppstyle(bytes: &[u8], start: usize) -> (u32, usize) {
    if bytes.get(start) == Some(&b'0') {
        if bytes.get(start + 1) == Some(&b'x') {
            return strtoul16(bytes, start + 2);
        }
        return strtoul8(bytes, start + 1);
    }
    strtoul10(bytes, start)
}

/// `strtol10`: an optional sign, then `strtoul10`; negated unless the
/// value (as `int`) is `INT_MAX` or `INT_MIN`.
pub fn strtol10(bytes: &[u8], start: usize) -> (i32, usize) {
    let mut position = start;
    let negative = at(bytes, position) == b'-';
    if negative || at(bytes, position) == b'+' {
        position += 1;
    }
    let (unsigned, end) = strtoul10(bytes, position);
    let mut value = unsigned as i32;
    if negative && value < i32::MAX && value > i32::MIN {
        value = -value;
    }
    (value, end)
}

/// `strtoul10_64` (the result's `end` is `None` when `out` is left unset:
/// on overflow it returns 0 without moving it).
pub fn strtoul10_64(bytes: &[u8], start: usize, max_digits: Option<&mut u32>) -> Result<(u64, Option<usize>), ImportError> {
    let mut position = start;
    if !at(bytes, position).is_ascii_digit() {
        return fail("The string cannot be converted into a value.");
    }
    let mut value = 0u64;
    let mut digits = 0u32;
    let limit = max_digits.as_ref().map(|limit| **limit);
    loop {
        let byte = at(bytes, position);
        if !byte.is_ascii_digit() {
            break;
        }
        let next = value.wrapping_mul(10).wrapping_add((byte - b'0') as u64);
        if next < value {
            return Ok((0, None));
        }
        value = next;
        position += 1;
        digits += 1;
        if limit == Some(digits) {
            while at(bytes, position).is_ascii_digit() {
                position += 1;
            }
            return Ok((value, Some(position)));
        }
    }
    if let Some(limit) = max_digits {
        *limit = digits;
    }
    Ok((value, Some(position)))
}

/// `strtol10_64`.
pub fn strtol10_64(bytes: &[u8], start: usize, max_digits: Option<&mut u32>) -> Result<(i64, Option<usize>), ImportError> {
    let mut position = start;
    let negative = at(bytes, position) == b'-';
    if negative || at(bytes, position) == b'+' {
        position += 1;
    }
    let (value, end) = strtoul10_64(bytes, position, max_digits)?;
    let signed = if negative { (value as i64).wrapping_neg() } else { value as i64 };
    Ok((signed, end))
}

fn starts_with_ignore_case(bytes: &[u8], start: usize, word: &[u8]) -> bool {
    word.iter().enumerate().all(|(offset, &letter)| at(bytes, start + offset).eq_ignore_ascii_case(&letter))
}

/// `fast_atoreal_move<float>`: the value and the position after it.
pub fn fast_atoreal_move(bytes: &[u8], start: usize, check_comma: bool) -> Result<(f32, usize), ImportError> {
    let mut position = start;
    let negative = at(bytes, position) == b'-';
    if negative || at(bytes, position) == b'+' {
        position += 1;
    }
    let first = at(bytes, position);
    if (first == b'N' || first == b'n') && starts_with_ignore_case(bytes, position, b"nan") {
        return Ok((f32::NAN, position + 3));
    }
    if (first == b'I' || first == b'i') && starts_with_ignore_case(bytes, position, b"inf") {
        let mut value = f32::INFINITY;
        if negative {
            value = -value;
        }
        position += 3;
        let next = at(bytes, position);
        if (next == b'I' || next == b'i') && starts_with_ignore_case(bytes, position, b"inity") {
            position += 5;
        }
        return Ok((value, position));
    }
    let is_point = |byte: u8| byte == b'.' || (check_comma && byte == b',');
    let starts_number = first.is_ascii_digit() || (is_point(first) && at(bytes, position + 1).is_ascii_digit());
    if !starts_number {
        return fail("Cannot parse string as a real number: does not start with digit or decimal point followed by digit.");
    }

    let mut value = 0.0f32;
    if !is_point(at(bytes, position)) {
        let (integer, end) = strtoul10_64(bytes, position, None)?;
        value = integer as f32;
        // On overflow `strtoul10_64` returns 0 and leaves the cursor.
        if let Some(end) = end {
            position = end;
        }
    }
    if is_point(at(bytes, position)) && at(bytes, position + 1).is_ascii_digit() {
        position += 1;
        let mut digits = 15u32;
        let (fraction, end) = strtoul10_64(bytes, position, Some(&mut digits))?;
        if let Some(end) = end {
            position = end;
        }
        let scaled = fraction as f64 * FRACTION_SCALE[digits as usize];
        value += scaled as f32;
    } else if at(bytes, position) == b'.' {
        position += 1;
    }
    let marker = at(bytes, position);
    if marker == b'e' || marker == b'E' {
        position += 1;
        let exponent_negative = at(bytes, position) == b'-';
        if exponent_negative || at(bytes, position) == b'+' {
            position += 1;
        }
        let (magnitude, end) = strtoul10_64(bytes, position, None)?;
        if let Some(end) = end {
            position = end;
        }
        let mut exponent = magnitude as f32;
        if exponent_negative {
            exponent = -exponent;
        }
        value *= 10.0f32.powf(exponent);
    }
    if negative {
        value = -value;
    }
    Ok((value, position))
}

/// `fast_atoreal_move<double>`: as the float version, in doubles.
pub fn fast_atoreal_move_f64(bytes: &[u8], start: usize, check_comma: bool) -> Result<(f64, usize), ImportError> {
    let mut position = start;
    let negative = at(bytes, position) == b'-';
    if negative || at(bytes, position) == b'+' {
        position += 1;
    }
    let first = at(bytes, position);
    if (first == b'N' || first == b'n') && starts_with_ignore_case(bytes, position, b"nan") {
        return Ok((f64::NAN, position + 3));
    }
    if (first == b'I' || first == b'i') && starts_with_ignore_case(bytes, position, b"inf") {
        let mut value = f64::INFINITY;
        if negative {
            value = -value;
        }
        position += 3;
        let next = at(bytes, position);
        if (next == b'I' || next == b'i') && starts_with_ignore_case(bytes, position, b"inity") {
            position += 5;
        }
        return Ok((value, position));
    }
    let is_point = |byte: u8| byte == b'.' || (check_comma && byte == b',');
    let starts_number = first.is_ascii_digit() || (is_point(first) && at(bytes, position + 1).is_ascii_digit());
    if !starts_number {
        return fail("Cannot parse string as a real number: does not start with digit or decimal point followed by digit.");
    }

    let mut value = 0.0f64;
    if !is_point(at(bytes, position)) {
        let (integer, end) = strtoul10_64(bytes, position, None)?;
        value = integer as f64;
        if let Some(end) = end {
            position = end;
        }
    }
    if is_point(at(bytes, position)) && at(bytes, position + 1).is_ascii_digit() {
        position += 1;
        let mut digits = 15u32;
        let (fraction, end) = strtoul10_64(bytes, position, Some(&mut digits))?;
        if let Some(end) = end {
            position = end;
        }
        value += fraction as f64 * FRACTION_SCALE[digits as usize];
    } else if at(bytes, position) == b'.' {
        position += 1;
    }
    let marker = at(bytes, position);
    if marker == b'e' || marker == b'E' {
        position += 1;
        let exponent_negative = at(bytes, position) == b'-';
        if exponent_negative || at(bytes, position) == b'+' {
            position += 1;
        }
        let (magnitude, end) = strtoul10_64(bytes, position, None)?;
        if let Some(end) = end {
            position = end;
        }
        let mut exponent = magnitude as f64;
        if exponent_negative {
            exponent = -exponent;
        }
        value *= 10.0f64.powf(exponent);
    }
    if negative {
        value = -value;
    }
    Ok((value, position))
}

/// `fast_atof(const char*)`: the value only.
pub fn fast_atof(bytes: &[u8]) -> Result<f32, ImportError> {
    let (value, _) = fast_atoreal_move(bytes, 0, true)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_like_assimp() {
        assert_eq!(fast_atof(b"1.5").unwrap(), 1.5);
        assert_eq!(fast_atof(b"-0.25e2").unwrap(), -25.0);
        assert_eq!(fast_atof(b"3,5").unwrap(), 3.5);
        assert_eq!(fast_atof(b".5").unwrap(), 0.5);
        assert_eq!(fast_atof(b"7.").unwrap(), 7.0);
        assert!(fast_atof(b"nan").unwrap().is_nan());
        assert_eq!(fast_atof(b"-Infinity").unwrap(), f32::NEG_INFINITY);
        assert!(fast_atof(b"x").is_err());
        assert!(fast_atof(b"1e").is_err());
    }
}
