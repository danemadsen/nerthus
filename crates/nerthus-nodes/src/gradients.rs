//! The `Gradients` library: radial, conical, hemispherical and linear
//! ramps.

use nerthus_engine::{Map, Resolution};

/// `GradientEdgeBehaviour`: what a linear gradient does past its ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeBehaviour {
    Clip,
    Repeat,
    Mirror,
}

impl EdgeBehaviour {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Repeat" => Self::Repeat,
            "Mirror" => Self::Mirror,
            _ => Self::Clip,
        }
    }
}

/// A smooth radial falloff: 1 at the centre to 0 at `radius`, eased with
/// `t * t * (3 - 2t)`.
pub fn radial(size: usize, center_x: f32, center_y: f32, radius: f32, height: f32) -> Map {
    let radius_inverse = 1.0 / radius;
    Map::generate(Resolution::square(size), |x, y| {
        let dx = x - center_x;
        let dy = y - center_y;
        let distance = (dx * dx + dy * dy).sqrt();
        let mut value = 1.0 - distance * radius_inverse;
        value = value.max(0.0);
        // `value *= value * (3 - 2 * value)`, as the library has it.
        #[allow(clippy::misrefactored_assign_op)]
        {
            value *= value * (3.0 - 2.0 * value);
        }
        value * height
    })
}

/// `Gradients.RadialGradient(resolution, radius)`: [`radial`] centred on the
/// map, `radius` given as a fraction of the map's diameter.
pub fn radial_relative(size: usize, radius: f32, height: f32) -> Map {
    let center = size as f32 / 2.0;
    let pixels = radius / 2.0 * size as f32;
    radial(size, center, center, pixels, height)
}

/// A cone: `1 - sqrt(min(d² / r², 1))`.
pub fn cone(size: usize, center_x: f32, center_y: f32, radius: f32, height: f32) -> Map {
    let radius_inverse_squared = 1.0 / (radius * radius);
    Map::generate(Resolution::square(size), |x, y| {
        let dx = x - center_x;
        let dy = y - center_y;
        let mut value = dx * dx + dy * dy;
        value *= radius_inverse_squared;
        value = value.min(1.0);
        value = 1.0 - value.sqrt();
        value * height
    })
}

/// A dome: `sqrt(1 - min(d² / r², 1))`.
pub fn hemisphere(resolution: Resolution, center_x: f32, center_y: f32, radius: f32, height: f32) -> Map {
    let radius_inverse_squared = 1.0 / (radius * radius);
    Map::generate(resolution, |x, y| {
        let dx = x - center_x;
        let dy = y - center_y;
        let mut value = dx * dx + dy * dy;
        value *= radius_inverse_squared;
        value = value.min(1.0);
        value = (1.0 - value).sqrt();
        value * height
    })
}

/// A flatter dome: `sqrt(1 - min(d² / r², 1)²)`.
pub fn hemisphere_flattened(resolution: Resolution, center_x: f32, center_y: f32, radius: f32, height: f32) -> Map {
    let radius_inverse_squared = 1.0 / (radius * radius);
    Map::generate(resolution, |x, y| {
        let dx = x - center_x;
        let dy = y - center_y;
        let mut value = dx * dx + dy * dy;
        value *= radius_inverse_squared;
        value = value.min(1.0);
        value = (1.0 - value * value).sqrt();
        value * height
    })
}

/// A ramp across the map at `azimuth` degrees, `length` pixels long and
/// centred on (`center_x`, `center_y`).
pub fn linear(size: usize, center_x: f32, center_y: f32, length: f32, azimuth: f32, edge: EdgeBehaviour) -> Map {
    let length_inverse = 1.0 / length;
    let radians = azimuth * (std::f32::consts::PI / 180.0);
    let (sine, cosine) = crate::maths::sin_cos(radians);
    let azimuth_x = 0.0 - sine;
    let azimuth_y = cosine;
    Map::generate(Resolution::square(size), |x, y| {
        let dx = x - center_x;
        let dy = y - center_y;
        let projected = 0.0 - (dx * azimuth_x + dy * azimuth_y);
        let mut value = projected * length_inverse + 0.5;
        match edge {
            EdgeBehaviour::Clip => {
                value = value.max(0.0);
                value = value.min(1.0);
            }
            EdgeBehaviour::Repeat => {
                value -= value.floor();
            }
            EdgeBehaviour::Mirror => {
                value *= 0.5;
                value -= value.floor();
                value *= 2.0;
                if value > 1.0 {
                    value = 2.0 - value;
                }
            }
        }
        value
    })
}
