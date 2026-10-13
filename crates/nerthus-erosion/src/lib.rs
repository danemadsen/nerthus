//! Erosion engines (ports of native erosion modules).

// Ports keep their sources' comparisons: `!(a < b)` is true for NaN, which
// `a >= b` is not, and results must match bit for bit.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

pub mod ce;
pub mod cs;
pub mod cw;
pub mod hydro;
pub mod ts;
