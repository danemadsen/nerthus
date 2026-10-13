//! Assimp's FBX importer (FBX 7.1+ ASCII and binary), as far as the mesh
//! loading sees it.

mod converter;
mod document;
mod parser;
mod tokenizer;

use crate::scene::{ImportError, Scene};

/// `FBXImporter::InternReadFile`: the file (plus a NUL) tokenized as
/// binary when it starts with the binary magic, else as text; parsed;
/// read into a document; converted.
pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    let mut input = bytes.to_vec();
    input.push(0);
    let binary = input.starts_with(b"Kaydara FBX Binary");
    let tokens = if binary { tokenizer::tokenize_binary(&input)? } else { tokenizer::tokenize_ascii(&input)? };
    let file = parser::File { input, tokens };
    let root = parser::parse(&file)?;
    let document = document::Document::new(&file, &root)?;
    let meshes = converter::convert(&document)?;
    Ok(Scene { meshes })
}
