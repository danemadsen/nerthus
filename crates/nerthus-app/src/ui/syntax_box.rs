//! `SyntaxBox`, the behaviour the expression boxes of Edit Expression and
//! the Math node's editor share: their colouring rules (`SyntaxConfig`),
//! and the keys taken from the text box while it has the focus. Tab and
//! Shift+Tab indent four spaces at a time (`ExpandTabs`, `IndentCount`),
//! the caret's spaces or, with a selection, its lines; Enter keeps the
//! line's indentation (`AutoIndent`). Neither box shows line numbers
//! (`ShowLineNumbers="False"`).

use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{Color32, Event, FontId, Id, Key, Ui};

use crate::theme;

/// `IndentCount`.
const INDENT: usize = 4;

/// The `KeywordRule`s, matched as whole words.
const KEYWORDS: [&str; 10] = ["if", "else", "return", "while", "for", "break", "continue", "switch", "case", "default"];
const DECLARATIONS: [&str; 7] = ["var", "local", "constant", "infix", "prefix", "postfix", "operator"];
const FUNCTIONS: [&str; 24] = [
    "abs", "acos", "asin", "atan", "atan2", "ceil", "cos", "cosh", "exp", "floor", "ln", "log", "log10", "max", "min", "pow", "rand", "round", "sin", "sinh", "sqrt", "tan", "tanh", "trunc",
];
/// The operator rule's keywords (`"+,-,/,*,%,=,!,^,|,&,<,>,"`: the
/// trailing comma ends the list), matched anywhere.
const OPERATORS: [char; 12] = ['+', '-', '/', '*', '%', '=', '!', '^', '|', '&', '<', '>'];
const BRACKETS: [char; 6] = ['(', ')', '[', ']', '{', '}'];
/// `Accent-Muted-Yellow`.
const MUTED_YELLOW: Color32 = Color32::from_rgb(0xcb, 0xcd, 0x77);

/// `Char.IsLetterOrDigit`, or `_`: what a whole word is made of (and
/// .NET's `\w`, near enough).
fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn paint(colours: &mut [Color32], from: usize, to: usize, colour: Color32) {
    for slot in &mut colours[from..to] {
        *slot = colour;
    }
}

/// The colour of each character of `text`, which starts in the box's
/// foreground (`TextBox-Foreground`). The rules paint in order, each over
/// the ones before it (`FormatInstruction`s by `RuleId`): keywords,
/// declarations and functions (whole words), operators (anywhere),
/// numbers, brackets, strings and line comments, each on every line with
/// its line break, then block comments over the whole text.
pub fn colours(text: &str) -> Vec<Color32> {
    let characters: Vec<char> = text.chars().collect();
    let mut colours = vec![theme::LABEL_DEFAULT; characters.len()];
    for (start, end) in lines(&characters) {
        paint_line(&characters[start..end], &mut colours[start..end]);
    }
    for (start, end) in block_comments(&characters) {
        paint(&mut colours, start, end, theme::ACCENT_MUTED_GREEN);
    }
    colours
}

/// The line rules, in order.
fn paint_line(line: &[char], colours: &mut [Color32]) {
    for (start, end) in words(line) {
        let word: String = line[start..end].iter().collect();
        let colour = if KEYWORDS.contains(&word.as_str()) {
            Some(theme::ACCENT_MUTED_BLUE)
        } else if DECLARATIONS.contains(&word.as_str()) {
            Some(theme::ACCENT_MUTED_TEAL)
        } else if FUNCTIONS.contains(&word.as_str()) {
            Some(theme::ACCENT_MUTED_PURPLE)
        } else {
            None
        };
        if let Some(colour) = colour {
            paint(colours, start, end, colour);
        }
    }
    for (position, character) in line.iter().enumerate() {
        if OPERATORS.contains(character) {
            colours[position] = theme::ACCENT_MUTED_RED;
        }
    }
    for (start, end) in numbers(line) {
        paint(colours, start, end, theme::ACCENT_GRAY);
    }
    for (position, character) in line.iter().enumerate() {
        if BRACKETS.contains(character) {
            colours[position] = MUTED_YELLOW;
        }
    }
    for (start, end) in strings(line) {
        paint(colours, start, end, theme::ACCENT_BRIGHT);
    }
    if let Some((start, end)) = line_comment(line) {
        paint(colours, start, end, theme::ACCENT_MUTED_GREEN);
    }
}

/// The runs of word characters: a keyword counts only as a whole one
/// (`IsStartWordBoundary`, `IsEndWordBoundary`).
fn words(line: &[char]) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut index = 0;
    while index < line.len() {
        if !is_word(line[index]) {
            index += 1;
            continue;
        }
        let start = index;
        while index < line.len() && is_word(line[index]) {
            index += 1;
        }
        found.push((start, index));
    }
    found
}

/// The matches of `\b\d+\.?\d*\b`, left to right.
fn numbers(line: &[char]) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut start = 0;
    while start < line.len() {
        match number_at(line, start) {
            Some(end) => {
                found.push((start, end));
                start = end;
            }
            None => start += 1,
        }
    }
    found
}

/// Where `\b\d+\.?\d*\b` matching at `start` ends: the regex tries the
/// longest number first and gives back characters until a word boundary
/// follows, so `1.5a` gives `1.` and `1. ` gives `1`.
fn number_at(line: &[char], start: usize) -> Option<usize> {
    let word_at = |index: usize| line.get(index).copied().is_some_and(is_word);
    let boundary = |index: usize| (index > 0 && word_at(index - 1)) != word_at(index);
    let digit_at = |index: usize| line.get(index).is_some_and(char::is_ascii_digit);
    if !digit_at(start) || !boundary(start) {
        return None;
    }
    let mut digits_end = start;
    while digit_at(digits_end) {
        digits_end += 1;
    }
    if line.get(digits_end) == Some(&'.') {
        let mut fraction_end = digits_end + 1;
        while digit_at(fraction_end) {
            fraction_end += 1;
        }
        for end in (digits_end + 1..=fraction_end).rev() {
            if boundary(end) {
                return Some(end);
            }
        }
    }
    // Fewer digits would end between two of them: never a boundary.
    boundary(digits_end).then_some(digits_end)
}

/// The matches of `"(?:[^"\\]|\\.)*"` on one line.
fn strings(line: &[char]) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut start = 0;
    while start < line.len() {
        if line[start] != '"' {
            start += 1;
            continue;
        }
        match string_end(line, start) {
            Some(end) => {
                found.push((start, end));
                start = end;
            }
            None => start += 1,
        }
    }
    found
}

/// The end of the quoted text opening at `start`, if it closes on the line.
/// A backslash takes the character after it (but not a line break).
fn string_end(line: &[char], start: usize) -> Option<usize> {
    let mut index = start + 1;
    loop {
        match line.get(index) {
            None => return None,
            Some('"') => return Some(index + 1),
            Some('\\') => match line.get(index + 1) {
                Some(next) if *next != '\n' => index += 2,
                _ => return None,
            },
            Some(_) => index += 1,
        }
    }
}

/// `//.*`: from the line's first `//` to its end.
fn line_comment(line: &[char]) -> Option<(usize, usize)> {
    let start = line.windows(2).position(|pair| pair == ['/', '/'])?;
    let rest = &line[start..];
    let length = rest.iter().position(|character| *character == '\n').unwrap_or(rest.len());
    Some((start, start + length))
}

/// `/\*[^*]*\*+(?:[^\/*][^*]*\*+)*/`: each `/*` to the first `*/` after
/// it; one that never closes is no comment.
fn block_comments(characters: &[char]) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut start = 0;
    while start + 1 < characters.len() {
        if characters[start] != '/' || characters[start + 1] != '*' {
            start += 1;
            continue;
        }
        let close = characters[start + 2..].windows(2).position(|pair| pair == ['*', '/']);
        match close {
            Some(offset) => {
                let end = start + 2 + offset + 2;
                found.push((start, end));
                start = end;
            }
            None => start += 1,
        }
    }
    found
}

/// The text laid out in its colours.
pub fn highlight(text: &str, font: FontId) -> LayoutJob {
    let characters: Vec<char> = text.chars().collect();
    let colours = colours(text);
    let mut job = LayoutJob::default();
    let mut start = 0;
    while start < characters.len() {
        let colour = colours[start];
        let mut end = start;
        while end < characters.len() && colours[end] == colour {
            end += 1;
        }
        let piece: String = characters[start..end].iter().collect();
        job.append(&piece, 0.0, TextFormat::simple(font.clone(), colour));
        start = end;
    }
    job
}

/// The lines (`Text.GetLines`): each ends after its line break, and the
/// last runs to the end (empty after a final break).
fn lines(characters: &[char]) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut start = 0;
    for (index, character) in characters.iter().enumerate() {
        if *character == '\n' {
            found.push((start, index + 1));
            start = index + 1;
        }
    }
    found.push((start, characters.len()));
    found
}

/// The line holding character `index` (`GetLineIndexFromCharacterIndex`).
fn line_of(lines: &[(usize, usize)], index: usize) -> usize {
    let holding = lines.iter().position(|(start, end)| *start <= index && index < *end);
    holding.unwrap_or(lines.len() - 1)
}

/// The spaces right before `position`, back to the line's start.
fn spaces_before(characters: &[char], position: usize) -> usize {
    let before = &characters[..position];
    before.iter().rev().take_while(|character| **character == ' ').count()
}

/// The spaces a line starts with.
fn leading_spaces(line: &[char]) -> usize {
    line.iter().take_while(|character| **character == ' ').count()
}

/// The text of a box and its selection (`start` to `end`; a caret when
/// they meet), as `SyntaxRenderer`'s key handling changes them.
#[derive(Clone, Debug, PartialEq)]
struct Editing {
    characters: Vec<char>,
    start: usize,
    end: usize,
}

impl Editing {
    fn text(&self) -> String {
        self.characters.iter().collect()
    }

    /// The text without the selection, the caret where it was.
    fn delete_selection(&mut self) {
        self.characters.drain(self.start..self.end);
        self.end = self.start;
    }

    fn insert(&mut self, inserted: &str) {
        self.delete_selection();
        let added: Vec<char> = inserted.chars().collect();
        let count = added.len();
        self.characters.splice(self.start..self.start, added);
        self.start += count;
        self.end = self.start;
    }

    /// `IncreaseCaretIndent`: spaces to the next multiple of four, counting
    /// the spaces before the caret.
    fn increase_caret_indent(&mut self) {
        let before = spaces_before(&self.characters, self.start);
        let count = INDENT - before % INDENT;
        self.insert(&" ".repeat(count));
    }

    /// `DecreaseCaretIndent`: back to the previous multiple of four, when
    /// spaces come before the caret.
    fn decrease_caret_indent(&mut self) {
        let before = spaces_before(&self.characters, self.start);
        if before == 0 {
            return;
        }
        let remainder = before % INDENT;
        let count = if remainder == 0 { INDENT } else { remainder };
        self.characters.drain(self.start - count..self.start);
        self.start -= count;
        self.end = self.start;
    }

    /// `IncreaseBlockIndent` and `DecreaseBlockIndent`: the lines from the
    /// selection's first to the one holding its last character get their
    /// leading spaces to the next (or previous) multiple of four; the
    /// selection keeps its distance from the ends of its first and last
    /// lines (no nearer their starts when unindenting).
    fn indent_block(&mut self, increase: bool) {
        let old_lines = lines(&self.characters);
        let first = line_of(&old_lines, self.start);
        let last = line_of(&old_lines, self.end - 1);
        let from_first_end = old_lines[first].1 - self.start;
        let from_last_end = old_lines[last].1 - self.end;

        let mut changed: Vec<char> = self.characters[..old_lines[first].0].to_vec();
        for &(start, end) in &old_lines[first..=last] {
            let line = &self.characters[start..end];
            let leading = leading_spaces(line);
            let remainder = leading % INDENT;
            let indented = if increase {
                let added = if remainder == 0 { INDENT } else { INDENT - remainder };
                leading + added
            } else if remainder == 0 {
                (leading / INDENT).saturating_sub(1) * INDENT
            } else {
                leading / INDENT * INDENT
            };
            changed.extend(std::iter::repeat_n(' ', indented));
            changed.extend_from_slice(&line[leading..]);
        }
        changed.extend_from_slice(&self.characters[old_lines[last].1..]);

        let new_lines = lines(&changed);
        let (first_start, first_end) = new_lines[first];
        let (last_start, last_end) = new_lines[last];
        if increase {
            self.start = first_end - from_first_end;
            self.end = last_end - from_last_end;
        } else {
            self.start = first_end.saturating_sub(from_first_end).max(first_start);
            self.end = last_end.saturating_sub(from_last_end).max(last_start);
        }
        self.characters = changed;
    }

    /// Tab, or Shift+Tab: the caret's spaces, or the selected lines.
    fn tab(&mut self, shift: bool) {
        match (self.start == self.end, shift) {
            (true, false) => self.increase_caret_indent(),
            (true, true) => self.decrease_caret_indent(),
            (false, increase) => self.indent_block(!increase),
        }
    }

    /// Enter (`AutoIndent`): a line break, then the whitespace the caret's
    /// line starts with.
    fn line_break(&mut self) {
        let all_lines = lines(&self.characters);
        let (start, end) = all_lines[line_of(&all_lines, self.start)];
        let line = &self.characters[start..end];
        let is_indentation = |character: &&char| character.is_whitespace() && **character != '\n' && **character != '\r';
        let indentation: String = line.iter().take_while(is_indentation).collect();
        self.insert(&format!("\n{indentation}"));
    }
}

/// What the keys the box takes do: a Tab or Enter press, with Shift.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Press {
    Tab { shift: bool },
    Enter,
}

/// Takes this frame's Tab and Enter presses out of the input, so the text
/// box does not handle them itself.
fn take_presses(ui: &Ui) -> Vec<Press> {
    ui.input_mut(|input| {
        let mut presses = Vec::new();
        input.events.retain(|event| {
            let Event::Key { key, pressed: true, modifiers, .. } = event else {
                return true;
            };
            match key {
                Key::Tab => presses.push(Press::Tab { shift: modifiers.shift }),
                Key::Enter => presses.push(Press::Enter),
                _ => return true,
            }
            false
        });
        presses
    })
}

/// Applies the keys `SyntaxBox` takes to the text box `id` (while it has
/// the focus), before the box itself runs; the box must leave Enter alone
/// (`return_key(None)`) and keep the focus on Tab (`lock_focus`). Returns
/// whether the text changed.
pub fn take_keys(ui: &Ui, id: Id, text: &mut String) -> bool {
    if !ui.memory(|memory| memory.has_focus(id)) {
        return false;
    }
    let presses = take_presses(ui);
    if presses.is_empty() {
        return false;
    }
    let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    let length = text.chars().count();
    let caret = CCursorRange::one(CCursor::new(length));
    let range = state.cursor.char_range().unwrap_or(caret);
    let [first, second] = range.sorted_cursors();
    let mut editing = Editing { characters: text.chars().collect(), start: first.index.0.min(length), end: second.index.0.min(length) };
    for press in presses {
        match press {
            Press::Tab { shift } => editing.tab(shift),
            Press::Enter => editing.line_break(),
        }
    }
    *text = editing.text();
    let selection = CCursorRange::two(CCursor::new(editing.start), CCursor::new(editing.end));
    state.cursor.set_char_range(Some(selection));
    state.store(ui.ctx(), id);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editing(text: &str, start: usize, end: usize) -> Editing {
        Editing { characters: text.chars().collect(), start, end }
    }

    /// The colour of the characters `from..to` of `text`, if they share one.
    fn colour_of(text: &str, from: usize, to: usize) -> Option<Color32> {
        let colours = colours(text);
        let first = colours[from];
        colours[from..to].iter().all(|colour| *colour == first).then_some(first)
    }

    #[test]
    fn rules_paint_over_the_ones_before() {
        let text = "sin(a) + 2 // twice";
        assert_eq!(colour_of(text, 0, 3), Some(theme::ACCENT_MUTED_PURPLE));
        assert_eq!(colour_of(text, 3, 4), Some(MUTED_YELLOW));
        assert_eq!(colour_of(text, 4, 5), Some(theme::LABEL_DEFAULT));
        assert_eq!(colour_of(text, 7, 8), Some(theme::ACCENT_MUTED_RED));
        assert_eq!(colour_of(text, 9, 10), Some(theme::ACCENT_GRAY));
        assert_eq!(colour_of(text, 11, 19), Some(theme::ACCENT_MUTED_GREEN));
        // Keywords only as whole words; commas are not operators.
        let words = "if(sine, x_if) else";
        assert_eq!(colour_of(words, 0, 2), Some(theme::ACCENT_MUTED_BLUE));
        assert_eq!(colour_of(words, 3, 7), Some(theme::LABEL_DEFAULT));
        assert_eq!(colour_of(words, 7, 8), Some(theme::LABEL_DEFAULT));
        assert_eq!(colour_of(words, 9, 13), Some(theme::LABEL_DEFAULT));
        assert_eq!(colour_of(words, 15, 19), Some(theme::ACCENT_MUTED_BLUE));
        // Strings paint over what is in them, comments over strings.
        let quoted = "\"a+1\" // \"b\"";
        assert_eq!(colour_of(quoted, 0, 5), Some(theme::ACCENT_BRIGHT));
        assert_eq!(colour_of(quoted, 6, 12), Some(theme::ACCENT_MUTED_GREEN));
    }

    #[test]
    fn numbers_end_where_the_regex_finds_a_boundary() {
        let number = |text: &str| numbers(&text.chars().collect::<Vec<char>>());
        assert_eq!(number("12.5"), vec![(0, 4)]);
        assert_eq!(number("1.5a"), vec![(0, 2)]);
        assert_eq!(number("1.x"), vec![(0, 2)]);
        assert_eq!(number("1. "), vec![(0, 1)]);
        assert_eq!(number("x2 log10 3"), vec![(9, 10)]);
        assert_eq!(number("12a"), Vec::<(usize, usize)>::new());
    }

    #[test]
    fn strings_and_comments_follow_their_patterns() {
        let chars = |text: &str| text.chars().collect::<Vec<char>>();
        assert_eq!(strings(&chars("say \"a\\\"b\" x")), vec![(4, 10)]);
        // An unclosed string is none, even past an escaped quote.
        assert_eq!(strings(&chars("\"a\\\"b\n")), Vec::<(usize, usize)>::new());
        assert_eq!(line_comment(&chars("a // b\n")), Some((2, 6)));
        // Block comments reach the first close, across lines.
        assert_eq!(block_comments(&chars("a /* b\n*/ c /* d")), vec![(2, 9)]);
        assert_eq!(block_comments(&chars("/**/ /*/")), vec![(0, 4)]);
        let text = "1 /* if\n2 */ x";
        assert_eq!(colour_of(text, 2, 12), Some(theme::ACCENT_MUTED_GREEN));
        assert_eq!(colour_of(text, 0, 1), Some(theme::ACCENT_GRAY));
    }

    #[test]
    fn tab_indents_the_caret_to_the_next_stop() {
        let mut caret = editing("ab", 1, 1);
        caret.tab(false);
        assert_eq!((caret.text().as_str(), caret.start), ("a    b", 5));
        let mut spaced = editing("  x", 2, 2);
        spaced.tab(false);
        assert_eq!((spaced.text().as_str(), spaced.start), ("    x", 4));
        // Shift+Tab takes the spaces back to the previous stop.
        let mut back = editing("      x", 6, 6);
        back.tab(true);
        assert_eq!((back.text().as_str(), back.start), ("    x", 4));
        back.tab(true);
        assert_eq!((back.text().as_str(), back.start), ("x", 0));
        back.tab(true);
        assert_eq!((back.text().as_str(), back.start), ("x", 0));
    }

    #[test]
    fn tab_indents_the_selected_lines() {
        // Lines "a" and "  b" selected from inside the first to inside the
        // second; the third is left alone.
        let mut block = editing("a\n  b\nc", 0, 4);
        block.tab(false);
        assert_eq!(block.text(), "    a\n    b\nc");
        assert_eq!((block.start, block.end), (4, 10));
        block.tab(true);
        assert_eq!(block.text(), "a\nb\nc");
        assert_eq!((block.start, block.end), (0, 2));
        // Unindenting keeps the selection on its lines.
        let mut short = editing("    ab", 5, 6);
        short.tab(true);
        assert_eq!((short.text().as_str(), short.start, short.end), ("ab", 1, 2));
    }

    #[test]
    fn enter_keeps_the_line_indentation() {
        let mut caret = editing("  if\n", 4, 4);
        caret.line_break();
        assert_eq!((caret.text().as_str(), caret.start), ("  if\n  \n", 7));
        let mut selected = editing("\tab cd", 3, 5);
        selected.line_break();
        assert_eq!((selected.text().as_str(), selected.start), ("\tab\n\td", 5));
    }
}
