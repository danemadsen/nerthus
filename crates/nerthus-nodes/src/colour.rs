//! The colour helpers (`Colorize` and `ColorCore`).

use nerthus_engine::Map;
use rayon::prelude::*;

use crate::maths::{dotnet_max, dotnet_min};
use crate::spline::Polyline;

/// The colour at `t` along evenly spaced `stops`
/// (`Vector3.Lerp` between the two around it).
fn gradient_colour(t: f32, stops: &[[f32; 3]]) -> [f32; 3] {
    let count = stops.len();
    let position = t * (count - 1) as f32;
    let lower = (position as i32).min(count as i32 - 1).max(0) as usize;
    let upper = (lower + 1).min(count - 1);
    let amount = position - lower as f32;
    let a = stops[lower];
    let b = stops[upper];
    [
        a[0] * (1.0 - amount) + b[0] * amount,
        a[1] * (1.0 - amount) + b[1] * amount,
        a[2] * (1.0 - amount) + b[2] * amount,
    ]
}

/// Maps a single-channel map onto an RGB gradient.
pub fn colorize(map: &Map, stops: &[[f32; 3]]) -> Map {
    let colours: Vec<[f32; 3]> = map.data().par_iter().map(|&value| gradient_colour(value, stops)).collect();
    let mut channels: Vec<Vec<f32>> = (0..3).map(|_| Vec::with_capacity(colours.len())).collect();
    for colour in colours {
        for (channel, value) in channels.iter_mut().zip(colour) {
            channel.push(value);
        }
    }
    Map::from_channels(map.resolution(), channels)
}

/// `ColorCore.HueToRGB`.
fn hue_to_rgb(p: f32, q: f32, mut t: f32) -> f32 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 0.5 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}

/// Clamps to [0, 1] with .NET's `MathF.Max`/`Min`.
fn saturate(value: f32) -> f32 {
    dotnet_max(0.0, dotnet_min(1.0, value))
}

/// `ColorCore.ModifyHSL`: shifts hue and scales saturation and lightness.
pub fn modify_hsl(rgb: [f32; 3], hue_shift: f32, saturation_factor: f32, lightness_factor: f32) -> [f32; 3] {
    let [r, g, b] = rgb;
    let maximum = dotnet_max(r, dotnet_max(g, b));
    let minimum = dotnet_min(r, dotnet_min(g, b));
    let chroma = maximum - minimum;
    let mut hue = 0.0f32;
    let mut saturation = 0.0f32;
    let mut lightness = (maximum + minimum) / 2.0;
    if chroma > 0.0 {
        let denominator = if lightness > 0.5 { 2.0 - maximum - minimum } else { maximum + minimum };
        saturation = chroma / denominator;
        hue = if maximum == r {
            (g - b) / chroma + if g < b { 6.0 } else { 0.0 }
        } else if maximum != g {
            (r - g) / chroma + 4.0
        } else {
            (b - r) / chroma + 2.0
        };
        hue /= 6.0;
    }
    hue = (hue + hue_shift) % 1.0;
    if hue < 0.0 {
        hue += 1.0;
    }
    saturation = saturate(saturation * saturation_factor);
    lightness = saturate(lightness * lightness_factor);
    let q = if lightness < 0.5 { lightness * (1.0 + saturation) } else { lightness + saturation - lightness * saturation };
    let p = 2.0 * lightness - q;
    [hue_to_rgb(p, q, hue + 1.0 / 3.0), hue_to_rgb(p, q, hue), hue_to_rgb(p, q, hue - 1.0 / 3.0)]
}

/// `ColorCore.HSL`: [`modify_hsl`] on every pixel of the first three
/// channels; any further channels are left as they are. The map must have
/// at least three channels.
pub fn hsl(map: &Map, hue: f32, saturation: f32, lightness: f32) -> Map {
    let count = map.width() * map.height();
    let pixels: Vec<[f32; 3]> = (0..count)
        .into_par_iter()
        .map(|index| {
            let rgb = [map.channel(0)[index], map.channel(1)[index], map.channel(2)[index]];
            modify_hsl(rgb, hue, 1.0 + saturation, 1.0 + lightness)
        })
        .collect();
    let mut adjusted = map.clone();
    for channel in 0..3 {
        let plane = adjusted.channel_mut(channel);
        for (value, pixel) in plane.iter_mut().zip(&pixels) {
            *value = pixel[channel];
        }
    }
    adjusted
}

/// `ColorCore.Clut(heightmap, GradientData)`: each colour channel through
/// its piecewise-linear curve (positions in W, values in X, Y and Z).
pub fn clut(heights: &Map, stops: &[[f32; 4]]) -> Map {
    let channels: Vec<Vec<f32>> = (0..3)
        .map(|channel| {
            let points: Vec<[f32; 2]> = stops.iter().map(|stop| [stop[3], stop[channel]]).collect();
            let curve = Polyline::from_unsorted(&points);
            heights.data().par_iter().map(|&height| curve.interpolate(height)).collect()
        })
        .collect();
    Map::from_channels(heights.resolution(), channels)
}

/// `ColorCore.Clut(heightmap, Color[])`: the colours spread evenly, the
/// i-th of n at i/n.
pub fn clut_colours(heights: &Map, colours: &[[f32; 3]]) -> Map {
    let count = colours.len() as i32 as f32;
    let stops: Vec<[f32; 4]> = colours
        .iter()
        .enumerate()
        .map(|(index, colour)| [colour[0], colour[1], colour[2], index as i32 as f32 / count])
        .collect();
    clut(heights, &stops)
}
