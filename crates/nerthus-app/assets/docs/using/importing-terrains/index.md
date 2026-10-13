# Importing Terrains

Imported terrain data can be used as a starting point, a reference layer, a mask source, or a production handoff between Nerthus and another tool. Once the data is in the graph, it behaves like any other Nerthus terrain and can be eroded, reshaped, colorized, baked, or exported.

The most common import path is the [File](../../reference/nodes/primitive/file.md) node, which loads supported heightfield and image formats. Use it for DEMs, RAW heightmaps, EXR or TIFF displacement maps, masks, and color textures.

## Choosing an Import Method

Use [File](../../reference/nodes/primitive/file.md) when you have a single heightmap, mask, or color image. This is the standard choice for bringing terrain data into a graph.

Use [TileInput](../../reference/nodes/primitive/tile-input.md) when you are working with an existing tile set and need the imported data to match a tiled build workflow.

Use [Object](../../reference/nodes/primitive/object.md) when you want to bring mesh data into Nerthus and rasterize it into a heightfield. For mesh-specific workflow notes, see [Importing Meshes](../getting-started/basics/importing-meshes.md).

## Scale and Precision

Imported data often arrives with different assumptions about height range, units, bit depth, and clamping. Before you process it heavily, check whether the terrain is using the vertical range you expect.

* Use high precision formats such as EXR, TIFF, R32, or Nerthus RAW when you need to preserve height detail.
* Avoid clamping data too early if the source contains negative elevations, bathymetry, or values outside a normalized 0 to 1 range.
* Use [Match or Correct Heights](../../guides/scenarios/helpful-info/match-height.md) when you need to fit incoming DEM or heightmap data into a known maximum altitude.
* Review [Scale and Resolution](../getting-started/important/scale-and-resolution.md) when the imported terrain must match a real-world size or a downstream engine scale.

## After Import

Imported terrain usually benefits from a small cleanup stage before it becomes part of the main graph. Autolevel, Match, Clamp, Blur, Denoise, or erosion can help bring the source into a stable working range.

If the imported data is a fixed production input, consider baking it with [Baking Nodes](../baking-nodes/index.md) once the setup is correct. This keeps later graph work responsive and reduces the chance of accidentally changing the imported baseline.

For supported file types and precision notes, see [File Formats](../advanced-topics/technical-information/file-formats.md).
