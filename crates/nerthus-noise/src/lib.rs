//! Noise generation for Nerthus.
//!
//! `fast_noise` is a faithful port of FastNoiseSIMD, the library behind
//! almost every procedural primitive.

pub mod fast_noise;

pub use fast_noise::{
    calculate_fractal_bounding, CellularDistanceFunction, CellularReturnType, FastNoise, FractalType, NoiseType, PerturbType,
};
