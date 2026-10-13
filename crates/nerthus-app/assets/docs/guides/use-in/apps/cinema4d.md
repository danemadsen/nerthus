# Cinema4D

Nerthus terrains can be used in Cinema 4D as displacement maps or as exported terrain meshes. Use displacement maps when you want the renderer to provide the final surface detail. Use mesh exports when you need editable terrain geometry in the scene.

Assumptions:

* Terrain footprint = **5000 m x 5000 m**
* Heightmap is **0..1**, where black = 0 m and white = 2500 m
* Scene units are meters

> [!TIP]
> These are Nerthus defaults. Adjust the dimensions if your Terrain Definition uses a different width or height. See [Dimensions and Scale](../../../using/advanced-topics/technical-information/dimensions-and-scale.md) for scale guidance.

## Import as a displacement map

### 1. Export from Nerthus

Export the main heightfield from Nerthus as 32-bit EXR or TIFF when possible. Use 16-bit PNG or TIFF only when the target render setup does not need full height precision.

### 2. Set Cinema 4D units

Open the project settings and set the scene scale to meters before building the terrain object. Keeping the scene in meters makes the terrain width and displacement height easier to match to Nerthus.

### 3. Create the terrain base

Create a plane and set its width and depth to match the Nerthus terrain footprint. For the default terrain size, use `5000 m x 5000 m`.

Add enough subdivisions for the displacement workflow you are using. For viewport or standard modifier displacement, the mesh needs enough segments to show the heightfield. For render-time displacement, use the subdivision and displacement options required by your renderer.

### 4. Apply the heightmap

Load the Nerthus heightmap into the displacement channel or displacement shader used by your material or renderer. Treat the heightmap as data, not color:

* Disable sRGB or color correction on the height image.
* Use linear interpretation for EXR, TIFF, RAW-derived, or PNG height data.
* Set the displacement height to match the Nerthus terrain height, such as `2500 m`.

If the terrain displaces in the wrong direction, invert the height data or switch the displacement direction in the material/modifier settings.

## Import as a mesh

### 1. Export from Nerthus

Use the Mesher node or an export workflow that writes OBJ or FBX. A normalized mesh is usually easiest to move between tools because the terrain can be scaled after import.

### 2. Import into Cinema 4D

Import the OBJ or FBX file into the scene. If the mesh was exported at normalized scale, set its dimensions to match the Nerthus terrain:

* X = `5000 m`
* Z = `5000 m`
* Y = `2500 m`, if the vertical range was normalized

Apply or freeze transforms once the terrain is at the correct size, especially before adding deformers, materials, or collision workflows.

## Common gotchas

* Heightmaps must be treated as linear data. Gamma correction can flatten or exaggerate terrain height.
* Displacement needs enough geometry or render-time subdivision to show detail.
* Keep width and height values tied to the Nerthus Terrain Definition.
* For general export principles, see [Universal Instructions](../index.md).
