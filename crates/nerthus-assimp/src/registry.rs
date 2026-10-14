//! Assimp's importer registry (`GetImporterInstanceList`) as the native
//! build has it — every importer but C4D, M3D and USD — with each importer's
//! extensions and `CanRead`, and `Importer::ReadFile`'s choice among them:
//! the one importer claiming the file's extension, else the first claimant
//! whose `CanRead` accepts the file, else the first importer of all whose
//! `CanRead` does.

use crate::scene::{ImportError, Scene, fail};

/// The file being imported, read whole.
pub(crate) struct FileInput<'a> {
    pub path: &'a str,
    pub bytes: &'a [u8],
}

type CanRead = fn(&FileInput) -> Result<bool, ImportError>;
type Read = fn(&FileInput) -> Result<Scene, ImportError>;

/// One importer: its `aiImporterDesc` name and extensions, `CanRead`
/// (always asked with `checkSig`, as `ReadFile` asks), and the port of its
/// `InternReadFile` where there is one.
pub(crate) struct Importer {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    pub can_read: CanRead,
    pub read: Option<Read>,
}

/// `AI_MAKE_MAGIC`: the first byte most significant.
const fn make_magic(text: &[u8; 4]) -> u32 {
    ((text[0] as u32) << 24) | ((text[1] as u32) << 16) | ((text[2] as u32) << 8) | text[3] as u32
}

/// `IsGcsVersion` and `StripVersionHash`: a `#` after the last `.`
/// followed only by digits is cut off.
fn strip_version_hash(path: &[u8]) -> &[u8] {
    let Some(hash) = path.iter().rposition(|&byte| byte == b'#') else {
        return path;
    };
    let after_dot = path.iter().rposition(|&byte| byte == b'.').is_some_and(|dot| hash > dot);
    let version = &path[hash + 1..];
    if after_dot && !version.is_empty() && version.iter().all(|byte| byte.is_ascii_digit()) {
        return &path[..hash];
    }
    path
}

/// `BaseImporter::HasExtension`: the path ends in `.` and one of the
/// extensions, in any case.
pub(crate) fn has_extension(path: &str, extensions: &[&str]) -> bool {
    let file = strip_version_hash(path.as_bytes());
    extensions.iter().any(|extension| {
        let suffix_length = extension.len() + 1;
        if suffix_length > file.len() {
            return false;
        }
        let ending = &file[file.len() - suffix_length..];
        ending[0] == b'.' && ending[1..].eq_ignore_ascii_case(extension.as_bytes())
    })
}

/// `BaseImporter::GetExtension`: after the last `.`, lowercase.
pub(crate) fn get_extension(path: &str) -> String {
    let extension = name_extension(path.as_bytes());
    String::from_utf8_lossy(&extension).into_owned()
}

/// `BaseImporter::GetExtension` of a name that need not be UTF-8 (a name
/// in an archive).
pub(crate) fn name_extension(name: &[u8]) -> Vec<u8> {
    let file = strip_version_hash(name);
    match file.iter().rposition(|&byte| byte == b'.') {
        Some(dot) => file[dot + 1..].to_ascii_lowercase(),
        None => Vec::new(),
    }
}

/// `BaseImporter::SearchFileHeaderForToken`: the first `search_bytes`
/// bytes, lowercased, NULs dropped; each token's first occurrence counts
/// when it starts a line (`tokens_sol`) or has no printable character
/// before it (`no_graph_before`), when asked.
pub(crate) fn search_header(bytes: &[u8], tokens: &[&str], search_bytes: usize, tokens_sol: bool, no_graph_before: bool) -> bool {
    let read = &bytes[..bytes.len().min(search_bytes)];
    if read.is_empty() {
        return false;
    }
    let buffer: Vec<u8> = read.iter().map(u8::to_ascii_lowercase).filter(|&byte| byte != 0).collect();
    for token in tokens {
        let wanted = token.to_ascii_lowercase();
        let wanted = wanted.as_bytes();
        let Some(found) = buffer.windows(wanted.len()).position(|window| window == wanted) else {
            continue;
        };
        let before = if found == 0 { None } else { Some(buffer[found - 1]) };
        if no_graph_before && before.is_some_and(|byte| byte.is_ascii_graphic()) {
            continue;
        }
        if !tokens_sol || before.is_none_or(|byte| byte == b'\r' || byte == b'\n') {
            return true;
        }
    }
    false
}

/// `SearchFileHeaderForToken` with its defaults (200 bytes, anywhere).
pub(crate) fn header_has(bytes: &[u8], tokens: &[&str]) -> bool {
    search_header(bytes, tokens, 200, false, false)
}

/// `BaseImporter::CheckMagicToken` for four-byte tokens at `offset`: the
/// little-endian word or its byte swap.
pub(crate) fn check_magic_u32(bytes: &[u8], tokens: &[u32], offset: usize) -> bool {
    let Some(data) = bytes.get(offset..offset + 4) else {
        return false;
    };
    let word = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    tokens.iter().any(|&token| word == token || word == token.swap_bytes())
}

/// `CheckMagicToken` for two-byte tokens.
pub(crate) fn check_magic_u16(bytes: &[u8], tokens: &[u16], offset: usize) -> bool {
    let Some(data) = bytes.get(offset..offset + 2) else {
        return false;
    };
    let half = u16::from_le_bytes([data[0], data[1]]);
    tokens.iter().any(|&token| half == token || half == token.swap_bytes())
}

fn tokens(input: &FileInput, list: &[&str]) -> Result<bool, ImportError> {
    Ok(header_has(input.bytes, list))
}

fn by_extension(input: &FileInput, list: &[&str]) -> Result<bool, ImportError> {
    Ok(has_extension(input.path, list))
}

fn magic(input: &FileInput, list: &[&[u8; 4]], offset: usize) -> Result<bool, ImportError> {
    let words: Vec<u32> = list.iter().map(|text| make_magic(text)).collect();
    Ok(check_magic_u32(input.bytes, &words, offset))
}

fn can_read_x(input: &FileInput) -> Result<bool, ImportError> {
    magic(input, &[b"xof "], 0)
}

fn can_read_obj(input: &FileInput) -> Result<bool, ImportError> {
    let list = ["mtllib", "usemtl", "v ", "vt ", "vn ", "o ", "g ", "s ", "f "];
    Ok(search_header(input.bytes, &list, 200, false, true))
}

fn can_read_amf(input: &FileInput) -> Result<bool, ImportError> {
    tokens(input, &["<amf"])
}

fn can_read_3ds(input: &FileInput) -> Result<bool, ImportError> {
    Ok(check_magic_u16(input.bytes, &[0x4d4d, 0x3dc2], 0))
}

fn can_read_md3(input: &FileInput) -> Result<bool, ImportError> {
    magic(input, &[b"3PDI"], 0)
}

fn can_read_md2(input: &FileInput) -> Result<bool, ImportError> {
    magic(input, &[b"2PDI"], 0)
}

fn can_read_ply(input: &FileInput) -> Result<bool, ImportError> {
    tokens(input, &["ply"])
}

fn can_read_mdl(input: &FileInput) -> Result<bool, ImportError> {
    magic(input, &[b"TSDI", b"QSDI", b"7LDM", b"5LDM", b"4LDM", b"3LDM", b"2LDM", b"OPDI"], 0)
}

fn can_read_ase(input: &FileInput) -> Result<bool, ImportError> {
    tokens(input, &["*3dsmax_asciiexport"])
}

fn can_read_hmp(input: &FileInput) -> Result<bool, ImportError> {
    magic(input, &[b"4PMH", b"5PMH", b"7PMH"], 0)
}

fn can_read_smd(input: &FileInput) -> Result<bool, ImportError> {
    by_extension(input, &["smd", "vta"])
}

fn can_read_mdc(input: &FileInput) -> Result<bool, ImportError> {
    magic(input, &[b"IDPC"], 0)
}

fn can_read_md5(input: &FileInput) -> Result<bool, ImportError> {
    tokens(input, &["MD5Version"])
}

fn can_read_stl(input: &FileInput) -> Result<bool, ImportError> {
    tokens(input, &["STL", "solid"])
}

fn can_read_lwo(input: &FileInput) -> Result<bool, ImportError> {
    magic(input, &[b"LWOB", b"LWO2", b"LXOB"], 8)
}

fn can_read_dxf(input: &FileInput) -> Result<bool, ImportError> {
    Ok(search_header(input.bytes, &["SECTION", "HEADER", "ENDSEC", "BLOCKS"], 32, false, false))
}

fn can_read_nff(input: &FileInput) -> Result<bool, ImportError> {
    by_extension(input, &["nff", "enff"])
}

fn can_read_raw(input: &FileInput) -> Result<bool, ImportError> {
    by_extension(input, &["raw"])
}

fn can_read_sib(input: &FileInput) -> Result<bool, ImportError> {
    by_extension(input, &["sib"])
}

fn can_read_off(input: &FileInput) -> Result<bool, ImportError> {
    Ok(search_header(input.bytes, &["off"], 3, false, false))
}

fn can_read_ac3d(input: &FileInput) -> Result<bool, ImportError> {
    magic(input, &[b"AC3D"], 0)
}

fn can_read_bvh(input: &FileInput) -> Result<bool, ImportError> {
    tokens(input, &["HIERARCHY"])
}

fn can_read_irrmesh(input: &FileInput) -> Result<bool, ImportError> {
    tokens(input, &["irrmesh"])
}

fn can_read_irr(input: &FileInput) -> Result<bool, ImportError> {
    tokens(input, &["irr_scene"])
}

fn can_read_q3d(input: &FileInput) -> Result<bool, ImportError> {
    tokens(input, &["quick3Do", "quick3Ds"])
}

/// `B3DImporter::CanRead`: the text after the last `.` is "b3d" in any
/// case.
fn can_read_b3d(input: &FileInput) -> Result<bool, ImportError> {
    let Some(dot) = input.path.rfind('.') else {
        return Ok(false);
    };
    Ok(input.path[dot + 1..].eq_ignore_ascii_case("b3d"))
}

fn can_read_collada(input: &FileInput) -> Result<bool, ImportError> {
    if let Some(archive) = crate::zip::Archive::open(input.bytes)? {
        let root = crate::collada::read_zae_manifest(&archive)?;
        return Ok(!root.is_empty());
    }
    tokens(input, &["<collada"])
}

fn can_read_terragen(input: &FileInput) -> Result<bool, ImportError> {
    tokens(input, &["terragen"])
}

fn can_read_csm(input: &FileInput) -> Result<bool, ImportError> {
    tokens(input, &["$Filename"])
}

fn can_read_unreal(input: &FileInput) -> Result<bool, ImportError> {
    by_extension(input, &["3d", "uc"])
}

fn can_read_lws(input: &FileInput) -> Result<bool, ImportError> {
    magic(input, &[b"LWSC", b"LWMO"], 0)
}

/// `OgreImporter::CanRead`: an XML mesh with a `<mesh>` tag, or any
/// `.mesh`.
fn can_read_ogre(input: &FileInput) -> Result<bool, ImportError> {
    let lowercase = input.path.to_ascii_lowercase();
    if lowercase.ends_with(".mesh.xml") {
        return tokens(input, &["<mesh>"]);
    }
    Ok(lowercase.ends_with(".mesh"))
}

fn can_read_opengex(input: &FileInput) -> Result<bool, ImportError> {
    tokens(input, &["Metric", "GeometryNode", "VertexArray (attrib", "IndexArray"])
}

fn can_read_ms3d(input: &FileInput) -> Result<bool, ImportError> {
    tokens(input, &["MS3D000000"])
}

fn can_read_cob(input: &FileInput) -> Result<bool, ImportError> {
    tokens(input, &["Caligary"])
}

/// `BlenderImporter::CanRead` (`ParseMagicToken`): "BLENDER" at the start,
/// of the file or of its gzip stream.
fn can_read_blender(input: &FileInput) -> Result<bool, ImportError> {
    if input.bytes.starts_with(b"BLENDER") {
        return Ok(true);
    }
    if !input.bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(false);
    }
    let mut decoder = flate2::read::GzDecoder::new(input.bytes);
    let mut head = [0u8; 7];
    let read = std::io::Read::read_exact(&mut decoder, &mut head);
    Ok(read.is_ok() && &head == b"BLENDER")
}

/// `Q3BSPFileImporter::CanRead` only answers without `checkSig`, which
/// `ReadFile` never asks: the importer is picked by extension alone.
fn can_read_q3bsp(_input: &FileInput) -> Result<bool, ImportError> {
    Ok(false)
}

fn can_read_ndo(input: &FileInput) -> Result<bool, ImportError> {
    Ok(search_header(input.bytes, &["nendo"], 5, false, false))
}

fn can_read_ifc(input: &FileInput) -> Result<bool, ImportError> {
    tokens(input, &["ISO-10303-21"])
}

fn can_read_xgl(input: &FileInput) -> Result<bool, ImportError> {
    tokens(input, &["<world>", "<World>", "<WORLD>"])
}

fn can_read_fbx(input: &FileInput) -> Result<bool, ImportError> {
    tokens(input, &[" \n\r\n "])
}

/// `AssbinImporter::CanRead`: the dump's 19-character signature.
fn can_read_assbin(input: &FileInput) -> Result<bool, ImportError> {
    Ok(input.bytes.starts_with(b"ASSIMP.binary-dump."))
}

fn can_read_gltf1(input: &FileInput) -> Result<bool, ImportError> {
    Ok(crate::gltf1::can_read(input.bytes))
}

fn can_read_gltf2(input: &FileInput) -> Result<bool, ImportError> {
    Ok(crate::gltf2::can_read(input.bytes))
}

/// `D3MFImporter::CanRead`: a zip archive holding the model part.
fn can_read_3mf(input: &FileInput) -> Result<bool, ImportError> {
    let Some(archive) = crate::zip::Archive::open(input.bytes)? else {
        return Ok(false);
    };
    Ok(archive.exists(b"3D/3dmodel.model"))
}

fn can_read_x3d(input: &FileInput) -> Result<bool, ImportError> {
    Ok(get_extension(input.path) == "x3d")
}

fn can_read_mmd(input: &FileInput) -> Result<bool, ImportError> {
    tokens(input, &["PMX "])
}

/// `IQMImporter::CanRead`: the extension, else the 15-byte signature.
fn can_read_iqm(input: &FileInput) -> Result<bool, ImportError> {
    if get_extension(input.path) == "iqm" {
        return Ok(true);
    }
    Ok(input.bytes.starts_with(b"INTERQUAKEMODEL"))
}

fn read_obj(input: &FileInput) -> Result<Scene, ImportError> {
    crate::obj::read(input.path, input.bytes)
}

fn read_collada(input: &FileInput) -> Result<Scene, ImportError> {
    crate::collada::read_file(input.path, input.bytes)
}

fn read_fbx(input: &FileInput) -> Result<Scene, ImportError> {
    crate::fbx::read(input.bytes)
}

fn read_gltf1(input: &FileInput) -> Result<Scene, ImportError> {
    crate::gltf1::read(input.path, input.bytes)
}

fn read_gltf2(input: &FileInput) -> Result<Scene, ImportError> {
    crate::gltf2::read(input.path, input.bytes)
}

fn read_stl(input: &FileInput) -> Result<Scene, ImportError> {
    crate::stl::read(input.bytes)
}

fn read_ply(input: &FileInput) -> Result<Scene, ImportError> {
    crate::ply::read(input.bytes)
}

fn read_off(input: &FileInput) -> Result<Scene, ImportError> {
    crate::off::read(input.bytes)
}

fn read_terragen(input: &FileInput) -> Result<Scene, ImportError> {
    crate::terragen::read(input.bytes)
}

fn read_csm(input: &FileInput) -> Result<Scene, ImportError> {
    crate::csm::read(input.bytes)
}

fn read_raw(input: &FileInput) -> Result<Scene, ImportError> {
    crate::raw::read(input.bytes)
}

fn read_ndo(input: &FileInput) -> Result<Scene, ImportError> {
    crate::ndo::read(input.bytes)
}

fn read_iqm(input: &FileInput) -> Result<Scene, ImportError> {
    crate::iqm::read(input.bytes)
}

fn read_unreal(input: &FileInput) -> Result<Scene, ImportError> {
    crate::unreal::read(input.path)
}

fn read_q3d(input: &FileInput) -> Result<Scene, ImportError> {
    crate::q3d::read(input.bytes)
}

fn read_bvh(input: &FileInput) -> Result<Scene, ImportError> {
    crate::bvh::read(input.bytes)
}

fn read_ms3d(input: &FileInput) -> Result<Scene, ImportError> {
    crate::ms3d::read(input.bytes)
}

fn read_b3d(input: &FileInput) -> Result<Scene, ImportError> {
    crate::b3d::read(input.bytes)
}

fn read_md2(input: &FileInput) -> Result<Scene, ImportError> {
    crate::md2::read(input.bytes)
}

fn read_sib(input: &FileInput) -> Result<Scene, ImportError> {
    crate::sib::read(input.bytes)
}

fn read_xgl(input: &FileInput) -> Result<Scene, ImportError> {
    crate::xgl::read(input.path, input.bytes)
}

fn read_mdc(input: &FileInput) -> Result<Scene, ImportError> {
    crate::mdc::read(input.bytes)
}

fn read_x(input: &FileInput) -> Result<Scene, ImportError> {
    crate::xfile::read(input.bytes)
}

fn read_ase(input: &FileInput) -> Result<Scene, ImportError> {
    crate::ase::read(input.path, input.bytes)
}

fn read_3ds(input: &FileInput) -> Result<Scene, ImportError> {
    crate::discreet3ds::read(input.bytes)
}

fn read_dxf(input: &FileInput) -> Result<Scene, ImportError> {
    crate::dxf::read(input.bytes)
}

fn read_smd(input: &FileInput) -> Result<Scene, ImportError> {
    crate::smd::read(input.path, input.bytes)
}

fn read_lwo(input: &FileInput) -> Result<Scene, ImportError> {
    crate::lwo::read(input.bytes)
}

fn read_mdl(input: &FileInput) -> Result<Scene, ImportError> {
    crate::mdl::read(input.path, input.bytes)
}

fn read_hmp(input: &FileInput) -> Result<Scene, ImportError> {
    crate::hmp::read(input.path, input.bytes)
}

fn read_opengex(input: &FileInput) -> Result<Scene, ImportError> {
    crate::opengex::read(input.bytes)
}

fn read_mmd(input: &FileInput) -> Result<Scene, ImportError> {
    crate::mmd::read(input.path, input.bytes)
}

fn read_irrmesh(input: &FileInput) -> Result<Scene, ImportError> {
    crate::irrmesh::read(input.path, input.bytes)
}

fn read_3mf(input: &FileInput) -> Result<Scene, ImportError> {
    crate::d3mf::read(input.path, input.bytes)
}

fn read_amf(input: &FileInput) -> Result<Scene, ImportError> {
    crate::amf::read(input.path, input.bytes)
}

fn read_assbin(input: &FileInput) -> Result<Scene, ImportError> {
    crate::assbin::read(input.bytes)
}

fn read_cob(input: &FileInput) -> Result<Scene, ImportError> {
    crate::cob::read(input.bytes)
}

fn read_md5(input: &FileInput) -> Result<Scene, ImportError> {
    crate::md5::read(input.path, input.bytes)
}

fn read_md3(input: &FileInput) -> Result<Scene, ImportError> {
    crate::md3::read(input.path, input.bytes)
}

fn read_ac3d(input: &FileInput) -> Result<Scene, ImportError> {
    crate::ac3d::read(input.bytes)
}

fn read_nff(input: &FileInput) -> Result<Scene, ImportError> {
    crate::nff::read(input.path, input.bytes)
}

fn read_q3bsp(input: &FileInput) -> Result<Scene, ImportError> {
    crate::q3bsp::read(input.path, input.bytes)
}

macro_rules! importer {
    ($name:expr, [$($extension:expr),*], $can_read:expr) => {
        Importer { name: $name, extensions: &[$($extension),*], can_read: $can_read, read: None }
    };
    ($name:expr, [$($extension:expr),*], $can_read:expr, $read:expr) => {
        Importer { name: $name, extensions: &[$($extension),*], can_read: $can_read, read: Some($read) }
    };
}

/// `GetImporterInstanceList`, in order.
pub(crate) static IMPORTERS: [Importer; 48] = [
    importer!("Direct3D XFile Importer", ["x"], can_read_x, read_x),
    importer!("Wavefront Object Importer", ["obj"], can_read_obj, read_obj),
    importer!("Additive manufacturing file format(AMF) Importer", ["amf"], can_read_amf, read_amf),
    importer!("Discreet 3DS Importer", ["3ds", "prj"], can_read_3ds, read_3ds),
    importer!("Quake III Mesh Importer", ["md3"], can_read_md3, read_md3),
    importer!("Quake II Mesh Importer", ["md2"], can_read_md2, read_md2),
    importer!("Stanford Polygon Library (PLY) Importer", ["ply"], can_read_ply, read_ply),
    importer!("Quake Mesh / 3D GameStudio Mesh Importer", ["mdl"], can_read_mdl, read_mdl),
    importer!("ASE Importer", ["ase", "ask"], can_read_ase, read_ase),
    importer!("3D GameStudio Heightmap (HMP) Importer", ["hmp"], can_read_hmp, read_hmp),
    importer!("Valve SMD Importer", ["smd", "vta"], can_read_smd, read_smd),
    importer!("Return To Castle Wolfenstein Mesh Importer", ["mdc"], can_read_mdc, read_mdc),
    importer!("Doom 3 / MD5 Mesh Importer", ["md5mesh", "md5camera", "md5anim"], can_read_md5, read_md5),
    importer!("Stereolithography (STL) Importer", ["stl"], can_read_stl, read_stl),
    importer!("LightWave/Modo Object Importer", ["lwo", "lxo"], can_read_lwo, read_lwo),
    importer!("Drawing Interchange Format (DXF) Importer", ["dxf"], can_read_dxf, read_dxf),
    importer!("Neutral File Format Importer", ["enff", "nff"], can_read_nff, read_nff),
    importer!("Raw Importer", ["raw"], can_read_raw, read_raw),
    importer!("Silo SIB Importer", ["sib"], can_read_sib, read_sib),
    importer!("OFF Importer", ["off"], can_read_off, read_off),
    importer!("AC3D Importer", ["ac", "acc", "ac3d"], can_read_ac3d, read_ac3d),
    importer!("BVH Importer (MoCap)", ["bvh"], can_read_bvh, read_bvh),
    importer!("Irrlicht Mesh Reader", ["xml", "irrmesh"], can_read_irrmesh, read_irrmesh),
    importer!("Irrlicht Scene Reader", ["irr", "xml"], can_read_irr),
    importer!("Quick3D Importer", ["q3o", "q3s"], can_read_q3d, read_q3d),
    importer!("BlitzBasic 3D Importer", ["b3d"], can_read_b3d, read_b3d),
    importer!("Collada Importer", ["dae", "xml", "zae"], can_read_collada, read_collada),
    importer!("Terragen Heightmap Importer", ["ter"], can_read_terragen, read_terragen),
    importer!("CharacterStudio Motion Importer (MoCap)", ["csm"], can_read_csm, read_csm),
    importer!("Unreal Mesh Importer", ["3d", "uc"], can_read_unreal, read_unreal),
    importer!("LightWave Scene Importer", ["lws", "mot"], can_read_lws),
    importer!("Ogre3D Mesh Importer", ["mesh", "mesh.xml"], can_read_ogre),
    importer!("Open Game Engine Exchange", ["ogex"], can_read_opengex, read_opengex),
    importer!("Milkshape 3D Importer", ["ms3d"], can_read_ms3d, read_ms3d),
    importer!("TrueSpace Object Importer", ["cob", "scn"], can_read_cob, read_cob),
    importer!("Blender 3D Importer (http://www.blender3d.org)", ["blend"], can_read_blender),
    importer!("Quake III BSP Importer", ["bsp", "pk3"], can_read_q3bsp, read_q3bsp),
    importer!("Nendo Mesh Importer", ["ndo"], can_read_ndo, read_ndo),
    importer!("Industry Foundation Classes (IFC) Importer", ["ifc", "ifczip", "step", "stp"], can_read_ifc),
    importer!("XGL Importer", ["xgl", "zgl"], can_read_xgl, read_xgl),
    importer!("Autodesk FBX Importer", ["fbx"], can_read_fbx, read_fbx),
    importer!("Assimp Binary Importer", ["assbin"], can_read_assbin, read_assbin),
    importer!("glTF Importer", ["gltf", "glb"], can_read_gltf1, read_gltf1),
    importer!("glTF2 Importer", ["gltf", "glb", "vrm"], can_read_gltf2, read_gltf2),
    importer!("3mf Importer", ["3mf"], can_read_3mf, read_3mf),
    importer!("Extensible 3D(X3D) Importer", ["x3d", "x3db"], can_read_x3d),
    importer!("MMD Importer", ["pmx"], can_read_mmd, read_mmd),
    importer!("Inter-Quake Model Importer", ["iqm"], can_read_iqm, read_iqm),
];

/// `Importer::ReadFile`'s choice of importer.
pub(crate) fn choose(input: &FileInput) -> Result<&'static Importer, ImportError> {
    let claimants: Vec<&Importer> = IMPORTERS.iter().filter(|importer| has_extension(input.path, importer.extensions)).collect();
    if claimants.len() == 1 {
        return Ok(claimants[0]);
    }
    for importer in claimants {
        if (importer.can_read)(input)? {
            return Ok(importer);
        }
    }
    for importer in &IMPORTERS {
        if (importer.can_read)(input)? {
            return Ok(importer);
        }
    }
    fail(format!("No suitable reader found for the file format of file \"{}\".", input.path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_match_the_end_in_any_case() {
        assert!(has_extension("model.STL", &["stl"]));
        assert!(has_extension("model.Mesh.XML", &["mesh", "mesh.xml"]));
        assert!(!has_extension("stl", &["stl"]));
        assert!(has_extension("model.glb#123", &["glb"]));
        assert!(!has_extension("model.glb#12a", &["glb"]));
        assert_eq!(get_extension("a/b.TeR"), "ter");
    }

    #[test]
    fn header_tokens_follow_their_conditions() {
        assert!(header_has(b"solid cube\n", &["STL", "solid"]));
        assert!(search_header(b"# comment\nv 1 2 3", &["v "], 200, false, true));
        assert!(!search_header(b"gltf blah", &["f "], 200, false, true));
        assert!(header_has(b"P\0L\0Y\0", &["ply"]));
        assert!(!search_header(b"xxoff", &["off"], 3, false, false));
    }

    #[test]
    fn magic_words_match_either_byte_order() {
        assert!(check_magic_u32(b"IDP2....", &[make_magic(b"2PDI")], 0));
        assert!(check_magic_u32(b"AC3Db", &[make_magic(b"AC3D")], 0));
        assert!(check_magic_u32(b"FORM....LWO2", &[make_magic(b"LWOB"), make_magic(b"LWO2")], 8));
        assert!(check_magic_u16(&[0x4d, 0x4d, 0, 0], &[0x4d4d], 0));
    }
}
