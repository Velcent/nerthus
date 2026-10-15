//! nclx colour profiles and the YCbCr matrices libheif derives from them
//! (`nclx.cc`), in single precision as there.

/// `nclx_profile`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Nclx {
    pub colour_primaries: u16,
    pub transfer_characteristics: u16,
    pub matrix_coefficients: u16,
    pub full_range: bool,
}

impl Nclx {
    /// `nclx_profile::undefined` (all unspecified, full range).
    pub fn undefined() -> Nclx {
        Nclx { colour_primaries: 2, transfer_characteristics: 2, matrix_coefficients: 2, full_range: true }
    }

    /// `nclx_profile::defaults` (sRGB).
    pub fn defaults() -> Nclx {
        Nclx { colour_primaries: 1, transfer_characteristics: 13, matrix_coefficients: 6, full_range: true }
    }

    /// `replace_undefined_values_with_sRGB_defaults`.
    pub fn with_srgb_defaults_for_unspecified(mut self) -> Nclx {
        if self.matrix_coefficients == 2 {
            self.matrix_coefficients = 6;
        }
        if self.colour_primaries == 2 {
            self.colour_primaries = 1;
        }
        if self.transfer_characteristics == 2 {
            self.transfer_characteristics = 13;
        }
        self
    }

    /// `equal_except_transfer_curve`.
    pub fn equal_except_transfer(&self, other: &Nclx) -> bool {
        self.matrix_coefficients == other.matrix_coefficients && self.colour_primaries == other.colour_primaries && self.full_range == other.full_range
    }
}

/// `heif_nclx_color_profile_set_*`'s accepted values; others become
/// unspecified (2).
pub fn known_colour_primaries(value: u16) -> u16 {
    if matches!(value, 1 | 2 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 22) { value } else { 2 }
}

pub fn known_transfer_characteristics(value: u16) -> u16 {
    if matches!(value, 1 | 2 | 4..=18) { value } else { 2 }
}

pub fn known_matrix_coefficients(value: u16) -> u16 {
    if matches!(value, 0 | 1 | 2 | 4..=14) { value } else { 2 }
}

struct Primaries {
    red_x: f32,
    red_y: f32,
    green_x: f32,
    green_y: f32,
    blue_x: f32,
    blue_y: f32,
    white_x: f32,
    white_y: f32,
}

/// `get_colour_primaries` (all zero when undefined).
fn colour_primaries(index: u16) -> Primaries {
    let (gx, gy, bx, by, rx, ry, wx, wy) = match index {
        1 => (0.300, 0.600, 0.150, 0.060, 0.640, 0.330, 0.3127, 0.3290),
        4 => (0.21, 0.71, 0.14, 0.08, 0.67, 0.33, 0.310, 0.316),
        5 => (0.29, 0.60, 0.15, 0.06, 0.64, 0.33, 0.3127, 0.3290),
        6 | 7 => (0.310, 0.595, 0.155, 0.070, 0.630, 0.340, 0.3127, 0.3290),
        8 => (0.243, 0.692, 0.145, 0.049, 0.681, 0.319, 0.310, 0.316),
        9 => (0.170, 0.797, 0.131, 0.046, 0.708, 0.292, 0.3127, 0.3290),
        10 => (0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.333333, 0.33333),
        11 => (0.265, 0.690, 0.150, 0.060, 0.680, 0.320, 0.314, 0.351),
        12 => (0.265, 0.690, 0.150, 0.060, 0.680, 0.320, 0.3127, 0.3290),
        22 => (0.295, 0.605, 0.155, 0.077, 0.630, 0.340, 0.3127, 0.3290),
        _ => (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
    };
    Primaries { red_x: rx, red_y: ry, green_x: gx, green_y: gy, blue_x: bx, blue_y: by, white_x: wx, white_y: wy }
}

/// `get_Kr_Kb` (zeros when the matrix is not one with weights).
fn kr_kb(matrix: u16, primaries: u16) -> (f32, f32) {
    if matrix == 12 || matrix == 13 {
        let p = colour_primaries(primaries);
        let zr = 1.0 - (p.red_x + p.red_y);
        let zg = 1.0 - (p.green_x + p.green_y);
        let zb = 1.0 - (p.blue_x + p.blue_y);
        let zw = 1.0 - (p.white_x + p.white_y);
        let denominator = p.white_y * (p.red_x * (p.green_y * zb - p.blue_y * zg) + p.green_x * (p.blue_y * zr - p.red_y * zb) + p.blue_x * (p.red_y * zg - p.green_y * zr));
        if denominator == 0.0 {
            return (0.0, 0.0);
        }
        let kr = (p.red_y * (p.white_x * (p.green_y * zb - p.blue_y * zg) + p.white_y * (p.blue_x * zg - p.green_x * zb) + zw * (p.green_x * p.blue_y - p.blue_x * p.green_y))) / denominator;
        let kb = (p.blue_y * (p.white_x * (p.red_y * zg - p.green_y * zr) + p.white_y * (p.green_x * zr - p.red_x * zg) + zw * (p.red_x * p.green_y - p.green_x * p.red_y))) / denominator;
        return (kr, kb);
    }
    match matrix {
        1 => (0.2126, 0.0722),
        4 => (0.30, 0.11),
        5 | 6 => (0.299, 0.114),
        7 => (0.212, 0.087),
        9 | 10 => (0.2627, 0.0593),
        _ => (0.0, 0.0),
    }
}

/// `YCbCr_to_RGB_coefficients`.
#[derive(Clone, Copy, Debug)]
pub struct YcbcrToRgb {
    pub r_cr: f32,
    pub g_cb: f32,
    pub g_cr: f32,
    pub b_cb: f32,
}

impl YcbcrToRgb {
    pub fn defaults() -> YcbcrToRgb {
        YcbcrToRgb { r_cr: 1.402, g_cb: -0.344136, g_cr: -0.714136, b_cb: 1.772 }
    }
}

/// `get_YCbCr_to_RGB_coefficients`.
pub fn ycbcr_to_rgb_coefficients(matrix: u16, primaries: u16) -> YcbcrToRgb {
    let (kr, kb) = kr_kb(matrix, primaries);
    if kb != 0.0 || kr != 0.0 {
        let denominator = kb + kr - 1.0;
        YcbcrToRgb { r_cr: 2.0 * (-kr + 1.0), g_cb: 2.0 * kb * (-kb + 1.0) / denominator, g_cr: 2.0 * kr * (-kr + 1.0) / denominator, b_cb: 2.0 * (-kb + 1.0) }
    } else {
        YcbcrToRgb::defaults()
    }
}

/// `RGB_to_YCbCr_coefficients`.
#[derive(Clone, Copy, Debug)]
pub struct RgbToYcbcr {
    pub c: [[f32; 3]; 3],
}

/// `get_RGB_to_YCbCr_coefficients`.
pub fn rgb_to_ycbcr_coefficients(matrix: u16, primaries: u16) -> RgbToYcbcr {
    let (kr, kb) = kr_kb(matrix, primaries);
    if kb != 0.0 || kr != 0.0 {
        let c = [
            [kr, 1.0 - kr - kb, kb],
            [-kr / (1.0 - kb) / 2.0, -(1.0 - kr - kb) / (1.0 - kb) / 2.0, 0.5],
            [0.5, -(1.0 - kr - kb) / (1.0 - kr) / 2.0, -kb / (1.0 - kr) / 2.0],
        ];
        RgbToYcbcr { c }
    } else {
        RgbToYcbcr { c: [[0.299, 0.587, 0.114], [-0.168735, -0.331264, 0.5], [0.5, -0.418688, -0.081312]] }
    }
}
