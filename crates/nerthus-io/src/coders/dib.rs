//! Windows device-independent bitmaps (a BMP without its file header) as
//! ImageMagick reads them (`coders/dib.c`): 1, 4 and 8 bits through a
//! colormap (8 bits needs a colour count), 16-bit 5-5-5 or (with bit
//! fields) 5-6-5, 24-bit BGR and 32-bit BGRA, raw or run-length encoded.
//! IM decodes RLE4 as RLE8 at eight bits a pixel, as it does here.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::colormap;
use crate::pixel_cache::{char_quantum, CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

const RGB_COMPRESSION: u32 = 0;
const RLE8_COMPRESSION: u32 = 1;
const RLE4_COMPRESSION: u32 = 2;
const BITFIELDS_COMPRESSION: u32 = 3;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

fn scale_5_to_8(color: u32) -> u8 {
    ((color << 3) | (color >> 2)) as u8
}

fn scale_6_to_8(color: u32) -> u8 {
    ((color << 2) | (color >> 4)) as u8
}

/// `DecodeImage`: run-length encoded rows (always as RLE8), up to the end
/// of bitmap marker or the last row.
fn decode_rle(blob: &mut Blob, columns: usize, rows: usize) -> Option<Vec<u8>> {
    let number_pixels = columns * rows;
    let mut pixels = vec![0u8; number_pixels];
    let mut p = 0usize;
    let mut x = 0usize;
    let mut y = 0usize;
    while y < rows {
        if p >= number_pixels {
            break;
        }
        let Some(count) = blob.byte() else {
            break;
        };
        if count > 0 {
            let count = (count as usize).min(number_pixels - p);
            let Some(byte) = blob.byte() else {
                break;
            };
            pixels[p..p + count].fill(byte);
            p += count;
            x += count;
            continue;
        }
        let Some(escape) = blob.byte() else {
            break;
        };
        match escape {
            0x01 => return Some(pixels),
            0x00 => {
                x = 0;
                y += 1;
                p = y * columns;
            }
            0x02 => {
                x += blob.byte()? as usize;
                y += blob.byte()? as usize;
                p = y * columns + x;
            }
            count => {
                let count = (count as usize).min(number_pixels.saturating_sub(p));
                for _ in 0..count {
                    let Some(byte) = blob.byte() else {
                        break;
                    };
                    pixels[p] = byte;
                    p += 1;
                }
                x += count;
                if count & 0x01 != 0 {
                    blob.byte();
                }
            }
        }
    }
    blob.byte();
    blob.byte();
    (y >= rows).then_some(pixels)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    if blob.lsb_u32() != 40 {
        bail!("ImproperImageHeader");
    }
    let width = blob.lsb_u32() as i32;
    let height = blob.lsb_u32() as i32;
    let planes = blob.lsb_u16();
    let mut bits_per_pixel = blob.lsb_u16() as u32;
    if bits_per_pixel > 32 {
        bail!("ImproperImageHeader");
    }
    let compression = blob.lsb_u32();
    let image_size = blob.lsb_u32();
    let _x_pixels = blob.lsb_u32();
    let _y_pixels = blob.lsb_u32();
    let number_colors = blob.lsb_u32();
    let colors_important = blob.lsb_u32();
    if !matches!(bits_per_pixel, 1 | 4 | 8 | 16 | 24 | 32) {
        bail!("ImproperImageHeader");
    }
    let mut red_mask = 0u16;
    if compression == BITFIELDS_COMPRESSION && matches!(bits_per_pixel, 16 | 32) {
        red_mask = blob.lsb_u32() as u16;
        let _green_mask = blob.lsb_u32();
        let _blue_mask = blob.lsb_u32();
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    if width <= 0 || height == 0 {
        bail!("NegativeOrZeroImageSize");
    }
    if planes != 1 {
        bail!("StaticPlanesValueNotEqualToOne");
    }
    if bits_per_pixel < 16 && number_colors as u64 > 1u64 << bits_per_pixel {
        bail!("UnrecognizedNumberOfColors");
    }
    if (compression == RLE8_COMPRESSION && bits_per_pixel != 8) || (compression == RLE4_COMPRESSION && bits_per_pixel != 4) || (compression == BITFIELDS_COMPRESSION && bits_per_pixel < 16) {
        bail!("UnsupportedBitsPerPixel");
    }
    match compression {
        RGB_COMPRESSION | RLE8_COMPRESSION | RLE4_COMPRESSION | BITFIELDS_COMPRESSION => {}
        4 => bail!("JPEGCompressNotSupported"),
        5 => bail!("PNGCompressNotSupported"),
        _ => bail!("UnrecognizedImageCompression"),
    }
    let columns = width.unsigned_abs() as usize;
    let rows = height.unsigned_abs() as usize;
    let alpha = bits_per_pixel == 32;
    if number_colors > 256 || colors_important > 256 {
        bail!("ImproperImageHeader");
    }
    if number_colors != 0 && bits_per_pixel > 8 {
        bail!("ImproperImageHeader");
    }
    if image_size != 0 && image_size as u64 > blob.size() {
        bail!("UnexpectedEndOfFile");
    }
    let mut pseudo = false;
    let mut colors = 0usize;
    if number_colors != 0 || bits_per_pixel < 8 {
        pseudo = true;
        colors = if number_colors != 0 { number_colors as usize } else { 1 << bits_per_pixel };
    }
    let mut entries = Vec::new();
    if pseudo {
        entries = colormap::acquire(colors).context("MemoryAllocationFailed")?;
        let table = blob.read(4 * colors);
        if table.len() != 4 * colors {
            bail!("InsufficientImageDataInFile");
        }
        for (entry, bgrx) in entries.iter_mut().zip(table.as_chunks::<4>().0) {
            entry[0] = char_quantum(bgrx[2]) as f64;
            entry[1] = char_quantum(bgrx[1]) as f64;
            entry[2] = char_quantum(bgrx[0]) as f64;
        }
    }
    if compression == RLE4_COMPRESSION {
        bits_per_pixel <<= 1;
    }
    let mut bytes_per_line = 4 * ((columns * bits_per_pixel as usize).div_ceil(32));
    let length = bytes_per_line * rows;
    if (length / 256) as u64 > blob.size() {
        bail!("InsufficientImageDataInFile");
    }
    let pixels = if compression == RGB_COMPRESSION || compression == BITFIELDS_COMPRESSION {
        let data = blob.read(length);
        if data.len() != length {
            bail!("InsufficientImageDataInFile");
        }
        data.to_vec()
    } else {
        decode_rle(&mut blob, columns, rows).context("UnableToRunlengthDecodeImage")?
    };

    let mut channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    if alpha {
        channels.push(PixelChannel::Alpha);
    }
    let pseudo_class = pseudo && bits_per_pixel <= 8;
    if pseudo_class {
        channels.push(PixelChannel::Index);
    }
    let stride = channels.len();
    let mut data = vec![0.0f32; columns * rows * stride];
    let constrain = |index: usize| -> Result<usize> {
        if index >= colors {
            bail!("InvalidColormapIndex");
        }
        Ok(index)
    };
    if matches!(compression, RLE8_COMPRESSION | RLE4_COMPRESSION) && bits_per_pixel == 8 {
        bytes_per_line = columns;
    }
    for y in (0..rows).rev() {
        let row_start = (rows - y - 1) * bytes_per_line;
        let row = &pixels[row_start.min(pixels.len())..];
        let target = &mut data[y * columns * stride..(y + 1) * columns * stride];
        match bits_per_pixel {
            1 => {
                for x in 0..columns {
                    let byte = row[x / 8];
                    target[x * stride + 3] = u8::from(byte & (0x80 >> (x % 8)) != 0) as f32;
                }
            }
            4 => {
                for x in 0..columns {
                    let byte = row[x / 2];
                    let nibble = if x % 2 == 0 { byte >> 4 } else { byte & 0x0f };
                    target[x * stride + 3] = constrain(nibble as usize)? as f32;
                }
            }
            8 => {
                for x in 0..columns {
                    target[x * stride + 3] = constrain(row[x] as usize)? as f32;
                }
            }
            16 => {
                for x in 0..columns {
                    let word = row[2 * x] as u32 | ((row[2 * x + 1] as u32) << 8);
                    let (red, green, blue) = if red_mask == 0 {
                        (scale_5_to_8((word >> 10) & 0x1f), scale_5_to_8((word >> 5) & 0x1f), scale_5_to_8(word & 0x1f))
                    } else {
                        (scale_5_to_8((word >> 11) & 0x1f), scale_6_to_8((word >> 5) & 0x3f), scale_5_to_8(word & 0x1f))
                    };
                    target[x * stride] = char_quantum(red);
                    target[x * stride + 1] = char_quantum(green);
                    target[x * stride + 2] = char_quantum(blue);
                }
            }
            _ => {
                let step = if alpha { 4 } else { 3 };
                for x in 0..columns {
                    let pixel = &row[x * step..x * step + step];
                    target[x * stride] = char_quantum(pixel[2]);
                    target[x * stride + 1] = char_quantum(pixel[1]);
                    target[x * stride + 2] = char_quantum(pixel[0]);
                    if alpha {
                        target[x * stride + 3] = char_quantum(pixel[3]);
                    }
                }
            }
        }
    }
    if pseudo_class {
        colormap::sync(&mut data, stride, stride - 1, None, &entries);
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    if height < 0 {
        // `FlipImage`.
        let row_length = columns * stride;
        for y in 0..rows / 2 {
            let (top, bottom) = data.split_at_mut((rows - 1 - y) * row_length);
            top[y * row_length..(y + 1) * row_length].swap_with_slice(&mut bottom[..row_length]);
        }
    }
    let mut cache = PixelCache::new(columns, rows, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA);
    if pseudo_class {
        cache.colormap = entries;
    }
    Ok(cache)
}
