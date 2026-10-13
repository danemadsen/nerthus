//! Assets: File, Object, FileSequence, TileInput and Export.

use std::path::{Path, PathBuf};

use nerthus_engine::{ApplicationMode, BuildContext, Map, NodeBuilder, NodeError, RenderIntent, Resolution};
use serde_json::{json, Value};

use crate::file_maps::load_map_file;
use crate::mesh_rasterizer::{self, Mesh};
use crate::nodes::transform::mesh_render_settings;
use crate::mesh_port;
use crate::transformer::{self, ResizeMethod};

/// `Path.GetRelativePath(from, to)`: `to` relative to the directory
/// `from`.
fn relative_path(from: &Path, to: &Path) -> PathBuf {
    let from_parts: Vec<_> = from.components().collect();
    let to_parts: Vec<_> = to.components().collect();
    let shared = from_parts.iter().zip(to_parts.iter()).take_while(|(a, b)| a == b).count();
    let mut relative = PathBuf::new();
    for _ in shared..from_parts.len() {
        relative.push("..");
    }
    for part in &to_parts[shared..] {
        relative.push(part);
    }
    relative
}

/// `Path.GetFullPath(path, basePath)`: `path` against `base`, with `.` and
/// `..` resolved.
fn full_path(path: &Path, base: &Path) -> PathBuf {
    let joined = base.join(path);
    let mut resolved = PathBuf::new();
    for part in joined.components() {
        match part {
            std::path::Component::ParentDir => {
                resolved.pop();
            }
            std::path::Component::CurDir => {}
            other => resolved.push(other),
        }
    }
    resolved
}

/// The file a File or Object node reads:
/// with RelativePath, an absolute name is first stored relative to the
/// saved project's folder, and names resolve against it.
fn asset_path(context: &mut BuildContext, mut file_name: String) -> PathBuf {
    let relative = context.bool("RelativePath");
    let project_folder = context.project_path().filter(|path| path.is_file()).and_then(Path::parent).map(Path::to_path_buf);
    if relative && Path::new(&file_name).is_absolute()
        && let Some(folder) = &project_folder {
            let shortened = relative_path(folder, Path::new(&file_name));
            file_name = shortened.to_string_lossy().into_owned();
            context.update_member("FileName", json!(file_name));
        }
    match (&project_folder, relative) {
        (Some(folder), true) => full_path(Path::new(&file_name), folder),
        _ => PathBuf::from(&file_name),
    }
}

/// `MeshPort_Load`: the native loader reads files whose name ends in `.obj`
/// (case sensitive, after the path's last dot) with tinyobjloader and
/// everything else with Assimp.
fn load_mesh(path: &Path) -> Result<Mesh, NodeError> {
    mesh_port::load(path).ok_or_else(|| NodeError::Failed("Unable to load mesh.".to_string()))
}

/// Object: a mesh file fitted into the terrain, rotated, scaled and moved,
/// and rendered as a heightmap (`Object.Build`).
pub struct Object;

impl NodeBuilder for Object {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let file_name = context.string("FileName").to_string();
        let path = asset_path(context, file_name);
        if !path.is_file() {
            return Err(NodeError::Failed("File not found.".to_string()));
        }
        let antialiasing = context.bool("Antialiasing");
        let settings = mesh_render_settings(context, antialiasing);
        let mesh = load_mesh(&path)?;
        let rendered = mesh_rasterizer::render_mesh(&mesh, &settings);
        context.commit(Map::from_data(context.resolution, rendered))
    }
}

/// File: a heightfield (or colour map) read from an image or raw file
/// (`File.Build`), resized to the build, times its input if connected.
/// No file name gives a blank map.
pub struct File;

impl NodeBuilder for File {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let file_name = context.string("FileName").to_string();
        if file_name.is_empty() {
            let blank = Map::filled(context.resolution, 0.0);
            return context.commit(blank);
        }
        let path = asset_path(context, file_name);
        if !path.is_file() {
            return Err(NodeError::Failed("File not found.".to_string()));
        }

        let is_rgb = context.bool("IsRGB");
        let enforce_linear_gamma = context.bool("EnforceLinearGamma");
        // The image read last, kept unless `NeverCache` (the cache entry
        // `(Id, "File")` in `CacheClusters.Bakes`, which Purge Cache drops).
        // A changed file name reads the new file.
        let never_cache = context.bool("NeverCache");
        let read = LoadedFile { path: path.clone(), is_rgb, enforce_linear_gamma, map: None };
        let kept = context.memory::<LoadedFile>().filter(|kept| kept.reads_like(&read));
        let from_cache = kept.is_some();
        let cached_map = kept.and_then(|kept| kept.map.clone());
        let mut map = match cached_map {
            Some(cached) if !never_cache => cached,
            _ => load_map_file(&path, is_rgb, enforce_linear_gamma).map_err(|error| NodeError::Failed(format!("File could not be read. {error}")))?,
        };
        if !never_cache && !from_cache {
            let loaded = LoadedFile { map: Some(map.clone()), ..read };
            context.remember(loaded);
        }
        let is_square = map.width() == map.height();
        if !is_square && context.bool("CropToSquare") {
            let side = map.width().min(map.height());
            let left = (map.width() - side) / 2;
            let top = (map.height() - side) / 2;
            map = transformer::crop(&map, left, top, Resolution::square(side));
        }
        if map.resolution() != context.resolution {
            map = transformer::resize_with(&map, context.resolution, ResizeMethod::HighQuality);
        }
        let intent = if is_rgb { RenderIntent::Color } else { RenderIntent::Heightfield };
        context.render_as(intent);
        if let Some(input) = context.input("In") {
            map = map.multiply(&input);
        }
        let clamp = !context.bool("AllowUnclamped");
        context.commit_with(map, true, clamp)
    }
}

/// The image a `File` node read, as it read it, and from where.
struct LoadedFile {
    path: PathBuf,
    is_rgb: bool,
    enforce_linear_gamma: bool,
    map: Option<Map>,
}

impl LoadedFile {
    /// Whether this was read the way `other` would read it.
    fn reads_like(&self, other: &LoadedFile) -> bool {
        self.path == other.path && self.is_rgb == other.is_rgb && self.enforce_linear_gamma == other.enforce_linear_gamma
    }
}

/// The frames a `FileSequence` loaded (its `cache` field).
struct LoadedFrames(Vec<Map>);

/// A folder's files in the file system's own order, as
/// `Directory.GetFiles` lists them (by name on NTFS, not on every file
/// system).
fn folder_files(folder: &Path) -> Result<Vec<PathBuf>, NodeError> {
    let entries = std::fs::read_dir(folder).map_err(|_| NodeError::Failed(format!("Could not find a part of the path '{}'.", folder.display())))?;
    let files = entries.filter_map(Result::ok).map(|entry| entry.path()).filter(|path| path.is_file()).collect();
    Ok(files)
}

/// FileSequence: one frame of a folder of images (`FileSequence.Build`).
/// The folder is read once and kept; the frame is chosen before that
/// first read, so a node's first build fails and later ones work.
pub struct FileSequence;

impl NodeBuilder for FileSequence {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        if context.bool("PurgeCache") {
            context.remember(LoadedFrames(Vec::new()));
            return Ok(());
        }
        let folder = context.string("Folder").to_string();
        if folder.is_empty() {
            let blank = Map::filled(context.resolution, 0.0);
            return context.commit(blank);
        }
        let files = folder_files(Path::new(&folder))?;
        let kept = context.memory::<LoadedFrames>();
        let kept_count = kept.as_ref().map_or(0, |frames| frames.0.len());
        let index = context.i32("Frame").min(kept_count as i32 - 1);
        let is_rgb = context.bool("IsRGB");

        let frames = match kept {
            Some(frames) if kept_count > 0 => frames,
            _ => {
                let mut loaded = Vec::with_capacity(files.len());
                for file in files {
                    if !file.is_file() {
                        context.remember(LoadedFrames(loaded));
                        return Err(NodeError::Failed("File could not be read. File may not exist!".to_string()));
                    }
                    let map = match load_map_file(&file, is_rgb, false) {
                        Ok(map) => map,
                        Err(error) => {
                            context.remember(LoadedFrames(loaded));
                            return Err(NodeError::Failed(format!("File could not be read. {error}")));
                        }
                    };
                    let resized = if map.resolution() != context.resolution { transformer::resize_with(&map, context.resolution, ResizeMethod::HighQuality) } else { map };
                    loaded.push(resized);
                }
                context.remember(LoadedFrames(loaded.clone()));
                std::sync::Arc::new(LoadedFrames(loaded))
            }
        };
        let Some(frame) = usize::try_from(index).ok().and_then(|index| frames.0.get(index)) else {
            return Err(NodeError::Failed("Index was out of range. Must be non-negative and less than the size of the collection. (Parameter 'index')".to_string()));
        };
        let map = frame.clone();
        let intent = if is_rgb { RenderIntent::Color } else { RenderIntent::Heightfield };
        context.render_as(intent);
        context.commit(map)
    }
}

/// A signed decimal (`-?\d+`) at the start of `text`: its value and length.
fn leading_integer(text: &[char]) -> Option<(i32, usize)> {
    let sign = usize::from(text.first() == Some(&'-'));
    let digits = text[sign..].iter().take_while(|character| character.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let number: String = text[..sign + digits].iter().collect();
    Some((number.parse().ok()?, sign + digits))
}

/// How a family of coordinate tiles is named: the text
/// around an `x<n>` and a `y<n>` token, in either order.
struct TileNaming {
    directory: PathBuf,
    extension: String,
    prefix: String,
    middle: String,
    suffix: String,
    x_first: bool,
    x_char: char,
    y_char: char,
    x_digits: usize,
    y_digits: usize,
}

impl TileNaming {
    /// The first `x<n><non-alphanumerics>y<n>` (or y-first) in a file's
    /// name.
    fn from_file(path: &Path) -> Option<TileNaming> {
        let stem: Vec<char> = path.file_stem()?.to_string_lossy().chars().collect();
        for x_first in [true, false] {
            let (first, second) = if x_first { (['x', 'X'], ['y', 'Y']) } else { (['y', 'Y'], ['x', 'X']) };
            for start in 0..stem.len() {
                if !first.contains(&stem[start]) {
                    continue;
                }
                let Some((_, first_length)) = leading_integer(&stem[start + 1..]) else {
                    continue;
                };
                let middle_start = start + 1 + first_length;
                let middle_length = stem[middle_start..].iter().take_while(|character| !character.is_ascii_alphanumeric()).count();
                let second_start = middle_start + middle_length;
                if second_start >= stem.len() || !second.contains(&stem[second_start]) {
                    continue;
                }
                let Some((_, second_length)) = leading_integer(&stem[second_start + 1..]) else {
                    continue;
                };
                let digits = |from: usize, length: usize| length - usize::from(stem[from] == '-');
                let (x_start, x_length, y_start, y_length) = if x_first {
                    (start, first_length, second_start, second_length)
                } else {
                    (second_start, second_length, start, first_length)
                };
                let naming = TileNaming {
                    directory: path.parent().map(Path::to_path_buf).unwrap_or_default(),
                    extension: path.extension().map(|extension| format!(".{}", extension.to_string_lossy())).unwrap_or_default(),
                    prefix: stem[..start].iter().collect(),
                    middle: stem[middle_start..second_start].iter().collect(),
                    suffix: stem[second_start + 1 + second_length..].iter().collect(),
                    x_first,
                    x_char: stem[x_start],
                    y_char: stem[y_start],
                    x_digits: digits(x_start + 1, x_length),
                    y_digits: digits(y_start + 1, y_length),
                };
                return Some(naming);
            }
        }
        None
    }

    /// A file's tile coordinates, if its name is of this family
    /// (case-insensitive).
    fn coordinates(&self, path: &Path) -> Option<(i32, i32)> {
        let stem: Vec<char> = path.file_stem()?.to_string_lossy().to_lowercase().chars().collect();
        let lower = |text: &str| text.to_lowercase().chars().collect::<Vec<char>>();
        let prefix = lower(&self.prefix);
        let middle = lower(&self.middle);
        let suffix = lower(&self.suffix);
        if !stem.starts_with(&prefix) || !stem.ends_with(&suffix) || stem.len() < prefix.len() + suffix.len() {
            return None;
        }
        let body = &stem[prefix.len()..stem.len() - suffix.len()];
        let (first_char, second_char) = if self.x_first { (self.x_char, self.y_char) } else { (self.y_char, self.x_char) };
        if body.first() != Some(&first_char.to_ascii_lowercase()) {
            return None;
        }
        let (first, first_length) = leading_integer(&body[1..])?;
        let rest = &body[1 + first_length..];
        if !rest.starts_with(&middle) {
            return None;
        }
        let rest = &rest[middle.len()..];
        if rest.first() != Some(&second_char.to_ascii_lowercase()) {
            return None;
        }
        let (second, second_length) = leading_integer(&rest[1..])?;
        if 1 + second_length != rest.len() {
            return None;
        }
        Some(if self.x_first { (first, second) } else { (second, first) })
    }

    /// The file of tile (x, y): the family's name with the
    /// first file's zero padding.
    fn file(&self, x: i32, y: i32) -> PathBuf {
        let padded = |value: i32, digits: usize| {
            if value < 0 {
                format!("-{:0digits$}", value.unsigned_abs())
            } else {
                format!("{value:0digits$}")
            }
        };
        let x_text = format!("{}{}", self.x_char, padded(x, self.x_digits));
        let y_text = format!("{}{}", self.y_char, padded(y, self.y_digits));
        let tokens = if self.x_first { format!("{x_text}{}{y_text}", self.middle) } else { format!("{y_text}{}{x_text}", self.middle) };
        self.directory.join(format!("{}{tokens}{}{}", self.prefix, self.suffix, self.extension))
    }
}

/// The first `(?<!\d)1\d{3}(?!\d)` in a name: a UDIM tile number and
/// where it starts.
fn udim_token(stem: &[char]) -> Option<(i32, usize)> {
    for start in 0..stem.len().saturating_sub(3) {
        let preceded = start > 0 && stem[start - 1].is_ascii_digit();
        let followed = stem.get(start + 4).is_some_and(|character| character.is_ascii_digit());
        let four_digits = stem[start..start + 4].iter().all(|character| character.is_ascii_digit());
        if stem[start] == '1' && four_digits && !preceded && !followed {
            let number: String = stem[start..start + 4].iter().collect();
            return Some((number.parse().ok()?, start));
        }
    }
    None
}

/// Whether a file names UDIM tiles (`_UDIM`, or a first
/// four digits in 1001–1999).
fn is_udim(path: &Path) -> bool {
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    if stem.to_lowercase().contains("_udim") {
        return true;
    }
    let characters: Vec<char> = stem.chars().collect();
    let first_four = (0..characters.len().saturating_sub(3)).find(|&start| characters[start..start + 4].iter().all(|character| character.is_ascii_digit()));
    let Some(start) = first_four else {
        return false;
    };
    let number: String = characters[start..start + 4].iter().collect();
    number.parse::<i32>().is_ok_and(|value| (1001..2000).contains(&value))
}

/// A tile sequence: its tiles with their coordinates,
/// rows first (UDIM: by tile number).
struct TileSequence {
    udim: bool,
    tiles: Vec<(PathBuf, i32, i32)>,
    naming: Option<TileNaming>,
}

fn same_extension(path: &Path, extension: &str) -> bool {
    let own = path.extension().map(|value| format!(".{}", value.to_string_lossy())).unwrap_or_default();
    own.eq_ignore_ascii_case(extension)
}

/// Whether files dropped together are two or more tiles
/// of the sequence the first of them belongs to (the graph then makes one
/// TileInput of them).
pub fn is_tile_set(files: &[PathBuf]) -> bool {
    if files.len() <= 1 || !files[0].is_file() {
        return false;
    }
    let Ok(sequence) = find_tile_sequence(&files[0]) else {
        return false;
    };
    if sequence.tiles.len() <= 1 {
        return false;
    }
    let tiles: std::collections::HashSet<String> = sequence.tiles.iter().map(|(path, _, _)| full_path_key(path)).collect();
    files.iter().all(|file| tiles.contains(&full_path_key(file)))
}

/// `Path.GetFullPath`, compared without case (`OrdinalIgnoreCase`).
fn full_path_key(path: &Path) -> String {
    let full = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    full.to_string_lossy().to_lowercase()
}

fn find_tile_sequence(file: &Path) -> Result<TileSequence, NodeError> {
    let extension = file.extension().map(|value| format!(".{}", value.to_string_lossy())).unwrap_or_default();
    let directory = file.parent().map(Path::to_path_buf).unwrap_or_default();
    let listed = |directory: &Path| -> Vec<PathBuf> {
        let entries = std::fs::read_dir(directory).into_iter().flatten().filter_map(Result::ok);
        let mut files: Vec<PathBuf> = entries.map(|entry| entry.path()).filter(|path| path.is_file() && same_extension(path, &extension)).collect();
        files.sort();
        files
    };

    if is_udim(file) {
        let stem: Vec<char> = file.file_stem().map(|stem| stem.to_string_lossy().chars().collect()).unwrap_or_default();
        let Some((_, start)) = udim_token(&stem) else {
            return Ok(TileSequence { udim: true, tiles: Vec::new(), naming: None });
        };
        let prefix: String = stem[..start].iter().collect::<String>().to_lowercase();
        let suffix: String = stem[start + 4..].iter().collect::<String>().to_lowercase();
        let mut tiles = Vec::new();
        for path in listed(&directory) {
            let name = path.file_stem().map(|stem| stem.to_string_lossy().to_lowercase()).unwrap_or_default();
            if !name.starts_with(&prefix) || !name.ends_with(&suffix) || name.len() != prefix.len() + 4 + suffix.len() {
                continue;
            }
            let digits = &name[prefix.len()..prefix.len() + 4];
            let Ok(number) = digits.parse::<i32>() else {
                continue;
            };
            if !digits.starts_with('1') || !(1001..=1999).contains(&number) {
                continue;
            }
            let x = (number - 1001) % 10;
            let y = (number - 1001) / 10;
            tiles.push((path, x, y));
        }
        tiles.sort_by_key(|(_, x, y)| y * 1000 + x);
        return Ok(TileSequence { udim: true, tiles, naming: None });
    }

    let Some(naming) = TileNaming::from_file(file) else {
        return Ok(TileSequence { udim: false, tiles: Vec::new(), naming: None });
    };
    let mut tiles: Vec<(PathBuf, i32, i32)> = Vec::new();
    for path in listed(&directory) {
        let Some((x, y)) = naming.coordinates(&path) else {
            continue;
        };
        if tiles.iter().any(|(_, other_x, other_y)| (*other_x, *other_y) == (x, y)) {
            return Err(NodeError::Failed(format!("Duplicate tile coordinate x{x}, y{y}: {}", path.display())));
        }
        tiles.push((path, x, y));
    }
    tiles.sort_by_key(|(_, x, y)| (*y, *x));
    Ok(TileSequence { udim: false, tiles, naming: Some(naming) })
}

/// TileInput: a heightfield merged from a sequence of tile files
/// (coordinate-named or UDIM), resized to the build (`TileInput.Build`).
pub struct TileInput;

impl TileInput {
    /// `LoadTileData`.
    fn load_tile(file: &Path, flip_y: bool, enforce_linear_gamma: bool) -> Result<Map, NodeError> {
        if !file.is_file() {
            return Err(NodeError::Failed(format!("Sequence file not found {}", file.display())));
        }
        let map = load_map_file(file, false, enforce_linear_gamma).map_err(|error| NodeError::Failed(format!("File could not be read. {error}")))?;
        Ok(if flip_y { crate::maths::flip_y(&map) } else { map })
    }

    /// `MergeTiles`: the tiles side by side (UDIM rows bottom-up).
    fn merge(tiles: &[(Map, i32, i32)], udim: bool) -> Result<Map, NodeError> {
        let Some((first, _, _)) = tiles.first() else {
            return Err(NodeError::Failed("No tiles were loaded.".to_string()));
        };
        let (tile_width, tile_height) = (first.width(), first.height());
        if tiles.iter().any(|(map, _, _)| map.width() != tile_width || map.height() != tile_height) {
            return Err(NodeError::Failed("All tiles must have identical resolutions.".to_string()));
        }
        let min_x = tiles.iter().map(|tile| tile.1).min().unwrap_or(0);
        let max_x = tiles.iter().map(|tile| tile.1).max().unwrap_or(0);
        let min_y = tiles.iter().map(|tile| tile.2).min().unwrap_or(0);
        let max_y = tiles.iter().map(|tile| tile.2).max().unwrap_or(0);
        let columns = (max_x - min_x + 1) as usize;
        let rows = (max_y - min_y + 1) as usize;
        let mut seen = std::collections::HashSet::new();
        for (_, x, y) in tiles {
            if !seen.insert((*x, *y)) {
                return Err(NodeError::Failed(format!("Duplicate tile coordinate x{x}, y{y}.")));
            }
        }
        if tiles.len() != columns * rows {
            return Err(NodeError::Failed(format!("Tile sequence is incomplete. Expected {} tiles but found {}.", columns * rows, tiles.len())));
        }
        let full_width = tile_width * columns;
        let full_height = tile_height * rows;
        let mut merged = Map::new(Resolution { x: full_width, y: full_height }, 1);
        for (map, x, y) in tiles {
            let column = (x - min_x) as usize;
            let row = if udim { (max_y - y) as usize } else { (y - min_y) as usize };
            let left = (column * tile_width) as f32;
            let top = (row * tile_height) as f32;
            let position = (left / full_width as f32, top / full_height as f32);
            transformer::insert(map, &mut merged, position, transformer::BlendMode::Blend, None, transformer::AnchorPosition::TopLeft);
        }
        Ok(merged)
    }
}

impl NodeBuilder for TileInput {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let mut file_name = context.string("FileName").to_string();
        let relative = context.bool("RelativePath");
        let project_folder = context.project_path().filter(|path| path.is_file()).and_then(Path::parent).map(Path::to_path_buf);
        if relative && Path::new(&file_name).is_absolute()
            && let Some(folder) = &project_folder {
                file_name = relative_path(folder, Path::new(&file_name)).to_string_lossy().into_owned();
                context.update_member("FileName", json!(file_name));
            }
        let path = match (&project_folder, relative) {
            (Some(folder), true) => full_path(Path::new(&file_name), folder),
            _ => PathBuf::from(&file_name),
        };
        if !path.is_file() {
            return Err(NodeError::Failed("Input file not found.".to_string()));
        }
        let sequence = find_tile_sequence(&path)?;
        if sequence.tiles.len() <= 1 {
            return Err(NodeError::Failed("No tile sequence found.".to_string()));
        }

        let flip_y = context.bool("FlipY");
        let enforce_linear_gamma = context.bool("EnforceLinearGamma");
        let mut loaded = Vec::with_capacity(sequence.tiles.len());
        if sequence.udim {
            // Each tile's place comes from the first UDIM number in its
            // name; files without one are skipped.
            for (file, _, _) in &sequence.tiles {
                let stem: Vec<char> = file.file_stem().map(|stem| stem.to_string_lossy().chars().collect()).unwrap_or_default();
                let Some((number, _)) = udim_token(&stem).filter(|(number, _)| (1001..=1999).contains(number)) else {
                    continue;
                };
                let (x, y) = ((number - 1001) % 10, (number - 1001) / 10);
                loaded.push((Self::load_tile(file, flip_y, enforce_linear_gamma)?, x, y));
            }
        } else {
            let min_x = sequence.tiles.iter().map(|tile| tile.1).min().unwrap_or(0);
            let max_x = sequence.tiles.iter().map(|tile| tile.1).max().unwrap_or(0);
            let min_y = sequence.tiles.iter().map(|tile| tile.2).min().unwrap_or(0);
            let max_y = sequence.tiles.iter().map(|tile| tile.2).max().unwrap_or(0);
            let expected = ((max_x - min_x + 1) * (max_y - min_y + 1)) as usize;
            if sequence.tiles.len() != expected {
                return Err(NodeError::Failed(format!("Tile sequence is incomplete. Expected {expected} tiles but found {}.", sequence.tiles.len())));
            }
            let naming = sequence.naming.as_ref().expect("coordinate tiles have a naming");
            for y in min_y..=max_y {
                for x in min_x..=max_x {
                    loaded.push((Self::load_tile(&naming.file(x, y), flip_y, enforce_linear_gamma)?, x, y));
                }
            }
        }
        let mut map = Self::merge(&loaded, sequence.udim)?;
        if map.resolution() != context.resolution {
            map = transformer::resize_with(&map, context.resolution, ResizeMethod::HighQuality);
        }
        context.commit(map)
    }
}

/// Export: passes its input on for the build's export step; command-line
/// builds give it a save definition (`Export.Build`).
pub struct Export;

impl NodeBuilder for Export {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let is_command_line = matches!(context.application_mode(), ApplicationMode::Swarm | ApplicationMode::SwarmAutomated);
        let save_definition = if is_command_line {
            let format = context.enum_name("Format").to_string();
            json!({"Node": context.node_id(), "Filename": "", "Format": format, "IsEnabled": true, "DisabledInProfiles": []})
        } else {
            Value::Null
        };
        let current = context.node.base.get("SaveDefinition").cloned().unwrap_or(Value::Null);
        if current != save_definition {
            context.update_member("SaveDefinition", save_definition);
        }
        let Some(input) = context.input("In") else {
            return Ok(());
        };
        let map = (*input).clone();
        context.render_as(map.info.render_intent);
        context.commit(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_tiles_of_one_sequence_are_a_tile_set() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|duration| duration.as_nanos()).unwrap_or(0);
        let folder = std::env::temp_dir().join(format!("nerthus-tiles-{nanos}"));
        std::fs::create_dir_all(&folder).unwrap();
        for name in ["land_x0_y0.png", "land_x1_y0.png", "land_x0_y1.png", "other.png"] {
            std::fs::write(folder.join(name), b"").unwrap();
        }
        let tile = |name: &str| folder.join(name);
        assert!(is_tile_set(&[tile("land_x0_y0.png"), tile("land_x1_y0.png")]));
        assert!(!is_tile_set(&[tile("land_x0_y0.png"), tile("other.png")]));
        assert!(!is_tile_set(&[tile("land_x0_y0.png")]));
        let _ = std::fs::remove_dir_all(folder);
    }
}
