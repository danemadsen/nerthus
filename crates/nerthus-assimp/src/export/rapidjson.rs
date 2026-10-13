//! The RapidJSON writing Assimp's glTF 2 exporter does: a document of
//! members in insertion order, written by `PrettyWriter` (four-space
//! indents, one array element per line) or the compact `Writer`, with
//! doubles through RapidJSON's Grisu2 `dtoa` and `Prettify`.

/// A RapidJSON value as the exporter builds it.
#[derive(Clone, Debug)]
pub enum Json {
    Object(Vec<(String, Json)>),
    Array(Vec<Json>),
    String(String),
    Int(i64),
    Uint(u64),
    Double(f64),
    Bool(bool),
}

impl Json {
    pub fn object() -> Self {
        Json::Object(Vec::new())
    }

    /// `AddMember`: appended, even when the name is already there.
    pub fn add(&mut self, name: &str, value: Json) {
        if let Json::Object(members) = self {
            members.push((name.to_string(), value));
        }
    }

    pub fn is_empty_object(&self) -> bool {
        matches!(self, Json::Object(members) if members.is_empty())
    }

    /// The array member `name` (the first one), for `mDoc["buffers"]`.
    pub fn member_mut(&mut self, name: &str) -> Option<&mut Json> {
        match self {
            Json::Object(members) => members.iter_mut().find(|(key, _)| key == name).map(|(_, value)| value),
            _ => None,
        }
    }
}

/// `DiyFp`: a 64-bit significand and a binary exponent.
#[derive(Clone, Copy, Debug)]
struct DiyFp {
    f: u64,
    e: i32,
}

const SIGNIFICAND_SIZE: i32 = 52;
const EXPONENT_BIAS: i32 = 0x3FF + SIGNIFICAND_SIZE;
const MIN_EXPONENT: i32 = -EXPONENT_BIAS;
const HIDDEN_BIT: u64 = 0x0010_0000_0000_0000;
const SIGNIFICAND_MASK: u64 = 0x000F_FFFF_FFFF_FFFF;
const EXPONENT_MASK: u64 = 0x7FF0_0000_0000_0000;

impl DiyFp {
    fn from_double(value: f64) -> Self {
        let bits = value.to_bits();
        let biased = ((bits & EXPONENT_MASK) >> SIGNIFICAND_SIZE) as i32;
        let significand = bits & SIGNIFICAND_MASK;
        if biased != 0 {
            DiyFp { f: significand + HIDDEN_BIT, e: biased - EXPONENT_BIAS }
        } else {
            DiyFp { f: significand, e: MIN_EXPONENT + 1 }
        }
    }

    fn minus(self, other: DiyFp) -> DiyFp {
        DiyFp { f: self.f.wrapping_sub(other.f), e: self.e }
    }

    /// `operator*` (the `_umul128` path): the high half, rounded up when
    /// the low half's top bit is set.
    fn times(self, other: DiyFp) -> DiyFp {
        let product = self.f as u128 * other.f as u128;
        let mut high = (product >> 64) as u64;
        let low = product as u64;
        if low & (1 << 63) != 0 {
            high = high.wrapping_add(1);
        }
        DiyFp { f: high, e: self.e + other.e + 64 }
    }

    fn normalize(self) -> DiyFp {
        let shift = self.f.leading_zeros() as i32;
        DiyFp { f: self.f << shift, e: self.e - shift }
    }

    fn normalize_boundary(self) -> DiyFp {
        let mut result = self;
        while result.f & (HIDDEN_BIT << 1) == 0 {
            result.f <<= 1;
            result.e -= 1;
        }
        let shift = 64 - SIGNIFICAND_SIZE - 2;
        result.f <<= shift;
        result.e -= shift;
        result
    }

    fn normalized_boundaries(self) -> (DiyFp, DiyFp) {
        let plus = DiyFp { f: (self.f << 1) + 1, e: self.e - 1 }.normalize_boundary();
        let mut minus = if self.f == HIDDEN_BIT { DiyFp { f: (self.f << 2) - 1, e: self.e - 2 } } else { DiyFp { f: (self.f << 1) - 1, e: self.e - 1 } };
        minus.f <<= minus.e - plus.e;
        minus.e = plus.e;
        (minus, plus)
    }
}

/// 10^-348, 10^-340, …, 10^340 (`kCachedPowers_F` / `kCachedPowers_E`).
const CACHED_POWERS_F: [u64; 87] = [
    0xfa8fd5a0081c0288, 0xbaaee17fa23ebf76, 0x8b16fb203055ac76, 0xcf42894a5dce35ea, 0x9a6bb0aa55653b2d, 0xe61acf033d1a45df, 0xab70fe17c79ac6ca,
    0xff77b1fcbebcdc4f, 0xbe5691ef416bd60c, 0x8dd01fad907ffc3c, 0xd3515c2831559a83, 0x9d71ac8fada6c9b5, 0xea9c227723ee8bcb, 0xaecc49914078536d,
    0x823c12795db6ce57, 0xc21094364dfb5637, 0x9096ea6f3848984f, 0xd77485cb25823ac7, 0xa086cfcd97bf97f4, 0xef340a98172aace5, 0xb23867fb2a35b28e,
    0x84c8d4dfd2c63f3b, 0xc5dd44271ad3cdba, 0x936b9fcebb25c996, 0xdbac6c247d62a584, 0xa3ab66580d5fdaf6, 0xf3e2f893dec3f126, 0xb5b5ada8aaff80b8,
    0x87625f056c7c4a8b, 0xc9bcff6034c13053, 0x964e858c91ba2655, 0xdff9772470297ebd, 0xa6dfbd9fb8e5b88f, 0xf8a95fcf88747d94, 0xb94470938fa89bcf,
    0x8a08f0f8bf0f156b, 0xcdb02555653131b6, 0x993fe2c6d07b7fac, 0xe45c10c42a2b3b06, 0xaa242499697392d3, 0xfd87b5f28300ca0e, 0xbce5086492111aeb,
    0x8cbccc096f5088cc, 0xd1b71758e219652c, 0x9c40000000000000, 0xe8d4a51000000000, 0xad78ebc5ac620000, 0x813f3978f8940984, 0xc097ce7bc90715b3,
    0x8f7e32ce7bea5c70, 0xd5d238a4abe98068, 0x9f4f2726179a2245, 0xed63a231d4c4fb27, 0xb0de65388cc8ada8, 0x83c7088e1aab65db, 0xc45d1df942711d9a,
    0x924d692ca61be758, 0xda01ee641a708dea, 0xa26da3999aef774a, 0xf209787bb47d6b85, 0xb454e4a179dd1877, 0x865b86925b9bc5c2, 0xc83553c5c8965d3d,
    0x952ab45cfa97a0b3, 0xde469fbd99a05fe3, 0xa59bc234db398c25, 0xf6c69a72a3989f5c, 0xb7dcbf5354e9bece, 0x88fcf317f22241e2, 0xcc20ce9bd35c78a5,
    0x98165af37b2153df, 0xe2a0b5dc971f303a, 0xa8d9d1535ce3b396, 0xfb9b7cd9a4a7443c, 0xbb764c4ca7a44410, 0x8bab8eefb6409c1a, 0xd01fef10a657842c,
    0x9b10a4e5e9913129, 0xe7109bfba19c0c9d, 0xac2820d9623bf429, 0x80444b5e7aa7cf85, 0xbf21e44003acdd2d, 0x8e679c2f5e44ff8f, 0xd433179d9c8cb841,
    0x9e19db92b4e31ba9, 0xeb96bf6ebadf77d9, 0xaf87023b9bf0ee6b,
];

const CACHED_POWERS_E: [i16; 87] = [
    -1220, -1193, -1166, -1140, -1113, -1087, -1060, -1034, -1007, -980, -954, -927, -901, -874, -847, -821, -794, -768, -741, -715, -688, -661,
    -635, -608, -582, -555, -529, -502, -475, -449, -422, -396, -369, -343, -316, -289, -263, -236, -210, -183, -157, -130, -103, -77, -50, -24, 3,
    30, 56, 83, 109, 136, 162, 189, 216, 242, 269, 295, 322, 348, 375, 402, 428, 455, 481, 508, 534, 561, 588, 614, 641, 667, 694, 720, 747, 774,
    800, 827, 853, 880, 907, 933, 960, 986, 1013, 1039, 1066,
];

/// `GetCachedPower`: the cached power of ten for binary exponent `e`, and
/// its decimal exponent negated.
fn cached_power(e: i32) -> (DiyFp, i32) {
    let estimate = (-61 - e) as f64 * 0.30102999566398114 + 347.0;
    let mut k = estimate as i32;
    if estimate - k as f64 > 0.0 {
        k += 1;
    }
    let index = ((k >> 3) + 1) as usize;
    let decimal = -(-348 + (index << 3) as i32);
    (DiyFp { f: CACHED_POWERS_F[index], e: CACHED_POWERS_E[index] as i32 }, decimal)
}

const POW10: [u64; 20] = [
    1,
    10,
    100,
    1000,
    10000,
    100000,
    1000000,
    10000000,
    100000000,
    1000000000,
    10000000000,
    100000000000,
    1000000000000,
    10000000000000,
    100000000000000,
    1000000000000000,
    10000000000000000,
    100000000000000000,
    1000000000000000000,
    10000000000000000000,
];

fn grisu_round(buffer: &mut [u8], length: usize, delta: u64, mut rest: u64, ten_kappa: u64, wp_w: u64) {
    while rest < wp_w
        && delta.wrapping_sub(rest) >= ten_kappa
        && (rest.wrapping_add(ten_kappa) < wp_w || wp_w.wrapping_sub(rest) > rest.wrapping_add(ten_kappa).wrapping_sub(wp_w))
    {
        buffer[length - 1] -= 1;
        rest = rest.wrapping_add(ten_kappa);
    }
}

fn count_decimal_digits(value: u32) -> i32 {
    match value {
        0..=9 => 1,
        10..=99 => 2,
        100..=999 => 3,
        1000..=9999 => 4,
        10000..=99999 => 5,
        100000..=999999 => 6,
        1000000..=9999999 => 7,
        10000000..=99999999 => 8,
        _ => 9,
    }
}

/// `DigitGen`: the digits into `buffer`; returns their count and adjusts
/// the decimal exponent `k`.
fn digit_gen(w: DiyFp, mp: DiyFp, mut delta: u64, buffer: &mut [u8], k: &mut i32) -> usize {
    let one = DiyFp { f: 1u64 << -mp.e, e: mp.e };
    let wp_w = mp.minus(w);
    let mut p1 = (mp.f >> -one.e) as u32;
    let mut p2 = mp.f & (one.f - 1);
    let mut kappa = count_decimal_digits(p1);
    let mut length = 0usize;
    while kappa > 0 {
        let divisor = POW10[(kappa - 1) as usize] as u32;
        let digit = p1 / divisor;
        p1 %= divisor;
        if digit != 0 || length != 0 {
            buffer[length] = b'0' + digit as u8;
            length += 1;
        }
        kappa -= 1;
        let remainder = ((p1 as u64) << -one.e) + p2;
        if remainder <= delta {
            *k += kappa;
            grisu_round(buffer, length, delta, remainder, POW10[kappa as usize] << -one.e, wp_w.f);
            return length;
        }
    }
    loop {
        p2 = p2.wrapping_mul(10);
        delta = delta.wrapping_mul(10);
        let digit = (p2 >> -one.e) as u8;
        if digit != 0 || length != 0 {
            buffer[length] = b'0' + digit;
            length += 1;
        }
        p2 &= one.f - 1;
        kappa -= 1;
        if p2 < delta {
            *k += kappa;
            let index = -kappa;
            let scale = if index < 20 { POW10[index as usize] } else { 0 };
            grisu_round(buffer, length, delta, p2, one.f, wp_w.f.wrapping_mul(scale));
            return length;
        }
    }
}

/// `Grisu2`: the digits of a positive double and their decimal exponent.
fn grisu2(value: f64, buffer: &mut [u8]) -> (usize, i32) {
    let v = DiyFp::from_double(value);
    let (w_minus, w_plus) = v.normalized_boundaries();
    let (cached, mut k) = cached_power(w_plus.e);
    let w = v.normalize().times(cached);
    let mut wp = w_plus.times(cached);
    let mut wm = w_minus.times(cached);
    wm.f = wm.f.wrapping_add(1);
    wp.f = wp.f.wrapping_sub(1);
    let length = digit_gen(w, wp, wp.f.wrapping_sub(wm.f), buffer, &mut k);
    (length, k)
}

fn write_exponent(mut exponent: i32, text: &mut String) {
    if exponent < 0 {
        text.push('-');
        exponent = -exponent;
    }
    text.push_str(&exponent.to_string());
}

/// `internal::dtoa` with the default 324 decimal places.
pub fn dtoa(value: f64) -> String {
    let mut text = String::new();
    if value == 0.0 {
        if value.is_sign_negative() {
            text.push('-');
        }
        text.push_str("0.0");
        return text;
    }
    let mut magnitude = value;
    if value < 0.0 {
        text.push('-');
        magnitude = -value;
    }
    let mut buffer = [0u8; 32];
    let (length, k) = grisu2(magnitude, &mut buffer);
    let digits = std::str::from_utf8(&buffer[..length]).unwrap_or("0");
    let length = length as i32;
    let kk = length + k;
    // `Prettify` (no decimal-place limit applies at 324).
    if 0 <= k && kk <= 21 {
        text.push_str(digits);
        for _ in length..kk {
            text.push('0');
        }
        text.push_str(".0");
    } else if 0 < kk && kk <= 21 {
        text.push_str(&digits[..kk as usize]);
        text.push('.');
        text.push_str(&digits[kk as usize..]);
    } else if -6 < kk && kk <= 0 {
        text.push_str("0.");
        for _ in 0..-kk {
            text.push('0');
        }
        text.push_str(digits);
    } else if kk < -324 {
        text.push_str("0.0");
    } else if length == 1 {
        text.push_str(digits);
        text.push('e');
        write_exponent(kk - 1, &mut text);
    } else {
        text.push_str(&digits[..1]);
        text.push('.');
        text.push_str(&digits[1..]);
        text.push('e');
        write_exponent(kk - 1, &mut text);
    }
    text
}

/// `Writer::WriteString`: quotes, with RapidJSON's escapes.
fn write_string(text: &str, output: &mut String) {
    output.push('"');
    for character in text.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{8}' => output.push_str("\\b"),
            '\u{c}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            control if (control as u32) < 0x20 => output.push_str(&format!("\\u{:04X}", control as u32)),
            other => output.push(other),
        }
    }
    output.push('"');
}

fn write_scalar(value: &Json, output: &mut String) {
    match value {
        Json::String(text) => write_string(text, output),
        Json::Int(number) => output.push_str(&number.to_string()),
        Json::Uint(number) => output.push_str(&number.to_string()),
        Json::Double(number) => output.push_str(&dtoa(*number)),
        Json::Bool(flag) => output.push_str(if *flag { "true" } else { "false" }),
        Json::Object(_) | Json::Array(_) => {}
    }
}

/// `Writer`: no whitespace at all.
pub fn write_compact(value: &Json) -> String {
    fn walk(value: &Json, output: &mut String) {
        match value {
            Json::Object(members) => {
                output.push('{');
                for (index, (name, member)) in members.iter().enumerate() {
                    if index > 0 {
                        output.push(',');
                    }
                    write_string(name, output);
                    output.push(':');
                    walk(member, output);
                }
                output.push('}');
            }
            Json::Array(items) => {
                output.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        output.push(',');
                    }
                    walk(item, output);
                }
                output.push(']');
            }
            scalar => write_scalar(scalar, output),
        }
    }
    let mut output = String::new();
    walk(value, &mut output);
    output
}

/// `PrettyWriter` (four spaces, `kFormatDefault`): every member and array
/// element on its own line; empty objects and arrays stay `{}` / `[]`.
pub fn write_pretty(value: &Json) -> String {
    fn indent(level: usize, output: &mut String) {
        for _ in 0..level * 4 {
            output.push(' ');
        }
    }
    fn walk(value: &Json, level: usize, output: &mut String) {
        match value {
            Json::Object(members) => {
                output.push('{');
                if members.is_empty() {
                    output.push('}');
                    return;
                }
                for (index, (name, member)) in members.iter().enumerate() {
                    if index > 0 {
                        output.push(',');
                    }
                    output.push('\n');
                    indent(level + 1, output);
                    write_string(name, output);
                    output.push_str(": ");
                    walk(member, level + 1, output);
                }
                output.push('\n');
                indent(level, output);
                output.push('}');
            }
            Json::Array(items) => {
                output.push('[');
                if items.is_empty() {
                    output.push(']');
                    return;
                }
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        output.push(',');
                    }
                    output.push('\n');
                    indent(level + 1, output);
                    walk(item, level + 1, output);
                }
                output.push('\n');
                indent(level, output);
                output.push(']');
            }
            scalar => write_scalar(scalar, output),
        }
    }
    let mut output = String::new();
    walk(value, 0, &mut output);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rapidjson_numbers() {
        assert_eq!(dtoa(0.0), "0.0");
        assert_eq!(dtoa(-0.0), "-0.0");
        assert_eq!(dtoa(0.5), "0.5");
        assert_eq!(dtoa(1.0), "1.0");
        assert_eq!(dtoa(0.4419859051704407), "0.4419859051704407");
        assert_eq!(dtoa(1e-7), "1e-7");
        assert_eq!(dtoa(123456.0), "123456.0");
        assert_eq!(dtoa(0.0001), "0.0001");
        assert_eq!(dtoa(1.5e300), "1.5e300");
        assert_eq!(dtoa(0.8275851011276245), "0.8275851011276245");
    }
}
