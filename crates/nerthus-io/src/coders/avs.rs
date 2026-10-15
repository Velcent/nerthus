//! AVS X images (`coders/avs.c`: big-endian size, then ARGB bytes) and
//! Dune AAI images (`coders/aai.c`: little-endian size, then BGRA bytes,
//! an alpha of 254 read as 255) as ImageMagick reads them. IM's AAI reader
//! only adds an alpha channel once it reads an alpha that isn't opaque,
//! which it can't see without the channel, so AAI images come out RGB.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use crate::pixel_cache::{char_quantum, CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

pub fn read(path: &Path, dune: bool) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes, dune)
}

pub fn decode(bytes: &[u8], dune: bool) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let (width, height) = if dune { (blob.lsb_u32(), blob.lsb_u32()) } else { (blob.msb_u32(), blob.msb_u32()) };
    if blob.eof || width == 0 || height == 0 {
        bail!("ImproperImageHeader");
    }
    let columns = width as usize;
    let rows = height as usize;
    let length = 4 * columns;
    let remaining = blob.size().saturating_sub(blob.tell());
    if (length as u64).saturating_mul(rows as u64) > remaining {
        bail!("UnableToReadImageData");
    }
    let stride = if dune { 3 } else { 4 };
    let mut data = Vec::with_capacity(columns * rows * stride);
    for _ in 0..rows {
        let row = blob.read(length);
        for pixel in row.as_chunks::<4>().0 {
            if dune {
                data.extend([char_quantum(pixel[2]), char_quantum(pixel[1]), char_quantum(pixel[0])]);
            } else {
                data.extend([char_quantum(pixel[1]), char_quantum(pixel[2]), char_quantum(pixel[3]), char_quantum(pixel[0])]);
            }
        }
    }
    let mut channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    if !dune {
        channels.push(PixelChannel::Alpha);
    }
    Ok(PixelCache::new(columns, rows, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA))
}
