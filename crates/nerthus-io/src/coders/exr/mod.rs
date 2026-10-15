//! OpenEXR as ImageMagick reads it through OpenEXRCore 3.4.9
//! (`coders/exr.c`): the first part's data window as the image, sRGB with
//! gamma 1 — GRAY when its only channel is luminance — and each channel
//! taken by the first letter of its name, after a prefix ending in `.`
//! that every channel shares: R, G, B and A, Y for the grey (red) channel,
//! anything else a meta channel. Half and float samples become
//! `QuantumRange · value`, unclamped. Colour channels the file lacks keep
//! what the pixel cache held, which is zero here.
//!
//! Each chunk is decoded into one buffer of interleaved samples that lives
//! for the whole image, a line of the image wide (or a tile), and read out
//! from it. A subsampled channel's samples land there packed into the
//! first columns and lines of the chunk, not spread out; the rest of its
//! slots keep what earlier chunks left (zero at first). The headers,
//! chunks and codecs are OpenEXRCore's ([`header`], [`chunks`],
//! [`codecs`], [`dwa`]).

mod chunks;
mod codecs;
mod dwa;
mod header;

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel};
use chunks::{ChunkInfo, Chunks};
use codecs::{Buffers, ChunkArea, CodingChannel};
use header::{sampled_height, sampled_width, Part, Storage, PIXEL_FLOAT, PIXEL_HALF};

/// Where a file channel's samples go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Red,
    Green,
    Blue,
    Alpha,
    /// Luminance, written to the grey (red) channel.
    Gray,
    Meta(usize),
}

/// `InitializeEXRChannels`: the prefix shared by every channel (up to and
/// including the first channel's last `.`), when they all share it.
fn shared_prefix_length(names: &[String]) -> usize {
    let Some(first) = names.first() else {
        return 0;
    };
    let Some(dot) = first.rfind('.') else {
        return 0;
    };
    let prefix = &first.as_bytes()[..dot + 1];
    let shared = names.iter().all(|name| name.as_bytes().starts_with(prefix));
    if shared {
        prefix.len()
    } else {
        0
    }
}

/// Each channel's target, by the first letter after the shared prefix
/// (any case); meta channels are numbered in channel order.
fn channel_targets(names: &[String]) -> Vec<Target> {
    let prefix_length = shared_prefix_length(names);
    let mut meta_count = 0;
    let mut targets = Vec::with_capacity(names.len());
    for name in names {
        let first = name.as_bytes().get(prefix_length).copied().unwrap_or(0).to_ascii_uppercase();
        let target = match first {
            b'R' => Target::Red,
            b'G' => Target::Green,
            b'B' => Target::Blue,
            b'A' => Target::Alpha,
            b'Y' => Target::Gray,
            _ => {
                meta_count += 1;
                Target::Meta(meta_count - 1)
            }
        };
        targets.push(target);
    }
    targets
}

/// IM's `HalfToSinglePrecision`.
fn half_to_single(half: u16) -> f32 {
    let sign = ((half >> 15) & 1) as u32;
    let mut exponent = ((half >> 10) & 0x1f) as u32;
    let mut significand = (half & 0x3ff) as u32;
    let value = if exponent == 0 {
        if significand == 0 {
            sign << 31
        } else {
            while significand & 0x400 == 0 {
                significand <<= 1;
                exponent = exponent.wrapping_sub(1);
            }
            exponent = exponent.wrapping_add(1);
            significand &= !0x400;
            exponent = exponent.wrapping_add(127 - 15);
            (sign << 31) | (exponent << 23) | (significand << 13)
        }
    } else if exponent == 0x1f {
        let mut bits = (sign << 31) | 0x7f80_0000;
        if significand != 0 {
            bits |= significand << 13;
        }
        bits
    } else {
        exponent += 127 - 15;
        (sign << 31) | (exponent << 23) | (significand << 13)
    };
    f32::from_bits(value)
}

/// `ClampToQuantum(QuantumRange · value)` in an HDRI build.
fn quantum(value: f32) -> f32 {
    (65535.0 * value as f64) as f32
}

/// The decode pipeline's channels for a chunk (`internal_coding_fill_channel_info`).
fn coding_channels(part: &Part, chunk: &ChunkInfo) -> Vec<CodingChannel> {
    part.channels()
        .iter()
        .map(|channel| CodingChannel {
            name: channel.name.clone(),
            width: sampled_width(chunk.width, channel.x_sampling, chunk.start_x),
            height: sampled_height(chunk.height, channel.y_sampling, chunk.start_y),
            x_samples: channel.x_sampling,
            y_samples: channel.y_sampling,
            p_linear: channel.p_linear,
            bytes_per_element: channel.bytes_per_element() as usize,
            data_type: channel.pixel_type,
        })
        .collect()
}

/// IM's sample buffer: every channel's samples interleaved (`pixel_size`
/// bytes a pixel), `line_stride` bytes a line, kept from chunk to chunk.
struct SampleBuffer {
    data: Vec<u8>,
    pixel_size: usize,
    line_stride: usize,
    /// Each channel's byte offset within a pixel.
    offsets: Vec<usize>,
}

/// `exr_decoding_run` for one chunk: its data read and decompressed, then
/// unpacked into the sample buffer (`generic_unpack`, which the faster
/// routines match): line by line, each channel that has a sample on the
/// line writing its samples to the buffer line its own line count reaches.
fn decode_chunk(chunks: &Chunks, part: &Part, chunk: &ChunkInfo, buffers: &mut Buffers, samples: &mut SampleBuffer) -> Result<()> {
    let channels = coding_channels(part, chunk);
    let packed = chunks.read(chunk)?;
    let unpacked_size = chunk.unpacked_size as usize;
    let area = ChunkArea { start_y: chunk.start_y, width: chunk.width, height: chunk.height };
    codecs::decompress(part.compression(), packed, unpacked_size, &channels, &area, buffers)?;
    if unpacked_size == 0 {
        return Ok(());
    }
    let unpacked = &buffers.unpacked;
    let mut source = 0usize;
    for y in 0..chunk.height {
        let line = y + chunk.start_y;
        for (channel, &offset) in channels.iter().zip(samples.offsets.iter()) {
            let width = channel.width.max(0) as usize;
            let bytes = channel.bytes_per_element;
            let row = if channel.y_samples > 1 {
                if line % channel.y_samples != 0 {
                    continue;
                }
                (y / channel.y_samples) as usize
            } else {
                y as usize
            };
            for x in 0..width {
                let target = row * samples.line_stride + x * samples.pixel_size + offset;
                let value = unpacked.get(source + x * bytes..source + (x + 1) * bytes).context("CorruptImage: chunk data too short")?;
                if let Some(slot) = samples.data.get_mut(target..target + bytes) {
                    slot.copy_from_slice(value);
                }
            }
            source += width * bytes;
        }
    }
    Ok(())
}

/// `ReadEXRPixels`: `count` pixels from the sample buffer, every channel
/// of each, into the cache from pixel `start` on; after each `columns`
/// pixels the buffer skips `skip` bytes and the cache moves to its next
/// row.
#[allow(clippy::too_many_arguments)]
fn read_pixels(samples: &SampleBuffer, part: &Part, positions: &[usize], data: &mut [f32], stride: usize, image_width: usize, origin: (usize, usize), count: usize, columns: usize, skip: usize) {
    let mut source = 0usize;
    for index in 0..count {
        let (x, y) = (origin.0 + index % columns, origin.1 + index / columns);
        for (channel, &position) in part.channels().iter().zip(positions.iter()) {
            let bytes = &samples.data[source..source + channel.bytes_per_element() as usize];
            let value = if channel.pixel_type == PIXEL_HALF { half_to_single(u16::from_ne_bytes([bytes[0], bytes[1]])) } else { f32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) };
            data[(y * image_width + x) * stride + position] = quantum(value);
            source += channel.bytes_per_element() as usize;
        }
        if skip != 0 && (count - index - 1).is_multiple_of(columns) {
            source += skip;
        }
    }
}

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    let headers = header::parse(&bytes).map_err(|error| anyhow::anyhow!("UnableToOpenBlob: {error}"))?;
    let part = headers.parts[0].clone();
    let window = part.data_window();
    if window.min_x >= window.max_x || window.min_y >= window.max_y {
        bail!("CorruptImage");
    }
    let width = (window.max_x as i64 - window.min_x as i64 + 1) as usize;
    let height = (window.max_y as i64 - window.min_y as i64 + 1) as usize;
    if !matches!(part.storage, Storage::Scanline | Storage::DeepScanline | Storage::Tiled | Storage::DeepTiled) {
        bail!("Unsupported storage type");
    }
    if part.storage.is_deep() {
        bail!("CorruptImage: deep data");
    }
    let mut chunks = Chunks::new(&bytes, headers);

    // `InitializeEXRChannels`.
    let file_channels = part.channels();
    if file_channels.len() >= 64 - 10 {
        bail!("MaximumChannelsExceeded");
    }
    if let Some(channel) = file_channels.iter().find(|channel| channel.pixel_type != PIXEL_HALF && channel.pixel_type != PIXEL_FLOAT) {
        bail!("Unsupported channel data type `{}'", channel.pixel_type);
    }
    let names: Vec<String> = file_channels.iter().map(|channel| String::from_utf8_lossy(&channel.name).to_string()).collect();
    let targets = channel_targets(&names);
    let meta_count = targets.iter().filter(|target| matches!(target, Target::Meta(_))).count();
    let has_alpha = targets.contains(&Target::Alpha);
    let gray = targets.len() == 1 && targets[0] == Target::Gray;
    let mut channels = if gray { vec![PixelChannel::Gray] } else { vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue] };
    let alpha_position = channels.len();
    if has_alpha && !gray {
        channels.push(PixelChannel::Alpha);
    }
    let meta_start = channels.len();
    channels.extend(std::iter::repeat_n(PixelChannel::Meta, meta_count));
    let positions: Vec<usize> = targets
        .iter()
        .map(|target| match target {
            Target::Red | Target::Gray => 0,
            Target::Green => 1,
            Target::Blue => 2,
            Target::Alpha => alpha_position,
            Target::Meta(index) => meta_start + index,
        })
        .collect();
    let pixel_size: usize = file_channels.iter().map(|channel| channel.bytes_per_element() as usize).sum();
    let mut offsets = Vec::with_capacity(file_channels.len());
    let mut offset = 0;
    for channel in file_channels {
        offsets.push(offset);
        offset += channel.bytes_per_element() as usize;
    }

    let stride = channels.len();
    let mut data = vec![0.0f32; width * height * stride];
    let mut buffers = Buffers::default();
    if part.storage.is_scanline() {
        // `ReadEXRScanlineImage`.
        let lines = part.lines_per_chunk.max(1) as usize;
        let first = chunks.scanline_chunk(0, window.min_y).map_err(|error| anyhow::anyhow!("CorruptImage: {error}"))?;
        let mut samples = SampleBuffer { data: vec![0u8; pixel_size * lines * width], pixel_size, line_stride: width * pixel_size, offsets };
        let mut chunk = first;
        for y in (0..height).step_by(lines) {
            if y != 0 {
                chunk = chunks.scanline_chunk(0, y as i32 + window.min_y).map_err(|error| anyhow::anyhow!("CorruptImage: {error}"))?;
            }
            decode_chunk(&chunks, &part, &chunk, &mut buffers, &mut samples).map_err(|error| anyhow::anyhow!("CorruptImage: {error}"))?;
            let rows = lines.min(height - y);
            read_pixels(&samples, &part, &positions, &mut data, stride, width, (0, y), rows * width, width, 0);
        }
    } else {
        // `ReadEXRTiledImage`.
        let tiles = part.tiles.expect("tiled parts have tiles");
        let (levels_x, levels_y) = (part.tile_level_counts.0.len(), part.tile_level_counts.1.len());
        let tile_width = (tiles.x_size as i64).min(part.tile_level_sizes.0[0] as i64) as usize;
        let tile_height = (tiles.y_size as i64).min(part.tile_level_sizes.1[0] as i64) as usize;
        chunks.tile_chunk(0, 0, 0).map_err(|error| anyhow::anyhow!("CorruptImage: {error}"))?;
        if levels_x != 1 || levels_y != 1 {
            bail!("Unsupported number of levels `{levels_x} {levels_y}'");
        }
        let mut samples = SampleBuffer { data: vec![0u8; pixel_size * tile_width * tile_height], pixel_size, line_stride: tile_width * pixel_size, offsets };
        for (tile_y, y) in (0..height).step_by(tile_height).enumerate() {
            for (tile_x, x) in (0..width).step_by(tile_width).enumerate() {
                let chunk = chunks.tile_chunk(0, tile_x as i32, tile_y as i32).map_err(|error| anyhow::anyhow!("CorruptImage: {error}"))?;
                decode_chunk(&chunks, &part, &chunk, &mut buffers, &mut samples).map_err(|error| anyhow::anyhow!("CorruptImage: {error}"))?;
                let columns = tile_width.min(width - x);
                let rows = tile_height.min(height - y);
                let skip = (tile_width - columns) * pixel_size;
                read_pixels(&samples, &part, &positions, &mut data, stride, width, (x, y), columns * rows, columns, skip);
            }
        }
    }
    let colorspace = if gray { CacheColorspace::Gray } else { CacheColorspace::SRgb };
    Ok(PixelCache::new(width, height, channels, data, colorspace, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn channels_map_by_their_first_letter_after_a_shared_prefix() {
        assert_eq!(channel_targets(&names(&["layer.B", "layer.G", "layer.R"])), vec![Target::Blue, Target::Green, Target::Red]);
        assert_eq!(channel_targets(&names(&["B", "layer.R", "other.G"])), vec![Target::Blue, Target::Meta(0), Target::Meta(1)]);
        assert_eq!(channel_targets(&names(&["alpha", "blue", "green", "r"])), vec![Target::Alpha, Target::Blue, Target::Green, Target::Red]);
        assert_eq!(channel_targets(&names(&["Y", "Z", "depth"])), vec![Target::Gray, Target::Meta(0), Target::Meta(1)]);
    }

    #[test]
    fn halves_convert_exactly() {
        for bits in 0..=u16::MAX {
            let ours = half_to_single(bits);
            let reference = half::f16::from_bits(bits).to_f32();
            if reference.is_nan() {
                assert!(ours.is_nan());
            } else {
                assert_eq!(ours.to_bits(), reference.to_bits(), "{bits:#x}");
            }
        }
    }
}
