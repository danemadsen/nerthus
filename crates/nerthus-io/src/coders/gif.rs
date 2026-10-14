//! GIF as ImageMagick reads it (`coders/gif.c`): the first frame, at its
//! own size, as a `PseudoClass` image — red, green, blue, alpha when a
//! graphic control extension names a transparent index, and the index —
//! decoded by IM's own LZW decoder. The colour space is sRGB unless an
//! "ImageMagick" application extension set the gamma to 1, when an
//! all-grey palette makes it GRAY and any other linear RGB.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use crate::pixel_cache::{CacheColorspace, DEFAULT_GAMMA, PixelCache, PixelChannel, char_quantum};

const MAXIMUM_LZW_BITS: usize = 12;
const MAXIMUM_LZW_CODE: usize = 1 << MAXIMUM_LZW_BITS;

/// `ReadBlobBlock`: one data sub-block into `data`; 0 at the terminator or
/// when the file runs out.
fn read_block(blob: &mut Blob, data: &mut [u8]) -> usize {
    let count = blob.read(1);
    if count.len() != 1 {
        return 0;
    }
    let length = count[0] as usize;
    let block = blob.read(length);
    if block.len() != length {
        return 0;
    }
    data[..length].copy_from_slice(block);
    length
}

/// The LZW decoder's state (`LZWInfo`).
struct Lzw {
    data_size: usize,
    maximum_data_value: usize,
    clear_code: usize,
    end_code: usize,
    bits: usize,
    first_code: usize,
    last_code: usize,
    maximum_code: usize,
    slot: usize,
    genesis: bool,
    prefix: Vec<usize>,
    suffix: Vec<usize>,
    stack: Vec<usize>,
    buffer: [u8; 280],
    count: usize,
    bit: usize,
    eof: bool,
}

impl Lzw {
    fn new(data_size: usize) -> Self {
        let maximum_data_value = (1 << data_size) - 1;
        let mut lzw = Self {
            data_size,
            maximum_data_value,
            clear_code: maximum_data_value + 1,
            end_code: maximum_data_value + 2,
            bits: 0,
            first_code: 0,
            last_code: 0,
            maximum_code: 0,
            slot: 0,
            genesis: true,
            prefix: vec![0; MAXIMUM_LZW_CODE],
            suffix: vec![0; MAXIMUM_LZW_CODE],
            stack: Vec::with_capacity(2 * MAXIMUM_LZW_CODE),
            buffer: [0; 280],
            count: 2,
            bit: 16,
            eof: false,
        };
        for value in 0..=maximum_data_value {
            lzw.suffix[value] = value;
        }
        lzw.reset();
        lzw
    }

    fn reset(&mut self) {
        self.bits = self.data_size + 1;
        self.maximum_code = 1 << self.bits;
        self.slot = self.maximum_data_value + 3;
        self.genesis = true;
    }

    /// `GetNextLZWCode`: -1 when the data has run out.
    fn next_code(&mut self, blob: &mut Blob, bits: usize) -> i64 {
        while self.bit + bits > 8 * self.count && !self.eof {
            self.buffer[0] = self.buffer[self.count - 2];
            self.buffer[1] = self.buffer[self.count - 1];
            self.bit -= 8 * (self.count - 2);
            self.count = 2;
            let mut block = [0u8; 256];
            let read = read_block(blob, &mut block);
            if read > 0 {
                self.buffer[2..2 + read].copy_from_slice(&block[..read]);
                self.count += read;
            } else {
                self.eof = true;
            }
        }
        if self.bit + bits > 8 * self.count {
            return -1;
        }
        let mut code = 0i64;
        for i in 0..bits {
            if self.buffer[self.bit / 8] & (1 << (self.bit % 8)) != 0 {
                code |= 1 << i;
            }
            self.bit += 1;
        }
        code
    }

    fn push(&mut self, value: usize) {
        if self.stack.len() < 2 * MAXIMUM_LZW_CODE {
            self.stack.push(value);
        }
    }

    fn pop(&mut self) -> i64 {
        self.stack.pop().map_or(-1, |value| value as i64)
    }

    /// `ReadBlobLZWByte`: the next index, or a negative value at the end.
    fn next_byte(&mut self, blob: &mut Blob) -> i64 {
        if !self.stack.is_empty() {
            return self.pop();
        }
        if self.genesis {
            self.genesis = false;
            loop {
                self.first_code = self.next_code(blob, self.bits) as usize;
                self.last_code = self.first_code;
                if self.first_code != self.clear_code {
                    break;
                }
            }
            return self.first_code as i64 as i32 as i64;
        }
        let code = self.next_code(blob, self.bits);
        if code < 0 {
            return code;
        }
        let code = code as usize;
        if code == self.clear_code {
            self.reset();
            return self.next_byte(blob);
        }
        if code == self.end_code {
            return -1;
        }
        let mut value = if code < self.slot {
            code
        } else {
            self.push(self.first_code);
            self.last_code
        };
        let mut count = 0usize;
        while value > self.maximum_data_value {
            if count > MAXIMUM_LZW_CODE {
                return -1;
            }
            count += 1;
            if value > MAXIMUM_LZW_CODE {
                return -1;
            }
            self.push(self.suffix[value]);
            value = self.prefix[value];
        }
        self.first_code = self.suffix[value];
        self.push(self.first_code);
        if self.slot < MAXIMUM_LZW_CODE {
            self.prefix[self.slot] = self.last_code;
            self.suffix[self.slot] = self.first_code;
            self.slot += 1;
            if self.slot >= self.maximum_code && self.bits < MAXIMUM_LZW_BITS {
                self.bits += 1;
                self.maximum_code = 1 << self.bits;
            }
        }
        self.last_code = code;
        self.pop()
    }
}

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut blob = Blob::new(&bytes);
    let header = blob.read(6);
    if header.len() != 6 || !(header[..5].eq_ignore_ascii_case(b"GIF87") || header[..5].eq_ignore_ascii_case(b"GIF89")) {
        bail!("ImproperImageHeader");
    }
    blob.lsb_u16();
    blob.lsb_u16();
    let flag = blob.byte().unwrap_or(0xFF);
    blob.byte();
    blob.byte();
    let global_colors = 1usize << ((flag as usize & 0x07) + 1);
    let mut global_colormap = vec![0u8; 3 * global_colors.max(256) + 3];
    if flag & 0x80 != 0 {
        let entries = blob.read(3 * global_colors);
        if entries.len() != 3 * global_colors {
            bail!("InsufficientImageDataInFile");
        }
        global_colormap[..entries.len()].copy_from_slice(entries);
    }
    let mut opacity: i64 = -1;
    let mut gamma = DEFAULT_GAMMA;
    let mut buffer = [0u8; 257];
    loop {
        let c = blob.read(1);
        if c.len() != 1 {
            bail!("NegativeOrZeroImageSize");
        }
        let c = c[0];
        if c == b';' {
            bail!("NegativeOrZeroImageSize");
        }
        if c == b'!' {
            buffer.fill(0);
            let label = blob.read(1);
            if label.len() != 1 {
                bail!("UnableToReadExtensionBlock");
            }
            match label[0] {
                0xf9 => {
                    while read_block(&mut blob, &mut buffer) != 0 {}
                    if buffer[0] & 0x01 == 0x01 {
                        opacity = buffer[3] as i64;
                    }
                }
                0xff => {
                    let loop_extension = read_block(&mut blob, &mut buffer) != 0 && buffer[..11].eq_ignore_ascii_case(b"NETSCAPE2.0");
                    if loop_extension {
                        while read_block(&mut blob, &mut buffer) != 0 {}
                    } else {
                        let magick = buffer[..11].eq_ignore_ascii_case(b"ImageMagick");
                        let mut info = Vec::new();
                        let mut block = [0u8; 256];
                        loop {
                            let length = read_block(&mut blob, &mut block);
                            if length == 0 {
                                break;
                            }
                            info.extend_from_slice(&block[..length]);
                        }
                        if magick {
                            gamma = string_to_double(info.get(6..).unwrap_or(&[]));
                        }
                    }
                }
                _ => while read_block(&mut blob, &mut buffer) != 0 {},
            }
            continue;
        }
        if c != b',' {
            continue;
        }
        blob.lsb_u16();
        blob.lsb_u16();
        let columns = blob.lsb_u16() as usize;
        let rows = blob.lsb_u16() as usize;
        let flag = blob.byte().unwrap_or(0xFF);
        let interlaced = flag & 0x40 != 0;
        let local_colors = if flag & 0x80 == 0 { global_colors } else { 1 << ((flag as usize & 0x07) + 1) };
        let mut colors = local_colors;
        if opacity == colors as i64 {
            colors += 1;
        } else if opacity > colors as i64 {
            opacity = -1;
        }
        let has_alpha = opacity >= 0;
        if columns == 0 || rows == 0 {
            bail!("NegativeOrZeroImageSize");
        }
        let source = if flag & 0x80 == 0 {
            global_colormap.clone()
        } else {
            let mut local = vec![0u8; 3 * local_colors.max(256) + 3];
            let entries = blob.read(3 * local_colors);
            if entries.len() != 3 * local_colors {
                bail!("InsufficientImageDataInFile");
            }
            local[..entries.len()].copy_from_slice(entries);
            local
        };
        let colormap: Vec<[f32; 3]> = (0..colors).map(|i| [char_quantum(source[3 * i]), char_quantum(source[3 * i + 1]), char_quantum(source[3 * i + 2])]).collect();
        // `SetImageColorspace` resets the gamma: GRAY to 1/2.2, RGB to 1.
        let colorspace = if gamma == 1.0 {
            let all_gray = colormap.iter().all(|entry| (entry[0] - entry[1]).abs() < 1e-12 && (entry[1] - entry[2]).abs() < 1e-12);
            if all_gray {
                gamma = DEFAULT_GAMMA;
                CacheColorspace::Gray
            } else {
                CacheColorspace::Rgb
            }
        } else {
            CacheColorspace::SRgb
        };
        let gray = colorspace == CacheColorspace::Gray;
        let mut channels = if gray { vec![PixelChannel::Gray] } else { vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue] };
        if has_alpha {
            channels.push(PixelChannel::Alpha);
        }
        channels.push(PixelChannel::Index);
        let stride = channels.len();
        let mut data = vec![0f32; columns * rows * stride];

        let data_size = blob.byte().unwrap_or(0xFF) as usize;
        if data_size > MAXIMUM_LZW_BITS {
            bail!("CorruptImage");
        }
        let mut lzw = Lzw::new(data_size);
        let mut invalid_index = false;
        let mut pass = 0;
        let mut offset = 0usize;
        let mut y = 0;
        while y < rows {
            if offset >= rows {
                break;
            }
            let mut x = 0;
            while x < columns {
                let c = lzw.next_byte(&mut blob);
                if c < 0 {
                    break;
                }
                let index = if (c as usize) < colors {
                    c as usize
                } else {
                    invalid_index = true;
                    0
                };
                let pixel = (offset * columns + x) * stride;
                let entry = colormap[index];
                if gray {
                    data[pixel] = entry[0];
                } else {
                    data[pixel..pixel + 3].copy_from_slice(&entry);
                }
                if has_alpha {
                    data[pixel + stride - 2] = if index as i64 == opacity { 0.0 } else { 65535.0 };
                }
                data[pixel + stride - 1] = index as f32;
                x += 1;
            }
            if x < columns {
                break;
            }
            if !interlaced {
                offset += 1;
            } else {
                offset += match pass {
                    2 => 4,
                    3 => 2,
                    _ => 8,
                };
                if pass == 0 && offset >= rows {
                    pass = 1;
                    offset = 4;
                }
                if pass == 1 && offset >= rows {
                    pass = 2;
                    offset = 2;
                }
                if pass == 2 && offset >= rows {
                    pass = 3;
                    offset = 1;
                }
            }
            y += 1;
        }
        if y < rows || invalid_index {
            bail!("CorruptImage");
        }
        let mut cache = PixelCache::new(columns, rows, channels, data, colorspace, gamma);
        for (index, entry) in colormap.iter().enumerate() {
            let alpha = if index as i64 == opacity { 0.0 } else { 65535.0 };
            cache.colormap.push([entry[0] as f64, entry[1] as f64, entry[2] as f64, alpha]);
        }
        return Ok(cache);
    }
}

/// `StringToDouble` (C `strtod`) of the leading number in `text`.
fn string_to_double(text: &[u8]) -> f64 {
    let text = String::from_utf8_lossy(text);
    let trimmed = text.trim_start();
    let mut end = 0;
    let bytes = trimmed.as_bytes();
    let mut seen_digit = false;
    let mut seen_point = false;
    let mut seen_exponent = false;
    while end < bytes.len() {
        let byte = bytes[end];
        let accepted = match byte {
            b'0'..=b'9' => {
                seen_digit = true;
                true
            }
            b'+' | b'-' => end == 0 || matches!(bytes[end - 1], b'e' | b'E'),
            b'.' if !seen_point && !seen_exponent => {
                seen_point = true;
                true
            }
            b'e' | b'E' if seen_digit && !seen_exponent => {
                seen_exponent = true;
                true
            }
            _ => false,
        };
        if !accepted {
            break;
        }
        end += 1;
    }
    let mut candidate = &trimmed[..end];
    while !candidate.is_empty() {
        if let Ok(value) = candidate.parse::<f64>() {
            return value;
        }
        candidate = &candidate[..candidate.len() - 1];
    }
    0.0
}
