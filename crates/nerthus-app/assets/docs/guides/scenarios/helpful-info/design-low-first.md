# Start with low detail shapes

It’s tempting to pour detail into the very first landform - lots of noise, breakup, micro ridges. But front-loading detail usually makes the rest of the graph fight you.

A cleaner approach is to build a **strong, low-detail macro shape** first, then let **Erosion** and **Surface** nodes generate the believable detail later.

## Why early detail is a trap

When your base terrain is already "busy":

* **Procedural shaping has less room to work**: transforms, carving, and large-scale edits get masked by pre-baked noise.
* **Erosion can’t speak clearly**: fine detail interferes with flow and creates messy, over-textured results.
* **Iteration gets fragile**: small upstream changes ripple into unpredictable downstream chaos.

> [!TIP]
> Macro first, micro later. Let erosion create the story, and let surface add the finish.

## The scalable workflow for huge or tiled builds

This approach is especially powerful for **tiled builds** and very large resolutions.

1. Build your **base shapes** at a manageable resolution like **4K or 8K** (broad forms only).
2. **Bake** those shape nodes so they become stable and cheap.
3. Use **TileGate** to treat those baked shapes as **static inputs** that will be upscaled efficiently for large tiled builds.
4. Apply your heavy detail *afterwards*: **Erosion**, **Surface**, and any downstream detailing/mask work.

Now you get the best of both worlds:

* The macro terrain remains **clean and controllable**
* Tiled builds stay **memory-friendly**
* Your final detail is generated at the stage where it matters, so the terrain **holds up** without losing fidelity

> [!WARNING]
> If you bake noisy "micro" detail into the initial shape, you can lock yourself out of realistic erosion and make high-resolution/tiled builds slower and less predictable.
