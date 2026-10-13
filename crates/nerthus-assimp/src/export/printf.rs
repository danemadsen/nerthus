//! C's `printf("%.*g")`, which C++ streams print floats with (MSVC's
//! `num_put` through the UCRT's correctly rounded `sprintf_s`): ties go to
//! even and trailing zeros are dropped.

/// The decimal digits of `value`'s magnitude rounded to `precision`
/// significant digits (ties to even, as the UCRT's correctly rounded
/// `printf` does), with the decimal exponent of the first digit.
fn rounded_digits(value: f64, precision: usize) -> (Vec<u8>, i32) {
    let text = format!("{:.*e}", precision - 1, value.abs());
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    let digits = mantissa.bytes().filter(u8::is_ascii_digit).map(|digit| digit - b'0').collect();
    let exponent = exponent.parse().unwrap_or(0);
    (digits, exponent)
}

/// The UCRT's spelling of infinities and NaNs.
fn non_finite(value: f64) -> String {
    if value.is_infinite() {
        return if value < 0.0 { "-inf".to_string() } else { "inf".to_string() };
    }
    let bits = value.to_bits();
    let negative = bits >> 63 != 0;
    let quiet_payload = bits & 0x000f_ffff_ffff_ffff;
    // The default quiet NaN with its sign set is "indeterminate".
    if negative && quiet_payload == 0x0008_0000_0000_0000 {
        return "-nan(ind)".to_string();
    }
    if negative { "-nan".to_string() } else { "nan".to_string() }
}

/// C's `%.*g` with `precision` significant digits (0 counts as 1): fixed
/// notation when the exponent is at least −4 and below the precision,
/// scientific otherwise, trailing zeros removed either way.
pub fn c_general(value: f64, precision: usize) -> String {
    if !value.is_finite() {
        return non_finite(value);
    }
    let precision = precision.max(1);
    let sign = if value.is_sign_negative() { "-" } else { "" };
    if value == 0.0 {
        return format!("{sign}0");
    }
    let (digits, exponent) = rounded_digits(value, precision);
    let mut text = String::from(sign);
    if exponent < -4 || exponent >= precision as i32 {
        let mut fraction: Vec<u8> = digits[1..].to_vec();
        while fraction.last() == Some(&0) {
            fraction.pop();
        }
        text.push((b'0' + digits[0]) as char);
        if !fraction.is_empty() {
            text.push('.');
            text.extend(fraction.iter().map(|digit| (b'0' + digit) as char));
        }
        let exponent_sign = if exponent < 0 { '-' } else { '+' };
        text.push('e');
        text.push(exponent_sign);
        text.push_str(&format!("{:02}", exponent.abs()));
        return text;
    }
    // Fixed: `precision − 1 − exponent` decimals, then trailing zeros go.
    let decimals = (precision as i32 - 1 - exponent) as usize;
    let fixed = format!("{:.*}", decimals, value.abs());
    let trimmed = if fixed.contains('.') { fixed.trim_end_matches('0').trim_end_matches('.') } else { fixed.as_str() };
    text.push_str(trimmed);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_layouts() {
        assert_eq!(c_general(0.5, 6), "0.5");
        assert_eq!(c_general(1234.5678, 6), "1234.57");
        assert_eq!(c_general(123456.0, 6), "123456");
        assert_eq!(c_general(1234567.0, 6), "1.23457e+06");
        assert_eq!(c_general(0.0001, 6), "0.0001");
        assert_eq!(c_general(0.00001234, 6), "1.234e-05");
        assert_eq!(c_general(-0.0, 6), "-0");
        assert_eq!(c_general(100.0, 6), "100");
        assert_eq!(c_general(1e100, 6), "1e+100");
    }

    /// The C library's text for one value in a `printf` format.
    fn printf(format: &std::ffi::CStr, value: f64) -> String {
        use std::ffi::{c_char, c_int, CStr};
        unsafe extern "C" {
            fn snprintf(buffer: *mut c_char, size: usize, format: *const c_char, ...) -> c_int;
        }
        let mut buffer = [0 as c_char; 64];
        unsafe {
            snprintf(buffer.as_mut_ptr(), buffer.len(), format.as_ptr(), value);
            CStr::from_ptr(buffer.as_ptr()).to_string_lossy().into_owned()
        }
    }

    /// macOS's `%g` keeps the trailing zeros of a mantissa it rounded from
    /// an exact tie (`2.01070e+06`); C (and the UCRT) drop them.
    fn strip_mantissa_zeros(text: &str) -> String {
        let (mantissa, exponent) = match text.find('e') {
            Some(position) => (&text[..position], &text[position..]),
            None => (text, ""),
        };
        if !mantissa.contains('.') {
            return text.to_string();
        }
        let trimmed = mantissa.trim_end_matches('0').trim_end_matches('.');
        format!("{trimmed}{exponent}")
    }

    /// `%.6g` against the platform's C library (correctly rounded, ties to
    /// even, like the UCRT's) for random floats of every magnitude.
    #[test]
    fn general_matches_printf() {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..200_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let value = f32::from_bits(state as u32);
            if !value.is_finite() {
                continue;
            }
            let scientific = printf(c"%.5e", value as f64);
            let (digits, exponent) = rounded_digits(value as f64, 6);
            let mantissa: String = scientific.split('e').next().unwrap().chars().filter(char::is_ascii_digit).collect();
            let expected_exponent: i32 = scientific.split('e').nth(1).unwrap().parse().unwrap();
            let ours: String = digits.iter().map(|digit| (b'0' + digit) as char).collect();
            assert_eq!((ours, exponent), (mantissa, expected_exponent), "{value:e}");
            let general = strip_mantissa_zeros(&printf(c"%.6g", value as f64));
            assert_eq!(c_general(value as f64, 6), general, "{value:e} ({:#010x})", value.to_bits());
        }
    }
}
