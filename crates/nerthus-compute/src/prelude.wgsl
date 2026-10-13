// Shared by every Nerthus kernel module (GpuDevice puts it in front of
// each one).

// Threads per workgroup of one-dimensional launches, and along each side
// of a two-dimensional launch's workgroup (gpu.rs GROUP_SIZE, GROUP_SIDE).
const GROUP_SIZE: u32 = 256u;
const GROUP_SIDE: u32 = 16u;

// The element a thread of a one-dimensional launch works on: launches
// longer than a dispatch dimension holds continue on further rows of
// workgroups (Batch::dispatch_linear).
fn linear_index(id: vec3<u32>, groups: vec3<u32>) -> u32 {
    return id.y * groups.x * GROUP_SIZE + id.x;
}
