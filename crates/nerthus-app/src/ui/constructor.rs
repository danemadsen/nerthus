//! Construction's Constructor ("God Mode: Constructor", 1280 by 720, Open
//! Constructor in its properties): on the left the base terrain with every
//! placed terrain previewed and shaded, a gizmo on each (the chosen one's
//! handles move, scale, raise, lift, shape, edge and turn its terrain); on
//! the right the toolbar (Colorize Previews, Refresh Previews, Update 3D
//! Viewport, the preview's size and Faster Previews), the terrains in the
//! order they are placed (drag one to place it earlier or later) and the
//! chosen terrain's settings (`TerrainTransformEditor`). The node builds
//! when a gizmo is let go with Update 3D Viewport on, and when the window
//! closes (`Constructor_Closing`).

use std::sync::Arc;

use egui::{Color32, CursorIcon, Id, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, Vec2};
use nerthus_engine::json::f32_value;
use nerthus_engine::Map;
use serde_json::{json, Value};

use crate::constructor_preview::{ConstructorRenderer, PreviewJob, PreviewTransform};
use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::editors::{self, SliderSpec};
use crate::ui::widgets::{check_item, icon_menu, menu_separator};

/// The right column's narrowest and the left's (`MinWidth`s).
const LIST_MIN_WIDTH: f32 = 300.0;
const PREVIEW_MIN_WIDTH: f32 = 720.0;
/// The preview's sizes (`mnuRes…`, 512 to begin with) and the finished
/// one's (`FinalResolution`).
const PREVIEW_SIZES: [usize; 3] = [256, 512, 1024];
const FINAL_RESOLUTION: usize = 2048;
/// `brushes`: the colours terrains without one are given, by place.
const BRUSHES: [u32; 9] = [0xffadad, 0xffd6a5, 0xfdffb6, 0xcaffbf, 0x9bf6ff, 0xa0c4ff, 0xbdb2ff, 0xffc6ff, 0xfffffc];
/// `GizmoConstruction`: the drag steps and snaps.
const HEIGHT_STEP: f32 = 0.005;
const SCALE_STEP: f32 = 0.01;
const ROTATE_STEP: f32 = 0.5;
const ROTATE_SNAP: f32 = 15.0;
/// The gizmo's scale ring (`ScaleHandle`, 37 to 41 round its middle).
const RING_OUTER: f32 = 41.5;
const RING_INNER: f32 = 37.0;
/// `GizmoConstruction.Color`'s default.
const GIZMO_COLOUR: Color32 = Color32::from_rgb(188, 0, 255);

/// Which of the chosen gizmo's handles is held.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Handle {
    Move,
    Scale,
    Height,
    Base,
    Shaper,
    Mask,
    Rotate,
}

/// A gizmo drag: the handle, where it started and the transform then.
#[derive(Clone, Debug)]
struct GizmoDrag {
    handle: Handle,
    start: Pos2,
    start_transform: PreviewTransform,
    /// `_mouseOffsetFromCenter`, in fractions of the preview.
    offset: Vec2,
    /// `_scaleDir`.
    scale_direction: f32,
}

/// The window while it is open.
pub struct ConstructorWindow {
    node: i32,
    renderer: ConstructorRenderer,
    /// What the newest request was made from.
    sent: Option<String>,
    picture: Option<egui::TextureHandle>,
    problem: Option<String>,
    /// The chosen terrain, by place.
    selected: usize,
    colorize: bool,
    faster: bool,
    update_viewport: bool,
    preview_size: usize,
    refresh: bool,
    drag: Option<GizmoDrag>,
    /// The list row a drag started on.
    row_drag: Option<usize>,
    /// An edit is under way (its undo step is taken), and edits since the
    /// node last built.
    editing: bool,
    unbuilt: bool,
}

impl ConstructorWindow {
    /// `OpenGodMode`.
    pub fn new(node: i32) -> Self {
        Self {
            node,
            renderer: ConstructorRenderer::start(),
            sent: None,
            picture: None,
            problem: None,
            selected: 0,
            colorize: false,
            faster: false,
            update_viewport: true,
            preview_size: 512,
            refresh: false,
            drag: None,
            row_drag: None,
            editing: false,
            unbuilt: false,
        }
    }

    pub fn node(&self) -> i32 {
        self.node
    }

    /// Whether the chosen gizmo's handles are held.
    pub fn gizmo_held(&self) -> bool {
        self.drag.is_some()
    }

    /// The nodes whose maps the previews are made from (`In` and each
    /// `Terrain`), for the builds to report.
    pub fn sources(&self, editor: &Editor) -> Vec<i32> {
        let Some(node) = editor.document.node(self.node) else {
            return Vec::new();
        };
        let records = node.ports.iter().filter(|port| port.is_input()).filter_map(|port| port.record.as_ref());
        records.map(|record| record.from).collect()
    }
}

/// The saved transforms (`Transforms`), in order.
fn transforms_of(editor: &Editor, node: i32) -> Vec<Value> {
    let saved = editor.document.node(node).and_then(|node| node.values.get("Transforms"));
    saved.and_then(Value::as_array).cloned().unwrap_or_default()
}

/// The map feeding one of the node's inputs, as the builds last reported.
fn input_map(editor: &Editor, node: i32, port: &str) -> Option<Arc<Map>> {
    let data = editor.document.node(node)?;
    let record = data.port(port)?.record.as_ref()?;
    let watched = editor.watched_outputs.get(&record.from)?;
    watched.outputs.iter().find(|(name, _)| *name == record.from_port).map(|(_, map)| map.clone())
}

/// `GetTerrainTitle`: its place and the node feeding its input, or
/// "Disconnected".
fn terrain_title(editor: &Editor, node: i32, transform: &PreviewTransform, place: usize) -> String {
    let port = format!("Terrain{}", transform.index + 1);
    let data = editor.document.node(node);
    let source = data.and_then(|data| data.port(&port)).and_then(|port| port.record.as_ref());
    let name = source.and_then(|record| editor.document.node(record.from)).map(|node| node.name.clone());
    format!("{}: {}", place + 1, name.unwrap_or_else(|| "Disconnected".to_string()))
}

fn brush(place: usize) -> Color32 {
    let value = BRUSHES[place % BRUSHES.len()];
    Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

fn transform_colour(transform: &PreviewTransform) -> Color32 {
    let [red, green, blue] = transform.colour.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8);
    Color32::from_rgb(red, green, blue)
}

/// The window; false once it has closed.
pub fn show(ctx: &egui::Context, editor: &mut Editor, window: &mut ConstructorWindow) -> bool {
    if editor.document.node(window.node).is_none() {
        return false;
    }
    take_preview(ctx, window);
    let mut open = true;
    let mut frame = egui::Frame::window(&ctx.global_style());
    frame.fill = theme::PANEL_BACKGROUND;
    frame.inner_margin = egui::Margin::same(12);
    let mut constructor = egui::Window::new("God Mode: Constructor");
    constructor = constructor.id(Id::new(("constructor", window.node)));
    constructor = constructor.open(&mut open);
    constructor = constructor.default_size(Vec2::new(1280.0, 720.0));
    constructor = constructor.collapsible(false);
    constructor = constructor.frame(frame);
    constructor.show(ctx, |ui| {
        let area = ui.available_rect_before_wrap();
        ui.allocate_rect(area, Sense::hover());
        let preview_width = ((area.width() - 5.0) / 2.0).max(PREVIEW_MIN_WIDTH).min((area.width() - 5.0 - LIST_MIN_WIDTH).max(200.0));
        let preview = Rect::from_min_size(area.min, Vec2::new(preview_width, area.height()));
        let panel = Rect::from_min_max(Pos2::new(preview.right() + 5.0, area.top()), area.max);
        preview_area(ui, preview, editor, window);
        let mut panel_ui = ui.new_child(UiBuilder::new().max_rect(panel));
        side_panel(&mut panel_ui, editor, window);
    });
    request_preview(editor, window);
    if !ctx.input(|input| input.pointer.primary_down()) {
        window.editing = false;
    }
    if !open && window.unbuilt {
        // `Constructor_Closing`: `B.SafelyBuildNode(Node)`.
        editor.changed();
    }
    open
}

/// Takes in a finished preview.
fn take_preview(ctx: &egui::Context, window: &mut ConstructorWindow) {
    let Some(image) = window.renderer.poll() else {
        return;
    };
    match image.colours {
        Ok(colours) => {
            let pixel_count = colours.width * colours.height;
            let picture = egui::ColorImage::from_rgb([colours.width, colours.height], &colours.rgb[..pixel_count * 3]);
            let texture = ctx.load_texture(format!("constructor-{}", window.node), picture, egui::TextureOptions::LINEAR);
            window.picture = Some(texture);
            window.problem = None;
        }
        Err(problem) => window.problem = Some(problem),
    }
}

/// Asks for a preview when what it shows changed: the fast size while
/// anything is held, the finished size otherwise.
fn request_preview(editor: &Editor, window: &mut ConstructorWindow) {
    let Some(base) = input_map(editor, window.node, "In") else {
        window.problem = Some("Construction requires a base terrain.".to_string());
        return;
    };
    let saved = transforms_of(editor, window.node);
    let transforms: Vec<PreviewTransform> = saved.iter().map(PreviewTransform::from_value).collect();
    let count = transforms.iter().map(|transform| transform.index + 1).max().unwrap_or(0);
    let terrains: Vec<Option<Arc<Map>>> = (0..count).map(|index| input_map(editor, window.node, &format!("Terrain{}", index + 1))).collect();
    let holding = window.drag.is_some() || window.editing;
    let resolution = if holding { window.preview_size } else { FINAL_RESOLUTION };
    let pointers: Vec<usize> = terrains.iter().map(|map| map.as_ref().map_or(0, |map| Arc::as_ptr(map) as usize)).collect();
    let fingerprint = format!("{saved:?}|{}|{pointers:?}|{resolution}|{}|{}|{:?}", Arc::as_ptr(&base) as usize, window.colorize, window.faster, editor.lighting);
    if window.sent.as_deref() == Some(fingerprint.as_str()) && !window.refresh {
        return;
    }
    let terrain = editor.document.terrain();
    let job = PreviewJob {
        base,
        terrains,
        transforms,
        resolution,
        colorize: window.colorize,
        faster: window.faster,
        refresh: std::mem::take(&mut window.refresh),
        lighting: editor.lighting,
        terrain: (terrain.width.max(1.0), terrain.height.max(1.0)),
    };
    window.renderer.request(job);
    window.sent = Some(fingerprint);
}

/// The preview (square, as large as fits) and the gizmos over it.
fn preview_area(ui: &mut Ui, rect: Rect, editor: &mut Editor, window: &mut ConstructorWindow) {
    ui.painter().rect_filled(rect, 0.0, theme::WINDOW_BACKGROUND);
    let side = rect.width().min(rect.height());
    let image = Rect::from_center_size(rect.center(), Vec2::splat(side));
    if let Some(picture) = &window.picture {
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        ui.painter().image(picture.id(), image, uv, Color32::WHITE);
    }
    if let Some(problem) = &window.problem {
        let galley = ui.painter().layout(problem.clone(), theme::interface(12.0), theme::ACCENT_RED, side - 32.0);
        let position = Pos2::new(image.center().x - galley.size().x / 2.0, image.center().y - galley.size().y / 2.0);
        ui.painter().galley(position, galley, theme::ACCENT_RED);
    }
    let saved = transforms_of(editor, window.node);
    let transforms: Vec<PreviewTransform> = saved.iter().map(PreviewTransform::from_value).collect();
    let painter = ui.painter_at(rect);
    // The others first, the chosen one on top.
    for (place, transform) in transforms.iter().enumerate() {
        if place == window.selected {
            continue;
        }
        let centre = image.min + Vec2::new(transform.x, transform.y) * side;
        paint_gizmo_base(&painter, centre, transform_colour(transform));
        let area = Rect::from_center_size(centre, Vec2::splat(2.0 * RING_OUTER));
        let response = ui.interact(area, Id::new(("constructor-gizmo", window.node, place)), Sense::click());
        if response.clicked() || response.drag_started() {
            // `G_Activated`: it becomes the chosen terrain.
            window.selected = place;
        }
    }
    let Some(transform) = transforms.get(window.selected) else {
        return;
    };
    if let Some(changed) = chosen_gizmo(ui, &painter, image, window, transform) {
        write_transform(editor, window, &changed);
        let finished = window.drag.is_none();
        if finished && window.update_viewport {
            // `Prime_ChangeCompleted`: the node builds.
            editor.changed();
            window.unbuilt = false;
        }
    }
}

/// The base of every gizmo: its shadow, rings and colour.
fn paint_gizmo_base(painter: &egui::Painter, centre: Pos2, colour: Color32) {
    painter.add(Shape::circle_filled(centre, RING_OUTER, Color32::from_black_alpha(0x40)));
    painter.add(Shape::circle_filled(centre, RING_OUTER * 0.6, Color32::from_black_alpha(0x30)));
    painter.circle_stroke(centre, 17.0, Stroke::new(1.0, Color32::BLACK));
    painter.circle_stroke(centre, 15.0, Stroke::new(2.0, Color32::WHITE));
    painter.circle(centre, 14.0, colour.gamma_multiply(0.4), Stroke::new(1.5, colour.gamma_multiply(0.4)));
}

/// The chosen gizmo with its handles; a changed transform while one is
/// dragged (and on the frame it is let go).
fn chosen_gizmo(ui: &mut Ui, painter: &egui::Painter, image: Rect, window: &mut ConstructorWindow, transform: &PreviewTransform) -> Option<PreviewTransform> {
    let side = image.width();
    let centre = image.min + Vec2::new(transform.x, transform.y) * side;
    let colour = if transform.colour == [0.0; 3] { GIZMO_COLOUR } else { transform_colour(transform) };
    paint_gizmo_base(painter, centre, colour);

    // The handles' places (`GizmoConstruction.xaml`).
    let rotate_centre = centre + Vec2::new(RING_OUTER + 14.0, 0.0);
    let height_centre = centre - Vec2::new(0.0, RING_OUTER + 12.0);
    let base_centre = centre + Vec2::new(0.0, RING_OUTER + 12.0);
    let shaper_centre = centre + Vec2::new(-RING_OUTER - 14.0, -8.0);
    let mask_centre = centre + Vec2::new(-RING_OUTER - 14.0, 8.0);
    let id = Id::new(("constructor-chosen", window.node));
    let ring = ui.interact(Rect::from_center_size(centre, Vec2::splat(2.0 * RING_OUTER)), id.with("scale"), Sense::drag());
    let movement = ui.interact(Rect::from_center_size(centre, Vec2::splat(32.0)), id.with("move"), Sense::drag());
    let handles = [
        (Handle::Rotate, rotate_centre, 12.0),
        (Handle::Height, height_centre, 18.0),
        (Handle::Base, base_centre, 18.0),
        (Handle::Shaper, shaper_centre, 12.0),
        (Handle::Mask, mask_centre, 12.0),
    ];
    let responses: Vec<(Handle, egui::Response)> = handles.iter().map(|(handle, at, size)| (*handle, ui.interact(Rect::from_center_size(*at, Vec2::splat(*size)), id.with(format!("{handle:?}")), Sense::drag()))).collect();
    let ring_hovered = ring.hover_pos().is_some_and(|pointer| {
        let distance = pointer.distance(centre);
        (RING_INNER - 2.0..=RING_OUTER + 2.0).contains(&distance)
    });

    // The cross, the scale ring and the handles, each in the accent under
    // the pointer.
    let tick = 18.0 / 24.0;
    for (from, to) in [((19.0, 12.0), (15.95, 12.0)), ((5.0, 12.0), (8.05, 12.0)), ((12.0, 19.0), (12.0, 15.95)), ((12.0, 5.0), (12.0, 8.05))] {
        let start = centre + Vec2::new((from.0 - 12.0) * tick, (from.1 - 12.0) * tick);
        let end = centre + Vec2::new((to.0 - 12.0) * tick, (to.1 - 12.0) * tick);
        painter.line_segment([start, end], Stroke::new(1.0, Color32::BLACK));
    }
    let middle = (RING_OUTER + RING_INNER) / 2.0;
    painter.circle_stroke(centre, middle, Stroke::new(RING_OUTER - RING_INNER + 2.0, Color32::BLACK));
    painter.circle_stroke(centre, middle, Stroke::new(RING_OUTER - RING_INNER, Color32::WHITE));
    let fill = |handle: Handle| {
        let active = responses.iter().any(|(candidate, response)| *candidate == handle && (response.hovered() || response.dragged()));
        if active { theme::ACCENT_PRIMARY } else { colour }
    };
    let triangle = |at: Pos2, down: bool| {
        let scale = 18.0 / 24.0;
        let flip = if down { -1.0 } else { 1.0 };
        let corner = |x: f32, y: f32| at + Vec2::new((x - 12.0) * scale, (y - 11.0) * scale * flip);
        vec![corner(12.0, 4.0), corner(20.0, 18.0), corner(4.0, 18.0)]
    };
    painter.add(Shape::convex_polygon(triangle(height_centre, false), fill(Handle::Height), Stroke::new(1.5, Color32::BLACK)));
    painter.add(Shape::convex_polygon(triangle(base_centre, true), fill(Handle::Base), Stroke::new(1.5, Color32::BLACK)));
    for (handle, at) in [(Handle::Rotate, rotate_centre), (Handle::Shaper, shaper_centre), (Handle::Mask, mask_centre)] {
        painter.circle(at, 6.0, fill(handle), Stroke::new(1.0, Color32::BLACK));
    }

    // Cursors: `SizeAll`, `SizeNS`, `SizeWE`.
    let hovered = |handle: Handle| responses.iter().any(|(candidate, response)| *candidate == handle && response.hovered());
    if movement.hovered() || movement.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::Move);
    } else if hovered(Handle::Height) || hovered(Handle::Base) || hovered(Handle::Rotate) {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
    } else if hovered(Handle::Shaper) || hovered(Handle::Mask) || ring_hovered {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }

    // `Handle_MouseLeftButtonDown`.
    let pointer = ui.input(|input| input.pointer.interact_pos());
    if window.drag.is_none()
        && let Some(pointer) = pointer
    {
        let pressed_handle = responses.iter().find(|(_, response)| response.drag_started_by(egui::PointerButton::Primary)).map(|(handle, _)| *handle);
        let handle = pressed_handle.or_else(|| movement.drag_started_by(egui::PointerButton::Primary).then_some(Handle::Move)).or_else(|| (ring.drag_started_by(egui::PointerButton::Primary) && ring_hovered).then_some(Handle::Scale));
        if let Some(handle) = handle {
            let offset = (pointer - centre) / side;
            let scale_direction = if pointer.x < centre.x { -1.0 } else { 1.0 };
            window.drag = Some(GizmoDrag { handle, start: pointer, start_transform: transform.clone(), offset, scale_direction });
        }
    }
    let drag = window.drag.clone()?;
    let modifiers = ui.input(|input| input.modifiers);
    let mut changed = transform.clone();
    if let Some(pointer) = pointer {
        let moved = pointer - drag.start;
        let start = &drag.start_transform;
        // Up and left raise these, a quarter as fast with Shift, in
        // twentieths with Alt.
        let nudged = |from: f32, distance: f32, low: f32, high: f32| {
            let step = if modifiers.shift { HEIGHT_STEP * 0.25 } else { HEIGHT_STEP };
            let mut value = from - distance * step;
            if modifiers.alt {
                value = (value / 0.05).round() * 0.05;
            }
            value.clamp(low, high)
        };
        match drag.handle {
            Handle::Move => {
                let mut target = (pointer - image.min) / side - drag.offset;
                if modifiers.shift {
                    let origin = Vec2::new(start.x, start.y);
                    target = origin + (target - origin) * 0.5;
                }
                changed.x = target.x.clamp(0.0, 1.0);
                changed.y = target.y.clamp(0.0, 1.0);
            }
            Handle::Height => changed.z = nudged(start.z, moved.y, 0.0, 2.0),
            Handle::Base => changed.base = nudged(start.base, moved.x, 0.0, 1.0),
            Handle::Shaper => changed.shaper = nudged(start.shaper, moved.x, -1.0, 1.0),
            Handle::Mask => changed.mask_bias = nudged(start.mask_bias, moved.x, 0.0, 1.0),
            Handle::Scale => {
                let step = if modifiers.alt { SCALE_STEP * 0.25 } else { SCALE_STEP };
                let mut scale = (start.scale + moved.x * drag.scale_direction * step).max(0.01);
                if modifiers.shift {
                    scale = ((scale / 0.05).round() * 0.05).max(0.01);
                }
                changed.scale = scale;
            }
            Handle::Rotate => {
                let step = if modifiers.shift { ROTATE_STEP * 0.25 } else { ROTATE_STEP };
                let raw = (start.rotate - moved.y * step).rem_euclid(360.0);
                changed.rotate = if modifiers.ctrl || modifiers.command { (raw / ROTATE_SNAP).round() * ROTATE_SNAP } else { raw };
            }
        }
    }
    if !ui.input(|input| input.pointer.primary_down()) {
        window.drag = None;
    }
    Some(changed)
}

/// `Gizmo2DHit` and the settings: the transform at the chosen place takes
/// `changed`'s pose; the edit's first change is its undo step.
fn write_transform(editor: &mut Editor, window: &mut ConstructorWindow, changed: &PreviewTransform) {
    let pose = [
        ("X", changed.x),
        ("Y", changed.y),
        ("Z", changed.z),
        ("Base", changed.base),
        ("Shaper", changed.shaper),
        ("Rotate", changed.rotate),
        ("Scale", changed.scale),
        ("MaskBias", changed.mask_bias),
    ];
    edit_transforms(editor, window, |transforms, place| {
        let Some(Value::Object(members)) = transforms.get_mut(place) else {
            return;
        };
        for (name, value) in pose {
            members.insert(name.to_string(), f32_value(value));
        }
    });
}

/// Changes the saved transforms (`f` gets them and the chosen place),
/// quietly after the edit's first change.
fn edit_transforms(editor: &mut Editor, window: &mut ConstructorWindow, f: impl FnOnce(&mut Vec<Value>, usize)) {
    let mut transforms = transforms_of(editor, window.node);
    let before = transforms.clone();
    f(&mut transforms, window.selected);
    if transforms == before {
        return;
    }
    if !window.editing {
        editor.document.checkpoint("Constructor");
        window.editing = true;
    }
    let values = vec![("Transforms".to_string(), Value::Array(transforms))];
    editor.document.set_values_quietly(window.node, &values);
    window.unbuilt = true;
}

/// The toolbar, the terrains and the chosen one's settings.
fn side_panel(ui: &mut Ui, editor: &mut Editor, window: &mut ConstructorWindow) {
    ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
    toolbar(ui, window);
    ui.add_space(4.0);
    terrain_list(ui, editor, window);
    ui.add_space(8.0);
    terrain_editor(ui, editor, window);
}

fn toolbar(ui: &mut Ui, window: &mut ConstructorWindow) {
    let mut frame = egui::Frame::new();
    frame.fill = theme::PANEL_CLADDING;
    frame.stroke = Stroke::new(1.5, theme::MENU_BORDER);
    frame.corner_radius = egui::CornerRadius::same(4);
    frame.inner_margin = egui::Margin::same(4);
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            // `tglColorize`, `btnRefresh`.
            if icon_text_button(ui, "group-palette", "Colorize Previews", Some(window.colorize)).on_hover_text("Colorize Terrains").clicked() {
                window.colorize = !window.colorize;
            }
            ui.separator();
            if icon_text_button(ui, "rotate-360", "Refresh Previews", None).clicked() {
                window.refresh = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // `tglUpdateViewport`, then the Preview Resolution menu.
                let toggled = crate::ui::widgets::icon_toggle(ui, "cube-plus", "Update 3D Viewport", window.update_viewport, theme::ACCENT_PRIMARY);
                if toggled.clicked() {
                    window.update_viewport = !window.update_viewport;
                }
                icon_menu(ui, "cube", "Preview Resolution", None, |ui| {
                    for size in PREVIEW_SIZES {
                        if check_item(ui, window.preview_size == size, &format!("{size} x {size}"), "").clicked() {
                            window.preview_size = size;
                        }
                    }
                    menu_separator(ui);
                    // "Faster Previews" ticked renders faster.
                    if check_item(ui, window.faster, "Faster Previews", "").clicked() {
                        window.faster = !window.faster;
                    }
                });
            });
        });
    });
}

/// `ToggleButton-IconXText` / `Button-IconText`: an icon and its text;
/// `on` for a toggle.
fn icon_text_button(ui: &mut Ui, icon: &str, text: &str, on: Option<bool>) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::interface(12.0), theme::LABEL_DEFAULT);
    let size = Vec2::new(16.0 + 6.0 + galley.size().x + 16.0, 32.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let checked = on == Some(true);
    if checked {
        ui.painter().rect_filled(rect, 4.0, theme::BUTTON_PRESSED);
        ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.0, theme::ACCENT_PRIMARY), StrokeKind::Inside);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 4.0, theme::BUTTON_HOVER);
    }
    let tint = if checked || response.hovered() { theme::LABEL_EMPHASIS } else { theme::ICON_DEFAULT };
    let icon_rect = Rect::from_min_size(Pos2::new(rect.left() + 8.0, rect.center().y - 8.0), Vec2::splat(icons::DEFAULT_SIZE));
    icons::paint(ui, icon_rect, icon, tint);
    let text_position = Pos2::new(icon_rect.right() + 6.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_position, galley, tint);
    response
}

/// `list`: a row a terrain, in the order they are placed: its colour, its
/// title and its blend. A row dragged onto another takes its place.
fn terrain_list(ui: &mut Ui, editor: &mut Editor, window: &mut ConstructorWindow) {
    let saved = transforms_of(editor, window.node);
    let transforms: Vec<PreviewTransform> = saved.iter().map(PreviewTransform::from_value).collect();
    let mut frame = egui::Frame::new();
    frame.fill = Color32::from_rgb(0x16, 0x16, 0x16);
    frame.stroke = Stroke::new(1.0, theme::LISTBOX_BORDER);
    let mut rows = Vec::with_capacity(transforms.len());
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_min_height(140.0);
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        for (place, transform) in transforms.iter().enumerate() {
            let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click_and_drag());
            if place == window.selected {
                ui.painter().rect_filled(rect, 0.0, theme::BUTTON_PRESSED);
            } else if response.hovered() {
                ui.painter().rect_filled(rect, 0.0, theme::BUTTON_HOVER);
            }
            let swatch = Rect::from_min_size(Pos2::new(rect.left() + 8.0, rect.center().y - 7.0), Vec2::splat(14.0));
            ui.painter().rect_filled(swatch, 2.0, transform_colour(transform));
            ui.painter().rect_stroke(swatch, 2.0, Stroke::new(1.0, Color32::from_black_alpha(0x66)), StrokeKind::Inside);
            let title = terrain_title(editor, window.node, transform, place);
            let galley = ui.painter().layout_no_wrap(title, theme::semibold(12.0), theme::LABEL_DEFAULT);
            ui.painter().galley(Pos2::new(swatch.right() + 10.0, rect.center().y - galley.size().y / 2.0), galley, theme::LABEL_DEFAULT);
            let blend = ui.painter().layout_no_wrap(transform.blend_mode.clone(), theme::interface(12.0), theme::ACCENT_GRAY);
            ui.painter().galley(Pos2::new(rect.right() - 8.0 - blend.size().x, rect.center().y - blend.size().y / 2.0), blend, theme::ACCENT_GRAY);
            if response.clicked() {
                window.selected = place;
            }
            if response.drag_started_by(egui::PointerButton::Primary) {
                window.row_drag = Some(place);
            }
            rows.push(rect);
        }
    });
    let Some(dragged) = window.row_drag else {
        return;
    };
    let pointer = ui.input(|input| input.pointer.interact_pos());
    let target = pointer.and_then(|pointer| rows.iter().position(|row| row.contains(pointer)));
    if let (Some(target), Some(row)) = (target, target.and_then(|index| rows.get(index)))
        && target != dragged
    {
        ui.painter().hline(row.x_range(), row.top(), Stroke::new(2.0, theme::ACCENT_PRIMARY));
    }
    if ui.input(|input| input.pointer.primary_down()) {
        return;
    }
    window.row_drag = None;
    let Some(target) = target.filter(|target| *target != dragged) else {
        return;
    };
    // `Drop`: the terrain moves to its new place; each placement stays with
    // its terrain.
    let mut transforms = saved;
    let moved = transforms.remove(dragged);
    transforms.insert(target.min(transforms.len()), moved);
    editor.document.checkpoint("Constructor");
    let values = vec![("Transforms".to_string(), Value::Array(transforms))];
    editor.document.set_values_quietly(window.node, &values);
    window.selected = target;
    window.unbuilt = true;
}

/// `TerrainTransformEditor` for the chosen terrain (margin 16, in its
/// place's colour): X, Y and Z; Rotate and Scale; Base, Shaper and Blend
/// Mode; Mask and Mask Bias.
fn terrain_editor(ui: &mut Ui, editor: &mut Editor, window: &mut ConstructorWindow) {
    let saved = transforms_of(editor, window.node);
    let Some(value) = saved.get(window.selected) else {
        return;
    };
    let transform = PreviewTransform::from_value(value);
    let colour = brush(window.selected);
    // `ShowEditor`: a terrain without a colour takes its place's.
    if transform.colour == [0.0; 3] {
        let red = colour.r() as f32 / 255.0;
        let green = colour.g() as f32 / 255.0;
        let blue = colour.b() as f32 / 255.0;
        let place = window.selected;
        let mut transforms = saved.clone();
        if let Some(Value::Object(members)) = transforms.get_mut(place) {
            members.insert("Color".to_string(), json!({"R": f32_value(red), "G": f32_value(green), "B": f32_value(blue)}));
        }
        let values = vec![("Transforms".to_string(), Value::Array(transforms))];
        editor.document.set_values_quietly(window.node, &values);
    }
    let mut changed = transform.clone();
    let mut blend = None;
    let mut mask = None;
    let mut frame = egui::Frame::new();
    frame.inner_margin = egui::Margin::same(16);
    frame.stroke = Stroke::new(1.0, colour);
    frame.corner_radius = egui::CornerRadius::same(4);
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        let title = terrain_title(editor, window.node, &transform, window.selected);
        let label = egui::RichText::new(title).font(theme::semibold(13.0)).color(colour);
        ui.label(label);
        ui.add_space(8.0);
        let id = Id::new(("constructor-editor", window.node, window.selected));
        let curved_scale = SliderSpec { curve: 3.0, default: Some(1.0), ..SliderSpec::linear(0.0001, 10.0) };
        ui.columns(3, |columns| {
            slide(&mut columns[0], id.with("X"), "X", &mut changed.x, &SliderSpec { default: Some(0.0), ..SliderSpec::linear(0.0, 1.0) }, theme::ACCENT_MUTED_RED);
            slide(&mut columns[1], id.with("Y"), "Y", &mut changed.y, &SliderSpec { default: Some(0.0), ..SliderSpec::linear(0.0, 1.0) }, theme::ACCENT_MUTED_GREEN);
            slide(&mut columns[2], id.with("Z"), "Z", &mut changed.z, &SliderSpec { default: Some(1.0), ..SliderSpec::linear(0.0, 2.0) }, theme::ACCENT_MUTED_BLUE);
        });
        ui.add_space(16.0);
        ui.columns(2, |columns| {
            slide(&mut columns[0], id.with("Rotate"), "Rotate", &mut changed.rotate, &SliderSpec { default: Some(0.0), ..SliderSpec::linear(0.0, 360.0) }, colour);
            slide(&mut columns[1], id.with("Scale"), "Scale", &mut changed.scale, &curved_scale, colour);
        });
        ui.add_space(16.0);
        slide(ui, id.with("Base"), "Base", &mut changed.base, &SliderSpec { default: Some(0.0), ..SliderSpec::linear(0.0, 1.0) }, theme::ACCENT_MUTED_TEAL);
        slide(ui, id.with("Shaper"), "Shaper", &mut changed.shaper, &SliderSpec { default: Some(0.0), ..SliderSpec::linear(-1.0, 1.0) }, theme::ACCENT_MUTED_PRIMARY);
        ui.add_space(8.0);
        let blends: Vec<(String, String)> = ["Max", "Min", "Add", "Subtract"].iter().map(|name| (name.to_string(), name.to_string())).collect();
        blend = editors::dropdown_row(ui, id.with("blend"), "Blend Mode", &blends, &transform.blend_mode, &transform.blend_mode);
        ui.add_space(16.0);
        ui.columns(2, |columns| {
            let masks: Vec<(String, String)> = ["None", "Square", "Round", "Soft"].iter().map(|name| (name.to_string(), name.to_string())).collect();
            mask = editors::dropdown_row(&mut columns[0], id.with("mask"), "Mask", &masks, &transform.mask, &transform.mask);
            slide(&mut columns[1], id.with("MaskBias"), "Mask Bias", &mut changed.mask_bias, &SliderSpec { default: Some(0.25), ..SliderSpec::linear(0.0, 1.0) }, colour);
        });
    });
    if changed != transform {
        write_transform(editor, window, &changed);
    }
    for (name, picked) in [("BlendMode", blend), ("Mask", mask)] {
        let Some(picked) = picked else {
            continue;
        };
        edit_transforms(editor, window, |transforms, place| {
            if let Some(Value::Object(members)) = transforms.get_mut(place) {
                members.insert(name.to_string(), json!(picked));
            }
        });
        // A pick is a whole edit.
        window.editing = false;
    }
}

/// A `SliderEditor` over one of the transform's numbers.
fn slide(ui: &mut Ui, id: Id, label: &str, value: &mut f32, spec: &SliderSpec, colour: Color32) {
    if let Some(changed) = editors::slider(ui, id, label, *value as f64, spec, colour) {
        *value = changed as f32;
    }
}
