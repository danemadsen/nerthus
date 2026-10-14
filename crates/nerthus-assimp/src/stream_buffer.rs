//! `IOStreamBuffer<char>`: the file read a cache at a time. The cache keeps
//! its first size, filled with '\n' at the start; a short last block leaves
//! the earlier block's bytes after it. Lines are copied into a caller's
//! vector resized to the block size, so bytes of earlier, longer lines stay
//! after a line's '\n'.

use crate::parsing::is_line_end;

pub(crate) struct StreamBuffer<'a> {
    data: &'a [u8],
    cache: Vec<u8>,
    cache_size: usize,
    cache_position: usize,
    file_position: usize,
}

impl<'a> StreamBuffer<'a> {
    /// The constructor and `open`: none for an empty file.
    pub fn open(data: &'a [u8], cache: usize) -> Option<Self> {
        if data.is_empty() {
            return None;
        }
        Some(Self { data, cache: vec![b'\n'; cache], cache_size: cache.min(data.len()), cache_position: 0, file_position: 0 })
    }

    fn cache_at(&self, index: usize) -> u8 {
        self.cache.get(index).copied().unwrap_or(b'\n')
    }

    /// `readNextBlock`.
    fn read_next_block(&mut self) -> bool {
        let remaining = self.data.len().saturating_sub(self.file_position);
        let read_length = self.cache_size.min(remaining);
        if read_length == 0 {
            return false;
        }
        let block = &self.data[self.file_position..self.file_position + read_length];
        self.cache[..read_length].copy_from_slice(block);
        if read_length < self.cache_size {
            self.cache_size = read_length;
        }
        self.file_position += self.cache_size;
        self.cache_position = 0;
        true
    }

    /// `getNextLine`: the next line (after any line ends) into `buffer`,
    /// ended by '\n', then the line ends after it skipped. False when the
    /// file ran out on the way.
    pub fn next_line(&mut self, buffer: &mut Vec<u8>) -> bool {
        buffer.resize(self.cache_size, 0);
        if (self.cache_position >= self.cache_size || self.file_position == 0) && !self.read_next_block() {
            return false;
        }
        if is_line_end(self.cache_at(self.cache_position)) {
            while self.cache_at(self.cache_position) != b'\n' {
                self.cache_position += 1;
            }
            self.cache_position += 1;
            if self.cache_position == self.cache_size && !self.read_next_block() {
                return false;
            }
        }
        let mut length = 0;
        while !is_line_end(self.cache_at(self.cache_position)) {
            buffer[length] = self.cache_at(self.cache_position);
            self.cache_position += 1;
            length += 1;
            if length == buffer.len() {
                let doubled = buffer.len() * 2;
                buffer.resize(doubled, 0);
            }
            if self.cache_position >= self.cache_size && !self.read_next_block() {
                return false;
            }
        }
        buffer[length] = b'\n';
        while self.cache_position < self.cache_size && matches!(self.cache_at(self.cache_position), b'\r' | b'\n') {
            self.cache_position += 1;
        }
        true
    }

    /// How many more blocks `readNextBlock` can read.
    pub fn blocks_left(&self) -> usize {
        let remaining = self.data.len().saturating_sub(self.file_position);
        remaining.div_ceil(self.cache_size.max(1))
    }

    /// `getNextBlock`: after lines were read, the rest of the cache (to its
    /// full first size); else the next block read, the whole cache.
    pub fn next_block(&mut self, buffer: &mut Vec<u8>) -> bool {
        if self.cache_position != 0 {
            let start = self.cache_position.min(self.cache.len());
            *buffer = self.cache[start..].to_vec();
            self.cache_position = 0;
            return true;
        }
        if !self.read_next_block() {
            return false;
        }
        *buffer = self.cache.clone();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_keep_stale_bytes_after_them() {
        let mut stream = StreamBuffer::open(b"longer line\nab\n", 64).unwrap();
        let mut line = Vec::new();
        assert!(stream.next_line(&mut line));
        assert_eq!(&line[..12], b"longer line\n");
        assert!(stream.next_line(&mut line));
        assert_eq!(&line[..12], b"ab\nger line\n");
    }

    #[test]
    fn a_block_runs_to_the_cache_end() {
        let mut stream = StreamBuffer::open(b"ply\n\x0a\x01", 16).unwrap();
        let mut line = Vec::new();
        assert!(stream.next_line(&mut line));
        let mut block = Vec::new();
        assert!(stream.next_block(&mut block));
        assert_eq!(block, vec![0x01, b'\n', b'\n', b'\n', b'\n', b'\n', b'\n', b'\n', b'\n', b'\n', b'\n']);
    }
}
