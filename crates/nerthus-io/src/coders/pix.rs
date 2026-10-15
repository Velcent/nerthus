//! Alias/Wavefront PIX images as ImageMagick reads them (`coders/pix.c`):
//! big-endian size and depth, then runs (a count, then a byte or BGR) to
//! the end of the image. IM takes an 8-bit run's byte as a quantum for
//! its index, so 8-bit images come out in the ramp's first colour; a zero
//! count ends the image there.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::colormap;
use crate::pixel_cache::{char_quantum, CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let columns = blob.msb_u16() as usize;
    let rows = blob.msb_u16() as usize;
    blob.msb_u16();
    blob.msb_u16();
    let bits_per_pixel = blob.msb_u16();
    if columns == 0 || rows == 0 || (bits_per_pixel != 8 && bits_per_pixel != 24) {
        bail!("ImproperImageHeader");
    }
    let pseudo = bits_per_pixel == 8;
    let entries = if pseudo { colormap::acquire(256).context("MemoryAllocationFailed")? } else { Vec::new() };
    let stride = if pseudo { 4 } else { 3 };
    let mut data = vec![0.0f32; columns * rows * stride];
    let (mut red, mut green, mut blue, mut index) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut length = 0usize;
    'rows: for y in 0..rows {
        for x in 0..columns {
            if length == 0 {
                match blob.byte() {
                    None | Some(0) => break 'rows,
                    Some(count) => length = count as usize,
                }
                if pseudo {
                    index = char_quantum(blob.byte().unwrap_or(0xff));
                } else {
                    blue = char_quantum(blob.byte().unwrap_or(0xff));
                    green = char_quantum(blob.byte().unwrap_or(0xff));
                    red = char_quantum(blob.byte().unwrap_or(0xff));
                }
            }
            let pixel = &mut data[(y * columns + x) * stride..(y * columns + x + 1) * stride];
            pixel[0] = red;
            pixel[1] = green;
            pixel[2] = blue;
            if pseudo {
                pixel[3] = index;
            }
            length -= 1;
        }
    }
    if pseudo {
        colormap::sync(&mut data, stride, 3, None, &entries);
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    let mut channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    if pseudo {
        channels.push(PixelChannel::Index);
    }
    let mut cache = PixelCache::new(columns, rows, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA);
    if pseudo {
        cache.colormap = entries;
    }
    Ok(cache)
}
