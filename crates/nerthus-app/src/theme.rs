//! The dark theme (`UI/Themes/Colors/Dark.xaml`, `Styles/_Global.xaml`) as
//! egui colours, fonts and widget visuals.

use std::sync::Arc;

use egui::epaint::text::VariationCoords;
use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, Visuals};

/// `#rrggbb` as a colour, usable in constants.
const fn rgb(value: u32) -> Color32 {
    let red = (value >> 16) as u8;
    let green = (value >> 8) as u8;
    let blue = value as u8;
    Color32::from_rgb(red, green, blue)
}

/// `#aarrggbb` as a colour, usable in constants.
const fn argb(value: u32) -> Color32 {
    let alpha = (value >> 24) as u8;
    let red = (value >> 16) as u8;
    let green = (value >> 8) as u8;
    let blue = value as u8;
    Color32::from_rgba_unmultiplied_const(red, green, blue, alpha)
}

pub const ACCENT_PRIMARY: Color32 = rgb(0xffc900);
pub const ACCENT_BRIGHT: Color32 = rgb(0xddedf2);
pub const ACCENT_BLUE: Color32 = rgb(0x38a6ff);
pub const ACCENT_GREEN: Color32 = rgb(0x6d9258);
pub const ACCENT_YELLOW: Color32 = rgb(0xcacd46);
pub const ACCENT_RED: Color32 = rgb(0xdc4a4a);
pub const ACCENT_MAGENTA: Color32 = rgb(0xd94f91);
pub const ACCENT_PURPLE: Color32 = rgb(0x6f459e);
pub const ACCENT_TEAL: Color32 = rgb(0x47b4b3);
pub const ACCENT_GRAY: Color32 = rgb(0xa2c5d2);
pub const ACCENT_STRONG_RED: Color32 = rgb(0x932121);
pub const ACCENT_STRONG_YELLOW: Color32 = rgb(0xa8ab31);
pub const ACCENT_STRONG_GREEN: Color32 = rgb(0x537a3d);
pub const ACCENT_STRONG_ORANGE: Color32 = rgb(0x933f21);
pub const ACCENT_STRONG_BLUE: Color32 = rgb(0x1969aa);
pub const ACCENT_TRANSPARENT_ORANGE: Color32 = argb(0x66dc704a);
pub const ACCENT_TRANSPARENT_BLUE: Color32 = argb(0x6638a6ff);
pub const ACCENT_TRANSPARENT_GREEN: Color32 = argb(0x666d9258);
pub const ACCENT_TRANSPARENT_TEAL: Color32 = argb(0x6647b4b3);
pub const ACCENT_TRANSPARENT_YELLOW: Color32 = argb(0x66cacd46);
pub const ACCENT_TRANSPARENT_RED: Color32 = argb(0x66dc4a4a);
pub const ACCENT_TRANSPARENT_PURPLE: Color32 = argb(0x666f459e);
/// `Button-Important-Border`.
pub const BUTTON_IMPORTANT_BORDER: Color32 = rgb(0x1492e6);
/// `ParamsA-Color1` to `4`: Options › Compute marks each device type with
/// one (CPU, CPU-accelerated, integrated GPU, discrete GPU).
pub const PARAMS_A: [Color32; 4] = [rgb(0xe4ba4a), rgb(0xa9b871), rgb(0x6eb699), rgb(0x47b4b3)];
pub const ACCENT_MUTED_BLUE: Color32 = rgb(0x77a8cf);
pub const ACCENT_MUTED_GRAY: Color32 = rgb(0x566876);
pub const ACCENT_MUTED_GREEN: Color32 = rgb(0x95cc7a);
pub const ACCENT_MUTED_ORANGE: Color32 = rgb(0xc28a75);
pub const ACCENT_MUTED_RED: Color32 = rgb(0xbc7878);
pub const ACCENT_MUTED_MAGENTA: Color32 = rgb(0xcf81a6);
pub const ACCENT_MUTED_PURPLE: Color32 = rgb(0x7c6495);
pub const ACCENT_MUTED_PRIMARY: Color32 = rgb(0xc0ac66);
pub const PORTAL_FOREGROUND: Color32 = rgb(0x181818);
pub const ACCENT_MUTED_TEAL: Color32 = rgb(0x68bdbd);

pub const LABEL_DEFAULT: Color32 = rgb(0x909090);
pub const LABEL_EMPHASIS: Color32 = rgb(0xdddddd);
pub const ICON_DEFAULT: Color32 = rgb(0x787878);
/// `Icon-Darkish`.
pub const ICON_DARKISH: Color32 = rgb(0x4f4f4f);

pub const BUTTON_BACKGROUND: Color32 = rgb(0x202020);
pub const BUTTON_BORDER: Color32 = rgb(0x343434);
pub const BUTTON_HOVER: Color32 = rgb(0x181818);
pub const ICON_DARK: Color32 = rgb(0x282828);
pub const OPTIONS_SELECTION: Color32 = rgb(0x3f3f3f);
pub const BUTTON_PRESSED: Color32 = rgb(0x111111);

pub const TEXTBOX_BACKGROUND: Color32 = rgb(0x101010);
pub const TEXTBOX_FOCUSED_BORDER: Color32 = rgb(0x1492e6);
/// `TextBox-Border`.
pub const TEXTBOX_BORDER: Color32 = rgb(0x363636);
pub const TEXTBOX_FILE_BORDER: Color32 = rgb(0x424242);
pub const LISTBOX_BACKGROUND: Color32 = rgb(0x161616);
pub const LISTBOX_BORDER: Color32 = rgb(0x181818);
pub const TEXTBOX_FILE_FOREGROUND: Color32 = rgb(0x1492e6);
pub const TOGGLE_INDICATOR: Color32 = rgb(0x202020);
pub const TOGGLE_BACKGROUND: Color32 = rgb(0x4f4f4f);

pub const MENU_BORDER: Color32 = rgb(0x2f2f2f);
pub const MENU_FOREGROUND: Color32 = rgb(0xe0e0e0);
pub const MENU_SEPARATOR: Color32 = rgb(0x4f4f4f);
pub const MENU_BACKGROUND: Color32 = rgb(0x202020);
pub const MENU_BACKGROUND_HOVER: Color32 = rgb(0x353535);

pub const EXPANDER_HEADER: Color32 = rgb(0x909090);
/// `Expander`'s fill: the property groups' panels.
pub const EXPANDER_BACKGROUND: Color32 = rgb(0x303030);
pub const TOOLBOX_BUTTON_BACKGROUND: Color32 = rgb(0x232323);

/// `SliderX`'s bar (`border` rectangle).
pub const SLIDER_BACKGROUND: Color32 = rgb(0x292929);
pub const SLIDER_BORDER: Color32 = rgb(0x424242);
/// The slider's right-click nudge bar (`Nudge-Background`).
pub const NUDGE_BACKGROUND: Color32 = argb(0x99111111);

/// `ListBox-OptionsX` and its items.
pub const OPTIONS_BACKGROUND: Color32 = rgb(0x282828);
pub const OPTIONS_BORDER: Color32 = rgb(0x303030);
pub const OPTIONS_LABEL: Color32 = rgb(0x939393);

/// `DropdownX` (`Dropdown-*`) and its list's hover (`ComboBoxItem`).
pub const DROPDOWN_BACKGROUND: Color32 = rgb(0x3f3f3f);
pub const DROPDOWN_HOVER: Color32 = rgb(0x4f4f4f);
pub const DROPDOWN_BORDER: Color32 = rgb(0x282828);

/// `ToggleButton`'s knob under the pointer.
pub const TOGGLE_HOVER: Color32 = rgb(0x3f3f3f);
/// `Button-Border-Alt` (`CommandX`).
pub const BUTTON_BORDER_ALT: Color32 = rgb(0x141414);
pub const BUTTON_DISABLED_BACKGROUND: Color32 = rgb(0x131313);
/// A menu entry's shortcut text (`InputGestureText`).
pub const MENU_GESTURE: Color32 = rgb(0x708090);
pub const MENU_FOREGROUND_HOVER: Color32 = rgb(0xffffff);
pub const MENU_ICON: Color32 = rgb(0xe2e2e2);

pub const GRAPH_TAB_BACKGROUND: Color32 = rgb(0x1a1a1a);
pub const TAB_BORDER: Color32 = rgb(0xd0b132);

pub const WINDOW_BACKGROUND: Color32 = rgb(0x181818);
pub const WINDOW_DARK: Color32 = rgb(0x101010);
pub const PANEL_BACKGROUND: Color32 = rgb(0x2a2a2a);
pub const PANEL_CLADDING: Color32 = rgb(0x1f1f1f);
pub const PANEL_BORDER: Color32 = rgb(0x181818);
pub const POPUP_BACKGROUND: Color32 = rgb(0x101010);
pub const POPUP_BORDER: Color32 = rgb(0x2f2f2f);

pub const PROGRESS_EMPTY: Color32 = rgb(0x444444);
pub const PORT_LABEL: Color32 = LABEL_DEFAULT;

/// Graph surface grid lines (`GraphControl` template).
pub const GRID_FINE: Color32 = rgb(0x333333);
pub const GRID_COARSE: Color32 = rgb(0x3f3f3f);
pub const GRID_MEGA: Color32 = rgb(0x444444);

/// `NodeElement` brushes.
pub const NODE_BACKGROUND: Color32 = rgb(0x1a1b1d);
pub const NODE_BACKGROUND_BUILDING: Color32 = rgb(0x010101);
pub const NODE_BACKGROUND_SELECTED: Color32 = rgb(0x0f1011);
/// A chosen node's fill in the Vibrant node style (`EffectiveSelectedBackground`).
pub const NODE_BACKGROUND_SELECTED_VIBRANT: Color32 = rgb(0x050505);
pub const NODE_BORDER_SELECTED: Color32 = rgb(0x797979);

/// The dragged connection's gradient ends (`ConnectionLine`).
pub const CONNECTION_LINE_START: Color32 = rgb(0xfff6b9);
pub const CONNECTION_LINE_END: Color32 = rgb(0xf4c403);

/// `SelectionBg`, the marquee fill.
pub const SELECTION_FILL: Color32 = argb(0x11ffc107);
pub const SELECTION_STROKE: Color32 = rgb(0xd9c218);

/// The colour of a node's toolbox category (`Toolbox-{category}`), used
/// for its title, icon, selection border and outgoing connections.
pub fn toolbox_color(category: &str) -> Color32 {
    match category {
        "Primitive" => rgb(0x94ac5c),
        "Terrain" => rgb(0x55a07a),
        "Modify" => rgb(0x5892a5),
        "Surface" => rgb(0xa2dcbe),
        "Simulate" => rgb(0xd39467),
        "Derive" => rgb(0xad5590),
        "Colorize" => rgb(0x60538f),
        "Output" => rgb(0xc86275),
        "Utility" => rgb(0xb390a6),
        "Plugins" => rgb(0x8d9396),
        "Macro" => rgb(0x55a07a),
        _ => rgb(0x8d9396),
    }
}

/// A graph tab's swatch (`Swatch-{name}`).
pub fn swatch_color(name: &str) -> Color32 {
    match name {
        "Purple" => rgb(0x9183ee),
        "Brass" => rgb(0xd0b132),
        "LightBlue" => rgb(0x31b0cd),
        "DarkPink" => rgb(0xce6469),
        "Green" => rgb(0x6ba02b),
        "Tan" => rgb(0xbc8f6f),
        "Blue" => rgb(0x5bb2fa),
        "Orange" => rgb(0xd67540),
        "Gray" => rgb(0xbdbdbd),
        "Yellow" => rgb(0xcacd38),
        "Teal" => rgb(0x2aa0a4),
        "OrchidPink" => rgb(0xd957a7),
        "Aqua" => rgb(0x6bc7a4),
        "Umber" => rgb(0x946a5b),
        "LightPurple" => rgb(0x6a8dc5),
        "LightPink" => rgb(0xa87c7d),
        _ => rgb(0xd0b132),
    }
}

/// WPF's `Color.ScR` of an sRGB byte (`sRgbToScRgb`). Colours taken from
/// brushes (a region's) are kept as the scRGB of their bytes.
pub fn wpf_linear(byte: u8) -> f32 {
    let value = byte as f32 / 255.0;
    if value <= 0.0 {
        return 0.0;
    }
    if (value as f64) <= 0.04045 {
        return value / 12.92;
    }
    if value < 1.0 {
        let base = (value as f64 + 0.055) / 1.055;
        return base.powf(2.4) as f32;
    }
    1.0
}

/// WPF's sRGB byte of an scRGB value (`ScRgbTosRgb`), as a brush made with
/// `ToWPFColor` shows it.
pub fn wpf_srgb(value: f32) -> u8 {
    // `!(val > 0.0)`: NaN too.
    if value.is_nan() || value <= 0.0 {
        return 0;
    }
    if (value as f64) <= 0.0031308 {
        return (255.0 * value * 12.92 + 0.5) as u8;
    }
    if (value as f64) < 1.0 {
        let curve = (value as f64).powf(1.0 / 2.4) as f32;
        return (255.0 * (1.055 * curve - 0.055) + 0.5) as u8;
    }
    255
}

/// `Brush.ToColor()`: a colour's scRGB.
pub fn sc_rgb(color: Color32) -> [f32; 3] {
    [wpf_linear(color.r()), wpf_linear(color.g()), wpf_linear(color.b())]
}

/// `Color.ToWPFColor(alpha)`: an scRGB colour as shown, `alpha` its
/// opacity.
pub fn from_sc_rgb(color: [f32; 3], alpha: f32) -> Color32 {
    let opacity = (alpha.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    Color32::from_rgba_unmultiplied(wpf_srgb(color[0]), wpf_srgb(color[1]), wpf_srgb(color[2]), opacity)
}

/// The interface font is Roboto and the monospace font Roboto Mono, both
/// under the Apache License 2.0; Roboto's medium weight serves as the
/// semibold.
pub const FONT_SEMIBOLD: &str = "semibold";
pub const FONT_BOLD: &str = "bold";
pub const FONT_MONO_SEMIBOLD: &str = "mono-semibold";
pub const FONT_MONO_BOLD: &str = "mono-bold";

/// Roboto Mono is a variable font: one file holds every weight.
const ROBOTO_MONO: &[u8] = include_bytes!("../assets/fonts/roboto-mono.ttf");

/// How far Roboto's and Roboto Mono's glyphs are moved down, as a fraction
/// of the font size. The layouts were tuned with Selawik and an Iosevka
/// build, whose ascent and descent differ; these put the baseline of a
/// line centred in its box back where those fonts had it.
const INTERFACE_SHIFT: f32 = 0.048;
const MONOSPACE_SHIFT: f32 = -0.048;

pub fn interface(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(FONT_SEMIBOLD.into()))
}

pub fn monospace(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

pub fn monospace_semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(FONT_MONO_SEMIBOLD.into()))
}

pub fn monospace_bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(FONT_MONO_BOLD.into()))
}

/// One of Roboto's weights.
fn roboto(bytes: &'static [u8]) -> Arc<FontData> {
    let mut data = FontData::from_static(bytes);
    data.tweak.y_offset_factor = INTERFACE_SHIFT;
    Arc::new(data)
}

/// Roboto Mono at a `wght` from 100 to 700.
fn roboto_mono(weight: f32) -> Arc<FontData> {
    let mut data = FontData::from_static(ROBOTO_MONO);
    data.tweak.coords = VariationCoords::new([(b"wght", weight)]);
    data.tweak.y_offset_factor = MONOSPACE_SHIFT;
    Arc::new(data)
}

/// Installs the fonts and the theme's visuals.
pub fn apply(context: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("roboto".into(), roboto(include_bytes!("../assets/fonts/roboto-regular.ttf")));
    fonts.font_data.insert("roboto-medium".into(), roboto(include_bytes!("../assets/fonts/roboto-medium.ttf")));
    fonts.font_data.insert("roboto-bold".into(), roboto(include_bytes!("../assets/fonts/roboto-bold.ttf")));
    fonts.font_data.insert("roboto-mono".into(), roboto_mono(400.0));
    fonts.font_data.insert("roboto-mono-semibold".into(), roboto_mono(600.0));
    fonts.font_data.insert("roboto-mono-bold".into(), roboto_mono(700.0));

    // egui's own fonts stay behind ours for symbols Roboto lacks (⚠).
    let fallbacks = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let with_fallbacks = |primary: &str| {
        let mut family = vec![primary.to_string()];
        family.extend(fallbacks.iter().cloned());
        family
    };
    fonts.families.insert(FontFamily::Proportional, with_fallbacks("roboto"));
    fonts.families.insert(FontFamily::Name(FONT_SEMIBOLD.into()), with_fallbacks("roboto-medium"));
    fonts.families.insert(FontFamily::Name(FONT_BOLD.into()), with_fallbacks("roboto-bold"));
    fonts.families.insert(FontFamily::Monospace, with_fallbacks("roboto-mono"));
    fonts.families.insert(FontFamily::Name(FONT_MONO_SEMIBOLD.into()), with_fallbacks("roboto-mono-semibold"));
    fonts.families.insert(FontFamily::Name(FONT_MONO_BOLD.into()), with_fallbacks("roboto-mono-bold"));
    context.set_fonts(fonts);

    let mut visuals = Visuals::dark();
    visuals.override_text_color = Some(LABEL_DEFAULT);
    visuals.panel_fill = WINDOW_BACKGROUND;
    // Menus (`Default-Menu-Item`'s popup): `Menu-Background`, a
    // `Menu-Border` line, corners 6 and a soft shadow (blur 10, 0.4).
    visuals.window_fill = MENU_BACKGROUND;
    visuals.window_stroke = Stroke::new(1.0, MENU_BORDER);
    visuals.popup_shadow = egui::Shadow { offset: [0, 0], blur: 10, spread: 0, color: Color32::from_black_alpha(102) };
    visuals.extreme_bg_color = TEXTBOX_BACKGROUND;
    visuals.faint_bg_color = PANEL_CLADDING;
    visuals.hyperlink_color = ACCENT_BLUE;
    visuals.selection.bg_fill = TEXTBOX_FOCUSED_BORDER;
    visuals.selection.stroke = Stroke::new(1.0, LABEL_EMPHASIS);
    visuals.menu_corner_radius = 6.0.into();
    visuals.window_corner_radius = 4.0.into();

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = PANEL_BACKGROUND;
    widgets.noninteractive.weak_bg_fill = PANEL_BACKGROUND;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, MENU_BORDER);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, LABEL_DEFAULT);
    widgets.inactive.bg_fill = BUTTON_BACKGROUND;
    widgets.inactive.weak_bg_fill = BUTTON_BACKGROUND;
    widgets.inactive.bg_stroke = Stroke::new(1.0, BUTTON_BORDER);
    widgets.inactive.fg_stroke = Stroke::new(1.0, LABEL_DEFAULT);
    widgets.hovered.bg_fill = MENU_BACKGROUND_HOVER;
    widgets.hovered.weak_bg_fill = MENU_BACKGROUND_HOVER;
    widgets.hovered.bg_stroke = Stroke::new(1.0, BUTTON_BORDER);
    widgets.hovered.fg_stroke = Stroke::new(1.0, LABEL_EMPHASIS);
    widgets.active.bg_fill = BUTTON_PRESSED;
    widgets.active.weak_bg_fill = BUTTON_PRESSED;
    widgets.active.bg_stroke = Stroke::new(1.0, TEXTBOX_FOCUSED_BORDER);
    widgets.active.fg_stroke = Stroke::new(1.0, LABEL_EMPHASIS);
    widgets.open.bg_fill = MENU_BACKGROUND_HOVER;
    widgets.open.weak_bg_fill = MENU_BACKGROUND_HOVER;
    widgets.open.fg_stroke = Stroke::new(1.0, MENU_FOREGROUND);
    context.set_visuals(visuals);

    context.global_style_mut(|style| {
        style.text_styles.insert(egui::TextStyle::Body, interface(12.0));
        style.text_styles.insert(egui::TextStyle::Button, interface(12.0));
        style.text_styles.insert(egui::TextStyle::Small, interface(10.0));
        style.text_styles.insert(egui::TextStyle::Monospace, monospace(11.0));
        style.text_styles.insert(egui::TextStyle::Heading, semibold(16.0));
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(8.0, 3.0);
        style.spacing.menu_margin = egui::Margin::same(1);
        style.spacing.interact_size.y = 20.0;
    });
}

/// The interface options the controls read as they draw (`Options`): set
/// at the start of each frame, kept in egui's memory for it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InterfaceSettings {
    /// `CompactUI`: the compact sizes (`Size.Compact`).
    pub compact: bool,
    /// `PreciseValue`: sliders show four decimals rather than two.
    pub precise_values: bool,
    /// `ShowTicks`: integer sliders under 100 mark each step.
    pub show_ticks: bool,
    /// `AcceptOnLostFocus`: a popup's text is kept when a click elsewhere
    /// closes it.
    pub accept_on_lost_focus: bool,
    /// `SliderSeed`: seeds get a slider instead of the seed box.
    pub seed_sliders: bool,
    /// `StackedSatelliteMaps`: palette lists show one strip a row.
    pub stacked_palettes: bool,
    /// `DisableAnimations`: fades jump to their end.
    pub disable_animations: bool,
}

impl Default for InterfaceSettings {
    fn default() -> Self {
        Self {
            compact: false,
            precise_values: false,
            show_ticks: true,
            accept_on_lost_focus: false,
            seed_sliders: false,
            stacked_palettes: false,
            disable_animations: false,
        }
    }
}

fn settings_id() -> egui::Id {
    egui::Id::new("nerthus-interface-settings")
}

/// Makes `settings` what the controls drawn from now on read.
pub fn set_settings(context: &egui::Context, settings: InterfaceSettings) {
    context.data_mut(|data| data.insert_temp(settings_id(), settings));
}

/// The interface options as set for this frame (the defaults until set).
pub fn settings(context: &egui::Context) -> InterfaceSettings {
    context.data(|data| data.get_temp(settings_id())).unwrap_or_default()
}

/// The size theme (`Size.Default`, or `Size.Compact` with Compact
/// Interface): the sizes the shell's styles take.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sizes {
    /// `Expander.Toggle.MinHeight`: a group panel's header.
    pub expander_header: f32,
    /// `Expander.Toggle.IconMargin`'s left and top: the chevron's.
    pub expander_icon_margin: [f32; 2],
    /// `Expander.Margin` (left, top, right, bottom) and `Expander.Padding`.
    pub expander_margin: [f32; 4],
    pub expander_padding: f32,
    /// `OptionsX.Item.Padding` (across, down), `OptionsX.Item.Margin` and
    /// `OptionsX.Item.MinHeight`.
    pub option_padding: [f32; 2],
    pub option_margin: f32,
    pub options_min_height: f32,
    /// `Slider.MinHeight` and `Slider.Margin`'s left: a slider's height and
    /// how far into its row its bar starts.
    pub slider_height: f32,
    pub slider_inset: f32,
    /// `Slider.BorderStroke.Thickness`: the bar's outline.
    pub slider_border: f32,
    /// `Slider.InnerMargin` (left, top, right, bottom): the shading's.
    pub slider_shading_margin: [f32; 4],
    /// `Editor.BottomMargin`: under a group panel's editors.
    pub editor_bottom: f32,
    /// `Separator.Margin`'s top and bottom (16 each side across).
    pub separator_margin: f32,
    /// `Column.Size`: the header column of toggles and dropdowns.
    pub header_column: f32,
    /// `Property.Border.Thickness` (left, top, right, bottom): the
    /// property area's command and warning rows' margins, and
    /// `Property.Border.Padding` (across, down) round the warning.
    pub property_border: [f32; 4],
    pub property_padding: [f32; 2],
    /// `Toolbar.Padding` (across, down), `Toolbar.Margin` and
    /// `Toolbar.MinHeight`: a `Toolbar` border's.
    pub toolbar_padding: [f32; 2],
    pub toolbar_margin: f32,
    pub toolbar_min_height: f32,
    /// `Toolbar.MenuMargin` (across): a toolbar menu's.
    pub toolbar_menu_margin: f32,
    /// `Toolbar.ButtonSize`.
    pub toolbar_button: f32,
    /// `TreeView.DockPadding` (across, down): round a tree row's icon and
    /// name.
    pub tree_padding: [f32; 2],
}

/// `Size.Default.xaml`.
pub const DEFAULT_SIZES: Sizes = Sizes {
    expander_header: 45.0,
    expander_icon_margin: [17.0, 2.0],
    expander_margin: [10.0, 5.0, 0.0, 5.0],
    expander_padding: 0.0,
    option_padding: [8.0, 4.0],
    option_margin: 2.0,
    options_min_height: 32.0,
    slider_height: 26.0,
    slider_inset: 20.0,
    slider_border: 1.0,
    slider_shading_margin: [1.0, 2.0, 0.0, 2.0],
    editor_bottom: 16.0,
    separator_margin: 8.0,
    header_column: 160.0,
    property_border: [4.0, 0.0, 4.0, 8.0],
    property_padding: [8.0, 4.0],
    toolbar_padding: [4.0, 2.0],
    toolbar_margin: 4.0,
    toolbar_min_height: 36.0,
    toolbar_menu_margin: -5.0,
    toolbar_button: 32.0,
    tree_padding: [8.0, 4.0],
};

/// `Size.Compact.xaml`.
pub const COMPACT_SIZES: Sizes = Sizes {
    expander_header: 28.0,
    expander_icon_margin: [4.0, 2.0],
    expander_margin: [2.0, 2.0, 2.0, 2.0],
    expander_padding: 1.0,
    option_padding: [2.0, 2.0],
    option_margin: 1.0,
    options_min_height: 22.0,
    slider_height: 22.0,
    slider_inset: 10.0,
    slider_border: 0.0,
    slider_shading_margin: [-1.0, 0.0, 0.0, 0.0],
    editor_bottom: 4.0,
    separator_margin: 2.0,
    header_column: 120.0,
    property_border: [4.0, 4.0, 4.0, 4.0],
    property_padding: [2.0, 2.0],
    toolbar_padding: [1.0, 1.0],
    toolbar_margin: 2.0,
    toolbar_min_height: 26.0,
    toolbar_menu_margin: 0.0,
    toolbar_button: 28.0,
    tree_padding: [2.0, 1.0],
};

/// The size theme as set for this frame.
pub fn sizes(context: &egui::Context) -> &'static Sizes {
    if settings(context).compact { &COMPACT_SIZES } else { &DEFAULT_SIZES }
}
