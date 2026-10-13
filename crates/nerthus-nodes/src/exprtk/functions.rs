//! ExprTk's numeric functions for `float` (`exprtk::details::numeric`) as
//! an MSVC build compiles them: `__cplusplus` is `199711L` there, so
//! ExprTk's own `acosh`/`asinh`/`atanh`/`expm1`/`log1p`/`trunc` replace the
//! C++11 library calls.

use crate::exprtk::node::{FunctionOp, Node, VarArgOp};

/// `numeric::constant::log2` as a float.
pub const LOG2: f32 = std::f64::consts::LN_2 as f32;
/// `numeric::constant::_180_pi`.
pub const _180_PI: f32 = (180.0 / std::f64::consts::PI) as f32;
/// `numeric::constant::pi_180`.
pub const PI_180: f32 = (std::f64::consts::PI / 180.0) as f32;
/// `T(10.0 / 9.0)`.
pub const TEN_NINTHS: f32 = (10.0f64 / 9.0) as f32;
/// `T(9.0 / 10.0)`.
pub const NINE_TENTHS: f32 = (9.0f64 / 10.0) as f32;
/// `numeric::constant::sqrt2`.
const SQRT2: f32 = std::f64::consts::SQRT_2 as f32;
/// `epsilon_type<float>::value()`.
const EPSILON: f32 = 0.000_001;

/// x86's default NaN (`0xFFC00000`): what SSE and the x86 libm give for an
/// invalid operation. ARM gives `0x7FC00000`.
pub const X86_NAN: f32 = f32::from_bits(0xFFC0_0000);

/// `v` as a quiet NaN (sign kept).
fn quiet(value: f32) -> f32 {
    f32::from_bits(value.to_bits() | 0x0040_0000)
}

/// The result of a one-input x86 operation: a NaN input passes through
/// with its sign (x86's libm keeps it, ARM's may not), a NaN the
/// operation created is x86's.
#[inline]
pub fn nan1(result: f32, a: f32) -> f32 {
    if !result.is_nan() {
        result
    } else if a.is_nan() {
        quiet(a)
    } else {
        X86_NAN
    }
}

/// The result of a two-input x86 operation: the first NaN input passes
/// through, a NaN the operation created is x86's.
#[inline]
pub fn nan2(result: f32, a: f32, b: f32) -> f32 {
    if !result.is_nan() {
        result
    } else if a.is_nan() {
        quiet(a)
    } else if b.is_nan() {
        quiet(b)
    } else {
        X86_NAN
    }
}

#[inline]
pub fn add(a: f32, b: f32) -> f32 {
    nan2(a + b, a, b)
}

/// `a + b` where MSVC's code puts `b` first: its NaN wins a tie.
#[inline]
pub fn add_r(a: f32, b: f32) -> f32 {
    nan2(a + b, b, a)
}

/// `a * b` where MSVC's code puts `b` first.
#[inline]
pub fn mul_r(a: f32, b: f32) -> f32 {
    nan2(a * b, b, a)
}

#[inline]
pub fn sub(a: f32, b: f32) -> f32 {
    nan2(a - b, a, b)
}

#[inline]
pub fn mul(a: f32, b: f32) -> f32 {
    nan2(a * b, a, b)
}

#[inline]
pub fn div(a: f32, b: f32) -> f32 {
    nan2(a / b, a, b)
}

/// `static_cast<int>(v)` on x64 (`cvttss2si`): out of range and NaN give
/// `INT_MIN`.
pub fn truncate_to_i32(value: f32) -> i32 {
    let in_range = value > -2_147_483_904.0 && value < 2_147_483_648.0;
    if in_range {
        value as i32
    } else {
        i32::MIN
    }
}

/// `static_cast<long long>(v)` on x64: out of range and NaN give
/// `LLONG_MIN`.
fn truncate_to_i64(value: f32) -> i64 {
    let in_range = (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&value);
    if in_range {
        value as i64
    } else {
        i64::MIN
    }
}

/// `trunc_impl`: `T(static_cast<long long>(v))`.
pub fn trunc(value: f32) -> f32 {
    truncate_to_i64(value) as f32
}

/// `abs_impl`: `(v < 0) ? -v : v`, which MSVC (`/fp:fast`) compiles to
/// clearing the sign bit, so `abs(-0)` is `+0` and NaNs lose their sign.
pub fn abs(value: f32) -> f32 {
    f32::from_bits(value.to_bits() & 0x7FFF_FFFF)
}

/// `round_impl`: halves away from zero.
pub fn round(value: f32) -> f32 {
    if value < 0.0 {
        (value - 0.5).ceil()
    } else {
        (value + 0.5).floor()
    }
}

pub fn sgn(value: f32) -> f32 {
    if value > 0.0 {
        1.0
    } else if value < 0.0 {
        -1.0
    } else {
        0.0
    }
}

pub fn sinc(value: f32) -> f32 {
    if abs(value) >= f32::EPSILON {
        div(nan1(value.sin(), value), value)
    } else {
        1.0
    }
}

/// `expm1_impl` (pre-C++11 branch).
pub fn expm1(value: f32) -> f32 {
    if abs(value) < 0.000_01 {
        add(value, mul(mul(0.5, value), value))
    } else {
        sub(value.exp(), 1.0)
    }
}

/// `log1p_impl` (pre-C++11 branch).
pub fn log1p(value: f32) -> f32 {
    if value > -1.0 {
        if abs(value) > 0.0001 {
            let sum = add(1.0, value);
            nan1(sum.ln(), sum)
        } else {
            mul(add(mul(-0.5, value), 1.0), value)
        }
    } else {
        f32::NAN
    }
}

/// `std::erf(float)` (`erff`).
pub fn erf(value: f32) -> f32 {
    unsafe extern "C" {
        fn erff(x: f32) -> f32;
    }
    unsafe { erff(value) }
}

/// `std::erfc(float)` (`erfcf`).
pub fn erfc(value: f32) -> f32 {
    unsafe extern "C" {
        fn erfcf(x: f32) -> f32;
    }
    unsafe { erfcf(value) }
}

pub fn ncdf(value: f32) -> f32 {
    let scaled = -div(value, SQRT2);
    mul(0.5, nan1(erfc(scaled), scaled))
}

/// `std::max<T>(a, b)`: `b` only when `a` is smaller.
fn max2(a: f32, b: f32) -> f32 {
    if a < b {
        b
    } else {
        a
    }
}

/// `std::min<T>(a, b)`: `b` only when it is smaller.
fn min2(a: f32, b: f32) -> f32 {
    if b < a {
        b
    } else {
        a
    }
}

/// `equal_impl` with ExprTk's relative epsilon.
pub fn equal(left: f32, right: f32) -> f32 {
    let scale = max2(1.0, max2(abs(left), abs(right)));
    if abs(left - right) <= scale * EPSILON {
        1.0
    } else {
        0.0
    }
}

/// `nequal_impl` (not `1 - equal`: NaN is unequal to nothing here).
pub fn not_equal(left: f32, right: f32) -> f32 {
    let scale = max2(1.0, max2(abs(left), abs(right)));
    if abs(left - right) > scale * EPSILON {
        1.0
    } else {
        0.0
    }
}

/// `fast_exp<T, N>`: ExprTk's whole powers by multiplication.
pub fn fast_exp(value: f32, power: u32) -> f32 {
    match power {
        0 => 1.0,
        1 => value,
        2 => mul(value, value),
        3 => mul(mul(value, value), value),
        4 => {
            let square = mul(value, value);
            mul(square, square)
        }
        5 => mul(fast_exp(value, 4), value),
        6 => {
            let cube = fast_exp(value, 3);
            mul(cube, cube)
        }
        7 => mul(fast_exp(value, 6), value),
        8 => {
            let fourth = fast_exp(value, 4);
            mul(fourth, fourth)
        }
        9 => mul(fast_exp(value, 8), value),
        10 => {
            let fifth = fast_exp(value, 5);
            mul(fifth, fifth)
        }
        _ => {
            let mut remaining = power;
            let mut result = 1.0f32;
            let mut base = value;
            while remaining != 0 {
                if remaining % 2 == 1 {
                    result = mul(result, base);
                    remaining -= 1;
                }
                base = mul(base, base);
                remaining /= 2;
            }
            result
        }
    }
}

/// `roundn_impl`.
pub fn roundn(value: f32, digits: f32) -> f32 {
    const POW10: [f64; 17] = [1.0, 1.0e1, 1.0e2, 1.0e3, 1.0e4, 1.0e5, 1.0e6, 1.0e7, 1.0e8, 1.0e9, 1.0e10, 1.0e11, 1.0e12, 1.0e13, 1.0e14, 1.0e15, 1.0e16];
    let wanted = truncate_to_i32(digits.floor());
    let index = wanted.min(POW10.len() as i32 - 1).max(0) as usize;
    let p10 = POW10[index] as f32;
    if value < 0.0 {
        div(sub(mul(value, p10), 0.5).ceil(), p10)
    } else {
        div(add(mul(value, p10), 0.5).floor(), p10)
    }
}

/// `static_cast<std::size_t>(v)` for a non-negative (or NaN) float in an
/// MSVC build: NaN and values past `u64` give 2^63.
fn truncate_to_u64(value: f32) -> u64 {
    if value.is_nan() || value >= 18_446_744_073_709_551_616.0 {
        return 1 << 63;
    }
    value as u64
}

/// `std::fmod` (`fmodf`) as x86 gives it: a NaN input passes through, an
/// invalid operation gives a positive NaN.
pub fn fmod(a: f32, b: f32) -> f32 {
    let result = a % b;
    if !result.is_nan() {
        result
    } else if a.is_nan() {
        quiet(a)
    } else if b.is_nan() {
        quiet(b)
    } else {
        f32::NAN
    }
}

/// `root_impl` as this ExprTk snapshot has it (before 2024): the degree is
/// truncated to a whole number, negative degrees give NaN.
pub fn root(value: f32, degree: f32) -> f32 {
    if degree < 0.0 {
        return f32::NAN;
    }
    let whole = truncate_to_u64(degree);
    if value < 0.0 && whole.is_multiple_of(2) {
        return f32::NAN;
    }
    let exponent = div(1.0, whole as f32);
    nan2(value.powf(exponent), value, exponent)
}

/// The three-argument functions (`trinary_node::value`).
pub fn function(op: FunctionOp, lower: f32, value: f32, upper: f32) -> f32 {
    match op {
        FunctionOp::Inrange => {
            if value < lower || value > upper {
                0.0
            } else {
                1.0
            }
        }
        FunctionOp::Clamp => {
            if value < lower {
                lower
            } else if value > upper {
                upper
            } else {
                value
            }
        }
        FunctionOp::Iclamp => {
            if value <= lower || value >= upper {
                value
            } else if 2.0 * value <= upper + lower {
                lower
            } else {
                upper
            }
        }
    }
}

/// The variable-argument functions (`vararg_*_op::process`), with
/// ExprTk's groupings; `mand`/`mor` stop at the first deciding argument.
pub fn vararg(op: VarArgOp, arguments: &[Node], variables: &mut [f32]) -> f32 {
    match op {
        VarArgOp::Mand => {
            for argument in arguments {
                if argument.evaluate(variables) == 0.0 {
                    return 0.0;
                }
            }
            1.0
        }
        VarArgOp::Mor => {
            for argument in arguments {
                if argument.evaluate(variables) != 0.0 {
                    return 1.0;
                }
            }
            0.0
        }
        _ => {
            let mut values = Vec::with_capacity(arguments.len());
            for argument in arguments {
                values.push(argument.evaluate(variables));
            }
            vararg_values(op, &values)
        }
    }
}

/// `vararg_*_op::process` on evaluated arguments.
pub fn vararg_values(op: VarArgOp, values: &[f32]) -> f32 {
    let count = values.len();
    match op {
        VarArgOp::Sum | VarArgOp::Avg => {
            if count == 0 {
                return 0.0;
            }
            // Up to five arguments are added from the first, more from 0.
            let mut total = if count > 5 { add(0.0, values[0]) } else { values[0] };
            for &value in &values[1..] {
                // Two arguments: the second one's NaN wins.
                total = if count == 2 && total.is_nan() && value.is_nan() { nan1(value, value) } else { add(total, value) };
            }
            if op == VarArgOp::Avg && count > 1 {
                div(total, count as f32)
            } else {
                total
            }
        }
        VarArgOp::Mul => {
            if count == 0 {
                return 0.0;
            }
            let mut product = values[0];
            for &value in &values[1..] {
                product = if count == 2 && product.is_nan() && value.is_nan() { nan1(value, value) } else { mul(product, value) };
            }
            product
        }
        VarArgOp::Min | VarArgOp::Max => {
            let pick = if op == VarArgOp::Min { min2 } else { max2 };
            match count {
                0 => 0.0,
                1 => values[0],
                2 => pick(values[0], values[1]),
                3 => pick(pick(values[0], values[1]), values[2]),
                4 => pick(pick(values[0], values[1]), pick(values[2], values[3])),
                5 => pick(pick(pick(values[0], values[1]), pick(values[2], values[3])), values[4]),
                _ => {
                    let mut result = values[0];
                    for &value in &values[1..] {
                        let better = if op == VarArgOp::Min { value < result } else { value > result };
                        if better {
                            result = value;
                        }
                    }
                    result
                }
            }
        }
        VarArgOp::Mand => {
            if values.contains(&0.0) {
                0.0
            } else {
                1.0
            }
        }
        VarArgOp::Mor => {
            if values.iter().any(|&value| value != 0.0) {
                1.0
            } else {
                0.0
            }
        }
        VarArgOp::Multi => values.last().copied().unwrap_or(f32::NAN),
    }
}

/// The ids of `sf00`..`sf30` (`sfNN_op::id()`), which are also what they
/// compute.
pub const SF3_PATTERNS: [&str; 31] = [
    "(t+t)/t", "(t+t)*t", "(t+t)-t", "(t+t)+t", "(t-t)+t", "(t-t)/t", "(t-t)*t", "(t*t)+t", "(t*t)-t", "(t*t)/t", "(t*t)*t", "(t/t)+t", "(t/t)-t", "(t/t)/t",
    "(t/t)*t", "t/(t+t)", "t/(t-t)", "t/(t*t)", "t/(t/t)", "t*(t+t)", "t*(t-t)", "t*(t*t)", "t*(t/t)", "t-(t+t)", "t-(t-t)", "t-(t/t)", "t-(t*t)", "t+(t*t)",
    "t+(t/t)", "t+(t+t)", "t+(t-t)",
];

/// The `sf4_map_` ids: `sf48`..`sf83` (indices 0..36) then `sfext00`..`sfext61`.
pub const SF4_PATTERNS: [&str; 98] = [
    "t+((t+t)/t)", "t+((t+t)*t)", "t+((t-t)/t)", "t+((t-t)*t)", "t+((t*t)/t)", "t+((t*t)*t)", "t+((t/t)+t)", "t+((t/t)/t)", "t+((t/t)*t)", "t-((t+t)/t)",
    "t-((t+t)*t)", "t-((t-t)/t)", "t-((t-t)*t)", "t-((t*t)/t)", "t-((t*t)*t)", "t-((t/t)/t)", "t-((t/t)*t)", "((t+t)*t)-t", "((t-t)*t)-t", "((t*t)*t)-t",
    "((t/t)*t)-t", "((t+t)/t)-t", "((t-t)/t)-t", "((t*t)/t)-t", "((t/t)/t)-t", "(t*t)+(t*t)", "(t*t)-(t*t)", "(t*t)+(t/t)", "(t*t)-(t/t)", "(t/t)+(t/t)",
    "(t/t)-(t/t)", "(t/t)-(t*t)", "t/(t+(t*t))", "t/(t-(t*t))", "t*(t+(t*t))", "t*(t-(t*t))", "(t+t)-(t*t)", "(t+t)-(t/t)", "(t+t)+(t*t)", "(t+t)+(t/t)",
    "(t-t)+(t*t)", "(t-t)+(t/t)", "(t-t)-(t*t)", "(t-t)-(t/t)", "(t+t)-(t-t)", "(t+t)+(t-t)", "(t+t)+(t+t)", "(t+t)*(t-t)", "(t+t)/(t-t)", "(t-t)-(t+t)",
    "(t-t)+(t+t)", "(t-t)*(t+t)", "(t-t)/(t+t)", "(t*t)-(t+t)", "(t/t)-(t+t)", "(t*t)+(t+t)", "(t/t)+(t+t)", "(t*t)+(t-t)", "(t/t)+(t-t)", "(t*t)-(t-t)",
    "(t/t)-(t-t)", "(t+t)*(t*t)", "(t+t)*(t/t)", "(t+t)/(t*t)", "(t+t)/(t/t)", "(t-t)/(t*t)", "(t-t)/(t/t)", "(t-t)*(t*t)", "(t-t)*(t/t)", "(t*t)*(t+t)",
    "(t/t)*(t+t)", "(t*t)/(t+t)", "(t/t)/(t+t)", "(t*t)/(t-t)", "(t/t)/(t-t)", "(t*t)*(t-t)", "(t*t)/(t*t)", "(t/t)*(t/t)", "(t/t)*(t-t)", "(t*t)*(t*t)",
    "t+(t*(t/t))", "t-(t*(t/t))", "t+(t/(t*t))", "t-(t/(t*t))", "((t-t)-t)*t", "((t-t)-t)/t", "((t-t)+t)*t", "((t-t)+t)/t", "(t+(t-t))*t", "(t+(t-t))/t",
    "(t+t)/(t+t)", "(t-t)/(t-t)", "(t+t)*(t+t)", "(t-t)*(t-t)", "(t-t)+(t-t)", "(t-t)-(t-t)", "(t/t)+(t*t)", "((t*t)*t)/t",
];

/// `sfext37` (`(t*t)/(t-t)`): ExprTk 2024-01-01 registers `sfext36` twice
/// instead, so its `sf4_map_` lacks it.
const SF4_UNREGISTERED: usize = 36 + 37;

/// `sf3_optimisable`: the sf3 operation an id names.
pub fn sf3_lookup(id: &str) -> Option<u8> {
    // `register_sf3_extid("(t-t)-t", 23)`: (t-t)-t --> t-(t+t).
    if id == "(t-t)-t" {
        return Some(23);
    }
    SF3_PATTERNS.iter().position(|pattern| *pattern == id).map(|index| index as u8)
}

/// `sf4_optimisable`.
pub fn sf4_lookup(id: &str) -> Option<u8> {
    let index = SF4_PATTERNS.iter().position(|pattern| *pattern == id)?;
    if index == SF4_UNREGISTERED {
        return None;
    }
    Some(index as u8)
}

/// `sfNN_op::process` for `sf00`..`sf47`.
pub fn sf3(index: u8, x: f32, y: f32, z: f32) -> f32 {
    match index {
        0 => div(add_r(x, y), z),
        1 => mul(add_r(x, y), z),
        2 => sub(add_r(x, y), z),
        3 => add(add_r(x, y), z),
        4 => add(sub(x, y), z),
        5 => div(sub(x, y), z),
        6 => mul(sub(x, y), z),
        7 => add(mul_r(x, y), z),
        8 => sub(mul_r(x, y), z),
        9 => div(mul_r(x, y), z),
        10 => mul(mul_r(x, y), z),
        11 => add(div(x, y), z),
        12 => sub(div(x, y), z),
        13 => div(div(x, y), z),
        14 => mul(div(x, y), z),
        15 => div(x, add_r(y, z)),
        16 => div(x, sub(y, z)),
        17 => div(x, mul_r(y, z)),
        18 => div(x, div(y, z)),
        19 => mul_r(x, add_r(y, z)),
        20 => mul_r(x, sub(y, z)),
        21 => mul_r(x, mul_r(y, z)),
        22 => mul_r(x, div(y, z)),
        23 => sub(x, add_r(y, z)),
        24 => sub(x, sub(y, z)),
        25 => sub(x, div(y, z)),
        26 => sub(x, mul_r(y, z)),
        27 => add_r(x, mul_r(y, z)),
        28 => add_r(x, div(y, z)),
        29 => add_r(x, add_r(y, z)),
        30 => add_r(x, sub(y, z)),
        // `sf31`..`sf38`: a * x^n + b.
        31..=38 => {
            let power = u32::from(index) - 29;
            add(mul(x, fast_exp(y, power)), z)
        }
        39 => add(mul(x, nan1(y.ln(), y)), z),
        40 => sub(mul(x, nan1(y.ln(), y)), z),
        41 => add(mul(x, nan1(y.log10(), y)), z),
        42 => sub(mul(x, nan1(y.log10(), y)), z),
        43 => add(mul(x, nan1(y.sin(), y)), z),
        44 => sub(mul(x, nan1(y.sin(), y)), z),
        45 => add(mul(x, nan1(y.cos(), y)), z),
        46 => sub(mul(x, nan1(y.cos(), y)), z),
        _ => {
            if x != 0.0 {
                y
            } else {
                z
            }
        }
    }
}

/// An entry of `SF4_PATTERNS` on four values.
pub fn sf4(index: u8, x: f32, y: f32, z: f32, w: f32) -> f32 {
    match index {
        0 => add_r(x, div(add_r(y, z), w)),
        1 => add_r(x, mul(add_r(y, z), w)),
        2 => add_r(x, div(sub(y, z), w)),
        3 => add_r(x, mul(sub(y, z), w)),
        4 => add_r(x, div(mul_r(y, z), w)),
        5 => add_r(x, mul(mul_r(y, z), w)),
        6 => add_r(x, add(div(y, z), w)),
        7 => add_r(x, div(div(y, z), w)),
        8 => add_r(x, mul(div(y, z), w)),
        9 => sub(x, div(add_r(y, z), w)),
        10 => sub(x, mul(add_r(y, z), w)),
        11 => sub(x, div(sub(y, z), w)),
        12 => sub(x, mul(sub(y, z), w)),
        13 => sub(x, div(mul_r(y, z), w)),
        14 => sub(x, mul(mul_r(y, z), w)),
        15 => sub(x, div(div(y, z), w)),
        16 => sub(x, mul(div(y, z), w)),
        17 => sub(mul(add_r(x, y), z), w),
        18 => sub(mul(sub(x, y), z), w),
        19 => sub(mul(mul_r(x, y), z), w),
        20 => sub(mul(div(x, y), z), w),
        21 => sub(div(add_r(x, y), z), w),
        22 => sub(div(sub(x, y), z), w),
        23 => sub(div(mul_r(x, y), z), w),
        24 => sub(div(div(x, y), z), w),
        25 => add(mul_r(x, y), mul_r(z, w)),
        26 => sub(mul_r(x, y), mul_r(z, w)),
        27 => add(mul_r(x, y), div(z, w)),
        28 => sub(mul_r(x, y), div(z, w)),
        29 => add(div(x, y), div(z, w)),
        30 => sub(div(x, y), div(z, w)),
        31 => sub(div(x, y), mul_r(z, w)),
        32 => div(x, add_r(y, mul_r(z, w))),
        33 => div(x, sub(y, mul_r(z, w))),
        34 => mul_r(x, add_r(y, mul_r(z, w))),
        35 => mul_r(x, sub(y, mul_r(z, w))),
        36 => sub(add_r(x, y), mul_r(z, w)),
        37 => sub(add_r(x, y), div(z, w)),
        38 => add(add_r(x, y), mul_r(z, w)),
        39 => add(add_r(x, y), div(z, w)),
        40 => add(sub(x, y), mul_r(z, w)),
        41 => add(sub(x, y), div(z, w)),
        42 => sub(sub(x, y), mul_r(z, w)),
        43 => sub(sub(x, y), div(z, w)),
        44 => sub(add_r(x, y), sub(z, w)),
        45 => add(add_r(x, y), sub(z, w)),
        46 => add(add_r(x, y), add_r(z, w)),
        47 => mul(add_r(x, y), sub(z, w)),
        48 => div(add_r(x, y), sub(z, w)),
        49 => sub(sub(x, y), add_r(z, w)),
        50 => add(sub(x, y), add_r(z, w)),
        51 => mul(sub(x, y), add_r(z, w)),
        52 => div(sub(x, y), add_r(z, w)),
        53 => sub(mul_r(x, y), add_r(z, w)),
        54 => sub(div(x, y), add_r(z, w)),
        55 => add(mul_r(x, y), add_r(z, w)),
        56 => add(div(x, y), add_r(z, w)),
        57 => add(mul_r(x, y), sub(z, w)),
        58 => add(div(x, y), sub(z, w)),
        59 => sub(mul_r(x, y), sub(z, w)),
        60 => sub(div(x, y), sub(z, w)),
        61 => mul(add_r(x, y), mul_r(z, w)),
        62 => mul(add_r(x, y), div(z, w)),
        63 => div(add_r(x, y), mul_r(z, w)),
        64 => div(add_r(x, y), div(z, w)),
        65 => div(sub(x, y), mul_r(z, w)),
        66 => div(sub(x, y), div(z, w)),
        67 => mul(sub(x, y), mul_r(z, w)),
        68 => mul(sub(x, y), div(z, w)),
        69 => mul(mul_r(x, y), add_r(z, w)),
        70 => mul(div(x, y), add_r(z, w)),
        71 => div(mul_r(x, y), add_r(z, w)),
        72 => div(div(x, y), add_r(z, w)),
        73 => div(mul_r(x, y), sub(z, w)),
        74 => div(div(x, y), sub(z, w)),
        75 => mul(mul_r(x, y), sub(z, w)),
        76 => div(mul_r(x, y), mul_r(z, w)),
        77 => mul(div(x, y), div(z, w)),
        78 => mul(div(x, y), sub(z, w)),
        79 => mul(mul_r(x, y), mul_r(z, w)),
        80 => add_r(x, mul_r(y, div(z, w))),
        81 => sub(x, mul_r(y, div(z, w))),
        82 => add_r(x, div(y, mul_r(z, w))),
        83 => sub(x, div(y, mul_r(z, w))),
        84 => mul(sub(sub(x, y), z), w),
        85 => div(sub(sub(x, y), z), w),
        86 => mul(add(sub(x, y), z), w),
        87 => div(add(sub(x, y), z), w),
        88 => mul(add_r(x, sub(y, z)), w),
        89 => div(add_r(x, sub(y, z)), w),
        90 => div(add_r(x, y), add_r(z, w)),
        91 => div(sub(x, y), sub(z, w)),
        92 => mul(add_r(x, y), add_r(z, w)),
        93 => mul(sub(x, y), sub(z, w)),
        94 => add(sub(x, y), sub(z, w)),
        95 => sub(sub(x, y), sub(z, w)),
        96 => add(div(x, y), mul_r(z, w)),
        _ => div(mul(mul_r(x, y), z), w),
    }
}

/// `$fNN(...)`: `sf00`..`sf47` take three values, `sf48`..`sf99` four.
pub fn special(index: u8, values: [f32; 4]) -> f32 {
    let [x, y, z, w] = values;
    if index < 48 {
        return sf3(index, x, y, z);
    }
    if index < 84 {
        return sf4(index - 48, x, y, z, w);
    }
    let truth = |condition: bool| if condition { z } else { w };
    match index {
        // `sf84`..`sf91`: a * x^n + b * y^n.
        84..=91 => {
            let power = u32::from(index) - 82;
            add(mul(x, fast_exp(y, power)), mul(z, fast_exp(w, power)))
        }
        92 => truth(x != 0.0 && y != 0.0),
        93 => truth(x != 0.0 || y != 0.0),
        94 => truth(x < y),
        95 => truth(x <= y),
        96 => truth(x > y),
        97 => truth(x >= y),
        98 => truth(equal(x, y) != 0.0),
        _ => add(mul(x, nan1(y.sin(), y)), mul(z, nan1(w.cos(), w))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_powers_multiply() {
        assert_eq!(fast_exp(2.0, 10), 1024.0);
        assert_eq!(fast_exp(3.0, 13), 1_594_323.0);
    }

    #[test]
    fn sf_ids_resolve() {
        assert_eq!(sf3_lookup("(t+t)*t"), Some(1));
        assert_eq!(sf3_lookup("(t-t)-t"), Some(23));
        assert_eq!(sf3_lookup("(t<t)+t"), None);
        assert_eq!(sf4_lookup("t+((t+t)/t)"), Some(0));
        assert_eq!(sf4_lookup("((t*t)*t)/t"), Some(97));
        assert_eq!(sf4_lookup("(t*t)/(t-t)"), None);
        assert_eq!(sf4(97, 2.0, 3.0, 4.0, 6.0), 4.0);
    }

    #[test]
    fn x86_truncation() {
        assert_eq!(trunc(f32::NAN), -9.223372e18);
        assert_eq!(trunc(1.0e20), -9.223372e18);
        assert_eq!(trunc(-2.5), -2.0);
        assert_eq!(truncate_to_i32(3.0e9), i32::MIN);
    }
}
