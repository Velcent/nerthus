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
    let camera = find_camera_row(make, model);
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

/// The table's colour values for one camera row.
pub(super) struct MetaColorRow {
    /// A row matched the model prefix.
    pub matched: bool,
    /// The row's camera-to-XYZ matrix (`imgdata.color.cam_xyz`), divided as
    /// float and widened per LibRaw; `None` when the row carries none.
    pub camera_xyz: Option<[[f64; 3]; 4]>,
    /// The table's black level, when its fill rule fired.
    pub black: Option<u32>,
    /// The table's white level, when its fill rule fired.
    pub maximum: Option<u32>,
}

/// The engine's `adobe_coeff` lookup for `adobe_coeff(maker_index, model,
/// internal_only)`: one row, its level fills decided exactly as the engine's
/// non-DNG branch does, but nothing written yet.
pub(super) fn find_meta_color_row(
    make: &str,
    model: &str,
    black: u32,
    cblack: &[u32],
    maximum: u32,
) -> MetaColorRow {
    let mut row = MetaColorRow {
        matched: false,
        camera_xyz: None,
        black: None,
        maximum: None,
    };
    let Some(camera) = find_camera_row(make, model) else {
        return row;
    };
    row.matched = true;

    // `rblack` collects the black levels already parsed.
    let black4 = (cblack[0] + cblack[1] + cblack[2] + cblack[3]) / 4;
    let mut black64: i32 = 0;
    let pattern = cblack[4] as u64 * cblack[5] as u64;
    if pattern > 0 {
        for index in 0..4096.min(pattern as usize) {
            black64 += cblack[index + 6] as i32;
        }
        black64 /= pattern as i32;
    }
    let running_black = black as i32 + black4 as i32 + black64;

    if camera.black > 0 || (camera.black < 0 && running_black == 0) {
        row.black = Some(camera.black.unsigned_abs());
    }
    if camera.maximum != 0 {
        row.maximum = Some(camera.maximum);
    }
    if camera.matrix[0] != 0 {
        let mut camera_xyz = [[0.0f64; 3]; 4];
        for (row_index, coefficients) in camera_xyz.iter_mut().enumerate() {
            for (column, coefficient) in coefficients.iter_mut().enumerate() {
                *coefficient = (camera.matrix[row_index * 3 + column] as f32 / 10000.0f32) as f64;
            }
        }
        row.camera_xyz = Some(camera_xyz);
    }
    row
}

fn find_camera_row(make: &str, model: &str) -> Option<&'static CameraColor> {
    CAMERA_TABLE.camera.iter().find(|camera| {
        let maker_matches = camera.maker.eq_ignore_ascii_case(make);
        let prefix = model.get(..camera.prefix.len());
        maker_matches && prefix.is_some_and(|prefix| prefix.eq_ignore_ascii_case(&camera.prefix))
    })
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
