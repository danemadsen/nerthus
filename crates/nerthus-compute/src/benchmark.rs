//! The device benchmark behind Options › Compute's "Scan Hardware and
//! Determine Speeds": a blur timed on every device with its uploads and
//! downloads, scored
//! 5000 over the milliseconds one run takes, and saved by device id as
//! `Benchmark.json` in the data folder. The scores label the devices in
//! Options › Compute, and the status bar warns when the fastest is off.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use web_time::Instant;

use rayon::prelude::*;

use crate::devices::{self, Device, DeviceInfo};
use crate::gpu::{self, Binding, GpuDevice, GpuError, Shader};

/// The file the scores are saved in, in the data folder.
pub const BENCHMARK_FILE: &str = "Benchmark.json";

/// `ThermalBenchmarkWithTransfer(input, output, 2048, 60, 10)`: the map
/// size, the blur passes per run and the runs timed.
const RESOLUTION: usize = 2048;
const BLUR_ITERATIONS: usize = 60;
const BENCHMARK_ITERATIONS: usize = 10;

const SHADER: Shader = Shader {
    name: "benchmark",
    source: include_str!("benchmark.wgsl"),
    bindings: &[Binding::Uniform, Binding::Read, Binding::ReadWrite],
    entries: &[],
};

/// Scores by device id.
pub type Scores = BTreeMap<String, f64>;

/// The scores saved in the data folder; none when the file is missing or
/// unreadable.
pub fn read_scores(folder: &Path) -> Scores {
    let path = folder.join(BENCHMARK_FILE);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Scores::new();
    };
    let text = text.trim_start_matches('\u{feff}');
    serde_json::from_str(text).unwrap_or_default()
}

/// Saves the scores in the data folder.
pub fn write_scores(folder: &Path, scores: &Scores) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let text = serde_json::to_string_pretty(scores).map_err(std::io::Error::other)?;
    std::fs::write(folder.join(BENCHMARK_FILE), text)
}

/// The id with the highest score (the later one on ties, as an
/// `Aggregate` over the scores keeps it).
pub fn best_device(scores: &Scores) -> Option<&str> {
    let mut best: Option<(&str, f64)> = None;
    for (id, &score) in scores {
        let keep_best = matches!(best, Some((_, best_score)) if best_score > score);
        if !keep_best {
            best = Some((id, score));
        }
    }
    best.map(|(id, _)| id)
}

/// Benchmarks every available device that can compute, telling `report`
/// how each did. Devices that fail score zero.
pub fn scan(mut report: impl FnMut(String)) -> Scores {
    let (devices, _) = devices::available_devices();
    let input = random_plane();
    let mut scores = Scores::new();
    for device in devices.iter().filter(|device| !device.requires_driver_update) {
        let score = benchmark(device, &input).unwrap_or(0.0);
        report(format!("{score:.0} - {} ({})", device.device_name, device.backend_name));
        scores.insert(device.id.clone(), score);
    }
    scores
}

/// One device's score: 5000 over the milliseconds a run takes.
fn benchmark(device: &DeviceInfo, input: &[f32]) -> Option<f64> {
    let milliseconds = if device.is_fallback_device { Some(cpu_run_time(input)) } else { gpu_run_time(device, input) };
    match milliseconds {
        Some(time) if time > 0.0 => Some(5000.0 / time),
        _ => None,
    }
}

/// A map of uniform noise in [0, 1).
fn random_plane() -> Vec<f32> {
    let mut state = 0x2545_F491u32;
    let mut values = Vec::with_capacity(RESOLUTION * RESOLUTION);
    for _ in 0..RESOLUTION * RESOLUTION {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        values.push((state >> 8) as f32 * (1.0 / 16_777_216.0));
    }
    values
}

fn cpu_blur(source: &[f32], target: &mut [f32]) {
    let last = RESOLUTION as i32 - 1;
    target.par_chunks_mut(RESOLUTION).enumerate().for_each(|(y, row)| {
        for (x, value) in row.iter_mut().enumerate() {
            let mut sum = 0.0f32;
            for dy in -1..=1 {
                let sample_y = (y as i32 + dy).clamp(0, last) as usize;
                for dx in -1..=1 {
                    let sample_x = (x as i32 + dx).clamp(0, last) as usize;
                    sum += source[sample_y * RESOLUTION + sample_x];
                }
            }
            *value = sum * (1.0 / 9.0);
        }
    });
}

/// Milliseconds per run on the CPU, the copies in and out included.
fn cpu_run_time(input: &[f32]) -> f64 {
    let start = Instant::now();
    for _ in 0..BENCHMARK_ITERATIONS {
        let mut source = input.to_vec();
        let mut target = vec![0.0f32; input.len()];
        for _ in 0..BLUR_ITERATIONS {
            cpu_blur(&source, &mut target);
            std::mem::swap(&mut source, &mut target);
        }
        std::hint::black_box(&source);
    }
    start.elapsed().as_secs_f64() * 1000.0 / BENCHMARK_ITERATIONS as f64
}

/// The GPU with this id: the active one, or one opened for the benchmark.
fn gpu_for(device: &DeviceInfo) -> Option<Arc<GpuDevice>> {
    let active = devices::active_device(&device.id);
    if let Some(Device::Gpu(gpu)) = active {
        return Some(gpu);
    }
    gpu::open_device(&device.id).map(Arc::new)
}

fn gpu_run(gpu: &GpuDevice, input: &[f32]) -> Result<(), GpuError> {
    let kernel = gpu.kernel(&SHADER, "blur")?;
    let size = [RESOLUTION as u32, RESOLUTION as u32];
    let parameters = gpu.uniform(&size);
    let first = gpu.upload("benchmark input", input)?;
    let second = gpu.zeroed_words("benchmark output", input.len())?;
    let forward = gpu.bind(&kernel, &[&parameters, &first, &second]);
    let backward = gpu.bind(&kernel, &[&parameters, &second, &first]);
    let mut batch = gpu.batch();
    for iteration in 0..BLUR_ITERATIONS {
        let group = if iteration % 2 == 0 { &forward } else { &backward };
        batch.dispatch_plane(&kernel, group, RESOLUTION, RESOLUTION)?;
    }
    batch.finish()?;
    let result: Vec<f32> = gpu.read(&first, input.len())?;
    std::hint::black_box(result);
    Ok(())
}

/// Milliseconds per run on a GPU, uploads and downloads included (after
/// one run that compiles the kernel).
fn gpu_run_time(device: &DeviceInfo, input: &[f32]) -> Option<f64> {
    let gpu = gpu_for(device)?;
    let timed = gpu.run(|gpu| {
        gpu_run(gpu, input)?;
        let start = Instant::now();
        for _ in 0..BENCHMARK_ITERATIONS {
            gpu_run(gpu, input)?;
        }
        Ok(start.elapsed().as_secs_f64() * 1000.0 / BENCHMARK_ITERATIONS as f64)
    });
    match timed {
        Ok(time) => Some(time),
        Err(error) => {
            log::warn!("benchmark failed on {}: {error}", device.device_name);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_best_device_keeps_the_later_tie() {
        let mut scores = Scores::new();
        scores.insert("a".to_string(), 3.0);
        scores.insert("b".to_string(), 7.0);
        scores.insert("c".to_string(), 7.0);
        assert_eq!(best_device(&scores), Some("c"));
        assert_eq!(best_device(&Scores::new()), None);
    }

    #[test]
    fn scores_round_trip_through_the_file() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|duration| duration.as_nanos()).unwrap_or(0);
        let folder = std::env::temp_dir().join(format!("nerthus-benchmark-{nanos}"));
        let mut scores = Scores::new();
        scores.insert("Native/Cpu/Native/Apple/Apple M2 Pro/cpu".to_string(), 12.5);
        write_scores(&folder, &scores).unwrap();
        assert_eq!(read_scores(&folder), scores);
        let _ = std::fs::remove_dir_all(folder);
    }
}
