# File

The File node loads any supported image or data file as a terrain. It
supports all major file formats, and you can load either a grayscale
image as a heightfield, or a RGB color image as a color map.

## When to Use It
Use File when a heightfield, mask, or color map made elsewhere, as an image or data file, needs to come into the graph.

## A note about scale

In most cases, you can use the `Raw` Scale setting. This will interpret
the data within a normal 8, 16, or 32 bit data range. If your uses
excessive scale values that go below zero, you may need to use the
`Normalized` or `Mapped` mode. If you wish to handle scale manually, you
should turn on `Clamp` in [Modifier Stack](../../../ui/interface/property-editor/modifier-stack.md) to ensure data does not go beyond
anticipated ranges which may cause other nodes to break.

The `Mapped` mode is mainly used for communicating with other
applications in automation scenarios where the incoming and outgoing
data must retain a specific number range.

## Properties

### File

#### File

Specifies the file to load as input for the node. This can be a heightmap, mask, or other supported data file.

#### Relative path

Defines the file path relative to the project or asset directory, ensuring portability and correct asset referencing.

#### Is RGB

Indicates whether the loaded file should be interpreted as an RGB image. Enable this for color maps or multi-channel data.

#### Enforce linear gamma

Forces the loaded terrain or image data to use linear gamma correction, ensuring accurate color and height interpretation.

#### Allow unclamped

Allows values outside the standard 0–1 range to be loaded, preserving high dynamic range or special data in the file.

#### Crop to square

Automatically crops the loaded file to a square aspect ratio, which is often required for terrain processing.

#### Prepare for Bridge

Prepares the loaded file for use with the Bridge system, enabling compatibility with external tools or workflows.

### Caching

#### Purge cache

Clears the cached version of the loaded file, forcing a reload from disk on the next access.

#### Never cache

Prevents the loaded file from being cached, ensuring it is always read directly from disk.
