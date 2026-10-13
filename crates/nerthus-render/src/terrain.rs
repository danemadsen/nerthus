//! The previewed terrain: a grid lifted by the height map as it draws
//! (`PlaneX` with `_DisplacementTex`), draped with the colours the viewport
//! renderer baked. The surface emits its `_Albedo` texture, so the
//! lighting, shadows and occlusion are in the colours and the mesh is drawn
//! unlit; its underside is `_Backface` grey.
//!
//! The grid has a vertex for each texel of the preview (`ResizeTerrain`:
//! 512 to 4096 a side), except that Conserve memory when displaying 4K
//! terrains draws a 4K terrain with the 2K grid.

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::render::render_resource::{AsBindGroup, Extent3d, Face, ShaderType, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;
use nerthus_engine::{Map, Resolution};

/// The largest grid (`PlaneX(4096, 4096)`), and the one a 4K terrain gets
/// while conserving memory (`preventHiMesh`).
const MAX_MESH_SIDE: usize = 4096;
const CONSERVING_MESH_SIDE: usize = 2048;

/// The vertex stage that lifts the grid (`terrain.wesl`).
const TERRAIN_SHADER: &str = "embedded://nerthus_render/terrain.wesl";

/// The terrain's material: the standard one, unlit, with the height map
/// and its shape for the vertex stage.
pub(crate) type TerrainMaterial = ExtendedMaterial<StandardMaterial, TerrainHeights>;

/// What the vertex stage reads: the height map (first channel, 32-bit
/// float) and its shape.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub(crate) struct TerrainHeights {
    #[uniform(100)]
    shape: TerrainShape,
    #[texture(101, sample_type = "float", filterable = false)]
    heights: Handle<Image>,
}

/// How high a height of 1 stands, the map's size in texels, and whether
/// the grid spans the map twice each way (1) or once (0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect, ShaderType)]
struct TerrainShape {
    height_scale: f32,
    width: u32,
    height: u32,
    tiled: u32,
}

impl MaterialExtension for TerrainHeights {
    fn vertex_shader() -> ShaderRef {
        TERRAIN_SHADER.into()
    }
}

/// The terrain before anything has been rendered.
const UNRENDERED: Color = Color::srgb(0.5, 0.5, 0.5);
/// `_Backface`: the terrain seen from below.
const BACKFACE: Color = Color::srgb(0.0377, 0.0377, 0.0377);

/// The colours draped over the terrain: RGB bytes, row by row from the
/// top (`_Albedo`, an RGB24 sRGB texture).
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainColours {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

/// What the viewport shows. Bump `revision` after changing it.
#[derive(Resource)]
pub struct TerrainPreview {
    pub heights: Option<Arc<Map>>,
    /// What the viewport renderer made of the preview; the terrain is grey
    /// until there is some.
    pub colours: Option<Arc<TerrainColours>>,
    /// The terrain's size in metres (`Terrain.Width`, `Terrain.Height`):
    /// a height of 1 stands `height / width` of the terrain's width tall.
    pub width: f32,
    pub height: f32,
    /// Seamless's Preview Tiling (`Comms.ToggleTiling`): the map and its
    /// colours repeat twice each way across the terrain (texture scale 2,
    /// wrapping).
    pub tiled: bool,
    /// Conserve memory when displaying 4K terrains (`PreventHiRes`): a 4K
    /// map is drawn with the 2K grid.
    pub conserve_memory: bool,
    pub revision: u64,
}

impl Default for TerrainPreview {
    fn default() -> Self {
        Self {
            heights: None,
            colours: None,
            width: 5000.0,
            height: 2500.0,
            tiled: false,
            conserve_memory: false,
            revision: 0,
        }
    }
}

/// Viewport presentation settings.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct ViewportSettings {
    /// The lighting panel's `Show Skybox`: the sky behind the terrain;
    /// off, the viewport background.
    pub skybox: bool,
    /// `Viewport Elements`: the ground grid, the terrain's bounds and the
    /// navigation compass.
    pub grid: bool,
    pub bounds: bool,
    pub compass: bool,
    /// Behind the terrain when there is no sky (`Options.ViewportBackground`,
    /// sRGB).
    pub background: [f32; 3],
    /// The lighting's sun (degrees clockwise from north, and above the
    /// horizon), which the sky is lit by, and its `Fog Density`.
    pub sun_azimuth: f32,
    pub sun_elevation: f32,
    pub fog_density: f32,
}

impl Default for ViewportSettings {
    fn default() -> Self {
        Self {
            skybox: false,
            grid: false,
            bounds: false,
            compass: false,
            background: [0.01960784; 3],
            sun_azimuth: 45.0,
            sun_elevation: 33.0,
            fog_density: 0.2,
        }
    }
}

impl TerrainPreview {
    /// World height of a height of 1 (the terrain spans one unit), as
    /// `Comms.SetHeight(Terrain.Ratio)` sets it.
    pub fn height_scale(&self) -> f32 {
        self.height / self.width.max(1.0)
    }
}

/// Marks the terrain mesh entity.
#[derive(Component)]
pub struct TerrainSurface;

/// Marks the terrain's underside, drawn from the same mesh.
#[derive(Component)]
pub struct TerrainBackface;

/// What the terrain was last drawn from: the preview's revision, and the
/// grid's side and tiling.
#[derive(Resource, Default)]
pub(crate) struct BuiltTerrain {
    revision: u64,
    grid: Option<(usize, bool)>,
}

pub(crate) fn spawn_terrain(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<TerrainMaterial>, images: &mut Assets<Image>) {
    let mesh = meshes.add(grid_mesh(2, false));
    let flat = Map::new(Resolution::square(2), 1);
    let heights = TerrainHeights {
        shape: TerrainShape { height_scale: 0.0, width: 2, height: 2, tiled: 0 },
        heights: images.add(height_texture(&flat)),
    };
    let surface_material = TerrainMaterial {
        base: StandardMaterial {
            base_color: UNRENDERED,
            unlit: true,
            ..default()
        },
        extension: heights.clone(),
    };
    let backface_material = TerrainMaterial {
        base: StandardMaterial {
            base_color: BACKFACE,
            unlit: true,
            cull_mode: Some(Face::Front),
            ..default()
        },
        extension: heights,
    };
    // The grid's bounds are flat; the heights come later, so the terrain is
    // never culled.
    let surface = (TerrainSurface, Mesh3d(mesh.clone()), MeshMaterial3d(materials.add(surface_material)), Transform::default(), NotShadowCaster, NoFrustumCulling);
    let backface = (TerrainBackface, Mesh3d(mesh), MeshMaterial3d(materials.add(backface_material)), Transform::default(), NotShadowCaster, NoFrustumCulling);
    commands.spawn(surface);
    commands.spawn(backface);
}

/// `ResizeTerrain`: the grid's side for a map, a vertex a texel up to
/// 4096, 2048 for a 4K map while conserving memory.
fn grid_side(heights: &Map, conserve_memory: bool) -> usize {
    let side = heights.width().max(heights.height()).clamp(2, MAX_MESH_SIDE);
    if conserve_memory && side >= MAX_MESH_SIDE {
        return CONSERVING_MESH_SIDE;
    }
    side
}

/// Puts the preview on the terrain when it changes: a new grid when its
/// size or tiling changes, the heights and colours each time.
pub(crate) fn rebuild_terrain(
    preview: Res<TerrainPreview>,
    mut built: ResMut<BuiltTerrain>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
    mut images: ResMut<Assets<Image>>,
    surface: Single<(&Mesh3d, &MeshMaterial3d<TerrainMaterial>), With<TerrainSurface>>,
    backface: Single<&MeshMaterial3d<TerrainMaterial>, With<TerrainBackface>>,
) {
    let flat = Arc::new(Map::new(Resolution::square(2), 1));
    let heights = preview.heights.clone().unwrap_or(flat);
    let grid = (grid_side(&heights, preview.conserve_memory), preview.tiled);
    if built.revision == preview.revision && built.grid == Some(grid) {
        return;
    }
    built.revision = preview.revision;

    let (mesh_handle, surface_material) = *surface;
    // Both sides draw the one grid.
    if built.grid != Some(grid)
        && let Some(mut existing) = meshes.get_mut(&mesh_handle.0)
    {
        *existing = grid_mesh(grid.0, grid.1);
    }
    built.grid = Some(grid);

    let shape = TerrainShape {
        height_scale: preview.height_scale(),
        width: heights.width() as u32,
        height: heights.height() as u32,
        tiled: u32::from(preview.tiled),
    };
    let height_image = images.add(height_texture(&heights));
    if let Some(mut material) = materials.get_mut(&backface.0) {
        material.extension.shape = shape;
        material.extension.heights = height_image.clone();
    }
    let Some(mut material) = materials.get_mut(&surface_material.0) else {
        return;
    };
    material.extension.shape = shape;
    material.extension.heights = height_image;
    match &preview.colours {
        Some(colours) => {
            let texture = colour_texture(colours, preview.tiled);
            material.base.base_color_texture = Some(images.add(texture));
            material.base.base_color = Color::WHITE;
        }
        None => {
            material.base.base_color_texture = None;
            material.base.base_color = UNRENDERED;
        }
    }
}

/// The height map's first channel as a 32-bit float texture.
fn height_texture(heights: &Map) -> Image {
    let plane = heights.channel(0);
    let mut bytes = Vec::with_capacity(plane.len() * 4);
    for value in plane {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let extent = Extent3d {
        width: heights.width() as u32,
        height: heights.height() as u32,
        depth_or_array_layers: 1,
    };
    Image::new(extent, TextureDimension::D2, bytes, TextureFormat::R32Float, RenderAssetUsages::RENDER_WORLD)
}

/// `PlaneX`: a grid of `side` by `side` vertices over the unit square
/// centred on the origin (X right, Z down the rows, so the map's top row
/// is north at −Z), flat; the vertex stage lifts it. Each vertex's
/// coordinates are its cell's centre, `(i + 0.5) / side`, running to 2 when
/// tiled (the map and its colours repeat twice each way).
fn grid_mesh(side: usize, tiled: bool) -> Mesh {
    let side = side.max(2);
    let last = (side - 1) as f32;
    let repeats = if tiled { 2.0 } else { 1.0 };
    let mut positions = Vec::with_capacity(side * side);
    let mut uvs = Vec::with_capacity(side * side);
    for row in 0..side {
        for column in 0..side {
            positions.push([column as f32 / last - 0.5, 0.0, row as f32 / last - 0.5]);
            let u = repeats * (column as f32 + 0.5) / side as f32;
            let v = repeats * (row as f32 + 0.5) / side as f32;
            uvs.push([u, v]);
        }
    }
    let mut indices = Vec::with_capacity((side - 1) * (side - 1) * 6);
    for row in 0..side - 1 {
        for column in 0..side - 1 {
            let corner = (row * side + column) as u32;
            let below = corner + side as u32;
            indices.extend_from_slice(&[corner, below, corner + 1, corner + 1, below, below + 1]);
        }
    }
    // Only the GPU keeps the grid.
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// The sRGB texture of the baked colours, wrapping when tiled.
fn colour_texture(colours: &TerrainColours, tiled: bool) -> Image {
    let pixel_count = colours.width * colours.height;
    let mut bytes = Vec::with_capacity(pixel_count * 4);
    let (pixels, _) = colours.rgb.as_chunks::<3>();
    for pixel in pixels.iter().take(pixel_count) {
        bytes.extend_from_slice(pixel);
        bytes.push(255);
    }
    let extent = Extent3d {
        width: colours.width as u32,
        height: colours.height as u32,
        depth_or_array_layers: 1,
    };
    let mut image = Image::new(extent, TextureDimension::D2, bytes, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::default());
    if tiled {
        let sampler = ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            ..default()
        };
        image.sampler = ImageSampler::Descriptor(sampler);
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertices_sit_on_cell_centres() {
        let mesh = grid_mesh(4, false);
        let Some(bevy::mesh::VertexAttributeValues::Float32x2(uvs)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) else {
            panic!("the mesh has UVs");
        };
        assert_eq!(uvs[0], [0.125, 0.125]);
        assert_eq!(uvs[15], [0.875, 0.875]);
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
            panic!("the mesh has positions");
        };
        assert_eq!(positions[0], [-0.5, 0.0, -0.5]);
        let [x, y, z] = positions[6];
        assert!((x - 1.0 / 6.0).abs() < 1e-6 && y == 0.0 && (z + 1.0 / 6.0).abs() < 1e-6);
        let tiled = grid_mesh(4, true);
        let Some(bevy::mesh::VertexAttributeValues::Float32x2(tiled_uvs)) = tiled.attribute(Mesh::ATTRIBUTE_UV_0) else {
            panic!("the mesh has UVs");
        };
        assert_eq!(tiled_uvs[15], [1.75, 1.75]);
    }

    #[test]
    fn grids_follow_the_map_up_to_4k() {
        let map = |side: usize| Map::new(Resolution::square(side), 1);
        assert_eq!(grid_side(&map(512), false), 512);
        assert_eq!(grid_side(&map(1024), true), 1024);
        assert_eq!(grid_side(&map(2048), true), 2048);
        assert_eq!(grid_side(&map(4096), false), 4096);
        assert_eq!(grid_side(&map(4096), true), 2048);
        assert_eq!(grid_side(&map(8192), false), 4096);
    }

    #[test]
    fn heights_go_up_as_floats() {
        let heights = Map::from_data(Resolution::square(2), vec![0.0, 0.25, 0.5, 1.0]);
        let texture = height_texture(&heights);
        assert_eq!(texture.texture_descriptor.format, TextureFormat::R32Float);
        assert_eq!(texture.data.as_deref().map(<[u8]>::len), Some(16));
        assert_eq!(texture.data.as_deref().map(|bytes| &bytes[4..8]), Some(&0.25f32.to_le_bytes()[..]));
    }

    #[test]
    fn colour_texture_pads_alpha() {
        let colours = TerrainColours {
            width: 2,
            height: 1,
            rgb: vec![1, 2, 3, 4, 5, 6],
        };
        let texture = colour_texture(&colours, false);
        assert_eq!(texture.data.as_deref(), Some(&[1, 2, 3, 255, 4, 5, 6, 255][..]));
    }
}
