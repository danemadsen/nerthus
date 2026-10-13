//! The app icon: assets/nerthus.svg drawn for the Dock on macOS and for the
//! window and taskbar on Windows and X11 (Wayland compositors take icons from
//! .desktop files instead). build.rs puts the same logo in the Windows
//! executable for Explorer.

use bevy::prelude::*;
use resvg::{tiny_skia, usvg};

const LOGO_SVG: &[u8] = include_bytes!("../assets/nerthus.svg");

pub struct AppIconPlugin;

impl Plugin for AppIconPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(target_os = "macos")]
        app.add_systems(Startup, dock::set_dock_icon);
        #[cfg(not(target_os = "macos"))]
        app.add_systems(Update, window_icons::set_window_icons);
    }
}

/// The logo drawn into a `size`² square, `inset` (a fraction of `size`) in
/// from each side.
fn render_logo(size: u32, inset: f32) -> Option<tiny_skia::Pixmap> {
    let options = usvg::Options::default();
    let tree = match usvg::Tree::from_data(LOGO_SVG, &options) {
        Ok(tree) => tree,
        Err(error) => {
            warn!("Cannot read the app icon: {error}");
            return None;
        }
    };
    let mut pixmap = tiny_skia::Pixmap::new(size, size)?;

    // The logo's longer side fills the room inside the inset, centred.
    let logo_size = tree.size();
    let room = size as f32 * (1.0 - 2.0 * inset);
    let scale = room / logo_size.width().max(logo_size.height());
    let left = (size as f32 - logo_size.width() * scale) / 2.0;
    let top = (size as f32 - logo_size.height() * scale) / 2.0;
    let transform = tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, left, top);

    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Some(pixmap)
}

/// A bare executable has no bundle icon, so the Dock shows the generic
/// executable icon until the application gives it an image.
#[cfg(target_os = "macos")]
mod dock {
    use bevy::ecs::system::NonSendMarker;
    use bevy::log::warn;
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    /// The Dock's largest: 256 points magnified, at 2x.
    const DOCK_ICON_SIZE: u32 = 512;
    /// Apple's icon grid keeps the artwork to 824 of 1024 pixels, so the logo
    /// matches the size of the other apps in the Dock.
    const DOCK_ICON_INSET: f32 = 100.0 / 1024.0;

    pub fn set_dock_icon(_main_thread: NonSendMarker) {
        let Some(main_thread) = MainThreadMarker::new() else { return };
        let Some(pixmap) = super::render_logo(DOCK_ICON_SIZE, DOCK_ICON_INSET) else { return };
        let png = match pixmap.encode_png() {
            Ok(png) => png,
            Err(error) => {
                warn!("Cannot encode the Dock icon: {error}");
                return;
            }
        };

        let data = NSData::with_bytes(&png);
        let allocation = NSImage::alloc();
        let Some(image) = NSImage::initWithData(allocation, &data) else {
            warn!("Cannot load the Dock icon");
            return;
        };

        let application = NSApplication::sharedApplication(main_thread);
        // SAFETY: on the main thread, with an image (the setter is unsafe
        // because it may not accept nil).
        unsafe { application.setApplicationIconImage(Some(&image)) };
    }
}

#[cfg(not(target_os = "macos"))]
mod window_icons {
    use bevy::ecs::system::NonSendMarker;
    use bevy::prelude::*;
    use bevy::window::{WindowBackendScaleFactorChanged, WindowCreated};
    use bevy::winit::WINIT_WINDOWS;
    use winit::window::Icon;

    /// Gives each new window its icons, and draws them again when a window
    /// moves to a display with another scale.
    pub fn set_window_icons(
        _main_thread: NonSendMarker,
        mut created: MessageReader<WindowCreated>,
        mut rescaled: MessageReader<WindowBackendScaleFactorChanged>,
        windows: Query<&Window>,
    ) {
        let mut changed: Vec<Entity> = created.read().map(|message| message.window).collect();
        let rescaled_windows = rescaled.read().map(|message| message.window);
        changed.extend(rescaled_windows);

        for entity in changed {
            let Ok(window) = windows.get(entity) else { continue };
            let scale = window.resolution.base_scale_factor();
            WINIT_WINDOWS.with_borrow(|winit_windows| {
                if let Some(winit_window) = winit_windows.get_window(entity) {
                    apply_icons(winit_window, scale);
                }
            });
        }
    }

    /// Windows shows the small icon in title bars and the big one in the
    /// taskbar and Alt Tab: 16 and 24 points (winit's `set_window_icon` and
    /// `set_taskbar_icon`), drawn at the display's scale.
    #[cfg(target_os = "windows")]
    fn apply_icons(window: &winit::window::Window, scale: f32) {
        use winit::platform::windows::WindowExtWindows;

        let small_size = (16.0 * scale).round() as u32;
        let big_size = (24.0 * scale).round() as u32;
        window.set_window_icon(icon(small_size));
        window.set_taskbar_icon(icon(big_size));
    }

    /// X11 keeps one image per window, which window managers scale for
    /// title bars, taskbars and docks.
    #[cfg(not(target_os = "windows"))]
    fn apply_icons(window: &winit::window::Window, _scale: f32) {
        window.set_window_icon(icon(128));
    }

    /// The logo as straight-alpha RGBA, which winit wants.
    fn icon(size: u32) -> Option<Icon> {
        let pixmap = super::render_logo(size, 0.0)?;
        let mut rgba = Vec::with_capacity(pixmap.data().len());
        for pixel in pixmap.pixels() {
            let color = pixel.demultiply();
            rgba.extend_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
        }
        Icon::from_rgba(rgba, size, size).ok()
    }
}
