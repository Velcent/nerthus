//! libheif's pixel image (`HeifPixelImage` in `pixelimage.cc`): planes of
//! samples with their bit depths, the nclx colour profile, and the
//! rotate, mirror, crop and scaling operations items apply.

use std::collections::BTreeMap;

use anyhow::{bail, Result};

use super::color::{convert_colorspace, ConversionOptions};
use super::nclx::Nclx;

/// `heif_colorspace`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colorspace {
    YCbCr,
    Rgb,
    Monochrome,
}

/// `heif_chroma` (the interleaved formats keep their byte order, which
/// only the conversion graph looks at; samples are stored as values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chroma {
    Undefined,
    Monochrome,
    C420,
    C422,
    C444,
    InterleavedRgb,
    InterleavedRgba,
    InterleavedRrggbbBe,
    InterleavedRrggbbaaBe,
    InterleavedRrggbbLe,
    InterleavedRrggbbaaLe,
}

impl Chroma {
    /// `chroma_h_subsampling`.
    pub fn h_subsampling(self) -> u32 {
        match self {
            Chroma::C420 | Chroma::C422 => 2,
            _ => 1,
        }
    }

    /// `chroma_v_subsampling`.
    pub fn v_subsampling(self) -> u32 {
        match self {
            Chroma::C420 => 2,
            _ => 1,
        }
    }

    /// `num_interleaved_pixels_per_plane`.
    pub fn interleaved_components(self) -> usize {
        match self {
            Chroma::InterleavedRgb | Chroma::InterleavedRrggbbBe | Chroma::InterleavedRrggbbLe => 3,
            Chroma::InterleavedRgba | Chroma::InterleavedRrggbbaaBe | Chroma::InterleavedRrggbbaaLe => 4,
            _ => 1,
        }
    }

    /// `is_interleaved_with_alpha`.
    pub fn interleaved_with_alpha(self) -> bool {
        matches!(self, Chroma::InterleavedRgba | Chroma::InterleavedRrggbbaaBe | Chroma::InterleavedRrggbbaaLe)
    }

    pub fn is_planar(self) -> bool {
        matches!(self, Chroma::Monochrome | Chroma::C420 | Chroma::C422 | Chroma::C444)
    }

    /// The HEVC/AV1 chroma format number (0 mono … 3 4:4:4).
    pub fn from_format(format: i32) -> Chroma {
        match format {
            0 => Chroma::Monochrome,
            1 => Chroma::C420,
            2 => Chroma::C422,
            3 => Chroma::C444,
            _ => Chroma::Undefined,
        }
    }
}

/// `heif_channel`, ordered as libheif's enum (`std::set` iterates in this
/// order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Channel {
    Y = 0,
    Cb = 1,
    Cr = 2,
    R = 3,
    G = 4,
    B = 5,
    Alpha = 6,
    Interleaved = 10,
}

/// One plane: samples row by row (`components` of them a pixel for an
/// interleaved plane).
#[derive(Clone, Debug)]
pub struct Plane {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub components: usize,
    pub samples: Vec<u16>,
}

impl Plane {
    pub fn new(width: u32, height: u32, bit_depth: u8, components: usize) -> Plane {
        Plane { width, height, bit_depth, components, samples: vec![0; width as usize * height as usize * components] }
    }

    pub fn stride(&self) -> usize {
        self.width as usize * self.components
    }

    pub fn get(&self, x: usize, y: usize) -> u16 {
        self.samples[y * self.stride() + x]
    }

    pub fn set(&mut self, x: usize, y: usize, value: u16) {
        let stride = self.stride();
        self.samples[y * stride + x] = value;
    }
}

#[derive(Clone, Debug)]
pub struct PixelImage {
    pub width: u32,
    pub height: u32,
    pub colorspace: Colorspace,
    pub chroma: Chroma,
    pub planes: BTreeMap<Channel, Plane>,
    pub nclx: Nclx,
    pub premultiplied_alpha: bool,
}

/// `chroma_width`.
pub fn chroma_width(width: u32, chroma: Chroma) -> u32 {
    match chroma {
        Chroma::C420 | Chroma::C422 => width.div_ceil(2),
        _ => width,
    }
}

/// `chroma_height`.
pub fn chroma_height(height: u32, chroma: Chroma) -> u32 {
    match chroma {
        Chroma::C420 => height.div_ceil(2),
        _ => height,
    }
}

/// `channel_width`.
pub fn channel_width(width: u32, chroma: Chroma, channel: Channel) -> u32 {
    if channel == Channel::Cb || channel == Channel::Cr { chroma_width(width, chroma) } else { width }
}

/// `channel_height`.
pub fn channel_height(height: u32, chroma: Chroma, channel: Channel) -> u32 {
    if channel == Channel::Cb || channel == Channel::Cr { chroma_height(height, chroma) } else { height }
}

impl PixelImage {
    /// `create`: no planes yet.
    pub fn new(width: u32, height: u32, colorspace: Colorspace, chroma: Chroma) -> PixelImage {
        PixelImage { width, height, colorspace, chroma, planes: BTreeMap::new(), nclx: Nclx::undefined(), premultiplied_alpha: false }
    }

    /// `add_plane` (24/32-bit interleaved RGB depths mean 8 bits).
    pub fn add_plane(&mut self, channel: Channel, width: u32, height: u32, bit_depth: u8) {
        let mut depth = bit_depth;
        if (self.chroma == Chroma::InterleavedRgb && depth == 24) || (self.chroma == Chroma::InterleavedRgba && depth == 32) {
            depth = 8;
        }
        let components = self.chroma.interleaved_components();
        self.planes.insert(channel, Plane::new(width, height, depth, components));
    }

    pub fn has_channel(&self, channel: Channel) -> bool {
        self.planes.contains_key(&channel)
    }

    pub fn plane(&self, channel: Channel) -> &Plane {
        &self.planes[&channel]
    }

    pub fn plane_mut(&mut self, channel: Channel) -> &mut Plane {
        self.planes.get_mut(&channel).expect("plane")
    }

    /// `get_bits_per_pixel` (255 for a missing plane, the C `uint8_t(-1)`).
    pub fn bits_per_pixel(&self, channel: Channel) -> u8 {
        self.planes.get(&channel).map_or(255, |plane| plane.bit_depth)
    }

    pub fn channel_width(&self, channel: Channel) -> u32 {
        self.planes.get(&channel).map_or(0, |plane| plane.width)
    }

    pub fn channel_height(&self, channel: Channel) -> u32 {
        self.planes.get(&channel).map_or(0, |plane| plane.height)
    }

    /// `has_nclx_color_profile`.
    pub fn has_nclx(&self) -> bool {
        self.nclx != Nclx::undefined()
    }

    fn has_odd_width(&self) -> bool {
        self.width & 1 == 1
    }

    fn has_odd_height(&self) -> bool {
        self.height & 1 == 1
    }

    /// A copy of the image converted to YCbCr 4:4:4, as rotation, mirroring
    /// and cropping of odd subsampled sizes need.
    fn to_ycbcr444(&self) -> Result<PixelImage> {
        let options = ConversionOptions::defaults_for_transforms();
        let depth = self.bits_per_pixel(Channel::Y) as i32;
        convert_colorspace(self, Colorspace::YCbCr, Chroma::C444, Nclx::undefined(), depth, &options)
    }

    /// `rotate_ccw`.
    pub fn rotate_ccw(&self, angle: i32) -> Result<PixelImage> {
        let need_conversion = match self.chroma {
            Chroma::C422 => angle == 90 || angle == 270 || (angle == 180 && self.has_odd_height()),
            Chroma::C420 => (angle == 90 && self.has_odd_width()) || (angle == 180 && (self.has_odd_width() || self.has_odd_height())) || (angle == 270 && self.has_odd_height()),
            _ => false,
        };
        if need_conversion {
            let converted = self.to_ycbcr444()?;
            return converted.rotate_ccw(angle);
        }
        if angle == 0 {
            return Ok(self.clone());
        }
        let (out_width, out_height) = if angle == 90 || angle == 270 { (self.height, self.width) } else { (self.width, self.height) };
        let mut out = PixelImage::new(out_width, out_height, self.colorspace, self.chroma);
        for (&channel, plane) in &self.planes {
            let (plane_width, plane_height) = if angle == 90 || angle == 270 { (plane.height, plane.width) } else { (plane.width, plane.height) };
            let mut rotated = Plane::new(plane_width, plane_height, plane.bit_depth, plane.components);
            let w = plane.width as usize;
            let h = plane.height as usize;
            let out_stride = rotated.stride();
            // Interleaved planes rotate as whole sample units in C (the
            // element type is the sample), so the components move apart;
            // libheif only rotates planar images here.
            match angle {
                270 => {
                    for x in 0..h {
                        for y in 0..w {
                            rotated.samples[y * out_stride + x] = plane.samples[(h - 1 - x) * plane.stride() + y];
                        }
                    }
                }
                180 => {
                    for y in 0..h {
                        for x in 0..w {
                            rotated.samples[y * out_stride + x] = plane.samples[(h - 1 - y) * plane.stride() + (w - 1 - x)];
                        }
                    }
                }
                90 => {
                    for x in 0..h {
                        for y in 0..w {
                            rotated.samples[y * out_stride + x] = plane.samples[x * plane.stride() + (w - 1 - y)];
                        }
                    }
                }
                _ => {}
            }
            out.planes.insert(channel, rotated);
        }
        out.nclx = self.nclx;
        Ok(out)
    }

    /// `mirror_inplace`; `horizontal` flips left to right.
    pub fn mirror(&self, horizontal: bool) -> Result<PixelImage> {
        let need_conversion = match self.chroma {
            Chroma::C422 => horizontal && self.has_odd_width(),
            Chroma::C420 => self.has_odd_width() || self.has_odd_height(),
            _ => false,
        };
        if need_conversion {
            let converted = self.to_ycbcr444()?;
            return converted.mirror(horizontal);
        }
        let mut out = self.clone();
        for plane in out.planes.values_mut() {
            let w = plane.width as usize;
            let h = plane.height as usize;
            let stride = plane.stride();
            if horizontal {
                for y in 0..h {
                    for x in 0..w / 2 {
                        plane.samples.swap(y * stride + x, y * stride + w - 1 - x);
                    }
                }
            } else {
                for y in 0..h / 2 {
                    for x in 0..w {
                        plane.samples.swap(y * stride + x, (h - 1 - y) * stride + x);
                    }
                }
            }
        }
        Ok(out)
    }

    /// `crop` to the inclusive rectangle.
    pub fn crop(&self, left: u32, right: u32, top: u32, bottom: u32) -> Result<PixelImage> {
        let need_conversion = match self.chroma {
            Chroma::C422 => left & 1 == 1,
            Chroma::C420 => left & 1 == 1 || top & 1 == 1,
            _ => false,
        };
        if need_conversion {
            let converted = self.to_ycbcr444()?;
            return converted.crop(left, right, top, bottom);
        }
        let mut out = PixelImage::new(right - left + 1, bottom - top + 1, self.colorspace, self.chroma);
        for (&channel, plane) in &self.planes {
            let subsampled = channel == Channel::Cb || channel == Channel::Cr;
            let (h_sub, v_sub) = if subsampled { (self.chroma.h_subsampling(), self.chroma.v_subsampling()) } else { (1, 1) };
            let plane_left = left / h_sub;
            let plane_right = right / h_sub;
            let plane_top = top / v_sub;
            let plane_bottom = bottom / v_sub;
            let mut cropped = Plane::new(plane_right - plane_left + 1, plane_bottom - plane_top + 1, plane.bit_depth, plane.components);
            let row_samples = cropped.stride();
            for y in plane_top..=plane_bottom {
                let source = y as usize * plane.stride() + plane_left as usize * plane.components;
                let target = (y - plane_top) as usize * row_samples;
                cropped.samples[target..target + row_samples].copy_from_slice(&plane.samples[source..source + row_samples]);
            }
            out.planes.insert(channel, cropped);
        }
        out.nclx = self.nclx;
        Ok(out)
    }

    /// `scale_nearest_neighbor` (every plane indexed with the image's own
    /// size, chroma planes included, as libheif does).
    pub fn scale_nearest_neighbor(&self, width: u32, height: u32) -> Result<PixelImage> {
        let mut out = PixelImage::new(width, height, self.colorspace, self.chroma);
        if self.has_channel(Channel::Interleaved) {
            out.add_plane(Channel::Interleaved, width, height, self.bits_per_pixel(Channel::Interleaved));
        } else {
            match self.colorspace {
                Colorspace::Rgb => {
                    if !self.has_channel(Channel::R) || !self.has_channel(Channel::G) || !self.has_channel(Channel::B) {
                        bail!("RGB input without R,G,B, planes");
                    }
                    for channel in [Channel::R, Channel::G, Channel::B] {
                        out.add_plane(channel, width, height, self.bits_per_pixel(channel));
                    }
                }
                Colorspace::Monochrome => {
                    if !self.has_channel(Channel::Y) {
                        bail!("monochrome input with no Y plane");
                    }
                    out.add_plane(Channel::Y, width, height, self.bits_per_pixel(Channel::Y));
                }
                Colorspace::YCbCr => {
                    if !self.has_channel(Channel::Y) || !self.has_channel(Channel::Cb) || !self.has_channel(Channel::Cr) {
                        bail!("YCbCr image without Y,Cb,Cr planes");
                    }
                    let chroma_w = width.div_ceil(self.chroma.h_subsampling());
                    let chroma_h = height.div_ceil(self.chroma.v_subsampling());
                    out.add_plane(Channel::Y, width, height, self.bits_per_pixel(Channel::Y));
                    out.add_plane(Channel::Cb, chroma_w, chroma_h, self.bits_per_pixel(Channel::Cb));
                    out.add_plane(Channel::Cr, chroma_w, chroma_h, self.bits_per_pixel(Channel::Cr));
                }
            }
            if self.has_channel(Channel::Alpha) {
                out.add_plane(Channel::Alpha, width, height, self.bits_per_pixel(Channel::Alpha));
            }
        }
        let components = self.chroma.interleaved_components();
        for (&channel, plane) in &self.planes {
            let Some(target) = out.planes.get_mut(&channel) else {
                bail!("scaling input has extra color plane");
            };
            let out_w = target.width;
            let out_h = target.height;
            let target_stride = target.stride();
            for y in 0..out_h {
                let iy = (y.wrapping_mul(self.height) / height) as usize;
                for x in 0..out_w {
                    let ix = (x.wrapping_mul(self.width) / width) as usize;
                    for c in 0..components {
                        let value = plane.samples[iy * plane.stride() + ix * components + c];
                        target.samples[y as usize * target_stride + x as usize * components + c] = value;
                    }
                }
            }
        }
        Ok(out)
    }

    /// `create_clone_image_at_new_size`.
    pub fn clone_at_size(source: &PixelImage, width: u32, height: u32) -> PixelImage {
        let chroma = source.chroma;
        let mut image = PixelImage::new(width, height, source.colorspace, chroma);
        match source.colorspace {
            Colorspace::Monochrome => image.add_plane(Channel::Y, width, height, source.bits_per_pixel(Channel::Y)),
            Colorspace::YCbCr => {
                image.add_plane(Channel::Y, width, height, source.bits_per_pixel(Channel::Y));
                image.add_plane(Channel::Cb, chroma_width(width, chroma), chroma_height(height, chroma), source.bits_per_pixel(Channel::Cb));
                image.add_plane(Channel::Cr, chroma_width(width, chroma), chroma_height(height, chroma), source.bits_per_pixel(Channel::Cr));
            }
            Colorspace::Rgb => {
                if chroma == Chroma::C444 {
                    for channel in [Channel::R, Channel::G, Channel::B] {
                        image.add_plane(channel, width, height, source.bits_per_pixel(channel));
                    }
                } else {
                    image.add_plane(Channel::Interleaved, width, height, source.bits_per_pixel(Channel::Interleaved));
                }
            }
        }
        if source.has_channel(Channel::Alpha) {
            image.add_plane(Channel::Alpha, width, height, source.bits_per_pixel(Channel::Alpha));
        }
        image
    }

    /// `fill_plane`.
    pub fn fill_plane(&mut self, channel: Channel, value: u16) {
        let plane = self.plane_mut(channel);
        let stored = if plane.bit_depth <= 8 { value & 0xff } else { value };
        plane.samples.fill(stored);
    }

    /// `copy_image_to`: pastes a tile at (`x0`, `y0`), clipped to the image.
    pub fn copy_image_to(&mut self, source: &PixelImage, x0: u32, y0: u32) -> Result<()> {
        let w = self.width;
        let h = self.height;
        let chroma = self.chroma;
        for (&channel, tile) in &source.planes {
            if w <= x0 || h <= y0 {
                bail!("Invalid grid data");
            }
            if !self.has_channel(channel) {
                bail!("grid image has no {channel:?} plane");
            }
            if tile.bit_depth != self.bits_per_pixel(channel) {
                bail!("Wrong tile image pixel depth");
            }
            let copy_width = tile.width.min(channel_width(w - x0, chroma, channel)) as usize * tile.components;
            let copy_height = tile.height.min(channel_height(h - y0, chroma, channel)) as usize;
            let xs = channel_width(x0, chroma, channel) as usize * tile.components;
            let ys = channel_height(y0, chroma, channel) as usize;
            let target = self.plane_mut(channel);
            let target_stride = target.stride();
            for py in 0..copy_height {
                let from = py * tile.stride();
                let to = xs + (ys + py) * target_stride;
                target.samples[to..to + copy_width].copy_from_slice(&tile.samples[from..from + copy_width]);
            }
        }
        Ok(())
    }
}
