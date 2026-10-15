//! MATLAB matrices as ImageMagick reads them (`coders/mat.c`): level 5
//! files (each `miMATRIX` object, zlib-compressed or not: 2D grey or 3D
//! RGB, integer, logical or float, complex parts tinting the image) and
//! level 4 files (the first matrix). Every matrix is read, columns as rows,
//! then turned a quarter clockwise; the first is the image, but any matrix
//! IM can't read fails the file.
//!
//! IM's quirks are kept: float samples are offset by their minimum but not
//! scaled to their range (level 4: not scaled at all), signed integers are
//! shifted by half the quantum range once per colour channel (three times
//! on grey), complex parts and signed shifts step through the pixels one
//! sample at a time, and compressed objects' extremes are read big-endian.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::quantum::{self, Endian, Picture, QuantumInfo, QuantumType};
use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

const QUANTUM_RANGE: f64 = 65535.0;
const MI_INT8: u32 = 1;
const MI_UINT8: u32 = 2;
const MI_INT16: u32 = 3;
const MI_UINT16: u32 = 4;
const MI_INT32: u32 = 5;
const MI_UINT32: u32 = 6;
const MI_SINGLE: u32 = 7;
const MI_DOUBLE: u32 = 9;
const MI_INT64: u32 = 12;
const MI_UINT64: u32 = 13;
const MI_MATRIX: u32 = 14;
const MI_COMPRESSED: u32 = 15;
const FLAG_COMPLEX: u32 = 0x8;
const FLAG_LOGICAL: u32 = 0x2;
/// The classes IM reads: char, double, single and the integer ones.
const READABLE_CLASSES: [u32; 11] = [4, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
/// `MagickMinBufferExtent`: the compressed bytes read at a time.
const MIN_BUFFER_EXTENT: usize = 16384;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

/// A matrix being filled: grey (one sample a pixel) or RGB (three).
struct Matrix {
    columns: usize,
    rows: usize,
    gray: bool,
    data: Vec<f32>,
}

impl Matrix {
    fn stride(&self) -> usize {
        if self.gray { 1 } else { 3 }
    }

    /// `RotateImage(90)`: a quarter turn clockwise.
    fn rotated(&self) -> Matrix {
        let stride = self.stride();
        let mut data = vec![0.0f32; self.data.len()];
        for y in 0..self.rows {
            for x in 0..self.columns {
                let source = (y * self.columns + x) * stride;
                let target = (x * self.rows + (self.rows - 1 - y)) * stride;
                data[target..target + stride].copy_from_slice(&self.data[source..source + stride]);
            }
        }
        Matrix { columns: self.rows, rows: self.columns, gray: self.gray, data }
    }

    fn into_cache(self) -> PixelCache {
        let (channels, colorspace) = if self.gray {
            (vec![PixelChannel::Gray], CacheColorspace::Gray)
        } else {
            (vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue], CacheColorspace::SRgb)
        };
        PixelCache::new(self.columns, self.rows, channels, self.data, colorspace, DEFAULT_GAMMA)
    }
}

fn read_long(blob: &mut Blob, big_endian: bool) -> u32 {
    if big_endian { blob.msb_u32() } else { blob.lsb_u32() }
}

fn read_short(blob: &mut Blob, big_endian: bool) -> u16 {
    if big_endian { blob.msb_u16() } else { blob.lsb_u16() }
}

/// `ReadBlobDouble` in the blob's byte order (0 when cut short).
fn read_double(blob: &mut Blob, big_endian: bool) -> f64 {
    let bytes = blob.read(8);
    if bytes.len() != 8 {
        return 0.0;
    }
    let array: [u8; 8] = bytes.try_into().unwrap();
    if big_endian { f64::from_be_bytes(array) } else { f64::from_le_bytes(array) }
}

/// `ReadBlobFloat` in the blob's byte order (0 when cut short).
fn read_float(blob: &mut Blob, big_endian: bool) -> f32 {
    f32::from_bits(read_long(blob, big_endian))
}

/// `ReadBlobDoublesXXX` / `ReadBlobFloatsXXX`: whole values of `length`
/// bytes into the buffer, the rest skipped.
fn read_values(blob: &mut Blob, big_endian: bool, length: usize, double: bool, buffer: &mut [u8]) {
    let size = if double { 8 } else { 4 };
    let mut remaining = length;
    let mut index = 0;
    while remaining >= size {
        if double {
            let value = read_double(blob, big_endian);
            buffer[index * 8..index * 8 + 8].copy_from_slice(&value.to_ne_bytes());
        } else {
            let value = read_float(blob, big_endian);
            buffer[index * 4..index * 4 + 4].copy_from_slice(&value.to_ne_bytes());
        }
        index += 1;
        remaining -= size;
    }
    if remaining > 0 {
        let target = blob.tell() + remaining as u64;
        blob.seek(target as i64);
    }
}

fn buffer_double(buffer: &[u8], index: usize) -> f64 {
    f64::from_ne_bytes(buffer[index * 8..index * 8 + 8].try_into().unwrap())
}

fn buffer_float(buffer: &[u8], index: usize) -> f32 {
    f32::from_ne_bytes(buffer[index * 4..index * 4 + 4].try_into().unwrap())
}

/// `CalcMinMax`: the extremes of every row's first `size_x` values, the
/// blob put back where it was.
#[allow(clippy::too_many_arguments)]
fn min_max(blob: &mut Blob, big_endian: bool, size_x: usize, size_y: usize, cell_type: u32, row_length: usize, buffer: &mut [u8]) -> (f64, f64) {
    let start = blob.tell();
    let mut minimum = 0.0;
    let mut maximum = 0.0;
    for row in 0..size_y {
        if cell_type == MI_DOUBLE {
            read_values(blob, big_endian, row_length, true, buffer);
            if row == 0 {
                minimum = buffer_double(buffer, 0);
                maximum = minimum;
            }
            for x in 0..size_x {
                let value = buffer_double(buffer, x);
                if minimum > value {
                    minimum = value;
                }
                if maximum < value {
                    maximum = value;
                }
            }
        }
        if cell_type == MI_SINGLE {
            read_values(blob, big_endian, row_length, false, buffer);
            if row == 0 {
                minimum = buffer_float(buffer, 0) as f64;
                maximum = minimum;
            }
            for x in 0..size_x {
                let value = buffer_float(buffer, x) as f64;
                if minimum > value {
                    minimum = value;
                }
                if maximum < value {
                    maximum = value;
                }
            }
        }
    }
    blob.seek(start as i64);
    (minimum, maximum)
}

/// `FixSignedValues`: half the quantum range and one more added to the
/// red, green and blue of `count` pixels, stepping one sample at a time
/// (grey: the same sample three times).
fn fix_signed(matrix: &mut Matrix, start: usize, count: usize) {
    let half = (QUANTUM_RANGE as f32) / 2.0;
    let (red, green, blue) = if matrix.gray { (0, 0, 0) } else { (0, 1, 2) };
    for q in start..start + count {
        for offset in [red, green, blue] {
            let sample = &mut matrix.data[q + offset];
            *sample = (*sample + half) + 1.0;
        }
    }
}

/// `FixLogical`: a row of bytes packed into bits, most significant first.
fn fix_logical(buffer: &mut [u8], length: usize) {
    if length == 0 {
        return;
    }
    let mut mask = 128u8;
    let mut value = 0u8;
    let mut out = 0;
    for index in 0..length {
        if buffer[index] != 0 {
            value |= mask;
        }
        mask >>= 1;
        if mask == 0 {
            buffer[out] = value;
            out += 1;
            value = 0;
            mask = 128;
        }
    }
    buffer[out] = value;
}

/// The red, green and blue offsets of the sample at `q`.
fn rgb_offsets(matrix: &Matrix) -> (usize, usize, usize) {
    if matrix.gray { (0, 0, 0) } else { (0, 1, 2) }
}

/// `InsertComplexDoubleRow`: positive parts push red up and green and blue
/// down, negative ones blue up and red and green down; one sample a pixel.
fn insert_complex_double(matrix: &mut Matrix, values: &[f64], y: usize, mut minimum: f64, mut maximum: f64) {
    if minimum >= 0.0 {
        minimum = -1.0;
    }
    if maximum <= 0.0 {
        maximum = 1.0;
    }
    let (r, g, b) = rgb_offsets(matrix);
    let range = QUANTUM_RANGE as f32;
    let row_start = y * matrix.columns * matrix.stride();
    for (column, &value) in values.iter().take(matrix.columns).enumerate() {
        // ImageMagick steps one value, not one pixel, a column.
        let q = row_start + column;
        let data = &mut matrix.data;
        if value > 0.0 {
            let mut f = (value / maximum) * (QUANTUM_RANGE - data[q + r] as f64);
            if f.is_nan() {
                f = 0.0;
            }
            if f + data[q + r] as f64 >= QUANTUM_RANGE {
                data[q + r] = range;
            } else {
                data[q + r] += f as f32;
            }
            let mut f = data[q + g] as f64 - f / 2.0;
            if f.is_nan() {
                f = 0.0;
            }
            if f <= 0.0 {
                data[q + g] = 0.0;
                data[q + b] = 0.0;
            } else {
                data[q + b] = f as f32;
                data[q + g] = f as f32;
            }
        }
        if value < 0.0 {
            let mut f = (value / minimum) * (range - data[q + b]) as f64;
            if f.is_nan() {
                f = 0.0;
            }
            if f + data[q + b] as f64 >= QUANTUM_RANGE {
                data[q + b] = range;
            } else {
                data[q + b] += f as f32;
            }
            let f = data[q + g] as f64 - f / 2.0;
            if f <= 0.0 {
                data[q + r] = 0.0;
                data[q + g] = 0.0;
            } else {
                data[q + r] = f as f32;
                data[q + g] = f as f32;
            }
        }
    }
}

/// `InsertComplexFloatRow`: as the double one but halving differently, and
/// scaling negative parts by the maximum too (the minimum goes unused).
fn insert_complex_float(matrix: &mut Matrix, values: &[f32], y: usize, mut maximum: f64) {
    if maximum <= 0.0 {
        maximum = 1.0;
    }
    let (r, g, b) = rgb_offsets(matrix);
    let range = QUANTUM_RANGE as f32;
    let row_start = y * matrix.columns * matrix.stride();
    for (column, &value) in values.iter().take(matrix.columns).enumerate() {
        // ImageMagick steps one value, not one pixel, a column.
        let q = row_start + column;
        let data = &mut matrix.data;
        if value > 0.0 {
            let mut f = (value as f64 / maximum) * (QUANTUM_RANGE - data[q + r] as f64);
            if f.is_nan() {
                f = 0.0;
            }
            if f + (data[q + r] as f64) < QUANTUM_RANGE {
                data[q + r] += f as f32;
            } else {
                data[q + r] = range;
            }
            f /= 2.0;
            if f < data[q + g] as f64 {
                data[q + b] -= f as f32;
                data[q + g] = data[q + b];
            } else {
                data[q + g] = 0.0;
                data[q + b] = 0.0;
            }
        }
        if value < 0.0 {
            let mut f = (value as f64 / maximum) * (range - data[q + b]) as f64;
            if f.is_nan() {
                f = 0.0;
            }
            if f + (data[q + b] as f64) < QUANTUM_RANGE {
                data[q + b] += f as f32;
            } else {
                data[q + b] = range;
            }
            f /= 2.0;
            if f < data[q + g] as f64 {
                data[q + r] -= f as f32;
                data[q + g] = data[q + r];
            } else {
                data[q + g] = 0.0;
                data[q + r] = 0.0;
            }
        }
    }
}

/// Imports a row into the matrix through a picture over its samples.
fn import_row(matrix: &mut Matrix, info: &QuantumInfo, kind: QuantumType, row: usize, bytes: &[u8]) -> Result<()> {
    let mut picture = Picture::new(matrix.columns, 1, matrix.gray, false, false, false, 0);
    let stride = matrix.stride();
    let start = row * matrix.columns * stride;
    picture.data.copy_from_slice(&matrix.data[start..start + matrix.columns * stride]);
    quantum::import(&mut picture, info, kind, 0, matrix.columns, bytes)?;
    matrix.data[start..start + matrix.columns * stride].copy_from_slice(&picture.data);
    Ok(())
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    if bytes.len() < 124 {
        bail!("ImproperImageHeader");
    }
    if !bytes.starts_with(b"MATLAB") {
        return decode_v4(bytes);
    }
    let mut blob = Blob::new(bytes);
    blob.seek(124);
    let _version = blob.lsb_u16();
    let indicator = blob.read(2);
    let big_endian = match indicator {
        b"IM" => false,
        b"MI" => true,
        _ => bail!("ImproperImageHeader"),
    };
    let size = blob.size();
    let mut images: Vec<Matrix> = Vec::new();
    let mut filepos = blob.tell();
    while filepos < size && !blob.eof {
        blob.seek(filepos as i64);
        let data_type = read_long(&mut blob, big_endian);
        if blob.eof {
            break;
        }
        let object_size = read_long(&mut blob, big_endian);
        if blob.eof {
            break;
        }
        if object_size as u64 + filepos >= size {
            bail!("ImproperImageHeader");
        }
        filepos += object_size as u64 + 8;

        // A compressed object reads from its inflated bytes, whose own
        // byte order IM leaves undefined (read big-endian).
        if data_type == MI_COMPRESSED {
            let inflated = inflate_object(&mut blob, object_size);
            let mut inner_blob = Blob::new(&inflated);
            if read_long(&mut inner_blob, big_endian) != MI_MATRIX {
                continue;
            }
            read_matrix(&mut inner_blob, true, big_endian, inflated.len() as u32, filepos, &mut images)?;
        } else {
            if data_type != MI_MATRIX {
                continue;
            }
            read_matrix(&mut blob, false, big_endian, object_size, filepos, &mut images)?;
        }
        if blob.eof {
            break;
        }
    }
    let first = images.into_iter().find(|matrix| matrix.columns != 0 && matrix.rows != 0);
    let Some(first) = first else {
        bail!("ImproperImageHeader");
    };
    Ok(first.into_cache())
}

/// `decompress_block`: the object's bytes inflated a buffer at a time until
/// the stream ends or breaks.
fn inflate_object(blob: &mut Blob, object_size: u32) -> Vec<u8> {
    let mut decoder = flate2::Decompress::new(true);
    let mut output = Vec::new();
    let mut remaining = object_size as usize;
    let mut chunk = [0u8; 4096];
    'outer: while remaining > 0 && !blob.eof {
        let input = blob.read(remaining.min(MIN_BUFFER_EXTENT));
        if input.is_empty() {
            break;
        }
        let mut consumed = 0;
        while consumed < input.len() {
            let before_in = decoder.total_in();
            let before_out = decoder.total_out();
            let status = decoder.decompress(&input[consumed..], &mut chunk, flate2::FlushDecompress::None);
            consumed += (decoder.total_in() - before_in) as usize;
            let produced = (decoder.total_out() - before_out) as usize;
            output.extend_from_slice(&chunk[..produced]);
            match status {
                Ok(flate2::Status::StreamEnd) => break 'outer,
                Ok(_) if produced == 0 && decoder.total_in() == before_in => break 'outer,
                Ok(_) => {}
                Err(_) => break 'outer,
            }
        }
        remaining -= input.len();
    }
    output
}

/// One `miMATRIX` object (its tag read): its frames appended to `images`.
fn read_matrix(source: &mut Blob, compressed: bool, big_endian: bool, object_size: u32, filepos: u64, images: &mut Vec<Matrix>) -> Result<()> {
    let _array_flags_type = read_long(source, big_endian);
    let _array_flags_size = read_long(source, big_endian);
    let flags = read_long(source, big_endian) as u16 as u32;
    let structure_class = flags & 0xff;
    let structure_flag = (flags >> 8) & 0xff;
    let _nzmax = read_long(source, big_endian);
    if compressed {
        let _ = read_long(source, big_endian);
    }
    let _dimensions_type = read_long(source, big_endian);
    let dimension_flag = read_long(source, big_endian);
    let size_x = read_long(source, big_endian) as usize;
    let size_y = read_long(source, big_endian) as usize;
    let mut frames = 1u32;
    let z2;
    match dimension_flag {
        8 => z2 = 1,
        12 => {
            z2 = read_long(source, big_endian);
            let _ = read_long(source, big_endian);
            if z2 != 3 {
                bail!("MultidimensionalMatricesAreNotSupported");
            }
        }
        16 => {
            z2 = read_long(source, big_endian);
            if z2 != 3 && z2 != 1 {
                bail!("MultidimensionalMatricesAreNotSupported");
            }
            frames = read_long(source, big_endian);
            if frames == 0 {
                bail!("ImproperImageHeader");
            }
        }
        _ => bail!("MultidimensionalMatricesAreNotSupported"),
    }
    let _flag1 = read_short(source, big_endian);
    let name_flag = read_short(source, big_endian);
    if !READABLE_CLASSES.contains(&structure_class) {
        bail!("UnsupportedCellTypeInTheMatrix");
    }
    match name_flag {
        0 => {
            let length = read_long(source, big_endian) as u64;
            let skip = 4 * ((length + 3 + 1) / 4);
            let target = source.tell() + skip;
            source.seek(target as i64);
        }
        1..=4 => {
            source.read(4);
        }
        _ => bail!("ImproperImageHeader"),
    }
    let mut cell_type = read_long(source, big_endian);
    if source.read(4).len() != 4 {
        bail!("ImproperImageHeader");
    }
    // Minima and maxima are read in the blob's byte order: the header's,
    // or big-endian for an inflated object.
    let values_big_endian = compressed || big_endian;
    let gray = dimension_flag == 8 && structure_flag & FLAG_COMPLEX == 0;
    loop {
        let (depth, row_length, float) = match cell_type {
            MI_INT8 | MI_UINT8 => (if structure_flag & FLAG_LOGICAL != 0 { 1 } else { 8 }, size_x, false),
            MI_INT16 | MI_UINT16 => (16, 2 * size_x, false),
            MI_INT32 | MI_UINT32 => (32, 4 * size_x, false),
            MI_INT64 | MI_UINT64 => (64, 8 * size_x, false),
            MI_SINGLE => (32, 4 * size_x, true),
            MI_DOUBLE => (64, 8 * size_x, true),
            _ => bail!("UnsupportedCellTypeInTheMatrix"),
        };
        if size_x == 0 || size_y == 0 {
            bail!("ImproperImageHeader");
        }
        if (row_length as u64).saturating_mul(size_y as u64) > object_size as u64 {
            bail!("ImproperImageHeader");
        }
        // `SetImageBackgroundColor`: white.
        let samples = size_x * size_y * if gray { 1 } else { 3 };
        let mut matrix = Matrix { columns: size_x, rows: size_y, gray, data: vec![QUANTUM_RANGE as f32; samples] };
        let mut info = QuantumInfo::new(depth, float);
        info.endian = if big_endian { Endian::Msb } else { Endian::Lsb };
        let mut buffer = vec![0u8; row_length * 8];
        if cell_type == MI_DOUBLE || cell_type == MI_SINGLE {
            let (minimum, _maximum) = min_max(source, values_big_endian, size_x, size_y, cell_type, row_length, &mut buffer);
            info.minimum = minimum;
        }
        let kinds = [QuantumType::Gray, QuantumType::Blue, QuantumType::Green, QuantumType::Red];
        let mut z = if z2 == 1 { 0 } else { z2 as usize };
        let logical = matches!(cell_type, MI_INT8 | MI_UINT8) && structure_flag & FLAG_LOGICAL != 0;
        let signed = matches!(cell_type, MI_INT8 | MI_INT16 | MI_INT32 | MI_INT64);
        loop {
            for i in 0..size_y {
                let row = size_y - i - 1;
                let read = source.read(row_length);
                if read.len() != row_length {
                    bail!("UnexpectedEndOfFile");
                }
                buffer[..row_length].copy_from_slice(read);
                if logical {
                    fix_logical(&mut buffer, row_length);
                }
                import_row(&mut matrix, &info, kinds[z], row, &buffer)?;
                if !logical && z <= 1 && signed {
                    let start = row * size_x * matrix.stride();
                    fix_signed(&mut matrix, start, size_x);
                }
            }
            let again = z >= 2;
            z = z.wrapping_sub(1);
            if !again {
                break;
            }
        }

        if structure_flag & FLAG_COMPLEX != 0 {
            cell_type = read_long(source, big_endian);
            let _complex_size = read_long(source, big_endian);
            let (mut minimum, mut maximum) = (0.0, 0.0);
            if cell_type == MI_DOUBLE || cell_type == MI_SINGLE {
                (minimum, maximum) = min_max(source, values_big_endian, size_x, size_y, cell_type, row_length, &mut buffer);
            }
            if cell_type == MI_DOUBLE {
                for i in 0..size_y {
                    read_values(source, values_big_endian, row_length, true, &mut buffer);
                    if source.eof && !compressed {
                        break;
                    }
                    let values: Vec<f64> = (0..size_x).map(|x| buffer_double(&buffer, x)).collect();
                    insert_complex_double(&mut matrix, &values, i, minimum, maximum);
                }
            }
            if cell_type == MI_SINGLE {
                for i in 0..size_y {
                    read_values(source, values_big_endian, row_length, false, &mut buffer);
                    if source.eof && !compressed {
                        break;
                    }
                    let values: Vec<f32> = (0..size_x).map(|x| buffer_float(&buffer, x)).collect();
                    insert_complex_float(&mut matrix, &values, i, maximum);
                }
            }
        }
        images.push(matrix.rotated());
        if source.eof && !compressed {
            break;
        }
        frames -= 1;
        if frames == 0 || compressed || source.eof || source.tell() >= filepos {
            break;
        }
    }
    Ok(())
}

/// A level 4 file's first matrix: a type code (byte order, then sample
/// type: double, single, then 16-bit unsigned and signed, else bytes),
/// the size and name, then the samples a column at a time.
fn decode_v4(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let code = blob.lsb_u32();
    if blob.eof || code > 9999 {
        bail!("ImproperImageHeader");
    }
    let mut code = code;
    let t = code % 10;
    code /= 10;
    let p = code % 10;
    code /= 10;
    let o = code % 10;
    code /= 10;
    let m = code;
    if t != 0 || p != 0 {
        bail!("ImproperImageHeader");
    }
    let big_endian = m != 0;
    let rows_count = read_long(&mut blob, big_endian);
    let columns_count = read_long(&mut blob, big_endian);
    let imaginary = read_long(&mut blob, big_endian);
    let name_length = read_long(&mut blob, big_endian);
    if imaginary != 0 && imaginary != 1 {
        bail!("ImproperImageHeader");
    }
    if name_length > 0xffff {
        bail!("ImproperImageHeader");
    }
    for _ in 0..name_length {
        if blob.byte().is_none() {
            bail!("UnexpectedEndOfFile");
        }
    }
    let columns = rows_count as usize;
    let rows = columns_count as usize;
    if columns == 0 || rows == 0 {
        bail!("ImproperImageHeader");
    }
    let (depth, float) = match o {
        0 => (64, true),
        1 => (32, true),
        2 | 3 => (16, false),
        _ => (8, false),
    };
    let row_length = depth / 8 * columns;
    // Rows past the end of the file fail the read.
    let needed = row_length as u64 * rows as u64 * (1 + imaginary as u64);
    if needed > blob.size().saturating_sub(blob.tell()) {
        bail!("UnexpectedEndOfFile");
    }
    let mut info = QuantumInfo::new(depth as u32, float);
    info.endian = if big_endian { Endian::Msb } else { Endian::Lsb };
    info.scale = 1.0;
    let mut matrix = Matrix { columns, rows, gray: true, data: vec![QUANTUM_RANGE as f32; columns * rows] };
    // IM's quantum buffer, reused by every row.
    let mut buffer = vec![0u8; columns.max(rows) * 8];
    for y in 0..rows {
        let read = blob.read(row_length);
        buffer[..read.len()].copy_from_slice(read);
        import_row(&mut matrix, &info, QuantumType::Gray, rows - y - 1, &buffer[..row_length])?;
        if o == 2 || o == 3 {
            fix_signed(&mut matrix, (rows - y - 1) * columns, columns);
        }
    }
    if imaginary == 1 {
        for y in 0..rows {
            let read = blob.read(row_length);
            buffer[..read.len()].copy_from_slice(read);
            if o == 0 {
                let values: Vec<f64> = (0..columns).map(|x| buffer_double(&buffer, x)).collect();
                insert_complex_double(&mut matrix, &values, y, 0.0, 0.0);
            } else {
                let values: Vec<f32> = (0..columns).map(|x| buffer_float(&buffer, x)).collect();
                insert_complex_float(&mut matrix, &values, y, 0.0);
            }
        }
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    Ok(matrix.rotated().into_cache())
}
