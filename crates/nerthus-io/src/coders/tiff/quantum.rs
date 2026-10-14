//! ImageMagick's `ImportQuantumPixels` for the quantum types its TIFF
//! reader uses, with its quirks: min-is-white honoured only at 1, 8 and
//! 16 bits; pads skipped by the byte-sized paths but not the bit-packed
//! ones; 10-bit RGBA scaled as `value << 6`; associated alpha divided out
//! of every other updated channel afterwards.

use anyhow::{bail, Result};

/// `QuantumRange`.
const QUANTUM_RANGE: f64 = 65535.0;
/// `MagickEpsilon`.
const MAGICK_EPSILON: f64 = 1.0e-12;

/// The image being filled: ImageMagick's pixel cache and channel map.
pub(super) struct Picture {
    pub width: usize,
    pub height: usize,
    pub cmyk: bool,
    pub alpha: bool,
    pub pseudo: bool,
    pub meta: usize,
    /// `GetPixelChannels`.
    pub stride: usize,
    pub data: Vec<f32>,
    /// The colormap of a PseudoClass image (red, green, blue).
    pub colormap: Vec<[f64; 3]>,
    pub red: usize,
    pub green: usize,
    pub blue: usize,
    pub black: Option<usize>,
    pub alpha_offset: Option<usize>,
    pub index: Option<usize>,
    pub meta_offset: usize,
}

impl Picture {
    /// `ResetPixelChannelMap`: grey or red, green, blue; black; alpha; the
    /// index; the meta channels.
    pub fn new(width: usize, height: usize, gray: bool, cmyk: bool, alpha: bool, pseudo: bool, meta: usize) -> Self {
        let mut count = 0;
        let (red, green, blue) = if gray {
            count += 1;
            (0, 0, 0)
        } else {
            count += 3;
            (0, 1, 2)
        };
        let black = cmyk.then(|| {
            count += 1;
            count - 1
        });
        let alpha_offset = alpha.then(|| {
            count += 1;
            count - 1
        });
        let index = pseudo.then(|| {
            count += 1;
            count - 1
        });
        let meta_offset = count;
        count += meta;
        let mut data = vec![0f32; width * height * count];
        if let Some(offset) = alpha_offset {
            // `SetImageAlphaChannel(OpaqueAlphaChannel)`.
            for pixel in data.chunks_exact_mut(count) {
                pixel[offset] = QUANTUM_RANGE as f32;
            }
        }
        Self { width, height, cmyk, alpha, pseudo, meta, stride: count, data, colormap: Vec::new(), red, green, blue, black, alpha_offset, index, meta_offset }
    }

    /// `GetImageChannels`: the channels with the update trait (all but
    /// the index).
    fn update_channels(&self) -> usize {
        let count = self.stride - usize::from(self.pseudo);
        count.max(1)
    }
}

/// The ImageMagick quantum types the TIFF reader imports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum QuantumType {
    Gray,
    GrayAlpha,
    Index,
    IndexAlpha,
    Rgb,
    Rgba,
    Cmyk,
    Cmyka,
    Alpha,
    Red,
    Green,
    Blue,
    Black,
    Multispectral,
}

impl QuantumType {
    /// `GetQuantumExtent`'s channel count.
    fn channels(self, picture: &Picture) -> usize {
        match self {
            QuantumType::GrayAlpha | QuantumType::IndexAlpha => 2,
            QuantumType::Rgb => 3,
            QuantumType::Rgba | QuantumType::Cmyk => 4,
            QuantumType::Cmyka => 5,
            QuantumType::Multispectral => picture.update_channels(),
            _ => 1,
        }
    }
}

/// `QuantumInfo`: how samples are laid out.
#[derive(Clone, Debug)]
pub(super) struct QuantumInfo {
    pub depth: u32,
    pub float: bool,
    pub min_is_white: bool,
    pub pad: usize,
    pub associated_alpha: bool,
    /// The meta channel a separate plane goes to (1-based; 0 is none).
    pub meta_channel: usize,
}

impl QuantumInfo {
    /// `SetQuantumDepth`: floats round up to 16, 24, 32 or 64 bits.
    pub fn new(depth: u32, float: bool) -> Self {
        let mut depth = depth.min(64);
        if float {
            depth = if depth > 32 {
                64
            } else if depth > 24 {
                32
            } else if depth > 16 {
                24
            } else {
                16
            };
        }
        Self { depth, float, min_is_white: false, pad: 0, associated_alpha: false, meta_channel: 0 }
    }

    /// `GetQuantumExtent` (packed samples).
    pub fn extent(&self, picture: &Picture, kind: QuantumType) -> usize {
        let channels = kind.channels(picture);
        (channels * picture.width * self.depth as usize).div_ceil(8) + self.pad * picture.width
    }
}

/// Samples read from a row of bytes, ImageMagick's `Push*Pixel` in
/// little-endian order; bytes past the buffer read as zero.
struct Samples<'a> {
    bytes: &'a [u8],
    position: usize,
    /// `quantum_info->state`: the byte being taken apart bit by bit.
    bits: u32,
    pixel: u32,
}

impl Samples<'_> {
    fn byte(&mut self) -> u8 {
        let value = self.bytes.get(self.position).copied().unwrap_or(0);
        self.position += 1;
        value
    }

    fn skip(&mut self, count: usize) {
        self.position += count;
    }

    fn short(&mut self) -> u16 {
        let low = self.byte() as u16;
        let high = self.byte() as u16;
        low | (high << 8)
    }

    fn long(&mut self) -> u32 {
        let mut value = 0u32;
        for shift in [0, 8, 16, 24] {
            value |= (self.byte() as u32) << shift;
        }
        value
    }

    /// `ScaleFloatPixel` on a float's bits.
    fn scale_float(bits: u32) -> f32 {
        let pixel = f32::from_bits(bits) as f64 * QUANTUM_RANGE;
        if pixel < -(f32::MAX as f64) {
            return -f32::MAX;
        }
        if pixel > f32::MAX as f64 {
            return f32::MAX;
        }
        pixel as f32
    }

    /// `PushQuantumFloatPixel`.
    fn float(&mut self) -> f32 {
        let bits = self.long();
        Self::scale_float(bits)
    }

    /// `PushQuantumFloat24Pixel`: sign, 7-bit exponent biased by 63 and a
    /// 16-bit mantissa, widened to a float.
    fn float24(&mut self) -> f32 {
        let mut quantum = [self.byte(), self.byte(), self.byte(), 0u8];
        if (quantum[0] | quantum[1] | quantum[2]) != 0 {
            let sign_bit = quantum[2] & 0x80;
            let mut exponent = quantum[2] & 0x7F;
            if exponent != 0 {
                exponent = exponent.wrapping_sub(63).wrapping_add(127);
            }
            quantum[3] = sign_bit | (exponent >> 1);
            quantum[2] = ((exponent & 1) << 7) | ((quantum[1] & 0xFE) >> 1);
            quantum[1] = ((quantum[1] & 0x01) << 7) | ((quantum[0] & 0xFE) >> 1);
            quantum[0] = (quantum[0] & 0x01) << 7;
        }
        Self::scale_float(u32::from_le_bytes(quantum))
    }

    /// `PushDoublePixel`.
    fn double(&mut self) -> f64 {
        let mut eight = [0u8; 8];
        for byte in eight.iter_mut() {
            *byte = self.byte();
        }
        f64::from_le_bytes(eight) * QUANTUM_RANGE
    }

    /// `PushQuantumPixel`: `depth` bits, most significant first.
    fn any(&mut self, depth: u32) -> u32 {
        let mut quantum: u64 = 0;
        let mut remaining = depth;
        while remaining > 0 {
            if self.bits == 0 {
                self.pixel = self.byte() as u32;
                self.bits = 8;
            }
            let count = remaining.min(self.bits);
            remaining -= count;
            self.bits -= count;
            let part = (self.pixel >> self.bits) as u64 & !(u64::MAX << count);
            quantum = (quantum << count) | part;
        }
        quantum as u32
    }

    /// A half float (`HalfToSinglePrecision`) times the quantum range.
    fn half(&mut self) -> f32 {
        let value = self.short();
        (QUANTUM_RANGE * half::f16::from_bits(value).to_f32() as f64) as f32
    }
}

/// `GetQuantumRange`.
fn quantum_range(depth: u32) -> u64 {
    if depth == 0 {
        return 0;
    }
    if depth >= 64 {
        return u64::MAX;
    }
    (1u64 << depth) - 1
}

/// `ScaleAnyToQuantum`.
fn scale_any(quantum: u64, range: u64) -> f32 {
    if quantum > range {
        return QUANTUM_RANGE as f32;
    }
    let reciprocal = if range == 0 { 1.0 / MAGICK_EPSILON } else { 1.0 / range as f64 };
    (QUANTUM_RANGE * (quantum as f64 * reciprocal)) as f32
}

fn char_quantum(value: u8) -> f32 {
    (257.0 * value as f64) as f32
}

fn long_quantum(value: u32) -> f32 {
    (value as f64 / 65537.0) as f32
}

/// Where each sample goes in a pixel.
#[derive(Clone, Copy)]
enum Slot {
    Red,
    Green,
    Blue,
    Black,
    Alpha,
    Meta(usize),
    /// A raw position (the multispectral import's `q[i]`).
    Raw(usize),
}

impl Picture {
    /// `SetPixelRed` and friends: colour setters always write (to the
    /// shared grey channel of a grey image); black and alpha only when
    /// the image has them.
    fn set(&mut self, pixel: usize, slot: Slot, value: f32) {
        let base = pixel * self.stride;
        let offset = match slot {
            Slot::Red => Some(self.red),
            Slot::Green => Some(self.green),
            Slot::Blue => Some(self.blue),
            Slot::Black => self.black,
            Slot::Alpha => self.alpha_offset,
            Slot::Meta(index) => Some(self.meta_offset + index),
            Slot::Raw(index) => Some(index),
        };
        if let Some(offset) = offset
            && let Some(sample) = self.data.get_mut(base + offset)
        {
            *sample = value;
        }
    }

    /// `ImportPixelChannel`'s target: an undefined channel's offset is 0.
    fn channel_offset(&self, slot: Slot) -> usize {
        match slot {
            Slot::Red => self.red,
            Slot::Green => self.green,
            Slot::Blue => self.blue,
            Slot::Black => self.black.unwrap_or(0),
            Slot::Alpha => self.alpha_offset.unwrap_or(0),
            Slot::Meta(index) => self.meta_offset + index,
            Slot::Raw(index) => index,
        }
    }

    /// `SetPixelIndex` plus `SetPixelViaPixelInfo` from the colormap.
    fn set_index(&mut self, pixel: usize, index: u64, range_error: &mut bool) {
        let index = if (index as usize) < self.colormap.len() {
            index as usize
        } else {
            *range_error = true;
            0
        };
        let base = pixel * self.stride;
        if let Some(offset) = self.index {
            self.data[base + offset] = index as f32;
        }
        let entry = self.colormap.get(index).copied().unwrap_or([0.0; 3]);
        self.data[base + self.red] = entry[0] as f32;
        self.data[base + self.green] = entry[1] as f32;
        self.data[base + self.blue] = entry[2] as f32;
        if let Some(offset) = self.alpha_offset {
            self.data[base + offset] = QUANTUM_RANGE as f32;
        }
    }
}

/// Reads one sample of the general paths (8, 16, 24, 32 and 64 bits, or
/// packed bits).
fn general_sample(samples: &mut Samples, info: &QuantumInfo) -> f32 {
    match info.depth {
        8 => char_quantum(samples.byte()),
        16 if info.float => samples.half(),
        16 => samples.short() as f32,
        32 if info.float => samples.float(),
        32 => long_quantum(samples.long()),
        24 if info.float => samples.float24(),
        64 if info.float => samples.double() as f32,
        depth => scale_any(samples.any(depth) as u64, quantum_range(depth)),
    }
}

/// Whether the general path at this depth skips the pad (the bit-packed
/// fallback does not).
fn general_pads(info: &QuantumInfo) -> bool {
    match info.depth {
        8 | 16 | 32 => true,
        24 | 64 => info.float,
        _ => false,
    }
}

/// The colour channels of a quantum type, in order.
fn slots(kind: QuantumType) -> &'static [Slot] {
    match kind {
        QuantumType::Rgb => &[Slot::Red, Slot::Green, Slot::Blue],
        QuantumType::Rgba => &[Slot::Red, Slot::Green, Slot::Blue, Slot::Alpha],
        QuantumType::Cmyk => &[Slot::Red, Slot::Green, Slot::Blue, Slot::Black],
        QuantumType::Cmyka => &[Slot::Red, Slot::Green, Slot::Blue, Slot::Black, Slot::Alpha],
        _ => &[],
    }
}

/// `ImportQuantumPixels` for `count` pixels from pixel `start` of the
/// picture.
pub(super) fn import(picture: &mut Picture, info: &QuantumInfo, kind: QuantumType, start: usize, count: usize, bytes: &[u8]) -> Result<()> {
    let mut samples = Samples { bytes, position: 0, bits: 0, pixel: 0 };
    match kind {
        QuantumType::Alpha => {
            if !picture.alpha {
                bail!("ImageDoesNotHaveAnAlphaChannel");
            }
            import_channel(picture, info, start, count, &mut samples, Slot::Alpha);
        }
        QuantumType::Black => {
            if !picture.cmyk {
                bail!("ColorSeparatedImageRequired");
            }
            import_channel(picture, info, start, count, &mut samples, Slot::Black);
        }
        QuantumType::Red => import_channel(picture, info, start, count, &mut samples, Slot::Red),
        QuantumType::Green => import_channel(picture, info, start, count, &mut samples, Slot::Green),
        QuantumType::Blue => import_channel(picture, info, start, count, &mut samples, Slot::Blue),
        QuantumType::Gray => import_gray(picture, info, start, count, &mut samples),
        QuantumType::GrayAlpha => import_gray_alpha(picture, info, start, count, &mut samples),
        QuantumType::Index => import_index(picture, info, start, count, &mut samples)?,
        QuantumType::IndexAlpha => import_index_alpha(picture, info, start, count, &mut samples)?,
        QuantumType::Rgb | QuantumType::Rgba | QuantumType::Cmyk | QuantumType::Cmyka => {
            if matches!(kind, QuantumType::Cmyk | QuantumType::Cmyka) && !picture.cmyk {
                bail!("ColorSeparatedImageRequired");
            }
            import_colours(picture, info, kind, start, count, &mut samples);
        }
        QuantumType::Multispectral => {
            if picture.meta == 0 {
                bail!("MultispectralImageRequired");
            }
            if info.meta_channel != 0 {
                let slot = Slot::Meta(info.meta_channel - 1);
                import_channel(picture, info, start, count, &mut samples, slot);
            } else {
                import_multispectral(picture, info, start, count, &mut samples);
            }
        }
    }
    if info.associated_alpha
        && let Some(alpha) = picture.alpha_offset
    {
        // Disassociate alpha from every updated channel.
        for pixel in start..start + count {
            let base = pixel * picture.stride;
            let sa = picture.data[base + alpha] as f64 / QUANTUM_RANGE;
            let gamma = if sa > -MAGICK_EPSILON && sa < MAGICK_EPSILON { 1.0 / MAGICK_EPSILON } else { 1.0 / sa };
            for channel in 0..picture.stride {
                if channel == alpha || Some(channel) == picture.index {
                    continue;
                }
                let value = picture.data[base + channel] as f64;
                picture.data[base + channel] = (gamma * value) as f32;
            }
        }
    }
    Ok(())
}

/// `ImportPixelChannel`: one channel per pixel (every path skips the
/// pad).
fn import_channel(picture: &mut Picture, info: &QuantumInfo, start: usize, count: usize, samples: &mut Samples, slot: Slot) {
    let offset = picture.channel_offset(slot);
    for pixel in start..start + count {
        let value = general_sample(samples, info);
        if let Some(sample) = picture.data.get_mut(pixel * picture.stride + offset) {
            *sample = value;
        }
        samples.skip(info.pad);
    }
}

/// `ImportGrayQuantum`.
fn import_gray(picture: &mut Picture, info: &QuantumInfo, start: usize, count: usize, samples: &mut Samples) {
    let set = |picture: &mut Picture, pixel: usize, value: f32| picture.set(pixel, Slot::Red, value);
    let set_opaque = |picture: &mut Picture, pixel: usize| picture.set(pixel, Slot::Alpha, QUANTUM_RANGE as f32);
    match info.depth {
        1 => {
            let (black, white) = if info.min_is_white { (QUANTUM_RANGE as f32, 0.0) } else { (0.0, QUANTUM_RANGE as f32) };
            let mut pixel = start;
            let mut x = 0;
            while x + 7 < count {
                let byte = samples.byte();
                for bit in 0..8 {
                    set(picture, pixel, if byte & (1 << (7 - bit)) == 0 { black } else { white });
                    pixel += 1;
                }
                x += 8;
            }
            let rest = count % 8;
            if rest != 0 {
                let byte = samples.bytes.get(samples.position).copied().unwrap_or(0);
                for bit in 0..rest {
                    set(picture, pixel, if byte & (1 << (7 - bit)) == 0 { black } else { white });
                    pixel += 1;
                }
            }
        }
        4 => {
            let range = quantum_range(4);
            let mut pixel = start;
            let mut x = 0;
            while x + 1 < count {
                let byte = samples.byte();
                set(picture, pixel, scale_any(((byte >> 4) & 0xF) as u64, range));
                set(picture, pixel + 1, scale_any((byte & 0xF) as u64, range));
                pixel += 2;
                x += 2;
            }
            if count % 2 == 1 {
                let byte = samples.byte();
                set(picture, pixel, scale_any((byte >> 4) as u64, range));
            }
        }
        8 => {
            for pixel in start..start + count {
                let value = char_quantum(samples.byte());
                let value = if info.min_is_white { QUANTUM_RANGE as f32 - value } else { value };
                set(picture, pixel, value);
                set_opaque(picture, pixel);
                samples.skip(info.pad);
            }
        }
        16 if info.min_is_white => {
            for pixel in start..start + count {
                let value = QUANTUM_RANGE as f32 - samples.short() as f32;
                set(picture, pixel, value);
                samples.skip(info.pad);
            }
        }
        10 | 12 => {
            let range = quantum_range(info.depth);
            for pixel in start..start + count {
                let value = scale_any(samples.any(info.depth) as u64, range);
                set(picture, pixel, value);
                samples.skip(info.pad);
            }
        }
        _ => {
            for pixel in start..start + count {
                let value = general_sample(samples, info);
                set(picture, pixel, value);
                samples.skip(info.pad);
            }
        }
    }
}

/// `ImportGrayAlphaQuantum`.
fn import_gray_alpha(picture: &mut Picture, info: &QuantumInfo, start: usize, count: usize, samples: &mut Samples) {
    match info.depth {
        1 => {
            // Four grey/alpha bit pairs a byte; the grey bit reads
            // inverted in whole bytes but not in the tail.
            let mut pixel = start;
            let mut x = count as isize - 3;
            while x > 0 {
                let byte = samples.byte();
                for bit in (0..8).step_by(2) {
                    let gray = if byte & (1 << (7 - bit)) != 0 { 0.0 } else { QUANTUM_RANGE as f32 };
                    picture.set(pixel, Slot::Red, gray);
                    let alpha = if byte as u32 & (1u32 << (6 - bit)) == 0 { 0.0 } else { QUANTUM_RANGE as f32 };
                    picture.set(pixel, Slot::Alpha, alpha);
                    pixel += 1;
                }
                x -= 4;
            }
            if !count.is_multiple_of(4) {
                let byte = samples.bytes.get(samples.position).copied().unwrap_or(0);
                let mut bit: isize = 3;
                while bit >= (4 - (count % 4)) as isize {
                    let gray = if byte & (1 << (7 - bit)) != 0 { QUANTUM_RANGE as f32 } else { 0.0 };
                    picture.set(pixel, Slot::Red, gray);
                    let alpha = if byte as u32 & (1u32 << (6 - bit) as u32) == 0 { 0.0 } else { QUANTUM_RANGE as f32 };
                    picture.set(pixel, Slot::Alpha, alpha);
                    pixel += 1;
                    bit -= 2;
                }
            }
        }
        4 => {
            let range = quantum_range(4);
            for pixel in start..start + count {
                let byte = samples.byte();
                picture.set(pixel, Slot::Red, scale_any(((byte >> 4) & 0xF) as u64, range));
                picture.set(pixel, Slot::Alpha, scale_any((byte & 0xF) as u64, range));
            }
        }
        10 | 12 => {
            let range = quantum_range(info.depth);
            for pixel in start..start + count {
                let gray = scale_any(samples.any(info.depth) as u64, range);
                picture.set(pixel, Slot::Red, gray);
                let alpha = scale_any(samples.any(info.depth) as u64, range);
                picture.set(pixel, Slot::Alpha, alpha);
                samples.skip(info.pad);
            }
        }
        _ => {
            for pixel in start..start + count {
                let gray = general_sample(samples, info);
                picture.set(pixel, Slot::Red, gray);
                let alpha = general_sample(samples, info);
                picture.set(pixel, Slot::Alpha, alpha);
                samples.skip(info.pad);
            }
        }
    }
}

/// An index sample at the general depths (floats go through the quantum
/// scale, then truncate).
fn index_sample(samples: &mut Samples, info: &QuantumInfo) -> u64 {
    match info.depth {
        8 => samples.byte() as u64,
        16 if info.float => samples.half() as u64,
        16 => samples.short() as u64,
        32 if info.float => samples.float() as u64,
        32 => samples.long() as u64,
        24 if info.float => samples.float24() as u64,
        64 if info.float => samples.double() as f32 as u64,
        depth => samples.any(depth) as u64,
    }
}

/// `ImportIndexQuantum`.
fn import_index(picture: &mut Picture, info: &QuantumInfo, start: usize, count: usize, samples: &mut Samples) -> Result<()> {
    if !picture.pseudo {
        bail!("ColormappedImageRequired");
    }
    let mut range_error = false;
    match info.depth {
        1 => {
            let mut pixel = start;
            let mut x = 0;
            while x + 7 < count {
                let byte = samples.byte();
                for bit in 0..8 {
                    let set_bit = byte & (1 << (7 - bit)) != 0;
                    let index = u64::from(set_bit != info.min_is_white);
                    picture.set_index(pixel, index, &mut range_error);
                    pixel += 1;
                }
                x += 8;
            }
            let byte = samples.bytes.get(samples.position).copied().unwrap_or(0);
            for bit in 0..count % 8 {
                let set_bit = byte & (1 << (7 - bit)) != 0;
                let index = u64::from(set_bit != info.min_is_white);
                picture.set_index(pixel, index, &mut range_error);
                pixel += 1;
            }
        }
        4 => {
            let mut pixel = start;
            let mut x = 0;
            while x + 1 < count {
                let byte = samples.byte();
                picture.set_index(pixel, ((byte >> 4) & 0xF) as u64, &mut range_error);
                picture.set_index(pixel + 1, (byte & 0xF) as u64, &mut range_error);
                pixel += 2;
                x += 2;
            }
            if count % 2 == 1 {
                let byte = samples.byte();
                picture.set_index(pixel, ((byte >> 4) & 0xF) as u64, &mut range_error);
            }
        }
        _ => {
            for pixel in start..start + count {
                let index = index_sample(samples, info);
                picture.set_index(pixel, index, &mut range_error);
                samples.skip(info.pad);
            }
        }
    }
    if range_error {
        bail!("InvalidColormapIndex");
    }
    Ok(())
}

/// `ImportIndexAlphaQuantum`.
fn import_index_alpha(picture: &mut Picture, info: &QuantumInfo, start: usize, count: usize, samples: &mut Samples) -> Result<()> {
    if !picture.pseudo {
        bail!("ColormappedImageRequired");
    }
    let mut range_error = false;
    match info.depth {
        1 => {
            // Grey and index from the bit, alpha from the next; the byte
            // pointer never advances.
            let byte = samples.bytes.get(samples.position).copied().unwrap_or(0);
            let mut pixel = start;
            let mut x = count as isize - 3;
            while x > 0 {
                for bit in (0..8).step_by(2) {
                    let set_bit = byte & (1 << (7 - bit)) != 0;
                    let on = set_bit != info.min_is_white;
                    picture.set(pixel, Slot::Red, if on { QUANTUM_RANGE as f32 } else { 0.0 });
                    let alpha = if byte as u32 & (1u32 << (6 - bit)) == 0 { 0.0 } else { QUANTUM_RANGE as f32 };
                    picture.set(pixel, Slot::Alpha, alpha);
                    if let Some(offset) = picture.index {
                        picture.data[pixel * picture.stride + offset] = if on { 1.0 } else { 0.0 };
                    }
                    pixel += 1;
                }
                x -= 4;
            }
            if !count.is_multiple_of(4) {
                let mut bit: isize = 3;
                while bit >= (4 - (count % 4)) as isize {
                    let set_bit = byte & (1 << (7 - bit)) != 0;
                    let on = set_bit != info.min_is_white;
                    if let Some(offset) = picture.index {
                        picture.data[pixel * picture.stride + offset] = if on { 1.0 } else { 0.0 };
                    }
                    picture.set(pixel, Slot::Red, if on { QUANTUM_RANGE as f32 } else { 0.0 });
                    let alpha = if byte as u32 & (1u32 << (6 - bit) as u32) == 0 { 0.0 } else { QUANTUM_RANGE as f32 };
                    picture.set(pixel, Slot::Alpha, alpha);
                    pixel += 1;
                    bit -= 2;
                }
            }
        }
        4 => {
            let range = quantum_range(4);
            for pixel in start..start + count {
                let byte = samples.byte();
                picture.set_index(pixel, ((byte >> 4) & 0xF) as u64, &mut range_error);
                picture.set(pixel, Slot::Alpha, scale_any((byte & 0xF) as u64, range));
            }
        }
        _ => {
            let range = quantum_range(info.depth);
            for pixel in start..start + count {
                let index = index_sample(samples, info);
                picture.set_index(pixel, index, &mut range_error);
                let alpha = match info.depth {
                    8 | 16 | 32 => general_sample(samples, info),
                    24 | 64 if info.float => general_sample(samples, info),
                    depth => scale_any(samples.any(depth) as u64, range),
                };
                picture.set(pixel, Slot::Alpha, alpha);
                samples.skip(info.pad);
            }
        }
    }
    if range_error {
        bail!("InvalidColormapIndex");
    }
    Ok(())
}

/// `ImportRGBQuantum`, `ImportRGBAQuantum`, `ImportCMYKQuantum`,
/// `ImportCMYKAQuantum`.
fn import_colours(picture: &mut Picture, info: &QuantumInfo, kind: QuantumType, start: usize, count: usize, samples: &mut Samples) {
    let order = slots(kind);
    if info.depth == 10 && kind == QuantumType::Rgba {
        // Scaled as 16-bit values shifted up by six.
        for pixel in start..start + count {
            for &slot in order {
                let value = (samples.any(10) << 6) as u16;
                picture.set(pixel, slot, value as f32);
            }
        }
        return;
    }
    let pads = general_pads(info);
    for pixel in start..start + count {
        for &slot in order {
            let value = general_sample(samples, info);
            picture.set(pixel, slot, value);
        }
        if kind == QuantumType::Rgb && info.depth == 8 {
            picture.set(pixel, Slot::Alpha, QUANTUM_RANGE as f32);
        }
        if pads {
            samples.skip(info.pad);
        }
    }
}

/// `ImportMultispectralQuantum`: every updated channel in order (no pad
/// skipped by the bit-packed path).
fn import_multispectral(picture: &mut Picture, info: &QuantumInfo, start: usize, count: usize, samples: &mut Samples) {
    let channels = picture.update_channels();
    let pads = general_pads(info);
    for pixel in start..start + count {
        for channel in 0..channels {
            let value = general_sample(samples, info);
            picture.set(pixel, Slot::Raw(channel), value);
        }
        if pads {
            samples.skip(info.pad);
        }
    }
}
