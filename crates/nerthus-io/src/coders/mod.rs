//! ImageMagick's readers for the formats the File node reads through
//! Magick.NET beyond the common ones (IM 7.1.2-19's `coders/*.c`): what
//! each leaves in the pixel cache — channels, colorspace, gamma and quanta
//! — which the node reads raw.

mod bc7_tables;
pub mod dds;
pub mod dng;
pub mod jp2;
pub mod miff;
pub mod svg;
pub mod webp;
