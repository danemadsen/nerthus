//! LibRaw's file reads (`utils/read_utils.cpp`) over the bytes of a file:
//! a position that may run past the end (reads there come back empty,
//! `get2`/`get4` then give their 0xff fill) and the byte order of the
//! TIFF structure being read (`order`).

/// `order` for little-endian ("II") data.
pub(super) const LITTLE_ENDIAN: u16 = 0x4949;
/// `order` for big-endian ("MM") data.
pub(super) const BIG_ENDIAN: u16 = 0x4d4d;

// LIBRAW_EXIFTAG_TYPE_*
pub(super) const TYPE_SHORT: u32 = 3;
pub(super) const TYPE_LONG: u32 = 4;
pub(super) const TYPE_RATIONAL: u32 = 5;
pub(super) const TYPE_SSHORT: u32 = 8;
pub(super) const TYPE_SLONG: u32 = 9;
pub(super) const TYPE_SRATIONAL: u32 = 10;
pub(super) const TYPE_FLOAT: u32 = 11;
pub(super) const TYPE_DOUBLE: u32 = 12;
pub(super) const TYPE_IFD8: u32 = 18;

/// `tagtype_dataunit_bytes`.
pub(super) const TYPE_SIZES: [u32; 20] = [1, 1, 1, 2, 4, 8, 1, 1, 2, 4, 8, 4, 8, 4, 2, 8, 8, 8, 8, 8];

pub(super) struct Stream<'a> {
    data: &'a [u8],
    position: u64,
    pub order: u16,
}

impl<'a> Stream<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Stream { data, position: 0, order: LITTLE_ENDIAN }
    }

    /// The whole file.
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    pub fn size(&self) -> u64 {
        self.data.len() as u64
    }

    pub fn tell(&self) -> u64 {
        self.position
    }

    pub fn seek(&mut self, position: u64) {
        self.position = position;
    }

    pub fn skip(&mut self, count: i64) {
        self.position = self.position.saturating_add_signed(count);
    }

    pub fn eof(&self) -> bool {
        self.position >= self.size()
    }

    /// `fgetc`: the next byte, or `None` at the end of the file.
    pub fn getc(&mut self) -> Option<u8> {
        let byte = self.data.get(self.position as usize).copied();
        if byte.is_some() {
            self.position += 1;
        }
        byte
    }

    /// `fread`: copies what is left of `buffer.len()` bytes, returning how
    /// many were read.
    pub fn read(&mut self, buffer: &mut [u8]) -> usize {
        let start = (self.position as usize).min(self.data.len());
        let count = buffer.len().min(self.data.len() - start);
        buffer[..count].copy_from_slice(&self.data[start..start + count]);
        self.position += count as u64;
        count
    }

    pub fn sget2(&self, bytes: [u8; 2]) -> u16 {
        if self.order == LITTLE_ENDIAN {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        }
    }

    pub fn sget4(&self, bytes: [u8; 4]) -> u32 {
        if self.order == LITTLE_ENDIAN {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        }
    }

    pub fn get2(&mut self) -> u16 {
        let mut bytes = [0xff; 2];
        self.read(&mut bytes);
        self.sget2(bytes)
    }

    pub fn get4(&mut self) -> u32 {
        let mut bytes = [0xff; 4];
        self.read(&mut bytes);
        self.sget4(bytes)
    }

    pub fn getint(&mut self, kind: u32) -> u32 {
        if kind == TYPE_SHORT {
            self.get2() as u32
        } else {
            self.get4()
        }
    }

    pub fn getreal(&mut self, kind: u32) -> f64 {
        match kind {
            TYPE_SHORT => self.get2() as f64,
            TYPE_LONG => self.get4() as f64,
            TYPE_RATIONAL => {
                let numerator = self.get4() as f64;
                let denominator = self.get4() as f64;
                numerator / if denominator != 0.0 { denominator } else { 1.0 }
            }
            TYPE_SSHORT => self.get2() as i16 as f64,
            TYPE_SLONG => self.get4() as i32 as f64,
            TYPE_SRATIONAL => {
                let numerator = self.get4() as i32 as f64;
                let denominator = self.get4() as i32 as f64;
                numerator / if denominator != 0.0 { denominator } else { 1.0 }
            }
            TYPE_FLOAT => f32::from_bits(self.get4()) as f64,
            TYPE_DOUBLE => {
                // Bytes past the end read as EOF (0xff).
                let mut bytes = [0u8; 8];
                for byte in bytes.iter_mut() {
                    *byte = self.getc().unwrap_or(0xff);
                }
                if self.order == LITTLE_ENDIAN {
                    f64::from_le_bytes(bytes)
                } else {
                    f64::from_be_bytes(bytes)
                }
            }
            _ => self.getc().map_or(-1.0, |byte| byte as f64),
        }
    }

    pub fn getrealf(&mut self, kind: u32) -> f32 {
        self.getreal(kind) as f32
    }

    /// `read_shorts`: `count` 16-bit values in the file's byte order; a
    /// short read leaves the rest of `values` as it was.
    pub fn read_shorts(&mut self, values: &mut [u16]) {
        let mut bytes = vec![0u8; values.len() * 2];
        let count = self.read(&mut bytes) / 2;
        let (pairs, _) = bytes.as_chunks::<2>();
        for (value, &pair) in values.iter_mut().zip(pairs).take(count) {
            *value = self.sget2(pair);
        }
    }

    /// `fgets(buffer, 64, ifp)` as a string: up to 63 bytes, ending at a
    /// newline or NUL.
    pub fn read_string(&mut self, limit: usize) -> String {
        let mut text = Vec::new();
        while text.len() + 1 < limit {
            let Some(byte) = self.getc() else {
                break;
            };
            if byte == b'\n' {
                break;
            }
            text.push(byte);
        }
        if let Some(end) = text.iter().position(|&byte| byte == 0) {
            text.truncate(end);
        }
        String::from_utf8_lossy(&text).into_owned()
    }
}
