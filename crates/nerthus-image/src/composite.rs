//! Compositing (`composite.c`). Images with 2 or 4 channels carry alpha in
//! the last channel; everything else is opaque.

use rayon::prelude::*;

use crate::image::{pixel_intensity, Image, Interpolate};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CompositeOp {
    Over,
    Atop,
    Multiply,
    Screen,
    Plus,
    Minus,
    Difference,
    Lighten,
    Darken,
    /// `Blend` with optional `compose:args`.
    Blend(Option<BlendArgs>),
    CopyAlpha,
    Copy,
    /// Porter-Duff `In` (Magick.NET's default for `Composite(image, x, y)`).
    /// Pixels outside the source keep their colour; only the overlaid
    /// region is written, which matches IM for opaque canvases.
    In,
    /// The canvas scaled by the source's intensity.
    Bumpmap,
    /// Multiply or screen depending on the canvas value.
    Overlay,
    /// `Sc + Dc - 1` (Photoshop's linear burn).
    LinearBurn,
}

/// A Blend composite's `compose:args` ("source[xcanvas]" percentages); the
/// canvas share defaults to `1 - source` like IM.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlendArgs {
    pub source: f64,
    pub canvas: Option<f64>,
}

fn has_alpha(image: &Image) -> bool {
    image.channels == 2 || image.channels == 4
}

fn color_channels(image: &Image) -> usize {
    if has_alpha(image) { image.channels - 1 } else { image.channels }
}

/// Composites `source` onto `canvas` with its top-left at (`x`, `y`).
pub fn composite(canvas: &mut Image, source: &Image, x: isize, y: isize, op: CompositeOp) {
    if op == CompositeOp::CopyAlpha {
        copy_alpha(canvas, source, x, y);
        return;
    }
    let canvas_width = canvas.width as isize;
    let canvas_height = canvas.height as isize;
    let canvas_channels = canvas.channels;
    let canvas_colors = color_channels(canvas);
    let canvas_has_alpha = has_alpha(canvas);
    let source_colors = color_channels(source);
    let source_has_alpha = has_alpha(source);
    let (source_dissolve, canvas_dissolve) = match op {
        CompositeOp::Blend(Some(arguments)) => {
            let source = arguments.source / 100.0;
            let canvas = match arguments.canvas {
                Some(canvas) => canvas / 100.0,
                None => 1.0 - source,
            };
            (source, canvas)
        }
        CompositeOp::Blend(None) => (1.0, 1.0),
        _ => (1.0, 1.0),
    };

    let rows: Vec<isize> = (0..source.height as isize).filter(|row| y + row >= 0 && y + row < canvas_height).collect();
    let row_len = canvas.width * canvas_channels;
    let canvas_data = &mut canvas.data;
    let row_slices: Vec<(isize, &mut [f32])> = canvas_data
        .chunks_mut(row_len)
        .enumerate()
        .filter_map(|(index, row)| {
            let source_row = index as isize - y;
            if rows.binary_search(&source_row).is_ok() { Some((source_row, row)) } else { None }
        })
        .collect();

    row_slices.into_par_iter().for_each(|(source_row, canvas_row)| {
        for source_column in 0..source.width as isize {
            let canvas_column = x + source_column;
            if canvas_column < 0 || canvas_column >= canvas_width {
                continue;
            }
            let source_index = (source_row as usize * source.width + source_column as usize) * source.channels;
            let canvas_index = canvas_column as usize * canvas_channels;
            let sa = if source_has_alpha { source.data[source_index + source.channels - 1] as f64 } else { 1.0 };
            let da = if canvas_has_alpha { canvas_row[canvas_index + canvas_channels - 1] as f64 } else { 1.0 };
            for channel in 0..canvas_colors {
                let source_channel = channel.min(source_colors - 1);
                let sc = source.data[source_index + source_channel] as f64;
                let dc = canvas_row[canvas_index + channel] as f64;
                let value = if op == CompositeOp::Bumpmap {
                    bumpmap_channel(source, source_index, sa, dc)
                } else {
                    blend_channel(op, sc, sa, dc, da, source_dissolve, canvas_dissolve)
                };
                canvas_row[canvas_index + channel] = value.clamp(0.0, 1.0) as f32;
            }
            if canvas_has_alpha {
                let alpha = blend_alpha(op, sa, da, source_dissolve, canvas_dissolve);
                canvas_row[canvas_index + canvas_channels - 1] = alpha.clamp(0.0, 1.0) as f32;
            }
        }
    });
}

/// IM `BumpmapCompositeOp`: `QuantumScale * intensity(source) * Dc`, or the
/// canvas unchanged under fully transparent source pixels.
fn bumpmap_channel(source: &Image, source_index: usize, sa: f64, dc: f64) -> f64 {
    if (crate::image::QUANTUM_RANGE * sa).abs() < 1.0e-12 {
        return dc;
    }
    let pixel = &source.data[source_index..source_index + source.channels];
    let intensity = if source.channels < 3 { pixel[0] as f64 } else { pixel_intensity(pixel[0], pixel[1], pixel[2]) };
    intensity * dc
}

fn blend_channel(op: CompositeOp, sc: f64, sa: f64, dc: f64, da: f64, source_dissolve: f64, canvas_dissolve: f64) -> f64 {
    let sca = sc * sa;
    let dca = dc * da;
    match op {
        CompositeOp::Copy => sc,
        CompositeOp::In => sca * da,
        CompositeOp::Over => {
            let alpha = sa + da - sa * da;
            if alpha <= 0.0 {
                return 0.0;
            }
            (sca + dca * (1.0 - sa)) / alpha
        }
        CompositeOp::Atop => {
            if da <= 0.0 {
                return dc;
            }
            (sca * da + dca * (1.0 - sa)) / da
        }
        CompositeOp::Multiply => {
            let alpha = sa + da - sa * da;
            if alpha <= 0.0 {
                return 0.0;
            }
            (sca * dca + sca * (1.0 - da) + dca * (1.0 - sa)) / alpha
        }
        CompositeOp::Screen => sca + dca - sca * dca,
        CompositeOp::Overlay => {
            let alpha = (sa + da - sa * da).clamp(0.0, 1.0);
            let gamma = crate::image::perceptible_reciprocal(alpha);
            let value = if 2.0 * dca < da {
                2.0 * dca * sca + dca * (1.0 - sa) + sca * (1.0 - da)
            } else {
                da * sa - 2.0 * (sa - sca) * (da - dca) + dca * (1.0 - sa) + sca * (1.0 - da)
            };
            gamma * value
        }
        CompositeOp::LinearBurn => {
            let alpha = (sa + da - sa * da).clamp(0.0, 1.0);
            let gamma = crate::image::perceptible_reciprocal(alpha);
            gamma * (sca + dca - sa * da)
        }
        CompositeOp::Plus => sca + dca,
        CompositeOp::Minus => dca - sca,
        CompositeOp::Difference => (sca - dca).abs(),
        CompositeOp::Lighten => sca.max(dca),
        CompositeOp::Darken => sca.min(dca),
        CompositeOp::Blend(_) => {
            // gamma = 1 / RoundToUnity(sd * Sa + cd * Da)
            let alpha = (source_dissolve * sa + canvas_dissolve * da).clamp(0.0, 1.0);
            let gamma = crate::image::perceptible_reciprocal(alpha);
            gamma * (source_dissolve * sa * sc + canvas_dissolve * da * dc)
        }
        CompositeOp::CopyAlpha | CompositeOp::Bumpmap => dc,
    }
}

fn blend_alpha(op: CompositeOp, sa: f64, da: f64, source_dissolve: f64, canvas_dissolve: f64) -> f64 {
    match op {
        CompositeOp::Copy => sa,
        CompositeOp::In => sa * da,
        CompositeOp::Atop => da,
        CompositeOp::Blend(_) => (source_dissolve * sa + canvas_dissolve * da).clamp(0.0, 1.0),
        _ => sa + da - sa * da,
    }
}

fn copy_alpha(canvas: &mut Image, source: &Image, x: isize, y: isize) {
    if !has_alpha(canvas) {
        add_alpha(canvas, 1.0);
    }
    let channels = canvas.channels;
    for row in 0..source.height as isize {
        let canvas_y = y + row;
        if canvas_y < 0 || canvas_y >= canvas.height as isize {
            continue;
        }
        for column in 0..source.width as isize {
            let canvas_x = x + column;
            if canvas_x < 0 || canvas_x >= canvas.width as isize {
                continue;
            }
            let alpha = if has_alpha(source) {
                source.get(column as usize, row as usize, source.channels - 1)
            } else {
                source.intensity(column as usize, row as usize) as f32
            };
            let index = (canvas_y as usize * canvas.width + canvas_x as usize) * channels + channels - 1;
            canvas.data[index] = alpha;
        }
    }
}

/// Adds an alpha channel with a constant value.
pub fn add_alpha(image: &mut Image, alpha: f32) {
    if has_alpha(image) {
        return;
    }
    let channels = image.channels;
    let mut data = Vec::with_capacity(image.pixel_count() * (channels + 1));
    for pixel in image.data.chunks(channels) {
        data.extend_from_slice(pixel);
        data.push(alpha);
    }
    image.data = data;
    image.channels = channels + 1;
}

/// Drops the alpha channel (IM `Alpha(Off)`).
pub fn remove_alpha(image: &mut Image) {
    if !has_alpha(image) {
        return;
    }
    let channels = image.channels;
    let colors = channels - 1;
    let mut data = Vec::with_capacity(image.pixel_count() * colors);
    for pixel in image.data.chunks(channels) {
        data.extend_from_slice(&pixel[..colors]);
    }
    image.data = data;
    image.channels = colors;
}

/// IM `-compose displace` with `compose:args = "{horizontal}x{vertical}"`.
///
/// The displacement map's red (x) and green (y) channels are centred at
/// `(QuantumRange + 1) / 2`; for grey maps both axes use the same value.
pub fn displace(canvas: &Image, map: &Image, horizontal: f64, vertical: f64) -> Image {
    let channels = canvas.channels;
    let width = canvas.width;
    let mut result = canvas.clone();
    let half_range = (65535.0 + 1.0) / 2.0;
    let map_width = map.width;
    let map_height = map.height;
    result
        .data
        .par_chunks_mut(width * channels)
        .enumerate()
        .for_each(|(y, out_row)| {
            if y >= map_height {
                return;
            }
            for x in 0..width.min(map_width) {
                let (red, green) = if map.channels >= 3 {
                    (map.get(x, y, 0) as f64, map.get(x, y, 1) as f64)
                } else {
                    let gray = map.get(x, y, 0) as f64;
                    (gray, gray)
                };
                let offset_x = horizontal * (red * 65535.0 - half_range) / half_range + x as f64;
                let offset_y = vertical * (green * 65535.0 - half_range) / half_range + y as f64;
                // The displaced canvas is composited back with ClampPixel.
                for channel in 0..channels {
                    let value = canvas.interpolate(offset_x, offset_y, channel, Interpolate::Bilinear);
                    out_row[x * channels + channel] = value.clamp(0.0, 1.0);
                }
            }
        });
    result
}

/// Combines grey channels into an RGB image (IM `Combine`).
pub fn combine_channels(red: &Image, green: &Image, blue: &Image) -> Image {
    let width = red.width;
    let height = red.height;
    let mut data = Vec::with_capacity(width * height * 3);
    for index in 0..width * height {
        data.push(red.data[index * red.channels]);
        data.push(green.data[index * green.channels]);
        data.push(blue.data[index * blue.channels]);
    }
    Image::from_data(width, height, 3, data)
}

/// Extracts one channel as a grey image (IM `Separate`).
pub fn separate_channel(image: &Image, channel: usize) -> Image {
    let channels = image.channels;
    let data: Vec<f32> = image.data.chunks(channels).map(|pixel| pixel[channel.min(channels - 1)]).collect();
    Image::from_data(image.width, image.height, 1, data)
}

/// Grey conversion (IM `ColorType = Grayscale`).
pub fn to_grayscale(image: &Image) -> Image {
    if image.channels == 1 {
        return image.clone();
    }
    let channels = image.channels;
    let data: Vec<f32> = image.data.chunks(channels).map(|pixel| pixel_intensity(pixel[0], pixel[1], pixel[2]) as f32).collect();
    let mut result = Image::from_data(image.width, image.height, 1, data);
    result.virtual_pixel = image.virtual_pixel;
    result
}
