//! Decodes length-prefixed HEVC NAL units (from tools/re/heif/heif-nals.py)
//! or an Annex-B stream (split as tools/re/heif/de265-planes.c splits it)
//! and writes the planes in heif-planes' format, for comparing with it.
//!
//! usage: hevc-planes <input.nals|input.hevc> <output.planes>

use std::io::Write;

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() < 3 {
        eprintln!("usage: hevc-planes <input.nals> <output.planes>");
        std::process::exit(2);
    }
    let bytes = std::fs::read(&arguments[1]).expect("read input");
    let units = if arguments[1].ends_with(".hevc") { split_annex_b(&bytes) } else { split_length_prefixed(&bytes) };
    let picture = match nerthus_hevc::decode_nal_units(&units) {
        Ok(picture) => picture,
        Err(error) => {
            eprintln!("decode: {error}");
            std::process::exit(1);
        }
    };
    let mut output = std::fs::File::create(&arguments[2]).expect("create output");
    let mut header = format!("chroma={}", picture.chroma_format);
    let names = ["Y", "Cb", "Cr"];
    let plane_count = if picture.chroma_format == 0 { 1 } else { 3 };
    for (index, name) in names.iter().enumerate().take(plane_count) {
        let (width, height, bits) = if index == 0 { (picture.width, picture.height, picture.bit_depth_luma) } else { (picture.chroma_width, picture.chroma_height, picture.bit_depth_chroma) };
        header.push_str(&format!(" {name}={width}x{height}x{bits}"));
    }
    header.push('\n');
    output.write_all(header.as_bytes()).expect("write header");
    for index in 0..plane_count {
        let mut samples = Vec::with_capacity(picture.planes[index].len() * 2);
        for value in &picture.planes[index] {
            samples.extend_from_slice(&value.to_le_bytes());
        }
        output.write_all(&samples).expect("write plane");
    }
}

fn split_length_prefixed(bytes: &[u8]) -> Vec<&[u8]> {
    let mut units = Vec::new();
    let mut position = 0;
    while position + 4 <= bytes.len() {
        let size = u32::from_be_bytes([bytes[position], bytes[position + 1], bytes[position + 2], bytes[position + 3]]) as usize;
        position += 4;
        units.push(&bytes[position..position + size]);
        position += size;
    }
    units
}

/// Units between start codes, without their trailing zero bytes.
fn split_annex_b(bytes: &[u8]) -> Vec<&[u8]> {
    let mut ranges = Vec::new();
    let mut start: Option<usize> = None;
    let mut position = 0;
    while position + 3 <= bytes.len() {
        if bytes[position] == 0 && bytes[position + 1] == 0 && bytes[position + 2] == 1 {
            if let Some(start) = start {
                ranges.push((start, position));
            }
            position += 3;
            start = Some(position);
            continue;
        }
        position += 1;
    }
    if let Some(start) = start {
        ranges.push((start, bytes.len()));
    }
    let mut units = Vec::with_capacity(ranges.len());
    for (start, mut end) in ranges {
        while end > start && bytes[end - 1] == 0 {
            end -= 1;
        }
        units.push(&bytes[start..end]);
    }
    units
}
