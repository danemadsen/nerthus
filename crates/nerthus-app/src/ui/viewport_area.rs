//! The viewport area (`ViewportArea`): the 3D view in a dark rounded frame
//! with the viewport toolbar (`CollapsedBorder`) down its right side: the
//! resolution, lighting, render style, mask enhancement, helpers and
//! screenshot tools at the top; the docked 2D view, measuring tools and
//! viewport options at the bottom. Over the view: the navigation compass
//! and the lighting panel.

use bevy::input::mouse::MouseButton;
use egui::{Align, Color32, CornerRadius, Layout, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, Vec2};
use nerthus_nodes::shade::LightRig;
use nerthus_render::{NavigationSettings, ViewportSettings};

use crate::editor::{Editor, RenderMode, PREVIEW_RESOLUTIONS};
use crate::overlays::{CUSTOM_OVERLAY, OVERLAY_IMAGES, OVERLAY_OPACITIES};
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::editors::{self, SliderSpec};
use crate::ui::property_area::group_panel;
use crate::ui::viewport_2d::Viewport2DState;
use crate::ui::widgets::{check_item, checked_icon_item, icon_button, icon_menu, icon_toggle, menu_caption, menu_item, menu_separator, toolbar_background, toolbar_button, toolbar_separator};
use crate::viewport_render::{MaskGradient, MASK_GRADIENTS};

/// The toolbar column: a `Toolbar.ButtonSize` button, the toolbar's
/// `Toolbar.Padding` and border 1.5, and its margin 2 each side.
fn toolbar_width(sizes: &theme::Sizes) -> f32 {
    sizes.toolbar_button + 2.0 * (sizes.toolbar_padding[0] + 1.5 + 2.0)
}

/// `ToggleSkyEditor`: the popup is 400 wide, its panel inset 24 (the
/// border's margin), placed left of the sun button.
const LIGHTING_POPUP_WIDTH: f32 = 400.0;
const LIGHTING_POPUP_MARGIN: f32 = 24.0;
/// `SunControl`: the dial, the sun's glow and how far past the dial the
/// glow may be dragged (`MoveFactor`, three quarters of the glow).
const SUN_DIAL: f32 = 96.0;
const SUN_GLOW: f32 = 48.0;
const SUN_MOVE_FACTOR: f32 = SUN_GLOW * 0.75;

/// `DecoCompass`: 32 pixels, its centre 48 in from the view's top right
/// (Unity's `ConstantPixelSize`, so the interface scale leaves it be).
const COMPASS_SIZE: f32 = 32.0;
const COMPASS_INSET: f32 = 48.0;
/// The sprite's ring (from 0.81 to 0.97 of its radius), its north arc and
/// the rest.
const COMPASS_RING_INNER: f32 = 0.81;
const COMPASS_RING_OUTER: f32 = 0.97;
const COMPASS_NORTH: Color32 = Color32::from_rgb(255, 133, 0);
const COMPASS_RING: Color32 = Color32::from_rgb(240, 240, 240);
/// The status text: `#707070` at 16 px, in a box 32 in from the view's
/// bottom left.
const NAVIGATION_HINT_COLOUR: Color32 = Color32::from_rgb(0x70, 0x70, 0x70);
const NAVIGATION_HINT_SIZE: f32 = 16.0;
const NAVIGATION_HINT_INSET: f32 = 32.0;
const NAVIGATION_HINT_BOX: (f32, f32) = (523.5, 300.0);

/// What is drawn over the 3D view besides the lighting panel.
pub struct ViewOverlays<'a> {
    /// Where north is on screen: radians clockwise from up.
    pub north_angle: f32,
    /// The navigation hint, while it shows.
    pub navigation_hint: Option<&'a str>,
}

/// Draws the frame and toolbar, and over the view the compass, the
/// navigation hint and the lighting panel; returns where the 3D view goes.
#[allow(clippy::too_many_arguments)]
pub fn show(ui: &mut Ui, rect: Rect, editor: &Editor, settings: &mut ViewportSettings, sun_open: &mut bool, view_2d: &mut Viewport2DState, overlays: &ViewOverlays, commands: &mut Vec<Command>) -> Rect {
    let frame_stroke = Stroke::new(2.0, egui::Color32::from_rgb(0x21, 0x21, 0x21));
    ui.painter().rect_stroke(rect, 6.0, frame_stroke, StrokeKind::Inside);

    let sizes = theme::sizes(ui.ctx());
    let toolbar_column = Rect::from_min_max(Pos2::new(rect.right() - toolbar_width(sizes), rect.top()), rect.max);
    // The column has the frame's own dark fill behind the toolbar.
    ui.painter().rect_filled(toolbar_column.shrink(2.0), 4.0, egui::Color32::from_rgb(0x05, 0x05, 0x05));
    let toolbar_rect = toolbar_column.shrink(2.0);
    toolbar_background(ui, toolbar_rect, theme::MENU_BORDER, 1.5);
    let [padding_across, padding_down] = sizes.toolbar_padding;
    let inner = toolbar_rect.shrink(1.5).shrink2(Vec2::new(padding_across, padding_down));

    let top_builder = UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Center));
    let mut top = ui.new_child(top_builder);
    top.spacing_mut().item_spacing.y = 0.0;
    resolution_menu(&mut top, editor, commands);
    toolbar_separator(&mut top, 8.0, 2.0);
    let was_open = *sun_open;
    let sun_button = icon_toggle(&mut top, "sun", "Lighting", *sun_open, theme::ACCENT_PRIMARY);
    if sun_button.clicked() {
        *sun_open = !*sun_open;
    }
    render_style_menu(&mut top, editor, commands);
    toolbar_separator(&mut top, 2.0, 2.0);
    helpers_menu(&mut top, editor, settings, commands);
    screenshot_menu(&mut top, editor, commands);
    toolbar_separator(&mut top, 2.0, 2.0);

    let bottom_builder = UiBuilder::new().max_rect(inner).layout(Layout::bottom_up(Align::Center));
    let mut bottom = ui.new_child(bottom_builder);
    bottom.spacing_mut().item_spacing.y = 0.0;
    icon_menu(&mut bottom, "menu-deep", "Viewport Options", None, |ui| {
        menu_caption(ui, "Viewport");
        // `TglEmbed_OnClick`, `TglFloat_OnClick`.
        let floating = editor.user.options.viewport_floating();
        if check_item(ui, !floating, "Embedded", "").clicked() && floating {
            commands.push(Command::SetViewportFloating(false));
        }
        if check_item(ui, floating, "Floating", "").clicked() && !floating {
            commands.push(Command::SetViewportFloating(true));
        }
        menu_separator(ui);
        if menu_item(ui, None, None, "Load Camera...", "").clicked() {
            commands.push(Command::LoadCamera);
        }
        if menu_item(ui, None, None, "Save Camera...", "").clicked() {
            commands.push(Command::SaveCamera);
        }
        menu_separator(ui);
        if menu_item(ui, None, None, "Reset to Loaded...", "").clicked() {
            commands.push(Command::ResetCameraToLoaded);
        }
        if menu_item(ui, None, None, "Reset to Default...", "").clicked() {
            commands.push(Command::ResetCameraToDefault);
        }
    });
    toolbar_separator(&mut bottom, 8.0, 8.0);
    // `btnPickHeight` and `btnMeasure` open the docked 2D view as needed.
    let picking = view_2d.is_picking();
    let measuring = view_2d.is_measuring();
    icon_menu(&mut bottom, "ruler-measure", "Measure", None, |ui| {
        if checked_icon_item(ui, picking, Some("color-picker"), Some(theme::ACCENT_MUTED_PURPLE), "Pick Height/Color", "") {
            commands.push(Command::TogglePickHeight);
        }
        if check_item(ui, measuring, "Measurement Tools", "").clicked() {
            commands.push(Command::ToggleMeasure);
        }
    });
    if icon_toggle(&mut bottom, "square-half", "Toggle Docked 2D Viewport", view_2d.docked, theme::ACCENT_PRIMARY).clicked() {
        commands.push(Command::Toggle2DViewport);
    }

    let area = Rect::from_min_max(rect.min, Pos2::new(toolbar_column.left(), rect.bottom())).shrink(2.0);
    let host = crate::ui::viewport_2d::dock(ui, area, editor, view_2d, commands);
    if settings.compass {
        compass(ui, host, overlays.north_angle);
    }
    if let Some(hint) = overlays.navigation_hint {
        navigation_hint(ui, host, hint);
    }
    if *sun_open {
        let panel = lighting_panel(ui, sun_button.rect, editor, settings, commands);
        // `StaysOpen` is off: a click elsewhere in the window closes it.
        if was_open && clicked_outside(ui, panel) {
            *sun_open = false;
        }
    }
    host
}

/// The preview resolution: the size drawn as text (monospace bold,
/// `Accent-Green`), as the `resolution-…` icons spell it.
pub fn resolution_menu(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    let response = icon_menu(ui, "", "Resolution", None, |ui| {
        menu_caption(ui, "Preview Resolution");
        for size in PREVIEW_RESOLUTIONS {
            let checked = editor.preview_resolution == size;
            if check_item(ui, checked, &format!("{size} x {size}"), "").clicked() {
                commands.push(Command::SetPreviewResolution(size));
            }
        }
    });
    let label = resolution_label(editor.preview_resolution);
    let galley = ui.painter().layout_no_wrap(label.to_string(), theme::monospace_bold(10.0), theme::ACCENT_GREEN);
    ui.painter().galley(response.rect.center() - galley.size() / 2.0, galley, theme::ACCENT_GREEN);
}

/// What the `resolution-0K`… icons spell.
pub fn resolution_label(resolution: usize) -> &'static str {
    match resolution {
        512 => "0K",
        1024 => "1K",
        2048 => "2K",
        _ => "4K",
    }
}

/// `miPreviewRender`: the current renderer's icon in `Accent-Green`.
fn render_style_menu(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    let render_icon = match editor.render_mode {
        RenderMode::Data => "render-0",
        RenderMode::Clay => "render-1",
        RenderMode::Realistic => "lens",
    };
    icon_menu(ui, render_icon, "Render Style", Some(theme::ACCENT_GREEN), |ui| {
        menu_caption(ui, "Preview Renderer");
        let modes = [
            (RenderMode::Data, "render-0", Some(theme::ACCENT_MUTED_MAGENTA), "Visualize Data (Render Off)"),
            (RenderMode::Clay, "render-1", None, "Clay Render"),
            (RenderMode::Realistic, "lens", Some(theme::ACCENT_MUTED_BLUE), "Realistic Render"),
        ];
        for (mode, icon, accent, text) in modes {
            if checked_icon_item(ui, editor.render_mode == mode, Some(icon), accent, text, "") {
                commands.push(Command::SetRenderMode(mode));
            }
        }
    });
}

/// `miOverlay`: the viewport's helper elements, and the overlay laid over
/// the render (`lstOverlayOpacity`, `lstOverlayImage`).
fn helpers_menu(ui: &mut Ui, editor: &Editor, settings: &mut ViewportSettings, commands: &mut Vec<Command>) {
    icon_menu(ui, "perspective-view", "Viewport Helpers", None, |ui| {
        menu_caption(ui, "Viewport Elements");
        if checked_icon_item(ui, settings.grid, Some("grid-4x4"), None, "Grid", "") {
            settings.grid = !settings.grid;
        }
        if checked_icon_item(ui, settings.bounds, Some("border-corners"), None, "Bounds", "") {
            settings.bounds = !settings.bounds;
        }
        if checked_icon_item(ui, settings.compass, Some("navigation-north"), None, "Navigation Compass", "") {
            settings.compass = !settings.compass;
        }
        menu_separator(ui);
        menu_caption(ui, "Overlay");
        let opacities: Vec<String> = OVERLAY_OPACITIES.iter().map(|(text, _)| text.to_string()).collect();
        let opacity_id = ui.id().with("overlay-opacity");
        let picked_opacity = editors::option_list(ui, opacity_id, &opacities, Some(editor.overlay.opacity), 3, 11.0);
        if let Some(index) = picked_opacity.filter(|index| *index != editor.overlay.opacity) {
            commands.push(Command::SetOverlayOpacity(index));
        }
        let images: Vec<String> = OVERLAY_IMAGES.iter().map(|text| text.to_string()).collect();
        let image_id = ui.id().with("overlay-image");
        // Custom asks for its image every time it is picked.
        let picked_image = editors::option_list(ui, image_id, &images, Some(editor.overlay.image), 3, 11.0);
        if let Some(index) = picked_image.filter(|index| *index != editor.overlay.image || *index == CUSTOM_OVERLAY) {
            commands.push(Command::SetOverlayImage(index));
        }
    });
}

/// `miScreenshot`.
fn screenshot_menu(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    icon_menu(ui, "camera", "Screenshot Tools", None, |ui| {
        if menu_item(ui, Some("camera"), Some(theme::ACCENT_STRONG_GREEN), "Save Screenshot", "F9").clicked() {
            commands.push(Command::SaveScreenshot);
        }
        menu_caption(ui, "Screenshot Options");
        screenshot_options(ui, editor, commands);
    });
}

/// The screenshot options both screenshot menus share: the other
/// screenshots, the folder, and the 3D view's screenshot resolution.
pub fn screenshot_options(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    if menu_item(ui, None, None, "Save Graph Screenshot as...", "").clicked() {
        commands.push(Command::SaveGraphScreenshotAs);
    }
    if menu_item(ui, None, None, "Save Viewport Screenshot as...", "Ctrl F9").clicked() {
        commands.push(Command::SaveViewportScreenshotAs);
    }
    if menu_item(ui, None, None, "Open Screenshots Folder", "").clicked() {
        commands.push(Command::OpenScreenshotsFolder);
    }
    menu_separator(ui);
    menu_caption(ui, "Resolution");
    for (scale, text) in [(1, "1x"), (2, "2x"), (4, "4x")] {
        if check_item(ui, editor.screenshot_supersample == scale, text, "").clicked() {
            commands.push(Command::SetScreenshotScale(scale));
        }
    }
}

/// `DecoCompass` (`CompassBehavior`): a ring at the view's top right whose
/// orange arc points north, the map's top, as the camera turns.
fn compass(ui: &Ui, host: Rect, north_angle: f32) {
    let pixel = 1.0 / ui.ctx().zoom_factor();
    let centre = Pos2::new(host.right() - COMPASS_INSET * pixel, host.top() + COMPASS_INSET * pixel);
    let radius = COMPASS_SIZE / 2.0 * pixel;
    let ring_radius = radius * (COMPASS_RING_INNER + COMPASS_RING_OUTER) / 2.0;
    let ring_width = radius * (COMPASS_RING_OUTER - COMPASS_RING_INNER);
    let turn = north_angle.to_degrees();
    // Four arcs with small gaps at the diagonals: north orange.
    let arcs = [(-43.0, 43.0, COMPASS_NORTH), (47.0, 133.0, COMPASS_RING), (137.0, 223.0, COMPASS_RING), (227.0, 313.0, COMPASS_RING)];
    let painter = ui.painter().with_clip_rect(host);
    for (from, to, colour) in arcs {
        let points = arc_points(centre, ring_radius, from + turn, to + turn);
        painter.add(Shape::line(points, Stroke::new(ring_width, colour)));
    }
}

/// `SetInputOptions`' hint: which button does what, each line ended as
/// `StringBuilder.AppendLine` ends it.
pub fn navigation_hint_text(navigation: &NavigationSettings) -> String {
    let actions = [
        (navigation.alt_rotate, navigation.rotate_button, "to Rotate"),
        (navigation.alt_pan, navigation.pan_button, "to Pan"),
        (navigation.alt_zoom, navigation.zoom_button, "or Mouse Wheel to Zoom"),
    ];
    let mut text = String::new();
    for (alt, button, action) in actions {
        let button_name = match button {
            MouseButton::Right => "Mouse R",
            MouseButton::Middle => "Mouse M",
            _ => "Mouse L",
        };
        let prefix = if alt { "Alt + " } else { "" };
        text.push_str(&format!("{prefix}{button_name} {action}\n"));
    }
    text.push_str("WASD to move around\n");
    text
}

/// The viewport's status text (`Text (TMP)` in a 523.5 by 300 box 32 in
/// from the view's bottom left): grey 16 px lines, bottom up, in Unity's
/// pixels.
fn navigation_hint(ui: &Ui, host: Rect, hint: &str) {
    let pixel = 1.0 / ui.ctx().zoom_factor();
    let box_size = Vec2::new(NAVIGATION_HINT_BOX.0, NAVIGATION_HINT_BOX.1) * pixel;
    let bottom_left = Pos2::new(host.left() + NAVIGATION_HINT_INSET * pixel, host.bottom() - NAVIGATION_HINT_INSET * pixel);
    let font = theme::interface(NAVIGATION_HINT_SIZE * pixel);
    let mut job = egui::text::LayoutJob::simple(hint.to_string(), font, NAVIGATION_HINT_COLOUR, box_size.x);
    job.halign = Align::LEFT;
    let galley = ui.painter().layout_job(job);
    let height = galley.size().y.min(box_size.y);
    let position = Pos2::new(bottom_left.x, bottom_left.y - height);
    let painter = ui.painter().with_clip_rect(host);
    painter.galley(position, galley, NAVIGATION_HINT_COLOUR);
}

/// Points along an arc, angles in degrees clockwise from up.
fn arc_points(centre: Pos2, radius: f32, from: f32, to: f32) -> Vec<Pos2> {
    let steps = 24;
    let mut points = Vec::with_capacity(steps + 1);
    for step in 0..=steps {
        let angle = (from + (to - from) * step as f32 / steps as f32).to_radians();
        let offset = Vec2::new(angle.sin(), -angle.cos()) * radius;
        points.push(centre + offset);
    }
    points
}

/// Whether a click this frame landed outside `panel` on the window itself
/// (not on a menu or list the panel opened).
fn clicked_outside(ui: &Ui, panel: Rect) -> bool {
    let ctx = ui.ctx();
    let click = ctx.input(|input| if input.pointer.any_click() { input.pointer.interact_pos() } else { None });
    let Some(position) = click else {
        return false;
    };
    if panel.contains(position) {
        return false;
    }
    let layer = ctx.layer_id_at(position);
    layer.is_none_or(|layer| layer.order == egui::Order::Background)
}

/// `skyPopup`: the lighting panel (`PrivateSky` in an `EditorBase`) left
/// of the sun button: a toolbar with Reset Sky, the sun dial and the
/// parameters. Returns where the panel is.
fn lighting_panel(ui: &Ui, sun_button: Rect, editor: &Editor, settings: &mut ViewportSettings, commands: &mut Vec<Command>) -> Rect {
    let panel_width = LIGHTING_POPUP_WIDTH - 2.0 * LIGHTING_POPUP_MARGIN;
    let position = Pos2::new(sun_button.left() - LIGHTING_POPUP_WIDTH + LIGHTING_POPUP_MARGIN, sun_button.top() + LIGHTING_POPUP_MARGIN);
    let area = egui::Area::new(ui.id().with("lighting-panel")).fixed_pos(position).order(egui::Order::Foreground);
    let shown = area.show(ui.ctx(), |ui| {
        let mut frame = egui::Frame::new();
        frame.fill = theme::PANEL_BACKGROUND;
        frame.stroke = Stroke::new(1.0, theme::PANEL_BORDER);
        frame.corner_radius = CornerRadius::same(4);
        frame.inner_margin = egui::Margin { left: 0, right: 8, top: 0, bottom: 0 };
        frame.shadow = ui.visuals().popup_shadow;
        frame.show(ui, |ui| {
            ui.set_width(panel_width - 8.0);
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            // An area's room is its size last frame; the panel may run to
            // the window's foot, scrolling past that.
            let lowest = ui.ctx().content_rect().bottom() - LIGHTING_POPUP_MARGIN;
            ui.set_max_height(lowest - ui.cursor().top());
            lighting_toolbar(ui, settings, commands);
            let room = (lowest - ui.cursor().top()).max(120.0);
            let mut scroll = egui::ScrollArea::vertical();
            scroll = scroll.max_height(room);
            scroll.show(ui, |ui| {
                let mut padding = egui::Frame::new();
                padding.inner_margin = egui::Margin::same(8);
                padding.show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    lighting_editor(ui, editor, settings, commands);
                });
            });
        });
    });
    shown.response.rect
}

/// The panel's toolbar: Reset Sky (`restore`).
fn lighting_toolbar(ui: &mut Ui, settings: &mut ViewportSettings, commands: &mut Vec<Command>) {
    let height = toolbar_button(ui) + 2.0 * (4.0 + 1.5);
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    toolbar_background(ui, row, theme::MENU_BORDER, 1.5);
    let inner = row.shrink(1.5 + 4.0);
    let mut toolbar = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
    if icon_button(&mut toolbar, "restore", "Reset Sky", None).clicked() {
        settings.skybox = false;
        commands.push(Command::ResetSky);
    }
}

/// The dial, then `PrivateSky`'s parameters in their groups. Each group
/// moves to the next palette and each editor takes the next colour, as
/// `EditorBase` colours them.
fn lighting_editor(ui: &mut Ui, editor: &Editor, settings: &mut ViewportSettings, commands: &mut Vec<Command>) {
    let lighting = editor.lighting;
    let mut changed = lighting;
    ui.vertical_centered(|ui| {
        if let Some((azimuth, elevation)) = sun_dial(ui, lighting.sun_azimuth, lighting.sun_elevation) {
            changed.sun_azimuth = azimuth;
            changed.sun_elevation = elevation;
        }
    });
    ui.add_space(8.0);

    let color = |palette: usize, index: usize| editors::palette_color(palette, index);
    group_panel(ui, (-1, 0), "Sun", true, |ui| {
        float_row(ui, "Sun Azimuth", &mut changed.sun_azimuth, (0.0, 360.0, 327.0), color(1, 0));
        float_row(ui, "Sun Elevation", &mut changed.sun_elevation, (0.0, 90.0, 25.0), color(1, 1));
    });
    group_panel(ui, (-1, 1), "Visualization", true, |ui| {
        float_row(ui, "Ground Brightness", &mut changed.ground_brightness, (0.0, 1.0, 0.8), color(2, 2));
        let entries: Vec<(String, String)> = MASK_GRADIENTS.iter().map(|(_, name)| (name.to_string(), name.to_string())).collect();
        let current = editor.mask_gradient.name();
        let id = ui.id().with("mask-gradient");
        let picked = editors::dropdown_row(ui, id, "Mask Gradient", &entries, current, current);
        if let Some(gradient) = picked.as_deref().and_then(MaskGradient::from_name) {
            commands.push(Command::SetMaskGradient(gradient));
        }
    });
    group_panel(ui, (-1, 2), "Light", true, |ui| {
        float_row(ui, "Exposure", &mut changed.exposure, (0.0, 4.0, 1.0), color(0, 4));
        float_row(ui, "Sun Intensity", &mut changed.sun_intensity, (0.0, 2.0, 1.0), color(0, 5));
        float_row(ui, "Ambient Intensity", &mut changed.ambient_intensity, (0.0, 2.0, 1.0), color(0, 6));
        switch_row(ui, "Shadows", &mut changed.shadows);
        switch_row(ui, "Ambient Occlusion", &mut changed.ambient_occlusion);
    });
    group_panel(ui, (-1, 3), "Atmosphere", true, |ui| {
        float_row(ui, "Air Density", &mut changed.air_density, (0.0, 2.0, 1.0), color(1, 1));
        float_row(ui, "Haze", &mut changed.haze, (0.0, 2.0, 1.0), color(1, 2));
        float_row(ui, "Ozone", &mut changed.ozone, (0.0, 2.0, 1.0), color(1, 3));
        switch_row(ui, "Show Skybox", &mut settings.skybox);
        float_row(ui, "Fog Density", &mut changed.fog_density, (0.0, 1.0, 0.2), color(1, 5));
    });
    if changed != lighting {
        commands.push(Command::SetLighting(changed));
    }
}

/// A `float` parameter's slider over (minimum, maximum, default).
fn float_row(ui: &mut Ui, label: &str, value: &mut f32, range: (f64, f64, f64), color: Color32) {
    let (minimum, maximum, default) = range;
    let spec = SliderSpec {
        default: Some(default),
        ..SliderSpec::linear(minimum, maximum)
    };
    let id = ui.id().with(("lighting", label));
    if let Some(new_value) = editors::slider(ui, id, label, *value as f64, &spec, color) {
        *value = new_value as f32;
    }
}

/// A `bool` parameter's switch.
fn switch_row(ui: &mut Ui, label: &str, value: &mut bool) {
    let id = ui.id().with(("lighting", label));
    if let Some(new_value) = editors::toggle(ui, id, label, *value, theme::ACCENT_PRIMARY) {
        *value = new_value;
    }
}

/// `SunControl`: a 96 dial (north up, overhead in the middle, the horizon
/// at its rim) with the sun's glow where the sun stands, coloured by the
/// sky that sun gives. Clicking or dragging moves the sun; returns its
/// new azimuth and elevation.
fn sun_dial(ui: &mut Ui, azimuth: f32, elevation: f32) -> Option<(f32, f32)> {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(SUN_DIAL), Sense::click_and_drag());
    let centre = rect.center();
    let radius = SUN_DIAL / 2.0;

    // `UpdateColors`: the dial's own rig at this sun.
    let rig = LightRig {
        automatic_colors: true,
        ambient_intensity: 1.8,
        sun_intensity: 1.8,
        air_density: 0.01,
        aerosols: 0.05,
        ozone: 0.0025,
        ..LightRig::default()
    };
    let sky = rig.sky(azimuth, elevation);
    let sun_colour = linear_colour(sky.sun_color, 1.0);
    let painter = ui.painter_at(rect);
    painter.circle_filled(centre, radius, Color32::from_rgb(0x18, 0x18, 0x18));
    // The sky's colour (`ambient`, 1 in from the rim), fainter in the
    // middle: its opacity rises from 0.3 to 0.6 at the rim.
    let rings = 12;
    let ring_width = (radius - 1.0) / rings as f32;
    for ring in 0..rings {
        let middle = (ring as f32 + 0.5) / rings as f32;
        let opacity = 0.3 + 0.3 * middle;
        let colour = linear_colour(sky.ambient_color, opacity);
        // A little overlap hides the seams between rings.
        painter.circle_stroke(centre, (radius - 1.0) * middle, Stroke::new(ring_width + 0.75, colour));
    }

    // `AzEl2XY`: the glow's centre on the canvas.
    let angle = (450.0 + azimuth).to_radians();
    let reach = 0.5 - 0.5 * elevation / 90.0;
    let x = 0.5 + reach * angle.cos();
    let y = 0.5 + reach * angle.sin();
    let glow_centre = rect.min + Vec2::new((1.0 - x) * SUN_DIAL, (1.0 - y) * SUN_DIAL);
    // The glow's opacity mask: solid to half its radius, then fading.
    let glow_radius = SUN_GLOW / 2.0;
    for (fraction, opacity) in [(1.0, 0.05), (0.75, 0.12), (0.578, 0.3), (0.497, 1.0)] {
        painter.circle_filled(glow_centre, glow_radius * fraction, linear_colour(sky.sun_color, opacity));
    }
    painter.circle_stroke(centre, radius - 1.0, Stroke::new(2.0, sun_colour));

    let pointer = response.interact_pointer_pos()?;
    // The right button opens the dial's menu instead (`Thumb_MouseDown`).
    let primary = ui.input(|input| input.pointer.primary_down());
    let pressed = response.is_pointer_button_down_on() && primary;
    if !(pressed || response.clicked()) {
        return None;
    }
    // `MoveSun`: the glow follows the pointer, held within the dial and
    // its margin; the angle and distance from the middle give the sun.
    let lowest = -SUN_MOVE_FACTOR;
    let highest = SUN_DIAL + SUN_MOVE_FACTOR;
    let offset = pointer - rect.min - Vec2::splat(glow_radius);
    let left = offset.x.clamp(lowest, highest);
    let top = offset.y.clamp(lowest, highest);
    let glow_x = left + glow_radius;
    let glow_y = top + glow_radius;
    let from_middle_x = glow_x - radius;
    let from_middle_y = glow_y - radius;
    let new_azimuth = (450.0 + from_middle_y.atan2(from_middle_x).to_degrees()) % 360.0;
    let distance = (from_middle_x * from_middle_x + from_middle_y * from_middle_y).sqrt() as f64;
    let new_elevation = ((0.5 - distance * 0.009999999776482582) * 2.0 * 90.0).max(0.0) as f32;
    Some((new_azimuth, new_elevation))
}

/// A linear (scRGB) colour, as `Color.FromScRgb` makes it, at `opacity`.
fn linear_colour(colour: [f32; 3], opacity: f32) -> Color32 {
    let rgba = egui::Rgba::from_rgba_unmultiplied(colour[0].clamp(0.0, 1.0), colour[1].clamp(0.0, 1.0), colour[2].clamp(0.0, 1.0), opacity.clamp(0.0, 1.0));
    Color32::from(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewport_render::ViewportLighting;

    #[test]
    fn arcs_turn_clockwise_from_up() {
        let centre = Pos2::new(0.0, 0.0);
        let points = arc_points(centre, 10.0, 0.0, 90.0);
        let first = points[0];
        let last = points[points.len() - 1];
        assert!(first.x.abs() < 1e-4 && (first.y + 10.0).abs() < 1e-4);
        assert!((last.x - 10.0).abs() < 1e-4 && last.y.abs() < 1e-4);
    }

    #[test]
    fn lighting_defaults_sit_on_the_panel() {
        let defaults = ViewportLighting::panel_defaults();
        assert_eq!((defaults.sun_azimuth, defaults.sun_elevation), (327.0, 25.0));
    }
}
