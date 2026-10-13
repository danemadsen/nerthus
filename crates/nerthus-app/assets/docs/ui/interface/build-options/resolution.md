# Resolution

The **Resolution** tab controls how much of the terrain is built and how the output is divided. Use it to select the build scope, target resolution, subdivision strategy, and whether the result is exported as a single image or as tiles.

## Resolution Settings

* **Region**: Select the build scope. You can build the whole terrain or choose a specific region from the dropdown. With a region chosen, **Resolution** sets the region's own resolution, and there is no subdivision or output to choose.
* **Resolution**: Select the output resolution of the terrain, such as `2K` or `4K`. Higher resolutions provide more detail but require more memory.
* **Hybrid Subdivision**: Choose the build speed and quality balance. Some choices are not available at lower resolutions.
  * **None**: No subdivision. Best for faster builds when your world fits within the available resolution limits.
  * **Faster**: Faster build speed with moderate quality.
  * **Balanced**: A balance between speed and quality.
  * **Slower**: Highest quality but slower build speed.
* **Output**: Choose the output format.
  * **Single Image**: Exports as a single image file.
  * **Tiled Images**: Divides the output into tiles.
* **Tile Size**: Set the size of each tile when tiled output is enabled, for example `1024 x 1024`.
* **Blending**: Determines the blending percentage between adjacent tiles for smoother transitions. Shown for tiled output with a subdivision chosen.

Below the choices, Nerthus shows how many divisions and tiles the build makes, and warns when a 16K build without subdivision may need more memory than most computers have.

See [Scale and Resolution](../../../using/getting-started/important/scale-and-resolution.md) for guidance on choosing terrain size and output detail.
