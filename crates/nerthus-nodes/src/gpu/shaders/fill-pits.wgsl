// The flow simulations' pit filling on the GPU (flow.rs fill_pyramid): one
// pass of raising pits to their second-lowest neighbour, and a finer
// level rebuilt from a filled coarse one.

struct Parameters {
    width: u32,
    height: u32,
    coarse_width: u32,
    coarse_height: u32,
}

@group(0) @binding(0) var<uniform> parameters: Parameters;
// A pass reads `heights` and writes `filled`, flagging the pixels it
// raised (flags only ever turn on).
@group(0) @binding(1) var<storage, read> heights: array<f32>;
@group(0) @binding(2) var<storage, read_write> filled: array<f32>;
@group(0) @binding(3) var<storage, read_write> flags: array<f32>;
// Reconstruction reads the filled coarse level and its flags with the
// finer level's detail, and writes the finer level.
@group(0) @binding(4) var<storage, read> coarse: array<f32>;
@group(0) @binding(5) var<storage, read> coarse_flags: array<f32>;
@group(0) @binding(6) var<storage, read> detail: array<f32>;
@group(0) @binding(7) var<storage, read_write> fine: array<f32>;

// The largest float, which "no neighbour yet" starts from.
const NONE: f32 = 3.40282347e38;
// Detail is only restored where less than this much filling reached.
const FILLED_THRESHOLD: f32 = 0.001;

// A pixel no higher than the second lowest of its neighbours is raised to
// it (and flagged).
@compute @workgroup_size(16, 16)
fn fill_pits(@builtin(global_invocation_id) id: vec3<u32>) {
    let width = parameters.width;
    let height = parameters.height;
    if id.x >= width || id.y >= height {
        return;
    }
    let x = id.x;
    let y = id.y;
    let first_y = max(y, 1u) - 1u;
    let last_y = min(y + 1u, height - 1u);
    let first_x = max(x, 1u) - 1u;
    let last_x = min(x + 1u, width - 1u);
    var lowest = NONE;
    var second = NONE;
    for (var row = first_y; row <= last_y; row++) {
        for (var column = first_x; column <= last_x; column++) {
            if row == y && column == x {
                continue;
            }
            let value = heights[row * width + column];
            if value < lowest {
                second = lowest;
                lowest = value;
            } else if value < second {
                second = value;
            }
        }
    }
    var floor_height = second;
    if second == NONE {
        floor_height = lowest;
    }
    let pixel = y * width + x;
    let own = heights[pixel];
    if floor_height < own {
        filled[pixel] = own;
    } else {
        filled[pixel] = floor_height;
        flags[pixel] = 1.0;
    }
}

// The coarse neighbour of fine coordinate `x` nearer it than its own
// coarse cell's other neighbour, clamped to `last`.
fn coarse_side(x: u32, last: u32) -> u32 {
    let own = i32(x / 2u);
    let side = own + i32(x & 1u) * 2 - 1;
    return u32(clamp(side, 0, i32(last)));
}

// The finer level rebuilt from the filled coarse one, keeping its detail
// only where no filling reached.
@compute @workgroup_size(16, 16)
fn reconstruct(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.width || id.y >= parameters.height {
        return;
    }
    let width = parameters.coarse_width;
    let own_x = id.x / 2u;
    let own_y = id.y / 2u;
    let side_x = coarse_side(id.x, width - 1u);
    let side_y = coarse_side(id.y, parameters.coarse_height - 1u);
    let own = own_y * width + own_x;
    let across = own_y * width + side_x;
    let down = side_y * width + own_x;
    let diagonal = side_y * width + side_x;
    let lifted = fma(coarse[down] + coarse[across], 3.0, fma(coarse[own], 9.0, coarse[diagonal]));
    let reached = fma(coarse_flags[down] + coarse_flags[across], 3.0, fma(coarse_flags[own], 9.0, coarse_flags[diagonal]));
    let pixel = id.y * parameters.width + id.x;
    var kept = 0.0;
    if reached * 0.0625 < FILLED_THRESHOLD {
        kept = detail[pixel];
    }
    fine[pixel] = fma(0.0625, lifted, kept);
}
