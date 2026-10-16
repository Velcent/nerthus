//! Image files decoded into what ImageMagick 7.1.2 (Q16 HDRI, through
//! Magick.NET) holds after reading them: the pixel cache's channels in
//! their order — palette images (`PseudoClass`) carry an index channel —
//! as quanta (floats in 0..65535), with the colorspace and gamma IM
//! assigns. The File node reads the cache raw, so these details reach its
//! maps.

use std::path::Path;

use anyhow::{bail, Context, Result};

/// IM's default image gamma (`1.000/2.200`).
pub(crate) const DEFAULT_GAMMA: f64 = 1.0 / 2.2;

/// One channel of the pixel cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelChannel {
    Gray,
    Red,
    Green,
    Blue,
    Alpha,
    /// The colormap index of a `PseudoClass` image.
    Index,
    /// A CMYK image's black.
    Black,
    /// A meta channel (a PSD's extra channels).
    Meta,
}

/// The colorspace IM gives an image (`ColorspaceType`): sRGB for most
/// files, linear RGB, grey and linear grey, CMYK (ink amounts, not
/// negated), CIE Lab and XYZ, YCbCr, and any other a MIFF names.
pub use nerthus_image::colorspace_transform::Colorspace as CacheColorspace;
pub use nerthus_image::colorspace_transform::TransformSettings;

/// `image->type` as a reader leaves it. Writing pixels resets it to
/// `UndefinedType`, so only readers that set it after their pixels give
/// anything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageType {
    Undefined,
    Bilevel,
    Grayscale,
    GrayscaleAlpha,
    Palette,
    PaletteAlpha,
    TrueColor,
    TrueColorAlpha,
    ColorSeparation,
}

/// An image as IM's pixel cache holds it.
#[derive(Clone, Debug)]
pub struct PixelCache {
    pub width: usize,
    pub height: usize,
    /// The cache's channels, in order.
    pub channels: Vec<PixelChannel>,
    /// Interleaved quanta, `channels.len()` per pixel.
    pub data: Vec<f32>,
    pub colorspace: CacheColorspace,
    /// `image->gamma`.
    pub gamma: f64,
    /// `image->type`.
    pub image_type: ImageType,
    /// A `PseudoClass` image's colormap (red, green, blue and alpha, as
    /// doubles); empty for `DirectClass`. Its pixels normally hold their
    /// entries' colours, but colorspace transforms rebuild them from it
    /// (`SyncImage`), so it matters where they don't.
    pub colormap: Vec<[f64; 4]>,
    /// The image properties a transform to sRGB reads (Log film settings,
    /// Jzazbz white luminance).
    pub transform_settings: TransformSettings,
    /// `image->alpha_trait`: whether IM counts the alpha channel. A reader
    /// may clear it after the pixels are in, which leaves the channel in
    /// the cache until something resyncs it.
    pub alpha_trait: bool,
}

impl PixelCache {
    /// A `DirectClass` cache of undefined type.
    pub fn new(width: usize, height: usize, channels: Vec<PixelChannel>, data: Vec<f32>, colorspace: CacheColorspace, gamma: f64) -> Self {
        let alpha_trait = channels.contains(&PixelChannel::Alpha);
        Self { width, height, channels, data, colorspace, gamma, image_type: ImageType::Undefined, colormap: Vec::new(), transform_settings: TransformSettings::default(), alpha_trait }
    }

    pub fn has_channel(&self, channel: PixelChannel) -> bool {
        self.channels.contains(&channel)
    }

    pub fn is_pseudo_class(&self) -> bool {
        self.has_channel(PixelChannel::Index)
    }
}

/// `ScaleCharToQuantum`.
pub(crate) fn char_quantum(value: u8) -> f32 {
    257.0 * value as f32
}

/// Reads an image file the way Magick.NET does: the format picked as IM
/// picks it (`coders::formats`), then read as IM's coder for it reads it.
pub fn read_pixel_cache(path: &Path) -> Result<PixelCache> {
    let format = crate::coders::formats::select(path)?;
    match (format.module, format.name) {
        ("PNG", "JNG") => crate::coders::jng::read(path),
        ("PNG", "MNG") => crate::coders::mng::read(path),
        ("PNG", _) => read_png(path),
        ("JPEG", _) => crate::coders::jpeg::read(path),
        ("GIF", _) => crate::coders::gif::read(path),
        ("BMP", _) => crate::coders::bmp::read(path),
        ("TIFF", "GROUP4") => bail!("GROUP4 images are not supported"),
        ("TIFF", _) => crate::coders::tiff::read(path),
        ("EXR", _) => crate::coders::exr::read(path),
        ("WEBP", _) => crate::coders::webp::read(path),
        ("JP2", _) => crate::coders::jp2::read(path),
        ("JXL", _) => crate::coders::jxl::read(path),
        ("TTF", _) => crate::coders::font::read(path),
        ("PES", _) => crate::coders::pes::read(path),
        ("DDS", _) => crate::coders::dds::read(path),
        ("MIFF", _) => crate::coders::miff::read(path),
        ("SVG", _) => crate::coders::svg::read(path),
        ("DNG", "DNG") => crate::coders::dng::read(path),
        ("DNG", _) => crate::coders::dng::read_camera_raw(path),
        ("PSD", _) => crate::coders::psd::read(path),
        ("EMF", _) => crate::coders::emf::read(path),
        ("SGI", _) => crate::coders::sgi::read(path),
        ("HDR", _) => crate::coders::hdr::read(path),
        ("PNM", _) => crate::coders::pnm::read(path),
        ("QOI", _) => crate::coders::qoi::read(path),
        ("FARBFELD", _) => crate::coders::farbfeld::read(path),
        ("WBMP", _) => crate::coders::wbmp::read(path),
        ("TGA", _) => crate::coders::tga::read(path),
        ("XBM", _) => crate::coders::xbm::read(path),
        ("SUN", _) => crate::coders::sun::read(path),
        ("PCX", name) => crate::coders::pcx::read(path, name == "DCX"),
        ("ICON", _) => crate::coders::icon::read(path),
        ("XPM", _) => crate::coders::xpm::read(path),
        ("VICAR", _) => crate::coders::vicar::read(path),
        ("FITS", _) => crate::coders::fits::read(path),
        ("MAT", _) => crate::coders::mat::read(path),
        ("FL32", _) => crate::coders::fl32::read(path),
        ("VIPS", _) => crate::coders::vips::read(path),
        ("DPX", _) => crate::coders::dpx::read(path),
        ("CIN", _) => crate::coders::cin::read(path),
        ("MTV", _) => crate::coders::mtv::read(path),
        ("AVS", _) => crate::coders::avs::read(path, false),
        ("AAI", _) => crate::coders::avs::read(path, true),
        ("HRZ", _) => crate::coders::hrz::read(path),
        ("OTB", _) => crate::coders::otb::read(path),
        ("RGF", _) => crate::coders::rgf::read(path),
        ("PGX", _) => crate::coders::pgx::read(path),
        ("IPL", _) => crate::coders::ipl::read(path),
        ("VIFF", _) => crate::coders::viff::read(path),
        ("DIB", "ICODIB") => bail!("ICODIB images are not supported"),
        ("DIB", _) => crate::coders::dib::read(path),
        ("PALM", _) => crate::coders::palm::read(path),
        ("TXT", "TXT") => crate::coders::txt::read(path),
        ("SIXEL", _) => crate::coders::sixel::read(path),
        ("PDB", _) => crate::coders::pdb::read(path),
        ("CALS", _) => crate::coders::cals::read(path),
        ("SCR", _) => crate::coders::scr::read(path),
        ("ART", _) => crate::coders::art::read(path),
        ("MAC", _) => crate::coders::mac::read(path),
        ("TIM", _) => crate::coders::tim::read(path),
        ("PIX", _) => crate::coders::pix::read(path),
        ("RLA", _) => crate::coders::rla::read(path),
        ("SCT", _) => crate::coders::sct::read(path),
        ("RLE", _) => crate::coders::rle::read(path),
        ("SF3", _) => crate::coders::sf3::read(path),
        ("TIM2", _) => crate::coders::tim2::read(path),
        ("CUT", _) => crate::coders::cut::read(path),
        ("JNX", _) => crate::coders::jnx::read(path),
        ("FAX", "G4") => bail!("G4 images need a size"),
        ("FAX", _) => crate::coders::fax::read(path),
        ("ORA", _) => crate::coders::ora::read(path),
        ("XCF", _) => crate::coders::xcf::read(path),
        ("PICT", _) => crate::coders::pict::read(path),
        ("META", name) => crate::coders::meta::read(path, name),
        ("MPC", "MPC") => crate::coders::mpc::read(path),
        ("PCD", name) => crate::coders::pcd::read(path, name),
        ("WPG", _) => crate::coders::wpg::read(path),
        ("DCM", _) => crate::coders::dcm::read(path),
        ("HEIC", _) => crate::coders::heif::read(path),
        // IM 7.1.2's SFW reader checks the file's size against itself plus
        // 4096, so it (and PWP, made of SFW images) never reads anything.
        ("SFW" | "PWP", _) => bail!("MemoryAllocationFailed"),
        (_, name) => bail!("{name} images are not supported"),
    }
}

/// The samples of one PNG row, unpacked from their bit depth.
fn unpack_row(row: &[u8], bit_depth: u8, count: usize) -> Vec<u32> {
    match bit_depth {
        16 => {
            let (pairs, _) = row.as_chunks::<2>();
            pairs.iter().take(count).map(|pair| u16::from_be_bytes(*pair) as u32).collect()
        }
        8 => row.iter().take(count).map(|&byte| byte as u32).collect(),
        depth => {
            let per_byte = 8 / depth as usize;
            let mask = (1u32 << depth) - 1;
            let mut samples = Vec::with_capacity(count);
            for index in 0..count {
                let byte = row[index / per_byte] as u32;
                let shift = 8 - depth as usize * (index % per_byte + 1);
                samples.push((byte >> shift) & mask);
            }
            samples
        }
    }
}

/// IM's PNG reader (`coders/png.c`): palette images and grey below 16
/// bits are `PseudoClass` (their index kept as a channel); grey types are
/// GRAY or, with a gamma above 0.75, LinearGRAY; colour types sRGB or RGB.
/// A tRNS chunk adds alpha.
fn read_png(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    read_png_bytes(&bytes)
}

/// The chunks of a PNG without its tRNS chunks, and the first tRNS's
/// data when it comes before the image data (libpng ignores later and
/// repeated ones), with whether a palette came before it.
fn split_transparency(bytes: &[u8]) -> (Vec<u8>, Option<(Vec<u8>, bool)>) {
    let mut kept = bytes[..bytes.len().min(8)].to_vec();
    let mut transparency = None;
    let mut seen_palette = false;
    let mut seen_data = false;
    let mut position = 8;
    while position + 12 <= bytes.len() {
        let length = u32::from_be_bytes([bytes[position], bytes[position + 1], bytes[position + 2], bytes[position + 3]]) as usize;
        let kind = &bytes[position + 4..position + 8];
        let end = position.saturating_add(12).saturating_add(length);
        if end > bytes.len() {
            break;
        }
        match kind {
            b"PLTE" => seen_palette = true,
            b"IDAT" => seen_data = true,
            _ => {}
        }
        if kind == b"tRNS" {
            if transparency.is_none() && !seen_data {
                let data = bytes[position + 8..position + 8 + length].to_vec();
                transparency = Some((data, seen_palette));
            }
        } else {
            kept.extend_from_slice(&bytes[position..end]);
        }
        position = end;
    }
    kept.extend_from_slice(&bytes[position.min(bytes.len())..]);
    (kept, transparency)
}

/// A PNG held in memory (one embedded in another file).
pub(crate) fn read_png_bytes(bytes: &[u8]) -> Result<PixelCache> {
    read_png_bytes_with(bytes, false)
}

/// `read_png_bytes`, optionally undoing MNG's intrapixel differencing
/// (filter method 64: red and blue stored less green), which libpng does
/// to each row before anything else.
pub(crate) fn read_png_bytes_with(bytes: &[u8], intrapixel: bool) -> Result<PixelCache> {
    let (stripped, transparency_chunk) = split_transparency(bytes);
    let mut decoder = png::Decoder::new(std::io::Cursor::new(stripped));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info()?;
    let info = reader.info().clone();
    let mut gamma = DEFAULT_GAMMA;
    if info.srgb.is_some() {
        gamma = 0.45455;
    }
    if let Some(file_gamma) = info.gamma() {
        gamma = file_gamma.into_value() as f64;
    }
    let palette: Vec<u8> = info.palette.as_ref().map(|palette| palette.to_vec()).unwrap_or_default();
    let width = info.width as usize;
    let height = info.height as usize;
    let bit_depth = info.bit_depth as u8;
    let color_type = info.color_type;
    let transparency = png_transparency(transparency_chunk, &info, palette.len() / 3);

    let mut buffer = vec![0u8; reader.output_buffer_size().context("PNG too large")?];
    let frame = reader.next_frame(&mut buffer)?;
    let line_size = frame.line_size;

    let gray = matches!(color_type, png::ColorType::Grayscale | png::ColorType::GrayscaleAlpha);
    let colorspace = if gray { gray_colorspace(gamma) } else { colour_colorspace(gamma) };
    let pseudo = color_type == png::ColorType::Indexed || (color_type == png::ColorType::Grayscale && bit_depth < 16);
    let file_alpha = matches!(color_type, png::ColorType::GrayscaleAlpha | png::ColorType::Rgba);
    let alpha = file_alpha || transparency != PngTransparency::None;
    let mut channels = if gray { vec![PixelChannel::Gray] } else { vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue] };
    if alpha {
        channels.push(PixelChannel::Alpha);
    }
    if pseudo {
        channels.push(PixelChannel::Index);
    }

    // PseudoClass images keep their colormap: the palette, or a ramp of
    // every grey the depth holds (`AcquireImageColormap`). A tRNS chunk
    // gives palette entries their alpha, or clears the grey that matches.
    let ramp_step = 65535.0 / ((1u32 << bit_depth.min(15)) - 1).max(1) as f64;
    let mut colormap: Vec<[f64; 4]> = Vec::new();
    if color_type == png::ColorType::Indexed {
        for entry in palette.as_chunks::<3>().0 {
            colormap.push([char_quantum(entry[0]) as f64, char_quantum(entry[1]) as f64, char_quantum(entry[2]) as f64, 65535.0]);
        }
        if let PngTransparency::Palette(alphas) = &transparency {
            for (entry, &value) in colormap.iter_mut().zip(alphas.iter()) {
                entry[3] = char_quantum(value) as f64;
            }
        }
    } else if pseudo {
        for index in 0..1usize << bit_depth {
            let level = index as f64 * ramp_step;
            colormap.push([level, level, level, 65535.0]);
        }
        if let PngTransparency::Colour([transparent_gray, _, _]) = transparency {
            for entry in colormap.iter_mut() {
                if scale_quantum_to_short(entry[0] as f32) == transparent_gray {
                    entry[3] = 0.0;
                }
            }
        }
    }

    let samples_per_pixel = color_type.samples();
    let mut data = Vec::with_capacity(width * height * channels.len());
    for y in 0..height {
        let row = &buffer[y * line_size..(y + 1) * line_size];
        let mut samples = unpack_row(row, bit_depth, width * samples_per_pixel);
        if intrapixel && matches!(color_type, png::ColorType::Rgb | png::ColorType::Rgba) {
            let mask = if bit_depth == 16 { 0xffff } else { 0xff };
            for pixel in samples.chunks_exact_mut(samples_per_pixel) {
                pixel[0] = (pixel[0] + pixel[1]) & mask;
                pixel[2] = (pixel[2] + pixel[1]) & mask;
            }
        }
        for pixel in samples.chunks_exact(samples_per_pixel) {
            if pseudo {
                // `ConstrainColormapIndex` refuses an index past the colormap.
                let index = pixel[0] as usize;
                let Some(entry) = colormap.get(index) else {
                    bail!("InvalidColormapIndex");
                };
                if gray {
                    data.push(entry[2] as f32);
                } else {
                    data.extend([entry[0] as f32, entry[1] as f32, entry[2] as f32]);
                }
                if alpha {
                    data.push(entry[3] as f32);
                }
                data.push(index as f32);
                continue;
            }
            let start = data.len();
            if bit_depth == 16 {
                data.extend(pixel.iter().map(|&value| value as f32));
            } else {
                data.extend(pixel.iter().map(|&value| char_quantum(value as u8)));
            }
            // A pixel of the tRNS colour is transparent, every other opaque.
            if let PngTransparency::Colour(colour) = transparency {
                let pixel_colour = if gray { [data[start]; 3] } else { [data[start], data[start + 1], data[start + 2]] };
                let matches = pixel_colour.iter().zip(colour.iter()).all(|(&value, &wanted)| scale_quantum_to_short(value) == wanted);
                data.push(if matches { 0.0 } else { 65535.0 });
            }
        }
    }

    // The type `ReadOnePNGImage` sets once the pixels are in.
    let image_type = match (color_type, alpha) {
        (png::ColorType::GrayscaleAlpha, _) => ImageType::GrayscaleAlpha,
        (png::ColorType::Indexed, true) => ImageType::PaletteAlpha,
        (png::ColorType::Indexed, false) => ImageType::Palette,
        (png::ColorType::Grayscale, false) => ImageType::Grayscale,
        (_, true) => ImageType::TrueColorAlpha,
        (_, false) => ImageType::TrueColor,
    };
    let mut cache = PixelCache::new(width, height, channels, data, colorspace, gamma);
    cache.image_type = image_type;
    cache.colormap = colormap;
    Ok(cache)
}

/// What a PNG's tRNS chunk gives, as IM takes it.
#[derive(Clone, Debug, PartialEq)]
enum PngTransparency {
    None,
    /// The first palette entries' alphas.
    Palette(Vec<u8>),
    /// The transparent grey or colour, scaled to 16 bits.
    Colour([u32; 3]),
}

/// The tRNS chunk as libpng accepts it (a grey's two bytes, a colour's
/// six, no more alphas than palette entries and after them, none with an
/// alpha channel) and IM keeps it: a grey or colour sample past the bit
/// depth's range drops it.
fn png_transparency(chunk: Option<(Vec<u8>, bool)>, info: &png::Info, palette_entries: usize) -> PngTransparency {
    let Some((bytes, after_palette)) = chunk else {
        return PngTransparency::None;
    };
    if info.color_type == png::ColorType::Indexed && !after_palette {
        return PngTransparency::None;
    }
    let depth = info.bit_depth as u32;
    let max_sample = (1u32 << depth) - 1;
    let scale_to_short = 65535 / max_sample;
    let sample = |offset: usize| u16::from_be_bytes([bytes[offset], bytes[offset + 1]]) as u32;
    match info.color_type {
        png::ColorType::Indexed if !bytes.is_empty() && bytes.len() <= palette_entries => PngTransparency::Palette(bytes.to_vec()),
        png::ColorType::Grayscale if bytes.len() == 2 => {
            let gray = sample(0);
            if gray > max_sample {
                return PngTransparency::None;
            }
            PngTransparency::Colour([gray * scale_to_short; 3])
        }
        png::ColorType::Rgb if bytes.len() == 6 => {
            let colour = [sample(0), sample(2), sample(4)];
            if colour.iter().any(|&value| value > max_sample) {
                return PngTransparency::None;
            }
            PngTransparency::Colour(colour.map(|value| value * scale_to_short))
        }
        _ => PngTransparency::None,
    }
}

/// `ScaleQuantumToShort` (Q16 HDRI).
fn scale_quantum_to_short(quantum: f32) -> u32 {
    if quantum <= 0.0 {
        return 0;
    }
    if quantum >= 65535.0 {
        return 65535;
    }
    (quantum + 0.5) as u32
}

fn gray_colorspace(gamma: f64) -> CacheColorspace {
    if gamma > 0.75 {
        CacheColorspace::LinearGray
    } else {
        CacheColorspace::Gray
    }
}

fn colour_colorspace(gamma: f64) -> CacheColorspace {
    if gamma > 0.75 {
        CacheColorspace::Rgb
    } else {
        CacheColorspace::SRgb
    }
}
