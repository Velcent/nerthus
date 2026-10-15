//! LEGO Mindstorms RGF images as ImageMagick reads them (`coders/rgf.c`):
//! byte sizes, then rows of bits, least significant first, set bits black
//! (a white and black colormap). IM reads a byte for every pixel before
//! decoding the bits, so a file needs that many bytes to open.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let columns = blob.byte().unwrap_or(0xff) as usize;
    let rows = blob.byte().unwrap_or(0xff) as usize;
    if blob.eof {
        bail!("ImproperImageHeader");
    }
    if columns == 0 || rows == 0 {
        bail!("NegativeOrZeroImageSize");
    }
    let raw = blob.read(columns * rows);
    if raw.len() != columns * rows {
        bail!("UnexpectedEndOfFile");
    }
    let colormap = vec![[65535.0, 65535.0, 65535.0, 65535.0], [0.0, 0.0, 0.0, 65535.0]];
    let mut data = Vec::with_capacity(columns * rows * 4);
    let mut p = 0;
    for _ in 0..rows {
        let mut bit = 0;
        let mut byte = 0u8;
        for _ in 0..columns {
            if bit == 0 {
                byte = raw[p];
                p += 1;
            }
            let index = (byte & 0x01) as usize;
            let entry = colormap[index];
            data.extend([entry[0] as f32, entry[1] as f32, entry[2] as f32, index as f32]);
            bit += 1;
            byte >>= 1;
            if bit == 8 {
                bit = 0;
            }
        }
    }
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Index];
    let mut cache = PixelCache::new(columns, rows, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA);
    cache.colormap = colormap;
    Ok(cache)
}
