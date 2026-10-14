//! SVG (`coders/svg.c` through librsvg and cairo, as Magick.NET has it).
//! The SVG density is set only after reading, so IM reads at its default
//! resolution, 72 dpi: a drawing in pixels comes out at 72/96 of its size
//! (scaled by cairo), one in absolute units at its size at 72 dpi. Each
//! pixel is composited over the white background (`CompositePixelOver`),
//! so the image is opaque; it keeps an alpha channel (`BlendPixelTrait`).
//! Nerthus draws with resvg, which antialiases edges a little differently
//! from cairo.

use std::path::Path;

use anyhow::{bail, Context, Result};
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{Options, Tree};

use crate::pixel_cache::{CacheColorspace, DEFAULT_GAMMA, PixelCache, PixelChannel};

/// `DefaultResolution` and `DefaultSVGDensity`.
const DEFAULT_RESOLUTION: f32 = 72.0;
const SVG_DENSITY: f32 = 96.0;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    let parse = |dpi: f32| -> Result<Tree> {
        let options = Options { dpi, resources_dir: path.parent().map(Path::to_path_buf), ..Options::default() };
        Tree::from_data(&bytes, &options).map_err(|error| anyhow::anyhow!("cannot read {}: {error}", path.display()))
    };
    let tree = parse(DEFAULT_RESOLUTION)?;
    // `rsvg_handle_get_dimensions` gives whole pixels.
    let size = tree.size();
    let (base_width, base_height) = (size.width().round(), size.height().round());
    // The density applies unless the size follows the dpi (`apply_density`).
    let scaled = parse(DEFAULT_RESOLUTION * 256.0)?.size();
    let follows_dpi = (scaled.width().round() - base_width).abs() >= f32::EPSILON || (scaled.height().round() - base_height).abs() >= f32::EPSILON;
    let (width, height, scale) = if follows_dpi {
        (base_width as usize, base_height as usize, 1.0)
    } else {
        let factor = DEFAULT_RESOLUTION / SVG_DENSITY;
        ((DEFAULT_RESOLUTION as f64 * base_width as f64 / SVG_DENSITY as f64) as usize, (DEFAULT_RESOLUTION as f64 * base_height as f64 / SVG_DENSITY as f64) as usize, factor)
    };
    if width == 0 || height == 0 {
        bail!("the SVG has no size");
    }
    let mut pixmap = Pixmap::new(width as u32, height as u32).context("the SVG is too large")?;
    resvg::render(&tree, Transform::from_scale(scale, scale), &mut pixmap.as_mut());

    // Premultiplied samples over opaque white: `Sc·Sa + 65535·(1 − Sa)`
    // with `Sc` the demultiplied colour (IM divides the 8-bit premultiplied
    // value, scaled to a quantum, by the alpha).
    let mut data = Vec::with_capacity(width * height * 4);
    for pixel in pixmap.data().as_chunks::<4>().0 {
        let alpha = 257.0 * pixel[3] as f64 / 65535.0;
        let over = |premultiplied: u8| -> f32 {
            let source = 257.0 * premultiplied as f64;
            (source + 65535.0 * (1.0 - alpha)) as f32
        };
        data.extend([over(pixel[0]), over(pixel[1]), over(pixel[2]), 65535.0]);
    }
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Alpha];
    Ok(PixelCache::new(width, height, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA))
}
