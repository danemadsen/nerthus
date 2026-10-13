//! ImageMagick-compatible raster operations.
//!
//! A large part of the node maths is ImageMagick's (Magick.NET, Q16 HDRI).
//! This crate reproduces the operations the nodes use with ImageMagick 7's
//! algorithms so the nodes behave the same way.

pub mod colorspace;
pub mod composite;
pub mod convolve;
pub mod distort;
pub mod filters;
pub mod image;
pub mod levels;
pub mod liquid_rescale;
pub mod misc;
pub mod morphology;
pub mod octree;
pub mod quanta;
pub mod resize;
pub mod threshold_maps;

pub use colorspace::Colorspace;
pub use composite::CompositeOp;
pub use image::{Image, Interpolate, VirtualPixel};
pub use levels::EvaluateOperator;
pub use misc::{NoiseType, SparseColorMethod, SparsePoint};
pub use morphology::MorphologyMethod;
pub use resize::Filter;
