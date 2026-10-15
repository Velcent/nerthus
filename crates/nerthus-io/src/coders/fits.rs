//! FITS images as ImageMagick reads them (`coders/fits.c`): 80-byte header
//! cards in 2880-byte blocks (extension headers followed while `EXTEND` is
//! set and the header has no image), then the first plane, bottom row
//! first, as grey: `BSCALE`/`BZERO` applied, scaled from `DATAMIN` to
//! `DATAMAX` (the bit depth's range, or a float image's own extremes).
//! IM's flip of the top bit of a byte of each 16-, 32- or 64-bit sample
//! (meant for unsigned data, but made on the sample's `double`) is kept.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::text::{interpret_locale_value, is_space};
use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

const BLOCK_SIZE: u64 = 2880;
const QUANTUM_RANGE: f64 = 65535.0;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

struct FitsInfo {
    extend: bool,
    simple: bool,
    bits_per_pixel: i32,
    columns: i32,
    rows: i32,
    number_axes: i32,
    number_planes: i32,
    min_data: f64,
    max_data: f64,
    zero: f64,
    scale: f64,
    big_endian: bool,
}

/// `StringToLong` (`strtol`) into a Windows (32-bit) `long`, saturating.
fn string_to_long(text: &[u8]) -> i32 {
    let mut p = 0;
    while p < text.len() && is_space(text[p]) {
        p += 1;
    }
    let negative = text.get(p) == Some(&b'-');
    if matches!(text.get(p), Some(b'+' | b'-')) {
        p += 1;
    }
    let mut value = 0i64;
    while let Some(digit) = text.get(p).filter(|byte| byte.is_ascii_digit()) {
        value = (value * 10 + (digit - b'0') as i64).min(i32::MAX as i64 + 1);
        p += 1;
    }
    if negative {
        (-value).max(i32::MIN as i64) as i32
    } else {
        value.min(i32::MAX as i64) as i32
    }
}

/// `GetFITSPixel`: a sample of `depth` bits in the image's byte order;
/// zero (a byte: -1) past the end.
fn sample(blob: &mut Blob, depth: u32, bits_per_pixel: i32, big_endian: bool) -> f64 {
    let mut take = |count: usize| -> Option<u64> {
        let bytes = blob.read(count);
        if bytes.len() != count {
            return None;
        }
        let mut value = 0u64;
        for index in 0..count {
            let byte = if big_endian { bytes[index] } else { bytes[count - 1 - index] };
            value = (value << 8) | byte as u64;
        }
        Some(value)
    };
    match depth >> 3 {
        1 => take(1).map_or(-1.0, |value| value as f64),
        2 => take(2).unwrap_or(0) as u16 as i16 as f64,
        4 if bits_per_pixel > 0 => take(4).unwrap_or(0) as u32 as i32 as f64,
        4 => f32::from_bits(take(4).unwrap_or(0) as u32) as f64,
        8 if bits_per_pixel > 0 => take(8).unwrap_or(0) as i64 as f64,
        _ => f64::from_bits(take(8).unwrap_or(0)),
    }
}

/// `SetFITSUnsignedPixels` on a sample's `double`: the top bit of its
/// lowest byte flipped (big-endian files) or of its byte `depth / 8 - 1`.
fn flip_unsigned(pixel: f64, depth: u32, big_endian: bool) -> f64 {
    let mut bytes = pixel.to_le_bytes();
    let index = if big_endian { 0 } else { (depth >> 3) as usize - 1 };
    bytes[index] ^= 0x80;
    f64::from_le_bytes(bytes)
}

/// `GetQuantumRange` as `GetFITSPixelRange` reads it: through a signed
/// 64-bit offset.
fn pixel_range(depth: u64) -> f64 {
    if depth == 0 {
        return 0.0;
    }
    let shift = depth.min(64) - 1;
    let range = (1u64 << shift).wrapping_add((1u64 << shift) - 1);
    range as i64 as f64
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let mut info = FitsInfo {
        extend: false,
        simple: false,
        bits_per_pixel: 8,
        columns: 1,
        rows: 1,
        number_axes: 0,
        number_planes: 1,
        min_data: 0.0,
        max_data: 0.0,
        zero: 0.0,
        scale: 1.0,
        big_endian: true,
    };
    while !blob.eof {
        while !blob.eof {
            let keyword_bytes = blob.read(8);
            if keyword_bytes.len() != 8 {
                break;
            }
            let length = keyword_bytes.iter().position(|&byte| is_space(byte)).unwrap_or(8);
            let keyword = keyword_bytes[..length].to_ascii_lowercase();
            let value_bytes = blob.read(72);
            if value_bytes.len() != 72 {
                break;
            }
            let value_length = value_bytes.iter().position(|&byte| byte == 0).unwrap_or(72);
            let value = &value_bytes[..value_length];
            let mut p = 0;
            if value.first() == Some(&b'=') {
                p = 2;
                while p < value.len() && is_space(value[p]) {
                    p += 1;
                }
            }
            let text = value.get(p..).unwrap_or(&[]);
            let first = text.first().copied().unwrap_or(0);
            match keyword.as_slice() {
                b"end" => break,
                b"extend" => info.extend = first == b'T' || first == b't',
                b"simple" => info.simple = first == b'T' || first == b't',
                b"bitpix" => info.bits_per_pixel = string_to_long(text),
                b"naxis" => info.number_axes = string_to_long(text),
                b"naxis1" => info.columns = string_to_long(text),
                b"naxis2" => info.rows = string_to_long(text),
                b"naxis3" => info.number_planes = string_to_long(text),
                b"datamax" => info.max_data = interpret_locale_value(text, 0).0,
                b"datamin" => info.min_data = interpret_locale_value(text, 0).0,
                b"bzero" => info.zero = interpret_locale_value(text, 0).0,
                b"bscale" => info.scale = interpret_locale_value(text, 0).0,
                b"xendian" => info.big_endian = text.len() >= 3 && text[..3].eq_ignore_ascii_case(b"big"),
                _ => {}
            }
        }
        while !blob.tell().is_multiple_of(BLOCK_SIZE) {
            if blob.byte().is_none() {
                break;
            }
        }
        if !info.extend {
            break;
        }
        if !matches!(info.bits_per_pixel, 8 | 16 | 32 | 64 | -32 | -64) {
            bail!("ImproperImageHeader");
        }
        if info.columns <= 0 || info.rows <= 0 || info.number_axes < 0 || info.number_planes <= 0 {
            bail!("ImproperImageHeader");
        }
        let number_pixels = info.columns as i64 * info.rows as i64;
        if info.simple && (1..=4).contains(&info.number_axes) && number_pixels != 0 {
            break;
        }
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    let number_pixels = (info.columns as i64 as u64).wrapping_mul(info.rows as i64 as u64);
    if !info.simple || info.number_axes < 1 || info.number_axes > 4 || number_pixels == 0 || info.number_planes <= 0 {
        // No image in this header: its data is skipped, and what follows
        // is read as the image.
        let data = number_pixels.wrapping_mul(info.bits_per_pixel.unsigned_abs() as u64) / 8;
        let blocks = data.wrapping_add(BLOCK_SIZE - 1) / BLOCK_SIZE * BLOCK_SIZE;
        let target = blob.tell().wrapping_add(blocks);
        blob.seek(target as i64);
        blob.eof = false;
    }
    if info.number_planes <= 0 {
        bail!("ImproperImageHeader");
    }
    if info.columns <= 0 || info.rows <= 0 {
        bail!("NegativeOrZeroImageSize");
    }
    let columns = info.columns as usize;
    let rows = info.rows as usize;
    let depth = info.bits_per_pixel.unsigned_abs();
    let sample_bytes = match depth >> 3 {
        1 => 1,
        2 => 2,
        4 => 4,
        _ => 8,
    };
    // Samples past the end of the file end the read with an error.
    let remaining = blob.size().saturating_sub(blob.tell());
    if (columns as u64).saturating_mul(rows as u64).saturating_mul(sample_bytes) > remaining {
        bail!("UnexpectedEndOfFile");
    }

    if info.min_data == 0.0 && info.max_data == 0.0 {
        if info.bits_per_pixel == -32 || info.bits_per_pixel == -64 {
            // `GetFITSPixelExtrema`: every sample read, then back.
            let start = blob.tell();
            let mut minimum = f64::MAX;
            let mut maximum = -f64::MAX;
            for _ in 0..columns * rows {
                let pixel = sample(&mut blob, depth, info.bits_per_pixel, info.big_endian);
                if pixel < minimum {
                    minimum = pixel;
                }
                if pixel > maximum {
                    maximum = pixel;
                }
            }
            info.min_data = minimum;
            info.max_data = maximum;
            blob.seek(start as i64);
        } else {
            info.max_data = pixel_range(info.bits_per_pixel as i64 as u64);
        }
    } else {
        info.max_data = pixel_range(info.bits_per_pixel as i64 as u64);
    }
    let span = info.max_data - info.min_data;
    let reciprocal = if span > -1.0e-12 && span < 1.0e-12 { 1.0e12 } else { 1.0 / span };
    let scale = QUANTUM_RANGE * reciprocal;
    let mut data = vec![0.0f32; columns * rows];
    for y in (0..rows).rev() {
        for x in 0..columns {
            let mut pixel = sample(&mut blob, depth, info.bits_per_pixel, info.big_endian);
            if matches!(depth, 16 | 32 | 64) {
                pixel = flip_unsigned(pixel, depth, info.big_endian);
            }
            data[y * columns + x] = (scale * (info.scale * (pixel - info.min_data) + info.zero)) as f32;
        }
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    Ok(PixelCache::new(columns, rows, vec![PixelChannel::Gray], data, CacheColorspace::Gray, DEFAULT_GAMMA))
}
