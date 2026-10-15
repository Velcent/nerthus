//! Wireless bitmaps as ImageMagick reads them (`coders/wbmp.c`): level-0
//! files only, a two-entry black and white colormap (PseudoClass sRGB),
//! rows padded to whole bytes.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

/// `WBMPReadInteger`: seven bits a byte while the top bit is set.
fn integer(blob: &mut Blob) -> Option<usize> {
    let mut value = 0usize;
    loop {
        let byte = blob.byte()?;
        value = (value << 7) | (byte & 0x7f) as usize;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let header = blob.read(2);
    if header.len() != 2 {
        bail!("ImproperImageHeader");
    }
    if header != [0, 0] {
        bail!("OnlyLevelZerofilesSupported");
    }
    let width = integer(&mut blob).context("CorruptWBMPimage")?;
    let height = integer(&mut blob).context("CorruptWBMPimage")?;
    if width == 0 || height == 0 {
        bail!("ImproperImageHeader");
    }
    let colormap = [[0.0f64, 0.0, 0.0, 65535.0], [65535.0, 65535.0, 65535.0, 65535.0]];
    let mut data = Vec::with_capacity(width * height * 4);
    for _ in 0..height {
        let mut bit = 0;
        let mut byte = 0u8;
        for _ in 0..width {
            if bit == 0 {
                byte = blob.byte().context("CorruptImage")?;
            }
            let index = usize::from(byte & (0x01 << (7 - bit)) != 0);
            let entry = colormap[index];
            data.extend([entry[0] as f32, entry[1] as f32, entry[2] as f32, index as f32]);
            bit = (bit + 1) % 8;
        }
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Index];
    let mut cache = PixelCache::new(width, height, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA);
    cache.colormap = colormap.to_vec();
    Ok(cache)
}
