//! Macros: reusable graphs packaged as `.macro` files and run by
//! `Macro` nodes.
//!
//! A `.macro` file is a zip of `metadata.json` (`MacroMetadata`) and
//! `data.bin`: the macro's asset (`MacroAsset`, an asset with metadata) as
//! base32 of base64 of a little-endian length and the gzip of its JSON.
//! Inside, `MacroPort` nodes are the macro's inputs and outputs.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use indexmap::IndexMap;
use serde_json::Value;
use thiserror::Error;

use crate::automation::Variable;
use crate::catalogue::Catalogue;
use crate::format_names::MACRO_VERSION_MEMBER;
use crate::graph::{Engine, NodeError};
use crate::json::{parse_lenient, References};
use crate::map::{Map, Resolution};
use crate::project::{parse_asset, Asset, NodeData};

#[derive(Debug, Error)]
pub enum MacroError {
    #[error("could not read the macro: {0}")]
    Io(#[from] std::io::Error),
    #[error("the macro file is not a readable zip archive")]
    Archive,
    #[error("the macro has no {0}")]
    Missing(&'static str),
    #[error("the macro's data could not be decoded")]
    Encoding,
    #[error("the macro's data is not a valid asset: {0}")]
    Asset(String),
    #[error("Dependent macro [{0}] was not found. Install this macro first then try again.")]
    MissingDependency(String),
}

/// `MacroMetadata`.
#[derive(Clone, Debug)]
pub struct MacroMetadata {
    pub name: String,
    pub author: String,
    pub url: String,
    pub version: String,
    /// The `Project.Id` of the project the macro was exported from.
    pub source: String,
    pub format_version: String,
    pub description: String,
    pub search_string: String,
    /// Macros this one uses (`DependentMacros`).
    pub dependent_macros: Vec<String>,
}

impl Default for MacroMetadata {
    /// `new MacroMetadata()`.
    fn default() -> Self {
        Self {
            name: String::new(),
            author: String::new(),
            url: String::new(),
            version: "1.0.0.0".to_string(),
            source: String::new(),
            format_version: "2.3.0.0".to_string(),
            description: String::new(),
            search_string: String::new(),
            dependent_macros: Vec::new(),
        }
    }
}

impl MacroMetadata {
    /// Members as Newtonsoft fills them: missing ones keep the class's
    /// values.
    pub fn from_json(value: &Value) -> MacroMetadata {
        let defaults = MacroMetadata::default();
        let text = |name: &str, default: &str| value.get(name).and_then(Value::as_str).unwrap_or(default).to_string();
        let dependent = value.get("DependentMacros").map(crate::json::collection_items).unwrap_or(&[]);
        MacroMetadata {
            name: text("Name", ""),
            author: text("Author", ""),
            url: text("URL", ""),
            version: text("Version", &defaults.version),
            source: text("Source", &defaults.source),
            format_version: text(MACRO_VERSION_MEMBER, &defaults.format_version),
            description: text("Description", ""),
            search_string: text("SearchString", ""),
            dependent_macros: dependent.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        }
    }

    /// `new MacroMetadata(name, metadata)` (Export Macro's): the
    /// dialog's fields (the version typed in it included), `Name Macro ` to
    /// search by, and this format version.
    pub fn for_export(name: &str, author: &str, url: &str, version: &str, description: &str, source: &str) -> MacroMetadata {
        MacroMetadata {
            name: name.to_string(),
            author: author.to_string(),
            url: url.to_string(),
            version: version.to_string(),
            source: source.to_string(),
            format_version: FORMAT_VERSION.to_string(),
            description: description.to_string(),
            search_string: format!("{name} Macro "),
            dependent_macros: Vec::new(),
        }
    }

    /// `MacroMetadata.Id`: `Author.Name`.
    pub fn id(&self) -> String {
        format!("{}.{}", self.author, self.name)
    }

    /// `metadata.json` (indented, with reference ids; the
    /// read-only `Id` comes first).
    pub fn to_json(&self) -> String {
        let dependent: Vec<String> = self.dependent_macros.iter().map(|id| format!("      {}", json_string(id))).collect();
        let dependent_values = if dependent.is_empty() { "[]".to_string() } else { format!("[\r\n{}\r\n    ]", dependent.join(",\r\n")) };
        let members = [
            ("Id", json_string(&self.id())),
            ("Name", json_string(&self.name)),
            ("Author", json_string(&self.author)),
            ("URL", json_string(&self.url)),
            ("Version", json_string(&self.version)),
            ("Source", json_string(&self.source)),
            (MACRO_VERSION_MEMBER, json_string(&self.format_version)),
            ("Description", json_string(&self.description)),
            ("SearchString", json_string(&self.search_string)),
            ("DependentMacros", format!("{{\r\n    \"$id\": \"2\",\r\n    \"$values\": {dependent_values}\r\n  }}")),
        ];
        let mut lines = vec!["  \"$id\": \"1\"".to_string()];
        for (name, value) in members {
            lines.push(format!("  \"{name}\": {value}"));
        }
        format!("{{\r\n{}\r\n}}", lines.join(",\r\n"))
    }

    /// `MacroMetadata.VersionObject` (`System.Version.Parse`), as numbers.
    pub fn version_numbers(&self) -> Option<Vec<u32>> {
        parse_version(&self.version)
    }

    /// The format version the macro was made with, as numbers.
    pub fn format_version_numbers(&self) -> Option<Vec<u32>> {
        parse_version(&self.format_version)
    }
}

/// The project format version Nerthus writes.
pub const FORMAT_VERSION: &str = "2.3.1.0";

/// `System.Version.Parse`: two to four dot-separated numbers.
pub fn parse_version(text: &str) -> Option<Vec<u32>> {
    let parts: Vec<&str> = text.trim().split('.').collect();
    if !(2..=4).contains(&parts.len()) {
        return None;
    }
    parts.iter().map(|part| part.trim().parse::<u32>().ok()).collect()
}

/// Two versions compared as `System.Version` does (a missing part counts
/// below zero).
pub fn compare_versions(left: &[u32], right: &[u32]) -> std::cmp::Ordering {
    for index in 0..4 {
        let left_part = left.get(index).map(|part| *part as i64).unwrap_or(-1);
        let right_part = right.get(index).map(|part| *part as i64).unwrap_or(-1);
        match left_part.cmp(&right_part) {
            std::cmp::Ordering::Equal => continue,
            unequal => return unequal,
        }
    }
    std::cmp::Ordering::Equal
}

/// A JSON string as Newtonsoft escapes it (`StringEscapeHandling.Default`).
fn json_string(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len() + 2);
    escaped.push('"');
    for character in text.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\u{8}' => escaped.push_str("\\b"),
            '\u{c}' => escaped.push_str("\\f"),
            control if (control as u32) < 0x20 || control == '\u{85}' || control == '\u{2028}' || control == '\u{2029}' => {
                escaped.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => escaped.push(other),
        }
    }
    escaped.push('"');
    escaped
}

/// `MacroAsset`: a macro's graph, automation and metadata.
#[derive(Clone, Debug)]
pub struct MacroAsset {
    pub metadata: MacroMetadata,
    pub asset: Asset,
}

impl MacroAsset {
    /// From the asset's JSON (what `data.bin` holds).
    pub fn from_json(text: &str) -> Result<MacroAsset, MacroError> {
        let document = parse_lenient(text).map_err(|error| MacroError::Asset(error.to_string()))?;
        let references = References::collect(&document);
        let mut unknown_types = Vec::new();
        let asset = parse_asset(&document, &references, Catalogue::get(), &mut unknown_types).map_err(|error| MacroError::Asset(error.to_string()))?;
        let metadata = document.get("Metadata").map(|value| references.resolve(value));
        let metadata = metadata.map(MacroMetadata::from_json).unwrap_or_default();
        Ok(MacroAsset { metadata, asset })
    }

    /// What adding the macro to a project does to it (`M.AddToProject`,
    /// the macro library): its `MacroDefinition` dropped and its node
    /// ids moved up by `offset` (1000 × (the project's macro count + 1)),
    /// with its connections and bindings — but not its expressions, whose
    /// keys keep the old ids and so no longer apply.
    pub fn add_to_project(&mut self, offset: i32) {
        let terrain = &mut self.asset.terrain;
        let nodes = std::mem::take(&mut terrain.nodes);
        for (_, mut node) in nodes {
            if node.type_name == "MacroDefinition" {
                continue;
            }
            node.id += offset;
            for port in node.ports.iter_mut() {
                if let Some(record) = port.record.as_mut() {
                    record.from += offset;
                    record.to += offset;
                }
            }
            terrain.nodes.insert(node.id, node);
        }
        for binding in self.asset.automation.bindings.iter_mut() {
            binding.node += offset;
        }
    }

    /// Reads a `.macro` file.
    pub fn read(path: &Path) -> Result<MacroAsset, MacroError> {
        let bytes = std::fs::read(path)?;
        let entries = read_zip(&bytes)?;
        let data = entries.iter().find(|(name, _)| name.eq_ignore_ascii_case("data.bin")).ok_or(MacroError::Missing("data.bin"))?;
        let text = String::from_utf8(data.1.clone()).map_err(|_| MacroError::Encoding)?;
        let json = decode_macro_data(text.trim())?;
        MacroAsset::from_json(&json)
    }

    /// A `.macro` file's `metadata.json` alone (`M.LoadMetadata`).
    pub fn read_metadata(path: &Path) -> Result<MacroMetadata, MacroError> {
        let bytes = std::fs::read(path)?;
        let entries = read_zip(&bytes)?;
        let file = entries.iter().find(|(name, _)| name == "metadata.json").ok_or(MacroError::Missing("metadata.json"))?;
        let text = String::from_utf8_lossy(&file.1);
        let text = text.trim_start_matches('\u{feff}');
        let document = parse_lenient(text).map_err(|error| MacroError::Asset(error.to_string()))?;
        Ok(MacroMetadata::from_json(&document))
    }
}

/// Writes a `.macro` file (`M.ExportMacro`): a zip of `data.bin` (the
/// macro asset's JSON, encoded) and `metadata.json`, deflated fast, as
/// `ZipFile.CreateFromDirectory` lists them.
pub fn write_macro_file(path: &Path, metadata: &MacroMetadata, asset_json: &str) -> std::io::Result<()> {
    let data = encode_macro_data(asset_json);
    let metadata_json = metadata.to_json();
    let entries = [("data.bin", data.as_bytes()), ("metadata.json", metadata_json.as_bytes())];
    let archive = write_zip(&entries)?;
    std::fs::write(path, archive)
}

/// RFC 4648 base32 without padding (`Base32Url.ToBase32String`).
fn encode_base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut text = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for byte in bytes {
        buffer = (buffer << 8) | *byte as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            text.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
        buffer &= (1 << bits) - 1;
    }
    if bits > 0 {
        text.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    text
}

/// The asset's JSON to `data.bin`'s text — base32 of the
/// base64 of its UTF-8 length (little-endian) and its gzip.
pub fn encode_macro_data(json: &str) -> String {
    use std::io::Write as _;
    let bytes = json.as_bytes();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    // Writing into memory cannot fail.
    let _ = encoder.write_all(bytes);
    let gzip = encoder.finish().unwrap_or_default();
    let mut packed = Vec::with_capacity(gzip.len() + 4);
    packed.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    packed.extend_from_slice(&gzip);
    let base64_text = base64::engine::general_purpose::STANDARD.encode(packed);
    encode_base32(base64_text.as_bytes())
}

/// The zip's CRC-32 (IEEE, reflected).
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// A zip archive of deflated files, dated now.
fn write_zip(entries: &[(&str, &[u8])]) -> std::io::Result<Vec<u8>> {
    use std::io::Write as _;
    let (dos_time, dos_date) = dos_date_time();
    let mut archive = Vec::new();
    let mut directory = Vec::new();
    for (name, contents) in entries {
        let mut deflater = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
        deflater.write_all(contents)?;
        let compressed = deflater.finish()?;
        let crc = crc32(contents);
        let offset = archive.len() as u32;
        let name_bytes = name.as_bytes();

        let mut local = Vec::new();
        local.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        local.extend_from_slice(&20u16.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(&8u16.to_le_bytes());
        local.extend_from_slice(&dos_time.to_le_bytes());
        local.extend_from_slice(&dos_date.to_le_bytes());
        local.extend_from_slice(&crc.to_le_bytes());
        local.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        local.extend_from_slice(&(contents.len() as u32).to_le_bytes());
        local.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(name_bytes);
        archive.extend_from_slice(&local);
        archive.extend_from_slice(&compressed);

        directory.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        directory.extend_from_slice(&20u16.to_le_bytes());
        directory.extend_from_slice(&20u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&8u16.to_le_bytes());
        directory.extend_from_slice(&dos_time.to_le_bytes());
        directory.extend_from_slice(&dos_date.to_le_bytes());
        directory.extend_from_slice(&crc.to_le_bytes());
        directory.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        directory.extend_from_slice(&(contents.len() as u32).to_le_bytes());
        directory.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        // Extra field, comment, disk number, internal and external
        // attributes.
        directory.extend_from_slice(&[0u8; 12]);
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name_bytes);
    }
    let directory_offset = archive.len() as u32;
    archive.extend_from_slice(&directory);
    archive.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    archive.extend_from_slice(&[0u8; 4]);
    archive.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    archive.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    archive.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    archive.extend_from_slice(&directory_offset.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes());
    Ok(archive)
}

/// The time now (UTC) in MS-DOS form (two-second steps).
fn dos_date_time() -> (u16, u16) {
    let seconds = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|elapsed| elapsed.as_secs()).unwrap_or(0);
    let days = (seconds / 86_400) as i64;
    let of_day = seconds % 86_400;
    let (hour, minute, second) = (of_day / 3600, of_day / 60 % 60, of_day % 60);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let time = ((hour << 11) | (minute << 5) | (second / 2)) as u16;
    let dos_year = (year - 1980).clamp(0, 127) as u64;
    let date = ((dos_year << 9) | ((month as u64) << 5) | day as u64) as u16;
    (time, date)
}

/// RFC 4648 base32 without padding (padding is accepted).
fn decode_base32(text: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut bytes = Vec::with_capacity(text.len() * 5 / 8);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for character in text.bytes().filter(|character| *character != b'=') {
        let value = ALPHABET.iter().position(|candidate| *candidate == character)? as u32;
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            bytes.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(bytes)
}

/// `data.bin`'s text to the asset's JSON.
pub fn decode_macro_data(text: &str) -> Result<String, MacroError> {
    let base64_text = decode_base32(text).ok_or(MacroError::Encoding)?;
    let packed = base64::engine::general_purpose::STANDARD.decode(base64_text).map_err(|_| MacroError::Encoding)?;
    if packed.len() < 4 {
        return Err(MacroError::Encoding);
    }
    let mut json = String::new();
    let mut decoder = flate2::read::GzDecoder::new(&packed[4..]);
    decoder.read_to_string(&mut json).map_err(|_| MacroError::Encoding)?;
    Ok(json)
}

fn u16_at(bytes: &[u8], offset: usize) -> Option<usize> {
    let pair = bytes.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([pair[0], pair[1]]) as usize)
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<usize> {
    let quad = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([quad[0], quad[1], quad[2], quad[3]]) as usize)
}

/// The files of a zip archive (stored or deflated), by name.
fn read_zip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, MacroError> {
    const END_OF_DIRECTORY: u32 = 0x0605_4b50;
    const DIRECTORY_ENTRY: u32 = 0x0201_4b50;
    const LOCAL_HEADER: u32 = 0x0403_4b50;
    let signature_at = |offset: usize, signature: u32| u32_at(bytes, offset) == Some(signature as usize);

    let search_start = bytes.len().saturating_sub(22 + 65535);
    let end = (search_start..bytes.len().saturating_sub(21)).rev().find(|offset| signature_at(*offset, END_OF_DIRECTORY));
    let end = end.ok_or(MacroError::Archive)?;
    let count = u16_at(bytes, end + 10).ok_or(MacroError::Archive)?;
    let mut entry = u32_at(bytes, end + 16).ok_or(MacroError::Archive)?;

    let mut files = Vec::with_capacity(count);
    for _ in 0..count {
        if !signature_at(entry, DIRECTORY_ENTRY) {
            return Err(MacroError::Archive);
        }
        let method = u16_at(bytes, entry + 10).ok_or(MacroError::Archive)?;
        let compressed_size = u32_at(bytes, entry + 20).ok_or(MacroError::Archive)?;
        let name_length = u16_at(bytes, entry + 28).ok_or(MacroError::Archive)?;
        let extra_length = u16_at(bytes, entry + 30).ok_or(MacroError::Archive)?;
        let comment_length = u16_at(bytes, entry + 32).ok_or(MacroError::Archive)?;
        let local = u32_at(bytes, entry + 42).ok_or(MacroError::Archive)?;
        let name_bytes = bytes.get(entry + 46..entry + 46 + name_length).ok_or(MacroError::Archive)?;
        let name = String::from_utf8_lossy(name_bytes).into_owned();
        entry += 46 + name_length + extra_length + comment_length;

        if !signature_at(local, LOCAL_HEADER) {
            return Err(MacroError::Archive);
        }
        let local_name_length = u16_at(bytes, local + 26).ok_or(MacroError::Archive)?;
        let local_extra_length = u16_at(bytes, local + 28).ok_or(MacroError::Archive)?;
        let start = local + 30 + local_name_length + local_extra_length;
        let data = bytes.get(start..start + compressed_size).ok_or(MacroError::Archive)?;
        let contents = match method {
            0 => data.to_vec(),
            8 => {
                let mut inflated = Vec::new();
                flate2::read::DeflateDecoder::new(data).read_to_end(&mut inflated)?;
                inflated
            }
            _ => return Err(MacroError::Archive),
        };
        files.push((name, contents));
    }
    Ok(files)
}

/// Runs a macro's graph for a `Macro` node (`Macro.ExecuteMacro`) in
/// `engine`, a fresh engine of the building one's kind (the current asset
/// is swapped, so the macro sees its own terrain size and variables):
/// the node's variable values over the macro's, the inputs given to the
/// input `MacroPort`s by name, the output `MacroPort`s built. Returns each
/// output port's map by name.
pub(crate) fn run_macro(engine: &mut Engine, asset: &MacroAsset, variables: &IndexMap<String, Variable>, inputs: &[(String, Arc<Map>)], resolution: Resolution) -> Result<Vec<(String, Map)>, NodeError> {
    engine.building_component = true;
    engine.automation = asset.asset.automation.clone();
    for (name, variable) in variables {
        engine.automation.variables.insert(name.clone(), variable.clone());
    }

    let terrain = &asset.asset.terrain;
    let direction = |node: &NodeData| node.values.get("Direction").and_then(Value::as_str).unwrap_or("Input").to_string();
    let ports: Vec<&NodeData> = terrain.nodes.values().filter(|node| node.type_name == "MacroPort").collect();
    for port in ports.iter().filter(|port| direction(port) == "Input") {
        let given = inputs.iter().find(|(name, _)| *name == port.name);
        if let Some((_, map)) = given {
            engine.cache.store(port.id, "Out", map.clone());
            engine.mark_built(port.id);
        }
    }

    let mut results = Vec::new();
    for port in ports.iter().filter(|port| direction(port) == "Output") {
        engine.build(terrain, port.id, resolution);
        let Some(map) = engine.cache.get(port.id, "Out") else {
            return Err(NodeError::Failed("Node failed Object reference not set to an instance of an object.".to_string()));
        };
        results.push((port.name.clone(), (*map).clone()));
    }
    Ok(results)
}

/// Installed macros by id (`Project.MacroSource`).
pub type MacroLibrary = HashMap<String, Arc<MacroAsset>>;

/// The file of the macro `id` in the first of `folders` that has it:
/// `<id>.macro` (how installed macros are named), else any `.macro` file
/// whose metadata has that id.
pub fn find_macro_file(id: &str, folders: &[std::path::PathBuf]) -> Option<std::path::PathBuf> {
    for folder in folders {
        let named = folder.join(format!("{id}.macro"));
        if named.is_file() {
            return Some(named);
        }
        let Ok(entries) = std::fs::read_dir(folder) else {
            continue;
        };
        let mut files: Vec<std::path::PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        files.sort();
        let is_macro = |path: &std::path::PathBuf| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("macro"));
        let found = files.into_iter().filter(is_macro).find(|path| MacroAsset::read_metadata(path).is_ok_and(|metadata| metadata.id() == id));
        if found.is_some() {
            return found;
        }
    }
    None
}

/// The macros a project uses, each from the first of `folders` that has
/// it (see [`find_macro_file`]), added to the project as
/// [`load_project_macros`] adds them.
pub fn load_project_macros_from(macro_ids: &[String], folders: &[std::path::PathBuf]) -> (MacroLibrary, Vec<(String, MacroError)>) {
    let offset = 1000 * (macro_ids.len() as i32 + 1);
    let mut library = MacroLibrary::new();
    let mut problems = Vec::new();
    for id in macro_ids {
        let Some(path) = find_macro_file(id, folders) else {
            let missing = std::io::Error::new(std::io::ErrorKind::NotFound, format!("{id}.macro is not installed"));
            problems.push((id.clone(), MacroError::Io(missing)));
            continue;
        };
        match MacroAsset::read(&path) {
            Ok(mut asset) => {
                asset.add_to_project(offset);
                library.insert(id.clone(), Arc::new(asset));
            }
            Err(error) => problems.push((id.clone(), error)),
        }
    }
    (library, problems)
}

/// `M.LoadDependentMacro`: the macros `asset` depends on
/// (`DependentMacros`), each read with what it depends on before it, each
/// once. `find` gives an installed macro's file. A dependency that is not
/// installed stops `asset` from loading (the error); one that cannot be
/// read, or whose own dependencies are not installed, is left out and
/// reported in `problems`.
pub fn load_dependencies(asset: &MacroAsset, find: &dyn Fn(&str) -> Option<PathBuf>, problems: &mut Vec<(String, MacroError)>) -> Result<Vec<MacroAsset>, MacroError> {
    let mut seen = vec![asset.metadata.id()];
    let mut loaded = Vec::new();
    for id in &asset.metadata.dependent_macros {
        let Some(path) = find(id) else {
            return Err(MacroError::MissingDependency(id.clone()));
        };
        let before = loaded.len();
        if let Err(error) = load_dependency(&path, find, &mut seen, &mut loaded, problems) {
            loaded.truncate(before);
            problems.push((id.clone(), error));
        }
    }
    Ok(loaded)
}

/// `M.LoadMacro` for a dependency: what it depends on first (a dependency
/// of its that is not installed fails it), then it. A macro met before (a
/// loop of dependencies included) is not read again.
fn load_dependency(path: &Path, find: &dyn Fn(&str) -> Option<PathBuf>, seen: &mut Vec<String>, loaded: &mut Vec<MacroAsset>, problems: &mut Vec<(String, MacroError)>) -> Result<(), MacroError> {
    let dependency = MacroAsset::read(path)?;
    let id = dependency.metadata.id();
    if seen.contains(&id) {
        return Ok(());
    }
    seen.push(id);
    for nested in &dependency.metadata.dependent_macros {
        let Some(nested_path) = find(nested) else {
            return Err(MacroError::MissingDependency(nested.clone()));
        };
        let before = loaded.len();
        if let Err(error) = load_dependency(&nested_path, find, seen, loaded, problems) {
            loaded.truncate(before);
            problems.push((nested.clone(), error));
        }
    }
    loaded.push(dependency);
    Ok(())
}

/// What the editor reads for a project's macros (`M.EnsureMacros`).
pub struct EnsuredMacros {
    pub library: MacroLibrary,
    /// Dependencies the project did not list, in the order they were
    /// added to it.
    pub added: Vec<String>,
    pub problems: Vec<(String, MacroError)>,
}

/// `M.EnsureMacros` as the editor opens a project: each macro it uses read
/// from the first of `folders` that has it, with the macros it depends on
/// before it (see [`load_dependencies`]); a macro whose dependencies are
/// not installed is not loaded. All are added to the project as
/// [`load_project_macros`] adds them.
pub fn ensure_project_macros(macro_ids: &[String], folders: &[PathBuf]) -> EnsuredMacros {
    let offset = 1000 * (macro_ids.len() as i32 + 1);
    let find = |id: &str| find_macro_file(id, folders);
    let mut ensured = EnsuredMacros { library: MacroLibrary::new(), added: Vec::new(), problems: Vec::new() };
    for id in macro_ids {
        let Some(path) = find(id) else {
            let missing = std::io::Error::new(std::io::ErrorKind::NotFound, format!("{id}.macro is not installed"));
            ensured.problems.push((id.clone(), MacroError::Io(missing)));
            continue;
        };
        let mut asset = match MacroAsset::read(&path) {
            Ok(asset) => asset,
            Err(error) => {
                ensured.problems.push((id.clone(), error));
                continue;
            }
        };
        let dependencies = match load_dependencies(&asset, &find, &mut ensured.problems) {
            Ok(dependencies) => dependencies,
            Err(error) => {
                ensured.problems.push((id.clone(), error));
                continue;
            }
        };
        for mut dependency in dependencies {
            let dependency_id = dependency.metadata.id();
            if ensured.library.contains_key(&dependency_id) {
                continue;
            }
            let listed = macro_ids.contains(&dependency_id) || ensured.added.contains(&dependency_id);
            if !listed {
                ensured.added.push(dependency_id.clone());
            }
            dependency.add_to_project(offset);
            ensured.library.insert(dependency_id, Arc::new(dependency));
        }
        asset.add_to_project(offset);
        ensured.library.insert(id.clone(), Arc::new(asset));
    }
    ensured
}

/// The macros a project uses (`Project.Macros`), read from the macro
/// directory (`<id>.macro`) and added to the project as
/// `M.EnsureMacros` does. Macros that cannot be read are reported.
pub fn load_project_macros(macro_ids: &[String], directory: &Path) -> (MacroLibrary, Vec<(String, MacroError)>) {
    let offset = 1000 * (macro_ids.len() as i32 + 1);
    let mut library = MacroLibrary::new();
    let mut problems = Vec::new();
    for id in macro_ids {
        let path = directory.join(format!("{id}.macro"));
        match MacroAsset::read(&path) {
            Ok(mut asset) => {
                asset.add_to_project(offset);
                library.insert(id.clone(), Arc::new(asset));
            }
            Err(error) => problems.push((id.clone(), error)),
        }
    }
    (library, problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macros_write_and_read_back() {
        let encoded = encode_macro_data("{\"hello\": \"world\", \"n\": 12345}");
        assert_eq!(decode_macro_data(&encoded).unwrap(), "{\"hello\": \"world\", \"n\": 12345}");
        assert_eq!(encode_base32(b"f"), "MY");
        assert_eq!(encode_base32(b"fooba"), "MZXW6YTB");
        assert_eq!(encode_base32(b"foobar"), "MZXW6YTBOI");

        let project_template = r#"{"Assets": {"$values": [{"Terrain": {"Id": "8b2d5f35-3c63-4c1c-9e55-0d2a6f0f9b4b",
            "Nodes": {"101": {"$type": "MACRO_PORT_TYPE", "Id": 101, "Name": "In", "Position": {"X": 0.0, "Y": 0.0},
              "Ports": {"$values": [{"Name": "In", "Type": "PrimaryIn"}, {"Name": "Out", "Type": "PrimaryOut"}]}}},
            "Width": 5000.0, "Height": 2500.0}, "Automation": {"Variables": {}}}]}}"#;
        let macro_port_type = crate::format_names::node_type("MacroPort");
        let project_text = project_template.replace("MACRO_PORT_TYPE", &macro_port_type);
        let project = crate::project::Project::parse(&project_text).unwrap();
        let mut metadata = MacroMetadata::for_export("Ridge", "Ann", "https://example.com", "1.5", "A \"ridge\"", "abc12345");
        metadata.dependent_macros.push("Ann.Other".to_string());
        let metadata_value: Value = serde_json::from_str(&metadata.to_json()).unwrap();
        assert_eq!(metadata_value["Id"], "Ann.Ridge");
        assert_eq!(metadata_value["DependentMacros"]["$values"][0], "Ann.Other");
        let asset_json = crate::writer::to_macro_asset_json(&project.assets[0], &metadata_value);

        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_nanos()).unwrap_or(0);
        let path = std::env::temp_dir().join(format!("nerthus-macro-{nanos}.macro"));
        write_macro_file(&path, &metadata, &asset_json).unwrap();
        let read_metadata = MacroAsset::read_metadata(&path).unwrap();
        let read_asset = MacroAsset::read(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(read_metadata.id(), "Ann.Ridge");
        assert_eq!(read_metadata.description, "A \"ridge\"");
        assert_eq!(read_metadata.version, "1.5");
        assert_eq!(read_asset.metadata.source, "abc12345");
        assert_eq!(read_asset.asset.terrain.nodes.len(), 1);
    }

    /// A macro `Author.Name` of one MacroPort depending on `dependencies`,
    /// written as `<id>.macro` in `folder`.
    fn write_test_macro(folder: &Path, author: &str, name: &str, dependencies: &[&str]) {
        let project_template = r#"{"Assets": {"$values": [{"Terrain": {"Id": "8b2d5f35-3c63-4c1c-9e55-0d2a6f0f9b4b",
            "Nodes": {"101": {"$type": "MACRO_PORT_TYPE", "Id": 101, "Name": "In", "Position": {"X": 0.0, "Y": 0.0},
              "Ports": {"$values": [{"Name": "In", "Type": "PrimaryIn"}, {"Name": "Out", "Type": "PrimaryOut"}]}}},
            "Width": 5000.0, "Height": 2500.0}, "Automation": {"Variables": {}}}]}}"#;
        let macro_port_type = crate::format_names::node_type("MacroPort");
        let project_text = project_template.replace("MACRO_PORT_TYPE", &macro_port_type);
        let project = crate::project::Project::parse(&project_text).unwrap();
        let mut metadata = MacroMetadata::for_export(name, author, "", "1.0", "", "source");
        metadata.dependent_macros = dependencies.iter().map(|id| id.to_string()).collect();
        let metadata_value: Value = serde_json::from_str(&metadata.to_json()).unwrap();
        let asset_json = crate::writer::to_macro_asset_json(&project.assets[0], &metadata_value);
        let path = folder.join(format!("{}.macro", metadata.id()));
        write_macro_file(&path, &metadata, &asset_json).unwrap();
    }

    #[test]
    fn dependencies_load_first_and_missing_ones_stop_a_macro() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_nanos()).unwrap_or(0);
        let folder = std::env::temp_dir().join(format!("nerthus-macro-dependencies-{nanos}"));
        std::fs::create_dir_all(&folder).unwrap();
        write_test_macro(&folder, "Ann", "Base", &[]);
        write_test_macro(&folder, "Ann", "Middle", &["Ann.Base"]);
        write_test_macro(&folder, "Ann", "Top", &["Ann.Middle", "Ann.Base"]);
        // A loop of dependencies loads each once.
        write_test_macro(&folder, "Ann", "Loop", &["Ann.Loop"]);
        write_test_macro(&folder, "Ann", "Broken", &["Ann.Gone"]);
        let folders = vec![folder.clone()];

        let ids = vec!["Ann.Top".to_string(), "Ann.Loop".to_string(), "Ann.Broken".to_string()];
        let ensured = ensure_project_macros(&ids, &folders);
        assert_eq!(ensured.added, ["Ann.Base", "Ann.Middle"]);
        let mut loaded: Vec<&String> = ensured.library.keys().collect();
        loaded.sort();
        assert_eq!(loaded, ["Ann.Base", "Ann.Loop", "Ann.Middle", "Ann.Top"]);
        let problems: Vec<String> = ensured.problems.iter().map(|(id, error)| format!("{id}: {error}")).collect();
        assert_eq!(problems, ["Ann.Broken: Dependent macro [Ann.Gone] was not found. Install this macro first then try again."]);

        // The order a macro brings its dependencies in: deepest first.
        let top = MacroAsset::read(&folder.join("Ann.Top.macro")).unwrap();
        let find = |id: &str| find_macro_file(id, &folders);
        let mut problems = Vec::new();
        let dependencies = load_dependencies(&top, &find, &mut problems).unwrap();
        let order: Vec<String> = dependencies.iter().map(|dependency| dependency.metadata.id()).collect();
        assert_eq!(order, ["Ann.Base", "Ann.Middle"]);
        assert!(problems.is_empty());
        std::fs::remove_dir_all(&folder).ok();
    }

    #[test]
    fn versions_compare_like_system_version() {
        let parse = |text| parse_version(text).unwrap();
        assert_eq!(compare_versions(&parse("2.3.1.0"), &parse("2.3.1")), std::cmp::Ordering::Greater);
        assert_eq!(compare_versions(&parse("2.2.9.9"), &parse("2.3")), std::cmp::Ordering::Less);
        assert!(parse_version("2").is_none());
    }

    #[test]
    fn data_bin_decodes() {
        // A stored encoding of {"hello": "world", "n": 12345}.
        let encoded = "JBTUCQKBIIVUYQ2BIFAUCQKBIFAUET3SKZZXASL2MNXEUVRXJJJVKQ3SKBGDQ4CKKVSEUUSVJVXUIODHPFHGURJRJVQXORTFGJ3TMSKIM5AUCQKBHU6Q";
        let json = decode_macro_data(encoded).unwrap();
        assert_eq!(json, "{\"hello\": \"world\", \"n\": 12345}");
    }
}
