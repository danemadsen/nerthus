# Compute

The **Compute** tab allows you to configure which available hardware devices are used for computation, optimizing performance across CPU and GPU resources.

## **Available Devices**

This section lists all detected compute-capable devices, grouped by device, with a **performance score** for each way of reaching it, indicating its relative speed in Nerthus’s benchmarks (higher is better).

## Device List

Lists all available compute devices that Nerthus can use for processing terrains:

* **Native** – Nerthus's own CPU code. It is always on, and every node can run on it.
* **GPUs** – each graphics card, through the graphics interfaces the system offers: **Metal** on macOS, **Vulkan** and **DirectX 12** on Windows, and **Vulkan** on Linux. Turn on the ones Nerthus may use. Nodes that run as compute kernels (such as Median, Thermal2, ThermalShaper, Flow, Cellular3D, and the erosion and lighting kernels inside other nodes) use the GPUs turned on, a dedicated GPU before an integrated one; the rest of each build stays on the CPU.

**Scan Hardware and Determine Speeds**: Initiates a scan of all hardware to evaluate processing capabilities. Once done, a score for each device and interface is shown in the list.

See [Benchmark Devices](../../../using/troubleshooting/diagnostics/benchmark-devices.md) for additional information.

## Driver Issues

If Nerthus detects driver issues with any devices, a **Requires driver update** button is shown next to that device, and the device cannot be turned on until its driver has been updated. The button opens this page.

To update a driver, get the newest one from the graphics card maker's website (NVIDIA, AMD or Intel) and install it, or on macOS install the system's updates, which carry Apple's drivers. Then start Nerthus again: the devices are looked at as it starts.

## CPU-only mode

Nerthus can be forced to run using CPU only. This may not necessarily bring performance down too much as Nerthus uses advanced techniques to keep performance up while using the CPU.

If you are experiencing instabilities, disabling GPU and Accelerators may improve stability.

See [CPU Only Mode](../../../using/troubleshooting/diagnostics/cpu-only-mode.md)

* **Toggle Switches** – Enable or disable each device and interface.
* **Score** – Benchmark performance metric; helps you select the fastest option.
* **Scan Hardware and Determine Speeds** – Detects devices and benchmarks their compute speeds.
* **Enforce Optimal Settings** – On Windows with an NVIDIA graphics card: creates an NVIDIA driver profile for Nerthus that runs it on the NVIDIA GPU of a laptop with two GPUs, keeps CUDA programs from falling back on system memory, and limits Nerthus to 15 frames a second while it is in the background. Restart Nerthus for the profile to take effect. The first-run setup creates the same profile.
* **Use CPU-only Mode** – Restart Nerthus to compute on the CPU only. In CPU-only mode, the page offers to restart Nerthus with all devices enabled instead.

## **Usage Tips**

* Prefer a dedicated GPU over an integrated one for maximum performance.
* Use **Native** CPU mode if the GPU is unavailable or reserved for other tasks.
* Disable low-scoring devices to prevent work being scheduled on slow hardware.
* Re-scan after driver updates or hardware changes.
