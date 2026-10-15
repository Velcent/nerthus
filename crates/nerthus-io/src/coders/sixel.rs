//! DEC SIXEL graphics as ImageMagick reads them (`coders/sixel.c`): the
//! file's lines joined, then the sixel stream drawn into a growing canvas
//! of colour indices (the VT340 and xterm palette until `#` registers
//! change it), cropped to what was drawn or the raster attributes' size.
//! PseudoClass over the colours up to the highest one used (three at
//! least).

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::colormap;
use super::text::joined_lines;
use crate::pixel_cache::{char_quantum, CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

const PALETTE_MAX: usize = 1024;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

fn rgb(red: i64, green: i64, blue: i64) -> i32 {
    (((red & 0xff) << 16) + ((green & 0xff) << 8) + (blue & 0xff)) as i32
}

/// `SIXEL_PALVAL(n, 255, 100)`.
fn percent(value: i64) -> i64 {
    (value * 255 + 50) / 100
}

fn xrgb(red: i64, green: i64, blue: i64) -> i32 {
    rgb(percent(red), percent(green), percent(blue))
}

fn hue_to_rgb(n1: i64, n2: i64, mut hue: i64) -> i64 {
    const HLSMAX: i64 = 100;
    if hue < 0 {
        hue += HLSMAX;
    }
    if hue > HLSMAX {
        hue -= HLSMAX;
    }
    if hue < HLSMAX / 6 {
        return n1 + ((n2 - n1) * hue + HLSMAX / 12) / (HLSMAX / 6);
    }
    if hue < HLSMAX / 2 {
        return n2;
    }
    if hue < (HLSMAX * 2) / 3 {
        return n1 + ((n2 - n1) * ((HLSMAX * 2) / 3 - hue) + HLSMAX / 12) / (HLSMAX / 6);
    }
    n1
}

fn hls_to_rgb(hue: i64, lum: i64, sat: i64) -> i32 {
    const HLSMAX: i64 = 100;
    const RGBMAX: i64 = 255;
    let (red, green, blue);
    if sat == 0 {
        let grey = (lum * RGBMAX) / HLSMAX;
        (red, green, blue) = (grey, grey, grey);
    } else {
        let magic2 = if lum <= HLSMAX / 2 { (lum * (HLSMAX + sat) + HLSMAX / 2) / HLSMAX } else { lum + sat - ((lum * sat) + HLSMAX / 2) / HLSMAX };
        let magic1 = 2 * lum - magic2;
        blue = (hue_to_rgb(magic1, magic2, hue + HLSMAX / 3) * RGBMAX + HLSMAX / 2) / HLSMAX;
        red = (hue_to_rgb(magic1, magic2, hue) * RGBMAX + HLSMAX / 2) / HLSMAX;
        green = (hue_to_rgb(magic1, magic2, hue - HLSMAX / 3) * RGBMAX + HLSMAX / 2) / HLSMAX;
    }
    rgb(red, green, blue)
}

fn at(bytes: &[u8], index: usize) -> u8 {
    bytes.get(index).copied().unwrap_or(0)
}

/// `get_params`: numbers separated by `;` (an empty field is 0), ten at
/// most kept.
fn get_params(bytes: &[u8], mut p: usize, param: &mut [i64; 10]) -> (usize, usize) {
    let mut len = 0;
    while at(bytes, p) != 0 {
        while matches!(at(bytes, p), b' ' | b'\t') {
            p += 1;
        }
        if at(bytes, p).is_ascii_digit() {
            let mut n = 0i64;
            while at(bytes, p).is_ascii_digit() {
                let next = n * 10 + (at(bytes, p) - b'0') as i64;
                if next > i32::MAX as i64 {
                    n = i32::MAX as i64;
                    break;
                }
                n = next;
                p += 1;
            }
            if len < 10 {
                param[len] = n;
                len += 1;
            }
            while matches!(at(bytes, p), b' ' | b'\t') {
                p += 1;
            }
            if at(bytes, p) == b';' {
                p += 1;
            }
        } else if at(bytes, p) == b';' {
            if len < 10 {
                param[len] = 0;
                len += 1;
            }
            p += 1;
        } else {
            break;
        }
    }
    (p, len)
}

/// A canvas of colour indices that grows as IM's does.
struct Canvas {
    width: i64,
    height: i64,
    indices: Vec<u16>,
}

impl Canvas {
    fn resize(&mut self, width: i64, height: i64) -> Result<()> {
        // IM can't allocate a canvas this big either.
        if width as u64 * height as u64 > (1u64 << 28) {
            bail!("CorruptImage");
        }
        let mut indices = vec![0u16; (width * height) as usize];
        let copy = self.width.min(width) as usize;
        for y in 0..self.height.min(height) as usize {
            let source = y * self.width as usize;
            let target = y * width as usize;
            indices[target..target + copy].copy_from_slice(&self.indices[source..source + copy]);
        }
        self.width = width;
        self.height = height;
        self.indices = indices;
        Ok(())
    }
}

/// `sixel_decode`: the canvas and the palette of the colours used.
fn sixel_decode(bytes: &[u8]) -> Result<(Canvas, Vec<i32>)> {
    let extent = bytes.iter().position(|&byte| byte == 0).unwrap_or(bytes.len());
    let mut position_x: i64 = 0;
    let mut position_y: i64 = 0;
    let mut max_x: i64 = 0;
    let mut max_y: i64 = 0;
    let mut attributed_ph: i64 = 0;
    let mut attributed_pv: i64 = 0;
    let mut repeat_count: i64 = 1;
    let mut color_index: i64 = 0;
    let mut max_color_index: i64 = 2;
    let mut param = [0i64; 10];
    let mut canvas = Canvas { width: 2048, height: 2048, indices: vec![0; 2048 * 2048] };
    let mut palette = vec![0i32; PALETTE_MAX];
    let defaults = [(0, 0, 0), (20, 20, 80), (80, 13, 13), (20, 80, 20), (80, 20, 80), (20, 80, 80), (80, 80, 20), (53, 53, 53), (26, 26, 26), (33, 33, 60), (60, 26, 26), (33, 60, 33), (60, 33, 60), (33, 60, 60), (60, 60, 33), (80, 80, 80)];
    let mut n = 0;
    for (red, green, blue) in defaults {
        palette[n] = xrgb(red, green, blue);
        n += 1;
    }
    for red in 0..6 {
        for green in 0..6 {
            for blue in 0..6 {
                palette[n] = rgb(red * 51, green * 51, blue * 51);
                n += 1;
            }
        }
    }
    for grey in 0..24 {
        palette[n] = rgb(grey * 11, grey * 11, grey * 11);
        n += 1;
    }
    for entry in palette.iter_mut().skip(n) {
        *entry = rgb(255, 255, 255);
    }

    let mut p = 0usize;
    while at(bytes, p) != 0 {
        let here = at(bytes, p);
        if (here == 0x1b && at(bytes, p + 1) == b'P') || here == 0x90 {
            if here == 0x1b {
                p += 1;
            }
            // The pixel aspect ratio parameters change nothing drawn.
            (p, _) = get_params(bytes, p + 1, &mut param);
            if at(bytes, p) == b'q' {
                p += 1;
            }
        } else if (here == 0x1b && at(bytes, p + 1) == b'\\') || here == 0x9c {
            break;
        } else if here == b'"' {
            let count;
            (p, count) = get_params(bytes, p + 1, &mut param);
            if count > 2 && param[2] > 0 {
                attributed_ph = param[2] & 0xffff;
            }
            if count > 3 && param[3] > 0 {
                attributed_pv = param[3] & 0xffff;
            }
            if canvas.width < attributed_ph || canvas.height < attributed_pv {
                let width = canvas.width.max(attributed_ph);
                let height = canvas.height.max(attributed_pv);
                canvas.resize(width, height)?;
            }
        } else if here == b'!' {
            let count;
            (p, count) = get_params(bytes, p + 1, &mut param);
            if count > 0 && param[0] > 0 {
                repeat_count = param[0];
                if repeat_count > extent as i64 {
                    break;
                }
            }
        } else if here == b'#' {
            let count;
            (p, count) = get_params(bytes, p + 1, &mut param);
            if count > 0 {
                color_index = param[0];
                if color_index < 0 {
                    color_index = 0;
                } else if color_index >= PALETTE_MAX as i64 {
                    color_index = PALETTE_MAX as i64 - 1;
                }
            }
            if count > 4 {
                if param[1] == 1 {
                    param[2] = param[2].min(360);
                    param[3] = param[3].min(100);
                    param[4] = param[4].min(100);
                    palette[color_index as usize] = hls_to_rgb(param[2] * 100 / 360, param[3], param[4]);
                } else if param[1] == 2 {
                    param[2] = param[2].min(100);
                    param[3] = param[3].min(100);
                    param[4] = param[4].min(100);
                    palette[color_index as usize] = xrgb(param[2], param[3], param[4]);
                }
            }
        } else if here == b'$' {
            p += 1;
            position_x = 0;
            repeat_count = 1;
        } else if here == b'-' {
            p += 1;
            position_x = 0;
            position_y += 6;
            repeat_count = 1;
        } else if (b'?'..=0x7f).contains(&here) {
            if canvas.width < position_x + repeat_count || canvas.height < position_y + 6 {
                let mut width = canvas.width * 2;
                let mut height = canvas.height * 2;
                while width < position_x + repeat_count || height < position_y + 6 {
                    width *= 2;
                    height *= 2;
                }
                canvas.resize(width, height)?;
            }
            if color_index > max_color_index {
                max_color_index = color_index;
            }
            let bits = (here - b'?') as i64;
            p += 1;
            if bits == 0 {
                position_x += repeat_count;
            } else if repeat_count <= 1 {
                let mut mask = 0x01;
                for i in 0..6 {
                    if bits & mask != 0 {
                        let offset = canvas.width * (position_y + i) + position_x;
                        if offset < 0 || offset >= canvas.width * canvas.height {
                            bail!("CorruptImage");
                        }
                        canvas.indices[offset as usize] = color_index as u16;
                        max_x = max_x.max(position_x);
                        max_y = max_y.max(position_y + i);
                    }
                    mask <<= 1;
                }
                position_x += 1;
            } else {
                let mut mask = 0x01i64;
                let mut i = 0;
                while i < 6 {
                    if bits & mask != 0 {
                        let mut c = mask << 1;
                        let mut run = 1;
                        while i + run < 6 {
                            if bits & c == 0 {
                                break;
                            }
                            c <<= 1;
                            run += 1;
                        }
                        for y in position_y + i..position_y + i + run {
                            let offset = canvas.width * y + position_x;
                            if offset < 0 || offset + repeat_count >= canvas.width * canvas.height {
                                bail!("CorruptImage");
                            }
                            let start = offset as usize;
                            canvas.indices[start..start + repeat_count as usize].fill(color_index as u16);
                        }
                        max_x = max_x.max(position_x + repeat_count - 1);
                        max_y = max_y.max(position_y + i + run - 1);
                        i += run - 1;
                        mask <<= run - 1;
                    }
                    mask <<= 1;
                    i += 1;
                }
                position_x += repeat_count;
            }
            repeat_count = 1;
        } else {
            p += 1;
        }
    }
    max_x += 1;
    if max_x < attributed_ph {
        max_x = attributed_ph;
    }
    max_y += 1;
    if max_y < attributed_pv {
        max_y = attributed_pv;
    }
    if canvas.width > max_x || canvas.height > max_y {
        canvas.resize(max_x, max_y)?;
    }
    let colors = (max_color_index + 1) as usize;
    Ok((canvas, palette[..colors].to_vec()))
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let buffer = joined_lines(&mut blob);
    let (canvas, palette) = sixel_decode(&buffer)?;
    let columns = canvas.width as usize;
    let rows = canvas.height as usize;
    if columns == 0 || rows == 0 {
        bail!("NegativeOrZeroImageSize");
    }
    let mut entries = colormap::acquire(palette.len()).context("MemoryAllocationFailed")?;
    for (entry, &value) in entries.iter_mut().zip(palette.iter()) {
        entry[0] = char_quantum((value >> 16 & 0xff) as u8) as f64;
        entry[1] = char_quantum((value >> 8 & 0xff) as u8) as f64;
        entry[2] = char_quantum((value & 0xff) as u8) as f64;
    }
    let mut data = Vec::with_capacity(columns * rows * 4);
    for &index in &canvas.indices {
        let index = index as usize;
        if index >= entries.len() {
            bail!("InvalidColormapIndex");
        }
        let entry = entries[index];
        data.extend([entry[0] as f32, entry[1] as f32, entry[2] as f32, index as f32]);
    }
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Index];
    let mut cache = PixelCache::new(columns, rows, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA);
    cache.colormap = entries;
    Ok(cache)
}
