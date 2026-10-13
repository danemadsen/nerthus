# Primitive

Primitive nodes create foundational shapes, gradients, patterns, and noise sources. Use this family when you need a starting form, reusable mask, procedural signal, or shape generator at the beginning of a graph.

Start here when you need to:

- create a clean shape or gradient
- generate base noise for later erosion or surface work
- build procedural masks and reusable patterns
- import a file, object, or tile source into the graph

Useful starting points:

- [Noise](noise.md), [Perlin](perlin.md), [MultiFractal](multi-fractal.md), and [Cellular](cellular.md) for general procedural noise
- [Shape](shape.md), [Cone](cone.md), [Hemisphere](hemisphere.md), [LinearGradient](linear-gradient.md), and [RadialGradient](radial-gradient.md) for simple forms
- [Cracks](cracks.md), [Voronoi](voronoi.md), [Gabor](gabor.md), and [LineNoise](line-noise.md) for stronger pattern character
- [File](file.md), [Object](object.md), and [TileInput](tile-input.md) for external or tiled inputs

## In this section

- [Cellular](cellular.md): Cellular creates cell-based patterns and noises that can be quite versatile as they're ever-present in nature
- [Cellular3D](cellular-3d.md): Generates three-dimensional cellular noise useful for volumetric-looking patterns and complex terrain breakup
- [Cone](cone.md): Generates a simple conical gradient that is useful as a primitive shape or mask
- [Constant](constant.md): The Constant node is the most basic primitive available in Nerthus
- [Cracks](cracks.md): The Cracks primitive creates large, cracked patterns on a flat base
- [CutNoise](cut-noise.md): A creased noise of sharp, cut-like ridges and valleys
- [DotNoise](dot-noise.md): Generates dotted or speckled noise patterns for texturing and breakup
- [Draw](draw.md): Draw draws entire mountain ranges in the shape you choose
- [DriftNoise](drift-noise.md): DriftNoise is a unique primitive that can create overlapping "shelves". It is a versatile primitive noise that can be used in various scenarios from cliffs to crack production
- [File](file.md): The File node loads any supported image or data file as a terrain
- [FileSequence](file-sequence.md): One image from a folder of images, chosen by its frame number
- [Gabor](gabor.md): Gabor noise is one of the lesser used noises, but provides a wonderful pattern-friendly noise that can be used to simulate various aspects of nature
- [Hemisphere](hemisphere.md): Generates a hemispherical gradient useful for domes, mounds, and radial falloffs
- [LinearGradient](linear-gradient.md): Generates a straight linear gradient across the terrain
- [LineNoise](line-noise.md): LineNoise is a geometry generator that creates sets of lines which can be distorted and used with other noises and fractals to produce layered ridges
- [MultiFractal](multi-fractal.md): MultiFractal creates a multi-noise that can have a large level of variations. Often useful for texturing
- [Noise](noise.md): Applies controlled single-pixel noise onto your terrain
- [Object](object.md): Loads external object or mesh data for use inside the graph
- [Pattern](pattern.md): Generates repeatable geometric or decorative patterns for masking and stylized terrain work
- [Perlin](perlin.md): Perlin is the most basic and essential noise. In Nerthus, we've made a "geo-variant" by modifying the base Perlin shape to fit the needs of terrain design from the beginning
- [RadialGradient](radial-gradient.md): Generates a circular gradient radiating outward from a center point
- [Shape](shape.md): Generates basic geometric terrain shapes for use as primitives or masks
- [TileInput](tile-input.md): Reads a set of tiles for 1:1 input into a tiled build, or combines it into a single input
- [Voronoi](voronoi.md): Voronoi is a staple of procedural design and creates sharp, geometric patterns. In Nerthus, we've made a "geo-variant" by modifying the base Voronoi shape to fit the needs of terrain design from the beginning
- [WaveShine](wave-shine.md): Generates wave-like reflective or rippling patterns useful for stylized surfaces and water-like forms
