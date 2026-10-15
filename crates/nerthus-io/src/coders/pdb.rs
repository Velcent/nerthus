//! Palm ImageViewer databases as ImageMagick reads them (`coders/pdb.c`):
//! a "vIMG"/"View" database whose first record is a 1-, 2- or 4-bit grey
//! image (darkest index highest), raw or run-length encoded. Rows aren't
//! byte aligned: each starts at the first bit of the byte the last one
//! ended in, as IM reads them.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::colormap;
use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

/// `DecodeImage`: runs (`n > 128`: `n - 127` copies of the next byte) and
/// copies (`n + 1` bytes).
fn decode_rle(blob: &mut Blob, length: usize) -> Option<Vec<u8>> {
    let mut pixels = Vec::with_capacity(length);
    let mut count = 0;
    let mut run = false;
    let mut data = 0u8;
    while pixels.len() < length {
        if count == 0 {
            let first = blob.byte()?;
            if first > 128 {
                run = true;
                count = first as usize - 128 + 1;
                data = blob.byte()?;
            } else {
                run = false;
                count = first as usize + 1;
            }
        }
        if !run {
            data = blob.byte()?;
        }
        pixels.push(data);
        count -= 1;
    }
    Some(pixels)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    if blob.read(32).len() != 32 {
        bail!("ImproperImageHeader");
    }
    let _attributes = blob.msb_u16();
    let _version = blob.msb_u16();
    for _ in 0..6 {
        blob.msb_u32();
    }
    let kind = blob.read(4).to_vec();
    let id = blob.read(4).to_vec();
    let _seed = blob.msb_u32();
    let next_record = blob.msb_u32();
    let number_records = blob.msb_u16() as i16;
    if kind != b"vIMG" || id != b"View" {
        bail!("ImproperImageHeader");
    }
    if next_record != 0 {
        bail!("MultipleRecordListNotSupported");
    }
    let image_offset = blob.msb_u32() as i32 as i64;
    let _record_attributes = blob.byte();
    if blob.read(3) != b"\x6f\x80\x00" {
        bail!("CorruptImage");
    }
    if number_records > 1 {
        let _comment_offset = blob.msb_u32();
        let _comment_attributes = blob.byte();
        if blob.read(3) != b"\x6f\x80\x01" {
            bail!("CorruptImage");
        }
    }
    let padding = (image_offset - blob.tell() as i64) as u64;
    let mut skipped = 0u64;
    while skipped != padding {
        if blob.byte().is_none() {
            break;
        }
        skipped += 1;
    }
    if blob.read(32).len() != 32 {
        bail!("ImproperImageHeader");
    }
    let version = blob.byte().unwrap_or(0xff);
    let image_type = blob.byte().unwrap_or(0xff);
    let _reserved_1 = blob.msb_u32();
    let _note = blob.msb_u32();
    let _x_last = blob.msb_u16();
    let _y_last = blob.msb_u16();
    let _reserved_2 = blob.msb_u32();
    let _x_anchor = blob.msb_u16();
    let _y_anchor = blob.msb_u16();
    let width = blob.msb_u16() as i16;
    let height = blob.msb_u16() as i16;
    let columns = width.unsigned_abs() as usize;
    let rows = height.unsigned_abs() as usize;
    let bits_per_pixel = match image_type {
        0 => 2,
        2 => 4,
        _ => 1,
    };
    let entries = colormap::acquire(1 << bits_per_pixel).context("MemoryAllocationFailed")?;
    if columns == 0 || rows == 0 {
        bail!("NegativeOrZeroImageSize");
    }
    let packets = (bits_per_pixel * columns).div_ceil(8);
    let length = packets * rows;
    let mut pixels = match version & 0x07 {
        0 => {
            let data = blob.read(length);
            if data.len() != length {
                bail!("RLEDecoderError");
            }
            data.to_vec()
        }
        1 => decode_rle(&mut blob, length).context("RLEDecoderError")?,
        _ => bail!("UnrecognizedImageCompressionType"),
    };
    pixels.resize((packets + 257) * rows, 0);
    let mut data = Vec::with_capacity(columns * rows * 4);
    let mut p = 0usize;
    for _ in 0..rows {
        match bits_per_pixel {
            1 => {
                let mut bit = 0;
                for _ in 0..columns {
                    let index = if pixels[p] & (0x80 >> bit) != 0 { 0 } else { 1 };
                    data.extend([0.0, 0.0, 0.0, index as f32]);
                    bit += 1;
                    if bit == 8 {
                        p += 1;
                        bit = 0;
                    }
                }
            }
            depth => {
                let mut shift = 8;
                let top = (1usize << depth) - 1;
                for _ in 0..columns {
                    shift -= depth;
                    let index = top - ((pixels[p] as usize >> shift) & top);
                    data.extend([0.0, 0.0, 0.0, index as f32]);
                    if shift == 0 {
                        shift = 8;
                        p += 1;
                    }
                }
            }
        }
    }
    colormap::sync(&mut data, 4, 3, None, &entries);
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Index];
    let mut cache = PixelCache::new(columns, rows, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA);
    cache.colormap = entries;
    Ok(cache)
}
