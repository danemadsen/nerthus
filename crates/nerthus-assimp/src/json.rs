//! A JSON reader with RapidJSON's default (`kParseNoFlags`) behaviour, as
//! Assimp's glTF importers use it: members keep their order and duplicates
//! (lookups take the first), integers carry RapidJSON's Int/Uint/Int64/
//! Uint64 flags, other numbers its "normal precision" double conversion,
//! strings any bytes, and a NUL byte ends the input.

const FLAG_INT: u8 = 1;
const FLAG_UINT: u8 = 2;
const FLAG_INT64: u8 = 4;
const FLAG_UINT64: u8 = 8;
const FLAG_DOUBLE: u8 = 16;

#[derive(Clone, Copy, Debug)]
pub struct Number {
    flags: u8,
    bits: u64,
    double: f64,
}

impl Number {
    /// `GenericValue(unsigned)`.
    fn from_u32(value: u32) -> Self {
        let mut flags = FLAG_UINT | FLAG_UINT64 | FLAG_INT64;
        if value & 0x8000_0000 == 0 {
            flags |= FLAG_INT;
        }
        Number { flags, bits: value as u64, double: 0.0 }
    }

    /// `GenericValue(int)`.
    fn from_i32(value: i32) -> Self {
        let mut flags = FLAG_INT | FLAG_INT64;
        if value >= 0 {
            flags |= FLAG_UINT | FLAG_UINT64;
        }
        Number { flags, bits: value as i64 as u64, double: 0.0 }
    }

    /// `GenericValue(uint64_t)`.
    fn from_u64(value: u64) -> Self {
        let mut flags = FLAG_UINT64;
        if value & 0x8000_0000_0000_0000 == 0 {
            flags |= FLAG_INT64;
        }
        if value & 0xFFFF_FFFF_0000_0000 == 0 {
            flags |= FLAG_UINT;
        }
        if value & 0xFFFF_FFFF_8000_0000 == 0 {
            flags |= FLAG_INT;
        }
        Number { flags, bits: value, double: 0.0 }
    }

    /// `GenericValue(int64_t)`.
    fn from_i64(value: i64) -> Self {
        let mut flags = FLAG_INT64;
        if value >= 0 {
            flags |= FLAG_UINT64;
            if value as u64 & 0xFFFF_FFFF_0000_0000 == 0 {
                flags |= FLAG_UINT;
            }
            if value as u64 & 0xFFFF_FFFF_8000_0000 == 0 {
                flags |= FLAG_INT;
            }
        } else if value >= i32::MIN as i64 {
            flags |= FLAG_INT;
        }
        Number { flags, bits: value as u64, double: 0.0 }
    }

    fn from_f64(value: f64) -> Self {
        Number { flags: FLAG_DOUBLE, bits: 0, double: value }
    }
}

#[derive(Clone, Debug)]
pub enum Value {
    Null,
    Bool(bool),
    Number(Number),
    String(Vec<u8>),
    Array(Vec<Value>),
    Object(Vec<(Vec<u8>, Value)>),
}

impl Value {
    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }

    pub fn is_array(&self) -> bool {
        matches!(self, Value::Array(_))
    }

    pub fn is_string(&self) -> bool {
        matches!(self, Value::String(_))
    }

    pub fn is_bool(&self) -> bool {
        matches!(self, Value::Bool(_))
    }

    pub fn is_number(&self) -> bool {
        matches!(self, Value::Number(_))
    }

    fn has_flag(&self, flag: u8) -> bool {
        match self {
            Value::Number(number) => number.flags & flag != 0,
            _ => false,
        }
    }

    pub fn is_int(&self) -> bool {
        self.has_flag(FLAG_INT)
    }

    pub fn is_uint(&self) -> bool {
        self.has_flag(FLAG_UINT)
    }

    pub fn is_int64(&self) -> bool {
        self.has_flag(FLAG_INT64)
    }

    pub fn is_uint64(&self) -> bool {
        self.has_flag(FLAG_UINT64)
    }

    pub fn get_int(&self) -> i32 {
        match self {
            Value::Number(number) => number.bits as u32 as i32,
            _ => 0,
        }
    }

    pub fn get_uint(&self) -> u32 {
        match self {
            Value::Number(number) => number.bits as u32,
            _ => 0,
        }
    }

    pub fn get_uint64(&self) -> u64 {
        match self {
            Value::Number(number) => number.bits,
            _ => 0,
        }
    }

    /// `GetDouble`: integers converted from their widest flagged type.
    pub fn get_double(&self) -> f64 {
        let Value::Number(number) = self else {
            return 0.0;
        };
        if number.flags & FLAG_DOUBLE != 0 {
            return number.double;
        }
        if number.flags & FLAG_INT != 0 {
            return number.bits as u32 as i32 as f64;
        }
        if number.flags & FLAG_UINT != 0 {
            return number.bits as u32 as f64;
        }
        if number.flags & FLAG_INT64 != 0 {
            return number.bits as i64 as f64;
        }
        number.bits as f64
    }

    pub fn get_bool(&self) -> bool {
        matches!(self, Value::Bool(true))
    }

    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Value::String(bytes) => bytes,
            _ => &[],
        }
    }

    /// The string up to its first NUL (`GetString()` read as a C string).
    pub fn c_str(&self) -> &[u8] {
        let bytes = self.as_bytes();
        let end = bytes.iter().position(|&byte| byte == 0).unwrap_or(bytes.len());
        &bytes[..end]
    }

    pub fn members(&self) -> &[(Vec<u8>, Value)] {
        match self {
            Value::Object(members) => members,
            _ => &[],
        }
    }

    pub fn elements(&self) -> &[Value] {
        match self {
            Value::Array(elements) => elements,
            _ => &[],
        }
    }

    /// `FindMember`: the first member with the name.
    pub fn member(&self, name: &str) -> Option<&Value> {
        self.member_bytes(name.as_bytes())
    }

    /// `FindMember` for a name taken from the document.
    pub fn member_bytes(&self, name: &[u8]) -> Option<&Value> {
        self.members().iter().find(|(key, _)| key.as_slice() == name).map(|(_, value)| value)
    }
}

/// A parse failure (RapidJSON's error code name and offset).
#[derive(Debug)]
pub struct ParseError {
    pub code: &'static str,
    pub offset: usize,
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

include!("pow10.rs");

/// `internal::FastPath`.
fn fast_path(significand: f64, exponent: i32) -> f64 {
    if exponent < -308 {
        return 0.0;
    }
    if exponent >= 0 {
        return significand * POW10[exponent as usize];
    }
    significand / POW10[(-exponent) as usize]
}

/// `internal::StrtodNormalPrecision`.
fn strtod_normal_precision(value: f64, exponent: i32) -> f64 {
    if exponent < -308 {
        let partial = fast_path(value, -308);
        return fast_path(partial, exponent + 308);
    }
    fast_path(value, exponent)
}

impl Reader<'_> {
    fn peek(&self) -> u8 {
        self.bytes.get(self.position).copied().unwrap_or(0)
    }

    fn take(&mut self) -> u8 {
        let byte = self.peek();
        if byte != 0 {
            self.position += 1;
        }
        byte
    }

    fn consume(&mut self, expected: u8) -> bool {
        if self.peek() == expected {
            self.position += 1;
            return true;
        }
        false
    }

    fn error<T>(&self, code: &'static str, offset: usize) -> Result<T, ParseError> {
        Err(ParseError { code, offset })
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), b' ' | b'\n' | b'\r' | b'\t') {
            self.position += 1;
        }
    }

    fn parse_value(&mut self) -> Result<Value, ParseError> {
        match self.peek() {
            b'n' => self.parse_literal(b"null", Value::Null),
            b't' => self.parse_literal(b"true", Value::Bool(true)),
            b'f' => self.parse_literal(b"false", Value::Bool(false)),
            b'"' => {
                let bytes = self.parse_string()?;
                Ok(Value::String(bytes))
            }
            b'{' => self.parse_object(),
            b'[' => self.parse_array(),
            _ => self.parse_number(),
        }
    }

    fn parse_literal(&mut self, literal: &[u8], value: Value) -> Result<Value, ParseError> {
        self.position += 1;
        for &expected in &literal[1..] {
            if !self.consume(expected) {
                return self.error("ValueInvalid", self.position);
            }
        }
        Ok(value)
    }

    fn parse_hex4(&mut self, escape_offset: usize) -> Result<u32, ParseError> {
        let mut codepoint = 0u32;
        for _ in 0..4 {
            let byte = self.peek();
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'A'..=b'F' => byte - b'A' + 10,
                b'a'..=b'f' => byte - b'a' + 10,
                _ => return self.error("StringUnicodeEscapeInvalidHex", escape_offset),
            };
            codepoint = (codepoint << 4) + digit as u32;
            self.position += 1;
        }
        Ok(codepoint)
    }

    fn parse_string(&mut self) -> Result<Vec<u8>, ParseError> {
        self.position += 1;
        let mut output = Vec::new();
        loop {
            let byte = self.peek();
            if byte == b'\\' {
                let escape_offset = self.position;
                self.position += 1;
                let escaped = self.peek();
                let simple = match escaped {
                    b'"' => Some(b'"'),
                    b'\\' => Some(b'\\'),
                    b'/' => Some(b'/'),
                    b'b' => Some(8),
                    b'f' => Some(12),
                    b'n' => Some(b'\n'),
                    b'r' => Some(b'\r'),
                    b't' => Some(b'\t'),
                    _ => None,
                };
                if let Some(replacement) = simple {
                    self.position += 1;
                    output.push(replacement);
                    continue;
                }
                if escaped != b'u' {
                    return self.error("StringEscapeInvalid", escape_offset);
                }
                self.position += 1;
                let mut codepoint = self.parse_hex4(escape_offset)?;
                if (0xD800..=0xDFFF).contains(&codepoint) {
                    if codepoint > 0xDBFF {
                        return self.error("StringUnicodeSurrogateInvalid", escape_offset);
                    }
                    if !self.consume(b'\\') || !self.consume(b'u') {
                        return self.error("StringUnicodeSurrogateInvalid", escape_offset);
                    }
                    let low = self.parse_hex4(escape_offset)?;
                    if !(0xDC00..=0xDFFF).contains(&low) {
                        return self.error("StringUnicodeSurrogateInvalid", escape_offset);
                    }
                    codepoint = (((codepoint - 0xD800) << 10) | (low - 0xDC00)) + 0x10000;
                }
                encode_utf8(&mut output, codepoint);
                continue;
            }
            if byte == b'"' {
                self.position += 1;
                return Ok(output);
            }
            if byte < 0x20 {
                if byte == 0 {
                    return self.error("StringMissQuotationMark", self.position);
                }
                return self.error("StringInvalidEncoding", self.position);
            }
            output.push(byte);
            self.position += 1;
        }
    }

    fn parse_object(&mut self) -> Result<Value, ParseError> {
        self.position += 1;
        self.skip_whitespace();
        let mut members = Vec::new();
        if self.consume(b'}') {
            return Ok(Value::Object(members));
        }
        loop {
            if self.peek() != b'"' {
                return self.error("ObjectMissName", self.position);
            }
            let name = self.parse_string()?;
            self.skip_whitespace();
            if !self.consume(b':') {
                return self.error("ObjectMissColon", self.position);
            }
            self.skip_whitespace();
            let value = self.parse_value()?;
            members.push((name, value));
            self.skip_whitespace();
            match self.peek() {
                b',' => {
                    self.position += 1;
                    self.skip_whitespace();
                }
                b'}' => {
                    self.position += 1;
                    return Ok(Value::Object(members));
                }
                _ => return self.error("ObjectMissCommaOrCurlyBracket", self.position),
            }
        }
    }

    fn parse_array(&mut self) -> Result<Value, ParseError> {
        self.position += 1;
        self.skip_whitespace();
        let mut elements = Vec::new();
        if self.consume(b']') {
            return Ok(Value::Array(elements));
        }
        loop {
            let value = self.parse_value()?;
            elements.push(value);
            self.skip_whitespace();
            match self.peek() {
                b',' => {
                    self.position += 1;
                    self.skip_whitespace();
                }
                b']' => {
                    self.position += 1;
                    return Ok(Value::Array(elements));
                }
                _ => return self.error("ArrayMissCommaOrSquareBracket", self.position),
            }
        }
    }

    fn is_digit(&self) -> bool {
        self.peek().is_ascii_digit()
    }

    fn digit(&mut self) -> u8 {
        self.take() - b'0'
    }

    /// `ParseNumber` without the NaN/Infinity and full-precision options.
    fn parse_number(&mut self) -> Result<Value, ParseError> {
        let start = self.position;
        let minus = self.consume(b'-');
        let mut small = 0u32;
        let mut wide = 0u64;
        let mut use_64bit = false;
        let mut significand_digits = 0;
        if self.peek() == b'0' {
            self.position += 1;
        } else if (b'1'..=b'9').contains(&self.peek()) {
            small = self.digit() as u32;
            let (limit, last) = if minus { (214_748_364u32, b'8') } else { (429_496_729u32, b'5') };
            while self.is_digit() {
                if small >= limit && (small != limit || self.peek() > last) {
                    wide = small as u64;
                    use_64bit = true;
                    break;
                }
                small = small * 10 + self.digit() as u32;
                significand_digits += 1;
            }
        } else {
            return self.error("ValueInvalid", self.position);
        }

        let mut use_double = false;
        let mut double = 0.0f64;
        if use_64bit {
            let (limit, last) = if minus { (0x0CCC_CCCC_CCCC_CCCCu64, b'8') } else { (0x1999_9999_9999_9999u64, b'5') };
            while self.is_digit() {
                if wide >= limit && (wide != limit || self.peek() > last) {
                    double = wide as f64;
                    use_double = true;
                    break;
                }
                wide = wide * 10 + self.digit() as u64;
                significand_digits += 1;
            }
        }
        if use_double {
            while self.is_digit() {
                double = double * 10.0 + self.digit() as f64;
            }
        }

        let mut fraction_exponent = 0i32;
        if self.consume(b'.') {
            if !self.is_digit() {
                return self.error("NumberMissFraction", self.position);
            }
            if !use_double {
                if !use_64bit {
                    wide = small as u64;
                }
                while self.is_digit() {
                    if wide > 0x1F_FFFF_FFFF_FFFF {
                        break;
                    }
                    wide = wide * 10 + self.digit() as u64;
                    fraction_exponent -= 1;
                    if wide != 0 {
                        significand_digits += 1;
                    }
                }
                double = wide as f64;
                use_double = true;
            }
            while self.is_digit() {
                if significand_digits < 17 {
                    double = double * 10.0 + self.digit() as f64;
                    fraction_exponent -= 1;
                    if double > 0.0 {
                        significand_digits += 1;
                    }
                } else {
                    self.position += 1;
                }
            }
        }

        let mut exponent = 0i32;
        if self.consume(b'e') || self.consume(b'E') {
            if !use_double {
                double = if use_64bit { wide as f64 } else { small as f64 };
                use_double = true;
            }
            let mut exponent_minus = false;
            if self.consume(b'+') {
            } else if self.consume(b'-') {
                exponent_minus = true;
            }
            if !self.is_digit() {
                return self.error("NumberMissExponent", self.position);
            }
            exponent = self.digit() as i32;
            if exponent_minus {
                let max_exponent = (fraction_exponent + 2_147_483_639) / 10;
                while self.is_digit() {
                    exponent = exponent * 10 + self.digit() as i32;
                    if exponent > max_exponent {
                        while self.is_digit() {
                            self.position += 1;
                        }
                    }
                }
                exponent = -exponent;
            } else {
                let max_exponent = 308 - fraction_exponent;
                while self.is_digit() {
                    exponent = exponent * 10 + self.digit() as i32;
                    if exponent > max_exponent {
                        return self.error("NumberTooBig", start);
                    }
                }
            }
        }

        if use_double {
            let value = strtod_normal_precision(double, exponent + fraction_exponent);
            if value > f64::MAX {
                return self.error("NumberTooBig", start);
            }
            let signed = if minus { -value } else { value };
            return Ok(Value::Number(Number::from_f64(signed)));
        }
        let number = match (use_64bit, minus) {
            (true, true) => Number::from_i64((!wide).wrapping_add(1) as i64),
            (true, false) => Number::from_u64(wide),
            (false, true) => Number::from_i32((!small).wrapping_add(1) as i32),
            (false, false) => Number::from_u32(small),
        };
        Ok(Value::Number(number))
    }
}

/// `UTF8::Encode`.
fn encode_utf8(output: &mut Vec<u8>, codepoint: u32) {
    if codepoint <= 0x7F {
        output.push(codepoint as u8);
    } else if codepoint <= 0x7FF {
        output.push(0xC0 | ((codepoint >> 6) & 0xFF) as u8);
        output.push(0x80 | (codepoint & 0x3F) as u8);
    } else if codepoint <= 0xFFFF {
        output.push(0xE0 | ((codepoint >> 12) & 0xFF) as u8);
        output.push(0x80 | ((codepoint >> 6) & 0x3F) as u8);
        output.push(0x80 | (codepoint & 0x3F) as u8);
    } else {
        output.push(0xF0 | ((codepoint >> 18) & 0xFF) as u8);
        output.push(0x80 | ((codepoint >> 12) & 0x3F) as u8);
        output.push(0x80 | ((codepoint >> 6) & 0x3F) as u8);
        output.push(0x80 | (codepoint & 0x3F) as u8);
    }
}

/// `Document::ParseInsitu` with default flags.
pub fn parse(bytes: &[u8]) -> Result<Value, ParseError> {
    let mut reader = Reader { bytes, position: 0 };
    reader.skip_whitespace();
    if reader.peek() == 0 {
        return reader.error("DocumentEmpty", reader.position);
    }
    let value = reader.parse_value()?;
    reader.skip_whitespace();
    if reader.peek() != 0 {
        return reader.error("DocumentRootNotSingular", reader.position);
    }
    Ok(value)
}
