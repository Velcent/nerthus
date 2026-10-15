//! DPX images as ImageMagick reads them (`coders/dpx.c`): the file and
//! image headers in either byte order, then each image element (every
//! element into the same image, so the last one is what's left): luma,
//! RGB, RGBA (ABGR read as RGBA) or CbYCr 4:2:2 / 4:4:4, at 1 to 32 bits,
//! packed or filled into 32-bit words. Logarithmic and printing-density
//! elements are Log colourspace, CbYCr ones Rec. 709 YCbCr.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::quantum::{self, Endian, Picture, QuantumInfo, QuantumType};
use crate::pixel_cache::{CacheColorspace, PixelCache, DEFAULT_GAMMA};

const LUMA: u8 = 6;
const RGB: u8 = 50;
const RGBA: u8 = 51;
const ABGR: u8 = 52;
const CBYCRY_422: u8 = 100;
const CBYACRYA_4224: u8 = 101;
const CBYCR_444: u8 = 102;
const PRINTING_DENSITY: u8 = 1;
const LOGARITHMIC: u8 = 3;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
}

#[derive(Default, Clone, Copy)]
struct Element {
    descriptor: u8,
    transfer: u8,
    bit_size: u8,
    packing: u16,
    data_offset: u32,
}

/// `GetBytesPerRow`.
fn bytes_per_row(columns: usize, samples: usize, bits: usize, pad: bool) -> usize {
    match bits {
        10 if pad => 4 * (32 * (samples * columns).div_ceil(3)).div_ceil(32),
        12 if pad => 2 * (16 * samples * columns).div_ceil(16),
        16 if !pad => 2 * (samples * columns * bits).div_ceil(16),
        64 => 8 * (samples * columns * bits).div_ceil(64),
        _ => 4 * (samples * columns * bits).div_ceil(32),
    }
}

struct Reader<'a> {
    blob: Blob<'a>,
    big_endian: bool,
}

impl Reader<'_> {
    fn short(&mut self) -> u16 {
        if self.big_endian { self.blob.msb_u16() } else { self.blob.lsb_u16() }
    }

    fn long(&mut self) -> u32 {
        if self.big_endian { self.blob.msb_u32() } else { self.blob.lsb_u32() }
    }

    fn byte(&mut self) -> u8 {
        self.blob.byte().unwrap_or(0xff)
    }

    fn skip(&mut self, count: usize) {
        self.blob.read(count);
    }
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let magic = blob.read(4);
    let big_endian = if magic.eq_ignore_ascii_case(b"SDPX") {
        true
    } else if magic.eq_ignore_ascii_case(b"XPDS") {
        false
    } else {
        bail!("ImproperImageHeader");
    };
    let mut reader = Reader { blob, big_endian };

    // The file header (768 bytes).
    let image_offset = reader.long();
    reader.skip(8);
    let _file_size = reader.long();
    let _ditto_key = reader.long();
    let _generic_size = reader.long();
    let _industry_size = reader.long();
    let user_size = reader.long();
    reader.skip(100 + 24 + 100 + 200 + 200);
    let _encrypt_key = reader.long();
    reader.skip(104);

    // The image header (640 bytes).
    let orientation = reader.short();
    if orientation > 7 {
        bail!("ImproperImageHeader");
    }
    let number_elements = reader.short();
    if !(1..=8).contains(&number_elements) {
        bail!("ImproperImageHeader");
    }
    let columns = reader.long() as usize;
    let rows = reader.long() as usize;
    let mut elements = [Element::default(); 8];
    for element in elements.iter_mut() {
        let _data_sign = reader.long();
        let _low_data = reader.long();
        let _low_quantity = reader.long();
        let _high_data = reader.long();
        let _high_quantity = reader.long();
        element.descriptor = reader.byte();
        element.transfer = reader.byte();
        let _colorimetric = reader.byte();
        element.bit_size = reader.byte();
        element.packing = reader.short();
        let _encoding = reader.short();
        element.data_offset = reader.long();
        let _end_of_line_padding = reader.long();
        let _end_of_image_padding = reader.long();
        reader.skip(32);
    }
    reader.skip(52);
    let mut offset: u64 = 1408;
    if image_offset >= 1664 {
        reader.skip(256);
        offset += 256;
    }
    if image_offset >= 1920 {
        reader.skip(256);
        offset += 256;
    }
    if image_offset >= 2048 {
        reader.skip(128);
        offset += 128;
    }
    if image_offset > 2080 {
        offset += reader.blob.read(32).len() as u64;
        if user_size != u32::MAX && user_size as usize > 32 {
            let length = user_size as usize - 32;
            if length as u64 > reader.blob.size() {
                bail!("InsufficientImageDataInFile");
            }
            offset += reader.blob.read(length).len() as u64;
        }
    }
    while offset < image_offset as u64 {
        if reader.blob.byte().is_none() {
            break;
        }
        offset += 1;
    }
    if reader.blob.eof {
        bail!("UnexpectedEndOfFile");
    }
    if columns == 0 || rows == 0 {
        bail!("NegativeOrZeroImageSize");
    }

    // Every element goes into the same image: the last one's colourspace
    // and samples are what's left, an alpha channel once added stays.
    let mut colorspace = CacheColorspace::Rgb;
    let mut alpha = false;
    let mut picture = Picture::new(columns, rows, false, false, false, false, 0);
    picture.reset();
    let mut image_type = crate::pixel_cache::ImageType::Undefined;
    for element in elements.iter().take(number_elements as usize) {
        if element.data_offset != u32::MAX && element.data_offset != 0 {
            let data_offset = element.data_offset as u64;
            if data_offset < offset {
                reader.blob.seek(data_offset as i64);
                offset = data_offset;
            } else {
                while offset < data_offset {
                    if reader.blob.byte().is_none() {
                        break;
                    }
                    offset += 1;
                }
            }
            if offset != data_offset {
                bail!("UnableToReadImageData");
            }
        }
        let depth = element.bit_size as u32;
        if depth == 0 || depth > 32 {
            bail!("ImproperImageHeader");
        }
        let (samples, kind) = match element.descriptor {
            CBYCRY_422 => (2, QuantumType::CbYCrY),
            CBYACRYA_4224 | CBYCR_444 => (3, QuantumType::Rgb),
            RGB => (3, QuantumType::Rgb),
            ABGR | RGBA => {
                alpha = true;
                (4, QuantumType::Rgba)
            }
            _ => (1, QuantumType::Gray),
        };
        let swap_red_green = matches!(element.descriptor, CBYACRYA_4224 | CBYCR_444);
        colorspace = match element.descriptor {
            CBYCRY_422 | CBYACRYA_4224 | CBYCR_444 => CacheColorspace::Rec709YCbCr,
            LUMA => CacheColorspace::Gray,
            _ if element.transfer == LOGARITHMIC || element.transfer == PRINTING_DENSITY => CacheColorspace::Log,
            _ => CacheColorspace::SRgb,
        };
        let gray = colorspace == CacheColorspace::Gray;
        picture = relayout(picture, gray, alpha);
        let pad = element.packing != 0;
        let extent = bytes_per_row(columns, samples, depth as usize, pad);
        let mut info = QuantumInfo::new(depth, false);
        info.endian = if big_endian { Endian::Msb } else { Endian::Lsb };
        info.quantum = 32;
        info.pack = !pad;
        let remaining = reader.blob.size().saturating_sub(reader.blob.tell());
        if (extent as u64).saturating_mul(rows as u64) > remaining {
            bail!("UnableToReadImageData");
        }
        for row in 0..rows {
            let data = reader.blob.read(extent);
            quantum::import(&mut picture, &info, kind, row * columns, columns, data)?;
            if swap_red_green {
                for pixel in row * columns..(row + 1) * columns {
                    let base = pixel * picture.stride;
                    picture.data.swap(base + picture.red, base + picture.green);
                }
            }
        }
        image_type = quantum::image_type(kind, depth);
    }
    let gray = colorspace == CacheColorspace::Gray;
    Ok(picture.into_cache(gray, colorspace, DEFAULT_GAMMA, image_type))
}

/// The picture with a new channel layout (`SetImageColorspace` and alpha
/// changes between elements): samples kept by channel, grey taking red.
fn relayout(picture: Picture, gray: bool, alpha: bool) -> Picture {
    let was_gray = picture.red == picture.green && picture.stride > 0 && picture.green == picture.blue;
    if was_gray == gray && picture.alpha == alpha {
        return picture;
    }
    let mut next = Picture::new(picture.width, picture.height, gray, false, alpha, false, 0);
    for pixel in 0..picture.width * picture.height {
        let source = pixel * picture.stride;
        let target = pixel * next.stride;
        if gray {
            next.data[target + next.red] = picture.data[source + picture.red];
        } else {
            next.data[target + next.red] = picture.data[source + picture.red];
            next.data[target + next.green] = picture.data[source + picture.green];
            next.data[target + next.blue] = picture.data[source + picture.blue];
        }
        if let (Some(from), Some(to)) = (picture.alpha_offset, next.alpha_offset) {
            next.data[target + to] = picture.data[source + from];
        }
    }
    next
}
