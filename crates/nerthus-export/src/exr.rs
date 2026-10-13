//! OpenEXR: a TinyEXR wrapper over tinyexr 1.0's
//! `SaveEXRImageToFile`, 32-bit float, no compression.
//!
//! The wrapper names one channel `Y`, three `B G R`, four `B G R A` and
//! any other count `A B C …`, and hands tinyexr the planes in reverse
//! order. That is right for three channels; with four the names do not
//! match the data (`B` holds alpha, `A` holds red), and Nerthus writes the
//! same.

use std::fs;
use std::io;
use std::path::Path;

use nerthus_engine::Map;

/// tinyexr's `TINYEXR_PIXELTYPE_FLOAT`.
const PIXEL_TYPE_FLOAT: i32 = 2;

pub fn write_exr(path: &Path, map: &Map) -> io::Result<()> {
    let bytes = exr_bytes(map);
    fs::write(path, bytes)
}

/// The wrapper's channel names.
fn channel_names(count: usize) -> Vec<String> {
    let names = match count {
        1 => "Y",
        3 => "BGR",
        4 => "BGRA",
        _ => "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
    };
    let letters: Vec<char> = names.chars().collect();
    (0..count).map(|index| letters[index % letters.len()].to_string()).collect()
}

fn attribute(header: &mut Vec<u8>, name: &str, kind: &str, data: &[u8]) {
    header.extend_from_slice(name.as_bytes());
    header.push(0);
    header.extend_from_slice(kind.as_bytes());
    header.push(0);
    header.extend_from_slice(&(data.len() as i32).to_le_bytes());
    header.extend_from_slice(data);
}

pub fn exr_bytes(map: &Map) -> Vec<u8> {
    let width = map.width();
    let height = map.height();
    let count = map.channel_count();
    let names = channel_names(count);
    // The wrapper's `images[j] = channel[count - 1 - j]`; its copy
    // (`v * (1 - 0) + 0` in double) turns -0 into +0.
    let planes: Vec<Vec<f32>> = (0..count)
        .map(|index| {
            let source = map.channel(count - 1 - index);
            source.iter().map(|&value| ((value as f64) * (1.0 - 0.0) + 0.0) as f32).collect()
        })
        .collect();

    let mut file = Vec::with_capacity(width * height * count * 4 + 512);
    file.extend_from_slice(&[0x76, 0x2f, 0x31, 0x01]);
    file.extend_from_slice(&[2, 0, 0, 0]);

    // `WriteChannelInfo`: name, pixel type, pLinear 0 and three reserved
    // bytes, x and y sampling 1, then a closing zero.
    let mut channels = Vec::new();
    for name in &names {
        channels.extend_from_slice(name.as_bytes());
        channels.push(0);
        channels.extend_from_slice(&PIXEL_TYPE_FLOAT.to_le_bytes());
        channels.extend_from_slice(&[0, 0, 0, 0]);
        channels.extend_from_slice(&1i32.to_le_bytes());
        channels.extend_from_slice(&1i32.to_le_bytes());
    }
    channels.push(0);
    attribute(&mut file, "channels", "chlist", &channels);
    attribute(&mut file, "compression", "compression", &[0]);
    let mut window = Vec::with_capacity(16);
    for value in [0, 0, width as i32 - 1, height as i32 - 1] {
        window.extend_from_slice(&value.to_le_bytes());
    }
    attribute(&mut file, "dataWindow", "box2i", &window);
    attribute(&mut file, "displayWindow", "box2i", &window);
    attribute(&mut file, "lineOrder", "lineOrder", &[0]);
    attribute(&mut file, "pixelAspectRatio", "float", &1.0f32.to_le_bytes());
    let mut center = Vec::with_capacity(8);
    center.extend_from_slice(&0.0f32.to_le_bytes());
    center.extend_from_slice(&0.0f32.to_le_bytes());
    attribute(&mut file, "screenWindowCenter", "v2f", &center);
    // tinyexr writes the image width here, not OpenEXR's usual 1.
    attribute(&mut file, "screenWindowWidth", "float", &(width as f32).to_le_bytes());
    file.push(0);

    // One scanline per block: the offset table, then each block as its
    // row number, its size and every channel's row in header order.
    let block_size = width * count * 4;
    let table_start = file.len();
    let first_block = table_start + height * 8;
    for row in 0..height {
        let offset = (first_block + row * (8 + block_size)) as u64;
        file.extend_from_slice(&offset.to_le_bytes());
    }
    for row in 0..height {
        file.extend_from_slice(&(row as i32).to_le_bytes());
        file.extend_from_slice(&(block_size as u32).to_le_bytes());
        for plane in &planes {
            for value in &plane[row * width..(row + 1) * width] {
                file.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    file
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerthus_engine::Resolution;

    #[test]
    fn names_follow_the_wrapper() {
        assert_eq!(channel_names(1), vec!["Y"]);
        assert_eq!(channel_names(3), vec!["B", "G", "R"]);
        assert_eq!(channel_names(4), vec!["B", "G", "R", "A"]);
        assert_eq!(channel_names(2), vec!["A", "B"]);
    }

    #[test]
    fn layout_is_scanline_blocks() {
        let map = Map::from_data(Resolution::new(2, 2), vec![0.0, 0.25, 0.5, 1.0]);
        let bytes = exr_bytes(&map);
        let header_end = bytes.len() - 2 * 8 - 2 * (8 + 8);
        let first_offset = u64::from_le_bytes(bytes[header_end..header_end + 8].try_into().unwrap());
        assert_eq!(first_offset as usize, header_end + 16);
        let last_row = &bytes[bytes.len() - 8..];
        assert_eq!(last_row, [0.5f32.to_le_bytes(), 1.0f32.to_le_bytes()].concat());
    }
}
