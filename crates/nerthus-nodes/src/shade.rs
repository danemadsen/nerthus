//! The terrain renderer: the native `ShadeIspc` (ported from its AVX2 ISPC
//! variant) and the engine's lighting rig that feeds it.
//! It lights a heightfield with direct sunlight, soft shadows and ambient
//! occlusion, and mixes them with a colour texture. The Shade, AO and
//! LightX nodes use it.
//!
//! Shadows and raytraced occlusion are sweeps like the sunlight integral's
//! (see [`lighting`]): lines of sight in 16.16 fixed point that count or
//! add up light in integers, so the parallel order does not change them.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};

use nerthus_engine::Map;
use rayon::prelude::*;

use crate::dotnet_vector::{self, Vector3};
use crate::lighting::{self, DEGREE, FIXED_SCALE};
use crate::{gpu, maths, sycl, x86};

/// `LightingAmbientOcclusionModel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AmbientOcclusionModel {
    /// Lines of sight from every direction of a sphere, at full resolution.
    Raytraced,
    /// Raytraced on a coarser level of a pyramid, then refined by the
    /// local relief at full resolution.
    Hybrid,
    /// From each cell's slope and curvature alone.
    Local,
}

/// The lighting rig in front of the native renderer, with its
/// defaults.
#[derive(Clone, Debug)]
pub struct LightRig {
    /// Sun and sky colours from the atmosphere model rather than
    /// [`Self::sun_color`] and [`Self::ambient_color`].
    pub automatic_colors: bool,
    /// The albedo when there is no texture.
    pub default_color: f32,
    pub ao_model: AmbientOcclusionModel,
    /// Rings of soft-shadow samples (−1, −4 and −8 pick fixed
    /// sample patterns).
    pub shadow_quality: i32,
    /// The sun's angular diameter in degrees.
    pub sun_angular_diameter: f32,
    pub air_density: f32,
    /// The sky colour when the colours are not automatic.
    pub ambient_color: Vector3,
    /// The sun colour when the colours are not automatic.
    pub sun_color: Vector3,
    pub exposure: f32,
    pub sun_intensity: f32,
    pub ambient_intensity: f32,
    /// Icosphere subdivisions of the hybrid occlusion.
    pub hybrid_ao_quality: i32,
    /// Pyramid levels of the hybrid occlusion.
    pub hybrid_ao_octaves: i32,
    /// Icosphere subdivisions of the raytraced occlusion.
    pub raytraced_ao_quality: i32,
    /// The red, green and blue wavelengths in nanometres.
    pub wavelengths: Vector3,
    pub aerosols: f32,
    /// The ozone (Chappuis) absorption.
    pub ozone: f32,
    pub direct_lighting: bool,
    pub ambient_lighting: bool,
    pub shadows: bool,
    pub ambient_occlusion: bool,
}

impl Default for LightRig {
    fn default() -> Self {
        Self {
            automatic_colors: true,
            default_color: 0.5,
            ao_model: AmbientOcclusionModel::Raytraced,
            shadow_quality: 5,
            sun_angular_diameter: 0.5,
            air_density: 0.01,
            ambient_color: [0.5, 0.9, 1.0],
            sun_color: [1.0, 0.8, 0.6],
            exposure: 1.0,
            sun_intensity: 1.0,
            ambient_intensity: 1.0,
            hybrid_ao_quality: 3,
            hybrid_ao_octaves: 4,
            raytraced_ao_quality: 4,
            wavelengths: [600.0, 540.0, 460.0],
            aerosols: 0.05,
            ozone: 0.0025,
            direct_lighting: true,
            ambient_lighting: true,
            shadows: true,
            ambient_occlusion: true,
        }
    }
}

/// The sun's direction (pointing from the ground to the sun) and the sun
/// and sky colours for one sun position.
#[derive(Clone, Copy, Debug)]
pub struct Sky {
    pub sun_direction: Vector3,
    pub sun_color: Vector3,
    pub ambient_color: Vector3,
}

/// The maps the renderer produces. Maps it does not compute for the rig's
/// settings stay zero.
pub struct ShadeMaps {
    /// The sun's cosine on each cell.
    pub diffuse: Map,
    /// The share of the sun's disk each cell sees.
    pub shadow: Map,
    /// The share of the sky each cell sees.
    pub occlusion: Map,
    /// The lit colour.
    pub color: Map,
}

/// The sun's azimuth and elevation in degrees at
/// `latitude` on `day` of the year at solar `hour`.
pub fn sun_position(latitude: f32, day: i32, hour: f32) -> (f32, f32) {
    let latitude_angle = latitude * DEGREE;
    let declination = 23.45 * maths::sin(0.98630136 * (284.0 + day as f32) * DEGREE) * DEGREE;
    let hour_angle = 15.0 * (hour - 12.0);
    let hour_radians = hour_angle * DEGREE;
    let rise = maths::sin(declination) * maths::sin(latitude_angle) + maths::cos(declination) * maths::cos(latitude_angle) * maths::cos(hour_radians);
    let elevation_radians = maths::asin(rise);
    let elevation = elevation_radians / DEGREE;
    if hour_angle == 0.0 {
        return (180.0, elevation);
    }
    let toward_north = maths::sin(declination) * maths::cos(latitude_angle) - maths::cos(declination) * maths::sin(latitude_angle) * maths::cos(hour_radians);
    let azimuth = maths::acos(toward_north / maths::cos(elevation_radians)) / DEGREE;
    if hour_angle > 0.0 {
        return (360.0 - azimuth, elevation);
    }
    (azimuth, elevation)
}

/// `vector · factor`.
fn scale(vector: Vector3, factor: f32) -> Vector3 {
    [vector[0] * factor, vector[1] * factor, vector[2] * factor]
}

/// `vector / divisor`, component by component (.NET 8's `Vector3 / float`).
fn divide(vector: Vector3, divisor: f32) -> Vector3 {
    [vector[0] / divisor, vector[1] / divisor, vector[2] / divisor]
}

/// `e^(factor · values)` for each component.
fn exp_each(factor: f32, values: Vector3) -> Vector3 {
    [maths::exp(factor * values[0]), maths::exp(factor * values[1]), maths::exp(factor * values[2])]
}

impl LightRig {
    /// The sun's direction at `azimuth` and `elevation`
    /// (degrees) and the sun and sky colours the atmosphere gives it.
    pub fn sky(&self, azimuth: f32, elevation: f32) -> Sky {
        let elevation = if elevation == 90.0 { 89.99999 } else { elevation };
        let toward_sun = [maths::sin(DEGREE * azimuth), 0.0 - maths::cos(DEGREE * azimuth), maths::tan(DEGREE * elevation)];
        let sun_direction = dotnet_vector::normalize(toward_sun);

        // Rayleigh scattering: the air density over the wavelength to the
        // fourth.
        let mut wavelength = scale(self.wavelengths, 0.001);
        wavelength = dotnet_vector::multiply(wavelength, wavelength);
        wavelength = dotnet_vector::multiply(wavelength, wavelength);
        let air = 1.35 * self.air_density;
        let rayleigh = [air / wavelength[0], air / wavelength[1], air / wavelength[2]];

        let height = maths::sin(DEGREE * maths::dotnet_max(elevation, 0.0));
        let mut air_mass = 1.0 / (height + 0.025 * maths::exp(-11.0 * height));
        air_mass = 42.0 * maths::tanh(air_mass / 42.0);
        air_mass *= 0.6;
        let mut sun_light = exp_each(0.0 - air_mass, rayleigh);

        let raised = 0.707 + height;
        let mut sky_mass = 2.0 + 4.0 / (raised * 0.5 + 0.05 * maths::exp(-11.0 * raised * 0.5));
        let ozone = 0.0 - self.ozone;
        let total_mass = sky_mass + air_mass;
        let sky_ozone = [maths::exp(ozone * 1.4 * total_mass), maths::exp(ozone * 1.2 * total_mass), 1.0];
        let sun_ozone = [maths::exp(ozone * 1.4 * air_mass), maths::exp(ozone * 1.2 * air_mass), 1.0];
        sun_light = dotnet_vector::multiply(sun_light, sun_ozone);

        let density = self.air_density / 0.08;
        sky_mass = 1.2 * sky_mass / (1.0 + sky_mass * density);
        let haze = 0.75 * self.aerosols;
        let extinction = dotnet_vector::add(rayleigh, [haze, haze, haze]);
        let scattered = exp_each(-0.5 * sky_mass, extinction);
        let lost = [1.0 - scattered[0], 1.0 - scattered[1], 1.0 - scattered[2]];
        let four_pi = std::f32::consts::PI * 4.0;
        let mut multiple = dotnet_vector::multiply(lost, extinction);
        multiple = scale(multiple, four_pi);
        multiple = scale(multiple, maths::exp((0.0 - sky_mass) * 0.4));
        let mut sky = dotnet_vector::add(scale(lost, 0.5), scale(multiple, 1.5));
        let reddening = exp_each(-0.58 * sky_mass, rayleigh);
        sky = dotnet_vector::multiply(sky, reddening);
        sky = dotnet_vector::multiply(sky, sky_ozone);

        if sun_light[0] != 0.0 {
            sun_light = divide(sun_light, sun_light[0]);
        }
        let brightest = maths::dotnet_max(sky[0], maths::dotnet_max(sky[1], sky[2]));
        if brightest != 0.0 {
            sky = divide(sky, brightest);
        }
        let horizon = 1.0 / (height + 0.12 * maths::exp(-6.0 * height));
        sky = scale(sky, maths::exp(-0.02 * horizon + 0.005 - 0.75 * self.aerosols * 2.0));

        let (sun_color, ambient_color) = if self.automatic_colors { (sun_light, sky) } else { (self.sun_color, self.ambient_color) };
        Sky {
            sun_direction,
            sun_color,
            ambient_color: scale(ambient_color, 0.5),
        }
    }

    /// Lights `heightmap` (its first channel) under
    /// `sky`, over the colours of `texture` or the default grey. It runs as
    /// SYCL kernels on a discrete GPU, except for local occlusion and
    /// hybrid occlusion with many octaves at low resolutions, which (like
    /// every render without a discrete GPU) go to the native `ShadeIspc`.
    pub fn shade(&self, sky: &Sky, heightmap: &Map, texture: Option<&Map>, terrain: (f32, f32)) -> ShadeMaps {
        let cpu = || self.shade_cpu(sky, heightmap, texture, terrain);
        let resolution = heightmap.width();
        let octaves = self.hybrid_ao_octaves;
        let small_hybrid = self.ao_model == AmbientOcclusionModel::Hybrid && ((octaves >= 4 && resolution <= 2048) || (octaves >= 3 && resolution <= 1024));
        let on_cpu = !sycl::discrete_gpu_active() || self.ao_model == AmbientOcclusionModel::Local || small_hybrid;
        if on_cpu {
            return cpu();
        }
        let gpu = |device: &nerthus_compute::GpuDevice| gpu::lighting::shade(device, self, sky, heightmap, texture, terrain);
        sycl::run("Shade", resolution, sycl::DISCRETE, gpu, cpu)
    }

    /// [`Self::shade`] on the CPU (`ShadeIspc`).
    pub fn shade_cpu(&self, sky: &Sky, heightmap: &Map, texture: Option<&Map>, terrain: (f32, f32)) -> ShadeMaps {
        let resolution = heightmap.resolution();
        let width = resolution.x;
        let height = resolution.y;
        let heights = heightmap.channel(0);
        let area = width * height;
        let normal_maps = lighting::normals(heights, width, height, terrain);

        let diffuse = if self.direct_lighting { direct_light(&normal_maps, sky.sun_direction) } else { vec![0.0; area] };
        let shadow = if self.direct_lighting && self.shadows {
            soft_shadows(heights, width, sky.sun_direction, self, terrain)
        } else {
            vec![0.0; area]
        };
        let occlusion = if self.ambient_lighting && self.ambient_occlusion {
            match self.ao_model {
                AmbientOcclusionModel::Raytraced => raytraced_occlusion(heights, &normal_maps, width, self.raytraced_ao_quality, terrain),
                AmbientOcclusionModel::Hybrid => hybrid_occlusion(heights, &normal_maps[2], width, self, terrain),
                AmbientOcclusionModel::Local => local_occlusion(heights, width, height, terrain),
            }
        } else {
            vec![0.0; area]
        };

        let color = self.combine(sky, &diffuse, &shadow, &occlusion, texture, resolution);
        ShadeMaps {
            diffuse: Map::from_data(resolution, diffuse),
            shadow: Map::from_data(resolution, shadow),
            occlusion: Map::from_data(resolution, occlusion),
            color,
        }
    }

    /// The lit colour: albedo × (sun light × the lit
    /// share of the direct light + sky light × the open share of the sky).
    /// Terms switched off count as fully lit or as no light.
    fn combine(&self, sky: &Sky, diffuse: &[f32], shadow: &[f32], occlusion: &[f32], texture: Option<&Map>, resolution: nerthus_engine::Resolution) -> Map {
        let sun_scale = self.exposure * self.sun_intensity;
        let sun_light = scale(sky.sun_color, sun_scale);
        let ambient_scale = self.exposure * self.ambient_intensity;
        let ambient_light = scale(sky.ambient_color, ambient_scale);
        let unshadowed = if self.direct_lighting { 1.0 } else { 0.0 };
        let unoccluded = if self.ambient_lighting { 1.0 } else { 0.0 };
        let shadow_map = self.direct_lighting && self.shadows;
        let occlusion_map = self.ambient_lighting && self.ambient_occlusion;

        let mut color = Map::new(resolution, 3);
        for channel in 0..3 {
            let albedo = texture.map(|texture| texture.rgb_channel(channel));
            let values = color.channel_mut(channel);
            values.par_iter_mut().enumerate().for_each(|(index, value)| {
                let lit = if shadow_map { shadow[index] } else { unshadowed };
                let open = if occlusion_map { occlusion[index] } else { unoccluded };
                let facing = if self.direct_lighting { diffuse[index] } else { 1.0 };
                let direct = lit * facing;
                let light = direct.mul_add(sun_light[channel], open * ambient_light[channel]);
                let base = albedo.map_or(self.default_color, |albedo| albedo[index]);
                *value = base * light;
            });
        }
        color
    }
}

/// The sun's cosine on each cell, never negative.
fn direct_light(normals: &[Vec<f32>; 3], sun: Vector3) -> Vec<f32> {
    let [normal_x, normal_y, normal_z] = normals;
    let count = normal_x.len();
    (0..count)
        .into_par_iter()
        .map(|index| {
            let facing = sun[0].mul_add(normal_x[index], sun[1] * normal_y[index]);
            let facing = sun[2].mul_add(normal_z[index], facing);
            x86::maxss(facing, 0.0)
        })
        .collect()
}

/// `v · (1 / √(v·v))`: how the native float3 helpers normalise.
fn normalize(vector: Vector3) -> Vector3 {
    let length_squared = vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2];
    let inverse = 1.0 / length_squared.sqrt();
    scale(vector, inverse)
}

/// One soft-shadow sample: the direction to a point on the sun's disk and
/// where its lines start inside their first cell.
pub struct ShadowSample {
    pub direction: Vector3,
    pub offset: (f32, f32),
}

/// The offsets of `ShadowQuality` −4 (the 4× multisampling pattern).
const PATTERN_4: [(f32, f32); 4] = [(0.625, 0.125), (0.125, 0.375), (0.875, 0.625), (0.375, 0.875)];

/// The offsets of `ShadowQuality` −8 (the 8× multisampling pattern).
const PATTERN_8: [(f32, f32); 8] = [
    (0.0625, 0.3125),
    (0.1875, 0.5625),
    (0.3125, 0.1875),
    (0.4375, 0.9375),
    (0.5625, 0.0625),
    (0.6875, 0.8125),
    (0.8125, 0.4375),
    (0.9375, 0.6875),
];

/// The soft-shadow samples and how many the shadow is
/// averaged over. Qualities −8, −4 and −1 sample the sun's centre at
/// fixed offsets. A quality `q` ≥ 0 adds `3q(q + 1)` samples on `q` rings
/// around the centre one, jittered along each ring.
pub fn shadow_samples(sun: Vector3, quality: i32, angular_diameter: f32) -> (usize, Vec<ShadowSample>) {
    let at_centre = |offset: &(f32, f32)| ShadowSample { direction: sun, offset: *offset };
    match quality {
        -8 => return (8, PATTERN_8.iter().map(at_centre).collect()),
        -4 => return (4, PATTERN_4.iter().map(at_centre).collect()),
        -1 => return (1, vec![at_centre(&(0.5, 0.5))]),
        _ => {}
    }
    let rings = quality;
    // Other negative qualities would average over `-quality` samples but
    // only fill in the centre one.
    let count = if rings < 0 { -rings } else { (rings + 1) * rings * 3 + 1 };
    let mut samples = vec![at_centre(&(0.5, 0.5))];

    // How far the centre direction reaches against a ring of unit radius.
    let spread = if angular_diameter == 0.0 { 1e19 } else { maths::tan((180.0 - angular_diameter) * 0.008_726_646) };
    let [sun_x, sun_y, sun_z] = sun;
    let side = normalize([sun_y - sun_z * 0.0, sun_z * 0.0 - sun_x, sun_x * 0.0 - sun_y * 0.0]);
    let up = normalize([side[2] * sun_y - side[1] * sun_z, side[0] * sun_z - side[2] * sun_x, side[1] * sun_x - side[0] * sun_y]);
    let centre = scale(sun, spread);
    let ring_count = rings as f32;

    let mut state: u32 = 0xA235_1342;
    for ring in 1..=rings {
        let radius = ring as f32;
        let across = [radius * side[0], radius * side[1], radius * side[2]];
        let along = [radius * up[0], radius * up[1], up[2] * radius];
        for step in 0..6 * ring {
            state = lighting::xorshift(state);
            let jitter = lighting::unit(state) * 0.6;
            let angle = (jitter + step as f32) / radius * std::f32::consts::FRAC_PI_3;
            let (sine, cosine) = maths::sin_cos(angle);

            // The start offset: the ring position pushed out to a square,
            // rotated by atan(1/2), within the cell.
            let largest = x86::maxss(sine.abs(), cosine.abs());
            let square = (cosine / largest, sine / largest);
            let rotated_x = square.0 * 0.8944272 - square.1 * 0.4472136;
            let rotated_y = square.1 * 0.8944272 + square.0 * 0.4472136;
            let offset = (rotated_x * 0.5 + 0.5, rotated_y * 0.5 + 0.5);

            let direction_x = (across[0] * cosine) / ring_count + (along[0] * sine) / ring_count + centre[0];
            let direction_y = (along[1] * sine) / ring_count + (cosine * across[1]) / ring_count + centre[1];
            let direction_z = (across[2] * cosine) / ring_count + (along[2] * sine) / ring_count + centre[2];
            let direction = normalize([direction_x, direction_y, direction_z]);
            samples.push(ShadowSample { direction, offset });
        }
    }
    (count as usize, samples)
}

/// The bilinear height a soft-shadow line tests at each step: like
/// [`lighting::sample_bilinear`], but with the top row's products fused
/// the other way round. That is how the native kernel computes it when all
/// eight lanes are active, which is every line for maps whose size is a
/// multiple of eight; its tail path for partial groups matches
/// `sample_bilinear` instead.
fn shadow_height(heights: &[f32], width: i32, x: f32, y: f32) -> f32 {
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
    let top = at(x1, y0).mul_add(fx, at(x0, y0) * (1.0 - fx));
    let bottom = (1.0 - fx).mul_add(at(x0, y1), fx * at(x1, y1));
    (1.0 - fy).mul_add(top, fy * bottom)
}

/// One soft-shadow line of sight: cells whose
/// interpolated height rises above the line count as lit and raise it.
/// After wrapping across a map edge the line restarts from the height just
/// behind it.
#[allow(clippy::too_many_arguments)]
fn trace_shadow(heights: &[f32], width: i32, mut x: i32, mut y: i32, step_x: i32, step_y: i32, drop: f32, lit: &[AtomicI32]) {
    let wrap = width << 16;
    let height_behind = |x: i32, y: i32| {
        let behind_x = x.wrapping_sub(step_x) as f32 * (1.0 / FIXED_SCALE) + -0.5;
        let behind_y = y.wrapping_sub(step_y) as f32 * (1.0 / FIXED_SCALE) + -0.5;
        lighting::sample_bilinear(heights, width, behind_x, behind_y)
    };
    let mut ray = height_behind(x, y);
    for _ in 0..width {
        let mut wrapped = false;
        if y < 0 {
            y = y.wrapping_add(wrap);
            wrapped = true;
        }
        if y >= wrap {
            y = y.wrapping_sub(wrap);
            wrapped = true;
        }
        if x < 0 {
            x = x.wrapping_add(wrap);
            wrapped = true;
        }
        if x >= wrap {
            x = x.wrapping_sub(wrap);
            wrapped = true;
        }
        ray += drop;
        if wrapped {
            ray = height_behind(x, y) + drop;
        }
        let position_x = x as f32 * (1.0 / FIXED_SCALE) + -0.5;
        let position_y = y as f32 * (1.0 / FIXED_SCALE) + -0.5;
        let height = shadow_height(heights, width, position_x, position_y);
        if ray < height {
            let index = (y >> 16) * width + (x >> 16);
            lit[index as usize].fetch_add(1, Ordering::Relaxed);
            ray = height;
        }
        x = x.wrapping_add(step_x);
        y = y.wrapping_add(step_y);
    }
}

/// The cells one soft-shadow sample lights, counted per cell (one sample's
/// share of [`soft_shadows`], for checking against the native sweep kernel).
pub fn shadow_sample_counts(heights: &[f32], width: usize, sample: &ShadowSample, vertical_scale: f32) -> Vec<i32> {
    let lit: Vec<AtomicI32> = heights.iter().map(|_| AtomicI32::new(0)).collect();
    lighting::for_each_line(width, sample.direction, sample.offset, vertical_scale, |x, y, step_x, step_y, drop| {
        trace_shadow(heights, width as i32, x, y, step_x, step_y, drop, &lit);
    });
    lit.into_iter().map(AtomicI32::into_inner).collect()
}

/// The share of the sun's disk each cell sees: every
/// sample sweeps its lines and counts the cells it reaches.
fn soft_shadows(heights: &[f32], width: usize, sun: Vector3, rig: &LightRig, terrain: (f32, f32)) -> Vec<f32> {
    let (count, samples) = shadow_samples(sun, rig.shadow_quality, rig.sun_angular_diameter);
    let vertical_scale = terrain.0 / (width as f32 * terrain.1);
    let lit: Vec<AtomicI32> = heights.iter().map(|_| AtomicI32::new(0)).collect();
    samples.par_iter().for_each(|sample| {
        lighting::for_each_line(width, sample.direction, sample.offset, vertical_scale, |x, y, step_x, step_y, drop| {
            trace_shadow(heights, width as i32, x, y, step_x, step_y, drop, &lit);
        });
    });
    let weight = 1.0 / count as f32;
    lit.into_iter().map(|cell| weight * cell.into_inner() as f32).collect()
}

/// The 12 vertices of the icosahedron the occlusion directions start from.
const ICOSAHEDRON: [Vector3; 12] = [
    [1.0, 0.0, 0.0],
    [0.447_213_6, 0.0, 0.894_427_2],
    [0.447_213_6, 0.850_650_8, 0.276_393_2],
    [0.447_213_6, 0.525_731_1, -0.723_606_8],
    [0.447_213_6, -0.525_731_1, -0.723_606_8],
    [0.447_213_6, -0.850_650_8, 0.276_393_2],
    [-0.447_213_6, 0.0, -0.894_427_2],
    [-0.447_213_6, -0.850_650_8, -0.276_393_2],
    [-0.447_213_6, -0.525_731_1, 0.723_606_8],
    [-0.447_213_6, 0.525_731_1, 0.723_606_8],
    [-0.447_213_6, 0.850_650_8, -0.276_393_2],
    [-1.0, 0.0, 0.0],
];

/// The icosahedron's 30 edges.
const ICOSAHEDRON_EDGES: [(usize, usize); 30] = [
    (0, 1), (0, 2), (0, 3), (0, 4), (0, 5), (1, 2), (2, 3), (3, 4), (4, 5), (5, 1),
    (1, 9), (9, 2), (2, 10), (10, 3), (3, 6), (6, 4), (4, 7), (7, 5), (5, 8), (8, 1),
    (8, 9), (9, 10), (10, 6), (6, 7), (7, 8), (6, 11), (9, 11), (10, 11), (7, 11), (8, 11),
];

/// The icosahedron's 20 faces as triples of edges.
const ICOSAHEDRON_FACES: [(usize, usize, usize); 20] = [
    (0, 1, 5), (1, 2, 6), (2, 3, 7), (3, 4, 8), (0, 4, 9), (9, 18, 19), (10, 19, 20), (5, 11, 10), (6, 12, 13), (13, 14, 22),
    (7, 14, 15), (15, 16, 23), (8, 16, 17), (29, 24, 28), (20, 29, 26), (21, 26, 27), (22, 25, 27), (23, 25, 28), (11, 12, 21), (17, 18, 24),
];

/// The three corners of an icosahedron face given by its edges.
fn face_corners(edges: (usize, usize, usize)) -> [usize; 3] {
    let mut corners: Vec<usize> = Vec::with_capacity(3);
    for edge in [edges.0, edges.1, edges.2] {
        let (a, b) = ICOSAHEDRON_EDGES[edge];
        for vertex in [a, b] {
            if !corners.contains(&vertex) {
                corners.push(vertex);
            }
        }
    }
    [corners[0], corners[1], corners[2]]
}

/// The vertices of the icosahedron subdivided `levels` times, every edge
/// split at its normalised midpoint. The sweeps only
/// depend on the set of directions, not their order.
fn icosphere(levels: i32) -> Vec<Vector3> {
    let mut vertices = ICOSAHEDRON.to_vec();
    let mut faces: Vec<[usize; 3]> = ICOSAHEDRON_FACES.iter().map(|&edges| face_corners(edges)).collect();
    for _ in 0..levels {
        let mut midpoints: HashMap<(usize, usize), usize> = HashMap::new();
        let mut midpoint = |a: usize, b: usize, vertices: &mut Vec<Vector3>| -> usize {
            let key = (a.min(b), a.max(b));
            *midpoints.entry(key).or_insert_with(|| {
                let sum = dotnet_vector::add(vertices[key.0], vertices[key.1]);
                vertices.push(normalize(sum));
                vertices.len() - 1
            })
        };
        let mut next_faces = Vec::with_capacity(faces.len() * 4);
        for face in &faces {
            let ab = midpoint(face[0], face[1], &mut vertices);
            let bc = midpoint(face[1], face[2], &mut vertices);
            let ca = midpoint(face[2], face[0], &mut vertices);
            next_faces.push([face[0], ab, ca]);
            next_faces.push([face[1], bc, ab]);
            next_faces.push([face[2], ca, bc]);
            next_faces.push([ab, bc, ca]);
        }
        faces = next_faces;
    }
    vertices
}

/// The directions the occlusion sweeps, each with unit
/// intensity: icosphere vertices above −0.15 that do not point straight
/// up or down. Also returns the vertex count the light is averaged over.
pub(crate) fn occlusion_directions(levels: i32) -> (usize, Vec<([f32; 3], f32)>) {
    let vertices = icosphere(levels);
    let count = vertices.len();
    let kept = vertices
        .into_iter()
        .filter(|vertex| -0.15 < vertex[2] && (vertex[0] != 0.0 || vertex[1] != 0.0))
        .map(|vertex| (vertex, 1.0))
        .collect();
    (count, kept)
}

/// Occlusion from summed 16.16 light: clamped at zero and averaged over
/// every direction of the sphere.
fn light_to_occlusion(light: &[i32], direction_count: usize) -> Vec<f32> {
    let unit = 1.0 / (((direction_count as i32) << 16) as f32);
    light.par_iter().map(|&value| value.max(0) as f32 * unit).collect()
}

/// Raytraced occlusion: lines of sight from every
/// direction of an icosphere subdivided `quality` times.
fn raytraced_occlusion(heights: &[f32], normals: &[Vec<f32>; 3], width: usize, quality: i32, terrain: (f32, f32)) -> Vec<f32> {
    let (count, directions) = occlusion_directions(quality);
    let vertical_scale = terrain.0 / (width as f32 * terrain.1);
    let light = lighting::sweep_light(heights, normals, width, &directions, vertical_scale);
    light_to_occlusion(&light, count)
}

/// Hybrid occlusion: raytraced on the coarsest level of
/// a pyramid of halvings, upsampled back to full size, then corrected for
/// the relief the coarse levels lost.
fn hybrid_occlusion(heights: &[f32], normal_z: &[f32], width: usize, rig: &LightRig, terrain: (f32, f32)) -> Vec<f32> {
    let (count, directions) = occlusion_directions(rig.hybrid_ao_quality);

    let mut level_widths = vec![width];
    let mut coarse_levels: Vec<Vec<f32>> = Vec::new();
    for _ in 1..rig.hybrid_ao_octaves {
        let parent_width = *level_widths.last().expect("the pyramid has a level");
        let parent = coarse_levels.last().map_or(heights, Vec::as_slice);
        let coarse = lighting::downsample(parent, parent_width, parent_width);
        coarse_levels.push(coarse);
        level_widths.push(parent_width >> 1);
    }

    let coarsest = coarse_levels.last().map_or(heights, Vec::as_slice);
    let coarsest_width = *level_widths.last().expect("the pyramid has a level");
    let coarse_normals = lighting::normals(coarsest, coarsest_width, coarsest_width, terrain);
    let vertical_scale = terrain.0 / (coarsest_width as f32 * terrain.1);
    let light = lighting::sweep_light(coarsest, &coarse_normals, coarsest_width, &directions, vertical_scale);
    let mut occlusion = light_to_occlusion(&light, count);
    let mut smoothed = coarsest.to_vec();

    for level in (0..level_widths.len() - 1).rev() {
        let fine_width = level_widths[level];
        let coarse_width = level_widths[level + 1];
        occlusion = lighting::upsample(&occlusion, coarse_width, coarse_width, fine_width, fine_width);
        smoothed = lighting::upsample(&smoothed, coarse_width, coarse_width, fine_width, fine_width);
    }
    if level_widths.len() > 1 {
        let relief = Relief {
            heights,
            smoothed: &smoothed,
            normal_z,
            width,
            coarse_width: level_widths[1],
            octaves: rig.hybrid_ao_octaves,
        };
        relief.correct(&mut occlusion, terrain);
    }
    occlusion
}

/// What the hybrid occlusion's full-resolution correction reads.
struct Relief<'a> {
    heights: &'a [f32],
    /// The coarsest heights upsampled back to full size.
    smoothed: &'a [f32],
    normal_z: &'a [f32],
    width: usize,
    /// The size of the first coarse level.
    coarse_width: usize,
    octaves: i32,
}

impl Relief<'_> {
    /// Brightens cells that stand above the smoothed
    /// terrain or on convex ground and darkens those below it, in hollows
    /// or on steep slopes, more so the more octaves the pyramid had.
    fn correct(&self, occlusion: &mut [f32], terrain: (f32, f32)) {
        let width = self.width;
        let height = self.heights.len() / width;
        let vertical_scale = terrain.0 / (self.coarse_width as f32 * terrain.1);
        let relief_scale = 0.04 / vertical_scale;
        let curvature_scale = 1.0 / vertical_scale;
        let octave_weight = (self.octaves - 1) as f32 * 0.3;
        let weight = if 0.0 > octave_weight { 0.0 } else { octave_weight };

        occlusion.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
            let up = y.saturating_sub(1);
            let down = (y + 1).min(height - 1);
            for (x, value) in row.iter_mut().enumerate() {
                let left = x.saturating_sub(1);
                let right = (x + 1).min(width - 1);
                let at = |c: usize, r: usize| self.heights[r * width + c];
                let index = y * width + x;

                let raised = (self.heights[index] - self.smoothed[index]) * relief_scale;
                let relief = raised * if raised > 0.0 { 1.7 } else { 1.0 };
                let cross = at(left, y) + at(x, up) + at(x, down) + at(right, y);
                let diagonal = at(left, up) + at(right, up) + at(left, down) + at(right, down);
                let laplacian = ((at(x, y) - cross * 0.16666667) - diagonal * 0.083333336) * curvature_scale;
                let curvature = x86::minss(0.5, x86::maxss(-0.5, laplacian)) * 0.7;
                let curvature_weight = if curvature > 0.0 { 0.5 } else { 1.5 };
                let shaped = curvature * weight * curvature_weight + relief;

                let tilt = (self.normal_z[index] * 0.7 - 0.7) * weight;
                let darkening = maths::exp(tilt + x86::minss(shaped, 0.0));
                let brightening = if shaped > 0.0 { shaped + 1.0 } else { 1.0 };
                let corrected = darkening * brightening * *value;
                *value = if corrected > 1.0 { 1.0 } else { corrected };
            }
        });
    }
}

/// `atan`'s polynomial coefficients in ISPC's standard library, after the
/// leading -0.0046721357.
const ATAN_COEFFICIENTS: [f32; 7] = [
    0.024_172_833,
    -0.059_366_5,
    0.099_050_835,
    -0.140_156_88,
    0.199_689_21,
    -0.333_319_16,
    0.999_999_9,
];

/// ISPC's `atan`: a polynomial on [0, 1], reflected through `1/x` above it.
fn ispc_atan(x: f32) -> f32 {
    let negated = -x;
    let magnitude = if negated > x { negated } else { x };
    let above_one = 1.0 < magnitude;
    let reduced = if above_one { 1.0 / magnitude } else { magnitude };
    let squared = reduced * reduced;
    let mut polynomial = -0.004_672_135_7_f32;
    for coefficient in ATAN_COEFFICIENTS {
        polynomial = polynomial.mul_add(squared, coefficient);
    }
    let mut result = reduced * polynomial;
    if above_one {
        result = 1.570_796_4 - result;
    }
    if x < 0.0 {
        result = -result;
    }
    result
}

/// Local occlusion: darker on steep slopes and in
/// hollows, from each cell's 3×3 neighbourhood alone.
fn local_occlusion(heights: &[f32], width: usize, height: usize, terrain: (f32, f32)) -> Vec<f32> {
    let slope_scale = (terrain.1 * 0.03125) / (terrain.0 / width as f32);
    let curvature_scale = slope_scale * 32.0;
    let mut occlusion = vec![0.0f32; width * height];
    occlusion.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
        let up = y.saturating_sub(1);
        let down = (y + 1).min(height - 1);
        for (x, value) in row.iter_mut().enumerate() {
            let left = x.saturating_sub(1);
            let right = (x + 1).min(width - 1);
            let at = |c: usize, r: usize| heights[r * width + c];
            let up_left = at(left, up);
            let up_middle = at(x, up);
            let up_right = at(right, up);
            let middle_left = at(left, y);
            let middle_right = at(right, y);
            let down_left = at(left, down);
            let down_middle = at(x, down);
            let down_right = at(right, down);

            // Scharr gradient and the slope it gives.
            let side_x = (up_right + down_right - up_left - down_left) * 3.0;
            let gradient_x = (middle_right - middle_left).mul_add(10.0, side_x);
            let side_y = (up_left + up_right - down_left - down_right) * 3.0;
            let gradient_y = (up_middle - down_middle).mul_add(10.0, side_y);
            let slope = gradient_y.mul_add(gradient_y, gradient_x * gradient_x).sqrt() * slope_scale;

            // A normalised Laplacian: the cell above its neighbours'
            // weighted average.
            let cross = up_middle + middle_left + down_middle + middle_right;
            let centre = cross.mul_add(-0.146_446_62, at(x, y));
            let diagonal = up_left + up_right + down_left + down_right;
            let laplacian = diagonal.mul_add(-0.103_553_39, centre) * curvature_scale;

            // Curvature squashed into −1..1 by 1 − 1/(1 + |c|), with ISPC's
            // refined reciprocal.
            let sign = if 0.0 < laplacian { 1.0 } else { -1.0 };
            let denominator = laplacian.mul_add(sign, 1.0);
            let estimate = x86::rcp(denominator);
            let correction = (-denominator).mul_add(estimate, 2.0);
            let squashed = (-correction).mul_add(estimate, 1.0);
            let curvature = sign * squashed;

            let tilt = (ispc_atan(slope) * -57.295_78).mul_add(0.011_111_111, 1.0);
            let shade = curvature.mul_add(0.25, 0.75) * tilt;
            *value = shade * (2.0 - shade);
        }
    });
    occlusion
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icosphere_vertex_counts() {
        for levels in 0..5 {
            let expected = 10 * (1 << (2 * levels)) + 2;
            assert_eq!(icosphere(levels).len(), expected);
        }
    }

    #[test]
    fn shadow_sample_counts() {
        let sun = normalize([0.3, -0.5, 0.8]);
        let (count, samples) = shadow_samples(sun, 5, 0.5);
        assert_eq!(count, 91);
        assert_eq!(samples.len(), 91);
        let (count, samples) = shadow_samples(sun, -8, 0.5);
        assert_eq!((count, samples.len()), (8, 8));
    }

    #[test]
    fn ispc_atan_is_close() {
        for step in -40..=40 {
            let x = step as f32 * 0.25;
            assert!((ispc_atan(x) - x.atan()).abs() < 1e-6);
        }
    }
}
