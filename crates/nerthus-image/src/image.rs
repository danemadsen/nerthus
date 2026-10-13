//! Float raster mirroring a Magick.NET `MagickImage<float>` (Q16 HDRI).
//!
//! Values are stored normalised so that 1.0 equals ImageMagick's
//! `QuantumRange`. HDRI semantics apply: nothing is clamped unless an
//! operation explicitly clamps.

use nerthus_core::Field;
use rayon::prelude::*;

use crate::colorspace::Colorspace;
use crate::resize::Filter;

/// ImageMagick `QuantumRange` for Q16 builds; used where IM maths depends on it.
pub const QUANTUM_RANGE: f64 = 65535.0;
pub const QUANTUM_SCALE: f64 = 1.0 / QUANTUM_RANGE;

/// How pixels outside the image are synthesised.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VirtualPixel {
    /// IM's default (`Undefined` behaves like `Edge`).
    #[default]
    Edge,
    Mirror,
    Tile,
    Black,
    White,
    Transparent,
}

/// The origin of a virtual pixel: a real pixel or a constant colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VirtualSource {
    Pixel(usize, usize),
    Constant(f32),
}

/// IM `PixelInterpolateMethod`. `Bilinear` is also what IM's `Undefined`
/// resolves to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Interpolate {
    #[default]
    Bilinear,
    Nearest,
    Integer,
    Catrom,
    Mesh,
    Spline,
    Average,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub channels: usize,
    pub data: Vec<f32>,
    pub virtual_pixel: VirtualPixel,
    /// IM `image->interpolate`, used by operations that fall back to
    /// interpolated lookups (for example EWA resampling).
    pub interpolate: Interpolate,
    /// IM `image->colorspace`.
    pub colorspace: Colorspace,
    /// IM `image->filter` (Magick.NET `FilterType`); `None` is IM's
    /// undefined filter, which lets each operation pick its default.
    pub filter: Option<Filter>,
}

impl Image {
    pub fn new(width: usize, height: usize, channels: usize) -> Self {
        Self {
            width,
            height,
            channels,
            data: vec![0.0; width * height * channels],
            virtual_pixel: VirtualPixel::Edge,
            interpolate: Interpolate::Bilinear,
            colorspace: Colorspace::for_channels(channels),
            filter: None,
        }
    }

    pub fn filled(width: usize, height: usize, channels: usize, value: f32) -> Self {
        let mut image = Self::new(width, height, channels);
        image.data.fill(value);
        image
    }

    pub fn from_data(width: usize, height: usize, channels: usize, data: Vec<f32>) -> Self {
        assert_eq!(data.len(), width * height * channels, "image buffer has the wrong length");
        Self {
            width,
            height,
            channels,
            data,
            virtual_pixel: VirtualPixel::Edge,
            interpolate: Interpolate::Bilinear,
            colorspace: Colorspace::for_channels(channels),
            filter: None,
        }
    }

    /// Square grey image from a heightfield buffer.
    pub fn from_gray(data: &[f32]) -> Self {
        let size = nerthus_core::field::square_side(data.len());
        Self::from_data(size, size, 1, data.to_vec())
    }

    /// Square RGB image from an interleaved buffer.
    pub fn from_rgb(data: &[f32]) -> Self {
        let size = nerthus_core::field::square_side(data.len() / 3);
        Self::from_data(size, size, 3, data.to_vec())
    }

    pub fn from_field(field: &Field) -> Self {
        Self::from_data(field.size, field.size, field.channels, field.data.clone())
    }

    pub fn into_field(self) -> Field {
        assert_eq!(self.width, self.height, "fields must be square");
        Field::from_data(self.width, self.channels, self.data)
    }

    /// Converts to a single grey channel (forcing `ColorType.Grayscale`,
    /// IM's sRGB to GRAY transform).
    pub fn to_gray_data(&self) -> Vec<f32> {
        if self.channels < 3 {
            return self.data.iter().step_by(self.channels).copied().collect();
        }
        let channels = self.channels;
        self.data
            .par_chunks(channels)
            .map(|pixel| pixel_intensity(pixel[0], pixel[1], pixel[2]) as f32)
            .collect()
    }

    /// A blank image with this image's channels and IM attributes (virtual
    /// pixels, interpolation, colourspace), like IM's `CloneImage(w, h)`.
    pub fn new_like(&self, width: usize, height: usize) -> Image {
        let mut image = Image::new(width, height, self.channels);
        image.virtual_pixel = self.virtual_pixel;
        image.interpolate = self.interpolate;
        image.colorspace = self.colorspace;
        image.filter = self.filter;
        image
    }

    /// Expands a grey image to three equal channels, like an sRGB IM image
    /// holding grey values.
    pub fn gray_to_rgb(&self) -> Image {
        let mut data = Vec::with_capacity(self.pixel_count() * 3);
        for value in self.data.iter().step_by(self.channels) {
            data.extend_from_slice(&[*value, *value, *value]);
        }
        let mut result = Image::from_data(self.width, self.height, 3, data);
        result.virtual_pixel = self.virtual_pixel;
        result.interpolate = self.interpolate;
        result.colorspace = Colorspace::SRgb;
        result
    }

    pub fn with_virtual_pixel(mut self, virtual_pixel: VirtualPixel) -> Self {
        self.virtual_pixel = virtual_pixel;
        self
    }

    pub fn pixel_count(&self) -> usize {
        self.width * self.height
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize, channel: usize) -> f32 {
        self.data[(y * self.width + x) * self.channels + channel]
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, channel: usize, value: f32) {
        let index = (y * self.width + x) * self.channels + channel;
        self.data[index] = value;
    }

    /// Where the pixel at (`x`, `y`) comes from under the virtual-pixel method.
    #[inline]
    pub fn virtual_source(&self, x: isize, y: isize) -> VirtualSource {
        let width = self.width as isize;
        let height = self.height as isize;
        if x >= 0 && y >= 0 && x < width && y < height {
            return VirtualSource::Pixel(x as usize, y as usize);
        }
        match self.virtual_pixel {
            VirtualPixel::Edge => {
                let cx = x.clamp(0, width - 1) as usize;
                let cy = y.clamp(0, height - 1) as usize;
                VirtualSource::Pixel(cx, cy)
            }
            VirtualPixel::Mirror => {
                let cx = mirror_index(x, width) as usize;
                let cy = mirror_index(y, height) as usize;
                VirtualSource::Pixel(cx, cy)
            }
            VirtualPixel::Tile => {
                let cx = x.rem_euclid(width) as usize;
                let cy = y.rem_euclid(height) as usize;
                VirtualSource::Pixel(cx, cy)
            }
            VirtualPixel::Black | VirtualPixel::Transparent => VirtualSource::Constant(0.0),
            VirtualPixel::White => VirtualSource::Constant(1.0),
        }
    }

    /// Pixel lookup that honours the virtual-pixel method.
    #[inline]
    pub fn virtual_get(&self, x: isize, y: isize, channel: usize) -> f32 {
        match self.virtual_source(x, y) {
            VirtualSource::Pixel(column, row) => self.get(column, row, channel),
            VirtualSource::Constant(value) => value,
        }
    }

    /// IM `InterpolatePixelInfo` for one channel of an opaque image. Integer
    /// coordinates are pixel centres; all maths runs in double precision.
    pub fn interpolate(&self, x: f64, y: f64, channel: usize, method: Interpolate) -> f32 {
        let value = match method {
            Interpolate::Nearest => {
                let xi = (x + 0.5).floor() as isize;
                let yi = (y + 0.5).floor() as isize;
                self.virtual_get(xi, yi, channel) as f64
            }
            Interpolate::Integer => {
                let xi = x.floor() as isize;
                let yi = y.floor() as isize;
                self.virtual_get(xi, yi, channel) as f64
            }
            Interpolate::Average => self.average(x, y, channel),
            Interpolate::Bilinear => self.bilinear_f64(x, y, channel),
            Interpolate::Mesh => self.mesh(x, y, channel),
            Interpolate::Catrom => self.cubic(x, y, channel, catrom_weights),
            Interpolate::Spline => self.cubic(x, y, channel, spline_weights),
        };
        value as f32
    }

    pub fn bilinear(&self, x: f64, y: f64, channel: usize) -> f32 {
        self.bilinear_f64(x, y, channel) as f32
    }

    /// The four pixels of the 2x2 cell whose top-left corner is `floor(x, y)`.
    fn cell(&self, x: f64, y: f64, channel: usize) -> (f64, f64, [f64; 4]) {
        let x0 = x.floor();
        let y0 = y.floor();
        let xi = x0 as isize;
        let yi = y0 as isize;
        let pixels = [
            self.virtual_get(xi, yi, channel) as f64,
            self.virtual_get(xi + 1, yi, channel) as f64,
            self.virtual_get(xi, yi + 1, channel) as f64,
            self.virtual_get(xi + 1, yi + 1, channel) as f64,
        ];
        (x - x0, y - y0, pixels)
    }

    /// `AverageInterpolatePixel`: mean of the 2x2 cell.
    fn average(&self, x: f64, y: f64, channel: usize) -> f64 {
        let (_, _, pixels) = self.cell(x, y, channel);
        let mut sum = 0.0;
        for pixel in pixels {
            sum += pixel;
        }
        sum * 0.25
    }

    /// `BilinearInterpolatePixel`, including IM's normalisation by the summed
    /// (unit) alpha weights.
    fn bilinear_f64(&self, x: f64, y: f64, channel: usize) -> f64 {
        let (delta_x, delta_y, pixels) = self.cell(x, y, channel);
        let epsilon_x = 1.0 - delta_x;
        let epsilon_y = 1.0 - delta_y;
        let gamma = epsilon_y * (epsilon_x + delta_x) + delta_y * (epsilon_x + delta_x);
        let gamma = perceptible_reciprocal(gamma);
        let top = epsilon_x * pixels[0] + delta_x * pixels[1];
        let bottom = epsilon_x * pixels[2] + delta_x * pixels[3];
        gamma * (epsilon_y * top + delta_y * bottom)
    }

    /// `MeshInterpolatePixel`: splits the cell into two triangles along the
    /// diagonal with the smaller luma difference.
    fn mesh(&self, x: f64, y: f64, channel: usize) -> f64 {
        let (mut delta_x, mut delta_y, pixels) = self.cell(x, y, channel);
        let xi = x.floor() as isize;
        let yi = y.floor() as isize;
        let luma = [
            self.virtual_luma(xi, yi),
            self.virtual_luma(xi + 1, yi),
            self.virtual_luma(xi, yi + 1),
            self.virtual_luma(xi + 1, yi + 1),
        ];
        let luminance_x = luma[0] as f64 - luma[3] as f64;
        let luminance_y = luma[1] as f64 - luma[2] as f64;
        let (p, px, py) = if luminance_x.abs() < luminance_y.abs() {
            // Diagonal 0-3 NW-SE.
            if delta_x <= delta_y {
                delta_y = 1.0 - delta_y;
                (pixels[2], pixels[3], pixels[0])
            } else {
                delta_x = 1.0 - delta_x;
                (pixels[1], pixels[0], pixels[3])
            }
        } else if delta_x <= 1.0 - delta_y {
            // Diagonal 1-2 NE-SW, top-left triangle.
            (pixels[0], pixels[1], pixels[2])
        } else {
            delta_x = 1.0 - delta_x;
            delta_y = 1.0 - delta_y;
            (pixels[3], pixels[2], pixels[1])
        };
        let gamma = mesh_interpolate(delta_x, delta_y, 1.0, 1.0, 1.0);
        let gamma = perceptible_reciprocal(gamma);
        gamma * mesh_interpolate(delta_x, delta_y, p, px, py)
    }

    /// 4x4 separable cubic (`CatromInterpolatePixel`, `SplineInterpolatePixel`).
    fn cubic(&self, x: f64, y: f64, channel: usize, weights: fn(f64) -> [f64; 4]) -> f64 {
        let x0 = x.floor();
        let y0 = y.floor();
        let cx = weights(x - x0);
        let cy = weights(y - y0);
        let xi = x0 as isize;
        let yi = y0 as isize;
        let mut sum = 0.0;
        for (j, weight_y) in cy.iter().enumerate() {
            let row_y = yi - 1 + j as isize;
            let mut row = 0.0;
            for (i, weight_x) in cx.iter().enumerate() {
                let value = self.virtual_get(xi - 1 + i as isize, row_y, channel) as f64;
                row += weight_x * value;
            }
            sum += weight_y * row;
        }
        sum
    }

    /// IM `GetPixelLuma` (single precision, Rec.709 weights) through virtual pixels.
    fn virtual_luma(&self, x: isize, y: isize) -> f32 {
        if self.channels < 3 {
            let value = self.virtual_get(x, y, 0);
            return pixel_luma(value, value, value);
        }
        let red = self.virtual_get(x, y, 0);
        let green = self.virtual_get(x, y, 1);
        let blue = self.virtual_get(x, y, 2);
        pixel_luma(red, green, blue)
    }

    /// IM `GetPixelIntensity` (the grey value itself for grey images).
    pub fn intensity(&self, x: usize, y: usize) -> f64 {
        if self.channels < 3 {
            return self.get(x, y, 0) as f64;
        }
        pixel_intensity(self.get(x, y, 0), self.get(x, y, 1), self.get(x, y, 2))
    }

    pub fn map_in_place(&mut self, function: impl Fn(f32) -> f32 + Sync) {
        self.data.par_iter_mut().for_each(|value| *value = function(*value));
    }

    pub fn negate(&mut self) {
        self.map_in_place(|value| 1.0 - value);
    }

    pub fn clamp(&mut self) {
        self.map_in_place(|value| value.clamp(0.0, 1.0));
    }

    pub fn flip(&mut self) {
        let row_len = self.width * self.channels;
        let height = self.height;
        for y in 0..height / 2 {
            let (top, bottom) = self.data.split_at_mut((height - 1 - y) * row_len);
            top[y * row_len..(y + 1) * row_len].swap_with_slice(&mut bottom[..row_len]);
        }
    }

    pub fn flop(&mut self) {
        let width = self.width;
        let channels = self.channels;
        self.data.par_chunks_mut(width * channels).for_each(|row| {
            for x in 0..width / 2 {
                for channel in 0..channels {
                    row.swap(x * channels + channel, (width - 1 - x) * channels + channel);
                }
            }
        });
    }

    /// Copies a region (IM `Crop`), clipped to the image.
    pub fn crop(&self, x: isize, y: isize, width: usize, height: usize) -> Image {
        let mut result = self.new_like(width, height);
        for row in 0..height {
            for column in 0..width {
                for channel in 0..self.channels {
                    let value = self.virtual_get(x + column as isize, y + row as isize, channel);
                    result.set(column, row, channel, value);
                }
            }
        }
        result
    }

    /// Region copy that reads through virtual pixels (IM `Clone(x, y, w, h)` on a canvas
    /// with a virtual-pixel method).
    pub fn extract(&self, x: isize, y: isize, width: usize, height: usize) -> Image {
        self.crop(x, y, width, height)
    }

    pub fn min_max(&self) -> (f32, f32) {
        nerthus_core::field::min_max(&self.data)
    }
}

/// IM `GetPixelIntensity` with its default Rec.709 luma weights, evaluated in
/// double precision. The sRGB to GRAY colourspace transform uses the same sum.
#[inline]
pub fn pixel_intensity(red: f32, green: f32, blue: f32) -> f64 {
    0.212656 * red as f64 + 0.715158 * green as f64 + 0.072186 * blue as f64
}

/// IM `GetPixelLuma`: the Rec.709 weights in single precision.
#[inline]
pub fn pixel_luma(red: f32, green: f32, blue: f32) -> f32 {
    0.212_656f32 * red + 0.715_158f32 * green + 0.072_186f32 * blue
}

#[inline]
fn mirror_index(index: isize, size: isize) -> isize {
    // IM mirror: reflects across edges with the edge pixel duplicated.
    let period = size * 2;
    let mut value = index.rem_euclid(period);
    if value >= size {
        value = period - 1 - value;
    }
    value
}

/// IM `PerceptibleReciprocal`.
#[inline]
pub fn perceptible_reciprocal(x: f64) -> f64 {
    const MAGICK_EPSILON: f64 = 1.0e-12;
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    if sign * x >= MAGICK_EPSILON {
        return 1.0 / x;
    }
    sign / MAGICK_EPSILON
}

/// IM `MeshInterpolate`: `dx * x + dy * y + (1 - dx - dy) * p`.
#[inline]
fn mesh_interpolate(delta_x: f64, delta_y: f64, p: f64, x: f64, y: f64) -> f64 {
    delta_x * x + delta_y * y + (1.0 - delta_x - delta_y) * p
}

/// IM `CatromWeights` (Robidoux's refactoring of the Catmull-Rom weights).
fn catrom_weights(x: f64) -> [f64; 4] {
    let alpha = 1.0 - x;
    let beta = -0.5 * x * alpha;
    let weight_0 = alpha * beta;
    let weight_3 = x * beta;
    let gamma = weight_3 - weight_0;
    let weight_1 = alpha - weight_0 + gamma;
    let weight_2 = x - weight_3 - gamma;
    [weight_0, weight_1, weight_2, weight_3]
}

/// IM `SplineWeights` (cubic B-spline smoothing weights).
fn spline_weights(x: f64) -> [f64; 4] {
    let alpha = 1.0 - x;
    let weight_3 = (1.0 / 6.0) * x * x * x;
    let weight_0 = (1.0 / 6.0) * alpha * alpha * alpha;
    let beta = weight_3 - weight_0;
    let weight_1 = alpha - weight_0 + beta;
    let weight_2 = x - weight_3 - beta;
    [weight_0, weight_1, weight_2, weight_3]
}
