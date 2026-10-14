//! The xz container as liblzma's stream decoder reads it: blocks whose
//! filter chain is LZMA2, optionally after a delta filter, each checked
//! with its CRC32, CRC64 or SHA-256; and the older `.lzma` format, which
//! `lzma_auto_decoder` also takes. LZMA2 itself is `lzma-rs`'s.

use anyhow::{bail, Context, Result};
use sha2::Digest;

const MAGIC: [u8; 6] = [0xFD, b'7', b'z', b'X', b'Z', 0x00];
const FILTER_DELTA: u64 = 0x03;
const FILTER_LZMA2: u64 = 0x21;

/// A multibyte integer (`lzma_vli_decode`).
fn vli(bytes: &[u8], position: &mut usize) -> Result<u64> {
    let mut value = 0u64;
    for index in 0..9 {
        let byte = *bytes.get(*position).context("truncated xz data")?;
        *position += 1;
        value |= ((byte & 0x7F) as u64) << (7 * index);
        if byte & 0x80 == 0 {
            if index > 0 && byte == 0 {
                bail!("corrupt xz integer");
            }
            return Ok(value);
        }
    }
    bail!("corrupt xz integer")
}

/// Undoes the delta filter in place: each byte plus the one `distance`
/// before it.
fn undo_delta(data: &mut [u8], distance: usize) {
    for index in distance..data.len() {
        data[index] = data[index].wrapping_add(data[index - distance]);
    }
}

/// Decompresses `.xz` data (its first stream) or, without the xz magic,
/// `.lzma` data.
pub fn decompress_auto(data: &[u8]) -> Result<Vec<u8>> {
    if data.starts_with(&MAGIC) {
        return decompress(data);
    }
    let mut output = Vec::new();
    let mut input = std::io::BufReader::new(data);
    lzma_rs::lzma_decompress(&mut input, &mut output).map_err(|error| anyhow::anyhow!("corrupt LZMA data: {error:?}"))?;
    Ok(output)
}

/// Decompresses the first stream of `.xz` data.
pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 12 || data[..6] != MAGIC {
        bail!("not xz data");
    }
    let flags = &data[6..8];
    let stored = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    if crc32fast::hash(flags) != stored || flags[0] != 0 || flags[1] & 0xF0 != 0 {
        bail!("corrupt xz stream header");
    }
    let check = flags[1] & 0x0F;
    let check_size = match check {
        0 => 0,
        1..=3 => 4,
        4..=6 => 8,
        7..=9 => 16,
        10..=12 => 32,
        _ => 64,
    };
    let mut position = 12;
    let mut output = Vec::new();
    loop {
        let size_byte = *data.get(position).context("truncated xz data")?;
        if size_byte == 0 {
            // The index follows: the blocks are done.
            break;
        }
        let header_size = (size_byte as usize + 1) * 4;
        let header = data.get(position..position + header_size).context("truncated xz data")?;
        let stored = u32::from_le_bytes([header[header_size - 4], header[header_size - 3], header[header_size - 2], header[header_size - 1]]);
        if crc32fast::hash(&header[..header_size - 4]) != stored {
            bail!("corrupt xz block header");
        }
        let block_flags = header[1];
        let filter_count = (block_flags & 0x03) as usize + 1;
        let mut cursor = 2;
        let compressed_size = if block_flags & 0x40 != 0 { Some(vli(header, &mut cursor)?) } else { None };
        let uncompressed_size = if block_flags & 0x80 != 0 { Some(vli(header, &mut cursor)?) } else { None };
        let mut delta = None;
        let mut lzma2 = false;
        for filter in 0..filter_count {
            let id = vli(header, &mut cursor)?;
            let properties_size = vli(header, &mut cursor)? as usize;
            let properties = header.get(cursor..cursor + properties_size).context("corrupt xz block header")?;
            cursor += properties_size;
            match id {
                FILTER_DELTA if filter + 1 < filter_count && properties_size == 1 => delta = Some(properties[0] as usize + 1),
                FILTER_LZMA2 if filter + 1 == filter_count && properties_size == 1 => lzma2 = true,
                _ => bail!("unsupported xz filter 0x{id:x}"),
            }
        }
        if !lzma2 {
            bail!("unsupported xz filter chain");
        }
        position += header_size;
        let block_start = position;
        let mut input = std::io::BufReader::new(&data[position..]);
        let mut block = Vec::new();
        lzma_rs::lzma2_decompress(&mut input, &mut block).map_err(|error| anyhow::anyhow!("corrupt xz data: {error:?}"))?;
        let consumed = lzma2_length(&data[position..])?;
        position += consumed;
        if let Some(size) = compressed_size
            && size as usize != position - block_start
        {
            bail!("corrupt xz block size");
        }
        if let Some(size) = uncompressed_size
            && size as usize != block.len()
        {
            bail!("corrupt xz block size");
        }
        if let Some(distance) = delta {
            undo_delta(&mut block, distance);
        }
        while !(position - block_start).is_multiple_of(4) {
            if data.get(position).copied() != Some(0) {
                bail!("corrupt xz block padding");
            }
            position += 1;
        }
        let stored = data.get(position..position + check_size).context("truncated xz data")?;
        let verified = match check {
            0 => true,
            1 => crc32fast::hash(&block).to_le_bytes() == stored,
            4 => crc::Crc::<u64>::new(&crc::CRC_64_XZ).checksum(&block).to_le_bytes() == stored,
            10 => sha2::Sha256::digest(&block).as_slice() == stored,
            // Checks liblzma does not know are skipped unverified.
            _ => true,
        };
        if !verified {
            bail!("corrupt xz data: check failed");
        }
        position += check_size;
        output.extend_from_slice(&block);
    }
    Ok(output)
}

/// The length of a raw LZMA2 stream: its chunks up to the end marker.
fn lzma2_length(data: &[u8]) -> Result<usize> {
    let mut position = 0;
    loop {
        let control = *data.get(position).context("truncated xz data")?;
        if control == 0x00 {
            return Ok(position + 1);
        }
        if control == 0x01 || control == 0x02 {
            let size = u16::from_be_bytes([*data.get(position + 1).context("truncated xz data")?, *data.get(position + 2).context("truncated xz data")?]) as usize + 1;
            position += 3 + size;
            continue;
        }
        if control < 0x80 {
            bail!("corrupt LZMA2 data");
        }
        let packed = u16::from_be_bytes([*data.get(position + 3).context("truncated xz data")?, *data.get(position + 4).context("truncated xz data")?]) as usize + 1;
        let header = if control >= 0xC0 { 6 } else { 5 };
        position += header + packed;
    }
}
