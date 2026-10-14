//! The windows File opens: the Start dialog (`StartExperience`), the
//! examples (`ExamplesWindow`) and Export Macro (`ExportMacroDialog`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use egui::{Align2, Color32, Id, Key, RichText, ScrollArea, Sense, Stroke, TextureHandle, Ui, Vec2};

use crate::editor::Editor;
use crate::files::{self, FileCommand, OpenMode};
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::options_window::OptionsState;
use crate::ui::tool_commands::{self, ToolCommand};
use crate::ui::UiState;

/// The Start dialog's size (`Width="770" Height="660"`) and its column of
/// buttons (164 wide, 64 from the right and 56 from the bottom).
const START_SIZE: Vec2 = Vec2::new(770.0, 660.0);
const START_COLUMN: f32 = 164.0;
/// The examples window (`Width="1280" Height="660"`, six columns).
const EXAMPLES_SIZE: Vec2 = Vec2::new(1280.0, 660.0);
const EXAMPLE_COLUMNS: usize = 6;
/// Thumbnails are kept at most this wide.
const THUMBNAIL_WIDTH: u32 = 400;
/// `CreateMRUButton(…, attemptThumbnail: count < 20)`.
const THUMBNAIL_LIMIT: usize = 19;

/// What the windows remember between frames.
#[derive(Clone, Default)]
struct WindowMemory {
    /// The Start dialog's search box.
    search: String,
    /// Thumbnails read so far (none when a file has none).
    thumbnails: HashMap<PathBuf, Option<TextureHandle>>,
    /// The documents folder's projects joined the searchable list
    /// (`ConsumeSuperMRU`, once a dialog).
    documents_read: bool,
}

fn memory(ctx: &egui::Context) -> WindowMemory {
    ctx.data_mut(|data| data.get_temp_mut_or_default::<WindowMemory>(Id::new("file-windows")).clone())
}

fn store(ctx: &egui::Context, memory: WindowMemory) {
    ctx.data_mut(|data| data.insert_temp(Id::new("file-windows"), memory));
}

/// The file windows that are open, and the files side's requests.
pub fn show(ctx: &egui::Context, editor: &mut Editor, ui_state: &mut UiState, commands: &mut Vec<Command>) {
    crate::ui::file_commands::take_requests(editor, ui_state, commands);
    if editor.files.start_dialog_open {
        start_dialog(ctx, editor, ui_state, commands);
    }
    if editor.files.examples_open {
        examples_window(ctx, editor, commands);
    }
    if editor.files.export_macro.is_some() {
        export_macro_window(ctx, editor, commands);
    }
}

/// A plain button (`Button`), or the accented one (`Button-Important`).
fn button(ui: &mut Ui, text: &str, important: bool, size: Vec2) -> egui::Response {
    let label = RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_EMPHASIS);
    let border = if important { theme::BUTTON_IMPORTANT_BORDER } else { theme::BUTTON_BORDER };
    let mut widget = egui::Button::new(label);
    widget = widget.fill(theme::BUTTON_BACKGROUND);
    widget = widget.stroke(Stroke::new(1.0, border));
    widget = widget.min_size(size);
    ui.add(widget)
}

/// A thumbnail as a texture, read once (JPEG, scaled down).
fn thumbnail(ctx: &egui::Context, memory: &mut WindowMemory, path: &Path) -> Option<TextureHandle> {
    if let Some(known) = memory.thumbnails.get(path) {
        return known.clone();
    }
    let texture = read_thumbnail(path).map(|image| ctx.load_texture(path.to_string_lossy(), image, egui::TextureOptions::LINEAR));
    memory.thumbnails.insert(path.to_path_buf(), texture.clone());
    texture
}

fn read_thumbnail(path: &Path) -> Option<egui::ColorImage> {
    let picture = open_picture(path)?;
    let picture = if picture.width() > THUMBNAIL_WIDTH { picture.resize(THUMBNAIL_WIDTH, u32::MAX, image::imageops::FilterType::Triangle) } else { picture };
    let rgba = picture.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

/// `CreateMRUButton`'s thumbnail: `<project>.jpg` beside it, else where
/// the File Thumbnails option keeps them.
fn project_thumbnail(editor: &Editor, project: &Path) -> Option<PathBuf> {
    let beside = project.with_extension("jpg");
    if beside.is_file() {
        return Some(beside);
    }
    let kept = files::thumbnail_path(project, editor.user.options.thumbnail_storage())?;
    kept.is_file().then_some(kept)
}

/// The menu on a file's button (`PopulateMRU`'s context menu).
fn open_menu(response: &egui::Response, path: &Path, example: bool, commands: &mut Vec<Command>) {
    response.context_menu(|ui| {
        crate::ui::widgets::menu_style(ui);
        let open = |mode| {
            if example {
                FileCommand::OpenExample { path: path.to_path_buf(), mode }
            } else {
                FileCommand::OpenDirect { path: path.to_path_buf(), mode }
            }
        };
        if crate::ui::widgets::menu_item(ui, None, None, "Force low resolution", "").clicked() {
            commands.push(Command::File(open(OpenMode::LowResolution)));
        }
        if crate::ui::widgets::menu_item(ui, None, None, "Open and suspend engine", "").clicked() {
            commands.push(Command::File(open(OpenMode::SuspendEngine)));
        }
        crate::ui::widgets::menu_separator(ui);
        if crate::ui::widgets::menu_item(ui, None, None, "Open Folder", "").clicked()
            && let Some(folder) = path.parent()
        {
            commands.push(Command::OpenFolder(folder.to_path_buf()));
        }
    });
}

/// The Start dialog: New Project, Open Project, the recent files (or,
/// while searching, every project seen whose name has the text), Open
/// Examples and the settings. Escape closes it.
fn start_dialog(ctx: &egui::Context, editor: &mut Editor, ui_state: &mut UiState, commands: &mut Vec<Command>) {
    let mut memory = memory(ctx);
    if !memory.documents_read {
        memory.documents_read = true;
        files::consume_documents(&editor.user.preferences.documents_folder());
    }
    let mut open = true;
    let title = concat!("Welcome to Nerthus ", env!("CARGO_PKG_VERSION"));
    let mut window = egui::Window::new(title);
    window = window.id(Id::new("start-dialog"));
    window = window.collapsible(false);
    window = window.resizable(false);
    window = window.fixed_size(START_SIZE);
    window = window.anchor(Align2::CENTER_CENTER, Vec2::ZERO);
    window = window.frame(egui::Frame::window(&ctx.global_style()).fill(theme::WINDOW_DARK));
    window = window.open(&mut open);
    window.show(ctx, |ui| {
        let area = ui.max_rect();
        // The logo, top left (at 82, 82).
        let logo = egui::Rect::from_min_size(area.min + Vec2::new(82.0, 82.0), Vec2::splat(64.0));
        let source = egui::ImageSource::Bytes { uri: "bytes://nerthus-logo.svg".into(), bytes: include_bytes!("../../assets/nerthus.svg").as_slice().into() };
        egui::Image::new(source).paint_at(ui, logo);
        let name = ui.painter().layout_no_wrap("NERTHUS".to_string(), theme::semibold(28.0), theme::ACCENT_PRIMARY);
        ui.painter().galley(egui::Pos2::new(logo.right() + 16.0, logo.center().y - name.size().y / 2.0), name, theme::ACCENT_PRIMARY);

        // The settings, bottom left.
        let settings_rect = egui::Rect::from_min_size(egui::Pos2::new(area.left() + 24.0, area.bottom() - 24.0 - 26.0), Vec2::splat(26.0));
        let settings = ui.put(settings_rect, egui::Button::new("").frame(false));
        icons::paint(ui, settings_rect.shrink(5.0), "settings-cog", theme::LABEL_DEFAULT);
        if settings.on_hover_text("Options").clicked() {
            editor.files.start_dialog_open = false;
            ui_state.options = Some(OptionsState::new(ui_state.last_options_tab));
        }
        // `NahimicWarning` beside it (`Button-IconText`, padding 16, 8).
        if crate::nahimic::detected() {
            let galley = ui.painter().layout_no_wrap("Nahimic Detected!".to_string(), theme::interface(10.0), theme::ACCENT_MUTED_MAGENTA);
            let width = 16.0 + 16.0 + 6.0 + galley.size().x + 16.0;
            let warning_corner = egui::Pos2::new(settings_rect.right() + 8.0, settings_rect.center().y - 16.0);
            let warning_rect = egui::Rect::from_min_size(warning_corner, Vec2::new(width, 32.0));
            let warning = ui.put(warning_rect, egui::Button::new("").frame(false));
            let icon_corner = egui::Pos2::new(warning_rect.left() + 16.0, warning_rect.center().y - 8.0);
            let icon_rect = egui::Rect::from_min_size(icon_corner, Vec2::splat(16.0));
            icons::paint(ui, icon_rect, "exclamation-circle", theme::ACCENT_MUTED_MAGENTA);
            let text_position = egui::Pos2::new(icon_rect.right() + 6.0, warning_rect.center().y - galley.size().y / 2.0);
            ui.painter().galley(text_position, galley, theme::ACCENT_MUTED_MAGENTA);
            if warning.clicked() {
                commands.push(Command::Tools(ToolCommand::ShowNahimicWarning));
            }
        }

        let column_height = 4.0 + 28.0 + 8.0 + 28.0 + 8.0 + 4.0 + 24.0 + 8.0 + 250.0 + 4.0 + 120.0;
        let column = egui::Rect::from_min_size(egui::Pos2::new(area.right() - 64.0 - START_COLUMN, area.bottom() - 56.0 - column_height), Vec2::new(START_COLUMN, column_height));
        let mut column_ui = ui.new_child(egui::UiBuilder::new().max_rect(column).layout(egui::Layout::top_down(egui::Align::Min)));
        start_column(&mut column_ui, ctx, editor, &mut memory, commands);
    });
    // Escape belongs to a window over the dialog (What's New, the Nahimic
    // warning) while one shows.
    let escaped = !tool_commands::modal_open(ui_state) && ctx.input(|input| input.key_pressed(Key::Escape));
    if !open || escaped {
        editor.files.start_dialog_open = false;
    }
    store(ctx, memory);
}

fn start_column(ui: &mut Ui, ctx: &egui::Context, editor: &mut Editor, memory: &mut WindowMemory, commands: &mut Vec<Command>) {
    let width = START_COLUMN;
    ui.add_space(4.0);
    if button(ui, "New Project", true, Vec2::new(width, 28.0)).clicked() {
        commands.push(Command::File(FileCommand::StartNew));
    }
    ui.add_space(8.0);
    if button(ui, "Open Project", false, Vec2::new(width, 28.0)).clicked() {
        commands.push(Command::File(FileCommand::StartOpen));
    }
    ui.add_space(12.0);
    let search = egui::TextEdit::singleline(&mut memory.search).hint_text("Search files...").desired_width(width);
    let search_response = ui.add(search);
    // Enter with a path typed opens it (`TxtSearch_KeyUp`).
    if search_response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
        let typed = memory.search.trim().trim_matches('"').to_string();
        let path = PathBuf::from(&typed);
        if path.is_file() {
            commands.push(Command::File(FileCommand::OpenDirect { path, mode: OpenMode::Normal }));
        }
    }
    ui.add_space(8.0);

    let searching = !memory.search.is_empty();
    let files: Vec<PathBuf> = if searching {
        let needle = memory.search.to_lowercase();
        let all = files::all_recent();
        all.into_iter().filter(|path| path.file_stem().is_some_and(|stem| stem.to_string_lossy().to_lowercase().contains(&needle))).collect()
    } else {
        editor.user.recent.files().to_vec()
    };
    let listed = searching || editor.user.options.populate_recent_on_start();
    ScrollArea::vertical().id_salt("start-recent").max_height(250.0).min_scrolled_height(250.0).show(ui, |ui| {
        ui.set_width(width);
        if !listed {
            return;
        }
        for (index, path) in files.iter().enumerate() {
            if !path.is_file() {
                continue;
            }
            let stem = path.file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_default();
            let label = RichText::new(stem).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT);
            let row = egui::Button::new(label).frame(false).min_size(Vec2::new(width, 22.0));
            let response = ui.add(row);
            let with_thumbnail = index < THUMBNAIL_LIMIT;
            let picture = if with_thumbnail { project_thumbnail(editor, path).and_then(|file| thumbnail(ctx, memory, &file)) } else { None };
            let response = response.on_hover_ui(|ui| {
                let name = path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
                ui.label(RichText::new(name).strong());
                if let Some(picture) = &picture {
                    let size = picture.size_vec2();
                    let shown = Vec2::new(320.0, 320.0 * size.y / size.x.max(1.0));
                    ui.add(egui::Image::new((picture.id(), shown)));
                }
                let folder = path.parent().map(|folder| folder.display().to_string()).unwrap_or_default();
                ui.label(RichText::new(folder).color(theme::LABEL_DEFAULT.gamma_multiply(0.5)));
            });
            if response.clicked() {
                commands.push(Command::File(FileCommand::OpenDirect { path: path.clone(), mode: OpenMode::Normal }));
            }
            open_menu(&response, path, false, commands);
        }
    });
    ui.add_space(4.0);
    if button(ui, "Open Examples...", false, Vec2::new(width, 28.0)).clicked() {
        editor.files.start_dialog_open = false;
        editor.files.examples_open = true;
    }
}

/// The example projects Nerthus comes with (assets/examples) and their
/// pictures, by file name.
pub(crate) const BUNDLED_EXAMPLES: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/example-files.rs"));

/// Where the examples are (`Paths.Root\Examples`): an Examples folder of
/// projects beside the program, else the one in the user's Nerthus folder,
/// which gets the examples Nerthus comes with.
pub fn examples_folder() -> Option<PathBuf> {
    let beside_program = std::env::current_exe().ok().and_then(|program| program.parent().map(|folder| folder.join("Examples")));
    // On a case-insensitive disk a build's `examples` folder answers to
    // `Examples` too, so it counts only when it holds projects.
    if let Some(folder) = beside_program.filter(|folder| holds_projects(folder)) {
        return Some(folder);
    }
    let in_data = crate::user::data_folder()?.join("Examples");
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| install_examples(&in_data));
    holds_projects(&in_data).then_some(in_data)
}

/// Writes the bundled examples into `folder`, those missing or changed
/// since (once a session; the user's own projects there stay).
fn install_examples(folder: &Path) {
    if std::fs::create_dir_all(folder).is_err() {
        return;
    }
    for (name, bytes) in BUNDLED_EXAMPLES {
        let path = folder.join(name);
        let current = std::fs::read(&path).ok();
        if current.as_deref() != Some(*bytes) {
            let _ = std::fs::write(&path, bytes);
        }
    }
}

fn holds_projects(folder: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return false;
    };
    entries.flatten().any(|entry| entry.path().extension().is_some_and(|extension| extension == "terrain"))
}

#[cfg(not(target_arch = "wasm32"))]
fn open_picture(path: &Path) -> Option<image::DynamicImage> {
    image::open(path).ok()
}

/// The web build's pictures are the examples' (`crate::web::read`).
#[cfg(target_arch = "wasm32")]
fn open_picture(path: &Path) -> Option<image::DynamicImage> {
    image::load_from_memory(&crate::web::read(path)?).ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn is_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(target_arch = "wasm32")]
fn is_file(path: &Path) -> bool {
    crate::web::exists(path)
}

/// The examples' projects, by name. The web build has no Examples folder:
/// its examples are the ones it carries, named by their file names.
#[cfg(target_arch = "wasm32")]
fn examples() -> Vec<PathBuf> {
    BUNDLED_EXAMPLES.iter().map(|(name, _)| PathBuf::from(name)).filter(|path| path.extension().is_some_and(|extension| extension == "terrain")).collect()
}

/// The examples' projects, by name.
#[cfg(not(target_arch = "wasm32"))]
fn examples() -> Vec<PathBuf> {
    let Some(folder) = examples_folder() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut projects: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|extension| extension == "terrain")).collect();
    projects.sort();
    projects
}

/// The examples (`ExamplesWindow`): a numbered button for each, with its
/// picture (`<example>.jpg`, else the folder's `null.jpg`) and name.
fn examples_window(ctx: &egui::Context, editor: &mut Editor, commands: &mut Vec<Command>) {
    let mut memory = memory(ctx);
    let mut open = true;
    let mut window = egui::Window::new("Examples");
    window = window.id(Id::new("examples-window"));
    window = window.collapsible(false);
    window = window.resizable(true);
    window = window.default_size(EXAMPLES_SIZE);
    window = window.anchor(Align2::CENTER_CENTER, Vec2::ZERO);
    window = window.frame(egui::Frame::window(&ctx.global_style()).fill(theme::WINDOW_DARK));
    window = window.open(&mut open);
    let projects = examples();
    window.show(ctx, |ui| {
        if projects.is_empty() {
            let text = "No examples found. Put an Examples folder of .terrain projects beside Nerthus or in the Nerthus settings folder.";
            ui.label(RichText::new(text).color(theme::LABEL_DEFAULT));
            return;
        }
        ScrollArea::vertical().id_salt("examples").show(ui, |ui| {
            let cell_width = (ui.available_width() / EXAMPLE_COLUMNS as f32 - 8.0).max(80.0);
            egui::Grid::new("examples-grid").spacing(Vec2::splat(8.0)).show(ui, |ui| {
                for (index, path) in projects.iter().enumerate() {
                    example_button(ui, ctx, &mut memory, path, index + 1, cell_width, commands);
                    if (index + 1) % EXAMPLE_COLUMNS == 0 {
                        ui.end_row();
                    }
                }
            });
        });
    });
    if !open {
        editor.files.examples_open = false;
    }
    store(ctx, memory);
}

fn example_button(ui: &mut Ui, ctx: &egui::Context, memory: &mut WindowMemory, path: &Path, number: usize, width: f32, commands: &mut Vec<Command>) {
    let beside = path.with_extension("jpg");
    let fallback = path.parent().map(|folder| folder.join("null.jpg"));
    let picture_file = if is_file(&beside) { Some(beside) } else { fallback.filter(|file| is_file(file)) };
    let picture = picture_file.and_then(|file| thumbnail(ctx, memory, &file));
    let image_height = picture.as_ref().map(|picture| width * picture.size_vec2().y / picture.size_vec2().x.max(1.0)).unwrap_or(width * 0.5625);
    let size = Vec2::new(width, image_height + 48.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = if response.hovered() { theme::BUTTON_HOVER } else { theme::BUTTON_BACKGROUND };
    ui.painter().rect_filled(rect, 2.0, fill);
    ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, theme::BUTTON_BORDER), egui::StrokeKind::Inside);
    let inner = rect.shrink(8.0);
    let number_text = format!("{number:02}");
    let number_color = Color32::from_white_alpha(26);
    ui.painter().text(inner.left_top() + Vec2::new(0.0, -8.0), Align2::LEFT_TOP, number_text, theme::semibold(48.0), number_color);
    if let Some(picture) = &picture {
        let image_rect = egui::Rect::from_min_size(inner.left_top() + Vec2::new(0.0, 16.0), Vec2::new(inner.width(), image_height - 16.0));
        egui::Image::new((picture.id(), image_rect.size())).paint_at(ui, image_rect);
    }
    let name = path.file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_default();
    ui.painter().text(egui::Pos2::new(inner.left(), inner.bottom()), Align2::LEFT_BOTTOM, name, theme::semibold(12.0), theme::LABEL_EMPHASIS);
    let file_name = path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
    let folder = path.parent().map(|folder| folder.display().to_string()).unwrap_or_default();
    let response = response.on_hover_text(format!("{file_name}\n\n{folder}"));
    if response.clicked() {
        commands.push(Command::File(FileCommand::OpenExample { path: path.to_path_buf(), mode: OpenMode::Normal }));
    }
    open_menu(&response, path, true, commands);
}

/// Export Macro (`ExportMacroDialog`): name, version, author, URL and
/// description, the id they make, and whether to save a file and install
/// the macro. Its button runs `BtnOK_OnClick`'s checks.
fn export_macro_window(ctx: &egui::Context, editor: &mut Editor, commands: &mut Vec<Command>) {
    let Some(mut form) = editor.files.export_macro.clone() else {
        return;
    };
    let mut open = true;
    let mut action = None;
    let mut window = egui::Window::new("Export Macro");
    window = window.id(Id::new("export-macro"));
    window = window.collapsible(false);
    window = window.resizable(false);
    window = window.default_width(720.0);
    window = window.anchor(Align2::CENTER_CENTER, Vec2::ZERO);
    window = window.open(&mut open);
    window.show(ctx, |ui| {
        ui.set_width(696.0);
        // The text's runs, with its bold ones.
        let runs = [
            ("Export your macro to a .macro file. Each macro is identified by its ", false),
            ("Macro ID", true),
            (".\n\nTo install a macro, drag and drop the .macro file into the Nerthus window. Exporting a macro also installs it automatically into your ", false),
            ("current macro collection", true),
            (".\n\n", false),
            ("Best practices:", true),
            ("\n• ", false),
            ("Keep the same Macro ID", true),
            (" when updating an existing macro (so it upgrades in place).\n• ", false),
            ("Use a new Macro ID", true),
            (" when you want a new macro to exist alongside an older version (so both can be installed).", false),
        ];
        let regular = egui::TextFormat { font_id: theme::interface(12.0), color: theme::LABEL_DEFAULT, ..Default::default() };
        let bold_font = egui::FontId::new(12.0, egui::FontFamily::Name(theme::FONT_BOLD.into()));
        let bold = egui::TextFormat { font_id: bold_font, color: theme::LABEL_DEFAULT, ..Default::default() };
        let mut intro = egui::text::LayoutJob::default();
        intro.wrap.max_width = ui.available_width();
        for (text, is_bold) in runs {
            let format = if is_bold { bold.clone() } else { regular.clone() };
            intro.append(text, 0.0, format);
        }
        ui.label(intro);
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);
        // Two equal columns (`UniformGrid Rows="2"`).
        ui.columns(2, |columns| {
            columns[0].label("Macro Name");
            columns[0].add(egui::TextEdit::singleline(&mut form.name).desired_width(f32::INFINITY));
            columns[1].label("Macro Version");
            columns[1].add(egui::TextEdit::singleline(&mut form.version).desired_width(f32::INFINITY));
        });
        ui.add_space(4.0);
        ui.columns(2, |columns| {
            columns[0].label("Author Name");
            columns[0].add(egui::TextEdit::singleline(&mut form.author).desired_width(f32::INFINITY));
            columns[1].label("URL");
            columns[1].add(egui::TextEdit::singleline(&mut form.url).desired_width(f32::INFINITY));
        });
        ui.add_space(8.0);
        ui.label("Description");
        ui.add(egui::TextEdit::singleline(&mut form.description).desired_width(f32::INFINITY));
        ui.add_space(16.0);
        let id_frame = egui::Frame::new().fill(theme::ACCENT_MUTED_GREEN.gamma_multiply(0.15)).stroke(Stroke::new(1.5, theme::ACCENT_MUTED_GREEN)).corner_radius(6.0).inner_margin(egui::Margin::symmetric(6, 4));
        id_frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            let preview = format!("Macro ID preview: {}", form.macro_id());
            ui.label(RichText::new(preview).font(theme::monospace(12.0)).color(theme::LABEL_EMPHASIS));
        });
        ui.add_space(12.0);
        ui.checkbox(&mut form.save_file, "Save as file");
        ui.add_space(8.0);
        ui.checkbox(&mut form.install, "Install in Nerthus");
        ui.add_space(16.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if button(ui, "Cancel", false, Vec2::new(90.0, 28.0)).clicked() {
                action = Some(false);
            }
            if button(ui, "Export Macro", true, Vec2::new(120.0, 28.0)).clicked() {
                action = Some(true);
            }
        });
    });
    editor.files.export_macro = Some(form.clone());
    if !open || action == Some(false) {
        form.remember(editor);
        editor.files.export_macro = None;
        return;
    }
    if action == Some(true) {
        export_macro_checks(editor, &form, commands);
    }
}

/// `BtnOK_OnClick`: an option must be chosen and the project must make a
/// macro; a macro of the same id asks before it is replaced.
fn export_macro_checks(editor: &mut Editor, form: &files::ExportForm, commands: &mut Vec<Command>) {
    if !form.save_file && !form.install {
        editor.files.show_message("Nerthus", "You must select an export option.");
        return;
    }
    if let Err(problem) = editor.document.can_export_macro() {
        editor.files.show_message("Nerthus", problem);
        return;
    }
    if editor.files.macros.get(&form.macro_id()).is_some() {
        let mut question = files::Message::new("Macro Exists", "A macro with the same ID exists in your current installation. Would you like to overwrite it?");
        question.footnote = Some("If you have a project open with this macro being used, you should reload that file for it to take effect.".to_string());
        question.yes = Some(FileCommand::ExportMacroChecked);
        editor.files.messages.push(question);
        return;
    }
    commands.push(Command::File(FileCommand::ExportMacroChecked));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_examples_install_and_open() {
        let folder = std::env::temp_dir().join(format!("nerthus-examples-{}", std::process::id()));
        install_examples(&folder);
        let projects: Vec<&str> = BUNDLED_EXAMPLES.iter().map(|(name, _)| *name).filter(|name| name.ends_with(".terrain")).collect();
        assert!(projects.len() >= 12, "{projects:?}");
        for project in &projects {
            let picture = project.replace(".terrain", ".jpg");
            assert!(folder.join(&picture).is_file(), "{picture}");
            let document = crate::document::Document::open(&folder.join(project));
            let document = document.unwrap_or_else(|error| panic!("{project}: {error:#}"));
            assert!(document.project.unknown_types.is_empty(), "{project}");
        }
        // A second install leaves them as they are; a changed one is put back.
        let first = folder.join(projects[0]);
        std::fs::write(&first, b"changed").unwrap();
        install_examples(&folder);
        assert_ne!(std::fs::read(&first).unwrap(), b"changed");
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
