//! C++ numeric conversions as x64 compilers emit them.

/// `(unsigned int)` of a float or double: a 64-bit truncation
/// (`cvttsd2si` to a 64-bit register), its low 32 bits. Out of range or
/// NaN gives the "integer indefinite" `i64::MIN`, whose low bits are 0.
pub(crate) fn to_unsigned(value: f64) -> u32 {
    if value.is_nan() || value.abs() >= 9.223_372_036_854_776e18 {
        return 0;
    }
    value as i64 as u32
}

/// `(int)` of a float or double: a 32-bit truncation (`cvttsd2si`), whose
/// out of range or NaN result is `INT_MIN`.
pub(crate) fn to_int(value: f64) -> i32 {
    if value.is_nan() || value <= -2_147_483_649.0 || value >= 2_147_483_648.0 {
        return i32::MIN;
    }
    value as i32
}

/// `(long long)` of a double: a 64-bit truncation (`cvttsd2si`), whose
/// out of range or NaN result is `LLONG_MIN`.
pub(crate) fn to_i64(value: f64) -> i64 {
    const TWO_TO_63: f64 = 9.223_372_036_854_776e18;
    if value.is_nan() || !(-TWO_TO_63..TWO_TO_63).contains(&value) {
        return i64::MIN;
    }
    value as i64
}

/// `(unsigned long long)` (and `size_t`) of a double as MSVC emits it on
/// x64: below 2^63 (and NaN) a 64-bit truncation, negatives wrapping;
/// below 2^64 the truncation of `value - 2^63` with the top bit added;
/// past that (and NaN) the "integer indefinite" 0x8000000000000000.
pub(crate) fn to_u64(value: f64) -> u64 {
    const TWO_TO_63: f64 = 9.223_372_036_854_776e18;
    const INDEFINITE: u64 = 1 << 63;
    let truncate = |x: f64| {
        if x.is_nan() || !(-TWO_TO_63..TWO_TO_63).contains(&x) {
            return INDEFINITE;
        }
        x as i64 as u64
    };
    if value.is_nan() || value < TWO_TO_63 {
        return truncate(value);
    }
    let reduced = value - TWO_TO_63;
    if reduced >= TWO_TO_63 {
        return INDEFINITE;
    }
    truncate(reduced).wrapping_add(INDEFINITE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsigned_64_conversions_follow_msvc() {
        assert_eq!(to_u64(-1.0), u64::MAX);
        assert_eq!(to_u64(1.5e19), 15_000_000_000_000_000_000);
        assert_eq!(to_u64(f64::NAN), 1 << 63);
        assert_eq!(to_u64(2.0e19), 1 << 63);
        assert_eq!(to_u64(f64::INFINITY), 1 << 63);
        assert_eq!(to_u64(f64::NEG_INFINITY), 1 << 63);
    }

    #[test]
    fn conversions_follow_x64() {
        assert_eq!(to_unsigned(-1.0), u32::MAX);
        assert_eq!(to_unsigned(4294967297.0), 1);
        assert_eq!(to_unsigned(f64::NAN), 0);
        assert_eq!(to_int(-2_147_483_648.9), i32::MIN);
        assert_eq!(to_int(-7.9), -7);
        assert_eq!(to_int(2_147_483_648.0), i32::MIN);
        assert_eq!(to_int(f64::NAN), i32::MIN);
        assert_eq!(to_i64(-7.9), -7);
        assert_eq!(to_i64(-9.223_372_036_854_776e18), i64::MIN);
        assert_eq!(to_i64(9.223_372_036_854_776e18), i64::MIN);
        assert_eq!(to_i64(f64::NAN), i64::MIN);
    }
}
