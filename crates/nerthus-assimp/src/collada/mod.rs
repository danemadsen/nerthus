//! Assimp's COLLADA importer (`ColladaLoader` + `ColladaParser`) over
//! pugixml's tree.

mod loader;
mod parser;

use crate::scene::{ImportError, Scene, fail};
use crate::xml;

/// `ColladaLoader::InternReadFile` for a `.dae` file's bytes.
pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    let Ok(document) = xml::parse(bytes) else {
        return fail("Unable to read file, malformed XML");
    };
    let parser = parser::Parser::new(&document)?;
    loader::load(&parser)
}
