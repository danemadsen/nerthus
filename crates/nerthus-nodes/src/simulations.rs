//! The managed simulations (`Simulations`).

use nerthus_engine::{Map, Resolution};
use rayon::prelude::*;

use crate::maths::dotnet_max;

/// `Simulations.ComputeOutflow`: pipe-model outflow towards the four
/// neighbours (left, right, up, down), limited by the water present.
fn compute_outflow(water: &[f32], outflow: &mut [[f32; 4]], heights: &[f32], width: usize, height: usize) {
    if width * height < 256 {
        return;
    }
    outflow.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
        let above = if y != 0 { y - 1 } else { 0 };
        let below = if y == height - 1 { height - 1 } else { y + 1 };
        for (x, flow) in row.iter_mut().enumerate() {
            let left = if x != 0 { x - 1 } else { 0 };
            let right = if x == width - 1 { width - 1 } else { x + 1 };
            let at = |values: &[f32], column: usize, line: usize| values[line * width + column];
            let level = at(water, x, y);
            let ground = at(heights, x, y);
            let neighbours = [(left, y), (right, y), (x, above), (x, below)];
            let mut pushed = [0.0f32; 4];
            for (direction, &(column, line)) in neighbours.iter().enumerate() {
                let difference = level + ground - (at(water, column, line) + at(heights, column, line));
                pushed[direction] = dotnet_max(0.0, flow[direction] + difference);
            }
            let total = pushed[0] + pushed[1] + pushed[2] + pushed[3];
            if total > 0.0 {
                let limit = (level / (total * 0.1)).clamp(0.0, 1.0);
                for direction in 0..4 {
                    flow[direction] = pushed[direction] * limit;
                }
            } else {
                *flow = [0.0; 4];
            }
        }
    });
}

/// `Simulations.UpdateWaterMap`: water in minus water out.
fn update_water(water: &mut [f32], outflow: &[[f32; 4]], width: usize, height: usize) {
    water.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
        for (x, level) in row.iter_mut().enumerate() {
            let own = outflow[y * width + x];
            let leaving = own[0] + own[1] + own[2] + own[3];
            let mut arriving = 0.0f32;
            arriving += if x == 0 { 0.0 } else { outflow[y * width + x - 1][1] };
            arriving += if x == width - 1 { 0.0 } else { outflow[y * width + x + 1][0] };
            arriving += if y == 0 { 0.0 } else { outflow[(y - 1) * width + x][3] };
            arriving += if y == height - 1 { 0.0 } else { outflow[(y + 1) * width + x][2] };
            let mut updated = *level + (arriving - leaving) * 0.1;
            if updated < 0.0 {
                updated = 0.0;
            }
            *level = updated;
        }
    });
}

/// `Simulations.CalculateVelocityField`: the speed of the net flow
/// through each cell (written transposed when `oriented`).
fn velocity_field(outflow: &[[f32; 4]], width: usize, height: usize, oriented: bool) -> Vec<f32> {
    let speeds: Vec<Vec<f32>> = (0..height)
        .into_par_iter()
        .map(|y| {
            (0..width)
                .map(|x| {
                    let at = |column: usize, line: usize, direction: usize| outflow[line * width + column][direction];
                    let from_left = if x == 0 { 0.0 } else { at(x - 1, y, 1) - at(x, y, 0) };
                    let to_right = if x == width - 1 { 0.0 } else { at(x, y, 1) - at(x + 1, y, 0) };
                    let from_below = if y == height - 1 { 0.0 } else { at(x, y + 1, 2) - at(x, y, 3) };
                    let from_above = if y == 0 { 0.0 } else { at(x, y, 2) - at(x, y - 1, 3) };
                    let horizontal = (from_left + to_right) * 0.5;
                    let vertical = (from_above + from_below) * 0.5;
                    (horizontal * horizontal + vertical * vertical).sqrt()
                })
                .collect()
        })
        .collect();
    let mut velocity = vec![0.0f32; width * height];
    for (y, row) in speeds.into_iter().enumerate() {
        for (x, speed) in row.into_iter().enumerate() {
            let index = if oriented { x * width + y } else { y * width + x };
            velocity[index] = speed;
        }
    }
    velocity
}

/// `Simulations.CreateFlowMap`: rain spread over `heights` and run
/// through a pipe model for `iterations` steps; returns the flow speed.
pub fn create_flow_map(heights: &Map, iterations: i32, rain: f32, oriented: bool) -> Map {
    let width = heights.width();
    let height = heights.height();
    let mut water = vec![rain; width * height];
    let mut outflow = vec![[0.0f32; 4]; width * height];
    for _ in 0..iterations {
        compute_outflow(&water, &mut outflow, heights.data(), width, height);
        update_water(&mut water, &outflow, width, height);
    }
    let velocity = velocity_field(&outflow, width, height, oriented);
    Map::from_data(Resolution::new(width, height), velocity)
}
