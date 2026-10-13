//! The flow simulation (`flow::Flow::process` in the SYCL kernels,
//! exported as `FlowSimulation`): pits are filled through a Laplacian
//! pyramid, then particles seeded on every pixel (or where a precipitation
//! map allows) roll downhill in batches, and each batch's visit counts are
//! folded into the flow map while raising the terrain they ran over.

use std::sync::atomic::{AtomicU32, Ordering};

use nerthus_engine::{Map, Resolution};
use rayon::prelude::*;

use crate::{gpu, sycl, x86};

/// The parameters of `FlowSimulation` (`Flow.Parameters`).
pub struct FlowSettings {
    pub flow_length: f32,
    pub flow_volume: f32,
    pub seed: i32,
}

/// Generators whose state would start at zero use this instead.
pub(crate) const ZERO_SEED: u32 = 0xA235_1342;
pub(crate) const XORSHIFT_MULTIPLIER: u32 = 0x2545_F381;

/// 2⁻³², turning a 32-bit random number into [0, 1].
pub(crate) const UNIT_SCALE: f32 = 2.328_306_4e-10;

/// 0.8 · 2⁻³²: particles start within [0.1, 0.9) of their pixel.
const JITTER_SCALE: f32 = 1.862_645_2e-10;
const JITTER_OFFSET: f32 = 0.1;

/// Fill-pits passes on each pyramid level.
pub(crate) const FILL_PASSES: usize = 20;

/// Detail is only restored where less than this much filling reached.
const FILLED_THRESHOLD: f32 = 0.001;

/// Particles are traced in this many batches.
pub(crate) const BATCHES: i32 = 32;

/// At most this many particles (over all batches) get the long trace.
pub(crate) const LONG_TRACE_PARTICLES: i32 = 0x40000;

/// The step count of the short traces.
pub(crate) const SHORT_STEPS: u32 = 16;

/// The unit directions particles are nudged along, `(cos, sin)` of
/// `k · 2π/32` as the kernel's table has them.
pub(crate) const DIRECTION_BITS: [[u32; 2]; 32] = [
    [0x3F80_0000, 0x0000_0000],
    [0x3F7B_14BE, 0x3E47_C5C2],
    [0x3F6C_835E, 0x3EC3_EF15],
    [0x3F54_DB31, 0x3F0E_39DA],
    [0x3F35_04F3, 0x3F35_04F3],
    [0x3F0E_39D9, 0x3F54_DB32],
    [0x3EC3_EF14, 0x3F6C_835E],
    [0x3E47_C5B0, 0x3F7B_14BF],
    [0xB33B_BD2E, 0x3F80_0000],
    [0xBE47_C5C2, 0x3F7B_14BE],
    [0xBEC3_EF18, 0x3F6C_835E],
    [0xBF0E_39DB, 0x3F54_DB30],
    [0xBF35_04F3, 0x3F35_04F3],
    [0xBF54_DB32, 0x3F0E_39D9],
    [0xBF6C_835F, 0x3EC3_EF12],
    [0xBF7B_14BF, 0x3E47_C5C0],
    [0xBF80_0000, 0xB3BB_BD2E],
    [0xBF7B_14BE, 0xBE47_C5C6],
    [0xBF6C_835E, 0xBEC3_EF14],
    [0xBF54_DB30, 0xBF0E_39DA],
    [0xBF35_04F2, 0xBF35_04F4],
    [0xBF0E_39D8, 0xBF54_DB33],
    [0xBEC3_EF10, 0xBF6C_835F],
    [0xBE47_C5C8, 0xBF7B_14BE],
    [0x324C_BBB5, 0xBF80_0000],
    [0x3E47_C5C8, 0xBF7B_14BE],
    [0x3EC3_EF1A, 0xBF6C_835D],
    [0x3F0E_39DC, 0xBF54_DB2F],
    [0x3F35_04F6, 0xBF35_04F0],
    [0x3F54_DB31, 0xBF0E_39DA],
    [0x3F6C_835F, 0xBEC3_EF14],
    [0x3F7B_14BF, 0xBE47_C5BC],
];

// --- random numbers ------------------------------------------------------------------

pub(crate) fn xorshift(state: u32) -> u32 {
    let mut next = state ^ (state << 13);
    next ^= next >> 17;
    next ^ (next << 5)
}

/// The 32-bit random number a generator state gives, as a float.
pub(crate) fn random_bits(state: u32) -> f32 {
    let bits = state.wrapping_mul(XORSHIFT_MULTIPLIER).wrapping_sub(1);
    bits as f32
}

/// `std::mt19937`.
pub struct Mt19937 {
    state: [u32; 624],
    index: usize,
}

impl Mt19937 {
    pub fn new(seed: u32) -> Mt19937 {
        let mut state = [0u32; 624];
        state[0] = seed;
        for index in 1..624 {
            let previous = state[index - 1];
            let mixed = 0x6C07_8965u32.wrapping_mul(previous ^ (previous >> 30));
            state[index] = mixed.wrapping_add(index as u32);
        }
        Mt19937 { state, index: 624 }
    }

    fn twist(&mut self) {
        for index in 0..624 {
            let upper = self.state[index] & 0x8000_0000;
            let lower = self.state[(index + 1) % 624] & 0x7FFF_FFFF;
            let combined = upper | lower;
            let mut next = self.state[(index + 397) % 624] ^ (combined >> 1);
            if combined & 1 != 0 {
                next ^= 0x9908_B0DF;
            }
            self.state[index] = next;
        }
        self.index = 0;
    }

    pub fn next_u32(&mut self) -> u32 {
        if self.index >= 624 {
            self.twist();
        }
        let mut value = self.state[self.index];
        self.index += 1;
        value ^= value >> 11;
        value ^= (value << 7) & 0x9D2C_5680;
        value ^= (value << 15) & 0xEFC6_0000;
        value ^ (value >> 18)
    }
}

/// MSVC's `_Rng_from_urng<ptrdiff_t, mt19937>`: an unbiased index in
/// `[0, count)` built from 32-bit draws.
pub fn random_index(generator: &mut Mt19937, count: u64) -> u64 {
    loop {
        let mut value = 0u64;
        let mut mask = 0u64;
        while mask < count - 1 {
            value = (value << 32) | generator.next_u32() as u64;
            mask = (mask << 32) | 0xFFFF_FFFF;
        }
        let unbiased = value / count < mask / count || mask % count == count - 1;
        if unbiased {
            return value % count;
        }
    }
}

/// MSVC's `std::shuffle`.
pub fn shuffle<T>(items: &mut [T], generator: &mut Mt19937) {
    for index in 1..items.len() {
        let target = random_index(generator, index as u64 + 1) as usize;
        if target != index {
            items.swap(index, target);
        }
    }
}

// --- pit filling ---------------------------------------------------------------------

/// A row-major plane of floats.
#[derive(Clone)]
pub(crate) struct Plane {
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) data: Vec<f32>,
}

impl Plane {
    pub(crate) fn zeros(width: usize, height: usize) -> Plane {
        Plane { width, height, data: vec![0.0; width * height] }
    }

    pub(crate) fn row(&self, y: usize) -> &[f32] {
        &self.data[y * self.width..(y + 1) * self.width]
    }
}

/// Whether column `x` of a kernel launch `width` wide is one of the eight
/// lanes AdaptiveCpp's vectorised loop computes together: rows are cut
/// into chunks of 1024 and each chunk's whole groups of eight are
/// vectorised (launches three wide or narrower are never vectorised).
pub(crate) fn vector_lane(x: usize, width: usize) -> bool {
    if width <= 3 {
        return false;
    }
    let chunk_start = x / 1024 * 1024;
    let chunk_length = (chunk_start + 1024).min(width) - chunk_start;
    let vectorised = chunk_length & !7;
    x - chunk_start < vectorised
}

/// `LaplacianLevel::create_subsequent_level`'s first kernel: the level at
/// half resolution, a separable [1 3 3 1]/8 filter over 4×4 fine pixels
/// (the vectorised lanes add in a different order).
pub(crate) fn downsample(fine: &Plane) -> Plane {
    let width = fine.width >> 1;
    let height = fine.height >> 1;
    let last_x = fine.width as i32 - 1;
    let last_y = fine.height as i32 - 1;
    let mut coarse = Plane::zeros(width, height);
    coarse.data.par_chunks_mut(width).enumerate().for_each(|(y, out_row)| {
        let y = y as i32;
        let row_a = fine.row((2 * y).max(1) as usize - 1);
        let row_b = fine.row((2 * y) as usize);
        let row_c = fine.row((2 * y + 1).min(last_y) as usize);
        let row_d = fine.row((2 * y + 2).min(last_y) as usize);
        for (x, out) in out_row.iter_mut().enumerate() {
            let column = x as i32;
            let a = (2 * column).max(1) as usize - 1;
            let b = (2 * column) as usize;
            let c = (2 * column + 1).min(last_x) as usize;
            let d = (2 * column + 2).min(last_x) as usize;

            let (edges, centre, corners) = if vector_lane(x, width) {
                let edges_top = (((row_a[b] + row_a[c]) + row_b[d]) + row_c[d]) + row_d[c];
                let edges_bottom = row_d[b] + (row_b[a] + row_c[a]);
                let centre = (row_c[c] + row_b[c]) + (row_b[b] + row_c[b]);
                let corners = (row_a[d] + row_a[a]) + row_d[a];
                (edges_top + edges_bottom, centre, corners)
            } else {
                let mut edges = row_a[c] + row_a[b];
                edges += row_b[a];
                edges += row_b[d];
                edges += row_c[a];
                edges += row_c[d];
                edges += row_d[b];
                edges += row_d[c];
                let centre = ((row_b[c] + row_b[b]) + row_c[b]) + row_c[c];
                let corners = (row_a[d] + row_a[a]) + row_d[a];
                (edges, centre, corners)
            };
            let weighted = centre.mul_add(9.0, corners) + row_d[d];
            let total = edges.mul_add(3.0, weighted);
            *out = total * 0.015625;
        }
    });
    coarse
}

/// Whether column `x` of a launch `width` wide is an AVX lane of the
/// raw-pointer `downsample_magic`: rows are cut into chunks of 1024, a
/// chunk is vectorised only when longer than eight, and its last one to
/// eight columns always go to the scalar tail.
fn usm_vector_lane(x: usize, width: usize) -> bool {
    if width <= 3 {
        return false;
    }
    let chunk_start = x / 1024 * 1024;
    let chunk_length = (chunk_start + 1024).min(width) - chunk_start;
    if chunk_length < 9 {
        return false;
    }
    let tail = if chunk_length.is_multiple_of(8) { 8 } else { chunk_length % 8 };
    x - chunk_start < chunk_length - tail
}

/// `resampling::downsample_magic` over raw pointers (`usm_ptr`), as the
/// thermal shaper calls it: the same [1 3 3 1]/8 filter as `downsample`,
/// its AVX lanes adding in yet another order.
pub(crate) fn downsample_usm(fine: &Plane) -> Plane {
    let width = fine.width >> 1;
    let height = fine.height >> 1;
    let last_x = fine.width as i32 - 1;
    let last_y = fine.height as i32 - 1;
    let mut coarse = Plane::zeros(width, height);
    coarse.data.par_chunks_mut(width).enumerate().for_each(|(y, out_row)| {
        let y = y as i32;
        let row_a = fine.row((2 * y).max(1) as usize - 1);
        let row_b = fine.row((2 * y) as usize);
        let row_c = fine.row((2 * y + 1).min(last_y) as usize);
        let row_d = fine.row((2 * y + 2).min(last_y) as usize);
        for (x, out) in out_row.iter_mut().enumerate() {
            let column = x as i32;
            let a = (2 * column).max(1) as usize - 1;
            let b = (2 * column) as usize;
            let c = (2 * column + 1).min(last_x) as usize;
            let d = (2 * column + 2).min(last_x) as usize;

            let (edges, centre, corners) = if usm_vector_lane(x, width) {
                let top = (row_a[c] + row_a[b]) + (row_b[d] + row_b[a]);
                let bottom = (row_c[d] + row_c[a]) + row_d[b];
                let edges = (top + bottom) + row_d[c];
                let centre = row_c[c] + (row_b[c] + (row_c[b] + row_b[b]));
                let corners = (row_a[d] + row_a[a]) + row_d[a];
                (edges, centre, corners)
            } else {
                let mut edges = row_a[c] + row_a[b];
                edges += row_b[a];
                edges += row_b[d];
                edges += row_c[a];
                edges += row_c[d];
                edges += row_d[b];
                edges += row_d[c];
                let centre = ((row_b[c] + row_b[b]) + row_c[b]) + row_c[c];
                let corners = (row_a[d] + row_a[a]) + row_d[a];
                (edges, centre, corners)
            };
            let weighted = centre.mul_add(9.0, corners) + row_d[d];
            let total = edges.mul_add(3.0, weighted);
            *out = total * 0.015625;
        }
    });
    coarse
}

/// The coarse neighbour indices of fine coordinate `x`: its own coarse
/// cell and the nearer of the two either side, clamped to `last`.
pub(crate) fn coarse_neighbours(x: usize, last: i32) -> (usize, usize) {
    let own = (x / 2) as i32;
    let side = own + (x as i32 & 1) * 2 - 1;
    let side = side.max(0).min(last);
    (own as usize, side as usize)
}

/// `9·own + diagonal + 3·(the two sides)` of the bilinear 2× upsample, in
/// the kernel's order; `scale` the final factor (1/16).
pub(crate) fn upsample_sum(coarse: &Plane, x: usize, y: usize) -> f32 {
    let (own_x, side_x) = coarse_neighbours(x, coarse.width as i32 - 1);
    let (own_y, side_y) = coarse_neighbours(y, coarse.height as i32 - 1);
    let own_row = coarse.row(own_y);
    let side_row = coarse.row(side_y);
    let sides = side_row[own_x] + own_row[side_x];
    let centre = own_row[own_x].mul_add(9.0, side_row[side_x]);
    sides.mul_add(3.0, centre)
}

/// The second kernel: the detail the coarse level loses, `fine −
/// upsample(coarse)`.
pub(crate) fn detail(fine: &Plane, coarse: &Plane) -> Plane {
    let mut detail = Plane::zeros(fine.width, fine.height);
    detail.data.par_chunks_mut(fine.width).enumerate().for_each(|(y, out_row)| {
        for (x, out) in out_row.iter_mut().enumerate() {
            let (own_x, side_x) = coarse_neighbours(x, coarse.width as i32 - 1);
            let (own_y, side_y) = coarse_neighbours(y, coarse.height as i32 - 1);
            let own_row = coarse.row(own_y);
            let side_row = coarse.row(side_y);
            let sides = side_row[own_x] + own_row[side_x];
            let centre = 9.0f32.mul_add(own_row[own_x], side_row[side_x]);
            let difference = sides.mul_add(-3.0, -centre);
            *out = difference.mul_add(0.0625, fine.data[y * fine.width + x]);
        }
    });
    detail
}

/// One `Flow::fill_pits` pass: a pixel no higher than the second lowest of
/// its neighbours is raised to it (and flagged).
fn fill_pits(heights: &Plane, flags: &mut Plane) -> Plane {
    let width = heights.width;
    let height = heights.height;
    let mut filled = Plane::zeros(width, height);
    let raised_rows: Vec<Vec<usize>> = filled
        .data
        .par_chunks_mut(width)
        .enumerate()
        .map(|(y, out_row)| {
            let mut raised = Vec::new();
            let first_y = y.saturating_sub(1);
            let last_y = (y + 1).min(height - 1);
            for (x, out) in out_row.iter_mut().enumerate() {
                let first_x = x.saturating_sub(1);
                let last_x = (x + 1).min(width - 1);
                let mut lowest = f32::MAX;
                let mut second = f32::MAX;
                for neighbour_y in first_y..=last_y {
                    let row = heights.row(neighbour_y);
                    for (offset, &value) in row[first_x..=last_x].iter().enumerate() {
                        let neighbour_x = first_x + offset;
                        if neighbour_y == y && neighbour_x == x {
                            continue;
                        }
                        if value < lowest {
                            second = lowest;
                            lowest = value;
                        } else {
                            second = x86::minss(value, second);
                        }
                    }
                }
                let floor = if second == f32::MAX { lowest } else { second };
                let own = heights.data[y * width + x];
                if floor < own {
                    *out = own;
                } else {
                    *out = floor;
                    raised.push(x);
                }
            }
            raised
        })
        .collect();
    for (y, raised) in raised_rows.iter().enumerate() {
        for &x in raised {
            flags.data[y * width + x] = 1.0;
        }
    }
    filled
}

/// `LaplacianLevel::reconstruct`: the finer level rebuilt from the filled
/// coarse one, keeping its detail only where no filling reached.
fn reconstruct(coarse: &Plane, coarse_flags: &Plane, detail: &Plane) -> Plane {
    let mut fine = Plane::zeros(detail.width, detail.height);
    fine.data.par_chunks_mut(detail.width).enumerate().for_each(|(y, out_row)| {
        for (x, out) in out_row.iter_mut().enumerate() {
            let heights = upsample_sum(coarse, x, y);
            let filled = upsample_sum(coarse_flags, x, y) * 0.0625;
            let kept = if filled < FILLED_THRESHOLD { detail.data[y * detail.width + x] } else { 0.0 };
            *out = 0.0625f32.mul_add(heights, kept);
        }
    });
    fine
}

/// One level of the pit-filling pyramid.
struct Level {
    heights: Plane,
    detail: Plane,
    flags: Plane,
}

/// The levels of the pit-filling pyramid for a map `width` wide: down to
/// 256 pixels wide.
pub(crate) fn fill_level_count(width: usize) -> i32 {
    x86::cvttss2si((width as i32 as f32 * 0.007_812_5).log2())
}

/// `Flow::process`'s pit filling: a Laplacian pyramid down to 256 pixels
/// wide (none for 256 and smaller), pits filled on each coarse level, and
/// the finer levels rebuilt from them.
pub(crate) fn fill_pyramid(heights: Plane) -> Plane {
    let level_count = fill_level_count(heights.width);
    let empty = Plane::zeros(0, 0);
    let mut pyramid = vec![Level { heights, detail: empty.clone(), flags: empty.clone() }];
    for _ in 1..level_count {
        let parent = pyramid.last_mut().unwrap();
        let coarse = downsample(&parent.heights);
        parent.detail = detail(&parent.heights, &coarse);
        let flags = Plane::zeros(coarse.width, coarse.height);
        pyramid.push(Level { heights: coarse, detail: empty.clone(), flags });
    }

    while pyramid.len() > 1 {
        let mut level = pyramid.pop().unwrap();
        for _ in 0..FILL_PASSES {
            level.heights = fill_pits(&level.heights, &mut level.flags);
        }
        let parent = pyramid.last_mut().unwrap();
        parent.heights = reconstruct(&level.heights, &level.flags, &parent.detail);
    }
    pyramid.pop().unwrap().heights
}

// --- particles -----------------------------------------------------------------------

/// Particles spawn on every pixel, or with the precipitation map's
/// probability, at a jittered spot inside it.
pub(crate) fn seed_particles(resolution: Resolution, precipitation: Option<&[f32]>, seed: i32) -> Vec<[f32; 2]> {
    let mut state = if seed != 0 { seed as u32 } else { ZERO_SEED };
    let mut particles = Vec::new();
    for y in 0..resolution.y {
        let row_offset = y as i32 as f32 + JITTER_OFFSET;
        for x in 0..resolution.x {
            if let Some(precipitation) = precipitation {
                state = xorshift(state);
                let chance = UNIT_SCALE * random_bits(state);
                let dry = chance >= precipitation[y * resolution.x + x];
                if dry {
                    continue;
                }
            }
            let first = xorshift(state);
            state = xorshift(first);
            let particle_y = random_bits(first).mul_add(JITTER_SCALE, row_offset);
            let column_offset = x as i32 as f32 + JITTER_OFFSET;
            let particle_x = JITTER_SCALE.mul_add(random_bits(state), column_offset);
            particles.push([particle_x, particle_y]);
        }
    }
    particles
}

/// The shuffled particles and nudge directions both simulations start
/// from: particles in MSVC `std::shuffle` order under `mt19937(seed)`,
/// directions under `mt19937(seed + 14)`.
pub(crate) fn prepare_particles(resolution: Resolution, precipitation: Option<&[f32]>, seed: i32) -> (Vec<[f32; 2]>, [[f32; 2]; 32]) {
    let mut particles = seed_particles(resolution, precipitation, seed);
    if !particles.is_empty() {
        let mut generator = Mt19937::new(seed as u32);
        shuffle(&mut particles, &mut generator);
    }
    let mut directions = DIRECTION_BITS.map(|[x, y]| [f32::from_bits(x), f32::from_bits(y)]);
    let mut direction_generator = Mt19937::new((seed as u32).wrapping_add(14));
    shuffle(&mut directions, &mut direction_generator);
    (particles, directions)
}

/// The batch weight both simulations fold a batch's visits in with:
/// `short_count` particles traced `short_cap` steps and `long_share`
/// traced `long_steps`, out of `particle_count` over `pixel_count` pixels.
pub(crate) fn batch_weight(particle_count: i32, long_share: i32, short_count: i32, short_cap: f32, long_steps: f32, pixel_count: f32) -> f32 {
    let particle_count_f = particle_count as f32;
    let crowding = (particle_count_f * 0.7) / pixel_count;
    let traced = (short_count as f32).mul_add(short_cap, long_steps * long_share as f32);
    let inverse = 1.0 / traced;
    let share = (short_count + long_share) as f32;
    let ratio = inverse * ((short_cap + long_steps) * share);
    let spread = (-particle_count_f).mul_add(inverse, ratio);
    (-0.25 - crowding).mul_add(spread, ratio)
}

/// The terrain under a particle: bilinear height and gradient around
/// `(x − ½, y − ½)` (`trunc` picks the pixels, `floor` the weights).
pub(crate) struct Sample {
    pub(crate) gradient_x: f32,
    pub(crate) gradient_y: f32,
    pub(crate) height: f32,
}

pub(crate) fn sample(heights: &Plane, x: f32, y: f32) -> Sample {
    let sample_y = y + -0.5;
    let sample_x = x + -0.5;
    let column = x86::cvttss2si(sample_x);
    let row = x86::cvttss2si(sample_y);
    let row_0 = row.max(0) as usize;
    let column_0 = column.max(0) as usize;
    let row_1 = (row + 1).min(heights.height as i32 - 1) as usize;
    let column_1 = (column + 1).min(heights.width as i32 - 1) as usize;
    let top = heights.row(row_0);
    let bottom = heights.row(row_1);
    let h00 = top[column_0];
    let h01 = top[column_1];
    let h10 = bottom[column_0];
    let h11 = bottom[column_1];

    let fraction_x = sample_x - sample_x.floor();
    let fraction_y = sample_y - sample_y.floor();
    let rest_x = 1.0 - fraction_x;
    let rest_y = 1.0 - fraction_y;
    let gradient_y = (h11 - h01).mul_add(fraction_x, rest_x * (h10 - h00));
    let gradient_x = (h11 - h10).mul_add(fraction_y, rest_y * (h01 - h00));
    let top_mix = fraction_x.mul_add(h01, rest_x * h00);
    let bottom_mix = h11.mul_add(fraction_x, rest_x * h10);
    let height = bottom_mix.mul_add(fraction_y, rest_y * top_mix);
    Sample { gradient_x, gradient_y, height }
}

/// `x · rsqrt(len²)` refined by one Newton-Raphson step, as the kernel
/// normalises vectors.
pub(crate) fn inverse_length(length_squared: f32) -> f32 {
    let estimate = x86::rsqrt(length_squared);
    let correction = (length_squared * estimate).mul_add(estimate, -3.0);
    (estimate * -0.5) * correction
}

/// One launch of `Flow::flow`: particles `start..start + count`, each
/// traced for at most `steps` steps.
struct Launch<'a> {
    heights: &'a Plane,
    particles: &'a [[f32; 2]],
    directions: &'a [[f32; 2]; 32],
    start: u32,
    seed: u32,
    steps: u32,
}

/// How a trace ends, for chaining the kernel's stale stack slot.
struct TraceEnd {
    /// The height the first uphill test compares (`None` when the trace
    /// stops before it).
    first_height: Option<f32>,
    /// The slot's value afterwards (`None`: untouched).
    slot: Option<f32>,
}

/// One particle of `Flow::flow`.
///
/// The kernel counts consecutive steps that fail to descend and stops after
/// too many; its first test compares the start height with a stack slot it
/// never initialises, which holds whatever the previous particle on that
/// thread left there. `first_uphill` is that test's outcome (resolved by
/// the caller), and `visit` is called for each pixel the particle counts.
fn trace(launch: &Launch, offset: u32, first_uphill: bool, mut visit: impl FnMut(usize, usize)) -> TraceEnd {
    let heights = launch.heights;
    let index = offset.wrapping_add(launch.start);
    let seeded = index.wrapping_add(launch.seed);
    let mut state = if seeded != 0 { seeded } else { ZERO_SEED };
    let [mut x, mut y] = launch.particles[index as usize];
    let untouched = TraceEnd { first_height: None, slot: None };

    let right = heights.width as f32 + -1.0;
    let bottom = heights.height as f32 + -1.0;
    let inside = x >= 0.0 && x < right && y >= 0.0 && y < bottom;
    if !inside {
        return untouched;
    }
    let start = sample(heights, x, y);
    let start_length = start.gradient_x * start.gradient_x;
    let start_length = start.gradient_y.mul_add(start.gradient_y, start_length);
    if !(start_length > 0.0) {
        return untouched;
    }

    state = xorshift(state);
    let uphill_limit = x86::maxss(launch.steps as f32 * 0.08, 4.0);
    let last_step = launch.steps.wrapping_sub(1) as f32;
    let scale = inverse_length(start_length);
    let mut direction_y = -start.gradient_y * scale;
    let mut direction_x = -start.gradient_x * scale;

    let mut steps_taken = 0.0f32;
    let mut uphill = 0.0f32;
    let mut first_test = true;
    let mut current_height = start.height;
    let mut previous_height = 0.0f32;
    let mut end = TraceEnd { first_height: Some(start.height), slot: None };
    loop {
        let column = x86::cvttss2si(x) as u32 as usize;
        let row = x86::cvttss2si(y) as u32 as usize;
        visit(column, row);

        x += direction_x;
        if x < 0.0 || x >= right {
            break;
        }
        y += direction_y;
        if y < 0.0 || y >= bottom {
            break;
        }
        let here = sample(heights, x, y);
        let length = here.gradient_x * here.gradient_x;
        let length = here.gradient_y.mul_add(here.gradient_y, length);
        state = xorshift(state);
        if length > 0.0 {
            let scale = inverse_length(length);
            let pick = (state.wrapping_mul(XORSHIFT_MULTIPLIER).wrapping_sub(1) & 31) as usize;
            let [nudge_x, nudge_y] = launch.directions[pick];
            let mut new_x = (-1.2f32).mul_add(direction_x, here.gradient_x * scale);
            new_x = 0.7f32.mul_add(nudge_x, -new_x);
            let mut new_y = (-1.2f32).mul_add(direction_y, here.gradient_y * scale);
            new_y = 0.7f32.mul_add(nudge_y, -new_y);
            let new_length = new_x * new_x;
            let new_length = new_y.mul_add(new_y, new_length);
            let new_scale = inverse_length(new_length);
            direction_x = new_x * new_scale;
            direction_y = new_y * new_scale;
        }

        steps_taken += 1.0;
        if steps_taken == last_step {
            break;
        }
        let descended = if first_test { !first_uphill } else { current_height < previous_height };
        first_test = false;
        if !descended {
            uphill += 1.0;
        }
        previous_height = current_height;
        current_height = here.height;
        end.slot = Some(previous_height);
        if !(uphill <= uphill_limit) {
            break;
        }
    }
    end
}

/// The slot value the first particle of a launch finds (the original reads
/// whatever earlier code left on the stack; Nerthus treats it as a start
/// from above).
const INITIAL_SLOT: f32 = f32::INFINITY;

/// Runs one `Flow::flow` launch, adding each particle's visits to
/// `counts`. Particles run in index order on one stack in the original, so
/// each one's first uphill test reads the slot the previous one left;
/// that chain is resolved first, then the traces run in parallel.
fn run_launch(launch: &Launch, count: u32, counts: &[AtomicU32]) {
    let offsets = 0..count;
    let ends: Vec<[TraceEnd; 2]> = offsets
        .into_par_iter()
        .map(|offset| {
            let descending = trace(launch, offset, false, |_, _| {});
            let ascending = trace(launch, offset, true, |_, _| {});
            [descending, ascending]
        })
        .collect();

    let mut slot = INITIAL_SLOT;
    let mut first_uphill = vec![false; count as usize];
    for (offset, outcomes) in ends.iter().enumerate() {
        let Some(first_height) = outcomes[0].first_height else {
            continue;
        };
        let uphill = !(first_height < slot);
        first_uphill[offset] = uphill;
        if let Some(value) = outcomes[uphill as usize].slot {
            slot = value;
        }
    }

    let width = launch.heights.width;
    (0..count).into_par_iter().for_each(|offset| {
        let uphill = first_uphill[offset as usize];
        trace(launch, offset, uphill, |column, row| {
            counts[row * width + column].fetch_add(1, Ordering::Relaxed);
        });
    });
}

/// The kernel that folds a batch's visit counts into the flow map and
/// raises the terrain by them. The vectorised lanes divide through
/// `RCPPS` and one Newton-Raphson step.
fn accumulate(flow: &mut Plane, terrain: &mut Plane, counts: &[AtomicU32], weight: f32, raise: f32) {
    let width = flow.width;
    let scaled_weight = weight * 0.03125;
    let flow_rows = flow.data.par_chunks_mut(width);
    let terrain_rows = terrain.data.par_chunks_mut(width);
    flow_rows.zip(terrain_rows).enumerate().for_each(|(y, (flow_row, terrain_row))| {
        for x in 0..width {
            let count = counts[y * width + x].swap(0, Ordering::Relaxed) as f32;
            let amount = count * scaled_weight;
            let damping = count.mul_add(weight, 8.0);
            let added = if vector_lane(x, width) {
                let reciprocal = x86::rcp(damping);
                let estimate = amount * reciprocal;
                let error = estimate.mul_add(damping, -amount);
                (-reciprocal).mul_add(error, estimate)
            } else {
                amount / damping
            };
            flow_row[x] += added;
            terrain_row[x] = count.mul_add(raise, terrain_row[x]);
        }
    });
}

/// `FlowSimulation`: the flow map of `heights`, with particles spawned
/// where `precipitation` allows (everywhere without one). Runs on the GPU
/// when one is active (see [`sycl`]).
pub fn simulate(heights: &Map, precipitation: Option<&Map>, settings: &FlowSettings) -> Map {
    let gpu = |device: &nerthus_compute::GpuDevice| gpu::flow::simulate(device, heights, precipitation, settings);
    let cpu = || simulate_cpu(heights, precipitation, settings);
    sycl::run("FlowSimulation", heights.width(), sycl::PREFERRED, gpu, cpu)
}

/// [`simulate`] on the CPU.
pub fn simulate_cpu(heights: &Map, precipitation: Option<&Map>, settings: &FlowSettings) -> Map {
    let resolution = heights.resolution();
    let width = resolution.x;
    let height = resolution.y;
    let source = Plane { width, height, data: heights.channel(0).to_vec() };
    let mut terrain = fill_pyramid(source);

    let precipitation_data = precipitation.map(|map| map.channel(0));
    let (particles, directions) = prepare_particles(resolution, precipitation_data, settings.seed);

    let mut flow = Plane::zeros(width, height);
    let counts: Vec<AtomicU32> = (0..width * height).map(|_| AtomicU32::new(0)).collect();

    let particle_count = particles.len() as i32;
    let batch = (particle_count >> 5) + i32::from(particle_count & 31 != 0);
    let long_share = particle_count.min(LONG_TRACE_PARTICLES) / BATCHES;
    let pixel_count = (height as i64 * width as i64) as f32;
    let raise = 1e-5 * settings.flow_volume;

    let mut start = 0i32;
    loop {
        let remaining = particle_count - start;
        if remaining <= 0 {
            break;
        }
        let this_batch = remaining.min(batch);
        let long_steps = (width as i32 as f32).mul_add(settings.flow_length, 2.0);
        let short_cap = x86::minss(long_steps, 16.0);

        let short_count = (this_batch - long_share).max(0);
        if short_count > 0 {
            let launch = Launch { heights: &terrain, particles: &particles, directions: &directions, start: start as u32, seed: settings.seed as u32, steps: SHORT_STEPS };
            run_launch(&launch, short_count as u32, &counts);
        }
        let long_count = this_batch.min(long_share);
        let long_step_count = long_steps as i64 as u32;
        let long_start = (start + short_count) as u32;
        let launch = Launch { heights: &terrain, particles: &particles, directions: &directions, start: long_start, seed: settings.seed as u32, steps: long_step_count };
        run_launch(&launch, long_count as u32, &counts);

        let weight = batch_weight(particle_count, long_share, short_count, short_cap, long_steps, pixel_count);
        accumulate(&mut flow, &mut terrain, &counts, weight, raise);

        start += batch;
    }
    Map::from_data(resolution, flow.data)
}
