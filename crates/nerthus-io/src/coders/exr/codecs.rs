//! OpenEXRCore 3.4.9's decompressors (`internal_rle.c`, `internal_zip.c`,
//! `internal_piz.c`, `internal_pxr24.c`, `internal_b44.c`, with DWA in
//! [`super::dwa`]), each giving a chunk's samples in the file's layout:
//! for each line, for each channel that has a sample on it, the channel's
//! samples, little-endian. Channels that are subsampled have fewer samples
//! per line and lines.
//!
//! zlib streams are inflated as libdeflate does: the stream must use up
//! its input exactly and fit the buffer it is given, and may decode to
//! less than the buffer holds.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use anyhow::{anyhow, bail, Result};
use flate2::{Decompress, FlushDecompress, Status};

use super::header::{COMPRESSION_B44, COMPRESSION_B44A, COMPRESSION_DWAA, COMPRESSION_DWAB, COMPRESSION_NONE, COMPRESSION_PIZ, COMPRESSION_PXR24, COMPRESSION_RLE, COMPRESSION_ZIP, COMPRESSION_ZIPS, PIXEL_FLOAT, PIXEL_HALF, PIXEL_UINT};

/// A channel of the chunk being decoded (`exr_coding_channel_info_t`): its
/// samples in this chunk.
#[derive(Clone, Debug)]
pub(super) struct CodingChannel {
    pub name: Vec<u8>,
    pub width: i32,
    pub height: i32,
    pub x_samples: i32,
    pub y_samples: i32,
    pub p_linear: bool,
    pub bytes_per_element: usize,
    pub data_type: u8,
}

/// The chunk's place: the first line, and its size.
#[derive(Clone, Copy, Debug)]
pub(super) struct ChunkArea {
    pub start_y: i32,
    pub width: i32,
    pub height: i32,
}

fn corrupt() -> anyhow::Error {
    anyhow!("Unable to decompress image data: corrupt chunk")
}

/// Whether a sample of channel `channel` lies on line `y` of the file.
fn has_line(channel: &CodingChannel, y: i32) -> bool {
    channel.y_samples <= 1 || y % channel.y_samples == 0
}

/// `exr_uncompress_buffer` (libdeflate's `libdeflate_zlib_decompress_ex`):
/// the stream inflated into at most `capacity` bytes.
pub(super) fn inflate(input: &[u8], capacity: usize) -> Result<Vec<u8>> {
    let mut decompressor = Decompress::new(true);
    let mut output = vec![0u8; capacity];
    let status = decompressor.decompress(input, &mut output, FlushDecompress::Finish).map_err(|_| corrupt())?;
    let written = decompressor.total_out() as usize;
    match status {
        Status::StreamEnd if decompressor.total_in() as usize == input.len() => {
            output.truncate(written);
            Ok(output)
        }
        // Output space ran out before the stream ended.
        _ if written == capacity && decompressor.total_in() as usize != input.len() => Err(anyhow!("Unable to decompress image data: out of memory")),
        _ => Err(corrupt()),
    }
}

/// `exr_compress_max_buffer_size`: the scratch space PXR24 inflates into.
fn compress_max_buffer_size(bytes: usize) -> usize {
    let blocks = bytes.div_ceil(5000).max(1);
    let mut size = 6 + 5 * blocks + bytes + 9;
    let padded = size * 130 / 128;
    if padded > size {
        size = padded;
    }
    // The Huffman coder's spare space.
    let huffman_spare = 65537 * (8 + 8 + 8 + 4);
    size = size.max(bytes + huffman_spare);
    size.max(8192)
}

/// `internal_rle_decompress` then `unpredict_and_reorder`.
fn undo_rle(input: &[u8], expected: usize) -> Result<Vec<u8>> {
    let mut scratch = Vec::with_capacity(expected);
    let mut position = 0;
    while position < input.len() {
        let count = input[position] as i8;
        position += 1;
        if count < 0 {
            let run = (-(count as i32)) as usize;
            if position + run > input.len() || scratch.len() + run > expected {
                return Err(corrupt());
            }
            scratch.extend_from_slice(&input[position..position + run]);
            position += run;
        } else {
            if position + 1 > input.len() {
                return Err(corrupt());
            }
            let run = count as usize + 1;
            if scratch.len() + run > expected {
                return Err(corrupt());
            }
            let value = input[position];
            scratch.extend(std::iter::repeat_n(value, run));
            position += 1;
        }
    }
    if scratch.len() != expected {
        return Err(corrupt());
    }
    for index in 1..scratch.len() {
        scratch[index] = (scratch[index - 1] as i32 + scratch[index] as i8 as i32 - 128) as u8;
    }
    Ok(interleave(&scratch))
}

/// The predictor undone in place (`reconstruct`).
fn reconstruct(bytes: &mut [u8]) {
    for index in 1..bytes.len() {
        bytes[index] = (bytes[index - 1] as i32 + bytes[index] as i32 - 128) as u8;
    }
}

/// `interleave`: the first half's bytes and the second half's taken in
/// turn.
fn interleave(source: &[u8]) -> Vec<u8> {
    let half = source.len().div_ceil(2);
    let mut output = Vec::with_capacity(source.len());
    let (mut first, mut second) = (0, half);
    while output.len() < source.len() {
        output.push(source[first]);
        first += 1;
        if output.len() < source.len() {
            output.push(source[second]);
            second += 1;
        }
    }
    output
}

/// `internal_exr_undo_zip`: inflated, then the predictor and byte order
/// undone. A stream that inflates short fills only the front of `output`.
fn undo_zip(input: &[u8], output: &mut [u8]) -> Result<()> {
    if input.len() == output.len() {
        output.copy_from_slice(input);
        return Ok(());
    }
    let capacity = output.len().max(input.len());
    let mut inflated = inflate(input, capacity)?;
    if input.len() > inflated.len() || inflated.len() > output.len() {
        return Err(corrupt());
    }
    reconstruct(&mut inflated);
    let restored = interleave(&inflated);
    output[..restored.len()].copy_from_slice(&restored);
    Ok(())
}

/// `internal_exr_undo_pxr24`: each line's channels as byte planes of
/// differences (floats lose their low byte).
fn undo_pxr24(input: &[u8], output: &mut [u8], channels: &[CodingChannel], area: &ChunkArea) -> Result<()> {
    let scratch_size = compress_max_buffer_size(output.len());
    let inflated = inflate(input, scratch_size)?;
    let mut last_in = 0usize;
    let mut out = 0usize;
    let mut decoded = 0usize;
    for y in 0..area.height {
        let line = y + area.start_y;
        for channel in channels {
            let width = channel.width.max(0) as usize;
            let bytes = width * channel.bytes_per_element;
            if channel.height == 0 || !has_line(channel, line) {
                continue;
            }
            if out + bytes > output.len() {
                return Err(anyhow!("Unable to decompress image data: out of memory"));
            }
            match channel.data_type {
                PIXEL_UINT | PIXEL_FLOAT => {
                    let planes = if channel.data_type == PIXEL_UINT { 4 } else { 3 };
                    let needed = if channel.data_type == PIXEL_UINT { bytes } else { width * 3 };
                    if decoded + needed > inflated.len() {
                        return Err(corrupt());
                    }
                    let mut pixel = 0u32;
                    for x in 0..width {
                        let mut difference = 0u32;
                        for plane in 0..planes {
                            difference |= (inflated[last_in + plane * width + x] as u32) << (24 - 8 * plane);
                        }
                        pixel = pixel.wrapping_add(difference);
                        output[out + 4 * x..out + 4 * x + 4].copy_from_slice(&pixel.to_le_bytes());
                    }
                    last_in += planes * width;
                    decoded += needed;
                }
                PIXEL_HALF => {
                    if decoded + bytes > inflated.len() {
                        return Err(corrupt());
                    }
                    let mut pixel = 0u32;
                    for x in 0..width {
                        let difference = (inflated[last_in + x] as u32) << 8 | inflated[last_in + width + x] as u32;
                        pixel = pixel.wrapping_add(difference);
                        output[out + 2 * x..out + 2 * x + 2].copy_from_slice(&(pixel as u16).to_le_bytes());
                    }
                    last_in += 2 * width;
                    decoded += bytes;
                }
                _ => bail!("invalid argument"),
            }
            out += bytes;
        }
    }
    Ok(())
}

/// `wdec14`.
fn wdec14(l: u16, h: u16) -> (u16, u16) {
    let hi = h as i16 as i32;
    let li = l as i16 as i32;
    let ai = li + (hi & 1) + (hi >> 1);
    let a = ai as i16;
    let b = (ai - hi) as i16;
    (a as u16, b as u16)
}

/// `wdec14_4`: four values decoded at once.
fn wdec14_4(px: u16, p01: u16, p10: u16, p11: u16) -> (u16, u16, u16, u16) {
    let a = px as i16 as i32;
    let b = p10 as i16 as i32;
    let c = p01 as i16 as i32;
    let d = p11 as i16 as i32;
    let i00 = a + (b & 1) + (b >> 1);
    let i10 = i00 - b;
    let i01 = c + (d & 1) + (d >> 1);
    let i11 = i01 - d;
    let ai = i00 + (i01 & 1) + (i01 >> 1);
    let bi = ai - i01;
    let ci = i10 + (i11 & 1) + (i11 >> 1);
    let di = ci - i11;
    (ai as u16, bi as u16, ci as u16, di as u16)
}

const A_OFFSET: i32 = 1 << 15;
const MOD_MASK: i32 = (1 << 16) - 1;

/// `wdec16`.
fn wdec16(l: u16, h: u16) -> (u16, u16) {
    let m = l as i32;
    let d = h as i32;
    let bb = (m - (d >> 1)) & MOD_MASK;
    let aa = (d + bb - A_OFFSET) & MOD_MASK;
    (aa as u16, bb as u16)
}

/// `wav_2D_decode` on `data[start..]` with element step `ox` and row step
/// `oy`.
fn wavelet_decode(data: &mut [u16], start: usize, nx: i32, ox: i32, ny: i32, oy: i32, maximum: u16) {
    let w14 = maximum < (1 << 14);
    let n = nx.min(ny);
    let mut p = 1i64;
    while p <= n as i64 {
        p <<= 1;
    }
    p >>= 1;
    let mut p2 = p;
    p >>= 1;
    let (ox, oy) = (ox as i64, oy as i64);
    while p >= 1 {
        let mut py = start as i64;
        let ey = start as i64 + oy * (ny as i64 - p2);
        let oy1 = oy * p;
        let oy2 = oy * p2;
        let ox1 = ox * p;
        let ox2 = ox * p2;
        while py <= ey {
            let mut px = py;
            let ex = py + ox * (nx as i64 - p2);
            while px <= ex {
                let p01 = px + ox1;
                let p10 = px + oy1;
                let p11 = p10 + ox1;
                let (a, b, c, d) = (data[px as usize], data[p01 as usize], data[p10 as usize], data[p11 as usize]);
                if w14 {
                    let (n00, n01, n10, n11) = wdec14_4(a, b, c, d);
                    data[px as usize] = n00;
                    data[p01 as usize] = n01;
                    data[p10 as usize] = n10;
                    data[p11 as usize] = n11;
                } else {
                    let (i00, i10) = wdec16(a, c);
                    let (i01, i11) = wdec16(b, d);
                    let (n00, n01) = wdec16(i00, i01);
                    let (n10, n11) = wdec16(i10, i11);
                    data[px as usize] = n00;
                    data[p01 as usize] = n01;
                    data[p10 as usize] = n10;
                    data[p11 as usize] = n11;
                }
                px += ox2;
            }
            if nx as i64 & p != 0 {
                let p10 = px + oy1;
                let (first, second) = if w14 { wdec14(data[px as usize], data[p10 as usize]) } else { wdec16(data[px as usize], data[p10 as usize]) };
                data[px as usize] = first;
                data[p10 as usize] = second;
            }
            py += oy2;
        }
        if ny as i64 & p != 0 {
            let mut px = py;
            let ex = py + ox * (nx as i64 - p2);
            while px <= ex {
                let p01 = px + ox1;
                let (first, second) = if w14 { wdec14(data[px as usize], data[p01 as usize]) } else { wdec16(data[px as usize], data[p01 as usize]) };
                data[px as usize] = first;
                data[p01 as usize] = second;
                px += ox2;
            }
        }
        p2 = p;
        p >>= 1;
    }
}

/// `internal_exr_undo_piz`: the bitmap of values used, the Huffman-coded
/// wavelet data, each channel's wavelet undone, values mapped back, then
/// the samples put in line order.
fn undo_piz(input: &[u8], output: &mut [u8], channels: &[CodingChannel], area: &ChunkArea) -> Result<()> {
    const BITMAP_SIZE: usize = 8192;
    if input.len() < 4 {
        return Err(corrupt());
    }
    let min_non_zero = u16::from_le_bytes([input[0], input[1]]) as usize;
    let max_non_zero = u16::from_le_bytes([input[2], input[3]]) as usize;
    let mut position = 4;
    if max_non_zero >= BITMAP_SIZE {
        return Err(corrupt());
    }
    let mut bitmap = vec![0u8; BITMAP_SIZE];
    if min_non_zero <= max_non_zero {
        let count = max_non_zero - min_non_zero + 1;
        if position + count > input.len() {
            return Err(corrupt());
        }
        bitmap[min_non_zero..=max_non_zero].copy_from_slice(&input[position..position + count]);
        position += count;
    }
    // `reverseLutFromBitmap`.
    let mut lut = vec![0u16; 65536];
    let mut k = 0usize;
    for value in 0..65536usize {
        if value == 0 || bitmap[value >> 3] & (1 << (value & 7)) != 0 {
            lut[k] = value as u16;
            k += 1;
        }
    }
    let maximum = (k - 1) as u16;
    if position + 4 > input.len() {
        return Err(corrupt());
    }
    let huffman_bytes = u32::from_le_bytes([input[position], input[position + 1], input[position + 2], input[position + 3]]) as usize;
    position += 4;
    if position + huffman_bytes > input.len() {
        return Err(corrupt());
    }
    let mut values = super::dwa::huffman::decompress(&input[position..position + huffman_bytes], output.len() / 2)?;

    let mut start = 0usize;
    for channel in channels {
        let nx = channel.width;
        let ny = channel.height;
        let words = (channel.bytes_per_element / 2) as i32;
        if words > 0 && nx > i32::MAX / words {
            return Err(corrupt());
        }
        for word in 0..words {
            wavelet_decode(&mut values, start + word as usize, nx, words, ny, words * nx, maximum);
        }
        start += (nx as i64 * ny as i64 * words as i64).max(0) as usize;
    }
    for value in values.iter_mut() {
        *value = lut[*value as usize];
    }

    let mut out = 0usize;
    for y in 0..area.height {
        let line = y + area.start_y;
        let mut plane_start = 0usize;
        for channel in channels {
            let bytes = channel.width.max(0) as usize * channel.bytes_per_element;
            if bytes == 0 {
                continue;
            }
            let plane = plane_start;
            plane_start += channel.height.max(0) as usize * bytes;
            let row = if channel.y_samples > 1 {
                if line % channel.y_samples != 0 {
                    continue;
                }
                (y / channel.y_samples) as usize
            } else {
                y as usize
            };
            let first = (plane + row * bytes) / 2;
            for (index, value) in values[first..first + bytes / 2].iter().enumerate() {
                output[out + 2 * index..out + 2 * index + 2].copy_from_slice(&value.to_le_bytes());
            }
            out += bytes;
        }
    }
    if out != output.len() {
        return Err(corrupt());
    }
    Ok(())
}

/// `half_to_float` (Imath).
fn half_to_float(half: u16) -> f32 {
    super::dwa::half_to_float(half)
}

/// `float_to_half` (Imath, round to nearest even).
fn float_to_half(value: f32) -> u16 {
    super::dwa::float_to_half(value)
}

/// B44's logarithmic tables (`exrcore_logTable`), filled the first time a
/// B44 (not B44A) chunk is decoded in the process; until then they are
/// zero.
static B44_TABLES_READY: AtomicBool = AtomicBool::new(false);

fn log_table() -> &'static [u16] {
    static TABLE: OnceLock<Vec<u16>> = OnceLock::new();
    TABLE.get_or_init(|| {
        (0..=u16::MAX)
            .map(|x| {
                if x & 0x7c00 == 0x7c00 || x > 0x8000 {
                    return 0;
                }
                let value = 8.0 * half_to_float(x).ln();
                float_to_half(value)
            })
            .collect()
    })
}

/// `unpack14`: a 4 × 4 block from 14 bytes.
fn unpack14(b: &[u8]) -> [u16; 16] {
    let mut s = [0u16; 16];
    s[0] = (b[0] as u16) << 8 | b[1] as u16;
    let shift = (b[2] >> 2) as u32;
    let bias = 0x20u32 << shift;
    let step = |previous: u16, bits: u32| -> u16 { (previous as u32).wrapping_add(bits << shift).wrapping_sub(bias) as u16 };
    let b = |index: usize| b[index] as u32;
    s[4] = step(s[0], ((b(2) << 4) | (b(3) >> 4)) & 0x3f);
    s[8] = step(s[4], ((b(3) << 2) | (b(4) >> 6)) & 0x3f);
    s[12] = step(s[8], b(4) & 0x3f);
    s[1] = step(s[0], b(5) >> 2);
    s[5] = step(s[4], ((b(5) << 4) | (b(6) >> 4)) & 0x3f);
    s[9] = step(s[8], ((b(6) << 2) | (b(7) >> 6)) & 0x3f);
    s[13] = step(s[12], b(7) & 0x3f);
    s[2] = step(s[1], b(8) >> 2);
    s[6] = step(s[5], ((b(8) << 4) | (b(9) >> 4)) & 0x3f);
    s[10] = step(s[9], ((b(9) << 2) | (b(10) >> 6)) & 0x3f);
    s[14] = step(s[13], b(10) & 0x3f);
    s[3] = step(s[2], b(11) >> 2);
    s[7] = step(s[6], ((b(11) << 4) | (b(12) >> 4)) & 0x3f);
    s[11] = step(s[10], ((b(12) << 2) | (b(13) >> 6)) & 0x3f);
    s[15] = step(s[14], b(13) & 0x3f);
    for value in s.iter_mut() {
        *value = if *value & 0x8000 != 0 { *value & 0x7fff } else { !*value };
    }
    s
}

/// `unpack3`: a flat block from 3 bytes.
fn unpack3(b: &[u8]) -> [u16; 16] {
    let mut first = (b[0] as u16) << 8 | b[1] as u16;
    first = if first & 0x8000 != 0 { first & 0x7fff } else { !first };
    [first; 16]
}

/// `uncompress_b44_impl`: half channels as 4 × 4 blocks (14 bytes, or 3
/// for a flat one), others stored raw, then put in line order. `scratch`
/// keeps what earlier chunks left in it where blocks don't reach.
fn undo_b44(input: &[u8], output: &mut [u8], channels: &[CodingChannel], area: &ChunkArea, scratch: &mut Vec<u8>, b44a: bool) -> Result<()> {
    if !b44a {
        B44_TABLES_READY.store(true, Ordering::Relaxed);
    }
    let tables_ready = B44_TABLES_READY.load(Ordering::Relaxed);
    // `compute_scratch_buffer_size`.
    let padded: usize = channels.iter().map(|channel| (channel.width.max(0) as usize).next_multiple_of(4) * (channel.height.max(0) as usize).next_multiple_of(4) * channel.bytes_per_element).sum();
    let needed = output.len().max(padded);
    if scratch.len() < needed {
        *scratch = vec![0u8; needed];
    }
    let mut position = 0usize;
    let mut base = 0usize;
    for channel in channels {
        let nx = channel.width.max(0) as usize;
        let ny = channel.height.max(0) as usize;
        let bytes = nx * ny * channel.bytes_per_element;
        if bytes == 0 {
            continue;
        }
        if channel.data_type != PIXEL_HALF {
            if position + bytes > input.len() {
                return Err(anyhow!("Unable to decompress image data: out of memory"));
            }
            scratch[base..base + bytes].copy_from_slice(&input[position..position + bytes]);
            position += bytes;
            base += bytes;
            continue;
        }
        for y in (0..ny).step_by(4) {
            let row0 = base + 2 * y * nx;
            for x in (0..nx).step_by(4) {
                if position + 3 > input.len() {
                    return Err(anyhow!("Unable to decompress image data: out of memory"));
                }
                let mut block = if input[position + 2] >= (13 << 2) {
                    let block = unpack3(&input[position..]);
                    position += 3;
                    block
                } else {
                    if position + 14 > input.len() {
                        return Err(anyhow!("Unable to decompress image data: out of memory"));
                    }
                    let block = unpack14(&input[position..]);
                    position += 14;
                    block
                };
                if channel.p_linear {
                    for value in block.iter_mut() {
                        *value = if tables_ready { log_table()[*value as usize] } else { 0 };
                    }
                }
                let count = if x + 3 < nx { 4 } else { nx - x };
                let rows = if y + 3 < ny { 4 } else { ny - y };
                for row in 0..rows {
                    let start = row0 + 2 * (row * nx + x);
                    for column in 0..count {
                        let value = block[row * 4 + column];
                        scratch[start + 2 * column..start + 2 * column + 2].copy_from_slice(&value.to_le_bytes());
                    }
                }
            }
        }
        base += bytes;
    }

    let mut written = 0usize;
    for y in 0..area.height {
        let line = y + area.start_y;
        let mut plane = 0usize;
        for channel in channels {
            let line_bytes = channel.width.max(0) as usize * channel.bytes_per_element;
            let bytes = channel.height.max(0) as usize * line_bytes;
            if bytes == 0 {
                continue;
            }
            let row = if channel.y_samples > 1 {
                if line % channel.y_samples != 0 {
                    plane += bytes;
                    continue;
                }
                (y / channel.y_samples) as usize
            } else {
                y as usize
            };
            if written + line_bytes > output.len() {
                return Err(anyhow!("Unable to decompress image data: out of memory"));
            }
            let source = plane + row * line_bytes;
            output[written..written + line_bytes].copy_from_slice(&scratch[source..source + line_bytes]);
            written += line_bytes;
            plane += bytes;
        }
    }
    Ok(())
}

/// The decoder's buffers that outlive a chunk.
#[derive(Default)]
pub(super) struct Buffers {
    /// `unpacked_buffer`, reused from chunk to chunk.
    pub unpacked: Vec<u8>,
    /// `scratch_buffer_1` (B44's block planes).
    scratch: Vec<u8>,
}

/// `exr_uncompress_chunk` (`decompress_data`): `packed` into
/// `buffers.unpacked` (`unpacked_size` bytes). A chunk stored at its full
/// size is copied as it is.
pub(super) fn decompress(compression: u8, packed: &[u8], unpacked_size: usize, channels: &[CodingChannel], area: &ChunkArea, buffers: &mut Buffers) -> Result<()> {
    if buffers.unpacked.len() < unpacked_size {
        buffers.unpacked.resize(unpacked_size, 0);
    }
    if packed.is_empty() {
        return Ok(());
    }
    let output = &mut buffers.unpacked[..unpacked_size];
    if packed.len() == unpacked_size {
        output.copy_from_slice(packed);
        return Ok(());
    }
    match compression {
        COMPRESSION_NONE => bail!("no compression set but still trying to decompress"),
        COMPRESSION_RLE => {
            let restored = undo_rle(packed, unpacked_size)?;
            output.copy_from_slice(&restored);
        }
        COMPRESSION_ZIP | COMPRESSION_ZIPS => undo_zip(packed, output)?,
        COMPRESSION_PIZ => undo_piz(packed, output, channels, area)?,
        COMPRESSION_PXR24 => undo_pxr24(packed, output, channels, area)?,
        COMPRESSION_B44 | COMPRESSION_B44A => undo_b44(packed, output, channels, area, &mut buffers.scratch, compression == COMPRESSION_B44A)?,
        COMPRESSION_DWAA | COMPRESSION_DWAB => {
            let dwa_channels: Vec<super::dwa::Channel> = channels
                .iter()
                .map(|channel| super::dwa::Channel {
                    name: &channel.name,
                    pixel_type: channel.data_type,
                    width: channel.width.max(0) as usize,
                    height: channel.height.max(0) as usize,
                    x_samples: channel.x_samples,
                    y_samples: channel.y_samples,
                    p_linear: channel.p_linear,
                })
                .collect();
            let dwa_area = super::dwa::ChunkArea { start_y: area.start_y, width: area.width, height: area.height };
            let restored = super::dwa::decompress(packed, &dwa_channels, &dwa_area, unpacked_size)?;
            output.copy_from_slice(&restored);
        }
        other => bail!("Compression technique 0x{other:02X} is not supported"),
    }
    Ok(())
}
