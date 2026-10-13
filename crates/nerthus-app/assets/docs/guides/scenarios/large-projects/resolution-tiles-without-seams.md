# Choose Resolution and Tiles Without Creating Seams

Tile seams are often caused before the tile build runs.

The graph may be normalizing per tile, processing a tile-unfriendly branch locally, using the wrong tile orientation, or building without a baked world-space base. Increasing blend can hide some problems, but it cannot fix a different value range baked into every tile.

> [!TIP]
> Run a small neighboring-tile test before launching the full-world build.

## What usually causes seams

If height jumps at tile borders, look for local range adjustment or normalization happening too late. If masks change from tile to tile, an Autolevel or Equalize-style operation may be acting per tile. If erosion creates a border artifact, the tile may not have enough context. If the target app places tiles incorrectly, check naming, suffix pattern, leading zeroes, organization, and Y orientation in [Tiles](../../../ui/interface/build-options/tiles.md).

## A safer tile workflow

Build the world-stage terrain first: macro shapes, major masks, and global range operations. Bake tile-unfriendly stages at the closest practical full-world resolution. Keep normalization and global value decisions before the tiled stage.

Choose tile size from the target engine or DCC, not just from what is convenient in Nerthus. Set the tile naming and orientation deliberately. Then build a 2x2 or adjacent-tile test and inspect borders in the target app before launching the full build.

> [!WARNING]
> Increasing blend will not fix every seam. If each tile has a different value range, the seam is already part of the data.

For lower resolutions, you can also use Split Build, as the tiling process is made entierly after the build, however this can be very resource intensive depending on graph complexity.
