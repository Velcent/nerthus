//! VIPS native images as ImageMagick reads them (`coders/vips.c`): a
//! 64-byte header in either byte order, then uncoded samples, a pixel's
//! bands together: grey (`B_W`, `GREY16`), linear RGB (`RGB`, `RGB16`),
//! sRGB or CMYK, an alpha band after them. `B_W` and `RGB` keep only the
//! low byte of wider samples and CMYK drops its black band, as IM does,
//! and a short file reads on past its end.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::blob::Blob;
use super::color;
use crate::pixel_cache::{char_quantum, CacheColorspace, PixelCache, PixelChannel, DEFAULT_GAMMA};

const MAGIC_LSB: u32 = 0x08f2_a6b6;
const MAGIC_MSB: u32 = 0xb6a6_f208;
const QUANTUM_RANGE: f64 = 65535.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum BandFormat {
    Char,
    Short,
    Int,
    Float,
    Double,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VipsType {
    BlackWhite,
    Cmyk,
    Rgb,
    SRgb,
    Grey16,
    Rgb16,
}

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    decode(&bytes)
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

    fn float(&mut self) -> f32 {
        f32::from_bits(self.long())
    }

    fn double(&mut self) -> f64 {
        let bytes = self.blob.read(8);
        if bytes.len() != 8 {
            return 0.0;
        }
        let array: [u8; 8] = bytes.try_into().unwrap();
        if self.big_endian { f64::from_be_bytes(array) } else { f64::from_le_bytes(array) }
    }

    /// `ReadVIPSPixelNONE`.
    fn pixel(&mut self, format: BandFormat, kind: VipsType) -> f32 {
        match kind {
            VipsType::BlackWhite | VipsType::Rgb => {
                let value = match format {
                    BandFormat::Char => self.blob.byte().unwrap_or(0xff),
                    BandFormat::Short => self.short() as u8,
                    BandFormat::Int => self.long() as u8,
                    BandFormat::Float => self.float() as f64 as u8,
                    BandFormat::Double => self.double() as u8,
                };
                char_quantum(value)
            }
            VipsType::Grey16 | VipsType::Rgb16 => {
                let value = match format {
                    BandFormat::Char => 0,
                    BandFormat::Short => self.short(),
                    BandFormat::Int => self.long() as u16,
                    BandFormat::Float => self.float() as f64 as u16,
                    BandFormat::Double => self.double() as u16,
                };
                value as f32
            }
            VipsType::Cmyk | VipsType::SRgb => match format {
                BandFormat::Char => char_quantum(self.blob.byte().unwrap_or(0xff)),
                BandFormat::Short => self.short() as f32,
                BandFormat::Int => (self.long() as f64 / 65537.0) as f32,
                BandFormat::Float => (QUANTUM_RANGE * (self.float() as f64 / 1.0)) as f32,
                BandFormat::Double => (QUANTUM_RANGE * (self.double() / 1.0)) as f32,
            },
        }
    }
}

pub fn decode(bytes: &[u8]) -> Result<PixelCache> {
    let mut blob = Blob::new(bytes);
    let big_endian = match blob.lsb_u32() {
        MAGIC_LSB => false,
        MAGIC_MSB => true,
        _ => bail!("ImproperImageHeader"),
    };
    let mut reader = Reader { blob, big_endian };
    let columns = reader.long() as usize;
    let rows = reader.long() as usize;
    if columns == 0 || rows == 0 {
        bail!("NegativeOrZeroImageSize");
    }
    let channels = reader.long();
    let _legacy = reader.long();
    let format = match reader.long() {
        0 | 1 => BandFormat::Char,
        2 | 3 => BandFormat::Short,
        4 | 5 => BandFormat::Int,
        6 => BandFormat::Float,
        8 => BandFormat::Double,
        _ => bail!("Unsupported band format"),
    };
    let coding = reader.long();
    let kind = match reader.long() {
        15 => VipsType::Cmyk,
        1 => VipsType::BlackWhite,
        26 => VipsType::Grey16,
        17 => VipsType::Rgb,
        25 => VipsType::Rgb16,
        22 => VipsType::SRgb,
        _ => bail!("Unsupported colorspace"),
    };
    let (colorspace, gamma, alpha) = match kind {
        VipsType::Cmyk => (CacheColorspace::Cmyk, DEFAULT_GAMMA, channels == 5),
        VipsType::BlackWhite | VipsType::Grey16 => (CacheColorspace::Gray, DEFAULT_GAMMA, channels == 2),
        VipsType::Rgb | VipsType::Rgb16 => (CacheColorspace::Rgb, 1.0, channels == 4),
        VipsType::SRgb => (CacheColorspace::SRgb, DEFAULT_GAMMA, channels == 4),
    };
    let _resolution_x = reader.float();
    let _resolution_y = reader.float();
    for _ in 0..3 {
        reader.blob.read(8);
    }
    let wide_ok = matches!(format, BandFormat::Short | BandFormat::Int | BandFormat::Float | BandFormat::Double);
    if matches!(kind, VipsType::Grey16 | VipsType::Rgb16) && !wide_ok {
        bail!("Unsupported combination of band format and colorspace");
    }
    if channels == 0 || channels > 5 {
        bail!("Unsupported number of channels");
    }
    if coding != 0 {
        bail!("Unsupported coding");
    }
    // Reading on past the end of the file gives samples, not an error;
    // a file that short can't be meant to be this big.
    let sample_bytes = match format {
        BandFormat::Char => 1,
        BandFormat::Short => 2,
        BandFormat::Int | BandFormat::Float => 4,
        BandFormat::Double => 8,
    };
    let needed = (columns as u64).saturating_mul(rows as u64).saturating_mul(channels as u64 * sample_bytes);
    if needed > (reader.blob.size() + 1).saturating_mul(1 << 12) {
        bail!("UnexpectedEndOfFile");
    }

    let gray = colorspace == CacheColorspace::Gray;
    let cmyk = colorspace == CacheColorspace::Cmyk;
    let mut layout = if gray { vec![PixelChannel::Gray] } else { vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue] };
    if cmyk {
        layout.push(PixelChannel::Black);
    }
    if alpha {
        layout.push(PixelChannel::Alpha);
    }
    let stride = layout.len();
    let (red, green, blue) = if gray { (0, 0, 0) } else { (0, 1, 2) };
    let alpha_offset = alpha.then_some(stride - 1);
    // `SetImageBackgroundColor`: white (in CMYK, as `ConvertRGBToCMYK`
    // makes it).
    let mut data = vec![QUANTUM_RANGE as f32; columns * rows * stride];
    if cmyk {
        let white = color::srgb_to_cmyk(QUANTUM_RANGE, QUANTUM_RANGE, QUANTUM_RANGE);
        for pixel in data.chunks_exact_mut(stride) {
            for (sample, &value) in pixel.iter_mut().zip(white.iter()) {
                *sample = value as f32;
            }
        }
    }
    for pixel in data.chunks_exact_mut(stride) {
        let value = reader.pixel(format, kind);
        pixel[red] = value;
        if channels < 3 {
            pixel[green] = value;
            pixel[blue] = value;
            if channels == 2 {
                let second = reader.pixel(format, kind);
                if let Some(offset) = alpha_offset {
                    pixel[offset] = second;
                }
            }
        } else {
            pixel[green] = reader.pixel(format, kind);
            pixel[blue] = reader.pixel(format, kind);
            if channels == 4 {
                // CMYK's fourth band goes to an index channel it doesn't have.
                let fourth = reader.pixel(format, kind);
                if !cmyk && let Some(offset) = alpha_offset {
                    pixel[offset] = fourth;
                }
            } else if channels == 5 {
                let _index = reader.pixel(format, kind);
                let fifth = reader.pixel(format, kind);
                if let Some(offset) = alpha_offset {
                    pixel[offset] = fifth;
                }
            }
        }
    }
    Ok(PixelCache::new(columns, rows, layout, data, colorspace, gamma))
}
