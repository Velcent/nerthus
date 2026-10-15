//! Utah raster toolkit RLE images as ImageMagick reads them
//! (`coders/rle.c`): a header (planes, background, colormaps, comment),
//! then opcodes painting planes bottom row first. Three or more planes are
//! RGB (and matte); one plane is grey through a 256-entry ramp. IM checks
//! colormap values against an image that has no colours yet, so a file
//! with colormaps fails.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::colormap;
use crate::pixel_cache::{char_quantum, CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

const SKIP_LINES: u8 = 0x01;
const SET_COLOR: u8 = 0x02;
const SKIP_PIXELS: u8 = 0x03;
const BYTE_DATA: u8 = 0x05;
const RUN_DATA: u8 = 0x06;
const EOF_OP: u8 = 0x07;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

/// An opcode's operand: a byte, or (long form) a signed short whose value
/// -1 IM can't tell from the end of the file.
fn operand(blob: &mut Blob, opcode: u8) -> Result<i64> {
    let short = blob.byte().context("UnexpectedEndOfFile")?;
    if opcode & 0x40 == 0 {
        return Ok(short as i64);
    }
    let long = blob.lsb_u16() as i16 as i64;
    if long == -1 {
        bail!("UnexpectedEndOfFile");
    }
    Ok(long)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    if blob.read(2) != b"\x52\xcc" {
        bail!("ImproperImageHeader");
    }
    blob.lsb_u16();
    blob.lsb_u16();
    let columns = blob.lsb_u16() as usize;
    let rows = blob.lsb_u16() as usize;
    let flags = blob.byte().unwrap_or(0xff);
    let alpha = flags & 0x04 != 0;
    let mut planes = blob.byte().map_or(usize::MAX, usize::from);
    let bits_per_pixel = blob.byte().unwrap_or(0xff);
    let colormaps = blob.byte().map_or(usize::MAX, usize::from);
    let map_length_log = blob.byte().unwrap_or(0xff);
    if map_length_log >= 22 {
        bail!("ImproperImageHeader");
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    let map_length = 1usize << map_length_log;
    if planes == 0 || planes == 2 || (alpha && (planes <= 2 || planes > 254)) || bits_per_pixel != 8 {
        bail!("ImproperImageHeader");
    }
    if planes > 4 {
        bail!("ImproperImageHeader");
    }
    if columns == 0 || columns >= 32768 || rows == 0 || rows >= 32768 {
        bail!("ImproperImageHeader");
    }
    let mut background = vec![0u8; planes];
    if flags & 0x02 != 0 {
        blob.byte();
    } else {
        for value in background.iter_mut() {
            *value = blob.byte().unwrap_or(0xff);
        }
    }
    if planes & 0x01 == 0 {
        blob.byte();
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    if alpha {
        planes += 1;
    }
    let number_pixels = columns * rows;
    let size = blob.size();
    if size == 0 || (number_pixels as u64 * planes as u64 * bits_per_pixel as u64 / 8) / size > 254 {
        bail!("InsufficientImageDataInFile");
    }
    if (colormaps * map_length) as u64 > size {
        bail!("InsufficientImageDataInFile");
    }
    for _ in 0..colormaps * map_length {
        blob.lsb_u16();
        if blob.eof {
            bail!("UnexpectedEndOfFile");
        }
    }
    if flags & 0x08 != 0 {
        let length = blob.lsb_u16() as usize;
        if length != 0 {
            if blob.read(length - 1).len() != length - 1 {
                bail!("UnexpectedEndOfFile");
            }
            if length & 0x01 == 0 {
                blob.byte();
            }
        }
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }

    let filled = if planes.is_multiple_of(2) { planes } else { planes + 1 };
    let length = columns * rows * filled.max(4);
    let mut pixels = vec![0u8; length];
    if flags & 0x01 != 0 && flags & 0x02 == 0 {
        let mut p = 0;
        for _ in 0..number_pixels {
            if !alpha {
                pixels[p..p + planes].copy_from_slice(&background);
                p += planes;
            } else {
                pixels[p..p + planes - 1].copy_from_slice(&background[..planes - 1]);
                pixels[p + planes - 1] = 0;
                p += planes;
            }
        }
    }
    let mut plane = 0usize;
    let mut x: i64 = 0;
    let mut y: i64 = 0;
    let mut opcode = blob.byte().context("UnexpectedEndOfFile")?;
    loop {
        match opcode & 0x3f {
            SKIP_LINES => {
                let count = operand(&mut blob, opcode)?;
                x = 0;
                y += count;
            }
            SET_COLOR => {
                let value = blob.byte().context("UnexpectedEndOfFile")?;
                plane = if value == 255 { planes - 1 } else { value as usize };
                x = 0;
            }
            SKIP_PIXELS => {
                x += operand(&mut blob, opcode)?;
            }
            kind @ (BYTE_DATA | RUN_DATA) => {
                let mut count = operand(&mut blob, opcode)?;
                let run_value = if kind == RUN_DATA {
                    let value = blob.byte().unwrap_or(0xff);
                    blob.byte();
                    Some(value)
                } else {
                    None
                };
                let offset = (rows as i64 - y - 1) * columns as i64 * planes as i64 + x * planes as i64 + plane as i64;
                count += 1;
                if offset < 0 || (offset + count * planes as i64) as u64 > length as u64 {
                    bail!("UnableToReadImageData");
                }
                let mut p = offset as usize;
                for i in 0..count {
                    let value = match run_value {
                        Some(value) => value,
                        None => blob.byte().unwrap_or(0xff),
                    };
                    if y < rows as i64 && x + i < columns as i64 {
                        pixels[p] = value;
                    }
                    p += planes;
                }
                if run_value.is_none() && count & 0x01 != 0 {
                    blob.byte();
                }
                x += count;
            }
            _ => {}
        }
        opcode = blob.byte().context("UnexpectedEndOfFile")?;
        if opcode & 0x3f == EOF_OP {
            break;
        }
    }
    if colormaps != 0 {
        // `ValidateColormapValue` against an image of no colours.
        bail!("InvalidColormapIndex");
    }

    let cache = if planes >= 3 {
        let mut channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
        if alpha {
            channels.push(PixelChannel::Alpha);
        }
        let step = channels.len();
        let data: Vec<f32> = pixels[..number_pixels * step].iter().map(|&value| char_quantum(value)).collect();
        PixelCache::new(columns, rows, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA)
    } else {
        let entries = colormap::acquire(256).context("MemoryAllocationFailed")?;
        let mut data = Vec::with_capacity(number_pixels * 4);
        for &index in &pixels[..number_pixels] {
            let entry = entries[index as usize];
            data.extend([entry[0] as f32, entry[1] as f32, entry[2] as f32, index as f32]);
        }
        let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Index];
        let mut cache = PixelCache::new(columns, rows, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA);
        cache.colormap = entries;
        cache
    };
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    Ok(cache)
}
