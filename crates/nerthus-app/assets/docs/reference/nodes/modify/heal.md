# Heal

The Heal node can reconstruct damaged, low res, or 8-bit data with 16-bit, high resolution fidelity. The Heal node can take quantized images with extreme banding of any shape, and clean up the terrain into a naturalistic output.

Heal can consume 8-bit data, visually damaged images, or low-quality satellite data, even with bad JPEG artifacts, and turn it into a reasonably clean heightfield. It is also useful for upscaling lower-resolution data such as Google map extracts.

> [!NOTE]
> Obviously, not all data can be reconstructed, but you can get a fair bit of quality back. Below is an example where we downsample a 16-bit heightfield to 8-bit, and then reconstruct it using the Heal node.

## When to Use It
Use Heal when imported, damaged, or low-quality height data needs cleanup and reconstruction before other terrain work begins.

## Properties

#### Reconstruction

Controls how aggressively missing or damaged areas are rebuilt from surrounding data (higher values preserve continuity but can soften detail).

#### Passes

Number of healing iterations to run; more passes produce a cleaner, more continuous surface but may blur fine detail.
