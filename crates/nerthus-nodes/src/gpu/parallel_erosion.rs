//! `ParallelErosionSimulation` on a GPU: [`crate::parallel_erosion`]'s
//! pyramid, droplet rounds and shaping as kernels. The plan of levels and
//! rounds is worked out on the CPU, and the CPU checks every few steps
//! whether any droplet still moves.

use bytemuck::{Pod, Zeroable};
use nerthus_compute::wgpu;
use nerthus_compute::{Batch, Binding, Entry, GpuDevice, GpuError, Shader};
use nerthus_engine::Map;

use crate::parallel_erosion::{self, ErosionMaps, ErosionSettings, Prepared};

const SHADER: Shader = Shader {
    name: "parallel-erosion",
    source: include_str!("shaders/parallel-erosion.wgsl"),
    bindings: &[
        Binding::Uniform,
        Binding::ReadWrite,
        Binding::Read,
        Binding::Read,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::Read,
        Binding::ReadWrite,
        Binding::Read,
        Binding::Read,
        Binding::Read,
        Binding::Read,
        Binding::Read,
        Binding::ReadWrite,
    ],
    entries: &[
        Entry { name: "spawn", bindings: &[0, 2, 4] },
        Entry { name: "advance", bindings: &[0, 1, 3, 4, 5, 9] },
        Entry { name: "settle", bindings: &[0, 4, 6, 7, 8, 9] },
        Entry { name: "apply", bindings: &[0, 1, 6, 7, 8, 9, 10, 11] },
        Entry { name: "reshape", bindings: &[0, 1, 11, 12] },
        Entry { name: "wet", bindings: &[0, 5, 13] },
        Entry { name: "reshape_wet", bindings: &[0, 1, 11, 12, 13, 14] },
        Entry { name: "apply_shaping", bindings: &[0, 1, 12] },
        Entry { name: "downsample", bindings: &[0, 15, 16] },
        Entry { name: "sharpen", bindings: &[0, 15, 16] },
        Entry { name: "detail_of", bindings: &[0, 15, 17, 22] },
        Entry { name: "refine", bindings: &[0, 1, 10, 11, 17, 18, 19, 22] },
        Entry { name: "refine_wetness", bindings: &[0, 13, 14, 20, 21] },
    ],
};

/// Bytes of the kernel's `Droplet`.
const DROPLET_BYTES: u64 = 240;
/// Cells a droplet's path holds per step.
const PATH_CAPACITY: usize = 12;
/// Steps run between checks that droplets still move.
const STEPS_PER_CHECK: usize = 8;

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct Parameters {
    width: u32,
    height: u32,
    coarse_width: u32,
    coarse_height: u32,
    first_droplet: u32,
    droplet_count: u32,
    seed: i32,
    has_precipitation: u32,
    cell_size: f32,
    downcutting: f32,
    settings_downcutting: f32,
    downcutting_type: f32,
    suspended_tangent: f32,
    suspended_load_amount: f32,
    bed_tangent: f32,
    bed_load_amount: f32,
    coarse_tangent: f32,
    coarse_sediments_amount: f32,
    extra_deposition_boost: f32,
    boost: f32,
    no_erosion: u32,
    shape: f32,
    shape_sharpness: f32,
    relaxation: f32,
    refine_cell_size: f32,
    padding: [u32; 3],
}

/// A level's precipitation mask and the wetness its shaping spreads.
struct Precipitation {
    mask: wgpu::Buffer,
    wetness: wgpu::Buffer,
    spread: wgpu::Buffer,
}

/// One level of the pyramid on the GPU.
struct Level {
    width: usize,
    height: usize,
    heights: wgpu::Buffer,
    flow: wgpu::Buffer,
    deposition: wgpu::Buffer,
    wear: wgpu::Buffer,
    /// This level less the next coarser one (none on the coarsest).
    detail: Option<wgpu::Buffer>,
    precipitation: Option<Precipitation>,
}

impl Level {
    fn new(gpu: &GpuDevice, width: usize, height: usize, heights: wgpu::Buffer, mask: Option<wgpu::Buffer>) -> Result<Level, GpuError> {
        let cells = width * height;
        let precipitation = match mask {
            Some(mask) => Some(Precipitation {
                mask,
                wetness: gpu.zeroed_words("wetness", cells)?,
                spread: gpu.zeroed_words("spread wetness", cells)?,
            }),
            None => None,
        };
        Ok(Level {
            width,
            height,
            heights,
            flow: gpu.zeroed_words("erosion flow", cells)?,
            deposition: gpu.zeroed_words("deposition", cells)?,
            wear: gpu.zeroed_words("wear", cells)?,
            detail: None,
            precipitation,
        })
    }
}

/// The run-wide values and the kernels' shared parameters.
struct Run<'a> {
    gpu: &'a GpuDevice,
    prepared: Prepared,
    terrain: (f32, f32),
    has_precipitation: bool,
    directions: wgpu::Buffer,
}

impl Run<'_> {
    fn parameters(&self, level: &Level) -> Parameters {
        let settings = &self.prepared.settings;
        Parameters {
            width: level.width as u32,
            height: level.height as u32,
            seed: settings.seed,
            has_precipitation: u32::from(self.has_precipitation),
            settings_downcutting: settings.downcutting,
            downcutting_type: settings.downcutting_type,
            suspended_tangent: self.prepared.suspended_tangent,
            suspended_load_amount: settings.suspended_load_amount,
            bed_tangent: self.prepared.bed_tangent,
            bed_load_amount: settings.bed_load_amount,
            coarse_tangent: self.prepared.coarse_tangent,
            coarse_sediments_amount: settings.coarse_sediments_amount,
            extra_deposition_boost: settings.extra_deposition_boost,
            no_erosion: u32::from(settings.no_erosion),
            shape_sharpness: settings.shape_sharpness,
            ..Parameters::default()
        }
    }

    /// The next coarser level: heights (and mask, sharpened back) filtered
    /// down to half size, and the detail `fine` loses to it.
    fn coarsen(&self, batch: &mut Batch, fine: &mut Level) -> Result<Level, GpuError> {
        let gpu = self.gpu;
        let width = fine.width >> 1;
        let height = fine.height >> 1;
        let base = Parameters {
            coarse_width: width as u32,
            coarse_height: height as u32,
            ..self.parameters(fine)
        };
        let uniform = gpu.uniform(&base);
        let downsample = gpu.kernel(&SHADER, "downsample")?;
        let heights = gpu.zeroed_words("coarse heights", width * height)?;
        let group = gpu.bind(&downsample, &[&uniform, &fine.heights, &heights]);
        batch.dispatch_plane(&downsample, &group, width, height)?;

        let mask = match &fine.precipitation {
            Some(precipitation) => {
                let halved = gpu.zeroed_words("halved mask", width * height)?;
                let group = gpu.bind(&downsample, &[&uniform, &precipitation.mask, &halved]);
                batch.dispatch_plane(&downsample, &group, width, height)?;
                let sharpen = gpu.kernel(&SHADER, "sharpen")?;
                let sharpened = gpu.zeroed_words("coarse mask", width * height)?;
                let coarse_size = Parameters {
                    width: width as u32,
                    height: height as u32,
                    ..base
                };
                let coarse_uniform = gpu.uniform(&coarse_size);
                let group = gpu.bind(&sharpen, &[&coarse_uniform, &halved, &sharpened]);
                batch.dispatch_plane(&sharpen, &group, width, height)?;
                Some(sharpened)
            }
            None => None,
        };

        let detail_kernel = gpu.kernel(&SHADER, "detail_of")?;
        let detail = gpu.zeroed_words("erosion detail", fine.width * fine.height)?;
        let group = gpu.bind(&detail_kernel, &[&uniform, &fine.heights, &heights, &detail]);
        batch.dispatch_plane(&detail_kernel, &group, fine.width, fine.height)?;
        fine.detail = Some(detail);
        Level::new(gpu, width, height, heights, mask)
    }

    /// Every round of droplets on one level, `index` levels finer than the
    /// coarsest.
    fn erode_level(&self, batch: &mut Batch, level: &mut Level, index: i32) -> Result<(), GpuError> {
        let gpu = self.gpu;
        let prepared = &self.prepared;
        let settings = &prepared.settings;
        let plan = parallel_erosion::plan_level(index, prepared.octaves, prepared.detail_octave, settings, level.width, level.height);
        let (terrain_width, terrain_height) = self.terrain;
        let cells = level.width * level.height;
        let count = plan.droplets as usize;
        let base = Parameters {
            droplet_count: plan.droplets,
            cell_size: (terrain_width / terrain_height) / level.width as f32,
            downcutting: plan.downcutting,
            boost: parallel_erosion::deposition_boost(index, self.has_precipitation, settings),
            shape: plan.shape,
            relaxation: plan.relaxation,
            ..self.parameters(level)
        };

        let droplets = gpu.zeroed("droplets", count as u64 * DROPLET_BYTES)?;
        let touched = gpu.zeroed_words("touched cells", count * PATH_CAPACITY)?;
        let counters = gpu.zeroed_words("erosion counters", 4)?;
        let sums = gpu.zeroed_words("height sums", cells)?;
        let weights = gpu.zeroed_words("height weights", cells)?;
        let shaping = gpu.zeroed_words("shaping", cells)?;
        let spawn = gpu.kernel(&SHADER, "spawn")?;
        let advance = gpu.kernel(&SHADER, "advance")?;
        let settle = gpu.kernel(&SHADER, "settle")?;
        let apply = gpu.kernel(&SHADER, "apply")?;

        let mut first_droplet = 0u32;
        for _ in 0..plan.rounds {
            let round = Parameters { first_droplet, ..base };
            let uniform = gpu.uniform(&round);
            if count > 0 {
                let mask = level.precipitation.as_ref().map_or(gpu.spare(), |precipitation| &precipitation.mask);
                let spawn_group = gpu.bind(&spawn, &[&uniform, mask, &droplets]);
                batch.dispatch_linear(&spawn, &spawn_group, count)?;
                let advance_group = gpu.bind(&advance, &[&uniform, &level.heights, &self.directions, &droplets, &level.flow, &counters]);
                let settle_group = gpu.bind(&settle, &[&uniform, &droplets, &sums, &weights, &touched, &counters]);
                let apply_group = gpu.bind(&apply, &[&uniform, &level.heights, &sums, &weights, &touched, &counters, &level.deposition, &level.wear]);
                loop {
                    for _ in 0..STEPS_PER_CHECK {
                        batch.clear(&counters);
                        batch.dispatch_linear(&advance, &advance_group, count)?;
                        batch.dispatch_linear(&settle, &settle_group, count)?;
                        batch.dispatch_linear(&apply, &apply_group, count * PATH_CAPACITY)?;
                    }
                    batch.submit()?;
                    let moving: Vec<u32> = gpu.read(&counters, 1)?;
                    if moving[0] == 0 {
                        break;
                    }
                }
            }
            if settings.shape > 0.0 {
                self.reshape(batch, level, &uniform, &shaping)?;
            }
            first_droplet = first_droplet.wrapping_add(plan.droplets);
        }
        Ok(())
    }

    /// One shaping pass over the level (with a precipitation mask, only
    /// where water has run).
    fn reshape(&self, batch: &mut Batch, level: &mut Level, uniform: &wgpu::Buffer, shaping: &wgpu::Buffer) -> Result<(), GpuError> {
        let gpu = self.gpu;
        let (width, height) = (level.width, level.height);
        match level.precipitation.as_mut() {
            Some(precipitation) => {
                let wet = gpu.kernel(&SHADER, "wet")?;
                let group = gpu.bind(&wet, &[uniform, &level.flow, &precipitation.wetness]);
                batch.dispatch_plane(&wet, &group, width, height)?;
                let reshape = gpu.kernel(&SHADER, "reshape_wet")?;
                let buffers = [uniform, &level.heights, &level.wear, shaping, &precipitation.wetness, &precipitation.spread];
                let group = gpu.bind(&reshape, &buffers);
                batch.dispatch_plane(&reshape, &group, width, height)?;
                std::mem::swap(&mut precipitation.wetness, &mut precipitation.spread);
            }
            None => {
                let reshape = gpu.kernel(&SHADER, "reshape")?;
                let group = gpu.bind(&reshape, &[uniform, &level.heights, &level.wear, shaping]);
                batch.dispatch_plane(&reshape, &group, width, height)?;
            }
        }
        let apply = gpu.kernel(&SHADER, "apply_shaping")?;
        let group = gpu.bind(&apply, &[uniform, &level.heights, shaping]);
        batch.dispatch_plane(&apply, &group, width, height)
    }

    /// `fine` takes back the eroded surface of `coarse`.
    fn refine(&self, batch: &mut Batch, fine: &mut Level, coarse: &Level) -> Result<(), GpuError> {
        let gpu = self.gpu;
        let Some(detail) = &fine.detail else {
            return Ok(());
        };
        let (terrain_width, terrain_height) = self.terrain;
        let parameters = Parameters {
            coarse_width: coarse.width as u32,
            coarse_height: coarse.height as u32,
            refine_cell_size: (terrain_width / coarse.width as f32) / terrain_height,
            ..self.parameters(fine)
        };
        let uniform = gpu.uniform(&parameters);
        let refine = gpu.kernel(&SHADER, "refine")?;
        let buffers = [&uniform, &fine.heights, &fine.deposition, &fine.wear, &coarse.heights, &coarse.deposition, &coarse.wear, detail];
        let group = gpu.bind(&refine, &buffers);
        batch.dispatch_plane(&refine, &group, fine.width, fine.height)?;
        if let (Some(fine_wet), Some(coarse_wet)) = (&fine.precipitation, &coarse.precipitation) {
            let kernel = gpu.kernel(&SHADER, "refine_wetness")?;
            let buffers = [&uniform, &fine_wet.wetness, &fine_wet.spread, &coarse_wet.wetness, &coarse_wet.spread];
            let group = gpu.bind(&kernel, &buffers);
            batch.dispatch_plane(&kernel, &group, fine.width, fine.height)?;
        }
        Ok(())
    }
}

/// `ParallelErosion_Erode` on `gpu`.
pub fn erode(gpu: &GpuDevice, heightmap: &Map, precipitation: Option<&Map>, settings: &ErosionSettings, terrain: (f32, f32)) -> Result<ErosionMaps, GpuError> {
    let resolution = heightmap.resolution();
    let prepared = parallel_erosion::prepare(settings, resolution.x, terrain);
    let level_count = prepared.level_count;
    let directions = gpu.upload("erosion directions", &prepared.directions)?;
    let run = Run {
        gpu,
        prepared,
        terrain,
        has_precipitation: precipitation.is_some(),
        directions,
    };

    let mut batch = gpu.batch();
    let heights = gpu.upload("erosion heights", heightmap.data())?;
    let mask = match precipitation {
        Some(map) => Some(gpu.upload("precipitation", map.data())?),
        None => None,
    };
    let finest = Level::new(gpu, resolution.x, resolution.y, heights, mask)?;
    let mut levels = vec![finest];
    for _ in 1..level_count {
        let finer = levels.last_mut().expect("the pyramid has a finest level");
        let coarser = run.coarsen(&mut batch, finer)?;
        levels.push(coarser);
    }

    let mut finished_levels = 0;
    while levels.len() > 1 {
        let mut coarser = levels.pop().expect("more than one level is left");
        run.erode_level(&mut batch, &mut coarser, finished_levels)?;
        let finer = levels.last_mut().expect("a finer level is left");
        run.refine(&mut batch, finer, &coarser)?;
        finished_levels += 1;
    }
    let mut finest = levels.pop().expect("the finest level is left");
    run.erode_level(&mut batch, &mut finest, finished_levels)?;
    batch.finish()?;

    let cells = resolution.x * resolution.y;
    let visits: Vec<u32> = gpu.read(&finest.flow, cells)?;
    let flow: Vec<f32> = visits.iter().map(|&count| crate::x86::minss(1.0, 0.001 * count as f32)).collect();
    Ok(ErosionMaps {
        height: Map::from_data(resolution, gpu.read(&finest.heights, cells)?),
        flow: Map::from_data(resolution, flow),
        deposition: Map::from_data(resolution, gpu.read(&finest.deposition, cells)?),
        wear: Map::from_data(resolution, gpu.read(&finest.wear, cells)?),
    })
}
