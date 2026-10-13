//! The bridge between maps and ImageMagick images for the nodes that run
//! through Magick.NET. Images hold the
//! Q16 HDRI quanta the bridge writes (`value × 65535` in single precision), for
//! `nerthus_image::quanta` and the scale-free operations of
//! `nerthus_image`.

use nerthus_engine::{Map, Resolution};
use nerthus_image::quanta::QUANTUM_RANGE;
use nerthus_image::{Colorspace, Image};

/// A map as a Magick image of its quanta. Grey maps become GRAY
/// images; colour maps become sRGB (with alpha when they have four
/// channels).
pub fn to_image(map: &Map) -> Image {
    let resolution = map.resolution();
    let channels = map.channel_count();
    let quanta: Vec<f32> = map.to_interleaved().iter().map(|&value| value * QUANTUM_RANGE).collect();
    let mut image = Image::from_data(resolution.x, resolution.y, channels, quanta);
    image.colorspace = if channels == 1 { Colorspace::Gray } else { Colorspace::SRgb };
    image
}

/// An image back as a map (grey images as one channel, colour
/// ones without alpha), its quanta times `1 / 65535` in single precision.
pub fn to_map(image: &Image) -> Map {
    let resolution = Resolution::new(image.width, image.height);
    let unit = 1.0f32 / QUANTUM_RANGE;
    if image.channels < 3 {
        let gray: Vec<f32> = image.data.iter().step_by(image.channels).map(|&quantum| quantum * unit).collect();
        return Map::from_data(resolution, gray);
    }
    let channels = image.channels.min(3);
    let mut planes = vec![Vec::with_capacity(image.width * image.height); channels];
    for pixel in image.data.chunks(image.channels) {
        for (channel, plane) in planes.iter_mut().enumerate() {
            plane.push(pixel[channel] * unit);
        }
    }
    Map::from_channels(resolution, planes)
}
