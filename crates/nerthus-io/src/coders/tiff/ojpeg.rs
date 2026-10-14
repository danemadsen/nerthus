//! libtiff's old-style JPEG codec (`tif_ojpeg.c`).

use anyhow::{bail, Result};

use super::directory::Tiff;

pub(super) struct OldJpeg;

impl OldJpeg {
    pub fn setup(_tiff: &Tiff) -> Result<Self> {
        bail!("old-style JPEG in TIFF is not supported yet")
    }

    pub fn decode(&mut self, _tiff: &Tiff, _raw: &[u8], _output: &mut [u8], _plane: u16) -> Result<()> {
        bail!("old-style JPEG in TIFF is not supported yet")
    }
}
