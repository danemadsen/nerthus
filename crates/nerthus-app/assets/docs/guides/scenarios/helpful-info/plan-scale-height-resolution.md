# Plan Scale, Height, and Resolution Before You Start

Scale problems usually do not look like scale problems at first.

A terrain feels soft in engine, cliffs feel too vertical, erosion feels too small, or an 8K build still does not hold enough detail. The graph may be fine. The problem is often that terrain width, height range, and output resolution were never chosen as one production setup.

> [!TIP]
> Write down width, height range, target app, and final output type before adding detail nodes.

## Fix the setup before fixing the graph

Start with the three numbers that define the terrain contract: width, height, and resolution. Width and height live in Terrain Definition. Build resolution lives in the resolution settings. Together, they decide the world-space density of the output.

Resolution does not mean quality by itself. A 4096 map across 4096 m gives about 1 m per sample. The same map across 8192 m gives about 2 m per sample. If the terrain feels soft, increasing erosion or surface detail may not help; the world-space density may be wrong.

## A reliable starting pass

Set the target first: gameplay terrain, cinematic vista, closeup patch, tiled world, mesh, or engine landscape. Pick the terrain width in meters. Pick a height range that fits the landscape type. Hills, coastal cliffs, alpine ranges, and badlands need different vertical contracts.

Work at a fast preview resolution while designing macro shape. Choose final build resolution from the output target, not from habit. Use [Using Regions](../../../using/build-and-export/regions/index.md) for closeups and [Tiled Builds](../../../using/build-and-export/tiled-builds.md) for large worlds instead of forcing the whole terrain to one expensive build.

## Stop and revise if

Stop if you only know the resolution, not the terrain width. Stop if the terrain looks correct only from one viewport camera. Stop if you are changing Terrain Definition after masks and erosion are finished. Stop if you cannot explain the terrain height in the receiving app.

Those are signs the graph is carrying a scale decision that should have been made earlier.
