# Normals

Generates a normal map from the terrain for shading and downstream workflows.

## When to Use It
Use Normals when you need a normal map of the terrain for shading in another application, or as a source of directional masks.

## Properties

#### Type

Specifies the method used to generate the normal map. Determines the level of detail and the intended use for the resulting normals.

- **Standard**: Generates a standard normal map based on the main heightfield, suitable for general use in rendering and texturing.
- **Detail**: Generates a high-frequency detail normal map, emphasizing fine surface features for use as a detail overlay or in advanced shading.

#### Width

Sets the horizontal resolution of the generated normal map in pixels. Higher values produce more detailed maps but increase memory usage.

#### Height

Sets the vertical resolution of the generated normal map in pixels. Higher values produce more detailed maps but increase memory usage.

#### Detail size

Controls the scale of fine details captured in the normal map. Smaller values emphasize finer features, while larger values smooth out detail.

#### Handedness

Determines the coordinate system used for the normal map, affecting how normals are interpreted in different rendering engines.

- **Left**: Generates the normal map using a left-handed coordinate system, commonly used in some 3D applications and engines.
- **Right**: Generates the normal map using a right-handed coordinate system, standard in many 3D applications and engines.

#### Up axis

Specifies which axis is considered 'up' for the normal map, ensuring compatibility with different engine conventions.

- **YUp**: Sets the Y axis as the vertical (up) direction, matching engines and workflows that use Y as up.
- **ZUp**: Sets the Z axis as the vertical (up) direction, matching engines and workflows that use Z as up.
