//! Surface texturing from minimal curvature (`Surfacer`): the rocky detail
//! that GroundTexture, Snowfield and Dusting screen over a terrain.

use nerthus_engine::Map;
use rayon::prelude::*;

use crate::maths;

/// `m_cellLength`: the cell spacing the curvature is measured over.
const CELL_LENGTH: f32 = 5.0;

/// Curvature at or below this counts as fully rocky.
const ROCKY_LIMIT: f32 = 0.025;

/// `FMath.SafeDiv` with its default epsilon.
fn safe_div(numerator: f32, denominator: f32) -> f32 {
    if denominator.abs() < 1e-18 {
        return 0.0;
    }
    numerator / denominator
}

/// `FMath.SafeSqrt`.
fn safe_sqrt(value: f32) -> f32 {
    if value <= 0.0 {
        return 0.0;
    }
    value.sqrt()
}

/// First derivatives `(zx, zy)` and second derivatives `(zxx, zyy, zxy)`.
struct Derivatives {
    zx: f32,
    zy: f32,
    zxx: f32,
    zyy: f32,
    zxy: f32,
}

/// `GetDerivatives`: Evans-Young derivatives of the 3x3 neighbourhood in
/// metres, negated.
fn derivatives(data: &[f32], width: usize, x: usize, y: usize, terrain_height: f32) -> Derivatives {
    let height_at = |column: usize, row: usize| data[column + row * width] * terrain_height;

    let cell_area = CELL_LENGTH * CELL_LENGTH;
    let h1 = height_at(x - 1, y + 1);
    let h2 = height_at(x, y + 1);
    let h3 = height_at(x + 1, y + 1);
    let h4 = height_at(x - 1, y);
    let h5 = height_at(x, y);
    let h6 = height_at(x + 1, y);
    let h7 = height_at(x - 1, y - 1);
    let h8 = height_at(x, y - 1);
    let h9 = height_at(x + 1, y - 1);

    let zx = (h3 + h6 + h9 - h1 - h4 - h7) / (6.0 * CELL_LENGTH);
    let zy = (h1 + h2 + h3 - h7 - h8 - h9) / (6.0 * CELL_LENGTH);
    let zxx = (h1 + h3 + h4 + h6 + h7 + h9 - 2.0 * (h2 + h5 + h8)) / (3.0 * cell_area);
    let zyy = (h1 + h2 + h3 + h7 + h8 + h9 - 2.0 * (h4 + h5 + h6)) / (3.0 * cell_area);
    let zxy = (h3 + h7 - h1 - h9) / (4.0 * cell_area);
    Derivatives {
        zx: -zx,
        zy: -zy,
        zxx: -zxx,
        zyy: -zyy,
        zxy: -zxy,
    }
}

/// `MeanCurvature`.
fn mean_curvature(d: &Derivatives) -> f32 {
    let zx_squared = d.zx * d.zx;
    let zy_squared = d.zy * d.zy;
    let gradient_squared = zx_squared + zy_squared;
    let numerator = (1.0 + zy_squared) * d.zxx - 2.0 * d.zxy * d.zx * d.zy + (1.0 + zx_squared) * d.zyy;
    let denominator = 2.0 * maths::pow(gradient_squared + 1.0, 1.5);
    safe_div(numerator, denominator)
}

/// `GaussianCurvature`.
fn gaussian_curvature(d: &Derivatives) -> f32 {
    let zx_squared = d.zx * d.zx;
    let zy_squared = d.zy * d.zy;
    let gradient_squared = zx_squared + zy_squared;
    let numerator = d.zxx * d.zyy - d.zxy * d.zxy;
    let denominator = maths::pow(gradient_squared + 1.0, 2.0);
    safe_div(numerator, denominator)
}

/// `MinimalCurvature`.
fn minimal_curvature(d: &Derivatives) -> f32 {
    let mean = mean_curvature(d);
    let gaussian = gaussian_curvature(d);
    mean - safe_sqrt(mean * mean - gaussian)
}

/// `Surfacer.Rocky`: 1 where the minimal curvature is near flat or
/// convex, falling off over `falloff` either side; `density` then
/// strengthens the result. The one-cell border stays 0.
pub fn rocky(heights: &Map, falloff: f32, density: f32, terrain_height: f32) -> Map {
    // `m_terrainHeight` is an int.
    let terrain_height = terrain_height as i32 as f32;
    let falloff_max = ROCKY_LIMIT + falloff;
    let falloff_min = 0.0 - falloff;
    let min_factor = 0.0 - falloff_min;
    let max_factor = falloff_max - ROCKY_LIMIT;

    let resolution = heights.resolution();
    let width = resolution.x;
    let height = resolution.y;
    let data = heights.channel(0);

    let mut curve_map = Map::new(resolution, 1);
    let interior = curve_map.data_mut().par_chunks_mut(width).enumerate().skip(1).take(height.saturating_sub(2));
    interior.for_each(|(y, row)| {
        for (x, value) in row.iter_mut().enumerate().take(width - 1).skip(1) {
            let d = derivatives(data, width, x, y, terrain_height);
            let curvature = minimal_curvature(&d);
            if curvature >= 0.0 {
                if curvature <= ROCKY_LIMIT {
                    *value = 1.0;
                } else if curvature < falloff_max {
                    *value = 1.0 - (curvature - ROCKY_LIMIT) / max_factor;
                }
            } else if curvature > falloff_min {
                *value = (curvature - falloff_min) / min_factor;
            }
        }
    });

    if density as f64 != 0.0 {
        maths::strengthen(&mut curve_map, density);
    }
    curve_map
}
