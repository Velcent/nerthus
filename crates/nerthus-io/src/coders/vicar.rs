//! VICAR images as ImageMagick reads them (`coders/vicar.c`): a label of
//! `KEYWORD=value` pairs (read until both `NL`/`LINES` and `NS` are known,
//! then skipped to `LBLSIZE`), then big-endian grey samples: `BYTE`,
//! `HALF` (read unsigned), `FULL` and `REAL` as 32-bit floats, anything
//! else as 64-bit floats.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::quantum::{self, Endian, Picture, QuantumInfo, QuantumType};
use crate::pixel_cache::{CacheColorspace, PixelCache, DEFAULT_GAMMA};

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

fn is_graph(c: Option<u8>) -> bool {
    c.is_some_and(|byte| byte.is_ascii_graphic())
}

fn is_alnum(c: Option<u8>) -> bool {
    c.is_some_and(|byte| byte.is_ascii_alphanumeric())
}

fn is_space(c: Option<u8>) -> bool {
    c.is_some_and(super::text::is_space)
}

/// `strtol`/`strtoul` into a Windows (32-bit) `long`: the leading number,
/// wrapping.
fn string_to_long(value: &[u8]) -> i64 {
    let text = String::from_utf8_lossy(value);
    let trimmed = text.trim_start();
    let (negative, digits) = match trimmed.as_bytes().first() {
        Some(b'-') => (true, &trimmed[1..]),
        Some(b'+') => (false, &trimmed[1..]),
        _ => (false, trimmed),
    };
    let mut number = 0i64;
    for byte in digits.bytes().take_while(u8::is_ascii_digit) {
        number = number.saturating_mul(10).saturating_add((byte - b'0') as i64);
    }
    let number = number.min(i32::MAX as i64 + 1);
    if negative { -number } else { number.min(i32::MAX as i64) }
}

/// `strtoul` into a Windows (32-bit) `unsigned long`: saturating, a minus
/// sign wrapping.
fn string_to_unsigned_long(value: &[u8]) -> usize {
    let text = String::from_utf8_lossy(value);
    let trimmed = text.trim_start();
    let (negative, digits) = match trimmed.as_bytes().first() {
        Some(b'-') => (true, &trimmed[1..]),
        Some(b'+') => (false, &trimmed[1..]),
        _ => (false, trimmed),
    };
    let mut number = 0u64;
    for byte in digits.bytes().take_while(u8::is_ascii_digit) {
        number = number.saturating_mul(10).saturating_add((byte - b'0') as u64);
    }
    if number > u32::MAX as u64 {
        return u32::MAX as usize;
    }
    let number = number as u32;
    (if negative { number.wrapping_neg() } else { number }) as usize
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let mut c = blob.byte();
    let mut count = 1usize;
    if c.is_none() {
        bail!("ImproperImageHeader");
    }
    let mut format = b"byte".to_vec();
    let mut length = 0usize;
    let mut columns = 0usize;
    let mut rows = 0usize;
    while is_graph(c) && (columns == 0 || rows == 0) {
        if !is_alnum(c) {
            c = blob.byte();
            count += 1;
        } else {
            let offset = blob.tell() as usize - 1;
            let mut keyword = Vec::new();
            loop {
                if keyword.len() < 4095 {
                    keyword.push(c.unwrap_or(0));
                }
                c = blob.byte();
                if c.is_none() {
                    break;
                }
                count += 1;
                if !(is_alnum(c) || c == Some(b'_')) {
                    break;
                }
            }
            let mut value_expected = false;
            while is_space(c) || c == Some(b'=') {
                if c == Some(b'=') {
                    value_expected = true;
                }
                c = blob.byte();
                if c.is_none() {
                    break;
                }
                count += 1;
            }
            if !value_expected {
                continue;
            }
            let mut value = Vec::new();
            let close = match c {
                Some(b'\'') => Some(b'\''),
                Some(b'"') => Some(b'"'),
                Some(b'(') => Some(b')'),
                _ => None,
            };
            if let Some(close) = close {
                c = blob.byte();
                count += 1;
                while c != Some(close) {
                    if value.len() < 4095 {
                        value.push(c.unwrap_or(0xff));
                    }
                    c = blob.byte();
                    if c.is_none() {
                        break;
                    }
                    count += 1;
                }
            } else {
                while is_alnum(c) {
                    if value.len() < 4095 {
                        value.push(c.unwrap_or(0));
                    }
                    c = blob.byte();
                    if c.is_none() {
                        break;
                    }
                    count += 1;
                }
            }
            if keyword.eq_ignore_ascii_case(b"END") {
                break;
            }
            if keyword.eq_ignore_ascii_case(b"FORMAT") {
                format = value.clone();
            }
            if keyword.eq_ignore_ascii_case(b"LABEL_RECORDS") {
                length = length.wrapping_mul(string_to_long(&value) as usize);
            }
            if keyword.eq_ignore_ascii_case(b"LBLSIZE") {
                length = (string_to_long(&value) + offset as i64) as usize;
            }
            if keyword.eq_ignore_ascii_case(b"LINES") || keyword.eq_ignore_ascii_case(b"NL") {
                rows = string_to_unsigned_long(&value);
            }
            if keyword.eq_ignore_ascii_case(b"NS") {
                columns = string_to_unsigned_long(&value);
            }
        }
        if c.is_none() {
            break;
        }
        while is_space(c) {
            c = blob.byte();
            if c.is_none() {
                break;
            }
            count += 1;
        }
    }
    while count < length {
        if blob.byte().is_none() {
            break;
        }
        count += 1;
    }
    if columns == 0 || rows == 0 {
        bail!("NegativeOrZeroImageSize");
    }
    let info = if format.eq_ignore_ascii_case(b"byte") {
        QuantumInfo::new(8, false)
    } else if format.eq_ignore_ascii_case(b"half") {
        QuantumInfo::new(16, false)
    } else if format.eq_ignore_ascii_case(b"full") || format.eq_ignore_ascii_case(b"real") {
        QuantumInfo::new(32, true)
    } else {
        QuantumInfo::new(64, true)
    };
    let info = QuantumInfo { endian: Endian::Msb, ..info };
    let row_length = info.depth as usize / 8 * columns;
    // Rows the file can't fill end the read with an error.
    let remaining = blob.size().saturating_sub(blob.tell()) as usize;
    if row_length.saturating_mul(rows) > remaining {
        bail!("UnexpectedEndOfFile");
    }
    let mut picture = Picture::new(columns, rows, true, false, false, false, 0);
    for y in 0..rows {
        let row = blob.read(row_length);
        quantum::import(&mut picture, &info, QuantumType::Gray, y * columns, columns, row)?;
    }
    let image_type = quantum::image_type(QuantumType::Gray, info.depth);
    Ok(picture.into_cache(true, CacheColorspace::Gray, DEFAULT_GAMMA, image_type))
}
