//! Interface icons (`IconX`): Tabler icons drawn by name, stroked
//! with the control's brush. The SVGs (assets/icons, with data/icons.toml
//! naming the file each icon draws; both written by tools/ui/fetch-icons.py)
//! are drawn white and tinted.

use std::collections::HashMap;
use std::sync::OnceLock;

use egui::{Color32, Image, ImageSource, Rect, Ui, Vec2};

/// The SVG file each icon name draws.
const INDEX: &str = include_str!("../data/icons.toml");

/// Every SVG in assets/icons as (file name, bytes), listed by build.rs.
const FILES: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/icon-files.rs"));

/// `IconX`'s default size.
pub const DEFAULT_SIZE: f32 = 16.0;

/// The SVG file each icon name draws, by name.
fn index() -> &'static HashMap<String, String> {
    static INDEX_MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    INDEX_MAP.get_or_init(|| toml::from_str(INDEX).expect("the embedded icon index is valid"))
}

/// SVG bytes by file name, with `currentColor` made white so a tint gives
/// the icon its colour.
fn library() -> &'static HashMap<&'static str, &'static [u8]> {
    static LIBRARY: OnceLock<HashMap<&'static str, &'static [u8]>> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        let mut library = HashMap::new();
        for &(file, svg) in FILES {
            let white = String::from_utf8_lossy(svg).replace("currentColor", "#ffffff");
            let bytes: &'static [u8] = Box::leak(white.into_bytes().into_boxed_slice());
            library.insert(file, bytes);
        }
        library
    })
}

/// The icons with their outlines filled (`IconX.IsFilled`), by file name.
fn filled_library() -> &'static HashMap<&'static str, &'static [u8]> {
    static FILLED: OnceLock<HashMap<&'static str, &'static [u8]>> = OnceLock::new();
    FILLED.get_or_init(|| {
        let mut filled = HashMap::new();
        for (file, bytes) in library() {
            let svg = String::from_utf8_lossy(bytes).replace("fill=\"none\"", "fill=\"#ffffff\"");
            let leaked: &'static [u8] = Box::leak(svg.into_bytes().into_boxed_slice());
            filled.insert(*file, leaked);
        }
        filled
    })
}

/// The SVG file `name` draws, when it is an icon.
fn file_for(name: &str) -> Option<&'static str> {
    let file = index().get(name)?.as_str();
    if !library().contains_key(file) {
        return None;
    }
    Some(file)
}

/// Draws the icon filled into `rect`.
pub fn paint_filled(ui: &Ui, rect: Rect, name: &str, tint: Color32) {
    let Some(file) = file_for(name) else {
        return;
    };
    let bytes = filled_library()[file];
    let uri = format!("bytes://icon-filled/{file}");
    let source = ImageSource::Bytes { uri: uri.into(), bytes: bytes.into() };
    Image::new(source).tint(tint).paint_at(ui, rect);
}

pub fn exists(name: &str) -> bool {
    file_for(name).is_some()
}

/// The icon as an image source (`bytes://icon/<file>`; the icons that draw
/// the same file share it).
pub fn source(name: &str) -> Option<ImageSource<'static>> {
    let file = file_for(name)?;
    let bytes = library()[file];
    let uri = format!("bytes://icon/{file}");
    Some(ImageSource::Bytes { uri: uri.into(), bytes: bytes.into() })
}

/// The icon at `size` points, tinted.
pub fn image(name: &str, size: f32, tint: Color32) -> Option<Image<'static>> {
    let source = source(name)?;
    let image = Image::new(source).tint(tint).fit_to_exact_size(Vec2::splat(size));
    Some(image)
}

/// Draws the icon into `rect` with `ui`'s painter, so it clips to `ui`'s
/// clip rect, not to a narrower painter made from it (`painter_at`).
pub fn paint(ui: &Ui, rect: Rect, name: &str, tint: Color32) {
    let Some(source) = source(name) else {
        return;
    };
    let image = Image::new(source).tint(tint);
    image.paint_at(ui, rect);
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn index_and_files_agree() {
        for (name, file) in index() {
            assert!(library().contains_key(file.as_str()), "{name} draws {file}, which is not in assets/icons");
        }
        let referenced: HashSet<&str> = index().values().map(String::as_str).collect();
        for &(file, _) in FILES {
            assert!(referenced.contains(file), "assets/icons/{file} is not in data/icons.toml");
        }
    }

    #[test]
    fn aliases_share_a_file() {
        assert!(exists("xmark"));
        assert_eq!(file_for("xmark"), file_for("close-x"));
        assert_eq!(file_for("Pattern"), Some("grid-pattern.svg"));
        assert!(!exists("no-such-icon"));
    }
}
