//! Geometric distortions (`distort.c`, `resample.c`, `fx.c`).
//!
//! `DistortImage` maps every destination pixel centre back into the source
//! image and samples it with IM's elliptical weighted average (EWA)
//! resampler: a cylindrical cubic filter (Robidoux unless the image's filter
//! is Catrom) through its 1024-entry weight table.

use std::fmt;
use std::sync::OnceLock;

use rayon::prelude::*;

use crate::colorspace::Colorspace;
use crate::image::{perceptible_reciprocal, Image, Interpolate, VirtualPixel, QUANTUM_RANGE, QUANTUM_SCALE};
use crate::resize::{resize_with_filter, Filter};

const WLUT_WIDTH: usize = 1024;
const ROBIDOUX_B: f64 = 0.378_215_755_093_998_7;
const ROBIDOUX_C: f64 = 0.310_892_122_453_000_7;
/// Support of both cylindrical cubic filters.
const CUBIC_SUPPORT: f64 = 2.0;
const MAGICK_EPSILON: f64 = 1.0e-12;
const MAGICK_PI: f64 = std::f64::consts::PI;
const MAGICK_2PI: f64 = std::f64::consts::TAU;

/// An argument error raised by `DistortImage` (Magick.NET throws these).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DistortError(pub String);

impl fmt::Display for DistortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for DistortError {}

/// IM `DegreesToRadians`.
fn degrees_to_radians(degrees: f64) -> f64 {
    MAGICK_PI * degrees / 180.0
}

/// IM `MagickRound` from `distort.c`: halves round up.
fn magick_round(x: f64) -> f64 {
    if x - x.floor() < x.ceil() - x {
        return x.floor();
    }
    x.ceil()
}

fn cubic_bc(x: f64, b: f64, c: f64) -> f64 {
    let x = x.abs();
    let p0 = (6.0 - 2.0 * b) / 6.0;
    let p2 = (-18.0 + 12.0 * b + 6.0 * c) / 6.0;
    let p3 = (12.0 - 9.0 * b - 6.0 * c) / 6.0;
    let q0 = (8.0 * b + 24.0 * c) / 6.0;
    let q1 = (-12.0 * b - 48.0 * c) / 6.0;
    let q2 = (6.0 * b + 30.0 * c) / 6.0;
    let q3 = (-b - 6.0 * c) / 6.0;
    if x < 1.0 {
        return p0 + x * (x * (p2 + x * p3));
    }
    if x < 2.0 {
        return q0 + x * (q1 + x * (q2 + x * q3));
    }
    0.0
}

/// `SetResampleFilter`'s weight table for a cubic BC filter, indexed by
/// the squared radius.
fn cubic_lut(b: f64, c: f64) -> Vec<f64> {
    let r_scale = CUBIC_SUPPORT * (1.0 / WLUT_WIDTH as f64).sqrt();
    let mut lut = Vec::with_capacity(WLUT_WIDTH);
    for q in 0..WLUT_WIDTH {
        let radius = (q as f64).sqrt() * r_scale;
        lut.push(cubic_bc(radius, b, c));
    }
    lut
}

/// The weight table for the image's filter: Catmull-Rom for Catrom,
/// otherwise Robidoux (IM's default for an undefined filter). The nodes
/// only distort images whose filter is undefined, Catrom or Point (which
/// turns EWA off), so other filters are not modelled.
fn resample_lut(filter: Option<Filter>) -> &'static [f64] {
    static ROBIDOUX: OnceLock<Vec<f64>> = OnceLock::new();
    static CATROM: OnceLock<Vec<f64>> = OnceLock::new();
    match filter {
        Some(Filter::Catrom) => CATROM.get_or_init(|| cubic_lut(0.0, 0.5)),
        _ => ROBIDOUX.get_or_init(|| cubic_lut(ROBIDOUX_B, ROBIDOUX_C)),
    }
}

/// Major and minor ellipse axes clamped to at least one pixel
/// (`ClampUpAxes`, Robidoux and Racette's SVD).
struct ClampedAxes {
    major_mag: f64,
    minor_mag: f64,
    major_unit_x: f64,
    major_unit_y: f64,
    minor_unit_x: f64,
    minor_unit_y: f64,
}

fn clamp_up_axes(dux: f64, dvx: f64, duy: f64, dvy: f64) -> ClampedAxes {
    let a = dux;
    let b = duy;
    let c = dvx;
    let d = dvy;
    let aa = a * a;
    let bb = b * b;
    let cc = c * c;
    let dd = d * d;
    let n11 = aa + bb;
    let n12 = a * c + b * d;
    let n21 = n12;
    let n22 = cc + dd;
    let det = a * d - b * c;
    let twice_det = det + det;
    let frobenius_squared = n11 + n22;
    let discriminant = (frobenius_squared + twice_det) * (frobenius_squared - twice_det);
    let sqrt_discriminant = if discriminant > 0.0 { discriminant } else { 0.0 }.sqrt();
    let s1s1 = 0.5 * (frobenius_squared + sqrt_discriminant);
    let s2s2 = 0.5 * (frobenius_squared - sqrt_discriminant);
    let s1s1_minus_n11 = s1s1 - n11;
    let s1s1_minus_n22 = s1s1 - n22;
    let s1s1_minus_n11_squared = s1s1_minus_n11 * s1s1_minus_n11;
    let s1s1_minus_n22_squared = s1s1_minus_n22 * s1s1_minus_n22;
    let use_first = s1s1_minus_n11_squared >= s1s1_minus_n22_squared;
    let temp_u11 = if use_first { n12 } else { s1s1_minus_n22 };
    let temp_u21 = if use_first { s1s1_minus_n11 } else { n21 };
    let norm = (temp_u11 * temp_u11 + temp_u21 * temp_u21).sqrt();
    let u11 = if norm > 0.0 { temp_u11 / norm } else { 1.0 };
    let u21 = if norm > 0.0 { temp_u21 / norm } else { 0.0 };
    ClampedAxes {
        major_mag: if s1s1 <= 1.0 { 1.0 } else { s1s1.sqrt() },
        minor_mag: if s2s2 <= 1.0 { 1.0 } else { s2s2.sqrt() },
        major_unit_x: u11,
        major_unit_y: u21,
        minor_unit_x: -u21,
        minor_unit_y: u11,
    }
}

/// One thread's IM `ResampleFilter`: EWA sampling of `image` with the
/// ellipse set by the last `scale` call.
struct ResampleFilter<'a> {
    image: &'a Image,
    /// A Point filter makes IM interpolate instead of filtering.
    do_interpolate: bool,
    lut: &'static [f64],
    /// Whole-image average used when the ellipse is impossibly large.
    average: &'a OnceLock<Vec<f32>>,
    image_area: f64,
    limit_reached: bool,
    a: f64,
    b: f64,
    c: f64,
    u_limit: f64,
    v_limit: f64,
    u_width: f64,
    slope: f64,
}

impl<'a> ResampleFilter<'a> {
    /// `AcquireResampleFilter`: the image's filter at unit scale.
    fn new(image: &'a Image, average: &'a OnceLock<Vec<f32>>) -> Self {
        let mut filter = Self {
            image,
            do_interpolate: image.filter == Some(Filter::Point),
            lut: resample_lut(image.filter),
            average,
            image_area: (image.width * image.height) as f64,
            limit_reached: false,
            a: 0.0,
            b: 0.0,
            c: 0.0,
            u_limit: 0.0,
            v_limit: 0.0,
            u_width: 0.0,
            slope: 0.0,
        };
        filter.scale(1.0, 0.0, 0.0, 1.0);
        filter
    }

    /// `ScaleResampleFilter` with the destination-to-source partial
    /// derivatives `du/dx`, `du/dy`, `dv/dx`, `dv/dy`.
    fn scale(&mut self, dux: f64, duy: f64, dvx: f64, dvy: f64) {
        self.limit_reached = false;
        let axes = clamp_up_axes(dux, dvx, duy, dvy);
        let major_x = axes.major_unit_x * axes.major_mag;
        let major_y = axes.major_unit_y * axes.major_mag;
        let minor_x = axes.minor_unit_x * axes.minor_mag;
        let minor_y = axes.minor_unit_y * axes.minor_mag;
        let a = major_y * major_y + minor_y * minor_y;
        let b = -2.0 * (major_x * major_y + minor_x * minor_y);
        let c = major_x * major_x + minor_x * minor_x;
        let mut f = axes.major_mag * axes.minor_mag;
        f *= f;
        if 4.0 * a * c - b * b > f64::MAX {
            self.limit_reached = true;
            return;
        }
        f *= CUBIC_SUPPORT;
        f *= CUBIC_SUPPORT;
        self.u_limit = (c * f / (a * c - 0.25 * b * b)).sqrt();
        self.v_limit = (a * f / (a * c - 0.25 * b * b)).sqrt();
        self.u_width = (f / a).sqrt();
        self.slope = -b / (2.0 * a);
        if self.u_width * self.v_limit > 4.0 * self.image_area {
            self.limit_reached = true;
            return;
        }
        let scale = WLUT_WIDTH as f64 * perceptible_reciprocal(f);
        self.a = a * scale;
        self.b = b * scale;
        self.c = c * scale;
    }

    fn interpolate_into(&self, u0: f64, v0: f64, method: Interpolate, out: &mut [f32]) {
        for (channel, value) in out.iter_mut().enumerate() {
            *value = self.image.interpolate(u0, v0, channel, method);
        }
    }

    /// Does the sampled area miss the image so that a virtual pixel colour
    /// can be returned directly?
    fn misses_image(&self, u0: f64, v0: f64) -> bool {
        let last_column = self.image.width as f64 - 1.0;
        let last_row = self.image.height as f64 - 1.0;
        let left = u0 + self.u_limit < 0.0;
        let right = u0 - self.u_limit > last_column;
        let above = v0 + self.v_limit < 0.0;
        let below = v0 - self.v_limit > last_row;
        match self.image.virtual_pixel {
            VirtualPixel::Black | VirtualPixel::White | VirtualPixel::Transparent => self.limit_reached || left || right || above || below,
            VirtualPixel::Edge => (left && above) || (left && below) || (right && above) || (right && below),
            VirtualPixel::Mirror | VirtualPixel::Tile => false,
        }
    }

    /// `ResamplePixelColor` at source coordinates where integers are pixel centres.
    fn sample(&self, u0: f64, v0: f64, out: &mut [f32]) {
        if self.do_interpolate {
            self.interpolate_into(u0, v0, self.image.interpolate, out);
            return;
        }
        if self.misses_image(u0, v0) {
            self.interpolate_into(u0, v0, Interpolate::Integer, out);
            return;
        }
        if self.limit_reached {
            match self.image.virtual_pixel {
                VirtualPixel::Mirror | VirtualPixel::Tile => {
                    let average = self.average.get_or_init(|| resize_with_filter(self.image, 1, 1, Filter::Box).data);
                    out.copy_from_slice(average);
                }
                _ => self.interpolate_into(u0, v0, Interpolate::Average, out),
            }
            return;
        }
        let image = self.image;
        let channels = image.channels;
        let alpha_channel = if channels == 2 || channels == 4 { Some(channels - 1) } else { None };
        let mut sums = [0.0f64; 4];
        let mut divisor_c = 0.0f64;
        let mut divisor_m = 0.0f64;
        let mut hits = 0usize;
        let v1 = (v0 - self.v_limit).ceil() as isize;
        let v2 = (v0 + self.v_limit).floor() as isize;
        let mut u1 = u0 + (v1 as f64 - v0) * self.slope - self.u_width;
        let uw = (2.0 * self.u_width) as isize + 1;
        let ddq = 2.0 * self.a;
        for v in v1..=v2 {
            let u_start = u1.ceil() as isize;
            u1 += self.slope;
            let big_u = u_start as f64 - u0;
            let big_v = v as f64 - v0;
            let mut q = (self.a * big_u + self.b * big_v) * big_u + self.c * big_v * big_v;
            let mut dq = self.a * (2.0 * big_u + 1.0) + self.b * big_v;
            for step in 0..uw {
                if q < WLUT_WIDTH as f64 {
                    let mut weight = self.lut[q as usize];
                    let u = u_start + step;
                    match alpha_channel {
                        Some(alpha_index) => {
                            let alpha = image.virtual_get(u, v, alpha_index) as f64;
                            sums[alpha_index] += weight * alpha;
                            divisor_m += weight;
                            weight *= alpha;
                        }
                        None => divisor_m += weight,
                    }
                    for (channel, sum) in sums.iter_mut().enumerate().take(channels) {
                        if Some(channel) != alpha_channel {
                            *sum += weight * image.virtual_get(u, v, channel) as f64;
                        }
                    }
                    divisor_c += weight;
                    hits += 1;
                }
                q += dq;
                dq += ddq;
            }
        }
        if hits == 0 || divisor_m <= MAGICK_EPSILON || divisor_c <= MAGICK_EPSILON {
            self.interpolate_into(u0, v0, image.interpolate, out);
            return;
        }
        let divisor_m = 1.0 / divisor_m;
        let divisor_c = 1.0 / divisor_c;
        for (channel, value) in out.iter_mut().enumerate() {
            let divisor = if Some(channel) == alpha_channel { divisor_m } else { divisor_c };
            *value = (divisor * sums[channel]) as f32;
        }
    }
}

/// How `DistortImage` maps a destination pixel centre into the source.
enum Mapping<'a> {
    /// `u = c0 x + c1 y + c2`, `v = c3 x + c4 y + c5`.
    Affine([f64; 6]),
    /// Sanitised (Rmax, Rmin, cx, cy, from, to) plus the angle and radius
    /// to pixel ratios.
    Polar([f64; 8]),
    /// Inverse distance weighted displacement of the control points
    /// `u, v, x, y` with the weighting power halved.
    Shepards { power: f64, points: &'a [f64] },
}

/// The main loop of IM `DistortImage` without best fit or viewport, so the
/// result has the size of the source.
fn distort_image(image: &Image, mapping: &Mapping) -> Image {
    let width = image.width;
    let channels = image.channels;
    let source_columns = image.width as f64;
    let mut result = image.new_like(width, image.height);
    let average = OnceLock::new();
    let rows = result.data.par_chunks_mut(width * channels);
    rows.enumerate().for_each(|(j, out_row)| {
        let mut filter = ResampleFilter::new(image, &average);
        if let Mapping::Affine(coefficients) = mapping {
            filter.scale(coefficients[0], coefficients[1], coefficients[3], coefficients[4]);
        }
        let destination_y = j as f64 + 0.5;
        for i in 0..width {
            let destination_x = i as f64 + 0.5;
            let (source_x, source_y) = match mapping {
                Mapping::Affine(coefficients) => {
                    let x = coefficients[0] * destination_x + coefficients[1] * destination_y + coefficients[2];
                    let y = coefficients[3] * destination_x + coefficients[4] * destination_y + coefficients[5];
                    (x, y)
                }
                Mapping::Polar(coefficients) => {
                    let delta_x = destination_x - coefficients[2];
                    let delta_y = destination_y - coefficients[3];
                    let mut angle = delta_x.atan2(delta_y) - (coefficients[4] + coefficients[5]) / 2.0;
                    angle /= MAGICK_2PI;
                    angle -= magick_round(angle);
                    angle *= MAGICK_2PI;
                    let radius = delta_x.hypot(delta_y);
                    if radius > MAGICK_EPSILON {
                        filter.scale(coefficients[6] / (MAGICK_2PI * radius), 0.0, 0.0, coefficients[7]);
                    } else {
                        filter.scale(width as f64 * 2.0, 0.0, 0.0, coefficients[7]);
                    }
                    let x = angle * coefficients[6] + source_columns / 2.0;
                    let y = (radius - coefficients[1]) * coefficients[7];
                    (x, y)
                }
                Mapping::Shepards { power, points } => shepards_source(destination_x, destination_y, *power, points),
            };
            let pixel = &mut out_row[i * channels..(i + 1) * channels];
            filter.sample(source_x - 0.5, source_y - 0.5, pixel);
        }
    });
    result
}

fn shepards_source(destination_x: f64, destination_y: f64, power: f64, points: &[f64]) -> (f64, f64) {
    let mut denominator = 0.0;
    let mut source_x = 0.0;
    let mut source_y = 0.0;
    for point in points.as_chunks::<4>().0 {
        let offset_x = destination_x - point[2];
        let offset_y = destination_y - point[3];
        let mut weight = offset_x * offset_x + offset_y * offset_y;
        if power != 1.0 {
            weight = weight.powf(power);
        }
        weight = if weight < 1.0 { 1.0 } else { 1.0 / weight };
        source_x += (point[0] - point[2]) * weight;
        source_y += (point[1] - point[3]) * weight;
        denominator += weight;
    }
    source_x /= denominator;
    source_y /= denominator;
    (source_x + destination_x, source_y + destination_y)
}

/// `ScaleRotateTranslate` coefficients for the argument forms IM accepts:
/// `angle`, `scale angle`, `X,Y angle`, `X,Y scale angle`,
/// `X,Y scaleX,scaleY angle`, `X,Y scale angle NewX,NewY` and
/// `X,Y scaleX,scaleY angle NewX,NewY`.
pub fn srt_coefficients(image: &Image, arguments: &[f64]) -> Result<[f64; 6], DistortError> {
    let mut x = image.width as f64 / 2.0;
    let mut y = image.height as f64 / 2.0;
    let mut sx = 1.0;
    let mut sy = 1.0;
    let angle;
    match arguments.len() {
        0 => return Err(DistortError("ScaleRotateTranslate : 'Needs at least 1 argument'".into())),
        1 => angle = arguments[0],
        2 => {
            sx = arguments[0];
            sy = arguments[0];
            angle = arguments[1];
        }
        3..=7 => {
            x = arguments[0];
            y = arguments[1];
            match arguments.len() {
                3 => angle = arguments[2],
                4 | 6 => {
                    sx = arguments[2];
                    sy = arguments[2];
                    angle = arguments[3];
                }
                _ => {
                    sx = arguments[2];
                    sy = arguments[3];
                    angle = arguments[4];
                }
            }
        }
        _ => return Err(DistortError("ScaleRotateTranslate : 'Too Many Arguments (7 or less)'".into())),
    }
    let (nx, ny) = match arguments.len() {
        6 => (arguments[4], arguments[5]),
        7 => (arguments[5], arguments[6]),
        _ => (x, y),
    };
    if sx.abs() < MAGICK_EPSILON || sy.abs() < MAGICK_EPSILON {
        return Err(DistortError("ScaleRotateTranslate : 'Zero Scale Given'".into()));
    }
    let radians = degrees_to_radians(angle);
    let cosine = radians.cos();
    let sine = radians.sin();
    let c0 = cosine / sx;
    let c1 = sine / sx;
    let c2 = x - nx * c0 - ny * c1;
    let c3 = -sine / sy;
    let c4 = cosine / sy;
    let c5 = y - nx * c3 - ny * c4;
    Ok([c0, c1, c2, c3, c4, c5])
}

/// `Distort(DistortMethod.Affine-equivalent)` with explicit coefficients.
pub fn distort_affine(image: &Image, coefficients: [f64; 6]) -> Image {
    distort_image(image, &Mapping::Affine(coefficients))
}

/// `Distort(DistortMethod.ScaleRotateTranslate, arguments)`.
pub fn distort_srt(image: &Image, arguments: &[f64]) -> Result<Image, DistortError> {
    let coefficients = srt_coefficients(image, arguments)?;
    Ok(distort_affine(image, coefficients))
}

/// `Distort(DistortMethod.Polar, Rmax, Rmin, CenterX, CenterY, from, to)`
/// (Cartesian to polar view). An `Rmax` of 0 uses the nearest edge and -1
/// the furthest corner.
pub fn distort_polar(image: &Image, arguments: &[f64]) -> Result<Image, DistortError> {
    if arguments.len() == 3 || arguments.len() > 6 {
        return Err(DistortError("Polar : number of arguments".into()));
    }
    let columns = image.width as f64;
    let rows = image.height as f64;
    let mut coefficients = [0.0f64; 8];
    coefficients[0] = arguments.first().copied().unwrap_or(0.0);
    coefficients[1] = arguments.get(1).copied().unwrap_or(0.0);
    if arguments.len() >= 4 {
        coefficients[2] = arguments[2];
        coefficients[3] = arguments[3];
    } else {
        coefficients[2] = columns / 2.0;
        coefficients[3] = rows / 2.0;
    }
    coefficients[4] = -MAGICK_PI;
    if arguments.len() >= 5 {
        coefficients[4] = degrees_to_radians(arguments[4]);
    }
    coefficients[5] = coefficients[4];
    if arguments.len() >= 6 {
        coefficients[5] = degrees_to_radians(arguments[5]);
    }
    if (coefficients[4] - coefficients[5]).abs() < MAGICK_EPSILON {
        coefficients[5] += MAGICK_2PI;
    }
    if coefficients[0] < MAGICK_EPSILON {
        if coefficients[0].abs() < MAGICK_EPSILON {
            let mut nearest = coefficients[2].abs().min(coefficients[3].abs());
            nearest = nearest.min((coefficients[2] - columns).abs());
            nearest = nearest.min((coefficients[3] - rows).abs());
            coefficients[0] = nearest;
        }
        if (-1.0 - coefficients[0]).abs() < MAGICK_EPSILON {
            let mut rx = coefficients[2];
            let mut ry = coefficients[3];
            let mut furthest = rx * rx + ry * ry;
            ry = coefficients[3] - rows;
            furthest = furthest.max(rx * rx + ry * ry);
            rx = coefficients[2] - columns;
            furthest = furthest.max(rx * rx + ry * ry);
            ry = coefficients[3];
            furthest = furthest.max(rx * rx + ry * ry);
            coefficients[0] = furthest.sqrt();
        }
    }
    let invalid_radius = coefficients[0] < MAGICK_EPSILON || coefficients[1] < -MAGICK_EPSILON || coefficients[0] - coefficients[1] < MAGICK_EPSILON;
    if invalid_radius {
        return Err(DistortError("Polar : Invalid Radius".into()));
    }
    coefficients[6] = columns / (coefficients[5] - coefficients[4]);
    coefficients[7] = rows / (coefficients[0] - coefficients[1]);
    Ok(distort_image(image, &Mapping::Polar(coefficients)))
}

/// `Distort(DistortMethod.Shepards, u1,v1,x1,y1, ...)`. `power_artifact` is
/// the image's `shepards:power` artifact, if set.
pub fn distort_shepards(image: &Image, points: &[f64], power_artifact: Option<f64>) -> Result<Image, DistortError> {
    if !points.len().is_multiple_of(4) || points.len() < 4 {
        return Err(DistortError("Shepards : 'requires CP's (4 numbers each)'".into()));
    }
    let power = match power_artifact {
        Some(value) => value / 2.0,
        None => 1.0,
    };
    if power < MAGICK_EPSILON {
        return Err(DistortError("InvalidArgument -define shepards:power".into()));
    }
    let mapping = Mapping::Shepards { power, points };
    Ok(distort_image(image, &mapping))
}

/// IM `SwirlImage(degrees, method)`.
pub fn swirl(image: &Image, degrees: f64, method: Interpolate) -> Image {
    let center_x = image.width as f64 / 2.0;
    let center_y = image.height as f64 / 2.0;
    let radius = center_x.max(center_y);
    let mut scale_x = 1.0;
    let mut scale_y = 1.0;
    if image.width > image.height {
        scale_y = image.width as f64 / image.height as f64;
    } else if image.width < image.height {
        scale_x = image.height as f64 / image.width as f64;
    }
    let radians = degrees_to_radians(degrees);
    let channels = image.channels;
    let width = image.width;
    let mut result = image.clone();
    let rows = result.data.par_chunks_mut(width * channels);
    rows.enumerate().for_each(|(y, out_row)| {
        let delta_y = scale_y * (y as f64 - center_y);
        for x in 0..width {
            let delta_x = scale_x * (x as f64 - center_x);
            let distance = delta_x * delta_x + delta_y * delta_y;
            if distance >= radius * radius {
                continue;
            }
            let factor = 1.0 - distance.sqrt() / radius;
            let angle = radians * factor * factor;
            let sine = angle.sin();
            let cosine = angle.cos();
            let source_x = (cosine * delta_x - sine * delta_y) / scale_x + center_x;
            let source_y = (sine * delta_x + cosine * delta_y) / scale_y + center_y;
            for channel in 0..channels {
                out_row[x * channels + channel] = image.interpolate(source_x, source_y, channel, method);
            }
        }
    });
    result
}

/// IM `ImplodeImage(amount, method)`.
pub fn implode(image: &Image, amount: f64, method: Interpolate) -> Image {
    let center_x = 0.5 * image.width as f64;
    let center_y = 0.5 * image.height as f64;
    let mut radius = center_x;
    let mut scale_x = 1.0;
    let mut scale_y = 1.0;
    if image.width > image.height {
        scale_y = image.width as f64 * perceptible_reciprocal(image.height as f64);
    } else if image.width < image.height {
        scale_x = image.height as f64 * perceptible_reciprocal(image.width as f64);
        radius = center_y;
    }
    let inverse_radius = perceptible_reciprocal(radius);
    let inverse_scale_x = perceptible_reciprocal(scale_x);
    let inverse_scale_y = perceptible_reciprocal(scale_y);
    let channels = image.channels;
    let width = image.width;
    let mut result = image.clone();
    let rows = result.data.par_chunks_mut(width * channels);
    rows.enumerate().for_each(|(y, out_row)| {
        let delta_y = scale_y * (y as f64 - center_y);
        for x in 0..width {
            let delta_x = scale_x * (x as f64 - center_x);
            let distance = delta_x * delta_x + delta_y * delta_y;
            if distance >= radius * radius {
                continue;
            }
            let mut factor = 1.0;
            if distance > 0.0 {
                let angle = MAGICK_PI * distance.sqrt() * inverse_radius / 2.0;
                factor = angle.sin().powf(-amount);
            }
            let source_x = factor * delta_x * inverse_scale_x + center_x;
            let source_y = factor * delta_y * inverse_scale_y + center_y;
            for channel in 0..channels {
                out_row[x * channels + channel] = image.interpolate(source_x, source_y, channel, method);
            }
        }
    });
    result
}

/// Exact right-angle rotation (IM `RotateImage` for multiples of 90 degrees).
pub fn rotate_right_angle(image: &Image, quarter_turns: i32) -> Image {
    let turns = quarter_turns.rem_euclid(4);
    if turns == 0 {
        return image.clone();
    }
    let channels = image.channels;
    let (width, height) = if turns % 2 == 1 { (image.height, image.width) } else { (image.width, image.height) };
    let mut result = image.new_like(width, height);
    for y in 0..image.height {
        for x in 0..image.width {
            let (target_x, target_y) = match turns {
                1 => (image.height - 1 - y, x),
                2 => (image.width - 1 - x, image.height - 1 - y),
                _ => (y, image.width - 1 - x),
            };
            for channel in 0..channels {
                result.set(target_x, target_y, channel, image.get(x, y, channel));
            }
        }
    }
    result
}

/// IM `RotateImage(degrees)`: right angles are exact, other angles expand the
/// canvas (background black) like IM's shear rotation.
pub fn rotate(image: &Image, degrees: f64) -> Image {
    let normalized = degrees.rem_euclid(360.0);
    let quarter = (normalized / 90.0).round();
    if (normalized - quarter * 90.0).abs() < 1e-9 {
        return rotate_right_angle(image, quarter as i32);
    }
    let radians = normalized.to_radians();
    let cosine = radians.cos();
    let sine = radians.sin();
    let width = image.width as f64;
    let height = image.height as f64;
    let new_width = (width * cosine.abs() + height * sine.abs()).ceil() as usize;
    let new_height = (width * sine.abs() + height * cosine.abs()).ceil() as usize;
    let channels = image.channels;
    let mut result = image.new_like(new_width, new_height);
    let center_x = width / 2.0;
    let center_y = height / 2.0;
    let new_center_x = new_width as f64 / 2.0;
    let new_center_y = new_height as f64 / 2.0;
    let mut source = image.clone();
    source.virtual_pixel = crate::image::VirtualPixel::Black;
    result
        .data
        .par_chunks_mut(new_width * channels)
        .enumerate()
        .for_each(|(y, out_row)| {
            let dy = y as f64 + 0.5 - new_center_y;
            for x in 0..new_width {
                let dx = x as f64 + 0.5 - new_center_x;
                let source_x = cosine * dx + sine * dy + center_x - 0.5;
                let source_y = -sine * dx + cosine * dy + center_y - 0.5;
                for channel in 0..channels {
                    out_row[x * channels + channel] = source.bilinear(source_x, source_y, channel);
                }
            }
        });
    result
}

/// One channel of an IM `gradient:` at a stop offset (`DrawGradientImage`
/// with a pad spread, blended over the `xc:` canvas filled with the start
/// colour). Colours and the result are in quantum units.
fn gradient_channel(offset: f64, start: f64, stop: f64) -> f64 {
    let stop_offsets = [0.0, 1.0];
    let mut index = 0;
    while index < stop_offsets.len() {
        if offset < stop_offsets[index] {
            break;
        }
        index += 1;
    }
    let (color, alpha) = if offset < 0.0 || index == 0 {
        (start, QUANTUM_RANGE)
    } else if offset > 1.0 || index == stop_offsets.len() {
        (stop, QUANTUM_RANGE)
    } else {
        // CompositePixelInfoBlend(start, 1 - t, stop, t).
        let t = (offset - stop_offsets[index - 1]) / (stop_offsets[index] - stop_offsets[index - 1]);
        let source_alpha = QUANTUM_SCALE * ((1.0 - t) * QUANTUM_RANGE);
        let destination_alpha = QUANTUM_SCALE * (t * QUANTUM_RANGE);
        let gamma = (source_alpha + destination_alpha).clamp(0.0, 1.0);
        let alpha = QUANTUM_RANGE * gamma.clamp(0.0, 1.0);
        let color = perceptible_reciprocal(gamma) * (source_alpha * start + destination_alpha * stop);
        (color, alpha)
    };
    // CompositePixelInfoOver(composite, alpha, canvas, opaque).
    let source_alpha = QUANTUM_SCALE * alpha;
    let destination_alpha = QUANTUM_SCALE * QUANTUM_RANGE;
    let gamma = perceptible_reciprocal(source_alpha + destination_alpha - source_alpha * destination_alpha);
    let over = source_alpha * color + destination_alpha * start * (1.0 - source_alpha);
    gamma * over
}

/// The stop offset of pixel (`x`, `y`) in an IM linear gradient running down
/// the image (or left to right for a single row).
fn linear_gradient_offsets(width: usize, height: usize) -> Vec<f64> {
    let x2 = if height > 1 { 0.0 } else { width as f64 - 1.0 };
    let y2 = height as f64 - 1.0;
    let vector_length = (x2 * x2 + y2 * y2).sqrt();
    let inverse_length = perceptible_reciprocal(vector_length);
    let mut offsets = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            // GetStopColorOffset for a linear gradient from (0, 0).
            let qx = x as f64;
            let qy = y as f64;
            let length = (qx * qx + qy * qy).sqrt();
            let gamma = perceptible_reciprocal(vector_length * length);
            let scale = x2 * qx + y2 * qy;
            offsets.push(gamma * scale * length * inverse_length);
        }
    }
    offsets
}

/// IM `gradient:`: white at the top to black at the bottom (left to right
/// for a single row). IM builds it in sRGB with equal channels, which is
/// labelled GRAY here because it converts to grey losslessly.
pub fn linear_gradient_image(width: usize, height: usize) -> Image {
    let offsets = linear_gradient_offsets(width, height);
    let mut image = Image::new(width, height, 1);
    image.colorspace = Colorspace::Gray;
    image.data.par_iter_mut().zip(offsets.par_iter()).for_each(|(value, &offset)| {
        *value = (gradient_channel(offset, QUANTUM_RANGE, 0.0) / QUANTUM_RANGE) as f32;
    });
    image
}

/// IM `gradient:start-stop` with normalised RGB colours, as an sRGB image.
pub fn color_gradient_image(width: usize, height: usize, start: [f64; 3], stop: [f64; 3]) -> Image {
    let offsets = linear_gradient_offsets(width, height);
    let mut image = Image::new(width, height, 3);
    image.colorspace = Colorspace::SRgb;
    image.data.par_chunks_mut(3).zip(offsets.par_iter()).for_each(|(pixel, &offset)| {
        for (channel, value) in pixel.iter_mut().enumerate() {
            let quantum = gradient_channel(offset, start[channel] * QUANTUM_RANGE, stop[channel] * QUANTUM_RANGE);
            *value = (quantum / QUANTUM_RANGE) as f32;
        }
    });
    image
}

/// IM `radial-gradient:`: white centre to black at half the larger side,
/// as a grey image.
pub fn radial_gradient_image(width: usize, height: usize) -> Image {
    let center_x = (width as f64 - 1.0) / 2.0;
    let center_y = (height as f64 - 1.0) / 2.0;
    let radius = (width as f64 - 1.0).max(height as f64 - 1.0) / 2.0;
    let inverse_radius = perceptible_reciprocal(radius);
    let angle = degrees_to_radians(0.0);
    let cosine = angle.cos();
    let sine = angle.sin();
    let mut image = Image::new(width, height, 1);
    image.colorspace = Colorspace::Gray;
    let rows = image.data.par_chunks_mut(width);
    rows.enumerate().for_each(|(y, row)| {
        for (x, value) in row.iter_mut().enumerate() {
            let dx = x as f64 - center_x;
            let dy = y as f64 - center_y;
            let vx = (dx * cosine + dy * sine) * inverse_radius;
            let vy = (dx * sine - dy * cosine) * inverse_radius;
            let offset = (vx * vx + vy * vy).sqrt();
            *value = (gradient_channel(offset, QUANTUM_RANGE, 0.0) / QUANTUM_RANGE) as f32;
        }
    });
    image
}
