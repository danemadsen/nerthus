# Common Issues

## Nerthus fails or freezes on starting up.

Most problems at startup come from the graphics card's driver, which Nerthus uses both to draw the 3D view and to compute.

* Start Nerthus in [CPU only mode](diagnostics/cpu-only-mode.md): from a terminal, `nerthus --cpu-only`. If that works, the GPU devices are the cause.
* Update your graphics driver, then start Nerthus normally again. See [Driver Issues](../../ui/interface/options/compute.md#driver-issues).
* Open [Diagnostics](diagnostics/index.md) with `nerthus --diagnostics`. From there you can start Nerthus with CPU only or with Verbose Logging, reset the settings to their defaults, and create a diagnostics package.

When Nerthus crashes while starting, it asks whether to diagnose the issue. **Yes** opens this page in your web browser.

### Audio and Overlay Utilities

On Windows, utilities that inject themselves into other programs, such as the "Nahimic" audio tweaker bundled with some ASUS and MSI computers, or screen overlays, can interfere with the 3D view. Exclude Nerthus from them, or disable or uninstall them.

***

## Nerthus crashed.

When something goes wrong, Nerthus asks whether to try and continue.

* **Yes** keeps the session going. Save your work, preferably as a new file, and restart Nerthus soon after.
* **No** closes Nerthus and starts it again with [Diagnostics](diagnostics/index.md) open.

Either way, your work is not lost: the project as it was is saved in the data folder's `Autosaves` folder as `DISASTER_RECOVERY_<name>_<time>.terrain`. File › Open Autosave lists the recent copies. The data folder's `Logs` folder holds what happened, in `CRASH_<time>.txt` and `CRASH_Error_<time>.txt`.

***

## Some nodes build slowly, or differently on the GPU.

Nodes that run as compute kernels use the GPUs turned on in [Options › Compute](../../ui/interface/options/compute.md). Results on a GPU can differ very slightly from the CPU's. To compare, or if a device misbehaves, turn it off there, or use CPU only mode.

***

## Checking for updates fails.

If your network needs a proxy, turn on **Use Web Proxy** in [Options › General](../../ui/interface/options/general.md) and name the proxy there (or leave it empty to use the system's).
