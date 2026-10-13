//! The report a build leaves in the build folder once it is done:
//! `report.txt`, the text of what it printed (its
//! console records the header it prints as the build starts, the lines it
//! writes as it goes, and the report's tables), and `report.json`, its
//! `BuildReport` as Newtonsoft writes it. A silent build (`--silent`)
//! leaves neither.
//!
//! The text is laid out as Spectre.Console lays it out for a console 80
//! characters wide (its width when the output is not a terminal): the
//! header's rule and table, and for a Standard build the Build
//! Results and Statistics tables side by side. Hybrid, Tiled and Region
//! builds report no node times, and their report ends at its rule.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use chrono::{DateTime, Datelike, Local, Timelike};

/// Spectre.Console's width for output that is not a terminal.
pub const CONSOLE_WIDTH: usize = 80;

/// `Environment.NewLine`, which Spectre.Console ends its lines with.
const NEWLINE: &str = if cfg!(windows) { "\r\n" } else { "\n" };

/// What the build was, for the header.
#[derive(Clone, Debug, Default)]
pub struct ReportHeader {
    /// The project built (`Filename`, shown as a shortened path).
    pub project_path: String,
    /// `Build Location`: the build folder.
    pub folder: String,
    /// `Build Type`: Standard, Standard Cropped to Tiles, Hybrid, Tiled or
    /// Region.
    pub build_type: String,
    pub resolution: usize,
    /// The build profile, if the build used one.
    pub profile: Option<String>,
    /// The nodes in the terrain and the nodes it exports.
    pub nodes: usize,
    pub exporting: usize,
    /// `TotalTiles`, for tiled output.
    pub tiles: Option<usize>,
    /// A Region build's region: its name and position.
    pub region: Option<(String, [f32; 2])>,
}

/// The `BuildReport`.
#[derive(Clone, Debug)]
pub struct BuildReport {
    /// The build resolution (Standard builds; 0, left out, otherwise).
    pub resolution: usize,
    pub node_count: usize,
    /// `Success`, the build loop's last `BuildResult` (`AlreadyBuilt`,
    /// `Failed`, …), `Cancelled` or `Failed`.
    pub result: String,
    pub start_time: DateTime<Local>,
    pub end_time: DateTime<Local>,
    /// `SecondsTaken`: each node of the terrain with the time its last
    /// build took, in ticks (100 ns), in the terrain's order (Standard
    /// builds only).
    pub seconds_taken: Vec<(String, i64)>,
}

const TICKS_PER_SECOND: i64 = 10_000_000;

impl BuildReport {
    /// `Duration`: the end time less the start time, in ticks.
    pub fn duration(&self) -> i64 {
        let elapsed = self.end_time - self.start_time;
        let nanoseconds = elapsed.num_nanoseconds().unwrap_or(i64::MAX);
        nanoseconds / 100
    }

    /// `FastestNode`: the first node with the shortest time.
    fn fastest(&self) -> Option<usize> {
        let mut found: Option<usize> = None;
        for (index, (_, ticks)) in self.seconds_taken.iter().enumerate() {
            if found.is_none_or(|best| *ticks < self.seconds_taken[best].1) {
                found = Some(index);
            }
        }
        found
    }

    /// `SlowestNode`: the first node with the longest time.
    fn slowest(&self) -> Option<usize> {
        let mut found: Option<usize> = None;
        for (index, (_, ticks)) in self.seconds_taken.iter().enumerate() {
            if found.is_none_or(|best| *ticks > self.seconds_taken[best].1) {
                found = Some(index);
            }
        }
        found
    }
}

/// `TimeSpan.FromMilliseconds(Node.BuildTime)` in ticks.
pub fn milliseconds_to_ticks(milliseconds: u64) -> i64 {
    (milliseconds as i64).saturating_mul(10_000)
}

/// The header's title: Nerthus's name and version.
fn title() -> String {
    format!(" NERTHUS BUILD  {}", env!("CARGO_PKG_VERSION"))
}

/// A blank line, the title's rule, a blank line and the table of
/// what is being built.
pub fn header_text(header: &ReportHeader) -> String {
    let mut text = String::new();
    text.push_str(NEWLINE);
    push_line(&mut text, &rule(&title(), CONSOLE_WIDTH));
    text.push_str(NEWLINE);

    let mut table = Table::new(true);
    table.add_column(Justify::Right, (1, 1));
    table.add_column(Justify::Left, (1, 1));
    table.add_text_row("Filename: ", Content::Path(TextPath::new(&header.project_path)));
    table.add_text_row("Build Location: ", Content::text(&header.folder));
    table.add_text_row("Build Type: ", Content::text(&header.build_type));
    let resolution = format!("{0} × {0}", header.resolution);
    table.add_text_row("Build Resolution: ", Content::text(&resolution));
    let profile = header.profile.clone().unwrap_or_else(|| "None / Last Saved Settings".to_string());
    table.add_text_row("Build Profile: ", Content::text(&profile));
    let nodes = format!("{:02} to build\n{:02} to export", header.nodes, header.exporting);
    table.add_text_row("Nodes: ", Content::text(&nodes));
    if let Some(tiles) = header.tiles {
        table.add_text_row("Tiles: ", Content::text(&tiles.to_string()));
    }
    if let Some((name, [x, y])) = &header.region {
        // `Float2.ToString()`: `{X:#0.000} x {Y:#0.000}`.
        let region = format!("{name} ({} x {})", fixed_three(*x), fixed_three(*y));
        table.add_text_row("Region: ", Content::text(&region));
    }
    for line in table.render(CONSOLE_WIDTH) {
        push_line(&mut text, &line);
    }
    text
}

/// A line the build writes as it goes (`AnsiConsole.MarkupLine`),
/// wrapped to the console.
pub fn console_line(line: &str) -> String {
    let mut text = String::new();
    let paragraph = Paragraph::new(line);
    for rendered in paragraph.render(CONSOLE_WIDTH, Justify::Left) {
        push_line(&mut text, &rendered);
    }
    text
}

/// A failed node as the build writes it (`[red]{Id:000} - {Name} - failed![/]`).
pub fn failed_node_line(id: i32, name: &str) -> String {
    console_line(&format!("{id:03} - {name} - failed!"))
}

/// The report's text: the header again, the report's rule and, for a
/// Standard build, its tables.
pub fn report_text(header: &ReportHeader, report: &BuildReport, standard: bool) -> String {
    let mut text = header_text(header);
    text.push_str(NEWLINE);
    push_line(&mut text, &rule("Build Report", CONSOLE_WIDTH));
    text.push_str(NEWLINE);
    if standard {
        let grid = report_grid(report);
        for line in grid.render(CONSOLE_WIDTH) {
            push_line(&mut text, &line);
        }
    }
    text.push_str(NEWLINE);
    text.push_str(NEWLINE);
    text
}

/// The Build Results and Statistics tables, side by side.
fn report_grid(report: &BuildReport) -> Table {
    let mut results = Table::new(true);
    results.title = Some("Build Results".to_string());
    results.add_column(Justify::Right, (1, 1));
    results.add_column(Justify::Left, (1, 1));
    let result = if report.result == "Success" { "Success" } else { report.result.as_str() };
    results.add_text_row("Result:", Content::text(result));
    results.add_text_row("Resolution:", Content::text(&report.resolution.to_string()));
    results.add_text_row("Node Count:", Content::text(&report.node_count.to_string()));
    results.add_text_row("Start Time:", Content::text(&invariant_date_time(&report.start_time)));
    results.add_text_row("End Time:", Content::text(&invariant_date_time(&report.end_time)));
    let duration = report.duration();
    results.add_text_row("Duration:", Content::text(&constant_time_span(duration)));
    // `TimeSpan.FromSeconds(Duration.TotalSeconds - Σ TotalSeconds)`.
    let mut built_seconds = 0.0f64;
    for (_, ticks) in &report.seconds_taken {
        built_seconds += *ticks as f64 / TICKS_PER_SECOND as f64;
    }
    let other_seconds = duration as f64 / TICKS_PER_SECOND as f64 - built_seconds;
    let other_ticks = (other_seconds * TICKS_PER_SECOND as f64) as i64;
    results.add_text_row("Non-Build Time:", Content::text(&constant_time_span(other_ticks)));

    let mut statistics = Table::new(true);
    statistics.title = Some("Statistics".to_string());
    statistics.caption = Some("* nodes that took <100ms are excluded.".to_string());
    statistics.add_column(Justify::Right, (1, 1));
    statistics.add_column(Justify::Left, (1, 1));
    for (name, ticks) in &report.seconds_taken {
        if *ticks as f64 / 10_000.0 > 100.0 {
            statistics.add_text_row(name, Content::text(&general_time_span(*ticks)));
        }
    }
    statistics.add_row(vec![Content::text(""), Content::text("")]);
    if let Some(index) = report.fastest() {
        let (name, ticks) = &report.seconds_taken[index];
        statistics.add_text_row("Fastest", Content::text(&format!("{name} @ {}", general_time_span(*ticks))));
    }
    if let Some(index) = report.slowest() {
        let (name, ticks) = &report.seconds_taken[index];
        statistics.add_text_row("Slowest", Content::text(&format!("{name} @ {}", general_time_span(*ticks))));
    }

    let mut grid = Table::grid();
    grid.add_column(Justify::Left, (0, 2));
    grid.add_column(Justify::Left, (0, 2));
    grid.add_row(vec![Content::Table(Box::new(results)), Content::Table(Box::new(statistics))]);
    grid
}

/// `report.json`: the report as the serializer writes it (indented,
/// members at their defaults left out, every object and list numbered for
/// references, the fastest and slowest nodes as references into the
/// list).
pub fn report_json(report: &BuildReport) -> String {
    let mut text = String::from("{\n  \"$id\": \"1\"");
    if report.resolution != 0 {
        let _ = write!(text, ",\n  \"Resolution\": {}", report.resolution);
    }
    if report.node_count != 0 {
        let _ = write!(text, ",\n  \"NodeCount\": {}", report.node_count);
    }
    let _ = write!(text, ",\n  \"Result\": {}", json_string(&report.result));
    let _ = write!(text, ",\n  \"StartTime\": \"{}\"", iso_date_time(&report.start_time));
    let _ = write!(text, ",\n  \"EndTime\": \"{}\"", iso_date_time(&report.end_time));
    let duration = report.duration();
    if duration != 0 {
        let _ = write!(text, ",\n  \"Duration\": \"{}\"", constant_time_span(duration));
    }
    text.push_str(",\n  \"SecondsTaken\": {\n    \"$id\": \"2\",\n    \"$values\": [");
    let first_item_id = 3;
    for (index, (name, ticks)) in report.seconds_taken.iter().enumerate() {
        let separator = if index == 0 { "" } else { "," };
        let _ = write!(text, "{separator}\n      {{\n        \"$id\": \"{}\",\n        \"Item1\": {}", first_item_id + index, json_string(name));
        if *ticks != 0 {
            let _ = write!(text, ",\n        \"Item2\": \"{}\"", constant_time_span(*ticks));
        }
        text.push_str("\n      }");
    }
    if report.seconds_taken.is_empty() {
        text.push_str("]\n  }");
    } else {
        text.push_str("\n    ]\n  }");
    }
    if let Some(index) = report.fastest() {
        let _ = write!(text, ",\n  \"FastestNode\": {{\n    \"$ref\": \"{}\"\n  }}", first_item_id + index);
    }
    if let Some(index) = report.slowest() {
        let _ = write!(text, ",\n  \"SlowestNode\": {{\n    \"$ref\": \"{}\"\n  }}", first_item_id + index);
    }
    text.push_str("\n}");
    text
}

/// Writes `report.txt` (the recorded text, its trailing line ends cut as
/// Spectre.Console's export cuts them) and `report.json` to the folder.
pub fn write_report(folder: &Path, recorded: &str, report: &BuildReport) -> std::io::Result<()> {
    let text = recorded.trim_end_matches('\n');
    fs::write(folder.join("report.txt"), text)?;
    fs::write(folder.join("report.json"), report_json(report))?;
    Ok(())
}

/// A JSON string as Newtonsoft escapes it (the default escaping: quotes,
/// backslashes and control characters).
fn json_string(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for character in text.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            '\u{8}' => quoted.push_str("\\b"),
            '\u{c}' => quoted.push_str("\\f"),
            control if (control as u32) < 0x20 => {
                let _ = write!(quoted, "\\u{:04x}", control as u32);
            }
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

/// A local time's ticks within its second.
fn fraction_ticks(time: &DateTime<Local>) -> u32 {
    time.nanosecond() % 1_000_000_000 / 100
}

/// Newtonsoft's ISO date of a local `DateTime`
/// (`yyyy-MM-ddTHH:mm:ss.FFFFFFFK`).
fn iso_date_time(time: &DateTime<Local>) -> String {
    let mut text = time.format("%Y-%m-%dT%H:%M:%S").to_string();
    let ticks = fraction_ticks(time);
    if ticks != 0 {
        let digits = format!("{ticks:07}");
        text.push('.');
        text.push_str(digits.trim_end_matches('0'));
    }
    text.push_str(&time.format("%:z").to_string());
    text
}

/// `DateTime.ToString(CultureInfo.InvariantCulture)`: `MM/dd/yyyy HH:mm:ss`.
fn invariant_date_time(time: &DateTime<Local>) -> String {
    format!("{:02}/{:02}/{:04} {:02}:{:02}:{:02}", time.month(), time.day(), time.year(), time.hour(), time.minute(), time.second())
}

/// `float.ToString("#0.000")`: the float's seven significant digits (as
/// .NET formats a float with a custom format), rounded half away from
/// zero to three decimals.
fn fixed_three(value: f32) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    let scientific = format!("{:.6e}", f64::from(value.abs()));
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let mut digits: Vec<u8> = mantissa.bytes().filter(u8::is_ascii_digit).map(|digit| digit - b'0').collect();
    let exponent: i32 = exponent.parse().unwrap_or(0);
    // The digits stand for 0.d1d2… × 10^(exponent + 1); keep three
    // decimals.
    let point = exponent + 1;
    let keep = point + 3;
    let mut thousandths: u64 = 0;
    if keep > 0 {
        let keep = keep as usize;
        let round_up = digits.get(keep).is_some_and(|digit| *digit >= 5);
        digits.resize(keep.max(digits.len()), 0);
        digits.truncate(keep);
        for digit in &digits {
            thousandths = thousandths * 10 + u64::from(*digit);
        }
        if round_up {
            thousandths += 1;
        }
    }
    let negative = value < 0.0 && thousandths != 0;
    let sign = if negative { "-" } else { "" };
    format!("{sign}{}.{:03}", thousandths / 1000, thousandths % 1000)
}

/// `TimeSpan.ToString()` (`c`): `[-][d.]hh:mm:ss[.fffffff]`.
fn constant_time_span(ticks: i64) -> String {
    let sign = if ticks < 0 { "-" } else { "" };
    let ticks = ticks.unsigned_abs();
    let ticks_per_second = TICKS_PER_SECOND as u64;
    let total_seconds = ticks / ticks_per_second;
    let fraction = ticks % ticks_per_second;
    let days = total_seconds / 86_400;
    let hours = total_seconds / 3600 % 24;
    let minutes = total_seconds / 60 % 60;
    let seconds = total_seconds % 60;
    let mut text = String::from(sign);
    if days != 0 {
        let _ = write!(text, "{days}.");
    }
    let _ = write!(text, "{hours:02}:{minutes:02}:{seconds:02}");
    if fraction != 0 {
        let _ = write!(text, ".{fraction:07}");
    }
    text
}

/// `TimeSpan.ToString("g")`: `[-][d:]h:mm:ss[.FFFFFFF]`.
fn general_time_span(ticks: i64) -> String {
    let sign = if ticks < 0 { "-" } else { "" };
    let ticks = ticks.unsigned_abs();
    let ticks_per_second = TICKS_PER_SECOND as u64;
    let total_seconds = ticks / ticks_per_second;
    let fraction = ticks % ticks_per_second;
    let days = total_seconds / 86_400;
    let hours = total_seconds / 3600 % 24;
    let minutes = total_seconds / 60 % 60;
    let seconds = total_seconds % 60;
    let mut text = String::from(sign);
    if days != 0 {
        let _ = write!(text, "{days}:");
    }
    let _ = write!(text, "{hours}:{minutes:02}:{seconds:02}");
    if fraction != 0 {
        let digits = format!("{fraction:07}");
        text.push('.');
        text.push_str(digits.trim_end_matches('0'));
    }
    text
}

fn push_line(text: &mut String, line: &str) {
    text.push_str(line);
    text.push_str(NEWLINE);
}

/// How many console cells a text takes (one a character).
fn cells(text: &str) -> usize {
    text.chars().count()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Justify {
    Left,
    Right,
    Center,
}

/// `Aligner.Align` on a line of `width` cells.
fn align(line: &str, justify: Justify, width: usize) -> String {
    let length = cells(line);
    if justify == Justify::Left || length >= width {
        return line.to_string();
    }
    let space = width - length;
    match justify {
        Justify::Right => format!("{}{line}", " ".repeat(space)),
        Justify::Center => {
            let left = space / 2;
            let right = left + space % 2;
            format!("{}{line}{}", " ".repeat(left), " ".repeat(right))
        }
        Justify::Left => line.to_string(),
    }
}

/// `Rule` with its title on the right (`Justify.Right`): the line, a space,
/// the title, a space and two more of the line.
fn rule(title: &str, width: usize) -> String {
    let title_cells = cells(title);
    if width <= 6 || title_cells > width - 6 {
        return "─".repeat(width);
    }
    let left = width - title_cells - 3 - 1;
    format!("{} {title} ──", "─".repeat(left))
}

/// `StringExtensions.SplitWords`: runs of white space and of anything else.
fn split_words(line: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_is_space = false;
    for character in line.chars() {
        let is_space = character.is_whitespace();
        if !current.is_empty() && is_space != current_is_space {
            words.push(std::mem::take(&mut current));
        }
        current_is_space = is_space;
        current.push(character);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// A run of text the wrapping works on (`Segment`); `None` is a line break.
type Word = Option<String>;

fn is_white_space(word: &str) -> bool {
    word.chars().all(char::is_whitespace)
}

/// `Paragraph`: lines of words, wrapped to a width as Spectre.Console wraps
/// them (long words folded).
#[derive(Clone, Debug)]
struct Paragraph {
    lines: Vec<Vec<String>>,
}

impl Paragraph {
    fn new(text: &str) -> Paragraph {
        let normalized = text.replace("\r\n", "\n");
        let mut lines = Vec::new();
        for line in normalized.split('\n') {
            if line.is_empty() {
                lines.push(vec![String::new()]);
            } else {
                lines.push(split_words(line));
            }
        }
        Paragraph { lines }
    }

    fn line_cells(line: &[String]) -> usize {
        line.iter().map(|word| cells(word)).sum()
    }

    /// `Measure`: the longest word, and the longest line (at most `width`).
    fn measure(&self, width: usize) -> (usize, usize) {
        let longest_word = self.lines.iter().flat_map(|line| line.iter().map(|word| cells(word))).max().unwrap_or(0);
        let longest_line = self.lines.iter().map(|line| Self::line_cells(line)).max().unwrap_or(0);
        (longest_word, longest_line.min(width))
    }

    /// `SplitLines`: the words laid into lines of at most `width` cells.
    fn split_lines(&self, width: usize) -> Vec<Vec<String>> {
        if width == 0 {
            return Vec::new();
        }
        let longest = self.lines.iter().map(|line| Self::line_cells(line)).max().unwrap_or(0);
        if longest <= width {
            return self.lines.clone();
        }
        // `SegmentLineIterator`: the words, a line break between lines.
        let mut stream: Vec<Word> = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            if index > 0 && !line.is_empty() {
                stream.push(None);
            }
            for word in line {
                stream.push(Some(word.clone()));
            }
        }
        let mut lines: Vec<Vec<String>> = Vec::new();
        let mut line: Vec<String> = Vec::new();
        let mut queue: std::collections::VecDeque<String> = std::collections::VecDeque::new();
        let mut words = stream.into_iter();
        loop {
            let word = match queue.pop_front() {
                Some(queued) => Some(queued),
                None => match words.next() {
                    Some(next) => next,
                    None => break,
                },
            };
            let Some(word) = word else {
                lines.push(std::mem::take(&mut line));
                continue;
            };
            let mut broke = false;
            let word_cells = cells(&word);
            let line_cells = Self::line_cells(&line);
            if word_cells > width {
                let parts = fold(&word, width);
                if line_cells + cells(&parts[0]) > width {
                    lines.push(std::mem::take(&mut line));
                    queue.extend(parts);
                } else {
                    let mut rest = parts.into_iter();
                    if let Some(first) = rest.next() {
                        line.push(first);
                    }
                    queue.extend(rest);
                }
                continue;
            } else if line_cells + word_cells > width {
                line.push(String::new());
                lines.push(std::mem::take(&mut line));
                broke = true;
            }
            if !broke || !is_white_space(&word) {
                line.push(word);
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        lines
    }

    /// The paragraph's lines at `width`, each aligned.
    fn render(&self, width: usize, justify: Justify) -> Vec<String> {
        let lines = self.split_lines(width);
        lines.iter().map(|line| align(&line.concat(), justify, width)).collect()
    }
}

/// `Segment.SplitSegment`: a word too long for a line in pieces of
/// `width` cells.
fn fold(word: &str, width: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut current_cells = 0;
    for character in word.chars() {
        if current_cells + 1 > width {
            parts.push(std::mem::take(&mut current));
            current_cells = 0;
        }
        current_cells += 1;
        current.push(character);
    }
    parts.push(current);
    parts
}

/// `TextPath`: a path shortened to fit, its middle folders replaced by an
/// ellipsis.
#[derive(Clone, Debug)]
struct TextPath {
    parts: Vec<String>,
    rooted: bool,
    windows: bool,
}

impl TextPath {
    fn new(path: &str) -> TextPath {
        let path = path.replace('\\', "/");
        let path = path.trim_end_matches('/').trim().to_string();
        let mut parts: Vec<String> = path.split('/').filter(|part| !part.is_empty()).map(str::to_string).collect();
        let mut rooted = false;
        let mut windows = false;
        if path.starts_with('/') {
            rooted = true;
            parts.insert(0, "/".to_string());
        } else if parts.first().is_some_and(|first| first.ends_with(':')) {
            rooted = true;
            windows = true;
        }
        TextPath { parts, rooted, windows }
    }

    /// `Fit`: the parts that fit in `width`.
    fn fit(&self, width: usize) -> Vec<String> {
        let count = self.parts.len();
        if count == 0 {
            return Vec::new();
        }
        let whole: usize = self.parts.iter().map(|part| cells(part)).sum::<usize>() + (count - 1);
        if whole <= width {
            return self.parts.clone();
        }
        let ellipsis = "…";
        let last = &self.parts[count - 1];
        if count >= 2 {
            let skip = usize::from(self.rooted);
            let kept = if self.rooted { 2 } else { 1 };
            let root_cells = if self.rooted { cells(&self.parts[0]) } else { 0 };
            let mut queue: std::collections::VecDeque<&String> = self.parts.iter().skip(skip).take(count - kept).collect();
            while !queue.is_empty() {
                queue.pop_front();
                let middle: usize = queue.iter().map(|part| cells(part)).sum();
                let needed = root_cells + 1 + middle + cells(last) + queue.len() + kept;
                if width >= needed {
                    let mut fitted = Vec::new();
                    if self.rooted {
                        fitted.push(self.parts[0].clone());
                    }
                    fitted.push(ellipsis.to_string());
                    fitted.extend(queue.iter().map(|part| (*part).clone()));
                    fitted.push(last.clone());
                    return fitted;
                }
            }
        }
        let last_chars: Vec<char> = last.chars().collect();
        let shown = last_chars.len().min(width.saturating_sub(1));
        let start = last_chars.len() - shown;
        let tail: String = last_chars[start..].iter().collect();
        vec![format!("{ellipsis}{tail}")]
    }

    fn measure(&self, width: usize) -> (usize, usize) {
        let fitted = self.fit(width);
        let separators = fitted.len().saturating_sub(1);
        let length = fitted.iter().map(|part| cells(part)).sum::<usize>() + separators;
        let length = length.min(width);
        (length, length)
    }

    fn render(&self, width: usize) -> String {
        let fitted = self.fit(width);
        let mut text = String::new();
        let count = fitted.len();
        for (index, part) in fitted.iter().enumerate() {
            let last = index + 1 == count;
            if last {
                text.push_str(part);
            } else if index == 0 && self.rooted {
                text.push_str(part);
                if self.windows {
                    text.push('/');
                }
            } else {
                text.push_str(part);
                text.push('/');
            }
        }
        text
    }
}

/// What a table cell holds.
#[derive(Clone, Debug)]
enum Content {
    Text(Paragraph),
    Path(TextPath),
    Table(Box<Table>),
}

impl Content {
    fn text(text: &str) -> Content {
        Content::Text(Paragraph::new(text))
    }

    fn measure(&self, width: usize) -> (usize, usize) {
        match self {
            Content::Text(paragraph) => paragraph.measure(width),
            Content::Path(path) => path.measure(width),
            Content::Table(table) => table.measure(width),
        }
    }

    fn render(&self, width: usize, justify: Justify) -> Vec<String> {
        match self {
            Content::Text(paragraph) => paragraph.render(width, justify),
            Content::Path(path) => vec![path.render(width)],
            Content::Table(table) => table.render(width),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Column {
    justify: Justify,
    /// Left and right padding.
    padding: (usize, usize),
}

/// `Table` with the `Minimal` border and its headers hidden, or a `Grid`
/// (no border; the last column's right padding left out).
#[derive(Clone, Debug)]
struct Table {
    columns: Vec<Column>,
    rows: Vec<Vec<Content>>,
    bordered: bool,
    title: Option<String>,
    caption: Option<String>,
}

impl Table {
    fn new(bordered: bool) -> Table {
        Table {
            columns: Vec::new(),
            rows: Vec::new(),
            bordered,
            title: None,
            caption: None,
        }
    }

    fn grid() -> Table {
        Table::new(false)
    }

    fn add_column(&mut self, justify: Justify, padding: (usize, usize)) {
        self.columns.push(Column { justify, padding });
    }

    fn add_row(&mut self, row: Vec<Content>) {
        self.rows.push(row);
    }

    fn add_text_row(&mut self, label: &str, value: Content) {
        self.add_row(vec![Content::text(label), value]);
    }

    /// Whether a column's right padding counts (`PadRightCell`, which a
    /// grid of default columns turns off for its last column).
    fn pads_right(&self, index: usize) -> bool {
        self.bordered || index + 1 < self.columns.len()
    }

    /// `GetNonColumnWidth`: the separators, the edges and the padding.
    fn non_column_width(&self) -> usize {
        let separators = if self.bordered { self.columns.len().saturating_sub(1) + 2 } else { 0 };
        let mut padding = 0;
        for (index, column) in self.columns.iter().enumerate() {
            padding += column.padding.0;
            if self.pads_right(index) {
                padding += column.padding.1;
            }
        }
        separators + padding
    }

    /// `MeasureColumn`: the widest of its cells' shortest and longest
    /// widths (the hidden header counts as empty).
    fn measure_column(&self, index: usize, width: usize) -> (usize, usize) {
        let mut minimum = 0;
        let mut maximum = 0;
        for row in &self.rows {
            if let Some(cell) = row.get(index) {
                let (cell_minimum, cell_maximum) = cell.measure(width);
                minimum = minimum.max(cell_minimum);
                maximum = maximum.max(cell_maximum);
            }
        }
        (minimum, maximum)
    }

    fn measure(&self, width: usize) -> (usize, usize) {
        let cell_width = width.saturating_sub(self.non_column_width());
        let mut minimum = 0;
        let mut maximum = 0;
        for index in 0..self.columns.len() {
            let (column_minimum, column_maximum) = self.measure_column(index, cell_width);
            minimum += column_minimum;
            maximum += column_maximum;
        }
        (minimum + self.non_column_width(), maximum + self.non_column_width())
    }

    /// `CalculateColumnWidths`.
    fn column_widths(&self, width: usize) -> Vec<usize> {
        let mut widths: Vec<usize> = (0..self.columns.len()).map(|index| self.measure_column(index, width).1).collect();
        let total: usize = widths.iter().sum();
        if total > width {
            widths = collapse_widths(widths, width);
            let total: usize = widths.iter().sum();
            if total > width {
                let ones = vec![1; widths.len()];
                widths = ratio_reduce(total - width, &ones, &widths.clone(), &widths);
            }
        }
        widths
    }

    /// The table's lines at most `width` cells wide (`TableRenderer.Render`).
    fn render(&self, width: usize) -> Vec<String> {
        let non_column = self.non_column_width();
        let cell_width = width.saturating_sub(non_column);
        let widths = self.column_widths(cell_width);
        let table_width = widths.iter().sum::<usize>() + non_column;
        if table_width == 0 || table_width > width {
            return vec!["…".to_string()];
        }
        let mut lines = Vec::new();
        if let Some(title) = &self.title {
            lines.extend(Paragraph::new(title).render(table_width, Justify::Center));
        }
        let row_count = self.rows.len();
        for (row_index, row) in self.rows.iter().enumerate() {
            let mut cells_lines: Vec<Vec<String>> = Vec::new();
            for (index, column_width) in widths.iter().enumerate() {
                let justify = self.columns[index].justify;
                let rendered = row.get(index).map(|cell| cell.render(*column_width, justify)).unwrap_or_default();
                cells_lines.push(rendered);
            }
            let height = cells_lines.iter().map(Vec::len).max().unwrap_or(0).max(1);
            if row_index == 0 && self.bordered {
                lines.push(self.border_row(&widths));
            }
            for line_index in 0..height {
                let mut line = String::new();
                for (index, cell_lines) in cells_lines.iter().enumerate() {
                    let column = self.columns[index];
                    let first = index == 0;
                    let last = index + 1 == cells_lines.len();
                    if first && self.bordered {
                        line.push(' ');
                    }
                    line.push_str(&" ".repeat(column.padding.0));
                    let content = cell_lines.get(line_index).map(String::as_str).unwrap_or("");
                    line.push_str(content);
                    let content_cells = cells(content);
                    if content_cells < widths[index] {
                        line.push_str(&" ".repeat(widths[index] - content_cells));
                    }
                    if self.pads_right(index) {
                        line.push_str(&" ".repeat(column.padding.1));
                    }
                    if self.bordered {
                        line.push_str(if last { " " } else { "│" });
                    }
                }
                lines.push(line);
            }
            if row_index + 1 == row_count && self.bordered {
                lines.push(self.border_row(&widths));
            }
        }
        if let Some(caption) = &self.caption {
            lines.extend(Paragraph::new(caption).render(table_width, Justify::Center));
        }
        lines
    }

    /// The `Minimal` border's top and bottom rows: spaces across.
    fn border_row(&self, widths: &[usize]) -> String {
        let mut cells_across = 2 + widths.len().saturating_sub(1);
        for (index, width) in widths.iter().enumerate() {
            cells_across += self.columns[index].padding.0 + width + self.columns[index].padding.1;
        }
        " ".repeat(cells_across)
    }
}

/// `TableMeasurer.CollapseWidths`: the widest columns narrowed, down to
/// the next widest, until the widths fit.
fn collapse_widths(mut widths: Vec<usize>, width: usize) -> Vec<usize> {
    let mut total: usize = widths.iter().sum();
    let mut excess = total as i64 - width as i64;
    while total != 0 && excess > 0 {
        let widest = *widths.iter().max().unwrap_or(&0);
        let second = widths.iter().map(|&column| if column == widest { 1 } else { column }).max().unwrap_or(1);
        let difference = widest as i64 - second as i64;
        let ratios: Vec<usize> = widths.iter().map(|&column| usize::from(column == widest)).collect();
        if ratios.iter().all(|&ratio| ratio == 0) || difference == 0 {
            break;
        }
        let maximum = excess.min(difference) as usize;
        let maximums = vec![maximum; widths.len()];
        widths = ratio_reduce(excess as usize, &ratios, &maximums, &widths);
        total = widths.iter().sum();
        excess = total as i64 - width as i64;
    }
    widths
}

/// `Ratio.Reduce`: `total` taken from the values in proportion to the
/// ratios, each by at most its maximum.
fn ratio_reduce(total: usize, ratios: &[usize], maximums: &[usize], values: &[usize]) -> Vec<usize> {
    let ratios: Vec<usize> = ratios.iter().zip(maximums).map(|(&ratio, &maximum)| if maximum > 0 { ratio } else { 0 }).collect();
    let mut ratio_total: usize = ratios.iter().sum();
    if ratio_total == 0 {
        return values.to_vec();
    }
    let mut remaining = total;
    let mut reduced = Vec::with_capacity(values.len());
    for ((&ratio, &maximum), &value) in ratios.iter().zip(maximums).zip(values) {
        if ratio != 0 && ratio_total > 0 {
            // `Math.Round` of an integer division: the quotient itself.
            let share = ratio * remaining / ratio_total;
            let taken = maximum.min(share);
            reduced.push(value.saturating_sub(taken));
            remaining -= taken;
            ratio_total -= ratio;
        } else {
            reduced.push(value);
        }
    }
    reduced
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn report() -> BuildReport {
        let start = Local.with_ymd_and_hms(2026, 10, 13, 0, 59, 56).unwrap() + chrono::Duration::nanoseconds(646_555_000);
        let end = start + chrono::Duration::nanoseconds(3_416_267_000);
        BuildReport {
            resolution: 256,
            node_count: 5,
            result: "AlreadyBuilt".to_string(),
            start_time: start,
            end_time: end,
            seconds_taken: vec![
                ("Mountain".to_string(), milliseconds_to_ticks(2125)),
                ("Thermal".to_string(), milliseconds_to_ticks(524)),
                ("CLUTer".to_string(), milliseconds_to_ticks(104)),
                ("Export".to_string(), milliseconds_to_ticks(1)),
                ("Height".to_string(), 0),
            ],
        }
    }

    #[test]
    fn positions_format_as_dotnet() {
        assert_eq!(fixed_three(0.25), "0.250");
        assert_eq!(fixed_three(0.1), "0.100");
        assert_eq!(fixed_three(0.0625), "0.063");
        assert_eq!(fixed_three(0.0), "0.000");
        assert_eq!(fixed_three(12.3456), "12.346");
        assert_eq!(fixed_three(0.0004), "0.000");
    }

    #[test]
    fn time_spans_format_as_dotnet() {
        assert_eq!(constant_time_span(34_162_670), "00:00:03.4162670");
        assert_eq!(constant_time_span(0), "00:00:00");
        assert_eq!(general_time_span(21_250_000), "0:00:02.125");
        assert_eq!(general_time_span(0), "0:00:00");
    }

    #[test]
    fn json_matches_the_report_layout() {
        let json = report_json(&report());
        let offset = report().start_time.format("%:z").to_string();
        let expected = format!(
            "{{\n  \"$id\": \"1\",\n  \"Resolution\": 256,\n  \"NodeCount\": 5,\n  \"Result\": \"AlreadyBuilt\",\n  \"StartTime\": \"2026-10-13T00:59:56.646555{offset}\",\n  \"EndTime\": \"2026-10-13T01:00:00.062822{offset}\",\n  \"Duration\": \"00:00:03.4162670\",\n  \"SecondsTaken\": {{\n    \"$id\": \"2\",\n    \"$values\": [\n      {{\n        \"$id\": \"3\",\n        \"Item1\": \"Mountain\",\n        \"Item2\": \"00:00:02.1250000\"\n      }},\n      {{\n        \"$id\": \"4\",\n        \"Item1\": \"Thermal\",\n        \"Item2\": \"00:00:00.5240000\"\n      }},\n      {{\n        \"$id\": \"5\",\n        \"Item1\": \"CLUTer\",\n        \"Item2\": \"00:00:00.1040000\"\n      }},\n      {{\n        \"$id\": \"6\",\n        \"Item1\": \"Export\",\n        \"Item2\": \"00:00:00.0010000\"\n      }},\n      {{\n        \"$id\": \"7\",\n        \"Item1\": \"Height\"\n      }}\n    ]\n  }},\n  \"FastestNode\": {{\n    \"$ref\": \"7\"\n  }},\n  \"SlowestNode\": {{\n    \"$ref\": \"3\"\n  }}\n}}"
        );
        assert_eq!(json, expected);
    }

    #[test]
    fn empty_lists_are_written_as_empty_values() {
        let mut bucketed = report();
        bucketed.resolution = 0;
        bucketed.result = "Success".to_string();
        bucketed.seconds_taken.clear();
        let json = report_json(&bucketed);
        assert!(json.contains("\"SecondsTaken\": {\n    \"$id\": \"2\",\n    \"$values\": []\n  }\n}"));
        assert!(!json.contains("Resolution"));
        assert!(!json.contains("FastestNode"));
    }

    #[test]
    fn grid_lays_out_as_spectre() {
        let lines = report_grid(&report()).render(CONSOLE_WIDTH);
        let expected = [
            "              Build Results                             Statistics              ",
            "                                                                                ",
            "          Result: │ AlreadyBuilt             Mountain │ 0:00:02.125             ",
            "      Resolution: │ 256                       Thermal │ 0:00:00.524             ",
            "      Node Count: │ 5                          CLUTer │ 0:00:00.104             ",
            "      Start Time: │ 10/13/2026 00:59:56               │                         ",
            "        End Time: │ 10/13/2026 01:00:00       Fastest │ Height @ 0:00:00        ",
            "        Duration: │ 00:00:03.4162670          Slowest │ Mountain @ 0:00:02.125  ",
            "  Non-Build Time: │ 00:00:00.6622669                                            ",
            "                                               * nodes that took <100ms are     ",
            "                                                         excluded.              ",
        ];
        assert_eq!(lines, expected);
    }

    #[test]
    fn header_wraps_long_folders_and_shortens_paths() {
        let header = ReportHeader {
            project_path: "/private/tmp/claude-501/-Users-danemadsen-nerthus/22df8798-8458-4e65-94e6-14307a882075/scratchpad/report/basic.terrain".to_string(),
            folder: "/private/tmp/claude-501/-Users-danemadsen-nerthus/22df8798-8458-4e65-94e6-14307a882075/scratchpad/report/maps".to_string(),
            build_type: "Standard".to_string(),
            resolution: 256,
            profile: None,
            nodes: 5,
            exporting: 4,
            tiles: None,
            region: None,
        };
        let text = header_text(&header);
        let lines: Vec<&str> = text.split(NEWLINE).collect();
        assert_eq!(lines[0], "");
        assert_eq!(lines[1].chars().count(), CONSOLE_WIDTH);
        assert!(lines[1].ends_with(" NERTHUS BUILD  0.1.0 ──"));
        assert_eq!(lines[3], " ".repeat(80));
        assert_eq!(lines[4], "          Filename:  │ /…/scratchpad/report/basic.terrain                       ");
        assert_eq!(lines[5], "    Build Location:  │ /private/tmp/claude-501/-Users-danemadsen-nerthus/22df8  ");
        assert_eq!(lines[6], "                     │ 798-8458-4e65-94e6-14307a882075/scratchpad/report/maps   ");
        assert_eq!(lines[8], "  Build Resolution:  │ 256 × 256                                                ");
        assert_eq!(lines[10], "             Nodes:  │ 05 to build                                              ");
        assert_eq!(lines[11], "                     │ 04 to export                                             ");
    }
}
