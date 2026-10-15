//! MacPaint images as ImageMagick reads them (`coders/mac.c`): a 512-byte
//! header (640 with a MacBinary one), then 576×720 PackBits rows of
//! inverted bits. IM reads 0x80 as a run of 129, continues a run that
//! crosses a row end with zeros, and stops quietly at the end of the file;
//! a run past the last row fails the read.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::colormap;
use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

const COLUMNS: usize = 576;
const ROWS: usize = 720;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let length = blob.lsb_u16();
    if length & 0xff != 0 {
        bail!("CorruptImage");
    }
    let skip = if length == 0 { 510 } else { 638 };
    for _ in 0..skip {
        if blob.byte().is_none() {
            bail!("CorruptImage");
        }
    }
    let entries = colormap::acquire(2).context("MemoryAllocationFailed")?;
    let row_length = COLUMNS.div_ceil(8);
    let mut indices = vec![0u8; COLUMNS * ROWS];
    let mut row = vec![0u8; row_length];
    let mut offset = 0usize;
    let mut y = 0usize;
    // Whether a row was flushed: IM reuses the run's byte to take the row
    // apart, which leaves it zero.
    let mut put = |byte: u8, offset: &mut usize, y: &mut usize, row: &mut Vec<u8>| -> Result<bool> {
        row[*offset] = byte;
        *offset += 1;
        if *offset >= row_length {
            if *y >= ROWS {
                bail!("PixelsAreNotAuthentic");
            }
            for x in 0..COLUMNS {
                indices[*y * COLUMNS + x] = (row[x / 8] >> (7 - x % 8)) & 1;
            }
            *offset = 0;
            *y += 1;
            return Ok(true);
        }
        Ok(false)
    };
    while y < ROWS {
        let count = blob.byte().unwrap_or(0xff);
        if blob.eof {
            break;
        }
        if count == 0 || count >= 128 {
            let mut byte = !blob.byte().unwrap_or(0xff);
            let repeat = (!count).wrapping_add(2);
            for _ in 0..repeat {
                if put(byte, &mut offset, &mut y, &mut row)? {
                    byte = 0;
                }
            }
            continue;
        }
        for _ in 0..count as usize + 1 {
            let byte = !blob.byte().unwrap_or(0xff);
            put(byte, &mut offset, &mut y, &mut row)?;
        }
    }
    let mut data = Vec::with_capacity(COLUMNS * ROWS * 4);
    for &index in &indices {
        let entry = entries[index as usize];
        data.extend([entry[0] as f32, entry[1] as f32, entry[2] as f32, index as f32]);
    }
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Index];
    let mut cache = PixelCache::new(COLUMNS, ROWS, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA);
    cache.colormap = entries;
    Ok(cache)
}
