//! Format-specific raw unpacking in Rust. Rawler supplies sensor samples
//! and camera metadata; Nerthus applies the same processing as its DNG
//! reader, including LibRaw's camera table, demosaicing and gamma curve.

use anyhow::{Context, Result, bail, ensure};
use rawler::decoders::RawDecodeParams;
use rawler::rawimage::RawPhotometricInterpretation;
use rawler::rawsource::RawSource;
use rawler::{RawImage, RawImageData};

use super::camera_colors::{apply_camera_colors, normalized_make};
use super::camera_loaders::unpack_camera;
use super::camera_raw_meta::{CameraMeta, parse_camera_meta, SensorKind};
use super::identify::{CBLACK_SIZE, Decoder, NOT_TILED, Raw, cam_xyz_coeff};
use super::process::{self, Output};
use super::stream::LITTLE_ENDIAN;
use super::unpack::RawData;

pub(super) fn decode_camera_raw(bytes: &[u8]) -> Result<Output> {
    // The engine's own pixel loaders run for the makers whose loader is
    // ported; rawler still decodes the remainder.
    let engine_metadata = parse_camera_meta(bytes).filter(|meta| {
        matches!(
            meta.loader,
            SensorKind::Samsung
                | SensorKind::Pentax
                | SensorKind::Olympus
                | SensorKind::Panasonic
                | SensorKind::Unpacked
                | SensorKind::NikonPlain
                | SensorKind::CanonLosslessJpeg
                | SensorKind::CanonCiff
                | SensorKind::CanonSraw
                | SensorKind::CanonCrx
                | SensorKind::FujiCompressed
                | SensorKind::SonyArw2
                | SensorKind::SonyArw
        )
    });
    if let Some(mut meta) = engine_metadata {
        ensure!(
            (22..=64000).contains(&meta.raw_width) && (22..=64000).contains(&meta.raw_height),
            "invalid camera raw dimensions"
        );
        let mut state = unpacked_camera_state(&meta);
        let raw_data = unpack_camera(&mut meta, &mut state, bytes)?;
        return process::process(&mut state, raw_data);
    }

    let source = RawSource::new_from_slice(bytes);
    let parameters = RawDecodeParams::default();
    let decoded = rawler::decode(&source, &parameters).context("cannot unpack the camera raw image")?;
    let (mut state, samples) = prepare_camera_raw(decoded)?;
    process::process(&mut state, samples)
}

/// The engine's `Raw` state from the walker's resolved metadata, the path
/// the ported pixel loaders take. The walker carries the colour state the
/// engine's identify reaches, so the ported processing runs on it directly.
fn unpacked_camera_state(meta: &CameraMeta) -> Raw {
    Raw {
        order: LITTLE_ENDIAN,
        make: meta.make.clone(),
        model: meta.model.clone(),
        dng_version: 0,
        raw_width: meta.raw_width,
        raw_height: meta.raw_height,
        width: meta.width,
        height: meta.height,
        top_margin: meta.top_margin,
        left_margin: meta.left_margin,
        pixel_aspect: meta.pixel_aspect,
        fuji_width: meta.fuji_width,
        fuji_raw_rotated: meta.fuji_layout != 0,
        filters: meta.filters,
        colors: meta.colors,
        tiff_bps: meta.tiff_bps,
        tiff_samples: meta.tiff_samples,
        data_offset: meta.data_offset,
        tile_width: NOT_TILED,
        tile_length: NOT_TILED,
        meta_offset: 0,
        decoder: Decoder::Packed,
        curve: meta.curve.clone(),
        black: meta.black,
        cblack: meta.cblack.to_vec(),
        maximum: meta.maximum,
        pre_mul: meta.pre_mul,
        cam_mul: meta.cam_mul,
        rgb_cam: meta.rgb_cam,
        raw_color: meta.raw_color,
        mask: meta.mask,
        xtrans: meta.xtrans,
        zero_is_bad: meta.zero_is_bad,
        ifds: Vec::new(),
        raw_ifd: 0,
    }
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
        zero_is_bad: false,
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

    /// Facts about each camera raw for comparing with the LibRaw probe's
    /// `0-opened.txt`: NERTHUS_RAW_DIAGNOSTICS=<dump dir> cargo test
    /// -p nerthus-io inspect_raw_metadata -- --ignored
    #[test]
    #[ignore]
    fn inspect_raw_metadata() {
        let directory = std::env::var("NERTHUS_RAW_DIAGNOSTICS").unwrap();
        let suffixes = ["cr2", "cr3", "crw", "nef", "arw", "orf", "raf", "rw2", "pef", "srw"];
        let wanted: Vec<usize> = std::env::var("NERTHUS_RAW_CASES")
            .ok()
            .map(|list| list.split(',').map(|case| case.trim().parse().unwrap()).collect())
            .unwrap_or((1..=20).collect());
        for index in wanted {
            let folder = std::path::Path::new(&directory).join(format!("{index:02}"));
            let entries = std::fs::read_dir(folder).unwrap().collect::<Vec<_>>();
            for entry in entries {
                let path = entry.unwrap().path();
                let extension = path.extension().unwrap().to_str().unwrap().to_ascii_lowercase();
                if !suffixes.contains(&extension.as_str()) { continue; }
                let bytes = std::fs::read(&path).unwrap();
                if let Some(meta) = super::super::camera_raw_meta::parse_camera_meta(&bytes) {
                    println!(
                        "CASE {index:02} width {} height {} raw_width {} raw_height {} top {} left {} colors {} filters {} black {} maximum {} bps {} samples {} load_flags {} zero_is_bad {}",
                        meta.width, meta.height, meta.raw_width, meta.raw_height, meta.top_margin,
                        meta.left_margin, meta.colors, meta.filters, meta.black, meta.maximum,
                        meta.tiff_bps, meta.tiff_samples, meta.load_flags, u32::from(meta.zero_is_bad)
                    );
                    let cblack: Vec<String> = meta.cblack.iter().map(|&level| level.to_string()).collect();
                    println!("CASE {index:02} cblack {}", cblack.join(" "));
                    let pre_mul: Vec<String> = meta.pre_mul.iter().map(|&value| format!("{value:?}")).collect();
                    let cam_mul: Vec<String> = meta.cam_mul.iter().map(|&value| format!("{value:?}")).collect();
                    println!("CASE {index:02} pre_mul {} cam_mul {}", pre_mul.join(" "), cam_mul.join(" "));
                    for row in &meta.rgb_cam {
                        let values: Vec<String> = row.iter().map(|&value| format!("{value:?}")).collect();
                        println!("CASE {index:02} rgb_cam {}", values.join(" "));
                    }
                    continue;
                }
                let source = RawSource::new_from_slice(&bytes);
                let params = RawDecodeParams::default();
                let decoded = rawler::decode(&source, &params).unwrap();
                println!(
                    "CASE {index:02} make {:?} model {:?} clean {:?} {:?}",
                    decoded.make, decoded.model, decoded.clean_make, decoded.clean_model
                );
                let (state, _) = prepare_camera_raw(decoded).unwrap();
                println!(
                    "CASE {index:02} width {} height {} raw_width {} raw_height {} top {} left {}",
                    state.width, state.height, state.raw_width, state.raw_height, state.top_margin, state.left_margin
                );
                println!("CASE {index:02} colors {} filters {} black {} maximum {} bps {} samples {}", state.colors, state.filters, state.black, state.maximum, state.tiff_bps, state.tiff_samples);
                let cblack: Vec<String> = state.cblack.iter().map(|&level| level.to_string()).collect();
                println!("CASE {index:02} cblack {}", cblack.join(" "));
                let pre_mul: Vec<String> = state.pre_mul.iter().map(|&value| format!("{value:?}")).collect();
                let cam_mul: Vec<String> = state.cam_mul.iter().map(|&value| format!("{value:?}")).collect();
                println!("CASE {index:02} pre_mul {} cam_mul {}", pre_mul.join(" "), cam_mul.join(" "));
                for row in &state.rgb_cam {
                    let values: Vec<String> = row.iter().map(|&value| format!("{value:?}")).collect();
                    println!("CASE {index:02} rgb_cam {}", values.join(" "));
                }
            }
        }
    }

    /// The engine's own pixel decoding for byte comparisons with the LibRaw
    /// probe's `raw.bin` and `final.bin` dumps: NERTHUS_RAW_DIAGNOSTICS=<case
    /// dirs> NERTHUS_RAW_DECODE=<out dir> cargo test -p nerthus-io
    /// inspect_raw_decode -- --ignored
    #[test]
    #[ignore]
    fn inspect_raw_decode() {
        let directory = std::env::var("NERTHUS_RAW_DIAGNOSTICS").unwrap();
        let decoded_directory = std::env::var("NERTHUS_RAW_DECODE").unwrap();
        let suffixes = ["cr2", "cr3", "crw", "nef", "arw", "orf", "raf", "rw2", "pef", "srw"];
        let wanted: Vec<usize> = std::env::var("NERTHUS_RAW_CASES")
            .ok()
            .map(|list| list.split(',').map(|case| case.trim().parse().unwrap()).collect())
            .unwrap_or((1..=20).collect());
        for index in wanted {
            let folder = std::path::Path::new(&directory).join(format!("{index:02}"));
            let entries = std::fs::read_dir(folder).unwrap().collect::<Vec<_>>();
            for entry in entries {
                let path = entry.unwrap().path();
                let extension = path.extension().unwrap().to_str().unwrap().to_ascii_lowercase();
                if !suffixes.contains(&extension.as_str()) { continue; }
                let source_bytes = std::fs::read(&path).unwrap();
                // Only cases the engine's own loader runs on dump; the rest
                // stay with rawler.
                let metadata = parse_camera_meta(&source_bytes).filter(|meta| {
                    matches!(
                        meta.loader,
                        SensorKind::Samsung
                | SensorKind::Pentax
                | SensorKind::Olympus
                | SensorKind::Panasonic
                | SensorKind::Unpacked
                | SensorKind::NikonPlain
                | SensorKind::CanonLosslessJpeg
                | SensorKind::CanonCiff
                | SensorKind::CanonSraw
                | SensorKind::CanonCrx
                | SensorKind::FujiCompressed
                | SensorKind::SonyArw2
                | SensorKind::SonyArw
                    )
                });
                let Some(mut meta) = metadata else { continue; };
                let mut state = unpacked_camera_state(&meta);
                let raw_data = unpack_camera(&mut meta, &mut state, &source_bytes).unwrap();

                let raw_channels = match raw_data {
                    RawData::Bayer(_) => 1u32,
                    RawData::Color3(_) => 3,
                    RawData::Color4(_) => 4,
                };
                let raw_header = [meta.raw_width, meta.raw_height, meta.raw_pitch, raw_channels];
                let mut raw_dump = Vec::new();
                for value in raw_header {
                    raw_dump.extend_from_slice(&value.to_le_bytes());
                }
                match &raw_data {
                    RawData::Bayer(buffer) => {
                        for &sample in buffer {
                            raw_dump.extend_from_slice(&sample.to_le_bytes());
                        }
                    }
                    RawData::Color3(pixels) => {
                        for pixel in pixels {
                            for &sample in pixel {
                                raw_dump.extend_from_slice(&sample.to_le_bytes());
                            }
                        }
                    }
                    RawData::Color4(pixels) => {
                        for pixel in pixels {
                            for &sample in pixel {
                                raw_dump.extend_from_slice(&sample.to_le_bytes());
                            }
                        }
                    }
                }
                let raw_path = std::path::Path::new(&decoded_directory).join(format!("{index:02}-raw.bin"));
                std::fs::write(raw_path, raw_dump).unwrap();

                let output = process::process(&mut state, raw_data).unwrap();
                let final_header = [output.width as u32, output.height as u32, output.colors as u32, 16u32];
                let mut final_dump = Vec::new();
                for value in final_header {
                    final_dump.extend_from_slice(&value.to_le_bytes());
                }
                for &sample in &output.samples {
                    final_dump.extend_from_slice(&sample.to_le_bytes());
                }
                let final_path = std::path::Path::new(&decoded_directory).join(format!("{index:02}-final.bin"));
                std::fs::write(final_path, final_dump).unwrap();

                println!("CASE {index:02} decoded {}x{} colours {}", output.width, output.height, output.colors);
            }
        }
    }
}
