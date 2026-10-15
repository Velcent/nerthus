//! ZSoft PCX (and DCX, a page table of PCX images) as ImageMagick reads it
//! (`coders/pcx.c`): 1, 2, 4 and 8 bits a pixel in one plane or bit planes
//! (PseudoClass), or 8-bit planes (DirectClass, with alpha past three).
//! The colormap is the header's 16 entries, a version 5 file's 256 at the
//! end, or black and white for one bit. Only the first image.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::colormap;
use crate::pixel_cache::{char_quantum, CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

const DCX_MAGIC: u32 = 987654321;

pub fn read(path: &Path, dcx: bool) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes, dcx)
}

pub fn decode(bytes: &[u8], dcx: bool) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    if dcx {
        if blob.lsb_u32() != DCX_MAGIC {
            bail!("ImproperImageHeader");
        }
        // The page table runs to a zero entry (1024 at most); the first
        // image is at the first entry.
        let first_page = blob.lsb_u32();
        let mut entry = first_page;
        let mut entries = 1;
        while entry != 0 && entries < 1024 {
            entry = blob.lsb_u32();
            entries += 1;
        }
        blob.seek(first_page as i64);
        blob.eof = false;
    }
    let identifier = blob.read(1).first().copied();
    let version = blob.byte().unwrap_or(0xff);
    if identifier != Some(0x0a) {
        bail!("ImproperImageHeader");
    }
    let encoding = blob.byte().unwrap_or(0xff);
    let bits_per_pixel = blob.byte().context("ImproperImageHeader")? as usize;
    let left = blob.lsb_u16();
    let top = blob.lsb_u16();
    let right = blob.lsb_u16();
    let bottom = blob.lsb_u16();
    let _horizontal_resolution = blob.lsb_u16();
    let _vertical_resolution = blob.lsb_u16();
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    if right < left || bottom < top || !matches!(bits_per_pixel, 1 | 2 | 4 | 8) {
        bail!("ImproperImageHeader");
    }
    let width = (right - left) as usize + 1;
    let height = (bottom - top) as usize + 1;
    if (width * height / 255) as u64 > blob.size() {
        bail!("InsufficientImageDataInFile");
    }
    let mut header_colormap = [0u8; 768];
    let header_entries = blob.read(48);
    if header_entries.len() != 48 {
        bail!("ImproperImageHeader");
    }
    header_colormap[..48].copy_from_slice(header_entries);
    let _reserved = blob.byte();
    let planes = blob.byte().unwrap_or(0xff) as usize;
    if planes == 0 || planes > 6 {
        bail!("ImproperImageHeader");
    }
    let mut colors = 16;
    if (bits_per_pixel != 8 || planes == 1) && (version == 3 || version == 5 || bits_per_pixel * planes == 1) {
        colors = (1usize << (bits_per_pixel * planes)).min(256);
    }
    let mut colormap = colormap::acquire(colors).context("MemoryAllocationFailed")?;
    let pseudo = !(bits_per_pixel >= 8 && planes != 1);
    for (entry, rgb) in colormap.iter_mut().zip(header_colormap.as_chunks::<3>().0) {
        entry[0] = char_quantum(rgb[0]) as f64;
        entry[1] = char_quantum(rgb[1]) as f64;
        entry[2] = char_quantum(rgb[2]) as f64;
    }
    let bytes_per_line = blob.lsb_u16() as usize;
    let _palette_info = blob.lsb_u16();
    let _horizontal_screen_size = blob.lsb_u16();
    let _vertical_screen_size = blob.lsb_u16();
    for _ in 0..54 {
        blob.byte();
    }

    let packet_count = height * bytes_per_line * planes;
    if bits_per_pixel * planes * width > packet_count * 8 {
        bail!("ImproperImageHeader");
    }
    if (packet_count / 32 + 128) as u64 > blob.size() {
        bail!("ImproperImageHeader");
    }
    let least_bytes_per_line = (width * bits_per_pixel).div_ceil(8);
    if least_bytes_per_line == 0 || bytes_per_line < least_bytes_per_line {
        bail!("ImproperImageHeader");
    }

    // The packets, raw or run-length encoded (`0xc0 | count`, then the byte).
    let mut packets = vec![0u8; packet_count * 2];
    let mut written = 0;
    let mut remaining = packet_count;
    if encoding == 0 {
        while remaining != 0 {
            let packet = blob.byte().context("UnexpectedEndOfFile")?;
            packets[written] = packet;
            written += 1;
            remaining -= 1;
        }
    } else {
        while remaining != 0 {
            let packet = blob.byte().context("UnexpectedEndOfFile")?;
            if packet & 0xc0 != 0xc0 {
                packets[written] = packet;
                written += 1;
                remaining -= 1;
                continue;
            }
            let count = packet & 0x3f;
            let value = blob.byte().context("UnexpectedEndOfFile")?;
            for _ in 0..count {
                packets[written] = value;
                written += 1;
                remaining -= 1;
                if remaining == 0 {
                    break;
                }
            }
        }
    }
    let alpha = !pseudo && planes > 3;

    // Each row through a scanline that keeps what earlier rows left in it.
    let mut scanline = vec![0u8; width.max(bytes_per_line) * planes.max(8)];
    let mut channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    if alpha {
        channels.push(PixelChannel::Alpha);
    }
    if pseudo {
        channels.push(PixelChannel::Index);
    }
    let stride = channels.len();
    let mut data = Vec::with_capacity(width * height * stride);
    for y in 0..height {
        let mut p = y * bytes_per_line * planes;
        if !pseudo {
            for plane in 0..planes {
                let mut r = plane;
                for _ in 0..bytes_per_line {
                    scanline[r] = packets[p];
                    p += 1;
                    r += planes;
                }
            }
        } else if planes > 1 {
            scanline[..width].fill(0);
            for plane in 0..planes {
                let mut r = 0;
                for _ in 0..bytes_per_line {
                    let bits = packets[p];
                    p += 1;
                    let mut mask = 0x80u8;
                    while mask != 0 {
                        if bits & mask != 0 {
                            scanline[r] |= 1 << plane;
                        }
                        r += 1;
                        mask >>= 1;
                    }
                }
            }
        } else {
            let mut r = 0;
            match bits_per_pixel {
                1 => {
                    let mut x = 0;
                    while x + 7 < width {
                        for bit in (0..8).rev() {
                            scanline[r] = u8::from(packets[p] & (1 << bit) == 0);
                            r += 1;
                        }
                        p += 1;
                        x += 8;
                    }
                    if !width.is_multiple_of(8) {
                        for bit in ((8 - width % 8)..8).rev() {
                            scanline[r] = u8::from(packets[p] & (1 << bit) == 0);
                            r += 1;
                        }
                    }
                }
                2 => {
                    let mut x = 0;
                    while x + 3 < width {
                        for shift in [6, 4, 2, 0] {
                            scanline[r] = (packets[p] >> shift) & 0x3;
                            r += 1;
                        }
                        p += 1;
                        x += 4;
                    }
                    if !width.is_multiple_of(4) {
                        for index in ((4 - width % 4)..4).rev() {
                            scanline[r] = (packets[p] >> (index * 2)) & 0x3;
                            r += 1;
                        }
                    }
                }
                4 => {
                    let mut x = 0;
                    while x + 1 < width {
                        scanline[r] = (packets[p] >> 4) & 0xf;
                        scanline[r + 1] = packets[p] & 0xf;
                        r += 2;
                        p += 1;
                        x += 2;
                    }
                    if !width.is_multiple_of(2) {
                        scanline[r] = (packets[p] >> 4) & 0xf;
                    }
                }
                _ => scanline[..width].copy_from_slice(&packets[p..p + width]),
            }
        }
        let mut r = 0;
        for _ in 0..width {
            if pseudo {
                data.extend([0.0, 0.0, 0.0, scanline[r] as f32]);
                r += 1;
            } else {
                data.extend([char_quantum(scanline[r]), char_quantum(scanline[r + 1]), char_quantum(scanline[r + 2])]);
                r += 3;
                if alpha {
                    data.push(char_quantum(scanline[r]));
                    r += 1;
                }
            }
        }
    }

    if pseudo {
        if version == 5 || bits_per_pixel * planes == 1 {
            if bits_per_pixel * planes == 1 {
                colormap[0][..3].fill(0.0);
                colormap[1][..3].fill(65535.0);
            } else if colors > 16 {
                let _signature = blob.byte();
                let entries = blob.read(3 * colors);
                header_colormap[..entries.len()].copy_from_slice(entries);
                for (entry, rgb) in colormap.iter_mut().zip(header_colormap.as_chunks::<3>().0) {
                    entry[0] = char_quantum(rgb[0]) as f64;
                    entry[1] = char_quantum(rgb[1]) as f64;
                    entry[2] = char_quantum(rgb[2]) as f64;
                }
            }
        }
        colormap::sync(&mut data, stride, stride - 1, None, &colormap);
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    let mut cache = PixelCache::new(width, height, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA);
    if pseudo {
        cache.colormap = colormap;
    }
    Ok(cache)
}
