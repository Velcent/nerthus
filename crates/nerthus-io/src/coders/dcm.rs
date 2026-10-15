//! DICOM medical images as ImageMagick reads them (`coders/dcm.c`): data
//! elements after an optional 128-byte preamble and "DICM", implicit or
//! explicit VR (IM's own dictionary, `data/dicom.toml`, gives the implicit
//! ones), little- or big-endian by transfer syntax, up to the pixel data.
//! One-sample pixels index a colormap (a grey ramp, or the file's lookup
//! tables) and take its colour truncated to a whole number; colour pixels
//! are scaled to quanta and truncated the same way; RLE and two-segment
//! RLE frames are unpacked; JPEG and JPEG 2000 frames are decoded as their
//! own images. A grey result is labelled GRAY, anything else linear RGB.
//!
//! Asked for one frame, IM reads the first and then goes on parsing the
//! rest of the file as data elements (the next frames' pixels included), so
//! what follows can still fail the read; this does the same.

use std::path::Path;
use std::sync::OnceLock;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use super::blob::Blob;
use super::colormap;
use super::quantum::{char_quantum, quantum_range, scale_any};
use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

#[derive(Deserialize)]
struct Element {
    group: u16,
    element: u16,
    vr: String,
}

#[derive(Deserialize)]
struct Dictionary {
    elements: Vec<Element>,
}

/// The VR IM assumes for an element: the first dictionary match, else "xs".
fn implicit_vr(group: u16, element: u16) -> [u8; 2] {
    static TABLE: OnceLock<std::collections::HashMap<(u16, u16), [u8; 2]>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let text = include_str!("../../data/dicom.toml");
        let dictionary: Dictionary = toml::from_str(text).expect("the DICOM dictionary parses");
        let mut table = std::collections::HashMap::new();
        for entry in dictionary.elements {
            let vr = entry.vr.as_bytes();
            table.entry((entry.group, entry.element)).or_insert([vr[0], vr[1]]);
        }
        table
    });
    if group == 0xffff {
        return *b"xs";
    }
    table.get(&(group, element)).copied().unwrap_or(*b"xs")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Endian {
    Lsb,
    Msb,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Compression {
    None,
    Jpeg,
    Jpeg2000,
    Rle,
}

/// `DCMInfo`, which sequences push and pop.
#[derive(Clone)]
struct Info {
    polarity: bool,
    scale: Option<Vec<f32>>,
    bits_allocated: u64,
    bits_per_entry: u64,
    bytes_per_pixel: u64,
    depth: u64,
    mask: u64,
    max_value: u64,
    samples_per_pixel: u64,
    signed_data: u64,
    significant_bits: u64,
    width: u64,
    height: u64,
    window_center: f64,
    window_width: f64,
}

/// `DCMStreamInfo`: the RLE stream's state.
#[derive(Default)]
struct Stream {
    remaining: u64,
    segment_count: u64,
    segments: [i64; 15],
    offsets: Vec<i64>,
    count: i64,
    byte: i32,
}

/// The lookup tables (`DCMMap`).
#[derive(Default)]
struct Maps {
    gray: Option<Vec<i32>>,
    red: Option<Vec<i32>>,
    green: Option<Vec<i32>>,
    blue: Option<Vec<i32>>,
}

/// The IM image being read.
struct Frame {
    endian: Endian,
    compression: Compression,
    depth: u64,
    plane_interlace: bool,
}

impl Frame {
    /// `AcquireImage` (or `AcquireNextImage`, which keeps the byte order).
    fn new(endian: Endian) -> Frame {
        Frame { endian, compression: Compression::None, depth: 16, plane_interlace: false }
    }
}

fn byte(blob: &mut Blob) -> i32 {
    blob.byte().map_or(-1, i32::from)
}

/// `ReadBlobSignedShort` in the image's byte order (or little-endian for
/// group 2).
fn signed_short(blob: &mut Blob, endian: Endian) -> i32 {
    let value = match endian {
        Endian::Lsb => blob.lsb_u16(),
        Endian::Msb => blob.msb_u16(),
    };
    value as i16 as i32
}

fn signed_long(blob: &mut Blob, endian: Endian) -> i32 {
    let value = match endian {
        Endian::Lsb => blob.lsb_u32(),
        Endian::Msb => blob.msb_u32(),
    };
    value as i32
}

/// `StringToUnsignedLong`: `strtoul` with a 32-bit `unsigned long`.
fn unsigned_long(text: &[u8]) -> u64 {
    let mut i = 0;
    while i < text.len() && super::text::is_space(text[i]) {
        i += 1;
    }
    let negative = text.get(i) == Some(&b'-');
    if matches!(text.get(i), Some(b'-' | b'+')) {
        i += 1;
    }
    let mut value: u64 = 0;
    while let Some(digit) = text.get(i).filter(|byte| byte.is_ascii_digit()) {
        value = (value * 10 + (digit - b'0') as u64).min(u64::from(u32::MAX) + 1);
        i += 1;
    }
    if value > u64::from(u32::MAX) {
        return u64::from(u32::MAX);
    }
    if negative {
        return u64::from((value as u32).wrapping_neg());
    }
    value
}

/// The bytes of a C string.
fn c_str(bytes: &[u8]) -> &[u8] {
    let end = bytes.iter().position(|&byte| byte == 0).unwrap_or(bytes.len());
    &bytes[..end]
}

/// `sscanf(text, ".%d.%d")`: how many numbers it assigns, and them.
fn scan_syntax(text: &[u8]) -> (usize, i32, i32) {
    let mut values = [1i32, 0i32];
    let mut position = 0;
    for (index, value) in values.iter_mut().enumerate() {
        if text.get(position) != Some(&b'.') {
            return (index, values[0], values[1]);
        }
        position += 1;
        while position < text.len() && super::text::is_space(text[position]) {
            position += 1;
        }
        let start = position;
        if matches!(text.get(position), Some(b'-' | b'+')) {
            position += 1;
        }
        let digits = position;
        while text.get(position).is_some_and(u8::is_ascii_digit) {
            position += 1;
        }
        if position == digits {
            return (index, values[0], values[1]);
        }
        let number = std::str::from_utf8(&text[start..position]).unwrap_or("0");
        *value = number.parse::<i64>().map_or(i32::MAX, |parsed| parsed.clamp(i32::MIN as i64, i32::MAX as i64) as i32);
    }
    (2, values[0], values[1])
}

/// The reader: the blob, the RLE stream and the current image.
struct Reader<'a> {
    blob: Blob<'a>,
    stream: Stream,
    frame: Frame,
}

impl Reader<'_> {
    /// `ReadDCMByte`.
    fn read_byte(&mut self) -> i32 {
        if self.frame.compression != Compression::Rle {
            return byte(&mut self.blob);
        }
        let stream = &mut self.stream;
        if stream.count == 0 {
            stream.remaining = stream.remaining.saturating_sub(2);
            let count = byte(&mut self.blob) as i64;
            let value = byte(&mut self.blob);
            if count == 128 {
                return 0;
            }
            if count < 128 {
                stream.count = count;
                stream.byte = -1;
            } else {
                stream.count = 256 - count;
                stream.byte = value;
            }
            return value;
        }
        stream.count -= 1;
        if stream.byte >= 0 {
            return stream.byte;
        }
        if stream.remaining > 0 {
            stream.remaining -= 1;
        }
        byte(&mut self.blob)
    }

    /// `ReadDCMShort`.
    fn read_short(&mut self) -> u16 {
        if self.frame.compression != Compression::Rle {
            return self.blob.lsb_u16();
        }
        let shift = if self.frame.depth < 16 { 4 } else { 8 };
        let high = self.read_byte() as u16 as u32;
        let low = self.read_byte() as u16 as u32;
        ((high << shift) | low) as u16
    }

    /// `ReadDCMPixels` for one segment: the first sets the pixels, the
    /// second adds its bytes to them.
    fn read_pixels(&mut self, image: &mut Image, info: &Info, first_segment: bool) -> Result<()> {
        let stride = image.stride();
        let mut packed_byte = 0i32;
        let mut packed_index = 0usize;
        for y in 0..image.rows {
            for x in 0..image.columns {
                let pixel_offset = (y * image.columns + x) * stride;
                let mut pixel = [0u32; 3];
                if info.samples_per_pixel == 1 {
                    let mut value: i32;
                    if info.bytes_per_pixel == 1 {
                        value = self.read_byte();
                        if info.polarity {
                            value = (info.max_value as i32).wrapping_sub(value);
                        }
                    } else if info.bits_allocated != 12 || info.significant_bits != 12 {
                        let short = self.read_short();
                        value = if info.signed_data != 0 { short as i16 as i32 } else { short as i32 };
                        if info.polarity {
                            value = (info.max_value as i32).wrapping_sub(value);
                        }
                    } else {
                        if packed_index & 1 != 0 {
                            value = packed_byte;
                            packed_byte = self.read_byte();
                            if packed_byte >= 0 {
                                value |= packed_byte << 8;
                            }
                        } else {
                            value = self.read_short() as i16 as i32;
                            packed_byte = value & 0x0f;
                            value >>= 4;
                        }
                        packed_index += 1;
                    }
                    if info.signed_data == 1 {
                        value -= 32767;
                    }
                    let index = (value as i64 & info.mask as i64) as i32;
                    if index < 0 || index as usize >= image.colormap.len() {
                        bail!("InvalidColormapIndex");
                    }
                    let index_slot = &mut image.data[pixel_offset + 3];
                    *index_slot = if first_segment { index as f32 } else { second_segment(index as u64, *index_slot) };
                    let entry = image.colormap[index as usize];
                    pixel = [entry[0] as u32, entry[1] as u32, entry[2] as u32];
                } else {
                    for value in pixel.iter_mut() {
                        *value = if info.bytes_per_pixel == 1 { self.read_byte() as u32 } else { self.read_short() as u32 };
                    }
                    for value in pixel.iter_mut() {
                        *value &= info.mask as u32;
                    }
                    if let Some(scale) = &info.scale {
                        let range = quantum_range(info.depth as u32);
                        for value in pixel.iter_mut() {
                            if (*value as u64) <= range {
                                *value = scale[*value as usize] as u32;
                            }
                        }
                    }
                }
                for (channel, &value) in pixel.iter().enumerate() {
                    let slot = &mut image.data[pixel_offset + channel];
                    *slot = if first_segment { value as f32 } else { second_segment(value as u64, *slot) };
                }
            }
        }
        Ok(())
    }

}

/// A second RLE segment's sample joined to the first's: the first becomes
/// the high byte (`ReadDCMPixels` little-endian, as without the
/// `dcm:fix-byte-order` option).
fn second_segment(value: u64, first: f32) -> f32 {
    (value | ((first as u64) << 8)) as f32
}

/// A frame's pixels: red, green, blue and, for one-sample images, the
/// colormap index.
struct Image {
    columns: usize,
    rows: usize,
    pseudo: bool,
    colormap: Vec<[f64; 4]>,
    data: Vec<f32>,
}

impl Image {
    fn stride(&self) -> usize {
        3 + usize::from(self.pseudo)
    }

    /// `IdentifyImageCoderGray`: every pixel's red, green and blue equal.
    fn is_gray(&self) -> bool {
        let stride = self.stride();
        self.data.chunks_exact(stride).all(|pixel| {
            let red_green = pixel[0] as f64 - pixel[1] as f64;
            let green_blue = pixel[1] as f64 - pixel[2] as f64;
            red_green.abs() < 1.0e-12 && green_blue.abs() < 1.0e-12
        })
    }

    fn into_pixel_cache(self) -> PixelCache {
        let stride = self.stride();
        let gray = self.is_gray();
        let (colorspace, mut channels, data) = if gray {
            let mut data = Vec::with_capacity(self.columns * self.rows * (stride - 2));
            for pixel in self.data.chunks_exact(stride) {
                data.push(pixel[0]);
                if self.pseudo {
                    data.push(pixel[3]);
                }
            }
            (CacheColorspace::Gray, vec![PixelChannel::Gray], data)
        } else {
            (CacheColorspace::Rgb, vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue], self.data)
        };
        if self.pseudo {
            channels.push(PixelChannel::Index);
        }
        let mut cache = PixelCache::new(self.columns, self.rows, channels, data, colorspace, DEFAULT_GAMMA);
        if self.pseudo {
            cache.colormap = self.colormap;
        }
        cache
    }
}

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

/// How an element's value is read, from its VR (`quantum`).
fn value_size(vr: &[u8; 2]) -> u64 {
    match vr {
        b"OW" | b"SS" | b"US" => 2,
        b"FL" | b"OF" | b"SL" | b"UL" => 4,
        b"FD" => 8,
        _ => 1,
    }
}

/// A lookup table's entries (`map.red` etc.): little- or big-endian words
/// read `bits_per_entry` bytes apart.
fn read_lookup(data: &[u8], colors: usize, step: usize, endian: Endian) -> Vec<i32> {
    let mut map = vec![0i32; colors.max(65536)];
    for (i, entry) in map.iter_mut().take(colors).enumerate() {
        let first = data.get(i * step).copied().unwrap_or(0) as i32;
        let second = data.get(i * step + 1).copied().unwrap_or(0) as i32;
        *entry = match endian {
            Endian::Msb => (first << 8) | second,
            Endian::Lsb => first | (second << 8),
        };
    }
    map
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut reader = Reader { blob: Blob::new(bytes), stream: Stream::default(), frame: Frame::new(Endian::Lsb) };
    reader.frame.depth = 8;
    if reader.blob.read(128).len() != 128 {
        bail!("ImproperImageHeader");
    }
    let magick = reader.blob.read(4);
    if magick != b"DICM" {
        reader.blob.seek(0);
    }
    let mut info = Info {
        polarity: false,
        scale: None,
        bits_allocated: 8,
        bits_per_entry: 1,
        bytes_per_pixel: 1,
        depth: 8,
        mask: 0xffff,
        max_value: 255,
        samples_per_pixel: 1,
        signed_data: u64::from(u32::MAX),
        significant_bits: 0,
        width: 0,
        height: 0,
        window_center: 0.0,
        window_width: 0.0,
    };
    let mut maps = Maps::default();
    let mut stack: Vec<Info> = Vec::new();
    let mut element: u16 = 0;
    let mut explicit_vr = [0u8; 2];
    let mut explicit_file = false;
    let mut explicit_retry = false;
    let mut colors: u64 = 0;
    let mut number_scenes: u64 = 1;
    let blob_size = reader.blob.size() as i64;
    let mut images: Vec<PixelCache> = Vec::new();

    while (reader.blob.tell() as i64) < blob_size - 10 {
        let mut group: u16 = 0;
        let mut ended = false;
        while group != 0x7fe0 || element != 0x0010 {
            let blob = &mut reader.blob;
            group = blob.lsb_u16();
            element = blob.lsb_u16();
            if (group == 0xfffc && element == 0xfffc) || (group == 0 && element == 0) {
                ended = true;
                break;
            }
            if group != 0x0002 && reader.frame.endian == Endian::Msb {
                group = group.swap_bytes();
                element = element.swap_bytes();
            }
            let mut vr = implicit_vr(group, element);
            let read = blob.read(2);
            if read.len() != 2 {
                bail!("ImproperImageHeader");
            }
            explicit_vr.copy_from_slice(read);
            if !explicit_file && group != 0x0002 {
                explicit_file = explicit_vr[0].is_ascii_uppercase() && explicit_vr[1].is_ascii_uppercase();
            }
            let use_explicit = (group == 0x0002 && !explicit_retry) || explicit_file;
            if use_explicit && &vr == b"xs" {
                vr = explicit_vr;
                if vr[0] == 0 {
                    vr[1] = 0;
                }
            }
            let mut quantum: u64;
            if !use_explicit || &vr == b"!!" {
                let back = blob.tell() as i64 - 2;
                blob.seek(back);
                quantum = 4;
            } else {
                quantum = 2;
                if matches!(&explicit_vr, b"OB" | b"OW" | b"OF" | b"SQ" | b"UN" | b"UT") {
                    blob.lsb_u16();
                    quantum = 4;
                }
            }
            // Leaving a sequence restores what entering it saved.
            if group == 0xfffe && element == 0xe0dd
                && let Some(saved) = stack.pop() {
                    info = saved;
                }
            if info.scale.is_some() && &explicit_vr == b"SQ" {
                stack.push(info.clone());
            }
            let value_endian = if group == 0x0002 { Endian::Lsb } else { reader.frame.endian };
            let mut datum: i32 = match quantum {
                4 => signed_long(blob, value_endian),
                2 => signed_short(blob, value_endian),
                _ => 0,
            };
            quantum = 0;
            let mut length: u64 = 1;
            if datum != 0 {
                quantum = value_size(&vr);
                if datum != -1 {
                    length = (datum as i64 as u64) / quantum;
                } else {
                    quantum = 0;
                    length = 0;
                }
            }
            if group == 0x7fe0 && element == 0x0010 {
                break;
            }
            let mut data: Option<Vec<u8>> = None;
            if length == 1 && quantum == 1 {
                datum = byte(blob);
            } else if length == 1 && quantum == 2 {
                datum = signed_short(blob, value_endian);
            } else if length == 1 && quantum == 4 {
                datum = signed_long(blob, value_endian);
            } else if quantum != 0 && length != 0 {
                if length > blob_size as u64 {
                    bail!("InsufficientImageDataInFile");
                }
                let size = (quantum * length) as usize;
                let read = blob.read(size);
                if read.len() != size {
                    bail!("InsufficientImageDataInFile");
                }
                let mut value = read.to_vec();
                value.push(0);
                data = Some(value);
            }
            if group == 0xfffe && element == 0xe0dd {
                continue;
            }
            let data_bytes = data.as_deref();
            match (group, element) {
                (0x0002, 0x0010) => {
                    if datum == 0 && !explicit_retry {
                        explicit_retry = true;
                        blob.seek(0);
                        group = 0;
                        element = 0;
                    } else {
                        let syntax = data_bytes.map(c_str).unwrap_or(&[]);
                        let syntax = &syntax[..syntax.len().min(4095)];
                        if syntax.starts_with(b"1.2.840.10008.1.2") {
                            let mut kind = 1;
                            let mut subtype = 0;
                            if syntax.len() > 17 {
                                let (count, first, second) = scan_syntax(&syntax[17..]);
                                if count < 1 {
                                    bail!("ImproperImageHeader");
                                }
                                kind = first;
                                if count > 1 {
                                    subtype = second;
                                }
                            }
                            match kind {
                                1 => reader.frame.endian = Endian::Lsb,
                                2 => reader.frame.endian = Endian::Msb,
                                4 => {
                                    reader.frame.compression = if (90..=93).contains(&subtype) { Compression::Jpeg2000 } else { Compression::Jpeg };
                                }
                                5 => reader.frame.compression = Compression::Rle,
                                _ => {}
                            }
                        }
                    }
                }
                (0x0028, 0x0002) => {
                    info.samples_per_pixel = datum as i64 as u64;
                    if info.samples_per_pixel == 0 || info.samples_per_pixel > 4 {
                        bail!("ImproperImageHeader");
                    }
                }
                (0x0028, 0x0004) => {
                    if let Some(data) = data_bytes {
                        let photometric = c_str(&data[..(length as usize).min(4095)]);
                        info.polarity = photometric.eq_ignore_ascii_case(b"MONOCHROME1 ");
                    }
                }
                (0x0028, 0x0006) => {
                    if datum == 1 {
                        reader.frame.plane_interlace = true;
                    }
                }
                (0x0028, 0x0008) => {
                    if let Some(data) = data_bytes {
                        number_scenes = unsigned_long(data);
                    }
                }
                (0x0028, 0x0010) => info.height = datum as i64 as u64,
                (0x0028, 0x0011) => info.width = datum as i64 as u64,
                (0x0028, 0x0100) => {
                    info.bits_allocated = datum as i64 as u64;
                    info.bytes_per_pixel = if datum > 8 { 2 } else { 1 };
                    info.depth = info.bits_allocated;
                    if info.depth == 0 || info.depth > 32 {
                        bail!("ImproperImageHeader");
                    }
                    // `1UL << bits` on a 32-bit `unsigned long`.
                    info.max_value = u64::from(1u32.wrapping_shl(info.bits_allocated as u32).wrapping_sub(1));
                    reader.frame.depth = info.depth;
                }
                (0x0028, 0x0101) => {
                    info.significant_bits = datum as i64 as u64;
                    info.bytes_per_pixel = if info.significant_bits > 8 { 2 } else { 1 };
                    info.depth = info.significant_bits;
                    if info.depth == 0 || info.depth > 16 {
                        bail!("ImproperImageHeader");
                    }
                    info.max_value = u64::from(1u32.wrapping_shl(info.significant_bits as u32).wrapping_sub(1));
                    info.mask = quantum_range(info.significant_bits as u32);
                    reader.frame.depth = info.depth;
                }
                (0x0028, 0x0103) => info.signed_data = datum as i64 as u64,
                (0x0028, 0x1050) => {
                    if let Some(data) = data_bytes {
                        info.window_center = super::text::strtod(data, 0).0;
                    }
                }
                (0x0028, 0x1051) => {
                    if let Some(data) = data_bytes {
                        info.window_width = super::text::strtod(data, 0).0;
                    }
                }
                (0x0028, 0x1200) | (0x0028, 0x3006) => {
                    if let Some(data) = data_bytes {
                        colors = length / info.bytes_per_pixel;
                        let mut map = vec![0i32; (colors as usize).max(65536)];
                        for (i, entry) in map.iter_mut().take(colors as usize).enumerate() {
                            *entry = if info.bytes_per_pixel == 1 {
                                data[i] as i32
                            } else {
                                i16::from_le_bytes([data[2 * i], data[2 * i + 1]]) as i32
                            };
                        }
                        maps.gray = Some(map);
                    }
                }
                (0x0028, 0x1201..=0x1203) => {
                    if let Some(data) = data_bytes {
                        colors = length / info.bytes_per_pixel;
                        let map = read_lookup(data, colors as usize, info.bits_per_entry as usize, reader.frame.endian);
                        match element {
                            0x1201 => maps.red = Some(map),
                            0x1202 => maps.green = Some(map),
                            _ => maps.blue = Some(map),
                        }
                    }
                }
                (0x0028, 0x3002) => {
                    info.bits_per_entry = datum as i64 as u64;
                    if info.bits_per_entry == 0 || info.bits_per_entry > 2 {
                        bail!("ImproperImageHeader");
                    }
                }
                (0x2050, 0x0020)
                    if data_bytes.is_some_and(|data| data.starts_with(b"INVERSE")) => {
                        info.polarity = true;
                    }
                _ => {}
            }
            if blob.eof {
                bail!("UnexpectedEndOfFile");
            }
        }
        if ended {
            // The image being read is dropped; the earlier ones stay.
            break;
        }
        if info.width == 0 || info.height == 0 {
            bail!("ImproperImageHeader");
        }
        if info.signed_data == 0xffff {
            info.signed_data = u64::from(info.significant_bits == 16);
        }
        if matches!(reader.frame.compression, Compression::Jpeg | Compression::Jpeg2000) {
            return read_encapsulated(&mut reader, number_scenes, blob_size);
        }
        if info.depth != 16 {
            let range = quantum_range(info.depth as u32);
            if range.saturating_add(1) > blob_size as u64 {
                bail!("InsufficientImageDataInFile");
            }
            let mut scale = vec![0.0f32; (range as usize + 1).max(65535) + 1];
            for (i, entry) in scale.iter_mut().take(range as usize + 1).enumerate() {
                *entry = scale_any(i as u64, range);
            }
            info.scale = Some(scale);
        }
        if reader.frame.compression == Compression::Rle {
            for _ in 0..reader.stream.remaining {
                if reader.blob.byte().is_none() {
                    break;
                }
            }
            reader.blob.lsb_u16();
            reader.blob.lsb_u16();
            let length = reader.blob.lsb_u32() as u64;
            if length > blob_size as u64 {
                bail!("InsufficientImageDataInFile");
            }
            let offset_count = (length >> 2) as usize;
            if offset_count != 0 {
                let mut offsets = vec![0i64; offset_count];
                for offset in offsets.iter_mut() {
                    *offset = reader.blob.lsb_u32() as i32 as i64;
                    if reader.blob.eof {
                        break;
                    }
                }
                let base = reader.blob.tell() as i64;
                for offset in offsets.iter_mut() {
                    *offset += base;
                }
                reader.stream.offsets = offsets;
            }
        }
        // One frame (Magick.NET asks for one; each later image is one too).
        if number_scenes > 0 {
            let image = read_frame(&mut reader, &mut info, &maps, &mut colors)?;
            images.push(image.into_pixel_cache());
            if reader.blob.eof {
                bail!("UnexpectedEndOfFile");
            }
        } else {
            bail!("DICOM files with no frames are not supported");
        }
        if (reader.blob.tell() as i64) < blob_size - 10 {
            let endian = reader.frame.endian;
            reader.frame = Frame::new(endian);
        }
    }
    images.into_iter().next().context("ImproperImageHeader")
}

/// One frame of uncompressed or RLE pixels (the body of IM's scene loop).
fn read_frame(reader: &mut Reader, info: &mut Info, maps: &Maps, colors: &mut u64) -> Result<Image> {
    let columns = info.width as usize;
    let rows = info.height as usize;
    reader.frame.depth = info.depth;
    let pseudo = info.samples_per_pixel == 1;
    // `SetImageBackgroundColor`: white.
    let mut image = Image { columns, rows, pseudo, colormap: Vec::new(), data: Vec::new() };
    image.data = vec![65535.0; columns * rows * image.stride()];
    if pseudo {
        let stride = image.stride();
        for pixel in image.data.chunks_exact_mut(stride) {
            pixel[3] = 0.0;
        }
        if *colors == 0 {
            *colors = 1u64 << info.depth.min(63);
        }
        let count = usize::try_from(*colors).ok();
        let mut colormap = count.and_then(colormap::acquire).context("UnableToCreateColormap")?;
        let tables = [(&maps.red, 0usize), (&maps.green, 1), (&maps.blue, 2)];
        for (table, channel) in tables {
            if let Some(table) = table {
                for (entry, &value) in colormap.iter_mut().zip(table.iter()) {
                    entry[channel] = value as u16 as f64;
                }
            }
        }
        if let Some(gray) = &maps.gray {
            for (entry, &value) in colormap.iter_mut().zip(gray.iter()) {
                let level = value as u16 as f64;
                entry[0] = level;
                entry[1] = level;
                entry[2] = level;
            }
        }
        image.colormap = colormap;
    }
    if reader.frame.compression == Compression::Rle {
        for _ in 0..reader.stream.remaining {
            if reader.blob.byte().is_none() {
                break;
            }
        }
        let tag = ((reader.blob.lsb_u16() as u32) << 16) | reader.blob.lsb_u16() as u32;
        reader.stream.remaining = reader.blob.lsb_u32() as u64;
        if tag != 0xfffe_e000 || reader.stream.remaining <= 64 || reader.blob.eof {
            bail!("ImproperImageHeader");
        }
        reader.stream.count = 0;
        reader.stream.segment_count = reader.blob.lsb_u32() as u64;
        for segment in reader.stream.segments.iter_mut() {
            *segment = reader.blob.lsb_u32() as i32 as i64;
        }
        reader.stream.remaining -= 64;
        if reader.stream.segment_count > 1 {
            info.bytes_per_pixel = 1;
            info.depth = 8;
            if let Some(&first) = reader.stream.offsets.first() {
                let target = first + reader.stream.segments[0];
                reader.blob.seek(target);
            }
        }
    }
    if info.samples_per_pixel > 1 && reader.frame.plane_interlace {
        // Planar colour: one byte a sample, plane after plane.
        for plane in 0..info.samples_per_pixel as usize {
            for pixel in 0..columns * rows {
                let value = char_quantum(reader.read_byte() as u8);
                if plane < 3 {
                    image.data[pixel * 3 + plane] = value;
                }
            }
        }
    } else {
        reader.read_pixels(&mut image, info, true)?;
        if reader.stream.segment_count > 1 {
            if let Some(&first) = reader.stream.offsets.first() {
                let target = first + reader.stream.segments[1];
                reader.blob.seek(target);
            }
            reader.read_pixels(&mut image, info, false)?;
        }
    }
    Ok(image)
}

/// JPEG and JPEG 2000 frames: each fragment after the offset table is an
/// image of its own; the first is the result, but every frame is read.
fn read_encapsulated(reader: &mut Reader, number_scenes: u64, blob_size: i64) -> Result<PixelCache> {
    let blob = &mut reader.blob;
    for _ in 0..reader.stream.remaining {
        if blob.byte().is_none() {
            break;
        }
    }
    blob.lsb_u16();
    blob.lsb_u16();
    let length = blob.lsb_u32() as u64;
    if length > blob_size as u64 {
        bail!("InsufficientImageDataInFile");
    }
    for _ in 0..length >> 2 {
        blob.lsb_u32();
    }
    let mut images: Vec<PixelCache> = Vec::new();
    let mut complete = true;
    for _ in 0..number_scenes {
        let tag = ((blob.lsb_u16() as u32) << 16) | blob.lsb_u16() as u32;
        let length = blob.lsb_u32() as u64;
        if length > blob_size as u64 {
            bail!("InsufficientImageDataInFile");
        }
        if blob.eof {
            complete = false;
            break;
        }
        if tag == 0xfffe_e0dd {
            break;
        }
        if tag != 0xfffe_e000 {
            complete = false;
            break;
        }
        if length == 0 {
            // IM's copy loop ends on its EOF sentinel, which then stops it.
            break;
        }
        let fragment = blob.read(length as usize);
        if fragment.len() as u64 != length {
            bail!("UnexpectedEndOfFile");
        }
        let image = match reader.frame.compression {
            Compression::Jpeg2000 => super::jp2::decode_bytes(fragment)?,
            _ => super::jpeg::decode_bytes(fragment)?,
        };
        images.push(image);
    }
    if !complete {
        bail!("CorruptImageError");
    }
    images.into_iter().next().context("ImproperImageHeader")
}
