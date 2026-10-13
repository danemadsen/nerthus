//! `Cellular3D_Render` on a GPU: [`crate::cellular3d`]'s four kernels, the
//! grid laid out on the CPU as the SYCL host lays it out.

use bytemuck::{Pod, Zeroable};
use nerthus_compute::{Binding, Entry, GpuDevice, GpuError, Shader};
use nerthus_engine::Map;

use crate::cellular3d::{Cellular3dSettings, Grid};

const SHADER: Shader = Shader {
    name: "cellular3d",
    source: include_str!("shaders/cellular3d.wgsl"),
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
    ],
    entries: &[
        Entry { name: "make_cells", bindings: &[0, 1, 2, 3, 4] },
        Entry { name: "find_tops", bindings: &[0, 3, 5] },
        Entry { name: "dilate_tops", bindings: &[0, 5, 6] },
        Entry { name: "render", bindings: &[0, 3, 4, 6, 7, 8] },
    ],
};

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct Parameters {
    columns: u32,
    rows: u32,
    layers: u32,
    top_layer: i32,
    width: u32,
    height: u32,
    seed: u32,
    use_gap_map: u32,
    cells_per_pixel_x: f32,
    cells_per_pixel_y: f32,
    pixels_per_cell_x: f32,
    pixels_per_cell_y: f32,
    height_per_cell: f32,
    gap: f32,
    jitter_x: f32,
    jitter_y: f32,
    jitter_z: f32,
    padding: [u32; 3],
}

/// `Cellular3D_Render` on `gpu`: the cell surface and each pixel's cell
/// hash.
pub fn render(gpu: &GpuDevice, heights: &Map, gap_map: Option<&Map>, settings: &Cellular3dSettings) -> Result<(Map, Map), GpuError> {
    let resolution = heights.resolution();
    let width = resolution.x;
    let height = resolution.y;
    let grid = Grid::new(resolution, settings);
    let cell_count = grid.len();
    let column_count = grid.columns * grid.rows;

    let parameters = Parameters {
        columns: grid.columns as u32,
        rows: grid.rows as u32,
        layers: grid.layers as u32,
        top_layer: grid.top_layer,
        width: width as u32,
        height: height as u32,
        seed: settings.seed,
        use_gap_map: u32::from(gap_map.is_some()),
        cells_per_pixel_x: grid.cells_per_pixel[0],
        cells_per_pixel_y: grid.cells_per_pixel[1],
        pixels_per_cell_x: grid.pixels_per_cell[0],
        pixels_per_cell_y: grid.pixels_per_cell[1],
        height_per_cell: grid.height_per_cell,
        gap: settings.gap,
        jitter_x: settings.jitter[0],
        jitter_y: settings.jitter[1],
        jitter_z: settings.jitter[2],
        padding: [0; 3],
    };
    let uniform = gpu.uniform(&parameters);
    let height_buffer = gpu.upload("cellular heights", heights.channel(0))?;
    let gap_buffer = match gap_map {
        Some(map) => Some(gpu.upload("cellular gaps", map.channel(0))?),
        None => None,
    };
    let gap_binding = gap_buffer.as_ref().unwrap_or(gpu.spare());
    let cells = gpu.zeroed("cells", cell_count as u64 * 16)?;
    let hashes = gpu.zeroed_words("cell hashes", cell_count)?;
    let tops = gpu.zeroed_words("column tops", column_count)?;
    let dilated = gpu.zeroed_words("spread tops", column_count)?;
    let surface = gpu.zeroed_words("cell surface", width * height)?;
    let pixel_hashes = gpu.zeroed_words("pixel hashes", width * height)?;

    let make_cells = gpu.kernel(&SHADER, "make_cells")?;
    let find_tops = gpu.kernel(&SHADER, "find_tops")?;
    let dilate_tops = gpu.kernel(&SHADER, "dilate_tops")?;
    let render = gpu.kernel(&SHADER, "render")?;
    let mut batch = gpu.batch();
    let group = gpu.bind(&make_cells, &[&uniform, &height_buffer, gap_binding, &cells, &hashes]);
    batch.dispatch_linear(&make_cells, &group, cell_count)?;
    let group = gpu.bind(&find_tops, &[&uniform, &cells, &tops]);
    batch.dispatch_linear(&find_tops, &group, column_count)?;
    let group = gpu.bind(&dilate_tops, &[&uniform, &tops, &dilated]);
    batch.dispatch_plane(&dilate_tops, &group, grid.columns, grid.rows)?;
    let group = gpu.bind(&render, &[&uniform, &cells, &hashes, &dilated, &surface, &pixel_hashes]);
    batch.dispatch_plane(&render, &group, width, height)?;
    batch.finish()?;

    let surface_data: Vec<f32> = gpu.read(&surface, width * height)?;
    let hash_data: Vec<f32> = gpu.read(&pixel_hashes, width * height)?;
    Ok((Map::from_data(resolution, surface_data), Map::from_data(resolution, hash_data)))
}
