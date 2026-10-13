# Shade

Produces a shaded terrain render based on lighting direction and surface relief.

## When to Use It
Use Shade when you want a quick lit preview of terrain relief without leaving the graph.

## Properties

#### Render style

Specifies the shading technique used to render the terrain. Each style provides a different visual representation for analysis or artistic purposes.

- **TableTop**: Renders the terrain for a stylized overview.
- **HillShade**: Applies traditional hillshading to simulate sunlight and shadow, enhancing the perception of terrain relief and surface features.
- **Occlusion**: Highlights areas of the terrain that are occluded or in shadow, useful for analyzing light accessibility and terrain exposure.
- **SoftLight**: Uses a soft, diffuse lighting model to gently illuminate the terrain, reducing harsh shadows and creating a more natural look.

#### Sun azimuth

Sets the horizontal angle of the sun in degrees, determining the direction from which sunlight is cast across the terrain.

#### Sun elevation

Controls the vertical angle of the sun above the horizon, affecting the length and intensity of shadows on the terrain.

#### Sunlight

Adjusts the overall intensity or brightness of direct sunlight in the shading calculation.

#### Shade bias

Offsets the shading calculation to reduce or increase shadow strength, helping to fine-tune the balance between light and dark areas.

#### Soft shadows

Enables or adjusts the softness of shadows, creating more gradual transitions between light and shadow for a realistic effect.

#### Reflected light

Simulates indirect illumination by adding light reflected from surrounding terrain, enhancing realism and reducing overly dark shadowed areas.

#### High quality

Enables high-quality rendering for shading, using more advanced calculations for smoother gradients and more accurate lighting effects.
