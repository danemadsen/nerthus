# SlopeWarp

SlopeWarp is a versatile, low-level node that warps a terrain along the slopes of a Guide terrain or mask. It behaves similarly to SlopeBlur.

## When to Use It
Use SlopeWarp when you want to push a terrain's shapes along the slopes of a guide terrain or mask.

## Properties

### SlopeWarp

#### Intensity

Controls the strength of the slope warp effect. Higher values produce more pronounced warping and distortion of the terrain along the slope.

#### Iterations

Specifies how many times the slope warp operation is applied. Increasing iterations can create more complex or exaggerated warping effects.

#### Direction

Sets the primary direction in which the slope warp is applied. Adjust this to control the orientation of the warping across the terrain.

#### Normalized

When enabled, normalizes the effect so that the intensity is consistent regardless of terrain scale or resolution.

### Quality

#### Quality

Settings related to the quality and precision of the slope warp operation. Higher quality may increase processing time but results in smoother, more accurate warping.

#### Antialiasing

Applies antialiasing to the slope warp effect, reducing jagged edges and improving visual smoothness.

- **Off**: No antialiasing is applied. May result in sharper but potentially jagged edges.
- **x4**: Applies 4x antialiasing for moderate smoothing of edges.
- **x16**: Applies 16x antialiasing for maximum edge smoothing and highest visual quality.
