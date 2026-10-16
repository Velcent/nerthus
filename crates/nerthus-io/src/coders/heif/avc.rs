//! AVC items and tracks through the pure Rust H.264 decoder. Keep the
//! decoded I420 planes and let the existing libheif colour path convert
//! them, just as its OpenH264 plugin does.

#![allow(non_snake_case)]

use std::collections::VecDeque;

use anyhow::{anyhow, bail, Result};
use rusty_h264_decoder::Decoder;

use super::boxes::AvcC;
use super::image::{Channel, Chroma, Colorspace, PixelImage};

pub fn decode(configuration: &AvcC, data: &[u8]) -> Result<PixelImage> {
    let mut decoder = SequenceDecoder::new(configuration.clone());
    decoder.push(data, 0)?;
    let (image, _) = decoder.nextImage().ok_or_else(|| anyhow!("AVC input did not produce an image"))?;
    Ok(image)
}

pub struct SequenceDecoder {
    decoder: Decoder,
    configuration: AvcC,
    initialized: bool,
    queue: VecDeque<(PixelImage, u32)>,
}

impl SequenceDecoder {
    pub fn new(configuration: AvcC) -> Self {
        Self { decoder: Decoder::new(), configuration, initialized: false, queue: VecDeque::new() }
    }

    pub fn push(&mut self, data: &[u8], user_data: u32) -> Result<()> {
        if data.is_empty() {
            bail!("Input with empty data extent.");
        }
        if self.configuration.bit_depth_luma != 8 {
            bail!("AVC decoder supports only 8-bit images");
        }
        let mut annex_b = Vec::new();
        if !self.initialized {
            for unit in &self.configuration.nal_units {
                annex_b.extend_from_slice(&[0, 0, 0, 1]);
                annex_b.extend_from_slice(unit);
            }
        }
        appendSample(&mut annex_b, data, self.configuration.length_size)?;
        let frame = self.decoder.decode(&annex_b).map_err(|error| anyhow!("AVC decoding error: {error:?}"))?;
        self.initialized = true;
        let Some(frame) = frame else {
            return Ok(());
        };
        let width = u32::try_from(frame.width)?;
        let height = u32::try_from(frame.height)?;
        let mut image = PixelImage::new(width, height, Colorspace::YCbCr, Chroma::C420);
        let planes = [(Channel::Y, frame.y), (Channel::Cb, frame.u), (Channel::Cr, frame.v)];
        for (channel, samples) in planes {
            let (plane_width, plane_height) = if channel == Channel::Y {
                (width, height)
            } else {
                (width.div_ceil(2), height.div_ceil(2))
            };
            if samples.len() != plane_width as usize * plane_height as usize {
                bail!("AVC decoder returned inconsistent plane dimensions");
            }
            image.add_plane(channel, plane_width, plane_height, 8);
            image.plane_mut(channel).samples = samples.into_iter().map(u16::from).collect();
        }
        // OpenH264's plugin does not attach SPS/VUI colour metadata. The
        // item's colr property, when present, is applied by Context.
        self.queue.push_back((image, user_data));
        Ok(())
    }

    pub fn nextImage(&mut self) -> Option<(PixelImage, u32)> {
        self.queue.pop_front()
    }
}

fn appendSample(annex_b: &mut Vec<u8>, data: &[u8], length_size: u8) -> Result<()> {
    if !matches!(length_size, 1 | 2 | 4) {
        bail!("Invalid AVC NAL length size");
    }
    let mut remaining = data;
    while !remaining.is_empty() {
        if remaining.len() < length_size as usize {
            bail!("Truncated AVC NAL length");
        }
        let (length_bytes, payload) = remaining.split_at(length_size as usize);
        let mut length = 0_usize;
        for &byte in length_bytes {
            length = (length << 8) | byte as usize;
        }
        if length == 0 || length > payload.len() {
            bail!("Invalid AVC NAL length");
        }
        let (unit, tail) = payload.split_at(length);
        annex_b.extend_from_slice(&[0, 0, 0, 1]);
        annex_b.extend_from_slice(unit);
        remaining = tail;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::*;

    #[test]
    fn matchesOpenH264PixelCaches() {
        // SHA-256 of the little-endian float Q16 pixel cache returned by
        // Magick.NET 14.10.1 / ImageMagick 7.1.2-19 / OpenH264.
        let cases: [(&[u8], &str); 4] = [
            (include_bytes!("fixtures/avc-baseline.heic"), "3f350fcb77f93d52e52326f37615bf8edcc25571509e87a1c2e6d14dde86879f"),
            (include_bytes!("fixtures/avc-main.heic"), "dbbaead1db369155a459a257c012030d5f315621b82861928fcfd36fc67db26d"),
            (include_bytes!("fixtures/avc-high.heic"), "dbbaead1db369155a459a257c012030d5f315621b82861928fcfd36fc67db26d"),
            (include_bytes!("fixtures/avc-sequence.heic"), "80fabdbd142bcd426e4c321e00558a178f08a4ab6db5ad20215b687f8a222363"),
        ];
        for (bytes, expected) in cases {
            let image = super::super::decode(bytes).unwrap();
            assert_eq!((image.width, image.height), (32, 24));
            assert_eq!(image.channels.len(), 3);
            let mut hash = Sha256::new();
            for sample in image.data {
                hash.update(sample.to_le_bytes());
            }
            let actual = format!("{:x}", hash.finalize());
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn acceptsAllNalLengthWidths() {
        let expected = [0, 0, 0, 1, 0x65, 0x88, 0x42];
        for length_size in [1, 2, 4] {
            let mut sample = vec![0; length_size];
            sample[length_size - 1] = 3;
            sample.extend_from_slice(&[0x65, 0x88, 0x42]);
            let mut annex_b = Vec::new();
            appendSample(&mut annex_b, &sample, length_size as u8).unwrap();
            assert_eq!(annex_b, expected);
        }
    }

    #[test]
    fn rejectsInvalidNalLengths() {
        let malformed: [&[u8]; 4] = [&[0, 0, 0], &[0, 0, 0, 0], &[0, 0, 0, 2, 0x65], &[255, 255, 255, 255, 0x65]];
        for data in malformed {
            let mut annex_b = Vec::new();
            assert!(appendSample(&mut annex_b, data, 4).is_err());
        }
    }
}
