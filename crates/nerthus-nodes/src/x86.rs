//! Emulation of x86 approximation instructions that the native code
//! (compiled by MSVC and ISPC for AVX2) relies on. Their
//! results are not correctly rounded and differ between vendors; these
//! follow Intel, as Rosetta 2 does.

use crate::x86_tables::{RCP_MANTISSAS, RSQRT_MANTISSAS};

/// `RCPSS`/`RCPPS`: a 12-bit approximation of `1 / x`.
///
/// The mantissa comes from a table indexed by the top 11 mantissa bits of
/// `x`; zero and denormal inputs give infinity, results too small to be
/// normal flush to zero.
pub fn rcp(x: f32) -> f32 {
    let bits = x.to_bits();
    let sign = bits & 0x8000_0000;
    let exponent = (bits >> 23) & 0xFF;
    let mantissa = bits & 0x007F_FFFF;
    if exponent == 0 {
        return f32::from_bits(sign | 0x7F80_0000);
    }
    if exponent == 0xFF {
        if mantissa != 0 {
            return f32::from_bits(bits | 0x0040_0000);
        }
        return f32::from_bits(sign);
    }

    let result_exponent = 253 - exponent as i32;
    if result_exponent <= 0 {
        return f32::from_bits(sign);
    }
    let index = (mantissa >> 12) as usize;
    let result_mantissa = (RCP_MANTISSAS[index] as u32) << 11;
    f32::from_bits(sign | ((result_exponent as u32) << 23) | result_mantissa)
}

/// `RSQRTSS`/`RSQRTPS`: a 12-bit approximation of `1 / sqrt(x)`.
///
/// The mantissa comes from a table indexed by the exponent's parity and the
/// top 11 mantissa bits of `x`; zero and denormal inputs give infinity of
/// their sign, negative inputs the default NaN and infinity zero.
pub fn rsqrt(x: f32) -> f32 {
    let bits = x.to_bits();
    let sign = bits & 0x8000_0000;
    let exponent = (bits >> 23) & 0xFF;
    let mantissa = bits & 0x007F_FFFF;
    if exponent == 0 {
        return f32::from_bits(sign | 0x7F80_0000);
    }
    if exponent == 0xFF && mantissa != 0 {
        return f32::from_bits(bits | 0x0040_0000);
    }
    if sign != 0 {
        return f32::from_bits(0xFFC0_0000);
    }
    if exponent == 0xFF {
        return 0.0;
    }

    // Odd biased exponents are even powers of two: inputs in [1, 2) after
    // scaling, the first half of the table; even ones are in [2, 4).
    let parity = (exponent & 1) ^ 1;
    let index = (parity << 11 | mantissa >> 12) as usize;
    let result_exponent = (380 - exponent) >> 1;
    let result_mantissa = (RSQRT_MANTISSAS[index] as u32) << 11;
    f32::from_bits((result_exponent << 23) | result_mantissa)
}

/// ISPC's `rcp()` on AVX2: [`rcp`] refined by one Newton-Raphson step,
/// `r · (2 − r·x)` with the inner part fused.
pub fn rcp_refined(x: f32) -> f32 {
    let estimate = rcp(x);
    let correction = (-estimate).mul_add(x, 2.0);
    estimate * correction
}

/// x86's default NaN (`0xFFC00000`): what SSE gives for an invalid
/// operation. ARM gives `0x7FC00000`.
pub const DEFAULT_NAN: f32 = f32::from_bits(0xFFC0_0000);

/// `value` as a quiet NaN (sign and payload kept).
fn quiet(value: f32) -> f32 {
    f32::from_bits(value.to_bits() | 0x0040_0000)
}

/// The NaN rules of scalar SSE/AVX arithmetic: a NaN source passes
/// through quieted (the first source's when both are NaN), and a NaN the
/// operation creates is [`DEFAULT_NAN`]. LLVM may swap the operands of
/// `+` and `*`, so code that must keep x86's NaN signs uses these.
fn nan_rules(result: f32, first: f32, second: f32) -> f32 {
    if !result.is_nan() {
        result
    } else if first.is_nan() {
        quiet(first)
    } else if second.is_nan() {
        quiet(second)
    } else {
        DEFAULT_NAN
    }
}

/// `ADDSS first, second`.
pub fn addss(first: f32, second: f32) -> f32 {
    nan_rules(first + second, first, second)
}

/// `SUBSS first, second`: `first − second`.
pub fn subss(first: f32, second: f32) -> f32 {
    nan_rules(first - second, first, second)
}

/// `MULSS first, second`.
pub fn mulss(first: f32, second: f32) -> f32 {
    nan_rules(first * second, first, second)
}

/// `DIVSS first, second`: `first / second`.
pub fn divss(first: f32, second: f32) -> f32 {
    nan_rules(first / second, first, second)
}

/// `SQRTSS`: a NaN passes through, negative numbers give the default NaN.
pub fn sqrtss(x: f32) -> f32 {
    nan_rules(x.sqrt(), x, x)
}

/// A one-input library function (`acosf`, …) as x86 libm behaves: a NaN
/// input comes back with its sign, a NaN it creates is the default NaN.
pub fn libm_nan(result: f32, input: f32) -> f32 {
    nan_rules(result, input, input)
}

/// `MAXSS`: `a` only when it is strictly greater (so a NaN in either
/// operand gives `b`).
pub fn maxss(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

/// `MINSS`: `a` only when it is strictly smaller (so a NaN in either
/// operand gives `b`).
pub fn minss(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

/// `CVTTSS2SI`: truncation towards zero; NaN and values outside the `i32`
/// range give `i32::MIN` (Rust's `as` saturates instead).
pub fn cvttss2si(x: f32) -> i32 {
    let in_range = (-2147483648.0..2147483648.0).contains(&x);
    if in_range { x as i32 } else { i32::MIN }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_keeps_x86_nan_rules() {
        let negative = f32::from_bits(0xFFC0_0000);
        let positive = f32::from_bits(0x7FC0_0000);
        // The first source's NaN wins a tie.
        assert_eq!(mulss(positive, negative).to_bits(), 0x7FC0_0000);
        assert_eq!(mulss(negative, positive).to_bits(), 0xFFC0_0000);
        assert_eq!(addss(1.0, positive).to_bits(), 0x7FC0_0000);
        // Invalid operations give the default NaN.
        assert_eq!(mulss(0.0, f32::INFINITY).to_bits(), 0xFFC0_0000);
        assert_eq!(subss(f32::INFINITY, f32::INFINITY).to_bits(), 0xFFC0_0000);
        assert_eq!(divss(0.0, 0.0).to_bits(), 0xFFC0_0000);
        assert_eq!(sqrtss(-1.0).to_bits(), 0xFFC0_0000);
        // Signalling NaNs come back quiet; numbers are untouched.
        assert_eq!(addss(f32::from_bits(0x7F80_0001), 1.0).to_bits(), 0x7FC0_0001);
        assert_eq!(libm_nan(f32::NAN, f32::from_bits(0xFFC0_1234)).to_bits(), 0xFFC0_1234);
        assert_eq!(addss(1.0, 2.0), 3.0);
        assert_eq!(divss(1.0, 4.0), 0.25);
    }

    #[test]
    fn rcp_matches_known_values() {
        assert_eq!(rcp(1.0).to_bits(), 0x3F7F_F000);
        assert_eq!(rcp(3.0).to_bits(), 0x3EAA_A000);
        assert_eq!(rcp(-1.0).to_bits(), 0xBF7F_F000);
        assert_eq!(rcp(f32::from_bits(0x3FFF_FFFF)).to_bits(), 0x3F00_0800);
        assert_eq!(rcp(f32::from_bits(0x7E7F_FFFF)).to_bits(), 0x0080_0800);
        assert_eq!(rcp(f32::from_bits(0x7E80_0000)).to_bits(), 0);
        assert_eq!(rcp(0.0).to_bits(), 0x7F80_0000);
        assert_eq!(rcp(-0.0).to_bits(), 0xFF80_0000);
        assert_eq!(rcp(f32::INFINITY).to_bits(), 0);
        assert_eq!(rcp(f32::from_bits(0x7FA0_0000)).to_bits(), 0x7FE0_0000);
    }

    #[test]
    fn rsqrt_matches_known_values() {
        let cases: [(u32, u32); 14] = [
            (0x0000_0000, 0x7F80_0000),
            (0x8000_0000, 0xFF80_0000),
            (0x0000_0001, 0x7F80_0000),
            (0x007F_FFFF, 0x7F80_0000),
            (0x0080_0000, 0x5EFF_F000),
            (0x7F7F_FFFF, 0x1F80_0800),
            (0x7F80_0000, 0x0000_0000),
            (0xFF80_0000, 0xFFC0_0000),
            (0xBF80_0000, 0xFFC0_0000),
            (0x7FA0_0000, 0x7FE0_0000),
            (0x3F80_0000, 0x3F7F_F000),
            (0x4040_0000, 0x3F13_C800),
            (0x4080_0000, 0x3EFF_F000),
            (0x3E80_0000, 0x3FFF_F000),
        ];
        for (input, expected) in cases {
            assert_eq!(rsqrt(f32::from_bits(input)).to_bits(), expected, "rsqrt({input:#010x})");
        }
    }
}
