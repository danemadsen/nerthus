# Snow

Snow simulates snow fall, melt, and settling like in nature, and controls how the snow adheres and flows along the terrain.

## When to Use It
Use Snow when a terrain needs snow that falls, melts, and settles as it would in nature.

## Properties

### Snow

#### Duration

Controls the length of time snow accumulates on the terrain, affecting overall snow coverage and depth.

#### Intensity

Determines the rate at which snow falls and accumulates, influencing the thickness and density of the snow layer.

#### Settle Duration

Specifies the time it takes for freshly fallen snow to settle and compact, impacting the smoothness and stability of the snow surface.

### Melt

#### Melt type

Defines the method by which snow melts from the terrain, affecting the pattern and distribution of melting.

- **Uniform**: Snow melts evenly across the entire terrain, resulting in a consistent reduction of snow coverage.
- **Directional**: Snow melts preferentially in a specific direction, simulating effects such as sun exposure or wind, leading to uneven snow retreat.

#### Melt

Defines the method by which snow melts from the terrain, affecting the pattern and distribution of melting.

#### Melt remnants

Specifies the amount and appearance of snow left behind after melting, such as patches or traces of residual snow.

#### Direction

Sets the primary direction for directional melting, influencing which areas of the terrain lose snow first.

#### Snow line

Defines the altitude or threshold below which snow does not accumulate, simulating natural snow lines on mountains.

### Advanced

#### Slip-off angle

Specifies the minimum slope angle at which snow will slide off, affecting snow accumulation on steep surfaces.

#### Adhered snow mass

Controls the amount of snow that remains attached to surfaces, even on steep or vertical terrain.

#### Model scale

Sets the scale of the terrain model, influencing how snow parameters are interpreted relative to real-world dimensions.

#### Real Scale

Specifies the actual physical scale represented by the terrain, used for accurate snow simulation and parameter scaling.

#### Terrain Scale

Adjusts the scaling factor for the terrain, affecting how snow features are distributed and visualized.

#### Verticality

Controls the influence of vertical surfaces on snow accumulation, determining how much snow can adhere to cliffs and steep slopes.

#### Seed

Sets the randomization seed for snow distribution, ensuring reproducible results or introducing variation in snow patterns.
