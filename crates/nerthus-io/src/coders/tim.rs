//! PlayStation TIM images as ImageMagick reads them (`coders/tim.c`): an
//! optional 16- or 256-entry BGR555 colour lookup table, then 4- or 8-bit
//! indices (low nibble first, an odd last pixel its high nibble), BGR555
//! or 24-bit RGB. Without a table, indexed pixels stay black; with one,
//! direct colours give way to its first entry, as IM leaves them.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::colormap;
use crate::pixel_cache::{char_quantum, CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

fn scale_5_to_8(color: u32) -> u8 {
    ((color << 3) | (color >> 2)) as u8
}

/// A BGR555 word as red, green and blue quanta.
fn bgr555(word: u32) -> [f32; 3] {
    let red = char_quantum(scale_5_to_8(word & 0x1f));
    let green = char_quantum(scale_5_to_8((word >> 5) & 0x1f));
    let blue = char_quantum(scale_5_to_8((word >> 10) & 0x1f));
    [red, green, blue]
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    if blob.lsb_u32() != 0x10 {
        bail!("ImproperImageHeader");
    }
    let flag = blob.lsb_u32();
    let has_clut = flag & (1 << 3) != 0;
    let pixel_mode = flag & 0x07;
    let bits_per_pixel = match pixel_mode {
        1 => 8,
        2 => 16,
        3 => 24,
        _ => 4,
    };
    let mut entries = Vec::new();
    if has_clut {
        blob.lsb_u32();
        blob.lsb_u16();
        blob.lsb_u16();
        blob.lsb_u16();
        blob.lsb_u16();
        let colors = if pixel_mode == 1 { 256 } else { 16 };
        entries = colormap::acquire(colors).context("MemoryAllocationFailed")?;
        let table = blob.read(2 * colors);
        if table.len() != 2 * colors {
            bail!("InsufficientImageDataInFile");
        }
        for (entry, pair) in entries.iter_mut().zip(table.as_chunks::<2>().0) {
            let [red, green, blue] = bgr555(u16::from_le_bytes([pair[0], pair[1]]) as u32);
            entry[0] = red as f64;
            entry[1] = green as f64;
            entry[2] = blue as f64;
        }
    }
    blob.lsb_u32();
    blob.lsb_u16();
    blob.lsb_u16();
    let width = blob.lsb_u16() as usize;
    let height = blob.lsb_u16() as usize;
    let image_size = 2 * width * height;
    if image_size as u64 > blob.size() {
        bail!("InsufficientImageDataInFile");
    }
    let bytes_per_line = width * 2;
    let columns = width * 16 / bits_per_pixel;
    let rows = height;
    if columns == 0 || rows == 0 {
        bail!("NegativeOrZeroImageSize");
    }
    let pixels = blob.read(image_size);
    if pixels.len() != image_size {
        bail!("InsufficientImageDataInFile");
    }
    let pseudo = has_clut;
    let mut channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    if pseudo {
        channels.push(PixelChannel::Index);
    }
    let stride = channels.len();
    let mut data = vec![0.0f32; columns * rows * stride];
    for y in 0..rows {
        let row = &pixels[y * bytes_per_line..(y + 1) * bytes_per_line];
        let target = &mut data[y * columns * stride..(y + 1) * columns * stride];
        match bits_per_pixel {
            4 => {
                if pseudo {
                    for x in 0..columns {
                        let byte = row[x / 2];
                        let nibble = if x % 2 == 1 || x + 1 == columns && columns % 2 == 1 { byte >> 4 } else { byte & 0x0f };
                        target[x * stride + 3] = nibble as f32;
                    }
                }
            }
            8 => {
                if pseudo {
                    for x in 0..columns {
                        target[x * stride + 3] = row[x] as f32;
                    }
                }
            }
            16 => {
                for x in 0..columns {
                    let word = u16::from_le_bytes([row[2 * x], row[2 * x + 1]]) as u32;
                    target[x * stride..x * stride + 3].copy_from_slice(&bgr555(word));
                }
            }
            _ => {
                for x in 0..columns {
                    for channel in 0..3 {
                        target[x * stride + channel] = char_quantum(row[3 * x + channel]);
                    }
                }
            }
        }
    }
    if pseudo {
        colormap::sync(&mut data, stride, 3, None, &entries);
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    let mut cache = PixelCache::new(columns, rows, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA);
    if pseudo {
        cache.colormap = entries;
    }
    Ok(cache)
}
