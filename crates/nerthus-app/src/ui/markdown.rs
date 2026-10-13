//! Markdown as the help windows draw it: the shortcut guide, the change
//! log, update notes and the documentation. CommonMark with tables,
//! strikethrough and GitHub's alerts (parsed by pulldown-cmark): headings,
//! paragraphs, bulleted and numbered lists (nested), quotes and alerts,
//! code blocks, tables and rules, and inside them bold, italics,
//! strikethrough, code and links. Pictures and HTML are left out, but for
//! `<br>`.

use std::iter::Peekable;
use std::ops::Range;

use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Color32, CornerRadius, FontId, Galley, Margin, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use pulldown_cmark::{Alignment, BlockQuoteKind, Event, HeadingLevel, Parser, Tag, TagEnd};
use std::sync::Arc;

use crate::theme;

/// Body text's size.
const TEXT_SIZE: f32 = 13.0;
/// Space after a paragraph, and after one inside a list item.
const PARAGRAPH_GAP: f32 = 10.0;
const ITEM_PARAGRAPH_GAP: f32 = 2.0;
/// A list's indent, its markers' column and the gap after them.
const LIST_INDENT: f32 = 12.0;
const BULLET_WIDTH: f32 = 10.0;
const NUMBER_WIDTH: f32 = 22.0;
const MARKER_GAP: f32 = 4.0;
/// How far code's plate reaches past its text.
const CODE_PADDING: Vec2 = Vec2::new(3.0, 1.0);

/// A parsed text, drawn as often as it shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    blocks: Vec<Block>,
}

/// A block of the text.
#[derive(Clone, Debug, PartialEq)]
enum Block {
    Heading { level: usize, anchor: String, spans: Vec<Span> },
    Paragraph(Vec<Span>),
    /// Numbered from `start`, else bulleted.
    List { start: Option<u64>, items: Vec<Vec<Block>> },
    Quote { alert: Option<BlockQuoteKind>, blocks: Vec<Block> },
    Code(String),
    Table { alignments: Vec<Alignment>, header: Vec<Vec<Span>>, rows: Vec<Vec<Vec<Span>>> },
    Rule,
}

/// A run of text in one style.
#[derive(Clone, Debug, Default, PartialEq)]
struct Span {
    text: String,
    strong: bool,
    emphasis: bool,
    strikethrough: bool,
    code: bool,
    link: Option<String>,
}

/// What drawing a text did.
#[derive(Debug, Default)]
pub struct Drawn {
    /// The destination of the link clicked.
    pub clicked: Option<String>,
    /// The heading scrolled to.
    pub scrolled: bool,
}

/// How a text is drawn: the heading to scroll to.
#[derive(Clone, Copy, Debug, Default)]
pub struct View<'a> {
    pub scroll_to: Option<&'a str>,
}

/// Parses `text`.
pub fn parse(text: &str) -> Document {
    let parser = Parser::new_ext(text, crate::docs::markdown_options());
    let mut events = parser.peekable();
    let blocks = parse_blocks(&mut events, None);
    Document { blocks }
}

/// Draws `text`, opening the links clicked in the browser.
pub fn show(ui: &mut Ui, text: &str) {
    let document = parse(text);
    let drawn = draw(ui, &document, View::default());
    if let Some(link) = drawn.clicked {
        crate::export_build::open_url(&link);
    }
}

/// Draws `document`, leaving the links clicked to the caller.
pub fn draw(ui: &mut Ui, document: &Document, view: View) -> Drawn {
    let mut drawn = Drawn::default();
    let mut context = DrawContext { view, drawn: &mut drawn, in_item: false, depth: 0 };
    draw_blocks(ui, &document.blocks, &mut context);
    drawn
}

type Events<'a> = Peekable<Parser<'a>>;

fn heading_number(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Whether an event belongs inside a paragraph.
fn is_inline(event: &Event) -> bool {
    match event {
        Event::Text(_) | Event::Code(_) | Event::SoftBreak | Event::HardBreak | Event::InlineHtml(_) => true,
        Event::Start(tag) => matches!(tag, Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. } | Tag::Image { .. }),
        Event::End(tag) => matches!(tag, TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link | TagEnd::Image),
        _ => false,
    }
}

/// Blocks until `end` (or the text's end).
fn parse_blocks(events: &mut Events, end: Option<TagEnd>) -> Vec<Block> {
    let mut blocks = Vec::new();
    while let Some(event) = events.peek() {
        // A tight list's items hold their text without paragraphs.
        if is_inline(event) {
            let spans = parse_inlines(events, None);
            if !spans.is_empty() {
                blocks.push(Block::Paragraph(spans));
            }
            continue;
        }
        let Some(event) = events.next() else {
            break;
        };
        match event {
            Event::End(tag) if Some(tag) == end => return blocks,
            Event::Start(Tag::Paragraph) => {
                let spans = parse_inlines(events, Some(TagEnd::Paragraph));
                blocks.push(Block::Paragraph(spans));
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let spans = parse_inlines(events, Some(TagEnd::Heading(level)));
                let anchor = crate::docs::slug(&plain(&spans));
                blocks.push(Block::Heading { level: heading_number(level), anchor, spans });
            }
            Event::Start(Tag::List(start)) => {
                let mut items = Vec::new();
                while let Some(event) = events.next() {
                    match event {
                        Event::Start(Tag::Item) => items.push(parse_blocks(events, Some(TagEnd::Item))),
                        Event::End(TagEnd::List(_)) => break,
                        _ => {}
                    }
                }
                blocks.push(Block::List { start, items });
            }
            Event::Start(Tag::BlockQuote(alert)) => {
                let inner = parse_blocks(events, Some(TagEnd::BlockQuote(alert)));
                blocks.push(Block::Quote { alert, blocks: inner });
            }
            Event::Start(Tag::CodeBlock(_)) => {
                let mut code = String::new();
                for event in events.by_ref() {
                    match event {
                        Event::Text(text) => code.push_str(&text),
                        Event::End(TagEnd::CodeBlock) => break,
                        _ => {}
                    }
                }
                let trimmed = code.strip_suffix('\n').unwrap_or(&code).to_string();
                blocks.push(Block::Code(trimmed));
            }
            Event::Start(Tag::Table(alignments)) => blocks.push(parse_table(events, alignments)),
            Event::Start(Tag::HtmlBlock) => skip_until(events, TagEnd::HtmlBlock),
            Event::Rule => blocks.push(Block::Rule),
            _ => {}
        }
    }
    blocks
}

fn skip_until(events: &mut Events, end: TagEnd) {
    for event in events.by_ref() {
        if event == Event::End(end) {
            break;
        }
    }
}

/// A table's header and rows.
fn parse_table(events: &mut Events, alignments: Vec<Alignment>) -> Block {
    let mut header = Vec::new();
    let mut rows = Vec::new();
    let mut row = Vec::new();
    while let Some(event) = events.next() {
        match event {
            Event::Start(Tag::TableCell) => row.push(parse_inlines(events, Some(TagEnd::TableCell))),
            Event::End(TagEnd::TableHead) => header = std::mem::take(&mut row),
            Event::End(TagEnd::TableRow) => rows.push(std::mem::take(&mut row)),
            Event::End(TagEnd::Table) => break,
            _ => {}
        }
    }
    Block::Table { alignments, header, rows }
}

/// Text until `end`; with none, until the next event that is not part of
/// a paragraph.
fn parse_inlines(events: &mut Events, end: Option<TagEnd>) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut strong = 0;
    let mut emphasis = 0;
    let mut strikethrough = 0;
    let mut links: Vec<String> = Vec::new();
    let mut images = 0;
    loop {
        let open = strong + emphasis + strikethrough + links.len() + images;
        if end.is_none() && open == 0 {
            match events.peek() {
                Some(event) if is_inline(event) => {}
                _ => break,
            }
        }
        let Some(event) = events.next() else {
            break;
        };
        match event {
            Event::End(tag) if Some(tag) == end => break,
            Event::Start(Tag::Strong) => strong += 1,
            Event::End(TagEnd::Strong) => strong -= 1,
            Event::Start(Tag::Emphasis) => emphasis += 1,
            Event::End(TagEnd::Emphasis) => emphasis -= 1,
            Event::Start(Tag::Strikethrough) => strikethrough += 1,
            Event::End(TagEnd::Strikethrough) => strikethrough -= 1,
            Event::Start(Tag::Link { dest_url, .. }) => links.push(dest_url.to_string()),
            Event::End(TagEnd::Link) => {
                links.pop();
            }
            Event::Start(Tag::Image { .. }) => images += 1,
            Event::End(TagEnd::Image) => images -= 1,
            Event::Text(text) if images == 0 => {
                let span = Span {
                    text: text.to_string(),
                    strong: strong > 0,
                    emphasis: emphasis > 0,
                    strikethrough: strikethrough > 0,
                    code: false,
                    link: links.last().cloned(),
                };
                spans.push(span);
            }
            Event::Code(text) if images == 0 => {
                let span = Span { text: text.to_string(), code: true, link: links.last().cloned(), ..Span::default() };
                spans.push(span);
            }
            Event::SoftBreak => spans.push(Span { text: " ".to_string(), ..Span::default() }),
            Event::HardBreak => spans.push(Span { text: "\n".to_string(), ..Span::default() }),
            Event::InlineHtml(html) if is_line_break(&html) => spans.push(Span { text: "\n".to_string(), ..Span::default() }),
            _ => {}
        }
    }
    spans
}

fn is_line_break(html: &str) -> bool {
    let tag: String = html.chars().filter(|character| !character.is_whitespace()).collect::<String>().to_lowercase();
    matches!(tag.as_str(), "<br>" | "<br/>")
}

/// The spans' text, without their styles.
fn plain(spans: &[Span]) -> String {
    let mut text = String::new();
    for span in spans {
        text.push_str(&span.text);
    }
    text
}

/// Where drawing is: the view, what has happened, and whether inside a
/// list item (tighter paragraphs) how many lists deep.
struct DrawContext<'a, 'b> {
    view: View<'a>,
    drawn: &'b mut Drawn,
    in_item: bool,
    depth: usize,
}

fn draw_blocks(ui: &mut Ui, blocks: &[Block], context: &mut DrawContext) {
    for block in blocks {
        match block {
            Block::Heading { level, anchor, spans } => heading(ui, *level, anchor, spans, context),
            Block::Paragraph(spans) => {
                let style = TextStyle { size: TEXT_SIZE, color: theme::LABEL_DEFAULT, bold: false };
                paragraph(ui, spans, style, context);
                let gap = if context.in_item { ITEM_PARAGRAPH_GAP } else { PARAGRAPH_GAP };
                ui.add_space(gap);
            }
            Block::List { start, items } => list(ui, *start, items, context),
            Block::Quote { alert, blocks } => quote(ui, *alert, blocks, context),
            Block::Code(code) => code_block(ui, code),
            Block::Table { alignments, header, rows } => {
                table(ui, alignments, header, rows, context);
                ui.add_space(12.0);
            }
            Block::Rule => {
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
            }
        }
    }
}

/// How a run of text is drawn by default.
#[derive(Clone, Copy)]
struct TextStyle {
    size: f32,
    color: Color32,
    bold: bool,
}

fn heading(ui: &mut Ui, level: usize, anchor: &str, spans: &[Span], context: &mut DrawContext) {
    let (size, bold, color, above, below) = match level {
        1 => (26.0, false, theme::LABEL_EMPHASIS, 12.0, 10.0),
        2 => (21.0, false, theme::LABEL_EMPHASIS, 14.0, 8.0),
        3 => (16.0, true, theme::LABEL_EMPHASIS, 10.0, 6.0),
        _ => (14.0, true, theme::ACCENT_PRIMARY, 10.0, 6.0),
    };
    ui.add_space(above);
    let style = TextStyle { size, color, bold };
    let rect = paragraph(ui, spans, style, context);
    ui.add_space(below);
    if context.view.scroll_to == Some(anchor) {
        ui.scroll_to_rect(rect.expand(above), Some(Align::TOP));
        context.drawn.scrolled = true;
    }
}

/// What part of a laid-out text is code, and what part links where.
#[derive(Default)]
struct Marks {
    code: Vec<Range<usize>>,
    links: Vec<(Range<usize>, String)>,
}

/// The spans as one wrapped text, with the character ranges of their code
/// and links.
fn layout_job(spans: &[Span], style: TextStyle, width: f32) -> (LayoutJob, Marks) {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    let mut marks = Marks::default();
    let mut characters = 0;
    for span in spans {
        let font: FontId = if span.code {
            theme::monospace(style.size - 1.0)
        } else if span.strong || style.bold {
            theme::semibold(style.size)
        } else {
            theme::interface(style.size)
        };
        let color = if span.link.is_some() {
            theme::ACCENT_MUTED_GREEN
        } else if span.code {
            theme::ACCENT_PRIMARY
        } else if span.strong {
            theme::LABEL_EMPHASIS
        } else {
            style.color
        };
        let mut format = TextFormat::simple(font, color);
        format.italics = span.emphasis;
        if span.strikethrough {
            format.strikethrough = Stroke::new(1.0, color);
        }
        let count = span.text.chars().count();
        let range = characters..characters + count;
        if span.code {
            marks.code.push(range.clone());
        }
        if let Some(link) = &span.link {
            marks.links.push((range, link.clone()));
        }
        characters += count;
        job.append(&span.text, 0.0, format);
    }
    (job, marks)
}

/// Each glyph's character index and rectangle in the galley.
fn glyph_rects(galley: &Galley) -> Vec<(usize, Rect)> {
    let mut rects = Vec::new();
    let mut character = 0;
    for placed in &galley.rows {
        for glyph in &placed.row.glyphs {
            let left = placed.pos.x + glyph.pos.x;
            let rect = Rect::from_min_max(Pos2::new(left, placed.pos.y), Pos2::new(left + glyph.advance_width, placed.pos.y + placed.row.size.y));
            rects.push((character, rect));
            character += 1;
        }
        if placed.ends_with_newline {
            character += 1;
        }
    }
    rects
}

/// The code's plates: per row, the span of its glyphs.
fn code_plates(glyphs: &[(usize, Rect)], code: &[Range<usize>]) -> Vec<Rect> {
    let mut plates: Vec<Rect> = Vec::new();
    let mut current: Option<(usize, Rect)> = None;
    for (range_index, range) in code.iter().enumerate() {
        for (character, rect) in glyphs {
            if !range.contains(character) {
                continue;
            }
            current = match current {
                Some((index, plate)) if index == range_index && (plate.center().y - rect.center().y).abs() < 1.0 => Some((index, plate.union(*rect))),
                Some((_, plate)) => {
                    plates.push(plate);
                    Some((range_index, *rect))
                }
                None => Some((range_index, *rect)),
            };
        }
    }
    if let Some((_, plate)) = current {
        plates.push(plate);
    }
    plates
}

/// A wrapped text at the full width: its code on plates and its links
/// live. Returns where it was drawn.
fn paragraph(ui: &mut Ui, spans: &[Span], style: TextStyle, context: &mut DrawContext) -> Rect {
    let width = ui.available_width().max(40.0);
    let (job, marks) = layout_job(spans, style, width);
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    let sense = if marks.links.is_empty() { Sense::hover() } else { Sense::click() };
    let (rect, response) = ui.allocate_exact_size(galley.size(), sense);
    paint_text(ui, rect.min, &galley, &marks, style.color);
    let glyphs = glyph_rects(&galley);
    if let Some(pointer) = response.hover_pos() {
        let local = pointer - rect.min.to_vec2();
        let hovered = glyphs.iter().find(|(_, glyph)| glyph.contains(local)).map(|(character, _)| *character);
        let link = hovered.and_then(|character| marks.links.iter().find(|(range, _)| range.contains(&character)));
        if let Some((_, link)) = link {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if link.contains("://") {
                response.clone().on_hover_text(link.as_str());
            }
            if response.clicked() {
                context.drawn.clicked = Some(link.clone());
            }
        }
    }
    rect
}

/// The galley with its code's plates behind it.
fn paint_text(ui: &Ui, origin: Pos2, galley: &Arc<Galley>, marks: &Marks, color: Color32) {
    if !marks.code.is_empty() {
        let glyphs = glyph_rects(galley);
        for plate in code_plates(&glyphs, &marks.code) {
            let plate = plate.translate(origin.to_vec2()).expand2(CODE_PADDING);
            ui.painter().rect_filled(plate, 3.0, theme::TEXTBOX_BACKGROUND);
            ui.painter().rect_stroke(plate, 3.0, Stroke::new(1.0, theme::BUTTON_BORDER), egui::StrokeKind::Inside);
        }
    }
    ui.painter().galley(origin, galley.clone(), color);
}

fn list(ui: &mut Ui, start: Option<u64>, items: &[Vec<Block>], context: &mut DrawContext) {
    let marker_width = if start.is_some() { NUMBER_WIDTH } else { BULLET_WIDTH };
    let line_height = ui.fonts_mut(|fonts| fonts.row_height(&theme::interface(TEXT_SIZE)));
    for (index, item) in items.iter().enumerate() {
        let marker = match start {
            Some(first) => format!("{}.", first + index as u64),
            None if context.depth == 0 => "•".to_string(),
            None => "–".to_string(),
        };
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(LIST_INDENT);
            let (marker_rect, _) = ui.allocate_exact_size(Vec2::new(marker_width, line_height), Sense::hover());
            let galley = ui.painter().layout_no_wrap(marker, theme::interface(TEXT_SIZE), theme::LABEL_DEFAULT);
            let left = if start.is_some() { marker_rect.right() - galley.size().x } else { marker_rect.left() };
            ui.painter().galley(Pos2::new(left, marker_rect.top()), galley, theme::LABEL_DEFAULT);
            ui.add_space(MARKER_GAP);
            ui.vertical(|ui| {
                let was_in_item = context.in_item;
                context.in_item = true;
                context.depth += 1;
                draw_blocks(ui, item, context);
                context.depth -= 1;
                context.in_item = was_in_item;
            });
        });
        ui.add_space(4.0);
    }
    if !context.in_item {
        ui.add_space(PARAGRAPH_GAP - 4.0);
    }
}

/// A quote, or an alert in its colours with its title.
fn quote(ui: &mut Ui, alert: Option<BlockQuoteKind>, blocks: &[Block], context: &mut DrawContext) {
    let (title, accent, fill) = match alert {
        Some(BlockQuoteKind::Note) => (Some("Note"), theme::ACCENT_STRONG_BLUE, theme::ACCENT_TRANSPARENT_BLUE),
        Some(BlockQuoteKind::Tip) => (Some("Tip"), theme::ACCENT_STRONG_GREEN, theme::ACCENT_TRANSPARENT_GREEN),
        Some(BlockQuoteKind::Important) => (Some("Important"), theme::ACCENT_PURPLE, theme::ACCENT_TRANSPARENT_PURPLE),
        Some(BlockQuoteKind::Warning) => (Some("Warning"), theme::ACCENT_STRONG_ORANGE, theme::ACCENT_TRANSPARENT_ORANGE),
        Some(BlockQuoteKind::Caution) => (Some("Caution"), theme::ACCENT_STRONG_RED, theme::ACCENT_TRANSPARENT_RED),
        None => (None, theme::MENU_SEPARATOR, theme::PANEL_CLADDING),
    };
    let mut frame = egui::Frame::new();
    frame = frame.fill(fill.gamma_multiply(0.45));
    frame = frame.corner_radius(CornerRadius::same(4));
    frame = frame.inner_margin(Margin { left: 14, right: 10, top: 8, bottom: 2 });
    let shown = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        if let Some(title) = title {
            let label = egui::RichText::new(title).font(theme::semibold(TEXT_SIZE)).color(theme::LABEL_EMPHASIS);
            ui.label(label);
            ui.add_space(4.0);
        }
        draw_blocks(ui, blocks, context);
    });
    let rect = shown.response.rect;
    let bar = Rect::from_min_max(rect.min, Pos2::new(rect.left() + 3.0, rect.bottom()));
    ui.painter().rect_filled(bar, CornerRadius { nw: 4, sw: 4, ne: 0, se: 0 }, accent);
    ui.add_space(PARAGRAPH_GAP);
}

/// Code on a dark plate, selectable, scrolling sideways when long.
fn code_block(ui: &mut Ui, code: &str) {
    let mut frame = egui::Frame::new();
    frame = frame.fill(theme::TEXTBOX_BACKGROUND);
    frame = frame.stroke(Stroke::new(1.0, theme::BUTTON_BORDER));
    frame = frame.corner_radius(CornerRadius::same(3));
    frame = frame.inner_margin(Margin::same(8));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        let mut scroll = egui::ScrollArea::horizontal();
        scroll = scroll.id_salt(("code-block", code.len(), code.get(..code.len().min(32))));
        scroll.show(ui, |ui| {
            let mut text = code;
            let mut field = egui::TextEdit::multiline(&mut text);
            field = field.font(theme::monospace(TEXT_SIZE - 1.0));
            field = field.text_color(theme::ACCENT_PRIMARY);
            field = field.frame(egui::Frame::NONE);
            field = field.desired_rows(1);
            field = field.desired_width(f32::INFINITY);
            ui.add(field);
        });
    });
    ui.add_space(PARAGRAPH_GAP);
}

/// The columns' widths: each its text's width when all fit, else at least
/// its longest word's and the rest shared out by what each would like.
fn column_widths(ui: &Ui, header: &[Vec<Span>], rows: &[Vec<Vec<Span>>], width: f32, padding: f32) -> Vec<f32> {
    let columns = header.len().max(rows.iter().map(Vec::len).max().unwrap_or(0)).max(1);
    let mut natural = vec![0.0f32; columns];
    let mut narrowest = vec![0.0f32; columns];
    let mut measure = |cells: &[Vec<Span>], bold: bool| {
        for (column, cell) in cells.iter().enumerate().take(columns) {
            let style = TextStyle { size: TEXT_SIZE, color: theme::LABEL_DEFAULT, bold };
            let (job, _) = layout_job(cell, style, f32::INFINITY);
            let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
            natural[column] = natural[column].max(galley.size().x + padding);
            for word in plain(cell).split_whitespace() {
                let word_galley = ui.painter().layout_no_wrap(word.to_string(), theme::interface(TEXT_SIZE), Color32::WHITE);
                narrowest[column] = narrowest[column].max(word_galley.size().x + padding);
            }
        }
    };
    measure(header, true);
    for row in rows {
        measure(row, false);
    }
    let wanted: f32 = natural.iter().sum();
    if wanted <= width {
        // The table spans the width: the room left goes to each column by
        // its share.
        let extra = width - wanted;
        return natural.iter().map(|natural| natural + extra * natural / wanted.max(1.0)).collect();
    }
    let least: f32 = narrowest.iter().sum();
    let room = (width - least).max(0.0);
    let flexible: f32 = natural.iter().zip(&narrowest).map(|(natural, narrowest)| (natural - narrowest).max(0.0)).sum();
    let mut widths = Vec::new();
    for (natural, narrowest) in natural.iter().zip(&narrowest) {
        let share = if flexible > 0.0 { (natural - narrowest).max(0.0) / flexible } else { 1.0 / columns as f32 };
        widths.push(narrowest + room * share);
    }
    widths
}

/// A table: the header in semibold over a line, every other row shaded,
/// cells wrapping within their columns.
fn table(ui: &mut Ui, alignments: &[Alignment], header: &[Vec<Span>], rows: &[Vec<Vec<Span>>], context: &mut DrawContext) {
    let padding = 8.0;
    let width = ui.available_width();
    let widths = column_widths(ui, header, rows, width, padding * 2.0);
    let total: f32 = widths.iter().sum();
    let mut draw_row = |ui: &mut Ui, cells: &[Vec<Span>], is_header: bool, shaded: bool| {
        let mut galleys = Vec::new();
        let mut height: f32 = 26.0;
        for (column, cell_width) in widths.iter().enumerate() {
            let spans = cells.get(column).map(Vec::as_slice).unwrap_or_default();
            let style = TextStyle { size: TEXT_SIZE, color: theme::LABEL_DEFAULT, bold: is_header };
            let (job, marks) = layout_job(spans, style, cell_width - padding * 2.0);
            let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
            height = height.max(galley.size().y + 10.0);
            galleys.push((galley, marks));
        }
        let (rect, response) = ui.allocate_exact_size(Vec2::new(total, height), Sense::click());
        if shaded {
            ui.painter().rect_filled(rect, 0.0, theme::PANEL_CLADDING);
        }
        if is_header {
            let bottom = rect.bottom() - 0.5;
            ui.painter().hline(rect.x_range(), bottom, Stroke::new(1.0, theme::MENU_SEPARATOR));
        }
        let mut left = rect.left();
        for (column, (galley, marks)) in galleys.iter().enumerate() {
            let cell_width = widths[column];
            let alignment = alignments.get(column).copied().unwrap_or(Alignment::None);
            let text_left = match alignment {
                Alignment::Right => left + cell_width - padding - galley.size().x,
                Alignment::Center => left + (cell_width - galley.size().x) / 2.0,
                _ => left + padding,
            };
            let origin = Pos2::new(text_left, rect.center().y - galley.size().y / 2.0);
            paint_text(ui, origin, galley, marks, theme::LABEL_DEFAULT);
            if let Some(pointer) = response.hover_pos() {
                let local = pointer - origin.to_vec2();
                let glyphs = glyph_rects(galley);
                let hovered = glyphs.iter().find(|(_, glyph)| glyph.contains(local)).map(|(character, _)| *character);
                let link = hovered.and_then(|character| marks.links.iter().find(|(range, _)| range.contains(&character)));
                if let Some((_, link)) = link {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    if response.clicked() {
                        context.drawn.clicked = Some(link.clone());
                    }
                }
            }
            left += cell_width;
        }
    };
    if !header.is_empty() {
        draw_row(ui, header, true, false);
    }
    for (index, row) in rows.iter().enumerate() {
        draw_row(ui, row, false, index % 2 == 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(text: &str) -> Span {
        Span { text: text.to_string(), ..Span::default() }
    }

    #[test]
    fn inline_styles_are_found() {
        let document = parse("Press `Ctrl + N` or **bold** and *it* [here](https://example.com).");
        let Block::Paragraph(spans) = &document.blocks[0] else {
            panic!("not a paragraph: {document:?}");
        };
        assert_eq!(spans[0], text("Press "));
        assert_eq!(spans[1], Span { text: "Ctrl + N".to_string(), code: true, ..Span::default() });
        assert_eq!(spans[3], Span { text: "bold".to_string(), strong: true, ..Span::default() });
        assert_eq!(spans[5], Span { text: "it".to_string(), emphasis: true, ..Span::default() });
        assert_eq!(spans[7], Span { text: "here".to_string(), link: Some("https://example.com".to_string()), ..Span::default() });
    }

    #[test]
    fn double_backticks_hold_a_backtick() {
        let document = parse("`` ` ``");
        assert_eq!(document.blocks, vec![Block::Paragraph(vec![Span { text: "`".to_string(), code: true, ..Span::default() }])]);
    }

    #[test]
    fn tables_keep_their_alignment() {
        let document = parse("| Command | Shortcut |\n| :--- | ---: |\n| New | `Ctrl + N` |");
        let Block::Table { alignments, header, rows } = &document.blocks[0] else {
            panic!("not a table: {document:?}");
        };
        assert_eq!(alignments, &vec![Alignment::Left, Alignment::Right]);
        assert_eq!(plain(&header[1]), "Shortcut");
        assert_eq!(rows[0][1], vec![Span { text: "Ctrl + N".to_string(), code: true, ..Span::default() }]);
    }

    #[test]
    fn tight_and_nested_lists_hold_their_text() {
        let document = parse("- one\n- two\n  1. inner\n\n***\n");
        let Block::List { start: None, items } = &document.blocks[0] else {
            panic!("not a list: {document:?}");
        };
        assert_eq!(items[0], vec![Block::Paragraph(vec![text("one")])]);
        assert!(matches!(items[1][1], Block::List { start: Some(1), .. }));
        assert_eq!(document.blocks[1], Block::Rule);
    }

    #[test]
    fn alerts_and_headings_are_kept() {
        let document = parse("## Menus and Fast Access\n\n> [!WARNING]\n> Careful.\n");
        let Block::Heading { level: 2, anchor, .. } = &document.blocks[0] else {
            panic!("not a heading: {document:?}");
        };
        assert_eq!(anchor, "menus-and-fast-access");
        assert!(matches!(document.blocks[1], Block::Quote { alert: Some(BlockQuoteKind::Warning), .. }));
    }

    #[test]
    fn breaks_and_pictures() {
        let document = parse("a<br>b ![picture](x.png) c");
        let Block::Paragraph(spans) = &document.blocks[0] else {
            panic!("not a paragraph: {document:?}");
        };
        assert_eq!(plain(spans), "a\nb  c");
    }
}
