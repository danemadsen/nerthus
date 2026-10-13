//! The thermal shaper (`thermal_shaper::ThermalShaper` in the SYCL
//! kernels, exported as `ThermalShaper_Process`): reshapes
//! slopes towards a talus-like profile, coarse to fine over a Laplacian
//! pyramid. Each pass moves every pixel by its curvature against its slope,
//! never further than its steepest drop; the result is blended with the
//! input by the influence.

use nerthus_engine::Map;
use rayon::prelude::*;

use crate::flow::{self, Plane};
use crate::{gpu, sycl, x86};

/// The parameters of `ThermalShaper_Process` (`ThermalShaper.Parameters`).
pub struct ThermalShaperSettings {
    /// The terrain's width in metres.
    pub terrain_width: f32,
    /// The terrain's height in metres.
    pub terrain_height: f32,
    pub scale: f32,
    pub influence: f32,
    pub shape: f32,
    pub microdetail: f32,
}

/// 0.7071, not 1/√2, as the kernels have it.
#[allow(clippy::approx_constant)]
const DIAGONAL: f32 = 0.7071;
/// The curvature's weights for the sides and the corners.
const SIDE_WEIGHT: f32 = f32::from_bits(0x3e15_f61a);
const CORNER_WEIGHT: f32 = f32::from_bits(0x3dd4_13cd);
/// log2(1.5).
const LOG2_THREE_HALVES: f32 = 0.584_962_5;

/// UCRT's float maths, taken as the double functions rounded.
fn ucrt_exp2f(x: f32) -> f32 {
    (x as f64).exp2() as f32
}

fn ucrt_log2f(x: f32) -> f32 {
    (x as f64).log2() as f32
}

fn ucrt_tanhf(x: f32) -> f32 {
    (x as f64).tanh() as f32
}

fn ucrt_powf(x: f32, y: f32) -> f32 {
    (x as f64).powf(y as f64) as f32
}

/// One level of `thermal_shaper::LaplacianLevel`.
struct Level {
    heights: Plane,
    /// What this level loses to the next coarser one.
    detail: Plane,
    intensity: Option<Plane>,
    /// Counted from the coarsest level.
    index: i32,
}

/// The values one pass of `ThermalShaper::process_level` reads.
struct Pass {
    /// Pixels across per unit of height: scales the gradient to a slope.
    scale: f32,
    /// How strongly curvature counts (more on finer levels).
    growth: f32,
    /// The pass's share of the shape.
    shape: f32,
    /// Height per pixel across: scales the change back to height.
    cell: f32,
}

/// One pass over a level: each pixel moves by its softened curvature
/// against its slope, no further than its steepest drop.
fn shape_pass(source: &Plane, target: &mut Plane, intensity: Option<&Plane>, pass: &Pass) {
    let width = source.width;
    let height = source.height;
    target.data.par_chunks_mut(width).enumerate().for_each(|(y, out_row)| {
        let top_row = source.row(y.max(1) - 1);
        let row = source.row(y);
        let bottom_row = source.row((y + 1).min(height - 1));
        for (x, out) in out_row.iter_mut().enumerate() {
            let left = x.max(1) - 1;
            let right = (x + 1).min(width - 1);
            let (tl, t, tr) = (top_row[left], top_row[x], top_row[right]);
            let (l, c, r) = (row[left], row[x], row[right]);
            let (bl, b, br) = (bottom_row[left], bottom_row[x], bottom_row[right]);

            let diagonal = br + (tr - (tl + bl));
            let across = (r - l) * 10.0;
            let sum_x = 3.0f32.mul_add(diagonal, across);
            let top_corners = tl + tr;
            let bottom_corners = br + bl;
            let vertical = top_corners - bottom_corners;
            let down = (t - b) * 10.0;
            let sum_y = 3.0f32.mul_add(vertical, down);
            let gradient_x = (pass.scale * 0.03125) * sum_x;
            let gradient_y = (pass.scale * -0.03125) * sum_y;
            let length_squared = gradient_y.mul_add(gradient_y, gradient_x * gradient_x);
            let slope = length_squared.sqrt();
            let softness = if length_squared > 0.0 {
                let eased = ucrt_powf(slope * 0.1, DIAGONAL) + 1.0;
                1.0 - 1.0 / eased
            } else {
                0.0
            };

            let corners = bottom_corners + top_corners;
            let sides = (t + l) + (b + r);
            let curvature = (-sides).mul_add(SIDE_WEIGHT, c);
            let curvature = corners.mul_add(CORNER_WEIGHT, -curvature);
            let weight = pass.growth * pass.scale;
            let excess = x86::minss(curvature.mul_add(weight, -slope), 0.0);
            let mut change = softness * excess;

            let corner = x86::maxss(x86::maxss(c - tl, c - tr), x86::maxss(c - bl, c - br));
            let straight = x86::maxss(x86::maxss(c - t, c - l), x86::maxss(c - r, c - b));
            let steepest = x86::maxss(corner * DIAGONAL, straight);
            let floor = steepest * -pass.scale;

            change *= pass.shape;
            if let Some(intensity) = intensity {
                change *= intensity.data[y * width + x];
            }
            *out = x86::maxss(change, floor).mul_add(pass.cell, c);
        }
    });
}

/// The passes `ThermalShaper::process_level` gives a level, and what
/// they share.
pub(crate) struct LevelPlan {
    /// Passes to run; the last is scaled by `remainder`.
    pub(crate) count: i32,
    pub(crate) remainder: f32,
    pub(crate) scale: f32,
    pub(crate) growth: f32,
    pub(crate) cell: f32,
}

/// The plan for the level `index` levels finer than the coarsest,
/// `columns` wide.
pub(crate) fn plan_level(index: i32, columns: usize, fraction: f32, budget: f32, settings: &ThermalShaperSettings) -> LevelPlan {
    let multiplier = if index > 0 { 1.0 } else { fraction };
    let shifted = x86::maxss(0.0, index as f32 + (-1.0 + fraction));
    let iterations = (multiplier * budget) * ucrt_exp2f(-shifted);
    let columns = columns as i32 as f32;
    let cell = settings.terrain_height / (columns * settings.terrain_width);
    let growth_base = x86::maxss(0.0, (index as f32 + -1.0) + fraction);
    let growth = ucrt_exp2f(growth_base * LOG2_THREE_HALVES);
    let count = x86::cvttss2si(iterations.ceil());
    let remainder = iterations - iterations.floor();
    let scale = 1.0 / cell;
    LevelPlan { count, remainder, scale, growth, cell }
}

/// `ThermalShaper::process_level`: the level's passes, fewer on finer
/// levels, the last scaled by the fraction of a pass left. They ping-pong
/// between the level's plane and a scratch one, and the level keeps what
/// its own plane last received: after an odd number of passes, the
/// second-to-last.
fn process_level(level: &mut Level, fraction: f32, budget: f32, shape: f32, settings: &ThermalShaperSettings) {
    let plan = plan_level(level.index, level.heights.width, fraction, budget, settings);
    let count = plan.count;
    if count <= 0 {
        return;
    }

    let mut source = level.heights.clone();
    let mut target = Plane::zeros(source.width, source.height);
    for pass in 0..count {
        let weight = if pass < count - 1 { 1.0 } else { plan.remainder };
        let settings = Pass { scale: plan.scale, growth: plan.growth, shape: weight * shape, cell: plan.cell };
        shape_pass(&source, &mut target, level.intensity.as_ref(), &settings);
        std::mem::swap(&mut source, &mut target);
    }
    let level_plane_last_written = if count % 2 == 0 { source } else { target };
    level.heights = level_plane_last_written;
}

/// `LaplacianLevel::reconstruct_previous_level`: the finer level from the
/// coarse one and its own detail, kept by the microdetail preservation.
fn reconstruct(coarse: &Plane, fine: &mut Level, microdetail: f32) {
    let width = fine.heights.width;
    let detail = &fine.detail;
    fine.heights.data.par_chunks_mut(width).enumerate().for_each(|(y, out_row)| {
        for (x, out) in out_row.iter_mut().enumerate() {
            let kept = microdetail * detail.data[y * width + x];
            *out = flow::upsample_sum(coarse, x, y).mul_add(0.0625, kept);
        }
    });
}

/// How the pyramid is laid out: the fraction of an octave it spans and how
/// many levels it has.
pub(crate) fn pyramid_shape(width: usize, settings: &ThermalShaperSettings) -> (f32, i32) {
    let mut octaves = ucrt_log2f(width as i32 as f32 * settings.scale / settings.terrain_width);
    if !(octaves >= 1.0) {
        octaves = ucrt_tanhf(octaves + -1.0) + 1.0;
    }
    let fraction = octaves - octaves.floor();
    let level_count = x86::cvttss2si(octaves.ceil());
    (fraction, level_count)
}

/// The pass budget the levels share out: 64² over the levels' total
/// weight, which shrinks geometrically from the finest.
pub(crate) fn pass_budget(levels: usize, shape: f32) -> f32 {
    let base_budget = 64.0f32;
    let mut total = base_budget;
    if levels > 1 {
        let ratio = 0.25f32.mul_add(shape, 0.25);
        let scaled = ratio * base_budget;
        let mut power = 1.0f32;
        for _ in 1..levels {
            total = scaled.mul_add(power, total);
            power *= ratio;
        }
    }
    (base_budget * base_budget) / total
}

/// Whether the shaper works on the negated heights (a shape of zero or
/// more), and the shape it then uses.
pub(crate) fn orientation(shape: f32) -> (bool, f32) {
    let negated = !(0.0 > shape);
    if negated { (true, shape) } else { (false, -shape) }
}

/// `ThermalShaper_Process`: `heights` reshaped, `intensity` scaling the
/// change per pixel. Runs on the GPU when one is active (see [`sycl`]).
pub fn process(heights: &Map, intensity: Option<&Map>, settings: &ThermalShaperSettings) -> Map {
    let gpu = |device: &nerthus_compute::GpuDevice| gpu::thermal_shaper::process(device, heights, intensity, settings);
    let cpu = || process_cpu(heights, intensity, settings);
    sycl::run("ThermalShaper_Process", heights.width(), sycl::PREFERRED, gpu, cpu)
}

/// [`process`] on the CPU.
pub fn process_cpu(heights: &Map, intensity: Option<&Map>, settings: &ThermalShaperSettings) -> Map {
    let resolution = heights.resolution();
    let width = resolution.x;
    let height = resolution.y;
    let original = heights.channel(0);
    let plane = |data: &[f32]| Plane { width, height, data: data.to_vec() };

    let mut finest = plane(original);
    let (negated, shape) = orientation(settings.shape);
    if negated {
        for value in finest.data.iter_mut() {
            *value = -*value;
        }
    }
    let intensity = intensity.map(|map| plane(map.channel(0)));

    let (fraction, level_count) = pyramid_shape(width, settings);

    let finest = Level { detail: Plane::zeros(width, height), heights: finest, intensity, index: 0 };
    let mut pyramid = vec![finest];
    for _ in 1..level_count.max(1) {
        let parent = pyramid.last_mut().unwrap();
        let heights = flow::downsample_usm(&parent.heights);
        let intensity = parent.intensity.as_ref().map(flow::downsample_usm);
        parent.detail = flow::detail(&parent.heights, &heights);
        let detail = Plane::zeros(heights.width, heights.height);
        for finer in pyramid.iter_mut() {
            finer.index += 1;
        }
        pyramid.push(Level { heights, detail, intensity, index: 0 });
    }

    let budget = pass_budget(pyramid.len(), shape);

    while pyramid.len() > 1 {
        let mut level = pyramid.pop().unwrap();
        process_level(&mut level, fraction, budget, shape, settings);
        let parent = pyramid.last_mut().unwrap();
        reconstruct(&level.heights, parent, settings.microdetail);
    }
    let mut finest = pyramid.pop().unwrap();
    process_level(&mut finest, fraction, budget, shape, settings);

    let blended: Vec<f32> = finest
        .heights
        .data
        .par_iter()
        .zip(original.par_iter())
        .map(|(&result, &before)| {
            if negated {
                (-(before + result)).mul_add(settings.influence, before)
            } else {
                (result - before).mul_add(settings.influence, before)
            }
        })
        .collect();
    Map::from_data(resolution, blended)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_keep_their_bits() {
        assert_eq!(DIAGONAL.to_bits(), 0x3f35_0481);
        assert_eq!(LOG2_THREE_HALVES.to_bits(), 0x3f15_c01a);
        assert_eq!(0.1f32.to_bits(), 0x3dcc_cccd);
    }
}
