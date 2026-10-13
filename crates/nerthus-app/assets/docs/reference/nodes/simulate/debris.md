# Debris

Simulates loose debris across the terrain surface to create broken rocky accumulation.

## When to Use It
Use Debris when slopes need loose, broken rock gathered across them.

## Properties

#### Debris amount

Controls how many individual rocks or debris pieces are emitted across the terrain.

#### Amount multiplier

Scales the emitted debris amount globally without changing other parameters, allowing quick adjustment of overall density.

#### Scale

Controls the overall scale of debris elements, affecting their size relative to the terrain.

#### Friction

Determines how easily debris slides; higher values make debris stop sooner and form steeper piles.

#### Restitution

Controls energy loss on impact, affecting how far debris bounces or continues sliding after being emitted.

#### Size

Sets the overall size of the emitted rocks or debris pieces.

#### Height

Defines the vertical offset or emission height for debris release.

#### Seed

Randomization seed for generating varied debris distributions.

#### Shape

Determines the geometric style of debris elements, affecting their appearance.

- **Rounded**: Produces smoother, rounded rock fragments.
- **Sharp**: Produces jagged, angular rock fragments.

#### Render still rocks

Enables rendering of non-moving debris that has settled on the terrain.

#### Distribution

Specifies how debris is distributed across the emission area.

- **Uniform**: Evenly spreads debris across the emission area.
- **Natural**: Creates more organic, irregular clustering of debris.

#### Export point cloud

Enables the export of debris positions as a point cloud for further processing or use in external applications.
