//! Truevision TGA as ImageMagick reads it (`coders/tga.c`): colormapped,
//! true-colour and monochrome images, raw or run-length encoded, at 1, 8,
//! 15/16, 24 and 32 bits, turned upright (and unmirrored) by the origin
//! bits, with the interleaved row order of attribute bits 6–7. A colormap
//! makes the image PseudoClass even when its pixels are true colour (their
//! index stays 0). Colormapped and true-colour images have alpha when the
//! attribute bits, a 32-bit depth or 32-bit colormap say so — unless the
//! TGA 2.0 extension area's attribute type says otherwise; one colour value
//! carries over from pixel to pixel, so samples a depth lacks (24-bit
//! alpha) keep the last ones read. Monochrome images are GRAY.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::quantum::{char_quantum, quantum_range, scale_any};
use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

const QUANTUM_RANGE: f32 = 65535.0;

const COLORMAP: u8 = 1;
const RGB: u8 = 2;
const MONOCHROME: u8 = 3;
const RLE_COLORMAP: u8 = 9;
const RLE_RGB: u8 = 10;
const RLE_MONOCHROME: u8 = 11;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

/// A colour as the reader keeps it (`PixelInfo`).
#[derive(Clone, Copy)]
struct Colour {
    red: f32,
    green: f32,
    blue: f32,
    alpha: f32,
}

/// 5 bits each of red, green and blue from a little-endian pair.
fn colour_555(low: u8, high: u8) -> (f32, f32, f32) {
    let range = quantum_range(5);
    let red = scale_any(((high & 0x7c) >> 2) as u64, range);
    let green = scale_any((((high & 0x03) as u64) << 3) + (((low & 0xe0) >> 5) as u64), range);
    let blue = scale_any((low & 0x1f) as u64, range);
    (red, green, blue)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let id_length = blob.read(1);
    if id_length.len() != 1 {
        bail!("ImproperImageHeader");
    }
    let id_length = id_length[0];
    let colormap_type = blob.byte().unwrap_or(0xFF);
    let image_type = blob.byte().unwrap_or(0xFF);
    let known = matches!(image_type, COLORMAP | RGB | MONOCHROME | RLE_COLORMAP | RLE_RGB | RLE_MONOCHROME);
    let colormapped = matches!(image_type, COLORMAP | RLE_COLORMAP);
    if !known || (colormapped && colormap_type == 0) {
        bail!("ImproperImageHeader");
    }
    let colormap_index = blob.lsb_u16() as usize;
    let colormap_length = blob.lsb_u16() as usize;
    let colormap_size = blob.byte().unwrap_or(0xFF);
    blob.lsb_u16();
    blob.lsb_u16();
    let width = blob.lsb_u16() as usize;
    let height = blob.lsb_u16() as usize;
    let bits_per_pixel = blob.byte().unwrap_or(0xFF);
    let attributes = blob.byte().unwrap_or(0xFF);
    if blob.eof {
        bail!("UnableToReadImageData");
    }
    if !(1..17).contains(&bits_per_pixel) && bits_per_pixel != 24 && bits_per_pixel != 32 {
        bail!("ImproperImageHeader");
    }
    let monochrome = matches!(image_type, MONOCHROME | RLE_MONOCHROME);
    let rle = matches!(image_type, RLE_COLORMAP | RLE_RGB | RLE_MONOCHROME);
    let mut alpha = !monochrome && ((attributes & 0x0F) > 0 || bits_per_pixel == 32 || colormap_size == 32);
    let mut pseudo = colormapped;
    let mut colors = if colormapped { colormap_index + colormap_length } else { 0 };
    if id_length != 0 {
        blob.read(id_length as usize);
    }
    let (flip_x, flip_y) = match (attributes & (1 << 4) != 0, attributes & (1 << 5) != 0) {
        (false, false) => (false, true),
        (false, true) => (false, false),
        (true, false) => (true, true),
        (true, true) => (true, false),
    };
    if width == 0 || height == 0 {
        bail!("NegativeOrZeroImageSize");
    }

    let mut pixel = Colour { red: 0.0, green: 0.0, blue: 0.0, alpha: QUANTUM_RANGE };
    let mut colormap: Vec<Colour> = Vec::new();
    if colormap_type != 0 {
        if colors < colormap_index {
            colors = colormap_index;
        }
        // `AcquireImageColormap` (which also makes the image PseudoClass).
        let count = colors.max(1);
        if count > 65536 {
            bail!("UnableToCreateColormap");
        }
        pseudo = true;
        colormap = (0..count)
            .map(|index| {
                let level = (index as f64 * (65535.0 / (count.max(2) - 1) as f64)) as f32;
                Colour { red: level, green: level, blue: level, alpha: QUANTUM_RANGE }
            })
            .collect();
        colors = count;
        for entry in colormap.iter_mut().take(colormap_index) {
            *entry = pixel;
        }
        for entry in colormap.iter_mut().skip(colormap_index) {
            match colormap_size {
                15 | 16 => {
                    let low = blob.byte().unwrap_or(0xFF);
                    let high = blob.byte().unwrap_or(0xFF);
                    let (red, green, blue) = colour_555(low, high);
                    pixel.red = red;
                    pixel.green = green;
                    pixel.blue = blue;
                }
                24 => {
                    pixel.blue = char_quantum(blob.byte().unwrap_or(0xFF));
                    pixel.green = char_quantum(blob.byte().unwrap_or(0xFF));
                    pixel.red = char_quantum(blob.byte().unwrap_or(0xFF));
                }
                32 => {
                    pixel.blue = char_quantum(blob.byte().unwrap_or(0xFF));
                    pixel.green = char_quantum(blob.byte().unwrap_or(0xFF));
                    pixel.red = char_quantum(blob.byte().unwrap_or(0xFF));
                    pixel.alpha = char_quantum(blob.byte().unwrap_or(0xFF));
                }
                _ => {
                    let gray = char_quantum(blob.byte().unwrap_or(0xFF));
                    pixel.red = gray;
                    pixel.green = gray;
                    pixel.blue = gray;
                }
            }
            *entry = pixel;
        }
    }

    let gray = monochrome;
    let mut channels = if gray { vec![PixelChannel::Gray] } else { vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue] };
    if alpha {
        channels.push(PixelChannel::Alpha);
    }
    if pseudo {
        channels.push(PixelChannel::Index);
    }
    let stride = channels.len();
    let mut data = vec![0.0f32; width * height * stride];
    let constrain = |index: i64| -> Result<usize> {
        if index < 0 || index as usize >= colors {
            bail!("InvalidColormapIndex");
        }
        Ok(index as usize)
    };

    let mut base = 0usize;
    let mut offset = 0usize;
    let step = if (attributes & 0xc0) >> 6 == 2 { 2 } else { 1 };
    let mut run_length = 0u8;
    let mut flag = 0u8;
    let mut skip = false;
    let mut index: f32 = 0.0;
    for _ in 0..height {
        let row = if flip_y { height - 1 - offset } else { offset };
        for x in 0..width {
            if rle {
                if run_length != 0 {
                    run_length -= 1;
                    skip = flag != 0;
                } else {
                    let byte = blob.read(1);
                    if byte.len() != 1 {
                        bail!("UnableToReadImageData");
                    }
                    run_length = byte[0];
                    flag = run_length & 0x80;
                    if flag != 0 {
                        run_length -= 128;
                    }
                    skip = false;
                }
            }
            if !skip {
                match bits_per_pixel {
                    1 => {
                        if x & 0x07 == 0 {
                            let byte = blob.read(1);
                            if byte.len() != 1 {
                                bail!("UnableToReadImageData");
                            }
                            index = byte[0] as f32;
                        } else {
                            index = ((index as usize) << 1) as f32;
                        }
                        let bit = (index as i64 as u8) & 0x80 != 0;
                        if colormap_type != 0 {
                            pixel = colormap[constrain(i64::from(bit))?];
                        } else {
                            let level = if bit { QUANTUM_RANGE } else { 0.0 };
                            pixel.red = level;
                            pixel.green = level;
                            pixel.blue = level;
                        }
                    }
                    15 | 16 => {
                        let pair = blob.read(2);
                        if pair.len() != 2 {
                            bail!("UnableToReadImageData");
                        }
                        let (low, high) = (pair[0], pair[1]);
                        let (red, green, blue) = colour_555(low, high);
                        pixel.red = red;
                        pixel.green = green;
                        pixel.blue = blue;
                        if alpha {
                            pixel.alpha = if high & 0x80 == 0 { 0.0 } else { QUANTUM_RANGE };
                        }
                        if pseudo {
                            index = constrain(((high as i64) << 8) + low as i64)? as f32;
                        }
                    }
                    24 => {
                        let three = blob.read(3);
                        if three.len() != 3 {
                            bail!("UnableToReadImageData");
                        }
                        pixel.blue = char_quantum(three[0]);
                        pixel.green = char_quantum(three[1]);
                        pixel.red = char_quantum(three[2]);
                    }
                    32 => {
                        let four = blob.read(4);
                        if four.len() != 4 {
                            bail!("UnableToReadImageData");
                        }
                        pixel.blue = char_quantum(four[0]);
                        pixel.green = char_quantum(four[1]);
                        pixel.red = char_quantum(four[2]);
                        pixel.alpha = char_quantum(four[3]);
                    }
                    _ => {
                        let byte = blob.read(1);
                        if byte.len() != 1 {
                            bail!("UnableToReadImageData");
                        }
                        index = byte[0] as f32;
                        if colormap_type != 0 {
                            pixel = colormap[constrain(byte[0] as i64)?];
                        } else {
                            let level = char_quantum(byte[0]);
                            pixel.red = level;
                            pixel.green = level;
                            pixel.blue = level;
                        }
                    }
                }
            }
            let column = if flip_x { width - 1 - x } else { x };
            let base_offset = (row * width + column) * stride;
            let target = &mut data[base_offset..base_offset + stride];
            if gray {
                target[0] = pixel.blue;
            } else {
                target[0] = pixel.red;
                target[1] = pixel.green;
                target[2] = pixel.blue;
            }
            let mut next = if gray { 1 } else { 3 };
            if alpha {
                target[next] = pixel.alpha;
                next += 1;
            }
            if pseudo {
                target[next] = index;
            }
        }
        offset += step;
        if offset >= height {
            base += 1;
            offset = base;
        }
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }

    // The TGA 2.0 extension area: an alpha image whose attribute type isn't
    // 3 (straight alpha) loses its alpha channel.
    if bytes.len() >= 26 + 18 {
        let footer = &bytes[bytes.len() - 26..];
        let extension = u32::from_le_bytes([footer[0], footer[1], footer[2], footer[3]]) as usize;
        let signature = &footer[8..26];
        let signature_end = signature.iter().position(|&byte| byte == 0).unwrap_or(signature.len());
        let matches_signature = signature[..signature_end].eq_ignore_ascii_case(b"TRUEVISION-XFILE.");
        if extension > 3 && matches_signature && extension + 2 <= bytes.len() && u16::from_le_bytes([bytes[extension], bytes[extension + 1]]) == 495 {
            let attributes_type = bytes.get(extension + 2 + 41 + 324 + 59 + 41 + 27).copied();
            if alpha && attributes_type != Some(3) {
                // `alpha_trait` is cleared, but the cache keeps the channel.
                alpha = false;
            }
        }
    }
    let colorspace = if gray { CacheColorspace::Gray } else { CacheColorspace::SRgb };
    let mut cache = PixelCache::new(width, height, channels, data, colorspace, DEFAULT_GAMMA);
    cache.alpha_trait = alpha;
    if pseudo {
        cache.colormap = colormap.iter().map(|entry| [entry.red as f64, entry.green as f64, entry.blue as f64, entry.alpha as f64]).collect();
    }
    Ok(cache)
}
