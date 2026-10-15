//! JPEG Network Graphics as ImageMagick reads them (`ReadJNGImage` in
//! `coders/png.c`): the JDAT chunks make a JPEG for the colour; a colour
//! type with alpha (12 or 14) adds IDAT chunks (a grey PNG IM wraps in its
//! own header) or JDAA chunks (a JPEG) whose grey becomes the alpha. The
//! image takes the colour JPEG's first three channels and the header's
//! size, which the JPEG must match.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

const SIGNATURE: &[u8; 8] = b"\x8bJNG\r\n\x1a\n";

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

fn be32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// A PNG chunk with its CRC.
fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::with_capacity(12 + data.len());
    chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
    chunk.extend_from_slice(kind);
    chunk.extend_from_slice(data);
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(kind);
    hasher.update(data);
    chunk.extend_from_slice(&hasher.finalize().to_be_bytes());
    chunk
}

/// The first channel of a decoded image, as `GetPixelRed` reads it (a grey
/// image's grey).
fn first_channel(image: &PixelCache, pixel: usize) -> f32 {
    image.data[pixel * image.channels.len()]
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let magic = blob.read(8);
    if magic != SIGNATURE {
        bail!("ImproperImageHeader");
    }
    if blob.size() < 147 {
        bail!("InsufficientImageDataInFile");
    }
    let mut width = 0u32;
    let mut height = 0u32;
    let mut color_type = 0u8;
    let mut alpha_depth = 8u8;
    let mut alpha_compression = 0u8;
    let mut gamma = DEFAULT_GAMMA;
    let mut color: Option<Vec<u8>> = None;
    let mut alpha: Option<Vec<u8>> = None;
    let mut reading_data = false;
    let mut read_separator = false;
    loop {
        let length = blob.msb_u32() as u64;
        let kind = blob.read(4);
        if length > 0x7fff_ffff || kind.is_empty() {
            bail!("ImproperImageHeader");
        }
        let mut kind_bytes = *b"errr";
        kind_bytes[..kind.len()].copy_from_slice(kind);
        if length > blob.size() {
            bail!("InsufficientImageDataInFile");
        }
        let mut chunk = blob.read(length as usize).to_vec();
        chunk.resize(length as usize, 0);
        blob.msb_u32();
        match &kind_bytes {
            b"JHDR" => {
                if length == 16 {
                    width = be32(&chunk);
                    height = be32(&chunk[4..]);
                    if width == 0 || height == 0 {
                        bail!("NegativeOrZeroImageSize");
                    }
                    color_type = chunk[8];
                    alpha_depth = chunk[12];
                    alpha_compression = chunk[13];
                }
                if width > 65535 || height > 65535 {
                    bail!("ImproperImageHeader");
                }
                continue;
            }
            b"JDAT" | b"JdAA" | b"IDAT" | b"JDAA" if !reading_data && !read_separator => {
                color = Some(Vec::new());
                if color_type >= 12 {
                    if alpha_compression != 0 && alpha_compression != 8 {
                        bail!("ImproperImageHeader");
                    }
                    let mut stream = Vec::new();
                    if alpha_compression == 0 {
                        stream.extend_from_slice(b"\x89PNG\r\n\x1a\n");
                        let mut header = Vec::with_capacity(13);
                        header.extend_from_slice(&width.to_be_bytes());
                        header.extend_from_slice(&height.to_be_bytes());
                        header.extend_from_slice(&[alpha_depth, 0, 0, 0, 0]);
                        stream.extend_from_slice(&png_chunk(b"IHDR", &header));
                    }
                    alpha = Some(stream);
                }
                reading_data = true;
            }
            _ => {}
        }
        match &kind_bytes {
            b"JDAT" => {
                if let Some(color) = color.as_mut() {
                    color.extend_from_slice(&chunk);
                }
            }
            b"IDAT" => {
                if let Some(alpha) = alpha.as_mut() {
                    alpha.extend_from_slice(&png_chunk(b"IDAT", &chunk));
                }
            }
            b"JDAA" | b"JdAA" => {
                if let Some(alpha) = alpha.as_mut() {
                    alpha.extend_from_slice(&chunk);
                }
            }
            b"JSEP" => read_separator = true,
            b"gAMA" => {
                if length == 4 {
                    gamma = be32(&chunk) as f64 * 0.00001;
                }
            }
            b"sRGB" => {
                if length == 1 {
                    gamma = 0.45455f32 as f64;
                }
            }
            b"IEND" => break,
            _ => {}
        }
    }
    let Some(color) = color else {
        bail!("JNG files without image data are not supported");
    };
    let jpeg = super::jpeg::decode_bytes(&color)?;
    let columns = width as usize;
    let rows = height as usize;
    if jpeg.width != columns || jpeg.height != rows {
        bail!("ImproperImageHeader");
    }
    let jpeg_stride = jpeg.channels.len();
    // `GetPixelRed`/`Green`/`Blue`: a grey image's grey for all three.
    let offsets: [usize; 3] = if jpeg.channels[0] == PixelChannel::Gray { [0, 0, 0] } else { [0, 1, 2] };
    let mut rgb = Vec::with_capacity(columns * rows * 3);
    for pixel in jpeg.data.chunks_exact(jpeg_stride) {
        for offset in offsets {
            rgb.push(pixel[offset]);
        }
    }
    let mut alpha_values: Option<Vec<f32>> = None;
    if let Some(mut stream) = alpha {
        let alpha_image = if alpha_compression == 0 {
            stream.extend_from_slice(&png_chunk(b"IEND", &[]));
            crate::pixel_cache::read_png_bytes(&stream)?
        } else {
            super::jpeg::decode_bytes(&stream)?
        };
        let mut values = Vec::with_capacity(columns * rows);
        for y in 0..rows {
            for x in 0..columns {
                // `GetVirtualPixels` past the alpha image takes its edge.
                let source_x = x.min(alpha_image.width.saturating_sub(1));
                let source_y = y.min(alpha_image.height.saturating_sub(1));
                values.push(first_channel(&alpha_image, source_y * alpha_image.width + source_x));
            }
        }
        alpha_values = Some(values);
    }
    let mut channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    let data = match alpha_values {
        Some(values) => {
            channels.push(PixelChannel::Alpha);
            let mut data = Vec::with_capacity(columns * rows * 4);
            for (pixel, value) in rgb.as_chunks::<3>().0.iter().zip(values) {
                data.extend_from_slice(pixel);
                data.push(value);
            }
            data
        }
        None => rgb,
    };
    Ok(PixelCache::new(columns, rows, channels, data, CacheColorspace::SRgb, gamma))
}
