//! JPEG XL's first displayed frame, with ImageMagick's channel layout,
//! sample precision and unassociated alpha. Decoding stays in Rust.

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use jxl_oxide::color::{ColourEncoding, TransferFunction};
use jxl_oxide::JxlImage;

use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let builder = JxlImage::builder();
    let mut image = builder.read(bytes).map_err(|error| anyhow!("JPEG XL: {error}"))?;
    // Spot channels are metadata to ImageMagick, not paint added to RGB.
    image.set_render_spot_color(false);
    let header = image.image_header();
    let width = header.size.width as usize;
    let height = header.size.height as usize;
    let depth = header.metadata.bit_depth.bits_per_sample();
    let associated_alpha = header.metadata.ec_info.iter().find_map(|channel| channel.alpha_associated()).unwrap_or(false);
    let pixel_format = image.pixel_format();
    let is_gray = pixel_format.is_grayscale();
    let has_alpha = pixel_format.has_alpha();

    // Like libjxl, leave the data in its declared encoding. The File node
    // reads the cache before any display colour management is performed.
    let transfer = match &header.metadata.colour_encoding {
        ColourEncoding::Enum(encoding) => encoding.tf,
        ColourEncoding::IccProfile(_) => TransferFunction::Srgb,
    };
    let linear = transfer == TransferFunction::Linear;
    let colorspace = match (is_gray, linear) {
        (true, true) => CacheColorspace::LinearGray,
        (true, false) => CacheColorspace::Gray,
        (false, true) => CacheColorspace::Rgb,
        (false, false) => CacheColorspace::SRgb,
    };
    let gamma = match transfer {
        TransferFunction::Linear => 1.0,
        TransferFunction::Gamma { g, inverted: true } => g as f64 / 10_000_000.0,
        TransferFunction::Gamma { g, inverted: false } => 10_000_000.0 / g as f64,
        _ => DEFAULT_GAMMA,
    };
    let mut channels = if is_gray {
        vec![PixelChannel::Gray]
    } else {
        vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue]
    };
    if has_alpha {
        channels.push(PixelChannel::Alpha);
    }
    let render = image.render_frame(0).map_err(|error| anyhow!("JPEG XL: {error}"))?;
    let mut stream = render.stream();
    let stream_width = stream.width() as usize;
    let stream_components = stream.channels() as usize;
    let sample_count = width.checked_mul(height).and_then(|count| count.checked_mul(stream_components));
    let sample_count = sample_count.ok_or_else(|| anyhow!("JPEG XL image dimensions overflow"))?;
    if stream_components != channels.len() + usize::from(pixel_format.has_black()) {
        bail!("JPEG XL channel count does not match its header");
    }
    let mut samples = vec![0.0_f32; sample_count];
    if stream.write_to_buffer(&mut samples) != sample_count {
        bail!("JPEG XL frame is incomplete");
    }

    // ImageMagick asks libjxl to keep orientation in metadata. jxl-oxide
    // applies it when streaming, so address that buffer in original order.
    let mut data = Vec::with_capacity(sample_count);
    let component_count = channels.len();
    let color_count = component_count - usize::from(has_alpha);
    for y in 0..height {
        for x in 0..width {
            let (oriented_x, oriented_y) = match render.orientation() {
                1 => (x, y),
                2 => (width - x - 1, y),
                3 => (width - x - 1, height - y - 1),
                4 => (x, height - y - 1),
                5 => (y, x),
                6 => (height - y - 1, x),
                7 => (height - y - 1, width - x - 1),
                8 => (y, width - x - 1),
                _ => bail!("Invalid JPEG XL orientation"),
            };
            let offset = (oriented_y * stream_width + oriented_x) * stream_components;
            let pixel = &samples[offset..offset + stream_components];
            // libjxl's RGB(A) output excludes extra channels, including
            // black. ImageMagick retains the ICC profile as metadata.
            for component in 0..component_count {
                let source_component = if has_alpha && component == color_count { stream_components - 1 } else { component };
                let value = pixel[source_component];
                let value = if associated_alpha && component < color_count {
                    let alpha = pixel[stream_components - 1];
                    if alpha > 0.0 { value / alpha } else { value }
                } else {
                    value
                };
                // libjxl's unsigned output clips and rounds; float output
                // remains HDRI. Q16 receives uint8 scaled by 257.
                let quantum = if depth <= 8 {
                    (value.clamp(0.0, 1.0) * 255.0).round() * 257.0
                } else if depth <= 16 {
                    (value.clamp(0.0, 1.0) * 65535.0).round()
                } else {
                    value * 65535.0
                };
                data.push(quantum);
            }
        }
    }
    Ok(PixelCache::new(width, height, channels, data, colorspace, gamma))
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    #[test]
    fn readsLosslessRgbAndAlpha() {
        let cases: [(&[u8], usize); 2] = [
            (include_bytes!("jxl-fixtures/rgb.jxl"), 3),
            (include_bytes!("jxl-fixtures/rgba.jxl"), 4),
        ];
        for (bytes, components) in cases {
            let image = decode(bytes).unwrap();
            assert_eq!((image.width, image.height), (4, 3));
            assert_eq!(image.channels.len(), components);
            assert_eq!(image.alpha_trait, components == 4);
            assert_eq!(image.colorspace, CacheColorspace::SRgb);
            let mut expected = Vec::new();
            for y in 0..3 {
                for x in 0..4 {
                    for component in 0..components {
                        expected.push(((x * 29 + y * 61 + component * 77) % 256) as f32 * 257.0);
                    }
                }
            }
            assert_eq!(image.data, expected);
        }
    }

    #[test]
    fn preservesSixteenBitGray() {
        let image = decode(include_bytes!("jxl-fixtures/gray16.jxl")).unwrap();
        assert_eq!((image.width, image.height), (4, 3));
        assert_eq!(image.channels, [PixelChannel::Gray]);
        assert_eq!(image.colorspace, CacheColorspace::Gray);
        let mut expected = Vec::new();
        for y in 0..3 {
            for x in 0..4 {
                expected.push(((x * 5701 + y * 12013) % 65536) as f32);
            }
        }
        assert_eq!(image.data, expected);
    }

    #[test]
    fn rejectsTruncatedCodestream() {
        let bytes = include_bytes!("jxl-fixtures/rgb.jxl");
        assert!(decode(&bytes[..bytes.len() / 2]).is_err());
        assert!(decode(b"\xff\x0a").is_err());
    }
}
