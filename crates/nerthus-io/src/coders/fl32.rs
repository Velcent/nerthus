//! FilmLight FL32 images as ImageMagick reads them (`coders/fl32.c`):
//! little-endian 32-bit float samples, grey, grey and alpha, RGB or RGBA
//! (five or more channels: RGB, its extra channels read as the next
//! pixels' and left as meta channels).

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::quantum::{self, Endian, Picture, QuantumInfo, QuantumType};
use crate::pixel_cache::{CacheColorspace, PixelCache, DEFAULT_GAMMA};

const MAGIC: u32 = 842222662;
/// `MaxPixelChannels`.
const MAX_CHANNELS: u32 = 64;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    if blob.lsb_u32() != MAGIC {
        bail!("ImproperImageHeader");
    }
    let rows = blob.lsb_u32() as usize;
    let columns = blob.lsb_u32() as usize;
    let channels = blob.lsb_u32();
    if columns == 0 || rows == 0 || channels == 0 || channels >= MAX_CHANNELS {
        bail!("ImproperImageHeader");
    }
    let (gray, alpha, kind, meta) = match channels {
        1 => (true, false, QuantumType::Gray, 0),
        2 => (true, true, QuantumType::GrayAlpha, 0),
        3 => (false, false, QuantumType::Rgb, 0),
        4 => (false, true, QuantumType::Rgba, 0),
        _ => (false, false, QuantumType::Rgb, channels as usize - 3),
    };
    let info = QuantumInfo { endian: Endian::Lsb, ..QuantumInfo::new(32, true) };
    let mut picture = Picture::new(columns, rows, gray, false, alpha, false, meta);
    let extent = info.extent(&picture, kind);
    let remaining = blob.size().saturating_sub(blob.tell());
    if (extent as u64).saturating_mul(rows as u64) > remaining {
        bail!("UnableToReadImageData");
    }
    for y in 0..rows {
        let row = blob.read(extent);
        quantum::import(&mut picture, &info, kind, y * columns, columns, row)?;
    }
    let colorspace = if gray { CacheColorspace::Gray } else { CacheColorspace::SRgb };
    let image_type = quantum::image_type(kind, 32);
    Ok(picture.into_cache(gray, colorspace, DEFAULT_GAMMA, image_type))
}
