//! The pyramid steps the SYCL functions share, on a GPU: the next coarser
//! level and the detail a level loses to it (`flow::downsample`,
//! `flow::downsample_usm` and `flow::detail`).

use bytemuck::{Pod, Zeroable};
use nerthus_compute::{Batch, Binding, Entry, GpuDevice, GpuError, Shader};

use crate::gpu::GpuPlane;

const SHADER: Shader = Shader {
    name: "pyramid",
    source: include_str!("shaders/pyramid.wgsl"),
    bindings: &[Binding::Uniform, Binding::Read, Binding::ReadWrite, Binding::ReadWrite],
    entries: &[Entry { name: "downsample", bindings: &[0, 1, 2] }],
};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Parameters {
    fine_width: u32,
    fine_height: u32,
    coarse_width: u32,
    coarse_height: u32,
}

fn parameters(fine: &GpuPlane, coarse_width: usize, coarse_height: usize) -> Parameters {
    Parameters {
        fine_width: fine.width as u32,
        fine_height: fine.height as u32,
        coarse_width: coarse_width as u32,
        coarse_height: coarse_height as u32,
    }
}

/// The level at half resolution (recorded in `batch`).
pub(crate) fn downsample(gpu: &GpuDevice, batch: &mut Batch, fine: &GpuPlane) -> Result<GpuPlane, GpuError> {
    let width = fine.width >> 1;
    let height = fine.height >> 1;
    let coarse = GpuPlane::zeroed(gpu, "coarse level", width, height)?;
    let kernel = gpu.kernel(&SHADER, "downsample")?;
    let uniform = gpu.uniform(&parameters(fine, width, height));
    let group = gpu.bind(&kernel, &[&uniform, &fine.buffer, &coarse.buffer]);
    batch.dispatch_plane(&kernel, &group, width, height)?;
    Ok(coarse)
}

/// The detail `fine` loses to `coarse`.
pub(crate) fn detail(gpu: &GpuDevice, batch: &mut Batch, fine: &GpuPlane, coarse: &GpuPlane) -> Result<GpuPlane, GpuError> {
    let detail = GpuPlane::zeroed(gpu, "detail", fine.width, fine.height)?;
    let kernel = gpu.kernel(&SHADER, "detail_of")?;
    let uniform = gpu.uniform(&parameters(fine, coarse.width, coarse.height));
    let group = gpu.bind(&kernel, &[&uniform, &fine.buffer, &coarse.buffer, &detail.buffer]);
    batch.dispatch_plane(&kernel, &group, fine.width, fine.height)?;
    Ok(detail)
}
