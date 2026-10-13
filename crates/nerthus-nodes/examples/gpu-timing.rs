//! Times the heavier SYCL functions on a GPU alone, for sizes where the
//! CPU comparison (gpu-compare) would take too long.
//!
//! usage: gpu-timing [resolution]

use std::time::Instant;

use nerthus_engine::{Map, Resolution};
use nerthus_nodes::flow::FlowSettings;
use nerthus_nodes::gpu;
use nerthus_nodes::parallel_erosion::ErosionSettings;
use nerthus_nodes::thermal::ThermalSettings;

const TERRAIN: (f32, f32) = (5000.0, 2600.0);

/// A smooth bumpy heightfield in [0, 1].
fn terrain(size: usize) -> Map {
    let mut data = vec![0.0f32; size * size];
    for y in 0..size {
        for x in 0..size {
            let u = x as f32 / size as f32 * 12.0;
            let v = y as f32 / size as f32 * 12.0;
            let value = (u.sin() * v.cos() + (u * 2.3 + 1.0).sin() * 0.5 + (v * 3.1).cos() * 0.25) * 0.25 + 0.5;
            data[y * size + x] = value;
        }
    }
    Map::from_data(Resolution::square(size), data)
}

fn timed(name: &str, work: impl FnOnce()) {
    let start = Instant::now();
    work();
    println!("{name:<12} {:.3}s", start.elapsed().as_secs_f64());
}

fn main() {
    let size = std::env::args().nth(1).and_then(|text| text.parse().ok()).unwrap_or(2048);
    let (devices, _) = nerthus_compute::available_devices();
    let usable = devices.iter().find(|device| !device.is_fallback_device && !device.requires_driver_update).expect("a GPU can compute here");
    let gpu = nerthus_compute::open_device(&usable.id).expect("the GPU opens");
    println!("GPU: {} ({}), resolution {size}", usable.device_name, usable.backend_name);
    let heights = terrain(size);

    let flow = FlowSettings { flow_length: 0.1, flow_volume: 0.5, seed: 2869 };
    timed("flow", || {
        gpu.run(|gpu| gpu::flow::simulate(gpu, &heights, None, &flow)).expect("flow runs");
    });
    let thermal = ThermalSettings {
        duration: 0.04,
        strength: 0.25,
        anisotropy: 0.25,
        talus_angle: 32.0,
        erosion_scale: 25.0,
        heightmap_size: TERRAIN.0,
        heightmap_height: TERRAIN.1,
        sediment_removal: 0.0,
    };
    timed("thermal", || {
        gpu.run(|gpu| gpu::thermal::simulate(gpu, &heights, None, None, &thermal)).expect("thermal runs");
    });
    let erosion = ErosionSettings { seed: 19888, ..ErosionSettings::default() };
    timed("erosion", || {
        gpu.run(|gpu| gpu::parallel_erosion::erode(gpu, &heights, None, &erosion, TERRAIN)).expect("erosion runs");
    });
}
