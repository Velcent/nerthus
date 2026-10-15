//! X pixmaps as ImageMagick reads them (`coders/xpm.c`): the quoted strings
//! of the file, the first with the size, colour count and characters a
//! pixel, then a colour line each (the `c` colour by preference, then
//! `g`, `g4`, `m`, `b`, `s`; names as XPM knows them), then the pixel rows.
//! PseudoClass sRGB, or DirectClass with alpha when a colour is `None`.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::color::{self, Compliance};
use super::colormap;
use super::text::{is_space, joined_lines};
use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

const PATH_EXTENT: usize = 4096;
const TARGETS: [&[u8]; 6] = [b"c ", b"g ", b"g4 ", b"m ", b"b ", b"s "];

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

fn at(bytes: &[u8], index: usize) -> u8 {
    bytes.get(index).copied().unwrap_or(0)
}

/// A C string's length from `start`.
fn c_len(bytes: &[u8], start: usize) -> usize {
    bytes.get(start..).map_or(0, |rest| rest.iter().position(|&byte| byte == 0).unwrap_or(rest.len()))
}

/// `GetNextToken` on a quoted string: its characters, `\"` and `\\`
/// unescaped, to the closing quote.
fn quoted_token(buffer: &[u8], start: usize) -> Vec<u8> {
    let mut token = Vec::new();
    let mut p = start + 1;
    while at(buffer, p) != 0 {
        if at(buffer, p) == b'\\' && matches!(at(buffer, p + 1), b'"' | b'\\') {
            p += 1;
        } else if at(buffer, p) == b'"' {
            break;
        }
        if token.len() < PATH_EXTENT - 1 {
            token.push(at(buffer, p));
        }
        if p - start >= PATH_EXTENT - 1 {
            break;
        }
        p += 1;
    }
    token
}

/// `sscanf("%lu %lu %lu %lu")` into Windows (32-bit) `unsigned long`s: the
/// values read, in order.
fn scan_values(text: &[u8]) -> Vec<u32> {
    let mut values = Vec::new();
    let mut p = 0;
    while values.len() < 4 {
        while is_space(at(text, p)) {
            p += 1;
        }
        let negative = at(text, p) == b'-';
        if matches!(at(text, p), b'+' | b'-') {
            p += 1;
        }
        if !at(text, p).is_ascii_digit() {
            break;
        }
        let mut value = 0u64;
        while at(text, p).is_ascii_digit() {
            value = value.wrapping_mul(10).wrapping_add((at(text, p) - b'0') as u64);
            p += 1;
        }
        let value = if negative { value.wrapping_neg() } else { value };
        values.push(value as u32);
    }
    values
}

/// `CopyXPMColor`: up to `length` characters from `source` into
/// `destination` until a quote or the end, ended with a NUL unless the
/// count ran out exactly there. Returns how many were copied.
fn copy_color(destination: &mut [u8], source: &[u8], start: usize, length: usize) -> usize {
    let mut remaining = length;
    let mut p = start;
    let mut d = 0;
    loop {
        if remaining == 0 {
            remaining = usize::MAX;
            break;
        }
        remaining -= 1;
        let byte = at(source, p);
        if byte == 0 || byte == b'"' {
            break;
        }
        destination[d] = byte;
        d += 1;
        p += 1;
    }
    if remaining != 0 {
        destination[d] = 0;
    }
    p - start
}

/// `ParseXPMColor` from the start of a colour line: where the first of
/// the keys (after a blank, in their order of preference) is.
fn find_key(text: &[u8], color: usize) -> Option<usize> {
    if at(text, color) == 0 {
        return None;
    }
    for target in TARGETS {
        let mut p = color;
        while at(text, p) != 0 {
            if at(text, p) == b'\n' {
                break;
            }
            if at(text, p) == target[0] && is_space(at(text, p.wrapping_sub(1))) {
                let matched = target.iter().enumerate().all(|(offset, &byte)| at(text, p + offset) == byte);
                if matched {
                    return Some(p);
                }
            }
            p += 1;
        }
    }
    None
}

/// `ParseXPMColor` within a colour: where the next key starts (or the
/// end).
fn find_next_key(target: &[u8]) -> Option<usize> {
    if at(target, 0) == 0 {
        return None;
    }
    let mut p = 1;
    while at(target, p) != 0 {
        if at(target, p) == b'\n' {
            break;
        }
        if is_space(at(target, p - 1)) && !is_space(at(target, p)) {
            let starts_key = TARGETS.iter().any(|key| at(target, p) == key[0] && at(target, p + 1) == key[1]);
            if starts_key {
                return Some(p);
            }
        }
        p += 1;
    }
    Some(p)
}

/// `StripMagickString`: blanks and one quote off each end (a string of one
/// character stays), newlines into spaces.
fn strip_magick_string(text: &[u8]) -> Vec<u8> {
    if text.len() <= 1 {
        return text.to_vec();
    }
    let mut p = 0;
    while is_space(at(text, p)) {
        p += 1;
    }
    if matches!(at(text, p), b'\'' | b'"') {
        p += 1;
    }
    let mut q = text.len() - 1;
    while is_space(text[q]) && q > p {
        q -= 1;
    }
    if q > p && matches!(text[q], b'\'' | b'"') {
        q -= 1;
    }
    let kept = text.get(p..q + 1).unwrap_or(&[]);
    kept.iter().map(|&byte| if byte == b'\n' { b' ' } else { byte }).collect()
}

fn next_line(text: &[u8], start: usize) -> Option<usize> {
    let rest = text.get(start..)?;
    let length = c_len(text, start);
    rest[..length].iter().position(|&byte| byte == b'\n').map(|offset| start + offset + 1)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let buffer = joined_lines(&mut blob);

    // The first quoted string with four numbers: the size, colour count
    // and characters a pixel.
    let mut values = Vec::new();
    let mut start = None;
    for (index, &byte) in buffer.iter().enumerate() {
        if byte != b'"' {
            continue;
        }
        let token = quoted_token(&buffer, index);
        values = scan_values(&token);
        if values.len() == 4 {
            start = Some(index);
            break;
        }
    }
    let Some(start) = start else {
        bail!("ImproperImageHeader");
    };
    let [columns, rows, colors, width] = [values[0] as usize, values[1] as usize, values[2] as usize, values[3] as usize];
    if width == 0 || width > 3 || columns == 0 || rows == 0 || colors == 0 || colors > 65536 {
        bail!("ImproperImageHeader");
    }

    // The quoted strings from there, a line each (each keeps a quote at
    // its end).
    let mut text = Vec::new();
    let mut active = false;
    let mut p = start;
    while p < buffer.len() {
        let byte = buffer[p];
        p += 1;
        if byte == b'"' {
            if active {
                text.push(b'\n');
            }
            active = !active;
        }
        if active {
            text.push(at(&buffer, p));
        }
    }
    if active {
        bail!("UnexpectedEndOfFile");
    }
    let text_length = c_len(&text, 0);
    text.truncate(text_length);

    // The colours.
    let mut colormap = colormap::acquire(colors).context("MemoryAllocationFailed")?;
    let mut has_alpha = false;
    let mut keys: HashMap<Vec<u8>, usize> = HashMap::new();
    let mut key = [0u8; PATH_EXTENT];
    let mut line = 0;
    let mut next = next_line(&text, 0);
    let mut read_colors = 0;
    while read_colors < colors {
        let Some(line_start) = next else {
            break;
        };
        line = line_start;
        next = next_line(&text, line);
        let Some(next_start) = next else {
            break;
        };
        let length = width.min(PATH_EXTENT - 1);
        if copy_color(&mut key, &text, line, length) != length {
            break;
        }
        keys.insert(key[..c_len(&key, 0)].to_vec(), read_colors);

        let mut target = [0u8; PATH_EXTENT];
        target[..4].copy_from_slice(b"gray");
        let mut found = None;
        if c_len(&text, line) > width {
            found = find_key(&text, line + width);
        }
        if let Some(mut q) = found {
            while !is_space(at(&text, q)) && at(&text, q) != 0 {
                q += 1;
            }
            if q > next_start {
                break;
            }
            let span = (next_start - q).min(PATH_EXTENT - 1);
            copy_color(&mut target, &text, q, span);
            if let Some(symbolic) = find_next_key(&target) {
                target[symbolic] = 0;
            }
        }
        let name_length = c_len(&target, 0);
        let mut name = strip_magick_string(&target[..name_length]);
        if let Some(grey) = name.windows(4).position(|window| window == b"grey") {
            name[grey + 2] = b'a';
        }
        if name.eq_ignore_ascii_case(b"none") {
            has_alpha = true;
        }
        let Some(parsed) = color::query(&name, Compliance::Xpm) else {
            break;
        };
        colormap[read_colors] = [parsed.red, parsed.green, parsed.blue, if parsed.has_alpha { parsed.alpha } else { 65535.0 }];
        read_colors += 1;
    }
    if read_colors < colors {
        bail!("CorruptImage");
    }

    // The pixels, `width` characters each. Too few characters for them all
    // run out before the last row.
    if columns.saturating_mul(rows).saturating_mul(width) > text.len() {
        bail!("NotEnoughPixelData");
    }
    let pseudo = !has_alpha;
    let mut channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    channels.push(if pseudo { PixelChannel::Index } else { PixelChannel::Alpha });
    let mut data = Vec::with_capacity(columns * rows * 4);
    let length = width.min(PATH_EXTENT - 1);
    let mut p = line;
    for _ in 0..rows {
        p = next_line(&text, p).context("NotEnoughPixelData")?;
        for _ in 0..columns {
            let count = copy_color(&mut key, &text, p, length);
            if count != width {
                bail!("NotEnoughPixelData");
            }
            let index = keys.get(&key[..c_len(&key, 0)]).copied().unwrap_or(0);
            let entry = colormap[index];
            data.extend([entry[0] as f32, entry[1] as f32, entry[2] as f32]);
            data.push(if pseudo { index as f32 } else { entry[3] as f32 });
            p += count;
        }
    }
    let mut cache = PixelCache::new(columns, rows, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA);
    if pseudo {
        cache.colormap = colormap;
    }
    Ok(cache)
}
