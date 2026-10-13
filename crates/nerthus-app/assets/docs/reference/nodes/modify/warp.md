# Warp

Warp can take the terrain and "warp" the shape creating more organic shapes.

## When to Use It
Use Warp when the terrain feels too synthetic and needs broader organic distortion.

## Warp

Warp can take the terrain and "warp" the shape creating more organic shapes. In terrains it can help create more believable shapes or alter simple shapes into a more complex formation. In masks, it can help create breakups that can make a linear mask more random and chaotic without losing its general shape.

By using vector fields, the warping created by this node result in more comprehensive shapes without creating directional tearing or over-stretching the original shape.

**Warp as a Post Process**

Warp is also built into the [Modifier Stack](../../../ui/interface/property-editor/modifier-stack.md) for easy access.

## Properties

### Warp

#### Size

The size of the warp noise. Controls the scale of the distortion pattern applied to the input.

#### Strength

The strength of the warp noise. Higher values produce more pronounced distortions.

#### Z Scale

Vertical axis scale for the noise, affecting the intensity of the warp along the Z (height) axis.

#### Warp Source

Selects the noise or pattern used as the source for the warp. Choose from built-in options or provide a custom source.

- **PerlinFBM**: Uses Perlin fractal Brownian motion noise as the warp source, producing smooth, natural distortions.
- **VoronoiR**: Uses Voronoi (R) noise as the warp source, creating cell-like, ridged distortions.
- **VoronoiP**: Uses Voronoi (P) noise as the warp source, suitable for generating mountainous or peak-like distortions.
- **VoronoiA**: Uses Voronoi (A) noise as the warp source, blending characteristics of other Voronoi types for varied effects.
- **VoronoiS**: Uses Voronoi (S) noise as the warp source, emphasizing separated, individual cell boundaries.
- **VoronoiM**: Uses Voronoi (M) noise as the warp source, favoring dominant cells for strong, directional warping.
- **VoronoiD**: Uses Voronoi (D) noise as the warp source, emphasizing ridges and linear features in the distortion.

#### Perturbation

Applies additional random displacement to the warp, increasing the irregularity and complexity of the effect.

#### Complexity

The complexity of the warp noise source. Higher values add more detail and layers to the distortion.

#### Roughness

The roughness of the warp noise. Higher values (especially above 0.5) create harsher, more jagged distortions.

#### Normalized

Specifies whether the noise is normalized. Normalization ensures the output remains within a standard range.

#### Edge Behaviour

Determines how the warp handles the edges of the input, preventing visible seams or artifacts.

- **Edge**: Applies standard edge handling, which may result in wrapping or stretching at the borders.
- **Mirror**: Edge pixels are mirrored, creating seamless, reflected borders for the warp effect.

#### Modulation

The amount of modulation to use from the Modulation input (if connected), allowing external control over the warp intensity.

#### Modulation Direction

Sets the direction of the modulation, controlling how the modulation input influences the warp.

#### Seed

The randomization pattern or seed for the node's process. Changing the seed produces different warp results.

### Iterative

#### Iterations

Number of warp iterations to apply. More iterations increase the complexity and layering of the distortion.

#### Mode

Specifies the mode used for iterative warping, affecting how each pass is calculated and applied.

- **Bitmap**: Applies the warp using standard bitmap-based processing for each iteration.
- **VectorField**: Uses a vector field to guide the warp, allowing for more directional and flow-based distortions.
- **VectorFieldIntegral**: Integrates the vector field over multiple passes, producing complex, cumulative warp effects.
