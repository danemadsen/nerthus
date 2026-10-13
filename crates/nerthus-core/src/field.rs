//! Square raster fields: heightfields (1 channel) and colour maps (3 channels).
//!
//! Every map is stored as a flat `float[]` of `size * size * channels`
//! values, row-major with interleaved channels. Heights are normalised to
//! [0, 1] (values may overshoot while processing).

use rayon::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub size: usize,
    pub channels: usize,
    pub data: Vec<f32>,
}

impl Field {
    pub fn new(size: usize, channels: usize) -> Self {
        Self {
            size,
            channels,
            data: vec![0.0; size * size * channels],
        }
    }

    pub fn filled(size: usize, channels: usize, value: f32) -> Self {
        Self {
            size,
            channels,
            data: vec![value; size * size * channels],
        }
    }

    /// Wraps a single-channel buffer. Panics when the length is not a square.
    pub fn from_height(data: Vec<f32>) -> Self {
        let size = square_side(data.len());
        Self { size, channels: 1, data }
    }

    /// Wraps an interleaved RGB buffer.
    pub fn from_rgb(data: Vec<f32>) -> Self {
        let size = square_side(data.len() / 3);
        Self { size, channels: 3, data }
    }

    pub fn from_data(size: usize, channels: usize, data: Vec<f32>) -> Self {
        assert_eq!(data.len(), size * size * channels, "field buffer has the wrong length");
        Self { size, channels, data }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn pixel_count(&self) -> usize {
        self.size * self.size
    }

    pub fn is_color(&self) -> bool {
        self.channels >= 3
    }

    #[inline]
    pub fn index(&self, x: usize, y: usize) -> usize {
        y * self.size + x
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> f32 {
        self.data[y * self.size + x]
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, value: f32) {
        let index = y * self.size + x;
        self.data[index] = value;
    }

    /// Clamped integer lookup (edges extend).
    #[inline]
    pub fn get_clamped(&self, x: isize, y: isize) -> f32 {
        let max = self.size as isize - 1;
        let cx = x.clamp(0, max) as usize;
        let cy = y.clamp(0, max) as usize;
        self.data[cy * self.size + cx]
    }

    /// Bilinear sample in pixel coordinates with clamped edges.
    pub fn sample_bilinear(&self, x: f32, y: f32) -> f32 {
        let max = (self.size - 1) as f32;
        let px = x.clamp(0.0, max);
        let py = y.clamp(0.0, max);
        let x0 = px.floor() as usize;
        let y0 = py.floor() as usize;
        let x1 = (x0 + 1).min(self.size - 1);
        let y1 = (y0 + 1).min(self.size - 1);
        let tx = px - x0 as f32;
        let ty = py - y0 as f32;
        let top = self.get(x0, y0) + (self.get(x1, y0) - self.get(x0, y0)) * tx;
        let bottom = self.get(x0, y1) + (self.get(x1, y1) - self.get(x0, y1)) * tx;
        top + (bottom - top) * ty
    }

    pub fn min_max(&self) -> (f32, f32) {
        min_max(&self.data)
    }

    /// Expands a heightfield to grey RGB; alpha channels
    /// (grey + alpha or RGBA fields) are dropped.
    pub fn to_rgb(&self) -> Field {
        if self.channels == 3 {
            return self.clone();
        }
        let channels = self.channels;
        let mut data = Vec::with_capacity(self.pixel_count() * 3);
        for pixel in self.data.chunks(channels) {
            if channels < 3 {
                data.extend_from_slice(&[pixel[0], pixel[0], pixel[0]]);
            } else {
                data.extend_from_slice(&pixel[..3]);
            }
        }
        Field::from_data(self.size, 3, data)
    }

    /// Luminance-free channel average for colour fields, identity for
    /// heightfields (a grey + alpha field keeps its grey channel).
    pub fn to_height(&self) -> Field {
        if self.channels == 1 {
            return self.clone();
        }
        let channels = self.channels;
        let average = move |pixel: &[f32]| {
            if channels < 3 {
                return pixel[0];
            }
            (pixel[0] + pixel[1] + pixel[2]) / 3.0
        };
        let data: Vec<f32> = self.data.par_chunks(channels).map(average).collect();
        Field::from_data(self.size, 1, data)
    }

    pub fn map_in_place(&mut self, function: impl Fn(f32) -> f32 + Sync) {
        self.data.par_iter_mut().for_each(|value| *value = function(*value));
    }
}

pub fn square_side(len: usize) -> usize {
    let side = (len as f64).sqrt() as usize;
    assert_eq!(side * side, len, "buffer length {len} is not a square");
    side
}

pub fn min_max(values: &[f32]) -> (f32, f32) {
    let reduce = |a: (f32, f32), b: (f32, f32)| (a.0.min(b.0), a.1.max(b.1));
    values
        .par_iter()
        .fold(|| (f32::INFINITY, f32::NEG_INFINITY), |acc, &value| reduce(acc, (value, value)))
        .reduce(|| (f32::INFINITY, f32::NEG_INFINITY), reduce)
}
