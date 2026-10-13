//! The mask helpers (`Masking`).

use nerthus_engine::{Map, Resolution};
use rayon::prelude::*;

use crate::fast_blur;
use crate::gradients;
use crate::maths;
use crate::mathv;

/// `Masking.Mask`: `a · (1 − mask) + b · mask`.
pub fn mask(a: &Map, b: &Map, mask: &Map) -> Map {
    let inverse = mask.subtract_from(1.0);
    let kept = a.multiply(&inverse);
    let replaced = b.multiply(mask);
    kept.add(&replaced)
}

/// `Masking.Vignette`: fades the map out towards a soft circle `size` of
/// the map across.
pub fn vignette(map: &Map, size: f32, softness: f32) -> Map {
    let width = map.width();
    let gradient = gradients::radial_relative(width, 1.0 - size, 1.0);
    let softened = fast_blur::blur_relative(&gradient, 6.0 * softness);
    map.multiply(&softened)
}

/// `Masking.PreciseEdge`: zeroes a border `pixels` wide around a square map.
pub fn precise_edge(map: &Map, pixels: i32) -> Map {
    let size = map.width();
    let margin = pixels;
    let margin_opposite = size as i32 - margin;
    let mut frame = vec![0.0f32; size * size];
    frame.par_chunks_mut(size).enumerate().for_each(|(row, values)| {
        let y = row as i32;
        for (column, value) in values.iter_mut().enumerate() {
            let x = column as i32;
            let inside = y >= margin && x >= margin && y <= margin_opposite && x <= margin_opposite;
            *value = if inside { 1.0 } else { 0.0 };
        }
    });
    let frame_map = Map::from_data(map.resolution(), frame);
    let leveled = maths::autolevel(frame_map);
    map.multiply(&leveled)
}

/// How far into the fade of a square edge mask a position is: 1 inside,
/// easing to 0 at the edge (one axis).
fn edge_fade(position: f32, center: f32, transition_inverse: f32) -> f32 {
    let distance = (center - position).abs() / center;
    let fade = mathv::min(1.0, transition_inverse * (1.0 - distance));
    mathv::smoothstep7(fade)
}

/// `Masking.GenerateSquareEdgeMask`: 1 in the middle, fading to 0 at the
/// borders over the outer `transition` of each half.
pub fn square_edge_mask(resolution: Resolution, transition: f32) -> Map {
    let center_x = resolution.x as f32 * 0.5;
    let center_y = resolution.y as f32 * 0.5;
    let transition_inverse = 1.0 / transition;
    Map::generate(resolution, |x, y| {
        let fade_x = edge_fade(x, center_x, transition_inverse);
        let fade_y = edge_fade(y, center_y, transition_inverse);
        fade_x * fade_y
    })
}

/// `Masking.MaskBySquareEdge`: the map faded out towards its borders.
pub fn mask_by_square_edge(map: &Map, transition: f32) -> Map {
    let resolution = map.resolution();
    let width = resolution.x;
    let center_x = resolution.x as f32 * 0.5;
    let center_y = resolution.y as f32 * 0.5;
    let transition_inverse = 1.0 / transition;
    let mut channels = Vec::with_capacity(map.channel_count());
    for channel in 0..map.channel_count() {
        let source = map.channel(channel);
        let mut faded = vec![0.0f32; source.len()];
        faded.par_chunks_mut(width).zip(source.par_chunks(width)).enumerate().for_each(|(row, (values, originals))| {
            let fade_y = edge_fade(row as f32, center_y, transition_inverse);
            for (column, (value, &original)) in values.iter_mut().zip(originals).enumerate() {
                let fade_x = edge_fade(column as f32, center_x, transition_inverse);
                *value = original * fade_x * fade_y;
            }
        });
        channels.push(faded);
    }
    Map::from_channels(resolution, channels)
}
