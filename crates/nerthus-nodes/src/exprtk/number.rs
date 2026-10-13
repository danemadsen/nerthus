//! ExprTk's number parser for `float` (`details::string_to_real`), which
//! accumulates digits in `float` and scales them through a `double` table
//! of powers of ten, so its results can differ from a correctly rounded
//! parse.

/// `numeric_info<float>::min_exp`/`max_exp`.
const MIN_EXPONENT: i32 = -38;
const MAX_EXPONENT: i32 = 38;
/// `std::numeric_limits<float>::min_exponent10`.
const MIN_EXPONENT10: i32 = -37;

/// `compute_pow10`'s `fract10` table (`fract10[0]` is 0).
fn fract10(index: usize) -> f64 {
    if index == 0 {
        return 0.0;
    }
    let text = format!("1e{index}");
    text.parse().unwrap_or(f64::INFINITY)
}

/// `compute_pow10(d, exponent)`.
fn compute_pow10(value: f32, exponent: i32) -> f32 {
    let magnitude = exponent.unsigned_abs() as usize;
    if exponent >= MIN_EXPONENT10 {
        if magnitude < 309 {
            let scale = fract10(magnitude);
            if exponent > 0 {
                return (f64::from(value) * scale) as f32;
            }
            return (f64::from(value) / scale) as f32;
        }
        let scale = 10.0f64.powf(10.0 * f64::from(exponent));
        return (f64::from(value) * scale) as f32;
    }
    let first_scale = fract10((-MIN_EXPONENT10) as usize) as f32;
    let scaled = value / first_scale;
    let rest = (-exponent + MIN_EXPONENT10) as usize;
    (f64::from(scaled) / fract10(rest)) as f32
}

fn valid_exponent(exponent: i32) -> bool {
    (MIN_EXPONENT..=MAX_EXPONENT).contains(&exponent)
}

/// `string_to_type_converter_impl_ref` for the `int` after an `e`: at most
/// four significant digits. Returns whether it succeeded and the value so
/// far (which ExprTk adds either way).
fn parse_exponent(bytes: &[u8], index: &mut usize) -> (bool, i32) {
    let end = bytes.len();
    if *index == end {
        return (false, 0);
    }
    let negative = bytes[*index] == b'-';
    if negative || bytes[*index] == b'+' {
        *index += 1;
        if *index == end {
            return (false, 0);
        }
    }
    while *index < end && bytes[*index] == b'0' {
        *index += 1;
    }
    let length = end - *index;
    let mut result = 0i32;
    let mut succeeded = true;
    let mut last_digit = 0;
    if length <= 4 {
        // All but the last digit, then the last one.
        for _ in 1..length {
            let digit = bytes[*index].wrapping_sub(b'0');
            *index += 1;
            if digit >= 10 {
                succeeded = false;
                break;
            }
            result = result * 10 + i32::from(digit);
        }
        if succeeded && length > 0 {
            let digit = bytes[*index].wrapping_sub(b'0');
            if digit >= 10 {
                succeeded = false;
            } else {
                last_digit = i32::from(digit);
            }
        }
    } else {
        succeeded = false;
    }
    if length > 0 && succeeded {
        result = result * 10 + last_digit;
        *index += 1;
    }
    if negative {
        result = -result;
    }
    (succeeded, result)
}

/// Digits accumulated as ExprTk does: `d = d * T(10) + digit`.
fn accumulate_digits(bytes: &[u8], index: &mut usize, value: &mut f32) {
    while *index < bytes.len() {
        let digit = bytes[*index].wrapping_sub(b'0');
        if digit >= 10 {
            break;
        }
        *value = *value * 10.0 + f32::from(digit);
        *index += 1;
    }
}

/// `details::string_to_real<float>`: `None` where ExprTk reports "Failed
/// to convert" (or the lexer's "Invalid numeric token").
pub fn string_to_real(text: &str) -> Option<f32> {
    let bytes = text.as_bytes();
    let end = bytes.len();
    if end == 0 {
        return None;
    }
    let mut index = 0;
    let negative = bytes[0] == b'-';
    if negative || bytes[0] == b'+' {
        index += 1;
        if index == end {
            return None;
        }
    }

    let mut instate = false;
    let mut value = 0.0f32;
    if bytes[index] != b'.' {
        let start = index;
        while index < end && bytes[index] == b'0' {
            index += 1;
        }
        accumulate_digits(bytes, &mut index, &mut value);
        if start != index {
            instate = true;
        }
    }

    let mut exponent = 0;
    if index < end {
        if bytes[index] == b'.' {
            index += 1;
            let start = index;
            let mut fraction = 0.0f32;
            accumulate_digits(bytes, &mut index, &mut fraction);
            if start != index {
                instate = true;
                let fraction_exponent = -((index - start) as i32);
                if !valid_exponent(fraction_exponent) {
                    return None;
                }
                value += compute_pow10(fraction, fraction_exponent);
            }
        }
        if index < end {
            let character = bytes[index];
            if character == b'e' || character == b'E' {
                index += 1;
                let (succeeded, parsed) = parse_exponent(bytes, &mut index);
                if !succeeded && index == end {
                    return None;
                }
                exponent += parsed;
            }
            // ExprTk also takes f/F/l/L suffixes and #inf/#nan here; the
            // lexer never produces them.
            if index < end {
                return None;
            }
        }
    }

    if index != end || !instate || !valid_exponent(exponent) {
        return None;
    }
    if exponent != 0 {
        value = compute_pow10(value, exponent);
    }
    Some(if negative { -value } else { value })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_like_exprtk() {
        assert_eq!(string_to_real("1.5"), Some(1.5));
        assert_eq!(string_to_real(".25"), Some(0.25));
        assert_eq!(string_to_real("1."), Some(1.0));
        assert_eq!(string_to_real("2e3"), Some(2000.0));
        assert_eq!(string_to_real("1e38"), Some(1.0e38));
        assert_eq!(string_to_real("1e39"), None);
        assert_eq!(string_to_real("1e00001"), Some(10.0));
        assert_eq!(string_to_real("1e12345"), None);
        assert_eq!(string_to_real("16777217"), Some(16_777_216.0));
        assert_eq!(string_to_real("0.1"), Some(0.1));
    }
}
