//! The terrain lighting (the engine's lighting class): sunlight
//! integrated over a range of days (the native `SunLightIntegralIspc`,
//! ported from its AVX2 ISPC variant, the one Rosetta and most CPUs run).
//!
//! The light is gathered at half resolution by sweeping lines of sight
//! across the map towards each sampled sun position; cells that rise above
//! the line add their lit amount in 16.16 fixed point. Integer sums do not
//! depend on order, so sequential sweeps match parallel ones.

use std::sync::atomic::{AtomicI32, Ordering};

use nerthus_compute::DeviceType;
use nerthus_engine::Map;
use rayon::prelude::*;

use crate::{gpu, maths, sycl, x86};

/// One degree in radians, single precision.
pub(crate) const DEGREE: f32 = 0.017453292;
/// One degree in radians, double precision.
const DEGREE_F64: f64 = 0.0174532925199433;
/// `0.65⁴`: the red wavelength (µm) to the fourth, for Rayleigh scattering.
const RED_WAVELENGTH_POWER: f32 = 0.17850628;
/// `2⁻³²`.
const UNIT_SCALE: f32 = 2.3283064e-10;
/// 16.16 fixed point.
const FIXED_ONE: i32 = 1 << 16;
pub(crate) const FIXED_SCALE: f32 = 65536.0;

/// The xorshift32 step the native code uses for its random streams.
pub(crate) fn xorshift(state: u32) -> u32 {
    let mut value = state;
    value ^= value << 13;
    value ^= value >> 17;
    value ^= value << 5;
    value
}

/// The uniform [0, 1) value of a xorshift state (`state · 0x2545F381 ·
/// 2⁻³²`, the product converted as an unsigned 32-bit integer).
pub(crate) fn unit(state: u32) -> f32 {
    let mixed = state.wrapping_mul(0x2545_F381);
    mixed as f32 * UNIT_SCALE
}

/// The sun directions sampled over the days, with the light each brings
/// through the atmosphere.
struct SunSamples {
    /// Days sampled, accepted or not (the light's normalisation).
    count: usize,
    directions: Vec<[f32; 3]>,
    intensities: Vec<f32>,
}

/// Settings of [`sunlight_integral`] (`NativeLighting.Parameters`).
#[derive(Clone, Copy, Debug)]
pub struct SunlightSettings {
    pub day_begin: i32,
    pub day_end: i32,
    pub latitude: f32,
    pub north: f32,
    /// `AirDensity` (the lighting default is 0.01).
    pub air_density: f32,
}

/// The sun position loop of `SunLightIntegralIspc`: jittered times spread
/// over the day range, the solar azimuth and elevation at each, and the
/// air mass the light crosses. Positions below −0.15 (the sun under the
/// horizon) or straight overhead are dropped.
fn sample_suns(settings: &SunlightSettings) -> SunSamples {
    let day_begin = settings.day_begin;
    let day_end = if day_begin <= settings.day_end { settings.day_end } else { settings.day_end + 365 };
    let days = (day_end - day_begin + 1) as f32;
    let count = x86::cvttss2si((days * 2000.0) / (days + 2.0));
    let air = settings.air_density / RED_WAVELENGTH_POWER;

    let mut state: u32 = 0xA235_1342;
    let mut jitter = 0.5f32;
    let mut directions = Vec::new();
    let mut intensities = Vec::new();
    let latitude = settings.latitude * DEGREE;
    for sample in 0..count.max(0) {
        state = xorshift(state);
        jitter = (unit(state) + jitter) * 0.5;
        let time = ((sample as f32 + jitter) * days) / count as f32 + day_begin as f32;
        let day = x86::cvttss2si(time) as f32;
        let declination = maths::sin((day + 284.0) * 0.98630136 * DEGREE) * 23.45 * DEGREE;
        let hour_angle = ((time - day) * 24.0 - 12.0) * 15.0;
        let latitude_sine = maths::sin(latitude);
        let declination_sine = maths::sin(declination);
        let hour_cosine = maths::cos(hour_angle * DEGREE);
        let latitude_cosine = maths::cos(latitude);
        let declination_cosine = maths::cos(declination);
        let elevation = maths::asin(declination_cosine * latitude_cosine * hour_cosine + declination_sine * latitude_sine);
        let elevation_degrees = elevation / DEGREE;
        let azimuth = if hour_angle == 0.0 {
            180.0
        } else {
            let numerator = latitude_cosine * declination_sine - declination_cosine * latitude_sine * hour_cosine;
            let angle = maths::acos(numerator / maths::cos(elevation)) / DEGREE;
            if hour_angle > 0.0 { 360.0 - angle } else { angle }
        };
        let heading = (settings.north + azimuth) as f64 * DEGREE_F64;
        let toward_x = std::hint::black_box(heading).sin() as f32;
        let toward_y = (0.0 - std::hint::black_box(heading).cos()) as f32;
        let rise = (std::hint::black_box(elevation_degrees as f64 * DEGREE_F64)).tan() as f32;
        let length_squared = toward_y * toward_y + toward_x * toward_x + rise * rise;
        let inverse = 1.0 / length_squared.sqrt();
        let z = rise * inverse;
        state = xorshift(state);
        let wobble = unit(state);
        if !(z > -0.15) {
            continue;
        }
        let x = toward_x * inverse;
        let y = toward_y * inverse;
        if x == 0.0 && y == 0.0 {
            continue;
        }
        directions.push([x, y, z]);
        let wobbled = ((wobble * 0.5 - 0.25) + elevation_degrees) as f64 * DEGREE_F64;
        let height = x86::maxss(0.0, std::hint::black_box(wobbled).sin() as f32);
        let haze = maths::exp(height * -11.0);
        let air_mass = -1.0 / (haze * 0.025 + height);
        intensities.push(maths::exp(air_mass * air));
    }
    SunSamples {
        count: count.max(0) as usize,
        directions,
        intensities,
    }
}

/// The sun directions (with their intensities) that [`sunlight_integral`]
/// sweeps for `settings`, and how many times were sampled in all.
pub fn sun_directions(settings: &SunlightSettings) -> (usize, Vec<([f32; 3], f32)>) {
    let suns = sample_suns(settings);
    let lit_suns = suns.directions.iter().copied().zip(suns.intensities.iter().copied()).collect();
    (suns.count, lit_suns)
}

/// The [1 3 3 1]² downsampling to half resolution, summed in the native order.
pub fn downsample(heights: &[f32], width: usize, height: usize) -> Vec<f32> {
    let half_width = width >> 1;
    let half_height = height >> 1;
    let last_column = width as i32 - 1;
    let last_row = height as i32 - 1;
    let mut coarse = vec![0.0f32; half_width * half_height];
    coarse.par_chunks_mut(half_width).enumerate().for_each(|(row, values)| {
        let y = row as i32 * 2;
        let rows = [(y - 1).max(0), y, (y + 1).min(last_row), (y + 2).min(last_row)];
        for (column, value) in values.iter_mut().enumerate() {
            let x = column as i32 * 2;
            let columns = [(x - 1).max(0), x, (x + 1).min(last_column), (x + 2).min(last_column)];
            let at = |r: usize, c: usize| heights[rows[r] as usize * width + columns[c] as usize];
            let mut sum = at(0, 1) * 3.0 + at(0, 0);
            sum += at(0, 2) * 3.0;
            sum += at(0, 3);
            sum += at(1, 0) * 3.0;
            sum += at(1, 1) * 9.0;
            sum += at(1, 2) * 9.0;
            sum += at(1, 3) * 3.0;
            sum += at(2, 0) * 3.0;
            sum += at(2, 1) * 9.0;
            sum += at(2, 2) * 9.0;
            sum += at(2, 3) * 3.0;
            sum += at(3, 0);
            sum += at(3, 1) * 3.0;
            sum += at(3, 2) * 3.0;
            sum += at(3, 3);
            *value = sum * 0.015625;
        }
    });
    coarse
}

/// Unit surface normals from a Scharr (3, 10, 3) gradient, normalised with
/// ISPC's refined reciprocal of the length (0 for a zero length).
pub fn normals(heights: &[f32], width: usize, height: usize, terrain: (f32, f32)) -> [Vec<f32>; 3] {
    let cell = (terrain.0 / width as f32) * 32.0;
    let x_scale = 0.0 - terrain.1;
    let y_scale = terrain.1;
    let count = width * height;
    let mut normal_x = vec![0.0f32; count];
    let mut normal_y = vec![0.0f32; count];
    let mut normal_z = vec![0.0f32; count];
    let rows = normal_x.par_chunks_mut(width).zip(normal_y.par_chunks_mut(width)).zip(normal_z.par_chunks_mut(width));
    rows.enumerate().for_each(|(row, ((row_x, row_y), row_z))| {
        let up = row.saturating_sub(1);
        let down = (row + 1).min(height - 1);
        for column in 0..width {
            let left = if column >= 2 { column - 1 } else { 0 };
            let right = if column >= width - 1 { width - 1 } else { column + 1 };
            let at = |r: usize, c: usize| heights[r * width + c];
            let up_left = at(up, left);
            let up_right = at(up, right);
            let middle_left = at(row, left);
            let middle_right = at(row, right);
            let down_left = at(down, left);
            let down_right = at(down, right);
            let gradient_x = middle_right.mul_add(10.0, up_right * 3.0) + down_right * 3.0 - up_left * 3.0;
            let gradient_x = (-10.0f32).mul_add(middle_left, gradient_x) - down_left * 3.0;
            let gradient_y = at(up, column).mul_add(10.0, up_left * 3.0) + up_right * 3.0 - down_left * 3.0;
            let gradient_y = (-10.0f32).mul_add(at(down, column), gradient_y) - down_right * 3.0;
            let nx = gradient_x * x_scale;
            let ny = gradient_y * y_scale;
            let length_squared = ny.mul_add(ny, nx * nx) + cell * cell;
            let inverse = if length_squared != 0.0 { x86::rcp_refined(length_squared.sqrt()) } else { 0.0 };
            row_x[column] = nx * inverse;
            row_y[column] = ny * inverse;
            row_z[column] = cell * inverse;
        }
    });
    [normal_x, normal_y, normal_z]
}

/// Bilinear height at `(x, y)` in cell units, with the cell indices (not
/// the position) clamped to the map, interpolated with fused
/// multiply-adds.
pub(crate) fn sample_bilinear(heights: &[f32], width: i32, x: f32, y: f32) -> f32 {
    let last = width - 1;
    let column = x86::cvttss2si(x);
    let row = x86::cvttss2si(y);
    let x0 = column.clamp(0, last);
    let x1 = column.wrapping_add(1).clamp(0, last);
    let y0 = row.clamp(0, last);
    let y1 = row.wrapping_add(1).clamp(0, last);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let at = |c: i32, r: i32| heights[(r * width + c) as usize];
    let top = (1.0 - fx).mul_add(at(x0, y0), fx * at(x1, y0));
    let bottom = (1.0 - fx).mul_add(at(x0, y1), fx * at(x1, y1));
    (1.0 - fy).mul_add(top, fy * bottom)
}

/// Walks the lines of one sweep towards `direction` (pointing from the
/// ground to the light): one line per row or column, whichever the
/// direction crosses faster, each starting `offset` into its first cell on
/// the side the light comes from. `trace` gets every line's 16.16 start,
/// its 16.16 steps and the height the line drops per step.
pub(crate) fn for_each_line(width: usize, direction: [f32; 3], offset: (f32, f32), vertical_scale: f32, mut trace: impl FnMut(i32, i32, i32, i32, f32)) {
    let plan = plan_lines(width, direction, offset, vertical_scale);
    for line in 0..width {
        let along = x86::cvttss2si(line as f32 * FIXED_SCALE);
        let (x, y) = if plan.x_major { (plan.start.0, along + plan.start.1) } else { (along + plan.start.0, plan.start.1) };
        trace(x, y, plan.step.0, plan.step.1, plan.drop);
    }
}

/// The lines of one sweep: the first line's 16.16 start (the others
/// advance a cell along y when the direction is x-major, along x
/// otherwise), their 16.16 step and the height they drop per step.
pub(crate) struct LinePlan {
    pub(crate) start: (i32, i32),
    pub(crate) step: (i32, i32),
    pub(crate) drop: f32,
    pub(crate) x_major: bool,
}

pub(crate) fn plan_lines(width: usize, direction: [f32; 3], offset: (f32, f32), vertical_scale: f32) -> LinePlan {
    let far_edge = (width as i32) * FIXED_ONE - FIXED_ONE;
    let offset_x = x86::cvttss2si(offset.0 * FIXED_SCALE);
    let offset_y = x86::cvttss2si(offset.1 * FIXED_SCALE);
    let reach = [0.0 - direction[0], 0.0 - direction[1], (0.0 - direction[2]) * vertical_scale];
    let magnitude_x = direction[0].abs();
    let magnitude_y = direction[1].abs();
    let x_major = magnitude_y < magnitude_x;
    let major = if x_major { magnitude_x } else { magnitude_y };
    let step = [reach[0] / major, reach[1] / major, reach[2] / major];
    let step_x = x86::cvttss2si(step[0] * FIXED_SCALE);
    let step_y = x86::cvttss2si(step[1] * FIXED_SCALE);
    let start = if x_major {
        let x = if 0.0 < step[0] { offset_x } else { far_edge + offset_x };
        (x, offset_y)
    } else {
        let y = if 0.0 < step[1] { offset_y } else { far_edge + offset_y };
        (offset_x, y)
    };
    LinePlan { start, step: (step_x, step_y), drop: step[2], x_major }
}

/// The maps one light sweep reads.
struct Sweep<'a> {
    heights: &'a [f32],
    normals: &'a [Vec<f32>; 3],
    width: i32,
}

impl Sweep<'_> {
    /// One line of sight, `width` steps across the wrapping map: cells
    /// that rise above it are lit and raise it.
    #[allow(clippy::too_many_arguments)]
    fn trace(&self, mut x: i32, mut y: i32, step_x: i32, step_y: i32, drop: f32, sun: [f32; 3], scale: f32, light: &[AtomicI32]) {
        let wrap = self.width << 16;
        let previous_x = x.wrapping_sub(step_x) as f32 * (1.0 / FIXED_SCALE);
        let previous_y = y.wrapping_sub(step_y) as f32 * (1.0 / FIXED_SCALE);
        let mut ray = sample_bilinear(self.heights, self.width, previous_x + -0.5, previous_y + -0.5);
        let last = self.width - 1;
        for _ in 0..self.width {
            let mut wrapped = false;
            if y < 0 {
                y = y.wrapping_add(wrap);
                wrapped = true;
            } else if y >= wrap {
                y = y.wrapping_sub(wrap);
                wrapped = true;
            }
            if x < 0 {
                x = x.wrapping_add(wrap);
                wrapped = true;
            } else if x >= wrap {
                x = x.wrapping_sub(wrap);
                wrapped = true;
            }
            ray += drop;
            let column = x >> 16;
            let row = y >> 16;
            if wrapped {
                let index = row.clamp(0, last) * self.width + column.clamp(0, last);
                ray = self.heights[index as usize] + drop;
            }
            let index = (row * self.width + column) as usize;
            let height = self.heights[index];
            if ray < height {
                let normal_x = self.normals[0][index];
                let normal_y = self.normals[1][index];
                let normal_z = self.normals[2][index];
                let facing = normal_x.mul_add(sun[0], normal_y * sun[1]);
                let facing = normal_z.mul_add(sun[2], facing);
                let lit = x86::cvttss2si(facing * scale);
                light[index].fetch_add(lit, Ordering::Relaxed);
                ray = x86::maxss(height, ray);
            }
            x = x.wrapping_add(step_x);
            y = y.wrapping_add(step_y);
        }
    }
}

/// The light gathered by sweeping lines of sight towards each sun (and
/// its intensity) across a square map, in 16.16 units. Suns sweep in
/// parallel; the integer sums do not depend on their order.
pub fn sweep_light(heights: &[f32], normals: &[Vec<f32>; 3], width: usize, suns: &[([f32; 3], f32)], vertical_scale: f32) -> Vec<i32> {
    let sweep = Sweep {
        heights,
        normals,
        width: width as i32,
    };
    let light: Vec<AtomicI32> = heights.iter().map(|_| AtomicI32::new(0)).collect();
    suns.par_iter().for_each(|(sun, intensity)| {
        let scale = 262144.0 * *intensity;
        for_each_line(width, *sun, (0.5, 0.5), vertical_scale, |x, y, step_x, step_y, drop| {
            sweep.trace(x, y, step_x, step_y, drop, *sun, scale, &light);
        });
    });
    light.into_iter().map(AtomicI32::into_inner).collect()
}

/// The 9/3/3/1 upsampling of a half-size map to `width × height`.
pub(crate) fn upsample(coarse: &[f32], coarse_width: usize, coarse_height: usize, width: usize, height: usize) -> Vec<f32> {
    let last_column = coarse_width as i32 - 1;
    let last_row = coarse_height as i32 - 1;
    let mut fine = vec![0.0f32; width * height];
    fine.par_chunks_mut(width).enumerate().for_each(|(y, values)| {
        let center_y = (y >> 1) as i32;
        let neighbour_y = (center_y + (y as i32 & 1) * 2 - 1).clamp(0, last_row);
        for (x, value) in values.iter_mut().enumerate() {
            let center_x = (x >> 1) as i32;
            let neighbour_x = (center_x + (x as i32 & 1) * 2 - 1).clamp(0, last_column);
            let at = |c: i32, r: i32| coarse[(r * coarse_width as i32 + c) as usize];
            let mut sum = at(neighbour_x, center_y) * 3.0 + at(center_x, center_y) * 9.0;
            sum += at(center_x, neighbour_y) * 3.0;
            sum += at(neighbour_x, neighbour_y);
            *value = sum * 0.0625;
        }
    });
    fine
}

/// `SunLightIntegral`: the share of daylight each cell receives over the
/// days from `day_begin` to `day_end` (wrapping past the year end), at the
/// given latitude and north direction. It runs as SYCL kernels on a
/// discrete GPU when any GPU or accelerator is active,
/// otherwise as the native `SunLightIntegralIspc`.
pub fn sunlight_integral(heightmap: &Map, settings: &SunlightSettings, terrain: (f32, f32)) -> Map {
    let cpu = || sunlight_integral_cpu(heightmap, settings, terrain);
    let types = [DeviceType::GpuDiscrete, DeviceType::CpuAccelerated, DeviceType::Accelerator, DeviceType::GpuIntegrated, DeviceType::Cpu];
    if nerthus_compute::first_active_device(&types).device_type == DeviceType::Cpu {
        return cpu();
    }
    let gpu = |device: &nerthus_compute::GpuDevice| gpu::lighting::sunlight_integral(device, heightmap, settings, terrain);
    sycl::run("SunLightIntegral", heightmap.width(), sycl::DISCRETE, gpu, cpu)
}

/// [`sunlight_integral`] on the CPU (`SunLightIntegralIspc`).
pub fn sunlight_integral_cpu(heightmap: &Map, settings: &SunlightSettings, terrain: (f32, f32)) -> Map {
    let resolution = heightmap.resolution();
    let half_width = resolution.x >> 1;
    let half_height = resolution.y >> 1;
    let coarse = downsample(heightmap.data(), resolution.x, resolution.y);
    let normal_maps = normals(&coarse, half_width, half_height, terrain);
    let vertical_scale = terrain.0 / (half_width as f32 * terrain.1);

    let (count, lit_suns) = sun_directions(settings);
    let light = sweep_light(&coarse, &normal_maps, half_width, &lit_suns, vertical_scale);
    let unit_light = 0.67 / (((count as i32) << 16) as f32);
    // Suns below the horizon can leave a cell with negative light; it is
    // clamped to zero.
    let coarse_light: Vec<f32> = light.iter().map(|&value| value.max(0) as f32 * unit_light).collect();
    let output = upsample(&coarse_light, half_width, half_height, resolution.x, resolution.y);
    Map::from_data(resolution, output)
}
