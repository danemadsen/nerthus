//! Assimp's file access during an import: `DefaultIOSystem` (Windows
//! semantics: `Exists` is `_wstat64`, so directories exist too) behind the
//! `FileSystemFilter` `BaseImporter::ReadFile` wraps it in, which retries a
//! failed open relative to the imported file's folder. A file a scene
//! loads as one of its objects is read through the scene's filter, so its
//! own filter wraps that one, and all of them share the default file
//! system's directory stack.

use std::path::Path;
use std::sync::{Arc, Mutex};

/// `FileSystemFilter`.
#[derive(Clone)]
pub struct FileSystem {
    base: Vec<u8>,
    separator: u8,
    source: Vec<u8>,
    /// The filter this one wraps; `None` for `DefaultIOSystem`.
    wrapped: Option<Box<FileSystem>>,
    /// The imported file itself, its path made canonical, when known.
    identity: Option<std::path::PathBuf>,
    /// `IOSystem::m_pathStack` of the default file system, which every
    /// filter's directory calls reach.
    directories: Arc<Mutex<Vec<Vec<u8>>>>,
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

/// A file's canonical path, which tells files apart however they are
/// named.
fn canonical(path: &[u8]) -> Option<std::path::PathBuf> {
    let text = std::str::from_utf8(path).ok()?;
    std::fs::canonicalize(text).ok()
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

/// `ASSIMP_stricmp(a, b) == 0`: ASCII letters in either case, up to a NUL.
fn same_ignoring_case(first: &[u8], second: &[u8]) -> bool {
    let first_end = first.iter().position(|&byte| byte == 0).unwrap_or(first.len());
    let second_end = second.iter().position(|&byte| byte == 0).unwrap_or(second.len());
    first[..first_end].eq_ignore_ascii_case(&second[..second_end])
}

/// `MakeAbsolutePath`: `_wfullpath` on Windows, `realpath` elsewhere; the
/// path unchanged when that fails.
fn make_absolute(path: &[u8]) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(path) else {
        return path.to_vec();
    };
    #[cfg(windows)]
    let absolute = std::path::absolute(text);
    #[cfg(not(windows))]
    let absolute = std::fs::canonicalize(text);
    match absolute {
        Ok(absolute) => absolute.to_string_lossy().into_owned().into_bytes(),
        Err(_) => path.to_vec(),
    }
}

impl FileSystem {
    /// The filter for an import of `file` through `wrapped` (`None` for the
    /// default file system): the file's folder (with a trailing separator,
    /// `./` when it has none) is the base.
    pub fn new(file: &str, wrapped: Option<&FileSystem>) -> Self {
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
        let identity = match wrapped {
            None => canonical(file.as_bytes()),
            Some(_) => None,
        };
        let directories = match wrapped {
            Some(outer) => outer.directories.clone(),
            None => Arc::default(),
        };
        let wrapped = wrapped.map(|outer| Box::new(outer.clone()));
        FileSystem { base, separator, source: file.as_bytes().to_vec(), wrapped, identity, directories }
    }

    /// `PushDirectory`: an empty path is not pushed.
    pub fn push_directory(&self, path: &[u8]) -> bool {
        if path.is_empty() {
            return false;
        }
        let mut directories = self.directories.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        directories.push(path.to_vec());
        true
    }

    /// `CurrentDirectory`: the top of the stack, empty when there is none.
    pub fn current_directory(&self) -> Vec<u8> {
        let directories = self.directories.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        directories.last().cloned().unwrap_or_default()
    }

    /// `StackSize`.
    pub fn directory_count(&self) -> usize {
        let directories = self.directories.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        directories.len()
    }

    /// `PopDirectory`: whether there was one.
    pub fn pop_directory(&self) -> bool {
        let mut directories = self.directories.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        directories.pop().is_some()
    }

    /// Records which file this filter's import opened (for a file opened
    /// through an outer filter).
    pub fn set_identity(&mut self, identity: Option<std::path::PathBuf>) {
        self.identity = identity;
    }

    /// Whether this import, or one it is nested in, is of `file`.
    pub fn imports(&self, file: &std::path::Path) -> bool {
        if self.identity.as_deref() == Some(file) {
            return true;
        }
        self.wrapped.as_ref().is_some_and(|outer| outer.imports(file))
    }

    /// How many filters deep this one is (1 for a file opened directly).
    pub fn depth(&self) -> usize {
        1 + self.wrapped.as_ref().map_or(0, |outer| outer.depth())
    }

    /// `Exists`: another file than the imported one goes through
    /// `BuildPath`, then `Cleanup`.
    pub fn exists(&self, path: &str) -> bool {
        self.exists_bytes(path.as_bytes())
    }

    /// `Exists` for a path from file contents (not necessarily UTF-8).
    pub fn exists_bytes(&self, path: &[u8]) -> bool {
        let mut built = path.to_vec();
        if built != self.source {
            self.build_path(&mut built);
            cleanup(&mut built, self.separator);
        }
        self.wrapped_exists(&built)
    }

    /// `getOsSeparator`.
    pub fn separator(&self) -> char {
        self.separator as char
    }

    /// `ComparePaths`, which every filter hands down to `DefaultIOSystem`:
    /// equal ignoring case as given, or once made absolute.
    pub fn compare_paths(&self, first: &[u8], second: &[u8]) -> bool {
        if same_ignoring_case(first, second) {
            return true;
        }
        let first_absolute = make_absolute(first);
        let second_absolute = make_absolute(second);
        same_ignoring_case(&first_absolute, &second_absolute)
    }

    /// `Open`: the path as given, then `BuildPath`, then `Cleanup` and
    /// `BuildPath`.
    pub fn open(&self, path: &str) -> Option<Vec<u8>> {
        self.open_bytes(path.as_bytes())
    }

    /// `Open` for a path from file contents (not necessarily UTF-8).
    pub fn open_bytes(&self, original: &[u8]) -> Option<Vec<u8>> {
        self.open_resolved(original).map(|(contents, _)| contents)
    }

    /// `Open`, with the path the default file system read in the end.
    pub fn open_resolved(&self, original: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
        if let Some(opened) = self.wrapped_read(original) {
            return Some(opened);
        }
        let mut built = original.to_vec();
        self.build_path(&mut built);
        if let Some(opened) = self.wrapped_read(&built) {
            return Some(opened);
        }
        let mut cleaned = original.to_vec();
        cleanup(&mut cleaned, self.separator);
        self.build_path(&mut cleaned);
        self.wrapped_read(&cleaned)
    }

    /// `mWrapped->Exists`.
    fn wrapped_exists(&self, path: &[u8]) -> bool {
        match &self.wrapped {
            Some(outer) => outer.exists_bytes(path),
            None => exists(path),
        }
    }

    /// `mWrapped->Open`, read whole, and the path it read.
    fn wrapped_read(&self, path: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
        match &self.wrapped {
            Some(outer) => outer.open_resolved(path),
            None => read(path).map(|contents| (contents, path.to_vec())),
        }
    }

    /// The canonical path of a file the default file system read.
    pub fn identify(path: &[u8]) -> Option<std::path::PathBuf> {
        canonical(path)
    }

    /// `BuildPath`: the base plus the path, then the base plus ever longer
    /// tails of it (each cut `length − last separator` bytes long).
    fn build_path(&self, input: &mut Vec<u8>) {
        if input.len() < 3 || self.wrapped_exists(input) {
            return;
        }
        if input[1] != b':' {
            let mut joined = self.base.clone();
            joined.extend_from_slice(input);
            if self.wrapped_exists(&joined) {
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
            if self.wrapped_exists(&candidate) {
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
