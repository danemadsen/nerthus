//! Device-independent bitmaps as metafile records carry them (a
//! `BITMAPINFO` header and colour table, then the bits): every bit depth,
//! run-length and bit-field encodings, and embedded JPEG and PNG.

/// `BI_RGB`, `BI_RLE8`, `BI_RLE4`, `BI_BITFIELDS`, `BI_JPEG`, `BI_PNG`,
/// `BI_ALPHABITFIELDS`.
const BI_RGB: u32 = 0;
const BI_RLE8: u32 = 1;
const BI_RLE4: u32 = 2;
const BI_BITFIELDS: u32 = 3;
const BI_JPEG: u32 = 4;
const BI_PNG: u32 = 5;
const BI_ALPHABITFIELDS: u32 = 6;

/// `DIB_PAL_COLORS`: the colour table holds indices into the selected
/// palette.
pub const DIB_PAL_COLORS: u32 = 1;

/// A decoded bitmap: top-down rows of straight RGBA. The fourth byte of a
/// 32-bit pixel is kept as its alpha (blits ignore it, `AlphaBlend` uses
/// it); other depths are opaque.
#[derive(Clone, Debug)]
pub struct Dib {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<[u8; 4]>,
    /// A palette image's indices (1, 4 and 8 bits), for monochrome brushes
    /// and masks, which take their colours from the device context.
    pub indices: Option<Vec<u8>>,
}

impl Dib {
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        self.pixels[y * self.width + x]
    }

    /// Whether a monochrome bitmap's pixel is set (index 1); any other
    /// bitmap's pixel counts as set unless it is black.
    pub fn bit(&self, x: usize, y: usize) -> bool {
        match &self.indices {
            Some(indices) => indices[y * self.width + x] != 0,
            None => {
                let [red, green, blue, _] = self.pixel(x, y);
                red != 0 || green != 0 || blue != 0
            }
        }
    }
}

fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    let slice = bytes.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn i32_at(bytes: &[u8], offset: usize) -> Option<i32> {
    u32_at(bytes, offset).map(|value| value as i32)
}

/// A bit field's mask: where it starts and how many values it holds.
#[derive(Clone, Copy)]
struct Field {
    mask: u32,
    shift: u32,
    maximum: u32,
}

impl Field {
    fn new(mask: u32) -> Self {
        let shift = if mask == 0 { 0 } else { mask.trailing_zeros() };
        let maximum = if mask == 0 { 0 } else { mask >> shift };
        Field { mask, shift, maximum }
    }

    fn value(&self, pixel: u32, absent: u8) -> u8 {
        if self.maximum == 0 {
            return absent;
        }
        let raw = (pixel & self.mask) >> self.shift;
        ((raw as u64 * 255 + self.maximum as u64 / 2) / self.maximum as u64) as u8
    }
}

/// Decodes `bitmap_info` (`BITMAPINFO`) and `bits`. `usage` says how the
/// colour table reads; `palette` is the selected logical palette.
pub fn decode(bitmap_info: &[u8], bits: &[u8], usage: u32, palette: &[[u8; 4]]) -> Option<Dib> {
    let header_size = u32_at(bitmap_info, 0)? as usize;
    let core = header_size == 12;
    let (width, height, bit_count, compression, colours_used) = if core {
        let width = u16_at(bitmap_info, 4)? as i64;
        let height = u16_at(bitmap_info, 6)? as i16 as i64;
        let bit_count = u16_at(bitmap_info, 10)?;
        (width, height, bit_count, BI_RGB, 0u32)
    } else {
        if header_size < 40 {
            return None;
        }
        let width = i32_at(bitmap_info, 4)? as i64;
        let height = i32_at(bitmap_info, 8)? as i64;
        let bit_count = u16_at(bitmap_info, 14)?;
        let compression = u32_at(bitmap_info, 16)?;
        let colours_used = u32_at(bitmap_info, 32)?;
        (width, height, bit_count, compression, colours_used)
    };
    if width <= 0 || height == 0 {
        return None;
    }
    let width = width as usize;
    let top_down = height < 0;
    let height = height.unsigned_abs() as usize;
    if width.checked_mul(height)? > 1 << 28 {
        return None;
    }

    if compression == BI_JPEG || compression == BI_PNG {
        return decode_embedded(bits, top_down);
    }

    // Bit-field masks: inside a V2+ header, else after a plain one.
    let mut fields = None;
    let mut table_start = header_size;
    if compression == BI_BITFIELDS || compression == BI_ALPHABITFIELDS {
        let (masks_at, count) = if header_size >= 52 {
            (40, if header_size >= 56 { 4 } else { 3 })
        } else {
            let count = if compression == BI_ALPHABITFIELDS { 4 } else { 3 };
            table_start += 4 * count;
            (header_size, count)
        };
        let red = Field::new(u32_at(bitmap_info, masks_at)?);
        let green = Field::new(u32_at(bitmap_info, masks_at + 4)?);
        let blue = Field::new(u32_at(bitmap_info, masks_at + 8)?);
        let alpha = if count == 4 { Field::new(u32_at(bitmap_info, masks_at + 12)?) } else { Field::new(0) };
        fields = Some([red, green, blue, alpha]);
    }

    // The colour table.
    let entry_size = if core { 3 } else { 4 };
    let table_length = if colours_used != 0 {
        colours_used as usize
    } else if bit_count <= 8 {
        1 << bit_count
    } else {
        0
    };
    let mut table = Vec::with_capacity(table_length.min(256));
    for index in 0..table_length.min(256) {
        let colour = if usage == DIB_PAL_COLORS {
            let entry = u16_at(bitmap_info, table_start + 2 * index).unwrap_or(0) as usize;
            palette.get(entry).copied().unwrap_or([entry as u8, entry as u8, entry as u8, 255])
        } else {
            let at = table_start + entry_size * index;
            match bitmap_info.get(at..at + 3) {
                Some(entry) => [entry[2], entry[1], entry[0], 255],
                None => [0, 0, 0, 255],
            }
        };
        table.push(colour);
    }

    let row_of = |row: usize| if top_down { row } else { height - 1 - row };
    let mut pixels = vec![[0u8, 0, 0, 255]; width * height];
    let mut indices = None;
    match (bit_count, compression) {
        (1 | 4 | 8, BI_RGB) => {
            let stride = (width * bit_count as usize).div_ceil(32) * 4;
            let mut found = vec![0u8; width * height];
            for row in 0..height {
                let Some(line) = bits.get(row * stride..row * stride + stride) else {
                    break;
                };
                let target = row_of(row) * width;
                for (x, index) in found[target..target + width].iter_mut().enumerate() {
                    *index = palette_index(line, x, bit_count);
                }
            }
            indices = Some(found);
        }
        (8, BI_RLE8) | (4, BI_RLE4) => {
            let mut found = vec![0u8; width * height];
            decode_rle(bits, width, height, bit_count == 4, &mut found);
            if !top_down {
                let mut flipped = vec![0u8; width * height];
                for row in 0..height {
                    let source = row * width;
                    let target = (height - 1 - row) * width;
                    flipped[target..target + width].copy_from_slice(&found[source..source + width]);
                }
                found = flipped;
            }
            indices = Some(found);
        }
        (16 | 32, BI_RGB | BI_BITFIELDS | BI_ALPHABITFIELDS) | (24, BI_RGB) => {
            let bytes_per_pixel = bit_count as usize / 8;
            let stride = (width * bit_count as usize).div_ceil(32) * 4;
            for row in 0..height {
                let Some(line) = bits.get(row * stride..row * stride + width * bytes_per_pixel) else {
                    break;
                };
                let target = row_of(row) * width;
                for (x, pixel) in pixels[target..target + width].iter_mut().enumerate() {
                    let at = x * bytes_per_pixel;
                    *pixel = match (bit_count, fields) {
                        (24, _) => [line[at + 2], line[at + 1], line[at], 255],
                        (32, None) => [line[at + 2], line[at + 1], line[at], line[at + 3]],
                        (16, None) => {
                            // 5-5-5.
                            let value = u16::from_le_bytes([line[at], line[at + 1]]) as u32;
                            let five = |shift: u32| (((value >> shift) & 31) * 255 / 31) as u8;
                            [five(10), five(5), five(0), 255]
                        }
                        (_, Some([red, green, blue, alpha])) => {
                            let value = if bit_count == 16 {
                                u16::from_le_bytes([line[at], line[at + 1]]) as u32
                            } else {
                                u32::from_le_bytes([line[at], line[at + 1], line[at + 2], line[at + 3]])
                            };
                            [red.value(value, 0), green.value(value, 0), blue.value(value, 0), alpha.value(value, 255)]
                        }
                        _ => [0, 0, 0, 255],
                    };
                }
            }
        }
        _ => return None,
    }
    if let Some(found) = &indices {
        for (pixel, index) in pixels.iter_mut().zip(found) {
            *pixel = table.get(*index as usize).copied().unwrap_or([0, 0, 0, 255]);
        }
    }
    Some(Dib { width, height, pixels, indices })
}

/// The index of pixel `x` of a 1, 4 or 8-bit row (the high bits first).
fn palette_index(line: &[u8], x: usize, bit_count: u16) -> u8 {
    match bit_count {
        1 => (line[x / 8] >> (7 - x % 8)) & 1,
        4 => {
            let byte = line[x / 2];
            if x.is_multiple_of(2) { byte >> 4 } else { byte & 15 }
        }
        _ => line[x],
    }
}

/// `BI_RLE8` and `BI_RLE4`, into bottom-up rows of indices.
fn decode_rle(bits: &[u8], width: usize, height: usize, four: bool, indices: &mut [u8]) {
    let mut x = 0usize;
    let mut y = 0usize;
    let mut at = 0usize;
    let mut put = |x: usize, y: usize, value: u8| {
        if x < width && y < height {
            indices[y * width + x] = value;
        }
    };
    while at + 1 < bits.len() && y < height {
        let count = bits[at] as usize;
        let value = bits[at + 1];
        at += 2;
        if count > 0 {
            // A run: the byte repeated (RLE4: its two nibbles in turn).
            for step in 0..count {
                let index = if four { if step % 2 == 0 { value >> 4 } else { value & 15 } } else { value };
                put(x, y, index);
                x += 1;
            }
            continue;
        }
        match value {
            // End of line, end of bitmap, a move.
            0 => {
                x = 0;
                y += 1;
            }
            1 => break,
            2 => {
                let (Some(&right), Some(&up)) = (bits.get(at), bits.get(at + 1)) else {
                    break;
                };
                at += 2;
                x += right as usize;
                y += up as usize;
            }
            // Literal indices, padded to a whole word.
            literal => {
                let literal = literal as usize;
                let length = if four { literal.div_ceil(2) } else { literal };
                for step in 0..literal {
                    let Some(&byte) = bits.get(at + if four { step / 2 } else { step }) else {
                        break;
                    };
                    let index = if four { if step % 2 == 0 { byte >> 4 } else { byte & 15 } } else { byte };
                    put(x, y, index);
                    x += 1;
                }
                at += length + length % 2;
            }
        }
    }
}

/// A JPEG or PNG carried whole (`BI_JPEG`, `BI_PNG`).
fn decode_embedded(bits: &[u8], top_down: bool) -> Option<Dib> {
    let decoded = image::load_from_memory(bits).ok()?;
    let rgba = decoded.to_rgba8();
    let (width, height) = (rgba.width() as usize, rgba.height() as usize);
    let mut pixels: Vec<[u8; 4]> = rgba.pixels().map(|pixel| pixel.0).collect();
    // The stream is top-down; a bottom-up header flips it.
    if !top_down {
        let mut flipped = Vec::with_capacity(pixels.len());
        for row in pixels.chunks(width).rev() {
            flipped.extend_from_slice(row);
        }
        pixels = flipped;
    }
    Some(Dib { width, height, pixels, indices: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info_header(width: i32, height: i32, bit_count: u16, compression: u32, colours: u32) -> Vec<u8> {
        let mut header = Vec::new();
        header.extend(40u32.to_le_bytes());
        header.extend(width.to_le_bytes());
        header.extend(height.to_le_bytes());
        header.extend(1u16.to_le_bytes());
        header.extend(bit_count.to_le_bytes());
        header.extend(compression.to_le_bytes());
        header.extend([0u8; 12]);
        header.extend(colours.to_le_bytes());
        header.extend(0u32.to_le_bytes());
        header
    }

    #[test]
    fn bottom_up_24_bit_rows_come_out_top_down() {
        let info = info_header(2, 2, 24, BI_RGB, 0);
        // Bottom row first, each padded to 8 bytes; pixels are B, G, R.
        let bits = [0, 0, 255, 0, 255, 0, 0, 0, 255, 0, 0, 255, 255, 255, 0, 0];
        let dib = decode(&info, &bits, 0, &[]).unwrap();
        assert_eq!(dib.pixel(0, 0), [0, 0, 255, 255]);
        assert_eq!(dib.pixel(1, 0), [255, 255, 255, 255]);
        assert_eq!(dib.pixel(0, 1), [255, 0, 0, 255]);
        assert_eq!(dib.pixel(1, 1), [0, 255, 0, 255]);
    }

    #[test]
    fn monochrome_bitmaps_keep_their_bits() {
        let mut info = info_header(3, -1, 1, BI_RGB, 2);
        info.extend([0, 0, 0, 0, 255, 255, 255, 0]);
        let bits = [0b1010_0000, 0, 0, 0];
        let dib = decode(&info, &bits, 0, &[]).unwrap();
        assert!(dib.bit(0, 0) && !dib.bit(1, 0) && dib.bit(2, 0));
        assert_eq!(dib.pixel(1, 0), [0, 0, 0, 255]);
        assert_eq!(dib.pixel(2, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn rle8_runs_literals_and_moves() {
        let mut info = info_header(4, 2, 8, BI_RLE8, 3);
        info.extend([0, 0, 0, 0, 0, 0, 255, 0, 0, 255, 0, 0]);
        // Row 0 (the bottom): two of index 1, then the literal 2, 1, 2
        // (the last past the edge; padded to a word); end of line; row 1:
        // a move right by 2, then a run of index 2.
        let bits = [2, 1, 0, 3, 2, 1, 2, 0, 0, 0, 0, 2, 2, 0, 2, 2, 0, 1];
        let dib = decode(&info, &bits, 0, &[]).unwrap();
        let indices = dib.indices.unwrap();
        assert_eq!(&indices[4..8], &[1, 1, 2, 1]);
        assert_eq!(&indices[0..4], &[0, 0, 2, 2]);
    }

    #[test]
    fn bit_fields_scale_each_channel() {
        let mut info = info_header(1, 1, 16, BI_BITFIELDS, 0);
        info.extend(0xF800u32.to_le_bytes());
        info.extend(0x07E0u32.to_le_bytes());
        info.extend(0x001Fu32.to_le_bytes());
        let bits = 0xF81Fu16.to_le_bytes();
        let dib = decode(&info, &[bits[0], bits[1], 0, 0], 0, &[]).unwrap();
        assert_eq!(dib.pixel(0, 0), [255, 0, 255, 255]);
    }
}
