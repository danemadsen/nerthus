# File Formats

Nerthus can read and write common terrain, image, and mesh formats used by DCC applications, game engines, and custom pipelines. Choose the format based on the kind of data you are moving and how much precision it needs to preserve.

## Supported Formats

### Bitmap and Data Formats

| Format | Read | Write | Notes |
|---|:---:|:---:|---|
| OpenEXR (`.exr`) | Yes | 32-bit | Best general-purpose choice for high precision heightfields and maps. |
| TIFF (`.tif`, `.tiff`) | Yes | 8, 16 and 32-bit | Good compatibility for heightfields, masks, and color maps. |
| PNG (`.png`) | Yes | 8 and 16-bit, and 16 bits per channel color | Useful for masks, color maps, and 16-bit heightfield workflows. |
| Raw (`.raw`, `.r16`, `.r32`) | `.raw` and `.r32` | 16-bit unsigned or half float, 32-bit float | Headerless heightfield data for tools that require raw values. |
| Pure Raw (`.graw`) | No | 32-bit | A high-fidelity raw format, with its size in a `.meta` file beside it. |
| JPEG (`.jpg`, `.jpeg`) | Yes | 8-bit | Color/reference images only; avoid for height data. |
| WebP (`.webp`) | Yes | No | Color/reference images only. |
| BMP, TGA, GIF | Yes | No | Legacy color/reference image support. |
| HDR (`.hdr`) | Yes | No | High dynamic range image data. |
| JPEG 2000 (`.jp2`), DDS (`.dds`), MIFF (`.miff`) | Yes | No | Image import for source/reference material. |
| PSD (`.psd`, `.psb`) | Yes | No | Image import for source/reference material. |
| SVG (`.svg`) and Windows metafiles (`.emf`) | Yes | No | Vector source/reference material. |
| Camera raw (`.dng`) | Yes | No | Lossless DNG photographs. |

> [!NOTE]
> Raw files may use `.raw`, `.r16`, or `.r32` depending on the target application. Different tools use different extension and precision conventions, so always confirm the expected bit depth before export.

### Mesh and Object Formats

Nerthus can import 3D objects through the Object node and export terrain meshes through the Mesher, PointCloud and TextureBaker nodes.

* Wavefront OBJ (`.obj`): import and export
* Autodesk Filmbox (`.fbx`): import and export
* Collada (`.dae`): import and export
* glTF (`.gltf`, `.glb`): import and export
* Point clouds (`.ply`, `.xyz`): export

## Import Formats

Use bitmap/data formats when you need to bring in a heightfield, mask, color map, or reference image. Use mesh/object formats when you need an external model or terrain surface as part of a graph.

For heightfields, prefer 32-bit EXR, TIFF or R32 when fidelity matters. For masks and secondary maps, 16-bit PNG or TIFF is usually enough. Use 8-bit formats for reference images, color sources, or simple masks where precision is not critical.

## Export Formats

For production heightfields, EXR and TIFF provide the best balance of precision and compatibility. Use `.r32` or `.raw` when another tool expects simple raw data or when you need to move data between Nerthus projects with minimal loss.

For game engines and DCC tools, the correct export depends on the target:

* Use 32-bit EXR or TIFF for high-quality displacement in rendering applications.
* Use 16-bit PNG, TIFF, or RAW when the target tool expects 16-bit heightfields.
* Use OBJ or FBX when the target workflow needs a baked terrain mesh.
* Use color formats such as PNG, TIFF, or EXR for masks, splat maps, color maps, and supporting outputs.

## Project Formats

### `.terrain`

Nerthus projects are saved as `.terrain` files. This is the main graph file format.

Automation and bridge workflows can run `.terrain` files without manually opening the full interface. See [Automation](../../../developers/automation/index.md) for the supported command-line and file-based automation workflow.

## Precision

Nerthus stores and processes heightfields in 32-bit floating point precision. Export at lower precision only when the target workflow can tolerate the reduced range.

### 32-bit

Use 32-bit output for primary heightfields, displacement maps, and any data that must survive further processing. EXR and TIFF are the most broadly useful 32-bit formats. R32 is useful for custom pipelines that read simple floating point data directly.

> [!NOTE]
> If you are saving output from Nerthus to bring back into Nerthus, use `.r32` for maximum fidelity and efficient loading.

### 16-bit

Use 16-bit output for many masks, secondary maps, and heightfields headed to tools that require 16-bit terrain input. This can save disk space and memory, but may introduce visible stepping on very smooth or high-relief displacement.

Some game engine terrain workflows require 16-bit unsigned RAW data.

### 8-bit

Use 8-bit output for reference images, color maps that do not require high precision, and simple black-and-white masks. Avoid 8-bit output for primary terrain heightfields unless the loss of precision is acceptable.

## Raw Technical Details

For custom workflows, 32-bit float (`.r32`) and 16-bit unsigned short (`.raw`) are the simplest terrain data formats. They are headerless binary arrays that can be read directly from disk.

* `.r32` stores one 32-bit IEEE 754 float per pixel.
* `.raw` stores one 16-bit unsigned integer per pixel.
* Values are stored in little-endian byte order.
* The files are intended for square heightfields.

To infer the resolution, divide the byte length by the element size, then take the square root. Use 4 bytes per pixel for `.r32` and 2 bytes per pixel for 16-bit `.raw`.

The `.r32` format stores normalized floating point values between `0.0f` and `1.0f`. The 16-bit `.raw` format stores integer values between `0` and `65535`.

Both raw formats are recommended for grayscale heightfield data only.
