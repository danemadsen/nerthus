//! `FlowSimulation` on a GPU: [`crate::flow`]'s pit filling and particle
//! launches as kernels. The particles are seeded and shuffled on the CPU,
//! as the SYCL host seeds them.

use bytemuck::{Pod, Zeroable};
use nerthus_compute::{Batch, Binding, Entry, GpuDevice, GpuError, Shader};
use nerthus_engine::Map;

use crate::flow::{self, FlowSettings};
use crate::gpu::{pyramid, GpuPlane};

const FILL_SHADER: Shader = Shader {
    name: "fill-pits",
    source: include_str!("shaders/fill-pits.wgsl"),
    bindings: &[
        Binding::Uniform,
        Binding::Read,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::Read,
        Binding::Read,
        Binding::Read,
        Binding::ReadWrite,
    ],
    entries: &[Entry { name: "fill_pits", bindings: &[0, 1, 2, 3] }, Entry { name: "reconstruct", bindings: &[0, 4, 5, 6, 7] }],
};

const FLOW_SHADER: Shader = Shader {
    name: "flow",
    source: concat!(include_str!("shaders/flow.wgsl"), include_str!("shaders/terrain-sample.wgsl")),
    bindings: &[
        Binding::Uniform,
        Binding::Read,
        Binding::Read,
        Binding::Read,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
    ],
    entries: &[
        // The probe shares the trace's code, counts included (never run).
        Entry { name: "probe", bindings: &[0, 1, 2, 3, 4, 7] },
        Entry { name: "chain", bindings: &[0, 7, 8] },
        Entry { name: "trace", bindings: &[0, 1, 2, 3, 4, 8] },
        Entry { name: "accumulate", bindings: &[0, 4, 5, 6] },
    ],
};

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct FillParameters {
    width: u32,
    height: u32,
    coarse_width: u32,
    coarse_height: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct FlowParameters {
    width: u32,
    height: u32,
    start: u32,
    count: u32,
    seed: u32,
    steps: u32,
    uphill_limit: f32,
    last_step: f32,
    weight: f32,
    raise: f32,
    padding: [u32; 2],
}

/// One level of the pit-filling pyramid on the GPU.
struct FillLevel {
    heights: GpuPlane,
    detail: Option<GpuPlane>,
    flags: Option<GpuPlane>,
}

/// `Flow::process`'s pit filling (see `flow::fill_pyramid`): the filled
/// heights.
pub(crate) fn fill_pyramid(gpu: &GpuDevice, batch: &mut Batch, heights: GpuPlane) -> Result<GpuPlane, GpuError> {
    let level_count = flow::fill_level_count(heights.width);
    let finest = FillLevel { heights, detail: None, flags: None };
    let mut levels = vec![finest];
    for _ in 1..level_count {
        let parent = levels.last_mut().expect("the pyramid has a level");
        let coarse = pyramid::downsample(gpu, batch, &parent.heights)?;
        parent.detail = Some(pyramid::detail(gpu, batch, &parent.heights, &coarse)?);
        let flags = GpuPlane::zeroed(gpu, "fill flags", coarse.width, coarse.height)?;
        levels.push(FillLevel { heights: coarse, detail: None, flags: Some(flags) });
    }

    let fill_kernel = gpu.kernel(&FILL_SHADER, "fill_pits")?;
    let reconstruct_kernel = gpu.kernel(&FILL_SHADER, "reconstruct")?;
    while levels.len() > 1 {
        let level = levels.pop().expect("more than one level is left");
        let width = level.heights.width;
        let height = level.heights.height;
        let flags = level.flags.as_ref().expect("coarse levels have flags");
        let scratch = GpuPlane::zeroed(gpu, "filled heights", width, height)?;
        let parameters = FillParameters {
            width: width as u32,
            height: height as u32,
            ..FillParameters::default()
        };
        let uniform = gpu.uniform(&parameters);
        let forward = gpu.bind(&fill_kernel, &[&uniform, &level.heights.buffer, &scratch.buffer, &flags.buffer]);
        let backward = gpu.bind(&fill_kernel, &[&uniform, &scratch.buffer, &level.heights.buffer, &flags.buffer]);
        // An even number of passes leaves the result in the level's plane.
        for pass in 0..flow::FILL_PASSES {
            let group = if pass % 2 == 0 { &forward } else { &backward };
            batch.dispatch_plane(&fill_kernel, group, width, height)?;
        }

        let parent = levels.last_mut().expect("a finer level is left");
        let detail = parent.detail.as_ref().expect("finer levels have detail");
        let fine = GpuPlane::zeroed(gpu, "refilled heights", detail.width, detail.height)?;
        let parameters = FillParameters {
            width: fine.width as u32,
            height: fine.height as u32,
            coarse_width: width as u32,
            coarse_height: height as u32,
        };
        let uniform = gpu.uniform(&parameters);
        let group = gpu.bind(&reconstruct_kernel, &[&uniform, &level.heights.buffer, &flags.buffer, &detail.buffer, &fine.buffer]);
        batch.dispatch_plane(&reconstruct_kernel, &group, fine.width, fine.height)?;
        parent.heights = fine;
    }
    let finest = levels.pop().expect("the finest level is left");
    Ok(finest.heights)
}

/// `FlowSimulation` on `gpu`.
pub fn simulate(gpu: &GpuDevice, heights: &Map, precipitation: Option<&Map>, settings: &FlowSettings) -> Result<Map, GpuError> {
    let resolution = heights.resolution();
    let width = resolution.x;
    let height = resolution.y;
    let mut batch = gpu.batch();
    let source = GpuPlane::upload(gpu, "flow heights", heights.channel(0), width, height)?;
    let terrain = fill_pyramid(gpu, &mut batch, source)?;

    let precipitation_data = precipitation.map(|map| map.channel(0));
    let (particles, directions) = flow::prepare_particles(resolution, precipitation_data, settings.seed);
    let particle_buffer = gpu.upload("flow particles", &particles)?;
    let direction_buffer = gpu.upload("flow directions", &directions)?;
    let counts = gpu.zeroed_words("flow visits", width * height)?;
    let flow_map = GpuPlane::zeroed(gpu, "flow", width, height)?;

    let probe = gpu.kernel(&FLOW_SHADER, "probe")?;
    let chain = gpu.kernel(&FLOW_SHADER, "chain")?;
    let trace = gpu.kernel(&FLOW_SHADER, "trace")?;
    let accumulate = gpu.kernel(&FLOW_SHADER, "accumulate")?;
    let particle_count = particles.len() as i32;
    let batch_size = (particle_count >> 5) + i32::from(particle_count & 31 != 0);
    // No launch is longer than a batch.
    let launch_room = batch_size.max(1) as usize;
    let probes = gpu.zeroed("flow probes", launch_room as u64 * 16)?;
    let first_uphill = gpu.zeroed_words("flow first tests", launch_room)?;
    let long_share = particle_count.min(flow::LONG_TRACE_PARTICLES) / flow::BATCHES;
    let pixel_count = (height as i64 * width as i64) as f32;
    let raise = 1e-5 * settings.flow_volume;
    let base = FlowParameters {
        width: width as u32,
        height: height as u32,
        seed: settings.seed as u32,
        raise,
        ..FlowParameters::default()
    };

    let launch = |batch: &mut Batch, start: u32, count: i32, steps: u32| -> Result<(), GpuError> {
        if count <= 0 {
            return Ok(());
        }
        let parameters = FlowParameters {
            start,
            count: count as u32,
            steps,
            uphill_limit: crate::x86::maxss(steps as f32 * 0.08, 4.0),
            last_step: steps.wrapping_sub(1) as f32,
            ..base
        };
        let uniform = gpu.uniform(&parameters);
        let probe_group = gpu.bind(&probe, &[&uniform, &terrain.buffer, &particle_buffer, &direction_buffer, &counts, &probes]);
        batch.dispatch_linear(&probe, &probe_group, count as usize)?;
        let chain_group = gpu.bind(&chain, &[&uniform, &probes, &first_uphill]);
        batch.dispatch(&chain, &chain_group, [1, 1, 1])?;
        let trace_group = gpu.bind(&trace, &[&uniform, &terrain.buffer, &particle_buffer, &direction_buffer, &counts, &first_uphill]);
        batch.dispatch_linear(&trace, &trace_group, count as usize)
    };

    let mut start = 0i32;
    loop {
        let remaining = particle_count - start;
        if remaining <= 0 {
            break;
        }
        let this_batch = remaining.min(batch_size);
        let long_steps = (width as i32 as f32).mul_add(settings.flow_length, 2.0);
        let short_cap = crate::x86::minss(long_steps, 16.0);
        let short_count = (this_batch - long_share).max(0);
        launch(&mut batch, start as u32, short_count, flow::SHORT_STEPS)?;
        let long_count = this_batch.min(long_share);
        let long_step_count = long_steps as i64 as u32;
        launch(&mut batch, (start + short_count) as u32, long_count, long_step_count)?;

        let weight = flow::batch_weight(particle_count, long_share, short_count, short_cap, long_steps, pixel_count);
        let parameters = FlowParameters { weight, ..base };
        let uniform = gpu.uniform(&parameters);
        let group = gpu.bind(&accumulate, &[&uniform, &counts, &terrain.buffer, &flow_map.buffer]);
        batch.dispatch_plane(&accumulate, &group, width, height)?;
        start += batch_size;
    }
    batch.finish()?;

    let data = flow_map.read(gpu)?;
    Ok(Map::from_data(resolution, data))
}
