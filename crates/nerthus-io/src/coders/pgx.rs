//! JPEG 2000 PGX test images as ImageMagick reads them (`coders/pgx.c`): a
//! "PG ML +8 width height" line, then grey samples of that precision,
//! bit-packed, big-endian whatever the byte-order field says (IM only
//! notes "ML", and an unset byte order reads big-endian).

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::quantum::{self, Endian, Picture, QuantumInfo, QuantumType};
use super::text::is_space;
use crate::pixel_cache::{CacheColorspace, PixelCache, DEFAULT_GAMMA};

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

/// `%d`: optional blanks and sign, then digits, into a 32-bit `int`.
fn scan_int(line: &[u8], p: &mut usize) -> Option<i32> {
    while *p < line.len() && is_space(line[*p]) {
        *p += 1;
    }
    let negative = line.get(*p) == Some(&b'-');
    if matches!(line.get(*p), Some(b'+' | b'-')) {
        *p += 1;
    }
    let start = *p;
    let mut value = 0i64;
    while let Some(digit) = line.get(*p).filter(|byte| byte.is_ascii_digit()) {
        value = value.wrapping_mul(10).wrapping_add((digit - b'0') as i64);
        *p += 1;
    }
    if *p == start {
        return None;
    }
    let value = if negative { value.wrapping_neg() } else { value };
    Some(value as i32)
}

/// `%[set]`: one or more characters of the set.
fn scan_set(line: &[u8], p: &mut usize, set: &[u8]) -> bool {
    let start = *p;
    while *p < line.len() && set.contains(&line[*p]) {
        *p += 1;
    }
    *p > start
}

/// `sscanf("PG%[ \t]%2s%[ \t+-]%d%[ \t]%d%[ \t]%d")`: the precision,
/// width and height when all eight fields match.
fn scan_header(line: &[u8]) -> Option<(i32, i32, i32)> {
    if !line.starts_with(b"PG") {
        return None;
    }
    let mut p = 2;
    if !scan_set(line, &mut p, b" \t") {
        return None;
    }
    let start = p;
    while p < line.len() && p - start < 2 && !is_space(line[p]) {
        p += 1;
    }
    if p == start {
        return None;
    }
    if !scan_set(line, &mut p, b" \t+-") {
        return None;
    }
    let precision = scan_int(line, &mut p)?;
    if !scan_set(line, &mut p, b" \t") {
        return None;
    }
    let width = scan_int(line, &mut p)?;
    if !scan_set(line, &mut p, b" \t") {
        return None;
    }
    let height = scan_int(line, &mut p)?;
    Some((precision, width, height))
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let line = blob.string().context("ImproperImageHeader")?;
    let Some((precision, width, height)) = scan_header(&line) else {
        bail!("ImproperImageHeader");
    };
    if width == 0 || height == 0 {
        bail!("ImproperImageHeader");
    }
    if width < 0 || height < 0 || precision <= 0 || precision > 64 {
        bail!("WidthOrHeightExceedsLimit");
    }
    let columns = width as usize;
    let rows = height as usize;
    let info = QuantumInfo { endian: Endian::Msb, ..QuantumInfo::new(precision as u32, false) };
    let mut picture = Picture::new(columns, rows, true, false, false, false, 0);
    let length = info.extent(&picture, QuantumType::Gray);
    let remaining = blob.size().saturating_sub(blob.tell());
    if (length as u64).saturating_mul(rows as u64) > remaining {
        bail!("UnexpectedEndOfFile");
    }
    for y in 0..rows {
        let row = blob.read(length);
        quantum::import(&mut picture, &info, QuantumType::Gray, y * columns, columns, row)?;
    }
    let image_type = quantum::image_type(QuantumType::Gray, info.depth);
    Ok(picture.into_cache(true, CacheColorspace::Gray, DEFAULT_GAMMA, image_type))
}
