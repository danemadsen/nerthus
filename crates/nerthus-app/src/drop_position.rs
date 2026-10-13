//! Where files dragged over the window are. The windowing library says
//! that files are dragged in and dropped, but not where, and the pointer
//! reports no movement during a drag; so while files are over the window,
//! and as they drop, the pointer's place is read from the system and passed
//! on as a cursor movement. The property panel's file fields and the graph
//! then see where the files are: a field lights up under them and takes
//! them, the graph adds its nodes where they land.

use bevy::ecs::system::NonSendMarker;
use bevy::prelude::*;
use bevy::window::{CursorMoved, FileDragAndDrop, RawHandleWrapper, WindowEvent};
use bevy_egui::EguiPreUpdateSet;

pub struct DropPositionPlugin;

impl Plugin for DropPositionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DraggedFiles>();
        // Before the interface reads this frame's pointer and drops.
        app.add_systems(PreUpdate, follow_dragged_files.before(EguiPreUpdateSet::ProcessInput));
    }
}

/// The window files are being dragged over, if any.
#[derive(Resource, Default)]
struct DraggedFiles {
    over: Option<Entity>,
}

/// Each frame files are over a window, and the frame they drop, the
/// pointer goes where the system says it is.
fn follow_dragged_files(_main_thread: NonSendMarker, mut drags: MessageReader<FileDragAndDrop>, mut dragged: ResMut<DraggedFiles>, windows: Query<(&Window, &RawHandleWrapper)>, mut events: MessageWriter<WindowEvent>) {
    let mut dropped_on = None;
    for drag in drags.read() {
        match drag {
            FileDragAndDrop::HoveredFile { window, .. } => dragged.over = Some(*window),
            FileDragAndDrop::DroppedFile { window, .. } => {
                dropped_on = Some(*window);
                dragged.over = None;
            }
            FileDragAndDrop::HoveredFileCanceled { .. } => dragged.over = None,
        }
    }
    let Some(entity) = dropped_on.or(dragged.over) else {
        return;
    };
    let Ok((window, handles)) = windows.get(entity) else {
        return;
    };
    let Some(position) = system_pointer::position(handles, window.scale_factor()) else {
        return;
    };
    let moved = CursorMoved { window: entity, position, delta: None };
    events.write(WindowEvent::CursorMoved(moved));
}

/// The pointer's place in a window as the system has it, in logical
/// pixels from the window's top left; none where the system can't say
/// (Wayland gives programs no pointer position of their own asking).
mod system_pointer {
    use bevy::math::Vec2;
    use bevy::window::RawHandleWrapper;

    /// AppKit: the mouse's place on the screen, into the window, then the
    /// view (which counts up from its bottom unless flipped); points are
    /// logical pixels already.
    #[cfg(target_os = "macos")]
    pub fn position(handles: &RawHandleWrapper, _scale_factor: f32) -> Option<Vec2> {
        use objc2_app_kit::{NSEvent, NSView};
        use raw_window_handle::RawWindowHandle;

        let RawWindowHandle::AppKit(handle) = handles.get_window_handle() else {
            return None;
        };
        // SAFETY: the handle is the window's content view, which lives as
        // long as the window's entity; this runs on the main thread.
        let view: &NSView = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
        let window = view.window()?;
        let on_screen = NSEvent::mouseLocation();
        let in_window = window.convertPointFromScreen(on_screen);
        let in_view = view.convertPoint_fromView(in_window, None);
        let y = if view.isFlipped() { in_view.y } else { view.bounds().size.height - in_view.y };
        Some(Vec2::new(in_view.x as f32, y as f32))
    }

    /// Win32: the cursor's place on the screen into the window's client
    /// area, in physical pixels.
    #[cfg(windows)]
    pub fn position(handles: &RawHandleWrapper, scale_factor: f32) -> Option<Vec2> {
        use raw_window_handle::RawWindowHandle;
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

        let RawWindowHandle::Win32(handle) = handles.get_window_handle() else {
            return None;
        };
        let mut point = POINT { x: 0, y: 0 };
        // SAFETY: both take a point to fill; the window handle is live.
        let found = unsafe { GetCursorPos(&mut point) != 0 && ScreenToClient(handle.hwnd.get() as _, &mut point) != 0 };
        found.then(|| Vec2::new(point.x as f32, point.y as f32) / scale_factor)
    }

    /// X11: `XQueryPointer` on the window, in physical pixels (Xlib loaded
    /// once, on the main thread).
    #[cfg(target_os = "linux")]
    pub fn position(handles: &RawHandleWrapper, scale_factor: f32) -> Option<Vec2> {
        use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
        use x11_dl::xlib::Xlib;

        thread_local! {
            static XLIB: Option<Xlib> = Xlib::open().ok();
        }
        let (RawWindowHandle::Xlib(window), RawDisplayHandle::Xlib(display)) = (handles.get_window_handle(), handles.get_display_handle()) else {
            return None;
        };
        let display = display.display?.as_ptr().cast();
        XLIB.with(|xlib| {
            let xlib = xlib.as_ref()?;
            let (mut root, mut child) = (0, 0);
            let (mut root_x, mut root_y, mut x, mut y) = (0, 0, 0, 0);
            let mut buttons = 0;
            // SAFETY: the display and window are the window's own; the rest
            // are places to fill.
            let found = unsafe { (xlib.XQueryPointer)(display, window.window, &mut root, &mut child, &mut root_x, &mut root_y, &mut x, &mut y, &mut buttons) };
            (found != 0).then(|| Vec2::new(x as f32, y as f32) / scale_factor)
        })
    }

    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    pub fn position(_handles: &RawHandleWrapper, _scale_factor: f32) -> Option<Vec2> {
        None
    }
}
