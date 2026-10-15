//! Slow-scan TV HRZ images as ImageMagick reads them (`coders/hrz.c`):
//! 256×240 6-bit RGB, each sample times four kept to a byte.

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::pixel_cache::{char_quantum, CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

const COLUMNS: usize = 256;
const ROWS: usize = 240;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    if bytes.len() < COLUMNS * ROWS * 3 {
        bail!("UnableToReadImageData");
    }
    let data = bytes[..COLUMNS * ROWS * 3].iter().map(|&value| char_quantum(value.wrapping_mul(4))).collect();
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    Ok(PixelCache::new(COLUMNS, ROWS, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA))
}
