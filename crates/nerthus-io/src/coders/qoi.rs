//! The Quite OK Image format as ImageMagick reads it (`coders/qoi.c`): RGB or
//! RGBA, sRGB or (by its colorspace byte) linear RGB with gamma 1. Decoding
//! stops at the end of the file; too few pixels fail the read.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::quantum::char_quantum;
use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

const OP_INDEX: u8 = 0x00;
const OP_DIFF: u8 = 0x40;
const OP_LUMA: u8 = 0x80;
const OP_RUN: u8 = 0xc0;
const OP_RGB: u8 = 0xfe;
const OP_RGBA: u8 = 0xff;
const MASK_2: u8 = 0xc0;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let magic = blob.read(4);
    if magic.len() != 4 || !magic.eq_ignore_ascii_case(b"qoif") {
        bail!("ImproperImageHeader");
    }
    let width = blob.msb_u32() as usize;
    let height = blob.msb_u32() as usize;
    if width == 0 || height == 0 {
        bail!("NegativeOrZeroImageSize");
    }
    let channel_count = blob.byte();
    let alpha = match channel_count {
        Some(3) => false,
        Some(4) => true,
        _ => bail!("ImproperImageHeader"),
    };
    let (colorspace, gamma) = match blob.byte() {
        Some(0) => (CacheColorspace::SRgb, DEFAULT_GAMMA),
        Some(1) => (CacheColorspace::Rgb, 1.0),
        _ => bail!("ImproperImageHeader"),
    };
    let stride = 3 + usize::from(alpha);
    let total = width * height;
    let mut data = vec![0.0f32; total * stride];
    let mut pixel = [0u8, 0, 0, 255];
    let mut lookup = [[0u8; 4]; 64];
    let mut index = 0usize;
    while index < total {
        let mut run = 0i32;
        let Some(op) = blob.byte() else {
            break;
        };
        if op == OP_RGB {
            let Some(r) = blob.byte() else { break };
            pixel[0] = r;
            let Some(g) = blob.byte() else { break };
            pixel[1] = g;
            let Some(b) = blob.byte() else { break };
            pixel[2] = b;
        } else if op == OP_RGBA {
            let Some(r) = blob.byte() else { break };
            pixel[0] = r;
            let Some(g) = blob.byte() else { break };
            pixel[1] = g;
            let Some(b) = blob.byte() else { break };
            pixel[2] = b;
            let Some(a) = blob.byte() else { break };
            pixel[3] = a;
        } else if op & MASK_2 == OP_INDEX {
            pixel = lookup[(op & !MASK_2) as usize];
        } else if op & MASK_2 == OP_DIFF {
            pixel[0] = pixel[0].wrapping_add(((op >> 4) & 0x03).wrapping_sub(2));
            pixel[1] = pixel[1].wrapping_add(((op >> 2) & 0x03).wrapping_sub(2));
            pixel[2] = pixel[2].wrapping_add((op & 0x03).wrapping_sub(2));
        } else if op & MASK_2 == OP_LUMA {
            let green = (op & !MASK_2) as i32 - 32;
            let Some(second) = blob.byte() else { break };
            pixel[0] = pixel[0].wrapping_add((green - 8 + ((second >> 4) & 0x0f) as i32) as u8);
            pixel[1] = pixel[1].wrapping_add(green as u8);
            pixel[2] = pixel[2].wrapping_add((green - 8 + (second & 0x0f) as i32) as u8);
        } else if op & MASK_2 == OP_RUN {
            run = (op & !MASK_2) as i32;
        }
        let hash = (pixel[0] as usize * 3 + pixel[1] as usize * 5 + pixel[2] as usize * 7 + pixel[3] as usize * 11) % 64;
        lookup[hash] = pixel;
        loop {
            if index < total {
                let target = &mut data[index * stride..index * stride + stride];
                for (channel, &value) in target.iter_mut().zip(pixel.iter()) {
                    *channel = char_quantum(value);
                }
            }
            index += 1;
            if run <= 0 {
                break;
            }
            run -= 1;
        }
    }
    if index < total {
        bail!("NotEnoughPixelData");
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    let mut channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    if alpha {
        channels.push(PixelChannel::Alpha);
    }
    Ok(PixelCache::new(width, height, channels, data, colorspace, gamma))
}
