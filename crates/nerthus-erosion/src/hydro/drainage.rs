//! The per-level drainage passes of Hydrologic.dll: pit raising, the
//! priority-flood drainage tree, channel carving and the correction
//! smoothing.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use super::Level;

/// How far a pit rises towards its lowest neighbour.
const PIT_RAISE: f32 = f32::from_bits(0x3f7f_ff58);
/// Diagonal weight.
const DIAGONAL: f32 = f32::from_bits(0x3f35_0481);
/// Per-step fall of a carved channel (over the resolution).
const CHANNEL_FALL: f32 = -0.005;
/// Smoothing kernel weights.
const SMOOTH_CORNER: f32 = f32::from_bits(0x3d9a_042a);
const SMOOTH_EDGE: f32 = f32::from_bits(0x3dfd_eb30);
const SMOOTH_CENTRE: f32 = f32::from_bits(0x3e50_214d);

/// Cell states of the flood (`[11]`).
const UNVISITED: u8 = 0;
const QUEUED: u8 = 1;
const DONE: u8 = 2;

/// A queued cell. The engine keeps them in a `std::multiset` ordered by
/// height only, so equal heights leave in the order they arrived.
#[derive(Clone, Copy)]
struct Queued {
    height: f32,
    x: i32,
    y: i32,
    sequence: u64,
}

impl PartialEq for Queued {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Queued {}

impl PartialOrd for Queued {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Queued {
    /// Reversed so the max-heap pops the lowest, earliest cell.
    fn cmp(&self, other: &Self) -> Ordering {
        let by_height = other.height.partial_cmp(&self.height).unwrap_or(Ordering::Equal);
        by_height.then(other.sequence.cmp(&self.sequence))
    }
}

struct Queue {
    heap: BinaryHeap<Queued>,
    next: u64,
}

impl Queue {
    fn new() -> Queue {
        Queue {
            heap: BinaryHeap::new(),
            next: 0,
        }
    }

    fn push(&mut self, height: f32, x: i32, y: i32) {
        let entry = Queued {
            height,
            x,
            y,
            sequence: self.next,
        };
        self.next += 1;
        self.heap.push(entry);
    }
}

/// `x <= y ? x : y` the way the pit raise picks its minimum.
fn lower(current: f32, candidate: f32) -> f32 {
    if current <= candidate { current } else { candidate }
}

pub(crate) struct Drainage {
    downcutting: f32,
}

impl Drainage {
    pub fn new(downcutting: f32) -> Drainage {
        Drainage { downcutting }
    }

    /// One level: raise pits, build the drainage tree, carve channels into
    /// `level.correction` and smooth them.
    pub fn correct(&mut self, level: &mut Level) {
        raise_pits(&mut level.height.data, level.size);
        let parents = drainage_tree(&level.height.data, level.size, None);
        self.carve(level, &parents);
        smooth(&level.correction, &mut level.scratch, level.size);
        if level.coarsest {
            level.correction.copy_from_slice(&level.scratch);
        } else {
            smooth(&level.scratch, &mut level.correction, level.size);
        }
    }

    /// From every pit (or flat that is not completely
    /// flat) follow the drainage tree and lower the cells above the pit.
    fn carve(&self, level: &mut Level, parents: &[(i32, i32)]) {
        let n = level.size as i32;
        let heights = &level.height.data;
        for y in 1..n - 1 {
            for x in 1..n - 1 {
                let at = |cx: i32, cy: i32| heights[(cy * n + cx) as usize];
                let centre = at(x, y);
                let mut lowest = at(x - 1, y - 1);
                let mut equal = u8::from(lowest == centre);
                let neighbours = [(x, y - 1), (x + 1, y - 1), (x - 1, y), (x + 1, y), (x - 1, y + 1), (x, y + 1), (x + 1, y + 1)];
                for (cx, cy) in neighbours {
                    let value = at(cx, cy);
                    if value < lowest {
                        lowest = value;
                    }
                    if value == centre {
                        equal += 1;
                    }
                }
                let is_pit = centre < lowest || (lowest == centre && equal < 8);
                if !is_pit {
                    continue;
                }
                if self.downcutting <= 0.0 {
                    let step = if 0.0 <= self.downcutting { 0.0 } else { CHANNEL_FALL / n as f32 };
                    carve_flat(heights, &mut level.correction, parents, n, x, y, step);
                } else {
                    carve_graded(heights, &mut level.correction, parents, n, x, y, self.downcutting);
                }
            }
        }
    }
}

/// The parent of (`x`, `y`) as the engine reads it (`(int)(float)` of each part).
fn parent_of(parents: &[(i32, i32)], n: i32, x: i32, y: i32) -> (i32, i32) {
    let (px, py) = parents[(y * n + x) as usize];
    ((px as f32) as i32, (py as f32) as i32)
}

/// Downcutting <= 0: lower each cell along the path to the pit's level
/// (falling `step` per cell) until the path drops below it or meets the
/// border.
fn carve_flat(heights: &[f32], correction: &mut [f32], parents: &[(i32, i32)], n: i32, x: i32, y: i32, step: f32) {
    let mut level = heights[(y * n + x) as usize];
    let (mut nx, mut ny) = parent_of(parents, n, x, y);
    let mut neighbour = heights[(ny * n + nx) as usize];
    if !(level <= neighbour) {
        return;
    }
    loop {
        let index = (ny * n + nx) as usize;
        if level - neighbour < correction[index] {
            correction[index] = level - neighbour;
        }
        if nx == 0 || ny == 0 || nx == n - 1 || ny == n - 1 {
            break;
        }
        level += step;
        let next = parent_of(parents, n, nx, ny);
        nx = next.0;
        ny = next.1;
        neighbour = heights[(ny * n + nx) as usize];
        if neighbour < level {
            break;
        }
    }
}

/// The downcut channel (Downcutting > 0): find where the path's average fall
/// from the pit is steepest, then grade the channel down to that point.
fn carve_graded(heights: &[f32], correction: &mut [f32], parents: &[(i32, i32)], n: i32, x: i32, y: i32, downcutting: f32) {
    let start = heights[(y * n + x) as usize];
    let mut best = 0.0f32;
    let mut steps = 0i32;
    let mut cx = x;
    let mut cy = y;
    let target;
    loop {
        steps += 1;
        let count = steps as f32;
        let next = parent_of(parents, n, cx, cy);
        cx = next.0;
        cy = next.1;
        let value = heights[(cy * n + cx) as usize];
        let mut candidate = best;
        if value < start {
            candidate = (value - start) / (count - 1.0);
            if best < candidate {
                target = value;
                break;
            }
        }
        let at_border = cx == 0 || cy == 0 || cx == n - 1 || cy == n - 1;
        if at_border {
            target = if value <= start { value } else { start };
            break;
        }
        best = candidate;
    }
    let drop = target - start;
    let count = steps as f32;
    let mut cx = x;
    let mut cy = y;
    for step in 1..=steps {
        let next = parent_of(parents, n, cx, cy);
        cx = next.0;
        cy = next.1;
        let index = (cy * n + cx) as usize;
        let value = heights[index];
        let graded = (((step as f32) * drop) / count) * downcutting + start;
        let carved = if graded <= value { graded } else { value };
        if carved - value < correction[index] {
            correction[index] = carved - value;
        }
    }
}

/// The pit raise at one interior cell: a cell lower than all eight
/// neighbours rises almost to the lowest one.
pub(crate) fn raise_pit(data: &mut [f32], n: i32, x: i32, y: i32) {
    let at = |data: &[f32], cx: i32, cy: i32| data[(cy * n + cx) as usize];
    let centre = at(data, x, y);
    let mut lowest = lower(at(data, x - 1, y - 1), at(data, x, y - 1));
    lowest = lower(lowest, at(data, x + 1, y - 1));
    lowest = lower(lowest, at(data, x - 1, y));
    lowest = lower(lowest, at(data, x + 1, y));
    lowest = lower(lowest, at(data, x - 1, y + 1));
    lowest = lower(lowest, at(data, x, y + 1));
    lowest = lower(lowest, at(data, x + 1, y + 1));
    if centre < lowest {
        data[(y * n + x) as usize] = (lowest - centre) * PIT_RAISE + centre;
    }
}

/// The pit raise over every interior cell, in place row by row.
pub(crate) fn raise_pits(data: &mut [f32], size: usize) {
    let n = size as i32;
    for y in 1..n - 1 {
        for x in 1..n - 1 {
            raise_pit(data, n, x, y);
        }
    }
}

/// A priority flood from the border.
/// Returns each cell's parent: the cell it was reached from. Rivers'
/// version also accumulates `vectors`, each cell's flow
/// direction weighted by the height step that reached it.
pub(crate) fn drainage_tree(heights: &[f32], size: usize, mut vectors: Option<&mut [[f32; 2]]>) -> Vec<(i32, i32)> {
    let n = size as i32;
    let at = |x: i32, y: i32| heights[(y * n + x) as usize];
    let mut state = vec![UNVISITED; size * size];
    let mut parents = vec![(0i32, 0i32); size * size];
    let mut queue = Queue::new();
    for x in 0..n {
        queue.push(at(x, 0), x, 0);
        queue.push(at(x, n - 1), x, n - 1);
    }
    for y in 1..n - 1 {
        queue.push(at(0, y), 0, y);
        queue.push(at(n - 1, y), n - 1, y);
    }

    while let Some(cell) = queue.heap.pop() {
        let (x, y) = (cell.x, cell.y);
        let index = (y * n + x) as usize;
        if state[index] > QUEUED {
            continue;
        }
        state[index] = DONE;
        let left = (x - 1).max(0);
        let right = (x + 1).min(n - 1);
        let top = (y - 1).max(0);
        let bottom = (y + 1).min(n - 1);
        let centre = at(x, y);
        let mut slope = f32::MAX;
        for ny in top..=bottom {
            for nx in left..=right {
                if state[(ny * n + nx) as usize] != UNVISITED {
                    continue;
                }
                let weight = if nx == x || ny == y { 1.0 } else { DIAGONAL };
                let candidate = (at(nx, ny) - centre) * weight;
                slope = lower(slope, candidate);
            }
        }
        let mut add = |nx: i32, ny: i32, state: &mut Vec<u8>, queue: &mut Queue, vectors: &mut Option<&mut [[f32; 2]]>| {
            let neighbour = (ny * n + nx) as usize;
            state[neighbour] = QUEUED;
            parents[neighbour] = (x, y);
            queue.push(at(nx, ny), nx, ny);
            if let Some(vectors) = vectors.as_deref_mut() {
                let step = (at(x, y) - at(nx, ny)).abs();
                let vector = &mut vectors[neighbour];
                if nx != x && ny != y {
                    vector[0] += (x - nx) as f32 * step;
                    vector[1] += (y - ny) as f32 * step;
                } else if ny == y {
                    let offset = (nx - x) as f32;
                    vector[0] -= (offset + offset) * step;
                } else {
                    let offset = (ny - y) as f32;
                    vector[1] -= (offset + offset) * step;
                }
            }
        };
        for ny in top..=bottom {
            for nx in left..=right {
                if state[(ny * n + nx) as usize] != UNVISITED {
                    continue;
                }
                let orthogonal = nx == x || ny == y;
                if orthogonal {
                    add(nx, ny, &mut state, &mut queue, &mut vectors);
                    continue;
                }
                if 0.0 <= slope {
                    // A diagonal step is skipped when a queued side cell
                    // already drains the centre better.
                    let diagonal_fall = (centre - at(nx, ny)) * DIAGONAL;
                    let blocked = [(x, ny), (nx, y)].iter().any(|&(sx, sy)| state[(sy * n + sx) as usize] == QUEUED && diagonal_fall > centre - at(sx, sy));
                    if !blocked {
                        add(nx, ny, &mut state, &mut queue, &mut vectors);
                    }
                } else if !(slope < at(nx, ny) - centre) {
                    add(nx, ny, &mut state, &mut queue, &mut vectors);
                }
            }
        }
    }
    parents
}

/// A 3x3 blur of the correction that can only lower it.
pub(crate) fn smooth(source: &[f32], target: &mut [f32], size: usize) {
    let n = size as i32;
    for y in 0..n {
        let top = (y - 1).max(0);
        let bottom = (y + 1).min(n - 1);
        for x in 0..n {
            let left = (x - 1).max(0);
            let right = (x + 1).min(n - 1);
            let at = |cx: i32, cy: i32| source[(cy * n + cx) as usize];
            let centre = at(x, y);
            let blurred = SMOOTH_CORNER * at(left, top) + 0.0 + SMOOTH_EDGE * at(x, top) + SMOOTH_CORNER * at(right, top) + SMOOTH_EDGE * at(left, y) + centre * SMOOTH_CENTRE + SMOOTH_EDGE * at(right, y) + SMOOTH_CORNER * at(left, bottom) + SMOOTH_EDGE * at(x, bottom) + SMOOTH_CORNER * at(right, bottom);
            target[(y * n + x) as usize] = if blurred <= centre { blurred } else { centre };
        }
    }
}
