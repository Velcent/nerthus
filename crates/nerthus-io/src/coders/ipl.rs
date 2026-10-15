//! Scanalytics IPL images as ImageMagick reads them (`coders/ipl.c`): an
//! "iiii" or "mmmm" byte-order mark, a "data" header, then the first
//! frame: grey rows, or red, green and blue planes, of bytes, 16- or
//! 32-bit integers (signed ones read unsigned) or floats.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::quantum::{self, Endian, Picture, QuantumInfo, QuantumType};
use crate::pixel_cache::{CacheColorspace, PixelCache, DEFAULT_GAMMA};

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let magic = blob.read(4);
    let big_endian = if magic.eq_ignore_ascii_case(b"iiii") {
        false
    } else if magic.eq_ignore_ascii_case(b"mmmm") {
        true
    } else {
        bail!("ImproperImageHeader");
    };
    blob.read(8);
    if !blob.read(4).eq_ignore_ascii_case(b"data") {
        bail!("ImproperImageHeader");
    }
    let long = |blob: &mut Blob| if big_endian { blob.msb_u32() } else { blob.lsb_u32() };
    let _size = long(&mut blob);
    let width = long(&mut blob);
    let height = long(&mut blob);
    if width == 0 || height == 0 {
        bail!("ImproperImageHeader");
    }
    let colors = long(&mut blob);
    let z = long(&mut blob);
    let _time = long(&mut blob);
    let byte_type = long(&mut blob);
    let (depth, float) = match byte_type {
        0 | 5 => (8, false),
        1 | 2 | 6 => (16, false),
        3 => (32, false),
        4 => (32, true),
        10 => (64, true),
        _ => (16, false),
    };
    let extent = (width as u64).saturating_mul(height as u64).saturating_mul(z as u64).saturating_mul(depth as u64 / 8);
    if extent > blob.size() {
        bail!("InsufficientImageDataInFile");
    }
    let columns = width as usize;
    let rows = height as usize;
    let gray = colors != 3;
    let info = QuantumInfo { endian: if big_endian { Endian::Msb } else { Endian::Lsb }, ..QuantumInfo::new(depth, float) };
    let mut picture = Picture::new(columns, rows, gray, false, false, false, 0);
    let length = columns * depth as usize / 8;
    let planes: &[QuantumType] = if colors == 1 { &[QuantumType::Gray] } else { &[QuantumType::Red, QuantumType::Green, QuantumType::Blue] };
    let remaining = blob.size().saturating_sub(blob.tell());
    if (length as u64).saturating_mul((rows * planes.len()) as u64) > remaining {
        bail!("UnexpectedEndOfFile");
    }
    for &kind in planes {
        for y in 0..rows {
            let row = blob.read(length);
            quantum::import(&mut picture, &info, kind, y * columns, columns, row)?;
        }
    }
    let (colorspace, image_kind) = if gray { (CacheColorspace::Gray, QuantumType::Gray) } else { (CacheColorspace::SRgb, QuantumType::Rgb) };
    let image_type = quantum::image_type(image_kind, depth);
    Ok(picture.into_cache(gray, colorspace, DEFAULT_GAMMA, image_type))
}
