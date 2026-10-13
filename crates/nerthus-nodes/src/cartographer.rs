//! The map drawing helpers (`Cartographer`).

use nerthus_engine::{Map, Resolution};

use crate::aspect::{self, CurvatureType};
use crate::combiner;
use crate::fast_blur;
use crate::maths;
use crate::morphology;
use crate::mosaic;
use crate::profiles;
use crate::transformer::{self, ResizeMethod};

/// Edge lines of a terraced map: its vertical curvature, leveled.
fn terrace_lines(terraced: &Map, intensity: f32) -> Map {
    let curvature = aspect::curvature(terraced, 0.0, 1.0, 1.0, CurvatureType::Vertical);
    maths::autolevel(curvature).multiply_scalar(intensity)
}

/// What [`contours`] returns.
pub struct ContourMaps {
    pub lines: Map,
    pub terraces: Map,
    pub subterraces: Map,
}

/// `Cartographer.Contours`: contour lines every 1/`count` of the height
/// (brighter) and `subdivision` times as often (dimmer).
pub fn contours(map: &Map, count: i32, subdivision: i32, accuracy: f32, line_intensity: f32, sub_intensity: f32, thickness: i32) -> ContourMaps {
    let smoothing = 0.0 + (1.0 - accuracy) * (0.75 - 0.0);
    let smoothed = fast_blur::blur_relative(map, smoothing);
    let subterraces = profiles::terrace(&smoothed, (count * subdivision).max(0) as usize, 1.0, 1.0, 1.0, 0, false);
    let sub_lines = terrace_lines(&subterraces, sub_intensity);
    let terraces = profiles::terrace(&subterraces, count.max(0) as usize, 1.0, 1.0, 1.0, 0, false);
    let main_lines = terrace_lines(&terraces, line_intensity);
    let combined = combiner::max(&main_lines, &sub_lines, 1.0);
    let size = smoothed.width();
    let dilated = morphology::dilate_cross(combined.data(), size, thickness as f32);
    ContourMaps {
        lines: Map::from_data(maths::square_resolution(dilated.len()), dilated),
        terraces,
        subterraces,
    }
}

/// `Cartographer.Outline`: the edges of a mask.
pub fn outline(map: &Map, intensity: f32, thickness: i32) -> Map {
    let lines = terrace_lines(map, intensity);
    if thickness <= 1 {
        return lines;
    }
    let dilated = morphology::dilate_cross(lines.data(), map.width(), thickness as f32);
    Map::from_data(maths::square_resolution(dilated.len()), dilated)
}

/// `Cartographer.Grid`: a 1024 grid cell with ten sub-lines, resized and
/// tiled `subdivision` times, with a border line.
pub fn grid(resolution: Resolution, subdivision: i32, line_intensity: f32, sub_intensity: f32, thickness: i32) -> Map {
    let cell = 1024usize;
    let main_spacing = cell;
    let sub_spacing = main_spacing / 10;
    let mut lines = vec![0.0f32; cell * cell];
    for row in 0..cell {
        for column in 0..cell {
            let on_main = row % main_spacing == 0 || column % main_spacing == 0;
            let on_sub = row % sub_spacing == 0 || column % sub_spacing == 0;
            // [i, j] is indexed as (x = i, y = j): the row loop runs over x.
            let index = column * cell + row;
            if on_main {
                lines[index] += line_intensity;
            } else if on_sub {
                lines[index] += sub_intensity;
            }
        }
    }
    let cell_map = Map::from_data(Resolution::square(cell), lines);
    let resized = transformer::resize_with(&cell_map, resolution, ResizeMethod::HighQuality);
    let mut output = mosaic::repeat(&resized, subdivision, 1.0);
    let width = output.width();
    let data = output.data_mut();
    for position in 0..width {
        data[position] += line_intensity;
        data[(width - 1) * width + position] += line_intensity;
    }
    for position in 0..width {
        data[position * width] += line_intensity;
        data[position * width + width - 1] += line_intensity;
    }
    if thickness > 1 {
        let dilated = morphology::dilate_cross(output.data(), resolution.x, thickness as f32);
        output = Map::from_data(maths::square_resolution(dilated.len()), dilated);
    }
    output
}

/// A palette colour: three floats, or a `#rrggbb` string parsed as
/// `Color(string)` does (each byte over 255).
#[derive(Clone, Copy)]
enum PaletteColour {
    Rgb(f32, f32, f32),
    Hex(&'static str),
}

impl PaletteColour {
    fn rgb(self) -> [f32; 3] {
        match self {
            PaletteColour::Rgb(red, green, blue) => [red, green, blue],
            PaletteColour::Hex(text) => {
                let byte = |start: usize| u8::from_str_radix(&text[start..start + 2], 16).unwrap_or(0) as i32 as f32 / 255.0;
                [byte(1), byte(3), byte(5)]
            }
        }
    }
}

use PaletteColour::{Hex, Rgb};

const LAND_CLASSIC: &[PaletteColour] = &[
    Rgb(0.6745, 0.8157, 0.6471),
    Rgb(0.5804, 0.749, 0.5451),
    Rgb(0.6588, 0.7765, 0.5608),
    Rgb(0.7412, 0.8, 0.5882),
    Rgb(0.8196, 0.8431, 0.6706),
    Rgb(0.8824, 0.8941, 0.7098),
    Rgb(0.9373, 0.9216, 0.7529),
    Rgb(0.9098, 0.8824, 0.7137),
    Rgb(0.8706, 0.8392, 0.6392),
    Rgb(0.8275, 0.7922, 0.6157),
    Rgb(0.7922, 0.7255, 0.5098),
    Rgb(0.7333, 0.6902, 0.5451),
    Rgb(0.6745, 0.651, 0.5804),
    Rgb(0.6275, 0.6235, 0.6078),
    Rgb(0.698, 0.6941, 0.6902),
    Rgb(0.8078, 0.8078, 0.8078),
    Rgb(0.9216, 0.9216, 0.9216),
];
const LAND_MINIMAL: &[PaletteColour] = &[
    Rgb(0.637_596_9, 0.617_206_6, 0.274_677_34),
    Rgb(0.701_101_9, 0.479_320_2, 0.351_532_64),
    Rgb(198.0 / 255.0, 154.0 / 255.0, 126.0 / 255.0),
    Rgb(0.814_846_6, 0.686_685_3, 0.456_411_03),
    Rgb(0.708_375_8, 0.665_387_33, 0.637_596_9),
    Rgb(0.879_622_4, 0.879_622_4, 0.879_622_4),
];
const LAND_GREEN_RED: &[PaletteColour] = &[Hex("#587f56"), Hex("#dd8e72"), Hex("#e1926b"), Hex("#a98471"), Hex("#f2d9c5")];
const LAND_GREEN_FERTILE: &[PaletteColour] = &[Hex("#203629"), Hex("#576d60"), Hex("#cbbd8d"), Hex("#8e997b")];
const LAND_RED_BLUE: &[PaletteColour] = &[Hex("#d8ac7f"), Hex("#e09786"), Hex("#d78e88"), Hex("#c0b7a8"), Hex("#588e9a"), Hex("#d0dfe7")];
const LAND_RED_YELLOW: &[PaletteColour] = &[Hex("#d49e43"), Hex("#c69c52"), Hex("#bd9a74"), Hex("#945163"), Hex("#5a6b80")];
const LAND_GREEN_COOL: &[PaletteColour] = &[Hex("#657033"), Hex("#9ba374"), Hex("#859690"), Hex("#88939f"), Hex("#c1ccd0")];
const LAND_GREEN_SWAMP: &[PaletteColour] = &[Hex("#6b6c64"), Hex("#72756a"), Hex("#98918b"), Hex("#dec798")];
const WATER_CLASSIC: &[PaletteColour] = &[Rgb(0.7843, 0.9216, 1.0), Rgb(0.3725, 0.6549, 0.7333), Rgb(0.2941, 0.549, 0.6235)];
const WATER_BLACK_SEA: &[PaletteColour] = &[Hex("#afb3d8"), Hex("#8b8eb9"), Hex("#606289"), Hex("#4b4b66"), Hex("#323348")];
const WATER_CARIBBEAN: &[PaletteColour] = &[Hex("#c7f1f7"), Hex("#71b8d1"), Hex("#34a0c4"), Hex("#1878a6"), Hex("#053d6e")];

/// `Cartographer.LandPalettes`, in `CartographyLandPalette` order.
const LAND_PALETTES: [&[PaletteColour]; 8] = [
    LAND_CLASSIC,
    LAND_MINIMAL,
    LAND_GREEN_RED,
    LAND_GREEN_FERTILE,
    LAND_RED_BLUE,
    LAND_RED_YELLOW,
    LAND_GREEN_COOL,
    LAND_GREEN_SWAMP,
];

/// `Cartographer.WaterPalettes`, in `CartographyWaterPalette` order.
const WATER_PALETTES: [&[PaletteColour]; 3] = [WATER_CLASSIC, WATER_BLACK_SEA, WATER_CARIBBEAN];

fn palette_colours(palette: &[PaletteColour]) -> Vec<[f32; 3]> {
    palette.iter().map(|colour| colour.rgb()).collect()
}

/// A land palette's colours by `CartographyLandPalette` index.
pub fn land_palette(index: usize) -> Vec<[f32; 3]> {
    palette_colours(LAND_PALETTES[index.min(LAND_PALETTES.len() - 1)])
}

/// A water palette's colours by `CartographyWaterPalette` index.
pub fn water_palette(index: usize) -> Vec<[f32; 3]> {
    palette_colours(WATER_PALETTES[index.min(WATER_PALETTES.len() - 1)])
}
