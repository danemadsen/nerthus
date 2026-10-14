//! Where the editor window opens (`Shell_Loaded`): on the monitor Options ›
//! General › Start Nerthus on chooses, maximised there
//! (`PositionWindowOnMonitor`, then `WindowState.Maximized`). The window
//! is made before Bevy knows the monitors, so it moves once they are in,
//! on the first frame.

use bevy::prelude::*;
use bevy::window::{Monitor, MonitorSelection, PrimaryWindow, WindowPosition};

use crate::editor::Editor;
use crate::ui::UiState;

pub struct StartMonitorPlugin {
    /// `--monitor`: the display to open on instead of the option's.
    pub monitor: Option<usize>,
    /// Maximised there; otherwise only centred on it.
    pub maximised: bool,
}

impl Plugin for StartMonitorPlugin {
    fn build(&self, app: &mut App) {
        let placement = StartPlacement { monitor: self.monitor, maximised: self.maximised };
        app.insert_resource(placement);
        app.add_systems(Update, place_window.run_if(resource_exists::<StartPlacement>));
    }
}

/// The placement still to make.
#[derive(Resource)]
struct StartPlacement {
    monitor: Option<usize>,
    maximised: bool,
}

/// Moves the window onto its monitor, maximised, once Bevy lists the
/// monitors.
fn place_window(
    mut commands: Commands,
    placement: Res<StartPlacement>,
    editor: Res<Editor>,
    monitors: Query<(), With<Monitor>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut ui_state: ResMut<UiState>,
) {
    let count = monitors.iter().count();
    if count == 0 {
        return;
    }
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    let chosen = match placement.monitor {
        Some(index) => index as i64,
        None => editor.user.options.monitor(),
    };
    window.position = WindowPosition::Centered(start_monitor(chosen, count));
    if placement.maximised {
        window.set_maximized(true);
        ui_state.maximized = true;
    }
    commands.remove_resource::<StartPlacement>();
}

/// `PositionWindowOnMonitor`'s choice among the monitors (`Screen.AllScreens`,
/// in the order the system lists them): -1 is the primary one, and the
/// first stands in for one that is not there.
fn start_monitor(chosen: i64, count: usize) -> MonitorSelection {
    if chosen < 0 {
        return MonitorSelection::Primary;
    }
    let index = chosen as usize;
    if index >= count {
        return MonitorSelection::Index(0);
    }
    MonitorSelection::Index(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_option_picks_a_monitor_or_the_primary_one() {
        assert_eq!(start_monitor(-1, 2), MonitorSelection::Primary);
        assert_eq!(start_monitor(1, 2), MonitorSelection::Index(1));
        assert_eq!(start_monitor(2, 2), MonitorSelection::Index(0));
    }
}
