//! Help's windows: the Shortcut Guide (Help › Shortcut Reference,
//! `ShortcutsWindow`), What's new in this version (`WhatsNewWindow`),
//! About Nerthus (`About`, also the title bar's version button), and what
//! Help › Check for updates found (`CheckForUpdates` and
//! `UpdateService.ShowUpdateDialog`), whose Download Update starts the
//! update's download.

use egui::{Align, Align2, Color32, Id, Layout, RichText, Sense, Stroke, Ui, Vec2};

use crate::theme;
use crate::ui::markdown;
use crate::updates::{UpdateAnswer, UpdateToken};

const SHORTCUTS: &str = include_str!("../../data/shortcuts.md");
const WHATS_NEW: &str = include_str!("../../data/whatsnew.md");
const WHATS_NEW_NOTES: &str = include_str!("../../data/whatsnew-notes.md");

/// The theme's `Accent-Orange`.
pub const ACCENT_ORANGE: Color32 = Color32::from_rgb(0xdc, 0x70, 0x4a);

/// What's New's tabs: `whatsnew.md` is the only page besides the notes, so
/// its tab reads "Change Log"; an empty tab spaces the notes off.
const WHATS_NEW_TABS: [SideTab; 3] = [
    SideTab { title: "Change Log", brush: theme::ACCENT_PRIMARY },
    SideTab { title: "", brush: theme::ACCENT_MUTED_TEAL },
    SideTab { title: "Notes", brush: theme::ACCENT_MUTED_TEAL },
];

/// The data folder's note of the version that last showed What's New by
/// itself.
const WHATS_NEW_FILE: &str = "whatsnew.dat";

/// Written the first time a version starts; from the next start on, the
/// status bar's memory readout is teal.
pub const ECO_FILE: &str = "eco.dat";

/// The help windows open.
#[derive(Default)]
pub struct HelpWindows {
    pub shortcuts: bool,
    /// What's New, on this tab.
    pub whats_new: Option<usize>,
    /// What's New showed by itself: closing it notes the version.
    pub whats_new_first_run: bool,
    pub about: bool,
    pub update: Option<UpdateDialog>,
    /// Download Update was clicked: the release to download.
    pub download_requested: Option<UpdateToken>,
}

impl HelpWindows {
    /// Whether one of the windows is up (they take the keyboard, as modal
    /// dialogs do).
    pub fn any_open(&self) -> bool {
        self.shortcuts || self.whats_new.is_some() || self.about || self.update.is_some()
    }
}

/// An update check's answer on screen.
pub struct UpdateDialog {
    answer: UpdateAnswer,
    /// The release notes' expander.
    notes_open: bool,
}

impl UpdateDialog {
    pub fn new(answer: UpdateAnswer) -> UpdateDialog {
        UpdateDialog { answer, notes_open: false }
    }

    /// A made-up newer release with its notes showing (`--show
    /// updates:sample`, for checking the dialog).
    pub fn sample() -> UpdateDialog {
        let token = UpdateToken {
            version: "9.9.9".to_string(),
            date: "1/2/2027".to_string(),
            size: Some(123_456_789),
            url: crate::updates::releases_page(),
            notes: WHATS_NEW.to_string(),
            download: Some("https://example.com/nerthus-9.9.9.zip".to_string()),
        };
        UpdateDialog { answer: UpdateAnswer::Available(token), notes_open: true }
    }
}

/// A tab of `WhatsNewTabs`: its header and the brush it shows on when
/// selected.
#[derive(Clone, Copy, Debug)]
pub struct SideTab {
    pub title: &'static str,
    pub brush: Color32,
}

/// Draws the windows open.
pub fn show(ctx: &egui::Context, help: &mut HelpWindows) {
    if help.shortcuts && !shortcuts_window(ctx) {
        help.shortcuts = false;
    }
    if let Some(tab) = help.whats_new.as_mut()
        && !whats_new_window(ctx, tab)
    {
        help.whats_new = None;
        if std::mem::take(&mut help.whats_new_first_run) {
            note_version_shown();
        }
    }
    if help.about && !about_window(ctx) {
        help.about = false;
    }
    if let Some(dialog) = help.update.as_mut() {
        let (open, download) = update_window(ctx, dialog);
        if download.is_some() {
            help.download_requested = download;
        }
        if !open {
            help.update = None;
        }
    }
}

/// A borderless dialog (`Window-Dark` on `Menu-Border`,
/// rounded, with a shadow), centred at this size or the screen's less a
/// margin.
pub fn dialog_window(ctx: &egui::Context, id: &str, wanted: Vec2) -> egui::Window<'static> {
    let screen = ctx.content_rect().size();
    let size = Vec2::new(wanted.x.min(screen.x - 40.0), wanted.y.min(screen.y - 40.0));
    let mut frame = egui::Frame::new();
    frame = frame.fill(theme::WINDOW_DARK);
    frame = frame.stroke(Stroke::new(1.0, theme::MENU_BORDER));
    frame = frame.corner_radius(6.0);
    // `DropShadowEffect`: blur 20, depth 5 straight down, half opaque.
    let shadow = egui::Shadow { offset: [0, 5], blur: 20, spread: 0, color: Color32::from_black_alpha(128) };
    frame = frame.shadow(shadow);
    let mut window = egui::Window::new(id.to_string());
    window = window.id(Id::new(id.to_string()));
    window = window.title_bar(false);
    window = window.resizable(false);
    window = window.collapsible(false);
    window = window.frame(frame);
    window = window.fixed_size(size);
    window = window.anchor(Align2::CENTER_CENTER, Vec2::ZERO);
    window = window.order(egui::Order::Foreground);
    window
}

/// `Button-Important`: the accented button, its text centred inside this
/// padding, at least `min_width` wide.
pub fn important_button(ui: &mut Ui, text: &str, size: f32, min_width: f32, padding: Vec2) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::interface(size), theme::LABEL_EMPHASIS);
    let width = (galley.size().x + padding.x * 2.0).max(min_width);
    let height = galley.size().y + padding.y * 2.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let fill = if response.is_pointer_button_down_on() {
        theme::BUTTON_PRESSED
    } else if response.hovered() {
        theme::BUTTON_HOVER
    } else {
        theme::BUTTON_BACKGROUND
    };
    ui.painter().rect_filled(rect, 2.0, fill);
    ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, theme::BUTTON_IMPORTANT_BORDER), egui::StrokeKind::Inside);
    let position = rect.center() - galley.size() / 2.0;
    ui.painter().galley(position, galley, theme::LABEL_EMPHASIS);
    response
}

/// `WhatsNewTabs`' headers: stacked on the left, right-aligned in 18 px,
/// the selected one on its brush in black. An empty title only takes room.
pub fn side_tabs(ui: &mut Ui, tabs: &[SideTab], selected: &mut usize, width: f32) -> bool {
    let mut changed = false;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for (index, tab) in tabs.iter().enumerate() {
            let height = 8.0 + 24.0 + 8.0;
            let sense = if tab.title.is_empty() { Sense::hover() } else { Sense::click() };
            let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), sense);
            if tab.title.is_empty() {
                continue;
            }
            let chosen = *selected == index;
            if chosen {
                ui.painter().rect_filled(rect.shrink(2.0), 3.0, tab.brush);
            }
            let color = if chosen {
                Color32::BLACK
            } else if response.hovered() {
                theme::LABEL_EMPHASIS
            } else {
                theme::LABEL_DEFAULT
            };
            let galley = ui.painter().layout_no_wrap(tab.title.to_string(), theme::interface(18.0), color);
            let position = egui::pos2(rect.right() - 2.0 - 24.0 - galley.size().x, rect.center().y - galley.size().y / 2.0);
            ui.painter().galley(position, galley, color);
            if response.clicked() && !chosen {
                *selected = index;
                changed = true;
            }
        }
    });
    changed
}

/// Escape or Enter, which close a window whose button is both `IsCancel`
/// and `IsDefault`.
fn close_key(ctx: &egui::Context) -> bool {
    ctx.input(|input| input.key_pressed(egui::Key::Escape) || input.key_pressed(egui::Key::Enter))
}

/// The Shortcut Guide (`shortcuts.md`). Returns whether it stays open.
fn shortcuts_window(ctx: &egui::Context) -> bool {
    let mut open = !close_key(ctx);
    let window = dialog_window(ctx, "shortcut-guide", Vec2::new(680.0, 920.0));
    window.show(ctx, |ui| {
        let height = ui.available_height();
        ui.vertical_centered(|ui| {
            ui.add_space(16.0);
            ui.label(RichText::new("Shortcut Guide").font(theme::interface(32.0)).color(theme::LABEL_EMPHASIS));
            ui.add_space(16.0);
        });
        let footer = 18.0 + 2.0 * 8.0 + 8.0 + 32.0;
        let mut scroll = egui::ScrollArea::vertical();
        scroll = scroll.id_salt("shortcut-guide-text");
        scroll = scroll.max_height((height - 80.0 - footer).max(100.0));
        scroll = scroll.auto_shrink([false, false]);
        scroll.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(36.0);
                ui.vertical(|ui| {
                    ui.set_max_width(ui.available_width() - 36.0);
                    markdown::show(ui, SHORTCUTS);
                });
            });
        });
        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            if important_button(ui, "Close", 18.0, 192.0, Vec2::splat(8.0)).clicked() {
                open = false;
            }
        });
        ui.add_space(32.0);
    });
    open
}

/// What's new in this version (`whatsnew.md` and `whatsnew-notes.md`).
/// Returns whether it stays open.
fn whats_new_window(ctx: &egui::Context, tab: &mut usize) -> bool {
    let mut open = !close_key(ctx);
    let window = dialog_window(ctx, "whats-new", Vec2::new(1240.0, 880.0));
    window.show(ctx, |ui| {
        let size = ui.available_size();
        ui.add_space(24.0);
        let footer = 18.0 + 2.0 * 8.0 + 32.0 + 24.0;
        let body = Vec2::new(size.x - 48.0, (size.y - 24.0 - 16.0 - footer).max(100.0));
        ui.horizontal_top(|ui| {
            ui.add_space(24.0 + 32.0);
            ui.allocate_ui_with_layout(Vec2::new(164.0, body.y), Layout::top_down(Align::Min), |ui| {
                ui.add_space(36.0);
                side_tabs(ui, &WHATS_NEW_TABS, tab, 164.0);
            });
            ui.add_space(8.0 + 8.0);
            let text_width = body.x - 164.0 - 32.0 - 32.0 - 16.0;
            ui.allocate_ui_with_layout(Vec2::new(text_width, body.y), Layout::top_down(Align::Min), |ui| {
                let mut scroll = egui::ScrollArea::vertical();
                scroll = scroll.id_salt(("whats-new-text", *tab));
                scroll = scroll.auto_shrink([false, false]);
                scroll.show(ui, |ui| {
                    let text = if *tab == 2 { WHATS_NEW_NOTES } else { WHATS_NEW };
                    markdown::show(ui, text);
                });
            });
        });
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            if important_button(ui, "Close", 18.0, 192.0, Vec2::new(16.0, 8.0)).clicked() {
                open = false;
            }
        });
        ui.add_space(32.0);
    });
    open
}

/// `StartExperience.VersionFirstRun`: What's New shows by itself the first
/// time a version starts, when no version has shown it yet or an older one
/// did (`whatsnew.dat`); `eco.dat` is written then. A note that cannot be
/// read keeps it from showing.
pub fn version_first_run() -> bool {
    let Some(folder) = crate::user::data_folder() else {
        return false;
    };
    let noted = folder.join(WHATS_NEW_FILE);
    let first_run = if noted.exists() {
        let text = std::fs::read_to_string(&noted).unwrap_or_default();
        let seen = parse_version(&text);
        let current = parse_version(env!("CARGO_PKG_VERSION"));
        match (seen, current) {
            (Some(seen), Some(current)) => seen < current,
            _ => {
                log::error!("Cannot read the version in {WHATS_NEW_FILE}: {text:?}");
                false
            }
        }
    } else {
        true
    };
    if first_run {
        let written = std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(folder.join(ECO_FILE), "!"));
        if let Err(error) = written {
            log::warn!("cannot write {ECO_FILE}: {error}");
        }
    }
    first_run
}

/// `ShowWhatsNew`, once the window closes: this version showed it.
fn note_version_shown() {
    let Some(folder) = crate::user::data_folder() else {
        return;
    };
    let written = std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(folder.join(WHATS_NEW_FILE), env!("CARGO_PKG_VERSION")));
    if let Err(error) = written {
        log::error!("cannot write {WHATS_NEW_FILE}: {error}");
    }
}

/// A version's two to four numbers (`Version.Parse`), before any
/// pre-release or build suffix; earlier numbers count first, and a
/// missing one comes before 0.
fn parse_version(text: &str) -> Option<Vec<u64>> {
    let release = text.trim().split(['-', '+']).next().unwrap_or_default();
    let numbers: Option<Vec<u64>> = release.split('.').map(|part| part.parse().ok()).collect();
    numbers.filter(|numbers| (2..=4).contains(&numbers.len()))
}

/// The authors from the package, without their addresses.
fn authors() -> String {
    let mut names = Vec::new();
    for author in env!("CARGO_PKG_AUTHORS").split(':') {
        let name = author.split('<').next().unwrap_or(author);
        names.push(name.trim());
    }
    names.join(", ")
}

/// About Nerthus. Returns whether it stays open.
fn about_window(ctx: &egui::Context) -> bool {
    let mut open = true;
    let title = concat!("About Nerthus ", env!("CARGO_PKG_VERSION"));
    let mut window = egui::Window::new(title);
    window = window.id(Id::new("about-nerthus"));
    window = window.collapsible(false);
    window = window.resizable(false);
    window = window.fixed_size(Vec2::new(476.0, 600.0));
    window = window.anchor(Align2::CENTER_CENTER, Vec2::ZERO);
    window = window.order(egui::Order::Foreground);
    window = window.open(&mut open);
    let mut close = ctx.input(|input| input.key_pressed(egui::Key::Escape) || input.key_pressed(egui::Key::Enter));
    window.show(ctx, |ui| {
        ui.add_space(16.0);
        let logo = egui::Image::new(egui::include_image!("../../assets/nerthus.svg"));
        ui.vertical_centered(|ui| ui.add(logo.fit_to_exact_size(Vec2::splat(180.0))));
        ui.add_space(24.0);
        let caption = |text: &str| RichText::new(text).font(theme::semibold(10.0)).color(Color32::from_gray(0x80));
        ui.label(caption("Developed by"));
        ui.label(RichText::new(authors()).font(theme::interface(13.0)).color(theme::LABEL_DEFAULT));
        ui.add_space(12.0);
        ui.label(caption("Built with"));
        let built_with = "Bevy, egui, wgpu, resvg, the Roboto and Roboto Mono fonts, and palettes from Copernicus Sentinel-2 imagery";
        ui.add(egui::Label::new(RichText::new(built_with).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT)).wrap());
        ui.label(RichText::new(nerthus_nodes::colour_library::NOTICE).font(theme::interface(10.0)).color(Color32::from_gray(0x80)));
        ui.add_space(24.0);
        let rows = [("Version", env!("CARGO_PKG_VERSION")), ("License", env!("CARGO_PKG_LICENSE"))];
        for (name, value) in rows {
            ui.horizontal(|ui| {
                ui.label(RichText::new(name).font(theme::interface(12.0)).color(Color32::from_gray(0x80)));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(value).font(theme::monospace_bold(12.0)).color(Color32::from_gray(0x80)));
                });
            });
        }
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let link = RichText::new("Open source information").font(theme::interface(12.0)).color(theme::ACCENT_MUTED_GREEN);
            let response = ui.add(egui::Label::new(link).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand);
            if response.clicked() {
                let address = concat!(env!("CARGO_PKG_REPOSITORY"), "#license");
                crate::export_build::open_url(address);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if important_button(ui, "Close", 12.0, 128.0 + 30.0, Vec2::new(64.0, 8.0)).clicked() {
                    close = true;
                }
            });
        });
    });
    open && !close
}

/// `ByteSize.Bytes(size).ToString()`: the largest whole unit, to two
/// decimals at most.
fn byte_size(bytes: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < units.len() {
        value /= 1024.0;
        unit += 1;
    }
    let rounded = (value * 100.0).round() / 100.0;
    format!("{rounded} {}", units[unit])
}

/// The release notes as the expander shows them (`Notes`, cut at 1000
/// characters).
fn notes_text(token: &UpdateToken) -> String {
    let characters: Vec<char> = token.notes.chars().collect();
    if characters.len() <= 1000 {
        return token.notes.clone();
    }
    let start: String = characters[..1000].iter().collect();
    format!("{start}...\n(read more on download page)")
}

/// The check's answer as a task dialog. Returns whether it stays open,
/// and the release to download when Download Update was clicked.
fn update_window(ctx: &egui::Context, dialog: &mut UpdateDialog) -> (bool, Option<UpdateToken>) {
    let mut open = true;
    let mut download = None;
    let caption = match &dialog.answer {
        UpdateAnswer::Available(_) => "Update available!".to_string(),
        _ => concat!("Nerthus ", env!("CARGO_PKG_VERSION")).to_string(),
    };
    let modal = egui::Modal::new(Id::new("update-check"));
    let response = modal.show(ctx, |ui| {
        ui.set_width(460.0);
        ui.label(RichText::new(caption).font(theme::interface(11.0)).color(theme::LABEL_DEFAULT));
        ui.add_space(6.0);
        let heading = |ui: &mut Ui, text: &str| {
            ui.label(RichText::new(text).font(theme::semibold(15.0)).color(theme::ACCENT_MUTED_BLUE));
            ui.add_space(8.0);
        };
        let body = |ui: &mut Ui, text: &str| {
            ui.add(egui::Label::new(RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT)).wrap());
        };
        let link = |ui: &mut Ui, text: &str, address: &str| {
            let label = RichText::new(text).font(theme::interface(12.0)).color(theme::ACCENT_MUTED_GREEN);
            let response = ui.add(egui::Label::new(label).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand);
            if response.clicked() {
                crate::export_build::open_url(address);
            }
        };
        let releases = crate::updates::releases_page();
        match &dialog.answer {
            UpdateAnswer::UpToDate => {
                heading(ui, "You're up-to-date!");
                body(ui, "Congratulations! You're running the latest version of Nerthus with the most recent feature updates!\n\nIf you would like to re-install Nerthus, visit");
                link(ui, &releases, &releases);
                ui.add_space(12.0);
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    if ui.button("OK").clicked() {
                        open = false;
                    }
                });
            }
            UpdateAnswer::Failed(error) => {
                heading(ui, "Error loading update information.");
                body(ui, error);
                ui.add_space(12.0);
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    if ui.button("OK").clicked() {
                        open = false;
                    }
                });
            }
            UpdateAnswer::Available(token) => {
                heading(ui, &format!("Nerthus Version {} is available", token.version));
                // Without a download for this system, the page opens
                // instead.
                let question = if token.download.is_some() { "Would you like to download and install the update?" } else { "Would you like to download the update?" };
                let mut text = format!("{question}\n\n");
                if let Some(size) = token.size {
                    text.push_str(&format!("Size: {}\n", byte_size(size)));
                }
                text.push_str(&format!("Released date: {}", token.date));
                body(ui, &text);
                ui.add_space(8.0);
                let expander = egui::CollapsingHeader::new("Release Notes").id_salt("update-release-notes").open(Some(dialog.notes_open));
                let shown = expander.show(ui, |ui| {
                    let mut scroll = egui::ScrollArea::vertical();
                    scroll = scroll.max_height(240.0);
                    scroll.show(ui, |ui| markdown::show(ui, &notes_text(token)));
                });
                if shown.header_response.clicked() {
                    dialog.notes_open = !dialog.notes_open;
                }
                ui.add_space(12.0);
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    if ui.button("Cancel").clicked() {
                        open = false;
                    }
                    if important_button(ui, "Download Update", 12.0, 0.0, Vec2::new(12.0, 4.0)).clicked() {
                        if token.download.is_some() {
                            download = Some(token.clone());
                        } else {
                            crate::export_build::open_url(&token.url);
                        }
                        open = false;
                    }
                });
                ui.add_space(8.0);
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    link(ui, "Open download page", &releases);
                    body(ui, " to read more or download manually.");
                });
            }
        }
    });
    if response.should_close() {
        open = false;
    }
    (open, download)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_as_bytesize_does() {
        assert_eq!(byte_size(512), "512 B");
        assert_eq!(byte_size(1536), "1.5 KB");
        assert_eq!(byte_size(52_428_800), "50 MB");
    }

    #[test]
    fn authors_lose_their_addresses() {
        assert!(!authors().contains('<'));
    }

    #[test]
    fn versions_compare_number_by_number() {
        let older = parse_version("0.9.12\n");
        let newer = parse_version("0.10.0");
        assert!(older.is_some() && older < newer);
        assert!(parse_version("2.3") < parse_version("2.3.0"));
        assert_eq!(parse_version("1.2.0-beta"), parse_version("1.2.0"));
        assert_eq!(parse_version("1"), None);
        assert_eq!(parse_version("one.two"), None);
    }
}
