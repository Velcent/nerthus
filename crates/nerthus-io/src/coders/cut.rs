//! Dr. Halo CUT images as ImageMagick reads them (`coders/cut.c`): RLE
//! rows whose first line gives the depth (8, 4 or 1 bit), coloured by a
//! Dr. Halo palette next to the file (`.PAL`, `.pal` or no extension) or
//! else a grey ramp; a two-level image without a palette is made black and
//! white.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::colormap;
use crate::pixel_cache::{char_quantum, CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

const QUANTUM_RANGE: f64 = 65535.0;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    let palette = find_palette(path).and_then(|palette| std::fs::read(palette).ok());
    decode(&bytes, palette.as_deref())
}

/// The palette file IM looks for: the name with its extension replaced by
/// ".PAL", then ".pal", then with none.
fn find_palette(path: &Path) -> Option<PathBuf> {
    let name = path.to_string_lossy().to_string();
    let bytes = name.as_bytes();
    let end = bytes.len();
    let mut i = end;
    loop {
        if i <= 1 {
            i = 0;
            break;
        }
        i -= 1;
        if bytes[i] == b'.' {
            break;
        }
        if matches!(bytes[i], b'/' | b'\\' | b':') {
            i = end;
            break;
        }
    }
    let stem = &name[..i];
    for candidate in [format!("{stem}.PAL"), format!("{stem}.pal"), stem.to_string()] {
        let candidate = PathBuf::from(candidate);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// The palette's colormap, or none when it isn't a usable "AH" palette.
fn read_palette(bytes: &[u8]) -> Result<Option<Vec<[f64; 4]>>> {
    let mut blob = Blob::new(bytes);
    if blob.read(2) != b"AH" {
        return Ok(None);
    }
    blob.lsb_u16();
    blob.lsb_u16();
    blob.byte();
    blob.byte();
    blob.lsb_u16();
    blob.lsb_u16();
    let max_index = blob.lsb_u16() as usize;
    let mut maxima = [blob.lsb_u16() as u32, blob.lsb_u16() as u32, blob.lsb_u16() as u32];
    blob.read(20);
    if max_index < 1 {
        return Ok(None);
    }
    let mut entries = colormap::acquire(max_index + 1).context("MemoryAllocationFailed")?;
    for maximum in maxima.iter_mut() {
        if *maximum == 0 {
            *maximum = 65535;
        }
    }
    for entry in entries.iter_mut().take(max_index + 1) {
        let position = blob.tell();
        if position % 512 > 512 - 6 {
            blob.seek(((position / 512 + 1) * 512) as i64);
        }
        for (channel, &maximum) in maxima.iter().enumerate() {
            let mut value = blob.lsb_u16() as f32 as f64;
            if maximum != 65535 {
                value = ((value * QUANTUM_RANGE + (maximum >> 1) as f64) / maximum as f64) as f32 as f64;
            }
            entry[channel] = value;
        }
    }
    Ok(Some(entries))
}

pub fn decode(bytes: &[u8], palette: Option<&[u8]>) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let width = blob.lsb_u16() as usize;
    let height = blob.lsb_u16() as usize;
    let reserved = blob.lsb_u16();
    if width == 0 || height == 0 || reserved != 0 {
        bail!("ImproperImageHeader");
    }
    // The first line sets the bytes a row and so the depth.
    let mut encoded = blob.lsb_u16() as u64;
    let mut run_count = blob.byte().unwrap_or(0xff);
    let mut masked = run_count & 0x7f;
    let mut row_length = 0usize;
    while masked != 0 {
        let skip = if run_count < 0x80 { masked as u64 } else { 1 };
        let target = blob.tell() + skip;
        blob.seek(target as i64);
        blob.eof = false;
        if blob.eof {
            bail!("ImproperImageHeader");
        }
        encoded = encoded.wrapping_sub(skip + 1);
        row_length += masked as usize;
        run_count = blob.byte().unwrap_or(0xff);
        if blob.eof {
            bail!("ImproperImageHeader");
        }
        masked = run_count & 0x7f;
    }
    if encoded != 1 {
        bail!("ImproperImageHeader");
    }
    let mut depth = 0;
    if row_length == width {
        depth = 8;
    }
    if 2 * row_length == width {
        depth = 4;
    }
    if 8 * row_length == width {
        depth = 1;
    }
    if depth == 0 {
        bail!("ImproperImageHeader");
    }
    let palette_entries = match palette {
        Some(bytes) => read_palette(bytes)?,
        None => None,
    };
    let has_palette = palette_entries.is_some();
    let mut entries = match palette_entries {
        Some(entries) => entries,
        None => {
            let mut entries = colormap::acquire(256).context("MemoryAllocationFailed")?;
            for (i, entry) in entries.iter_mut().enumerate() {
                let level = char_quantum(i as u8) as f64;
                entry[..3].fill(level);
            }
            entries
        }
    };
    let colors = entries.len();

    let mut row = vec![0u8; row_length];
    let mut data = vec![0.0f32; width * height * 4];
    blob.seek(6);
    let mut complete = true;
    'rows: for y in 0..height {
        blob.lsb_u16();
        let mut p = 0usize;
        let mut left = row_length;
        let mut run_count = blob.byte().unwrap_or(0xff);
        let mut masked = (run_count & 0x7f) as usize;
        while masked != 0 {
            if masked > left {
                masked = left;
                if left == 0 {
                    break;
                }
            }
            if run_count > 0x80 {
                let value = blob.byte().unwrap_or(0xff);
                row[p..p + masked].fill(value);
            } else {
                let read = blob.read(masked);
                row[p..p + read.len()].copy_from_slice(read);
            }
            p += masked;
            left -= masked;
            if blob.eof {
                complete = false;
                break 'rows;
            }
            run_count = blob.byte().unwrap_or(0xff);
            masked = (run_count & 0x7f) as usize;
        }
        // `InsertRow`.
        let target = &mut data[y * width * 4..(y + 1) * width * 4];
        for x in 0..width {
            let index = match depth {
                1 => usize::from(row[x / 8] & (0x80 >> (x % 8)) != 0),
                4 => {
                    let byte = row[x / 2];
                    let nibble = if x % 2 == 0 { byte >> 4 } else { byte & 0x0f };
                    nibble as usize
                }
                _ => row[x] as usize,
            };
            if depth != 1 && index >= colors {
                bail!("InvalidColormapIndex");
            }
            target[x * 4 + 3] = index as f32;
        }
    }
    if complete {
        colormap::sync(&mut data, 4, 3, None, &entries);
        if !has_palette {
            // Black and white: indices 0 and 1 only, 1 made white.
            let mut maximum = 0.0f32;
            let mut many = false;
            for pixel in data.as_chunks::<4>().0 {
                if maximum < pixel[0] {
                    maximum = pixel[0];
                }
                if maximum >= char_quantum(16) {
                    many = true;
                    break;
                }
            }
            if !many && maximum < char_quantum(2) {
                entries[1][..3].fill(QUANTUM_RANGE);
                for pixel in data.as_chunks_mut::<4>().0 {
                    if pixel[0] == char_quantum(1) {
                        pixel[..3].fill(QUANTUM_RANGE as f32);
                    }
                }
            }
        }
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Index];
    let mut cache = PixelCache::new(width, height, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA);
    cache.colormap = entries;
    Ok(cache)
}
