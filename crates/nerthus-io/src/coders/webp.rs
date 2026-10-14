//! WebP (`coders/webp.c`): libwebp's RGBA output (`MODE_RGBA`, fancy
//! upsampling for lossy images, as `WebPInitDecoderConfig` leaves it), each
//! sample scaled to a quantum; an alpha channel only when the file has one
//! (`features.has_alpha`). sRGB with IM's default gamma.

use std::io::BufReader;
use std::path::Path;

use anyhow::{bail, Context, Result};
use image_webp::{UpsamplingMethod, WebPDecoder};

use crate::pixel_cache::{CacheColorspace, DEFAULT_GAMMA, PixelCache, PixelChannel, char_quantum};

pub fn read(path: &Path) -> Result<PixelCache> {
    let file = std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut decoder = WebPDecoder::new(BufReader::new(file)).with_context(|| format!("cannot read {}", path.display()))?;
    if decoder.is_animated() {
        bail!("animated WebP files are not supported");
    }
    decoder.set_lossy_upsampling(UpsamplingMethod::Bilinear);
    let (width, height) = decoder.dimensions();
    let has_alpha = decoder.has_alpha();
    let size = decoder.output_buffer_size().context("the WebP image is too large")?;
    let mut samples = vec![0u8; size];
    decoder.read_image(&mut samples)?;
    let mut channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    if has_alpha {
        channels.push(PixelChannel::Alpha);
    }
    let data = samples.iter().map(|&sample| char_quantum(sample)).collect();
    Ok(PixelCache::new(width as usize, height as usize, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA))
}
