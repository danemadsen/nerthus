//! The documentation: Markdown pages embedded from assets/docs (listed by
//! build.rs), with their table of contents and the page each node type's
//! help opens in data/docs.toml. Pages link to each other by relative
//! path (`../options/compute.md#available-devices`); links that leave
//! the documentation open in the browser. The help windows show the pages
//! (`ui::docs_window`); a crash while Nerthus starts writes them out as
//! HTML for the browser instead (`open_in_browser`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use pulldown_cmark::{CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::Deserialize;

/// The contents and the node pages.
const INDEX: &str = include_str!("../data/docs.toml");

/// Every page as (path under assets/docs, text), listed by build.rs.
const PAGES: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/doc-files.rs"));

/// The licence of the documentation the pages are adapted from.
pub const LICENSE: &str = include_str!("../assets/docs/LICENSE.txt");

/// Help › User Guide.
pub const HOME: &str = "index.md";
/// Help › Node Reference, and node help for a type without a page.
pub const NODE_REFERENCE: &str = "reference/index.md";
/// A parameter's Help with Automation.
pub const PROPERTY_AUTOMATION: &str = "ui/interface/property-editor/binding-variables.md";
/// "More Info" on the note that a node can add input ports.
pub const ADD_PORTS: &str = "ui/interface/graph/adding-ports.md";
/// Build Settings › Regions' Help.
pub const REGIONS: &str = "using/build-and-export/regions/index.md";
/// Build Settings › Script's "Learn more".
pub const POST_BUILD_SCRIPT: &str = "ui/interface/build-options/script.md";
/// Options › Compute's "Requires driver update".
pub const DRIVER_UPDATE: &str = "ui/interface/options/compute.md#driver-issues";
/// What a crash while Nerthus starts offers to open.
pub const STARTUP_TROUBLESHOOTING: &str = "using/troubleshooting/common-issues-and-workarounds.md";

/// The Markdown the pages are written in: CommonMark with tables,
/// strikethrough and GitHub's alerts (`> [!NOTE]`).
pub fn markdown_options() -> Options {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_GFM);
    options
}

/// A line of the table of contents: a page, or a heading for the lines
/// one deeper under it (which may have a page of its own).
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ContentsEntry {
    pub title: String,
    #[serde(default)]
    pub page: Option<String>,
    #[serde(default)]
    pub depth: usize,
}

#[derive(Deserialize)]
struct Index {
    contents: Vec<ContentsEntry>,
    #[serde(default)]
    nodes: HashMap<String, String>,
}

/// The pages, their contents and the node pages.
pub struct Library {
    pages: HashMap<&'static str, &'static str>,
    contents: Vec<ContentsEntry>,
    nodes: HashMap<String, String>,
    /// Each page's title: its line in the contents, else its first heading.
    titles: HashMap<String, String>,
    /// What searches look through, made on the first search.
    search_text: OnceLock<Vec<SearchText>>,
}

/// The documentation, read on first use.
pub fn library() -> &'static Library {
    static LIBRARY: OnceLock<Library> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        let index: Index = toml::from_str(INDEX).expect("the embedded documentation index is valid");
        let pages: HashMap<&'static str, &'static str> = PAGES.iter().copied().collect();
        let mut titles = HashMap::new();
        for (&path, &text) in &pages {
            if let Some(heading) = first_heading(text) {
                titles.insert(path.to_string(), heading);
            }
        }
        for entry in &index.contents {
            if let Some(page) = &entry.page {
                titles.insert(page.clone(), entry.title.clone());
            }
        }
        Library { pages, contents: index.contents, nodes: index.nodes, titles, search_text: OnceLock::new() }
    })
}

impl Library {
    /// The page's Markdown.
    pub fn page(&self, path: &str) -> Option<&'static str> {
        self.pages.get(path).copied()
    }

    /// What a page is called.
    pub fn title(&self, path: &str) -> String {
        match self.titles.get(path) {
            Some(title) => title.clone(),
            None => path.to_string(),
        }
    }

    pub fn contents(&self) -> &[ContentsEntry] {
        &self.contents
    }

    /// The page documenting a node type.
    pub fn node_page(&self, type_name: &str) -> Option<&str> {
        self.nodes.get(type_name).map(String::as_str)
    }

    /// The contents lines leading to a page (the page's line last).
    pub fn trail(&self, path: &str) -> Vec<&ContentsEntry> {
        let Some(position) = self.contents.iter().position(|entry| entry.page.as_deref() == Some(path)) else {
            return Vec::new();
        };
        let mut trail = vec![&self.contents[position]];
        let mut depth = self.contents[position].depth;
        for entry in self.contents[..position].iter().rev() {
            if depth == 0 {
                break;
            }
            if entry.depth < depth {
                trail.push(entry);
                depth = entry.depth;
            }
        }
        trail.reverse();
        trail
    }

    /// The pages that mention every word of `query`, best first: words in
    /// the title count most, then in headings, then in the text.
    pub fn search(&self, query: &str) -> Vec<SearchHit> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() {
            return Vec::new();
        }
        let phrase = words.join(" ");
        let texts = self.search_text.get_or_init(|| self.make_search_text());
        let mut hits = Vec::new();
        for text in texts {
            let mut score = 0;
            let mut found_all = true;
            for word in &words {
                let in_title = text.title.contains(word.as_str());
                let in_headings = text.headings.contains(word.as_str());
                let in_body = text.lower_body.contains(word.as_str());
                if !in_title && !in_headings && !in_body {
                    found_all = false;
                    break;
                }
                score += if in_title { 10 } else { 0 };
                score += if in_headings { 3 } else { 0 };
                score += if in_body { 1 } else { 0 };
            }
            if !found_all {
                continue;
            }
            if text.title.contains(&phrase) {
                score += 100;
            }
            let snippet = snippet(&text.body, &text.lower_body, &words[0]);
            let place = self.place(&text.path);
            hits.push(SearchHit { path: text.path.clone(), title: self.title(&text.path), place, snippet, score });
        }
        hits.sort_by(|left, right| right.score.cmp(&left.score).then_with(|| left.title.cmp(&right.title)));
        hits.truncate(SEARCH_LIMIT);
        hits
    }

    /// Where a page sits in the contents: the lines above it, from the
    /// top ("Node Reference › Nodes › Simulate").
    fn place(&self, path: &str) -> String {
        let trail = self.trail(path);
        let above = trail.split_last().map(|(_, above)| above).unwrap_or_default();
        let titles: Vec<&str> = above.iter().map(|entry| entry.title.as_str()).collect();
        titles.join(" › ")
    }

    fn make_search_text(&self) -> Vec<SearchText> {
        let mut texts = Vec::new();
        for (&path, &markdown) in &self.pages {
            let (body, headings) = plain_text(markdown);
            let lower_body = body.to_lowercase();
            let title = self.title(path).to_lowercase();
            texts.push(SearchText { path: path.to_string(), title, headings: headings.to_lowercase(), body, lower_body });
        }
        texts.sort_by(|left, right| left.path.cmp(&right.path));
        texts
    }
}

/// How many pages a search lists at most.
const SEARCH_LIMIT: usize = 60;

/// A page a search found.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchHit {
    pub path: String,
    pub title: String,
    /// Where the page is in the contents.
    pub place: String,
    /// The text around the first word found.
    pub snippet: String,
    score: u32,
}

/// A page as searches see it.
struct SearchText {
    path: String,
    title: String,
    headings: String,
    body: String,
    lower_body: String,
}

/// The page's first heading's text.
fn first_heading(markdown: &str) -> Option<String> {
    let mut in_heading = false;
    let mut heading = String::new();
    for event in Parser::new_ext(markdown, markdown_options()) {
        match event {
            Event::Start(Tag::Heading { .. }) => in_heading = true,
            Event::End(TagEnd::Heading(_)) => return Some(heading),
            Event::Text(text) | Event::Code(text) if in_heading => heading.push_str(&text),
            _ => {}
        }
    }
    None
}

/// The page's words without the Markdown (the title aside, which the
/// results show already), and its headings' words.
fn plain_text(markdown: &str) -> (String, String) {
    let mut body = String::new();
    let mut headings = String::new();
    let mut in_heading = false;
    let mut in_title = false;
    for event in Parser::new_ext(markdown, markdown_options()) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                in_heading = true;
                in_title = level == HeadingLevel::H1;
            }
            Event::End(TagEnd::Heading(_)) => {
                in_heading = false;
                headings.push('\n');
                if !in_title {
                    body.push('\n');
                }
                in_title = false;
            }
            Event::Text(text) | Event::Code(text) => {
                if in_heading {
                    headings.push_str(&text);
                }
                if !in_title {
                    body.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak => body.push(' '),
            Event::End(TagEnd::Paragraph | TagEnd::Item | TagEnd::TableCell) => body.push(' '),
            _ => {}
        }
    }
    (body, headings)
}

/// About 120 characters of `body` around the first `word`.
fn snippet(body: &str, lower_body: &str, word: &str) -> String {
    let Some(found) = lower_body.find(word) else {
        return String::new();
    };
    // Lower-casing keeps byte offsets for the ASCII the pages are written
    // in; elsewhere fall back to the start.
    let found = if body.is_char_boundary(found) { found } else { 0 };
    let mut start = found.saturating_sub(40);
    while !body.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (found + 80).min(body.len());
    while !body.is_char_boundary(end) {
        end += 1;
    }
    let mut text: String = body[start..end].split_whitespace().collect::<Vec<_>>().join(" ");
    if start > 0 {
        text.insert(0, '…');
    }
    if end < body.len() {
        text.push('…');
    }
    text
}

/// Where a link goes.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// A page, and the heading on it.
    Page { path: String, anchor: Option<String> },
    /// A web page (or mail), opened in the browser.
    External(String),
    /// The documentation's licence (`LICENSE.txt`).
    Licence,
    /// A page the documentation does not have.
    Missing(String),
}

/// Where `href` on the page `from` leads.
pub fn resolve(from: &str, href: &str) -> Target {
    let is_web = href.starts_with("http://") || href.starts_with("https://") || href.starts_with("mailto:");
    if is_web {
        return Target::External(href.to_string());
    }
    let (path_part, anchor) = match href.split_once('#') {
        Some((path, anchor)) => (path, Some(anchor.to_string())),
        None => (href, None),
    };
    if path_part.is_empty() {
        return Target::Page { path: from.to_string(), anchor };
    }
    if path_part.ends_with("LICENSE.txt") {
        return Target::Licence;
    }
    let folder = match from.rsplit_once('/') {
        Some((folder, _)) => folder,
        None => "",
    };
    let joined = if folder.is_empty() { path_part.to_string() } else { format!("{folder}/{path_part}") };
    let mut path = normalize(&joined);
    if path.is_empty() || path.ends_with('/') {
        path.push_str("index.md");
    }
    if library().page(&path).is_some() {
        return Target::Page { path, anchor };
    }
    Target::Missing(path)
}

/// `a/./b/../c` → `a/c`.
fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    let mut normalized = parts.join("/");
    if path.ends_with('/') && !normalized.is_empty() {
        normalized.push('/');
    }
    normalized
}

/// A heading's anchor (GitHub's): lower case, spaces as hyphens, only
/// letters, digits, hyphens and underscores kept.
pub fn slug(heading: &str) -> String {
    let mut slug = String::new();
    for character in heading.trim().chars() {
        if character.is_alphanumeric() || character == '-' || character == '_' {
            slug.extend(character.to_lowercase());
        } else if character == ' ' {
            slug.push('-');
        }
    }
    slug
}

/// Shows the page in the browser: the documentation is written out as
/// HTML to a folder of its own in the temporary folder (once a session)
/// and the page opened there.
pub fn open_in_browser(target: &str) {
    let (path, anchor) = match target.split_once('#') {
        Some((path, anchor)) => (path, Some(anchor)),
        None => (target, None),
    };
    let folder = std::env::temp_dir().join(concat!("nerthus-docs-", env!("CARGO_PKG_VERSION")));
    static WRITTEN: OnceLock<std::io::Result<()>> = OnceLock::new();
    let written = WRITTEN.get_or_init(|| write_html(&folder));
    if let Err(error) = written {
        log::error!("Cannot write the documentation to {}: {error}", folder.display());
        return;
    }
    let page = folder.join(html_path(path));
    let mut address = format!("file://{}", page.display().to_string().replace('\\', "/"));
    if !address.starts_with("file:///") {
        address = address.replacen("file://", "file:///", 1);
    }
    if let Some(anchor) = anchor {
        address.push('#');
        address.push_str(anchor);
    }
    crate::export_build::open_url(&address);
}

/// `ui/index.md` → `ui/index.html`.
fn html_path(page: &str) -> PathBuf {
    let html = match page.strip_suffix(".md") {
        Some(stem) => format!("{stem}.html"),
        None => page.to_string(),
    };
    PathBuf::from(html)
}

/// Every page as an HTML file under `folder`, its links to other pages
/// pointing at their HTML and its headings carrying their anchors.
fn write_html(folder: &Path) -> std::io::Result<()> {
    let library = library();
    for &(path, markdown) in PAGES {
        let title = library.title(path);
        let body = page_html(markdown);
        let depth = path.matches('/').count();
        let mut back = String::new();
        for _ in 0..depth {
            back.push_str("../");
        }
        let document = format!(
            "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>{} - Nerthus Documentation</title>\n<style>{HTML_STYLE}</style>\n</head>\n<body>\n<nav><a href=\"{back}index.html\">Nerthus Documentation</a></nav>\n<main>\n{body}</main>\n</body>\n</html>\n",
            escape_html(&title)
        );
        let file = folder.join(html_path(path));
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(file, document)?;
    }
    std::fs::write(folder.join("LICENSE.txt"), LICENSE)?;
    Ok(())
}

/// The page's Markdown as HTML.
fn page_html(markdown: &str) -> String {
    let mut events: Vec<Event> = Parser::new_ext(markdown, markdown_options()).collect();
    // Headings carry their anchors; links to pages go to their HTML.
    let mut index = 0;
    while index < events.len() {
        if let Event::Start(Tag::Heading { level, .. }) = &events[index] {
            let level: HeadingLevel = *level;
            let mut text = String::new();
            for event in &events[index + 1..] {
                match event {
                    Event::End(TagEnd::Heading(_)) => break,
                    Event::Text(part) | Event::Code(part) => text.push_str(part),
                    _ => {}
                }
            }
            let id = CowStr::from(slug(&text));
            events[index] = Event::Start(Tag::Heading { level, id: Some(id), classes: Vec::new(), attrs: Vec::new() });
        }
        if let Event::Start(Tag::Link { link_type, dest_url, title, id }) = &events[index] {
            let is_page = !dest_url.contains("://") && !dest_url.starts_with("mailto:");
            if is_page {
                let rewritten = dest_url.replacen(".md", ".html", 1);
                let link = Tag::Link { link_type: *link_type, dest_url: CowStr::from(rewritten), title: title.clone(), id: id.clone() };
                events[index] = Event::Start(link);
            }
        }
        index += 1;
    }
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, events.into_iter());
    html
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The written pages' look: the editor's dark greys and accents.
const HTML_STYLE: &str = "body{margin:0;background:#1b1b1b;color:#c8c8c8;font:15px/1.6 system-ui,sans-serif}nav{padding:12px 32px;background:#262626;border-bottom:1px solid #363636}main{max-width:960px;margin:0 auto;padding:16px 32px 64px}a{color:#8fbf7f}h1,h2,h3,h4{color:#f0f0f0;font-weight:600}code,pre{font-family:ui-monospace,monospace;background:#101010;border:1px solid #363636;border-radius:3px}code{padding:0 4px}pre{padding:12px;overflow:auto}pre code{border:0;padding:0}table{border-collapse:collapse;width:100%}th,td{border-bottom:1px solid #363636;padding:6px 8px;text-align:left}blockquote{margin:16px 0;padding:4px 16px;border-left:4px solid #4a90c8;background:#202830}";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_index_and_the_pages_agree() {
        let library = library();
        for entry in library.contents() {
            if let Some(page) = &entry.page {
                assert!(library.page(page).is_some(), "the contents list {page}, which is not in assets/docs");
            }
        }
        for (type_name, page) in &library.nodes {
            assert!(library.page(page).is_some(), "{type_name}'s page {page} is not in assets/docs");
        }
    }

    #[test]
    fn every_page_is_in_the_contents() {
        let library = library();
        let listed: Vec<&str> = library.contents().iter().filter_map(|entry| entry.page.as_deref()).collect();
        for &(path, _) in PAGES {
            assert!(listed.contains(&path), "{path} is not in the contents (data/docs.toml)");
        }
    }

    #[test]
    fn the_editor_links_lead_to_pages() {
        let links = [HOME, NODE_REFERENCE, PROPERTY_AUTOMATION, ADD_PORTS, REGIONS, POST_BUILD_SCRIPT, DRIVER_UPDATE, STARTUP_TROUBLESHOOTING];
        for link in links {
            match resolve("", link) {
                Target::Page { path, anchor } => {
                    let text = library().page(&path).unwrap();
                    if let Some(anchor) = anchor {
                        let (_, headings) = plain_text(text);
                        let found = headings.lines().any(|heading| slug(heading) == anchor);
                        assert!(found, "{path} has no heading #{anchor}");
                    }
                }
                other => panic!("{link} leads to {other:?}"),
            }
        }
    }

    #[test]
    fn every_link_between_pages_resolves() {
        let mut broken = Vec::new();
        for &(path, markdown) in PAGES {
            for event in Parser::new_ext(markdown, markdown_options()) {
                let Event::Start(Tag::Link { dest_url, .. }) = event else {
                    continue;
                };
                match resolve(path, &dest_url) {
                    Target::Page { path: target, anchor: Some(anchor) } => {
                        let (_, headings) = plain_text(library().page(&target).unwrap());
                        if !headings.lines().any(|heading| slug(heading) == anchor) {
                            broken.push(format!("{path}: {dest_url} (no heading #{anchor})"));
                        }
                    }
                    Target::Missing(target) => broken.push(format!("{path}: {dest_url} ({target} is missing)")),
                    _ => {}
                }
            }
        }
        assert!(broken.is_empty(), "broken links:\n{}", broken.join("\n"));
    }

    #[test]
    fn links_resolve_from_the_page_they_are_on() {
        assert_eq!(normalize("a/b/../c/./d.md"), "a/c/d.md");
        assert_eq!(resolve("ui/index.md", "https://example.com"), Target::External("https://example.com".to_string()));
        let same_page = resolve("ui/index.md", "#viewport");
        assert_eq!(same_page, Target::Page { path: "ui/index.md".to_string(), anchor: Some("viewport".to_string()) });
    }

    #[test]
    fn slugs_follow_github() {
        assert_eq!(slug("Menus and Fast Access"), "menus-and-fast-access");
        assert_eq!(slug("2D Viewport (Docked)"), "2d-viewport-docked");
        assert_eq!(slug("Build_Tokens & More"), "build_tokens--more");
    }

    #[test]
    fn searches_put_titles_first() {
        let hits = library().search("erosion");
        assert!(!hits.is_empty());
        assert!(hits[0].title.to_lowercase().contains("erosion"));
    }

    #[test]
    fn search_hits_say_where_they_are() {
        let hits = library().search("transpose");
        let node = hits.iter().find(|hit| hit.path == "reference/nodes/modify/transpose.md").expect("the node's page");
        assert_eq!(node.place, "Node Reference › Nodes › Modify");
        // The title shows above the snippet, so the snippet leaves it out.
        assert!(node.snippet.starts_with("Transpose takes the character"));
    }
}
