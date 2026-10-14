//! The documentation window (Help › User Guide and Node Reference, a
//! node's help, and the help links around the editor): the contents on
//! the left, or what a search found; the page on the right; back, forward
//! and home above it. Links to other pages open here, links to the web in
//! the browser. The mouse's back and forward buttons go back and forward.

use std::collections::HashSet;

use egui::{Align, Id, Layout, PointerButton, Rect, RichText, Sense, Stroke, Ui, Vec2};

use crate::docs::{self, SearchHit, Target};
use crate::icons;
use crate::theme;
use crate::ui::markdown::{self, Document, View};
use crate::ui::tool_window;

/// The contents' width as the window opens.
const CONTENTS_WIDTH: f32 = 280.0;
/// A contents line's height and how far each level is indented.
const LINE_HEIGHT: f32 = 24.0;
const LEVEL_INDENT: f32 = 14.0;
/// The pseudo-page showing the documentation's licence.
const LICENSE_PAGE: &str = "LICENSE.txt";

/// A place in the documentation: a page and the heading on it.
#[derive(Clone, Debug, PartialEq)]
struct Place {
    page: String,
    anchor: Option<String>,
}

/// The window while it is open.
pub struct DocsWindow {
    place: Place,
    document: Document,
    /// The heading still to be scrolled to.
    scroll_to: Option<String>,
    back: Vec<Place>,
    forward: Vec<Place>,
    search: String,
    /// What the search found, and the text it was for.
    hits: Vec<SearchHit>,
    searched: String,
    /// The contents lines whose children show.
    open_groups: HashSet<usize>,
    /// A link that went nowhere, said above the page.
    problem: Option<String>,
}

impl DocsWindow {
    /// The window on `target` (a page, with `#heading` if wanted).
    pub fn open(target: &str) -> DocsWindow {
        let place = place_of(target);
        let mut window = DocsWindow {
            place: place.clone(),
            document: Document::default(),
            scroll_to: None,
            back: Vec::new(),
            forward: Vec::new(),
            search: String::new(),
            hits: Vec::new(),
            searched: String::new(),
            open_groups: HashSet::new(),
            problem: None,
        };
        window.load(place);
        window
    }

    /// Opens `target`, the page shown now going on the back list.
    pub fn go(&mut self, target: &str) {
        let place = place_of(target);
        self.visit(place);
    }

    /// The search box holding `query`, its results listed.
    pub fn search_for(&mut self, query: &str) {
        self.search = query.to_string();
    }

    fn visit(&mut self, place: Place) {
        if place == self.place {
            self.scroll_to = place.anchor.clone();
            return;
        }
        let previous = std::mem::replace(&mut self.place, place.clone());
        self.back.push(previous);
        self.forward.clear();
        self.load(place);
    }

    fn go_back(&mut self) {
        let Some(place) = self.back.pop() else {
            return;
        };
        let current = std::mem::replace(&mut self.place, place.clone());
        self.forward.push(current);
        self.load(place);
    }

    fn go_forward(&mut self) {
        let Some(place) = self.forward.pop() else {
            return;
        };
        let current = std::mem::replace(&mut self.place, place.clone());
        self.back.push(current);
        self.load(place);
    }

    /// The page parsed, scrolled to its heading, and its contents lines
    /// opened.
    fn load(&mut self, place: Place) {
        self.problem = None;
        let text = if place.page == LICENSE_PAGE {
            format!("# Documentation Licence\n\n```\n{}\n```\n", docs::LICENSE.trim_end())
        } else {
            match docs::library().page(&place.page) {
                Some(text) => text.to_string(),
                None => format!("# Page not found\n\nThe documentation has no page `{}`.\n", place.page),
            }
        };
        self.document = markdown::parse(&text);
        self.scroll_to = place.anchor.clone();
        let library = docs::library();
        let contents = library.contents();
        for entry in library.trail(&place.page) {
            if let Some(index) = contents.iter().position(|candidate| std::ptr::eq(candidate, entry)) {
                self.open_groups.insert(index);
            }
        }
        self.place = place;
    }

    /// A link clicked on the page.
    fn follow(&mut self, href: &str) {
        match docs::resolve(&self.place.page, href) {
            Target::Page { path, anchor } => self.visit(Place { page: path, anchor }),
            Target::Licence => self.visit(Place { page: LICENSE_PAGE.to_string(), anchor: None }),
            Target::External(address) => crate::export_build::open_url(&address),
            Target::Missing(path) => self.problem = Some(format!("The documentation has no page {path}.")),
        }
    }
}

/// `path#heading` as a place.
fn place_of(target: &str) -> Place {
    match target.split_once('#') {
        Some((page, anchor)) => Place { page: page.to_string(), anchor: Some(anchor.to_string()) },
        None => Place { page: target.to_string(), anchor: None },
    }
}

/// The window. Returns whether it is still open.
pub fn show(ctx: &egui::Context, docs: &mut DocsWindow) -> bool {
    let mut open = true;
    let mut window = egui::Window::new("Nerthus Documentation");
    window = window.collapsible(false);
    window = window.resizable(true);
    window = window.min_size(Vec2::new(640.0, 360.0));
    window = tool_window::place(ctx, window, Id::new("documentation"), Vec2::new(1080.0, 760.0));
    window = window.open(&mut open);
    let shown = window.show(ctx, |ui| {
        toolbar(ui, docs);
        ui.add_space(4.0);
        ui.separator();
        let mut contents = egui::Panel::left("documentation-contents");
        contents = contents.resizable(true);
        contents = contents.default_size(CONTENTS_WIDTH);
        contents = contents.size_range(180.0..=480.0);
        contents.show(ui, |ui| {
            if docs.search.trim().is_empty() {
                contents_tree(ui, docs);
            } else {
                search_results(ui, docs);
            }
        });
        egui::CentralPanel::default().show(ui, |ui| page(ui, docs));
    });
    // The mouse's back and forward buttons, over the window.
    if let Some(shown) = shown {
        let (back, forward) = ctx.input(|input| (input.pointer.button_pressed(PointerButton::Extra1), input.pointer.button_pressed(PointerButton::Extra2)));
        if shown.response.contains_pointer() {
            if back {
                docs.go_back();
            }
            if forward {
                docs.go_forward();
            }
        }
    }
    open
}

/// Back, forward and home, where the page is, and the search box.
fn toolbar(ui: &mut Ui, docs: &mut DocsWindow) {
    ui.horizontal(|ui| {
        if icon_button(ui, "chevron-left", "Back", !docs.back.is_empty()).clicked() {
            docs.go_back();
        }
        if icon_button(ui, "chevron-right", "Forward", !docs.forward.is_empty()).clicked() {
            docs.go_forward();
        }
        if icon_button(ui, "home-simple", "Home", true).clicked() {
            docs.go(docs::HOME);
        }
        ui.add_space(8.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut field = egui::TextEdit::singleline(&mut docs.search);
            field = field.hint_text("Search the documentation");
            field = field.desired_width(260.0);
            field = field.font(theme::interface(12.0));
            ui.add(field);
            ui.add_space(8.0);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let trail = docs::library().trail(&docs.place.page);
                let titles: Vec<&str> = trail.iter().map(|entry| entry.title.as_str()).collect();
                let text = if titles.is_empty() { docs::library().title(&docs.place.page) } else { titles.join("  ›  ") };
                let label = egui::Label::new(RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT));
                ui.add(label.truncate());
            });
        });
    });
}

/// A 24-point button with an icon, dimmed when it can't be used.
fn icon_button(ui: &mut Ui, icon: &str, tooltip: &str, enabled: bool) -> egui::Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(24.0), sense);
    if enabled && response.hovered() {
        ui.painter().rect_filled(rect, 3.0, theme::MENU_BACKGROUND_HOVER);
    }
    let tint = if enabled { theme::MENU_ICON } else { theme::ICON_DARKISH };
    icons::paint(ui, rect.shrink(4.0), icon, tint);
    response.on_hover_text(tooltip)
}

/// The contents: groups open and close with their chevrons, the page shown
/// is lit.
fn contents_tree(ui: &mut Ui, docs: &mut DocsWindow) {
    let library = docs::library();
    let contents = library.contents();
    let mut scroll = egui::ScrollArea::vertical();
    scroll = scroll.id_salt("documentation-contents-scroll");
    scroll = scroll.auto_shrink([false, false]);
    scroll.show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        // Lines below a closed group are skipped until one as shallow.
        let mut hidden_below: Option<usize> = None;
        let mut clicked = None;
        for (index, entry) in contents.iter().enumerate() {
            if let Some(depth) = hidden_below {
                if entry.depth > depth {
                    continue;
                }
                hidden_below = None;
            }
            let is_group = contents.get(index + 1).is_some_and(|next| next.depth > entry.depth);
            let is_open = docs.open_groups.contains(&index);
            if is_group && !is_open {
                hidden_below = Some(entry.depth);
            }
            let current = entry.page.as_deref() == Some(docs.place.page.as_str());
            let response = contents_line(ui, &entry.title, entry.depth, is_group, is_open, current);
            if response.clicked() {
                // The chevron folds; the title opens the page, if any.
                let chevron_right = response.rect.left() + chevron_left(entry.depth) + 14.0;
                let on_chevron = response.interact_pointer_pos().is_some_and(|pointer| pointer.x < chevron_right);
                clicked = Some((index, is_group, on_chevron));
            }
        }
        let Some((index, is_group, on_chevron)) = clicked else {
            return;
        };
        let page = contents[index].page.as_ref();
        match page {
            Some(page) if !(is_group && on_chevron) => {
                docs.go(page);
                if is_group {
                    docs.open_groups.insert(index);
                }
            }
            _ if docs.open_groups.contains(&index) => {
                docs.open_groups.remove(&index);
            }
            _ => {
                docs.open_groups.insert(index);
            }
        }
    });
}

/// Where a line's chevron starts, from the line's left.
fn chevron_left(depth: usize) -> f32 {
    4.0 + depth as f32 * LEVEL_INDENT
}

/// A contents line: its chevron when it holds others, its title.
fn contents_line(ui: &mut Ui, title: &str, depth: usize, is_group: bool, is_open: bool, current: bool) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, LINE_HEIGHT), Sense::click());
    if current {
        ui.painter().rect_filled(rect, 3.0, theme::OPTIONS_SELECTION);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 3.0, theme::MENU_BACKGROUND_HOVER);
    }
    let left = rect.left() + chevron_left(depth);
    if is_group {
        let chevron = if is_open { "chevron-down" } else { "chevron-right" };
        let chevron_rect = Rect::from_center_size(egui::pos2(left + 6.0, rect.center().y), Vec2::splat(12.0));
        icons::paint(ui, chevron_rect, chevron, theme::ICON_DEFAULT);
    }
    let font = if depth == 0 { theme::semibold(12.0) } else { theme::interface(12.0) };
    let color = if current {
        theme::ACCENT_PRIMARY
    } else if response.hovered() || depth == 0 {
        theme::LABEL_EMPHASIS
    } else {
        theme::LABEL_DEFAULT
    };
    let text_left = left + 16.0;
    let mut job = egui::text::LayoutJob::simple_singleline(title.to_string(), font, color);
    job.wrap.max_width = (rect.right() - text_left - 4.0).max(10.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    let position = egui::pos2(text_left, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(position, galley, color);
    response
}

/// What the search found: each page's title, where it is, and the
/// words around the match.
fn search_results(ui: &mut Ui, docs: &mut DocsWindow) {
    let query = docs.search.trim().to_string();
    if query != docs.searched {
        docs.hits = docs::library().search(&query);
        docs.searched = query;
    }
    let mut scroll = egui::ScrollArea::vertical();
    scroll = scroll.id_salt("documentation-search-scroll");
    scroll = scroll.auto_shrink([false, false]);
    let mut chosen = None;
    scroll.show(ui, |ui| {
        if docs.hits.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new("No pages found.").font(theme::interface(12.0)).color(theme::LABEL_DEFAULT));
            return;
        }
        for hit in &docs.hits {
            let current = hit.path == docs.place.page;
            let width = ui.available_width();
            let title = ui.painter().layout(hit.title.clone(), theme::semibold(12.0), theme::LABEL_EMPHASIS, width - 12.0);
            let place = ui.painter().layout(hit.place.clone(), theme::interface(10.0), theme::MENU_SEPARATOR, width - 12.0);
            let snippet = ui.painter().layout(hit.snippet.clone(), theme::interface(11.0), theme::LABEL_DEFAULT, width - 12.0);
            let place_height = if hit.place.is_empty() { 0.0 } else { place.size().y + 1.0 };
            let height = title.size().y + place_height + snippet.size().y + 12.0;
            let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
            if current {
                ui.painter().rect_filled(rect, 3.0, theme::OPTIONS_SELECTION);
            } else if response.hovered() {
                ui.painter().rect_filled(rect, 3.0, theme::MENU_BACKGROUND_HOVER);
            }
            let title_color = if current { theme::ACCENT_PRIMARY } else { theme::LABEL_EMPHASIS };
            let title_height = title.size().y;
            ui.painter().galley(rect.min + Vec2::new(6.0, 4.0), title, title_color);
            if !hit.place.is_empty() {
                ui.painter().galley(rect.min + Vec2::new(6.0, 5.0 + title_height), place, theme::MENU_SEPARATOR);
            }
            let snippet_top = 6.0 + title_height + place_height;
            ui.painter().galley(rect.min + Vec2::new(6.0, snippet_top), snippet, theme::LABEL_DEFAULT);
            ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, theme::PANEL_CLADDING));
            if response.clicked() {
                chosen = Some(hit.path.clone());
            }
        }
    });
    if let Some(path) = chosen {
        docs.go(&path);
    }
}

/// The page, its links followed.
fn page(ui: &mut Ui, docs: &mut DocsWindow) {
    if let Some(problem) = &docs.problem {
        ui.label(RichText::new(problem.as_str()).font(theme::interface(12.0)).color(theme::ACCENT_MUTED_RED));
    }
    let mut scroll = egui::ScrollArea::vertical();
    scroll = scroll.id_salt(("documentation-page", docs.place.page.as_str()));
    scroll = scroll.auto_shrink([false, false]);
    let mut clicked = None;
    scroll.show(ui, |ui| {
        ui.add_space(4.0);
        egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 24, top: 0, bottom: 32 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let view = View { scroll_to: docs.scroll_to.as_deref() };
            let drawn = markdown::draw(ui, &docs.document, view);
            if drawn.scrolled {
                docs.scroll_to = None;
            }
            clicked = drawn.clicked;
        });
    });
    // A heading that is not on the page is not waited for.
    if docs.scroll_to.is_some() {
        docs.scroll_to = None;
    }
    if let Some(href) = clicked {
        docs.follow(&href);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_goes_back_and_forward() {
        let mut window = DocsWindow::open(docs::HOME);
        window.go(docs::NODE_REFERENCE);
        assert_eq!(window.place.page, docs::NODE_REFERENCE);
        window.go_back();
        assert_eq!(window.place.page, docs::HOME);
        window.go_forward();
        assert_eq!(window.place.page, docs::NODE_REFERENCE);
        assert!(window.forward.is_empty());
    }

    #[test]
    fn links_open_their_pages() {
        let mut window = DocsWindow::open(docs::HOME);
        window.follow(docs::NODE_REFERENCE);
        assert_eq!(window.place.page, docs::NODE_REFERENCE);
        window.follow("no-such-page.md");
        assert!(window.problem.is_some());
        assert_eq!(window.place.page, docs::NODE_REFERENCE);
    }

    #[test]
    fn opening_a_page_opens_its_groups() {
        let window = DocsWindow::open(docs::REGIONS);
        let library = docs::library();
        let trail = library.trail(docs::REGIONS);
        assert!(trail.len() > 1);
        assert!(window.open_groups.len() >= trail.len() - 1);
    }
}
