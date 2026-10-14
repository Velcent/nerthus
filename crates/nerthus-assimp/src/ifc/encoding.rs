//! `StringToUTF8`: a STEP string literal's escapes as UTF-8. `\S\c` is
//! the Latin-1 character c + 128, `\X\hh` a Mac Roman one, `\X2\…\X0\`
//! UTF-16 and `\X4\…\X0\` UTF-32 (utf8-cpp's checked converters, which
//! throw on surrogates and code points past U+10FFFF).

use super::Failure;

/// Mac Roman 0x20–0xFF as Unicode.
const MAC_ROMAN: [u16; 224] = [
    0x0020, 0x0021, 0x0022, 0x0023, 0x0024, 0x0025, 0x0026, 0x0027, 0x0028, 0x0029, 0x002A, 0x002B, 0x002C, 0x002D, 0x002E, 0x002F,
    0x0030, 0x0031, 0x0032, 0x0033, 0x0034, 0x0035, 0x0036, 0x0037, 0x0038, 0x0039, 0x003A, 0x003B, 0x003C, 0x003D, 0x003E, 0x003F,
    0x0040, 0x0041, 0x0042, 0x0043, 0x0044, 0x0045, 0x0046, 0x0047, 0x0048, 0x0049, 0x004A, 0x004B, 0x004C, 0x004D, 0x004E, 0x004F,
    0x0050, 0x0051, 0x0052, 0x0053, 0x0054, 0x0055, 0x0056, 0x0057, 0x0058, 0x0059, 0x005A, 0x005B, 0x005C, 0x005D, 0x005E, 0x005F,
    0x0060, 0x0061, 0x0062, 0x0063, 0x0064, 0x0065, 0x0066, 0x0067, 0x0068, 0x0069, 0x006A, 0x006B, 0x006C, 0x006D, 0x006E, 0x006F,
    0x0070, 0x0071, 0x0072, 0x0073, 0x0074, 0x0075, 0x0076, 0x0077, 0x0078, 0x0079, 0x007A, 0x007B, 0x007C, 0x007D, 0x007E, 0x007F,
    0x00C4, 0x00C5, 0x00C7, 0x00C9, 0x00D1, 0x00D6, 0x00DC, 0x00E1, 0x00E0, 0x00E2, 0x00E4, 0x00E3, 0x00E5, 0x00E7, 0x00E9, 0x00E8,
    0x00EA, 0x00EB, 0x00ED, 0x00EC, 0x00EE, 0x00EF, 0x00F1, 0x00F3, 0x00F2, 0x00F4, 0x00F6, 0x00F5, 0x00FA, 0x00F9, 0x00FB, 0x00FC,
    0x2020, 0x00B0, 0x00A2, 0x00A3, 0x00A7, 0x2022, 0x00B6, 0x00DF, 0x00AE, 0x00A9, 0x2122, 0x00B4, 0x00A8, 0x2260, 0x00C6, 0x00D8,
    0x221E, 0x00B1, 0x2264, 0x2265, 0x00A5, 0x00B5, 0x2202, 0x2211, 0x220F, 0x03C0, 0x222B, 0x00AA, 0x00BA, 0x03A9, 0x00E6, 0x00F8,
    0x00BF, 0x00A1, 0x00AC, 0x221A, 0x0192, 0x2248, 0x2206, 0x00AB, 0x00BB, 0x2026, 0x00A0, 0x00C0, 0x00C3, 0x00D5, 0x0152, 0x0153,
    0x2013, 0x2014, 0x201C, 0x201D, 0x2018, 0x2019, 0x00F7, 0x25CA, 0x00FF, 0x0178, 0x2044, 0x20AC, 0x2039, 0x203A, 0xFB01, 0xFB02,
    0x2021, 0x00B7, 0x201A, 0x201E, 0x2030, 0x00C2, 0x00CA, 0x00C1, 0x00CB, 0x00C8, 0x00CD, 0x00CE, 0x00CF, 0x00CC, 0x00D3, 0x00D4,
    0xF8FF, 0x00D2, 0x00DA, 0x00DB, 0x00D9, 0x0131, 0x02C6, 0x02DC, 0x00AF, 0x02D8, 0x02D9, 0x02DA, 0x00B8, 0x02DD, 0x02DB, 0x02C7,
];

/// `HexDigitToDecimal`: `UINT_MAX` for a non-digit.
fn hex_digit(byte: u8) -> u32 {
    match byte {
        b'0'..=b'9' => (byte - b'0') as u32,
        b'a'..=b'f' => 10 + (byte - b'a') as u32,
        b'A'..=b'F' => 10 + (byte - b'A') as u32,
        _ => u32::MAX,
    }
}

/// `HexOctetToDecimal`: the two digits' `uint8_t` values, the first
/// shifted (non-digits are 0xFF).
fn hex_octet(text: &[u8], at: usize) -> u8 {
    let high = hex_digit(text.get(at).copied().unwrap_or(0)) as u8;
    let low = hex_digit(text.get(at + 1).copied().unwrap_or(0)) as u8;
    (high << 4).wrapping_add(low)
}

/// `utf8::append` (checked): one code point as UTF-8.
fn append_utf8(code_point: u32, out: &mut Vec<u8>) -> Result<(), Failure> {
    let Some(character) = char::from_u32(code_point) else {
        return Err(Failure::Std(format!("Invalid code point {code_point}")));
    };
    let mut buffer = [0; 4];
    let encoded = character.encode_utf8(&mut buffer);
    out.extend_from_slice(encoded.as_bytes());
    Ok(())
}

/// `utf8::utf16to8` (checked): a lead surrogate needs a trail one after
/// it, a trail one alone throws.
fn utf16_to_utf8(units: &[u16], out: &mut Vec<u8>) -> Result<(), Failure> {
    let mut index = 0;
    while index < units.len() {
        let mut code_point = units[index] as u32;
        index += 1;
        if (0xD800..=0xDBFF).contains(&code_point) {
            let Some(&trail) = units.get(index) else {
                return Err(Failure::Std(format!("Invalid UTF-16 {code_point}")));
            };
            index += 1;
            let trail = trail as u32;
            if !(0xDC00..=0xDFFF).contains(&trail) {
                return Err(Failure::Std(format!("Invalid UTF-16 {trail}")));
            }
            code_point = (code_point << 10).wrapping_add(trail).wrapping_add(0xFCA0_2400);
        } else if (0xDC00..=0xDFFF).contains(&code_point) {
            return Err(Failure::Std(format!("Invalid UTF-16 {code_point}")));
        }
        append_utf8(code_point, out)?;
    }
    Ok(())
}

/// `StringToUTF8`: the escapes decoded in place; false where the C++
/// gives up (the rest left as it is), an error where utf8-cpp throws.
pub(super) fn string_to_utf8(text: &mut Vec<u8>) -> Result<bool, Failure> {
    let mut index = 0;
    while index < text.len() {
        if text[index] == b'\\' {
            let size = text.len();
            if index + 3 < size && text[index + 1] == b'S' && text[index + 2] == b'\\' {
                Failure::assertion(text[index + 3] < 0x80, "(uint8_t)s[i+3] < 0x80")?;
                let character = text[index + 3].wrapping_add(0x80);
                text[index] = 0xC0 | ((character & 0xC0) >> 6);
                text[index + 1] = 0x80 | (character & 0x3F);
                text.drain(index + 2..index + 4);
                index += 1;
            } else if index + 4 < size && text[index + 1] == b'X' && text[index + 2] == b'\\' {
                let mac = hex_octet(text, index + 3);
                if mac < 0x20 {
                    return Ok(false);
                }
                let mut encoded = Vec::new();
                append_utf8(MAC_ROMAN[(mac - 0x20) as usize] as u32, &mut encoded)?;
                let count = encoded.len();
                text.splice(index..index + 5, encoded);
                index += count;
            } else if index + 3 < size && text[index + 1] == b'X' && text[index + 2].is_ascii_digit() {
                let width = text[index + 2];
                if width != b'2' && width != b'4' {
                    return Ok(false);
                }
                if text[index + 3] == b'\\' {
                    let start = index + 4;
                    let end = size - 3;
                    let mut close = start;
                    while close < end {
                        if text[close] == b'\\' && text[close + 1] == b'X' && text[close + 2] == b'0' && text[close + 3] == b'\\' {
                            break;
                        }
                        close += 1;
                    }
                    if close == end {
                        return Ok(false);
                    }
                    if close == start {
                        // `erase(i, 8)` stops at the end.
                        let erased = (index + 8).min(text.len());
                        text.drain(index..erased);
                        continue;
                    }
                    let digits = if width == b'2' { 4 } else { 8 };
                    if (close - start) % digits != 0 {
                        return Ok(false);
                    }
                    let mut decoded = Vec::new();
                    if width == b'2' {
                        let mut units = Vec::new();
                        for unit in (start..close).step_by(4) {
                            let high = hex_octet(text, unit) as u16;
                            let low = hex_octet(text, unit + 2) as u16;
                            units.push((high << 8) | low);
                        }
                        utf16_to_utf8(&units, &mut decoded)?;
                    } else {
                        for unit in (start..close).step_by(8) {
                            let mut code_point = 0u32;
                            for byte in 0..4 {
                                code_point = (code_point << 8) | hex_octet(text, unit + 2 * byte) as u32;
                            }
                            append_utf8(code_point, &mut decoded)?;
                        }
                    }
                    let count = decoded.len();
                    text.splice(index..close + 4, decoded);
                    index += count;
                    continue;
                }
            }
        }
        index += 1;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(text: &str) -> (Vec<u8>, bool) {
        let mut bytes = text.as_bytes().to_vec();
        let complete = string_to_utf8(&mut bytes).unwrap();
        (bytes, complete)
    }

    #[test]
    fn latin_and_mac_escapes_decode() {
        assert_eq!(decode("a\\S\\Db").0, "aÄb".as_bytes());
        // The character after a Mac Roman escape is passed over.
        assert_eq!(decode("\\X\\8A\\S\\D").0, "ä\\S\\D".as_bytes());
    }

    #[test]
    fn unicode_runs_decode() {
        assert_eq!(decode("x\\X2\\00E4\\X0\\y").0, "xäy".as_bytes());
        assert_eq!(decode("\\X4\\0001F600\\X0\\").0, "😀".as_bytes());
        assert_eq!(decode("a\\X2\\\\X0\\b").0, b"ab");
    }

    #[test]
    fn broken_escapes_stop_or_throw() {
        assert!(!decode("\\X2\\00E\\X0\\").1);
        assert!(!decode("\\X3\\").1);
        let mut surrogate = b"\\X2\\D800\\X0\\".to_vec();
        assert!(string_to_utf8(&mut surrogate).is_err());
    }
}
