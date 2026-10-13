# Separate a Project

Big terrains get heavy fast. Even if your graph is well-optimized, a single "everything in one file" project can balloon memory use, slow previews, and make experimentation risky.

A simple way to stay fast (and sane) is to **break your project into multiple parts** - then pass data forward by exporting stable results as **Nerthus RAW** and ingesting them in the next file with a **File** node.

> [!TIP]
> Think of each file as a "chapter": Shape → Detail → Output. Keep each chapter focused and your iteration speed stays high.

## Why this helps

Splitting a project into separate files gives you several practical wins:

* **Lower memory consumption**: You only load the graphs you need for the task you’re doing right now.
* **Faster iteration**: Smaller graphs preview and rebuild faster than monolithic ones.
* **Safer experimentation**: Try new techniques in a new file without breaking a working baseline.
* **Collaboration-friendly**: Different people can work on different portions (macro shape, erosion pass, texturing, masks) without stepping on each other.
* **Source control friendly**: Smaller, purpose-built files reduce churn and make changes easier to track.

## A simple split that works well: Shape vs. Detailing

Most terrains naturally divide into two phases:

### 1. Base Shape File (macro)

Use this file for:

* Landmass, primary forms, broad elevation design
* Large-scale masks and region planning
* Any "foundation" work you don’t want to constantly re-run later

When the macro shape is in a good place, **export the key outputs as Nerthus RAW** (heightfield, and any critical masks you’ll need downstream).

### 2. Detail File (micro)

In a new file:

* Add a **File** node and ingest the RAW outputs
* Perform higher-cost detailing: erosion variants, sediment passes, fine noise, breakup, surface detail, specialized masks
* Iterate aggressively - because your upstream foundation is now stable and lightweight

> [!TIP]
> If you’re unsure what will matter later, export one extra "utility" mask (e.g., broad slope/height zones). It’s cheap insurance for downstream flexibility.

## Work in modules (even inside the same terrain)

You can apply the same approach beyond "shape vs detail":

* **Erosion Lab**: A dedicated file just for erosion exploration - export the best result back into the pipeline.
* **Mask Factory**: One file that produces clean masks (flow, wear zones, deposition regions) for use elsewhere.
* **Biome/Color Pass**: Separate file for albedo/biome logic so you can iterate visuals without touching terrain structure.

## Variations without fear

Instead of branching inside one giant graph, do it with files:

* Duplicate the detail file and try a new erosion method or experimental node chain.
* Keep your "gold master" foundation untouched.
* Swap between variations by pointing a File node at a different RAW export.

This makes "what if we try…" almost free - because you’re not destabilizing the entire project.

## Team workflow benefits

Splitting files makes it much easier to divide labor:

* Artist A: base landmass and macro design
* Artist B: erosion and surface detail
* Artist C: masks, biomes, and export packaging

As long as everyone agrees on the **shared interface** (the RAW outputs and naming), the pipeline stays clean.

## Source control tips

Terrain files and RAW outputs are often large/binary, so a modular approach helps:

* Commit smaller `.terrain` files more frequently (less change risk).
* Store large RAW exports in a predictable folder (and consider Git LFS if needed).
* Use consistent names like:

  * `Terrain_BaseShape_v03.raw`
  * `Terrain_Masks_Slope_v03.raw`
  * `Terrain_Detail_ErosionA_v02.terrain`

> [!TIP]
> When chaining files, keep your terrain definition consistent (scale, dimensions, and intended resolution). If you change the base terrain’s scale later, you’ll likely need to re-export RAW and refresh any downstream files that ingest it.

---

# Organize Modular Projects Without Breaking Links

When you split a terrain into multiple `.terrain` files (Shape → Detail → Lookdev), you gain speed and stability - but you also introduce more *outputs* (RAWs, masks, builds) that need to stay predictable.

A clean folder layout + stable "latest" build paths lets you iterate hard and [Avoid Roundtrips](../helpful-info/roundtrip.md) in your DCC.

> [!TIP]
> Design your folder structure so source files never move, RAW handoff files have stable names, and DCC links always point at a static "Latest" folder.

## Recommended folder layout

Here’s a practical structure that works for solo and teams:

```text
Project_Terrain/
├── _docs/
│   ├── notes.md
│   └── refs/
├── _dcc/
│   ├── blender/
│   ├── houdini/
│   ├── maya/
│   └── unreal/
├── _nerthus/
│   ├── 00_shape/
│   │   ├── Terrain_Shape.terrain
│   │   └── exports/
│   │       ├── Terrain_Shape_height.raw
│   │       ├── Terrain_Shape_masks_height.raw
│   │       └── Terrain_Shape_masks_slope.raw
│   ├── 10_detail/
│   │   ├── Terrain_Detail.terrain
│   │   └── exports/
│   │       ├── Terrain_Detail_deposition.raw
│   │       ├── Terrain_Detail_flow.raw
│   │       └── Terrain_Detail_height.raw
│   └── 20_lookdev/
│       ├── Terrain_Lookdev.terrain
│       └── exports/
│           ├── Terrain_Lookdev_color.exr
│           └── Terrain_Lookdev_material_masks.exr
├── _builds/
│   ├── shape/
│   │   ├── 2026-01-01_[+++]/
│   │   └── Latest/
│   ├── detail/
│   │   ├── 2026-01-01_[+++]/
│   │   └── Latest/
│   └── final/
│       ├── 2026-01-01_[+++]/
│       └── Latest/
└── _publish/
    └── Latest/
        ├── height.raw
        ├── masks/
        └── textures/
```

### What goes where

* **`_nerthus/`**: your editable `.terrain` files, split by stage
* **`exports/`** inside each stage: *handoff* outputs (usually Nerthus RAW + key masks) that the next stage ingests via **File**
* **`_builds/`**: heavy build outputs (8K/16K, multi-texture sets, etc.)
* **`_publish/Latest/`**: optional "clean" packaging layer for the rest of the pipeline (especially helpful if you want DCC to reference one simple folder)

## Naming conventions that scale

Keep names *semantic* and stable. Avoid embedding versions in filenames unless you truly need multiple variants live.

### Terrain files

* `Terrain_Shape.terrain`
* `Terrain_Detail.terrain`
* `Terrain_Lookdev.terrain`

### RAW handoff exports (stable interface)

* `Terrain_Shape_height.raw`
* `Terrain_Shape_masks_slope.raw`
* `Terrain_Detail_height.raw`
* `Terrain_Detail_flow.raw`

:::
Treat RAW exports like an API between files: stable names, stable meaning. If you change what a file outputs, change the name (or add a new output), not the path.
:::

## Avoid roundtrips with "Latest" folders

This is the big workflow unlock.

In your Build Destination, you can keep an incremental history **and** a stable folder for DCC links by enabling:

* `Maintain a static folder with the latest copy` (in [Build Settings](../../../ui/interface/build-options/index.md), Build tab)

Then use an incremental destination pattern (like `[+++]`) for history, while your DCC always points to:

* `_builds/final/Latest/`

Result:

* You keep per-build archives (numbered folders)
* Your DCC references never change (always `Latest`)

> [!TIP]
> If your DCC points at numbered build folders, you’re guaranteeing roundtrips. Point it at Latest and keep numbered folders strictly as history.

You can read more about [Avoid Roundtrips](../helpful-info/roundtrip.md).

### Optional: snapshot the exact `.terrain`

If you want perfect reproducibility for archived builds, enable:

* `Copy the .terrain file to the build folder`

That gives you a time-stamped "this exact graph made this output" record.

## A "handoff-first" workflow that stays stable

**In Shape file**

* Export: `Terrain_Shape_height.raw` (and a few masks)
* Next file ingests via File node (path stays constant)

**In Detail file**

* Ingest shape RAW
* Export: `Terrain_Detail_height.raw` (+ detail masks)
* Lookdev ingests detail RAW (again: stable path)

**In Final/Lookdev builds**

* Build to `_builds/final/[+++]/`
* Let Nerthus maintain `_builds/final/Latest/`
* DCC references `_builds/final/Latest/…`

## Team + source control notes

* Put editable `.terrain` files under `_nerthus/` and treat that as "source".
* Put `_builds/` in ignore rules (it’s usually huge and regenerable).
* Decide whether RAW handoff exports live in version control:

  * **Small team / fast iteration**: keep RAWs out of VC, regenerate as needed
  * **Distributed team / reproducibility**: store critical RAW handoffs (or manage them via an asset system / LFS)

> [!WARNING]
> If multiple people work in parallel, agree on the "contract" files:
> Terrain_Shape_height.raw, …_slope.raw, etc. Don’t rename them casually.

## One more trick: a clean "Publish Latest"

If your DCC wants a simplified structure, make `_publish/Latest/` the only folder external tools see.

* Copy (or script-copy) from `_builds/final/Latest/` into `_publish/Latest/`
* Keep names consistent (`height.raw`, `masks/`, `textures/`)
* DCC points to `_publish/Latest/` forever

This is especially nice when different Nerthus stages output different internal naming, but your downstream pipeline expects a consistent package.
