//! `SunLightIntegral` and `Shade` on a GPU: [`crate::lighting`]'s and
//! [`crate::shade`]'s sweeps and per-pixel steps as kernels. The suns,
//! shadow samples and occlusion directions are worked out on the CPU, as
//! the SYCL host works them out, and each becomes a sweep of lines.

use bytemuck::{Pod, Zeroable};
use nerthus_compute::wgpu;
use nerthus_compute::{Batch, Binding, Entry, GpuDevice, GpuError, Shader};
use nerthus_engine::Map;

use crate::lighting::{self, SunlightSettings};
use crate::shade::{self, AmbientOcclusionModel, LightRig, ShadeMaps, Sky};

const SHADER: Shader = Shader {
    name: "lighting",
    source: include_str!("shaders/lighting.wgsl"),
    bindings: &[
        Binding::Uniform,
        Binding::Read,
        Binding::ReadWrite,
        Binding::Read,
        Binding::ReadWrite,
        Binding::Read,
        Binding::ReadWrite,
        Binding::Read,
        Binding::Read,
        Binding::Read,
        Binding::Read,
        Binding::ReadWrite,
        Binding::Read,
        Binding::Read,
    ],
    entries: &[
        Entry { name: "downsample", bindings: &[0, 12, 6] },
        Entry { name: "surface_normals", bindings: &[0, 1, 2] },
        Entry { name: "sweep_light", bindings: &[0, 1, 2, 3, 4] },
        Entry { name: "sweep_shadow", bindings: &[0, 1, 3, 4] },
        Entry { name: "to_values", bindings: &[0, 4, 6] },
        Entry { name: "upsample", bindings: &[0, 5, 6] },
        Entry { name: "direct_light", bindings: &[0, 2, 6] },
        Entry { name: "local_occlusion", bindings: &[0, 1, 6] },
        Entry { name: "relief", bindings: &[0, 1, 13, 2, 6] },
        Entry { name: "combine", bindings: &[0, 7, 8, 9, 10, 11] },
    ],
};

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct Parameters {
    sun_light: [f32; 4],
    ambient_light: [f32; 4],
    width: u32,
    height: u32,
    coarse_width: u32,
    coarse_height: u32,
    sweep_count: u32,
    factor: f32,
    cell: f32,
    x_scale: f32,
    y_scale: f32,
    sun_x: f32,
    sun_y: f32,
    sun_z: f32,
    slope_scale: f32,
    curvature_scale: f32,
    relief_scale: f32,
    relief_curvature_scale: f32,
    octave_weight: f32,
    default_colour: f32,
    direct_lighting: u32,
    shadow_map: u32,
    occlusion_map: u32,
    unshadowed: f32,
    unoccluded: f32,
    use_texture: u32,
    padding: [u32; 4],
}

/// The kernel's `Sweep`.
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct Sweep {
    start: [i32; 2],
    advance_axis: u32,
    padding: u32,
    step: [i32; 2],
    drop: f32,
    scale: f32,
    direction: [f32; 4],
}

/// The sweep of lines towards `direction` across a map `width` wide.
fn sweep(width: usize, direction: [f32; 3], offset: (f32, f32), vertical_scale: f32, scale: f32) -> Sweep {
    let plan = lighting::plan_lines(width, direction, offset, vertical_scale);
    Sweep {
        start: [plan.start.0, plan.start.1],
        advance_axis: if plan.x_major { 0 } else { 1 },
        padding: 0,
        step: [plan.step.0, plan.step.1],
        drop: plan.drop,
        scale,
        direction: [direction[0], direction[1], direction[2], 0.0],
    }
}

/// A square plane of `width` on the GPU.
struct Plane {
    buffer: wgpu::Buffer,
    width: usize,
}

struct Lighting<'a> {
    gpu: &'a GpuDevice,
    terrain: (f32, f32),
}

impl Lighting<'_> {
    fn uniform(&self, parameters: &Parameters) -> wgpu::Buffer {
        self.gpu.uniform(parameters)
    }

    fn square(&self, width: usize) -> Parameters {
        Parameters {
            width: width as u32,
            height: width as u32,
            ..Parameters::default()
        }
    }

    /// The level at half resolution.
    fn downsample(&self, batch: &mut Batch, fine: &Plane) -> Result<Plane, GpuError> {
        let width = fine.width >> 1;
        let coarse = self.gpu.zeroed_words("coarse light level", width * width)?;
        let kernel = self.gpu.kernel(&SHADER, "downsample")?;
        let parameters = Parameters {
            coarse_width: width as u32,
            coarse_height: width as u32,
            ..self.square(fine.width)
        };
        let uniform = self.uniform(&parameters);
        let group = self.gpu.bind(&kernel, &[&uniform, &fine.buffer, &coarse]);
        batch.dispatch_plane(&kernel, &group, width, width)?;
        Ok(Plane { buffer: coarse, width })
    }

    /// The plane upsampled to twice its size.
    fn upsample(&self, batch: &mut Batch, coarse: &Plane) -> Result<Plane, GpuError> {
        let width = coarse.width * 2;
        let fine = self.gpu.zeroed_words("upsampled", width * width)?;
        let kernel = self.gpu.kernel(&SHADER, "upsample")?;
        let parameters = Parameters {
            coarse_width: coarse.width as u32,
            coarse_height: coarse.width as u32,
            ..self.square(width)
        };
        let uniform = self.uniform(&parameters);
        let group = self.gpu.bind(&kernel, &[&uniform, &coarse.buffer, &fine]);
        batch.dispatch_plane(&kernel, &group, width, width)?;
        Ok(Plane { buffer: fine, width })
    }

    /// Unit normals of a plane (three planes in one buffer).
    fn normals(&self, batch: &mut Batch, heights: &Plane) -> Result<wgpu::Buffer, GpuError> {
        let width = heights.width;
        let normals = self.gpu.zeroed_words("normals", 3 * width * width)?;
        let kernel = self.gpu.kernel(&SHADER, "surface_normals")?;
        let (terrain_width, terrain_height) = self.terrain;
        let parameters = Parameters {
            cell: (terrain_width / width as f32) * 32.0,
            x_scale: 0.0 - terrain_height,
            y_scale: terrain_height,
            ..self.square(width)
        };
        let uniform = self.uniform(&parameters);
        let group = self.gpu.bind(&kernel, &[&uniform, &heights.buffer, &normals]);
        batch.dispatch_plane(&kernel, &group, width, width)?;
        Ok(normals)
    }

    /// Height per cell across, for lines on a plane `width` wide.
    fn vertical_scale(&self, width: usize) -> f32 {
        self.terrain.0 / (width as f32 * self.terrain.1)
    }

    /// Lines of sight towards each of `suns` (with its intensity), the
    /// light they gather turned into values by `factor`.
    fn sweep_light(&self, batch: &mut Batch, heights: &Plane, normals: &wgpu::Buffer, suns: &[([f32; 3], f32)], factor: f32) -> Result<Plane, GpuError> {
        let width = heights.width;
        let vertical_scale = self.vertical_scale(width);
        let sweeps: Vec<Sweep> = suns.iter().map(|&(direction, intensity)| sweep(width, direction, (0.5, 0.5), vertical_scale, 262144.0 * intensity)).collect();
        let light = self.gpu.zeroed_words("light", width * width)?;
        if !sweeps.is_empty() {
            let kernel = self.gpu.kernel(&SHADER, "sweep_light")?;
            let sweep_buffer = self.gpu.upload("sweeps", &sweeps)?;
            let parameters = Parameters {
                sweep_count: sweeps.len() as u32,
                ..self.square(width)
            };
            let uniform = self.uniform(&parameters);
            let group = self.gpu.bind(&kernel, &[&uniform, &heights.buffer, normals, &sweep_buffer, &light]);
            batch.dispatch_linear(&kernel, &group, sweeps.len() * width)?;
        }
        self.to_values(batch, &light, width, factor)
    }

    fn to_values(&self, batch: &mut Batch, light: &wgpu::Buffer, width: usize, factor: f32) -> Result<Plane, GpuError> {
        let values = self.gpu.zeroed_words("light values", width * width)?;
        let kernel = self.gpu.kernel(&SHADER, "to_values")?;
        let parameters = Parameters { factor, ..self.square(width) };
        let uniform = self.uniform(&parameters);
        let group = self.gpu.bind(&kernel, &[&uniform, light, &values]);
        batch.dispatch_linear(&kernel, &group, width * width)?;
        Ok(Plane { buffer: values, width })
    }
}

/// `SunLightIntegral` on `gpu`.
pub fn sunlight_integral(gpu: &GpuDevice, heightmap: &Map, settings: &SunlightSettings, terrain: (f32, f32)) -> Result<Map, GpuError> {
    let resolution = heightmap.resolution();
    let lighting = Lighting { gpu, terrain };
    let mut batch = gpu.batch();
    let heights = Plane {
        buffer: gpu.upload("sunlight heights", heightmap.data())?,
        width: resolution.x,
    };
    let coarse = lighting.downsample(&mut batch, &heights)?;
    let normals = lighting.normals(&mut batch, &coarse)?;
    let (count, suns) = lighting::sun_directions(settings);
    let unit_light = 0.67 / (((count as i32) << 16) as f32);
    let light = lighting.sweep_light(&mut batch, &coarse, &normals, &suns, unit_light)?;
    let output = lighting.upsample(&mut batch, &light)?;
    batch.finish()?;
    let data = gpu.read(&output.buffer, resolution.x * resolution.y)?;
    Ok(Map::from_data(resolution, data))
}

/// `Shade` on `gpu`: `rig` lighting `heightmap` under `sky`.
pub fn shade(gpu: &GpuDevice, rig: &LightRig, sky: &Sky, heightmap: &Map, texture: Option<&Map>, terrain: (f32, f32)) -> Result<ShadeMaps, GpuError> {
    let resolution = heightmap.resolution();
    let width = resolution.x;
    let area = width * resolution.y;
    let lighting = Lighting { gpu, terrain };
    let mut batch = gpu.batch();
    let heights = Plane {
        buffer: gpu.upload("shade heights", heightmap.channel(0))?,
        width,
    };
    let normals = lighting.normals(&mut batch, &heights)?;
    let sun = sky.sun_direction;

    let diffuse = gpu.zeroed_words("diffuse", area)?;
    if rig.direct_lighting {
        let kernel = gpu.kernel(&SHADER, "direct_light")?;
        let parameters = Parameters {
            sun_x: sun[0],
            sun_y: sun[1],
            sun_z: sun[2],
            ..lighting.square(width)
        };
        let uniform = lighting.uniform(&parameters);
        let group = gpu.bind(&kernel, &[&uniform, &normals, &diffuse]);
        batch.dispatch_linear(&kernel, &group, area)?;
    }

    let shadow = if rig.direct_lighting && rig.shadows {
        let (count, samples) = shade::shadow_samples(sun, rig.shadow_quality, rig.sun_angular_diameter);
        let vertical_scale = lighting.vertical_scale(width);
        let sweeps: Vec<Sweep> = samples.iter().map(|sample| sweep(width, sample.direction, sample.offset, vertical_scale, 0.0)).collect();
        let lit = gpu.zeroed_words("lit samples", area)?;
        if !sweeps.is_empty() {
            let kernel = gpu.kernel(&SHADER, "sweep_shadow")?;
            let sweep_buffer = gpu.upload("shadow sweeps", &sweeps)?;
            let parameters = Parameters {
                sweep_count: sweeps.len() as u32,
                ..lighting.square(width)
            };
            let uniform = lighting.uniform(&parameters);
            let group = gpu.bind(&kernel, &[&uniform, &heights.buffer, &sweep_buffer, &lit]);
            batch.dispatch_linear(&kernel, &group, sweeps.len() * width)?;
        }
        lighting.to_values(&mut batch, &lit, width, 1.0 / count as f32)?.buffer
    } else {
        gpu.zeroed_words("shadow", area)?
    };

    let occlusion = if rig.ambient_lighting && rig.ambient_occlusion {
        match rig.ao_model {
            AmbientOcclusionModel::Raytraced => {
                let (count, directions) = shade::occlusion_directions(rig.raytraced_ao_quality);
                let unit = 1.0 / (((count as i32) << 16) as f32);
                lighting.sweep_light(&mut batch, &heights, &normals, &directions, unit)?.buffer
            }
            AmbientOcclusionModel::Hybrid => hybrid_occlusion(&lighting, &mut batch, &heights, &normals, rig)?,
            AmbientOcclusionModel::Local => {
                let values = gpu.zeroed_words("local occlusion", area)?;
                let kernel = gpu.kernel(&SHADER, "local_occlusion")?;
                let slope_scale = (terrain.1 * 0.03125) / (terrain.0 / width as f32);
                let parameters = Parameters {
                    slope_scale,
                    curvature_scale: slope_scale * 32.0,
                    ..lighting.square(width)
                };
                let uniform = lighting.uniform(&parameters);
                let group = gpu.bind(&kernel, &[&uniform, &heights.buffer, &values]);
                batch.dispatch_plane(&kernel, &group, width, width)?;
                values
            }
        }
    } else {
        gpu.zeroed_words("occlusion", area)?
    };

    let colour = gpu.zeroed_words("lit colour", 3 * area)?;
    let albedo = match texture {
        Some(map) => {
            let mut planes = Vec::with_capacity(3 * area);
            for channel in 0..3 {
                planes.extend_from_slice(map.rgb_channel(channel));
            }
            gpu.upload("albedo", &planes)?
        }
        None => gpu.zeroed_words("no albedo", 4)?,
    };
    let sun_scale = rig.exposure * rig.sun_intensity;
    let ambient_scale = rig.exposure * rig.ambient_intensity;
    let sun_light = [sky.sun_color[0] * sun_scale, sky.sun_color[1] * sun_scale, sky.sun_color[2] * sun_scale, 0.0];
    let ambient_light = [sky.ambient_color[0] * ambient_scale, sky.ambient_color[1] * ambient_scale, sky.ambient_color[2] * ambient_scale, 0.0];
    let parameters = Parameters {
        sun_light,
        ambient_light,
        default_colour: rig.default_color,
        direct_lighting: u32::from(rig.direct_lighting),
        shadow_map: u32::from(rig.direct_lighting && rig.shadows),
        occlusion_map: u32::from(rig.ambient_lighting && rig.ambient_occlusion),
        unshadowed: if rig.direct_lighting { 1.0 } else { 0.0 },
        unoccluded: if rig.ambient_lighting { 1.0 } else { 0.0 },
        use_texture: u32::from(texture.is_some()),
        ..lighting.square(width)
    };
    let uniform = lighting.uniform(&parameters);
    let kernel = gpu.kernel(&SHADER, "combine")?;
    let group = gpu.bind(&kernel, &[&uniform, &diffuse, &shadow, &occlusion, &albedo, &colour]);
    batch.dispatch_linear(&kernel, &group, area)?;
    batch.finish()?;

    let colour_data: Vec<f32> = gpu.read(&colour, 3 * area)?;
    let channels: Vec<Vec<f32>> = colour_data.chunks(area.max(1)).map(<[f32]>::to_vec).collect();
    Ok(ShadeMaps {
        diffuse: Map::from_data(resolution, gpu.read(&diffuse, area)?),
        shadow: Map::from_data(resolution, gpu.read(&shadow, area)?),
        occlusion: Map::from_data(resolution, gpu.read(&occlusion, area)?),
        color: Map::from_channels(resolution, channels),
    })
}

/// Hybrid occlusion: raytraced on the coarsest level of a pyramid of
/// halvings, upsampled back to full size, then corrected for the relief
/// the coarse levels lost.
fn hybrid_occlusion(lighting: &Lighting, batch: &mut Batch, heights: &Plane, normals: &wgpu::Buffer, rig: &LightRig) -> Result<wgpu::Buffer, GpuError> {
    let gpu = lighting.gpu;
    let (count, directions) = shade::occlusion_directions(rig.hybrid_ao_quality);
    let mut levels: Vec<Plane> = Vec::new();
    for _ in 1..rig.hybrid_ao_octaves {
        let parent = levels.last().unwrap_or(heights);
        let coarse = lighting.downsample(batch, parent)?;
        levels.push(coarse);
    }
    let coarsest = levels.last().unwrap_or(heights);
    let coarse_normals = lighting.normals(batch, coarsest)?;
    let unit = 1.0 / (((count as i32) << 16) as f32);
    let mut occlusion = lighting.sweep_light(batch, coarsest, &coarse_normals, &directions, unit)?;
    let mut smoothed = Plane {
        buffer: gpu.zeroed_words("smoothed", coarsest.width * coarsest.width)?,
        width: coarsest.width,
    };
    batch.copy(&coarsest.buffer, &smoothed.buffer, (coarsest.width * coarsest.width * 4) as u64);
    for _ in 0..levels.len() {
        occlusion = lighting.upsample(batch, &occlusion)?;
        smoothed = lighting.upsample(batch, &smoothed)?;
    }
    if levels.is_empty() {
        return Ok(occlusion.buffer);
    }

    let (terrain_width, terrain_height) = lighting.terrain;
    let coarse_width = levels[0].width;
    let vertical_scale = terrain_width / (coarse_width as f32 * terrain_height);
    let octave_weight = (rig.hybrid_ao_octaves - 1) as f32 * 0.3;
    let parameters = Parameters {
        relief_scale: 0.04 / vertical_scale,
        relief_curvature_scale: 1.0 / vertical_scale,
        octave_weight: if 0.0 > octave_weight { 0.0 } else { octave_weight },
        ..lighting.square(heights.width)
    };
    let uniform = lighting.uniform(&parameters);
    let kernel = gpu.kernel(&SHADER, "relief")?;
    let group = gpu.bind(&kernel, &[&uniform, &heights.buffer, &smoothed.buffer, normals, &occlusion.buffer]);
    batch.dispatch_plane(&kernel, &group, heights.width, heights.width)?;
    Ok(occlusion.buffer)
}
