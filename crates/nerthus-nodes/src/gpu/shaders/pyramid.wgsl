// The Laplacian pyramid steps the SYCL functions share (flow.rs
// downsample, downsample_usm and detail): a level at half resolution, and
// the detail a level loses to the next coarser one. The sums go in the
// order of the CPU code's scalar path.

struct Parameters {
    fine_width: u32,
    fine_height: u32,
    coarse_width: u32,
    coarse_height: u32,
}

@group(0) @binding(0) var<uniform> parameters: Parameters;
@group(0) @binding(1) var<storage, read> fine: array<f32>;
@group(0) @binding(2) var<storage, read_write> coarse: array<f32>;
@group(0) @binding(3) var<storage, read_write> detail: array<f32>;

fn fine_at(column: u32, row: u32) -> f32 {
    return fine[row * parameters.fine_width + column];
}

// The coarse level: a separable [1 3 3 1]/8 filter over 4×4 fine pixels.
@compute @workgroup_size(16, 16)
fn downsample(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.coarse_width || id.y >= parameters.coarse_height {
        return;
    }
    let last_x = i32(parameters.fine_width) - 1;
    let last_y = i32(parameters.fine_height) - 1;
    let y = i32(id.y);
    let x = i32(id.x);
    let row_a = u32(max(2 * y, 1) - 1);
    let row_b = u32(2 * y);
    let row_c = u32(min(2 * y + 1, last_y));
    let row_d = u32(min(2 * y + 2, last_y));
    let a = u32(max(2 * x, 1) - 1);
    let b = u32(2 * x);
    let c = u32(min(2 * x + 1, last_x));
    let d = u32(min(2 * x + 2, last_x));

    var edges = fine_at(c, row_a) + fine_at(b, row_a);
    edges += fine_at(a, row_b);
    edges += fine_at(d, row_b);
    edges += fine_at(a, row_c);
    edges += fine_at(d, row_c);
    edges += fine_at(b, row_d);
    edges += fine_at(c, row_d);
    let centre = ((fine_at(c, row_b) + fine_at(b, row_b)) + fine_at(b, row_c)) + fine_at(c, row_c);
    let corners = (fine_at(d, row_a) + fine_at(a, row_a)) + fine_at(a, row_d);
    let weighted = fma(centre, 9.0, corners) + fine_at(d, row_d);
    let total = fma(edges, 3.0, weighted);
    coarse[id.y * parameters.coarse_width + id.x] = total * 0.015625;
}

// The coarse neighbour of fine coordinate `x` nearer it than its own
// coarse cell's other neighbour, clamped to `last`.
fn coarse_side(x: u32, last: u32) -> u32 {
    let own = i32(x / 2u);
    let side = own + i32(x & 1u) * 2 - 1;
    return u32(clamp(side, 0, i32(last)));
}

// The detail the coarse level loses: fine − upsample(coarse).
@compute @workgroup_size(16, 16)
fn detail_of(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.fine_width || id.y >= parameters.fine_height {
        return;
    }
    let width = parameters.coarse_width;
    let own_x = id.x / 2u;
    let own_y = id.y / 2u;
    let side_x = coarse_side(id.x, width - 1u);
    let side_y = coarse_side(id.y, parameters.coarse_height - 1u);
    let sides = coarse[side_y * width + own_x] + coarse[own_y * width + side_x];
    let centre = fma(9.0, coarse[own_y * width + own_x], coarse[side_y * width + side_x]);
    let difference = fma(sides, -3.0, -centre);
    let pixel = id.y * parameters.fine_width + id.x;
    detail[pixel] = fma(difference, 0.0625, fine[pixel]);
}
