//! LibRaw 0.22's `adobe_coeff` camera colour and level fallbacks.

use std::sync::LazyLock;

use serde::Deserialize;

use super::identify::{Raw, cam_xyz_coeff};

#[derive(Deserialize)]
struct CameraTable {
    camera: Vec<CameraColor>,
}

#[derive(Deserialize)]
struct CameraColor {
    maker: String,
    prefix: String,
    matrix: [i32; 12],
    #[serde(default)]
    black: i32,
    #[serde(default)]
    maximum: u32,
}

static CAMERA_TABLE: LazyLock<CameraTable> = LazyLock::new(|| {
    let source = include_str!("../../../data/libraw-colordata.toml");
    toml::from_str(source).expect("the bundled LibRaw camera table is valid")
});

/// First matching model prefix, as in LibRaw. DNG black and white tags
/// always take precedence over the camera table's levels.
pub(super) fn apply_camera_colors(state: &mut Raw, make: &str, model: &str) -> bool {
    let camera = CAMERA_TABLE.camera.iter().find(|camera| {
        let maker_matches = camera.maker.eq_ignore_ascii_case(make);
        let prefix = model.get(..camera.prefix.len());
        maker_matches && prefix.is_some_and(|prefix| prefix.eq_ignore_ascii_case(&camera.prefix))
    });
    let Some(camera) = camera else {
        return false;
    };

    if state.dng_version == 0 {
        let has_black = state.black != 0 || state.cblack.iter().any(|&level| level != 0);
        if camera.black > 0 || (camera.black < 0 && !has_black) {
            state.black = camera.black.unsigned_abs();
            state.cblack.fill(0);
        }
        if camera.maximum != 0 {
            state.maximum = camera.maximum;
        }
    }
    if camera.matrix[0] != 0 {
        let mut camera_xyz = [[0.0f64; 3]; 4];
        for (row, coefficients) in camera_xyz.iter_mut().enumerate() {
            for (column, coefficient) in coefficients.iter_mut().enumerate() {
                // LibRaw divides as float before assigning the double.
                *coefficient = (camera.matrix[row * 3 + column] as f32 / 10000.0f32) as f64;
            }
        }
        state.rgb_cam = cam_xyz_coeff(&camera_xyz, state.colors, &mut state.pre_mul);
        state.raw_color = false;
    }
    true
}

pub(super) fn normalized_make(make: &str) -> &str {
    let lower_make = make.to_ascii_lowercase();
    for camera in &CAMERA_TABLE.camera {
        if lower_make.contains(&camera.maker.to_ascii_lowercase()) {
            return &camera.maker;
        }
    }
    if lower_make.contains("olympus") || lower_make.contains("om digital") {
        return "Olympus";
    }
    make.trim_end()
}
