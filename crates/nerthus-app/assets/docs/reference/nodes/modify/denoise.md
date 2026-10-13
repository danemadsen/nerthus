# Denoise

Denoise is a general purpose noise removal filter. Denoise systematically cleans up random noise, spikes, and extremely sharp edges.

A single iteration is appropriate in most cases. For extremely noisy terrains that require several passes, it is recommended that the desired area be masked to protect the rest of the terrain from losing important details.

## When to Use It
Use Denoise when tiny unwanted chatter needs to be smoothed out before erosion, masking, or export.

## Properties

#### Type

Selects the denoising method used to smooth noise while preserving overall features.

- **OnePass**: Runs a single denoise pass for fast, moderate smoothing.
- **TwoPass**: Runs a stronger two-stage denoise for cleaner results with better feature retention.
- **Pixels**: Targets pixel-level speckle and stair-stepping artifacts, focusing on discrete noisy pixels.

#### Amount

Strength of the denoising effect (higher values smooth more, but can soften detail).

#### Passes

Number of denoise iterations to apply for the selected method.
