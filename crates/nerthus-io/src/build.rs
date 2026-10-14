//! The build service: where a build writes and how saved fields become
//! files.

use std::path::{Path, PathBuf, MAIN_SEPARATOR_STR};

use anyhow::Result;
use nerthus_core::project::BuildSettings;
use nerthus_core::{Field, TerrainDefinition};
use nerthus_image::colorspace::Colorspace;
use nerthus_image::image::Image;
use nerthus_image::resize::resize;

use crate::heightmap::{format_path, write_image_file, write_samples, ExportFormat, RangeMode};

/// Values the location template's tokens expand to.
pub struct LocationTokens<'a> {
    /// The project file (`[Filename]`, `[FileLocation]`).
    pub project_file: &'a Path,
    /// The `[Builds]` folder (the `BuildRepo` preference).
    pub builds_root: &'a Path,
    /// `[Resolution]`.
    pub resolution: usize,
}

/// The default `BuildRepo`: `Documents/Nerthus/Builds`.
pub fn default_builds_root() -> PathBuf {
    let user_dirs = directories::UserDirs::new();
    let documents = user_dirs.as_ref().and_then(|dirs| dirs.document_dir());
    let base = documents.map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    base.join("Nerthus").join("Builds")
}

fn user_name() -> String {
    let variable = std::env::var("USER").or_else(|_| std::env::var("USERNAME"));
    variable.unwrap_or_default()
}

#[cfg(not(target_arch = "wasm32"))]
fn machine_name() -> String {
    let host = gethostname::gethostname();
    host.to_string_lossy().into_owned()
}

/// A browser does not tell pages the machine's name.
#[cfg(target_arch = "wasm32")]
fn machine_name() -> String {
    String::new()
}

/// Expands the build location template. A trailing `[+++]` is
/// replaced by the first unused three-digit folder (001, 002, ...).
pub fn resolve_build_folder(template: &str, tokens: &LocationTokens) -> PathBuf {
    let mut location = template.replace('/', "\\");
    if location.is_empty() {
        location = "[Builds]\\[Filename]\\[+++]".to_string();
    }
    location = location.trim_end_matches('\\').to_string();
    let last_part = location.rsplit('\\').next().unwrap_or("");
    let numbered = last_part == "[+++]";
    if numbered {
        location = location.replace("[+++]", "");
        location = location.trim_end_matches('\\').to_string();
    }

    let parent = tokens.project_file.parent().filter(|folder| !folder.as_os_str().is_empty());
    let project_folder = parent.map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    let project_stem = tokens.project_file.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let now = chrono::Local::now();
    let replacements = [
        ("[FileLocation]", project_folder.to_string_lossy().into_owned()),
        ("[Builds]", tokens.builds_root.to_string_lossy().into_owned()),
        ("[Timestamp]", now.format("%Y-%m-%d_%H-%M-%S").to_string()),
        ("[Filename]", project_stem),
        ("[Username]", user_name()),
        // .NET reports the machine name as the domain outside Windows.
        ("[Domain]", machine_name()),
        ("[Machine]", machine_name()),
        ("[Time]", now.format("%H-%M-%S").to_string()),
        ("[Date]", now.format("%Y-%m-%d").to_string()),
        ("[Resolution]", tokens.resolution.to_string()),
    ];
    for (token, value) in replacements {
        location = location.replace(token, &value);
    }

    let folder = PathBuf::from(location.replace('\\', MAIN_SEPARATOR_STR));
    if !numbered {
        return folder;
    }
    let mut number = 1;
    loop {
        let candidate = folder.join(format!("{number:03}"));
        if !candidate.exists() {
            return candidate;
        }
        number += 1;
    }
}

/// How a file is encoded, from the save format or a path's extension.
#[derive(Clone, Debug, PartialEq)]
pub enum FileEncoding {
    Format(ExportFormat),
    /// An extension with no case of its own: Magick picks the encoder (8 bits).
    Other(String),
}

impl FileEncoding {
    pub fn from_extension(extension: &str) -> FileEncoding {
        match ExportFormat::from_extension(extension) {
            Some(format) => FileEncoding::Format(format),
            None => FileEncoding::Other(extension.to_string()),
        }
    }

    /// The raw formats written straight from the float data, without
    /// colourspace tagging (`.exr`, `.raw`, `.r16` and `.r32`).
    fn is_raw_data(&self) -> bool {
        matches!(self, FileEncoding::Format(ExportFormat::Exr32 | ExportFormat::Raw16 | ExportFormat::R16 | ExportFormat::R32))
    }

    /// The path written for `stem` (with the extension the writer ends up with).
    pub fn path(&self, stem: &Path) -> PathBuf {
        match self {
            FileEncoding::Format(format) => format_path(stem, *format),
            FileEncoding::Other(extension) => {
                let mut name = stem.as_os_str().to_owned();
                name.push(".");
                name.push(extension);
                PathBuf::from(name)
            }
        }
    }
}

/// `Build.Mode == 1`: every saved file is split into a grid of tiles.
#[derive(Clone, Debug)]
pub struct TileSplit {
    /// Tiles per side (`TileCount + 2`).
    pub tiles: usize,
    /// Pixels each tile shares with its right and lower neighbours.
    pub overlap: usize,
    /// `[FileName]_y[y]_x[x]` by default.
    pub filename_format: String,
    pub flip_y: bool,
}

/// The build settings read when writing a saved field.
#[derive(Clone, Debug)]
pub struct BuildWriter {
    pub range: RangeMode,
    /// The resolution nodes were built at.
    pub resolution: usize,
    /// The resolution files are written at (`Resolution.Resample`).
    pub resample: usize,
    pub split: Option<TileSplit>,
    /// `Range.ColorSpace` for colour files: 0 linear RGB, 1 sRGB, 2 scRGB.
    pub color_space: i32,
}

impl BuildWriter {
    /// The writer for a project's build settings, built at `resolution`
    /// (the command-line override sets both the build and the resample size).
    pub fn from_settings(settings: &BuildSettings, definition: &TerrainDefinition, resolution: usize) -> BuildWriter {
        let range = &settings.range;
        let range_mode = RangeMode::from_build(range.range_type, range.min, range.max, definition.height, definition.scale);
        let project_resolution = settings.resolution.build.max(1) as usize;
        let resample = if resolution == project_resolution { settings.resolution.resample.max(1) as usize } else { resolution };
        let split = if settings.mode == 1 {
            let tiles = if settings.tile_count == -1 { 3 } else { (settings.tile_count + 2).max(1) as usize };
            let filename_format = if settings.filename_format.is_empty() { "[FileName]_y[y]_x[x]".to_string() } else { settings.filename_format.clone() };
            let tile_split = TileSplit {
                tiles,
                overlap: settings.overlap.max(0) as usize,
                filename_format,
                flip_y: settings.flip_y,
            };
            Some(tile_split)
        } else {
            None
        };
        BuildWriter {
            range: range_mode,
            resolution,
            resample,
            split,
            color_space: range.color_space,
        }
    }

    /// Writes `field` as `stem` + the encoding's extension (or a
    /// grid of tiles named by the filename format). `bitmap` marks colour
    /// ports; `rgba` keeps alpha (the `RGBA` port). Returns the files written.
    pub fn write(&self, field: &Field, stem: &Path, encoding: &FileEncoding, bitmap: bool, rgba: bool) -> Result<Vec<PathBuf>> {
        let size = field.size;
        // Saved colour is packed into one float per pixel; here colour
        // fields are recognisable by their channels whatever the port claims.
        let bitmap = bitmap || field.channels >= 3;
        let mut data = field.data.clone();
        let mut channels = field.channels;
        if !bitmap && channels == 2 {
            let height = field.to_height();
            data = height.data;
            channels = 1;
        }
        if !bitmap {
            self.range.apply(&mut data, size);
        }
        let mut image = Image::from_data(size, size, channels, data);
        if bitmap && !encoding.is_raw_data() {
            self.tag_color_space(&mut image);
        }

        let Some(split) = &self.split else {
            if self.resample != self.resolution {
                image = resize(&image, self.resample, self.resample);
            }
            finish_image(&mut image, bitmap, rgba);
            let path = encoding.path(stem);
            write_image(&image, &path, encoding)?;
            return Ok(vec![path]);
        };

        let tile_size = if self.resample != self.resolution { self.resample } else { self.resolution };
        let tiles_per_side = split.tiles;
        let full_size = tile_size * tiles_per_side;
        let enlarged = resize(&image, full_size, full_size);
        let mut tiles = Vec::with_capacity(tiles_per_side * tiles_per_side);
        for row in 0..tiles_per_side {
            for column in 0..tiles_per_side {
                let x = (column * tile_size) as isize;
                let y = (row * tile_size) as isize;
                tiles.push(enlarged.crop(x, y, tile_size, tile_size));
            }
        }
        if split.overlap > 0 {
            share_overlaps(&mut tiles, tile_size, tiles_per_side, split.overlap);
        }

        let folder = stem.parent().map(Path::to_path_buf).unwrap_or_default();
        let file_name = stem.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let mut written = Vec::with_capacity(tiles.len());
        for (index, tile) in tiles.iter_mut().enumerate() {
            let row = index / tiles_per_side;
            let column = index % tiles_per_side;
            if split.flip_y {
                tile.flip();
            }
            if !rgba {
                tile.alpha_off();
            }
            if !bitmap {
                tile.set_grayscale_type();
                tile.transform_colorspace(Colorspace::LinearGray);
            }
            let tile_name = tile_file_name(&split.filename_format, &file_name, column, row);
            let tile_stem = folder.join(tile_name);
            let path = encoding.path(&tile_stem);
            write_image(tile, &path, encoding)?;
            written.push(path);
        }
        Ok(written)
    }

    /// Magick.NET's `ColorSpace` setter converts colour files to
    /// the build's colourspace (linear for RGB and scRGB).
    fn tag_color_space(&self, image: &mut Image) {
        if image.channels < 3 {
            return;
        }
        if self.color_space == 0 || self.color_space == 2 {
            image.transform_colorspace(Colorspace::LinearRgb);
        }
    }
}

/// The single-file tail of a saved field: alpha off (unless RGBA), and grey files
/// go through Magick's grayscale / linear grey round trip.
fn finish_image(image: &mut Image, bitmap: bool, rgba: bool) {
    if !rgba {
        image.alpha_off();
    }
    if !bitmap {
        image.alpha_off();
        image.set_grayscale_type();
        image.transform_colorspace(Colorspace::LinearGray);
    }
}

fn write_image(image: &Image, path: &Path, encoding: &FileEncoding) -> Result<()> {
    let size = image.width;
    match encoding {
        FileEncoding::Format(format) => write_samples(path, &image.data, size, image.channels, *format),
        FileEncoding::Other(_) => write_image_file(path, &image.data, size, image.channels),
    }
}

/// The filename format with `[FileName]`, `[x]` and `[y]` filled in.
fn tile_file_name(format: &str, file_name: &str, column: usize, row: usize) -> String {
    let with_name = format.replace("[FileName]", file_name);
    let with_x = with_name.replace("[x]", &format!("{column:03}"));
    with_x.replace("[y]", &format!("{row:03}"))
}

/// The tile classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TileType {
    TopLeft,
    TopMiddle,
    TopRight,
    MiddleLeft,
    Middle,
    MiddleRight,
    BottomLeft,
    BottomMiddle,
    BottomRight,
}

/// The class of the tile at `column`, `row` in a grid whose last index is
/// `last`.
fn tile_type(column: usize, row: usize, last: usize) -> TileType {
    if row == 0 && column == 0 {
        return TileType::TopLeft;
    }
    if row == 0 && column == last {
        return TileType::TopRight;
    }
    if row == last && column == last {
        return TileType::BottomRight;
    }
    if row == last && column == 0 {
        return TileType::BottomLeft;
    }
    let top = row < last && row == 0;
    let left = column < last && column == 0;
    let right = row < last && column == last;
    let bottom = column < last && row == last;
    let middle = column > 0 && column < last && row > 0 && row < last;
    if left && !top && !bottom {
        return TileType::MiddleLeft;
    }
    if right && !top && !bottom {
        return TileType::MiddleRight;
    }
    if top && !left && !right {
        return TileType::TopMiddle;
    }
    if bottom && !left && !right {
        return TileType::BottomMiddle;
    }
    if middle {
        return TileType::Middle;
    }
    TileType::TopLeft
}

/// Overlapping tiles. Tiles are classified as if the grid were 3 x 3 (a
/// last index of 2, and `x + 1 < 3`); a tile's last `overlap` columns are
/// taken from its right neighbour's first columns, and its last `overlap`
/// rows all repeat the first row of the tile below (the row counter resets
/// inside the loop). Neighbours outside the grid are skipped rather than
/// failing the whole write.
fn share_overlaps(tiles: &mut [Image], size: usize, tiles_per_side: usize, overlap: usize) {
    let overlap = overlap.min(size);
    for row in 0..tiles_per_side {
        for column in 0..tiles_per_side {
            let kind = tile_type(column, row, 2);
            let (take_right, take_below) = match kind {
                TileType::TopLeft | TileType::TopMiddle | TileType::MiddleLeft | TileType::Middle => (column + 1 < 3, row + 1 < 3),
                TileType::TopRight | TileType::MiddleRight => (false, row + 1 < 3),
                TileType::BottomLeft | TileType::BottomMiddle => (column + 1 < 3, false),
                TileType::BottomRight => (false, false),
            };
            let index = row * tiles_per_side + column;
            if take_right && column + 1 < tiles_per_side {
                let right = tiles[index + 1].clone();
                copy_right_columns(&mut tiles[index], &right, size, overlap);
            }
            if take_below && row + 1 < tiles_per_side {
                let below = tiles[index + tiles_per_side].clone();
                copy_first_row_down(&mut tiles[index], &below, size, overlap);
            }
        }
    }
}

/// A tile's last `overlap` columns taken from its right neighbour's first.
fn copy_right_columns(tile: &mut Image, right: &Image, size: usize, overlap: usize) {
    let channels = tile.channels;
    let start = size - overlap;
    for y in 0..size {
        for x in start..size {
            let source = (y * size + (x - start)) * channels;
            let target = (y * size + x) * channels;
            tile.data[target..target + channels].copy_from_slice(&right.data[source..source + channels]);
        }
    }
}

/// A tile's last `overlap` rows taken from the first row of the tile below.
fn copy_first_row_down(tile: &mut Image, below: &Image, size: usize, overlap: usize) {
    let channels = tile.channels;
    let row_length = size * channels;
    let first_row = &below.data[..row_length];
    for y in size - overlap..size {
        let start = y * row_length;
        tile.data[start..start + row_length].copy_from_slice(first_row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_types_classify_a_three_by_three_grid() {
        assert_eq!(tile_type(0, 0, 2), TileType::TopLeft);
        assert_eq!(tile_type(1, 0, 2), TileType::TopMiddle);
        assert_eq!(tile_type(2, 0, 2), TileType::TopRight);
        assert_eq!(tile_type(0, 1, 2), TileType::MiddleLeft);
        assert_eq!(tile_type(1, 1, 2), TileType::Middle);
        assert_eq!(tile_type(2, 1, 2), TileType::MiddleRight);
        assert_eq!(tile_type(0, 2, 2), TileType::BottomLeft);
        assert_eq!(tile_type(1, 2, 2), TileType::BottomMiddle);
        assert_eq!(tile_type(2, 2, 2), TileType::BottomRight);
    }

    #[test]
    fn numbered_build_folders() {
        let root = std::env::temp_dir().join(format!("nerthus-build-test-{}", std::process::id()));
        std::fs::create_dir_all(root.join("Project").join("001")).unwrap();
        let project = root.join("Project.terrain");
        let tokens = LocationTokens {
            project_file: &project,
            builds_root: &root,
            resolution: 1024,
        };
        let folder = resolve_build_folder("[Builds]\\[Filename]\\[+++]\\", &tokens);
        assert_eq!(folder, root.join("Project").join("002"));
        let plain = resolve_build_folder("[Builds]/out-[Resolution]", &tokens);
        assert_eq!(plain, root.join("out-1024"));
        std::fs::remove_dir_all(&root).ok();
    }
}
