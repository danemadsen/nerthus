//! The 2D viewport (`Viewport2D`): the viewport renderer's image of a node
//! seen from above, zoomed with the wheel (×1.1 a notch) and panned with
//! the middle button, double-click switching between the whole map and
//! one pixel per pixel (`ZoomBorder`). Docked (F6, `Toggle2DViewport`) it
//! sits square between the 3D view and the toolbar and shows what the 3D
//! view shows; its menu also picks heights and colours and opens the
//! measurement tools. Floating (Ctrl F6, `Show2DView(node)`: a
//! `MapViewer` window titled with the node's name) it shows one node,
//! whatever is selected, as builds leave it. Ctrl M opens one of those for
//! the selected node, or with nothing selected shows or hides the Map
//! Viewer (`Show2DView()`), which follows the preview: what the 3D view
//! shows, or another output of the previewed node picked from its menu.
//! Every menu saves the image (Save to file) and shows any of the node's
//! outputs.

use std::sync::Arc;

use egui::{Color32, ColorImage, CursorIcon, Id, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, Ui, Vec2};
use nerthus_engine::Map;
use nerthus_render::TerrainColours;

use crate::editor::{Editor, RenderSource};
use crate::ui::gizmos::{gizmo_layer, GizmoKind, GizmoState};
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::tool_window;
use crate::ui::widgets::{check_item, icon_menu, menu_caption, menu_item, menu_separator};
use crate::viewport_render::{MaskEnhancement, RenderJob, RenderQuality, ViewportRenderer};

/// `ZoomSpeed`.
const ZOOM_STEP: f32 = 1.1;
/// The zoom border's limits, as multiples of the fitted size.
const MIN_ZOOM: f32 = 0.2;
const MAX_ZOOM: f32 = 40.0;
/// `mainGrid`'s background.
const BACKGROUND: Color32 = Color32::from_rgb(0x18, 0x18, 0x18);
/// The readouts' backing (`#66000000`).
const READOUT_BACKING: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 0x66);
/// `MapViewer`: 720 square, and its title when it shows no one node.
const FLOATING_SIZE: f32 = 720.0;
const FOLLOWING_TITLE: &str = "Map Viewer";
/// The docked column's narrowest.
const MIN_DOCKED: f32 = 120.0;

/// How one view is zoomed and what it is doing.
#[derive(Clone)]
pub struct View2D {
    /// Multiples of the fitted size (1 = the whole map).
    zoom: f32,
    /// The image's centre from the view's, in points.
    pan: Vec2,
    /// `fixedZoom`: the map fills the view (and keeps doing so as it
    /// resizes).
    fixed: bool,
    /// Picking heights and colours (`IsPicking`) and measuring
    /// (`IsMeasuring`); the measure's two ends (fractions of the map) and
    /// whether it reads metres (`tglMetric`).
    picking: bool,
    measuring: bool,
    measure: [Pos2; 2],
    metric: bool,
    texture: Option<(usize, TextureHandle)>,
}

impl Default for View2D {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: Vec2::ZERO,
            fixed: true,
            picking: false,
            measuring: false,
            // `thumb1` and `thumb2` start at (222, 130) and (427, 235) of a
            // 1024 image (+10 to their centres).
            measure: [Pos2::new(232.0 / 1024.0, 140.0 / 1024.0), Pos2::new(437.0 / 1024.0, 245.0 / 1024.0)],
            metric: false,
            texture: None,
        }
    }
}

/// A floating view of one node (`MapViewer` with its `NodeId`).
pub struct FloatingView {
    node: i32,
    port: String,
    view: View2D,
    renderer: ViewportRenderer,
    /// What was last sent to the renderer: the watched maps' revision and
    /// the render settings and port, and the generation it got; whether it
    /// was lit quickly (a gizmo or the painter was held).
    sent: Option<(u64, String)>,
    generation: u64,
    rendered_quickly: bool,
    colours: Option<Arc<TerrainColours>>,
    open: bool,
}

/// The Map Viewer that follows the preview (`O.MainMapViewer`): the 3D
/// view's render, or the previewed node's output picked from its menu
/// (`RenderPort`), rendered for it alone until another node is previewed
/// (`G_AfterNodeChanged`).
pub struct FollowingView {
    view: View2D,
    /// The output picked, and the node it was picked for.
    picked: Option<(i32, String)>,
    renderer: ViewportRenderer,
    /// What the picked output's last render was made from, the generation
    /// it got, and whether it was lit quickly.
    sent: Option<String>,
    generation: u64,
    rendered_quickly: bool,
    colours: Option<Arc<TerrainColours>>,
}

impl FollowingView {
    fn new() -> Self {
        Self {
            view: View2D::default(),
            picked: None,
            renderer: ViewportRenderer::start(),
            sent: None,
            generation: 0,
            rendered_quickly: false,
            colours: None,
        }
    }
}

/// A sample `HeightPicked` reports (`Viewport2D_HeightPicked`): the values
/// and the terrain height they are read against.
#[derive(Clone, Debug)]
struct Picked {
    values: Vec<f32>,
    terrain_height: f32,
}

/// Everything 2D: the docked view and its column, and the floating views.
#[derive(Default)]
pub struct Viewport2DState {
    /// The docked view is shown (`ViewportArea.MapViewer`).
    pub docked: bool,
    /// The docked column's width once the splitter moved it.
    docked_width: Option<f32>,
    docked_view: View2D,
    /// Where the docked view was last drawn (the Lazy Menu moves the
    /// pointer there).
    docked_rect: Option<Rect>,
    floating: Vec<FloatingView>,
    following: Option<FollowingView>,
    picked: Option<Picked>,
    /// The gizmo on the docked view (`Gizmo2DLayer`/`GizmoSRTLayer`), and
    /// whether Toggle Gizmo was on last frame.
    gizmo: Option<GizmoState>,
    gizmo_was_on: bool,
    gizmo_at_launch: bool,
    following_at_launch: bool,
    /// The node selected last frame.
    last_active: Option<i32>,
}

impl Viewport2DState {
    /// `Toggle2DViewport` (F6).
    pub fn toggle_docked(&mut self) {
        self.docked = !self.docked;
        if !self.docked {
            self.docked_view.picking = false;
            self.docked_view.measuring = false;
            self.docked_rect = None;
        }
    }

    /// Where the docked view is, while it shows.
    pub fn docked_rect(&self) -> Option<Rect> {
        self.docked_rect.filter(|_| self.docked)
    }

    /// `BtnPickHeight_OnClick`: the docked view opened if needed, then
    /// picking toggled (`TogglePicking`).
    pub fn toggle_picking(&mut self, editor: &Editor) {
        self.docked = true;
        let view = &mut self.docked_view;
        if editor.active_node().is_none() {
            view.picking = false;
            return;
        }
        view.picking = !view.picking;
        if view.picking {
            view.measuring = false;
        }
    }

    /// `BtnMeasure_OnClick`: the docked view opened if needed, then the
    /// measurement tools toggled (`ToggleMeasuring`).
    pub fn toggle_measuring(&mut self, editor: &Editor) {
        self.docked = true;
        let view = &mut self.docked_view;
        if editor.active_node().is_none() {
            view.measuring = false;
            return;
        }
        view.measuring = !view.measuring;
        if view.measuring {
            view.picking = false;
        }
    }

    pub fn is_picking(&self) -> bool {
        self.docked && self.docked_view.picking
    }

    pub fn is_measuring(&self) -> bool {
        self.docked && self.docked_view.measuring
    }

    /// `Show2DView(node)` (Ctrl F6): a floating view of the node, unless
    /// one is open already.
    pub fn open_floating(&mut self, node: i32) {
        if self.floating.iter().any(|view| view.node == node) {
            return;
        }
        let view = FloatingView {
            node,
            port: "Out".to_string(),
            view: View2D::default(),
            renderer: ViewportRenderer::start(),
            sent: None,
            generation: 0,
            rendered_quickly: false,
            colours: None,
            open: true,
        };
        self.floating.push(view);
    }

    /// Ctrl M: a floating view of the selected node (`Show2DView(node)`),
    /// or with none selected the Map Viewer shown or hidden
    /// (`Show2DView()`).
    pub fn show_2d_view(&mut self, editor: &Editor) {
        if let Some(node) = editor.active_node() {
            self.open_floating(node);
            return;
        }
        self.following = match self.following {
            Some(_) => None,
            None => Some(FollowingView::new()),
        };
    }

    /// Whether the docked view's gizmo is being dragged.
    pub fn gizmo_held(&self) -> bool {
        self.gizmo.as_ref().is_some_and(GizmoState::held)
    }

    /// The nodes the floating views show, and the input under a Snow
    /// gizmo (what the builds report back).
    fn watched_nodes(&self, editor: &Editor) -> Vec<i32> {
        let mut nodes: Vec<i32> = self.floating.iter().map(|view| view.node).collect();
        if let Some(source) = self.gizmo.as_ref().and_then(|gizmo| snow_source(editor, gizmo)) {
            nodes.push(source);
        }
        nodes
    }

    /// `ToggleGizmo`: the gizmo follows Toggle Gizmo and the node it is on,
    /// and turning it on opens the docked view.
    fn follow_gizmo(&mut self, editor: &Editor) {
        let on = editor.gizmo_active;
        let node = editor.active_node();
        let kind = node.and_then(|node| editor.document.node(node)).and_then(|data| GizmoKind::of(&data.type_name));
        match (on, node, kind) {
            (true, Some(node), Some(kind)) => {
                if !self.gizmo_was_on {
                    self.docked = true;
                }
                let current = self.gizmo.as_ref().is_some_and(|gizmo| gizmo.node() == node && gizmo.kind() == kind);
                if !current {
                    self.gizmo = Some(GizmoState::new(node, kind));
                }
            }
            _ => self.gizmo = None,
        }
        self.gizmo_was_on = on;
    }

    /// `G_AfterNodeChanged`: picking and measuring stop when another node is
    /// selected. Neither can be on without a node, so the first selection
    /// (the node `--show` opened them for) leaves them alone.
    fn follow_selection(&mut self, editor: &Editor) {
        let active = editor.active_node();
        if active != self.last_active && self.last_active.is_some() {
            self.docked_view.picking = false;
            self.docked_view.measuring = false;
        }
        self.last_active = active;
    }

    /// The docked view's picture belongs to the window it was drawn in;
    /// moving the viewport to another window makes it again.
    pub fn forget_pictures(&mut self) {
        self.docked_view.texture = None;
    }

    /// `--show`: what to open once the editor runs (`viewport-2d`,
    /// `map-viewer`, `gizmo`, `pick`, `measure`).
    pub fn open_at_launch(&mut self, what: &str) {
        match what {
            "viewport-2d" => self.docked = true,
            "map-viewer" => self.following_at_launch = true,
            "gizmo" => self.gizmo_at_launch = true,
            "pick" => {
                self.docked = true;
                self.docked_view.picking = true;
            }
            "measure" => {
                self.docked = true;
                self.docked_view.measuring = true;
            }
            _ => {}
        }
    }
}

/// The docked column: square (`viewportColumn` as wide as the area is high,
/// less 8) unless the splitter moved it, between the 3D view and the
/// toolbar. Returns the 3D view's part of `area`.
pub fn dock(ui: &mut Ui, area: Rect, editor: &Editor, state: &mut Viewport2DState, commands: &mut Vec<Command>) -> Rect {
    if !state.docked {
        return area;
    }
    let wanted = state.docked_width.unwrap_or(area.height() - 8.0);
    let width = wanted.clamp(MIN_DOCKED.min(area.width() * 0.5), (area.width() - MIN_DOCKED).max(MIN_DOCKED));
    let column = Rect::from_min_max(Pos2::new(area.right() - width, area.top()), area.max);
    // `splitter`: 4 wide on the column's left edge.
    let splitter = Rect::from_min_size(Pos2::new(column.left() - 2.0, column.top()), Vec2::new(4.0, column.height()));
    let response = ui.interact(splitter, Id::new("viewport-2d-splitter"), Sense::drag());
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if response.dragged() {
        state.docked_width = Some(width - response.drag_delta().x);
    }
    state.docked_rect = Some(column);
    show_docked(ui, column, editor, state, commands);
    Rect::from_min_max(area.min, Pos2::new(column.left() - 2.0, area.bottom()))
}

/// The node whose output a Snow gizmo reads (`In`).
fn snow_source(editor: &Editor, gizmo: &GizmoState) -> Option<i32> {
    let node = editor.document.node(gizmo.node())?;
    if node.type_name != "Snow" {
        return None;
    }
    node.port("In")?.record.as_ref().map(|record| record.from)
}

/// The map under a Snow gizmo (`In.GetMap()`), as the builds left it.
fn snow_input(editor: &Editor, gizmo: &GizmoState) -> Option<Arc<Map>> {
    let node = editor.document.node(gizmo.node())?;
    let record = node.port("In")?.record.as_ref()?;
    let watched = editor.watched_outputs.get(&record.from)?;
    watched.outputs.iter().find(|(port, _)| *port == record.from_port).map(|(_, map)| map.clone())
}

/// The docked view (`IsMain`): the 3D view's render, its port menu,
/// picking, measuring and the gizmo.
fn show_docked(ui: &mut Ui, rect: Rect, editor: &Editor, state: &mut Viewport2DState, commands: &mut Vec<Command>) {
    let colours = editor.render_colours.clone();
    let view = &mut state.docked_view;
    upload(ui.ctx(), view, colours.as_ref(), "viewport-2d-docked");
    let id = Id::new("viewport-2d-docked");
    let node = editor.preview_node;
    let ports = node.map(|node| output_ports(editor, node)).unwrap_or_default();
    let reference = if view.picking || view.measuring { pick_reference(editor) } else { None };
    let mut menu = ViewMenu::default();
    let resolution = colours.as_ref().map(|colours| colours.width).unwrap_or(1);
    view_contents(ui, rect, id, view, resolution, &mut menu, |ui, menu| {
        context_menu(ui, menu, &ports, &editor.preview_port, true, colours.is_some());
    });
    let painter = ui.painter_at(rect);
    let image = image_rect(rect, view);
    if view.measuring {
        measure_overlay(ui, &painter, rect, image, view, reference.as_deref(), editor);
    }
    if view.picking {
        if let Some(reference) = &reference {
            pick_readout(ui, &painter, rect, image, reference, editor);
        }
        let response = ui.interact(image.intersect(rect), id.with("pick"), Sense::click());
        if response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        }
        if response.clicked()
            && let (Some(pointer), Some(reference)) = (response.interact_pointer_pos(), &reference)
        {
            // `OnMouseLeftButtonUp` → `HeightPicked`, which stops picking.
            let position = (pointer - image.min) / image.size();
            let values = sample(reference, position.x, position.y);
            let terrain_height = editor.document.terrain().height;
            view.picking = false;
            state.picked = Some(Picked { values, terrain_height });
        }
    }
    let zoom = view.zoom;
    if let Some(gizmo) = state.gizmo.as_mut()
        && let Some(node) = editor.document.node(gizmo.node())
    {
        let input = snow_input(editor, gizmo);
        let edit = gizmo_layer(ui, image, rect, zoom, node, gizmo, input.as_deref());
        if let Some(edit) = edit {
            let changes = edit.values.iter().any(|(name, value)| node.values.get(name) != Some(value));
            if changes || edit.started || edit.finished {
                commands.push(Command::GizmoEdit(edit));
            }
        }
    }
    // Enter ends picking and measuring (`Viewport2D_KeyUp`).
    if ui.input(|input| input.key_released(egui::Key::Enter)) {
        view.picking = false;
        view.measuring = false;
    }
    // `RenderPort` in the docked view: the 3D view shows the port too.
    if let Some(port) = apply_menu(&menu, colours, commands) {
        commands.push(Command::SetPreviewPort(port));
    }
    if menu_toggles(&menu) {
        // Pick and measure leave each other off.
        if menu.pick {
            view.measuring = false;
            view.picking = !view.picking && node.is_some();
        }
        if menu.measure {
            view.picking = false;
            view.measuring = !view.measuring && node.is_some();
        }
    }
}

/// The floating views (`MapViewer`s), each in its own window titled with
/// its node's name, and the Map Viewer; the editor keeps their nodes' maps
/// coming. `also_watched` are other nodes whose maps are wanted (the
/// Constructor's inputs).
pub fn show_floating(ctx: &egui::Context, editor: &mut Editor, state: &mut Viewport2DState, also_watched: &[i32], commands: &mut Vec<Command>) {
    state.floating.retain(|view| view.open && editor.document.node(view.node).is_some());
    if std::mem::take(&mut state.gizmo_at_launch) {
        editor.gizmo_active = true;
    }
    // `--show map-viewer`, once the first render is in.
    if state.following_at_launch && editor.rendered() {
        state.following_at_launch = false;
        state.following = Some(FollowingView::new());
    }
    state.follow_selection(editor);
    state.follow_gizmo(editor);
    let mut watched = state.watched_nodes(editor);
    for node in also_watched {
        if !watched.contains(node) {
            watched.push(*node);
        }
    }
    editor.watch_nodes(watched);
    for floating in &mut state.floating {
        refresh_floating(editor, floating);
        let title = editor.document.node(floating.node).map(|node| node.name.clone()).unwrap_or_default();
        let window_id = Id::new(("map-viewer", floating.node));
        let mut open = true;
        let window = map_viewer_window(ctx, window_id, title, &mut open);
        let ports = output_ports(editor, floating.node);
        let colours = floating.colours.clone();
        let texture_name = format!("map-viewer-{}", floating.node);
        let mut menu = ViewMenu::default();
        window.show(ctx, |ui| {
            let port = floating.port.clone();
            window_view(ui, window_id.with("view"), &mut floating.view, colours.as_ref(), &texture_name, &mut menu, |ui, menu| {
                context_menu(ui, menu, &ports, &port, false, colours.is_some());
            });
        });
        if !open {
            floating.open = false;
        }
        if let Some(port) = apply_menu(&menu, colours, commands) {
            // `RenderPort` for a floating view: only it renders the port.
            floating.port = port;
            floating.sent = None;
        }
    }
    show_following(ctx, editor, state, commands);
    picked_dialog(ctx, state, commands);
}

/// A `MapViewer` window: 720 square (its title bar included) in the
/// middle of the shell's body, on top of the shell.
fn map_viewer_window<'open>(ctx: &egui::Context, id: Id, title: String, open: &'open mut bool) -> egui::Window<'open> {
    let mut window = egui::Window::new(title);
    window = window.open(open);
    window = window.collapsible(false);
    window = window.order(egui::Order::Foreground);
    window = window.frame(egui::Frame::window(&ctx.global_style()).fill(theme::WINDOW_DARK).inner_margin(0));
    tool_window::place(ctx, window, id, Vec2::splat(FLOATING_SIZE))
}

/// A floating view filling its window.
fn window_view(ui: &mut Ui, id: Id, view: &mut View2D, colours: Option<&Arc<TerrainColours>>, texture_name: &str, menu: &mut ViewMenu, contents: impl Fn(&mut Ui, &mut ViewMenu)) {
    let size = ui.available_size().max(Vec2::splat(64.0));
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    upload(ui.ctx(), view, colours, texture_name);
    let resolution = colours.map(|colours| colours.width).unwrap_or(1);
    view_contents(ui, rect, id, view, resolution, menu, contents);
}

/// The Map Viewer while it shows: the previewed node's outputs in its
/// menu, the one shown checked.
fn show_following(ctx: &egui::Context, editor: &Editor, state: &mut Viewport2DState, commands: &mut Vec<Command>) {
    let Some(following) = state.following.as_mut() else {
        return;
    };
    refresh_following(editor, following);
    let window_id = Id::new("map-viewer-following");
    let mut open = true;
    let window = map_viewer_window(ctx, window_id, FOLLOWING_TITLE.to_string(), &mut open);
    let ports = editor.preview_node.map(|node| output_ports(editor, node)).unwrap_or_default();
    let shown_port = match &following.picked {
        Some((_, port)) => port.clone(),
        None => editor.preview_port.clone(),
    };
    let colours = if following.picked.is_some() { following.colours.clone() } else { editor.render_colours.clone() };
    let mut menu = ViewMenu::default();
    window.show(ctx, |ui| {
        window_view(ui, window_id.with("view"), &mut following.view, colours.as_ref(), "map-viewer-following", &mut menu, |ui, menu| {
            context_menu(ui, menu, &ports, &shown_port, false, colours.is_some());
        });
    });
    if let Some(port) = apply_menu(&menu, colours, commands) {
        // `RenderPort`: another output is rendered here alone; the one the
        // 3D view shows is followed again.
        following.picked = match editor.preview_node {
            Some(node) if port != editor.preview_port => Some((node, port)),
            _ => None,
        };
        following.sent = None;
    }
    if !open {
        state.following = None;
    }
}

/// Renders the Map Viewer's picked output when its map or the render
/// settings changed, or when it was lit quickly and nothing is held any
/// more, and takes in what came back. Another node previewed drops the
/// pick.
fn refresh_following(editor: &Editor, following: &mut FollowingView) {
    if let Some(view) = following.renderer.poll()
        && view.generation == following.generation
    {
        following.colours = view.colours;
    }
    let previewed = editor.preview_node;
    if following.picked.as_ref().is_some_and(|(node, _)| Some(*node) != previewed) {
        following.picked = None;
        following.sent = None;
        following.colours = None;
    }
    let Some((_, port)) = following.picked.clone() else {
        return;
    };
    let Some((_, map)) = editor.outputs.iter().find(|(name, _)| *name == port) else {
        return;
    };
    let settings = format!("{}|{:?}|{:?}|{:?}|{:?}", Arc::as_ptr(map) as usize, editor.render_mode, editor.mask_enhancement, editor.mask_gradient, editor.lighting);
    let lit_again = following.rendered_quickly && editor.render_quality() == RenderQuality::Normal;
    if following.sent.as_deref() == Some(settings.as_str()) && !lit_again {
        return;
    }
    let mut source = editor.preview_source();
    source.port = &port;
    following.generation += 1;
    let job = editor.render_job_for(&source, following.generation);
    following.rendered_quickly = job.as_ref().is_some_and(RenderJob::lit_quickly);
    if let Some(job) = job {
        following.renderer.request(job);
    }
    following.sent = Some(settings);
}

/// Sends a floating view's node to its renderer when its maps, its port or
/// the render settings changed, or when it was lit quickly and nothing is
/// held any more, and takes in what came back.
fn refresh_floating(editor: &Editor, floating: &mut FloatingView) {
    if let Some(view) = floating.renderer.poll()
        && view.generation == floating.generation
    {
        floating.colours = view.colours;
    }
    let Some(watched) = editor.watched_outputs.get(&floating.node) else {
        return;
    };
    let settings = format!("{:?}|{:?}|{:?}|{:?}|{}", editor.render_mode, editor.mask_enhancement, editor.mask_gradient, editor.lighting, floating.port);
    let wanted = (editor.watched_revision, settings);
    // Lit quickly while a gizmo or the painter was held: lit again once
    // nothing is.
    let lit_again = floating.rendered_quickly && editor.render_quality() == RenderQuality::Normal;
    if floating.sent.as_ref() == Some(&wanted) && !lit_again {
        return;
    }
    let source = RenderSource {
        node: Some(floating.node),
        port: &floating.port,
        outputs: &watched.outputs,
        underlay: watched.underlay.as_ref(),
        custom_underlay: watched.custom_underlay.as_ref(),
        custom_colours: watched.custom_colours.as_ref(),
        port_intents: &watched.port_intents,
    };
    floating.generation += 1;
    let job = editor.render_job_for(&source, floating.generation);
    floating.rendered_quickly = job.as_ref().is_some_and(RenderJob::lit_quickly);
    if let Some(job) = job {
        floating.renderer.request(job);
    }
    floating.sent = Some(wanted);
}

/// The node's output ports, as the menu lists them (`Node.Outs`).
fn output_ports(editor: &Editor, node: i32) -> Vec<String> {
    let Some(data) = editor.document.node(node) else {
        return Vec::new();
    };
    data.ports.iter().filter(|port| !port.is_input()).map(|port| port.name.clone()).collect()
}

/// `pickReference`: the selected node's main output (`Cache "Out"`).
fn pick_reference(editor: &Editor) -> Option<Arc<Map>> {
    let active = editor.active_node()?;
    if editor.preview_node == Some(active) {
        return editor.outputs.iter().find(|(port, _)| port == "Out").map(|(_, map)| map.clone());
    }
    let watched = editor.watched_outputs.get(&active)?;
    watched.outputs.iter().find(|(port, _)| port == "Out").map(|(_, map)| map.clone())
}

/// `GetHeight`: every channel of the map at a point (fractions of it).
fn sample(map: &Map, x: f32, y: f32) -> Vec<f32> {
    let column = ((x * map.width() as f32) as i64).clamp(0, map.width() as i64 - 1) as usize;
    let row = ((y * map.height() as f32) as i64).clamp(0, map.height() as i64 - 1) as usize;
    (0..map.channel_count()).map(|channel| map.channel(channel)[row * map.width() + column]).collect()
}

/// What a view's menu asked for this frame.
#[derive(Default)]
struct ViewMenu {
    save: bool,
    port: Option<String>,
    pick: bool,
    measure: bool,
    enhancement: Option<MaskEnhancement>,
}

fn menu_toggles(menu: &ViewMenu) -> bool {
    menu.pick || menu.measure
}

/// `RecreateMenu`: Save to file, the node's outputs, and in the docked
/// view Pick Height/Color and Measurement Tools.
fn context_menu(ui: &mut Ui, menu: &mut ViewMenu, ports: &[String], current: &str, main: bool, has_image: bool) {
    crate::ui::widgets::menu_style(ui);
    if crate::ui::widgets::menu_item_enabled(ui, Some("device-floppy"), None, "Save to file", "", has_image).clicked() {
        menu.save = true;
    }
    menu_separator(ui);
    for port in ports {
        if check_item(ui, port == current, port, "").clicked() {
            menu.port = Some(port.clone());
        }
    }
    if main {
        menu_separator(ui);
        if menu_item(ui, Some("color-picker"), Some(theme::ACCENT_MUTED_PURPLE), "Pick Height/Color", "").clicked() {
            menu.pick = true;
        }
        if menu_item(ui, Some("ruler-2"), Some(theme::ACCENT_MUTED_PURPLE), "Measurement Tools", "").clicked() {
            menu.measure = true;
        }
    }
}

/// Runs a view's menu choices: saving and the mask enhancement; returns
/// the port chosen (`RenderPort`).
fn apply_menu(menu: &ViewMenu, colours: Option<Arc<TerrainColours>>, commands: &mut Vec<Command>) -> Option<String> {
    if menu.save
        && let Some(colours) = colours
    {
        commands.push(Command::Save2DView(colours));
    }
    if let Some(enhancement) = menu.enhancement {
        commands.push(Command::SetMaskEnhancement(enhancement));
    }
    menu.port.clone()
}

/// The image's place in the view: fitted and centred at zoom 1, scaled
/// about the view's centre and moved by the pan.
fn image_rect(rect: Rect, view: &View2D) -> Rect {
    let fitted = rect.width().min(rect.height());
    let size = fitted * view.zoom;
    Rect::from_center_size(rect.center() + view.pan, Vec2::splat(size))
}

/// Takes the renderer's colours into the view's texture when they change.
fn upload(ctx: &egui::Context, view: &mut View2D, colours: Option<&Arc<TerrainColours>>, name: &str) {
    let Some(colours) = colours else {
        return;
    };
    let key = Arc::as_ptr(colours) as usize;
    if view.texture.as_ref().map(|(shown, _)| *shown) == Some(key) {
        return;
    }
    let pixel_count = colours.width * colours.height;
    if colours.rgb.len() < pixel_count * 3 {
        return;
    }
    let image = ColorImage::from_rgb([colours.width, colours.height], &colours.rgb[..pixel_count * 3]);
    let texture = ctx.load_texture(name, image, TextureOptions::LINEAR);
    view.texture = Some((key, texture));
}

/// The view itself: background, image, zooming and panning, the toolbar
/// (its menu button and Mask Enhancement) and the right-click menu.
fn view_contents(ui: &mut Ui, rect: Rect, id: Id, view: &mut View2D, resolution: usize, menu: &mut ViewMenu, contents: impl Fn(&mut Ui, &mut ViewMenu)) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, BACKGROUND);
    let response = ui.interact(rect, id, Sense::click_and_drag());
    // `ZoomBorder`: the wheel zooms about the pointer, the middle button
    // pans; a double-click fits the map or shows it pixel for pixel.
    if response.hovered() {
        let scroll = wheel_direction(ui);
        if scroll != 0.0 {
            let factor = if scroll > 0.0 { ZOOM_STEP } else { 1.0 / ZOOM_STEP };
            let new_zoom = (view.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
            if let Some(pointer) = ui.input(|input| input.pointer.hover_pos()) {
                let anchor = pointer - rect.center() - view.pan;
                view.pan -= anchor * (new_zoom / view.zoom - 1.0);
            }
            view.zoom = new_zoom;
            view.fixed = false;
        }
    }
    if response.dragged_by(egui::PointerButton::Middle) {
        view.pan += response.drag_delta();
        view.fixed = false;
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    }
    if response.double_clicked() {
        if view.fixed {
            // `Zoom100`: one image pixel to one screen pixel.
            let fitted = rect.width().min(rect.height()).max(1.0);
            let pixels_per_point = ui.ctx().pixels_per_point();
            view.zoom = (resolution as f32 / pixels_per_point / fitted).clamp(MIN_ZOOM, MAX_ZOOM);
            view.pan = Vec2::ZERO;
            view.fixed = false;
        } else {
            view.zoom = 1.0;
            view.pan = Vec2::ZERO;
            view.fixed = true;
        }
    }
    if view.fixed {
        view.zoom = 1.0;
        view.pan = Vec2::ZERO;
    }
    let image = image_rect(rect, view);
    if let Some((_, texture)) = &view.texture {
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        painter.image(texture.id(), image, uv, Color32::WHITE);
    }
    response.context_menu(|ui| contents(ui, menu));

    // `Toolbar` at the top right: the menu button (`tglToolbar`) and Mask
    // Enhancement.
    let button = crate::ui::widgets::toolbar_button(ui);
    let toolbar = Rect::from_min_size(Pos2::new(rect.right() - 4.0 - 40.0, rect.top() + 4.0), Vec2::new(40.0, 2.0 * button + 12.0));
    crate::ui::widgets::toolbar_background(ui, toolbar, theme::MENU_BORDER, 1.5);
    let inner = toolbar.shrink(1.5).shrink2(Vec2::new(2.0, 4.0));
    let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Center)));
    bar.spacing_mut().item_spacing.y = 2.0;
    icon_menu(&mut bar, "menu-deep", "Menu", None, |ui| contents(ui, menu));
    icon_menu(&mut bar, "render-lvl", "Mask Enhancement", None, |ui| {
        menu_caption(ui, "Mask Enhancement");
        let choices = [(MaskEnhancement::None, "None / Raw"), (MaskEnhancement::Autolevel, "Autolevel"), (MaskEnhancement::Equalize, "Equalized")];
        for (enhancement, text) in choices {
            if menu_item(ui, None, None, text, "").clicked() {
                menu.enhancement = Some(enhancement);
            }
        }
    });
}

/// Which way the wheel turned this frame (`MouseWheelEventArgs.Delta`'s
/// sign): 1 away from the user, -1 towards, 0 not at all.
pub fn wheel_direction(ui: &Ui) -> f32 {
    let delta: f32 = ui.input(|input| {
        let wheels = input.events.iter().filter_map(|event| match event {
            egui::Event::MouseWheel { delta, .. } => Some(delta.y),
            _ => None,
        });
        wheels.sum()
    });
    if delta > 0.0 {
        1.0
    } else if delta < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// `MeasureControl`: a darkened layer over the image with two round
/// handles and the line between them (6 wide in the accent over 2 dark),
/// the readout at the bottom left and the metric toggle (`tglMetric`).
fn measure_overlay(ui: &mut Ui, painter: &egui::Painter, rect: Rect, image: Rect, view: &mut View2D, reference: Option<&Map>, editor: &Editor) {
    painter.rect_filled(image.intersect(rect), 0.0, READOUT_BACKING);
    let to_screen = |point: Pos2| image.min + point.to_vec2() * image.width();
    for index in 0..2 {
        let centre = to_screen(view.measure[index]);
        let handle = Rect::from_center_size(centre, Vec2::splat(20.0));
        let response = ui.interact(handle, Id::new(("measure-handle", index)), Sense::drag());
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::AllScroll);
        }
        if response.dragged() {
            let moved = view.measure[index] + response.drag_delta() / image.width();
            // `Thumb_DragDelta`: the handle may hang 10 past the edges.
            let slack = 10.0 / image.width();
            view.measure[index] = Pos2::new(moved.x.clamp(-slack, 1.0 - slack), moved.y.clamp(-slack, 1.0 - slack));
        }
    }
    let first = to_screen(view.measure[0]);
    let second = to_screen(view.measure[1]);
    painter.line_segment([first, second], Stroke::new(6.0, theme::ACCENT_PRIMARY));
    painter.line_segment([first, second], Stroke::new(2.0, theme::WINDOW_DARK));
    for point in [first, second] {
        painter.circle_filled(point, 10.0, theme::ACCENT_PRIMARY);
        painter.circle_stroke(point, 10.0, Stroke::new(1.0, theme::WINDOW_DARK));
    }
    let terrain = editor.document.terrain();
    let text = measure_text(view.measure, view.metric, reference, terrain.width, terrain.height);
    let readout = readout(painter, rect, &text);
    // `tglMetric` (24, margin 10 from the bottom left).
    let toggle = Rect::from_min_size(Pos2::new(readout.left() + 2.0, readout.top() - 4.0 - 24.0), Vec2::splat(24.0));
    let mut toggle_ui = ui.new_child(egui::UiBuilder::new().max_rect(toggle));
    if crate::ui::widgets::icon_toggle_sized(&mut toggle_ui, "ruler-2", "Toggle between Metric and Normalized values", view.metric, theme::ACCENT_PRIMARY, Vec2::splat(24.0)).clicked() {
        view.metric = !view.metric;
    }
}

/// `MeasureControl.UpdateDistance`'s readout.
fn measure_text(points: [Pos2; 2], metric: bool, reference: Option<&Map>, terrain_width: f32, terrain_height: f32) -> String {
    let clamp = |point: Pos2| Pos2::new(point.x.clamp(0.0, 1.0), point.y.clamp(0.0, 1.0));
    let first = clamp(points[0]);
    let mut second = clamp(points[1]);
    if first == second {
        second = Pos2::new(second.x * 1.001, second.y * 1.001);
    }
    let dx = (first.x - second.x) as f64;
    let dy = (first.y - second.y) as f64;
    let flat = (dx * dx + dy * dy).sqrt();
    let (height_first, height_second) = match reference {
        Some(map) => {
            let size = map.width() as f32;
            let at = |point: Pos2| {
                let column = (point.x * size).clamp(0.0, size - 1.0) as usize;
                let row = (point.y * size).clamp(0.0, size - 1.0) as usize;
                map.data()[row * map.width() + column]
            };
            (at(first), at(second))
        }
        None => (0.0, 0.0),
    };
    if metric {
        let first_metres = height_first * terrain_height;
        let second_metres = height_second * terrain_height;
        let rise = (first_metres - second_metres) as f64;
        let across = flat * terrain_width as f64;
        let spatial = (across * across + rise * rise).sqrt();
        return format!(
            "{}m, {}m, {}m\n × \n{}m, {}m, {}m\n\nDistance 2D: {}m\nDistance 3D: {}m\n Difference: {}m\n\n",
            fixed(first.x * terrain_width, 1),
            fixed(first.y * terrain_width, 1),
            fixed(first_metres, 1),
            fixed(second.x * terrain_width, 1),
            fixed(second.y * terrain_width, 1),
            fixed(second_metres, 1),
            grouped_optional(across, 2, true),
            grouped_optional(spatial, 2, true),
            grouped_optional((first_metres - second_metres).abs() as f64, 2, true),
        );
    }
    let rise = (height_first - height_second) as f64;
    let spatial = (dx * dx + dy * dy + rise * rise).sqrt();
    format!(
        "{}, {}, {}\n × \n{}, {}, {}\n\nDistance 2D: {}\nDistance 3D: {}\n Difference: {}\n\n",
        fixed(first.x, 3),
        fixed(first.y, 3),
        fixed(height_first, 3),
        fixed(second.x, 3),
        fixed(second.y, 3),
        fixed(height_second, 3),
        fixed(flat as f32, 3),
        fixed(spatial as f32, 3),
        fixed((height_first - height_second).abs(), 3),
    )
}

/// Picking's readout (`Viewport2D_MouseMove`): the pixel under the pointer
/// and its value.
fn pick_readout(ui: &Ui, painter: &egui::Painter, rect: Rect, image: Rect, reference: &Map, editor: &Editor) {
    let Some(pointer) = ui.input(|input| input.pointer.hover_pos()) else {
        return;
    };
    if !rect.contains(pointer) {
        return;
    }
    let position = (pointer - image.min) / image.size();
    let column = ((position.x * reference.width() as f32) as i64).clamp(0, reference.width() as i64 - 1) as usize;
    let row = ((position.y * reference.height() as f32) as i64).clamp(0, reference.height() as i64 - 1) as usize;
    let index = row * reference.width() + column;
    let text = if reference.channel_count() == 1 {
        let value = reference.data()[index];
        let height = editor.document.terrain().height * value;
        format!("{column} x {row}\n\n{}\n{}m", grouped(value as f64, 6), grouped(height as f64, 2))
    } else {
        let [red, green, blue] = [0, 1, 2].map(|channel| reference.channel(channel.min(reference.channel_count() - 1))[index]);
        let byte = |value: f32| format!("{:03}", (255.0 * value) as u8);
        format!("{column} x {row}\n\n{} {} {}\n\n{} {} {}", grouped(red as f64, 6), grouped(green as f64, 6), grouped(blue as f64, 6), byte(red), byte(green), byte(blue))
    };
    readout(painter, rect, &text);
}

/// A readout box at the view's bottom left (margin 8, padding 12, 8;
/// monospace, white on `#66000000`).
fn readout(painter: &egui::Painter, rect: Rect, text: &str) -> Rect {
    let galley = painter.layout_no_wrap(text.trim_end().to_string(), theme::monospace(12.0), Color32::WHITE);
    let size = galley.size() + Vec2::new(24.0, 16.0);
    let box_rect = Rect::from_min_size(Pos2::new(rect.left() + 8.0, rect.bottom() - 8.0 - size.y), size);
    painter.rect_filled(box_rect, 0.0, READOUT_BACKING);
    painter.galley(box_rect.min + Vec2::new(12.0, 8.0), galley, Color32::WHITE);
    box_rect
}

/// The "Pixel sampled" dialog (`Viewport2D_HeightPicked`): the value as
/// metres, percent and raw (or a colour as floats and bytes), with buttons
/// copying it.
fn picked_dialog(ctx: &egui::Context, state: &mut Viewport2DState, commands: &mut Vec<Command>) {
    let Some(picked) = state.picked.clone() else {
        return;
    };
    let colour = picked.values.len() > 1;
    let value = |index: usize| picked.values.get(index).copied().unwrap_or(0.0);
    let byte = |value: f32| (255.0 * value) as u8;
    let (content, status, buttons) = if colour {
        let [red, green, blue] = [value(0), value(1), value(2)];
        let floats = format!("      32-bit: {} {} {}\n", float_text(red), float_text(green), float_text(blue));
        let bytes = format!("      8-bit: {} {} {}", byte(red), byte(green), byte(blue));
        let status = format!("Color sampled: {} {} {}", byte(red), byte(green), byte(blue));
        (format!("Color samples:\n{floats}{bytes}"), status, ["Copy 32-bit Floats", "Copy 8-bit Bytes"])
    } else {
        let height = value(0);
        let metres = picked.terrain_height * height;
        let metric = format!("      Metric: {}m\n", grouped_optional(metres as f64, 2, false));
        let percentage = format!("      Percentage: {}%\n", grouped((height * 100.0) as f64, 1));
        let raw = format!("      Raw value: {}", float_text(height));
        let status = format!("Height sampled: {} m", grouped_optional(metres as f64, 0, false));
        (format!("Height sampled:\n{metric}{percentage}{raw}"), status, ["Copy in metric", "Copy in percentage"])
    };
    if !state.picked.as_ref().is_some_and(|_| ctx.data(|data| data.get_temp::<bool>(Id::new("picked-reported"))).unwrap_or(false)) {
        commands.push(Command::Report(status));
        ctx.data_mut(|data| data.insert_temp(Id::new("picked-reported"), true));
    }
    let mut close = false;
    let modal = egui::Modal::new(Id::new("pixel-sampled"));
    let response = modal.show(ctx, |ui| {
        ui.set_width(380.0);
        ui.label(egui::RichText::new("Pixel sampled").font(theme::semibold(13.0)).color(theme::LABEL_EMPHASIS));
        ui.add_space(8.0);
        ui.label(egui::RichText::new(content.as_str()).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            for (index, label) in buttons.iter().enumerate() {
                if ui.button(*label).clicked() {
                    let text = copied_text(&picked, colour, index);
                    ui.ctx().copy_text(text);
                    close = true;
                }
            }
            if ui.button("Close").clicked() {
                close = true;
            }
        });
    });
    if close || response.should_close() {
        state.picked = None;
        ctx.data_mut(|data| data.remove::<bool>(Id::new("picked-reported")));
    }
}

/// What each copy button puts on the clipboard.
fn copied_text(picked: &Picked, colour: bool, button: usize) -> String {
    let value = |index: usize| picked.values.get(index).copied().unwrap_or(0.0);
    let byte = |value: f32| (255.0 * value) as u8;
    match (colour, button) {
        (true, 0) => format!("{},{},{}", float_text(value(0)), float_text(value(1)), float_text(value(2))),
        (true, _) => format!("{} {} {}", byte(value(0)), byte(value(1)), byte(value(2))),
        (false, 0) => format!("{}m", grouped_optional((picked.terrain_height * value(0)) as f64, 2, false)),
        (false, _) => grouped((value(0) * 100.0) as f64, 1),
    }
}

/// `float.ToString()`.
fn float_text(value: f32) -> String {
    nerthus_engine::automation::dotnet_float_string(value)
}

/// `"0.0"`, `"0.000"`: fixed decimals, no grouping.
fn fixed(value: f32, decimals: usize) -> String {
    format!("{value:.decimals$}")
}

/// `"N2"`, `"###,##0.00"`: fixed decimals with thousands grouped.
fn grouped(value: f64, decimals: usize) -> String {
    let text = format!("{:.decimals$}", value.abs());
    let (integer, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let mut with_commas = String::new();
    for (index, digit) in integer.chars().enumerate() {
        if index > 0 && (integer.len() - index) % 3 == 0 {
            with_commas.push(',');
        }
        with_commas.push(digit);
    }
    let sign = if value < 0.0 && text.chars().any(|digit| digit.is_ascii_digit() && digit != '0') { "-" } else { "" };
    if fraction.is_empty() { format!("{sign}{with_commas}") } else { format!("{sign}{with_commas}.{fraction}") }
}

/// `"###,###.##"` (or `"###,###,###.00"` with `fixed_decimals`): grouped,
/// up to `decimals` decimals (exactly, when fixed), no leading zero.
fn grouped_optional(value: f64, decimals: usize, fixed_decimals: bool) -> String {
    let text = grouped(value, decimals);
    let (integer, fraction) = text.split_once('.').map(|(integer, fraction)| (integer.to_string(), fraction.to_string())).unwrap_or((text.clone(), String::new()));
    let fraction = if fixed_decimals { fraction } else { fraction.trim_end_matches('0').to_string() };
    let integer = if integer == "0" || integer == "-0" { integer.trim_end_matches('0').to_string() } else { integer };
    if fraction.is_empty() { integer } else { format!("{integer}.{fraction}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_formats_follow_dotnet() {
        assert_eq!(grouped(1234.56789, 2), "1,234.57");
        assert_eq!(grouped(0.1234567, 6), "0.123457");
        assert_eq!(grouped_optional(0.5, 2, false), ".5");
        assert_eq!(grouped_optional(1500.0, 2, false), "1,500");
        assert_eq!(grouped_optional(0.0, 0, false), "");
        assert_eq!(grouped_optional(12.3, 2, true), "12.30");
    }

    #[test]
    fn measures_read_normalised_or_metres() {
        let points = [Pos2::new(0.0, 0.0), Pos2::new(1.0, 0.0)];
        let text = measure_text(points, false, None, 5000.0, 2500.0);
        assert!(text.starts_with("0.000, 0.000, 0.000\n × \n1.000, 0.000, 0.000\n\nDistance 2D: 1.000"));
        let metres = measure_text(points, true, None, 5000.0, 2500.0);
        assert!(metres.contains("Distance 2D: 5,000.00m"));
    }

    #[test]
    fn ctrl_m_opens_the_selected_node_else_shows_or_hides_the_map_viewer() {
        let mut document = crate::document::Document::new();
        let node = document.add_node("Mountain", [26000.0, 26000.0]).unwrap();
        let mut editor = Editor::new(document);
        let mut state = Viewport2DState::default();
        editor.select(None);
        state.show_2d_view(&editor);
        assert!(state.following.is_some());
        state.show_2d_view(&editor);
        assert!(state.following.is_none());

        editor.select(Some(node));
        state.show_2d_view(&editor);
        assert_eq!(state.floating.iter().map(|view| view.node).collect::<Vec<_>>(), [node]);
        assert!(state.following.is_none());
        // One view a node.
        state.show_2d_view(&editor);
        assert_eq!(state.floating.len(), 1);
    }

    #[test]
    fn the_map_viewer_renders_a_pick_until_another_node_is_previewed() {
        let mut document = crate::document::Document::new();
        let erosion = document.add_node("ErosionPro", [26000.0, 26000.0]).unwrap();
        let other = document.add_node("Mountain", [26300.0, 26000.0]).unwrap();
        let mut editor = Editor::new(document);
        editor.select(Some(erosion));
        let resolution = nerthus_engine::Resolution::square(4);
        let heights = Arc::new(Map::filled(resolution, 0.5));
        let deposits = Arc::new(Map::filled(resolution, 0.25));
        editor.outputs = vec![("Out".to_string(), heights), ("Deposits".to_string(), deposits)];

        let mut following = FollowingView::new();
        following.picked = Some((erosion, "Deposits".to_string()));
        refresh_following(&editor, &mut following);
        assert_eq!(following.generation, 1);
        // Nothing changed: no render.
        refresh_following(&editor, &mut following);
        assert_eq!(following.generation, 1);

        editor.select(Some(other));
        refresh_following(&editor, &mut following);
        assert!(following.picked.is_none());
        assert!(following.sent.is_none());
    }
}
