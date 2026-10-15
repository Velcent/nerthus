//! Khoros VIFF images as ImageMagick reads them (`coders/viff.c`): a
//! 1024-byte header (byte order from the machine dependency), an optional
//! colormap, then planar bands of bits, bytes, 16- or 32-bit integers or
//! floats. Wider samples are scaled to the quantum range and then cut to a
//! byte (`(unsigned char) ClampToQuantum(value)`), as IM does; big-endian
//! doubles are read little-endian.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::colormap;
use crate::pixel_cache::{char_quantum, CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

const QUANTUM_RANGE: f64 = 65535.0;
const DEP_DECORDER: u8 = 0x4;
const DEP_NSORDER: u8 = 0x8;
const TYP_BIT: u32 = 0;
const TYP_1_BYTE: u32 = 1;
const TYP_2_BYTE: u32 = 2;
const TYP_4_BYTE: u32 = 4;
const TYP_FLOAT: u32 = 5;
const TYP_DOUBLE: u32 = 9;
const MAPTYP_DOUBLE: u32 = 7;
const MS_NONE: u32 = 0;
const MS_ONEPERBAND: u32 = 1;
const MS_SHARED: u32 = 3;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

/// A sample of `storage` type at `index`: shorts, ints and floats swapped
/// to big-endian order for big-endian files, doubles never.
fn typed_value(bytes: &[u8], index: usize, storage: u32, double_type: u32, big_endian: bool) -> f64 {
    let take = |size: usize| -> Vec<u8> {
        let start = index * size;
        let mut chunk = bytes.get(start..start + size).map_or(vec![0; size], <[u8]>::to_vec);
        if big_endian && size != 8 {
            chunk.reverse();
        }
        chunk
    };
    match storage {
        TYP_2_BYTE => i16::from_le_bytes(take(2).try_into().unwrap()) as f64,
        TYP_4_BYTE => i32::from_le_bytes(take(4).try_into().unwrap()) as f64,
        TYP_FLOAT => f32::from_le_bytes(take(4).try_into().unwrap()) as f64,
        storage if storage == double_type => f64::from_le_bytes(take(8).try_into().unwrap()),
        _ => bytes.get(index).copied().unwrap_or(0) as f64,
    }
}

/// `(unsigned char)` of a float on MSVC x64: through a 32-bit integer, 0
/// for NaN or out of range.
fn float_to_byte(value: f32) -> u8 {
    if value.is_nan() || value >= 2_147_483_648.0 || value < -2_147_483_648.0 {
        return 0;
    }
    value as i32 as u8
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    if blob.read(1) != [0xab] {
        bail!("NotAVIFFImage");
    }
    let _file_type = blob.byte();
    let _release = blob.byte();
    let _version = blob.byte();
    let dependency = blob.byte().unwrap_or(0);
    blob.read(3);
    if blob.read(512).len() != 512 {
        bail!("ImproperImageHeader");
    }
    let big_endian = dependency != DEP_DECORDER && dependency != DEP_NSORDER;
    let long = |blob: &mut Blob| if big_endian { blob.msb_u32() } else { blob.lsb_u32() };
    let viff_rows = long(&mut blob);
    let viff_columns = long(&mut blob);
    let _subrows = long(&mut blob);
    let _x_offset = long(&mut blob);
    let _y_offset = long(&mut blob);
    let x_bits_per_pixel = long(&mut blob) as f32;
    let _y_bits_per_pixel = long(&mut blob);
    let location_type = long(&mut blob);
    let _location_dimension = long(&mut blob);
    let number_of_images = long(&mut blob);
    let bands = long(&mut blob);
    let storage = long(&mut blob);
    let encode_scheme = long(&mut blob);
    let mut map_scheme = long(&mut blob);
    let map_storage = long(&mut blob);
    let map_rows = long(&mut blob);
    let map_columns = long(&mut blob);
    let _map_subrows = long(&mut blob);
    let _map_enable = long(&mut blob);
    let _maps_per_cycle = long(&mut blob);
    let color_space_model = long(&mut blob);
    blob.read(420);
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    let number_pixels = viff_columns as u64 * viff_rows as u64;
    if number_pixels > 8 * blob.size() {
        bail!("InsufficientImageDataInFile");
    }
    if number_pixels == 0 {
        bail!("ImageColumnOrRowSizeIsNotSupported");
    }
    let columns = viff_rows as usize;
    let rows = viff_columns as usize;
    let depth = if x_bits_per_pixel <= 8.0 { 8 } else { 16 };
    let alpha = bands == 4;
    if !(1..=4).contains(&bands) {
        bail!("ImproperImageHeader");
    }
    if !matches!(storage, TYP_BIT | TYP_1_BYTE | TYP_2_BYTE | TYP_4_BYTE | TYP_FLOAT | TYP_DOUBLE) {
        bail!("DataStorageTypeIsNotSupported");
    }
    if encode_scheme != 0 {
        bail!("DataEncodingSchemeIsNotSupported");
    }
    if !matches!(map_storage, 0 | 1 | 2 | 4 | 5 | MAPTYP_DOUBLE) {
        bail!("MapStorageTypeIsNotSupported");
    }
    if !matches!(color_space_model, 0 | 1 | 15) {
        bail!("ColorspaceModelIsNotSupported");
    }
    if location_type != 1 {
        bail!("LocationTypeIsNotSupported");
    }
    if number_of_images != 1 {
        bail!("NumberOfImagesIsNotSupported");
    }
    if map_rows == 0 {
        map_scheme = MS_NONE;
    }

    let mut colormap: Option<Vec<[f64; 4]>> = None;
    match map_scheme {
        MS_NONE => {
            if bands < 3 {
                let colors = if storage == TYP_BIT {
                    2
                } else if storage == 1 || depth <= 8 {
                    256
                } else {
                    65536
                };
                colormap = Some(colormap::acquire(colors).context("MemoryAllocationFailed")?);
            }
        }
        MS_ONEPERBAND | MS_SHARED => {
            let entry_bytes = match map_storage {
                2 => 2,
                4 | 5 => 4,
                MAPTYP_DOUBLE => 8,
                _ => 1,
            };
            let colors = map_columns as usize;
            let count = entry_bytes * colors * map_rows as usize;
            if count as u64 > blob.size() {
                bail!("InsufficientImageDataInFile");
            }
            let mut entries = colormap::acquire(colors).context("MemoryAllocationFailed")?;
            let colors = entries.len();
            let map = blob.read(count);
            if map.len() != count {
                bail!("InsufficientImageDataInFile");
            }
            let map_type = match map_storage {
                2 => TYP_2_BYTE,
                4 => TYP_4_BYTE,
                5 => TYP_FLOAT,
                MAPTYP_DOUBLE => TYP_DOUBLE,
                _ => TYP_1_BYTE,
            };
            for i in 0..map_rows as usize * colors {
                let value = typed_value(map, i, map_type, TYP_DOUBLE, big_endian);
                let quantum = char_quantum(value as u8) as f64;
                if i < colors {
                    entries[i][0] = quantum;
                    entries[i][1] = quantum;
                    entries[i][2] = quantum;
                } else if i < 2 * colors {
                    entries[i % colors][1] = quantum;
                } else if i < 3 * colors {
                    entries[i % colors][2] = quantum;
                }
            }
            colormap = Some(entries);
        }
        _ => bail!("ColormapTypeNotSupported"),
    }
    let bit = storage == TYP_BIT;
    if bit {
        colormap = Some(colormap::acquire(2).context("MemoryAllocationFailed")?);
    }

    let bytes_per_pixel = match storage {
        TYP_2_BYTE => 2,
        TYP_4_BYTE | TYP_FLOAT => 4,
        TYP_DOUBLE => 8,
        _ => 1,
    };
    let max_packets = if bit { columns.div_ceil(8) * rows } else { number_pixels as usize * bands as usize };
    let count = bytes_per_pixel * max_packets;
    if count as u64 > blob.size() {
        bail!("ImproperImageHeader");
    }
    let raw = blob.read(count);
    if raw.len() != count {
        bail!("ImproperImageHeader");
    }
    let mut min_value = 0.0;
    let mut scale_factor = 1.0;
    if storage != TYP_1_BYTE && map_scheme == MS_NONE {
        let first = typed_value(raw, 0, storage, TYP_DOUBLE, big_endian);
        let mut max_value = first;
        min_value = first;
        for i in 0..max_packets {
            let value = typed_value(raw, i, storage, TYP_DOUBLE, big_endian);
            if value > max_value {
                max_value = value;
            } else if value < min_value {
                min_value = value;
            }
        }
        if min_value == 0.0 && max_value == 0.0 {
            scale_factor = 0.0;
        } else if min_value == max_value {
            scale_factor = QUANTUM_RANGE / min_value;
            min_value = 0.0;
        } else {
            scale_factor = QUANTUM_RANGE / (max_value - min_value);
        }
    }
    let mut packets = vec![0u8; max_packets.max(number_pixels as usize) * bytes_per_pixel];
    for (i, packet) in packets.iter_mut().take(max_packets).enumerate() {
        let mut value = typed_value(raw, i, storage, TYP_DOUBLE, big_endian);
        if map_scheme == MS_NONE {
            value = (value - min_value) * scale_factor;
        }
        *packet = float_to_byte(value as f32);
    }
    // Bytes the conversion didn't touch keep the read's raw bytes.
    for (i, packet) in packets.iter_mut().enumerate().skip(max_packets) {
        *packet = raw.get(i).copied().unwrap_or(0);
    }

    let pseudo = colormap.is_some();
    let gray = bit;
    let mut channels = if gray { vec![PixelChannel::Gray] } else { vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue] };
    if alpha {
        channels.push(PixelChannel::Alpha);
    }
    if pseudo {
        channels.push(PixelChannel::Index);
    }
    let stride = channels.len();
    let index_offset = stride - 1;
    let mut data = vec![65535.0f32; columns * rows * stride];
    if bit {
        let mut p = 0;
        for y in 0..rows {
            for x in 0..columns {
                let byte = packets[p + x / 8];
                let quantum = if byte & (1 << (x % 8)) != 0 { 0.0 } else { 1.0 };
                let pixel = &mut data[(y * columns + x) * stride..(y * columns + x + 1) * stride];
                pixel[0] = if quantum == 0.0 { 0.0 } else { 65535.0 };
                pixel[index_offset] = quantum;
            }
            p += columns.div_ceil(8);
        }
    } else if pseudo {
        for (pixel, &packet) in data.chunks_exact_mut(stride).zip(packets.iter()) {
            pixel[index_offset] = packet as f32;
        }
    } else {
        let n = number_pixels as usize;
        for (p, pixel) in data.chunks_exact_mut(stride).enumerate() {
            pixel[0] = char_quantum(packets[p]);
            pixel[1] = char_quantum(packets.get(p + n).copied().unwrap_or(0));
            pixel[2] = char_quantum(packets.get(p + 2 * n).copied().unwrap_or(0));
            if alpha {
                pixel[3] = char_quantum(packets.get(p + 3 * n).copied().unwrap_or(0));
            }
        }
    }
    let mut colorspace = CacheColorspace::SRgb;
    if let Some(entries) = &colormap
        && !bit {
            let alpha_offset = alpha.then_some(3);
            colormap::sync(&mut data, stride, index_offset, alpha_offset, entries);
        }
    if bit {
        colorspace = CacheColorspace::Gray;
    }
    let mut cache = PixelCache::new(columns, rows, channels, data, colorspace, DEFAULT_GAMMA);
    if let Some(entries) = colormap {
        cache.colormap = entries;
    }
    Ok(cache)
}
