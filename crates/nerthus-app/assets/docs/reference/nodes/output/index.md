# Output

Output nodes prepare terrain data for viewing, baking, rendering, or handoff to another tool. Use this family when the graph is ready to produce a map, preview, mesh, engine export, or final downstream asset.

Start here when you need to:

- preview lighting, ambient occlusion, or presentation renders
- bake derived texture maps from terrain data
- export heightfields, masks, meshes, point clouds, or engine-ready packages
- prepare output for a game engine or another production pipeline

Useful starting points:

- [Export](export.md) for general file export
- [TextureBaker](texture-baker.md) for derived texture maps
- [Mesher](mesher.md) for mesh output
- [AO](ao.md), [Shade](shade.md), [Sunlight](sunlight.md), and [LightX](light-x.md) for visual preview and render support

## In this section

- [AO](ao.md): Creates an ambient occlusion map that darkens recessed areas and enhances terrain depth
- [Cartography](cartography.md): Cartography creates a topographical overlay for a map
- [Export](export.md): Exports terrain, masks, or textures to external files
- [Halftone](halftone.md): Converts the terrain or mask into a halftone-style rendered output
- [LightX](light-x.md): Creates a fully lit render using controllable sun, shadow, and ambient lighting
- [Mesher](mesher.md): Mesher can save specific nodes as meshes instead of heightfields, and can also generate LODs
- [PointCloud](point-cloud.md): Exports the terrain as a point cloud, in PLY or XYZ format
- [Shade](shade.md): Produces a shaded terrain render based on lighting direction and surface relief
- [Sunlight](sunlight.md): Sunlight is a Data Map that gives you the sunlight integral (sun light/shadow map) of any given period (up to 365 days). It can be used to drive snow melt, vegetation growth, and more
- [TextureBaker](texture-baker.md): Bakes derived texture maps from the terrain for export and downstream use
