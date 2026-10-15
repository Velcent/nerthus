//! Nokia OTA bitmaps as ImageMagick reads them (`coders/otb.c`): byte or
//! (info bit 4) 16-bit sizes, depth 1, rows of bits, most significant
//! first, set bits black. PseudoClass over a black and white colormap.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::colormap;
use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let info = blob.byte().unwrap_or(0xff);
    let (columns, rows) = if (info >> 4) & 1 == 0 {
        (blob.byte().unwrap_or(0xff) as usize, blob.byte().unwrap_or(0xff) as usize)
    } else {
        (blob.msb_u16() as usize, blob.msb_u16() as usize)
    };
    if columns == 0 || rows == 0 {
        bail!("ImproperImageHeader");
    }
    if blob.byte() != Some(1) {
        bail!("OnlyLevelZerofilesSupported");
    }
    let colormap = colormap::acquire(2).context("MemoryAllocationFailed")?;
    let mut data = Vec::with_capacity(columns * rows * 4);
    for _ in 0..rows {
        let mut bit = 0;
        let mut byte = 0u8;
        for _ in 0..columns {
            if bit == 0 {
                byte = blob.byte().context("CorruptImage")?;
            }
            let index = if byte & (0x01 << (7 - bit)) != 0 { 0.0 } else { 1.0 };
            data.extend([0.0, 0.0, 0.0, index]);
            bit = (bit + 1) % 8;
        }
    }
    colormap::sync(&mut data, 4, 3, None, &colormap);
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Index];
    let mut cache = PixelCache::new(columns, rows, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA);
    cache.colormap = colormap;
    Ok(cache)
}
