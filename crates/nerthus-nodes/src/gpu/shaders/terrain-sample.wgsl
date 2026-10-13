// The terrain under a flow particle (flow.rs sample): bilinear height and
// gradient around (x − ½, y − ½), `trunc` picking the pixels and `floor`
// the weights. Modules that include this declare the
// `heights` binding and `parameters.width` / `parameters.height`.

struct Sample {
    gradient: vec2<f32>,
    height: f32,
}

fn terrain_at(column: i32, row: i32) -> f32 {
    return heights[u32(row) * parameters.width + u32(column)];
}

fn sample_terrain(x: f32, y: f32) -> Sample {
    let sample_y = y + -0.5;
    let sample_x = x + -0.5;
    let column = i32(sample_x);
    let row = i32(sample_y);
    let row_0 = max(row, 0);
    let column_0 = max(column, 0);
    let row_1 = min(row + 1, i32(parameters.height) - 1);
    let column_1 = min(column + 1, i32(parameters.width) - 1);
    let h00 = terrain_at(column_0, row_0);
    let h01 = terrain_at(column_1, row_0);
    let h10 = terrain_at(column_0, row_1);
    let h11 = terrain_at(column_1, row_1);

    let fraction_x = sample_x - floor(sample_x);
    let fraction_y = sample_y - floor(sample_y);
    let rest_x = 1.0 - fraction_x;
    let rest_y = 1.0 - fraction_y;
    var result: Sample;
    let gradient_y = fma(h11 - h01, fraction_x, rest_x * (h10 - h00));
    let gradient_x = fma(h11 - h10, fraction_y, rest_y * (h01 - h00));
    result.gradient = vec2<f32>(gradient_x, gradient_y);
    let top_mix = fma(fraction_x, h01, rest_x * h00);
    let bottom_mix = fma(h11, fraction_x, rest_x * h10);
    result.height = fma(bottom_mix, fraction_y, rest_y * top_mix);
    return result;
}

// The generators of the particles.
const ZERO_SEED: u32 = 0xA2351342u;
const XORSHIFT_MULTIPLIER: u32 = 0x2545F381u;

fn xorshift(state: u32) -> u32 {
    var next = state ^ (state << 13u);
    next ^= next >> 17u;
    return next ^ (next << 5u);
}

// The 32-bit random number a generator state gives.
fn random_bits(state: u32) -> u32 {
    return state * XORSHIFT_MULTIPLIER - 1u;
}
