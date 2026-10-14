//! Assimp's file access during an import: `DefaultIOSystem` (Windows
//! semantics: `Exists` is `_wstat64`, so directories exist too) behind the
//! `FileSystemFilter` `BaseImporter::ReadFile` wraps it in, which retries a
//! failed open relative to the imported file's folder.

use std::path::Path;

/// `FileSystemFilter`.
pub struct FileSystem {
    base: Vec<u8>,
    separator: u8,
}

fn exists(path: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(path) else {
        return false;
    };
    Path::new(text).metadata().is_ok()
}

fn read(path: &[u8]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(path).ok()?;
    std::fs::read(Path::new(text)).ok()
}

/// `std::string::rfind(ch, pos)`: the last `byte` at or before `limit`.
fn rfind(bytes: &[u8], byte: u8, limit: Option<usize>) -> Option<usize> {
    let end = match limit {
        Some(limit) => (limit + 1).min(bytes.len()),
        None => bytes.len(),
    };
    bytes[..end].iter().rposition(|&candidate| candidate == byte)
}

/// `IsSpaceOrNewLine`.
fn is_space_or_newline(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | 0 | 0x0c)
}

impl FileSystem {
    /// The filter for an import of `file`: its folder (with a trailing
    /// separator, `./` when it has none) is the base.
    pub fn new(file: &str) -> Self {
        let separator = std::path::MAIN_SEPARATOR as u8;
        let mut base = file.as_bytes().to_vec();
        match base.iter().rposition(|&byte| byte == b'\\' || byte == b'/') {
            Some(position) => base.truncate(position),
            None => base.clear(),
        }
        if base.is_empty() {
            base = vec![b'.', separator];
        } else if !matches!(base.last(), Some(b'\\') | Some(b'/')) {
            base.push(separator);
        }
        FileSystem { base, separator }
    }

    /// `Exists` for another file than the imported one: `BuildPath`, then
    /// `Cleanup`.
    pub fn exists(&self, path: &str) -> bool {
        let mut built = path.as_bytes().to_vec();
        self.build_path(&mut built);
        cleanup(&mut built, self.separator);
        exists(&built)
    }

    /// `getOsSeparator`.
    pub fn separator(&self) -> char {
        self.separator as char
    }

    /// `Open`: the path as given, then `BuildPath`, then `Cleanup` and
    /// `BuildPath`.
    pub fn open(&self, path: &str) -> Option<Vec<u8>> {
        self.open_bytes(path.as_bytes())
    }

    /// `Open` for a path from file contents (not necessarily UTF-8).
    pub fn open_bytes(&self, original: &[u8]) -> Option<Vec<u8>> {
        if let Some(contents) = read(original) {
            return Some(contents);
        }
        let mut built = original.to_vec();
        self.build_path(&mut built);
        if let Some(contents) = read(&built) {
            return Some(contents);
        }
        let mut cleaned = original.to_vec();
        cleanup(&mut cleaned, self.separator);
        self.build_path(&mut cleaned);
        read(&cleaned)
    }

    /// `BuildPath`: the base plus the path, then the base plus ever longer
    /// tails of it (each cut `length − last separator` bytes long).
    fn build_path(&self, input: &mut Vec<u8>) {
        if input.len() < 3 || exists(input) {
            return;
        }
        if input[1] != b':' {
            let mut joined = self.base.clone();
            joined.extend_from_slice(input);
            if exists(&joined) {
                *input = joined;
                return;
            }
        }
        let position = rfind(input, b'/', None).or_else(|| rfind(input, b'\\', None));
        let Some(position) = position else {
            return;
        };
        let mut last_separator: Option<usize> = None;
        loop {
            let mut candidate = self.base.clone();
            candidate.push(self.separator);
            let separator = rfind(input, b'/', last_separator).or_else(|| rfind(input, b'\\', last_separator));
            let Some(separator) = separator else {
                break;
            };
            if separator == 0 {
                break;
            }
            last_separator = Some(separator - 1);
            let start = separator + 1;
            let end = (start + (input.len() - position)).min(input.len());
            candidate.extend_from_slice(&input[start..end]);
            if exists(&candidate) {
                *input = candidate;
                return;
            }
        }
    }
}

/// `FileSystemFilter::Cleanup`: drops leading whitespace (and, by a slip,
/// the first character after it), turns separators into the OS one and
/// drops doubled ones, stepping over "://" and a leading "\\". (Its
/// "%xx" decoding tests the '%' itself for a hex digit, so never runs.)
fn cleanup(input: &mut Vec<u8>, separator: u8) {
    if input.is_empty() {
        return;
    }
    let leading = input.iter().take_while(|&&byte| is_space_or_newline(byte)).count();
    if leading != 0 {
        let end = (leading + 1).min(input.len());
        input.drain(..end);
    }
    let mut last = 0u8;
    let mut index = 0usize;
    while index < input.len() {
        if input[index..].starts_with(b"://") {
            index += 4;
            continue;
        }
        if index == 0 && input.starts_with(b"\\\\") {
            index += 3;
            continue;
        }
        let byte = input[index];
        if byte == b'/' || byte == b'\\' {
            input[index] = separator;
            if last == separator {
                input.remove(index);
                index -= 1;
            }
        }
        last = input[index];
        index += 1;
    }
}
