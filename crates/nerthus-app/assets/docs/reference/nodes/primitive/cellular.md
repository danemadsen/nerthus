# Cellular

Cellular creates cell-based patterns and noises that can be quite versatile as they're ever-present in nature.

## When to Use It
Use Cellular when you need cell-based patterns, such as cracked ground, scales, or rocky cells, as a base or a mask.

## Properties

#### Size

Controls the overall scale of the cellular pattern. Higher values produce smaller cells (finer detail); lower values produce larger cells (coarser detail).

#### Density

Controls how many feature points are generated per area. Higher density creates more, smaller cells; lower density creates fewer, larger cells.

#### Jitter

Randomizes feature-point placement. Low values are more uniform/structured; higher values create more organic, irregular cells.

#### Metric morph

Blends between the paired distance behaviors implied by the selected Distance Metric (e.g., Euclidian ↔ Manhattan/Chebyshev), changing cell geometry continuously.

#### Distance metric

Selects the distance-space used to evaluate the cellular field, affecting the fundamental shape of cells and borders.

- **Euclidian_Manhattan**: Morph between rounded Euclidian cells and diamond-like Manhattan cells (axis-aligned, sharper corners) using Metric Morph.
- **Euclidian_Chebyshev**: Morph between rounded Euclidian cells and square-like Chebyshev cells using Metric Morph.
- **Minkowski**: Generalized metric that spans a continuous family of distance shapes (useful for smoothly varying from rounded to more boxy forms).

#### Coordinate axes jitter

Adds jitter in coordinate/axis space before evaluation, warping the field and introducing directional wobble and boundary distortion.

#### CAJ clustering

Controls how Coordinate Axes Jitter groups together. Low values spread distortion evenly; higher values create patchy/clumped distortion regions.

#### Anisotropy

Stretches the pattern directionally, producing elongated/elliptical cells and emphasizing a dominant direction.

#### Distance function

Defines what value is output from the cellular field (cell IDs, distances to points, borders, or edges).

- **Hash**: Outputs a stable random value per cell region (cell ID-style mask), useful for random patching, selection, and variation.
- **DistanceInv**: Outputs inverse distance to the nearest feature point (bright centers fading toward borders).
- **Distance2Inv**: Outputs inverse distance to the second-nearest feature point, producing a different internal falloff useful for variation and breakup.
- **DistanceDiff**: Outputs the difference between nearest distances (classic Voronoi edge emphasis), great for ridges, veins, and cell borders.
- **DistanceToEdge**: Outputs distance to the nearest cell border, ideal for crack networks and edge masks when remapped.

#### Seed

Changes the random distribution of feature points without changing other settings.
