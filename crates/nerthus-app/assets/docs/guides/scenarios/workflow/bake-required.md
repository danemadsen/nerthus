# Bake Only What Matters

Baking is one of the simplest ways to keep large projects fast and memory-friendly - especially when you’re working with Regions, tiled builds, or heavy simulations.

But baking manually, node by node, gets old fast.

That’s where **Gates** come in.

## What Gates Are For

A **Gate** is a simple "checkpoint" node you place at the end of a chain you want to treat as *static*.

Then, instead of baking dozens of nodes individually, you can bake *only the gated checkpoints* using one command.

## The Gate Workflow

1. Identify the parts of your graph that are stable (base shapes, imports, foundational masks).
2. Place a **Gate** node at the end of each of those chains.
3. Use **Bake → Bake Required Nodes** to bake all Gates in one go.
4. If you need to revise the base work later, use **Bake → Unbake All**, make changes, then bake again.

> [!TIP]
> The **Terrain tab filter → Required Baking** is a quick way to see what must be baked before a Region/Hybrid/Tiled workflow.

## Why This Saves Memory (A Lot)

When baked caches are loaded, Nerthus keeps only the "linchpin" baked nodes required by any unbaked downstream work - older ancestors can be unloaded. This reduces live memory consumption dramatically.

In practice, that means:

- fewer rebuilds,
- smaller working set in RAM,
- faster iteration on the parts you’re actively changing.

## Where Gates Shine

### Regions
Regions often require initial generators to be baked before preview/build. Gates make that painless.

### Tiled and Huge Builds
Bake stable foundations once, then iterate on erosion, surface, and lookdev downstream without reprocessing everything upstream.

### Team Graphs
Gates also make handoffs cleaner: "Everything before this Gate is locked".

> [!WARNING]
> Bake your gated checkpoints at the highest practical resolution for your workflow. If you bake too low, you may reduce preview clarity for downstream decisions.
