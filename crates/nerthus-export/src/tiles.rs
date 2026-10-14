//! The pieces of Hybrid and tiled builds that work on maps and names: the
//! bucket grid (`Bucket`), a bucket's edge blended into its neighbour's,
//! the margins a baked map gets around a bucket, tile names, the overlap
//! strips tiles share (`TileVertexBorder.Expand`), the folders tiles go to
//! and the raw `.bin` files a Hybrid build keeps its buckets and tiles in
//! between steps.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nerthus_engine::{Map, Resolution};
use nerthus_nodes::transformer::{self, ResizeMethod};

/// `BucketPosition`: where a bucket sits in the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BucketPosition {
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

impl BucketPosition {
    /// The position of column `x`, row `y` in a grid whose
    /// last index is `last`. Anything the tests miss falls back to
    /// `TopLeft` (a one-bucket grid, or edges of a two-bucket one are
    /// corners already).
    pub fn of(x: usize, y: usize, last: usize) -> Self {
        if y == 0 && x == 0 {
            return Self::TopLeft;
        }
        if y == 0 && x == last {
            return Self::TopRight;
        }
        if y == last && x == last {
            return Self::BottomRight;
        }
        if y == last && x == 0 {
            return Self::BottomLeft;
        }
        let top = y < last && y == 0;
        let left = x < last && x == 0;
        let right = y < last && x == last;
        let bottom = x < last && y == last;
        let interior = x > 0 && x < last && y > 0 && y < last;
        if left && !top && !bottom {
            return Self::MiddleLeft;
        }
        if right && !top && !bottom {
            return Self::MiddleRight;
        }
        if top && !left && !right {
            return Self::TopMiddle;
        }
        if bottom && !left && !right {
            return Self::BottomMiddle;
        }
        if interior {
            return Self::Middle;
        }
        Self::TopLeft
    }

    /// Buckets in the first column: no left neighbour to blend with.
    pub fn is_left_edge(self) -> bool {
        matches!(self, Self::TopLeft | Self::MiddleLeft | Self::BottomLeft)
    }

    /// Buckets in the first row: no neighbour above.
    pub fn is_top_edge(self) -> bool {
        matches!(self, Self::TopLeft | Self::TopMiddle | Self::TopRight)
    }
}

/// One square of the build (`Bucket`): its column and row in the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bucket {
    pub x: usize,
    pub y: usize,
    pub position: BucketPosition,
}

impl Bucket {
    /// The bucket to the left (`Int2.Left()`).
    pub fn left(&self) -> (usize, usize) {
        (self.x - 1, self.y)
    }

    /// The bucket above (`Int2.Top()`).
    pub fn top(&self) -> (usize, usize) {
        (self.x, self.y - 1)
    }
}

/// `count` × `count` buckets, row by row.
pub fn buckets(count: usize) -> Vec<Bucket> {
    let last = count.saturating_sub(1);
    let mut grid = Vec::with_capacity(count * count);
    for y in 0..count {
        for x in 0..count {
            let position = BucketPosition::of(x, y, last);
            grid.push(Bucket { x, y, position });
        }
    }
    grid
}

/// The geometry of a bucketed build, from the `BuildDefinition`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BucketGeometry {
    /// `Resolution`: the whole terrain's size.
    pub resolution: usize,
    /// `BucketResolution`.
    pub bucket_resolution: usize,
    /// `EdgeBlending`: the margin as a fraction of the bucket.
    pub edge_blending: f32,
    /// `TileResolution`.
    pub tile_resolution: usize,
}

impl BucketGeometry {
    /// `BuildDefinition.BucketCount`: buckets per side.
    pub fn bucket_count(&self) -> usize {
        self.resolution / self.bucket_resolution
    }

    /// `BuildDefinition.EdgeSize`: the margin on each side, in pixels.
    pub fn edge_size(&self) -> usize {
        let edge = self.bucket_resolution as f32 * self.edge_blending;
        edge as usize
    }

    /// `BuildDefinition.BucketSizeWithMargin`.
    pub fn size_with_margin(&self) -> usize {
        self.bucket_resolution + self.edge_size() * 2
    }

    /// The size each bucket builds at: without margins
    /// when the build has one bucket.
    pub fn build_size(&self) -> usize {
        if self.bucket_count() == 1 {
            self.bucket_resolution
        } else {
            self.size_with_margin()
        }
    }

    /// Tiles per bucket side (`Bucket.GetTileCountPerDimension`).
    pub fn tiles_per_bucket(&self) -> usize {
        self.bucket_resolution / self.tile_resolution
    }
}

/// Perlin's smootherstep, `t³(6t² − 15t + 10)`.
fn smootherstep(t: f32) -> f32 {
    let squared = t * t;
    t * squared * (6.0 * squared - 15.0 * t + 10.0)
}

/// Which side of a bucket `blend_edge` works on (`Side`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Top,
    Left,
}

/// The first `2 · edge` rows (`Top`) or columns (`Left`)
/// of `main` taken from the last `2 · edge` of its neighbour `edge_map`:
/// the first `edge + 1` copied, the rest mixed towards `main` by
/// smootherstep weights that start at 0 and stop short of 1.
pub fn blend_edge(main: &mut Map, edge_map: &Map, side: Side, size_with_margin: usize, edge: usize) {
    let edge_length = edge as f32;
    let start = size_with_margin - edge * 2;
    let width = main.width();
    let edge_width = edge_map.width();
    for channel in 0..main.channel_count() {
        let neighbour = edge_map.channel(channel);
        let target = main.channel_mut(channel);
        let mut step = 0usize;
        for (destination, source) in (start..size_with_margin).enumerate() {
            let weight = smootherstep(step as f32 / edge_length);
            for along in 0..size_with_margin {
                let (target_index, source_index) = match side {
                    Side::Top => (destination * width + along, source * edge_width + along),
                    Side::Left => (along * width + destination, along * edge_width + source),
                };
                let neighbour_value = neighbour[source_index];
                if destination <= edge {
                    target[target_index] = neighbour_value;
                } else {
                    let own = target[target_index];
                    target[target_index] = own * weight + neighbour_value * (1.0 - weight);
                }
            }
            if destination > edge {
                step += 1;
            }
        }
    }
}

/// The padding with every side on: `map` grown by `pad` on each side,
/// the border mirrored from the edge inwards (the edge row repeated once).
pub fn mirror_pad(map: &Map, pad: usize) -> Map {
    let width = map.width();
    let height = map.height();
    let padded_width = width + pad * 2;
    let padded_height = height + pad * 2;
    let resolution = Resolution::new(padded_width, padded_height);
    let mut channels = Vec::with_capacity(map.channel_count());
    for channel in 0..map.channel_count() {
        let source = map.channel(channel);
        let mut padded = vec![0.0f32; resolution.area()];
        for y in 0..padded_height {
            let source_y = mirror_index(y as isize - pad as isize, height);
            for x in 0..padded_width {
                let source_x = mirror_index(x as isize - pad as isize, width);
                padded[y * padded_width + x] = source[source_y * width + source_x];
            }
        }
        channels.push(padded);
    }
    Map::from_channels(resolution, channels)
}

/// The source index the padding's border closures read for `index` (which may
/// lie up to the pad outside `0..length`): `-1 - i` reads `i`, `length + i`
/// reads `length - 1 - i`.
fn mirror_index(index: isize, length: usize) -> usize {
    if index < 0 {
        return (-1 - index) as usize;
    }
    let index = index as usize;
    if index >= length {
        return length - 1 - (index - length);
    }
    index
}

/// The part of a baked map a bucket builds from, with the
/// bucket's margins, resized to the bucket's size with margins. The share
/// of the map a bucket covers comes from the build (`BucketResolution`
/// over the resolution being built), so a bake at any resolution maps
/// onto the same part of the terrain. Buckets off the grid's interior read
/// their margins from a mirrored border.
pub fn crop_baked_for_bucket(map: &Map, bucket: &Bucket, geometry: &BucketGeometry) -> Map {
    let share = geometry.bucket_resolution as f32 / geometry.resolution as f32;
    let bucket_pixels = (map.width() as f32 * share) as usize;
    let margin = (bucket_pixels as f32 * geometry.edge_blending) as usize;
    let size = bucket_pixels + margin * 2;
    let window = Resolution::square(size);
    let cropped = if bucket.position == BucketPosition::Middle {
        let left = bucket.x * bucket_pixels - margin;
        let top = bucket.y * bucket_pixels - margin;
        transformer::crop(map, left, top, window)
    } else {
        let padded = mirror_pad(map, margin);
        let left = bucket.x * bucket_pixels;
        let top = bucket.y * bucket_pixels;
        transformer::crop(&padded, left, top, window)
    };
    let target = Resolution::square(geometry.size_with_margin());
    transformer::resize_with(&cropped, target, ResizeMethod::HighQuality)
}

/// The suffix a tile's files get. `%X%` and `%Y%` become
/// the column and row (from 1 when `one_based`, zero-padded to
/// `pad_digits` when that is over 1); a pattern with `_UDIM` gets the UDIM
/// number instead (`1001 + x + 10·y`), rows counted from the bottom of
/// `udim_rows` when `flip_udim`.
pub fn tile_suffix(x: usize, y: usize, pattern: &str, one_based: bool, pad_digits: usize, udim_rows: usize, flip_udim: bool) -> String {
    if pattern.to_lowercase().contains("_udim") {
        let row = if flip_udim { udim_rows as isize - 1 - y as isize } else { y as isize };
        let udim = 1001 + x as isize + row * 10;
        return replace_ignoring_case(pattern, "_UDIM", &format!("_{udim}"));
    }
    let (column, row) = if one_based { (x + 1, y + 1) } else { (x, y) };
    let column_text = if pad_digits > 1 { format!("{column:0pad_digits$}") } else { column.to_string() };
    let row_text = if pad_digits > 1 { format!("{row:0pad_digits$}") } else { row.to_string() };
    let with_column = pattern.replace("%X%", &column_text);
    with_column.replace("%Y%", &row_text)
}

/// `string.Replace(old, new)`: ordinal, so only the exact `_UDIM` is
/// replaced even though the test before it ignores case.
fn replace_ignoring_case(text: &str, old: &str, new: &str) -> String {
    text.replace(old, new)
}

/// The tile patterns Build Settings offers.
pub const TILE_PATTERN_PRESETS: [&str; 3] = ["_y%Y%_x%X%", "_x%X%_y%Y%", "_UDIM"];

/// `TileVertexBorder.Expand(Map, …)`: the first channel of `source` grown
/// by `strip` columns on the right and rows at the bottom, taken from the
/// neighbouring tiles (or from the tile's own last columns and rows where
/// there is no neighbour). `Map.Data` is copied, so the result has one
/// channel whatever the tile had.
pub fn expand_tile(source: &Map, strip: usize, right: Option<&Map>, bottom: Option<&Map>, bottom_right: Option<&Map>) -> Map {
    let width = source.width();
    let height = source.height();
    let expanded_width = width + strip;
    let expanded_height = height + strip;
    let mut output = vec![0.0f32; expanded_width * expanded_height];
    let source_data = source.data();

    // The tile itself.
    for row in 0..height {
        let from = row * width;
        let to = row * expanded_width;
        output[to..to + width].copy_from_slice(&source_data[from..from + width]);
    }

    // The right strip.
    let (right_data, right_offset) = match right {
        Some(map) => (map.data(), 0),
        None => (source_data, width - strip),
    };
    for row in 0..height {
        let from = row * width + right_offset;
        let to = row * expanded_width + width;
        output[to..to + strip].copy_from_slice(&right_data[from..from + strip]);
    }

    // The bottom strip.
    let (bottom_data, bottom_offset) = match bottom {
        Some(map) => (map.data(), 0),
        None => (source_data, height - strip),
    };
    for row in 0..strip {
        let from = (bottom_offset + row) * width;
        let to = (height + row) * expanded_width;
        output[to..to + width].copy_from_slice(&bottom_data[from..from + width]);
    }

    // The corner.
    let (corner_data, corner_x, corner_y) = match (bottom_right, bottom, right) {
        (Some(map), _, _) => (map.data(), 0, 0),
        (None, Some(map), _) => (map.data(), width - strip, 0),
        (None, None, Some(map)) => (map.data(), 0, height - strip),
        (None, None, None) => (source_data, width - strip, height - strip),
    };
    for row in 0..strip {
        let from = (corner_y + row) * width + corner_x;
        let to = (height + row) * expanded_width + width;
        output[to..to + strip].copy_from_slice(&corner_data[from..from + strip]);
    }

    Map::from_data(Resolution::new(expanded_width, expanded_height), output)
}

/// One tile handed to [`expand_tiles`]: its column and row in the whole
/// tile grid.
pub struct GridTile<'a> {
    pub x: usize,
    pub y: usize,
    pub map: &'a Map,
}

/// `TileVertexBorder.Expand(IReadOnlyList<Tile>, …)`: every tile grown by
/// `strip` from its right, bottom and bottom-right neighbours among
/// `tiles` (only those: a tile whose neighbour is not in the list uses its
/// own edge), in the list's order. Bad arguments are errors.
pub fn expand_tiles(tiles: &[GridTile<'_>], tiles_per_dimension: usize, strip: usize) -> Result<Vec<Map>, String> {
    if tiles_per_dimension == 0 {
        return Err("tilesPerDimension is out of range.".to_string());
    }
    let mut by_position = std::collections::HashMap::with_capacity(tiles.len());
    for tile in tiles {
        if strip == 0 {
            return Err("Strip size must be greater than zero.".to_string());
        }
        if strip >= tile.map.width() || strip >= tile.map.height() {
            return Err(format!("Strip size must be smaller than the map dimensions {}x{}.", tile.map.width(), tile.map.height()));
        }
        if tile.x >= tiles_per_dimension {
            return Err(format!("Tile X coordinate {} is outside the tile grid.", tile.x));
        }
        if tile.y >= tiles_per_dimension {
            return Err(format!("Tile Y coordinate {} is outside the tile grid.", tile.y));
        }
        if by_position.insert((tile.x, tile.y), tile.map).is_some() {
            return Err(format!("Duplicate tile coordinate ({}, {}).", tile.x, tile.y));
        }
    }
    let mut expanded = Vec::with_capacity(tiles.len());
    for tile in tiles {
        let right = by_position.get(&(tile.x + 1, tile.y)).copied();
        let bottom = by_position.get(&(tile.x, tile.y + 1)).copied();
        let bottom_right = by_position.get(&(tile.x + 1, tile.y + 1)).copied();
        for neighbour in [right, bottom, bottom_right].into_iter().flatten() {
            if neighbour.resolution() != tile.map.resolution() {
                return Err(format!("Map must be {} × {}, but was {}x{}.", tile.map.width(), tile.map.height(), neighbour.width(), neighbour.height()));
            }
        }
        expanded.push(expand_tile(tile.map, strip, right, bottom, bottom_right));
    }
    Ok(expanded)
}

/// `OrganizeFiles`: how tiled builds lay out their files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OrganizeFiles {
    /// Everything in the build folder.
    #[default]
    None,
    /// A folder per node.
    NodeSubFolder,
    /// A folder per node with one per port inside.
    NodeAndPortSubFolder,
}

impl OrganizeFiles {
    pub fn from_name(name: &str) -> Self {
        match name {
            "NodeSubFolder" => Self::NodeSubFolder,
            "NodeAndPortSubFolder" => Self::NodeAndPortSubFolder,
            _ => Self::None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::NodeSubFolder => "NodeSubFolder",
            Self::NodeAndPortSubFolder => "NodeAndPortSubFolder",
        }
    }
}

/// The folder a node's port's tiles go to. The parts are
/// joined with `\`, so an empty node name leaves an empty path component
/// (harmless on either system).
pub fn organized_folder(folder: &Path, organize: OrganizeFiles, node_name: &str, port: &str) -> PathBuf {
    match organize {
        OrganizeFiles::None => folder.to_path_buf(),
        OrganizeFiles::NodeSubFolder => folder.join(node_name),
        OrganizeFiles::NodeAndPortSubFolder => folder.join(node_name).join(port),
    }
}

/// The samples as raw little-endian floats, one channel as it is, several
/// interleaved. The
/// file is written fresh (the build's own files are always new or
/// rewritten at the same size or larger).
pub fn write_bin(path: &Path, map: &Map) -> io::Result<()> {
    let samples = if map.channel_count() <= 1 { map.channel(0).to_vec() } else { map.to_interleaved() };
    let mut bytes = Vec::with_capacity(samples.len() * 4);
    for value in samples {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    fs::write(path, bytes)
}

/// A `.bin` read back as a map at `size`: one
/// channel when it holds no more than `size²` samples (its own square size
/// then), otherwise three interleaved channels at `size`.
pub fn read_bin(path: &Path, size: usize) -> io::Result<Map> {
    let samples = read_samples(path)?;
    Ok(samples_to_map(samples, size))
}

/// The floats of a raw file.
pub fn read_samples(path: &Path) -> io::Result<Vec<f32>> {
    let bytes = fs::read(path)?;
    let (words, _) = bytes.as_chunks::<4>();
    let samples = words.iter().map(|word| f32::from_le_bytes(*word)).collect();
    Ok(samples)
}

/// Samples as a map of a known `size`: one channel when they fit in one,
/// three interleaved otherwise.
pub fn samples_to_map(samples: Vec<f32>, size: usize) -> Map {
    if samples.len() <= size * size {
        return single_channel(samples);
    }
    Map::from_interleaved(Resolution::square(size), &samples, 3)
}

/// One channel, square, its size from the sample count
/// (`(int)Math.Sqrt((float)length)`).
pub fn single_channel(samples: Vec<f32>) -> Map {
    nerthus_nodes::graw::single_channel(samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_positions_follow_the_edges() {
        let grid = buckets(3);
        let positions: Vec<BucketPosition> = grid.iter().map(|bucket| bucket.position).collect();
        let expected = [
            BucketPosition::TopLeft,
            BucketPosition::TopMiddle,
            BucketPosition::TopRight,
            BucketPosition::MiddleLeft,
            BucketPosition::Middle,
            BucketPosition::MiddleRight,
            BucketPosition::BottomLeft,
            BucketPosition::BottomMiddle,
            BucketPosition::BottomRight,
        ];
        assert_eq!(positions, expected);
        let two = buckets(2);
        let corners: Vec<BucketPosition> = two.iter().map(|bucket| bucket.position).collect();
        assert_eq!(corners, [BucketPosition::TopLeft, BucketPosition::TopRight, BucketPosition::BottomLeft, BucketPosition::BottomRight]);
    }

    #[test]
    fn suffixes() {
        assert_eq!(tile_suffix(2, 5, "_y%Y%_x%X%", false, 1, 8, true), "_y5_x2");
        assert_eq!(tile_suffix(2, 5, "_y%Y%_x%X%", true, 2, 8, true), "_y06_x03");
        assert_eq!(tile_suffix(1, 0, "_UDIM", false, 1, 4, true), "_1032");
        assert_eq!(tile_suffix(1, 0, "_UDIM", false, 1, 4, false), "_1002");
    }

    #[test]
    fn mirror_padding_repeats_the_edge() {
        let map = Map::from_data(Resolution::square(3), (0..9).map(|value| value as f32).collect());
        let padded = mirror_pad(&map, 2);
        assert_eq!(padded.width(), 7);
        let first_row: Vec<f32> = padded.channel(0)[0..7].to_vec();
        // Row -2 reads row 1; columns -2, -1 read 1, 0; 3, 4 read 2, 1.
        assert_eq!(first_row, vec![4.0, 3.0, 3.0, 4.0, 5.0, 5.0, 4.0]);
    }

    #[test]
    fn expansion_takes_neighbours_or_own_edge() {
        let left = Map::from_data(Resolution::square(2), vec![1.0, 2.0, 3.0, 4.0]);
        let right = Map::from_data(Resolution::square(2), vec![5.0, 6.0, 7.0, 8.0]);
        let tiles = [GridTile { x: 0, y: 0, map: &left }, GridTile { x: 1, y: 0, map: &right }];
        let expanded = expand_tiles(&tiles, 2, 1).unwrap();
        assert_eq!(expanded[0].channel(0), &[1.0, 2.0, 5.0, 3.0, 4.0, 7.0, 3.0, 4.0, 7.0]);
        assert_eq!(expanded[1].channel(0), &[5.0, 6.0, 6.0, 7.0, 8.0, 8.0, 7.0, 8.0, 8.0]);
    }
}
