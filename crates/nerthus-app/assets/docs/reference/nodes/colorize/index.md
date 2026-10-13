# Colorize

Colorize nodes create, process, split, combine, and weather color data. Use this family when the terrain needs visual color, material palette work, or channel operations for downstream texturing.

Start here when you need to:

- create a first-pass terrain color map
- layer, tint, split, merge, or process color channels
- make color follow terrain flow, sediment, or weathering behavior
- prepare color data for texturing, rendering, or export

Useful starting points:

- [SatelliteMap](satellite-map.md) and [SuperColor](super-color.md) for terrain-aware base color
- [HueErosion](hue-erosion.md) and [Weathering](weathering.md) for weathered or flow-aware color passes
- [WaterColor](water-color.md) for water-specific color
- [RGBMerge](rgb-merge.md), [RGBSplit](rgb-split.md), [HSL](hsl.md), [Gamma](gamma.md), and [Tint](tint.md) for channel and color adjustments
- [Synth](synth.md) and [CLUTer](cluter.md) for source-driven or lookup-based color workflows

## In this section

- [CLUTer](cluter.md): CLUTer lets you create color gradients which can be mapped to texture masks for creating color maps
- [HueErosion](hue-erosion.md): Applies erosion-style transport and deposition to color data, creating more natural weathered color variation
- [Gamma](gamma.md): The Gamma node applies gamma adjustment to the heightfield (or mask) values
- [HSL](hsl.md): The HSL node adjusts Hue, Saturation, and Lightness of a color input
- [RGBMerge](rgb-merge.md): RGBMerge combines three heightfields into a single RGB color output
- [RGBSplit](rgb-split.md): RGBSplit takes an RGB color input and outputs separate heightfields derived from the Red, Green, and Blue channels
- [SatelliteMap](satellite-map.md): Generates terrain color from preset satellite-style palettes with controllable range and processing
- [Splat](splat.md): Splat creates a single heightfield by combining the channels of an RGBA color input
- [SuperColor](super-color.md): Applies procedural terrain colorization using texture, flow, peaks, and palette controls
- [Synth](synth.md): Synth takes a bitmap image (photo or artwork) and converts it to CLUT map that can be used with a texture mask to colorize your terrain
- [Tint](tint.md): Tint converts a heightmap or mask into a color texture by blending between two colors across the input range
- [WaterColor](water-color.md): Colorizes water bodies using palette-based shallow and deep water tones
- [Weathering](weathering.md): Weathering adds subtle tonal variation based on surface form: it brightens protrusions and exposed features, and darkens crevices and sheltered areas
