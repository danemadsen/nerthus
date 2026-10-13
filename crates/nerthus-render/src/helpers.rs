//! The viewport helpers: the ground grid (`SuperGrid`'s `Grid_Big`, a
//! `DecoGrid`) that runs far past the terrain, and the frame round the
//! terrain's footprint (`Place`, a `DecoPlace`). Both are textured planes
//! just under the terrain's base, added onto what is behind them (Unity's
//! `Particles/Additive (Soft)`). The textures are drawn here to the
//! measurements of the `Grid1` and `baseGrid` textures.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::NotShadowCaster;
use bevy::math::Affine2;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::terrain::ViewportSettings;

/// The measurements below take the terrain as 20 Unity units across;
/// Nerthus's spans one.
const UNITY_UNIT: f32 = 1.0 / 20.0;
/// `SuperGrid` sits 0.003 units under the terrain's base; the frame lies
/// between the two.
const GRID_DEPTH: f32 = -0.003 * UNITY_UNIT;
const BOUNDS_DEPTH: f32 = GRID_DEPTH * 0.5;
/// `Grid_Big` is 2000 units square with its tile repeated 400 times: a
/// tile is a quarter of the terrain, a cell a fortieth.
const GRID_SIZE: f32 = 2000.0 * UNITY_UNIT;
const GRID_REPEATS: f32 = 400.0;

/// The textures' side. The shapes below are measured in texels of 2048
/// square textures and drawn at half that.
const TEXTURE_SIDE: usize = 1024;
const DESIGN_SIDE: f32 = 2048.0;

/// `Grid1`: ten cells a tile, cell lines two texels wide at 25/255, the
/// tile's edge two texels at 51/255 on its near side, and the grey all
/// its lines share.
const GRID_GREY: u8 = 112;
const GRID_CELLS: usize = 10;
const CELL_LINE_WIDTH: f32 = 2.0;
const CELL_LINE_ALPHA: f32 = 25.0 / 255.0;
const TILE_EDGE_ALPHA: f32 = 51.0 / 255.0;

/// `baseGrid`: a white frame line 2.5 texels wide just inside the edge;
/// each corner has a bright bracket, a gap and a bright cap before the
/// line carries on dimmer.
const FRAME_INSET: f32 = 0.5;
const FRAME_WIDTH: f32 = 2.5;
const FRAME_ALPHA: f32 = 0.42;
const BRACKET_LENGTH: f32 = 17.0;
const CAP_START: f32 = 45.0;
const CAP_END: f32 = 53.0;
const BRACKET_ALPHA: f32 = 0.98;

/// Marks the ground grid.
#[derive(Component)]
pub struct GroundGrid;

/// Marks the frame round the terrain.
#[derive(Component)]
pub struct TerrainBounds;

pub(crate) fn spawn_helpers(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>, images: &mut Assets<Image>) {
    let grid_alpha = grid_tile_alpha(TEXTURE_SIDE);
    let grid_texture = mipmapped_texture(TEXTURE_SIDE, GRID_GREY, &grid_alpha, ImageAddressMode::Repeat);
    let grid_material = StandardMaterial {
        base_color_texture: Some(images.add(grid_texture)),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        cull_mode: None,
        fog_enabled: false,
        uv_transform: Affine2::from_scale(Vec2::splat(GRID_REPEATS)),
        ..default()
    };
    let grid_mesh = Plane3d::default().mesh().size(GRID_SIZE, GRID_SIZE);
    let grid_place = Transform::from_xyz(0.0, GRID_DEPTH, 0.0);
    let grid = (GroundGrid, Mesh3d(meshes.add(grid_mesh)), MeshMaterial3d(materials.add(grid_material)), grid_place, Visibility::Hidden, NotShadowCaster);
    commands.spawn(grid);

    let frame_alpha = frame_alpha(TEXTURE_SIDE);
    let frame_texture = mipmapped_texture(TEXTURE_SIDE, 255, &frame_alpha, ImageAddressMode::ClampToEdge);
    let frame_material = StandardMaterial {
        base_color_texture: Some(images.add(frame_texture)),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        cull_mode: None,
        fog_enabled: false,
        ..default()
    };
    let frame_mesh = Plane3d::default().mesh().size(1.0, 1.0);
    let frame_place = Transform::from_xyz(0.0, BOUNDS_DEPTH, 0.0);
    let frame = (TerrainBounds, Mesh3d(meshes.add(frame_mesh)), MeshMaterial3d(materials.add(frame_material)), frame_place, Visibility::Hidden, NotShadowCaster);
    commands.spawn(frame);
}

/// Shows or hides the grid and the frame (`tglDecoGrid`, `tglPlace`).
pub(crate) fn update_helpers(
    settings: Res<ViewportSettings>,
    turntable: Res<crate::turntable::Turntable>,
    mut grid: Single<&mut Visibility, With<GroundGrid>>,
    mut bounds: Single<&mut Visibility, (With<TerrainBounds>, Without<GroundGrid>)>,
) {
    if !settings.is_changed() && !turntable.is_changed() {
        return;
    }
    // A turntable recording may leave them out of its pictures.
    let hidden = turntable.hides_helpers();
    **grid = if settings.grid && !hidden { Visibility::Inherited } else { Visibility::Hidden };
    **bounds = if settings.bounds && !hidden { Visibility::Inherited } else { Visibility::Hidden };
}

/// How much of the texel `[start, start + size)` the span `[low, high)`
/// covers.
fn coverage(start: f32, size: f32, low: f32, high: f32) -> f32 {
    let overlap = (start + size).min(high) - start.max(low);
    overlap.max(0.0) / size
}

/// Paints `top` over `bottom` (both coverages of one colour).
fn over(bottom: f32, top: f32) -> f32 {
    bottom + top - bottom * top
}

/// One axis of the grid tile: each texel's cover by the cell lines and
/// the tile's edge (the last cell line, which ends the tile, sits just
/// inside its far side).
fn grid_profile(side: usize) -> Vec<f32> {
    let texel = DESIGN_SIDE / side as f32;
    let cell = DESIGN_SIDE / GRID_CELLS as f32;
    let mut profile = Vec::with_capacity(side);
    for index in 0..side {
        let start = index as f32 * texel;
        let mut alpha = coverage(start, texel, 0.0, CELL_LINE_WIDTH) * TILE_EDGE_ALPHA;
        let far_edge = coverage(start, texel, DESIGN_SIDE - CELL_LINE_WIDTH, DESIGN_SIDE) * CELL_LINE_ALPHA;
        alpha = over(alpha, far_edge);
        for line in 1..GRID_CELLS {
            let centre = line as f32 * cell;
            let low = centre - CELL_LINE_WIDTH / 2.0;
            let high = centre + CELL_LINE_WIDTH / 2.0;
            let cell_line = coverage(start, texel, low, high) * CELL_LINE_ALPHA;
            alpha = over(alpha, cell_line);
        }
        profile.push(alpha);
    }
    profile
}

/// The grid tile: the lines of both axes.
fn grid_tile_alpha(side: usize) -> Vec<f32> {
    let profile = grid_profile(side);
    let mut alpha = Vec::with_capacity(side * side);
    for row in &profile {
        for column in &profile {
            alpha.push(over(*row, *column));
        }
    }
    alpha
}

/// A run along one side of the frame: where it starts and ends along the
/// side, and how bright it is.
struct FrameRun {
    from: f32,
    to: f32,
    alpha: f32,
}

/// The runs along each side: a bracket at each corner, then after a gap
/// a cap and the line.
fn frame_runs() -> [FrameRun; 5] {
    [
        FrameRun { from: 0.0, to: BRACKET_LENGTH, alpha: BRACKET_ALPHA },
        FrameRun { from: CAP_START, to: CAP_END, alpha: BRACKET_ALPHA },
        FrameRun { from: CAP_END, to: DESIGN_SIDE - CAP_END, alpha: FRAME_ALPHA },
        FrameRun { from: DESIGN_SIDE - CAP_END, to: DESIGN_SIDE - CAP_START, alpha: BRACKET_ALPHA },
        FrameRun { from: DESIGN_SIDE - BRACKET_LENGTH, to: DESIGN_SIDE, alpha: BRACKET_ALPHA },
    ]
}

/// The frame: each side's runs, inset from the edge.
fn frame_alpha(side: usize) -> Vec<f32> {
    let texel = DESIGN_SIDE / side as f32;
    let runs = frame_runs();
    let near_edge = (FRAME_INSET, FRAME_INSET + FRAME_WIDTH);
    let far_edge = (DESIGN_SIDE - FRAME_INSET - FRAME_WIDTH, DESIGN_SIDE - FRAME_INSET);
    let mut alpha = vec![0.0; side * side];
    for row in 0..side {
        let row_start = row as f32 * texel;
        for column in 0..side {
            let column_start = column as f32 * texel;
            let mut value: f32 = 0.0;
            for (edge_low, edge_high) in [near_edge, far_edge] {
                // Down the left and right sides, then along the top and
                // bottom.
                let across_vertical = coverage(column_start, texel, edge_low, edge_high);
                let across_horizontal = coverage(row_start, texel, edge_low, edge_high);
                if across_vertical == 0.0 && across_horizontal == 0.0 {
                    continue;
                }
                for run in &runs {
                    let along_vertical = coverage(row_start, texel, run.from, run.to);
                    let along_horizontal = coverage(column_start, texel, run.from, run.to);
                    let vertical = across_vertical * along_vertical * run.alpha;
                    let horizontal = across_horizontal * along_horizontal * run.alpha;
                    value = value.max(vertical).max(horizontal);
                }
            }
            alpha[row * side + column] = value;
        }
    }
    alpha
}

/// An sRGB texture of one grey at the given coverage, with its mip chain
/// (each level the average of the one above) and anisotropic filtering,
/// as Unity samples its textures.
fn mipmapped_texture(side: usize, grey: u8, alpha: &[f32], address_mode: ImageAddressMode) -> Image {
    let mut levels: Vec<Vec<f32>> = vec![alpha.to_vec()];
    let mut level_side = side;
    while level_side > 1 {
        let next_side = level_side / 2;
        let Some(previous) = levels.last() else {
            break;
        };
        let mut next = Vec::with_capacity(next_side * next_side);
        for row in 0..next_side {
            for column in 0..next_side {
                let top = 2 * row * level_side + 2 * column;
                let bottom = top + level_side;
                let sum = previous[top] + previous[top + 1] + previous[bottom] + previous[bottom + 1];
                next.push(sum / 4.0);
            }
        }
        levels.push(next);
        level_side = next_side;
    }

    let mut data = Vec::new();
    for level in &levels {
        for value in level {
            let alpha_byte = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
            data.extend_from_slice(&[grey, grey, grey, alpha_byte]);
        }
    }
    let extent = Extent3d {
        width: side as u32,
        height: side as u32,
        depth_or_array_layers: 1,
    };
    let mut image = Image::new_uninit(extent, TextureDimension::D2, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels.len() as u32;
    let sampler = ImageSamplerDescriptor {
        address_mode_u: address_mode,
        address_mode_v: address_mode,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 16,
        ..default()
    };
    image.sampler = ImageSampler::Descriptor(sampler);
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_lines_follow_grid1() {
        let profile = grid_profile(2048);
        // The tile's edge, then the first cell line round 204.8.
        assert!((profile[0] - TILE_EDGE_ALPHA).abs() < 1e-6);
        assert!((profile[1] - TILE_EDGE_ALPHA).abs() < 1e-6);
        assert_eq!(profile[2], 0.0);
        assert!((profile[2047] - CELL_LINE_ALPHA).abs() < 1e-6);
        let first_line: f32 = profile[203..207].iter().sum();
        assert!((first_line - 2.0 * CELL_LINE_ALPHA).abs() < 1e-5);
        assert_eq!(profile[300], 0.0);
    }

    #[test]
    fn frame_has_corner_brackets() {
        let side = 256;
        let alpha = frame_alpha(side);
        let at = |column: usize, row: usize| alpha[row * side + column];
        // Down the left side: the bracket, the gap, the cap, the line.
        assert!(at(0, 1) > 0.2);
        assert_eq!(at(0, 4), 0.0);
        assert!(at(0, 6) > at(0, 50));
        assert!(at(0, 50) > 0.05);
        assert_eq!(at(50, 50), 0.0);
        // The frame is the same on every side.
        assert!((at(side - 1, 50) - at(0, 50)).abs() < 1e-6);
        assert!((at(50, 0) - at(50, side - 1)).abs() < 1e-6);
    }

    #[test]
    fn mip_chain_averages() {
        let alpha = vec![1.0, 0.0, 0.0, 0.0];
        let image = mipmapped_texture(2, 112, &alpha, ImageAddressMode::Repeat);
        assert_eq!(image.texture_descriptor.mip_level_count, 2);
        let data = image.data.unwrap();
        assert_eq!(data.len(), 4 * 5);
        assert_eq!(&data[16..20], &[112, 112, 112, 64]);
    }
}
