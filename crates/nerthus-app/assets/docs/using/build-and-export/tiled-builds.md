# Tiled Builds

Tiled Builds process a terrain as a grid of smaller chunks, or tiles, instead of building the entire world as one large image.

This helps in two ways:

* Building is more manageable because the workload is split into smaller pieces, reducing peak CPU, GPU, and memory pressure.
* Exported results are already chunked for engines that stream terrain in sections, reducing runtime memory and enabling selective loading.

Tiled Builds are primarily a production feature. You can iterate on a smaller preview resolution while authoring, then switch to tiled output when you need final detail and export-ready assets.

## Why tiles can look different than full builds

Some operations behave differently when they only "see" a portion of the terrain. Any node that depends on information outside the current tile can produce discontinuities at tile or bucket boundaries if it is evaluated in isolation.

Common causes:

* Screen-space or frame-dependent generators and transforms that assume a single full canvas.
* Large-scale filters that sample far outside the local area.
* Procedural detail that is implicitly normalized, auto-leveled, or equalized per region.

The solution is usually one of these:

* Bake the upstream, tile-unfriendly part of the graph into a stable base.
* Use Hybrid Builds with enough bucket size and blending so downstream nodes have context.
* Avoid per-tile normalization steps unless they are intentionally local.

## Setting Up a Tiled Build

Nerthus can produce tiled builds for almost any terrain, but it helps to structure the graph with tiling in mind.

Recommended workflow:

* Build your macro shapes first, the forms that define the overall mountain ranges, basins, and major structures.
* Identify tile-unfriendly nodes in the macro stage and bake them into a base terrain before tiling.
* Apply high-detail, tile-friendly nodes after the bake, such as erosion and surface detailing, because these are usually where most of the perceived fidelity comes from.

Practical checks before you commit to a tiled build:

* Make sure your world resolution divides cleanly into your tile resolution, or choose a tile size that produces an integer tile grid.
* Keep notes on what each output represents, especially if you are exporting multiple maps per tile, such as height, masks, color, and splat maps.
* Avoid operations that "auto adjust" independently per tile if you want consistent tonality across the entire world.

## Baking tile-unfriendly nodes

Some nodes, especially generators and other screen-space operations, are not tile-friendly and should be baked before the tiled build.

Baking is generally safe because:
* The baked result becomes a stable, world-consistent base that every tile can reference.
* Downstream nodes like erosion, wear, deposits, and surface detailing can still generate high-frequency detail per tile, as long as they have sufficient context from Hybrid buckets and blending.

> [!TIP]
> Don't know which nodes to build? Go to the `Graph Menu > Bake > Bake Required Nodes` this will automatically baked any nodes that need to be baked.
>
> Remember: baking is not permanent, you can unbake and re-bake as often as you like.

A useful mental model is to separate your graph into two stages:
* World stage: anything that defines global structure and should be identical no matter how the terrain is chunked.
* Detail stage: anything that refines, weathers, and textures the structure and can be evaluated locally with overlap.

> [!TIP]
> Always bake at a version closest to your complete world size. For example, if you have `8 x 8` tiles of `512 x 512` then choose `4096 x 4096` as your baking resolution.

## Hybrid Builds

Hybrid Builds are a tiled build mode where the final exported tile size can remain small, such as 512 x 512, but the terrain is built in larger buckets, such as 4096 x 4096, and then subdivided into tiles.

Why this matters:

* Building at very small tile resolutions can lose fidelity, especially for nodes that need neighborhood context.
* Larger buckets give downstream nodes more surrounding data, improving continuity and detail.
* Modern processors and Nerthus's engine make 4K-class bucket builds practical, so you can often gain quality without a large speed penalty.

A simple example:

* Final tiles: 512 x 512
* Bucket size: 4096 x 4096
* Result: 8 x 8 tiles per bucket, 64 tiles generated from one bucket build

If your world fits inside 8K or smaller, a single-bucket build can give a strong balance between high fidelity and efficiency while still outputting tiles at your desired size. Even if your world fits inside 8K, you may still choose smaller buckets to reduce memory usage on complex graphs or limited hardware.

Hybrid Subdivision options:

* None: no subdivision for fast, high-quality builds, only available when your world fits inside 8K or 16K
* Faster: faster build speed with moderate quality
* Balanced: a balance between speed and quality
* Slower: highest quality but slower build speed

## Blending

When you use Hybrid Subdivision, buckets are processed separately. Some nodes can produce visible mismatches at bucket borders because features near the edge do not have full context from the neighboring bucket.

To compensate, increase the blend-in amount between buckets. Around 25% is often a good starting point.

Guidelines:

* Increase blending when you see seams in erosion, deposits, or any effect that depends on neighborhood sampling.
* Prefer increasing bucket size before pushing blending very high, if memory allows. Bigger buckets reduce the number of boundaries in the first place.
* If you are still seeing seams, check for nodes that normalize or equalize locally, and move those steps to the baked world stage.

## Choosing tile size for engines

Tile size is mostly an integration decision:

* Smaller tiles stream more flexibly and can reduce runtime memory spikes, but increase tile count and file overhead.
* Larger tiles reduce file count and stitching work, but increase streaming granularity.

Common choices are 512 or 1024, depending on your engine and world size. Match tile size to how your engine partitions terrain and how much detail you need per chunk.

## Output consistency tips

* Keep global tonality global. If you auto-level, equalize, or normalize, do it on a world-consistent stage, not per tile, unless you explicitly want local variation.
* Treat data outputs as data. For splat maps, masks, and other packed control textures, ensure your pipeline preserves linear values and that the target engine imports them as non-color data.
* Validate with a seam test. Build a small set of adjacent tiles and inspect their borders before running a full-world build.

## Tile Options

For details on Tile Options, see "[Tiles](../../ui/interface/build-options/tiles.md)".
