//! The C library's text parsing as ImageMagick's text readers lean on it
//! (MSVC's: `isspace`, `strtod`, `strtoul`), over bytes, and IM's own
//! (`GetNextToken`, the XPM and SIXEL readers' line joining).

use super::blob::Blob;

/// C's `isspace` in the "C" locale.
pub(crate) fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

fn at(bytes: &[u8], index: usize) -> u8 {
    bytes.get(index).copied().unwrap_or(0)
}

fn starts_with_ignore_case(bytes: &[u8], index: usize, word: &[u8]) -> bool {
    bytes.get(index..index + word.len()).is_some_and(|slice| slice.eq_ignore_ascii_case(word))
}

/// `strtod`: after blanks, the longest prefix that reads as a number (a
/// decimal or hexadecimal one, "inf", "infinity" or "nan", maybe signed),
/// correctly rounded, and where it ends; 0 and `start` where there is none.
pub(crate) fn strtod(bytes: &[u8], start: usize) -> (f64, usize) {
    let mut index = start;
    while is_space(at(bytes, index)) {
        index += 1;
    }
    let negative = at(bytes, index) == b'-';
    if matches!(at(bytes, index), b'+' | b'-') {
        index += 1;
    }
    let (magnitude, end) = if starts_with_ignore_case(bytes, index, b"infinity") {
        (f64::INFINITY, index + 8)
    } else if starts_with_ignore_case(bytes, index, b"inf") {
        (f64::INFINITY, index + 3)
    } else if starts_with_ignore_case(bytes, index, b"nan") {
        let mut end = index + 3;
        if at(bytes, end) == b'(' {
            let mut close = end + 1;
            while at(bytes, close).is_ascii_alphanumeric() || at(bytes, close) == b'_' {
                close += 1;
            }
            if at(bytes, close) == b')' {
                end = close + 1;
            }
        }
        (f64::NAN, end)
    } else if at(bytes, index) == b'0' && (at(bytes, index + 1) | 0x20) == b'x' && hex_mantissa_starts(bytes, index + 2) {
        hex_float(bytes, index + 2)
    } else {
        match decimal(bytes, index) {
            Some(parsed) => parsed,
            None => return (0.0, start),
        }
    };
    (if negative { -magnitude } else { magnitude }, end)
}

fn hex_mantissa_starts(bytes: &[u8], index: usize) -> bool {
    let first = at(bytes, index);
    first.is_ascii_hexdigit() || (first == b'.' && at(bytes, index + 1).is_ascii_hexdigit())
}

/// A decimal number's magnitude and end: digits, '.', digits (one digit at
/// least), then an exponent when it has digits.
fn decimal(bytes: &[u8], start: usize) -> Option<(f64, usize)> {
    let mut index = start;
    let mut digits = 0;
    while at(bytes, index).is_ascii_digit() {
        index += 1;
        digits += 1;
    }
    if at(bytes, index) == b'.' {
        index += 1;
        while at(bytes, index).is_ascii_digit() {
            index += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if (at(bytes, index) | 0x20) == b'e' {
        let mut exponent = index + 1;
        if matches!(at(bytes, exponent), b'+' | b'-') {
            exponent += 1;
        }
        if at(bytes, exponent).is_ascii_digit() {
            while at(bytes, exponent).is_ascii_digit() {
                exponent += 1;
            }
            index = exponent;
        }
    }
    let text = std::str::from_utf8(&bytes[start..index]).ok()?;
    let normalised = if text.starts_with('.') { format!("0{text}") } else { text.to_string() };
    Some((normalised.parse::<f64>().unwrap_or(0.0), index))
}

/// A hexadecimal float ("0x" passed): hex digits, '.', hex digits, then a
/// binary exponent ("p") when it has digits.
fn hex_float(bytes: &[u8], start: usize) -> (f64, usize) {
    let mut index = start;
    let mut mantissa = 0.0f64;
    let mut exponent = 0i64;
    let mut seen_point = false;
    loop {
        let byte = at(bytes, index);
        if byte == b'.' && !seen_point {
            seen_point = true;
            index += 1;
            continue;
        }
        let Some(digit) = (byte as char).to_digit(16) else {
            break;
        };
        mantissa = mantissa * 16.0 + digit as f64;
        if seen_point {
            exponent -= 4;
        }
        index += 1;
    }
    if (at(bytes, index) | 0x20) == b'p' {
        let mut position = index + 1;
        let negative = at(bytes, position) == b'-';
        if matches!(at(bytes, position), b'+' | b'-') {
            position += 1;
        }
        if at(bytes, position).is_ascii_digit() {
            let mut value = 0i64;
            while at(bytes, position).is_ascii_digit() {
                value = (value * 10 + (at(bytes, position) - b'0') as i64).min(1 << 20);
                position += 1;
            }
            exponent += if negative { -value } else { value };
            index = position;
        }
    }
    (mantissa * 2f64.powi(exponent.clamp(-2200, 2200) as i32), index)
}

/// `strtoul` in base 16 into a Windows (32-bit) `unsigned long`, which
/// saturates; "0x" may lead.
pub(crate) fn strtoul_hex(bytes: &[u8], start: usize) -> (u64, usize) {
    let mut index = start;
    while is_space(at(bytes, index)) {
        index += 1;
    }
    let negative = at(bytes, index) == b'-';
    if matches!(at(bytes, index), b'+' | b'-') {
        index += 1;
    }
    if at(bytes, index) == b'0' && (at(bytes, index + 1) | 0x20) == b'x' && at(bytes, index + 2).is_ascii_hexdigit() {
        index += 2;
    }
    let digits_start = index;
    let mut value = 0u64;
    let mut overflow = false;
    while let Some(digit) = (at(bytes, index) as char).to_digit(16) {
        value = value * 16 + digit as u64;
        if value > u32::MAX as u64 {
            overflow = true;
            value = u32::MAX as u64;
        }
        index += 1;
    }
    if index == digits_start {
        return (0, start);
    }
    if overflow {
        return (u32::MAX as u64, index);
    }
    let value = if negative { (value as u32).wrapping_neg() as u64 } else { value };
    (value, index)
}

/// `InterpretLocaleValue` (`StringToDouble`): hexadecimal integers after
/// "0x", else `strtod`.
pub(crate) fn interpret_locale_value(bytes: &[u8], start: usize) -> (f64, usize) {
    if at(bytes, start) == b'0' && (at(bytes, start + 1) | 0x20) == b'x' {
        let (value, end) = strtoul_hex(bytes, start);
        return (value as f64, end);
    }
    strtod(bytes, start)
}

/// `GetNextToken` (`token.c`, a Windows build: `\` is the directory
/// separator): the next token from `start` (a quoted string, a number, a
/// name with any parenthesised part, or one other character) and where
/// the text after it, blanks skipped, starts.
pub(crate) fn next_token(bytes: &[u8], start: usize, extent: usize) -> (Vec<u8>, usize) {
    let mut token = Vec::new();
    let mut p = start;
    while is_space(at(bytes, p)) && at(bytes, p) != 0 {
        p += 1;
    }
    let push = |token: &mut Vec<u8>, byte: u8| {
        if token.len() < extent - 1 {
            token.push(byte);
        }
    };
    match at(bytes, p) {
        0 => {}
        first @ (b'"' | b'\'' | b'`' | b'{') => {
            let escape = match first {
                b'"' => b'"',
                b'{' => b'}',
                _ => b'\'',
            };
            p += 1;
            while at(bytes, p) != 0 {
                if at(bytes, p) == b'\\' && (at(bytes, p + 1) == escape || at(bytes, p + 1) == b'\\') {
                    p += 1;
                } else if at(bytes, p) == escape {
                    p += 1;
                    break;
                }
                push(&mut token, at(bytes, p));
                if p - start >= extent - 1 {
                    break;
                }
                p += 1;
            }
        }
        b'/' => {
            push(&mut token, b'/');
            p += 1;
            if matches!(at(bytes, p), b'>' | b'/') {
                push(&mut token, at(bytes, p));
                p += 1;
            }
        }
        _ => 'default: {
            let (_, q) = interpret_locale_value(bytes, p);
            if p != q && at(bytes, p) != b',' {
                while p < q && at(bytes, p) != b',' {
                    push(&mut token, at(bytes, p));
                    if p - start >= extent - 1 {
                        break;
                    }
                    p += 1;
                }
                if at(bytes, p) == b'%' {
                    push(&mut token, b'%');
                    p += 1;
                }
                break 'default;
            }
            let here = at(bytes, p);
            if here != 0 && !here.is_ascii_alphabetic() && here != b'\\' && here != b'#' && here != b'<' {
                push(&mut token, here);
                p += 1;
                break 'default;
            }
            while at(bytes, p) != 0 {
                let byte = at(bytes, p);
                let previous = if p > 0 { at(bytes, p - 1) } else { 0 };
                if (is_space(byte) || matches!(byte, b'=' | b',' | b':' | b';')) && previous != b'\\' {
                    break;
                }
                if !token.is_empty() && byte == b'<' {
                    break;
                }
                push(&mut token, byte);
                if byte == b'>' {
                    break;
                }
                if byte == b'(' {
                    p += 1;
                    while at(bytes, p) != 0 {
                        push(&mut token, at(bytes, p));
                        if at(bytes, p) == b')' && at(bytes, p - 1) != b'\\' {
                            break;
                        }
                        if p - start >= extent - 1 {
                            break;
                        }
                        p += 1;
                    }
                    if at(bytes, p) == 0 {
                        break;
                    }
                }
                if p - start >= extent - 1 {
                    break;
                }
                p += 1;
            }
        }
    }
    if token.len() >= 5 && token[..5].eq_ignore_ascii_case(b"url(#")
        && let Some(close) = token.iter().rposition(|&byte| byte == b')') {
            token = token[5..close].to_vec();
        }
    while is_space(at(bytes, p)) {
        p += 1;
    }
    (token, p)
}

/// The file's lines run together, as the XPM and SIXEL readers gather them
/// (`ReadBlobString` drops each newline): a `#` line at the start is
/// dropped, and a line starting `};` ends it.
pub(crate) fn joined_lines(blob: &mut Blob) -> Vec<u8> {
    let mut buffer = Vec::new();
    while let Some(line) = blob.string() {
        if line.first() == Some(&b'#') && (buffer.is_empty() || buffer.last() == Some(&b'\n')) {
            continue;
        }
        let ends = line.starts_with(b"};");
        buffer.extend_from_slice(&line);
        if ends {
            break;
        }
    }
    buffer
}
