//! Metadata files as ImageMagick reads them (`coders/meta.c`): an ICC
//! profile, XMP packet, IPTC or Exif block or Photoshop resource file opens
//! as one white pixel carrying it. Photoshop resource *text* fails when no
//! line gives a record, and APP1JPEG always fails, as it needs an IPTC
//! profile a plain read never has.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

pub fn read(path: &Path, name: &str) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes, name)
}

pub fn decode(bytes: &[u8], name: &str) -> Result<PixelCache> {
    match name {
        "8BIMTEXT" if !has_record(bytes, false) => bail!("CorruptImage"),
        "8BIMWTEXT" if !has_record(bytes, true) => bail!("CorruptImage"),
        "APP1JPEG" => bail!("NoIPTCProfileAvailable"),
        _ => {}
    }
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue];
    let white = vec![65535.0; 3];
    Ok(PixelCache::new(1, 1, channels, white, CacheColorspace::SRgb, DEFAULT_GAMMA))
}

/// `super_fgets` (bytes) or `super_fgets_w` (little-endian 16-bit units,
/// their low bytes kept): a line up to a newline, none when it's empty.
fn next_line(blob: &mut Blob, wide: bool) -> Option<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        if wide {
            let unit = blob.lsb_u16() as i16;
            if unit == -1 || unit == 0x0a || blob.eof {
                break;
            }
            line.push(unit as u8);
        } else {
            match blob.byte() {
                None | Some(b'\n') => break,
                Some(byte) => line.push(byte),
            }
        }
    }
    if line.is_empty() {
        return None;
    }
    // The line is a C string from here on.
    if let Some(end) = line.iter().position(|&byte| byte == 0) {
        line.truncate(end);
    }
    Some(line)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TokenState {
    White,
    Token,
    Quote,
    Ozone,
}

/// IM's `Tokenizer` with no white space or escape: the next token ended by
/// one of `breaks` (outside `quotes`), or none at the end of the line.
fn tokenize(line: &[u8], next: &mut usize, breaks: &[u8], quotes: &[u8]) -> Option<Vec<u8>> {
    if *next >= line.len() {
        return None;
    }
    let mut token = Vec::new();
    let mut state = TokenState::White;
    let mut quote = 0u8;
    while *next < line.len() {
        let c = line[*next];
        if breaks.contains(&c) {
            if state == TokenState::Quote {
                token.push(c);
            } else {
                *next += 1;
                return Some(token);
            }
        } else if quotes.contains(&c) {
            match state {
                TokenState::White => {
                    state = TokenState::Quote;
                    quote = c;
                }
                TokenState::Quote => {
                    if c != quote {
                        token.push(c);
                    } else {
                        state = TokenState::Ozone;
                    }
                }
                TokenState::Token | TokenState::Ozone => return Some(token),
            }
        } else {
            match state {
                TokenState::White => {
                    state = TokenState::Token;
                    token.push(c);
                }
                TokenState::Token | TokenState::Quote => token.push(c),
                TokenState::Ozone => return Some(token),
            }
        }
        *next += 1;
    }
    Some(token)
}

/// `StringToLong`: `strtol` with a 32-bit `long`, as an unsigned char.
fn dataset_number(text: &[u8]) -> u8 {
    let mut i = 0;
    while i < text.len() && matches!(text[i], b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
        i += 1;
    }
    let negative = text.get(i) == Some(&b'-');
    if matches!(text.get(i), Some(b'-' | b'+')) {
        i += 1;
    }
    let mut value: i64 = 0;
    while let Some(digit) = text.get(i).filter(|byte| byte.is_ascii_digit()) {
        value = (value * 10 + (digit - b'0') as i64).min(i64::from(i32::MAX) + 1);
        i += 1;
    }
    let signed = if negative { -value } else { value };
    let long = signed.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    long as u8
}

/// Whether `parse8BIM` (or `parse8BIMW`) writes anything: some line before
/// the first empty one has a value after its `=`, and is either a Photoshop
/// resource (dataset "8BIM") or short enough for an IPTC record. Shrinking
/// a value by its HTML entities isn't counted (it matters only past 32 KiB).
fn has_record(bytes: &[u8], wide: bool) -> bool {
    let mut blob = Blob::new(bytes);
    let mut dataset = 0u8;
    while let Some(line) = next_line(&mut blob, wide) {
        let mut next = 0;
        let mut state = 0;
        while let Some(token) = tokenize(&line, &mut next, b"=", b"\"") {
            if state == 0 {
                let mut inner = 0;
                if let Some(piece) = tokenize(&token, &mut inner, b"#", b"") {
                    dataset = if piece == b"8BIM" { 255 } else { dataset_number(&piece) };
                }
            } else if state == 1 && (dataset == 255 || token.len() <= 0x7fff) {
                return true;
            }
            state += 1;
        }
    }
    false
}
