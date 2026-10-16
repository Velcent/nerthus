//! SVG (`coders/svg.c` through librsvg and cairo, as Magick.NET has it).
//! The File node sets its density only after reading. `ReadSVGImage`
//! supplies its own default of 96 dpi before opening the SVG. Each
//! pixel is composited over the white background (`CompositePixelOver`),
//! so the image is opaque; it keeps an alpha channel (`BlendPixelTrait`).
//! Paths use Cairo's fixed-point edge coverage in Rust. Resvg supplies
//! SVG parsing and the rendering of filters, patterns and embedded images.

use std::path::Path;

use anyhow::{bail, Context, Result};
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{Options, Tree};

use crate::pixel_cache::{CacheColorspace, DEFAULT_GAMMA, PixelCache, PixelChannel};

#[path = "svg-raster.rs"]
mod raster;
#[path = "svg-render.rs"]
mod render;

/// `SVGDensityGeometry`, used by `ReadSVGImage` for an unspecified density.
const SVG_DENSITY: f32 = 96.0;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    readBytes(&bytes, path)
}

/// Also used by the PES reader, which imports embroidery as an SVG drawing.
#[allow(non_snake_case)]
pub(crate) fn readBytes(bytes: &[u8], path: &Path) -> Result<PixelCache> {
    let options = Options { dpi: SVG_DENSITY, resources_dir: path.parent().map(Path::to_path_buf), ..Options::default() };
    let tree = Tree::from_data(bytes, &options).with_context(|| format!("cannot read {}", path.display()))?;
    // `rsvg_handle_get_dimensions` gives whole pixels.
    let size = tree.size();
    let (width, height) = (size.width().round() as usize, size.height().round() as usize);
    if width == 0 || height == 0 {
        bail!("the SVG has no size");
    }
    let mut pixmap = Pixmap::new(width as u32, height as u32).context("the SVG is too large")?;
    render::render(&tree, Transform::identity(), &mut pixmap);

    // Premultiplied samples over opaque white: `Sc·Sa + 65535·(1 − Sa)`
    // with `Sc` the demultiplied colour (IM divides the 8-bit premultiplied
    // value, scaled to a quantum, by the alpha).
    let mut data = Vec::with_capacity(width * height * 4);
    for pixel in pixmap.data().as_chunks::<4>().0 {
        let alpha = 257.0 * pixel[3] as f64 / 65535.0;
        let over = |premultiplied: u8| -> f32 {
            let source = 257.0 * premultiplied as f64;
            (source + 65535.0 * (1.0 - alpha)) as f32
        };
        data.extend([over(pixel[0]), over(pixel[1]), over(pixel[2]), 65535.0]);
    }
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Alpha];
    Ok(PixelCache::new(width, height, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA))
}
