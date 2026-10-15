//! GIMP XCF images as ImageMagick reads them (`coders/xcf.c`): 8-bit RGB or
//! grey documents, one image a layer, not composited. IM lists the layers
//! bottom first, so the image read is the bottom layer: its tiles (raw or
//! RLE) copied onto a layer that starts clear, or white at the layer's
//! opacity when that isn't full. A document without layers fails.
//!
//! IM's quirks are kept: raw RGB tiles are read four bytes a pixel and an
//! alpha byte of 255 reads as clear, a raw tile's length runs to the next
//! tile (24576 bytes for the last), and indexed documents fail.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use crate::pixel_cache::{char_quantum, CacheColorspace, ImageType, PixelCache, PixelChannel, DEFAULT_GAMMA};

const GIMP_RGB: u32 = 0;
const GIMP_GRAY: u32 = 1;
const GIMP_INDEXED: u32 = 2;
const TILE: usize = 64;
const COMPRESS_NONE: u8 = 0;
const COMPRESS_RLE: u8 = 1;
const COMPRESS_ZLIB: u8 = 2;
const COMPRESS_FRACTAL: u8 = 3;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

struct Document {
    version: u64,
    gray: bool,
    compression: u8,
    file_size: u64,
    bytes_per_pixel: usize,
}

impl Document {
    /// `GetXCFOffset`: 64-bit from version 4.
    fn offset(&self, blob: &mut Blob) -> u64 {
        if self.version >= 4 {
            let high = blob.msb_u32() as u64;
            let low = blob.msb_u32() as u64;
            (high << 32) | low
        } else {
            blob.msb_u32() as u64
        }
    }
}

/// A layer's pixels: grey or RGB, then alpha.
struct Layer {
    width: usize,
    height: usize,
    data: Vec<f32>,
}

/// `DiscardBlobBytes`.
fn discard(blob: &mut Blob, count: u64) -> bool {
    let wanted = usize::try_from(count).unwrap_or(usize::MAX);
    blob.read(wanted).len() == wanted
}

/// `ReadBlobStringWithLongSize`.
fn skip_string(blob: &mut Blob) -> Result<()> {
    let length = blob.msb_u32() as u64;
    let kept = length.min(1023);
    for _ in 0..kept {
        if blob.byte().is_none() {
            return Ok(());
        }
    }
    let target = blob.tell() as i64 + (length - kept) as i64;
    blob.seek(target);
    blob.eof = false;
    Ok(())
}

/// Skips an unknown property 16 bytes at a time.
fn skip_property(blob: &mut Blob, mut size: u64) -> Result<()> {
    while size > 0 && !blob.eof {
        let amount = size.min(16) as usize;
        let read = blob.read(amount).len();
        if read == 0 {
            bail!("CorruptImage");
        }
        size -= (read as u64).min(16);
    }
    Ok(())
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let magick = blob.read(14);
    if magick.len() != 14 || !magick[..8].eq_ignore_ascii_case(b"gimp xcf") {
        bail!("ImproperImageHeader");
    }
    let version_text = &magick[10..];
    let digits: Vec<u8> = version_text.iter().copied().take_while(u8::is_ascii_digit).collect();
    let version = String::from_utf8_lossy(&digits).parse::<u64>().unwrap_or(0);
    let width = blob.msb_u32() as usize;
    let height = blob.msb_u32() as usize;
    if width > 262144 || height > 262144 {
        bail!("ImproperImageHeader");
    }
    let image_type = blob.msb_u32();
    if version >= 4 {
        let mut precision = blob.msb_u32();
        if precision == 0 && version == 4 {
            precision = 150;
        }
        if precision == 100 && version < 7 {
            precision = 150;
        }
        if precision != 150 {
            bail!("DataStorageTypeIsNotSupported");
        }
    }
    if width == 0 || height == 0 {
        bail!("NegativeOrZeroImageSize");
    }
    if image_type == GIMP_INDEXED {
        bail!("ColormapTypeNotSupported");
    }
    if image_type != GIMP_RGB && image_type != GIMP_GRAY {
        bail!("ImproperImageHeader");
    }
    let mut document = Document { version, gray: image_type == GIMP_GRAY, compression: 0, file_size: bytes.len() as u64, bytes_per_pixel: 0 };

    // Image properties.
    let mut found_end = false;
    while !found_end && !blob.eof {
        let kind = blob.msb_u32();
        let size = blob.msb_u32() as u64;
        match kind {
            0 => found_end = true,
            1 => {
                let colors = blob.msb_u32() as u64;
                if !discard(&mut blob, 3 * colors) {
                    bail!("UnexpectedEndOfFile");
                }
            }
            17 => {
                document.compression = blob.byte().unwrap_or(0xff);
                if !matches!(document.compression, COMPRESS_NONE | COMPRESS_RLE | COMPRESS_ZLIB | COMPRESS_FRACTAL) {
                    bail!("UnrecognizedImageCompression");
                }
            }
            18 | 21 | 23 => {
                if !discard(&mut blob, size) {
                    bail!("UnexpectedEndOfFile");
                }
            }
            19 => {
                blob.msb_u32();
                blob.msb_u32();
            }
            20 | 22 => {
                blob.msb_u32();
            }
            24 => {
                blob.msb_u32();
                blob.msb_u32();
                for _ in 0..5 {
                    skip_string(&mut blob)?;
                }
            }
            _ => skip_property(&mut blob, size)?,
        }
    }
    if !found_end {
        bail!("ImproperImageHeader");
    }

    // Count the layers, then read them; only the bottom one (the first
    // image IM lists) is read in full.
    let layer_list = blob.tell();
    let mut number_layers = 0usize;
    loop {
        let offset = document.offset(&mut blob);
        if blob.eof {
            bail!("UnexpectedEndOfFile");
        }
        if offset == 0 {
            break;
        }
        number_layers += 1;
    }
    // No layers: the layer table's allocation fails.
    if number_layers == 0 {
        bail!("MemoryAllocationFailed");
    }
    blob.seek(layer_list as i64);
    let mut bottom: Option<Layer> = None;
    for index in 0..number_layers {
        let offset = document.offset(&mut blob);
        if offset == 0 {
            break;
        }
        let saved = blob.tell();
        blob.seek(offset as i64);
        blob.eof = false;
        let full = index == number_layers - 1;
        let layer = read_layer(&mut blob, &mut document, full).context("NotEnoughPixelData")?;
        if full {
            bottom = layer;
        }
        blob.seek(saved as i64);
        blob.eof = false;
    }
    let gray = document.gray;
    let mut channels = if gray { vec![PixelChannel::Gray] } else { vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue] };
    channels.push(PixelChannel::Alpha);
    let colorspace = if gray { CacheColorspace::Gray } else { CacheColorspace::SRgb };
    let layer = bottom.context("NotEnoughPixelData")?;
    let mut cache = PixelCache::new(layer.width, layer.height, channels, layer.data, colorspace, DEFAULT_GAMMA);
    if gray {
        cache.image_type = ImageType::Grayscale;
    }
    Ok(cache)
}

/// `ReadOneLayer`: the layer's header and properties, and (when `full`)
/// its pixels.
fn read_layer(blob: &mut Blob, document: &mut Document, full: bool) -> Result<Option<Layer>> {
    let width = blob.msb_u32() as usize;
    let height = blob.msb_u32() as usize;
    let _kind = blob.msb_u32();
    skip_string(blob)?;
    if blob.eof {
        bail!("InsufficientImageDataInFile");
    }
    if width == 0 || height == 0 {
        bail!("ImproperImageHeader");
    }
    let mut opacity = 0u32;
    let mut found_end = false;
    while !found_end && !blob.eof {
        let kind = blob.msb_u32();
        let size = blob.msb_u32() as u64;
        match kind {
            0 => found_end = true,
            2 => {}
            5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 20 => {
                let value = blob.msb_u32();
                if kind == 6 {
                    opacity = value;
                }
            }
            15 => {
                blob.msb_u32();
                blob.msb_u32();
            }
            21 => {
                if !discard(blob, size) {
                    bail!("UnexpectedEndOfFile");
                }
            }
            _ => skip_property(blob, size)?,
        }
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    if !found_end {
        bail!("NotEnoughPixelData");
    }
    if !full {
        return Ok(None);
    }
    let stride = if document.gray { 2 } else { 4 };
    // A clear layer, or white at the layer's opacity.
    let mut data = vec![0.0f32; width * height * stride];
    let alpha = char_quantum(opacity as u8);
    if opacity != 255 {
        for pixel in data.chunks_exact_mut(stride) {
            pixel[..stride - 1].fill(65535.0);
            pixel[stride - 1] = alpha;
        }
    }
    let background = |pixel: &mut [f32]| {
        pixel[..stride - 1].fill(65535.0);
        pixel[stride - 1] = if opacity != 255 { alpha } else { 65535.0 };
    };
    let mut layer = Layer { width, height, data };

    let hierarchy = document.offset(blob);
    let _mask = document.offset(blob);
    blob.seek(hierarchy as i64);
    blob.eof = false;
    // `load_hierarchy`.
    blob.msb_u32();
    blob.msb_u32();
    document.bytes_per_pixel = blob.msb_u32() as usize;
    let level = document.offset(blob);
    if level >= blob.size() {
        bail!("InsufficientImageDataInFile");
    }
    while blob.msb_u32() != 0 {
        if blob.eof {
            break;
        }
    }
    blob.seek(level as i64);
    blob.eof = false;
    // `load_level`.
    let level_width = blob.msb_u32() as usize;
    let level_height = blob.msb_u32() as usize;
    let mut offset = document.offset(blob);
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    if offset == 0 {
        return Ok(Some(layer));
    }
    let tile_rows = level_height.div_ceil(TILE);
    let tile_columns = level_width.div_ceil(TILE);
    let mut left = 0usize;
    let mut top = 0usize;
    for _ in 0..tile_rows * tile_columns {
        if offset == 0 {
            bail!("NotEnoughTiles");
        }
        let saved = blob.tell();
        let mut next = document.offset(blob);
        if next >= document.file_size {
            bail!("InsufficientImageDataInFile");
        }
        if next == 0 {
            next = (offset as f64 + (TILE * TILE * 4) as f64 * 1.5) as u64;
        }
        if offset > next {
            bail!("InsufficientImageDataInFile");
        }
        blob.seek(offset as i64);
        blob.eof = false;
        let mut tile_width = if left == tile_columns - 1 { level_width % TILE } else { TILE };
        if tile_width == 0 {
            tile_width = TILE;
        }
        let mut tile_height = if top == tile_rows - 1 { level_height % TILE } else { TILE };
        if tile_height == 0 {
            tile_height = TILE;
        }
        let mut tile = vec![0.0f32; tile_width * tile_height * stride];
        for pixel in tile.chunks_exact_mut(stride) {
            background(pixel);
        }
        let length = (next - offset) as usize;
        let loaded = match document.compression {
            COMPRESS_NONE => load_tile(blob, document, &mut tile, stride, opacity, length)?,
            COMPRESS_RLE => load_tile_rle(blob, document, &mut tile, stride, opacity, length),
            COMPRESS_ZLIB => bail!("ZipCompressNotSupported"),
            _ => bail!("FractalCompressNotSupported"),
        };
        if !loaded {
            bail!("NotEnoughPixelData");
        }
        // `CompositeImage(Copy)`, clipped to the layer.
        for y in 0..tile_height {
            let target_y = top * TILE + y;
            if target_y >= layer.height {
                break;
            }
            for x in 0..tile_width {
                let target_x = left * TILE + x;
                if target_x >= layer.width {
                    break;
                }
                let source = (y * tile_width + x) * stride;
                let target = (target_y * layer.width + target_x) * stride;
                layer.data[target..target + stride].copy_from_slice(&tile[source..source + stride]);
            }
        }
        left += 1;
        if left >= tile_columns {
            left = 0;
            top += 1;
        }
        blob.seek(saved as i64);
        blob.eof = false;
        offset = document.offset(blob);
    }
    if offset != 0 {
        bail!("CorruptImage");
    }
    Ok(Some(layer))
}

/// `load_tile`: grey bytes, or RGB read as four bytes a pixel (an alpha
/// byte of 255 clear, any other the layer's opacity).
fn load_tile(blob: &mut Blob, document: &Document, tile: &mut [f32], stride: usize, opacity: u32, length: usize) -> Result<bool> {
    let pixels = tile.len() / stride;
    let extent = if document.gray { pixels } else { pixels * 4 };
    if extent > length {
        bail!("NotEnoughPixelData");
    }
    let data = blob.read(length);
    if data.len() != length {
        bail!("NotEnoughPixelData");
    }
    let alpha = char_quantum(opacity as u8);
    for (index, pixel) in tile.chunks_exact_mut(stride).enumerate() {
        if document.gray {
            pixel[0] = char_quantum(data[index]);
            pixel[1] = alpha;
        } else {
            let source = &data[index * 4..index * 4 + 4];
            pixel[0] = char_quantum(source[0]);
            pixel[1] = char_quantum(source[1]);
            pixel[2] = char_quantum(source[2]);
            pixel[3] = if source[3] == 255 { 0.0 } else { alpha };
        }
    }
    Ok(true)
}

/// `load_tile_rle`: each channel's runs over the whole tile; false for a
/// bad stream.
fn load_tile_rle(blob: &mut Blob, document: &Document, tile: &mut [f32], stride: usize, opacity: u32, length: usize) -> bool {
    let mut data = vec![0u8; length];
    let read = blob.read(length);
    data[..read.len()].copy_from_slice(read);
    let count = read.len();
    // `xcfdatalimit`: the last byte read.
    let limit = count as isize - 1;
    let alpha = char_quantum(opacity as u8);
    let pixels = tile.len() / stride;
    let mut p = 0isize;
    for channel in 0..document.bytes_per_pixel {
        let mut q = 0usize;
        let mut size = pixels as i64;
        let set = |q: &mut usize, value: f32, tile: &mut [f32]| {
            if *q < pixels {
                let pixel = &mut tile[*q * stride..(*q + 1) * stride];
                match channel {
                    0 => {
                        pixel[..stride - 1].fill(value);
                        pixel[stride - 1] = alpha;
                    }
                    1 => {
                        if document.gray {
                            pixel[stride - 1] = value;
                        } else {
                            pixel[1] = value;
                        }
                    }
                    2 => pixel[2.min(stride - 1)] = value,
                    3 => pixel[stride - 1] = value,
                    _ => {}
                }
            }
            *q += 1;
        };
        while size > 0 {
            if p > limit {
                return false;
            }
            let mut run = data[p as usize] as usize;
            p += 1;
            if run >= 128 {
                run = 255 - (run - 1);
                if run == 128 {
                    if p >= limit {
                        return false;
                    }
                    run = (((data[p as usize] as usize) << 8) + data[p as usize + 1] as usize) & 0xffff;
                    p += 2;
                }
                size -= run as i64;
                if size < 0 || run < 1 || run > length {
                    return false;
                }
                if p + run as isize - 1 > limit {
                    return false;
                }
                for _ in 0..run {
                    let value = char_quantum(data[p as usize]);
                    p += 1;
                    set(&mut q, value, tile);
                }
            } else {
                run += 1;
                if run == 128 {
                    if p >= limit {
                        return false;
                    }
                    run = (((data[p as usize] as usize) << 8) + data[p as usize + 1] as usize) & 0xffff;
                    p += 2;
                }
                size -= run as i64;
                if size < 0 {
                    return false;
                }
                if p > limit {
                    return false;
                }
                let value = char_quantum(data[p as usize]);
                p += 1;
                for _ in 0..run {
                    set(&mut q, value, tile);
                }
            }
        }
    }
    true
}
