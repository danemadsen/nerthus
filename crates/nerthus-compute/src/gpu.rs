//! The GPUs wgpu finds (Nerthus's second compute library), and what the
//! kernels need to run on one: shader pipelines, buffers, batches of
//! dispatches and reading results back.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use bytemuck::Pod;
use wgpu::util::DeviceExt;

use crate::devices::{DeviceInfo, DeviceType, NativeResult};

/// The library id of wgpu's devices.
pub(crate) const LIBRARY: &str = "wgpu";

/// The backends Nerthus computes on. OpenGL is left out: its compute
/// support is too thin for the kernels.
const BACKENDS: wgpu::Backends = wgpu::Backends::PRIMARY;

/// Threads per workgroup of one-dimensional launches.
pub const GROUP_SIZE: u32 = 256;
/// Threads along each side of a two-dimensional launch's workgroup.
pub const GROUP_SIDE: u32 = 16;

/// Storage buffers the biggest kernel binds at once.
const STORAGE_BUFFERS: u32 = 8;

/// Dispatches recorded before a batch hands them to the GPU, keeping each
/// command buffer short.
const DISPATCHES_PER_SUBMISSION: usize = 64;

/// WGSL every kernel module starts with.
const PRELUDE: &str = include_str!("prelude.wgsl");

static INSTANCE: LazyLock<wgpu::Instance> = LazyLock::new(|| {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = BACKENDS;
    descriptor.flags = wgpu::InstanceFlags::empty();
    let descriptor = descriptor.with_env();
    wgpu::Instance::new(descriptor)
});

/// What a kernel failure means for the device it ran on.
#[derive(Clone, Debug)]
pub enum GpuError {
    /// The device has too little memory (or too small buffer limits).
    OutOfMemory(String),
    /// Anything else: a validation or internal error, a lost device.
    Failed(String),
}

impl GpuError {
    /// The `NativeResult` a SYCL library returns for the failure.
    pub fn native_result(&self) -> NativeResult {
        match self {
            GpuError::OutOfMemory(_) => NativeResult::InsufficientDeviceMemory,
            GpuError::Failed(_) => NativeResult::UndefinedError,
        }
    }
}

impl fmt::Display for GpuError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GpuError::OutOfMemory(message) => write!(formatter, "not enough device memory: {message}"),
            GpuError::Failed(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for GpuError {}

/// How a kernel module's bindings (group 0, numbered in order) are used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binding {
    Uniform,
    /// A storage buffer the kernels only read.
    Read,
    /// A storage buffer the kernels write (or update atomically).
    ReadWrite,
}

/// A WGSL module and the buffers its kernels bind (group 0, by binding
/// number). An entry point binds all of them unless `entries` names the
/// ones it uses.
pub struct Shader {
    pub name: &'static str,
    pub source: &'static str,
    pub bindings: &'static [Binding],
    pub entries: &'static [Entry],
}

/// An entry point that binds only some of its module's bindings.
pub struct Entry {
    pub name: &'static str,
    /// Its binding numbers, in the order `GpuDevice::bind` takes buffers.
    pub bindings: &'static [u32],
}

/// A compiled entry point.
pub struct Kernel {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    bindings: Vec<u32>,
}

/// An open GPU: its device and queue, and the kernels compiled for it.
pub struct GpuDevice {
    info: DeviceInfo,
    device: wgpu::Device,
    queue: wgpu::Queue,
    limits: wgpu::Limits,
    lost: Arc<AtomicBool>,
    modules: Mutex<HashMap<&'static str, wgpu::ShaderModule>>,
    kernels: Mutex<HashMap<(&'static str, &'static str), Arc<Kernel>>>,
    spare: OnceLock<wgpu::Buffer>,
}

/// The device type a wgpu device type stands for; software rasterisers are
/// not compute devices.
fn device_type(kind: wgpu::DeviceType) -> Option<DeviceType> {
    match kind {
        wgpu::DeviceType::DiscreteGpu => Some(DeviceType::GpuDiscrete),
        wgpu::DeviceType::IntegratedGpu => Some(DeviceType::GpuIntegrated),
        wgpu::DeviceType::VirtualGpu | wgpu::DeviceType::Other => Some(DeviceType::Unknown),
        wgpu::DeviceType::Cpu => None,
    }
}

fn backend_name(backend: wgpu::Backend) -> Option<&'static str> {
    match backend {
        wgpu::Backend::Metal => Some("Metal"),
        wgpu::Backend::Vulkan => Some("Vulkan"),
        wgpu::Backend::Dx12 => Some("DirectX 12"),
        _ => None,
    }
}

/// The maker of an adapter, from its PCI vendor id or, where the backend
/// gives none (Metal), from its name.
fn vendor_name(vendor: u32, name: &str) -> String {
    let by_id = match vendor {
        0x10DE => Some("NVIDIA"),
        0x1002 | 0x1022 => Some("AMD"),
        0x8086 => Some("Intel"),
        0x106B => Some("Apple"),
        0x13B5 => Some("ARM"),
        0x5143 => Some("Qualcomm"),
        0x1010 => Some("Imagination"),
        0x14E4 => Some("Broadcom"),
        0x1414 => Some("Microsoft"),
        _ => None,
    };
    if let Some(vendor) = by_id {
        return vendor.to_string();
    }
    let lowered = name.to_lowercase();
    let by_name = [("nvidia", "NVIDIA"), ("geforce", "NVIDIA"), ("radeon", "AMD"), ("amd", "AMD"), ("intel", "Intel"), ("apple", "Apple")];
    for (needle, vendor) in by_name {
        if lowered.contains(needle) {
            return vendor.to_string();
        }
    }
    format!("0x{vendor:04X}")
}

/// Why an adapter cannot run the kernels, if it cannot.
fn missing_capability(adapter: &wgpu::Adapter) -> Option<String> {
    let downlevel = adapter.get_downlevel_capabilities();
    if !downlevel.flags.contains(wgpu::DownlevelFlags::COMPUTE_SHADERS) {
        return Some("no compute shaders".to_string());
    }
    let limits = adapter.limits();
    if limits.max_storage_buffers_per_shader_stage < STORAGE_BUFFERS {
        let count = limits.max_storage_buffers_per_shader_stage;
        return Some(format!("{count} storage buffers per kernel, {STORAGE_BUFFERS} needed"));
    }
    let workgroups_fit = limits.max_compute_invocations_per_workgroup >= GROUP_SIZE && limits.max_compute_workgroup_size_x >= GROUP_SIZE && limits.max_compute_workgroup_size_y >= GROUP_SIDE;
    if !workgroups_fit {
        return Some(format!("workgroups smaller than {GROUP_SIZE} threads"));
    }
    None
}

/// An adapter wgpu found, the device Nerthus reports it as, and why it
/// cannot be used (if so).
struct FoundAdapter {
    adapter: wgpu::Adapter,
    info: DeviceInfo,
    problem: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
fn find_adapters() -> Vec<FoundAdapter> {
    let adapters = pollster::block_on(INSTANCE.enumerate_adapters(BACKENDS));
    let mut found = Vec::new();
    let mut seen: HashMap<String, u32> = HashMap::new();
    for adapter in adapters {
        let adapter_info = adapter.get_info();
        let Some(kind) = device_type(adapter_info.device_type) else {
            continue;
        };
        let Some(backend) = backend_name(adapter_info.backend) else {
            continue;
        };
        let vendor = vendor_name(adapter_info.vendor, &adapter_info.name);
        let mut inner_id = if adapter_info.device_pci_bus_id.is_empty() {
            format!("{:04x}:{:04x}", adapter_info.vendor, adapter_info.device)
        } else {
            adapter_info.device_pci_bus_id.clone()
        };
        // Identical cards on one backend are told apart by their order.
        let key = format!("{backend}/{}/{inner_id}", adapter_info.name);
        let ordinal = seen.entry(key).or_insert(0);
        if *ordinal > 0 {
            inner_id = format!("{inner_id}#{ordinal}");
        }
        *ordinal += 1;

        let problem = missing_capability(&adapter);
        let mut info = DeviceInfo::new(LIBRARY, kind, backend, &vendor, &adapter_info.name, &inner_id);
        info.requires_driver_update = problem.is_some();
        found.push(FoundAdapter { adapter, info, problem });
    }
    found
}

/// None in a browser: its GPU (WebGPU) answers only asynchronously, and
/// the kernels wait on it, which a web page cannot do; the CPU computes.
#[cfg(target_arch = "wasm32")]
fn find_adapters() -> Vec<FoundAdapter> {
    Vec::new()
}

/// The GPUs wgpu finds, and a line for each one that cannot run the
/// kernels.
pub(crate) fn available_devices() -> (Vec<DeviceInfo>, String) {
    let mut problems = Vec::new();
    let mut devices = Vec::new();
    for found in find_adapters() {
        if let Some(problem) = &found.problem {
            problems.push(format!("{} ({}): {problem}", found.info.device_name, found.info.backend_name));
        }
        devices.push(found.info);
    }
    (devices, problems.join("\n"))
}

/// Opens the GPU with this id without activating it (for benchmarks, and
/// for activation).
pub fn open_device(id: &str) -> Option<GpuDevice> {
    let found = find_adapters().into_iter().find(|found| found.info.id == id)?;
    if let Some(problem) = &found.problem {
        log::warn!("{} ({}) cannot compute: {problem}", found.info.device_name, found.info.backend_name);
        return None;
    }
    match GpuDevice::open(&found.adapter, found.info) {
        Ok(gpu) => Some(gpu),
        Err(error) => {
            log::warn!("cannot open {id}: {error}");
            None
        }
    }
}

impl GpuDevice {
    fn open(adapter: &wgpu::Adapter, info: DeviceInfo) -> Result<GpuDevice, String> {
        let limits = adapter.limits();
        let descriptor = wgpu::DeviceDescriptor {
            label: Some("Nerthus compute"),
            required_limits: limits.clone(),
            ..Default::default()
        };
        let requested = adapter.request_device(&descriptor);
        let (device, queue) = pollster::block_on(requested).map_err(|error| error.to_string())?;

        let device_name = info.device_name.clone();
        let report = move |error: wgpu::Error| log::error!("{device_name}: {error}");
        device.on_uncaptured_error(Arc::new(report));
        let lost = Arc::new(AtomicBool::new(false));
        let lost_flag = Arc::clone(&lost);
        let device_name = info.device_name.clone();
        device.set_device_lost_callback(move |reason, message| {
            log::warn!("{device_name} was lost ({reason:?}): {message}");
            lost_flag.store(true, Ordering::Relaxed);
        });

        Ok(GpuDevice {
            info,
            device,
            queue,
            limits,
            lost,
            modules: Mutex::new(HashMap::new()),
            kernels: Mutex::new(HashMap::new()),
            spare: OnceLock::new(),
        })
    }

    pub fn info(&self) -> &DeviceInfo {
        &self.info
    }

    pub fn limits(&self) -> &wgpu::Limits {
        &self.limits
    }

    /// Runs `work` on this device and reports any error the GPU raised
    /// while it ran: the SYCL function's `NativeResult` on this device.
    pub fn run<T>(&self, work: impl FnOnce(&GpuDevice) -> Result<T, GpuError>) -> Result<T, GpuError> {
        if self.lost.load(Ordering::Relaxed) {
            return Err(GpuError::Failed("the device was lost".to_string()));
        }
        let memory_scope = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation_scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal_scope = self.device.push_error_scope(wgpu::ErrorFilter::Internal);
        let result = work(self);
        let internal_error = pollster::block_on(internal_scope.pop());
        let validation_error = pollster::block_on(validation_scope.pop());
        let memory_error = pollster::block_on(memory_scope.pop());

        if let Some(error) = memory_error {
            return Err(GpuError::OutOfMemory(error.to_string()));
        }
        if let Some(error) = validation_error.or(internal_error) {
            return Err(GpuError::Failed(error.to_string()));
        }
        if self.lost.load(Ordering::Relaxed) {
            return Err(GpuError::Failed("the device was lost".to_string()));
        }
        result
    }

    fn module(&self, shader: &Shader) -> wgpu::ShaderModule {
        let mut modules = self.modules.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let module = modules.entry(shader.name).or_insert_with(|| {
            let source = format!("{PRELUDE}\n{}", shader.source);
            let descriptor = wgpu::ShaderModuleDescriptor {
                label: Some(shader.name),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            };
            self.device.create_shader_module(descriptor)
        });
        module.clone()
    }

    /// The entry point `entry` of `shader`, compiled on first use.
    pub fn kernel(&self, shader: &Shader, entry: &'static str) -> Result<Arc<Kernel>, GpuError> {
        let key = (shader.name, entry);
        {
            let kernels = self.kernels.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(kernel) = kernels.get(&key) {
                return Ok(Arc::clone(kernel));
            }
        }

        let listed = shader.entries.iter().find(|listed| listed.name == entry);
        let bindings: Vec<u32> = match listed {
            Some(listed) => listed.bindings.to_vec(),
            None => (0..shader.bindings.len() as u32).collect(),
        };
        let unknown = bindings.iter().find(|&&number| number as usize >= shader.bindings.len());
        if let Some(number) = unknown {
            return Err(GpuError::Failed(format!("{}::{entry} lists binding {number}, which the module lacks", shader.name)));
        }

        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = self.module(shader);
        let entries: Vec<wgpu::BindGroupLayoutEntry> = bindings.iter().map(|&number| layout_entry(number, shader.bindings[number as usize])).collect();
        let layout_descriptor = wgpu::BindGroupLayoutDescriptor { label: Some(shader.name), entries: &entries };
        let layout = self.device.create_bind_group_layout(&layout_descriptor);
        let layouts = [Some(&layout)];
        let pipeline_layout_descriptor = wgpu::PipelineLayoutDescriptor {
            label: Some(shader.name),
            bind_group_layouts: &layouts,
            immediate_size: 0,
        };
        let pipeline_layout = self.device.create_pipeline_layout(&pipeline_layout_descriptor);
        let pipeline_descriptor = wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        };
        let pipeline = self.device.create_compute_pipeline(&pipeline_descriptor);
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(GpuError::Failed(format!("{}::{entry} does not compile: {error}", shader.name)));
        }

        let kernel = Arc::new(Kernel { pipeline, layout, bindings });
        let mut kernels = self.kernels.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        kernels.insert(key, Arc::clone(&kernel));
        Ok(kernel)
    }

    /// Refuses buffers larger than the device can bind.
    fn check_size(&self, bytes: u64) -> Result<(), GpuError> {
        let limit = self.limits.max_storage_buffer_binding_size.min(self.limits.max_buffer_size);
        if bytes > limit {
            return Err(GpuError::OutOfMemory(format!("a buffer of {bytes} bytes is over the device's limit of {limit}")));
        }
        Ok(())
    }

    /// A storage buffer holding `data` (four bytes at least: empty data
    /// becomes one zero word, which kernels never read).
    pub fn upload<T: Pod>(&self, label: &str, data: &[T]) -> Result<wgpu::Buffer, GpuError> {
        let bytes: &[u8] = bytemuck::cast_slice(data);
        if bytes.is_empty() {
            return self.zeroed(label, 4);
        }
        self.check_size(bytes.len() as u64)?;
        let descriptor = wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage: storage_usage(),
        };
        Ok(self.device.create_buffer_init(&descriptor))
    }

    /// A zeroed storage buffer of `bytes` bytes (rounded up to whole words,
    /// four at least).
    pub fn zeroed(&self, label: &str, bytes: u64) -> Result<wgpu::Buffer, GpuError> {
        let size = bytes.max(4).next_multiple_of(4);
        self.check_size(size)?;
        let descriptor = wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: storage_usage(),
            mapped_at_creation: false,
        };
        Ok(self.device.create_buffer(&descriptor))
    }

    /// A zeroed storage buffer of `count` floats or words.
    pub fn zeroed_words(&self, label: &str, count: usize) -> Result<wgpu::Buffer, GpuError> {
        self.zeroed(label, count as u64 * 4)
    }

    /// A small storage buffer to fill bindings a kernel does not use (a
    /// buffer may not be bound both writable and read in one dispatch).
    pub fn spare(&self) -> &wgpu::Buffer {
        self.spare.get_or_init(|| {
            let descriptor = wgpu::BufferDescriptor {
                label: Some("spare"),
                size: 16,
                usage: storage_usage(),
                mapped_at_creation: false,
            };
            self.device.create_buffer(&descriptor)
        })
    }

    /// A uniform buffer holding `value`.
    pub fn uniform<T: Pod>(&self, value: &T) -> wgpu::Buffer {
        let descriptor = wgpu::util::BufferInitDescriptor {
            label: Some("parameters"),
            contents: bytemuck::bytes_of(value),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        };
        self.device.create_buffer_init(&descriptor)
    }

    /// Writes `data` into the start of `buffer` before the next submission.
    pub fn write<T: Pod>(&self, buffer: &wgpu::Buffer, data: &[T]) {
        self.queue.write_buffer(buffer, 0, bytemuck::cast_slice(data));
    }

    /// Binds `buffers` to the bindings a kernel uses, in order.
    pub fn bind(&self, kernel: &Kernel, buffers: &[&wgpu::Buffer]) -> wgpu::BindGroup {
        let entries: Vec<wgpu::BindGroupEntry> = kernel
            .bindings
            .iter()
            .zip(buffers)
            .map(|(&number, buffer)| wgpu::BindGroupEntry {
                binding: number,
                resource: buffer.as_entire_binding(),
            })
            .collect();
        let descriptor = wgpu::BindGroupDescriptor {
            label: None,
            layout: &kernel.layout,
            entries: &entries,
        };
        self.device.create_bind_group(&descriptor)
    }

    /// A batch of dispatches and copies to record.
    pub fn batch(&self) -> Batch<'_> {
        Batch {
            gpu: self,
            encoder: self.encoder(),
            recorded: 0,
            in_flight: None,
        }
    }

    fn encoder(&self) -> wgpu::CommandEncoder {
        let descriptor = wgpu::CommandEncoderDescriptor { label: Some("Nerthus compute") };
        self.device.create_command_encoder(&descriptor)
    }

    fn wait(&self, submission: Option<wgpu::SubmissionIndex>) -> Result<(), GpuError> {
        let poll = wgpu::PollType::Wait { submission_index: submission, timeout: None };
        match self.device.poll(poll) {
            Ok(_) => Ok(()),
            Err(error) => Err(GpuError::Failed(error.to_string())),
        }
    }

    /// The first `count` values of `buffer`, once everything submitted
    /// before has run.
    pub fn read<T: Pod>(&self, buffer: &wgpu::Buffer, count: usize) -> Result<Vec<T>, GpuError> {
        let bytes = (count * size_of::<T>()) as u64;
        if bytes == 0 {
            return Ok(Vec::new());
        }
        let size = bytes.next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
        let staging_descriptor = wgpu::BufferDescriptor {
            label: Some("read back"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        };
        let staging = self.device.create_buffer(&staging_descriptor);
        let mut encoder = self.encoder();
        encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, size);
        let submission = self.queue.submit([encoder.finish()]);

        let slice = staging.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |mapped| {
            let _ = sender.send(mapped);
        });
        self.wait(Some(submission))?;
        let mapped = receiver.recv().map_err(|error| GpuError::Failed(error.to_string()))?;
        mapped.map_err(|error| GpuError::Failed(error.to_string()))?;

        let view = slice.get_mapped_range().map_err(|error| GpuError::Failed(error.to_string()))?;
        let values: Vec<T> = bytemuck::pod_collect_to_vec(&view[..bytes as usize]);
        drop(view);
        staging.unmap();
        Ok(values)
    }
}

fn storage_usage() -> wgpu::BufferUsages {
    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST
}

fn layout_entry(number: u32, binding: Binding) -> wgpu::BindGroupLayoutEntry {
    let buffer_type = match binding {
        Binding::Uniform => wgpu::BufferBindingType::Uniform,
        Binding::Read => wgpu::BufferBindingType::Storage { read_only: true },
        Binding::ReadWrite => wgpu::BufferBindingType::Storage { read_only: false },
    };
    wgpu::BindGroupLayoutEntry {
        binding: number,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: buffer_type,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Dispatches and copies recorded for one device. Every
/// `DISPATCHES_PER_SUBMISSION` dispatches the batch hands its commands to
/// the GPU and waits for the hand-over before last, so the GPU always has
/// work queued but never far ahead of the recording.
pub struct Batch<'a> {
    gpu: &'a GpuDevice,
    encoder: wgpu::CommandEncoder,
    recorded: usize,
    in_flight: Option<wgpu::SubmissionIndex>,
}

impl Batch<'_> {
    /// Runs `kernel` over `workgroups` groups.
    pub fn dispatch(&mut self, kernel: &Kernel, group: &wgpu::BindGroup, workgroups: [u32; 3]) -> Result<(), GpuError> {
        if workgroups.contains(&0) {
            return Ok(());
        }
        let limit = self.gpu.limits.max_compute_workgroups_per_dimension;
        if workgroups.iter().any(|&count| count > limit) {
            return Err(GpuError::OutOfMemory(format!("{workgroups:?} workgroups are over the device's limit of {limit}")));
        }
        {
            let descriptor = wgpu::ComputePassDescriptor { label: None, timestamp_writes: None };
            let mut pass = self.encoder.begin_compute_pass(&descriptor);
            pass.set_pipeline(&kernel.pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(workgroups[0], workgroups[1], workgroups[2]);
        }
        self.recorded += 1;
        if self.recorded >= DISPATCHES_PER_SUBMISSION {
            self.submit()?;
        }
        Ok(())
    }

    /// One thread per element of a `count`-long launch (`GROUP_SIZE` to a
    /// group); launches beyond the dimension limit spread over y, which the
    /// prelude's `linear_index` undoes.
    pub fn dispatch_linear(&mut self, kernel: &Kernel, group: &wgpu::BindGroup, count: usize) -> Result<(), GpuError> {
        let groups = count.div_ceil(GROUP_SIZE as usize) as u64;
        let limit = self.gpu.limits.max_compute_workgroups_per_dimension.max(1) as u64;
        let columns = groups.min(limit);
        let rows = groups.div_ceil(columns.max(1));
        self.dispatch(kernel, group, [columns as u32, rows as u32, 1])
    }

    /// One thread per pixel of a `width × height` launch.
    pub fn dispatch_plane(&mut self, kernel: &Kernel, group: &wgpu::BindGroup, width: usize, height: usize) -> Result<(), GpuError> {
        let columns = width.div_ceil(GROUP_SIDE as usize) as u32;
        let rows = height.div_ceil(GROUP_SIDE as usize) as u32;
        self.dispatch(kernel, group, [columns, rows, 1])
    }

    /// Copies `bytes` from the start of `source` to the start of `target`.
    pub fn copy(&mut self, source: &wgpu::Buffer, target: &wgpu::Buffer, bytes: u64) {
        let size = bytes.next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
        self.encoder.copy_buffer_to_buffer(source, 0, target, 0, size);
    }

    /// Zeroes a whole buffer.
    pub fn clear(&mut self, buffer: &wgpu::Buffer) {
        self.encoder.clear_buffer(buffer, 0, None);
    }

    /// Hands what is recorded to the GPU.
    pub fn submit(&mut self) -> Result<(), GpuError> {
        let encoder = std::mem::replace(&mut self.encoder, self.gpu.encoder());
        let submission = self.gpu.queue.submit([encoder.finish()]);
        self.recorded = 0;
        if let Some(previous) = self.in_flight.replace(submission) {
            self.gpu.wait(Some(previous))?;
        }
        Ok(())
    }

    /// Hands everything to the GPU and waits for it to finish.
    pub fn finish(mut self) -> Result<(), GpuError> {
        self.submit()?;
        let last = self.in_flight.take();
        self.gpu.wait(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendors_come_from_ids_then_names() {
        assert_eq!(vendor_name(0, "Apple M2 Pro"), "Apple");
        assert_eq!(vendor_name(0x10DE, "NVIDIA GeForce RTX 4090"), "NVIDIA");
        assert_eq!(vendor_name(0, "AMD Radeon Pro 5500M"), "AMD");
        assert_eq!(vendor_name(0x1234, "Mystery"), "0x1234");
    }

    #[test]
    fn software_adapters_are_not_devices() {
        assert_eq!(device_type(wgpu::DeviceType::Cpu), None);
        assert_eq!(device_type(wgpu::DeviceType::IntegratedGpu), Some(DeviceType::GpuIntegrated));
        assert_eq!(backend_name(wgpu::Backend::Gl), None);
    }
}
