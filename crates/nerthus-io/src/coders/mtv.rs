//! MTV ray-tracer images as ImageMagick reads them (`coders/mtv.c`): a
//! "columns rows" line, then 8-bit RGB.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use crate::pixel_cache::{char_quantum, CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

/// `sscanf(line, "%lu %lu")` into Windows (32-bit) `unsigned long`s.
fn scan_size(line: &[u8]) -> Option<(usize, usize)> {
    let mut values = Vec::new();
    let mut p = 0;
    while values.len() < 2 {
        while p < line.len() && super::text::is_space(line[p]) {
            p += 1;
        }
        let negative = line.get(p) == Some(&b'-');
        if matches!(line.get(p), Some(b'+' | b'-')) {
            p += 1;
        }
        let start = p;
        let mut value = 0u64;
        while let Some(digit) = line.get(p).filter(|byte| byte.is_ascii_digit()) {
            value = value.wrapping_mul(10).wrapping_add((digit - b'0') as u64);
            p += 1;
        }
        if p == start {
            break;
        }
        let value = if negative { value.wrapping_neg() } else { value };
        values.push(value as u32 as usize);
    }
    (values.len() == 2).then(|| (values[0], values[1]))
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let line = blob.string().unwrap_or_default();
    let Some((columns, rows)) = scan_size(&line) else {
        bail!("ImproperImageHeader");
    };
    if columns == 0 || rows == 0 {
        bail!("NegativeOrZeroImageSize");
    }
    let length = 3 * columns;
    let remaining = blob.size().saturating_sub(blob.tell());
    if (length as u64).saturating_mul(rows as u64) > remaining {
        bail!("UnableToReadImageData");
    }
    let mut data = Vec::with_capacity(columns * rows * 3);
    for _ in 0..rows {
        let row = blob.read(length);
        data.extend(row.iter().map(|&value| char_quantum(value)));
    }
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    Ok(PixelCache::new(columns, rows, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA))
}
