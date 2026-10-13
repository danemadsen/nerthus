//! Port of the CW water engine behind the Lake node (`erode_terrain`) and
//! the Sea node's edge fill.

mod blur;
mod engine;
mod fill;

pub use engine::{edge_fill, erode_lakes, LakeSettings, WaterOutput};
