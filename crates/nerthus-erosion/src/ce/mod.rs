//! Port of the CE erosion engine behind the Erosion, Wizard, Stacked
//! Erosion, Shatter, CEt and Draw nodes.

mod droplet;
mod engine;
pub(crate) mod kernel;
mod layer;
mod thermal;

pub use engine::{erode, ErosionOutput, ErosionSettings};
