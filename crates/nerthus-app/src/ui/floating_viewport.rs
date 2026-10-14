//! Viewport Options › Floating (`Shell.FloatViewport`): the viewport area
//! (its toolbar, the 3D view and the docked 2D view) in a window of its
//! own, "Nerthus - <file>", at the leftmost screen's top left and
//! maximized when there is more than one screen, while the graph takes
//! the main window's whole left column. Embedded puts it back
//! (`Shell.EmbedViewport`). The window does not close on its own
//! (`ViewportWindow_Closing`), and the choice is kept in the options
//! (`ViewportConfig`), so Nerthus starts the way it was left.

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{CameraOutputMode, RenderTarget};
use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;
use bevy::render::render_resource::BlendState;
use bevy::window::{Monitor, WindowRef};
use bevy_egui::{egui, EguiContext, EguiSchedule, PrimaryEguiContext};
use egui::{LayerId, Ui, UiBuilder};
use nerthus_render::{NavigationSettings, StrategyCamera, ViewportCamera, ViewportInput, ViewportSettings};

use crate::editor::Editor;
use crate::theme;
use crate::ui::{fit_camera, surround, viewport_area, UiState};

/// The floating window's egui pass.
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct FloatingViewportPass;

/// The floating window, and the camera its interface is drawn with.
#[derive(Component)]
pub struct FloatingViewportWindow;

#[derive(Component)]
pub struct FloatingViewportUi;

/// The window's size before it is maximized.
const WINDOW_SIZE: (u32, u32) = (1280, 800);
/// `ViewportArea`'s margin inside the window (`SquareBorders`).
const AREA_MARGIN: f32 = 2.0;

/// The window's title (`"Nerthus - " + file name`).
fn title(editor: &Editor) -> String {
    format!("Nerthus - {}", editor.document.file_name())
}

/// Opens the window when the options say Floating and closes it when they
/// say Embedded, moving the 3D view between the windows; keeps its title.
#[allow(clippy::too_many_arguments)]
pub fn manage(
    mut commands: Commands,
    editor: Res<Editor>,
    mut ui_state: ResMut<UiState>,
    mut windows: Query<(Entity, &mut Window), With<FloatingViewportWindow>>,
    interfaces: Query<Entity, With<FloatingViewportUi>>,
    mut main_interface: Single<&mut Camera, With<PrimaryEguiContext>>,
    viewport_camera: Single<Entity, With<ViewportCamera>>,
    monitors: Query<&Monitor>,
) {
    let floating = editor.user.options.viewport_floating();
    let open = windows.iter().next().is_some();
    if floating == open {
        if let Some((_, mut window)) = windows.iter_mut().next() {
            let wanted = title(&editor);
            if window.title != wanted {
                window.title = wanted;
            }
        }
        return;
    }
    // The docked 2D view's pictures belong to the context it was drawn in.
    ui_state.viewport_2d.forget_pictures();
    // Embedded, the 3D view clears the main window before its interface
    // is drawn over it; floating, nothing else draws there.
    let main_clear = if floating { ClearColorConfig::Default } else { ClearColorConfig::None };
    main_interface.output_mode = CameraOutputMode::Write {
        blend_state: Some(BlendState::ALPHA_BLENDING),
        clear_color: main_clear,
    };
    if !floating {
        for (window, _) in &windows {
            commands.entity(window).despawn();
        }
        for interface in &interfaces {
            commands.entity(interface).despawn();
        }
        commands.entity(*viewport_camera).insert(RenderTarget::Window(WindowRef::Primary));
        return;
    }

    let leftmost = monitors.iter().min_by_key(|monitor| monitor.physical_position.x);
    let corner = leftmost.map(|monitor| monitor.physical_position).unwrap_or(IVec2::ZERO);
    let mut window = Window {
        title: title(&editor),
        resolution: WINDOW_SIZE.into(),
        position: WindowPosition::At(corner),
        ..default()
    };
    if monitors.iter().count() > 1 {
        window.set_maximized(true);
    }
    let window_entity = commands.spawn((window, FloatingViewportWindow)).id();
    let output_mode = CameraOutputMode::Write {
        blend_state: Some(BlendState::ALPHA_BLENDING),
        clear_color: ClearColorConfig::None,
    };
    let camera = Camera {
        order: 2,
        output_mode,
        clear_color: ClearColorConfig::Custom(Color::NONE),
        ..default()
    };
    let target = RenderTarget::Window(WindowRef::Entity(window_entity));
    commands.spawn((FloatingViewportUi, Camera2d, RenderLayers::none(), camera, target.clone(), EguiSchedule::new(FloatingViewportPass)));
    commands.entity(*viewport_camera).insert(target);
}

/// Whether the floating window's interface has been styled yet.
#[derive(Default)]
pub struct Styled(bool);

/// Draws the viewport area over the whole floating window and fits the
/// 3D view into it; what it asks for runs with the main window's
/// commands next frame.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    mut context: Single<&mut EguiContext, With<FloatingViewportUi>>,
    editor: Res<Editor>,
    mut ui_state: ResMut<UiState>,
    mut settings: ResMut<ViewportSettings>,
    mut input: ResMut<ViewportInput>,
    navigation: Res<NavigationSettings>,
    camera: Single<(&mut Camera, &StrategyCamera), With<ViewportCamera>>,
    window: Single<&Window, With<FloatingViewportWindow>>,
    mut styled: Local<Styled>,
) {
    let ctx = context.get_mut().clone();
    if !styled.0 {
        theme::apply(&ctx);
        ctx.options_mut(|options| options.zoom_with_keyboard = false);
        egui_extras::install_image_loaders(&ctx);
        styled.0 = true;
        ctx.request_repaint();
        return;
    }
    ctx.set_zoom_factor(editor.user.options.ui_scale());
    crate::theme::set_settings(&ctx, editor.user.options.interface_settings());
    crate::ui::formulas::publish(&ctx, &editor);
    let full = ctx.content_rect();
    let builder = UiBuilder::new().layer_id(LayerId::background()).max_rect(full);
    let mut root = Ui::new(ctx.clone(), "nerthus-floating-viewport".into(), builder);
    let background = root.painter().add(egui::Shape::Noop);

    let (mut camera, strategy) = camera.into_inner();
    let hint = viewport_area::navigation_hint_text(&navigation);
    let now = ctx.input(|state| state.time);
    let hint_shows = ui_state.navigation_hint_until.is_some_and(|until| now < until);
    let overlays = viewport_area::ViewOverlays {
        north_angle: strategy.north_angle(),
        navigation_hint: hint_shows.then_some(hint.as_str()),
    };
    let mut pending = Vec::new();
    let area = full.shrink(AREA_MARGIN);
    let interface = ui_state.as_mut();
    let host = viewport_area::show(&mut root, area, &editor, &mut settings, &mut interface.sun_open, &mut interface.viewport_2d, &overlays, &mut pending);
    root.painter().set(background, surround(full, host));
    interface.floating_commands.extend(pending);

    let physical_size = UVec2::new(window.physical_width(), window.physical_height());
    fit_camera(&mut camera, ctx.pixels_per_point(), physical_size, host);
    // The 3D view takes the pointer and keys here as it does embedded.
    let pointer = ctx.input(|state| state.pointer.hover_pos());
    let over_view = pointer.is_some_and(|pointer| host.contains(pointer));
    let layer = pointer.and_then(|pointer| ctx.layer_id_at(pointer));
    let over_window = layer.is_some_and(|layer| layer.order != egui::Order::Background);
    input.enabled = over_view && !over_window && !ctx.egui_is_using_pointer() && !ctx.any_popup_open();
    if ctx.input(|state| state.pointer.any_pressed()) {
        interface.viewport_focused = over_view && !over_window;
    }
    let modifiers = ctx.input(|state| state.modifiers);
    input.keys = interface.viewport_focused && !ctx.egui_wants_keyboard_input() && !modifiers.command && !modifiers.ctrl;
}
