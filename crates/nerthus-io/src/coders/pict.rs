//! Apple Macintosh QuickDraw PICT pictures as ImageMagick reads them
//! (`coders/pict.c`): a black canvas the size of the picture frame onto
//! which the colour pixmaps (`PackBitsRect` with a colour table,
//! `DirectBitsRect`, and their region forms) are copied. Drawing opcodes
//! are skipped, plain one-bit bitmaps are decoded but never drawn, and an
//! embedded JPEG is copied at its frame's left and *right* (IM's mix-up),
//! after which no more pixmaps are drawn.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::colormap;
use crate::pixel_cache::{char_quantum, CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};
use nerthus_image::colorspace_transform::rgb_to_cmyk;
use nerthus_image::image::Image;
use nerthus_image::resize::{resize_with_filter, Filter};

const QUANTUM_RANGE: f64 = 65535.0;

/// The bytes each opcode up to `LongComment` carries when IM skips it; -1
/// means a word giving no length is read (`codes[].length`).
const CODE_LENGTHS: [i8; 0xa2] = [
    0, 0, 8, 2, 1, 2, 4, 4, 2, 8, 8, 4, 4, 2, 4, 4, // 0x00
    8, 1, 0, 0, 0, 2, 2, 0, 0, 0, 6, 6, 0, 6, 0, 6, // 0x10
    8, 4, 6, 2, -1, -1, -1, -1, 0, 0, 0, 0, -1, -1, -1, -1, // 0x20
    8, 8, 8, 8, 8, 8, 8, 8, 0, 0, 0, 0, 0, 0, 0, 0, // 0x30
    8, 8, 8, 8, 8, 8, 8, 8, 0, 0, 0, 0, 0, 0, 0, 0, // 0x40
    8, 8, 8, 8, 8, 8, 8, 8, 0, 0, 0, 0, 0, 0, 0, 0, // 0x50
    12, 12, 12, 12, 12, 12, 12, 12, 4, 4, 4, 4, 4, 4, 4, 4, // 0x60
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 0x70
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 0x80
    0, 0, -1, -1, -1, -1, -1, -1, 0, 0, 0, 0, -1, -1, -1, -1, // 0x90
    2, 0, // 0xa0
];

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

#[derive(Clone, Copy, Default)]
struct Rectangle {
    top: i16,
    left: i16,
    bottom: i16,
    right: i16,
}

impl Rectangle {
    fn width(&self) -> usize {
        (self.right as i32 - self.left as i32) as usize
    }

    fn height(&self) -> usize {
        (self.bottom as i32 - self.top as i32) as usize
    }
}

/// The `PixMap` fields IM uses; zero until one is read.
#[derive(Clone, Copy, Default)]
struct Pixmap {
    bits_per_pixel: i16,
    component_count: i16,
}

/// `ReadRectangle`: none for one cut short, with a negative side, or
/// turned inside out.
fn read_rectangle(blob: &mut Blob) -> Result<Rectangle> {
    let top = blob.msb_u16() as i16;
    let left = blob.msb_u16() as i16;
    let bottom = blob.msb_u16() as i16;
    let right = blob.msb_u16() as i16;
    let negative = top < 0 || left < 0 || bottom < 0 || right < 0;
    if blob.eof || negative || bottom < top || right < left {
        bail!("ImproperImageHeader");
    }
    Ok(Rectangle { top, left, bottom, right })
}

/// `ReadPixmap`.
fn read_pixmap(blob: &mut Blob) -> Result<Pixmap> {
    blob.read(2 + 2 + 4 + 2 + 2 + 2 + 2 + 2);
    let bits_per_pixel = blob.msb_u16() as i16;
    let component_count = blob.msb_u16() as i16;
    let component_size = blob.msb_u16() as i16;
    blob.read(4 + 4 + 4);
    let bad_depth = bits_per_pixel <= 0 || bits_per_pixel > 32;
    let bad_components = component_count <= 0 || component_count > 4 || component_size <= 0;
    if blob.eof || bad_depth || bad_components {
        bail!("ImproperImageHeader");
    }
    Ok(Pixmap { bits_per_pixel, component_count })
}

/// Reads and drops up to `count` bytes, stopping at the end of the file.
fn skip(blob: &mut Blob, count: u64) {
    for _ in 0..count {
        if blob.byte().is_none() {
            break;
        }
    }
}

/// A length word, which may not exceed the file.
fn read_length(blob: &mut Blob) -> Result<u64> {
    let length = blob.msb_u16() as u64;
    if length > blob.size() {
        bail!("InsufficientImageDataInFile");
    }
    Ok(length)
}

/// `UnpackScanline`: the bytes `count` packed bytes expand to (one per
/// pixel below eight bits); 8-, 16- and 32-bit pixels are their own bytes
/// and other depths leave the buffer as it was.
fn unpack_scanline<'a>(packed: &'a [u8], bits_per_pixel: u32, buffer: &'a mut [u8; 2048], count: &mut usize) -> &'a [u8] {
    match bits_per_pixel {
        8 | 16 | 32 => return &packed[..*count],
        4 => {
            for (i, &byte) in packed[..*count].iter().enumerate() {
                buffer[2 * i] = byte >> 4;
                buffer[2 * i + 1] = byte & 15;
            }
            *count *= 2;
        }
        2 => {
            for (i, &byte) in packed[..*count].iter().enumerate() {
                buffer[4 * i] = (byte >> 6) & 3;
                buffer[4 * i + 1] = (byte >> 4) & 3;
                buffer[4 * i + 2] = (byte >> 2) & 3;
                buffer[4 * i + 3] = byte & 3;
            }
            *count *= 4;
        }
        1 => {
            for (i, &byte) in packed[..*count].iter().enumerate() {
                for bit in 0..8 {
                    buffer[8 * i + bit] = (byte >> (7 - bit)) & 1;
                }
            }
            *count *= 8;
        }
        _ => {}
    }
    &buffer[..*count]
}

/// The tile `DecodeImage` unpacks into.
struct TileLayout {
    columns: usize,
    rows: usize,
    direct: bool,
    alpha: bool,
    /// `GetPixelChannels(tile_image)`.
    channels: usize,
}

/// `DecodeImage`: the tile's PackBits rows, unpacked to a byte (or a
/// sample) a pixel; `None` when a row is cut short or too long. Rows of
/// fewer than eight bytes are stored, not packed, and land `channels`
/// times further apart than they're read back (as in IM).
fn decode_image(blob: &mut Blob, tile: &TileLayout, bytes_per_line: usize, bits_per_pixel: u32) -> Option<Vec<u8>> {
    let mut bytes_per_line = bytes_per_line;
    if bits_per_pixel <= 8 {
        bytes_per_line &= 0x7fff;
    }
    let mut width = tile.columns;
    let mut bytes_per_pixel = 1;
    if bits_per_pixel == 16 {
        bytes_per_pixel = 2;
        width *= 2;
    } else if bits_per_pixel == 32 {
        width *= if tile.alpha { 4 } else { 3 };
    }
    if bytes_per_line == 0 {
        bytes_per_line = width;
    }
    let row_bytes = if tile.direct { (4 * tile.columns) | 0x8000 } else { tile.columns | 0x8000 };
    let extent = row_bytes * tile.rows;
    let mut pixels = vec![0u8; extent];
    let mut scanline = vec![0u8; 2 * row_bytes];
    let mut unpack_buffer = [0u8; 2048];
    if bytes_per_line < 8 {
        for y in 0..tile.rows {
            let q = y * width * tile.channels;
            let mut number_pixels = bytes_per_line;
            let read = blob.read(number_pixels);
            if read.len() != number_pixels {
                return None;
            }
            scanline[..number_pixels].copy_from_slice(read);
            let unpacked = unpack_scanline(&scanline, bits_per_pixel, &mut unpack_buffer, &mut number_pixels);
            if q + number_pixels > extent {
                return None;
            }
            pixels[q..q + number_pixels].copy_from_slice(unpacked);
        }
        return Some(pixels);
    }
    for y in 0..tile.rows {
        let mut q = y * width;
        let scanline_length = if bytes_per_line > 250 {
            blob.msb_u16() as usize
        } else {
            blob.byte().map_or(usize::MAX, usize::from)
        };
        if scanline_length >= row_bytes || scanline_length == 0 {
            return None;
        }
        let read = blob.read(scanline_length);
        if read.len() != scanline_length {
            return None;
        }
        scanline[..scanline_length].copy_from_slice(read);
        let mut j = 0;
        while j < scanline_length {
            let flag = scanline[j];
            if flag & 0x80 == 0 {
                let length = flag as usize + 1;
                let mut number_pixels = length * bytes_per_pixel;
                let unpacked = unpack_scanline(&scanline[j + 1..], bits_per_pixel, &mut unpack_buffer, &mut number_pixels);
                if q + number_pixels <= extent {
                    pixels[q..q + number_pixels].copy_from_slice(unpacked);
                }
                q += number_pixels;
                j += length * bytes_per_pixel + 1;
            } else {
                let length = (flag ^ 0xff) as usize + 2;
                let mut number_pixels = bytes_per_pixel;
                let unpacked = unpack_scanline(&scanline[j + 1..], bits_per_pixel, &mut unpack_buffer, &mut number_pixels);
                for _ in 0..length {
                    if q + number_pixels <= extent {
                        pixels[q..q + number_pixels].copy_from_slice(unpacked);
                    }
                    q += number_pixels;
                }
                j += bytes_per_pixel + 1;
            }
        }
    }
    Some(pixels)
}

/// The picture IM draws into: its pixels, interleaved, in `channels`.
struct Canvas {
    columns: usize,
    rows: usize,
    colorspace: CacheColorspace,
    channels: Vec<PixelChannel>,
    data: Vec<f32>,
}

impl Canvas {
    /// `SetImageExtent` then `ResetImagePixels`: all black.
    fn reset(&mut self, columns: usize, rows: usize) -> Result<()> {
        if columns == 0 || rows == 0 {
            bail!("NegativeOrZeroImageSize");
        }
        self.columns = columns;
        self.rows = rows;
        self.data = vec![0.0; columns * rows * self.channels.len()];
        Ok(())
    }

    /// `SetImageExtent` to a larger size: the pixels keep their places.
    fn grow(&mut self, columns: usize, rows: usize) {
        if columns == self.columns && rows == self.rows {
            return;
        }
        let stride = self.channels.len();
        let mut data = vec![0.0; columns * rows * stride];
        let kept_columns = self.columns.min(columns);
        for y in 0..self.rows.min(rows) {
            let source = &self.data[y * self.columns * stride..][..kept_columns * stride];
            data[y * columns * stride..][..kept_columns * stride].copy_from_slice(source);
        }
        self.columns = columns;
        self.rows = rows;
        self.data = data;
    }

    /// `TransformImageColorspace` from sRGB to the grey or CMYK of an
    /// embedded JPEG.
    fn transform(&mut self, colorspace: CacheColorspace) -> Result<()> {
        if colorspace == self.colorspace {
            return Ok(());
        }
        if self.colorspace != CacheColorspace::SRgb {
            bail!("PICT images whose embedded JPEGs change colour space twice are not supported");
        }
        let pixels = self.data.as_chunks::<3>().0.iter();
        match colorspace {
            CacheColorspace::Gray => {
                let gray: Vec<f32> = pixels
                    .map(|pixel| (0.212656 * pixel[0] as f64 + 0.715158 * pixel[1] as f64 + 0.072186 * pixel[2] as f64) as f32)
                    .collect();
                self.data = gray;
                self.channels = vec![PixelChannel::Gray];
            }
            CacheColorspace::Cmyk => {
                let mut cmyk = Vec::with_capacity(self.columns * self.rows * 4);
                for pixel in pixels {
                    let inks = rgb_to_cmyk(pixel[0] as f64, pixel[1] as f64, pixel[2] as f64);
                    cmyk.extend(inks.iter().map(|&ink| ink as f32));
                }
                self.data = cmyk;
                self.channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Black];
            }
            _ => bail!("PICT images with embedded JPEGs in this colour space are not supported"),
        }
        self.colorspace = colorspace;
        Ok(())
    }

    /// `CompositeImage(CopyCompositeOp)` with clipping: the channels both
    /// share, where the source overlaps the canvas.
    fn copy(&mut self, source: &PixelCache, x_offset: isize, y_offset: isize) {
        let stride = self.channels.len();
        let source_stride = source.channels.len();
        let mut pairs = Vec::new();
        for (index, channel) in self.channels.iter().enumerate() {
            if let Some(source_index) = source.channels.iter().position(|candidate| candidate == channel) {
                pairs.push((index, source_index));
            }
        }
        for y in 0..source.height {
            let canvas_y = y as isize + y_offset;
            if canvas_y < 0 || canvas_y >= self.rows as isize {
                continue;
            }
            for x in 0..source.width {
                let canvas_x = x as isize + x_offset;
                if canvas_x < 0 || canvas_x >= self.columns as isize {
                    continue;
                }
                let target = (canvas_y as usize * self.columns + canvas_x as usize) * stride;
                let from = (y * source.width + x) * source_stride;
                for &(index, source_index) in &pairs {
                    self.data[target + index] = source.data[from + source_index];
                }
            }
        }
    }
}

/// `ResizeImage` with no filter given: Mitchell for a palette image, one
/// with alpha or an enlargement, Lanczos otherwise.
fn resize_tile(tile: &PixelCache, columns: usize, rows: usize, pseudo: bool, alpha: bool) -> Result<PixelCache> {
    if columns == 0 || rows == 0 {
        bail!("NegativeOrZeroImageSize");
    }
    let x_factor = columns as f64 / tile.width as f64;
    let y_factor = rows as f64 / tile.height as f64;
    let filter = if pseudo || alpha || x_factor * y_factor > 1.0 { Filter::Mitchell } else { Filter::Lanczos };
    let stride = tile.channels.len();
    let kept = if alpha { 4 } else { 3 };
    let mut data = Vec::with_capacity(tile.width * tile.height * kept);
    for pixel in tile.data.chunks_exact(stride) {
        data.extend_from_slice(&pixel[..kept]);
    }
    let mut image = Image::new(tile.width, tile.height, kept);
    image.data = data;
    let resized = resize_with_filter(&image, columns, rows, filter);
    let channels = tile.channels[..kept].to_vec();
    Ok(PixelCache::new(columns, rows, channels, resized.data, CacheColorspace::SRgb, DEFAULT_GAMMA))
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    // A 512-byte header, "PICT" (embedded in OLE2), or none when the first
    // word is the picture size; the header bytes are signed chars.
    let signed = |byte: Option<u8>| byte.map_or(-1i64, |value| value as i8 as i64) as u64;
    let first = blob.byte();
    let second = blob.byte();
    let size = (signed(first) << 8) | signed(second);
    if blob.size() % 65536 != size {
        let third = blob.byte();
        let fourth = blob.byte();
        let ole = [first, second, third, fourth] == [Some(b'P'), Some(b'I'), Some(b'C'), Some(b'T')];
        if !ole {
            blob.read(508);
        }
        blob.msb_u16();
    }
    let mut frame = read_rectangle(&mut blob)?;
    let mut c = blob.byte();
    while c == Some(0) {
        c = blob.byte();
    }
    if c != Some(0x11) {
        bail!("ImproperImageHeader");
    }
    let version = blob.byte();
    match version {
        Some(2) => {
            if blob.byte() != Some(0xff) {
                bail!("ImproperImageHeader");
            }
        }
        Some(1) => {}
        _ => bail!("ImproperImageHeader"),
    }
    let version = version.unwrap_or(0);
    if frame.left >= frame.right || frame.top >= frame.bottom {
        bail!("ImproperImageHeader");
    }
    let mut canvas = Canvas {
        columns: 0,
        rows: 0,
        colorspace: CacheColorspace::SRgb,
        channels: vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue],
        data: Vec::new(),
    };
    canvas.reset(frame.width(), frame.height())?;

    let mut pixmap = Pixmap::default();
    let mut flags = 0u16;
    let mut jpeg = false;
    let mut code: i64 = 0;
    while !blob.eof {
        if version == 1 || !blob.tell().is_multiple_of(2) {
            code = blob.byte().map_or(-1, i64::from);
        }
        if version == 2 {
            code = blob.msb_u16() as i16 as i64;
        }
        code &= 0xffff;
        if code == 0 {
            continue;
        }
        if code <= 0xa1 {
            match code {
                0x01 => {
                    // The clipping region: a rectangle resizes the canvas,
                    // black again.
                    let length = read_length(&mut blob)?;
                    if length != 0x0a {
                        skip(&mut blob, length.saturating_sub(2));
                    } else {
                        frame = read_rectangle(&mut blob)?;
                        canvas.reset(frame.width(), frame.height())?;
                    }
                }
                0x12..=0x14 => {
                    let pattern = blob.msb_u16();
                    skip(&mut blob, 8);
                    if pattern == 2 {
                        skip(&mut blob, 5);
                    } else {
                        if pattern != 1 {
                            bail!("UnknownPatternType");
                        }
                        read_length(&mut blob)?;
                        frame = read_rectangle(&mut blob)?;
                        pixmap = read_pixmap(&mut blob)?;
                        blob.msb_u32();
                        flags = blob.msb_u16();
                        let mut length = read_length(&mut blob)? as usize;
                        for _ in 0..=length {
                            blob.msb_u32();
                        }
                        // IM swaps the sides here.
                        let mut width = frame.height();
                        let height = frame.width();
                        if pixmap.bits_per_pixel <= 8 {
                            length &= 0x7fff;
                        }
                        if pixmap.bits_per_pixel == 16 {
                            width <<= 1;
                        }
                        if length == 0 {
                            length = width;
                        }
                        if length < 8 {
                            skip(&mut blob, (length * height) as u64);
                        } else {
                            for _ in 0..height {
                                if blob.eof {
                                    break;
                                }
                                let scanline_length = if length > 250 {
                                    blob.msb_u16() as u64
                                } else {
                                    blob.byte().map_or(u64::MAX, u64::from)
                                };
                                if scanline_length > blob.size() {
                                    bail!("InsufficientImageDataInFile");
                                }
                                skip(&mut blob, scanline_length);
                            }
                        }
                    }
                }
                0x1b => {
                    // The background colour, which nothing here uses.
                    blob.read(6);
                }
                0x70..=0x77 => {
                    let length = read_length(&mut blob)?;
                    skip(&mut blob, length.saturating_sub(2));
                }
                0x90 | 0x91 | 0x98 | 0x99 | 0x9a | 0x9b => {
                    let direct = code == 0x9a || code == 0x9b;
                    let mut bytes_per_line = 0usize;
                    if direct {
                        blob.read(6);
                    } else {
                        bytes_per_line = blob.msb_u16() as usize;
                    }
                    let packed = bytes_per_line & 0x8000 != 0;
                    frame = read_rectangle(&mut blob)?;
                    // A tile with no area is a copy of the canvas sharing its
                    // pixels, which clearing it clears too.
                    let mut tile_columns = frame.width();
                    let mut tile_rows = frame.height();
                    if tile_columns == 0 || tile_rows == 0 {
                        tile_columns = canvas.columns;
                        tile_rows = canvas.rows;
                        canvas.data.fill(0.0);
                    }
                    let mut alpha = false;
                    if direct || packed {
                        pixmap = read_pixmap(&mut blob)?;
                        alpha = pixmap.component_count == 4;
                    }
                    let pseudo = !direct;
                    let mut entries = Vec::new();
                    if pseudo {
                        let mut colors = 2;
                        if packed {
                            blob.msb_u32();
                            flags = blob.msb_u16();
                            colors = blob.msb_u16() as usize + 1;
                        }
                        entries = colormap::acquire(colors).context("MemoryAllocationFailed")?;
                        if packed {
                            for i in 0..colors {
                                let mut k = blob.msb_u16() as usize % colors;
                                if flags & 0x8000 != 0 {
                                    k = i;
                                }
                                entries[k][0] = blob.msb_u16() as f64;
                                entries[k][1] = blob.msb_u16() as f64;
                                entries[k][2] = blob.msb_u16() as f64;
                            }
                        } else {
                            for entry in entries.iter_mut() {
                                for value in entry[..3].iter_mut() {
                                    *value = QUANTUM_RANGE - *value;
                                }
                            }
                        }
                    }
                    if blob.eof {
                        bail!("InsufficientImageDataInFile");
                    }
                    let source = read_rectangle(&mut blob)?;
                    let destination = read_rectangle(&mut blob)?;
                    blob.msb_u16();
                    if code == 0x91 || code == 0x99 || code == 0x9b {
                        let length = read_length(&mut blob)?;
                        skip(&mut blob, length.saturating_sub(2));
                    }
                    let base_channels = canvas.channels.len();
                    let layout = TileLayout {
                        columns: tile_columns,
                        rows: tile_rows,
                        direct,
                        alpha,
                        // `AcquireImageColormap` doesn't resync the cache, so
                        // the index channel isn't counted yet.
                        channels: base_channels + usize::from(alpha),
                    };
                    let bits_per_pixel = if !direct && !packed { 1 } else { pixmap.bits_per_pixel as u32 };
                    let pixels = decode_image(&mut blob, &layout, bytes_per_line, bits_per_pixel).context("UnableToUncompressImage")?;

                    // The tile's red, green, blue and (opaque) alpha.
                    let stride = 4;
                    let mut data = vec![0.0f32; tile_columns * tile_rows * stride];
                    let byte_at = |offset: isize| -> u8 {
                        if offset < 0 {
                            return 0;
                        }
                        pixels.get(offset as usize).copied().unwrap_or(0)
                    };
                    let extent = pixels.len() as isize;
                    let columns = tile_columns as isize;
                    let canvas_columns = canvas.columns as isize;
                    let mut p: isize = 0;
                    for y in 0..tile_rows {
                        if p > extent + canvas_columns {
                            bail!("NotEnoughPixelData");
                        }
                        let row = &mut data[y * tile_columns * stride..(y + 1) * tile_columns * stride];
                        for pixel in row.chunks_exact_mut(stride) {
                            pixel[3] = QUANTUM_RANGE as f32;
                            if pseudo {
                                let index = byte_at(p) as usize;
                                if index >= entries.len() {
                                    bail!("InvalidColormapIndex");
                                }
                                for (channel, value) in pixel[..3].iter_mut().enumerate() {
                                    *value = entries[index][channel] as f32;
                                }
                            } else if bits_per_pixel == 16 {
                                let high = byte_at(p);
                                p += 1;
                                let low = byte_at(p);
                                pixel[0] = char_quantum((high & 0x7c) << 1);
                                pixel[1] = char_quantum(((high & 0x03) << 6) | ((low & 0xe0) >> 2));
                                pixel[2] = char_quantum((low & 0x1f) << 3);
                            } else if !alpha {
                                if p > extent + 2 * canvas_columns {
                                    bail!("NotEnoughPixelData");
                                }
                                pixel[0] = char_quantum(byte_at(p));
                                pixel[1] = char_quantum(byte_at(p + columns));
                                pixel[2] = char_quantum(byte_at(p + 2 * columns));
                            } else {
                                if p > extent + 3 * canvas_columns {
                                    bail!("NotEnoughPixelData");
                                }
                                pixel[3] = char_quantum(byte_at(p));
                                pixel[0] = char_quantum(byte_at(p + columns));
                                pixel[1] = char_quantum(byte_at(p + 2 * columns));
                                pixel[2] = char_quantum(byte_at(p + 3 * columns));
                            }
                            p += 1;
                        }
                        if direct && bits_per_pixel != 16 {
                            p += (pixmap.component_count as isize - 1) * columns;
                            if p < 0 {
                                break;
                            }
                        }
                    }
                    let drawn = direct || packed;
                    if !jpeg && !blob.eof && drawn {
                        let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Alpha];
                        let mut tile = PixelCache::new(tile_columns, tile_rows, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA);
                        let same_width = source.width() == destination.width();
                        let same_height = source.height() == destination.height();
                        if !same_width || !same_height {
                            tile = resize_tile(&tile, destination.width(), destination.height(), pseudo, alpha)?;
                        }
                        canvas.copy(&tile, destination.left as isize, destination.top as isize);
                    }
                }
                0xa1 => {
                    let _kind = blob.msb_u16();
                    let mut length = read_length(&mut blob)? as usize;
                    if length != 0 {
                        blob.msb_u32();
                        length -= length.min(4);
                        if length != 0 && blob.read(length).len() != length {
                            bail!("UnableToReadImageData");
                        }
                    }
                }
                _ => {
                    let length = CODE_LENGTHS[code as usize];
                    if length == -1 {
                        blob.msb_u16();
                    } else {
                        skip(&mut blob, length as u64);
                    }
                }
            }
        }
        if code == 0xc00 {
            skip(&mut blob, 24);
            continue;
        }
        if (0xb0..=0xcf).contains(&code) || (0x8000..=0x80ff).contains(&code) {
            continue;
        }
        if code == 0x8200 {
            // QuickTime: a JPEG after a 154-byte image description.
            jpeg = true;
            let length = blob.msb_u32() as u64;
            if length > blob.size() {
                bail!("InsufficientImageDataInFile");
            }
            if length <= 154 {
                continue;
            }
            blob.read(24);
            frame = read_rectangle(&mut blob)?;
            skip(&mut blob, 122);
            let length = (length - 154) as usize;
            let stream = blob.read(length);
            if stream.len() != length {
                bail!("ImproperImageHeader");
            }
            let tile = super::jpeg::decode_bytes(stream)?;
            let columns = canvas.columns.max(tile.width);
            let rows = canvas.rows.max(tile.height);
            canvas.grow(columns, rows);
            canvas.transform(tile.colorspace)?;
            canvas.copy(&tile, frame.left as isize, frame.right as isize);
            continue;
        }
        if code == 0xff || code == 0xffff {
            break;
        }
        if (0xd0..=0xfe).contains(&code) || (0x8100..=0xffff).contains(&code) {
            let length = read_length(&mut blob)?;
            skip(&mut blob, length);
            continue;
        }
        if (0x100..=0x7fff).contains(&code) {
            let length = ((code >> 7) & 0xff) as u64;
            skip(&mut blob, length);
            continue;
        }
    }
    Ok(PixelCache::new(canvas.columns, canvas.rows, canvas.channels, canvas.data, canvas.colorspace, DEFAULT_GAMMA))
}
