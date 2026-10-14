//! The nodes of Nerthus: every node type, each checked bit for bit
//! against reference outputs.

// Ports keep their sources' comparisons: `!(a < b)` is true for NaN, which
// `a >= b` is not, and results must match bit for bit.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

pub mod aspect;
pub mod blur;
pub mod cellular3d;
pub mod cartographer;
pub mod classic_combiner;
pub mod colour;
pub mod colour_library;
pub mod colour_transport;
pub mod combiner;
pub mod craterfield;
pub mod debris;
pub mod delaunator;
pub mod dotnet_priority_queue;
pub mod dotnet_sort;
pub mod dotnet_vector;
pub mod erosions;
pub mod exprtk;
pub mod fast_blur;
pub mod file_maps;
pub mod flow;
pub mod filter_core;
pub mod fractal_terraces;
pub mod gpu;
pub mod gradients;
pub mod graw;
pub mod histogram;
pub mod lighting;
pub mod magick;
pub mod lake;
pub mod landscapes;
pub mod masking;
pub mod maths;
pub mod median;
pub mod mesh_port;
pub mod mesh_rasterizer;
pub mod meshify;
pub mod mathv;
pub mod mathx;
pub mod migrated;
pub mod morph;
pub mod morphology;
pub mod morphology2;
pub mod mosaic;
pub mod multi_fractal;
pub mod nodes;
pub mod obj_loader;
pub mod noise;
pub mod others;
pub mod parallel_erosion;
pub mod patterns;
pub mod profiles;
pub mod random;
pub mod random_noises;
pub mod rock_core;
pub mod sat_blur;
pub mod shade;
pub mod scatters;
pub mod simulations;
pub mod slope_blur;
pub mod spline;
pub mod surfacer;
pub mod tangent_bake;
mod sycl;
pub mod texturize;
pub mod thermal;
pub mod thermal_shaper;
pub mod transformer;
pub mod vector_mask;
pub mod warps;
pub mod waters;
pub mod x86;
mod x86_tables;

use nerthus_engine::Registry;

/// A registry with every implemented node and modifier.
pub fn registry() -> Registry {
    let mut registry = Registry::default();
    nodes::register(&mut registry);
    registry
}
