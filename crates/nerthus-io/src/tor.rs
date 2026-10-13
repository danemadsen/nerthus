//! The `.tor` container: base64 text of a little-endian length prefix
//! followed by a gzip stream of the project XML.

use std::io::{Read, Write};

use anyhow::{bail, Context, Result};
use base64::Engine as _;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;

/// Decodes a `.tor` file's bytes into XML text. Plain XML is passed through.
pub fn decode(bytes: &[u8]) -> Result<String> {
    let trimmed = trim_ascii(bytes);
    if trimmed.first() == Some(&b'<') || trimmed.starts_with(&[0xef, 0xbb, 0xbf]) {
        let text = String::from_utf8_lossy(trimmed).into_owned();
        return Ok(text.trim_start_matches('\u{feff}').to_string());
    }
    let engine = base64::engine::general_purpose::STANDARD;
    let raw = engine.decode(trimmed).context("project is not valid base64")?;
    if raw.len() < 4 {
        bail!("project data is truncated");
    }
    let expected = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
    let mut decoder = GzDecoder::new(&raw[4..]);
    let mut xml = Vec::with_capacity(expected);
    decoder.read_to_end(&mut xml).context("project data is not valid gzip")?;
    let text = String::from_utf8(xml).context("project XML is not UTF-8")?;
    Ok(text.trim_start_matches('\u{feff}').to_string())
}

/// Encodes XML text into the `.tor` container format.
pub fn encode(xml: &str) -> Result<String> {
    let bytes = xml.as_bytes();
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes)?;
    let compressed = encoder.finish()?;
    let mut payload = Vec::with_capacity(compressed.len() + 4);
    payload.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    payload.extend_from_slice(&compressed);
    let engine = base64::engine::general_purpose::STANDARD;
    Ok(engine.encode(payload))
}

fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let start = bytes.iter().position(|byte| !byte.is_ascii_whitespace()).unwrap_or(bytes.len());
    let end = bytes.iter().rposition(|byte| !byte.is_ascii_whitespace()).map(|index| index + 1).unwrap_or(start);
    &bytes[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let xml = "<?xml version=\"1.0\"?><Terrain Name=\"x\" />";
        let encoded = encode(xml).unwrap();
        let decoded = decode(encoded.as_bytes()).unwrap();
        assert_eq!(decoded, xml);
    }
}
