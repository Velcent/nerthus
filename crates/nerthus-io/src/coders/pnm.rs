//! Netpbm images as ImageMagick reads them (`coders/pnm.c`): plain and raw
//! PBM, PGM and PPM, PAM by its tuple type, and the PFM and PHM float
//! formats (rows bottom to top, little-endian when the scale is negative).
//! The depth is the fewest bits that hold the maximum value; 8-, 16- and
//! 32-bit raw samples are taken as full-range bytes, shorts and longs
//! whatever that maximum, other depths scaled by it. A `4` straight after
//! the magic number's digit gives the image an alpha channel. Grey formats
//! are GRAY, the rest sRGB (PAM's CMYK tuples CMYK); pixels start at zero.
//! Magick.NET reads one scene, so only the first image of a file is read.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::quantum::{self, char_quantum, long_quantum, quantum_range, scale_any, Endian, Picture, QuantumInfo, QuantumType};
use crate::pixel_cache::{CacheColorspace, ImageType, PixelCache, DEFAULT_GAMMA};

const QUANTUM_RANGE: f32 = 65535.0;
const MAGICK_EPSILON: f64 = 1.0e-12;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

/// `PNMComment`: the rest of a `#` line; the character that ended it.
fn comment(blob: &mut Blob) -> Option<u8> {
    let mut c = Some(b'#');
    while let Some(byte) = c {
        if byte == b'\n' || byte == b'\r' {
            break;
        }
        c = blob.byte();
    }
    c
}

/// `PNMInteger`: a decimal number after whitespace and comments (one digit
/// for `base` 2, as plain PBM packs its bits); zero at the end of the file.
fn integer(blob: &mut Blob, base: u32) -> u32 {
    let mut c;
    loop {
        let Some(byte) = blob.byte() else {
            return 0;
        };
        c = Some(byte);
        if byte == b'#' {
            c = comment(blob);
        }
        if !matches!(c, Some(b' ' | b'\t' | b'\n' | b'\r')) {
            break;
        }
    }
    let Some(first) = c else {
        // A comment that ran to the end of the file: `c` is EOF (-1).
        return if base == 2 { (-1i32 - b'0' as i32) as u32 } else { 0 };
    };
    if base == 2 {
        return (first as u32).wrapping_sub(b'0' as u32);
    }
    let mut value = 0u32;
    let mut current = first;
    while current.is_ascii_digit() {
        if value <= (i32::MAX / 10) as u32 {
            value *= 10;
            let digit = (current - b'0') as u32;
            if value <= i32::MAX as u32 - digit {
                value += digit;
            }
        }
        match blob.byte() {
            Some(byte) => current = byte,
            None => return value,
        }
    }
    if current == b'#' {
        comment(blob);
    }
    value
}

/// `PNMString`: up to the end of the line (`None` at the end of the file).
fn string(blob: &mut Blob, extent: usize) -> Option<String> {
    let mut text = Vec::new();
    for index in 0..extent - 1 {
        let Some(byte) = blob.byte() else {
            if index == 0 {
                return None;
            }
            break;
        };
        if byte == b'\n' || byte == b'\r' {
            break;
        }
        text.push(byte);
    }
    Some(String::from_utf8_lossy(&text).to_string())
}

/// `StringToDouble`: the longest leading number (`strtod`).
fn string_to_double(text: &str) -> f64 {
    let trimmed = text.trim_start();
    let mut best = 0.0;
    for end in 1..=trimmed.len() {
        if !trimmed.is_char_boundary(end) {
            continue;
        }
        if let Ok(value) = trimmed[..end].parse::<f64>() {
            best = value;
        }
    }
    best
}

/// C's `isspace`.
fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

/// `PNMEOFBlob`: the file ran out before the last pixel.
fn ran_out(blob: &Blob, x: usize, y: usize, width: usize, height: usize) -> bool {
    blob.eof && (x < width - 1 || y != height - 1)
}

/// `IsPixelGray` and `IsPixelMonochrome`.
fn is_gray(rgb: [f32; 3]) -> bool {
    ((rgb[0] as f64 - rgb[1] as f64).abs() < MAGICK_EPSILON) && ((rgb[1] as f64 - rgb[2] as f64).abs() < MAGICK_EPSILON)
}

fn is_monochrome(rgb: [f32; 3]) -> bool {
    let red = rgb[0] as f64;
    if red.abs() >= MAGICK_EPSILON && (red - QUANTUM_RANGE as f64).abs() >= MAGICK_EPSILON {
        return false;
    }
    is_gray(rgb)
}

/// Reads a big-endian sample of `bytes` bytes.
fn push(row: &[u8], position: &mut usize, bytes: usize) -> u32 {
    let mut value = 0u32;
    for _ in 0..bytes {
        value = (value << 8) | row.get(*position).copied().unwrap_or(0) as u32;
        *position += 1;
    }
    value
}

/// One image's header and pixels.
fn decode_one(blob: &mut Blob, _first: bool) -> Result<Option<(PixelCache, u8)>> {
    let Some(magic) = blob.byte() else {
        bail!("ImproperImageHeader");
    };
    if magic != b'P' {
        bail!("ImproperImageHeader");
    }
    let format = blob.byte().unwrap_or(0xFF);
    let mut alpha = false;
    let mut width = 0usize;
    let mut height = 0usize;
    let mut max_value: u64 = 1;
    let mut colorspace = None;
    let mut quantum_type = None;
    let mut quantum_scale = 1.0f64;
    if format != b'7' {
        if blob.byte() == Some(b'4') {
            alpha = true;
        }
        width = integer(blob, 10) as usize;
        height = integer(blob, 10) as usize;
        if matches!(format, b'f' | b'F' | b'h' | b'H') {
            if let Some(scale) = string(blob, 32) {
                quantum_scale = string_to_double(&scale);
            }
        } else if matches!(format, b'1' | b'4') {
            max_value = 1;
        } else {
            max_value = integer(blob, 10) as u64;
        }
    } else {
        // PAM: keyword and value lines up to ENDHDR. Each line's terminator
        // is dropped (the loop reads on from the next byte).
        let mut c = blob.byte();
        while c.is_some() {
            while c.is_some_and(is_space) {
                c = blob.byte();
            }
            if c == Some(b'#') {
                while c == Some(b'#') {
                    comment(blob);
                    c = blob.byte();
                }
                while c.is_some_and(is_space) {
                    c = blob.byte();
                }
            }
            let mut keyword = Vec::new();
            loop {
                keyword.push(c.unwrap_or(0xFF));
                c = blob.byte();
                if !c.is_some_and(|value| value.is_ascii_alphanumeric()) {
                    break;
                }
            }
            let keyword = String::from_utf8_lossy(&keyword).to_ascii_lowercase();
            if keyword == "endhdr" {
                break;
            }
            while c.is_some_and(is_space) {
                c = blob.byte();
            }
            let mut value = Vec::new();
            while let Some(character) = c.filter(|value| value.is_ascii_alphanumeric() || *value == b'_') {
                value.push(character);
                c = blob.byte();
            }
            let value = String::from_utf8_lossy(&value).to_string();
            let digits: String = value.chars().take_while(|character| character.is_ascii_digit()).collect();
            let number = digits.parse::<u64>().unwrap_or(0);
            match keyword.as_str() {
                "height" => height = number as usize,
                "maxval" => max_value = number,
                "width" => width = number as usize,
                "tupltype" if quantum_type.is_none() => {
                    match value.to_ascii_uppercase().as_str() {
                        "BLACKANDWHITE" | "GRAYSCALE" => {
                            colorspace = Some(CacheColorspace::Gray);
                            quantum_type = Some(QuantumType::Gray);
                        }
                        "BLACKANDWHITE_ALPHA" | "GRAYSCALE_ALPHA" => {
                            colorspace = Some(CacheColorspace::Gray);
                            alpha = true;
                            quantum_type = Some(QuantumType::GrayAlpha);
                        }
                        "RGB_ALPHA" => {
                            alpha = true;
                            quantum_type = Some(QuantumType::Rgba);
                        }
                        "CMYK" => {
                            colorspace = Some(CacheColorspace::Cmyk);
                            quantum_type = Some(QuantumType::Cmyk);
                        }
                        "CMYK_ALPHA" => {
                            colorspace = Some(CacheColorspace::Cmyk);
                            alpha = true;
                            quantum_type = Some(QuantumType::Cmyka);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            c = blob.byte();
        }
    }
    let quantum_type = quantum_type.unwrap_or(QuantumType::Rgb);
    if width == 0 || height == 0 {
        bail!("NegativeOrZeroImageSize");
    }
    if max_value == 0 || max_value > 4_294_967_295 {
        bail!("ImproperImageHeader");
    }
    let mut depth = 1u32;
    while quantum_range(depth) < max_value {
        depth += 1;
    }
    if (width * height / 8) as u64 > blob.size() {
        bail!("InsufficientImageDataInFile");
    }

    let gray = matches!(format, b'1' | b'2' | b'4' | b'5' | b'f' | b'h') || colorspace == Some(CacheColorspace::Gray);
    let cmyk = colorspace == Some(CacheColorspace::Cmyk);
    let mut picture = Picture::new(width, height, gray, cmyk, alpha, false, 0);
    picture.reset();
    let mut image_type = ImageType::Undefined;
    let mut rows_read = 0usize;
    let mut colours_gray = true;
    let mut colours_mono = true;
    let range = max_value;
    match format {
        b'1' => {
            'rows: for y in 0..height {
                for x in 0..width {
                    let value = if integer(blob, 2) == 0 { QUANTUM_RANGE } else { 0.0 };
                    picture.set_red(y * width + x, value);
                    if ran_out(blob, x, y, width, height) {
                        break 'rows;
                    }
                }
                rows_read = y + 1;
            }
            image_type = ImageType::Bilevel;
        }
        b'2' => {
            'rows: for y in 0..height {
                for x in 0..width {
                    let value = scale_any(integer(blob, 10) as u64, range);
                    if ran_out(blob, x, y, width, height) {
                        break 'rows;
                    }
                    picture.set_red(y * width + x, value);
                }
                rows_read = y + 1;
            }
            image_type = ImageType::Grayscale;
        }
        b'3' => {
            'rows: for y in 0..height {
                for x in 0..width {
                    let pixel = y * width + x;
                    let red = scale_any(integer(blob, 10) as u64, range);
                    if ran_out(blob, x, y, width, height) {
                        break 'rows;
                    }
                    picture.set_red(pixel, red);
                    let green = scale_any(integer(blob, 10) as u64, range);
                    picture.set_green(pixel, green);
                    let blue = scale_any(integer(blob, 10) as u64, range);
                    picture.set_blue(pixel, blue);
                    let rgb = picture.rgb(pixel);
                    colours_gray &= is_gray(rgb);
                    colours_mono &= is_monochrome(rgb);
                }
                rows_read = y + 1;
            }
            if colours_gray {
                image_type = ImageType::Grayscale;
            }
            if colours_mono {
                image_type = ImageType::Bilevel;
            }
        }
        b'4' | b'5' | b'7' => {
            let mut info = QuantumInfo::new(depth, false);
            info.endian = Endian::Msb;
            let kind = if format == b'4' {
                info.min_is_white = true;
                QuantumType::Gray
            } else if format == b'5' {
                QuantumType::Gray
            } else {
                quantum_type
            };
            let sample_bytes = if depth <= 8 { 1 } else if depth <= 16 { 2 } else { 4 };
            let samples_per_pixel = match kind {
                QuantumType::Gray | QuantumType::GrayAlpha => 1,
                QuantumType::Cmyk | QuantumType::Cmyka => 4,
                _ => 3,
            } + usize::from(alpha && format == b'7');
            let extent = if format == b'4' { info.extent(&picture, kind) } else { samples_per_pixel * sample_bytes * width };
            for y in 0..height {
                let row = blob.read(extent);
                if row.len() != extent {
                    break;
                }
                if format == b'4' || matches!(depth, 8 | 16 | 32) {
                    quantum::import(&mut picture, &info, kind, y * width, width, row)?;
                } else {
                    custom_row(&mut picture, row, y * width, width, kind, depth, alpha && format == b'7', range, format);
                }
                rows_read = y + 1;
            }
            image_type = quantum::image_type(kind, depth);
        }
        b'6' => {
            let sample_bytes = if depth <= 8 { 1 } else if depth <= 16 { 2 } else { 4 };
            let extent = 3 * sample_bytes * width;
            for y in 0..height {
                let row = blob.read(extent);
                if row.len() != extent {
                    break;
                }
                let mut position = 0;
                for x in 0..width {
                    let pixel = y * width + x;
                    let mut rgb = [0.0f32; 3];
                    for value in rgb.iter_mut() {
                        let sample = push(row, &mut position, sample_bytes);
                        *value = match depth {
                            8 => char_quantum(sample as u8),
                            16 => sample as f32,
                            32 => long_quantum(sample),
                            _ => scale_any(sample as u64, range),
                        };
                    }
                    picture.set_red(pixel, rgb[0]);
                    picture.set_green(pixel, rgb[1]);
                    picture.set_blue(pixel, rgb[2]);
                    picture.set_alpha(pixel, QUANTUM_RANGE);
                    colours_gray &= is_gray(rgb);
                    colours_mono &= is_monochrome(rgb);
                }
                rows_read = y + 1;
            }
            if colours_gray {
                image_type = ImageType::Grayscale;
            }
            if colours_mono {
                image_type = ImageType::Bilevel;
            }
        }
        b'F' | b'f' | b'H' | b'h' => {
            let kind = if format == b'f' || format == b'h' {
                QuantumType::Gray
            } else if alpha {
                QuantumType::Rgba
            } else {
                QuantumType::Rgb
            };
            let bits = if matches!(format, b'F' | b'f') { 32 } else { 16 };
            let mut info = QuantumInfo::new(bits, true);
            info.endian = if quantum_scale < 0.0 { Endian::Lsb } else { Endian::Msb };
            info.scale = QUANTUM_RANGE as f64 * quantum_scale.abs();
            let extent = info.extent(&picture, kind);
            for row_index in 0..height {
                let row = blob.read(extent);
                if row.len() != extent {
                    break;
                }
                let y = height - row_index - 1;
                quantum::import(&mut picture, &info, kind, y * width, width, row)?;
                rows_read = row_index + 1;
            }
            image_type = quantum::image_type(kind, bits);
        }
        _ => bail!("ImproperImageHeader"),
    }
    if rows_read < height {
        bail!("UnableToReadImageData");
    }
    let colorspace = colorspace.unwrap_or(if gray { CacheColorspace::Gray } else { CacheColorspace::SRgb });
    Ok(Some((picture.into_cache(gray, colorspace, DEFAULT_GAMMA, image_type), format)))
}

/// A raw PGM or PAM row at a depth other than 8, 16 or 32: big-endian
/// samples of a byte, a short or a long, each scaled by the maximum value.
#[allow(clippy::too_many_arguments)]
fn custom_row(picture: &mut Picture, row: &[u8], start: usize, width: usize, kind: QuantumType, depth: u32, alpha: bool, range: u64, format: u8) {
    let sample_bytes = if depth <= 8 { 1 } else if depth <= 16 { 2 } else { 4 };
    let mut position = 0;
    for x in 0..width {
        let pixel = start + x;
        let mut next = || scale_any(push(row, &mut position, sample_bytes) as u64, range);
        if format == b'5' {
            picture.set_red(pixel, next());
            continue;
        }
        match kind {
            QuantumType::Gray | QuantumType::GrayAlpha => {
                picture.set_red(pixel, next());
                picture.set_alpha(pixel, QUANTUM_RANGE);
                if alpha {
                    let value = next();
                    let value = if depth == 1 { QUANTUM_RANGE - value } else { value };
                    picture.set_alpha(pixel, value);
                }
            }
            QuantumType::Cmyk | QuantumType::Cmyka => {
                picture.set_red(pixel, next());
                picture.set_green(pixel, next());
                picture.set_blue(pixel, next());
                picture.set_black(pixel, next());
                picture.set_alpha(pixel, QUANTUM_RANGE);
                if alpha {
                    picture.set_alpha(pixel, next());
                }
            }
            _ => {
                picture.set_red(pixel, next());
                picture.set_green(pixel, next());
                picture.set_blue(pixel, next());
                picture.set_alpha(pixel, QUANTUM_RANGE);
                if alpha {
                    picture.set_alpha(pixel, next());
                }
            }
        }
    }
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    // Magick.NET asks for one scene, so the reader stops after it.
    let Some((first, _)) = decode_one(&mut blob, true)? else {
        bail!("ImproperImageHeader");
    };
    Ok(first)
}
