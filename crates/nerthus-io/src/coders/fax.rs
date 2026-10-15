//! Raw Group 3 fax data as ImageMagick reads it (`coders/fax.c` and its
//! own `HuffmanDecodeImage` in `compress.c`): one-dimensional modified
//! Huffman lines, 2592 pixels wide, decoded until three empty lines in a
//! row (the end of page) or 3508 lines; the last three lines read are
//! dropped. White is index 0, black 1. IM finds codes through small hash
//! tables in which later codes can push out earlier ones, as here.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

const COLUMNS: usize = 2592;
const ROWS: usize = 3508;
const HASH_SIZE: usize = 1021;
const MB_HASH_A: usize = 293;
const MB_HASH_B: usize = 2695;
const MW_HASH_A: usize = 3510;
const MW_HASH_B: usize = 1178;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    TerminatingWhite,
    MakeupWhite,
    TerminatingBlack,
    MakeupBlack,
    Extended,
}

/// (code, length, count)
type Entry = (usize, usize, usize);

const MB_TABLE: [Entry; 27] = [
    (0x0f, 10, 64), (0xc8, 12, 128), (0xc9, 12, 192), (0x5b, 12, 256), (0x33, 12, 320), (0x34, 12, 384), (0x35, 12, 448),
    (0x6c, 13, 512), (0x6d, 13, 576), (0x4a, 13, 640), (0x4b, 13, 704), (0x4c, 13, 768), (0x4d, 13, 832), (0x72, 13, 896),
    (0x73, 13, 960), (0x74, 13, 1024), (0x75, 13, 1088), (0x76, 13, 1152), (0x77, 13, 1216), (0x52, 13, 1280),
    (0x53, 13, 1344), (0x54, 13, 1408), (0x55, 13, 1472), (0x5a, 13, 1536), (0x5b, 13, 1600), (0x64, 13, 1664),
    (0x65, 13, 1728),
];

const EX_TABLE: [Entry; 13] = [
    (0x08, 11, 1792), (0x0c, 11, 1856), (0x0d, 11, 1920), (0x12, 12, 1984), (0x13, 12, 2048), (0x14, 12, 2112),
    (0x15, 12, 2176), (0x16, 12, 2240), (0x17, 12, 2304), (0x1c, 12, 2368), (0x1d, 12, 2432), (0x1e, 12, 2496),
    (0x1f, 12, 2560),
];

const MW_TABLE: [Entry; 27] = [
    (0x1b, 5, 64), (0x12, 5, 128), (0x17, 6, 192), (0x37, 7, 256), (0x36, 8, 320), (0x37, 8, 384), (0x64, 8, 448),
    (0x65, 8, 512), (0x68, 8, 576), (0x67, 8, 640), (0xcc, 9, 704), (0xcd, 9, 768), (0xd2, 9, 832), (0xd3, 9, 896),
    (0xd4, 9, 960), (0xd5, 9, 1024), (0xd6, 9, 1088), (0xd7, 9, 1152), (0xd8, 9, 1216), (0xd9, 9, 1280),
    (0xda, 9, 1344), (0xdb, 9, 1408), (0x98, 9, 1472), (0x99, 9, 1536), (0x9a, 9, 1600), (0x18, 6, 1664),
    (0x9b, 9, 1728),
];

const TB_TABLE: [Entry; 64] = [
    (0x37, 10, 0), (0x02, 3, 1), (0x03, 2, 2), (0x02, 2, 3), (0x03, 3, 4), (0x03, 4, 5), (0x02, 4, 6), (0x03, 5, 7),
    (0x05, 6, 8), (0x04, 6, 9), (0x04, 7, 10), (0x05, 7, 11), (0x07, 7, 12), (0x04, 8, 13), (0x07, 8, 14),
    (0x18, 9, 15), (0x17, 10, 16), (0x18, 10, 17), (0x08, 10, 18), (0x67, 11, 19), (0x68, 11, 20), (0x6c, 11, 21),
    (0x37, 11, 22), (0x28, 11, 23), (0x17, 11, 24), (0x18, 11, 25), (0xca, 12, 26), (0xcb, 12, 27), (0xcc, 12, 28),
    (0xcd, 12, 29), (0x68, 12, 30), (0x69, 12, 31), (0x6a, 12, 32), (0x6b, 12, 33), (0xd2, 12, 34), (0xd3, 12, 35),
    (0xd4, 12, 36), (0xd5, 12, 37), (0xd6, 12, 38), (0xd7, 12, 39), (0x6c, 12, 40), (0x6d, 12, 41), (0xda, 12, 42),
    (0xdb, 12, 43), (0x54, 12, 44), (0x55, 12, 45), (0x56, 12, 46), (0x57, 12, 47), (0x64, 12, 48), (0x65, 12, 49),
    (0x52, 12, 50), (0x53, 12, 51), (0x24, 12, 52), (0x37, 12, 53), (0x38, 12, 54), (0x27, 12, 55), (0x28, 12, 56),
    (0x58, 12, 57), (0x59, 12, 58), (0x2b, 12, 59), (0x2c, 12, 60), (0x5a, 12, 61), (0x66, 12, 62), (0x67, 12, 63),
];

const TW_TABLE: [Entry; 64] = [
    (0x35, 8, 0), (0x07, 6, 1), (0x07, 4, 2), (0x08, 4, 3), (0x0b, 4, 4), (0x0c, 4, 5), (0x0e, 4, 6), (0x0f, 4, 7),
    (0x13, 5, 8), (0x14, 5, 9), (0x07, 5, 10), (0x08, 5, 11), (0x08, 6, 12), (0x03, 6, 13), (0x34, 6, 14),
    (0x35, 6, 15), (0x2a, 6, 16), (0x2b, 6, 17), (0x27, 7, 18), (0x0c, 7, 19), (0x08, 7, 20), (0x17, 7, 21),
    (0x03, 7, 22), (0x04, 7, 23), (0x28, 7, 24), (0x2b, 7, 25), (0x13, 7, 26), (0x24, 7, 27), (0x18, 7, 28),
    (0x02, 8, 29), (0x03, 8, 30), (0x1a, 8, 31), (0x1b, 8, 32), (0x12, 8, 33), (0x13, 8, 34), (0x14, 8, 35),
    (0x15, 8, 36), (0x16, 8, 37), (0x17, 8, 38), (0x28, 8, 39), (0x29, 8, 40), (0x2a, 8, 41), (0x2b, 8, 42),
    (0x2c, 8, 43), (0x2d, 8, 44), (0x04, 8, 45), (0x05, 8, 46), (0x0a, 8, 47), (0x0b, 8, 48), (0x52, 8, 49),
    (0x53, 8, 50), (0x54, 8, 51), (0x55, 8, 52), (0x24, 8, 53), (0x25, 8, 54), (0x58, 8, 55), (0x59, 8, 56),
    (0x5a, 8, 57), (0x5b, 8, 58), (0x4a, 8, 59), (0x4b, 8, 60), (0x32, 8, 61), (0x33, 8, 62), (0x34, 8, 63),
];

type Hash = Vec<Option<(Kind, Entry)>>;

/// `InitializeHashTable`: later entries overwrite earlier ones.
fn fill_hash(hash: &mut Hash, kind: Kind, table: &[Entry], a: usize, b: usize) {
    for &entry in table {
        let (code, length, _) = entry;
        hash[((length + a) * (code + b)) % HASH_SIZE] = Some((kind, entry));
    }
}

/// The bit reader of `InputBit`: `runlength` counts bits since the last 1.
struct Bits<'a> {
    blob: Blob<'a>,
    byte: i32,
    mask: u32,
    runlength: usize,
}

impl Bits<'_> {
    /// `InputBit`: false at the end of the file (the macro's `break`).
    fn input(&mut self, bit: &mut usize) -> bool {
        if self.mask & 0xff == 0 {
            match self.blob.byte() {
                None => {
                    self.byte = -1;
                    return false;
                }
                Some(byte) => self.byte = byte as i32,
            }
            self.mask = 0x80;
        }
        self.runlength += 1;
        *bit = usize::from(self.byte & self.mask as i32 != 0);
        self.mask >>= 1;
        if *bit != 0 {
            self.runlength = 0;
        }
        true
    }

    /// Skips to just after the next end of line (eleven zeros, then a one).
    fn skip_to_eol(&mut self, bit: &mut usize) {
        while self.runlength < 11 {
            if !self.input(bit) {
                break;
            }
        }
        loop {
            if !self.input(bit) {
                break;
            }
            if *bit != 0 {
                break;
            }
        }
    }
}

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut mw_hash: Hash = vec![None; HASH_SIZE];
    let mut mb_hash: Hash = vec![None; HASH_SIZE];
    fill_hash(&mut mw_hash, Kind::TerminatingWhite, &TW_TABLE, MW_HASH_A, MW_HASH_B);
    fill_hash(&mut mw_hash, Kind::MakeupWhite, &MW_TABLE, MW_HASH_A, MW_HASH_B);
    fill_hash(&mut mw_hash, Kind::Extended, &EX_TABLE, MW_HASH_A, MW_HASH_B);
    fill_hash(&mut mb_hash, Kind::TerminatingBlack, &TB_TABLE, MB_HASH_A, MB_HASH_B);
    fill_hash(&mut mb_hash, Kind::MakeupBlack, &MB_TABLE, MB_HASH_A, MB_HASH_B);
    fill_hash(&mut mb_hash, Kind::Extended, &EX_TABLE, MB_HASH_A, MB_HASH_B);

    let mut bits = Bits { blob: Blob::new(bytes), byte: 0, mask: 0, runlength: 0 };
    let mut bit = 0usize;
    bits.skip_to_eol(&mut bit);
    let mut indices = Vec::with_capacity(COLUMNS * 64);
    let mut scanline = vec![0u8; COLUMNS];
    let mut null_lines = 0;
    let mut y = 0usize;
    while y < ROWS && null_lines < 3 {
        scanline.fill(0);
        let mut white = true;
        let mut code = 0usize;
        let mut count: i64 = 0;
        let mut length = 0usize;
        bits.runlength = 0;
        let mut x: i64 = 0;
        loop {
            if bits.byte == -1 {
                break;
            }
            if x >= COLUMNS as i64 {
                bits.skip_to_eol(&mut bit);
                break;
            }
            let mut bail = false;
            loop {
                if bits.runlength < 11 {
                    if !bits.input(&mut bit) {
                        break;
                    }
                } else {
                    if !bits.input(&mut bit) {
                        break;
                    }
                    if bit != 0 {
                        null_lines += 1;
                        if x != 0 {
                            null_lines = 0;
                        }
                        bail = true;
                        break;
                    }
                }
                code = (code << 1) + bit;
                length += 1;
                if code != 0 {
                    break;
                }
            }
            if bail {
                break;
            }
            if length > 13 {
                bits.skip_to_eol(&mut bit);
                break;
            }
            let entry = if white {
                if length < 4 {
                    continue;
                }
                mw_hash[((length + MW_HASH_A) * (code + MW_HASH_B)) % HASH_SIZE]
            } else {
                if length < 2 {
                    continue;
                }
                mb_hash[((length + MB_HASH_A) * (code + MB_HASH_B)) % HASH_SIZE]
            };
            let Some((kind, (entry_code, entry_length, entry_count))) = entry else {
                continue;
            };
            if entry_length != length || entry_code != code {
                continue;
            }
            match kind {
                Kind::TerminatingWhite | Kind::TerminatingBlack => {
                    count += entry_count as i64;
                    if x + count > COLUMNS as i64 {
                        count = COLUMNS as i64 - x;
                    }
                    if count > 0 {
                        if white {
                            x += count;
                            count = 0;
                        } else {
                            while count > 0 {
                                if x >= 0 && x < COLUMNS as i64 {
                                    scanline[x as usize] = 1;
                                    x += 1;
                                }
                                count -= 1;
                            }
                        }
                    }
                    white = !white;
                }
                Kind::MakeupWhite | Kind::MakeupBlack | Kind::Extended => {
                    count += entry_count as i64;
                }
            }
            code = 0;
            length = 0;
        }
        indices.extend_from_slice(&scanline);
        y += 1;
    }
    if bits.blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    if y < 3 {
        bail!("ImproperImageHeader");
    }
    let rows = (y - 3).max(1);
    let mut data = Vec::with_capacity(COLUMNS * rows * 4);
    for &index in &indices[..COLUMNS * rows] {
        let level = if index == 0 { 65535.0 } else { 0.0 };
        data.extend([level, level, level, index as f32]);
    }
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Index];
    let mut cache = PixelCache::new(COLUMNS, rows, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA);
    cache.colormap = vec![[65535.0, 65535.0, 65535.0, 65535.0], [0.0, 0.0, 0.0, 65535.0]];
    Ok(cache)
}
