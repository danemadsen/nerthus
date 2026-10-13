//! EMF+ records, played as GDI+ plays them into a picture: in world
//! units through the world and page transforms (and those of any
//! containers), anti-aliased unless a record turns smoothing off, pixel
//! centres on whole coordinates unless the pixel offset mode is half,
//! colours with their alpha blended over what is there (or copied, in
//! source-copy compositing).

use std::collections::HashMap;
use std::rc::Rc;

use resvg::tiny_skia::{BlendMode, Color, FillRule, FilterQuality, GradientStop, LineCap, LineJoin, LinearGradient, Mask, Paint, Path, PathBuilder, Pattern, Pixmap, Point, RadialGradient, Shader, SpreadMode, Stroke, StrokeDash, Transform};

use super::text::{self, PlusFont, StringLayout};
use super::{Affine, Bytes, Header, PlusRecord};

const OBJECT: u16 = 0x4008;
const CLEAR: u16 = 0x4009;
const FILL_RECTS: u16 = 0x400A;
const DRAW_RECTS: u16 = 0x400B;
const FILL_POLYGON: u16 = 0x400C;
const DRAW_LINES: u16 = 0x400D;
const FILL_ELLIPSE: u16 = 0x400E;
const DRAW_ELLIPSE: u16 = 0x400F;
const FILL_PIE: u16 = 0x4010;
const DRAW_PIE: u16 = 0x4011;
const DRAW_ARC: u16 = 0x4012;
const FILL_REGION: u16 = 0x4013;
const FILL_PATH: u16 = 0x4014;
const DRAW_PATH: u16 = 0x4015;
const FILL_CLOSED_CURVE: u16 = 0x4016;
const DRAW_CLOSED_CURVE: u16 = 0x4017;
const DRAW_CURVE: u16 = 0x4018;
const DRAW_BEZIERS: u16 = 0x4019;
const DRAW_IMAGE: u16 = 0x401A;
const DRAW_IMAGE_POINTS: u16 = 0x401B;
const DRAW_STRING: u16 = 0x401C;
const SET_RENDERING_ORIGIN: u16 = 0x401D;
const SET_ANTI_ALIAS_MODE: u16 = 0x401E;
const SET_TEXT_RENDERING_HINT: u16 = 0x401F;
const SET_INTERPOLATION_MODE: u16 = 0x4021;
const SET_PIXEL_OFFSET_MODE: u16 = 0x4022;
const SET_COMPOSITING_MODE: u16 = 0x4023;
const SAVE: u16 = 0x4025;
const RESTORE: u16 = 0x4026;
const BEGIN_CONTAINER: u16 = 0x4027;
const BEGIN_CONTAINER_NO_PARAMS: u16 = 0x4028;
const END_CONTAINER: u16 = 0x4029;
const SET_WORLD_TRANSFORM: u16 = 0x402A;
const RESET_WORLD_TRANSFORM: u16 = 0x402B;
const MULTIPLY_WORLD_TRANSFORM: u16 = 0x402C;
const TRANSLATE_WORLD_TRANSFORM: u16 = 0x402D;
const SCALE_WORLD_TRANSFORM: u16 = 0x402E;
const ROTATE_WORLD_TRANSFORM: u16 = 0x402F;
const SET_PAGE_TRANSFORM: u16 = 0x4030;
const RESET_CLIP: u16 = 0x4031;
const SET_CLIP_RECT: u16 = 0x4032;
const SET_CLIP_PATH: u16 = 0x4033;
const SET_CLIP_REGION: u16 = 0x4034;
const OFFSET_CLIP: u16 = 0x4035;
const DRAW_DRIVER_STRING: u16 = 0x4036;
/// `EmfPlusGetDC`: the GDI records after it are played.
pub(super) const GET_DC: u16 = 0x4004;

/// Record flags: the brush is a colour, coordinates are 16-bit,
/// relative, a closed figure, winding fill, post-multiplied transforms.
const SOLID_COLOUR: u16 = 0x8000;
const COMPRESSED: u16 = 0x4000;
const RELATIVE: u16 = 0x0800;
const CLOSED: u16 = 0x2000;
const WINDING_FILL: u16 = 0x2000;
const APPEND: u16 = 0x2000;

/// Units (`UnitType`).
const UNIT_WORLD: u32 = 0;
const UNIT_DISPLAY: u32 = 1;
const UNIT_PIXEL: u32 = 2;
const UNIT_POINT: u32 = 3;
const UNIT_INCH: u32 = 4;
const UNIT_DOCUMENT: u32 = 5;
const UNIT_MILLIMETER: u32 = 6;

/// Object types.
const OBJECT_BRUSH: u16 = 1;
const OBJECT_PEN: u16 = 2;
const OBJECT_PATH: u16 = 3;
const OBJECT_REGION: u16 = 4;
const OBJECT_IMAGE: u16 = 5;
const OBJECT_FONT: u16 = 6;
const OBJECT_STRING_FORMAT: u16 = 7;

/// Clip combinations (`CombineMode`).
const COMBINE_REPLACE: u32 = 0;
const COMBINE_INTERSECT: u32 = 1;
const COMBINE_UNION: u32 = 2;
const COMBINE_XOR: u32 = 3;
const COMBINE_EXCLUDE: u32 = 4;
const COMBINE_COMPLEMENT: u32 = 5;

#[derive(Clone)]
enum Brush {
    Solid([u8; 4]),
    Hatch { style: u32, fore: [u8; 4], back: [u8; 4] },
    Texture { image: Rc<Pixmap>, transform: Affine, wrap: u32 },
    Linear { rect: [f64; 4], stops: Vec<(f32, [u8; 4])>, transform: Affine, wrap: u32 },
    /// A path gradient, drawn as an elliptical gradient over the
    /// boundary's bounds: kept inside the boundary when clamped, its box
    /// repeated otherwise.
    Radial { centre: (f64, f64), centre_colour: [u8; 4], surround: [u8; 4], boundary: Rc<Path>, transform: Affine, wrap: u32 },
}

#[derive(Clone)]
struct Pen {
    width: f64,
    unit: u32,
    start_cap: u32,
    join: u32,
    miter_limit: f32,
    dashes: Option<Vec<f32>>,
    dash_offset: f32,
    brush: Brush,
}

/// A region: a tree of rectangles and paths combined.
#[derive(Clone)]
enum Region {
    Rect([f64; 4]),
    Path(Rc<Path>),
    Empty,
    Infinite,
    Combine(u32, Box<Region>, Box<Region>),
}

#[derive(Clone)]
enum Image {
    Bitmap(Rc<Pixmap>),
    Metafile(Rc<Vec<u8>>),
}

#[derive(Clone)]
struct Font {
    em: f64,
    unit: u32,
    style: i32,
    family: String,
}

#[derive(Clone)]
enum Object {
    Brush(Brush),
    Pen(Pen),
    Path(Rc<Path>),
    Region(Region),
    Image(Image),
    Font(Font),
    Format(StringLayout),
}

#[derive(Clone)]
struct State {
    world: Affine,
    page_unit: u32,
    page_scale: f64,
    /// A container's own space to the picture's device space.
    container: Affine,
    clip: Option<Rc<Mask>>,
    anti_alias: bool,
    pixel_offset_half: bool,
    copy: bool,
    interpolation: u32,
    text_hint: u32,
}

pub(super) struct Player {
    playback: Affine,
    width: u32,
    height: u32,
    /// The recording device's resolution (`LogicalDpiX/Y`).
    dpi: (f64, f64),
    /// Recorded on a display: display units are pixels.
    display: bool,
    objects: HashMap<u8, Object>,
    /// An object arriving in parts: its id, type, total size and data so
    /// far.
    partial: Option<(u8, u16, usize, Vec<u8>)>,
    state: State,
    saved: Vec<(u32, State)>,
    depth: u32,
}

impl Player {
    pub(super) fn new(header: &Header, plus_header: &[u8], playback: Affine, width: u32, height: u32, depth: u32) -> Player {
        let (header_dpi_x, header_dpi_y) = header.dpi();
        let logical_x = plus_header.u32_at(8) as f64;
        let logical_y = plus_header.u32_at(12) as f64;
        let dpi = (if logical_x > 0.0 { logical_x } else { header_dpi_x }, if logical_y > 0.0 { logical_y } else { header_dpi_y });
        let display = plus_header.u32_at(4) & 1 != 0;
        let state = State {
            world: Affine::IDENTITY,
            page_unit: UNIT_DISPLAY,
            page_scale: 1.0,
            container: Affine::IDENTITY,
            clip: None,
            anti_alias: true,
            pixel_offset_half: false,
            copy: false,
            interpolation: 7,
            text_hint: 5,
        };
        Player { playback, width, height, dpi, display, objects: HashMap::new(), partial: None, state, saved: Vec::new(), depth }
    }

    /// Pixels per unit.
    fn unit_scale(&self, unit: u32) -> (f64, f64) {
        let (x, y) = self.dpi;
        match unit {
            UNIT_PIXEL => (1.0, 1.0),
            UNIT_DISPLAY if self.display => (1.0, 1.0),
            UNIT_DISPLAY => (x / 100.0, y / 100.0),
            UNIT_POINT => (x / 72.0, y / 72.0),
            UNIT_INCH => (x, y),
            UNIT_DOCUMENT => (x / 300.0, y / 300.0),
            UNIT_MILLIMETER => (x / 25.4, y / 25.4),
            _ => (1.0, 1.0),
        }
    }

    fn page(&self) -> Affine {
        let (x, y) = self.unit_scale(self.state.page_unit);
        Affine::scale(x * self.state.page_scale, y * self.state.page_scale)
    }

    /// World units to the picture's pixels (on tiny-skia's grid).
    fn transform(&self) -> Affine {
        let offset = if self.state.pixel_offset_half { 0.0 } else { 0.5 };
        self.state.world.then(&self.page()).then(&self.state.container).then(&self.playback).then(&Affine::translate(offset, offset))
    }

    pub(super) fn play(&mut self, record: &PlusRecord, pixmap: &mut Pixmap) {
        let data = record.data;
        let flags = record.flags;
        match record.kind {
            OBJECT => self.object(flags, data),
            CLEAR => {
                let colour = argb(data.u32_at(0));
                self.clear(colour, pixmap);
            }
            FILL_RECTS => {
                let count = data.u32_at(4) as usize;
                let rects = read_rects(data, 8, count, flags & COMPRESSED != 0);
                let mut builder = PathBuilder::new();
                for rect in rects {
                    add_rect(&mut builder, rect);
                }
                if let Some(path) = builder.finish() {
                    let brush = self.brush(flags, data.u32_at(0));
                    self.fill(&path, &brush, FillRule::Winding, pixmap);
                }
            }
            DRAW_RECTS => {
                let count = data.u32_at(0) as usize;
                let rects = read_rects(data, 4, count, flags & COMPRESSED != 0);
                let mut builder = PathBuilder::new();
                for rect in rects {
                    add_rect(&mut builder, rect);
                }
                if let Some(path) = builder.finish() {
                    self.draw(&path, (flags & 0xFF) as u8, pixmap);
                }
            }
            FILL_POLYGON => {
                let count = data.u32_at(4) as usize;
                let points = read_points(data, 8, count, flags);
                if let Some(path) = polyline(&points, true) {
                    let brush = self.brush(flags, data.u32_at(0));
                    self.fill(&path, &brush, FillRule::EvenOdd, pixmap);
                }
            }
            DRAW_LINES => {
                let count = data.u32_at(0) as usize;
                let points = read_points(data, 4, count, flags);
                if let Some(path) = polyline(&points, flags & CLOSED != 0) {
                    self.draw(&path, (flags & 0xFF) as u8, pixmap);
                }
            }
            FILL_ELLIPSE => {
                let rect = read_rect(data, 4, flags & COMPRESSED != 0);
                let brush = self.brush(flags, data.u32_at(0));
                if let Some(path) = ellipse(rect) {
                    self.fill(&path, &brush, FillRule::Winding, pixmap);
                }
            }
            DRAW_ELLIPSE => {
                let rect = read_rect(data, 0, flags & COMPRESSED != 0);
                if let Some(path) = ellipse(rect) {
                    self.draw(&path, (flags & 0xFF) as u8, pixmap);
                }
            }
            FILL_PIE => {
                let (start, sweep) = (data.f32_at(4) as f64, data.f32_at(8) as f64);
                let rect = read_rect(data, 12, flags & COMPRESSED != 0);
                let brush = self.brush(flags, data.u32_at(0));
                if let Some(path) = arc(rect, start, sweep, true) {
                    self.fill(&path, &brush, FillRule::Winding, pixmap);
                }
            }
            DRAW_PIE | DRAW_ARC => {
                let (start, sweep) = (data.f32_at(0) as f64, data.f32_at(4) as f64);
                let rect = read_rect(data, 8, flags & COMPRESSED != 0);
                if let Some(path) = arc(rect, start, sweep, record.kind == DRAW_PIE) {
                    self.draw(&path, (flags & 0xFF) as u8, pixmap);
                }
            }
            FILL_REGION => {
                let Some(Object::Region(region)) = self.objects.get(&((flags & 0xFF) as u8)).cloned() else {
                    return;
                };
                let brush = self.brush(flags, data.u32_at(0));
                let mask = self.region_mask(&region);
                self.fill_mask(&mask, &brush, pixmap);
            }
            FILL_PATH => {
                let Some(Object::Path(path)) = self.objects.get(&((flags & 0xFF) as u8)).cloned() else {
                    return;
                };
                let brush = self.brush(flags, data.u32_at(0));
                self.fill(&path, &brush, path_fill_rule(&path), pixmap);
            }
            DRAW_PATH => {
                let Some(Object::Path(path)) = self.objects.get(&((flags & 0xFF) as u8)).cloned() else {
                    return;
                };
                self.draw(&path, data.u32_at(0) as u8, pixmap);
            }
            FILL_CLOSED_CURVE => {
                let tension = data.f32_at(4) as f64;
                let count = data.u32_at(8) as usize;
                let points = read_points(data, 12, count, flags);
                if let Some(path) = curve(&points, tension, true, 0, points.len()) {
                    let brush = self.brush(flags, data.u32_at(0));
                    let rule = if flags & WINDING_FILL != 0 { FillRule::Winding } else { FillRule::EvenOdd };
                    self.fill(&path, &brush, rule, pixmap);
                }
            }
            DRAW_CLOSED_CURVE => {
                let tension = data.f32_at(0) as f64;
                let count = data.u32_at(4) as usize;
                let points = read_points(data, 8, count, flags);
                if let Some(path) = curve(&points, tension, true, 0, points.len()) {
                    self.draw(&path, (flags & 0xFF) as u8, pixmap);
                }
            }
            DRAW_CURVE => {
                let tension = data.f32_at(0) as f64;
                let offset = data.u32_at(4) as usize;
                let segments = data.u32_at(8) as usize;
                let count = data.u32_at(12) as usize;
                let points = read_points(data, 16, count, flags);
                if let Some(path) = curve(&points, tension, false, offset, segments) {
                    self.draw(&path, (flags & 0xFF) as u8, pixmap);
                }
            }
            DRAW_BEZIERS => {
                let count = data.u32_at(0) as usize;
                let points = read_points(data, 4, count, flags);
                if points.len() >= 4 {
                    let mut builder = PathBuilder::new();
                    builder.move_to(points[0].0, points[0].1);
                    for segment in points[1..].as_chunks::<3>().0 {
                        builder.cubic_to(segment[0].0, segment[0].1, segment[1].0, segment[1].1, segment[2].0, segment[2].1);
                    }
                    if let Some(path) = builder.finish() {
                        self.draw(&path, (flags & 0xFF) as u8, pixmap);
                    }
                }
            }
            DRAW_IMAGE => {
                let source = [data.f32_at(8), data.f32_at(12), data.f32_at(16), data.f32_at(20)].map(|value| value as f64);
                let destination = read_rect(data, 24, flags & COMPRESSED != 0).map(|value| value as f64);
                let corners = [(destination[0], destination[1]), (destination[0] + destination[2], destination[1]), (destination[0], destination[1] + destination[3])];
                self.draw_image((flags & 0xFF) as u8, source, corners, pixmap);
            }
            DRAW_IMAGE_POINTS => {
                let source = [data.f32_at(8), data.f32_at(12), data.f32_at(16), data.f32_at(20)].map(|value| value as f64);
                let points = read_points(data, 28, 3, flags);
                if points.len() == 3 {
                    let corners = [0, 1, 2].map(|index| (points[index].0 as f64, points[index].1 as f64));
                    self.draw_image((flags & 0xFF) as u8, source, corners, pixmap);
                }
            }
            DRAW_STRING => self.draw_string(flags, data, pixmap),
            DRAW_DRIVER_STRING => self.draw_driver_string(flags, data, pixmap),
            SET_RENDERING_ORIGIN => {}
            SET_ANTI_ALIAS_MODE => self.state.anti_alias = flags & 1 != 0,
            SET_TEXT_RENDERING_HINT => self.state.text_hint = (flags & 0xFF) as u32,
            SET_INTERPOLATION_MODE => self.state.interpolation = (flags & 0xFF) as u32,
            SET_PIXEL_OFFSET_MODE => {
                let mode = flags & 0xFF;
                self.state.pixel_offset_half = mode == 2 || mode == 4;
            }
            SET_COMPOSITING_MODE => self.state.copy = flags & 0xFF == 1,
            SAVE => {
                let index = data.u32_at(0);
                self.saved.push((index, self.state.clone()));
            }
            RESTORE | END_CONTAINER => {
                let index = data.u32_at(0);
                if let Some(position) = self.saved.iter().rposition(|(saved, _)| *saved == index) {
                    let (_, state) = self.saved.remove(position);
                    self.saved.truncate(position);
                    self.state = state;
                }
            }
            BEGIN_CONTAINER => {
                let destination = [data.f32_at(0), data.f32_at(4), data.f32_at(8), data.f32_at(12)].map(|value| value as f64);
                let source = [data.f32_at(16), data.f32_at(20), data.f32_at(24), data.f32_at(28)].map(|value| value as f64);
                let index = data.u32_at(32);
                let unit = ((flags >> 8) & 0xFF) as u32;
                self.saved.push((index, self.state.clone()));
                // The source rectangle (in its unit) lands on the destination.
                let (unit_x, unit_y) = self.unit_scale(unit);
                let source_pixels = [source[0] * unit_x, source[1] * unit_y, source[2] * unit_x, source[3] * unit_y];
                let scale_x = if source_pixels[2] != 0.0 { destination[2] / source_pixels[2] } else { 1.0 };
                let scale_y = if source_pixels[3] != 0.0 { destination[3] / source_pixels[3] } else { 1.0 };
                let mapping = Affine { a: scale_x, b: 0.0, c: 0.0, d: scale_y, e: destination[0] - source_pixels[0] * scale_x, f: destination[1] - source_pixels[1] * scale_y };
                let outer = self.state.world.then(&self.page()).then(&self.state.container);
                self.state.container = Affine::scale(unit_x, unit_y).then(&mapping).then(&outer);
                self.state.world = Affine::IDENTITY;
                self.state.page_unit = UNIT_PIXEL;
                self.state.page_scale = 1.0;
            }
            BEGIN_CONTAINER_NO_PARAMS => {
                let index = data.u32_at(0);
                self.saved.push((index, self.state.clone()));
                self.state.container = self.state.world.then(&self.page()).then(&self.state.container);
                self.state.world = Affine::IDENTITY;
                self.state.page_unit = UNIT_PIXEL;
                self.state.page_scale = 1.0;
            }
            SET_WORLD_TRANSFORM => self.state.world = read_matrix(data, 0),
            RESET_WORLD_TRANSFORM => self.state.world = Affine::IDENTITY,
            MULTIPLY_WORLD_TRANSFORM => {
                let matrix = read_matrix(data, 0);
                self.multiply(matrix, flags & APPEND != 0);
            }
            TRANSLATE_WORLD_TRANSFORM => {
                let matrix = Affine::translate(data.f32_at(0) as f64, data.f32_at(4) as f64);
                self.multiply(matrix, flags & APPEND != 0);
            }
            SCALE_WORLD_TRANSFORM => {
                let matrix = Affine::scale(data.f32_at(0) as f64, data.f32_at(4) as f64);
                self.multiply(matrix, flags & APPEND != 0);
            }
            ROTATE_WORLD_TRANSFORM => {
                let (sin, cos) = (data.f32_at(0) as f64).to_radians().sin_cos();
                let matrix = Affine { a: cos, b: sin, c: -sin, d: cos, e: 0.0, f: 0.0 };
                self.multiply(matrix, flags & APPEND != 0);
            }
            SET_PAGE_TRANSFORM => {
                self.state.page_unit = (flags & 0xFF) as u32;
                self.state.page_scale = data.f32_at(0) as f64;
            }
            RESET_CLIP => self.state.clip = None,
            SET_CLIP_RECT => {
                let mode = ((flags >> 8) & 0xF) as u32;
                let rect = [data.f32_at(0), data.f32_at(4), data.f32_at(8), data.f32_at(12)].map(|value| value as f64);
                let mask = self.region_mask(&Region::Rect(rect));
                self.combine_clip(mask, mode);
            }
            SET_CLIP_PATH => {
                let mode = ((flags >> 8) & 0xF) as u32;
                if let Some(Object::Path(path)) = self.objects.get(&((flags & 0xFF) as u8)).cloned() {
                    let mask = self.region_mask(&Region::Path(path));
                    self.combine_clip(mask, mode);
                }
            }
            SET_CLIP_REGION => {
                let mode = ((flags >> 8) & 0xF) as u32;
                if let Some(Object::Region(region)) = self.objects.get(&((flags & 0xFF) as u8)).cloned() {
                    let mask = self.region_mask(&region);
                    self.combine_clip(mask, mode);
                }
            }
            OFFSET_CLIP => {
                let (x, y) = self.transform().apply_vector(data.f32_at(0) as f64, data.f32_at(4) as f64);
                self.offset_clip(x.round() as i64, y.round() as i64);
            }
            _ => {}
        }
    }

    fn multiply(&mut self, matrix: Affine, append: bool) {
        self.state.world = if append { self.state.world.then(&matrix) } else { matrix.then(&self.state.world) };
    }

    // Objects.

    fn object(&mut self, flags: u16, data: &[u8]) {
        let id = (flags & 0xFF) as u8;
        let kind = (flags >> 8) & 0x7F;
        let continued = flags & 0x8000 != 0;
        let whole: Vec<u8> = if continued {
            let total = data.u32_at(0) as usize;
            let chunk = data.get(4..).unwrap_or_default();
            let mut partial = match self.partial.take() {
                Some(partial) if partial.0 == id && partial.1 == kind => partial,
                _ => (id, kind, total, Vec::with_capacity(total.min(1 << 26))),
            };
            partial.3.extend_from_slice(chunk);
            if partial.3.len() < partial.2 {
                self.partial = Some(partial);
                return;
            }
            partial.3
        } else {
            match self.partial.take() {
                // The last part of an object arrives without the flag.
                Some(mut partial) if partial.0 == id && partial.1 == kind => {
                    partial.3.extend_from_slice(data);
                    partial.3
                }
                _ => data.to_vec(),
            }
        };
        let object = match kind {
            OBJECT_BRUSH => read_brush(&whole, self.depth).map(Object::Brush),
            OBJECT_PEN => read_pen(&whole, self.depth).map(Object::Pen),
            OBJECT_PATH => read_path(&whole).map(|(path, _)| Object::Path(Rc::new(path))),
            OBJECT_REGION => read_region(&whole).map(Object::Region),
            OBJECT_IMAGE => read_image(&whole).map(Object::Image),
            OBJECT_FONT => read_font(&whole).map(Object::Font),
            OBJECT_STRING_FORMAT => Some(Object::Format(read_format(&whole))),
            _ => None,
        };
        match object {
            Some(object) => {
                self.objects.insert(id, object);
            }
            None => {
                self.objects.remove(&id);
            }
        }
    }

    /// A record's brush: a colour (`S` flag) or a brush object.
    fn brush(&self, flags: u16, value: u32) -> Brush {
        if flags & SOLID_COLOUR != 0 {
            return Brush::Solid(argb(value));
        }
        match self.objects.get(&(value as u8)) {
            Some(Object::Brush(brush)) => brush.clone(),
            _ => Brush::Solid([0, 0, 0, 0]),
        }
    }

    // Painting.

    fn fill(&self, path: &Path, brush: &Brush, rule: FillRule, pixmap: &mut Pixmap) {
        let transform = self.transform();
        let Some(mapped) = path.clone().transform(transform.to_skia()) else {
            return;
        };
        self.paint_path(&mapped, brush, rule, None, pixmap);
    }

    /// Fills `path` (on the picture) with `brush`, inside the clip.
    fn paint_path(&self, path: &Path, brush: &Brush, rule: FillRule, stroke: Option<&Stroke>, pixmap: &mut Pixmap) {
        let transform = self.transform();
        // A tiled brush's tile, and how it lies on the picture.
        let mut tile: Option<(Rc<Pixmap>, Affine, SpreadMode, FilterQuality)> = None;
        let mut boundary_mask = None;
        let shader = match brush {
            Brush::Solid(colour) => Shader::SolidColor(colour_of(*colour)),
            Brush::Hatch { style, fore, back } => {
                let origin = self.playback.apply(0.0, 0.0);
                let placement = Affine::translate(origin.0, origin.1);
                tile = Some((Rc::new(hatch_tile(*style, *fore, *back)), placement, SpreadMode::Repeat, FilterQuality::Nearest));
                Shader::SolidColor(Color::TRANSPARENT)
            }
            Brush::Texture { image, transform: brush_transform, wrap } => {
                tile = Some((image.clone(), brush_transform.then(&transform), spread(*wrap), self.filter_quality()));
                Shader::SolidColor(Color::TRANSPARENT)
            }
            Brush::Linear { rect, stops, transform: brush_transform, wrap } => {
                let stops: Vec<GradientStop> = stops.iter().map(|(position, colour)| GradientStop::new(*position, colour_of(*colour))).collect();
                let start = Point::from_xy(rect[0] as f32, rect[1] as f32);
                let end = Point::from_xy((rect[0] + rect[2]) as f32, rect[1] as f32);
                let mapping = brush_transform.then(&transform).to_skia();
                LinearGradient::new(start, end, stops, spread(*wrap), mapping).unwrap_or(Shader::SolidColor(Color::TRANSPARENT))
            }
            Brush::Radial { boundary, transform: brush_transform, wrap, surround, .. } => {
                let brush_to_picture = brush_transform.then(&transform);
                if *wrap == 4 {
                    // Clamped: the gradient inside its boundary only.
                    if let Some(mapped) = boundary.as_ref().clone().transform(brush_to_picture.to_skia()) {
                        let mut mask = Mask::new(self.width, self.height).expect("the picture's size was checked");
                        mask.fill_path(&mapped, FillRule::Winding, self.state.anti_alias, Transform::identity());
                        if let Some(clip) = self.state.clip.as_deref() {
                            for (value, clip) in mask.data_mut().iter_mut().zip(clip.data()) {
                                *value = ((*value as u16 * *clip as u16) / 255) as u8;
                            }
                        }
                        boundary_mask = Some(mask);
                    }
                    radial_shader(brush, &brush_to_picture).unwrap_or(Shader::SolidColor(colour_of(*surround)))
                } else {
                    // Tiled: the boundary's box, drawn once and repeated.
                    match radial_tile(brush, &brush_to_picture) {
                        Some((pixmap, tile_to_brush)) => {
                            tile = Some((Rc::new(pixmap), tile_to_brush.then(&brush_to_picture), spread(*wrap), FilterQuality::Bilinear));
                            Shader::SolidColor(Color::TRANSPARENT)
                        }
                        None => Shader::SolidColor(colour_of(*surround)),
                    }
                }
            }
        };
        let shader = match &tile {
            Some((pixmap, placement, spread_mode, quality)) => Pattern::new(pixmap.as_ref().as_ref(), *spread_mode, *quality, 1.0, placement.to_skia()),
            None => shader,
        };
        let blend_mode = if self.state.copy { BlendMode::Source } else { BlendMode::SourceOver };
        let paint = Paint { shader, anti_alias: self.state.anti_alias, blend_mode, ..Paint::default() };
        let clip = boundary_mask.as_ref().or(self.state.clip.as_deref());
        match stroke {
            Some(stroke) => pixmap.stroke_path(path, &paint, stroke, Transform::identity(), clip),
            None => pixmap.fill_path(path, &paint, rule, Transform::identity(), clip),
        }
    }

    /// Strokes `path` (world units) with pen `id`.
    fn draw(&self, path: &Path, id: u8, pixmap: &mut Pixmap) {
        let Some(Object::Pen(pen)) = self.objects.get(&id) else {
            return;
        };
        let transform = self.transform();
        // A pen in world units widens with the transform; one in another
        // unit keeps its size on the picture.
        let (width, outline) = if pen.unit == UNIT_WORLD {
            let width_pixels = pen.width * transform.mean_scale();
            let stroke = self.stroke_for(pen, pen.width as f32);
            let outline = if width_pixels < 1.0 {
                None
            } else {
                path.stroke(&stroke, transform.mean_scale() as f32).and_then(|outline| outline.transform(transform.to_skia()))
            };
            (width_pixels, outline)
        } else {
            let (scale, _) = self.unit_scale(pen.unit);
            (pen.width * scale, None)
        };
        match outline {
            Some(outline) => self.paint_path(&outline, &pen.brush, FillRule::Winding, None, pixmap),
            None => {
                let Some(mapped) = path.clone().transform(transform.to_skia()) else {
                    return;
                };
                let stroke = self.stroke_for(pen, if width < 1.0 { 0.0 } else { width as f32 });
                self.paint_path(&mapped, &pen.brush, FillRule::Winding, Some(&stroke), pixmap);
            }
        }
    }

    fn stroke_for(&self, pen: &Pen, width: f32) -> Stroke {
        let line_cap = match pen.start_cap {
            1 | 0x11 => LineCap::Square,
            2 | 0x12 => LineCap::Round,
            _ => LineCap::Butt,
        };
        let line_join = match pen.join {
            1 => LineJoin::Bevel,
            2 => LineJoin::Round,
            _ => LineJoin::Miter,
        };
        let unit = width.max(1e-3);
        let dash = pen.dashes.as_ref().and_then(|dashes| {
            let intervals: Vec<f32> = dashes.iter().map(|length| (length * unit).max(1e-3)).collect();
            StrokeDash::new(intervals, pen.dash_offset * unit)
        });
        Stroke { width, miter_limit: pen.miter_limit.max(1.0), line_cap, line_join, dash }
    }

    fn filter_quality(&self) -> FilterQuality {
        match self.state.interpolation {
            5 => FilterQuality::Nearest,
            1 | 3 | 6 => FilterQuality::Bilinear,
            _ => FilterQuality::Bicubic,
        }
    }

    /// Fills the pixels of a mask with `brush` (a region).
    fn fill_mask(&self, mask: &Mask, brush: &Brush, pixmap: &mut Pixmap) {
        let mut combined = mask.clone();
        if let Some(clip) = self.state.clip.as_deref() {
            for (value, clip) in combined.data_mut().iter_mut().zip(clip.data()) {
                *value = (*value).min(*clip);
            }
        }
        let mut whole = PathBuilder::new();
        whole.push_rect(resvg::tiny_skia::Rect::from_xywh(0.0, 0.0, self.width as f32, self.height as f32).expect("the picture has a size"));
        let Some(path) = whole.finish() else {
            return;
        };
        let player = Player { state: State { clip: Some(Rc::new(combined)), ..self.state.clone() }, objects: HashMap::new(), partial: None, saved: Vec::new(), ..*self };
        player.paint_path(&path, brush, FillRule::Winding, None, pixmap);
    }

    /// `Clear`: the clip filled with a colour, replacing what is there.
    fn clear(&self, colour: [u8; 4], pixmap: &mut Pixmap) {
        let premultiplied = colour_of(colour).premultiply().to_color_u8();
        let rgba = [premultiplied.red(), premultiplied.green(), premultiplied.blue(), premultiplied.alpha()];
        let clip = self.state.clip.as_deref().map(Mask::data);
        for (index, pixel) in pixmap.data_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
            match clip.map(|clip| clip[index]) {
                None | Some(255) => pixel.copy_from_slice(&rgba),
                Some(0) => {}
                Some(coverage) => {
                    for (channel, value) in pixel.iter_mut().zip(rgba) {
                        *channel = ((value as u16 * coverage as u16 + *channel as u16 * (255 - coverage) as u16) / 255) as u8;
                    }
                }
            }
        }
    }

    // Clipping.

    /// A region on the picture, as a mask (regions are not smoothed).
    fn region_mask(&self, region: &Region) -> Mask {
        let mut mask = Mask::new(self.width, self.height).expect("the picture's size was checked");
        let transform = self.transform().to_skia();
        match region {
            Region::Rect(rect) => {
                let mut builder = PathBuilder::new();
                add_rect(&mut builder, rect.map(|value| value as f32));
                if let Some(path) = builder.finish() {
                    mask.fill_path(&path, FillRule::Winding, false, transform);
                }
            }
            Region::Path(path) => mask.fill_path(path, path_fill_rule(path), false, transform),
            Region::Empty => {}
            Region::Infinite => mask.data_mut().fill(255),
            Region::Combine(mode, left, right) => {
                let left = self.region_mask(left);
                let right = self.region_mask(right);
                mask = combine(&left, &right, *mode);
            }
        }
        mask
    }

    fn combine_clip(&mut self, region: Mask, mode: u32) {
        let combined = match (&self.state.clip, mode) {
            (_, COMBINE_REPLACE) => Some(region),
            (None, COMBINE_INTERSECT) => Some(region),
            (None, COMBINE_UNION) => None,
            (None, _) => {
                let mut everything = Mask::new(self.width, self.height).expect("the picture's size was checked");
                everything.data_mut().fill(255);
                Some(combine(&everything, &region, mode))
            }
            (Some(current), _) => Some(combine(current, &region, mode)),
        };
        self.state.clip = combined.map(Rc::new);
    }

    fn offset_clip(&mut self, x: i64, y: i64) {
        let Some(clip) = self.state.clip.as_deref() else {
            return;
        };
        let (width, height) = (self.width as i64, self.height as i64);
        let mut moved = Mask::new(self.width, self.height).expect("the picture's size was checked");
        for row in 0..height {
            for column in 0..width {
                let (from_row, from_column) = (row - y, column - x);
                if (0..height).contains(&from_row) && (0..width).contains(&from_column) {
                    moved.data_mut()[(row * width + column) as usize] = clip.data()[(from_row * width + from_column) as usize];
                }
            }
        }
        self.state.clip = Some(Rc::new(moved));
    }

    // Images.

    /// Draws `source` (pixels of image `id`) onto the parallelogram whose
    /// corners are `corners` (upper left, upper right, lower left; world).
    fn draw_image(&self, id: u8, source: [f64; 4], corners: [(f64, f64); 3], pixmap: &mut Pixmap) {
        let Some(Object::Image(image)) = self.objects.get(&id) else {
            return;
        };
        let transform = self.transform();
        let [origin, along, down] = corners.map(|(x, y)| transform.apply(x, y));
        let bitmap: Rc<Pixmap> = match image {
            Image::Bitmap(bitmap) => bitmap.clone(),
            Image::Metafile(bytes) => {
                // Drawn at its size on the picture, then placed.
                let width = (along.0 - origin.0).hypot(along.1 - origin.1).round().clamp(1.0, 8192.0) as u32;
                let height = (down.0 - origin.0).hypot(down.1 - origin.1).round().clamp(1.0, 8192.0) as u32;
                let Some(mut drawn) = Pixmap::new(width, height) else {
                    return;
                };
                if !super::draw_nested(bytes, &mut drawn, self.depth) {
                    return;
                }
                return self.place(&drawn, [0.0, 0.0, width as f64, height as f64], [origin, along, down], pixmap);
            }
        };
        self.place(&bitmap, source, [origin, along, down], pixmap);
    }

    /// Maps the `source` rectangle of `bitmap` onto a parallelogram on the
    /// picture.
    fn place(&self, bitmap: &Pixmap, source: [f64; 4], [origin, along, down]: [(f64, f64); 3], pixmap: &mut Pixmap) {
        if source[2] == 0.0 || source[3] == 0.0 {
            return;
        }
        let to_unit = Affine { a: 1.0 / source[2], b: 0.0, c: 0.0, d: 1.0 / source[3], e: -source[0] / source[2], f: -source[1] / source[3] };
        let to_picture = Affine { a: along.0 - origin.0, b: along.1 - origin.1, c: down.0 - origin.0, d: down.1 - origin.1, e: origin.0, f: origin.1 };
        let mapping = to_unit.then(&to_picture);
        let mut builder = PathBuilder::new();
        let corners = [(source[0], source[1]), (source[0] + source[2], source[1]), (source[0] + source[2], source[1] + source[3]), (source[0], source[1] + source[3])];
        for (index, (x, y)) in corners.iter().enumerate() {
            let (x, y) = mapping.apply(*x, *y);
            if index == 0 {
                builder.move_to(x as f32, y as f32);
            } else {
                builder.line_to(x as f32, y as f32);
            }
        }
        builder.close();
        let Some(path) = builder.finish() else {
            return;
        };
        let shader = Pattern::new(bitmap.as_ref(), SpreadMode::Pad, self.filter_quality(), 1.0, mapping.to_skia());
        let blend_mode = if self.state.copy { BlendMode::Source } else { BlendMode::SourceOver };
        let paint = Paint { shader, anti_alias: self.state.anti_alias, blend_mode, ..Paint::default() };
        pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), self.state.clip.as_deref());
    }

    // Text.

    /// A font's size in world units: world-unit sizes as they are, others
    /// converted to the page's unit (so the page scale still applies).
    fn font_em(&self, font: &Font) -> f64 {
        if font.unit == UNIT_WORLD {
            return font.em;
        }
        let (font_unit, _) = self.unit_scale(font.unit);
        let (page_unit, _) = self.unit_scale(self.state.page_unit);
        font.em * font_unit / page_unit
    }

    fn draw_string(&self, flags: u16, data: &[u8], pixmap: &mut Pixmap) {
        let Some(Object::Font(font)) = self.objects.get(&((flags & 0xFF) as u8)) else {
            return;
        };
        let brush = self.brush(flags, data.u32_at(0));
        let format = match self.objects.get(&(data.u32_at(4) as u8)) {
            Some(Object::Format(format)) if data.u32_at(4) != u32::MAX => format.clone(),
            _ => StringLayout::default(),
        };
        let length = (data.u32_at(8) as usize).min(data.len().saturating_sub(28) / 2);
        let layout = [data.f32_at(12), data.f32_at(16), data.f32_at(20), data.f32_at(24)].map(|value| value as f64);
        let units: Vec<u16> = (0..length).map(|index| data.u16_at(28 + 2 * index)).collect();
        let plus_font = PlusFont { family: &font.family, style: font.style, em: self.font_em(font) };
        let transform = self.transform();
        let Some((outline, embolden)) = text::lay_out_string(&plus_font, &units, layout, &format, &transform) else {
            return;
        };
        self.paint_text(&outline, embolden, &brush, pixmap);
    }

    fn draw_driver_string(&self, flags: u16, data: &[u8], pixmap: &mut Pixmap) {
        let Some(Object::Font(font)) = self.objects.get(&((flags & 0xFF) as u8)) else {
            return;
        };
        let brush = self.brush(flags, data.u32_at(0));
        let options = data.u32_at(4);
        let has_matrix = data.u32_at(8) != 0;
        let count = data.u32_at(12) as usize;
        if 16 + 10 * count > data.len() {
            return;
        }
        let glyphs: Vec<u16> = (0..count).map(|index| data.u16_at(16 + 2 * index)).collect();
        let positions_at = 16 + 2 * count;
        let positions: Vec<(f64, f64)> = (0..count).map(|index| (data.f32_at(positions_at + 8 * index) as f64, data.f32_at(positions_at + 8 * index + 4) as f64)).collect();
        let matrix = has_matrix.then(|| read_matrix(data, positions_at + 8 * count));
        let plus_font = PlusFont { family: &font.family, style: font.style, em: self.font_em(font) };
        let transform = self.transform();
        let Some((outline, embolden)) = text::lay_out_glyphs(&plus_font, &glyphs, &positions, options, matrix, &transform) else {
            return;
        };
        self.paint_text(&outline, embolden, &brush, pixmap);
    }

    /// Text: smoothed unless the rendering hint asks for whole pixels.
    fn paint_text(&self, outline: &Path, embolden: Option<f32>, brush: &Brush, pixmap: &mut Pixmap) {
        let smooth = !matches!(self.state.text_hint, 1 | 2);
        let player = Player { state: State { anti_alias: smooth, ..self.state.clone() }, objects: HashMap::new(), partial: None, saved: Vec::new(), ..*self };
        player.paint_path(outline, brush, FillRule::Winding, None, pixmap);
        if let Some(width) = embolden {
            let stroke = Stroke { width, ..Stroke::default() };
            player.paint_path(outline, brush, FillRule::Winding, Some(&stroke), pixmap);
        }
    }
}

/// A path gradient as an elliptical gradient over its boundary's box:
/// the centre colour at the centre point, the surrounding colour at the
/// box's edge (`brush_to_target` maps brush space to where it is drawn).
fn radial_shader(brush: &Brush, brush_to_target: &Affine) -> Option<Shader<'static>> {
    let Brush::Radial { centre, centre_colour, surround, boundary, .. } = brush else {
        return None;
    };
    let bounds = boundary.bounds();
    let radius_x = (bounds.width() / 2.0).max(1e-3) as f64;
    let radius_y = (bounds.height() / 2.0).max(1e-3) as f64;
    let middle = (bounds.left() as f64 + radius_x, bounds.top() as f64 + radius_y);
    let ellipse = Affine { a: radius_x, b: 0.0, c: 0.0, d: radius_y, e: middle.0, f: middle.1 };
    let mapping = ellipse.then(brush_to_target).to_skia();
    let offset = ((centre.0 - middle.0) / radius_x, (centre.1 - middle.1) / radius_y);
    let stops = vec![GradientStop::new(0.0, colour_of(*centre_colour)), GradientStop::new(1.0, colour_of(*surround))];
    let start = Point::from_xy(offset.0 as f32, offset.1 as f32);
    RadialGradient::new(start, Point::from_xy(0.0, 0.0), 1.0, stops, SpreadMode::Pad, mapping)
}

/// A tiled path gradient's tile: its boundary's box at about the size it
/// shows on the picture, the gradient drawn inside the boundary; with the
/// map from the tile back to brush space.
fn radial_tile(brush: &Brush, brush_to_picture: &Affine) -> Option<(Pixmap, Affine)> {
    let Brush::Radial { boundary, .. } = brush else {
        return None;
    };
    let bounds = boundary.bounds();
    let scale = brush_to_picture.mean_scale().max(1e-6);
    let width = (bounds.width() as f64 * scale).ceil().clamp(1.0, 2048.0);
    let height = (bounds.height() as f64 * scale).ceil().clamp(1.0, 2048.0);
    let scale_x = width / (bounds.width() as f64).max(1e-6);
    let scale_y = height / (bounds.height() as f64).max(1e-6);
    let brush_to_tile = Affine { a: scale_x, b: 0.0, c: 0.0, d: scale_y, e: -bounds.left() as f64 * scale_x, f: -bounds.top() as f64 * scale_y };
    let mut tile = Pixmap::new(width as u32, height as u32)?;
    let shader = radial_shader(brush, &brush_to_tile)?;
    let outline = boundary.as_ref().clone().transform(brush_to_tile.to_skia())?;
    let paint = Paint { shader, anti_alias: true, ..Paint::default() };
    tile.fill_path(&outline, &paint, FillRule::Winding, Transform::identity(), None);
    Some((tile, brush_to_tile.invert()?))
}

// Reading objects.

fn argb(value: u32) -> [u8; 4] {
    [(value >> 16) as u8, (value >> 8) as u8, value as u8, (value >> 24) as u8]
}

fn colour_of(rgba: [u8; 4]) -> Color {
    Color::from_rgba8(rgba[0], rgba[1], rgba[2], rgba[3])
}

fn spread(wrap: u32) -> SpreadMode {
    match wrap {
        1..=3 => SpreadMode::Reflect,
        4 => SpreadMode::Pad,
        _ => SpreadMode::Repeat,
    }
}

fn read_matrix(data: &[u8], at: usize) -> Affine {
    Affine::from_floats([0, 1, 2, 3, 4, 5].map(|index| data.f32_at(at + 4 * index)))
}

fn read_rect(data: &[u8], at: usize, compressed: bool) -> [f32; 4] {
    if compressed {
        [0, 1, 2, 3].map(|index| data.i16_at(at + 2 * index) as f32)
    } else {
        [0, 1, 2, 3].map(|index| data.f32_at(at + 4 * index))
    }
}

fn read_rects(data: &[u8], at: usize, count: usize, compressed: bool) -> Vec<[f32; 4]> {
    let size = if compressed { 8 } else { 16 };
    let count = count.min(data.len().saturating_sub(at) / size);
    (0..count).map(|index| read_rect(data, at + size * index, compressed)).collect()
}

/// Points: floats, 16-bit integers (`C`) or relative ones (`P`), each of
/// those a 7-bit or 15-bit integer.
fn read_points(data: &[u8], at: usize, count: usize, flags: u16) -> Vec<(f32, f32)> {
    let mut points = Vec::with_capacity(count.min(1 << 20));
    if flags & RELATIVE != 0 {
        let mut offset = at;
        let read_integer = |offset: &mut usize| -> Option<i32> {
            let first = *data.get(*offset)?;
            if first & 0x80 == 0 {
                *offset += 1;
                // Seven bits, signed.
                Some(((first << 1) as i8 >> 1) as i32)
            } else {
                let second = *data.get(*offset + 1)?;
                *offset += 2;
                let value = (((first & 0x7F) as u16) << 8) | second as u16;
                Some(((value << 1) as i16 >> 1) as i32)
            }
        };
        let (mut x, mut y) = (0.0f32, 0.0f32);
        for _ in 0..count {
            let (Some(dx), Some(dy)) = (read_integer(&mut offset), read_integer(&mut offset)) else {
                break;
            };
            x += dx as f32;
            y += dy as f32;
            points.push((x, y));
        }
        return points;
    }
    let compressed = flags & COMPRESSED != 0;
    let size = if compressed { 4 } else { 8 };
    let count = count.min(data.len().saturating_sub(at) / size);
    for index in 0..count {
        let point = if compressed {
            (data.i16_at(at + 4 * index) as f32, data.i16_at(at + 4 * index + 2) as f32)
        } else {
            (data.f32_at(at + 8 * index), data.f32_at(at + 8 * index + 4))
        };
        points.push(point);
    }
    points
}

/// An `EmfPlusPath`: points, then their types (or runs of them); also
/// returns its size in bytes.
fn read_path(data: &[u8]) -> Option<(Path, usize)> {
    let count = data.u32_at(4) as usize;
    let point_flags = data.u32_at(8);
    let relative = point_flags & 0x0800 != 0;
    let run_length = point_flags & 0x1000 != 0;
    let compressed = point_flags & 0x4000 != 0;
    let flags = (if relative { RELATIVE } else { 0 }) | (if compressed { COMPRESSED } else { 0 });
    let points = read_points(data, 12, count, flags);
    let points_size = if relative {
        // Relative points are as long as they read; measure by re-reading.
        measure_relative(data, 12, count)
    } else if compressed {
        4 * count
    } else {
        8 * count
    };
    let mut at = 12 + points_size;
    let mut types = Vec::with_capacity(count);
    if run_length {
        while types.len() < count && at + 1 < data.len() {
            let run = (data[at] & 0x3F) as usize;
            let kind = data[at + 1];
            types.extend(std::iter::repeat_n(kind, run));
            at += 2;
        }
    } else {
        types.extend(data.get(at..at + count)?.iter().copied());
        at += count;
    }
    let size = 12 + points_size + (at - 12 - points_size).next_multiple_of(4);
    let mut builder = PathBuilder::new();
    let mut index = 0;
    while index < points.len().min(types.len()) {
        let kind = types[index];
        let (x, y) = points[index];
        match kind & 0x07 {
            0 => builder.move_to(x, y),
            3 if index + 2 < points.len() => {
                let (x2, y2) = points[index + 1];
                let (x3, y3) = points[index + 2];
                builder.cubic_to(x, y, x2, y2, x3, y3);
                index += 2;
            }
            _ => builder.line_to(x, y),
        }
        if types.get(index).is_some_and(|kind| kind & 0x80 != 0) {
            builder.close();
        }
        index += 1;
    }
    builder.finish().map(|path| (path, size))
}

fn measure_relative(data: &[u8], at: usize, count: usize) -> usize {
    let mut offset = at;
    for _ in 0..count * 2 {
        let Some(first) = data.get(offset) else {
            break;
        };
        offset += if first & 0x80 == 0 { 1 } else { 2 };
    }
    offset - at
}

/// Paths carry no fill rule of their own here: GDI+ fills them alternate
/// unless their creator asked for winding, which the object does not
/// record.
fn path_fill_rule(_path: &Path) -> FillRule {
    FillRule::EvenOdd
}

/// An `EmfPlusRegion`: its node tree.
fn read_region(data: &[u8]) -> Option<Region> {
    let mut at = 8;
    read_region_node(data, &mut at, 0)
}

fn read_region_node(data: &[u8], at: &mut usize, depth: u32) -> Option<Region> {
    if depth > 64 {
        return None;
    }
    let kind = data.get(*at..*at + 4).map(|bytes| bytes.u32_at(0))?;
    *at += 4;
    match kind {
        1..=5 => {
            let left = read_region_node(data, at, depth + 1)?;
            let right = read_region_node(data, at, depth + 1)?;
            Some(Region::Combine(kind, Box::new(left), Box::new(right)))
        }
        0x1000_0000 => {
            let rect = [0, 1, 2, 3].map(|index| data.f32_at(*at + 4 * index) as f64);
            *at += 16;
            Some(Region::Rect(rect))
        }
        0x1000_0001 => {
            let size = data.u32_at(*at) as usize;
            let path = data.get(*at + 4..*at + 4 + size).and_then(read_path);
            *at += 4 + size;
            Some(path.map(|(path, _)| Region::Path(Rc::new(path))).unwrap_or(Region::Empty))
        }
        0x1000_0002 => Some(Region::Empty),
        0x1000_0003 => Some(Region::Infinite),
        _ => None,
    }
}

/// Region and clip combinations: intersect, union, xor, exclude (the
/// second taken from the first) and complement (the first taken from the
/// second).
fn combine(first: &Mask, second: &Mask, mode: u32) -> Mask {
    let mut result = first.clone();
    for (value, other) in result.data_mut().iter_mut().zip(second.data()) {
        let (a, b) = (*value, *other);
        *value = match mode {
            COMBINE_INTERSECT => a.min(b),
            COMBINE_UNION => a.max(b),
            COMBINE_XOR => a ^ b,
            COMBINE_EXCLUDE => a.min(255 - b),
            COMBINE_COMPLEMENT => b.min(255 - a),
            _ => b,
        };
    }
    result
}

/// An `EmfPlusBrush`.
fn read_brush(data: &[u8], depth: u32) -> Option<Brush> {
    let kind = data.u32_at(4);
    let body = data.get(8..)?;
    match kind {
        0 => Some(Brush::Solid(argb(body.u32_at(0)))),
        1 => Some(Brush::Hatch { style: body.u32_at(0), fore: argb(body.u32_at(4)), back: argb(body.u32_at(8)) }),
        2 => {
            let flags = body.u32_at(0);
            let wrap = body.u32_at(4);
            let mut at = 8;
            let mut transform = Affine::IDENTITY;
            if flags & 0x02 != 0 {
                transform = read_matrix(body, at);
                at += 24;
            }
            match read_image(body.get(at..)?)? {
                Image::Bitmap(image) => Some(Brush::Texture { image, transform, wrap }),
                Image::Metafile(bytes) => {
                    let header = super::Header::parse(&bytes)?;
                    let [_, _, width, height] = header.frame_pixels();
                    let mut drawn = Pixmap::new(width.round().clamp(1.0, 4096.0) as u32, height.round().clamp(1.0, 4096.0) as u32)?;
                    super::draw_nested(&bytes, &mut drawn, depth);
                    Some(Brush::Texture { image: Rc::new(drawn), transform, wrap })
                }
            }
        }
        3 => read_path_gradient(body),
        4 => {
            let flags = body.u32_at(0);
            let wrap = body.u32_at(4);
            let rect = [8, 12, 16, 20].map(|at| body.f32_at(at) as f64);
            let start = argb(body.u32_at(24));
            let end = argb(body.u32_at(28));
            let mut at = 40;
            let mut transform = Affine::IDENTITY;
            if flags & 0x02 != 0 {
                transform = read_matrix(body, at);
                at += 24;
            }
            let mut stops = vec![(0.0, start), (1.0, end)];
            if flags & 0x04 != 0 {
                // Preset colours: positions, then colours.
                let count = body.u32_at(at) as usize;
                let positions: Vec<f32> = (0..count).map(|index| body.f32_at(at + 4 + 4 * index)).collect();
                let colours_at = at + 4 + 4 * count;
                stops = positions.iter().enumerate().map(|(index, position)| (*position, argb(body.u32_at(colours_at + 4 * index)))).collect();
            } else if flags & 0x08 != 0 {
                // Blend factors: how far towards the end colour, by position.
                let count = body.u32_at(at) as usize;
                let factors_at = at + 4 + 4 * count;
                stops = (0..count)
                    .map(|index| {
                        let position = body.f32_at(at + 4 + 4 * index);
                        let factor = body.f32_at(factors_at + 4 * index).clamp(0.0, 1.0);
                        let colour = [0, 1, 2, 3].map(|channel| (start[channel] as f32 + (end[channel] as f32 - start[channel] as f32) * factor).round() as u8);
                        (position, colour)
                    })
                    .collect();
            }
            Some(Brush::Linear { rect, stops, transform, wrap })
        }
        _ => None,
    }
}

/// A path gradient: centre colour and point, the surrounding colours,
/// and the boundary (a path, or points).
fn read_path_gradient(body: &[u8]) -> Option<Brush> {
    let flags = body.u32_at(0);
    let wrap = body.u32_at(4);
    let centre_colour = argb(body.u32_at(8));
    let centre = (body.f32_at(12) as f64, body.f32_at(16) as f64);
    let surround_count = body.u32_at(20) as usize;
    let surround_colours: Vec<[u8; 4]> = (0..surround_count.min(4096)).map(|index| argb(body.u32_at(24 + 4 * index))).collect();
    let surround = average(&surround_colours).unwrap_or(centre_colour);
    let mut at = 24 + 4 * surround_count;
    let boundary = if flags & 0x01 != 0 {
        let size = body.u32_at(at) as usize;
        let path = body.get(at + 4..at + 4 + size).and_then(read_path).map(|(path, _)| path);
        at += 4 + size;
        path
    } else {
        let count = body.u32_at(at) as usize;
        let points = read_points(body, at + 4, count, 0);
        at += 4 + 8 * count;
        polyline(&points, true)
    }?;
    let transform = if flags & 0x02 != 0 { read_matrix(body, at) } else { Affine::IDENTITY };
    Some(Brush::Radial { centre, centre_colour, surround, boundary: Rc::new(boundary), transform, wrap })
}

fn average(colours: &[[u8; 4]]) -> Option<[u8; 4]> {
    if colours.is_empty() {
        return None;
    }
    let mut sums = [0u32; 4];
    for colour in colours {
        for channel in 0..4 {
            sums[channel] += colour[channel] as u32;
        }
    }
    Some(sums.map(|sum| (sum / colours.len() as u32) as u8))
}

/// An `EmfPlusPen`: width, unit, the optional parts its flags name, then
/// its brush.
fn read_pen(data: &[u8], depth: u32) -> Option<Pen> {
    let flags = data.u32_at(8);
    let unit = data.u32_at(12);
    let width = data.f32_at(16) as f64;
    let mut at = 20;
    let mut pen = Pen { width, unit, start_cap: 0, join: 0, miter_limit: 10.0, dashes: None, dash_offset: 0.0, brush: Brush::Solid([0, 0, 0, 255]) };
    if flags & 0x01 != 0 {
        at += 24;
    }
    if flags & 0x02 != 0 {
        pen.start_cap = data.u32_at(at);
        at += 4;
    }
    if flags & 0x04 != 0 {
        at += 4;
    }
    if flags & 0x08 != 0 {
        pen.join = data.u32_at(at);
        at += 4;
    }
    if flags & 0x10 != 0 {
        pen.miter_limit = data.f32_at(at);
        at += 4;
    }
    if flags & 0x20 != 0 {
        pen.dashes = match data.u32_at(at) {
            1 => Some(vec![3.0, 1.0]),
            2 => Some(vec![1.0, 1.0]),
            3 => Some(vec![3.0, 1.0, 1.0, 1.0]),
            4 => Some(vec![3.0, 1.0, 1.0, 1.0, 1.0, 1.0]),
            _ => None,
        };
        at += 4;
    }
    if flags & 0x40 != 0 {
        at += 4;
    }
    if flags & 0x80 != 0 {
        pen.dash_offset = data.f32_at(at);
        at += 4;
    }
    if flags & 0x100 != 0 {
        let count = data.u32_at(at) as usize;
        let dashes: Vec<f32> = (0..count.min(256)).map(|index| data.f32_at(at + 4 + 4 * index)).collect();
        if dashes.len() >= 2 && dashes.len().is_multiple_of(2) {
            pen.dashes = Some(dashes);
        }
        at += 4 + 4 * count;
    }
    if flags & 0x200 != 0 {
        at += 4;
    }
    if flags & 0x400 != 0 {
        let count = data.u32_at(at) as usize;
        at += 4 + 4 * count;
    }
    if flags & 0x800 != 0 {
        at += 4 + data.u32_at(at) as usize;
    }
    if flags & 0x1000 != 0 {
        at += 4 + data.u32_at(at) as usize;
    }
    if let Some(brush) = data.get(at..).and_then(|rest| read_brush(rest, depth)) {
        pen.brush = brush;
    }
    Some(pen)
}

/// An `EmfPlusImage`: a bitmap (pixels or a compressed file) or a
/// metafile.
fn read_image(data: &[u8]) -> Option<Image> {
    match data.u32_at(4) {
        1 => read_bitmap(data.get(8..)?).map(|pixmap| Image::Bitmap(Rc::new(pixmap))),
        2 => {
            let kind = data.u32_at(8);
            let size = data.u32_at(12) as usize;
            let bytes = data.get(16..16 + size)?;
            // EMF, EMF+ only and EMF+ dual; WMF is not read.
            (3..=5).contains(&kind).then(|| Image::Metafile(Rc::new(bytes.to_vec())))
        }
        _ => None,
    }
}

/// `EmfPlusBitmap`: width, height, stride, pixel format and type, then
/// the pixels (with a palette for indexed formats) or a whole file.
fn read_bitmap(data: &[u8]) -> Option<Pixmap> {
    let width = data.i32_at(0);
    let height = data.i32_at(4);
    let stride = data.i32_at(8).unsigned_abs() as usize;
    let format = data.u32_at(12);
    let kind = data.u32_at(16);
    let body = data.get(20..)?;
    if kind == 1 {
        let decoded = image::load_from_memory(body).ok()?.to_rgba8();
        return rgba_pixmap(decoded.width(), decoded.height(), |x, y| decoded.get_pixel(x, y).0);
    }
    if width <= 0 || height <= 0 || (width as u64) * (height as u64) > 1 << 28 {
        return None;
    }
    let (width, height) = (width as u32, height as u32);
    let bits = (format >> 8) & 0xFF;
    let indexed = format & 0x0001_0000 != 0;
    let (palette, pixels) = if indexed {
        let count = body.u32_at(4) as usize;
        let palette: Vec<[u8; 4]> = (0..count.min(256)).map(|index| argb(body.u32_at(8 + 4 * index))).collect();
        (palette, body.get(8 + 4 * count..)?)
    } else {
        (Vec::new(), body)
    };
    let stride = if stride == 0 { (width as usize * bits as usize).div_ceil(32) * 4 } else { stride };
    rgba_pixmap(width, height, |x, y| {
        let row = &pixels[(y as usize * stride).min(pixels.len())..];
        let x = x as usize;
        let byte = |at: usize| row.get(at).copied().unwrap_or(0);
        let word = |at: usize| u16::from_le_bytes([byte(at), byte(at + 1)]);
        match format {
            0x0003_0101 => palette.get(((byte(x / 8) >> (7 - x % 8)) & 1) as usize).copied().unwrap_or([0, 0, 0, 255]),
            0x0003_0402 => {
                let value = byte(x / 2);
                let index = if x.is_multiple_of(2) { value >> 4 } else { value & 15 };
                palette.get(index as usize).copied().unwrap_or([0, 0, 0, 255])
            }
            0x0003_0803 => palette.get(byte(x) as usize).copied().unwrap_or([0, 0, 0, 255]),
            0x0010_1004 => {
                let grey = (word(2 * x) >> 8) as u8;
                [grey, grey, grey, 255]
            }
            0x0002_1005 | 0x0006_1007 => {
                let value = word(2 * x);
                let five = |shift: u16| (((value >> shift) & 31) * 255 / 31) as u8;
                let alpha = if format == 0x0006_1007 && value & 0x8000 == 0 { 0 } else { 255 };
                [five(10), five(5), five(0), alpha]
            }
            0x0002_1006 => {
                let value = word(2 * x);
                [(((value >> 11) & 31) * 255 / 31) as u8, (((value >> 5) & 63) * 255 / 63) as u8, ((value & 31) * 255 / 31) as u8, 255]
            }
            0x0002_1808 => [byte(3 * x + 2), byte(3 * x + 1), byte(3 * x), 255],
            0x0002_2009 => [byte(4 * x + 2), byte(4 * x + 1), byte(4 * x), 255],
            0x0026_200A => [byte(4 * x + 2), byte(4 * x + 1), byte(4 * x), byte(4 * x + 3)],
            0x000E_200B => {
                let alpha = byte(4 * x + 3);
                let straight = |value: u8| if alpha == 0 { 0 } else { ((value as u16 * 255 + alpha as u16 / 2) / alpha as u16).min(255) as u8 };
                [straight(byte(4 * x + 2)), straight(byte(4 * x + 1)), straight(byte(4 * x)), alpha]
            }
            0x0010_300C => [(word(6 * x + 4) >> 5).min(255) as u8, (word(6 * x + 2) >> 5).min(255) as u8, (word(6 * x) >> 5).min(255) as u8, 255],
            0x0034_400D | 0x001A_400E => [(word(8 * x + 4) >> 5).min(255) as u8, (word(8 * x + 2) >> 5).min(255) as u8, (word(8 * x) >> 5).min(255) as u8, (word(8 * x + 6) >> 5).min(255) as u8],
            _ => [0, 0, 0, 0],
        }
    })
}

/// A pixmap from straight RGBA pixels.
fn rgba_pixmap(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(width, height)?;
    let data = pixmap.data_mut();
    for y in 0..height {
        for x in 0..width {
            let [red, green, blue, alpha] = pixel(x, y);
            let premultiply = |value: u8| ((value as u16 * alpha as u16 + 127) / 255) as u8;
            let at = ((y * width + x) * 4) as usize;
            data[at..at + 4].copy_from_slice(&[premultiply(red), premultiply(green), premultiply(blue), alpha]);
        }
    }
    Some(pixmap)
}

fn read_font(data: &[u8]) -> Option<Font> {
    let em = data.f32_at(4) as f64;
    let unit = data.u32_at(8);
    let style = data.i32_at(12);
    let length = (data.u32_at(20) as usize).min(data.len().saturating_sub(24) / 2);
    let units: Vec<u16> = (0..length).map(|index| data.u16_at(24 + 2 * index)).collect();
    let family = String::from_utf16_lossy(&units);
    Some(Font { em, unit, style, family })
}

fn read_format(data: &[u8]) -> StringLayout {
    StringLayout {
        flags: data.u32_at(4),
        alignment: data.u32_at(12),
        line_alignment: data.u32_at(16),
        leading_margin: data.f32_at(36) as f64,
        trailing_margin: data.f32_at(40) as f64,
    }
}

// Shapes.

fn add_rect(builder: &mut PathBuilder, rect: [f32; 4]) {
    let [x, y, width, height] = rect;
    builder.move_to(x, y);
    builder.line_to(x + width, y);
    builder.line_to(x + width, y + height);
    builder.line_to(x, y + height);
    builder.close();
}

fn polyline(points: &[(f32, f32)], closed: bool) -> Option<Path> {
    let (first, rest) = points.split_first()?;
    let mut builder = PathBuilder::new();
    builder.move_to(first.0, first.1);
    for (x, y) in rest {
        builder.line_to(*x, *y);
    }
    if closed {
        builder.close();
    }
    builder.finish()
}

fn ellipse(rect: [f32; 4]) -> Option<Path> {
    let [x, y, width, height] = rect;
    let oval = resvg::tiny_skia::Rect::from_xywh(x.min(x + width), y.min(y + height), width.abs(), height.abs())?;
    PathBuilder::from_oval(oval)
}

/// An arc of the ellipse in `rect`, from `start` through `sweep` degrees
/// (clockwise, measured as angles of the ellipse's own points); a pie
/// joins it to the centre.
fn arc(rect: [f32; 4], start: f64, sweep: f64, pie: bool) -> Option<Path> {
    let [x, y, width, height] = rect.map(|value| value as f64);
    let radii = (width / 2.0, height / 2.0);
    let centre = (x + radii.0, y + radii.1);
    if radii.0 <= 0.0 || radii.1 <= 0.0 {
        return None;
    }
    let sweep = sweep.clamp(-360.0, 360.0);
    // The parameter of the point at a geometric angle.
    let parameter = |degrees: f64| {
        let (sin, cos) = degrees.to_radians().sin_cos();
        (sin / radii.1).atan2(cos / radii.0)
    };
    let start_parameter = parameter(start);
    let mut end_parameter = parameter(start + sweep);
    if sweep >= 0.0 {
        while end_parameter < start_parameter {
            end_parameter += std::f64::consts::TAU;
        }
        if sweep >= 360.0 {
            end_parameter = start_parameter + std::f64::consts::TAU;
        }
    } else {
        while end_parameter > start_parameter {
            end_parameter -= std::f64::consts::TAU;
        }
        if sweep <= -360.0 {
            end_parameter = start_parameter - std::f64::consts::TAU;
        }
    }
    let mut builder = PathBuilder::new();
    let first = (centre.0 + radii.0 * start_parameter.cos(), centre.1 + radii.1 * start_parameter.sin());
    if pie {
        builder.move_to(centre.0 as f32, centre.1 as f32);
        builder.line_to(first.0 as f32, first.1 as f32);
    } else {
        builder.move_to(first.0 as f32, first.1 as f32);
    }
    let identity = |x: f64, y: f64| (x as f32, y as f32);
    // `arc_segments` turns counter-clockwise with y down: negate.
    super::gdi::arc_segments(&mut builder, centre, radii, -start_parameter, -(end_parameter - start_parameter), &identity);
    if pie {
        builder.close();
    }
    builder.finish()
}

/// A cardinal spline through `points` (`tension` as GDI+ takes it), as
/// Béziers; an open one from `offset` through `segments` segments.
fn curve(points: &[(f32, f32)], tension: f64, closed: bool, offset: usize, segments: usize) -> Option<Path> {
    let count = points.len();
    if count < 2 {
        return None;
    }
    let point = |index: isize| -> (f64, f64) {
        let index = if closed { index.rem_euclid(count as isize) as usize } else { index.clamp(0, count as isize - 1) as usize };
        (points[index].0 as f64, points[index].1 as f64)
    };
    let factor = tension / 3.0;
    let (first, last) = if closed { (0, count) } else { (offset.min(count - 1), (offset + segments).min(count - 1)) };
    let mut builder = PathBuilder::new();
    let start = point(first as isize);
    builder.move_to(start.0 as f32, start.1 as f32);
    for index in first..last {
        let index = index as isize;
        let (previous, current, next, after) = (point(index - 1), point(index), point(index + 1), point(index + 2));
        let control_one = (current.0 + (next.0 - previous.0) * factor, current.1 + (next.1 - previous.1) * factor);
        let control_two = (next.0 - (after.0 - current.0) * factor, next.1 - (after.1 - current.1) * factor);
        builder.cubic_to(control_one.0 as f32, control_one.1 as f32, control_two.0 as f32, control_two.1 as f32, next.0 as f32, next.1 as f32);
    }
    if closed {
        builder.close();
    }
    builder.finish()
}

/// GDI+'s hatch patterns as 8×8 tiles: the six line patterns as GDI has
/// them, the percentage ones as ordered dots of that coverage, the rest
/// half covered.
fn hatch_tile(style: u32, fore: [u8; 4], back: [u8; 4]) -> Pixmap {
    const LINES: [[u8; 8]; 6] = [
        [0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        [0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80],
        [0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01],
        [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80],
        [0xff, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80],
        [0x81, 0x42, 0x24, 0x18, 0x18, 0x24, 0x42, 0x81],
    ];
    const BAYER: [[u8; 8]; 8] = [
        [0, 32, 8, 40, 2, 34, 10, 42],
        [48, 16, 56, 24, 50, 18, 58, 26],
        [12, 44, 4, 36, 14, 46, 6, 38],
        [60, 28, 52, 20, 62, 30, 54, 22],
        [3, 35, 11, 43, 1, 33, 9, 41],
        [51, 19, 59, 27, 49, 17, 57, 25],
        [15, 47, 7, 39, 13, 45, 5, 37],
        [63, 31, 55, 23, 61, 29, 53, 21],
    ];
    const PERCENTS: [u8; 12] = [5, 10, 20, 25, 30, 40, 50, 60, 70, 75, 80, 90];
    let set = |x: usize, y: usize| -> bool {
        match style {
            0..=5 => LINES[style as usize][y] & (0x80 >> x) != 0,
            6..=17 => (BAYER[y][x] as u32) * 100 < PERCENTS[style as usize - 6] as u32 * 64,
            _ => (x + y).is_multiple_of(2),
        }
    };
    rgba_pixmap(8, 8, |x, y| if set(x as usize, y as usize) { fore } else { back }).expect("an 8×8 tile")
}
