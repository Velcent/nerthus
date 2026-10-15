//! MIFF, ImageMagick's own format (`coders/miff.c`): a text header of
//! `key=value` pairs ending in `:` and one more byte, the profiles (each a
//! big-endian length and its bytes), a PseudoClass image's colormap, then
//! the pixels row by row — uncompressed, RLE (a packet and a repeat count),
//! or deflated (a zlib stream in length-prefixed chunks). Samples turn into
//! quanta as `ImportQuantumPixels` does: 8 bits ×257, 16 bits as they are,
//! 32 bits ÷65537, floats ×65535; other depths `ScaleAnyToQuantum`.
//! Pixels may also come as BZip2 or LZMA (xz) streams. Images may be in
//! any of IM's colorspaces (CMYK has a black sample after blue, grey one
//! sample, the others three) and multispectral, with meta channels after
//! the colours and alpha; the properties that colorspace transforms read
//! (Log film settings, Jzazbz's white luminance) come from the header.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::pixel_cache::{CacheColorspace, ImageType, PixelCache, PixelChannel, TransformSettings, DEFAULT_GAMMA};

/// The header's keywords (keys lowercased), and where the data starts.
struct Header {
    values: HashMap<String, String>,
    data_start: usize,
}

impl Header {
    fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    fn number(&self, key: &str) -> Option<usize> {
        self.get(key).and_then(|text| text.trim().parse().ok())
    }
}

/// `ReadMIFFImage`'s header loop: `{comments}`, `key=value` (a value in
/// braces may hold spaces), up to the `:` that ends the header; the byte
/// after it (`\x1a`) is skipped.
fn parse_header(bytes: &[u8]) -> Result<Header> {
    let mut values = HashMap::new();
    let mut position = 0;
    let next = |position: &mut usize| -> Option<u8> {
        let byte = bytes.get(*position).copied();
        *position += 1;
        byte
    };
    let mut c = next(&mut position);
    loop {
        let Some(byte) = c else {
            bail!("ImproperImageHeader");
        };
        if byte == b':' || !(byte.is_ascii_graphic() || byte.is_ascii_whitespace()) {
            break;
        }
        if byte.is_ascii_whitespace() {
            c = next(&mut position);
            continue;
        }
        if byte == b'{' {
            // A comment: everything to the closing brace.
            while let Some(inner) = next(&mut position) {
                if inner == b'\\' {
                    next(&mut position);
                    continue;
                }
                if inner == b'}' {
                    break;
                }
            }
            c = next(&mut position);
            continue;
        }
        if !byte.is_ascii_alphanumeric() {
            c = next(&mut position);
            continue;
        }
        let mut keyword = Vec::new();
        let mut current = Some(byte);
        while let Some(character) = current {
            if character == b'=' {
                break;
            }
            keyword.push(character);
            current = next(&mut position);
        }
        while current.is_some_and(|character| character.is_ascii_whitespace()) {
            current = next(&mut position);
        }
        let mut value = Vec::new();
        if current == Some(b'=') {
            current = next(&mut position);
            let braced = current == Some(b'{');
            if braced {
                current = next(&mut position);
            }
            while let Some(character) = current {
                if character == b'}' || (!braced && character == b'{') {
                    break;
                }
                value.push(character);
                current = next(&mut position);
                if current == Some(b'\\') {
                    let escaped = next(&mut position);
                    if matches!(escaped, Some(b'{') | Some(b'}')) {
                        value.push(escaped.unwrap_or_default());
                        current = next(&mut position);
                    } else {
                        current = escaped;
                    }
                }
                if !braced && current.is_some_and(|character| character.is_ascii_whitespace()) {
                    break;
                }
            }
            if braced && current == Some(b'}') {
                current = next(&mut position);
            }
        }
        let key = String::from_utf8_lossy(&keyword).to_ascii_lowercase();
        let text = String::from_utf8_lossy(&value).to_string();
        if key == "profile" {
            let profiles = values.entry("profile".to_string()).or_insert_with(String::new);
            if !profiles.is_empty() {
                profiles.push('\n');
            }
            profiles.push_str(&text);
        } else {
            values.insert(key, text);
        }
        c = current;
    }
    // One character beyond the `:`.
    let data_start = position + 1;
    Ok(Header { values, data_start })
}

/// How samples are stored.
#[derive(Clone, Copy, PartialEq)]
enum Format {
    Unsigned,
    Float,
}

/// Reads samples of `depth` bits from a byte stream in the file's order.
struct Samples<'a> {
    bytes: &'a [u8],
    position: usize,
    depth: usize,
    format: Format,
    little_endian: bool,
    /// For depths that are not whole bytes: the bits left of the current
    /// byte.
    bit_buffer: u32,
    bits: usize,
}

impl<'a> Samples<'a> {
    fn new(bytes: &'a [u8], depth: usize, format: Format, little_endian: bool) -> Self {
        Self { bytes, position: 0, depth, format, little_endian, bit_buffer: 0, bits: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let slice = self.bytes.get(self.position..self.position + count).context("UnableToReadImageData")?;
        self.position += count;
        Ok(slice)
    }

    fn word(&mut self, count: usize) -> Result<u64> {
        let bytes = self.take(count)?;
        let mut value = 0u64;
        for index in 0..count {
            let byte = if self.little_endian { bytes[count - 1 - index] } else { bytes[index] };
            value = (value << 8) | byte as u64;
        }
        Ok(value)
    }

    /// The next sample as `PushQuantumPixel` reads packed bits.
    fn bits(&mut self) -> Result<u64> {
        let mut value = 0u64;
        let mut remaining = self.depth;
        while remaining > 0 {
            if self.bits == 0 {
                self.bit_buffer = self.take(1)?[0] as u32;
                self.bits = 8;
            }
            let count = remaining.min(self.bits);
            let shift = self.bits - count;
            let part = (self.bit_buffer >> shift) & ((1u32 << count) - 1);
            value = (value << count) | part as u64;
            self.bits -= count;
            remaining -= count;
        }
        Ok(value)
    }

    /// The next sample as a quantum (`ImportQuantumPixels`).
    fn quantum(&mut self) -> Result<f32> {
        match (self.depth, self.format) {
            (8, _) => Ok((257.0 * self.take(1)?[0] as f64) as f32),
            (16, Format::Float) => {
                let half = half::f16::from_bits(self.word(2)? as u16);
                Ok((65535.0 * f64::from(half.to_f32())) as f32)
            }
            (16, Format::Unsigned) => Ok(self.word(2)? as f32),
            (32, Format::Float) => {
                let value = f32::from_bits(self.word(4)? as u32) as f64 * 65535.0;
                Ok(value.clamp(-f32::MAX as f64, f32::MAX as f64) as f32)
            }
            (32, Format::Unsigned) => Ok((self.word(4)? as f64 / 65537.0) as f32),
            (64, Format::Float) => Ok((f64::from_bits(self.word(8)?) * 65535.0) as f32),
            (64, Format::Unsigned) => Ok((self.word(8)? as f64 / 281_479_271_743_489.0) as f32),
            (depth, _) => {
                let range = ((1u64 << depth) - 1) as f64;
                let value = self.bits()?;
                if value as f64 > range {
                    return Ok(65535.0);
                }
                Ok((65535.0 * (value as f64 * (1.0 / range))) as f32)
            }
        }
    }

    /// The next colormap index.
    fn index(&mut self) -> Result<usize> {
        match self.depth {
            8 => Ok(self.take(1)?[0] as usize),
            16 => Ok(self.word(2)? as usize),
            32 => Ok(self.word(4)? as usize),
            _ => Ok(self.bits()? as usize),
        }
    }

    /// The start of the next row: packed bits do not run across rows.
    fn end_row(&mut self) {
        self.bits = 0;
    }
}

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let header = parse_header(bytes)?;
    let width = header.number("columns").unwrap_or(0);
    let height = header.number("rows").unwrap_or(0);
    let raw_depth = header.number("depth").unwrap_or(8);
    // What `AcquireImage` starts an image with: DirectClass, sRGB, no
    // compression.
    let class = header.get("class").unwrap_or("DirectClass").to_ascii_lowercase();
    let compression = header.get("compression").unwrap_or("None").to_ascii_lowercase();
    // An unknown colorspace name leaves the image sRGB.
    let named_colorspace = header.get("colorspace").and_then(CacheColorspace::from_option_name);
    let colorspace = named_colorspace.unwrap_or(CacheColorspace::SRgb);
    // Meta channels past IM's 64 channels make the header improper.
    let meta_channels = header.number("number-meta-channels").unwrap_or(0);
    let header_channels = header.number("number-channels").unwrap_or(0);
    let too_many_channels = meta_channels > 54 || header_channels + meta_channels >= 64;
    if header.get("id") != Some("ImageMagick") || class == "undefined" || compression == "undefined" || colorspace == CacheColorspace::Undefined || width == 0 || height == 0 || raw_depth == 0 || raw_depth > 64 || too_many_channels {
        bail!("ImproperImageHeader");
    }
    let gray = matches!(colorspace, CacheColorspace::Gray | CacheColorspace::LinearGray);
    let cmyk = colorspace == CacheColorspace::Cmyk;
    let alpha_trait = header.get("alpha-trait").map(|trait_name| !trait_name.eq_ignore_ascii_case("undefined"));
    let matte = header.get("matte").map(|matte| matte.eq_ignore_ascii_case("true"));
    let alpha = alpha_trait.or(matte).unwrap_or(false);
    let pseudo = class == "pseudoclass";
    if meta_channels != 0 && (pseudo || compression == "rle" || compression == "runlengthencoded") {
        bail!("multispectral MIFF images with a colormap or RLE pixels are not supported");
    }
    let gamma = header.get("gamma").and_then(|text| text.trim().parse::<f64>().ok()).unwrap_or(DEFAULT_GAMMA);
    let little_endian = header.get("endian").is_some_and(|endian| endian.eq_ignore_ascii_case("lsb"));
    let floating = header.get("quantum:format").or(header.get("quantum-format")).is_some_and(|format| format.eq_ignore_ascii_case("floating-point"));
    // `GetImageQuantumDepth`, then a float format's own depths.
    let mut depth = match raw_depth {
        0..=8 => 8,
        9..=16 => 16,
        17..=32 => 32,
        _ => 64,
    };
    if floating {
        depth = depth.max(16);
    }
    let format = if floating { Format::Float } else { Format::Unsigned };

    let mut position = header.data_start;
    // Profiles: a big-endian length and the bytes, in header order.
    if let Some(profiles) = header.get("profile") {
        for _ in profiles.split('\n') {
            let length_bytes = bytes.get(position..position + 4).context("UnableToReadImageData")?;
            let length = u32::from_be_bytes([length_bytes[0], length_bytes[1], length_bytes[2], length_bytes[3]]) as usize;
            if length == 0 || length > bytes.len() {
                break;
            }
            position += 4 + length;
        }
    }

    // The colormap (`AcquireImageColormap`, then the file's colours).
    let mut colormap: Vec<[f32; 3]> = Vec::new();
    if pseudo {
        let colors = header.number("colors").unwrap_or(0);
        let count = if colors != 0 { colors } else { 256 };
        for index in 0..count {
            let level = (index as f64 * (65535.0 / (count.max(2) - 1) as f64)) as f32;
            colormap.push([level, level, level]);
        }
        if colors != 0 {
            let packet = 3 * depth / 8;
            let table = bytes.get(position..position + packet * colors).context("InsufficientImageDataInFile")?;
            let mut entries = Samples::new(table, depth, Format::Unsigned, false);
            for entry in colormap.iter_mut() {
                for channel in entry.iter_mut() {
                    *channel = entries.quantum()?;
                }
            }
            position += packet * colors;
        }
    }

    let mut channels: Vec<PixelChannel> = if pseudo && gray {
        vec![PixelChannel::Gray, PixelChannel::Index]
    } else if pseudo {
        vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Index]
    } else if gray {
        vec![PixelChannel::Gray]
    } else if cmyk {
        vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Black]
    } else {
        vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue]
    };
    if alpha {
        channels.insert(channels.len() - usize::from(pseudo), PixelChannel::Alpha);
    }
    channels.extend(std::iter::repeat_n(PixelChannel::Meta, meta_channels));
    // Samples a pixel holds in the file (`packet_size` / depth).
    let samples_per_pixel = if pseudo || gray { 1 } else { 3 } + usize::from(alpha) + usize::from(cmyk) + meta_channels;
    let packet_size = samples_per_pixel * depth / 8;
    let row_bytes = (samples_per_pixel * depth * width).div_ceil(8);
    // The most compressed data IM reads in one go: what one row can
    // compress to under any of its codecs (`packet_size` here counts the
    // header's depth, so it is zero below 8 bits).
    let header_packet_size = if meta_channels != 0 {
        samples_per_pixel * (raw_depth / 8)
    } else {
        (if pseudo || gray { 1 } else { 3 } + usize::from(alpha) + usize::from(cmyk)) * (raw_depth / 8)
    };
    let row_extent = header_packet_size * width;
    let compress_extent = bzip_max_extent(row_extent).max(lzma_max_extent(row_extent)).max(zip_max_extent(row_extent));
    let version_one = header.get("version").and_then(|text| text.trim().parse::<f64>().ok()).unwrap_or(0.0) != 0.0;

    let data_bytes = &bytes[position.min(bytes.len())..];
    let mut data = Vec::with_capacity(width * height * channels.len());
    match compression.as_str() {
        "none" => {
            let pixels = data_bytes.get(..row_bytes * height).context("UnableToReadImageData")?;
            import_rows(pixels, row_bytes, width, height, depth, format, little_endian, pseudo, gray, cmyk, alpha, meta_channels, &colormap, &mut data)?;
        }
        "bzip" => {
            let stream = readable_input(data_bytes, version_one, bzip_max_extent(row_extent), compress_extent);
            let mut unpacked = vec![0u8; row_bytes * height];
            let mut decoder = bzip2::read::BzDecoder::new(stream.as_slice());
            decoder.read_exact(&mut unpacked).context("UnableToReadImageData")?;
            import_rows(&unpacked, row_bytes, width, height, depth, format, little_endian, pseudo, gray, cmyk, alpha, meta_channels, &colormap, &mut data)?;
        }
        "lzma" => {
            // `lzma_auto_decoder`: an xz stream, or the older LZMA format.
            let stream = readable_input(data_bytes, true, 0, compress_extent);
            let unpacked = super::xz::decompress_auto(&stream).context("UnableToReadImageData")?;
            let pixels = unpacked.get(..row_bytes * height).context("UnableToReadImageData")?;
            import_rows(pixels, row_bytes, width, height, depth, format, little_endian, pseudo, gray, cmyk, alpha, meta_channels, &colormap, &mut data)?;
        }
        "zip" | "lzw" => {
            let stream = readable_input(data_bytes, version_one, zip_max_extent(row_extent), compress_extent);
            let mut inflated = vec![0u8; row_bytes * height];
            let mut decoder = flate2::read::ZlibDecoder::new(stream.as_slice());
            decoder.read_exact(&mut inflated).context("UnableToReadImageData")?;
            import_rows(&inflated, row_bytes, width, height, depth, format, little_endian, pseudo, gray, cmyk, alpha, meta_channels, &colormap, &mut data)?;
        }
        "rle" | "runlengthencoded" => {
            read_rle(data_bytes, packet_size, width, height, depth, pseudo, gray, cmyk, alpha, &colormap, &mut data)?;
        }
        other => bail!("MIFF pixels compressed with {other} are not supported"),
    }
    // `SetQuantumImageType` with the quantum type the pixels were read as.
    let image_type = if pseudo {
        ImageType::Palette
    } else if gray && raw_depth == 1 {
        ImageType::Bilevel
    } else if gray {
        ImageType::Grayscale
    } else if cmyk || meta_channels != 0 {
        ImageType::ColorSeparation
    } else {
        ImageType::TrueColor
    };
    let mut cache = PixelCache::new(width, height, channels, data, colorspace, gamma);
    cache.image_type = image_type;
    // Keywords the reader doesn't know become properties; `gamma` is one
    // it knows.
    cache.transform_settings = TransformSettings::from_properties(|name| if name == "gamma" { None } else { header.get(name).map(str::to_string) });
    cache.colormap = colormap.iter().map(|entry| [entry[0] as f64, entry[1] as f64, entry[2] as f64, 65535.0]).collect();
    Ok(cache)
}

/// `BZipMaxExtent`, `LZMAMaxExtent` and `ZipMaxExtent`: the most a row of
/// `x` bytes can compress to.
fn bzip_max_extent(x: usize) -> usize {
    x + x / 100 + 600
}

fn lzma_max_extent(x: usize) -> usize {
    x + x / 3 + 128
}

fn zip_max_extent(x: usize) -> usize {
    x + ((x + 7) >> 3) + ((x + 63) >> 6) + 11
}

/// The compressed pixel data IM gets to read, joined: a version-1 file's
/// chunks (each a big-endian length, then its bytes), or a version-0
/// file's blocks of `block` bytes, up to the first chunk longer than
/// `compress_extent` or cut short by the end of the file. IM reads a chunk
/// only when it needs more input, so such a chunk fails the read only if
/// the pixels need data past it — which the decompressor finds by running
/// dry.
fn readable_input(bytes: &[u8], version_one: bool, block: usize, compress_extent: usize) -> Vec<u8> {
    let mut joined = Vec::new();
    let mut position = 0;
    loop {
        let length = if version_one {
            let Some(length_bytes) = bytes.get(position..position + 4) else {
                break;
            };
            position += 4;
            u32::from_be_bytes([length_bytes[0], length_bytes[1], length_bytes[2], length_bytes[3]]) as usize
        } else {
            block
        };
        if length > compress_extent || length == 0 {
            break;
        }
        let Some(chunk) = bytes.get(position..position + length) else {
            break;
        };
        joined.extend_from_slice(chunk);
        position += length;
    }
    joined
}

/// `ImportQuantumPixels` for each row: grey, RGB or an index, then alpha
/// and the meta channels; an index brings its colormap colour
/// (`SetPixelViaPixelInfo`).
#[allow(clippy::too_many_arguments)]
fn import_rows(pixels: &[u8], row_bytes: usize, width: usize, height: usize, depth: usize, format: Format, little_endian: bool, pseudo: bool, gray: bool, cmyk: bool, alpha: bool, meta_channels: usize, colormap: &[[f32; 3]], data: &mut Vec<f32>) -> Result<()> {
    for y in 0..height {
        let row = &pixels[y * row_bytes..(y + 1) * row_bytes];
        let mut samples = Samples::new(row, depth, format, little_endian);
        for _ in 0..width {
            if pseudo {
                let index = samples.index()?;
                let (colour, index) = match colormap.get(index) {
                    Some(colour) => (*colour, index),
                    None => (colormap.first().copied().unwrap_or_default(), 0),
                };
                if gray {
                    data.push(colour[0]);
                } else {
                    data.extend(colour);
                }
                if alpha {
                    data.push(samples.quantum()?);
                }
                data.push(index as f32);
                continue;
            }
            let colours = if gray { 1 } else { 3 } + usize::from(cmyk);
            let samples_per_pixel = colours + usize::from(alpha) + meta_channels;
            for _ in 0..samples_per_pixel {
                data.push(samples.quantum()?);
            }
        }
        samples.end_row();
    }
    Ok(())
}

/// RLE pixels (`PushRunlengthPacket`): a packet then how many more times
/// it repeats. 16- and 32-bit samples are shifted down to 16 bits; an
/// index sets only the index channel (its colours stay black, as IM leaves
/// them).
#[allow(clippy::too_many_arguments)]
fn read_rle(bytes: &[u8], packet_size: usize, width: usize, height: usize, depth: usize, pseudo: bool, gray: bool, cmyk: bool, alpha: bool, colormap: &[[f32; 3]], data: &mut Vec<f32>) -> Result<()> {
    let read_sample = |packet: &[u8], offset: &mut usize| -> f32 {
        let value = match depth {
            8 => {
                let byte = packet[*offset];
                *offset += 1;
                return (257.0 * byte as f64) as f32;
            }
            16 => {
                let value = u16::from_be_bytes([packet[*offset], packet[*offset + 1]]) as u64;
                *offset += 2;
                value
            }
            _ => {
                let value = u32::from_be_bytes([packet[*offset], packet[*offset + 1], packet[*offset + 2], packet[*offset + 3]]) as u64;
                *offset += 4;
                value >> (depth - 16)
            }
        };
        value as f32
    };
    let mut position = 0;
    let mut remaining = 0usize;
    let mut pixel: Vec<f32> = Vec::new();
    for _ in 0..height {
        for _ in 0..width {
            if remaining == 0 {
                let packet = bytes.get(position..position + packet_size + 1).context("UnableToReadImageData")?;
                position += packet_size + 1;
                let mut offset = 0;
                pixel.clear();
                if pseudo {
                    let index = match depth {
                        8 => packet[0] as usize,
                        16 => u16::from_be_bytes([packet[0], packet[1]]) as usize,
                        _ => u32::from_be_bytes([packet[0], packet[1], packet[2], packet[3]]) as usize,
                    };
                    offset += depth / 8;
                    let index = if index < colormap.len() { index } else { 0 };
                    let colours = if gray { 1 } else { 3 };
                    pixel.extend(std::iter::repeat_n(0.0, colours));
                    if alpha {
                        pixel.push(read_sample(packet, &mut offset));
                    }
                    pixel.push(index as f32);
                } else {
                    let first = read_sample(packet, &mut offset);
                    if gray {
                        pixel.push(first);
                    } else {
                        pixel.push(first);
                        pixel.push(read_sample(packet, &mut offset));
                        pixel.push(read_sample(packet, &mut offset));
                    }
                    if cmyk {
                        pixel.push(read_sample(packet, &mut offset));
                    }
                    if alpha {
                        pixel.push(read_sample(packet, &mut offset));
                    }
                }
                remaining = packet[packet_size] as usize + 1;
            }
            remaining -= 1;
            data.extend_from_slice(&pixel);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_read_like_imagemagick() {
        let text = b"id=ImageMagick  version=1.0\nclass=DirectClass colors=0 alpha-trait=Undefined\n{a comment}\nlabel={two words}\ncolumns=2  rows=1  depth=8\ncolorspace=sRGB\ngamma=0.454545\n\x0c\n:\x1a\x01\x02\x03\x04\x05\x06";
        let image = decode(text).unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.channels, [PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue]);
        assert_eq!(image.data, [257.0, 514.0, 771.0, 1028.0, 1285.0, 1542.0]);
        assert!((image.gamma - 0.454545).abs() < 1e-9);
    }

    #[test]
    fn run_lengths_repeat_packets() {
        let text = b"id=ImageMagick class=DirectClass compression=RLE columns=3 rows=1 depth=16 colorspace=Gray\n:\x1a\x01\x00\x02\x00\x00\x00";
        let image = decode(text).unwrap();
        assert_eq!(image.data, vec![256.0, 256.0, 256.0]);
    }
}
