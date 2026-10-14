//! `StreamReader`: a whole file read for binary parsing, little or big
//! endian. Reads past the read limit throw. The cursor is an offset from
//! the buffer's start: `IncPtr` checks only the upper limit, so it can
//! step before the start, and a read there (undefined behaviour) is an
//! error here.

use crate::scene::{ImportError, fail};

pub(crate) struct StreamReader<'a> {
    buffer: &'a [u8],
    current: i64,
    end: i64,
    limit: i64,
    big_endian: bool,
}

const PAST_THE_LIMIT: &str = "End of file or stream limit was reached";

impl<'a> StreamReader<'a> {
    /// `InternBegin`: an empty file throws.
    pub fn new(buffer: &'a [u8], big_endian: bool) -> Result<Self, ImportError> {
        if buffer.is_empty() {
            return fail("StreamReader: File is empty or EOF is already reached");
        }
        let end = buffer.len() as i64;
        Ok(Self { buffer, current: 0, end, limit: end, big_endian })
    }

    /// The bytes of a read of `size` at the cursor, which then moves on.
    fn take(&mut self, size: usize) -> Result<&'a [u8], ImportError> {
        if self.current + size as i64 > self.limit {
            return fail(PAST_THE_LIMIT);
        }
        if self.current < 0 {
            return fail("undefined behaviour: a read before the file");
        }
        let start = self.current as usize;
        self.current += size as i64;
        Ok(&self.buffer[start..start + size])
    }

    fn get<const N: usize>(&mut self) -> Result<[u8; N], ImportError> {
        let bytes = self.take(N)?;
        let mut value: [u8; N] = bytes.try_into().unwrap();
        if self.big_endian {
            value.reverse();
        }
        Ok(value)
    }

    pub fn get_u1(&mut self) -> Result<u8, ImportError> {
        Ok(self.get::<1>()?[0])
    }

    pub fn get_i1(&mut self) -> Result<i8, ImportError> {
        Ok(self.get::<1>()?[0] as i8)
    }

    pub fn get_u2(&mut self) -> Result<u16, ImportError> {
        Ok(u16::from_le_bytes(self.get()?))
    }

    pub fn get_i2(&mut self) -> Result<i16, ImportError> {
        Ok(i16::from_le_bytes(self.get()?))
    }

    pub fn get_u4(&mut self) -> Result<u32, ImportError> {
        Ok(u32::from_le_bytes(self.get()?))
    }

    pub fn get_i4(&mut self) -> Result<i32, ImportError> {
        Ok(i32::from_le_bytes(self.get()?))
    }

    pub fn get_f4(&mut self) -> Result<f32, ImportError> {
        Ok(f32::from_le_bytes(self.get()?))
    }

    pub fn get_f8(&mut self) -> Result<f64, ImportError> {
        Ok(f64::from_le_bytes(self.get()?))
    }

    /// `GetRemainingSize`: to the end, as a 32-bit count.
    pub fn remaining_size(&self) -> usize {
        (self.end - self.current) as u32 as usize
    }

    /// `GetRemainingSizeToLimit`: to the read limit, as a 32-bit count (a
    /// limit set before the cursor wraps).
    pub fn remaining_size_to_limit(&self) -> usize {
        (self.limit - self.current) as u32 as usize
    }

    /// `IncPtr`.
    pub fn inc_ptr(&mut self, plus: i64) -> Result<(), ImportError> {
        self.current += plus;
        if self.current > self.limit {
            return fail("End of file or read limit was reached");
        }
        Ok(())
    }

    /// `GetPtr`, as an offset.
    pub fn ptr(&self) -> i64 {
        self.current
    }

    /// `SetPtr`, from an offset.
    pub fn set_ptr(&mut self, position: i64) -> Result<(), ImportError> {
        self.current = position;
        if self.current > self.limit || self.current < 0 {
            return fail("End of file or read limit was reached");
        }
        Ok(())
    }

    /// `CopyAndAdvance`: the next `size` bytes.
    pub fn copy_and_advance(&mut self, size: usize) -> Result<&'a [u8], ImportError> {
        let start = self.current;
        self.set_ptr(start + size as i64)?;
        Ok(&self.buffer[start as usize..start as usize + size])
    }

    /// `GetCurrentPos`.
    pub fn current_pos(&self) -> i32 {
        self.current as u32 as i32
    }

    /// `SetCurrentPos`.
    pub fn set_current_pos(&mut self, position: usize) -> Result<(), ImportError> {
        self.set_ptr(position as i64)
    }

    /// `SetReadLimit`: the previous limit; `u32::MAX` resets it to the end.
    pub fn set_read_limit(&mut self, limit: u32) -> Result<u32, ImportError> {
        let previous = self.read_limit();
        if limit == u32::MAX {
            self.limit = self.end;
            return Ok(previous);
        }
        self.limit = limit as i64;
        if self.limit > self.end {
            return fail("StreamReader: Invalid read limit");
        }
        Ok(previous)
    }

    /// `GetReadLimit`.
    pub fn read_limit(&self) -> u32 {
        self.limit as u32
    }

    /// `SkipToReadLimit`.
    pub fn skip_to_read_limit(&mut self) {
        self.current = self.limit;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_byte_orders_and_stops_at_the_limit() {
        let bytes = [1, 2, 3, 4, 5];
        let mut little = StreamReader::new(&bytes, false).unwrap();
        assert_eq!(little.get_u2().unwrap(), 0x0201);
        let mut big = StreamReader::new(&bytes, true).unwrap();
        assert_eq!(big.get_u4().unwrap(), 0x0102_0304);
        assert!(big.get_u2().is_err());
        big.set_read_limit(2).unwrap();
        assert_eq!(big.remaining_size_to_limit(), u32::MAX as usize - 1);
        assert!(StreamReader::new(&[], false).is_err());
    }
}
