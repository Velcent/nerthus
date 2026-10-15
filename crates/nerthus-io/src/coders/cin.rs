//! Kodak Cineon images as ImageMagick reads them (`coders/cin.c`):
//! big-endian headers (the little-endian magic isn't accepted), then RGB
//! (or, with one channel, grey into the red of a white image) samples of
//! the first channel's depth in filled 32-bit words, in Log colourspace.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::quantum::{self, Endian, Picture, QuantumInfo, QuantumType};
use crate::pixel_cache::{CacheColorspace, PixelCache, DEFAULT_GAMMA};

const MAGIC: [u8; 4] = [0x80, 0x2a, 0x5f, 0xd7];

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

/// CIN's `GetBytesPerRow` (always padded).
fn bytes_per_row(columns: usize, samples: usize, bits: usize) -> usize {
    match bits {
        10 => 4 * (32 * (samples * columns).div_ceil(3)).div_ceil(32),
        12 => 2 * (16 * samples * columns).div_ceil(16),
        16 => 2 * ((samples * columns * bits + 8) / 16),
        64 => 8 * (samples * columns * bits).div_ceil(64),
        _ => 4 * (samples * columns * bits).div_ceil(32),
    }
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    if blob.read(4) != MAGIC {
        bail!("ImproperImageHeader");
    }
    let image_offset = blob.msb_u32();
    if image_offset < 712 {
        bail!("ImproperImageHeader");
    }
    let _generic_length = blob.msb_u32();
    let _industry_length = blob.msb_u32();
    let user_length = blob.msb_u32();
    let _file_size = blob.msb_u32();
    blob.read(8 + 100 + 12 + 12 + 36);
    let _orientation = blob.byte();
    let number_channels = blob.byte().unwrap_or(0xff);
    blob.read(2);
    let mut depth = 0u32;
    let mut columns = 0usize;
    let mut rows = 0usize;
    for channel in 0..8 {
        blob.read(2);
        let bits = blob.byte().unwrap_or(0xff);
        blob.read(1);
        let pixels_per_line = blob.msb_u32();
        let lines_per_image = blob.msb_u32();
        blob.read(16);
        if channel == 0 {
            depth = bits as u32;
            columns = pixels_per_line as usize;
            rows = lines_per_image as usize;
        }
    }
    // Chromaticities, label, reserve; data format; origination.
    blob.read(32 + 200 + 28);
    blob.read(32);
    blob.read(312);
    let mut offset: u64 = 1024;
    if image_offset > 2048 && user_length != 0 {
        offset += blob.read(1024).len() as u64;
        if user_length as u64 > blob.size() {
            bail!("InsufficientImageDataInFile");
        }
        offset += blob.read(user_length as usize).len() as u64;
    }
    if (columns as u64 * rows as u64) / 8 > blob.size() {
        bail!("InsufficientImageDataInFile");
    }
    while offset < image_offset as u64 {
        if blob.byte().is_none() {
            break;
        }
        offset += 1;
    }
    if offset < image_offset as u64 {
        bail!("ImproperImageHeader");
    }
    if columns == 0 || rows == 0 {
        bail!("NegativeOrZeroImageSize");
    }

    let (kind, samples) = if number_channels == 1 { (QuantumType::Gray, 1) } else { (QuantumType::Rgb, 3) };
    let length = bytes_per_row(columns, samples, depth as usize);
    let remaining = blob.size().saturating_sub(blob.tell());
    if (length as u64).saturating_mul(rows as u64) > remaining {
        bail!("UnexpectedEndOfFile");
    }
    let mut info = QuantumInfo::new(depth, false);
    info.endian = Endian::Msb;
    info.quantum = 32;
    info.pack = false;
    // `SetImageBackgroundColor`: white.
    let mut picture = Picture::new(columns, rows, false, false, false, false, 0);
    picture.data.fill(65535.0);
    for y in 0..rows {
        let row = blob.read(length);
        quantum::import(&mut picture, &info, kind, y * columns, columns, row)?;
    }
    let image_type = quantum::image_type(kind, depth);
    Ok(picture.into_cache(false, CacheColorspace::Log, DEFAULT_GAMMA, image_type))
}
