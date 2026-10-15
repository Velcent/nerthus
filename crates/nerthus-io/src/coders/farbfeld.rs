//! farbfeld as ImageMagick reads it (`coders/farbfeld.c`): big-endian 16-bit
//! RGBA rows; sRGB with alpha, of true-colour type.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::quantum::{self, Endian, Picture, QuantumInfo, QuantumType};
use crate::pixel_cache::{CacheColorspace, ImageType, PixelCache, DEFAULT_GAMMA};

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let magic = blob.read(8);
    if magic != b"farbfeld" {
        bail!("ImproperImageHeader");
    }
    let width = blob.msb_u32() as usize;
    let height = blob.msb_u32() as usize;
    if width == 0 || height == 0 {
        bail!("NegativeOrZeroImageSize");
    }
    let mut picture = Picture::new(width, height, false, false, true, false, 0);
    let mut info = QuantumInfo::new(16, false);
    info.endian = Endian::Msb;
    let extent = info.extent(&picture, QuantumType::Rgba);
    let mut rows = 0;
    for y in 0..height {
        let row = blob.read(extent);
        if row.len() != extent {
            break;
        }
        quantum::import(&mut picture, &info, QuantumType::Rgba, y * width, width, row)?;
        rows = y + 1;
    }
    if rows < height {
        bail!("UnableToReadImageData");
    }
    Ok(picture.into_cache(false, CacheColorspace::SRgb, DEFAULT_GAMMA, ImageType::TrueColor))
}
