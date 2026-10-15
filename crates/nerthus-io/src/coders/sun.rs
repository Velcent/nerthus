//! Sun rasterfiles as ImageMagick reads them (`coders/sun.c`): 1- and 8-bit
//! colormapped images (PseudoClass, a grey ramp without a colormap), 24-bit
//! BGR or RGB and 32-bit with alpha first, raw or run-length encoded. Only
//! the first image of a file.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::colormap;
use crate::pixel_cache::{char_quantum, CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

const MAGIC: u32 = 0x59a6_6a95;
const RMT_NONE: u32 = 0;
const RMT_EQUAL_RGB: u32 = 1;
const RMT_RAW: u32 = 2;
const RT_STANDARD: u32 = 1;
const RT_ENCODED: u32 = 2;
const RT_FORMAT_RGB: u32 = 3;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

/// `DecodeImage`: bytes, and runs of `0x80 count byte` (`count + 1`
/// copies; `0x80 0` is one `0x80`). Fails unless it fills `extent`.
fn decode_rle(compressed: &[u8], extent: usize) -> Option<Vec<u8>> {
    let mut pixels = Vec::with_capacity(extent);
    let mut position = 0;
    while position < compressed.len() && pixels.len() < extent {
        let mut byte = compressed[position];
        position += 1;
        if byte != 128 {
            pixels.push(byte);
            continue;
        }
        if position >= compressed.len() {
            break;
        }
        let mut count = compressed[position] as i32;
        position += 1;
        if count > 0 {
            if position >= compressed.len() {
                break;
            }
            byte = compressed[position];
            position += 1;
        }
        while count >= 0 && pixels.len() < extent {
            pixels.push(byte);
            count -= 1;
        }
    }
    (pixels.len() == extent).then_some(pixels)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let magic = blob.msb_u32();
    if magic != MAGIC {
        bail!("ImproperImageHeader");
    }
    let width = blob.msb_u32();
    let height = blob.msb_u32();
    let depth = blob.msb_u32();
    let length = blob.msb_u32();
    let kind = blob.msb_u32();
    let map_type = blob.msb_u32();
    let map_length = blob.msb_u32();
    if map_length as u64 > blob.size() {
        bail!("InsufficientImageDataInFile");
    }
    if kind != RT_STANDARD && kind != RT_ENCODED && kind != RT_FORMAT_RGB {
        bail!("ImproperImageHeader");
    }
    if map_type == RMT_NONE && map_length != 0 {
        bail!("ImproperImageHeader");
    }
    if !matches!(depth, 1 | 8 | 24 | 32) {
        bail!("ImproperImageHeader");
    }
    if map_type != RMT_NONE && map_type != RMT_EQUAL_RGB && map_type != RMT_RAW {
        bail!("ColormapTypeNotSupported");
    }

    // Images under 24 bits are PseudoClass.
    let mut colors = 0usize;
    let mut colormap = Vec::new();
    if depth < 24 {
        colors = match map_type {
            RMT_NONE => 1 << depth,
            RMT_EQUAL_RGB => map_length as usize / 3,
            _ => map_length as usize,
        };
        if colors == 0 {
            bail!("ImproperImageHeader");
        }
        colormap = colormap::acquire(colors).context("UnableToCreateColormap")?;
    }
    match map_type {
        RMT_EQUAL_RGB => {
            // No colours (a 24- or 32-bit image): the map's allocation fails.
            if colors == 0 {
                bail!("MemoryAllocationFailed");
            }
            for component in 0..3 {
                let values = blob.read(colors);
                if values.len() != colors {
                    bail!("UnexpectedEndOfFile");
                }
                for (entry, &value) in colormap.iter_mut().zip(values) {
                    entry[component] = char_quantum(value) as f64;
                }
            }
        }
        RMT_RAW => {
            let map = blob.read(map_length as usize);
            if map.len() != map_length as usize {
                bail!("UnexpectedEndOfFile");
            }
        }
        _ => {}
    }
    let alpha = depth == 32;
    if width == 0 || height == 0 {
        bail!("NegativeOrZeroImageSize");
    }
    if length == 0 {
        bail!("ImproperImageHeader");
    }
    let width = width as usize;
    let height = height as usize;
    let number_pixels = (width * height) as u64;
    // `8UL * length` in a Windows (32-bit) `unsigned long`.
    let data_bits = 8u32.wrapping_mul(length) as u64;
    if kind != RT_ENCODED && number_pixels * depth as u64 > data_bits {
        bail!("ImproperImageHeader");
    }
    if kind != RT_ENCODED && length as u64 > blob.size() {
        bail!("InsufficientImageDataInFile");
    }
    let mut data = vec![0u8; length as usize];
    let read = blob.read(length as usize);
    if kind != RT_ENCODED && read.len() != length as usize {
        bail!("UnableToReadImageData");
    }
    data[..read.len()].copy_from_slice(read);

    let quantum = if depth == 1 { 15 } else { 7 };
    let bytes_per_line = (width * depth as usize + quantum) / 8;
    let pixels_length = height.checked_mul(bytes_per_line).context("ImproperImageHeader")?;
    let mut pixels = vec![0u8; pixels_length + height];
    if kind == RT_ENCODED {
        let decoded = decode_rle(&data, pixels_length).context("UnableToReadImageData")?;
        pixels[..pixels_length].copy_from_slice(&decoded);
    } else {
        if blob.eof {
            bail!("UnexpectedEndOfFile");
        }
        if length as usize > pixels_length + height {
            bail!("ImproperImageHeader");
        }
        pixels[..data.len()].copy_from_slice(&data);
    }

    let pseudo = depth < 24;
    let mut channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    if alpha {
        channels.push(PixelChannel::Alpha);
    }
    if pseudo {
        channels.push(PixelChannel::Index);
    }
    let stride = channels.len();
    let mut cache_data = Vec::with_capacity(width * height * stride);
    let mut p = 0usize;
    if depth == 1 {
        for _ in 0..height {
            let mut x = 0;
            while x + 7 < width {
                for bit in (0..8).rev() {
                    let index = if pixels[p] & (1 << bit) != 0 { 0.0 } else { 1.0 };
                    cache_data.extend([0.0, 0.0, 0.0, index]);
                }
                p += 1;
                x += 8;
            }
            if !width.is_multiple_of(8) {
                for bit in ((8 - width % 8)..8).rev() {
                    let index = if pixels[p] & (1 << bit) != 0 { 0.0 } else { 1.0 };
                    cache_data.extend([0.0, 0.0, 0.0, index]);
                }
                p += 1;
            }
            if !width.div_ceil(8).is_multiple_of(2) {
                p += 1;
            }
        }
    } else if pseudo {
        for _ in 0..height {
            for _ in 0..width {
                let index = pixels[p] as usize;
                if index >= colors {
                    bail!("InvalidColormapIndex");
                }
                cache_data.extend([0.0, 0.0, 0.0, index as f32]);
                p += 1;
            }
            if !width.is_multiple_of(2) {
                p += 1;
            }
        }
    } else {
        let bytes_per_pixel = 3 + usize::from(alpha);
        for _ in 0..height {
            for _ in 0..width {
                let alpha_value = if alpha {
                    p += 1;
                    Some(char_quantum(pixels[p - 1]))
                } else {
                    None
                };
                let (red, green, blue) = if kind == RT_STANDARD {
                    (pixels[p + 2], pixels[p + 1], pixels[p])
                } else {
                    (pixels[p], pixels[p + 1], pixels[p + 2])
                };
                p += 3;
                cache_data.extend([char_quantum(red), char_quantum(green), char_quantum(blue)]);
                if let Some(alpha_value) = alpha_value {
                    cache_data.push(alpha_value);
                }
            }
            if !(bytes_per_pixel * width).is_multiple_of(2) {
                p += 1;
            }
        }
    }
    if pseudo {
        colormap::sync(&mut cache_data, stride, stride - 1, None, &colormap);
    }
    let mut cache = PixelCache::new(width, height, channels, cache_data, CacheColorspace::SRgb, DEFAULT_GAMMA);
    cache.colormap = colormap;
    Ok(cache)
}
