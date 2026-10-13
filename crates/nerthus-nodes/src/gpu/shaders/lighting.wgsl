// The terrain lighting on the GPU (lighting.rs and shade.rs, the SYCL
// SunLightIntegral and Shade): the half-size pyramid steps, Scharr
// normals, lines of sight swept across the wrapping map (lighting cells in
// 16.16 fixed point, or counting the cells a shadow ray reaches), and the
// per-pixel steps of the renderer: direct light, local and hybrid
// occlusion, and the lit colour.

struct Parameters {
    // The lit colour: the sun's and the sky's light per channel.
    sun_light: vec4<f32>,
    ambient_light: vec4<f32>,
    width: u32,
    height: u32,
    coarse_width: u32,
    coarse_height: u32,
    // Sweeps: how many, and the scale light or counts are turned into
    // values with.
    sweep_count: u32,
    factor: f32,
    // Normals: the cell's horizontal size (× 32) and the height scales.
    cell: f32,
    x_scale: f32,
    y_scale: f32,
    // Direct light: the sun's direction.
    sun_x: f32,
    sun_y: f32,
    sun_z: f32,
    // Local occlusion.
    slope_scale: f32,
    curvature_scale: f32,
    // Hybrid occlusion's relief correction.
    relief_scale: f32,
    relief_curvature_scale: f32,
    octave_weight: f32,
    // The lit colour: the albedo without a texture, and which terms count.
    default_colour: f32,
    direct_lighting: u32,
    shadow_map: u32,
    occlusion_map: u32,
    unshadowed: f32,
    unoccluded: f32,
    use_texture: u32,
    _padding_0: u32,
    _padding_1: u32,
    _padding_2: u32,
    _padding_3: u32,
}

// One sweep of lines: the first line's 16.16 start, which axis the lines
// advance along (0: y, 1: x), their 16.16 steps, the height a line drops
// per step, the light scale, and the direction towards the light.
struct Sweep {
    start: vec2<i32>,
    advance_axis: u32,
    _padding: u32,
    step: vec2<i32>,
    drop: f32,
    scale: f32,
    direction: vec4<f32>,
}

@group(0) @binding(0) var<uniform> parameters: Parameters;
@group(0) @binding(1) var<storage, read> heights: array<f32>;
// Three planes: x, y and z.
@group(0) @binding(2) var<storage, read_write> normals: array<f32>;
@group(0) @binding(3) var<storage, read> sweeps: array<Sweep>;
@group(0) @binding(4) var<storage, read_write> light: array<atomic<i32>>;
@group(0) @binding(5) var<storage, read> coarse: array<f32>;
@group(0) @binding(6) var<storage, read_write> values: array<f32>;
@group(0) @binding(7) var<storage, read> diffuse: array<f32>;
@group(0) @binding(8) var<storage, read> shadow: array<f32>;
@group(0) @binding(9) var<storage, read> occlusion: array<f32>;
// Three planes: red, green and blue.
@group(0) @binding(10) var<storage, read> albedo: array<f32>;
@group(0) @binding(11) var<storage, read_write> colour: array<f32>;
@group(0) @binding(12) var<storage, read> fine: array<f32>;
@group(0) @binding(13) var<storage, read> smoothed: array<f32>;

const INVERSE_FIXED_SCALE: f32 = 1.52587891e-5;

fn plane_size() -> u32 {
    return parameters.width * parameters.height;
}

fn fine_at(row: i32, column: i32) -> f32 {
    return fine[u32(row) * parameters.width + u32(column)];
}

// The [1 3 3 1]² downsampling to half resolution, summed in the kernel's order.
@compute @workgroup_size(16, 16)
fn downsample(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.coarse_width || id.y >= parameters.coarse_height {
        return;
    }
    let last_column = i32(parameters.width) - 1;
    let last_row = i32(parameters.height) - 1;
    let y = i32(id.y) * 2;
    let x = i32(id.x) * 2;
    let rows = array<i32, 4>(max(y - 1, 0), y, min(y + 1, last_row), min(y + 2, last_row));
    let columns = array<i32, 4>(max(x - 1, 0), x, min(x + 1, last_column), min(x + 2, last_column));
    var sum = fine_at(rows[0], columns[1]) * 3.0 + fine_at(rows[0], columns[0]);
    sum += fine_at(rows[0], columns[2]) * 3.0;
    sum += fine_at(rows[0], columns[3]);
    sum += fine_at(rows[1], columns[0]) * 3.0;
    sum += fine_at(rows[1], columns[1]) * 9.0;
    sum += fine_at(rows[1], columns[2]) * 9.0;
    sum += fine_at(rows[1], columns[3]) * 3.0;
    sum += fine_at(rows[2], columns[0]) * 3.0;
    sum += fine_at(rows[2], columns[1]) * 9.0;
    sum += fine_at(rows[2], columns[2]) * 9.0;
    sum += fine_at(rows[2], columns[3]) * 3.0;
    sum += fine_at(rows[3], columns[0]);
    sum += fine_at(rows[3], columns[1]) * 3.0;
    sum += fine_at(rows[3], columns[2]) * 3.0;
    sum += fine_at(rows[3], columns[3]);
    values[id.y * parameters.coarse_width + id.x] = sum * 0.015625;
}

fn height_at(row: u32, column: u32) -> f32 {
    return heights[row * parameters.width + column];
}

// Unit surface normals from a Scharr (3, 10, 3) gradient.
@compute @workgroup_size(16, 16)
fn surface_normals(@builtin(global_invocation_id) id: vec3<u32>) {
    let width = parameters.width;
    let height = parameters.height;
    if id.x >= width || id.y >= height {
        return;
    }
    let row = id.y;
    let column = id.x;
    let up = max(row, 1u) - 1u;
    let down = min(row + 1u, height - 1u);
    let left = max(column, 1u) - 1u;
    let right = min(column + 1u, width - 1u);
    let up_left = height_at(up, left);
    let up_right = height_at(up, right);
    let middle_left = height_at(row, left);
    let middle_right = height_at(row, right);
    let down_left = height_at(down, left);
    let down_right = height_at(down, right);
    var gradient_x = fma(middle_right, 10.0, up_right * 3.0) + down_right * 3.0 - up_left * 3.0;
    gradient_x = fma(-10.0, middle_left, gradient_x) - down_left * 3.0;
    var gradient_y = fma(height_at(up, column), 10.0, up_left * 3.0) + up_right * 3.0 - down_left * 3.0;
    gradient_y = fma(-10.0, height_at(down, column), gradient_y) - down_right * 3.0;
    let nx = gradient_x * parameters.x_scale;
    let ny = gradient_y * parameters.y_scale;
    let cell = parameters.cell;
    let length_squared = fma(ny, ny, nx * nx) + cell * cell;
    var inverse = 0.0;
    if length_squared != 0.0 {
        inverse = 1.0 / sqrt(length_squared);
    }
    let pixel = row * width + column;
    let size = plane_size();
    normals[pixel] = nx * inverse;
    normals[size + pixel] = ny * inverse;
    normals[2u * size + pixel] = cell * inverse;
}

// Bilinear height at (x, y) in cell units, the cell indices (not the
// position) clamped to the map.
fn sample_bilinear(x: f32, y: f32) -> f32 {
    let last = i32(parameters.width) - 1;
    let column = i32(x);
    let row = i32(y);
    let x0 = clamp(column, 0, last);
    let x1 = clamp(column + 1, 0, last);
    let y0 = clamp(row, 0, last);
    let y1 = clamp(row + 1, 0, last);
    let fx = x - f32(x0);
    let fy = y - f32(y0);
    let top = fma(1.0 - fx, height_at(u32(y0), u32(x0)), fx * height_at(u32(y0), u32(x1)));
    let bottom = fma(1.0 - fx, height_at(u32(y1), u32(x0)), fx * height_at(u32(y1), u32(x1)));
    return fma(1.0 - fy, top, fy * bottom);
}

// The same with the top row's products fused the other way round, as the
// soft-shadow rays sample.
fn shadow_height(x: f32, y: f32) -> f32 {
    let last = i32(parameters.width) - 1;
    let column = i32(x);
    let row = i32(y);
    let x0 = clamp(column, 0, last);
    let x1 = clamp(column + 1, 0, last);
    let y0 = clamp(row, 0, last);
    let y1 = clamp(row + 1, 0, last);
    let fx = x - f32(x0);
    let fy = y - f32(y0);
    let top = fma(height_at(u32(y0), u32(x1)), fx, height_at(u32(y0), u32(x0)) * (1.0 - fx));
    let bottom = fma(1.0 - fx, height_at(u32(y1), u32(x0)), fx * height_at(u32(y1), u32(x1)));
    return fma(1.0 - fy, top, fy * bottom);
}

// The height just behind a line position (where a ray restarts).
fn height_behind(x: i32, y: i32, sweep: Sweep) -> f32 {
    let behind_x = f32(x - sweep.step.x) * INVERSE_FIXED_SCALE + -0.5;
    let behind_y = f32(y - sweep.step.y) * INVERSE_FIXED_SCALE + -0.5;
    return sample_bilinear(behind_x, behind_y);
}

struct LineStart {
    sweep: Sweep,
    position: vec2<i32>,
}

fn line_start(thread: u32) -> LineStart {
    let width = parameters.width;
    let sweep = sweeps[thread / width];
    let along = i32(thread % width) << 16u;
    var position = sweep.start;
    if sweep.advance_axis == 0u {
        position.y += along;
    } else {
        position.x += along;
    }
    var result: LineStart;
    result.sweep = sweep;
    result.position = position;
    return result;
}

// One line of sight towards a light: cells that rise above it are lit (by
// how squarely they face the light) and raise it.
@compute @workgroup_size(256)
fn sweep_light(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let thread = linear_index(id, groups);
    let width = parameters.width;
    if thread >= parameters.sweep_count * width {
        return;
    }
    let start = line_start(thread);
    let sweep = start.sweep;
    var x = start.position.x;
    var y = start.position.y;
    let wrap = i32(width) << 16u;
    var ray = height_behind(x, y, sweep);
    let last = i32(width) - 1;
    let size = plane_size();
    for (var step = 0u; step < width; step++) {
        var wrapped = false;
        if y < 0 {
            y += wrap;
            wrapped = true;
        } else if y >= wrap {
            y -= wrap;
            wrapped = true;
        }
        if x < 0 {
            x += wrap;
            wrapped = true;
        } else if x >= wrap {
            x -= wrap;
            wrapped = true;
        }
        ray += sweep.drop;
        let column = x >> 16u;
        let row = y >> 16u;
        if wrapped {
            let restart = clamp(row, 0, last) * i32(width) + clamp(column, 0, last);
            ray = heights[u32(restart)] + sweep.drop;
        }
        let index = u32(row * i32(width) + column);
        let here = heights[index];
        if ray < here {
            let facing = fma(normals[2u * size + index], sweep.direction.z, fma(normals[index], sweep.direction.x, normals[size + index] * sweep.direction.y));
            atomicAdd(&light[index], i32(facing * sweep.scale));
            ray = max(here, ray);
        }
        x += sweep.step.x;
        y += sweep.step.y;
    }
}

// One soft-shadow line of sight: cells whose interpolated height rises
// above the line count as lit and raise it; after wrapping across an edge
// the line restarts from the height just behind it.
@compute @workgroup_size(256)
fn sweep_shadow(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let thread = linear_index(id, groups);
    let width = parameters.width;
    if thread >= parameters.sweep_count * width {
        return;
    }
    let start = line_start(thread);
    let sweep = start.sweep;
    var x = start.position.x;
    var y = start.position.y;
    let wrap = i32(width) << 16u;
    var ray = height_behind(x, y, sweep);
    for (var step = 0u; step < width; step++) {
        var wrapped = false;
        if y < 0 {
            y += wrap;
            wrapped = true;
        }
        if y >= wrap {
            y -= wrap;
            wrapped = true;
        }
        if x < 0 {
            x += wrap;
            wrapped = true;
        }
        if x >= wrap {
            x -= wrap;
            wrapped = true;
        }
        ray += sweep.drop;
        if wrapped {
            ray = height_behind(x, y, sweep) + sweep.drop;
        }
        let position_x = f32(x) * INVERSE_FIXED_SCALE + -0.5;
        let position_y = f32(y) * INVERSE_FIXED_SCALE + -0.5;
        let here = shadow_height(position_x, position_y);
        if ray < here {
            let index = u32((y >> 16u) * i32(width) + (x >> 16u));
            atomicAdd(&light[index], 1);
            ray = here;
        }
        x += sweep.step.x;
        y += sweep.step.y;
    }
}

// Summed light (or counts) as values: clamped at zero and scaled.
@compute @workgroup_size(256)
fn to_values(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let index = linear_index(id, groups);
    if index >= plane_size() {
        return;
    }
    let summed = atomicLoad(&light[index]);
    values[index] = f32(max(summed, 0)) * parameters.factor;
}

fn coarse_at(column: i32, row: i32) -> f32 {
    return coarse[u32(row) * parameters.coarse_width + u32(column)];
}

// The 9/3/3/1 upsampling of a half-size plane.
@compute @workgroup_size(16, 16)
fn upsample(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.width || id.y >= parameters.height {
        return;
    }
    let x = i32(id.x);
    let y = i32(id.y);
    let center_x = x >> 1u;
    let center_y = y >> 1u;
    let neighbour_x = clamp(center_x + (x & 1) * 2 - 1, 0, i32(parameters.coarse_width) - 1);
    let neighbour_y = clamp(center_y + (y & 1) * 2 - 1, 0, i32(parameters.coarse_height) - 1);
    var sum = coarse_at(neighbour_x, center_y) * 3.0 + coarse_at(center_x, center_y) * 9.0;
    sum += coarse_at(center_x, neighbour_y) * 3.0;
    sum += coarse_at(neighbour_x, neighbour_y);
    values[id.y * parameters.width + id.x] = sum * 0.0625;
}

// The sun's cosine on each cell, never negative.
@compute @workgroup_size(256)
fn direct_light(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let index = linear_index(id, groups);
    let size = plane_size();
    if index >= size {
        return;
    }
    var facing = fma(parameters.sun_x, normals[index], parameters.sun_y * normals[size + index]);
    facing = fma(parameters.sun_z, normals[2u * size + index], facing);
    values[index] = max(facing, 0.0);
}

// `atan` as ISPC's standard library evaluates it: a polynomial on [0, 1],
// reflected through 1/x above it.
fn ispc_atan(x: f32) -> f32 {
    let magnitude = abs(x);
    let above_one = 1.0 < magnitude;
    var reduced = magnitude;
    if above_one {
        reduced = 1.0 / magnitude;
    }
    let squared = reduced * reduced;
    var polynomial = -0.0046721357;
    polynomial = fma(polynomial, squared, 0.024172833);
    polynomial = fma(polynomial, squared, -0.0593665);
    polynomial = fma(polynomial, squared, 0.099050835);
    polynomial = fma(polynomial, squared, -0.14015688);
    polynomial = fma(polynomial, squared, 0.19968921);
    polynomial = fma(polynomial, squared, -0.33331916);
    polynomial = fma(polynomial, squared, 0.9999999);
    var result = reduced * polynomial;
    if above_one {
        result = 1.5707964 - result;
    }
    if x < 0.0 {
        result = -result;
    }
    return result;
}

// Local occlusion: darker on steep slopes and in hollows, from each cell's
// 3×3 neighbourhood alone.
@compute @workgroup_size(16, 16)
fn local_occlusion(@builtin(global_invocation_id) id: vec3<u32>) {
    let width = parameters.width;
    let height = parameters.height;
    if id.x >= width || id.y >= height {
        return;
    }
    let x = id.x;
    let y = id.y;
    let up = max(y, 1u) - 1u;
    let down = min(y + 1u, height - 1u);
    let left = max(x, 1u) - 1u;
    let right = min(x + 1u, width - 1u);
    let up_left = height_at(up, left);
    let up_middle = height_at(up, x);
    let up_right = height_at(up, right);
    let middle_left = height_at(y, left);
    let middle_right = height_at(y, right);
    let down_left = height_at(down, left);
    let down_middle = height_at(down, x);
    let down_right = height_at(down, right);

    let side_x = (up_right + down_right - up_left - down_left) * 3.0;
    let gradient_x = fma(middle_right - middle_left, 10.0, side_x);
    let side_y = (up_left + up_right - down_left - down_right) * 3.0;
    let gradient_y = fma(up_middle - down_middle, 10.0, side_y);
    let slope = sqrt(fma(gradient_y, gradient_y, gradient_x * gradient_x)) * parameters.slope_scale;

    let cross = up_middle + middle_left + down_middle + middle_right;
    let centre = fma(cross, -0.14644662, height_at(y, x));
    let diagonal = up_left + up_right + down_left + down_right;
    let laplacian = fma(diagonal, -0.10355339, centre) * parameters.curvature_scale;

    var sign = -1.0;
    if 0.0 < laplacian {
        sign = 1.0;
    }
    let denominator = fma(laplacian, sign, 1.0);
    let estimate = 1.0 / denominator;
    let correction = fma(-denominator, estimate, 2.0);
    let squashed = fma(-correction, estimate, 1.0);
    let curvature = sign * squashed;

    let tilt = fma(ispc_atan(slope) * -57.29578, 0.011111111, 1.0);
    let shade = fma(curvature, 0.25, 0.75) * tilt;
    values[y * width + x] = shade * (2.0 - shade);
}

// Hybrid occlusion's correction at full resolution: brighter where a cell
// stands above the smoothed terrain or on convex ground, darker below it,
// in hollows or on steep slopes. Corrects `values` in place.
@compute @workgroup_size(16, 16)
fn relief(@builtin(global_invocation_id) id: vec3<u32>) {
    let width = parameters.width;
    let height = parameters.height;
    if id.x >= width || id.y >= height {
        return;
    }
    let x = id.x;
    let y = id.y;
    let up = max(y, 1u) - 1u;
    let down = min(y + 1u, height - 1u);
    let left = max(x, 1u) - 1u;
    let right = min(x + 1u, width - 1u);
    let index = y * width + x;
    let weight = parameters.octave_weight;

    let raised = (heights[index] - smoothed[index]) * parameters.relief_scale;
    var relief_change = raised;
    if raised > 0.0 {
        relief_change = raised * 1.7;
    }
    let cross = height_at(y, left) + height_at(up, x) + height_at(down, x) + height_at(y, right);
    let diagonal = height_at(up, left) + height_at(up, right) + height_at(down, left) + height_at(down, right);
    let laplacian = ((height_at(y, x) - cross * 0.16666667) - diagonal * 0.083333336) * parameters.relief_curvature_scale;
    let curvature = min(0.5, max(-0.5, laplacian)) * 0.7;
    var curvature_weight = 1.5;
    if curvature > 0.0 {
        curvature_weight = 0.5;
    }
    let shaped = curvature * weight * curvature_weight + relief_change;

    let tilt = (normals[2u * plane_size() + index] * 0.7 - 0.7) * weight;
    let darkening = exp(tilt + min(shaped, 0.0));
    var brightening = 1.0;
    if shaped > 0.0 {
        brightening = shaped + 1.0;
    }
    let corrected = darkening * brightening * values[index];
    values[index] = min(corrected, 1.0);
}

// The lit colour: albedo × (sun light × the lit share of the direct light
// + sky light × the open share of the sky).
@compute @workgroup_size(256)
fn combine(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let index = linear_index(id, groups);
    let size = plane_size();
    if index >= size {
        return;
    }
    var lit = parameters.unshadowed;
    if parameters.shadow_map != 0u {
        lit = shadow[index];
    }
    var open = parameters.unoccluded;
    if parameters.occlusion_map != 0u {
        open = occlusion[index];
    }
    var facing = 1.0;
    if parameters.direct_lighting != 0u {
        facing = diffuse[index];
    }
    let direct = lit * facing;
    for (var channel = 0u; channel < 3u; channel++) {
        let light_amount = fma(direct, parameters.sun_light[channel], open * parameters.ambient_light[channel]);
        var base = parameters.default_colour;
        if parameters.use_texture != 0u {
            base = albedo[channel * size + index];
        }
        colour[channel * size + index] = base * light_amount;
    }
}
