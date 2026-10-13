//! The little Markdown the help windows need (the shortcut guide and the
//! change log): headings, paragraphs, bulleted lists,
//! tables, rules, and inside them bold, italics, code and links.

use egui::{Align, Color32, FontId, Layout, RichText, Sense, Stroke, Ui, Vec2};

use crate::theme;

/// A piece of a line, as it is drawn.
#[derive(Clone, Debug, PartialEq)]
enum Span {
    Plain(String),
    Bold(String),
    Italic(String),
    Code(String),
    Link { text: String, url: String },
}

/// A block of the text.
#[derive(Clone, Debug, PartialEq)]
enum Block {
    Heading { level: usize, text: String },
    Paragraph(String),
    Bullet(String),
    Rule,
    Table { header: Vec<String>, right_aligned: Vec<bool>, rows: Vec<Vec<String>> },
}

/// Draws the text.
pub fn show(ui: &mut Ui, text: &str) {
    for block in blocks(text) {
        match block {
            Block::Heading { level, text } => heading(ui, level, &text),
            Block::Paragraph(text) => {
                spans_wrapped(ui, &spans(&text), 13.0);
                ui.add_space(10.0);
            }
            Block::Bullet(text) => {
                // The text in a column of its own, so wrapped lines keep
                // the indent.
                ui.horizontal_top(|ui| {
                    ui.add_space(12.0);
                    ui.label(RichText::new("•").font(theme::interface(13.0)).color(theme::LABEL_DEFAULT));
                    ui.add_space(4.0);
                    ui.vertical(|ui| spans_wrapped(ui, &spans(&text), 13.0));
                });
                ui.add_space(4.0);
            }
            Block::Rule => {
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
            }
            Block::Table { header, right_aligned, rows } => {
                table(ui, &header, &right_aligned, &rows);
                ui.add_space(12.0);
            }
        }
    }
}

/// The text's blocks: consecutive lines of a paragraph joined, a table's
/// rows gathered.
fn blocks(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut table_lines: Vec<&str> = Vec::new();
    let flush_paragraph = |paragraph: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph(paragraph.join(" ")));
            paragraph.clear();
        }
    };
    let flush_table = |table_lines: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !table_lines.is_empty() {
            blocks.push(table_block(table_lines));
            table_lines.clear();
        }
    };
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.starts_with('|') {
            flush_paragraph(&mut paragraph, &mut blocks);
            table_lines.push(line);
            continue;
        }
        flush_table(&mut table_lines, &mut blocks);
        if line.is_empty() {
            flush_paragraph(&mut paragraph, &mut blocks);
            continue;
        }
        let is_rule = line.len() >= 3 && (line.chars().all(|character| character == '*') || line.chars().all(|character| character == '-'));
        if is_rule {
            flush_paragraph(&mut paragraph, &mut blocks);
            blocks.push(Block::Rule);
            continue;
        }
        if line.starts_with('#') {
            flush_paragraph(&mut paragraph, &mut blocks);
            let level = line.chars().take_while(|character| *character == '#').count();
            let text = line[level..].trim().to_string();
            blocks.push(Block::Heading { level, text });
            continue;
        }
        if let Some(item) = line.strip_prefix("* ").or_else(|| line.strip_prefix("- ")) {
            flush_paragraph(&mut paragraph, &mut blocks);
            blocks.push(Block::Bullet(item.trim().to_string()));
            continue;
        }
        paragraph.push(line);
    }
    flush_table(&mut table_lines, &mut blocks);
    flush_paragraph(&mut paragraph, &mut blocks);
    blocks
}

/// `| a | b |` → the cells, trimmed.
fn cells(line: &str) -> Vec<String> {
    let inner = line.trim().trim_start_matches('|').trim_end_matches('|');
    inner.split('|').map(|cell| cell.trim().to_string()).collect()
}

/// A table's rows: the header, the alignment row (`---:` is right-aligned)
/// and the body.
fn table_block(lines: &[&str]) -> Block {
    let header = cells(lines[0]);
    let mut right_aligned = vec![false; header.len()];
    let mut body_start = 1;
    if let Some(alignment) = lines.get(1) {
        let alignment_cells = cells(alignment);
        let is_alignment = alignment_cells.iter().all(|cell| !cell.is_empty() && cell.chars().all(|character| matches!(character, ':' | '-')));
        if is_alignment {
            for (column, cell) in alignment_cells.iter().enumerate() {
                if column < right_aligned.len() {
                    right_aligned[column] = cell.ends_with(':') && !cell.starts_with(':');
                }
            }
            body_start = 2;
        }
    }
    let rows = lines[body_start..].iter().map(|line| cells(line)).collect();
    Block::Table { header, right_aligned, rows }
}

/// A line's spans: `**bold**`, `*italic*`, `` `code` `` (or ``` `` ` `` ```)
/// and `[text](url)`.
fn spans(text: &str) -> Vec<Span> {
    let mut result = Vec::new();
    let mut plain = String::new();
    let characters: Vec<char> = text.chars().collect();
    let mut index = 0;
    let push_plain = |plain: &mut String, result: &mut Vec<Span>| {
        if !plain.is_empty() {
            result.push(Span::Plain(std::mem::take(plain)));
        }
    };
    while index < characters.len() {
        let rest: String = characters[index..].iter().collect();
        if rest.starts_with("``")
            && let Some(end) = rest[2..].find("``")
        {
            push_plain(&mut plain, &mut result);
            let code = rest[2..2 + end].trim().to_string();
            result.push(Span::Code(code));
            index += rest[..2 + end + 2].chars().count();
            continue;
        }
        if rest.starts_with('`')
            && let Some(end) = rest[1..].find('`')
        {
            push_plain(&mut plain, &mut result);
            result.push(Span::Code(rest[1..1 + end].to_string()));
            index += rest[..1 + end + 1].chars().count();
            continue;
        }
        if rest.starts_with("**")
            && let Some(end) = rest[2..].find("**")
        {
            push_plain(&mut plain, &mut result);
            result.push(Span::Bold(rest[2..2 + end].to_string()));
            index += rest[..2 + end + 2].chars().count();
            continue;
        }
        if rest.starts_with('*')
            && !rest.starts_with("* ")
            && let Some(end) = rest[1..].find('*')
        {
            push_plain(&mut plain, &mut result);
            result.push(Span::Italic(rest[1..1 + end].to_string()));
            index += rest[..1 + end + 1].chars().count();
            continue;
        }
        if rest.starts_with('[')
            && let Some(middle) = rest.find("](")
            && let Some(end) = rest[middle..].find(')')
        {
            push_plain(&mut plain, &mut result);
            let link_text = rest[1..middle].to_string();
            let url = rest[middle + 2..middle + end].to_string();
            result.push(Span::Link { text: link_text, url });
            index += rest[..middle + end + 1].chars().count();
            continue;
        }
        plain.push(characters[index]);
        index += 1;
    }
    push_plain(&mut plain, &mut result);
    result
}

fn heading(ui: &mut Ui, level: usize, text: &str) {
    let (font, color, above, below): (FontId, Color32, f32, f32) = match level {
        1 => (theme::interface(26.0), theme::LABEL_EMPHASIS, 12.0, 10.0),
        2 => (theme::interface(21.0), theme::LABEL_EMPHASIS, 14.0, 8.0),
        3 => (theme::semibold(16.0), theme::LABEL_EMPHASIS, 10.0, 6.0),
        _ => (theme::semibold(14.0), theme::ACCENT_PRIMARY, 10.0, 6.0),
    };
    ui.add_space(above);
    ui.label(RichText::new(text).font(font).color(color));
    ui.add_space(below);
}

/// Spans in a wrapping row of their own.
fn spans_wrapped(ui: &mut Ui, line: &[Span], size: f32) {
    ui.horizontal_wrapped(|ui| add_spans(ui, line, size));
}

fn add_spans(ui: &mut Ui, line: &[Span], size: f32) {
    ui.spacing_mut().item_spacing.x = 0.0;
    for span in line {
        match span {
            Span::Plain(text) => {
                ui.label(RichText::new(text).font(theme::interface(size)).color(theme::LABEL_DEFAULT));
            }
            Span::Bold(text) => {
                ui.label(RichText::new(text).font(theme::semibold(size)).color(theme::LABEL_EMPHASIS));
            }
            Span::Italic(text) => {
                ui.label(RichText::new(text).font(theme::interface(size)).color(theme::LABEL_DEFAULT).italics());
            }
            Span::Code(text) => code(ui, text, size),
            Span::Link { text, url } => {
                let label = RichText::new(text).font(theme::interface(size)).color(theme::ACCENT_MUTED_GREEN);
                let response = ui.add(egui::Label::new(label).sense(Sense::click()));
                let response = response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(url);
                if response.clicked() {
                    crate::export_build::open_url(url);
                }
            }
        }
    }
}

/// A header cell's plain text in bold.
fn emphasized(span: Span) -> Span {
    match span {
        Span::Plain(text) => Span::Bold(text),
        other => other,
    }
}

/// Code: monospace on a dark plate.
fn code(ui: &mut Ui, text: &str, size: f32) {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::monospace(size - 1.0), theme::ACCENT_PRIMARY);
    let padding = Vec2::new(5.0, 2.0);
    let (rect, _) = ui.allocate_exact_size(galley.size() + padding * 2.0, Sense::hover());
    ui.painter().rect_filled(rect, 3.0, theme::TEXTBOX_BACKGROUND);
    ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.0, theme::BUTTON_BORDER), egui::StrokeKind::Inside);
    ui.painter().galley(rect.min + padding, galley, theme::ACCENT_PRIMARY);
}

/// A table: the columns share the width, the header in semibold over a
/// line, every other row shaded.
fn table(ui: &mut Ui, header: &[String], right_aligned: &[bool], rows: &[Vec<String>]) {
    let width = ui.available_width();
    let columns = header.len().max(1);
    let column_width = width / columns as f32;
    let row_height = 26.0;
    let draw_row = |ui: &mut Ui, cells: &[String], is_header: bool, shaded: bool| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, row_height), Sense::hover());
        if shaded {
            ui.painter().rect_filled(rect, 0.0, theme::PANEL_CLADDING);
        }
        if is_header {
            let bottom = rect.bottom() - 0.5;
            ui.painter().hline(rect.x_range(), bottom, Stroke::new(1.0, theme::MENU_SEPARATOR));
        }
        for column in 0..columns {
            let text = cells.get(column).map(String::as_str).unwrap_or_default();
            if text.is_empty() {
                continue;
            }
            let left = rect.left() + column as f32 * column_width;
            let cell = egui::Rect::from_min_size(egui::pos2(left, rect.top()), Vec2::new(column_width, row_height));
            let right = right_aligned.get(column).copied().unwrap_or(false);
            let layout = if right { Layout::right_to_left(Align::Center) } else { Layout::left_to_right(Align::Center) };
            let inner = cell.shrink2(Vec2::new(8.0, 0.0));
            let builder = egui::UiBuilder::new().max_rect(inner).layout(layout);
            let mut child = ui.new_child(builder);
            let mut line = spans(text);
            if is_header {
                line = line.into_iter().map(emphasized).collect();
            }
            if right {
                line.reverse();
            }
            add_spans(&mut child, &line, 13.0);
        }
    };
    draw_row(ui, header, true, false);
    for (index, row) in rows.iter().enumerate() {
        draw_row(ui, row, false, index % 2 == 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_spans_are_found() {
        let found = spans("Press `Ctrl + N` or **bold** and *it* [here](https://example.com).");
        assert_eq!(
            found,
            vec![
                Span::Plain("Press ".to_string()),
                Span::Code("Ctrl + N".to_string()),
                Span::Plain(" or ".to_string()),
                Span::Bold("bold".to_string()),
                Span::Plain(" and ".to_string()),
                Span::Italic("it".to_string()),
                Span::Plain(" ".to_string()),
                Span::Link { text: "here".to_string(), url: "https://example.com".to_string() },
                Span::Plain(".".to_string()),
            ]
        );
    }

    #[test]
    fn double_backticks_hold_a_backtick() {
        assert_eq!(spans("`` ` ``"), vec![Span::Code("`".to_string())]);
    }

    #[test]
    fn tables_keep_their_alignment() {
        let text = "| Command | Shortcut |\n| :--- | ---: |\n| New | `Ctrl + N` |";
        let found = blocks(text);
        let expected = Block::Table {
            header: vec!["Command".to_string(), "Shortcut".to_string()],
            right_aligned: vec![false, true],
            rows: vec![vec!["New".to_string(), "`Ctrl + N`".to_string()]],
        };
        assert_eq!(found, vec![expected]);
    }

    #[test]
    fn blocks_split_on_blank_lines_and_rules() {
        let found = blocks("# Title\n\nOne\ntwo\n\n***\n* item");
        let expected = vec![
            Block::Heading { level: 1, text: "Title".to_string() },
            Block::Paragraph("One two".to_string()),
            Block::Rule,
            Block::Bullet("item".to_string()),
        ];
        assert_eq!(found, expected);
    }
}
