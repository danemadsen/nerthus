//! The stamp scattering (`Scatters` and `Noises.Scatter`).

use nerthus_engine::Map;

use crate::masking;
use crate::random::XorShift64Star;
use crate::transformer::{self, AnchorPosition, BlendMode, TransformQuality};

/// `BombDistribution`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BombDistribution {
    Random,
    Grid,
}

/// Settings of [`bomb`] (`Scatters.Bomb`).
#[derive(Clone, Copy, Debug)]
pub struct BombSettings {
    pub iterations: i32,
    pub multiplier: f32,
    pub size: (f32, f32),
    pub size_distribution: f32,
    pub rotation: (f32, f32),
    pub maintain_proportions: bool,
    pub variable_height: bool,
    pub height: (f32, f32),
    pub fade_edges: bool,
    pub seed: i32,
    pub jitter: f32,
    pub distribution: BombDistribution,
    pub mode: BlendMode,
    pub quality: TransformQuality,
}

/// `Scatters.GetGridPositions`: a jittered square grid of at least
/// `count` cells.
pub fn grid_positions(count: i32, jitter: f32, seed: i32) -> Vec<(f32, f32)> {
    let side = (count as f64).sqrt().ceil() as i32;
    let mut random = XorShift64Star::new(seed);
    let mut positions = Vec::with_capacity((side * side).max(0) as usize);
    for row in 0..side {
        for column in 0..side {
            let x = (column as f32 + 0.5 + (random.next_float() - 0.5) * jitter) / side as f32;
            let y = (row as f32 + 0.5 + (random.next_float() - 0.5) * jitter) / side as f32;
            positions.push((x, y));
        }
    }
    positions
}

/// `1 / (e^(10·distribution + 10⁻⁶) − 1)`: normalises the size skew.
fn size_normalizer(distribution: f32) -> f32 {
    1.0 / (crate::maths::exp(distribution * 10.0 + 1e-6) - 1.0)
}

/// A random size between `low` and `high`, skewed towards `low` by
/// `distribution`.
fn skewed_size(random: &mut XorShift64Star, low: f32, high: f32, distribution: f32, normalizer: f32) -> f32 {
    let mut size = random.next_float();
    size = (crate::maths::exp(size * distribution * 10.0 + 1e-6) - 1.0) * normalizer;
    low + (high - low) * size
}

/// The stamps and blend mask that edge fading calls for.
fn faded_stamps(stamps: &mut [Map], fade_edges: bool, mode: BlendMode, resolution: nerthus_engine::Resolution) -> Option<Map> {
    if !fade_edges {
        return None;
    }
    if mode != BlendMode::Blend {
        for stamp in stamps.iter_mut() {
            *stamp = masking::mask_by_square_edge(stamp, 0.7);
        }
        return None;
    }
    Some(masking::square_edge_mask(resolution, 0.9))
}

/// `Scatters.Bomb`: stamps dropped at random (or grid) positions with
/// random size, rotation and height, avoiding `inhibit` above 0.5, added
/// onto `data`.
pub fn bomb(data: &Map, stamps: &mut [Map], inhibit: Option<&Map>, settings: &BombSettings) -> Map {
    let normalizer = size_normalizer(settings.size_distribution);
    let mask = faded_stamps(stamps, settings.fade_edges, settings.mode, data.resolution());
    let mut scattered = Map::new(data.resolution(), data.channel_count());
    let mut random = XorShift64Star::new(settings.seed);
    let mut count = (settings.iterations as f32 * settings.multiplier) as i32;
    let grid = if settings.distribution == BombDistribution::Grid {
        let positions = grid_positions(count, settings.jitter, settings.seed);
        count = positions.len() as i32;
        positions
    } else {
        Vec::new()
    };
    let width = data.width() as f32;
    let height = data.height() as f32;
    for index in 0..count {
        let stamp = &stamps[random.next_range(0, stamps.len() as i32) as usize];
        let mut position = if settings.distribution == BombDistribution::Random {
            (random.next_float(), random.next_float())
        } else {
            grid[index as usize]
        };
        if let Some(inhibit) = inhibit {
            while inhibit.get((position.0 * width) as i32 as usize, (position.1 * height) as i32 as usize) >= 0.5 {
                position = (random.next_float(), random.next_float());
            }
        }
        let mut lift = if settings.variable_height {
            random.next_float_range(settings.height.0, settings.height.1)
        } else {
            random.next_float();
            1.0
        };
        let size = skewed_size(&mut random, settings.size.0, settings.size.1, settings.size_distribution, normalizer);
        if settings.maintain_proportions {
            lift *= size;
        }
        let rotation = 360.0 * random.next_float_range(settings.rotation.0, settings.rotation.1);
        transformer::multi_transform(stamp, &mut scattered, position, (size, size), rotation, lift, settings.mode, settings.quality, mask.as_ref(), true, AnchorPosition::Center);
    }
    data.add(&scattered)
}

/// `Noises.ScatterInstance`.
#[derive(Clone, Copy, Debug)]
pub struct ScatterInstance {
    pub position: (f32, f32),
    pub hash: i32,
    pub scale: (f32, f32),
}

/// FNV-1a over three integers.
fn hash3(a: i32, b: i32, c: i32) -> u32 {
    let first = (-2128831035i32 ^ a).wrapping_mul(16777619);
    let second = (first ^ b).wrapping_mul(16777619);
    (second ^ c).wrapping_mul(16777619) as u32
}

/// `Noises.Scatter`: `density` points per unit cell of a `scale`d,
/// offset unit square, with the cells' own seeds, kept within one object
/// size of the square.
pub fn scatter(density: i32, scale: f32, offset: (f32, f32), seed: i32) -> Vec<ScatterInstance> {
    let half = 0.5f32;
    let to_grid = |corner: f32, shift: f32| (corner - half) / scale + half + shift;
    let top_left = (to_grid(0.0, offset.0), to_grid(0.0, offset.1));
    let bottom_right = (to_grid(1.0, offset.0), to_grid(1.0, offset.1));
    let first_x = top_left.0 as i32;
    let first_y = top_left.1 as i32;
    let last_x = (bottom_right.0 - 1e-7) as i32;
    let last_y = (bottom_right.1 - 1e-7) as i32;
    let span = (bottom_right.0 - top_left.0, bottom_right.1 - top_left.1);
    let scale_multiplier = (scale / span.0, scale / span.1);
    let mut instances = Vec::new();
    for row in first_y - 1..=last_y + 1 {
        for column in first_x - 1..=last_x + 1 {
            let mut random = XorShift64Star::new(hash3(column, row, seed) as i32);
            for _ in 0..density {
                let x = random.next_float() + column as f32;
                let y = random.next_float() + row as f32;
                let mut position = (x - top_left.0, y - top_left.1);
                position = (position.0 / span.0, position.1 / span.1);
                position = (position.0 - half, position.1 - half);
                position = (position.0 + half, position.1 + half);
                let hash = random.next_int();
                let inside = position.0 > 0.0 - 1.0 && position.0 < 1.0 + 1.0 && position.1 > 0.0 - 1.0 && position.1 < 1.0 + 1.0;
                if inside {
                    instances.push(ScatterInstance {
                        position,
                        hash,
                        scale: scale_multiplier,
                    });
                }
            }
        }
    }
    instances
}
