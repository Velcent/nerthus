//! Kodak Photo CD images as ImageMagick reads them (`coders/pcd.c`) when
//! asked for one frame: the scene number (zero) picks the 192x128 Base/16
//! image, luma rows and half-size chroma rows interleaved from sector 4,
//! chroma doubled in place by IM's `Upsample`, left in Photo YCC (PCDS
//! labels the same samples sRGB), and turned when the header says so.
//! Overview packs (`PCD_OPA`) are a montage of labelled thumbnails, which
//! needs IM's font rendering, so they are not read.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use crate::pixel_cache::{char_quantum, CacheColorspace, PixelCache, PixelChannel};

const SECTOR: usize = 0x800;

pub fn read(path: &Path, name: &str) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes, name == "PCDS")
}

/// `Upsample`: a `width` x `height` plane at the start of each
/// `scaled_width` row doubled in both directions, in place, averaging
/// neighbours (rounding up) for the new samples.
fn upsample(width: usize, height: usize, scaled_width: usize, pixels: &mut [u8]) {
    for y in 0..height {
        let mut p = (height - 1 - y) * scaled_width + (width - 1);
        let mut q = ((height - 1 - y) << 1) * scaled_width + ((width - 1) << 1);
        pixels[q] = pixels[p];
        pixels[q + 1] = pixels[p];
        for _ in 1..width {
            p -= 1;
            q -= 2;
            pixels[q] = pixels[p];
            pixels[q + 1] = ((pixels[p] as usize + pixels[p + 1] as usize + 1) >> 1) as u8;
        }
    }
    for y in 0..height - 1 {
        let mut p = (y << 1) * scaled_width;
        let mut q = p + scaled_width;
        let mut r = q + scaled_width;
        for _ in 0..width - 1 {
            pixels[q] = ((pixels[p] as usize + pixels[r] as usize + 1) >> 1) as u8;
            let corners = pixels[p] as usize + pixels[p + 2] as usize + pixels[r] as usize + pixels[r + 2] as usize;
            pixels[q + 1] = ((corners + 2) >> 2) as u8;
            q += 2;
            p += 2;
            r += 2;
        }
        pixels[q] = ((pixels[p] as usize + pixels[r] as usize + 1) >> 1) as u8;
        pixels[q + 1] = ((pixels[p + 1] as usize + pixels[r + 1] as usize + 1) >> 1) as u8;
    }
    let p = (2 * height - 2) * scaled_width;
    let q = (2 * height - 1) * scaled_width;
    pixels.copy_within(p..p + 2 * width, q);
}

pub fn decode(bytes: &[u8], srgb: bool) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let header = blob.read(3 * SECTOR);
    if header.len() != 3 * SECTOR {
        bail!("ImproperImageHeader");
    }
    let overview = &header[..7] == b"PCD_OPA";
    if &header[SECTOR..SECTOR + 3] != b"PCD" && !overview {
        bail!("ImproperImageHeader");
    }
    if overview {
        bail!("PCD overview packs are not supported");
    }
    let rotate = header[0x0e02] & 0x03;

    // Base/16, one sector on from the header.
    let columns = 192;
    let rows = 128;
    for _ in 0..SECTOR {
        if blob.byte().is_none() {
            bail!("UnexpectedEndOfFile");
        }
    }
    let plane_size = columns * rows;
    let mut luma = vec![0u8; plane_size];
    let mut chroma1 = vec![0u8; plane_size];
    let mut chroma2 = vec![0u8; plane_size];
    for y in (0..rows).step_by(2) {
        for row in [y, y + 1] {
            let read = blob.read(columns);
            luma[row * columns..row * columns + read.len()].copy_from_slice(read);
        }
        let chroma_row = (y / 2) * columns;
        for plane in [&mut chroma1, &mut chroma2] {
            let read = blob.read(columns >> 1);
            plane[chroma_row..chroma_row + read.len()].copy_from_slice(read);
        }
        if blob.eof {
            bail!("UnexpectedEndOfFile");
        }
    }
    upsample(columns >> 1, rows >> 1, columns, &mut chroma1);
    upsample(columns >> 1, rows >> 1, columns, &mut chroma2);
    let mut data = Vec::with_capacity(plane_size * 3);
    for i in 0..plane_size {
        data.push(char_quantum(luma[i]));
        data.push(char_quantum(chroma1[i]));
        data.push(char_quantum(chroma2[i]));
    }

    // `RotateImage` by -90 (1) or 90 (3) degrees: an exact quarter turn.
    let (width, height, data) = match rotate {
        1 | 3 => {
            let mut turned = vec![0.0f32; data.len()];
            for y in 0..columns {
                for x in 0..rows {
                    let (source_x, source_y) = if rotate == 3 { (y, rows - 1 - x) } else { (columns - 1 - y, x) };
                    let from = (source_y * columns + source_x) * 3;
                    let to = (y * rows + x) * 3;
                    turned[to..to + 3].copy_from_slice(&data[from..from + 3]);
                }
            }
            (rows, columns, turned)
        }
        _ => (columns, rows, data),
    };
    let colorspace = if srgb { CacheColorspace::SRgb } else { CacheColorspace::Ycc };
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    // `image->gamma=1.000f/2.200f`, a float quotient.
    let gamma = (1.0f32 / 2.2f32) as f64;
    Ok(PixelCache::new(width, height, channels, data, colorspace, gamma))
}
