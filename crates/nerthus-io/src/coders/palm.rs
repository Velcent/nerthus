//! Palm OS bitmaps as ImageMagick reads them (`coders/palm.c`): the first
//! bitmap, 1, 2, 4 or 8 bits through a colormap (a grey ramp, the file's
//! colormap or the Palm system palette, entries counted down from 255) or
//! 16-bit 5-6-5, raw, RLE or scanline compressed. Indices count down from
//! the top of the colormap; a transparent index makes its colour clear.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::colormap;
use crate::pixel_cache::{char_quantum, CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

const IS_COMPRESSED: u16 = 0x8000;
const HAS_COLORMAP: u16 = 0x4000;
const HAS_TRANSPARENCY: u16 = 0x2000;
const COMPRESSION_SCANLINE: u8 = 0x00;
const COMPRESSION_RLE: u8 = 0x01;
const COMPRESSION_NONE: u8 = 0xff;
const QUANTUM_RANGE: f32 = 65535.0;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

/// The Palm system palette: the 6×6×6 colour cube without black, ten
/// greys, silver, maroon, purple, green and teal, then black.
fn palm_palette() -> Vec<[u8; 3]> {
    const LEVELS: [u8; 6] = [255, 204, 153, 102, 51, 0];
    let mut palette = Vec::with_capacity(256);
    for blues in [[255u8, 204, 153], [102, 51, 0]] {
        for red in LEVELS {
            for blue in blues {
                for green in LEVELS {
                    palette.push([red, green, blue]);
                }
            }
        }
    }
    palette.truncate(215);
    for grey in [17u8, 34, 68, 85, 119, 136, 170, 187, 221, 238] {
        palette.push([grey, grey, grey]);
    }
    palette.extend([[192, 192, 192], [128, 0, 0], [128, 0, 128], [0, 128, 0], [0, 128, 128]]);
    palette.resize(256, [0, 0, 0]);
    palette
}

/// `IsFuzzyEquivalencePixelInfo` with no fuzz between a pixel and the
/// transparent colour, both opaque.
fn same_colour(pixel: &[f32], target: &[f64; 4]) -> bool {
    let fuzz = 0.5f64 * 3.0;
    let scale = (pixel_alpha_scale(65535.0)) * pixel_alpha_scale(target[3]);
    let mut distance = 0.0;
    for (value, &wanted) in pixel.iter().zip(target.iter()).take(3) {
        let difference = *value as f64 - wanted;
        distance += scale * difference * difference;
        if distance > fuzz {
            return false;
        }
    }
    true
}

fn pixel_alpha_scale(alpha: f64) -> f64 {
    (1.0 / 65535.0) * alpha
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let columns = blob.msb_u16() as usize;
    let rows = blob.msb_u16() as usize;
    if blob.eof {
        bail!("ImproperImageHeader");
    }
    if columns == 0 || rows == 0 {
        bail!("NegativeOrZeroImageSize");
    }
    let bytes_per_row = blob.msb_u16() as usize;
    let flags = blob.msb_u16();
    let bits_per_pixel = blob.byte().map_or(usize::MAX, usize::from);
    if !matches!(bits_per_pixel, 1 | 2 | 4 | 8 | 16) {
        bail!("UnsupportedBitsPerPixel");
    }
    let version = blob.byte().unwrap_or(0xff);
    if version > 2 {
        bail!("FileFormatVersionMismatch");
    }
    let _next_depth_offset = blob.msb_u16();
    let transparent_index = blob.byte().map_or(usize::MAX, usize::from);
    let compression = blob.byte().unwrap_or(0xff);
    if !matches!(compression, COMPRESSION_NONE | COMPRESSION_SCANLINE | COMPRESSION_RLE) {
        bail!("UnrecognizedImageCompression");
    }
    let _pad = blob.msb_u16();
    let mut entries = Vec::new();
    if bits_per_pixel < 16 {
        entries = colormap::acquire(1 << bits_per_pixel).context("MemoryAllocationFailed")?;
    }
    let colors = entries.len();
    if bits_per_pixel == 16 {
        blob.read(5);
        // The transparent colour, unused: a 16-bit image with transparency
        // fails on its colormap index below.
        blob.read(3);
    }
    if bits_per_pixel == 8 {
        if flags & HAS_COLORMAP != 0 {
            let count = blob.msb_u16() as usize;
            for i in 0..count {
                blob.byte();
                let index = 255isize - i as isize;
                if index < 0 || index as usize >= colors {
                    bail!("InvalidColormapIndex");
                }
                let entry = &mut entries[index as usize];
                for channel in entry.iter_mut().take(3) {
                    *channel = char_quantum(blob.byte().unwrap_or(0xff)) as f64;
                }
            }
        } else {
            for (i, rgb) in palm_palette().iter().enumerate() {
                let entry = &mut entries[255 - i];
                entry[0] = char_quantum(rgb[0]) as f64;
                entry[1] = char_quantum(rgb[1]) as f64;
                entry[2] = char_quantum(rgb[2]) as f64;
            }
        }
    }
    if flags & IS_COMPRESSED != 0 {
        let _size = blob.msb_u16();
    }

    let extent = bytes_per_row.max(2 * columns);
    let mut one_row = vec![0u8; extent];
    let mut last_row = vec![0u8; extent];
    let pseudo = bits_per_pixel < 16;
    let mask = (1usize << bits_per_pixel) - 1;
    let mut channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    if pseudo {
        channels.push(PixelChannel::Index);
    }
    let stride = channels.len();
    // `SetImageBackgroundColor`: white.
    let mut data = vec![QUANTUM_RANGE; columns * rows * stride];
    for y in 0..rows {
        if flags & IS_COMPRESSED == 0 {
            let row = blob.read(bytes_per_row);
            if row.len() != bytes_per_row {
                break;
            }
            one_row[..bytes_per_row].copy_from_slice(row);
        } else if compression == COMPRESSION_RLE {
            let mut i = 0;
            while i < bytes_per_row {
                let Some(count) = blob.byte() else {
                    break;
                };
                let count = (count as usize).min(bytes_per_row - i);
                let byte = blob.byte().unwrap_or(0xff);
                one_row[i..i + count].fill(byte);
                i += count;
            }
        } else if compression == COMPRESSION_SCANLINE {
            let mut i = 0;
            while i < bytes_per_row {
                let Some(count) = blob.byte() else {
                    break;
                };
                for bit in 0..(bytes_per_row - i).min(8) {
                    one_row[i + bit] = if y == 0 || count & (1 << (7 - bit)) != 0 { blob.byte().unwrap_or(0xff) } else { last_row[i + bit] };
                }
                i += 8;
            }
            last_row[..bytes_per_row].copy_from_slice(&one_row[..bytes_per_row]);
        }
        let target = &mut data[y * columns * stride..(y + 1) * columns * stride];
        if bits_per_pixel == 16 {
            if columns > 2 * bytes_per_row {
                bail!("CorruptImage");
            }
            for x in 0..columns {
                let color = ((one_row[2 * x] as u32) << 8) | one_row[2 * x + 1] as u32;
                target[x * stride] = (QUANTUM_RANGE * ((color >> 11) & 0x1f) as f32) / 31.0;
                target[x * stride + 1] = (QUANTUM_RANGE * ((color >> 5) & 0x3f) as f32) / 63.0;
                target[x * stride + 2] = (QUANTUM_RANGE * (color & 0x1f) as f32) / 31.0;
            }
        } else {
            let mut bit = 8 - bits_per_pixel as i32;
            let mut ptr = 0usize;
            for x in 0..columns {
                if ptr >= bytes_per_row {
                    bail!("CorruptImage");
                }
                let index = mask - ((one_row[ptr] as usize & (mask << bit)) >> bit);
                let entry = entries[index];
                target[x * stride] = entry[0] as f32;
                target[x * stride + 1] = entry[1] as f32;
                target[x * stride + 2] = entry[2] as f32;
                target[x * stride + 3] = index as f32;
                if bit != 0 {
                    bit -= bits_per_pixel as i32;
                } else {
                    ptr += 1;
                    bit = 8 - bits_per_pixel as i32;
                }
            }
        }
    }

    let mut alpha = false;
    if flags & HAS_TRANSPARENCY != 0 {
        let index = mask as isize - transparent_index as isize;
        if index < 0 || index as usize >= colors {
            bail!("InvalidColormapIndex");
        }
        // `TransparentPaintImage`: DirectClass with an opaque alpha
        // channel, cleared where the colour matches.
        let target = entries[index as usize];
        let mut with_alpha = Vec::with_capacity(columns * rows * 4);
        for pixel in data.chunks_exact(stride) {
            let clear = same_colour(&pixel[..3], &target);
            with_alpha.extend_from_slice(&pixel[..3]);
            with_alpha.push(if clear { 0.0 } else { QUANTUM_RANGE });
        }
        data = with_alpha;
        channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Alpha];
        alpha = true;
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    let mut cache = PixelCache::new(columns, rows, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA);
    if pseudo && !alpha {
        cache.colormap = entries;
    }
    Ok(cache)
}
