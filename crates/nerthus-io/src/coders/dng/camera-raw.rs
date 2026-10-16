//! Format-specific raw unpacking in Rust. Rawler supplies sensor samples
//! and camera metadata; Nerthus applies the same processing as its DNG
//! reader, including LibRaw's camera table, demosaicing and gamma curve.

use anyhow::{Context, Result, bail, ensure};
use rawler::decoders::RawDecodeParams;
use rawler::rawimage::RawPhotometricInterpretation;
use rawler::rawsource::RawSource;
use rawler::{RawImage, RawImageData};

use super::camera_colors::{apply_camera_colors, normalized_make};
use super::identify::{CBLACK_SIZE, Decoder, NOT_TILED, Raw, cam_xyz_coeff};
use super::process::{self, Output};
use super::stream::LITTLE_ENDIAN;
use super::unpack::RawData;

pub(super) fn decode_camera_raw(bytes: &[u8]) -> Result<Output> {
    let source = RawSource::new_from_slice(bytes);
    let parameters = RawDecodeParams::default();
    let decoded = rawler::decode(&source, &parameters).context("cannot unpack the camera raw image")?;
    let (mut state, samples) = prepare_camera_raw(decoded)?;
    process::process(&mut state, samples)
}

fn prepare_camera_raw(decoded: RawImage) -> Result<(Raw, RawData)> {
    let raw_width = decoded.width;
    let raw_height = decoded.height;
    ensure!((22..=64000).contains(&raw_width) && (22..=64000).contains(&raw_height), "invalid camera raw dimensions");
    ensure!((1..=4).contains(&decoded.cpp), "unsupported camera raw components");

    let mut left = 0;
    let mut top = 0;
    let mut width = raw_width;
    let mut height = raw_height;
    if decoded.fuji_rotation_width.is_none() {
        if let Some(area) = decoded.active_area {
            left = area.p.x;
            top = area.p.y;
            width = area.d.w;
            height = area.d.h;
        }
    }
    ensure!(width >= 22 && height >= 22 && left + width <= raw_width && top + height <= raw_height, "invalid camera raw active area");

    let mut colors = decoded.cpp;
    let mut filters = 0;
    let mut xtrans = [[0u8; 6]; 6];
    if let RawPhotometricInterpretation::Cfa(configuration) = &decoded.photometric {
        ensure!(decoded.cpp == 1, "invalid camera raw CFA components");
        let cfa = &configuration.cfa;
        let plane_lookup = configuration.colors.plane_lookup_table();
        colors = configuration.colors.plane_count();
        ensure!((1..=4).contains(&colors), "unsupported camera raw colour planes");
        if cfa.width == 6 && cfa.height == 6 {
            filters = 9;
            for (row, pattern_row) in xtrans.iter_mut().enumerate() {
                for (column, colour) in pattern_row.iter_mut().enumerate() {
                    let plane = plane_lookup[cfa.color_at(row + top, column + left)];
                    ensure!(plane < colors, "invalid camera raw CFA colour");
                    *colour = plane as u8;
                }
            }
        } else if cfa.width == 2 && matches!(cfa.height, 2 | 8) {
            for row in 0..8 {
                for column in 0..2 {
                    let plane = plane_lookup[cfa.color_at(row + top, column + left)];
                    ensure!(plane < colors, "invalid camera raw CFA colour");
                    filters |= (plane as u32) << ((row * 2 + column) * 2);
                }
            }
            if colors == 3 {
                filters |= ((filters >> 2 & 0x22222222) | (filters << 2 & 0x88888888)) & filters << 1;
            }
        } else {
            bail!("unsupported camera raw CFA layout {}×{}", cfa.width, cfa.height);
        }
    }

    let mut maximum = *decoded.whitelevel.0.first().context("missing camera raw white level")?;
    let is_float = matches!(decoded.data, RawImageData::Float(_));
    let sample_scale = if is_float { 65535.0f32 / maximum.max(1) as f32 } else { 1.0 };
    if is_float {
        maximum = 65535;
    }
    let mut cblack = vec![0u32; CBLACK_SIZE];
    let black_levels = &decoded.blacklevel;
    let black_count = black_levels.width.checked_mul(black_levels.height).context("invalid camera raw black level dimensions")?;
    ensure!(black_count > 0 && black_count <= CBLACK_SIZE - 6, "invalid camera raw black level pattern");
    if decoded.cpp == 1 {
        ensure!(black_levels.levels.len() >= black_count, "missing camera raw black levels");
        cblack[4] = black_levels.height as u32;
        cblack[5] = black_levels.width as u32;
        for row in 0..black_levels.height {
            for column in 0..black_levels.width {
                let source_row = (row + top) % black_levels.height;
                let source_column = (column + left) % black_levels.width;
                let level = black_levels.levels[source_row * black_levels.width + source_column].as_f32();
                cblack[6 + row * black_levels.width + column] = (level * sample_scale) as u32;
            }
        }
    } else {
        ensure!(black_levels.levels.len() >= decoded.cpp, "missing camera raw channel black levels");
        for (channel, value) in cblack[..decoded.cpp].iter_mut().enumerate() {
            *value = (black_levels.levels[channel].as_f32() * sample_scale) as u32;
        }
    }
    let mut camera_multipliers = decoded.wb_coeffs;
    for multiplier in &mut camera_multipliers {
        if !multiplier.is_finite() || *multiplier <= 0.001 {
            *multiplier = 0.0;
        }
    }
    let mut state = Raw {
        order: LITTLE_ENDIAN,
        make: decoded.make.clone(),
        model: decoded.model.clone(),
        dng_version: 0,
        raw_width: raw_width as u32,
        raw_height: raw_height as u32,
        width: width as u32,
        height: height as u32,
        left_margin: left as u32,
        top_margin: top as u32,
        pixel_aspect: 1.0,
        fuji_width: decoded.fuji_rotation_width.unwrap_or(0) as u32,
        fuji_raw_rotated: decoded.fuji_rotation_width.is_some(),
        filters,
        colors,
        tiff_bps: decoded.bps as u32,
        tiff_samples: decoded.cpp as u32,
        data_offset: 0,
        tile_width: NOT_TILED,
        tile_length: NOT_TILED,
        meta_offset: 0,
        decoder: Decoder::Packed,
        curve: Vec::new(),
        black: 0,
        cblack,
        maximum,
        pre_mul: [1.0, 1.0, 1.0, 0.0],
        cam_mul: camera_multipliers,
        rgb_cam: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
        raw_color: true,
        mask: [[0; 4]; 8],
        xtrans,
        ifds: Vec::new(),
        raw_ifd: 0,
    };
    let make = normalized_make(&decoded.clean_make);
    let has_camera_color = apply_camera_colors(&mut state, make, &decoded.clean_model);
    if !has_camera_color {
        let mut camera_xyz = [[0.0f64; 3]; 4];
        for (target, source) in camera_xyz.iter_mut().zip(decoded.xyz_to_cam) {
            for (target_value, source_value) in target.iter_mut().zip(source) {
                *target_value = source_value as f64;
            }
        }
        if camera_xyz[0][0] != 0.0 {
            state.rgb_cam = cam_xyz_coeff(&camera_xyz, colors, &mut state.pre_mul);
            state.raw_color = false;
        }
    }

    let samples = match decoded.data {
        RawImageData::Integer(samples) => samples,
        RawImageData::Float(values) => values.iter().map(|&value| (value * sample_scale).clamp(0.0, 65535.0) as u16).collect(),
    };
    let expected_count = raw_width * raw_height * decoded.cpp;
    ensure!(samples.len() == expected_count, "incomplete camera raw sensor data");
    let raw_data = match decoded.cpp {
        1 => RawData::Bayer(samples),
        3 => {
            let pixels = samples.chunks_exact(3).map(|pixel| [pixel[0], pixel[1], pixel[2]]).collect();
            RawData::Color3(pixels)
        }
        4 => {
            let pixels = samples.chunks_exact(4).map(|pixel| [pixel[0], pixel[1], pixel[2], pixel[3]]).collect();
            RawData::Color4(pixels)
        }
        _ => bail!("unsupported camera raw components"),
    };
    Ok((state, raw_data))
}

#[cfg(test)]
mod diagnostics {
    use super::*;

    #[test]
    #[ignore]
    fn inspect_raw_metadata() {
        let directory = std::env::var("NERTHUS_RAW_DIAGNOSTICS").unwrap();
        for index in 1..=20 {
            let folder = std::path::Path::new(&directory).join(format!("{index:02}"));
            for entry in std::fs::read_dir(folder).unwrap() {
                let path = entry.unwrap().path();
                let extension = path.extension().unwrap().to_str().unwrap().to_ascii_lowercase();
                if !["cr2", "cr3", "crw", "nef", "arw", "orf", "raf", "rw2", "pef", "srw"].contains(&extension.as_str()) { continue; }
                let bytes = std::fs::read(&path).unwrap();
                let source = RawSource::new_from_slice(&bytes);
                let params = RawDecodeParams::default();
                let decoded = rawler::decode(&source, &params).unwrap();
                eprintln!("CASE {index:02} {} {} {}x{} active {:?} crop {:?} fuji {:?} WB {:?} black {:?} white {:?} cfa {:?}", decoded.clean_make, decoded.clean_model, decoded.width, decoded.height, decoded.active_area, decoded.crop_area, decoded.fuji_rotation_width, decoded.wb_coeffs, decoded.blacklevel, decoded.whitelevel, decoded.photometric);
                let (state, _) = prepare_camera_raw(decoded).unwrap();
                eprintln!("STATE {index:02} {}x{} margins {}:{} filters {:x} pre {:?} rgb {:?}",state.width,state.height,state.left_margin,state.top_margin,state.filters,state.pre_mul,state.rgb_cam);
            }
        }
    }
}
