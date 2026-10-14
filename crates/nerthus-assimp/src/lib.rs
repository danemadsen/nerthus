//! The parts of Assimp that Nerthus's mesh loading uses, ported from
//! Assimp "5.4.3.0 (Commit 97257a3b)":
//! `Importer::ReadFile(path, JoinIdenticalVertices | Triangulate |
//! SortByPType)` for the formats the Object node offers.

mod ac3d;
mod amf;
mod ase;
mod assbin;
mod b3d;
mod base64;
mod batch;
mod bvh;
mod cast;
mod cob;
mod collada;
mod compression;
mod csm;
mod d3mf;
mod discreet3ds;
mod dxf;
pub mod export;
mod fast_atof;
mod fbx;
mod hmp;
mod gltf1;
pub mod gltf2;
mod io;
mod iqm;
mod irr;
mod irrmesh;
pub mod json;
mod line_splitter;
mod lwo;
mod lws;
mod matrix;
mod md2;
mod md3;
mod md5;
mod mmd;
mod mdc;
mod mdl;
mod msvc;
mod ms3d;
mod ndo;
mod nff;
mod node;
mod obj;
mod off;
mod ogre;
mod opengex;
mod parsing;
mod ply;
mod raw;
mod postprocess;
mod q3bsp;
mod q3d;
mod registry;
pub mod scene;
mod shapes;
mod sib;
mod skeleton;
mod smd;
mod smoothing_groups;
mod spatial_sort;
mod subdivision;
mod stl;
mod stream_buffer;
mod stream_reader;
mod terragen;
mod unreal;
mod vector;
mod x3d;
mod xfile;
mod xgl;
mod xml;
mod zip;

pub use scene::{ImportError, Mesh, Scene};

/// `Importer::ReadFile(path, 0x800a)`: the importer chosen as Assimp
/// chooses it (see [`registry`]), then the post-processing. A format
/// whose importer is not ported fails to load.
pub fn read_file(path: &str) -> Result<Scene, ImportError> {
    let Ok(bytes) = std::fs::read(path) else {
        return scene::fail(format!("Unable to open file \"{path}\"."));
    };
    let properties = registry::ImportProperties::default();
    let files = io::FileSystem::new(path, None);
    let state = std::cell::RefCell::default();
    let input = registry::FileInput { path, bytes: &bytes, files, properties: &properties, state: &state };
    let importer = registry::choose(&input)?;
    let Some(read) = importer.read else {
        return scene::fail(format!("{} files cannot be read yet", importer.name));
    };
    let mut scene = read(&input)?;
    postprocess::run(&mut scene)?;
    Ok(scene)
}
