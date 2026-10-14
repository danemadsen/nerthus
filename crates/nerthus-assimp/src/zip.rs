//! Zip archives as Assimp reads them: `ZipArchiveIOSystem` over its bundled
//! minizip (`contrib/unzip`), through Assimp's IO stream as the Windows
//! build has it. A read past the end gets the bytes there are, a seek past
//! the end succeeds, and an offset of 4 GiB or more fails (minizip narrows
//! it to a 32-bit `uLong`). The stream never reports an error, so a number
//! read short is `UNZ_EOF`, which is `UNZ_OK`, with the value 0; a name or
//! data read short fails.
//!
//! An archive opens when its end of central directory record is found and
//! consistent. Its entries are mapped by name (empty ones left out, the
//! first of a repeated name kept, names simplified), and an entry is
//! extracted 65535 bytes at a time through `unzReadCurrentFile`: stored
//! data as it is, bzip2 data too (minizip is built without bzip2, so it
//! reads it raw), deflated data through zlib's inflate, 16 KiB of input at
//! a time. The CRC is never checked (only an `ai_assert` calls
//! `unzCloseCurrentFile`).

use std::collections::BTreeMap;

use zlib_rs::{Inflate, InflateFlush, Status};

use crate::scene::{self, ImportError};

/// `BUFREADCOMMENT`.
const BACKWARD_READ: u64 = 0x400;
/// `SIZECENTRALDIRITEM` and `SIZEZIPLOCALHEADER`.
const CENTRAL_ITEM_SIZE: u64 = 0x2e;
const LOCAL_HEADER_SIZE: u64 = 0x1e;
/// `ZipArchiveIOSystem::Implement::FileNameSize`.
const FILE_NAME_SIZE: u64 = 256;
/// `UNZ_BUFSIZE`: compressed data is read this much at a time.
const READ_BUFFER_SIZE: u64 = 16384;
/// `UINT16_MAX`: `ZipFileInfo::Extract` reads this much at a time.
const EXTRACT_CHUNK: u64 = 0xffff;
/// `MAXU32`, also the largest offset a seek takes.
const MAXU32: u64 = 0xffff_ffff;
/// `Z_DEFLATED` and `Z_BZIP2ED`.
const DEFLATED: u64 = 8;
const BZIP2ED: u64 = 12;
/// `MAX_WBITS`.
const WINDOW_BITS: u8 = 15;

/// The archive file behind `IOSystem2Unzip` and `DefaultIOStream` (see the
/// module notes). Every minizip operation seeks before it reads, so each
/// one gets its own.
struct Stream<'a> {
    bytes: &'a [u8],
    position: u64,
}

impl<'a> Stream<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    /// `ZSEEK64` from the start.
    fn seek_set(&mut self, offset: u64) -> bool {
        if offset > MAXU32 {
            return false;
        }
        self.position = offset;
        true
    }

    /// `ZSEEK64` from the current position.
    fn seek_cur(&mut self, offset: u64) -> bool {
        if offset > MAXU32 {
            return false;
        }
        self.position += offset;
        true
    }

    /// `ZREAD64` (`fread`): up to `size` bytes, the position moved past
    /// the ones read.
    fn read(&mut self, size: u64) -> &'a [u8] {
        let length = self.bytes.len() as u64;
        if self.position >= length {
            return &[];
        }
        let count = size.min(length - self.position);
        let start = self.position as usize;
        let end = start + count as usize;
        self.position += count;
        &self.bytes[start..end]
    }

    /// `unz64local_getShort`: 0 when read short.
    fn get_u16(&mut self) -> u64 {
        let pair = self.read(2);
        if pair.len() != 2 {
            return 0;
        }
        u16::from_le_bytes([pair[0], pair[1]]) as u64
    }

    /// `unz64local_getLong`: 0 when read short.
    fn get_u32(&mut self) -> u64 {
        let quad = self.read(4);
        if quad.len() != 4 {
            return 0;
        }
        u32::from_le_bytes([quad[0], quad[1], quad[2], quad[3]]) as u64
    }

    /// `unz64local_getLong64`: 0 when read short.
    fn get_u64(&mut self) -> u64 {
        let octet = self.read(8);
        let Ok(value) = <[u8; 8]>::try_from(octet) else {
            return 0;
        };
        u64::from_le_bytes(value)
    }
}

/// `unz64local_SearchCentralDir` (and `…64`): the last `signature` within
/// the final 64 KiB, read backwards a kilobyte (plus four bytes) at a time.
fn search_backwards(bytes: &[u8], signature: [u8; 4]) -> Option<u64> {
    let size = bytes.len() as u64;
    let max_back = size.min(0xffff);
    let mut back = 4u64;
    while back < max_back {
        back = (back + BACKWARD_READ).min(max_back);
        let read_position = size - back;
        let read_size = (BACKWARD_READ + 4).min(size - read_position);
        let window = &bytes[read_position as usize..(read_position + read_size) as usize];
        let mut index = window.len() as i64 - 3;
        loop {
            index -= 1;
            if index < 0 {
                break;
            }
            let at = index as usize;
            if window[at..at + 4] == signature {
                return Some(read_position + at as u64);
            }
        }
    }
    None
}

/// `unz64local_SearchCentralDir64`: the Zip64 locator, then the record it
/// points at.
fn search_central_dir64(bytes: &[u8]) -> Option<u64> {
    let locator = search_backwards(bytes, [0x50, 0x4b, 0x06, 0x07])?;
    let mut stream = Stream::new(bytes);
    if !stream.seek_set(locator) {
        return None;
    }
    stream.get_u32();
    if stream.get_u32() != 0 {
        return None;
    }
    let relative_offset = stream.get_u64();
    if stream.get_u32() != 1 {
        return None;
    }
    if !stream.seek_set(relative_offset) {
        return None;
    }
    if stream.get_u32() != 0x0606_4b50 {
        return None;
    }
    Some(relative_offset)
}

/// One central directory entry as `unz64local_GetCurrentFileInfoInternal`
/// reads it (with any Zip64 sizes and offset).
#[derive(Clone, Debug)]
struct FileInfo {
    compression_method: u64,
    crc: u64,
    compressed_size: u64,
    uncompressed_size: u64,
    size_filename: u64,
    size_file_extra: u64,
    size_file_comment: u64,
    local_offset: u64,
}

/// A mapped entry (`ZipFileInfo`): where its directory entry is, as
/// `unz_file_pos` keeps it (a 32-bit `uLong`), and the size it has in the
/// 32-bit `unz_file_info`.
#[derive(Clone, Copy, Debug)]
struct Entry {
    position: u64,
    size: u64,
}

/// An open archive (`unz64_s`) and its name map.
pub(crate) struct Archive<'a> {
    bytes: &'a [u8],
    offset_central_dir: u64,
    byte_before_the_zipfile: u64,
    number_entry: u64,
    map: BTreeMap<Vec<u8>, Entry>,
}

/// A name passed as `const char *`: up to its first NUL.
fn c_string(name: &[u8]) -> &[u8] {
    match name.iter().position(|&byte| byte == 0) {
        Some(nul) => &name[..nul],
        None => name,
    }
}

impl<'a> Archive<'a> {
    /// `unzOpen2` then `MapArchive`: none when the file is not a zip
    /// archive (`isOpen`); an error when mapping a name throws (see
    /// [`simplify_filename`]), which fails the whole import.
    pub fn open(bytes: &'a [u8]) -> Result<Option<Self>, ImportError> {
        let Some(mut archive) = Self::open_handle(bytes) else {
            return Ok(None);
        };
        archive.map_archive()?;
        Ok(Some(archive))
    }

    /// `unzOpenInternal`: the end of central directory record (Zip64
    /// first), one disk only, the directory before the record (the sum
    /// wraps).
    fn open_handle(bytes: &'a [u8]) -> Option<Self> {
        let mut stream = Stream::new(bytes);
        let central_position;
        let number_entry;
        let size_central_dir;
        let offset_central_dir;
        if let Some(position) = search_central_dir64(bytes) {
            if !stream.seek_set(position) {
                return None;
            }
            stream.get_u32();
            stream.get_u64();
            stream.get_u16();
            stream.get_u16();
            let number_disk = stream.get_u32();
            let disk_with_directory = stream.get_u32();
            number_entry = stream.get_u64();
            let entries_in_directory = stream.get_u64();
            if entries_in_directory != number_entry || disk_with_directory != 0 || number_disk != 0 {
                return None;
            }
            size_central_dir = stream.get_u64();
            offset_central_dir = stream.get_u64();
            central_position = position;
        } else {
            let position = search_backwards(bytes, [0x50, 0x4b, 0x05, 0x06])?;
            if !stream.seek_set(position) {
                return None;
            }
            stream.get_u32();
            let number_disk = stream.get_u16();
            let disk_with_directory = stream.get_u16();
            number_entry = stream.get_u16();
            let entries_in_directory = stream.get_u16();
            if entries_in_directory != number_entry || disk_with_directory != 0 || number_disk != 0 {
                return None;
            }
            size_central_dir = stream.get_u32();
            offset_central_dir = stream.get_u32();
            central_position = position;
        }
        let directory_end = offset_central_dir.wrapping_add(size_central_dir);
        if central_position < directory_end {
            return None;
        }
        let byte_before_the_zipfile = central_position - directory_end;
        Some(Self { bytes, offset_central_dir, byte_before_the_zipfile, number_entry, map: BTreeMap::new() })
    }

    /// `unz64local_GetCurrentFileInfoInternal` at a central directory
    /// position: the fixed fields, then (for `unzGetCurrentFileInfo`'s
    /// 256-byte buffer) the name, then the extra fields read one after
    /// another (a Zip64 field's values are read in place of the sizes and
    /// offset that are all ones, whatever its size says).
    fn file_info(&self, position_in_directory: u64, read_name: bool) -> Option<(FileInfo, Vec<u8>)> {
        let mut stream = Stream::new(self.bytes);
        let position = position_in_directory.wrapping_add(self.byte_before_the_zipfile);
        if !stream.seek_set(position) {
            return None;
        }
        if stream.get_u32() != 0x0201_4b50 {
            return None;
        }
        stream.get_u16();
        stream.get_u16();
        stream.get_u16();
        let compression_method = stream.get_u16();
        stream.get_u32();
        let crc = stream.get_u32();
        let mut compressed_size = stream.get_u32();
        let mut uncompressed_size = stream.get_u32();
        let size_filename = stream.get_u16();
        let size_file_extra = stream.get_u16();
        let size_file_comment = stream.get_u16();
        let mut disk_num_start = stream.get_u16();
        stream.get_u16();
        stream.get_u32();
        let mut local_offset = stream.get_u32();

        // What is left of the name to skip (`lSeek`).
        let mut name = Vec::new();
        let mut name_left = size_filename;
        if read_name {
            let name_length = size_filename.min(FILE_NAME_SIZE);
            if size_filename > 0 {
                let read = stream.read(name_length);
                if read.len() as u64 != name_length {
                    return None;
                }
                name = read.to_vec();
            }
            name_left -= name_length;
        }

        if size_file_extra != 0 {
            if name_left != 0 && !stream.seek_cur(name_left) {
                return None;
            }
            let mut accumulated = 0u64;
            while accumulated < size_file_extra {
                let header_id = stream.get_u16();
                let data_size = stream.get_u16();
                if header_id == 0x0001 {
                    if uncompressed_size == MAXU32 {
                        uncompressed_size = stream.get_u64();
                    }
                    if compressed_size == MAXU32 {
                        compressed_size = stream.get_u64();
                    }
                    if local_offset == MAXU32 {
                        local_offset = stream.get_u64();
                    }
                    if disk_num_start == 0xffff {
                        disk_num_start = stream.get_u32();
                    }
                } else if !stream.seek_cur(data_size) {
                    return None;
                }
                accumulated += 4 + data_size;
            }
        }

        let info = FileInfo {
            compression_method,
            crc,
            compressed_size,
            uncompressed_size,
            size_filename,
            size_file_extra,
            size_file_comment,
            local_offset,
        };
        Some((info, name))
    }

    /// `MapArchive`: `unzGoToFirstFile`, then each entry read again with
    /// its name (`unzGetCurrentFileInfo`) and mapped, then
    /// `unzGoToNextFile`, until the count is reached or an entry fails to
    /// read (an entry only its name fails for is left out).
    fn map_archive(&mut self) -> Result<(), ImportError> {
        let mut position = self.offset_central_dir;
        let mut number = 0u64;
        let Some((mut current, _)) = self.file_info(position, false) else {
            return Ok(());
        };
        loop {
            if let Some((info, name)) = self.file_info(position, true) {
                let size = info.uncompressed_size & MAXU32;
                if size != 0 && info.size_filename <= FILE_NAME_SIZE {
                    let simplified = simplify_filename(&name)?;
                    let entry = Entry { position: position & MAXU32, size };
                    self.map.entry(simplified).or_insert(entry);
                }
            }
            if self.number_entry != 0xffff && number.wrapping_add(1) == self.number_entry {
                return Ok(());
            }
            let entry_size = CENTRAL_ITEM_SIZE + current.size_filename + current.size_file_extra + current.size_file_comment;
            position = position.wrapping_add(entry_size);
            number += 1;
            match self.file_info(position, false) {
                Some((next, _)) => current = next,
                None => return Ok(()),
            }
        }
    }

    /// `Exists`: an exact match among the mapped (simplified) names.
    pub fn exists(&self, name: &[u8]) -> bool {
        self.map.contains_key(c_string(name))
    }

    /// The mapped names, in the map's order.
    pub fn names(&self) -> Vec<Vec<u8>> {
        self.map.keys().cloned().collect()
    }

    /// `OpenFile`: the entry named (after simplifying the name),
    /// extracted; none when it is missing or fails to extract.
    pub fn read(&self, name: &[u8]) -> Result<Option<Vec<u8>>, ImportError> {
        let simplified = simplify_filename(c_string(name))?;
        let Some(entry) = self.map.get(&simplified) else {
            return Ok(None);
        };
        self.extract(*entry)
    }

    /// `ZipFileInfo::Extract`: the entry found again (`unzGoToFilePos`),
    /// opened (`unzOpenCurrentFile`), its buffer allocated, and read
    /// 65535 bytes at a time to the size the map has; none when a read
    /// gives less.
    fn extract(&self, entry: Entry) -> Result<Option<Vec<u8>>, ImportError> {
        let Some((info, _)) = self.file_info(entry.position, false) else {
            return Ok(None);
        };
        let Some(data_position) = self.local_data_position(&info) else {
            return Ok(None);
        };
        scene::check_allocation(entry.size, 1)?;

        let mut current = CurrentFile::new(&info, data_position);
        let mut output = Vec::new();
        let mut chunk = vec![0u8; entry.size.min(EXTRACT_CHUNK) as usize];
        while (output.len() as u64) < entry.size {
            let chunk_size = (entry.size - output.len() as u64).min(EXTRACT_CHUNK);
            let buffer = &mut chunk[..chunk_size as usize];
            let read = current.read(self, buffer);
            if read != Some(chunk_size) {
                return Ok(None);
            }
            output.extend_from_slice(buffer);
        }
        Ok(Some(output))
    }

    /// `unz64local_CheckCurrentFileCoherencyHeader` for
    /// `unzOpenCurrentFile3`: the local header matches the directory
    /// entry (the method, and the CRC and sizes unless its flags defer
    /// them to a data descriptor; a size of all ones always matches), and
    /// the method is stored, deflated or bzip2. Gives where the data
    /// starts.
    fn local_data_position(&self, info: &FileInfo) -> Option<u64> {
        let mut stream = Stream::new(self.bytes);
        let header = info.local_offset.wrapping_add(self.byte_before_the_zipfile);
        if !stream.seek_set(header) {
            return None;
        }
        if stream.get_u32() != 0x0403_4b50 {
            return None;
        }
        stream.get_u16();
        let flags = stream.get_u16();
        if stream.get_u16() != info.compression_method {
            return None;
        }
        if info.compression_method != 0 && info.compression_method != BZIP2ED && info.compression_method != DEFLATED {
            return None;
        }
        stream.get_u32();
        let deferred = flags & 8 != 0;
        let crc = stream.get_u32();
        if crc != info.crc && !deferred {
            return None;
        }
        let compressed_size = stream.get_u32();
        if compressed_size != MAXU32 && compressed_size != info.compressed_size && !deferred {
            return None;
        }
        let uncompressed_size = stream.get_u32();
        if uncompressed_size != MAXU32 && uncompressed_size != info.uncompressed_size && !deferred {
            return None;
        }
        let size_filename = stream.get_u16();
        if size_filename != info.size_filename {
            return None;
        }
        let size_extra_field = stream.get_u16();
        let data_offset = LOCAL_HEADER_SIZE + size_filename + size_extra_field;
        Some(info.local_offset.wrapping_add(data_offset))
    }
}

/// What `unzReadCurrentFile` keeps of an open entry
/// (`file_in_zip64_read_info_s`).
struct CurrentFile<'a> {
    /// `raw`: set for bzip2, which is then copied like stored data.
    raw: bool,
    /// None for data copied as it is.
    inflate: Option<Inflate>,
    position: u64,
    rest_compressed: u64,
    rest_uncompressed: u64,
    /// The block of compressed data read last and how far into it the
    /// reading is (`next_in`, `avail_in`).
    input: &'a [u8],
    input_start: usize,
}

impl<'a> CurrentFile<'a> {
    fn new(info: &FileInfo, data_position: u64) -> Self {
        let raw = info.compression_method == BZIP2ED;
        let inflate = if info.compression_method == DEFLATED { Some(Inflate::new(false, WINDOW_BITS)) } else { None };
        Self {
            raw,
            inflate,
            position: data_position,
            rest_compressed: info.compressed_size,
            rest_uncompressed: info.uncompressed_size,
            input: &[],
            input_start: 0,
        }
    }

    fn available_input(&self) -> u64 {
        (self.input.len() - self.input_start) as u64
    }

    /// `unzReadCurrentFile` into `output`: the count read (0 also for
    /// `UNZ_EOF`), none for an error. Stored data stops at the entry's
    /// uncompressed size and raw data at its compressed size; compressed
    /// data is read a block at a time and has to be there in full.
    fn read(&mut self, archive: &Archive<'a>, output: &mut [u8]) -> Option<u64> {
        let length = output.len() as u64;
        let mut avail_out = length;
        if length > self.rest_uncompressed && !self.raw {
            avail_out = self.rest_uncompressed;
        }
        let input_left = self.rest_compressed.wrapping_add(self.available_input());
        if length > input_left && self.raw {
            avail_out = input_left;
        }

        let mut read = 0u64;
        while avail_out > 0 {
            if self.available_input() == 0 && self.rest_compressed > 0 {
                let read_this = self.rest_compressed.min(READ_BUFFER_SIZE);
                let mut stream = Stream::new(archive.bytes);
                let block_position = self.position.wrapping_add(archive.byte_before_the_zipfile);
                if !stream.seek_set(block_position) {
                    return None;
                }
                let block = stream.read(read_this);
                if block.len() as u64 != read_this {
                    return None;
                }
                self.input = block;
                self.input_start = 0;
                self.position = self.position.wrapping_add(read_this);
                self.rest_compressed -= read_this;
            }

            let start = read as usize;
            let Some(inflate) = self.inflate.as_mut() else {
                let available = self.input.len() - self.input_start;
                if available == 0 && self.rest_compressed == 0 {
                    return Some(read);
                }
                let count = (avail_out as usize).min(available);
                let source = &self.input[self.input_start..self.input_start + count];
                output[start..start + count].copy_from_slice(source);
                self.input_start += count;
                self.rest_uncompressed = self.rest_uncompressed.wrapping_sub(count as u64);
                avail_out -= count as u64;
                read += count as u64;
                continue;
            };

            let input = &self.input[self.input_start..];
            let target = &mut output[start..start + avail_out as usize];
            let total_in = inflate.total_in();
            let total_out = inflate.total_out();
            let result = inflate.decompress(input, target, InflateFlush::SyncFlush);
            let consumed = inflate.total_in() - total_in;
            let produced = inflate.total_out() - total_out;
            self.input_start += consumed as usize;
            self.rest_uncompressed = self.rest_uncompressed.wrapping_sub(produced);
            avail_out -= produced;
            read += produced;
            match result {
                Ok(Status::StreamEnd) => return Some(read),
                Ok(Status::Ok) => {}
                Ok(Status::BufError) | Err(_) => return None,
            }
        }
        Some(read)
    }
}

/// `SimplifyFilename`: backslashes as slashes, leading `.` and `/` gone,
/// `folder/../` collapsed back to the slash before `folder`. A `/../`
/// with no slash before it makes `std::string::erase` throw
/// `out_of_range`.
fn simplify_filename(name: &[u8]) -> Result<Vec<u8>, ImportError> {
    let mut simplified: Vec<u8> = name.iter().map(|&byte| if byte == b'\\' { b'/' } else { byte }).collect();
    let leading = simplified.iter().position(|&byte| byte != b'.' && byte != b'/').unwrap_or(simplified.len());
    simplified.drain(..leading);
    let relative = b"/../";
    while let Some(position) = simplified.windows(4).position(|window| window == relative) {
        let Some(previous) = simplified[..position].iter().rposition(|&byte| byte == b'/') else {
            return Err(ImportError("std::exception: invalid string position".to_string()));
        };
        simplified.drain(previous..position + 3);
    }
    Ok(simplified)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn names_are_simplified() {
        assert_eq!(simplify_filename(b"./3D\\3dmodel.model").unwrap(), b"3D/3dmodel.model");
        assert_eq!(simplify_filename(b"a/b/../c.png").unwrap(), b"a/c.png");
        assert_eq!(simplify_filename(b"../x").unwrap(), b"x");
        assert!(simplify_filename(b"a/../c.png").is_err());
    }

    /// A one-entry archive: local header, data, central directory, end
    /// record.
    fn archive_with(name: &[u8], method: u16, data: &[u8], compressed_size: u32, uncompressed_size: u32, crc: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        bytes.extend_from_slice(&[20, 0, 0, 0]);
        bytes.extend_from_slice(&method.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&crc.to_le_bytes());
        bytes.extend_from_slice(&compressed_size.to_le_bytes());
        bytes.extend_from_slice(&uncompressed_size.to_le_bytes());
        bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(data);
        let directory_start = bytes.len() as u32;
        bytes.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        bytes.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
        bytes.extend_from_slice(&method.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&crc.to_le_bytes());
        bytes.extend_from_slice(&compressed_size.to_le_bytes());
        bytes.extend_from_slice(&uncompressed_size.to_le_bytes());
        bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&[0; 12]);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(name);
        let directory_size = bytes.len() as u32 - directory_start;
        bytes.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        bytes.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
        bytes.extend_from_slice(&directory_size.to_le_bytes());
        bytes.extend_from_slice(&directory_start.to_le_bytes());
        bytes.extend_from_slice(&[0, 0]);
        bytes
    }

    #[test]
    fn a_cut_end_record_reads_as_zeros() {
        let archive = Archive::open(b"PK\x05\x06\x00\x00").unwrap().unwrap();
        assert!(archive.names().is_empty());
    }

    #[test]
    fn stored_and_deflated_entries_extract() {
        let stored = archive_with(b"a.txt", 0, b"hello", 5, 5, 7);
        let archive = Archive::open(&stored).unwrap().unwrap();
        assert_eq!(archive.names(), vec![b"a.txt".to_vec()]);
        assert_eq!(archive.read(b"./a.txt\0ignored").unwrap().unwrap(), b"hello");

        let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(b"deflated text").unwrap();
        let compressed = encoder.finish().unwrap();
        let deflated = archive_with(b"b.txt", 8, &compressed, compressed.len() as u32, 13, 0);
        let archive = Archive::open(&deflated).unwrap().unwrap();
        assert_eq!(archive.read(b"b.txt").unwrap().unwrap(), b"deflated text");
    }

    #[test]
    fn stored_data_is_read_a_block_at_a_time() {
        // Five bytes wanted of a thousand claimed: the whole block of a
        // thousand has to be there, which runs past the file's end.
        let short = archive_with(b"a", 0, b"01234567", 1000, 5, 0);
        let archive = Archive::open(&short).unwrap().unwrap();
        assert_eq!(archive.read(b"a").unwrap(), None);
        let enough = archive_with(b"a", 0, b"0123456789", 10, 5, 0);
        let archive = Archive::open(&enough).unwrap().unwrap();
        assert_eq!(archive.read(b"a").unwrap().unwrap(), b"01234");
    }
}
