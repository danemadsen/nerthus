# Viewport

The **Viewport** tab allows you to configure the visual display settings for Nerthus’s 3D viewport. These options affect how terrains and masks are rendered during editing and previewing.

## **Antialiasing**

* Options: **None, 2x, 4x, 8x**
* Sets how smoothly the 3D viewport redraws while you work in it: **None** draws 20 frames a second, **2x** 30, and **4x** and **8x** 60.
* Higher values look smoother but use more of the GPU.
* **Default:** none chosen, which draws 30 frames a second.

## **Display Options**

* **Do not clear viewport when unselecting a node** _(Toggle)_
  * When enabled, the last viewed terrain remains visible even when no node is selected.
* **Show Grid** _(Toggle)_
  * Displays a reference grid in the viewport for spatial orientation.
* **Show Compass** _(Toggle)_
  * Adds a small compass widget indicating cardinal directions.
* **Show Base Marker** _(Toggle)_
  * Shows a marker for the origin/base of the terrain.
* **Conserve memory when displaying 4K terrains** _(Toggle)_
  * Optimizes memory usage for high-resolution terrains (4K+).
  * Can reduce visual fidelity slightly to save RAM and GPU memory.

## **Background**

* **Color Picker**
  * Sets the background color of the viewport.
  * Click the color box to select a custom color or press **X** to reset to default.

## **Mask Gradient**

* Defines the color scheme used when displaying mask outputs in the viewport.
* Options:
  * **Greyscale** – Black-to-white gradient. _(Default)_
  * **Hot** – Black through blue, purple, red and yellow to white.
  * **Red** – Red intensity map.
  * **Blue** – Blue intensity map.
  * **Spectrum** – Multi-color gradient for better visual separation of mask values.

## **Gizmos**

* Where node gizmos are shown:
  * **2D Viewport** – In the 2D viewport only. _(Default)_
  * **3D + 2D Viewport** – In the 3D viewport as well.

## **Notes**

* On a slower GPU, **None** or **2x** keeps the rest of Nerthus responsive.
* Enabling grid and compass can help with orientation but may add slight visual clutter.
* Conserving memory is recommended for projects with multiple high-resolution terrains or limited VRAM.
