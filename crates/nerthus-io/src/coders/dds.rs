//! DirectDraw Surface (`coders/dds.c`): the first image of the file, BC1
//! to BC3 (DXT1, DXT3, DXT5), BC5, BC7 and the uncompressed layouts IM
//! reads, with IM's own block decoding (its integer and float roundings,
//! and its quirks: a DXT1 image gains alpha at its first transparent
//! texel; BC5 and BC7 edge blocks take their texels in order, not by
//! position; R10G10B10A2 swaps green and blue).

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::bc7_tables::{ANCHORS, PARTITIONS};
use crate::pixel_cache::{CacheColorspace, DEFAULT_GAMMA, PixelCache, PixelChannel, char_quantum};

const DDSD_CAPS: u32 = 0x1;
const DDSD_HEIGHT: u32 = 0x2;
const DDSD_WIDTH: u32 = 0x4;
const DDSD_PIXELFORMAT: u32 = 0x1000;
const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_ALPHA: u32 = 0x2;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;
const DDPF_LUMINANCE: u32 = 0x20000;
const FOURCC_ATI2: u32 = 0x3249_5441;
const FOURCC_BC5U: u32 = 0x5535_4342;
const FOURCC_DXT1: u32 = 0x3154_5844;
const FOURCC_DXT3: u32 = 0x3354_5844;
const FOURCC_DXT5: u32 = 0x3554_5844;
const FOURCC_DX10: u32 = 0x3031_5844;
const DIMENSION_TEX2D: u32 = 3;

/// The `DXGI_FORMAT`s IM reads.
const R10G10B10A2_UNORM: u32 = 24;
const R8G8B8A8_UNORM: u32 = 28;
const R8_UNORM: u32 = 61;
const BC1_UNORM: u32 = 71;
const BC2_UNORM: u32 = 74;
const BC3_UNORM: u32 = 77;
const BC5_UNORM: u32 = 83;
const B5G6R5_UNORM: u32 = 85;
const B5G5R5A1_UNORM: u32 = 86;
const B8G8R8A8_UNORM: u32 = 87;
const B8G8R8X8_UNORM: u32 = 88;
const BC7_UNORM: u32 = 98;
const BC7_UNORM_SRGB: u32 = 99;

/// `BC7_weight2`, `3` and `4`.
const WEIGHT2: [u8; 4] = [0, 21, 43, 64];
const WEIGHT3: [u8; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
const WEIGHT4: [u8; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

/// `BC7_mode_info`: partition bits, subsets, colour and alpha precision,
/// p-bits, index and second index precision.
const BC7_MODES: [[u8; 7]; 8] = [[4, 3, 4, 0, 6, 3, 0], [6, 2, 6, 0, 2, 3, 0], [6, 3, 5, 0, 0, 2, 0], [6, 2, 7, 0, 4, 2, 0], [0, 1, 5, 6, 0, 2, 3], [0, 1, 7, 8, 0, 2, 2], [0, 1, 7, 7, 2, 4, 0], [6, 2, 5, 5, 4, 2, 0]];

/// `DDSInfo`.
struct Info {
    height: usize,
    width: usize,
    flags: u32,
    fourcc: u32,
    bit_count: u32,
    masks: [u32; 4],
    ext_format: u32,
    ext_dimension: u32,
}

impl Info {
    fn is_mask(&self, red: u32, green: u32, blue: u32, alpha: u32) -> bool {
        self.masks == [red, green, blue, alpha]
    }
}

/// Which reader the file needs (`ReadDDSImage`'s choice).
#[derive(Clone, Copy, PartialEq)]
enum Decoder {
    Dxt1,
    Dxt3,
    Dxt5,
    Bc5,
    Bc7,
    UncompressedRgb,
    UncompressedRgba,
    UncompressedAlpha,
}

/// The image as it is built: quanta for red, green, blue and alpha per
/// pixel, which channels it has, and whether it is grey.
struct Canvas {
    width: usize,
    height: usize,
    pixels: Vec<[f32; 4]>,
    alpha: bool,
    gray: bool,
}

impl Canvas {
    fn set(&mut self, x: usize, y: usize, red: f32, green: f32, blue: f32) {
        let pixel = &mut self.pixels[y * self.width + x];
        pixel[0] = red;
        pixel[1] = green;
        pixel[2] = blue;
    }

    fn set_alpha(&mut self, x: usize, y: usize, alpha: f32) {
        if self.alpha {
            self.pixels[y * self.width + x][3] = alpha;
        }
    }
}

/// Reads little-endian values in order (`ReadBlobLSB*`); reads past the
/// end give zero, as IM's do, and are noted.
struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
    ended: bool,
}

impl Reader<'_> {
    fn byte(&mut self) -> u8 {
        match self.bytes.get(self.position) {
            Some(byte) => {
                self.position += 1;
                *byte
            }
            None => {
                self.ended = true;
                0
            }
        }
    }

    fn short(&mut self) -> u16 {
        u16::from_le_bytes([self.byte(), self.byte()])
    }

    fn long(&mut self) -> u32 {
        u32::from_le_bytes([self.byte(), self.byte(), self.byte(), self.byte()])
    }

    fn block(&mut self) -> Result<[u8; 16]> {
        let slice = self.bytes.get(self.position..self.position + 16).context("UnexpectedEndOfFile")?;
        self.position += 16;
        let mut block = [0u8; 16];
        block.copy_from_slice(slice);
        Ok(block)
    }
}

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    if !bytes.starts_with(b"DDS ") {
        bail!("ImproperImageHeader");
    }
    let mut reader = Reader { bytes, position: 4, ended: false };
    let info = read_info(&mut reader)?;
    let (decoder, alpha) = choose_decoder(&info)?;

    // `SetImageBackgroundColor`: opaque white to begin with.
    let mut canvas = Canvas { width: info.width, height: info.height, pixels: vec![[65535.0; 4]; info.width * info.height], alpha, gray: false };
    match decoder {
        Decoder::Dxt1 => read_dxt1(&mut reader, &mut canvas)?,
        Decoder::Dxt3 => read_dxt3(&mut reader, &mut canvas)?,
        Decoder::Dxt5 => read_dxt5(&mut reader, &mut canvas)?,
        Decoder::Bc5 => read_bc5(&mut reader, &mut canvas)?,
        Decoder::Bc7 => read_bc7(&mut reader, &mut canvas)?,
        Decoder::UncompressedRgb => read_uncompressed_rgb(&mut reader, &info, &mut canvas)?,
        Decoder::UncompressedRgba => read_uncompressed_rgba(&mut reader, &info, &mut canvas)?,
        Decoder::UncompressedAlpha => read_uncompressed_alpha(&mut reader, &info, &mut canvas)?,
    }
    Ok(into_cache(canvas))
}

/// `ReadDDSInfo`.
fn read_info(reader: &mut Reader) -> Result<Info> {
    if reader.long() != 124 {
        bail!("ImproperImageHeader");
    }
    let flags = reader.long();
    let required = DDSD_CAPS | DDSD_WIDTH | DDSD_HEIGHT | DDSD_PIXELFORMAT;
    if flags & required != required {
        bail!("ImproperImageHeader");
    }
    let height = reader.long() as usize;
    let width = reader.long() as usize;
    let _pitch = reader.long();
    let _depth = reader.long();
    let _mipmaps = reader.long();
    reader.position += 44;
    if reader.long() != 32 {
        bail!("ImproperImageHeader");
    }
    let pixel_flags = reader.long();
    let fourcc = reader.long();
    let bit_count = reader.long();
    let masks = [reader.long(), reader.long(), reader.long(), reader.long()];
    let _caps1 = reader.long();
    let _caps2 = reader.long();
    reader.position += 12;
    let (ext_format, ext_dimension) = if pixel_flags & DDPF_FOURCC != 0 && fourcc == FOURCC_DX10 {
        let format = reader.long();
        let dimension = reader.long();
        reader.position += 12;
        (format, dimension)
    } else {
        (0, 0)
    };
    if width == 0 || height == 0 {
        bail!("ImproperImageHeader");
    }
    Ok(Info { height, width, flags: pixel_flags, fourcc, bit_count, masks, ext_format, ext_dimension })
}

/// `ReadDDSImage`'s pixel format: the reader, and whether the image has
/// alpha.
fn choose_decoder(info: &Info) -> Result<(Decoder, bool)> {
    let has_alpha_pixels = info.flags & DDPF_ALPHAPIXELS != 0;
    if info.flags & DDPF_RGB != 0 || (info.flags & DDPF_ALPHA == 0 && info.flags & DDPF_LUMINANCE != 0) {
        let decoder = if has_alpha_pixels { Decoder::UncompressedRgba } else { Decoder::UncompressedRgb };
        return Ok((decoder, has_alpha_pixels));
    }
    if info.flags & DDPF_ALPHA != 0 {
        return Ok((Decoder::UncompressedAlpha, true));
    }
    if info.flags & DDPF_FOURCC == 0 {
        bail!("ImageTypeNotSupported");
    }
    let chosen = match info.fourcc {
        FOURCC_ATI2 | FOURCC_BC5U => (Decoder::Bc5, false),
        FOURCC_DXT1 => (Decoder::Dxt1, false),
        FOURCC_DXT3 => (Decoder::Dxt3, true),
        FOURCC_DXT5 => (Decoder::Dxt5, true),
        FOURCC_DX10 => {
            if info.ext_dimension != DIMENSION_TEX2D {
                bail!("ImageTypeNotSupported");
            }
            match info.ext_format {
                R8_UNORM | B5G6R5_UNORM | B8G8R8X8_UNORM => (Decoder::UncompressedRgb, false),
                B5G5R5A1_UNORM | B8G8R8A8_UNORM | R8G8B8A8_UNORM | R10G10B10A2_UNORM => (Decoder::UncompressedRgba, true),
                BC1_UNORM => (Decoder::Dxt1, false),
                BC2_UNORM => (Decoder::Dxt3, true),
                BC3_UNORM => (Decoder::Dxt5, true),
                BC5_UNORM => (Decoder::Bc5, false),
                BC7_UNORM | BC7_UNORM_SRGB => (Decoder::Bc7, true),
                _ => bail!("ImageTypeNotSupported"),
            }
        }
        _ => bail!("ImageTypeNotSupported"),
    };
    Ok(chosen)
}

/// The pixel cache: grey or RGB, then alpha.
fn into_cache(canvas: Canvas) -> PixelCache {
    let mut channels = if canvas.gray { vec![PixelChannel::Gray] } else { vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue] };
    if canvas.alpha {
        channels.push(PixelChannel::Alpha);
    }
    let mut data = Vec::with_capacity(canvas.pixels.len() * channels.len());
    for pixel in &canvas.pixels {
        if canvas.gray {
            data.push(pixel[0]);
        } else {
            data.extend_from_slice(&pixel[..3]);
        }
        if canvas.alpha {
            data.push(pixel[3]);
        }
    }
    let colorspace = if canvas.gray { CacheColorspace::Gray } else { CacheColorspace::SRgb };
    PixelCache::new(canvas.width, canvas.height, channels, data, colorspace, DEFAULT_GAMMA)
}

/// `DDSColors`.
#[derive(Clone, Copy, Default)]
struct Colors {
    r: [u8; 4],
    g: [u8; 4],
    b: [u8; 4],
    a: [u8; 4],
}

fn c565_red(value: u16) -> u8 {
    let red = (value & 0xF800) >> 11;
    ((red << 3) | (red >> 2)) as u8
}

fn c565_green(value: u16) -> u8 {
    let green = (value & 0x07E0) >> 5;
    ((green << 2) | (green >> 4)) as u8
}

fn c565_blue(value: u16) -> u8 {
    let blue = value & 0x001F;
    ((blue << 3) | (blue >> 2)) as u8
}

/// `CalculateColors`: the block's four colours (three and transparent
/// black when `c0 <= c1` and alpha counts).
fn calculate_colors(c0: u16, c1: u16, ignore_alpha: bool) -> Colors {
    let mut colors = Colors::default();
    colors.r[0] = c565_red(c0);
    colors.g[0] = c565_green(c0);
    colors.b[0] = c565_blue(c0);
    colors.r[1] = c565_red(c1);
    colors.g[1] = c565_green(c1);
    colors.b[1] = c565_blue(c1);
    let third = |first: u8, second: u8| ((2 * first as u32 + second as u32) / 3) as u8;
    if ignore_alpha || c0 > c1 {
        colors.r[2] = third(colors.r[0], colors.r[1]);
        colors.g[2] = third(colors.g[0], colors.g[1]);
        colors.b[2] = third(colors.b[0], colors.b[1]);
        colors.r[3] = third(colors.r[1], colors.r[0]);
        colors.g[3] = third(colors.g[1], colors.g[0]);
        colors.b[3] = third(colors.b[1], colors.b[0]);
    } else {
        let half = |first: u8, second: u8| ((first as u32 + second as u32) / 2) as u8;
        colors.r[2] = half(colors.r[0], colors.r[1]);
        colors.g[2] = half(colors.g[0], colors.g[1]);
        colors.b[2] = half(colors.b[0], colors.b[1]);
        colors.a[3] = 255;
    }
    colors
}

/// The texels of block (`x`, `y`) that lie in the image, with their
/// index in the 4×4 block.
fn block_texels(canvas: &Canvas, x: usize, y: usize) -> Vec<(usize, usize, usize)> {
    let mut texels = Vec::with_capacity(16);
    for j in 0..4 {
        for i in 0..4 {
            if x + i < canvas.width && y + j < canvas.height {
                texels.push((x + i, y + j, 4 * j + i));
            }
        }
    }
    texels
}

/// `ReadDXT1Pixels` with `SetDXT1Pixels`: opacity from the colour's alpha;
/// the first transparent texel of an image without alpha gives it alpha
/// (`SetImageAlpha`, opaque everywhere) before the block is set again.
fn read_dxt1(reader: &mut Reader, canvas: &mut Canvas) -> Result<()> {
    for y in (0..canvas.height).step_by(4) {
        for x in (0..canvas.width).step_by(4) {
            let c0 = reader.short();
            let c1 = reader.short();
            let bits = reader.long();
            let colors = calculate_colors(c0, c1, false);
            if reader.ended {
                bail!("UnexpectedEndOfFile");
            }
            for (px, py, index) in block_texels(canvas, x, y) {
                let code = ((bits >> (index * 2)) & 3) as usize;
                if colors.a[code] != 0 && !canvas.alpha {
                    canvas.alpha = true;
                }
                canvas.set(px, py, char_quantum(colors.r[code]), char_quantum(colors.g[code]), char_quantum(colors.b[code]));
                let opacity = char_quantum(colors.a[code]);
                canvas.set_alpha(px, py, 65535.0 - opacity);
            }
        }
    }
    Ok(())
}

/// `ReadDXT3Pixels`: explicit 4-bit alpha (×17).
fn read_dxt3(reader: &mut Reader, canvas: &mut Canvas) -> Result<()> {
    for y in (0..canvas.height).step_by(4) {
        for x in (0..canvas.width).step_by(4) {
            let a0 = reader.long();
            let a1 = reader.long();
            let c0 = reader.short();
            let c1 = reader.short();
            let bits = reader.long();
            let colors = calculate_colors(c0, c1, true);
            if reader.ended {
                bail!("UnexpectedEndOfFile");
            }
            for (px, py, index) in block_texels(canvas, x, y) {
                let code = ((bits >> (index * 2)) & 3) as usize;
                canvas.set(px, py, char_quantum(colors.r[code]), char_quantum(colors.g[code]), char_quantum(colors.b[code]));
                let (j, i) = (index / 4, index % 4);
                let nibble = if j < 2 { (a0 >> (4 * (4 * j + i))) & 0xf } else { (a1 >> (4 * (4 * (j - 2) + i))) & 0xf };
                let alpha = (17 * nibble) as u8;
                canvas.set_alpha(px, py, char_quantum(alpha));
            }
        }
    }
    Ok(())
}

/// `ReadDXT5Pixels`: interpolated 3-bit alpha.
fn read_dxt5(reader: &mut Reader, canvas: &mut Canvas) -> Result<()> {
    for y in (0..canvas.height).step_by(4) {
        for x in (0..canvas.width).step_by(4) {
            let a0 = reader.byte() as u64;
            let a1 = reader.byte() as u64;
            let low = reader.long() as u64;
            let high = reader.short() as u64;
            let alpha_bits = low | (high << 32);
            let c0 = reader.short();
            let c1 = reader.short();
            let bits = reader.long();
            let colors = calculate_colors(c0, c1, true);
            if reader.ended {
                bail!("UnexpectedEndOfFile");
            }
            for (px, py, index) in block_texels(canvas, x, y) {
                let code = ((bits >> (index * 2)) & 3) as usize;
                canvas.set(px, py, char_quantum(colors.r[code]), char_quantum(colors.g[code]), char_quantum(colors.b[code]));
                let alpha_code = (alpha_bits >> (3 * index)) & 7;
                let alpha = match alpha_code {
                    0 => a0,
                    1 => a1,
                    _ if a0 > a1 => ((8 - alpha_code) * a0 + (alpha_code - 1) * a1) / 7,
                    6 => 0,
                    7 => 255,
                    _ => ((6 - alpha_code) * a0 + (alpha_code - 1) * a1) / 5,
                };
                canvas.set_alpha(px, py, char_quantum(alpha as u8));
            }
        }
    }
    Ok(())
}

/// `GetBit`.
fn get_bit(block: &[u8; 16], start_bit: &mut usize) -> u8 {
    let index = *start_bit >> 3;
    let base = *start_bit - (index << 3);
    *start_bit += 1;
    if index > 15 {
        return 0;
    }
    (block[index] >> base) & 1
}

/// `GetBits` (at most 8 bits, never past the block).
fn get_bits(block: &[u8; 16], start_bit: &mut usize, count: u8) -> u8 {
    let index = *start_bit >> 3;
    let base = *start_bit - (index << 3);
    if index > 15 {
        return 0;
    }
    let value = if base + count as usize > 8 {
        let first_bits = 8 - base;
        let next_bits = count as usize - first_bits;
        let next = block.get(index + 1).copied().unwrap_or(0) as u32;
        ((block[index] as u32 >> base) | ((next & ((1u32 << next_bits) - 1)) << first_bits)) as u8
    } else {
        ((block[index] as u32 >> base) & ((1u32 << count) - 1)) as u8
    };
    *start_bit += count as usize;
    value
}

/// The texels of block (`x`, `y`) as `QueueAuthenticPixels` hands them
/// out: row by row within the part of the block in the image, numbered
/// in that order (BC5 and BC7 take texel `k`'s data for the `k`th).
fn queued_texels(canvas: &Canvas, x: usize, y: usize) -> Vec<(usize, usize)> {
    let block_width = 4.min(canvas.width - x);
    let block_height = 4.min(canvas.height - y);
    let mut texels = Vec::with_capacity(16);
    for j in 0..block_height {
        for i in 0..block_width {
            texels.push((x + i, y + j));
        }
    }
    texels
}

/// `ReadBC5Pixels`: two BC4 channels, red and green, blue zero.
fn read_bc5(reader: &mut Reader, canvas: &mut Canvas) -> Result<()> {
    for y in (0..canvas.height).step_by(4) {
        for x in (0..canvas.width).step_by(4) {
            let block = reader.block()?;
            let red = bc4_palette(block[0], block[1]);
            let green = bc4_palette(block[8], block[9]);
            let mut start_red = 16;
            let mut start_green = 80;
            for (px, py) in queued_texels(canvas, x, y) {
                let red_value = red[get_bits(&block, &mut start_red, 3) as usize];
                let green_value = green[get_bits(&block, &mut start_green, 3) as usize];
                canvas.set(px, py, char_quantum(red_value), char_quantum(green_value), 0.0);
            }
        }
    }
    Ok(())
}

/// A BC4 channel's eight values, as IM works them out in float.
fn bc4_palette(first: u8, second: u8) -> [u8; 8] {
    let mut palette = [0u8; 8];
    palette[0] = first;
    palette[1] = second;
    let mode: u8 = if first > second { 6 } else { 4 };
    for step in 0..mode {
        let weighted = (mode - step) as f32 * first as f32 + (step + 1) as f32 * second as f32;
        palette[step as usize + 2] = (weighted / (mode + 1) as f32) as u8;
    }
    if mode == 4 {
        palette[6] = 0;
        palette[7] = 255;
    }
    palette
}

/// `ReadEndpoints`: a BC7 block's endpoint colours, widened to 8 bits.
fn read_endpoints(block: &[u8; 16], mode: usize, start_bit: &mut usize) -> [[u8; 6]; 4] {
    let [_, subsets, color_precision, alpha_precision, _, _, _] = BC7_MODES[mode];
    let count = subsets as usize * 2;
    let mut endpoints = [[0u8; 6]; 4];
    for channel in endpoints.iter_mut().take(3) {
        for endpoint in channel.iter_mut().take(count) {
            *endpoint = get_bits(block, start_bit, color_precision);
        }
    }
    for endpoint in endpoints[3].iter_mut().take(count) {
        *endpoint = 255;
    }
    let has_alpha = mode >= 4;
    if has_alpha {
        for endpoint in endpoints[3].iter_mut().take(count) {
            *endpoint = get_bits(block, start_bit, alpha_precision);
        }
    }
    let has_pbits = matches!(mode, 0 | 1 | 3 | 6 | 7);
    let mut color_bits = color_precision;
    let mut alpha_bits = alpha_precision;
    if has_pbits {
        for channel in endpoints.iter_mut() {
            for endpoint in channel.iter_mut().take(count) {
                *endpoint <<= 1;
            }
        }
        if mode == 1 {
            let first = get_bit(block, start_bit);
            let second = get_bit(block, start_bit);
            for channel in endpoints.iter_mut().take(3) {
                channel[0] |= first;
                channel[1] |= first;
                channel[2] |= second;
                channel[3] |= second;
            }
        } else {
            for index in 0..count {
                let bit = get_bit(block, start_bit);
                for channel in endpoints.iter_mut() {
                    channel[index] |= bit;
                }
            }
        }
        color_bits += 1;
        alpha_bits += 1;
    }
    // Widen to 8 bits, the top bits repeated below.
    for channel in endpoints.iter_mut().take(3) {
        for endpoint in channel.iter_mut().take(count) {
            let value = endpoint.wrapping_shl(8 - color_bits as u32);
            *endpoint = value | value.wrapping_shr(color_bits as u32);
        }
    }
    for endpoint in endpoints[3].iter_mut().take(count) {
        let alpha = endpoint.wrapping_shl(8u32.wrapping_sub(alpha_bits as u32));
        *endpoint = alpha | alpha.wrapping_shr(alpha_bits as u32);
    }
    if !has_alpha {
        for endpoint in endpoints[3].iter_mut().take(count) {
            *endpoint = 255;
        }
    }
    endpoints
}

/// `ReadBC7Pixels`.
fn read_bc7(reader: &mut Reader, canvas: &mut Canvas) -> Result<()> {
    for y in (0..canvas.height).step_by(4) {
        for x in (0..canvas.width).step_by(4) {
            let block = reader.block()?;
            let mut start_bit = 0;
            while start_bit <= 8 && get_bit(&block, &mut start_bit) == 0 {}
            let mode = start_bit.wrapping_sub(1);
            if mode > 7 {
                bail!("corrupt BC7 block");
            }
            let [partition_bits, subsets, _, _, _, index_precision, index2_precision] = BC7_MODES[mode];
            let mut partition = 0u8;
            if matches!(mode, 0 | 1 | 2 | 3 | 7) {
                partition = get_bits(&block, &mut start_bit, partition_bits);
                if partition > 63 {
                    bail!("corrupt BC7 block");
                }
            }
            let rotation = if mode == 4 || mode == 5 { get_bits(&block, &mut start_bit, 2) } else { 0 };
            let selector = if mode == 4 { get_bit(&block, &mut start_bit) } else { 0 };
            let endpoints = read_endpoints(&block, mode, &mut start_bit);
            let mut index_bits = index_precision;
            let mut alpha_indices = [0u8; 16];
            if mode == 4 && selector == 1 {
                index_bits = 3;
                alpha_indices[0] = get_bit(&block, &mut start_bit);
                for alpha_index in alpha_indices.iter_mut().skip(1) {
                    *alpha_index = get_bits(&block, &mut start_bit, 2);
                }
            }
            let mut subset_indices = [0u8; 16];
            let mut color_indices = [0u8; 16];
            for pixel in 0..16 {
                let subset = match subsets {
                    2 => PARTITIONS[0][partition as usize][pixel],
                    3 => PARTITIONS[1][partition as usize][pixel],
                    _ => 0,
                };
                subset_indices[pixel] = subset;
                let table = match (subset, subsets) {
                    (0, _) => 0,
                    (1, 2) => 1,
                    (1, 3) => 2,
                    _ => 3,
                };
                let anchor = ANCHORS[table][partition as usize] as usize == pixel;
                let bits = if anchor { index_bits - 1 } else { index_bits };
                color_indices[pixel] = get_bits(&block, &mut start_bit, bits);
            }
            if mode == 5 || (mode == 4 && selector == 0) {
                alpha_indices[0] = get_bits(&block, &mut start_bit, index2_precision - 1);
                for alpha_index in alpha_indices.iter_mut().skip(1) {
                    *alpha_index = get_bits(&block, &mut start_bit, index2_precision);
                }
            }
            for (texel, (px, py)) in queued_texels(canvas, x, y).into_iter().enumerate() {
                let first = 2 * subset_indices[texel] as usize;
                let second = first + 1;
                let code = color_indices[texel] as usize;
                let weight = match index_bits {
                    2 => WEIGHT2.get(code).copied().unwrap_or(64),
                    3 => WEIGHT3.get(code).copied().unwrap_or(64),
                    _ => WEIGHT4.get(code).copied().unwrap_or(64),
                } as u32;
                let blend = |channel: usize, weight: u32| (((64 - weight) * endpoints[channel][first] as u32 + weight * endpoints[channel][second] as u32 + 32) >> 6) as u8;
                let mut red = blend(0, weight);
                let mut green = blend(1, weight);
                let mut blue = blend(2, weight);
                let mut alpha = blend(3, weight);
                if mode == 4 || mode == 5 {
                    let alpha_code = alpha_indices[texel] as usize;
                    let mut alpha_weight = weight;
                    if let Some(value) = WEIGHT2.get(alpha_code) {
                        alpha_weight = *value as u32;
                    }
                    if mode == 4 && selector == 0
                        && let Some(value) = WEIGHT3.get(alpha_code)
                    {
                        alpha_weight = *value as u32;
                    }
                    alpha = blend(3, alpha_weight);
                }
                match rotation {
                    1 => std::mem::swap(&mut alpha, &mut red),
                    2 => std::mem::swap(&mut alpha, &mut green),
                    3 => std::mem::swap(&mut alpha, &mut blue),
                    _ => {}
                }
                canvas.set(px, py, char_quantum(red), char_quantum(green), char_quantum(blue));
                canvas.set_alpha(px, py, char_quantum(alpha));
            }
        }
    }
    Ok(())
}

/// `(unsigned char) ((value / steps) * 255)`.
fn scaled_byte(value: u16, steps: f64) -> u8 {
    ((value as f64 / steps) * 255.0) as u8
}

/// `ReadUncompressedRGB`: 8-bit grey (`SetImageType(GrayscaleType)`),
/// 565, or 24/32-bit RGB (in mask order, else BGR).
fn read_uncompressed_rgb(reader: &mut Reader, info: &Info, canvas: &mut Canvas) -> Result<()> {
    let gray = info.bit_count == 8 || info.ext_format == R8_UNORM;
    let is_565 = info.is_mask(0xf800, 0x07e0, 0x001f, 0);
    if !gray && info.bit_count == 16 && !is_565 {
        bail!("ImageTypeNotSupported");
    }
    canvas.gray = gray;
    let is_rgb = info.is_mask(0xff, 0xff00, 0xff_0000, 0);
    for y in 0..canvas.height {
        for x in 0..canvas.width {
            if gray {
                let value = char_quantum(reader.byte());
                canvas.set(x, y, value, value, value);
            } else if info.bit_count == 16 || info.ext_format == B5G6R5_UNORM {
                let color = reader.short();
                let red = scaled_byte(color >> 11, 31.0);
                let green = scaled_byte((color << 5) >> 10, 63.0);
                let blue = scaled_byte((color << 11) >> 11, 31.0);
                canvas.set(x, y, char_quantum(red), char_quantum(green), char_quantum(blue));
            } else {
                let (first, second, third) = (reader.byte(), reader.byte(), reader.byte());
                if is_rgb {
                    canvas.set(x, y, char_quantum(first), char_quantum(second), char_quantum(third));
                } else {
                    canvas.set(x, y, char_quantum(third), char_quantum(second), char_quantum(first));
                }
                if info.bit_count == 32 || info.ext_format == B8G8R8X8_UNORM {
                    reader.byte();
                }
            }
        }
        if reader.ended {
            bail!("UnexpectedEndOfFile");
        }
    }
    Ok(())
}

/// `ReadUncompressedAlpha`: alpha alone over the white background.
fn read_uncompressed_alpha(reader: &mut Reader, info: &Info, canvas: &mut Canvas) -> Result<()> {
    if info.bit_count != 8 {
        bail!("ImageTypeNotSupported");
    }
    for y in 0..canvas.height {
        for x in 0..canvas.width {
            let alpha = char_quantum(reader.byte());
            canvas.set_alpha(x, y, alpha);
        }
        if reader.ended {
            bail!("UnexpectedEndOfFile");
        }
    }
    Ok(())
}

/// `ReadUncompressedRGBA`: 1555, grey with alpha (`GrayscaleAlphaType`),
/// 4444, R10G10B10A2, or 32-bit RGBA (in mask order, else BGRA).
fn read_uncompressed_rgba(reader: &mut Reader, info: &Info, canvas: &mut Canvas) -> Result<()> {
    let mut alpha_bits = 0;
    if info.bit_count == 16 {
        if info.is_mask(0x7c00, 0x03e0, 0x001f, 0x8000) {
            alpha_bits = 1;
        } else if info.is_mask(0x00ff, 0x00ff, 0x00ff, 0xff00) || info.is_mask(0x00ff, 0, 0, 0xff00) {
            alpha_bits = 2;
            canvas.gray = true;
        } else if info.is_mask(0x0f00, 0x00f0, 0x000f, 0xf000) {
            alpha_bits = 4;
        } else {
            bail!("ImageTypeNotSupported");
        }
    }
    if info.ext_format == B5G5R5A1_UNORM {
        alpha_bits = 1;
    }
    let is_rgba = info.is_mask(0xff, 0xff00, 0xff_0000, 0xff0_0000);
    for y in 0..canvas.height {
        for x in 0..canvas.width {
            if info.bit_count == 16 || info.ext_format == B5G5R5A1_UNORM {
                let color = reader.short();
                match alpha_bits {
                    1 => {
                        let alpha = if color & 0x8000 != 0 { 65535.0 } else { 0.0 };
                        canvas.set_alpha(x, y, alpha);
                        let red = scaled_byte((color << 1) >> 11, 31.0);
                        let green = scaled_byte((color << 6) >> 11, 31.0);
                        let blue = scaled_byte((color << 11) >> 11, 31.0);
                        canvas.set(x, y, char_quantum(red), char_quantum(green), char_quantum(blue));
                    }
                    2 => {
                        canvas.set_alpha(x, y, char_quantum((color >> 8) as u8));
                        let gray = char_quantum(color as u8);
                        canvas.set(x, y, gray, gray, gray);
                    }
                    _ => {
                        canvas.set_alpha(x, y, char_quantum(scaled_byte(color >> 12, 15.0)));
                        let red = scaled_byte((color << 4) >> 12, 15.0);
                        let green = scaled_byte((color << 8) >> 12, 15.0);
                        let blue = scaled_byte((color << 12) >> 12, 15.0);
                        canvas.set(x, y, char_quantum(red), char_quantum(green), char_quantum(blue));
                    }
                }
            } else if info.ext_format == R10G10B10A2_UNORM {
                let pixel = reader.long();
                // `(unsigned short) (value / steps) * 65535`: the cast comes
                // before the product, so each channel is 0 or full.
                let channel = |shift: u32, mask: u32, steps: f64| ((((pixel >> shift) & mask) as f64 / steps) as u16 as u32 * 65535) as u16 as f32;
                let red = channel(0, 0x3ff, 1023.0);
                let blue = channel(10, 0x3ff, 1023.0);
                let green = channel(20, 0x3ff, 1023.0);
                canvas.set(x, y, red, green, blue);
                canvas.set_alpha(x, y, channel(30, 3, 3.0));
            } else {
                let (first, second, third, fourth) = (reader.byte(), reader.byte(), reader.byte(), reader.byte());
                if info.ext_format == R8G8B8A8_UNORM || is_rgba {
                    canvas.set(x, y, char_quantum(first), char_quantum(second), char_quantum(third));
                } else {
                    canvas.set(x, y, char_quantum(third), char_quantum(second), char_quantum(first));
                }
                canvas.set_alpha(x, y, char_quantum(fourth));
            }
        }
        if reader.ended {
            bail!("UnexpectedEndOfFile");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dxt_colours_follow_imagemagick() {
        let colors = calculate_colors(0xF800, 0x001F, false);
        assert_eq!((colors.r[0], colors.b[1]), (255, 255));
        assert_eq!((colors.r[2], colors.b[2]), (170, 85));
        let transparent = calculate_colors(0x001F, 0xF800, false);
        assert_eq!(transparent.a[3], 255);
        assert_eq!(bc4_palette(200, 100)[2], 185);
    }
}
