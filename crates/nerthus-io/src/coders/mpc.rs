//! Magick Persistent Cache images as ImageMagick reads them
//! (`coders/mpc.c`): a MIFF-like `key=value` header in the `.mpc` file and
//! the pixel cache itself, raw single-precision quanta in IM's channel
//! order, in a `.cache` file beside it. A cache only opens in the IM build
//! that wrote it (its `magick-signature`), and later images in the file are
//! checked as well, at page-aligned offsets in the cache.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

/// `GetMagickSignature("MagickPixelCache")` for a Q16 HDRI build with
/// 64-bit channel masks (library interface 10, little-endian).
const MAGICK_SIGNATURE: u32 = 785_653_784;

/// `GetMagickPageSize` on x64 Windows.
const PAGE_SIZE: u64 = 4096;

/// `CompressOptions`, bar "Undefined".
const COMPRESSIONS: [&str; 29] = [
    "B44A", "B44", "BZip", "DWAA", "DWAB", "DXT1", "DXT3", "DXT5", "BC7", "Fax", "Group4", "JBIG1", "JBIG2", "JPEG2000", "JPEG", "LERC",
    "LosslessJPEG", "Lossless", "LZMA", "LZW", "None", "Piz", "Pxr24", "RLE", "RunlengthEncoded", "WebP", "ZipS", "Zip", "Zstd",
];

/// `MaxPixelChannels` and `MetaPixelChannels`.
const MAX_PIXEL_CHANNELS: u64 = 64;
const META_PIXEL_CHANNELS: u64 = 10;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    let cache_path = cache_file_name(path);
    decode(&bytes, &cache_path)
}

/// `AppendImageFormat("cache", filename)`: the name with its extension
/// (and a compression suffix before it) replaced.
fn cache_file_name(path: &Path) -> PathBuf {
    let mut name = path.to_string_lossy().to_string();
    let strip = |name: &str| -> String {
        let separator = name.rfind(['/', '\\']).map_or(0, |index| index + 1);
        match name[separator..].rfind('.') {
            Some(dot) if dot > 0 => name[..separator + dot].to_string(),
            _ => name.to_string(),
        }
    };
    let extension = name.rsplit_once('.').map(|(_, extension)| extension.to_ascii_lowercase());
    let compressed = matches!(extension.as_deref(), Some("z" | "bz2" | "gz" | "wmz" | "svgz"));
    if compressed {
        name = strip(&name);
    }
    PathBuf::from(format!("{}.cache", strip(&name)))
}

/// `StringToUnsignedLong`: `strtoul` with a 32-bit `unsigned long`.
fn unsigned_long(text: &str) -> u64 {
    let text = text.trim_start();
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let end = digits.bytes().position(|byte| !byte.is_ascii_digit()).unwrap_or(digits.len());
    let mut value: u64 = 0;
    for byte in digits[..end].bytes() {
        value = (value * 10 + (byte - b'0') as u64).min(u64::from(u32::MAX) + 1);
    }
    if value > u64::from(u32::MAX) {
        return u64::from(u32::MAX);
    }
    if negative {
        return u64::from((value as u32).wrapping_neg());
    }
    value
}

/// One image's header, as `ReadMPCImage` keeps it.
struct Header {
    id: String,
    pseudo: bool,
    undefined_class: bool,
    alpha: bool,
    colors: u64,
    colorspace: CacheColorspace,
    undefined_compression: bool,
    columns: u64,
    rows: u64,
    depth: u64,
    gamma: f64,
    signature: u32,
    montage: bool,
    number_channels: u64,
    number_meta_channels: u64,
    profiles: usize,
}

impl Header {
    fn new() -> Header {
        Header {
            id: String::new(),
            pseudo: false,
            undefined_class: false,
            alpha: false,
            colors: 0,
            colorspace: CacheColorspace::SRgb,
            undefined_compression: false,
            columns: 0,
            rows: 0,
            depth: 8,
            gamma: DEFAULT_GAMMA,
            signature: 0,
            montage: false,
            number_channels: 3,
            number_meta_channels: 0,
            profiles: 0,
        }
    }

    fn assign(&mut self, keyword: &str, value: &str) {
        let keyword = keyword.to_ascii_lowercase();
        match keyword.as_str() {
            "alpha-trait" => {
                const TRAITS: [&str; 4] = ["undefined", "blend", "copy", "update"];
                if let Some(index) = TRAITS.iter().position(|name| name.eq_ignore_ascii_case(value)) {
                    self.alpha = index != 0;
                }
            }
            "class" => {
                if value.eq_ignore_ascii_case("DirectClass") {
                    self.pseudo = false;
                    self.undefined_class = false;
                } else if value.eq_ignore_ascii_case("PseudoClass") {
                    self.pseudo = true;
                    self.undefined_class = false;
                } else if value.eq_ignore_ascii_case("Undefined") {
                    self.pseudo = false;
                    self.undefined_class = true;
                }
            }
            "colors" => self.colors = unsigned_long(value),
            "colorspace" => {
                if let Some(colorspace) = CacheColorspace::from_option_name(value) {
                    self.colorspace = colorspace;
                }
            }
            "compression" => {
                if value.eq_ignore_ascii_case("Undefined") {
                    self.undefined_compression = true;
                } else if COMPRESSIONS.iter().any(|name| name.eq_ignore_ascii_case(value)) {
                    self.undefined_compression = false;
                }
            }
            "columns" => self.columns = unsigned_long(value),
            "depth" => self.depth = unsigned_long(value),
            "gamma" => self.gamma = super::text::strtod(value.as_bytes(), 0).0,
            "id" => self.id = value.to_string(),
            "magick-signature" => self.signature = unsigned_long(value) as u32,
            "montage" => self.montage = true,
            "number-channels" => self.number_channels = unsigned_long(value),
            "number-meta-channels" => self.number_meta_channels = unsigned_long(value),
            "profile" => self.profiles += 1,
            "rows" => self.rows = unsigned_long(value),
            _ => {}
        }
    }

    /// The pixel cache's channels (`ResetPixelChannelMap`).
    fn channels(&self) -> Vec<PixelChannel> {
        let mut channels = match self.colorspace {
            CacheColorspace::Gray | CacheColorspace::LinearGray => vec![PixelChannel::Gray],
            _ => vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue],
        };
        if self.colorspace == CacheColorspace::Cmyk {
            channels.push(PixelChannel::Black);
        }
        if self.alpha {
            channels.push(PixelChannel::Alpha);
        }
        if self.pseudo {
            channels.push(PixelChannel::Index);
        }
        for _ in 0..self.number_meta_channels {
            channels.push(PixelChannel::Meta);
        }
        channels
    }
}

fn is_graph(c: Option<u8>) -> bool {
    c.is_some_and(|byte| byte.is_ascii_graphic())
}

fn is_space(c: Option<u8>) -> bool {
    c.is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'))
}

/// The header loop: `{comments}` and `key=value` pairs up to the `:`, then
/// the byte after it.
fn parse_header(blob: &mut Blob, first: Option<u8>) -> Header {
    let mut header = Header::new();
    let mut c = first;
    while is_graph(c) && c != Some(b':') {
        if c == Some(b'{') {
            loop {
                c = blob.byte();
                if c == Some(b'\\') {
                    // The escaped character only goes into the comment.
                    blob.byte();
                } else if c.is_none() || c == Some(b'}') {
                    break;
                }
            }
            c = blob.byte();
        } else if c.is_some_and(|byte| byte.is_ascii_alphanumeric()) {
            let mut keyword = Vec::new();
            while let Some(byte) = c {
                if byte == b'=' {
                    break;
                }
                if keyword.len() < 4095 {
                    keyword.push(byte);
                }
                c = blob.byte();
            }
            while is_space(c) {
                c = blob.byte();
            }
            let mut value = Vec::new();
            if c == Some(b'=') {
                c = blob.byte();
                if c == Some(b'{') {
                    value.push(b'{');
                    c = blob.byte();
                }
                while let Some(byte) = c.filter(|byte| *byte != b'{' && *byte != b'}') {
                    value.push(byte);
                    c = blob.byte();
                    if c == Some(b'\\') {
                        c = blob.byte();
                        if let Some(brace) = c.filter(|byte| *byte == b'{' || *byte == b'}') {
                            value.push(brace);
                            c = blob.byte();
                        }
                    }
                    if value.first() != Some(&b'{') && is_space(c) {
                        break;
                    }
                }
            }
            if value.first() == Some(&b'{') {
                value.remove(0);
            }
            let end = value.iter().position(|&byte| byte == 0).unwrap_or(value.len());
            let keyword_end = keyword.iter().position(|&byte| byte == 0).unwrap_or(keyword.len());
            let keyword = String::from_utf8_lossy(&keyword[..keyword_end]).to_string();
            let value = String::from_utf8_lossy(&value[..end]).to_string();
            header.assign(&keyword, &value);
        } else {
            c = blob.byte();
        }
        while is_space(c) {
            c = blob.byte();
        }
    }
    blob.byte();
    header
}

/// `GetImageQuantumDepth(image, MagickFalse)`.
fn quantum_depth(depth: u64) -> u64 {
    match depth {
        0..=8 => 8,
        9..=16 => 16,
        17..=32 => 32,
        _ => 64,
    }
}

pub fn decode(bytes: &[u8], cache_path: &Path) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let mut c = blob.byte();
    if c.is_none() {
        bail!("ImproperImageHeader");
    }
    let cache_size = std::fs::metadata(cache_path).map(|metadata| metadata.len()).ok();
    let mut offset = 0u64;
    let mut first: Option<PixelCache> = None;
    loop {
        let header = parse_header(&mut blob, c);
        let channels = header.channels();
        let invalid = header.id != "MagickPixelCache"
            || header.undefined_class
            || header.undefined_compression
            || header.columns == 0
            || header.rows == 0
            || header.number_channels > MAX_PIXEL_CHANNELS
            || header.number_meta_channels > MAX_PIXEL_CHANNELS - META_PIXEL_CHANNELS
            || header.number_channels + header.number_meta_channels >= MAX_PIXEL_CHANNELS
            || header.depth == 0
            || header.depth > 64;
        if invalid {
            bail!("ImproperImageHeader");
        }
        if header.signature != MAGICK_SIGNATURE {
            bail!("IncompatibleAPI");
        }
        if header.montage {
            // The image directory, to its NUL.
            while let Some(byte) = blob.byte() {
                if byte == 0 {
                    break;
                }
            }
        }
        for _ in 0..header.profiles {
            let length = blob.msb_u32() as u64;
            if length > blob.size() {
                break;
            }
            if blob.read(length as usize).len() as u64 != length {
                break;
            }
        }
        let mut colormap = Vec::new();
        if header.pseudo {
            let depth = quantum_depth(header.depth);
            let packet_size = 3 * depth / 8;
            if packet_size * header.colors > blob.size() {
                bail!("InsufficientImageDataInFile");
            }
            if header.colors != 0 {
                let length = (packet_size * header.colors) as usize;
                let packed = blob.read(length);
                if packed.len() != length {
                    bail!("InsufficientImageDataInFile");
                }
                for entry in packed.chunks_exact(packet_size as usize) {
                    let mut values = [0.0f64; 4];
                    values[3] = 65535.0;
                    for (channel, value) in values[..3].iter_mut().enumerate() {
                        *value = match depth {
                            8 => entry[channel] as f64 * 257.0,
                            16 => u16::from_be_bytes([entry[2 * channel], entry[2 * channel + 1]]) as f64,
                            32 => {
                                let word = u32::from_be_bytes([entry[4 * channel], entry[4 * channel + 1], entry[4 * channel + 2], entry[4 * channel + 3]]);
                                (word as f64 / 65537.0) as f32 as f64
                            }
                            _ => bail!("ImageDepthNotSupported"),
                        };
                    }
                    colormap.push(values);
                }
            }
        }
        // Attach the cache: it must hold this image's pixels at `offset`.
        let length = header.columns * header.rows * channels.len() as u64 * 4;
        let Some(cache_size) = cache_size else {
            bail!("UnableToOpenPixelCache");
        };
        if cache_size < offset + length {
            bail!("UnableToExtendCache");
        }
        if first.is_none() {
            let cache = std::fs::read(cache_path).context("UnableToOpenPixelCache")?;
            let pixels = &cache[offset as usize..(offset + length) as usize];
            let data: Vec<f32> = pixels.as_chunks::<4>().0.iter().map(|quad| f32::from_le_bytes([quad[0], quad[1], quad[2], quad[3]])).collect();
            let mut image = PixelCache::new(header.columns as usize, header.rows as usize, channels, data, header.colorspace, header.gamma);
            image.colormap = colormap;
            first = Some(image);
        }
        offset += length + PAGE_SIZE - length % PAGE_SIZE;
        if blob.eof {
            bail!("UnexpectedEndOfFile");
        }
        // The next image starts at the next graphic character.
        loop {
            c = blob.byte();
            if c.is_none() || is_graph(c) {
                break;
            }
        }
        if !matches!(c, Some(b'i' | b'I')) {
            break;
        }
    }
    first.context("ImproperImageHeader")
}
