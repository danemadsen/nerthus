//! Debris (`DebrisCore` and `Scatters.RenderRockColored`): rocks dropped
//! on the terrain roll
//! downhill under gravity and friction, bouncing off the slope, and are
//! rendered where they stop as warped domes or faceted rocks.

use nerthus_engine::{Map, Resolution};
use nerthus_noise::fast_noise::{FastNoise, FractalType, NoiseType};
use rayon::prelude::*;

use crate::dotnet_vector::{self, Vector3};
use crate::random::XorShift64Star;
use crate::x86;

/// `DebrisCore.Parameters` as the Debris node fills them.
pub struct DebrisSettings {
    pub amount: i32,
    pub seed: i32,
    pub friction: f32,
    pub restitution: f32,
    /// The terrain's width in metres over the node's scale.
    pub heightmap_size: f32,
    /// The terrain's height in metres over the node's scale.
    pub scene_height: f32,
    pub min_size: f32,
    pub max_size: f32,
    /// `SizeDistribution.Natural` (else uniform).
    pub natural_sizes: bool,
    pub render_still_rocks: bool,
    pub rock_height: f32,
    /// `DebrisShape.Sharp` (faceted rocks) rather than rounded ones.
    pub sharp: bool,
    /// The terrain's width and height in metres.
    pub terrain: (f32, f32),
}

/// The rocks a Debris node placed when it exports a point cloud
/// (`Debris.pointCloud`, filled outside the editor): each rock's position
/// over the map's size and its top in terrain widths. PointCloud exports
/// these instead of its map.
#[derive(Clone, Debug, Default)]
pub struct DebrisPointCloud(pub Vec<[f32; 3]>);

/// The maps the Debris node commits.
pub struct DebrisMaps {
    pub heights: Map,
    pub colour_index: Map,
    pub debris: Map,
}

const GRAVITY: f32 = 9.81;
const TIME_STEP: f32 = 0.03;
const WARP_SIZE: usize = 1024;

/// A row-major plane addressed like `Map[x, y]`.
struct Grid {
    width: usize,
    height: usize,
    data: Vec<f32>,
}

impl Grid {
    fn at(&self, x: i32, y: i32) -> f32 {
        self.data[y as usize * self.width + x as usize]
    }

    /// `Map.InterpolateBilinear(position)`: texel centres at +0.5, the
    /// cell truncated towards zero (so a negative position extrapolates).
    fn bilinear(&self, x: f32, y: f32) -> f32 {
        let x = x - 0.5;
        let y = y - 0.5;
        let column = x86::cvttss2si(x);
        let row = x86::cvttss2si(y);
        let fraction_x = x - column as f32;
        let fraction_y = y - row as f32;
        let last_x = self.width as i32 - 1;
        let last_y = self.height as i32 - 1;
        let left = column.max(0).min(last_x);
        let top = row.max(0).min(last_y);
        let right = (left + 1).min(last_x);
        let bottom = (top + 1).min(last_y);
        let upper = self.at(left, top) * (1.0 - fraction_x) + self.at(right, top) * fraction_x;
        let lower = self.at(left, bottom) * (1.0 - fraction_x) + self.at(right, bottom) * fraction_x;
        upper * (1.0 - fraction_y) + lower * fraction_y
    }
}

/// `WarpField`: two 1024² fractal Perlin fields that wobble the rocks'
/// outlines (tighter and gentler for sharp rocks).
struct WarpField {
    x: Grid,
    y: Grid,
}

impl WarpField {
    fn new(seed: i32, sharp: bool) -> WarpField {
        let wavelength = 256.0 * if sharp { 0.3 } else { 1.0 };
        let gain = if sharp { 0.5 } else { 0.45 };
        let amplitude = if sharp { 0.3 } else { 1.4 };
        let make = |noise_seed: i32| {
            let mut generator = FastNoise::new(1337);
            generator.set_fractal_type(FractalType::Fbm);
            generator.set_noise_type(NoiseType::PerlinFractal);
            generator.set_fractal_octaves(9);
            generator.set_frequency(1.0 / wavelength);
            generator.set_fractal_gain(gain);
            generator.set_seed(noise_seed);
            let mut data = vec![0.0f32; WARP_SIZE * WARP_SIZE];
            data.par_chunks_mut(WARP_SIZE).enumerate().for_each(|(row, values)| {
                generator.fill_noise_set(values, 0, row as i32, 0, WARP_SIZE, 1, 1, 1.0);
                for value in values.iter_mut() {
                    *value *= amplitude;
                }
            });
            Grid { width: WARP_SIZE, height: WARP_SIZE, data }
        };
        WarpField { x: make(seed + 8), y: make(seed + 9) }
    }
}

/// `Plane` (in `Nodes.Core`): the height over (x, y) of the plane
/// through three points.
#[derive(Clone, Copy, Default)]
struct FacetPlane {
    a: f32,
    b: f32,
    c: f32,
}

impl FacetPlane {
    fn new(first: Vector3, second: Vector3, third: Vector3) -> FacetPlane {
        let [ax, ay, az] = first;
        let [bx, by, bz] = second;
        let [cx, cy, cz] = third;
        let determinant = (bx - ax) * cy + (ay - by) * cx + ax * by - ay * bx;
        let inverse = 1.0 / determinant;
        let a = -((by - ay) * cz + (az - bz) * cy + ay * bz - az * by) * inverse;
        let b = ((bx - ax) * cz + (az - bz) * cx + ax * bz - az * bx) * inverse;
        let c = ((ax * by - ay * bx) * cz + (az * bx - ax * bz) * cy + (ay * bz - az * by) * cx) * inverse;
        FacetPlane { a, b, c }
    }

    fn z(&self, x: f32, y: f32) -> f32 {
        self.a * x + self.b * y + self.c
    }
}

/// `FacetedRock`: a pyramid of four faces around a random peak, cut by a
/// fifth tilted face.
#[derive(Clone, Copy, Default)]
struct FacetedRock {
    planes: [FacetPlane; 5],
}

impl FacetedRock {
    fn circle_vertex(index: i32) -> Vector3 {
        let angle = index as f32 * (std::f32::consts::PI / 16.0);
        [angle.cos() * 0.5 + 0.5, angle.sin() * 0.5 + 0.5, 0.0]
    }

    fn new(seed: i32) -> FacetedRock {
        let mut random = XorShift64Star::new(seed.wrapping_sub(29));
        let start = random.next_int() & 0x1f;
        let corner_0 = Self::circle_vertex((start + (random.next_int() & 7)) & 0x1f);
        let corner_1 = Self::circle_vertex((start + 8 + (random.next_int() & 7)) & 0x1f);
        let corner_2 = Self::circle_vertex((start + 16 + (random.next_int() & 7)) & 0x1f);
        let corner_3 = Self::circle_vertex((start + 24 + (random.next_int() & 7)) & 0x1f);
        let blend_x = random.next_float() * 0.8 + 0.1;
        let blend_y = random.next_float() * 0.8 + 0.1;
        let mix = |p: Vector3, q: Vector3, t: f32| -> Vector3 {
            let scaled_p = [p[0] * t, p[1] * t, p[2] * t];
            let keep = 1.0 - t;
            let scaled_q = [q[0] * keep, q[1] * keep, q[2] * keep];
            dotnet_vector::add(scaled_p, scaled_q)
        };
        let near = mix(corner_0, corner_1, blend_x);
        let far = mix(corner_2, corner_3, blend_x);
        let near_scaled = [near[0] * blend_y, near[1] * blend_y, near[2] * blend_y];
        let keep = 1.0 - blend_y;
        let far_scaled = [keep * far[0], keep * far[1], keep * far[2]];
        let mut peak = dotnet_vector::add(near_scaled, far_scaled);
        peak[2] = 2.0;

        let (z_a, z_b, z_c);
        if random.next_int() & 1 == 0 {
            z_a = random.next_float() * 2.0;
            z_b = random.next_float() * 2.0;
            z_c = random.next_float() * 2.0;
        } else {
            z_c = random.next_float() * 2.0;
            z_b = random.next_float() * 2.0;
            z_a = random.next_float() * 2.0;
        }
        let (a, b, c) = if random.next_int() & 1 == 0 {
            ([0.5, 0.0, z_a], [0.933_012_7, 0.75, z_b], [0.066_987_3, 0.75, z_c])
        } else {
            ([0.5, 1.0, z_a], [0.066_987_3, 0.25, z_b], [0.933_012_7, 0.25, z_c])
        };
        FacetedRock {
            planes: [
                FacetPlane::new(corner_0, corner_1, peak),
                FacetPlane::new(corner_1, corner_2, peak),
                FacetPlane::new(corner_2, corner_3, peak),
                FacetPlane::new(corner_3, corner_0, peak),
                FacetPlane::new(a, b, c),
            ],
        }
    }

    fn z(&self, x: f32, y: f32) -> f32 {
        let [p0, p1, p2, p3, p4] = &self.planes;
        let first = p0.z(x, y).min(p1.z(x, y));
        let second = p2.z(x, y).min(p3.z(x, y));
        first.min(second).min(p4.z(x, y))
    }
}

/// The maps a rock is rendered into.
struct Canvas {
    heights: Grid,
    debris: Grid,
    colour_index: Grid,
}

/// `RockRenderingParameters`.
struct RockShape {
    position: (f32, f32),
    size: f32,
    relative_size: f32,
    id: i32,
}

/// `Scatters.MeasureArea`: the ground under a rock — its mean height over
/// the rock's span, the height at its centre, and the slope across it.
fn measure_area(heights: &Grid, centre: (f32, f32), size: f32) -> (f32, f32, f32, f32) {
    let last_x = heights.width as i32 - 1;
    let last_y = heights.height as i32 - 1;
    let x = x86::cvttss2si(centre.0).min(last_x).max(0);
    let y = x86::cvttss2si(centre.1).min(last_y).max(0);
    if size < 4.0 {
        let fraction_x = centre.0 - x as f32;
        let fraction_y = centre.1 - y as f32;
        let right = (x + 1).min(last_x);
        let bottom = (y + 1).min(last_y);
        let top_left = heights.at(x, y);
        let top_right = heights.at(right, y);
        let bottom_left = heights.at(x, bottom);
        let bottom_right = heights.at(right, bottom);
        let upper = top_left * (1.0 - fraction_x) + top_right * fraction_x;
        let lower = bottom_left * (1.0 - fraction_x) + bottom_right * fraction_x;
        let mean = upper * (1.0 - fraction_y) + lower * fraction_y;
        let slope_x = 0.5 * (top_right - top_left + bottom_right - bottom_left);
        let slope_y = 0.5 * (bottom_left - top_left + bottom_right - top_right);
        return (mean, heights.at(x, y), slope_x, slope_y);
    }
    let half = x86::cvttss2si(size) >> 1;
    let left = (x - half).min(last_x).max(0);
    let top = (y - half).min(last_y).max(0);
    let right = (x + half).min(last_x).max(0);
    let bottom = (y + half).min(last_y).max(0);
    let top_left = heights.at(left, top);
    let top_right = heights.at(right, top);
    let bottom_left = heights.at(left, bottom);
    let bottom_right = heights.at(right, bottom);
    let mean = 0.25 * (top_left + top_right + bottom_left + bottom_right);
    let scale = 0.5 / (half as f32 * 2.0 + 1.0);
    let slope_x = (top_right - top_left + bottom_right - bottom_left) * scale;
    let slope_y = (bottom_left - top_left + bottom_right - top_right) * scale;
    (mean, heights.at(x, y), slope_x, slope_y)
}

/// `Scatters.RenderRockColored`: the rock as a dome (or faceted rock)
/// tilted with the ground, its outline warped, raising the heightmap where
/// it stands above it.
fn render_rock(canvas: &mut Canvas, rock: &RockShape, warp: &WarpField, settings: &DebrisSettings) {
    let heights = &mut canvas.heights;
    let resolution_x = heights.width as i32;
    let resolution_y = heights.height as i32;
    let mut random = XorShift64Star::new(rock.id);
    let (position_x, position_y) = rock.position;
    let pixels = rock.size * (resolution_x as f32 / settings.heightmap_size);
    let rise = (((0.3 - rock.relative_size * 0.1) * rock.size) / settings.scene_height) * settings.rock_height;
    if pixels < 1.5 {
        let raised = rise * pixels * (random.next_float() * 0.5 + 0.5);
        let x = x86::cvttss2si(position_x);
        let y = x86::cvttss2si(position_y);
        if x >= 0 && x < resolution_x && y >= 0 && y < resolution_y {
            let pixel = y as usize * heights.width + x as usize;
            heights.data[pixel] += raised * settings.rock_height;
            canvas.debris.data[pixel] = 1.0;
            canvas.colour_index.data[pixel] = random.next_float();
        }
        return;
    }
    let reach = x86::cvttss2si(pixels * 0.75);
    let left = (x86::cvttss2si(position_x) - reach - 1).max(0);
    let right = (x86::cvttss2si(position_x) + reach).min(resolution_x - 1);
    let top = (x86::cvttss2si(position_y) - reach - 1).max(0);
    let bottom = (x86::cvttss2si(position_y) + reach).min(resolution_y - 1);
    let radius_squared = pixels * pixels * 0.25;
    let sharp = settings.sharp && !(pixels < 3.0);

    let (mean, centre_height, slope_x, slope_y) = measure_area(heights, rock.position, pixels);
    let slope_x = slope_x * 0.7;
    let slope_y = slope_y * 0.7;
    let unevenness = ((centre_height - mean) / rise).abs();
    let base = if sharp { 0.5 * (mean + centre_height.max(mean)) } else { mean };
    let pixel_size = settings.terrain.0 / resolution_x as f32;
    let height_per_pixel = 1.0 * pixel_size / settings.terrain.1;
    let gradient = (slope_x * slope_x + slope_y * slope_y).sqrt();
    let steepness = gradient / height_per_pixel;
    let mut flatten = steepness * 0.5 + 1.0;
    let (mut downhill_x, mut downhill_y) = (slope_x, slope_y);
    if gradient > 0.0 {
        let inverse = 1.0 / gradient;
        downhill_x *= inverse;
        downhill_y *= inverse;
    }
    let (across_x, across_y) = (-downhill_y, downhill_x);
    flatten += unevenness;
    let warp_span = WARP_SIZE as f32 - 256.0;
    let warp_left = x86::cvttss2si(random.next_float() * warp_span) as f32;
    let warp_top = x86::cvttss2si(random.next_float() * warp_span) as f32;
    let warp_step = if pixels < 2.0 { 1.0 } else { 128.0 / pixels };
    let colour = random.next_float();
    let nearest_warp = pixels < 64.0;
    let facet_scale = 0.7 / pixels;
    let (rock_a, rock_b) = if sharp {
        let first = FacetedRock::new(random.next_int());
        let second = FacetedRock::new(random.next_int());
        (first, second)
    } else {
        (FacetedRock::default(), FacetedRock::default())
    };
    let half_width = (right - left) as f32 / 2.0;
    let half_height = (bottom - top) as f32 / 2.0;

    for i in top..=bottom {
        for j in left..=right {
            let (warp_x, warp_y, sample_x, sample_y);
            if gradient > 0.0 {
                let offset_x = j as f32 - position_x;
                let offset_y = i as f32 - position_y;
                let along = across_x * offset_x + across_y * offset_y;
                let mut down = downhill_x * offset_x + downhill_y * offset_y;
                down *= steepness * 0.7 + 1.0;
                let mut u = along + half_width;
                let mut v = down + half_height;
                v = v.max(0.0).min((bottom - top) as f32);
                u = u.max(0.0).min((right - left) as f32);
                warp_x = warp_left + u * warp_step;
                warp_y = warp_top + v * warp_step;
                let back_x = along * across_x + down * downhill_x;
                let back_y = along * across_y + down * downhill_y;
                sample_x = (back_x + position_x).max(0.0);
                sample_y = (back_y + position_y).max(0.0);
            } else {
                sample_x = j as f32;
                sample_y = i as f32;
                warp_x = warp_left + (j - left) as f32 * warp_step;
                warp_y = warp_top + (i - top) as f32 * warp_step;
            }
            let (shift_x, shift_y) = if nearest_warp {
                let column = x86::cvttss2si(warp_x);
                let row = x86::cvttss2si(warp_y);
                (warp.x.at(column, row) * 0.6 * pixels, warp.y.at(column, row) * 0.6 * pixels)
            } else {
                (warp.x.bilinear(warp_x, warp_y) * 0.6 * pixels, warp.y.bilinear(warp_x, warp_y) * 0.6 * pixels)
            };
            let warped_x = sample_x + shift_x;
            let warped_y = sample_y + shift_y;
            let mut shape;
            if sharp {
                let facet_x = (position_x - warped_x) * facet_scale + 0.5;
                let facet_y = (position_y - warped_y) * facet_scale + 0.5;
                let first = rock_a.z(facet_x, facet_y);
                let second = rock_b.z(facet_x, facet_y);
                shape = (first + second + first.min(second)) * 0.22;
                if shape < 0.0 {
                    shape = if shape > -1.0 { shape * (1.0 / (1.0 + shape)) } else { -99999.9 };
                }
                shape *= 1.0 + 0.5 * steepness;
            } else {
                let distance_x = position_x - warped_x;
                let distance_y = position_y - warped_y;
                let distance = distance_x * distance_x + distance_y * distance_y;
                let distance = distance / radius_squared;
                shape = if distance <= 1.0 { (1.0 - distance).sqrt() } else { -10.0 };
                shape = shape * flatten + (1.0 - flatten);
                shape *= 1.0 + 0.5 * steepness;
            }
            let tilt = (j as f32 - position_x) * slope_x + (i as f32 - position_y) * slope_y;
            let height = base + shape * rise + tilt;
            let pixel = i as usize * heights.width + j as usize;
            if height > heights.data[pixel] {
                heights.data[pixel] = height;
                canvas.colour_index.data[pixel] = colour;
                canvas.debris.data[pixel] = 1.0;
            }
        }
    }
}

/// `DebrisCore.Velocity3D`: a 2D velocity laid onto the terrain's plane.
fn velocity_on_slope(velocity: (f32, f32), normal: Vector3) -> Vector3 {
    let flat = [velocity.0, velocity.1, 0.0];
    let along = dotnet_vector::dot(flat, normal);
    let removed = [along * normal[0], along * normal[1], along * normal[2]];
    let projected = [flat[0] - removed[0], flat[1] - removed[1], flat[2] - removed[2]];
    dotnet_vector::normalize(projected)
}

fn length_2d(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

/// `DebrisCore.Simulation`: `settings.amount` rocks, each dropped at a
/// random spot (one the emitter marks, when given) with a random size,
/// then rolled until it stops, leaves the map or is removed. With
/// `point_cloud`, each rendered rock also adds a point: its position and
/// the height there once it is drawn, lifted by part of its size.
pub fn simulate(heights: &Map, emitter: Option<&Map>, removal: Option<&Map>, settings: &DebrisSettings, mut point_cloud: Option<&mut Vec<[f32; 3]>>) -> DebrisMaps {
    let resolution: Resolution = heights.resolution();
    let width = resolution.x;
    let height = resolution.y;
    let grid = |map: &Map| Grid { width, height, data: map.channel(0).to_vec() };
    let zeros = || Grid { width, height, data: vec![0.0; width * height] };
    let emitter = emitter.map(grid);
    let removal = removal.map(grid);
    let warp = WarpField::new(settings.seed, settings.sharp);
    let mut canvas = Canvas { heights: grid(heights), debris: zeros(), colour_index: zeros() };

    let last_x = (width as i32 - 1) as f32;
    let last_y = (height as i32 - 1) as f32;
    let cell = settings.heightmap_size / width as i32 as f32;
    // `vector`: what each point is scaled by.
    let point_scale = [1.0 / width as i32 as f32, 1.0 / height as i32 as f32, settings.scene_height / settings.heightmap_size];
    let mut random = XorShift64Star::new(settings.seed.wrapping_add(17));
    let smallest = settings.min_size / settings.max_size;

    for index in 0..settings.amount {
        let mut position;
        if let Some(emitter) = &emitter {
            let mut tries = 0;
            loop {
                let x = random.next_float() * last_x;
                let y = random.next_float() * last_y;
                position = (x, y);
                let keep_trying = tries < 2500;
                tries += 1;
                if !keep_trying || !(emitter.at(x86::cvttss2si(x), x86::cvttss2si(y)) < 0.5) {
                    break;
                }
            }
            if tries >= 2500 {
                continue;
            }
        } else {
            let x = random.next_float() * last_x;
            let y = random.next_float() * last_y;
            position = (x, y);
        }
        let mut relative = if settings.natural_sizes {
            let sum = random.next_float() + random.next_float() + random.next_float() + random.next_float();
            let mut size = 0.25 * sum;
            size *= size;
            size *= size;
            size *= size;
            size
        } else {
            random.next_float()
        };
        relative = smallest + (1.0 - smallest) * relative;

        let size = relative * settings.max_size;
        let half = size * 0.5 / cell;
        let steepness_scale = settings.scene_height / (size * 2.0);
        let mut velocity = (0.0f32, 0.0f32);
        let mut velocity_3d: Vector3 = [0.0; 3];
        let mut steps = 0;
        let mut kept = true;
        let _stress = random.next_float() * 0.67;
        let removal_threshold = random.next_float() * 0.999 + 0.0005;
        let ground = &canvas.heights;
        loop {
            let (x, y) = position;
            let top_left = ground.bilinear(x - half, y - half);
            let top_right = ground.bilinear(x + half, y - half);
            let bottom_left = ground.bilinear(x - half, y + half);
            let bottom_right = ground.bilinear(x + half, y + half);
            let slope_x = ((top_right - top_left) + bottom_right - bottom_left) * steepness_scale;
            let slope_y = ((bottom_left - top_left) + bottom_right - top_right) * steepness_scale;
            let normal = dotnet_vector::normalize([-slope_x, -slope_y, 1.0]);
            let gradient = length_2d(slope_x, slope_y);
            let sine = gradient.atan().sin();
            let cosine = (1.0 - sine * sine).sqrt();
            let (downhill_x, downhill_y) = if gradient == 0.0 { (0.0, 0.0) } else { (slope_x / gradient, slope_y / gradient) };
            let pull = GRAVITY * sine;
            let drag = -GRAVITY * settings.friction * cosine;
            let push = pull * TIME_STEP;
            velocity.0 += push * -downhill_x;
            velocity.1 += push * -downhill_y;
            let speed = length_2d(velocity.0, velocity.1);
            if speed == 0.0 {
                break;
            }
            let slowed = (speed + drag * TIME_STEP).max(0.0) / speed;
            velocity.0 *= slowed;
            velocity.1 *= slowed;
            let mut approach = dotnet_vector::dot(velocity_3d, normal);
            if approach < 0.0 && relative > 0.1 {
                approach *= -1.0;
                let loss = (approach * 1.08 - 0.08).max(0.0);
                let twice = 2.0 * approach;
                let bounced_x = velocity.0 - twice * downhill_x;
                let bounced_y = velocity.1 - twice * downhill_y;
                let restitution = settings.restitution;
                let keep = 1.0 - restitution;
                velocity.0 = restitution * bounced_x + keep * velocity.0;
                velocity.1 = restitution * bounced_y + keep * velocity.1;
                let damping = 1.0 - loss;
                velocity.0 *= damping;
                velocity.1 *= damping;
            }
            velocity_3d = velocity_on_slope(velocity, normal);
            if velocity.0 == 0.0 && velocity.1 == 0.0 {
                break;
            }
            position.0 += velocity.0 * TIME_STEP * cosine / cell;
            position.1 += velocity.1 * TIME_STEP * cosine / cell;
            let (x, y) = position;
            let outside = x < 0.0 || y < 0.0 || x > last_x || y > last_x;
            if outside {
                break;
            }
            let too_long = steps > 1000;
            steps += 1;
            if too_long {
                break;
            }
            if let Some(removal) = &removal
                && removal.at(x86::cvttss2si(x), x86::cvttss2si(y)) > removal_threshold {
                    kept = false;
                    break;
                }
        }
        let (x, y) = position;
        let inside = x >= 0.0 && y >= 0.0 && x <= last_x && y <= last_y;
        if kept && (steps > 0 || settings.render_still_rocks) && inside {
            let rock = RockShape { position, size, relative_size: relative, id: index };
            render_rock(&mut canvas, &rock, &warp, settings);
            if let Some(points) = point_cloud.as_deref_mut() {
                let ground = canvas.heights.bilinear(x, y);
                let lift = (0.3 - relative * 0.1) * size / settings.scene_height * settings.rock_height * 0.25;
                let top = ground + lift;
                points.push([x * point_scale[0], y * point_scale[1], top * point_scale[2]]);
            }
        }
    }

    DebrisMaps {
        heights: Map::from_data(resolution, canvas.heights.data),
        colour_index: Map::from_data(resolution, canvas.colour_index.data),
        debris: Map::from_data(resolution, canvas.debris.data),
    }
}
