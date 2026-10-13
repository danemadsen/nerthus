// The flow simulation on the GPU (flow.rs): a launch of particles
// rolling downhill and counting the pixels they pass, and the kernel that
// folds a batch's counts into the flow map while raising the terrain.
// terrain-sample.wgsl follows this module.
//
// The original kernel's first uphill test compares the start height with a
// stack slot it never sets, which holds what the previous particle on the
// thread left there. Like the CPU code, a launch first probes how each
// particle ends for either outcome of that test, chains the outcomes in
// particle order on one thread, then traces for real.

struct Parameters {
    width: u32,
    height: u32,
    // The launch: its first particle, how many it traces and how far.
    start: u32,
    count: u32,
    seed: u32,
    steps: u32,
    uphill_limit: f32,
    last_step: f32,
    // The batch's weight, and how far each visit raises the terrain.
    weight: f32,
    raise: f32,
    _padding_0: u32,
    _padding_1: u32,
}

@group(0) @binding(0) var<uniform> parameters: Parameters;
@group(0) @binding(1) var<storage, read> heights: array<f32>;
@group(0) @binding(2) var<storage, read> particles: array<vec2<f32>>;
@group(0) @binding(3) var<storage, read> directions: array<vec2<f32>, 32>;
@group(0) @binding(4) var<storage, read_write> counts: array<atomic<u32>>;
@group(0) @binding(5) var<storage, read_write> terrain: array<f32>;
@group(0) @binding(6) var<storage, read_write> flow: array<f32>;
// Per particle of the launch: the start height, the slot's value after a
// descending and after an ascending first test, and which of those three
// are set (bits 0, 1 and 2, as a float).
@group(0) @binding(7) var<storage, read_write> probes: array<vec4<f32>>;
// Per particle of the launch: whether its first test counts as uphill.
@group(0) @binding(8) var<storage, read_write> first_uphill: array<u32>;

// How a trace ended: the height its first test compares (if it got past
// its start) and the slot's last value (if a test wrote it).
struct TraceEnd {
    first_height: f32,
    started: bool,
    slot: f32,
    slot_written: bool,
}

// One particle of `Flow::flow`: it follows the slope with a little inertia
// and jitter, counting each pixel it passes (when `counting`), until it
// leaves the map, runs out of steps or climbs too often.
fn run_trace(offset: u32, ascending_first: bool, counting: bool) -> TraceEnd {
    var end: TraceEnd;
    end.first_height = 0.0;
    end.started = false;
    end.slot = 0.0;
    end.slot_written = false;

    let index = offset + parameters.start;
    let seeded = index + parameters.seed;
    var state = select(ZERO_SEED, seeded, seeded != 0u);
    var position = particles[index];
    let right = f32(parameters.width) + -1.0;
    let bottom = f32(parameters.height) + -1.0;
    let inside = position.x >= 0.0 && position.x < right && position.y >= 0.0 && position.y < bottom;
    if !inside {
        return end;
    }
    let start = sample_terrain(position.x, position.y);
    let start_length = fma(start.gradient.y, start.gradient.y, start.gradient.x * start.gradient.x);
    if !(start_length > 0.0) {
        return end;
    }
    end.first_height = start.height;
    end.started = true;

    state = xorshift(state);
    let start_scale = inverseSqrt(start_length);
    var direction = vec2<f32>(-start.gradient.x * start_scale, -start.gradient.y * start_scale);
    var steps_taken = 0.0;
    var uphill = 0.0;
    var first_test = true;
    var current_height = start.height;
    var previous_height = 0.0;
    loop {
        if counting {
            let column = u32(position.x);
            let row = u32(position.y);
            atomicAdd(&counts[row * parameters.width + column], 1u);
        }

        position.x += direction.x;
        if position.x < 0.0 || position.x >= right {
            break;
        }
        position.y += direction.y;
        if position.y < 0.0 || position.y >= bottom {
            break;
        }
        let here = sample_terrain(position.x, position.y);
        let length = fma(here.gradient.y, here.gradient.y, here.gradient.x * here.gradient.x);
        state = xorshift(state);
        if length > 0.0 {
            let scale = inverseSqrt(length);
            let nudge = directions[random_bits(state) & 31u];
            var new_x = fma(-1.2, direction.x, here.gradient.x * scale);
            new_x = fma(0.7, nudge.x, -new_x);
            var new_y = fma(-1.2, direction.y, here.gradient.y * scale);
            new_y = fma(0.7, nudge.y, -new_y);
            let new_length = fma(new_y, new_y, new_x * new_x);
            let new_scale = inverseSqrt(new_length);
            direction = vec2<f32>(new_x * new_scale, new_y * new_scale);
        }

        steps_taken += 1.0;
        if steps_taken == parameters.last_step {
            break;
        }
        var descended = current_height < previous_height;
        if first_test {
            descended = !ascending_first;
        }
        first_test = false;
        if !descended {
            uphill += 1.0;
        }
        previous_height = current_height;
        current_height = here.height;
        end.slot = previous_height;
        end.slot_written = true;
        if !(uphill <= parameters.uphill_limit) {
            break;
        }
    }
    return end;
}

// How each particle of the launch ends for either outcome of its first
// test.
@compute @workgroup_size(256)
fn probe(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let offset = linear_index(id, groups);
    if offset >= parameters.count {
        return;
    }
    let descending = run_trace(offset, false, false);
    let ascending = run_trace(offset, true, false);
    var flags = 0u;
    if descending.started {
        flags |= 1u;
    }
    if descending.slot_written {
        flags |= 2u;
    }
    if ascending.slot_written {
        flags |= 4u;
    }
    probes[offset] = vec4<f32>(descending.first_height, descending.slot, ascending.slot, f32(flags));
}

// The particles' first tests in launch order, each comparing with the slot
// the previous particle left (the launch starts from above every height).
@compute @workgroup_size(1)
fn chain() {
    var slot = 3.40282347e38;
    for (var offset = 0u; offset < parameters.count; offset++) {
        let probed = probes[offset];
        let flags = u32(probed.w);
        if (flags & 1u) == 0u {
            first_uphill[offset] = 0u;
            continue;
        }
        let uphill = !(probed.x < slot);
        first_uphill[offset] = select(0u, 1u, uphill);
        if uphill && (flags & 4u) != 0u {
            slot = probed.z;
        } else if !uphill && (flags & 2u) != 0u {
            slot = probed.y;
        }
    }
}

// The launch's particles traced for real, counting their visits.
@compute @workgroup_size(256)
fn trace(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let offset = linear_index(id, groups);
    if offset >= parameters.count {
        return;
    }
    let ascending_first = first_uphill[offset] != 0u;
    _ = run_trace(offset, ascending_first, true);
}

// A batch's visit counts folded into the flow map, raising the terrain.
@compute @workgroup_size(16, 16)
fn accumulate(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.width || id.y >= parameters.height {
        return;
    }
    let pixel = id.y * parameters.width + id.x;
    let count = f32(atomicExchange(&counts[pixel], 0u));
    let weight = parameters.weight;
    let amount = count * (weight * 0.03125);
    let damping = fma(count, weight, 8.0);
    flow[pixel] += amount / damping;
    terrain[pixel] = fma(count, parameters.raise, terrain[pixel]);
}
