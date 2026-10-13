# Crafting Masks

## What is Color Lookup/CLUT?

All color texture generation in Nerthus relies on applying color ramps (gradients) through a black-and-white mask.

A simple example: the darkest part of the gradient maps to the lowest terrain elevations, while the brightest part maps to the highest. Intermediate grayscale values distribute the gradient proportionally. This is known as a CLUT or Color Look Up Table.

For example, map the gradient from a CLUTer node directly to the heightfield: its colors follow the elevation. A more detailed, colorful gradient distributes across the terrain in the same way.

Now, if you use the same CLUT map but feed it a flow map (from the Flow node) instead of the terrain height, the texture will follow the flow lines defined by the flow map, producing a very different result.

### Colorization with Texture Nodes

Nerthus provides nodes such as [TextureBase](../../reference/nodes/derive/texture-base.md) and [Texturizer](../../reference/nodes/derive/texturizer.md) provide complex colorization masks based on the terrain's inherent features.

Which can create more complex textures such as this:

## Data Maps

Data Maps are specialized masks for selecting key terrain properties like slope, angle, and curvature, or for generating complex simulation-based data such as water flow and soil deposits. Additional maps like TextureBase produce pseudo-random texture masks for quick, easy color texturing.

Unlike the classic approach that combines basic data (slope, angle) with noise (e.g., Perlin) or depends heavily on erosion flow outputs - often requiring careful seed tuning - Data Maps introduce controlled randomness derived from systematic terrain analysis. This produces more natural, believable color maps.

### Aspect Maps

Aspect Data Maps mask out aspects of the terrain such as Height, Slope, Curvature, Angle, Peaks, etc.

### Generative Maps

Generate Data Maps run various algorithms on the terrain to generate information such as Flows, Soil, Rock Occlusion, etc.

Data Maps can be combined together or with Texture nodes to create complex texture masks for colorization.
