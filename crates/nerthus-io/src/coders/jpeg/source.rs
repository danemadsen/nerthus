//! The compressed data as the decoder sees it, and its markers.
//!
//! Bytes come in loads of up to 16 KiB; once the file is used up each
//! load is a fake EOI marker (`FF D9`) and a warning, so a truncated
//! stream ends rather than failing. The profile readers' own byte reader
//! reports the end of the data instead, when a load finds nothing left.
//!
//! libtiff's source (`tif_jpeg.c`) hands over a whole strip at once: every
//! later load is the fake EOI, and a skip past the end loads it and drops
//! the rest.

use super::JpegError;

/// The bytes one load of the source takes from the file.
const LOAD_SIZE: usize = 16384;

pub(super) const M_SOF0: u8 = 0xC0;
pub(super) const M_SOF1: u8 = 0xC1;
pub(super) const M_SOF2: u8 = 0xC2;
pub(super) const M_SOF3: u8 = 0xC3;
pub(super) const M_SOF5: u8 = 0xC5;
pub(super) const M_SOF6: u8 = 0xC6;
pub(super) const M_SOF7: u8 = 0xC7;
pub(super) const M_JPG: u8 = 0xC8;
pub(super) const M_SOF9: u8 = 0xC9;
pub(super) const M_SOF10: u8 = 0xCA;
pub(super) const M_SOF11: u8 = 0xCB;
pub(super) const M_SOF13: u8 = 0xCD;
pub(super) const M_SOF14: u8 = 0xCE;
pub(super) const M_SOF15: u8 = 0xCF;
pub(super) const M_DHT: u8 = 0xC4;
pub(super) const M_DAC: u8 = 0xCC;
pub(super) const M_RST0: u8 = 0xD0;
pub(super) const M_RST7: u8 = 0xD7;
pub(super) const M_SOI: u8 = 0xD8;
pub(super) const M_EOI: u8 = 0xD9;
pub(super) const M_SOS: u8 = 0xDA;
pub(super) const M_DQT: u8 = 0xDB;
pub(super) const M_DNL: u8 = 0xDC;
pub(super) const M_DRI: u8 = 0xDD;
pub(super) const M_APP0: u8 = 0xE0;
pub(super) const M_APP13: u8 = 0xED;
pub(super) const M_APP14: u8 = 0xEE;
pub(super) const M_APP15: u8 = 0xEF;
pub(super) const M_COM: u8 = 0xFE;
pub(super) const M_TEM: u8 = 0x01;

/// The source of compressed bytes (the reader's source manager).
pub(super) struct Source<'a> {
    file: &'a [u8],
    /// The next file byte a load takes.
    file_position: usize,
    buffer: Vec<u8>,
    /// The next byte of `buffer` to read.
    next: usize,
    start_of_blob: bool,
    /// libtiff's source: the whole data in one load.
    libtiff: bool,
    /// Warnings raised so far.
    pub warnings: usize,
}

impl<'a> Source<'a> {
    pub fn new(file: &'a [u8]) -> Self {
        Self { file, file_position: 0, buffer: Vec::new(), next: 0, start_of_blob: true, libtiff: false, warnings: 0 }
    }

    /// libtiff's source (`std_init_source` with the strip's bytes).
    pub fn new_libtiff(data: &'a [u8]) -> Self {
        Self { file: data, file_position: data.len(), buffer: data.to_vec(), next: 0, start_of_blob: false, libtiff: true, warnings: 0 }
    }

    pub fn bytes_in_buffer(&self) -> usize {
        self.buffer.len() - self.next
    }

    /// Loads the next block of the file, or a fake EOI with a warning once
    /// the file is used up. Returns whether that was the end.
    fn fill(&mut self) -> Result<bool, JpegError> {
        let count = LOAD_SIZE.min(self.file.len() - self.file_position);
        let at_end = count == 0;
        if at_end {
            if self.start_of_blob {
                return Err(JpegError::new("Input file empty"));
            }
            self.warn();
            self.buffer = vec![0xFF, M_EOI];
        } else {
            self.buffer = self.file[self.file_position..self.file_position + count].to_vec();
            self.file_position += count;
        }
        self.next = 0;
        self.start_of_blob = false;
        Ok(at_end)
    }

    pub fn warn(&mut self) {
        self.warnings += 1;
    }

    pub fn read_byte(&mut self) -> Result<u8, JpegError> {
        if self.bytes_in_buffer() == 0 {
            self.fill()?;
        }
        let byte = self.buffer[self.next];
        self.next += 1;
        Ok(byte)
    }

    pub fn read_u16(&mut self) -> Result<u32, JpegError> {
        let high = self.read_byte()? as u32;
        let low = self.read_byte()? as u32;
        Ok((high << 8) + low)
    }

    /// Skips `count` bytes, loading as many blocks as that takes (libtiff:
    /// past the end, one fake EOI instead).
    pub fn skip(&mut self, count: i64) -> Result<(), JpegError> {
        if count <= 0 {
            return Ok(());
        }
        if self.libtiff {
            if count as usize > self.bytes_in_buffer() {
                self.fill()?;
            } else {
                self.next += count as usize;
            }
            return Ok(());
        }
        let mut remaining = count as usize;
        while remaining > self.bytes_in_buffer() {
            remaining -= self.bytes_in_buffer();
            self.fill()?;
        }
        self.next += remaining;
        Ok(())
    }

    /// The profile readers' byte reader: nothing once a load finds the
    /// file used up (the fake EOI stays in the buffer).
    pub fn get_character(&mut self) -> Option<u8> {
        if self.bytes_in_buffer() == 0 {
            match self.fill() {
                Ok(false) => {}
                _ => return None,
            }
        }
        let byte = self.buffer[self.next];
        self.next += 1;
        Some(byte)
    }
}

/// Marker reading state.
pub(super) struct MarkerState {
    pub saw_soi: bool,
    pub saw_sof: bool,
    /// Bytes skipped looking for the next marker.
    pub discarded_bytes: usize,
    /// A marker read but not yet processed (0 for none).
    pub unread_marker: u8,
    /// The RSTn the decoder expects next.
    pub next_restart_num: u8,
}

impl MarkerState {
    pub fn new() -> Self {
        Self { saw_soi: false, saw_sof: false, discarded_bytes: 0, unread_marker: 0, next_restart_num: 0 }
    }
}

/// `first_marker`: the data must start with SOI.
pub(super) fn first_marker(source: &mut Source, markers: &mut MarkerState) -> Result<(), JpegError> {
    let c = source.read_byte()?;
    let c2 = source.read_byte()?;
    if c != 0xFF || c2 != M_SOI {
        return Err(JpegError::new(&format!("Not a JPEG file: starts with 0x{c:02x} 0x{c2:02x}")));
    }
    markers.unread_marker = c2;
    Ok(())
}

/// `next_marker`: skips anything but a marker (warning about what it
/// skipped), swallowing fill bytes and stuffed zeros.
pub(super) fn next_marker(source: &mut Source, markers: &mut MarkerState) -> Result<(), JpegError> {
    let mut c;
    loop {
        c = source.read_byte()?;
        while c != 0xFF {
            markers.discarded_bytes += 1;
            c = source.read_byte()?;
        }
        loop {
            c = source.read_byte()?;
            if c != 0xFF {
                break;
            }
        }
        if c != 0 {
            break;
        }
        markers.discarded_bytes += 2;
    }
    if markers.discarded_bytes != 0 {
        source.warn();
        markers.discarded_bytes = 0;
    }
    markers.unread_marker = c;
    Ok(())
}

/// `read_restart_marker`: the expected RSTn is swallowed; anything else
/// goes to `resync_to_restart`.
pub(super) fn read_restart_marker(source: &mut Source, markers: &mut MarkerState) -> Result<(), JpegError> {
    if markers.unread_marker == 0 {
        next_marker(source, markers)?;
    }
    if markers.unread_marker == M_RST0 + markers.next_restart_num {
        markers.unread_marker = 0;
    } else {
        resync_to_restart(source, markers, markers.next_restart_num as i32)?;
    }
    markers.next_restart_num = (markers.next_restart_num + 1) & 7;
    Ok(())
}

/// `jpeg_resync_to_restart`: decides from the marker found whether to
/// discard it, scan on to the next one, or leave it (making the decoder
/// treat the segment as empty).
fn resync_to_restart(source: &mut Source, markers: &mut MarkerState, desired: i32) -> Result<(), JpegError> {
    source.warn();
    let mut marker = markers.unread_marker as i32;
    loop {
        // 1: the desired restart or too far away; 2: invalid, or a prior
        // restart, so advance; 3: a non-restart marker or one of the next
        // two restarts.
        let action = if marker < M_SOF0 as i32 {
            2
        } else if marker < M_RST0 as i32 || marker > M_RST7 as i32 {
            3
        } else {
            let next = marker == M_RST0 as i32 + ((desired + 1) & 7) || marker == M_RST0 as i32 + ((desired + 2) & 7);
            let prior = marker == M_RST0 as i32 + ((desired - 1) & 7) || marker == M_RST0 as i32 + ((desired - 2) & 7);
            if next {
                3
            } else if prior {
                2
            } else {
                1
            }
        };
        match action {
            1 => {
                markers.unread_marker = 0;
                return Ok(());
            }
            2 => {
                next_marker(source, markers)?;
                marker = markers.unread_marker as i32;
            }
            _ => return Ok(()),
        }
    }
}
