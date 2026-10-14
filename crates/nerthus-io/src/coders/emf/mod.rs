//! Windows enhanced metafiles (`coders/emf.c`, which Magick.NET builds on
//! GDI+): the picture is as large as the header's frame at the reference
//! device's resolution, drawn onto opaque white (`Graphics.Clear` with the
//! background colour) and read back as BGRA with an alpha channel
//! (`BlendPixelTrait`). GDI+ plays a metafile's EMF+ records when it has
//! them, anti-aliased (`SmoothingModeHighQuality`), and only the GDI
//! records an `EmfPlusGetDC` lets through; a plain metafile's GDI records
//! go through GDI, without anti-aliasing. Nerthus plays both kinds itself
//! onto a tiny-skia canvas, so edges and text differ a little from
//! Windows'.

mod dib;
mod gdi;
mod plus;
mod text;

use std::path::Path;

use anyhow::{bail, Context, Result};
use resvg::tiny_skia::{Color, Pixmap, Transform};

use crate::pixel_cache::{CacheColorspace, DEFAULT_GAMMA, PixelCache, PixelChannel, char_quantum};

pub(crate) const EMR_HEADER: u32 = 1;
pub(crate) const EMR_EOF: u32 = 14;
pub(crate) const EMR_COMMENT: u32 = 70;
/// `ENHMETA_SIGNATURE`: " EMF".
const SIGNATURE: u32 = 0x464D_4520;
/// An EMF+ comment's identifier: "EMF+".
const EMF_PLUS: u32 = 0x2B46_4D45;
/// `EmfPlusHeader`.
const PLUS_HEADER: u16 = 0x4001;

/// The largest picture read, a side.
const MAXIMUM_SIDE: f64 = 32768.0;
/// How deep metafiles drawn inside metafiles go.
const MAXIMUM_DEPTH: u32 = 4;

pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    let header = Header::parse(&bytes).context("UnableToOpenFile")?;
    let (width, height) = header.picture_size();
    if width < 1.0 || height < 1.0 {
        bail!("NegativeOrZeroImageSize");
    }
    if width > MAXIMUM_SIDE || height > MAXIMUM_SIDE {
        bail!("WidthOrHeightExceedsLimit");
    }
    let (width, height) = (width as usize, height as usize);
    let mut pixmap = Pixmap::new(width as u32, height as u32).context("MemoryAllocationFailed")?;
    pixmap.fill(Color::WHITE);
    play(&bytes, &header, &mut pixmap, 0);

    // `LockBits` gives straight BGRA; each byte becomes a quantum.
    let mut data = Vec::with_capacity(width * height * 4);
    for pixel in pixmap.pixels() {
        let colour = pixel.demultiply();
        data.extend([colour.red(), colour.green(), colour.blue(), colour.alpha()].map(char_quantum));
    }
    let channels = vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Alpha];
    Ok(PixelCache::new(width, height, channels, data, CacheColorspace::SRgb, DEFAULT_GAMMA))
}

/// Draws a metafile held in memory over `pixmap`, filling it (a metafile
/// image inside an EMF+ record).
pub(crate) fn draw_nested(bytes: &[u8], pixmap: &mut Pixmap, depth: u32) -> bool {
    if depth >= MAXIMUM_DEPTH {
        return false;
    }
    let Some(header) = Header::parse(bytes) else {
        return false;
    };
    play(bytes, &header, pixmap, depth + 1);
    true
}

/// The metafile header (`EMR_HEADER`).
#[derive(Clone, Debug)]
pub(crate) struct Header {
    /// `rclFrame`, in hundredths of a millimetre.
    pub frame: [i32; 4],
    /// `szlDevice`: the reference device in pixels.
    pub device: (i32, i32),
    /// `szlMillimeters`: the reference device in millimetres.
    pub millimeters: (i32, i32),
    /// `nHandles`.
    pub handles: usize,
}

impl Header {
    fn parse(bytes: &[u8]) -> Option<Header> {
        if bytes.u32_at(0) != EMR_HEADER || bytes.len() < 88 || bytes.u32_at(40) != SIGNATURE {
            return None;
        }
        let frame = [bytes.i32_at(24), bytes.i32_at(28), bytes.i32_at(32), bytes.i32_at(36)];
        let handles = bytes.u16_at(56) as usize;
        let device = (bytes.i32_at(72), bytes.i32_at(76));
        let millimeters = (bytes.i32_at(80), bytes.i32_at(84));
        if device.0 <= 0 || device.1 <= 0 || millimeters.0 <= 0 || millimeters.1 <= 0 {
            return None;
        }
        Some(Header { frame, device, millimeters, handles })
    }

    /// The reference device's dots per inch (`MetafileHeader.DpiX/Y`).
    pub fn dpi(&self) -> (f64, f64) {
        let x = self.device.0 as f64 * 25.4 / self.millimeters.0 as f64;
        let y = self.device.1 as f64 * 25.4 / self.millimeters.1 as f64;
        (x, y)
    }

    /// The frame in reference device pixels: left, top, width, height.
    pub fn frame_pixels(&self) -> [f64; 4] {
        let (dpi_x, dpi_y) = self.dpi();
        let left = self.frame[0] as f64 / 2540.0 * dpi_x;
        let top = self.frame[1] as f64 / 2540.0 * dpi_y;
        let width = (self.frame[2] - self.frame[0]) as f64 / 2540.0 * dpi_x;
        let height = (self.frame[3] - self.frame[1]) as f64 / 2540.0 * dpi_y;
        [left, top, width, height]
    }

    /// `Image.GetWidth/GetHeight`: the frame's size in pixels, rounded.
    fn picture_size(&self) -> (f64, f64) {
        let [_, _, width, height] = self.frame_pixels();
        ((width + 0.5).floor(), (height + 0.5).floor())
    }

    /// Reference device pixels to the picture's (`DrawImage` into the
    /// picture's bounds): the frame fills it.
    fn playback(&self, width: f64, height: f64) -> Affine {
        let [left, top, frame_width, frame_height] = self.frame_pixels();
        let scale_x = if frame_width != 0.0 { width / frame_width } else { 1.0 };
        let scale_y = if frame_height != 0.0 { height / frame_height } else { 1.0 };
        Affine { a: scale_x, b: 0.0, c: 0.0, d: scale_y, e: -left * scale_x, f: -top * scale_y }
    }
}

/// Plays the records onto `pixmap`: the EMF+ ones when the metafile has
/// them (and the GDI ones an `EmfPlusGetDC` lets through), else the GDI
/// ones.
fn play(bytes: &[u8], header: &Header, pixmap: &mut Pixmap, depth: u32) {
    let playback = header.playback(pixmap.width() as f64, pixmap.height() as f64);
    let mut records = Records { bytes, at: 0 };
    let _header_record = records.next();
    let mut rest = records.clone();
    let plus_header = rest.next().and_then(|record| plus_records(record.data).next()).filter(|record| record.kind == PLUS_HEADER);

    let mut gdi = gdi::Player::new(header, playback, pixmap.width(), pixmap.height());
    let mut plus = plus_header.map(|record| plus::Player::new(header, record.data, playback, pixmap.width(), pixmap.height(), depth));
    let mut gdi_on = plus.is_none();
    for record in records {
        if record.kind == EMR_EOF {
            break;
        }
        if record.kind == EMR_COMMENT {
            if let Some(player) = plus.as_mut() {
                for plus_record in plus_records(record.data) {
                    gdi_on = plus_record.kind == plus::GET_DC;
                    player.play(&plus_record, pixmap);
                }
            }
            continue;
        }
        if gdi_on {
            gdi.play(&record, pixmap);
        }
    }
}

/// One metafile record: its type and all its bytes (offsets in records
/// count from the record's start).
#[derive(Clone, Copy)]
pub(crate) struct Record<'a> {
    pub kind: u32,
    pub data: &'a [u8],
}

#[derive(Clone)]
struct Records<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Iterator for Records<'a> {
    type Item = Record<'a>;

    /// A record too short, past the end or not a whole number of words
    /// ends playback.
    fn next(&mut self) -> Option<Record<'a>> {
        let kind = self.bytes.get(self.at..self.at + 8).map(|head| head.u32_at(0))?;
        let size = self.bytes.u32_at(self.at + 4) as usize;
        if size < 8 || !size.is_multiple_of(4) || self.at + size > self.bytes.len() {
            return None;
        }
        let data = &self.bytes[self.at..self.at + size];
        self.at += size;
        Some(Record { kind, data })
    }
}

/// An EMF+ record: type, flags and data.
pub(crate) struct PlusRecord<'a> {
    pub kind: u16,
    pub flags: u16,
    pub data: &'a [u8],
}

/// The EMF+ records an `EMR_COMMENT` carries (none unless it is an EMF+
/// comment).
fn plus_records(comment: &[u8]) -> impl Iterator<Item = PlusRecord<'_>> {
    let is_plus = comment.u32_at(0) == EMR_COMMENT && comment.u32_at(12) == EMF_PLUS;
    let length = (comment.u32_at(8) as usize).saturating_sub(4);
    let body = if is_plus { comment.get(16..16 + length).unwrap_or_default() } else { &[] };
    let mut at = 0usize;
    std::iter::from_fn(move || {
        let head = body.get(at..at + 12)?;
        let size = head.u32_at(4) as usize;
        let data_size = head.u32_at(8) as usize;
        if size < 12 || at + size > body.len() || data_size > size - 12 {
            return None;
        }
        let record = PlusRecord { kind: head.u16_at(0), flags: head.u16_at(2), data: &body[at + 12..at + 12 + data_size] };
        at += size;
        Some(record)
    })
}

/// Little-endian reads that give 0 past the end.
pub(crate) trait Bytes {
    fn u8_at(&self, at: usize) -> u8;
    fn u16_at(&self, at: usize) -> u16;
    fn i16_at(&self, at: usize) -> i16;
    fn u32_at(&self, at: usize) -> u32;
    fn i32_at(&self, at: usize) -> i32;
    fn f32_at(&self, at: usize) -> f32;
}

impl Bytes for [u8] {
    fn u8_at(&self, at: usize) -> u8 {
        self.get(at).copied().unwrap_or(0)
    }

    fn u16_at(&self, at: usize) -> u16 {
        match self.get(at..at + 2) {
            Some(bytes) => u16::from_le_bytes([bytes[0], bytes[1]]),
            None => 0,
        }
    }

    fn i16_at(&self, at: usize) -> i16 {
        self.u16_at(at) as i16
    }

    fn u32_at(&self, at: usize) -> u32 {
        match self.get(at..at + 4) {
            Some(bytes) => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            None => 0,
        }
    }

    fn i32_at(&self, at: usize) -> i32 {
        self.u32_at(at) as i32
    }

    fn f32_at(&self, at: usize) -> f32 {
        f32::from_bits(self.u32_at(at))
    }
}

/// A 2D affine map in double precision: `x' = a·x + c·y + e`,
/// `y' = b·x + d·y + f` (an `XFORM`'s `eM11, eM12, eM21, eM22, eDx, eDy`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Affine {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Affine {
    pub const IDENTITY: Affine = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    pub fn translate(x: f64, y: f64) -> Affine {
        Affine { e: x, f: y, ..Affine::IDENTITY }
    }

    pub fn scale(x: f64, y: f64) -> Affine {
        Affine { a: x, d: y, ..Affine::IDENTITY }
    }

    /// Six floats as an `XFORM` (or an EMF+ matrix) stores them.
    pub fn from_floats(values: [f32; 6]) -> Affine {
        let [a, b, c, d, e, f] = values.map(|value| value as f64);
        Affine { a, b, c, d, e, f }
    }

    /// `self`, then `next`.
    pub fn then(&self, next: &Affine) -> Affine {
        Affine {
            a: self.a * next.a + self.b * next.c,
            b: self.a * next.b + self.b * next.d,
            c: self.c * next.a + self.d * next.c,
            d: self.c * next.b + self.d * next.d,
            e: self.e * next.a + self.f * next.c + next.e,
            f: self.e * next.b + self.f * next.d + next.f,
        }
    }

    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }

    pub fn apply_vector(&self, x: f64, y: f64) -> (f64, f64) {
        (self.a * x + self.c * y, self.b * x + self.d * y)
    }

    pub fn determinant(&self) -> f64 {
        self.a * self.d - self.b * self.c
    }

    pub fn invert(&self) -> Option<Affine> {
        let determinant = self.determinant();
        if determinant == 0.0 || !determinant.is_finite() {
            return None;
        }
        let a = self.d / determinant;
        let b = -self.b / determinant;
        let c = -self.c / determinant;
        let d = self.a / determinant;
        let e = -(self.e * a + self.f * c);
        let f = -(self.e * b + self.f * d);
        Some(Affine { a, b, c, d, e, f })
    }

    /// How much lengths grow, on average over the two axes.
    pub fn mean_scale(&self) -> f64 {
        let x = self.a.hypot(self.b);
        let y = self.c.hypot(self.d);
        (x + y) / 2.0
    }

    pub fn is_axis_aligned(&self) -> bool {
        self.b == 0.0 && self.c == 0.0
    }

    pub fn to_skia(self) -> Transform {
        Transform::from_row(self.a as f32, self.b as f32, self.c as f32, self.d as f32, self.e as f32, self.f as f32)
    }
}

#[cfg(test)]
mod tests;
