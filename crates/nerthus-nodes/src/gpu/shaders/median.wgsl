// The SYCL median filter on the GPU (median.rs): one smoothed-median
// pass over the disc of radius r + ½ around each pixel, edges clamped.

struct Parameters {
    width: u32,
    height: u32,
    radius: i32,
    _padding: u32,
}

@group(0) @binding(0) var<uniform> parameters: Parameters;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read_write> filtered: array<f32>;

// The disc of the largest radius (4) holds 69 samples.
const MOST_SAMPLES: u32 = 69u;

@compute @workgroup_size(16, 16)
fn smoothed_median(@builtin(global_invocation_id) id: vec3<u32>) {
    let width = parameters.width;
    let height = parameters.height;
    if id.x >= width || id.y >= height {
        return;
    }
    let radius = parameters.radius;
    let limit = radius * (radius + 1);
    let last_x = i32(width) - 1;
    let last_y = i32(height) - 1;

    var window: array<f32, MOST_SAMPLES>;
    var count = 0u;
    for (var offset_y = -radius; offset_y <= radius; offset_y++) {
        let row = u32(clamp(i32(id.y) + offset_y, 0, last_y));
        for (var offset_x = -radius; offset_x <= radius; offset_x++) {
            if offset_x * offset_x + offset_y * offset_y <= limit {
                let column = u32(clamp(i32(id.x) + offset_x, 0, last_x));
                window[count] = source[row * width + column];
                count++;
            }
        }
    }

    // The middle order statistic and its two neighbours in sorted order. A
    // sample's rank is how many samples sort before it, equal ones by
    // their place in the window.
    let middle = count / 2u;
    var below = 0.0;
    var centre = 0.0;
    var above = 0.0;
    for (var index = 0u; index < count; index++) {
        let value = window[index];
        var rank = 0u;
        for (var other = 0u; other < count; other++) {
            let compared = window[other];
            if compared < value || (compared == value && other < index) {
                rank++;
            }
        }
        if rank + 1u == middle {
            below = value;
        } else if rank == middle {
            centre = value;
        } else if rank == middle + 1u {
            above = value;
        }
    }
    filtered[id.y * width + id.x] = fma(below + above, 0.25, centre * 0.5);
}
