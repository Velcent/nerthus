//! X bitmaps as ImageMagick reads them (`coders/xbm.c`): the `_width` and
//! `_height` defines, then the hex values of the `_bits[]` array (X10
//! `short` arrays or X11 `char` ones), least significant bit first. A
//! PseudoClass sRGB image whose colormap is white, then black.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

/// A little `sscanf`: literal words separated by whitespace that matches
/// any amount, then (`%s`) a word. Returns the word when everything before
/// it matched.
fn scan_word<'a>(line: &'a [u8], literals: &[&str]) -> Option<&'a [u8]> {
    let mut position = 0;
    let skip_space = |position: &mut usize| {
        while *position < line.len() && line[*position].is_ascii_whitespace() {
            *position += 1;
        }
    };
    for (index, literal) in literals.iter().enumerate() {
        if index > 0 {
            skip_space(&mut position);
        }
        let bytes = literal.as_bytes();
        if line.get(position..position + bytes.len()) != Some(bytes) {
            return None;
        }
        position += bytes.len();
    }
    skip_space(&mut position);
    let start = position;
    while position < line.len() && !line[position].is_ascii_whitespace() && position - start < 1024 {
        position += 1;
    }
    if position == start {
        return None;
    }
    Some(&line[start..position])
}

/// `sscanf(line, "#define %1024s %ld", …) == 2`: the name and value.
fn scan_define(line: &[u8]) -> Option<(Vec<u8>, i64)> {
    let name = scan_word(line, &["#define"])?;
    let name_end = name.as_ptr() as usize - line.as_ptr() as usize + name.len();
    let rest = &line[name_end..];
    let text = String::from_utf8_lossy(rest);
    let trimmed = text.trim_start();
    let mut end = 0;
    let bytes = trimmed.as_bytes();
    if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
        end += 1;
    }
    let digits_start = end;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end == digits_start {
        return None;
    }
    // A Windows `long`: 32 bits, wrapping.
    let mut value = 0u64;
    for &digit in &bytes[digits_start..end] {
        value = value.wrapping_mul(10).wrapping_add((digit - b'0') as u64);
    }
    if bytes[0] == b'-' {
        value = value.wrapping_neg();
    }
    Some((name.to_vec(), value as i32 as i64))
}

fn ends_with_ignore_case(name: &[u8], suffix: &str) -> bool {
    name.len() >= suffix.len() && name[name.len() - suffix.len()..].eq_ignore_ascii_case(suffix.as_bytes())
}

/// The hex digit table (`-1` for anything else).
fn hex_digit(byte: u8) -> i32 {
    match byte {
        b'0'..=b'9' => (byte - b'0') as i32,
        b'A'..=b'F' => (byte - b'A' + 10) as i32,
        b'a'..=b'f' => (byte - b'a' + 10) as i32,
        b'x' => 0,
        _ => -1,
    }
}

/// `XBMInteger`: after whitespace, the first character and the hex digits
/// that follow (a first character that is no digit wraps the value).
fn integer(blob: &mut Blob) -> i32 {
    let mut c;
    loop {
        let Some(byte) = blob.byte() else {
            return -1;
        };
        c = byte;
        if !matches!(c, b' ' | b'\t' | b'\n' | b'\r') {
            break;
        }
    }
    let mut value: u32 = 0;
    loop {
        if value <= (i32::MAX / 16) as u32 {
            value = value.wrapping_mul(16);
            let digit = hex_digit(c);
            if (value as i64) <= (i32::MAX as i64 - 1) - digit as i64 {
                value = value.wrapping_add(digit as u32);
            }
        }
        let Some(byte) = blob.byte() else {
            return -1;
        };
        c = byte;
        if hex_digit(c) < 0 {
            break;
        }
    }
    value as i32
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let mut width = 0i64;
    let mut height = 0i64;
    while let Some(line) = blob.string() {
        if let Some((name, value)) = scan_define(&line) {
            width = value;
            if name.len() >= 6 && ends_with_ignore_case(&name, "_width") {
                break;
            }
        }
    }
    while let Some(line) = blob.string() {
        if let Some((name, value)) = scan_define(&line) {
            height = value;
            if name.len() >= 7 && ends_with_ignore_case(&name, "_height") {
                break;
            }
        }
    }
    if width <= 0 || height <= 0 || blob.eof {
        bail!("ImproperImageHeader");
    }
    let width = width as usize;
    let height = height as usize;

    // Up to the `…_bits[]` array: X10 shorts or X11 chars. The seek is to
    // just after the last `{` of the line that stopped the scan (none at the
    // end of the file), counted from the line after the last line that
    // declared no array.
    let mut version = 11;
    let mut offset = blob.tell();
    let mut bits_line = Vec::new();
    while let Some(line) = blob.string() {
        let name = if let Some(name) = scan_word(&line, &["static", "short"]) {
            version = 10;
            name
        } else if let Some(name) = scan_word(&line, &["static", "unsigned", "char"]) {
            version = 11;
            name
        } else if let Some(name) = scan_word(&line, &["static", "char"]) {
            version = 11;
            name
        } else {
            offset = blob.tell();
            continue;
        };
        let tail = match name.iter().rposition(|&byte| byte == b'_') {
            Some(index) => &name[index + 1..],
            None => name,
        };
        if tail.eq_ignore_ascii_case(b"bits[]") {
            bits_line = line;
            break;
        }
    }
    if let Some(brace) = bits_line.iter().rposition(|&byte| byte == b'{') {
        blob.seek((offset + brace as u64 + 1) as i64);
    }

    let padding = usize::from(!width.is_multiple_of(16) && width % 16 < 9 && version == 10);
    let bytes_per_line = width.div_ceil(8) + padding;
    let length = bytes_per_line.checked_mul(height).context("ImproperImageHeader")?;
    let mut data = Vec::with_capacity(length);
    if version == 10 {
        let mut index = 0;
        while index < length {
            let value = integer(&mut blob);
            if value < 0 {
                bail!("ImproperImageHeader");
            }
            data.push(value as u8);
            if padding == 0 || !(index + 2).is_multiple_of(bytes_per_line) {
                data.push((value >> 8) as u8);
            }
            index += 2;
        }
    } else {
        for _ in 0..length {
            let value = integer(&mut blob);
            if value < 0 {
                bail!("ImproperImageHeader");
            }
            data.push(value as u8);
        }
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }

    let colormap = [[65535.0f64, 65535.0, 65535.0, 65535.0], [0.0, 0.0, 0.0, 65535.0]];
    let mut pixels = Vec::with_capacity(width * height * 4);
    let mut cursor = 0;
    for _ in 0..height {
        let mut bit = 0;
        let mut byte = 0u32;
        for _ in 0..width {
            if bit == 0 {
                byte = data.get(cursor).copied().unwrap_or(0) as u32;
                cursor += 1;
            }
            let index = (byte & 0x01) as usize;
            let entry = colormap[index];
            pixels.extend([entry[0] as f32, entry[1] as f32, entry[2] as f32, index as f32]);
            bit += 1;
            byte >>= 1;
            if bit == 8 {
                bit = 0;
            }
        }
    }
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Index];
    let mut cache = PixelCache::new(width, height, channels, pixels, CacheColorspace::SRgb, DEFAULT_GAMMA);
    cache.colormap = colormap.to_vec();
    Ok(cache)
}
