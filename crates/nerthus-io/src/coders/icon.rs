//! Windows icons and cursors as ImageMagick reads them (`coders/icon.c`):
//! the first entry of the directory, a PNG or a bottom-up bitmap — 1, 4, 8
//! or 16-bit colormapped or 24-bit, then its AND mask as alpha, or 32-bit
//! BGRA. Windows 1.x icons keep their AND and XOR masks as a three-entry
//! colormap.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::colormap;
use crate::pixel_cache::{char_quantum, read_png_bytes, CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

const MAX_ICONS: u16 = 1024;

struct IconEntry {
    width: u8,
    height: u8,
    size: u32,
    offset: u32,
}

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

/// `GetICONSize`: the bitmap's size, else the directory's, else 256.
fn icon_size(directory_size: u8, image_size: usize) -> usize {
    if image_size != 0 {
        return image_size;
    }
    if directory_size != 0 {
        return directory_size as usize;
    }
    256
}

/// `ReadBlobByte` as these loops use it: all bits set at the end.
fn next_byte(blob: &mut Blob) -> usize {
    blob.byte().map_or(usize::MAX, |byte| byte as usize)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let reserved = blob.lsb_u16();
    if matches!(reserved, 0x0001 | 0x0101 | 0x0201) {
        return decode_1x(&mut blob);
    }
    let resource_type = blob.lsb_u16();
    let icon_count = blob.lsb_u16();
    if reserved != 0 || (resource_type != 1 && resource_type != 2) || icon_count > MAX_ICONS {
        bail!("ImproperImageHeader");
    }
    let mut entries = Vec::with_capacity(icon_count as usize);
    let mut extent = 0u64;
    for _ in 0..icon_count {
        let width = blob.byte().unwrap_or(0xff);
        let height = blob.byte().unwrap_or(0xff);
        let _colors = blob.byte();
        let _reserved = blob.byte();
        let _planes = blob.lsb_u16();
        let _bits_per_pixel = blob.lsb_u16();
        let size = blob.lsb_u32();
        let offset = blob.lsb_u32();
        if blob.eof {
            break;
        }
        extent = extent.max(size as u64);
        entries.push(IconEntry { width, height, size, offset });
    }
    if blob.eof || extent > blob.size() {
        bail!("UnexpectedEndOfFile");
    }
    // Magick.NET reads one frame: the first entry.
    let Some(entry) = entries.first() else {
        bail!("NoImagesDefined");
    };
    blob.seek(entry.offset as i64);
    blob.eof = false;
    let size = blob.lsb_u32();
    let width = blob.lsb_u32() as i32 as i64;
    let height = (blob.lsb_u32() as i32 / 2) as i64;
    let planes = blob.lsb_u16();
    let bits_per_pixel = blob.lsb_u16();
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    if (planes == 18505 && bits_per_pixel == 21060) || size == 0x474e_5089 {
        let length = entry.size as usize;
        if length < 16 {
            bail!("MemoryAllocationFailed");
        }
        let mut png = Vec::with_capacity(length);
        png.extend_from_slice(b"\x89PNG\r\n\x1a\n\0\0\0\x0d");
        png.extend(planes.to_le_bytes());
        png.extend(bits_per_pixel.to_le_bytes());
        let rest = blob.read(length - 16);
        if rest.len() != length - 16 {
            bail!("InsufficientImageDataInFile");
        }
        png.extend_from_slice(rest);
        return read_png_bytes(&png);
    }

    if bits_per_pixel > 32 {
        bail!("ImproperImageHeader");
    }
    let _compression = blob.lsb_u32();
    let _image_size = blob.lsb_u32();
    let _x_pixels = blob.lsb_u32();
    let _y_pixels = blob.lsb_u32();
    let number_colors = blob.lsb_u32() as u64;
    if number_colors > blob.size() {
        bail!("InsufficientImageDataInFile");
    }
    let _colors_important = blob.lsb_u32();
    // `(size_t)` of a negative size is too big for `SetImageExtent`.
    if width < 0 || height < 0 {
        bail!("WidthOrHeightExceedsLimit");
    }
    let columns = icon_size(entry.width, width as usize);
    let rows = icon_size(entry.height, height as usize);
    let mut pseudo = false;
    let mut colors = 0u64;
    if number_colors != 0 || bits_per_pixel <= 16 {
        pseudo = true;
        colors = number_colors;
        if colors == 0 || colors > 256 {
            colors = 1u64 << bits_per_pixel;
        }
    }
    let mut colormap = Vec::new();
    if pseudo {
        if colors > blob.size() {
            bail!("InsufficientImageDataInFile");
        }
        colormap = colormap::acquire(colors as usize).context("MemoryAllocationFailed")?;
        let table = blob.read(4 * colors as usize);
        if table.len() != 4 * colors as usize {
            bail!("InsufficientImageDataInFile");
        }
        for (entry, bgrx) in colormap.iter_mut().zip(table.as_chunks::<4>().0) {
            entry[0] = char_quantum(bgrx[2]) as f64;
            entry[1] = char_quantum(bgrx[1]) as f64;
            entry[2] = char_quantum(bgrx[0]) as f64;
        }
    }
    if columns == 0 || rows == 0 {
        bail!("NegativeOrZeroImageSize");
    }

    // Rows of fewer bytes than that run into the end of the file, which
    // fails the read.
    let bits = columns * bits_per_pixel as usize;
    let row_bytes = ((bits + 31) & !31) >> 3;
    let remaining = blob.size().saturating_sub(blob.tell());
    if (rows as u64).saturating_mul(row_bytes as u64) > remaining {
        bail!("UnexpectedEndOfFile");
    }
    let scanline_pad = (((bits + 31) & !31) - bits) >> 3;

    // Red, green, blue, alpha and (while PseudoClass) the index.
    const STRIDE: usize = 5;
    let mut pixels = vec![0.0f32; columns * rows * STRIDE];
    let skip = |blob: &mut Blob, count: usize| {
        for _ in 0..count {
            blob.byte();
        }
    };
    for y in (0..rows).rev() {
        let row = &mut pixels[y * columns * STRIDE..(y + 1) * columns * STRIDE];
        match bits_per_pixel {
            1 => {
                let mut x = 0;
                while x + 7 < columns {
                    let byte = next_byte(&mut blob);
                    for bit in 0..8 {
                        row[(x + bit) * STRIDE + 4] = u8::from(byte & (0x80 >> bit) != 0) as f32;
                    }
                    x += 8;
                }
                if !columns.is_multiple_of(8) {
                    let byte = next_byte(&mut blob);
                    for bit in 0..columns % 8 {
                        row[(x + bit) * STRIDE + 4] = u8::from(byte & (0x80 >> bit) != 0) as f32;
                    }
                }
                skip(&mut blob, scanline_pad);
            }
            4 => {
                let mut x = 0;
                while x + 1 < columns {
                    let byte = next_byte(&mut blob);
                    row[x * STRIDE + 4] = ((byte >> 4) & 0xf) as f32;
                    row[(x + 1) * STRIDE + 4] = (byte & 0xf) as f32;
                    x += 2;
                }
                if !columns.is_multiple_of(2) {
                    let byte = next_byte(&mut blob);
                    row[x * STRIDE + 4] = ((byte >> 4) & 0xf) as f32;
                }
                skip(&mut blob, scanline_pad);
            }
            8 => {
                for x in 0..columns {
                    row[x * STRIDE + 4] = next_byte(&mut blob) as f32;
                }
                skip(&mut blob, scanline_pad);
            }
            16 => {
                for x in 0..columns {
                    let low = next_byte(&mut blob);
                    let high = next_byte(&mut blob);
                    row[x * STRIDE + 4] = (low | (high << 8)) as f32;
                }
                skip(&mut blob, scanline_pad);
            }
            24 | 32 => {
                for x in 0..columns {
                    let pixel = &mut row[x * STRIDE..(x + 1) * STRIDE];
                    pixel[2] = char_quantum(next_byte(&mut blob) as u8);
                    pixel[1] = char_quantum(next_byte(&mut blob) as u8);
                    pixel[0] = char_quantum(next_byte(&mut blob) as u8);
                    if bits_per_pixel == 32 {
                        pixel[3] = char_quantum(next_byte(&mut blob) as u8);
                    }
                }
                if bits_per_pixel == 24 {
                    skip(&mut blob, scanline_pad);
                }
            }
            _ => bail!("ImproperImageHeader"),
        }
    }
    if bits_per_pixel <= 16 {
        colormap::sync(&mut pixels, STRIDE, 4, Some(3), &colormap);
    }
    if bits_per_pixel != 32 {
        // The AND mask: set bits are transparent. The image is DirectClass
        // from here.
        pseudo = false;
        for y in (0..rows).rev() {
            let row = &mut pixels[y * columns * STRIDE..(y + 1) * columns * STRIDE];
            let mut x = 0;
            while x + 7 < columns {
                let byte = next_byte(&mut blob);
                for bit in 0..8 {
                    row[(x + bit) * STRIDE + 3] = if byte & (0x80 >> bit) != 0 { 0.0 } else { 65535.0 };
                }
                x += 8;
            }
            if !columns.is_multiple_of(8) {
                let byte = next_byte(&mut blob);
                for bit in 0..columns % 8 {
                    row[(x + bit) * STRIDE + 3] = if byte & (0x80 >> bit) != 0 { 0.0 } else { 65535.0 };
                }
            }
            if !columns.is_multiple_of(32) {
                skip(&mut blob, (32 - columns % 32) / 8);
            }
        }
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }

    let mut channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Alpha];
    let data = if pseudo {
        channels.push(PixelChannel::Index);
        pixels
    } else {
        let mut direct = Vec::with_capacity(columns * rows * 4);
        for pixel in pixels.as_chunks::<STRIDE>().0 {
            direct.extend_from_slice(&pixel[..4]);
        }
        direct
    };
    let mut cache = PixelCache::new(columns, rows, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA);
    if pseudo {
        cache.colormap = colormap;
    }
    Ok(cache)
}

/// `Read1XImage`: a Windows 1.x icon, 32 or 64 pixels square, its AND
/// mask (entry 1, transparent) then XOR mask (entry 2) over a three-level
/// grey colormap. The end of the file reads as set bits.
fn decode_1x(blob: &mut Blob) -> Result<PixelCache> {
    let _hot_spot = blob.lsb_u32();
    let columns = blob.lsb_u16() as usize;
    let rows = blob.lsb_u16() as usize;
    let _width_bytes = blob.lsb_u16();
    let _cursor_colour = blob.lsb_u16();
    if (rows != 32 && rows != 64) || (columns != 32 && columns != 64) {
        bail!("ImproperImageHeader");
    }
    let mut colormap = colormap::acquire(3).context("MemoryAllocationFailed")?;
    colormap[1][3] = 0.0;
    const STRIDE: usize = 5;
    let mut pixels = vec![0.0f32; columns * rows * STRIDE];
    // Rows run to the width: past the last row, queueing pixels fails.
    if columns > rows {
        bail!("PixelsAreNotAuthentic");
    }
    for pass in 0..2 {
        for y in 0..columns {
            let row = &mut pixels[y * columns * STRIDE..(y + 1) * columns * STRIDE];
            for x in (0..columns).step_by(8) {
                let byte = next_byte(blob);
                for bit in 0..8 {
                    let index = if byte & (0x80 >> bit) != 0 { if pass == 0 { 1.0 } else { 2.0 } } else { 0.0 };
                    let slot = &mut row[(x + bit) * STRIDE + 4];
                    if pass == 0 || *slot != 1.0 {
                        *slot = index;
                    }
                }
            }
        }
    }
    colormap::sync(&mut pixels, STRIDE, 4, Some(3), &colormap);
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Alpha, PixelChannel::Index];
    let mut cache = PixelCache::new(columns, rows, channels, pixels, CacheColorspace::SRgb, DEFAULT_GAMMA);
    cache.colormap = colormap;
    Ok(cache)
}
