//! Garmin JNX tile maps as ImageMagick reads them (`coders/jnx.c`): every
//! level's tiles are JPEGs missing their start-of-image marker; IM reads
//! them all (a bad one fails the file) and the first is the image.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use crate::pixel_cache::PixelCache;

const MAX_LEVELS: i32 = 20;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let signed = |blob: &mut Blob| blob.lsb_u32() as i32;
    let version = signed(&mut blob);
    if version != 3 && version != 4 {
        bail!("ImproperImageHeader");
    }
    for _ in 0..5 {
        signed(&mut blob);
    }
    let levels = signed(&mut blob);
    if levels > MAX_LEVELS {
        bail!("ImproperImageHeader");
    }
    for _ in 0..4 {
        signed(&mut blob);
    }
    blob.lsb_u32();
    if version > 3 {
        signed(&mut blob);
    }
    if blob.eof {
        bail!("ImproperImageHeader");
    }
    let mut level_info = Vec::new();
    for _ in 0..levels.max(0) {
        let count = signed(&mut blob);
        if count > 50000 {
            bail!("ImproperImageHeader");
        }
        let offset = signed(&mut blob);
        blob.lsb_u32();
        if version > 3 {
            blob.lsb_u32();
            while blob.lsb_u16() != 0 {}
        }
        if blob.eof {
            bail!("UnexpectedEndOfFile");
        }
        level_info.push((count, offset));
    }
    let mut first = None;
    for (count, offset) in level_info {
        if offset < 0 {
            continue;
        }
        blob.seek(offset as i64);
        blob.eof = false;
        for _ in 0..count.max(0) {
            for _ in 0..4 {
                signed(&mut blob);
            }
            blob.lsb_u16();
            blob.lsb_u16();
            if blob.eof {
                bail!("UnexpectedEndOfFile");
            }
            let tile_length = blob.lsb_u32() as usize;
            let tile_offset = signed(&mut blob);
            if tile_offset == -1 {
                continue;
            }
            let restore = blob.tell();
            if tile_offset < 0 {
                continue;
            }
            blob.seek(tile_offset as i64);
            blob.eof = false;
            if tile_length as u64 > blob.size() {
                bail!("InsufficientImageDataInFile");
            }
            let data = blob.read(tile_length);
            if data.len() != tile_length {
                bail!("UnexpectedEndOfFile");
            }
            let mut jpeg = Vec::with_capacity(tile_length + 2);
            jpeg.extend_from_slice(&[0xff, 0xd8]);
            jpeg.extend_from_slice(data);
            let tile = super::jpeg::decode_bytes(&jpeg)?;
            blob.seek(restore as i64);
            blob.eof = false;
            if first.is_none() {
                first = Some(tile);
            }
        }
    }
    first.context("NoImagesDefined")
}
