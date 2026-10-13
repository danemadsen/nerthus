//! Runs the SYCL functions on the CPU and on a GPU and compares them: how
//! far apart the results are and how long each took.
//!
//! usage: gpu-compare [resolution] [function…]
//!
//! With no functions named, every one runs. The GPU is the first wgpu
//! device that can compute, opened directly (whatever Options › Compute
//! has active).

use std::time::Instant;

use nerthus_compute::GpuDevice;
use nerthus_engine::{Map, Resolution};
use nerthus_nodes::gpu;
use nerthus_nodes::cellular3d::{self, Cellular3dSettings};
use nerthus_nodes::colour_transport::{self, ColourTransportSettings};
use nerthus_nodes::flow::{self, FlowSettings};
use nerthus_nodes::lighting::{self, SunlightSettings};
use nerthus_nodes::median::{self, MedianQuality};
use nerthus_nodes::parallel_erosion::{self, ErosionSettings};
use nerthus_nodes::mesh_rasterizer::{self, RenderSettings, Texture, TriplanarSettings};
use nerthus_nodes::shade::{AmbientOcclusionModel, LightRig};
use nerthus_nodes::thermal::{self, ThermalSettings};
use nerthus_nodes::thermal_shaper::{self, ThermalShaperSettings};

/// The terrain size the cases build for, in metres.
const TERRAIN: (f32, f32) = (5000.0, 2600.0);

/// A heightfield with some structure at every scale: value-noise fBm,
/// ridged at the large scales, in [0, 1].
fn terrain(size: usize, seed: u32) -> Map {
    let hash = |x: i32, y: i32, octave: u32| {
        let mut state = (x as u32).wrapping_mul(0x8DA6_B343) ^ (y as u32).wrapping_mul(0xD816_3841) ^ seed.wrapping_add(octave).wrapping_mul(0xCB1A_B31F);
        state ^= state >> 13;
        state = state.wrapping_mul(0x5BD1_E995);
        state ^= state >> 15;
        (state & 0xFFFF) as f32 / 65535.0
    };
    let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
    let mut data = vec![0.0f32; size * size];
    for y in 0..size {
        for x in 0..size {
            let mut value = 0.0f32;
            let mut amplitude = 0.5f32;
            let mut frequency = 4.0 / size as f32;
            for octave in 0..8u32 {
                let u = x as f32 * frequency;
                let v = y as f32 * frequency;
                let (cell_x, cell_y) = (u.floor() as i32, v.floor() as i32);
                let (fx, fy) = (smooth(u - cell_x as f32), smooth(v - cell_y as f32));
                let top = hash(cell_x, cell_y, octave) * (1.0 - fx) + hash(cell_x + 1, cell_y, octave) * fx;
                let bottom = hash(cell_x, cell_y + 1, octave) * (1.0 - fx) + hash(cell_x + 1, cell_y + 1, octave) * fx;
                let mut noise = top * (1.0 - fy) + bottom * fy;
                if octave < 3 {
                    noise = 1.0 - (noise * 2.0 - 1.0).abs();
                }
                value += noise * amplitude;
                amplitude *= 0.5;
                frequency *= 2.0;
            }
            data[y * size + x] = value;
        }
    }
    let low = data.iter().copied().fold(f32::MAX, f32::min);
    let high = data.iter().copied().fold(f32::MIN, f32::max);
    for value in data.iter_mut() {
        *value = (*value - low) / (high - low);
    }
    Map::from_data(Resolution::square(size), data)
}

/// How far two results are apart.
struct Difference {
    largest: f32,
    mean: f32,
    range: f32,
    different: usize,
    /// Pearson's correlation of the two.
    correlation: f64,
    /// The GPU's total over the CPU's.
    total_ratio: f64,
}

fn compare(cpu: &[f32], gpu: &[f32]) -> Difference {
    let mut largest = 0.0f32;
    let mut sum = 0.0f64;
    let mut different = 0;
    for (&a, &b) in cpu.iter().zip(gpu) {
        let gap = (a - b).abs();
        if gap.is_nan() || a.to_bits() != b.to_bits() {
            different += 1;
        }
        largest = largest.max(if gap.is_nan() { f32::INFINITY } else { gap });
        sum += gap as f64;
    }
    let low = cpu.iter().copied().fold(f32::MAX, f32::min);
    let high = cpu.iter().copied().fold(f32::MIN, f32::max);
    let count = cpu.len().max(1) as f64;
    let cpu_total: f64 = cpu.iter().map(|&value| value as f64).sum();
    let gpu_total: f64 = gpu.iter().map(|&value| value as f64).sum();
    let (cpu_mean, gpu_mean) = (cpu_total / count, gpu_total / count);
    let mut covariance = 0.0f64;
    let mut cpu_variance = 0.0f64;
    let mut gpu_variance = 0.0f64;
    for (&a, &b) in cpu.iter().zip(gpu) {
        let (da, db) = (a as f64 - cpu_mean, b as f64 - gpu_mean);
        covariance += da * db;
        cpu_variance += da * da;
        gpu_variance += db * db;
    }
    let spread = (cpu_variance * gpu_variance).sqrt();
    Difference {
        largest,
        mean: (sum / count) as f32,
        range: high - low,
        different,
        correlation: if spread > 0.0 { covariance / spread } else { 1.0 },
        total_ratio: if cpu_total != 0.0 { gpu_total / cpu_total } else { 1.0 },
    }
}

/// With `NERTHUS_COMPARE_DUMP=<folder>`, writes both results as 16-bit
/// PGM images (scaled to the CPU result's range) for a look.
fn dump(name: &str, cpu: &[f32], gpu: &[f32]) {
    let Some(folder) = std::env::var_os("NERTHUS_COMPARE_DUMP") else {
        return;
    };
    let size = (cpu.len() as f64).sqrt() as usize;
    if size * size != cpu.len() || gpu.len() != cpu.len() {
        return;
    }
    let low = cpu.iter().copied().fold(f32::MAX, f32::min);
    let high = cpu.iter().copied().fold(f32::MIN, f32::max);
    let span = if high > low { high - low } else { 1.0 };
    let file_name = name.replace(' ', "-");
    for (side, values) in [("cpu", cpu), ("gpu", gpu)] {
        let mut bytes = format!("P5 {size} {size} 65535\n").into_bytes();
        for &value in values {
            let level = (((value - low) / span).clamp(0.0, 1.0) * 65535.0) as u16;
            bytes.extend_from_slice(&level.to_be_bytes());
        }
        let path = std::path::Path::new(&folder).join(format!("{file_name}-{side}.pgm"));
        let _ = std::fs::write(path, bytes);
    }
}

fn report(name: &str, cpu: &[f32], gpu: &[f32], cpu_seconds: f64, gpu_seconds: f64) {
    dump(name, cpu, gpu);
    if cpu.len() != gpu.len() {
        println!("{name:<28} sizes differ: {} vs {}", cpu.len(), gpu.len());
        return;
    }
    let difference = compare(cpu, gpu);
    let relative = if difference.range > 0.0 { difference.largest / difference.range } else { difference.largest };
    println!(
        "{name:<32} max {:.2e} ({:.2e} of range)  mean {:.2e}  r {:.6}  total {:.5}  {}/{} differ  cpu {:.3}s gpu {:.3}s",
        difference.largest,
        relative,
        difference.mean,
        difference.correlation,
        difference.total_ratio,
        difference.different,
        cpu.len(),
        cpu_seconds,
        gpu_seconds
    );
}

fn timed<T>(work: impl FnOnce() -> T) -> (T, f64) {
    let start = Instant::now();
    let result = work();
    (result, start.elapsed().as_secs_f64())
}

fn open_gpu() -> GpuDevice {
    let (devices, _) = nerthus_compute::available_devices();
    let usable = devices.iter().find(|device| !device.is_fallback_device && !device.requires_driver_update);
    let Some(device) = usable else {
        panic!("no GPU can compute here");
    };
    println!("GPU: {} ({})", device.device_name, device.backend_name);
    nerthus_compute::open_device(&device.id).expect("the GPU opens")
}

fn median_cases(gpu: &GpuDevice, heights: &Map) {
    for (quality, strength) in [(MedianQuality::Low, 1), (MedianQuality::Medium, 4), (MedianQuality::High, 2)] {
        let (cpu, cpu_seconds) = timed(|| median::resolution_independent_cpu(heights, strength, quality));
        let (on_gpu, gpu_seconds) = timed(|| gpu.run(|gpu| gpu::median::resolution_independent(gpu, heights, strength, quality)));
        let on_gpu = on_gpu.expect("median runs on the GPU");
        let name = format!("median {quality:?} x{strength}");
        report(&name, cpu.data(), on_gpu.data(), cpu_seconds, gpu_seconds);
    }
}

fn thermal_shaper_cases(gpu: &GpuDevice, heights: &Map) {
    let mask = terrain(heights.width(), 11);
    let cases = [("default", -0.25, 20.0, false), ("convex", 0.6, 60.0, false), ("masked", -0.25, 20.0, true)];
    for (label, shape, scale, masked) in cases {
        let settings = ThermalShaperSettings {
            terrain_width: TERRAIN.0,
            terrain_height: TERRAIN.1,
            scale,
            influence: 0.5,
            shape,
            microdetail: 0.5,
        };
        let intensity = masked.then_some(&mask);
        let (cpu, cpu_seconds) = timed(|| thermal_shaper::process_cpu(heights, intensity, &settings));
        let (on_gpu, gpu_seconds) = timed(|| gpu.run(|gpu| gpu::thermal_shaper::process(gpu, heights, intensity, &settings)));
        let on_gpu = on_gpu.expect("the thermal shaper runs on the GPU");
        report(&format!("thermal shaper {label}"), cpu.data(), on_gpu.data(), cpu_seconds, gpu_seconds);
    }
}

fn thermal_cases(gpu: &GpuDevice, heights: &Map) {
    let mask = terrain(heights.width(), 13);
    // Duration, anisotropy, sediment removal, masks.
    let cases = [
        ("short", 0.002, 0.25, 0.0, false),
        ("default", 0.04, 0.25, 0.0, false),
        ("long anisotropic", 0.2, 0.8, 0.0, false),
        ("masked removal", 0.04, 0.25, 0.5, true),
    ];
    for (label, duration, anisotropy, sediment_removal, masked) in cases {
        let settings = ThermalSettings {
            duration,
            strength: 0.25,
            anisotropy,
            talus_angle: 32.0,
            erosion_scale: 25.0,
            heightmap_size: TERRAIN.0,
            heightmap_height: TERRAIN.1,
            sediment_removal,
        };
        let masks = masked.then_some(&mask);
        let (cpu, cpu_seconds) = timed(|| thermal::simulate_cpu(heights, masks, masks, &settings));
        let (on_gpu, gpu_seconds) = timed(|| gpu.run(|gpu| gpu::thermal::simulate(gpu, heights, masks, masks, &settings)));
        let on_gpu = on_gpu.expect("thermal erosion runs on the GPU");
        report(&format!("thermal {label} heights"), cpu.heights.data(), on_gpu.heights.data(), cpu_seconds, gpu_seconds);
        report(&format!("thermal {label} deposits"), cpu.deposition.data(), on_gpu.deposition.data(), cpu_seconds, gpu_seconds);
        report(&format!("thermal {label} wear"), cpu.wear.data(), on_gpu.wear.data(), cpu_seconds, gpu_seconds);
    }
}

fn flow_cases(gpu: &GpuDevice, heights: &Map) {
    let rain = terrain(heights.width(), 17);
    let cases = [("default", 0.1, 0.5, false), ("long", 0.6, 1.0, false), ("rain", 0.1, 0.5, true)];
    for (label, flow_length, flow_volume, raining) in cases {
        let settings = FlowSettings { flow_length, flow_volume, seed: 2869 };
        let precipitation = raining.then_some(&rain);
        let (cpu, cpu_seconds) = timed(|| flow::simulate_cpu(heights, precipitation, &settings));
        let (on_gpu, gpu_seconds) = timed(|| gpu.run(|gpu| gpu::flow::simulate(gpu, heights, precipitation, &settings)));
        let on_gpu = on_gpu.expect("the flow simulation runs on the GPU");
        report(&format!("flow {label}"), cpu.data(), on_gpu.data(), cpu_seconds, gpu_seconds);
    }
}

fn colour_transport_cases(gpu: &GpuDevice, heights: &Map) {
    let size = heights.width();
    let red = terrain(size, 21);
    let green = terrain(size, 22);
    let blue = terrain(size, 23);
    let colour = Map::from_channels(heights.resolution(), vec![red.data().to_vec(), green.data().to_vec(), blue.data().to_vec()]);
    let cases = [("default", 0.5, false, 0.5, false), ("laminar far", 1.0, true, 0.9, false), ("grey", 0.5, false, 0.5, true)];
    for (label, transport_distance, laminar_flow, colour_hold, grey) in cases {
        let settings = ColourTransportSettings {
            transport_distance,
            sediment_density: 0.5,
            blend: 0.5,
            flow_volume: 0.0,
            colour_hold,
            laminar_flow,
            seed: 36557,
        };
        let texture = if grey { &red } else { &colour };
        let (cpu, cpu_seconds) = timed(|| colour_transport::simulate_cpu(heights, texture, None, None, &settings));
        let (on_gpu, gpu_seconds) = timed(|| gpu.run(|gpu| gpu::colour_transport::simulate(gpu, heights, texture, None, None, &settings)));
        let on_gpu = on_gpu.expect("colour transport runs on the GPU");
        report(&format!("colour transport {label}"), cpu.data(), on_gpu.data(), cpu_seconds, gpu_seconds);
    }
}

fn cellular3d_cases(gpu: &GpuDevice, heights: &Map) {
    let gaps = terrain(heights.width(), 31);
    // Cells across, gap, z scale, z jitter, gap map.
    let cases = [
        ("default", 10.0, 0.0, 1.0, 1.0, false),
        ("ice floe", 1.0 / 0.3, 0.5, 5.0, 1.0, true),
        ("dense", 60.0, 0.2, 1.0, 1.0, false),
        ("flat jitter", 12.0, 0.1, 1.0, 0.0, false),
    ];
    for (label, cells, gap, scale_z, jitter_z, gapped) in cases {
        let settings = Cellular3dSettings {
            cells,
            gap,
            scale: [1.0, 1.0, scale_z],
            jitter: [1.0, 1.0, jitter_z],
            seed: 40861,
            terrain_width: TERRAIN.0,
            terrain_height: TERRAIN.1,
        };
        let gap_map = gapped.then_some(&gaps);
        let (cpu, cpu_seconds) = timed(|| cellular3d::render_cpu(heights, gap_map, &settings));
        let (on_gpu, gpu_seconds) = timed(|| gpu.run(|gpu| gpu::cellular3d::render(gpu, heights, gap_map, &settings)));
        let on_gpu = on_gpu.expect("Cellular3D renders on the GPU");
        report(&format!("cellular3d {label} surface"), cpu.0.data(), on_gpu.0.data(), cpu_seconds, gpu_seconds);
        report(&format!("cellular3d {label} hashes"), cpu.1.data(), on_gpu.1.data(), cpu_seconds, gpu_seconds);
    }
}

fn mesh_cases(gpu: &GpuDevice, heights: &Map) {
    let size = heights.width();
    let side = size as i32;
    let ratio = TERRAIN.1 / TERRAIN.0;
    let mesh = mesh_rasterizer::heightmap_to_mesh(heights.data(), size, size, TERRAIN.0, TERRAIN.1);
    // Pitch, yaw, roll, scale, antialiasing.
    let renders = [("flat", 0.0, 0.0, 0.0, 1.0, false), ("tilted antialiased", 25.0, 30.0, 10.0, 0.8, true), ("close", 0.0, 45.0, 0.0, 3.0, false)];
    for (label, pitch, yaw, roll, scale, antialiasing) in renders {
        let settings = RenderSettings {
            width: side,
            height: side,
            height_ratio: ratio,
            pitch,
            yaw,
            roll,
            scale,
            offset: [0.0, 0.0, 0.0],
            drop_to_floor: true,
            antialiasing,
        };
        let (cpu, cpu_seconds) = timed(|| mesh_rasterizer::render_mesh_cpu(&mesh, &settings));
        let (on_gpu, gpu_seconds) = timed(|| gpu.run(|gpu| gpu::mesh_rasterizer::render_mesh(gpu, &mesh, &settings)));
        let on_gpu = on_gpu.expect("meshes render on the GPU");
        report(&format!("mesh {label}"), &cpu, &on_gpu, cpu_seconds, gpu_seconds);
    }

    // A point cloud: jittered points carrying the terrain's heights.
    let mut state = 0x1234_5678u32;
    let mut random = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        (state >> 8) as f32 / 16_777_216.0
    };
    let points: Vec<[f32; 3]> = (0..size * size / 16)
        .map(|_| {
            let x = random() * size as f32;
            let y = random() * size as f32;
            let z = heights.get((x as usize).min(size - 1), (y as usize).min(size - 1));
            [x, y, z]
        })
        .collect();
    let (cpu, cpu_seconds) = timed(|| mesh_rasterizer::render_point_cloud_cpu(&points, side, side, None));
    let (on_gpu, gpu_seconds) = timed(|| gpu.run(|gpu| gpu::mesh_rasterizer::render_point_cloud(gpu, &points, side, side, None)));
    let on_gpu = on_gpu.expect("point clouds render on the GPU");
    report("point cloud heights", &cpu.0, &on_gpu.heights, cpu_seconds, gpu_seconds);
    report("point cloud hashes", &cpu.1, &on_gpu.hashes, cpu_seconds, gpu_seconds);

    let red = terrain(size, 41);
    let green = terrain(size, 42);
    let blue = terrain(size, 43);
    let mut cpu_planes = vec![red.data().to_vec(), green.data().to_vec(), blue.data().to_vec()];
    let gpu_planes = cpu_planes.clone();
    let painted = Texture::new(cpu_planes.iter_mut().map(|plane| plane.as_mut_slice()).collect());
    let (cpu, cpu_seconds) = timed(|| mesh_rasterizer::render_point_cloud_cpu(&points, side, side, Some(painted)));
    let planes: Vec<&[f32]> = gpu_planes.iter().map(Vec::as_slice).collect();
    let (on_gpu, gpu_seconds) = timed(|| gpu.run(|gpu| gpu::mesh_rasterizer::render_point_cloud(gpu, &points, side, side, Some(&planes))));
    let on_gpu = on_gpu.expect("textured point clouds render on the GPU");
    report("textured cloud heights", &cpu.0, &on_gpu.heights, cpu_seconds, gpu_seconds);
    let painted = on_gpu.painted.expect("the texture comes back painted");
    report("textured cloud colours", &cpu_planes.concat(), &painted.concat(), cpu_seconds, gpu_seconds);

    let normals_mesh = mesh_rasterizer::heightmap_to_mesh_with_normals(heights.data(), size, size, TERRAIN.0, TERRAIN.1, 6);
    for (label, horizontal, antialiasing) in [("triplanar", false, false), ("triplanar horizontal antialiased", true, true)] {
        let settings = TriplanarSettings {
            width: side,
            height: side,
            height_ratio: ratio,
            strength: 0.1,
            offset: -0.4,
            horizontal,
            antialiasing,
        };
        let maps = [red.data(), green.data(), blue.data()];
        let (cpu, cpu_seconds) = timed(|| mesh_rasterizer::render_mesh_triplanar_cpu(&normals_mesh, &settings, maps));
        let (on_gpu, gpu_seconds) = timed(|| gpu.run(|gpu| gpu::mesh_rasterizer::render_mesh_triplanar(gpu, &normals_mesh, &settings, maps)));
        let on_gpu = on_gpu.expect("displaced meshes render on the GPU");
        report(label, &cpu, &on_gpu, cpu_seconds, gpu_seconds);
    }
}

fn erosion_cases(gpu: &GpuDevice, heights: &Map) {
    let rain = terrain(heights.width(), 51);
    let defaults = ErosionSettings { seed: 19888, shape_detail_scale: 0.25, ..ErosionSettings::default() };
    let heavy = ErosionSettings {
        downcutting: 0.6,
        downcutting_type: 0.5,
        bed_load_amount: 0.5,
        coarse_sediments_amount: 0.5,
        deposition_boost: 0.5,
        extra_deposition_boost: 0.3,
        ..defaults
    };
    let cases = [("default", defaults, false), ("rain", defaults, true), ("heavy", heavy, false)];
    for (label, settings, raining) in cases {
        let precipitation = raining.then_some(&rain);
        let (cpu, cpu_seconds) = timed(|| parallel_erosion::erode_cpu(heights, precipitation, &settings, TERRAIN));
        let (on_gpu, gpu_seconds) = timed(|| gpu.run(|gpu| gpu::parallel_erosion::erode(gpu, heights, precipitation, &settings, TERRAIN)));
        let on_gpu = on_gpu.expect("erosion runs on the GPU");
        report(&format!("erosion {label} heights"), cpu.height.data(), on_gpu.height.data(), cpu_seconds, gpu_seconds);
        report(&format!("erosion {label} flow"), cpu.flow.data(), on_gpu.flow.data(), cpu_seconds, gpu_seconds);
        report(&format!("erosion {label} deposition"), cpu.deposition.data(), on_gpu.deposition.data(), cpu_seconds, gpu_seconds);
        report(&format!("erosion {label} wear"), cpu.wear.data(), on_gpu.wear.data(), cpu_seconds, gpu_seconds);
    }

    // The GPU's integer sums make it repeat itself exactly.
    let (first, first_seconds) = timed(|| gpu.run(|gpu| gpu::parallel_erosion::erode(gpu, heights, Some(&rain), &defaults, TERRAIN)));
    let (second, second_seconds) = timed(|| gpu.run(|gpu| gpu::parallel_erosion::erode(gpu, heights, Some(&rain), &defaults, TERRAIN)));
    let (first, second) = (first.expect("erosion runs"), second.expect("erosion runs"));
    report("erosion repeat heights", first.height.data(), second.height.data(), first_seconds, second_seconds);
    report("erosion repeat flow", first.flow.data(), second.flow.data(), first_seconds, second_seconds);
}

fn lighting_cases(gpu: &GpuDevice, heights: &Map) {
    let settings = SunlightSettings {
        day_begin: 73,
        day_end: 219,
        latitude: 60.0,
        north: 0.0,
        air_density: 0.01,
    };
    let (cpu, cpu_seconds) = timed(|| lighting::sunlight_integral_cpu(heights, &settings, TERRAIN));
    let (on_gpu, gpu_seconds) = timed(|| gpu.run(|gpu| gpu::lighting::sunlight_integral(gpu, heights, &settings, TERRAIN)));
    let on_gpu = on_gpu.expect("sunlight runs on the GPU");
    report("sunlight", cpu.data(), on_gpu.data(), cpu_seconds, gpu_seconds);

    let colour = Map::from_channels(heights.resolution(), vec![terrain(heights.width(), 61).into_data(), terrain(heights.width(), 62).into_data(), terrain(heights.width(), 63).into_data()]);
    let models = [("raytraced", AmbientOcclusionModel::Raytraced, 5, true), ("hybrid", AmbientOcclusionModel::Hybrid, 2, false), ("local", AmbientOcclusionModel::Local, -4, true)];
    for (label, ao_model, shadow_quality, textured) in models {
        let rig = LightRig { ao_model, shadow_quality, hybrid_ao_octaves: 3, ..LightRig::default() };
        let sky = rig.sky(135.0, 30.0);
        let texture = textured.then_some(&colour);
        let (cpu, cpu_seconds) = timed(|| rig.shade_cpu(&sky, heights, texture, TERRAIN));
        let (on_gpu, gpu_seconds) = timed(|| gpu.run(|gpu| gpu::lighting::shade(gpu, &rig, &sky, heights, texture, TERRAIN)));
        let on_gpu = on_gpu.expect("shading runs on the GPU");
        report(&format!("shade {label} diffuse"), cpu.diffuse.data(), on_gpu.diffuse.data(), cpu_seconds, gpu_seconds);
        report(&format!("shade {label} shadow"), cpu.shadow.data(), on_gpu.shadow.data(), cpu_seconds, gpu_seconds);
        report(&format!("shade {label} occlusion"), cpu.occlusion.data(), on_gpu.occlusion.data(), cpu_seconds, gpu_seconds);
        report(&format!("shade {label} colour"), cpu.color.data(), on_gpu.color.data(), cpu_seconds, gpu_seconds);
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let mut arguments = std::env::args().skip(1).peekable();
    let resolution = match arguments.peek().and_then(|text| text.parse::<usize>().ok()) {
        Some(resolution) => {
            arguments.next();
            resolution
        }
        None => 512,
    };
    let wanted: Vec<String> = arguments.collect();
    let runs = |name: &str| wanted.is_empty() || wanted.iter().any(|wanted| wanted == name);

    let gpu = open_gpu();
    let heights = terrain(resolution, 7);
    println!("resolution {resolution}");
    if runs("cellular3d") {
        cellular3d_cases(&gpu, &heights);
    }
    if runs("colour-transport") {
        colour_transport_cases(&gpu, &heights);
    }
    if runs("erosion") {
        erosion_cases(&gpu, &heights);
    }
    if runs("flow") {
        flow_cases(&gpu, &heights);
    }
    if runs("mesh") {
        mesh_cases(&gpu, &heights);
    }
    if runs("lighting") {
        lighting_cases(&gpu, &heights);
    }
    if runs("median") {
        median_cases(&gpu, &heights);
    }
    if runs("thermal") {
        thermal_cases(&gpu, &heights);
    }
    if runs("thermal-shaper") {
        thermal_shaper_cases(&gpu, &heights);
    }
}
