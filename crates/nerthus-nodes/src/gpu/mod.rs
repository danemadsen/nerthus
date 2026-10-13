//! The SYCL functions' kernels on a GPU (see [`crate::sycl`]): WGSL
//! versions of the CPU code in the modules of the same names. The steps
//! SYCL libraries run on their host (pyramid levels, pass counts, random
//! shuffles) stay on the CPU here too.

pub mod cellular3d;
pub mod colour_transport;
pub mod flow;
pub mod lighting;
pub mod median;
pub mod mesh_rasterizer;
pub mod parallel_erosion;
pub mod thermal;
pub mod thermal_shaper;

mod pyramid;

use nerthus_compute::wgpu;
use nerthus_compute::{GpuDevice, GpuError};

/// A plane of floats in a GPU buffer.
pub(crate) struct GpuPlane {
    pub(crate) buffer: wgpu::Buffer,
    pub(crate) width: usize,
    pub(crate) height: usize,
}

impl GpuPlane {
    pub(crate) fn upload(gpu: &GpuDevice, label: &str, data: &[f32], width: usize, height: usize) -> Result<GpuPlane, GpuError> {
        let buffer = gpu.upload(label, data)?;
        Ok(GpuPlane { buffer, width, height })
    }

    pub(crate) fn zeroed(gpu: &GpuDevice, label: &str, width: usize, height: usize) -> Result<GpuPlane, GpuError> {
        let buffer = gpu.zeroed_words(label, width * height)?;
        Ok(GpuPlane { buffer, width, height })
    }

    pub(crate) fn len(&self) -> usize {
        self.width * self.height
    }

    pub(crate) fn read(&self, gpu: &GpuDevice) -> Result<Vec<f32>, GpuError> {
        gpu.read(&self.buffer, self.len())
    }
}
