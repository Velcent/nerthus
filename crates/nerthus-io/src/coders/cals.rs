//! CALS raster images as ImageMagick reads them (`coders/cals.c`): up to
//! sixteen 128-byte header records (the size from `rpelcnt:`), then a
//! raw CCITT Group 4 stream, which IM wraps in a TIFF (one strip, 1 bit,
//! min-is-black) and reads with its TIFF reader, as is done here.

use std::path::Path;

use anyhow::{Context, Result};

use super::blob::Blob;
use crate::pixel_cache::PixelCache;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

/// `sscanf("%lu,%lu")` into Windows (32-bit) `unsigned long`s.
fn scan_pair(text: &[u8]) -> (Option<u32>, Option<u32>) {
    let mut p = 0;
    let number = |p: &mut usize| -> Option<u32> {
        while *p < text.len() && super::text::is_space(text[*p]) {
            *p += 1;
        }
        let negative = text.get(*p) == Some(&b'-');
        if matches!(text.get(*p), Some(b'+' | b'-')) {
            *p += 1;
        }
        let start = *p;
        let mut value = 0u64;
        while let Some(digit) = text.get(*p).filter(|byte| byte.is_ascii_digit()) {
            value = value.wrapping_mul(10).wrapping_add((digit - b'0') as u64);
            *p += 1;
        }
        if *p == start {
            return None;
        }
        Some(if negative { (value as u32).wrapping_neg() } else { value as u32 })
    };
    let first = number(&mut p);
    if first.is_none() || text.get(p) != Some(&b',') {
        return (first, None);
    }
    p += 1;
    (first, number(&mut p))
}

/// A little-endian TIFF around a raw Group 4 strip, as `ReadGROUP4Image`
/// writes it.
fn group4_tiff(width: u32, height: u32, strip: &[u8]) -> Vec<u8> {
    const ENTRIES: u16 = 10;
    let data_offset = 8 + 2 + ENTRIES as u32 * 12 + 4;
    let mut tiff = Vec::with_capacity(data_offset as usize + strip.len());
    tiff.extend_from_slice(b"II*\0");
    tiff.extend(8u32.to_le_bytes());
    tiff.extend(ENTRIES.to_le_bytes());
    let mut entry = |tag: u16, kind: u16, value: u32| {
        tiff.extend(tag.to_le_bytes());
        tiff.extend(kind.to_le_bytes());
        tiff.extend(1u32.to_le_bytes());
        if kind == 3 {
            tiff.extend((value as u16).to_le_bytes());
            tiff.extend([0, 0]);
        } else {
            tiff.extend(value.to_le_bytes());
        }
    };
    entry(256, 4, width);
    entry(257, 4, height);
    entry(258, 3, 1);
    entry(259, 3, 4);
    entry(262, 3, 1);
    entry(273, 4, data_offset);
    entry(274, 3, 1);
    entry(277, 3, 1);
    entry(278, 4, height);
    entry(279, 4, strip.len() as u32);
    tiff.extend(0u32.to_le_bytes());
    tiff.extend_from_slice(strip);
    tiff
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let mut width = 0u32;
    let mut height = 0u32;
    for _ in 0..16 {
        let record = blob.read(128);
        if record.len() != 128 {
            break;
        }
        if record.len() >= 8 && record[..8].eq_ignore_ascii_case(b"rpelcnt:") {
            let (first, second) = scan_pair(&record[8..]);
            if let Some(value) = first {
                width = value;
            }
            if let Some(value) = second {
                height = value;
            }
        }
    }
    let rest = blob.read(usize::MAX);
    let tiff = group4_tiff(width, height, rest);
    super::tiff::decode(&tiff)
}
