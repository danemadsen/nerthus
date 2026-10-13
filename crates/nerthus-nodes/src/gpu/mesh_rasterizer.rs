//! The mesh rasteriser on a GPU: [`crate::mesh_rasterizer`]'s renders with
//! the per-pixel triangle tests as kernels. The vertices are transformed
//! (and point clouds triangulated) on the CPU, as the SYCL host prepares
//! them, and the CPU sorts the triangles into small ones (a thread each)
//! and large ones (a thread per candidate pixel).

use bytemuck::{Pod, Zeroable};
use nerthus_compute::wgpu;
use nerthus_compute::{Batch, Binding, Entry, GpuDevice, GpuError, Shader};
use rayon::prelude::*;

use crate::mesh_rasterizer::{self, CloudRender, Mesh, RenderSettings, TriplanarSettings};
use crate::x86::{cvttss2si, maxss, minss};

const SHADER: Shader = Shader {
    name: "mesh-rasterizer",
    source: include_str!("shaders/mesh-rasterizer.wgsl"),
    bindings: &[
        Binding::Uniform,
        Binding::Read,
        Binding::Read,
        Binding::ReadWrite,
        Binding::Read,
        Binding::Read,
        Binding::Read,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::ReadWrite,
        Binding::Read,
        Binding::ReadWrite,
    ],
    entries: &[
        Entry { name: "rasterize_small", bindings: &[0, 1, 2, 3] },
        Entry { name: "rasterize_large", bindings: &[0, 1, 2, 3, 4, 5] },
        Entry { name: "triangle_colours", bindings: &[0, 1, 2, 6, 7] },
        Entry { name: "resolve_cloud", bindings: &[0, 1, 2, 3, 7, 8, 9, 10] },
        Entry { name: "downsample_box", bindings: &[0, 11, 12] },
    ],
};

/// Triangles with more candidates than this go to the large launches (the
/// kernel's `SMALL_LIMIT`).
const SMALL_LIMIT: u32 = 64;

/// The most candidates one large launch numbers.
const LAUNCH_CANDIDATES: u64 = 1 << 30;

/// What the kernels render (the kernel's `mode`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Mesh,
    Cloud,
    TexturedCloud,
    Displaced,
}

impl Mode {
    fn code(self) -> u32 {
        match self {
            Mode::Mesh => 0,
            Mode::Cloud => 1,
            Mode::TexturedCloud => 2,
            Mode::Displaced => 3,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct Parameters {
    width: i32,
    height: i32,
    triangle_count: u32,
    mode: u32,
    inlined: u32,
    large_first: u32,
    large_count: u32,
    candidate_total: u32,
    channel_count: u32,
    padding: [u32; 3],
}

/// The candidates of a triangle as the kernel's `candidates` counts them.
fn candidate_count(a: [f32; 3], b: [f32; 3], c: [f32; 3], mode: Mode, width: i32, height: i32) -> u32 {
    if mode == Mode::Displaced {
        let first = (b[0] - a[0]) * (c[1] - a[1]);
        let second = (c[0] - a[0]) * (b[1] - a[1]);
        if first > second {
            return 0;
        }
        let low_y = cvttss2si(minss(a[1], minss(c[1], b[1])).floor());
        let high_y = cvttss2si(maxss(a[1], maxss(c[1], b[1])).ceil());
        let high_x = cvttss2si(maxss(a[0], maxss(b[0], c[0])).ceil());
        let row_start = low_y.max(0);
        let row_end = if high_y < height - 1 { high_y } else { height - 1 };
        let column_end = if high_x < width - 1 { high_x } else { width - 1 };
        if row_end < row_start {
            return 0;
        }
        let column_start = cvttss2si(minss(a[0], minss(b[0], c[0])).floor()).max(0);
        if column_end < column_start {
            return 0;
        }
        return (column_end - column_start + 1) as u32 * (row_end - row_start + 1) as u32;
    }
    let count = if mode == Mode::TexturedCloud { mesh_rasterizer::box_pixel_count(a, b, c) } else { mesh_rasterizer::candidate_count(a, b, c) };
    let low_x = cvttss2si(minss(minss(a[0], b[0]), c[0]));
    let high_x = cvttss2si(maxss(maxss(a[0], b[0]), c[0]));
    let box_width = high_x.wrapping_sub(low_x).wrapping_add(1) as u32;
    if box_width == 0 {
        return 0;
    }
    count
}

/// Rasterises `triangles` into a raster of `width × height` words: the
/// highest surface's bits per pixel, or one more than the last triangle
/// over it for point clouds.
struct Raster<'a> {
    gpu: &'a GpuDevice,
    vertices: wgpu::Buffer,
    triangles: wgpu::Buffer,
    raster: wgpu::Buffer,
    base: Parameters,
}

fn rasterize<'a>(gpu: &'a GpuDevice, batch: &mut Batch, vertices: &[[f32; 3]], triangles: &[[u32; 3]], width: i32, height: i32, mode: Mode) -> Result<Raster<'a>, GpuError> {
    let pixel_count = width.max(0) as usize * height.max(0) as usize;
    let counts: Vec<u32> = triangles
        .par_iter()
        .map(|triangle| {
            let [a, b, c] = triangle.map(|index| vertices[index as usize]);
            candidate_count(a, b, c, mode, width, height)
        })
        .collect();
    // The plain region tests pixels out of line when there are three
    // candidates or fewer in all (an i32 running sum).
    let mut running = 0i32;
    for &count in &counts {
        running = running.wrapping_add(count as i32);
    }
    let inlined = running as i64 as u64 > 3;

    let flat_vertices: Vec<f32> = vertices.iter().flatten().copied().collect();
    let flat_triangles: Vec<u32> = triangles.iter().flatten().copied().collect();
    let base = Parameters {
        width,
        height,
        triangle_count: triangles.len() as u32,
        mode: mode.code(),
        inlined: u32::from(inlined),
        ..Parameters::default()
    };
    let raster = Raster {
        gpu,
        vertices: gpu.upload("mesh vertices", &flat_vertices)?,
        triangles: gpu.upload("mesh triangles", &flat_triangles)?,
        raster: gpu.zeroed_words("raster", pixel_count)?,
        base,
    };
    if triangles.is_empty() || pixel_count == 0 {
        return Ok(raster);
    }

    let small = gpu.kernel(&SHADER, "rasterize_small")?;
    let uniform = gpu.uniform(&base);
    let group = gpu.bind(&small, &[&uniform, &raster.vertices, &raster.triangles, &raster.raster]);
    batch.dispatch_linear(&small, &group, triangles.len())?;

    // The large triangles, in launches of at most LAUNCH_CANDIDATES.
    let mut large = Vec::new();
    let mut starts = Vec::new();
    let mut launches = Vec::new();
    let mut launch_first = 0usize;
    let mut launch_total = 0u64;
    for (index, &count) in counts.iter().enumerate() {
        if count <= SMALL_LIMIT {
            continue;
        }
        if launch_total > 0 && launch_total + count as u64 > LAUNCH_CANDIDATES {
            launches.push((launch_first, large.len() - launch_first, launch_total));
            launch_first = large.len();
            launch_total = 0;
        }
        large.push(index as u32);
        starts.push(launch_total as u32);
        launch_total += count as u64;
    }
    if launch_total > 0 {
        launches.push((launch_first, large.len() - launch_first, launch_total));
    }
    if launches.is_empty() {
        return Ok(raster);
    }
    let large_kernel = gpu.kernel(&SHADER, "rasterize_large")?;
    let large_buffer = gpu.upload("large triangles", &large)?;
    let start_buffer = gpu.upload("large triangle starts", &starts)?;
    for (first, count, total) in launches {
        let parameters = Parameters {
            large_first: first as u32,
            large_count: count as u32,
            candidate_total: total as u32,
            ..base
        };
        let uniform = gpu.uniform(&parameters);
        let buffers = [&uniform, &raster.vertices, &raster.triangles, &raster.raster, &large_buffer, &start_buffer];
        let group = gpu.bind(&large_kernel, &buffers);
        batch.dispatch_linear(&large_kernel, &group, total as usize)?;
    }
    Ok(raster)
}

impl Raster<'_> {
    /// The raster as heights (the surfaces' bits), box-downsampled to
    /// `target` when antialiased.
    fn heights(&self, batch: Batch, target: (i32, i32), antialiased: bool) -> Result<Vec<f32>, GpuError> {
        let gpu = self.gpu;
        let mut batch = batch;
        let (width, height) = (target.0.max(0) as usize, target.1.max(0) as usize);
        if !antialiased {
            batch.finish()?;
            return gpu.read(&self.raster, width * height);
        }
        let kernel = gpu.kernel(&SHADER, "downsample_box")?;
        let parameters = Parameters {
            width: target.0,
            height: target.1,
            ..self.base
        };
        let uniform = gpu.uniform(&parameters);
        let coarse = gpu.zeroed_words("antialiased", width * height)?;
        let group = gpu.bind(&kernel, &[&uniform, &self.raster, &coarse]);
        batch.dispatch_plane(&kernel, &group, width, height)?;
        batch.finish()?;
        gpu.read(&coarse, width * height)
    }
}

/// `MeshRasterizer_RenderMesh` on `gpu`.
pub fn render_mesh(gpu: &GpuDevice, mesh: &Mesh, settings: &RenderSettings) -> Result<Vec<f32>, GpuError> {
    let factor = if settings.antialiasing { 2 } else { 1 };
    let width = settings.width * factor;
    let height = settings.height * factor;
    let mut vertices = mesh.vertices.clone();
    mesh_rasterizer::process_vertices(&mut vertices, width, height, settings);
    let mut batch = gpu.batch();
    let raster = rasterize(gpu, &mut batch, &vertices, &mesh.triangles, width, height, Mode::Mesh)?;
    raster.heights(batch, (settings.width, settings.height), settings.antialiasing)
}

/// `MeshRasterizer_RenderMeshWithTriplanarDisplacement` on `gpu`.
pub fn render_mesh_triplanar(gpu: &GpuDevice, mesh: &Mesh, settings: &TriplanarSettings, maps: [&[f32]; 3]) -> Result<Vec<f32>, GpuError> {
    let (width, height) = mesh_rasterizer::triplanar_size(settings);
    let vertices = mesh_rasterizer::displaced_vertices(mesh, settings, maps);
    let mut batch = gpu.batch();
    let raster = rasterize(gpu, &mut batch, &vertices, &mesh.triangles, width, height, Mode::Displaced)?;
    raster.heights(batch, (settings.width, settings.height), settings.antialiasing)
}

/// `MeshRasterizer_RenderPointCloud` on `gpu`: the heights, the triangle
/// hashes and, with a texture (its planes), the texture painted with each
/// triangle's colour.
pub fn render_point_cloud(gpu: &GpuDevice, points: &[[f32; 3]], width: i32, height: i32, texture: Option<&[&[f32]]>) -> Result<CloudRender, GpuError> {
    let pixel_count = width.max(0) as usize * height.max(0) as usize;
    let Some(Mesh { vertices, triangles, .. }) = mesh_rasterizer::cloud_mesh(points, width, height) else {
        let painted = texture.map(|planes| planes.iter().map(|plane| plane.to_vec()).collect());
        return Ok(CloudRender { heights: vec![0.0; pixel_count], hashes: vec![0.0; pixel_count], painted });
    };
    let mode = if texture.is_some() { Mode::TexturedCloud } else { Mode::Cloud };
    let mut batch = gpu.batch();
    let raster = rasterize(gpu, &mut batch, &vertices, &triangles, width, height, mode)?;

    let channel_count = texture.map_or(1, <[&[f32]]>::len);
    let parameters = Parameters {
        channel_count: channel_count as u32,
        ..raster.base
    };
    let uniform = gpu.uniform(&parameters);
    let heights = gpu.zeroed_words("cloud heights", pixel_count)?;
    let hashes = gpu.zeroed_words("cloud hashes", pixel_count)?;
    let (colours, painted) = match texture {
        Some(planes) => {
            let flat: Vec<f32> = planes.iter().flat_map(|plane| plane.iter().copied()).collect();
            let texture_planes = gpu.upload("cloud texture", &flat)?;
            let painted = gpu.upload("painted texture", &flat)?;
            let colours = gpu.zeroed("triangle colours", triangles.len() as u64 * 16)?;
            let kernel = gpu.kernel(&SHADER, "triangle_colours")?;
            let group = gpu.bind(&kernel, &[&uniform, &raster.vertices, &raster.triangles, &texture_planes, &colours]);
            batch.dispatch_linear(&kernel, &group, triangles.len())?;
            (colours, painted)
        }
        None => (gpu.zeroed_words("unused colours", 4)?, gpu.zeroed_words("unused texture", 4)?),
    };
    let kernel = gpu.kernel(&SHADER, "resolve_cloud")?;
    let buffers = [&uniform, &raster.vertices, &raster.triangles, &raster.raster, &colours, &heights, &hashes, &painted];
    let group = gpu.bind(&kernel, &buffers);
    batch.dispatch_plane(&kernel, &group, width.max(0) as usize, height.max(0) as usize)?;
    batch.finish()?;

    let painted_planes = match texture {
        Some(planes) => {
            let flat: Vec<f32> = gpu.read(&painted, planes.len() * pixel_count)?;
            Some(flat.chunks(pixel_count.max(1)).map(<[f32]>::to_vec).collect())
        }
        None => None,
    };
    Ok(CloudRender {
        heights: gpu.read(&heights, pixel_count)?,
        hashes: gpu.read(&hashes, pixel_count)?,
        painted: painted_planes,
    })
}
