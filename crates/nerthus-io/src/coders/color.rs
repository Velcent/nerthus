//! Colours by name as ImageMagick reads them (`QueryColorCompliance`):
//! `#` hex in 1 to 16 digits a channel, CSS-style functions such as
//! `rgb(…)`, `hsl(…)` or `gray(…)`, and the built-in names
//! (`data/colors.toml`), each known to some standards only.

use std::sync::OnceLock;

use nerthus_image::colorspace_transform::{decode_pixel_gamma, hue_to_rgb, Colorspace};
use serde::Deserialize;

use super::text::{interpret_locale_value, is_space};

const QUANTUM_RANGE: f64 = 65535.0;

/// The standards a name lookup sees (`ComplianceType`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Compliance {
    Xpm,
    All,
}

impl Compliance {
    fn accepts(self, standards: &str) -> bool {
        let wanted = match self {
            Compliance::Xpm => "XPM",
            Compliance::All => return true,
        };
        standards.split(' ').any(|standard| standard == wanted)
    }
}

/// A parsed colour (`PixelInfo`): quanta as doubles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Color {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub black: f64,
    pub alpha: f64,
    /// Whether the colour says its alpha (`alpha_trait`).
    pub has_alpha: bool,
}

impl Color {
    /// `GetPixelInfo`: opaque black.
    fn new() -> Self {
        Color { red: 0.0, green: 0.0, blue: 0.0, black: 0.0, alpha: QUANTUM_RANGE, has_alpha: false }
    }
}

#[derive(Deserialize)]
struct NamedColor {
    name: String,
    rgb: [u8; 3],
    alpha: f32,
    standards: String,
}

#[derive(Deserialize)]
struct ColorTable {
    colors: Vec<NamedColor>,
}

fn named_colors() -> &'static [NamedColor] {
    static TABLE: OnceLock<Vec<NamedColor>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let text = include_str!("../../data/colors.toml");
        let table: ColorTable = toml::from_str(text).expect("the colour table parses");
        table.colors
    })
}

fn at(bytes: &[u8], index: usize) -> u8 {
    bytes.get(index).copied().unwrap_or(0)
}

/// A C string: the bytes up to the first NUL.
fn c_str(bytes: &[u8]) -> &[u8] {
    let end = bytes.iter().position(|&byte| byte == 0).unwrap_or(bytes.len());
    &bytes[..end]
}

/// `GetQuantumRange`.
fn quantum_range(depth: u32) -> u64 {
    if depth == 0 {
        return 0;
    }
    let shift = depth.min(64) - 1;
    (1u64 << shift) + ((1u64 << shift) - 1)
}

/// `ScaleAnyToQuantum` (HDRI: no rounding).
fn scale_any_to_quantum(quantum: u64, range: u64) -> f64 {
    if quantum > range {
        return QUANTUM_RANGE;
    }
    let range = range as f64;
    let reciprocal = if range > -1.0e-12 && range < 1.0e-12 { 1.0e12 } else { 1.0 / range };
    (QUANTUM_RANGE * (quantum as f64 * reciprocal)) as f32 as f64
}

/// `ClampToQuantum` (HDRI: only the conversion to a float quantum).
fn clamp_to_quantum(value: f64) -> f64 {
    value as f32 as f64
}

/// `ConvertRGBToCMYK` of an sRGB colour (gamma decoded first): cyan,
/// magenta, yellow and black quanta, as a CMYK image's background takes
/// them (white keeps a trace of black from the decoding).
pub(crate) fn srgb_to_cmyk(red: f64, green: f64, blue: f64) -> [f64; 4] {
    let scale = 1.0 / QUANTUM_RANGE;
    let red = scale * decode_pixel_gamma(red);
    let green = scale * decode_pixel_gamma(green);
    let blue = scale * decode_pixel_gamma(blue);
    if red.abs() < 1.0e-12 && green.abs() < 1.0e-12 && blue.abs() < 1.0e-12 {
        return [0.0, 0.0, 0.0, QUANTUM_RANGE];
    }
    let cyan = 1.0 - red;
    let magenta = 1.0 - green;
    let yellow = 1.0 - blue;
    let black = cyan.min(magenta).min(yellow);
    let span = 1.0 - black;
    let reciprocal = if span > -1.0e-12 && span < 1.0e-12 { 1.0e12 } else { 1.0 / span };
    [QUANTUM_RANGE * (reciprocal * (cyan - black)), QUANTUM_RANGE * (reciprocal * (magenta - black)), QUANTUM_RANGE * (reciprocal * (yellow - black)), QUANTUM_RANGE * black]
}

/// `QueryColorCompliance`: `None` for an unknown colour.
pub(crate) fn query(name: &[u8], compliance: Compliance) -> Option<Color> {
    let name = c_str(name);
    let name: &[u8] = if name.is_empty() { b"#ffffff" } else { name };
    if name.len() > 4096 {
        return None;
    }
    let start = name.iter().position(|&byte| !is_space(byte)).unwrap_or(name.len());
    let name = &name[start..];
    if name.first() == Some(&b'#') {
        return parse_hex(&name[1..]);
    }
    if let Some(open) = name.iter().position(|&byte| byte == b'(') {
        return parse_function(name, open);
    }
    lookup_name(name, compliance)
}

fn hex_value(byte: u8) -> Option<u32> {
    (byte as char).to_digit(16)
}

/// A `#` colour: 3, 6, 9, 12, 24 or 48 digits of RGB, or 4, 8, 16, 32
/// or 64 of RGBA, each channel kept in 32 bits.
fn parse_hex(digits: &[u8]) -> Option<Color> {
    let count = digits.iter().take_while(|byte| byte.is_ascii_hexdigit()).count();
    let mut red;
    let mut green = 0u32;
    let mut blue = 0u32;
    let mut alpha = 0u32;
    let mut position = 0;
    
    let depth = if matches!(count, 3 | 6 | 9 | 12 | 24 | 48) {
        loop {
            red = green;
            green = blue;
            blue = 0;
            for _ in 0..count / 3 {
                blue = (blue << 4) | hex_value(digits[position])?;
                position += 1;
            }
            if !at(digits, position).is_ascii_hexdigit() {
                break;
            }
        }
        4 * (count / 3) as u32
    } else {
        if !matches!(count, 4 | 8 | 16 | 32 | 64) {
            return None;
        }
        loop {
            red = green;
            green = blue;
            blue = alpha;
            alpha = 0;
            for _ in 0..count / 4 {
                alpha = (alpha << 4) | hex_value(digits[position])?;
                position += 1;
            }
            if !at(digits, position).is_ascii_hexdigit() {
                break;
            }
        }
        4 * (count / 4) as u32
    };
    let range = quantum_range(depth);
    let mut color = Color::new();
    color.red = scale_any_to_quantum(red as u64, range);
    color.green = scale_any_to_quantum(green as u64, range);
    color.blue = scale_any_to_quantum(blue as u64, range);
    if !count.is_multiple_of(3) {
        color.has_alpha = true;
        color.alpha = scale_any_to_quantum(alpha as u64, range);
    }
    Some(color)
}

/// A built-in name, blanks ignored and case not, among the names the
/// standard knows.
fn lookup_name(name: &[u8], compliance: Compliance) -> Option<Color> {
    let stripped: Vec<u8> = name.iter().copied().filter(|&byte| !is_space(byte)).collect();
    let found = named_colors().iter().find(|entry| compliance.accepts(&entry.standards) && entry.name.as_bytes().eq_ignore_ascii_case(&stripped))?;
    let mut color = Color::new();
    color.red = 257.0 * found.rgb[0] as f64;
    color.green = 257.0 * found.rgb[1] as f64;
    color.blue = 257.0 * found.rgb[2] as f64;
    color.alpha = QUANTUM_RANGE * found.alpha as f64;
    color.has_alpha = color.alpha != QUANTUM_RANGE;
    Some(color)
}

/// What `ParseCSSColor` found: up to five values and which.
#[derive(Default)]
struct CssValues {
    values: [f64; 5],
    present: [bool; 5],
    alpha_slash: bool,
}

impl CssValues {
    fn any(&self) -> bool {
        self.present.iter().any(|&present| present) || self.alpha_slash
    }
}

/// `ParseCSSColor`: up to five numbers (percentages of 255), commas or a
/// slash between them, a "deg" after the first.
fn parse_css_values(text: &[u8]) -> CssValues {
    let mut parsed = CssValues::default();
    if text.is_empty() {
        return parsed;
    }
    let mut position = usize::from(text[0] == b'(');
    for index in 0..5 {
        if at(text, position) == b')' || at(text, position) == 0 {
            break;
        }
        let (value, end) = interpret_locale_value(text, position);
        if end == position {
            break;
        }
        let mut intensity = value as f32 as f64;
        position = end;
        if at(text, position) == b'%' {
            intensity *= 0.01 * 255.0;
            position += 1;
        }
        parsed.values[index] = intensity;
        parsed.present[index] = true;
        if index == 0 && text.get(position..position + 3).is_some_and(|word| word.eq_ignore_ascii_case(b"deg")) {
            position += 3;
        }
        while is_space(at(text, position)) {
            position += 1;
        }
        if at(text, position) == b',' {
            position += 1;
        }
        if at(text, position) == b'/' {
            parsed.alpha_slash = true;
            position += 1;
        }
    }
    parsed
}

/// A colour function, `name(values)`, the name a colourspace option
/// (with "a" for alpha, "device-" for full-range values, or `icc-color`
/// / `color` naming one first); values that don't parse are a name.
fn parse_function(name: &[u8], open: usize) -> Option<Color> {
    // IM works in a C buffer whose old bytes outlive a shortened string.
    let mut colorspace = vec![0u8; 2 * 4096];
    colorspace[..name.len()].copy_from_slice(name);
    let mut i = open as isize;
    colorspace[i as usize] = 0;
    i -= 1;
    let mut scale = 257.0f64;
    let mut icc_color = false;
    if c_str(&colorspace).len() >= 7 && colorspace[..7].eq_ignore_ascii_case(b"device-") {
        let shortened = c_str(&colorspace[7..]).to_vec();
        colorspace[..shortened.len()].copy_from_slice(&shortened);
        colorspace[shortened.len()] = 0;
        if !name.contains(&b'%') {
            scale = QUANTUM_RANGE;
        }
        icc_color = true;
    }
    let current = c_str(&colorspace).to_vec();
    if current.eq_ignore_ascii_case(b"color") || current.eq_ignore_ascii_case(b"icc-color") {
        let rest = name.get((i + 2) as usize..).unwrap_or(&[]);
        let rest = c_str(rest);
        colorspace[..rest.len()].copy_from_slice(rest);
        colorspace[rest.len()] = 0;
        let mut j = 0;
        while colorspace[j] != 0 && colorspace[j] != b' ' && colorspace[j] != b',' {
            j += 1;
        }
        colorspace[j] = 0;
        i += j as isize - 1 + 3;
        scale = QUANTUM_RANGE;
        icc_color = true;
    }
    let length = c_str(&colorspace).len();
    colorspace[..length].make_ascii_lowercase();
    let mut color = Color::new();
    if i > 0 && colorspace.get(i as usize) == Some(&b'a') {
        colorspace[i as usize] = 0;
        color.has_alpha = true;
    }
    let option = String::from_utf8_lossy(c_str(&colorspace)).to_string();
    let mut space = Colorspace::from_option_name(&option)?;
    if !icc_color && space == Colorspace::Rgb {
        space = Colorspace::SRgb;
    }
    let values_start = (i + 1) as usize;
    let values_text: &[u8] = if values_start >= name.len() { name } else { &name[values_start..] };
    let css = parse_css_values(values_text);
    if !css.any() {
        // `rgb(red)`: the name inside, any standard's.
        let inner: Vec<u8> = values_text.iter().copied().filter(|&byte| byte != b'(' && byte != b')').collect();
        if inner.is_empty() || inner.eq_ignore_ascii_case(name) {
            return None;
        }
        return query(&inner, Compliance::All);
    }
    let [rho, mut sigma, mut xi, psi, chi] = css.values;
    let [has_rho, has_sigma, has_xi, has_psi, has_chi] = css.present;
    let quantum_plus_one_half = (QUANTUM_RANGE + 1.0) / 2.0;
    if space == Colorspace::Oklch {
        if has_rho {
            color.red = clamp_to_quantum(QUANTUM_RANGE * rho);
        }
        if has_sigma {
            color.green = clamp_to_quantum(QUANTUM_RANGE * sigma);
        }
        if has_xi {
            color.blue = clamp_to_quantum(QUANTUM_RANGE * (std::f64::consts::PI * xi / 180.0));
        }
    } else if matches!(space, Colorspace::Lab | Colorspace::Lch | Colorspace::LchAb | Colorspace::LchUv | Colorspace::Oklab) {
        color.red = clamp_to_quantum(QUANTUM_RANGE * rho / 100.0);
        if has_sigma {
            color.green = clamp_to_quantum(scale * sigma + quantum_plus_one_half);
        }
        if has_xi {
            color.blue = clamp_to_quantum(scale * xi + quantum_plus_one_half);
        }
    } else {
        if has_rho {
            color.red = clamp_to_quantum(scale * rho);
        }
        if has_sigma {
            color.green = clamp_to_quantum(scale * sigma);
        }
        if has_xi {
            color.blue = clamp_to_quantum(scale * xi);
        }
    }
    if css.alpha_slash {
        color.has_alpha = true;
    }
    color.alpha = QUANTUM_RANGE;
    if has_psi {
        if space == Colorspace::Cmyk {
            color.black = clamp_to_quantum(scale * psi);
        } else if color.has_alpha {
            color.alpha = if css.alpha_slash { clamp_to_quantum(scale * psi) } else { clamp_to_quantum(QUANTUM_RANGE * psi) };
        }
    }
    if has_chi && color.has_alpha {
        color.alpha = clamp_to_quantum(QUANTUM_RANGE * chi);
    }
    let lowered = c_str(&colorspace);
    if lowered == b"gray" || lowered == b"lineargray" {
        color.green = color.red;
        color.blue = color.red;
        if has_sigma && color.has_alpha {
            color.alpha = clamp_to_quantum(QUANTUM_RANGE * sigma);
        }
    }
    if matches!(lowered, b"hcl" | b"hsb" | b"hsl" | b"hsv" | b"hwb") {
        let percent_scale = 1.0 / 255.0;
        sigma *= percent_scale;
        xi *= percent_scale;
        let hue = ((rho % 360.0) + 360.0) % 360.0 / 360.0;
        let [red, green, blue] = hue_to_rgb(space, hue, sigma, xi).unwrap_or([0.0, 0.0, 0.0]);
        color.red = red;
        color.green = green;
        color.blue = blue;
    }
    Some(color)
}
