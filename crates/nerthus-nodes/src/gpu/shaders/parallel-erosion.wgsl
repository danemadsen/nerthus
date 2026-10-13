// The hydraulic erosion on the GPU (parallel_erosion.rs, the native
// ParallelErosion_Erode): a round's droplets spawned, each step of their
// walk (advance, then settle into fixed-point sums), the sums applied to
// the cells they touched, the shaping pass, and the pyramid between
// levels.
//
// The CPU code applies a cell's sums when the last droplet through it
// settles. Every visit in a step is settled in the same step, so here the
// cells a step touched are listed as they are first touched and applied
// after the settle kernel: the same sums, applied once.

struct Parameters {
    width: u32,
    height: u32,
    coarse_width: u32,
    coarse_height: u32,
    // The round: its first droplet, how many, and the seed.
    first_droplet: u32,
    droplet_count: u32,
    seed: i32,
    has_precipitation: u32,
    // Settle.
    cell_size: f32,
    downcutting: f32,
    settings_downcutting: f32,
    downcutting_type: f32,
    suspended_tangent: f32,
    suspended_load_amount: f32,
    bed_tangent: f32,
    bed_load_amount: f32,
    coarse_tangent: f32,
    coarse_sediments_amount: f32,
    extra_deposition_boost: f32,
    boost: f32,
    no_erosion: u32,
    // Shaping.
    shape: f32,
    shape_sharpness: f32,
    relaxation: f32,
    // Refinement: metres of height range per coarse cell.
    refine_cell_size: f32,
    _padding_0: u32,
    _padding_1: u32,
    _padding_2: u32,
}

// A droplet (512 bytes natively): its position, its water surface and the
// path of its current step.
struct Droplet {
    position: vec2<f32>,
    level: f32,
    previous_level: f32,
    sediment: f32,
    water: f32,
    total_steps: i32,
    uphill_steps: i32,
    direction_index: u32,
    path_length: u32,
    // Bit 0: stuck; bit 1: finished; bit 2: to settle this step.
    flags: u32,
    _padding: u32,
    path: array<vec2<f32>, 12>,
    heights: array<f32, 12>,
    rises: array<f32, 12>,
}

@group(0) @binding(0) var<uniform> parameters: Parameters;
@group(0) @binding(1) var<storage, read_write> heights: array<f32>;
@group(0) @binding(2) var<storage, read> precipitation: array<f32>;
@group(0) @binding(3) var<storage, read> directions: array<vec2<f32>, 32>;
@group(0) @binding(4) var<storage, read_write> droplets: array<Droplet>;
// Visits per cell (the flow).
@group(0) @binding(5) var<storage, read_write> flow: array<atomic<u32>>;
@group(0) @binding(6) var<storage, read_write> sums: array<atomic<i32>>;
@group(0) @binding(7) var<storage, read_write> weights: array<atomic<i32>>;
// The cells a step touched, and the counters: [0] droplets still moving,
// [1] cells touched.
@group(0) @binding(8) var<storage, read_write> touched: array<u32>;
@group(0) @binding(9) var<storage, read_write> counters: array<atomic<u32>, 4>;
@group(0) @binding(10) var<storage, read_write> deposition: array<f32>;
@group(0) @binding(11) var<storage, read_write> wear: array<f32>;
@group(0) @binding(12) var<storage, read_write> shaping: array<f32>;
// Wetness and its spread (precipitation only).
@group(0) @binding(13) var<storage, read_write> wetness: array<f32>;
@group(0) @binding(14) var<storage, read_write> spread: array<f32>;
// The pyramid: a fine plane and a coarse one.
@group(0) @binding(15) var<storage, read> fine_plane: array<f32>;
@group(0) @binding(16) var<storage, read_write> coarse_plane: array<f32>;
// Refinement reads the coarse level's planes.
@group(0) @binding(17) var<storage, read> coarse_heights: array<f32>;
@group(0) @binding(18) var<storage, read> coarse_deposition: array<f32>;
@group(0) @binding(19) var<storage, read> coarse_wear: array<f32>;
@group(0) @binding(20) var<storage, read> coarse_wetness: array<f32>;
@group(0) @binding(21) var<storage, read> coarse_spread: array<f32>;
@group(0) @binding(22) var<storage, read_write> detail: array<f32>;

// Steps a droplet takes per step of the simulation.
const STEPS_PER_ROUND: u32 = 11u;
// Height changes accumulate in 8.24 fixed point.
const FIXED_SCALE: f32 = 16777216.0;
const FIXED_UNIT: f32 = 5.96046448e-8;
// A droplet's water level before its first step, and the value above which
// a level counts as set.
const LEVEL_UNSET: f32 = -999999.875;
const LEVEL_SET_ABOVE: f32 = -999999.0;
const HASH_FALLBACK: u32 = 0xA2351342u;
const HASH_MULTIPLIER: u32 = 0x2545F381u;
const POSITION_SALT: u32 = 0x1B227u;
const PRECIPITATION_SALT: u32 = 0x1B253u;
const UNIT_SCALE: f32 = 2.32830644e-10;
const DIAGONAL_WEIGHT: f32 = 0.707099974;
const MEAN_NORMALISATION: f32 = 0.146447197;
const SIDE_DROP: f32 = 0.357735038;
const SQRT_2: f32 = 1.41421354;

const STUCK: u32 = 1u;
const FINISHED: u32 = 2u;
const SETTLING: u32 = 4u;

fn cell(x: i32, y: i32) -> u32 {
    return u32(y * i32(parameters.width) + x);
}

fn xorshift(state: u32) -> u32 {
    var value = state ^ (state << 13u);
    value ^= value >> 17u;
    return value ^ (value << 5u);
}

fn hash_start(salt: u32, droplet: u32) -> u32 {
    let mixed = bitcast<u32>(parameters.seed) * salt + droplet;
    return select(HASH_FALLBACK, mixed, mixed != 0u);
}

fn unit_float(state: u32) -> f32 {
    return f32(state * HASH_MULTIPLIER) * UNIT_SCALE;
}

// A round's droplets at hashed positions; with a precipitation mask each
// falls with the mask's probability (the rest start finished).
@compute @workgroup_size(256)
fn spawn(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let offset = linear_index(id, groups);
    if offset >= parameters.droplet_count {
        return;
    }
    let droplet_id = parameters.first_droplet + offset;
    let first = xorshift(hash_start(POSITION_SALT, droplet_id));
    let second = xorshift(first);
    let x = unit_float(first) * f32(parameters.width);
    let y = unit_float(second) * f32(parameters.height);
    var flags = 0u;
    if parameters.has_precipitation != 0u {
        let chance = unit_float(xorshift(hash_start(PRECIPITATION_SALT, droplet_id)));
        if !(chance < precipitation[cell(i32(x), i32(y))]) {
            flags = FINISHED;
        }
    }
    var droplet: Droplet;
    droplet.position = vec2<f32>(x, y);
    droplet.level = LEVEL_UNSET;
    droplet.previous_level = 0.0;
    droplet.sediment = 0.0;
    droplet.water = 0.0;
    droplet.total_steps = 0;
    droplet.uphill_steps = 0;
    droplet.direction_index = parameters.first_droplet & 31u;
    droplet.path_length = 0u;
    droplet.flags = flags;
    droplets[offset] = droplet;
}

// The surface under a point at least half a cell inside the map: its
// bilinear height, gradient and slope.
struct Surface {
    height: f32,
    gradient: vec2<f32>,
    slope: f32,
}

fn surface_at(x: f32, y: f32) -> Surface {
    let cell_x = i32(x - 0.5);
    let cell_y = i32(y - 0.5);
    let fraction_x = (x - f32(cell_x)) - 0.5;
    let fraction_y = (y - f32(cell_y)) - 0.5;
    let top_left = heights[cell(cell_x, cell_y)];
    let top_right = heights[cell(cell_x + 1, cell_y)];
    let bottom_left = heights[cell(cell_x, cell_y + 1)];
    let bottom_right = heights[cell(cell_x + 1, cell_y + 1)];
    let top = (1.0 - fraction_x) * top_left + fraction_x * top_right;
    let bottom = (1.0 - fraction_x) * bottom_left + fraction_x * bottom_right;
    var surface: Surface;
    surface.height = top * (1.0 - fraction_y) + bottom * fraction_y;
    let gradient_x = (top_right - top_left) * (1.0 - fraction_y) + (bottom_right - bottom_left) * fraction_y;
    let gradient_y = (bottom_left - top_left) * (1.0 - fraction_x) + (bottom_right - top_right) * fraction_x;
    surface.gradient = vec2<f32>(gradient_x, gradient_y);
    surface.slope = sqrt(gradient_y * gradient_y + gradient_x * gradient_x);
    return surface;
}

fn inside(x: f32, y: f32) -> bool {
    let limit_x = f32(parameters.width) - 1.5;
    let limit_y = f32(parameters.height) - 1.5;
    return i32(x - 0.5) >= 0 && i32(y - 0.5) >= 0 && x < limit_x && y < limit_y;
}

// Up to eleven steps downhill, with inertia and a little jitter, counting
// every cell visited.
@compute @workgroup_size(256)
fn advance(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let offset = linear_index(id, groups);
    if offset >= parameters.droplet_count {
        return;
    }
    var droplet = droplets[offset];
    if (droplet.flags & FINISHED) != 0u {
        return;
    }
    droplet.flags |= SETTLING;
    var x = droplet.position.x;
    var y = droplet.position.y;
    droplet.path_length = 0u;
    var finished = false;
    var stuck = (droplet.flags & STUCK) != 0u;
    if !inside(x, y) {
        droplet.flags |= FINISHED;
        droplets[offset] = droplet;
        return;
    }
    let start = surface_at(x, y);
    if !(start.slope > 0.0) {
        droplet.flags |= FINISHED;
        droplets[offset] = droplet;
        return;
    }
    var direction = vec2<f32>(-start.gradient.x / start.slope, -start.gradient.y / start.slope);
    var height = start.height;
    var water_level = height;
    if LEVEL_SET_ABOVE < droplet.level {
        water_level = droplet.level;
    }
    droplet.previous_level = height;
    if LEVEL_SET_ABOVE < water_level {
        droplet.previous_level = water_level;
    }
    droplet.level = water_level;

    var climbing = 0.0;
    var stop = false;
    var left_map = false;
    var step_start = vec2<f32>(x, y);
    var step = 0u;
    loop {
        droplet.path[step] = step_start;
        droplet.heights[step] = height;
        droplet.rises[step] = 0.0;
        atomicAdd(&flow[cell(i32(x), i32(y))], 1u);

        x += direction.x;
        y += direction.y;
        if !inside(x, y) {
            finished = true;
            left_map = true;
            stop = true;
        } else {
            finished = false;
            let surface = surface_at(x, y);
            droplet.direction_index = (droplet.direction_index + 1u) & 31u;
            if surface.slope > 0.0 {
                var inertia = 1.0;
                if climbing != 0.0 {
                    inertia = 30.0;
                }
                let jitter = directions[droplet.direction_index];
                let pull_x = -surface.gradient.x / surface.slope;
                let pull_y = -surface.gradient.y / surface.slope;
                let next_x = (0.5 * direction.x * inertia + 0.25 * jitter.x) + pull_x;
                let next_y = (0.5 * direction.y * inertia + 0.25 * jitter.y) + pull_y;
                let length = sqrt(next_y * next_y + next_x * next_x);
                let inverse = 1.0 / length;
                direction = vec2<f32>(inverse * next_x, inverse * next_y);
            } else {
                direction = vec2<f32>(0.0, 0.0);
                stop = true;
            }
            if surface.height >= height {
                droplet.uphill_steps += 1;
                climbing = 1.0;
                droplet.rises[step] = surface.height - height;
                droplet.heights[step] = surface.height;
                if droplet.uphill_steps >= 5 || droplet.total_steps + i32(step) <= 2 {
                    stop = true;
                    stuck = true;
                }
            } else {
                climbing = 0.0;
                droplet.rises[step] = 0.0;
            }
            height = surface.height;
        }
        step += 1u;
        if step == STEPS_PER_ROUND || stop {
            break;
        }
        step_start = vec2<f32>(x, y);
    }

    if climbing != 0.0 {
        stuck = true;
    }
    var last = step;
    if stuck || left_map {
        droplet.position = step_start;
        last = step - 1u;
    } else {
        droplet.path[step] = vec2<f32>(x, y);
        droplet.heights[step] = height;
        droplet.rises[step] = 0.0;
        droplet.position = vec2<f32>(x, y);
        atomicAdd(&flow[cell(i32(x), i32(y))], 1u);
    }
    droplet.path_length = last + 1u;
    droplet.total_steps += i32(last) + 1;
    if stuck {
        droplet.flags |= STUCK;
        finished = true;
    }
    if finished {
        droplet.flags |= FINISHED;
    } else {
        atomicAdd(&counters[0], 1u);
    }
    droplets[offset] = droplet;
}

// Erosion and deposition along a droplet's path, into the fixed-point
// sums of the cells it passed.
@compute @workgroup_size(256)
fn settle(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let offset = linear_index(id, groups);
    if offset >= parameters.droplet_count {
        return;
    }
    var droplet = droplets[offset];
    if (droplet.flags & SETTLING) == 0u {
        return;
    }
    droplet.flags &= ~SETTLING;
    let length = droplet.path_length;
    if length == 0u {
        droplets[offset] = droplet;
        return;
    }
    let cell_size = parameters.cell_size;
    let stuck = (droplet.flags & STUCK) != 0u;
    let last = length - 1u;
    let floor_bias = droplet.heights[last] * 0.05;
    for (var index = 0u; index < length; index++) {
        let position = droplet.path[index];
        let at = cell(i32(position.x), i32(position.y));
        let before = droplet.heights[max(index, 1u) - 1u];
        let after = droplet.heights[min(index + 1u, last)];
        let current = droplet.heights[index];
        let far_before = droplet.heights[max(index, 2u) - 2u];
        let far_after = droplet.heights[min(index + 2u, last)];

        let slope = ((before - after) * 0.5) / cell_size;
        let downhill = max(0.0, slope);
        var carried = 1.0;
        if !(parameters.suspended_tangent > downhill * parameters.suspended_load_amount) {
            carried = 0.5 / (downhill * 4.0 + 1.0);
        }
        let mean3 = ((current + before) + after) / 3.0;
        let mean5 = ((((before + far_before) + current) + after) + far_after) / 5.0;
        let rise3 = mean3 - current;
        let rise5 = mean5 - current;
        let fill = max(0.0, max(rise5, rise3));
        var amount = fill * carried;
        var smoothed = mean3;
        if 0.0 > rise3 && 0.0 > rise5 {
            smoothed = mean5;
        }

        let floor_height = far_after * 0.95 + floor_bias;
        let sediment = droplet.sediment;
        let lowest = min(current, smoothed);
        let downcutting_type = parameters.downcutting_type;
        let relative_cut = (lowest - floor_height) * parameters.downcutting * (1.0 - downcutting_type);
        let constant_cut = downcutting_type * parameters.settings_downcutting * cell_size * 0.15;
        let cut = relative_cut + constant_cut;
        if sediment > cut + fill {
            let coarse = parameters.coarse_tangent;
            if slope > coarse * 0.8 && coarse * 1.25 > slope {
                let dropped = sediment * 0.5 * parameters.coarse_sediments_amount * 0.5;
                amount += min(dropped, cut);
            }
            if parameters.bed_tangent > slope {
                var steepness = 1.0;
                if downhill < parameters.bed_tangent * 0.1 {
                    steepness = 100.0;
                }
                let dropped = sediment * 0.3 * parameters.bed_load_amount * 0.5 * steepness;
                amount += min(dropped, cut);
            }
        }

        let water = droplet.water;
        let target_height = current - (current - (lowest - cut));
        let water_level = droplet.level;
        let deposit = min(min((water_level - target_height) * 0.9, amount), sediment);
        // The factor that would scale suspended-load removal by the flow
        // is zero.
        let removal = min((droplet.previous_level - (deposit + target_height)) * 0.95, 0.0);
        droplet.previous_level = water_level;
        let kept = (1.0 - parameters.extra_deposition_boost) * removal;
        let total = removal + deposit;
        droplet.water = water - kept;
        let raised = max(0.0, total * parameters.boost);
        droplet.level = raised + target_height;
        let gain = droplet.level - current;
        var used = gain * 1.05;
        if 0.0 > gain {
            used = gain;
        }
        let returned = (raised - total) * parameters.extra_deposition_boost;
        droplet.sediment = max(0.0, returned + (sediment - used));

        var weight = 2;
        if index == 0u || index == last || stuck {
            weight = 1;
        }
        var flowing = 1.0;
        if stuck {
            flowing = 0.0;
        }
        var settled = 0.0;
        if droplet.total_steps > 3 {
            settled = 1.0;
        }
        var change = (f32(weight * weight) * 0.5) * flowing * settled * gain;
        let rise = droplet.rises[index];
        if rise > 0.0 {
            change = max(rise, change);
        }
        atomicAdd(&sums[at], i32(change * FIXED_SCALE));
        let earlier = atomicAdd(&weights[at], weight);
        if earlier == 0 {
            let slot = atomicAdd(&counters[1], 1u);
            touched[slot] = at;
        }
    }
    droplets[offset] = droplet;
}

// The weighted mean change of each cell the step touched, applied.
@compute @workgroup_size(256)
fn apply(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let slot = linear_index(id, groups);
    if slot >= atomicLoad(&counters[1]) {
        return;
    }
    let at = touched[slot];
    let sum = atomicExchange(&sums[at], 0);
    let weight = atomicExchange(&weights[at], 0);
    var delta = (f32(sum) * FIXED_UNIT) / f32(weight);
    if parameters.no_erosion != 0u {
        delta = max(0.0, delta);
    }
    heights[at] += delta;
    var deposited = delta * 0.95;
    if delta > 0.0 {
        deposited = delta;
    }
    deposition[at] = max(deposited + deposition[at], 0.0);
    let worn = min(0.0, delta);
    wear[at] += -worn;
}

// A cell and its eight neighbours, clamped at the edges.
struct Neighbourhood {
    top_left: f32,
    top: f32,
    top_right: f32,
    left: f32,
    center: f32,
    right: f32,
    bottom_left: f32,
    bottom: f32,
    bottom_right: f32,
}

fn heights_around(x: i32, y: i32) -> Neighbourhood {
    let above = max(y - 1, 0);
    let below = min(y + 1, i32(parameters.height) - 1);
    let left = max(x - 1, 0);
    let right = min(x + 1, i32(parameters.width) - 1);
    var n: Neighbourhood;
    n.top_left = heights[cell(left, above)];
    n.top = heights[cell(x, above)];
    n.top_right = heights[cell(right, above)];
    n.left = heights[cell(left, y)];
    n.center = heights[cell(x, y)];
    n.right = heights[cell(right, y)];
    n.bottom_left = heights[cell(left, below)];
    n.bottom = heights[cell(x, below)];
    n.bottom_right = heights[cell(right, below)];
    return n;
}

fn wetness_around(x: i32, y: i32) -> Neighbourhood {
    let above = max(y - 1, 0);
    let below = min(y + 1, i32(parameters.height) - 1);
    let left = max(x - 1, 0);
    let right = min(x + 1, i32(parameters.width) - 1);
    var n: Neighbourhood;
    n.top_left = wetness[cell(left, above)];
    n.top = wetness[cell(x, above)];
    n.top_right = wetness[cell(right, above)];
    n.left = wetness[cell(left, y)];
    n.center = wetness[cell(x, y)];
    n.right = wetness[cell(right, y)];
    n.bottom_left = wetness[cell(left, below)];
    n.bottom = wetness[cell(x, below)];
    n.bottom_right = wetness[cell(right, below)];
    return n;
}

// The shaping change at one cell: slopes steeper than a cell relax towards
// their lower neighbours, the rest towards the local mean.
fn shaping_at(cells: Neighbourhood) -> f32 {
    let cell_size = parameters.cell_size;
    let diagonals = (((cells.top_right + cells.top_left) + cells.bottom_left) + cells.bottom_right) * DIAGONAL_WEIGHT;
    let sides = ((cells.left + cells.top) + cells.right) + cells.bottom;
    let mean = (diagonals + sides) * MEAN_NORMALISATION;
    let center = cells.center;
    let above_mean = center - mean;

    var total_weight = 0.0;
    var pull = 0.0;
    var neighbours = array<f32, 8>(cells.top_left, cells.top, cells.top_right, cells.left, cells.right, cells.bottom_left, cells.bottom, cells.bottom_right);
    for (var index = 0; index < 8; index++) {
        let neighbour = neighbours[index];
        if center > neighbour {
            let difference = center - neighbour;
            let weight = difference / cell_size;
            total_weight += weight;
            pull -= weight * difference;
        }
    }
    if total_weight > 0.0 {
        pull /= total_weight;
    }

    let sharpness = parameters.shape_sharpness;
    let sharp_share = sharpness * sharpness * 0.9;
    let lower_row = ((cells.bottom + cells.bottom_left) + cells.bottom) + cells.bottom_right;
    let upper_row = ((cells.top + cells.top_left) + cells.top) + cells.top_right;
    let gradient_y = (lower_row - upper_row) * 0.125;
    let right_column = ((cells.right + cells.top_right) + cells.right) + cells.bottom_right;
    let left_column = ((cells.left + cells.top_left) + cells.left) + cells.bottom_left;
    let gradient_x = (right_column - left_column) * 0.125;
    let gradient = sqrt(gradient_x * gradient_x + gradient_y * gradient_y);
    let steepness = (gradient * (1.0 - sharp_share) - sharp_share * pull) * sharpness;
    let ridge = max(0.0, steepness - cell_size * 0.3) * 0.7;
    let cliff = max(0.0, steepness - cell_size) * 30.0;
    let relax = (1.0 - sharpness) * 0.02 * parameters.relaxation * above_mean;
    let pushed = -(ridge + cliff + relax);
    return max(pull, pushed);
}

// One shaping pass's change per cell, counted as wear.
@compute @workgroup_size(16, 16)
fn reshape(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.width || id.y >= parameters.height {
        return;
    }
    let at = cell(i32(id.x), i32(id.y));
    let shaped = shaping_at(heights_around(i32(id.x), i32(id.y))) * parameters.shape;
    shaping[at] = shaped;
    wear[at] += shaped;
}

// The wetness each cell has seen: the flow through it, capped at one.
@compute @workgroup_size(16, 16)
fn wet(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.width || id.y >= parameters.height {
        return;
    }
    let at = cell(i32(id.x), i32(id.y));
    let soaked = min(1.0, 0.25 * f32(atomicLoad(&flow[at])));
    if soaked > wetness[at] {
        wetness[at] = soaked;
    }
}

fn reach(neighbour: f32, limit: f32, wet: f32) -> f32 {
    if neighbour < limit {
        return wet;
    }
    return 0.0;
}

// The shaping pass under a precipitation mask: shaping only where water
// has run, the wetness spreading to every neighbour a little below.
@compute @workgroup_size(16, 16)
fn reshape_wet(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.width || id.y >= parameters.height {
        return;
    }
    let x = i32(id.x);
    let y = i32(id.y);
    let at = cell(x, y);
    let cells = heights_around(x, y);
    let wet = wetness_around(x, y);
    let side_drop = parameters.cell_size * SIDE_DROP;
    let diagonal_drop = side_drop * SQRT_2;
    let side_limit = cells.center - side_drop;
    let diagonal_limit = cells.center - diagonal_drop;
    let bottom_pair = max(reach(cells.bottom_right, diagonal_limit, wet.bottom_right), reach(cells.bottom, side_limit, wet.bottom));
    let crossed_pair = max(reach(cells.bottom_left, diagonal_limit, wet.bottom_left), reach(cells.right, side_limit, wet.right));
    let side_pair = max(reach(cells.left, side_limit, wet.left), reach(cells.top_right, diagonal_limit, wet.top_right));
    let top_pair = max(reach(cells.top, side_limit, wet.top), reach(cells.top_left, diagonal_limit, wet.top_left));
    let lower = max(bottom_pair, crossed_pair);
    let upper = max(max(side_pair, top_pair), wet.center);
    let spreading = max(lower, upper);
    spread[at] = spreading;
    let shaped = shaping_at(cells) * parameters.shape * spreading;
    shaping[at] = shaped;
    wear[at] += shaped;
}

// The shaping changes added to the heights and cleared.
@compute @workgroup_size(16, 16)
fn apply_shaping(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.width || id.y >= parameters.height {
        return;
    }
    let at = cell(i32(id.x), i32(id.y));
    heights[at] += shaping[at];
    shaping[at] = 0.0;
}

fn fine_at(column: i32, row: i32) -> f32 {
    return fine_plane[u32(row) * parameters.width + u32(column)];
}

// The [1 3 3 1]² / 64 filter around each coarse cell, clamped to the edges.
@compute @workgroup_size(16, 16)
fn downsample(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.coarse_width || id.y >= parameters.coarse_height {
        return;
    }
    let x = i32(id.x);
    let y = i32(id.y);
    let last_x = i32(parameters.width) - 1;
    let last_y = i32(parameters.height) - 1;
    let columns = array<i32, 4>(max(2 * x - 1, 0), 2 * x, min(2 * x + 1, last_x), min(2 * x + 2, last_x));
    let rows = array<i32, 4>(max(2 * y - 1, 0), 2 * y, min(2 * y + 1, last_y), min(2 * y + 2, last_y));
    var sum = fine_at(columns[1], rows[0]) * 3.0 + fine_at(columns[0], rows[0]);
    sum += fine_at(columns[2], rows[0]) * 3.0;
    sum += fine_at(columns[3], rows[0]);
    sum += fine_at(columns[0], rows[1]) * 3.0;
    sum += fine_at(columns[1], rows[1]) * 9.0;
    sum += fine_at(columns[2], rows[1]) * 9.0;
    sum += fine_at(columns[3], rows[1]) * 3.0;
    sum += fine_at(columns[0], rows[2]) * 3.0;
    sum += fine_at(columns[1], rows[2]) * 9.0;
    sum += fine_at(columns[2], rows[2]) * 9.0;
    sum += fine_at(columns[3], rows[2]) * 3.0;
    sum += fine_at(columns[0], rows[3]);
    sum += fine_at(columns[1], rows[3]) * 3.0;
    sum += fine_at(columns[2], rows[3]) * 3.0;
    sum += fine_at(columns[3], rows[3]);
    coarse_plane[id.y * parameters.coarse_width + id.x] = sum * 0.015625;
}

// The 3×3 filter that sharpens a downsampled mask back (corners 1/16,
// edges −3/8, centre 9/4); `fine_plane` is the downsampled mask here.
@compute @workgroup_size(16, 16)
fn sharpen(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.width || id.y >= parameters.height {
        return;
    }
    let x = i32(id.x);
    let y = i32(id.y);
    let above = max(y - 1, 0);
    let below = min(y + 1, i32(parameters.height) - 1);
    let left = max(x - 1, 0);
    let right = min(x + 1, i32(parameters.width) - 1);
    let corners = fine_at(right, above) + fine_at(left, above) + fine_at(left, below) + fine_at(right, below);
    let edges = fine_at(x, above) + fine_at(right, y) + fine_at(left, y) + fine_at(x, below);
    coarse_plane[id.y * parameters.width + id.x] = (corners * 0.0625 - edges * 0.375) + 2.25 * fine_at(x, y);
}

// A half-size plane sampled at fine cell (x, y): the coarse cell under it
// weighted 9, its nearer neighbours 3 and the diagonal 1.
struct Corners {
    near: u32,
    across: u32,
    down: u32,
    diagonal: u32,
}

fn coarse_corners(x: i32, y: i32) -> Corners {
    let center_x = x >> 1u;
    let center_y = y >> 1u;
    let neighbour_x = clamp(center_x + (x & 1) * 2 - 1, 0, i32(parameters.coarse_width) - 1);
    let neighbour_y = clamp(center_y + (y & 1) * 2 - 1, 0, i32(parameters.coarse_height) - 1);
    let width = i32(parameters.coarse_width);
    var corners: Corners;
    corners.near = u32(center_y * width + center_x);
    corners.across = u32(center_y * width + neighbour_x);
    corners.down = u32(neighbour_y * width + center_x);
    corners.diagonal = u32(neighbour_y * width + neighbour_x);
    return corners;
}

fn upsampled(near: f32, across: f32, down: f32, diagonal: f32) -> f32 {
    return (across * 3.0 + near * 9.0 + down * 3.0 + diagonal) * 0.0625;
}

// The detail a level loses to the next coarser one.
@compute @workgroup_size(16, 16)
fn detail_of(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.width || id.y >= parameters.height {
        return;
    }
    let at = id.y * parameters.width + id.x;
    let c = coarse_corners(i32(id.x), i32(id.y));
    let coarse = upsampled(coarse_heights[c.near], coarse_heights[c.across], coarse_heights[c.down], coarse_heights[c.diagonal]);
    detail[at] = fine_plane[at] - coarse;
}

// The finer level takes back the coarser level's eroded surface: deposits
// first fill its detail; where the detail stands above them it is added
// again, scaled by how worn the coarse surface is.
@compute @workgroup_size(16, 16)
fn refine(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.width || id.y >= parameters.height {
        return;
    }
    let at = id.y * parameters.width + id.x;
    let c = coarse_corners(i32(id.x), i32(id.y));
    var height = upsampled(coarse_heights[c.near], coarse_heights[c.across], coarse_heights[c.down], coarse_heights[c.diagonal]);
    let kept_detail = detail[at];
    var deposit = upsampled(coarse_deposition[c.near], coarse_deposition[c.across], coarse_deposition[c.down], coarse_deposition[c.diagonal]);
    let worn = upsampled(coarse_wear[c.near], coarse_wear[c.across], coarse_wear[c.down], coarse_wear[c.diagonal]);
    let cell_size = parameters.refine_cell_size;
    if !(deposit > 0.0) {
        height += kept_detail;
    } else if !(0.0 >= deposit - kept_detail) {
        if kept_detail > 0.0 {
            let fill = min((cell_size + cell_size) / deposit - 0.25, 1.0);
            let filled = fill * kept_detail;
            height += filled;
            deposit -= filled;
        }
    } else {
        let exposure = max(0.1, min(1.0, cell_size / worn));
        height += exposure * (kept_detail - deposit);
        deposit = 0.0;
    }
    deposition[at] = deposit;
    wear[at] = max(0.0, worn - kept_detail);
    heights[at] = height;
}

// Under a precipitation mask, the finer level also takes back the coarser
// one's wetness and its spread.
@compute @workgroup_size(16, 16)
fn refine_wetness(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.width || id.y >= parameters.height {
        return;
    }
    let at = id.y * parameters.width + id.x;
    let c = coarse_corners(i32(id.x), i32(id.y));
    wetness[at] = upsampled(coarse_wetness[c.near], coarse_wetness[c.across], coarse_wetness[c.down], coarse_wetness[c.diagonal]);
    spread[at] = upsampled(coarse_spread[c.near], coarse_spread[c.across], coarse_spread[c.down], coarse_spread[c.diagonal]);
}
