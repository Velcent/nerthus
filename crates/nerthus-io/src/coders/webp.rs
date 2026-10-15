//! WebP (`coders/webp.c`): libwebp's RGBA output (`MODE_RGBA`, fancy
//! upsampling for lossy images, as `WebPInitDecoderConfig` leaves it), each
//! sample scaled to a quantum; an alpha channel only when the file has one
//! (`features.has_alpha`). sRGB with IM's default gamma.
//!
//! An animation reads as its first frame on a canvas of the animation's
//! size: the frame's own pixels at its offset, every channel zero around
//! them, and an alpha channel when the frame's bitstream has one. Frames
//! are not blended or disposed.

use std::io::Cursor;
use std::path::Path;

use anyhow::{bail, Context, Result};
use image_webp::{UpsamplingMethod, WebPDecoder};

use crate::pixel_cache::{CacheColorspace, DEFAULT_GAMMA, PixelCache, PixelChannel, char_quantum};

/// A decoded bitstream: interleaved 8-bit RGB or RGBA samples.
struct Decoded {
    width: usize,
    height: usize,
    has_alpha: bool,
    samples: Vec<u8>,
}

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    let decoder = WebPDecoder::new(Cursor::new(&bytes)).with_context(|| format!("cannot read {}", path.display()))?;
    if !decoder.is_animated() {
        let decoded = decode(decoder)?;
        return Ok(to_cache(&decoded));
    }

    let (canvas_width, canvas_height) = decoder.dimensions();
    let frame = first_frame(&bytes).with_context(|| format!("cannot read {}", path.display()))?;
    let frame_decoder = WebPDecoder::new(Cursor::new(&frame.stream)).with_context(|| format!("cannot read {}", path.display()))?;
    let decoded = decode(frame_decoder)?;
    if frame.x + decoded.width > canvas_width as usize || frame.y + decoded.height > canvas_height as usize {
        bail!("cannot read {}: a frame lies outside the canvas", path.display());
    }

    let stride = if decoded.has_alpha { 4 } else { 3 };
    let canvas_width = canvas_width as usize;
    let canvas_height = canvas_height as usize;
    let mut samples = vec![0u8; canvas_width * canvas_height * stride];
    for row in 0..decoded.height {
        let source_start = row * decoded.width * stride;
        let source = &decoded.samples[source_start..source_start + decoded.width * stride];
        let target_start = ((frame.y + row) * canvas_width + frame.x) * stride;
        samples[target_start..target_start + source.len()].copy_from_slice(source);
    }
    let canvas = Decoded { width: canvas_width, height: canvas_height, has_alpha: decoded.has_alpha, samples };
    Ok(to_cache(&canvas))
}

fn decode(mut decoder: WebPDecoder<Cursor<&Vec<u8>>>) -> Result<Decoded> {
    decoder.set_lossy_upsampling(UpsamplingMethod::Bilinear);
    let (width, height) = decoder.dimensions();
    let has_alpha = decoder.has_alpha();
    let size = decoder.output_buffer_size().context("the WebP image is too large")?;
    let mut samples = vec![0u8; size];
    decoder.read_image(&mut samples)?;
    Ok(Decoded { width: width as usize, height: height as usize, has_alpha, samples })
}

fn to_cache(decoded: &Decoded) -> PixelCache {
    let mut channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    if decoded.has_alpha {
        channels.push(PixelChannel::Alpha);
    }
    let data = decoded.samples.iter().map(|&sample| char_quantum(sample)).collect();
    PixelCache::new(decoded.width, decoded.height, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA)
}

/// An animation's first frame as a WebP file of its own, and where it
/// goes on the canvas.
struct Frame {
    x: usize,
    y: usize,
    stream: Vec<u8>,
}

/// One RIFF chunk: its tag and payload.
struct Chunk<'a> {
    tag: [u8; 4],
    payload: &'a [u8],
}

/// The chunks in `data`, each padded to an even size.
fn chunks(data: &[u8]) -> Vec<Chunk<'_>> {
    let mut list = Vec::new();
    let mut offset = 0;
    while offset + 8 <= data.len() {
        let tag = [data[offset], data[offset + 1], data[offset + 2], data[offset + 3]];
        let size = u32::from_le_bytes([data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7]]) as usize;
        let start = offset + 8;
        let Some(end) = start.checked_add(size).filter(|&end| end <= data.len()) else {
            break;
        };
        list.push(Chunk { tag, payload: &data[start..end] });
        offset = end + (size & 1);
    }
    list
}

fn read_u24(bytes: &[u8]) -> usize {
    bytes[0] as usize | (bytes[1] as usize) << 8 | (bytes[2] as usize) << 16
}

/// `WebPDemuxGetFrame(demux, 1, …)`: the first `ANMF` chunk's offset and
/// bitstream (an optional `ALPH` chunk, then `VP8 ` or `VP8L`), wrapped
/// so that a decoder sees what libwebp sees in the fragment: lossless
/// frames carry their own alpha flag, lossy ones have alpha exactly when
/// an `ALPH` chunk comes with them.
fn first_frame(bytes: &[u8]) -> Result<Frame> {
    if bytes.len() < 12 {
        bail!("the file is truncated");
    }
    let Some(animation_frame) = chunks(&bytes[12..]).into_iter().find(|chunk| &chunk.tag == b"ANMF") else {
        bail!("the animation has no frames");
    };
    let payload = animation_frame.payload;
    if payload.len() < 16 {
        bail!("an animation frame is truncated");
    }
    let x = 2 * read_u24(&payload[0..3]);
    let y = 2 * read_u24(&payload[3..6]);
    let width = read_u24(&payload[6..9]) + 1;
    let height = read_u24(&payload[9..12]) + 1;

    let frame_chunks = chunks(&payload[16..]);
    let mut alpha = None;
    let mut image = None;
    for chunk in frame_chunks {
        match &chunk.tag {
            b"ALPH" if alpha.is_none() && image.is_none() => alpha = Some(chunk),
            b"VP8 " | b"VP8L" => {
                image = Some(chunk);
                break;
            }
            _ => {}
        }
    }
    let Some(image) = image else {
        bail!("an animation frame has no image");
    };

    let mut body = Vec::new();
    match (&image.tag, alpha) {
        (b"VP8 ", Some(alpha)) => {
            // VP8X: the alpha flag and the frame's size.
            let mut header = vec![0x10, 0, 0, 0];
            header.extend_from_slice(&((width - 1) as u32).to_le_bytes()[..3]);
            header.extend_from_slice(&((height - 1) as u32).to_le_bytes()[..3]);
            push_chunk(&mut body, b"VP8X", &header);
            push_chunk(&mut body, b"ALPH", alpha.payload);
            push_chunk(&mut body, b"VP8 ", image.payload);
        }
        _ => push_chunk(&mut body, &image.tag, image.payload),
    }
    let mut stream = Vec::with_capacity(body.len() + 12);
    stream.extend_from_slice(b"RIFF");
    stream.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
    stream.extend_from_slice(b"WEBP");
    stream.extend_from_slice(&body);
    Ok(Frame { x, y, stream })
}

/// Appends a RIFF chunk, padded to an even size.
fn push_chunk(body: &mut Vec<u8>, tag: &[u8; 4], payload: &[u8]) {
    body.extend_from_slice(tag);
    body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    body.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        body.push(0);
    }
}
