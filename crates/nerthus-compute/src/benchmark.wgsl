// The benchmark's blur (benchmark.rs): one 3×3 box blur pass with the
// edges clamped.

struct Parameters {
    width: u32,
    height: u32,
}

@group(0) @binding(0) var<uniform> parameters: Parameters;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read_write> blurred: array<f32>;

@compute @workgroup_size(16, 16)
fn blur(@builtin(global_invocation_id) id: vec3<u32>) {
    let width = parameters.width;
    let height = parameters.height;
    if id.x >= width || id.y >= height {
        return;
    }
    let x = i32(id.x);
    let y = i32(id.y);
    var sum = 0.0;
    for (var dy = -1; dy <= 1; dy++) {
        let row = u32(clamp(y + dy, 0, i32(height) - 1));
        for (var dx = -1; dx <= 1; dx++) {
            let column = u32(clamp(x + dx, 0, i32(width) - 1));
            sum += source[row * width + column];
        }
    }
    blurred[id.y * width + id.x] = sum * (1.0 / 9.0);
}
