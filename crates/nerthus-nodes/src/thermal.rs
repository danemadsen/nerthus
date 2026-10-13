//! The thermal erosion on the SYCL path (`thermal::Thermal` in the SYCL
//! kernels, exported as `ThermalSimulation`), behind Thermal2
//! and Sediments' fine mode. Talus erosion runs coarse to fine over a
//! Laplacian pyramid. Every pass measures each pixel's slope and wears the
//! surface into loose sediment, sums the pixel's drops to its neighbours,
//! and moves sediment down from its steeper neighbours. Near the end the
//! finest level switches to anisotropic passes, which tilt each pixel along
//! its slope.

use nerthus_engine::Map;
use rayon::prelude::*;

use crate::flow::{self, Plane};
use crate::{gpu, sycl, x86};

/// The parameters of `ThermalSimulation` (`Thermal2.Parameters`).
pub struct ThermalSettings {
    pub duration: f32,
    pub strength: f32,
    pub anisotropy: f32,
    pub talus_angle: f32,
    pub erosion_scale: f32,
    /// The terrain's width in metres.
    pub heightmap_size: f32,
    /// The terrain's height in metres.
    pub heightmap_height: f32,
    pub sediment_removal: f32,
}

/// The maps `ThermalSimulation` writes back.
pub struct ThermalMaps {
    pub heights: Map,
    pub deposition: Map,
    pub wear: Map,
}

/// The weight of a diagonal neighbour's drop (0.7071, not 1/√2).
#[allow(clippy::approx_constant)]
const DIAGONAL: f32 = 0.7071;
/// 1/180 (degrees to half turns), as the kernels round it.
const DEGREES_TO_HALF_TURNS: f32 = 0.005_555_555_7;
const MINUS_SIXTH: f32 = -0.166_666_67;
const MINUS_TWELFTH: f32 = -0.083_333_336;
/// The share of the sediment balance a pass applies.
pub(crate) const ISOTROPIC_STEP: f32 = 0.2;
pub(crate) const ANISOTROPIC_STEP: f32 = 0.14;
pub(crate) const ISOTROPIC_WEAR: f32 = 0.0044;
/// One ulp below the float nearest 0.00264.
pub(crate) const ANISOTROPIC_WEAR: f32 = f32::from_bits(0x3b2d_03d9);

/// UCRT's float maths, taken as the double functions rounded.
fn ucrt_expf(x: f32) -> f32 {
    (x as f64).exp() as f32
}

fn ucrt_log2f(x: f32) -> f32 {
    (x as f64).log2() as f32
}

/// One level of `thermal::LaplacianLevel`, its buffers in the kernels' order.
struct Level {
    heights: Plane,
    sediment: Plane,
    wear: Plane,
    next_heights: Plane,
    next_sediment: Plane,
    drops: Plane,
    /// What this level loses to the next coarser one.
    detail: Plane,
    area: Option<Plane>,
    removal: Option<Plane>,
}

impl Level {
    fn new(heights: Plane, area: Option<Plane>, removal: Option<Plane>) -> Level {
        let width = heights.width;
        let height = heights.height;
        Level {
            heights,
            sediment: Plane::zeros(width, height),
            wear: Plane::zeros(width, height),
            next_heights: Plane::zeros(width, height),
            next_sediment: Plane::zeros(width, height),
            drops: Plane::zeros(width, height),
            detail: Plane::zeros(width, height),
            area,
            removal,
        }
    }

    /// `LaplacianLevel::create_subsequent_level`: the level at half
    /// resolution (its masks too), storing the detail this one loses.
    fn coarser(&mut self) -> Level {
        let heights = flow::downsample(&self.heights);
        self.detail = flow::detail(&self.heights, &heights);
        let area = self.area.as_ref().map(flow::downsample);
        let removal = self.removal.as_ref().map(flow::downsample);
        Level::new(heights, area, removal)
    }

    fn width(&self) -> usize {
        self.heights.width
    }

    fn height(&self) -> usize {
        self.heights.height
    }

    /// Whether a mask counts on this level (one-pixel buffers, the
    /// stand-in for a missing mask, are skipped).
    fn uses(&self, mask: &Option<Plane>) -> bool {
        mask.is_some() && self.width() * self.height() != 1
    }
}

/// The values every pass of `Thermal::simulation` shares.
pub(crate) struct Constants {
    /// Metres across a pixel per metre of height range.
    pub(crate) cell: f32,
    pub(crate) inverse_cell: f32,
    /// The talus angle's tangent.
    pub(crate) talus: f32,
    pub(crate) plasticity: f32,
    pub(crate) anisotropy: f32,
    pub(crate) strength: f32,
    pub(crate) sediment_removal: f32,
}

impl Constants {
    /// The constants of a level `columns` wide.
    pub(crate) fn new(columns: usize, settings: &ThermalSettings) -> Constants {
        let columns = columns as i32 as f32;
        let cell = settings.heightmap_size / (columns * settings.heightmap_height);
        let half_turns = settings.talus_angle * DEGREES_TO_HALF_TURNS;
        let radians = half_turns as f64 * std::f64::consts::PI;
        let talus = radians.atan() as f32;
        let plasticity = settings.anisotropy.mul_add(1.333_333_4, -0.333_333_34);
        let anisotropy = settings.anisotropy * 4.0;
        Constants {
            cell,
            inverse_cell: 1.0 / cell,
            talus,
            plasticity: x86::maxss(plasticity, 0.0),
            anisotropy: x86::minss(anisotropy, 1.0),
            strength: settings.strength,
            sediment_removal: settings.sediment_removal,
        }
    }
}

/// A pixel's 3×3 neighbourhood as the slope kernels read it, clamped at
/// the edges. Columns clamp against the launch's row count and rows
/// against its column count, the same thing on square maps.
struct Neighbourhood {
    top_left: f32,
    top: f32,
    top_right: f32,
    left: f32,
    centre: f32,
    right: f32,
    bottom_left: f32,
    bottom: f32,
    bottom_right: f32,
}

impl Neighbourhood {
    fn read(heights: &Plane, x: usize, y: usize) -> Neighbourhood {
        let left_x = x.max(1) - 1;
        let right_x = (x + 1).min(heights.height - 1).min(heights.width - 1);
        let top_y = y.max(1) - 1;
        let bottom_y = (y + 1).min(heights.width - 1).min(heights.height - 1);
        let top_row = heights.row(top_y);
        let row = heights.row(y);
        let bottom_row = heights.row(bottom_y);
        Neighbourhood {
            top_left: top_row[left_x],
            top: top_row[x],
            top_right: top_row[right_x],
            left: row[left_x],
            centre: row[x],
            right: row[right_x],
            bottom_left: bottom_row[left_x],
            bottom: bottom_row[x],
            bottom_right: bottom_row[right_x],
        }
    }
}

/// The slope kernels' measures of a neighbourhood: the gradient (3-10-3
/// weights over 32) and the slope, the steeper of the gradient's length
/// and 0.7 of the steepest drop.
struct Measure {
    gradient_x: f32,
    gradient_y: f32,
    slope: f32,
}

/// `top_corners_reversed`: the flow kernel compares the top corners' drops
/// the other way round.
fn measure(n: &Neighbourhood, top_corners_reversed: bool) -> Measure {
    let diagonal = (n.top_right - (n.bottom_left + n.top_left)) + n.bottom_right;
    let across = (n.right - n.left) * 10.0;
    let gradient_x = 3.0f32.mul_add(diagonal, across) * 0.03125;
    let top_corners = n.top_right + n.top_left;
    let bottom_corners = n.bottom_right + n.bottom_left;
    let vertical = top_corners - bottom_corners;
    let down = (n.top - n.bottom) * 10.0;
    let gradient_y = 3.0f32.mul_add(vertical, down) * -0.03125;
    let length = gradient_x * gradient_x;
    let length = gradient_y.mul_add(gradient_y, length).sqrt();

    let c = n.centre;
    let right_or_bottom = x86::maxss(c - n.right, c - n.bottom);
    let top_or_left = x86::maxss(c - n.top, c - n.left);
    let straight = x86::maxss(top_or_left, right_or_bottom);
    let bottom_corner = x86::maxss(c - n.bottom_left, c - n.bottom_right);
    let top_corner = if top_corners_reversed {
        x86::maxss(c - n.top_right, c - n.top_left)
    } else {
        x86::maxss(c - n.top_left, c - n.top_right)
    };
    let corner = x86::maxss(bottom_corner, top_corner) * DIAGONAL;
    let straight = x86::maxss(straight, 0.0);
    let steepest = x86::maxss(straight, corner) * 0.7;
    Measure { gradient_x, gradient_y, slope: x86::maxss(length, steepest) }
}

/// How readily steep ground wears: 1 − plasticity, lowered with the slope
/// (more on convex ground) and blended by anisotropy.
fn erodibility(n: &Neighbourhood, tangent: f32, constants: &Constants) -> f32 {
    let sides = (n.left + n.right) + (n.top + n.bottom);
    let corners = (n.bottom_right + n.bottom_left) + (n.top_right + n.top_left);
    let curvature = sides.mul_add(MINUS_SIXTH, n.centre);
    let curvature = corners.mul_add(MINUS_TWELFTH, curvature);
    let convexity = constants.inverse_cell * curvature;
    let plasticity = constants.plasticity;
    let base = 1.0 - plasticity;
    let mut factor = base;
    if convexity > 0.0 {
        let boost = plasticity.mul_add(1.6, base);
        let scaled = plasticity * convexity;
        let shape = plasticity.mul_add(13.0, 7.0);
        let product = scaled * shape;
        factor = product.mul_add(boost, boost);
    }
    let lowered = (-tangent).mul_add(factor, base);
    (-lowered).mul_add(constants.anisotropy, base)
}

/// The sediment a pixel wears loose this pass, capped by its slope.
fn wear_amount(erodibility: f32, tangent: f32, sediment: f32, rate: f32, constants: &Constants) -> f32 {
    let hardness = (constants.inverse_cell * -0.15).mul_add(sediment, 1.0);
    let hardness = x86::maxss(hardness, 0.0) * rate;
    let erosion = hardness * (erodibility * constants.cell);
    let erosion = erosion * constants.strength;
    let cap = tangent * constants.cell;
    x86::minss(erosion, cap)
}

/// The first isotropic kernel: the slope, and the surface worn into
/// sediment where it is steeper than the talus.
fn wear_pass(level: &mut Level, slope: &mut Plane, threshold: f32, constants: &Constants) {
    let width = level.width();
    let area_used = level.uses(&level.area);
    let heights = &level.heights;
    let area = level.area.as_ref();
    let rows = slope.data.par_chunks_mut(width).zip(level.sediment.data.par_chunks_mut(width));
    let rows = rows.zip(level.wear.data.par_chunks_mut(width));
    rows.enumerate().for_each(|(y, ((slope_row, sediment_row), wear_row))| {
        for x in 0..width {
            let n = Neighbourhood::read(heights, x, y);
            let measured = measure(&n, false);
            slope_row[x] = measured.slope;
            let tangent = constants.inverse_cell * measured.slope;
            let sediment = sediment_row[x];
            let erodibility = if tangent > threshold { erodibility(&n, tangent, constants) } else { 0.0 };
            let mut amount = wear_amount(erodibility, tangent, sediment, ISOTROPIC_WEAR, constants);
            if area_used {
                amount *= area.unwrap().data[y * width + x];
            }
            sediment_row[x] = amount + sediment;
            wear_row[x] += amount;
        }
    });
}

/// The first anisotropic kernel: the wear as in `wear_pass`, and how far
/// each pixel tilts along its slope where steeper than half the talus.
fn flow_wear_pass(level: &mut Level, flow_x: &mut Plane, flow_y: &mut Plane, constants: &Constants) {
    let width = level.width();
    let area_used = level.uses(&level.area);
    let heights = &level.heights;
    let area = level.area.as_ref();
    let flow_rows = flow_x.data.par_chunks_mut(width).zip(flow_y.data.par_chunks_mut(width));
    let rows = flow_rows.zip(level.sediment.data.par_chunks_mut(width));
    let rows = rows.zip(level.wear.data.par_chunks_mut(width));
    rows.enumerate().for_each(|(y, (((flow_x_row, flow_y_row), sediment_row), wear_row))| {
        for x in 0..width {
            let n = Neighbourhood::read(heights, x, y);
            let measured = measure(&n, true);
            let tangent = measured.slope * constants.inverse_cell;
            let talus = constants.talus;
            let mut tilt_x = 0.0;
            let mut tilt_y = 0.0;
            if tangent > talus * 0.5 {
                let ratio = talus / tangent;
                let weight = if talus < tangent { ratio * ratio } else { 1.0 };
                let scale = weight * ratio;
                tilt_x = scale * measured.gradient_x;
                tilt_y = scale * measured.gradient_y;
            }
            flow_x_row[x] = tilt_x;
            flow_y_row[x] = tilt_y;

            let sediment = sediment_row[x];
            let erodibility = if tangent > talus { erodibility(&n, tangent, constants) } else { 0.0 };
            let mut amount = wear_amount(erodibility, tangent, sediment, ANISOTROPIC_WEAR, constants);
            if area_used {
                amount *= area.unwrap().data[y * width + x];
            }
            sediment_row[x] = sediment + amount;
            wear_row[x] += amount;
        }
    });
}

/// The second isotropic kernel: where the slope beats the talus, the sum
/// of the pixel's drops to its neighbours (diagonals weighted 0.7071);
/// the sediment it can give is the lesser of that and what it holds.
fn drops_pass(level: &mut Level, slope: &Plane, threshold: f32) {
    let width = level.width();
    let height = level.height();
    let heights = &level.heights;
    let sediment = &level.sediment;
    let rows = level.drops.data.par_chunks_mut(width).zip(level.next_sediment.data.par_chunks_mut(width));
    rows.enumerate().for_each(|(y, (drops_row, outflow_row))| {
        for x in 0..width {
            let pixel = y * width + x;
            let mut total = 0.0f32;
            if slope.data[pixel] > threshold {
                let centre = heights.data[pixel];
                let drop = |nx: usize, ny: usize, weight: Option<f32>| {
                    let fall = centre - heights.data[ny * width + nx];
                    let fall = match weight {
                        Some(weight) => fall * weight,
                        None => fall,
                    };
                    x86::maxss(fall, 0.0)
                };
                if y >= 1 {
                    if x >= 1 {
                        total += drop(x - 1, y - 1, Some(DIAGONAL));
                    }
                    total += drop(x, y - 1, None);
                    if x + 1 < width {
                        total += drop(x + 1, y - 1, Some(DIAGONAL));
                    }
                }
                if x >= 1 {
                    total += drop(x - 1, y, None);
                }
                if x + 1 < width {
                    total += drop(x + 1, y, None);
                }
                if y + 1 < height {
                    if x >= 1 {
                        total += drop(x - 1, y + 1, Some(DIAGONAL));
                    }
                    total += drop(x, y + 1, None);
                    if x + 1 < width {
                        total += drop(x + 1, y + 1, Some(DIAGONAL));
                    }
                }
            }
            drops_row[x] = total;
            outflow_row[x] = -x86::minss(total, sediment.data[pixel]);
        }
    });
}

/// The second anisotropic kernel: the drops to the neighbours from the
/// pixel tilted by its flow, inside the border. Launches wider than three
/// run vectorised, which adds the drops in another order (its scalar tail
/// too, apart from how the last drop is clamped).
fn flow_drops_pass(level: &mut Level, flow_x: &Plane, flow_y: &Plane) {
    let width = level.width();
    let height = level.height();
    let heights = &level.heights;
    let sediment = &level.sediment;
    let vectorised = width > 3;
    let rows = level.drops.data.par_chunks_mut(width).zip(level.next_sediment.data.par_chunks_mut(width));
    rows.enumerate().for_each(|(y, (drops_row, outflow_row))| {
        for x in 0..width {
            let pixel = y * width + x;
            let interior = x != 0 && y != 0 && x + 1 < width && y + 1 < height;
            let total = if !interior {
                0.0
            } else {
                let h = heights.data[pixel];
                let tilt_x = flow_x.data[pixel];
                let tilt_y = flow_y.data[pixel];
                let top_row = heights.row(y - 1);
                let row = heights.row(y);
                let bottom_row = heights.row(y + 1);
                let west = h - tilt_x;
                let top_left = x86::maxss((west - (tilt_y + top_row[x - 1])) * DIAGONAL, 0.0);
                let left = x86::maxss(west - row[x - 1], 0.0);
                let bottom_left = x86::maxss(((west + tilt_y) - bottom_row[x - 1]) * DIAGONAL, 0.0);
                let top = x86::maxss(h - (tilt_y + top_row[x]), 0.0);
                let bottom = x86::maxss((tilt_y + h) - bottom_row[x], 0.0);
                let east = tilt_x + h;
                let top_right = x86::maxss((east - (tilt_y + top_row[x + 1])) * DIAGONAL, 0.0);
                let right = x86::maxss(east - row[x + 1], 0.0);
                let bottom_right = ((east + tilt_y) - bottom_row[x + 1]) * DIAGONAL;
                if vectorised {
                    let western = (left + top_left) + bottom_left;
                    let rest = ((top + bottom) + top_right) + right;
                    let bottom_right = if flow::vector_lane(x, width) {
                        x86::maxss(0.0, bottom_right)
                    } else {
                        x86::maxss(bottom_right, 0.0)
                    };
                    (western + rest) + bottom_right
                } else {
                    let upper = ((top + (left + top_left)) + top_right) + x86::maxss(bottom_right, 0.0);
                    let lower = (bottom + bottom_left) + right;
                    upper + lower
                }
            };
            drops_row[x] = total;
            outflow_row[x] = -x86::minss(total, sediment.data[pixel]);
        }
    });
}

/// The third kernel's last step: the pass's change, the pixel's outflow
/// plus what it gains (less the share sediment removal takes), scaled by
/// `step`.
fn settle(gain: f32, outflow: f32, mask: f32, constants: &Constants, step: f32) -> f32 {
    let kept = mask.mul_add(constants.sediment_removal, -1.0);
    let net = (-kept).mul_add(gain, outflow);
    step * net
}

/// The share of a neighbour's outflow that reaches the pixel.
fn share(rise: f32, drops: f32, sediment: f32) -> f32 {
    let rise = x86::maxss(rise, 0.0);
    let moved = x86::minss(drops, sediment);
    rise * moved / drops
}

/// The third isotropic kernel: each pixel gains the share of its steeper
/// neighbours' outflow that falls its way, and the heights and sediment
/// take the pass's change.
fn gather_pass(level: &mut Level, slope: &Plane, threshold: f32, constants: &Constants) {
    let width = level.width();
    let height = level.height();
    let removal_used = level.uses(&level.removal);
    let heights = &level.heights;
    let sediment = &level.sediment;
    let drops = &level.drops;
    let removal = level.removal.as_ref();
    const NEIGHBOURS: [(isize, isize, bool); 8] = [
        (-1, -1, true),
        (-1, 0, false),
        (-1, 1, true),
        (0, -1, false),
        (0, 1, false),
        (1, -1, true),
        (1, 0, false),
        (1, 1, true),
    ];
    let rows = level.next_heights.data.par_chunks_mut(width).zip(level.next_sediment.data.par_chunks_mut(width));
    rows.enumerate().for_each(|(y, (heights_row, sediment_row))| {
        for x in 0..width {
            let pixel = y * width + x;
            let h = heights.data[pixel];
            let mut gain = 0.0f32;
            let interior = x != 0 && y != 0 && x + 1 < width && y + 1 < height;
            if interior {
                for (offset_x, offset_y, diagonal) in NEIGHBOURS {
                    let neighbour = (y as isize + offset_y) as usize * width + (x as isize + offset_x) as usize;
                    let steep = !(threshold >= slope.data[neighbour]);
                    let neighbour_drops = drops.data[neighbour];
                    if steep && !(0.0 >= neighbour_drops) {
                        let rise = heights.data[neighbour] - h;
                        let rise = if diagonal { rise * DIAGONAL } else { rise };
                        gain += share(rise, neighbour_drops, sediment.data[neighbour]);
                    }
                }
            }
            let mask = if removal_used { removal.unwrap().data[pixel] } else { 1.0 };
            let change = settle(gain, sediment_row[x], mask, constants, ISOTROPIC_STEP);
            heights_row[x] = change + h;
            sediment_row[x] = change + sediment.data[pixel];
        }
    });
}

/// The third anisotropic kernel: as `gather_pass`, the neighbours tilted
/// by their flow and no talus test.
fn flow_gather_pass(level: &mut Level, flow_x: &Plane, flow_y: &Plane, constants: &Constants) {
    let width = level.width();
    let height = level.height();
    let removal_used = level.uses(&level.removal);
    let heights = &level.heights;
    let sediment = &level.sediment;
    let drops = &level.drops;
    let removal = level.removal.as_ref();
    let rows = level.next_heights.data.par_chunks_mut(width).zip(level.next_sediment.data.par_chunks_mut(width));
    rows.enumerate().for_each(|(y, (heights_row, sediment_row))| {
        for x in 0..width {
            let pixel = y * width + x;
            let h = heights.data[pixel];
            let mut gain = 0.0f32;
            let interior = x != 0 && y != 0 && x + 1 < width && y + 1 < height;
            if interior {
                let at = |nx: usize, ny: usize| ny * width + nx;
                let mut add = |neighbour: usize, rise: f32| {
                    let neighbour_drops = drops.data[neighbour];
                    if !(0.0 >= neighbour_drops) {
                        gain += share(rise, neighbour_drops, sediment.data[neighbour]);
                    }
                };
                let n = at(x - 1, y - 1);
                add(n, (((heights.data[n] - h) + flow_y.data[n]) + flow_x.data[n]) * DIAGONAL);
                let n = at(x - 1, y);
                add(n, (heights.data[n] - h) + flow_x.data[n]);
                let n = at(x - 1, y + 1);
                add(n, ((heights.data[n] - (h + flow_y.data[n])) + flow_x.data[n]) * DIAGONAL);
                let n = at(x, y - 1);
                add(n, (heights.data[n] - h) + flow_y.data[n]);
                let n = at(x, y + 1);
                add(n, heights.data[n] - (h + flow_y.data[n]));
                let n = at(x + 1, y - 1);
                add(n, ((heights.data[n] + flow_y.data[n]) - (h + flow_x.data[n])) * DIAGONAL);
                let n = at(x + 1, y);
                add(n, heights.data[n] - (h + flow_x.data[n]));
                let n = at(x + 1, y + 1);
                add(n, (heights.data[n] - ((h + flow_y.data[n]) + flow_x.data[n])) * DIAGONAL);
            }
            let mask = if removal_used { removal.unwrap().data[pixel] } else { 1.0 };
            let change = settle(gain, sediment_row[x], mask, constants, ANISOTROPIC_STEP);
            heights_row[x] = change + h;
            sediment_row[x] = change + sediment.data[pixel];
        }
    });
}

/// One pass of `Thermal::simulation`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PassKind {
    /// The isotropic kernels with these talus thresholds (zero when off):
    /// on the tangent, and on the raw slope.
    Isotropic { threshold: f32, threshold_raw: f32 },
    /// The anisotropic kernels.
    Anisotropic,
}

/// The passes `Thermal::simulation` runs over a level, given its
/// `iterations`: isotropic with the talus threshold, except in the last
/// eight (and on the coarsest level), where the threshold is off, and on
/// the finest level near the end, where the anisotropic kernels run.
pub(crate) fn pass_schedule(iterations: f32, index: i32, finest: bool, constants: &Constants) -> Vec<PassKind> {
    let talus_raw = constants.cell * constants.talus;
    let last_talus = if index != 0 { iterations + -8.0 } else { 0.0 };
    let half = x86::minss(iterations * 0.5, 20.0);
    let anisotropic_from = iterations - half;

    let mut passes = Vec::new();
    loop {
        let step = passes.len() as i32 as f32;
        if !(iterations > step) {
            break;
        }
        let mut kind = PassKind::Isotropic { threshold: 0.0, threshold_raw: 0.0 };
        if last_talus > step {
            let late = !(anisotropic_from > step);
            if late && finest {
                kind = PassKind::Anisotropic;
            } else {
                kind = PassKind::Isotropic { threshold: constants.talus, threshold_raw: talus_raw };
            }
        }
        passes.push(kind);
    }
    passes
}

/// `Thermal::simulation`: `iterations` passes over a level; returns how
/// many ran (see [`pass_schedule`]).
fn simulate_level(level: &mut Level, iterations: f32, index: i32, finest: bool, settings: &ThermalSettings) -> usize {
    let constants = Constants::new(level.width(), settings);
    let width = level.width();
    let height = level.height();
    let mut slope = Plane::zeros(width, height);
    let mut flow_x = Plane::zeros(width, height);
    let mut flow_y = Plane::zeros(width, height);
    let passes = pass_schedule(iterations, index, finest, &constants);
    for &kind in &passes {
        match kind {
            PassKind::Anisotropic => {
                flow_wear_pass(level, &mut flow_x, &mut flow_y, &constants);
                flow_drops_pass(level, &flow_x, &flow_y);
                flow_gather_pass(level, &flow_x, &flow_y, &constants);
            }
            PassKind::Isotropic { threshold, threshold_raw } => {
                wear_pass(level, &mut slope, threshold, &constants);
                drops_pass(level, &slope, threshold_raw);
                gather_pass(level, &slope, threshold_raw, &constants);
            }
        }
        std::mem::swap(&mut level.heights, &mut level.next_heights);
        std::mem::swap(&mut level.sediment, &mut level.next_sediment);
    }
    passes.len()
}

/// `LaplacianLevel::reconstruct_previous_level`: the finer level rebuilt
/// from the simulated coarse one. Its detail returns where the coarse
/// sediment leaves it exposed (eased by the coarse wear), the sediment
/// left over buries it, and with a removal mask some of that sediment is
/// taken away.
fn reconstruct(coarse: &Level, fine: &mut Level, settings: &ThermalSettings) {
    let width = fine.width();
    let columns = coarse.width() as i32 as f32;
    let cell = settings.heightmap_size / (columns * settings.heightmap_height);
    let removal_used = fine.uses(&fine.removal);
    let detail = &fine.detail;
    let fine_removal = fine.removal.as_ref();
    let coarse_removal = coarse.removal.as_ref();
    let rows = fine.sediment.data.par_chunks_mut(width).zip(fine.heights.data.par_chunks_mut(width));
    let rows = rows.zip(fine.wear.data.par_chunks_mut(width));
    rows.enumerate().for_each(|(y, ((sediment_row, heights_row), wear_row))| {
        for x in 0..width {
            let pixel = y * width + x;
            let lifted = flow::upsample_sum(&coarse.heights, x, y) * 0.0625;
            let wear_sum = flow::upsample_sum(&coarse.wear, x, y);
            let detail = detail.data[pixel];
            let covering = flow::upsample_sum(&coarse.sediment, x, y) * 0.0625;
            let mut heights;
            let mut buried;
            if !(covering > 0.0) {
                heights = detail + lifted;
                buried = covering;
            } else if !(covering > detail) {
                let exposed = detail - covering;
                let limit = x86::maxss(wear_sum * 0.125, cell);
                let ratio = x86::minss(cell / limit, 1.0);
                heights = exposed.mul_add(ratio, lifted);
                buried = 0.0;
            } else {
                heights = lifted;
                buried = covering - detail;
            }

            let mut removal_detail = 0.0;
            if removal_used {
                let coarse_removal = coarse_removal.unwrap();
                let (own_x, side_x) = flow::coarse_neighbours(x, coarse_removal.width as i32 - 1);
                let (own_y, side_y) = flow::coarse_neighbours(y, coarse_removal.height as i32 - 1);
                let own_row = coarse_removal.row(own_y);
                let side_row = coarse_removal.row(side_y);
                let sides = side_row[own_x] + own_row[side_x];
                let centre = 9.0f32.mul_add(own_row[own_x], side_row[side_x]);
                let difference = sides.mul_add(-3.0, -centre);
                removal_detail = difference.mul_add(0.0625, fine_removal.unwrap().data[pixel]);
            }
            let exposure = x86::maxss(removal_detail, 0.0);
            let exposure = x86::minss(exposure + exposure, 1.0);
            let removal_strength = exposure * settings.sediment_removal;
            let removed = if buried > 0.0 && 0.0 < removal_strength { removal_strength * buried } else { 0.0 };
            buried -= removed;
            heights -= removed;
            sediment_row[x] = buried;
            heights_row[x] = heights;
            wear_row[x] = wear_sum * 0.0625;
        }
    });
}

/// The passes `Thermal::process` gives the level it processes `index`th
/// (coarsest first): fewer on finer levels, the coarsest scaled by the
/// fraction of an octave the pyramid spans.
pub(crate) fn pass_count(index: i32, fraction: f32, offset: f32, budget: f32) -> f32 {
    let multiplier = if index != 0 { 1.0 } else { fraction };
    let scale = multiplier * budget;
    let shifted = x86::maxss(0.0, offset + index as f32);
    let decay = ucrt_expf(shifted * -6.0);
    let exponent = decay.mul_add(0.166_666_67, shifted + MINUS_SIXTH);
    let falloff = ucrt_expf(exponent * -std::f32::consts::LN_2);
    (scale * falloff).ceil()
}

/// How the pyramid is laid out (`Thermal::process`): how many levels it
/// has, the fraction of an octave it spans, the offset [`pass_count`]
/// starts from and the pass budget.
pub(crate) struct PyramidPlan {
    pub(crate) level_count: f32,
    pub(crate) fraction: f32,
    pub(crate) offset: f32,
    pub(crate) budget: f32,
}

pub(crate) fn pyramid_plan(width: usize, settings: &ThermalSettings) -> PyramidPlan {
    let scale = (width as i32 as f32 * settings.erosion_scale) / settings.heightmap_size;
    let octaves = ucrt_log2f(scale);
    let level_count = octaves.ceil();
    let scaled = if octaves >= 1.0 { octaves } else { ucrt_expf(octaves + -1.0) };
    let fraction = scaled - scaled.floor();
    let fraction = if fraction == 0.0 { 1.0 } else { fraction };
    let offset = fraction + -2.5;
    let budget = settings.duration * 2560.0;
    PyramidPlan { level_count, fraction, offset, budget }
}

/// `ThermalSimulation`: thermal erosion of `heights`, `area` limiting the
/// wear and `removal` where sediment is removed. Runs on the GPU when one
/// is active (see [`sycl`]).
pub fn simulate(heights: &Map, area: Option<&Map>, removal: Option<&Map>, settings: &ThermalSettings) -> ThermalMaps {
    let gpu = |device: &nerthus_compute::GpuDevice| gpu::thermal::simulate(device, heights, area, removal, settings);
    let cpu = || simulate_cpu(heights, area, removal, settings);
    sycl::run("ThermalSimulation", heights.width(), sycl::PREFERRED, gpu, cpu)
}

/// [`simulate`] on the CPU. As with the kernels' buffers, the finest
/// level's height and sediment results come from the buffers that started as the
/// caller's: after an odd number of passes those hold the second-to-last
/// pass.
pub fn simulate_cpu(heights: &Map, area: Option<&Map>, removal: Option<&Map>, settings: &ThermalSettings) -> ThermalMaps {
    let resolution = heights.resolution();
    let width = resolution.x;
    let height = resolution.y;
    let plane = |map: &Map| Plane { width, height, data: map.channel(0).to_vec() };
    let finest = Level::new(plane(heights), area.map(plane), removal.map(plane));
    let plan = pyramid_plan(width, settings);

    let mut pyramid = vec![finest];
    while ((pyramid.len()) as i32 as f32) < plan.level_count {
        let coarse = pyramid.last_mut().unwrap().coarser();
        pyramid.push(coarse);
    }

    let mut index = 0;
    while pyramid.len() > 1 {
        let mut level = pyramid.pop().unwrap();
        let passes = pass_count(index, plan.fraction, plan.offset, plan.budget);
        simulate_level(&mut level, passes, index, false, settings);
        let parent = pyramid.last_mut().unwrap();
        reconstruct(&level, parent, settings);
        index += 1;
    }
    let mut finest = pyramid.pop().unwrap();
    let passes = pass_count(index, plan.fraction, plan.offset, plan.budget);
    let passes_run = simulate_level(&mut finest, passes, index, true, settings);

    let swapped = passes_run % 2 == 1;
    let (heights, sediment) = if swapped {
        (finest.next_heights, finest.next_sediment)
    } else {
        (finest.heights, finest.sediment)
    };
    ThermalMaps {
        heights: Map::from_data(resolution, heights.data),
        deposition: Map::from_data(resolution, sediment.data),
        wear: Map::from_data(resolution, finest.wear.data),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_keep_their_bits() {
        assert_eq!(DIAGONAL.to_bits(), 0x3f35_0481);
        assert_eq!(DEGREES_TO_HALF_TURNS.to_bits(), 0x3bb6_0b61);
        assert_eq!(MINUS_SIXTH.to_bits(), 0xbe2a_aaab);
        assert_eq!(MINUS_TWELFTH.to_bits(), 0xbdaa_aaab);
        assert_eq!(ISOTROPIC_STEP.to_bits(), 0x3e4c_cccd);
        assert_eq!(ANISOTROPIC_STEP.to_bits(), 0x3e0f_5c29);
        assert_eq!(ISOTROPIC_WEAR.to_bits(), 0x3b90_2de0);
        assert_eq!(ANISOTROPIC_WEAR.to_bits(), 0x3b2d_03d9);
        assert_eq!((-0.15f32).to_bits(), 0xbe19_999a);
        assert_eq!(1.6f32.to_bits(), 0x3fcc_cccd);
        assert_eq!(1.333_333_4f32.to_bits(), 0x3faa_aaab);
        assert_eq!((-0.333_333_34f32).to_bits(), 0xbeaa_aaab);
        assert_eq!(0.166_666_67f32.to_bits(), 0x3e2a_aaab);
        assert_eq!((-std::f32::consts::LN_2).to_bits(), 0xbf31_7218);
    }
}
