//! Text in metafiles: the fonts a record names, found among the
//! system's fonts (as the font mapper would pick them: the family, then
//! a stand-in for the common Windows ones, then by pitch and family), and
//! their glyphs laid out as outlines — `ExtTextOut`'s runs and EMF+'s
//! strings and glyph runs.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path as FilePath, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use resvg::tiny_skia::{Path, PathBuilder};
use ttf_parser::{Face, GlyphId, OutlineBuilder, Tag};

use super::Affine;

/// A `LOGFONTW`.
#[derive(Clone, Debug)]
pub(super) struct LogFont {
    pub height: i32,
    pub width: i32,
    pub escapement: i32,
    pub weight: i32,
    pub italic: bool,
    pub underline: bool,
    pub strike_out: bool,
    pub charset: u8,
    pub pitch_and_family: u8,
    pub face: String,
}

impl Default for LogFont {
    /// The stock fonts: a 16-unit sans-serif.
    fn default() -> LogFont {
        LogFont { height: 16, width: 0, escapement: 0, weight: 400, italic: false, underline: false, strike_out: false, charset: 0, pitch_and_family: 0, face: String::new() }
    }
}

impl LogFont {
    pub fn read(bytes: &[u8]) -> LogFont {
        use super::Bytes;
        let face_units: Vec<u16> = (0..32).map(|index| bytes.u16_at(28 + 2 * index)).take_while(|unit| *unit != 0).collect();
        LogFont {
            height: bytes.i32_at(0),
            width: bytes.i32_at(4),
            escapement: bytes.i32_at(8),
            weight: bytes.i32_at(16),
            italic: bytes.u8_at(20) != 0,
            underline: bytes.u8_at(21) != 0,
            strike_out: bytes.u8_at(22) != 0,
            charset: bytes.u8_at(23),
            pitch_and_family: bytes.u8_at(27),
            face: String::from_utf16_lossy(&face_units),
        }
    }
}

/// `TA_…` alignments.
const TA_RIGHT: u32 = 2;
const TA_CENTER: u32 = 6;
const TA_BOTTOM: u32 = 8;
const TA_BASELINE: u32 = 24;
/// `SYMBOL_CHARSET`.
const SYMBOL_CHARSET: u8 = 2;

/// An `ExtTextOut` run to lay out.
pub(super) struct GdiText<'a> {
    pub font: &'a LogFont,
    pub codes: &'a [u32],
    pub glyph_indices: bool,
    /// The spacing array: each character's advance (and rise), logical.
    pub advances: Option<&'a [(f64, f64)]>,
    /// The reference point, logical.
    pub reference: (f64, f64),
    pub align: u32,
    /// Logical units to the picture.
    pub transform: Affine,
    /// `GM_ADVANCED`: the text follows the transform; otherwise it stays
    /// upright on the picture, only its size and position mapped.
    pub advanced: bool,
}

/// Text laid out on the picture.
pub(super) struct LaidOut {
    pub outline: Option<Path>,
    /// The character cell, which opaque mode fills with the background.
    pub cell: Option<Path>,
    /// Where `TA_UPDATECP` leaves the current position, logical.
    pub next_position: (f64, f64),
    /// A stroke width that thickens a regular face shown bold.
    pub embolden: Option<f32>,
}

pub(super) fn lay_out_gdi(request: &GdiText) -> Option<LaidOut> {
    let font = request.font;
    let wants_bold = font.weight >= 600;
    let loaded = find_face(&font.face, font.weight.clamp(1, 1000) as u16, font.italic, font.pitch_and_family)?;
    let face = Face::parse(&loaded.data, loaded.index).ok()?;
    let metrics = Metrics::of(&face);
    let em_logical = if font.height < 0 {
        -font.height as f64
    } else if font.height > 0 {
        font.height as f64 * metrics.units / metrics.cell()
    } else {
        16.0 * metrics.units / metrics.cell()
    };
    let transform = &request.transform;
    let x_scale = transform.a.hypot(transform.b);
    let y_scale = transform.c.hypot(transform.d);
    // Text space: along the baseline, y down, from the reference point.
    let (em, spacing_scale, placement) = if request.advanced {
        (em_logical, 1.0, Affine::translate(request.reference.0, request.reference.1).then(transform))
    } else {
        let (x, y) = transform.apply(request.reference.0, request.reference.1);
        (em_logical * y_scale, x_scale, Affine::translate((x + 0.5).floor(), (y + 0.5).floor()))
    };
    let scale = em / metrics.units;
    let width_factor = if font.width != 0 && metrics.average_width > 0.0 {
        let wanted = font.width.unsigned_abs() as f64 * if request.advanced { 1.0 } else { x_scale };
        wanted / (metrics.average_width * scale)
    } else {
        1.0
    };
    let synthetic_italic = font.italic && !loaded.italic;

    let glyphs = glyph_ids(&face, request.codes, request.glyph_indices, font.charset == SYMBOL_CHARSET);
    let mut origins = Vec::with_capacity(glyphs.len());
    let mut pen = (0.0f64, 0.0f64);
    for (index, glyph) in glyphs.iter().enumerate() {
        origins.push(pen);
        let natural = face.glyph_hor_advance(*glyph).unwrap_or(0) as f64 * scale * width_factor;
        match request.advances.and_then(|advances| advances.get(index)) {
            Some((dx, dy)) => {
                pen.0 += dx * spacing_scale;
                pen.1 -= dy * spacing_scale;
            }
            None => pen.0 += natural,
        }
    }
    let total = pen.0;
    let (ascent, descent) = (metrics.ascent * scale, metrics.descent * scale);
    let shift_x = match request.align & TA_CENTER {
        TA_CENTER => -total / 2.0,
        TA_RIGHT => -total,
        _ => 0.0,
    };
    let shift_y = match request.align & TA_BASELINE {
        TA_BASELINE => 0.0,
        TA_BOTTOM => -descent,
        _ => ascent,
    };
    // The escapement turns the baseline in logical space (counter-clockwise
    // as `MM_TEXT` shows it); upright text keeps its glyphs unmirrored,
    // turned to wherever that baseline lands on the picture.
    let angle = (font.escapement as f64 / 10.0).to_radians();
    let angle = if request.advanced {
        angle
    } else {
        let (sin, cos) = angle.sin_cos();
        let (x, y) = transform.apply_vector(cos, -sin);
        if x == 0.0 && y == 0.0 { angle } else { (-y).atan2(x) }
    };
    let (sin, cos) = angle.sin_cos();
    let rotation = Affine { a: cos, b: -sin, c: sin, d: cos, e: 0.0, f: 0.0 };
    let text_to_picture = Affine::translate(shift_x, shift_y).then(&rotation).then(&placement);

    let mut builder = PathBuilder::new();
    for (glyph, origin) in glyphs.iter().zip(&origins) {
        let shear = if synthetic_italic { 0.2 } else { 0.0 };
        let glyph_to_text = Affine { a: scale * width_factor, b: 0.0, c: shear * scale * width_factor, d: -scale, e: origin.0, f: origin.1 };
        let mapping = glyph_to_text.then(&text_to_picture);
        outline_glyph(&face, *glyph, &mapping, &mut builder);
    }
    let decorations = [(font.underline, metrics.underline), (font.strike_out, metrics.strike_out)];
    for (wanted, (position, thickness)) in decorations {
        if wanted && total > 0.0 {
            let top = -(position + thickness / 2.0) * scale;
            let bottom = top + (thickness * scale).max(1.0 / text_to_picture.mean_scale().max(1e-9));
            add_rect(&mut builder, [0.0, top, total, bottom], &text_to_picture);
        }
    }
    let cell = {
        let mut cell = PathBuilder::new();
        add_rect(&mut cell, [0.0, -ascent, total, descent], &text_to_picture);
        cell.finish()
    };
    let advance_logical = if request.advanced || x_scale == 0.0 { total } else { total / x_scale };
    let next_position = match request.align & TA_CENTER {
        TA_CENTER => request.reference,
        TA_RIGHT => (request.reference.0 - advance_logical, request.reference.1),
        _ => (request.reference.0 + advance_logical, request.reference.1),
    };
    let embolden = (wants_bold && loaded.weight < 600).then(|| (em * text_to_picture.mean_scale() / 30.0) as f32);
    Some(LaidOut { outline: builder.finish(), cell, next_position, embolden })
}

/// How an EMF+ string sits in its layout rectangle.
#[derive(Clone, Debug)]
pub(super) struct StringLayout {
    /// `StringAlignment` across and down: near, centre, far.
    pub alignment: u32,
    pub line_alignment: u32,
    pub flags: u32,
    /// Space before and after each line, in ems.
    pub leading_margin: f64,
    pub trailing_margin: f64,
}

impl Default for StringLayout {
    /// The generic default format: a sixth of an em each side.
    fn default() -> StringLayout {
        StringLayout { alignment: 0, line_alignment: 0, flags: 0, leading_margin: 1.0 / 6.0, trailing_margin: 1.0 / 6.0 }
    }
}

/// `StringFormatFlagsNoWrap`.
const NO_WRAP: u32 = 0x1000;

/// An EMF+ font: family, style flags (bold 1, italic 2, underline 4,
/// strikeout 8) and size in world units.
pub(super) struct PlusFont<'a> {
    pub family: &'a str,
    pub style: i32,
    pub em: f64,
}

/// `DrawString`: lines broken at line feeds and (unless the format says
/// not to) wrapped to the layout rectangle, then aligned in it.
pub(super) fn lay_out_string(font: &PlusFont, text: &[u16], layout: [f64; 4], format: &StringLayout, transform: &Affine) -> Option<(Path, Option<f32>)> {
    let weight = if font.style & 1 != 0 { 700 } else { 400 };
    let loaded = find_face(font.family, weight, font.style & 2 != 0, 0)?;
    let face = Face::parse(&loaded.data, loaded.index).ok()?;
    let metrics = Metrics::of(&face);
    let scale = font.em / metrics.units;
    let line_height = metrics.line_spacing() * scale;
    let text = String::from_utf16_lossy(text);
    let margin_before = format.leading_margin * font.em;
    let margin_after = format.trailing_margin * font.em;
    let room = layout[2] - margin_before - margin_after;
    let wrap = layout[2] > 0.0 && format.flags & NO_WRAP == 0;

    let advance_of = |character: char| {
        let glyph = face.glyph_index(character).unwrap_or(GlyphId(0));
        face.glyph_hor_advance(glyph).unwrap_or(0) as f64 * scale
    };
    let width_of = |line: &str| line.chars().map(advance_of).sum::<f64>();
    let mut lines: Vec<String> = Vec::new();
    for paragraph in text.split('\n') {
        let paragraph = paragraph.trim_end_matches('\r');
        if !wrap {
            lines.push(paragraph.to_string());
            continue;
        }
        let mut line = String::new();
        for word in paragraph.split_inclusive(' ') {
            let candidate = format!("{line}{word}");
            if !line.is_empty() && width_of(candidate.trim_end()) > room {
                lines.push(std::mem::take(&mut line));
                line = word.to_string();
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }

    let total_height = line_height * lines.len() as f64;
    let top = match (format.line_alignment, layout[3] > 0.0) {
        (1, true) => layout[1] + (layout[3] - total_height) / 2.0,
        (2, true) => layout[1] + layout[3] - total_height,
        (1, false) => layout[1] - total_height / 2.0,
        (2, false) => layout[1] - total_height,
        _ => layout[1],
    };
    let synthetic_italic = font.style & 2 != 0 && !loaded.italic;
    let mut builder = PathBuilder::new();
    for (row, line) in lines.iter().enumerate() {
        let line = line.trim_end();
        let line_width = width_of(line);
        let left = match (format.alignment, layout[2] > 0.0) {
            (1, true) => layout[0] + margin_before + (room - line_width) / 2.0,
            (2, true) => layout[0] + layout[2] - margin_after - line_width,
            (1, false) => layout[0] - line_width / 2.0,
            (2, false) => layout[0] - line_width - margin_after,
            _ => layout[0] + margin_before,
        };
        let baseline = top + row as f64 * line_height + metrics.ascent * scale;
        let mut pen = left;
        for character in line.chars() {
            let glyph = face.glyph_index(character).unwrap_or(GlyphId(0));
            let shear = if synthetic_italic { 0.2 } else { 0.0 };
            let glyph_to_world = Affine { a: scale, b: 0.0, c: shear * scale, d: -scale, e: pen, f: baseline };
            outline_glyph(&face, glyph, &glyph_to_world.then(transform), &mut builder);
            pen += advance_of(character);
        }
        let decorations = [(font.style & 4 != 0, metrics.underline), (font.style & 8 != 0, metrics.strike_out)];
        for (wanted, (position, thickness)) in decorations {
            if wanted && line_width > 0.0 {
                let line_top = baseline - (position + thickness / 2.0) * scale;
                add_rect(&mut builder, [left, line_top, left + line_width, line_top + thickness * scale], transform);
            }
        }
    }
    let embolden = (font.style & 1 != 0 && loaded.weight < 600).then(|| (font.em * transform.mean_scale() / 30.0) as f32);
    builder.finish().map(|path| (path, embolden))
}

/// `DrawDriverString`: glyphs (or characters, `CmapLookup`) at their
/// origins, or from the first one by their advances
/// (`RealizedAdvance`), through an optional matrix.
pub(super) fn lay_out_glyphs(font: &PlusFont, glyphs: &[u16], positions: &[(f64, f64)], options: u32, matrix: Option<Affine>, transform: &Affine) -> Option<(Path, Option<f32>)> {
    let weight = if font.style & 1 != 0 { 700 } else { 400 };
    let loaded = find_face(font.family, weight, font.style & 2 != 0, 0)?;
    let face = Face::parse(&loaded.data, loaded.index).ok()?;
    let metrics = Metrics::of(&face);
    let scale = font.em / metrics.units;
    let ids: Vec<GlyphId> = if options & 1 != 0 {
        let codes: Vec<u32> = glyphs.iter().map(|unit| *unit as u32).collect();
        glyph_ids(&face, &codes, false, false)
    } else {
        glyphs.iter().map(|glyph| GlyphId(*glyph)).collect()
    };
    let to_picture = match matrix {
        Some(matrix) => matrix.then(transform),
        None => *transform,
    };
    let mut builder = PathBuilder::new();
    let mut pen = positions.first().copied().unwrap_or((0.0, 0.0));
    for (index, glyph) in ids.iter().enumerate() {
        let origin = if options & 4 != 0 { pen } else { positions.get(index).copied().unwrap_or(pen) };
        let glyph_to_world = Affine { a: scale, b: 0.0, c: 0.0, d: -scale, e: origin.0, f: origin.1 };
        outline_glyph(&face, *glyph, &glyph_to_world.then(&to_picture), &mut builder);
        pen = (origin.0 + face.glyph_hor_advance(*glyph).unwrap_or(0) as f64 * scale, origin.1);
    }
    let embolden = (font.style & 1 != 0 && loaded.weight < 600).then(|| (font.em * to_picture.mean_scale() / 30.0) as f32);
    builder.finish().map(|path| (path, embolden))
}

/// A face's metrics, in font units.
struct Metrics {
    units: f64,
    ascent: f64,
    descent: f64,
    line_gap: f64,
    average_width: f64,
    /// Position (above the baseline) and thickness.
    underline: (f64, f64),
    strike_out: (f64, f64),
}

impl Metrics {
    fn of(face: &Face) -> Metrics {
        let units = face.units_per_em().max(1) as f64;
        let os2 = face.tables().os2;
        let (ascent, descent) = match os2 {
            Some(table) if table.windows_ascender() > 0 => (table.windows_ascender() as f64, table.windows_descender().unsigned_abs() as f64),
            _ => (face.ascender() as f64, -(face.descender() as f64)),
        };
        let average_width = face.raw_face().table(Tag::from_bytes(b"OS/2")).map(|table| i16::from_be_bytes([table.get(2).copied().unwrap_or(0), table.get(3).copied().unwrap_or(0)]) as f64).unwrap_or(0.0);
        let underline = face.underline_metrics().map(|metrics| (metrics.position as f64, metrics.thickness as f64)).unwrap_or((-units / 10.0, units / 20.0));
        let strike_out = os2.map(|table| table.strikeout_metrics()).map(|metrics| (metrics.position as f64, metrics.thickness as f64)).unwrap_or((units / 4.0, units / 20.0));
        Metrics { units, ascent, descent, line_gap: face.line_gap() as f64, average_width, underline, strike_out }
    }

    /// A character cell's height (`tmHeight`).
    fn cell(&self) -> f64 {
        (self.ascent + self.descent).max(1.0)
    }

    fn line_spacing(&self) -> f64 {
        self.cell() + self.line_gap.max(0.0)
    }
}

/// The glyphs for character codes (UTF-16 units, surrogates paired), or
/// the codes as glyph indices; a symbol font's codes may sit at U+F0xx.
fn glyph_ids(face: &Face, codes: &[u32], indices: bool, symbol: bool) -> Vec<GlyphId> {
    if indices {
        return codes.iter().map(|code| GlyphId(*code as u16)).collect();
    }
    let units: Vec<u16> = codes.iter().map(|code| *code as u16).collect();
    let mut glyphs = Vec::with_capacity(codes.len());
    for decoded in char::decode_utf16(units.iter().copied()) {
        let character = decoded.unwrap_or(char::REPLACEMENT_CHARACTER);
        let mut glyph = face.glyph_index(character);
        if glyph.is_none() && (symbol || (character as u32) < 0x100) {
            glyph = char::from_u32(0xF000 + character as u32).and_then(|private| face.glyph_index(private));
        }
        glyphs.push(glyph.unwrap_or(GlyphId(0)));
    }
    glyphs
}

/// A glyph's outline through `mapping` (font units, y up, to the
/// picture).
fn outline_glyph(face: &Face, glyph: GlyphId, mapping: &Affine, builder: &mut PathBuilder) {
    let mut sink = Sink { builder, mapping };
    face.outline_glyph(glyph, &mut sink);
}

struct Sink<'a> {
    builder: &'a mut PathBuilder,
    mapping: &'a Affine,
}

impl Sink<'_> {
    fn map(&self, x: f32, y: f32) -> (f32, f32) {
        let (x, y) = self.mapping.apply(x as f64, y as f64);
        (x as f32, y as f32)
    }
}

impl OutlineBuilder for Sink<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.map(x, y);
        self.builder.move_to(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.map(x, y);
        self.builder.line_to(x, y);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x1, y1) = self.map(x1, y1);
        let (x, y) = self.map(x, y);
        self.builder.quad_to(x1, y1, x, y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x1, y1) = self.map(x1, y1);
        let (x2, y2) = self.map(x2, y2);
        let (x, y) = self.map(x, y);
        self.builder.cubic_to(x1, y1, x2, y2, x, y);
    }

    fn close(&mut self) {
        self.builder.close();
    }
}

fn add_rect(builder: &mut PathBuilder, rect: [f64; 4], mapping: &Affine) {
    let corners = [(rect[0], rect[1]), (rect[2], rect[1]), (rect[2], rect[3]), (rect[0], rect[3])];
    for (index, (x, y)) in corners.iter().enumerate() {
        let (x, y) = mapping.apply(*x, *y);
        if index == 0 {
            builder.move_to(x as f32, y as f32);
        } else {
            builder.line_to(x as f32, y as f32);
        }
    }
    builder.close();
}

// The system's fonts.

/// A face in the system's fonts.
#[derive(Clone, Debug)]
struct FontEntry {
    path: PathBuf,
    index: u32,
    /// Its family, typographic family and full names, lower case.
    names: Vec<String>,
    weight: u16,
    italic: bool,
}

/// A face's file loaded.
pub(super) struct LoadedFace {
    pub data: Arc<Vec<u8>>,
    pub index: u32,
    pub weight: u16,
    pub italic: bool,
}

/// Stand-ins for the Windows fonts metafiles name most.
const SUBSTITUTES: [(&str, &[&str]); 12] = [
    ("arial", &["liberation sans", "arimo", "helvetica", "helvetica neue"]),
    ("helvetica", &["arial", "liberation sans", "arimo"]),
    ("times new roman", &["liberation serif", "tinos", "times", "times roman"]),
    ("times", &["times new roman", "liberation serif", "tinos"]),
    ("courier new", &["liberation mono", "cousine", "courier", "menlo"]),
    ("courier", &["courier new", "liberation mono", "cousine"]),
    ("calibri", &["carlito", "arial", "liberation sans", "helvetica"]),
    ("cambria", &["caladea", "times new roman", "liberation serif", "times"]),
    ("segoe ui", &["arial", "liberation sans", "helvetica", "dejavu sans"]),
    ("tahoma", &["verdana", "dejavu sans", "arial", "liberation sans"]),
    ("ms sans serif", &["microsoft sans serif", "arial", "liberation sans", "helvetica"]),
    ("ms shell dlg", &["microsoft sans serif", "tahoma", "arial", "liberation sans"]),
];

const SANS: [&str; 6] = ["arial", "liberation sans", "arimo", "helvetica", "dejavu sans", "noto sans"];
const SERIF: [&str; 5] = ["times new roman", "liberation serif", "tinos", "times", "dejavu serif"];
const MONO: [&str; 6] = ["courier new", "liberation mono", "cousine", "courier", "menlo", "dejavu sans mono"];

/// `FF_ROMAN`, `FF_MODERN` (the family in a `LOGFONT`'s pitch and family
/// byte).
const FF_ROMAN: u8 = 0x10;
const FF_MODERN: u8 = 0x30;

/// The face to draw `family` with: the family if installed, else a stand-in,
/// else one by pitch and family, else any; the closest weight and slant.
pub(super) fn find_face(family: &str, weight: u16, italic: bool, pitch_and_family: u8) -> Option<LoadedFace> {
    let fonts = system_fonts();
    let wanted = family.trim().to_lowercase();
    let in_family = |name: &str| fonts.iter().filter(|entry| entry.names.iter().any(|entry_name| entry_name == name)).collect::<Vec<_>>();
    let mut candidates = in_family(&wanted);
    if candidates.is_empty() {
        let substitutes = SUBSTITUTES.iter().find(|(name, _)| *name == wanted).map(|(_, list)| *list).unwrap_or(&[]);
        let by_family: &[&str] = match pitch_and_family & 0xF0 {
            FF_ROMAN => &SERIF,
            FF_MODERN => &MONO,
            _ => &SANS,
        };
        candidates = substitutes.iter().chain(by_family).map(|name| in_family(name)).find(|found| !found.is_empty()).unwrap_or_default();
    }
    if candidates.is_empty() {
        candidates = fonts.iter().take(1).collect();
    }
    let best = candidates.into_iter().min_by_key(|entry| (entry.italic != italic) as u32 * 1000 + (entry.weight as i32 - weight as i32).unsigned_abs())?;
    let data = load_font(&best.path)?;
    Some(LoadedFace { data, index: best.index, weight: best.weight, italic: best.italic })
}

fn load_font(path: &FilePath) -> Option<Arc<Vec<u8>>> {
    static LOADED: OnceLock<Mutex<HashMap<PathBuf, Arc<Vec<u8>>>>> = OnceLock::new();
    let loaded = LOADED.get_or_init(Default::default);
    let mut loaded = loaded.lock().ok()?;
    if let Some(data) = loaded.get(path) {
        return Some(data.clone());
    }
    let data = Arc::new(std::fs::read(path).ok()?);
    loaded.insert(path.to_path_buf(), data.clone());
    Some(data)
}

/// The system's fonts, indexed once.
fn system_fonts() -> &'static [FontEntry] {
    static FONTS: OnceLock<Vec<FontEntry>> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut entries = Vec::new();
        for folder in font_folders() {
            index_folder(&folder, &mut entries, 0);
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path).then(a.index.cmp(&b.index)));
        entries
    })
}

fn font_folders() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut folders = Vec::new();
    if cfg!(target_os = "windows") {
        let windows = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        folders.push(windows.join("Fonts"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            folders.push(PathBuf::from(local).join("Microsoft").join("Windows").join("Fonts"));
        }
    } else if cfg!(target_os = "macos") {
        folders.push(PathBuf::from("/System/Library/Fonts"));
        folders.push(PathBuf::from("/Library/Fonts"));
        if let Some(home) = &home {
            folders.push(home.join("Library").join("Fonts"));
        }
    } else {
        folders.push(PathBuf::from("/usr/share/fonts"));
        folders.push(PathBuf::from("/usr/local/share/fonts"));
        if let Some(home) = &home {
            folders.push(home.join(".local").join("share").join("fonts"));
            folders.push(home.join(".fonts"));
        }
    }
    folders
}

fn index_folder(folder: &FilePath, entries: &mut Vec<FontEntry>, depth: u32) {
    let Ok(listing) = std::fs::read_dir(folder) else {
        return;
    };
    for item in listing.flatten() {
        let path = item.path();
        if path.is_dir() {
            if depth < 4 {
                index_folder(&path, entries, depth + 1);
            }
            continue;
        }
        let extension = path.extension().and_then(|value| value.to_str()).unwrap_or_default().to_ascii_lowercase();
        if matches!(extension.as_str(), "ttf" | "otf" | "ttc" | "otc") {
            index_file(&path, entries);
        }
    }
}

/// Reads a font file's faces' names, weights and slants from their `name`
/// and `OS/2` tables only.
fn index_file(path: &FilePath, entries: &mut Vec<FontEntry>) {
    let Ok(mut file) = std::fs::File::open(path) else {
        return;
    };
    let mut read_at = |offset: u64, length: usize| -> Option<Vec<u8>> {
        file.seek(SeekFrom::Start(offset)).ok()?;
        let mut buffer = vec![0u8; length];
        file.read_exact(&mut buffer).ok()?;
        Some(buffer)
    };
    let Some(head) = read_at(0, 12) else {
        return;
    };
    let offsets: Vec<u64> = if &head[0..4] == b"ttcf" {
        let count = u32::from_be_bytes([head[8], head[9], head[10], head[11]]).min(256) as usize;
        let Some(table) = read_at(12, 4 * count) else {
            return;
        };
        table.as_chunks::<4>().0.iter().map(|offset| u32::from_be_bytes(*offset) as u64).collect()
    } else {
        vec![0]
    };
    for (index, offset) in offsets.iter().enumerate() {
        let Some(directory) = read_at(*offset, 12) else {
            continue;
        };
        let tables = u16::from_be_bytes([directory[4], directory[5]]) as usize;
        let Some(records) = read_at(offset + 12, 16 * tables) else {
            continue;
        };
        let find = |tag: &[u8; 4]| {
            records.as_chunks::<16>().0.iter().find(|record| &record[0..4] == tag).map(|record| {
                let start = u32::from_be_bytes([record[8], record[9], record[10], record[11]]) as u64;
                let length = u32::from_be_bytes([record[12], record[13], record[14], record[15]]) as usize;
                (start, length)
            })
        };
        let Some((name_at, name_length)) = find(b"name") else {
            continue;
        };
        let Some(name_data) = read_at(name_at, name_length.min(1 << 20)) else {
            continue;
        };
        let Some(names) = ttf_parser::name::Table::parse(&name_data) else {
            continue;
        };
        let mut found: Vec<String> = Vec::new();
        for name in names.names {
            let wanted = matches!(name.name_id, ttf_parser::name_id::FAMILY | ttf_parser::name_id::TYPOGRAPHIC_FAMILY | ttf_parser::name_id::FULL_NAME);
            if !wanted {
                continue;
            }
            if let Some(text) = name.to_string() {
                let text = text.to_lowercase();
                if !found.contains(&text) {
                    found.push(text);
                }
            }
        }
        if found.is_empty() {
            continue;
        }
        let os2 = find(b"OS/2").and_then(|(at, length)| read_at(at, length.min(1 << 16)));
        let (weight, italic) = match os2.as_deref().and_then(ttf_parser::os2::Table::parse) {
            Some(table) => (table.weight().to_number(), table.style() != ttf_parser::Style::Normal),
            None => (400, false),
        };
        entries.push(FontEntry { path: path.to_path_buf(), index: index as u32, names: found, weight, italic });
    }
}
