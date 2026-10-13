//! Meshify's point placement (`Meshify.GetProbabilityMask`,
//! `PlacePoints0`, `PlacePoints1`): points drawn where the terrain bends,
//! which `mesh_rasterizer::render_point_cloud` then triangulates.

use nerthus_engine::Map;
use rayon::prelude::*;

use crate::aspect;
use crate::dotnet_priority_queue::PriorityQueue;
use crate::maths;
use crate::mathv;
use crate::random::XorShift64Star;

/// `GetProbabilityMask`: the absolute Laplacian (corners weighted 1/12,
/// edges 1/6), plus a little where a sloped pixel touches a flat one,
/// autolevelled and raised to `detail sensitivity × 4` (MathV's pow).
pub fn probability_mask(input: &Map, detail_sensitivity: f32, terrain_width: f32, terrain_height: f32) -> Map {
    let width = input.width();
    let height = input.height();
    let data = input.data();
    let flat = aspect::slope(input, 0.0, 0.0, 0.0, terrain_width, terrain_height);
    let flat = flat.data();

    let mut mask = vec![0.0f32; width * height];
    mask.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
        let above = y.saturating_sub(1);
        let below = (y + 1).min(height - 1);
        for (x, value) in row.iter_mut().enumerate() {
            let left = x.saturating_sub(1);
            let right = (x + 1).min(width - 1);
            let at = |column: usize, line: usize| data[line * width + column];
            let corners = ((at(left, above) + at(right, above)) + at(left, below)) + at(right, below);
            let edges = ((at(x, above) + at(left, y)) + at(right, y)) + at(x, below);
            let centre = at(x, y);
            let mut bend = ((centre - corners * (1.0f32 / 12.0)) - edges * (1.0f32 / 6.0)).abs();

            let flat_at = |column: usize, line: usize| flat[line * width + column];
            let neighbours = [
                flat_at(left, above),
                flat_at(x, above),
                flat_at(right, above),
                flat_at(left, y),
                flat_at(right, y),
                flat_at(left, below),
                flat_at(x, below),
                flat_at(right, below),
            ];
            let touches_flat = neighbours.iter().any(|&neighbour| neighbour > 0.0);
            let edge_of_flat = flat_at(x, y) == 0.0 && touches_flat;
            bend += if edge_of_flat { 0.0005 } else { 0.0 };
            *value = bend;
        }
    });

    let levelled = maths::autolevel(Map::from_data(input.resolution(), mask));
    let exponent = detail_sensitivity * 4.0;
    let raised: Vec<f32> = levelled.data().iter().map(|&value| mathv::max(mathv::pow(value, exponent), 1e-19)).collect();
    Map::from_data(input.resolution(), raised)
}

/// A point at a pixel's centre with the height there.
fn pixel_point(heightmap: &Map, x: usize, y: usize) -> [f32; 3] {
    let height = heightmap.data()[y * heightmap.width() + x];
    [x as f32 + 0.5, y as f32 + 0.5, height]
}

/// Exponential race keys (`−ln(r) / p`, negated): the queue keeps the
/// `capacity` pixels with the smallest keys.
fn race(queue: &mut PriorityQueue<(usize, usize)>, random: &mut XorShift64Star, x: usize, y: usize, probability: f32, capacity: usize) {
    let mut draw = random.next_float();
    if draw == 0.0 {
        draw = 1.0;
    }
    let key = (0.0 - maths::ln(draw)) / probability;
    queue.enqueue((x, y), 0.0 - key);
    if queue.len() > capacity {
        queue.dequeue();
    }
}

/// `PlacePoints0` (Meshify version 0): the four corner pixels, then the
/// winners of the race over every other pixel.
pub fn place_points_v0(heightmap: &Map, probability: &Map, points: &mut [[f32; 3]], seed: i32) {
    let width = probability.width();
    let height = probability.height();
    points[0] = [0.5, 0.5, heightmap.data()[0]];
    points[1] = [0.5, height as f32 - 0.5, heightmap.data()[(height - 1) * heightmap.width()]];
    points[2] = [width as f32 - 0.5, 0.5, heightmap.data()[width - 1]];
    points[3] = [width as f32 - 0.5, height as f32 - 0.5, heightmap.data()[(height - 1) * heightmap.width() + width - 1]];
    let capacity = points.len().saturating_sub(4);

    let mut queue = PriorityQueue::new();
    let mut random = XorShift64Star::new(seed);
    for y in 0..height {
        for x in 0..width {
            let corner = (x == 0 || x == width - 1) && (y == 0 || y == height - 1);
            if corner {
                continue;
            }
            let value = probability.data()[y * width + x];
            race(&mut queue, &mut random, x, y, value, capacity);
        }
    }
    let mut index = 4;
    while let Some((x, y)) = queue.dequeue() {
        points[index] = pixel_point(heightmap, x, y);
        index += 1;
    }
}

/// `PlacePoints1` (version 1): a jittered grid of about
/// `min(7%, 3√n)` points, then the race over the remaining pixels.
pub fn place_points_v1(heightmap: &Map, probability: &mut Map, points: &mut [[f32; 3]], seed: i32) {
    let count = points.len() as i32;
    let width = heightmap.width();
    let height = heightmap.height();
    let grid_target = ((count as f32 * 0.07) as i32).min((count as f32).sqrt() as i32 * 3);
    let mut random = XorShift64Star::new(seed);
    let aspect = width as i32 as f32 / height as i32 as f32;
    let mut rows = ((grid_target as f32 / aspect) as f64).sqrt().round_ties_even() as i32;
    let mut columns = ((rows as f32 * aspect) as f64).round_ties_even() as i32;
    if columns < 2 {
        columns = 2;
    }
    if rows < 2 {
        rows = 2;
    }
    let grid_count = columns * rows;
    let race_count = (count - grid_count).max(0) as usize;
    let column_step = (width as i32 - 1) as f32 / (columns - 1) as f32;
    let row_step = (height as i32 - 1) as f32 / (rows - 1) as f32;

    let mut filled = 0usize;
    for row in 0..rows {
        for column in 0..columns {
            let x = column as f32 * column_step;
            let y = row as f32 * row_step;
            let mut jitter_x = 0.0f32;
            let mut jitter_y = 0.0f32;
            if column != 0 && column != columns - 1 {
                jitter_x = (random.next_float() * 0.8 - 0.4) * column_step;
            }
            if row != 0 && row != rows - 1 {
                jitter_y = (random.next_float() * 0.8 - 0.4) * row_step;
            }
            let pixel_x = dotnet_clamp(x + jitter_x, 0.0, (width - 1) as f32) as i32 as usize;
            let pixel_y = dotnet_clamp(y + jitter_y, 0.0, (height - 1) as f32) as i32 as usize;
            points[filled] = pixel_point(heightmap, pixel_x, pixel_y);
            filled += 1;
            probability.data_mut()[pixel_y * width + pixel_x] = -1.0;
        }
    }

    let mut queue = PriorityQueue::new();
    for y in 0..height {
        for x in 0..width {
            let value = probability.data()[y * width + x];
            if value <= 0.0 {
                continue;
            }
            race(&mut queue, &mut random, x, y, value, race_count);
        }
    }
    while filled < points.len() {
        let Some((x, y)) = queue.dequeue() else {
            break;
        };
        points[filled] = pixel_point(heightmap, x, y);
        filled += 1;
    }
}

/// `Math.Clamp(float, float, float)`.
fn dotnet_clamp(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        return min;
    }
    if value > max {
        return max;
    }
    value
}
