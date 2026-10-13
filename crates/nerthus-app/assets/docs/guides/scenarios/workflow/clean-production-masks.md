# Create Clean Masks for Materials, Biomes, and Shaders

A mask can look good in Nerthus and still fail in production.

The usual symptoms are muddy material blending, strange biome borders, unreliable weight maps, or a shader that needs too much cleanup. The mask may be visually interesting, but it was not built as data.

> [!TIP]
> Name the mask by what it controls, then define what white means.

## Give every mask a job

Do not export "nice grayscale images." Export named signals.

Useful production masks include:

* `Mask_Slope_Rock`: white = steep exposed rock
* `Mask_Height_Snow`: white = high snow zone
* `Mask_Flow_Wet`: white = drainage or wet streaks
* `Mask_Deposit_Soil`: white = sediment or soil collection
* `Mask_Region_Path`: white = art-directed path or region

The important part is not which node made the mask. The important part is that the next tool knows how to use it.

## Build masks as outputs, not leftovers

Start with the consumer: material, biome, shader, Unreal layer, Unity terrain layer, color pass, or another Nerthus file. Then choose the source signal: terrain property, erosion data, Data Map, Region, or hand-authored mask.

Shape the range for the job. A shader mask may need clean black and white. A biome blend may need a soft gradient. Texture is useful only if it supports the rule. Mark only production masks for export through [Exporting Nodes](../../../using/build-and-export/exporting-nodes.md) or [Nodes](../../../ui/interface/build-options/nodes.md).

> [!WARNING]
> A colorization mask is not automatically a good weight map. Weight maps often need cleaner ranges and clearer ownership than viewport color masks.

## Fast checks before export

Toggle the mask view and make sure the shape is readable. Check whether mid-gray areas are intentional. Zoom out to gameplay or shot distance. Confirm filenames match material or layer names where possible. Remove masks that nobody downstream will use.

If a material looks muddy, the mask may have too many noisy mid-tones. If a layer appears everywhere, the threshold is probably too broad. If biomes feel random, the mask may be detail-driven instead of intent-driven.
