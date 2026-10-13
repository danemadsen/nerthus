# Transform

Transform allows you to move, rotate, and scale any terrain. The Transform node can be implemented at any point in your graph as it is a post-process effect.

> [!NOTE]
> If your terrain has a hard edge, try using the `Zero Borders` node before applying the Transform node.

## When to Use It
Use Transform when a terrain or mask needs straightforward translation, rotation, or scaling inside the graph.

## Properties

### Position

#### Offset x

Shifts the terrain or object horizontally along the X axis, allowing precise placement within the scene.

#### Offset y

Shifts the terrain or object horizontally along the Y axis, moving it forward or backward in the workspace.

#### Offset z

Moves the terrain or object vertically along the Z axis, raising or lowering its elevation.

### Scale

#### Uniform

Applies uniform scaling to all axes, maintaining the original proportions of the terrain or object.

#### Scale

Applies uniform scaling to all axes, maintaining the original proportions of the terrain or object.

#### Scale x

The non-uniform scale on the X axis, allowing independent horizontal stretching or compression.

#### Scale y

The non-uniform scale on the Y axis, allowing independent vertical stretching or compression.

### Rotation

#### Angle

The angle of rotation applied to the terrain or object, measured in degrees.

### Settings

#### Blend mode

Specifies how the transformed terrain or object is merged with the underlying data or input.

- **None**: No blending is applied; the transformed data overwrites the input.
- **Blend**: Blends the transformed data with the input using standard alpha or linear blending.
- **Add**: Adds the transformed values to the input, increasing elevation or intensity.
- **Subtract**: Subtracts the transformed values from the input, creating depressions or reductions.
- **Difference**: Emphasizes the difference between the transformed data and the input.
- **Multiply**: Multiplies the transformed data with the input, modulating the result.
- **Screen**: Uses a screen blend, lightening the result where both inputs are present.
- **Max**: Takes the maximum value between the transformed data and the input at each point.
- **Min**: Takes the minimum value between the transformed data and the input at each point.

#### Edges

Specifies how the edges of the transformed terrain or object are handled.

- **None**: No special edge handling; edges may be abrupt or unblended.
- **Thin**: Applies a thin edge blend, softening the transition at the boundaries.
- **Wide**: Applies a wide edge blend, creating a broader transition zone at the boundaries.
- **Soft**: Applies a soft, gradual edge blend for smooth transitions.

#### Quality

Sets the processing quality for the transform operation.

- **Draft**: Fastest processing with lower quality, suitable for previews.
- **Medium**: Balanced quality and performance for general use.
- **High**: Highest quality with the most detail, recommended for final output.
