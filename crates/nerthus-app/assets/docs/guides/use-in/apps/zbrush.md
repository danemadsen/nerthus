# ZBrush

ZBrush is usually best used after Nerthus has produced either a mesh or a high precision height/displacement map. Use a mesh when you want to sculpt or decimate terrain geometry. Use a height map when you want to apply Nerthus terrain detail as displacement or as an alpha-driven sculpting source.

Assumptions:

* Terrain footprint = **5000 m x 5000 m**
* Heightmap is **0..1**, where black = 0 m and white = 2500 m
* Nerthus is the source of the terrain scale and height range

> [!TIP]
> If you need exact world scale after ZBrush, record the Nerthus terrain width and height before export. See [Dimensions and Scale](../../../using/advanced-topics/technical-information/dimensions-and-scale.md) for scale guidance.

## Import as a mesh

### 1. Export from Nerthus

Use the Mesher node or export workflow to write OBJ or FBX. For sculpting, choose a mesh density that is high enough to preserve the terrain silhouette but not so high that it becomes hard to work with in ZBrush.

### 2. Import into ZBrush

Import the OBJ or FBX file as a tool. Confirm that the terrain orientation is correct and that the terrain is not unexpectedly flipped.

If the mesh was exported at normalized scale, keep a note of the original Nerthus dimensions. ZBrush workflows often focus on sculpting scale rather than real-world units, so you may need to restore the exact dimensions later in another DCC or engine.

### 3. Sculpt or refine

Use ZBrush for local sculpting, cleanup, decimation, or detail work that is easier to perform by hand. Keep broad terrain shape changes intentional, because round-tripping a heavily changed mesh back into a procedural Nerthus graph is usually not lossless.

## Use a height map or alpha

### 1. Export a high precision map

Export a 16-bit or 32-bit height map from Nerthus. Prefer EXR or TIFF when the next tool in the chain supports it. Avoid 8-bit formats for terrain height because they can create visible stepping.

### 2. Load the map as data

When using the height map as an alpha, displacement source, or sculpting reference, keep it linear. Do not apply color correction or gamma changes to height data.

### 3. Apply carefully

Height maps from Nerthus represent terrain height, not color. Test the displacement strength on a duplicate tool or layer before committing it to a sculpt.

## Exporting back out

Only export back from ZBrush when the sculpted result is meant to become a mesh asset or a new height source. If the terrain still needs procedural edits, keep the Nerthus `.terrain` file as the source of truth and use ZBrush for final mesh refinement.

For general export principles, see [Universal Instructions](../index.md).
