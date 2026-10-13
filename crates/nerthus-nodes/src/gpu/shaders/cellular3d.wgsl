// Cellular3D's renderer on the GPU (cellular3d.rs): the grid's cells, each
// column's highest occupied layer, those tops spread over each column's
// 3×3 neighbourhood, and the surface and hash of each pixel.

struct Parameters {
    columns: u32,
    rows: u32,
    layers: u32,
    // The highest layer the per-pixel kernel starts from (`zMax`).
    top_layer: i32,
    width: u32,
    height: u32,
    seed: u32,
    use_gap_map: u32,
    cells_per_pixel_x: f32,
    cells_per_pixel_y: f32,
    pixels_per_cell_x: f32,
    pixels_per_cell_y: f32,
    height_per_cell: f32,
    gap: f32,
    jitter_x: f32,
    jitter_y: f32,
    jitter_z: f32,
    _padding_0: u32,
    _padding_1: u32,
    _padding_2: u32,
}

@group(0) @binding(0) var<uniform> parameters: Parameters;
@group(0) @binding(1) var<storage, read> heights: array<f32>;
@group(0) @binding(2) var<storage, read> gap_map: array<f32>;
// Each cell's jittered point (in cells) and its weight: 1 − gap where the
// point lies under the surface, else zero.
@group(0) @binding(3) var<storage, read_write> cells: array<vec4<f32>>;
// Each cell's random number in [0, 1].
@group(0) @binding(4) var<storage, read_write> hashes: array<f32>;
@group(0) @binding(5) var<storage, read_write> tops: array<i32>;
@group(0) @binding(6) var<storage, read_write> dilated: array<i32>;
@group(0) @binding(7) var<storage, read_write> surface: array<f32>;
@group(0) @binding(8) var<storage, read_write> pixel_hashes: array<f32>;

// The grid reaches this many cells past the terrain on every side.
const BORDER: i32 = 2;
// Grid columns with no cell under the surface report this top layer.
const EMPTY_COLUMN: i32 = -3;
const FNV_OFFSET: u32 = 0x811C9DC5u;
const FNV_PRIME: u32 = 0x01000193u;
// A cell whose hash is zero seeds its generator with this instead.
const ZERO_HASH_SEED: u32 = 0xA2351342u;
const XORSHIFT_MULTIPLIER: u32 = 0x2545F381u;
// 2⁻³², turning a 32-bit random number into [0, 1].
const UNIT_SCALE: f32 = 2.32830644e-10;
const LARGEST: f32 = 3.40282347e38;

fn xorshift(state: u32) -> u32 {
    var next = state ^ (state << 13u);
    next ^= next >> 17u;
    return next ^ (next << 5u);
}

fn random_bits(state: u32) -> f32 {
    return f32(state * XORSHIFT_MULTIPLIER - 1u);
}

// The index of the cell at grid coordinates (x, y, z), which start at
// −BORDER.
fn cell_index(x: i32, y: i32, z: i32) -> u32 {
    let layer = u32(z + BORDER);
    let row = u32(y + BORDER);
    let column = u32(x + BORDER);
    return (layer * parameters.rows + row) * parameters.columns + column;
}

// The first kernel: a cell's point, jittered from the cell's centre by a
// generator seeded from its coordinates.
@compute @workgroup_size(256)
fn make_cells(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let index = linear_index(id, groups);
    if index >= parameters.columns * parameters.rows * parameters.layers {
        return;
    }
    let columns = i32(parameters.columns);
    let rows = i32(parameters.rows);
    let linear = i32(index);
    let x = linear % columns - BORDER;
    let y = (linear / columns) % rows - BORDER;
    let z = linear / (columns * rows) - BORDER;

    var hash = (bitcast<u32>(x) ^ FNV_OFFSET) * FNV_PRIME;
    hash = (hash ^ bitcast<u32>(y)) * FNV_PRIME;
    hash = (hash ^ bitcast<u32>(z)) * FNV_PRIME;
    hash = hash + parameters.seed;
    var state = select(ZERO_HASH_SEED, hash, hash != 0u);

    let jitter = vec3<f32>(parameters.jitter_x, parameters.jitter_y, parameters.jitter_z);
    let coordinates = vec3<f32>(f32(x), f32(y), f32(z));
    var point: vec3<f32>;
    for (var axis = 0; axis < 3; axis++) {
        state = xorshift(state);
        let offset = fma(random_bits(state), UNIT_SCALE, -0.5);
        let centre = coordinates[axis] + 0.5;
        point[axis] = fma(offset, jitter[axis], centre);
    }

    // The pixel under the point, clamped to the map.
    let pixel_x = clamp(i32(point.x * parameters.pixels_per_cell_x), 0, i32(parameters.width) - 1);
    let pixel_y = clamp(i32(point.y * parameters.pixels_per_cell_y), 0, i32(parameters.height) - 1);
    let pixel = u32(pixel_y) * parameters.width + u32(pixel_x);
    let under = heights[pixel];
    var gap = parameters.gap;
    if parameters.use_gap_map != 0u {
        gap *= gap_map[pixel];
    }
    let point_height = point.z * parameters.height_per_cell;

    state = xorshift(state);
    var weight = 0.0;
    if point_height < under {
        weight = 1.0 - gap;
    }
    cells[index] = vec4<f32>(point, weight);
    hashes[index] = random_bits(state) * UNIT_SCALE;
}

// The second kernel: each grid column's highest layer holding a cell under
// the surface.
@compute @workgroup_size(256)
fn find_tops(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let column = linear_index(id, groups);
    let column_count = parameters.columns * parameters.rows;
    if column >= column_count {
        return;
    }
    var top = EMPTY_COLUMN;
    for (var layer = i32(parameters.layers) - 1; layer >= 0; layer--) {
        let weight = cells[u32(layer) * column_count + column].w;
        if weight != 0.0 {
            top = layer - BORDER;
            break;
        }
    }
    tops[column] = top;
}

// The third kernel: the highest top of each column's 3×3 neighbourhood.
@compute @workgroup_size(16, 16)
fn dilate_tops(@builtin(global_invocation_id) id: vec3<u32>) {
    let columns = i32(parameters.columns);
    let rows = i32(parameters.rows);
    let column = i32(id.x);
    let row = i32(id.y);
    if column >= columns || row >= rows {
        return;
    }
    var highest = -2147483647 - 1;
    for (var neighbour_row = row - 1; neighbour_row <= row + 1; neighbour_row++) {
        if neighbour_row < 0 || neighbour_row >= rows {
            continue;
        }
        for (var neighbour_column = column - 1; neighbour_column <= column + 1; neighbour_column++) {
            if neighbour_column < 0 || neighbour_column >= columns {
                continue;
            }
            highest = max(highest, tops[u32(neighbour_row * columns + neighbour_column)]);
        }
    }
    dilated[u32(row * columns + column)] = highest;
}

// The top of the cell at (cell_x, cell_y, cell_z) over the pixel: the
// lowest ceiling above it, or nothing once a floor rises over a ceiling.
// The faces are the planes halfway between the cell's point and its
// neighbours', pushed back by the gap; only face and edge neighbours
// count, the y offset measured from the pixel's cell. A face whose
// normal is horizontal gives the CPU code infinities (or NaN when the
// pixel lies on it); those outcomes are spelled out here.
struct Top {
    found: bool,
    value: f32,
}

fn cell_top(cell_x: i32, cell_y: i32, cell_z: i32, position: vec2<f32>, pixel_cell_y: i32) -> Top {
    let cell = cells[cell_index(cell_x, cell_y, cell_z)];
    let gap = 1.0 - cell.w;
    var floor_height = 0.0;
    // The CPU's floor turns NaN (and is then replaced by the next floor)
    // when a horizontal face passes through the pixel.
    var floor_unset = false;
    var ceiling = LARGEST;
    var result: Top;
    result.found = false;
    result.value = 0.0;
    for (var neighbour_z = cell_z - 1; neighbour_z <= cell_z + 1; neighbour_z++) {
        for (var neighbour_y = cell_y - 1; neighbour_y <= cell_y + 1; neighbour_y++) {
            for (var neighbour_x = cell_x - 1; neighbour_x <= cell_x + 1; neighbour_x++) {
                if neighbour_x == cell_x && neighbour_y == cell_y && neighbour_z == cell_z {
                    continue;
                }
                let offset_x = neighbour_x - cell_x;
                let offset_y = neighbour_y - pixel_cell_y;
                let offset_z = neighbour_z - cell_z;
                if offset_x * offset_x + offset_y * offset_y + offset_z * offset_z > 3 {
                    continue;
                }

                let neighbour = cells[cell_index(neighbour_x, neighbour_y, neighbour_z)];
                let delta = neighbour.xyz - cell.xyz;
                let length_squared = fma(delta.z, delta.z, fma(delta.y, delta.y, delta.x * delta.x));
                let normal = delta * inverseSqrt(length_squared);
                var across_x = fma(cell.x + neighbour.x, -0.5, position.x);
                across_x = fma(normal.x, gap, across_x);
                across_x *= normal.x;
                var across = fma(cell.y + neighbour.y, -0.5, position.y);
                across = fma(normal.y, gap, across);
                across = fma(across, normal.y, across_x);

                if normal.z == 0.0 {
                    // across / 0: a floor at −∞ changes nothing, one at +∞
                    // rises over any ceiling, and NaN unsets the floor.
                    if across < 0.0 {
                        return result;
                    }
                    if across == 0.0 {
                        floor_unset = true;
                    }
                    continue;
                }
                var fall = across / normal.z;
                fall = fma(gap, normal.z, fall);
                let face = fma(cell.z + neighbour.z, 0.5, -fall) * parameters.height_per_cell;
                if !(normal.z > 0.0) {
                    if floor_unset {
                        floor_height = face;
                        floor_unset = false;
                    } else {
                        floor_height = max(floor_height, face);
                    }
                }
                if 0.0 < normal.z {
                    ceiling = min(ceiling, face);
                }
                if !floor_unset && floor_height > ceiling {
                    return result;
                }
            }
        }
    }
    result.found = true;
    result.value = ceiling;
    return result;
}

// The fourth kernel: the height and hash of the highest cell over the
// pixel, searched downwards from the highest occupied layer nearby and
// stopping one layer after the first that covers the pixel.
@compute @workgroup_size(16, 16)
fn render(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.width || id.y >= parameters.height {
        return;
    }
    let position = vec2<f32>(f32(id.x) * parameters.cells_per_pixel_x, f32(id.y) * parameters.cells_per_pixel_y);
    let pixel_cell_x = i32(position.x);
    let pixel_cell_y = i32(position.y);
    let row = min(max(pixel_cell_y, -BORDER) + BORDER, i32(parameters.rows) - 1);
    let column = min(max(pixel_cell_x, -BORDER) + BORDER, i32(parameters.columns) - 1);
    let top = dilated[u32(row) * parameters.columns + u32(column)];
    let lowest_layer = -BORDER;
    var layer = min(top, parameters.top_layer - 1);

    var best = 0.0;
    var best_hash = 0.0;
    if layer > lowest_layer {
        var covered_before = false;
        loop {
            for (var cell_y = pixel_cell_y - 1; cell_y <= pixel_cell_y + 1; cell_y++) {
                for (var cell_x = pixel_cell_x - 1; cell_x <= pixel_cell_x + 1; cell_x++) {
                    let index = cell_index(cell_x, cell_y, layer);
                    if cells[index].w == 0.0 {
                        continue;
                    }
                    let found = cell_top(cell_x, cell_y, layer, position, pixel_cell_y);
                    if found.found && found.value > best {
                        best = found.value;
                        best_hash = hashes[index];
                    }
                }
            }
            let covered = best > 0.0;
            if covered_before || layer - 1 <= lowest_layer {
                break;
            }
            covered_before = covered;
            layer -= 1;
        }
    }
    let pixel = id.y * parameters.width + id.x;
    surface[pixel] = best;
    pixel_hashes[pixel] = best_hash;
}
