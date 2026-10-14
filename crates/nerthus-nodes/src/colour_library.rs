//! The colour library (`data/color/*.clut`):
//! categories of 512-entry RGB palettes, as SatelliteMap and the texture nodes
//! use them, read from Copernicus Sentinel-2 imagery (`data/color/README.md`
//! says where each one comes from). Each file is an `i32` palette count
//! followed by the palettes' RGB bytes.

use std::collections::HashMap;
use std::sync::OnceLock;

/// The notice the Copernicus Sentinel data terms ask to go with the palettes.
pub const NOTICE: &str = "Contains modified Copernicus Sentinel data 2023–2024";

/// One palette: 512 RGB colours from low to high.
pub type Palette = [[u8; 3]; 512];

const PALETTE_BYTES: usize = 512 * 3;

const FILES: [(&str, &[u8]); 6] = [
    ("Blue", include_bytes!("../data/color/Blue.clut")),
    ("Color", include_bytes!("../data/color/Color.clut")),
    ("Green", include_bytes!("../data/color/Green.clut")),
    ("New", include_bytes!("../data/color/New.clut")),
    ("Rock", include_bytes!("../data/color/Rock.clut")),
    ("Sand", include_bytes!("../data/color/Sand.clut")),
];

fn parse(bytes: &[u8]) -> Vec<Palette> {
    let count_bytes: [u8; 4] = bytes[0..4].try_into().unwrap();
    let count = i32::from_le_bytes(count_bytes).max(0) as usize;
    let mut palettes = Vec::with_capacity(count);
    for index in 0..count {
        let start = 4 + index * PALETTE_BYTES;
        let raw = &bytes[start..start + PALETTE_BYTES];
        let mut palette = [[0u8; 3]; 512];
        let (colours, _) = raw.as_chunks::<3>();
        for (entry, rgb) in palette.iter_mut().zip(colours) {
            *entry = *rgb;
        }
        palettes.push(palette);
    }
    palettes
}

fn library() -> &'static HashMap<&'static str, Vec<Palette>> {
    static LIBRARY: OnceLock<HashMap<&'static str, Vec<Palette>>> = OnceLock::new();
    LIBRARY.get_or_init(|| FILES.iter().map(|(name, bytes)| (*name, parse(bytes))).collect())
}

/// The palettes of a category (the file name without `.clut`).
pub fn category(name: &str) -> Option<&'static [Palette]> {
    library().get(name).map(Vec::as_slice)
}

/// Palette `index` of category `name`.
pub fn palette(name: &str, index: usize) -> Option<&'static Palette> {
    category(name)?.get(index)
}

/// How many palettes the largest category has.
pub fn largest_category_len() -> usize {
    library().values().map(Vec::len).max().unwrap_or(0)
}
