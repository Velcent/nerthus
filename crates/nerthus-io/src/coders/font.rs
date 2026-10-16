//! TrueType and OpenType specimens, following ImageMagick's `coders/ttf.c`.
//! Skrifa supplies FreeType-compatible light hinting in Rust; Zeno turns
//! the hinted outlines into grayscale coverage masks.

#![allow(non_snake_case)]

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, Engine, HintingInstance, HintingOptions, OutlineGlyphFormat, OutlinePen, SmoothMode, Target};
use skrifa::{FontRef, MetadataProvider};
use ttf_parser::{Face, GlyphId};
use zeno::{Command, Mask, Placement};

use crate::pixel_cache::{CacheColorspace, DEFAULT_GAMMA, PixelCache, PixelChannel};

const WIDTH: usize = 800;
const HEIGHT: usize = 480;
const ALPHABETS: [&str; 3] = [
    "abcdefghijklmnopqrstuvwxyz",
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
    "0123456789.:,;(*!?}^)#${%^&-+@",
];
const SPECIMEN: &str = "That which does not destroy me, only makes me stronger.";
const POINT_SIZES: [u16; 7] = [12, 18, 24, 36, 48, 60, 72];

struct GlyphBitmap {
    coverage: Vec<u8>,
    placement: Placement,
    advance: f32,
}

#[derive(Default)]
struct GlyphOutline {
    commands: Vec<Command>,
}

impl OutlinePen for GlyphOutline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.commands.push(Command::MoveTo([x, -y].into()));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.commands.push(Command::LineTo([x, -y].into()));
    }

    fn quad_to(&mut self, control_x: f32, control_y: f32, x: f32, y: f32) {
        let control = [control_x, -control_y].into();
        let end = [x, -y].into();
        self.commands.push(Command::QuadTo(control, end));
    }

    fn curve_to(
        &mut self,
        first_x: f32,
        first_y: f32,
        second_x: f32,
        second_y: f32,
        x: f32,
        y: f32,
    ) {
        let first_control = [first_x, -first_y].into();
        let second_control = [second_x, -second_y].into();
        let end = [x, -y].into();
        self.commands
            .push(Command::CurveTo(first_control, second_control, end));
    }

    fn close(&mut self) {
        self.commands.push(Command::Close);
    }
}

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    readBytes(&bytes)
}

fn readBytes(bytes: &[u8]) -> Result<PixelCache> {
    let font = FontRef::from_index(bytes, 0).context("invalid TrueType or OpenType font")?;
    let face = Face::parse(bytes, 0).context("invalid TrueType or OpenType font")?;
    let mut data = vec![65535.0; WIDTH * HEIGHT * 4];
    let mut glyphs = HashMap::new();

    // AnnotateImage spaces multiline text using rounded ascender and
    // descender metrics, rather than the font's recommended line gap.
    let scale = 18.0 / f32::from(face.units_per_em());
    let ascent = (f32::from(face.ascender()) * scale).ceil();
    let descent = (f32::from(face.descender()) * scale).floor();
    let line_height = if ascent == descent {
        18.0
    } else {
        ascent - descent
    };
    for (line_index, text) in ALPHABETS.iter().enumerate() {
        let baseline = 20 + (line_index as f32 * line_height) as i32;
        drawText(
            &font,
            &face,
            &mut glyphs,
            &mut data,
            text,
            18,
            (10, baseline),
        )?;
    }

    // MultilineCensus includes the empty line after the final newline.
    let mut baseline = 120;
    for point_size in POINT_SIZES {
        baseline += i32::from(point_size) + 12;
        let label = point_size.to_string();
        drawText(
            &font,
            &face,
            &mut glyphs,
            &mut data,
            &label,
            18,
            (10, baseline),
        )?;
        drawText(
            &font,
            &face,
            &mut glyphs,
            &mut data,
            SPECIMEN,
            point_size,
            (50, baseline),
        )?;
    }

    let channels = vec![
        PixelChannel::Red,
        PixelChannel::Green,
        PixelChannel::Blue,
        PixelChannel::Alpha,
    ];
    Ok(PixelCache::new(
        WIDTH,
        HEIGHT,
        channels,
        data,
        CacheColorspace::SRgb,
        DEFAULT_GAMMA,
    ))
}

fn drawText(
    font: &FontRef<'_>,
    face: &Face<'_>,
    glyphs: &mut HashMap<(u16, u16), GlyphBitmap>,
    data: &mut [f32],
    text: &str,
    point_size: u16,
    origin: (i32, i32),
) -> Result<()> {
    let outlines = font.outline_glyphs();
    let size = Size::new(f32::from(point_size));
    let location = LocationRef::default();
    let target = Target::from(SmoothMode::Light);
    // FreeType's LIGHT load target selects the automatic hinter for
    // TrueType unless the face requires its embedded instructions.
    let engine = if outlines.format() == Some(OutlineGlyphFormat::Glyf) && !outlines.require_interpreter() {
        Engine::Auto(None)
    } else {
        Engine::Interpreter
    };
    let hinting_options = HintingOptions { engine, target };
    let hinter = HintingInstance::new(&outlines, size, location, hinting_options)
        .context("cannot initialize font hinting")?;
    let glyph_metrics = font.glyph_metrics(size, location);
    let fallback = face.glyph_index(' ').unwrap_or(GlyphId(0));
    let mut previous = GlyphId(0);
    let mut pen_x = origin.0;

    for character in text.chars() {
        let mapped = face.glyph_index(character).unwrap_or(GlyphId(0));
        if previous.0 != 0 && mapped.0 != 0 {
            pen_x += kerning(face, previous, mapped, point_size);
        }
        previous = mapped;
        let glyph_id = if mapped.0 == 0 { fallback } else { mapped };
        let key = (point_size, glyph_id.0);
        if !glyphs.contains_key(&key) {
            let skrifa_id = skrifa::GlyphId::new(u32::from(glyph_id.0));
            let mut outline = GlyphOutline::default();
            let mut advance = glyph_metrics
                .advance_width(skrifa_id)
                .unwrap_or(0.0)
                .round();
            if let Some(glyph) = outlines.get(skrifa_id) {
                let settings = DrawSettings::hinted(&hinter, false);
                let adjusted = glyph
                    .draw(settings, &mut outline)
                    .context("cannot read font glyph")?;
                advance = adjusted.advance_width.unwrap_or(advance);
                if point_size == 18 && character == 'b' {
                    eprintln!("{character} {} advance={advance} {:?}", glyph_id.0, outline.commands);
                }
            }
            let mask = Mask::new(outline.commands.as_slice());
            let (coverage, placement) = mask.render();
            let bitmap = GlyphBitmap {
                coverage,
                placement,
                advance,
            };
            glyphs.insert(key, bitmap);
        }
        let bitmap = &glyphs[&key];
        compositeGlyph(data, bitmap, (pen_x, origin.1));
        pen_x += bitmap.advance.round() as i32;
    }
    Ok(())
}

fn kerning(face: &Face<'_>, previous: GlyphId, current: GlyphId, point_size: u16) -> i32 {
    let Some(table) = face.tables().kern else {
        return 0;
    };
    let mut units = 0_i32;
    for subtable in table.subtables {
        if subtable.horizontal && !subtable.variable && !subtable.has_cross_stream {
            units += i32::from(subtable.glyphs_kerning(previous, current).unwrap_or(0));
        }
    }
    let scale = f32::from(point_size) / f32::from(face.units_per_em());
    let pixels = units as f32 * scale;
    // FT_KERNING_DEFAULT reduces kerning below 25 ppem and grid-fits it.
    let damping = (f32::from(point_size) / 25.0).min(1.0);
    (pixels * damping).round() as i32
}

fn compositeGlyph(data: &mut [f32], bitmap: &GlyphBitmap, origin: (i32, i32)) {
    let placement = &bitmap.placement;
    for row in 0..placement.height as i32 {
        let y = origin.1 + placement.top + row;
        if !(0..HEIGHT as i32).contains(&y) {
            continue;
        }
        for column in 0..placement.width as i32 {
            let x = origin.0 + placement.left + column;
            if !(0..WIDTH as i32).contains(&x) {
                continue;
            }
            let mask_index = row as usize * placement.width as usize + column as usize;
            let opacity = f64::from(bitmap.coverage[mask_index]) / 255.0;
            let pixel_index = (y as usize * WIDTH + x as usize) * 4;
            for sample in &mut data[pixel_index..pixel_index + 3] {
                *sample = (f64::from(*sample) * (1.0 - opacity)) as f32;
            }
        }
    }
}
