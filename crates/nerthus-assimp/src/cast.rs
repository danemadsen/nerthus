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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_follow_x64() {
        assert_eq!(to_unsigned(-1.0), u32::MAX);
        assert_eq!(to_unsigned(4294967297.0), 1);
        assert_eq!(to_unsigned(f64::NAN), 0);
    }
}
