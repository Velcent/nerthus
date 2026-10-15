//! PlayStation 2 TIM2 textures as ImageMagick reads them
//! (`coders/tim2.c`): one image, direct RGBA16, RGB24 or RGBA32 texels, or
//! 4- or 8-bit indices whose colour table follows the image. IM gives the
//! indexed pixels their colours (`SyncImage`) before it reads that table,
//! so they keep the grey ramp's; the table only becomes the colormap.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::colormap;
use crate::pixel_cache::{char_quantum, CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Rgba16,
    Rgb24,
    Rgba32,
}

/// `GetChannelValue`.
fn channel_value(word: u32, channel: u32, encoding: Encoding) -> f32 {
    match encoding {
        Encoding::Rgba16 => char_quantum((((word >> (channel * 5)) & 0x1f) << 3) as u8),
        Encoding::Rgb24 | Encoding::Rgba32 => char_quantum(((word >> (channel * 8)) & 0xff) as u8),
    }
}

/// `GetAlpha`.
fn alpha_value(word: u32, encoding: Encoding) -> f32 {
    match encoding {
        Encoding::Rgba16 => char_quantum(if (word >> 15) & 0x1f == 0 { 0 } else { 0xff }),
        Encoding::Rgba32 => char_quantum((((word >> 24) & 0xff) << 1).min(0xff) as u8),
        Encoding::Rgb24 => 255.0,
    }
}

fn discard(blob: &mut Blob, count: usize) -> Result<()> {
    if blob.read(count).len() != count {
        bail!("InsufficientImageDataInFile");
    }
    Ok(())
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    if blob.msb_u32() != 0x5449_4d32 {
        bail!("ImproperImageHeader");
    }
    if blob.byte() != Some(0x04) {
        bail!("ImageTypeNotSupported");
    }
    let format_type = blob.byte().unwrap_or(0xff);
    let image_count = blob.lsb_u16();
    discard(&mut blob, 8)?;
    if format_type > 0 {
        discard(&mut blob, 112)?;
    }
    if image_count != 1 {
        bail!("NumberOfImagesIsNotSupported");
    }
    let _total_size = blob.lsb_u32();
    let clut_size = blob.lsb_u32() as usize;
    let image_size = blob.lsb_u32();
    let header_size = blob.lsb_u16() as usize;
    let clut_color_count = blob.lsb_u16() as usize;
    let _image_format = blob.byte();
    let mipmap_count = blob.byte().unwrap_or(0xff);
    let clut_type = blob.byte().unwrap_or(0xff);
    let bpp_type = blob.byte().unwrap_or(0xff);
    let columns = blob.lsb_u16() as usize;
    let rows = blob.lsb_u16() as usize;
    blob.read(8 + 8 + 4 + 4);
    if mipmap_count != 1 {
        bail!("NumberOfImagesIsNotSupported");
    }
    if header_size < 48 {
        bail!("ImproperImageHeader");
    }
    if image_size as u64 > blob.size() || clut_size as u64 > blob.size() {
        bail!("InsufficientImageDataInFile");
    }
    let clut_depth = if clut_type != 0 {
        match clut_type & 0x0f {
            1 => 16,
            2 => 24,
            3 => 32,
            _ => bail!("ImproperImageHeader"),
        }
    } else {
        0
    };
    let bits_per_pixel = match bpp_type {
        1 => 16,
        2 => 24,
        3 => 32,
        4 => 4,
        5 => 8,
        _ => bail!("ImproperImageHeader"),
    };
    let depth = if clut_depth != 0 { clut_depth } else { bits_per_pixel };
    let alpha = depth == 16 || depth == 32;
    if columns == 0 || rows == 0 {
        bail!("NegativeOrZeroImageSize");
    }
    discard(&mut blob, header_size - 48)?;
    let bytes_per_line = (columns * bits_per_pixel).div_ceil(8);
    let pseudo = clut_depth != 0;
    let mut channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    if alpha {
        channels.push(PixelChannel::Alpha);
    }
    if pseudo {
        channels.push(PixelChannel::Index);
    }
    let stride = channels.len();
    let alpha_offset = alpha.then_some(3);
    let mut data = vec![0.0f32; columns * rows * stride];
    let mut entries = Vec::new();
    for y in 0..rows {
        let row = blob.read(bytes_per_line);
        if row.len() != bytes_per_line {
            bail!("InsufficientImageDataInFile");
        }
        let target = &mut data[y * columns * stride..(y + 1) * columns * stride];
        match (pseudo, bits_per_pixel) {
            (true, 4) => {
                for x in 0..columns {
                    let byte = row[x / 2];
                    let nibble = if x % 2 == 1 || (x + 1 == columns && columns % 2 == 1) { byte >> 4 } else { byte & 0x0f };
                    target[x * stride + stride - 1] = nibble as f32;
                }
            }
            (true, 8) => {
                for x in 0..columns {
                    target[x * stride + stride - 1] = row[x] as f32;
                }
            }
            (false, 16) | (false, 24) | (false, 32) => {
                let (encoding, size) = match bits_per_pixel {
                    16 => (Encoding::Rgba16, 2),
                    24 => (Encoding::Rgb24, 3),
                    _ => (Encoding::Rgba32, 4),
                };
                for x in 0..columns {
                    let mut word = 0u32;
                    for (index, &byte) in row[x * size..x * size + size].iter().enumerate() {
                        word |= (byte as u32) << (8 * index);
                    }
                    for channel in 0..3 {
                        target[x * stride + channel as usize] = channel_value(word, channel, encoding);
                    }
                    if let Some(offset) = alpha_offset {
                        target[x * stride + offset] = alpha_value(word, encoding);
                    }
                }
            }
            _ => bail!("ImproperImageHeader"),
        }
    }
    if pseudo {
        // The pixels take the ramp's colours before the table is read.
        entries = colormap::acquire(clut_color_count).context("MemoryAllocationFailed")?;
        colormap::sync(&mut data, stride, stride - 1, alpha_offset, &entries);
        let colors = entries.len();
        let size = clut_size.max(clut_depth / 8 * colors);
        let table = blob.read(size);
        if table.len() != size {
            bail!("InsufficientImageDataInFile");
        }
        let (encoding, entry_size) = match clut_depth {
            16 => (Encoding::Rgba16, 2),
            24 => (Encoding::Rgb24, 3),
            _ => (Encoding::Rgba32, 4),
        };
        for (i, entry) in entries.iter_mut().enumerate() {
            let mut word = 0u32;
            for (index, &byte) in table[i * entry_size..i * entry_size + entry_size].iter().enumerate() {
                word |= (byte as u32) << (8 * index);
            }
            entry[0] = channel_value(word, 0, encoding) as f64;
            entry[1] = channel_value(word, 1, encoding) as f64;
            entry[2] = channel_value(word, 2, encoding) as f64;
            if encoding != Encoding::Rgb24 {
                entry[3] = alpha_value(word, encoding) as f64;
            }
        }
        match clut_type >> 4 {
            0 => {
                // CSM1: the second and third blocks of eight swap in each
                // page of 32.
                let original = entries.clone();
                for page in 0..colors / 32 {
                    let base = page * 32;
                    entries[base + 8..base + 16].copy_from_slice(&original[base + 16..base + 24]);
                    entries[base + 16..base + 24].copy_from_slice(&original[base + 8..base + 16]);
                }
            }
            1 => {}
            _ => bail!("ImproperImageHeader"),
        }
        if blob.eof {
            bail!("UnexpectedEndOfFile");
        }
    }
    let mut cache = PixelCache::new(columns, rows, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA);
    if pseudo {
        cache.colormap = entries;
    }
    Ok(cache)
}
