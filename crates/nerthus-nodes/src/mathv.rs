//! The vector maths (`MathV`): polynomial approximations of `atan2`,
//! `sin`, `cos`, `exp` and `log2` used instead of the C library. They are
//! not exact, so nodes that call them must use these to stay bit-exact.
//!
//! They are evaluated on `Vector<float>` lanes; every operation is
//! per-lane, so the scalar forms here give the same results.

use std::f32::consts::PI;

const ATAN2_COEFFICIENT: f32 = 0.596227;

const SINE_COEFFICIENTS: [f32; 5] = [0.00038593775, -0.0068601873, 0.075187266, -0.52403617, 2.0261195];

const EXP_LIMIT: f32 = 88.37626;
const LOG2_E: f32 = std::f32::consts::LOG2_E;
const LN_2: f32 = std::f32::consts::LN_2;
const EXP_COEFFICIENTS: [f32; 6] = [0.00019875691, 0.0013981999, 0.008333452, 0.041665796, 0.16666666, 0.5];

const LOG2_COEFFICIENTS: [f32; 6] = [-0.034436006, 0.31821337, -1.2315303, 2.5988452, -3.324199, 3.11579];

/// `Vector.Max` as SSE evaluates it: the first operand only when it is
/// strictly greater.
pub fn max(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

/// `Vector.Min` as SSE evaluates it: the first operand only when it is
/// strictly smaller.
pub fn min(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

/// `MathV.Atan2(y, x)`: an approximation with results in `[0, 2π)`.
pub fn atan2(y: f32, x: f32) -> f32 {
    let x_sign = x.to_bits() & 0x8000_0000;
    let y_sign = y.to_bits() & 0x8000_0000;
    let quadrant_bits = ((!x_sign & y_sign) >> 29) | (x_sign >> 30);
    let quadrant = quadrant_bits as f32;

    let cross = (ATAN2_COEFFICIENT * x * y).abs();
    let numerator = cross + y * y;
    let ratio = numerator / (x * x + cross + numerator);
    let signed_ratio = f32::from_bits((x_sign ^ y_sign) | ratio.to_bits());
    let angle = (PI / 2.0) * (quadrant + signed_ratio);

    let both_zero = f32::from_bits(x.to_bits() | y.to_bits()) == 0.0;
    if both_zero { 0.0 } else { angle }
}

/// The odd polynomial both `sin` and `cos` end in, for `a` in
/// `[-1, 1]` standing for half a turn.
fn half_turn_sine(a: f32) -> f32 {
    let squared = a * a;
    let [c5, c4, c3, c2, c1] = SINE_COEFFICIENTS;
    let polynomial = ((((c5 * squared + c4) * squared + c3) * squared + c2) * squared + c1) * squared + -PI;
    (a - 1.0) * (a + 1.0) * polynomial * a
}

/// `MathV.Sin`.
pub fn sin(x: f32) -> f32 {
    let mut turns = x * (1.0 / PI);
    turns = (turns - 1.0) * 0.5;
    turns -= turns.floor();
    turns = turns * 2.0 - 1.0;
    half_turn_sine(turns)
}

/// `MathV.Cos`.
pub fn cos(x: f32) -> f32 {
    let mut turns = x * (1.0 / PI);
    turns += 0.5;
    turns = (turns - 1.0) * 0.5;
    turns -= turns.floor();
    turns = turns * 2.0 - 1.0;
    half_turn_sine(turns)
}

/// `MathV.Exp`.
pub fn exp(x: f32) -> f32 {
    let mut x = max(x, -EXP_LIMIT);
    x = min(x, EXP_LIMIT);
    let mut exponent = x * LOG2_E;
    exponent += 0.5;
    exponent = exponent.floor();
    x -= exponent * LN_2;
    let squared = x * x;
    let [c0, c1, c2, c3, c4, c5] = EXP_COEFFICIENTS;
    let polynomial = (((((c0 * x + c1) * x + c2) * x + c3) * x + c4) * x + c5) * squared + x + 1.0;
    let biased_exponent = (exponent as i32).wrapping_add(127);
    let scale = f32::from_bits((biased_exponent << 23) as u32);
    polynomial * scale
}

/// `MathV.Log2`.
pub fn log2(x: f32) -> f32 {
    let bits = x.to_bits() as i32;
    let exponent = (((bits & 0x7F80_0000) as u32 >> 23) as i32 - 127) as f32;
    let mantissa = f32::from_bits((bits & 0x007F_FFFF) as u32 | 1.0f32.to_bits());
    let [c6, c5, c4, c3, c2, c1] = LOG2_COEFFICIENTS;
    let polynomial = ((((c6 * mantissa + c5) * mantissa + c4) * mantissa + c3) * mantissa + c2) * mantissa + c1;
    polynomial * (mantissa - 1.0) + exponent
}

/// `MathV.Log`.
pub fn ln(x: f32) -> f32 {
    log2(x) * LN_2
}

/// `MathV.Pow`: `exp(y · ln(x))`.
pub fn pow(x: f32, y: f32) -> f32 {
    exp(y * ln(x))
}

/// `MathV.Mix`: `a · (1 − t) + b · t`.
pub fn mix(a: f32, b: f32, t: f32) -> f32 {
    a * (1.0 - t) + b * t
}

/// The seventh-order smoothstep:
/// `35a⁴ − 84a⁵ + 70a⁶ − 20a⁷`.
pub fn smoothstep7(a: f32) -> f32 {
    let squared = a * a;
    squared * squared * (35.0 + a * (-84.0 + a * (70.0 - 20.0 * a)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approximations_are_close() {
        for step in -40..=40 {
            let angle = step as f32 * 0.1;
            assert!((sin(angle) - angle.sin()).abs() < 1e-4, "sin {angle}");
            assert!((cos(angle) - angle.cos()).abs() < 1e-4, "cos {angle}");
        }
        for step in 1..=40 {
            let value = step as f32 * 0.25;
            assert!((exp(value) / value.exp() - 1.0).abs() < 1e-5, "exp {value}");
            assert!((log2(value) - value.log2()).abs() < 1e-3, "log2 {value}");
        }
        let angle = atan2(1.0, -1.0);
        assert!((angle - 3.0 * PI / 4.0).abs() < 1e-2);
        let angle = atan2(-1.0, 1.0);
        assert!((angle - 7.0 * PI / 4.0).abs() < 1e-2);
    }
}
