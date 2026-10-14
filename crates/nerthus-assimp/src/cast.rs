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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_follow_x64() {
        assert_eq!(to_unsigned(-1.0), u32::MAX);
        assert_eq!(to_unsigned(4294967297.0), 1);
        assert_eq!(to_unsigned(f64::NAN), 0);
        assert_eq!(to_int(-2_147_483_648.9), i32::MIN);
        assert_eq!(to_int(-7.9), -7);
        assert_eq!(to_int(2_147_483_648.0), i32::MIN);
        assert_eq!(to_int(f64::NAN), i32::MIN);
    }
}
