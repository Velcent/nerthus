//! OpenRaster images as ImageMagick reads them (`coders/ora.c`): the
//! archive's `mergedimage.png`, read as a PNG.

use std::path::Path;

use anyhow::{Context, Result};

use crate::pixel_cache::{read_png_bytes, PixelCache};

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let merged = super::zip::read_member(bytes, b"mergedimage.png")?.context("UnableToOpenFile")?;
    read_png_bytes(&merged)
}
