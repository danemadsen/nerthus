//! `FileFormats`: the image formats a build writes, and the writer each
//! format picks.

use std::io;
use std::path::Path;

use nerthus_engine::format_names::PURE_RAW_FORMAT;
use nerthus_engine::Map;

use crate::colorspace::Colorspace;
use crate::{exr, jpeg, png, raw, tiff};

/// `FileFormats`, in the order projects number them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileFormat {
    PureRaw,
    HalfRaw,
    HalfRaw16,
    UshortRaw,
    UshortRaw16,
    FloatRaw,
    FloatRaw32,
    Jpeg,
    Png8,
    Png16,
    Png64,
    Tiff8,
    Tiff16,
    Tiff32,
    Exr,
}

impl FileFormat {
    pub const ALL: [FileFormat; 15] = [
        FileFormat::PureRaw,
        FileFormat::HalfRaw,
        FileFormat::HalfRaw16,
        FileFormat::UshortRaw,
        FileFormat::UshortRaw16,
        FileFormat::FloatRaw,
        FileFormat::FloatRaw32,
        FileFormat::Jpeg,
        FileFormat::Png8,
        FileFormat::Png16,
        FileFormat::Png64,
        FileFormat::Tiff8,
        FileFormat::Tiff16,
        FileFormat::Tiff32,
        FileFormat::Exr,
    ];

    /// The member name `.terrain` files use.
    pub fn name(self) -> &'static str {
        match self {
            FileFormat::PureRaw => PURE_RAW_FORMAT,
            FileFormat::HalfRaw => "HalfRaw",
            FileFormat::HalfRaw16 => "HalfRaw16",
            FileFormat::UshortRaw => "UshortRaw",
            FileFormat::UshortRaw16 => "UshortRaw16",
            FileFormat::FloatRaw => "FloatRaw",
            FileFormat::FloatRaw32 => "FloatRaw32",
            FileFormat::Jpeg => "JPEG",
            FileFormat::Png8 => "PNG8",
            FileFormat::Png16 => "PNG16",
            FileFormat::Png64 => "PNG64",
            FileFormat::Tiff8 => "TIFF8",
            FileFormat::Tiff16 => "TIFF16",
            FileFormat::Tiff32 => "TIFF32",
            FileFormat::Exr => "EXR",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|format| format.name() == name)
    }

    /// The format's position in the enum (how Newtonsoft stores a missing
    /// member: Pure Raw, the first).
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|format| *format == self).unwrap_or(0)
    }

    pub fn extension(self) -> &'static str {
        match self {
            FileFormat::PureRaw => ".graw",
            FileFormat::HalfRaw | FileFormat::UshortRaw | FileFormat::FloatRaw => ".raw",
            FileFormat::HalfRaw16 | FileFormat::UshortRaw16 => ".r16",
            FileFormat::FloatRaw32 => ".r32",
            FileFormat::Jpeg => ".jpg",
            FileFormat::Png8 | FileFormat::Png16 | FileFormat::Png64 => ".png",
            FileFormat::Tiff8 | FileFormat::Tiff16 | FileFormat::Tiff32 => ".tif",
            FileFormat::Exr => ".exr",
        }
    }

    /// The enum's `[Description]`, as the format lists show it.
    pub fn description(self) -> &'static str {
        match self {
            FileFormat::PureRaw => "Pure Raw (.graw)",
            FileFormat::HalfRaw => "Half Raw 16-bit (.raw)",
            FileFormat::HalfRaw16 => "Half Raw 16-bit (.r16)",
            FileFormat::UshortRaw => "Ushort Raw 16-bit (.raw)",
            FileFormat::UshortRaw16 => "Ushort Raw 16-bit (.r16)",
            FileFormat::FloatRaw => "Float Raw 32-bit (.raw)",
            FileFormat::FloatRaw32 => "Float Raw 32-bit (.r32)",
            FileFormat::Jpeg => "JPEG 8-bit (.jpg)",
            FileFormat::Png8 => "PNG 8-bit (.png)",
            FileFormat::Png16 => "PNG 16-bit (.png)",
            FileFormat::Png64 => "PNG 64-bit (.png)",
            FileFormat::Tiff8 => "TIFF 8-bit (.tif)",
            FileFormat::Tiff16 => "TIFF 16-bit (.tif)",
            FileFormat::Tiff32 => "TIFF 32-bit (.tif)",
            FileFormat::Exr => "OpenEXR 32-bit (.exr)",
        }
    }
}

/// What the writers need besides the map.
#[derive(Clone, Debug, PartialEq)]
pub struct WriteOptions {
    /// The TIFF 32-bit `Artist` tag: "Nerthus" and its version by default.
    pub artist: String,
    /// PNG 8-bit's dither colour cache coarseness (`CacheShift`): 2 as in
    /// Windows builds of ImageMagick; Apple builds of ImageMagick use 3.
    pub dither_cache_shift: u32,
    /// The build's `ColorSpace` (a `Colorspace` member name), which the
    /// writers give the Magick.NET images of colour maps written as PNG 8- and
    /// 16-bit, TIFF 8- and 16-bit and JPEG (`Undefined` is what a build
    /// definition without one reads as).
    pub color_space: String,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            artist: format!("Nerthus {}", env!("CARGO_PKG_VERSION")),
            dither_cache_shift: 2,
            color_space: "sRGB".to_string(),
        }
    }
}

/// `Node.Export`'s preparation: every value clamped to
/// [0, 1] as `Vector.Min(Vector.Max(v, 0), 1)` does on x64, so NaN and
/// -0 become 0. The map keeps its info.
pub fn clamp_for_export(map: &Map) -> Map {
    let mut clamped = map.clone();
    clamped.apply(|value| {
        let low = if value > 0.0 { value } else { 0.0 };
        if low < 1.0 { low } else { 1.0 }
    });
    clamped
}

/// Writes a prepared map in `format`.
pub fn write_map(format: FileFormat, path: &Path, map: &Map, options: &WriteOptions) -> io::Result<()> {
    let color_space = Colorspace::from_name(&options.color_space);
    match format {
        // One-channel maps go out in the stored range, which reading a
        // `.graw` stored in another range sets.
        FileFormat::PureRaw => raw::write_graw(path, map, nerthus_nodes::graw::stored_range()),
        FileFormat::HalfRaw | FileFormat::HalfRaw16 => raw::write_half(path, map),
        FileFormat::UshortRaw | FileFormat::UshortRaw16 => raw::write_ushort(path, map),
        FileFormat::FloatRaw | FileFormat::FloatRaw32 => raw::write_float(path, map),
        FileFormat::Jpeg => jpeg::write_jpeg(path, map, color_space),
        FileFormat::Png8 => png::write_png8(path, map, options.dither_cache_shift, &options.color_space),
        FileFormat::Png16 => png::write_png(path, map, Some(color_space)),
        FileFormat::Png64 => png::write_png(path, map, None),
        FileFormat::Tiff8 => tiff::write_magick_tiff(path, map, tiff::SampleDepth::Eight, color_space),
        FileFormat::Tiff16 => tiff::write_magick_tiff(path, map, tiff::SampleDepth::Sixteen, color_space),
        FileFormat::Tiff32 => tiff::write_float_tiff(path, map, &options.artist),
        FileFormat::Exr => exr::write_exr(path, map),
    }
}
