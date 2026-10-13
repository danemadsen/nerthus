//! `Assimp::Base64::Decode`.

use crate::scene::{ImportError, fail};

const DECODE: [u8; 128] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, //
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, //
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 62, 0, 0, 0, 63, //
    52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 0, 0, 0, 64, 0, 0, //
    0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, //
    15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 0, 0, 0, 0, 0, //
    0, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, //
    41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 0, 0, 0, 0, 0,
];

fn decode_char(character: u8) -> Result<u32, ImportError> {
    if character & 0x80 != 0 {
        return fail(format!("Invalid base64 char value: {character}"));
    }
    Ok(DECODE[character as usize] as u32)
}

/// Groups of four characters; `=` decodes to 64, which drops the last
/// group's trailing bytes. The output length counts the trailing `=`s.
pub fn decode(input: &[u8]) -> Result<Vec<u8>, ImportError> {
    if !input.len().is_multiple_of(4) {
        return fail("Invalid base64 encoded data");
    }
    if input.len() < 4 {
        return Ok(Vec::new());
    }
    let equals = usize::from(input[input.len() - 1] == b'=') + usize::from(input[input.len() - 2] == b'=');
    let output_length = input.len() * 3 / 4 - equals;
    let mut output = vec![0u8; output_length];
    let mut written = 0usize;
    let mut put = |byte: u32| {
        if written < output.len() {
            output[written] = byte as u8;
        }
        written += 1;
    };
    let mut start = 0usize;
    while start + 4 < input.len() {
        let b0 = decode_char(input[start])?;
        let b1 = decode_char(input[start + 1])?;
        let b2 = decode_char(input[start + 2])?;
        let b3 = decode_char(input[start + 3])?;
        put((b0 << 2) | (b1 >> 4));
        put((b1 << 4) | (b2 >> 2));
        put((b2 << 6) | b3);
        start += 4;
    }
    let b0 = decode_char(input[start])?;
    let b1 = decode_char(input[start + 1])?;
    let b2 = decode_char(input[start + 2])?;
    let b3 = decode_char(input[start + 3])?;
    put((b0 << 2) | (b1 >> 4));
    if b2 < 64 {
        put((b1 << 4) | (b2 >> 2));
    }
    if b3 < 64 {
        put((b2 << 6) | b3);
    }
    Ok(output)
}
