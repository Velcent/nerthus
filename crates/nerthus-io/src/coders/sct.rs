//! Scitex continuous-tone images as ImageMagick reads them (`coders/sct.c`):
//! "CT" control and parameter blocks (the size as text), then each row's
//! separations: one is grey, three RGB, four (mask 0x0f) inverted CMYK;
//! rows of odd width are padded. The background starts white.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::color;
use super::text::interpret_locale_value;
use crate::pixel_cache::{char_quantum, CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

/// `StringToUnsignedLong` (`strtoul`, a 32-bit `unsigned long`).
fn string_to_unsigned_long(text: &[u8]) -> u64 {
    let mut p = 0;
    while p < text.len() && super::text::is_space(text[p]) {
        p += 1;
    }
    let negative = text.get(p) == Some(&b'-');
    if matches!(text.get(p), Some(b'+' | b'-')) {
        p += 1;
    }
    let mut value = 0u64;
    let mut overflow = false;
    while let Some(digit) = text.get(p).filter(|byte| byte.is_ascii_digit()) {
        value = value * 10 + (digit - b'0') as u64;
        if value > u32::MAX as u64 {
            overflow = true;
            value = u32::MAX as u64;
        }
        p += 1;
    }
    if overflow {
        return u32::MAX as u64;
    }
    if negative { (value as u32).wrapping_neg() as u64 } else { value }
}

fn c_text(bytes: &[u8]) -> &[u8] {
    let end = bytes.iter().position(|&byte| byte == 0).unwrap_or(bytes.len());
    &bytes[..end]
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    blob.read(80);
    let magick = blob.read(2).to_vec();
    let is = |word: &[u8]| magick.eq_ignore_ascii_case(word);
    if !(is(b"CT") || is(b"LW") || is(b"BM") || is(b"PG") || is(b"TX")) {
        bail!("ImproperImageHeader");
    }
    if !is(b"CT") {
        bail!("OnlyContinuousTonePictureSupported");
    }
    blob.read(174);
    blob.read(768);
    let _units = blob.byte();
    let separations = blob.byte().map_or(usize::MAX, usize::from);
    let mask = blob.msb_u16();
    let height = interpret_locale_value(c_text(blob.read(14)), 0).0;
    let width = interpret_locale_value(c_text(blob.read(14)), 0).0;
    let rows = string_to_unsigned_long(c_text(blob.read(12))) as usize;
    let columns = string_to_unsigned_long(c_text(blob.read(12))) as usize;
    blob.read(200);
    blob.read(768);
    let cmyk = mask == 0x0f;
    if columns < 1 || rows < 1 || width < 1.0e-12 || height < 1.0e-12 {
        bail!("ImproperImageHeader");
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    if (rows as u64).saturating_mul(columns as u64).saturating_mul(separations as u64) > blob.size() {
        bail!("UnexpectedEndOfFile");
    }
    let mut layout = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    if cmyk {
        layout.push(PixelChannel::Black);
    }
    let stride = layout.len();
    let background: Vec<f32> = if cmyk {
        color::srgb_to_cmyk(65535.0, 65535.0, 65535.0).iter().map(|&value| value as f32).collect()
    } else {
        vec![65535.0; 3]
    };
    let mut data = Vec::with_capacity(columns * rows * stride);
    for _ in 0..columns * rows {
        data.extend_from_slice(&background);
    }
    'rows: for y in 0..rows {
        for separation in 0..separations {
            for x in 0..columns {
                let Some(byte) = blob.byte() else {
                    break 'rows;
                };
                let mut pixel = char_quantum(byte);
                if cmyk {
                    pixel = 65535.0 - pixel;
                }
                let base = (y * columns + x) * stride;
                match separation {
                    0 => data[base..base + 3].fill(pixel),
                    1 => data[base + 1] = pixel,
                    2 => data[base + 2] = pixel,
                    3 if cmyk => data[base + 3] = pixel,
                    _ => {}
                }
            }
            if !columns.is_multiple_of(2) && blob.byte().is_none() {
                break 'rows;
            }
        }
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    let colorspace = if cmyk { CacheColorspace::Cmyk } else { CacheColorspace::SRgb };
    Ok(PixelCache::new(columns, rows, layout, data, colorspace, DEFAULT_GAMMA))
}
