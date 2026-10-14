//! Assimp's COLLADA importer (`ColladaLoader` + `ColladaParser`) over
//! pugixml's tree.

mod loader;
mod parser;

use crate::scene::{ImportError, Scene, fail};
use crate::xml;

/// `ColladaParser`'s constructor: a file not named `.dae` that opens as a
/// zip archive is a ZAE, its document named by the manifest; anything
/// else is read as XML.
pub fn read_file(path: &str, bytes: &[u8]) -> Result<Scene, ImportError> {
    if crate::registry::get_extension(path) != "dae"
        && let Some(archive) = crate::zip::Archive::open(bytes)?
    {
        let document_name = read_zae_manifest(&archive)?;
        if document_name.is_empty() {
            return fail("Invalid ZAE");
        }
        let Some(document) = archive.read(&document_name)? else {
            let shown_name = String::from_utf8_lossy(&document_name);
            return fail(format!("Invalid ZAE manifest: '{shown_name}' is missing"));
        };
        return read(&document);
    }
    read(bytes)
}

/// `ColladaParser::ReadZaeManifest`: without a manifest, the first `.dae`
/// in the archive; with one, the text of its first `dae_root` node,
/// URI-decoded. (A manifest without `dae_root` dereferences a null
/// pointer in Assimp; here it fails the import.)
pub fn read_zae_manifest(archive: &crate::zip::Archive) -> Result<Vec<u8>, ImportError> {
    let Some(manifest) = archive.read(b"manifest.xml")? else {
        let first_document = archive.names().into_iter().find(|name| crate::registry::name_extension(name) == b"dae");
        return Ok(first_document.unwrap_or_default());
    };
    let Ok(document) = xml::parse(&manifest) else {
        return Ok(Vec::new());
    };
    let Some(root) = document.find_node("dae_root") else {
        return fail("Invalid ZAE manifest");
    };
    let mut value = root.text().to_vec();
    value.truncate(1023);
    Ok(uri_decode_path(&value))
}

/// `ColladaParser::UriDecodePath`: `file://` dropped, the slash before a
/// drive letter dropped, `%xy` decoded (when three more characters
/// follow the `%`), up to the first NUL.
fn uri_decode_path(text: &[u8]) -> Vec<u8> {
    let mut data = text.to_vec();
    if data.starts_with(b"file://") {
        data.drain(..7);
    }
    if data.len() >= 3 && data[0] == b'/' && data[1].is_ascii_alphabetic() && data[2] == b':' {
        data.remove(0);
    }
    let mut decoded = Vec::with_capacity(data.len());
    let mut index = 0;
    while index < data.len() {
        if data[index] == b'%' && index + 3 < data.len() {
            let mut value = 0u32;
            for &digit in &data[index + 1..index + 3] {
                let Some(nibble) = (digit as char).to_digit(16) else {
                    break;
                };
                value = (value << 4) + nibble;
            }
            decoded.push(value as u8);
            index += 3;
        } else {
            decoded.push(data[index]);
            index += 1;
        }
    }
    if let Some(nul) = decoded.iter().position(|&byte| byte == 0) {
        decoded.truncate(nul);
    }
    decoded
}

/// `ColladaLoader::InternReadFile` for a `.dae` file's bytes.
pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    let Ok(document) = xml::parse(bytes) else {
        return fail("Unable to read file, malformed XML");
    };
    let parser = parser::Parser::new(&document)?;
    loader::load(&parser)
}
