//! A file as ImageMagick's readers see it (`BlobInfo`): reads that run past
//! the end come up short and set end of file, which a seek back into the
//! file clears; the number readers give zero when cut short.

pub(crate) struct Blob<'a> {
    bytes: &'a [u8],
    offset: usize,
    /// `EOFBlob`: a read went past the end.
    pub eof: bool,
}

impl<'a> Blob<'a> {
    pub fn new(bytes: &'a [u8]) -> Blob<'a> {
        Blob { bytes, offset: 0, eof: false }
    }

    /// `GetBlobSize`.
    pub fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// `TellBlob`.
    pub fn tell(&self) -> u64 {
        self.offset as u64
    }

    /// `ReadBlob`: up to `length` bytes.
    pub fn read(&mut self, length: usize) -> &'a [u8] {
        if self.offset >= self.bytes.len() {
            self.eof = true;
            return &[];
        }
        let end = self.offset.saturating_add(length).min(self.bytes.len());
        let read = &self.bytes[self.offset..end];
        self.offset = end;
        if read.len() != length {
            self.eof = true;
        }
        read
    }

    /// `ReadBlobByte`: none at the end.
    pub fn byte(&mut self) -> Option<u8> {
        if self.offset >= self.bytes.len() {
            self.eof = true;
            return None;
        }
        let byte = self.bytes[self.offset];
        self.offset += 1;
        Some(byte)
    }

    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        let bytes = self.read(N);
        if bytes.len() != N {
            return None;
        }
        let mut array = [0u8; N];
        array.copy_from_slice(bytes);
        Some(array)
    }

    /// `ReadBlobLSBShort`.
    pub fn lsb_u16(&mut self) -> u16 {
        self.array::<2>().map_or(0, u16::from_le_bytes)
    }

    /// `ReadBlobLSBLong`.
    pub fn lsb_u32(&mut self) -> u32 {
        self.array::<4>().map_or(0, u32::from_le_bytes)
    }

    /// `ReadBlobMSBShort`.
    pub fn msb_u16(&mut self) -> u16 {
        self.array::<2>().map_or(0, u16::from_be_bytes)
    }

    /// `ReadBlobMSBLong`.
    pub fn msb_u32(&mut self) -> u32 {
        self.array::<4>().map_or(0, u32::from_be_bytes)
    }

    /// `SeekBlob(SEEK_SET)`: `None` for a negative offset.
    pub fn seek(&mut self, offset: i64) -> Option<u64> {
        if offset < 0 {
            return None;
        }
        self.offset = usize::try_from(offset).unwrap_or(usize::MAX);
        if self.offset < self.bytes.len() {
            self.eof = false;
        }
        Some(offset as u64)
    }
}
