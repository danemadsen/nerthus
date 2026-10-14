//! ImageMagick's readers for the formats the File node reads through
//! Magick.NET beyond the common ones (IM 7.1.2-19's `coders/*.c`): what
//! each leaves in the pixel cache — channels, colorspace, gamma and quanta
//! — which the node reads raw.

mod bc7_tables;
pub(crate) mod blob;
pub mod bmp;
pub mod dds;
pub mod dng;
pub mod emf;
pub mod exr;
pub mod gif;
pub mod hdr;
mod exr_dwa;
pub mod jpeg;
pub mod jp2;
pub mod miff;
pub mod psd;
pub mod sgi;
pub mod svg;
pub mod tiff;
pub mod webp;
pub(crate) mod xz;
