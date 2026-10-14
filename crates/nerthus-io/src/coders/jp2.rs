//! JPEG 2000 (`coders/jp2.c`): OpenJPEG's components, each sample
//! `QuantumRange / (2^precision − 1) · (sample + offset)` (the offset
//! centres signed samples), in double, stored as a float quantum
//! (`ClampToQuantum` does not clamp in HDRI). One component is grey (its
//! alpha flag makes it alpha too), two are grey and alpha, three or more
//! are RGB (YCbCr held as they are for sYCC), a fourth alpha when flagged.

use std::path::Path;

#[cfg(not(target_arch = "wasm32"))]
use anyhow::Context;
use anyhow::{bail, Result};

use crate::pixel_cache::PixelCache;
#[cfg(not(target_arch = "wasm32"))]
use crate::pixel_cache::{CacheColorspace, PixelChannel, DEFAULT_GAMMA};

/// OpenJPEG's Rust port calls the C library's allocator, which the web
/// build (wasm32-unknown-unknown) does not have.
#[cfg(target_arch = "wasm32")]
pub fn read(path: &Path) -> Result<PixelCache> {
    bail!("cannot decode {}: JPEG 2000 images cannot be read in the web version", path.display())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    let image = jpeg2k::Image::from_bytes(&bytes).map_err(|error| anyhow::anyhow!("cannot decode {}: {error}", path.display()))?;
    let components = image.components();
    let Some(first) = components.first() else {
        bail!("the JPEG 2000 image has no components");
    };
    let width = first.width() as usize;
    let height = first.height() as usize;
    let irregular = components.iter().any(|component| component.precision() != first.precision() || component.is_signed() != first.is_signed() || component.width() == 0 || component.height() == 0);
    if irregular || first.precision() > 64 {
        bail!("Irregular channel geometry not supported");
    }

    let count = components.len();
    let gray_space = count == 1 || matches!(image.color_space(), jpeg2k::ColorSpace::Gray);
    let alpha = match count {
        1 => first.is_alpha(),
        2 => components[1].is_alpha(),
        _ => components.get(3).is_some_and(|component| component.is_alpha()),
    };
    let channels: Vec<PixelChannel> = match count {
        1 if alpha => vec![PixelChannel::Gray, PixelChannel::Alpha],
        1 => vec![PixelChannel::Gray],
        2 if alpha => vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Alpha],
        2 => vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue],
        _ if alpha => vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Alpha],
        _ => vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue],
    };

    // `comps_info`: each component's scale, offset and subsampling.
    let scales: Vec<f64> = components.iter().map(|component| 65535.0 / ((1u128 << component.precision()) - 1) as f64).collect();
    let offsets: Vec<f64> = components.iter().map(|component| if component.is_signed() { (1u128 << (component.precision() - 1)) as f64 } else { 0.0 }).collect();
    let steps: Vec<(usize, usize)> = components.iter().map(|component| (width.div_ceil(component.width() as usize).max(1), height.div_ceil(component.height() as usize).max(1))).collect();

    let mut data = Vec::with_capacity(width * height * channels.len());
    for y in 0..height {
        for x in 0..width {
            let sample = |index: usize| -> f32 {
                let component = &components[index];
                let (step_x, step_y) = steps[index];
                let position = (y / step_y) * component.width() as usize + x / step_x;
                let value = component.data().get(position).copied().unwrap_or(0) as f64;
                (scales[index] * (value + offsets[index])) as f32
            };
            match count {
                1 => {
                    let gray = sample(0);
                    data.push(gray);
                    if alpha {
                        data.push(gray);
                    }
                }
                // The first sample goes to red, green and blue; the second
                // is alpha when flagged, else it replaces green.
                2 if alpha => {
                    let gray = sample(0);
                    data.extend([gray, gray, gray, sample(1)]);
                }
                2 => {
                    let gray = sample(0);
                    data.extend([gray, sample(1), gray]);
                }
                _ => {
                    data.extend([sample(0), sample(1), sample(2)]);
                    if alpha {
                        data.push(sample(3));
                    }
                }
            }
        }
    }
    let colorspace = if gray_space { CacheColorspace::Gray } else { CacheColorspace::SRgb };
    Ok(PixelCache::new(width, height, channels, data, colorspace, DEFAULT_GAMMA))
}
