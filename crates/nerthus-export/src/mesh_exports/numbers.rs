//! Number text as the mesh writers produce it: C's `%.*g` (what MSVC's
//! `std::ostream << float` prints through `num_put` and the UCRT's
//! `sprintf_s`), and .NET 8's `float.ToString()` under the en-US culture.

use nerthus_assimp::export::printf::c_general;

/// `std::ostream << float` with the stream's default precision (6).
pub fn ostream_float(value: f32) -> String {
    c_general(value as f64, 6)
}

/// The ASCII digits and decimal exponent of a decimal in scientific form.
fn split_scientific(text: &str) -> (Vec<u8>, i32) {
    let (mantissa, exponent) = text.split_once('e').unwrap_or((text, "0"));
    let digits = mantissa.bytes().filter(u8::is_ascii_digit).collect();
    (digits, exponent.parse().unwrap_or(0))
}

/// The shortest digits that read back as `magnitude`, as .NET's Dragon4
/// picks them: when the value sits exactly halfway between the two
/// shortest candidates and both read back, the even last digit wins
/// (Rust's shortest formatting takes the upper one).
fn dotnet_shortest_digits(magnitude: f32) -> (Vec<u8>, i32) {
    let (digits, exponent) = split_scientific(&format!("{magnitude:e}"));
    let (exact, exact_exponent) = split_scientific(&format!("{:.120e}", magnitude as f64));
    let count = digits.len();
    let odd = digits.last().is_some_and(|digit| (digit - b'0') % 2 == 1);
    let tie = exact_exponent == exponent && exact.len() > count && exact[count] == b'5' && exact[count + 1..].iter().all(|digit| *digit == b'0');
    if !odd || !tie {
        return (digits, exponent);
    }
    let truncated = &exact[..count];
    let alternative = if digits.as_slice() == truncated {
        // Rust rounded down to an odd digit: the other candidate is one
        // up (an odd last digit cannot carry).
        let mut raised = truncated.to_vec();
        *raised.last_mut().unwrap() += 1;
        raised
    } else {
        truncated.to_vec()
    };
    let text = format!("{}.{}e{}", alternative[0] as char, String::from_utf8_lossy(&alternative[1..]), exponent);
    if text.parse::<f32>() == Ok(magnitude) {
        return (alternative, exponent);
    }
    (digits, exponent)
}

/// .NET 8's `float.ToString()` with en-US formats: the shortest digits that
/// read back as the same float, laid out by the "G" rules — scientific
/// (`E+XX`) when the decimal point would sit more than 9 (or the digit
/// count) places right of the first digit or more than 3 zeros left of it.
pub fn dotnet_single(value: f32) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-∞".to_string() } else { "∞".to_string() };
    }
    let sign = if value.is_sign_negative() { "-" } else { "" };
    if value == 0.0 {
        return format!("{sign}0");
    }
    let (digits, exponent) = dotnet_shortest_digits(value.abs());
    // `Number.Scale`: value = 0.d1d2… × 10^scale.
    let scale = exponent + 1;
    let max_digits = digits.len().max(9) as i32;

    let mut text = String::from(sign);
    let scientific = scale > max_digits || scale < -3;
    let mut point = if scientific { 1 } else { scale };
    let mut next = 0usize;
    if point > 0 {
        while point > 0 {
            let digit = digits.get(next).copied().unwrap_or(b'0');
            text.push(digit as char);
            next += 1;
            point -= 1;
        }
    } else {
        text.push('0');
    }
    if next < digits.len() || point < 0 {
        text.push('.');
        while point < 0 {
            text.push('0');
            point += 1;
        }
        while next < digits.len() {
            text.push(digits[next] as char);
            next += 1;
        }
    }
    if scientific {
        let power = scale - 1;
        let power_sign = if power < 0 { '-' } else { '+' };
        text.push('E');
        text.push(power_sign);
        text.push_str(&format!("{:02}", power.abs()));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotnet_layouts() {
        assert_eq!(dotnet_single(0.5), "0.5");
        assert_eq!(dotnet_single(1.0 / 1024.0), "0.0009765625");
        assert_eq!(dotnet_single(1.0 / 16384.0), "6.1035156E-05");
        assert_eq!(dotnet_single(-60.0), "-60");
        assert_eq!(dotnet_single(123456789.0), "123456790");
        assert_eq!(dotnet_single(1e10), "1E+10");
        assert_eq!(dotnet_single(0.1), "0.1");
        assert_eq!(dotnet_single(-0.0), "-0");
    }
}
