//! `LineSplitter`: a `StreamReader`'s bytes read a line at a time. A line
//! ends at '\r' or '\n'. Skipping empty lines passes every following
//! space, '\r' and '\n'; otherwise a "\r\n" pair is passed as one break,
//! and trimming passes the next line's leading spaces and tabs. Either
//! way the scan steps back onto the byte that stopped it only when bytes
//! remain after it, so a file's last byte is lost when it stops a scan.
//!
//! Token scans stop skipping blanks at `getEnd()`: a pointer 1024 bytes
//! into the line's buffer (reserved for 1039 bytes), which a line any
//! longer moves for good.

use crate::parsing::{at, is_line_end, is_space, skip_spaces};
use crate::scene::{ImportError, fail};

/// `std::string::reserve(1024)`'s capacity (MSVC and libc++ alike).
const LINE_CAPACITY: usize = 1039;

/// Where `getEnd()` points into the line's first buffer.
const END_OFFSET: usize = 1024;

pub(crate) struct LineSplitter<'a> {
    bytes: &'a [u8],
    position: usize,
    current: Vec<u8>,
    swallow: bool,
    skip_empty_lines: bool,
    trim: bool,
    /// Whether a line outgrew the first buffer, leaving `getEnd()` behind.
    buffer_moved: bool,
}

impl<'a> LineSplitter<'a> {
    /// The constructor: the first line read (`bytes` are a
    /// `StreamReader`'s, which is never empty).
    pub fn new(bytes: &'a [u8], skip_empty_lines: bool, trim: bool) -> Self {
        let mut splitter = Self { bytes, position: 0, current: Vec::new(), swallow: false, skip_empty_lines, trim, buffer_moved: false };
        splitter.advance();
        splitter
    }

    /// The constructor as it throws (`logic_error`) on an empty stream.
    pub fn begin(bytes: &'a [u8], skip_empty_lines: bool, trim: bool) -> Result<Self, ImportError> {
        if bytes.is_empty() {
            return fail("End of file, no more lines to be retrieved.");
        }
        Ok(Self::new(bytes, skip_empty_lines, trim))
    }

    /// The bytes left in the stream (`GetRemainingSize`).
    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    fn next_byte(&mut self) -> u8 {
        let byte = self.bytes[self.position];
        self.position += 1;
        byte
    }

    /// `operator bool`: whether bytes remain.
    pub fn is_valid(&self) -> bool {
        self.remaining() > 0
    }

    /// The current line (`operator*`), without its break.
    pub fn current(&self) -> &[u8] {
        &self.current
    }

    /// `operator++`: the next line; false where it throws `logic_error`
    /// (no bytes remain).
    pub fn advance(&mut self) -> bool {
        if self.swallow {
            self.swallow = false;
            return true;
        }
        if !self.is_valid() {
            return false;
        }
        self.current.clear();
        while self.is_valid() {
            let byte = self.next_byte();
            if byte != b'\n' && byte != b'\r' {
                self.current.push(byte);
                continue;
            }
            if self.skip_empty_lines {
                while self.is_valid() {
                    let next = self.next_byte();
                    if next != b' ' && next != b'\r' && next != b'\n' {
                        break;
                    }
                }
                if self.is_valid() {
                    self.position -= 1;
                }
            } else {
                if self.is_valid() && byte == b'\r' && self.next_byte() != b'\n' {
                    self.position -= 1;
                }
                if self.trim {
                    while self.is_valid() {
                        let next = self.next_byte();
                        if next != b' ' && next != b'\t' {
                            break;
                        }
                    }
                    if self.is_valid() {
                        self.position -= 1;
                    }
                }
            }
            break;
        }
        if self.current.len() > LINE_CAPACITY {
            self.buffer_moved = true;
        }
        true
    }

    /// `operator++` as it throws `logic_error` once no bytes remain.
    pub fn next_line(&mut self) -> Result<(), ImportError> {
        if !self.advance() {
            return fail("End of file, no more lines to be retrieved.");
        }
        Ok(())
    }

    /// `swallow_next_increment`.
    pub fn swallow_next_increment(&mut self) {
        self.swallow = true;
    }

    /// `get_stream().IncPtr(plus)`.
    pub fn inc_ptr(&mut self, plus: u64) -> Result<(), ImportError> {
        let target = self.position as u64 + plus;
        if target > self.bytes.len() as u64 {
            return fail("End of file or read limit was reached");
        }
        self.position = target as usize;
        Ok(())
    }

    /// `getEnd()` as an offset into the current line, `usize::MAX` once it
    /// points into a buffer the line no longer uses.
    pub fn end_bound(&self) -> usize {
        if self.buffer_moved {
            return usize::MAX;
        }
        END_OFFSET
    }

    /// `match_start`: the line starts with `check`.
    pub fn match_start(&self, check: &[u8]) -> bool {
        self.current.starts_with(check)
    }

    /// `operator[]`: where token `index` starts in the current line.
    pub fn token(&self, index: usize) -> Result<usize, ImportError> {
        let line = &self.current;
        let bound = self.end_bound();
        let (mut position, _) = skip_spaces(line, 0, bound);
        for _ in 0..index {
            while !is_space(at(line, position)) {
                if is_line_end(at(line, position)) {
                    return fail("Token index out of range, EOL reached");
                }
                position += 1;
            }
            position = skip_spaces(line, position, bound).0;
        }
        Ok(position)
    }

    /// `get_tokens`: where each of the first `N` tokens starts.
    pub fn tokens<const N: usize>(&self) -> Result<[usize; N], ImportError> {
        let line = &self.current;
        let bound = self.end_bound();
        let (mut position, _) = skip_spaces(line, 0, bound);
        let mut tokens = [0usize; N];
        for token in &mut tokens {
            if is_line_end(at(line, position)) {
                return fail("Token count out of range, EOL reached");
            }
            *token = position;
            while at(line, position) != 0 && !is_space(at(line, position)) {
                position += 1;
            }
            position = skip_spaces(line, position, bound).0;
        }
        Ok(tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(bytes: &[u8], skip_empty_lines: bool) -> Vec<Vec<u8>> {
        let mut splitter = LineSplitter::new(bytes, skip_empty_lines, true);
        let mut found = vec![splitter.current().to_vec()];
        while splitter.advance() {
            found.push(splitter.current().to_vec());
        }
        found
    }

    #[test]
    fn breaks_and_trimming() {
        // The trim after "c\r" eats the final "d".
        assert_eq!(lines(b"a\r\n  b\n\n\tc\rd", false), vec![b"a".to_vec(), b"b".to_vec(), b"".to_vec(), b"c".to_vec()]);
        assert_eq!(lines(b"a\r\n  b\n\n\tc\rd\n", false), vec![b"a".to_vec(), b"b".to_vec(), b"".to_vec(), b"c".to_vec(), b"d".to_vec()]);
        assert_eq!(lines(b"a\n\n \nb\n", true), vec![b"a".to_vec(), b"b".to_vec()]);
        // A trimmed break's scan eats the final byte it stops on.
        assert_eq!(lines(b"a\n x", false), vec![b"a".to_vec()]);
    }
}
