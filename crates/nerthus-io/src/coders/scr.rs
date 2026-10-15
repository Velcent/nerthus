//! ZX Spectrum screen dumps as ImageMagick reads them (`coders/scr.c`):
//! 6144 bytes of bitmap in the Spectrum's interleaved line order, then 768
//! attribute bytes (ink, paper and brightness a cell), 256×192 RGB.

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::pixel_cache::{char_quantum, CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

const PALETTE: [[u8; 3]; 16] = [
    [0, 0, 0],
    [0, 0, 192],
    [192, 0, 0],
    [192, 0, 192],
    [0, 192, 0],
    [0, 192, 192],
    [192, 192, 0],
    [192, 192, 192],
    [0, 0, 0],
    [0, 0, 255],
    [255, 0, 0],
    [255, 0, 255],
    [0, 255, 0],
    [0, 255, 255],
    [255, 255, 0],
    [255, 255, 255],
];

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    if bytes.len() < 6144 + 768 {
        bail!("ImproperImageHeader");
    }
    let (bitmap, attributes) = bytes.split_at(6144);
    let mut data = vec![0.0f32; 256 * 192 * 3];
    for zone in 0..3 {
        for zone_line in 0..8 {
            for octet_line in 0..8 {
                for octet in zone_line * 32..zone_line * 32 + 32 {
                    let value = bitmap[octet + 256 * octet_line + zone * 2048];
                    let attribute = attributes[octet + 256 * zone];
                    let bright = (attribute >> 6) & 1 != 0;
                    let mut ink = (attribute & 0x07) as usize;
                    let mut paper = ((attribute >> 3) & 0x07) as usize;
                    if bright {
                        ink += 8;
                        paper += 8;
                    }
                    let x = 8 * octet - 256 * zone_line;
                    let y = octet_line + 8 * zone_line + zone * 64;
                    for bit in 0..8 {
                        let colour = if value & (0x80 >> bit) != 0 { PALETTE[ink] } else { PALETTE[paper] };
                        let base = (y * 256 + x + bit) * 3;
                        for channel in 0..3 {
                            data[base + channel] = char_quantum(colour[channel]);
                        }
                    }
                }
            }
        }
    }
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    Ok(PixelCache::new(256, 192, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA))
}
