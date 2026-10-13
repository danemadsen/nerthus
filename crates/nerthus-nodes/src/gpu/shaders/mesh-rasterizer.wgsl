// The mesh rasteriser on the GPU (mesh_rasterizer.rs): each triangle's
// candidate pixels tested against the triangle grown by 0.5%, the highest
// surface kept per pixel (meshes), or the last triangle per pixel (point
// clouds); the point clouds' triangle colours; and the antialiasing 2×2
// mean.
//
// Small triangles get a thread each; the candidates of large ones are
// spread over threads, each finding its triangle by binary search over the
// large triangles' running counts.

struct Parameters {
    width: i32,
    height: i32,
    triangle_count: u32,
    // 0: a mesh (`render`), 1: an untextured point cloud, 2: a textured
    // point cloud (no candidate limit), 3: a displaced mesh (`render3`).
    mode: u32,
    // Meshes with more than three candidates in all test pixels as the
    // inlined code does.
    inlined: u32,
    // The large triangles of this launch: where they start in `large`, how
    // many, and their candidates in all.
    large_first: u32,
    large_count: u32,
    candidate_total: u32,
    // The texture's planes (point clouds).
    channel_count: u32,
    _padding_0: u32,
    _padding_1: u32,
    _padding_2: u32,
}

@group(0) @binding(0) var<uniform> parameters: Parameters;
// Three floats per vertex, three indices per triangle.
@group(0) @binding(1) var<storage, read> vertices: array<f32>;
@group(0) @binding(2) var<storage, read> triangles: array<u32>;
// The highest surface's bits per pixel (meshes), or one more than the last
// triangle over it (point clouds).
@group(0) @binding(3) var<storage, read_write> raster: array<atomic<u32>>;
// The large triangles and each one's first candidate (running counts that
// start from zero at every launch's first).
@group(0) @binding(4) var<storage, read> large: array<u32>;
@group(0) @binding(5) var<storage, read> large_starts: array<u32>;
// Point clouds: the texture's planes, each triangle's colour, and the maps
// the resolve writes.
@group(0) @binding(6) var<storage, read> texture_planes: array<f32>;
@group(0) @binding(7) var<storage, read_write> colours: array<vec4<f32>>;
@group(0) @binding(8) var<storage, read_write> heights: array<f32>;
@group(0) @binding(9) var<storage, read_write> hashes: array<f32>;
@group(0) @binding(10) var<storage, read_write> painted: array<f32>;
// The antialiasing mean reads the raster at twice the size.
@group(0) @binding(11) var<storage, read> fine: array<f32>;
@group(0) @binding(12) var<storage, read_write> coarse: array<f32>;

const ONE_THIRD: f32 = 0.333333343;
// 1.005: the triangle grows by 0.5% about its centroid.
const GROWTH: f32 = 1.005;
const MISS: f32 = -3.40282347e38;
// Triangles whose bounding box reaches this many pixels are skipped.
const TRIANGLE_PIXEL_LIMIT: u32 = 0x10000000u;
// Triangles with more candidates go to the large launch.
const SMALL_LIMIT: u32 = 64u;

fn vertex(index: u32) -> vec3<f32> {
    let base = index * 3u;
    return vec3<f32>(vertices[base], vertices[base + 1u], vertices[base + 2u]);
}

struct Corners {
    a: vec3<f32>,
    b: vec3<f32>,
    c: vec3<f32>,
}

fn corners(triangle: u32) -> Corners {
    let base = triangle * 3u;
    var result: Corners;
    result.a = vertex(triangles[base]);
    result.b = vertex(triangles[base + 1u]);
    result.c = vertex(triangles[base + 2u]);
    return result;
}

// The triangle grown by 0.5% about its centroid: its centre, its first
// corner's offset from the centre, and its two edges from that corner.
struct Grown {
    centre: vec2<f32>,
    corner: vec2<f32>,
    first_edge: vec2<f32>,
    second_edge: vec2<f32>,
}

fn grow(t: Corners) -> Grown {
    var grown: Grown;
    let centre_x = ((t.b.x + t.a.x) + t.c.x) * ONE_THIRD;
    let centre_y = ((t.b.y + t.a.y) + t.c.y) * ONE_THIRD;
    let corner_x = GROWTH * (t.a.x - centre_x);
    let corner_y = GROWTH * (t.a.y - centre_y);
    grown.centre = vec2<f32>(centre_x, centre_y);
    grown.corner = vec2<f32>(corner_x, corner_y);
    grown.first_edge = vec2<f32>(fma(GROWTH, t.b.x - centre_x, -corner_x), fma(GROWTH, t.b.y - centre_y, -corner_y));
    grown.second_edge = vec2<f32>(fma(GROWTH, t.c.x - centre_x, -corner_x), fma(GROWTH, t.c.y - centre_y, -corner_y));
    return grown;
}

// The height at a point given its dot products with the two edges, or MISS
// outside when `tested` (degenerate triangles miss everywhere).
fn surface_height(grown: Grown, first_dot: f32, second_dot: f32, t: Corners, tested: bool) -> f32 {
    let e1 = grown.first_edge;
    let e2 = grown.second_edge;
    let second_second = fma(e2.y, e2.y, e2.x * e2.x);
    let first_second = fma(e1.y, e2.y, e2.x * e1.x);
    let first_first = fma(e1.y, e1.y, e1.x * e1.x);
    let denominator = fma(second_second, first_first, -(first_second * first_second));
    if denominator == 0.0 {
        return MISS;
    }
    let v_numerator = fma(first_dot, second_second, -(second_dot * first_second));
    let inverse = 1.0 / denominator;
    let v = v_numerator * inverse;
    let w_numerator = fma(second_dot, first_first, -(first_dot * first_second));
    let w = w_numerator * inverse;
    let u = 1.0 - (w + v);
    let inside = u >= 0.0 && v >= 0.0 && w >= 0.0;
    if tested && !inside {
        return MISS;
    }
    return fma(u, t.a.z, fma(w, t.c.z, v * t.b.z));
}

// The out-of-line test: the point is the pixel centre.
fn sample_outlined(grown: Grown, x: i32, y: i32, t: Corners, tested: bool) -> f32 {
    let point_x = f32(x) + 0.5;
    let point_y = f32(y) + 0.5;
    let dx = point_x - (grown.centre.x + grown.corner.x);
    let dy = point_y - (grown.centre.y + grown.corner.y);
    let second_dot = fma(dy, grown.second_edge.y, dx * grown.second_edge.x);
    let first_dot = fma(grown.first_edge.y, dy, dx * grown.first_edge.x);
    return surface_height(grown, first_dot, second_dot, t, tested);
}

// The test as inlined in the chunked region: the half-pixel offset
// reassociated, the dot products fusing the other product.
fn sample_inlined(grown: Grown, x: i32, y: i32, t: Corners) -> f32 {
    let dx = f32(x) + (0.5 - (grown.centre.x + grown.corner.x));
    let dy = (0.5 - (grown.centre.y + grown.corner.y)) + f32(y);
    let second_dot = fma(dx, grown.second_edge.x, dy * grown.second_edge.y);
    let first_dot = fma(grown.first_edge.x, dx, dy * grown.first_edge.y);
    return surface_height(grown, first_dot, second_dot, t, true);
}

// The pixels a triangle's candidates walk: numbered row by row from
// `origin` across rows `box_width` wide, `count` of them.
struct Candidates {
    origin: vec2<i32>,
    box_width: u32,
    count: u32,
}

fn low_x(t: Corners) -> f32 {
    return min(min(t.a.x, t.b.x), t.c.x);
}

fn candidates(t: Corners) -> Candidates {
    var result: Candidates;
    result.count = 0u;
    result.box_width = 0u;
    result.origin = vec2<i32>(0, 0);
    if parameters.mode == 3u {
        // render3: triangles facing the right way, their bounding box
        // rounded outwards and clamped to the target.
        let first = (t.b.x - t.a.x) * (t.c.y - t.a.y);
        let second = (t.c.x - t.a.x) * (t.b.y - t.a.y);
        if first > second {
            return result;
        }
        let low_y = i32(floor(min(t.a.y, min(t.c.y, t.b.y))));
        let high_y = i32(ceil(max(t.a.y, max(t.c.y, t.b.y))));
        let high_x = i32(ceil(max(t.a.x, max(t.b.x, t.c.x))));
        let row_start = max(low_y, 0);
        let row_end = min(high_y, parameters.height - 1);
        let column_end = min(high_x, parameters.width - 1);
        if row_end < row_start {
            return result;
        }
        let column_start = max(i32(floor(min(t.a.x, min(t.b.x, t.c.x)))), 0);
        if column_end < column_start {
            return result;
        }
        result.origin = vec2<i32>(column_start, row_start);
        result.box_width = u32(column_end - column_start + 1);
        result.count = result.box_width * u32(row_end - row_start + 1);
        return result;
    }
    // render: the count rounds the box outwards, the walk truncates it.
    let low_y_round = i32(floor(min(t.a.y, min(t.b.y, t.c.y))));
    let high_y_round = i32(ceil(max(t.a.y, max(t.b.y, t.c.y))));
    let low_x_round = i32(floor(min(t.a.x, min(t.b.x, t.c.x))));
    let high_x_round = i32(ceil(max(t.a.x, max(t.b.x, t.c.x))));
    let rows = high_y_round - low_y_round + 1;
    let columns = high_x_round - low_x_round + 1;
    let product = rows * columns;
    var count = u32(select(product, -product, product < 0));
    if parameters.mode != 2u && count >= TRIANGLE_PIXEL_LIMIT {
        count = 0u;
    }
    let walk_x = i32(low_x(t));
    let walk_high_x = i32(max(max(t.a.x, t.b.x), t.c.x));
    let walk_y = i32(min(min(t.a.y, t.b.y), t.c.y));
    result.origin = vec2<i32>(walk_x, walk_y);
    result.box_width = u32(walk_high_x - walk_x + 1);
    if result.box_width == 0u {
        return result;
    }
    result.count = count;
    return result;
}

// Hashes a triangle index into [0, 1) (the point clouds' second map).
fn triangle_hash(index: u32) -> f32 {
    var state = select(0xA2351342u, index, index != 0u);
    state ^= state << 13u;
    state ^= state >> 17u;
    state ^= state << 5u;
    let mixed = state * 0x2545F381u - 1u;
    return f32(mixed) * 2.32830644e-10;
}

// Tests one candidate of a triangle and keeps what it finds.
fn test_candidate(triangle: u32, t: Corners, grown: Grown, walk: Candidates, local: u32) {
    let x = i32(local % walk.box_width) + walk.origin.x;
    if x < 0 {
        return;
    }
    let y = i32(local / walk.box_width) + walk.origin.y;
    if y < 0 || x >= parameters.width || y >= parameters.height {
        return;
    }
    var surface: f32;
    if parameters.mode == 3u || (parameters.mode == 0u && parameters.inlined != 0u) {
        surface = sample_inlined(grown, x, y, t);
    } else {
        surface = sample_outlined(grown, x, y, t, true);
    }
    if !(surface > MISS) {
        return;
    }
    let pixel = u32(y * parameters.width + x);
    if parameters.mode == 1u || parameters.mode == 2u {
        // Later triangles overwrite earlier ones.
        atomicMax(&raster[pixel], triangle + 1u);
    } else if surface > 0.0 {
        // Positive floats order as their bits do.
        atomicMax(&raster[pixel], bitcast<u32>(surface));
    }
}

@compute @workgroup_size(256)
fn rasterize_small(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let triangle = linear_index(id, groups);
    if triangle >= parameters.triangle_count {
        return;
    }
    let t = corners(triangle);
    let walk = candidates(t);
    if walk.count == 0u || walk.count > SMALL_LIMIT {
        return;
    }
    let grown = grow(t);
    for (var local = 0u; local < walk.count; local++) {
        test_candidate(triangle, t, grown, walk, local);
    }
}

@compute @workgroup_size(256)
fn rasterize_large(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let candidate = linear_index(id, groups);
    if candidate >= parameters.candidate_total {
        return;
    }
    // The last large triangle starting at or before the candidate.
    var low = parameters.large_first;
    var high = parameters.large_first + parameters.large_count - 1u;
    while low < high {
        let middle = (low + high + 1u) / 2u;
        if large_starts[middle] <= candidate {
            low = middle;
        } else {
            high = middle - 1u;
        }
    }
    let triangle = large[low];
    let t = corners(triangle);
    let walk = candidates(t);
    let local = candidate - large_starts[low];
    if local >= walk.count {
        return;
    }
    test_candidate(triangle, t, grow(t), walk, local);
}

// The colour of each texture plane at a pixel (a grey texture's one plane
// serving all three).
fn texture_colour(pixel: u32) -> vec3<f32> {
    let size = u32(parameters.width * parameters.height);
    let last = parameters.channel_count - 1u;
    let red = texture_planes[min(0u, last) * size + pixel];
    let green = texture_planes[min(1u, last) * size + pixel];
    let blue = texture_planes[min(2u, last) * size + pixel];
    return vec3<f32>(red, green, blue);
}

fn texture_at(point: vec2<f32>) -> vec3<f32> {
    let x = clamp(i32(point.x), 0, parameters.width - 1);
    let y = clamp(i32(point.y), 0, parameters.height - 1);
    return texture_colour(u32(y * parameters.width + x));
}

fn inverse_distance(colour: vec3<f32>, other: vec3<f32>) -> f32 {
    let red = colour.x - other.x;
    let green = colour.y - other.y;
    let blue = colour.z - other.z;
    return 1.0 / fma(blue, blue, fma(green, green, fma(red, red, 0.001)));
}

// A textured point cloud's triangle colour: of the texture colours at
// thirteen points in the triangle, the one closest to the others.
@compute @workgroup_size(256)
fn triangle_colours(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let triangle = linear_index(id, groups);
    if triangle >= parameters.triangle_count {
        return;
    }
    let t = corners(triangle);
    let third = 0.333000004;
    let a = t.a.xy;
    let b = t.b.xy;
    let c = t.c.xy;
    let a2 = a * 0.2;
    let b2 = b * 0.2;
    let c2 = c * 0.2;
    let a4 = a * 0.4;
    let b4 = b * 0.4;
    let c4 = c * 0.4;
    let a15 = a * 0.15;
    let b15 = b * 0.15;
    let c15 = c * 0.15;
    let aq = a * 0.25;
    let bq = b * 0.25;
    let cq = c * 0.25;
    var points: array<vec2<f32>, 13>;
    points[0] = vec2<f32>(((a.x + b.x) + c.x) * third, ((b.y + a.y) + c.y) * third);
    points[1] = vec2<f32>(fma(a.x, 0.6, b2.x) + c2.x, fma(a.y, 0.6, b2.y) + c2.y);
    points[2] = vec2<f32>(fma(b.x, 0.6, a2.x) + c2.x, fma(b.y, 0.6, a2.y) + c2.y);
    points[3] = vec2<f32>(fma(c.x, 0.6, a2.x + b2.x), fma(c.y, 0.6, b2.y + a2.y));
    points[4] = vec2<f32>((b4.x + a4.x) + c2.x, (b4.y + a4.y) + c2.y);
    points[5] = vec2<f32>((a4.x + b2.x) + c4.x, c4.y + (a4.y + b2.y));
    points[6] = vec2<f32>((b4.x + a2.x) + c4.x, c4.y + (b4.y + a2.y));
    points[7] = vec2<f32>(c15.x + fma(a.x, 0.7, b15.x), fma(a.y, 0.7, b15.y) + c15.y);
    points[8] = vec2<f32>(c15.x + fma(b.x, 0.7, a15.x), fma(b.y, 0.7, a15.y) + c15.y);
    points[9] = vec2<f32>(fma(c.x, 0.7, b15.x + a15.x), fma(c.y, 0.7, b15.y + a15.y));
    points[10] = vec2<f32>(fma(a.x, 0.5, bq.x) + cq.x, fma(a.y, 0.5, bq.y) + cq.y);
    points[11] = vec2<f32>(fma(b.x, 0.5, aq.x) + cq.x, fma(b.y, 0.5, aq.y) + cq.y);
    points[12] = vec2<f32>(fma(c.x, 0.5, bq.x + aq.x), fma(c.y, 0.5, bq.y + aq.y));

    var samples: array<vec3<f32>, 13>;
    for (var index = 0; index < 13; index++) {
        samples[index] = texture_at(points[index]);
    }
    var best = 0.0;
    var chosen = vec3<f32>(0.0, 0.0, 0.0);
    for (var candidate = 0; candidate < 13; candidate++) {
        let colour = samples[candidate];
        var terms: array<f32, 13>;
        for (var other = 0; other < 13; other++) {
            terms[other] = inverse_distance(colour, samples[other]);
        }
        let first = (terms[2] + terms[3]) + (terms[0] + terms[1]);
        let second = first + (terms[6] + (terms[5] + terms[4]));
        let third_sum = second + (terms[10] + (terms[9] + (terms[8] + terms[7])));
        let total = (terms[11] + terms[12]) + third_sum;
        if total > best {
            best = total;
            chosen = colour;
        }
    }
    colours[triangle] = vec4<f32>(chosen, 0.0);
}

// A point cloud's pixels from the last triangle over each: its surface,
// its hash, and (textured) its colour painted into the texture.
@compute @workgroup_size(16, 16)
fn resolve_cloud(@builtin(global_invocation_id) id: vec3<u32>) {
    if i32(id.x) >= parameters.width || i32(id.y) >= parameters.height {
        return;
    }
    let pixel = id.y * u32(parameters.width) + id.x;
    let winner = atomicLoad(&raster[pixel]);
    if winner == 0u {
        return;
    }
    let triangle = winner - 1u;
    let t = corners(triangle);
    // The triangle passed the test here when rasterised; the fast-math
    // compiler may round this copy of it differently, so it is not
    // repeated.
    heights[pixel] = sample_outlined(grow(t), i32(id.x), i32(id.y), t, false);
    hashes[pixel] = triangle_hash(triangle);
    if parameters.mode == 2u {
        let size = u32(parameters.width * parameters.height);
        let colour = colours[triangle];
        let last = parameters.channel_count - 1u;
        // Writes go to the clamped planes in order, so a grey texture
        // keeps the blue.
        painted[min(0u, last) * size + pixel] = colour.x;
        painted[min(1u, last) * size + pixel] = colour.y;
        painted[min(2u, last) * size + pixel] = colour.z;
    }
}

// `resampling::downsample_box`: the 2×2 mean, adding in the order the CPU
// code adds each column (rows run in chunks of 1024 columns whose vector
// part adds the row pairs first).
@compute @workgroup_size(16, 16)
fn downsample_box(@builtin(global_invocation_id) id: vec3<u32>) {
    let width = u32(parameters.width);
    if id.x >= width || i32(id.y) >= parameters.height {
        return;
    }
    let fine_width = width * 2u;
    let top = 2u * id.y * fine_width;
    let bottom = top + fine_width;
    let t0 = fine[top + 2u * id.x];
    let t1 = fine[top + 2u * id.x + 1u];
    let b0 = fine[bottom + 2u * id.x];
    let b1 = fine[bottom + 2u * id.x + 1u];
    let start = id.x / 1024u * 1024u;
    let end = min(start + 1024u, width);
    let length = end - start;
    var vector_end = start;
    if width > 3u && length >= 9u {
        var tail = length % 8u;
        if tail == 0u {
            tail = 8u;
        }
        vector_end = end - tail;
    }
    var sum: f32;
    if id.x < vector_end {
        sum = (t0 + t1) + (b0 + b1);
    } else {
        sum = ((t1 + t0) + b0) + b1;
    }
    coarse[id.y * width + id.x] = sum * 0.25;
}
