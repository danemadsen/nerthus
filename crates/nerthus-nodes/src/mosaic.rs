//! The tiling helpers (`Mosaic`).

use nerthus_engine::{Map, Resolution};
use rayon::prelude::*;

use crate::maths::{dotnet_max, dotnet_min};
use crate::transformer::{self, AnchorPosition, BlendMode, ResizeMethod};

/// `Mosaic.Smoothstep`: the quintic `6x⁵ − 15x⁴ + 10x³`.
fn smoothstep(x: f32) -> f32 {
    let squared = x * x;
    x * squared * (6.0 * squared - 15.0 * x + 10.0)
}

/// `Mosaic.Combine`.
fn combine(a: f32, b: f32, fade: f32) -> f32 {
    a * fade + b * (1.0 - fade)
}

/// The cross-fade weight towards the map's own copy at a distance from the
/// centre line (1 in the middle, easing to 0 at the edges).
fn seam_weight(position: usize, half: usize, edge: f32) -> f32 {
    let distance = (half as i32 - position as i32).abs() as f32;
    let mut weight = 1.0 - distance / half as f32;
    weight = dotnet_min(weight * edge, 1.0);
    smoothstep(weight)
}

/// `Mosaic.Seamless`: makes the heightmap (and optional colour texture)
/// tile by cross-fading each half with a copy shifted by `shift`, first
/// across, then down. Returns the heightmap and, given a texture, the
/// matching colour map.
pub fn seamless(heightmap: &Map, texture: Option<&Map>, edge: f32, shift_x: f32, shift_y: f32) -> (Map, Option<Map>) {
    let size = heightmap.width();
    let mut edge = dotnet_max(edge, 0.01);
    let span = (size - 1) as f32;
    let shift_columns = (0.5 + span * ((shift_x - 0.5) * (1.0 - edge) + 0.5)) as i32 as usize;
    let shift_rows = (0.5 + span * ((shift_y - 0.5) * (1.0 - edge) + 0.5)) as i32 as usize;
    edge = 1.0 / edge;
    let half = size >> 1;

    // The heightmap and the texture's three channels get the same blends.
    let mut planes: Vec<&[f32]> = vec![heightmap.data()];
    if let Some(texture) = texture {
        for channel in 0..3 {
            planes.push(texture.rgb_channel(channel));
        }
    }

    let mut results = Vec::with_capacity(planes.len());
    for plane in planes {
        let mut across = vec![0.0f32; size * size];
        across.par_chunks_mut(size).enumerate().for_each(|(row, values)| {
            for (column, value) in values.iter_mut().enumerate() {
                let weight = seam_weight(column, half, edge);
                let shifted = (column + shift_columns) % size;
                *value = combine(plane[row * size + column], plane[row * size + shifted], weight);
            }
        });
        let mut down = vec![0.0f32; size * size];
        down.par_chunks_mut(size).enumerate().for_each(|(row, values)| {
            let weight = seam_weight(row, half, edge);
            let shifted = (row + shift_rows) % size;
            for (column, value) in values.iter_mut().enumerate() {
                *value = combine(across[row * size + column], across[shifted * size + column], weight);
            }
        });
        results.push(down);
    }

    let resolution = heightmap.resolution();
    let mut results = results.into_iter();
    let height = Map::from_data(resolution, results.next().expect("the heightmap was blended"));
    let colour_channels: Vec<Vec<f32>> = results.collect();
    let colour = (!colour_channels.is_empty()).then(|| Map::from_channels(resolution, colour_channels));
    (height, colour)
}

/// `Mosaic.Repeat`: the map shrunk to one of `tiles` × `tiles` cells and
/// pasted into each; single-channel tiles are scaled by `height`.
pub fn repeat(map: &Map, tiles: i32, height: f32) -> Map {
    let size = map.width() as f32;
    let cell = size / tiles as f32;
    let cell_resolution = Resolution::square(cell.ceil() as usize);
    let mut source = transformer::resize_with(map, cell_resolution, ResizeMethod::HighQuality);
    if height != 1.0 && map.channel_count() == 1 {
        source = source.multiply_scalar(height);
    }
    let mut result = Map::new(map.resolution(), map.channel_count());
    for row in 0..tiles {
        let top = row as f32 * cell;
        for column in 0..tiles {
            let left = column as f32 * cell;
            let position = (left / map.width() as f32, top / map.height() as f32);
            transformer::insert(&source, &mut result, position, BlendMode::Blend, None, AnchorPosition::TopLeft);
        }
    }
    result
}
