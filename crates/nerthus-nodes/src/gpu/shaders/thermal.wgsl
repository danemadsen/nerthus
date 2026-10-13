// The thermal erosion on the GPU (thermal.rs): the three kernels of an
// isotropic pass, the three of an anisotropic pass, and the finer level
// rebuilt from a simulated coarse one.

struct Parameters {
    width: u32,
    height: u32,
    coarse_width: u32,
    coarse_height: u32,
    // The level's constants (thermal.rs Constants).
    cell: f32,
    inverse_cell: f32,
    talus: f32,
    plasticity: f32,
    anisotropy: f32,
    strength: f32,
    sediment_removal: f32,
    // The pass: the talus tests (0 when off), its share of the sediment
    // balance, and the rate it wears at.
    threshold: f32,
    threshold_raw: f32,
    step: f32,
    rate: f32,
    // Whether the area and removal masks count on this level.
    area_used: u32,
    removal_used: u32,
    // Reconstruction: metres across a coarse pixel per metre of height.
    coarse_cell: f32,
    _padding_0: u32,
    _padding_1: u32,
}

@group(0) @binding(0) var<uniform> parameters: Parameters;
// The level a pass reads.
@group(0) @binding(1) var<storage, read> heights: array<f32>;
@group(0) @binding(2) var<storage, read> area: array<f32>;
@group(0) @binding(3) var<storage, read_write> slope: array<f32>;
@group(0) @binding(4) var<storage, read_write> sediment: array<f32>;
@group(0) @binding(5) var<storage, read_write> wear: array<f32>;
// The anisotropic tilt of each pixel along its slope.
@group(0) @binding(6) var<storage, read_write> tilt: array<vec2<f32>>;
@group(0) @binding(7) var<storage, read_write> drops: array<f32>;
// What a pass writes: the sediment (first the outflow) and the heights.
@group(0) @binding(8) var<storage, read_write> next_sediment: array<f32>;
@group(0) @binding(9) var<storage, read_write> next_heights: array<f32>;
@group(0) @binding(10) var<storage, read> removal: array<f32>;
// Reconstruction reads the simulated coarse level, the finer level's
// detail and its removal mask's detail, and writes the finer level.
@group(0) @binding(11) var<storage, read> coarse_heights: array<f32>;
@group(0) @binding(12) var<storage, read> coarse_wear: array<f32>;
@group(0) @binding(13) var<storage, read> coarse_sediment: array<f32>;
@group(0) @binding(14) var<storage, read> detail: array<f32>;
@group(0) @binding(15) var<storage, read> removal_detail: array<f32>;
@group(0) @binding(16) var<storage, read_write> fine_heights: array<f32>;
@group(0) @binding(17) var<storage, read_write> fine_sediment: array<f32>;
@group(0) @binding(18) var<storage, read_write> fine_wear: array<f32>;

// The weight of a diagonal neighbour's drop (0.7071, not 1/√2).
const DIAGONAL: f32 = 0.7071;
const MINUS_SIXTH: f32 = -0.166666672;
const MINUS_TWELFTH: f32 = -0.0833333358;

// A pixel's 3×3 neighbourhood, clamped at the edges (columns clamp
// against the row count and rows against the column count, the same on
// square maps).
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

fn height_at(column: u32, row: u32) -> f32 {
    return heights[row * parameters.width + column];
}

fn neighbourhood(x: u32, y: u32) -> Neighbourhood {
    let width = parameters.width;
    let height = parameters.height;
    let left_x = max(x, 1u) - 1u;
    let right_x = min(min(x + 1u, height - 1u), width - 1u);
    let top_y = max(y, 1u) - 1u;
    let bottom_y = min(min(y + 1u, width - 1u), height - 1u);
    var n: Neighbourhood;
    n.top_left = height_at(left_x, top_y);
    n.top = height_at(x, top_y);
    n.top_right = height_at(right_x, top_y);
    n.left = height_at(left_x, y);
    n.centre = height_at(x, y);
    n.right = height_at(right_x, y);
    n.bottom_left = height_at(left_x, bottom_y);
    n.bottom = height_at(x, bottom_y);
    n.bottom_right = height_at(right_x, bottom_y);
    return n;
}

// The gradient (3-10-3 weights over 32) and the slope, the steeper of the
// gradient's length and 0.7 of the steepest drop.
struct Measure {
    gradient: vec2<f32>,
    slope: f32,
}

fn measure(n: Neighbourhood) -> Measure {
    let diagonal = (n.top_right - (n.bottom_left + n.top_left)) + n.bottom_right;
    let across = (n.right - n.left) * 10.0;
    let gradient_x = fma(3.0, diagonal, across) * 0.03125;
    let top_corners = n.top_right + n.top_left;
    let bottom_corners = n.bottom_right + n.bottom_left;
    let vertical = top_corners - bottom_corners;
    let fall = (n.top - n.bottom) * 10.0;
    let gradient_y = fma(3.0, vertical, fall) * -0.03125;
    let length = sqrt(fma(gradient_y, gradient_y, gradient_x * gradient_x));

    let c = n.centre;
    let right_or_bottom = max(c - n.right, c - n.bottom);
    let top_or_left = max(c - n.top, c - n.left);
    let straight = max(max(top_or_left, right_or_bottom), 0.0);
    let bottom_corner = max(c - n.bottom_left, c - n.bottom_right);
    let top_corner = max(c - n.top_left, c - n.top_right);
    let corner = max(bottom_corner, top_corner) * DIAGONAL;
    let steepest = max(straight, corner) * 0.7;
    var result: Measure;
    result.gradient = vec2<f32>(gradient_x, gradient_y);
    result.slope = max(length, steepest);
    return result;
}

// How readily steep ground wears: 1 − plasticity, lowered with the slope
// (more on convex ground) and blended by anisotropy.
fn erodibility(n: Neighbourhood, tangent: f32) -> f32 {
    let sides = (n.left + n.right) + (n.top + n.bottom);
    let corners = (n.bottom_right + n.bottom_left) + (n.top_right + n.top_left);
    var curvature = fma(sides, MINUS_SIXTH, n.centre);
    curvature = fma(corners, MINUS_TWELFTH, curvature);
    let convexity = parameters.inverse_cell * curvature;
    let plasticity = parameters.plasticity;
    let base = 1.0 - plasticity;
    var factor = base;
    if convexity > 0.0 {
        let boost = fma(plasticity, 1.6, base);
        let scaled = plasticity * convexity;
        let shape = fma(plasticity, 13.0, 7.0);
        let product = scaled * shape;
        factor = fma(product, boost, boost);
    }
    let lowered = fma(-tangent, factor, base);
    return fma(-lowered, parameters.anisotropy, base);
}

// The sediment a pixel wears loose this pass, capped by its slope.
fn wear_amount(erodibility: f32, tangent: f32, held: f32) -> f32 {
    var hardness = fma(parameters.inverse_cell * -0.15, held, 1.0);
    hardness = max(hardness, 0.0) * parameters.rate;
    var erosion = hardness * (erodibility * parameters.cell);
    erosion = erosion * parameters.strength;
    let cap = tangent * parameters.cell;
    return min(erosion, cap);
}

// The first isotropic kernel: the slope, and the surface worn into
// sediment where it is steeper than the talus.
@compute @workgroup_size(16, 16)
fn wear_pass(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.width || id.y >= parameters.height {
        return;
    }
    let pixel = id.y * parameters.width + id.x;
    let n = neighbourhood(id.x, id.y);
    let measured = measure(n);
    slope[pixel] = measured.slope;
    let tangent = parameters.inverse_cell * measured.slope;
    let held = sediment[pixel];
    var wearing = 0.0;
    if tangent > parameters.threshold {
        wearing = erodibility(n, tangent);
    }
    var amount = wear_amount(wearing, tangent, held);
    if parameters.area_used != 0u {
        amount *= area[pixel];
    }
    sediment[pixel] = amount + held;
    wear[pixel] += amount;
}

// The first anisotropic kernel: the wear as in `wear_pass`, and how far
// each pixel tilts along its slope where steeper than half the talus.
@compute @workgroup_size(16, 16)
fn tilt_wear_pass(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.width || id.y >= parameters.height {
        return;
    }
    let pixel = id.y * parameters.width + id.x;
    let n = neighbourhood(id.x, id.y);
    let measured = measure(n);
    let tangent = measured.slope * parameters.inverse_cell;
    let talus = parameters.talus;
    var tilted = vec2<f32>(0.0, 0.0);
    if tangent > talus * 0.5 {
        let ratio = talus / tangent;
        var weight = 1.0;
        if talus < tangent {
            weight = ratio * ratio;
        }
        let scale = weight * ratio;
        tilted = vec2<f32>(scale * measured.gradient.x, scale * measured.gradient.y);
    }
    tilt[pixel] = tilted;

    let held = sediment[pixel];
    var wearing = 0.0;
    if tangent > talus {
        wearing = erodibility(n, tangent);
    }
    var amount = wear_amount(wearing, tangent, held);
    if parameters.area_used != 0u {
        amount *= area[pixel];
    }
    sediment[pixel] = held + amount;
    wear[pixel] += amount;
}

// A drop to a neighbour, never negative.
fn drop_to(centre: f32, column: u32, row: u32, weight: f32) -> f32 {
    let fall = (centre - height_at(column, row)) * weight;
    return max(fall, 0.0);
}

// The second isotropic kernel: where the slope beats the talus, the sum of
// the pixel's drops to its neighbours (diagonals weighted 0.7071); the
// sediment it can give is the lesser of that and what it holds.
@compute @workgroup_size(16, 16)
fn drops_pass(@builtin(global_invocation_id) id: vec3<u32>) {
    let width = parameters.width;
    let height = parameters.height;
    if id.x >= width || id.y >= height {
        return;
    }
    let x = id.x;
    let y = id.y;
    let pixel = y * width + x;
    var total = 0.0;
    if slope[pixel] > parameters.threshold_raw {
        let centre = heights[pixel];
        if y >= 1u {
            if x >= 1u {
                total += drop_to(centre, x - 1u, y - 1u, DIAGONAL);
            }
            total += drop_to(centre, x, y - 1u, 1.0);
            if x + 1u < width {
                total += drop_to(centre, x + 1u, y - 1u, DIAGONAL);
            }
        }
        if x >= 1u {
            total += drop_to(centre, x - 1u, y, 1.0);
        }
        if x + 1u < width {
            total += drop_to(centre, x + 1u, y, 1.0);
        }
        if y + 1u < height {
            if x >= 1u {
                total += drop_to(centre, x - 1u, y + 1u, DIAGONAL);
            }
            total += drop_to(centre, x, y + 1u, 1.0);
            if x + 1u < width {
                total += drop_to(centre, x + 1u, y + 1u, DIAGONAL);
            }
        }
    }
    drops[pixel] = total;
    next_sediment[pixel] = -min(total, sediment[pixel]);
}

// The second anisotropic kernel: the drops to the neighbours from the pixel
// tilted by its flow, inside the border (summed in the CPU's vectorised
// order).
@compute @workgroup_size(16, 16)
fn tilt_drops_pass(@builtin(global_invocation_id) id: vec3<u32>) {
    let width = parameters.width;
    let height = parameters.height;
    if id.x >= width || id.y >= height {
        return;
    }
    let x = id.x;
    let y = id.y;
    let pixel = y * width + x;
    let interior = x != 0u && y != 0u && x + 1u < width && y + 1u < height;
    var total = 0.0;
    if interior {
        let h = heights[pixel];
        let tilted = tilt[pixel];
        let west = h - tilted.x;
        let top_left = max((west - (tilted.y + height_at(x - 1u, y - 1u))) * DIAGONAL, 0.0);
        let left = max(west - height_at(x - 1u, y), 0.0);
        let bottom_left = max(((west + tilted.y) - height_at(x - 1u, y + 1u)) * DIAGONAL, 0.0);
        let top = max(h - (tilted.y + height_at(x, y - 1u)), 0.0);
        let bottom = max((tilted.y + h) - height_at(x, y + 1u), 0.0);
        let east = tilted.x + h;
        let top_right = max((east - (tilted.y + height_at(x + 1u, y - 1u))) * DIAGONAL, 0.0);
        let right = max(east - height_at(x + 1u, y), 0.0);
        let bottom_right = max(((east + tilted.y) - height_at(x + 1u, y + 1u)) * DIAGONAL, 0.0);
        let western = (left + top_left) + bottom_left;
        let rest = ((top + bottom) + top_right) + right;
        total = (western + rest) + bottom_right;
    }
    drops[pixel] = total;
    next_sediment[pixel] = -min(total, sediment[pixel]);
}

// The pass's change: the pixel's outflow plus what it gains (less the
// share sediment removal takes), scaled by the pass's step.
fn settle(gain: f32, outflow: f32, mask: f32) -> f32 {
    let kept = fma(mask, parameters.sediment_removal, -1.0);
    let net = fma(-kept, gain, outflow);
    return parameters.step * net;
}

// The share of a neighbour's outflow that reaches the pixel.
fn share(rise: f32, neighbour_drops: f32, neighbour_sediment: f32) -> f32 {
    let lift = max(rise, 0.0);
    let moved = min(neighbour_drops, neighbour_sediment);
    return lift * moved / neighbour_drops;
}

// The neighbours in the order the gather visits them: column offset, row
// offset, and whether diagonal.
const NEIGHBOURS = array<vec3<i32>, 8>(
    vec3<i32>(-1, -1, 1),
    vec3<i32>(-1, 0, 0),
    vec3<i32>(-1, 1, 1),
    vec3<i32>(0, -1, 0),
    vec3<i32>(0, 1, 0),
    vec3<i32>(1, -1, 1),
    vec3<i32>(1, 0, 0),
    vec3<i32>(1, 1, 1),
);

// The third isotropic kernel: each pixel gains the share of its steeper
// neighbours' outflow that falls its way, and the heights and sediment
// take the pass's change.
@compute @workgroup_size(16, 16)
fn gather_pass(@builtin(global_invocation_id) id: vec3<u32>) {
    let width = parameters.width;
    let height = parameters.height;
    if id.x >= width || id.y >= height {
        return;
    }
    let x = id.x;
    let y = id.y;
    let pixel = y * width + x;
    let h = heights[pixel];
    var gain = 0.0;
    let interior = x != 0u && y != 0u && x + 1u < width && y + 1u < height;
    if interior {
        for (var index = 0; index < 8; index++) {
            let offset = NEIGHBOURS[index];
            let column = u32(i32(x) + offset.x);
            let row = u32(i32(y) + offset.y);
            let neighbour = row * width + column;
            let steep = !(parameters.threshold_raw >= slope[neighbour]);
            let neighbour_drops = drops[neighbour];
            if steep && !(0.0 >= neighbour_drops) {
                var rise = heights[neighbour] - h;
                if offset.z != 0 {
                    rise *= DIAGONAL;
                }
                gain += share(rise, neighbour_drops, sediment[neighbour]);
            }
        }
    }
    var mask = 1.0;
    if parameters.removal_used != 0u {
        mask = removal[pixel];
    }
    let change = settle(gain, next_sediment[pixel], mask);
    next_heights[pixel] = change + h;
    next_sediment[pixel] = change + sediment[pixel];
}

// A neighbour's share in the anisotropic gather.
fn tilted_share(neighbour: u32, rise: f32) -> f32 {
    let neighbour_drops = drops[neighbour];
    if 0.0 >= neighbour_drops {
        return 0.0;
    }
    return share(rise, neighbour_drops, sediment[neighbour]);
}

// The third anisotropic kernel: as `gather_pass`, the neighbours tilted by
// their flow and no talus test.
@compute @workgroup_size(16, 16)
fn tilt_gather_pass(@builtin(global_invocation_id) id: vec3<u32>) {
    let width = parameters.width;
    let height = parameters.height;
    if id.x >= width || id.y >= height {
        return;
    }
    let x = id.x;
    let y = id.y;
    let pixel = y * width + x;
    let h = heights[pixel];
    var gain = 0.0;
    let interior = x != 0u && y != 0u && x + 1u < width && y + 1u < height;
    if interior {
        var n = (y - 1u) * width + (x - 1u);
        gain += tilted_share(n, (((heights[n] - h) + tilt[n].y) + tilt[n].x) * DIAGONAL);
        n = y * width + (x - 1u);
        gain += tilted_share(n, (heights[n] - h) + tilt[n].x);
        n = (y + 1u) * width + (x - 1u);
        gain += tilted_share(n, ((heights[n] - (h + tilt[n].y)) + tilt[n].x) * DIAGONAL);
        n = (y - 1u) * width + x;
        gain += tilted_share(n, (heights[n] - h) + tilt[n].y);
        n = (y + 1u) * width + x;
        gain += tilted_share(n, heights[n] - (h + tilt[n].y));
        n = (y - 1u) * width + (x + 1u);
        gain += tilted_share(n, ((heights[n] + tilt[n].y) - (h + tilt[n].x)) * DIAGONAL);
        n = y * width + (x + 1u);
        gain += tilted_share(n, heights[n] - (h + tilt[n].x));
        n = (y + 1u) * width + (x + 1u);
        gain += tilted_share(n, (heights[n] - ((h + tilt[n].y) + tilt[n].x)) * DIAGONAL);
    }
    var mask = 1.0;
    if parameters.removal_used != 0u {
        mask = removal[pixel];
    }
    let change = settle(gain, next_sediment[pixel], mask);
    next_heights[pixel] = change + h;
    next_sediment[pixel] = change + sediment[pixel];
}

// The coarse neighbour of fine coordinate `x` nearer it than its own
// coarse cell's other neighbour, clamped to `last`.
fn coarse_side(x: u32, last: u32) -> u32 {
    let own = i32(x / 2u);
    let side = own + i32(x & 1u) * 2 - 1;
    return u32(clamp(side, 0, i32(last)));
}

// 9·own + diagonal + 3·(the two sides) of a coarse plane's 2× upsample at
// fine pixel (x, y), for each of the three coarse planes.
fn upsample_sums(x: u32, y: u32) -> vec3<f32> {
    let width = parameters.coarse_width;
    let own_x = x / 2u;
    let own_y = y / 2u;
    let side_x = coarse_side(x, width - 1u);
    let side_y = coarse_side(y, parameters.coarse_height - 1u);
    let own = own_y * width + own_x;
    let across = own_y * width + side_x;
    let down = side_y * width + own_x;
    let diagonal = side_y * width + side_x;
    let lifted = fma(coarse_heights[down] + coarse_heights[across], 3.0, fma(coarse_heights[own], 9.0, coarse_heights[diagonal]));
    let worn = fma(coarse_wear[down] + coarse_wear[across], 3.0, fma(coarse_wear[own], 9.0, coarse_wear[diagonal]));
    let covering = fma(coarse_sediment[down] + coarse_sediment[across], 3.0, fma(coarse_sediment[own], 9.0, coarse_sediment[diagonal]));
    return vec3<f32>(lifted, worn, covering);
}

// The finer level rebuilt from the simulated coarse one: its detail
// returns where the coarse sediment leaves it exposed (eased by the
// coarse wear), the sediment left over buries it, and with a removal mask
// some of that sediment is taken away.
@compute @workgroup_size(16, 16)
fn reconstruct(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.width || id.y >= parameters.height {
        return;
    }
    let pixel = id.y * parameters.width + id.x;
    let sums = upsample_sums(id.x, id.y);
    let lifted = sums.x * 0.0625;
    let wear_sum = sums.y;
    let kept_detail = detail[pixel];
    let covering = sums.z * 0.0625;
    let cell = parameters.coarse_cell;
    var raised: f32;
    var buried: f32;
    if !(covering > 0.0) {
        raised = kept_detail + lifted;
        buried = covering;
    } else if !(covering > kept_detail) {
        let exposed = kept_detail - covering;
        let limit = max(wear_sum * 0.125, cell);
        let ratio = min(cell / limit, 1.0);
        raised = fma(exposed, ratio, lifted);
        buried = 0.0;
    } else {
        raised = lifted;
        buried = covering - kept_detail;
    }

    var removal_change = 0.0;
    if parameters.removal_used != 0u {
        removal_change = removal_detail[pixel];
    }
    var exposure = max(removal_change, 0.0);
    exposure = min(exposure + exposure, 1.0);
    let removal_strength = exposure * parameters.sediment_removal;
    var removed = 0.0;
    if buried > 0.0 && 0.0 < removal_strength {
        removed = removal_strength * buried;
    }
    buried -= removed;
    raised -= removed;
    fine_sediment[pixel] = buried;
    fine_heights[pixel] = raised;
    fine_wear[pixel] = wear_sum * 0.0625;
}
