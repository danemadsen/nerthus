# Shrubs

Adds low vegetation or shrub-like surface growth across the terrain.

## When to Use It
Use Shrubs when you need low vegetation or shrub-like surface growth across the terrain.

## Properties

### Shrubs

#### Count in Thousands

Specifies the number of shrubs to generate, in thousands. Controls the overall density of shrubs across the terrain.

#### Is relative

If enabled, the shrub count is relative to the terrain size or other factors, rather than being an absolute number.

#### Shape

Determines the shape or type of shrub clusters to be placed. Affects the visual appearance and distribution pattern of shrubs.

- **Point**: Places individual shrubs as single points, resulting in a scattered, sparse distribution.
- **Big**: Generates larger shrub clusters, simulating denser or more mature shrub growth.
- **BigOnly**: Restricts placement to only large shrub clusters, omitting individual or small shrubs.

#### Seed

Sets the randomization seed for shrub placement, ensuring reproducible or varied results.

### Growth

#### Seek Water

Determines how shrubs respond to water sources, influencing their tendency to grow near or away from water.

- **Seek**: Shrubs preferentially grow closer to water sources, increasing density near moist areas.
- **Avoid**: Shrubs avoid water sources, resulting in sparser growth near moist areas.

#### Avoid trees

Controls how shrubs interact with tree placement, affecting their distribution relative to trees.

- **None**: Shrubs are placed without regard to tree locations.
- **Previous**: Shrubs avoid areas where trees were previously placed, reducing overlap.
- **All**: Shrubs avoid all tree-covered areas, ensuring no overlap between shrubs and trees.

#### Health

Represents the overall health or vigor of shrubs, affecting their size, density, and appearance.

#### Spread

Controls how far shrubs can spread from their initial placement, influencing patch size and coverage.

#### Random

Introduces randomness to shrub placement and growth, creating more natural and varied patterns.

#### Simulation bias

Adjusts the simulation to favor certain environmental conditions or behaviors, influencing shrub distribution.

### Aspect Inhibition

#### Slope

Limits shrub growth on steep slopes, simulating natural growth patterns where shrubs prefer gentler terrain.

#### Slope falloff

Controls how quickly shrub density decreases as slope increases.

#### Altitude

Restricts shrub growth above or below certain altitudes, simulating elevation-based vegetation zones.

#### Altitude falloff

Controls the rate at which shrub density decreases with altitude.

### Snow Inhibition

#### Dead flow

Prevents shrub growth in areas with stagnant or 'dead' water flow, simulating natural avoidance of such regions.

#### Consolidate flows

Encourages shrubs to grow along consolidated water flow paths, such as stream banks or drainage lines.

#### Bias

Applies a directional or environmental bias to shrub placement, favoring certain areas or conditions.

### Artistic Control

#### Chaos

Increases the irregularity and unpredictability of shrub placement, creating a more natural look.

#### Blur edges

Softens the boundaries of shrub patches, blending them more smoothly into the surrounding terrain.

### Custom Inhibitors

#### Custom influence

Allows custom input maps or masks to influence shrub placement and density.

#### Trim

Removes or reduces shrubs in specified areas, providing manual control over final distribution.
