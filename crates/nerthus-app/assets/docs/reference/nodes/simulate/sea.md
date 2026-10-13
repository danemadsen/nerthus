# Sea

Sea is a multi-function water surface generator and coastal erosion tool. Its primary purpose is to create a water surface, while Coastal Erosion creates beaches and cliffs.

The water surface can be generated either as global surface or filled from the edges without affecting inland features. In the latter mode, you can have custom control over which edges fill inward by using the `Edge` input mask.

## When to Use It
Use Sea when you want one node to both establish a water surface and shape nearby coastal response.

## Properties

#### Sea

The main Sea node, responsible for generating the sea area in the terrain. Contains options to control the global and surrounding sea behavior.

- **Global**: Enables the sea to cover the entire terrain globally, overriding local variations.
- **Surrounding**: Restricts the sea to only surround the terrain, creating a border or coastal effect rather than filling the entire area.

#### Level

Controls the water level of the sea. Higher values raise the sea, submerging more terrain, while lower values expose more land.

#### Coastal erosion

Applies erosion to the coastal areas where the sea meets the land, simulating natural shoreline wear and shaping.

#### Shore size

Determines the width of the shore created by coastal erosion. Larger values produce broader beaches or transition zones.

#### Shore height

Sets the elevation difference between the sea level and the top of the shore, affecting the steepness of the coastline.

#### Variation

Controls the amount of natural variation and irregularity along the coastline, making the shore appear more organic and less uniform.

#### Uniform variations

When enabled, applies consistent variation along the entire coast, resulting in a more uniform shoreline appearance.

#### Extra cliff details

Adds additional detail and features to cliffs along the coast, enhancing realism and visual interest in steep shoreline areas.

#### Render surface

Determines whether the sea surface is rendered in the final terrain output. Enable to visualize the water surface.
