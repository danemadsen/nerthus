//! The crater field generator (`CraterfieldGenerator`) and the landscapes
//! built on it (`Landscapes.Crater`, `Landscapes.Craterfield`).
//!
//! The field is cut into unit cells in world space; each cell holds
//! `density` craters with exponentially distributed radii. Craters are
//! pressed into the heightmap one after another (sorted by hash), each
//! levelling the ground it lands on and warped by a smooth noise field.

use std::cmp::Ordering;

use nerthus_engine::{Map, Resolution};
use nerthus_noise::fast_noise::{FastNoise, FractalType, NoiseType};
use rayon::prelude::*;

use crate::dotnet_sort;
use crate::maths::{self, dotnet_max, dotnet_min};
use crate::random::XorShift64Star;

/// `CraterfieldGenerator.hash`: FNV-1a over two integers.
fn cell_hash(a: i32, b: i32) -> u32 {
    let mixed = (-2128831035i32 ^ a).wrapping_mul(16777619);
    (mixed ^ b).wrapping_mul(16777619) as u32
}

/// `CraterfieldGenerator.random`: one LCG step.
fn next_random(value: u32) -> u32 {
    1402024253u32.wrapping_mul(value).wrapping_add(586950981)
}

#[derive(Clone, Copy, Debug)]
struct Crater {
    hash: u32,
    x: f32,
    y: f32,
    radius: f32,
}

impl Crater {
    fn new(x: f32, y: f32, radius: f32, hash: u32) -> Self {
        Self {
            hash: next_random(hash.wrapping_add(13)),
            x,
            y,
            radius,
        }
    }
}

/// `RenderContext`: maps world coordinates onto the heightmap and holds
/// the warp field.
struct RenderContext {
    resolution: Resolution,
    top_left: [f32; 2],
    field_size: f32,
    world_to_heightmap: f32,
    warp_x: Vec<f32>,
    warp_y: Vec<f32>,
    heights: Vec<f32>,
    depth: f32,
}

/// `RenderContext.fade`: the quintic smoothstep.
fn fade(x: f32) -> f32 {
    x * x * x * (x * (x * 6.0 - 15.0) + 10.0)
}

impl RenderContext {
    fn new(heightmap: &Map, depth: f32, top_left: [f32; 2], field_size: f32, warp_strength: f32, seed: i32) -> Self {
        let resolution = heightmap.resolution();
        let world_to_heightmap = 1.0 / field_size * resolution.x as f32;
        let wavelength = 0.025f32;
        let make_noise = |noise_seed: i32| {
            let mut noise = FastNoise::new(noise_seed);
            noise.set_fractal_type(FractalType::Fbm);
            noise.set_noise_type(NoiseType::PerlinFractal);
            noise.set_fractal_octaves(7);
            noise.set_frequency(1.0 / wavelength);
            noise.set_fractal_gain(0.45);
            noise
        };
        let noise_x = make_noise(seed + 8);
        let noise_y = make_noise(seed + 9);
        let fade_length = 0.5 * resolution.x as f32 * wavelength / field_size;

        let width = resolution.x;
        let height = resolution.y;
        let rows: Vec<(Vec<f32>, Vec<f32>)> = (0..height)
            .into_par_iter()
            .map(|y| {
                let row_y = top_left[1] + field_size * y as f32 / height as f32;
                let positions: Vec<[f32; 3]> = (0..width)
                    .map(|x| [top_left[0] + field_size * x as f32 / width as f32, row_y, 0.0])
                    .collect();
                let values_x = noise_x.fill_vector_set(&positions, [0.0; 3]);
                let values_y = noise_y.fill_vector_set(&positions, [0.0; 3]);
                let row_edge = y.min(height - 1 - y) as f32;
                let row_fade = fade(dotnet_min(row_edge / fade_length, 1.0));
                let mut warped_x = Vec::with_capacity(width);
                let mut warped_y = Vec::with_capacity(width);
                for x in 0..width {
                    let column_edge = x.min(width - 1 - x) as f32;
                    let column_fade = fade(dotnet_min(column_edge / fade_length, 1.0));
                    warped_x.push(values_x[x] * column_fade * row_fade * warp_strength);
                    warped_y.push(values_y[x] * column_fade * row_fade * warp_strength);
                }
                (warped_x, warped_y)
            })
            .collect();
        let mut warp_x = Vec::with_capacity(width * height);
        let mut warp_y = Vec::with_capacity(width * height);
        for (row_x, row_y) in rows {
            warp_x.extend(row_x);
            warp_y.extend(row_y);
        }

        Self {
            resolution,
            top_left,
            field_size,
            world_to_heightmap,
            warp_x,
            warp_y,
            heights: heightmap.data().to_vec(),
            depth,
        }
    }

    fn to_heightmap(&self, x: f32, y: f32) -> (f32, f32) {
        ((x - self.top_left[0]) * self.world_to_heightmap, (y - self.top_left[1]) * self.world_to_heightmap)
    }

    fn to_world(&self, x: f32, y: f32) -> (f32, f32) {
        (x / self.world_to_heightmap + self.top_left[0], y / self.world_to_heightmap + self.top_left[1])
    }

    /// `Map.InterpolateBilinear(Vector2)`: fractions taken before the
    /// indices are clamped.
    fn bilinear(&self, x: f32, y: f32) -> f32 {
        let x = x - 0.5;
        let y = y - 0.5;
        let column = x as i32;
        let row = y as i32;
        let fraction_x = x - column as f32;
        let fraction_y = y - row as f32;
        let width = self.resolution.x as i32;
        let height = self.resolution.y as i32;
        let left = column.max(0).min(width - 1);
        let top = row.max(0).min(height - 1);
        let right = (left + 1).min(width - 1);
        let bottom = (top + 1).min(height - 1);
        let at = |column: i32, row: i32| self.heights[(row * width + column) as usize];
        let upper = at(left, top) * (1.0 - fraction_x) + at(right, top) * fraction_x;
        let lower = at(left, bottom) * (1.0 - fraction_x) + at(right, bottom) * fraction_x;
        upper * (1.0 - fraction_y) + lower * fraction_y
    }
}

/// `craterProfile`: the crater's relative depth, its blend mask and the
/// relative floor height at `distance` (0 at the centre, 1 at the rim).
fn crater_profile(distance: f32, size: f32) -> (f32, f32, f32) {
    let low = -1.0f32;
    let rim = 0.4 / (1.0 + size * 3.0);
    if distance > 1.0 {
        return (0.0, 0.0, 0.0);
    }
    let mut maturity = 1.0 - size;
    // `maturity *= maturity * maturity`, as the generator has it.
    #[allow(clippy::misrefactored_assign_op)]
    {
        maturity *= maturity * maturity;
    }
    maturity = 1.0 - maturity;
    let floor = -1.1 + maturity * 1.05;
    if distance < 0.5 {
        let mut profile = distance * distance * distance * 8.0;
        profile *= rim - low;
        profile += low;
        (profile, 1.0, floor)
    } else {
        let mut profile = 2.0 * (1.0 - distance);
        profile *= profile;
        let mask = profile;
        profile *= rim;
        (profile, mask, floor)
    }
}

/// `smoothFloor`: lifts values below `threshold` onto a soft floor.
fn smooth_floor(mut value: f32, mut threshold: f32, mut smoothing: f32, slope: f32) -> f32 {
    let lift = |value: f32, threshold: f32, smoothing: f32| {
        if !(value < threshold) {
            return value;
        }
        let mut depth = (threshold - value) / (smoothing * 2.0);
        if depth > 1.0 - slope {
            threshold - smoothing * (1.0 - slope * slope + 2.0 * slope * (depth - 1.0 + slope))
        } else {
            depth = 1.0 - depth;
            depth *= depth;
            threshold + smoothing * (depth - 1.0)
        }
    };
    if smoothing < 0.0 {
        smoothing = 0.0 - smoothing;
        value = 0.0 - value;
        threshold = 0.0 - threshold;
        value = lift(value, threshold, smoothing);
        return 0.0 - value;
    }
    lift(value, threshold, smoothing)
}

/// `Cellfield.RenderCrater`: presses one crater (heightmap units) into
/// the context's heights within the given bounds.
fn render_crater(context: &mut RenderContext, crater: &Crater, seed: i32, bounds: (i32, i32, i32, i32)) {
    let (center_x, center_y) = context.to_heightmap(crater.x, crater.y);
    let radius = crater.radius * context.world_to_heightmap;
    let relative_size = crater.radius;
    let (x_min, y_min, x_max, y_max) = bounds;
    let width = context.resolution.x;
    let height = context.resolution.y;

    let mut hash = next_random(crater.hash.wrapping_add(seed as u32));
    let warp_strength = width as f32 / context.field_size * 0.03 * relative_size.sqrt() * (0.8 + 9.765625e-5 * (hash & 0xFFF) as f32);
    let height_scale = 0.15 * (radius / width as f32) * context.depth;
    let radius_inverse = 1.0 / radius;
    let mut center_height = context.bilinear(center_x, center_y);
    let half = radius * 0.5;
    let top_left = context.bilinear(center_x - half, center_y - half);
    let top_right = context.bilinear(center_x + half, center_y - half);
    let bottom_left = context.bilinear(center_x - half, center_y + half);
    let bottom_right = context.bilinear(center_x + half, center_y + half);
    center_height += top_left + top_right + bottom_left + bottom_right;
    center_height *= 0.2;

    let mut x0 = (center_x - radius - warp_strength) as i32 + 1;
    let mut x1 = (center_x + radius + warp_strength) as i32;
    let mut y0 = (center_y - radius - warp_strength) as i32 + 1;
    let mut y1 = (center_y + radius + warp_strength) as i32;
    let mut gradient_x = 0.5 * (top_right - top_left + bottom_right - bottom_left) / (radius * 0.5);
    let mut gradient_y = 0.5 * (bottom_left - top_left + bottom_right - top_right) / (radius * 0.5);
    gradient_x *= 0.425;
    gradient_y *= 0.425;
    x0 = x0.min(x_max).max(x_min);
    x1 = x1.min(x_max).max(x_min);
    y0 = y0.min(y_max).max(y_min);
    y1 = y1.min(y_max).max(y_min);

    let mut warp_offset = |extent: usize| {
        hash = next_random(hash);
        ((hash & 0xFFFF) as f32 * (1.0 / 65536.0) * extent as f32 / context.field_size) as i32
    };
    let x_shift_x = warp_offset(width);
    let y_shift_x = warp_offset(width);
    let x_shift_y = warp_offset(height);
    let y_shift_y = warp_offset(height);

    let floor_smoothing = context.depth * 0.004;
    let mask_x = width as i32 - 1;
    let mask_y = height as i32 - 1;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let warp_x_index = (((y + y_shift_x) & mask_x) * width as i32 + ((x + x_shift_x) & mask_x)) as usize;
            let warp_y_index = (((y + y_shift_y) & mask_y) * width as i32 + ((x + x_shift_y) & mask_y)) as usize;
            let warped_x = x as f32 + context.warp_x[warp_x_index] * warp_strength;
            let warped_y = y as f32 + context.warp_y[warp_y_index] * warp_strength;
            let offset_x = warped_x - center_x;
            let offset_y = warped_y - center_y;
            let distance_squared = offset_x * offset_x + offset_y * offset_y;
            if distance_squared < radius * radius {
                let index = (y * width as i32 + x) as usize;
                let previous = context.heights[index];
                let (mut profile, mask, mut floor) = crater_profile(distance_squared.sqrt() * radius_inverse, relative_size);
                profile *= height_scale;
                floor *= height_scale;
                floor += center_height;
                let mut value = center_height + offset_x * gradient_x + offset_y * gradient_y + profile;
                value = smooth_floor(value, floor, floor_smoothing, 0.2);
                context.heights[index] = value * mask + (1.0 - mask) * previous;
            }
        }
    }
}

/// `Cell.CreateCraterList`: the craters of unit cell (`x`, `y`).
fn cell_craters(x: i32, y: i32, seed: i32, density: i32, minimum_radius: f32) -> Vec<Crater> {
    let cell_seed = next_random(cell_hash(x, y)).wrapping_add(seed as u32);
    let mut random = XorShift64Star::new(cell_seed as i32);
    let steepness = 12.0f32;
    let normalizer = 1.0 / (maths::exp(steepness) - 1.0);
    let mut craters = Vec::new();
    for _ in 0..density {
        let mut size = 0.0f32;
        for _ in 0..5 {
            size += random.next_float();
        }
        size *= 0.2;
        size = (maths::exp(size * steepness) - 1.0) * normalizer;
        let offset_x = random.next_float();
        let offset_y = random.next_float();
        let hash = random.next_int() as u32;
        if size >= minimum_radius {
            craters.push(Crater::new(x as f32 + offset_x, y as f32 + offset_y, size, hash));
        }
    }
    craters
}

fn by_hash(a: &Crater, b: &Crater) -> Ordering {
    a.hash.cmp(&b.hash)
}

/// `CraterfieldGenerator.Render` + `Cellfield.RenderWhole`: every crater of
/// the cells covering the field, in hash order.
fn render_field(heightmap: &Map, density: i32, depth: f32, seed: i32, top_left: [f32; 2], field_size: f32) -> Map {
    let mut context = RenderContext::new(heightmap, depth, top_left, field_size, 1.0, seed);
    let minimum_radius = context.field_size / context.resolution.x as f32 * 0.5;
    let first_x = top_left[0] as i32 - 1;
    let first_y = top_left[1] as i32 - 1;
    let cells = field_size as i32 + 3;
    let rows: Vec<Vec<Crater>> = (0..cells)
        .into_par_iter()
        .map(|row| {
            let mut craters = Vec::new();
            for column in 0..cells {
                craters.extend(cell_craters(column + first_x, row + first_y, seed, density, minimum_radius));
            }
            craters
        })
        .collect();
    let mut craters: Vec<Crater> = rows.into_iter().flatten().collect();
    dotnet_sort::introspective_sort(&mut craters, &by_hash);

    let bounds = (0, 0, context.resolution.x as i32 - 1, context.resolution.y as i32 - 1);
    for crater in &craters {
        render_crater(&mut context, crater, seed, bounds);
    }
    Map::from_data(context.resolution, context.heights)
}

/// `Landscapes.Craterfield`: a crater field over `map` (or a flat 0.5
/// plane), `scale` craters per unit across, offset by (`x`, `y`).
pub fn craterfield(map: &Map, scale: f32, density: f32, depth: f32, x: f32, y: f32, seed: i32) -> Map {
    let field_size = 1.0 / dotnet_max(scale, 0.01);
    let top_left = [x - field_size / 2.0, y - field_size / 2.0];
    render_field(map, (density * 10000.0) as i32, depth, seed, top_left, field_size)
}

/// `Landscapes.Crater`: one crater of relative size `scale` at (`x`, `y`)
/// (0..1 of the map), its walls warped by `warp`.
pub fn crater(map: &Map, scale: f32, depth: f32, warp: f32, seed: i32, x: f32, y: f32) -> Map {
    let field_size = 1.0 / dotnet_max(warp * 2.5, 0.01);
    let top_left = [x - field_size / 2.0, y - field_size / 2.0];
    let mut context = RenderContext::new(map, depth, top_left, field_size, 1.0, seed);
    let resolution = context.resolution;
    let (world_x, world_y) = context.to_world(resolution.x as f32 * x, resolution.y as f32 * y);
    let world_radius = resolution.x as f32 * scale / context.world_to_heightmap;
    let hash = (seed.wrapping_mul(1234)) as u32;
    let crater = Crater::new(world_x, world_y, world_radius, hash);
    let bounds = (0, 0, resolution.x as i32 - 1, resolution.y as i32 - 1);
    render_crater(&mut context, &crater, seed, bounds);
    Map::from_data(resolution, context.heights)
}
