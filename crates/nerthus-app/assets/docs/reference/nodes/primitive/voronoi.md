# Voronoi

Voronoi is a staple of procedural design. It creates sharp, geometric patterns. The version in Nerthus, however, is a "geo-variant". We have modified the base Voronoi shape to fit the needs of terrain design from the beginning.

## When to Use It
Use Voronoi when you want a sharp cell-based pattern as the foundation for rocky landforms, crack networks, or broken surface structure.

## Properties

### Noise

#### Scale

The perceptual scale of the Voronoi shapes. Higher values create more, smaller shapes; lower values produce fewer, larger cells.

#### Jitter

Controls the offset of the Voronoi points, ranging from perfectly symmetrical (low jitter) to highly chaotic (high jitter) cell placement.

#### Function

Specifies the distance function used to calculate Voronoi cells, affecting the shape and boundaries of each cell.

- **Euclidean**: Uses the standard Euclidean distance, resulting in natural, rounded cell shapes.
- **Manhattan**: Uses the Manhattan (grid-based) distance, producing square or diamond-shaped cells.

#### Form

Determines how the height or value of each Voronoi cell is calculated, affecting the resulting terrain or pattern.

- **C**: Voronoi cells with height determined by the cell itself, creating classic cell-based patterns.
- **N**: Voronoi cells with height determined by the nearest neighbor, resulting in smoother transitions between cells.
- **R**: Voronoi cells with height influenced by the distance to the cell's edge, creating ring-like or ridge effects.
- **P**: The default look for this geo-variant, suitable for creating mountainous terrain with prominent peaks.
- **A**: A mix between the P and M forms, blending characteristics of both for varied terrain features.
- **S**: Generates individual mountains with fully separated boundaries, emphasizing isolated peaks.
- **M**: Favors the strongest cell, ideal for creating a large mountain slope or dominant landform.
- **D**: Emphasizes ridges over cell centers, producing terrain with pronounced ridge lines.

#### Gain

Adjusts the contrast or intensity of the Voronoi pattern, amplifying or reducing the difference between cell heights.

#### Clamp

Limits the output values to a specified range, preventing extreme heights or depths in the generated pattern.

#### Seed

The randomization pattern or seed for the node's process. Changing the seed produces different Voronoi arrangements.

### Warp

#### Type

Specifies the type of warp applied to the Voronoi pattern, controlling the complexity of the distortion.

- **None**: No warping is applied; the Voronoi pattern remains unaltered.
- **Simple**: Applies basic warping to the pattern, introducing gentle distortions.
- **Complex**: Applies advanced, multi-layered warping for highly intricate and organic distortions.

#### Frequency

Controls the frequency of the warp, determining how often the pattern is distorted across the terrain.

#### Amplitude

Sets the strength of the warp effect, controlling how far the Voronoi cells are displaced.

#### Octaves

Specifies the number of detail layers used in the warp, increasing the complexity and richness of the distortion.

### Transform

#### Scale x

The non-uniform scale of the X axis, stretching or compressing the pattern horizontally.

#### Scale y

The non-uniform scale of the Y axis, stretching or compressing the pattern vertically.

#### X

Offsets the Voronoi pattern along the X axis, shifting its position horizontally.

#### Y

Offsets the Voronoi pattern along the Y axis, shifting its position vertically.
