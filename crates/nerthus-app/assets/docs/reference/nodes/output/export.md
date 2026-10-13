# Export

Exports terrain, masks, or textures to external files.

## When to Use It
Use Export when you want a terrain, mask, or texture written to a file in a build, under its own name and format.

## Properties

#### Format

Specifies the file format for the exported output. Choose the format that best fits your workflow or target application.

- **Pure Raw**: Exports the data in Nerthus's native RAW format, preserving full precision and compatibility with Nerthus.
- **HalfRaw**: Exports the data as 16-bit half-precision RAW, balancing file size and detail.
- **HalfRaw16**: Exports the data as 16-bit half-precision RAW with explicit 16-bit formatting.
- **UshortRaw**: Exports the data as unsigned short (16-bit) RAW, suitable for many terrain and 3D applications.
- **UshortRaw16**: Exports the data as unsigned short (16-bit) RAW with explicit 16-bit formatting.
- **FloatRaw**: Exports the data as 32-bit floating point RAW, providing maximum precision for advanced workflows.
- **FloatRaw32**: Exports the data as 32-bit floating point RAW with explicit 32-bit formatting.
- **JPEG**: Exports the data as a JPEG image, using lossy compression for reduced file size and broad compatibility.
- **PNG8**: Exports the data as an 8-bit PNG image, suitable for masks and lightweight outputs.
- **PNG16**: Exports the data as a 16-bit PNG image, preserving more detail for heightfields and masks.
- **PNG64**: Exports the data as a 64-bit PNG image, supporting high dynamic range and advanced workflows.
- **TIFF8**: Exports the data as an 8-bit TIFF image, commonly used for masks and simple outputs.
- **TIFF16**: Exports the data as a 16-bit TIFF image, suitable for high-quality heightfields and masks.
- **TIFF32**: Exports the data as a 32-bit TIFF image, supporting high dynamic range and scientific workflows.
- **EXR**: Exports the data as an OpenEXR image, ideal for high dynamic range and professional VFX pipelines.

#### Location

Specifies where the exported file will be saved.

- **BuildFolder**: Exports the file to the project's build/output folder.
- **Custom**: Allows you to specify a custom folder for the exported file.
- **Explicit**: Exports the file to an explicitly defined path, overriding other location settings.

#### Output path

Defines the full file path or directory where the exported file will be saved.

#### If file exists

Determines what happens if a file with the same name already exists at the export location.

- **Overwrite**: Replaces the existing file with the new export.
- **Increment**: Automatically increments the filename to avoid overwriting existing files.

#### Prepare for Bridge

Prepares the exported file for use with the Bridge system, enabling compatibility with external tools or workflows.
