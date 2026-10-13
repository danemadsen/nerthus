# Mesher

Mesher can save specific nodes as meshes instead of heightfields, and can also generate LODs.

## When to Use It
Use Mesher when a terrain needs to become an actual mesh, especially if you also want generated LODs rather than a heightfield export.

## Properties

#### Format

Specifies the file format for the exported mesh. Choose the format that best fits your workflow or target application.

- **OBJ**: Exports the mesh in the standard OBJ format, widely supported for 3D models and compatible with most 3D software.
- **FBX**: Exports the mesh in the FBX format, suitable for advanced workflows and supporting animation, hierarchy, and metadata.
- **DAE**: Exports the mesh in the Collada (DAE) format, an open standard for exchanging 3D assets between applications.
- **GLTF**: Exports the mesh in the glTF format, optimized for real-time rendering and web-based 3D applications.
- **GLB**: Exports the mesh in the binary GLB format, a compact version of glTF for efficient storage and transmission.

#### Scale

Sets the unit or normalization method for the exported mesh coordinates, determining how the model's size is interpreted.

- **Normalized**: Exports the mesh with coordinates normalized to a 0–1 range, useful for workflows that require unitless or relative positions.
- **Meter**: Exports the mesh with coordinates in meters, matching real-world scale for compatibility with engineering and GIS applications.
- **Kilometer**: Exports the mesh with coordinates in kilometers, suitable for large-scale terrain or geospatial datasets.

#### Topology

Specifies the type of mesh topology used for the exported model, affecting how the surface is constructed and rendered.

- **Tris**: Generates the mesh using triangles for all faces, ensuring compatibility with all 3D engines and applications.
- **Quads**: Generates the mesh using quadrilaterals where possible, preferred for modeling and sculpting workflows.
- **AdaptiveTris**: Uses adaptive triangulation to optimize mesh density, reducing polygon count in flat areas and increasing detail where needed.

#### Topology warning

#### Vertices per side

Sets the resolution of the mesh by specifying the number of vertices along each side. Higher values increase detail but also file size.

#### Artifact reduction

Applies post-processing to reduce visual artifacts in the mesh, such as smoothing or correcting problematic geometry.

- **None**: No artifact reduction is applied; the mesh is exported as generated.
- **Low**: Applies minimal artifact reduction, preserving most original detail while correcting minor issues.
- **Med**: Applies moderate artifact reduction, balancing detail preservation with improved mesh quality.

#### Create normals

If enabled, generates normal vectors for the mesh, improving shading and rendering quality in 3D applications.

#### Create UVs

If enabled, generates UV coordinates for the mesh, allowing textures to be mapped accurately onto the surface.

#### UV origin

Specifies the origin point for the UV coordinate system, affecting how textures are mapped onto the mesh.

- **TopLeft**: Sets the UV origin to the top-left corner, matching conventions used in some graphics applications.
- **BottomLeft**: Sets the UV origin to the bottom-left corner, standard in most 3D engines and texture workflows.

#### Create walls

Creates vertical walls along the edges of the mesh, useful for generating closed or extruded terrain models.

#### Center pivot

If enabled, centers the mesh's pivot point at the origin, making it easier to manipulate in 3D applications.

#### Level of detail

Specifies the level of detail (LOD) for the exported mesh, allowing for simplified versions to be generated for performance optimization.

- **None**: Exports the mesh at full resolution with no LOD applied.
- **LOD1**: Exports a simplified mesh with reduced detail, suitable for distant rendering or performance-sensitive applications.
- **LOD2**: Exports a further simplified mesh with even fewer polygons.
- **LOD3**: Exports a mesh with progressively lower detail, useful for very distant or background objects.
- **LOD4**: Exports a highly simplified mesh, minimizing polygon count for maximum performance.
- **LOD5**: Exports an extremely low-detail mesh, suitable for extreme distance or preview purposes.
- **LOD6**: Exports the lowest possible detail mesh, retaining only the broadest features.

#### LOD info
