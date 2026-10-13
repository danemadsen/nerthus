//! A plain metafile's GDI records, played as GDI draws them: on whole
//! device pixels without anti-aliasing (text aside), a pixel lit when its
//! centre is inside a shape, rectangles and ellipses leaving out their
//! right and bottom edges. The device context keeps the mapping mode,
//! window and viewport, world transform, selected objects, colours and
//! modes, clipping and the path being built; `SaveDC`/`RestoreDC` keep
//! copies of it.

use std::rc::Rc;

use resvg::tiny_skia::{Color, FillRule, LineCap, LineJoin, Mask, Paint, Path, PathBuilder, Pattern, Pixmap, Shader, SpreadMode, FilterQuality, Stroke, StrokeDash, Transform};

use super::dib::{self, Dib};
use super::text::{self, LogFont};
use super::{Affine, Bytes, Header, Record};

const EMR_POLYBEZIER: u32 = 2;
const EMR_POLYGON: u32 = 3;
const EMR_POLYLINE: u32 = 4;
const EMR_POLYBEZIERTO: u32 = 5;
const EMR_POLYLINETO: u32 = 6;
const EMR_POLYPOLYLINE: u32 = 7;
const EMR_POLYPOLYGON: u32 = 8;
const EMR_SETWINDOWEXTEX: u32 = 9;
const EMR_SETWINDOWORGEX: u32 = 10;
const EMR_SETVIEWPORTEXTEX: u32 = 11;
const EMR_SETVIEWPORTORGEX: u32 = 12;
const EMR_SETBRUSHORGEX: u32 = 13;
const EMR_SETPIXELV: u32 = 15;
const EMR_SETMAPMODE: u32 = 17;
const EMR_SETBKMODE: u32 = 18;
const EMR_SETPOLYFILLMODE: u32 = 19;
const EMR_SETROP2: u32 = 20;
const EMR_SETSTRETCHBLTMODE: u32 = 21;
const EMR_SETTEXTALIGN: u32 = 22;
const EMR_SETTEXTCOLOR: u32 = 24;
const EMR_SETBKCOLOR: u32 = 25;
const EMR_OFFSETCLIPRGN: u32 = 26;
const EMR_MOVETOEX: u32 = 27;
const EMR_SETMETARGN: u32 = 28;
const EMR_EXCLUDECLIPRECT: u32 = 29;
const EMR_INTERSECTCLIPRECT: u32 = 30;
const EMR_SCALEVIEWPORTEXTEX: u32 = 31;
const EMR_SCALEWINDOWEXTEX: u32 = 32;
const EMR_SAVEDC: u32 = 33;
const EMR_RESTOREDC: u32 = 34;
const EMR_SETWORLDTRANSFORM: u32 = 35;
const EMR_MODIFYWORLDTRANSFORM: u32 = 36;
const EMR_SELECTOBJECT: u32 = 37;
const EMR_CREATEPEN: u32 = 38;
const EMR_CREATEBRUSHINDIRECT: u32 = 39;
const EMR_DELETEOBJECT: u32 = 40;
const EMR_ANGLEARC: u32 = 41;
const EMR_ELLIPSE: u32 = 42;
const EMR_RECTANGLE: u32 = 43;
const EMR_ROUNDRECT: u32 = 44;
const EMR_ARC: u32 = 45;
const EMR_CHORD: u32 = 46;
const EMR_PIE: u32 = 47;
const EMR_SELECTPALETTE: u32 = 48;
const EMR_CREATEPALETTE: u32 = 49;
const EMR_SETPALETTEENTRIES: u32 = 50;
const EMR_EXTFLOODFILL: u32 = 53;
const EMR_LINETO: u32 = 54;
const EMR_ARCTO: u32 = 55;
const EMR_POLYDRAW: u32 = 56;
const EMR_SETARCDIRECTION: u32 = 57;
const EMR_SETMITERLIMIT: u32 = 58;
const EMR_BEGINPATH: u32 = 59;
const EMR_ENDPATH: u32 = 60;
const EMR_CLOSEFIGURE: u32 = 61;
const EMR_FILLPATH: u32 = 62;
const EMR_STROKEANDFILLPATH: u32 = 63;
const EMR_STROKEPATH: u32 = 64;
const EMR_FLATTENPATH: u32 = 65;
const EMR_WIDENPATH: u32 = 66;
const EMR_SELECTCLIPPATH: u32 = 67;
const EMR_ABORTPATH: u32 = 68;
const EMR_FILLRGN: u32 = 71;
const EMR_FRAMERGN: u32 = 72;
const EMR_INVERTRGN: u32 = 73;
const EMR_PAINTRGN: u32 = 74;
const EMR_EXTSELECTCLIPRGN: u32 = 75;
const EMR_BITBLT: u32 = 76;
const EMR_STRETCHBLT: u32 = 77;
const EMR_MASKBLT: u32 = 78;
const EMR_PLGBLT: u32 = 79;
const EMR_SETDIBITSTODEVICE: u32 = 80;
const EMR_STRETCHDIBITS: u32 = 81;
const EMR_EXTCREATEFONTINDIRECTW: u32 = 82;
const EMR_EXTTEXTOUTA: u32 = 83;
const EMR_EXTTEXTOUTW: u32 = 84;
const EMR_POLYBEZIER16: u32 = 85;
const EMR_POLYGON16: u32 = 86;
const EMR_POLYLINE16: u32 = 87;
const EMR_POLYBEZIERTO16: u32 = 88;
const EMR_POLYLINETO16: u32 = 89;
const EMR_POLYPOLYLINE16: u32 = 90;
const EMR_POLYPOLYGON16: u32 = 91;
const EMR_POLYDRAW16: u32 = 92;
const EMR_CREATEMONOBRUSH: u32 = 93;
const EMR_CREATEDIBPATTERNBRUSHPT: u32 = 94;
const EMR_EXTCREATEPEN: u32 = 95;
const EMR_POLYTEXTOUTA: u32 = 96;
const EMR_POLYTEXTOUTW: u32 = 97;
const EMR_SMALLTEXTOUT: u32 = 108;
const EMR_ALPHABLEND: u32 = 114;
const EMR_TRANSPARENTBLT: u32 = 116;
const EMR_GRADIENTFILL: u32 = 118;

/// Mapping modes.
const MM_TEXT: u32 = 1;
const MM_LOMETRIC: u32 = 2;
const MM_HIMETRIC: u32 = 3;
const MM_LOENGLISH: u32 = 4;
const MM_HIENGLISH: u32 = 5;
const MM_TWIPS: u32 = 6;
const MM_ISOTROPIC: u32 = 7;
const MM_ANISOTROPIC: u32 = 8;

/// Pen styles, end caps, joins and type.
const PS_STYLE_MASK: u32 = 0x0F;
const PS_DASH: u32 = 1;
const PS_DOT: u32 = 2;
const PS_DASHDOT: u32 = 3;
const PS_DASHDOTDOT: u32 = 4;
const PS_NULL: u32 = 5;
const PS_INSIDEFRAME: u32 = 6;
const PS_USERSTYLE: u32 = 7;
const PS_ALTERNATE: u32 = 8;
const PS_ENDCAP_MASK: u32 = 0x0F00;
const PS_ENDCAP_SQUARE: u32 = 0x0100;
const PS_ENDCAP_FLAT: u32 = 0x0200;
const PS_JOIN_MASK: u32 = 0xF000;
const PS_JOIN_BEVEL: u32 = 0x1000;
const PS_JOIN_MITER: u32 = 0x2000;
const PS_GEOMETRIC: u32 = 0x0001_0000;

/// Brush styles.
const BS_SOLID: u32 = 0;
const BS_NULL: u32 = 1;
const BS_HATCHED: u32 = 2;

/// `SetBkMode`, `SetPolyFillMode`, `SetArcDirection`, `SetROP2`.
const OPAQUE: u32 = 2;
const WINDING: u32 = 2;
const AD_CLOCKWISE: u32 = 2;
const R2_BLACK: u32 = 1;
const R2_NOT: u32 = 6;
const R2_NOP: u32 = 11;
const R2_WHITE: u32 = 16;
/// `SetStretchBltMode(HALFTONE)`.
const HALFTONE: u32 = 4;

/// Region combinations (`RGN_AND` … `RGN_COPY`).
const RGN_AND: u32 = 1;
const RGN_OR: u32 = 2;
const RGN_XOR: u32 = 3;
const RGN_DIFF: u32 = 4;
const RGN_COPY: u32 = 5;

/// `ModifyWorldTransform` modes.
const MWT_IDENTITY: u32 = 1;
const MWT_LEFTMULTIPLY: u32 = 2;
const MWT_RIGHTMULTIPLY: u32 = 3;
const MWT_SET: u32 = 4;

/// `PolyDraw` point types.
const PT_CLOSEFIGURE: u8 = 1;
const PT_LINETO: u8 = 2;
const PT_BEZIERTO: u8 = 4;
const PT_MOVETO: u8 = 6;

/// `ExtTextOut` options.
const ETO_OPAQUE: u32 = 0x2;
const ETO_CLIPPED: u32 = 0x4;
const ETO_GLYPH_INDEX: u32 = 0x10;
const ETO_NO_RECT: u32 = 0x100;
const ETO_SMALL_CHARS: u32 = 0x200;
const ETO_PDY: u32 = 0x2000;

/// The GDI hatch patterns, 8×8, the high bit leftmost (`HS_HORIZONTAL`
/// … `HS_DIAGCROSS`).
const HATCHES: [[u8; 8]; 6] = [
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff],
    [0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08],
    [0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01],
    [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80],
    [0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0xff],
    [0x81, 0x42, 0x24, 0x18, 0x18, 0x24, 0x42, 0x81],
];

/// A brush.
#[derive(Clone)]
pub(super) enum Brush {
    Null,
    Solid([u8; 3]),
    Hatch { style: u32, colour: [u8; 3] },
    /// A bitmap tiled; a monochrome one takes the text and background
    /// colours.
    Pattern { bitmap: Rc<Dib>, monochrome: bool },
}

/// A pen: `CreatePen`'s or `ExtCreatePen`'s.
#[derive(Clone)]
pub(super) struct Pen {
    style: u32,
    /// In logical units; 0 is one pixel.
    width: f64,
    colour: [u8; 3],
    extended: bool,
    /// `PS_USERSTYLE` dashes and gaps.
    dashes: Vec<f64>,
}

impl Pen {
    fn solid(colour: [u8; 3]) -> Pen {
        Pen { style: 0, width: 0.0, colour, extended: false, dashes: Vec::new() }
    }

    fn is_null(&self) -> bool {
        self.style & PS_STYLE_MASK == PS_NULL
    }
}

#[derive(Clone)]
enum Object {
    Pen(Pen),
    Brush(Brush),
    Font(LogFont),
    Palette(Vec<[u8; 4]>),
}

/// The device context's state: what `SaveDC` keeps.
#[derive(Clone)]
struct State {
    map_mode: u32,
    window_origin: (f64, f64),
    window_extent: (f64, f64),
    viewport_origin: (f64, f64),
    viewport_extent: (f64, f64),
    world: Affine,
    pen: Pen,
    brush: Brush,
    font: LogFont,
    palette: Rc<Vec<[u8; 4]>>,
    text_colour: [u8; 3],
    background_colour: [u8; 3],
    background_mode: u32,
    fill_mode: u32,
    raster_operation: u32,
    stretch_mode: u32,
    text_align: u32,
    arc_direction: u32,
    miter_limit: f32,
    /// The current position, in logical units.
    position: (f64, f64),
    brush_origin: (f64, f64),
    /// The clipping region on the picture; none clips nothing.
    clip: Option<Rc<Mask>>,
}

impl State {
    fn new() -> State {
        State {
            map_mode: MM_TEXT,
            window_origin: (0.0, 0.0),
            window_extent: (1.0, 1.0),
            viewport_origin: (0.0, 0.0),
            viewport_extent: (1.0, 1.0),
            world: Affine::IDENTITY,
            pen: Pen::solid([0, 0, 0]),
            brush: Brush::Solid([255, 255, 255]),
            font: LogFont::default(),
            palette: Rc::new(Vec::new()),
            text_colour: [0, 0, 0],
            background_colour: [255, 255, 255],
            background_mode: OPAQUE,
            fill_mode: 1,
            raster_operation: 13,
            stretch_mode: 1,
            text_align: 0,
            arc_direction: 1,
            miter_limit: 10.0,
            position: (0.0, 0.0),
            brush_origin: (0.0, 0.0),
            clip: None,
        }
    }
}

/// A path being built (`BeginPath` … `EndPath`) or finished, in picture
/// coordinates.
#[derive(Default)]
struct PathState {
    building: Option<PathBuilder>,
    figure_open: bool,
    finished: Option<Path>,
}

pub(super) struct Player {
    device: (f64, f64),
    millimeters: (f64, f64),
    /// Reference device pixels to the picture's.
    playback: Affine,
    width: u32,
    height: u32,
    state: State,
    saved: Vec<State>,
    objects: Vec<Option<Object>>,
    path: PathState,
    /// `SetMetaRgn`'s region, which later clipping stays inside.
    meta: Option<Rc<Mask>>,
}

impl Player {
    pub(super) fn new(header: &Header, playback: Affine, width: u32, height: u32) -> Player {
        let device = (header.device.0 as f64, header.device.1 as f64);
        let millimeters = (header.millimeters.0 as f64, header.millimeters.1 as f64);
        let objects = vec![None; header.handles.clamp(1, 65536)];
        Player { device, millimeters, playback, width, height, state: State::new(), saved: Vec::new(), objects, path: PathState::default(), meta: None }
    }

    /// Logical units to the picture's pixels.
    fn transform(&self) -> Affine {
        let state = &self.state;
        let scale_x = state.viewport_extent.0 / state.window_extent.0;
        let scale_y = state.viewport_extent.1 / state.window_extent.1;
        let page = Affine {
            a: scale_x,
            b: 0.0,
            c: 0.0,
            d: scale_y,
            e: state.viewport_origin.0 - state.window_origin.0 * scale_x,
            f: state.viewport_origin.1 - state.window_origin.1 * scale_y,
        };
        state.world.then(&page).then(&self.playback)
    }

    /// A logical point on the picture, rounded to whole device units as
    /// GDI keeps them (tiny-skia's aliased rasterizer then lights the
    /// pixels GDI does).
    fn point(&self, transform: &Affine, x: f64, y: f64) -> (f32, f32) {
        let (x, y) = transform.apply(x, y);
        ((x + 0.5).floor() as f32, (y + 0.5).floor() as f32)
    }

    pub(super) fn play(&mut self, record: &Record, pixmap: &mut Pixmap) {
        let data = record.data;
        match record.kind {
            EMR_POLYBEZIER | EMR_POLYGON | EMR_POLYLINE | EMR_POLYBEZIERTO | EMR_POLYLINETO => {
                let points = read_points(data, 24, false);
                self.poly(record.kind, &points, pixmap);
            }
            EMR_POLYBEZIER16 | EMR_POLYGON16 | EMR_POLYLINE16 | EMR_POLYBEZIERTO16 | EMR_POLYLINETO16 => {
                let points = read_points(data, 24, true);
                let kind = record.kind - EMR_POLYBEZIER16 + EMR_POLYBEZIER;
                self.poly(kind, &points, pixmap);
            }
            EMR_POLYPOLYLINE | EMR_POLYPOLYGON | EMR_POLYPOLYLINE16 | EMR_POLYPOLYGON16 => {
                let small = record.kind >= EMR_POLYPOLYLINE16;
                let polygon = record.kind == EMR_POLYPOLYGON || record.kind == EMR_POLYPOLYGON16;
                self.poly_poly(data, small, polygon, pixmap);
            }
            EMR_POLYDRAW | EMR_POLYDRAW16 => self.poly_draw(data, record.kind == EMR_POLYDRAW16, pixmap),
            EMR_SETWINDOWEXTEX => self.set_extent(false, data.i32_at(8) as f64, data.i32_at(12) as f64),
            EMR_SETVIEWPORTEXTEX => self.set_extent(true, data.i32_at(8) as f64, data.i32_at(12) as f64),
            EMR_SETWINDOWORGEX => self.state.window_origin = (data.i32_at(8) as f64, data.i32_at(12) as f64),
            EMR_SETVIEWPORTORGEX => self.state.viewport_origin = (data.i32_at(8) as f64, data.i32_at(12) as f64),
            EMR_SETBRUSHORGEX => self.state.brush_origin = (data.i32_at(8) as f64, data.i32_at(12) as f64),
            EMR_SETPIXELV => {
                let transform = self.transform();
                let (x, y) = self.point(&transform, data.i32_at(8) as f64, data.i32_at(12) as f64);
                let colour = self.colour_ref(data.u32_at(16));
                self.set_pixel(pixmap, x.floor() as i64, y.floor() as i64, colour);
            }
            EMR_SETMAPMODE => self.set_map_mode(data.u32_at(8)),
            EMR_SETBKMODE => self.state.background_mode = data.u32_at(8),
            EMR_SETPOLYFILLMODE => self.state.fill_mode = data.u32_at(8),
            EMR_SETROP2 => self.state.raster_operation = data.u32_at(8),
            EMR_SETSTRETCHBLTMODE => self.state.stretch_mode = data.u32_at(8),
            EMR_SETTEXTALIGN => self.state.text_align = data.u32_at(8),
            EMR_SETTEXTCOLOR => self.state.text_colour = self.colour_ref(data.u32_at(8)),
            EMR_SETBKCOLOR => self.state.background_colour = self.colour_ref(data.u32_at(8)),
            EMR_OFFSETCLIPRGN => {
                let (x, y) = self.transform().apply_vector(data.i32_at(8) as f64, data.i32_at(12) as f64);
                self.offset_clip(x.round() as i64, y.round() as i64);
            }
            EMR_MOVETOEX => {
                self.state.position = (data.i32_at(8) as f64, data.i32_at(12) as f64);
                let transform = self.transform();
                let (x, y) = self.point(&transform, self.state.position.0, self.state.position.1);
                if let Some(builder) = self.path.building.as_mut() {
                    builder.move_to(x, y);
                    self.path.figure_open = true;
                }
            }
            EMR_SETMETARGN => {
                self.meta = self.state.clip.clone();
            }
            EMR_EXCLUDECLIPRECT | EMR_INTERSECTCLIPRECT => {
                let rect = read_rect(data, 8);
                let path = self.rect_path(rect);
                let mode = if record.kind == EMR_EXCLUDECLIPRECT { RGN_DIFF } else { RGN_AND };
                if let Some(path) = path {
                    let region = self.mask_of(&path, FillRule::Winding);
                    self.combine_clip(Some(region), mode);
                }
            }
            EMR_SCALEVIEWPORTEXTEX | EMR_SCALEWINDOWEXTEX => {
                let viewport = record.kind == EMR_SCALEVIEWPORTEXTEX;
                let (x_numerator, x_denominator) = (data.i32_at(8) as f64, data.i32_at(12) as f64);
                let (y_numerator, y_denominator) = (data.i32_at(16) as f64, data.i32_at(20) as f64);
                if x_denominator != 0.0 && y_denominator != 0.0 {
                    let current = if viewport { self.state.viewport_extent } else { self.state.window_extent };
                    let x = (current.0 * x_numerator / x_denominator).round();
                    let y = (current.1 * y_numerator / y_denominator).round();
                    self.set_extent(viewport, x, y);
                }
            }
            EMR_SAVEDC => self.saved.push(self.state.clone()),
            EMR_RESTOREDC => {
                let which = data.i32_at(8);
                let depth = self.saved.len() as i64;
                let target = if which < 0 { depth + which as i64 } else { which as i64 - 1 };
                if (0..depth).contains(&target) {
                    self.saved.truncate(target as usize + 1);
                    if let Some(state) = self.saved.pop() {
                        self.state = state;
                    }
                }
            }
            EMR_SETWORLDTRANSFORM => self.state.world = read_xform(data, 8),
            EMR_MODIFYWORLDTRANSFORM => {
                let xform = read_xform(data, 8);
                match data.u32_at(32) {
                    MWT_IDENTITY => self.state.world = Affine::IDENTITY,
                    MWT_LEFTMULTIPLY => self.state.world = xform.then(&self.state.world),
                    MWT_RIGHTMULTIPLY => self.state.world = self.state.world.then(&xform),
                    MWT_SET => self.state.world = xform,
                    _ => {}
                }
            }
            EMR_SELECTOBJECT => self.select_object(data.u32_at(8)),
            EMR_CREATEPEN => {
                let style = data.u32_at(12);
                let width = data.i32_at(16) as f64;
                let colour = self.colour_ref(data.u32_at(24));
                let pen = Pen { style, width, colour, extended: false, dashes: Vec::new() };
                self.store(data.u32_at(8), Object::Pen(pen));
            }
            EMR_EXTCREATEPEN => {
                let style = data.u32_at(28);
                let width = data.u32_at(32) as f64;
                let brush_style = data.u32_at(36);
                let colour = self.colour_ref(data.u32_at(40));
                let count = (data.u32_at(48) as usize).min(data.len().saturating_sub(52) / 4);
                let dashes = (0..count).map(|index| data.u32_at(52 + 4 * index) as f64).collect();
                let style = if brush_style == BS_NULL { PS_NULL } else { style };
                let pen = Pen { style, width, colour, extended: true, dashes };
                self.store(data.u32_at(8), Object::Pen(pen));
            }
            EMR_CREATEBRUSHINDIRECT => {
                let colour = self.colour_ref(data.u32_at(16));
                let brush = match data.u32_at(12) {
                    BS_SOLID => Brush::Solid(colour),
                    BS_HATCHED => Brush::Hatch { style: data.u32_at(20), colour },
                    BS_NULL => Brush::Null,
                    _ => Brush::Solid(colour),
                };
                self.store(data.u32_at(8), Object::Brush(brush));
            }
            EMR_CREATEMONOBRUSH | EMR_CREATEDIBPATTERNBRUSHPT => {
                let usage = data.u32_at(12);
                let bitmap = self.bitmap(data, usage, 16, 24);
                let brush = match bitmap {
                    Some(bitmap) => Brush::Pattern { bitmap: Rc::new(bitmap), monochrome: record.kind == EMR_CREATEMONOBRUSH },
                    None => Brush::Solid([0, 0, 0]),
                };
                self.store(data.u32_at(8), Object::Brush(brush));
            }
            EMR_EXTCREATEFONTINDIRECTW => {
                let font = LogFont::read(&data[12.min(data.len())..]);
                self.store(data.u32_at(8), Object::Font(font));
            }
            EMR_CREATEPALETTE => {
                let count = (data.u16_at(14) as usize).min(data.len().saturating_sub(16) / 4);
                let entries = (0..count).map(|index| {
                    let at = 16 + 4 * index;
                    [data.u8_at(at), data.u8_at(at + 1), data.u8_at(at + 2), 255]
                });
                self.store(data.u32_at(8), Object::Palette(entries.collect()));
            }
            EMR_SETPALETTEENTRIES => {
                let index = data.u32_at(8) as usize;
                let start = data.u32_at(12) as usize;
                let count = (data.u32_at(16) as usize).min(data.len().saturating_sub(20) / 4);
                if let Some(Some(Object::Palette(entries))) = self.objects.get_mut(index) {
                    for step in 0..count {
                        let at = 20 + 4 * step;
                        let entry = [data.u8_at(at), data.u8_at(at + 1), data.u8_at(at + 2), 255];
                        if let Some(slot) = entries.get_mut(start + step) {
                            *slot = entry;
                        }
                    }
                }
            }
            EMR_SELECTPALETTE => {
                let handle = data.u32_at(8);
                if let Some(Some(Object::Palette(entries))) = self.objects.get(handle as usize) {
                    self.state.palette = Rc::new(entries.clone());
                }
            }
            EMR_DELETEOBJECT => {
                if let Some(slot) = self.objects.get_mut(data.u32_at(8) as usize) {
                    *slot = None;
                }
            }
            EMR_ANGLEARC => {
                let (cx, cy) = (data.i32_at(8) as f64, data.i32_at(12) as f64);
                let radius = data.u32_at(16) as f64;
                let start = (data.f32_at(20) as f64).to_radians();
                let sweep = (data.f32_at(24) as f64).to_radians();
                self.angle_arc(cx, cy, radius, start, sweep, pixmap);
            }
            EMR_ELLIPSE | EMR_RECTANGLE => {
                let rect = read_rect(data, 8);
                self.box_shape(record.kind, rect, (0.0, 0.0), None, pixmap);
            }
            EMR_ROUNDRECT => {
                let rect = read_rect(data, 8);
                let corner = (data.i32_at(24) as f64, data.i32_at(28) as f64);
                self.box_shape(record.kind, rect, corner, None, pixmap);
            }
            EMR_ARC | EMR_CHORD | EMR_PIE | EMR_ARCTO => {
                let rect = read_rect(data, 8);
                let start = (data.i32_at(24) as f64, data.i32_at(28) as f64);
                let end = (data.i32_at(32) as f64, data.i32_at(36) as f64);
                self.box_shape(record.kind, rect, (0.0, 0.0), Some((start, end)), pixmap);
            }
            EMR_EXTFLOODFILL => {
                let transform = self.transform();
                let (x, y) = self.point(&transform, data.i32_at(8) as f64, data.i32_at(12) as f64);
                let colour = self.colour_ref(data.u32_at(16));
                self.flood_fill(pixmap, x.floor() as i64, y.floor() as i64, colour, data.u32_at(20) == 1);
            }
            EMR_LINETO => {
                let to = (data.i32_at(8) as f64, data.i32_at(12) as f64);
                let from = self.state.position;
                self.state.position = to;
                self.lines(&[from, to], false, true, pixmap);
            }
            EMR_SETARCDIRECTION => self.state.arc_direction = data.u32_at(8),
            EMR_SETMITERLIMIT => {
                // A float, though some writers store a whole number.
                let value = data.f32_at(8);
                self.state.miter_limit = if value >= 1.0 && value.is_finite() { value } else { data.u32_at(8).max(1) as f32 };
            }
            EMR_BEGINPATH => {
                self.path = PathState { building: Some(PathBuilder::new()), figure_open: false, finished: None };
            }
            EMR_ENDPATH => {
                if let Some(builder) = self.path.building.take() {
                    self.path.finished = builder.finish();
                }
            }
            EMR_CLOSEFIGURE => {
                if let Some(builder) = self.path.building.as_mut() {
                    builder.close();
                    self.path.figure_open = false;
                }
            }
            EMR_ABORTPATH => self.path = PathState::default(),
            EMR_FILLPATH | EMR_STROKEANDFILLPATH | EMR_STROKEPATH => {
                if let Some(path) = self.path.finished.take() {
                    let fill = record.kind != EMR_STROKEPATH;
                    let stroke = record.kind != EMR_FILLPATH;
                    if fill {
                        let closed = close_all(&path);
                        self.fill(&closed, pixmap);
                    }
                    if stroke {
                        self.stroke(&path, pixmap);
                    }
                }
            }
            EMR_FLATTENPATH => {}
            EMR_WIDENPATH => {
                if let Some(path) = self.path.finished.take() {
                    let stroke = self.pen_stroke();
                    self.path.finished = path.stroke(&stroke, 1.0).or(Some(path));
                }
            }
            EMR_SELECTCLIPPATH => {
                if let Some(path) = self.path.finished.take() {
                    let closed = close_all(&path);
                    let region = self.mask_of(&closed, self.fill_rule());
                    self.combine_clip(Some(region), data.u32_at(8));
                }
            }
            EMR_FILLRGN | EMR_PAINTRGN | EMR_FRAMERGN | EMR_INVERTRGN => self.region_paint(record.kind, data, pixmap),
            EMR_EXTSELECTCLIPRGN => {
                let size = data.u32_at(8) as usize;
                let mode = data.u32_at(12);
                if size == 0 {
                    // No region: only `RGN_COPY` means anything, a reset.
                    if mode == RGN_COPY {
                        self.state.clip = self.meta.clone();
                    }
                } else {
                    let rects = read_region(data, 16, size);
                    let path = rects_path(&rects, &self.playback);
                    let region = match path {
                        Some(path) => self.mask_of(&path, FillRule::Winding),
                        None => Mask::new(self.width, self.height).expect("the picture's size was checked"),
                    };
                    self.combine_clip(Some(region), mode);
                }
            }
            EMR_BITBLT | EMR_STRETCHBLT | EMR_ALPHABLEND | EMR_TRANSPARENTBLT => self.stretch_blt(record.kind, data, pixmap),
            EMR_MASKBLT => self.mask_blt(data, pixmap),
            EMR_PLGBLT => self.plg_blt(data, pixmap),
            EMR_SETDIBITSTODEVICE => self.set_dibits(data, pixmap),
            EMR_STRETCHDIBITS => self.stretch_dibits(data, pixmap),
            EMR_EXTTEXTOUTA | EMR_EXTTEXTOUTW => {
                let mode = data.u32_at(24);
                self.text_out(data, 36, mode, record.kind == EMR_EXTTEXTOUTW, pixmap);
            }
            EMR_POLYTEXTOUTA | EMR_POLYTEXTOUTW => {
                let mode = data.u32_at(24);
                let count = data.u32_at(36) as usize;
                for index in 0..count.min(data.len() / 40) {
                    self.text_out(data, 40 + 40 * index, mode, record.kind == EMR_POLYTEXTOUTW, pixmap);
                }
            }
            EMR_SMALLTEXTOUT => self.small_text_out(data, pixmap),
            EMR_GRADIENTFILL => self.gradient_fill(data, pixmap),
            _ => {}
        }
    }

    // Mapping.

    fn set_map_mode(&mut self, mode: u32) {
        let (size_x, size_y) = self.millimeters;
        let (resolution_x, resolution_y) = self.device;
        let metric = |units_per_millimetre: f64| ((size_x * units_per_millimetre).round(), (size_y * units_per_millimetre).round());
        let window = match mode {
            MM_TEXT => {
                self.state.window_extent = (1.0, 1.0);
                self.state.viewport_extent = (1.0, 1.0);
                None
            }
            MM_LOMETRIC | MM_ISOTROPIC => Some(metric(10.0)),
            MM_HIMETRIC => Some(metric(100.0)),
            MM_LOENGLISH => Some(metric(1000.0 / 254.0)),
            MM_HIENGLISH => Some(metric(10000.0 / 254.0)),
            MM_TWIPS => Some(metric(14400.0 / 254.0)),
            MM_ANISOTROPIC => None,
            _ => return,
        };
        if let Some(extent) = window {
            self.state.window_extent = extent;
            self.state.viewport_extent = (resolution_x, -resolution_y);
        }
        self.state.map_mode = mode;
    }

    /// `SetWindowExtEx` and `SetViewportExtEx`: only the scalable mapping
    /// modes take them; the isotropic one keeps its units square.
    fn set_extent(&mut self, viewport: bool, x: f64, y: f64) {
        let mode = self.state.map_mode;
        if (mode != MM_ISOTROPIC && mode != MM_ANISOTROPIC) || x == 0.0 || y == 0.0 {
            return;
        }
        if viewport {
            self.state.viewport_extent = (x, y);
        } else {
            self.state.window_extent = (x, y);
        }
        if mode == MM_ISOTROPIC {
            self.fix_isotropic();
        }
    }

    fn fix_isotropic(&mut self) {
        let state = &mut self.state;
        let x_size = (state.viewport_extent.0 * self.millimeters.0 / (self.device.0 * state.window_extent.0)).abs();
        let y_size = (state.viewport_extent.1 * self.millimeters.1 / (self.device.1 * state.window_extent.1)).abs();
        if x_size > y_size {
            let minimum = if state.viewport_extent.0 >= 0.0 { 1.0 } else { -1.0 };
            let x = (state.viewport_extent.0 * y_size / x_size).round();
            state.viewport_extent.0 = if x == 0.0 { minimum } else { x };
        } else if y_size > 0.0 {
            let minimum = if state.viewport_extent.1 >= 0.0 { 1.0 } else { -1.0 };
            let y = (state.viewport_extent.1 * x_size / y_size).round();
            state.viewport_extent.1 = if y == 0.0 { minimum } else { y };
        }
    }

    // Objects.

    fn store(&mut self, handle: u32, object: Object) {
        let index = handle as usize;
        if index >= self.objects.len() && index < 65536 {
            self.objects.resize(index + 1, None);
        }
        if let Some(slot) = self.objects.get_mut(index) {
            *slot = Some(object);
        }
    }

    fn select_object(&mut self, handle: u32) {
        if handle & 0x8000_0000 != 0 {
            let grey = |value: u8| Brush::Solid([value, value, value]);
            match handle & 0x7FFF_FFFF {
                0 => self.state.brush = grey(255),
                1 => self.state.brush = grey(192),
                2 => self.state.brush = grey(128),
                3 => self.state.brush = grey(64),
                4 => self.state.brush = grey(0),
                5 => self.state.brush = Brush::Null,
                6 => self.state.pen = Pen::solid([255, 255, 255]),
                7 => self.state.pen = Pen::solid([0, 0, 0]),
                8 => self.state.pen = Pen { style: PS_NULL, ..Pen::solid([0, 0, 0]) },
                10..=14 | 16 | 17 => self.state.font = LogFont::default(),
                15 => self.state.palette = Rc::new(Vec::new()),
                18 => self.state.brush = grey(255),
                19 => self.state.pen = Pen::solid([0, 0, 0]),
                _ => {}
            }
            return;
        }
        match self.objects.get(handle as usize) {
            Some(Some(Object::Pen(pen))) => self.state.pen = pen.clone(),
            Some(Some(Object::Brush(brush))) => self.state.brush = brush.clone(),
            Some(Some(Object::Font(font))) => self.state.font = font.clone(),
            Some(Some(Object::Palette(entries))) => self.state.palette = Rc::new(entries.clone()),
            _ => {}
        }
    }

    /// A `COLORREF`, palette indices looked up.
    fn colour_ref(&self, value: u32) -> [u8; 3] {
        if value >> 24 == 1 {
            let index = (value & 0xFFFF) as usize;
            let entry = self.state.palette.get(index).copied().unwrap_or([0, 0, 0, 255]);
            return [entry[0], entry[1], entry[2]];
        }
        [value as u8, (value >> 8) as u8, (value >> 16) as u8]
    }

    /// A record's bitmap: its `BITMAPINFO` and bits at the offsets given.
    fn bitmap(&self, data: &[u8], usage: u32, info_at: usize, bits_at: usize) -> Option<Dib> {
        let info_offset = data.u32_at(info_at) as usize;
        let info_size = data.u32_at(info_at + 4) as usize;
        let bits_offset = data.u32_at(bits_at) as usize;
        let bits_size = data.u32_at(bits_at + 4) as usize;
        if info_size == 0 {
            return None;
        }
        let info = data.get(info_offset..info_offset.checked_add(info_size)?)?;
        let bits = data.get(bits_offset..bits_offset.checked_add(bits_size)?)?;
        dib::decode(info, bits, usage, &self.state.palette)
    }

    // Shapes.

    fn fill_rule(&self) -> FillRule {
        if self.state.fill_mode == WINDING { FillRule::Winding } else { FillRule::EvenOdd }
    }

    /// Polylines, polygons and Béziers, `…To` ones from the current
    /// position.
    fn poly(&mut self, kind: u32, points: &[(f64, f64)], pixmap: &mut Pixmap) {
        match kind {
            EMR_POLYLINE => self.lines(points, false, false, pixmap),
            EMR_POLYGON => self.lines(points, true, false, pixmap),
            EMR_POLYLINETO => {
                let mut all = vec![self.state.position];
                all.extend_from_slice(points);
                if let Some(last) = points.last() {
                    self.state.position = *last;
                }
                self.lines(&all, false, true, pixmap);
            }
            EMR_POLYBEZIER | EMR_POLYBEZIERTO => {
                let mut all = Vec::new();
                if kind == EMR_POLYBEZIERTO {
                    all.push(self.state.position);
                    if let Some(last) = points.last() {
                        self.state.position = *last;
                    }
                }
                all.extend_from_slice(points);
                self.beziers(&all, kind == EMR_POLYBEZIERTO, pixmap);
            }
            _ => {}
        }
    }

    fn poly_poly(&mut self, data: &[u8], small: bool, polygon: bool, pixmap: &mut Pixmap) {
        let groups = data.u32_at(24) as usize;
        let total = data.u32_at(28) as usize;
        let point_size = if small { 4 } else { 8 };
        if 32 + 4 * groups + point_size * total > data.len() {
            return;
        }
        let counts: Vec<usize> = (0..groups).map(|index| data.u32_at(32 + 4 * index) as usize).collect();
        let points = read_point_list(data, 32 + 4 * groups, total, small);
        let transform = self.transform();
        let mut builder = PathBuilder::new();
        let mut at = 0;
        for count in counts {
            let Some(group) = points.get(at..at + count) else {
                break;
            };
            at += count;
            for (index, (x, y)) in group.iter().enumerate() {
                let (x, y) = self.point(&transform, *x, *y);
                if index == 0 { builder.move_to(x, y) } else { builder.line_to(x, y) }
            }
            if polygon {
                builder.close();
            }
        }
        let Some(path) = builder.finish() else {
            return;
        };
        if self.add_to_path(&path) {
            return;
        }
        if polygon {
            self.fill(&path, pixmap);
        }
        self.stroke(&path, pixmap);
    }

    /// `PolyDraw`: moves, lines and Béziers by point type.
    fn poly_draw(&mut self, data: &[u8], small: bool, pixmap: &mut Pixmap) {
        let count = data.u32_at(24) as usize;
        let point_size = if small { 4 } else { 8 };
        let types_at = 28 + point_size * count;
        if types_at + count > data.len() {
            return;
        }
        let points = read_point_list(data, 28, count, small);
        let transform = self.transform();
        let mut builder = PathBuilder::new();
        let (start_x, start_y) = self.point(&transform, self.state.position.0, self.state.position.1);
        builder.move_to(start_x, start_y);
        let mut index = 0;
        while index < count {
            let kind = data[types_at + index];
            let (x, y) = self.point(&transform, points[index].0, points[index].1);
            match kind & !PT_CLOSEFIGURE {
                PT_MOVETO => builder.move_to(x, y),
                PT_LINETO => builder.line_to(x, y),
                PT_BEZIERTO if index + 2 < count => {
                    let (x2, y2) = self.point(&transform, points[index + 1].0, points[index + 1].1);
                    let (x3, y3) = self.point(&transform, points[index + 2].0, points[index + 2].1);
                    builder.cubic_to(x, y, x2, y2, x3, y3);
                    index += 2;
                }
                _ => {}
            }
            if data[types_at + index] & PT_CLOSEFIGURE != 0 {
                builder.close();
            }
            index += 1;
        }
        if let Some(last) = points.last() {
            self.state.position = *last;
        }
        if let Some(path) = builder.finish()
            && !self.add_to_path(&path)
        {
            self.stroke(&path, pixmap);
        }
    }

    /// Connected lines; closed ones are filled first.
    fn lines(&mut self, points: &[(f64, f64)], closed: bool, continues: bool, pixmap: &mut Pixmap) {
        if points.len() < 2 && !(closed && !points.is_empty()) {
            return;
        }
        let transform = self.transform();
        let device: Vec<(f32, f32)> = points.iter().map(|(x, y)| self.point(&transform, *x, *y)).collect();
        if let Some(builder) = self.path.building.as_mut() {
            // In a path, `…To` lines carry on the open figure.
            let (first_x, first_y) = device[0];
            if continues && self.path.figure_open {
                builder.line_to(first_x, first_y);
            } else {
                builder.move_to(first_x, first_y);
            }
            for (x, y) in &device[1..] {
                builder.line_to(*x, *y);
            }
            if closed {
                builder.close();
                self.path.figure_open = false;
            } else {
                self.path.figure_open = true;
            }
            return;
        }
        let mut builder = PathBuilder::new();
        builder.move_to(device[0].0, device[0].1);
        for (x, y) in &device[1..] {
            builder.line_to(*x, *y);
        }
        if closed {
            builder.close();
        }
        // A line of no length lights nothing (it stops short of its end).
        let Some(path) = builder.finish() else {
            return;
        };
        if closed {
            self.fill(&path, pixmap);
        }
        self.stroke(&path, pixmap);
    }

    /// Cubic Béziers: a start point, then three points each.
    fn beziers(&mut self, points: &[(f64, f64)], continues: bool, pixmap: &mut Pixmap) {
        if points.len() < 4 {
            return;
        }
        let transform = self.transform();
        let device: Vec<(f32, f32)> = points.iter().map(|(x, y)| self.point(&transform, *x, *y)).collect();
        let segments = (device.len() - 1) / 3;
        let mut builder = PathBuilder::new();
        builder.move_to(device[0].0, device[0].1);
        for segment in 0..segments {
            let [one, two, three] = [device[1 + 3 * segment], device[2 + 3 * segment], device[3 + 3 * segment]];
            builder.cubic_to(one.0, one.1, two.0, two.1, three.0, three.1);
        }
        let Some(path) = builder.finish() else {
            return;
        };
        if let Some(building) = self.path.building.as_mut() {
            append(building, &path, continues && self.path.figure_open);
            self.path.figure_open = true;
            return;
        }
        self.stroke(&path, pixmap);
    }

    /// Adds `path` to the path being built, if there is one.
    fn add_to_path(&mut self, path: &Path) -> bool {
        let Some(building) = self.path.building.as_mut() else {
            return false;
        };
        append(building, path, false);
        self.path.figure_open = false;
        true
    }

    /// `AngleArc`: a line from the current position to the arc's start,
    /// then the arc; the current position moves to its end.
    fn angle_arc(&mut self, cx: f64, cy: f64, radius: f64, start: f64, sweep: f64, pixmap: &mut Pixmap) {
        let transform = self.transform();
        let at = |angle: f64| (cx + radius * angle.cos(), cy - radius * angle.sin());
        let mut builder = PathBuilder::new();
        let (x, y) = self.point(&transform, self.state.position.0, self.state.position.1);
        builder.move_to(x, y);
        let (start_x, start_y) = at(start);
        let (device_x, device_y) = map_exact(&transform, start_x, start_y);
        builder.line_to(device_x, device_y);
        let map = |x: f64, y: f64| map_exact(&transform, x, y);
        arc_segments(&mut builder, (cx, cy), (radius, radius), start, sweep, &map);
        self.state.position = at(start + sweep);
        if let Some(path) = builder.finish() {
            if let Some(building) = self.path.building.as_mut() {
                append(building, &path, self.path.figure_open);
                self.path.figure_open = true;
                return;
            }
            self.stroke(&path, pixmap);
        }
    }

    /// Rectangles, rounded rectangles, ellipses, arcs, chords and pies
    /// within a bounding box.
    fn box_shape(&mut self, kind: u32, rect: [i32; 4], corner: (f64, f64), radials: Option<((f64, f64), (f64, f64))>, pixmap: &mut Pixmap) {
        let transform = self.transform();
        let inside_frame = self.state.pen.style & PS_STYLE_MASK == PS_INSIDEFRAME;
        let pen_width = if inside_frame { self.pen_width(&transform).0 as f64 } else { 0.0 };
        // Axis-aligned: in device pixels, GDI's way; otherwise the shape is
        // built in logical units and mapped.
        let (left, top, right, bottom, map): (f64, f64, f64, f64, PointMap) = if transform.is_axis_aligned() {
            let corner_a = transform.apply(rect[0] as f64, rect[1] as f64);
            let corner_b = transform.apply(rect[2] as f64, rect[3] as f64);
            let round = |value: f64| (value + 0.5).floor();
            let (mut left, mut right) = (round(corner_a.0.min(corner_b.0)), round(corner_a.0.max(corner_b.0)));
            let (mut top, mut bottom) = (round(corner_a.1.min(corner_b.1)), round(corner_a.1.max(corner_b.1)));
            right -= 1.0;
            bottom -= 1.0;
            if inside_frame && pen_width > 1.0 {
                let inset = (pen_width / 2.0).floor();
                left += inset;
                top += inset;
                right -= inset;
                bottom -= inset;
            }
            (left, top, right, bottom, Box::new(|x: f64, y: f64| (x as f32, y as f32)))
        } else {
            let (left, right) = ((rect[0].min(rect[2])) as f64, (rect[0].max(rect[2])) as f64);
            let (top, bottom) = ((rect[1].min(rect[3])) as f64, (rect[1].max(rect[3])) as f64);
            let mapped = transform;
            (left, top, right, bottom, Box::new(move |x: f64, y: f64| map_exact(&mapped, x, y)))
        };
        if right < left || bottom < top {
            return;
        }
        let device_corner = if transform.is_axis_aligned() {
            let (x, y) = transform.apply_vector(corner.0, corner.1);
            (x.abs(), y.abs())
        } else {
            corner
        };
        let mut builder = PathBuilder::new();
        let closed = kind != EMR_ARC && kind != EMR_ARCTO;
        match kind {
            EMR_RECTANGLE => {
                let points = [(left, top), (right, top), (right, bottom), (left, bottom)];
                let ordered = if self.state.arc_direction == AD_CLOCKWISE { [points[0], points[1], points[2], points[3]] } else { [points[1], points[0], points[3], points[2]] };
                let (x, y) = map(ordered[0].0, ordered[0].1);
                builder.move_to(x, y);
                for (x, y) in &ordered[1..] {
                    let (x, y) = map(*x, *y);
                    builder.line_to(x, y);
                }
                builder.close();
            }
            EMR_ROUNDRECT => {
                let radius_x = (device_corner.0 / 2.0).min((right - left) / 2.0);
                let radius_y = (device_corner.1 / 2.0).min((bottom - top) / 2.0);
                rounded_rect(&mut builder, left, top, right, bottom, radius_x, radius_y, &*map);
            }
            EMR_ELLIPSE => {
                let centre = ((left + right) / 2.0, (top + bottom) / 2.0);
                let radii = ((right - left) / 2.0, (bottom - top) / 2.0);
                let (x, y) = map(centre.0 + radii.0, centre.1);
                builder.move_to(x, y);
                arc_segments(&mut builder, centre, radii, 0.0, std::f64::consts::TAU, &*map);
                builder.close();
            }
            _ => {
                let Some((start, end)) = radials else {
                    return;
                };
                let centre = ((left + right) / 2.0, (top + bottom) / 2.0);
                let radii = ((right - left) / 2.0, (bottom - top) / 2.0);
                let (start, end) = if transform.is_axis_aligned() {
                    let start = transform.apply(start.0, start.1);
                    let end = transform.apply(end.0, end.1);
                    (((start.0 + 0.5).floor(), (start.1 + 0.5).floor()), ((end.0 + 0.5).floor(), (end.1 + 0.5).floor()))
                } else {
                    (start, end)
                };
                let angle_of = |point: (f64, f64)| (-(point.1 - centre.1) * radii.0).atan2((point.0 - centre.0) * radii.1);
                let start_angle = angle_of(start);
                let end_angle = angle_of(end);
                let mut sweep = end_angle - start_angle;
                // Counter-clockwise on the picture (whatever the mapping).
                let flipped = !transform.is_axis_aligned() && transform.determinant() < 0.0;
                let clockwise = (self.state.arc_direction == AD_CLOCKWISE) != flipped;
                if clockwise {
                    while sweep >= 0.0 {
                        sweep -= std::f64::consts::TAU;
                    }
                } else {
                    while sweep <= 0.0 {
                        sweep += std::f64::consts::TAU;
                    }
                }
                let point_at = |angle: f64| (centre.0 + radii.0 * angle.cos(), centre.1 - radii.1 * angle.sin());
                let first = point_at(start_angle);
                let (first_x, first_y) = map(first.0, first.1);
                if kind == EMR_PIE {
                    let (x, y) = map(centre.0, centre.1);
                    builder.move_to(x, y);
                    builder.line_to(first_x, first_y);
                } else if kind == EMR_ARCTO {
                    let transform = self.transform();
                    let (x, y) = self.point(&transform, self.state.position.0, self.state.position.1);
                    builder.move_to(x, y);
                    builder.line_to(first_x, first_y);
                } else {
                    builder.move_to(first_x, first_y);
                }
                arc_segments(&mut builder, centre, radii, start_angle, sweep, &*map);
                if closed {
                    builder.close();
                }
                if kind == EMR_ARCTO {
                    // The current position, back in logical units.
                    let last = point_at(start_angle + sweep);
                    let logical = if transform.is_axis_aligned() { transform.invert().map(|inverse| inverse.apply(last.0, last.1)) } else { Some(last) };
                    if let Some(position) = logical {
                        self.state.position = position;
                    }
                }
            }
        }
        let Some(path) = builder.finish() else {
            return;
        };
        if let Some(building) = self.path.building.as_mut() {
            append(building, &path, kind == EMR_ARCTO && self.path.figure_open);
            self.path.figure_open = kind == EMR_ARCTO;
            return;
        }
        if closed {
            self.fill(&path, pixmap);
        }
        self.stroke(&path, pixmap);
    }

    // Painting.

    /// The pen's width on the picture, and whether it is drawn one pixel
    /// wide.
    fn pen_width(&self, transform: &Affine) -> (f32, bool) {
        let pen = &self.state.pen;
        let geometric = pen.extended && pen.style & PS_GEOMETRIC != 0 || !pen.extended && pen.width > 1.0;
        if !geometric {
            return (1.0, true);
        }
        let width = (pen.width * transform.mean_scale()).round().max(1.0);
        (width as f32, width <= 1.0)
    }

    fn pen_stroke(&self) -> Stroke {
        let pen = &self.state.pen;
        let transform = self.transform();
        let (width, thin) = self.pen_width(&transform);
        let mut stroke = Stroke { width: if thin { 0.0 } else { width }, miter_limit: self.state.miter_limit, ..Stroke::default() };
        let (cap, join) = if pen.extended {
            let cap = match pen.style & PS_ENDCAP_MASK {
                PS_ENDCAP_SQUARE => LineCap::Square,
                PS_ENDCAP_FLAT => LineCap::Butt,
                _ => LineCap::Round,
            };
            let join = match pen.style & PS_JOIN_MASK {
                PS_JOIN_BEVEL => LineJoin::Bevel,
                PS_JOIN_MITER => LineJoin::Miter,
                _ => LineJoin::Round,
            };
            (cap, join)
        } else {
            (LineCap::Round, LineJoin::Round)
        };
        stroke.line_cap = cap;
        stroke.line_join = join;
        // Dashes: a pixel's lengths for one-pixel pens, the width's for
        // wider ones; `CreatePen`'s wide pens are always solid.
        let unit = if thin { 1.0 } else { width as f64 };
        let dashes: Vec<f64> = match pen.style & PS_STYLE_MASK {
            PS_DASH if thin => vec![18.0, 6.0],
            PS_DOT if thin => vec![3.0, 3.0],
            PS_DASHDOT if thin => vec![9.0, 6.0, 3.0, 6.0],
            PS_DASHDOTDOT if thin => vec![9.0, 3.0, 3.0, 3.0, 3.0, 3.0],
            PS_DASH if pen.extended => vec![3.0 * unit, unit],
            PS_DOT if pen.extended => vec![unit, unit],
            PS_DASHDOT if pen.extended => vec![3.0 * unit, unit, unit, unit],
            PS_DASHDOTDOT if pen.extended => vec![3.0 * unit, unit, unit, unit, unit, unit],
            PS_ALTERNATE => vec![1.0, 1.0],
            PS_USERSTYLE => {
                let scale = if thin { 1.0 } else { transform.mean_scale() };
                pen.dashes.iter().map(|length| length * scale).collect()
            }
            _ => Vec::new(),
        };
        let usable = dashes.len() >= 2 && dashes.len().is_multiple_of(2) && dashes.iter().all(|length| *length > 0.0);
        if usable {
            let intervals = dashes.iter().map(|length| *length as f32).collect();
            stroke.dash = StrokeDash::new(intervals, 0.0);
        }
        stroke
    }

    /// The pen's colour under `SetROP2`; `None` draws nothing.
    fn raster_colour(&self, colour: [u8; 3]) -> Option<[u8; 3]> {
        match self.state.raster_operation {
            R2_NOP => None,
            R2_BLACK => Some([0, 0, 0]),
            R2_WHITE => Some([255, 255, 255]),
            _ => Some(colour),
        }
    }

    fn stroke(&self, path: &Path, pixmap: &mut Pixmap) {
        let pen = &self.state.pen;
        if pen.is_null() {
            return;
        }
        let stroke = self.pen_stroke();
        if stroke.width == 0.0 {
            let dashes = stroke.dash.as_ref().map(|_| self.thin_dashes()).unwrap_or_default();
            self.thin_stroke(path, &dashes, pixmap);
            return;
        }
        if self.state.raster_operation == R2_NOT {
            self.invert_under(path, pixmap, true);
            return;
        }
        let Some(colour) = self.raster_colour(pen.colour) else {
            return;
        };
        let paint = solid_paint(colour, false);
        let clip = self.state.clip.as_deref();
        pixmap.stroke_path(path, &paint, &stroke, Transform::identity(), clip);
    }

    /// A one-pixel pen's dashes and gaps, in pixels.
    fn thin_dashes(&self) -> Vec<f64> {
        let pen = &self.state.pen;
        match pen.style & PS_STYLE_MASK {
            PS_DASH => vec![18.0, 6.0],
            PS_DOT => vec![3.0, 3.0],
            PS_DASHDOT => vec![9.0, 6.0, 3.0, 6.0],
            PS_DASHDOTDOT => vec![9.0, 3.0, 3.0, 3.0, 3.0, 3.0],
            PS_ALTERNATE => vec![1.0, 1.0],
            PS_USERSTYLE => pen.dashes.clone(),
            _ => Vec::new(),
        }
    }

    /// One-pixel pens, as GDI draws them: each segment lights its pixels
    /// from its start up to, not including, its end — so a closed figure
    /// lights every corner and an open one stops short of its last point
    /// — and dashes count whole pixels along the way.
    fn thin_stroke(&self, path: &Path, dashes: &[f64], pixmap: &mut Pixmap) {
        let invert = self.state.raster_operation == R2_NOT;
        let colour = self.raster_colour(self.state.pen.colour);
        if colour.is_none() && !invert {
            return;
        }
        let period: f64 = dashes.iter().sum();
        let mut step = 0usize;
        let mut plot = |x: i64, y: i64, pixmap: &mut Pixmap| {
            let on = if period > 0.0 {
                let mut phase = step as f64 % period;
                let mut on = true;
                for length in dashes {
                    if phase < *length {
                        break;
                    }
                    phase -= length;
                    on = !on;
                }
                on
            } else {
                true
            };
            step += 1;
            if !on {
                return;
            }
            if invert {
                self.invert_pixel(pixmap, x, y);
            } else if let Some(colour) = colour {
                self.set_pixel(pixmap, x, y, colour);
            }
        };
        for (points, closed) in flatten(path) {
            let count = points.len();
            let segments = if closed { count } else { count.saturating_sub(1) };
            for index in 0..segments {
                let (x0, y0) = points[index];
                let (x1, y1) = points[(index + 1) % count];
                line_pixels(x0, y0, x1, y1, |x, y| plot(x, y, pixmap));
            }
        }
    }

    fn invert_pixel(&self, pixmap: &mut Pixmap, x: i64, y: i64) {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            return;
        }
        let index = y as usize * self.width as usize + x as usize;
        if self.state.clip.as_deref().is_some_and(|clip| clip.data()[index] == 0) {
            return;
        }
        for channel in &mut pixmap.data_mut()[index * 4..index * 4 + 3] {
            *channel = 255 - *channel;
        }
    }

    fn fill(&self, path: &Path, pixmap: &mut Pixmap) {
        let brush = self.state.brush.clone();
        self.fill_with(&brush, path, self.fill_rule(), pixmap);
    }

    fn fill_with(&self, brush: &Brush, path: &Path, rule: FillRule, pixmap: &mut Pixmap) {
        if self.state.raster_operation == R2_NOP {
            return;
        }
        let clip = self.state.clip.as_deref();
        match brush {
            Brush::Null => {}
            Brush::Solid(colour) => {
                let Some(colour) = self.raster_colour(*colour) else {
                    return;
                };
                pixmap.fill_path(path, &solid_paint(colour, false), rule, Transform::identity(), clip);
            }
            Brush::Hatch { style, colour } => {
                let Some(tile) = self.hatch_tile(*style, *colour) else {
                    pixmap.fill_path(path, &solid_paint(*colour, false), rule, Transform::identity(), clip);
                    return;
                };
                let paint = self.pattern_paint(&tile);
                pixmap.fill_path(path, &paint, rule, Transform::identity(), clip);
            }
            Brush::Pattern { bitmap, monochrome } => {
                let Some(tile) = self.pattern_tile(bitmap, *monochrome) else {
                    return;
                };
                let paint = self.pattern_paint(&tile);
                pixmap.fill_path(path, &paint, rule, Transform::identity(), clip);
            }
        }
    }

    fn pattern_paint<'a>(&self, tile: &'a Pixmap) -> Paint<'a> {
        let (x, y) = self.playback.apply(self.state.brush_origin.0, self.state.brush_origin.1);
        let origin = Transform::from_translate(x.round() as f32, y.round() as f32);
        let shader = Pattern::new(tile.as_ref(), SpreadMode::Repeat, FilterQuality::Nearest, 1.0, origin);
        Paint { shader, anti_alias: false, ..Paint::default() }
    }

    /// A hatch's 8×8 tile: the brush colour on the background colour, or
    /// on nothing in transparent mode.
    fn hatch_tile(&self, style: u32, colour: [u8; 3]) -> Option<Pixmap> {
        let rows = HATCHES.get(style as usize)?;
        let mut tile = Pixmap::new(8, 8)?;
        let background = self.state.background_colour;
        let opaque = self.state.background_mode == OPAQUE;
        for (y, row) in rows.iter().enumerate() {
            for x in 0..8 {
                let set = row & (0x80 >> x) != 0;
                let rgba = if set { [colour[0], colour[1], colour[2], 255] } else if opaque { [background[0], background[1], background[2], 255] } else { [0, 0, 0, 0] };
                let at = (y * 8 + x) * 4;
                tile.data_mut()[at..at + 4].copy_from_slice(&rgba);
            }
        }
        Some(tile)
    }

    /// A pattern brush's tile; a monochrome one's set bits take the
    /// background colour, its clear ones the text colour.
    fn pattern_tile(&self, bitmap: &Dib, monochrome: bool) -> Option<Pixmap> {
        let mut tile = Pixmap::new(bitmap.width as u32, bitmap.height as u32)?;
        for y in 0..bitmap.height {
            for x in 0..bitmap.width {
                let rgb = if monochrome {
                    if bitmap.bit(x, y) { self.state.background_colour } else { self.state.text_colour }
                } else {
                    let [red, green, blue, _] = bitmap.pixel(x, y);
                    [red, green, blue]
                };
                let at = (y * bitmap.width + x) * 4;
                tile.data_mut()[at..at + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
        Some(tile)
    }

    /// `R2_NOT`: the pixels under a stroke (or a fill) inverted.
    fn invert_under(&self, path: &Path, pixmap: &mut Pixmap, stroke: bool) {
        let Some(mut mask) = Mask::new(self.width, self.height) else {
            return;
        };
        if stroke {
            let outline = path.stroke(&self.pen_stroke(), 1.0);
            let Some(outline) = outline else {
                return;
            };
            mask.fill_path(&outline, FillRule::Winding, false, Transform::identity());
        } else {
            mask.fill_path(path, self.fill_rule(), false, Transform::identity());
        }
        self.invert_masked(&mask, pixmap);
    }

    fn invert_masked(&self, mask: &Mask, pixmap: &mut Pixmap) {
        let clip = self.state.clip.as_deref().map(Mask::data);
        for (index, pixel) in pixmap.data_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let inside = mask.data()[index] != 0 && clip.is_none_or(|clip| clip[index] != 0);
            if inside {
                for channel in &mut pixel[..3] {
                    *channel = 255 - *channel;
                }
            }
        }
    }

    fn set_pixel(&self, pixmap: &mut Pixmap, x: i64, y: i64, colour: [u8; 3]) {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            return;
        }
        let index = y as usize * self.width as usize + x as usize;
        if self.state.clip.as_deref().is_some_and(|clip| clip.data()[index] == 0) {
            return;
        }
        let at = index * 4;
        pixmap.data_mut()[at..at + 4].copy_from_slice(&[colour[0], colour[1], colour[2], 255]);
    }

    /// `ExtFloodFill` with the brush's colour: outwards from a pixel to a
    /// border colour, or across the pixels of one colour.
    fn flood_fill(&self, pixmap: &mut Pixmap, x: i64, y: i64, colour: [u8; 3], surface: bool) {
        let (width, height) = (self.width as i64, self.height as i64);
        if x < 0 || y < 0 || x >= width || y >= height {
            return;
        }
        let fill = match &self.state.brush {
            Brush::Solid(fill) | Brush::Hatch { colour: fill, .. } => *fill,
            _ => return,
        };
        let matches = |pixmap: &Pixmap, x: i64, y: i64| {
            let at = (y as usize * self.width as usize + x as usize) * 4;
            let pixel = &pixmap.data()[at..at + 3];
            let same = pixel == colour;
            if surface { same } else { !same }
        };
        let mut done = vec![false; (width * height) as usize];
        let mut stack = vec![(x, y)];
        while let Some((x, y)) = stack.pop() {
            if x < 0 || y < 0 || x >= width || y >= height {
                continue;
            }
            let index = (y * width + x) as usize;
            if done[index] || !matches(pixmap, x, y) {
                continue;
            }
            done[index] = true;
            self.set_pixel(pixmap, x, y, fill);
            stack.extend([(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]);
        }
    }

    // Clipping.

    fn mask_of(&self, path: &Path, rule: FillRule) -> Mask {
        let mut mask = Mask::new(self.width, self.height).expect("the picture's size was checked");
        mask.fill_path(path, rule, false, Transform::identity());
        mask
    }

    /// Combines `region` with the clipping region (`ExtSelectClipRgn`'s
    /// modes); the meta region bounds the result.
    fn combine_clip(&mut self, region: Option<Mask>, mode: u32) {
        let Some(region) = region else {
            return;
        };
        let current = self.state.clip.clone();
        let combined: Option<Mask> = match mode {
            RGN_COPY => Some(region),
            RGN_AND => Some(match current {
                Some(current) => combine_masks(&current, &region, |a, b| a.min(b)),
                None => region,
            }),
            RGN_OR => current.map(|current| combine_masks(&current, &region, |a, b| a.max(b))),
            RGN_XOR => Some(match current {
                Some(current) => combine_masks(&current, &region, |a, b| a ^ b),
                None => inverted(&region),
            }),
            RGN_DIFF => Some(match current {
                Some(current) => combine_masks(&current, &region, |a, b| a.min(255 - b)),
                None => inverted(&region),
            }),
            _ => return,
        };
        let bounded = match (&self.meta, combined) {
            (Some(meta), Some(mask)) => Some(combine_masks(meta, &mask, |a, b| a.min(b))),
            (Some(meta), None) => Some((**meta).clone()),
            (None, mask) => mask,
        };
        self.state.clip = bounded.map(Rc::new);
    }

    fn offset_clip(&mut self, x: i64, y: i64) {
        let Some(clip) = self.state.clip.as_deref() else {
            return;
        };
        let (width, height) = (self.width as i64, self.height as i64);
        let mut moved = Mask::new(self.width, self.height).expect("the picture's size was checked");
        for row in 0..height {
            let from_row = row - y;
            if from_row < 0 || from_row >= height {
                continue;
            }
            for column in 0..width {
                let from_column = column - x;
                if from_column < 0 || from_column >= width {
                    continue;
                }
                moved.data_mut()[(row * width + column) as usize] = clip.data()[(from_row * width + from_column) as usize];
            }
        }
        self.state.clip = Some(Rc::new(moved));
    }

    /// A logical rectangle as a closed path on the picture, its corners on
    /// whole pixels (so its right and bottom edges are left out).
    fn rect_path(&self, rect: [i32; 4]) -> Option<Path> {
        let transform = self.transform();
        let corners = [(rect[0], rect[1]), (rect[2], rect[1]), (rect[2], rect[3]), (rect[0], rect[3])];
        let mut builder = PathBuilder::new();
        for (index, (x, y)) in corners.iter().enumerate() {
            let (x, y) = transform.apply(*x as f64, *y as f64);
            let (x, y) = ((x + 0.5).floor() as f32, (y + 0.5).floor() as f32);
            if index == 0 { builder.move_to(x, y) } else { builder.line_to(x, y) }
        }
        builder.close();
        builder.finish()
    }

    // Regions.

    fn region_paint(&mut self, kind: u32, data: &[u8], pixmap: &mut Pixmap) {
        let size = data.u32_at(24) as usize;
        let (brush_handle, region_at) = match kind {
            EMR_FILLRGN => (Some(data.u32_at(28)), 32),
            EMR_FRAMERGN => (Some(data.u32_at(28)), 40),
            _ => (None, 28),
        };
        let rects = read_region(data, region_at, size);
        let transform = self.transform();
        let brush = match brush_handle {
            Some(handle) => self.brush_of(handle),
            None => self.state.brush.clone(),
        };
        if kind == EMR_FRAMERGN {
            let (frame_x, frame_y) = (data.i32_at(32) as f64, data.i32_at(36) as f64);
            let mut frames = Vec::new();
            for rect in &rects {
                let [left, top, right, bottom] = rect.map(|value| value as f64);
                frames.push([left, top, right, top + frame_y]);
                frames.push([left, bottom - frame_y, right, bottom]);
                frames.push([left, top, left + frame_x, bottom]);
                frames.push([right - frame_x, top, right, bottom]);
            }
            let as_rects: Vec<[i32; 4]> = frames.iter().map(|rect| rect.map(|value| value as i32)).collect();
            if let Some(path) = rects_path(&as_rects, &transform) {
                self.fill_with(&brush, &path, FillRule::Winding, pixmap);
            }
            return;
        }
        let Some(path) = rects_path(&rects, &transform) else {
            return;
        };
        if kind == EMR_INVERTRGN {
            let mask = self.mask_of(&path, FillRule::Winding);
            self.invert_masked(&mask, pixmap);
            return;
        }
        self.fill_with(&brush, &path, FillRule::Winding, pixmap);
    }

    fn brush_of(&self, handle: u32) -> Brush {
        if handle & 0x8000_0000 != 0 {
            let grey = |value: u8| Brush::Solid([value, value, value]);
            return match handle & 0x7FFF_FFFF {
                0 | 18 => grey(255),
                1 => grey(192),
                2 => grey(128),
                3 => grey(64),
                4 => grey(0),
                _ => Brush::Null,
            };
        }
        match self.objects.get(handle as usize) {
            Some(Some(Object::Brush(brush))) => brush.clone(),
            _ => self.state.brush.clone(),
        }
    }

    // Bitmaps.

    fn stretch_blt(&mut self, kind: u32, data: &[u8], pixmap: &mut Pixmap) {
        let destination = [data.i32_at(24), data.i32_at(28), data.i32_at(32), data.i32_at(36)].map(|value| value as f64);
        let operation = data.u32_at(40);
        let source_origin = (data.i32_at(44) as f64, data.i32_at(48) as f64);
        let source_transform = read_xform(data, 52);
        let usage = data.u32_at(80);
        let bitmap = self.bitmap(data, usage, 84, 92);
        let source_size = if kind == EMR_BITBLT { (destination[2], destination[3]) } else { (data.i32_at(100) as f64, data.i32_at(104) as f64) };
        // The source rectangle in the bitmap, through the source's transform.
        let (source_x, source_y) = source_transform.apply(source_origin.0, source_origin.1);
        let (source_width, source_height) = source_transform.apply_vector(source_size.0, source_size.1);
        let source = [source_x, source_y, source_width, source_height];
        let mode = match kind {
            EMR_ALPHABLEND => BlitMode::Alpha { constant: (operation >> 16) as u8, per_pixel: (operation >> 24) & 1 != 0 },
            EMR_TRANSPARENTBLT => BlitMode::Transparent(self.colour_ref(operation)),
            _ => BlitMode::Raster(operation),
        };
        let transform = self.transform();
        let corners = parallelogram(&transform, destination);
        self.blit(pixmap, corners, bitmap.as_ref(), source, mode, None);
    }

    fn mask_blt(&mut self, data: &[u8], pixmap: &mut Pixmap) {
        let destination = [data.i32_at(24), data.i32_at(28), data.i32_at(32), data.i32_at(36)].map(|value| value as f64);
        let operation = data.u32_at(40);
        let source = [data.i32_at(44) as f64, data.i32_at(48) as f64, destination[2], destination[3]];
        let bitmap = self.bitmap(data, data.u32_at(80), 84, 92);
        let mask = self.bitmap(data, data.u32_at(108), 112, 120);
        let mask_origin = (data.i32_at(100) as f64, data.i32_at(104) as f64);
        let transform = self.transform();
        let corners = parallelogram(&transform, destination);
        let mask = mask.map(|mask| (mask, mask_origin));
        self.blit(pixmap, corners, bitmap.as_ref(), source, BlitMode::Masked(operation), mask.as_ref());
    }

    fn plg_blt(&mut self, data: &[u8], pixmap: &mut Pixmap) {
        let points: Vec<(f64, f64)> = (0..3).map(|index| (data.i32_at(24 + 8 * index) as f64, data.i32_at(28 + 8 * index) as f64)).collect();
        let source = [data.i32_at(48), data.i32_at(52), data.i32_at(56), data.i32_at(60)].map(|value| value as f64);
        let bitmap = self.bitmap(data, data.u32_at(92), 96, 104);
        let mask = self.bitmap(data, data.u32_at(120), 124, 132);
        let mask_origin = (data.i32_at(112) as f64, data.i32_at(116) as f64);
        let transform = self.transform();
        let origin = transform.apply(points[0].0, points[0].1);
        let along = transform.apply(points[1].0, points[1].1);
        let down = transform.apply(points[2].0, points[2].1);
        let corners = [origin, (along.0 - origin.0, along.1 - origin.1), (down.0 - origin.0, down.1 - origin.1)];
        let mask = mask.map(|mask| (mask, mask_origin));
        let operation = if mask.is_some() { BlitMode::Masked(0xAACC_0020) } else { BlitMode::Raster(0x00CC_0020) };
        self.blit(pixmap, corners, bitmap.as_ref(), source, operation, mask.as_ref());
    }

    fn set_dibits(&mut self, data: &[u8], pixmap: &mut Pixmap) {
        let destination = (data.i32_at(24) as f64, data.i32_at(28) as f64);
        let (source_x, source_y) = (data.i32_at(32) as f64, data.i32_at(36) as f64);
        let (width, height) = (data.i32_at(40) as f64, data.i32_at(44) as f64);
        let Some(bitmap) = self.bitmap(data, data.u32_at(64), 48, 56) else {
            return;
        };
        let first_scan = data.u32_at(68) as f64;
        // The bits are `cScans` lines from `iStartScan`, counted from the
        // bottom; the source rectangle's corner is its lower left.
        let top = bitmap.height as f64 - (source_y - first_scan) - height;
        let transform = self.transform();
        let corners = parallelogram(&transform, [destination.0, destination.1, width, height]);
        let source = [source_x, top, width, height];
        self.blit(pixmap, corners, Some(&bitmap), source, BlitMode::Raster(0x00CC_0020), None);
    }

    fn stretch_dibits(&mut self, data: &[u8], pixmap: &mut Pixmap) {
        let destination = [data.i32_at(24), data.i32_at(28), data.i32_at(72), data.i32_at(76)].map(|value| value as f64);
        let (source_x, source_y) = (data.i32_at(32) as f64, data.i32_at(36) as f64);
        let (source_width, source_height) = (data.i32_at(40) as f64, data.i32_at(44) as f64);
        let operation = data.u32_at(68);
        let usage = data.u32_at(64);
        let bitmap = self.bitmap(data, usage, 48, 56);
        // A bottom-up bitmap's source corner counts from its bottom.
        let info_offset = data.u32_at(48) as usize;
        let bottom_up = data.get(info_offset..).is_some_and(|info| info.u32_at(0) != 12 && info.i32_at(8) > 0);
        let top = match &bitmap {
            Some(bitmap) if bottom_up => bitmap.height as f64 - source_y - source_height,
            _ => source_y,
        };
        let transform = self.transform();
        let corners = parallelogram(&transform, destination);
        let source = [source_x, top, source_width, source_height];
        self.blit(pixmap, corners, bitmap.as_ref(), source, BlitMode::Raster(operation), None);
    }

    /// Draws a source rectangle of `bitmap` into a parallelogram on the
    /// picture (`corners`: an origin and two sides), each pixel whose
    /// centre falls inside taking the source pixel it maps to; the raster
    /// operation combines it with the brush and the picture.
    #[allow(clippy::too_many_arguments)]
    fn blit(&self, pixmap: &mut Pixmap, corners: [(f64, f64); 3], bitmap: Option<&Dib>, source: [f64; 4], mode: BlitMode, mask: Option<&(Dib, (f64, f64))>) {
        let [origin, along, down] = corners;
        let determinant = along.0 * down.1 - along.1 * down.0;
        if determinant == 0.0 {
            return;
        }
        let xs = [origin.0, origin.0 + along.0, origin.0 + down.0, origin.0 + along.0 + down.0];
        let ys = [origin.1, origin.1 + along.1, origin.1 + down.1, origin.1 + along.1 + down.1];
        let left = xs.iter().copied().fold(f64::INFINITY, f64::min).floor().max(0.0) as i64;
        let right = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max).ceil().min(self.width as f64) as i64;
        let top = ys.iter().copied().fold(f64::INFINITY, f64::min).floor().max(0.0) as i64;
        let bottom = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max).ceil().min(self.height as f64) as i64;
        let pattern = match &self.state.brush {
            Brush::Solid(colour) | Brush::Hatch { colour, .. } => *colour,
            _ => [0, 0, 0],
        };
        let halftone = self.state.stretch_mode == HALFTONE;
        let clip = self.state.clip.as_deref().map(Mask::data);
        let width = self.width as usize;
        let data = pixmap.data_mut();
        for y in top..bottom {
            for x in left..right {
                let (px, py) = (x as f64 + 0.5 - origin.0, y as f64 + 0.5 - origin.1);
                let u = (px * down.1 - py * down.0) / determinant;
                let v = (along.0 * py - along.1 * px) / determinant;
                if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                    continue;
                }
                let index = y as usize * width + x as usize;
                if clip.is_some_and(|clip| clip[index] == 0) {
                    continue;
                }
                let source_x = source[0] + u * source[2];
                let source_y = source[1] + v * source[3];
                let sample = bitmap.map(|bitmap| sample(bitmap, source_x, source_y, halftone));
                let at = index * 4;
                let current = [data[at], data[at + 1], data[at + 2]];
                let result = match mode {
                    BlitMode::Raster(operation) => {
                        let code = (operation >> 16) as u8;
                        let source_rgb = sample.map(|[red, green, blue, _]| [red, green, blue]).unwrap_or([0, 0, 0]);
                        Some(rop3_rgb(code, pattern, source_rgb, current))
                    }
                    BlitMode::Masked(operation) => {
                        let mask_bit = mask.is_none_or(|(mask, (mask_x, mask_y))| {
                            let mx = (mask_x + u * source[2]).floor();
                            let my = (mask_y + v * source[3]).floor();
                            mx >= 0.0 && my >= 0.0 && (mx as usize) < mask.width && (my as usize) < mask.height && mask.bit(mx as usize, my as usize)
                        });
                        let code = if mask_bit { (operation >> 16) as u8 } else { (operation >> 24) as u8 };
                        let source_rgb = sample.map(|[red, green, blue, _]| [red, green, blue]).unwrap_or([0, 0, 0]);
                        Some(rop3_rgb(code, pattern, source_rgb, current))
                    }
                    BlitMode::Transparent(key) => sample.and_then(|[red, green, blue, _]| ([red, green, blue] != key).then_some([red, green, blue])),
                    BlitMode::Alpha { constant, per_pixel } => sample.map(|[red, green, blue, alpha]| {
                        let constant = constant as f64 / 255.0;
                        let blend = |source: u8, destination: u8| -> u8 {
                            let value = if per_pixel {
                                source as f64 * constant + destination as f64 * (1.0 - alpha as f64 / 255.0 * constant)
                            } else {
                                source as f64 * constant + destination as f64 * (1.0 - constant)
                            };
                            value.round().clamp(0.0, 255.0) as u8
                        };
                        [blend(red, current[0]), blend(green, current[1]), blend(blue, current[2])]
                    }),
                };
                if let Some([red, green, blue]) = result {
                    data[at..at + 4].copy_from_slice(&[red, green, blue, 255]);
                }
            }
        }
    }

    // Gradients.

    fn gradient_fill(&mut self, data: &[u8], pixmap: &mut Pixmap) {
        let vertex_count = data.u32_at(24) as usize;
        let object_count = data.u32_at(28) as usize;
        let mode = data.u32_at(32);
        let objects_at = 36 + 16 * vertex_count;
        if objects_at > data.len() {
            return;
        }
        let transform = self.transform();
        let vertices: Vec<((f64, f64), [f64; 3])> = (0..vertex_count)
            .map(|index| {
                let at = 36 + 16 * index;
                let (x, y) = transform.apply(data.i32_at(at) as f64, data.i32_at(at + 4) as f64);
                let colour = [data.u16_at(at + 8), data.u16_at(at + 10), data.u16_at(at + 12)].map(|value| (value >> 8) as f64);
                (((x + 0.5).floor(), (y + 0.5).floor()), colour)
            })
            .collect();
        let width = self.width as i64;
        let height = self.height as i64;
        let clip = self.state.clip.clone();
        let mut put = |x: i64, y: i64, colour: [f64; 3]| {
            if x < 0 || y < 0 || x >= width || y >= height {
                return;
            }
            let index = (y * width + x) as usize;
            if clip.as_deref().is_some_and(|clip| clip.data()[index] == 0) {
                return;
            }
            let rgb = colour.map(|value| value.round().clamp(0.0, 255.0) as u8);
            pixmap.data_mut()[index * 4..index * 4 + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        };
        if mode == 2 {
            for index in 0..object_count {
                let at = objects_at + 12 * index;
                let corners = [data.u32_at(at), data.u32_at(at + 4), data.u32_at(at + 8)].map(|vertex| vertices.get(vertex as usize));
                let [Some(a), Some(b), Some(c)] = corners else {
                    continue;
                };
                fill_triangle(*a, *b, *c, &mut put);
            }
            return;
        }
        for index in 0..object_count {
            let at = objects_at + 8 * index;
            let (Some(first), Some(second)) = (vertices.get(data.u32_at(at) as usize), vertices.get(data.u32_at(at + 4) as usize)) else {
                continue;
            };
            let (left, right) = (first.0.0.min(second.0.0) as i64, first.0.0.max(second.0.0) as i64);
            let (top, bottom) = (first.0.1.min(second.0.1) as i64, first.0.1.max(second.0.1) as i64);
            let (start, end) = if mode == 0 { if first.0.0 <= second.0.0 { (first.1, second.1) } else { (second.1, first.1) } } else if first.0.1 <= second.0.1 { (first.1, second.1) } else { (second.1, first.1) };
            for y in top..bottom {
                for x in left..right {
                    let t = if mode == 0 { (x - left) as f64 / (right - left).max(1) as f64 } else { (y - top) as f64 / (bottom - top).max(1) as f64 };
                    let colour = [0, 1, 2].map(|channel| start[channel] + (end[channel] - start[channel]) * t);
                    put(x, y, colour);
                }
            }
        }
    }

    // Text.

    /// One `EMRTEXT` at `at`: `ExtTextOut` with the selected font.
    fn text_out(&mut self, data: &[u8], at: usize, graphics_mode: u32, wide: bool, pixmap: &mut Pixmap) {
        let reference = (data.i32_at(at) as f64, data.i32_at(at + 4) as f64);
        let count = data.u32_at(at + 8) as usize;
        let string_at = data.u32_at(at + 12) as usize;
        let options = data.u32_at(at + 16);
        let rect = read_rect(data, at + 20);
        let dx_at = data.u32_at(at + 36) as usize;
        let codes: Vec<u32> = if wide {
            (0..count).map(|index| data.u16_at(string_at + 2 * index) as u32).collect()
        } else {
            (0..count).map(|index| data.u8_at(string_at + index) as u32).collect()
        };
        let pairs = options & ETO_PDY != 0;
        let advances: Option<Vec<(f64, f64)>> = (dx_at != 0 && dx_at < data.len()).then(|| {
            (0..count)
                .map(|index| if pairs { (data.i32_at(dx_at + 8 * index) as f64, data.i32_at(dx_at + 8 * index + 4) as f64) } else { (data.i32_at(dx_at + 4 * index) as f64, 0.0) })
                .collect()
        });
        let has_rect = options & ETO_NO_RECT == 0;
        let run = TextCall { reference, codes, glyph_indices: options & ETO_GLYPH_INDEX != 0, advances, options, rect: has_rect.then_some(rect), advanced: graphics_mode == 2 };
        self.draw_text(&run, pixmap);
    }

    fn small_text_out(&mut self, data: &[u8], pixmap: &mut Pixmap) {
        let reference = (data.i32_at(8) as f64, data.i32_at(12) as f64);
        let count = data.u32_at(16) as usize;
        let options = data.u32_at(20);
        let graphics_mode = data.u32_at(24);
        let has_rect = options & ETO_NO_RECT == 0;
        let text_at = if has_rect { 52 } else { 36 };
        let rect = read_rect(data, 36);
        let codes: Vec<u32> = if options & ETO_SMALL_CHARS != 0 {
            (0..count).map(|index| data.u8_at(text_at + index) as u32).collect()
        } else {
            (0..count).map(|index| data.u16_at(text_at + 2 * index) as u32).collect()
        };
        let run = TextCall { reference, codes, glyph_indices: options & ETO_GLYPH_INDEX != 0, advances: None, options, rect: has_rect.then_some(rect), advanced: graphics_mode == 2 };
        self.draw_text(&run, pixmap);
    }

    fn draw_text(&mut self, call: &TextCall, pixmap: &mut Pixmap) {
        let transform = self.transform();
        let font = self.state.font.clone();
        let clip_rect = call.rect.and_then(|rect| self.rect_path(rect));
        // `ETO_OPAQUE`: the rectangle filled with the background colour.
        if call.options & ETO_OPAQUE != 0
            && let Some(path) = &clip_rect
        {
            let clip = self.state.clip.as_deref();
            pixmap.fill_path(path, &solid_paint(self.state.background_colour, false), FillRule::Winding, Transform::identity(), clip);
        }
        let align = self.state.text_align;
        let update_position = align & 1 != 0;
        let reference = if update_position { self.state.position } else { call.reference };
        let request = text::GdiText {
            font: &font,
            codes: &call.codes,
            glyph_indices: call.glyph_indices,
            advances: call.advances.as_deref(),
            reference,
            align,
            transform,
            advanced: call.advanced,
        };
        let Some(laid_out) = text::lay_out_gdi(&request) else {
            return;
        };
        if update_position {
            self.state.position = laid_out.next_position;
        }
        // The text's cell in the background colour, in opaque mode.
        if self.state.background_mode == OPAQUE
            && call.options & ETO_OPAQUE == 0
            && let Some(cell) = &laid_out.cell
        {
            let clip = self.state.clip.as_deref();
            pixmap.fill_path(cell, &solid_paint(self.state.background_colour, true), FillRule::Winding, Transform::identity(), clip);
        }
        let Some(outline) = laid_out.outline else {
            return;
        };
        let embolden = laid_out.embolden;
        let clipped;
        let clip = match (call.options & ETO_CLIPPED != 0, &clip_rect) {
            (true, Some(rect)) => {
                let mut mask = self.mask_of(rect, FillRule::Winding);
                if let Some(current) = self.state.clip.as_deref() {
                    mask = combine_masks(current, &mask, |a, b| a.min(b));
                }
                clipped = mask;
                Some(&clipped)
            }
            _ => self.state.clip.as_deref(),
        };
        let paint = solid_paint(self.state.text_colour, true);
        pixmap.fill_path(&outline, &paint, FillRule::Winding, Transform::identity(), clip);
        // A regular face shown bold: its outline thickened.
        if let Some(width) = embolden {
            let stroke = Stroke { width, ..Stroke::default() };
            pixmap.stroke_path(&outline, &paint, &stroke, Transform::identity(), clip);
        }
    }
}

/// Maps a shape's points onto the picture.
type PointMap = Box<dyn Fn(f64, f64) -> (f32, f32)>;

/// An `ExtTextOut` call's parts.
struct TextCall {
    reference: (f64, f64),
    codes: Vec<u32>,
    glyph_indices: bool,
    advances: Option<Vec<(f64, f64)>>,
    options: u32,
    rect: Option<[i32; 4]>,
    /// `GM_ADVANCED`: the text follows the world transform.
    advanced: bool,
}

#[derive(Clone, Copy)]
enum BlitMode {
    /// A ternary raster operation (`SRCCOPY` …).
    Raster(u32),
    /// `MaskBlt`'s quaternary one: the foreground operation where the mask
    /// is set, the background one where it is clear.
    Masked(u32),
    /// `TransparentBlt`: every pixel but this colour.
    Transparent([u8; 3]),
    /// `AlphaBlend`: a constant opacity, and the source's own alpha
    /// (premultiplied) when it has one.
    Alpha { constant: u8, per_pixel: bool },
}

/// A ternary raster operation on each colour byte: each bit of the code
/// says what one combination of pattern, source and destination bits
/// gives.
fn rop3(code: u8, pattern: u8, source: u8, destination: u8) -> u8 {
    let mut result = 0u8;
    for combination in 0..8u8 {
        if code & (1 << combination) == 0 {
            continue;
        }
        let pattern_bits = if combination & 4 != 0 { pattern } else { !pattern };
        let source_bits = if combination & 2 != 0 { source } else { !source };
        let destination_bits = if combination & 1 != 0 { destination } else { !destination };
        result |= pattern_bits & source_bits & destination_bits;
    }
    result
}

fn rop3_rgb(code: u8, pattern: [u8; 3], source: [u8; 3], destination: [u8; 3]) -> [u8; 3] {
    if code == 0xCC {
        return source;
    }
    [0, 1, 2].map(|channel| rop3(code, pattern[channel], source[channel], destination[channel]))
}

/// The source pixel under a point: the nearest, or (`HALFTONE`) the four
/// around it blended.
fn sample(bitmap: &Dib, x: f64, y: f64, smooth: bool) -> [u8; 4] {
    let clamp_x = |value: f64| value.clamp(0.0, bitmap.width as f64 - 1.0) as usize;
    let clamp_y = |value: f64| value.clamp(0.0, bitmap.height as f64 - 1.0) as usize;
    if !smooth {
        return bitmap.pixel(clamp_x(x.floor()), clamp_y(y.floor()));
    }
    let (fx, fy) = (x - 0.5, y - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let corners = [bitmap.pixel(clamp_x(x0), clamp_y(y0)), bitmap.pixel(clamp_x(x0 + 1.0), clamp_y(y0)), bitmap.pixel(clamp_x(x0), clamp_y(y0 + 1.0)), bitmap.pixel(clamp_x(x0 + 1.0), clamp_y(y0 + 1.0))];
    [0, 1, 2, 3].map(|channel| {
        let top = corners[0][channel] as f64 * (1.0 - tx) + corners[1][channel] as f64 * tx;
        let bottom = corners[2][channel] as f64 * (1.0 - tx) + corners[3][channel] as f64 * tx;
        (top * (1.0 - ty) + bottom * ty).round() as u8
    })
}

/// A logical rectangle's corner and sides on the picture, its corners
/// rounded to whole pixels.
fn parallelogram(transform: &Affine, rect: [f64; 4]) -> [(f64, f64); 3] {
    let round = |(x, y): (f64, f64)| ((x + 0.5).floor(), (y + 0.5).floor());
    let origin = round(transform.apply(rect[0], rect[1]));
    let along = round(transform.apply(rect[0] + rect[2], rect[1]));
    let down = round(transform.apply(rect[0], rect[1] + rect[3]));
    [origin, (along.0 - origin.0, along.1 - origin.1), (down.0 - origin.0, down.1 - origin.1)]
}

/// Gouraud shading: each pixel whose centre is inside the triangle takes
/// the corners' colours by its barycentric weights.
fn fill_triangle(a: ((f64, f64), [f64; 3]), b: ((f64, f64), [f64; 3]), c: ((f64, f64), [f64; 3]), put: &mut impl FnMut(i64, i64, [f64; 3])) {
    let ((ax, ay), (bx, by), (cx, cy)) = (a.0, b.0, c.0);
    let area = (bx - ax) * (cy - ay) - (cx - ax) * (by - ay);
    if area == 0.0 {
        return;
    }
    let left = ax.min(bx).min(cx).floor() as i64;
    let right = ax.max(bx).max(cx).ceil() as i64;
    let top = ay.min(by).min(cy).floor() as i64;
    let bottom = ay.max(by).max(cy).ceil() as i64;
    for y in top..bottom {
        for x in left..right {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let weight_a = ((bx - px) * (cy - py) - (cx - px) * (by - py)) / area;
            let weight_b = ((cx - px) * (ay - py) - (ax - px) * (cy - py)) / area;
            let weight_c = 1.0 - weight_a - weight_b;
            if weight_a < 0.0 || weight_b < 0.0 || weight_c < 0.0 {
                continue;
            }
            let colour = [0, 1, 2].map(|channel| a.1[channel] * weight_a + b.1[channel] * weight_b + c.1[channel] * weight_c);
            put(x, y, colour);
        }
    }
}

fn solid_paint(colour: [u8; 3], anti_alias: bool) -> Paint<'static> {
    let shader = Shader::SolidColor(Color::from_rgba8(colour[0], colour[1], colour[2], 255));
    Paint { shader, anti_alias, ..Paint::default() }
}

/// A path's figures as whole-pixel points, curves broken into short
/// lines, each with whether it is closed.
fn flatten(path: &Path) -> Vec<(Vec<(i64, i64)>, bool)> {
    use resvg::tiny_skia::PathSegment;
    let round = |x: f32, y: f32| ((x as f64 + 0.5).floor() as i64, (y as f64 + 0.5).floor() as i64);
    let mut figures: Vec<(Vec<(i64, i64)>, bool)> = Vec::new();
    let mut last = (0.0f32, 0.0f32);
    let push = |figures: &mut Vec<(Vec<(i64, i64)>, bool)>, point: (i64, i64)| {
        if let Some((points, _)) = figures.last_mut()
            && points.last() != Some(&point)
        {
            points.push(point);
        }
    };
    for segment in path.segments() {
        match segment {
            PathSegment::MoveTo(point) => {
                figures.push((vec![round(point.x, point.y)], false));
                last = (point.x, point.y);
            }
            PathSegment::LineTo(point) => {
                push(&mut figures, round(point.x, point.y));
                last = (point.x, point.y);
            }
            PathSegment::QuadTo(control, point) => {
                let steps = curve_steps(&[last, (control.x, control.y), (point.x, point.y)]);
                for step in 1..=steps {
                    let t = step as f32 / steps as f32;
                    let u = 1.0 - t;
                    let x = u * u * last.0 + 2.0 * u * t * control.x + t * t * point.x;
                    let y = u * u * last.1 + 2.0 * u * t * control.y + t * t * point.y;
                    push(&mut figures, round(x, y));
                }
                last = (point.x, point.y);
            }
            PathSegment::CubicTo(one, two, point) => {
                let steps = curve_steps(&[last, (one.x, one.y), (two.x, two.y), (point.x, point.y)]);
                for step in 1..=steps {
                    let t = step as f32 / steps as f32;
                    let u = 1.0 - t;
                    let x = u * u * u * last.0 + 3.0 * u * u * t * one.x + 3.0 * u * t * t * two.x + t * t * t * point.x;
                    let y = u * u * u * last.1 + 3.0 * u * u * t * one.y + 3.0 * u * t * t * two.y + t * t * t * point.y;
                    push(&mut figures, round(x, y));
                }
                last = (point.x, point.y);
            }
            PathSegment::Close => {
                if let Some((points, closed)) = figures.last_mut() {
                    *closed = true;
                    if points.len() > 1 && points.first() == points.last() {
                        points.pop();
                    }
                }
            }
        }
    }
    figures
}

/// How many lines a curve breaks into: about one per two pixels of its
/// control polygon.
fn curve_steps(points: &[(f32, f32)]) -> usize {
    let length: f32 = points.windows(2).map(|pair| (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1)).sum();
    ((length / 2.0).ceil() as usize).clamp(2, 512)
}

/// Bresenham's line from `(x0, y0)` towards `(x1, y1)`, its end left out.
fn line_pixels(x0: i64, y0: i64, x1: i64, y1: i64, mut plot: impl FnMut(i64, i64)) {
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (step_x, step_y) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let (mut x, mut y) = (x0, y0);
    let mut error = dx + dy;
    while (x, y) != (x1, y1) {
        plot(x, y);
        let doubled = 2 * error;
        if doubled >= dy {
            error += dy;
            x += step_x;
        }
        if doubled <= dx {
            error += dx;
            y += step_y;
        }
    }
}

/// A point mapped without rounding.
fn map_exact(transform: &Affine, x: f64, y: f64) -> (f32, f32) {
    let (x, y) = transform.apply(x, y);
    (x as f32, y as f32)
}

/// Adds an elliptical arc as cubic Béziers (a quarter turn at most each):
/// from `start` (radians, counter-clockwise with y down) through `sweep`.
pub(super) fn arc_segments(builder: &mut PathBuilder, centre: (f64, f64), radii: (f64, f64), start: f64, sweep: f64, map: &dyn Fn(f64, f64) -> (f32, f32)) {
    let segments = (sweep.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = sweep / segments as f64;
    let handle = 4.0 / 3.0 * (step / 4.0).tan();
    let point = |angle: f64| (centre.0 + radii.0 * angle.cos(), centre.1 - radii.1 * angle.sin());
    let tangent = |angle: f64| (-radii.0 * angle.sin(), -radii.1 * angle.cos());
    for segment in 0..segments {
        let from = start + step * segment as f64;
        let to = from + step;
        let (x0, y0) = point(from);
        let (x3, y3) = point(to);
        let (dx0, dy0) = tangent(from);
        let (dx3, dy3) = tangent(to);
        let (x1, y1) = map(x0 + handle * dx0, y0 + handle * dy0);
        let (x2, y2) = map(x3 - handle * dx3, y3 - handle * dy3);
        let (x3, y3) = map(x3, y3);
        builder.cubic_to(x1, y1, x2, y2, x3, y3);
    }
}

#[allow(clippy::too_many_arguments)]
fn rounded_rect(builder: &mut PathBuilder, left: f64, top: f64, right: f64, bottom: f64, radius_x: f64, radius_y: f64, map: &dyn Fn(f64, f64) -> (f32, f32)) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let (x, y) = map(right - radius_x, top);
    builder.move_to(x, y);
    arc_segments(builder, (right - radius_x, top + radius_y), (radius_x, radius_y), FRAC_PI_2, -FRAC_PI_2, map);
    let (x, y) = map(right, bottom - radius_y);
    builder.line_to(x, y);
    arc_segments(builder, (right - radius_x, bottom - radius_y), (radius_x, radius_y), 0.0, -FRAC_PI_2, map);
    let (x, y) = map(left + radius_x, bottom);
    builder.line_to(x, y);
    arc_segments(builder, (left + radius_x, bottom - radius_y), (radius_x, radius_y), -FRAC_PI_2, -FRAC_PI_2, map);
    let (x, y) = map(left, top + radius_y);
    builder.line_to(x, y);
    arc_segments(builder, (left + radius_x, top + radius_y), (radius_x, radius_y), PI, -FRAC_PI_2, map);
    builder.close();
}

/// Appends `path` to `builder`; `joined` carries on its open figure
/// instead of starting one.
fn append(builder: &mut PathBuilder, path: &Path, joined: bool) {
    use resvg::tiny_skia::PathSegment;
    let mut first = true;
    for segment in path.segments() {
        match segment {
            PathSegment::MoveTo(point) => {
                if first && joined {
                    builder.line_to(point.x, point.y);
                } else {
                    builder.move_to(point.x, point.y);
                }
            }
            PathSegment::LineTo(point) => builder.line_to(point.x, point.y),
            PathSegment::QuadTo(control, point) => builder.quad_to(control.x, control.y, point.x, point.y),
            PathSegment::CubicTo(one, two, point) => builder.cubic_to(one.x, one.y, two.x, two.y, point.x, point.y),
            PathSegment::Close => builder.close(),
        }
        first = false;
    }
}

/// `path` with every figure closed (for filling).
fn close_all(path: &Path) -> Path {
    use resvg::tiny_skia::PathSegment;
    let mut builder = PathBuilder::new();
    let mut open = false;
    for segment in path.segments() {
        match segment {
            PathSegment::MoveTo(point) => {
                if open {
                    builder.close();
                }
                builder.move_to(point.x, point.y);
                open = true;
            }
            PathSegment::LineTo(point) => builder.line_to(point.x, point.y),
            PathSegment::QuadTo(control, point) => builder.quad_to(control.x, control.y, point.x, point.y),
            PathSegment::CubicTo(one, two, point) => builder.cubic_to(one.x, one.y, two.x, two.y, point.x, point.y),
            PathSegment::Close => {
                builder.close();
                open = false;
            }
        }
    }
    if open {
        builder.close();
    }
    builder.finish().unwrap_or_else(|| path.clone())
}

fn combine_masks(first: &Mask, second: &Mask, operation: impl Fn(u8, u8) -> u8) -> Mask {
    let mut combined = first.clone();
    for (value, other) in combined.data_mut().iter_mut().zip(second.data()) {
        *value = operation(*value, *other);
    }
    combined
}

fn inverted(mask: &Mask) -> Mask {
    let mut copy = mask.clone();
    copy.invert();
    copy
}

/// A region's rectangles as one path, through `transform`.
fn rects_path(rects: &[[i32; 4]], transform: &Affine) -> Option<Path> {
    let mut builder = PathBuilder::new();
    for rect in rects {
        let corners = [(rect[0], rect[1]), (rect[2], rect[1]), (rect[2], rect[3]), (rect[0], rect[3])];
        for (index, (x, y)) in corners.iter().enumerate() {
            let (x, y) = transform.apply(*x as f64, *y as f64);
            let (x, y) = ((x + 0.5).floor() as f32, (y + 0.5).floor() as f32);
            if index == 0 { builder.move_to(x, y) } else { builder.line_to(x, y) }
        }
        builder.close();
    }
    builder.finish()
}

/// `RGNDATA`'s rectangles.
fn read_region(data: &[u8], at: usize, size: usize) -> Vec<[i32; 4]> {
    let count = data.u32_at(at + 8) as usize;
    let header = data.u32_at(at) as usize;
    let header = if header >= 32 { header } else { 32 };
    let available = size.saturating_sub(header) / 16;
    (0..count.min(available)).map(|index| read_rect(data, at + header + 16 * index)).collect()
}

fn read_rect(data: &[u8], at: usize) -> [i32; 4] {
    [data.i32_at(at), data.i32_at(at + 4), data.i32_at(at + 8), data.i32_at(at + 12)]
}

fn read_xform(data: &[u8], at: usize) -> Affine {
    let values = [0, 1, 2, 3, 4, 5].map(|index| data.f32_at(at + 4 * index));
    Affine::from_floats(values)
}

/// A record's counted points (`cptl`, then the points), after its bounds.
fn read_points(data: &[u8], count_at: usize, small: bool) -> Vec<(f64, f64)> {
    let count = data.u32_at(count_at) as usize;
    let point_size = if small { 4 } else { 8 };
    let count = count.min(data.len().saturating_sub(count_at + 4) / point_size);
    read_point_list(data, count_at + 4, count, small)
}

fn read_point_list(data: &[u8], at: usize, count: usize, small: bool) -> Vec<(f64, f64)> {
    (0..count)
        .map(|index| {
            if small {
                (data.i16_at(at + 4 * index) as f64, data.i16_at(at + 4 * index + 2) as f64)
            } else {
                (data.i32_at(at + 8 * index) as f64, data.i32_at(at + 8 * index + 4) as f64)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raster_operations_follow_their_truth_tables() {
        // SRCCOPY, PATCOPY, DSTINVERT, SRCAND, SRCPAINT, SRCINVERT.
        assert_eq!(rop3(0xCC, 0x12, 0x34, 0x56), 0x34);
        assert_eq!(rop3(0xF0, 0x12, 0x34, 0x56), 0x12);
        assert_eq!(rop3(0x55, 0x12, 0x34, 0x56), !0x56);
        assert_eq!(rop3(0x88, 0x12, 0x34, 0x56), 0x34 & 0x56);
        assert_eq!(rop3(0xEE, 0x12, 0x34, 0x56), 0x34 | 0x56);
        assert_eq!(rop3(0x66, 0x12, 0x34, 0x56), 0x34 ^ 0x56);
    }
}
