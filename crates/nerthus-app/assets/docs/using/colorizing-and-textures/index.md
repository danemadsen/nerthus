# Colorizing and Textures

Colorizing in Nerthus is built around masks. A terrain's height, slope, curvature, flow, soil, and other derived data can drive gradients, Satellite Maps, layered materials, and exported masks for downstream tools.

The goal is not only to make the viewport look finished. Good color and texture graphs also produce useful production data: albedo or diffuse maps, splat masks, erosion masks, vegetation masks, wetness masks, and other control maps that can be used in engines and DCC applications.

## Core Ideas

[Crafting Masks](crafting-masks.md) explains how grayscale masks and color lookup tables shape most texture workflows. A mask can distribute a gradient by elevation, isolate steep slopes, follow erosion flow, or target specific terrain features.

[Working with Satellite Maps](working-with-satellite-maps.md) covers Nerthus's satellite-derived color maps. Satellite Maps are useful for fast natural color variation, especially when combined with masks or refined through other color nodes.

[Synthesizing Colors](synthesizing-colors.md) shows how [Synth](../../reference/nodes/colorize/synth.md) can convert an existing color source into an editable gradient. This is helpful when you want to extract a palette from a map and then art-direct it inside Nerthus.

[Layering Textures](layering-textures.md) focuses on the [Mixer](../../reference/nodes/utility/mixer.md) node, which lets you stack multiple color layers and control them with terrain masks.

[HueErosion](hue-erosion.md) applies sediment-style logic to color maps so color can flow, collect, and weather in a way that follows the terrain.

## Useful Data Sources

* Aspect maps such as [Height](../../reference/nodes/derive/height.md), [Slope](../../reference/nodes/derive/slope.md), [Angle](../../reference/nodes/derive/angle.md), [Curvature](../../reference/nodes/derive/curvature.md), and [Peaks](../../reference/nodes/derive/peaks.md) help isolate structural terrain features.
* Generative maps such as [Flow](../../reference/nodes/derive/flow.md), [Soil](../../reference/nodes/derive/soil.md), [RockMap](../../reference/nodes/derive/rock-map.md), [Occlusion](../../reference/nodes/derive/occlusion.md), [TextureBase](../../reference/nodes/derive/texture-base.md), and [Texturizer](../../reference/nodes/derive/texturizer.md) create more natural variation than simple height or slope masks alone.
* Simulation outputs from erosion, snow, water, debris, or vegetation can become highly specific masks for color and material work.

## Practical Workflow

1. Start with large color zones, usually driven by height, slope, or a SatelliteMap.
2. Add feature masks from erosion, soil, flow, rock, or occlusion data.
3. Layer secondary color variation with [Mixer](../../reference/nodes/utility/mixer.md) or additional CLUT-style nodes.
4. Keep important masks available for export if they will drive materials, scattering, or landscape layers in another application.
5. Check exported maps as data, not just as viewport color, especially for masks and splat maps.
