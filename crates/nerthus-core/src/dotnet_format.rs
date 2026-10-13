//! .NET Framework number formatting, for values the nodes hand to ImageMagick
//! as strings (`double.ToString()`, Magick.NET's `Percentage` as `"0.##%"`).
//!
//! .NET Framework first turns a double into fifteen significant decimal
//! digits and then rounds that digit string half away from zero.

/// The fifteen significant digits .NET Framework starts from, as
/// (digits, decimal exponent) where value = 0.d1d2... x 10^exponent.
fn fifteen_digits(value: f64) -> (Vec<u8>, i32) {
    let text = format!("{:.14e}", value.abs());
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    let mut digits: Vec<u8> = mantissa.bytes().filter(u8::is_ascii_digit).map(|digit| digit - b'0').collect();
    let exponent: i32 = exponent.parse().unwrap_or(0);
    while digits.len() > 1 && digits.last() == Some(&0) {
        digits.pop();
    }
    (digits, exponent + 1)
}

/// Rounds the digit string so that at most `decimals` digits follow the
/// decimal point, half away from zero. Returns the new digits and exponent.
fn round_digits(mut digits: Vec<u8>, mut exponent: i32, decimals: usize) -> (Vec<u8>, i32) {
    let keep = exponent + decimals as i32;
    if keep < 0 {
        return (vec![0], 1);
    }
    let keep = keep as usize;
    if keep >= digits.len() {
        return (digits, exponent);
    }
    let round_up = digits[keep] >= 5;
    digits.truncate(keep);
    if round_up {
        let mut index = keep;
        loop {
            if index == 0 {
                digits.insert(0, 1);
                exponent += 1;
                break;
            }
            index -= 1;
            if digits[index] == 9 {
                digits[index] = 0;
            } else {
                digits[index] += 1;
                break;
            }
        }
    }
    if digits.is_empty() {
        digits.push(0);
    }
    (digits, exponent)
}

/// Builds the plain decimal text for digits and exponent (no trailing zeros).
fn decimal_text(negative: bool, digits: &[u8], exponent: i32) -> String {
    let mut integer = String::new();
    let mut fraction = String::new();
    for (index, digit) in digits.iter().enumerate() {
        let character = char::from(b'0' + digit);
        if (index as i32) < exponent {
            integer.push(character);
        } else {
            fraction.push(character);
        }
    }
    while (integer.len() as i32) < exponent {
        integer.push('0');
    }
    if exponent < 0 {
        let zeros = "0".repeat((-exponent) as usize);
        fraction.insert_str(0, &zeros);
    }
    let fraction = fraction.trim_end_matches('0');
    if integer.is_empty() {
        integer.push('0');
    }
    let is_zero = integer.chars().all(|character| character == '0') && fraction.is_empty();
    let sign = if negative && !is_zero { "-" } else { "" };
    if fraction.is_empty() {
        format!("{sign}{integer}")
    } else {
        format!("{sign}{integer}.{fraction}")
    }
}

/// `value.ToString(format)` for custom formats like `"0.##"`: at most
/// `decimals` fraction digits, trailing zeros dropped.
pub fn format_decimals(value: f64, decimals: usize) -> String {
    if value == 0.0 || !value.is_finite() {
        return "0".to_string();
    }
    let (digits, exponent) = fifteen_digits(value);
    let (digits, exponent) = round_digits(digits, exponent, decimals);
    decimal_text(value < 0.0, &digits, exponent)
}

/// The number a C `strtod` reads back from .NET Framework's
/// `double.ToString()` (fifteen significant digits).
pub fn general_round_trip(value: f64) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    let text = format!("{value:.14e}");
    text.parse().unwrap_or(value)
}

/// The number IM reads back from a Magick.NET `Percentage` (`"{0:0.##}%"`).
pub fn percentage_round_trip(value: f64) -> f64 {
    let text = format_decimals(value, 2);
    text.parse().unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_dotnet_custom_decimals() {
        assert_eq!(format_decimals(10.0, 2), "10");
        assert_eq!(format_decimals(12.345, 2), "12.35");
        assert_eq!(format_decimals(0.005, 2), "0.01");
        assert_eq!(format_decimals(99.999, 2), "100");
        assert_eq!(format_decimals(-1.25, 1), "-1.3");
        assert_eq!(format_decimals(0.0001, 2), "0");
        assert_eq!(format_decimals(1234.5, 0), "1235");
    }

    #[test]
    fn general_round_trip_keeps_fifteen_digits() {
        assert_eq!(general_round_trip(2.4), 2.4);
        let float_value = 0.3f32 as f64 * 8.0;
        assert_eq!(general_round_trip(float_value), 2.40000009536743);
    }
}
