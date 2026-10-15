//! Simple File Format images as ImageMagick reads them (`coders/sf3.c`):
//! the first layer, grey, linear RGB or CMYK samples of 1 to 8 bytes,
//! little-endian. IM never gives the image an alpha channel, so it imports
//! only the colour samples' count from each row (BGR orders are read as
//! RGB), as here.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::quantum::{self, Endian, Picture, QuantumInfo, QuantumType};
use crate::pixel_cache::{CacheColorspace, PixelCache, DEFAULT_GAMMA};

const MAGIC: &[u8] = b"\x81SF3\x00\xe0\xd0\r\n\n\x03";

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let header = blob.read(16);
    if header.len() != 16 || &header[..11] != MAGIC {
        bail!("ImproperImageHeader");
    }
    let columns = blob.lsb_u32() as usize;
    let rows = blob.lsb_u32() as usize;
    let layers = blob.lsb_u32();
    if columns == 0 || rows == 0 || layers == 0 {
        bail!("NegativeOrZeroImageSize");
    }
    let channels = blob.byte().unwrap_or(0xff);
    let format = blob.byte().unwrap_or(0xff);
    let (colorspace, kind, gamma) = match channels {
        0x01 | 0x02 | 0x12 => (CacheColorspace::Gray, QuantumType::Gray, DEFAULT_GAMMA),
        0x03 | 0x04 | 0x13 | 0x14 | 0x24 | 0x34 => (CacheColorspace::Rgb, QuantumType::Rgb, 1.0),
        0x44 | 0x54 => (CacheColorspace::Cmyk, QuantumType::Cmyk, DEFAULT_GAMMA),
        _ => bail!("ImproperImageHeader"),
    };
    let float = match format {
        0x01 | 0x02 | 0x04 | 0x08 | 0x11 | 0x12 | 0x14 | 0x18 => false,
        0x22 | 0x24 | 0x28 => true,
        _ => bail!("ImproperImageHeader"),
    };
    let depth = (format & 0x0f) as u32 * 8;
    let info = QuantumInfo { endian: Endian::Lsb, ..QuantumInfo::new(depth, float) };
    let gray = colorspace == CacheColorspace::Gray;
    let cmyk = colorspace == CacheColorspace::Cmyk;
    let mut picture = Picture::new(columns, rows, gray, cmyk, false, false, 0);
    let length = columns * (format & 0x0f) as usize * (channels & 0x0f) as usize;
    for y in 0..rows {
        let row = blob.read(length);
        if row.len() != length {
            bail!("UnexpectedEndOfFile");
        }
        quantum::import(&mut picture, &info, kind, y * columns, columns, row)?;
    }
    Ok(picture.into_cache(gray, colorspace, gamma, crate::pixel_cache::ImageType::Undefined))
}
