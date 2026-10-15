//! PFS: 1st Publisher clip art as ImageMagick reads it (`coders/art.c`):
//! the width and height among four little-endian shorts, then 1-bit rows,
//! each padded to an even length, set bits white.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::quantum::{self, Endian, Picture, QuantumInfo, QuantumType};
use crate::pixel_cache::{CacheColorspace, PixelCache, DEFAULT_GAMMA};

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    blob.lsb_u16();
    let columns = blob.lsb_u16() as usize;
    blob.lsb_u16();
    let rows = blob.lsb_u16() as usize;
    if columns == 0 || rows == 0 {
        bail!("ImproperImageHeader");
    }
    let info = QuantumInfo { endian: Endian::Msb, ..QuantumInfo::new(1, false) };
    let mut picture = Picture::new(columns, rows, true, false, false, false, 0);
    let length = info.extent(&picture, QuantumType::Gray);
    for y in 0..rows {
        let row = blob.read(length);
        if row.len() != length {
            bail!("UnableToReadImageData");
        }
        quantum::import(&mut picture, &info, QuantumType::Gray, y * columns, columns, row)?;
        blob.read(length & 1);
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    let image_type = quantum::image_type(QuantumType::Gray, 1);
    Ok(picture.into_cache(true, CacheColorspace::Gray, DEFAULT_GAMMA, image_type))
}
