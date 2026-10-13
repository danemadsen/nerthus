//! The colour transport (`color_transport::ColorTransport::process` in
//! the SYCL kernels, exported as `ColorTransportSimulation`): the flow
//! simulation's particles, each carrying the colour it picked up and
//! picking it up again every so often, deposit colour where they pass;
//! every batch blends the texture towards the colour deposited on it.

use nerthus_engine::{Map, Resolution};
use rayon::prelude::*;

use crate::flow::{self, Plane, Sample, LONG_TRACE_PARTICLES, UNIT_SCALE, XORSHIFT_MULTIPLIER, ZERO_SEED};
use crate::{gpu, sycl, x86};

/// The parameters of `ColorTransportSimulation` (`ColorTransport.Parameters`).
pub struct ColourTransportSettings {
    pub transport_distance: f32,
    pub sediment_density: f32,
    pub blend: f32,
    pub flow_volume: f32,
    pub colour_hold: f32,
    pub laminar_flow: bool,
    pub seed: i32,
}

/// Particles traced in one launch of `ColorTransport::transport`.
const LAUNCH_CHUNK: usize = 4096;

/// One launch of the transport kernel.
struct Launch<'a> {
    terrain: &'a Plane,
    texture: &'a [Plane; 3],
    lengths: Option<&'a Plane>,
    particles: &'a [[f32; 2]],
    directions: &'a [[f32; 2]; 32],
    start: u32,
    seed: u32,
    steps: f32,
    /// Steps between colour pickups, scaled by a random fraction.
    hold: f32,
    laminar: bool,
}

/// A deposit: the pixel and the colour left there.
type Deposit = (usize, [f32; 3]);

fn texture_colour(texture: &[Plane; 3], pixel: usize) -> [f32; 3] {
    [texture[0].data[pixel], texture[1].data[pixel], texture[2].data[pixel]]
}

/// The pixel under `(x, y)`, as the kernel indexes its maps.
fn pixel_at(x: f32, y: f32, width: usize) -> usize {
    let column = x86::cvttss2si(x) as u32 as usize;
    let row = x86::cvttss2si(y) as u32 as usize;
    row * width + column
}

/// One particle: rolls like a flow particle and deposits its colour on
/// every pixel it visits.
fn trace(launch: &Launch, offset: u32, deposits: &mut Vec<Deposit>) {
    let terrain = launch.terrain;
    let width = terrain.width;
    let index = offset.wrapping_add(launch.start);
    let [mut x, mut y] = launch.particles[index as usize];

    let right = terrain.width as f32 + -1.0;
    let bottom = terrain.height as f32 + -1.0;
    let inside = !(x < 0.0) && x < right && !(y < 0.0) && y < bottom;
    if !inside {
        return;
    }
    let start = flow::sample(terrain, x, y);
    let start_length = start.gradient_x * start.gradient_x;
    let start_length = start.gradient_y.mul_add(start.gradient_y, start_length);
    if !(start_length > 0.0) {
        return;
    }
    let scale = flow::inverse_length(start_length);
    let mut direction_x = -start.gradient_x * scale;
    let mut direction_y = -start.gradient_y * scale;

    let start_pixel = pixel_at(x, y, width);
    let uphill_limit = x86::cvttss2si(launch.steps * 0.2).max(4) as u32;
    let seeded = index.wrapping_add(launch.seed);
    let mut state = if seeded != 0 { seeded } else { ZERO_SEED };
    let length = match launch.lengths {
        Some(lengths) => lengths.data[start_pixel],
        None => 1.0,
    };
    state = flow::xorshift(flow::xorshift(state));
    let mut pickup = (launch.hold * UNIT_SCALE) * flow::random_bits(state);
    let mut colour = texture_colour(launch.texture, start_pixel);
    let last_step = launch.steps.mul_add(length, -1.0);

    let mut uphill = 0u32;
    let mut steps_taken = 0.0f32;
    let mut previous_height = start.height;
    loop {
        let pixel = pixel_at(x, y, width);
        deposits.push((pixel, colour));
        if steps_taken >= pickup {
            colour = texture_colour(launch.texture, pixel);
            state = flow::xorshift(state);
            let fraction = flow::random_bits(state) * UNIT_SCALE;
            pickup = fraction.mul_add(launch.hold, pickup);
        }

        x += direction_x;
        if x < 0.0 || !(x < right) {
            return;
        }
        y += direction_y;
        if y < 0.0 || !(y < bottom) {
            return;
        }
        let here: Sample = flow::sample(terrain, x, y);
        let length_squared = here.gradient_x * here.gradient_x;
        let length_squared = here.gradient_y.mul_add(here.gradient_y, length_squared);
        state = flow::xorshift(state);
        if length_squared > 0.0 {
            let (nudge_x, nudge_y) = if launch.laminar {
                (0.0, 0.0)
            } else {
                let pick = (state.wrapping_mul(XORSHIFT_MULTIPLIER).wrapping_sub(1) & 31) as usize;
                let [direction_x, direction_y] = launch.directions[pick];
                (0.7 * direction_x, 0.7 * direction_y)
            };
            let inverse = 1.0 / length_squared.sqrt();
            let pushed_x = direction_x.mul_add(0.9, -(here.gradient_x * inverse));
            let new_x = nudge_x + pushed_x;
            let pushed_y = direction_y.mul_add(0.9, -(here.gradient_y * inverse));
            let new_y = pushed_y + nudge_y;
            let new_length = new_x * new_x;
            let new_length = new_y.mul_add(new_y, new_length);
            let new_scale = flow::inverse_length(new_length);
            direction_x = new_scale * new_x;
            direction_y = new_scale * new_y;
        }

        steps_taken += 1.0;
        if !(here.height < previous_height) && !here.height.is_nan() && !previous_height.is_nan() {
            uphill += 1;
        }
        if steps_taken >= last_step {
            return;
        }
        previous_height = here.height;
        if uphill > uphill_limit {
            return;
        }
    }
}

/// Runs a launch: traces in parallel, deposits in particle order (the
/// colour sums depend on their order).
fn run_launch(launch: &Launch, count: u32, sums: &mut [Plane; 3], counts: &mut Plane) {
    let mut first = 0usize;
    let count = count as usize;
    while first < count {
        let last = (first + LAUNCH_CHUNK).min(count);
        let chunk: Vec<Vec<Deposit>> = (first..last)
            .into_par_iter()
            .map(|offset| {
                let mut deposits = Vec::new();
                trace(launch, offset as u32, &mut deposits);
                deposits
            })
            .collect();
        for deposits in &chunk {
            for &(pixel, colour) in deposits {
                for channel in 0..3 {
                    sums[channel].data[pixel] += colour[channel];
                }
                counts.data[pixel] += 1.0;
            }
        }
        first = last;
    }
}

/// The kernel that blends the texture towards each pixel's deposited
/// colour and raises the terrain by the visits; the vectorised lanes
/// divide through `RCPPS` and one Newton-Raphson step.
fn blend(texture: &mut [Plane; 3], sums: &mut [Plane; 3], counts: &mut Plane, terrain: &mut Plane, weights: [f32; 4]) {
    let [weight, base, blend, raise] = weights;
    let width = counts.width;
    for pixel in 0..counts.data.len() {
        let x = pixel % width;
        let count = counts.data[pixel];
        if !(0.0 >= count) {
            let damping = count.mul_add(weight, base);
            let inverse = if flow::vector_lane(x, width) {
                let reciprocal = x86::rcp(damping);
                let error = damping.mul_add(reciprocal, -1.0);
                (-error).mul_add(reciprocal, reciprocal)
            } else {
                1.0 / damping
            };
            for channel in 0..3 {
                let current = texture[channel].data[pixel];
                let deposited = weight.mul_add(sums[channel].data[pixel], current * base);
                let difference = (-deposited).mul_add(inverse, current);
                texture[channel].data[pixel] = (-difference).mul_add(blend, current);
            }
        }
        terrain.data[pixel] = count.mul_add(raise, terrain.data[pixel]);
        counts.data[pixel] = 0.0;
        for sum in sums.iter_mut() {
            sum.data[pixel] = 0.0;
        }
    }
}

/// `ColorTransport::transport`'s pickup spacing: steps between colour
/// pickups for colour hold `hold` on a map `rows` high.
pub(crate) fn pickup_spacing(hold: f32, rows: usize) -> f32 {
    let growth = ((hold * 5.0) as f64).exp() as f32 + -1.0;
    let rows = rows as f32;
    let scale = rows * 0.006_783_655;
    let excess = hold + -0.95;
    let extra = ((excess * excess) * 400.0) * rows;
    let extra = if hold < 0.95 { 0.0 } else { extra };
    growth.mul_add(scale, extra)
}

/// `ColorTransportSimulation`: `texture` (one or three channels) carried
/// over `heights` by flow; `precipitation` decides where particles spawn
/// and `lengths` scales how far each travels. Runs on the GPU when one is
/// active (see [`sycl`]).
pub fn simulate(heights: &Map, texture: &Map, precipitation: Option<&Map>, lengths: Option<&Map>, settings: &ColourTransportSettings) -> Map {
    let gpu = |device: &nerthus_compute::GpuDevice| gpu::colour_transport::simulate(device, heights, texture, precipitation, lengths, settings);
    let cpu = || simulate_cpu(heights, texture, precipitation, lengths, settings);
    sycl::run("ColorTransportSimulation", heights.width(), sycl::PREFERRED, gpu, cpu)
}

/// [`simulate`] on the CPU.
pub fn simulate_cpu(heights: &Map, texture: &Map, precipitation: Option<&Map>, lengths: Option<&Map>, settings: &ColourTransportSettings) -> Map {
    let resolution: Resolution = heights.resolution();
    let width = resolution.x;
    let height = resolution.y;
    let source = Plane { width, height, data: heights.channel(0).to_vec() };
    let mut terrain = flow::fill_pyramid(source);

    let channel_count = texture.channel_count();
    let mut colours = [0, 1, 2].map(|channel| Plane { width, height, data: texture.rgb_channel(channel).to_vec() });
    let length_plane = lengths.map(|map| Plane { width, height, data: map.channel(0).to_vec() });

    let precipitation_data = precipitation.map(|map| map.channel(0));
    let (particles, directions) = flow::prepare_particles(resolution, precipitation_data, settings.seed);

    let mut sums = [0, 1, 2].map(|_| Plane::zeros(width, height));
    let mut counts = Plane::zeros(width, height);

    let particle_count = particles.len() as i32;
    let batch = (particle_count >> 5) + i32::from(particle_count & 31 != 0);
    let long_share = particle_count.min(LONG_TRACE_PARTICLES) / flow::BATCHES;
    let pixel_count = (height as i64 * width as i64) as f32;
    let base = 16.0 / (settings.sediment_density + 0.1) + -14.0;
    let raise = 1e-5 * settings.flow_volume;
    let hold = pickup_spacing(settings.colour_hold, height);

    let mut start = 0i32;
    loop {
        let remaining = particle_count - start;
        if remaining <= 0 {
            break;
        }
        let this_batch = remaining.min(batch);
        let long_steps = (width as i32 as f32).mul_add(settings.transport_distance, 2.0);
        let short_cap = x86::minss(long_steps, 16.0);
        let short_count = (this_batch - long_share).max(0);
        if short_count > 0 {
            let launch = Launch {
                terrain: &terrain,
                texture: &colours,
                lengths: length_plane.as_ref(),
                particles: &particles,
                directions: &directions,
                start: start as u32,
                seed: settings.seed as u32,
                steps: short_cap,
                hold,
                laminar: settings.laminar_flow,
            };
            run_launch(&launch, short_count as u32, &mut sums, &mut counts);
        }
        let long_count = this_batch.min(long_share);
        let launch = Launch {
            terrain: &terrain,
            texture: &colours,
            lengths: length_plane.as_ref(),
            particles: &particles,
            directions: &directions,
            start: (start + short_count) as u32,
            seed: settings.seed as u32,
            steps: long_steps,
            hold,
            laminar: settings.laminar_flow,
        };
        run_launch(&launch, long_count as u32, &mut sums, &mut counts);

        let weight = flow::batch_weight(particle_count, long_share, short_count, short_cap, long_steps, pixel_count);
        blend(&mut colours, &mut sums, &mut counts, &mut terrain, [weight, base, settings.blend, raise]);
        start += batch;
    }

    let channels: Vec<Vec<f32>> = colours.into_iter().take(channel_count.min(3)).map(|plane| plane.data).collect();
    Map::from_channels(resolution, channels)
}
