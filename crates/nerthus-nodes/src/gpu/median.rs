//! `MedianFilter` on a GPU: [`crate::median`]'s passes, one dispatch each,
//! ping-ponging between two buffers.

use bytemuck::{Pod, Zeroable};
use nerthus_compute::{Binding, GpuDevice, GpuError, Shader};
use nerthus_engine::Map;

use crate::median::MedianQuality;

const SHADER: Shader = Shader {
    name: "median",
    source: include_str!("shaders/median.wgsl"),
    bindings: &[Binding::Uniform, Binding::Read, Binding::ReadWrite],
    entries: &[],
};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Parameters {
    width: u32,
    height: u32,
    radius: i32,
    padding: u32,
}

/// `MedianFilter.ResolutionIndependent` on `gpu`.
pub fn resolution_independent(gpu: &GpuDevice, map: &Map, strength: i32, quality: MedianQuality) -> Result<Map, GpuError> {
    let width = map.width();
    let height = map.height();
    let pixels = width * height;
    let radius = quality.radius();
    let iterations = crate::median::iterations(width, strength, quality);

    let kernel = gpu.kernel(&SHADER, "smoothed_median")?;
    let parameters = Parameters {
        width: width as u32,
        height: height as u32,
        radius,
        padding: 0,
    };
    let parameters = gpu.uniform(&parameters);
    let first = gpu.upload("median", map.channel(0))?;
    let second = gpu.zeroed_words("median", pixels)?;
    let forward = gpu.bind(&kernel, &[&parameters, &first, &second]);
    let backward = gpu.bind(&kernel, &[&parameters, &second, &first]);

    let mut batch = gpu.batch();
    for iteration in 0..iterations {
        let group = if iteration % 2 == 0 { &forward } else { &backward };
        batch.dispatch_plane(&kernel, group, width, height)?;
    }
    batch.finish()?;

    let result = if iterations % 2 == 1 { &second } else { &first };
    let data = gpu.read(result, pixels)?;
    Ok(Map::from_data(map.resolution(), data))
}
