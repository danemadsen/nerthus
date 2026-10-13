//! The previewed terrain: a heightfield mesh draped with the colours the
//! viewport renderer baked. The tessellation surface emits its
//! `_Albedo` texture, so the lighting, shadows and occlusion are in the
//! colours and the mesh is drawn unlit; its underside is `_Backface` grey.

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::render::render_resource::{Extent3d, Face, TextureDimension, TextureFormat};
use nerthus_engine::{Map, Resolution};

/// Largest number of vertices per mesh side; bigger maps are sampled.
const MAX_MESH_SIDE: usize = 1024;

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

/// The terrain revision the current mesh was built from.
#[derive(Resource, Default)]
pub(crate) struct BuiltTerrain {
    revision: u64,
}

pub(crate) fn spawn_terrain(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) {
    let flat = Map::new(Resolution::square(2), 1);
    let mesh = meshes.add(build_mesh(&flat, 0.0, false));
    let surface_material = StandardMaterial {
        base_color: UNRENDERED,
        unlit: true,
        ..default()
    };
    let backface_material = StandardMaterial {
        base_color: BACKFACE,
        unlit: true,
        cull_mode: Some(Face::Front),
        ..default()
    };
    let surface = (TerrainSurface, Mesh3d(mesh.clone()), MeshMaterial3d(materials.add(surface_material)), Transform::default(), NotShadowCaster);
    let backface = (TerrainBackface, Mesh3d(mesh), MeshMaterial3d(materials.add(backface_material)), Transform::default(), NotShadowCaster);
    commands.spawn(surface);
    commands.spawn(backface);
}

/// Rebuilds the terrain mesh and texture when the preview changes.
pub(crate) fn rebuild_terrain(
    preview: Res<TerrainPreview>,
    mut built: ResMut<BuiltTerrain>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    terrain: Single<(&Mesh3d, &MeshMaterial3d<StandardMaterial>), With<TerrainSurface>>,
) {
    if built.revision == preview.revision {
        return;
    }
    built.revision = preview.revision;

    let (mesh_handle, material_handle) = *terrain;
    let flat = Arc::new(Map::new(Resolution::square(2), 1));
    let heights = preview.heights.clone().unwrap_or(flat);
    let mesh = build_mesh(&heights, preview.height_scale(), preview.tiled);
    if let Some(mut existing) = meshes.get_mut(&mesh_handle.0) {
        *existing = mesh;
    }

    let Some(mut material) = materials.get_mut(&material_handle.0) else {
        return;
    };
    match &preview.colours {
        Some(colours) => {
            let texture = colour_texture(colours, preview.tiled);
            material.base_color_texture = Some(images.add(texture));
            material.base_color = Color::WHITE;
        }
        None => {
            material.base_color_texture = None;
            material.base_color = UNRENDERED;
        }
    }
}

/// A grid mesh over the unit square centred on the origin (X right, Z down
/// the rows, so the map's top row is north at −Z), heights (the first
/// channel) scaled by `height_scale`. Each vertex samples its texel's
/// centre, as `PlaneX`'s UVs do. Tiled, the mesh spans the map twice each
/// way, its UVs running to 2 for the wrapping texture.
fn build_mesh(heights: &Map, height_scale: f32, tiled: bool) -> Mesh {
    let map_width = heights.width();
    let map_height = heights.height();
    let repeats = if tiled { 2 } else { 1 };
    let span_width = map_width * repeats;
    let span_height = map_height * repeats;
    let longest = span_width.max(span_height).max(2);
    let step = longest.div_ceil(MAX_MESH_SIDE).max(1);
    let columns = ((span_width.max(2) - 1) / step + 1).max(2);
    let rows = ((span_height.max(2) - 1) / step + 1).max(2);
    let last_column = (columns - 1) as f32;
    let last_row = (rows - 1) as f32;
    let plane = heights.channel(0);
    let span_x = |column: usize| (column * step).min(span_width - 1);
    let span_y = |row: usize| (row * step).min(span_height - 1);
    let sample = |column: usize, row: usize| -> f32 {
        let map_x = span_x(column) % map_width;
        let map_y = span_y(row) % map_height;
        plane[map_y * map_width + map_x] * height_scale
    };

    let mut positions = Vec::with_capacity(columns * rows);
    let mut uvs = Vec::with_capacity(columns * rows);
    for row in 0..rows {
        for column in 0..columns {
            let x = column as f32 / last_column - 0.5;
            let z = row as f32 / last_row - 0.5;
            positions.push([x, sample(column, row), z]);
            let u = (span_x(column) as f32 + 0.5) / map_width as f32;
            let v = (span_y(row) as f32 + 0.5) / map_height as f32;
            uvs.push([u, v]);
        }
    }

    let column_spacing = 1.0 / last_column;
    let row_spacing = 1.0 / last_row;
    let mut normals = Vec::with_capacity(columns * rows);
    for row in 0..rows {
        for column in 0..columns {
            let previous_column = column.saturating_sub(1);
            let next_column = (column + 1).min(columns - 1);
            let previous_row = row.saturating_sub(1);
            let next_row = (row + 1).min(rows - 1);
            let left = sample(previous_column, row);
            let right = sample(next_column, row);
            let up = sample(column, previous_row);
            let down = sample(column, next_row);
            let horizontal_span = column_spacing * (next_column - previous_column) as f32;
            let vertical_span = row_spacing * (next_row - previous_row) as f32;
            let normal = Vec3::new(-(right - left) / horizontal_span, 1.0, -(down - up) / vertical_span);
            normals.push(normal.normalize().to_array());
        }
    }

    let mut indices = Vec::with_capacity((columns - 1) * (rows - 1) * 6);
    for row in 0..rows - 1 {
        for column in 0..columns - 1 {
            let corner = (row * columns + column) as u32;
            let below = corner + columns as u32;
            indices.extend_from_slice(&[corner, below, corner + 1, corner + 1, below, below + 1]);
        }
    }

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
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
    fn vertices_sample_texel_centres() {
        let heights = Map::from_data(Resolution::square(4), (0..16).map(|value| value as f32).collect());
        let mesh = build_mesh(&heights, 1.0, false);
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
        assert!((x - 1.0 / 6.0).abs() < 1e-6 && y == 6.0 && (z + 1.0 / 6.0).abs() < 1e-6);
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
