//! One erosion droplet of the CE engine, transcribed from
//! the compiled code so every float operation happens in the same order and
//! precision. Rivers mode is not supported here yet.

use super::engine::{lcg, Erosion, RANDOM_SCALE};
use super::kernel::FloatMap;
use super::layer::Layer;

/// Adds `deposit + extra` to the height and `deposit` to
/// the deposit map at the cell containing `position`, scaled by the
/// sediment-removal map when it is active.
pub(super) fn deposit_at(erosion: &Erosion, layer: &mut Layer, position: (f32, f32), deposit: f32, extra: f32) {
    let x = position.0 as i32;
    let y = position.1 as i32;
    let mut deposit = deposit;
    let mut extra = extra;
    if erosion.sediment_removal {
        let removal = layer.removal.as_ref().expect("removal map present when sediment removal is on");
        let keep = (1.0f64 - removal.at(x, y) as f64) as f32;
        deposit *= keep;
        extra *= keep;
    }
    let index = layer.height.index(x, y);
    layer.height.data[index] += deposit + extra;
    let index = layer.deposit.index(x, y);
    let total = deposit + layer.deposit.data[index];
    layer.deposit.data[index] = if 0.0 > total { 0.0 } else { total };
}

/// The four corner heights of a cell and the bilinear weights inside it.
#[derive(Clone, Copy)]
struct Cell {
    top_left: f32,
    top_right: f32,
    bottom_left: f32,
    bottom_right: f32,
    tx: f32,
    ty: f32,
    ux: f32,
    uy: f32,
}

impl Cell {
    /// Bilinear height, `(tx*br + bl*ux)*ty + (tx*tr + tl*ux)*uy`.
    fn height(&self) -> f32 {
        (self.tx * self.bottom_right + self.bottom_left * self.ux) * self.ty + (self.tx * self.top_right + self.top_left * self.ux) * self.uy
    }
}

#[inline]
fn bump_visit_mark(visits: &mut [u8], index: usize, decay: bool) {
    let value = visits[index] as i32;
    visits[index] = if decay {
        let reduced = value - 0x80;
        if reduced < 0 { 0 } else { reduced as u8 }
    } else {
        let raised = value + 4;
        if raised > 0xff { 0xff } else { raised as u8 }
    };
}

#[inline]
fn bump_flow(flow: &mut [u16], index: usize) {
    if flow[index] < 65000 {
        flow[index] += 1;
    }
}

#[inline]
fn min_ss(a: f32, b: f32) -> f32 {
    // MINSS a, b
    if a < b { a } else { b }
}

#[inline]
fn max_ss(a: f32, b: f32) -> f32 {
    // MAXSS a, b
    if a > b { a } else { b }
}

#[inline]
fn mask_value(mask: &Option<FloatMap>, x: i32, y: i32) -> f32 {
    mask.as_ref().expect("mask present for its masking mode").at(x, y)
}

/// Runs droplet number `droplet` (its index only picks the random
/// direction jitter; the start position comes from the shared generator).
pub(super) fn run_droplet(erosion: &mut Erosion, layer: &mut Layer, droplet: i32) {
    // Per-droplet constants hoisted by the compiler.
    let strength_softness = (erosion.rock_softness * erosion.strength) as f32;
    let squared = strength_softness * strength_softness;
    let fourth = squared * squared;
    let fill_factor = 0.75f32 / (fourth * 0.4 + 1.0) + 0.25;
    let softness = erosion.rock_softness as f32;
    let strength = erosion.strength as f32;
    let flow_bias = if erosion.laminar_flow { 1.0f32 } else { 0.25 };
    let iterations = erosion.iterations as f32;
    let late_iteration = (iterations - (iterations * 0.04 + 15.0) * flow_bias) as i32;

    erosion.random_state = lcg(erosion.random_state);
    let first = erosion.random_state;
    let inhibition = erosion.inhibition;
    let downcut_limit_scale = 1.5f64 / (inhibition * 10.0 + 0.0010000000474974513);
    let downcut_limit = ((downcut_limit_scale + (0.8500000238418579 - inhibition * 0.8999999761581421)) * layer.cell as f64) as f32;
    let second = lcg(first);
    let start_x = first as f32 * RANDOM_SCALE;
    let start_y = second as f32 * RANDOM_SCALE;
    let third = lcg(second);
    let span = (layer.resolution - 2) as f32;
    let mut y = span * start_y + 1.0;
    let mut x = span * start_x + 1.0;
    let mark_decays = third.is_multiple_of(33);
    if erosion.masking_mode == 3 {
        let chance = lcg(third) as f32 * RANDOM_SCALE;
        if chance > mask_value(&layer.mask, x as i32, y as i32) {
            return;
        }
    }

    let size = layer.height.size;
    let sample_cell = |heights: &FloatMap, x: f32, y: f32| -> (Cell, i32, i32) {
        let column = (x - 0.5) as i32;
        let row = (y - 0.5) as i32;
        let top = row * size;
        let bottom = (row + 1) * size;
        let tx = (x - 0.5) - column as f32;
        let ty = (y - 0.5) - row as f32;
        let cell = Cell {
            top_left: heights.data[(top + column) as usize],
            top_right: heights.data[(top + column + 1) as usize],
            bottom_left: heights.data[(bottom + column) as usize],
            bottom_right: heights.data[(bottom + column + 1) as usize],
            tx,
            ty,
            ux: 1.0 - tx,
            uy: 1.0 - ty,
        };
        (cell, column, row)
    };

    let (mut cell, _, _) = sample_cell(&layer.height, x, y);
    let mut surface = cell.height();
    let mut previous_level = surface;
    let start_index = layer.flow.index(x as i32, y as i32);
    bump_flow(&mut layer.flow.data, start_index);
    if layer.level == 0 {
        let index = (y as i32 * layer.resolution + x as i32) as usize;
        let visits = layer.visits.as_mut().expect("full resolution layer has visit marks");
        bump_visit_mark(visits, index, mark_decays);
    }

    let mut direction_x = 0.0f32;
    let mut direction_y = 0.0f32;
    let mut sediment = 0.0f32;
    let mut water = 0.0f32;
    let mut downhill = false;
    let mut uphill_fills = 0i32;
    let mut steps = 0i32;
    let mut trail_x = x;
    let mut trail_y = y;
    let ramp = 1.0f32;

    loop {
        steps += 1;
        // Gradient of the (possibly flow-raised) cell corners.
        let gradient_x = (cell.bottom_right - cell.bottom_left) * cell.ty + (cell.top_right - cell.top_left) * cell.uy;
        let gradient_y = (cell.bottom_right - cell.top_right) * cell.tx + (cell.bottom_left - cell.top_left) * cell.ux;
        let length_squared = gradient_y * gradient_y + gradient_x * gradient_x;
        if 1e-7f32 > length_squared {
            return;
        }
        let length = length_squared.sqrt();
        let descent_x = -(gradient_x / length);
        let descent_y = -(gradient_y / length);
        if !erosion.laminar_flow && downhill {
            let slot = ((droplet + steps) & 0x1f) as usize;
            let jitter = erosion.directions[slot];
            let jitter_x = jitter[0] * 0.3 * ramp;
            let jitter_y = jitter[1] * 0.3 * ramp;
            let flow_index = layer.flow.index(x as i32, y as i32);
            let passes = (erosion.iteration + 1) as f32;
            let mut momentum = layer.flow.data[flow_index] as i32 as f32 / passes;
            if momentum > 120.0 {
                momentum = 120.0;
            }
            momentum *= 0.03;
            let blended_y = direction_y * momentum * ramp + descent_y + jitter_y;
            let blended_x = direction_x * momentum * ramp + descent_x + jitter_x;
            let norm = (blended_y * blended_y + blended_x * blended_x).sqrt();
            let inverse = 1.0 / norm;
            direction_y = inverse * blended_y;
            direction_x = inverse * blended_x;
        } else {
            let norm = (descent_y * descent_y + descent_x * descent_x).sqrt();
            let inverse = 1.0 / norm;
            direction_y = descent_y * inverse;
            direction_x = descent_x * inverse;
        }

        let next_x = direction_x + x;
        let next_y = direction_y + y;
        if 0.5 > next_x || 0.5 > next_y {
            return;
        }
        let limit = layer.resolution as f32 - 0.5;
        if next_x >= limit || next_y >= limit {
            return;
        }
        if steps > layer.resolution * 2 {
            return;
        }

        let (raw, column, row) = sample_cell(&layer.height, next_x, next_y);
        let mut next_cell = raw;
        let volume = (erosion.flow_volume * 0.019999999552965164 * layer.cell as f64) as f32;
        if erosion.flow_volume > 0.0 && erosion.iteration < late_iteration {
            let trail = layer.trail.as_ref().expect("trail map allocated with flow volume");
            let at = |dx: i32, dy: i32| trail.data[((row + dy) * trail.size + column + dx) as usize] as i32 as f32;
            next_cell.top_left = at(0, 0) * volume + raw.top_left;
            next_cell.top_right = at(1, 0) * volume + raw.top_right;
            next_cell.bottom_left = at(0, 1) * volume + raw.bottom_left;
            next_cell.bottom_right = at(1, 1) * volume + raw.bottom_right;
        }
        let height = raw.height();

        let cell_x = x as i32;
        let cell_y = y as i32;
        if height > surface {
            // Uphill: fill the pit with carried material or stop.
            downhill = false;
            let capacity = sediment + water;
            let rise = height - surface;
            if capacity < rise || uphill_fills >= 10 {
                let amount = min_ss(capacity, rise);
                let index = layer.height.index(cell_x, cell_y);
                layer.height.data[index] += amount + 0.0;
                let index = layer.deposit.index(cell_x, cell_y);
                let total = amount + layer.deposit.data[index];
                layer.deposit.data[index] = if 0.0 > total { 0.0 } else { total };
                return;
            }
            sediment -= rise;
            if 0.0 > sediment {
                let remaining = sediment + water;
                sediment = 0.0;
                water = max_ss(remaining, 0.0);
            }
            let fill = (layer.cell * 0.001 + height) - surface;
            previous_level = layer.cell * 0.002 + height;
            surface += fill;
            deposit_at(erosion, layer, (x, y), fill, 0.0);
            uphill_fills += 1;
            direction_x = 0.0;
            direction_y = 0.0;
        } else {
            downhill = true;
        }

        let flow_index = layer.flow.index(cell_x, cell_y);
        bump_flow(&mut layer.flow.data, flow_index);
        if layer.level == 0 {
            let index = (cell_y * layer.resolution + cell_x) as usize;
            let visits = layer.visits.as_mut().expect("full resolution layer has visit marks");
            bump_visit_mark(visits, index, mark_decays);
        }

        let drop = surface - height;
        let mut erosion_amount = drop * strength;
        if erosion.masking_mode == 1 {
            erosion_amount *= mask_value(&layer.mask, cell_x, cell_y);
        }
        if steps == 1 {
            erosion_amount = min_ss(erosion_amount, drop * 0.25);
        }
        let mut change = water - erosion_amount;
        if 0.0 > change {
            change *= softness;
            if erosion.masking_mode == 2 {
                change *= mask_value(&layer.mask, cell_x, cell_y);
            }
            let floor = (height - surface) as f64;
            let widened = change as f64;
            change = (if widened > floor { widened } else { floor }) as f32;
        } else {
            let settle = (previous_level - surface) * fill_factor;
            change = min_ss(change, settle);
        }
        water -= change;

        if erosion.downcutting > 0.0 && erosion.iteration < late_iteration {
            let headroom = ((change + surface) - height) as f64;
            let cell_size = layer.cell;
            let carve = (drop * 0.02 + cell_size * 0.005) as f64 * erosion.downcutting * 4.0 * erosion.strength * softness as f64;
            let carve = if carve < headroom { carve } else { headroom } as f32;
            let threshold = downcut_limit / (drop / cell_size + 0.8);
            let deposit_index = layer.deposit.index(cell_x, cell_y);
            let downcut = if layer.deposit.data[deposit_index] > threshold {
                0.0
            } else {
                ((inhibition * 0.5 + 1.0) * carve as f64) as f32
            };
            sediment += downcut;
            change -= downcut;
            let carried = sediment * 0.1 * 0.2;
            let flow_count = layer.flow.data[flow_index] as i32 as f32;
            let base = (erosion.base_level * 0.0010000000474974513 * flow_count as f64 * carried as f64) as f32;
            let level_gap = ((previous_level + height) * 0.5 - (change + surface + downcut)) * 0.2;
            let release = max_ss(base, level_gap);
            let released = max_ss(min_ss(sediment - release, release), 0.0);
            sediment -= released;
            change += released;
        }

        if 0.0 <= change + 0.0 {
            deposit_at(erosion, layer, (x, y), change, 0.0);
        } else {
            let index = layer.deposit.index(cell_x, cell_y);
            let before = layer.deposit.data[index];
            let mut after = before + change;
            layer.deposit.data[index] = after;
            if 0.0 > after {
                layer.deposit.data[index] = 0.0;
                after = 0.0;
            }
            let regained = max_ss(after - before, 0.0) - change;
            let index = layer.height.index(cell_x, cell_y);
            layer.height.data[index] += change;
            let index = layer.wear.index(cell_x, cell_y);
            layer.wear.data[index] += regained;
        }

        if erosion.flow_volume > 0.0 || erosion.randomness > 0.0 || erosion.debris > 0.0 {
            let trail = layer.trail.as_mut().expect("trail map allocated");
            let index = (trail_y as i32 * trail.size + trail_x as i32) as usize;
            trail.data[index] = trail.data[index].wrapping_add(1);
        }

        previous_level = change + surface;
        surface = height;
        trail_x = x;
        trail_y = y;
        x = next_x;
        y = next_y;
        cell = next_cell;
    }
}
