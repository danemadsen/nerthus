// The colour transport on the GPU (colour_transport.rs): a launch of
// flow particles that carry colour and leave it where they pass, and the
// kernel that blends the texture towards what each batch deposited. The
// colour sums are float atomics (compare-and-swap), so they add in
// whatever order the particles arrive, as on any GPU.
// terrain-sample.wgsl follows this module.

struct Parameters {
    width: u32,
    height: u32,
    // The launch: its first particle, how many it traces, how far, and
    // the steps between colour pickups.
    start: u32,
    count: u32,
    seed: u32,
    laminar: u32,
    steps: f32,
    hold: f32,
    use_lengths: u32,
    // The blend: the batch's weight, the base, the blend and the raise.
    weight: f32,
    base: f32,
    blend: f32,
    raise: f32,
    _padding_0: u32,
    _padding_1: u32,
    _padding_2: u32,
}

@group(0) @binding(0) var<uniform> parameters: Parameters;
@group(0) @binding(1) var<storage, read> heights: array<f32>;
// The texture's red, green and blue planes one after another.
@group(0) @binding(2) var<storage, read> colours: array<f32>;
@group(0) @binding(3) var<storage, read> lengths: array<f32>;
@group(0) @binding(4) var<storage, read> particles: array<vec2<f32>>;
@group(0) @binding(5) var<storage, read> directions: array<vec2<f32>, 32>;
// The colour deposited per pixel (float bits), planes as in the texture.
@group(0) @binding(6) var<storage, read_write> sums: array<atomic<u32>>;
@group(0) @binding(7) var<storage, read_write> counts: array<atomic<u32>>;
// The blend updates the texture and raises the terrain.
@group(0) @binding(8) var<storage, read_write> blended: array<f32>;
@group(0) @binding(9) var<storage, read_write> terrain: array<f32>;

// 2⁻³², turning a 32-bit random number into [0, 1].
const UNIT_SCALE: f32 = 2.32830644e-10;

fn plane_size() -> u32 {
    return parameters.width * parameters.height;
}

fn colour_at(pixel: u32) -> vec3<f32> {
    let size = plane_size();
    return vec3<f32>(colours[pixel], colours[size + pixel], colours[2u * size + pixel]);
}

fn add_float(index: u32, value: f32) {
    var old = atomicLoad(&sums[index]);
    loop {
        let updated = bitcast<u32>(bitcast<f32>(old) + value);
        let exchange = atomicCompareExchangeWeak(&sums[index], old, updated);
        if exchange.exchanged {
            break;
        }
        old = exchange.old_value;
    }
}

fn deposit(pixel: u32, colour: vec3<f32>) {
    let size = plane_size();
    add_float(pixel, colour.x);
    add_float(size + pixel, colour.y);
    add_float(2u * size + pixel, colour.z);
    atomicAdd(&counts[pixel], 1u);
}

fn pixel_at(x: f32, y: f32) -> u32 {
    return u32(y) * parameters.width + u32(x);
}

// One particle: it rolls like a flow particle and deposits its colour on
// every pixel it visits, picking up the colour under it every so often.
@compute @workgroup_size(256)
fn transport(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let offset = linear_index(id, groups);
    if offset >= parameters.count {
        return;
    }
    let index = offset + parameters.start;
    var position = particles[index];
    let right = f32(parameters.width) + -1.0;
    let bottom = f32(parameters.height) + -1.0;
    let inside = !(position.x < 0.0) && position.x < right && !(position.y < 0.0) && position.y < bottom;
    if !inside {
        return;
    }
    let start = sample_terrain(position.x, position.y);
    let start_length = fma(start.gradient.y, start.gradient.y, start.gradient.x * start.gradient.x);
    if !(start_length > 0.0) {
        return;
    }
    let start_scale = inverseSqrt(start_length);
    var direction = vec2<f32>(-start.gradient.x * start_scale, -start.gradient.y * start_scale);

    let start_pixel = pixel_at(position.x, position.y);
    let steps = parameters.steps;
    let uphill_limit = u32(max(i32(steps * 0.2), 4));
    let seeded = index + parameters.seed;
    var state = select(ZERO_SEED, seeded, seeded != 0u);
    var length = 1.0;
    if parameters.use_lengths != 0u {
        length = lengths[start_pixel];
    }
    state = xorshift(xorshift(state));
    let hold = parameters.hold;
    var pickup = (hold * UNIT_SCALE) * f32(random_bits(state));
    var colour = colour_at(start_pixel);
    let last_step = fma(steps, length, -1.0);

    var uphill = 0u;
    var steps_taken = 0.0;
    var previous_height = start.height;
    loop {
        let pixel = pixel_at(position.x, position.y);
        deposit(pixel, colour);
        if steps_taken >= pickup {
            colour = colour_at(pixel);
            state = xorshift(state);
            let fraction = f32(random_bits(state)) * UNIT_SCALE;
            pickup = fma(fraction, hold, pickup);
        }

        position.x += direction.x;
        if position.x < 0.0 || !(position.x < right) {
            return;
        }
        position.y += direction.y;
        if position.y < 0.0 || !(position.y < bottom) {
            return;
        }
        let here = sample_terrain(position.x, position.y);
        let length_squared = fma(here.gradient.y, here.gradient.y, here.gradient.x * here.gradient.x);
        state = xorshift(state);
        if length_squared > 0.0 {
            var nudge = vec2<f32>(0.0, 0.0);
            if parameters.laminar == 0u {
                let picked = directions[random_bits(state) & 31u];
                nudge = vec2<f32>(0.7 * picked.x, 0.7 * picked.y);
            }
            let inverse = 1.0 / sqrt(length_squared);
            let pushed_x = fma(direction.x, 0.9, -(here.gradient.x * inverse));
            let new_x = nudge.x + pushed_x;
            let pushed_y = fma(direction.y, 0.9, -(here.gradient.y * inverse));
            let new_y = pushed_y + nudge.y;
            let new_length = fma(new_y, new_y, new_x * new_x);
            let new_scale = inverseSqrt(new_length);
            direction = vec2<f32>(new_scale * new_x, new_scale * new_y);
        }

        steps_taken += 1.0;
        if !(here.height < previous_height) {
            uphill += 1u;
        }
        if steps_taken >= last_step {
            return;
        }
        previous_height = here.height;
        if uphill > uphill_limit {
            return;
        }
    }
}

// The texture blended towards each pixel's deposited colour, the terrain
// raised by the visits, and the sums and counts cleared for the next
// batch.
@compute @workgroup_size(16, 16)
fn blend(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.width || id.y >= parameters.height {
        return;
    }
    let size = plane_size();
    let pixel = id.y * parameters.width + id.x;
    let count = f32(atomicExchange(&counts[pixel], 0u));
    let weight = parameters.weight;
    let base = parameters.base;
    var deposited_sums: array<f32, 3>;
    for (var channel = 0u; channel < 3u; channel++) {
        deposited_sums[channel] = bitcast<f32>(atomicExchange(&sums[channel * size + pixel], 0u));
    }
    if !(0.0 >= count) {
        let damping = fma(count, weight, base);
        let inverse = 1.0 / damping;
        for (var channel = 0u; channel < 3u; channel++) {
            let index = channel * size + pixel;
            let current = blended[index];
            let deposited = fma(weight, deposited_sums[channel], current * base);
            let difference = fma(-deposited, inverse, current);
            blended[index] = fma(-difference, parameters.blend, current);
        }
    }
    terrain[pixel] = fma(count, parameters.raise, terrain[pixel]);
}
