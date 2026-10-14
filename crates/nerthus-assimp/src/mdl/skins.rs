//! MDL skins (`MDLMaterialLoader.cpp`), shared with HMP files: textures
//! decoded as far as their reads, their size and whether all their texels
//! are equal (an MDL5 model's texture coordinates depend on that).

use super::Buffer;
use crate::io::FileSystem;
use crate::scene::{ImportError, check_allocation, fail};

/// Built-in palette entries that repeat an earlier entry's colour: (entry,
/// earlier entry).
const PALETTE_REPEATS: [(u8, u8); 12] = [
    (48, 0),
    (80, 51),
    (81, 52),
    (82, 53),
    (157, 141),
    (158, 142),
    (159, 143),
    (175, 16),
    (192, 111),
    (207, 112),
    (223, 32),
    (247, 76),
];

fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// The palette 8-bit skins are decoded with (`SearchPalette`): a
/// "colormap.lmp" of at least 768 bytes the import can open, else the
/// built-in one.
pub(crate) enum Palette {
    BuiltIn,
    File(Vec<u8>),
}

impl Palette {
    pub(crate) fn search(files: &FileSystem) -> Palette {
        match files.open("colormap.lmp") {
            Some(bytes) if bytes.len() >= 768 => Palette::File(bytes[..768].to_vec()),
            _ => Palette::BuiltIn,
        }
    }

    /// A key equal for entries of equal colour.
    fn key(&self, index: u8) -> u32 {
        match self {
            Palette::BuiltIn => {
                let repeat = PALETTE_REPEATS.iter().find(|&&(entry, _)| entry == index);
                match repeat {
                    Some(&(_, earlier)) => earlier as u32,
                    None => index as u32,
                }
            }
            Palette::File(colors) => {
                let start = index as usize * 3;
                u32::from_le_bytes([colors[start], colors[start + 1], colors[start + 2], 0])
            }
        }
    }
}

/// Lazily searched palette (the search opens a file).
pub(crate) struct Skins<'a> {
    pub(crate) files: &'a FileSystem,
    palette: Option<Palette>,
}

impl<'a> Skins<'a> {
    pub(crate) fn new(files: &'a FileSystem) -> Self {
        Skins { files, palette: None }
    }

    fn palette(&mut self) -> &Palette {
        let files = self.files;
        self.palette.get_or_insert_with(|| Palette::search(files))
    }
}

/// A texture's texels as far as they are read back.
pub(crate) enum Texels {
    /// A key per texel, equal for equal texels.
    Keys(Vec<u32>),
    /// Never written (a colour format the decoder does not know).
    Unwritten,
    /// An embedded DDS file.
    Dds(Vec<u8>),
}

/// An `aiTexture` made from a skin (a DDS one has a height of 0 and its
/// byte count as width).
pub(crate) struct Texture {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) texels: Texels,
}

/// `ReplaceTextureWithColor`: whether all texels are equal (a texture of
/// one texel always is).
pub(crate) fn is_single_color(texture: &Texture) -> Result<bool, ImportError> {
    if texture.height == 0 || texture.width == 0 {
        return Ok(false);
    }
    let count = texture.height.wrapping_mul(texture.width);
    match &texture.texels {
        Texels::Keys(keys) => {
            if count == 0 {
                return undefined("texels compared past an empty texture");
            }
            let first = keys[0];
            Ok(keys.iter().all(|&key| key == first))
        }
        Texels::Unwritten => undefined("uninitialised texels compared"),
        Texels::Dds(_) => Ok(false),
    }
}

/// The MIP map texels after the first level.
fn mip_count(pixels: u32) -> u32 {
    let quarter = pixels >> 2;
    let sixteenth = pixels >> 4;
    let sixty_fourth = pixels >> 6;
    quarter.wrapping_add(sixteenth).wrapping_add(sixty_fourth)
}

/// Keys of `count` texels of `size` bytes each at `position`.
fn texel_keys(buffer: &Buffer, position: i64, count: usize, size: usize) -> Result<Vec<u32>, ImportError> {
    let bytes = buffer.bytes(position, count * size)?;
    let keys = bytes
        .chunks_exact(size)
        .map(|chunk| {
            let mut key = [0u8; 4];
            key[..size].copy_from_slice(chunk);
            u32::from_le_bytes(key)
        })
        .collect();
    Ok(keys)
}

/// `ParseTextureColorData`: the texels of a texture of type `kind` at
/// `position` (when `read`), and the bytes to skip in `skip` (left alone
/// for an unknown type). Sizes are `unsigned int` arithmetic.
#[allow(clippy::too_many_arguments)]
pub(crate) fn parse_texture_color_data(
    buffer: &Buffer,
    skins: &mut Skins,
    position: i64,
    kind: u32,
    skip: &mut u32,
    read: bool,
    width: u32,
    height: u32,
) -> Result<Option<Texels>, ImportError> {
    if read {
        if width != 0 && height > u32::MAX / width {
            return fail("Invalid MDL file. A texture is too big.");
        }
        check_allocation(width as u64 * height as u64, 4)?;
    }
    let pixels = width.wrapping_mul(height);
    let count = pixels as usize;
    let keys = match kind {
        // R5G6B5 and A4R4G4B4 (with MIP maps from 10 on)
        2 | 10 | 3 | 11 => {
            let checked = if kind == 2 || kind == 10 { pixels.wrapping_mul(2) } else { pixels.wrapping_mul(4) };
            buffer.size_check(position.wrapping_add(checked as i64))?;
            let keys = if read { Some(texel_keys(buffer, position, count, 2)?) } else { None };
            *skip = pixels.wrapping_mul(2);
            if kind == 10 || kind == 11 {
                *skip = skip.wrapping_add(mip_count(pixels) << 1);
                buffer.size_check(position.wrapping_add(*skip as i64))?;
            }
            keys
        }
        // R8G8B8
        4 | 12 => {
            buffer.size_check(position.wrapping_add(pixels.wrapping_mul(3) as i64))?;
            let keys = if read { Some(texel_keys(buffer, position, count, 3)?) } else { None };
            *skip = pixels.wrapping_mul(3);
            if kind == 12 {
                *skip = skip.wrapping_add(mip_count(pixels).wrapping_mul(3));
                buffer.size_check(position.wrapping_add(*skip as i64))?;
            }
            keys
        }
        // A8R8G8B8
        5 | 13 => {
            buffer.size_check(position.wrapping_add(pixels.wrapping_mul(4) as i64))?;
            let keys = if read { Some(texel_keys(buffer, position, count, 4)?) } else { None };
            *skip = pixels.wrapping_mul(4);
            if kind == 13 {
                *skip = skip.wrapping_add(mip_count(pixels) << 2);
            }
            keys
        }
        // 8-bit palette indices
        0 => {
            buffer.size_check(position.wrapping_add(pixels as i64))?;
            let keys = if read {
                let indices = buffer.bytes(position, count)?.to_vec();
                let palette = skins.palette();
                Some(indices.iter().map(|&index| palette.key(index)).collect())
            } else {
                None
            };
            *skip = pixels;
            keys
        }
        _ => return Ok(read.then_some(Texels::Unwritten)),
    };
    Ok(keys.map(Texels::Keys))
}

/// `CreateTextureARGB8_3DGS_MDL3`: an 8-bit skin of the header's size.
pub(crate) fn create_texture_mdl3(buffer: &Buffer, skins: &mut Skins, position: i64, width: i32, height: i32) -> Result<Texture, ImportError> {
    // (`size_t` from the `int` product.)
    let length = width.wrapping_mul(height) as i64;
    buffer.size_check(position.wrapping_add(length))?;
    let width = width as u32;
    let height = height as u32;
    if width != 0 && height > u32::MAX / width {
        return fail("Invalid MDL file. A texture is too big.");
    }
    let count = width * height;
    check_allocation(count as u64, 4)?;
    let indices = buffer.bytes(position, count as usize)?.to_vec();
    let palette = skins.palette();
    let keys = indices.iter().map(|&index| palette.key(index)).collect();
    Ok(Texture { width, height, texels: Texels::Keys(keys) })
}

/// `CreateTexture_3DGS_MDL4`: only types 0, 2 and 3; a texture when `skip`
/// does not ask for a skip (`UINT_MAX`).
pub(crate) fn create_texture_mdl4(
    buffer: &Buffer,
    skins: &mut Skins,
    position: i64,
    kind: u32,
    skip: &mut u32,
    width: i32,
    height: i32,
) -> Result<Option<Texture>, ImportError> {
    if kind == 1 || kind > 3 {
        return Ok(None);
    }
    let read = *skip != u32::MAX;
    let width = width as u32;
    let height = height as u32;
    let texels = parse_texture_color_data(buffer, skins, position, kind, skip, read, width, height)?;
    Ok(texels.map(|texels| Texture { width, height, texels }))
}

/// `CreateTexture_3DGS_MDL5`: the size first, then the texels or an
/// embedded DDS file (type 6).
pub(crate) fn create_texture_mdl5(buffer: &Buffer, skins: &mut Skins, position: i64, kind: u32, skip: &mut u32) -> Result<Option<Texture>, ImportError> {
    let read = *skip != u32::MAX;
    buffer.size_check(position.wrapping_add(8))?;
    let width = buffer.u32(position)?;
    let height = buffer.u32(position.wrapping_add(4))?;
    let data = position.wrapping_add(8);

    let texture = if kind == 6 {
        *skip = width;
        buffer.size_check(data.wrapping_add(width as i64))?;
        if read {
            check_allocation(width as u64, 1)?;
            let bytes = buffer.bytes(data, width as usize)?.to_vec();
            Some(Texture { width, height: 0, texels: Texels::Dds(bytes) })
        } else {
            None
        }
    } else {
        let texels = parse_texture_color_data(buffer, skins, data, kind, skip, read, width, height)?;
        texels.map(|texels| Texture { width, height, texels })
    };
    *skip = skip.wrapping_add(8);
    Ok(texture)
}

/// The 8×8 chess pattern a skin without a size gets.
fn chess_pattern() -> Texture {
    let mut keys = Vec::with_capacity(64);
    for y in 0..8u32 {
        for x in 0..8u32 {
            let set = (x % 2 == 0 && y % 2 != 0) || (x % 2 != 0 && y % 2 == 0);
            keys.push(u32::from(set));
        }
    }
    Texture { width: 8, height: 8, texels: Texels::Keys(keys) }
}

/// The MDL7 skin type bits: a material follows, an effect text follows.
const SKIN_MATERIAL: u32 = 0x10;
const SKIN_EFFECT: u32 = 0x20;

/// `sizeof(MDL::Material_MDL7)`.
const MATERIAL_SIZE: i64 = 68;

/// `ParseSkinLump_3DGS_MDL7` after the skin header: a reference, an
/// embedded DDS file, an external file name or texels by the type's low
/// bits, then a material and an effect text by its high bits. `None` when
/// an empty DDS skin returns before the cursor is stored.
pub(crate) fn parse_skin_lump(
    buffer: &Buffer,
    skins: &mut Skins,
    position: i64,
    kind: u32,
    width: u32,
    height: u32,
) -> Result<Option<i64>, ImportError> {
    let masked = kind & 0xF;
    let mut position = position;
    let mut texture = None;
    if masked == 0x1 {
        // A reference to another skin.
    } else if masked == 0x6 {
        if width == 0 {
            return Ok(None);
        }
        buffer.size_check(position.wrapping_add(width as i64))?;
        check_allocation(width as u64, 1)?;
        let bytes = buffer.bytes(position, width as usize)?.to_vec();
        texture = Some(Texture { width, height: 0, texels: Texels::Dds(bytes) });
        position = position.wrapping_add(width as i64);
    } else if masked == 0x7 {
        // The name is copied into an `aiString` (1023 bytes at most).
        let length = buffer.c_string_length(position)?;
        let copied = length.min(1023);
        position = position.wrapping_add(copied as i64 + 1);
    } else if masked != 0 || kind == 0 || (width != 0 && height != 0) {
        if height == 0 || width == 0 {
            texture = Some(chess_pattern());
        } else {
            let mut skip = 0u32;
            let texels = parse_texture_color_data(buffer, skins, position, masked, &mut skip, true, width, height)?;
            if let Some(texels) = texels {
                texture = Some(Texture { width, height, texels });
            }
            position = position.wrapping_add(skip as i64);
        }
    }

    if let Some(texture) = &texture {
        is_single_color(texture)?;
    }
    if kind & SKIN_MATERIAL != 0 {
        position = position.wrapping_add(MATERIAL_SIZE);
        buffer.size_check(position)?;
    }
    if kind & SKIN_EFFECT != 0 {
        buffer.size_check(position)?;
        let length = buffer.i32(position)?;
        position = position.wrapping_add(length as i64 + 4);
        buffer.size_check(position)?;
    }
    buffer.size_check(position)?;
    Ok(Some(position))
}

/// `sizeof(MDL::Skin_MDL7)`.
pub(crate) const SKIN_HEADER_SIZE: i64 = 28;

/// `ParseSkinLump_3DGS_MDL7` with its header (type, width, height and a
/// 16-byte name, read whole only when it does not start with a NUL): the
/// cursor after it, or the skin's start when the lump returned early.
pub(crate) fn parse_skin(buffer: &Buffer, skins: &mut Skins, position: i64) -> Result<i64, ImportError> {
    let kind = buffer.u8(position)? as u32;
    let width = buffer.i32(position.wrapping_add(4))? as u32;
    let height = buffer.i32(position.wrapping_add(8))? as u32;
    let after_header = position.wrapping_add(SKIN_HEADER_SIZE);
    let next = parse_skin_lump(buffer, skins, after_header, kind, width, height)?;
    let name = position.wrapping_add(12);
    if buffer.u8(name)? != 0 {
        buffer.bytes(name, 16)?;
    }
    Ok(next.unwrap_or(position))
}

/// `SkipSkinLump_3DGS_MDL7` (HMP files): the same sizes without reading
/// texels, a whole external file name, and no check after a material.
pub(crate) fn skip_skin_lump(buffer: &Buffer, skins: &mut Skins, position: i64, kind: u32, width: u32, height: u32) -> Result<i64, ImportError> {
    let masked = kind & 0xF;
    let mut position = position;
    if masked == 0x6 {
        position = position.wrapping_add(width as i64);
    }
    if masked == 0x7 {
        let length = buffer.c_string_length(position)?;
        position = position.wrapping_add(length as i64 + 1);
    } else if masked != 0 || kind == 0 {
        let mut skip = 0u32;
        parse_texture_color_data(buffer, skins, position, masked, &mut skip, false, width, height)?;
        position = position.wrapping_add(skip as i64);
    }
    if kind & SKIN_MATERIAL != 0 {
        position = position.wrapping_add(MATERIAL_SIZE);
    }
    if kind & SKIN_EFFECT != 0 {
        buffer.size_check(position.wrapping_add(4))?;
        let length = buffer.i32(position)?;
        position = position.wrapping_add(length as i64 + 4);
        buffer.size_check(position)?;
    }
    Ok(position)
}
