# Build and Export

Build and Export is the final handoff stage of a Nerthus project. This is where you decide what data leaves the graph, at what resolution, in which format, and how it should be organized for the target application or pipeline.

Nerthus can export heightfields, masks, color maps, splat maps, meshes, point clouds, tiled worlds, and application-specific outputs. The right setup depends on what the receiving tool expects and how much precision the terrain needs.

## Main Export Paths

[Exporting Nodes](exporting-nodes.md) explains how to mark nodes for export, choose output ports, name files, and use dedicated output nodes such as [Export](../../reference/nodes/output/export.md), [Mesher](../../reference/nodes/output/mesher.md) and [PointCloud](../../reference/nodes/output/point-cloud.md).

[Mesh Exports](mesh-exports.md) covers mesh output through the [Mesher](../../reference/nodes/output/mesher.md) node, including grid meshes, optimized triangle meshes, scale choices, texture coordinates, and normals.

[Tiled Builds](tiled-builds.md) explains how to build large worlds as tiles or hybrid builds, including when to bake tile-unfriendly graph sections.

[Using Regions](regions/index.md) shows how to build a specific part of a terrain at higher resolution without rebuilding the entire world.

For the interface that controls resolution, build scope, tiles, regions, profiles, output nodes, and scripts, see [Build Settings](../../ui/interface/build-options/index.md).

## Before You Build

* Confirm the final terrain size and height range in [Terrain](../../ui/interface/build-options/terrain.md).
* Check that every required output is marked for export or connected to an output node.
* Choose formats based on the type of data: heightfields need more precision than many masks or preview textures.
* Treat masks and packed maps as data so downstream tools do not apply unwanted color correction.
* Bake stable or tile-unfriendly sections before a tiled or hybrid build when needed.
* Run a small test build before committing to a large final output.

## Target Applications

For guidance specific to software such as [Blender](../../guides/use-in/apps/blender.md) or [Unreal](../../guides/use-in/engines/unreal.md), see [Apps](../../guides/use-in/apps/index.md).

For automation scenarios where output paths, variables, or repeated builds are controlled outside the UI, see [Command Line Automation](../../developers/automation/cli/command-line-automation.md) and [Managing Input and Output](../../developers/automation/building-bridges/managing-input-and-output.md).
