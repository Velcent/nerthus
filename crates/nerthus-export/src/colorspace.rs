//! ImageMagick 7.1.2's colourspace conversions on whole images, as
//! Magick.NET's `MagickImage.ColorSpace` setter runs them
//! (`TransformImageColorspace`), built on the pixel conversions of
//! `nerthus_image::colorspace_transform`.

pub use nerthus_image::colorspace_transform::*;

use crate::magick::{quantum_to_map, MagickImage};

impl MagickImage {
    /// `TransformImageColorspace` (Magick.NET's `ColorSpace` setter).
    pub fn transform_colorspace(&mut self, target: Colorspace) {
        if self.colorspace == target {
            return;
        }
        // Undefined only relabels the image.
        if target == Colorspace::Undefined {
            self.colorspace = Colorspace::Undefined;
            return;
        }
        if target.is_srgb() {
            self.transform_to_srgb();
            return;
        }
        if !self.colorspace.is_srgb() {
            self.transform_to_srgb();
        }
        self.transform_from_srgb(target);
    }

    /// The image's quanta with its colour channels replaced by `colors`
    /// per pixel, alpha kept.
    fn rebuild(&mut self, colors: usize, colorspace: Colorspace, convert: impl Fn(&[f32]) -> [f32; 4]) {
        let alpha = self.has_alpha();
        let channels = colors + usize::from(alpha);
        let mut quanta = Vec::with_capacity(self.pixel_count() * channels);
        for pixel in self.quanta.chunks(self.channels) {
            let converted = convert(pixel);
            quanta.extend_from_slice(&converted[..colors]);
            if alpha {
                quanta.push(pixel[pixel.len() - 1]);
            }
        }
        self.quanta = quanta;
        self.channels = channels;
        self.colorspace = colorspace;
    }

    /// `sRGBTransformImage`: the sRGB image to `target`.
    fn transform_from_srgb(&mut self, target: Colorspace) {
        match target {
            Colorspace::Cmyk => self.rebuild(4, target, |pixel| {
                let [cyan, magenta, yellow, black] = rgb_to_cmyk(pixel[0] as f64, pixel[1] as f64, pixel[2] as f64);
                [cyan as f32, magenta as f32, yellow as f32, black as f32]
            }),
            Colorspace::LinearGray => self.rebuild(1, target, |pixel| {
                let gray = 0.212656 * decode_pixel_gamma(pixel[0] as f64) + 0.715158 * decode_pixel_gamma(pixel[1] as f64) + 0.072186 * decode_pixel_gamma(pixel[2] as f64);
                [gray as f32, 0.0, 0.0, 0.0]
            }),
            Colorspace::Gray => self.rebuild(1, target, |pixel| {
                let gray = 0.212656 * pixel[0] as f64 + 0.715158 * pixel[1] as f64 + 0.072186 * pixel[2] as f64;
                [gray as f32, 0.0, 0.0, 0.0]
            }),
            Colorspace::Log => {
                let log_map = log_map_from_srgb();
                self.rebuild(3, target, |pixel| {
                    let mut converted = [0.0f32; 4];
                    for (channel, value) in converted.iter_mut().zip(pixel.iter()).take(3) {
                        let linear = decode_pixel_gamma(*value as f64);
                        *channel = log_map[quantum_to_map(linear as f32)];
                    }
                    converted
                });
            }
            Colorspace::Rgb | Colorspace::ScRgb => self.rebuild(3, target, |pixel| {
                let red = decode_pixel_gamma(pixel[0] as f64);
                let green = decode_pixel_gamma(pixel[1] as f64);
                let blue = decode_pixel_gamma(pixel[2] as f64);
                [red as f32, green as f32, blue as f32, 0.0]
            }),
            _ if is_generic(target) => self.rebuild(3, target, |pixel| {
                let [x, y, z] = rgb_to_generic(target, pixel[0] as f64, pixel[1] as f64, pixel[2] as f64);
                [(QUANTUM_RANGE * x) as f32, (QUANTUM_RANGE * y) as f32, (QUANTUM_RANGE * z) as f32, 0.0]
            }),
            _ => {
                let tables = TransformTables::from_srgb(target);
                self.rebuild(3, target, |pixel| {
                    let sums = tables.sums(quantum_to_map(pixel[0]), quantum_to_map(pixel[1]), quantum_to_map(pixel[2]));
                    [map_to_quantum(sums[0]), map_to_quantum(sums[1]), map_to_quantum(sums[2]), 0.0]
                });
            }
        }
    }

    /// `TransformsRGBImage`: the image back to sRGB. Writers never take
    /// grey back (it is sRGB-compatible).
    pub(crate) fn transform_to_srgb(&mut self) {
        let conversion = ToSrgb::new(self.colorspace, &TransformSettings::default());
        self.rebuild(3, Colorspace::SRgb, |pixel| {
            let [red, green, blue] = conversion.convert(pixel);
            [red, green, blue, 0.0]
        });
    }
}
