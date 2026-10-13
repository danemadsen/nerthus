# TextureBaker

Bakes derived texture maps from the terrain for export and downstream use.

## When to Use It
Use TextureBaker when you need texture maps derived from the terrain baked for a mesh, for export and use in other applications.

## Properties

#### Generate normals

Enables the generation of tangent-space normal maps for the mesh. When checked, MeshWarp will output normal maps that align with the mesh geometry at each LOD, suitable for advanced shading and detail.

#### Add Input

Adds an additional input port to connect masks, color maps, or other data layers. Each input will be processed and fitted to the mesh at all LODs.

#### LOD info

Shows Level of Detail (LOD) information for the mesh and all connected maps. Each input map will have an LOD generated to match the mesh's LODs as set in the Mesher node.
