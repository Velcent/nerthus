//! Brother PES embroidery, following ImageMagick's `coders/pes.c`.
//! The embedded PEC stitch stream becomes coloured SVG paths, including
//! jump stitches, before passing through the same SVG renderer as File.

#![allow(non_snake_case)]

use std::fmt::Write;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};

use crate::pixel_cache::PixelCache;

const THREAD_COLORS: [[u8; 3]; 65] = [
    [0, 0, 0],
    [14, 31, 124],
    [10, 85, 163],
    [48, 135, 119],
    [75, 107, 175],
    [237, 23, 31],
    [209, 92, 0],
    [145, 54, 151],
    [228, 154, 203],
    [145, 95, 172],
    [157, 214, 125],
    [232, 169, 0],
    [254, 186, 53],
    [255, 255, 0],
    [112, 188, 31],
    [192, 148, 0],
    [168, 168, 168],
    [123, 111, 0],
    [255, 255, 179],
    [79, 85, 86],
    [0, 0, 0],
    [11, 61, 145],
    [119, 1, 118],
    [41, 49, 51],
    [42, 19, 1],
    [246, 74, 138],
    [178, 118, 36],
    [252, 187, 196],
    [254, 55, 15],
    [240, 240, 240],
    [106, 28, 138],
    [168, 221, 196],
    [37, 132, 187],
    [254, 179, 67],
    [255, 240, 141],
    [208, 166, 96],
    [209, 84, 0],
    [102, 186, 73],
    [19, 74, 70],
    [135, 135, 135],
    [216, 202, 198],
    [67, 86, 7],
    [254, 227, 197],
    [249, 147, 188],
    [0, 56, 34],
    [178, 175, 212],
    [104, 106, 176],
    [239, 227, 185],
    [247, 56, 102],
    [181, 76, 100],
    [19, 43, 26],
    [199, 1, 85],
    [254, 158, 50],
    [168, 222, 235],
    [0, 103, 26],
    [78, 41, 144],
    [47, 126, 32],
    [253, 217, 222],
    [255, 217, 17],
    [9, 91, 166],
    [240, 249, 112],
    [227, 243, 91],
    [255, 200, 100],
    [255, 200, 150],
    [255, 200, 200],
];

#[derive(Default)]
struct StitchBlock {
    color: [u8; 3],
    points: Vec<(i64, i64)>,
}

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    let drawing = stitchDrawing(&bytes)?;
    super::svg::readBytes(drawing.as_bytes(), path)
}

fn stitchDrawing(bytes: &[u8]) -> Result<String> {
    ensure!(
        bytes.len() >= 12 && bytes[..4].eq_ignore_ascii_case(b"#PES"),
        "invalid PES header"
    );
    let pec_offset = i32::from_le_bytes(bytes[8..12].try_into().unwrap());
    ensure!(pec_offset >= 0, "invalid PES stitch offset");
    let color_offset = pec_offset as usize + 48;
    let color_count = usize::from(
        *bytes
            .get(color_offset)
            .context("truncated PES color table")?,
    ) + 1;
    let color_end = color_offset + 1 + color_count;
    let colors = bytes
        .get(color_offset + 1..color_end)
        .context("truncated PES color table")?;
    let stitch_offset = pec_offset as usize + 560;
    let stream = bytes
        .get(stitch_offset..)
        .context("truncated PES stitch header")?;
    let mut stitch_bytes = stream.iter().copied();
    let mut blocks = vec![StitchBlock::default()];
    let mut block_index = 0;
    let mut position = (0_i64, 0_i64);
    let mut minimum = (i64::MAX, i64::MAX);
    let mut maximum = (i64::MIN, i64::MIN);

    loop {
        let first = stitch_bytes.next().context("truncated PES stitch stream")?;
        let second = stitch_bytes.next().context("truncated PES stitch stream")?;
        if first == 0xff && second == 0 {
            break;
        }
        if first == 0xfe && second == 0xb0 {
            stitch_bytes.next().context("truncated PES color change")?;
            block_index += 1;
            ensure!(block_index < 255, "too many PES color changes");
            blocks.push(StitchBlock::default());
            continue;
        }

        let (delta_x, first_y) = if first & 0x80 == 0 {
            (shortDelta(first), second)
        } else {
            let first_y = stitch_bytes.next().context("truncated PES jump stitch")?;
            (longDelta(first, second), first_y)
        };
        let delta_y = if first_y & 0x80 == 0 {
            shortDelta(first_y)
        } else {
            let second_y = stitch_bytes.next().context("truncated PES jump stitch")?;
            longDelta(first_y, second_y)
        };
        position.0 += delta_x;
        position.1 += delta_y;
        minimum.0 = minimum.0.min(position.0);
        minimum.1 = minimum.1.min(position.1);
        maximum.0 = maximum.0.max(position.0);
        maximum.1 = maximum.1.max(position.1);
        blocks[block_index].points.push(position);
    }

    if minimum.0 == i64::MAX || maximum.0 <= minimum.0 || maximum.1 <= minimum.1 {
        bail!("the PES drawing has no size");
    }
    let width = maximum.0 - minimum.0;
    let height = maximum.1 - minimum.1;
    let mut drawing = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\">\n"
    );
    for (index, block) in blocks.iter_mut().enumerate() {
        let color_index = usize::from(colors.get(index).copied().unwrap_or(0));
        block.color = THREAD_COLORS.get(color_index).copied().unwrap_or([0; 3]);
        let Some(first) = block.points.first() else {
            continue;
        };
        let [red, green, blue] = block.color;
        write!(
            drawing,
            "<path stroke=\"#{red:02x}{green:02x}{blue:02x}\" fill=\"none\" d=\"M {} {}",
            first.0 - minimum.0,
            first.1 - minimum.1
        )?;
        for point in &block.points[1..] {
            write!(
                drawing,
                " L {} {}",
                point.0 - minimum.0,
                point.1 - minimum.1
            )?;
        }
        drawing.push_str("\"/>\n");
    }
    drawing.push_str("</svg>\n");
    Ok(drawing)
}

fn shortDelta(byte: u8) -> i64 {
    let value = i64::from(byte);
    if byte & 0x40 == 0 {
        value
    } else {
        value - 0x80
    }
}

fn longDelta(first: u8, second: u8) -> i64 {
    let value = (i64::from(first & 0x0f) << 8) | i64::from(second);
    if value & 0x800 == 0 {
        value
    } else {
        value - 0x1000
    }
}
