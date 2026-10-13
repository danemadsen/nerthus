//! `ThermalShaper_Process` on a GPU: [`crate::thermal_shaper`]'s pyramid,
//! shaping passes, reconstruction and blend as kernels. The plan of passes
//! is worked out on the CPU, as the SYCL host works it out.

use bytemuck::{Pod, Zeroable};
use nerthus_compute::wgpu;
use nerthus_compute::{Batch, Binding, Entry, GpuDevice, GpuError, Shader};
use nerthus_engine::Map;

use crate::gpu::{pyramid, GpuPlane};
use crate::thermal_shaper::{self, ThermalShaperSettings};

const SHADER: Shader = Shader {
    name: "thermal-shaper",
    source: include_str!("shaders/thermal-shaper.wgsl"),
    bindings: &[
        Binding::Uniform,
        Binding::Read,
        Binding::Read,
        Binding::ReadWrite,
        Binding::Read,
        Binding::Read,
        Binding::ReadWrite,
        Binding::Read,
        Binding::ReadWrite,
    ],
    entries: &[
        Entry { name: "shape_pass", bindings: &[0, 1, 2, 3] },
        Entry { name: "reconstruct", bindings: &[0, 4, 5, 6] },
        Entry { name: "blend", bindings: &[0, 7, 1, 8] },
    ],
};

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct Parameters {
    width: u32,
    height: u32,
    coarse_width: u32,
    coarse_height: u32,
    scale: f32,
    growth: f32,
    shape: f32,
    cell: f32,
    microdetail: f32,
    influence: f32,
    use_intensity: u32,
    negated: u32,
}

/// One level of the pyramid on the GPU.
struct Level {
    heights: GpuPlane,
    /// What this level loses to the next coarser one (none on the
    /// coarsest).
    detail: Option<GpuPlane>,
    intensity: Option<GpuPlane>,
    /// Counted from the coarsest level.
    index: i32,
}

/// The level's passes, ping-ponging between its plane and a scratch one;
/// like the CPU code, the level keeps what its own plane last received.
fn process_level(gpu: &GpuDevice, batch: &mut Batch, level: &Level, fraction: f32, budget: f32, shape: f32, settings: &ThermalShaperSettings) -> Result<(), GpuError> {
    let width = level.heights.width;
    let height = level.heights.height;
    let plan = thermal_shaper::plan_level(level.index, width, fraction, budget, settings);
    if plan.count <= 0 {
        return Ok(());
    }

    let kernel = gpu.kernel(&SHADER, "shape_pass")?;
    let scratch = GpuPlane::zeroed(gpu, "thermal shaper scratch", width, height)?;
    let intensity = level.intensity.as_ref().map_or(gpu.spare(), |plane| &plane.buffer);
    let pass = Parameters {
        width: width as u32,
        height: height as u32,
        scale: plan.scale,
        growth: plan.growth,
        shape,
        cell: plan.cell,
        use_intensity: u32::from(level.intensity.is_some()),
        ..Parameters::default()
    };
    let last_pass = Parameters { shape: plan.remainder * shape, ..pass };
    let full = gpu.uniform(&pass);
    let last = gpu.uniform(&last_pass);
    let group = |parameters: &wgpu::Buffer, source: &GpuPlane, shaped: &GpuPlane| gpu.bind(&kernel, &[parameters, &source.buffer, intensity, &shaped.buffer]);
    let forward = group(&full, &level.heights, &scratch);
    let backward = group(&full, &scratch, &level.heights);
    let last_forward = group(&last, &level.heights, &scratch);
    let last_backward = group(&last, &scratch, &level.heights);

    for pass in 0..plan.count {
        let is_last = pass == plan.count - 1;
        let reads_level = pass % 2 == 0;
        let chosen = match (is_last, reads_level) {
            (false, true) => &forward,
            (false, false) => &backward,
            (true, true) => &last_forward,
            (true, false) => &last_backward,
        };
        batch.dispatch_plane(&kernel, chosen, width, height)?;
    }
    Ok(())
}

/// The finer level rebuilt from the shaped coarse one and its own detail.
fn reconstruct(gpu: &GpuDevice, batch: &mut Batch, coarse: &GpuPlane, fine: &Level, microdetail: f32) -> Result<(), GpuError> {
    let Some(detail) = &fine.detail else {
        return Ok(());
    };
    let kernel = gpu.kernel(&SHADER, "reconstruct")?;
    let parameters = Parameters {
        width: fine.heights.width as u32,
        height: fine.heights.height as u32,
        coarse_width: coarse.width as u32,
        coarse_height: coarse.height as u32,
        microdetail,
        ..Parameters::default()
    };
    let uniform = gpu.uniform(&parameters);
    let group = gpu.bind(&kernel, &[&uniform, &coarse.buffer, &detail.buffer, &fine.heights.buffer]);
    batch.dispatch_plane(&kernel, &group, fine.heights.width, fine.heights.height)
}

/// `ThermalShaper_Process` on `gpu`.
pub fn process(gpu: &GpuDevice, heights: &Map, intensity: Option<&Map>, settings: &ThermalShaperSettings) -> Result<Map, GpuError> {
    let resolution = heights.resolution();
    let width = resolution.x;
    let height = resolution.y;
    let original = heights.channel(0);
    let (negated, shape) = thermal_shaper::orientation(settings.shape);
    let start: Vec<f32> = if negated { original.iter().map(|value| -value).collect() } else { original.to_vec() };
    let (fraction, level_count) = thermal_shaper::pyramid_shape(width, settings);

    let mut batch = gpu.batch();
    let finest_heights = GpuPlane::upload(gpu, "thermal shaper", &start, width, height)?;
    let finest_intensity = match intensity {
        Some(map) => Some(GpuPlane::upload(gpu, "intensity", map.channel(0), width, height)?),
        None => None,
    };
    let finest = Level { heights: finest_heights, detail: None, intensity: finest_intensity, index: 0 };
    let mut pyramid = vec![finest];
    for _ in 1..level_count.max(1) {
        let parent = pyramid.last_mut().expect("the pyramid has a level");
        let coarse_heights = pyramid::downsample(gpu, &mut batch, &parent.heights)?;
        let coarse_intensity = match &parent.intensity {
            Some(plane) => Some(pyramid::downsample(gpu, &mut batch, plane)?),
            None => None,
        };
        let detail = pyramid::detail(gpu, &mut batch, &parent.heights, &coarse_heights)?;
        parent.detail = Some(detail);
        for finer in pyramid.iter_mut() {
            finer.index += 1;
        }
        pyramid.push(Level { heights: coarse_heights, detail: None, intensity: coarse_intensity, index: 0 });
    }

    let budget = thermal_shaper::pass_budget(pyramid.len(), shape);
    while pyramid.len() > 1 {
        let level = pyramid.pop().expect("more than one level is left");
        process_level(gpu, &mut batch, &level, fraction, budget, shape, settings)?;
        let parent = pyramid.last().expect("a finer level is left");
        reconstruct(gpu, &mut batch, &level.heights, parent, settings.microdetail)?;
    }
    let finest = pyramid.pop().expect("the finest level is left");
    process_level(gpu, &mut batch, &finest, fraction, budget, shape, settings)?;

    let kernel = gpu.kernel(&SHADER, "blend")?;
    let original_plane = GpuPlane::upload(gpu, "original heights", original, width, height)?;
    let blended = GpuPlane::zeroed(gpu, "shaped heights", width, height)?;
    let parameters = Parameters {
        width: width as u32,
        height: height as u32,
        influence: settings.influence,
        negated: u32::from(negated),
        ..Parameters::default()
    };
    let uniform = gpu.uniform(&parameters);
    let group = gpu.bind(&kernel, &[&uniform, &original_plane.buffer, &finest.heights.buffer, &blended.buffer]);
    batch.dispatch_plane(&kernel, &group, width, height)?;
    batch.finish()?;

    let data = blended.read(gpu)?;
    Ok(Map::from_data(resolution, data))
}
