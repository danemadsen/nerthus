//! The shaped dilation (`Morphology2`):
//! disks, polygons, asterisks and corners built from flat line dilations
//! cast along rays in 16.16 fixed point.

use std::f32::consts::PI;

use nerthus_engine::Map;
use rayon::prelude::*;

use crate::maths::{self, dotnet_max, dotnet_min};
use crate::mathv;

/// `ApertureKernel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApertureKernel {
    Disk,
    Polygon,
    Asterisk,
    Line,
    Corner,
}

impl ApertureKernel {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Polygon" => Self::Polygon,
            "Asterisk" => Self::Asterisk,
            "Line" => Self::Line,
            "Corner" => Self::Corner,
            _ => Self::Disk,
        }
    }
}

/// 16.16 fixed point, as `Fixed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Fixed(i32);

impl Fixed {
    const ONE: Fixed = Fixed(1 << 16);
    const ZERO: Fixed = Fixed(0);

    fn from_int(value: i32) -> Self {
        Fixed(value << 16)
    }

    fn from_float(value: f32) -> Self {
        Fixed((value * 65536.0) as i32)
    }

    fn int(self) -> i32 {
        self.0 >> 16
    }

    fn float(self) -> f32 {
        self.0 as f32 / 65536.0
    }

    fn add(self, other: Fixed) -> Fixed {
        Fixed(self.0.wrapping_add(other.0))
    }

    fn sub(self, other: Fixed) -> Fixed {
        Fixed(self.0.wrapping_sub(other.0))
    }
}

/// `RayStep`: advances a ray and wraps it around the map; true when it
/// wrapped.
fn ray_step(x: &mut Fixed, y: &mut Fixed, dx: Fixed, dy: Fixed, size: Fixed) -> bool {
    *x = x.add(dx);
    *y = y.add(dy);
    let mut wrapped = false;
    if *y < Fixed::ZERO {
        *y = y.add(size);
        wrapped = true;
    }
    if *y >= size {
        *y = y.sub(size);
        wrapped = true;
    }
    if *x < Fixed::ZERO {
        *x = x.add(size);
        wrapped = true;
    }
    if *x >= size {
        *x = x.sub(size);
        wrapped = true;
    }
    wrapped
}

/// `Map.InterpolateBilinear(position)` on a square plane.
fn bilinear(data: &[f32], size: usize, position_x: f32, position_y: f32) -> f32 {
    let x = position_x - 0.5;
    let y = position_y - 0.5;
    let mut left = x as i32;
    let mut top = y as i32;
    let fraction_x = x - left as f32;
    let fraction_y = y - top as f32;
    left = left.max(0).min(size as i32 - 1);
    top = top.max(0).min(size as i32 - 1);
    let right = (left + 1).min(size as i32 - 1) as usize;
    let bottom = (top + 1).min(size as i32 - 1) as usize;
    let (left, top) = (left as usize, top as usize);
    let upper = data[top * size + left] * (1.0 - fraction_x) + data[top * size + right] * fraction_x;
    let lower = data[bottom * size + left] * (1.0 - fraction_x) + data[bottom * size + right] * fraction_x;
    upper * (1.0 - fraction_y) + lower * fraction_y
}

/// `DilateDiskNaive`: the maximum over a disk of `radius`, each sample
/// weighted by how far inside the rim it is (or thresholded).
fn dilate_disk(data: &[f32], size: usize, radius: f32, antialiased: bool) -> Vec<f32> {
    let limit = (radius - 0.5) * (radius - 0.5);
    // The squared distance is compared with the limit squared again.
    let limit_squared = limit * limit;
    let reach = radius as i32 + 1;
    let mut result = vec![0.0f32; size * size];
    result.par_chunks_mut(size).enumerate().for_each(|(row, values)| {
        let y = row as i32;
        let first_row = (y - reach).max(0);
        let last_row = (y + reach).min(size as i32 - 1);
        for (column, value) in values.iter_mut().enumerate() {
            let x = column as i32;
            let first_column = (x - reach).max(0);
            let last_column = (x + reach).min(size as i32 - 1);
            let mut maximum = -1e9f32;
            for sample_row in first_row..=last_row {
                for sample_column in first_column..=last_column {
                    let distance_squared = ((x - sample_column) * (x - sample_column) + (y - sample_row) * (y - sample_row)) as f32;
                    if distance_squared > limit_squared {
                        continue;
                    }
                    let distance = distance_squared.sqrt();
                    let mut weight = dotnet_min((radius - distance) * 1.4, 1.0);
                    if !antialiased {
                        weight = if weight > 0.5 { 1.0 } else { 0.0 };
                    }
                    let sample = data[sample_row as usize * size + sample_column as usize];
                    maximum = dotnet_max(maximum, sample * weight);
                }
            }
            *value = maximum;
        }
    });
    result
}

/// The per-ray buffers: values, segment starts and envelope ranges, with a
/// second set for the part after the ray wraps.
struct RayBuffers {
    values: [Vec<f32>; 2],
    centers: [Vec<usize>; 2],
    ranges: [Vec<f32>; 2],
}

impl RayBuffers {
    fn new(size: usize) -> Self {
        Self {
            values: [vec![0.0; size], vec![0.0; size]],
            centers: [vec![0; size], vec![0; size]],
            ranges: [vec![0.0; size + 1], vec![0.0; size + 1]],
        }
    }
}

/// Options of one dilation pass.
#[derive(Clone, Copy)]
struct RayOptions {
    kernel_size: f32,
    high_quality: bool,
    bilinear: bool,
    bidirectional: bool,
    fractional: bool,
}

/// `DilateRay`: a flat dilation of `kernel_size` along one ray, written
/// into `result` (spread over two cells with `high_quality`).
fn dilate_ray(source: &[f32], result: &mut [f32], size: usize, start: (Fixed, Fixed), step: (Fixed, Fixed), options: RayOptions, buffers: &mut RayBuffers) {
    let fixed_size = Fixed::from_int(size as i32);
    let (dx, dy) = step;
    let backward = if options.bidirectional { 1.0 } else { 0.0 };
    let kernel = options.kernel_size / (2.0 * (dx.float() * dx.float() + dy.float() * dy.float()).sqrt());
    let x_major = dx.float().abs() > dy.float().abs();

    let (mut x, mut y) = start;
    let mut part = 0;
    let mut position = 0usize;
    for _ in 0..size {
        buffers.values[part][position] = if options.bilinear {
            bilinear(source, size, x.float(), y.float())
        } else {
            source[y.int() as usize * size + x.int() as usize]
        };
        position += 1;
        if ray_step(&mut x, &mut y, dx, dy, fixed_size) {
            part = 1;
            position = 0;
        }
    }

    let (mut x, mut y) = start;
    part = 0;
    let mut segment = 0usize;
    let mut starting = true;
    let mut index = 0usize;
    for _ in 0..size {
        let values = &buffers.values[part];
        let centers = &mut buffers.centers[part];
        let ranges = &mut buffers.ranges[part];
        if starting {
            centers[0] = 0;
            ranges[0] = 0.0 - kernel;
            ranges[1] = kernel;
            starting = false;
            index = 0;
        } else {
            let mut begin = index as f32 - kernel * backward;
            if values[index] <= values[centers[segment]] {
                begin = ranges[segment + 1];
            }
            while begin < ranges[segment] {
                segment -= 1;
                begin = index as f32 - kernel * backward;
                if values[index] <= values[centers[segment]] {
                    begin = ranges[segment + 1];
                }
            }
            segment += 1;
            centers[segment] = index;
            ranges[segment] = begin;
            ranges[segment + 1] = index as f32 + kernel;
        }
        if ray_step(&mut x, &mut y, dx, dy, fixed_size) {
            part = 1;
            segment = 0;
            starting = true;
        }
        index += 1;
    }

    let (mut x, mut y) = start;
    part = 0;
    segment = 0;
    index = 0;
    let width = size as i32;
    for _ in 0..size {
        let values = &buffers.values[part];
        let centers = &buffers.centers[part];
        let ranges = &buffers.ranges[part];
        while ranges[segment + 1] < index as f32 {
            segment += 1;
        }
        let mut value = values[centers[segment]];
        if options.fractional {
            let mut blend = ranges[segment] - index as f32;
            blend += 1.0;
            blend *= 0.5;
            blend = blend.clamp(0.0, 1.0);
            let previous = values[centers[segment.saturating_sub(1)]];
            value = previous * blend + value * (1.0 - blend);
        }
        let column = x.int();
        let row = y.int();
        let cell = (row * width + column) as usize;
        if !options.high_quality {
            result[cell] = value;
        } else if x_major {
            let mut fraction = y.float() - row as f32 - 0.5;
            if fraction > 0.0 {
                result[cell] += value * (1.0 - fraction);
                if row < width - 2 {
                    result[cell + size] += value * fraction;
                }
            } else {
                fraction += 1.0;
                result[cell] += value * fraction;
                if row > 1 {
                    result[cell - size] += value * (1.0 - fraction);
                }
            }
            if row == 0 || row == width - 1 {
                result[cell] = value;
            }
        } else {
            let mut fraction = x.float() - column as f32 - 0.5;
            if fraction > 0.0 {
                result[cell] += value * (1.0 - fraction);
                if column < width - 2 {
                    result[cell + 1] += value * fraction;
                }
            } else {
                fraction += 1.0;
                result[cell] += value * fraction;
                if column > 1 {
                    result[cell - 1] += value * (1.0 - fraction);
                }
            }
            if column == 0 || column == width - 1 {
                result[cell] = value;
            }
        }
        index += 1;
        if ray_step(&mut x, &mut y, dx, dy, fixed_size) {
            part = 1;
            segment = 0;
            index = 0;
        }
    }
}

/// `DilateRayCast`: the rays of one direction across the whole map.
fn dilate_rays(source: &[f32], size: usize, direction: [f32; 2], options: RayOptions) -> Vec<f32> {
    let mut result = vec![0.0f32; size * size];
    let half = Fixed::from_float(0.5);
    let fixed_size = Fixed::from_int(size as i32);
    let mut step = [0.0 - direction[0], 0.0 - direction[1]];
    let absolute = [direction[0].abs(), direction[1].abs()];
    let rays = size / 16 * 16;
    let mut buffers = RayBuffers::new(size);
    if absolute[0] > absolute[1] {
        step = [step[0] / absolute[0], step[1] / absolute[0]];
        let step_y = Fixed::from_float(step[1]);
        for ray in 0..rays {
            let row = Fixed::from_int(ray as i32).add(half);
            let (start, step_x) = if step[0] > 0.0 { (half, Fixed::ONE) } else { (fixed_size.sub(Fixed::ONE).add(half), Fixed(-Fixed::ONE.0)) };
            dilate_ray(source, &mut result, size, (start, row), (step_x, step_y), options, &mut buffers);
        }
    } else {
        step = [step[0] / absolute[1], step[1] / absolute[1]];
        let step_x = Fixed::from_float(step[0]);
        for ray in 0..rays {
            let column = Fixed::from_int(ray as i32).add(half);
            let (start, step_y) = if step[1] > 0.0 { (half, Fixed::ONE) } else { (fixed_size.sub(Fixed::ONE).add(half), Fixed(-Fixed::ONE.0)) };
            dilate_ray(source, &mut result, size, (column, start), (step_x, step_y), options, &mut buffers);
        }
    }
    result
}

/// `_DilateND.Build`: one ray dilation per direction, chained (`serial`)
/// or combined by maximum.
fn dilate_directions(source: &[f32], size: usize, directions: &[[f32; 2]], sizes: &[f32], serial: bool, options: RayOptions) -> Vec<f32> {
    let mut heightmap = source.to_vec();
    let mut combined = vec![0.0f32; size * size];
    let mut last = vec![0.0f32; size * size];
    for (direction, &kernel_size) in directions.iter().zip(sizes) {
        let length = (direction[0] * direction[0] + direction[1] * direction[1]).sqrt();
        let normalized = [direction[0] / length, direction[1] / length];
        let pass = RayOptions {
            kernel_size,
            fractional: true,
            ..options
        };
        last = dilate_rays(&heightmap, size, normalized, pass);
        if serial {
            heightmap.copy_from_slice(&last);
        } else {
            combined.par_iter_mut().zip(last.par_iter()).for_each(|(value, &new)| *value = mathv::max(*value, new));
        }
    }
    if serial { last } else { combined }
}

/// The directions of a polygon or asterisk kernel with `count` spokes, in
/// the kernel's order (each next one as far from the previous as possible).
fn spoke_directions(count: usize, direction: f32) -> Vec<[f32; 2]> {
    let step = PI / count as f32;
    let mut remaining: Vec<f32> = (0..count)
        .map(|index| {
            let mut angle = index as f32 * step + direction;
            if angle > PI * 2.0 {
                angle -= PI * 2.0;
            }
            angle
        })
        .collect();
    let mut directions = Vec::with_capacity(count);
    let first = remaining.remove(0);
    let (sine, cosine) = maths::sin_cos(first);
    directions.push([cosine, sine]);
    let mut previous = first;
    for _ in 1..count {
        let mut best = 0.0f32;
        let mut chosen = None;
        for (position, &angle) in remaining.iter().enumerate() {
            let mut separation = (step * (angle - previous)).abs();
            separation = dotnet_min(separation, PI - separation);
            if separation > best {
                best = separation;
                chosen = Some(position);
            }
        }
        let position = chosen.expect("a remaining spoke is farther than zero");
        let angle = remaining.remove(position);
        let (sine, cosine) = maths::sin_cos(angle);
        directions.push([cosine, sine]);
        previous = angle;
    }
    directions
}

/// Settings of [`aperture`].
#[derive(Clone, Copy, Debug)]
pub struct ApertureSettings {
    /// Pixels.
    pub size: f32,
    pub kernel: ApertureKernel,
    pub erosion: bool,
    pub vertices: i32,
    pub antialiased: bool,
    /// Degrees.
    pub direction: f32,
    /// Degrees.
    pub corner_angle: f32,
}

/// `Morphology2.Aperture`: dilates (or erodes) by the chosen kernel shape.
pub fn aperture(map: &Map, settings: &ApertureSettings) -> Map {
    let size = map.width();
    let direction = settings.direction / 180.0 * PI;
    let corner_angle = settings.corner_angle / 180.0 * PI;
    let mut kernel = settings.kernel;
    let mut vertices = settings.vertices;
    if matches!(kernel, ApertureKernel::Polygon | ApertureKernel::Asterisk) {
        vertices = vertices.max(4);
    }
    if kernel == ApertureKernel::Line {
        kernel = ApertureKernel::Polygon;
        vertices = 2;
    }
    let mut source: Vec<f32> = map.data().to_vec();
    if settings.erosion {
        source.iter_mut().for_each(|value| *value = 1.0 - *value);
    }
    vertices /= 2;
    let mut serial = true;
    let mut bidirectional = true;
    let mut spokes = vertices.max(0) as usize;
    let finish = |mut data: Vec<f32>| {
        if settings.erosion {
            data.iter_mut().for_each(|value| *value = 1.0 - *value);
        }
        Map::from_data(maths::square_resolution(data.len()), data)
    };

    let (directions, sizes) = match kernel {
        ApertureKernel::Corner => {
            serial = false;
            bidirectional = false;
            let (first_sine, first_cosine) = maths::sin_cos(direction);
            let (second_sine, second_cosine) = maths::sin_cos(direction + corner_angle);
            let directions = vec![[first_cosine, first_sine], [second_cosine, second_sine]];
            (directions, vec![settings.size * 2.0, settings.size * 2.0])
        }
        _ => {
            if kernel == ApertureKernel::Disk {
                if settings.size <= 5.5 {
                    let disk = dilate_disk(&source, size, settings.size + 1.0, settings.antialiased);
                    return finish(disk);
                }
                let radius = dotnet_min(settings.size + 1.0, 4.5);
                source = dilate_disk(&source, size, radius, settings.antialiased);
                spokes = ((settings.size * 0.2) as i32).clamp(3, 30) as usize + 2;
                bidirectional = true;
            }
            let directions = spoke_directions(spokes, direction);
            if kernel == ApertureKernel::Asterisk {
                serial = false;
            }
            let spoke_size = settings.size * if serial { 1.0 / spokes as f32 * 2.0 } else { 1.0 };
            (directions, vec![spoke_size; spokes])
        }
    };

    let options = RayOptions {
        kernel_size: 0.0,
        high_quality: settings.antialiased,
        bilinear: settings.antialiased,
        bidirectional,
        fractional: true,
    };
    let dilated = dilate_directions(&source, size, &directions, &sizes, serial, options);
    finish(dilated)
}
