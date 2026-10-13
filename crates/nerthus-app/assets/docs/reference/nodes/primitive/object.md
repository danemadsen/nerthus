# Object

Loads external object or mesh data for use inside the graph.

## When to Use It
Use Object when you want a 3D model's shape as a heightfield in the graph.

## Properties

### Object

#### Mesh

Specifies the 3D mesh file to import. The mesh will be converted and rendered as a heightfield for use in terrain generation.

#### Relative path

Defines the file path to the mesh relative to the project or asset directory. Ensures portability and correct asset referencing.

### Position

#### Drop to Floor

Automatically positions the object so its lowest point rests on the terrain base, preventing it from floating or being embedded below the surface.

#### Offset x

Shifts the object horizontally along the X axis, allowing precise placement within the terrain.

#### Offset y

Shifts the object horizontally along the Y axis, allowing precise placement within the terrain.

#### Offset z

Moves the object vertically along the Z axis, raising or lowering it relative to the terrain surface.

### Scale

#### Scale

Group containing parameters for scaling the imported object.

### Rotation

#### Pitch

Rotates the object around the X axis, tilting it forward or backward.

#### Yaw

Rotates the object around the Y axis, turning it left or right.

#### Roll

Rotates the object around the Z axis, spinning it clockwise or counterclockwise.

### Quality

#### Antialiasing

Enables or adjusts antialiasing for the heightfield rendering, reducing jagged edges and improving visual smoothness.
