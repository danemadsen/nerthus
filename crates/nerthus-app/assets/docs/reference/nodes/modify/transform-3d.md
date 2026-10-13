# Transform3D

Applies full 3D transformation to the terrain, including rotation and spatial adjustment.

## When to Use It
Use Transform3D when a terrain needs rotating in three dimensions, such as tilting it, rather than only moving or scaling it on the plane.

## Properties

### Position

#### Drop to Floor

Automatically positions the object so its lowest point rests on the ground plane, preventing it from floating or being embedded below the surface.

#### Offset x

Shifts the object horizontally along the X axis, allowing precise placement within the 3D space.

#### Offset y

Shifts the object horizontally along the Y axis, moving it forward or backward in the 3D scene.

#### Offset z

Moves the object vertically along the Z axis, raising or lowering its elevation relative to the ground.

### Scale

#### Scale

Group containing parameters for scaling the object or mesh, affecting its size and proportions in 3D space.

### Rotation

#### Pitch

Rotates the object around the X axis, tilting it forward or backward.

#### Yaw

Rotates the object around the Y axis, turning it left or right.

#### Roll

Rotates the object around the Z axis, spinning it clockwise or counterclockwise.

### Quality

#### Antialiasing

Enables or adjusts antialiasing for the 3D transformation, reducing jagged edges and improving visual smoothness.
