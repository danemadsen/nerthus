# Derive

Derive nodes create masks and supporting maps from existing terrain, color, flow, slope, or surface structure. Use this family when you need data that describes the terrain rather than directly changing it.

Start here when you need to:

- extract masks from slope, height, angle, curvature, flow, or color
- generate normal, occlusion, rock, soil, or texture support maps
- create selection data for color, surfacing, masking, or export
- inspect terrain features as reusable data in another part of the graph

Useful starting points:

- [Height](height.md), [Slope](slope.md), [Angle](angle.md), and [Curvature](curvature.md) for core geometric selections
- [Flow](flow.md) and [FlowClassic](flow-classic.md) for flow direction and runoff-style data
- [Normals](normals.md) and [Occlusion](occlusion.md) for shading and render support
- [RockMap](rock-map.md), [Soil](soil.md), [TextureBase](texture-base.md), and [Texturizer](texturizer.md) for material and texture masks
- [ColorThreshold](color-threshold.md) for extracting a mask from a color range

## In this section

- [Angle](angle.md): Angle creates a selection mask covering all areas of the heightfield facing the specified range
- [Curvature](curvature.md): Curvature creates a mask where convex areas (protrusions, sharp edges, etc.) are selected. Very useful layer in texture creation, often as highlights on top of an existing texture map
- [ColorThreshold](color-threshold.md): Learn about the ColorThreshold node in Nerthus, including its purpose, controls, and typical terrain workflows
- [Flow](flow.md): Generates a modern flow map showing how water or material would travel across the terrain
- [FlowClassic](flow-classic.md): Generates a classic-style flow map based on legacy flow behavior
- [Height](height.md): The Height node creates a selection mask based on terrain elevation, isolating areas within a specified height range
- [Normals](normals.md): Generates a normal map from the terrain for shading and downstream workflows
- [Occlusion](occlusion.md): The Occlusion node behaves similarly to Ambient Occlusion but favors sedimentary processes and rock crevices rather than lighting
- [Peaks](peaks.md): The Peaks node creates a selection mask for peaks and ridgelines using protrusion, also described as inverse curvature
- [RockMap](rock-map.md): RockMap creates a rock-like coverage mask that can be used to texture terrains on its own or combined with other Data Maps
- [Slope](slope.md): The Slope node creates a selection mask based on the terrain's slope angle, isolating areas within a specified slope range
- [Soil](soil.md): Soil Map creates a soil mask, with increased density in crevices or other areas where the slope allows for soil to settle
- [TextureBase](texture-base.md): Creates a mask simulating natural material distribution onto which color from satmap or cluter or other color node is "flowed"
- [Texturizer](texturizer.md): Generates terrain texture masks using preset styles and controllable secondary effects
