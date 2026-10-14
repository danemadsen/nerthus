//! OpenEXR as ImageMagick reads it through OpenEXR's core library
//! (`coders/exr.c`): the first part's data window as the image, sRGB with
//! gamma 1 — GRAY when its only channel is luminance — and each channel
//! taken by the first letter of its name, after a prefix ending in `.`
//! that every channel shares: R, G, B and A, Y for the grey (red) channel,
//! anything else a meta channel. Half and float samples become
//! `QuantumRange · value`, unclamped. Colour channels the file lacks keep
//! what the pixel cache held, which is zero here.
//!
//! Every chunk is decompressed before its samples are taken; DWAA and DWAB
//! chunks go through [`super::exr_dwa`], the others through the `exr`
//! crate.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use anyhow::{bail, Context, Result};
use exr::block::chunk::{Chunk, CompressedBlock};
use exr::block::UncompressedBlock;
use exr::compression::Compression;
use exr::meta::attribute::{LevelMode, SampleType};
use exr::meta::header::Header;
use exr::meta::BlockDescription;

use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel};

/// Where a file channel's samples go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Red,
    Green,
    Blue,
    Alpha,
    /// Luminance, written to the grey (red) channel.
    Gray,
    Meta(usize),
}

/// `InitializeEXRChannels`: the prefix shared by every channel (up to and
/// including the first channel's last `.`), when they all share it.
fn shared_prefix_length(names: &[String]) -> usize {
    let Some(first) = names.first() else {
        return 0;
    };
    let Some(dot) = first.rfind('.') else {
        return 0;
    };
    let prefix = &first.as_bytes()[..dot + 1];
    let shared = names.iter().all(|name| name.as_bytes().starts_with(prefix));
    if shared {
        prefix.len()
    } else {
        0
    }
}

/// Each channel's target, by the first letter after the shared prefix
/// (any case); meta channels are numbered in channel order.
fn channel_targets(names: &[String]) -> Vec<Target> {
    let prefix_length = shared_prefix_length(names);
    let mut meta_count = 0;
    let mut targets = Vec::with_capacity(names.len());
    for name in names {
        let first = name.as_bytes().get(prefix_length).copied().unwrap_or(0).to_ascii_uppercase();
        let target = match first {
            b'R' => Target::Red,
            b'G' => Target::Green,
            b'B' => Target::Blue,
            b'A' => Target::Alpha,
            b'Y' => Target::Gray,
            _ => {
                meta_count += 1;
                Target::Meta(meta_count - 1)
            }
        };
        targets.push(target);
    }
    targets
}

/// IM's `HalfToSinglePrecision`.
fn half_to_single(half: u16) -> f32 {
    let sign = ((half >> 15) & 1) as u32;
    let mut exponent = ((half >> 10) & 0x1f) as u32;
    let mut significand = (half & 0x3ff) as u32;
    let value = if exponent == 0 {
        if significand == 0 {
            sign << 31
        } else {
            while significand & 0x400 == 0 {
                significand <<= 1;
                exponent = exponent.wrapping_sub(1);
            }
            exponent = exponent.wrapping_add(1);
            significand &= !0x400;
            exponent = exponent.wrapping_add(127 - 15);
            (sign << 31) | (exponent << 23) | (significand << 13)
        }
    } else if exponent == 0x1f {
        let mut bits = (sign << 31) | 0x7f80_0000;
        if significand != 0 {
            bits |= significand << 13;
        }
        bits
    } else {
        exponent += 127 - 15;
        (sign << 31) | (exponent << 23) | (significand << 13)
    };
    f32::from_bits(value)
}

/// `ClampToQuantum(QuantumRange · value)` in an HDRI build.
fn quantum(value: f32) -> f32 {
    (65535.0 * value as f64) as f32
}

/// Whether the part has one resolution level only (`exr_get_tile_levels`
/// giving 1 × 1), and the level counts when not.
fn level_counts(header: &Header) -> (usize, usize) {
    let BlockDescription::Tiles(tiles) = &header.blocks else {
        return (1, 1);
    };
    let round_up = matches!(tiles.rounding_mode, exr::math::RoundingMode::Up);
    let levels = |size: usize| -> usize {
        // floor or ceiling of log2, plus one.
        let mut count = 0;
        let mut remaining = size;
        let mut inexact = false;
        while remaining > 1 {
            if remaining & 1 == 1 {
                inexact = true;
            }
            remaining >>= 1;
            count += 1;
        }
        if round_up && inexact {
            count += 1;
        }
        count + 1
    };
    let (width, height) = (header.layer_size.width(), header.layer_size.height());
    match tiles.level_mode {
        LevelMode::Singular => (1, 1),
        LevelMode::MipMap => {
            let count = levels(width.max(height));
            (count, count)
        }
        LevelMode::RipMap => (levels(width), levels(height)),
    }
}

fn crate_error(error: exr::error::Error) -> anyhow::Error {
    anyhow::anyhow!("CorruptImage: {error}")
}

/// Decompresses one chunk into the file's line layout: for each line, for
/// each channel, that line's samples, in native byte order.
fn decompress(chunk: Chunk, meta: &exr::meta::MetaData, header: &Header) -> Result<UncompressedBlock> {
    match header.compression {
        Compression::DWAA(_) | Compression::DWAB(_) => decompress_dwa(chunk, header),
        _ => UncompressedBlock::decompress_chunk(chunk, meta, false).map_err(crate_error),
    }
}

/// A DWAA or DWAB chunk through [`super::exr_dwa`]. A chunk as big as its
/// pixels holds them uncompressed, as with every compression but B44.
fn decompress_dwa(chunk: Chunk, header: &Header) -> Result<UncompressedBlock> {
    let tile = header.get_block_data_indices(&chunk.compressed_block).map_err(crate_error)?;
    let bounds = header.get_absolute_block_pixel_coordinates(tile).map_err(crate_error)?;
    bounds.validate(Some(header.layer_size)).map_err(crate_error)?;
    let file_bounds = header.get_block_data_window_pixel_coordinates(tile).map_err(crate_error)?;
    let compressed = match &chunk.compressed_block {
        CompressedBlock::ScanLine(block) => &block.compressed_pixels_le,
        CompressedBlock::Tile(block) => &block.compressed_pixels_le,
        _ => bail!("CorruptImage: deep data"),
    };

    let (width, height) = (bounds.size.width(), bounds.size.height());
    let mut channels = Vec::with_capacity(header.channels.list.len());
    for channel in &header.channels.list {
        let pixel_type = match channel.sample_type {
            SampleType::U32 => super::exr_dwa::PIXEL_UINT,
            SampleType::F16 => super::exr_dwa::PIXEL_HALF,
            SampleType::F32 => super::exr_dwa::PIXEL_FLOAT,
        };
        let dwa_channel = super::exr_dwa::Channel {
            name: channel.name.bytes(),
            pixel_type,
            width,
            height,
            x_samples: channel.sampling.x() as i32,
            y_samples: channel.sampling.y() as i32,
            p_linear: channel.quantize_linearly,
        };
        channels.push(dwa_channel);
    }
    let line_size: usize = channels.iter().map(|channel| if channel.pixel_type == super::exr_dwa::PIXEL_HALF { 2 } else { 4 }).sum();
    let output_size = line_size * width * height;
    let data = if compressed.len() == output_size {
        compressed.clone()
    } else {
        let area = super::exr_dwa::ChunkArea { start_y: file_bounds.position.y(), width: width as i32, height: height as i32 };
        super::exr_dwa::decompress(compressed, &channels, &area, output_size).map_err(|error| anyhow::anyhow!("CorruptImage: {error}"))?
    };
    let pixel_position = bounds.position.to_usize("block position").map_err(crate_error)?;
    let index = exr::block::BlockIndex { layer: chunk.layer_index, pixel_position, level: tile.level_index, pixel_size: bounds.size };
    Ok(UncompressedBlock { index, data })
}

pub fn read(path: &Path) -> Result<PixelCache> {
    let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let reader = exr::block::read(BufReader::new(file), false).map_err(|error| anyhow::anyhow!("UnableToOpenBlob: {error}"))?;
    let meta = reader.meta_data().clone();
    let header = meta.headers.first().context("CorruptImage")?.clone();
    let width = header.layer_size.width();
    let height = header.layer_size.height();
    // A data window one pixel wide or high is refused.
    if width < 2 || height < 2 {
        bail!("CorruptImage");
    }
    if header.deep {
        bail!("CorruptImage: deep data");
    }
    let names: Vec<String> = header.channels.list.iter().map(|channel| channel.name.to_string()).collect();
    for channel in &header.channels.list {
        if channel.sample_type == SampleType::U32 {
            bail!("Unsupported channel data type `0'");
        }
        if channel.sampling.x() != 1 || channel.sampling.y() != 1 {
            bail!("subsampled channels are not supported");
        }
    }
    let (levels_x, levels_y) = level_counts(&header);
    if levels_x != 1 || levels_y != 1 {
        bail!("Unsupported number of levels `{levels_x} {levels_y}'");
    }
    let targets = channel_targets(&names);
    let meta_count = targets.iter().filter(|target| matches!(target, Target::Meta(_))).count();
    let has_alpha = targets.contains(&Target::Alpha);
    let gray = targets.len() == 1 && targets[0] == Target::Gray;

    let mut channels = if gray { vec![PixelChannel::Gray] } else { vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue] };
    let alpha_position = channels.len();
    if has_alpha && !gray {
        channels.push(PixelChannel::Alpha);
    }
    let meta_start = channels.len();
    channels.extend(std::iter::repeat_n(PixelChannel::Meta, meta_count));
    let positions: Vec<usize> = targets
        .iter()
        .map(|target| match target {
            Target::Red | Target::Gray => 0,
            Target::Green => 1,
            Target::Blue => 2,
            Target::Alpha => alpha_position,
            Target::Meta(index) => meta_start + index,
        })
        .collect();

    let stride = channels.len();
    let mut data = vec![0.0f32; width * height * stride];
    let sizes: Vec<usize> = header.channels.list.iter().map(|channel| if channel.sample_type == SampleType::F16 { 2 } else { 4 }).collect();
    let chunks = reader.all_chunks(false).map_err(|error| anyhow::anyhow!("CorruptImage: {error}"))?;
    for chunk in chunks {
        let chunk = chunk.map_err(|error| anyhow::anyhow!("CorruptImage: {error}"))?;
        let level_zero = match &chunk.compressed_block {
            CompressedBlock::Tile(tile) => tile.coordinates.level_index.x() == 0 && tile.coordinates.level_index.y() == 0,
            _ => true,
        };
        if chunk.layer_index != 0 || !level_zero {
            continue;
        }
        let block = decompress(chunk, &meta, &header)?;
        let (x0, y0) = (block.index.pixel_position.x(), block.index.pixel_position.y());
        let (block_width, block_height) = (block.index.pixel_size.width(), block.index.pixel_size.height());
        let mut cursor = 0;
        for line in 0..block_height {
            let y = y0 + line;
            for (channel, &size) in sizes.iter().enumerate() {
                let position = positions[channel];
                for column in 0..block_width {
                    let bytes = &block.data[cursor..cursor + size];
                    cursor += size;
                    let value = if size == 2 { half_to_single(u16::from_ne_bytes([bytes[0], bytes[1]])) } else { f32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) };
                    let x = x0 + column;
                    if x < width && y < height {
                        data[(y * width + x) * stride + position] = quantum(value);
                    }
                }
            }
        }
    }
    let colorspace = if gray { CacheColorspace::Gray } else { CacheColorspace::SRgb };
    Ok(PixelCache::new(width, height, channels, data, colorspace, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn channels_map_by_their_first_letter_after_a_shared_prefix() {
        assert_eq!(channel_targets(&names(&["layer.B", "layer.G", "layer.R"])), vec![Target::Blue, Target::Green, Target::Red]);
        assert_eq!(channel_targets(&names(&["B", "layer.R", "other.G"])), vec![Target::Blue, Target::Meta(0), Target::Meta(1)]);
        assert_eq!(channel_targets(&names(&["alpha", "blue", "green", "r"])), vec![Target::Alpha, Target::Blue, Target::Green, Target::Red]);
        assert_eq!(channel_targets(&names(&["Y", "Z", "depth"])), vec![Target::Gray, Target::Meta(0), Target::Meta(1)]);
    }

    #[test]
    fn halves_convert_exactly() {
        for bits in 0..=u16::MAX {
            let ours = half_to_single(bits);
            let reference = half::f16::from_bits(bits).to_f32();
            if reference.is_nan() {
                assert!(ours.is_nan());
            } else {
                assert_eq!(ours.to_bits(), reference.to_bits(), "{bits:#x}");
            }
        }
    }
}
