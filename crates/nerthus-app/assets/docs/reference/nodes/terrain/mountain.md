# Mountain

Mountain is one of the most versatile primitives in Nerthus. It uses a modulated Voronoi pattern and distortions to create realistic mountain shapes, ready for further modification and erosion.

> [!NOTE]
> For simple results, such as distant mountains that don't require much detail, you can create a Mountain primitive with a scale of your choice and add the Erosion node. This is the easiest way to create a lot of simple assets.

## When to Use It
Use Mountain when you need a fast large-scale landform that can anchor the rest of the terrain graph.

## Properties

#### Scale

Controls the overall size and extent of the mountain. Higher values create broader, more expansive mountain shapes.

#### Height

Sets the maximum elevation of the mountain, determining how tall the peak rises above the terrain.

#### Style

Specifies the geological and visual style of the mountain, affecting its shape, erosion, and surface features.

- **Basic**: Generates a simple, unmodified mountain with minimal erosion or geological detail.
- **Eroded**: Applies erosion effects to the mountain, creating realistic weathering, gullies, and worn surfaces.
- **Old**: Simulates an ancient mountain with softened features, rounded slopes, and significant erosion.
- **Alpine**: Creates sharp, rugged alpine features with steep faces and dramatic relief, typical of young mountain ranges.
- **Strata**: Adds visible geological strata or layers to the mountain, emphasizing sedimentary or volcanic banding.

#### Bulk

Controls the mass and volume of the mountain, affecting how thick or substantial the formation appears.

- **Low**: Produces slender, less massive mountains with more delicate features.
- **Medium**: Creates mountains with a balanced amount of mass, offering a natural appearance.
- **High**: Generates thick, heavy mountains with substantial volume and broad bases.

#### Reduce details

Reduces the amount of fine surface detail on the mountain, resulting in a smoother, more simplified shape.

#### Seed

Randomization seed for generating unique variations of the mountain. Changing the seed produces different shapes and arrangements.

#### X

Offsets the mountain horizontally along the X axis, shifting its position within the terrain.

#### Y

Offsets the mountain horizontally along the Y axis, shifting its position within the terrain.
