//! HEIF/HEIC/AVIF as ImageMagick 7.1.2 reads them (`coders/heic.c`)
//! through libheif 1.21.2: the primary image decoded to interleaved RGB
//! (RGBA when it has alpha), 8-bit or, deeper than 8 bits, 16-bit samples
//! holding the decoded value shifted up to 16 bits. Transformations
//! (rotation, mirroring, clean aperture) are applied. A file with an
//! image sequence gives the first frame of its first visual track instead,
//! always as 8-bit RGB(A).

#[cfg(not(target_arch = "wasm32"))]
mod av1;
mod boxes;
mod color;
mod context;
mod image;
mod nclx;
mod sequence;

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{bail, Context as _, Result};

use self::color::{convert_colorspace, ConversionOptions};
use self::context::Context;
use self::image::{Channel, Chroma, Colorspace};
use self::nclx::Nclx;
use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    if bytes.len() < 128 {
        bail!("InsufficientImageDataInFile");
    }
    if &bytes[4..8] != b"ftyp" {
        bail!("ImageTypeNotSupported");
    }
    let mut context = Context::read(bytes)?;
    if context.has_sequence() {
        return read_sequence_frame(&mut context);
    }
    let Some(primary) = context.primary else {
        bail!("No or invalid primary item");
    };
    let image = read_image_handle(&context, primary)?;

    // IM goes on to the next top-level image, which it only opens (its
    // pixels are past the one scene Magick.NET reads), so only an item
    // that cannot be opened fails the read.
    if context.top_level.len() > 1
        && let Some(&other) = context.top_level.iter().find(|&&id| id != primary)
        && let Some(error) = &context.items[&other].error
    {
        bail!("{error}");
    }
    Ok(image)
}

/// `ReadHEICImageHandle` for the first scene.
fn read_image_handle(context: &Context, id: u32) -> Result<PixelCache> {
    let item = &context.items[&id];
    if let Some(error) = &item.error {
        bail!("{error}");
    }
    let mut columns = item.width as i64;
    let mut rows = item.height as i64;
    if columns > i32::MAX as i64 {
        columns = 0;
    }
    if rows > i32::MAX as i64 {
        rows = 0;
    }
    let has_alpha = context.has_alpha(id);
    let bits = context.luma_bits_per_pixel(id);
    let depth = if bits != -1 { bits } else { 8 };
    if columns == 0 || rows == 0 {
        bail!("NegativeOrZeroImageSize");
    }

    let chroma = match (has_alpha, depth > 8) {
        (true, false) => Chroma::InterleavedRgba,
        (true, true) => Chroma::InterleavedRrggbbaaLe,
        (false, false) => Chroma::InterleavedRgb,
        (false, true) => Chroma::InterleavedRrggbbLe,
    };
    let decoded = context.decode_image(id, &BTreeSet::new())?;
    let image = to_output_colorspace(decoded, chroma)?;
    Ok(to_pixel_cache(&image, has_alpha, depth))
}

/// `ReadHEICSequenceFrames` for the one scene read: the first image of the
/// first visual track, as 8-bit RGB, or RGBA when an alpha track belongs
/// to it.
fn read_sequence_frame(context: &mut Context) -> Result<PixelCache> {
    let Some(id) = context.default_track() else {
        bail!("Invalid track id");
    };
    let track = &context.tracks[&id];
    if !track.is_decodable() {
        bail!("Cannot get resolution of non-visual track.");
    }
    let has_alpha = track.alpha_track.is_some();
    let chroma = if has_alpha { Chroma::InterleavedRgba } else { Chroma::InterleavedRgb };
    if track.end_of_sequence_reached() {
        // ImageMagick ends the sequence with no frame read.
        bail!("NegativeOrZeroImageSize");
    }
    let decoded = context.decode_next_track_image(id)?;
    let image = to_output_colorspace(decoded, chroma)?;
    let plane = image.plane(Channel::Interleaved);
    if plane.width == 0 || plane.height == 0 {
        bail!("NegativeOrZeroImageSize");
    }
    Ok(to_pixel_cache(&image, has_alpha, plane.bit_depth as i32))
}

/// `convert_to_output_colorspace`: a conversion unless the decoded image
/// already is the RGB interleaving asked for with the default (sRGB) nclx.
fn to_output_colorspace(decoded: image::PixelImage, chroma: Chroma) -> Result<image::PixelImage> {
    let image_nclx = if decoded.has_nclx() { decoded.nclx } else { Nclx::defaults() };
    let different = decoded.chroma != chroma || decoded.colorspace != Colorspace::Rgb || image_nclx != Nclx::defaults();
    if !different {
        return Ok(decoded);
    }
    let options = ConversionOptions::decoding_defaults();
    convert_colorspace(&decoded, Colorspace::Rgb, chroma, Nclx::defaults(), 0, &options)
}

/// The interleaved samples as quantum values: 8-bit samples scaled, deeper
/// ones shifted up to 16 bits.
fn to_pixel_cache(image: &image::PixelImage, has_alpha: bool, depth: i32) -> PixelCache {
    let plane = image.plane(Channel::Interleaved);
    let width = plane.width as usize;
    let height = plane.height as usize;
    let components = if has_alpha { 4 } else { 3 };
    let shift = 16 - depth;
    let mut data = Vec::with_capacity(width * height * components);
    for y in 0..height {
        for x in 0..width {
            for component in 0..components {
                let value = plane.samples[y * plane.stride() + x * plane.components + component];
                let quantum = if depth <= 8 { (value & 0xff) as f32 * 257.0 } else { (((value as u32) << shift) & 0xffff) as f32 };
                data.push(quantum);
            }
        }
    }
    let mut channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    if has_alpha {
        channels.push(PixelChannel::Alpha);
    }
    PixelCache::new(width, height, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA)
}
