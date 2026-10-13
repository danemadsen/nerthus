//! `ColorTransportSimulation` on a GPU: [`crate::colour_transport`]'s pit
//! filling, particle launches and blends as kernels, the particles seeded
//! and shuffled on the CPU as for [`crate::gpu::flow`].

use bytemuck::{Pod, Zeroable};
use nerthus_compute::{Batch, Binding, Entry, GpuDevice, GpuError, Shader};
use nerthus_engine::Map;

use crate::colour_transport::{self, ColourTransportSettings};
use crate::flow;
use crate::gpu::{self, GpuPlane};

const SHADER: Shader = Shader {
    name: "colour-transport",
    source: concat!(include_str!("shaders/colour-transport.wgsl"), include_str!("shaders/terrain-sample.wgsl")),
    bindings: &[
        Binding::Uniform,
        Binding::Read,
        Binding::Read,
        Binding::Read,
        Binding::Read,
        Binding::Read,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
    ],
    entries: &[
        Entry { name: "transport", bindings: &[0, 1, 2, 3, 4, 5, 6, 7] },
        Entry { name: "blend", bindings: &[0, 6, 7, 8, 9] },
    ],
};

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct Parameters {
    width: u32,
    height: u32,
    start: u32,
    count: u32,
    seed: u32,
    laminar: u32,
    steps: f32,
    hold: f32,
    use_lengths: u32,
    weight: f32,
    base: f32,
    blend: f32,
    raise: f32,
    padding: [u32; 3],
}

/// `ColorTransportSimulation` on `gpu`: `texture` (one or three channels)
/// carried over `heights` by flow.
pub fn simulate(gpu: &GpuDevice, heights: &Map, texture: &Map, precipitation: Option<&Map>, lengths: Option<&Map>, settings: &ColourTransportSettings) -> Result<Map, GpuError> {
    let resolution = heights.resolution();
    let width = resolution.x;
    let height = resolution.y;
    let mut batch = gpu.batch();
    let source = GpuPlane::upload(gpu, "colour transport heights", heights.channel(0), width, height)?;
    let terrain = gpu::flow::fill_pyramid(gpu, &mut batch, source)?;

    // As in the SYCL kernels' GPU buffers, the three planes are separate
    // copies even when the texture has one channel.
    let mut planes = Vec::with_capacity(3 * width * height);
    for channel in 0..3 {
        planes.extend_from_slice(texture.rgb_channel(channel));
    }
    let colours = gpu.upload("colours", &planes)?;
    let length_buffer = match lengths {
        Some(map) => Some(gpu.upload("transport lengths", map.channel(0))?),
        None => None,
    };
    let length_binding = length_buffer.as_ref().unwrap_or(gpu.spare());

    let precipitation_data = precipitation.map(|map| map.channel(0));
    let (particles, directions) = flow::prepare_particles(resolution, precipitation_data, settings.seed);
    let particle_buffer = gpu.upload("transport particles", &particles)?;
    let direction_buffer = gpu.upload("transport directions", &directions)?;
    let sums = gpu.zeroed_words("deposited colour", 3 * width * height)?;
    let counts = gpu.zeroed_words("deposit counts", width * height)?;

    let transport = gpu.kernel(&SHADER, "transport")?;
    let blend = gpu.kernel(&SHADER, "blend")?;
    let particle_count = particles.len() as i32;
    let batch_size = (particle_count >> 5) + i32::from(particle_count & 31 != 0);
    let long_share = particle_count.min(flow::LONG_TRACE_PARTICLES) / flow::BATCHES;
    let pixel_count = (height as i64 * width as i64) as f32;
    let base = Parameters {
        width: width as u32,
        height: height as u32,
        seed: settings.seed as u32,
        laminar: u32::from(settings.laminar_flow),
        hold: colour_transport::pickup_spacing(settings.colour_hold, height),
        use_lengths: u32::from(length_buffer.is_some()),
        base: 16.0 / (settings.sediment_density + 0.1) + -14.0,
        blend: settings.blend,
        raise: 1e-5 * settings.flow_volume,
        ..Parameters::default()
    };

    let launch = |batch: &mut Batch, start: u32, count: i32, steps: f32| -> Result<(), GpuError> {
        if count <= 0 {
            return Ok(());
        }
        let parameters = Parameters { start, count: count as u32, steps, ..base };
        let uniform = gpu.uniform(&parameters);
        let buffers = [&uniform, &terrain.buffer, &colours, length_binding, &particle_buffer, &direction_buffer, &sums, &counts];
        let group = gpu.bind(&transport, &buffers);
        batch.dispatch_linear(&transport, &group, count as usize)
    };

    let mut start = 0i32;
    loop {
        let remaining = particle_count - start;
        if remaining <= 0 {
            break;
        }
        let this_batch = remaining.min(batch_size);
        let long_steps = (width as i32 as f32).mul_add(settings.transport_distance, 2.0);
        let short_cap = crate::x86::minss(long_steps, 16.0);
        let short_count = (this_batch - long_share).max(0);
        launch(&mut batch, start as u32, short_count, short_cap)?;
        let long_count = this_batch.min(long_share);
        launch(&mut batch, (start + short_count) as u32, long_count, long_steps)?;

        let weight = flow::batch_weight(particle_count, long_share, short_count, short_cap, long_steps, pixel_count);
        let parameters = Parameters { weight, ..base };
        let uniform = gpu.uniform(&parameters);
        let group = gpu.bind(&blend, &[&uniform, &sums, &counts, &colours, &terrain.buffer]);
        batch.dispatch_plane(&blend, &group, width, height)?;
        start += batch_size;
    }
    batch.finish()?;

    let blended: Vec<f32> = gpu.read(&colours, 3 * width * height)?;
    let channel_count = texture.channel_count().min(3);
    let channels: Vec<Vec<f32>> = blended.chunks(width * height).take(channel_count).map(<[f32]>::to_vec).collect();
    Ok(Map::from_channels(resolution, channels))
}
