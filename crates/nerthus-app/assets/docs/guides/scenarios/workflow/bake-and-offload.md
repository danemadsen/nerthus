# Bake and Offload Data

At high resolutions, node previews can consume **a lot** of memory - and they’ll often take longer to process than the same graph at 1K–4K. If parts of your graph are stable (even temporarily), the simplest way to keep Nerthus responsive is to **bake those nodes** so they no longer need to be recalculated during regular work.

If you have portions of the graph that are not changing, even for the moment, the easiest thing is to [bake the nodes](../../../using/baking-nodes/index.md).

> [!TIP]
> Bake your nodes at the final build resolution whenever possible - or at least as high as you can comfortably manage.

## Why Baking Saves Memory

Baking captures a node’s current output at a chosen resolution and lets Nerthus reuse that data instead of rebuilding the upstream chain every time you preview or build. This reduces:

* **Live RAM usage** (less upstream data needs to remain in memory)
* **Rebuild time** (fewer nodes are recomputed)
* **Graph "thrash"** when tweaking downstream nodes

If you need to make changes later, simply **unbake** the node, adjust it, and bake again.

## Bake in Batches

If you have a lot of nodes and you try to bake everything at 8K or 16K in one go, you may run out of memory. Instead, bake **one (or a few) nodes at a time**:

1. Pick a stable "checkpoint" node near the end of a branch.
2. Bake it.
3. Move to the next branch or checkpoint.

As you bake, Nerthus can keep only the baked nodes that still matter for your current work session - specifically the **linchpin nodes** - and unload baked ancestors that are no longer required. This can dramatically reduce live memory consumption.

> [!TIP]
> For best results, bake "downstream checkpoints" first (nodes closer to where you’re currently working). That increases the chance earlier baked ancestors can be unloaded.

## Persist baked previews to disk

If you’re memory-constrained, consider enabling **Cache Previews to Disk**. This stores baked/processed previews on disk instead of keeping them in RAM, which can help large graphs stay workable across sessions.

## Baked Data in Builds

Once nodes are baked, your normal builds can reuse that baked data - so builds become noticeably faster and less memory-hungry, especially in complex graphs.

> [!WARNING]
> For 16K builds, we highly recommend a minimum of 128GB RAM. 256GB is preferred. Also remember that 16K nodes may not fit in your GPU memory, depending on your hardware and build settings.
