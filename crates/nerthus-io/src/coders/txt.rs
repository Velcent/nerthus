//! ImageMagick's own text format, the pixel enumeration (`coders/txt.c`,
//! `ReadTXTImage`): a "# ImageMagick pixel enumeration: width,height,
//! [meta,]max,colorspace" line, then a line a pixel, "x,y: (values…)",
//! where an "srgb(…)" later on the line takes over the values. Lines that
//! run out leave the background (white); other text isn't an image here.

use std::path::Path;

use anyhow::{bail, Context, Result};
use nerthus_image::colorspace_transform::Colorspace;

use super::blob::Blob;
use super::color;
use super::text::{is_space, next_token, strtod};
use crate::pixel_cache::{PixelCache, PixelChannel, DEFAULT_GAMMA};

const TXT_ID: &[u8] = b"# ImageMagick pixel enumeration:";
const PATH_EXTENT: usize = 4096;
/// `MaxPixelChannels` and the first meta channel.
const MAX_PIXEL_CHANNELS: usize = 64;
const META_PIXEL_CHANNELS: u64 = 10;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

/// `sscanf` of `%lu,` fields and a `%lf,` then `%32s`: the numbers read
/// and the word, in order, stopping at the first field that doesn't match.
fn scan_header(text: &[u8], integers: usize) -> (Vec<u64>, Option<f64>, Option<Vec<u8>>, usize) {
    let mut p = 0;
    let mut values = Vec::new();
    let mut count = 0;
    let skip = |p: &mut usize| {
        while *p < text.len() && is_space(text[*p]) {
            *p += 1;
        }
    };
    for index in 0..integers {
        skip(&mut p);
        let negative = text.get(p) == Some(&b'-');
        if matches!(text.get(p), Some(b'+' | b'-')) {
            p += 1;
        }
        let start = p;
        let mut value = 0u64;
        while let Some(digit) = text.get(p).filter(|byte| byte.is_ascii_digit()) {
            value = value.wrapping_mul(10).wrapping_add((digit - b'0') as u64);
            p += 1;
        }
        if p == start {
            return (values, None, None, count);
        }
        let value = if negative { value.wrapping_neg() } else { value };
        values.push(value as u32 as u64);
        count += 1;
        if text.get(p) != Some(&b',') {
            return (values, None, None, count);
        }
        p += 1;
        let _ = index;
    }
    let (maximum, end) = strtod(text, p);
    if end == p {
        return (values, None, None, count);
    }
    count += 1;
    p = end;
    if text.get(p) != Some(&b',') {
        return (values, Some(maximum), None, count);
    }
    p += 1;
    skip(&mut p);
    let start = p;
    while p < text.len() && !is_space(text[p]) && p - start < 32 {
        p += 1;
    }
    if p == start {
        return (values, Some(maximum), None, count);
    }
    (values, Some(maximum), Some(text[start..p].to_vec()), count + 1)
}

/// `GetQuantumRange`.
fn quantum_range(depth: u32) -> u64 {
    let shift = depth.min(64) - 1;
    (1u64 << shift).wrapping_add((1u64 << shift) - 1)
}

/// `ScaleAnyToQuantum` (HDRI: no rounding).
fn scale_any(quantum: u64, range: u64) -> f32 {
    if quantum > range {
        return 65535.0;
    }
    let range = range as f64;
    let reciprocal = if range < 1.0e-12 { 1.0e12 } else { 1.0 / range };
    (65535.0 * (quantum as f64 * reciprocal)) as f32
}

/// `CastDoubleToQuantumAny`.
fn to_quantum_any(value: f64) -> u64 {
    if value.is_nan() {
        return 0;
    }
    let value = value.trunc();
    if value < 0.0 {
        return 0;
    }
    if value >= u64::MAX as f64 {
        return u64::MAX;
    }
    value as u64
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let line = blob.string().unwrap_or_default();
    if line.len() < TXT_ID.len() || !line[..TXT_ID.len()].eq_ignore_ascii_case(TXT_ID) {
        bail!("ImproperImageHeader");
    }
    let header = &line[TXT_ID.len()..];
    let (mut values, mut maximum, mut name, count) = scan_header(header, 3);
    let mut meta = 0u64;
    let mut count = count;
    if count < 5 {
        (values, maximum, name, count) = scan_header(header, 2);
    } else {
        meta = values[2];
    }
    let maximum = maximum.unwrap_or(0.0);
    let width = values.first().copied().unwrap_or(0);
    let height = values.get(1).copied().unwrap_or(0);
    if count < 4 || width == 0 || height == 0 || maximum == 0.0 || meta >= MAX_PIXEL_CHANNELS as u64 - META_PIXEL_CHANNELS {
        bail!("ImproperImageHeader");
    }
    if maximum >= 18446744073709551615.0 {
        bail!("ImproperImageHeader");
    }
    let mut depth = 1u32;
    while (quantum_range(depth) as f64) + 1.0 < maximum {
        depth += 1;
    }
    let columns = width as usize;
    let rows = height as usize;
    let mut name = name.unwrap_or_default().to_ascii_lowercase();
    let alpha = name.len() > 1 && name.last() == Some(&b'a');
    if alpha {
        name.pop();
    }
    let option = String::from_utf8_lossy(&name).to_string();
    let colorspace = Colorspace::from_option_name(&option).context("ImproperImageHeader")?;

    // The channels in the cache's order, and the background they start at.
    let gray = matches!(colorspace, Colorspace::Gray | Colorspace::LinearGray);
    let cmyk = colorspace == Colorspace::Cmyk;
    let mut channels = if gray { vec![PixelChannel::Gray] } else { vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue] };
    if cmyk {
        channels.push(PixelChannel::Black);
    }
    if alpha {
        channels.push(PixelChannel::Alpha);
    }
    channels.extend(std::iter::repeat_n(PixelChannel::Meta, meta as usize));
    let stride = channels.len();
    let mut background = vec![65535.0f32; stride];
    if cmyk {
        let white = color::srgb_to_cmyk(65535.0, 65535.0, 65535.0);
        for (sample, value) in background.iter_mut().zip(white) {
            *sample = value as f32;
        }
    }
    for (index, channel) in channels.iter().enumerate() {
        if *channel == PixelChannel::Meta {
            background[index] = 0.0;
        }
    }
    let mut data = Vec::with_capacity(columns * rows * stride);
    for _ in 0..columns * rows {
        data.extend_from_slice(&background);
    }

    let range = quantum_range(depth);
    'rows: for y in 0..rows {
        for x in 0..columns {
            let Some(text) = blob.string() else {
                break 'rows;
            };
            let mut found = [0.0f64; MAX_PIXEL_CHANNELS + 2];
            found[0] = x as f64;
            found[1] = y as f64;
            let mut n = 0;
            let mut p = 0;
            while p < text.len() && text[p] != b')' && n < MAX_PIXEL_CHANNELS {
                if text.len() >= p + 5 && text[p..p + 5].eq_ignore_ascii_case(b"sRGB(") {
                    p += 5;
                    n = 2;
                }
                let (token, end) = next_token(&text, p, PATH_EXTENT);
                p = end;
                if token.first().is_some_and(u8::is_ascii_digit) {
                    let (value, number_end) = strtod(&token, 0);
                    found[n] = value;
                    let (_, end) = next_token(&text, p, PATH_EXTENT);
                    p = end;
                    if token.get(number_end) == Some(&b'%') {
                        found[n] *= 0.01 * range as f64;
                    }
                    n += 1;
                }
            }
            // `CastDoubleToSsizeT`: NaN is 0.
            let target_x = if found[0].is_nan() { 0.0 } else { found[0].trunc() };
            let target_y = if found[1].is_nan() { 0.0 } else { found[1].trunc() };
            if !(target_x >= 0.0 && target_y >= 0.0 && (target_x as usize) < columns && (target_y as usize) < rows) {
                bail!("PixelsAreNotAuthentic");
            }
            let base = (target_y as usize * columns + target_x as usize) * stride;
            for i in 0..stride {
                data[base + i] = scale_any(to_quantum_any(found[i + 2]), range);
            }
        }
    }
    let (colorspace, gamma) = match colorspace {
        Colorspace::Rgb | Colorspace::LinearGray | Colorspace::Xyz | Colorspace::XyY | Colorspace::ScRgb => (colorspace, 1.0),
        _ => (colorspace, DEFAULT_GAMMA),
    };
    let mut cache = PixelCache::new(columns, rows, channels, data, colorspace, gamma);
    cache.alpha_trait = alpha;
    Ok(cache)
}
