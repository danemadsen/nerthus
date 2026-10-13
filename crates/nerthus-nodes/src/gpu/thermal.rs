//! `ThermalSimulation` on a GPU: [`crate::thermal`]'s pyramid, passes and
//! reconstruction as kernels. The pass schedule is worked out on the CPU,
//! as the SYCL host works it out.

use bytemuck::{Pod, Zeroable};
use nerthus_compute::wgpu;
use nerthus_compute::{Batch, Binding, Entry, GpuDevice, GpuError, Shader};
use nerthus_engine::Map;

use crate::gpu::{pyramid, GpuPlane};
use crate::thermal::{self, Constants, PassKind, ThermalMaps, ThermalSettings};

const SHADER: Shader = Shader {
    name: "thermal",
    source: include_str!("shaders/thermal.wgsl"),
    bindings: &[
        Binding::Uniform,
        Binding::Read,
        Binding::Read,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::Read,
        Binding::Read,
        Binding::Read,
        Binding::Read,
        Binding::Read,
        Binding::Read,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
    ],
    entries: &[
        Entry { name: "wear_pass", bindings: &[0, 1, 2, 3, 4, 5] },
        Entry { name: "tilt_wear_pass", bindings: &[0, 1, 2, 6, 4, 5] },
        Entry { name: "drops_pass", bindings: &[0, 1, 3, 4, 7, 8] },
        Entry { name: "tilt_drops_pass", bindings: &[0, 1, 6, 4, 7, 8] },
        Entry { name: "gather_pass", bindings: &[0, 1, 3, 4, 7, 8, 9, 10] },
        Entry { name: "tilt_gather_pass", bindings: &[0, 1, 6, 4, 7, 8, 9, 10] },
        Entry { name: "reconstruct", bindings: &[0, 11, 12, 13, 14, 15, 16, 17, 18] },
    ],
};

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct Parameters {
    width: u32,
    height: u32,
    coarse_width: u32,
    coarse_height: u32,
    cell: f32,
    inverse_cell: f32,
    talus: f32,
    plasticity: f32,
    anisotropy: f32,
    strength: f32,
    sediment_removal: f32,
    threshold: f32,
    threshold_raw: f32,
    step: f32,
    rate: f32,
    area_used: u32,
    removal_used: u32,
    coarse_cell: f32,
    padding: [u32; 2],
}

/// One level of the pyramid on the GPU. The heights and sediment
/// ping-pong between two planes each; the first of each pair started as
/// the level's own.
struct Level {
    width: usize,
    height: usize,
    heights: [GpuPlane; 2],
    sediment: [GpuPlane; 2],
    /// Which plane of each pair holds the level as it stands.
    current: usize,
    wear: GpuPlane,
    /// What this level loses to the next coarser one.
    detail: Option<GpuPlane>,
    area: Option<GpuPlane>,
    removal: Option<GpuPlane>,
}

impl Level {
    fn new(gpu: &GpuDevice, heights: GpuPlane, area: Option<GpuPlane>, removal: Option<GpuPlane>) -> Result<Level, GpuError> {
        let width = heights.width;
        let height = heights.height;
        let next_heights = GpuPlane::zeroed(gpu, "thermal heights", width, height)?;
        let sediment = GpuPlane::zeroed(gpu, "thermal sediment", width, height)?;
        let next_sediment = GpuPlane::zeroed(gpu, "thermal sediment", width, height)?;
        let wear = GpuPlane::zeroed(gpu, "thermal wear", width, height)?;
        Ok(Level {
            width,
            height,
            heights: [heights, next_heights],
            sediment: [sediment, next_sediment],
            current: 0,
            wear,
            detail: None,
            area,
            removal,
        })
    }

    /// Whether a mask counts on this level (one-pixel buffers, the
    /// stand-in for a missing mask, are skipped).
    fn uses(&self, mask: &Option<GpuPlane>) -> bool {
        mask.is_some() && self.width * self.height != 1
    }

    /// The level at half resolution (its masks too), storing the detail
    /// this one loses.
    fn coarser(&mut self, gpu: &GpuDevice, batch: &mut Batch) -> Result<Level, GpuError> {
        let fine = &self.heights[self.current];
        let heights = pyramid::downsample(gpu, batch, fine)?;
        self.detail = Some(pyramid::detail(gpu, batch, fine, &heights)?);
        let area = match &self.area {
            Some(area) => Some(pyramid::downsample(gpu, batch, area)?),
            None => None,
        };
        let removal = match &self.removal {
            Some(removal) => Some(pyramid::downsample(gpu, batch, removal)?),
            None => None,
        };
        Level::new(gpu, heights, area, removal)
    }
}

/// The parameters a level's passes share.
fn level_parameters(level: &Level, constants: &Constants) -> Parameters {
    Parameters {
        width: level.width as u32,
        height: level.height as u32,
        cell: constants.cell,
        inverse_cell: constants.inverse_cell,
        talus: constants.talus,
        plasticity: constants.plasticity,
        anisotropy: constants.anisotropy,
        strength: constants.strength,
        sediment_removal: constants.sediment_removal,
        area_used: u32::from(level.uses(&level.area)),
        removal_used: u32::from(level.uses(&level.removal)),
        ..Parameters::default()
    }
}

/// The three kernels and their bind groups for one kind of pass, one per
/// state of the ping-pong.
struct PassGroups {
    kernels: [std::sync::Arc<nerthus_compute::Kernel>; 3],
    groups: [[wgpu::BindGroup; 3]; 2],
}

/// The scratch planes a level's passes share.
struct Scratch {
    slope: GpuPlane,
    tilt: wgpu::Buffer,
    drops: GpuPlane,
}

fn pass_groups(gpu: &GpuDevice, level: &Level, scratch: &Scratch, uniform: &wgpu::Buffer, anisotropic: bool) -> Result<PassGroups, GpuError> {
    let names = if anisotropic { ["tilt_wear_pass", "tilt_drops_pass", "tilt_gather_pass"] } else { ["wear_pass", "drops_pass", "gather_pass"] };
    let wear_kernel = gpu.kernel(&SHADER, names[0])?;
    let drops_kernel = gpu.kernel(&SHADER, names[1])?;
    let gather_kernel = gpu.kernel(&SHADER, names[2])?;
    let area = level.area.as_ref().map_or(gpu.spare(), |plane| &plane.buffer);
    let removal = level.removal.as_ref().map_or(gpu.spare(), |plane| &plane.buffer);
    let slope_or_tilt = if anisotropic { &scratch.tilt } else { &scratch.slope.buffer };

    let groups = [0usize, 1].map(|state| {
        let heights = &level.heights[state].buffer;
        let sediment = &level.sediment[state].buffer;
        let next_heights = &level.heights[1 - state].buffer;
        let next_sediment = &level.sediment[1 - state].buffer;
        let drops = &scratch.drops.buffer;
        let wear = &level.wear.buffer;
        let wear_group = gpu.bind(&wear_kernel, &[uniform, heights, area, slope_or_tilt, sediment, wear]);
        let drops_group = gpu.bind(&drops_kernel, &[uniform, heights, slope_or_tilt, sediment, drops, next_sediment]);
        let gather_group = gpu.bind(&gather_kernel, &[uniform, heights, slope_or_tilt, sediment, drops, next_sediment, next_heights, removal]);
        [wear_group, drops_group, gather_group]
    });
    Ok(PassGroups { kernels: [wear_kernel, drops_kernel, gather_kernel], groups })
}

/// `Thermal::simulation` over a level; returns how many passes ran.
fn simulate_level(gpu: &GpuDevice, batch: &mut Batch, level: &mut Level, iterations: f32, index: i32, finest: bool, settings: &ThermalSettings) -> Result<usize, GpuError> {
    let constants = Constants::new(level.width, settings);
    let passes = thermal::pass_schedule(iterations, index, finest, &constants);
    if passes.is_empty() {
        return Ok(0);
    }
    let width = level.width;
    let height = level.height;
    let scratch = Scratch {
        slope: GpuPlane::zeroed(gpu, "thermal slope", width, height)?,
        tilt: gpu.zeroed_words("thermal tilt", width * height * 2)?,
        drops: GpuPlane::zeroed(gpu, "thermal drops", width, height)?,
    };

    let base = level_parameters(level, &constants);
    let talus_raw = constants.cell * constants.talus;
    let steep = Parameters {
        threshold: constants.talus,
        threshold_raw: talus_raw,
        step: thermal::ISOTROPIC_STEP,
        rate: thermal::ISOTROPIC_WEAR,
        ..base
    };
    let flat = Parameters { threshold: 0.0, threshold_raw: 0.0, ..steep };
    let tilted = Parameters {
        step: thermal::ANISOTROPIC_STEP,
        rate: thermal::ANISOTROPIC_WEAR,
        ..base
    };
    let steep_uniform = gpu.uniform(&steep);
    let flat_uniform = gpu.uniform(&flat);
    let tilted_uniform = gpu.uniform(&tilted);
    let steep_groups = pass_groups(gpu, level, &scratch, &steep_uniform, false)?;
    let flat_groups = pass_groups(gpu, level, &scratch, &flat_uniform, false)?;
    let tilted_groups = pass_groups(gpu, level, &scratch, &tilted_uniform, true)?;

    let mut state = level.current;
    for kind in &passes {
        let chosen = match *kind {
            PassKind::Anisotropic => &tilted_groups,
            PassKind::Isotropic { threshold, threshold_raw } if threshold == steep.threshold && threshold_raw == steep.threshold_raw => &steep_groups,
            PassKind::Isotropic { .. } => &flat_groups,
        };
        for (kernel, group) in chosen.kernels.iter().zip(&chosen.groups[state]) {
            batch.dispatch_plane(kernel, group, width, height)?;
        }
        state = 1 - state;
    }
    level.current = state;
    Ok(passes.len())
}

/// The finer level rebuilt from the simulated coarse one.
fn reconstruct(gpu: &GpuDevice, batch: &mut Batch, coarse: &Level, fine: &Level, settings: &ThermalSettings) -> Result<(), GpuError> {
    let Some(detail) = &fine.detail else {
        return Ok(());
    };
    let columns = coarse.width as i32 as f32;
    let coarse_cell = settings.heightmap_size / (columns * settings.heightmap_height);
    let removal_used = fine.uses(&fine.removal);
    let removal_detail = match (&fine.removal, &coarse.removal) {
        (Some(fine_removal), Some(coarse_removal)) if removal_used => Some(pyramid::detail(gpu, batch, fine_removal, coarse_removal)?),
        _ => None,
    };

    let kernel = gpu.kernel(&SHADER, "reconstruct")?;
    let parameters = Parameters {
        width: fine.width as u32,
        height: fine.height as u32,
        coarse_width: coarse.width as u32,
        coarse_height: coarse.height as u32,
        sediment_removal: settings.sediment_removal,
        removal_used: u32::from(removal_detail.is_some()),
        coarse_cell,
        ..Parameters::default()
    };
    let uniform = gpu.uniform(&parameters);
    let removal_change = removal_detail.as_ref().map_or(gpu.spare(), |plane| &plane.buffer);
    let buffers = [
        &uniform,
        &coarse.heights[coarse.current].buffer,
        &coarse.wear.buffer,
        &coarse.sediment[coarse.current].buffer,
        &detail.buffer,
        removal_change,
        &fine.heights[fine.current].buffer,
        &fine.sediment[fine.current].buffer,
        &fine.wear.buffer,
    ];
    let group = gpu.bind(&kernel, &buffers);
    batch.dispatch_plane(&kernel, &group, fine.width, fine.height)
}

/// `ThermalSimulation` on `gpu`.
pub fn simulate(gpu: &GpuDevice, heights: &Map, area: Option<&Map>, removal: Option<&Map>, settings: &ThermalSettings) -> Result<ThermalMaps, GpuError> {
    let resolution = heights.resolution();
    let width = resolution.x;
    let height = resolution.y;
    let plan = thermal::pyramid_plan(width, settings);

    let upload = |label: &str, map: &Map| GpuPlane::upload(gpu, label, map.channel(0), width, height);
    let finest_heights = upload("thermal heights", heights)?;
    let finest_area = match area {
        Some(map) => Some(upload("thermal area", map)?),
        None => None,
    };
    let finest_removal = match removal {
        Some(map) => Some(upload("thermal removal", map)?),
        None => None,
    };
    let mut batch = gpu.batch();
    let finest = Level::new(gpu, finest_heights, finest_area, finest_removal)?;
    let mut pyramid = vec![finest];
    while (pyramid.len() as i32 as f32) < plan.level_count {
        let coarse = pyramid.last_mut().expect("the pyramid has a level").coarser(gpu, &mut batch)?;
        pyramid.push(coarse);
    }

    let mut index = 0;
    while pyramid.len() > 1 {
        let mut level = pyramid.pop().expect("more than one level is left");
        let passes = thermal::pass_count(index, plan.fraction, plan.offset, plan.budget);
        simulate_level(gpu, &mut batch, &mut level, passes, index, false, settings)?;
        let parent = pyramid.last().expect("a finer level is left");
        reconstruct(gpu, &mut batch, &level, parent, settings)?;
        index += 1;
    }
    let mut finest = pyramid.pop().expect("the finest level is left");
    let passes = thermal::pass_count(index, plan.fraction, plan.offset, plan.budget);
    simulate_level(gpu, &mut batch, &mut finest, passes, index, true, settings)?;
    batch.finish()?;

    // The planes that started as the caller's: after an odd number of
    // passes they hold the second-to-last, as on the CPU.
    let heights = finest.heights[0].read(gpu)?;
    let deposition = finest.sediment[0].read(gpu)?;
    let wear = finest.wear.read(gpu)?;
    Ok(ThermalMaps {
        heights: Map::from_data(resolution, heights),
        deposition: Map::from_data(resolution, deposition),
        wear: Map::from_data(resolution, wear),
    })
}
