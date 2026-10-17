//! WPF colour conversions shared by the minimal colour picker and the
//! property editors. Projects store scRGB floats; picker colours use sRGB
//! bytes, as Gaea's `Color.ScR/ScG/ScB` and `Color.R/G/B` do.

/// `Color.sRgbToScRgb`.
pub fn srgb_to_linear(byte: u8) -> f32 {
    let value = byte as f32 / 255.0;
    if value.is_nan() || value <= 0.0 {
        0.0
    } else if value <= 0.04045 {
        value / 12.92
    } else if value < 1.0 {
        (((value as f64) + 0.055) / 1.055).powf(2.4) as f32
    } else {
        1.0
    }
}

/// `Color.ScRgbTosRgb`.
pub fn linear_to_srgb(value: f32) -> u8 {
    if value.is_nan() || value <= 0.0 {
        0
    } else if value <= 0.0031308 {
        ((255.0 * value * 12.92) + 0.5) as u8
    } else if value < 1.0 {
        ((255.0 * ((1.055 * (value as f64).powf(1.0 / 2.4) as f32) - 0.055)) + 0.5) as u8
    } else {
        255
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wpf_colour_conversions_round_trip() {
        for byte in 0..=255u8 {
            assert_eq!(linear_to_srgb(srgb_to_linear(byte)), byte);
        }
        assert_eq!(srgb_to_linear(255), 1.0);
        assert_eq!(linear_to_srgb(0.5), 188);
    }
}
