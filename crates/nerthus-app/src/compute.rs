//! The compute devices in the editor (`App.SetComputeResources` and the
//! status bar's device button). In CPU-only mode only the CPU computes;
//! otherwise the devices the preferences name load as Nerthus starts, or,
//! when they name none, every device but integrated GPUs, which the
//! preferences then keep.

use crossbeam_channel::{Receiver, TryRecvError};

use crate::user::{self, User};

/// What the editor knows about its compute devices.
#[derive(Default)]
pub struct ComputeState {
    /// CPU-only mode (`--cpu-only`, or `cpuonly.dat` in the data folder).
    pub cpu_only: bool,
    /// The status bar's warning (`Shell.IsFastestDeviceActive`): the
    /// benchmark's fastest device, when it is not active.
    pub fastest_disabled: Option<String>,
    /// Devices loading on a worker thread; it sends whether the
    /// preferences named none.
    loading: Option<Receiver<bool>>,
}

impl ComputeState {
    /// `App.SetComputeResources`: CPU-only mode when asked or marked;
    /// otherwise the preferred devices (or all suitable ones) load in the
    /// background (`SyclLoader`). Safe Mode loads all suitable devices and
    /// leaves the preferences' list alone.
    pub fn start(cpu_only_asked: bool, safe_mode: bool, user: &User) -> ComputeState {
        let marked = user::data_folder().is_some_and(|folder| folder.join(nerthus_compute::CPU_ONLY_FILE).exists());
        // The web build computes on the CPU alone (`nerthus_compute` finds
        // no GPUs in a browser).
        let cpu_only = cpu_only_asked || marked || cfg!(target_arch = "wasm32");
        nerthus_compute::set_cpu_only(cpu_only);
        if cpu_only {
            log::info!("CPU Only mode");
            return ComputeState { cpu_only, ..ComputeState::default() };
        }

        let backends = user.preferences.compute_backends();
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let spawned = std::thread::Builder::new().name("compute devices".to_string()).spawn(move || {
            let named_none = backends.is_empty();
            if safe_mode {
                log::info!("SAFE MODE: Loading all suitable devices.");
                nerthus_compute::load_all_backends();
            } else if named_none {
                log::info!("Loading all suitable devices.");
                nerthus_compute::load_all_backends();
            } else {
                log::info!("Loading user preferred devices.");
                nerthus_compute::load_backends(&backends);
            }
            for device in nerthus_compute::active_devices() {
                log::debug!("  {} ({})", device.device_name, device.backend_name);
            }
            // The preferences keep the devices loaded when they named none
            // (not in Safe Mode, whose preferences are the defaults).
            let _ = sender.send(named_none && !safe_mode);
        });
        if let Err(error) = spawned {
            log::warn!("cannot load compute devices: {error}");
        }
        ComputeState { cpu_only, fastest_disabled: None, loading: Some(receiver) }
    }

    /// Once the devices are in: the preferences keep them if they named
    /// none, and the status bar checks the fastest device.
    pub fn poll(&mut self, user: &mut User) {
        let Some(receiver) = &self.loading else {
            return;
        };
        let named_none = match receiver.try_recv() {
            Ok(named_none) => named_none,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => false,
        };
        self.loading = None;
        if named_none {
            let active: Vec<String> = nerthus_compute::active_devices().into_iter().map(|device| device.id).collect();
            user.preferences.set_compute_backends(&active);
        }
        self.check_fastest();
    }

    /// `Shell.IsFastestDeviceActive`: with benchmark scores saved, warns
    /// when the best-scoring device is not among the active ones.
    pub fn check_fastest(&mut self) {
        self.fastest_disabled = None;
        if self.cpu_only {
            return;
        }
        let Some(folder) = user::data_folder() else {
            return;
        };
        let scores = nerthus_compute::read_scores(&folder);
        let Some(best) = nerthus_compute::best_device(&scores) else {
            return;
        };
        let active = nerthus_compute::active_devices();
        if active.iter().any(|device| device.id == best) {
            return;
        }
        // The id's second-to-last part is the device's name.
        let parts: Vec<&str> = best.split('/').collect();
        let name = if parts.len() >= 2 { parts[parts.len() - 2] } else { best };
        self.fastest_disabled = Some(format!("[{name}] is disabled!"));
    }
}
