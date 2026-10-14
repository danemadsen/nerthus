//! Show Skybox: the sky behind the terrain and the fog over it. The sky is
//! Eric Bruneton's precomputed atmospheric scattering
//! (`BrunetonsImprovedAtmosphere.Demo`: an Earth atmosphere with ozone,
//! the sun's own spectrum, white balance and an exposure of 8, seen from
//! wherever the camera is, a Unity unit being a kilometre) and fogs the
//! terrain with Unity's exponential fog (`FogDensity` × 0.05 a Unity unit)
//! in the colour of the sky at the horizon. Off, the camera clears to the
//! viewport background.
//!
//! Nerthus works the same atmosphere out on the CPU: a transmittance
//! table, Hillaire's multiple-scattering table (for Bruneton's higher
//! scattering orders), then the sky by elevation and angle to the sun
//! from the camera's height, spread over a dome round the camera, with
//! the sun's disc in front of it. A thread does the work when the sun or
//! the camera's height changes.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::OnceLock;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::terrain::ViewportSettings;
use crate::ViewportCamera;

/// The planet and the top of its atmosphere (`kBottomRadius`,
/// `topRadius`), in metres.
const BOTTOM_RADIUS: f32 = 6_360_000.0;
const TOP_RADIUS: f32 = 6_420_000.0;
const RAYLEIGH_SCALE_HEIGHT: f32 = 8000.0;
const MIE_SCALE_HEIGHT: f32 = 1200.0;
const MIE_PHASE_G: f32 = 0.8;
/// `kSunAngularRadius`, radians.
const SUN_ANGULAR_RADIUS: f32 = 0.004675;
/// The scene's `Demo.Exposure` (without luminance).
const EXPOSURE: f32 = 8.0;
/// The ground the multiple scattering bounces off (`GroundAlbedo`), and
/// the ground shown below the horizon (Bruneton's demo's dark sea).
const MODEL_GROUND_ALBEDO: f32 = 0.1;
const SHOWN_GROUND_ALBEDO: [f32; 3] = [0.0, 0.0, 0.04];
/// The wavelengths rendered without luminance (`kLambdaR`, `G`, `B`), nm.
const WAVELENGTHS: [f32; 3] = [680.0, 550.0, 440.0];
/// A Nerthus unit is 20 Unity units, a Unity unit a kilometre.
const METRES_PER_UNIT: f32 = 20_000.0;
/// The camera's height is kept within these, metres.
const LOWEST_CAMERA: f32 = 10.0;
const HIGHEST_CAMERA: f32 = 50_000.0;

/// The solar spectrum Bruneton's demo uses (W/m²/nm, 360 to 830 nm every
/// 10 nm).
const SOLAR_IRRADIANCE: [f32; 48] = [
    1.11776, 1.14259, 1.01249, 1.14716, 1.72765, 1.73054, 1.6887, 1.61253, 1.91198, 2.03474, 2.02042, 2.02212, 1.93377, 1.95809, 1.91686, 1.8298, 1.8685, 1.8931, 1.85149, 1.8504, 1.8341, 1.8345, 1.8147, 1.78158,
    1.7533, 1.6965, 1.68194, 1.64654, 1.6048, 1.52143, 1.55622, 1.5113, 1.474, 1.4482, 1.41018, 1.36775, 1.34188, 1.31429, 1.28303, 1.26758, 1.2367, 1.2082, 1.18737, 1.14683, 1.12362, 1.1058, 1.07124, 1.04992,
];
/// Ozone's absorption cross section at the three wavelengths (m²), and
/// its peak number density (300 Dobson units over 15 km).
const OZONE_CROSS_SECTION: [f32; 3] = [1.209e-25, 3.5e-25, 1.582e-26];
const OZONE_PEAK_DENSITY: f32 = 300.0 * 2.687e20 / 15000.0;

/// The tables' sizes.
const TRANSMITTANCE_HEIGHTS: usize = 64;
const TRANSMITTANCE_ANGLES: usize = 256;
const MULTIPLE_SIZE: usize = 32;
const MULTIPLE_DIRECTIONS: usize = 64;
const SKY_ELEVATIONS: usize = 192;
const SKY_AZIMUTHS: usize = 48;
const RAY_STEPS: usize = 40;
/// The dome's texture (azimuth by elevation) and its mesh.
const DOME_WIDTH: usize = 1024;
const DOME_HEIGHT: usize = 512;
const DOME_RADIUS: f32 = 500.0;
const SUN_DISTANCE: f32 = 400.0;
const DOME_SEGMENTS: (usize, usize) = (96, 48);

type Rgb = [f32; 3];

/// The Earth atmosphere (`Demo.Awake`) at the three wavelengths.
struct Atmosphere {
    rayleigh: Rgb,
    mie_scattering: f32,
    mie_extinction: f32,
    ozone: Rgb,
    solar: Rgb,
    white_point: Rgb,
}

impl Atmosphere {
    fn earth() -> Self {
        let rayleigh = WAVELENGTHS.map(|lambda| 6.2031e-7 * (lambda / 1000.0).powi(-4));
        let mie_extinction = 0.005328 / MIE_SCALE_HEIGHT;
        let ozone = OZONE_CROSS_SECTION.map(|section| section * OZONE_PEAK_DENSITY);
        let solar = WAVELENGTHS.map(|lambda| SOLAR_IRRADIANCE[((lambda - 360.0) / 10.0) as usize]);
        Self { rayleigh, mie_scattering: 0.9 * mie_extinction, mie_extinction, ozone, solar, white_point: white_point() }
    }

    /// Extinction and the two scatterings at a height (metres).
    fn medium(&self, height: f32) -> (Rgb, Rgb, f32) {
        let rayleigh_density = (-height / RAYLEIGH_SCALE_HEIGHT).exp();
        let mie_density = (-height / MIE_SCALE_HEIGHT).exp();
        let ozone_density = if height < 25_000.0 { height / 15_000.0 - 2.0 / 3.0 } else { -height / 15_000.0 + 8.0 / 3.0 };
        let ozone_density = ozone_density.clamp(0.0, 1.0);
        let rayleigh = self.rayleigh.map(|beta| beta * rayleigh_density);
        let mie = self.mie_scattering * mie_density;
        let mut extinction = [0.0; 3];
        for channel in 0..3 {
            extinction[channel] = rayleigh[channel] + self.mie_extinction * mie_density + self.ozone[channel] * ozone_density;
        }
        (extinction, rayleigh, mie)
    }
}

/// `ConvertSpectrumToLinearSrgb` of the solar spectrum over its mean: the
/// sun's colour, which the sky is divided by (`DoWhiteBalance`). The CIE
/// 1931 matching functions are Wyman, Sloan and Shirley's fit.
fn white_point() -> Rgb {
    let lobe = |lambda: f32, centre: f32, below: f32, above: f32| {
        let spread = if lambda < centre { below } else { above };
        (-0.5 * ((lambda - centre) / spread).powi(2)).exp()
    };
    let mut xyz = [0.0f32; 3];
    for step in 0..470 {
        let lambda = 360.0 + step as f32;
        let index = ((lambda - 360.0) / 10.0) as usize;
        let fraction = (lambda - 360.0) / 10.0 - index as f32;
        let next = (index + 1).min(SOLAR_IRRADIANCE.len() - 1);
        let value = SOLAR_IRRADIANCE[index] * (1.0 - fraction) + SOLAR_IRRADIANCE[next] * fraction;
        let x = 1.056 * lobe(lambda, 599.8, 37.9, 31.0) + 0.362 * lobe(lambda, 442.0, 16.0, 26.7) - 0.065 * lobe(lambda, 501.1, 20.4, 26.2);
        let y = 0.821 * lobe(lambda, 568.8, 46.9, 40.5) + 0.286 * lobe(lambda, 530.9, 16.3, 31.1);
        let z = 1.217 * lobe(lambda, 437.0, 11.8, 36.0) + 0.681 * lobe(lambda, 459.0, 26.0, 13.8);
        xyz[0] += value * x;
        xyz[1] += value * y;
        xyz[2] += value * z;
    }
    let red = 3.2406 * xyz[0] - 1.5372 * xyz[1] - 0.4986 * xyz[2];
    let green = -0.9689 * xyz[0] + 1.8758 * xyz[1] + 0.0415 * xyz[2];
    let blue = 0.0557 * xyz[0] - 0.204 * xyz[1] + 1.057 * xyz[2];
    let mean = (red + green + blue) / 3.0;
    [red / mean, green / mean, blue / mean]
}

/// Where a ray from radius `radius` at cosine `mu` (to the zenith) leaves
/// the atmosphere, and where it meets the ground if it does.
fn distance_to_top(radius: f32, mu: f32) -> f32 {
    let discriminant = radius * radius * (mu * mu - 1.0) + TOP_RADIUS * TOP_RADIUS;
    (-radius * mu + discriminant.max(0.0).sqrt()).max(0.0)
}

fn distance_to_ground(radius: f32, mu: f32) -> Option<f32> {
    let discriminant = radius * radius * (mu * mu - 1.0) + BOTTOM_RADIUS * BOTTOM_RADIUS;
    (mu < 0.0 && discriminant >= 0.0).then(|| (-radius * mu - discriminant.sqrt()).max(0.0))
}

fn add(left: Rgb, right: Rgb) -> Rgb {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn scale(colour: Rgb, factor: f32) -> Rgb {
    colour.map(|channel| channel * factor)
}

fn multiply(left: Rgb, right: Rgb) -> Rgb {
    [left[0] * right[0], left[1] * right[1], left[2] * right[2]]
}

/// Rayleigh's phase function, and Cornette and Shanks' for Mie.
fn rayleigh_phase(nu: f32) -> f32 {
    3.0 / (16.0 * std::f32::consts::PI) * (1.0 + nu * nu)
}

fn mie_phase(nu: f32) -> f32 {
    let g = MIE_PHASE_G;
    let k = 3.0 / (8.0 * std::f32::consts::PI) * (1.0 - g * g) / (2.0 + g * g);
    k * (1.0 + nu * nu) / (1.0 + g * g - 2.0 * g * nu).max(1e-6).powf(1.5)
}

/// The tables that do not depend on the sun or the camera.
struct Tables {
    atmosphere: Atmosphere,
    transmittance: Vec<Rgb>,
    multiple: Vec<Rgb>,
    /// The sky's light on the ground by the sun's cosine, for the ground
    /// shown below the horizon.
    ground_irradiance: Vec<Rgb>,
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(Tables::new)
}

impl Tables {
    fn new() -> Self {
        let mut tables = Tables {
            atmosphere: Atmosphere::earth(),
            transmittance: Vec::new(),
            multiple: vec![[0.0; 3]; MULTIPLE_SIZE * MULTIPLE_SIZE],
            ground_irradiance: vec![[0.0; 3]; MULTIPLE_SIZE],
        };
        tables.transmittance = tables.build_transmittance();
        tables.multiple = tables.build_multiple();
        tables.ground_irradiance = tables.build_ground_irradiance();
        tables
    }

    /// Transmittance to the top of the atmosphere, laid out as Bruneton
    /// lays it out (height by distance to the top).
    fn build_transmittance(&self) -> Vec<Rgb> {
        let mut table = Vec::with_capacity(TRANSMITTANCE_HEIGHTS * TRANSMITTANCE_ANGLES);
        for row in 0..TRANSMITTANCE_HEIGHTS {
            for column in 0..TRANSMITTANCE_ANGLES {
                let x_mu = column as f32 / (TRANSMITTANCE_ANGLES - 1) as f32;
                let x_r = row as f32 / (TRANSMITTANCE_HEIGHTS - 1) as f32;
                let (radius, mu) = transmittance_parameters(x_mu, x_r);
                table.push(self.integrate_transmittance(radius, mu));
            }
        }
        table
    }

    fn integrate_transmittance(&self, radius: f32, mu: f32) -> Rgb {
        let length = distance_to_top(radius, mu);
        let step = length / RAY_STEPS as f32;
        let mut depth = [0.0f32; 3];
        for sample in 0..RAY_STEPS {
            let distance = (sample as f32 + 0.5) * step;
            let height = (distance * distance + 2.0 * radius * mu * distance + radius * radius).sqrt() - BOTTOM_RADIUS;
            let (extinction, _, _) = self.atmosphere.medium(height);
            depth = add(depth, scale(extinction, step));
        }
        depth.map(|value| (-value).exp())
    }

    /// Transmittance from `radius` to the top, looking at cosine `mu`.
    fn transmittance(&self, radius: f32, mu: f32) -> Rgb {
        let (x_mu, x_r) = transmittance_coordinates(radius, mu);
        let column = x_mu * (TRANSMITTANCE_ANGLES - 1) as f32;
        let row = x_r * (TRANSMITTANCE_HEIGHTS - 1) as f32;
        bilinear(&self.transmittance, TRANSMITTANCE_ANGLES, TRANSMITTANCE_HEIGHTS, column, row)
    }

    /// Transmittance to the sun, faded as it sets behind the horizon
    /// (`GetTransmittanceToSun`).
    fn transmittance_to_sun(&self, radius: f32, mu_sun: f32) -> Rgb {
        let sin_horizon = BOTTOM_RADIUS / radius;
        let cos_horizon = -(1.0 - sin_horizon * sin_horizon).max(0.0).sqrt();
        let edge = sin_horizon * SUN_ANGULAR_RADIUS;
        let t = ((mu_sun - cos_horizon + edge) / (2.0 * edge)).clamp(0.0, 1.0);
        let visible = t * t * (3.0 - 2.0 * t);
        scale(self.transmittance(radius, mu_sun), visible)
    }

    /// Hillaire's multiple scattering table: by height and the sun's
    /// cosine, the light scattered twice or more, isotropic, per unit of
    /// scattering.
    fn build_multiple(&self) -> Vec<Rgb> {
        let directions = sphere_directions(MULTIPLE_DIRECTIONS);
        let mut table = Vec::with_capacity(MULTIPLE_SIZE * MULTIPLE_SIZE);
        let isotropic = 1.0 / (4.0 * std::f32::consts::PI);
        for row in 0..MULTIPLE_SIZE {
            let height = (row as f32 + 0.5) / MULTIPLE_SIZE as f32 * (TOP_RADIUS - BOTTOM_RADIUS);
            let radius = BOTTOM_RADIUS + height;
            for column in 0..MULTIPLE_SIZE {
                let mu_sun = -1.0 + 2.0 * (column as f32 + 0.5) / MULTIPLE_SIZE as f32;
                let sun = Vec3::new((1.0 - mu_sun * mu_sun).max(0.0).sqrt(), mu_sun, 0.0);
                let mut second = [0.0f32; 3];
                let mut transfer = [0.0f32; 3];
                for direction in &directions {
                    let (light, fraction) = self.second_order(radius, *direction, sun, isotropic);
                    second = add(second, light);
                    transfer = add(transfer, fraction);
                }
                let count = directions.len() as f32;
                let mut entry = [0.0; 3];
                for channel in 0..3 {
                    let mean_second = second[channel] / count;
                    let mean_transfer = (transfer[channel] / count).min(0.99);
                    entry[channel] = mean_second / (1.0 - mean_transfer);
                }
                table.push(entry);
            }
        }
        table
    }

    /// Along one direction from height `radius`: the light scattered once
    /// towards the point (isotropically), the ground's too, and how much
    /// of any light scattering gives back.
    fn second_order(&self, radius: f32, direction: Vec3, sun: Vec3, isotropic: f32) -> (Rgb, Rgb) {
        let mu = direction.y;
        let ground = distance_to_ground(radius, mu);
        let length = ground.unwrap_or_else(|| distance_to_top(radius, mu));
        let steps = RAY_STEPS / 2;
        let step = length / steps as f32;
        let origin = Vec3::new(0.0, radius, 0.0);
        let mut light = [0.0f32; 3];
        let mut transfer = [0.0f32; 3];
        let mut transmittance = [1.0f32; 3];
        for sample in 0..steps {
            let point = origin + direction * ((sample as f32 + 0.5) * step);
            let point_radius = point.length();
            let (extinction, rayleigh, mie) = self.atmosphere.medium(point_radius - BOTTOM_RADIUS);
            let to_sun = self.transmittance_to_sun(point_radius, point.dot(sun) / point_radius);
            for channel in 0..3 {
                let scattering = rayleigh[channel] + mie;
                let segment = (-extinction[channel] * step).exp();
                let integral = transmittance[channel] * (1.0 - segment) / extinction[channel].max(1e-12);
                light[channel] += integral * scattering * self.atmosphere.solar[channel] * to_sun[channel] * isotropic;
                transfer[channel] += integral * scattering;
                transmittance[channel] *= segment;
            }
        }
        if ground.is_some() {
            let point = origin + direction * length;
            let normal = point.normalize();
            let cos_sun = normal.dot(sun);
            let to_sun = self.transmittance_to_sun(BOTTOM_RADIUS, cos_sun);
            for channel in 0..3 {
                let irradiance = self.atmosphere.solar[channel] * to_sun[channel] * cos_sun.max(0.0);
                light[channel] += transmittance[channel] * irradiance * MODEL_GROUND_ALBEDO / std::f32::consts::PI;
            }
        }
        (light, transfer)
    }

    fn multiple_scattering(&self, radius: f32, mu_sun: f32) -> Rgb {
        let row = ((radius - BOTTOM_RADIUS) / (TOP_RADIUS - BOTTOM_RADIUS) * MULTIPLE_SIZE as f32 - 0.5).clamp(0.0, (MULTIPLE_SIZE - 1) as f32);
        let column = ((mu_sun + 1.0) / 2.0 * MULTIPLE_SIZE as f32 - 0.5).clamp(0.0, (MULTIPLE_SIZE - 1) as f32);
        bilinear(&self.multiple, MULTIPLE_SIZE, MULTIPLE_SIZE, column, row)
    }

    /// The sky's light falling on the ground, by the sun's cosine.
    fn build_ground_irradiance(&self) -> Vec<Rgb> {
        let directions: Vec<Vec3> = sphere_directions(64).into_iter().filter(|direction| direction.y > 0.0).collect();
        let mut table = Vec::with_capacity(MULTIPLE_SIZE);
        for column in 0..MULTIPLE_SIZE {
            let mu_sun = -1.0 + 2.0 * (column as f32 + 0.5) / MULTIPLE_SIZE as f32;
            let sun = Vec3::new((1.0 - mu_sun * mu_sun).max(0.0).sqrt(), mu_sun, 0.0);
            let mut irradiance = [0.0f32; 3];
            for direction in &directions {
                let (radiance, _) = self.sky_radiance(BOTTOM_RADIUS + 1.0, *direction, sun, false);
                irradiance = add(irradiance, scale(radiance, direction.y));
            }
            // The hemisphere's solid angle over the directions taken.
            table.push(scale(irradiance, 2.0 * std::f32::consts::PI / directions.len() as f32));
        }
        table
    }

    fn ground_irradiance(&self, mu_sun: f32) -> Rgb {
        let column = ((mu_sun + 1.0) / 2.0 * MULTIPLE_SIZE as f32 - 0.5).clamp(0.0, (MULTIPLE_SIZE - 1) as f32);
        bilinear(&self.ground_irradiance, MULTIPLE_SIZE, 1, column, 0.0)
    }

    /// The light reaching the camera at `radius` along `view` with the sun
    /// towards `sun` (`GetSkyRadiance`, or the ground's when the view
    /// meets it), and the transmittance along the view.
    fn sky_radiance(&self, radius: f32, view: Vec3, sun: Vec3, show_ground: bool) -> (Rgb, Rgb) {
        let mu = view.y;
        let ground = distance_to_ground(radius, mu);
        let length = ground.unwrap_or_else(|| distance_to_top(radius, mu));
        let step = length / RAY_STEPS as f32;
        let origin = Vec3::new(0.0, radius, 0.0);
        let nu = view.dot(sun);
        let rayleigh_phase = rayleigh_phase(nu);
        let mie_phase = mie_phase(nu);
        let mut radiance = [0.0f32; 3];
        let mut transmittance = [1.0f32; 3];
        for sample in 0..RAY_STEPS {
            let point = origin + view * ((sample as f32 + 0.5) * step);
            let point_radius = point.length();
            let mu_sun = point.dot(sun) / point_radius;
            let (extinction, rayleigh, mie) = self.atmosphere.medium(point_radius - BOTTOM_RADIUS);
            let to_sun = self.transmittance_to_sun(point_radius, mu_sun);
            let multiple = self.multiple_scattering(point_radius, mu_sun);
            for channel in 0..3 {
                let single = self.atmosphere.solar[channel] * to_sun[channel] * (rayleigh[channel] * rayleigh_phase + mie * mie_phase);
                let source = single + (rayleigh[channel] + mie) * multiple[channel];
                let segment = (-extinction[channel] * step).exp();
                radiance[channel] += transmittance[channel] * source * (1.0 - segment) / extinction[channel].max(1e-12);
                transmittance[channel] *= segment;
            }
        }
        if show_ground && ground.is_some() {
            let point = origin + view * length;
            let cos_sun = point.normalize().dot(sun);
            let to_sun = self.transmittance_to_sun(BOTTOM_RADIUS, cos_sun);
            let sky = self.ground_irradiance(cos_sun);
            for channel in 0..3 {
                let irradiance = self.atmosphere.solar[channel] * to_sun[channel] * cos_sun.max(0.0) + sky[channel];
                radiance[channel] += transmittance[channel] * irradiance * SHOWN_GROUND_ALBEDO[channel] / std::f32::consts::PI;
            }
        }
        (radiance, transmittance)
    }

    /// The shader's last step: `pow(1 - exp(-radiance / white_point *
    /// exposure), 1 / 2.2)`.
    fn tone_map(&self, radiance: Rgb) -> Rgb {
        let mut colour = [0.0; 3];
        for channel in 0..3 {
            let exposed = radiance[channel] / self.atmosphere.white_point[channel] * EXPOSURE;
            colour[channel] = (1.0 - (-exposed).exp()).max(0.0).powf(1.0 / 2.2);
        }
        colour
    }
}

/// Bruneton's transmittance table coordinates from radius and cosine.
fn transmittance_coordinates(radius: f32, mu: f32) -> (f32, f32) {
    let horizon = (TOP_RADIUS * TOP_RADIUS - BOTTOM_RADIUS * BOTTOM_RADIUS).sqrt();
    let rho = (radius * radius - BOTTOM_RADIUS * BOTTOM_RADIUS).max(0.0).sqrt();
    let distance = distance_to_top(radius, mu);
    let nearest = TOP_RADIUS - radius;
    let farthest = rho + horizon;
    let x_mu = ((distance - nearest) / (farthest - nearest).max(1.0)).clamp(0.0, 1.0);
    let x_r = (rho / horizon).clamp(0.0, 1.0);
    (x_mu, x_r)
}

/// The radius and cosine a transmittance table cell stands for.
fn transmittance_parameters(x_mu: f32, x_r: f32) -> (f32, f32) {
    let horizon = (TOP_RADIUS * TOP_RADIUS - BOTTOM_RADIUS * BOTTOM_RADIUS).sqrt();
    let rho = horizon * x_r;
    let radius = (rho * rho + BOTTOM_RADIUS * BOTTOM_RADIUS).sqrt();
    let nearest = TOP_RADIUS - radius;
    let farthest = rho + horizon;
    let distance = nearest + x_mu * (farthest - nearest);
    let mu = if distance <= 0.0 { 1.0 } else { ((horizon * horizon - rho * rho - distance * distance) / (2.0 * radius * distance)).clamp(-1.0, 1.0) };
    (radius, mu)
}

/// Evenly spread directions over the sphere (a Fibonacci spiral).
fn sphere_directions(count: usize) -> Vec<Vec3> {
    let golden = std::f32::consts::PI * (3.0 - 5.0f32.sqrt());
    (0..count)
        .map(|index| {
            let y = 1.0 - 2.0 * (index as f32 + 0.5) / count as f32;
            let ring = (1.0 - y * y).max(0.0).sqrt();
            let angle = golden * index as f32;
            Vec3::new(ring * angle.cos(), y, ring * angle.sin())
        })
        .collect()
}

fn bilinear(table: &[Rgb], width: usize, height: usize, column: f32, row: f32) -> Rgb {
    let column = column.clamp(0.0, (width - 1) as f32);
    let row = row.clamp(0.0, (height - 1) as f32);
    let left = (column.floor() as usize).min(width.saturating_sub(2));
    let top = (row.floor() as usize).min(height.saturating_sub(2));
    let right = (left + 1).min(width - 1);
    let bottom = (top + 1).min(height - 1);
    let across = column - left as f32;
    let down = row - top as f32;
    let mut value = [0.0; 3];
    for channel in 0..3 {
        let upper = table[top * width + left][channel] * (1.0 - across) + table[top * width + right][channel] * across;
        let lower = table[bottom * width + left][channel] * (1.0 - across) + table[bottom * width + right][channel] * across;
        value[channel] = upper * (1.0 - down) + lower * down;
    }
    value
}

/// What the sky is worked out for.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SkyRequest {
    sun_azimuth: f32,
    sun_elevation: f32,
    /// The camera's height in metres.
    altitude: f32,
}

/// A finished sky: the dome's texture (sRGB bytes), the fog's colour and
/// the sun's disc, if it is above the horizon.
struct SkyImage {
    request: SkyRequest,
    pixels: Vec<u8>,
    fog: Rgb,
    sun: Option<Rgb>,
}

/// The sky by elevation (closer together at the horizon) and angle to
/// the sun's azimuth, from the camera's height.
struct SkyTable {
    sun_elevation: f32,
    altitude: f32,
    /// The horizon's dip below level, radians.
    dip: f32,
    colours: Vec<Rgb>,
}

impl SkyTable {
    fn new(sun_elevation: f32, altitude: f32) -> Self {
        let tables = tables();
        let radius = BOTTOM_RADIUS + altitude;
        let dip = (BOTTOM_RADIUS / radius).clamp(-1.0, 1.0).acos();
        let elevation = sun_elevation.to_radians();
        let sun = Vec3::new(elevation.cos(), elevation.sin(), 0.0);
        let mut colours = Vec::with_capacity(SKY_ELEVATIONS * SKY_AZIMUTHS);
        for row in 0..SKY_ELEVATIONS {
            let view_elevation = table_elevation(row, dip);
            for column in 0..SKY_AZIMUTHS {
                let relative = column as f32 / (SKY_AZIMUTHS - 1) as f32 * std::f32::consts::PI;
                let view = Vec3::new(view_elevation.cos() * relative.cos(), view_elevation.sin(), view_elevation.cos() * relative.sin());
                let (radiance, _) = tables.sky_radiance(radius, view, sun, true);
                colours.push(tables.tone_map(radiance));
            }
        }
        Self { sun_elevation, altitude, dip, colours }
    }

    /// The colour looking at `elevation` (radians), `relative` (radians,
    /// 0 to π) round from the sun.
    fn colour(&self, elevation: f32, relative: f32) -> Rgb {
        let row = table_row(elevation, self.dip);
        let column = relative / std::f32::consts::PI * (SKY_AZIMUTHS - 1) as f32;
        bilinear(&self.colours, SKY_AZIMUTHS, SKY_ELEVATIONS, column, row)
    }
}

/// The elevation a table row stands for: rows bunch round the horizon
/// (Hillaire's sky-view mapping), which sits `dip` below level.
fn table_elevation(row: usize, dip: f32) -> f32 {
    let t = row as f32 / (SKY_ELEVATIONS - 1) as f32 * 2.0 - 1.0;
    let half_pi = std::f32::consts::FRAC_PI_2;
    if t >= 0.0 { -dip + (half_pi + dip) * t * t } else { -dip - (half_pi - dip) * t * t }
}

fn table_row(elevation: f32, dip: f32) -> f32 {
    let half_pi = std::f32::consts::FRAC_PI_2;
    let t = if elevation >= -dip { ((elevation + dip) / (half_pi + dip)).max(0.0).sqrt() } else { -((-dip - elevation) / (half_pi - dip)).max(0.0).sqrt() };
    (t + 1.0) / 2.0 * (SKY_ELEVATIONS - 1) as f32
}

/// The dome's texture, the fog's colour and the sun's disc for a request,
/// reusing the table when only the sun's azimuth changed.
fn build_sky(request: SkyRequest, table: &mut Option<SkyTable>) -> SkyImage {
    let reusable = table.as_ref().is_some_and(|built| built.sun_elevation == request.sun_elevation && built.altitude == request.altitude);
    if !reusable {
        *table = Some(SkyTable::new(request.sun_elevation, request.altitude));
    }
    let sky = table.as_ref().expect("the table was just built");
    let sun_azimuth = request.sun_azimuth.to_radians();
    let mut pixels = Vec::with_capacity(DOME_WIDTH * DOME_HEIGHT * 4);
    for y in 0..DOME_HEIGHT {
        let elevation = std::f32::consts::FRAC_PI_2 - (y as f32 + 0.5) / DOME_HEIGHT as f32 * std::f32::consts::PI;
        for x in 0..DOME_WIDTH {
            let azimuth = (x as f32 + 0.5) / DOME_WIDTH as f32 * std::f32::consts::TAU;
            let offset = (azimuth - sun_azimuth).rem_euclid(std::f32::consts::TAU);
            let relative = if offset > std::f32::consts::PI { std::f32::consts::TAU - offset } else { offset };
            let colour = sky.colour(elevation, relative);
            for channel in colour {
                pixels.push((channel.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            pixels.push(255);
        }
    }
    // The fog takes the sky just above the horizon, all the way round.
    let mut fog = [0.0; 3];
    let samples = 16;
    for step in 0..samples {
        let relative = (step as f32 + 0.5) / samples as f32 * std::f32::consts::PI;
        fog = add(fog, sky.colour(-sky.dip + 0.035, relative));
    }
    let fog = scale(fog, 1.0 / samples as f32);
    let sun = sun_colour(request, sky.dip);
    SkyImage { request, pixels, fog, sun }
}

/// The sun's disc as the camera sees it (`radiance + transmittance *
/// GetSolarRadiance()`), or none once it is below the horizon.
fn sun_colour(request: SkyRequest, dip: f32) -> Option<Rgb> {
    let elevation = request.sun_elevation.to_radians();
    if elevation < -dip - SUN_ANGULAR_RADIUS {
        return None;
    }
    let tables = tables();
    let radius = BOTTOM_RADIUS + request.altitude;
    let sun = Vec3::new(elevation.cos(), elevation.sin(), 0.0);
    let (radiance, transmittance) = tables.sky_radiance(radius, sun, sun, false);
    let solid_angle = std::f32::consts::PI * SUN_ANGULAR_RADIUS * SUN_ANGULAR_RADIUS;
    let solar = scale(tables.atmosphere.solar, 1.0 / solid_angle);
    let seen = add(radiance, multiply(transmittance, solar));
    Some(tables.tone_map(seen))
}

/// The thread that works skies out, the newest request first. The web
/// build has no threads: there the requests wait until `finished` works
/// out the newest.
#[derive(Resource)]
pub(crate) struct SkyWorker {
    requests: Sender<SkyRequest>,
    results: std::sync::Mutex<Receiver<SkyImage>>,
    #[cfg(target_arch = "wasm32")]
    waiting: std::sync::Mutex<(Receiver<SkyRequest>, Sender<SkyImage>, Option<SkyTable>)>,
}

impl Default for SkyWorker {
    fn default() -> Self {
        let (request_sender, request_receiver) = channel::<SkyRequest>();
        let (result_sender, result_receiver) = channel::<SkyImage>();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let spawned = thread::Builder::new().name("nerthus-sky".to_string()).spawn(move || {
                let mut table = None;
                while let Ok(request) = request_receiver.recv() {
                    if !build_newest(request, &request_receiver, &result_sender, &mut table) {
                        return;
                    }
                }
            });
            if let Err(error) = spawned {
                error!("cannot start the sky thread: {error}");
            }
        }
        Self {
            requests: request_sender,
            results: std::sync::Mutex::new(result_receiver),
            #[cfg(target_arch = "wasm32")]
            waiting: std::sync::Mutex::new((request_receiver, result_sender, None)),
        }
    }
}

impl SkyWorker {
    /// The newest sky finished since the last call.
    fn finished(&self) -> Option<SkyImage> {
        #[cfg(target_arch = "wasm32")]
        if let Ok(mut waiting) = self.waiting.lock() {
            let (requests, results, table) = &mut *waiting;
            if let Ok(request) = requests.try_recv() {
                build_newest(request, requests, results, table);
            }
        }
        self.results.lock().ok().and_then(|results| results.try_iter().last())
    }
}

/// Works out the sky for `request`, or for the newest of the requests that
/// came after it; false once nothing takes the skies.
fn build_newest(mut request: SkyRequest, requests: &Receiver<SkyRequest>, results: &Sender<SkyImage>, table: &mut Option<SkyTable>) -> bool {
    while let Ok(newer) = requests.try_recv() {
        request = newer;
    }
    let image = build_sky(request, table);
    results.send(image).is_ok()
}

/// The sky's state between frames.
#[derive(Resource, Default)]
pub(crate) struct SkyState {
    requested: Option<SkyRequest>,
    shown: Option<SkyRequest>,
    fog: Option<Rgb>,
    sun: Option<Rgb>,
}

#[derive(Component)]
pub(crate) struct SkyDome;

#[derive(Component)]
pub(crate) struct SunDisc;

pub(crate) fn spawn_sky(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) {
    let dome_material = StandardMaterial {
        base_color: Color::srgb(0.53, 0.63, 0.74),
        unlit: true,
        fog_enabled: false,
        cull_mode: None,
        ..default()
    };
    let dome = (SkyDome, Mesh3d(meshes.add(dome_mesh())), MeshMaterial3d(materials.add(dome_material)), Transform::default(), Visibility::Hidden, NotShadowCaster);
    commands.spawn(dome);
    let sun_material = StandardMaterial {
        base_color: Color::WHITE,
        unlit: true,
        fog_enabled: false,
        cull_mode: None,
        ..default()
    };
    let disc = Circle::new(1.0);
    let sun = (SunDisc, Mesh3d(meshes.add(disc)), MeshMaterial3d(materials.add(sun_material)), Transform::default(), Visibility::Hidden, NotShadowCaster);
    commands.spawn(sun);
}

/// A sphere round the origin seen from inside, its texture running round
/// from north (−Z) clockwise and down from the zenith.
fn dome_mesh() -> Mesh {
    let (around, down) = DOME_SEGMENTS;
    let mut positions = Vec::with_capacity((around + 1) * (down + 1));
    let mut normals = Vec::with_capacity((around + 1) * (down + 1));
    let mut uvs = Vec::with_capacity((around + 1) * (down + 1));
    for row in 0..=down {
        let v = row as f32 / down as f32;
        let elevation = std::f32::consts::FRAC_PI_2 - v * std::f32::consts::PI;
        for column in 0..=around {
            let u = column as f32 / around as f32;
            let azimuth = u * std::f32::consts::TAU;
            let direction = sky_direction(azimuth, elevation);
            positions.push((direction * DOME_RADIUS).to_array());
            normals.push((-direction).to_array());
            uvs.push([u, v]);
        }
    }
    let mut indices = Vec::with_capacity(around * down * 6);
    for row in 0..down {
        for column in 0..around {
            let corner = (row * (around + 1) + column) as u32;
            let below = corner + (around + 1) as u32;
            indices.extend_from_slice(&[corner, corner + 1, below, corner + 1, below + 1, below]);
        }
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// The direction at `azimuth` (clockwise from north, −Z) and `elevation`,
/// radians.
fn sky_direction(azimuth: f32, elevation: f32) -> Vec3 {
    Vec3::new(azimuth.sin() * elevation.cos(), elevation.sin(), -azimuth.cos() * elevation.cos())
}

/// The dome and the sun's disc as the sky moves them.
type DomeQuery<'a> = (&'a mut Transform, &'a mut Visibility, &'a MeshMaterial3d<StandardMaterial>);

/// Shows the sky and fog while Show Skybox is on: asks for a new sky when
/// the sun or the camera's height moved, takes in finished ones, keeps
/// the dome and the sun round the camera and the fog on it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_sky(
    mut commands: Commands,
    settings: Res<ViewportSettings>,
    worker: Res<SkyWorker>,
    mut state: ResMut<SkyState>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    camera: Single<(Entity, &GlobalTransform, Option<&DistanceFog>), With<ViewportCamera>>,
    mut dome: Single<DomeQuery, (With<SkyDome>, Without<SunDisc>)>,
    mut sun: Single<DomeQuery, (With<SunDisc>, Without<SkyDome>)>,
) {
    let (camera_entity, camera_transform, fog) = *camera;
    let eye = camera_transform.translation();
    if !settings.skybox {
        *dome.1 = Visibility::Hidden;
        *sun.1 = Visibility::Hidden;
        if fog.is_some() {
            commands.entity(camera_entity).remove::<DistanceFog>();
        }
        return;
    }

    // A new sky once the camera's height moves by a fiftieth.
    let altitude = (eye.y * METRES_PER_UNIT).clamp(LOWEST_CAMERA, HIGHEST_CAMERA);
    let steps = (altitude.ln() / 1.02f32.ln()).round();
    let altitude = 1.02f32.powf(steps);
    let request = SkyRequest { sun_azimuth: settings.sun_azimuth, sun_elevation: settings.sun_elevation, altitude };
    if state.requested != Some(request) {
        let _ = worker.requests.send(request);
        state.requested = Some(request);
    }
    let finished = worker.finished();
    if let Some(image) = finished {
        let extent = Extent3d { width: DOME_WIDTH as u32, height: DOME_HEIGHT as u32, depth_or_array_layers: 1 };
        let mut texture = Image::new(extent, TextureDimension::D2, image.pixels, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::default());
        let sampler = ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::ClampToEdge,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            ..default()
        };
        texture.sampler = ImageSampler::Descriptor(sampler);
        if let Some(mut material) = materials.get_mut(&dome.2.0) {
            material.base_color_texture = Some(images.add(texture));
            material.base_color = Color::WHITE;
        }
        if let (Some(colour), Some(mut material)) = (image.sun, materials.get_mut(&sun.2.0)) {
            material.base_color = Color::srgb(colour[0], colour[1], colour[2]);
        }
        state.shown = Some(image.request);
        state.fog = Some(image.fog);
        state.sun = image.sun;
    }

    *dome.1 = Visibility::Visible;
    dome.0.translation = eye;
    let elevation = settings.sun_elevation.to_radians();
    let direction = sky_direction(settings.sun_azimuth.to_radians(), elevation);
    let disc_centre = eye + direction * SUN_DISTANCE;
    *sun.1 = if state.sun.is_some() { Visibility::Visible } else { Visibility::Hidden };
    let disc_radius = SUN_DISTANCE * SUN_ANGULAR_RADIUS.tan();
    *sun.0 = Transform::from_translation(disc_centre).looking_at(eye, Vec3::Y).with_scale(Vec3::splat(disc_radius));

    // `SetFog(FogDensity * 0.1)`, halved, a Unity unit being a twentieth
    // of Nerthus's: `FogDensity` a unit.
    let colour = state.fog.unwrap_or([0.53, 0.63, 0.74]);
    let wanted = (settings.fog_density > 0.0).then(|| DistanceFog {
        color: Color::srgb(colour[0], colour[1], colour[2]),
        directional_light_color: Color::NONE,
        directional_light_exponent: 8.0,
        falloff: FogFalloff::Exponential { density: settings.fog_density },
    });
    match wanted {
        Some(wanted) => {
            let same = fog.is_some_and(|fog| fog.color == wanted.color && matches!(fog.falloff, FogFalloff::Exponential { density } if density == settings.fog_density));
            if !same {
                commands.entity(camera_entity).insert(wanted);
            }
        }
        None => {
            if fog.is_some() {
                commands.entity(camera_entity).remove::<DistanceFog>();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transmittance_coordinates_invert() {
        for (radius, mu) in [(BOTTOM_RADIUS + 1000.0, 0.3), (BOTTOM_RADIUS + 20_000.0, -0.05), (BOTTOM_RADIUS + 100.0, 0.95)] {
            let (x_mu, x_r) = transmittance_coordinates(radius, mu);
            let (back_radius, back_mu) = transmittance_parameters(x_mu, x_r);
            assert!((back_radius - radius).abs() < 1.0, "{radius} came back as {back_radius}");
            assert!((back_mu - mu).abs() < 1e-3, "{mu} came back as {back_mu}");
        }
        assert!((table_row(table_elevation(57, 0.05), 0.05) - 57.0).abs() < 1e-3);
    }

    /// `NERTHUS_SKY_PPM=<folder>` writes the dome's texture for a few suns
    /// and heights there, to look at.
    #[test]
    fn domes_can_be_written_out() {
        let Ok(folder) = std::env::var("NERTHUS_SKY_PPM") else {
            return;
        };
        let mut table = None;
        for (elevation, altitude) in [(33.0, 1000.0), (5.0, 1000.0), (33.0, 40_000.0), (1.0, 2000.0)] {
            let request = SkyRequest { sun_azimuth: 45.0, sun_elevation: elevation, altitude };
            let image = build_sky(request, &mut table);
            let mut ppm = format!("P6\n{DOME_WIDTH} {DOME_HEIGHT}\n255\n").into_bytes();
            for pixel in image.pixels.chunks(4) {
                ppm.extend_from_slice(&pixel[..3]);
            }
            let path = std::path::Path::new(&folder).join(format!("sky-{elevation}-{altitude}.ppm"));
            std::fs::write(path, ppm).expect("the sky is written");
        }
    }

    #[test]
    fn the_noon_sky_is_blue_and_the_sunset_glows() {
        let noon = SkyTable::new(60.0, 1000.0);
        let zenith = noon.colour(std::f32::consts::FRAC_PI_2 - 0.01, 0.0);
        assert!(zenith[2] > zenith[1] && zenith[1] > zenith[0], "{zenith:?}");
        // The exposure leaves the sun's disc white even this low.
        let request = SkyRequest { sun_azimuth: 0.0, sun_elevation: 1.0, altitude: 1000.0 };
        let sun = sun_colour(request, noon.dip).expect("the sun is up");
        assert!(sun.iter().all(|channel| *channel > 0.99), "{sun:?}");
        // The horizon under a setting sun is warm.
        let sunset = SkyTable::new(1.0, 2000.0);
        let glow = sunset.colour(-sunset.dip + 0.01, 0.0);
        assert!(glow[0] > glow[2], "{glow:?}");
        let white = Atmosphere::earth().white_point;
        assert!((white[0] + white[1] + white[2] - 3.0).abs() < 1e-4);
    }
}
