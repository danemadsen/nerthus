//! ImageMagick morphology (`morphology.c`): kernel strings, the built-in
//! kernels and `MorphologyApply` with its primitives.
//!
//! Kernel values are kept as IM defines them. Additive kernels (Distance)
//! hold quantum units, so pixels are scaled to quantum units for those sums.

use std::fmt;

use rayon::prelude::*;

use crate::image::{Image, VirtualSource, QUANTUM_RANGE};

const MAGICK_EPSILON: f64 = 1.0e-12;

/// The morphology methods the nodes use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MorphologyMethod {
    Dilate,
    Erode,
    Open,
    Close,
    Smooth,
    TopHat,
    BottomHat,
    EdgeIn,
    EdgeOut,
    Edge,
    HitAndMiss,
    Thinning,
    Thicken,
    Distance,
}

/// A kernel string IM could not parse (`AcquireKernelInfo` returned NULL).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelError(pub String);

impl fmt::Display for KernelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid morphology kernel '{}'", self.0)
    }
}

impl std::error::Error for KernelError {}

/// IM `KernelInfoType` for the kernels implemented here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KernelKind {
    UserDefined,
    Unity,
    Binomial,
    Diamond,
    Square,
    Rectangle,
    Octagon,
    Disk,
    Plus,
    Cross,
    Ring,
    Peaks,
    Sobel,
    Roberts,
    Prewitt,
    Corners,
    ThinSe,
    Chebyshev,
    Manhattan,
    Octagonal,
    Euclidean,
}

impl KernelKind {
    fn from_name(name: &str) -> Option<Self> {
        let lower = name.to_ascii_lowercase();
        let kind = match lower.as_str() {
            "unity" => KernelKind::Unity,
            "binomial" => KernelKind::Binomial,
            "diamond" => KernelKind::Diamond,
            "square" => KernelKind::Square,
            "rectangle" => KernelKind::Rectangle,
            "octagon" => KernelKind::Octagon,
            "disk" => KernelKind::Disk,
            "plus" => KernelKind::Plus,
            "cross" => KernelKind::Cross,
            "ring" => KernelKind::Ring,
            "peaks" => KernelKind::Peaks,
            "sobel" => KernelKind::Sobel,
            "roberts" => KernelKind::Roberts,
            "prewitt" => KernelKind::Prewitt,
            "corners" => KernelKind::Corners,
            "thinse" => KernelKind::ThinSe,
            "chebyshev" => KernelKind::Chebyshev,
            "manhattan" => KernelKind::Manhattan,
            "octagonal" => KernelKind::Octagonal,
            "euclidean" => KernelKind::Euclidean,
            _ => return None,
        };
        Some(kind)
    }
}

/// One IM `KernelInfo` (a kernel list is a `Vec<MorphKernel>`).
#[derive(Clone, Debug)]
pub struct MorphKernel {
    pub kind: KernelKind,
    pub width: usize,
    pub height: usize,
    pub x: isize,
    pub y: isize,
    /// Row-major values; NaN marks elements outside the neighbourhood.
    pub values: Vec<f64>,
    pub angle: f64,
}

impl MorphKernel {
    fn new(kind: KernelKind, width: usize, height: usize) -> Self {
        Self {
            kind,
            width,
            height,
            x: (width as isize - 1) / 2,
            y: (height as isize - 1) / 2,
            values: vec![0.0; width * height],
            angle: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Geometry and token parsing
// ---------------------------------------------------------------------------

const X_VALUE: u32 = 0x0001;
const Y_VALUE: u32 = 0x0002;
const WIDTH_VALUE: u32 = 0x0004;
const HEIGHT_VALUE: u32 = 0x0008;
const XI_NEGATIVE: u32 = 0x0020;
const PSI_NEGATIVE: u32 = 0x0040;
const PERCENT_VALUE: u32 = 0x1000;
const ASPECT_VALUE: u32 = 0x2000;
const LESS_VALUE: u32 = 0x4000;
const GREATER_VALUE: u32 = 0x8000;
const AREA_VALUE: u32 = 0x20000;

/// IM `GeometryInfo` plus the flags `ParseGeometry` returned.
#[derive(Clone, Copy, Debug, Default)]
struct GeometryInfo {
    rho: f64,
    sigma: f64,
    xi: f64,
    psi: f64,
    flags: u32,
}

/// C `strtod` on the start of `text`: the value and the bytes consumed
/// (0 when no number was found).
fn parse_c_double(text: &[u8]) -> (f64, usize) {
    let mut index = 0;
    while index < text.len() && text[index].is_ascii_whitespace() {
        index += 1;
    }
    let start = index;
    if index < text.len() && (text[index] == b'+' || text[index] == b'-') {
        index += 1;
    }
    let digits_start = index;
    while index < text.len() && text[index].is_ascii_digit() {
        index += 1;
    }
    let mut digit_count = index - digits_start;
    if index < text.len() && text[index] == b'.' {
        index += 1;
        let fraction_start = index;
        while index < text.len() && text[index].is_ascii_digit() {
            index += 1;
        }
        digit_count += index - fraction_start;
    }
    if digit_count == 0 {
        return (0.0, 0);
    }
    if index < text.len() && (text[index] == b'e' || text[index] == b'E') {
        let mut exponent_end = index + 1;
        if exponent_end < text.len() && (text[exponent_end] == b'+' || text[exponent_end] == b'-') {
            exponent_end += 1;
        }
        let exponent_digits = exponent_end;
        while exponent_end < text.len() && text[exponent_end].is_ascii_digit() {
            exponent_end += 1;
        }
        if exponent_end > exponent_digits {
            index = exponent_end;
        }
    }
    let number = std::str::from_utf8(&text[start..index]).unwrap_or("0");
    (number.parse().unwrap_or(0.0), index)
}

/// The `sscanf("%lf%*[ ,]%lf%*[ ,]%lf%*[ ,]%lf")` coordinate special case.
fn scan_four_numbers(text: &[u8]) -> Option<[f64; 4]> {
    let mut values = [0.0; 4];
    let mut position = 0;
    for (index, value) in values.iter_mut().enumerate() {
        if index > 0 {
            let separator_start = position;
            while position < text.len() && (text[position] == b' ' || text[position] == b',') {
                position += 1;
            }
            if position == separator_start {
                return None;
            }
        }
        let (number, consumed) = parse_c_double(&text[position..]);
        if consumed == 0 {
            return None;
        }
        *value = number;
        position += consumed;
    }
    Some(values)
}

/// IM `ParseGeometry` (without the chi value, which kernels never use).
fn parse_geometry(geometry: &str) -> GeometryInfo {
    let mut info = GeometryInfo::default();
    if geometry.is_empty() {
        return info;
    }
    if let Some([rho, sigma, xi, psi]) = scan_four_numbers(geometry.as_bytes()) {
        info.rho = rho;
        info.sigma = sigma;
        info.xi = xi;
        info.psi = psi;
        info.flags = WIDTH_VALUE | HEIGHT_VALUE | X_VALUE | Y_VALUE;
        return info;
    }
    let mut pedantic = Vec::with_capacity(geometry.len());
    for &byte in geometry.as_bytes() {
        match byte {
            b'%' => info.flags |= PERCENT_VALUE,
            b'!' => info.flags |= ASPECT_VALUE,
            b'<' => info.flags |= LESS_VALUE,
            b'>' => info.flags |= GREATER_VALUE,
            b'@' => info.flags |= AREA_VALUE,
            b'^' | b'(' | b')' => {}
            _ if byte.is_ascii_whitespace() => {}
            b'x' | b'X' | b'-' | b'+' | b',' | b'/' | b'e' | b'E' | b'.' | b':' => pedantic.push(byte),
            _ if byte.is_ascii_digit() => pedantic.push(byte),
            _ => return GeometryInfo::default(),
        }
    }
    let text = pedantic.as_slice();
    if text.is_empty() {
        return info;
    }
    let is_separator = |byte: Option<&u8>| matches!(byte, Some(b'x') | Some(b'X') | Some(b':') | Some(b',') | Some(b'/'));
    let mut position = 0;
    let (_, first_length) = parse_c_double(text);
    let after_first = text.get(first_length);
    if is_separator(after_first) || after_first.is_none() {
        let (value, consumed) = parse_c_double(text);
        if consumed > 0 {
            info.flags |= WIDTH_VALUE;
            info.rho = value;
        }
        position = consumed;
    }
    let separator = text.get(position).copied();
    if is_separator(separator.as_ref()) {
        position += 1;
        let next = text.get(position).copied();
        let size_separator = matches!(separator, Some(b'x') | Some(b'X') | Some(b':'));
        if !size_separator || (next != Some(b'+') && next != Some(b'-')) {
            let (value, consumed) = parse_c_double(&text[position..]);
            if consumed > 0 {
                info.flags |= HEIGHT_VALUE;
                info.sigma = value;
            }
            position += consumed;
        }
    }
    let offset_start = |byte: Option<&u8>| matches!(byte, Some(b'+') | Some(b'-') | Some(b',') | Some(b'/') | Some(b':'));
    if offset_start(text.get(position)) {
        if matches!(text.get(position), Some(b',') | Some(b'/') | Some(b':')) {
            position += 1;
        }
        while matches!(text.get(position), Some(b'+') | Some(b'-')) {
            if text[position] == b'-' {
                info.flags ^= XI_NEGATIVE;
            }
            position += 1;
        }
        let (value, consumed) = parse_c_double(&text[position..]);
        if consumed > 0 {
            info.flags |= X_VALUE;
            info.xi = if info.flags & XI_NEGATIVE != 0 { -value } else { value };
        }
        position += consumed;
        if offset_start(text.get(position)) {
            if matches!(text.get(position), Some(b',') | Some(b'/') | Some(b':')) {
                position += 1;
            }
            while matches!(text.get(position), Some(b'+') | Some(b'-')) {
                if text[position] == b'-' {
                    info.flags ^= PSI_NEGATIVE;
                }
                position += 1;
            }
            let (value, consumed) = parse_c_double(&text[position..]);
            if consumed > 0 {
                info.flags |= Y_VALUE;
                info.psi = if info.flags & PSI_NEGATIVE != 0 { -value } else { value };
            }
        }
    }
    if text.contains(&b':') {
        if info.flags & HEIGHT_VALUE != 0 {
            info.rho *= crate::image::perceptible_reciprocal(info.sigma);
        }
        info.sigma = 1.0;
        if info.flags & X_VALUE != 0 && info.xi == 0.0 {
            info.sigma = 2.0;
        }
    }
    let negative_height = info.flags & WIDTH_VALUE != 0 && info.flags & HEIGHT_VALUE == 0 && info.flags & X_VALUE != 0 && info.flags & XI_NEGATIVE != 0;
    if negative_height && info.flags & Y_VALUE == 0 {
        info.sigma = info.xi;
        info.xi = 0.0;
        info.flags |= HEIGHT_VALUE;
        info.flags &= !X_VALUE;
    }
    info
}

/// Value tokens of a kernel array: numbers, "nan" or "-", split on spaces
/// and commas.
fn value_tokens(text: &str) -> Vec<&str> {
    let separators = |character: char| character.is_whitespace() || character == ',';
    let mut tokens = Vec::new();
    for token in text.split(separators) {
        if !token.is_empty() {
            tokens.push(token);
        }
    }
    tokens
}

// ---------------------------------------------------------------------------
// Kernel construction
// ---------------------------------------------------------------------------

/// IM `ParseKernelArray`: `"WxH[+x+y]: values"` or an old style odd square
/// list of values.
fn parse_kernel_array(definition: &str) -> Result<Vec<MorphKernel>, KernelError> {
    let error = || KernelError(definition.to_string());
    let mut kernel = MorphKernel::new(KernelKind::UserDefined, 1, 1);
    let mut flags = 0;
    let values_text = match definition.find(':') {
        Some(colon) => {
            let mut info = parse_geometry(&definition[..colon]);
            flags = info.flags;
            if info.flags & WIDTH_VALUE == 0 {
                info.rho = info.sigma;
            }
            if info.rho < 1.0 {
                info.rho = 1.0;
            }
            if info.sigma < 1.0 {
                info.sigma = info.rho;
            }
            kernel.width = info.rho as usize;
            kernel.height = info.sigma as usize;
            if info.xi < 0.0 || info.psi < 0.0 {
                return Err(error());
            }
            kernel.x = if info.flags & X_VALUE != 0 { info.xi as isize } else { (kernel.width as isize - 1) / 2 };
            kernel.y = if info.flags & Y_VALUE != 0 { info.psi as isize } else { (kernel.height as isize - 1) / 2 };
            if kernel.x >= kernel.width as isize || kernel.y >= kernel.height as isize {
                return Err(error());
            }
            &definition[colon + 1..]
        }
        None => {
            let trimmed = definition.trim_start_matches(|character: char| character.is_whitespace() || character == '\'');
            let count = value_tokens(trimmed).len();
            kernel.width = ((count as f64) + 1.0).sqrt() as usize;
            kernel.height = kernel.width;
            kernel.x = (kernel.width as isize - 1) / 2;
            kernel.y = kernel.x;
            trimmed
        }
    };
    let tokens = value_tokens(values_text.trim_end_matches('\''));
    let count = kernel.width * kernel.height;
    if tokens.len() != count {
        return Err(error());
    }
    let mut any_value = false;
    kernel.values = Vec::with_capacity(count);
    for token in tokens {
        if token.eq_ignore_ascii_case("nan") || token == "-" {
            kernel.values.push(f64::NAN);
        } else {
            let (value, _) = parse_c_double(token.as_bytes());
            kernel.values.push(value);
            any_value = true;
        }
    }
    if !any_value {
        return Err(error());
    }
    let mut kernels = vec![kernel];
    if flags & AREA_VALUE != 0 {
        expand_rotate(&mut kernels, 45.0);
    } else if flags & GREATER_VALUE != 0 {
        expand_rotate(&mut kernels, 90.0);
    }
    Ok(kernels)
}

/// An odd square shape kernel: `value` inside, NaN outside.
fn shape_kernel(kind: KernelKind, width: usize, value: f64, inside: impl Fn(isize, isize) -> bool) -> MorphKernel {
    let mut kernel = MorphKernel::new(kind, width, width);
    let half = kernel.x;
    let mut index = 0;
    for v in -half..=half {
        for u in -half..=half {
            kernel.values[index] = if inside(u, v) { value } else { f64::NAN };
            index += 1;
        }
    }
    kernel
}

/// IM's integer factorial (`fact`).
fn factorial(n: usize) -> usize {
    let mut result = 1usize;
    for factor in 2..=n {
        result = result.wrapping_mul(factor);
    }
    result
}

/// A distance measuring kernel; radii below `minimum_radius` use that radius.
fn distance_kernel(kind: KernelKind, args: &GeometryInfo, minimum_radius: usize, distance: impl Fn(isize, isize) -> f64) -> MorphKernel {
    let width = if args.rho < minimum_radius as f64 { minimum_radius * 2 + 1 } else { (args.rho as usize) * 2 + 1 };
    let mut kernel = MorphKernel::new(kind, width, width);
    let half = kernel.x;
    let mut index = 0;
    for v in -half..=half {
        for u in -half..=half {
            kernel.values[index] = args.sigma * distance(u, v);
            index += 1;
        }
    }
    kernel
}

/// IM `AcquireKernelBuiltIn` for the supported kinds.
fn built_in_kernel(kind: KernelKind, args: &GeometryInfo, name: &str) -> Result<Vec<MorphKernel>, KernelError> {
    let odd_width = |default: usize| if args.rho < 1.0 { default } else { (args.rho as usize) * 2 + 1 };
    let kernel = match kind {
        KernelKind::Unity => {
            let mut kernel = MorphKernel::new(kind, 1, 1);
            kernel.values[0] = args.rho;
            kernel
        }
        KernelKind::Binomial => {
            let width = odd_width(3);
            let mut kernel = MorphKernel::new(kind, width, width);
            let order = factorial(width - 1);
            let mut index = 0;
            for v in 0..width {
                let row_divisor = factorial(v).wrapping_mul(factorial(width - v - 1));
                let alpha = order / row_divisor;
                for u in 0..width {
                    let divisor = factorial(u).wrapping_mul(factorial(width - u - 1));
                    kernel.values[index] = (alpha.wrapping_mul(order) / divisor) as f64;
                    index += 1;
                }
            }
            kernel
        }
        KernelKind::Diamond => {
            let width = odd_width(3);
            let half = (width as isize - 1) / 2;
            shape_kernel(kind, width, args.sigma, |u, v| u.abs() + v.abs() <= half)
        }
        KernelKind::Square => {
            let width = if args.rho < 1.0 { 3 } else { (2.0 * args.rho + 1.0) as usize };
            let mut kernel = MorphKernel::new(kind, width, width);
            kernel.values.fill(args.sigma);
            kernel
        }
        KernelKind::Rectangle => {
            if args.rho < 1.0 || args.sigma < 1.0 {
                return Err(KernelError(name.to_string()));
            }
            let width = args.rho as usize;
            let height = args.sigma as usize;
            if args.xi < 0.0 || args.xi > width as f64 || args.psi < 0.0 || args.psi > height as f64 {
                return Err(KernelError(name.to_string()));
            }
            let mut kernel = MorphKernel::new(kind, width, height);
            kernel.x = args.xi as isize;
            kernel.y = args.psi as isize;
            kernel.values.fill(1.0);
            kernel
        }
        KernelKind::Octagon => {
            let width = if args.rho < 1.0 { 5 } else { (args.rho as usize) * 2 + 1 };
            let half = (width as isize - 1) / 2;
            shape_kernel(kind, width, args.sigma, |u, v| u.abs() + v.abs() <= half + half / 2)
        }
        KernelKind::Disk => {
            let (width, limit) = if args.rho < 0.4 { (9, 18) } else { ((args.rho.abs() as usize) * 2 + 1, (args.rho * args.rho) as isize) };
            shape_kernel(kind, width, args.sigma, |u, v| u * u + v * v <= limit)
        }
        KernelKind::Plus => shape_kernel(kind, odd_width(5), args.sigma, |u, v| u == 0 || v == 0),
        KernelKind::Cross => shape_kernel(kind, odd_width(5), args.sigma, |u, v| u == v || u == -v),
        KernelKind::Ring | KernelKind::Peaks => {
            let (mut width, mut inner, mut outer) = if args.rho < args.sigma {
                ((args.sigma as usize) * 2 + 1, (args.rho * args.rho) as isize, (args.sigma * args.sigma) as isize)
            } else {
                ((args.rho as usize) * 2 + 1, (args.sigma * args.sigma) as isize, (args.rho * args.rho) as isize)
            };
            if outer <= 0 {
                width = 7;
                inner = 7;
                outer = 11;
            }
            let scale = if kind == KernelKind::Peaks { 0.0 } else { args.xi as isize as f64 };
            let mut kernel = shape_kernel(kind, width, scale, |u, v| {
                let radius = u * u + v * v;
                inner < radius && radius <= outer
            });
            if kind == KernelKind::Peaks {
                let center = (kernel.y as usize) * kernel.width + kernel.x as usize;
                kernel.values[center] = 1.0;
            }
            kernel
        }
        KernelKind::Sobel | KernelKind::Roberts | KernelKind::Prewitt => {
            let array = match kind {
                KernelKind::Sobel => "3: 1,0,-1  2,0,-2  1,0,-1",
                KernelKind::Roberts => "3: 0,0,0  1,-1,0  0,0,0",
                _ => "3: 1,0,-1  1,0,-1  1,0,-1",
            };
            let mut kernel = parse_kernel_array(array)?.remove(0);
            kernel.kind = kind;
            rotate_kernel(&mut kernel, args.rho);
            kernel
        }
        KernelKind::ThinSe => {
            let array = match args.rho as i32 {
                41 => "3: -,-,1  0,-,1  -,-,1",
                42 => "3: -,-,1  0,-,1  -,0,-",
                43 => "3: -,0,-  0,-,1  -,-,1",
                44 => "3: -,0,-  0,-,1  -,0,-",
                45 => "3: -,0,1  0,-,1  -,0,-",
                46 => "3: -,0,-  0,-,1  -,0,1",
                47 => "3: -,1,1  0,-,1  -,0,-",
                48 => "3: -,-,1  0,-,1  0,-,1",
                49 => "3: 0,-,1  0,-,1  -,-,1",
                81 => "3: -,1,-  0,-,1  -,1,-",
                82 => "3: -,1,-  0,-,1  0,-,-",
                83 => "3: 0,-,-  0,-,1  -,1,-",
                84 => "3: 0,-,-  0,-,1  0,-,-",
                85 => "3: 0,-,1  0,-,1  0,-,-",
                86 => "3: 0,-,-  0,-,1  0,-,1",
                87 => "3: -,1,-  0,-,1  0,0,-",
                88 => "3: -,1,-  0,-,1  0,1,-",
                89 => "3: 0,1,-  0,-,1  -,1,-",
                423 => "3: -,-,1  0,-,-  -,0,-",
                823 => "3: -,1,-  -,-,1  0,-,-",
                481 => "3: -,1,1  0,-,1  0,0,-",
                _ => "3: 0,-,1  0,-,1  0,-,1",
            };
            let mut kernel = parse_kernel_array(array)?.remove(0);
            kernel.kind = kind;
            rotate_kernel(&mut kernel, args.sigma);
            kernel
        }
        KernelKind::Corners => {
            let mut kernels = parse_kernel_name("ThinSE:87")?;
            for kernel in kernels.iter_mut() {
                kernel.kind = KernelKind::Corners;
            }
            expand_rotate(&mut kernels, 90.0);
            return Ok(kernels);
        }
        KernelKind::Chebyshev => distance_kernel(kind, args, 1, |u, v| (u.abs() as f64).max(v.abs() as f64)),
        KernelKind::Manhattan => distance_kernel(kind, args, 1, |u, v| (u.abs() + v.abs()) as f64),
        KernelKind::Octagonal => distance_kernel(kind, args, 2, |u, v| {
            let r1 = (u.abs() as f64).max(v.abs() as f64);
            let r2 = (((u.abs() + v.abs() + 1) as f64) / 1.5).floor();
            r1.max(r2)
        }),
        KernelKind::Euclidean => distance_kernel(kind, args, 1, |u, v| ((u * u + v * v) as f64).sqrt()),
        KernelKind::UserDefined => return Err(KernelError(name.to_string())),
    };
    Ok(vec![kernel])
}

/// IM `ParseKernelName`: a named kernel with optional geometry arguments.
fn parse_kernel_name(definition: &str) -> Result<Vec<MorphKernel>, KernelError> {
    let error = || KernelError(definition.to_string());
    let trimmed = definition.trim_start();
    let name_end = trimmed.find(|character: char| character.is_whitespace() || character == '=' || character == ',' || character == ':' || character == ';').unwrap_or(trimmed.len());
    let name = &trimmed[..name_end];
    let kind = KernelKind::from_name(name).ok_or_else(error)?;
    let rest = trimmed[name_end..].trim_start_matches(|character: char| character.is_whitespace() || character == ',' || character == ':');
    let arguments = match rest.find(';') {
        Some(end) => &rest[..end],
        None => rest,
    };
    let mut args = parse_geometry(arguments);
    let flags = args.flags;
    match kind {
        KernelKind::Unity => {
            if flags & WIDTH_VALUE == 0 {
                args.rho = 1.0;
            }
        }
        KernelKind::Square | KernelKind::Diamond | KernelKind::Octagon | KernelKind::Disk | KernelKind::Plus | KernelKind::Cross => {
            if flags & HEIGHT_VALUE == 0 {
                args.sigma = 1.0;
            }
        }
        KernelKind::Ring => {
            if flags & X_VALUE == 0 {
                args.xi = 1.0;
            }
        }
        KernelKind::Rectangle => {
            if flags & WIDTH_VALUE == 0 {
                args.rho = args.sigma;
            }
            if args.rho < 1.0 {
                args.rho = 3.0;
            }
            if args.sigma < 1.0 {
                args.sigma = args.rho;
            }
            if flags & X_VALUE == 0 {
                args.xi = ((args.rho as isize - 1) / 2) as f64;
            }
            if flags & Y_VALUE == 0 {
                args.psi = ((args.sigma as isize - 1) / 2) as f64;
            }
        }
        KernelKind::Chebyshev | KernelKind::Manhattan | KernelKind::Octagonal | KernelKind::Euclidean => {
            if flags & HEIGHT_VALUE == 0 {
                args.sigma = 100.0;
            } else if flags & ASPECT_VALUE != 0 {
                args.sigma = QUANTUM_RANGE / (args.sigma + 1.0);
            } else if flags & PERCENT_VALUE != 0 {
                args.sigma *= QUANTUM_RANGE / 100.0;
            }
        }
        _ => {}
    }
    let mut kernels = built_in_kernel(kind, &args, definition)?;
    if kernels.len() == 1 {
        if flags & AREA_VALUE != 0 {
            expand_rotate(&mut kernels, 45.0);
        } else if flags & GREATER_VALUE != 0 {
            expand_rotate(&mut kernels, 90.0);
        }
    }
    Ok(kernels)
}

/// IM `AcquireKernelInfo`: a `;` separated list of named kernels and
/// kernel arrays (Magick.NET passes `"Name:arguments"` or a user kernel).
pub fn parse_kernel(definition: &str) -> Result<Vec<MorphKernel>, KernelError> {
    let mut kernels = Vec::new();
    for part in definition.split(';') {
        let token = part.trim_start();
        if token.is_empty() {
            continue;
        }
        let parsed = if token.starts_with(|character: char| character.is_ascii_alphabetic()) { parse_kernel_name(token)? } else { parse_kernel_array(token)? };
        kernels.extend(parsed);
    }
    if kernels.is_empty() {
        return Err(KernelError(definition.to_string()));
    }
    Ok(kernels)
}

// ---------------------------------------------------------------------------
// Kernel rotation
// ---------------------------------------------------------------------------

/// IM `RotateKernelInfo` for one kernel.
fn rotate_kernel(kernel: &mut MorphKernel, angle: f64) {
    let mut angle = angle % 360.0;
    if angle < 0.0 {
        angle += 360.0;
    }
    if 337.5 < angle || angle <= 22.5 {
        return;
    }
    match kernel.kind {
        KernelKind::Disk | KernelKind::Peaks | KernelKind::Chebyshev | KernelKind::Manhattan | KernelKind::Euclidean => return,
        KernelKind::Square | KernelKind::Diamond | KernelKind::Plus | KernelKind::Cross => return,
        _ => {}
    }
    let quarter_remainder = angle % 90.0;
    if 22.5 < quarter_remainder && quarter_remainder <= 67.5
        && kernel.width == 3 && kernel.height == 3 {
            let values = &mut kernel.values;
            let first = values[0];
            values[0] = values[3];
            values[3] = values[6];
            values[6] = values[7];
            values[7] = values[8];
            values[8] = values[5];
            values[5] = values[2];
            values[2] = values[1];
            values[1] = first;
            if kernel.x != 1 || kernel.y != 1 {
                let mut x = kernel.x - 1;
                let mut y = kernel.y - 1;
                if x == y {
                    x = 0;
                } else if x == 0 {
                    x = -y;
                } else if x == -y {
                    y = 0;
                } else if y == 0 {
                    y = x;
                }
                kernel.x = x + 1;
                kernel.y = y + 1;
            }
            angle = (angle + 315.0) % 360.0;
            kernel.angle = (kernel.angle + 45.0) % 360.0;
        }
    let half_remainder = angle % 180.0;
    if 45.0 < half_remainder && half_remainder <= 135.0 {
        if kernel.width == 1 || kernel.height == 1 {
            std::mem::swap(&mut kernel.width, &mut kernel.height);
            std::mem::swap(&mut kernel.x, &mut kernel.y);
            if kernel.width == 1 {
                angle = (angle + 270.0) % 360.0;
                kernel.angle = (kernel.angle + 90.0) % 360.0;
            } else {
                angle = (angle + 90.0) % 360.0;
                kernel.angle = (kernel.angle + 270.0) % 360.0;
            }
        } else if kernel.width == kernel.height {
            let width = kernel.width;
            let values = &mut kernel.values;
            let mut i = 0usize;
            let mut x = width - 1;
            while i <= x {
                let mut j = 0usize;
                let mut y = width - 1;
                while j < y {
                    let saved = values[i + j * width];
                    values[i + j * width] = values[j + x * width];
                    values[j + x * width] = values[x + y * width];
                    values[x + y * width] = values[y + i * width];
                    values[y + i * width] = saved;
                    j += 1;
                    y -= 1;
                }
                i += 1;
                if x == 0 {
                    break;
                }
                x -= 1;
            }
            let x = kernel.x * 2 - kernel.width as isize + 1;
            let y = kernel.y * 2 - kernel.height as isize + 1;
            kernel.x = (-y + kernel.width as isize - 1) / 2;
            kernel.y = (x + kernel.height as isize - 1) / 2;
            angle = (angle + 270.0) % 360.0;
            kernel.angle = (kernel.angle + 90.0) % 360.0;
        }
    }
    if 135.0 < angle && angle <= 225.0 {
        kernel.values.reverse();
        kernel.x = kernel.width as isize - kernel.x - 1;
        kernel.y = kernel.height as isize - kernel.y - 1;
        kernel.angle = (kernel.angle + 180.0) % 360.0;
    }
}

/// IM `SameKernelInfo`.
fn same_kernel(first: &MorphKernel, second: &MorphKernel) -> bool {
    if first.width != second.width || first.height != second.height || first.x != second.x || first.y != second.y {
        return false;
    }
    for (a, b) in first.values.iter().zip(second.values.iter()) {
        if a.is_nan() != b.is_nan() {
            return false;
        }
        if (a - b).abs() >= MAGICK_EPSILON {
            return false;
        }
    }
    true
}

/// IM `ExpandRotateKernelInfo` on a single kernel: appends rotated copies
/// until the rotation returns to the first kernel.
fn expand_rotate(kernels: &mut Vec<MorphKernel>, angle: f64) {
    let first = kernels[0].clone();
    loop {
        let mut rotated = kernels.last().cloned().unwrap_or_else(|| first.clone());
        rotate_kernel(&mut rotated, angle);
        if same_kernel(&first, &rotated) {
            break;
        }
        kernels.push(rotated);
    }
}

/// A kernel reflected about its origin (`RotateKernelInfo(kernel, 180)`).
fn reflected(kernel: &MorphKernel) -> MorphKernel {
    let mut result = kernel.clone();
    rotate_kernel(&mut result, 180.0);
    result
}

// ---------------------------------------------------------------------------
// Primitives
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Primitive {
    Dilate,
    Erode,
    HitAndMiss,
    Thinning,
    Thicken,
}

/// One channel of an image with the virtual-pixel border IM reads for a
/// kernel (`GetCacheViewVirtualPixels(-offset_x, y - offset_y, ...)`).
struct PaddedChannel {
    data: Vec<f32>,
    stride: usize,
}

impl PaddedChannel {
    fn new(image: &Image, channel: usize, kernel: &MorphKernel, offset_x: isize, offset_y: isize) -> Self {
        let stride = image.width + kernel.width - 1;
        let rows = image.height + kernel.height - 1;
        let mut data = Vec::with_capacity(stride * rows);
        for row in 0..rows {
            let y = row as isize - offset_y;
            for column in 0..stride {
                let x = column as isize - offset_x;
                data.push(image.virtual_get(x, y, channel));
            }
        }
        Self { data, stride }
    }
}

/// IM `MorphologyPrimitive`: one pass of a primitive, returning the new
/// image and the number of changed samples.
fn apply_primitive(image: &Image, primitive: Primitive, kernel: &MorphKernel) -> (Image, usize) {
    let reflect = primitive == Primitive::Dilate;
    let (offset_x, offset_y) = if reflect { (kernel.width as isize - kernel.x - 1, kernel.height as isize - kernel.y - 1) } else { (kernel.x, kernel.y) };
    let size = kernel.width * kernel.height;
    let mut first_set = Vec::new();
    let mut second_set = Vec::new();
    let mut count = 0usize;
    for v in 0..kernel.height {
        for u in 0..kernel.width {
            let index = v * kernel.width + u;
            let value = if reflect { kernel.values[size - 1 - index] } else { kernel.values[index] };
            if value.is_nan() {
                continue;
            }
            match primitive {
                Primitive::Dilate if value > 0.5 => first_set.push((u, v)),
                Primitive::Erode if value >= 0.5 => first_set.push((u, v)),
                Primitive::HitAndMiss | Primitive::Thinning | Primitive::Thicken => {
                    if value > 0.7 {
                        first_set.push((u, v));
                    } else if value < 0.3 {
                        second_set.push((u, v));
                    }
                    count += 1;
                }
                _ => {}
            }
        }
    }
    let gamma = if count != 0 { (kernel.height as f64 * kernel.width as f64) / count as f64 } else { 1.0 };
    let channels = image.channels;
    let width = image.width;
    let mut result = image.clone();
    let mut changed = 0usize;
    for channel in 0..channels {
        let padded = PaddedChannel::new(image, channel, kernel, offset_x, offset_y);
        let center_offset = offset_y as usize * padded.stride + offset_x as usize;
        let row_changes: Vec<(Vec<f32>, usize)> = (0..image.height)
            .into_par_iter()
            .map(|y| {
                let mut row = Vec::with_capacity(width);
                let mut row_changed = 0usize;
                for x in 0..width {
                    let base = y * padded.stride + x;
                    let center = padded.data[base + center_offset] as f64;
                    let sample = |(u, v): &(usize, usize)| padded.data[base + v * padded.stride + u] as f64;
                    let mut pixel = match primitive {
                        Primitive::Dilate => {
                            let mut maximum = 0.0f64;
                            for offset in &first_set {
                                let value = sample(offset);
                                if value > maximum {
                                    maximum = value;
                                }
                            }
                            maximum
                        }
                        Primitive::Erode => {
                            let mut minimum = 1.0f64;
                            for offset in &first_set {
                                let value = sample(offset);
                                if value < minimum {
                                    minimum = value;
                                }
                            }
                            minimum
                        }
                        _ => {
                            // HitAndMiss starts from white; Thinning and Thicken
                            // start from the pixel itself (IM's default case).
                            let mut foreground = if primitive == Primitive::HitAndMiss { 1.0f64 } else { center };
                            let mut background = 0.0f64;
                            for offset in &first_set {
                                let value = sample(offset);
                                if value < foreground {
                                    foreground = value;
                                }
                            }
                            for offset in &second_set {
                                let value = sample(offset);
                                if value > background {
                                    background = value;
                                }
                            }
                            let mut matched = foreground - background;
                            if matched < 0.0 {
                                matched = 0.0;
                            }
                            match primitive {
                                Primitive::Thinning => center - matched,
                                Primitive::Thicken => matched + center + matched,
                                _ => matched,
                            }
                        }
                    };
                    if ((pixel - center) * QUANTUM_RANGE).abs() > MAGICK_EPSILON {
                        row_changed += 1;
                    }
                    pixel *= gamma;
                    row.push(pixel as f32);
                }
                (row, row_changed)
            })
            .collect();
        for (y, (row, row_changed)) in row_changes.into_iter().enumerate() {
            changed += row_changed;
            for (x, value) in row.into_iter().enumerate() {
                result.data[(y * width + x) * channels + channel] = value;
            }
        }
    }
    (result, changed)
}

/// One channel in IM's quantum units (single precision like HDRI pixels),
/// which an in-place Distance pass reads and rewrites.
struct QuantumPlane {
    values: Vec<f32>,
    width: usize,
}

impl QuantumPlane {
    fn get(&self, x: usize, y: usize) -> f64 {
        self.values[y * self.width + x] as f64
    }

    fn set(&mut self, x: usize, y: usize, value: f64) {
        self.values[y * self.width + x] = value as f32;
    }
}

/// A pixel as IM's virtual cache view sees it during an in-place pass:
/// row `current_row` still holds the values saved in `saved_row`.
fn direct_sample(image: &Image, plane: &QuantumPlane, x: isize, y: isize, current_row: usize, saved_row: &[f32]) -> f64 {
    match image.virtual_source(x, y) {
        VirtualSource::Pixel(column, row) if row == current_row => saved_row[column] as f64,
        VirtualSource::Pixel(column, row) => plane.get(column, row),
        VirtualSource::Constant(value) => (value as f64 * QUANTUM_RANGE) as f32 as f64,
    }
}

/// IM `MorphologyPrimitiveDirect` for `Distance`: two in-place passes that
/// reuse results already written in the current pass.
fn distance_direct(image: &mut Image, kernel: &MorphKernel) {
    let width = image.width;
    let height = image.height;
    let channels = image.channels;
    let kernel_width = kernel.width as isize;
    let offset_x = kernel.width as isize - kernel.x - 1;
    let offset_y = kernel.height as isize - kernel.y - 1;
    let values = &kernel.values;
    let mut saved_row = vec![0.0f32; width];
    for channel in 0..channels {
        let mut quantum_values = Vec::with_capacity(width * height);
        for pixel in image.data.chunks(channels) {
            quantum_values.push((pixel[channel] as f64 * QUANTUM_RANGE) as f32);
        }
        let mut plane = QuantumPlane { values: quantum_values, width };
        // Forward pass, top to bottom.
        for y in 0..height {
            saved_row.copy_from_slice(&plane.values[y * width..(y + 1) * width]);
            for x in 0..width {
                let mut pixel = QUANTUM_RANGE;
                let mut k = (kernel.width * kernel.height) as isize - 1;
                for v in 0..=offset_y {
                    let row = y as isize - offset_y + v;
                    for u in 0..kernel_width {
                        let value = values[k as usize];
                        if !value.is_nan() {
                            let column = x as isize - offset_x + u;
                            let sample = direct_sample(image, &plane, column, row, y, &saved_row);
                            if sample + value < pixel {
                                pixel = sample + value;
                            }
                        }
                        k -= 1;
                    }
                }
                let mut k = kernel.width as isize * (kernel.y + 1) - 1;
                for u in 0..offset_x {
                    let value = values[k as usize];
                    let column = x as isize + u - offset_x;
                    if !value.is_nan() && column >= 0 {
                        let sample = plane.get(column as usize, y);
                        if sample + value < pixel {
                            pixel = sample + value;
                        }
                    }
                    k -= 1;
                }
                plane.set(x, y, pixel);
            }
        }
        // Reverse pass, bottom to top.
        for y in (0..height).rev() {
            saved_row.copy_from_slice(&plane.values[y * width..(y + 1) * width]);
            for x in (0..width).rev() {
                let mut pixel = QUANTUM_RANGE;
                let mut k = kernel.width as isize * (kernel.y + 1) - 1;
                for v in offset_y..kernel.height as isize {
                    let row = y as isize + v - offset_y;
                    for u in 0..kernel_width {
                        let value = values[k as usize];
                        if !value.is_nan() {
                            let column = x as isize - offset_x + u;
                            let sample = direct_sample(image, &plane, column, row, y, &saved_row);
                            if sample + value < pixel {
                                pixel = sample + value;
                            }
                        }
                        k -= 1;
                    }
                }
                let mut k = kernel.width as isize * kernel.y + kernel.x - 1;
                for u in offset_x + 1..kernel_width {
                    let column = x as isize + u - offset_x;
                    if k >= 0 {
                        let value = values[k as usize];
                        if !value.is_nan() && column < width as isize {
                            let sample = plane.get(column as usize, y);
                            if sample + value < pixel {
                                pixel = sample + value;
                            }
                        }
                    }
                    k -= 1;
                }
                plane.set(x, y, pixel);
            }
        }
        for (pixel, value) in image.data.chunks_mut(channels).zip(plane.values.iter()) {
            pixel[channel] = (*value as f64 / QUANTUM_RANGE) as f32;
        }
    }
}

/// `CompositeImage(canvas, source, Difference)` for opaque images: the
/// clamped absolute difference.
fn difference(canvas: &Image, source: &Image) -> Image {
    let mut result = canvas.clone();
    result.data.par_iter_mut().zip(source.data.par_iter()).for_each(|(value, &other)| {
        let difference = (*value as f64 - other as f64).abs();
        *value = difference.clamp(0.0, 1.0) as f32;
    });
    result
}

/// `CompositeImage(canvas, source, Lighten)` for opaque images.
fn lighten(canvas: &mut Image, source: &Image) {
    canvas.data.par_iter_mut().zip(source.data.par_iter()).for_each(|(value, &other)| {
        let lighter = (*value as f64).max(other as f64);
        *value = lighter.clamp(0.0, 1.0) as f32;
    });
}

/// IM `MorphologyImage(image, method, iterations, kernels)` (`MorphologyApply`
/// with the default multi-kernel composition).
pub fn morphology(image: &Image, method: MorphologyMethod, kernels: &[MorphKernel], iterations: isize) -> Image {
    if iterations == 0 || kernels.is_empty() {
        return image.clone();
    }
    let mut kernel_limit = iterations as usize;
    if iterations < 0 {
        kernel_limit = image.width.max(image.height);
    }
    if method == MorphologyMethod::Distance {
        let mut result = image.clone();
        distance_direct(&mut result, &kernels[0]);
        return result;
    }
    let stage_limit = match method {
        MorphologyMethod::Smooth => 4,
        MorphologyMethod::Open | MorphologyMethod::TopHat | MorphologyMethod::Close | MorphologyMethod::BottomHat | MorphologyMethod::Edge => 2,
        _ => 1,
    };
    let mut method_limit = 1;
    if matches!(method, MorphologyMethod::HitAndMiss | MorphologyMethod::Thinning | MorphologyMethod::Thicken) {
        method_limit = kernel_limit;
        kernel_limit = 1;
    }
    let lighten_results = method == MorphologyMethod::HitAndMiss;
    let reflected_kernels: Vec<MorphKernel> = kernels.iter().map(reflected).collect();
    let mut current = image.clone();
    let mut result: Option<Image> = None;
    let mut method_loop = 0;
    let mut method_changed = 1;
    while method_loop < method_limit && method_changed > 0 {
        method_loop += 1;
        method_changed = 0;
        for (kernel_index, kernel) in kernels.iter().enumerate() {
            let reflected_kernel = &reflected_kernels[kernel_index];
            let mut saved: Option<Image> = None;
            for stage in 1..=stage_limit {
                let (primitive, this_kernel) = match method {
                    MorphologyMethod::Erode | MorphologyMethod::EdgeIn => (Primitive::Erode, kernel),
                    MorphologyMethod::Dilate | MorphologyMethod::EdgeOut => (Primitive::Dilate, kernel),
                    MorphologyMethod::Open | MorphologyMethod::TopHat => (if stage == 2 { Primitive::Dilate } else { Primitive::Erode }, kernel),
                    MorphologyMethod::Close | MorphologyMethod::BottomHat => (if stage == 2 { Primitive::Erode } else { Primitive::Dilate }, reflected_kernel),
                    MorphologyMethod::Smooth => match stage {
                        1 => (Primitive::Erode, kernel),
                        2 => (Primitive::Dilate, kernel),
                        3 => (Primitive::Dilate, reflected_kernel),
                        _ => (Primitive::Erode, reflected_kernel),
                    },
                    MorphologyMethod::Edge => {
                        if stage == 2 {
                            saved = Some(std::mem::replace(&mut current, image.clone()));
                            (Primitive::Erode, kernel)
                        } else {
                            (Primitive::Dilate, kernel)
                        }
                    }
                    MorphologyMethod::HitAndMiss => (Primitive::HitAndMiss, kernel),
                    MorphologyMethod::Thinning => (Primitive::Thinning, kernel),
                    MorphologyMethod::Thicken => (Primitive::Thicken, kernel),
                    MorphologyMethod::Distance => unreachable!("handled above"),
                };
                let mut kernel_loop = 0;
                let mut changed = 1;
                while kernel_loop < kernel_limit && changed > 0 {
                    kernel_loop += 1;
                    let (next, next_changed) = apply_primitive(&current, primitive, this_kernel);
                    changed = next_changed;
                    method_changed += next_changed;
                    current = next;
                }
            }
            match method {
                MorphologyMethod::EdgeOut | MorphologyMethod::EdgeIn | MorphologyMethod::TopHat | MorphologyMethod::BottomHat => {
                    current = difference(&current, image);
                }
                MorphologyMethod::Edge => {
                    if let Some(dilated) = &saved {
                        current = difference(&current, dilated);
                    }
                }
                _ => {}
            }
            if kernels.len() == 1 || !lighten_results {
                result = Some(current.clone());
            } else {
                match result.as_mut() {
                    None => result = Some(current.clone()),
                    Some(union) => lighten(union, &current),
                }
                current = image.clone();
            }
        }
    }
    result.unwrap_or(current)
}
