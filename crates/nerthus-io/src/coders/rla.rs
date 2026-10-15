//! Wavefront RLA images as ImageMagick reads them (`coders/rla.c`): a
//! 740-byte header (the active window gives the size), scanline offsets
//! bottom row first, then each scanline's channels run-length encoded as
//! 8-bit red, green, blue and matte (every channel past the third is
//! alpha). Samples IM never writes are left uninitialised there; here they
//! are zero.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use crate::pixel_cache::{char_quantum, CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let short = |blob: &mut Blob| blob.msb_u16() as i16 as i32;
    for _ in 0..4 {
        short(&mut blob);
    }
    let left = short(&mut blob);
    let right = short(&mut blob);
    let bottom = short(&mut blob);
    let top = short(&mut blob);
    let _frame = short(&mut blob);
    let _storage_type = short(&mut blob);
    let mut channels = short(&mut blob);
    if channels < 0 {
        bail!("ImproperImageHeader");
    }
    let matte_channels = short(&mut blob);
    if matte_channels < 0 {
        bail!("ImproperImageHeader");
    }
    if channels > 3 || matte_channels > 3 {
        bail!("Unsupported number of channels");
    }
    if channels == 0 {
        channels = 3;
    }
    channels += matte_channels;
    short(&mut blob);
    short(&mut blob);
    blob.read(16 + 24 * 4);
    blob.msb_u32();
    blob.read(128 + 128 + 64 + 32 + 32 + 20 + 24 + 8 + 32);
    short(&mut blob);
    blob.read(12 + 32);
    for _ in 0..5 {
        short(&mut blob);
    }
    blob.read(32);
    if blob.read(36).len() != 36 {
        bail!("UnableToReadImageData");
    }
    blob.msb_u32();
    let alpha = matte_channels != 0;
    let columns = right - left + 1;
    let rows = top - bottom + 1;
    if columns <= 0 || rows <= 0 {
        bail!("NegativeOrZeroImageSize");
    }
    let columns = columns as usize;
    let rows = rows as usize;
    let mut scanlines = Vec::with_capacity(rows);
    for _ in 0..rows {
        scanlines.push(blob.msb_u32() as i32 as i64);
    }
    if blob.eof {
        bail!("ImproperImageHeader");
    }
    let mut layout = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    if alpha {
        layout.push(PixelChannel::Alpha);
    }
    let stride = layout.len();
    let mut data = vec![0.0f32; columns * rows * stride];
    for y in 0..rows {
        let offset = scanlines[rows - y - 1];
        if offset < 0 {
            bail!("ImproperImageHeader");
        }
        blob.seek(offset);
        blob.eof = false;
        let mut x = 0usize;
        for channel in 0..channels as usize {
            let slot = match channel {
                0 => Some(0),
                1 => Some(1),
                2 => Some(2),
                _ => alpha.then_some(3),
            };
            let set = |x: usize, byte: u8, data: &mut Vec<f32>| {
                if let Some(slot) = slot {
                    data[(y * columns + x % columns) * stride + slot] = char_quantum(byte);
                }
            };
            let mut length = blob.msb_u16() as i16 as i32;
            while length > 0 {
                let byte = blob.byte().unwrap_or(0xff);
                let mut run = byte as i8 as i32;
                length -= 1;
                if length == 0 {
                    break;
                }
                if run < 0 {
                    while run < 0 {
                        let value = blob.byte().unwrap_or(0xff);
                        length -= 1;
                        set(x, value, &mut data);
                        x += 1;
                        run += 1;
                    }
                    continue;
                }
                let value = blob.byte().unwrap_or(0xff);
                length -= 1;
                run += 1;
                loop {
                    set(x, value, &mut data);
                    x += 1;
                    run -= 1;
                    if run <= 0 {
                        break;
                    }
                }
            }
        }
        if x / channels as usize > columns {
            bail!("CorruptImage");
        }
        if blob.eof {
            break;
        }
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    Ok(PixelCache::new(columns, rows, layout, data, CacheColorspace::SRgb, DEFAULT_GAMMA))
}
