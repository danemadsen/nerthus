//! Port of the CS snow simulation (`Erosion1`) behind the Snowfall node and
//! the snow passes of Arboreal and Crumble.

mod engine;
mod resample;

pub use engine::{simulate, simulate_with_snapshot, SnowOutput, SnowSettings, SnowSnapshot};
