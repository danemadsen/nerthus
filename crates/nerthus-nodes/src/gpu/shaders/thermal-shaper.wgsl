// The thermal shaper on the GPU (thermal_shaper.rs): a shaping pass over
// one level, the next finer level rebuilt from a shaped one, and the
// result blended with the input.

struct Parameters {
    width: u32,
    height: u32,
    coarse_width: u32,
    coarse_height: u32,
    // A pass: pixels across per unit of height, how strongly curvature
    // counts, the pass's share of the shape, and height per pixel across.
    scale: f32,
    growth: f32,
    shape: f32,
    cell: f32,
    microdetail: f32,
    influence: f32,
    use_intensity: u32,
    negated: u32,
}

@group(0) @binding(0) var<uniform> parameters: Parameters;
// A pass reads `level` and writes `shaped`.
@group(0) @binding(1) var<storage, read> level: array<f32>;
@group(0) @binding(2) var<storage, read> intensity: array<f32>;
@group(0) @binding(3) var<storage, read_write> shaped: array<f32>;
// Reconstruction reads the shaped `coarse` level and the finer one's
// `detail`, and writes the `fine` level.
@group(0) @binding(4) var<storage, read> coarse: array<f32>;
@group(0) @binding(5) var<storage, read> detail: array<f32>;
@group(0) @binding(6) var<storage, read_write> fine: array<f32>;
// The blend mixes the `original` heights with the shaped finest `level`.
@group(0) @binding(7) var<storage, read> original: array<f32>;
@group(0) @binding(8) var<storage, read_write> blended: array<f32>;

// 0.7071, not 1/√2, as the native code has it.
const DIAGONAL: f32 = 0.7071;
// The curvature's weights for the sides and the corners.
const SIDE_WEIGHT: f32 = 0.146446615;
const CORNER_WEIGHT: f32 = 0.103553392;

fn level_at(column: u32, row: u32) -> f32 {
    return level[row * parameters.width + column];
}

// Each pixel moves by its softened curvature against its slope, no further
// than its steepest drop.
@compute @workgroup_size(16, 16)
fn shape_pass(@builtin(global_invocation_id) id: vec3<u32>) {
    let width = parameters.width;
    let height = parameters.height;
    if id.x >= width || id.y >= height {
        return;
    }
    let x = id.x;
    let y = id.y;
    let left = max(x, 1u) - 1u;
    let right = min(x + 1u, width - 1u);
    let up = max(y, 1u) - 1u;
    let down = min(y + 1u, height - 1u);
    let tl = level_at(left, up);
    let t = level_at(x, up);
    let tr = level_at(right, up);
    let l = level_at(left, y);
    let c = level_at(x, y);
    let r = level_at(right, y);
    let bl = level_at(left, down);
    let b = level_at(x, down);
    let br = level_at(right, down);

    let scale = parameters.scale;
    let diagonal = br + (tr - (tl + bl));
    let across = (r - l) * 10.0;
    let sum_x = fma(3.0, diagonal, across);
    let top_corners = tl + tr;
    let bottom_corners = br + bl;
    let vertical = top_corners - bottom_corners;
    let fall = (t - b) * 10.0;
    let sum_y = fma(3.0, vertical, fall);
    let gradient_x = (scale * 0.03125) * sum_x;
    let gradient_y = (scale * -0.03125) * sum_y;
    let length_squared = fma(gradient_y, gradient_y, gradient_x * gradient_x);
    let slope = sqrt(length_squared);
    var softness = 0.0;
    if length_squared > 0.0 {
        let eased = pow(slope * 0.1, DIAGONAL) + 1.0;
        softness = 1.0 - 1.0 / eased;
    }

    let corners = bottom_corners + top_corners;
    let sides = (t + l) + (b + r);
    var curvature = fma(-sides, SIDE_WEIGHT, c);
    curvature = fma(corners, CORNER_WEIGHT, -curvature);
    let weight = parameters.growth * scale;
    let excess = min(fma(curvature, weight, -slope), 0.0);
    var change = softness * excess;

    let corner = max(max(c - tl, c - tr), max(c - bl, c - br));
    let straight = max(max(c - t, c - l), max(c - r, c - b));
    let steepest = max(corner * DIAGONAL, straight);
    let limit = steepest * -scale;

    change *= parameters.shape;
    let pixel = y * width + x;
    if parameters.use_intensity != 0u {
        change *= intensity[pixel];
    }
    shaped[pixel] = fma(max(change, limit), parameters.cell, c);
}

// The coarse neighbour of fine coordinate `x` nearer it than its own
// coarse cell's other neighbour, clamped to `last`.
fn coarse_side(x: u32, last: u32) -> u32 {
    let own = i32(x / 2u);
    let side = own + i32(x & 1u) * 2 - 1;
    return u32(clamp(side, 0, i32(last)));
}

// 9·own + diagonal + 3·(the two sides) of the bilinear 2× upsample.
fn upsample_sum(x: u32, y: u32) -> f32 {
    let width = parameters.coarse_width;
    let own_x = x / 2u;
    let own_y = y / 2u;
    let side_x = coarse_side(x, width - 1u);
    let side_y = coarse_side(y, parameters.coarse_height - 1u);
    let sides = coarse[side_y * width + own_x] + coarse[own_y * width + side_x];
    let centre = fma(coarse[own_y * width + own_x], 9.0, coarse[side_y * width + side_x]);
    return fma(sides, 3.0, centre);
}

// The finer level from the coarse one and its own detail, kept by the
// microdetail preservation.
@compute @workgroup_size(16, 16)
fn reconstruct(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.width || id.y >= parameters.height {
        return;
    }
    let pixel = id.y * parameters.width + id.x;
    let kept = parameters.microdetail * detail[pixel];
    fine[pixel] = fma(upsample_sum(id.x, id.y), 0.0625, kept);
}

// The shaped heights mixed into the input by the influence (undoing the
// negation the shaper may have worked under).
@compute @workgroup_size(16, 16)
fn blend(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.width || id.y >= parameters.height {
        return;
    }
    let pixel = id.y * parameters.width + id.x;
    let before = original[pixel];
    let result = level[pixel];
    if parameters.negated != 0u {
        blended[pixel] = fma(-(before + result), parameters.influence, before);
    } else {
        blended[pixel] = fma(result - before, parameters.influence, before);
    }
}
